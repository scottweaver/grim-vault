//! The Grim Dawn item record as serialized inside `player.gdc` and
//! `*.gst`, plus its two container positions.
//!
//! Ported from gdlc (MIT, dandels 2025), `src/inventory_item.rs`.
//!
//! Field order on the wire: `base_name, prefix_name, suffix_name,
//! modifier_name, transmute_name, seed, relic_name, relic_bonus,
//! relic_seed, augment_name, unknown, augment_seed,
//! [ascendant_record, ascendant_record_2h]₈, relic_completion_level,
//! stack_count, [seed_rerolls]₈, [affix_rerolls]₁₁` where the subscript is
//! the minimum [`ContainerVersion`] that carries the bracketed fields.
//!
//! Version policy: a field absent from the container's version reads as
//! its default; writing a non-default value into a version that lacks
//! the field is [`ItemEncodeError::FieldNotInVersion`], never silent
//! truncation. So an item read at any version re-encodes losslessly at
//! that version, and moving it to an older container is a visible
//! failure rather than a quiet loss.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::crypto::{DecodeError, Decoder, EncodeError, Encoder};

/// Version of an item-bearing block (inventory, per-character stash,
/// transfer stash). Selects the item and stash-tab layouts.
///
/// Accepts `MIN..=MAX`, the range gdlc and this crate have parsed
/// successfully; anything newer may carry fields this crate cannot
/// model and is refused so the containing block stays opaque.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContainerVersion(u32);

impl ContainerVersion {
    /// Oldest layout version this crate parses.
    pub const MIN: u32 = 4;
    /// Newest layout version this crate parses.
    pub const MAX: u32 = 11;

    /// Validates a raw block version.
    ///
    /// # Errors
    /// [`UnsupportedVersion`] outside `MIN..=MAX`.
    pub fn new(raw: u32) -> Result<Self, UnsupportedVersion> {
        if (Self::MIN..=Self::MAX).contains(&raw) {
            Ok(Self(raw))
        } else {
            Err(UnsupportedVersion { version: raw })
        }
    }

    /// The raw version as stored in the file.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Items carry `ascendant_record` / `ascendant_record_2h`.
    #[must_use]
    pub const fn has_ascendant_records(self) -> bool {
        self.0 >= 8
    }

    /// Items carry `seed_rerolls`.
    #[must_use]
    pub const fn has_seed_rerolls(self) -> bool {
        self.0 >= 8
    }

    /// Items carry `affix_rerolls`.
    #[must_use]
    pub const fn has_affix_rerolls(self) -> bool {
        self.0 >= 11
    }

    /// Stash tabs carry border / symbol / button-name decoration.
    #[must_use]
    pub const fn has_tab_decoration(self) -> bool {
        self.0 >= 9
    }
}

impl fmt::Display for ContainerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A block version outside the range this crate can lay out.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error(
    "container version {version} is outside the supported range {}..={}",
    ContainerVersion::MIN,
    ContainerVersion::MAX
)]
pub struct UnsupportedVersion {
    /// The raw version read from the file.
    pub version: u32,
}

/// Item fields whose presence depends on the container version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionedField {
    /// `ascendant_record`, present from v8.
    AscendantRecord,
    /// `ascendant_record_2h`, present from v8.
    AscendantRecord2h,
    /// `seed_rerolls`, present from v8.
    SeedRerolls,
    /// `affix_rerolls`, present from v11.
    AffixRerolls,
}

impl fmt::Display for VersionedField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AscendantRecord => "ascendant_record",
            Self::AscendantRecord2h => "ascendant_record_2h",
            Self::SeedRerolls => "seed_rerolls",
            Self::AffixRerolls => "affix_rerolls",
        })
    }
}

/// Why an item could not be written at the requested version.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ItemEncodeError {
    /// Cipher-level failure.
    #[error(transparent)]
    Cipher(#[from] EncodeError),
    /// A non-default field has no slot in the target version.
    #[error("item field {field} is set but container {version} does not carry it")]
    FieldNotInVersion {
        /// The field that would be lost.
        field: VersionedField,
        /// The version being written.
        version: ContainerVersion,
    },
}

/// One item as the game serializes it. Every field round-trips; see the
/// module docs for the version policy.
///
/// The JSON form (the vault store's) carries every field under the
/// game's own camelCase names, so a stored item is its full identity
/// and never depends on where it is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// Record path of the base item.
    pub base_name: String,
    /// Record path of the prefix affix, or empty.
    pub prefix_name: String,
    /// Record path of the suffix affix, or empty.
    pub suffix_name: String,
    /// Record path of the modifier, or empty.
    pub modifier_name: String,
    /// Record path of the transmute, or empty.
    pub transmute_name: String,
    /// Roll seed.
    pub seed: u32,
    /// Record path of the attached component / relic, or empty.
    pub relic_name: String,
    /// Record path of the relic completion bonus, or empty.
    pub relic_bonus: String,
    /// Relic roll seed.
    pub relic_seed: u32,
    /// Record path of the attached augment, or empty.
    pub augment_name: String,
    /// Unidentified word; observed as 0 (gdlc asserts it).
    pub unknown: u32,
    /// Augment roll seed.
    pub augment_seed: u32,
    /// Ascendant affix record (v8+), or empty.
    pub ascendant_record: String,
    /// Ascendant two-handed affix record (v8+), or empty.
    pub ascendant_record_2h: String,
    /// Component completion level (gdlc: `materia_combines`).
    pub relic_completion_level: u32,
    /// Stack size.
    pub stack_count: u32,
    /// Seed rerolls used (v8+).
    pub seed_rerolls: u32,
    /// Affix rerolls used (v11+).
    pub affix_rerolls: u32,
}

