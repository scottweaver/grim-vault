//! GD Stash's export file, `.gds`: a **read-only import** boundary
//! (ARCHITECTURE.md "External boundaries"). This app never writes one.
//!
//! Layout (`docs/format-references.md`, "GD Stash (eyes-only)
//! findings"; verified on the user's own exports): plain little-endian,
//! unobfuscated — `u32 version`, `u32 count`, then `count` entries.
//! Strings are `u8 length + UTF-8 bytes`, length 0 meaning absent. An
//! entry is the game's own item record under GD Stash's names —
//! `itemID, prefixID, suffixID, modifierID, transmuteID, seed,
//! relicID, relicBonusID, relicSeed, enchantmentID, enchantmentLevel,
//! enchantmentSeed, [v2+: ascendantID, ascendant2hID], var1,
//! stackCount, [v2+: rerollsUsed], [v3+: affixRerollsUsed]` — followed
//! by two facts the save formats never carry: `hardcore` (one byte, 0
//! or 1) and `charname`, the soulbound owner (absent otherwise). The
//! item fields map one-to-one onto [`Item`] in wire order;
//! `enchantmentLevel` is the augment level, the word [`Item::unknown`]
//! holds, and `var1` is [`Item::relic_completion_level`].
//!
//! Version policy mirrors [`crate::item`]: a field a version lacks reads
//! as its default, so every entry yields a complete [`Item`] and
//! nothing the file holds is dropped.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use univault_engine::ids::RecordId;
use univault_engine::reader::{ByteReader, Offset, ReadError};

use crate::gamedata::GameData;
use crate::item::Item;
use crate::store::{ItemOrigin, StoredItem, StoredItemId, Timestamp, VaultStore};

/// The export's layout version. GD Stash v1.90a writes 3 and reads
/// 1 through 3; anything else is refused so an unknown layout is never
/// read as garbage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GdsVersion(u32);

impl GdsVersion {
    /// Oldest layout this crate parses.
    pub const MIN: u32 = 1;
    /// Newest layout this crate parses, the one GD Stash writes today.
    pub const MAX: u32 = 3;

    /// Validates the header's version word.
    ///
    /// # Errors
    /// [`GdsError::UnsupportedVersion`] outside `MIN..=MAX`.
    pub fn new(raw: u32) -> Result<Self, GdsError> {
        if (Self::MIN..=Self::MAX).contains(&raw) {
            Ok(Self(raw))
        } else {
            Err(GdsError::UnsupportedVersion { version: raw })
        }
    }

    /// The raw version word.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    const fn has_ascendant_records(self) -> bool {
        self.0 >= 2
    }

    const fn has_seed_rerolls(self) -> bool {
        self.0 >= 2
    }

    const fn has_affix_rerolls(self) -> bool {
        self.0 >= 3
    }
}

impl fmt::Display for GdsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Which of the game's two modes an exported item was played in. The
/// game keeps softcore and hardcore stashes in separate files; GD
/// Stash keeps the flag per item, and an import keeps it as part of
/// the item's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameMode {
    Softcore,
    Hardcore,
}

impl GameMode {
    /// The export's `hardcore` byte; GD Stash writes only 0 and 1, so
    /// any other value means the entry was not read where it starts.
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Softcore),
            1 => Some(Self::Hardcore),
            _ => None,
        }
    }
}

impl fmt::Display for GameMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Softcore => "softcore",
            Self::Hardcore => "hardcore",
        })
    }
}

/// One exported item: the game record, complete, plus the two facts
/// only GD Stash records. Equality over all three is the identity an
/// import deduplicates on (see [`import`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GdsEntry {
    pub item: Item,
    pub mode: GameMode,
    /// The character the item is soulbound to, `None` when unbound.
    pub owner: Option<String>,
}

