//! Shared by the examples: loading the layered game data from an
//! install (three database layers, three text archives, three item
//! archives) and describing an item with resolved names.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use grimvault_core::gamedata::{GameData, text_from_archives};
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

pub fn load_game_data(game_dir: &Path) -> Result<GameData, Box<dyn Error>> {
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

/// `Prefix Base Suffix xN [Rarity WxH]`, with what the database cannot
/// resolve left out or marked.
pub fn describe(game_data: &GameData, item: &Item) -> String {
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
