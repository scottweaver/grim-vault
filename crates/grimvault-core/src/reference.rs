//! Reference cards: the game facts a player looks up while sorting
//! loot, built from the record database and shown beside the items.
//!
//! The first card is the affix table. Every named prefix and suffix —
//! the `LootRandomizer` records under `records/items/lootaffixes/
//! prefix/` and `…/suffix/`; the ascendant, completion-bonus, unique
//! and crafting folders carry no `lootRandomizerName` and stay out —
//! becomes one entry per name, position and rarity. A name covers
//! several records: level tiers of the same stats at rising values,
//! item-type forms (a prefix that rolls one way on armor and another
//! on shields), or one record per skill for the mastery prefixes. The
//! entry keeps every record as a [`Tier`] and collapses them into
//! [`Grant`]s: one line per stat with each varying number written as
//! its range across the tiers, and a [`Coverage`] that says when only
//! some tiers grant it, so a form or a per-skill variant is never
//! shown as if every roll carried it. Values are the records'
//! nominal numbers — `lootRandomizerJitter` moves them per roll, and
//! that mapping is not public (`docs/format-references.md`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use univault_engine::ids::RecordId;

use crate::gamedata::{GameData, Rarity};
use crate::search::{number_spans, stat_template};
use crate::stats::{LineKind, RecordStats, Scale, SkillLevel, StatLine};

const AFFIX_CLASS: &str = "LootRandomizer";

/// Where an affix sits in an item's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Position {
    Prefix,
    Suffix,
}

impl Position {
    pub const ALL: [Self; 2] = [Self::Prefix, Self::Suffix];

    const fn folder(self) -> &'static str {
        match self {
            Self::Prefix => "records/items/lootaffixes/prefix/",
            Self::Suffix => "records/items/lootaffixes/suffix/",
        }
    }

    /// The position of an affix record, by the folder the game files
    /// it under — the only witness the database offers. `None` for a
    /// record outside the two named folders.
    #[must_use]
    pub fn of(record: &RecordId) -> Option<Self> {
        let path = record.as_str().to_ascii_lowercase().replace('\\', "/");
        Self::ALL
            .into_iter()
            .find(|position| path.starts_with(position.folder()))
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Prefix => "prefix",
            Self::Suffix => "suffix",
        }
    }
}

/// The level requirements an entry's tiers span, lowest to highest;
/// tiers without one (a zero or absent `levelRequirement`) do not
/// count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelRange {
    pub low: u32,
    pub high: u32,
}

impl fmt::Display for LevelRange {
    /// `5–92`, or `5` when one level covers every tier.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.low == self.high {
            write!(f, "{}", self.low)
        } else {
            write!(f, "{}–{}", self.low, self.high)
        }
    }
}

impl LevelRange {
    fn including(range: Option<Self>, level: u32) -> Self {
        range.map_or(
            Self {
                low: level,
                high: level,
            },
            |range| Self {
                low: range.low.min(level),
                high: range.high.max(level),
            },
        )
    }
}

/// One record of an affix: its level requirement and its rendered
/// stats, shared with the tooltip memo.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier {
    pub record: RecordId,
    pub level: Option<u32>,
    pub stats: Arc<RecordStats>,
}

/// How many of an entry's tiers carry a grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Every,
    Some { tiers: usize, of: usize },
}

impl Coverage {
    fn of(tiers: usize, of: usize) -> Self {
        if tiers >= of {
            Self::Every
        } else {
            Self::Some { tiers, of }
        }
    }
}

/// One stat an affix grants, as the card shows it: the tiers' wording
/// with each number that varies written as its range, and how many
/// tiers grant it.
#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub line: StatLine,
    pub coverage: Coverage,
}

/// One named affix in one position at one rarity, with everything
/// its records grant.
#[derive(Clone, Debug, PartialEq)]
pub struct AffixEntry {
    pub name: String,
    pub position: Position,
    pub rarity: Option<Rarity>,
    pub levels: Option<LevelRange>,
    pub grants: Vec<Grant>,
    /// Every record of the affix, lowest level first.
    pub tiers: Vec<Tier>,
    haystack: String,
}

