//! Bulk operations on a whole container — every item of a stash tab
//! or sack moved or copied into the store at once, a stash tab
//! emptied, and the additive sync of the component / crafting-material
//! storage — under a rule a single drag never applies
//! ([`BulkDuplicates`]): by default an item the store already holds
//! is left where it is. "Already holds" is by record and roll seed,
//! and only for the classes the game tells apart that way
//! ([`Identity::Seed`]); a stack's seed is not an identity and a
//! reagent vaulted out of the storage has none, so a stackable is
//! never skipped. Every move goes through [`crate::transfer`] and
//! every copy records the origin a move would, so the store cannot
//! tell a bulk entry from a dragged one. The purge is the identity
//! rule's other half and ignores [`BulkDuplicates`]: it deletes from
//! a tab exactly what a skipping bulk move would leave — the items
//! the store already holds — and never touches the store.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use univault_engine::ids::RecordId;

use crate::block::StashTab;
use crate::bucket::Bucket;
use crate::campaign::Campaign;
use crate::gamedata::GameData;
use crate::gdc::{PlayerFile, Realm};
use crate::gst::{ReagentStorage, TransferStash};
use crate::item::Item;
use crate::settings::BulkDuplicates;
use crate::store::{ItemOrigin, StoredItem, StoredItemId, Timestamp, VaultStore};
use crate::transfer::{
    self, ItemIndex, SackIndex, TabIndex, TransferError, sack_ref, stack_of, tab_mut, tab_ref,
};

/// How the game tells two items of one record apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// By the roll seed — equipment and relics, one per instance: the
    /// seed rolls the affix values, so one record under one seed is
    /// one item.
    Seed,
    /// Not at all — a stack: components, augments, consumables, quest
    /// items, blueprints, notes, and any record whose `maxStackSize`
    /// is above one. The game merges these by record and their seed
    /// rolls nothing.
    Stack,
}

impl Identity {
    /// The rule for a record: a `maxStackSize` above one makes any
    /// record a stack (the template field overrides the engine's
    /// per-class default, `docs/format-references.md`); otherwise the
    /// equipment buckets and relics identify by seed and every other
    /// class stacks.
    #[must_use]
    pub const fn of(bucket: Bucket, max_stack_size: Option<u32>) -> Self {
        match max_stack_size {
            Some(size) if size > 1 => return Self::Stack,
            Some(_) | None => {}
        }
        match bucket {
            Bucket::OneHanded
            | Bucket::TwoHanded
            | Bucket::RangedOneHanded
            | Bucket::RangedTwoHanded
            | Bucket::Offhand
            | Bucket::Shield
            | Bucket::Head
            | Bucket::Chest
            | Bucket::Shoulders
            | Bucket::Hands
            | Bucket::Legs
            | Bucket::Feet
            | Bucket::Waist
            | Bucket::Amulet
            | Bucket::Ring
            | Bucket::Medal
            | Bucket::Relic => Self::Seed,
            Bucket::Component
            | Bucket::Material
            | Bucket::Augment
            | Bucket::Blueprint
            | Bucket::Transmuter
            | Bucket::Consumable
            | Bucket::Writ
            | Bucket::Quest
            | Bucket::Note
            | Bucket::Misc => Self::Stack,
        }
    }
}

/// Supplies an item's identity rule; `None` when no database layer
/// has the record, which no bulk move treats as a duplicate and no
/// stack rule folds.
pub trait Identities {
    fn identity(&self, item: &Item) -> Option<Identity>;

    /// Whether the record is known to stack — the items
    /// [`VaultStore::consolidate_stacks`] keeps as one entry.
    fn is_stack(&self, item: &Item) -> bool {
        self.identity(item) == Some(Identity::Stack)
    }
}

impl Identities for GameData {
    fn identity(&self, item: &Item) -> Option<Identity> {
        let id = RecordId::parse(item.base_name.clone())?;
        let info = self.item_info(&id)?.ok()?;
        let bucket = Bucket::of(info.class.as_ref(), info.reagent);
        Some(Identity::of(bucket, info.max_stack_size))
    }
}

/// What identifies a seed-identified item: its record under a
/// non-zero seed. Zero is the seed of an item the game never rolled —
/// a GD Stash creation, a reagent vaulted out of the storage — so it
/// identifies nothing and such an item is never a duplicate.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SeedKey {
    record: String,
    seed: u32,
}

impl SeedKey {
    #[must_use]
    pub fn of(item: &Item, identities: &impl Identities) -> Option<Self> {
        match (identities.identity(item)?, item.seed) {
            (Identity::Seed, seed) if seed != 0 => Some(Self {
                record: item.base_name.clone(),
                seed,
            }),
            (Identity::Seed | Identity::Stack, _) => None,
        }
    }
}

/// The seed keys the store holds, grown by each item a bulk move
/// admits so a second copy in the same batch is a duplicate of the
/// first.
#[derive(Debug, Default)]
pub struct HeldSeeds(HashSet<SeedKey>);

