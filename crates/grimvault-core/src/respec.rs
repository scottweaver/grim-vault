//! Respec: the plain full refunds — every attribute point back into
//! block 2's pool, and every mastery skill out of block 8 with its
//! points back, so both masteries can be chosen again. What the
//! in-game Spirit Guide sells one point at a time, done at once.
//!
//! Both rules are data-driven, read through [`GameData`]
//! (`docs/format-references.md`, "Respec"):
//!
//! - `records/creatures/pc/playerlevels.dbr` — what one attribute
//!   point buys (`strengthIncrement` / `dexterityIncrement` /
//!   `intelligenceIncrement`, the game keeping Titan Quest's names for
//!   physique / cunning / spirit), what it adds to the health pool
//!   (`lifeIncrement`, `lifeIncrementDexterity`,
//!   `lifeIncrementIntelligence`) and the energy pool
//!   (`manaIncrement`, spirit only), the attribute and skill points
//!   per level, and the devotion cap.
//! - `records/creatures/pc/malepc01.dbr` — the base attributes and
//!   pools a fresh character starts with (`characterStrength`,
//!   `characterDexterity`, `characterIntelligence`, `characterLife`,
//!   `characterMana`; `femalepc01.dbr` carries the same values) and
//!   the mastery trees `skillTree1..N`. A tree's `skillName*` list is
//!   its membership; the members' `grantedSkills` are the ones the
//!   mastery hands out for free, removed with the rest but refunding
//!   nothing.
//!
//! Block 2's `health` and `energy` are the base pools the points
//! bought, not the current values: on every real save they equal
//! exactly `characterLife + Σ points × increment`, so a reset puts
//! them back to the base and refuses when they do not add up — an
//! unknown rule is never overwritten.

use std::collections::BTreeSet;
use std::fmt;

use thiserror::Error;
use univault_engine::arz::{ArzError, DbRecord, DbValues};
use univault_engine::ids::{RecordId, normalize};

use crate::gamedata::GameData;
use crate::gdc::PlayerFile;

/// Record naming the mastery trees and the base attributes.
pub const PLAYER_RECORD: &str = "records/creatures/pc/malepc01.dbr";
/// Record naming what a point buys and the points per level.
pub const LEVELS_RECORD: &str = "records/creatures/pc/playerlevels.dbr";

/// The three attributes, in the game's own UI names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attribute {
    Physique,
    Cunning,
    Spirit,
}

impl Attribute {
    /// The attributes in the order block 2 stores them.
    pub const ALL: [Self; 3] = [Self::Physique, Self::Cunning, Self::Spirit];
}

impl fmt::Display for Attribute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Physique => "physique",
            Self::Cunning => "cunning",
            Self::Spirit => "spirit",
        })
    }
}

/// One value per attribute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerAttribute<T> {
    pub physique: T,
    pub cunning: T,
    pub spirit: T,
}

impl<T> PerAttribute<T> {
    #[must_use]
    pub fn get(&self, attribute: Attribute) -> &T {
        match attribute {
            Attribute::Physique => &self.physique,
            Attribute::Cunning => &self.cunning,
            Attribute::Spirit => &self.spirit,
        }
    }

    /// Builds one value per attribute, stopping at the first error.
    ///
    /// # Errors
    /// Whatever `make` returns.
    pub fn try_build<E>(mut make: impl FnMut(Attribute) -> Result<T, E>) -> Result<Self, E> {
        Ok(Self {
            physique: make(Attribute::Physique)?,
            cunning: make(Attribute::Cunning)?,
            spirit: make(Attribute::Spirit)?,
        })
    }
}

impl PerAttribute<u32> {
    #[must_use]
    pub fn total(&self) -> u32 {
        self.physique + self.cunning + self.spirit
    }
}

/// Position of a mastery in the player record's `skillTree<N>` list —
/// the game's own numbering, which also spells the header class tag
/// (`tagSkillClassName0306` is masteries 3 and 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MasteryIndex(u32);

impl MasteryIndex {
    #[must_use]
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for MasteryIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One mastery: its tree record, its name, and which skill records
/// belong to it. Membership is compared by normalized record path,
/// so the save's spelling need not match the database's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasteryTree {
    pub index: MasteryIndex,
    pub record: RecordId,
    /// The localized class name when the text tables have it, else
    /// the tree record's file stem.
    pub name: String,
    members: BTreeSet<String>,
    granted: BTreeSet<String>,
}

impl MasteryTree {
    /// Whether the skill record belongs to this mastery, as a tree
    /// member or as a skill one of the members grants.
    #[must_use]
    pub fn contains(&self, skill: &str) -> bool {
        let key = normalize(skill);
        self.members.contains(&key) || self.granted.contains(&key)
    }

