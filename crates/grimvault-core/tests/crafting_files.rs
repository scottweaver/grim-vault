//! Lossless-model gate for the crafting files: every `formulas.*` and
//! `transmutes.*` under `$GRIMVAULT_SAVE_DIR` (a **copy** of a save
//! directory, never the live one) and under each mod folder there
//! must parse typed and re-encode byte-for-byte, and an add must
//! survive encode → parse with the model intact. Passes vacuously
//! when the variable is unset.

use std::path::{Path, PathBuf};

use grimvault_core::formulas::{BlueprintEntry, FormulaRead, Formulas};
use grimvault_core::gst::{Added, GstFile};
use grimvault_core::loaded::Loaded;

fn save_dir() -> Option<PathBuf> {
    std::env::var_os("GRIMVAULT_SAVE_DIR").map(PathBuf::from)
}

/// The save root and every folder beside `main/` holding a
/// `transfer.gst` — the campaign folders.
fn campaign_dirs(save_dir: &Path) -> Vec<PathBuf> {
    let mut mods: Vec<PathBuf> = std::fs::read_dir(save_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.join("transfer.gst").is_file())
                .collect()
        })
        .unwrap_or_default();
    mods.sort();
    std::iter::once(save_dir.to_path_buf())
        .chain(mods)
        .collect()
}

/// Every file named `stem.<ext>` under `dir` for the stash-family
/// suffixes (`.gst` softcore, `.gsh` hardcore, and the per-expansion
/// twins).
fn family(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_stem().is_some_and(|found| found == stem)
                        && path.extension().is_some_and(|ext| {
                            ext.len() == 3
                                && ext
                                    .to_str()
                                    .is_some_and(|ext| ext.ends_with("st") || ext.ends_with("sh"))
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

#[test]
fn real_blueprint_files_round_trip_and_survive_an_add() {
    let Some(save_dir) = save_dir() else {
        return;
    };
    let mut seen = 0;
    for dir in campaign_dirs(&save_dir) {
        for path in family(&dir, "formulas") {
            let label = path.display().to_string();
            let bytes = std::fs::read(&path).unwrap();
            let loaded = Loaded::<Formulas>::load(bytes.clone())
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            assert_eq!(loaded.baseline(), &bytes[..], "{label}");
            let formulas = loaded.model();
            assert_eq!(formulas.version.raw(), 3, "{label}");
            assert!(!formulas.entries.is_empty(), "{label}: no blueprints");
            assert!(
                formulas
                    .entries
                    .iter()
                    .all(|entry| entry.record.starts_with("records/")),
                "{label}: an entry is not a record path"
            );

            let mut edited = formulas.clone();
            assert_eq!(
                edited.add(BlueprintEntry {
                    record: "records/items/crafting/blueprints/test_only/never_shipped.dbr".into(),
                    read: FormulaRead::Unread,
                }),
                Added::Added
            );
            let written = edited.encode().unwrap();
            assert_ne!(written, bytes, "{label}: the add changed nothing");
            assert_eq!(Formulas::parse(&written).unwrap(), edited, "{label}");
            seen += 1;
        }
    }
    assert!(seen > 0, "no formulas.* under {}", save_dir.display());
}

#[test]
fn real_illusion_files_round_trip_and_survive_an_add() {
    let Some(save_dir) = save_dir() else {
        return;
    };
    let mut seen = 0;
    for dir in campaign_dirs(&save_dir) {
        for path in family(&dir, "transmutes") {
            let label = path.display().to_string();
            let bytes = std::fs::read(&path).unwrap();
            let loaded = Loaded::<GstFile>::load(bytes.clone())
                .unwrap_or_else(|error| panic!("{label}: {error}"));
            assert_eq!(loaded.baseline(), &bytes[..], "{label}");
            let illusions = loaded
                .model()
                .illusions()
                .unwrap_or_else(|| panic!("{label}: block 19 not typed"));
            assert!(!illusions.slots.is_empty(), "{label}: no slots");
            let ids: Vec<u32> = illusions.slots.iter().map(|slot| slot.slot).collect();
            assert!(
                ids.iter()
                    .all(|id| [1, 3, 4, 5, 7, 8, 9, 14, 15].contains(id)),
                "{label}: unexpected slot id in {ids:?}"
            );

            let mut edited = loaded.model().clone();
            let collection = edited.illusions_mut().unwrap();
            assert_eq!(
                collection.add(
                    1,
                    "records/items/gearhead/test_only/never_shipped.dbr".into()
                ),
                Added::Added
            );
            let written = edited.encode().unwrap();
            assert_ne!(written, bytes, "{label}: the add changed nothing");
            assert_eq!(GstFile::parse(&written).unwrap(), edited, "{label}");
            seen += 1;
        }
    }
    assert!(seen > 0, "no transmutes.* under {}", save_dir.display());
}
