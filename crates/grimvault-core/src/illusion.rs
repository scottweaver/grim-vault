//! What the record database admits into `transmutes.gst`
//! ([`crate::gst::Illusions`]) and where: the nine equipment
//! categories the game keeps illusions under, the rule that maps a
//! record's `Class` onto one, and the ops — add one illusion, export
//! the collection as a self-describing JSON document, import such a
//! document, audit a file against the database. Adds only — nothing
//! here removes an entry (ARCHITECTURE.md "Source of truth").
//!
//! Slot ids 1 head, 3 torso, 4 legs, 5 feet, 7 hands, 8 off-hand
//! (foci and shields), 9 weapon (every weapon class), 14 shoulders,
//! 15 medal: the nine ids in the user's file, corroborated by GD
//! Stash's constants (eyes-only) and confirmed record by record on
//! the user's own collections 2026-09-06 — every listed record's
//! `Class` maps to the id of the list it sits in ([`audit`] is that
//! check, and `--check` runs it). The mapping follows the class
//! naming scheme: `ArmorProtective_<Part>` is that part,
//! `ArmorJewelry_Medal` the medal, `WeaponArmor_*` the off-hand,
//! every other `Weapon*` the weapon slot; belts, rings, and amulets
//! have no illusion.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use univault_engine::ids::RecordId;

use crate::campaign::Campaign;
use crate::gamedata::{GameData, GameDataError, ItemClass};
use crate::gst::{Added, IllusionSlot, Illusions};
use crate::interchange::{self, EnvelopeError, ImportReport};
use crate::store::Timestamp;

/// The `format` tag of an illusion export.
pub const EXPORT_FORMAT: &str = "grimvault-illusions";
/// The newest export version this crate reads and the one it writes.
pub const EXPORT_VERSION: u32 = 1;

/// The equipment category an illusion belongs to, keyed by the slot id
/// the file stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IllusionCategory {
    Head,
    Torso,
    Legs,
    Feet,
    Hands,
    OffHand,
    Weapon,
    Shoulders,
    Medal,
}

impl IllusionCategory {
    /// Every category in the file's order (ascending slot id).
    pub const ALL: [Self; 9] = [
        Self::Head,
        Self::Torso,
        Self::Legs,
        Self::Feet,
        Self::Hands,
        Self::OffHand,
        Self::Weapon,
        Self::Shoulders,
        Self::Medal,
    ];

    /// The slot id the file stores for this category.
    #[must_use]
    pub const fn slot_id(self) -> u32 {
        match self {
            Self::Head => 1,
            Self::Torso => 3,
            Self::Legs => 4,
            Self::Feet => 5,
            Self::Hands => 7,
            Self::OffHand => 8,
            Self::Weapon => 9,
            Self::Shoulders => 14,
            Self::Medal => 15,
        }
    }

    /// The category of a stored slot id; `None` for an id no sample
    /// has, which the file model still carries verbatim.
    #[must_use]
    pub fn from_slot_id(id: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.slot_id() == id)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Head => "Head",
            Self::Torso => "Torso",
            Self::Legs => "Legs",
            Self::Feet => "Feet",
            Self::Hands => "Hands",
            Self::OffHand => "Off-hand",
            Self::Weapon => "Weapons",
            Self::Shoulders => "Shoulders",
            Self::Medal => "Medal",
        }
    }

    /// The category a record of `class` unlocks an illusion for, by
    /// the class naming scheme; `None` for gear with no illusion.
    #[must_use]
    pub fn of_class(class: &ItemClass) -> Option<Self> {
        match class.as_str() {
            "ArmorProtective_Head" => Some(Self::Head),
            "ArmorProtective_Shoulders" => Some(Self::Shoulders),
            "ArmorProtective_Chest" => Some(Self::Torso),
            "ArmorProtective_Hands" => Some(Self::Hands),
            "ArmorProtective_Legs" => Some(Self::Legs),
            "ArmorProtective_Feet" => Some(Self::Feet),
            "ArmorJewelry_Medal" => Some(Self::Medal),
            class if class.starts_with("WeaponArmor_") => Some(Self::OffHand),
            class if class.starts_with("Weapon") => Some(Self::Weapon),
            _ => None,
        }
    }
}

impl fmt::Display for IllusionCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (slot {})", self.label(), self.slot_id())
    }
}

