//! The crafting files as the shell holds them — a campaign's
//! blueprint list (`formulas.gst`, the one plaintext file, so its own
//! document type) and illusion collection (`transmutes.gst`, a
//! [`GstDoc`]) — and the actions the crafting pane asks for: add one
//! record, export the list as this app's JSON document, import one.
//! Every write still goes through the document machinery (autosave,
//! backup-first, the external-change guard); this module only edits
//! the model and says which document changed.

use std::io;
use std::path::{Path, PathBuf};

use grimvault_core::blueprint::{BlueprintExport, add_blueprint, import_blueprints};
use grimvault_core::campaign::Campaign;
use grimvault_core::crypto::EncodeError;
use grimvault_core::formulas::{Formulas, FormulasError};
use grimvault_core::gamedata::GameData;
use grimvault_core::illusion::{IllusionExport, add_illusion, import_illusions};
use grimvault_core::interchange::EnvelopeError;
use grimvault_core::loaded::{LoadError, Loaded};
use grimvault_core::store::Timestamp;
use thiserror::Error;
use univault_engine::ids::RecordId;
use univault_io::{read_verified, write_synced};

use crate::app::Toasts;
use crate::documents::{Doc, Document, IllusionsDoc, Optional, SaveError, SaveOutcome, Tracking};

/// Why a `formulas.gst` could not be opened for editing.
#[derive(Debug, Error)]
pub enum FormulasOpenError {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Load {
        path: PathBuf,
        source: LoadError<FormulasError, EncodeError>,
    },
}

/// A `formulas.gst`, gated lossless and stamped.
#[derive(Debug)]
pub struct FormulasDoc {
    tracking: Tracking,
    loaded: Loaded<Formulas>,
}

impl FormulasDoc {
    #[must_use]
    pub fn path(&self) -> &Path {
        self.tracking.path()
    }

    #[must_use]
    pub fn formulas(&self) -> &Formulas {
        self.loaded.model()
    }

    /// The list for editing; call [`Tracking::mark_edited`] after.
    pub fn formulas_mut(&mut self) -> &mut Formulas {
        self.loaded.model_mut()
    }

    /// Size of the bytes the model was proven against.
    #[must_use]
    pub fn baseline_len(&self) -> usize {
        self.loaded.baseline().len()
    }
}

impl Document for FormulasDoc {
    type OpenError = FormulasOpenError;

    fn open(path: PathBuf) -> Result<Self, FormulasOpenError> {
        let bytes = read_verified(&path).map_err(|source| FormulasOpenError::Read {
            path: path.clone(),
            source,
        })?;
        let tracking = Tracking::fresh(path.clone());
        let loaded = Loaded::<Formulas>::load(bytes)
            .map_err(|source| FormulasOpenError::Load { path, source })?;
        Ok(Self { tracking, loaded })
    }

    fn tracking(&self) -> &Tracking {
        &self.tracking
    }

    fn tracking_mut(&mut self) -> &mut Tracking {
        &mut self.tracking
    }

    fn save(&mut self) -> Result<SaveOutcome, SaveError> {
        let bytes = self
            .loaded
            .encode()
            .map_err(grimvault_core::block::SaveEncodeError::from)?;
        self.tracking.save_bytes(&bytes)
    }

    fn reload(&mut self) -> Result<(), FormulasOpenError> {
        *self = Self::open(self.tracking.path().to_path_buf())?;
        Ok(())
    }
}

/// A campaign's blueprint list, however it fared.
pub type Blueprints = Optional<FormulasDoc>;
/// A campaign's illusion collection, however it fared.
pub type IllusionCollection = Optional<IllusionsDoc>;

/// Which crafting list an action is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Crafting {
    Blueprints,
    Illusions,
}

impl Crafting {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blueprints => "Blueprints",
            Self::Illusions => "Illusions",
        }
    }

    /// The singular, for toasts.
    #[must_use]
    pub const fn one(self) -> &'static str {
        match self {
            Self::Blueprints => "blueprint",
            Self::Illusions => "illusion",
        }
    }

    #[must_use]
    pub const fn doc(self) -> Doc {
        match self {
            Self::Blueprints => Doc::Blueprints,
            Self::Illusions => Doc::Illusions,
        }
    }

    /// The name an export dialog proposes: the list and the campaign.
    #[must_use]
    pub fn export_file_name(self, campaign: &Campaign) -> String {
        let campaign = match campaign {
            Campaign::Main => "main".to_string(),
            Campaign::Mod(name) => name.as_str().to_lowercase(),
        };
        format!("{}-{campaign}.json", self.one())
    }
}

/// What the crafting pane asked for this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Add { list: Crafting, record: RecordId },
    Export(Crafting),
    Import(Crafting),
}

/// The two files of the campaign in view.
pub struct CraftingFiles<'a> {
    pub blueprints: &'a mut Blueprints,
    pub illusions: &'a mut IllusionCollection,
}

