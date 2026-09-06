//! `*.gst` files: `transfer.gst` (the shared stash, block 18),
//! `reagents.gst` (the component and crafting-material storage, block
//! 20), and `transmutes.gst` (the unlocked illusions, block 19), all
//! through the same block walker.
//!
//! Block 18 ported from gdlc (MIT, dandels 2025), `src/stash.rs`.
//! Blocks 19 and 20 were established here from the user's own files
//! (2026-09-03, `docs/format-references.md`): both open like block 18
//! with a version word and a static zero marker; block 20 then carries
//! a word observed as 0 (modelled as block 18's mod name, which is
//! empty for the base game and encodes identically), the entry count,
//! and one nested id-0 block per entry holding the record path and the
//! count; block 19 carries the mod name, the expansion-status byte,
//! the slot count, and one nested id-0 block per equipment slot
//! holding the slot id, the record count, and the record paths.
//!
//! Layout: raw seed; one `u32` (observed 2 in `transfer.gst`, 1 in
//! `transmutes.gst` / `reagents.gst`); then framed blocks to end of
//! file. A modelled block at a version this crate does not lay out is
//! carried opaquely. `formulas.gst` is **not** obfuscated at all — it
//! is a plaintext `begin_block` / `end_block` key-value format — and
//! is refused here with [`GstError::PlaintextKeyValueFormat`];
//! [`crate::formulas`] reads it.

use std::fmt;

use thiserror::Error;
use univault_engine::ids::normalize;

use crate::block::{
    Dispatch, OpaqueBlock, OpaqueReason, SaveEncodeError, StashTab, length_word, read_block,
};
use crate::crypto::{BlockId, DecodeError, Decoder, Encoder};
use crate::item::ContainerVersion;

/// Whether an add to a record list — the blueprint list, the illusion
/// collection — changed it. Both are sets the game only grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    Added,
    AlreadyKnown,
}

const TRANSFER_STASH_MIN_VERSION: u32 = 5;
const PLAINTEXT_PREAMBLE: &[u8] = b"\x0b\x00\x00\x00begin_block";

/// Why a `.gst` could not be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GstError {
    /// Cipher / framing failure.
    #[error(transparent)]
    Decode(#[from] DecodeError),
    /// The file is the plaintext key-value format used by
    /// `formulas.gst`, which this walker does not read.
    #[error("plaintext begin_block/end_block key-value format, not a rolling-XOR save")]
    PlaintextKeyValueFormat,
}

/// Block 18: the shared stash.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferStash {
    /// Layout version.
    pub version: ContainerVersion,
    /// Mod name the stash belongs to; empty for the base game.
    pub mod_name: String,
    /// Expansion status byte following the mod name (observed 7, the
    /// same value as the `player.gdc` header's).
    pub expansion_status: u8,
    /// The tabs, in order.
    pub tabs: Vec<StashTab>,
}

impl TransferStash {
    fn read_body(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        dec.read_zero_marker()?;
        let mod_name = dec.read_string()?;
        let expansion_status = dec.read_u8()?;
        let count = dec.read_u32()?;
        let tabs = (0..count)
            .map(|_| StashTab::read(dec, version))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version,
            mod_name,
            expansion_status,
            tabs,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::TRANSFER_STASH, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_zero_marker();
            enc.write_string(&self.mod_name)?;
            enc.write_u8(self.expansion_status);
            enc.write_u32(length_word(self.tabs.len())?);
            self.tabs
                .iter()
                .try_for_each(|tab| tab.write(enc, self.version))
        })
    }
}

/// Version of block 20. Only [`Self::SUPPORTED`] has a sample; any
/// other version keeps the block opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReagentStorageVersion(u32);

impl ReagentStorageVersion {
    /// The version the current game writes.
    pub const SUPPORTED: u32 = 1;

