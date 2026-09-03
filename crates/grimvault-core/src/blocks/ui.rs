//! Block 14: UI state — the hotbar (skill / item slot) assignments and
//! the camera distance, preceded by fields neither reference names.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/misc.rs` (`UI`, `UISkillSet`,
//! `UISlot`); grim-save-parser's `ui_settings.rs` / `hot_slot.rs` agree
//! on the layout (it supports v5–7, yagde v4–7) and supply the slot-type
//! meanings behind [`HotSlot`].
//!
//! Before v7 there is exactly one hotbar set of a version-fixed slot
//! count and no set id; v7 stores the set count, the slots per set, and
//! an id per set. [`Hotbars`] carries that as one variant per version
//! so a set of the wrong size cannot be expressed for v4–v6; for v7 the
//! declared slot count is checked at write time.

use super::read_array;
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, EncodeError, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(14);

/// Slots in the single v4 hotbar set.
pub const V4_SLOTS: usize = 36;
/// Slots in the single v5 hotbar set.
pub const V5_SLOTS: usize = 46;
/// Slots in the single v6 hotbar set.
pub const V6_SLOTS: usize = 47;
/// Number of leading unknown string / string / byte entries.
pub const UNKNOWN_ENTRIES: usize = 5;

const SLOT_SKILL: u32 = 0;
const SLOT_HEALTH_POTION: u32 = 2;
const SLOT_ENERGY_POTION: u32 = 3;
const SLOT_ITEM: u32 = 4;
const SLOT_EMPTY: u32 = u32::MAX;

/// Layout versions yagde supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiVersion {
    /// Version 4: one set of [`V4_SLOTS`].
    V4,
    /// Version 5: one set of [`V5_SLOTS`].
    V5,
    /// Version 6: one set of [`V6_SLOTS`].
    V6,
    /// Version 7: counted sets with ids.
    V7,
}

impl UiVersion {
    /// The version enum for a raw version word, `None` outside the set.
    #[must_use]
    pub const fn new(raw: u32) -> Option<Self> {
        match raw {
            4 => Some(Self::V4),
            5 => Some(Self::V5),
            6 => Some(Self::V6),
            7 => Some(Self::V7),
            _ => None,
        }
    }

    /// The raw version word.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::V4 => 4,
            Self::V5 => 5,
            Self::V6 => 6,
            Self::V7 => 7,
        }
    }
}

/// One of the five leading entries neither reference names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnknownEntry {
    /// Unidentified string (v4+).
    pub unknown_string_4: String,
    /// Unidentified string (v4+).
    pub unknown_string_5: String,
    /// Unidentified byte (v4+).
    pub unknown_u8_6: u8,
}

impl UnknownEntry {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            unknown_string_4: dec.read_string()?,
            unknown_string_5: dec.read_string()?,
            unknown_u8_6: dec.read_u8()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        enc.write_string(&self.unknown_string_4)?;
        enc.write_string(&self.unknown_string_5)?;
        enc.write_u8(self.unknown_u8_6);
        Ok(())
    }
}

/// A slot type word other than the ones [`HotSlot`] names; carried so
/// the slot re-encodes exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OtherSlotType(u32);

impl OtherSlotType {
    /// Wraps a raw slot type, `None` for a type [`HotSlot`] models.
    #[must_use]
    pub const fn new(raw: u32) -> Option<Self> {
        match raw {
            SLOT_SKILL | SLOT_HEALTH_POTION | SLOT_ENERGY_POTION | SLOT_ITEM | SLOT_EMPTY => None,
            other => Some(Self(other)),
        }
    }

