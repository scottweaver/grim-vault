//! Item stat lines: the display text of every stat a database record
//! contributes, rendered through the game's own `tags_ui.txt`
//! templates, and the per-item assembly of those records into a
//! tooltip — base, affixes, component, augment, ascendant bonus, set,
//! requirements. The facts the search will filter on (a line's text,
//! its numbers, which family it belongs to) are typed values here, not
//! prose.
//!
//! Layout: [`vocabulary`] is the Grim Dawn dialect (which variables
//! are stats, which tag renders each, what scales and what is hidden);
//! [`render`] is the engine-generic machinery that turns one record
//! into [`RecordStats`] under that dialect; [`item`] assembles a
//! [`ItemDetails`] for an [`Item`]. Rendering runs once per
//! `(record, level, scale)` and is memoized in the [`StatCache`] the
//! game data carries, so the shell asks per frame for free.
//!
//! Every rendered number is the record's nominal value: the game rolls
//! affix values within `lootRandomizerJitter` per item seed, and the
//! seed → roll mapping is not public, so the true in-game figure can
//! sit a few percent either side of what is shown.

pub mod item;
pub mod render;
pub mod vocabulary;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use univault_engine::ids::{RecordId, normalize};

pub use item::{
    Block, BlockSource, ItemDetails, SetInfo, SetTier, item_details, requirements, set_name,
};
pub use render::{RecordStats, Renderer};
pub use vocabulary::{Effect, Emphasis, Family, Part, Section};

use crate::gamedata::GameData;

/// One rendered tooltip line: what produced it, where it sits, how it
/// is coloured, its text (colour codes stripped), and the numbers
/// that went into the text in order — so a search can bound "12-28
/// Fire Damage" on 12 and 28 without re-parsing the text.
#[derive(Clone, Debug, PartialEq)]
pub struct StatLine {
    pub kind: LineKind,
    pub section: Section,
    pub emphasis: Emphasis,
    pub text: String,
    pub values: Vec<f32>,
}

/// What a line renders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineKind {
    /// A stat family from the vocabulary.
    Effect(Effect),
    /// The weapon's speed class (`characterBaseAttackSpeedTag`).
    AttackSpeed,
    /// A shield's `blockRecoveryTime`.
    BlockRecovery,
    /// `conversionPercentage` with its in and out types.
    Conversion,
    /// A `racialBonus*` line for one race.
    Racial,
    /// `augmentSkillName<N>` + `augmentSkillLevel<N>`.
    SkillBonus,
    /// `augmentMasteryName<N>` + `augmentMasteryLevel<N>`.
    MasteryBonus,
    /// `augmentAllLevel`.
    AllSkillsBonus,
    /// The "Grants Skill: …" heading of `itemSkillName`.
    GrantedSkill,
    /// The granted skill's `skillBaseDescription`.
    SkillDescription,
    /// The granted skill's level.
    SkillLevel,
    /// The "Bonus to All Pets" heading of `petBonusName`.
    PetBonusHeading,
    /// A set's name.
    SetName,
    /// One member of a set.
    SetMember,
    /// A requirement line.
    Requirement(Requirement),
}

/// A variable the renderer could not turn into a line, and why —
/// reported rather than dropped so coverage is measurable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unrendered {
    pub variable: String,
    pub reason: UnrenderedReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnrenderedReason {
    /// The vocabulary does not know the variable.
    UnknownAttribute,
    /// The vocabulary names a tag the text archives lack.
    MissingTag(String),
    /// A referenced skill, controller, or bonus record is absent.
    MissingRecord(String),
}

impl fmt::Display for Unrendered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            UnrenderedReason::UnknownAttribute => write!(f, "{}: unknown attribute", self.variable),
            UnrenderedReason::MissingTag(tag) => {
                write!(f, "{}: no text for tag {tag}", self.variable)
            }
            UnrenderedReason::MissingRecord(id) => {
                write!(f, "{}: no record {id}", self.variable)
            }
        }
    }
}

/// The four requirements the game formats through
/// `MeetsRequirement`, under Grim Dawn's names for the three
/// attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Requirement {
    Level,
    Physique,
    Cunning,
    Spirit,
}

impl Requirement {
    /// Every requirement, level first.
    pub const ALL: [Self; 4] = [Self::Level, Self::Physique, Self::Cunning, Self::Spirit];

    /// The attribute requirements a record may state explicitly.
    pub const EXPLICIT: [Self; 3] = [Self::Physique, Self::Cunning, Self::Spirit];