    /// Validates a raw block version.
    ///
    /// # Errors
    /// [`UnsupportedBlockVersion`] for anything but [`Self::SUPPORTED`].
    pub fn new(raw: u32) -> Result<Self, UnsupportedBlockVersion> {
        if raw == Self::SUPPORTED {
            Ok(Self(raw))
        } else {
            Err(UnsupportedBlockVersion {
                block: BlockId::REAGENT_STORAGE,
                version: raw,
            })
        }
    }

    /// The raw version as stored in the file.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ReagentStorageVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// Version of block 19. Only [`Self::SUPPORTED`] has a sample; any
/// other version keeps the block opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IllusionsVersion(u32);

impl IllusionsVersion {
    /// The version the current game writes.
    pub const SUPPORTED: u32 = 2;

    /// Validates a raw block version.
    ///
    /// # Errors
    /// [`UnsupportedBlockVersion`] for anything but [`Self::SUPPORTED`].
    pub fn new(raw: u32) -> Result<Self, UnsupportedBlockVersion> {
        if raw == Self::SUPPORTED {
            Ok(Self(raw))
        } else {
            Err(UnsupportedBlockVersion {
                block: BlockId::ILLUSIONS,
                version: raw,
            })
        }
    }

    /// The raw version as stored in the file.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for IllusionsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A block version this crate has no layout for.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("{block} version {version} has no known layout")]
pub struct UnsupportedBlockVersion {
    /// The block.
    pub block: BlockId,
    /// The raw version read from the file.
    pub version: u32,
}

/// One kind of component or crafting material in storage: the record
/// path and how many are held. The storage keeps nothing else about
/// an item — no seed, no affixes — so a stack here is only its record
/// and count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReagentEntry {
    /// Record path of the item.
    pub record: String,
    /// How many are held.
    pub count: u32,
}

impl ReagentEntry {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        dec.read_block_start_expecting(BlockId::NESTED)?;
        let record = dec.read_string()?;
        let count = dec.read_u32()?;
        dec.read_block_end()?;
        Ok(Self { record, count })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::NESTED, |enc| {
            enc.write_string(&self.record)?;
            enc.write_u32(self.count);
            Ok(())
        })
    }
}

/// Block 20: the account-wide component and crafting-material storage
/// (`reagents.gst`), shown in-game as the Components and Crafting
/// Materials tabs. One entry per record, in file order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReagentStorage {
    /// Layout version.
    pub version: ReagentStorageVersion,
    /// Mod name the storage belongs to; empty for the base game. The
    /// sample holds a zero word here, which is exactly how an empty
    /// string encodes.
    pub mod_name: String,
    /// The entries, in order.
    pub entries: Vec<ReagentEntry>,
}

impl ReagentStorage {
    fn read_body(
        dec: &mut Decoder<'_>,
        version: ReagentStorageVersion,
    ) -> Result<Self, DecodeError> {
        dec.read_zero_marker()?;
        let mod_name = dec.read_string()?;
        let count = dec.read_u32()?;
        let entries = (0..count)
            .map(|_| ReagentEntry::read(dec))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version,
            mod_name,
            entries,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::REAGENT_STORAGE, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_zero_marker();
            enc.write_string(&self.mod_name)?;
            enc.write_u32(length_word(self.entries.len())?);
            self.entries.iter().try_for_each(|entry| entry.write(enc))
        })
    }

    /// The entries of one record, `None` when the storage holds none.
    #[must_use]
    pub fn position_of(&self, record: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.record == record)
    }

    /// Total items held across every entry.
    #[must_use]
    pub fn total_count(&self) -> u64 {
        self.entries
            .iter()
            .map(|entry| u64::from(entry.count))
            .sum()
    }
}

/// The illusions unlocked for one equipment slot: the game's slot id
/// (observed 1, 3, 4, 5, 7, 8, 9, 14, 15 — head, torso, legs, feet,
/// hands, off-hand, weapon, shoulders, medal in that order in the
/// sample) and the record paths of the unlocked appearances.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IllusionSlot {
    /// The game's equipment slot id.
    pub slot: u32,
    /// Record paths of the unlocked illusions, in file order.
    pub records: Vec<String>,
}

