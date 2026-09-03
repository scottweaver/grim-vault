//! Read-only smoke against a real install: joins every character's
//! save and the transfer stash to the layered game data and prints
//! resolved item names, rarity, and footprints, plus the byte-identical
//! round-trip status of every file.
//!
//! `cargo run --release -p grimvault-core --example smoke -- <game dir> <save dir>`

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use grimvault_core::gamedata::{GameData, text_from_archives};
use grimvault_core::gdc::PlayerFile;
use grimvault_core::gst::GstFile;
use grimvault_core::item::Item;
use univault_engine::arc::ArcFile;
use univault_engine::arz::{ArzDialect, ArzFile};
use univault_engine::codec::Codec;
use univault_engine::ids::RecordId;

const DATABASES: [&str; 3] = [
    "database/database.arz",
    "gdx1/database/GDX1.arz",
    "gdx2/database/GDX2.arz",
];
const TEXT_ARCHIVES: [&str; 3] = [
    "resources/Text_EN.arc",
    "gdx1/resources/Text_EN.arc",
    "gdx2/resources/Text_EN.arc",
];
const ITEM_ARCHIVES: [&str; 3] = [
    "resources/Items.arc",
    "gdx1/resources/Items.arc",
    "gdx2/resources/Items.arc",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(game_dir), Some(save_dir)) = (args.next(), args.next()) else {
        return Err("usage: smoke <game dir> <save dir>".into());
    };
    let game_data = load_game_data(Path::new(&game_dir))?;
    let save_dir = Path::new(&save_dir);

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

fn load_game_data(game_dir: &Path) -> Result<GameData, Box<dyn Error>> {
    let started = Instant::now();
    let databases = DATABASES
        .iter()
        .map(|relative| game_dir.join(relative))
        .filter(|path| path.is_file())
        .map(|path| Ok(ArzFile::parse(fs::read(path)?, ArzDialect::grim_dawn())?))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let text_archives = load_archives(game_dir, &TEXT_ARCHIVES)?;
    let item_archives = load_archives(game_dir, &ITEM_ARCHIVES)?;
    let text = text_from_archives(&text_archives)?;
    println!(
        "game data: {} database layers, {} text archives, {} item archives, loaded in {:.1?}",
        databases.len(),
        text_archives.len(),
        item_archives.len(),
        started.elapsed()
    );
    Ok(GameData::from_parts(databases, text, item_archives))
}

fn load_archives(game_dir: &Path, relatives: &[&str]) -> Result<Vec<ArcFile>, Box<dyn Error>> {
    relatives
        .iter()
        .map(|relative| game_dir.join(relative))
        .filter(|path| path.is_file())
        .map(|path| Ok(ArcFile::parse(fs::read(path)?, Codec::Lz4Block)?))
        .collect()
}

fn describe(game_data: &GameData, item: &Item) -> String {
    let Some(base) = RecordId::parse(item.base_name.clone()) else {
        return "<empty>".to_string();
    };
    let prefix = RecordId::parse(item.prefix_name.clone());
    let suffix = RecordId::parse(item.suffix_name.clone());
    let name = game_data
        .display_name(&base, prefix.as_ref(), suffix.as_ref())
        .unwrap_or_else(|| format!("<unknown record {}>", base.as_str()));
    let info = game_data.item_info(&base).and_then(Result::ok);
    let rarity = info
        .as_ref()
        .and_then(|info| info.rarity)
        .map_or_else(String::new, |rarity| format!("{rarity:?} "));
    let footprint = info
        .as_ref()
        .and_then(|info| info.bitmap.as_ref())
        .and_then(|bitmap| game_data.footprint(bitmap))
        .and_then(Result::ok)
        .map_or_else(
            || "?".to_string(),
            |footprint| format!("{}x{}", footprint.width, footprint.height),
        );
    let stack = if item.stack_count > 1 {
        format!(" x{}", item.stack_count)
    } else {
        String::new()
    };
    format!("{name}{stack} [{rarity}{footprint}]")
}

fn round_trip(original: &[u8], encoded: &[u8]) -> &'static str {
    if original == encoded {
        "OK (byte-identical)"
    } else {
        "FAIL"
    }
}