impl HeldSeeds {
    #[must_use]
    pub fn of(store: &VaultStore, identities: &impl Identities) -> Self {
        Self(
            store
                .items()
                .iter()
                .filter_map(|stored| SeedKey::of(stored.item(), identities))
                .collect(),
        )
    }

    fn admit(
        &mut self,
        item: &Item,
        identities: &impl Identities,
        rule: BulkDuplicates,
    ) -> Admission {
        match (rule, SeedKey::of(item, identities)) {
            (BulkDuplicates::Skip, Some(key)) => {
                if self.0.insert(key) {
                    Admission::New
                } else {
                    Admission::Duplicate
                }
            }
            (BulkDuplicates::Skip, None) | (BulkDuplicates::Allow, Some(_) | None) => {
                Admission::New
            }
        }
    }

    /// Whether `item` is a duplicate of something already held,
    /// without admitting it.
    #[must_use]
    pub fn holds(&self, item: &Item, identities: &impl Identities) -> bool {
        SeedKey::of(item, identities).is_some_and(|key| self.0.contains(&key))
    }
}

enum Admission {
    New,
    Duplicate,
}

/// A bulk operation's decision over one container: the items to take,
/// in container order, and how many stay as duplicates. A move walks
/// them last index first so each removal leaves the rest where they
/// are; a copy walks them in order so the store's ids follow the
/// container's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPlan {
    pub moving: Vec<ItemIndex>,
    pub duplicates: usize,
}

impl TabPlan {
    /// Decides every item of `items` against `held`, which grows with
    /// each item admitted under [`BulkDuplicates::Skip`]; under
    /// [`BulkDuplicates::Allow`] everything is taken.
    #[must_use]
    pub fn of<'a>(
        items: impl IntoIterator<Item = &'a Item>,
        held: &mut HeldSeeds,
        identities: &impl Identities,
        rule: BulkDuplicates,
    ) -> Self {
        let mut moving = Vec::new();
        let mut duplicates = 0;
        for (slot, item) in items.into_iter().enumerate() {
            match held.admit(item, identities, rule) {
                Admission::New => moving.push(ItemIndex::new(slot)),
                Admission::Duplicate => duplicates += 1,
            }
        }
        Self { moving, duplicates }
    }

    fn over<'a>(
        items: impl IntoIterator<Item = &'a Item>,
        store: &VaultStore,
        identities: &impl Identities,
        rule: BulkDuplicates,
    ) -> Self {
        Self::of(
            items,
            &mut HeldSeeds::of(store, identities),
            identities,
            rule,
        )
    }

    fn removals(&self) -> impl Iterator<Item = ItemIndex> + '_ {
        self.moving.iter().rev().copied()
    }
}

/// What a bulk move or copy did: the ids allocated and the duplicates
/// left where they were.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BulkSummary {
    pub moved: Vec<StoredItemId>,
    pub duplicates: usize,
}

impl BulkSummary {
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.moved.is_empty()
    }
}

impl fmt::Display for BulkSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} moved, {} duplicate(s) left in place",
            self.moved.len(),
            self.duplicates
        )
    }
}

/// What emptying a tab did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClearSummary {
    pub removed: usize,
}

impl ClearSummary {
    #[must_use]
    pub fn is_noop(self) -> bool {
        self.removed == 0
    }
}

impl fmt::Display for ClearSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} item(s) deleted", self.removed)
    }
}

/// Moves every item of transfer-stash tab `tab` the rule admits into
/// the store, each with [`ItemOrigin::TransferStash`]; the rest stay
/// in the tab.
///
/// # Errors
/// [`TransferError::NoSuchTab`], before anything moves.
pub fn vault_tab(
    stash: &mut TransferStash,
    campaign: &Campaign,
    tab: TabIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let plan = plan_for(&stash.tabs, tab, store, identities, rule)?;
    execute(plan.removals(), plan.duplicates, |index| {
        transfer::vault_from_stash(stash, campaign, tab, index, store, at)
    })
}

/// Copies every item of transfer-stash tab `tab` the rule admits into
/// the store, each with [`ItemOrigin::TransferStash`]; the tab is
/// untouched.
///
/// # Errors
/// [`TransferError::NoSuchTab`], before anything is copied.
pub fn copy_tab(
    stash: &TransferStash,
    campaign: &Campaign,
    tab: TabIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let origin = ItemOrigin::TransferStash {
        campaign: campaign.clone(),
        tab,
    };
    copy_from_tabs(&stash.tabs, tab, &origin, store, identities, rule, at)
}

