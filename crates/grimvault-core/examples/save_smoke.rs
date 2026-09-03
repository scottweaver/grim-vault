//! Smoke-tests the save codec against a **copy** of a real save
//! directory (never the live one):
//!
//! ```text
//! cargo run -p grimvault-core --example save_smoke -- /path/to/copy/of/save
//! ```
//!
//! For every `main/_*/player.gdc`: character, inventory, equipment and
//! stash summary, the block ids seen (a `~` suffix marks an opaque
//! block), and whether an unmodified re-encode is byte-identical. Then
//! the same for `transfer.gst`, `formulas.gst`, `transmutes.gst` and
//! `reagents.gst`. Exits non-zero if anything failed to parse or
//! round-trip.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use grimvault_core::block::{OpaqueBlock, OpaqueReason, SaveEncodeError, StashTab};
use grimvault_core::gdc::{Block, PlayerFile};
use grimvault_core::gst::{GstBlock, GstError, GstFile};

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
    let Ok(entries) = std::fs::read_dir(save_dir.join("main")) else {
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
    let blocks: Vec<String> = file
        .blocks()
        .iter()
        .map(|block| match block {
            Block::CharacterInfo(_) | Block::Inventory(_) | Block::Stash(_) => {
                block.id().raw().to_string()
            }
            Block::Opaque(opaque) => describe_opaque(opaque),
        })
        .collect();
    println!("   blocks: {}", blocks.join(" "));
    report_round_trip(&bytes, file.encode())
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
            GstBlock::TransferStash(_) => block.id().raw().to_string(),
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
