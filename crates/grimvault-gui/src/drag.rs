//! Drag-and-drop as data: where a drag began, where it may land, what
//! move that means, and applying it — every move through
//! `grimvault_core::transfer`, so a tab is never edited by hand. A
//! rearrangement within the stash is a vault into a scratch store
//! followed by a placement, and a placement that fails restores the
//! stash to the bytes it had, so a snapped-back drag leaves nothing
//! dirty.

use egui::Vec2;
use grimvault_core::gamedata::Footprint;
use grimvault_core::gst::{ReagentStorage, TransferStash};
use grimvault_core::item::Item;
use grimvault_core::reagents::{ReagentKind, ReagentKinds};
use grimvault_core::store::{StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{
    self, Footprints, ItemIndex, ReagentIndex, TabIndex, TransferError,
};
use thiserror::Error;
use univault_engine::grid::{CellRect, fits};
use univault_engine::ids::GridPos;

/// Where a drag was lifted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSource {
    Stash {
        tab: TabIndex,
        index: ItemIndex,
    },
    Store(StoredItemId),
    /// A row of the component / crafting-material storage; `count` is
    /// how many the drag carries (the whole entry, or the amount the
    /// user chose).
    Reagent {
        index: ReagentIndex,
        count: u32,
    },
}

/// A drag in flight. The item stays where it was (painted dimmed)
/// until the drop commits; `grab` is the pointer's offset from the
/// item's top-left corner so it hangs where it was picked up.
#[derive(Clone, Debug, PartialEq)]
pub struct DragState {
    pub source: DragSource,
    pub item: Item,
    pub footprint: Footprint,
    pub grab: Vec2,
}

/// Where the pointer is over this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropTarget {
    /// A cell of a stash tab's grid.
    StashCell { tab: TabIndex, cell: GridPos },
    /// A tab's header: first fit in that tab.
    StashTab(TabIndex),
    /// The store pane: filed by type, no cells.
    Store,
    /// One of the storage's tabs: merged by record, no cells. The tab
    /// is only where the pointer is — the storage is one file and the
    /// record decides the tab.
    Reagents(ReagentKind),
}

/// Whether a footprint can land at a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Fits,
    Blocked,
    /// The tab holds an item whose footprint is unknown, so nothing
    /// can be placed in it.
    Unresolvable,
}

/// The move a drop asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Vault {
        tab: TabIndex,
        index: ItemIndex,
    },
    PlaceAt {
        id: StoredItemId,
        tab: TabIndex,
        cell: GridPos,
    },
    PlaceFirstFit {
        id: StoredItemId,
        tab: TabIndex,
    },
    RearrangeAt {
        from: TabIndex,
        index: ItemIndex,
        to: TabIndex,
        cell: GridPos,
    },
    RearrangeFirstFit {
        from: TabIndex,
        index: ItemIndex,
        to: TabIndex,
    },
    /// Storage → store.
    VaultReagent {
        index: ReagentIndex,
        count: u32,
    },
    /// Store → storage.
    StoreReagent {
        id: StoredItemId,
    },
    /// Stash → storage.
    StashReagent {
        tab: TabIndex,
        index: ItemIndex,
    },
    /// Storage → stash at a cell.
    ReagentToStashAt {
        index: ReagentIndex,
        count: u32,
        tab: TabIndex,
        cell: GridPos,
    },
    /// Storage → stash, first fit.
    ReagentToStashFirstFit {
        index: ReagentIndex,
        count: u32,
        tab: TabIndex,
    },
    /// Dropped back where it already is.
    Stay,
}