    /// The requirement as the game names it in English.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Level => "Level",
            Self::Physique => "Physique",
            Self::Cunning => "Cunning",
            Self::Spirit => "Spirit",
        }
    }

    /// The record variable stating the requirement outright.
    #[must_use]
    pub fn variable(self) -> &'static str {
        match self {
            Self::Level => "levelRequirement",
            Self::Physique => "strengthRequirement",
            Self::Cunning => "dexterityRequirement",
            Self::Spirit => "intelligenceRequirement",
        }
    }

    /// The middle of the cost-formula equation name
    /// (`sword` + `Dexterity` + `Equation`).
    #[must_use]
    pub fn equation_attribute(self) -> Option<&'static str> {
        match self {
            Self::Level => None,
            Self::Physique => Some("Strength"),
            Self::Cunning => Some("Dexterity"),
            Self::Spirit => Some("Intelligence"),
        }
    }

    /// The tag naming the requirement in `MeetsRequirement`.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Level => "LevelRequirement",
            Self::Physique => "Strength",
            Self::Cunning => "Dexterity",
            Self::Spirit => "Intelligence",
        }
    }
}

/// A skill level, 1-based as the game counts; indexes a skill
/// record's per-level arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkillLevel(u32);

impl SkillLevel {
    pub const ONE: Self = Self(1);

    #[must_use]
    pub fn new(level: u32) -> Option<Self> {
        (level >= 1).then_some(Self(level))
    }

    /// An `itemSkillLevelEq` result: whole levels, never below one.
    #[must_use]
    pub fn from_equation(value: f64) -> Option<Self> {
        value.is_finite().then(|| Self(floor_level(value)))
    }

    #[must_use]
    pub fn get(self) -> u32 {
        self.0
    }

    /// The array index of this level.
    #[must_use]
    pub fn index(self) -> usize {
        (self.0 - 1) as usize
    }
}

/// An item's `attributeScalePercent`: the percentage its own and its
/// affixes' offensive values grow by (two-handed weapons carry the
/// largest).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Scale(i32);

impl Scale {
    pub const NONE: Self = Self(0);

    #[must_use]
    pub fn percent(percent: i32) -> Self {
        Self(percent)
    }

    #[must_use]
    pub fn factor(self) -> f32 {
        1.0 + whole(self.0) / 100.0
    }
}

/// The equations yield small positive levels (1..~30); the floor is
/// clamped to one before the cast.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 1.0..=f64::MAX before the cast"
)]
fn floor_level(value: f64) -> u32 {
    value.floor().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// A game count (a level, a piece count, a requirement) as the number
/// the format templates take.
#[allow(
    clippy::cast_precision_loss,
    reason = "game counts stay far below 2^24, where f32 is exact"
)]
pub(crate) fn count(value: u32) -> f32 {
    value as f32
}

/// A database integer as the number the format templates take.
#[allow(
    clippy::cast_precision_loss,
    reason = "database integers stay far below 2^24, where f32 is exact"
)]
pub(crate) fn whole(value: i32) -> f32 {
    value as f32
}

/// Renders once per `(record, level, scale)` and hands out shared
/// results; the game data owns one so every shell path sees the same
/// memo.
#[derive(Default)]
pub struct StatCache {
    entries: Mutex<HashMap<CacheKey, Arc<RecordStats>>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    record: String,
    level: SkillLevel,
    scale: Scale,
}

impl StatCache {
    /// The rendered stats of `id`, `None` when no layer has the record.
    pub fn record_stats(
        &self,
        game: &GameData,
        id: &RecordId,
        level: SkillLevel,
        scale: Scale,
    ) -> Option<Arc<RecordStats>> {
        let key = CacheKey {
            record: normalize(id.as_str()),
            level,
            scale,
        };
        if let Some(hit) = self.lock().get(&key) {
            return Some(Arc::clone(hit));
        }
        let record = game.record(id)?.ok()?;
        let rendered = Arc::new(Renderer::new(game).render(&record, level, scale));
        self.lock().insert(key, Arc::clone(&rendered));
        Some(rendered)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<CacheKey, Arc<RecordStats>>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_levels_start_at_one_and_floor_equations() {
        assert_eq!(SkillLevel::new(0), None);
        assert_eq!(SkillLevel::new(3).map(SkillLevel::index), Some(2));
        assert_eq!(
            SkillLevel::from_equation(19.75).map(SkillLevel::get),
            Some(19)
        );
        assert_eq!(SkillLevel::from_equation(0.2), Some(SkillLevel::ONE));
        assert_eq!(SkillLevel::from_equation(f64::NAN), None);
    }

    #[test]
    fn scale_is_a_percentage_over_one() {
        assert!((Scale::percent(30).factor() - 1.3).abs() < 1e-6);
        assert!((Scale::NONE.factor() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn requirements_name_their_variables_and_tags() {
        assert_eq!(Requirement::Physique.variable(), "strengthRequirement");
        assert_eq!(Requirement::Cunning.equation_attribute(), Some("Dexterity"));
        assert_eq!(Requirement::Level.equation_attribute(), None);
        assert_eq!(Requirement::Spirit.tag(), "Intelligence");
    }
}
