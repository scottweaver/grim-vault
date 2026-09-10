//! A character's build as the game thinks of it: the points in each
//! mastery's skills, the devotions, the skills worn gear grants, and
//! the pools the points came out of. Block 8 of `player.gdc` is a
//! flat list of skill records; the grouping comes from the mastery
//! trees the record database names (`respec::RespecRules`), which is
//! how the game itself decides what belongs to whom.

use grimvault_core::blocks::skills::{ItemSkill, Skill, Skills};
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::PlayerFile;
use grimvault_core::respec::{MasteryTree, RespecRules, RulesError};
use serde_json::{Value, json};
use univault_engine::arz::DbRecord;
use univault_engine::ids::{RecordId, normalize};

use crate::devotion::Constellations;
use crate::view;

/// The record class of a mastery's own bar (`_classtraining_classNN`).
const MASTERY_BAR_CLASS: &str = "Skill_Mastery";

/// The folder every devotion (constellation star) skill lives under.
const DEVOTION_FOLDER: &str = "\\devotion\\";

/// The folders whose skills every character carries without choosing
/// them: the default attacks and the potion modifiers.
const STANDARD_FOLDERS: [&str; 2] = ["\\SKILLS\\DEFAULT\\", "\\POTIONMODIFIERS\\"];

/// A skill record, when the database has it.
#[must_use]
pub fn skill_record(game: &GameData, record: &str) -> Option<DbRecord> {
    let id = RecordId::parse(record.to_string())?;
    game.record(&id)?.ok()
}

/// The localized name of a skill; a skill without one of its own (a
/// devotion proc whose name sits on the buff or pet it applies) takes
/// that record's, then the developers' file description, then the
/// file stem.
#[must_use]
pub fn skill_name(game: &GameData, record: &str) -> String {
    let found = skill_record(game, record);
    found
        .as_ref()
        .and_then(|found| translated(game, found, "skillDisplayName"))
        .or_else(|| {
            let found = found.as_ref()?;
            ["buffSkillName", "petSkillName"]
                .iter()
                .find_map(|variable| {
                    let referenced = skill_record(game, found.string(variable)?)?;
                    translated(game, &referenced, "skillDisplayName")
                })
        })
        .or_else(|| {
            found
                .as_ref()
                .and_then(|found| found.string("FileDescription"))
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
        .or_else(|| RecordId::parse(record.to_string()).map(|id| id.file_stem().to_string()))
        .unwrap_or_else(|| record.to_string())
}

fn translated(game: &GameData, record: &DbRecord, variable: &str) -> Option<String> {
    let tag = record.string(variable)?;
    if tag.is_empty() {
        return None;
    }
    game.tag_text(tag).map(str::to_string)
}

/// What a skill record says about itself: name, description, class,
/// tier, level caps, and the skills it references.
#[must_use]
pub fn skill_summary(game: &GameData, record: &str) -> Value {
    let Some(found) = skill_record(game, record) else {
        return json!({
            "record": record,
            "name": skill_name(game, record),
            "note": "no layer of the record database has this record",
        });
    };
    let strings = |variable: &str| -> Vec<String> {
        found
            .variable(variable)
            .map(|found| match &found.values {
                univault_engine::arz::DbValues::Strings(values) => values
                    .iter()
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .collect(),
                univault_engine::arz::DbValues::Integers(_)
                | univault_engine::arz::DbValues::Floats(_)
                | univault_engine::arz::DbValues::Booleans(_) => Vec::new(),
            })
            .unwrap_or_default()
    };
    let mut out = json!({
        "record": normalize(record),
        "name": skill_name(game, record),
        "class": found.record_type,
        "description": translated(game, &found, "skillBaseDescription"),
        "tier": found.integer("skillTier"),
        "max_level": found.integer("skillMaxLevel"),
        "ultimate_level": found.integer("skillUltimateLevel"),
        "mastery_level_required": found.integer("skillMasteryLevelRequired"),
    });
    for (key, variable) in [
        ("requires", "skillDependancy"),
        ("grants", "grantedSkills"),
        ("buff", "buffSkillName"),
        ("pet", "petSkillName"),
        ("modifies", "skillAffectedByModifiers"),
    ] {
        let values = strings(variable);
        if !values.is_empty() {
            out[key] = json!(values);
        }
    }
    out
}

/// Where a learned skill belongs.
enum Home<'a> {
    Mastery(&'a MasteryTree),
    Devotion,
    Standard,
    Other,
}

fn home_of<'a>(trees: &'a [MasteryTree], skill: &Skill) -> Home<'a> {
    if let Some(tree) = trees.iter().find(|tree| tree.contains(&skill.name)) {
        return Home::Mastery(tree);
    }
    let key = normalize(&skill.name);
    if key.contains(DEVOTION_FOLDER) || skill.devotion_level > 0 {
        return Home::Devotion;
    }
    if STANDARD_FOLDERS.iter().any(|folder| key.contains(folder)) {
        return Home::Standard;
    }
    Home::Other
}