impl Item {
    /// Whether the slot holds anything (empty equipment slots serialize
    /// as an item with no base record).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base_name.is_empty()
    }

    /// Reads an item laid out for `version`.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let base_name = dec.read_string()?;
        let prefix_name = dec.read_string()?;
        let suffix_name = dec.read_string()?;
        let modifier_name = dec.read_string()?;
        let transmute_name = dec.read_string()?;
        let seed = dec.read_u32()?;
        let relic_name = dec.read_string()?;
        let relic_bonus = dec.read_string()?;
        let relic_seed = dec.read_u32()?;
        let augment_name = dec.read_string()?;
        let unknown = dec.read_u32()?;
        let augment_seed = dec.read_u32()?;
        let (ascendant_record, ascendant_record_2h) = if version.has_ascendant_records() {
            (dec.read_string()?, dec.read_string()?)
        } else {
            (String::new(), String::new())
        };
        let relic_completion_level = dec.read_u32()?;
        let stack_count = dec.read_u32()?;
        let seed_rerolls = if version.has_seed_rerolls() {
            dec.read_u32()?
        } else {
            0
        };
        let affix_rerolls = if version.has_affix_rerolls() {
            dec.read_u32()?
        } else {
            0
        };
        Ok(Self {
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
        })
    }

    /// Writes the item laid out for `version`.
    ///
    /// # Errors
    /// [`ItemEncodeError::FieldNotInVersion`] if a set field has no slot
    /// in `version`; cipher errors otherwise.
    pub fn write(
        &self,
        enc: &mut Encoder,
        version: ContainerVersion,
    ) -> Result<(), ItemEncodeError> {
        enc.write_string(&self.base_name)?;
        enc.write_string(&self.prefix_name)?;
        enc.write_string(&self.suffix_name)?;
        enc.write_string(&self.modifier_name)?;
        enc.write_string(&self.transmute_name)?;
        enc.write_u32(self.seed);
        enc.write_string(&self.relic_name)?;
        enc.write_string(&self.relic_bonus)?;
        enc.write_u32(self.relic_seed);
        enc.write_string(&self.augment_name)?;
        enc.write_u32(self.unknown);
        enc.write_u32(self.augment_seed);
        if version.has_ascendant_records() {
            enc.write_string(&self.ascendant_record)?;
            enc.write_string(&self.ascendant_record_2h)?;
        } else {
            require_absent(
                self.ascendant_record.is_empty(),
                VersionedField::AscendantRecord,
                version,
            )?;
            require_absent(
                self.ascendant_record_2h.is_empty(),
                VersionedField::AscendantRecord2h,
                version,
            )?;
        }
        enc.write_u32(self.relic_completion_level);
        enc.write_u32(self.stack_count);
        if version.has_seed_rerolls() {
            enc.write_u32(self.seed_rerolls);
        } else {
            require_absent(self.seed_rerolls == 0, VersionedField::SeedRerolls, version)?;
        }
        if version.has_affix_rerolls() {
            enc.write_u32(self.affix_rerolls);
        } else {
            require_absent(
                self.affix_rerolls == 0,
                VersionedField::AffixRerolls,
                version,
            )?;
        }
        Ok(())
    }
}

fn require_absent(
    is_default: bool,
    field: VersionedField,
    version: ContainerVersion,
) -> Result<(), ItemEncodeError> {
    if is_default {
        Ok(())
    } else {
        Err(ItemEncodeError::FieldNotInVersion { field, version })
    }
}

/// An item in an inventory sack: integer grid cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SackItem {
    /// The item.
    pub item: Item,
    /// Column of the top-left cell.
    pub x: u32,
    /// Row of the top-left cell.
    pub y: u32,
}

impl SackItem {
    /// Reads an item followed by its `u32` cell.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let item = Item::read(dec, version)?;
        let x = dec.read_u32()?;
        let y = dec.read_u32()?;
        Ok(Self { item, x, y })
    }

    /// Writes the item followed by its `u32` cell.
    ///
    /// # Errors
    /// See [`Item::write`].
    pub fn write(
        &self,
        enc: &mut Encoder,
        version: ContainerVersion,
    ) -> Result<(), ItemEncodeError> {
        self.item.write(enc, version)?;
        enc.write_u32(self.x);
        enc.write_u32(self.y);
        Ok(())
    }
}

