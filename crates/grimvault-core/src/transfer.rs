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
//!
//! A character's own stash (`player.gdc` block 4) is a list of the same
//! tabs, and its inventory sacks (block 3) work the same way with
//! integer cells ([`SackItem`]), except that the save stores no sack
//! dimensions: they come from the game's UI records, recorded once in
//! [`SackDimensions`].
//!
//! The component / crafting-material storage (`reagents.gst`,
//! [`ReagentStorage`]) has no grid at all: it holds one counted entry
//! per record. Moving into it keeps only the record and the stack
//! count — that is all the game keeps — and is allowed only for a
//! record the database flags as storable ([`ReagentKinds`]); moving
//! out of it creates a plain item of that record with the count taken.
//!
//! Every move goes through the store: a container-to-container move
//! is a vault into a store followed by a placement out of it, which is
//! why there is one vault and one placement per container and no
//! pairwise operations.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use univault_engine::grid::{CellRect, find_open_cells, fits};
use univault_engine::ids::{GridPos, RecordId};

use crate::block::StashTab;
use crate::campaign::Campaign;
use crate::gamedata::{Footprint, GameData};
use crate::gdc::{PlayerFile, Realm, Sack};
use crate::gst::{ReagentEntry, ReagentStorage, TransferStash};
use crate::item::{Item, SackItem, StashItem};
use crate::reagents::ReagentKinds;
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

    /// The index as a position in the file's tab list.
    #[must_use]
    pub fn slot(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

impl fmt::Display for TabIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Position of a sack (bag) within a character's inventory, as the
/// file orders them: 0 is the main bag, 1 onward the additional bags.
/// Serializes as the bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SackIndex(u32);

impl SackIndex {
    /// The main bag.
    pub const MAIN: Self = Self(0);

    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The sack's grid size: [`MAIN_SACK`] for the main bag,
    /// [`EXTRA_SACK`] for every other.
    #[must_use]
    pub const fn dimensions(self) -> SackDimensions {
        if self.0 == 0 { MAIN_SACK } else { EXTRA_SACK }
    }

    fn slot(self) -> Option<usize> {
        usize::try_from(self.0).ok()
    }
}

impl fmt::Display for SackIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Grid size of a character sack in cells. The save does not store it.
/// Source: `records/game/gameengine.dbr` lays the main bag out at
/// `UICharWindowInventorySack0DimsX/Y` = 384 × 256 px and every
/// additional bag at `UICharWindowInventorySack1DimsX/Y` = 256 × 256 px
/// (the same sizes `records/ui/character/characterinventory/
/// inventory_grid0.dbr` and `inventory_grid1.dbr` give as
/// `inventoryXSize/YSize`), and an item cell is 32 px — the
/// footprint-from-bitmap rule in `docs/format-references.md`. Checked
/// 2026-09-03 against every sack of the vendored fixture and three real
/// saves: no occupant reaches past these bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SackDimensions {
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

/// The main bag: 384 × 256 px ÷ 32.
pub const MAIN_SACK: SackDimensions = SackDimensions {
    width: 12,
    height: 8,
};

/// Every additional bag: 256 × 256 px ÷ 32.
pub const EXTRA_SACK: SackDimensions = SackDimensions {
    width: 8,
    height: 8,
};

/// Position of an item within a tab's or sack's item list, as the file
/// orders them.
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

/// Position of an entry within the component / crafting-material
/// storage, as the file orders them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReagentIndex(usize);

impl ReagentIndex {
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn value(self) -> usize {
        self.0
    }
}

impl fmt::Display for ReagentIndex {
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
    /// The character's inventory block is not typed, so its sacks
    /// cannot be reached.
    #[error("the character's inventory is not typed")]
    NoInventory,
    #[error("the inventory has no sack {0}")]
    NoSuchSack(SackIndex),
    #[error("sack {sack} has no item {index}")]
    NoSuchSackItem { sack: SackIndex, index: ItemIndex },
    #[error("sack {sack} has no room for a {}x{} item", .footprint.width, .footprint.height)]
    SackNoRoom {
        sack: SackIndex,
        footprint: Footprint,
    },
    #[error("cells at ({},{}) in sack {sack} are occupied", .pos.x, .pos.y)]
    SackOccupied { sack: SackIndex, pos: GridPos },
    #[error("a {}x{} item at ({},{}) does not fit inside sack {sack}", .footprint.width, .footprint.height, .pos.x, .pos.y)]
    SackOutOfBounds {
        sack: SackIndex,
        pos: GridPos,
        footprint: Footprint,
    },
    #[error("the storage has no entry {0}")]
    NoSuchReagent(ReagentIndex),
    /// A move of zero items was asked for.
    #[error("cannot take zero of entry {0}")]
    ZeroReagentCount(ReagentIndex),
    #[error("entry {index} holds {available}, fewer than the {requested} asked for")]
    ReagentCountExceeded {
        index: ReagentIndex,
        requested: u32,
        available: u32,
    },
    /// Merging would push the entry's count past what the file can hold.
    #[error("entry {index} ({record:?}) cannot hold {more} more")]
    ReagentCountOverflow {
        index: ReagentIndex,
        record: String,
        more: u32,
    },
    /// The stored item's record is not one the game keeps in the
    /// storage (no `craftingMaterial` flag, or unknown to the
    /// database).
    #[error("stored item {id} ({base_name:?}) is not a component or crafting material")]
    NotAReagent { id: StoredItemId, base_name: String },
    /// The character's own stash block is not typed, so its tabs
    /// cannot be reached.
    #[error("the character's stash is not typed")]
    NoPlayerStash,
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
    campaign: &Campaign,
    tab: TabIndex,
    index: ItemIndex,
    store: &mut VaultStore,
    at: Timestamp,
) -> Result<StoredItemId, TransferError> {
    let placed = take_from_tabs(&mut stash.tabs, tab, index)?;
    let origin = ItemOrigin::TransferStash {
        campaign: campaign.clone(),
        tab,
    };
    Ok(store.add(placed.item, origin, at))
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
    place_in_tabs(store, id, &mut stash.tabs, tab, footprints)
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
    place_in_tabs_at(store, id, &mut stash.tabs, tab, pos, footprints)
}

/// Removes item `index` from tab `tab` of `player`'s own stash and
/// stores it with [`ItemOrigin::CharacterStash`] under `realm`, the
/// folder the file was read from, which the file itself never names.
///
/// # Errors
/// [`TransferError::NoPlayerStash`], [`TransferError::NoSuchTab`] or
/// [`TransferError::NoSuchItem`]; the store and player are unchanged
/// on error.
pub fn vault_from_player_stash(
    player: &mut PlayerFile,
    realm: Realm,
    tab: TabIndex,
    index: ItemIndex,
    store: &mut VaultStore,
    at: Timestamp,
) -> Result<StoredItemId, TransferError> {
    let name = player.character_name().to_owned();
    let placed = take_from_tabs(player_tabs_mut(player)?, tab, index)?;
    Ok(store.add(
        placed.item,
        ItemOrigin::CharacterStash { realm, name, tab },
        at,
    ))
}

/// Moves stored item `id` into the first free spot of tab `tab` of
/// `player`'s own stash, returning where it landed.
///
/// # Errors
/// Any [`TransferError`]; the store and player are unchanged on error.
pub fn place_in_player_stash(
    store: &mut VaultStore,
    id: StoredItemId,
    player: &mut PlayerFile,
    tab: TabIndex,
    footprints: &impl Footprints,
) -> Result<GridPos, TransferError> {
    place_in_tabs(store, id, player_tabs_mut(player)?, tab, footprints)
}

/// Moves stored item `id` into tab `tab` of `player`'s own stash at
/// exactly `pos`.
///
/// # Errors
/// [`TransferError::OutOfBounds`] or [`TransferError::Occupied`] when
/// the cells are not free, or any other [`TransferError`]; the store
/// and player are unchanged on error.
pub fn place_in_player_stash_at(
    store: &mut VaultStore,
    id: StoredItemId,
    player: &mut PlayerFile,
    tab: TabIndex,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<(), TransferError> {
    place_in_tabs_at(store, id, player_tabs_mut(player)?, tab, pos, footprints)
}

pub(crate) fn player_tabs_mut(player: &mut PlayerFile) -> Result<&mut [StashTab], TransferError> {
    player
        .stash_mut()
        .map(|stash| stash.tabs.as_mut_slice())
        .ok_or(TransferError::NoPlayerStash)
}

fn take_from_tabs(
    tabs: &mut [StashTab],
    tab: TabIndex,
    index: ItemIndex,
) -> Result<StashItem, TransferError> {
    let tab_ref = tab_mut(tabs, tab)?;
    if index.value() >= tab_ref.items.len() {
        return Err(TransferError::NoSuchItem { tab, index });
    }
    Ok(tab_ref.items.remove(index.value()))
}

fn place_in_tabs(
    store: &mut VaultStore,
    id: StoredItemId,
    tabs: &mut [StashTab],
    tab: TabIndex,
    footprints: &impl Footprints,
) -> Result<GridPos, TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let pos = find_slot(tab_ref(tabs, tab)?, footprint, footprints)?
        .ok_or(TransferError::NoRoom { tab, footprint })?;
    move_into_tab(store, id, tabs, tab, pos)?;
    Ok(pos)
}

fn place_in_tabs_at(
    store: &mut VaultStore,
    id: StoredItemId,
    tabs: &mut [StashTab],
    tab: TabIndex,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<(), TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let grid = grid(tab_ref(tabs, tab)?, footprints)?;
    check_placement(&grid, rect_at(pos, footprint)).map_err(|blocked| match blocked {
        Blocked::OutOfBounds => TransferError::OutOfBounds {
            tab,
            pos,
            footprint,
        },
        Blocked::Occupied => TransferError::Occupied { tab, pos },
    })?;
    move_into_tab(store, id, tabs, tab, pos)
}

/// The cells every item in sack `sack` occupies, in item order.
///
/// # Errors
/// [`OccupancyError`] when an occupant's footprint or position cannot
/// be resolved.
pub fn sack_occupancy(
    sack: SackIndex,
    contents: &Sack,
    footprints: &impl Footprints,
) -> Result<Vec<CellRect>, OccupancyError> {
    sack_grid(sack, contents, footprints).map(|grid| grid.occupied)
}

/// First free spot for `footprint` in sack `sack` (columns left to
/// right, each top to bottom); `Ok(None)` when nothing fits.
///
/// # Errors
/// [`OccupancyError`] when the sack's occupants cannot be laid out.
pub fn find_sack_slot(
    sack: SackIndex,
    contents: &Sack,
    footprint: Footprint,
    footprints: &impl Footprints,
) -> Result<Option<GridPos>, OccupancyError> {
    let grid = sack_grid(sack, contents, footprints)?;
    Ok(find_open_cells(
        &grid.occupied,
        footprint.width,
        footprint.height,
        grid.width,
        grid.height,
    ))
}

/// Whether `footprint` at `pos` lies inside sack `sack` and overlaps
/// nothing.
///
/// # Errors
/// [`OccupancyError`] when the sack's occupants cannot be laid out.
pub fn can_place_in_sack_at(
    sack: SackIndex,
    contents: &Sack,
    footprint: Footprint,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<bool, OccupancyError> {
    let grid = sack_grid(sack, contents, footprints)?;
    Ok(check_placement(&grid, rect_at(pos, footprint)).is_ok())
}

/// Removes item `index` from sack `sack` of `player` and stores it with
/// [`ItemOrigin::Character`]. Needs no footprint: the store has no
/// grid.
///
/// # Errors
/// [`TransferError::NoInventory`], [`TransferError::NoSuchSack`] or
/// [`TransferError::NoSuchSackItem`]; the store and player are
/// unchanged on error.
pub fn vault_from_sack(
    player: &mut PlayerFile,
    realm: Realm,
    sack: SackIndex,
    index: ItemIndex,
    store: &mut VaultStore,
    at: Timestamp,
) -> Result<StoredItemId, TransferError> {
    let name = player.character_name().to_owned();
    let contents = sack_mut(player, sack)?;
    if index.value() >= contents.items.len() {
        return Err(TransferError::NoSuchSackItem { sack, index });
    }
    let placed = contents.items.remove(index.value());
    Ok(store.add(placed.item, ItemOrigin::Character { realm, name, sack }, at))
}

/// Moves stored item `id` into the first free spot of sack `sack`,
/// returning where it landed.
///
/// # Errors
/// Any [`TransferError`]; the store and player are unchanged on error.
pub fn place_in_sack(
    store: &mut VaultStore,
    id: StoredItemId,
    player: &mut PlayerFile,
    sack: SackIndex,
    footprints: &impl Footprints,
) -> Result<GridPos, TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let pos = find_sack_slot(sack, sack_ref(player, sack)?, footprint, footprints)?
        .ok_or(TransferError::SackNoRoom { sack, footprint })?;
    move_into_sack(store, id, player, sack, sack_cell(sack, pos, footprint)?)?;
    Ok(pos)
}

/// Moves stored item `id` into sack `sack` at exactly `pos`.
///
/// # Errors
/// [`TransferError::SackOutOfBounds`] or [`TransferError::SackOccupied`]
/// when the cells are not free, or any other [`TransferError`]; the
/// store and player are unchanged on error.
pub fn place_in_sack_at(
    store: &mut VaultStore,
    id: StoredItemId,
    player: &mut PlayerFile,
    sack: SackIndex,
    pos: GridPos,
    footprints: &impl Footprints,
) -> Result<(), TransferError> {
    let footprint = stored_footprint(store, id, footprints)?;
    let grid = sack_grid(sack, sack_ref(player, sack)?, footprints)?;
    check_placement(&grid, rect_at(pos, footprint)).map_err(|blocked| match blocked {
        Blocked::OutOfBounds => TransferError::SackOutOfBounds {
            sack,
            pos,
            footprint,
        },
        Blocked::Occupied => TransferError::SackOccupied { sack, pos },
    })?;
    move_into_sack(store, id, player, sack, sack_cell(sack, pos, footprint)?)
}

/// Takes `count` of storage entry `index` into the store as one stack
/// with [`ItemOrigin::ReagentStorage`], removing the entry when it is
/// emptied.
///
/// # Errors
/// [`TransferError::NoSuchReagent`], [`TransferError::ZeroReagentCount`]
/// or [`TransferError::ReagentCountExceeded`]; the store and storage
/// are unchanged on error.
pub fn vault_from_reagents(
    storage: &mut ReagentStorage,
    campaign: &Campaign,
    index: ReagentIndex,
    count: u32,
    store: &mut VaultStore,
    at: Timestamp,
) -> Result<StoredItemId, TransferError> {
    let entry = reagent_ref(storage, index)?;
    if count == 0 {
        return Err(TransferError::ZeroReagentCount(index));
    }
    if count > entry.count {
        return Err(TransferError::ReagentCountExceeded {
            index,
            requested: count,
            available: entry.count,
        });
    }
    let item = Item {
        base_name: entry.record.clone(),
        stack_count: count,
        ..Item::default()
    };
    let origin = ItemOrigin::ReagentStorage {
        campaign: campaign.clone(),
    };
    let id = store.add(item, origin, at);
    take_from_entry(storage, index, count);
    Ok(id)
}

/// Moves stored item `id` into the storage: its stack count merges into
/// the entry for its record, or a new entry is appended.
///
/// # Errors
/// [`TransferError::NotAReagent`] when the record is not storable,
/// [`TransferError::ReagentCountOverflow`] when the merged count would
/// not fit, or [`TransferError::NoSuchStoredItem`]; the store and
/// storage are unchanged on error.
pub fn place_in_reagents(
    store: &mut VaultStore,
    id: StoredItemId,
    storage: &mut ReagentStorage,
    kinds: &impl ReagentKinds,
) -> Result<(), TransferError> {
    let stored = store.get(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    if kinds.reagent_kind(stored.item()).is_none() {
        return Err(TransferError::NotAReagent {
            id,
            base_name: stored.item().base_name.clone(),
        });
    }
    check_merge(storage, stored.item())?;
    let stored = store.take(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    merge_into_storage(storage, stored.into_item());
    Ok(())
}

/// The count an item contributes to the storage, [`Item::units`].
pub(crate) fn stack_of(item: &Item) -> u32 {
    item.units()
}

/// Whether the entry `item` would merge into can take its stack.
fn check_merge(storage: &ReagentStorage, item: &Item) -> Result<(), TransferError> {
    let more = stack_of(item);
    match storage.position_of(&item.base_name) {
        Some(slot) if storage.entries[slot].count.checked_add(more).is_none() => {
            Err(TransferError::ReagentCountOverflow {
                index: ReagentIndex::new(slot),
                record: item.base_name.clone(),
                more,
            })
        }
        Some(_) | None => Ok(()),
    }
}

/// Merges an item whose record [`check_merge`] accepted; a stack that
/// still overflows here is a defect in that check, so it saturates
/// rather than wrapping.
fn merge_into_storage(storage: &mut ReagentStorage, item: Item) {
    let more = stack_of(&item);
    match storage.position_of(&item.base_name) {
        Some(slot) => {
            let entry = &mut storage.entries[slot];
            entry.count = entry.count.saturating_add(more);
        }
        None => storage.entries.push(ReagentEntry {
            record: item.base_name,
            count: more,
        }),
    }
}

/// Decrements entry `index` by `count` — already checked to be within
/// the entry — and removes the entry once empty.
fn take_from_entry(storage: &mut ReagentStorage, index: ReagentIndex, count: u32) {
    let entry = &mut storage.entries[index.value()];
    entry.count = entry.count.saturating_sub(count);
    if entry.count == 0 {
        storage.entries.remove(index.value());
    }
}

fn reagent_ref(
    storage: &ReagentStorage,
    index: ReagentIndex,
) -> Result<&ReagentEntry, TransferError> {
    storage
        .entries
        .get(index.value())
        .ok_or(TransferError::NoSuchReagent(index))
}

/// A sack cell as the file stores it.
#[derive(Clone, Copy)]
struct SackCell {
    x: u32,
    y: u32,
}

/// A bounds-checked position is non-negative, so this only fails for a
/// position that never passed the check — reported as out of bounds.
fn sack_cell(
    sack: SackIndex,
    pos: GridPos,
    footprint: Footprint,
) -> Result<SackCell, TransferError> {
    match (u32::try_from(pos.x), u32::try_from(pos.y)) {
        (Ok(x), Ok(y)) => Ok(SackCell { x, y }),
        _ => Err(TransferError::SackOutOfBounds {
            sack,
            pos,
            footprint,
        }),
    }
}

fn sack_grid(
    sack: SackIndex,
    contents: &Sack,
    footprints: &impl Footprints,
) -> Result<TabGrid, OccupancyError> {
    let SackDimensions { width, height } = sack.dimensions();
    let (Some(width), Some(height)) = (dimension(width), dimension(height)) else {
        return Err(OccupancyError::GridTooLarge { width, height });
    };
    let occupied = contents
        .items
        .iter()
        .enumerate()
        .map(|(slot, placed)| sack_occupant_rect(ItemIndex::new(slot), placed, footprints))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TabGrid {
        width,
        height,
        occupied,
    })
}

fn sack_occupant_rect(
    index: ItemIndex,
    placed: &SackItem,
    footprints: &impl Footprints,
) -> Result<CellRect, OccupancyError> {
    let footprint =
        footprints
            .footprint(&placed.item)
            .ok_or_else(|| OccupancyError::UnknownFootprint {
                index,
                base_name: placed.item.base_name.clone(),
            })?;
    let (Some(x), Some(y)) = (dimension(placed.x), dimension(placed.y)) else {
        return Err(OccupancyError::InvalidPosition { index });
    };
    Ok(rect_at(GridPos { x, y }, footprint))
}

/// The final, fallible-only-by-lookup step of a sack placement: the
/// sack is resolved before the store is touched, so a missing sack
/// leaves the store intact.
fn move_into_sack(
    store: &mut VaultStore,
    id: StoredItemId,
    player: &mut PlayerFile,
    sack: SackIndex,
    cell: SackCell,
) -> Result<(), TransferError> {
    let contents = sack_mut(player, sack)?;
    let stored = store.take(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    contents.items.push(SackItem {
        item: stored.into_item(),
        x: cell.x,
        y: cell.y,
    });
    Ok(())
}

pub(crate) fn sack_ref(player: &PlayerFile, sack: SackIndex) -> Result<&Sack, TransferError> {
    let sacks = player
        .inventory()
        .ok_or(TransferError::NoInventory)?
        .sacks();
    sack.slot()
        .and_then(|slot| sacks.get(slot))
        .ok_or(TransferError::NoSuchSack(sack))
}

fn sack_mut(player: &mut PlayerFile, sack: SackIndex) -> Result<&mut Sack, TransferError> {
    let sacks = player
        .inventory_mut()
        .ok_or(TransferError::NoInventory)?
        .sacks_mut();
    sack.slot()
        .and_then(|slot| sacks.get_mut(slot))
        .ok_or(TransferError::NoSuchSack(sack))
}

/// A container's grid (stash tab or sack) with every occupant resolved
/// to cells.
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
    tabs: &mut [StashTab],
    tab: TabIndex,
    pos: GridPos,
) -> Result<(), TransferError> {
    let tab_ref = tab_mut(tabs, tab)?;
    let stored = store.take(id).ok_or(TransferError::NoSuchStoredItem(id))?;
    tab_ref.items.push(StashItem {
        item: stored.into_item(),
        x: cell_to_f32(pos.x),
        y: cell_to_f32(pos.y),
    });
    Ok(())
}

pub(crate) fn tab_ref(tabs: &[StashTab], tab: TabIndex) -> Result<&StashTab, TransferError> {
    tab.slot()
        .and_then(|slot| tabs.get(slot))
        .ok_or(TransferError::NoSuchTab(tab))
}

pub(crate) fn tab_mut(
    tabs: &mut [StashTab],
    tab: TabIndex,
) -> Result<&mut StashTab, TransferError> {
    tab.slot()
        .and_then(|slot| tabs.get_mut(slot))
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

        let id = vault_from_stash(
            &mut stash,
            &Campaign::Main,
            TAB0,
            ItemIndex::new(0),
            &mut store,
            NOW,
        )
        .unwrap();
        assert!(stash.tabs[0].items.is_empty());
        let stored = store.get(id).unwrap();
        assert_eq!(stored.item(), &item(CLUSTER));
        assert_eq!(
            stored.origin(),
            &ItemOrigin::TransferStash {
                campaign: Campaign::Main,
                tab: TAB0
            }
        );
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
                &Campaign::Main,
                TabIndex::new(3),
                ItemIndex::new(0),
                &mut store,
                NOW
            ),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
        assert_eq!(
            vault_from_stash(
                &mut stash,
                &Campaign::Main,
                TAB0,
                ItemIndex::new(1),
                &mut store,
                NOW
            ),
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

    mod reagents {
        use super::*;
        use crate::gst::{ReagentEntry, ReagentStorage, ReagentStorageVersion};
        use crate::reagents::ReagentKind;

        const SHARD: &str = "records/items/crafting/materials/craft_aethershard.dbr";

        struct Kinds;

        impl ReagentKinds for Kinds {
            fn reagent_kind(&self, item: &Item) -> Option<ReagentKind> {
                match item.base_name.as_str() {
                    CLUSTER => Some(ReagentKind::Component),
                    SHARD => Some(ReagentKind::CraftingMaterial),
                    _ => None,
                }
            }
        }

        fn entry(record: &str, count: u32) -> ReagentEntry {
            ReagentEntry {
                record: record.into(),
                count,
            }
        }

        fn storage(entries: Vec<ReagentEntry>) -> ReagentStorage {
            ReagentStorage {
                version: ReagentStorageVersion::new(1).unwrap(),
                mod_name: String::new(),
                entries,
            }
        }

        fn stack(base_name: &str, count: u32) -> Item {
            Item {
                base_name: base_name.into(),
                stack_count: count,
                ..Item::default()
            }
        }

        const FIRST: ReagentIndex = ReagentIndex::new(0);

        #[test]
        fn vaulting_part_of_an_entry_decrements_it_and_all_of_it_removes_it() {
            let mut storage = storage(vec![entry(SHARD, 15), entry(CLUSTER, 20)]);
            let mut store = VaultStore::new();

            let id = vault_from_reagents(&mut storage, &Campaign::Main, FIRST, 5, &mut store, NOW)
                .unwrap();
            let stored = store.get(id).unwrap();
            assert_eq!(stored.item(), &stack(SHARD, 5));
            assert_eq!(
                stored.origin(),
                &ItemOrigin::ReagentStorage {
                    campaign: Campaign::Main
                }
            );
            assert_eq!(stored.stored_at(), NOW);
            assert_eq!(storage.entries, vec![entry(SHARD, 10), entry(CLUSTER, 20)]);

            vault_from_reagents(&mut storage, &Campaign::Main, FIRST, 10, &mut store, NOW).unwrap();
            assert_eq!(storage.entries, vec![entry(CLUSTER, 20)]);
            assert_eq!(store.len(), 2);
        }

        #[test]
        fn vaulting_refuses_bad_indexes_and_counts_without_touching_the_store() {
            let mut storage = storage(vec![entry(SHARD, 15)]);
            let mut store = VaultStore::new();
            let before = storage.clone();
            assert_eq!(
                vault_from_reagents(
                    &mut storage,
                    &Campaign::Main,
                    ReagentIndex::new(1),
                    1,
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoSuchReagent(ReagentIndex::new(1)))
            );
            assert_eq!(
                vault_from_reagents(&mut storage, &Campaign::Main, FIRST, 0, &mut store, NOW),
                Err(TransferError::ZeroReagentCount(FIRST))
            );
            assert_eq!(
                vault_from_reagents(&mut storage, &Campaign::Main, FIRST, 16, &mut store, NOW),
                Err(TransferError::ReagentCountExceeded {
                    index: FIRST,
                    requested: 16,
                    available: 15
                })
            );
            assert!(store.is_empty());
            assert_eq!(storage, before);
            assert_eq!(
                store.add(stack(SHARD, 1), ItemOrigin::Unknown, NOW),
                StoredItemId::new(1)
            );
        }

        #[test]
        fn placing_merges_into_the_existing_entry_or_appends() {
            let mut storage = storage(vec![entry(SHARD, 15)]);
            let mut store = VaultStore::new();
            let shards = store.add(stack(SHARD, 5), ItemOrigin::Unknown, NOW);
            let clusters = store.add(stack(CLUSTER, 3), ItemOrigin::Unknown, NOW);
            let single = store.add(stack(CLUSTER, 0), ItemOrigin::Unknown, NOW);

            place_in_reagents(&mut store, shards, &mut storage, &Kinds).unwrap();
            assert_eq!(storage.entries, vec![entry(SHARD, 20)]);
            place_in_reagents(&mut store, clusters, &mut storage, &Kinds).unwrap();
            assert_eq!(storage.entries, vec![entry(SHARD, 20), entry(CLUSTER, 3)]);
            place_in_reagents(&mut store, single, &mut storage, &Kinds).unwrap();
            assert_eq!(storage.entries, vec![entry(SHARD, 20), entry(CLUSTER, 4)]);
            assert!(store.is_empty());
        }

        #[test]
        fn placing_refuses_non_reagents_and_overflow_without_touching_anything() {
            let mut storage = storage(vec![entry(SHARD, u32::MAX - 1)]);
            let mut store = VaultStore::new();
            let legs = store.add(item(LEGS), ItemOrigin::Unknown, NOW);
            let too_many = store.add(stack(SHARD, 2), ItemOrigin::Unknown, NOW);
            let before = (store.clone(), storage.clone());

            assert_eq!(
                place_in_reagents(&mut store, legs, &mut storage, &Kinds),
                Err(TransferError::NotAReagent {
                    id: legs,
                    base_name: LEGS.into()
                })
            );
            assert_eq!(
                place_in_reagents(&mut store, too_many, &mut storage, &Kinds),
                Err(TransferError::ReagentCountOverflow {
                    index: FIRST,
                    record: SHARD.into(),
                    more: 2
                })
            );
            assert_eq!(
                place_in_reagents(&mut store, StoredItemId::new(9), &mut storage, &Kinds),
                Err(TransferError::NoSuchStoredItem(StoredItemId::new(9)))
            );
            assert_eq!((store, storage), before);
        }

        #[test]
        fn vault_then_place_restores_the_storage_exactly() {
            let mut storage = storage(vec![entry(SHARD, 15), entry(CLUSTER, 20)]);
            let before = storage.clone();
            let mut store = VaultStore::new();
            let id = vault_from_reagents(&mut storage, &Campaign::Main, FIRST, 3, &mut store, NOW)
                .unwrap();
            place_in_reagents(&mut store, id, &mut storage, &Kinds).unwrap();
            assert_eq!(storage, before);
            assert!(store.is_empty());
        }
    }

    mod characters {
        use super::*;
        use crate::gdc::{
            Block, CharacterInfo, Inventory, InventoryContents, InventoryState, PlayerHeader,
            PlayerStash, Sex,
        };

        const MAIN: SackIndex = SackIndex::MAIN;
        const EXTRA: SackIndex = SackIndex::new(1);
        const FIRST: ItemIndex = ItemIndex::new(0);

        fn in_sack(base_name: &str, x: u32, y: u32) -> SackItem {
            SackItem {
                item: item(base_name),
                x,
                y,
            }
        }

        fn sack(items: Vec<SackItem>) -> Sack {
            Sack { flag: 1, items }
        }

        fn player(sacks: Vec<Sack>) -> PlayerFile {
            player_with_stash(sacks, vec![])
        }

        fn player_with_stash(sacks: Vec<Sack>, tabs: Vec<StashTab>) -> PlayerFile {
            let inventory = Inventory {
                version: ContainerVersion::new(11).unwrap(),
                flag: 0,
                state: InventoryState::Entered(Box::new(InventoryContents {
                    focused_sack: 0,
                    selected_sack: 0,
                    sacks,
                    use_alternate: 0,
                    equipment: Default::default(),
                    alternate_1: 0,
                    weapon_set_1: Default::default(),
                    alternate_2: 0,
                    weapon_set_2: Default::default(),
                })),
            };
            PlayerFile::from_parts(
                7,
                PlayerHeader {
                    name: "Sif".into(),
                    sex: Sex::Female,
                    class_tag: String::new(),
                    level: 1,
                    hardcore: false,
                    expansion_status: 7,
                    data_version: 8,
                    uid: [0; 16],
                },
                vec![
                    Block::CharacterInfo(CharacterInfo {
                        is_in_main_quest: true,
                        has_been_in_game: true,
                        difficulty: 0,
                        greatest_difficulty: 0,
                        money: 0,
                        greatest_survival_difficulty: 0,
                        current_tribute: 0,
                        compass_state: 0,
                        skill_window_show_help: 0,
                        weapon_swap_active: 0,
                        weapon_swap_enabled: 0,
                        texture: String::new(),
                        loot_filter: vec![],
                    }),
                    Block::Inventory(inventory),
                    Block::Stash(PlayerStash {
                        version: ContainerVersion::new(11).unwrap(),
                        tabs,
                    }),
                ],
            )
        }

        fn sacks_of(player: &PlayerFile) -> &[Sack] {
            player.inventory().unwrap().sacks()
        }

        fn stash_tabs_of(player: &PlayerFile) -> &[StashTab] {
            &player.stash().unwrap().tabs
        }

        #[test]
        fn own_stash_vault_then_place_round_trips_the_item() {
            let footprints = table();
            let mut player =
                player_with_stash(vec![], vec![tab(8, 16, vec![placed(CLUSTER, 1.0, 6.0)])]);
            let mut store = VaultStore::new();

            let id =
                vault_from_player_stash(&mut player, Realm::Main, TAB0, FIRST, &mut store, NOW)
                    .unwrap();
            assert!(stash_tabs_of(&player)[0].items.is_empty());
            assert_eq!(
                store.get(id).unwrap().origin(),
                &ItemOrigin::CharacterStash {
                    realm: Realm::Main,
                    name: "Sif".into(),
                    tab: TAB0
                }
            );

            let pos =
                place_in_player_stash(&mut store, id, &mut player, TAB0, &footprints).unwrap();
            assert_eq!(pos, at(0, 0));
            assert!(store.is_empty());
            assert_eq!(
                stash_tabs_of(&player)[0].items,
                vec![placed(CLUSTER, 0.0, 0.0)]
            );

            let id =
                vault_from_player_stash(&mut player, Realm::Main, TAB0, FIRST, &mut store, NOW)
                    .unwrap();
            place_in_player_stash_at(&mut store, id, &mut player, TAB0, at(3, 4), &footprints)
                .unwrap();
            assert_eq!(
                stash_tabs_of(&player)[0].items,
                vec![placed(CLUSTER, 3.0, 4.0)]
            );
        }

        #[test]
        fn own_stash_refusals_leave_the_store_and_player_unchanged() {
            let footprints = table();
            let mut player =
                player_with_stash(vec![], vec![tab(2, 2, vec![placed(CLUSTER, 0.0, 0.0)])]);
            let mut store = VaultStore::new();
            let id = store.add(item(LEGS), ItemOrigin::Unknown, NOW);
            let before = (player.clone(), store.clone());
            assert_eq!(
                vault_from_player_stash(
                    &mut player,
                    Realm::Main,
                    TabIndex::new(1),
                    FIRST,
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoSuchTab(TabIndex::new(1)))
            );
            assert_eq!(
                vault_from_player_stash(
                    &mut player,
                    Realm::Main,
                    TAB0,
                    ItemIndex::new(1),
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoSuchItem {
                    tab: TAB0,
                    index: ItemIndex::new(1)
                })
            );
            assert!(matches!(
                place_in_player_stash(&mut store, id, &mut player, TAB0, &footprints),
                Err(TransferError::NoRoom { tab: TAB0, .. })
            ));
            assert_eq!(
                place_in_player_stash_at(&mut store, id, &mut player, TAB0, at(0, 0), &footprints),
                Err(TransferError::OutOfBounds {
                    tab: TAB0,
                    pos: at(0, 0),
                    footprint: footprints.footprint(&item(LEGS)).unwrap()
                })
            );
            assert_eq!((&player, &store), (&before.0, &before.1));

            let mut untyped = PlayerFile::from_parts(7, player.header().clone(), vec![]);
            assert_eq!(
                vault_from_player_stash(&mut untyped, Realm::Main, TAB0, FIRST, &mut store, NOW),
                Err(TransferError::NoPlayerStash)
            );
            assert_eq!(
                place_in_player_stash(&mut store, id, &mut untyped, TAB0, &footprints),
                Err(TransferError::NoPlayerStash)
            );
        }

        #[test]
        fn dimensions_come_from_the_sack_index() {
            assert_eq!(MAIN.dimensions(), MAIN_SACK);
            assert_eq!(EXTRA.dimensions(), EXTRA_SACK);
            assert_eq!(SackIndex::new(4).dimensions(), EXTRA_SACK);
        }

        #[test]
        fn occupancy_resolves_each_item_to_its_cells() {
            let contents = sack(vec![in_sack(CLUSTER, 1, 6), in_sack(LEGS, 3, 0)]);
            assert_eq!(
                sack_occupancy(MAIN, &contents, &table()).unwrap(),
                vec![rect(1, 6, 1, 2), rect(3, 0, 2, 3)]
            );
            let bad = sack(vec![in_sack(CLUSTER, 70_000, 0)]);
            assert_eq!(
                sack_occupancy(MAIN, &bad, &table()).unwrap_err(),
                OccupancyError::InvalidPosition {
                    index: ItemIndex::new(0)
                }
            );
        }

        #[test]
        fn vault_then_place_round_trips_the_item() {
            let footprints = table();
            let mut player = player(vec![sack(vec![in_sack(CLUSTER, 1, 6)])]);
            let mut store = VaultStore::new();

            let id = vault_from_sack(
                &mut player,
                Realm::Main,
                MAIN,
                ItemIndex::new(0),
                &mut store,
                NOW,
            )
            .unwrap();
            assert!(sacks_of(&player)[0].items.is_empty());
            let stored = store.get(id).unwrap();
            assert_eq!(stored.item(), &item(CLUSTER));
            assert_eq!(
                stored.origin(),
                &ItemOrigin::Character {
                    realm: Realm::Main,
                    name: "Sif".into(),
                    sack: MAIN
                }
            );

            let pos = place_in_sack(&mut store, id, &mut player, MAIN, &footprints).unwrap();
            assert_eq!(pos, at(0, 0));
            assert!(store.is_empty());
            assert_eq!(sacks_of(&player)[0].items, vec![in_sack(CLUSTER, 0, 0)]);
        }

        #[test]
        fn placement_respects_each_sacks_own_bounds() {
            let footprints = table();
            let mut player = player(vec![sack(vec![]), sack(vec![])]);
            let mut store = VaultStore::new();
            let far_main = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
            let far_extra = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
            let too_far = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);

            place_in_sack_at(
                &mut store,
                far_main,
                &mut player,
                MAIN,
                at(11, 6),
                &footprints,
            )
            .unwrap();
            place_in_sack_at(
                &mut store,
                far_extra,
                &mut player,
                EXTRA,
                at(7, 6),
                &footprints,
            )
            .unwrap();
            assert_eq!(
                place_in_sack_at(
                    &mut store,
                    too_far,
                    &mut player,
                    EXTRA,
                    at(8, 0),
                    &footprints
                ),
                Err(TransferError::SackOutOfBounds {
                    sack: EXTRA,
                    pos: at(8, 0),
                    footprint: footprints.footprint(&item(CLUSTER)).unwrap()
                })
            );
            assert_eq!(
                place_in_sack_at(
                    &mut store,
                    too_far,
                    &mut player,
                    MAIN,
                    at(11, 6),
                    &footprints
                ),
                Err(TransferError::SackOccupied {
                    sack: MAIN,
                    pos: at(11, 6)
                })
            );
            assert_eq!(sacks_of(&player)[0].items, vec![in_sack(CLUSTER, 11, 6)]);
            assert_eq!(sacks_of(&player)[1].items, vec![in_sack(CLUSTER, 7, 6)]);
            assert_eq!(store.len(), 1);
        }

        #[test]
        fn full_sack_reports_no_room() {
            let footprints = table();
            let full: Vec<SackItem> = (0..8)
                .flat_map(|x| (0..4).map(move |row| in_sack(CLUSTER, x, row * 2)))
                .collect();
            let mut player = player(vec![sack(vec![]), sack(full)]);
            let mut store = VaultStore::new();
            let id = store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
            assert_eq!(
                place_in_sack(&mut store, id, &mut player, EXTRA, &footprints),
                Err(TransferError::SackNoRoom {
                    sack: EXTRA,
                    footprint: footprints.footprint(&item(CLUSTER)).unwrap()
                })
            );
            assert_eq!(
                place_in_sack(&mut store, id, &mut player, MAIN, &footprints),
                Ok(at(0, 0))
            );
        }

        #[test]
        fn vault_refuses_missing_sacks_and_items_without_touching_the_store() {
            let mut player = player(vec![sack(vec![in_sack(CLUSTER, 1, 6)])]);
            let mut store = VaultStore::new();
            assert_eq!(
                vault_from_sack(
                    &mut player,
                    Realm::Main,
                    EXTRA,
                    ItemIndex::new(0),
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoSuchSack(EXTRA))
            );
            assert_eq!(
                vault_from_sack(
                    &mut player,
                    Realm::Main,
                    MAIN,
                    ItemIndex::new(1),
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoSuchSackItem {
                    sack: MAIN,
                    index: ItemIndex::new(1)
                })
            );
            assert!(store.is_empty());
            assert_eq!(sacks_of(&player)[0].items.len(), 1);

            let mut untyped = PlayerFile::from_parts(7, player.header().clone(), vec![]);
            assert_eq!(
                vault_from_sack(
                    &mut untyped,
                    Realm::Main,
                    MAIN,
                    ItemIndex::new(0),
                    &mut store,
                    NOW
                ),
                Err(TransferError::NoInventory)
            );
            assert_eq!(
                place_in_sack(
                    &mut store,
                    StoredItemId::new(1),
                    &mut untyped,
                    MAIN,
                    &table()
                ),
                Err(TransferError::NoSuchStoredItem(StoredItemId::new(1)))
            );
        }
    }
}
