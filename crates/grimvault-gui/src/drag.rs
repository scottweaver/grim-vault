//! Drag-and-drop as data: where a drag began, where it may land, what
//! that asks for, and applying it — every move through
//! `grimvault_core::transfer`, so no container is ever edited by hand.
//!
//! A move is one shape whatever its ends: the item is lifted out of
//! its source into a scratch store and placed from there into the
//! target, the vault store itself being just another end. A placement
//! that fails restores the source wholesale, so a snapped-back drag
//! leaves nothing dirty. A copy skips the lift: the scratch store is
//! seeded with a clone and the source is never touched.

use egui::Vec2;
use grimvault_core::gamedata::Footprint;
use grimvault_core::gdc::PlayerFile;
use grimvault_core::gst::{ReagentStorage, TransferStash};
use grimvault_core::item::Item;
use grimvault_core::reagents::{ReagentKind, ReagentKinds};
use grimvault_core::store::{ItemOrigin, StoredItemId, Timestamp, VaultStore};
use grimvault_core::transfer::{
    self, Footprints, ItemIndex, ReagentIndex, SackIndex, TabIndex, TransferError,
};
use thiserror::Error;
use univault_engine::grid::{CellRect, fits};
use univault_engine::ids::GridPos;

use crate::documents::{CharacterSlot, Doc};

/// A cell grid an item can be lifted from or dropped into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    /// A tab of `transfer.gst`.
    TransferStash(TabIndex),
    /// A sack of a character's inventory.
    Sack {
        character: CharacterSlot,
        sack: SackIndex,
    },
    /// A tab of a character's own stash.
    CharacterStash {
        character: CharacterSlot,
        tab: TabIndex,
    },
}

impl Container {
    /// The document the container lives in.
    #[must_use]
    pub fn doc(self) -> Doc {
        match self {
            Self::TransferStash(_) => Doc::Stash,
            Self::Sack { character, .. } | Self::CharacterStash { character, .. } => {
                Doc::Character(character)
            }
        }
    }
}

/// Where a drag was lifted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSource {
    /// An item of a cell grid.
    Grid {
        container: Container,
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

impl DragSource {
    /// The document the source lives in.
    #[must_use]
    pub fn doc(self) -> Doc {
        match self {
            Self::Grid { container, .. } => container.doc(),
            Self::Store(_) => Doc::Store,
            Self::Reagent { .. } => Doc::Reagents,
        }
    }
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
    /// A cell of a grid.
    Cell { container: Container, cell: GridPos },
    /// A grid's header: first fit in that grid.
    Container(Container),
    /// The store pane: filed by type, no cells.
    Store,
    /// One of the storage's tabs: merged by record, no cells. The tab
    /// is only where the pointer is — the storage is one file and the
    /// record decides the tab.
    Reagents(ReagentKind),
}

impl DropTarget {
    /// The document the target lives in.
    #[must_use]
    pub fn doc(self) -> Doc {
        match self {
            Self::Cell { container, .. } | Self::Container(container) => container.doc(),
            Self::Store => Doc::Store,
            Self::Reagents(_) => Doc::Reagents,
        }
    }
}

/// Whether a footprint can land at a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Fits,
    Blocked,
    /// The grid holds an item whose footprint is unknown, so nothing
    /// can be placed in it.
    Unresolvable,
}

/// Whether a drop moves the item or leaves it behind and lands a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Move,
    Copy,
}

/// What a drop or double-click asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub source: DragSource,
    pub target: DropTarget,
    pub mode: Mode,
}

impl Move {
    /// A move that ends where it began: back onto its own grid's
    /// header, the store onto the store, or the storage onto the
    /// storage. A copy onto the same place is a duplicate, not a stay.
    #[must_use]
    pub fn is_stay(self) -> bool {
        match (self.source, self.target, self.mode) {
            (DragSource::Reagent { .. }, DropTarget::Reagents(_), Mode::Move | Mode::Copy)
            | (DragSource::Store(_), DropTarget::Store, Mode::Move) => true,
            (DragSource::Grid { container, .. }, DropTarget::Container(target), Mode::Move) => {
                container == target
            }
            (
                DragSource::Grid { .. } | DragSource::Store(_) | DragSource::Reagent { .. },
                DropTarget::Cell { .. }
                | DropTarget::Container(_)
                | DropTarget::Store
                | DropTarget::Reagents(_),
                Mode::Move | Mode::Copy,
            ) => false,
        }
    }
}

/// What a double-click asks for: a grid item or a storage row goes to
/// the store, a stored item takes the first fit in the current
/// transfer-stash tab.
#[must_use]
pub fn double_click(source: DragSource, mode: Mode, current_tab: TabIndex) -> Move {
    let target = match source {
        DragSource::Grid { .. } | DragSource::Reagent { .. } => DropTarget::Store,
        DragSource::Store(_) => DropTarget::Container(Container::TransferStash(current_tab)),
    };
    Move {
        source,
        target,
        mode,
    }
}

