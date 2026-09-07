//! The item query and sort: a conjunction of typed criteria — name,
//! affix and stat lines with value windows, requirement caps, set,
//! rarity, category, socket, and the tile facets — answered per item
//! with a three-way [`Verdict`] so an item whose facts could not be
//! resolved is reported as such rather than silently dropped as a
//! non-match. A shell owns the [`Query`] and asks with a [`Subject`]
//! it has memoized; the headless path resolves one from the database
//! ([`Resolved`]). The filter set and the stat-template rule are
//! ported in shape from tq-univault's `query.rs` (MIT OR Apache-2.0,
//! same author), rebuilt on Grim Dawn's item shape and stat lines.
//!
//! Stat text is matched against the line's [`stat_template`] — its
//! text with every number replaced by `#` — and against the text
//! itself, so a picked "+#% Cold Damage" template and free text like
//! "cold damage" or "40% cold" all hit; a value window compares the
//! line's largest rendered number ([`StatLine::values`]), so a
//! numberless line never satisfies a bounded criterion.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};
use univault_engine::ids::RecordId;

use crate::bucket::{Bucket, Group};
use crate::facets::{
    AffixEvidence, Ascension, AscensionTable, BaseEvidence, DoubleRare, Facets, MonsterInfrequent,
};
use crate::gamedata::{GameData, Rarity};
use crate::item::Item;
use crate::stats::{
    self, BlockSource, ItemDetails, LineKind, Requirement, StatLine, UnrenderedReason,
};

/// Whether a facet must hold.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Constraint {
    #[default]
    Any,
    Required,
}

impl Constraint {
    /// Flips between the two states, for a toggle.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Any => Self::Required,
            Self::Required => Self::Any,
        }
    }

    #[must_use]
    pub const fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Which ascension standing, if any, an item must have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AscensionFilter {
    #[default]
    Any,
    /// Eligible for an ascendant affix and without one yet.
    Upgradeable,
    /// Carrying an ascendant affix.
    Ascended,
}

/// A value window on a stat line: the line's largest number must fall
/// inside `[min, max]` (either side open). Fully open bounds make the
/// criterion presence-only — a numberless line passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ValueBounds {
    pub min: Option<f32>,
    pub max: Option<f32>,
}

impl ValueBounds {
    pub const ANY: Self = Self {
        min: None,
        max: None,
    };

    #[must_use]
    pub fn is_unbounded(self) -> bool {
        self.min.is_none() && self.max.is_none()
    }

    fn admits(self, largest: Option<f32>) -> bool {
        if self.is_unbounded() {
            return true;
        }
        let Some(value) = largest else { return false };
        self.min.is_none_or(|min| value >= min) && self.max.is_none_or(|max| value <= max)
    }
}

/// Where a stat or affix criterion looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// Any stat line on the item: base, affixes, component and its
    /// completion bonus, augment, ascendant bonus.
    #[default]
    AnyStat,
    /// Only the lines the prefix or suffix grants.
    AffixStat,
    /// The prefix's or suffix's own name.
    AffixName,
}

impl Scope {
    pub const ALL: [Self; 3] = [Self::AnyStat, Self::AffixStat, Self::AffixName];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AnyStat => "Any stat",
            Self::AffixStat => "Affix stat",
            Self::AffixName => "Affix name",
        }
    }
}

/// One stat or affix criterion. Text matches case-insensitively as a
/// substring; a criterion with blank text and no value window is
/// inert and constrains nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Criterion {
    /// A prefix or suffix whose name contains the text is present.
    HasAffix { text: String },
    /// A prefix or suffix grants a stat line matching the text, its
    /// largest number inside `bounds`.
    AffixStat { text: String, bounds: ValueBounds },
    /// Any stat line on the item matches the text, its largest number
    /// inside `bounds`.
    StatContains { text: String, bounds: ValueBounds },
}

impl Default for Criterion {
    fn default() -> Self {
        Scope::AnyStat.blank()
    }
}

impl Scope {
    /// An inert criterion of this scope, for a fresh bar row.
    #[must_use]
    pub fn blank(self) -> Criterion {
        match self {
            Self::AnyStat => Criterion::StatContains {
                text: String::new(),
                bounds: ValueBounds::ANY,
            },
            Self::AffixStat => Criterion::AffixStat {
                text: String::new(),
                bounds: ValueBounds::ANY,
            },
            Self::AffixName => Criterion::HasAffix {
                text: String::new(),
            },
        }
    }
}

impl Criterion {
    #[must_use]
    pub const fn scope(&self) -> Scope {
        match self {
            Self::HasAffix { .. } => Scope::AffixName,
            Self::AffixStat { .. } => Scope::AffixStat,
            Self::StatContains { .. } => Scope::AnyStat,
        }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            Self::HasAffix { text }
            | Self::AffixStat { text, .. }
            | Self::StatContains { text, .. } => text,
        }
    }

    pub fn text_mut(&mut self) -> &mut String {
        match self {
            Self::HasAffix { text }
            | Self::AffixStat { text, .. }
            | Self::StatContains { text, .. } => text,
        }
    }

    /// The value window, for the scopes that have one.
    pub fn bounds_mut(&mut self) -> Option<&mut ValueBounds> {
        match self {
            Self::HasAffix { .. } => None,
            Self::AffixStat { bounds, .. } | Self::StatContains { bounds, .. } => Some(bounds),
        }
    }

    /// The same text (and window, where the new scope has one) under
    /// another scope.
    #[must_use]
    pub fn with_scope(self, scope: Scope) -> Self {
        let bounds = match &self {
            Self::HasAffix { .. } => ValueBounds::ANY,
            Self::AffixStat { bounds, .. } | Self::StatContains { bounds, .. } => *bounds,
        };
        let text = match self {
            Self::HasAffix { text }
            | Self::AffixStat { text, .. }
            | Self::StatContains { text, .. } => text,
        };
        match scope {
            Scope::AnyStat => Self::StatContains { text, bounds },
            Scope::AffixStat => Self::AffixStat { text, bounds },
            Scope::AffixName => Self::HasAffix { text },
        }
    }

    /// Whether the criterion constrains nothing.
    #[must_use]
    pub fn is_inert(&self) -> bool {
        let blank = self.text().trim().is_empty();
        match self {
            Self::HasAffix { .. } => blank,
            Self::AffixStat { bounds, .. } | Self::StatContains { bounds, .. } => {
                blank && bounds.is_unbounded()
            }
        }
    }

    fn answer(&self, subject: &Subject<'_>) -> Answer {
        if self.is_inert() {
            return Answer::Holds;
        }
        let needle = self.text().trim().to_lowercase();
        match self {
            Self::HasAffix { .. } => {
                let named = |affix: AffixName<'_>| match affix {
                    AffixName::Named(name) => contains_ci(name, &needle),
                    AffixName::Absent | AffixName::Unresolved => false,
                };
                if named(subject.prefix) || named(subject.suffix) {
                    Answer::Holds
                } else if subject.prefix == AffixName::Unresolved
                    || subject.suffix == AffixName::Unresolved
                {
                    Answer::Unknown
                } else {
                    Answer::Fails
                }
            }
            Self::AffixStat { bounds, .. } => lines_answer(
                subject,
                |source| matches!(source, BlockSource::Prefix | BlockSource::Suffix),
                &needle,
                *bounds,
            ),
            Self::StatContains { bounds, .. } => lines_answer(subject, |_| true, &needle, *bounds),
        }
    }
}