impl AffixEntry {
    fn new(name: String, position: Position, rarity: Option<Rarity>, mut tiers: Vec<Tier>) -> Self {
        tiers.sort_by(|a, b| {
            a.level
                .cmp(&b.level)
                .then_with(|| a.record.as_str().cmp(b.record.as_str()))
        });
        let levels = tiers
            .iter()
            .filter_map(|tier| tier.level)
            .fold(None, |range, level| {
                Some(LevelRange::including(range, level))
            });
        let grants = collapse(&tiers);
        let haystack = std::iter::once(name.as_str())
            .chain(grants.iter().map(|grant| grant.line.text.as_str()))
            .chain(
                tiers
                    .iter()
                    .flat_map(|tier| tier.stats.lines.iter().map(|line| line.text.as_str())),
            )
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        Self {
            name,
            position,
            rarity,
            levels,
            grants,
            tiers,
            haystack,
        }
    }

    /// Whether the name, a grant, or any tier's line contains
    /// `needle_lowercase`.
    #[must_use]
    pub fn mentions(&self, needle_lowercase: &str) -> bool {
        self.haystack.contains(needle_lowercase)
    }
}

/// What the card's search bar and filters ask for; persisted as view
/// state, so every field defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AffixQuery {
    pub text: String,
    pub position: Option<Position>,
    pub rarity: Option<Rarity>,
}

impl AffixQuery {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.position.is_none() && self.rarity.is_none()
    }

    #[must_use]
    pub fn admits(&self, entry: &AffixEntry) -> bool {
        self.position
            .is_none_or(|position| position == entry.position)
            && self
                .rarity
                .is_none_or(|rarity| Some(rarity) == entry.rarity)
            && {
                let needle = self.text.trim().to_lowercase();
                needle.is_empty() || entry.mentions(&needle)
            }
    }
}

/// Every named affix the database defines, sorted by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AffixTable {
    entries: Vec<AffixEntry>,
}

impl AffixTable {
    /// Renders every named prefix and suffix once and groups the
    /// records by name, position and rarity. A record whose stats
    /// cannot be rendered, or whose name tag the text archives lack,
    /// is left out — the card shows what the game would show.
    #[must_use]
    pub fn build(game: &GameData) -> Self {
        let mut groups: BTreeMap<(String, Position, Option<Rarity>), Vec<Tier>> = BTreeMap::new();
        for (id, class) in game.record_types() {
            if class != AFFIX_CLASS {
                continue;
            }
            let Some(position) = Position::of(id) else {
                continue;
            };
            let Some(Ok(info)) = game.affix_info(id) else {
                continue;
            };
            let Some(name) = info.name else {
                continue;
            };
            let Some(stats) = game.record_stats(id, SkillLevel::ONE, Scale::NONE) else {
                continue;
            };
            let level = stats.level_requirement.filter(|level| *level > 0);
            groups
                .entry((name, position, info.rarity))
                .or_default()
                .push(Tier {
                    record: id.clone(),
                    level,
                    stats,
                });
        }
        let mut entries: Vec<AffixEntry> = groups
            .into_iter()
            .map(|((name, position, rarity), tiers)| AffixEntry::new(name, position, rarity, tiers))
            .collect();
        entries
            .sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.position, entry.rarity));
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[AffixEntry] {
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

    /// The entries `query` admits, with their positions in the table.
    pub fn matching<'a, 'q>(
        &'a self,
        query: &'q AffixQuery,
    ) -> impl Iterator<Item = (usize, &'a AffixEntry)> + use<'a, 'q> {
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, entry)| query.admits(entry))
    }
}

/// The lines of one stat shape across the tiers that carry it.
struct Shape<'a> {
    first: &'a StatLine,
    texts: Vec<&'a str>,
    tiers: BTreeSet<usize>,
}

