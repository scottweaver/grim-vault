//! The vault store: this app's own authoritative file for vaulted items
//! (ARCHITECTURE.md "Source of truth"). One flat, versioned,
//! self-describing JSON document — format tag [`FORMAT_TAG`], version
//! [`FORMAT_VERSION`] — holding every item with a stable id, the full
//! game [`Item`] record, where it came from, and when it was stored.
//! Nothing about an entry depends on its position in the file or on any
//! other entry, and type buckets are computed from the item itself
//! ([`crate::bucket`]), never stored, so an entry cannot be misfiled.
//!
//! Ids are monotonic and never reused: `nextId` only grows, and a file
//! whose `nextId` lags its items is healed to `max id + 1` on load.
//! Unknown top-level fields survive a load/save cycle untouched, so a
//! newer writer's additions are not erased by an older reader; a newer
//! *version* is refused outright.
//!
//! Beside the items the store keeps what the player has learned: the
//! blueprints ([`LearnedBlueprint`]), one entry per record, each
//! naming the campaign it was first seen learned in and when —
//! knowledge, not items; nothing here can be placed in a game grid.
//! The list is absent from a file that has none, so a store written
//! before it looks the same.
//!
//! The document shape:
//!
//! ```json
//! {
//!   "format": "grimvault-store",
//!   "version": 1,
//!   "nextId": 2,
//!   "items": [
//!     {
//!       "id": 1,
//!       "origin": { "kind": "transferStash", "tab": 0 },
//!       "storedAt": 1756900000,
//!       "item": { "baseName": "records/items/materia/a.dbr", "prefixName": "", "...": "..." }
//!     }
//!   ],
//!   "blueprints": [
//!     { "record": "records/items/crafting/blueprints/b.dbr", "campaign": "main", "learnedAt": 1756900000 }
//!   ]
//! }
//! ```

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use univault_engine::ids::normalize;

use crate::campaign::Campaign;
use crate::gdc::{EquipSlot, Realm};
use crate::gds::GameMode;
use crate::item::Item;
use crate::transfer::{SackIndex, TabIndex};

/// The `format` tag every store file carries.
pub const FORMAT_TAG: &str = "grimvault-store";
/// The newest document version this crate reads and the one it writes.
pub const FORMAT_VERSION: u32 = 1;

/// Stable identity of a stored item, allocated by [`VaultStore::add`]
/// and never reused within a store. Orders by allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StoredItemId(u64);

impl StoredItemId {
    /// An id as a caller spells it (a lookup key, not an allocation).
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw id.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    const fn successor(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for StoredItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A moment in Unix seconds, supplied by the caller — the core has no
/// clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    #[must_use]
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    #[must_use]
    pub const fn unix_seconds(self) -> u64 {
        self.0
    }
}

/// Where a stored item was taken from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemOrigin {
    /// A tab of a campaign's `transfer.gst`.
    TransferStash {
        #[serde(default = "campaign_before_mods_were_recorded")]
        campaign: Campaign,
        tab: TabIndex,
    },
    /// A sack of a character's inventory (`player.gdc` block 3).
    Character {
        #[serde(default = "realm_before_realms_were_recorded")]
        realm: Realm,
        name: String,
        sack: SackIndex,
    },
    /// A tab of a character's own stash (`player.gdc` block 4).
    CharacterStash {
        #[serde(default = "realm_before_realms_were_recorded")]
        realm: Realm,
        name: String,
        tab: TabIndex,
    },
    /// A slot of a character's worn gear or weapon sets (`player.gdc`
    /// block 3), taken off in this app.
    Equipped {
        realm: Realm,
        name: String,
        slot: EquipSlot,
    },
    /// A campaign's component / crafting-material storage,
    /// `reagents.gst`.
    ReagentStorage {
        #[serde(default = "campaign_before_mods_were_recorded")]
        campaign: Campaign,
    },
    /// An entry of a GD Stash export (`.gds`, [`crate::gds`]): the
    /// export's file name, and the two facts only that file records —
    /// the mode the item was played in and the character it is
    /// soulbound to, `None` when unbound.
    GdStashExport {
        file: String,
        mode: GameMode,
        owner: Option<String>,
    },
    /// Provenance not recorded.
    Unknown,
}

impl fmt::Display for ItemOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransferStash { campaign, tab } => {
                write!(f, "{campaign} transfer stash tab {tab}")
            }
            Self::Character { realm, name, sack } => {
                write!(f, "character {}/{name} sack {sack}", realm.dir_name())
            }
            Self::CharacterStash { realm, name, tab } => {
                write!(f, "character {}/{name} stash tab {tab}", realm.dir_name())
            }
            Self::Equipped { realm, name, slot } => {
                write!(f, "character {}/{name} equipped {slot}", realm.dir_name())
            }
            Self::ReagentStorage { campaign } => {
                write!(f, "{campaign} component / crafting-material storage")
            }
            Self::GdStashExport { file, mode, owner } => match owner {
                Some(owner) => write!(f, "GD Stash export {file} ({mode}, soulbound to {owner})"),
                None => write!(f, "GD Stash export {file} ({mode})"),
            },
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// Origins written before 2026-09-06 name no realm; the app then read
/// only `main/`, so such an origin can only be a main-campaign
/// character.
const fn realm_before_realms_were_recorded() -> Realm {
    Realm::Main
}

