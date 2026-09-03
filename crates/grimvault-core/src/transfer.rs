//! Moving items between the transfer stash and the vault store, as pure
//! model edits: nothing here touches a file, and a failed move leaves
//! both the store and the stash exactly as they were.
//!
//! A stash tab is a `width` × `height` grid of cells; an item at cell
//! `(x, y)` with footprint `w × h` occupies `[x, x+w) × [y, y+h)`. The
//! game stores those cells as `f32`s holding whole numbers
//! ([`StashItem`]), so every position is checked to be a whole,
//! in-range cell before it is treated as one. Footprints are reference
//! data the caller supplies through [`Footprints`] — normally the
//! layered [`GameData`] — because the save file does not carry them.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use univault_engine::grid::{CellRect, find_open_cells, fits};
use univault_engine::ids::{GridPos, RecordId};

use crate::block::StashTab;
use crate::gamedata::{Footprint, GameData};
use crate::gst::TransferStash;
use crate::item::{Item, StashItem};
use crate::store::{ItemOrigin, StoredItemId, Timestamp, VaultStore};

/// Position of a tab within the transfer stash, as the file orders
/// them. Serializes as the bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TabIndex(u32);

impl TabIndex {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    fn slot(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

impl fmt::Display for TabIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Position of an item within a tab's item list, as the file orders
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemIndex(usize);

impl ItemIndex {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn value(self) -> usize {
        self.0
    }
}

impl fmt::Display for ItemIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Supplies the grid footprint of an item, `None` when the source has
/// no record of it.
pub trait Footprints {
    fn footprint(&self, item: &Item) -> Option<Footprint>;
}

impl Footprints for GameData {
    fn footprint(&self, item: &Item) -> Option<Footprint> {
        let base = RecordId::parse(item.base_name.clone())?;
        let info = self.item_info(&base)?.ok()?;
        GameData::footprint(self, &info.bitmap?)?.ok()
    }
}

/// Largest cell coordinate or grid dimension handled: far beyond any
/// real tab, and below `f32`'s exact-integer limit (2^24) so every
/// cell converts to the file's `f32` and back without loss.
const MAX_CELLS: i32 = 1 << 16;

/// Why a tab's occupancy could not be computed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OccupancyError {
    /// The tab's dimensions exceed [`MAX_CELLS`].
    #[error("tab is {width}x{height} cells, beyond the {MAX_CELLS} this app handles")]
    GridTooLarge { width: u32, height: u32 },
    /// An occupant's footprint is unknown, so its cells cannot be known.
    #[error("item {index} ({base_name:?}) has no known footprint")]
    UnknownFootprint { index: ItemIndex, base_name: String },
    /// An occupant's position is not a whole, non-negative cell within
    /// [`MAX_CELLS`].
    #[error("item {index} is not at a whole, in-range cell")]
    InvalidPosition { index: ItemIndex },
}

/// Why a move between the store and the stash was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransferError {
    #[error("the stash has no tab {0}")]
    NoSuchTab(TabIndex),
    #[error("tab {tab} has no item {index}")]
    NoSuchItem { tab: TabIndex, index: ItemIndex },
    #[error("the store has no item {0}")]
    NoSuchStoredItem(StoredItemId),
    /// The stored item's own footprint is unknown, so it cannot be
    /// placed anywhere.
    #[error("stored item {id} ({base_name:?}) has no known footprint")]
    UnknownFootprint { id: StoredItemId, base_name: String },
    #[error("tab {tab} has no room for a {}x{} item", .footprint.width, .footprint.height)]
    NoRoom { tab: TabIndex, footprint: Footprint },
    #[error("cells at ({},{}) in tab {tab} are occupied", .pos.x, .pos.y)]
    Occupied { tab: TabIndex, pos: GridPos },
    #[error("a {}x{} item at ({},{}) does not fit inside tab {tab}", .footprint.width, .footprint.height, .pos.x, .pos.y)]
    OutOfBounds {
        tab: TabIndex,
        pos: GridPos,
        footprint: Footprint,
    },
    /// The tab's existing occupants could not be laid out.
    #[error(transparent)]
    Occupancy(#[from] OccupancyError),
}

/// The cells every item in `tab` occupies, in item order.
///
/// # Errors
/// [`OccupancyError`] when an occupant's footprint or position cannot
/// be resolved.
pub fn occupancy(
    tab: &StashTab,
    footprints: &impl Footprints,
) -> Result<Vec<CellRect>, OccupancyError> {
    grid(tab, footprints).map(|grid| grid.occupied)
}

/// First free spot for `footprint` in `tab` (columns left to right, each
/// top to bottom); `Ok(None)` when nothing fits.
///
/// # Errors
/// [`OccupancyError`] when the tab's occupants cannot be laid out.
pub fn find_slot(
    tab: &StashTab,
    footprint: Footprint,
    footprints: &impl Footprints,
) -> Result<Option<GridPos>, OccupancyError> {
    let grid = grid(tab, footprints)?;
    Ok(find_open_cells(
        &grid.occupied,
        footprint.width,
        footprint.height,
        grid.width,
        grid.height,
    ))
}

/// Whether `footprint` at `pos` lies inside `tab` and overlaps nothing.
///
/// # Errors
/// [`OccupancyError`] when the tab's occupants cannot be laid out.
pub fn can_place_at(
    tab: &StashTab,
    footprint: Footprint,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<bool, OccupancyError> {
    let grid = grid(tab, footprints)?;
    Ok(check_placement(&grid, rect_at(pos, footprint)).is_ok())
}

/// Removes item `index` from `tab` and stores it with
/// [`ItemOrigin::TransferStash`]. Needs no footprint: the store has no
/// grid.
///
/// # Errors
/// [`TransferError::NoSuchTab`] or [`TransferError::NoSuchItem`]; the
/// store and stash are unchanged on error.
pub fn vault_from_stash(
    stash: &mut TransferStash,
    tab: TabIndex,
    index: ItemIndex,
    store: &mut VaultStore,
    at: Timestamp,
) -> Result<StoredItemId, TransferError> {
    let tab_ref = tab_mut(stash, tab)?;
    if index.value() >= tab_ref.items.len() {
        return Err(TransferError::NoSuchItem { tab, index });
    }
    let placed = tab_ref.items.remove(index.value());
    Ok(store.add(placed.item, ItemOrigin::TransferStash { tab }, at))
}

/// Moves stored item `id` into the first free spot of `tab`, returning
/// where it landed.
///
/// # Errors
/// Any [`TransferError`]; the store and stash are unchanged on error.
pub fn place_in_stash(
    store: &mut VaultStore,
    id: StoredItemId,
    stash: &mut TransferStash,
    tab: TabIndex,
    footprints: &impl Footprints,
) -> Result<GridPos, TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let pos = find_slot(tab_ref(stash, tab)?, footprint, footprints)?
        .ok_or(TransferError::NoRoom { tab, footprint })?;
    move_into_tab(store, id, stash, tab, pos)?;
    Ok(pos)
}

/// Moves stored item `id` into `tab` at exactly `pos`.
///
/// # Errors
/// [`TransferError::OutOfBounds`] or [`TransferError::Occupied`] when
/// the cells are not free, or any other [`TransferError`]; the store
/// and stash are unchanged on error.
pub fn place_in_stash_at(
    store: &mut VaultStore,
    id: StoredItemId,
    stash: &mut TransferStash,
    tab: TabIndex,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<(), TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let grid = grid(tab_ref(stash, tab)?, footprints)?;
    check_placement(&grid, rect_at(pos, footprint)).map_err(|blocked| match blocked {
        Blocked::OutOfBounds => TransferError::OutOfBounds {
            tab,
            pos,
            footprint,
        },
        Blocked::Occupied => TransferError::Occupied { tab, pos },
    })?;
    move_into_tab(store, id, stash, tab, pos)
}

/// A tab's grid with every occupant resolved to cells.
struct TabGrid {
    width: i32,
    height: i32,
    occupied: Vec<CellRect>,
}

enum Blocked {
    OutOfBounds,
    Occupied,
}

fn grid(tab: &StashTab, footprints: &impl Footprints) -> Result<TabGrid, OccupancyError> {
    let (Some(width), Some(height)) = (dimension(tab.width), dimension(tab.height)) else {
        return Err(OccupancyError::GridTooLarge {
            width: tab.width,
            height: tab.height,
        });
    };
    let occupied = tab
        .items
        .iter()
        .enumerate()
        .map(|(slot, placed)| occupant_rect(ItemIndex::new(slot), placed, footprints))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TabGrid {
        width,
        height,
        occupied,
    })
}