    /// Whether the skill is one the mastery hands out for free: it
    /// leaves with the mastery but returns no points.
    #[must_use]
    pub fn grants(&self, skill: &str) -> bool {
        self.granted.contains(&normalize(skill))
    }

    #[must_use]
    pub fn member_count(&self) -> usize {
        self.members.len()
    }
}

/// The per-point and per-level rules of [`LEVELS_RECORD`].
#[derive(Clone, Debug, PartialEq)]
pub struct LevelRules {
    /// What one point adds to the attribute.
    pub attribute_increment: PerAttribute<f32>,
    /// What one point in the attribute adds to the health pool.
    pub health_increment: PerAttribute<f32>,
    /// What one spirit point adds to the energy pool.
    pub energy_increment: f32,
    /// Attribute points granted per level (`characterModifierPoints`).
    pub attribute_points_per_level: u32,
    /// Skill points granted on reaching level `i + 2`
    /// (`skillModifierPoints`).
    pub skill_points_by_level: Vec<u32>,
    /// `maxDevotionPoints`.
    pub max_devotion_points: u32,
}

/// What a fresh character starts with, and the masteries it may take,
/// from [`PLAYER_RECORD`].
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerBase {
    pub attributes: PerAttribute<f32>,
    pub health: f32,
    pub energy: f32,
    /// In `skillTree<N>` order.
    pub masteries: Vec<MasteryTree>,
}

/// Why the rules could not be read from the database.
#[derive(Debug, Error)]
pub enum RulesError {
    #[error("{} is not in the record database", record.as_str())]
    MissingRecord { record: RecordId },
    #[error("{}: {source}", record.as_str())]
    Unreadable { record: RecordId, source: ArzError },
    #[error("{} has no usable {variable}", record.as_str())]
    MissingVariable { record: RecordId, variable: String },
    #[error("{} names no mastery tree", record.as_str())]
    NoMasteries { record: RecordId },
}

/// Both records, read once per reset.
#[derive(Clone, Debug, PartialEq)]
pub struct RespecRules {
    pub levels: LevelRules,
    pub base: PlayerBase,
}

impl RespecRules {
    /// Reads both records from the topmost layer that has them.
    ///
    /// # Errors
    /// [`RulesError`] when a record, a variable, or every mastery tree
    /// is missing; nothing is guessed.
    pub fn load(game: &GameData) -> Result<Self, RulesError> {
        let levels = record(game, &record_id(LEVELS_RECORD))?;
        let player = record(game, &record_id(PLAYER_RECORD))?;
        Ok(Self {
            levels: LevelRules {
                attribute_increment: PerAttribute::try_build(|attribute| {
                    number(&levels, increment_variable(attribute))
                })?,
                health_increment: PerAttribute::try_build(|attribute| {
                    number(&levels, health_increment_variable(attribute))
                })?,
                energy_increment: number(&levels, "manaIncrement")?,
                attribute_points_per_level: count(&levels, "characterModifierPoints")?,
                skill_points_by_level: counts(&levels, "skillModifierPoints")?,
                max_devotion_points: count(&levels, "maxDevotionPoints")?,
            },
            base: PlayerBase {
                attributes: PerAttribute::try_build(|attribute| {
                    number(&player, base_variable(attribute))
                })?,
                health: number(&player, "characterLife")?,
                energy: number(&player, "characterMana")?,
                masteries: mastery_trees(game, &player)?,
            },
        })
    }
}

fn increment_variable(attribute: Attribute) -> &'static str {
    match attribute {
        Attribute::Physique => "strengthIncrement",
        Attribute::Cunning => "dexterityIncrement",
        Attribute::Spirit => "intelligenceIncrement",
    }
}

fn health_increment_variable(attribute: Attribute) -> &'static str {
    match attribute {
        Attribute::Physique => "lifeIncrement",
        Attribute::Cunning => "lifeIncrementDexterity",
        Attribute::Spirit => "lifeIncrementIntelligence",
    }
}

fn base_variable(attribute: Attribute) -> &'static str {
    match attribute {
        Attribute::Physique => "characterStrength",
        Attribute::Cunning => "characterDexterity",
        Attribute::Spirit => "characterIntelligence",
    }
}

fn record_id(path: &str) -> RecordId {
    RecordId::parse(path.to_string()).expect("a non-empty path literal parses")
}

fn record(game: &GameData, id: &RecordId) -> Result<DbRecord, RulesError> {
    match game.record(id) {
        None => Err(RulesError::MissingRecord { record: id.clone() }),
        Some(Err(source)) => Err(RulesError::Unreadable {
            record: id.clone(),
            source,
        }),
        Some(Ok(record)) => Ok(record),
    }
}