/// What [`plan`] decided for a source dropped on a target.
#[must_use]
pub fn plan(source: DragSource, target: DropTarget) -> Move {
    match (source, target) {
        (DragSource::Stash { tab, index }, DropTarget::Store) => Move::Vault { tab, index },
        (DragSource::Stash { tab: from, index }, DropTarget::StashCell { tab: to, cell }) => {
            Move::RearrangeAt {
                from,
                index,
                to,
                cell,
            }
        }
        (DragSource::Stash { tab: from, index }, DropTarget::StashTab(to)) => {
            if from == to {
                Move::Stay
            } else {
                Move::RearrangeFirstFit { from, index, to }
            }
        }
        (DragSource::Stash { tab, index }, DropTarget::Reagents(_)) => {
            Move::StashReagent { tab, index }
        }
        (DragSource::Store(id), DropTarget::StashCell { tab, cell }) => {
            Move::PlaceAt { id, tab, cell }
        }
        (DragSource::Store(id), DropTarget::StashTab(tab)) => Move::PlaceFirstFit { id, tab },
        (DragSource::Store(_), DropTarget::Store)
        | (DragSource::Reagent { .. }, DropTarget::Reagents(_)) => Move::Stay,
        (DragSource::Store(id), DropTarget::Reagents(_)) => Move::StoreReagent { id },
        (DragSource::Reagent { index, count }, DropTarget::Store) => {
            Move::VaultReagent { index, count }
        }
        (DragSource::Reagent { index, count }, DropTarget::StashCell { tab, cell }) => {
            Move::ReagentToStashAt {
                index,
                count,
                tab,
                cell,
            }
        }
        (DragSource::Reagent { index, count }, DropTarget::StashTab(tab)) => {
            Move::ReagentToStashFirstFit { index, count, tab }
        }
    }
}

/// What a double-click asks for: a stash item or a storage row vaults,
/// a stored item takes the first fit in the current tab.
#[must_use]
pub fn plan_double_click(source: DragSource, current_tab: TabIndex) -> Move {
    match source {
        DragSource::Stash { tab, index } => Move::Vault { tab, index },
        DragSource::Store(id) => Move::PlaceFirstFit {
            id,
            tab: current_tab,
        },
        DragSource::Reagent { index, count } => Move::VaultReagent { index, count },
    }
}

/// Preview verdict for `footprint` at `cell` over a tab whose
/// occupants are `occupied` (the dragged item itself excluded by the
/// caller). `resolvable` is false when any occupant's footprint is
/// unknown.
#[must_use]
pub fn fit_at(
    occupied: &[CellRect],
    resolvable: bool,
    footprint: Footprint,
    cell: GridPos,
    cols: i32,
    rows: i32,
) -> Fit {
    if !resolvable {
        return Fit::Unresolvable;
    }
    let candidate = CellRect {
        x: cell.x,
        y: cell.y,
        width: footprint.width,
        height: footprint.height,
    };
    if fits(occupied, candidate, cols, rows) {
        Fit::Fits
    } else {
        Fit::Blocked
    }
}

/// Preview verdict for a drag over one of the storage's tabs: a stash
/// or store item lands if its record is storable; a storage row is
/// already there.
#[must_use]
pub fn fit_in_reagents(source: DragSource, kind: Option<ReagentKind>) -> Fit {
    match source {
        DragSource::Stash { .. } | DragSource::Store(_) => {
            if kind.is_some() {
                Fit::Fits
            } else {
                Fit::Blocked
            }
        }
        DragSource::Reagent { .. } => Fit::Blocked,
    }
}

/// What a move did, for the shell to mark dirty and order writes by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Stash → store: the store is the destination, written first.
    Vaulted(StoredItemId),
    /// Store → stash at `GridPos`: the stash is the destination.
    Placed(GridPos),
    /// Stash → stash: only the stash changed.
    Rearranged(GridPos),
    /// Storage → store: the store is the destination.
    ReagentVaulted(StoredItemId),
    /// Store → storage: the storage is the destination.
    ReagentStored,
    /// Stash → storage: the storage is the destination.
    ReagentStashed,
    /// Storage → stash at `GridPos`: the stash is the destination.
    ReagentPlaced(GridPos),
    Unmoved,
}