fn occupant_rect(
    index: ItemIndex,
    placed: &StashItem,
    footprints: &impl Footprints,
) -> Result<CellRect, OccupancyError> {
    let footprint =
        footprints
            .footprint(&placed.item)
            .ok_or_else(|| OccupancyError::UnknownFootprint {
                index,
                base_name: placed.item.base_name.clone(),
            })?;
    let (Some(x), Some(y)) = (cell_from_f32(placed.x), cell_from_f32(placed.y)) else {
        return Err(OccupancyError::InvalidPosition { index });
    };
    Ok(rect_at(GridPos { x, y }, footprint))
}

/// Bounds first (against an empty grid), then overlap, so the two
/// refusals stay distinguishable.
fn check_placement(grid: &TabGrid, candidate: CellRect) -> Result<(), Blocked> {
    if !fits(&[], candidate, grid.width, grid.height) {
        return Err(Blocked::OutOfBounds);
    }
    if !fits(&grid.occupied, candidate, grid.width, grid.height) {
        return Err(Blocked::Occupied);
    }
    Ok(())
}

fn rect_at(pos: GridPos, footprint: Footprint) -> CellRect {
    CellRect {
        x: pos.x,
        y: pos.y,
        width: footprint.width,
        height: footprint.height,
    }
}

fn stored_footprint(
    store: &VaultStore,
    id: StoredItemId,
    footprints: &impl Footprints,
) -> Result<Footprint, TransferError> {
    let stored = store.get(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    footprints
        .footprint(stored.item())
        .ok_or_else(|| TransferError::UnknownFootprint {
            id,
            base_name: stored.item().base_name.clone(),
        })
}

/// The final, fallible-only-by-lookup step of a placement: the tab is
/// resolved before the store is touched, so a missing tab leaves the
/// store intact.
fn move_into_tab(
    store: &mut VaultStore,
    id: StoredItemId,
    stash: &mut TransferStash,
    tab: TabIndex,
    pos: GridPos,
) -> Result<(), TransferError> {
    let tab_ref = tab_mut(stash, tab)?;
    let stored = store.take(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    tab_ref.items.push(StashItem {
        item: stored.into_item(),
        x: cell_to_f32(pos.x),
        y: cell_to_f32(pos.y),
    });
    Ok(())
}

fn tab_ref(stash: &TransferStash, tab: TabIndex) -> Result<&StashTab, TransferError> {
    tab.slot()
        .and_then(|slot| stash.tabs.get(slot))
        .ok_or(TransferError::NoSuchTab(tab))
}

fn tab_mut(stash: &mut TransferStash, tab: TabIndex) -> Result<&mut StashTab, TransferError> {
    tab.slot()
        .and_then(|slot| stash.tabs.get_mut(slot))
        .ok_or(TransferError::NoSuchTab(tab))
}

fn dimension(cells: u32) -> Option<i32> {
    i32::try_from(cells)
        .ok()
        .filter(|&cells| cells <= MAX_CELLS)
}

fn cell_from_f32(value: f32) -> Option<i32> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "MAX_CELLS is far below f32's exact-integer limit"
    )]
    let max = MAX_CELLS as f32;
    if value.fract() != 0.0 || value < 0.0 || value > max {
        return None;
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "whole and within 0..=MAX_CELLS, checked above"
    )]
    let cell = value as i32;
    Some(cell)
}