/// Why a record was not added to the collection.
#[derive(Debug, Error)]
pub enum IllusionError {
    #[error("{0} is not in the record database")]
    UnknownRecord(RecordId),
    #[error("{record} is {}, which has no illusion", class_label(.class.as_ref()))]
    NotAnIllusion {
        record: RecordId,
        class: Option<ItemClass>,
    },
    #[error("{0} is already an unlocked illusion")]
    AlreadyKnown(RecordId),
    /// The record sits under (or would be put under) a slot id other
    /// than the one its class maps to.
    #[error("{record} is listed under slot {listed} but its class belongs to {category}")]
    Misfiled {
        record: RecordId,
        listed: u32,
        category: IllusionCategory,
    },
    #[error("record database: {0}")]
    Database(#[from] GameDataError),
}

fn class_label(class: Option<&ItemClass>) -> String {
    class.map_or_else(|| "of no class".to_string(), ToString::to_string)
}

/// The category the database puts `record` under.
///
/// # Errors
/// [`IllusionError::UnknownRecord`], [`IllusionError::NotAnIllusion`],
/// or a database read failure.
pub fn illusion_category(
    game: &GameData,
    record: &RecordId,
) -> Result<IllusionCategory, IllusionError> {
    let info = game
        .item_info(record)
        .ok_or_else(|| IllusionError::UnknownRecord(record.clone()))??;
    info.class
        .as_ref()
        .and_then(IllusionCategory::of_class)
        .ok_or(IllusionError::NotAnIllusion {
            record: record.clone(),
            class: info.class,
        })
}

/// Appends `record` to its category's list once the database vouches
/// for it, creating the list when the file has none for that slot.
///
/// # Errors
/// [`IllusionError`], including `AlreadyKnown` when any list has it.
pub fn add_illusion(
    illusions: &mut Illusions,
    game: &GameData,
    record: RecordId,
) -> Result<IllusionCategory, IllusionError> {
    let category = illusion_category(game, &record)?;
    match illusions.add(category.slot_id(), record.as_str().to_owned()) {
        Added::Added => Ok(category),
        Added::AlreadyKnown => Err(IllusionError::AlreadyKnown(record)),
    }
}

/// Every record the database would admit, by path — all gear with an
/// illusion category.
#[must_use]
pub fn available_illusions(game: &GameData) -> Vec<RecordId> {
    let mut records: Vec<RecordId> = game
        .record_types()
        .filter(|(_, class)| {
            IllusionCategory::of_class(&ItemClass::new((*class).to_string())).is_some()
        })
        .map(|(id, _)| id.clone())
        .collect();
    records.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    records
}

/// Every listed record the database disagrees with: unknown, of a
/// class with no illusion, or filed under another category's slot.
/// Empty on the user's own files.
#[must_use]
pub fn audit(illusions: &Illusions, game: &GameData) -> Vec<(String, IllusionError)> {
    illusions
        .slots
        .iter()
        .flat_map(|slot| slot.records.iter().map(move |record| (slot.slot, record)))
        .filter_map(|(listed, record)| {
            let id = RecordId::parse(record.clone())?;
            let verdict = illusion_category(game, &id).and_then(|category| {
                if category.slot_id() == listed {
                    Ok(())
                } else {
                    Err(IllusionError::Misfiled {
                        record: id,
                        listed,
                        category,
                    })
                }
            });
            verdict.err().map(|error| (record.clone(), error))
        })
        .collect()
}

/// One slot of an export: the stored slot id and its records, as the
/// file held them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportedSlot {
    pub slot: u32,
    pub records: Vec<String>,
}

/// A campaign's illusion collection as an interchange document:
///
/// ```json
/// {
///   "format": "grimvault-illusions",
///   "version": 1,
///   "campaign": "main",
///   "exportedAt": 1756900000,
///   "slots": [ { "slot": 1, "records": [ "records/items/gearhead/...dbr" ] } ]
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IllusionExport {
    pub campaign: Campaign,
    pub exported_at: Timestamp,
    pub slots: Vec<ExportedSlot>,
    extra: Map<String, Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    format: Cow<'a, str>,
    version: u32,
    campaign: Cow<'a, Campaign>,
    exported_at: Timestamp,
    slots: Cow<'a, [ExportedSlot]>,
    #[serde(flatten)]
    extra: Cow<'a, Map<String, Value>>,
}

impl IllusionExport {
    /// The collection as it stands, stamped with where and when it was
    /// taken.
    #[must_use]
    pub fn of(illusions: &Illusions, campaign: Campaign, exported_at: Timestamp) -> Self {
        Self {
            campaign,
            exported_at,
            slots: illusions
                .slots
                .iter()
                .map(|slot| ExportedSlot {
                    slot: slot.slot,
                    records: slot.records.clone(),
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
            slots: Cow::Borrowed(&self.slots),
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
            slots: document.slots.into_owned(),
            extra: document.extra.into_owned(),
        })
    }

    /// How many records the document lists.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.slots.iter().map(|slot| slot.records.len()).sum()
    }
}