/// Groups the tiers' lines by wording, writes each group's varying
/// numbers as ranges, and folds the per-skill variants of a kind —
/// two or more skills spread over the records — into one line; every
/// tier's grants first, the partial ones after.
fn collapse(tiers: &[Tier]) -> Vec<Grant> {
    let mut order: Vec<String> = Vec::new();
    let mut shapes: BTreeMap<String, Shape<'_>> = BTreeMap::new();
    for (index, tier) in tiers.iter().enumerate() {
        for line in &tier.stats.lines {
            let key = stat_template(&line.text);
            let shape = shapes.entry(key.clone()).or_insert_with(|| {
                order.push(key);
                Shape {
                    first: line,
                    texts: Vec::new(),
                    tiers: BTreeSet::new(),
                }
            });
            shape.texts.push(&line.text);
            shape.tiers.insert(index);
        }
    }
    let of = tiers.len();
    let mut grants: Vec<Grant> = Vec::new();
    let mut variants: BTreeMap<VariantKind, Vec<Shape<'_>>> = BTreeMap::new();
    for key in order {
        let Some(shape) = shapes.remove(&key) else {
            continue;
        };
        let coverage = Coverage::of(shape.tiers.len(), of);
        match (VariantKind::of(shape.first.kind), coverage) {
            (Some(kind), Coverage::Some { .. }) => variants.entry(kind).or_default().push(shape),
            (Some(_) | None, _) => grants.push(Grant {
                line: ranged(shape.first, &shape.texts),
                coverage,
            }),
        }
    }
    for (kind, shapes) in variants {
        match shapes.as_slice() {
            [only] => grants.push(Grant {
                line: ranged(only.first, &only.texts),
                coverage: Coverage::of(only.tiers.len(), of),
            }),
            several => grants.push(kind.fold(several, of)),
        }
    }
    grants.sort_by_key(|grant| matches!(grant.coverage, Coverage::Some { .. }));
    grants
}

/// The line kinds whose per-record variants a name may spread over —
/// one record per skill or mastery — folded into one grant when they
/// do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VariantKind {
    Skill,
    Mastery,
}

impl VariantKind {
    fn of(kind: LineKind) -> Option<Self> {
        match kind {
            LineKind::SkillBonus => Some(Self::Skill),
            LineKind::MasteryBonus => Some(Self::Mastery),
            LineKind::Effect(_)
            | LineKind::AttackSpeed
            | LineKind::BlockRecovery
            | LineKind::Conversion
            | LineKind::Racial
            | LineKind::AllSkillsBonus
            | LineKind::GrantedSkill
            | LineKind::SkillDescription
            | LineKind::SkillLevel
            | LineKind::PetBonusHeading
            | LineKind::SetName
            | LineKind::SetMember
            | LineKind::Requirement(_) => None,
        }
    }

    fn fold(self, shapes: &[Shape<'_>], of: usize) -> Grant {
        let texts: Vec<&str> = shapes
            .iter()
            .flat_map(|shape| shape.texts.iter().copied())
            .collect();
        let bonus = number_range(&texts).unwrap_or_default();
        let tiers: BTreeSet<usize> = shapes
            .iter()
            .flat_map(|shape| shape.tiers.iter().copied())
            .collect();
        let text = match self {
            Self::Skill => format!("+{bonus} to one of {} skills", shapes.len()),
            Self::Mastery => format!(
                "+{bonus} to all skills in one of {} masteries",
                shapes.len()
            ),
        };
        let first = shapes[0].first;
        Grant {
            line: StatLine {
                kind: first.kind,
                section: first.section,
                emphasis: first.emphasis,
                text,
                values: Vec::new(),
            },
            coverage: Coverage::of(tiers.len(), of),
        }
    }
}

/// The line as one grant: `first`'s kind, section and emphasis over
/// the ranged wording of `texts`.
fn ranged(first: &StatLine, texts: &[&str]) -> StatLine {
    StatLine {
        kind: first.kind,
        section: first.section,
        emphasis: first.emphasis,
        text: range_text(texts),
        values: Vec::new(),
    }
}

/// A run of numbers joined by `-` — a damage range — read as one
/// unit, as byte spans into a text.
#[derive(Clone, Copy)]
struct Unit {
    start: usize,
    end: usize,
}

fn units(text: &str) -> Vec<Unit> {
    let mut units: Vec<Unit> = Vec::new();
    for (start, end) in number_spans(text) {
        match units.last_mut() {
            Some(last) if &text[last.end..start] == "-" => last.end = end,
            Some(_) | None => units.push(Unit { start, end }),
        }
    }
    units
}

/// The numbers of one unit, for ordering its occurrences.
fn unit_values(text: &str) -> Vec<f32> {
    number_spans(text)
        .into_iter()
        .filter_map(|(start, end)| text[start..end].parse().ok())
        .collect()
}

fn ordered_by_value(a: &str, b: &str) -> std::cmp::Ordering {
    unit_values(a)
        .into_iter()
        .zip(unit_values(b))
        .map(|(x, y)| x.total_cmp(&y))
        .find(|ordering| ordering.is_ne())
        .unwrap_or_else(|| a.cmp(b))
}

/// The lowest and highest occurrence of one unit across the texts,
/// written as the game wrote them: one number when they agree, `lo–hi`
/// for a single number, `lo to hi` for a damage range.
fn unit_range(occurrences: &[&str]) -> Option<String> {
    let lo = occurrences
        .iter()
        .copied()
        .min_by(|a, b| ordered_by_value(a, b))?;
    let hi = occurrences
        .iter()
        .copied()
        .max_by(|a, b| ordered_by_value(a, b))?;
    Some(if lo == hi {
        lo.to_string()
    } else if lo.contains('-') && number_spans(lo).len() > 1 {
        format!("{lo} to {hi}")
    } else {
        format!("{lo}–{hi}")
    })
}

/// The first unit's range across the texts — the bonus of a skill or
/// mastery line.
fn number_range(texts: &[&str]) -> Option<String> {
    let occurrences: Vec<&str> = texts
        .iter()
        .filter_map(|text| units(text).first().map(|unit| &text[unit.start..unit.end]))
        .collect();
    unit_range(&occurrences)
}

/// `texts` share one wording; each unit that differs between them is
/// written as its range. A text whose units do not line up with the
/// first's — impossible for lines of one template — leaves the first
/// text as it is.
fn range_text(texts: &[&str]) -> String {
    let Some(first) = texts.first() else {
        return String::new();
    };
    let layout = units(first);
    let per_text: Vec<Vec<Unit>> = texts.iter().map(|text| units(text)).collect();
    if per_text.iter().any(|found| found.len() != layout.len()) {
        return (*first).to_string();
    }
    let mut out = String::with_capacity(first.len());
    let mut rest = 0;
    for (index, unit) in layout.iter().enumerate() {
        out.push_str(&first[rest..unit.start]);
        let occurrences: Vec<&str> = texts
            .iter()
            .zip(&per_text)
            .map(|(text, found)| &text[found[index].start..found[index].end])
            .collect();
        out.push_str(&unit_range(&occurrences).unwrap_or_default());
        rest = unit.end;
    }
    out.push_str(&first[rest..]);
    out
}

#[cfg(test)]
mod tests {
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::arz::{ArzDialect, ArzFile};
    use univault_engine::text::TextDb;