    /// The raw slot type word.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// One hotbar slot. Only skill (0) and item (4) slots carry a payload;
/// the type meanings are grim-save-parser's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HotSlot {
    /// Type 0: a skill.
    Skill {
        /// Skill record path.
        skill: String,
        /// Byte both references call `is_item_skill`.
        is_item_skill: u8,
        /// Record path of the granting item, or empty.
        item: String,
        /// Equipment slot of the granting item.
        equip_location: u32,
    },
    /// Type 2.
    HealthPotion,
    /// Type 3.
    EnergyPotion,
    /// Type 4: an item.
    Item {
        /// Item record path.
        item: String,
        /// Up-state bitmap path.
        bitmap_up: String,
        /// Down-state bitmap path.
        bitmap_down: String,
        /// Label shown on the slot.
        label: String,
    },
    /// Type `0xFFFF_FFFF`.
    Empty,
    /// Any other type word, with no payload.
    Other(OtherSlotType),
}

impl HotSlot {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        let slot_type = dec.read_u32()?;
        Ok(match slot_type {
            SLOT_SKILL => Self::Skill {
                skill: dec.read_string()?,
                is_item_skill: dec.read_u8()?,
                item: dec.read_string()?,
                equip_location: dec.read_u32()?,
            },
            SLOT_HEALTH_POTION => Self::HealthPotion,
            SLOT_ENERGY_POTION => Self::EnergyPotion,
            SLOT_ITEM => Self::Item {
                item: dec.read_string()?,
                bitmap_up: dec.read_string()?,
                bitmap_down: dec.read_string()?,
                label: dec.read_wstring()?,
            },
            SLOT_EMPTY => Self::Empty,
            other => Self::Other(OtherSlotType(other)),
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        match self {
            Self::Skill {
                skill,
                is_item_skill,
                item,
                equip_location,
            } => {
                enc.write_u32(SLOT_SKILL);
                enc.write_string(skill)?;
                enc.write_u8(*is_item_skill);
                enc.write_string(item)?;
                enc.write_u32(*equip_location);
            }
            Self::HealthPotion => enc.write_u32(SLOT_HEALTH_POTION),
            Self::EnergyPotion => enc.write_u32(SLOT_ENERGY_POTION),
            Self::Item {
                item,
                bitmap_up,
                bitmap_down,
                label,
            } => {
                enc.write_u32(SLOT_ITEM);
                enc.write_string(item)?;
                enc.write_string(bitmap_up)?;
                enc.write_string(bitmap_down)?;
                enc.write_wstring(label)?;
            }
            Self::Empty => enc.write_u32(SLOT_EMPTY),
            Self::Other(other) => enc.write_u32(other.raw()),
        }
        Ok(())
    }
}

/// A v7 hotbar set: its id and exactly `slots_per_set` slots.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotbarSet {
    /// Set id.
    pub id: u32,
    /// The slots in file order.
    pub slots: Vec<HotSlot>,
}

/// The hotbar sets; the variant is the block version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hotbars {
    /// Version 4: one set of [`V4_SLOTS`].
    V4(Box<[HotSlot; V4_SLOTS]>),
    /// Version 5: one set of [`V5_SLOTS`].
    V5(Box<[HotSlot; V5_SLOTS]>),
    /// Version 6: one set of [`V6_SLOTS`].
    V6(Box<[HotSlot; V6_SLOTS]>),
    /// Version 7: `sets.len()` sets of `slots_per_set` slots each.
    V7 {
        /// Slots every set holds.
        slots_per_set: u32,
        /// The sets in file order.
        sets: Vec<HotbarSet>,
    },
}

impl Hotbars {
    /// The version this layout belongs to.
    #[must_use]
    pub const fn version(&self) -> UiVersion {
        match self {
            Self::V4(_) => UiVersion::V4,
            Self::V5(_) => UiVersion::V5,
            Self::V6(_) => UiVersion::V6,
            Self::V7 { .. } => UiVersion::V7,
        }
    }

    /// Every slot of every set, in file order.
    pub fn slots(&self) -> impl Iterator<Item = &HotSlot> {
        let (single, sets): (&[HotSlot], &[HotbarSet]) = match self {
            Self::V4(slots) => (slots.as_slice(), &[]),
            Self::V5(slots) => (slots.as_slice(), &[]),
            Self::V6(slots) => (slots.as_slice(), &[]),
            Self::V7 { sets, .. } => (&[], sets.as_slice()),
        };
        single
            .iter()
            .chain(sets.iter().flat_map(|set| set.slots.iter()))
    }