/// An item in a stash tab: the game stores the cell as two `f32`s.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StashItem {
    /// The item.
    pub item: Item,
    /// Column of the top-left cell.
    pub x: f32,
    /// Row of the top-left cell.
    pub y: f32,
}

impl StashItem {
    /// Reads an item followed by its `f32` cell.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read(dec: &mut Decoder<'_>, version: ContainerVersion) -> Result<Self, DecodeError> {
        let item = Item::read(dec, version)?;
        let x = dec.read_f32()?;
        let y = dec.read_f32()?;
        Ok(Self { item, x, y })
    }

    /// Writes the item followed by its `f32` cell.
    ///
    /// # Errors
    /// See [`Item::write`].
    pub fn write(
        &self,
        enc: &mut Encoder,
        version: ContainerVersion,
    ) -> Result<(), ItemEncodeError> {
        self.item.write(enc, version)?;
        enc.write_f32(self.x);
        enc.write_f32(self.y);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_item() -> Item {
        Item {
            base_name: "records/items/gearweapons/swords/a.dbr".into(),
            prefix_name: "records/items/lootaffixes/prefix/p.dbr".into(),
            suffix_name: "records/items/lootaffixes/suffix/s.dbr".into(),
            modifier_name: "m".into(),
            transmute_name: "t".into(),
            seed: 0xDEAD_BEEF,
            relic_name: "records/items/materia/c.dbr".into(),
            relic_bonus: "records/items/lootaffixes/completion/b.dbr".into(),
            relic_seed: 7,
            augment_name: "records/items/enchants/e.dbr".into(),
            unknown: 0,
            augment_seed: 9,
            ascendant_record: "records/items/lootaffixes/ascendant/x.dbr".into(),
            ascendant_record_2h: String::new(),
            relic_completion_level: 3,
            stack_count: 1,
            seed_rerolls: 2,
            affix_rerolls: 4,
        }
    }

    fn base_item() -> Item {
        Item {
            ascendant_record: String::new(),
            seed_rerolls: 0,
            affix_rerolls: 0,
            ..full_item()
        }
    }

    fn round_trip(item: &Item, version: u32) -> Item {
        let version = ContainerVersion::new(version).unwrap();
        let mut enc = Encoder::new(0x1234_5678);
        item.write(&mut enc, version).unwrap();
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes).unwrap();
        let back = Item::read(&mut dec, version).unwrap();
        assert!(dec.is_at_end());
        back
    }

    #[test]
    fn round_trips_at_v4() {
        assert_eq!(round_trip(&base_item(), 4), base_item());
    }

    #[test]
    fn round_trips_at_v8_with_ascendant_and_seed_rerolls() {
        let item = Item {
            affix_rerolls: 0,
            ..full_item()
        };
        assert_eq!(round_trip(&item, 8), item);
    }

    #[test]
    fn round_trips_at_v11_with_every_field() {
        assert_eq!(round_trip(&full_item(), 11), full_item());
    }

    #[test]
    fn v4_refuses_fields_it_cannot_carry() {
        let version = ContainerVersion::new(4).unwrap();
        let mut enc = Encoder::new(1);
        assert_eq!(
            full_item().write(&mut enc, version),
            Err(ItemEncodeError::FieldNotInVersion {
                field: VersionedField::AscendantRecord,
                version
            })
        );
        let item = Item {
            ascendant_record: String::new(),
            ..full_item()
        };
        assert_eq!(
            item.write(&mut enc, version),
            Err(ItemEncodeError::FieldNotInVersion {
                field: VersionedField::SeedRerolls,
                version
            })
        );
    }

    #[test]
    fn v8_refuses_affix_rerolls() {
        let version = ContainerVersion::new(8).unwrap();
        let mut enc = Encoder::new(1);
        assert_eq!(
            full_item().write(&mut enc, version),
            Err(ItemEncodeError::FieldNotInVersion {
                field: VersionedField::AffixRerolls,
                version
            })
        );
    }

    #[test]
    fn version_range_is_enforced() {
        assert!(ContainerVersion::new(3).is_err());
        assert!(ContainerVersion::new(12).is_err());
        assert_eq!(ContainerVersion::new(11).unwrap().raw(), 11);
    }

    #[test]
    fn positions_round_trip_per_container_kind() {
        let version = ContainerVersion::new(11).unwrap();
        let sack = SackItem {
            item: full_item(),
            x: 3,
            y: 5,
        };
        let stash = StashItem {
            item: full_item(),
            x: 12.0,
            y: 0.0,
        };
        let mut enc = Encoder::new(42);
        sack.write(&mut enc, version).unwrap();
        stash.write(&mut enc, version).unwrap();
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes).unwrap();
        assert_eq!(SackItem::read(&mut dec, version).unwrap(), sack);
        assert_eq!(StashItem::read(&mut dec, version).unwrap(), stash);
        assert!(dec.is_at_end());
    }
}