    use super::*;
    use crate::stats::Emphasis;

    const CLERIC_A: &str = "records/items/lootaffixes/prefix/b_ar013_ar.dbr";
    const CLERIC_B: &str = "records/items/lootaffixes/prefix/b_ar013_ar_b.dbr";
    const CLERIC_C: &str = "records/items/lootaffixes/prefix/b_ar013_ar_c.dbr";
    const STONEHIDE_ARMOR: &str = "records/items/lootaffixes/prefix/b_ar028_ar.dbr";
    const STONEHIDE_SHIELD: &str = "records/items/lootaffixes/prefix/b_sh016_a.dbr";
    const BLOOD_A: &str = "records/items/lootaffixes/suffix/a035a_off_dmg_01_fo.dbr";
    const BLOOD_B: &str = "records/items/lootaffixes/suffix/a035a_off_dmg_02_fo.dbr";
    const NIGHTBLADE_1: &str = "records/items/lootaffixes/prefix/aa014a_nightblade01_je.dbr";
    const NIGHTBLADE_2: &str = "records/items/lootaffixes/prefix/aa014a_nightblade02_je.dbr";
    const ASCENDED: &str = "records/items/lootaffixes/ascended/ao303c.dbr";
    const COMPLETION: &str = "records/items/lootaffixes/completion/a24a_physretaliation.dbr";
    const SKILL_1: &str = "records/skills/nightblade/phantasmalblades.dbr";
    const SKILL_2: &str = "records/skills/nightblade/heartseeker.dbr";

