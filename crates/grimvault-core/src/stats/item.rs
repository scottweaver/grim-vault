//! Per-item assembly: the records an [`Item`] names, rendered and laid
//! out in the game's tooltip order — base, prefix, suffix, transmute
//! modifier, component and its completion bonus, augment, ascendant
//! bonus — with the item's requirements and set. Grim Dawn specific
//! in every rule (which item field is which block, whose
//! `attributeScalePercent` scales what, how requirements are computed).

use univault_engine::arz::DbRecord;
use univault_engine::format::{Arg, eval_equation, parse_format};
use univault_engine::ids::RecordId;

use super::vocabulary::{self, Emphasis, Section, tags};
use super::{
    LineKind, RecordStats, Requirement, Scale, SkillLevel, StatLine, Unrendered, UnrenderedReason,
    count, whole,
};
use crate::gamedata::GameData;
use crate::item::Item;

/// The tooltip body of one item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemDetails {
    /// One block per record the item names, in display order; a record
    /// no database layer has still yields its block, empty, with the
    /// gap in `unrendered`.
    pub blocks: Vec<Block>,
    /// The item's effective requirements, typed and sorted.
    pub requirements: Vec<(Requirement, u32)>,
    /// The requirements as the game phrases them ("Required Player
    /// Level: 65").
    pub requirement_lines: Vec<StatLine>,
    pub set: Option<SetInfo>,
    /// Every gap across every block.
    pub unrendered: Vec<Unrendered>,
}

/// The lines one of the item's records contributes.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub source: BlockSource,
    pub record: RecordId,
    /// The record's own name where the game shows one (a component, an
    /// augment) or the game's heading for the block (an ascendant
    /// bonus).
    pub title: Option<String>,
    pub flavor: Option<String>,
    pub lines: Vec<StatLine>,
}

/// Which item field a block came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockSource {
    Base,
    Prefix,
    Suffix,
    Modifier,
    Component,
    CompletionBonus,
    Augment,
    Ascendant,
    AscendantTwoHanded,
}

impl BlockSource {
    const ALL: [Self; 9] = [
        Self::Base,
        Self::Prefix,
        Self::Suffix,
        Self::Modifier,
        Self::Component,
        Self::CompletionBonus,
        Self::Augment,
        Self::Ascendant,
        Self::AscendantTwoHanded,
    ];

    fn field(self, item: &Item) -> &str {
        match self {
            Self::Base => &item.base_name,
            Self::Prefix => &item.prefix_name,
            Self::Suffix => &item.suffix_name,
            Self::Modifier => &item.modifier_name,
            Self::Component => &item.relic_name,
            Self::CompletionBonus => &item.relic_bonus,
            Self::Augment => &item.augment_name,
            Self::Ascendant => &item.ascendant_record,
            Self::AscendantTwoHanded => &item.ascendant_record_2h,
        }
    }

    /// Whether the base item's `attributeScalePercent` applies to the
    /// block: its two rolled affixes only — the base's own numbers are
    /// authored as displayed (an item pairing `+220%` physical with
    /// `+220%` trauma would otherwise show `+220%` and `+396%`).
    fn takes_item_scale(self) -> bool {
        match self {
            Self::Prefix | Self::Suffix => true,
            Self::Base
            | Self::Modifier
            | Self::Component
            | Self::CompletionBonus
            | Self::Augment
            | Self::Ascendant
            | Self::AscendantTwoHanded => false,
        }
    }

    /// The records whose level requirement gates the item.
    fn gates_level(self) -> bool {
        match self {
            Self::Base
            | Self::Prefix
            | Self::Suffix
            | Self::Component
            | Self::Augment
            | Self::Ascendant
            | Self::AscendantTwoHanded => true,
            Self::Modifier | Self::CompletionBonus => false,
        }
    }

    /// The records whose stats count towards the cost formulas'
    /// `totalAttCount`.
    fn counts_attributes(self) -> bool {
        matches!(self, Self::Base | Self::Prefix | Self::Suffix)
    }

    /// A short label for a block heading.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Prefix => "Prefix",
            Self::Suffix => "Suffix",
            Self::Modifier => "Modifier",
            Self::Component => "Component",
            Self::CompletionBonus => "Completion bonus",
            Self::Augment => "Augment",
            Self::Ascendant => "Ascended bonus",
            Self::AscendantTwoHanded => "Ascended bonus (two-handed)",
        }
    }
}

/// The set an item belongs to: its name, every member's name, and the
/// bonus each piece count adds.
#[derive(Clone, Debug, PartialEq)]
pub struct SetInfo {
    pub record: RecordId,
    pub name: String,
    pub members: Vec<String>,
    pub tiers: Vec<SetTier>,
}

/// The lines a set grants once `pieces` of it are worn, beyond what
/// fewer pieces already granted.
#[derive(Clone, Debug, PartialEq)]
pub struct SetTier {
    pub pieces: u32,
    pub lines: Vec<StatLine>,
}