fn learned_json(game: &GameData, skill: &Skill) -> Value {
    let mut out = json!({
        "record": skill.name,
        "name": skill_name(game, &skill.name),
        "points": skill.level,
    });
    if let Some(found) = skill_record(game, &skill.name) {
        out["tier"] = json!(found.integer("skillTier"));
        out["max_level"] = json!(found.integer("skillMaxLevel"));
        out["ultimate_level"] = json!(found.integer("skillUltimateLevel"));
    }
    if skill.devotion_level > 0 {
        out["devotion_level"] = json!(skill.devotion_level);
        out["experience"] = json!(skill.experience);
    }
    if !skill.auto_cast_skill.is_empty() {
        out["auto_cast"] = json!({
            "skill": skill.auto_cast_skill,
            "skill_name": skill_name(game, &skill.auto_cast_skill),
            "controller": skill.auto_cast_controller,
            "controller_name": skill_name(game, &skill.auto_cast_controller),
        });
    }
    out
}

fn item_skill_json(game: &GameData, skill: &ItemSkill) -> Value {
    json!({
        "record": skill.name,
        "name": skill_name(game, &skill.name),
        "from_item": {
            "slot_index": skill.item_slot,
            "record": skill.item_name,
            "name": view::record_name(game, &skill.item_name),
        },
    })
}

fn is_mastery_bar(game: &GameData, skill: &Skill) -> bool {
    skill_record(game, &skill.name).is_some_and(|found| found.record_type == MASTERY_BAR_CLASS)
}

/// One mastery's slice of the build: the bar's level and every skill
/// with points, tier order then name.
fn mastery_json(game: &GameData, tree: &MasteryTree, learned: &[&Skill]) -> Value {
    let bar = learned
        .iter()
        .find(|skill| is_mastery_bar(game, skill))
        .map_or(0, |skill| skill.level);
    let mut skills: Vec<(i32, String, Value)> = learned
        .iter()
        .filter(|skill| !is_mastery_bar(game, skill))
        .map(|skill| {
            let tier = skill_record(game, &skill.name)
                .and_then(|found| found.integer("skillTier"))
                .unwrap_or(i32::MAX);
            (
                tier,
                skill_name(game, &skill.name),
                learned_json(game, skill),
            )
        })
        .collect();
    skills.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let points: u32 = learned
        .iter()
        .filter(|skill| !tree.grants(&skill.name))
        .map(|skill| skill.level)
        .sum();
    json!({
        "index": tree.index.value(),
        "name": tree.name,
        "record": tree.record.as_str(),
        "mastery_level": bar,
        "points_spent": points,
        "skills": skills.into_iter().map(|(_, _, json)| json).collect::<Vec<_>>(),
    })
}