impl IllusionSlot {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        dec.read_block_start_expecting(BlockId::NESTED)?;
        let slot = dec.read_u32()?;
        let count = dec.read_u32()?;
        let records = (0..count)
            .map(|_| dec.read_string())
            .collect::<Result<Vec<_>, _>>()?;
        dec.read_block_end()?;
        Ok(Self { slot, records })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::NESTED, |enc| {
            enc.write_u32(self.slot);
            enc.write_u32(length_word(self.records.len())?);
            self.records
                .iter()
                .try_for_each(|record| enc.write_string(record))?;
            Ok(())
        })
    }
}

/// Block 19: the illusion collection (`transmutes.gst`). Writable
/// since 2026-09-06 for adds only — [`crate::illusion`] holds the
/// rule for which list a record joins; nothing removes an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Illusions {
    /// Layout version.
    pub version: IllusionsVersion,
    /// Mod name the collection belongs to; empty for the base game.
    pub mod_name: String,
    /// Expansion status byte following the mod name (observed 7, as
    /// in block 18).
    pub expansion_status: u8,
    /// One list per equipment slot, in file order.
    pub slots: Vec<IllusionSlot>,
}

impl Illusions {
    fn read_body(dec: &mut Decoder<'_>, version: IllusionsVersion) -> Result<Self, DecodeError> {
        dec.read_zero_marker()?;
        let mod_name = dec.read_string()?;
        let expansion_status = dec.read_u8()?;
        let count = dec.read_u32()?;
        let slots = (0..count)
            .map(|_| IllusionSlot::read(dec))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            version,
            mod_name,
            expansion_status,
            slots,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BlockId::ILLUSIONS, |enc| {
            enc.write_u32(self.version.raw());
            enc.write_zero_marker();
            enc.write_string(&self.mod_name)?;
            enc.write_u8(self.expansion_status);
            enc.write_u32(length_word(self.slots.len())?);
            self.slots.iter().try_for_each(|slot| slot.write(enc))
        })
    }

    /// Total unlocked illusions across every slot.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.slots.iter().map(|slot| slot.records.len()).sum()
    }

    /// The list stored under a slot id, when the file has one.
    #[must_use]
    pub fn slot(&self, id: u32) -> Option<&IllusionSlot> {
        self.slots.iter().find(|slot| slot.slot == id)
    }

    /// Whether any list holds `record`; record paths compare the way
    /// the game's database keys do (case-insensitive, either slash).
    #[must_use]
    pub fn contains(&self, record: &str) -> bool {
        let wanted = normalize(record);
        self.slots
            .iter()
            .flat_map(|slot| slot.records.iter())
            .any(|listed| normalize(listed) == wanted)
    }

    /// Appends `record` to the list under `slot` — created at the end
    /// of the file when absent — unless some list already holds it.
    pub fn add(&mut self, slot: u32, record: String) -> Added {
        if self.contains(&record) {
            return Added::AlreadyKnown;
        }
        let position = self
            .slots
            .iter()
            .position(|listed| listed.slot == slot)
            .unwrap_or_else(|| {
                self.slots.push(IllusionSlot {
                    slot,
                    records: Vec::new(),
                });
                self.slots.len() - 1
            });
        self.slots[position].records.push(record);
        Added::Added
    }
}

/// One top-level block of a `.gst`, in file order.
#[derive(Clone, Debug, PartialEq)]
pub enum GstBlock {
    /// Block 18 at a supported version.
    TransferStash(TransferStash),
    /// Block 19 at a supported version.
    Illusions(Illusions),
    /// Block 20 at a supported version.
    ReagentStorage(ReagentStorage),
    /// Anything else, preserved verbatim.
    Opaque(OpaqueBlock),
}

impl GstBlock {
    /// The block id.
    #[must_use]
    pub fn id(&self) -> BlockId {
        match self {
            Self::TransferStash(_) => BlockId::TRANSFER_STASH,
            Self::Illusions(_) => BlockId::ILLUSIONS,
            Self::ReagentStorage(_) => BlockId::REAGENT_STORAGE,
            Self::Opaque(block) => block.id(),
        }
    }