fn cell_to_f32(cell: i32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "cells are bounded by MAX_CELLS, below f32's exact-integer limit"
    )]
    let value = cell as f32;
    value
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::block::TabDecoration;
    use crate::item::ContainerVersion;

    const CLUSTER: &str = "records/items/materia/a01_aethercluster.dbr";
    const LEGS: &str = "records/items/gearlegs/l01.dbr";
    const MYSTERY: &str = "records/items/mystery.dbr";

    struct Table(HashMap<&'static str, Footprint>);

    impl Footprints for Table {
        fn footprint(&self, item: &Item) -> Option<Footprint> {
            self.0.get(item.base_name.as_str()).copied()
        }
    }

    fn table() -> Table {
        Table(HashMap::from([
            (
                CLUSTER,
                Footprint {
                    width: 1,
                    height: 2,
                },
            ),
            (
                LEGS,
                Footprint {
                    width: 2,
                    height: 3,
                },
            ),
        ]))
    }

    fn item(base_name: &str) -> Item {
        Item {
            base_name: base_name.into(),
            seed: 7,
            stack_count: 1,
            ..Item::default()
        }
    }

    fn placed(base_name: &str, x: f32, y: f32) -> StashItem {
        StashItem {
            item: item(base_name),
            x,
            y,
        }
    }

    fn tab(width: u32, height: u32, items: Vec<StashItem>) -> StashTab {
        StashTab {
            width,
            height,
            items,
            decoration: TabDecoration::default(),
        }
    }

    fn stash(tabs: Vec<StashTab>) -> TransferStash {
        TransferStash {
            version: ContainerVersion::new(11).unwrap(),
            mod_name: String::new(),
            expansion_status: 7,
            tabs,
        }
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> CellRect {
        CellRect {
            x,
            y,
            width,
            height,
        }
    }

    fn at(x: i32, y: i32) -> GridPos {
        GridPos { x, y }
    }

    const TAB0: TabIndex = TabIndex::new(0);
    const NOW: Timestamp = Timestamp::from_unix_seconds(1_756_900_000);

    #[test]
    fn occupancy_resolves_each_item_to_its_cells() {
        let tab = tab(
            10,
            19,
            vec![placed(CLUSTER, 1.0, 6.0), placed(LEGS, 3.0, 0.0)],
        );
        assert_eq!(
            occupancy(&tab, &table()).unwrap(),
            vec![rect(1, 6, 1, 2), rect(3, 0, 2, 3)]
        );
    }

    #[test]
    fn occupancy_refuses_unknown_footprints_and_bad_positions() {
        let unknown = tab(10, 19, vec![placed(MYSTERY, 0.0, 0.0)]);
        assert_eq!(
            occupancy(&unknown, &table()).unwrap_err(),
            OccupancyError::UnknownFootprint {
                index: ItemIndex::new(0),
                base_name: MYSTERY.into()
            }
        );
        for (x, y) in [(0.5, 0.0), (-1.0, 0.0), (0.0, 70_000.0), (f32::NAN, 0.0)] {
            let bad = tab(
                10,
                19,
                vec![placed(CLUSTER, 0.0, 0.0), placed(CLUSTER, x, y)],
            );
            assert_eq!(
                occupancy(&bad, &table()).unwrap_err(),
                OccupancyError::InvalidPosition {
                    index: ItemIndex::new(1)
                },
                "({x},{y})"
            );
        }
        let huge = tab(70_000, 19, vec![]);
        assert_eq!(
            occupancy(&huge, &table()).unwrap_err(),
            OccupancyError::GridTooLarge {
                width: 70_000,
                height: 19
            }
        );
    }

    #[test]
    fn find_slot_takes_the_first_fit_column_by_column() {
        let footprints = table();
        let legs = footprints.footprint(&item(LEGS)).unwrap();
        let empty = tab(10, 19, vec![]);
        assert_eq!(
            find_slot(&empty, legs, &footprints).unwrap(),
            Some(at(0, 0))
        );

        let column_blocked = tab(4, 4, vec![placed(LEGS, 0.0, 0.0)]);
        assert_eq!(
            find_slot(&column_blocked, legs, &footprints).unwrap(),
            Some(at(2, 0))
        );

        let full = tab(2, 3, vec![placed(LEGS, 0.0, 0.0)]);
        assert_eq!(find_slot(&full, legs, &footprints).unwrap(), None);
    }

    #[test]
    fn can_place_at_checks_bounds_then_overlap() {
        let footprints = table();
        let cluster = footprints.footprint(&item(CLUSTER)).unwrap();
        let tab = tab(10, 19, vec![placed(LEGS, 0.0, 0.0)]);
        assert!(can_place_at(&tab, cluster, at(2, 0), &footprints).unwrap());
        assert!(!can_place_at(&tab, cluster, at(1, 2), &footprints).unwrap());
        assert!(!can_place_at(&tab, cluster, at(9, 18), &footprints).unwrap());
        assert!(!can_place_at(&tab, cluster, at(-1, 0), &footprints).unwrap());
        assert!(can_place_at(&tab, cluster, at(9, 17), &footprints).unwrap());
    }

    #[test]
    fn vault_then_place_round_trips_the_item() {
        let footprints = table();
        let mut stash = stash(vec![tab(10, 19, vec![placed(CLUSTER, 1.0, 6.0)])]);
        let mut store = VaultStore::new();

        let id = vault_from_stash(&mut stash, TAB0, ItemIndex::new(0), &mut store, NOW).unwrap();
        assert!(stash.tabs[0].items.is_empty());
        let stored = store.get(id).unwrap();
        assert_eq!(stored.item(), &item(CLUSTER));
        assert_eq!(stored.origin(), &ItemOrigin::TransferStash { tab: TAB0 });
        assert_eq!(stored.stored_at(), NOW);

        let pos = place_in_stash(&mut store, id, &mut stash, TAB0, &footprints).unwrap();
        assert_eq!(pos, at(0, 0));
        assert!(store.is_empty());
        assert_eq!(stash.tabs[0].items, vec![placed(CLUSTER, 0.0, 0.0)]);
    }

    #[test]
    fn place_at_lands_exactly_where_asked() {
        let footprints = table();
        let mut stash = stash(vec![tab(10, 19, vec![placed(LEGS, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let id = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);

        place_in_stash_at(&mut store, id, &mut stash, TAB0, at(9, 17), &footprints).unwrap();
        assert!(store.is_empty());
        assert_eq!(stash.tabs[0].items[1], placed(CLUSTER, 9.0, 17.0));
    }

    #[test]
    fn vault_refuses_missing_tabs_and_items_without_touching_the_store() {
        let mut stash = stash(vec![tab(10, 19, vec![placed(CLUSTER, 1.0, 6.0)])]);
        let mut store = VaultStore::new();
        assert_eq!(
            vault_from_stash(
                &mut stash,
                TabIndex::new(3),
                ItemIndex::new(0),
                &mut store,
                NOW
            ),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
        assert_eq!(
            vault_from_stash(&mut stash, TAB0, ItemIndex::new(1), &mut store, NOW),
            Err(TransferError::NoSuchItem {
                tab: TAB0,
                index: ItemIndex::new(1)
            })
        );
        assert!(store.is_empty());
        assert_eq!(stash.tabs[0].items.len(), 1);
        assert_eq!(
            store.add(item(CLUSTER), ItemOrigin::Unknown, NOW),
            StoredItemId::new(1)
        );
    }

    #[test]
    fn failed_placements_leave_the_store_and_stash_unchanged() {
        let footprints = table();
        let mut stash = stash(vec![tab(2, 3, vec![placed(LEGS, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let legs = store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        let mystery = store.add(item(MYSTERY), ItemOrigin::Unknown, NOW);
        let before = (store.clone(), stash.clone());

        assert_eq!(
            place_in_stash(&mut store, legs, &mut stash, TAB0, &footprints),
            Err(TransferError::NoRoom {
                tab: TAB0,
                footprint: Footprint {
                    width: 2,
                    height: 3
                }
            })
        );
        assert_eq!(
            place_in_stash(&mut store, legs, &mut stash, TabIndex::new(9), &footprints),
            Err(TransferError::NoSuchTab(TabIndex::new(9)))
        );
        assert_eq!(
            place_in_stash(
                &mut store,
                StoredItemId::new(42),
                &mut stash,
                TAB0,
                &footprints
            ),
            Err(TransferError::NoSuchStoredItem(StoredItemId::new(42)))
        );
        assert_eq!(
            place_in_stash(&mut store, mystery, &mut stash, TAB0, &footprints),
            Err(TransferError::UnknownFootprint {
                id: mystery,
                base_name: MYSTERY.into()
            })
        );
        assert_eq!(
            place_in_stash_at(&mut store, legs, &mut stash, TAB0, at(0, 0), &footprints),
            Err(TransferError::Occupied {
                tab: TAB0,
                pos: at(0, 0)
            })
        );
        assert_eq!(
            place_in_stash_at(&mut store, legs, &mut stash, TAB0, at(1, 0), &footprints),
            Err(TransferError::OutOfBounds {
                tab: TAB0,
                pos: at(1, 0),
                footprint: Footprint {
                    width: 2,
                    height: 3
                }
            })
        );
        assert_eq!((store, stash), before);
    }

    #[test]
    fn an_unresolvable_occupant_blocks_placement_loudly() {
        let footprints = table();
        let mut stash = stash(vec![tab(10, 19, vec![placed(MYSTERY, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let id = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
        assert_eq!(
            place_in_stash(&mut store, id, &mut stash, TAB0, &footprints),
            Err(TransferError::Occupancy(OccupancyError::UnknownFootprint {
                index: ItemIndex::new(0),
                base_name: MYSTERY.into()
            }))
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn cells_convert_to_and_from_the_file_representation_exactly() {
        for cell in [0, 1, 9, 18, MAX_CELLS] {
            assert_eq!(cell_from_f32(cell_to_f32(cell)), Some(cell));
        }
        assert_eq!(cell_from_f32(0.25), None);
        assert_eq!(cell_from_f32(-0.0), Some(0));
    }
}
