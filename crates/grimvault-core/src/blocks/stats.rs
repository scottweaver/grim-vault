//! Block 16: play statistics.
//!
//! Ported from yagde (MIT, wr8fdy), `src/gd/stats.rs` (v7, v9, v11);
//! grim-save-parser's `play_stats.rs` (v11 only) agrees on the layout.
//! Names follow yagde where the two differ: its `nemesis_kills` are
//! grim-save-parser's `boss_kills`, its survival-mode quartet is
//! grim-save-parser's `survival_wave_tier / greatest_survival_score /
//! cooldown_remaining / cooldown_total`, and its `endless_souls /
//! endless_essence` are the Shattered Realm counters grim-save-parser
//! names as such.
//!
//! Version 12 — what the current game writes; neither reference knows
//! it — was established here (2026-09-03) from the vendored fixture and
//! two real saves: v11's layout plus two more words before the trailing
//! pair, so four unknown words close the block. Under that layout every
//! sample closes on its checksum. The first of the new words is 28 in
//! the fixture and 0 elsewhere; the rest are 0 everywhere. Versions 8
//! and 10 have no sample and are not laid out.

use super::{PerDifficulty, read_vec, write_vec};
use crate::block::SaveEncodeError;
use crate::crypto::{BlockId, DecodeError, Decoder, EncodeError, Encoder};

/// The block id.
pub const BLOCK_ID: BlockId = BlockId::new(16);

/// Layout versions this crate lays out: yagde's, plus v12 (module
/// docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatsVersion {
    /// Version 7: the base layout.
    V7,
    /// Version 9: adds the survival-mode counters.
    V9,
    /// Version 11: adds the skill map and Shattered Realm counters.
    V11,
    /// Version 12: adds two unknown words.
    V12,
}

impl StatsVersion {
    /// The version enum for a raw version word, `None` outside the set.
    #[must_use]
    pub const fn new(raw: u32) -> Option<Self> {
        match raw {
            7 => Some(Self::V7),
            9 => Some(Self::V9),
            11 => Some(Self::V11),
            12 => Some(Self::V12),
            _ => None,
        }
    }

    /// The raw version word.
    #[must_use]
    pub const fn raw(self) -> u32 {
        match self {
            Self::V7 => 7,
            Self::V9 => 9,
            Self::V11 => 11,
            Self::V12 => 12,
        }
    }
}

/// The per-difficulty monster records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonsterRecords {
    /// Record path of the strongest monster killed.
    pub greatest_monster_killed_name: String,
    /// Its level.
    pub greatest_monster_killed_level: u32,
    /// Its life and mana.
    pub greatest_monster_killed_life_and_mana: u32,
    /// Record path of the last monster hit.
    pub last_monster_hit: String,
    /// Record path of the last monster that hit the character.
    pub last_monster_hit_by: String,
}

impl MonsterRecords {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            greatest_monster_killed_name: dec.read_string()?,
            greatest_monster_killed_level: dec.read_u32()?,
            greatest_monster_killed_life_and_mana: dec.read_u32()?,
            last_monster_hit: dec.read_string()?,
            last_monster_hit_by: dec.read_string()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        enc.write_string(&self.greatest_monster_killed_name)?;
        enc.write_u32(self.greatest_monster_killed_level);
        enc.write_u32(self.greatest_monster_killed_life_and_mana);
        enc.write_string(&self.last_monster_hit)?;
        enc.write_string(&self.last_monster_hit_by)
    }
}

/// Survival-mode (Crucible) counters, v9+.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SurvivalStats {
    /// Highest wave reached.
    pub greatest_wave: u32,
    /// Highest score.
    pub greatest_score: u32,
    /// Defenses built.
    pub defenses_built: u32,
    /// Power-ups activated.
    pub powerups_activated: u32,
}

impl SurvivalStats {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            greatest_wave: dec.read_u32()?,
            greatest_score: dec.read_u32()?,
            defenses_built: dec.read_u32()?,
            powerups_activated: dec.read_u32()?,
        })
    }

    fn write(&self, enc: &mut Encoder) {
        enc.write_u32(self.greatest_wave);
        enc.write_u32(self.greatest_score);
        enc.write_u32(self.defenses_built);
        enc.write_u32(self.powerups_activated);
    }
}

/// One entry of the v11 skill map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillUse {
    /// Skill record path.
    pub skill: String,
    /// Word both references call `active` / `num`.
    pub active: u32,
}