    fn read(dec: &mut Decoder<'_>) -> Result<Self, GstError> {
        read_block(
            dec,
            |dec, header| {
                Ok::<_, GstError>(match header.id {
                    BlockId::TRANSFER_STASH => match ContainerVersion::new(header.version) {
                        Ok(version) if version.raw() >= TRANSFER_STASH_MIN_VERSION => {
                            Dispatch::Typed(Self::TransferStash(TransferStash::read_body(
                                dec, version,
                            )?))
                        }
                        Ok(_) | Err(_) => unsupported(header.version),
                    },
                    BlockId::ILLUSIONS => match IllusionsVersion::new(header.version) {
                        Ok(version) => {
                            Dispatch::Typed(Self::Illusions(Illusions::read_body(dec, version)?))
                        }
                        Err(_) => unsupported(header.version),
                    },
                    BlockId::REAGENT_STORAGE => match ReagentStorageVersion::new(header.version) {
                        Ok(version) => Dispatch::Typed(Self::ReagentStorage(
                            ReagentStorage::read_body(dec, version)?,
                        )),
                        Err(_) => unsupported(header.version),
                    },
                    _ => Dispatch::Opaque(OpaqueReason::Unmodeled),
                })
            },
            Self::Opaque,
        )
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        match self {
            Self::TransferStash(stash) => stash.write(enc),
            Self::Illusions(illusions) => illusions.write(enc),
            Self::ReagentStorage(storage) => storage.write(enc),
            Self::Opaque(block) => block.write(enc),
        }
    }
}

fn unsupported<T>(version: u32) -> Dispatch<T> {
    Dispatch::Opaque(OpaqueReason::UnsupportedVersion { version })
}

/// A parsed `.gst`: leading word plus the ordered block sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct GstFile {
    seed: u32,
    file_version: u32,
    blocks: Vec<GstBlock>,
}

impl GstFile {
    /// Parses a whole file image.
    ///
    /// # Errors
    /// [`GstError::PlaintextKeyValueFormat`] for `formulas.gst`-style
    /// files, or any cipher / framing failure.
    pub fn parse(bytes: &[u8]) -> Result<Self, GstError> {
        if bytes.starts_with(PLAINTEXT_PREAMBLE) {
            return Err(GstError::PlaintextKeyValueFormat);
        }
        let mut dec = Decoder::new(bytes)?;
        let file_version = dec.read_u32()?;
        let mut blocks = Vec::new();
        while !dec.is_at_end() {
            blocks.push(GstBlock::read(&mut dec)?);
        }
        Ok(Self {
            seed: dec.seed(),
            file_version,
            blocks,
        })
    }

    /// Re-encodes the file; byte-identical to the input when unmodified.
    ///
    /// # Errors
    /// [`SaveEncodeError`], notably `OpaqueRekeyed` when a block before
    /// an opaque one changed.
    pub fn encode(&self) -> Result<Vec<u8>, SaveEncodeError> {
        let mut enc = Encoder::new(self.seed);
        enc.write_u32(self.file_version);
        self.blocks
            .iter()
            .try_for_each(|block| block.write(&mut enc))?;
        Ok(enc.finish())
    }

    /// The raw cipher seed the file was written with.
    #[must_use]
    pub fn seed(&self) -> u32 {
        self.seed
    }

    /// The word between the seed and the first block.
    #[must_use]
    pub fn file_version(&self) -> u32 {
        self.file_version
    }

    /// The blocks in file order.
    #[must_use]
    pub fn blocks(&self) -> &[GstBlock] {
        &self.blocks
    }

    /// Block 18, when typed.
    #[must_use]
    pub fn transfer_stash(&self) -> Option<&TransferStash> {
        self.blocks.iter().find_map(|block| match block {
            GstBlock::TransferStash(stash) => Some(stash),
            GstBlock::Illusions(_) | GstBlock::ReagentStorage(_) | GstBlock::Opaque(_) => None,
        })
    }