impl GdsEntry {
    /// The entry a stored item came from, when it came from an export
    /// at all. Every other origin lacks a mode and an owner, so it can
    /// never be the same exported fact — an import never mistakes a
    /// vaulted stash item for one of its own.
    fn of_stored(stored: &StoredItem) -> Option<Self> {
        match stored.origin() {
            ItemOrigin::GdStashExport { mode, owner, .. } => Some(Self {
                item: stored.item().clone(),
                mode: *mode,
                owner: owner.clone(),
            }),
            ItemOrigin::TransferStash { .. }
            | ItemOrigin::Character { .. }
            | ItemOrigin::CharacterStash { .. }
            | ItemOrigin::Equipped { .. }
            | ItemOrigin::ReagentStorage { .. }
            | ItemOrigin::Unknown => None,
        }
    }
}

/// A parsed export: its version and every entry in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GdsExport {
    version: GdsVersion,
    entries: Vec<GdsEntry>,
}

impl GdsExport {
    #[must_use]
    pub fn version(&self) -> GdsVersion {
        self.version
    }

    #[must_use]
    pub fn entries(&self) -> &[GdsEntry] {
        &self.entries
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Why bytes are not a `.gds` export this app reads. Every variant
/// names where in the file the refusal happened.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GdsError {
    /// Shorter than the version and count words.
    #[error("{actual} bytes is too short for a .gds header (version and count)")]
    NoHeader { actual: usize },
    #[error(
        ".gds version {version} is outside the range this app reads ({}..={})",
        GdsVersion::MIN,
        GdsVersion::MAX
    )]
    UnsupportedVersion { version: u32 },
    /// The header promised more than the file holds, or a string's
    /// length byte reaches past the end.
    #[error("entry {entry} of {count} needs {wanted} more byte(s) at {at}")]
    Truncated {
        entry: usize,
        count: usize,
        at: Offset,
        wanted: usize,
    },
    #[error("entry {entry}: the string at {at} is not UTF-8")]
    NotUtf8 { entry: usize, at: Offset },
    #[error("entry {entry}: hardcore byte {value} at {at} is neither 0 nor 1")]
    BadHardcoreByte { entry: usize, at: Offset, value: u8 },
    /// An entry with no base record names no item at all.
    #[error("entry {entry} at {at} has no base item record")]
    NoBaseRecord { entry: usize, at: Offset },
    /// Bytes after the last entry the header counted.
    #[error("{trailing} byte(s) follow the last of {count} entries")]
    TrailingBytes { count: usize, trailing: usize },
}

/// Parses a whole export.
///
/// # Errors
/// [`GdsError`]; the file is refused as a whole — a partial read would
/// be a partial collection that looked complete.
pub fn parse(bytes: &[u8]) -> Result<GdsExport, GdsError> {
    let mut reader = ByteReader::new(bytes);
    let (version, count) = match (reader.read_u32(), reader.read_u32()) {
        (Ok(version), Ok(count)) => (GdsVersion::new(version)?, count),
        (Err(_), _) | (_, Err(_)) => {
            return Err(GdsError::NoHeader {
                actual: bytes.len(),
            });
        }
    };
    let count = usize::try_from(count).map_err(|_| GdsError::Truncated {
        entry: 0,
        count: usize::MAX,
        at: Offset(reader.pos()),
        wanted: usize::MAX,
    })?;
    let entries = (0..count)
        .map(|entry| read_entry(&mut reader, version).map_err(|error| error.at_entry(entry, count)))
        .collect::<Result<Vec<_>, _>>()?;
    let trailing = bytes.len() - reader.pos();
    if trailing > 0 {
        return Err(GdsError::TrailingBytes { count, trailing });
    }
    Ok(GdsExport { version, entries })
}

/// A refusal inside one entry, before the entry's position is known.
enum EntryError {
    Read(ReadError),
    NotUtf8 { at: Offset },
    BadHardcoreByte { at: Offset, value: u8 },
    NoBaseRecord { at: Offset },
}

impl EntryError {
    fn at_entry(self, entry: usize, count: usize) -> GdsError {
        match self {
            Self::Read(ReadError::UnexpectedEof { at, wanted }) => GdsError::Truncated {
                entry,
                count,
                at,
                wanted,
            },
            Self::Read(
                ReadError::NegativeLength { at, .. } | ReadError::KeyMismatch { at, .. },
            ) => {
                unreachable!("only unsigned, keyless reads are issued; {at} cannot fail otherwise")
            }
            Self::NotUtf8 { at } => GdsError::NotUtf8 { entry, at },
            Self::BadHardcoreByte { at, value } => GdsError::BadHardcoreByte { entry, at, value },
            Self::NoBaseRecord { at } => GdsError::NoBaseRecord { entry, at },
        }
    }
}

