//! What the record database admits into `formulas.gst`
//! ([`crate::formulas`]), and the ops over it: add one blueprint,
//! export the list as a self-describing JSON document, import such a
//! document into another campaign's list. Adds only — nothing here
//! removes an entry (ARCHITECTURE.md "Source of truth").
//!
//! A blueprint is a record of class [`BLUEPRINT_CLASS`]: every entry
//! of the user's own `formulas.gst`, `formulas.dst`, and mod file is
//! one (surveyed 2026-09-06), and the class is what the game's own
//! crafting UI keys on. The database is the gate: a record it does not
//! have, or has under another class, is refused with a typed error
//! rather than written into a file the game will read.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use univault_engine::ids::RecordId;

use crate::campaign::Campaign;
use crate::formulas::{BlueprintEntry, FormulaRead, Formulas};
use crate::gamedata::{GameData, GameDataError, ItemClass, ItemInfo};
use crate::gst::Added;
use crate::interchange::{self, EnvelopeError, ImportReport};
use crate::store::Timestamp;

/// The record `Class` of every blueprint.
pub const BLUEPRINT_CLASS: &str = "ItemArtifactFormula";

/// The `format` tag of a blueprint export.
pub const EXPORT_FORMAT: &str = "grimvault-blueprints";
/// The newest export version this crate reads and the one it writes.
pub const EXPORT_VERSION: u32 = 1;

/// Why a record was not added to the list.
#[derive(Debug, Error)]
pub enum BlueprintError {
    #[error("{0} is not in the record database")]
    UnknownRecord(RecordId),
    #[error("{record} is {}, not a blueprint ({BLUEPRINT_CLASS})", class_label(.class.as_ref()))]
    NotABlueprint {
        record: RecordId,
        class: Option<ItemClass>,
    },
    #[error("{0} is already a known blueprint")]
    AlreadyKnown(RecordId),
    #[error("record database: {0}")]
    Database(#[from] GameDataError),
}

fn class_label(class: Option<&ItemClass>) -> String {
    class.map_or_else(|| "of no class".to_string(), ToString::to_string)
}

/// The database's word on `record`: its item facts when it is a
/// blueprint.
///
/// # Errors
/// [`BlueprintError::UnknownRecord`], [`BlueprintError::NotABlueprint`],
/// or a database read failure.
pub fn check_blueprint(game: &GameData, record: &RecordId) -> Result<ItemInfo, BlueprintError> {
    let info = game
        .item_info(record)
        .ok_or_else(|| BlueprintError::UnknownRecord(record.clone()))??;
    if info
        .class
        .as_ref()
        .is_some_and(|class| class.as_str() == BLUEPRINT_CLASS)
    {
        Ok(info)
    } else {
        Err(BlueprintError::NotABlueprint {
            record: record.clone(),
            class: info.class,
        })
    }
}

/// Appends `record` as an unread blueprint once the database vouches
/// for it.
///
/// # Errors
/// [`BlueprintError`], including `AlreadyKnown` when the list has it.
pub fn add_blueprint(
    formulas: &mut Formulas,
    game: &GameData,
    record: RecordId,
) -> Result<(), BlueprintError> {
    check_blueprint(game, &record)?;
    match formulas.add(BlueprintEntry {
        record: record.as_str().to_owned(),
        read: FormulaRead::Unread,
    }) {
        Added::Added => Ok(()),
        Added::AlreadyKnown => Err(BlueprintError::AlreadyKnown(record)),
    }
}

/// Every blueprint record the database has, by path.
#[must_use]
pub fn available_blueprints(game: &GameData) -> Vec<RecordId> {
    let mut records: Vec<RecordId> = game.record_ids_of_type(BLUEPRINT_CLASS).cloned().collect();
    records.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    records
}

/// One entry of an export: the record and its read flag, as the file
/// held them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedBlueprint {
    pub record: String,
    pub read: FormulaRead,
}