/// Moves every item of tab `tab` of `player`'s own stash the rule
/// admits into the store, each with [`ItemOrigin::CharacterStash`]
/// under `realm`; the rest stay in the tab.
///
/// # Errors
/// [`TransferError::NoPlayerStash`] or [`TransferError::NoSuchTab`],
/// before anything moves.
pub fn vault_player_tab(
    player: &mut PlayerFile,
    realm: Realm,
    tab: TabIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let tabs = &player.stash().ok_or(TransferError::NoPlayerStash)?.tabs;
    let plan = plan_for(tabs, tab, store, identities, rule)?;
    execute(plan.removals(), plan.duplicates, |index| {
        transfer::vault_from_player_stash(player, realm, tab, index, store, at)
    })
}

/// Copies every item of tab `tab` of `player`'s own stash the rule
/// admits into the store, each with [`ItemOrigin::CharacterStash`]
/// under `realm`; the tab is untouched.
///
/// # Errors
/// [`TransferError::NoPlayerStash`] or [`TransferError::NoSuchTab`],
/// before anything is copied.
pub fn copy_player_tab(
    player: &PlayerFile,
    realm: Realm,
    tab: TabIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let tabs = &player.stash().ok_or(TransferError::NoPlayerStash)?.tabs;
    let origin = ItemOrigin::CharacterStash {
        realm,
        name: player.character_name().to_owned(),
        tab,
    };
    copy_from_tabs(tabs, tab, &origin, store, identities, rule, at)
}

/// Moves every item of sack `sack` of `player`'s inventory the rule
/// admits into the store, each with [`ItemOrigin::Character`] under
/// `realm`; the rest stay in the sack.
///
/// # Errors
/// [`TransferError::NoInventory`] or [`TransferError::NoSuchSack`],
/// before anything moves.
pub fn vault_sack(
    player: &mut PlayerFile,
    realm: Realm,
    sack: SackIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let items = sack_ref(player, sack)?
        .items
        .iter()
        .map(|placed| &placed.item);
    let plan = TabPlan::over(items, store, identities, rule);
    execute(plan.removals(), plan.duplicates, |index| {
        transfer::vault_from_sack(player, realm, sack, index, store, at)
    })
}

/// Copies every item of sack `sack` of `player`'s inventory the rule
/// admits into the store, each with [`ItemOrigin::Character`] under
/// `realm`; the sack is untouched.
///
/// # Errors
/// [`TransferError::NoInventory`] or [`TransferError::NoSuchSack`],
/// before anything is copied.
pub fn copy_sack(
    player: &PlayerFile,
    realm: Realm,
    sack: SackIndex,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let contents = &sack_ref(player, sack)?.items;
    let origin = ItemOrigin::Character {
        realm,
        name: player.character_name().to_owned(),
        sack,
    };
    let plan = TabPlan::over(
        contents.iter().map(|placed| &placed.item),
        store,
        identities,
        rule,
    );
    execute(plan.moving.iter().copied(), plan.duplicates, |index| {
        contents
            .get(index.value())
            .map(|placed| store.add(placed.item.clone(), origin.clone(), at))
            .ok_or(TransferError::NoSuchSackItem { sack, index })
    })
}

/// Removes every item of `tab` of `tabs`; nothing enters the store.
///
/// # Errors
/// [`TransferError::NoSuchTab`].
pub fn clear_tab(tabs: &mut [StashTab], tab: TabIndex) -> Result<ClearSummary, TransferError> {
    let items = &mut tab_mut(tabs, tab)?.items;
    let removed = items.len();
    items.clear();
    Ok(ClearSummary { removed })
}

/// The plan for `tab` of `tabs` against what `store` holds.
///
/// # Errors
/// [`TransferError::NoSuchTab`].
pub fn plan_for(
    tabs: &[StashTab],
    tab: TabIndex,
    store: &VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
) -> Result<TabPlan, TransferError> {
    let items = tab_ref(tabs, tab)?.items.iter().map(|placed| &placed.item);
    Ok(TabPlan::over(items, store, identities, rule))
}

fn copy_from_tabs(
    tabs: &[StashTab],
    tab: TabIndex,
    origin: &ItemOrigin,
    store: &mut VaultStore,
    identities: &impl Identities,
    rule: BulkDuplicates,
    at: Timestamp,
) -> Result<BulkSummary, TransferError> {
    let items = &tab_ref(tabs, tab)?.items;
    let plan = TabPlan::over(
        items.iter().map(|placed| &placed.item),
        store,
        identities,
        rule,
    );
    execute(plan.moving.iter().copied(), plan.duplicates, |index| {
        items
            .get(index.value())
            .map(|placed| store.add(placed.item.clone(), origin.clone(), at))
            .ok_or(TransferError::NoSuchItem { tab, index })
    })
}

fn execute(
    taking: impl Iterator<Item = ItemIndex>,
    duplicates: usize,
    take: impl FnMut(ItemIndex) -> Result<StoredItemId, TransferError>,
) -> Result<BulkSummary, TransferError> {
    let moved = taking.map(take).collect::<Result<Vec<_>, _>>()?;
    Ok(BulkSummary { moved, duplicates })
}

/// What a purge did: how many of the tab's items it deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurgeSummary {
    pub removed: usize,
}

