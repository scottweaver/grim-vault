//! The record renderer: one database record in, its display lines out.
//! Groups the record's variables by stat family, then composes each
//! group's lines the way the game's own tags dictate — a chance
//! prefix, an amount the label takes as text or formats itself, a
//! duration suffix — and follows the record's references for granted
//! skills, skill modifiers and pet bonuses. Everything game-specific
//! (which variables exist, which tag names what) comes from
//! [`super::vocabulary`]; this file only knows the engine's grammar of
//! `Min` / `Max` / `Chance` / `Duration` / `Modifier` parts and the
//! `{%…}` templates.
//!
//! Ported in shape from tq-univault's `stats::render` (MIT OR
//! Apache-2.0, same author; itself a port of `TQVaultAE`'s
//! `ItemProvider`, MIT), rebuilt on Grim Dawn's tag grammar: labels
//! carry their own placeholders (`{%t0}` for a pre-formatted amount,
//! `{%.0f0}` for a number) and every prefix and suffix tag brings its
//! own spacing, so a line is a plain concatenation.

use std::collections::BTreeMap;

use univault_engine::arz::{DbRecord, DbValues, DbVariable};
use univault_engine::format::{Arg, FormatSpec, eval_equation, parse_format};
use univault_engine::ids::RecordId;

use super::vocabulary::{
    self, Attribute, Effect, Emphasis, Family, Part, RecordKind, Section, SkillStatShape, tags,
};
use super::{
    LineKind, Requirement, Scale, SkillLevel, StatLine, Unrendered, UnrenderedReason, count, whole,
};
use crate::gamedata::GameData;

/// Rendered lines of one record plus everything the item-level
/// assembly needs from it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordStats {
    pub lines: Vec<StatLine>,
    pub unrendered: Vec<Unrendered>,
    /// The record's `itemText` flavor line, localized.
    pub flavor: Option<String>,
    pub level_requirement: Option<u32>,
    /// Explicit `strengthRequirement` / `dexterityRequirement` /
    /// `intelligenceRequirement` values (rare; the cost formulas
    /// usually decide).
    pub attribute_requirements: Vec<(Requirement, u32)>,
    pub item_level: Option<u32>,
    /// The record's contribution to the cost formulas' `totalAttCount`.
    pub attribute_count: u32,
    /// `itemSetName`, when the record belongs to a set.
    pub set: Option<RecordId>,
}

/// How deep granted skill → buff → pet references are followed.
const MAX_DEPTH: u32 = 3;

const INDENT: &str = "    ";

pub struct Renderer<'a> {
    game: &'a GameData,
}

#[derive(Clone, Copy)]
struct Ctx {
    kind: RecordKind,
    level: usize,
    scale: f32,
    depth: u32,
    /// Overrides the section of every line rendered (nested skill and
    /// pet-bonus lines belong to their heading's section).
    section: Option<Section>,
}

struct Group<'r> {
    effect: Effect,
    parts: Vec<(Part, f32)>,
    variables: Vec<&'r DbVariable>,
}

impl Group<'_> {
    fn part(&self, wanted: Part) -> Option<f32> {
        self.parts
            .iter()
            .find(|(part, _)| *part == wanted)
            .map(|(_, value)| *value)
    }

    fn has(&self, wanted: Part) -> bool {
        self.part(wanted).is_some()
    }
}

/// A line under construction: the parts joined at the end so prefixes
/// and suffixes keep the spacing their tags carry.
struct Composed {
    effect: Effect,
    text: String,
    values: Vec<f32>,
    emphasis: Emphasis,
    global: bool,
}

impl<'a> Renderer<'a> {
    #[must_use]
    pub fn new(game: &'a GameData) -> Self {
        Self { game }
    }

    /// Renders `record` at skill level `level` (its arrays indexed by
    /// level, capped to their length) with `scale` applied to the
    /// effects an item's `attributeScalePercent` multiplies.
    #[must_use]
    pub fn render(&self, record: &DbRecord, level: SkillLevel, scale: Scale) -> RecordStats {
        let ctx = Ctx {
            kind: RecordKind::of_class(&record.record_type),
            level: level.index(),
            scale: scale.factor(),
            depth: 0,
            section: None,
        };
        let mut stats = RecordStats {
            flavor: self.translated(record, "itemText"),
            level_requirement: integer_at(record, "levelRequirement", ctx.level),
            attribute_requirements: Requirement::EXPLICIT
                .iter()
                .filter_map(|requirement| {
                    integer_at(record, requirement.variable(), ctx.level)
                        .filter(|value| *value > 0)
                        .map(|value| (*requirement, value))
                })
                .collect(),
            item_level: integer_at(record, "itemLevel", 0),
            set: record
                .string("itemSetName")
                .and_then(|id| RecordId::parse(id.to_string())),
            ..RecordStats::default()
        };
        self.render_into(record, ctx, &mut stats);
        stats.lines.sort_by_key(|line| line.section);
        stats
    }