/// A campaign's blueprint list as an interchange document:
///
/// ```json
/// {
///   "format": "grimvault-blueprints",
///   "version": 1,
///   "campaign": "main",
///   "exportedAt": 1756900000,
///   "entries": [ { "record": "records/items/crafting/blueprints/...dbr", "read": true } ]
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlueprintExport {
    pub campaign: Campaign,
    pub exported_at: Timestamp,
    pub entries: Vec<ExportedBlueprint>,
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    format: Cow<'a, str>,
    version: u32,
    campaign: Cow<'a, Campaign>,
    exported_at: Timestamp,
    entries: Cow<'a, [ExportedBlueprint]>,
    #[serde(flatten)]
    extra: Cow<'a, Map<String, Value>>,
}

impl BlueprintExport {
    /// The list as it stands, stamped with where and when it was taken.
    #[must_use]
    pub fn of(formulas: &Formulas, campaign: Campaign, exported_at: Timestamp) -> Self {
        Self {
            campaign,
            exported_at,
            entries: formulas
                .entries
                .iter()
                .map(|entry| ExportedBlueprint {
                    record: entry.record.clone(),
                    read: entry.read,
                })
                .collect(),
            extra: Map::new(),
        }
    }

    /// The document, pretty-printed with a trailing newline.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "the expect guards a local invariant (plain data serializes), not a runtime condition"
    )]
    pub fn to_json(&self) -> Vec<u8> {
        let document = ExportFile {
            format: Cow::Borrowed(EXPORT_FORMAT),
            version: EXPORT_VERSION,
            campaign: Cow::Borrowed(&self.campaign),
            exported_at: self.exported_at,
            entries: Cow::Borrowed(&self.entries),
            extra: Cow::Borrowed(&self.extra),
        };
        let mut bytes = serde_json::to_vec_pretty(&document)
            .expect("export types serialize infallibly: string keys only, no fallible Serialize");
        bytes.push(b'\n');
        bytes
    }

    /// Parses a document, refusing any other kind by its tag.
    ///
    /// # Errors
    /// [`EnvelopeError`].
    pub fn from_json(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        interchange::check(bytes, EXPORT_FORMAT, EXPORT_VERSION)?;
        let document: ExportFile<'_> = serde_json::from_slice(bytes)?;
        Ok(Self {
            campaign: document.campaign.into_owned(),
            exported_at: document.exported_at,
            entries: document.entries.into_owned(),
            extra: document.extra.into_owned(),
        })
    }
}

