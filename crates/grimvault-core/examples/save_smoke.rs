//! Smoke-tests the save codec against a **copy** of a real save
//! directory (never the live one):
//!
//! ```text
//! cargo run -p grimvault-core --example save_smoke -- /path/to/copy/of/save
//! ```
//!
//! For every `main/_*/player.gdc` and `user/_*/player.gdc`: character, inventory, equipment and
//! stash summary, the block ids seen (a `~` suffix marks an opaque
//! block; every block the game writes should be typed), whether an
//! unmodified re-encode is byte-identical, and whether an edited model
//! survives encode → parse — one sack item removed, one added, one
//! moved to the first stash tab — which re-keys every block after the
//! inventory. Then the same read-only checks for `transfer.gst`,
//! `formulas.gst`, `transmutes.gst` and `reagents.gst`. Exits non-zero
//! if anything failed to parse, round-trip, or survive an edit.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use grimvault_core::block::{OpaqueBlock, OpaqueReason, SaveEncodeError, StashTab};
use grimvault_core::gdc::{Block, PlayerFile, Realm};
use grimvault_core::gst::{GstBlock, GstError, GstFile};
use grimvault_core::item::StashItem;

const GST_FILES: [&str; 4] = [
    "transfer.gst",
    "formulas.gst",
    "transmutes.gst",
    "reagents.gst",
];

fn main() -> ExitCode {
    let Some(save_dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: save_smoke <save-dir>");
        return ExitCode::FAILURE;
    };
    let player_outcomes = player_files(&save_dir)
        .iter()
        .map(|path| smoke_player(path))
        .collect::<Vec<_>>();
    let gst_outcomes = GST_FILES
        .iter()
        .map(|name| save_dir.join(name))
        .filter(|path| path.is_file())
        .map(|path| smoke_gst(&path))
        .collect::<Vec<_>>();
    if player_outcomes.iter().chain(&gst_outcomes).all(|&ok| ok) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn player_files(save_dir: &Path) -> Vec<PathBuf> {
    Realm::ALL
        .into_iter()
        .flat_map(|realm| player_files_under(&save_dir.join(realm.dir_name())))
        .collect()
}

fn player_files_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('_'))
        })
        .map(|dir| dir.join("player.gdc"))
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
}

fn smoke_player(path: &Path) -> bool {
    println!("== {}", path.display());
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            println!("   read error: {error}");
            return false;
        }
    };
    let file = match PlayerFile::parse(&bytes) {
        Ok(file) => file,
        Err(error) => {
            println!("   parse error: {error}");
            return false;
        }
    };
    let header = file.header();
    let name = file.character_name();
    let level = file.level();
    println!(
        "   character {name:?}  level {level}  class {:?}  {:?}  hardcore {}  expansion_status {}",
        header.class_tag, header.sex, header.hardcore, header.expansion_status
    );
    match file.inventory() {
        Some(inventory) => {
            let names: Vec<&str> = inventory
                .sacks()
                .iter()
                .flat_map(|sack| &sack.items)
                .map(|entry| entry.item.base_name.as_str())
                .collect();
            let version = inventory.version;
            let sacks = inventory.sacks().len();
            let items = names.len();
            let first: Vec<&str> = names.iter().copied().take(3).collect();
            println!("   inventory {version}: {sacks} sacks, {items} items, first {first:?}");
            println!("   equipped: {}", inventory.equipped().count());
        }
        None => println!("   inventory: not typed"),
    }
    match file.stash() {
        Some(stash) => {
            let version = stash.version;
            let tabs = stash.tabs.len();
            let per_tab = per_tab_counts(&stash.tabs);
            println!("   stash {version}: {tabs} tabs, items per tab {per_tab}");
        }
        None => println!("   stash: not typed"),
    }
    if let (Some(bio), Some(stats)) = (file.bio(), file.stats()) {
        println!(
            "   bio: experience {}  physique {}  cunning {}  spirit {}   stats {:?}: playtime {}s  deaths {}  kills {}",
            bio.experience,
            bio.physique,
            bio.cunning,
            bio.spirit,
            stats.version(),
            stats.playtime,
            stats.deaths,
            stats.kills
        );
    }
    let blocks: Vec<String> = file
        .blocks()
        .iter()
        .map(|block| match block {
            Block::Opaque(opaque) => describe_opaque(opaque),
            typed => typed.id().raw().to_string(),
        })
        .collect();
    let opaque = file.blocks().iter().filter(|b| b.is_opaque()).count();
    let typed = file.blocks().len() - opaque;
    println!(
        "   blocks: {}   ({typed} typed, {opaque} opaque)",
        blocks.join(" ")
    );
    let round_trip = report_round_trip(&bytes, file.encode());
    let edits = report_edits(&file);
    round_trip && edits
}

/// An edit that re-keys every block after the inventory; `false` when
/// the file has nothing to edit.
type Edit = fn(&mut PlayerFile) -> bool;

const EDITS: [(&str, Edit); 3] = [
    ("remove one sack item", remove_sack_item),
    ("add one sack item", add_sack_item),
    ("move one sack item to stash tab 0", move_sack_item_to_stash),
];