    /// Block 18 for editing, when typed. Edits are written by
    /// [`GstFile::encode`], which still refuses if an opaque block
    /// follows the edited one.
    #[must_use]
    pub fn transfer_stash_mut(&mut self) -> Option<&mut TransferStash> {
        self.blocks.iter_mut().find_map(|block| match block {
            GstBlock::TransferStash(stash) => Some(stash),
            GstBlock::Illusions(_) | GstBlock::ReagentStorage(_) | GstBlock::Opaque(_) => None,
        })
    }

    /// Block 20, when typed.
    #[must_use]
    pub fn reagent_storage(&self) -> Option<&ReagentStorage> {
        self.blocks.iter().find_map(|block| match block {
            GstBlock::ReagentStorage(storage) => Some(storage),
            GstBlock::TransferStash(_) | GstBlock::Illusions(_) | GstBlock::Opaque(_) => None,
        })
    }

    /// Block 20 for editing, when typed; see
    /// [`GstFile::transfer_stash_mut`] for the write rule.
    #[must_use]
    pub fn reagent_storage_mut(&mut self) -> Option<&mut ReagentStorage> {
        self.blocks.iter_mut().find_map(|block| match block {
            GstBlock::ReagentStorage(storage) => Some(storage),
            GstBlock::TransferStash(_) | GstBlock::Illusions(_) | GstBlock::Opaque(_) => None,
        })
    }

    /// Block 19, when typed.
    #[must_use]
    pub fn illusions(&self) -> Option<&Illusions> {
        self.blocks.iter().find_map(|block| match block {
            GstBlock::Illusions(illusions) => Some(illusions),
            GstBlock::TransferStash(_) | GstBlock::ReagentStorage(_) | GstBlock::Opaque(_) => None,
        })
    }

