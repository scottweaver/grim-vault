//! Read-only smoke against a real install: joins every character's
//! save and the transfer stash to the layered game data and prints
//! resolved item names, rarity, and footprints, plus the byte-identical
//! round-trip status of every file.
//!
//! `cargo run --release -p grimvault-core --example smoke -- [--game DIR] [--save DIR]`
//! — paths not given come from the app's saved settings.

mod support;

use std::error::Error;
use std::fs;
use std::path::Path;

use grimvault_core::gdc::PlayerFile;
use grimvault_core::gst::GstFile;

use support::{cli_paths, describe, load_game_data};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths = cli_paths(&args)?;
    if !paths.rest.is_empty() {
        return Err("usage: smoke [--game DIR] [--save DIR]".into());
    }
    let game_data = load_game_data(&paths.game_dir)?;
    let save_dir: &Path = &paths.save_dir;

    let mut characters: Vec<_> = fs::read_dir(save_dir.join("main"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("player.gdc"))
        .filter(|path| path.is_file())
        .collect();
    characters.sort();
    for path in characters {
        let bytes = fs::read(&path)?;
        let player = PlayerFile::parse(&bytes)?;
        println!(
            "\n{} — level {} {}",
            player.character_name(),
            player.level(),
            game_data
                .tag_text(&player.header().class_tag)
                .unwrap_or("(no class)")
        );
        if let Some(inventory) = player.inventory() {
            for (index, sack) in inventory.sacks().iter().enumerate() {
                println!("  sack {index}: {} items", sack.items.len());
                for placed in &sack.items {
                    println!(
                        "    ({:>2},{:>2}) {}",
                        placed.x,
                        placed.y,
                        describe(&game_data, &placed.item)
                    );
                }
            }
            let equipped: Vec<_> = inventory
                .equipped()
                .filter(|slot| !slot.item.is_empty())
                .collect();
            println!("  equipped: {} items", equipped.len());
            for slot in equipped {
                println!("    {}", describe(&game_data, &slot.item));
            }
        }
        if let Some(stash) = player.stash() {
            for (index, tab) in stash.tabs.iter().enumerate() {
                println!(
                    "  personal stash tab {index} ({}x{}): {} items",
                    tab.width,
                    tab.height,
                    tab.items.len()
                );
                for placed in tab.items.iter().take(5) {
                    println!(
                        "    ({:>4.0},{:>4.0}) {}",
                        placed.x,
                        placed.y,
                        describe(&game_data, &placed.item)
                    );
                }
            }
        }
        println!("  round-trip: {}", round_trip(&bytes, &player.encode()?));
    }

    let transfer_path = save_dir.join("transfer.gst");
    let bytes = fs::read(&transfer_path)?;
    let transfer = GstFile::parse(&bytes)?;
    println!("\ntransfer.gst");
    if let Some(stash) = transfer.transfer_stash() {
        for (index, tab) in stash.tabs.iter().enumerate() {
            println!(
                "  tab {index} ({}x{}): {} items",
                tab.width,
                tab.height,
                tab.items.len()
            );
            for placed in &tab.items {
                println!(
                    "    ({:>4.0},{:>4.0}) {}",
                    placed.x,
                    placed.y,
                    describe(&game_data, &placed.item)
                );
            }
        }
    }
    println!("  round-trip: {}", round_trip(&bytes, &transfer.encode()?));
    Ok(())
}

fn round_trip(original: &[u8], encoded: &[u8]) -> &'static str {
    if original == encoded {
        "OK (byte-identical)"
    } else {
        "FAIL"
    }
}