/// Why a move could not be applied.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplyError {
    #[error(transparent)]
    Transfer(#[from] TransferError),
    /// The move needs `reagents.gst`, which is not open.
    #[error("reagents.gst is not open, so components and crafting materials cannot be moved")]
    NoReagentStorage,
}

/// The documents a move may touch; `reagents` is `None` while
/// `reagents.gst` is absent or unusable.
pub struct Targets<'a> {
    pub stash: &'a mut TransferStash,
    pub store: &'a mut VaultStore,
    pub reagents: Option<&'a mut ReagentStorage>,
}

/// Performs `mv`. On any error every document is exactly as it was.
///
/// # Errors
/// The underlying [`TransferError`], or [`ApplyError::NoReagentStorage`].
pub fn apply(
    mv: Move,
    targets: Targets<'_>,
    facts: &(impl Footprints + ReagentKinds),
    now: Timestamp,
) -> Result<Applied, ApplyError> {
    let Targets {
        stash,
        store,
        reagents,
    } = targets;
    match mv {
        Move::Vault { tab, index } => {
            Ok(transfer::vault_from_stash(stash, tab, index, store, now).map(Applied::Vaulted)?)
        }
        Move::PlaceAt { id, tab, cell } => {
            transfer::place_in_stash_at(store, id, stash, tab, cell, facts)?;
            Ok(Applied::Placed(cell))
        }
        Move::PlaceFirstFit { id, tab } => {
            Ok(transfer::place_in_stash(store, id, stash, tab, facts).map(Applied::Placed)?)
        }
        Move::RearrangeAt {
            from,
            index,
            to,
            cell,
        } => rearrange(stash, from, index, now, |scratch, id, stash| {
            transfer::place_in_stash_at(scratch, id, stash, to, cell, facts).map(|()| cell)
        }),
        Move::RearrangeFirstFit { from, index, to } => {
            rearrange(stash, from, index, now, |scratch, id, stash| {
                transfer::place_in_stash(scratch, id, stash, to, facts)
            })
        }
        Move::VaultReagent { index, count } => {
            let storage = reagents.ok_or(ApplyError::NoReagentStorage)?;
            Ok(
                transfer::vault_from_reagents(storage, index, count, store, now)
                    .map(Applied::ReagentVaulted)?,
            )
        }
        Move::StoreReagent { id } => {
            let storage = reagents.ok_or(ApplyError::NoReagentStorage)?;
            transfer::place_in_reagents(store, id, storage, facts)?;
            Ok(Applied::ReagentStored)
        }
        Move::StashReagent { tab, index } => {
            let storage = reagents.ok_or(ApplyError::NoReagentStorage)?;
            transfer::stash_to_reagents(stash, tab, index, storage, facts)?;
            Ok(Applied::ReagentStashed)
        }
        Move::ReagentToStashAt {
            index,
            count,
            tab,
            cell,
        } => {
            let storage = reagents.ok_or(ApplyError::NoReagentStorage)?;
            transfer::reagents_to_stash_at(storage, index, count, stash, tab, cell, facts)?;
            Ok(Applied::ReagentPlaced(cell))
        }
        Move::ReagentToStashFirstFit { index, count, tab } => {
            let storage = reagents.ok_or(ApplyError::NoReagentStorage)?;
            Ok(
                transfer::reagents_to_stash(storage, index, count, stash, tab, facts)
                    .map(Applied::ReagentPlaced)?,
            )
        }
        Move::Stay => Ok(Applied::Unmoved),
    }
}