/// Upper limits on the item's effective requirements; a requirement
/// the item does not state passes any cap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RequirementCaps {
    pub level: Option<u32>,
    pub physique: Option<u32>,
    pub cunning: Option<u32>,
    pub spirit: Option<u32>,
}

impl RequirementCaps {
    #[must_use]
    pub const fn get(self, requirement: Requirement) -> Option<u32> {
        match requirement {
            Requirement::Level => self.level,
            Requirement::Physique => self.physique,
            Requirement::Cunning => self.cunning,
            Requirement::Spirit => self.spirit,
        }
    }

    pub const fn slot(&mut self, requirement: Requirement) -> &mut Option<u32> {
        match requirement {
            Requirement::Level => &mut self.level,
            Requirement::Physique => &mut self.physique,
            Requirement::Cunning => &mut self.cunning,
            Requirement::Spirit => &mut self.spirit,
        }
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        Requirement::ALL
            .iter()
            .all(|requirement| self.get(*requirement).is_none())
    }

    fn caps(self) -> impl Iterator<Item = (Requirement, u32)> {
        Requirement::ALL
            .into_iter()
            .filter_map(move |requirement| self.get(requirement).map(|cap| (requirement, cap)))
    }
}

/// Whether the item's base must belong to a set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SetFilter {
    #[default]
    Any,
    /// Any set at all.
    Member,
    /// A set whose name contains the text (blank: any set).
    Named { text: String },
}

/// Which type view the item must file under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CategoryFilter {
    #[default]
    Any,
    Group {
        group: Group,
    },
    Bucket {
        bucket: Bucket,
    },
}

/// Whether a component must (not) be socketed into the item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SocketFilter {
    #[default]
    Any,
    Socketed,
    Unsocketed,
}

/// The whole query, a conjunction. `name` matches case-insensitively
/// as a substring of the displayed name; blank means any. Every field
/// defaults to "any", so a persisted query from an older build
/// restores the fields it has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Query {
    pub name: String,
    pub criteria: Vec<Criterion>,
    pub requirements: RequirementCaps,
    pub set: SetFilter,
    pub rarity: Option<Rarity>,
    pub category: CategoryFilter,
    pub socket: SocketFilter,
    pub monster_infrequent: Constraint,
    pub double_rare: Constraint,
    pub ascension: AscensionFilter,
}

/// How one item answers a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Matches,
    Excluded,
    /// A constraint could not be decided for the item.
    Unresolved,
}

/// An affix slot as the query sees it: empty, naming a record the
/// database could not name, or resolved to its display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AffixName<'a> {
    Absent,
    Unresolved,
    Named(&'a str),
}

impl<'a> AffixName<'a> {
    /// From the slot's record path and the name the database gave it.
    #[must_use]
    pub fn of(path: &str, name: Option<&'a str>) -> Self {
        match (path.is_empty(), name) {
            (true, _) => Self::Absent,
            (false, None) => Self::Unresolved,
            (false, Some(name)) => Self::Named(name),
        }
    }
}

/// What a query is asked about: the item and the facts a shell has
/// resolved for it. `details` is the rendered stat body; a query
/// that needs it and finds none reports the item unresolved rather
/// than guessing.
#[derive(Clone, Copy, Debug)]
pub struct Subject<'a> {
    pub item: &'a Item,
    pub name: &'a str,
    pub prefix: AffixName<'a>,
    pub suffix: AffixName<'a>,
    pub base: BaseEvidence,
    pub facets: Facets,
    pub details: Option<&'a ItemDetails>,
}

/// One constraint's answer, folded into the item's verdict.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Holds,
    Fails,
    Unknown,
}

impl Answer {
    fn of(holds: bool) -> Self {
        if holds { Self::Holds } else { Self::Fails }
    }
}