/// Where a document that is absent or unusable leaves an action.
fn not_editable<D: Document>(list: Crafting, file: &Optional<D>, toasts: &mut Toasts) {
    match file {
        Optional::Open(_) => {}
        Optional::Absent { path } => toasts.error(format!(
            "{} does not exist yet: the game writes it once the first {} is learned in this campaign",
            path.display(),
            list.one()
        )),
        Optional::Failed { path, error, .. } => {
            toasts.error(format!("{} cannot be edited: {error}", path.display()));
        }
    }
}

impl CraftingFiles<'_> {
    /// Reports why `list`'s file cannot be edited.
    fn not_editable(&self, list: Crafting, toasts: &mut Toasts) {
        match list {
            Crafting::Blueprints => not_editable(list, self.blueprints, toasts),
            Crafting::Illusions => not_editable(list, self.illusions, toasts),
        }
    }
}

/// Performs a request against the campaign's files; `Some(doc)` when
/// a document's model changed and needs saving.
pub fn perform(
    request: Request,
    files: &mut CraftingFiles<'_>,
    game: &GameData,
    campaign: &Campaign,
    now: Timestamp,
    toasts: &mut Toasts,
) -> Option<Doc> {
    match request {
        Request::Add { list, record } => add(list, &record, files, game, toasts),
        Request::Export(list) => {
            export(list, files, campaign, now, toasts);
            None
        }
        Request::Import(list) => import(list, files, game, campaign, toasts),
    }
}

fn add(
    list: Crafting,
    record: &RecordId,
    files: &mut CraftingFiles<'_>,
    game: &GameData,
    toasts: &mut Toasts,
) -> Option<Doc> {
    let added = match list {
        Crafting::Blueprints => files.blueprints.doc_mut().map(|doc| {
            add_blueprint(doc.formulas_mut(), game, record.clone())
                .map(|()| (doc.tracking_mut(), "added blueprint".to_string()))
                .map_err(|error| error.to_string())
        }),
        Crafting::Illusions => files.illusions.doc_mut().map(|doc| {
            add_illusion(doc.illusions_mut(), game, record.clone())
                .map(|category| {
                    (
                        doc.tracking_mut(),
                        format!("added {} illusion", category.label().to_lowercase()),
                    )
                })
                .map_err(|error| error.to_string())
        }),
    };
    match added {
        Some(Ok((tracking, verb))) => {
            tracking.mark_edited();
            toasts.info(format!("{verb} {}", name_of(game, record)));
            Some(list.doc())
        }
        Some(Err(error)) => {
            toasts.error(error);
            None
        }
        None => {
            files.not_editable(list, toasts);
            None
        }
    }
}

/// The document a list exports as, `None` (reported) when its file
/// is not open.
fn export_document(
    list: Crafting,
    files: &CraftingFiles<'_>,
    campaign: &Campaign,
    now: Timestamp,
) -> Option<Vec<u8>> {
    match list {
        Crafting::Blueprints => files
            .blueprints
            .doc()
            .map(|doc| BlueprintExport::of(doc.formulas(), campaign.clone(), now).to_json()),
        Crafting::Illusions => files
            .illusions
            .doc()
            .map(|doc| IllusionExport::of(doc.illusions(), campaign.clone(), now).to_json()),
    }
}

fn export(
    list: Crafting,
    files: &CraftingFiles<'_>,
    campaign: &Campaign,
    now: Timestamp,
    toasts: &mut Toasts,
) {
    let Some(bytes) = export_document(list, files, campaign, now) else {
        files.not_editable(list, toasts);
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .set_file_name(list.export_file_name(campaign))
        .save_file()
    else {
        return;
    };
    match write_synced(&path, &bytes) {
        Ok(()) => toasts.info(format!(
            "exported the {} {} to {}",
            campaign,
            list.label().to_lowercase(),
            path.display()
        )),
        Err(error) => toasts.error(format!("could not write {}: {error}", path.display())),
    }
}

fn import(
    list: Crafting,
    files: &mut CraftingFiles<'_>,
    game: &GameData,
    campaign: &Campaign,
    toasts: &mut Toasts,
) -> Option<Doc> {
    let path = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .pick_file()?;
    let bytes = match read_verified(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            toasts.error(format!("could not read {}: {error}", path.display()));
            return None;
        }
    };
    let outcome = match list {
        Crafting::Blueprints => import_blueprint_document(files.blueprints, game, &bytes),
        Crafting::Illusions => import_illusion_document(files.illusions, game, &bytes),
    };
    match outcome {
        Ok(Imported::Changed(summary)) => {
            toasts.info(format!("imported into the {campaign}: {summary}"));
            Some(list.doc())
        }
        Ok(Imported::Unchanged(summary)) => {
            toasts.info(format!("nothing to add: {summary}"));
            None
        }
        Ok(Imported::NotEditable) => {
            files.not_editable(list, toasts);
            None
        }
        Err(error) => {
            toasts.error(format!("{}: {error}", path.display()));
            None
        }
    }
}