    fn render_into(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        let groups = group_variables(record, ctx, out);
        out.attribute_count += attribute_count(&groups);
        let mut lines: Vec<(Section, Composed)> = Vec::new();
        for group in &groups {
            self.render_group(group, ctx, &mut lines, out);
        }
        self.attach_global_headers(record, ctx, &mut lines);
        for (section, composed) in lines {
            out.lines.push(StatLine {
                kind: LineKind::Effect(composed.effect),
                section: ctx.section.unwrap_or(section),
                emphasis: composed.emphasis,
                text: composed.text,
                values: composed.values,
            });
        }
        self.render_specials(record, ctx, &groups, out);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one step per part of the game's damage grammar, in the order the line reads"
    )]
    fn render_damage(
        &self,
        group: &Group<'_>,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        let effect = group.effect;
        let scaled = |value: f32| scale_value(effect, value, ctx.scale);
        let global = group.has(Part::Global);
        let xor = group.has(Part::Xor);
        let min = group
            .part(Part::Min)
            .or_else(|| group.part(Part::Value))
            .or_else(|| group.part(Part::DrainMin));
        let max = group.part(Part::Max).or_else(|| group.part(Part::DrainMax));
        let duration = (group.part(Part::DurationMin), group.part(Part::DurationMax));
        let chance = group.part(Part::Chance);
        let retaliation = matches!(
            effect.family(),
            Family::Retaliation | Family::RetaliationDuration
        );

        if let Some(label_tag) = vocabulary::value_tag(effect, ctx.kind)
            && let Some(min) = min.or(max)
        {
            let Some(label) = self.game.tag_text(&label_tag) else {
                out.unrendered.push(Unrendered {
                    variable: effect.name().to_string(),
                    reason: UnrenderedReason::MissingTag(label_tag),
                });
                return;
            };
            let mut values = Vec::new();
            let mut parts: Vec<String> = Vec::new();
            let own_chance = chance.filter(|_| !global || xor);
            if let Some(chance) = own_chance {
                parts.push(self.fixed(tags::CHANCE_OF, &[Arg::Number(chance)]));
                values.push(chance);
            }
            let max = max.filter(|max| (*max - min).abs() > f32::EPSILON);
            let spec = parse_format(label);
            if vocabulary::is_influence(effect) {
                if spec.has_args() {
                    let (single, range) = if retaliation {
                        (tags::RETALIATION_FOR_SINGLE, tags::RETALIATION_FOR_RANGE)
                    } else {
                        (tags::FOR_SINGLE, tags::FOR_RANGE)
                    };
                    let amount = self.amount(single, range, min, max);
                    parts.push(spec.format(&[Arg::Text(amount)]));
                } else {
                    parts.push(spec.format(&[]));
                }
                values.push(min);
                values.extend(max);
            } else {
                let multiplier = match duration.0 {
                    Some(seconds) if vocabulary::scales_with_duration(effect) => seconds,
                    _ => 1.0,
                };
                let (low, high) = (
                    scaled(min) * multiplier,
                    max.map(|max| scaled(max) * multiplier),
                );
                let amount = self.amount(tags::SINGLE, tags::RANGE, low, high);
                if spec.has_args() {
                    parts.push(spec.format(&[Arg::Text(amount)]));
                } else {
                    parts.push(amount);
                    parts.push(spec.format(&[]));
                }
                values.push(low);
                values.extend(high);
                if let Some(seconds) = duration.0 {
                    let (single, range) = if vocabulary::scales_with_duration(effect) {
                        (tags::OVER_SINGLE, tags::OVER_RANGE)
                    } else {
                        (tags::FOR_SINGLE, tags::FOR_RANGE)
                    };
                    let longest = duration
                        .1
                        .filter(|max| (*max - seconds).abs() > f32::EPSILON);
                    parts.push(self.amount(single, range, seconds, longest));
                    values.push(seconds);
                    values.extend(longest);
                }
                if let Some(ratio) = group.part(Part::DamageRatio) {
                    parts.push(self.fixed(tags::MANA_BURN_RATIO, &[Arg::Number(ratio)]));
                    values.push(ratio);
                }
            }
            let base = vocabulary::is_base_value(effect, ctx.kind);
            lines.push((
                if base {
                    Section::Base
                } else {
                    effect.family().section()
                },
                Composed {
                    effect,
                    text: join(&parts),
                    values,
                    emphasis: if base {
                        Emphasis::Base
                    } else {
                        Emphasis::Bonus
                    },
                    global,
                },
            ));
        }

        if let Some(modifier) = group.part(Part::Modifier) {
            let Some(tag) = vocabulary::modifier_tag(effect) else {
                return;
            };
            let Some(label) = self.game.tag_text(&tag) else {
                out.unrendered.push(Unrendered {
                    variable: format!("{}Modifier", effect.name()),
                    reason: UnrenderedReason::MissingTag(tag),
                });
                return;
            };
            let mut values = Vec::new();
            let mut parts: Vec<String> = Vec::new();
            let modifier_chance = group.part(Part::ModifierChance).filter(|_| !global || xor);
            if let Some(chance) = modifier_chance {
                parts.push(self.fixed(tags::CHANCE_OF, &[Arg::Number(chance)]));
                values.push(chance);
            }
            let modifier = scaled(modifier);
            parts.push(parse_format(label).format(&[Arg::Number(modifier)]));
            values.push(modifier);
            if let Some(improved) = group.part(Part::DurationModifier) {
                parts.push(self.fixed(tags::IMPROVED_DURATION, &[Arg::Number(improved)]));
                values.push(improved);
            }
            lines.push((
                effect.family().section(),
                Composed {
                    effect,
                    text: join(&parts),
                    values,
                    emphasis: Emphasis::Bonus,
                    global,
                },
            ));
        }
    }

    fn render_group(
        &self,
        group: &Group<'_>,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        let effect = group.effect;
        match effect.family() {
            Family::Offense
            | Family::OffenseDuration
            | Family::Retaliation
            | Family::RetaliationDuration => {
                self.render_damage(group, ctx, lines, out);
            }
            Family::Defense => self.render_defense(group, ctx, lines, out),
            Family::Character => self.render_character(group, ctx, lines, out),
            Family::SkillStat => self.render_skill_stat(group, ctx, lines, out),
        }
    }

    fn render_defense(
        &self,
        group: &Group<'_>,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        let effect = group.effect;
        let is_block = effect.name() == "defensiveBlock";
        let tag_for = |part: Part| match part {
            Part::Value
                if effect.name() == "defensiveProtection" && ctx.kind != RecordKind::Armor =>
            {
                Some("DefenseAbsorptionProtectionPlus".to_string())
            }
            Part::Value => vocabulary::value_tag(effect, ctx.kind),
            Part::Modifier => vocabulary::modifier_tag(effect),
            other => vocabulary::defense_part_tag(effect, other),
        };
        for (part, chance_part, suffix) in [
            (Part::Value, Some(Part::Chance), ""),
            (Part::Modifier, Some(Part::ModifierChance), "Modifier"),
            (Part::Duration, Some(Part::DurationChance), "Duration"),
            (
                Part::DurationModifier,
                Some(Part::DurationModifierChance),
                "DurationModifier",
            ),
            (Part::MaxResist, None, "MaxResist"),
        ] {
            let Some(value) = group.part(part) else {
                continue;
            };
            let Some(tag) = tag_for(part) else {
                continue;
            };
            let variable = format!("{}{suffix}", effect.name());
            let Some(label) = self.game.tag_text(&tag) else {
                out.unrendered.push(Unrendered {
                    variable,
                    reason: UnrenderedReason::MissingTag(tag),
                });
                continue;
            };
            let mut values = Vec::new();
            let mut parts: Vec<String> = Vec::new();
            let chance = chance_part
                .and_then(|chance| group.part(chance))
                .filter(|_| !(is_block && part == Part::Value));
            if let Some(chance) = chance {
                parts.push(self.fixed(tags::CHANCE_OF, &[Arg::Number(chance)]));
                values.push(chance);
            }
            parts.push(parse_format(label).format(&[Arg::Number(value)]));
            values.push(value);
            let base = part == Part::Value && vocabulary::is_base_value(effect, ctx.kind);
            lines.push((
                if base {
                    Section::Base
                } else {
                    Section::Defense
                },
                Composed {
                    effect,
                    text: join(&parts),
                    values,
                    emphasis: if base {
                        Emphasis::Base
                    } else {
                        Emphasis::Bonus
                    },
                    global: false,
                },
            ));
        }
        if is_block
            && group.has(Part::Value)
            && let Some(chance) = group.part(Part::Chance)
        {
            self.block_chance_line(effect, chance, lines, out);
        }
    }

    /// A shield's "30% Chance to Block", the one chance the game shows
    /// as its own line.
    fn block_chance_line(
        &self,
        effect: Effect,
        chance: f32,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        match self.game.tag_text(tags::BLOCK_CHANCE) {
            Some(label) => lines.push((
                Section::Base,
                Composed {
                    effect,
                    text: join(&[format!("{chance:.0}%"), parse_format(label).format(&[])]),
                    values: vec![chance],
                    emphasis: Emphasis::Base,
                    global: false,
                },
            )),
            None => out.unrendered.push(Unrendered {
                variable: "defensiveBlockChance".to_string(),
                reason: UnrenderedReason::MissingTag(tags::BLOCK_CHANCE.to_string()),
            }),
        }
    }

    fn render_character(
        &self,
        group: &Group<'_>,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        let effect = group.effect;
        for (part, tag, suffix) in [
            (Part::Value, vocabulary::value_tag(effect, ctx.kind), ""),
            (Part::Modifier, vocabulary::modifier_tag(effect), "Modifier"),
        ] {
            let Some(value) = group.part(part) else {
                continue;
            };
            let Some(tag) = tag else {
                continue;
            };
            let Some(label) = self.game.tag_text(&tag) else {
                out.unrendered.push(Unrendered {
                    variable: format!("{}{suffix}", effect.name()),
                    reason: UnrenderedReason::MissingTag(tag),
                });
                continue;
            };
            lines.push((
                Section::Character,
                Composed {
                    effect,
                    text: tidy(&parse_format(label).format(&[Arg::Number(value)])),
                    values: vec![value],
                    emphasis: Emphasis::Bonus,
                    global: false,
                },
            ));
        }
    }

    fn render_skill_stat(
        &self,
        group: &Group<'_>,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
        out: &mut RecordStats,
    ) {
        let effect = group.effect;
        if let Some(value) = group.part(Part::Value)
            && let Some(shape) = vocabulary::skill_stat_shape(effect)
        {
            let value = scale_value(effect, value, ctx.scale);
            let composed = match shape {
                SkillStatShape::Direct(tag) => self
                    .label(tag)
                    .map(|label| (parse_format(label).format(&[Arg::Number(value)]), label)),
                SkillStatShape::Count(noun) => self.wrapped(tags::SKILL_COUNT, noun, value),
                SkillStatShape::Seconds(noun) => self.wrapped(tags::SKILL_SECONDS, noun, value),
                SkillStatShape::Meters(noun) => self.wrapped(tags::SKILL_METERS, noun, value),
                SkillStatShape::Percent(noun) => self.label(noun).map(|label| {
                    (
                        join(&[format!("{value:.0}%"), parse_format(label).format(&[])]),
                        label,
                    )
                }),
            };
            match composed {
                Some((text, _)) => lines.push((
                    Section::Skill,
                    Composed {
                        effect,
                        text: tidy(&text),
                        values: vec![value],
                        emphasis: Emphasis::Bonus,
                        global: false,
                    },
                )),
                None => out.unrendered.push(Unrendered {
                    variable: effect.name().to_string(),
                    reason: UnrenderedReason::MissingTag(match shape {
                        SkillStatShape::Direct(tag)
                        | SkillStatShape::Count(tag)
                        | SkillStatShape::Seconds(tag)
                        | SkillStatShape::Meters(tag)
                        | SkillStatShape::Percent(tag) => tag.to_string(),
                    }),
                }),
            }
        }
        if let Some(modifier) = group.part(Part::Modifier)
            && let Some(tag) = vocabulary::modifier_tag(effect)
        {
            match self.game.tag_text(&tag) {
                Some(label) => lines.push((
                    Section::Skill,
                    Composed {
                        effect,
                        text: tidy(&parse_format(label).format(&[Arg::Number(modifier)])),
                        values: vec![modifier],
                        emphasis: Emphasis::Bonus,
                        global: false,
                    },
                )),
                None => out.unrendered.push(Unrendered {
                    variable: format!("{}Modifier", effect.name()),
                    reason: UnrenderedReason::MissingTag(tag),
                }),
            }
        }
    }

    /// "15.0% Chance of:" / "…for one of the following:" over the lines
    /// a global chance governs, which the game indents beneath it.
    fn attach_global_headers(
        &self,
        record: &DbRecord,
        ctx: Ctx,
        lines: &mut Vec<(Section, Composed)>,
    ) {
        for (variable, sections) in [
            (
                "offensiveGlobalChance",
                [Section::Offense, Section::OffenseDuration].as_slice(),
            ),
            ("retaliationGlobalChance", [Section::Retaliation].as_slice()),
        ] {
            let Some(chance) = float_at(record, variable, ctx.level).filter(|c| *c > 0.0) else {
                continue;
            };
            let governed: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, (section, composed))| composed.global && sections.contains(section))
                .map(|(index, _)| index)
                .collect();
            let Some(&first) = governed.first() else {
                continue;
            };
            let exclusive = record.variables().any(|v| {
                v.name.to_ascii_lowercase().ends_with("xor") && value_at(v, ctx.level) == Some(1.0)
            });
            let tag = if exclusive {
                tags::GLOBAL_ONE
            } else {
                tags::GLOBAL_ALL
            };
            for &index in &governed {
                let composed = &mut lines[index].1;
                composed.text = format!("{INDENT}{}", composed.text);
            }
            let header = Composed {
                effect: lines[first].1.effect,
                text: tidy(&self.fixed(tag, &[Arg::Number(chance)])),
                values: vec![chance],
                emphasis: Emphasis::Bonus,
                global: false,
            };
            lines.insert(first, (lines[first].0, header));
        }
    }

    fn render_specials(
        &self,
        record: &DbRecord,
        ctx: Ctx,
        groups: &[Group<'_>],
        out: &mut RecordStats,
    ) {
        self.render_attack_speed(record, ctx, out);
        self.render_block_recovery(record, ctx, out);
        self.render_conversions(record, ctx, out);
        self.render_racial(record, ctx, groups, out);
        self.render_skill_bonuses(record, ctx, out);
        self.render_granted_skill(record, ctx, out);
        self.render_skill_modifiers(record, ctx, out);
        self.render_pet_bonus(record, ctx, out);
    }

    fn render_attack_speed(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        if ctx.kind != RecordKind::Weapon {
            return;
        }
        let Some(tag) = record.string("characterBaseAttackSpeedTag") else {
            return;
        };
        match self.game.tag_text(tag) {
            Some(label) => out.lines.push(StatLine {
                kind: LineKind::AttackSpeed,
                section: ctx.section.unwrap_or(Section::Base),
                emphasis: Emphasis::Base,
                text: tidy(&parse_format(label).format(&[])),
                values: Vec::new(),
            }),
            None => out.unrendered.push(Unrendered {
                variable: "characterBaseAttackSpeedTag".to_string(),
                reason: UnrenderedReason::MissingTag(tag.to_string()),
            }),
        }
    }

    fn render_block_recovery(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        let Some(seconds) = float_at(record, "blockRecoveryTime", ctx.level).filter(|s| *s > 0.0)
        else {
            return;
        };
        match self.game.tag_text(tags::BLOCK_RECOVERY) {
            Some(label) => out.lines.push(StatLine {
                kind: LineKind::BlockRecovery,
                section: ctx.section.unwrap_or(Section::Base),
                emphasis: Emphasis::Base,
                text: tidy(&parse_format(label).format(&[Arg::Number(seconds)])),
                values: vec![seconds],
            }),
            None => out.unrendered.push(Unrendered {
                variable: "blockRecoveryTime".to_string(),
                reason: UnrenderedReason::MissingTag(tags::BLOCK_RECOVERY.to_string()),
            }),
        }
    }

    fn render_conversions(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        for suffix in ["", "2"] {
            let percent = float_at(record, &format!("conversionPercentage{suffix}"), ctx.level)
                .filter(|p| *p > 0.0);
            let (Some(percent), Some(from), Some(to)) = (
                percent,
                record.string(&format!("conversionInType{suffix}")),
                record.string(&format!("conversionOutType{suffix}")),
            ) else {
                continue;
            };
            let named = |damage_type: &str| {
                let tag = vocabulary::conversion_tag(damage_type);
                self.game
                    .tag_text(&tag)
                    .map(|label| parse_format(label).format(&[]))
                    .ok_or(tag)
            };
            let composed = self
                .label(tags::CONVERSION)
                .ok_or_else(|| tags::CONVERSION.to_string())
                .and_then(|label| Ok((label, named(from)?, named(to)?)));
            match composed {
                Ok((label, from, to)) => out.lines.push(StatLine {
                    kind: LineKind::Conversion,
                    section: ctx.section.unwrap_or(Section::Conversion),
                    emphasis: Emphasis::Bonus,
                    text: tidy(&parse_format(label).format(&[
                        Arg::Number(percent),
                        Arg::Text(from),
                        Arg::Text(to),
                    ])),
                    values: vec![percent],
                }),
                Err(tag) => out.unrendered.push(Unrendered {
                    variable: format!("conversionPercentage{suffix}"),
                    reason: UnrenderedReason::MissingTag(tag),
                }),
            }
        }
    }

    fn render_racial(
        &self,
        record: &DbRecord,
        ctx: Ctx,
        groups: &[Group<'_>],
        out: &mut RecordStats,
    ) {
        let Some(DbValues::Strings(races)) = record
            .variable("racialBonusRace")
            .map(|variable| &variable.values)
        else {
            return;
        };
        let bonuses = [
            ("racialBonusPercentDamage", tags::RACIAL_PERCENT_DAMAGE),
            ("racialBonusPercentDefense", tags::RACIAL_PERCENT_DEFENSE),
            ("racialBonusAbsoluteDamage", tags::RACIAL_ABSOLUTE_DAMAGE),
            ("racialBonusAbsoluteDefense", tags::RACIAL_ABSOLUTE_DEFENSE),
        ];
        let mut counted = 0;
        for (variable, tag) in bonuses {
            let Some(value) = float_at(record, variable, ctx.level).filter(|v| *v != 0.0) else {
                continue;
            };
            let Some(label) = self.game.tag_text(tag) else {
                out.unrendered.push(Unrendered {
                    variable: variable.to_string(),
                    reason: UnrenderedReason::MissingTag(tag.to_string()),
                });
                continue;
            };
            for race in races.iter().filter(|race| !race.is_empty()) {
                let plural = format!("{}P", vocabulary::race_tag(race));
                let name = self
                    .game
                    .tag_text(&plural)
                    .or_else(|| self.game.tag_text(&vocabulary::race_tag(race)))
                    .map_or_else(|| race.clone(), |label| parse_format(label).format(&[]));
                out.lines.push(StatLine {
                    kind: LineKind::Racial,
                    section: ctx.section.unwrap_or(Section::Racial),
                    emphasis: Emphasis::Bonus,
                    text: tidy(&parse_format(label).format(&[Arg::Number(value), Arg::Text(name)])),
                    values: vec![value],
                });
                counted += 1;
            }
        }
        out.attribute_count += counted;
        let _ = groups;
    }

    fn render_skill_bonuses(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        let section = ctx.section.unwrap_or(Section::SkillBonus);
        for slot in 1..=5 {
            let Some(skill) = record
                .string(&format!("augmentSkillName{slot}"))
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let Some(level) = integer_at(record, &format!("augmentSkillLevel{slot}"), ctx.level)
                .filter(|l| *l > 0)
            else {
                continue;
            };
            let name = self.skill_name(skill, 0, out);
            self.push_bonus(
                tags::SKILL_BONUS,
                LineKind::SkillBonus,
                level,
                name,
                section,
                out,
            );
        }
        for slot in 1..=3 {
            let Some(mastery) = record
                .string(&format!("augmentMasteryName{slot}"))
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let Some(level) = integer_at(record, &format!("augmentMasteryLevel{slot}"), ctx.level)
                .filter(|l| *l > 0)
            else {
                continue;
            };
            let name = self.skill_name(mastery, 0, out);
            self.push_bonus(
                tags::MASTERY_BONUS,
                LineKind::MasteryBonus,
                level,
                name,
                section,
                out,
            );
        }
        if let Some(level) = integer_at(record, "augmentAllLevel", ctx.level).filter(|l| *l > 0) {
            match self.game.tag_text(tags::ALL_SKILLS_BONUS) {
                Some(label) => out.lines.push(StatLine {
                    kind: LineKind::AllSkillsBonus,
                    section,
                    emphasis: Emphasis::Bonus,
                    text: tidy(&parse_format(label).format(&[Arg::Number(count(level))])),
                    values: vec![count(level)],
                }),
                None => out.unrendered.push(Unrendered {
                    variable: "augmentAllLevel".to_string(),
                    reason: UnrenderedReason::MissingTag(tags::ALL_SKILLS_BONUS.to_string()),
                }),
            }
        }
    }

    fn push_bonus(
        &self,
        tag: &str,
        kind: LineKind,
        level: u32,
        name: String,
        section: Section,
        out: &mut RecordStats,
    ) {
        match self.game.tag_text(tag) {
            Some(label) => out.lines.push(StatLine {
                kind,
                section,
                emphasis: Emphasis::Bonus,
                text: tidy(
                    &parse_format(label).format(&[Arg::Number(count(level)), Arg::Text(name)]),
                ),
                values: vec![count(level)],
            }),
            None => out.unrendered.push(Unrendered {
                variable: name,
                reason: UnrenderedReason::MissingTag(tag.to_string()),
            }),
        }
    }

    fn render_granted_skill(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        let Some(skill_id) = record.string("itemSkillName").filter(|id| !id.is_empty()) else {
            return;
        };
        if ctx.depth >= MAX_DEPTH {
            return;
        }
        let section = ctx.section.unwrap_or(Section::GrantedSkill);
        let Some(skill) = self.record(skill_id) else {
            out.unrendered.push(Unrendered {
                variable: "itemSkillName".to_string(),
                reason: UnrenderedReason::MissingRecord(skill_id.to_string()),
            });
            return;
        };
        let name = self.skill_name(skill_id, 0, out);
        let heading = self.label(tags::GRANTS_SKILL).map_or_else(
            || "Grants Skill:".to_string(),
            |label| parse_format(label).format(&[]),
        );
        let autocast = self.autocast_suffix(record, out);
        out.lines.push(StatLine {
            kind: LineKind::GrantedSkill,
            section,
            emphasis: Emphasis::Heading,
            text: join(&[heading, name, autocast]),
            values: Vec::new(),
        });
        if let Some(description) = self.translated(&skill, "skillBaseDescription") {
            out.lines.push(StatLine {
                kind: LineKind::SkillDescription,
                section,
                emphasis: Emphasis::Muted,
                text: description,
                values: Vec::new(),
            });
        }
        let level = granted_level(record);
        let max_level = skill.integer("skillMaxLevel").unwrap_or(1);
        if max_level > 1
            && let Some(label) = self.label(tags::SKILL_LEVEL)
        {
            out.lines.push(StatLine {
                kind: LineKind::SkillLevel,
                section,
                emphasis: Emphasis::Muted,
                text: tidy(&parse_format(label).format(&[Arg::Number(count(level.get()))])),
                values: vec![count(level.get())],
            });
        }
        self.render_into(
            &skill,
            Ctx {
                kind: RecordKind::Other,
                level: level.index(),
                scale: 1.0,
                depth: ctx.depth + 1,
                section: Some(section),
            },
            out,
        );
    }

    /// " (30% Chance on Critical Attack)" from the item's autocast
    /// controller, or nothing when the skill is cast by hand.
    fn autocast_suffix(&self, record: &DbRecord, out: &mut RecordStats) -> String {
        let Some(controller_id) = record
            .string("itemSkillAutoController")
            .filter(|id| !id.is_empty())
        else {
            return String::new();
        };
        let Some(controller) = self.record(controller_id) else {
            out.unrendered.push(Unrendered {
                variable: "itemSkillAutoController".to_string(),
                reason: UnrenderedReason::MissingRecord(controller_id.to_string()),
            });
            return String::new();
        };
        let Some(trigger) = controller.string("triggerType") else {
            return String::new();
        };
        let Some(tag) = vocabulary::autocast_tag(trigger) else {
            out.unrendered.push(Unrendered {
                variable: "triggerType".to_string(),
                reason: UnrenderedReason::UnknownAttribute,
            });
            return String::new();
        };
        let chance = controller
            .integer("chanceToRun")
            .map(whole)
            .or_else(|| controller.float("chanceToRun"))
            .unwrap_or(100.0);
        let threshold = float_at(&controller, "lifeMonitorPercent", 0).unwrap_or(0.0);
        if let Some(label) = self.game.tag_text(tag) {
            return parse_format(label).format(&[Arg::Number(chance), Arg::Number(threshold)]);
        }
        out.unrendered.push(Unrendered {
            variable: "triggerType".to_string(),
            reason: UnrenderedReason::MissingTag(tag.to_string()),
        });
        String::new()
    }

    fn render_skill_modifiers(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        if ctx.depth >= MAX_DEPTH {
            return;
        }
        let section = ctx.section.unwrap_or(Section::SkillModifier);
        for slot in 1..=6 {
            let (Some(modifier_id), Some(target_id)) = (
                record
                    .string(&format!("modifierSkillName{slot}"))
                    .filter(|id| !id.is_empty()),
                record
                    .string(&format!("modifiedSkillName{slot}"))
                    .filter(|id| !id.is_empty()),
            ) else {
                continue;
            };
            let Some(modifier) = self.record(modifier_id) else {
                out.unrendered.push(Unrendered {
                    variable: format!("modifierSkillName{slot}"),
                    reason: UnrenderedReason::MissingRecord(modifier_id.to_string()),
                });
                continue;
            };
            let modifier = match modifier
                .string("petSkillName")
                .filter(|id| !id.is_empty())
                .and_then(|id| self.record(id))
            {
                Some(pet_modifier) => pet_modifier,
                None => modifier,
            };
            let target = self.skill_name(target_id, 0, out);
            let suffix = self.label(tags::MODIFIED_SKILL).map_or_else(
                || format!("to {target}"),
                |label| parse_format(label).format(&[Arg::Text(target.clone())]),
            );
            let mut nested = RecordStats::default();
            self.render_into(
                &modifier,
                Ctx {
                    kind: RecordKind::Other,
                    level: 0,
                    scale: 1.0,
                    depth: ctx.depth + 1,
                    section: Some(section),
                },
                &mut nested,
            );
            for mut line in nested.lines {
                line.text = join(&[line.text, suffix.clone()]);
                out.lines.push(line);
            }
            out.unrendered.extend(nested.unrendered);
        }
    }

    fn render_pet_bonus(&self, record: &DbRecord, ctx: Ctx, out: &mut RecordStats) {
        let Some(bonus_id) = record.string("petBonusName").filter(|id| !id.is_empty()) else {
            return;
        };
        if ctx.depth >= MAX_DEPTH {
            return;
        }
        let section = ctx.section.unwrap_or(Section::PetBonus);
        let Some(bonus) = self.record(bonus_id) else {
            out.unrendered.push(Unrendered {
                variable: "petBonusName".to_string(),
                reason: UnrenderedReason::MissingRecord(bonus_id.to_string()),
            });
            return;
        };
        let heading = self.label(tags::PET_BONUS).map_or_else(
            || "Bonus to All Pets".to_string(),
            |label| parse_format(label).format(&[]),
        );
        out.lines.push(StatLine {
            kind: LineKind::PetBonusHeading,
            section,
            emphasis: Emphasis::Heading,
            text: tidy(&heading),
            values: Vec::new(),
        });
        self.render_into(
            &bonus,
            Ctx {
                kind: RecordKind::Other,
                level: 0,
                scale: 1.0,
                depth: ctx.depth + 1,
                section: Some(section),
            },
            out,
        );
    }

    /// A skill's display name, following buff and pet redirections; the
    /// record's file stem when the database cannot name it, with the
    /// gap reported.
    fn skill_name(&self, skill_id: &str, depth: u32, out: &mut RecordStats) -> String {
        let stem = RecordId::parse(skill_id.to_string())
            .map_or_else(|| skill_id.to_string(), |id| id.file_stem().to_string());
        if depth > MAX_DEPTH {
            return stem;
        }
        let Some(skill) = self.record(skill_id) else {
            out.unrendered.push(Unrendered {
                variable: "skillDisplayName".to_string(),
                reason: UnrenderedReason::MissingRecord(skill_id.to_string()),
            });
            return stem;
        };
        if let Some(name) = self.translated(&skill, "skillDisplayName") {
            return name;
        }
        for redirect in ["buffSkillName", "petSkillName"] {
            if let Some(target) = skill.string(redirect).filter(|id| !id.is_empty()) {
                return self.skill_name(target, depth + 1, out);
            }
        }
        out.unrendered.push(Unrendered {
            variable: "skillDisplayName".to_string(),
            reason: UnrenderedReason::MissingTag(
                skill.string("skillDisplayName").unwrap_or("").to_string(),
            ),
        });
        stem
    }

    fn record(&self, id: &str) -> Option<DbRecord> {
        let id = RecordId::parse(id.to_string())?;
        self.game.record(&id)?.ok()
    }

    /// The localized text a record's tag variable points at.
    fn translated(&self, record: &DbRecord, variable: &str) -> Option<String> {
        let tag = record.string(variable).filter(|tag| !tag.is_empty())?;
        self.game
            .tag_text(tag)
            .map(|label| tidy(&parse_format(label).format(&[])))
    }

    fn label(&self, tag: &str) -> Option<&str> {
        self.game.tag_text(tag)
    }

    /// A fixed format tag rendered with `args`, falling back to the
    /// game's own spelling when a text archive lacks the tag.
    fn fixed(&self, tag: &str, args: &[Arg]) -> String {
        self.label(tag)
            .map_or_else(|| default_spec(tag), parse_format)
            .format(args)
    }

    /// `low` alone, or `low-high` when they differ, through the
    /// single / range format tags.
    fn amount(&self, single: &str, range: &str, low: f32, high: Option<f32>) -> String {
        match high {
            Some(high) => self.fixed(range, &[Arg::Number(low), Arg::Number(high)]),
            None => self.fixed(single, &[Arg::Number(low)]),
        }
    }

    /// A skill stat's noun wrapped in one of the `Skill*Format` tags.
    fn wrapped(&self, format: &str, noun: &str, value: f32) -> Option<(String, &str)> {
        let label = self.label(noun)?;
        let noun_text = parse_format(label).format(&[]);
        Some((
            self.fixed(format, &[Arg::Number(value), Arg::Text(noun_text)]),
            label,
        ))
    }
}

/// The game's spelling of the fixed formats, for installs whose text
/// archive lacks one (the spec falls back rather than dropping the
/// number).
fn default_spec(tag: &str) -> FormatSpec {
    parse_format(match tag {
        tags::SINGLE => "{%.0f0}",
        tags::RANGE => "{%.0f0}-{%.0f1}",
        tags::OVER_SINGLE => "over {%.1f0} Seconds",
        tags::OVER_RANGE => "over {%.1f0}-{%.1f1} Seconds",
        tags::FOR_SINGLE => "for {%.1f0} Seconds",
        tags::FOR_RANGE => "for {%.1f0} - {%.1f1} Seconds",
        tags::RETALIATION_FOR_SINGLE => "{%.1f0} Seconds",
        tags::RETALIATION_FOR_RANGE => "{%.1f0} - {%.1f1} Seconds",
        tags::CHANCE_OF => "{%.1f0}% Chance of ",
        tags::IMPROVED_DURATION => "with {%+.0f0}% Increased Duration",
        tags::MANA_BURN_RATIO => "({%.0f0}% of Energy Burnt causes Damage)",
        tags::GLOBAL_ALL => "{%.1f0}% Chance of:",
        tags::GLOBAL_ONE => "{%.1f0}% Chance for one of the following:",
        tags::SKILL_COUNT => "{%d0 %s1}",
        tags::SKILL_SECONDS => "{%.1f0 Second %s1}",
        tags::SKILL_METERS => "{%.1f0 Meter %s1}",
        _ => "",
    })
}

/// The record's non-zero numeric variables at the level, grouped by
/// stat family; unknown ones reported unless the vocabulary hides them.
fn group_variables<'r>(record: &'r DbRecord, ctx: Ctx, out: &mut RecordStats) -> Vec<Group<'r>> {
    let mut groups: BTreeMap<(u8, usize), Group<'r>> = BTreeMap::new();
    for variable in record.variables() {
        let Some(value) = value_at(variable, ctx.level) else {
            continue;
        };
        if value == 0.0 {
            continue;
        }
        match vocabulary::parse_variable(&variable.name) {
            Some(Attribute { effect, part }) => {
                let group = groups
                    .entry(vocabulary::order(effect))
                    .or_insert_with(|| Group {
                        effect,
                        parts: Vec::new(),
                        variables: Vec::new(),
                    });
                group.parts.push((part, value));
                group.variables.push(variable);
            }
            None if vocabulary::is_hidden(&variable.name) => {}
            None => out.unrendered.push(Unrendered {
                variable: variable.name.clone(),
                reason: UnrenderedReason::UnknownAttribute,
            }),
        }
    }
    groups.into_values().collect()
}