impl Query {
    /// Whether the query constrains nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.name.trim().is_empty()
            && self.criteria.iter().all(Criterion::is_inert)
            && self.requirements.is_empty()
            && self.set == SetFilter::Any
            && self.rarity.is_none()
            && self.category == CategoryFilter::Any
            && self.socket == SocketFilter::Any
            && self.monster_infrequent == Constraint::Any
            && self.double_rare == Constraint::Any
            && self.ascension == AscensionFilter::Any
    }

    /// Whether answering needs the item's rendered stat body — stat
    /// criteria, requirement caps, and the set filter read it; the
    /// rest is answered from the names and facets alone.
    #[must_use]
    pub fn needs_details(&self) -> bool {
        self.criteria
            .iter()
            .any(|criterion| !criterion.is_inert() && criterion.scope() != Scope::AffixName)
            || !self.requirements.is_empty()
            || self.set != SetFilter::Any
    }

    /// Answers for a subject the shell has already resolved.
    #[must_use]
    pub fn verdict(&self, subject: &Subject<'_>) -> Verdict {
        let mut answers = vec![
            self.name_answer(subject.name),
            self.set_answer(subject),
            self.rarity_answer(subject),
            self.category_answer(subject),
            self.socket_answer(subject),
            required(self.monster_infrequent, || {
                match subject.facets.monster_infrequent {
                    MonsterInfrequent::Yes => Answer::Holds,
                    MonsterInfrequent::No => Answer::Fails,
                    MonsterInfrequent::Unresolved => Answer::Unknown,
                }
            }),
            required(self.double_rare, || match subject.facets.double_rare {
                DoubleRare::Yes => Answer::Holds,
                DoubleRare::No => Answer::Fails,
                DoubleRare::Unresolved => Answer::Unknown,
            }),
            self.ascension_answer(subject.facets.ascension),
        ];
        answers.extend(
            self.criteria
                .iter()
                .map(|criterion| criterion.answer(subject)),
        );
        answers.extend(
            self.requirements
                .caps()
                .map(|(requirement, cap)| requirement_answer(subject, requirement, cap)),
        );
        if answers.contains(&Answer::Fails) {
            Verdict::Excluded
        } else if answers.contains(&Answer::Unknown) {
            Verdict::Unresolved
        } else {
            Verdict::Matches
        }
    }

    /// Answers for an item straight from the database.
    #[must_use]
    pub fn verdict_of(&self, game: &GameData, table: &AscensionTable, item: &Item) -> Verdict {
        self.verdict(&Resolved::of(game, table, item).subject(item))
    }

    fn name_answer(&self, name: &str) -> Answer {
        let needle = self.name.trim().to_lowercase();
        Answer::of(needle.is_empty() || contains_ci(name, &needle))
    }

    fn set_answer(&self, subject: &Subject<'_>) -> Answer {
        let needle = match &self.set {
            SetFilter::Any => return Answer::Holds,
            SetFilter::Member => String::new(),
            SetFilter::Named { text } => text.trim().to_lowercase(),
        };
        let Some(details) = subject.details else {
            return Answer::Unknown;
        };
        match (&details.set, subject.base) {
            (Some(set), _) => Answer::of(needle.is_empty() || contains_ci(&set.name, &needle)),
            (None, BaseEvidence::Unresolved) => Answer::Unknown,
            (None, BaseEvidence::Known { .. }) => Answer::Fails,
        }
    }

    /// The rarity the item displays as — its base's, raised by a rarer
    /// affix — which is how the game colours its name.
    fn rarity_answer(&self, subject: &Subject<'_>) -> Answer {
        let Some(wanted) = self.rarity else {
            return Answer::Holds;
        };
        subject
            .facets
            .displayed_rarity()
            .map_or(Answer::Unknown, |rarity| Answer::of(rarity == wanted))
    }

    fn category_answer(&self, subject: &Subject<'_>) -> Answer {
        let bucket = match (self.category, subject.base) {
            (CategoryFilter::Any, _) => return Answer::Holds,
            (_, BaseEvidence::Unresolved) => return Answer::Unknown,
            (_, BaseEvidence::Known { bucket, .. }) => bucket,
        };
        match self.category {
            CategoryFilter::Any => Answer::Holds,
            CategoryFilter::Group { group } => Answer::of(bucket.group() == group),
            CategoryFilter::Bucket { bucket: wanted } => Answer::of(bucket == wanted),
        }
    }

    fn socket_answer(&self, subject: &Subject<'_>) -> Answer {
        let socketed = !subject.item.relic_name.is_empty();
        match self.socket {
            SocketFilter::Any => Answer::Holds,
            SocketFilter::Socketed => Answer::of(socketed),
            SocketFilter::Unsocketed => Answer::of(!socketed),
        }
    }

    /// An ascended item is known from its own fields, so the
    /// `Ascended` filter never leaves an item unresolved; only
    /// eligibility depends on records that may be missing.
    fn ascension_answer(&self, ascension: Ascension) -> Answer {
        match self.ascension {
            AscensionFilter::Any => Answer::Holds,
            AscensionFilter::Ascended => Answer::of(ascension == Ascension::Ascended),
            AscensionFilter::Upgradeable => match ascension {
                Ascension::Eligible => Answer::Holds,
                Ascension::Ascended | Ascension::Ineligible => Answer::Fails,
                Ascension::Unresolved => Answer::Unknown,
            },
        }
    }
}

fn required(constraint: Constraint, facet: impl FnOnce() -> Answer) -> Answer {
    match constraint {
        Constraint::Any => Answer::Holds,
        Constraint::Required => facet(),
    }
}

/// A known requirement above the cap fails whatever else is unknown;
/// without the base record the effective requirement is incomplete,
/// so a pass is only a pass when the base is known.
fn requirement_answer(subject: &Subject<'_>, requirement: Requirement, cap: u32) -> Answer {
    let Some(details) = subject.details else {
        return Answer::Unknown;
    };
    let stated = details
        .requirements
        .iter()
        .find(|(known, _)| *known == requirement)
        .map(|(_, value)| *value);
    match (stated, subject.base) {
        (Some(value), _) if value > cap => Answer::Fails,
        (_, BaseEvidence::Unresolved) => Answer::Unknown,
        (_, BaseEvidence::Known { .. }) => Answer::Holds,
    }
}

/// Whether a line matching the needle inside the window exists among
/// the blocks `wanted` selects; a miss is unknown, not a failure, when
/// one of those blocks names a record the database lacks.
fn lines_answer(
    subject: &Subject<'_>,
    wanted: impl Fn(BlockSource) -> bool,
    needle: &str,
    bounds: ValueBounds,
) -> Answer {
    let Some(details) = subject.details else {
        return Answer::Unknown;
    };
    let blocks = details.blocks.iter().filter(|block| wanted(block.source));
    let matched = blocks
        .clone()
        .flat_map(|block| block.lines.iter())
        .filter(|line| is_searchable(line))
        .any(|line| line_passes(line, needle, bounds));
    if matched {
        Answer::Holds
    } else if blocks
        .clone()
        .any(|block| block_missing(details, block.record.as_str()))
    {
        Answer::Unknown
    } else {
        Answer::Fails
    }
}

fn block_missing(details: &ItemDetails, record: &str) -> bool {
    details.unrendered.iter().any(
        |gap| matches!(&gap.reason, UnrenderedReason::MissingRecord(missing) if missing == record),
    )
}

/// Whether a line is stat text a search should read: everything a
/// block renders except the prose of a granted skill's description.
#[must_use]
pub fn is_searchable(line: &StatLine) -> bool {
    !matches!(line.kind, LineKind::SkillDescription)
}

/// `needle` is already lowercased and trimmed.
fn line_passes(line: &StatLine, needle: &str, bounds: ValueBounds) -> bool {
    (contains_ci(&stat_template(&line.text), needle) || contains_ci(&line.text, needle))
        && bounds.admits(largest_value(line))
}

fn contains_ci(haystack: &str, needle_lowercase: &str) -> bool {
    haystack.to_lowercase().contains(needle_lowercase)
}

/// The largest number the line rendered.
#[must_use]
pub fn largest_value(line: &StatLine) -> Option<f32> {
    line.values.iter().copied().reduce(f32::max)
}

/// The display line with every number replaced by `#` — the rolled
/// values drop out, the wording stays: "+24% Pierce Resistance"
/// becomes "+#% Pierce Resistance" and "9-67 Lightning Damage"
/// becomes "#-# Lightning Damage". This is the key a stat vocabulary
/// groups lines under and one of the two haystacks stat criteria
/// match against.
#[must_use]
pub fn stat_template(text: &str) -> String {
    let mut template = String::with_capacity(text.len());
    let mut rest = 0;
    for (start, end) in number_spans(text) {
        template.push_str(&text[rest..start]);
        template.push('#');
        rest = end;
    }
    template.push_str(&text[rest..]);
    template
}