/// Adds every record of `export` the database vouches for, under the
/// slot the document names — which must be the one the record's class
/// maps to, or the entry is refused as misfiled rather than re-homed.
/// Nothing is removed.
pub fn import_illusions(
    illusions: &mut Illusions,
    game: &GameData,
    export: &IllusionExport,
) -> ImportReport<IllusionError> {
    let mut report = ImportReport::default();
    for (listed, record) in export
        .slots
        .iter()
        .flat_map(|slot| slot.records.iter().map(move |record| (slot.slot, record)))
    {
        let Some(id) = RecordId::parse(record.clone()) else {
            continue;
        };
        let verdict = illusion_category(game, &id).and_then(|category| {
            if category.slot_id() == listed {
                Ok(category)
            } else {
                Err(IllusionError::Misfiled {
                    record: id.clone(),
                    listed,
                    category,
                })
            }
        });
        match verdict {
            Ok(category) => match illusions.add(category.slot_id(), record.clone()) {
                Added::Added => report.added += 1,
                Added::AlreadyKnown => report.already_known += 1,
            },
            Err(error) => report.refused.push((record.clone(), error)),
        }
    }
    report
}

/// The records of one category as listed, for a pane.
#[must_use]
pub fn records_of(illusions: &Illusions, category: IllusionCategory) -> &[String] {
    illusions
        .slot(category.slot_id())
        .map_or(&[], |slot: &IllusionSlot| slot.records.as_slice())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::gamedata::fixture::game_with;
    use crate::gst::IllusionsVersion;

    const HELM: &str = "records/items/gearhead/a01_head.dbr";
    const HELM2: &str = "records/items/gearhead/a02_head.dbr";
    const SHIELD: &str = "records/items/gearweapons/shields/a01_shield.dbr";
    const SWORD: &str = "records/items/gearweapons/swords2h/a01_sword.dbr";
    const BELT: &str = "records/items/gearaccessories/belts/a01_belt.dbr";

    fn game() -> GameData {
        game_with(&[
            (HELM, "ArmorProtective_Head", &[]),
            (HELM2, "ArmorProtective_Head", &[]),
            (SHIELD, "WeaponArmor_Shield", &[]),
            (SWORD, "WeaponMelee_Sword2h", &[]),
            (BELT, "ArmorProtective_Waist", &[]),
        ])
    }

    fn id(record: &str) -> RecordId {
        RecordId::parse(record.to_string()).unwrap()
    }

    fn collection(slots: &[(u32, &[&str])]) -> Illusions {
        Illusions {
            version: IllusionsVersion::new(2).unwrap(),
            mod_name: String::new(),
            expansion_status: 7,
            slots: slots
                .iter()
                .map(|(slot, records)| IllusionSlot {
                    slot: *slot,
                    records: records.iter().map(|r| (*r).to_string()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn categories_map_to_the_files_slot_ids_and_back() {
        for category in IllusionCategory::ALL {
            assert_eq!(
                IllusionCategory::from_slot_id(category.slot_id()),
                Some(category)
            );
        }
        let ids: Vec<u32> = IllusionCategory::ALL
            .iter()
            .map(|category| category.slot_id())
            .collect();
        assert_eq!(ids, [1, 3, 4, 5, 7, 8, 9, 14, 15]);
        assert_eq!(IllusionCategory::from_slot_id(2), None);
        assert_eq!(IllusionCategory::Weapon.to_string(), "Weapons (slot 9)");
    }

    #[test]
    fn classes_map_by_the_naming_scheme() {
        let of = |class: &str| IllusionCategory::of_class(&ItemClass::new(class.to_string()));
        assert_eq!(of("ArmorProtective_Head"), Some(IllusionCategory::Head));
        assert_eq!(
            of("ArmorProtective_Shoulders"),
            Some(IllusionCategory::Shoulders)
        );
        assert_eq!(of("ArmorProtective_Chest"), Some(IllusionCategory::Torso));
        assert_eq!(of("ArmorProtective_Hands"), Some(IllusionCategory::Hands));
        assert_eq!(of("ArmorProtective_Legs"), Some(IllusionCategory::Legs));
        assert_eq!(of("ArmorProtective_Feet"), Some(IllusionCategory::Feet));
        assert_eq!(of("ArmorJewelry_Medal"), Some(IllusionCategory::Medal));
        assert_eq!(of("WeaponArmor_Shield"), Some(IllusionCategory::OffHand));
        assert_eq!(of("WeaponArmor_Offhand"), Some(IllusionCategory::OffHand));
        for weapon in [
            "WeaponMelee_Axe",
            "WeaponMelee_Sword2h",
            "WeaponHunting_Ranged1h",
            "WeaponHunting_Spear",
            "WeaponMagical_Staff",
        ] {
            assert_eq!(of(weapon), Some(IllusionCategory::Weapon), "{weapon}");
        }
        for none in [
            "ArmorProtective_Waist",
            "ArmorJewelry_Ring",
            "ArmorJewelry_Amulet",
            "ItemRelic",
            "ItemArtifactFormula",
        ] {
            assert_eq!(of(none), None, "{none}");
        }
    }

    #[test]
    fn add_files_a_record_under_its_category_creating_the_list_if_needed() {
        let game = game();
        let mut illusions = collection(&[(1, &[HELM])]);
        assert_eq!(
            add_illusion(&mut illusions, &game, id(SWORD)).unwrap(),
            IllusionCategory::Weapon
        );
        assert_eq!(
            add_illusion(&mut illusions, &game, id(HELM2)).unwrap(),
            IllusionCategory::Head
        );
        assert_eq!(illusions.slots.len(), 2);
        assert_eq!(illusions.slots[0].records, [HELM, HELM2]);
        assert_eq!(illusions.slots[1].slot, 9);
        assert_eq!(illusions.slots[1].records, [SWORD]);
        assert!(matches!(
            add_illusion(&mut illusions, &game, id(HELM)),
            Err(IllusionError::AlreadyKnown(_))
        ));
        assert!(matches!(
            add_illusion(&mut illusions, &game, id(BELT)),
            Err(IllusionError::NotAnIllusion { .. })
        ));
        assert!(matches!(
            add_illusion(&mut illusions, &game, id("records/items/nothing.dbr")),
            Err(IllusionError::UnknownRecord(_))
        ));
        assert_eq!(
            records_of(&illusions, IllusionCategory::Head),
            [HELM, HELM2]
        );
        assert!(records_of(&illusions, IllusionCategory::Medal).is_empty());
        assert_eq!(
            available_illusions(&game),
            vec![id(HELM), id(HELM2), id(SHIELD), id(SWORD)]
        );
    }

    #[test]
    fn audit_reports_what_the_database_disagrees_with() {
        let game = game();
        let clean = collection(&[(1, &[HELM]), (8, &[SHIELD]), (9, &[SWORD])]);
        assert!(audit(&clean, &game).is_empty());
        let dirty = collection(&[
            (1, &[HELM, SWORD]),
            (9, &[BELT, "records/items/nothing.dbr"]),
        ]);
        let problems = audit(&dirty, &game);
        assert_eq!(problems.len(), 3);
        assert!(matches!(
            problems[0],
            (ref record, IllusionError::Misfiled { listed: 1, category: IllusionCategory::Weapon, .. })
                if record == SWORD
        ));
        assert!(matches!(
            problems[1],
            (_, IllusionError::NotAnIllusion { .. })
        ));
        assert!(matches!(problems[2], (_, IllusionError::UnknownRecord(_))));
    }

    #[test]
    fn an_export_round_trips_and_import_files_by_the_documents_slot() {
        let game = game();
        let source = collection(&[(1, &[HELM, HELM2]), (8, &[SHIELD])]);
        let export = IllusionExport::of(&source, Campaign::Main, Timestamp::from_unix_seconds(3));
        let bytes = export.to_json();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["format"], json!(EXPORT_FORMAT));
        assert_eq!(value["campaign"], json!("main"));
        assert_eq!(
            value["slots"][0],
            json!({ "slot": 1, "records": [HELM, HELM2] })
        );
        let parsed = IllusionExport::from_json(&bytes).unwrap();
        assert_eq!(parsed, export);
        assert_eq!(parsed.total_count(), 3);

        let mut target = collection(&[(1, &[HELM])]);
        let report = import_illusions(&mut target, &game, &parsed);
        assert_eq!((report.added, report.already_known), (2, 1));
        assert!(report.refused.is_empty());
        assert_eq!(target.slots[0].records, [HELM, HELM2]);
        assert_eq!(target.slots[1].slot, 8);

        let misfiled = IllusionExport {
            campaign: Campaign::Main,
            exported_at: Timestamp::from_unix_seconds(0),
            slots: vec![ExportedSlot {
                slot: 1,
                records: vec![SWORD.into(), BELT.into()],
            }],
            extra: Map::new(),
        };
        let report = import_illusions(&mut target, &game, &misfiled);
        assert_eq!(report.added, 0);
        assert!(matches!(
            report.refused[0],
            (_, IllusionError::Misfiled { listed: 1, .. })
        ));
        assert!(matches!(
            report.refused[1],
            (_, IllusionError::NotAnIllusion { .. })
        ));
        assert!(matches!(
            IllusionExport::from_json(br#"{"format":"grimvault-blueprints","version":1}"#),
            Err(EnvelopeError::WrongFormat { .. })
        ));
    }
}