/// Preview verdict for `footprint` at `cell` over a grid whose
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

/// Preview verdict for a drag over one of the storage's tabs: a grid
/// or store item lands if its record is storable; a storage row is
/// already there.
#[must_use]
pub fn fit_in_reagents(source: DragSource, kind: Option<ReagentKind>) -> Fit {
    match source {
        DragSource::Grid { .. } | DragSource::Store(_) => {
            if kind.is_some() {
                Fit::Fits
            } else {
                Fit::Blocked
            }
        }
        DragSource::Reagent { .. } => Fit::Blocked,
    }
}

/// Preview verdict for a drag over the store: everything lands except
/// a stored item moved onto the store it is already in — copied, it
/// duplicates.
#[must_use]
pub fn fit_in_store(source: DragSource, mode: Mode) -> Fit {
    match (source, mode) {
        (DragSource::Store(_), Mode::Move) => Fit::Blocked,
        (DragSource::Store(_), Mode::Copy)
        | (DragSource::Grid { .. } | DragSource::Reagent { .. }, Mode::Move | Mode::Copy) => {
            Fit::Fits
        }
    }
}

/// Where a moved or copied item ended up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The top-left cell it took in a grid.
    Cell(GridPos),
    /// Its id in the store.
    Stored(StoredItemId),
    /// Merged into the storage's entry for its record.
    Merged,
}

/// What a move did, for the shell to mark dirty and order writes by:
/// `to` always changed; `from` changed only for a [`Mode::Move`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    Changed {
        mode: Mode,
        from: Doc,
        to: Doc,
        landing: Landing,
    },
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
    /// The move needs a character that is unreadable or read-only.
    #[error("character {} is not open for editing", .0.value() + 1)]
    CharacterNotEditable(CharacterSlot),
    /// A copy found nothing at its source: the container changed
    /// between the pane's report and the drop.
    #[error("the item is no longer where the drag began")]
    SourceGone,
}

/// Every container a move may touch; `reagents` is `None` while
/// `reagents.gst` is absent or unusable, and a character is `None`
/// while unreadable or read-only.
pub struct Containers<'a> {
    pub stash: &'a mut TransferStash,
    pub store: &'a mut VaultStore,
    pub reagents: Option<&'a mut ReagentStorage>,
    pub characters: Vec<Option<&'a mut PlayerFile>>,
}

impl Containers<'_> {
    fn character(&mut self, slot: CharacterSlot) -> Result<&mut PlayerFile, ApplyError> {
        self.characters
            .get_mut(slot.value())
            .and_then(|player| player.as_deref_mut())
            .ok_or(ApplyError::CharacterNotEditable(slot))
    }

    fn character_ref(&self, slot: CharacterSlot) -> Result<&PlayerFile, ApplyError> {
        self.characters
            .get(slot.value())
            .and_then(|player| player.as_deref())
            .ok_or(ApplyError::CharacterNotEditable(slot))
    }

    fn reagents(&mut self) -> Result<&mut ReagentStorage, ApplyError> {
        self.reagents
            .as_deref_mut()
            .ok_or(ApplyError::NoReagentStorage)
    }
}

/// Performs `mv`. On any error every container is exactly as it was.
///
/// # Errors
/// The underlying [`TransferError`], or an end that is not open.
pub fn apply(
    mv: Move,
    containers: &mut Containers<'_>,
    facts: &(impl Footprints + ReagentKinds),
    now: Timestamp,
) -> Result<Applied, ApplyError> {
    if mv.is_stay() {
        return Ok(Applied::Unmoved);
    }
    let Move {
        source,
        target,
        mode,
    } = mv;
    let mut scratch = VaultStore::new();
    let landing = match mode {
        Mode::Copy => {
            let (item, origin) = peek(containers, source)?;
            let id = scratch.add(item, origin, now);
            place(containers, target, &mut scratch, id, facts, now)?
        }
        Mode::Move => {
            let snapshot = Snapshot::take(containers, source.doc())?;
            let id = lift(containers, source, &mut scratch, now)?;
            match place(containers, target, &mut scratch, id, facts, now) {
                Ok(landing) => landing,
                Err(error) => {
                    snapshot.restore(containers);
                    return Err(error);
                }
            }
        }
    };
    Ok(Applied::Changed {
        mode,
        from: source.doc(),
        to: target.doc(),
        landing,
    })
}

