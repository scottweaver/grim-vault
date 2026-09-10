//! Devotion: the constellations as the game's own devotion window
//! defines them. Each `records/ui/skills/devotion/constellations/
//! constellationNN.dbr` (template `devotionconstellation.tpl`) names
//! its stars as button records (`devotionButtonN`), each of which
//! names the star's skill record — the record a character's block 8
//! lists once the star is taken — the affinity the constellation
//! requires and grants, and which star each hangs from
//! (`devotionLinksN`). Surveyed on the user's install, 2026-09-10.

use std::collections::HashMap;

use grimvault_core::gamedata::GameData;
use grimvault_core::stats::{Scale, SkillLevel};
use serde_json::{Value, json};
use univault_engine::arz::DbRecord;
use univault_engine::ids::{RecordId, normalize};

use crate::build::{skill_name, skill_record};
use crate::view::line_texts;

/// The folder the game's constellation records live under.
const CONSTELLATION_FOLDER: &str = "RECORDS\\UI\\SKILLS\\DEVOTION\\CONSTELLATIONS\\";

/// The template every constellation record uses.
const CONSTELLATION_TEMPLATE: &str = "devotionconstellation.tpl";

/// One star of a constellation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Star {
    /// The button's number in the constellation record, from 1.
    pub index: u32,
    /// The star's skill record, as the save lists it.
    pub skill: String,
    /// The number of the star this one hangs from, when it is not a
    /// root.
    pub links_from: Option<u32>,
}

/// An affinity amount the constellation requires or grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affinity {
    pub name: String,
    pub amount: i32,
}

/// One constellation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constellation {
    pub record: String,
    pub name: String,
    pub description: Option<String>,
    pub required: Vec<Affinity>,
    pub given: Vec<Affinity>,
    pub stars: Vec<Star>,
}

/// Every constellation, and which one each star skill belongs to.
#[derive(Clone, Debug, Default)]
pub struct Constellations {
    all: Vec<Constellation>,
    by_star: HashMap<String, (usize, usize)>,
}

impl Constellations {
    /// Reads every constellation record in the game's folder, in
    /// record order.
    #[must_use]
    pub fn read(game: &GameData) -> Self {
        let mut ids: Vec<&RecordId> = game
            .record_ids()
            .filter(|id| normalize(id.as_str()).starts_with(CONSTELLATION_FOLDER))
            .collect();
        ids.sort_by_key(|id| normalize(id.as_str()));
        let all: Vec<Constellation> = ids
            .into_iter()
            .filter_map(|id| {
                let record = game.record(id)?.ok()?;
                constellation(game, &record)
            })
            .collect();
        let mut by_star = HashMap::new();
        for (which, constellation) in all.iter().enumerate() {
            for (position, star) in constellation.stars.iter().enumerate() {
                by_star.insert(normalize(&star.skill), (which, position));
            }
        }
        Self { all, by_star }
    }

    #[must_use]
    pub fn all(&self) -> &[Constellation] {
        &self.all
    }

    /// The constellation and star a devotion skill record belongs to.
    #[must_use]
    pub fn of_star(&self, skill: &str) -> Option<(&Constellation, &Star)> {
        let (which, position) = self.by_star.get(&normalize(skill))?;
        let constellation = self.all.get(*which)?;
        Some((constellation, constellation.stars.get(*position)?))
    }

    /// The constellation a caller means: by name (case-insensitive,
    /// then as a substring) or by a substring of its record path.
    ///
    /// # Errors
    /// The known names, when none matches.
    pub fn resolve(&self, wanted: &str) -> Result<&Constellation, String> {
        let wanted = wanted.trim();
        let lowered = wanted.to_lowercase();
        self.all
            .iter()
            .find(|constellation| constellation.name.eq_ignore_ascii_case(wanted))
            .or_else(|| {
                self.all
                    .iter()
                    .find(|constellation| constellation.name.to_lowercase().contains(&lowered))
            })
            .or_else(|| {
                self.all.iter().find(|constellation| {
                    normalize(&constellation.record).contains(&normalize(wanted))
                })
            })
            .ok_or_else(|| {
                let names: Vec<&str> = self
                    .all
                    .iter()
                    .map(|constellation| constellation.name.as_str())
                    .collect();
                format!(
                    "no constellation matching '{wanted}' (known: {})",
                    names.join(", ")
                )
            })
    }
}