    /// Block 19 for editing, when typed; see
    /// [`GstFile::transfer_stash_mut`] for the write rule.
    #[must_use]
    pub fn illusions_mut(&mut self) -> Option<&mut Illusions> {
        self.blocks.iter_mut().find_map(|block| match block {
            GstBlock::Illusions(illusions) => Some(illusions),
            GstBlock::TransferStash(_) | GstBlock::ReagentStorage(_) | GstBlock::Opaque(_) => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::TabDecoration;
    use crate::crypto::EncodeError;
    use crate::item::{Item, StashItem};

    fn sample() -> GstFile {
        GstFile {
            seed: 0xC533_DF77,
            file_version: 2,
            blocks: vec![GstBlock::TransferStash(TransferStash {
                version: ContainerVersion::new(11).unwrap(),
                mod_name: String::new(),
                expansion_status: 7,
                tabs: vec![
                    StashTab {
                        width: 10,
                        height: 18,
                        items: vec![StashItem {
                            item: Item {
                                base_name: "records/items/gearrelic/r.dbr".into(),
                                seed: 99,
                                stack_count: 1,
                                ..Item::default()
                            },
                            x: 4.0,
                            y: 6.0,
                        }],
                        decoration: TabDecoration {
                            button_name: "Relics".into(),
                            ..TabDecoration::default()
                        },
                    },
                    StashTab {
                        width: 10,
                        height: 18,
                        items: vec![],
                        decoration: TabDecoration::default(),
                    },
                ],
            })],
        }
    }

    fn reagent_sample() -> GstFile {
        GstFile {
            seed: 0x77DF_33C5,
            file_version: 1,
            blocks: vec![GstBlock::ReagentStorage(ReagentStorage {
                version: ReagentStorageVersion::new(1).unwrap(),
                mod_name: String::new(),
                entries: vec![
                    ReagentEntry {
                        record: "records/items/crafting/materials/craft_aetherialmissive.dbr"
                            .into(),
                        count: 8,
                    },
                    ReagentEntry {
                        record: "records/items/materia/compa_moltenskin.dbr".into(),
                        count: 20,
                    },
                ],
            })],
        }
    }

    fn illusion_sample() -> GstFile {
        GstFile {
            seed: 0x77DF_33C5,
            file_version: 1,
            blocks: vec![GstBlock::Illusions(Illusions {
                version: IllusionsVersion::new(2).unwrap(),
                mod_name: String::new(),
                expansion_status: 7,
                slots: vec![
                    IllusionSlot {
                        slot: 1,
                        records: vec![
                            "records/items/gearhead/a10_head001.dbr".into(),
                            "records/items/gearhead/a01_head002.dbr".into(),
                        ],
                    },
                    IllusionSlot {
                        slot: 15,
                        records: vec![],
                    },
                ],
            })],
        }
    }

    #[test]
    fn transfer_stash_round_trips_through_bytes() {
        let file = sample();
        let bytes = file.encode().unwrap();
        let parsed = GstFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.encode().unwrap(), bytes);
        let stash = parsed.transfer_stash().unwrap();
        assert_eq!(stash.tabs.len(), 2);
        assert_eq!(stash.tabs[0].items.len(), 1);
        assert!(parsed.reagent_storage().is_none());
        assert!(parsed.illusions().is_none());
    }

    #[test]
    fn edits_through_transfer_stash_mut_are_encoded() {
        let mut file = sample();
        file.transfer_stash_mut().unwrap().tabs[1]
            .items
            .push(StashItem {
                item: Item {
                    base_name: "records/items/materia/m.dbr".into(),
                    stack_count: 3,
                    ..Item::default()
                },
                x: 0.0,
                y: 0.0,
            });
        let reparsed = GstFile::parse(&file.encode().unwrap()).unwrap();
        assert_eq!(reparsed, file);
        assert_eq!(reparsed.transfer_stash().unwrap().tabs[1].items.len(), 1);
    }

    #[test]
    fn reagent_storage_round_trips_through_bytes() {
        let file = reagent_sample();
        let bytes = file.encode().unwrap();
        let parsed = GstFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.encode().unwrap(), bytes);
        let storage = parsed.reagent_storage().unwrap();
        assert_eq!(storage.entries.len(), 2);
        assert_eq!(storage.total_count(), 28);
        assert_eq!(
            storage.position_of("records/items/materia/compa_moltenskin.dbr"),
            Some(1)
        );
        assert_eq!(storage.position_of("records/items/materia/other.dbr"), None);
        assert!(parsed.transfer_stash().is_none());
        assert_eq!(parsed.blocks()[0].id(), BlockId::REAGENT_STORAGE);
    }

    #[test]
    fn reagent_storage_matches_the_established_wire_shape() {
        let mut enc = Encoder::new(0x77DF_33C5);
        enc.write_u32(1);
        enc.write_block(BlockId::REAGENT_STORAGE, |enc| {
            enc.write_u32(1);
            enc.write_zero_marker();
            enc.write_u32(0);
            enc.write_u32(1);
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_string("records/items/crafting/materials/craft_aethershard.dbr")?;
                enc.write_u32(15);
                Ok::<(), EncodeError>(())
            })
        })
        .unwrap();
        let bytes = enc.finish();
        let file = GstFile::parse(&bytes).unwrap();
        let storage = file.reagent_storage().unwrap();
        assert_eq!(storage.mod_name, "");
        assert_eq!(
            storage.entries,
            vec![ReagentEntry {
                record: "records/items/crafting/materials/craft_aethershard.dbr".into(),
                count: 15
            }]
        );
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn edits_through_reagent_storage_mut_are_encoded() {
        let mut file = reagent_sample();
        let storage = file.reagent_storage_mut().unwrap();
        storage.entries[0].count = 7;
        storage.entries.push(ReagentEntry {
            record: "records/items/questitems/scrapmetal.dbr".into(),
            count: 80,
        });
        let reparsed = GstFile::parse(&file.encode().unwrap()).unwrap();
        assert_eq!(reparsed, file);
        assert_eq!(reparsed.reagent_storage().unwrap().entries.len(), 3);
    }

