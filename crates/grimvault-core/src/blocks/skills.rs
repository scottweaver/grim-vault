//! Block 8: mastery, devotion and item-granted skills.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/skills.rs` (v5, v6).
//! grim-save-parser's `character_skills.rs` / `skill.rs` /
//! `item_skill.rs` agree on every field through the item skills; for
//! the word v6 adds they diverge — yagde reads a counted list of
//! [`SubSkill`]s, grim-save-parser one unnamed `u32`. The two are
//! indistinguishable while the count is 0 (every save seen so far);
//! yagde's reading is used because it is the one with a writer.
//!
//! Version 8 — what the current game writes; neither reference knows
//! it — was established here (2026-09-03) from the vendored fixture and
//! three real saves: each [`Skill`] carries one more byte after
//! `enabled`, and the rest of the block is v6's. Every skill of every
//! v8 sample parses to a `records/skills/` path and the block closes on
//! its checksum under that layout. The byte is 1 on the
//! `itemskillsgdx3/potionmodifiers/healthpotion_*` entries and 0
//! elsewhere; its meaning is not established. Version 7 has no sample
//! and is not laid out.

use std::fmt;

use super::{read_vec, write_vec};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, EncodeError, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(8);

/// Layout versions this crate lays out: yagde's, plus v8 (module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SkillsVersion {
    /// Version 5: through the item skills.
    V5,
    /// Version 6: adds the sub-skill list.
    V6,
    /// Version 8: v6 plus one byte per skill.
    V8,
}

impl SkillsVersion {
    /// The version enum for a raw version word, `None` outside the set.
    #[must_use]
    pub const fn new(raw: u32) -> Option<Self> {
        match raw {
            5 => Some(Self::V5),
            6 => Some(Self::V6),
            8 => Some(Self::V8),
            _ => None,
        }
    }

    /// The raw version word.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::V5 => 5,
            Self::V6 => 6,
            Self::V8 => 8,
        }
    }

    /// Skills carry [`Skill::unknown_u8_v8`].
    #[must_use]
    pub const fn has_skill_unknown_u8_v8(self) -> bool {
        matches!(self, Self::V8)
    }
}

impl fmt::Display for SkillsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.raw())
    }
}

/// Skill fields whose presence depends on the block version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillField {
    /// [`Skill::unknown_u8_v8`], present from v8.
    UnknownU8V8,
}

impl fmt::Display for SkillField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownU8V8 => "unknown_u8_v8",
        })
    }
}

/// A mastery or devotion skill. `enabled` and the unknown bytes are
/// read without interpretation and kept as read. `unknown_u8_v8` reads
/// as 0 below v8 and refuses to be written non-zero there
/// ([`SaveEncodeError::SkillFieldNotInVersion`]), the policy of
/// [`crate::item::Item`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skill {
    /// Skill record path.
    pub name: String,
    /// Points invested.
    pub level: u32,
    /// Byte both references call `enabled`.
    pub enabled: u8,
    /// Unidentified byte (v8+); see the module docs.
    pub unknown_u8_v8: u8,
    /// Devotion level (0 for mastery skills).
    pub devotion_level: u32,
    /// Devotion experience.
    pub experience: u32,
    /// Word both references call `active`.
    pub active: u32,
    /// Unidentified byte (v5+).
    pub unknown_u8_1: u8,
    /// Unidentified byte (v5+).
    pub unknown_u8_2: u8,
    /// Record path of the skill this one auto-casts, or empty.
    pub auto_cast_skill: String,
    /// Record path of the auto-cast controller, or empty.
    pub auto_cast_controller: String,
}

impl Skill {
    fn read(dec: &mut Decoder<'_>, version: SkillsVersion) -> Result<Self, DecodeError> {
        Ok(Self {
            name: dec.read_string()?,
            level: dec.read_u32()?,
            enabled: dec.read_u8()?,
            unknown_u8_v8: if version.has_skill_unknown_u8_v8() {
                dec.read_u8()?
            } else {
                0
            },
            devotion_level: dec.read_u32()?,
            experience: dec.read_u32()?,
            active: dec.read_u32()?,
            unknown_u8_1: dec.read_u8()?,
            unknown_u8_2: dec.read_u8()?,
            auto_cast_skill: dec.read_string()?,
            auto_cast_controller: dec.read_string()?,
        })
    }