/// Every number in a display line as a byte span; a `-` is part of
/// the span only when directly attached and not itself following a
/// digit ("-15%" is one negative number; "9-67" is two).
fn number_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let signed = i > 0 && bytes[i - 1] == b'-' && (i == 1 || !bytes[i - 2].is_ascii_digit());
        let start = if signed { i - 1 } else { i };
        let mut end = i;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
            end += 1;
        }
        let trailing_dots = text[start..end].len() - text[start..end].trim_end_matches('.').len();
        spans.push((start, end - trailing_dots));
        i = end;
    }
    spans
}

/// Everything a query and a sort ask about one item, resolved from
/// the database — the headless path; a shell memoizes its own.
#[derive(Clone, Debug)]
pub struct Resolved {
    name: String,
    prefix: Option<String>,
    suffix: Option<String>,
    base: BaseEvidence,
    facets: Facets,
    details: ItemDetails,
}

impl Resolved {
    #[must_use]
    pub fn of(game: &GameData, table: &AscensionTable, item: &Item) -> Self {
        let base_id = RecordId::parse(item.base_name.clone());
        let prefix_id = RecordId::parse(item.prefix_name.clone());
        let suffix_id = RecordId::parse(item.suffix_name.clone());
        let base = base_id
            .as_ref()
            .and_then(|id| game.item_info(id))
            .and_then(Result::ok);
        let affix = |id: Option<&RecordId>| id.and_then(|id| game.affix_info(id)?.ok());
        let prefix = affix(prefix_id.as_ref());
        let suffix = affix(suffix_id.as_ref());
        let name = base_id
            .as_ref()
            .and_then(|id| game.display_name(id, prefix_id.as_ref(), suffix_id.as_ref()))
            .unwrap_or_else(|| item.base_name.clone());
        Self {
            name,
            prefix: prefix.as_ref().and_then(|info| info.name.clone()),
            suffix: suffix.as_ref().and_then(|info| info.name.clone()),
            base: BaseEvidence::of(base.as_ref()),
            facets: Facets::classify(
                item,
                BaseEvidence::of(base.as_ref()),
                AffixEvidence::of(&item.prefix_name, prefix.as_ref()),
                AffixEvidence::of(&item.suffix_name, suffix.as_ref()),
                table,
            ),
            details: stats::item_details(game, item),
        }
    }

    #[must_use]
    pub fn subject<'a>(&'a self, item: &'a Item) -> Subject<'a> {
        Subject {
            item,
            name: &self.name,
            prefix: AffixName::of(&item.prefix_name, self.prefix.as_deref()),
            suffix: AffixName::of(&item.suffix_name, self.suffix.as_deref()),
            base: self.base,
            facets: self.facets,
            details: Some(&self.details),
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn details(&self) -> &ItemDetails {
        &self.details
    }
}

/// What a result list can be ordered by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    #[default]
    Name,
    Rarity,
    Level,
    Category,
}

impl SortKey {
    pub const ALL: [Self; 4] = [Self::Name, Self::Rarity, Self::Level, Self::Category];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Rarity => "Rarity",
            Self::Level => "Level",
            Self::Category => "Type",
        }
    }
}

/// What sorting compares, taken from a subject once: the name folded
/// for a case-insensitive order, the displayed rarity, the level
/// requirement, and the type bucket. Every key ranks ascending and
/// breaks ties by name; a direction is the caller's to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortRank {
    name: String,
    rarity: Option<Rarity>,
    level: Option<u32>,
    bucket: Option<Bucket>,
}

impl SortRank {
    #[must_use]
    pub fn of(subject: &Subject<'_>) -> Self {
        Self {
            name: subject.name.to_lowercase(),
            rarity: subject.facets.displayed_rarity(),
            level: subject.details.and_then(|details| {
                details
                    .requirements
                    .iter()
                    .find(|(requirement, _)| *requirement == Requirement::Level)
                    .map(|(_, level)| *level)
            }),
            bucket: match subject.base {
                BaseEvidence::Unresolved => None,
                BaseEvidence::Known { bucket, .. } => Some(bucket),
            },
        }
    }

    /// The level requirement the rank carries, for display beside it.
    #[must_use]
    pub fn level(&self) -> Option<u32> {
        self.level
    }