impl PurgeSummary {
    #[must_use]
    pub fn is_noop(self) -> bool {
        self.removed == 0
    }
}

impl fmt::Display for PurgeSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} duplicate(s) deleted", self.removed)
    }
}

/// The items of `tab` the store already holds by record and roll
/// seed, last index first so each removal leaves the rest where they
/// are — the pure half of [`purge_duplicates`]. Membership is the
/// store's alone: two in-tab copies of a seed the store lacks both
/// stay.
///
/// # Errors
/// [`TransferError::NoSuchTab`].
pub fn purge_plan(
    tabs: &[StashTab],
    tab: TabIndex,
    store: &VaultStore,
    identities: &impl Identities,
) -> Result<Vec<ItemIndex>, TransferError> {
    let held = HeldSeeds::of(store, identities);
    let mut doomed: Vec<ItemIndex> = tab_ref(tabs, tab)?
        .items
        .iter()
        .enumerate()
        .filter(|(_, placed)| held.holds(&placed.item, identities))
        .map(|(slot, _)| ItemIndex::new(slot))
        .collect();
    doomed.reverse();
    Ok(doomed)
}

/// Deletes from tab `tab` of `tabs` every item the store already
/// holds by record and roll seed; the store is never changed.
///
/// # Errors
/// [`TransferError::NoSuchTab`], before anything is deleted.
pub fn purge_duplicates(
    tabs: &mut [StashTab],
    tab: TabIndex,
    store: &VaultStore,
    identities: &impl Identities,
) -> Result<PurgeSummary, TransferError> {
    let doomed = purge_plan(tabs, tab, store, identities)?;
    let items = &mut tab_mut(tabs, tab)?.items;
    for index in &doomed {
        items.remove(index.value());
    }
    Ok(PurgeSummary {
        removed: doomed.len(),
    })
}

/// Deletes from tab `tab` of `player`'s own stash every item the
/// store already holds by record and roll seed; the store is never
/// changed.
///
/// # Errors
/// [`TransferError::NoPlayerStash`] or [`TransferError::NoSuchTab`],
/// before anything is deleted.
pub fn purge_player_duplicates(
    player: &mut PlayerFile,
    tab: TabIndex,
    store: &VaultStore,
    identities: &impl Identities,
) -> Result<PurgeSummary, TransferError> {
    purge_duplicates(transfer::player_tabs_mut(player)?, tab, store, identities)
}

/// What the additive sync did: one new stack per record whose in-game
/// count exceeded the vault's, and the units added in all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub raised: Vec<StoredItemId>,
    pub units: u64,
}

impl SyncSummary {
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.raised.is_empty()
    }
}

impl fmt::Display for SyncSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} record(s) raised by {} unit(s)",
            self.raised.len(),
            self.units
        )
    }
}

/// One record the storage holds more of than the store: the record
/// and the shortfall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortfall {
    pub record: String,
    pub units: u32,
}

/// The records whose in-game count exceeds what the store holds bare
/// (no affixes) of them, in storage order — the pure half of
/// [`sync_reagents`]. A store item written without a stack counts as
/// one, as it does when placed into the storage.
#[must_use]
pub fn reagent_shortfall(storage: &ReagentStorage, store: &VaultStore) -> Vec<Shortfall> {
    let mut in_game: BTreeMap<&str, u64> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    for entry in &storage.entries {
        let slot = in_game.entry(entry.record.as_str()).or_insert_with(|| {
            order.push(entry.record.as_str());
            0
        });
        *slot = slot.saturating_add(u64::from(entry.count));
    }
    let mut held: BTreeMap<&str, u64> = BTreeMap::new();
    for item in store.items().iter().map(StoredItem::item) {
        if is_bare(item) && in_game.contains_key(item.base_name.as_str()) {
            let slot = held.entry(item.base_name.as_str()).or_default();
            *slot = slot.saturating_add(u64::from(stack_of(item)));
        }
    }
    order
        .into_iter()
        .filter_map(|record| {
            let missing = in_game[record].saturating_sub(held.get(record).copied().unwrap_or(0));
            (missing > 0).then(|| Shortfall {
                record: record.to_string(),
                units: u32::try_from(missing).unwrap_or(u32::MAX),
            })
        })
        .collect()
}

fn is_bare(item: &Item) -> bool {
    item.prefix_name.is_empty() && item.suffix_name.is_empty()
}