/// Assembles the tooltip body of `item` from the cached record stats.
#[must_use]
pub fn item_details(game: &GameData, item: &Item) -> ItemDetails {
    let base = RecordId::parse(item.base_name.clone());
    let base_record = base.as_ref().and_then(|id| game.record(id)?.ok());
    let scale = base_record
        .as_ref()
        .and_then(item_scale)
        .unwrap_or(Scale::NONE);
    let mut details = ItemDetails::default();
    let mut rendered: Vec<(BlockSource, std::sync::Arc<RecordStats>)> = Vec::new();
    for source in BlockSource::ALL {
        let Some(id) = RecordId::parse(source.field(item).to_string()) else {
            continue;
        };
        let scale = if source.takes_item_scale() {
            scale
        } else {
            Scale::NONE
        };
        let Some(stats) = game.record_stats(&id, SkillLevel::ONE, scale) else {
            details.unrendered.push(Unrendered {
                variable: source.label().to_string(),
                reason: UnrenderedReason::MissingRecord(id.as_str().to_string()),
            });
            details.blocks.push(Block {
                source,
                record: id,
                title: None,
                flavor: None,
                lines: Vec::new(),
            });
            continue;
        };
        details.unrendered.extend(stats.unrendered.iter().cloned());
        details.blocks.push(Block {
            source,
            title: block_title(game, source, &id),
            record: id,
            flavor: stats.flavor.clone(),
            lines: stats.lines.clone(),
        });
        rendered.push((source, stats));
    }
    details.requirements = merged_requirements(game, base_record.as_ref(), &rendered);
    details.requirement_lines = details
        .requirements
        .iter()
        .map(|(requirement, value)| requirement_line(game, *requirement, *value))
        .collect();
    details.set = rendered
        .iter()
        .find(|(source, _)| *source == BlockSource::Base)
        .and_then(|(_, stats)| stats.set.as_ref())
        .and_then(|set_id| set_info(game, set_id));
    details
}

/// The item's effective requirements: level from every gating record,
/// attributes from explicit values or the base class's cost formulas.
#[must_use]
pub fn requirements(game: &GameData, item: &Item) -> Vec<(Requirement, u32)> {
    item_details(game, item).requirements
}

/// The display name of the set the item's base record belongs to.
#[must_use]
pub fn set_name(game: &GameData, item: &Item) -> Option<String> {
    let base = RecordId::parse(item.base_name.clone())?;
    let stats = game.record_stats(&base, SkillLevel::ONE, Scale::NONE)?;
    let set_record = game.record(stats.set.as_ref()?)?.ok()?;
    set_record
        .string("setName")
        .and_then(|tag| game.tag_text(tag))
        .map(|label| parse_format(label).format(&[]))
}

fn item_scale(record: &DbRecord) -> Option<Scale> {
    record
        .float("attributeScalePercent")
        .or_else(|| record.integer("attributeScalePercent").map(whole))
        .map(scale_percent)
}

/// Percentages in the database are whole numbers between 0 and 100.
#[allow(
    clippy::cast_possible_truncation,
    reason = "clamped to a percentage before the cast"
)]
fn scale_percent(percent: f32) -> Scale {
    Scale::percent(percent.round().clamp(-1000.0, 1000.0) as i32)
}

fn block_title(game: &GameData, source: BlockSource, id: &RecordId) -> Option<String> {
    match source {
        BlockSource::Component | BlockSource::Augment => {
            game.item_info(id)?.ok().map(|info| info.name)
        }
        BlockSource::Ascendant | BlockSource::AscendantTwoHanded => game
            .tag_text(tags::ASCENDED_HEADING)
            .map(|label| parse_format(label).format(&[])),
        BlockSource::Base
        | BlockSource::Prefix
        | BlockSource::Suffix
        | BlockSource::Modifier
        | BlockSource::CompletionBonus => None,
    }
}

fn merged_requirements(
    game: &GameData,
    base: Option<&DbRecord>,
    rendered: &[(BlockSource, std::sync::Arc<RecordStats>)],
) -> Vec<(Requirement, u32)> {
    let mut merged: Vec<(Requirement, u32)> = Vec::new();
    for (source, stats) in rendered {
        if source.gates_level()
            && let Some(level) = stats.level_requirement
        {
            merge_max(&mut merged, Requirement::Level, level);
        }
        for (requirement, value) in &stats.attribute_requirements {
            merge_max(&mut merged, *requirement, *value);
        }
    }
    let explicit_attributes = merged
        .iter()
        .any(|(requirement, _)| *requirement != Requirement::Level);
    if let Some(base) = base
        && !explicit_attributes
    {
        let total_att_count: u32 = rendered
            .iter()
            .filter(|(source, _)| source.counts_attributes())
            .map(|(_, stats)| stats.attribute_count)
            .sum();
        for (requirement, value) in formula_requirements(game, base, total_att_count) {
            merge_max(&mut merged, requirement, value);
        }
    }
    merged.sort_by_key(|(requirement, _)| *requirement);
    merged
}