/// The item at `source` and where a store would record it as coming
/// from, without disturbing it.
///
/// # Errors
/// [`ApplyError::SourceGone`] when nothing is there, or an end that is
/// not open.
pub fn peek(
    containers: &Containers<'_>,
    source: DragSource,
) -> Result<(Item, ItemOrigin), ApplyError> {
    let found = match source {
        DragSource::Grid {
            container: Container::TransferStash(tab),
            index,
        } => grid_item(&containers.stash.tabs, tab, index)
            .map(|item| (item, ItemOrigin::TransferStash { tab })),
        DragSource::Grid {
            container: Container::Sack { character, sack },
            index,
        } => {
            let player = containers.character_ref(character)?;
            player
                .inventory()
                .and_then(|inventory| inventory.sacks().get(slot(sack.value())?))
                .and_then(|contents| contents.items.get(index.value()))
                .map(|placed| {
                    (
                        placed.item.clone(),
                        ItemOrigin::Character {
                            name: player.character_name().to_owned(),
                            sack,
                        },
                    )
                })
        }
        DragSource::Grid {
            container: Container::CharacterStash { character, tab },
            index,
        } => {
            let player = containers.character_ref(character)?;
            player
                .stash()
                .and_then(|stash| grid_item(&stash.tabs, tab, index))
                .map(|item| {
                    (
                        item,
                        ItemOrigin::CharacterStash {
                            name: player.character_name().to_owned(),
                            tab,
                        },
                    )
                })
        }
        DragSource::Store(id) => containers
            .store
            .get(id)
            .map(|stored| (stored.item().clone(), stored.origin().clone())),
        DragSource::Reagent { index, count } => containers
            .reagents
            .as_deref()
            .ok_or(ApplyError::NoReagentStorage)?
            .entries
            .get(index.value())
            .map(|entry| {
                (
                    Item {
                        base_name: entry.record.clone(),
                        stack_count: count,
                        ..Item::default()
                    },
                    ItemOrigin::ReagentStorage,
                )
            }),
    };
    found.ok_or(ApplyError::SourceGone)
}

fn grid_item(
    tabs: &[grimvault_core::block::StashTab],
    tab: TabIndex,
    index: ItemIndex,
) -> Option<Item> {
    tabs.get(slot(tab.value())?)
        .and_then(|tab| tab.items.get(index.value()))
        .map(|placed| placed.item.clone())
}

fn slot(index: u32) -> Option<usize> {
    usize::try_from(index).ok()
}

/// Takes the item at `source` out of its container into `into`.
fn lift(
    containers: &mut Containers<'_>,
    source: DragSource,
    into: &mut VaultStore,
    now: Timestamp,
) -> Result<StoredItemId, ApplyError> {
    Ok(match source {
        DragSource::Grid {
            container: Container::TransferStash(tab),
            index,
        } => transfer::vault_from_stash(containers.stash, tab, index, into, now)?,
        DragSource::Grid {
            container: Container::Sack { character, sack },
            index,
        } => transfer::vault_from_sack(containers.character(character)?, sack, index, into, now)?,
        DragSource::Grid {
            container: Container::CharacterStash { character, tab },
            index,
        } => transfer::vault_from_player_stash(
            containers.character(character)?,
            tab,
            index,
            into,
            now,
        )?,
        DragSource::Store(id) => {
            let (item, origin) = containers
                .store
                .take(id)
                .ok_or(TransferError::NoSuchStoredItem(id))?
                .into_parts();
            into.add(item, origin, now)
        }
        DragSource::Reagent { index, count } => {
            transfer::vault_from_reagents(containers.reagents()?, index, count, into, now)?
        }
    })
}

/// Puts stored item `id` of `from` down at `target`.
fn place(
    containers: &mut Containers<'_>,
    target: DropTarget,
    from: &mut VaultStore,
    id: StoredItemId,
    facts: &(impl Footprints + ReagentKinds),
    now: Timestamp,
) -> Result<Landing, ApplyError> {
    Ok(match target {
        DropTarget::Cell {
            container: Container::TransferStash(tab),
            cell,
        } => {
            transfer::place_in_stash_at(from, id, containers.stash, tab, cell, facts)?;
            Landing::Cell(cell)
        }
        DropTarget::Container(Container::TransferStash(tab)) => Landing::Cell(
            transfer::place_in_stash(from, id, containers.stash, tab, facts)?,
        ),
        DropTarget::Cell {
            container: Container::Sack { character, sack },
            cell,
        } => {
            let player = containers.character(character)?;
            transfer::place_in_sack_at(from, id, player, sack, cell, facts)?;
            Landing::Cell(cell)
        }
        DropTarget::Container(Container::Sack { character, sack }) => {
            let player = containers.character(character)?;
            Landing::Cell(transfer::place_in_sack(from, id, player, sack, facts)?)
        }
        DropTarget::Cell {
            container: Container::CharacterStash { character, tab },
            cell,
        } => {
            let player = containers.character(character)?;
            transfer::place_in_player_stash_at(from, id, player, tab, cell, facts)?;
            Landing::Cell(cell)
        }
        DropTarget::Container(Container::CharacterStash { character, tab }) => {
            let player = containers.character(character)?;
            Landing::Cell(transfer::place_in_player_stash(
                from, id, player, tab, facts,
            )?)
        }
        DropTarget::Store => {
            let (item, origin) = from
                .take(id)
                .ok_or(TransferError::NoSuchStoredItem(id))?
                .into_parts();
            Landing::Stored(containers.store.add(item, origin, now))
        }
        DropTarget::Reagents(_) => {
            transfer::place_in_reagents(from, id, containers.reagents()?, facts)?;
            Landing::Merged
        }
    })
}