/// Origins written before 2026-09-06 name no campaign; the app then
/// read only the main campaign's shared files.
const fn campaign_before_mods_were_recorded() -> Campaign {
    Campaign::Main
}

/// One store entry: the item, its identity in the store, and its
/// provenance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredItem {
    id: StoredItemId,
    origin: ItemOrigin,
    stored_at: Timestamp,
    item: Item,
}

impl StoredItem {
    #[must_use]
    pub fn id(&self) -> StoredItemId {
        self.id
    }

    #[must_use]
    pub fn origin(&self) -> &ItemOrigin {
        &self.origin
    }

    #[must_use]
    pub fn stored_at(&self) -> Timestamp {
        self.stored_at
    }

    #[must_use]
    pub fn item(&self) -> &Item {
        &self.item
    }

    /// The game item, leaving the store identity behind.
    #[must_use]
    pub fn into_item(self) -> Item {
        self.item
    }

    /// The game item and its provenance, leaving the store identity
    /// behind.
    #[must_use]
    pub fn into_parts(self) -> (Item, ItemOrigin) {
        (self.item, self.origin)
    }

    /// The vaulting event this entry records — everything but the id,
    /// which is the store's own — so the same entry read from two
    /// copies of a store is recognised as one.
    fn fact(&self) -> Fact<'_> {
        Fact {
            origin: &self.origin,
            stored_at: self.stored_at,
            item: &self.item,
        }
    }
}

/// See [`StoredItem::fact`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Fact<'a> {
    origin: &'a ItemOrigin,
    stored_at: Timestamp,
    item: &'a Item,
}

/// What makes two stacks one stack: the item less its roll seed —
/// which rolls nothing on a stack — and its count.
#[derive(Clone, PartialEq, Eq, Hash)]
struct StackKey(Item);

impl StackKey {
    fn of(item: &Item) -> Self {
        Self(Item {
            seed: 0,
            stack_count: 0,
            ..item.clone()
        })
    }
}

/// A blueprint the player has learned: its `ItemArtifactFormula`
/// record, the campaign it was first seen learned in, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedBlueprint {
    pub record: String,
    pub campaign: Campaign,
    pub learned_at: Timestamp,
}

/// What [`VaultStore::learn_blueprint`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Learned {
    Recorded,
    AlreadyKnown,
}

/// What [`VaultStore::merge`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    /// Entries added under fresh ids: every seed-identified entry the
    /// store lacked, and one stack per stackable record it held none
    /// of.
    pub added: usize,
    /// Entries the store already held, left as they were.
    pub already_held: usize,
    /// Stacks the store held fewer of than the other store, raised
    /// to the other's count.
    pub raised: usize,
    /// Learned blueprints the other store knew and this one did not.
    pub blueprints: usize,
}

impl fmt::Display for Merged {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} added, {} already held, {} stacks raised, {} blueprints recorded",
            self.added, self.already_held, self.raised, self.blueprints
        )
    }
}

/// What [`VaultStore::consolidate_stacks`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Folded {
    /// Entries folded away, their counts added to the stack that
    /// stays.
    pub entries: usize,
    /// Stacks that took them.
    pub stacks: usize,
}

impl fmt::Display for Folded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} entries folded into {} stacks",
            self.entries, self.stacks
        )
    }
}

/// Why a store document was refused.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The `format` tag is not [`FORMAT_TAG`].
    #[error("not a {FORMAT_TAG} document: format tag {found:?}")]
    WrongFormat { found: String },
    /// The document is newer than [`FORMAT_VERSION`].
    #[error("store document version {version} is newer than this app's {FORMAT_VERSION}")]
    UnsupportedVersion { version: u32 },
    /// Two entries claim the same id.
    #[error("stored item {0} appears more than once")]
    DuplicateId(StoredItemId),
    /// The bytes are not the document shape at all.
    #[error("store JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// The in-memory store: entries in file order plus the allocator state.
/// Load with [`VaultStore::from_json`], persist with
/// [`VaultStore::to_json`]; nothing here is authoritative until the
/// shell has written it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultStore {
    next_id: StoredItemId,
    items: Vec<StoredItem>,
    blueprints: Vec<LearnedBlueprint>,
    extra: Map<String, Value>,
}

impl Default for VaultStore {
    fn default() -> Self {
        Self::new()
    }
}