impl SkillUse {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            skill: dec.read_string()?,
            active: dec.read_u32()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        enc.write_string(&self.skill)?;
        enc.write_u32(self.active);
        Ok(())
    }
}

/// Shattered Realm counters and the skill map, v11+.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShatteredRealmStats {
    /// Skill map in file order.
    pub skill_map: Vec<SkillUse>,
    /// Souls (yagde `endless_souls`).
    pub souls: u32,
    /// Essence (yagde `endless_essence`).
    pub essence: u32,
    /// Byte both references call `difficulty_skip`.
    pub difficulty_skip: u8,
}

impl ShatteredRealmStats {
    fn read(dec: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        Ok(Self {
            skill_map: read_vec(dec, SkillUse::read)?,
            souls: dec.read_u32()?,
            essence: dec.read_u32()?,
            difficulty_skip: dec.read_u8()?,
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        write_vec(enc, &self.skill_map, |enc, entry| entry.write(enc))?;
        enc.write_u32(self.souls);
        enc.write_u32(self.essence);
        enc.write_u8(self.difficulty_skip);
        Ok(())
    }
}

/// The version-gated fields between the nemesis kills and the trailing
/// unknown words; the variant is the block version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsExtension {
    /// Version 7: nothing.
    V7,
    /// Version 9: survival counters.
    V9 {
        /// Survival-mode counters.
        survival: SurvivalStats,
    },
    /// Version 11: survival counters plus the Shattered Realm fields.
    V11 {
        /// Survival-mode counters.
        survival: SurvivalStats,
        /// Skill map and Shattered Realm counters.
        shattered_realm: ShatteredRealmStats,
    },
    /// Version 12: v11 plus two unidentified words.
    V12 {
        /// Survival-mode counters.
        survival: SurvivalStats,
        /// Skill map and Shattered Realm counters.
        shattered_realm: ShatteredRealmStats,
        /// Unidentified word (v12+; 28 in the fixture, else 0).
        unknown_u32_v12_a: u32,
        /// Unidentified word (v12+; 0 in every sample).
        unknown_u32_v12_b: u32,
    },
}

impl StatsExtension {
    /// The version this extension belongs to.
    #[must_use]
    pub const fn version(&self) -> StatsVersion {
        match self {
            Self::V7 => StatsVersion::V7,
            Self::V9 { .. } => StatsVersion::V9,
            Self::V11 { .. } => StatsVersion::V11,
            Self::V12 { .. } => StatsVersion::V12,
        }
    }

    fn read(dec: &mut Decoder<'_>, version: StatsVersion) -> Result<Self, DecodeError> {
        Ok(match version {
            StatsVersion::V7 => Self::V7,
            StatsVersion::V9 => Self::V9 {
                survival: SurvivalStats::read(dec)?,
            },
            StatsVersion::V11 => Self::V11 {
                survival: SurvivalStats::read(dec)?,
                shattered_realm: ShatteredRealmStats::read(dec)?,
            },
            StatsVersion::V12 => Self::V12 {
                survival: SurvivalStats::read(dec)?,
                shattered_realm: ShatteredRealmStats::read(dec)?,
                unknown_u32_v12_a: dec.read_u32()?,
                unknown_u32_v12_b: dec.read_u32()?,
            },
        })
    }