/// The source document as it was before a lift, so a refused
/// placement can put it back exactly.
enum Snapshot {
    Stash(TransferStash),
    Store(VaultStore),
    Reagents(ReagentStorage),
    Character(CharacterSlot, PlayerFile),
}

impl Snapshot {
    fn take(containers: &Containers<'_>, doc: Doc) -> Result<Self, ApplyError> {
        Ok(match doc {
            Doc::Stash => Self::Stash(containers.stash.clone()),
            Doc::Store => Self::Store(containers.store.clone()),
            Doc::Reagents => Self::Reagents(
                containers
                    .reagents
                    .as_deref()
                    .ok_or(ApplyError::NoReagentStorage)?
                    .clone(),
            ),
            Doc::Character(slot) => Self::Character(slot, containers.character_ref(slot)?.clone()),
        })
    }

    /// Restores a snapshot that was taken from these same containers;
    /// an end that has since closed simply has nothing to restore.
    fn restore(self, containers: &mut Containers<'_>) {
        match self {
            Self::Stash(stash) => *containers.stash = stash,
            Self::Store(store) => *containers.store = store,
            Self::Reagents(storage) => {
                if let Some(target) = containers.reagents.as_deref_mut() {
                    *target = storage;
                }
            }
            Self::Character(slot, player) => {
                if let Ok(target) = containers.character(slot) {
                    *target = player;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use grimvault_core::block::{StashTab, TabDecoration};
    use grimvault_core::gdc::{
        Block, CharacterInfo, Inventory, InventoryContents, InventoryState, PlayerHeader,
        PlayerStash, Sack, Sex,
    };
    use grimvault_core::gst::{ReagentEntry, ReagentStorageVersion};
    use grimvault_core::item::{ContainerVersion, SackItem, StashItem};

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

    fn in_sack(base_name: &str, x: u32, y: u32) -> SackItem {
        SackItem {
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

    fn player(sacks: Vec<Vec<SackItem>>, tabs: Vec<StashTab>) -> PlayerFile {
        let version = ContainerVersion::new(11).unwrap();
        let sacks = sacks
            .into_iter()
            .map(|items| Sack { flag: 1, items })
            .collect();
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
                Block::Inventory(Inventory {
                    version,
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
                }),
                Block::Stash(PlayerStash { version, tabs }),
            ],
        )
    }

    fn sack_items(player: &PlayerFile, sack: usize) -> &[SackItem] {
        &player.inventory().unwrap().sacks()[sack].items
    }

    fn own_stash_items(player: &PlayerFile, tab: usize) -> &[StashItem] {
        &player.stash().unwrap().tabs[tab].items
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
    const ROW0: ReagentIndex = ReagentIndex::new(0);
    const SIF: CharacterSlot = CharacterSlot::new(0);
    const STASH0: Container = Container::TransferStash(TAB0);
    const STASH1: Container = Container::TransferStash(TAB1);
    const MAIN_BAG: Container = Container::Sack {
        character: SIF,
        sack: SackIndex::MAIN,
    };
    const OWN0: Container = Container::CharacterStash {
        character: SIF,
        tab: TAB0,
    };
    const NOW: Timestamp = Timestamp::from_unix_seconds(1_756_900_000);

    fn grid(container: Container, index: usize) -> DragSource {
        DragSource::Grid {
            container,
            index: ItemIndex::new(index),
        }
    }

    fn cell(container: Container, x: i32, y: i32) -> DropTarget {
        DropTarget::Cell {
            container,
            cell: at(x, y),
        }
    }

    fn mv(source: DragSource, target: DropTarget) -> Move {
        Move {
            source,
            target,
            mode: Mode::Move,
        }
    }

    fn copy(source: DragSource, target: DropTarget) -> Move {
        Move {
            source,
            target,
            mode: Mode::Copy,
        }
    }

    /// A world with every end open, for the tests to take apart.
    struct World {
        stash: TransferStash,
        store: VaultStore,
        reagents: Option<ReagentStorage>,
        player: Option<PlayerFile>,
    }

    impl World {
        fn apply(&mut self, mv: Move) -> Result<Applied, ApplyError> {
            let mut containers = Containers {
                stash: &mut self.stash,
                store: &mut self.store,
                reagents: self.reagents.as_mut(),
                characters: vec![self.player.as_mut()],
            };
            apply(mv, &mut containers, &table(), NOW)
        }

        fn snapshot(
            &self,
        ) -> (
            TransferStash,
            VaultStore,
            Option<ReagentStorage>,
            Option<PlayerFile>,
        ) {
            (
                self.stash.clone(),
                self.store.clone(),
                self.reagents.clone(),
                self.player.clone(),
            )
        }

        fn player(&self) -> &PlayerFile {
            self.player.as_ref().unwrap()
        }
    }

    fn world() -> World {
        World {
            stash: stash(vec![tab(10, 19, vec![placed(CLUSTER, 1.0, 6.0)])]),
            store: VaultStore::new(),
            reagents: Some(storage(vec![(CLUSTER, 10)])),
            player: Some(player(
                vec![vec![in_sack(LEGS, 0, 0)], vec![]],
                vec![tab(8, 16, vec![placed(CLUSTER, 2.0, 2.0)])],
            )),
        }
    }

    fn changed(applied: Applied) -> (Mode, Doc, Doc, Landing) {
        match applied {
            Applied::Changed {
                mode,
                from,
                to,
                landing,
            } => (mode, from, to, landing),
            Applied::Unmoved => panic!("expected a change"),
        }
    }

    fn stored_id(applied: Applied) -> StoredItemId {
        match changed(applied).3 {
            Landing::Stored(id) => id,
            other => panic!("expected a store landing, got {other:?}"),
        }
    }

    #[test]
    fn double_clicks_go_to_the_store_or_the_current_tab() {
        let from_grid = grid(MAIN_BAG, 0);
        let from_store = DragSource::Store(StoredItemId::new(4));
        let from_row = DragSource::Reagent {
            index: ROW0,
            count: 5,
        };
        assert_eq!(
            double_click(from_grid, Mode::Move, TAB1),
            mv(from_grid, DropTarget::Store)
        );
        assert_eq!(
            double_click(from_row, Mode::Copy, TAB1),
            copy(from_row, DropTarget::Store)
        );
        assert_eq!(
            double_click(from_store, Mode::Move, TAB1),
            mv(from_store, DropTarget::Container(STASH1))
        );
    }

    #[test]
    fn stays_are_moves_that_end_where_they_began() {
        let from_stash = grid(STASH0, 0);
        let from_store = DragSource::Store(StoredItemId::new(4));
        let from_row = DragSource::Reagent {
            index: ROW0,
            count: 5,
        };
        assert!(mv(from_stash, DropTarget::Container(STASH0)).is_stay());
        assert!(!mv(from_stash, DropTarget::Container(STASH1)).is_stay());
        assert!(!mv(from_stash, cell(STASH0, 1, 6)).is_stay());
        assert!(!copy(from_stash, DropTarget::Container(STASH0)).is_stay());
        assert!(mv(from_store, DropTarget::Store).is_stay());
        assert!(!copy(from_store, DropTarget::Store).is_stay());
        assert!(mv(from_row, DropTarget::Reagents(ReagentKind::Component)).is_stay());
        assert!(copy(from_row, DropTarget::Reagents(ReagentKind::Component)).is_stay());
        assert!(!mv(from_row, DropTarget::Store).is_stay());
    }

    #[test]
    fn ends_name_their_documents() {
        assert_eq!(STASH0.doc(), Doc::Stash);
        assert_eq!(MAIN_BAG.doc(), Doc::Character(SIF));
        assert_eq!(OWN0.doc(), Doc::Character(SIF));
        assert_eq!(DragSource::Store(StoredItemId::new(1)).doc(), Doc::Store);
        assert_eq!(
            DragSource::Reagent {
                index: ROW0,
                count: 1
            }
            .doc(),
            Doc::Reagents
        );
        assert_eq!(cell(OWN0, 0, 0).doc(), Doc::Character(SIF));
        assert_eq!(
            DropTarget::Reagents(ReagentKind::Component).doc(),
            Doc::Reagents
        );
    }

    #[test]
    fn fit_previews_bounds_overlap_and_unresolvable_grids() {
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
    fn storage_and_store_previews_follow_the_source_and_mode() {
        let from_stash = grid(STASH0, 0);
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
        assert_eq!(
            fit_in_reagents(from_row, Some(ReagentKind::Component)),
            Fit::Blocked
        );
        assert_eq!(fit_in_store(from_store, Mode::Move), Fit::Blocked);
        assert_eq!(fit_in_store(from_store, Mode::Copy), Fit::Fits);
        assert_eq!(fit_in_store(from_stash, Mode::Move), Fit::Fits);
        assert_eq!(fit_in_store(from_row, Mode::Copy), Fit::Fits);
    }

    #[test]
    fn vault_and_place_go_through_transfer() {
        let mut world = world();
        let applied = world.apply(mv(grid(STASH0, 0), DropTarget::Store)).unwrap();
        let (mode, from, to, _) = changed(applied);
        assert_eq!((mode, from, to), (Mode::Move, Doc::Stash, Doc::Store));
        let id = stored_id(applied);
        assert!(world.stash.tabs[0].items.is_empty());
        assert_eq!(
            world.store.get(id).unwrap().origin(),
            &ItemOrigin::TransferStash { tab: TAB0 }
        );

        assert_eq!(
            world
                .apply(mv(DragSource::Store(id), cell(STASH0, 4, 4)))
                .unwrap(),
            Applied::Changed {
                mode: Mode::Move,
                from: Doc::Store,
                to: Doc::Stash,
                landing: Landing::Cell(at(4, 4))
            }
        );
        assert!(world.store.is_empty());
        assert_eq!(world.stash.tabs[0].items, vec![placed(CLUSTER, 4.0, 4.0)]);
    }

    #[test]
    fn first_fit_placement_lands_at_the_first_open_cells() {
        let mut world = world();
        world.stash = stash(vec![tab(4, 4, vec![placed(LEGS, 0.0, 0.0)])]);
        let id = world.store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        assert_eq!(
            changed(
                world
                    .apply(mv(DragSource::Store(id), DropTarget::Container(STASH0)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(2, 0))
        );
    }

    #[test]
    fn a_rearrangement_may_overlap_its_own_old_cells_and_leaves_the_store_alone() {
        let mut world = world();
        world.stash = stash(vec![tab(
            10,
            19,
            vec![placed(LEGS, 0.0, 0.0), placed(CLUSTER, 5.0, 5.0)],
        )]);
        let before = world.store.clone();
        assert_eq!(
            world
                .apply(mv(grid(STASH0, 0), cell(STASH0, 1, 1)))
                .unwrap(),
            Applied::Changed {
                mode: Mode::Move,
                from: Doc::Stash,
                to: Doc::Stash,
                landing: Landing::Cell(at(1, 1))
            }
        );
        assert_eq!(world.store, before);
        assert_eq!(
            world.stash.tabs[0].items,
            vec![placed(CLUSTER, 5.0, 5.0), placed(LEGS, 1.0, 1.0)]
        );
    }

    #[test]
    fn a_refused_move_restores_the_source_exactly() {
        let mut world = world();
        world.stash = stash(vec![
            tab(
                10,
                19,
                vec![placed(LEGS, 0.0, 0.0), placed(CLUSTER, 5.0, 5.0)],
            ),
            tab(2, 2, vec![]),
        ]);
        let legs = world.store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        let before = world.snapshot();

        assert_eq!(
            world.apply(mv(grid(STASH0, 0), cell(STASH0, 5, 4))),
            Err(ApplyError::Transfer(TransferError::Occupied {
                tab: TAB0,
                pos: at(5, 4)
            }))
        );
        assert!(matches!(
            world.apply(mv(grid(STASH0, 0), DropTarget::Container(STASH1))),
            Err(ApplyError::Transfer(TransferError::NoRoom {
                tab: TAB1,
                ..
            }))
        ));
        assert!(matches!(
            world.apply(mv(DragSource::Store(legs), DropTarget::Container(STASH1))),
            Err(ApplyError::Transfer(TransferError::NoRoom {
                tab: TAB1,
                ..
            }))
        ));
        assert_eq!(
            world.apply(mv(grid(MAIN_BAG, 0), cell(STASH0, 0, 0))),
            Err(ApplyError::Transfer(TransferError::Occupied {
                tab: TAB0,
                pos: at(0, 0)
            }))
        );
        assert_eq!(
            world.apply(mv(
                DragSource::Reagent {
                    index: ROW0,
                    count: 3
                },
                cell(STASH0, 5, 5)
            )),
            Err(ApplyError::Transfer(TransferError::Occupied {
                tab: TAB0,
                pos: at(5, 5)
            }))
        );
        assert_eq!(world.snapshot(), before);
    }

    #[test]
    fn an_unresolvable_grid_refuses_moves_without_side_effects() {
        let mut world = world();
        world.stash = stash(vec![tab(10, 19, vec![placed(MYSTERY, 0.0, 0.0)])]);
        let id = world.store.add(item(CLUSTER), ItemOrigin::Unknown, NOW);
        let before = world.snapshot();
        assert!(matches!(
            world.apply(mv(DragSource::Store(id), cell(STASH0, 3, 3))),
            Err(ApplyError::Transfer(TransferError::Occupancy(_)))
        ));
        assert_eq!(world.snapshot(), before);
    }

    #[test]
    fn storage_moves_go_through_the_scratch_store_in_every_direction() {
        let mut world = world();
        world.stash.tabs[0].items[0].item.stack_count = 3;

        assert_eq!(
            world
                .apply(mv(
                    grid(STASH0, 0),
                    DropTarget::Reagents(ReagentKind::Component)
                ))
                .unwrap(),
            Applied::Changed {
                mode: Mode::Move,
                from: Doc::Stash,
                to: Doc::Reagents,
                landing: Landing::Merged
            }
        );
        assert!(world.stash.tabs[0].items.is_empty());
        assert_eq!(world.reagents.as_ref().unwrap().entries[0].count, 13);

        let row = DragSource::Reagent {
            index: ROW0,
            count: 4,
        };
        let id = stored_id(world.apply(mv(row, DropTarget::Store)).unwrap());
        assert_eq!(world.store.get(id).unwrap().item().stack_count, 4);
        assert_eq!(
            world.store.get(id).unwrap().origin(),
            &ItemOrigin::ReagentStorage
        );
        assert_eq!(world.reagents.as_ref().unwrap().entries[0].count, 9);

        assert_eq!(
            changed(
                world
                    .apply(mv(
                        DragSource::Store(id),
                        DropTarget::Reagents(ReagentKind::Component)
                    ))
                    .unwrap()
            )
            .3,
            Landing::Merged
        );
        assert!(world.store.is_empty());
        assert_eq!(world.reagents.as_ref().unwrap().entries[0].count, 13);

        let two = DragSource::Reagent {
            index: ROW0,
            count: 2,
        };
        assert_eq!(
            changed(world.apply(mv(two, cell(STASH0, 3, 3))).unwrap()).3,
            Landing::Cell(at(3, 3))
        );
        let rest = DragSource::Reagent {
            index: ROW0,
            count: 11,
        };
        assert_eq!(
            changed(
                world
                    .apply(mv(rest, DropTarget::Container(STASH0)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(0, 0))
        );
        assert!(world.reagents.as_ref().unwrap().entries.is_empty());
        assert_eq!(world.stash.tabs[0].items.len(), 2);
        assert_eq!(world.stash.tabs[0].items[1].item.stack_count, 11);
    }

    #[test]
    fn storage_moves_need_an_open_storage_and_refuse_non_reagents() {
        let mut world = world();
        world.stash = stash(vec![tab(10, 19, vec![placed(LEGS, 0.0, 0.0)])]);
        let legs = world.store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        let before = world.snapshot();

        assert!(matches!(
            world.apply(mv(
                DragSource::Store(legs),
                DropTarget::Reagents(ReagentKind::Component)
            )),
            Err(ApplyError::Transfer(TransferError::NotAReagent { .. }))
        ));
        assert!(matches!(
            world.apply(mv(
                grid(STASH0, 0),
                DropTarget::Reagents(ReagentKind::Component)
            )),
            Err(ApplyError::Transfer(TransferError::NotAReagent { .. }))
        ));
        assert_eq!(world.snapshot(), before);

        world.reagents = None;
        let before = world.snapshot();
        assert_eq!(
            world.apply(mv(
                DragSource::Reagent {
                    index: ROW0,
                    count: 1
                },
                DropTarget::Store
            )),
            Err(ApplyError::NoReagentStorage)
        );
        assert_eq!(
            world.apply(copy(
                DragSource::Store(legs),
                DropTarget::Reagents(ReagentKind::Component)
            )),
            Err(ApplyError::NoReagentStorage)
        );
        assert_eq!(world.snapshot(), before);
    }

    #[test]
    fn character_containers_are_ends_like_any_other() {
        let mut world = world();

        assert_eq!(
            world
                .apply(mv(grid(MAIN_BAG, 0), cell(STASH0, 4, 0)))
                .unwrap(),
            Applied::Changed {
                mode: Mode::Move,
                from: Doc::Character(SIF),
                to: Doc::Stash,
                landing: Landing::Cell(at(4, 0))
            }
        );
        assert!(sack_items(world.player(), 0).is_empty());
        assert_eq!(world.stash.tabs[0].items[1], placed(LEGS, 4.0, 0.0));

        assert_eq!(
            world
                .apply(mv(grid(STASH0, 1), DropTarget::Container(MAIN_BAG)))
                .unwrap(),
            Applied::Changed {
                mode: Mode::Move,
                from: Doc::Stash,
                to: Doc::Character(SIF),
                landing: Landing::Cell(at(0, 0))
            }
        );
        assert_eq!(sack_items(world.player(), 0), &[in_sack(LEGS, 0, 0)]);

        let id = stored_id(
            world
                .apply(mv(grid(MAIN_BAG, 0), DropTarget::Store))
                .unwrap(),
        );
        assert_eq!(
            world.store.get(id).unwrap().origin(),
            &ItemOrigin::Character {
                name: "Sif".into(),
                sack: SackIndex::MAIN
            }
        );
        let extra = Container::Sack {
            character: SIF,
            sack: SackIndex::new(1),
        };
        assert_eq!(
            changed(
                world
                    .apply(mv(DragSource::Store(id), cell(extra, 6, 5)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(6, 5))
        );
        assert_eq!(sack_items(world.player(), 1), &[in_sack(LEGS, 6, 5)]);

        let id = stored_id(world.apply(mv(grid(OWN0, 0), DropTarget::Store)).unwrap());
        assert_eq!(
            world.store.get(id).unwrap().origin(),
            &ItemOrigin::CharacterStash {
                name: "Sif".into(),
                tab: TAB0
            }
        );
        assert!(own_stash_items(world.player(), 0).is_empty());
        assert_eq!(
            changed(
                world
                    .apply(mv(DragSource::Store(id), DropTarget::Container(OWN0)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(0, 0))
        );
        assert_eq!(
            changed(
                world
                    .apply(mv(grid(OWN0, 0), cell(MAIN_BAG, 3, 3)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(3, 3))
        );
        assert_eq!(sack_items(world.player(), 0), &[in_sack(CLUSTER, 3, 3)]);
        assert!(own_stash_items(world.player(), 0).is_empty());
    }

    #[test]
    fn a_character_that_is_not_open_refuses_every_move_untouched() {
        let mut world = world();
        world.player = None;
        let id = world.store.add(item(LEGS), ItemOrigin::Unknown, NOW);
        let before = world.snapshot();
        assert_eq!(
            world.apply(mv(grid(MAIN_BAG, 0), DropTarget::Store)),
            Err(ApplyError::CharacterNotEditable(SIF))
        );
        assert_eq!(
            world.apply(mv(DragSource::Store(id), DropTarget::Container(MAIN_BAG))),
            Err(ApplyError::CharacterNotEditable(SIF))
        );
        assert_eq!(
            world.apply(copy(grid(OWN0, 0), DropTarget::Store)),
            Err(ApplyError::CharacterNotEditable(SIF))
        );
        let other = Container::Sack {
            character: CharacterSlot::new(3),
            sack: SackIndex::MAIN,
        };
        assert_eq!(
            world.apply(mv(grid(STASH0, 0), cell(other, 0, 0))),
            Err(ApplyError::CharacterNotEditable(CharacterSlot::new(3)))
        );
        assert_eq!(world.snapshot(), before);
    }

    #[test]
    fn copies_leave_the_source_in_place() {
        let mut world = world();
        let before = world.snapshot();

        let id = stored_id(
            world
                .apply(copy(grid(STASH0, 0), DropTarget::Store))
                .unwrap(),
        );
        assert_eq!(world.stash, before.0);
        assert_eq!(world.store.get(id).unwrap().item(), &item(CLUSTER));
        assert_eq!(
            world.store.get(id).unwrap().origin(),
            &ItemOrigin::TransferStash { tab: TAB0 }
        );

        let twin = stored_id(
            world
                .apply(copy(DragSource::Store(id), DropTarget::Store))
                .unwrap(),
        );
        assert_ne!(twin, id);
        assert_eq!(world.store.len(), 2);
        assert_eq!(world.store.get(twin).unwrap().item(), &item(CLUSTER));

        assert_eq!(
            changed(
                world
                    .apply(copy(DragSource::Store(id), cell(MAIN_BAG, 5, 5)))
                    .unwrap()
            ),
            (
                Mode::Copy,
                Doc::Store,
                Doc::Character(SIF),
                Landing::Cell(at(5, 5))
            )
        );
        assert_eq!(world.store.len(), 2);
        assert_eq!(sack_items(world.player(), 0)[1], in_sack(CLUSTER, 5, 5));

        assert_eq!(
            changed(
                world
                    .apply(copy(grid(MAIN_BAG, 0), DropTarget::Container(STASH0)))
                    .unwrap()
            )
            .3,
            Landing::Cell(at(0, 0))
        );
        assert_eq!(sack_items(world.player(), 0).len(), 2);
        assert_eq!(world.stash.tabs[0].items[1], placed(LEGS, 0.0, 0.0));

        let row = DragSource::Reagent {
            index: ROW0,
            count: 4,
        };
        let id = stored_id(world.apply(copy(row, DropTarget::Store)).unwrap());
        assert_eq!(world.store.get(id).unwrap().item().stack_count, 4);
        assert_eq!(world.reagents, before.2);

        assert_eq!(
            world.apply(copy(grid(STASH0, 0), cell(STASH0, 1, 6))),
            Err(ApplyError::Transfer(TransferError::Occupied {
                tab: TAB0,
                pos: at(1, 6)
            }))
        );
        assert_eq!(
            world.apply(copy(
                DragSource::Store(StoredItemId::new(99)),
                DropTarget::Store
            )),
            Err(ApplyError::SourceGone)
        );
    }
}