fn missing(record: &DbRecord, variable: &str) -> RulesError {
    RulesError::MissingVariable {
        record: record.id.clone(),
        variable: variable.to_string(),
    }
}

/// A scalar the game stores as a float or an integer.
#[expect(
    clippy::cast_precision_loss,
    reason = "record integers are small game constants"
)]
fn number(record: &DbRecord, variable: &str) -> Result<f32, RulesError> {
    match record.variable(variable).map(|found| &found.values) {
        Some(DbValues::Floats(values)) => values.first().copied(),
        Some(DbValues::Integers(values)) => values.first().map(|value| *value as f32),
        Some(DbValues::Strings(_) | DbValues::Booleans(_)) | None => None,
    }
    .ok_or_else(|| missing(record, variable))
}

fn counts(record: &DbRecord, variable: &str) -> Result<Vec<u32>, RulesError> {
    match record.variable(variable).map(|found| &found.values) {
        Some(DbValues::Integers(values)) => values
            .iter()
            .map(|value| u32::try_from(*value).ok())
            .collect::<Option<Vec<u32>>>(),
        Some(DbValues::Floats(_) | DbValues::Strings(_) | DbValues::Booleans(_)) | None => None,
    }
    .filter(|values| !values.is_empty())
    .ok_or_else(|| missing(record, variable))
}

fn count(record: &DbRecord, variable: &str) -> Result<u32, RulesError> {
    Ok(counts(record, variable)?[0])
}

fn strings<'a>(record: &'a DbRecord, variable: &str) -> &'a [String] {
    match record.variable(variable).map(|found| &found.values) {
        Some(DbValues::Strings(values)) => values,
        Some(DbValues::Integers(_) | DbValues::Floats(_) | DbValues::Booleans(_)) | None => &[],
    }
}

/// `skillTree<N>` variables in index order; a mastery whose tree
/// record is missing is a database error, not a mastery to skip.
fn mastery_trees(game: &GameData, player: &DbRecord) -> Result<Vec<MasteryTree>, RulesError> {
    let mut indexed: Vec<(MasteryIndex, RecordId)> = player
        .variables()
        .filter_map(|variable| {
            let index = variable.name.strip_prefix("skillTree")?.parse().ok()?;
            let path = strings(player, &variable.name).first()?;
            Some((MasteryIndex::new(index), RecordId::parse(path.clone())?))
        })
        .collect();
    indexed.sort_by_key(|(index, _)| *index);
    if indexed.is_empty() {
        return Err(RulesError::NoMasteries {
            record: player.id.clone(),
        });
    }
    indexed
        .into_iter()
        .map(|(index, tree_id)| mastery_tree(game, index, tree_id))
        .collect()
}

fn mastery_tree(
    game: &GameData,
    index: MasteryIndex,
    tree_id: RecordId,
) -> Result<MasteryTree, RulesError> {
    let tree = record(game, &tree_id)?;
    let mut members = BTreeSet::new();
    let mut granted = BTreeSet::new();
    for variable in tree.variables() {
        if !variable.name.starts_with("skillName") {
            continue;
        }
        for member in strings(&tree, &variable.name) {
            members.insert(normalize(member));
            if let Some(Ok(skill)) = RecordId::parse(member.clone()).and_then(|id| game.record(&id))
            {
                granted.extend(
                    strings(&skill, "grantedSkills")
                        .iter()
                        .map(|s| normalize(s)),
                );
            }
        }
    }
    let name = game
        .tag_text(&format!("tagSkillClassName{:02}", index.value()))
        .map_or_else(|| tree_id.file_stem().to_string(), str::to_string);
    Ok(MasteryTree {
        index,
        record: tree_id,
        name,
        members,
        granted,
    })
}

/// What a reset returned to block 2's attribute pool, per attribute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttributeReset {
    pub refunded: PerAttribute<u32>,
}

impl AttributeReset {
    #[must_use]
    pub fn total(&self) -> u32 {
        self.refunded.total()
    }

    /// Nothing was spent, so nothing changed.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.total() == 0
    }
}

impl fmt::Display for AttributeReset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_noop() {
            return f.write_str("no attribute points were spent");
        }
        let spent: Vec<String> = Attribute::ALL
            .into_iter()
            .map(|attribute| (attribute, *self.refunded.get(attribute)))
            .filter(|(_, points)| *points > 0)
            .map(|(attribute, points)| format!("{points} {attribute}"))
            .collect();
        write!(
            f,
            "{} attribute points returned ({})",
            self.total(),
            spent.join(", ")
        )
    }
}

