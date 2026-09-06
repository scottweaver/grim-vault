//! The user's own GD Stash exports as the acceptance data for
//! `grimvault_core::gds`: the first-entry facts recorded in
//! `docs/format-references.md`, a full import into a fresh store, a
//! re-import that adds nothing, and the store's JSON round trip with
//! thousands of export origins.
//!
//! Reads `gd-stash-export.gds` and `reagent-export.gds` under
//! `$GRIMVAULT_GDS_DIR` (copies, never the save directory itself) and
//! passes vacuously when the variable is unset or a file is absent.

use std::path::{Path, PathBuf};

use grimvault_core::gds::{self, GameMode, GdsEntry, KnownRecords};
use grimvault_core::item::Item;
use grimvault_core::store::{ItemOrigin, Timestamp, VaultStore};

const NOW: Timestamp = Timestamp::from_unix_seconds(1_757_000_000);

/// No database in a unit test: every record counts as unknown, which
/// exercises the report without changing what is imported.
struct NoDatabase;

impl KnownRecords for NoDatabase {
    fn has_base_record(&self, _: &Item) -> bool {
        false
    }
}

fn export_path(name: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("GRIMVAULT_GDS_DIR").map(PathBuf::from)?;
    let path = dir.join(name);
    path.is_file().then_some(path)
}

fn first_entry_facts(path: &Path) -> GdsEntry {
    let bytes = std::fs::read(path).unwrap();
    let export = gds::parse(&bytes).unwrap();
    assert_eq!(export.version().raw(), 3, "{}", path.display());
    assert!(!export.is_empty(), "{}", path.display());
    export.entries()[0].clone()
}

#[test]
fn the_collection_export_starts_with_zarks_forty_augments() {
    let Some(path) = export_path("gd-stash-export.gds") else {
        return;
    };
    let first = first_entry_facts(&path);
    assert_eq!(
        first.item.base_name,
        "records/items/enchants/a07a_enchant.dbr"
    );
    assert_eq!(first.item.seed, 0x1c4c_23f4);
    assert_eq!(first.item.stack_count, 40);
    assert_eq!(first.mode, GameMode::Softcore);
    assert_eq!(first.owner.as_deref(), Some("Zark"));
}

#[test]
fn the_reagent_export_starts_with_the_same_augment_as_a_smaller_stack() {
    let Some(path) = export_path("reagent-export.gds") else {
        return;
    };
    let first = first_entry_facts(&path);
    assert_eq!(
        first.item.base_name,
        "records/items/enchants/a07a_enchant.dbr"
    );
    assert_eq!(first.item.seed, 0x1c4c_23f4);
    assert_eq!(first.item.stack_count, 5);
    assert_eq!(first.owner.as_deref(), Some("Zark"));
}

#[test]
fn every_entry_imports_once_and_a_re_import_adds_nothing() {
    for name in ["gd-stash-export.gds", "reagent-export.gds"] {
        let Some(path) = export_path(name) else {
            continue;
        };
        let export = gds::parse(&std::fs::read(&path).unwrap()).unwrap();
        let mut store = VaultStore::new();
        let first = gds::import(&mut store, &export, &path, &NoDatabase, NOW);
        assert_eq!(
            first.added.len(),
            export.len(),
            "{name}: every entry is distinct"
        );
        assert_eq!(first.duplicates, 0, "{name}");
        assert_eq!(store.len(), export.len(), "{name}");
        assert!(store.items().iter().all(|stored| matches!(
            stored.origin(),
            ItemOrigin::GdStashExport { file, .. } if file == name
        )));

        let again = gds::import(&mut store, &export, &path, &NoDatabase, NOW);
        assert!(again.added.is_empty(), "{name}: re-import added items");
        assert_eq!(again.duplicates, export.len(), "{name}");
        assert_eq!(store.len(), export.len(), "{name}");

        let reloaded = VaultStore::from_json(&store.to_json()).unwrap();
        assert_eq!(reloaded, store, "{name}: JSON round trip");
    }
}
