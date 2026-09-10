//! The mastery and skill tools: the playable masteries as the player
//! record lists them, one mastery's tree, and one skill rendered at
//! chosen levels through the same stat renderer the item tooltips use.

use grimvault_core::gamedata::GameData;
use grimvault_core::respec::{MasteryTree, RespecRules};
use grimvault_core::stats::{Scale, SkillLevel};
use serde_json::{Value, json};
use univault_engine::ids::RecordId;

use crate::build::{skill_name, skill_record, skill_summary};
use crate::view::line_texts;

/// Every mastery: its index (the game's own numbering, which the
/// header's class tag spells), name, tree record, and size.
#[must_use]
pub fn masteries_json(rules: &RespecRules) -> Vec<Value> {
    rules
        .base
        .masteries
        .iter()
        .map(|tree| {
            json!({
                "index": tree.index.value(),
                "name": tree.name,
                "record": tree.record.as_str(),
                "skills": tree.member_count(),
            })
        })
        .collect()
}

/// The mastery a caller means: by name (case-insensitive), by index,
/// or by a substring of the tree record's path.
///
/// # Errors
/// The known masteries, when none matches.
pub fn resolve_mastery<'a>(
    rules: &'a RespecRules,
    wanted: &str,
) -> Result<&'a MasteryTree, String> {
    let trees = &rules.base.masteries;
    let lowered = wanted.trim().to_lowercase();
    trees
        .iter()
        .find(|tree| tree.name.eq_ignore_ascii_case(wanted.trim()))
        .or_else(|| {
            wanted
                .trim()
                .parse::<u32>()
                .ok()
                .and_then(|index| trees.iter().find(|tree| tree.index.value() == index))
        })
        .or_else(|| {
            trees
                .iter()
                .find(|tree| tree.record.as_str().to_lowercase().contains(&lowered))
        })
        .ok_or_else(|| {
            let names: Vec<String> = trees
                .iter()
                .map(|tree| format!("{} ({})", tree.name, tree.index))
                .collect();
            format!(
                "no mastery matching '{wanted}' (known: {})",
                names.join(", ")
            )
        })
}

/// One mastery's tree: every skill record with its summary, tier
/// order then name; the mastery bar itself first.
#[must_use]
pub fn mastery_json(game: &GameData, tree: &MasteryTree) -> Value {
    let mut skills: Vec<(i32, String, Value)> = tree
        .members()
        .map(|record| {
            let tier = skill_record(game, record)
                .and_then(|found| found.integer("skillTier"))
                .unwrap_or(i32::MAX);
            let mut summary = skill_summary(game, record);
            if tree.grants(record) {
                summary["granted_free"] = json!(true);
            }
            (tier, skill_name(game, record), summary)
        })
        .collect();
    skills.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    json!({
        "index": tree.index.value(),
        "name": tree.name,
        "record": tree.record.as_str(),
        "skills": skills.into_iter().map(|(_, _, json)| json).collect::<Vec<_>>(),
    })
}

/// One skill: its summary and its stat lines at each requested
/// level, rendered as the game would show them.
///
/// # Errors
/// A level below one, or a record no layer defines.
pub fn skill_json(game: &GameData, record: &str, levels: &[u32]) -> Result<Value, String> {
    let id = RecordId::parse(record.to_string()).ok_or_else(|| "record is empty".to_string())?;
    if game.record(&id).is_none() {
        return Err(format!("no layer of the record database has {record}"));
    }
    let mut out = skill_summary(game, record);
    let mut rendered = Vec::new();
    for &level in levels {
        let skill_level =
            SkillLevel::new(level).ok_or_else(|| format!("level {level} is below 1"))?;
        let stats = game
            .record_stats(&id, skill_level, Scale::NONE)
            .ok_or_else(|| format!("no layer of the record database has {record}"))?;
        rendered.push(json!({
            "level": level,
            "lines": line_texts(&stats.lines),
            "unrendered": stats.unrendered.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }));
    }
    out["at_levels"] = json!(rendered);
    Ok(out)
}
