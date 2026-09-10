//! Full-database access behind the record tools: a lazily built
//! search index over every record, and the JSON shaping of a raw
//! record's variables. Pure over already-loaded data — no IO here.

use std::collections::BTreeMap;

use grimvault_core::gamedata::GameData;
use serde_json::{Value, json};
use univault_engine::arz::{DbRecord, DbValues};
use univault_engine::ids::{RecordId, normalize};

/// One searchable record: its path as the database spells it, its
/// class, its template, and the names the database gives it — the
/// localized one where a name tag resolves, and the developers'
/// `FileDescription` where set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: String,
    pub class: String,
    pub template: Option<String>,
    pub name: Option<String>,
    pub file_description: Option<String>,
}

impl IndexEntry {
    /// Whether the entry mentions the needle in its path (compared in
    /// the database's own normalized spelling, so either slash and any
    /// case match) or, lowercased, in either name.
    #[must_use]
    pub fn mentions(&self, needle: &Needle) -> bool {
        normalize(&self.path).contains(&needle.path)
            || self
                .name
                .as_deref()
                .is_some_and(|name| name.to_lowercase().contains(&needle.text))
            || self
                .file_description
                .as_deref()
                .is_some_and(|name| name.to_lowercase().contains(&needle.text))
    }

    /// Whether the entry's template file is `name` (case-insensitive,
    /// the file name only).
    #[must_use]
    pub fn uses_template(&self, name: &str) -> bool {
        self.template
            .as_deref()
            .is_some_and(|template| template.to_lowercase().ends_with(&name.to_lowercase()))
    }
}

/// A search needle prepared both ways an entry is matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Needle {
    path: String,
    text: String,
}