/// Raises the store's count of every record in `storage` to the
/// in-game count by adding one stack of the shortfall with
/// [`ItemOrigin::ReagentStorage`]; never removes or reduces anything,
/// and never touches the storage.
pub fn sync_reagents(
    storage: &ReagentStorage,
    campaign: &Campaign,
    store: &mut VaultStore,
    at: Timestamp,
) -> SyncSummary {
    let shortfall = reagent_shortfall(storage, store);
    let origin = ItemOrigin::ReagentStorage {
        campaign: campaign.clone(),
    };
    let units = shortfall.iter().map(|short| u64::from(short.units)).sum();
    let raised = shortfall
        .into_iter()
        .map(|short| {
            store.add(
                Item {
                    base_name: short.record,
                    stack_count: short.units,
                    ..Item::default()
                },
                origin.clone(),
                at,
            )
        })
        .collect();
    SyncSummary { raised, units }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use univault_engine::arz::ArzDialect;
    use univault_engine::arz::ArzFile;
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::text::TextDb;

    use super::*;
    use crate::block::TabDecoration;
    use crate::gdc::{
        Block, CharacterInfo, Inventory, InventoryContents, InventoryState, PlayerHeader,
        PlayerStash, Sack, Sex,
    };
    use crate::gst::{ReagentEntry, ReagentStorageVersion};
    use crate::item::{ContainerVersion, SackItem, StashItem};

    const SWORD: &str = "records/items/gearweapons/swords/a.dbr";
    const COMPONENT: &str = "records/items/materia/c.dbr";
    const POTION: &str = "records/items/potions/p.dbr";
    const STACKED_SWORD: &str = "records/items/gearweapons/swords/stacked.dbr";
    const MATERIAL: &str = "records/items/crafting/materials/m.dbr";
    const PREFIX: &str = "records/items/lootaffixes/prefix/p.dbr";
    const NOW: Timestamp = Timestamp::from_unix_seconds(1_756_900_000);
    const TAB0: TabIndex = TabIndex::new(0);
    const SKIP: BulkDuplicates = BulkDuplicates::Skip;
    const ALLOW: BulkDuplicates = BulkDuplicates::Allow;

    struct Rules(HashMap<&'static str, Identity>);

    impl Identities for Rules {
        fn identity(&self, item: &Item) -> Option<Identity> {
            self.0.get(item.base_name.as_str()).copied()
        }
    }

    fn rules() -> Rules {
        Rules(HashMap::from([
            (SWORD, Identity::Seed),
            (COMPONENT, Identity::Stack),
        ]))
    }

    fn database() -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            SWORD,
            "WeaponMelee_Sword",
            &[("Class", Values::Strings(&["WeaponMelee_Sword"]))],
        );
        builder.record(
            STACKED_SWORD,
            "WeaponMelee_Sword",
            &[
                ("Class", Values::Strings(&["WeaponMelee_Sword"])),
                ("maxStackSize", Values::Ints(&[5])),
            ],
        );
        builder.record(
            COMPONENT,
            "ItemRelic",
            &[("Class", Values::Strings(&["ItemRelic"]))],
        );
        builder.record(
            POTION,
            "OneShot_PotionHealth",
            &[
                ("Class", Values::Strings(&["OneShot_PotionHealth"])),
                ("maxStackSize", Values::Ints(&[100])),
            ],
        );
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        GameData::from_parts(vec![database], TextDb::new(), Vec::new())
    }

    fn item(base: &str, seed: u32) -> Item {
        Item {
            base_name: base.into(),
            seed,
            stack_count: 1,
            ..Item::default()
        }
    }

    fn placed(base: &str, seed: u32, x: f32) -> StashItem {
        StashItem {
            item: item(base, seed),
            x,
            y: 0.0,
        }
    }

    fn stash(items: Vec<StashItem>) -> TransferStash {
        TransferStash {
            version: ContainerVersion::new(11).unwrap(),
            mod_name: String::new(),
            expansion_status: 7,
            tabs: vec![StashTab {
                width: 18,
                height: 16,
                items,
                decoration: TabDecoration::default(),
            }],
        }
    }

    fn storage(entries: &[(&str, u32)]) -> ReagentStorage {
        ReagentStorage {
            version: ReagentStorageVersion::new(1).unwrap(),
            mod_name: String::new(),
            entries: entries
                .iter()
                .map(|(record, count)| ReagentEntry {
                    record: (*record).to_string(),
                    count: *count,
                })
                .collect(),
        }
    }

    fn origin() -> ItemOrigin {
        ItemOrigin::Unknown
    }

    fn in_sack(base: &str, seed: u32, x: u32) -> SackItem {
        SackItem {
            item: item(base, seed),
            x,
            y: 0,
        }
    }

    fn player(sack_items: Vec<SackItem>, tab_items: Vec<StashItem>) -> PlayerFile {
        let inventory = Inventory {
            version: ContainerVersion::new(11).unwrap(),
            flag: 0,
            state: InventoryState::Entered(Box::new(InventoryContents {
                focused_sack: 0,
                selected_sack: 0,
                sacks: vec![Sack {
                    flag: 1,
                    items: sack_items,
                }],
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
                    tabs: vec![StashTab {
                        width: 8,
                        height: 16,
                        items: tab_items,
                        decoration: TabDecoration::default(),
                    }],
                }),
            ],
        )
    }

    fn origins_of(store: &VaultStore, ids: &[StoredItemId]) -> Vec<ItemOrigin> {
        ids.iter()
            .map(|id| store.get(*id).unwrap().origin().clone())
            .collect()
    }

    #[test]
    fn identity_follows_the_stack_size_then_the_bucket() {
        let game = database();
        assert_eq!(game.identity(&item(SWORD, 1)), Some(Identity::Seed));
        assert_eq!(
            game.identity(&item(STACKED_SWORD, 1)),
            Some(Identity::Stack)
        );
        assert_eq!(game.identity(&item(COMPONENT, 1)), Some(Identity::Stack));
        assert_eq!(game.identity(&item(POTION, 1)), Some(Identity::Stack));
        assert_eq!(game.identity(&item("records/items/nowhere.dbr", 1)), None);
        assert_eq!(Identity::of(Bucket::Relic, None), Identity::Seed);
        assert_eq!(Identity::of(Bucket::Ring, Some(1)), Identity::Seed);
        assert_eq!(Identity::of(Bucket::Ring, Some(2)), Identity::Stack);
        assert_eq!(Identity::of(Bucket::Misc, None), Identity::Stack);
    }

    #[test]
    fn a_seed_key_needs_a_seed_identified_record_and_a_rolled_seed() {
        let rules = rules();
        assert!(SeedKey::of(&item(SWORD, 7), &rules).is_some());
        assert_eq!(SeedKey::of(&item(SWORD, 0), &rules), None);
        assert_eq!(SeedKey::of(&item(COMPONENT, 7), &rules), None);
        assert_eq!(SeedKey::of(&item(MATERIAL, 7), &rules), None);
    }

    #[test]
    fn vault_tab_leaves_duplicates_in_place_and_moves_the_rest() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 7), origin(), NOW);
        let mut stash = stash(vec![
            placed(SWORD, 7, 0.0),
            placed(SWORD, 8, 2.0),
            placed(COMPONENT, 7, 4.0),
            placed(SWORD, 0, 6.0),
        ]);
        let summary = vault_tab(
            &mut stash,
            &Campaign::Main,
            TAB0,
            &mut store,
            &rules,
            SKIP,
            NOW,
        )
        .unwrap();
        assert_eq!(summary.moved.len(), 3);
        assert_eq!(summary.duplicates, 1);
        assert_eq!(summary.to_string(), "3 moved, 1 duplicate(s) left in place");
        assert_eq!(stash.tabs[0].items, vec![placed(SWORD, 7, 0.0)]);
        assert_eq!(store.len(), 4);
        for id in &summary.moved {
            assert_eq!(
                store.get(*id).unwrap().origin(),
                &ItemOrigin::TransferStash {
                    campaign: Campaign::Main,
                    tab: TAB0
                }
            );
        }
    }

    #[test]
    fn allowing_duplicates_empties_the_tab() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 7), origin(), NOW);
        let mut stash = stash(vec![
            placed(SWORD, 7, 0.0),
            placed(SWORD, 7, 2.0),
            placed(COMPONENT, 7, 4.0),
        ]);
        let summary = vault_tab(
            &mut stash,
            &Campaign::Main,
            TAB0,
            &mut store,
            &rules,
            ALLOW,
            NOW,
        )
        .unwrap();
        assert_eq!(summary.moved.len(), 3);
        assert_eq!(summary.duplicates, 0);
        assert!(stash.tabs[0].items.is_empty());
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn two_copies_in_one_tab_move_the_first_and_leave_the_second() {
        let rules = rules();
        let mut store = VaultStore::new();
        let mut stash = stash(vec![placed(SWORD, 9, 0.0), placed(SWORD, 9, 2.0)]);
        let summary = vault_tab(
            &mut stash,
            &Campaign::Main,
            TAB0,
            &mut store,
            &rules,
            SKIP,
            NOW,
        )
        .unwrap();
        assert_eq!(summary.moved.len(), 1);
        assert_eq!(summary.duplicates, 1);
        assert_eq!(stash.tabs[0].items, vec![placed(SWORD, 9, 2.0)]);
        assert_eq!(store.get(summary.moved[0]).unwrap().item(), &item(SWORD, 9));
    }

    #[test]
    fn copy_tab_leaves_the_tab_whole_and_records_the_stash_origin() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 7), origin(), NOW);
        let stash = stash(vec![
            placed(SWORD, 7, 0.0),
            placed(SWORD, 8, 2.0),
            placed(SWORD, 8, 4.0),
            placed(COMPONENT, 7, 6.0),
        ]);
        let before = stash.clone();
        let summary =
            copy_tab(&stash, &Campaign::Main, TAB0, &mut store, &rules, SKIP, NOW).unwrap();
        assert_eq!(stash, before);
        assert_eq!(summary.duplicates, 2);
        assert_eq!(
            summary
                .moved
                .iter()
                .map(|id| store.get(*id).unwrap().item().clone())
                .collect::<Vec<_>>(),
            vec![item(SWORD, 8), item(COMPONENT, 7)]
        );
        let stash_origin = ItemOrigin::TransferStash {
            campaign: Campaign::Main,
            tab: TAB0,
        };
        assert_eq!(
            origins_of(&store, &summary.moved),
            vec![stash_origin.clone(), stash_origin]
        );
        let again = copy_tab(
            &stash,
            &Campaign::Main,
            TAB0,
            &mut store,
            &rules,
            ALLOW,
            NOW,
        )
        .unwrap();
        assert_eq!(again.moved.len(), 4);
        assert_eq!(again.duplicates, 0);
        assert_eq!(store.len(), 7);
    }

    #[test]
    fn a_character_tab_moves_and_copies_under_its_own_origin() {
        let rules = rules();
        let mut store = VaultStore::new();
        let mut player = player(vec![], vec![placed(SWORD, 3, 0.0), placed(SWORD, 3, 2.0)]);
        let realm = Realm::Custom;
        let own = ItemOrigin::CharacterStash {
            realm,
            name: "Sif".into(),
            tab: TAB0,
        };
        let copied = copy_player_tab(&player, realm, TAB0, &mut store, &rules, SKIP, NOW).unwrap();
        assert_eq!(copied.moved.len(), 1);
        assert_eq!(copied.duplicates, 1);
        assert_eq!(player.stash().unwrap().tabs[0].items.len(), 2);
        assert_eq!(origins_of(&store, &copied.moved), vec![own.clone()]);
        let moved =
            vault_player_tab(&mut player, realm, TAB0, &mut store, &rules, SKIP, NOW).unwrap();
        assert!(moved.is_noop());
        assert_eq!(moved.duplicates, 2);
        let moved =
            vault_player_tab(&mut player, realm, TAB0, &mut store, &rules, ALLOW, NOW).unwrap();
        assert_eq!(moved.moved.len(), 2);
        assert!(player.stash().unwrap().tabs[0].items.is_empty());
        assert_eq!(origins_of(&store, &moved.moved), vec![own.clone(), own]);
    }

    #[test]
    fn a_sack_moves_and_copies_under_its_own_origin() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 5), origin(), NOW);
        let mut player = player(
            vec![
                in_sack(SWORD, 5, 0),
                in_sack(SWORD, 6, 2),
                in_sack(COMPONENT, 5, 4),
            ],
            vec![],
        );
        let realm = Realm::Main;
        let sack = SackIndex::MAIN;
        let own = ItemOrigin::Character {
            realm,
            name: "Sif".into(),
            sack,
        };
        let copied = copy_sack(&player, realm, sack, &mut store, &rules, SKIP, NOW).unwrap();
        assert_eq!(copied.moved.len(), 2);
        assert_eq!(copied.duplicates, 1);
        assert_eq!(player.inventory().unwrap().sacks()[0].items.len(), 3);
        assert_eq!(
            origins_of(&store, &copied.moved),
            vec![own.clone(), own.clone()]
        );
        let moved = vault_sack(&mut player, realm, sack, &mut store, &rules, SKIP, NOW).unwrap();
        assert_eq!(moved.moved.len(), 1);
        assert_eq!(moved.duplicates, 2);
        assert_eq!(
            player.inventory().unwrap().sacks()[0].items,
            vec![in_sack(SWORD, 5, 0), in_sack(SWORD, 6, 2)]
        );
        assert_eq!(origins_of(&store, &moved.moved), vec![own]);
        assert_eq!(
            vault_sack(
                &mut player,
                realm,
                SackIndex::new(4),
                &mut store,
                &rules,
                SKIP,
                NOW
            ),
            Err(TransferError::NoSuchSack(SackIndex::new(4)))
        );
        assert_eq!(
            copy_sack(
                &player,
                realm,
                SackIndex::new(4),
                &mut store,
                &rules,
                SKIP,
                NOW
            ),
            Err(TransferError::NoSuchSack(SackIndex::new(4)))
        );
    }

    #[test]
    fn clear_tab_deletes_everything_and_refuses_a_missing_tab() {
        let mut stash = stash(vec![placed(SWORD, 9, 0.0), placed(COMPONENT, 1, 2.0)]);
        assert_eq!(
            clear_tab(&mut stash.tabs, TabIndex::new(3)),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
        assert_eq!(stash.tabs[0].items.len(), 2);
        let summary = clear_tab(&mut stash.tabs, TAB0).unwrap();
        assert_eq!(summary, ClearSummary { removed: 2 });
        assert_eq!(summary.to_string(), "2 item(s) deleted");
        assert!(stash.tabs[0].items.is_empty());
        assert!(clear_tab(&mut stash.tabs, TAB0).unwrap().is_noop());
    }

    #[test]
    fn a_missing_tab_is_refused_before_anything_moves() {
        let rules = rules();
        let mut store = VaultStore::new();
        let mut stash = stash(vec![placed(SWORD, 9, 0.0)]);
        let before = stash.clone();
        assert_eq!(
            vault_tab(
                &mut stash,
                &Campaign::Main,
                TabIndex::new(3),
                &mut store,
                &rules,
                SKIP,
                NOW
            ),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
        assert_eq!(stash, before);
        assert!(store.is_empty());
        let plan = plan_for(&stash.tabs, TAB0, &store, &rules, SKIP).unwrap();
        assert_eq!(
            plan,
            TabPlan {
                moving: vec![ItemIndex::new(0)],
                duplicates: 0
            }
        );
    }

    #[test]
    fn the_purge_deletes_only_what_the_store_holds_and_never_touches_the_store() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 7), origin(), NOW);
        store.add(item(COMPONENT, 7), origin(), NOW);
        let mut stash = stash(vec![
            placed(SWORD, 7, 0.0),
            placed(SWORD, 8, 2.0),
            placed(COMPONENT, 7, 4.0),
            placed(SWORD, 0, 6.0),
            placed(SWORD, 9, 8.0),
            placed(SWORD, 9, 10.0),
            placed(SWORD, 7, 12.0),
        ]);
        assert_eq!(
            purge_plan(&stash.tabs, TAB0, &store, &rules).unwrap(),
            vec![ItemIndex::new(6), ItemIndex::new(0)]
        );
        let summary = purge_duplicates(&mut stash.tabs, TAB0, &store, &rules).unwrap();
        assert_eq!(summary, PurgeSummary { removed: 2 });
        assert_eq!(summary.to_string(), "2 duplicate(s) deleted");
        assert_eq!(
            stash.tabs[0].items,
            vec![
                placed(SWORD, 8, 2.0),
                placed(COMPONENT, 7, 4.0),
                placed(SWORD, 0, 6.0),
                placed(SWORD, 9, 8.0),
                placed(SWORD, 9, 10.0),
            ]
        );
        assert_eq!(store.len(), 2);
        assert!(
            purge_duplicates(&mut stash.tabs, TAB0, &store, &rules)
                .unwrap()
                .is_noop()
        );
    }

    #[test]
    fn a_missing_tab_is_refused_before_anything_is_purged() {
        let rules = rules();
        let mut store = VaultStore::new();
        store.add(item(SWORD, 9), origin(), NOW);
        let mut stash = stash(vec![placed(SWORD, 9, 0.0)]);
        let before = stash.clone();
        assert_eq!(
            purge_duplicates(&mut stash.tabs, TabIndex::new(3), &store, &rules),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
        assert_eq!(stash, before);
        assert_eq!(
            purge_plan(&stash.tabs, TabIndex::new(3), &store, &rules),
            Err(TransferError::NoSuchTab(TabIndex::new(3)))
        );
    }

    #[test]
    fn the_sync_raises_the_vault_to_the_in_game_count_and_never_lowers_it() {
        let mut store = VaultStore::new();
        store.add(
            Item {
                stack_count: 12,
                ..item(COMPONENT, 0)
            },
            origin(),
            NOW,
        );
        store.add(
            Item {
                stack_count: 3,
                prefix_name: PREFIX.into(),
                ..item(COMPONENT, 0)
            },
            origin(),
            NOW,
        );
        let full = storage(&[(COMPONENT, 20), (MATERIAL, 5)]);
        assert_eq!(
            reagent_shortfall(&full, &store),
            vec![
                Shortfall {
                    record: COMPONENT.into(),
                    units: 8
                },
                Shortfall {
                    record: MATERIAL.into(),
                    units: 5
                },
            ]
        );
        let summary = sync_reagents(&full, &Campaign::Main, &mut store, NOW);
        assert_eq!(summary.raised.len(), 2);
        assert_eq!(summary.units, 13);
        assert_eq!(summary.to_string(), "2 record(s) raised by 13 unit(s)");
        let added = store.get(summary.raised[0]).unwrap();
        assert_eq!(
            added.item(),
            &Item {
                base_name: COMPONENT.into(),
                stack_count: 8,
                ..Item::default()
            }
        );
        assert_eq!(
            added.origin(),
            &ItemOrigin::ReagentStorage {
                campaign: Campaign::Main
            }
        );
        assert!(sync_reagents(&full, &Campaign::Main, &mut store, NOW).is_noop());
        let lowered = storage(&[(COMPONENT, 10)]);
        assert!(sync_reagents(&lowered, &Campaign::Main, &mut store, NOW).is_noop());
        assert_eq!(store.len(), 4);
    }

    #[test]
    fn the_sync_counts_an_unstacked_store_item_as_one_and_sums_split_entries() {
        let mut store = VaultStore::new();
        store.add(
            Item {
                stack_count: 0,
                ..item(COMPONENT, 0)
            },
            origin(),
            NOW,
        );
        let split = storage(&[(COMPONENT, 2), (COMPONENT, 3)]);
        assert_eq!(
            reagent_shortfall(&split, &store),
            vec![Shortfall {
                record: COMPONENT.into(),
                units: 4
            }]
        );
    }
}