/// Why the attributes could not be reset.
#[derive(Debug, Error, PartialEq)]
pub enum AttributeError {
    #[error("the character's bio (block 2) is not typed")]
    NoBio,
    /// The attribute is not the base plus a whole number of points.
    #[error("{attribute} is {value}, not the base {base} plus whole {increment}-point steps")]
    NotWholePoints {
        attribute: Attribute,
        value: f32,
        base: f32,
        increment: f32,
    },
    /// The health pool is not what the spent points account for.
    #[error("health is {found}, not the {expected} the spent attribute points account for")]
    HealthNotDerived { expected: f32, found: f32 },
    /// The energy pool is not what the spent points account for.
    #[error("energy is {found}, not the {expected} the spent attribute points account for")]
    EnergyNotDerived { expected: f32, found: f32 },
}

/// Returns every spent attribute point to the pool and puts the
/// attributes, health, and energy back to the base. A character with
/// nothing spent is untouched.
///
/// # Errors
/// [`AttributeError`] — the file is untouched.
pub fn reset_attributes(
    file: &mut PlayerFile,
    rules: &RespecRules,
) -> Result<AttributeReset, AttributeError> {
    let bio = file.bio().ok_or(AttributeError::NoBio)?;
    let current = PerAttribute {
        physique: bio.physique,
        cunning: bio.cunning,
        spirit: bio.spirit,
    };
    let refunded = PerAttribute::try_build(|attribute| {
        let value = *current.get(attribute);
        let base = *rules.base.attributes.get(attribute);
        let increment = *rules.levels.attribute_increment.get(attribute);
        points_spent(value, base, increment).ok_or(AttributeError::NotWholePoints {
            attribute,
            value,
            base,
            increment,
        })
    })?;
    let expected_health =
        rules.base.health + pool_bought(&refunded, &rules.levels.health_increment);
    if !same(expected_health, bio.health) {
        return Err(AttributeError::HealthNotDerived {
            expected: expected_health,
            found: bio.health,
        });
    }
    let expected_energy =
        rules.base.energy + points_as_f32(refunded.spirit) * rules.levels.energy_increment;
    if !same(expected_energy, bio.energy) {
        return Err(AttributeError::EnergyNotDerived {
            expected: expected_energy,
            found: bio.energy,
        });
    }
    let reset = AttributeReset { refunded };
    if reset.is_noop() {
        return Ok(reset);
    }
    let bio = file.bio_mut().ok_or(AttributeError::NoBio)?;
    bio.physique = rules.base.attributes.physique;
    bio.cunning = rules.base.attributes.cunning;
    bio.spirit = rules.base.attributes.spirit;
    bio.health = rules.base.health;
    bio.energy = rules.base.energy;
    bio.attribute_points_unspent += reset.total();
    Ok(reset)
}

/// The points that took `value` from `base` in `increment` steps;
/// `None` unless that is a whole, non-negative number.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is checked to be a whole number within u32 first"
)]
fn points_spent(value: f32, base: f32, increment: f32) -> Option<u32> {
    if increment <= 0.0 {
        return None;
    }
    let steps = (value - base) / increment;
    (steps >= 0.0 && steps.fract() == 0.0 && steps <= points_as_f32(u32::MAX))
        .then_some(steps as u32)
}

fn pool_bought(points: &PerAttribute<u32>, increment: &PerAttribute<f32>) -> f32 {
    Attribute::ALL
        .into_iter()
        .map(|attribute| points_as_f32(*points.get(attribute)) * *increment.get(attribute))
        .sum()
}

#[expect(
    clippy::cast_precision_loss,
    reason = "attribute points are a few hundred at most"
)]
fn points_as_f32(points: u32) -> f32 {
    points as f32
}

/// Exact equality: the pools are sums of small whole numbers, so any
/// difference is a rule this crate does not know, not rounding.
#[expect(
    clippy::float_cmp,
    reason = "exactness is the point; see the doc comment"
)]
fn same(expected: f32, found: f32) -> bool {
    expected == found
}

/// One mastery a reset removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovedMastery {
    pub index: MasteryIndex,
    pub name: String,
    pub skills_removed: usize,
    pub points_refunded: u32,
}

/// What a mastery reset removed and returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MasteryReset {
    /// The masteries that had skills, in `skillTree<N>` order.
    pub masteries: Vec<RemovedMastery>,
    /// Whether the header's class tag was non-empty and was cleared.
    pub class_tag_cleared: bool,
}

impl MasteryReset {
    #[must_use]
    pub fn points_refunded(&self) -> u32 {
        self.masteries.iter().map(|m| m.points_refunded).sum()
    }

    #[must_use]
    pub fn skills_removed(&self) -> usize {
        self.masteries.iter().map(|m| m.skills_removed).sum()
    }

    /// No mastery skill was present, so nothing changed.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.masteries.is_empty()
    }
}