/// Applies each edit to a copy of `file`, encodes it, parses the bytes
/// back, and requires the parse to equal the edited model — the proof
/// that every block after the inventory survived the key change.
fn report_edits(file: &PlayerFile) -> bool {
    EDITS.iter().all(|(label, edit)| {
        let mut edited = file.clone();
        if !edit(&mut edited) {
            println!("   edit `{label}`: not applicable (no sack item or stash tab)");
            return true;
        }
        let bytes = match edited.encode() {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("   edit `{label}`: FAIL: encode: {error}");
                return false;
            }
        };
        match PlayerFile::parse(&bytes) {
            Ok(parsed) if parsed == edited => {
                println!(
                    "   edit `{label}`: encode → parse equals the edited model ({} bytes)",
                    bytes.len()
                );
                true
            }
            Ok(_) => {
                println!("   edit `{label}`: FAIL: re-parsed model differs from the edited model");
                false
            }
            Err(error) => {
                println!("   edit `{label}`: FAIL: re-parse: {error}");
                false
            }
        }
    })
}

fn remove_sack_item(file: &mut PlayerFile) -> bool {
    let Some(inventory) = file.inventory_mut() else {
        return false;
    };
    inventory
        .sacks_mut()
        .iter_mut()
        .find(|sack| !sack.items.is_empty())
        .map(|sack| sack.items.remove(0))
        .is_some()
}

fn add_sack_item(file: &mut PlayerFile) -> bool {
    let Some(inventory) = file.inventory_mut() else {
        return false;
    };
    let Some(sack) = inventory
        .sacks_mut()
        .iter_mut()
        .find(|sack| !sack.items.is_empty())
    else {
        return false;
    };
    let mut copy = sack.items[0].clone();
    copy.x += 1;
    sack.items.push(copy);
    true
}

fn move_sack_item_to_stash(file: &mut PlayerFile) -> bool {
    let Some(taken) = file.inventory_mut().and_then(|inventory| {
        inventory
            .sacks_mut()
            .iter_mut()
            .find(|sack| !sack.items.is_empty())
            .map(|sack| sack.items.remove(0))
    }) else {
        return false;
    };
    let Some(tab) = file.stash_mut().and_then(|stash| stash.tabs.first_mut()) else {
        return false;
    };
    tab.items.push(StashItem {
        item: taken.item,
        x: 0.0,
        y: 0.0,
    });
    true
}

fn smoke_gst(path: &Path) -> bool {
    println!("== {}", path.display());
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            println!("   read error: {error}");
            return false;
        }
    };
    let file = match GstFile::parse(&bytes) {
        Ok(file) => file,
        Err(GstError::PlaintextKeyValueFormat) => {
            println!(
                "   not obfuscated: plaintext begin_block/end_block key-value format; skipped"
            );
            return true;
        }
        Err(error) => {
            println!("   parse error: {error}");
            return false;
        }
    };
    let blocks: Vec<String> = file
        .blocks()
        .iter()
        .map(|block| match block {
            GstBlock::TransferStash(_) | GstBlock::Illusions(_) | GstBlock::ReagentStorage(_) => {
                block.id().raw().to_string()
            }
            GstBlock::Opaque(opaque) => describe_opaque(opaque),
        })
        .collect();
    let file_version = file.file_version();
    println!(
        "   file_version {file_version}  blocks: {}",
        blocks.join(" ")
    );
    if let Some(stash) = file.transfer_stash() {
        let version = stash.version;
        let tabs = stash.tabs.len();
        let per_tab = per_tab_counts(&stash.tabs);
        println!(
            "   transfer stash {version}: mod {:?}  expansion_status {}  {tabs} tabs, items per tab {per_tab}",
            stash.mod_name, stash.expansion_status
        );
    }
    if let Some(storage) = file.reagent_storage() {
        println!(
            "   reagent storage {}: mod {:?}  {} entries, {} items",
            storage.version,
            storage.mod_name,
            storage.entries.len(),
            storage.total_count()
        );
    }
    if let Some(illusions) = file.illusions() {
        println!(
            "   illusions {}: mod {:?}  expansion_status {}  {} slots, {} unlocked",
            illusions.version,
            illusions.mod_name,
            illusions.expansion_status,
            illusions.slots.len(),
            illusions.total_count()
        );
    }
    report_round_trip(&bytes, file.encode())
}

fn per_tab_counts(tabs: &[StashTab]) -> String {
    tabs.iter()
        .map(|tab| tab.items.len().to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn describe_opaque(opaque: &OpaqueBlock) -> String {
    let id = opaque.id().raw();
    let reason = match opaque.reason() {
        OpaqueReason::Unmodeled => String::new(),
        OpaqueReason::UnsupportedVersion { version } => format!("v{version}"),
    };
    let nested = opaque.nested_ids().count();
    if nested == 0 {
        format!("{id}~{reason}")
    } else {
        format!("{id}~{reason}[{nested} nested]")
    }
}

fn report_round_trip(original: &[u8], encoded: Result<Vec<u8>, SaveEncodeError>) -> bool {
    match encoded {
        Ok(bytes) if bytes == original => {
            println!("   round-trip OK ({} bytes)", original.len());
            true
        }
        Ok(bytes) => {
            let first_difference = bytes
                .iter()
                .zip(original)
                .position(|(a, b)| a != b)
                .unwrap_or(bytes.len().min(original.len()));
            println!(
                "   round-trip FAIL: first difference at offset {first_difference}, encoded {} bytes vs {} original",
                bytes.len(),
                original.len()
            );
            false
        }
        Err(error) => {
            println!("   round-trip FAIL: {error}");
            false
        }
    }
}