/// The cost formulas' `totalAttCount`: one per amount, percent
/// modifier, duration modifier and max-resistance present.
fn attribute_count(groups: &[Group<'_>]) -> u32 {
    groups
        .iter()
        .map(|group| {
            let has = |part| u32::from(group.part(part).is_some_and(|value| value > 0.0));
            has(Part::Min)
                + u32::from(
                    !group.has(Part::Min) && group.part(Part::Value).is_some_and(|v| v > 0.0),
                )
                + has(Part::Modifier)
                + has(Part::DurationModifier)
                + has(Part::MaxResist)
        })
        .sum()
}

fn scale_value(effect: Effect, value: f32, factor: f32) -> f32 {
    if vocabulary::scales_with_item(effect) && (factor - 1.0).abs() > f32::EPSILON {
        (value * factor).trunc()
    } else {
        value
    }
}

/// The value at `index`, capped to the last one — a skill's arrays run
/// per level and stop where the values stop changing.
fn value_at(variable: &DbVariable, index: usize) -> Option<f32> {
    let capped = |len: usize| index.min(len.saturating_sub(1));
    match &variable.values {
        DbValues::Integers(values) => values.get(capped(values.len())).map(|v| whole(*v)),
        DbValues::Floats(values) => values.get(capped(values.len())).copied(),
        DbValues::Booleans(values) => values.get(capped(values.len())).map(|v| f32::from(*v)),
        DbValues::Strings(_) => None,
    }
}

fn float_at(record: &DbRecord, name: &str, index: usize) -> Option<f32> {
    record
        .variable(name)
        .and_then(|variable| value_at(variable, index))
}

fn integer_at(record: &DbRecord, name: &str, index: usize) -> Option<u32> {
    float_at(record, name, index).map(to_count)
}

/// A database number as the count it stands for; negatives and
/// anything beyond `u32` clamp.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "clamped to 0..=u32::MAX before the cast"
)]
fn to_count(value: f32) -> u32 {
    value.round().clamp(0.0, u32::MAX as f32) as u32
}

/// `itemSkillLevelEq` ("1", "itemLevel/4+1") evaluated against the
/// record's own `itemLevel`; level 1 when absent or unreadable.
fn granted_level(record: &DbRecord) -> SkillLevel {
    let item_level = f64::from(record.integer("itemLevel").unwrap_or(0));
    record
        .string("itemSkillLevelEq")
        .filter(|eq| !eq.is_empty())
        .and_then(|eq| eval_equation(eq, &|name| (name == "itemLevel").then_some(item_level)))
        .and_then(SkillLevel::from_equation)
        .unwrap_or(SkillLevel::ONE)
}

/// Joins the pieces of a line with single spaces. The text archive
/// trims every tag, so the spacing the tags carried has to be put
/// back; a piece beginning with `%` is the suffix of the number before
/// it ("20" + "% Slower target Movement").
fn join(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts.iter().filter(|part| !part.trim().is_empty()) {
        if !out.is_empty() && !part.starts_with('%') {
            out.push(' ');
        }
        out.push_str(part.trim());
    }
    tidy(&out)
}

/// Collapses the double spaces the tags' own padding leaves behind.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = true;
    for c in text.chars() {
        if c == ' ' {
            if !last_space {
                out.push(c);
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim_end().to_string()
}