    fn affix(
        builder: &mut ArzBuilder,
        id: &str,
        name: Option<&str>,
        rarity: &str,
        level: i32,
        floats: &[(&str, f32)],
        strings: &[(&str, &str)],
    ) {
        let rarity = [rarity];
        let level = [level];
        let mut variables: Vec<(&str, Values<'_>)> = vec![
            ("Class", Values::Strings(&[AFFIX_CLASS])),
            ("itemClassification", Values::Strings(&rarity)),
            ("levelRequirement", Values::Ints(&level)),
        ];
        let name = name.map(|tag| [tag]);
        if let Some(name) = &name {
            variables.push(("lootRandomizerName", Values::Strings(name)));
        }
        let floats: Vec<(&str, [f32; 1])> = floats.iter().map(|(k, v)| (*k, [*v])).collect();
        variables.extend(floats.iter().map(|(k, v)| (*k, Values::Floats(v))));
        let strings: Vec<(&str, [&str; 1])> = strings.iter().map(|(k, v)| (*k, [*v])).collect();
        variables.extend(strings.iter().map(|(k, v)| (*k, Values::Strings(v))));
        builder.record(id, AFFIX_CLASS, &variables);
    }

    fn fixture() -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        for (id, level, pierce, health) in [
            (CLERIC_A, 5, 10.0, 4.0),
            (CLERIC_B, 36, 14.0, 4.0),
            (CLERIC_C, 50, 24.0, 4.0),
        ] {
            affix(
                &mut builder,
                id,
                Some("tagCleric"),
                "Rare",
                level,
                &[
                    ("defensivePierce", pierce),
                    ("characterLifeModifier", health),
                ],
                &[],
            );
        }
        affix(
            &mut builder,
            STONEHIDE_ARMOR,
            Some("tagStonehide"),
            "Rare",
            5,
            &[("defensivePierce", 9.0), ("defensivePoison", 12.0)],
            &[],
        );
        affix(
            &mut builder,
            STONEHIDE_SHIELD,
            Some("tagStonehide"),
            "Rare",
            5,
            &[
                ("defensivePierce", 9.0),
                ("characterDefensiveAbility", 15.0),
                ("augmentSkillLevel1", 2.0),
            ],
            &[("augmentSkillName1", SKILL_1)],
        );
        for (id, level, min, max) in [(BLOOD_A, 0, 1.0, 15.0), (BLOOD_B, 16, 1.0, 23.0)] {
            affix(
                &mut builder,
                id,
                Some("tagBlood"),
                "Magical",
                level,
                &[("offensiveFireMin", min), ("offensiveFireMax", max)],
                &[],
            );
        }
        for (id, skill) in [(NIGHTBLADE_1, SKILL_1), (NIGHTBLADE_2, SKILL_2)] {
            affix(
                &mut builder,
                id,
                Some("tagNightblade"),
                "Magical",
                0,
                &[
                    ("characterDexterityModifier", 4.0),
                    ("augmentSkillLevel1", 2.0),
                ],
                &[("augmentSkillName1", skill)],
            );
        }
        strays_and_skills(&mut builder);
        GameData::from_parts(
            vec![ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap()],
            text(),
            Vec::new(),
        )
    }

    /// Two records the table must leave out — an ascendant affix and a
    /// completion bonus that even shares a name tag — and the two
    /// skills the mastery prefix points at.
    fn strays_and_skills(builder: &mut ArzBuilder) {
        affix(
            builder,
            ASCENDED,
            None,
            "Rare",
            0,
            &[("defensivePierce", 5.0)],
            &[],
        );
        affix(
            builder,
            COMPLETION,
            Some("tagCleric"),
            "Rare",
            0,
            &[("defensivePierce", 5.0)],
            &[],
        );
        for (id, tag) in [
            (SKILL_1, "tagPhantasmalBlades"),
            (SKILL_2, "tagHeartSeeker"),
        ] {
            builder.record(
                id,
                "Skill_AttackRadius",
                &[
                    ("Class", Values::Strings(&["Skill_AttackRadius"])),
                    ("skillDisplayName", Values::Strings(&[tag])),
                ],
            );
        }
    }

    fn text() -> TextDb {
        let mut text = TextDb::new();
        text.add_file(
            b"tagCleric=Cleric's\ntagStonehide=Stonehide\ntagBlood=of Blood\n\
              tagNightblade=Nightblade's\ntagPhantasmalBlades=Phantasmal Blades\n\
              tagHeartSeeker=Heart Seeker\n\
              DefensePierce={%.0f0}% Pierce Resistance\n\
              DefensePoison={%.0f0}% Poison & Acid Resistance\n\
              tagCharAttribute04Modifier=+{%.0f0}% Health\n\
              tagCharAttribute01Modifier=+{%.0f0}% Cunning\n\
              tagCharDefensiveAbility=+{%.0f0} Defensive Ability\n\
              DamageFire={%t0} Fire Damage\n\
              DamageRangeFormat={%.0f0}-{%.0f1}\nDamageSingleFormat={%.0f0}\n\
              ItemSkillIncrement=+{%.0f0} to {%s1}\n",
        );
        text
    }