impl Needle {
    #[must_use]
    pub fn new(raw: &str) -> Self {
        let trimmed = raw.trim();
        Self {
            path: normalize(trimmed),
            text: trimmed.to_lowercase(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Variables whose translated value serves as a record's display
/// name, in lookup order (monsters use `description`, skills their
/// display tag, items their name tag).
const NAME_VARIABLES: [&str; 3] = ["description", "skillDisplayName", "itemNameTag"];

/// The localized name a record carries, if any of its name tags
/// resolves.
#[must_use]
pub fn display_name(game: &GameData, record: &DbRecord) -> Option<String> {
    NAME_VARIABLES.iter().find_map(|variable| {
        let tag = record.string(variable)?;
        if tag.is_empty() {
            return None;
        }
        game.tag_text(tag).map(str::to_string)
    })
}

/// Decodes every record of every layer into search-index entries.
/// Expensive (several seconds over the shipped layers) — built once
/// and kept.
#[must_use]
pub fn build_index(game: &GameData) -> Vec<IndexEntry> {
    let mut entries: Vec<IndexEntry> = game
        .record_ids()
        .filter_map(|id| {
            let record = game.record(id)?.ok()?;
            Some(IndexEntry {
                path: id.as_str().to_string(),
                class: record.record_type.clone(),
                template: record
                    .string("templateName")
                    .filter(|text| !text.is_empty())
                    .map(str::to_string),
                name: display_name(game, &record),
                file_description: record
                    .string("FileDescription")
                    .filter(|text| !text.is_empty())
                    .map(str::to_string),
            })
        })
        .collect();
    entries.sort_by_key(|entry| normalize(&entry.path));
    entries
}

/// The record classes in use and how many records each has.
#[must_use]
pub fn class_counts(game: &GameData) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for (_, class) in game.record_types() {
        *counts.entry(class.to_string()).or_insert(0) += 1;
    }
    counts
}

/// A whole record as JSON. `everything: false` drops the template's
/// resting defaults (all-zero, all-false, all-empty variables) that
/// dominate raw records; `true` is the byte-faithful dump. `layers`
/// names, topmost first, the layers that define the record.
#[must_use]
pub fn record_json(
    game: &GameData,
    record: &DbRecord,
    layers: &[String],
    everything: bool,
) -> Value {
    let mut variables: BTreeMap<String, Value> = BTreeMap::new();
    let mut translated: BTreeMap<String, String> = BTreeMap::new();
    for variable in record.variables() {
        if !everything && default_valued(&variable.values) {
            continue;
        }
        variables.insert(variable.name.clone(), values_json(&variable.values));
        if let DbValues::Strings(values) = &variable.values
            && let Some(text) = values
                .iter()
                .find(|value| !value.is_empty())
                .and_then(|tag| game.tag_text(tag))
        {
            translated.insert(variable.name.clone(), text.to_string());
        }
    }
    let mut out = json!({
        "record": record.id.as_str(),
        "class": record.record_type,
        "defined_in": layers,
        "variables": variables,
    });
    if !translated.is_empty() {
        out["translated"] = json!(translated);
    }
    if !everything {
        out["note"] = json!(
            "template-default variables (all zero/false/empty) omitted; \
             pass everything: true for the byte-faithful dump"
        );
    }
    out
}

/// The names of the layers that define `id`, topmost first.
#[must_use]
pub fn layers_of(game: &GameData, layers: &[String], id: &RecordId) -> Vec<String> {
    game.defining_layers(id)
        .filter_map(|index| layers.get(index).cloned())
        .collect()
}

fn default_valued(values: &DbValues) -> bool {
    match values {
        DbValues::Integers(values) => values.iter().all(|&value| value == 0),
        DbValues::Floats(values) => values.iter().all(|&value| value == 0.0),
        DbValues::Strings(values) => values.iter().all(String::is_empty),
        DbValues::Booleans(values) => values.iter().all(|&value| !value),
    }
}

/// Values as JSON: a one-element array unwraps to its scalar (the
/// overwhelmingly common shape), floats lose their f32 noise.
#[must_use]
pub fn values_json(values: &DbValues) -> Value {
    fn unwrap_single(mut rendered: Vec<Value>) -> Value {
        if rendered.len() == 1 {
            rendered.remove(0)
        } else {
            Value::Array(rendered)
        }
    }
    match values {
        DbValues::Integers(values) => unwrap_single(values.iter().map(|&v| json!(v)).collect()),
        DbValues::Floats(values) => unwrap_single(values.iter().map(|&v| float_json(v)).collect()),
        DbValues::Strings(values) => unwrap_single(values.iter().map(|v| json!(v)).collect()),
        DbValues::Booleans(values) => unwrap_single(values.iter().map(|&v| json!(v)).collect()),
    }
}

fn float_json(value: f32) -> Value {
    json!((f64::from(value) * 10_000.0).round() / 10_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_unwrap_singles_and_round_float_noise() {
        assert_eq!(values_json(&DbValues::Integers(vec![7])), json!(7));
        assert_eq!(values_json(&DbValues::Integers(vec![1, 2])), json!([1, 2]));
        assert_eq!(values_json(&DbValues::Floats(vec![0.1])), json!(0.1));
        assert_eq!(
            values_json(&DbValues::Strings(vec!["a".into(), String::new()])),
            json!(["a", ""])
        );
    }

    #[test]
    fn template_defaults_are_recognized() {
        assert!(default_valued(&DbValues::Floats(vec![0.0, 0.0])));
        assert!(default_valued(&DbValues::Strings(vec![String::new()])));
        assert!(!default_valued(&DbValues::Floats(vec![0.0, 2.5])));
        assert!(!default_valued(&DbValues::Booleans(vec![true])));
    }

    #[test]
    fn an_index_entry_mentions_its_path_and_either_name() {
        let entry = IndexEntry {
            path: "records/skills/playerclass01/cadence.dbr".into(),
            class: "Skill_AttackRadius".into(),
            template: Some("database/templates/skill_attackradius.tpl".into()),
            name: Some("Cadence".into()),
            file_description: Some("Soldier: Cadence".into()),
        };
        assert!(entry.mentions(&Needle::new("playerclass01")));
        assert!(entry.mentions(&Needle::new("records/skills/PLAYERCLASS01/")));
        assert!(entry.mentions(&Needle::new("records\\skills\\playerclass01")));
        assert!(entry.mentions(&Needle::new("cadence")));
        assert!(entry.mentions(&Needle::new("soldier:")));
        assert!(!entry.mentions(&Needle::new("blade")));
        assert!(entry.uses_template("skill_attackradius.tpl"));
        assert!(!entry.uses_template("itemset.tpl"));
    }
}