    fn write(&self, enc: &mut Encoder) -> Result<(), EncodeError> {
        match self {
            Self::V7 => Ok(()),
            Self::V9 { survival } => {
                survival.write(enc);
                Ok(())
            }
            Self::V11 {
                survival,
                shattered_realm,
            } => {
                survival.write(enc);
                shattered_realm.write(enc)
            }
            Self::V12 {
                survival,
                shattered_realm,
                unknown_u32_v12_a,
                unknown_u32_v12_b,
            } => {
                survival.write(enc);
                shattered_realm.write(enc)?;
                enc.write_u32(*unknown_u32_v12_a);
                enc.write_u32(*unknown_u32_v12_b);
                Ok(())
            }
        }
    }
}

/// Block 16 at a [`StatsVersion`].
#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    /// Play time in seconds.
    pub playtime: u32,
    /// Deaths.
    pub deaths: u32,
    /// Kills.
    pub kills: u32,
    /// Experience earned from kills.
    pub experience_from_kills: u32,
    /// Health potions used.
    pub health_potions_used: u32,
    /// Energy potions used.
    pub mana_potions_used: u32,
    /// Highest level reached.
    pub max_level: u32,
    /// Hits received.
    pub hits_received: u32,
    /// Hits inflicted.
    pub hits_inflicted: u32,
    /// Critical hits inflicted.
    pub critical_hits_inflicted: u32,
    /// Critical hits received.
    pub critical_hits_received: u32,
    /// Greatest damage inflicted in one hit.
    pub greatest_damage_inflicted: f32,
    /// Monster records, per difficulty.
    pub monsters: PerDifficulty<MonsterRecords>,
    /// Champion kills.
    pub champion_kills: u32,
    /// Damage of the last hit inflicted.
    pub last_hit: f32,
    /// Damage of the last hit received.
    pub last_hit_by: f32,
    /// Greatest damage received in one hit.
    pub greatest_damage_received: f32,
    /// Hero kills.
    pub hero_kills: u32,
    /// Items crafted.
    pub items_crafted: u32,
    /// Relics crafted.
    pub relics_crafted: u32,
    /// Transcendent relics crafted.
    pub transcendent_relics_crafted: u32,
    /// Mythical relics crafted.
    pub mythical_relics_crafted: u32,
    /// Shrines restored.
    pub shrines_restored: u32,
    /// One-shot chests opened.
    pub one_shot_chests_opened: u32,
    /// Lore notes collected.
    pub lore_notes_collected: u32,
    /// Nemesis kills, per difficulty.
    pub nemesis_kills: PerDifficulty<u32>,
    /// The version-gated fields.
    pub extension: StatsExtension,
    /// Unidentified trailing word (v7+).
    pub unknown_u32_1: u32,
    /// Unidentified trailing word (v7+).
    pub unknown_u32_2: u32,
}

impl Stats {
    /// The block version, from the extension.
    #[must_use]
    pub const fn version(&self) -> StatsVersion {
        self.extension.version()
    }

    /// Reads the body after the version word.
    ///
    /// # Errors
    /// Cipher / bounds errors.
    pub fn read_body(dec: &mut Decoder<'_>, version: StatsVersion) -> Result<Self, DecodeError> {
        Ok(Self {
            playtime: dec.read_u32()?,
            deaths: dec.read_u32()?,
            kills: dec.read_u32()?,
            experience_from_kills: dec.read_u32()?,
            health_potions_used: dec.read_u32()?,
            mana_potions_used: dec.read_u32()?,
            max_level: dec.read_u32()?,
            hits_received: dec.read_u32()?,
            hits_inflicted: dec.read_u32()?,
            critical_hits_inflicted: dec.read_u32()?,
            critical_hits_received: dec.read_u32()?,
            greatest_damage_inflicted: dec.read_f32()?,
            monsters: PerDifficulty::read(|| MonsterRecords::read(dec))?,
            champion_kills: dec.read_u32()?,
            last_hit: dec.read_f32()?,
            last_hit_by: dec.read_f32()?,
            greatest_damage_received: dec.read_f32()?,
            hero_kills: dec.read_u32()?,
            items_crafted: dec.read_u32()?,
            relics_crafted: dec.read_u32()?,
            transcendent_relics_crafted: dec.read_u32()?,
            mythical_relics_crafted: dec.read_u32()?,
            shrines_restored: dec.read_u32()?,
            one_shot_chests_opened: dec.read_u32()?,
            lore_notes_collected: dec.read_u32()?,
            nemesis_kills: PerDifficulty::read(|| dec.read_u32())?,
            extension: StatsExtension::read(dec, version)?,
            unknown_u32_1: dec.read_u32()?,
            unknown_u32_2: dec.read_u32()?,
        })
    }