    fn entry<'a>(table: &'a AffixTable, name: &str) -> &'a AffixEntry {
        table
            .entries()
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("no entry {name}: {:?}", table.entries()))
    }

    fn grant_texts(entry: &AffixEntry) -> Vec<&str> {
        entry
            .grants
            .iter()
            .map(|grant| grant.line.text.as_str())
            .collect()
    }

    #[test]
    fn positions_come_from_the_two_named_folders() {
        let prefix = RecordId::parse(CLERIC_A.to_uppercase()).unwrap();
        assert_eq!(Position::of(&prefix), Some(Position::Prefix));
        assert_eq!(
            Position::of(&RecordId::parse(BLOOD_A.into()).unwrap()),
            Some(Position::Suffix)
        );
        assert_eq!(
            Position::of(&RecordId::parse(ASCENDED.into()).unwrap()),
            None
        );
        assert_eq!(
            Position::of(&RecordId::parse(COMPLETION.into()).unwrap()),
            None
        );
    }

    #[test]
    fn the_table_holds_one_entry_per_name_sorted_by_name() {
        let table = AffixTable::build(&fixture());
        let names: Vec<(&str, Position)> = table
            .entries()
            .iter()
            .map(|entry| (entry.name.as_str(), entry.position))
            .collect();
        assert_eq!(
            names,
            vec![
                ("Cleric's", Position::Prefix),
                ("Nightblade's", Position::Prefix),
                ("of Blood", Position::Suffix),
                ("Stonehide", Position::Prefix),
            ]
        );
    }

    #[test]
    fn tiers_sort_by_level_and_span_the_requirements() {
        let table = AffixTable::build(&fixture());
        let cleric = entry(&table, "Cleric's");
        assert_eq!(cleric.rarity, Some(Rarity::Rare));
        assert_eq!(cleric.levels, Some(LevelRange { low: 5, high: 50 }));
        assert_eq!(
            cleric
                .tiers
                .iter()
                .map(|tier| tier.level)
                .collect::<Vec<_>>(),
            vec![Some(5), Some(36), Some(50)]
        );
        assert_eq!(cleric.tiers[1].record.as_str(), CLERIC_B);
        let blood = entry(&table, "of Blood");
        assert_eq!(blood.levels, Some(LevelRange { low: 16, high: 16 }));
        assert_eq!(blood.tiers[0].level, None, "a zero requirement is none");
    }

    #[test]
    fn varying_numbers_become_ranges_and_constant_ones_stay() {
        let table = AffixTable::build(&fixture());
        let cleric = entry(&table, "Cleric's");
        assert_eq!(
            grant_texts(cleric),
            vec!["10–24% Pierce Resistance", "+4% Health"]
        );
        assert!(
            cleric
                .grants
                .iter()
                .all(|grant| grant.coverage == Coverage::Every)
        );
        assert_eq!(cleric.grants[0].line.emphasis, Emphasis::Bonus);
    }

    #[test]
    fn a_damage_range_is_one_unit() {
        let table = AffixTable::build(&fixture());
        assert_eq!(
            grant_texts(entry(&table, "of Blood")),
            vec!["1-15 to 1-23 Fire Damage"]
        );
    }

    #[test]
    fn a_stat_only_some_forms_carry_says_so_after_the_shared_ones() {
        let table = AffixTable::build(&fixture());
        let stonehide = entry(&table, "Stonehide");
        assert_eq!(stonehide.tiers.len(), 2);
        let grants: Vec<(&str, Coverage)> = stonehide
            .grants
            .iter()
            .map(|grant| (grant.line.text.as_str(), grant.coverage))
            .collect();
        assert_eq!(
            grants,
            vec![
                ("9% Pierce Resistance", Coverage::Every),
                (
                    "12% Poison & Acid Resistance",
                    Coverage::Some { tiers: 1, of: 2 }
                ),
                ("+15 Defensive Ability", Coverage::Some { tiers: 1, of: 2 }),
                (
                    "+2 to Phantasmal Blades",
                    Coverage::Some { tiers: 1, of: 2 }
                ),
            ],
            "a lone skill line is not folded into 'one of 1 skills'"
        );
    }

    #[test]
    fn per_skill_variants_fold_into_one_line() {
        let table = AffixTable::build(&fixture());
        let nightblade = entry(&table, "Nightblade's");
        let grants: Vec<(&str, Coverage)> = nightblade
            .grants
            .iter()
            .map(|grant| (grant.line.text.as_str(), grant.coverage))
            .collect();
        assert_eq!(
            grants,
            vec![
                ("+4% Cunning", Coverage::Every),
                ("+2 to one of 2 skills", Coverage::Every),
            ]
        );
        assert_eq!(nightblade.grants[1].line.kind, LineKind::SkillBonus);
    }

    #[test]
    fn nameless_and_misfiled_records_stay_out() {
        let table = AffixTable::build(&fixture());
        assert!(
            table
                .entries()
                .iter()
                .flat_map(|entry| &entry.tiers)
                .all(|tier| tier.record.as_str() != ASCENDED && tier.record.as_str() != COMPLETION)
        );
        assert_eq!(entry(&table, "Cleric's").tiers.len(), 3);
    }

    #[test]
    fn the_query_matches_names_grants_and_tier_lines_case_insensitively() {
        fn names<'a>(table: &'a AffixTable, query: &AffixQuery) -> Vec<&'a str> {
            table
                .matching(query)
                .map(|(_, entry)| entry.name.as_str())
                .collect()
        }
        let table = AffixTable::build(&fixture());
        let names = |query: &AffixQuery| names(&table, query);
        assert_eq!(names(&AffixQuery::default()).len(), 4);
        assert_eq!(
            names(&AffixQuery {
                text: " cleric ".into(),
                ..AffixQuery::default()
            }),
            vec!["Cleric's"]
        );
        assert_eq!(
            names(&AffixQuery {
                text: "pierce".into(),
                ..AffixQuery::default()
            }),
            vec!["Cleric's", "Stonehide"]
        );
        assert_eq!(
            names(&AffixQuery {
                text: "heart seeker".into(),
                ..AffixQuery::default()
            }),
            vec!["Nightblade's"],
            "a folded variant is still found by its own line"
        );
        assert_eq!(
            names(&AffixQuery {
                position: Some(Position::Suffix),
                ..AffixQuery::default()
            }),
            vec!["of Blood"]
        );
        assert_eq!(
            names(&AffixQuery {
                rarity: Some(Rarity::Magical),
                position: Some(Position::Prefix),
                ..AffixQuery::default()
            }),
            vec!["Nightblade's"]
        );
        let indices: Vec<usize> = table
            .matching(&AffixQuery {
                text: "blood".into(),
                ..AffixQuery::default()
            })
            .map(|(index, _)| index)
            .collect();
        assert_eq!(indices, vec![2]);
    }

    #[test]
    fn the_query_serializes_by_field_and_defaults_the_rest() {
        let query = AffixQuery {
            text: "cleric".into(),
            position: Some(Position::Prefix),
            rarity: Some(Rarity::Rare),
        };
        let json = serde_json::to_string(&query).unwrap();
        assert!(json.contains("\"position\":\"prefix\""), "{json}");
        assert_eq!(serde_json::from_str::<AffixQuery>(&json).unwrap(), query);
        let partial: AffixQuery = serde_json::from_str(r#"{"text":"x"}"#).unwrap();
        assert_eq!(partial.text, "x");
        assert_eq!(partial.position, None);
        assert!(!partial.is_empty());
        assert!(AffixQuery::default().is_empty());
    }

    #[test]
    fn range_text_writes_each_varying_unit_once() {
        assert_eq!(
            range_text(&["+4% Health", "+4% Health", "+6% Health"]),
            "+4–6% Health"
        );
        assert_eq!(
            range_text(&[
                "24 Electrocute Damage over 3.0 Seconds",
                "42 Electrocute Damage over 3.0 Seconds"
            ]),
            "24–42 Electrocute Damage over 3.0 Seconds"
        );
        assert_eq!(
            range_text(&["Stun target for 0.5 - 1.0 Seconds"]),
            "Stun target for 0.5 - 1.0 Seconds"
        );
        assert_eq!(
            range_text(&["-15% Skill Energy Cost", "-10% Skill Energy Cost"]),
            "-15–-10% Skill Energy Cost"
        );
        assert_eq!(range_text(&[]), "");
    }
}