impl From<ReadError> for EntryError {
    fn from(error: ReadError) -> Self {
        Self::Read(error)
    }
}

fn read_entry(reader: &mut ByteReader<'_>, version: GdsVersion) -> Result<GdsEntry, EntryError> {
    let start = Offset(reader.pos());
    let base_name = read_string(reader)?;
    if base_name.is_empty() {
        return Err(EntryError::NoBaseRecord { at: start });
    }
    let prefix_name = read_string(reader)?;
    let suffix_name = read_string(reader)?;
    let modifier_name = read_string(reader)?;
    let transmute_name = read_string(reader)?;
    let seed = reader.read_u32()?;
    let relic_name = read_string(reader)?;
    let relic_bonus = read_string(reader)?;
    let relic_seed = reader.read_u32()?;
    let augment_name = read_string(reader)?;
    let unknown = reader.read_u32()?;
    let augment_seed = reader.read_u32()?;
    let (ascendant_record, ascendant_record_2h) = if version.has_ascendant_records() {
        (read_string(reader)?, read_string(reader)?)
    } else {
        (String::new(), String::new())
    };
    let relic_completion_level = reader.read_u32()?;
    let stack_count = reader.read_u32()?;
    let seed_rerolls = if version.has_seed_rerolls() {
        reader.read_u32()?
    } else {
        0
    };
    let affix_rerolls = if version.has_affix_rerolls() {
        reader.read_u32()?
    } else {
        0
    };
    let mode_at = Offset(reader.pos());
    let mode_byte = reader.read_u8()?;
    let mode = GameMode::from_byte(mode_byte).ok_or(EntryError::BadHardcoreByte {
        at: mode_at,
        value: mode_byte,
    })?;
    let owner = Some(read_string(reader)?).filter(|owner| !owner.is_empty());
    Ok(GdsEntry {
        item: Item {
            base_name,
            prefix_name,
            suffix_name,
            modifier_name,
            transmute_name,
            seed,
            relic_name,
            relic_bonus,
            relic_seed,
            augment_name,
            unknown,
            augment_seed,
            ascendant_record,
            ascendant_record_2h,
            relic_completion_level,
            stack_count,
            seed_rerolls,
            affix_rerolls,
        },
        mode,
        owner,
    })
}

/// `u8 length + UTF-8 bytes`; length 0 is the empty string, which
/// every string field of the game's item record uses for "absent".
fn read_string(reader: &mut ByteReader<'_>) -> Result<String, EntryError> {
    let length = usize::from(reader.read_u8()?);
    let at = Offset(reader.pos());
    let bytes = reader.read_bytes(length)?;
    String::from_utf8(bytes.to_vec()).map_err(|_| EntryError::NotUtf8 { at })
}

/// Whether the layered record database defines an item's base record;
/// the store holds an item either way, and the import report names
/// the records no layer knows.
pub trait KnownRecords {
    fn has_base_record(&self, item: &Item) -> bool;
}

impl KnownRecords for GameData {
    fn has_base_record(&self, item: &Item) -> bool {
        RecordId::parse(item.base_name.clone()).is_some_and(|id| self.record(&id).is_some())
    }
}

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// The ids allocated, in file order.
    pub added: Vec<StoredItemId>,
    /// Entries skipped because the store already held that exact
    /// exported fact.
    pub duplicates: usize,
    /// Base records no database layer defines, each with the number of
    /// entries added under it.
    pub unknown_records: BTreeMap<String, usize>,
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} added, {} duplicate(s) skipped, {} unknown record(s)",
            self.added.len(),
            self.duplicates,
            self.unknown_records.len()
        )
    }
}