    fn read(dec: &mut Decoder<'_>, version: UiVersion) -> Result<Self, DecodeError> {
        Ok(match version {
            UiVersion::V4 => Self::V4(Box::new(read_array(|| HotSlot::read(dec))?)),
            UiVersion::V5 => Self::V5(Box::new(read_array(|| HotSlot::read(dec))?)),
            UiVersion::V6 => Self::V6(Box::new(read_array(|| HotSlot::read(dec))?)),
            UiVersion::V7 => {
                let set_count = dec.read_u32()?;
                let slots_per_set = dec.read_u32()?;
                let sets = (0..set_count)
                    .map(|_| HotbarSet::read(dec, slots_per_set))
                    .collect::<Result<Vec<_>, _>>()?;
                Self::V7 {
                    slots_per_set,
                    sets,
                }
            }
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        match self {
            Self::V4(slots) => write_slots(enc, slots.as_slice()),
            Self::V5(slots) => write_slots(enc, slots.as_slice()),
            Self::V6(slots) => write_slots(enc, slots.as_slice()),
            Self::V7 {
                slots_per_set,
                sets,
            } => {
                enc.write_u32(crate::block::length_word(sets.len())?);
                enc.write_u32(*slots_per_set);
                sets.iter()
                    .enumerate()
                    .try_for_each(|(index, set)| set.write(enc, index, *slots_per_set))
            }
        }
    }
}

impl HotbarSet {
    fn read(dec: &mut Decoder<'_>, slots_per_set: u32) -> Result<Self, DecodeError> {
        let id = dec.read_u32()?;
        let slots = (0..slots_per_set)
            .map(|_| HotSlot::read(dec))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { id, slots })
    }

    fn write(
        &self,
        enc: &mut Encoder,
        index: usize,
        slots_per_set: u32,
    ) -> Result<(), SaveEncodeError> {
        if crate::block::length_word(self.slots.len())? != slots_per_set {
            return Err(SaveEncodeError::HotbarSetSize {
                set: index,
                expected: slots_per_set,
                found: self.slots.len(),
            });
        }
        enc.write_u32(self.id);
        write_slots(enc, &self.slots)
    }
}

fn write_slots(enc: &mut Encoder, slots: &[HotSlot]) -> Result<(), SaveEncodeError> {
    slots
        .iter()
        .try_for_each(|slot| slot.write(enc).map_err(SaveEncodeError::from))
}

/// Block 14 at a [`UiVersion`].
#[derive(Clone, Debug, PartialEq)]
pub struct Ui {
    /// Unidentified byte (v4+).
    pub unknown_u8_1: u8,
    /// Unidentified word (v4+).
    pub unknown_u32_2: u32,
    /// Unidentified byte (v4+).
    pub unknown_u8_3: u8,
    /// The five unidentified entries (v4+).
    pub unknown_entries: [UnknownEntry; UNKNOWN_ENTRIES],
    /// The hotbar sets.
    pub hotbars: Hotbars,
    /// Camera distance.
    pub camera_distance: f32,
}

impl Ui {
    /// The block version, from the hotbar layout.
    #[must_use]
    pub const fn version(&self) -> UiVersion {
        self.hotbars.version()
    }

    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>, version: UiVersion) -> Result<Self, DecodeError> {
        let unknown_u8_1 = dec.read_u8()?;
        let unknown_u32_2 = dec.read_u32()?;
        let unknown_u8_3 = dec.read_u8()?;
        let unknown_entries = read_array(|| UnknownEntry::read(dec))?;
        let hotbars = Hotbars::read(dec, version)?;
        let camera_distance = dec.read_f32()?;
        Ok(Self {
            unknown_u8_1,
            unknown_u32_2,
            unknown_u8_3,
            unknown_entries,
            hotbars,
            camera_distance,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// [`SaveEncodeError::HotbarSetSize`] when a v7 set does not hold
    /// the declared slot count; cipher errors otherwise.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(self.version().raw());
            enc.write_u8(self.unknown_u8_1);
            enc.write_u32(self.unknown_u32_2);
            enc.write_u8(self.unknown_u8_3);
            self.unknown_entries
                .iter()
                .try_for_each(|entry| entry.write(enc))?;
            self.hotbars.write(enc)?;
            enc.write_f32(self.camera_distance);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::testing::{open_block, round_trip};

    fn slot(index: usize) -> HotSlot {
        match index % 6 {
            0 => HotSlot::Skill {
                skill: format!("records/skills/s{index}.dbr"),
                is_item_skill: 0,
                item: String::new(),
                equip_location: 0,
            },
            1 => HotSlot::HealthPotion,
            2 => HotSlot::EnergyPotion,
            3 => HotSlot::Item {
                item: "records/items/questitems/q.dbr".into(),
                bitmap_up: "ui/up.tex".into(),
                bitmap_down: "ui/down.tex".into(),
                label: "Résumé".into(),
            },
            4 => HotSlot::Empty,
            _ => HotSlot::Other(OtherSlotType::new(9).unwrap()),
        }
    }

    fn slots<const N: usize>() -> Box<[HotSlot; N]> {
        Box::new(std::array::from_fn(slot))
    }

    fn sample(hotbars: Hotbars) -> Ui {
        Ui {
            unknown_u8_1: 1,
            unknown_u32_2: 2,
            unknown_u8_3: 3,
            unknown_entries: std::array::from_fn(|i| UnknownEntry {
                unknown_string_4: format!("four{i}"),
                unknown_string_5: String::new(),
                unknown_u8_6: 6,
            }),
            hotbars,
            camera_distance: 43.5,
        }
    }

    #[test]
    fn round_trips_at_each_version() {
        let layouts = [
            Hotbars::V4(slots()),
            Hotbars::V5(slots()),
            Hotbars::V6(slots()),
            Hotbars::V7 {
                slots_per_set: 3,
                sets: vec![
                    HotbarSet {
                        id: 0,
                        slots: vec![slot(0), slot(3), slot(4)],
                    },
                    HotbarSet {
                        id: 1,
                        slots: vec![slot(1), slot(2), slot(5)],
                    },
                ],
            },
        ];
        for hotbars in layouts {
            let ui = sample(hotbars);
            let version = ui.version();
            round_trip(&ui, Ui::write, |dec| {
                assert_eq!(open_block(dec, BLOCK_ID), version.raw());
                let ui = Ui::read_body(dec, version).unwrap();
                dec.read_block_end().unwrap();
                ui
            });
        }
    }

    #[test]
    fn v7_set_of_the_wrong_size_is_refused() {
        let ui = sample(Hotbars::V7 {
            slots_per_set: 2,
            sets: vec![HotbarSet {
                id: 0,
                slots: vec![slot(0)],
            }],
        });
        let mut enc = Encoder::new(1);
        assert_eq!(
            ui.write(&mut enc),
            Err(SaveEncodeError::HotbarSetSize {
                set: 0,
                expected: 2,
                found: 1
            })
        );
    }

    #[test]
    fn slot_types_the_model_names_are_not_other() {
        for raw in [0, 2, 3, 4, u32::MAX] {
            assert_eq!(OtherSlotType::new(raw), None);
        }
        assert_eq!(OtherSlotType::new(1).map(OtherSlotType::raw), Some(1));
    }

    #[test]
    fn version_set_is_yagdes() {
        assert_eq!(UiVersion::new(3), None);
        assert_eq!(UiVersion::new(4), Some(UiVersion::V4));
        assert_eq!(UiVersion::new(7), Some(UiVersion::V7));
        assert_eq!(UiVersion::new(8), None);
    }
}