/// The build: pools and unspent points from block 2, skills grouped
/// by mastery from block 8, devotions, the rest, and item-granted
/// skills. Without the mastery trees (a database the rules could not
/// be read from) every skill is listed flat under `ungrouped`.
#[must_use]
pub fn build_json(
    game: &GameData,
    rules: Result<&RespecRules, &RulesError>,
    constellations: &Constellations,
    file: &PlayerFile,
) -> Value {
    let header = file.header();
    let class = game
        .tag_text(&header.class_tag)
        .map_or_else(|| header.class_tag.clone(), str::to_string);
    let mut out = json!({
        "level": header.level,
        "class": if class.is_empty() { Value::Null } else { json!(class) },
        "class_tag": header.class_tag,
    });
    if let Some(bio) = file.bio() {
        out["experience"] = json!(bio.experience);
        out["attributes"] = json!({
            "physique": bio.physique,
            "cunning": bio.cunning,
            "spirit": bio.spirit,
        });
        out["pools"] = json!({"health": bio.health, "energy": bio.energy});
        out["unspent"] = json!({
            "attribute_points": bio.attribute_points_unspent,
            "skill_points": bio.skill_points_unspent,
            "devotion_points": bio.devotion_points_unspent,
        });
        out["devotion_unlocked"] = json!(bio.total_devotion_unlocked);
    }
    let Some(skills) = file.skills() else {
        out["note"] = json!("block 8 (skills) is not typed in this file");
        return out;
    };
    out["masteries_allowed"] = json!(skills.masteries_allowed);
    out["reclaimed"] = json!({
        "skill_points": skills.skill_reclamation_points_used,
        "devotion_points": skills.devotion_reclamation_points_used,
    });
    match rules {
        Ok(rules) => group_skills(
            game,
            &rules.base.masteries,
            constellations,
            skills,
            &mut out,
        ),
        Err(error) => {
            out["note"] = json!(format!(
                "mastery trees unavailable ({error}); skills ungrouped"
            ));
            out["ungrouped"] = json!(
                skills
                    .skills
                    .iter()
                    .map(|skill| learned_json(game, skill))
                    .collect::<Vec<_>>()
            );
        }
    }
    out["item_skills"] = json!(
        skills
            .item_skills
            .iter()
            .map(|skill| item_skill_json(game, skill))
            .collect::<Vec<_>>()
    );
    out
}

fn group_skills(
    game: &GameData,
    trees: &[MasteryTree],
    constellations: &Constellations,
    skills: &Skills,
    out: &mut Value,
) {
    let mut by_mastery: Vec<(&MasteryTree, Vec<&Skill>)> = Vec::new();
    let mut devotions = Vec::new();
    let mut standard = 0_usize;
    let mut other = Vec::new();
    for skill in &skills.skills {
        match home_of(trees, skill) {
            Home::Mastery(tree) => match by_mastery.iter_mut().find(|(t, _)| t.index == tree.index)
            {
                Some((_, learned)) => learned.push(skill),
                None => by_mastery.push((tree, vec![skill])),
            },
            Home::Devotion => devotions.push(skill),
            Home::Standard => standard += 1,
            Home::Other => other.push(learned_json(game, skill)),
        }
    }
    by_mastery.sort_by_key(|(tree, _)| tree.index);
    out["masteries"] = json!(
        by_mastery
            .iter()
            .map(|(tree, learned)| mastery_json(game, tree, learned))
            .collect::<Vec<_>>()
    );
    out["devotions"] = devotions_json(game, constellations, &devotions);
    out["standard_skills"] = json!(standard);
    out["other_skills"] = json!(other);
}

/// The devotion stars taken, grouped by constellation in the game's
/// order, with each constellation's completion; a star no
/// constellation claims is listed under `unplaced`.
fn devotions_json(game: &GameData, constellations: &Constellations, taken: &[&Skill]) -> Value {
    let mut by_constellation: Vec<(usize, Value)> = Vec::new();
    let mut unplaced = Vec::new();
    for skill in taken {
        let Some((constellation, star)) = constellations.of_star(&skill.name) else {
            unplaced.push(learned_json(game, skill));
            continue;
        };
        let position = constellations
            .all()
            .iter()
            .position(|candidate| candidate.record == constellation.record)
            .unwrap_or(usize::MAX);
        let star_json = json!({
            "star": star.index,
            "skill": skill.name,
            "name": skill_name(game, &skill.name),
            "devotion_level": skill.devotion_level,
            "experience": skill.experience,
        });
        match by_constellation.iter_mut().find(|(at, _)| *at == position) {
            Some((_, entry)) => {
                if let Some(stars) = entry["stars"].as_array_mut() {
                    stars.push(star_json);
                }
            }
            None => by_constellation.push((
                position,
                json!({
                    "constellation": constellation.name,
                    "record": constellation.record,
                    "stars_total": constellation.stars.len(),
                    "stars": [star_json],
                }),
            )),
        }
    }
    by_constellation.sort_by_key(|(position, _)| *position);
    let mut points = unplaced.len();
    let constellations: Vec<Value> = by_constellation
        .into_iter()
        .map(|(_, mut entry)| {
            let taken = entry["stars"].as_array().map_or(0, Vec::len);
            let total = entry["stars_total"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok());
            points += taken;
            entry["stars_taken"] = json!(taken);
            entry["complete"] = json!(total == Some(taken));
            entry
        })
        .collect();
    json!({
        "points_placed": points,
        "constellations": constellations,
        "unplaced": unplaced,
    })
}