/// Adds every entry of `export` the database vouches for and the list
/// lacks, keeping each entry's read flag; the rest are counted or
/// refused in the report, and nothing is removed.
pub fn import_blueprints(
    formulas: &mut Formulas,
    game: &GameData,
    export: &BlueprintExport,
) -> ImportReport<BlueprintError> {
    let mut report = ImportReport::default();
    for entry in &export.entries {
        let Some(record) = RecordId::parse(entry.record.clone()) else {
            continue;
        };
        match check_blueprint(game, &record) {
            Ok(_) => match formulas.add(BlueprintEntry {
                record: entry.record.clone(),
                read: entry.read,
            }) {
                Added::Added => report.added += 1,
                Added::AlreadyKnown => report.already_known += 1,
            },
            Err(error) => report.refused.push((entry.record.clone(), error)),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::formulas::FormulasVersion;
    use crate::gamedata::fixture::game_with;

    const BOOTS: &str = "records/items/crafting/blueprints/armor/craft_feet_squiresboots02.dbr";
    const SEAL: &str = "records/items/crafting/blueprints/relics/craft_relic_sealnight.dbr";
    const HELM: &str = "records/items/gearhead/a01_head.dbr";

    fn game() -> GameData {
        game_with(&[
            (BOOTS, BLUEPRINT_CLASS, &[("itemNameTag", "tagBoots")]),
            (SEAL, BLUEPRINT_CLASS, &[("itemNameTag", "tagSeal")]),
            (HELM, "ArmorProtective_Head", &[("itemNameTag", "tagHelm")]),
        ])
    }

    fn id(record: &str) -> RecordId {
        RecordId::parse(record.to_string()).unwrap()
    }

    fn formulas(records: &[&str]) -> Formulas {
        Formulas {
            version: FormulasVersion::new(3).unwrap(),
            expansion_status: 7,
            entries: records
                .iter()
                .map(|record| BlueprintEntry {
                    record: (*record).to_string(),
                    read: FormulaRead::Read,
                })
                .collect(),
        }
    }

    #[test]
    fn the_database_vouches_for_blueprints_only() {
        let game = game();
        let boots = check_blueprint(&game, &id(BOOTS)).unwrap();
        assert_eq!(boots.class.unwrap().as_str(), BLUEPRINT_CLASS);
        assert_eq!(boots.name, "craft_feet_squiresboots02");
        assert!(matches!(
            check_blueprint(&game, &id(HELM)),
            Err(BlueprintError::NotABlueprint { class: Some(class), .. })
                if class.as_str() == "ArmorProtective_Head"
        ));
        assert!(matches!(
            check_blueprint(&game, &id("records/items/nothing.dbr")),
            Err(BlueprintError::UnknownRecord(_))
        ));
        assert_eq!(available_blueprints(&game), vec![id(BOOTS), id(SEAL)]);
    }

    #[test]
    fn add_appends_an_unread_entry_once() {
        let game = game();
        let mut list = formulas(&[BOOTS]);
        add_blueprint(&mut list, &game, id(SEAL)).unwrap();
        assert_eq!(
            list.entries[1],
            BlueprintEntry {
                record: SEAL.into(),
                read: FormulaRead::Unread
            }
        );
        assert!(matches!(
            add_blueprint(&mut list, &game, id(SEAL)),
            Err(BlueprintError::AlreadyKnown(_))
        ));
        assert!(matches!(
            add_blueprint(&mut list, &game, id(HELM)),
            Err(BlueprintError::NotABlueprint { .. })
        ));
        assert_eq!(list.entries.len(), 2);
    }

    #[test]
    fn an_export_round_trips_with_its_identity_and_unknown_fields() {
        let list = formulas(&[BOOTS, SEAL]);
        let export = BlueprintExport::of(&list, Campaign::Main, Timestamp::from_unix_seconds(5));
        let bytes = export.to_json();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["format"], json!(EXPORT_FORMAT));
        assert_eq!(value["version"], json!(EXPORT_VERSION));
        assert_eq!(value["campaign"], json!("main"));
        assert_eq!(value["exportedAt"], json!(5));
        assert_eq!(value["entries"][1], json!({ "record": SEAL, "read": true }));
        assert_eq!(BlueprintExport::from_json(&bytes).unwrap(), export);

        let with_extra = br#"{"format":"grimvault-blueprints","version":1,"campaign":"LootAscension","exportedAt":9,"entries":[],"note":"keep"}"#;
        let parsed = BlueprintExport::from_json(with_extra).unwrap();
        assert_eq!(parsed.campaign.to_string(), "LootAscension");
        let value: Value = serde_json::from_slice(&parsed.to_json()).unwrap();
        assert_eq!(value["note"], json!("keep"));
    }

    #[test]
    fn foreign_and_newer_documents_are_refused_by_the_envelope() {
        assert!(matches!(
            BlueprintExport::from_json(br#"{"format":"grimvault-illusions","version":1}"#),
            Err(EnvelopeError::WrongFormat { .. })
        ));
        assert!(matches!(
            BlueprintExport::from_json(br#"{"format":"grimvault-blueprints","version":2}"#),
            Err(EnvelopeError::UnsupportedVersion { version: 2, .. })
        ));
    }

    #[test]
    fn import_adds_what_the_database_vouches_for_and_reports_the_rest() {
        let game = game();
        let mut list = formulas(&[BOOTS]);
        let export = BlueprintExport {
            campaign: Campaign::Main,
            exported_at: Timestamp::from_unix_seconds(0),
            entries: [BOOTS, SEAL, HELM, "records/items/nothing.dbr", ""]
                .into_iter()
                .map(|record| ExportedBlueprint {
                    record: record.into(),
                    read: FormulaRead::Unread,
                })
                .collect(),
            extra: Map::new(),
        };
        let report = import_blueprints(&mut list, &game, &export);
        assert_eq!((report.added, report.already_known), (1, 1));
        assert_eq!(report.refused.len(), 2);
        assert!(report.changed());
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.entries[1].read, FormulaRead::Unread);
        assert!(matches!(
            report.refused[0],
            (ref record, BlueprintError::NotABlueprint { .. }) if record == HELM
        ));
        assert!(matches!(
            report.refused[1],
            (_, BlueprintError::UnknownRecord(_))
        ));
    }
}