impl fmt::Display for MasteryReset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_noop() {
            return f.write_str("no mastery was chosen");
        }
        let removed: Vec<String> = self
            .masteries
            .iter()
            .map(|m| format!("{} ({} skills)", m.name, m.skills_removed))
            .collect();
        write!(
            f,
            "removed {}; {} skill points returned",
            removed.join(" and "),
            self.points_refunded()
        )
    }
}

/// Why the masteries could not be reset.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MasteryError {
    #[error("the character's bio (block 2) is not typed")]
    NoBio,
    #[error("the character's skills (block 8) are not typed")]
    NoSkills,
}

/// Removes every skill of every mastery from block 8, refunds the
/// levels of the ones the mastery did not grant to block 2's skill
/// points, and clears the header's class tag, which the game derives
/// from the chosen masteries. Devotions, item-granted skills, the
/// default skills, the potion modifiers, the reclamation counters,
/// and `masteries_allowed` — the level gate on how many masteries may
/// be chosen, which is what lets both be chosen again — are left as
/// they are. A character with no mastery skill is untouched.
///
/// # Errors
/// [`MasteryError`] — the file is untouched.
pub fn reset_masteries(
    file: &mut PlayerFile,
    rules: &RespecRules,
) -> Result<MasteryReset, MasteryError> {
    file.bio().ok_or(MasteryError::NoBio)?;
    let skills = file.skills().ok_or(MasteryError::NoSkills)?;
    let masteries: Vec<RemovedMastery> = rules
        .base
        .masteries
        .iter()
        .filter_map(|tree| {
            let owned = skills
                .skills
                .iter()
                .filter(|skill| tree.contains(&skill.name));
            let skills_removed = owned.clone().count();
            (skills_removed > 0).then(|| RemovedMastery {
                index: tree.index,
                name: tree.name.clone(),
                skills_removed,
                points_refunded: owned
                    .filter(|skill| !tree.grants(&skill.name))
                    .map(|skill| skill.level)
                    .sum(),
            })
        })
        .collect();
    if masteries.is_empty() {
        return Ok(MasteryReset::default());
    }
    let reset = MasteryReset {
        masteries,
        class_tag_cleared: !file.header().class_tag.is_empty(),
    };
    let trees = &rules.base.masteries;
    file.skills_mut()
        .ok_or(MasteryError::NoSkills)?
        .skills
        .retain(|skill| !trees.iter().any(|tree| tree.contains(&skill.name)));
    file.bio_mut()
        .ok_or(MasteryError::NoBio)?
        .skill_points_unspent += reset.points_refunded();
    file.header_mut().class_tag.clear();
    Ok(reset)
}

/// Which reset to apply — the shell's and the CLI's one choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reset {
    Attributes,
    Masteries,
}

impl Reset {
    /// Applies the reset.
    ///
    /// # Errors
    /// [`ResetError`] — the file is untouched.
    pub fn apply(self, file: &mut PlayerFile, rules: &RespecRules) -> Result<Report, ResetError> {
        Ok(match self {
            Self::Attributes => Report::Attributes(reset_attributes(file, rules)?),
            Self::Masteries => Report::Masteries(reset_masteries(file, rules)?),
        })
    }
}

impl fmt::Display for Reset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Attributes => "attributes",
            Self::Masteries => "masteries",
        })
    }
}

/// What a [`Reset`] did.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    Attributes(AttributeReset),
    Masteries(MasteryReset),
}

impl Report {
    /// Nothing was there to reset, so the file is unchanged.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        match self {
            Self::Attributes(reset) => reset.is_noop(),
            Self::Masteries(reset) => reset.is_noop(),
        }
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attributes(reset) => reset.fmt(f),
            Self::Masteries(reset) => reset.fmt(f),
        }
    }
}