fn constellation(game: &GameData, record: &DbRecord) -> Option<Constellation> {
    if !record
        .string("templateName")
        .is_some_and(|template| template.to_lowercase().ends_with(CONSTELLATION_TEMPLATE))
    {
        return None;
    }
    let name = record
        .string("constellationDisplayTag")
        .and_then(|tag| game.tag_text(tag))
        .map(str::to_string)
        .or_else(|| record.string("FileDescription").map(str::to_string))?;
    let description = record
        .string("constellationInfoTag")
        .and_then(|tag| game.tag_text(tag))
        .map(str::to_string);
    let affinities = |prefix: &str| -> Vec<Affinity> {
        (1..=8)
            .filter_map(|n| {
                let name = record.string(&format!("{prefix}Name{n}"))?;
                let amount = record.integer(&format!("{prefix}{n}"))?;
                (!name.is_empty() && amount > 0).then(|| Affinity {
                    name: name.to_string(),
                    amount,
                })
            })
            .collect()
    };
    let stars = (1..=32)
        .filter_map(|n| {
            let button = record.string(&format!("devotionButton{n}"))?;
            let skill = RecordId::parse(button.to_string())
                .and_then(|id| game.record(&id)?.ok())
                .and_then(|button| button.string("skillName").map(str::to_string))?;
            Some(Star {
                index: n,
                skill,
                links_from: record
                    .integer(&format!("devotionLinks{n}"))
                    .and_then(|link| u32::try_from(link).ok())
                    .filter(|link| *link > 0),
            })
        })
        .collect();
    Some(Constellation {
        record: record.id.as_str().to_string(),
        name,
        description,
        required: affinities("affinityRequired"),
        given: affinities("affinityGiven"),
        stars,
    })
}

fn affinity_json(affinities: &[Affinity]) -> Value {
    json!(
        affinities
            .iter()
            .map(|affinity| json!({"affinity": affinity.name, "amount": affinity.amount}))
            .collect::<Vec<_>>()
    )
}

/// Every constellation in brief: name, record, affinities, star count.
#[must_use]
pub fn list_json(constellations: &Constellations) -> Vec<Value> {
    constellations
        .all()
        .iter()
        .map(|constellation| {
            json!({
                "name": constellation.name,
                "record": constellation.record,
                "requires": affinity_json(&constellation.required),
                "grants": affinity_json(&constellation.given),
                "stars": constellation.stars.len(),
            })
        })
        .collect()
}

/// One star with its skill's name and class, and its stat lines at
/// level 1 (a devotion star has one level; a proc's lines grow with
/// the level the character's devotion experience gives it).
#[must_use]
pub fn star_json(game: &GameData, star: &Star) -> Value {
    let mut out = json!({
        "star": star.index,
        "skill": star.skill,
        "name": skill_name(game, &star.skill),
        "links_from": star.links_from,
    });
    if let Some(found) = skill_record(game, &star.skill) {
        out["class"] = json!(found.record_type);
        out["max_level"] = json!(found.integer("skillMaxLevel"));
    }
    if let Some(stats) = RecordId::parse(star.skill.clone())
        .and_then(|id| game.record_stats(&id, SkillLevel::ONE, Scale::NONE))
    {
        out["lines"] = json!(line_texts(&stats.lines));
    }
    out
}

/// One constellation in full: its affinities and every star with its
/// lines.
#[must_use]
pub fn constellation_json(game: &GameData, constellation: &Constellation) -> Value {
    json!({
        "name": constellation.name,
        "record": constellation.record,
        "description": constellation.description,
        "requires": affinity_json(&constellation.required),
        "grants": affinity_json(&constellation.given),
        "stars": constellation
            .stars
            .iter()
            .map(|star| star_json(game, star))
            .collect::<Vec<_>>(),
    })
}