impl VaultStore {
    /// An empty store whose first allocated id is 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: StoredItemId(1),
            items: Vec::new(),
            blueprints: Vec::new(),
            extra: Map::new(),
        }
    }

    /// Parses a store document.
    ///
    /// # Errors
    /// [`StoreError::WrongFormat`] for any tag but [`FORMAT_TAG`],
    /// [`StoreError::UnsupportedVersion`] above [`FORMAT_VERSION`],
    /// [`StoreError::DuplicateId`] when two entries share an id, and
    /// [`StoreError::Json`] when the bytes are not the document shape.
    pub fn from_json(bytes: &[u8]) -> Result<Self, StoreError> {
        let envelope: Envelope = serde_json::from_slice(bytes)?;
        if envelope.format != FORMAT_TAG {
            return Err(StoreError::WrongFormat {
                found: envelope.format,
            });
        }
        if envelope.version > FORMAT_VERSION {
            return Err(StoreError::UnsupportedVersion {
                version: envelope.version,
            });
        }
        let document: StoreFile<'_> = serde_json::from_slice(bytes)?;
        Self::from_document(document)
    }

    fn from_document(document: StoreFile<'_>) -> Result<Self, StoreError> {
        let items = document.items.into_owned();
        let mut seen = HashSet::with_capacity(items.len());
        if let Some(duplicate) = items
            .iter()
            .map(StoredItem::id)
            .find(|id| !seen.insert(*id))
        {
            return Err(StoreError::DuplicateId(duplicate));
        }
        let next_id = items
            .iter()
            .map(StoredItem::id)
            .max()
            .map_or(document.next_id, |max| {
                document.next_id.max(max.successor())
            });
        let mut known: HashSet<String> = HashSet::new();
        let blueprints = document
            .blueprints
            .into_owned()
            .into_iter()
            .filter(|learned| known.insert(normalize(&learned.record)))
            .collect();
        Ok(Self {
            next_id,
            items,
            blueprints,
            extra: document.extra.into_owned(),
        })
    }

    /// The document, pretty-printed with a trailing newline. Key order
    /// is fixed: the envelope fields, then preserved unknown fields in
    /// sorted order.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "the expect guards a local invariant (plain data serializes), not a runtime condition"
    )]
    pub fn to_json(&self) -> Vec<u8> {
        let document = StoreFile {
            format: Cow::Borrowed(FORMAT_TAG),
            version: FORMAT_VERSION,
            next_id: self.next_id,
            items: Cow::Borrowed(&self.items),
            blueprints: Cow::Borrowed(&self.blueprints),
            extra: Cow::Borrowed(&self.extra),
        };
        let mut bytes = serde_json::to_vec_pretty(&document)
            .expect("store types serialize infallibly: string keys only, no fallible Serialize");
        bytes.push(b'\n');
        bytes
    }

    /// Entries in file order.
    #[must_use]
    pub fn items(&self) -> &[StoredItem] {
        &self.items
    }

    /// The learned blueprints, in the order they were recorded.
    #[must_use]
    pub fn blueprints(&self) -> &[LearnedBlueprint] {
        &self.blueprints
    }

    /// Whether a blueprint record is among the learned; record paths
    /// compare the way the game's database keys do.
    #[must_use]
    pub fn knows_blueprint(&self, record: &str) -> bool {
        let wanted = normalize(record);
        self.blueprints
            .iter()
            .any(|learned| normalize(&learned.record) == wanted)
    }

    /// Records a blueprint as learned unless it is known already; the
    /// first campaign and moment stand.
    pub fn learn_blueprint(
        &mut self,
        record: String,
        campaign: Campaign,
        at: Timestamp,
    ) -> Learned {
        if self.knows_blueprint(&record) {
            return Learned::AlreadyKnown;
        }
        self.blueprints.push(LearnedBlueprint {
            record,
            campaign,
            learned_at: at,
        });
        Learned::Recorded
    }

    #[must_use]
    pub fn get(&self, id: StoredItemId) -> Option<&StoredItem> {
        self.items.iter().find(|stored| stored.id == id)
    }

    /// The item of an entry for editing in place — a socket filled or
    /// freed, a stack split — keeping its id and provenance.
    pub fn item_mut(&mut self, id: StoredItemId) -> Option<&mut Item> {
        self.items
            .iter_mut()
            .find(|stored| stored.id == id)
            .map(|stored| &mut stored.item)
    }

    /// Stores an item under a fresh id.
    pub fn add(&mut self, item: Item, origin: ItemOrigin, stored_at: Timestamp) -> StoredItemId {
        let id = self.next_id;
        self.next_id = id.successor();
        self.items.push(StoredItem {
            id,
            origin,
            stored_at,
            item,
        });
        id
    }

    /// Adds what `other` holds and this store lacks, under fresh ids
    /// of this store's own; `other`'s ids are not carried over, since
    /// two stores' allocators know nothing of each other, and nothing
    /// here is ever removed. A seed-identified entry is already held
    /// when this store has one recording the same vaulting event — the
    /// same origin, moment, and item — so merging a copy of this
    /// store, or the same export twice, adds nothing. A stack
    /// (`is_stack`) is a count per record, not an event: the store's
    /// count is raised to `other`'s when that is higher, in the stack
    /// that already holds the record or a new one, and left alone
    /// otherwise — a high-water mark, so a repeated merge never doubles
    /// a stack. Learned blueprints are a union: every record `other`
    /// knows and this store does not is recorded under `other`'s
    /// campaign and moment.
    pub fn merge(&mut self, other: &VaultStore, is_stack: impl Fn(&Item) -> bool) -> Merged {
        let mut merged = Merged::default();
        for learned in &other.blueprints {
            if self.learn_blueprint(
                learned.record.clone(),
                learned.campaign.clone(),
                learned.learned_at,
            ) == Learned::Recorded
            {
                merged.blueprints += 1;
            }
        }
        let mut held: HashSet<Fact<'_>> = self
            .items
            .iter()
            .filter(|stored| !is_stack(&stored.item))
            .map(StoredItem::fact)
            .collect();
        let mut fresh: Vec<StoredItem> = Vec::new();
        let mut stacks: Vec<(StackKey, &StoredItem, u32)> = Vec::new();
        for stored in &other.items {
            if is_stack(&stored.item) {
                let key = StackKey::of(&stored.item);
                match stacks.iter_mut().find(|(known, _, _)| *known == key) {
                    Some((_, _, count)) => *count = count.saturating_add(stored.item.units()),
                    None => stacks.push((key, stored, stored.item.units())),
                }
            } else if held.insert(stored.fact()) {
                fresh.push(stored.clone());
            } else {
                merged.already_held += 1;
            }
        }
        merged.added = fresh.len();
        for stored in fresh {
            self.add(stored.item, stored.origin, stored.stored_at);
        }
        for (key, first, theirs) in stacks {
            let mine: u32 = self
                .items
                .iter()
                .filter(|stored| is_stack(&stored.item) && StackKey::of(&stored.item) == key)
                .map(|stored| stored.item.units())
                .fold(0, u32::saturating_add);
            let holder = self
                .items
                .iter()
                .position(|stored| is_stack(&stored.item) && StackKey::of(&stored.item) == key);
            match holder {
                Some(_) if theirs <= mine => merged.already_held += 1,
                Some(slot) => {
                    self.items[slot].item.stack_count =
                        self.items[slot].item.units().saturating_add(theirs - mine);
                    merged.raised += 1;
                }
                None => {
                    self.add(
                        Item {
                            stack_count: theirs,
                            ..first.item.clone()
                        },
                        first.origin.clone(),
                        first.stored_at,
                    );
                    merged.added += 1;
                }
            }
        }
        merged
    }

    /// Keeps one entry per stackable record (`is_stack`): every later
    /// entry of the same [`StackKey`] is folded into the first —
    /// its units added, its id retired — so a record the store holds
    /// in several stacks ends in one. The first entry keeps its id,
    /// origin, and moment; a record `is_stack` does not know is left
    /// alone.
    pub fn consolidate_stacks(&mut self, is_stack: impl Fn(&Item) -> bool) -> Folded {
        let mut first_of: Vec<(StackKey, usize)> = Vec::new();
        let mut folds: Vec<(usize, StoredItemId, u32)> = Vec::new();
        for (slot, stored) in self.items.iter().enumerate() {
            if !is_stack(&stored.item) {
                continue;
            }
            let key = StackKey::of(&stored.item);
            match first_of.iter().find(|(known, _)| *known == key) {
                Some((_, into)) => folds.push((*into, stored.id, stored.item.units())),
                None => first_of.push((key, slot)),
            }
        }
        let mut stacks: HashSet<usize> = HashSet::new();
        for (into, _, units) in &folds {
            let item = &mut self.items[*into].item;
            item.stack_count = item.units().saturating_add(*units);
            stacks.insert(*into);
        }
        let folded: HashSet<StoredItemId> = folds.iter().map(|(_, id, _)| *id).collect();
        self.items.retain(|stored| !folded.contains(&stored.id));
        Folded {
            entries: folds.len(),
            stacks: stacks.len(),
        }
    }

    /// Removes and returns an entry; its id is retired, never reissued.
    pub fn take(&mut self, id: StoredItemId) -> Option<StoredItem> {
        let position = self.items.iter().position(|stored| stored.id == id)?;
        Some(self.items.remove(position))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Just enough of a document to decide whether to read the rest.
#[derive(Deserialize)]
struct Envelope {
    format: String,
    version: u32,
}

/// The document as written: borrowed on the way out, owned on the way
/// in. `extra` catches every top-level field the envelope does not name.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoreFile<'a> {
    format: Cow<'a, str>,
    version: u32,
    next_id: StoredItemId,
    items: Cow<'a, [StoredItem]>,
    #[serde(default, skip_serializing_if = "<[LearnedBlueprint]>::is_empty")]
    blueprints: Cow<'a, [LearnedBlueprint]>,
    #[serde(flatten)]
    extra: Cow<'a, Map<String, Value>>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn cluster() -> Item {
        Item {
            base_name: "records/items/materia/a01_aethercluster.dbr".into(),
            seed: 12345,
            stack_count: 15,
            ..Item::default()
        }
    }

    fn at(seconds: u64) -> Timestamp {
        Timestamp::from_unix_seconds(seconds)
    }

    fn document(next_id: u64, ids: &[u64]) -> Vec<u8> {
        let items: Vec<Value> = ids
            .iter()
            .map(|id| {
                json!({
                    "id": id,
                    "origin": { "kind": "unknown" },
                    "storedAt": 1,
                    "item": serde_json::to_value(cluster()).unwrap(),
                })
            })
            .collect();
        serde_json::to_vec(&json!({
            "format": FORMAT_TAG,
            "version": FORMAT_VERSION,
            "nextId": next_id,
            "items": items,
        }))
        .unwrap()
    }

    #[test]
    fn round_trips_through_json_with_the_documented_shape() {
        let mut store = VaultStore::new();
        let id = store.add(
            cluster(),
            ItemOrigin::TransferStash {
                campaign: Campaign::Main,
                tab: TabIndex::new(0),
            },
            at(1_756_900_000),
        );
        assert_eq!(id, StoredItemId::new(1));

        let bytes = store.to_json();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.ends_with('\n'));
        let value: Value = serde_json::from_str(text).unwrap();
        assert_eq!(value["format"], json!(FORMAT_TAG));
        assert_eq!(value["version"], json!(FORMAT_VERSION));
        assert_eq!(value["nextId"], json!(2));
        assert_eq!(value["items"][0]["id"], json!(1));
        assert_eq!(
            value["items"][0]["origin"],
            json!({ "kind": "transferStash", "campaign": "main", "tab": 0 })
        );
        assert_eq!(value["items"][0]["storedAt"], json!(1_756_900_000));
        assert_eq!(
            value["items"][0]["item"]["baseName"],
            json!("records/items/materia/a01_aethercluster.dbr")
        );
        assert_eq!(value["items"][0]["item"]["stackCount"], json!(15));
        assert_eq!(value["items"][0]["item"]["relicCompletionLevel"], json!(0));

        let reloaded = VaultStore::from_json(&bytes).unwrap();
        assert_eq!(reloaded, store);
        assert_eq!(reloaded.get(id).unwrap().item(), &cluster());
    }

    #[test]
    fn envelope_keys_come_first_in_a_fixed_order() {
        let text = String::from_utf8(VaultStore::new().to_json()).unwrap();
        let format = text.find("\"format\"").unwrap();
        let version = text.find("\"version\"").unwrap();
        let next_id = text.find("\"nextId\"").unwrap();
        let items = text.find("\"items\"").unwrap();
        assert!(format < version && version < next_id && next_id < items);
    }

    #[test]
    fn wrong_format_tag_is_refused_before_the_shape_is_read() {
        let bytes = br#"{"format":"univault-store","version":1,"sacks":[]}"#;
        let error = VaultStore::from_json(bytes).unwrap_err();
        assert!(matches!(error, StoreError::WrongFormat { found } if found == "univault-store"));
    }

    #[test]
    fn newer_version_is_refused_and_older_accepted() {
        let newer = br#"{"format":"grimvault-store","version":2,"nextId":1,"items":[]}"#;
        assert!(matches!(
            VaultStore::from_json(newer).unwrap_err(),
            StoreError::UnsupportedVersion { version: 2 }
        ));
        let older = br#"{"format":"grimvault-store","version":0,"nextId":1,"items":[]}"#;
        assert!(VaultStore::from_json(older).unwrap().is_empty());
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let error = VaultStore::from_json(&document(10, &[3, 7, 3])).unwrap_err();
        assert!(matches!(error, StoreError::DuplicateId(id) if id == StoredItemId::new(3)));
    }

    #[test]
    fn lagging_next_id_is_healed_past_the_highest_item() {
        let mut store = VaultStore::from_json(&document(2, &[5, 9])).unwrap();
        assert_eq!(
            store.add(cluster(), ItemOrigin::Unknown, at(0)),
            StoredItemId::new(10)
        );

        let mut ahead = VaultStore::from_json(&document(50, &[5, 9])).unwrap();
        assert_eq!(
            ahead.add(cluster(), ItemOrigin::Unknown, at(0)),
            StoredItemId::new(50)
        );
    }

    #[test]
    fn ids_are_never_reused_after_take() {
        let mut store = VaultStore::new();
        let first = store.add(cluster(), ItemOrigin::Unknown, at(0));
        assert!(store.take(first).is_some());
        assert!(store.take(first).is_none());
        assert!(store.is_empty());
        let second = store.add(cluster(), ItemOrigin::Unknown, at(0));
        assert_eq!(second, StoredItemId::new(2));

        let reloaded = VaultStore::from_json(&store.to_json()).unwrap();
        assert_eq!(reloaded, store);
    }

    #[test]
    fn merging_adds_only_the_vaulting_events_the_store_lacks_under_fresh_ids() {
        let tab = |tab: u32| ItemOrigin::TransferStash {
            campaign: Campaign::Main,
            tab: TabIndex::new(tab),
        };
        let mut mine = VaultStore::new();
        mine.add(cluster(), tab(0), at(10));
        let retired = mine.add(cluster(), tab(1), at(11));
        mine.take(retired);

        let mut theirs = VaultStore::new();
        theirs.add(cluster(), tab(0), at(10));
        theirs.add(cluster(), tab(0), at(12));
        theirs.add(
            Item {
                stack_count: 3,
                ..cluster()
            },
            tab(0),
            at(10),
        );
        theirs.add(cluster(), tab(0), at(12));

        let merged = mine.merge(&theirs, |_| false);
        assert_eq!(
            merged,
            Merged {
                added: 2,
                already_held: 2,
                raised: 0,
                blueprints: 0
            }
        );
        assert_eq!(
            merged.to_string(),
            "2 added, 2 already held, 0 stacks raised, 0 blueprints recorded"
        );
        let ids: Vec<StoredItemId> = mine.items().iter().map(StoredItem::id).collect();
        assert_eq!(
            ids,
            vec![
                StoredItemId::new(1),
                StoredItemId::new(3),
                StoredItemId::new(4)
            ]
        );
        assert_eq!(mine.get(StoredItemId::new(3)).unwrap().stored_at(), at(12));
        assert_eq!(
            mine.get(StoredItemId::new(4)).unwrap().item().stack_count,
            3
        );

        assert_eq!(
            mine.merge(&theirs, |_| false),
            Merged {
                added: 0,
                already_held: 4,
                raised: 0,
                blueprints: 0
            }
        );
        let copy = VaultStore::from_json(&mine.to_json()).unwrap();
        assert_eq!(
            mine.merge(&copy, |_| false),
            Merged {
                added: 0,
                already_held: 3,
                raised: 0,
                blueprints: 0
            }
        );
        assert_eq!(mine.len(), 3);
        assert_eq!(
            VaultStore::new().merge(&VaultStore::new(), |_| false),
            Merged::default()
        );
    }

    const SHARD: &str = "records/items/materia/compa_soulshard.dbr";

    fn stack(base_name: &str, count: u32) -> Item {
        Item {
            base_name: base_name.into(),
            stack_count: count,
            ..Item::default()
        }
    }

    fn is_stack(item: &Item) -> bool {
        item.base_name != "records/items/gearweapons/swords/a.dbr"
    }

    #[test]
    fn stacks_merge_as_a_high_water_mark_per_record_and_never_double() {
        let mut mine = VaultStore::new();
        let shards = mine.add(stack(SHARD, 5), ItemOrigin::Unknown, at(1));
        let mut theirs = VaultStore::new();
        theirs.add(stack(SHARD, 4), ItemOrigin::Unknown, at(7));
        theirs.add(
            Item {
                seed: 99,
                ..stack(SHARD, 3)
            },
            ItemOrigin::Unknown,
            at(8),
        );
        theirs.add(cluster(), ItemOrigin::Unknown, at(9));

        let merged = mine.merge(&theirs, is_stack);
        assert_eq!(
            merged,
            Merged {
                added: 1,
                already_held: 0,
                raised: 1,
                blueprints: 0
            }
        );
        assert_eq!(mine.get(shards).unwrap().item().stack_count, 7);
        assert_eq!(mine.len(), 2);
        assert_eq!(mine.items()[1].item(), &cluster());

        assert_eq!(
            mine.merge(&theirs, is_stack),
            Merged {
                added: 0,
                already_held: 2,
                raised: 0,
                blueprints: 0
            }
        );
        assert_eq!(mine.get(shards).unwrap().item().stack_count, 7);

        let mut sword = VaultStore::new();
        sword.add(
            Item {
                base_name: "records/items/gearweapons/swords/a.dbr".into(),
                seed: 5,
                ..Item::default()
            },
            ItemOrigin::Unknown,
            at(2),
        );
        assert_eq!(
            mine.merge(&sword, is_stack),
            Merged {
                added: 1,
                already_held: 0,
                raised: 0,
                blueprints: 0
            }
        );
        assert_eq!(
            mine.merge(&sword, is_stack),
            Merged {
                added: 0,
                already_held: 1,
                raised: 0,
                blueprints: 0
            }
        );
    }

    #[test]
    fn consolidating_folds_later_stacks_into_the_first_and_keeps_the_rest() {
        let mut store = VaultStore::new();
        let first = store.add(stack(SHARD, 5), ItemOrigin::Unknown, at(1));
        let sword = store.add(
            Item {
                base_name: "records/items/gearweapons/swords/a.dbr".into(),
                seed: 5,
                ..Item::default()
            },
            ItemOrigin::Unknown,
            at(2),
        );
        store.add(
            Item {
                seed: 42,
                ..stack(SHARD, 0)
            },
            ItemOrigin::TransferStash {
                campaign: Campaign::Main,
                tab: TabIndex::new(0),
            },
            at(3),
        );
        let clusters = store.add(cluster(), ItemOrigin::Unknown, at(4));
        store.add(stack(SHARD, 1000), ItemOrigin::Unknown, at(5));
        store.add(cluster(), ItemOrigin::Unknown, at(6));
        store.add(
            Item {
                relic_completion_level: 1,
                ..stack(SHARD, 2)
            },
            ItemOrigin::Unknown,
            at(7),
        );

        let folded = store.consolidate_stacks(is_stack);
        assert_eq!(
            folded,
            Folded {
                entries: 3,
                stacks: 2
            }
        );
        assert_eq!(folded.to_string(), "3 entries folded into 2 stacks");
        let ids: Vec<StoredItemId> = store.items().iter().map(StoredItem::id).collect();
        assert_eq!(ids, vec![first, sword, clusters, StoredItemId::new(7)]);
        assert_eq!(store.get(first).unwrap().item().stack_count, 1006);
        assert_eq!(store.get(first).unwrap().stored_at(), at(1));
        assert_eq!(store.get(clusters).unwrap().item().stack_count, 30);
        assert_eq!(
            store.get(StoredItemId::new(7)).unwrap().item().stack_count,
            2
        );
        assert_eq!(store.consolidate_stacks(is_stack), Folded::default());
        assert_eq!(
            store.add(stack(SHARD, 1), ItemOrigin::Unknown, at(8)),
            StoredItemId::new(8)
        );
    }

    #[test]
    fn learned_blueprints_are_one_per_record_survive_the_round_trip_and_merge_as_a_union() {
        const BLUEPRINT: &str = "records/items/crafting/blueprints/craft_b.dbr";
        let mut store = VaultStore::new();
        assert!(!store.to_json().windows(10).any(|w| w == b"blueprints"));
        assert_eq!(
            store.learn_blueprint(BLUEPRINT.into(), Campaign::Main, at(5)),
            Learned::Recorded
        );
        let loot = Campaign::Mod(crate::campaign::ModName::parse("LootAscension").unwrap());
        assert_eq!(
            store.learn_blueprint(
                BLUEPRINT.to_uppercase().replace('/', "\\"),
                loot.clone(),
                at(9)
            ),
            Learned::AlreadyKnown
        );
        assert!(store.knows_blueprint(BLUEPRINT));
        assert!(!store.knows_blueprint("records/items/crafting/blueprints/other.dbr"));
        assert_eq!(store.blueprints().len(), 1);
        assert_eq!(store.blueprints()[0].campaign, Campaign::Main);
        assert_eq!(store.blueprints()[0].learned_at, at(5));

        let bytes = store.to_json();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["blueprints"],
            json!([{ "record": BLUEPRINT, "campaign": "main", "learnedAt": 5 }])
        );
        assert_eq!(VaultStore::from_json(&bytes).unwrap(), store);

        let twice = br#"{"format":"grimvault-store","version":1,"nextId":1,"items":[],"blueprints":[{"record":"records/a.dbr","campaign":"main","learnedAt":1},{"record":"RECORDS/A.DBR","campaign":"main","learnedAt":2}]}"#;
        assert_eq!(VaultStore::from_json(twice).unwrap().blueprints().len(), 1);

        let mut other = VaultStore::new();
        other.learn_blueprint(BLUEPRINT.into(), loot.clone(), at(1));
        other.learn_blueprint(
            "records/items/crafting/blueprints/craft_c.dbr".into(),
            loot,
            at(2),
        );
        let merged = store.merge(&other, |_| false);
        assert_eq!(merged.blueprints, 1);
        assert_eq!(store.blueprints().len(), 2);
        assert_eq!(store.merge(&other, |_| false).blueprints, 0);
        assert!(
            merged
                .to_string()
                .ends_with("0 stacks raised, 1 blueprints recorded")
        );
    }

    #[test]
    fn unknown_top_level_fields_survive_a_round_trip() {
        let bytes = br#"{"format":"grimvault-store","version":1,"nextId":1,"items":[],"note":"keep me","nested":{"a":[1,2]}}"#;
        let store = VaultStore::from_json(bytes).unwrap();
        let value: Value = serde_json::from_slice(&store.to_json()).unwrap();
        assert_eq!(value["note"], json!("keep me"));
        assert_eq!(value["nested"], json!({ "a": [1, 2] }));
        assert_eq!(value["items"], json!([]));
    }

    #[test]
    fn malformed_json_is_a_json_error() {
        assert!(matches!(
            VaultStore::from_json(b"{").unwrap_err(),
            StoreError::Json(_)
        ));
        assert!(matches!(
            VaultStore::from_json(br#"{"version":1}"#).unwrap_err(),
            StoreError::Json(_)
        ));
    }

    #[test]
    fn origins_are_tagged_by_kind() {
        let character = ItemOrigin::Character {
            realm: Realm::Main,
            name: "Sif".into(),
            sack: SackIndex::new(2),
        };
        assert_eq!(
            serde_json::to_value(&character).unwrap(),
            json!({ "kind": "character", "realm": "main", "name": "Sif", "sack": 2 })
        );
        let stash = ItemOrigin::CharacterStash {
            realm: Realm::Custom,
            name: "Sif".into(),
            tab: TabIndex::new(1),
        };
        assert_eq!(
            serde_json::to_value(&stash).unwrap(),
            json!({ "kind": "characterStash", "realm": "custom", "name": "Sif", "tab": 1 })
        );
        let equipped = ItemOrigin::Equipped {
            realm: Realm::Main,
            name: "Sif".into(),
            slot: EquipSlot::OffHand2,
        };
        let json =
            json!({ "kind": "equipped", "realm": "main", "name": "Sif", "slot": "offHand2" });
        assert_eq!(serde_json::to_value(&equipped).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<ItemOrigin>(json).unwrap(),
            equipped
        );
        assert_eq!(
            serde_json::to_value(ItemOrigin::Unknown).unwrap(),
            json!({ "kind": "unknown" })
        );
        assert_eq!(
            serde_json::to_value(ItemOrigin::ReagentStorage {
                campaign: Campaign::Main
            })
            .unwrap(),
            json!({ "kind": "reagentStorage", "campaign": "main" })
        );
        let loot = Campaign::Mod(crate::campaign::ModName::parse("LootAscension").unwrap());
        assert_eq!(
            serde_json::to_value(ItemOrigin::TransferStash {
                campaign: loot.clone(),
                tab: TabIndex::new(4)
            })
            .unwrap(),
            json!({ "kind": "transferStash", "campaign": "LootAscension", "tab": 4 })
        );
        let origin: ItemOrigin =
            serde_json::from_value(json!({ "kind": "reagentStorage" })).unwrap();
        assert_eq!(
            origin,
            ItemOrigin::ReagentStorage {
                campaign: Campaign::Main
            }
        );
        let bound = ItemOrigin::GdStashExport {
            file: "gd-stash-export.gds".into(),
            mode: GameMode::Softcore,
            owner: Some("Zark".into()),
        };
        let bound_json = json!({
            "kind": "gdStashExport",
            "file": "gd-stash-export.gds",
            "mode": "softcore",
            "owner": "Zark"
        });
        assert_eq!(serde_json::to_value(&bound).unwrap(), bound_json);
        assert_eq!(
            serde_json::from_value::<ItemOrigin>(bound_json).unwrap(),
            bound
        );
        let unbound = ItemOrigin::GdStashExport {
            file: "hc.gds".into(),
            mode: GameMode::Hardcore,
            owner: None,
        };
        let unbound_json =
            json!({ "kind": "gdStashExport", "file": "hc.gds", "mode": "hardcore", "owner": null });
        assert_eq!(serde_json::to_value(&unbound).unwrap(), unbound_json);
        assert_eq!(
            serde_json::from_value::<ItemOrigin>(unbound_json).unwrap(),
            unbound
        );
    }

    #[test]
    fn origins_describe_themselves() {
        assert_eq!(
            ItemOrigin::TransferStash {
                campaign: Campaign::Main,
                tab: TabIndex::new(2)
            }
            .to_string(),
            "main campaign transfer stash tab 2"
        );
        assert_eq!(
            ItemOrigin::Character {
                realm: Realm::Custom,
                name: "Zark".into(),
                sack: SackIndex::new(1)
            }
            .to_string(),
            "character user/Zark sack 1"
        );
        assert_eq!(
            ItemOrigin::Equipped {
                realm: Realm::Main,
                name: "Sif".into(),
                slot: EquipSlot::MainHand2,
            }
            .to_string(),
            "character main/Sif equipped Main hand (weapon set 2)"
        );
        assert_eq!(
            ItemOrigin::Equipped {
                realm: Realm::Main,
                name: "Sif".into(),
                slot: EquipSlot::Ring1,
            }
            .to_string(),
            "character main/Sif equipped Ring 1"
        );
        assert_eq!(
            ItemOrigin::GdStashExport {
                file: "gd-stash-export.gds".into(),
                mode: GameMode::Softcore,
                owner: Some("Zark".into()),
            }
            .to_string(),
            "GD Stash export gd-stash-export.gds (softcore, soulbound to Zark)"
        );
        assert_eq!(
            ItemOrigin::GdStashExport {
                file: "hc.gds".into(),
                mode: GameMode::Hardcore,
                owner: None,
            }
            .to_string(),
            "GD Stash export hc.gds (hardcore)"
        );
        assert_eq!(ItemOrigin::Unknown.to_string(), "unknown");
    }

    #[test]
    fn an_origin_without_a_realm_came_from_the_main_campaign() {
        let origin: ItemOrigin =
            serde_json::from_value(json!({ "kind": "character", "name": "Sif", "sack": 0 }))
                .unwrap();
        assert_eq!(
            origin,
            ItemOrigin::Character {
                realm: Realm::Main,
                name: "Sif".into(),
                sack: SackIndex::new(0),
            }
        );
        let origin: ItemOrigin = serde_json::from_value(
            json!({ "kind": "characterStash", "realm": "custom", "name": "Zark", "tab": 3 }),
        )
        .unwrap();
        assert_eq!(
            origin,
            ItemOrigin::CharacterStash {
                realm: Realm::Custom,
                name: "Zark".into(),
                tab: TabIndex::new(3),
            }
        );
    }
}