/// Why a [`Reset`] could not be applied.
#[derive(Debug, Error)]
pub enum ResetError {
    #[error(transparent)]
    Attributes(#[from] AttributeError),
    #[error(transparent)]
    Masteries(#[from] MasteryError),
}

#[cfg(test)]
pub(crate) mod testing {
    //! A database carrying the verified real values (see the module
    //! docs) with two small mastery trees, for tests that need rules
    //! without an install.

    use univault_engine::arz::ArzDialect;
    use univault_engine::arz::ArzFile;
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::text::TextDb;

    use super::{LEVELS_RECORD, PLAYER_RECORD};
    use crate::gamedata::GameData;

    pub(crate) const TREE_1: &str = "records/skills/playerclass01/_classtree_class01.dbr";
    pub(crate) const TREE_2: &str = "records/skills/playerclass02/_classtree_class02.dbr";
    pub(crate) const MASTERY_1: &str = "records/skills/playerclass01/_classtraining_class01.dbr";
    pub(crate) const MASTERY_2: &str = "records/skills/playerclass02/_classtraining_class02.dbr";
    pub(crate) const SHAPESHIFT: &str = "records/skills/playerclass01/werewolf1.dbr";
    pub(crate) const GRANTED_CLAWS: &str =
        "records/skills/playerclass01/werewolf1_skill01_claws.dbr";
    pub(crate) const GRANTED_CHARGE: &str =
        "records/skills/playerclass01/werewolf1_skill02_charge.dbr";
    pub(crate) const PASSIVE_1: &str = "records/skills/playerclass01/passive01.dbr";
    pub(crate) const SKILL_2: &str = "records/skills/playerclass02/blitz1.dbr";

    /// The rules as a database: the base game's numbers, `skillTree1`
    /// with a shapeshift that grants two skills, and `skillTree2`.
    /// `trees` replaces the two trees' member lists when given.
    pub(crate) fn game_data(trees: Option<&[(&str, &[&str])]>) -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            LEVELS_RECORD,
            "",
            &[
                ("strengthIncrement", Values::Ints(&[8])),
                ("dexterityIncrement", Values::Ints(&[8])),
                ("intelligenceIncrement", Values::Ints(&[8])),
                ("lifeIncrement", Values::Ints(&[20])),
                ("lifeIncrementDexterity", Values::Ints(&[8])),
                ("lifeIncrementIntelligence", Values::Ints(&[12])),
                ("manaIncrement", Values::Ints(&[16])),
                ("characterModifierPoints", Values::Ints(&[1])),
                ("skillModifierPoints", Values::Ints(&[3, 3, 2, 1])),
                ("maxDevotionPoints", Values::Ints(&[55])),
            ],
        );
        builder.record(
            PLAYER_RECORD,
            "Player",
            &[
                ("characterStrength", Values::Floats(&[50.0])),
                ("characterDexterity", Values::Floats(&[50.0])),
                ("characterIntelligence", Values::Floats(&[50.0])),
                ("characterLife", Values::Floats(&[250.0])),
                ("characterMana", Values::Floats(&[250.0])),
                ("skillTree2", Values::Strings(&[TREE_2])),
                ("skillTree1", Values::Strings(&[TREE_1])),
            ],
        );
        let default_trees: [(&str, &[&str]); 2] = [
            (
                TREE_1,
                &[
                    MASTERY_1,
                    SHAPESHIFT,
                    GRANTED_CLAWS,
                    GRANTED_CHARGE,
                    PASSIVE_1,
                ],
            ),
            (TREE_2, &[MASTERY_2, SKILL_2]),
        ];
        for (tree, members) in trees.unwrap_or(&default_trees) {
            let variables: Vec<(String, &str)> = members
                .iter()
                .enumerate()
                .map(|(slot, member)| (format!("skillName{}", slot + 1), *member))
                .collect();
            let variables: Vec<(&str, Values<'_>)> = variables
                .iter()
                .map(|(name, member)| {
                    (name.as_str(), Values::Strings(std::slice::from_ref(member)))
                })
                .collect();
            builder.record(tree, "SkillTree", &variables);
        }
        builder.record(
            SHAPESHIFT,
            "Skill_Shapeshift",
            &[(
                "grantedSkills",
                Values::Strings(&[GRANTED_CLAWS, GRANTED_CHARGE]),
            )],
        );
        let database = ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap();
        let mut text = TextDb::new();
        text.add_file(b"tagSkillClassName01=Soldier\ntagSkillClassName02=Demolitionist\n");
        GameData::from_parts(vec![database], text, vec![])
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::float_cmp,
        reason = "the pools are exact sums of small whole numbers; any drift is a wrong rule"
    )]

    use super::testing::*;
    use super::*;
    use crate::blocks::bio::Bio;
    use crate::blocks::skills::{Skill, Skills, SkillsExtension};
    use crate::gdc::{Block, PlayerHeader, Sex};

    fn skill(name: &str, level: u32) -> Skill {
        Skill {
            name: name.into(),
            level,
            enabled: 1,
            ..Skill::default()
        }
    }

    fn bio() -> Bio {
        Bio {
            level: 20,
            experience: 100_000,
            attribute_points_unspent: 2,
            skill_points_unspent: 3,
            devotion_points_unspent: 0,
            total_devotion_unlocked: 4,
            physique: 50.0 + 8.0 * 10.0,
            cunning: 50.0 + 8.0 * 2.0,
            spirit: 50.0 + 8.0 * 5.0,
            health: 250.0 + 20.0 * 10.0 + 8.0 * 2.0 + 12.0 * 5.0,
            energy: 250.0 + 16.0 * 5.0,
        }
    }