/// Adds every entry of `export` the store does not already hold,
/// recording each under [`ItemOrigin::GdStashExport`] named by
/// `path`'s file name.
///
/// Duplicate rule: an entry is skipped when an entry already imported
/// — into this store earlier, or earlier in this same run — is equal
/// as a [`GdsEntry`]: every field of the game record (stack count
/// included), the mode, and the owner. This is GD Stash's own
/// non-stackable identity; its stackable path merges counts instead,
/// which would make importing a file twice double every stack, so it
/// is not adopted. Two exports of the same collection at different
/// times therefore add a changed stack as a second entry rather than
/// silently reconciling counts. Importing the same file twice adds
/// nothing the second time.
pub fn import(
    store: &mut VaultStore,
    export: &GdsExport,
    path: &Path,
    records: &impl KnownRecords,
    at: Timestamp,
) -> ImportReport {
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let mut held: HashSet<GdsEntry> = store
        .items()
        .iter()
        .filter_map(GdsEntry::of_stored)
        .collect();
    let mut report = ImportReport::default();
    for entry in export.entries() {
        if !held.insert(entry.clone()) {
            report.duplicates += 1;
            continue;
        }
        if !records.has_base_record(&entry.item) {
            *report
                .unknown_records
                .entry(entry.item.base_name.clone())
                .or_default() += 1;
        }
        let origin = ItemOrigin::GdStashExport {
            file: file.clone(),
            mode: entry.mode,
            owner: entry.owner.clone(),
        };
        report.added.push(store.add(entry.item.clone(), origin, at));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUGMENT: &str = "records/items/enchants/a07a_enchant.dbr";
    const SWORD: &str = "records/items/gearweapons/swords/a.dbr";

    /// Everything a v3 entry can say, every field non-default, so a
    /// round trip proves nothing is dropped.
    fn full_entry() -> GdsEntry {
        GdsEntry {
            item: Item {
                base_name: SWORD.into(),
                prefix_name: "records/items/lootaffixes/prefix/p.dbr".into(),
                suffix_name: "records/items/lootaffixes/suffix/s.dbr".into(),
                modifier_name: "m".into(),
                transmute_name: "t".into(),
                seed: 0xDEAD_BEEF,
                relic_name: "records/items/materia/c.dbr".into(),
                relic_bonus: "records/items/lootaffixes/completion/b.dbr".into(),
                relic_seed: 7,
                augment_name: AUGMENT.into(),
                unknown: 3,
                augment_seed: 9,
                ascendant_record: "records/items/lootaffixes/ascendant/x.dbr".into(),
                ascendant_record_2h: "records/items/lootaffixes/ascendant/y.dbr".into(),
                relic_completion_level: 4,
                stack_count: 1,
                seed_rerolls: 2,
                affix_rerolls: 5,
            },
            mode: GameMode::Hardcore,
            owner: Some("Zark".into()),
        }
    }

    fn stack(base: &str, seed: u32, count: u32, owner: Option<&str>) -> GdsEntry {
        GdsEntry {
            item: Item {
                base_name: base.into(),
                seed,
                stack_count: count,
                ..Item::default()
            },
            mode: GameMode::Softcore,
            owner: owner.map(str::to_owned),
        }
    }

    fn push_string(out: &mut Vec<u8>, text: &str) {
        out.push(u8::try_from(text.len()).unwrap());
        out.extend_from_slice(text.as_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, word: u32) {
        out.extend_from_slice(&word.to_le_bytes());
    }

    /// Lays an entry out at `version`, the inverse of `read_entry`.
    fn encode_entry(out: &mut Vec<u8>, entry: &GdsEntry, version: u32) {
        let item = &entry.item;
        push_string(out, &item.base_name);
        push_string(out, &item.prefix_name);
        push_string(out, &item.suffix_name);
        push_string(out, &item.modifier_name);
        push_string(out, &item.transmute_name);
        push_u32(out, item.seed);
        push_string(out, &item.relic_name);
        push_string(out, &item.relic_bonus);
        push_u32(out, item.relic_seed);
        push_string(out, &item.augment_name);
        push_u32(out, item.unknown);
        push_u32(out, item.augment_seed);
        if version >= 2 {
            push_string(out, &item.ascendant_record);
            push_string(out, &item.ascendant_record_2h);
        }
        push_u32(out, item.relic_completion_level);
        push_u32(out, item.stack_count);
        if version >= 2 {
            push_u32(out, item.seed_rerolls);
        }
        if version >= 3 {
            push_u32(out, item.affix_rerolls);
        }
        out.push(match entry.mode {
            GameMode::Softcore => 0,
            GameMode::Hardcore => 1,
        });
        push_string(out, entry.owner.as_deref().unwrap_or(""));
    }

    fn encode(version: u32, entries: &[GdsEntry]) -> Vec<u8> {
        let mut out = Vec::new();
        push_u32(&mut out, version);
        push_u32(&mut out, u32::try_from(entries.len()).unwrap());
        for entry in entries {
            encode_entry(&mut out, entry, version);
        }
        out
    }

    struct Knows(Vec<&'static str>);

    impl KnownRecords for Knows {
        fn has_base_record(&self, item: &Item) -> bool {
            self.0.contains(&item.base_name.as_str())
        }
    }

    const NOW: Timestamp = Timestamp::from_unix_seconds(1_757_000_000);
    const EXPORT: &str = "/exports/gd-stash-export.gds";

    #[test]
    fn a_v3_entry_round_trips_with_every_field_set() {
        let entries = [full_entry(), stack(AUGMENT, 0x1c4c_23f4, 40, None)];
        let export = parse(&encode(3, &entries)).unwrap();
        assert_eq!(export.version(), GdsVersion::new(3).unwrap());
        assert_eq!(export.entries(), &entries);
        assert_eq!(export.len(), 2);
    }

    #[test]
    fn older_versions_read_their_missing_fields_as_defaults() {
        let v1 = GdsEntry {
            item: Item {
                ascendant_record: String::new(),
                ascendant_record_2h: String::new(),
                seed_rerolls: 0,
                affix_rerolls: 0,
                ..full_entry().item
            },
            ..full_entry()
        };
        assert_eq!(
            parse(&encode(1, std::slice::from_ref(&v1)))
                .unwrap()
                .entries(),
            &[v1]
        );
        let v2 = GdsEntry {
            item: Item {
                affix_rerolls: 0,
                ..full_entry().item
            },
            ..full_entry()
        };
        assert_eq!(
            parse(&encode(2, std::slice::from_ref(&v2)))
                .unwrap()
                .entries(),
            &[v2]
        );
    }

    #[test]
    fn an_empty_export_is_just_a_header() {
        let export = parse(&encode(3, &[])).unwrap();
        assert!(export.is_empty());
        assert_eq!(export.version().raw(), 3);
    }

    #[test]
    fn versions_outside_the_range_are_refused() {
        for version in [0, 4, u32::MAX] {
            assert_eq!(
                parse(&encode(version, &[])),
                Err(GdsError::UnsupportedVersion { version })
            );
        }
        assert_eq!(GdsVersion::new(3).unwrap().to_string(), "v3");
    }

    #[test]
    fn a_short_header_is_refused() {
        assert_eq!(parse(&[]), Err(GdsError::NoHeader { actual: 0 }));
        assert_eq!(
            parse(&3u32.to_le_bytes()[..]),
            Err(GdsError::NoHeader { actual: 4 })
        );
        assert_eq!(
            parse(&[3, 0, 0, 0, 1, 0, 0]),
            Err(GdsError::NoHeader { actual: 7 })
        );
    }

    #[test]
    fn a_count_past_the_data_is_truncation_at_that_entry() {
        let mut bytes = encode(3, &[stack(AUGMENT, 1, 1, None)]);
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes());
        let end = bytes.len();
        assert_eq!(
            parse(&bytes),
            Err(GdsError::Truncated {
                entry: 1,
                count: 2,
                at: Offset(end),
                wanted: 1,
            })
        );
    }

    #[test]
    fn a_string_length_past_the_data_is_truncation() {
        let mut bytes = encode(3, &[stack(AUGMENT, 1, 1, Some("Zark"))]);
        let owner_length_at = bytes.len() - "Zark".len() - 1;
        bytes[owner_length_at] = 200;
        assert_eq!(
            parse(&bytes),
            Err(GdsError::Truncated {
                entry: 0,
                count: 1,
                at: Offset(owner_length_at + 1),
                wanted: 200,
            })
        );
    }

    #[test]
    fn trailing_bytes_are_refused() {
        let mut bytes = encode(3, &[stack(AUGMENT, 1, 1, None)]);
        bytes.extend_from_slice(&[0, 0, 0]);
        assert_eq!(
            parse(&bytes),
            Err(GdsError::TrailingBytes {
                count: 1,
                trailing: 3
            })
        );
    }

    #[test]
    fn a_hardcore_byte_other_than_zero_or_one_is_refused() {
        let mut bytes = encode(3, &[stack(AUGMENT, 1, 1, None)]);
        let mode_at = bytes.len() - 2;
        bytes[mode_at] = 7;
        assert_eq!(
            parse(&bytes),
            Err(GdsError::BadHardcoreByte {
                entry: 0,
                at: Offset(mode_at),
                value: 7
            })
        );
    }

    #[test]
    fn a_string_that_is_not_utf8_is_refused() {
        let mut bytes = encode(3, &[stack(AUGMENT, 1, 1, Some("Zark"))]);
        let owner_at = bytes.len() - "Zark".len();
        bytes[owner_at] = 0xFF;
        assert_eq!(
            parse(&bytes),
            Err(GdsError::NotUtf8 {
                entry: 0,
                at: Offset(owner_at)
            })
        );
    }

    #[test]
    fn an_entry_without_a_base_record_is_refused() {
        let bytes = encode(3, &[stack(AUGMENT, 1, 1, None), stack("", 1, 1, None)]);
        let second_at = encode(3, &[stack(AUGMENT, 1, 1, None)]).len();
        assert_eq!(
            parse(&bytes),
            Err(GdsError::NoBaseRecord {
                entry: 1,
                at: Offset(second_at)
            })
        );
    }

    #[test]
    fn hardcore_and_owner_are_read_per_entry() {
        let entries = [
            stack(AUGMENT, 1, 1, None),
            GdsEntry {
                mode: GameMode::Hardcore,
                ..stack(AUGMENT, 1, 1, Some("Sif"))
            },
        ];
        let export = parse(&encode(3, &entries)).unwrap();
        assert_eq!(export.entries()[0].mode, GameMode::Softcore);
        assert_eq!(export.entries()[0].owner, None);
        assert_eq!(export.entries()[1].mode, GameMode::Hardcore);
        assert_eq!(export.entries()[1].owner.as_deref(), Some("Sif"));
        assert_eq!(GameMode::Hardcore.to_string(), "hardcore");
    }

    #[test]
    fn import_adds_every_entry_under_the_export_origin() {
        let export = parse(&encode(
            3,
            &[full_entry(), stack(AUGMENT, 0x1c4c_23f4, 40, Some("Zark"))],
        ))
        .unwrap();
        let mut store = VaultStore::new();
        let report = import(
            &mut store,
            &export,
            Path::new(EXPORT),
            &Knows(vec![SWORD, AUGMENT]),
            NOW,
        );
        assert_eq!(
            report,
            ImportReport {
                added: vec![StoredItemId::new(1), StoredItemId::new(2)],
                duplicates: 0,
                unknown_records: BTreeMap::new(),
            }
        );
        assert_eq!(
            report.to_string(),
            "2 added, 0 duplicate(s) skipped, 0 unknown record(s)"
        );
        let stored = store.get(StoredItemId::new(2)).unwrap();
        assert_eq!(stored.item(), &export.entries()[1].item);
        assert_eq!(stored.stored_at(), NOW);
        assert_eq!(
            stored.origin(),
            &ItemOrigin::GdStashExport {
                file: "gd-stash-export.gds".into(),
                mode: GameMode::Softcore,
                owner: Some("Zark".into()),
            }
        );
        assert_eq!(
            store.get(StoredItemId::new(1)).unwrap().origin(),
            &ItemOrigin::GdStashExport {
                file: "gd-stash-export.gds".into(),
                mode: GameMode::Hardcore,
                owner: Some("Zark".into()),
            }
        );
    }

    #[test]
    fn importing_the_same_file_twice_adds_nothing_the_second_time() {
        let export = parse(&encode(
            3,
            &[full_entry(), stack(AUGMENT, 0x1c4c_23f4, 40, Some("Zark"))],
        ))
        .unwrap();
        let mut store = VaultStore::new();
        let knows = Knows(vec![SWORD, AUGMENT]);
        import(&mut store, &export, Path::new(EXPORT), &knows, NOW);
        let again = import(
            &mut store,
            &export,
            Path::new("/elsewhere/renamed.gds"),
            &knows,
            NOW,
        );
        assert_eq!(again.added, Vec::new());
        assert_eq!(again.duplicates, 2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn identity_is_the_whole_record_with_mode_and_owner() {
        let base = stack(AUGMENT, 0x1c4c_23f4, 40, Some("Zark"));
        let other_count = stack(AUGMENT, 0x1c4c_23f4, 5, Some("Zark"));
        let other_owner = stack(AUGMENT, 0x1c4c_23f4, 40, None);
        let other_mode = GdsEntry {
            mode: GameMode::Hardcore,
            ..base.clone()
        };
        let export = parse(&encode(
            3,
            &[
                base.clone(),
                other_count,
                other_owner,
                other_mode,
                base.clone(),
            ],
        ))
        .unwrap();
        let mut store = VaultStore::new();
        let report = import(
            &mut store,
            &export,
            Path::new(EXPORT),
            &Knows(vec![AUGMENT]),
            NOW,
        );
        assert_eq!(report.added.len(), 4);
        assert_eq!(report.duplicates, 1);
    }

    #[test]
    fn items_vaulted_from_the_game_never_count_as_duplicates() {
        let entry = stack(AUGMENT, 0x1c4c_23f4, 40, None);
        let mut store = VaultStore::new();
        store.add(entry.item.clone(), ItemOrigin::Unknown, NOW);
        let export = parse(&encode(3, &[entry])).unwrap();
        let report = import(
            &mut store,
            &export,
            Path::new(EXPORT),
            &Knows(vec![AUGMENT]),
            NOW,
        );
        assert_eq!(report.added.len(), 1);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn unknown_records_are_imported_and_named() {
        let export = parse(&encode(
            3,
            &[
                stack("records/items/mod/x.dbr", 1, 1, None),
                stack("records/items/mod/x.dbr", 2, 1, None),
                stack(AUGMENT, 3, 1, None),
            ],
        ))
        .unwrap();
        let mut store = VaultStore::new();
        let report = import(
            &mut store,
            &export,
            Path::new(EXPORT),
            &Knows(vec![AUGMENT]),
            NOW,
        );
        assert_eq!(report.added.len(), 3);
        assert_eq!(
            report.unknown_records,
            BTreeMap::from([("records/items/mod/x.dbr".to_string(), 2)])
        );
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn a_path_without_a_file_name_is_recorded_whole() {
        let export = parse(&encode(3, &[stack(AUGMENT, 1, 1, None)])).unwrap();
        let mut store = VaultStore::new();
        import(
            &mut store,
            &export,
            Path::new(".."),
            &Knows(vec![AUGMENT]),
            NOW,
        );
        assert!(matches!(
            store.get(StoredItemId::new(1)).unwrap().origin(),
            ItemOrigin::GdStashExport { file, .. } if file == ".."
        ));
    }

    #[test]
    fn the_imported_store_round_trips_through_json() {
        let export = parse(&encode(
            3,
            &[full_entry(), stack(AUGMENT, 0x1c4c_23f4, 40, None)],
        ))
        .unwrap();
        let mut store = VaultStore::new();
        import(
            &mut store,
            &export,
            Path::new(EXPORT),
            &Knows(vec![SWORD, AUGMENT]),
            NOW,
        );
        let reloaded = VaultStore::from_json(&store.to_json()).unwrap();
        assert_eq!(reloaded, store);
        let again = import(
            &mut VaultStore::from_json(&store.to_json()).unwrap(),
            &export,
            Path::new(EXPORT),
            &Knows(vec![SWORD, AUGMENT]),
            NOW,
        );
        assert_eq!(again.duplicates, 2);
    }
}