    fn write(&self, enc: &mut Encoder, version: SkillsVersion) -> Result<(), SaveEncodeError> {
        enc.write_string(&self.name)?;
        enc.write_u32(self.level);
        enc.write_u8(self.enabled);
        if version.has_skill_unknown_u8_v8() {
            enc.write_u8(self.unknown_u8_v8);
        } else if self.unknown_u8_v8 != 0 {
            return Err(SaveEncodeError::SkillFieldNotInVersion {
                field: SkillField::UnknownU8V8,
                version,
            });
        }
        enc.write_u32(self.devotion_level);
        enc.write_u32(self.experience);
        enc.write_u32(self.active);
        enc.write_u8(self.unknown_u8_1);
        enc.write_u8(self.unknown_u8_2);
        enc.write_string(&self.auto_cast_skill)?;
        enc.write_string(&self.auto_cast_controller)?;
        Ok(())
    }
}

/// A skill granted by an equipped item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemSkill {
    /// Skill record path.
    pub name: String,
    /// Record path of the skill this one auto-casts, or empty.
    pub auto_cast_skill: String,
    /// Record path of the auto-cast controller, or empty.
    pub auto_cast_controller: String,
    /// Equipment slot index of the granting item.
    pub item_slot: u32,
    /// Record path of the granting item.
    pub item_name: String,
}

impl ItemSkill {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            name: dec.read_string()?,
            auto_cast_skill: dec.read_string()?,
            auto_cast_controller: dec.read_string()?,
            item_slot: dec.read_u32()?,
            item_name: dec.read_string()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        enc.write_string(&self.name)?;
        enc.write_string(&self.auto_cast_skill)?;
        enc.write_string(&self.auto_cast_controller)?;
        enc.write_u32(self.item_slot);
        enc.write_string(&self.item_name)
    }
}

/// A sub-skill entry (v6+), per yagde.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubSkill {
    /// Skill record path.
    pub name: String,
    /// Record path of the skill this one auto-casts, or empty.
    pub auto_cast_skill: String,
    /// Record path of the auto-cast controller, or empty.
    pub auto_cast_controller: String,
    /// Record path of the parent skill.
    pub parent_skill: String,
}

impl SubSkill {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            name: dec.read_string()?,
            auto_cast_skill: dec.read_string()?,
            auto_cast_controller: dec.read_string()?,
            parent_skill: dec.read_string()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        enc.write_string(&self.name)?;
        enc.write_string(&self.auto_cast_skill)?;
        enc.write_string(&self.auto_cast_controller)?;
        enc.write_string(&self.parent_skill)
    }
}

/// What follows the item skills; the variant is the block version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillsExtension {
    /// Version 5: nothing.
    V5,
    /// Version 6: the sub-skill list.
    V6 {
        /// Sub-skills in file order.
        sub_skills: Vec<SubSkill>,
    },
    /// Version 8: the sub-skill list (the per-skill byte lives in
    /// [`Skill::unknown_u8_v8`]).
    V8 {
        /// Sub-skills in file order.
        sub_skills: Vec<SubSkill>,
    },
}

impl SkillsExtension {
    /// The version this extension belongs to.
    #[must_use]
    pub const fn version(&self) -> SkillsVersion {
        match self {
            Self::V5 => SkillsVersion::V5,
            Self::V6 { .. } => SkillsVersion::V6,
            Self::V8 { .. } => SkillsVersion::V8,
        }
    }