    /// Writes the whole framed block.
    ///
    /// # Errors
    /// Cipher errors.
    pub fn write(&self, enc: &mut Encoder) -> Result<(), SaveEncodeError> {
        enc.write_block(BLOCK_ID, |enc| {
            enc.write_u32(self.version().raw());
            enc.write_u32(self.playtime);
            enc.write_u32(self.deaths);
            enc.write_u32(self.kills);
            enc.write_u32(self.experience_from_kills);
            enc.write_u32(self.health_potions_used);
            enc.write_u32(self.mana_potions_used);
            enc.write_u32(self.max_level);
            enc.write_u32(self.hits_received);
            enc.write_u32(self.hits_inflicted);
            enc.write_u32(self.critical_hits_inflicted);
            enc.write_u32(self.critical_hits_received);
            enc.write_f32(self.greatest_damage_inflicted);
            self.monsters.try_for_each(|records| records.write(enc))?;
            enc.write_u32(self.champion_kills);
            enc.write_f32(self.last_hit);
            enc.write_f32(self.last_hit_by);
            enc.write_f32(self.greatest_damage_received);
            enc.write_u32(self.hero_kills);
            enc.write_u32(self.items_crafted);
            enc.write_u32(self.relics_crafted);
            enc.write_u32(self.transcendent_relics_crafted);
            enc.write_u32(self.mythical_relics_crafted);
            enc.write_u32(self.shrines_restored);
            enc.write_u32(self.one_shot_chests_opened);
            enc.write_u32(self.lore_notes_collected);
            self.nemesis_kills.try_for_each(|&kills| {
                enc.write_u32(kills);
                Ok::<(), EncodeError>(())
            })?;
            self.extension.write(enc)?;
            enc.write_u32(self.unknown_u32_1);
            enc.write_u32(self.unknown_u32_2);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::testing::{open_block, round_trip};

    fn sample(extension: StatsExtension) -> Stats {
        Stats {
            playtime: 360_000,
            deaths: 3,
            kills: 45_000,
            experience_from_kills: 9_000_000,
            health_potions_used: 120,
            mana_potions_used: 30,
            max_level: 100,
            hits_received: 20_000,
            hits_inflicted: 90_000,
            critical_hits_inflicted: 12_000,
            critical_hits_received: 900,
            greatest_damage_inflicted: 123_456.5,
            monsters: PerDifficulty {
                normal: MonsterRecords {
                    greatest_monster_killed_name: "records/creatures/boss.dbr".into(),
                    greatest_monster_killed_level: 30,
                    greatest_monster_killed_life_and_mana: 40_000,
                    last_monster_hit: "records/creatures/zombie.dbr".into(),
                    last_monster_hit_by: "records/creatures/zombie.dbr".into(),
                },
                elite: MonsterRecords::default(),
                ultimate: MonsterRecords::default(),
            },
            champion_kills: 800,
            last_hit: 1_200.25,
            last_hit_by: 300.0,
            greatest_damage_received: 9_999.0,
            hero_kills: 200,
            items_crafted: 12,
            relics_crafted: 4,
            transcendent_relics_crafted: 1,
            mythical_relics_crafted: 0,
            shrines_restored: 20,
            one_shot_chests_opened: 5,
            lore_notes_collected: 60,
            nemesis_kills: PerDifficulty {
                normal: 2,
                elite: 0,
                ultimate: 1,
            },
            extension,
            unknown_u32_1: 0,
            unknown_u32_2: 7,
        }
    }

    #[test]
    fn round_trips_at_each_version() {
        let survival = SurvivalStats {
            greatest_wave: 150,
            greatest_score: 123_456,
            defenses_built: 20,
            powerups_activated: 9,
        };
        let shattered_realm = ShatteredRealmStats {
            skill_map: vec![SkillUse {
                skill: "records/skills/playerclass01/skill_01.dbr".into(),
                active: 4_200,
            }],
            souls: 12,
            essence: 34,
            difficulty_skip: 1,
        };
        for extension in [
            StatsExtension::V7,
            StatsExtension::V9 {
                survival: survival.clone(),
            },
            StatsExtension::V11 {
                survival: survival.clone(),
                shattered_realm: shattered_realm.clone(),
            },
            StatsExtension::V12 {
                survival,
                shattered_realm,
                unknown_u32_v12_a: 28,
                unknown_u32_v12_b: 0,
            },
        ] {
            let stats = sample(extension);
            let version = stats.version();
            round_trip(&stats, Stats::write, |dec| {
                assert_eq!(open_block(dec, BLOCK_ID), version.raw());
                let stats = Stats::read_body(dec, version).unwrap();
                dec.read_block_end().unwrap();
                stats
            });
        }
    }

    #[test]
    fn version_set_is_yagdes_plus_v12() {
        assert_eq!(StatsVersion::new(7), Some(StatsVersion::V7));
        assert_eq!(StatsVersion::new(8), None);
        assert_eq!(StatsVersion::new(9), Some(StatsVersion::V9));
        assert_eq!(StatsVersion::new(10), None);
        assert_eq!(StatsVersion::new(11), Some(StatsVersion::V11));
        assert_eq!(StatsVersion::new(12), Some(StatsVersion::V12));
        assert_eq!(StatsVersion::new(13), None);
    }
}