    #[must_use]
    pub fn compare(&self, other: &Self, key: SortKey) -> Ordering {
        let by_name = self.name.cmp(&other.name);
        match key {
            SortKey::Name => by_name,
            SortKey::Rarity => self.rarity.cmp(&other.rarity).then(by_name),
            SortKey::Level => self.level.cmp(&other.level).then(by_name),
            SortKey::Category => self.bucket.cmp(&other.bucket).then(by_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use univault_engine::arz::fixture::{ArzBuilder, Values};
    use univault_engine::arz::{ArzDialect, ArzFile};
    use univault_engine::text::TextDb;

    use crate::gamedata::Binding;
    use crate::stats::{Block, Emphasis, Section, SetInfo, Unrendered};

    use super::*;

    fn facets(
        monster_infrequent: MonsterInfrequent,
        double_rare: DoubleRare,
        ascension: Ascension,
    ) -> Facets {
        let mut facets = Facets::classify(
            &Item::default(),
            BaseEvidence::Unresolved,
            AffixEvidence::Absent,
            AffixEvidence::Absent,
            &AscensionTable::Absent,
        );
        facets.monster_infrequent = monster_infrequent;
        facets.double_rare = double_rare;
        facets.ascension = ascension;
        facets
    }

    fn resolved(mi: bool, dr: bool, ascension: Ascension) -> Facets {
        facets(
            if mi {
                MonsterInfrequent::Yes
            } else {
                MonsterInfrequent::No
            },
            if dr { DoubleRare::Yes } else { DoubleRare::No },
            ascension,
        )
    }

    fn known(rarity: Option<Rarity>, bucket: Bucket) -> BaseEvidence {
        BaseEvidence::Known {
            rarity,
            bucket,
            binding: Binding::Free,
        }
    }

    fn line(kind: LineKind, text: &str, values: &[f32]) -> StatLine {
        StatLine {
            kind,
            section: Section::Offense,
            emphasis: Emphasis::Bonus,
            text: text.to_string(),
            values: values.to_vec(),
        }
    }

    fn stat(text: &str, values: &[f32]) -> StatLine {
        line(LineKind::Conversion, text, values)
    }

    fn block(source: BlockSource, record: &str, lines: Vec<StatLine>) -> Block {
        Block {
            source,
            record: RecordId::parse(record.to_string()).unwrap(),
            title: None,
            flavor: None,
            lines,
        }
    }

    /// A sword with a fire prefix and a cold suffix, in a set.
    fn sword_details() -> ItemDetails {
        ItemDetails {
            blocks: vec![
                block(
                    BlockSource::Base,
                    "records/items/sword.dbr",
                    vec![
                        stat("12-31 Physical Damage", &[12.0, 31.0]),
                        stat("Speed: Very Slow", &[]),
                    ],
                ),
                block(
                    BlockSource::Prefix,
                    "records/items/lootaffixes/prefix/sharp.dbr",
                    vec![stat("10 Fire Damage", &[10.0])],
                ),
                block(
                    BlockSource::Suffix,
                    "records/items/lootaffixes/suffix/embers.dbr",
                    vec![
                        stat("7 Cold Damage", &[7.0]),
                        stat("+40% Cold Damage", &[40.0]),
                    ],
                ),
            ],
            requirements: vec![
                (Requirement::Level, 24),
                (Requirement::Physique, 141),
                (Requirement::Cunning, 122),
            ],
            requirement_lines: Vec::new(),
            set: Some(SetInfo {
                record: RecordId::parse("records/items/sets/olympus.dbr".into()).unwrap(),
                name: "Guardians of Olympus".into(),
                members: Vec::new(),
                tiers: Vec::new(),
            }),
            unrendered: Vec::new(),
        }
    }

    fn sword() -> Item {
        Item {
            base_name: "records/items/sword.dbr".into(),
            prefix_name: "records/items/lootaffixes/prefix/sharp.dbr".into(),
            suffix_name: "records/items/lootaffixes/suffix/embers.dbr".into(),
            ..Item::default()
        }
    }

    fn subject<'a>(item: &'a Item, details: Option<&'a ItemDetails>) -> Subject<'a> {
        Subject {
            item,
            name: "Sharp Broadsword of Embers",
            prefix: AffixName::of(&item.prefix_name, Some("Sharp")),
            suffix: AffixName::of(&item.suffix_name, Some("of Embers")),
            base: known(Some(Rarity::Epic), Bucket::OneHanded),
            facets: resolved(false, false, Ascension::Ineligible),
            details,
        }
    }

    fn stat_query(criterion: Criterion) -> Query {
        Query {
            criteria: vec![criterion],
            ..Query::default()
        }
    }

    fn at_least(min: f32) -> ValueBounds {
        ValueBounds {
            min: Some(min),
            max: None,
        }
    }

    fn contains(text: &str, bounds: ValueBounds) -> Criterion {
        Criterion::StatContains {
            text: text.into(),
            bounds,
        }
    }

    #[test]
    fn an_empty_query_matches_everything_resolved_or_not() {
        let query = Query::default();
        assert!(query.is_empty());
        assert!(!query.needs_details());
        let item = Item::default();
        let unknown = Subject {
            item: &item,
            name: "anything",
            prefix: AffixName::Unresolved,
            suffix: AffixName::Absent,
            base: BaseEvidence::Unresolved,
            facets: facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::Unresolved,
                Ascension::Unresolved,
            ),
            details: None,
        };
        assert_eq!(query.verdict(&unknown), Verdict::Matches);
    }

    #[test]
    fn the_name_matches_as_a_case_insensitive_substring() {
        let query = Query {
            name: "  BROADSWORD of ".into(),
            ..Query::default()
        };
        assert!(!query.is_empty());
        let item = sword();
        assert_eq!(query.verdict(&subject(&item, None)), Verdict::Matches);
        let miss = Query {
            name: "wendigo".into(),
            ..Query::default()
        };
        assert_eq!(miss.verdict(&subject(&item, None)), Verdict::Excluded);
    }

    #[test]
    fn required_facets_exclude_known_misses_and_flag_unresolved_ones() {
        let query = Query {
            monster_infrequent: Constraint::Required,
            double_rare: Constraint::Required,
            ..Query::default()
        };
        let item = sword();
        let verdict = |facets: Facets| {
            query.verdict(&Subject {
                facets,
                ..subject(&item, None)
            })
        };
        assert_eq!(
            verdict(resolved(true, true, Ascension::Ineligible)),
            Verdict::Matches
        );
        assert_eq!(
            verdict(resolved(true, false, Ascension::Ineligible)),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::Yes,
                Ascension::Ineligible
            )),
            Verdict::Unresolved
        );
        assert_eq!(
            verdict(facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::No,
                Ascension::Ineligible
            )),
            Verdict::Excluded
        );
    }

    #[test]
    fn the_ascension_filter_never_leaves_an_ascended_item_unresolved() {
        let upgradeable = Query {
            ascension: AscensionFilter::Upgradeable,
            ..Query::default()
        };
        let ascended = Query {
            ascension: AscensionFilter::Ascended,
            ..Query::default()
        };
        let item = sword();
        let verdict = |query: &Query, ascension: Ascension| {
            query.verdict(&Subject {
                facets: resolved(false, false, ascension),
                ..subject(&item, None)
            })
        };
        assert_eq!(verdict(&upgradeable, Ascension::Eligible), Verdict::Matches);
        assert_eq!(
            verdict(&upgradeable, Ascension::Ascended),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(&upgradeable, Ascension::Unresolved),
            Verdict::Unresolved
        );
        assert_eq!(verdict(&ascended, Ascension::Ascended), Verdict::Matches);
        assert_eq!(verdict(&ascended, Ascension::Eligible), Verdict::Excluded);
        assert_eq!(verdict(&ascended, Ascension::Unresolved), Verdict::Excluded);
    }

    #[test]
    fn a_constraint_toggles() {
        assert_eq!(Constraint::Any.toggled(), Constraint::Required);
        assert_eq!(Constraint::Required.toggled(), Constraint::Any);
        assert!(Constraint::Required.is_required());
    }

    #[test]
    fn affix_names_match_either_slot_and_stay_unknown_for_unnamed_records() {
        let item = sword();
        let has = |text: &str| {
            stat_query(Criterion::HasAffix { text: text.into() }).verdict(&subject(&item, None))
        };
        assert_eq!(has("sharp"), Verdict::Matches);
        assert_eq!(has("EMBERS"), Verdict::Matches);
        assert_eq!(has("frost"), Verdict::Excluded);
        assert_eq!(has("   "), Verdict::Matches);
        let unnamed = Subject {
            prefix: AffixName::Unresolved,
            ..subject(&item, None)
        };
        let query = stat_query(Criterion::HasAffix {
            text: "frost".into(),
        });
        assert_eq!(query.verdict(&unnamed), Verdict::Unresolved);
        let plain = Item::default();
        let no_affixes = Subject {
            prefix: AffixName::Absent,
            suffix: AffixName::Absent,
            ..subject(&plain, None)
        };
        assert_eq!(query.verdict(&no_affixes), Verdict::Excluded);
    }

    #[test]
    fn affix_stats_read_only_the_affix_blocks() {
        let item = sword();
        let details = sword_details();
        let fire = |bounds| {
            stat_query(Criterion::AffixStat {
                text: "fire".into(),
                bounds,
            })
            .verdict(&subject(&item, Some(&details)))
        };
        assert_eq!(fire(ValueBounds::ANY), Verdict::Matches);
        assert_eq!(fire(at_least(10.0)), Verdict::Matches);
        assert_eq!(fire(at_least(10.5)), Verdict::Excluded);
        let affixes_top_out_at_forty = stat_query(Criterion::AffixStat {
            text: String::new(),
            bounds: at_least(40.0),
        });
        assert_eq!(
            affixes_top_out_at_forty.verdict(&subject(&item, Some(&details))),
            Verdict::Matches
        );
        let base_only = stat_query(Criterion::AffixStat {
            text: "physical".into(),
            bounds: ValueBounds::ANY,
        });
        assert_eq!(
            base_only.verdict(&subject(&item, Some(&details))),
            Verdict::Excluded,
            "the base block's lines are not affix stats"
        );
    }

    #[test]
    fn stat_criteria_span_every_block_and_compare_the_largest_value() {
        let item = sword();
        let details = sword_details();
        let verdict = |criterion| stat_query(criterion).verdict(&subject(&item, Some(&details)));
        assert_eq!(
            verdict(contains("damage", at_least(31.0))),
            Verdict::Matches
        );
        assert_eq!(
            verdict(contains("cold damage", at_least(40.0))),
            Verdict::Matches
        );
        assert_eq!(
            verdict(contains("cold damage", at_least(41.0))),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(contains("poison", ValueBounds::ANY)),
            Verdict::Excluded
        );
        let window = |min, max| contains("damage", ValueBounds { min, max });
        assert_eq!(verdict(window(Some(31.0), Some(31.0))), Verdict::Matches);
        assert_eq!(
            verdict(window(Some(31.5), None)),
            Verdict::Matches,
            "the suffix's +40% line reaches past the base's 31"
        );
        assert_eq!(verdict(window(Some(40.5), None)), Verdict::Excluded);
        assert_eq!(verdict(window(Some(5.0), Some(8.0))), Verdict::Matches);
        assert_eq!(verdict(window(None, Some(6.0))), Verdict::Excluded);
        assert_eq!(
            verdict(contains("", at_least(100.0))),
            Verdict::Excluded,
            "a bare window still filters"
        );
        assert_eq!(
            verdict(contains("speed", at_least(1.0))),
            Verdict::Excluded,
            "a numberless line never satisfies a bounded criterion"
        );
    }

    #[test]
    fn picked_templates_and_free_text_match_alike() {
        let item = sword();
        let details = sword_details();
        for text in [
            "#-# Physical Damage",
            "physical damage",
            "12-31 phys",
            "-# Phys",
        ] {
            assert_eq!(
                stat_query(contains(text, ValueBounds::ANY))
                    .verdict(&subject(&item, Some(&details))),
                Verdict::Matches,
                "{text:?} should match the base damage line"
            );
        }
    }

    #[test]
    fn a_missing_record_leaves_a_stat_miss_unresolved() {
        let item = sword();
        let mut details = sword_details();
        details.blocks[2].lines.clear();
        details.unrendered.push(Unrendered {
            variable: "Suffix".into(),
            reason: UnrenderedReason::MissingRecord(
                "records/items/lootaffixes/suffix/embers.dbr".into(),
            ),
        });
        let verdict = |criterion| stat_query(criterion).verdict(&subject(&item, Some(&details)));
        assert_eq!(
            verdict(contains("cold", ValueBounds::ANY)),
            Verdict::Unresolved
        );
        assert_eq!(
            verdict(contains("fire", ValueBounds::ANY)),
            Verdict::Matches
        );
        assert_eq!(
            verdict(Criterion::AffixStat {
                text: "cold".into(),
                bounds: ValueBounds::ANY,
            }),
            Verdict::Unresolved
        );
        let without_details = stat_query(contains("fire", ValueBounds::ANY));
        assert_eq!(
            without_details.verdict(&subject(&item, None)),
            Verdict::Unresolved
        );
    }

    #[test]
    fn inert_criteria_constrain_nothing() {
        assert!(Criterion::default().is_inert());
        assert!(Scope::AffixName.blank().is_inert());
        assert!(Criterion::HasAffix { text: "  ".into() }.is_inert());
        assert!(!contains("", at_least(1.0)).is_inert());
        let query = Query {
            criteria: vec![Criterion::default(), Scope::AffixStat.blank()],
            ..Query::default()
        };
        assert!(query.is_empty());
        assert!(!query.needs_details());
        assert!(stat_query(contains("x", ValueBounds::ANY)).needs_details());
        assert!(
            !stat_query(Criterion::HasAffix { text: "x".into() }).needs_details(),
            "affix names come from the facts, not the stat body"
        );
    }

    #[test]
    fn a_criterion_changes_scope_keeping_its_text() {
        let criterion = contains("fire", at_least(5.0)).with_scope(Scope::AffixStat);
        assert_eq!(
            criterion,
            Criterion::AffixStat {
                text: "fire".into(),
                bounds: at_least(5.0),
            }
        );
        assert_eq!(criterion.scope(), Scope::AffixStat);
        let named = criterion.with_scope(Scope::AffixName);
        assert_eq!(
            named,
            Criterion::HasAffix {
                text: "fire".into()
            }
        );
        assert_eq!(
            named.with_scope(Scope::AnyStat),
            contains("fire", ValueBounds::ANY),
            "the window does not survive a scope without one"
        );
    }

    #[test]
    fn requirement_caps_pass_at_the_boundary_and_when_absent() {
        let item = sword();
        let details = sword_details();
        let cap = |requirement: Requirement, cap: u32| {
            let mut query = Query::default();
            *query.requirements.slot(requirement) = Some(cap);
            assert!(query.needs_details());
            query.verdict(&subject(&item, Some(&details)))
        };
        assert_eq!(cap(Requirement::Physique, 141), Verdict::Matches);
        assert_eq!(cap(Requirement::Physique, 140), Verdict::Excluded);
        assert_eq!(cap(Requirement::Level, 24), Verdict::Matches);
        assert_eq!(cap(Requirement::Level, 23), Verdict::Excluded);
        assert_eq!(cap(Requirement::Spirit, 1), Verdict::Matches);
        let mut query = Query::default();
        query.requirements.level = Some(30);
        let unknown_base = Subject {
            base: BaseEvidence::Unresolved,
            ..subject(&item, Some(&details))
        };
        assert_eq!(query.verdict(&unknown_base), Verdict::Unresolved);
        query.requirements.level = Some(10);
        assert_eq!(
            query.verdict(&unknown_base),
            Verdict::Excluded,
            "a stated requirement above the cap fails whatever else is unknown"
        );
        assert_eq!(query.verdict(&subject(&item, None)), Verdict::Unresolved);
    }

    #[test]
    fn the_set_filter_reads_the_set_name() {
        let item = sword();
        let details = sword_details();
        let verdict = |set: SetFilter| {
            Query {
                set,
                ..Query::default()
            }
            .verdict(&subject(&item, Some(&details)))
        };
        assert_eq!(verdict(SetFilter::Member), Verdict::Matches);
        assert_eq!(
            verdict(SetFilter::Named {
                text: "OLYMPUS".into()
            }),
            Verdict::Matches
        );
        assert_eq!(
            verdict(SetFilter::Named {
                text: "titans".into()
            }),
            Verdict::Excluded
        );
        let mut loose = sword_details();
        loose.set = None;
        let query = Query {
            set: SetFilter::Member,
            ..Query::default()
        };
        assert_eq!(
            query.verdict(&subject(&item, Some(&loose))),
            Verdict::Excluded
        );
        assert_eq!(
            query.verdict(&Subject {
                base: BaseEvidence::Unresolved,
                ..subject(&item, Some(&loose))
            }),
            Verdict::Unresolved
        );
    }

    #[test]
    fn rarity_and_category_match_exactly_and_need_a_known_base() {
        let item = sword();
        let epic_sword = Subject {
            facets: Facets::classify(
                &item,
                known(Some(Rarity::Epic), Bucket::OneHanded),
                AffixEvidence::Absent,
                AffixEvidence::Absent,
                &AscensionTable::Absent,
            ),
            ..subject(&item, None)
        };
        let rarity = |rarity| Query {
            rarity: Some(rarity),
            ..Query::default()
        };
        assert_eq!(rarity(Rarity::Epic).verdict(&epic_sword), Verdict::Matches);
        assert_eq!(rarity(Rarity::Rare).verdict(&epic_sword), Verdict::Excluded);
        let unknown = Subject {
            base: BaseEvidence::Unresolved,
            facets: facets(
                MonsterInfrequent::Unresolved,
                DoubleRare::Unresolved,
                Ascension::Unresolved,
            ),
            ..subject(&item, None)
        };
        assert_eq!(rarity(Rarity::Epic).verdict(&unknown), Verdict::Unresolved);

        let category = |category| Query {
            category,
            ..Query::default()
        };
        assert_eq!(
            category(CategoryFilter::Bucket {
                bucket: Bucket::OneHanded
            })
            .verdict(&epic_sword),
            Verdict::Matches
        );
        assert_eq!(
            category(CategoryFilter::Group {
                group: Group::Weapons
            })
            .verdict(&epic_sword),
            Verdict::Matches
        );
        assert_eq!(
            category(CategoryFilter::Group {
                group: Group::Armor
            })
            .verdict(&epic_sword),
            Verdict::Excluded
        );
        assert_eq!(
            category(CategoryFilter::Bucket {
                bucket: Bucket::Misc
            })
            .verdict(&unknown),
            Verdict::Unresolved,
            "an unknown record is not filed under Miscellaneous"
        );
    }

    #[test]
    fn the_socket_filter_reads_the_items_own_component_field() {
        let mut socketed = sword();
        socketed.relic_name = "records/items/materia/compa_sealnight.dbr".into();
        let plain = sword();
        let verdict = |item: &Item, socket| {
            Query {
                socket,
                ..Query::default()
            }
            .verdict(&subject(item, None))
        };
        assert_eq!(verdict(&socketed, SocketFilter::Socketed), Verdict::Matches);
        assert_eq!(
            verdict(&socketed, SocketFilter::Unsocketed),
            Verdict::Excluded
        );
        assert_eq!(verdict(&plain, SocketFilter::Unsocketed), Verdict::Matches);
        assert_eq!(verdict(&plain, SocketFilter::Socketed), Verdict::Excluded);
    }

    #[test]
    fn the_query_is_a_conjunction() {
        let item = sword();
        let details = sword_details();
        let mut query = Query {
            name: "broadsword".into(),
            socket: SocketFilter::Unsocketed,
            criteria: vec![contains("fire", ValueBounds::ANY)],
            ..Query::default()
        };
        assert_eq!(
            query.verdict(&subject(&item, Some(&details))),
            Verdict::Matches
        );
        query.socket = SocketFilter::Socketed;
        assert_eq!(
            query.verdict(&subject(&item, Some(&details))),
            Verdict::Excluded
        );
    }

    #[test]
    fn stat_templates_replace_numbers_with_hashes() {
        assert_eq!(
            stat_template("12-31 Physical Damage"),
            "#-# Physical Damage"
        );
        assert_eq!(
            stat_template("+24% Pierce Resistance"),
            "+#% Pierce Resistance"
        );
        assert_eq!(stat_template("-15% Something"), "#% Something");
        assert_eq!(stat_template("over 3.0 Seconds"), "over # Seconds");
        assert_eq!(
            stat_template("+134.0 Health Regenerated"),
            "+# Health Regenerated"
        );
        assert_eq!(
            stat_template("Grants Skill: Chain Lightning (50% Chance on Critical Attack)"),
            "Grants Skill: Chain Lightning (#% Chance on Critical Attack)"
        );
        assert_eq!(stat_template("Speed: Very Slow"), "Speed: Very Slow");
        assert_eq!(stat_template("ends with 7."), "ends with #.");
    }

    #[test]
    fn the_largest_value_is_read_from_the_rendered_numbers() {
        assert_eq!(
            largest_value(&stat("12-31 Physical Damage", &[12.0, 31.0])),
            Some(31.0)
        );
        assert_eq!(largest_value(&stat("Speed: Very Slow", &[])), None);
        assert!(is_searchable(&stat("+40% Cold Damage", &[40.0])));
        assert!(!is_searchable(&line(
            LineKind::SkillDescription,
            "Zaps the target with lightning.",
            &[]
        )));
    }

    #[test]
    fn ranks_sort_by_key_with_a_name_tiebreak() {
        let epic = Item::default();
        let rank = |name: &str, rarity: Option<Rarity>, level: Option<u32>, bucket: Bucket| {
            let details = ItemDetails {
                requirements: level
                    .map_or_else(Vec::new, |level| vec![(Requirement::Level, level)]),
                ..ItemDetails::default()
            };
            SortRank::of(&Subject {
                item: &epic,
                name,
                prefix: AffixName::Absent,
                suffix: AffixName::Absent,
                base: known(rarity, bucket),
                facets: Facets::classify(
                    &epic,
                    known(rarity, bucket),
                    AffixEvidence::Absent,
                    AffixEvidence::Absent,
                    &AscensionTable::Absent,
                ),
                details: Some(&details),
            })
        };
        let bravo = rank("Bravo", Some(Rarity::Rare), Some(30), Bucket::Head);
        let alpha = rank("alpha", Some(Rarity::Legendary), None, Bucket::TwoHanded);
        let charlie = rank("Charlie", Some(Rarity::Rare), Some(10), Bucket::Ring);
        assert_eq!(alpha.level(), None);
        assert_eq!(bravo.level(), Some(30));
        let order = |key: SortKey| {
            let mut ranks = [&bravo, &alpha, &charlie];
            ranks.sort_by(|a, b| a.compare(b, key));
            ranks
                .iter()
                .map(|rank| rank.name.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(order(SortKey::Name), ["alpha", "bravo", "charlie"]);
        assert_eq!(order(SortKey::Rarity), ["bravo", "charlie", "alpha"]);
        assert_eq!(order(SortKey::Level), ["alpha", "charlie", "bravo"]);
        assert_eq!(order(SortKey::Category), ["alpha", "bravo", "charlie"]);
        assert_eq!(SortKey::default(), SortKey::Name);
        assert_eq!(SortKey::ALL.len(), 4);
    }

    #[test]
    fn rarity_orders_by_tier() {
        assert!(Rarity::Common < Rarity::Magical);
        assert!(Rarity::Magical < Rarity::Rare);
        assert!(Rarity::Rare < Rarity::Epic);
        assert!(Rarity::Epic < Rarity::Legendary);
    }

    #[test]
    fn the_query_round_trips_through_json_and_defaults_missing_fields() {
        let mut query = Query {
            name: "sword".into(),
            criteria: vec![
                contains("fire", at_least(10.0)),
                Criterion::HasAffix {
                    text: "sharp".into(),
                },
            ],
            set: SetFilter::Named {
                text: "olympus".into(),
            },
            rarity: Some(Rarity::Epic),
            category: CategoryFilter::Bucket {
                bucket: Bucket::OneHanded,
            },
            socket: SocketFilter::Unsocketed,
            monster_infrequent: Constraint::Required,
            ..Query::default()
        };
        query.requirements.level = Some(50);
        let json = serde_json::to_string(&query).unwrap();
        assert!(json.contains("\"kind\":\"statContains\""), "{json}");
        assert!(json.contains("\"rarity\":\"epic\""), "{json}");
        assert_eq!(serde_json::from_str::<Query>(&json).unwrap(), query);
        let partial: Query = serde_json::from_str(r#"{"name":"x","rarity":"rare"}"#).unwrap();
        assert_eq!(partial.name, "x");
        assert_eq!(partial.rarity, Some(Rarity::Rare));
        assert_eq!(partial.criteria, Vec::new());
        assert_eq!(partial.category, CategoryFilter::Any);
    }

    fn fixture() -> GameData {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(
            "records/items/gearweapons/swords1h/broadsword.dbr",
            "WeaponMelee_Sword",
            &[
                ("Class", Values::Strings(&["WeaponMelee_Sword"])),
                ("itemNameTag", Values::Strings(&["tagSword"])),
                ("itemClassification", Values::Strings(&["Epic"])),
                ("offensivePhysicalMin", Values::Floats(&[12.0])),
                ("offensivePhysicalMax", Values::Floats(&[31.0])),
                ("levelRequirement", Values::Ints(&[24])),
            ],
        );
        builder.record(
            "records/items/lootaffixes/prefix/sharp.dbr",
            "LootRandomizer",
            &[
                ("Class", Values::Strings(&["LootRandomizer"])),
                ("lootRandomizerName", Values::Strings(&["tagSharp"])),
                ("itemClassification", Values::Strings(&["Rare"])),
                ("offensiveFireMin", Values::Floats(&[10.0])),
                ("levelRequirement", Values::Ints(&[30])),
            ],
        );
        let mut text = TextDb::new();
        text.add_file(
            b"tagSword=Broadsword\ntagSharp=Sharp\n\
              DamageBasePhysical={%t0} Physical Damage\nDamageFire={%t0} Fire Damage\n\
              DamageSingleFormat={%.0f0}\nDamageRangeFormat={%.0f0}-{%.0f1}\n\
              MeetsRequirement=Required {%s0}: {%.0f1}\nLevelRequirement=Player Level\n",
        );
        GameData::from_parts(
            vec![ArzFile::parse(builder.build(), ArzDialect::grim_dawn()).unwrap()],
            text,
            Vec::new(),
        )
    }

    #[test]
    fn the_headless_path_resolves_a_subject_from_the_database() {
        let game = fixture();
        let item = Item {
            base_name: "records/items/gearweapons/swords1h/broadsword.dbr".into(),
            prefix_name: "records/items/lootaffixes/prefix/sharp.dbr".into(),
            ..Item::default()
        };
        let resolved = Resolved::of(&game, &AscensionTable::Absent, &item);
        assert_eq!(resolved.name(), "Sharp Broadsword");
        let subject = resolved.subject(&item);
        assert_eq!(subject.prefix, AffixName::Named("Sharp"));
        assert_eq!(subject.suffix, AffixName::Absent);
        assert_eq!(subject.base, known(Some(Rarity::Epic), Bucket::OneHanded));
        assert_eq!(
            resolved.details().requirements,
            vec![(Requirement::Level, 30)],
            "the prefix's level gate raises the item's"
        );
        let verdict = |query: &Query| query.verdict_of(&game, &AscensionTable::Absent, &item);
        assert_eq!(
            verdict(&stat_query(contains("fire damage", at_least(10.0)))),
            Verdict::Matches
        );
        assert_eq!(
            verdict(&stat_query(contains("physical", at_least(32.0)))),
            Verdict::Excluded
        );
        assert_eq!(
            verdict(&Query {
                rarity: Some(Rarity::Epic),
                ..Query::default()
            }),
            Verdict::Matches
        );
        let mut capped = Query::default();
        capped.requirements.level = Some(29);
        assert_eq!(verdict(&capped), Verdict::Excluded);
        assert_eq!(SortRank::of(&subject).level(), Some(30));

        let unknown = Item {
            base_name: "records/items/nowhere.dbr".into(),
            ..Item::default()
        };
        let resolved = Resolved::of(&game, &AscensionTable::Absent, &unknown);
        assert_eq!(resolved.name(), "records/items/nowhere.dbr");
        assert_eq!(resolved.subject(&unknown).base, BaseEvidence::Unresolved);
        assert_eq!(
            stat_query(contains("fire", ValueBounds::ANY)).verdict_of(
                &game,
                &AscensionTable::Absent,
                &unknown
            ),
            Verdict::Unresolved
        );
    }
}