    fn read(dec: &mut Decoder<'_>, version: SkillsVersion) -> Result<Self, DecodeError> {
        Ok(match version {
            SkillsVersion::V5 => Self::V5,
            SkillsVersion::V6 => Self::V6 {
                sub_skills: read_vec(dec, SubSkill::read)?,
            },
            SkillsVersion::V8 => Self::V8 {
                sub_skills: read_vec(dec, SubSkill::read)?,
            },
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        match self {
            Self::V5 => Ok(()),
            Self::V6 { sub_skills } | Self::V8 { sub_skills } => {
                write_vec(enc, sub_skills, |enc, skill| skill.write(enc))
            }
        }
    }
}

/// Block 8 at a [`SkillsVersion`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skills {
    /// Mastery and devotion skills in file order.
    pub skills: Vec<Skill>,
    /// Masteries the character may choose.
    pub masteries_allowed: u32,
    /// Skill points reclaimed so far.
    pub skill_reclamation_points_used: u32,
    /// Devotion points reclaimed so far.
    pub devotion_reclamation_points_used: u32,
    /// Item-granted skills in file order.
    pub item_skills: Vec<ItemSkill>,
    /// The version-gated remainder.
    pub extension: SkillsExtension,
}

impl Skills {
    /// The block version, from the extension.
    #[must_use]
    pub const fn version(&self) -> SkillsVersion {
        self.extension.version()
    }

    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>, version: SkillsVersion) -> Result<Self, DecodeError> {
        let skills = read_vec(dec, |dec| Skill::read(dec, version))?;
        let masteries_allowed = dec.read_u32()?;
        let skill_reclamation_points_used = dec.read_u32()?;
        let devotion_reclamation_points_used = dec.read_u32()?;
        let item_skills = read_vec(dec, ItemSkill::read)?;
        let extension = SkillsExtension::read(dec, version)?;
        Ok(Self {
            skills,
            masteries_allowed,
            skill_reclamation_points_used,
            devotion_reclamation_points_used,
            item_skills,
            extension,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// [`SaveEncodeError::SkillFieldNotInVersion`] when a skill carries
    /// a field the version has no slot for; cipher errors otherwise.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        let version = self.version();
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(version.raw());
            write_vec(enc, &self.skills, |enc, skill| skill.write(enc, version))?;
            enc.write_u32(self.masteries_allowed);
            enc.write_u32(self.skill_reclamation_points_used);
            enc.write_u32(self.devotion_reclamation_points_used);
            write_vec(enc, &self.item_skills, |enc, skill| skill.write(enc))?;
            self.extension.write(enc)?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::testing::{open_block, round_trip};

    fn sample(extension: SkillsExtension) -> Skills {
        Skills {
            skills: vec![
                Skill {
                    name: "records/skills/playerclass01/skill_01.dbr".into(),
                    level: 12,
                    enabled: 1,
                    unknown_u8_v8: 0,
                    devotion_level: 0,
                    experience: 0,
                    active: 1,
                    unknown_u8_1: 0,
                    unknown_u8_2: 0,
                    auto_cast_skill: String::new(),
                    auto_cast_controller: String::new(),
                },
                Skill {
                    name: "records/skills/devotion/tier1_01.dbr".into(),
                    level: 1,
                    enabled: 1,
                    unknown_u8_v8: 0,
                    devotion_level: 3,
                    experience: 4_400,
                    active: 0,
                    unknown_u8_1: 1,
                    unknown_u8_2: 0,
                    auto_cast_skill: "records/skills/playerclass01/skill_01.dbr".into(),
                    auto_cast_controller: "records/controllers/player/autocast.dbr".into(),
                },
            ],
            masteries_allowed: 2,
            skill_reclamation_points_used: 7,
            devotion_reclamation_points_used: 2,
            item_skills: vec![ItemSkill {
                name: "records/skills/itemskills/proc.dbr".into(),
                auto_cast_skill: String::new(),
                auto_cast_controller: String::new(),
                item_slot: 3,
                item_name: "records/items/gearhead/h.dbr".into(),
            }],
            extension,
        }
    }

    fn sub_skills() -> Vec<SubSkill> {
        vec![SubSkill {
            name: "records/skills/sub.dbr".into(),
            auto_cast_skill: String::new(),
            auto_cast_controller: String::new(),
            parent_skill: "records/skills/parent.dbr".into(),
        }]
    }

    #[test]
    fn round_trips_at_each_version() {
        for extension in [
            SkillsExtension::V5,
            SkillsExtension::V6 {
                sub_skills: sub_skills(),
            },
            SkillsExtension::V8 {
                sub_skills: sub_skills(),
            },
        ] {
            let mut skills = sample(extension);
            if skills.version().has_skill_unknown_u8_v8() {
                skills.skills[0].unknown_u8_v8 = 1;
            }
            let version = skills.version();
            round_trip(&skills, Skills::write, |dec| {
                assert_eq!(open_block(dec, BLOCK_ID), version.raw());
                let skills = Skills::read_body(dec, version).unwrap();
                dec.read_block_end().unwrap();
                skills
            });
        }
    }

    #[test]
    fn the_v8_byte_refuses_older_versions() {
        let mut skills = sample(SkillsExtension::V6 { sub_skills: vec![] });
        skills.skills[1].unknown_u8_v8 = 1;
        let mut enc = Encoder::new(1);
        assert_eq!(
            skills.write(&mut enc),
            Err(SaveEncodeError::SkillFieldNotInVersion {
                field: SkillField::UnknownU8V8,
                version: SkillsVersion::V6
            })
        );
    }

    #[test]
    fn version_set_is_yagdes_plus_v8() {
        assert_eq!(SkillsVersion::new(5), Some(SkillsVersion::V5));
        assert_eq!(SkillsVersion::new(6), Some(SkillsVersion::V6));
        assert_eq!(SkillsVersion::new(8), Some(SkillsVersion::V8));
        assert_eq!(SkillsVersion::new(4), None);
        assert_eq!(SkillsVersion::new(7), None);
        assert_eq!(SkillsVersion::new(9), None);
    }
}