/// The strictest of two statements of the same requirement wins; a
/// zero says nothing.
fn merge_max(merged: &mut Vec<(Requirement, u32)>, requirement: Requirement, value: u32) {
    if value == 0 {
        return;
    }
    match merged.iter_mut().find(|(known, _)| *known == requirement) {
        Some((_, known)) => *known = (*known).max(value),
        None => merged.push((requirement, value)),
    }
}

/// The attribute requirements the base class's cost formulas yield at
/// the item's level: `<class><Attribute>Equation` in the record's
/// `itemCostName` (or the default formulas), evaluated with
/// `itemLevel` and `totalAttCount`, rounded up as the engine does.
fn formula_requirements(
    game: &GameData,
    base: &DbRecord,
    total_att_count: u32,
) -> Vec<(Requirement, u32)> {
    let Some(prefix) = vocabulary::requirement_equation_prefix(&base.record_type) else {
        return Vec::new();
    };
    let Some(item_level) = base.integer("itemLevel").filter(|level| *level > 0) else {
        return Vec::new();
    };
    let cost_id = base
        .string("itemCostName")
        .filter(|id| !id.is_empty())
        .unwrap_or(vocabulary::DEFAULT_COST_RECORD);
    let Some(cost) = RecordId::parse(cost_id.to_string()).and_then(|id| game.record(&id)?.ok())
    else {
        return Vec::new();
    };
    let lookup = |name: &str| match name {
        "itemLevel" => Some(f64::from(item_level)),
        "totalAttCount" => Some(f64::from(total_att_count)),
        _ => None,
    };
    Requirement::EXPLICIT
        .iter()
        .filter_map(|requirement| {
            let attribute = requirement.equation_attribute()?;
            let equation = cost.string(&format!("{prefix}{attribute}Equation"))?;
            let value = eval_equation(equation, &lookup)?;
            requirement_value(value).map(|value| (*requirement, value))
        })
        .collect()
}

/// A requirement equation's result, rounded up as the engine does.
/// Requirements are small positive integers; anything else is an
/// equation this app cannot follow, and is dropped.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "range-checked to 1..1e6 before the cast"
)]
fn requirement_value(value: f64) -> Option<u32> {
    let value = value.ceil();
    (value.is_finite() && value > 0.0 && value < 1.0e6).then_some(value as u32)
}

/// "Required Player Level: 65" through `MeetsRequirement`.
fn requirement_line(game: &GameData, requirement: Requirement, value: u32) -> StatLine {
    let name = game.tag_text(requirement.tag()).map_or_else(
        || requirement.tag().to_string(),
        |label| parse_format(label).format(&[]),
    );
    let text = game.tag_text(tags::REQUIREMENT).map_or_else(
        || format!("Required {name}: {value}"),
        |label| parse_format(label).format(&[Arg::Text(name.clone()), Arg::Number(count(value))]),
    );
    StatLine {
        kind: LineKind::Requirement(requirement),
        section: Section::Requirement,
        emphasis: Emphasis::Muted,
        text,
        values: vec![count(value)],
    }
}

/// The set a set record describes: its name, its members' names, and
/// the bonuses each piece count adds; `None` when the record is
/// missing or has no name the text tables know.
#[must_use]
pub fn set_info(game: &GameData, set_id: &RecordId) -> Option<SetInfo> {
    let set = game.record(set_id)?.ok()?;
    let name = set
        .string("setName")
        .and_then(|tag| game.tag_text(tag))
        .map(|label| parse_format(label).format(&[]))?;
    let members: Vec<String> = match set.variable("setMembers").map(|v| &v.values) {
        Some(univault_engine::arz::DbValues::Strings(ids)) => ids
            .iter()
            .filter_map(|id| RecordId::parse(id.clone()))
            .map(|id| {
                game.item_info(&id)
                    .and_then(Result::ok)
                    .map_or_else(|| id.file_stem().to_string(), |info| info.name)
            })
            .collect(),
        Some(_) | None => Vec::new(),
    };
    let piece_count = u32::try_from(members.len()).unwrap_or(u32::MAX);
    let mut tiers = Vec::new();
    let mut previous: Vec<StatLine> = Vec::new();
    for pieces in 1..=piece_count {
        let Some(level) = SkillLevel::new(pieces) else {
            break;
        };
        let Some(stats) = game.record_stats(set_id, level, Scale::NONE) else {
            break;
        };
        let added: Vec<StatLine> = stats
            .lines
            .iter()
            .filter(|line| !previous.contains(line))
            .cloned()
            .collect();
        if !added.is_empty() {
            tiers.push(SetTier {
                pieces,
                lines: added,
            });
        }
        previous.clone_from(&stats.lines);
    }
    Some(SetInfo {
        record: set_id.clone(),
        name,
        members,
        tiers,
    })
}
