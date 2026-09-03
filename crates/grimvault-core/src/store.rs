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
//!   ]
//! }
//! ```

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ItemOrigin {
    /// A tab of `transfer.gst`.
    TransferStash { tab: TabIndex },
    /// A sack of a character's inventory (`player.gdc` block 3).
    Character { name: String, sack: SackIndex },
    /// A tab of a character's own stash (`player.gdc` block 4).
    CharacterStash { name: String, tab: TabIndex },
    /// The component / crafting-material storage, `reagents.gst`.
    ReagentStorage,
    /// Provenance not recorded.
    Unknown,
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
        Ok(Self {
            next_id,
            items,
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

    #[must_use]
    pub fn get(&self, id: StoredItemId) -> Option<&StoredItem> {
        self.items.iter().find(|stored| stored.id == id)
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
            json!({ "kind": "transferStash", "tab": 0 })
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
            name: "Sif".into(),
            sack: SackIndex::new(2),
        };
        assert_eq!(
            serde_json::to_value(&character).unwrap(),
            json!({ "kind": "character", "name": "Sif", "sack": 2 })
        );
        let stash = ItemOrigin::CharacterStash {
            name: "Sif".into(),
            tab: TabIndex::new(1),
        };
        assert_eq!(
            serde_json::to_value(&stash).unwrap(),
            json!({ "kind": "characterStash", "name": "Sif", "tab": 1 })
        );
        assert_eq!(
            serde_json::to_value(ItemOrigin::Unknown).unwrap(),
            json!({ "kind": "unknown" })
        );
        assert_eq!(
            serde_json::to_value(ItemOrigin::ReagentStorage).unwrap(),
            json!({ "kind": "reagentStorage" })
        );
    }
}