/// How an import concluded; the summaries are the report's one-line
/// form with the refusals.
enum Imported {
    Changed(String),
    Unchanged(String),
    NotEditable,
}

fn import_blueprint_document(
    blueprints: &mut Blueprints,
    game: &GameData,
    bytes: &[u8],
) -> Result<Imported, EnvelopeError> {
    let export = BlueprintExport::from_json(bytes)?;
    let Some(doc) = blueprints.doc_mut() else {
        return Ok(Imported::NotEditable);
    };
    let report = import_blueprints(doc.formulas_mut(), game, &export);
    let summary = format!(
        "{} blueprints from the {}: {report}",
        export.entries.len(),
        export.campaign
    );
    Ok(if report.changed() {
        doc.tracking_mut().mark_edited();
        Imported::Changed(summary)
    } else {
        Imported::Unchanged(summary)
    })
}

fn import_illusion_document(
    illusions: &mut IllusionCollection,
    game: &GameData,
    bytes: &[u8],
) -> Result<Imported, EnvelopeError> {
    let export = IllusionExport::from_json(bytes)?;
    let Some(doc) = illusions.doc_mut() else {
        return Ok(Imported::NotEditable);
    };
    let report = import_illusions(doc.illusions_mut(), game, &export);
    let summary = format!(
        "{} illusions from the {}: {report}",
        export.total_count(),
        export.campaign
    );
    Ok(if report.changed() {
        doc.tracking_mut().mark_edited();
        Imported::Changed(summary)
    } else {
        Imported::Unchanged(summary)
    })
}

fn name_of(game: &GameData, record: &RecordId) -> String {
    game.item_info(record)
        .and_then(Result::ok)
        .map_or_else(|| record.file_stem().to_string(), |info| info.name)
}

#[cfg(test)]
mod tests {
    use grimvault_core::campaign::ModName;
    use grimvault_core::formulas::{BlueprintEntry, FormulaRead};

    use super::*;
    use crate::documents::{Backup, Edits};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("grimvault-crafting-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn formulas_bytes() -> Vec<u8> {
        Formulas {
            version: grimvault_core::formulas::FormulasVersion::new(3).unwrap(),
            expansion_status: 7,
            entries: vec![BlueprintEntry {
                record: "records/items/crafting/blueprints/armor/craft_feet_squiresboots02.dbr"
                    .into(),
                read: FormulaRead::Read,
            }],
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn a_blueprint_file_edits_saves_backup_first_and_reloads() {
        let scratch = Scratch::new("formulas");
        let path = scratch.0.join("formulas.gst");
        let bytes = formulas_bytes();
        std::fs::write(&path, &bytes).unwrap();

        let mut blueprints = Blueprints::open(path.clone());
        let doc = blueprints.doc_mut().expect("a lossless file opens");
        assert_eq!(doc.baseline_len(), bytes.len());
        assert_eq!(doc.tracking().backup(), Backup::Armed);
        doc.formulas_mut().entries.push(BlueprintEntry {
            record: "records/items/crafting/blueprints/relics/craft_relic_sealnight.dbr".into(),
            read: FormulaRead::Unread,
        });
        doc.tracking_mut().mark_edited();
        let outcome = doc.save().unwrap();
        assert!(
            matches!(outcome, SaveOutcome::Saved { backup: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(doc.tracking().edits(), Edits::Saved);
        let written = Formulas::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written.entries.len(), 2);

        std::fs::write(&path, &bytes).unwrap();
        blueprints.reload().unwrap();
        let reloaded = blueprints.doc().unwrap();
        assert_eq!(reloaded.formulas().entries.len(), 1);
        assert_eq!(reloaded.tracking().backup(), Backup::Armed);
    }

    #[test]
    fn an_absent_or_foreign_file_is_reported_not_fatal() {
        let scratch = Scratch::new("formulas-absent");
        let path = scratch.0.join("formulas.gst");
        let absent = Blueprints::open(path.clone());
        assert!(matches!(absent, Optional::Absent { .. }));
        assert!(absent.doc().is_none());

        std::fs::write(&path, b"\x0b\x00\x00\x00begin_block\xff").unwrap();
        let failed = Blueprints::open(path.clone());
        assert!(matches!(
            failed,
            Optional::Failed {
                error: FormulasOpenError::Load { .. },
                ..
            }
        ));
        assert_eq!(failed.edits(), Edits::Saved);
    }

    #[test]
    fn export_file_names_carry_the_list_and_the_campaign() {
        assert_eq!(
            Crafting::Blueprints.export_file_name(&Campaign::Main),
            "blueprint-main.json"
        );
        let loot = Campaign::Mod(ModName::parse("LootAscension").unwrap());
        assert_eq!(
            Crafting::Illusions.export_file_name(&loot),
            "illusion-lootascension.json"
        );
        assert_eq!(Crafting::Blueprints.doc(), Doc::Blueprints);
        assert_eq!(Crafting::Illusions.doc(), Doc::Illusions);
    }
}