    fn player(class_tag: &str, skills: Vec<Skill>) -> PlayerFile {
        PlayerFile::from_parts(
            0x0BAD_F00D,
            PlayerHeader {
                name: "Sif".into(),
                sex: Sex::Female,
                class_tag: class_tag.into(),
                level: 20,
                hardcore: false,
                expansion_status: 7,
                data_version: 8,
                uid: [0; 16],
            },
            vec![
                Block::Bio(bio()),
                Block::Skills(Skills {
                    skills,
                    masteries_allowed: 2,
                    skill_reclamation_points_used: 7,
                    devotion_reclamation_points_used: 0,
                    item_skills: vec![],
                    extension: SkillsExtension::V8 { sub_skills: vec![] },
                }),
            ],
        )
    }

    fn two_masteries() -> Vec<Skill> {
        vec![
            skill("records/skills/default/defaultweaponattack.dbr", 1),
            skill(MASTERY_1, 10),
            skill(SHAPESHIFT, 4),
            skill(GRANTED_CLAWS, 4),
            skill(GRANTED_CHARGE, 4),
            skill(PASSIVE_1, 3),
            skill("Records/Skills/PlayerClass02/_classtraining_class02.dbr", 6),
            skill(SKILL_2, 2),
            skill("records/skills/devotion/tier1_01.dbr", 1),
            skill(
                "records/skills/itemskillsgdx3/potionmodifiers/healthpotion_a.dbr",
                1,
            ),
        ]
    }