/// Lifts the item into a scratch store and lets `place` put it down;
/// a refused placement restores the stash wholesale, so the item's
/// bytes, position, and order in its tab are untouched.
fn rearrange(
    stash: &mut TransferStash,
    from: TabIndex,
    index: ItemIndex,
    now: Timestamp,
    place: impl FnOnce(
        &mut VaultStore,
        StoredItemId,
        &mut TransferStash,
    ) -> Result<GridPos, TransferError>,
) -> Result<Applied, ApplyError> {
    let before = stash.clone();
    let mut scratch = VaultStore::new();
    let lifted = transfer::vault_from_stash(stash, from, index, &mut scratch, now)?;
    match place(&mut scratch, lifted, stash) {
        Ok(cell) => Ok(Applied::Rearranged(cell)),
        Err(error) => {
            *stash = before;
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use grimvault_core::block::{StashTab, TabDecoration};
    use grimvault_core::gst::{ReagentEntry, ReagentStorageVersion};
    use grimvault_core::item::{ContainerVersion, StashItem};
    use grimvault_core::store::ItemOrigin;

    use super::*;

    const CLUSTER: &str = "records/items/materia/a01_aethercluster.dbr";
    const LEGS: &str = "records/items/gearlegs/l01.dbr";
    const MYSTERY: &str = "records/items/mystery.dbr";

    struct Table(HashMap<&'static str, Footprint>);

    impl Footprints for Table {
        fn footprint(&self, item: &Item) -> Option<Footprint> {
            self.0.get(item.base_name.as_str()).copied()
        }
    }

    impl ReagentKinds for Table {
        fn reagent_kind(&self, item: &Item) -> Option<ReagentKind> {
            (item.base_name == CLUSTER).then_some(ReagentKind::Component)
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

    fn storage(entries: Vec<(&str, u32)>) -> ReagentStorage {
        ReagentStorage {
            version: ReagentStorageVersion::new(1).unwrap(),
            mod_name: String::new(),
            entries: entries
                .into_iter()
                .map(|(record, count)| ReagentEntry {
                    record: record.into(),
                    count,
                })
                .collect(),
        }
    }

    fn at(x: i32, y: i32) -> GridPos {
        GridPos { x, y }
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> CellRect {
        CellRect {
            x,
            y,
            width,
            height,
        }
    }

    const TAB0: TabIndex = TabIndex::new(0);
    const TAB1: TabIndex = TabIndex::new(1);
    const FIRST: ItemIndex = ItemIndex::new(0);
    const ROW0: ReagentIndex = ReagentIndex::new(0);
    const NOW: Timestamp = Timestamp::from_unix_seconds(1_756_900_000);

    fn apply_to(
        mv: Move,
        stash: &mut TransferStash,
        store: &mut VaultStore,
        reagents: Option<&mut ReagentStorage>,
    ) -> Result<Applied, ApplyError> {
        apply(
            mv,
            Targets {
                stash,
                store,
                reagents,
            },
            &table(),
            NOW,
        )
    }

    #[test]
    fn plans_follow_the_source_and_target() {
        let from_stash = DragSource::Stash {
            tab: TAB0,
            index: FIRST,
        };
        let from_store = DragSource::Store(StoredItemId::new(4));
        assert_eq!(
            plan(from_stash, DropTarget::Store),
            Move::Vault {
                tab: TAB0,
                index: FIRST
            }
        );
        assert_eq!(
            plan(
                from_stash,
                DropTarget::StashCell {
                    tab: TAB1,
                    cell: at(2, 3)
                }
            ),
            Move::RearrangeAt {
                from: TAB0,
                index: FIRST,
                to: TAB1,
                cell: at(2, 3)
            }
        );
        assert_eq!(plan(from_stash, DropTarget::StashTab(TAB0)), Move::Stay);
        assert_eq!(
            plan(from_stash, DropTarget::StashTab(TAB1)),
            Move::RearrangeFirstFit {
                from: TAB0,
                index: FIRST,
                to: TAB1
            }
        );
        assert_eq!(
            plan(
                from_store,
                DropTarget::StashCell {
                    tab: TAB0,
                    cell: at(1, 1)
                }
            ),
            Move::PlaceAt {
                id: StoredItemId::new(4),
                tab: TAB0,
                cell: at(1, 1)
            }
        );
        assert_eq!(
            plan(from_store, DropTarget::StashTab(TAB1)),
            Move::PlaceFirstFit {
                id: StoredItemId::new(4),
                tab: TAB1
            }
        );
        assert_eq!(plan(from_store, DropTarget::Store), Move::Stay);
        assert_eq!(
            plan_double_click(from_store, TAB1),
            Move::PlaceFirstFit {
                id: StoredItemId::new(4),
                tab: TAB1
            }
        );
        assert_eq!(
            plan_double_click(from_stash, TAB1),
            Move::Vault {
                tab: TAB0,
                index: FIRST
            }
        );
    }

    #[test]
    fn plans_involving_the_storage_cover_every_pairing() {
        let from_stash = DragSource::Stash {
            tab: TAB0,
            index: FIRST,
        };
        let from_store = DragSource::Store(StoredItemId::new(4));
        let from_row = DragSource::Reagent {
            index: ROW0,
            count: 5,
        };
        for kind in ReagentKind::ALL {
            assert_eq!(
                plan(from_stash, DropTarget::Reagents(kind)),
                Move::StashReagent {
                    tab: TAB0,
                    index: FIRST
                }
            );
            assert_eq!(
                plan(from_store, DropTarget::Reagents(kind)),
                Move::StoreReagent {
                    id: StoredItemId::new(4)
                }
            );
            assert_eq!(plan(from_row, DropTarget::Reagents(kind)), Move::Stay);
        }
        assert_eq!(
            plan(from_row, DropTarget::Store),
            Move::VaultReagent {
                index: ROW0,
                count: 5
            }
        );
        assert_eq!(
            plan(
                from_row,
                DropTarget::StashCell {
                    tab: TAB1,
                    cell: at(2, 3)
                }
            ),
            Move::ReagentToStashAt {
                index: ROW0,
                count: 5,
                tab: TAB1,
                cell: at(2, 3)
            }
        );
        assert_eq!(
            plan(from_row, DropTarget::StashTab(TAB1)),
            Move::ReagentToStashFirstFit {
                index: ROW0,
                count: 5,
                tab: TAB1
            }
        );
        assert_eq!(
            plan_double_click(from_row, TAB1),
            Move::VaultReagent {
                index: ROW0,
                count: 5
            }
        );
    }

    #[test]
    fn fit_previews_bounds_overlap_and_unresolvable_tabs() {
        let occupied = [rect(0, 0, 2, 3)];
        let cluster = Footprint {
            width: 1,
            height: 2,
        };
        assert_eq!(
            fit_at(&occupied, true, cluster, at(2, 0), 10, 19),
            Fit::Fits
        );
        assert_eq!(
            fit_at(&occupied, true, cluster, at(1, 2), 10, 19),
            Fit::Blocked
        );
        assert_eq!(
            fit_at(&occupied, true, cluster, at(9, 18), 10, 19),
            Fit::Blocked
        );
        assert_eq!(
            fit_at(&occupied, false, cluster, at(5, 5), 10, 19),
            Fit::Unresolvable
        );
    }

    #[test]
    fn storage_tabs_accept_storable_records_only() {
        let from_stash = DragSource::Stash {
            tab: TAB0,
            index: FIRST,
        };
        let from_store = DragSource::Store(StoredItemId::new(4));
        let from_row = DragSource::Reagent {
            index: ROW0,
            count: 1,
        };
        assert_eq!(
            fit_in_reagents(from_stash, Some(ReagentKind::Component)),
            Fit::Fits
        );
        assert_eq!(
            fit_in_reagents(from_store, Some(ReagentKind::CraftingMaterial)),
            Fit::Fits
        );
        assert_eq!(fit_in_reagents(from_stash, None), Fit::Blocked);
        assert_eq!(fit_in_reagents(from_store, None), Fit::Blocked);
        assert_eq!(
            fit_in_reagents(from_row, Some(ReagentKind::Component)),
            Fit::Blocked
        );
    }

    #[test]
    fn vault_and_place_go_through_transfer() {
        let mut stash = stash(vec![tab(10, 19, vec![placed(CLUSTER, 1.0, 6.0)])]);
        let mut store = VaultStore::new();
        let vaulted = apply_to(
            Move::Vault {
                tab: TAB0,
                index: FIRST,
            },
            &mut stash,
            &mut store,
            None,
        )
        .unwrap();
        let Applied::Vaulted(id) = vaulted else {
            panic!("expected a vault, got {vaulted:?}");
        };
        assert!(stash.tabs[0].items.is_empty());
        assert_eq!(store.len(), 1);

        assert_eq!(
            apply_to(
                Move::PlaceAt {
                    id,
                    tab: TAB0,
                    cell: at(4, 4)
                },
                &mut stash,
                &mut store,
                None,
            )
            .unwrap(),
            Applied::Placed(at(4, 4))
        );
        assert!(store.is_empty());
        assert_eq!(stash.tabs[0].items, vec![placed(CLUSTER, 4.0, 4.0)]);
    }

    #[test]
    fn first_fit_placement_lands_at_the_first_open_cells() {
        let mut stash = stash(vec![tab(4, 4, vec![placed(LEGS, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let id = store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        assert_eq!(
            apply_to(
                Move::PlaceFirstFit { id, tab: TAB0 },
                &mut stash,
                &mut store,
                None,
            )
            .unwrap(),
            Applied::Placed(at(2, 0))
        );
    }

    #[test]
    fn a_rearrangement_may_overlap_its_own_old_cells_and_leaves_the_store_alone() {
        let mut stash = stash(vec![tab(
            10,
            19,
            vec![placed(LEGS, 0.0, 0.0), placed(CLUSTER, 5.0, 5.0)],
        )]);
        let mut store = VaultStore::new();
        let before = store.clone();
        assert_eq!(
            apply_to(
                Move::RearrangeAt {
                    from: TAB0,
                    index: FIRST,
                    to: TAB0,
                    cell: at(1, 1)
                },
                &mut stash,
                &mut store,
                None,
            )
            .unwrap(),
            Applied::Rearranged(at(1, 1))
        );
        assert_eq!(store, before);
        assert_eq!(
            stash.tabs[0].items,
            vec![placed(CLUSTER, 5.0, 5.0), placed(LEGS, 1.0, 1.0)]
        );
    }

    #[test]
    fn a_refused_rearrangement_restores_the_stash_exactly() {
        let mut stash = stash(vec![
            tab(
                10,
                19,
                vec![placed(LEGS, 0.0, 0.0), placed(CLUSTER, 5.0, 5.0)],
            ),
            tab(2, 2, vec![]),
        ]);
        let before = stash.clone();
        let mut store = VaultStore::new();
        let blocked = apply_to(
            Move::RearrangeAt {
                from: TAB0,
                index: FIRST,
                to: TAB0,
                cell: at(5, 4),
            },
            &mut stash,
            &mut store,
            None,
        );
        assert_eq!(
            blocked,
            Err(ApplyError::Transfer(TransferError::Occupied {
                tab: TAB0,
                pos: at(5, 4)
            }))
        );
        assert_eq!(stash, before);
        let no_room = apply_to(
            Move::RearrangeFirstFit {
                from: TAB0,
                index: FIRST,
                to: TAB1,
            },
            &mut stash,
            &mut store,
            None,
        );
        assert!(
            matches!(
                no_room,
                Err(ApplyError::Transfer(TransferError::NoRoom {
                    tab: TAB1,
                    ..
                }))
            ),
            "{no_room:?}"
        );
        assert_eq!(stash, before);
        assert!(store.is_empty());
    }

    #[test]
    fn an_unresolvable_tab_refuses_moves_without_side_effects() {
        let mut stash = stash(vec![tab(10, 19, vec![placed(MYSTERY, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let id = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
        let before = (stash.clone(), store.clone());
        assert!(matches!(
            apply_to(
                Move::PlaceAt {
                    id,
                    tab: TAB0,
                    cell: at(3, 3)
                },
                &mut stash,
                &mut store,
                None,
            ),
            Err(ApplyError::Transfer(TransferError::Occupancy(_)))
        ));
        assert_eq!((stash, store), before);
    }

    #[test]
    fn storage_moves_go_through_transfer_in_every_direction() {
        let mut stash = stash(vec![tab(10, 19, vec![placed(CLUSTER, 1.0, 6.0)])]);
        stash.tabs[0].items[0].item.stack_count = 3;
        let mut store = VaultStore::new();
        let mut storage = storage(vec![(CLUSTER, 10)]);

        assert_eq!(
            apply_to(
                Move::StashReagent {
                    tab: TAB0,
                    index: FIRST
                },
                &mut stash,
                &mut store,
                Some(&mut storage),
            )
            .unwrap(),
            Applied::ReagentStashed
        );
        assert!(stash.tabs[0].items.is_empty());
        assert_eq!(storage.entries[0].count, 13);

        let vaulted = apply_to(
            Move::VaultReagent {
                index: ROW0,
                count: 4,
            },
            &mut stash,
            &mut store,
            Some(&mut storage),
        )
        .unwrap();
        let Applied::ReagentVaulted(id) = vaulted else {
            panic!("expected a reagent vault, got {vaulted:?}");
        };
        assert_eq!(store.get(id).unwrap().item().stack_count, 4);
        assert_eq!(storage.entries[0].count, 9);

        assert_eq!(
            apply_to(
                Move::StoreReagent { id },
                &mut stash,
                &mut store,
                Some(&mut storage),
            )
            .unwrap(),
            Applied::ReagentStored
        );
        assert!(store.is_empty());
        assert_eq!(storage.entries[0].count, 13);

        assert_eq!(
            apply_to(
                Move::ReagentToStashAt {
                    index: ROW0,
                    count: 2,
                    tab: TAB0,
                    cell: at(3, 3)
                },
                &mut stash,
                &mut store,
                Some(&mut storage),
            )
            .unwrap(),
            Applied::ReagentPlaced(at(3, 3))
        );
        assert_eq!(
            apply_to(
                Move::ReagentToStashFirstFit {
                    index: ROW0,
                    count: 11,
                    tab: TAB0
                },
                &mut stash,
                &mut store,
                Some(&mut storage),
            )
            .unwrap(),
            Applied::ReagentPlaced(at(0, 0))
        );
        assert!(storage.entries.is_empty());
        assert_eq!(stash.tabs[0].items.len(), 2);
        assert_eq!(stash.tabs[0].items[1].item.stack_count, 11);
    }

    #[test]
    fn storage_moves_need_an_open_storage_and_refuse_non_reagents() {
        let mut stash = stash(vec![tab(10, 19, vec![placed(LEGS, 0.0, 0.0)])]);
        let mut store = VaultStore::new();
        let legs = store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        let mut storage = storage(vec![(CLUSTER, 1)]);
        let before = (stash.clone(), store.clone(), storage.clone());

        assert_eq!(
            apply_to(
                Move::VaultReagent {
                    index: ROW0,
                    count: 1
                },
                &mut stash,
                &mut store,
                None,
            ),
            Err(ApplyError::NoReagentStorage)
        );
        assert_eq!(
            apply_to(
                Move::StoreReagent { id: legs },
                &mut stash,
                &mut store,
                Some(&mut storage),
            ),
            Err(ApplyError::Transfer(TransferError::NotAReagent {
                id: legs,
                base_name: LEGS.into()
            }))
        );
        assert_eq!(
            apply_to(
                Move::StashReagent {
                    tab: TAB0,
                    index: FIRST
                },
                &mut stash,
                &mut store,
                Some(&mut storage),
            ),
            Err(ApplyError::Transfer(TransferError::StashItemNotAReagent {
                tab: TAB0,
                index: FIRST,
                base_name: LEGS.into()
            }))
        );
        assert_eq!((stash, store, storage), before);
    }
}