    #[test]
    fn illusions_round_trip_through_bytes() {
        let file = illusion_sample();
        let bytes = file.encode().unwrap();
        let parsed = GstFile::parse(&bytes).unwrap();
        assert_eq!(parsed, file);
        assert_eq!(parsed.encode().unwrap(), bytes);
        let illusions = parsed.illusions().unwrap();
        assert_eq!(illusions.slots.len(), 2);
        assert_eq!(illusions.total_count(), 2);
        assert_eq!(parsed.blocks()[0].id(), BlockId::ILLUSIONS);
    }

    #[test]
    fn edits_through_illusions_mut_are_encoded() {
        let mut file = illusion_sample();
        let illusions = file.illusions_mut().unwrap();
        assert!(illusions.contains("RECORDS\\ITEMS\\GEARHEAD\\A10_HEAD001.DBR"));
        assert_eq!(illusions.slot(15).unwrap().records.len(), 0);
        assert!(illusions.slot(9).is_none());
        assert_eq!(
            illusions.add(1, "records/items/gearhead/a10_head001.dbr".into()),
            Added::AlreadyKnown
        );
        assert_eq!(
            illusions.add(
                15,
                "records/items/gearaccessories/medals/a01_medal.dbr".into()
            ),
            Added::Added
        );
        assert_eq!(
            illusions.add(9, "records/items/gearweapons/swords/a01_sword.dbr".into()),
            Added::Added
        );
        assert_eq!(illusions.slots.len(), 3);
        assert_eq!(illusions.slots[2].slot, 9);
        assert_eq!(illusions.total_count(), 4);
        let reparsed = GstFile::parse(&file.encode().unwrap()).unwrap();
        assert_eq!(reparsed, file);
    }

    #[test]
    fn unsupported_versions_of_modelled_blocks_stay_opaque() {
        for (id, version) in [(BlockId::REAGENT_STORAGE, 2), (BlockId::ILLUSIONS, 3)] {
            let mut enc = Encoder::new(9);
            enc.write_u32(1);
            enc.write_block(id, |enc| {
                enc.write_u32(version);
                enc.write_zero_marker();
                enc.write_u32(0);
                enc.write_u32(0);
                Ok::<(), EncodeError>(())
            })
            .unwrap();
            let bytes = enc.finish();
            let file = GstFile::parse(&bytes).unwrap();
            assert!(file.reagent_storage().is_none());
            assert!(file.illusions().is_none());
            let GstBlock::Opaque(block) = &file.blocks()[0] else {
                panic!("{id} {version} should be opaque");
            };
            assert_eq!(block.reason(), OpaqueReason::UnsupportedVersion { version });
            assert_eq!(file.encode().unwrap(), bytes);
        }
        assert_eq!(
            ReagentStorageVersion::new(2),
            Err(UnsupportedBlockVersion {
                block: BlockId::REAGENT_STORAGE,
                version: 2
            })
        );
        assert_eq!(
            IllusionsVersion::new(1),
            Err(UnsupportedBlockVersion {
                block: BlockId::ILLUSIONS,
                version: 1
            })
        );
    }

    #[test]
    fn unknown_blocks_are_opaque_and_round_trip() {
        let mut enc = Encoder::new(1);
        enc.write_u32(1);
        enc.write_block(BlockId::new(21), |enc| {
            enc.write_u32(2);
            enc.write_block(BlockId::NESTED, |enc| {
                enc.write_string("records/items/crafting/x.dbr")?;
                enc.write_u32(0);
                Ok::<(), EncodeError>(())
            })?;
            Ok::<(), EncodeError>(())
        })
        .unwrap();
        let bytes = enc.finish();
        let file = GstFile::parse(&bytes).unwrap();
        assert!(file.transfer_stash().is_none());
        assert_eq!(file.blocks()[0].id(), BlockId::new(21));
        assert_eq!(file.encode().unwrap(), bytes);
    }

    #[test]
    fn plaintext_formulas_are_refused() {
        let mut bytes = PLAINTEXT_PREAMBLE.to_vec();
        bytes.extend_from_slice(&0xB01D_FACE_u32.to_le_bytes());
        assert_eq!(
            GstFile::parse(&bytes),
            Err(GstError::PlaintextKeyValueFormat)
        );
    }
}