    #[test]
    fn rules_read_both_records_and_the_trees_in_index_order() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        assert_eq!(
            rules.levels.attribute_increment,
            PerAttribute {
                physique: 8.0,
                cunning: 8.0,
                spirit: 8.0
            }
        );
        assert_eq!(
            rules.levels.health_increment,
            PerAttribute {
                physique: 20.0,
                cunning: 8.0,
                spirit: 12.0
            }
        );
        assert_eq!(rules.levels.energy_increment, 16.0);
        assert_eq!(rules.levels.attribute_points_per_level, 1);
        assert_eq!(rules.levels.skill_points_by_level, vec![3, 3, 2, 1]);
        assert_eq!(rules.levels.max_devotion_points, 55);
        assert_eq!(rules.base.health, 250.0);
        assert_eq!(rules.base.energy, 250.0);
        assert_eq!(rules.base.attributes.spirit, 50.0);
        let names: Vec<(u32, &str)> = rules
            .base
            .masteries
            .iter()
            .map(|tree| (tree.index.value(), tree.name.as_str()))
            .collect();
        assert_eq!(names, vec![(1, "Soldier"), (2, "Demolitionist")]);
        let soldier = &rules.base.masteries[0];
        assert_eq!(soldier.member_count(), 5);
        assert!(soldier.contains("RECORDS\\SKILLS\\PLAYERCLASS01\\PASSIVE01.DBR"));
        assert!(soldier.grants(GRANTED_CLAWS));
        assert!(!soldier.grants(SHAPESHIFT));
        assert!(!soldier.contains(SKILL_2));
    }

    #[test]
    fn a_missing_record_or_variable_is_refused() {
        let empty = GameData::from_parts(vec![], univault_engine::text::TextDb::new(), vec![]);
        assert!(matches!(
            RespecRules::load(&empty).unwrap_err(),
            RulesError::MissingRecord { record } if record.as_str() == LEVELS_RECORD
        ));
        let no_tree_records = game_data(Some(&[]));
        assert!(matches!(
            RespecRules::load(&no_tree_records).unwrap_err(),
            RulesError::MissingRecord { record } if record.as_str() == TREE_1
        ));
    }

    #[test]
    fn attributes_return_to_base_and_the_points_to_the_pool() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let mut file = player("tagSkillClassName0102", two_masteries());
        let reset = reset_attributes(&mut file, &rules).unwrap();
        assert_eq!(
            reset.refunded,
            PerAttribute {
                physique: 10,
                cunning: 2,
                spirit: 5
            }
        );
        assert_eq!(
            reset.to_string(),
            "17 attribute points returned (10 physique, 2 cunning, 5 spirit)"
        );
        let bio = file.bio().unwrap();
        assert_eq!((bio.physique, bio.cunning, bio.spirit), (50.0, 50.0, 50.0));
        assert_eq!((bio.health, bio.energy), (250.0, 250.0));
        assert_eq!(bio.attribute_points_unspent, 19);
        assert_eq!(bio.skill_points_unspent, 3);

        let again = file.clone();
        let reset = reset_attributes(&mut file, &rules).unwrap();
        assert!(reset.is_noop());
        assert_eq!(reset.to_string(), "no attribute points were spent");
        assert_eq!(file, again);
    }

    #[test]
    fn attributes_off_the_point_grid_are_refused_untouched() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let mut file = player("", vec![]);
        file.bio_mut().unwrap().cunning = 67.0;
        let before = file.clone();
        assert_eq!(
            reset_attributes(&mut file, &rules),
            Err(AttributeError::NotWholePoints {
                attribute: Attribute::Cunning,
                value: 67.0,
                base: 50.0,
                increment: 8.0
            })
        );
        assert_eq!(file, before);

        let mut file = player("", vec![]);
        file.bio_mut().unwrap().health += 1.0;
        assert!(matches!(
            reset_attributes(&mut file, &rules),
            Err(AttributeError::HealthNotDerived { expected, found })
                if expected == 526.0 && found == 527.0
        ));

        let mut file = player("", vec![]);
        file.bio_mut().unwrap().energy = 250.0;
        assert!(matches!(
            reset_attributes(&mut file, &rules),
            Err(AttributeError::EnergyNotDerived { expected, .. }) if expected == 330.0
        ));
    }

    #[test]
    fn masteries_leave_with_their_skills_and_refund_the_ungranted_levels() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let mut file = player("tagSkillClassName0102", two_masteries());
        let reset = reset_masteries(&mut file, &rules).unwrap();
        assert_eq!(
            reset.masteries,
            vec![
                RemovedMastery {
                    index: MasteryIndex::new(1),
                    name: "Soldier".into(),
                    skills_removed: 5,
                    points_refunded: 17,
                },
                RemovedMastery {
                    index: MasteryIndex::new(2),
                    name: "Demolitionist".into(),
                    skills_removed: 2,
                    points_refunded: 8,
                },
            ]
        );
        assert!(reset.class_tag_cleared);
        assert_eq!(reset.points_refunded(), 25);
        assert_eq!(reset.skills_removed(), 7);
        assert_eq!(
            reset.to_string(),
            "removed Soldier (5 skills) and Demolitionist (2 skills); 25 skill points returned"
        );
        let remaining: Vec<&str> = file
            .skills()
            .unwrap()
            .skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect();
        assert_eq!(
            remaining,
            vec![
                "records/skills/default/defaultweaponattack.dbr",
                "records/skills/devotion/tier1_01.dbr",
                "records/skills/itemskillsgdx3/potionmodifiers/healthpotion_a.dbr",
            ]
        );
        let skills = file.skills().unwrap();
        assert_eq!(skills.masteries_allowed, 2);
        assert_eq!(skills.skill_reclamation_points_used, 7);
        assert_eq!(file.bio().unwrap().skill_points_unspent, 28);
        assert_eq!(file.bio().unwrap().attribute_points_unspent, 2);
        assert_eq!(file.header().class_tag, "");

        let again = file.clone();
        let reset = reset_masteries(&mut file, &rules).unwrap();
        assert!(reset.is_noop());
        assert!(!reset.class_tag_cleared);
        assert_eq!(reset.to_string(), "no mastery was chosen");
        assert_eq!(file, again);
    }

    #[test]
    fn a_character_without_a_mastery_is_untouched() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let mut file = player(
            "tagSkillClassName01",
            vec![skill("records/skills/devotion/tier1_01.dbr", 1)],
        );
        let before = file.clone();
        assert_eq!(
            reset_masteries(&mut file, &rules).unwrap(),
            MasteryReset::default()
        );
        assert_eq!(file, before);
    }

    #[test]
    fn untyped_blocks_are_refused() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let header = player("", vec![]).header().clone();
        let mut no_blocks = PlayerFile::from_parts(1, header, vec![]);
        assert_eq!(
            reset_attributes(&mut no_blocks, &rules),
            Err(AttributeError::NoBio)
        );
        assert_eq!(
            reset_masteries(&mut no_blocks, &rules),
            Err(MasteryError::NoBio)
        );
        let header = no_blocks.header().clone();
        let mut bio_only = PlayerFile::from_parts(1, header, vec![Block::Bio(bio())]);
        assert_eq!(
            reset_masteries(&mut bio_only, &rules),
            Err(MasteryError::NoSkills)
        );
    }

    #[test]
    fn a_reset_applies_by_kind_and_reports_a_noop() {
        let rules = RespecRules::load(&game_data(None)).unwrap();
        let mut file = player("tagSkillClassName0102", two_masteries());
        let report = Reset::Masteries.apply(&mut file, &rules).unwrap();
        assert!(matches!(&report, Report::Masteries(reset) if reset.points_refunded() == 25));
        assert!(!report.is_noop());
        let report = Reset::Attributes.apply(&mut file, &rules).unwrap();
        assert!(matches!(&report, Report::Attributes(reset) if reset.total() == 17));
        assert!(
            Reset::Attributes
                .apply(&mut file, &rules)
                .unwrap()
                .is_noop()
        );
        assert_eq!(Reset::Attributes.to_string(), "attributes");
        assert_eq!(Reset::Masteries.to_string(), "masteries");
    }
}
