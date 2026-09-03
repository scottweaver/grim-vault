//! Shared by the examples: loading the layered game data from an
//! install (three database layers, three text archives, three item
//! archives) and describing an item with resolved names.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use grimvault_core::gamedata::{GameData, text_from_archives};
use grimvault_core::item::Item;
use grimvault_core::settings::{ConfigDir, Settings};
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

/// Where an example reads from: explicit `--game DIR`, `--save DIR`,
/// and `--store FILE` flags win, and anything not given comes from the
/// app's saved settings (`settings.json` in the config directory, the
/// store beside it) so the paths need typing only once, in the app.
pub struct CliPaths {
    pub game_dir: PathBuf,
    pub save_dir: PathBuf,
    // Shared by every example; the read-only ones never open the store.
    #[allow(dead_code)]
    pub store_path: PathBuf,
    /// The arguments that were not path flags, in order.
    pub rest: Vec<String>,
}

pub fn cli_paths(args: &[String]) -> Result<CliPaths, Box<dyn Error>> {
    let (mut game, mut save, mut store) = (None, None, None);
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let slot = match word.as_str() {
            "--game" => &mut game,
            "--save" => &mut save,
            "--store" => &mut store,
            _ => {
                rest.push(word.clone());
                continue;
            }
        };
        let Some(value) = words.next() else {
            return Err(format!("{word} needs a value").into());
        };
        *slot = Some(PathBuf::from(value));
    }
    let config = ConfigDir::resolve()?;
    let settings = if game.is_none() || save.is_none() {
        Some(load_settings(&config)?)
    } else {
        None
    };
    let from_settings = |field: fn(&Settings) -> &PathBuf| settings.as_ref().map(field).cloned();
    Ok(CliPaths {
        game_dir: game
            .or_else(|| from_settings(|s| &s.game_dir))
            .ok_or("no game dir: pass --game DIR or run the app once")?,
        save_dir: save
            .or_else(|| from_settings(|s| &s.save_dir))
            .ok_or("no save dir: pass --save DIR or run the app once")?,
        store_path: store.unwrap_or_else(|| config.store_file()),
        rest,
    })
}

fn load_settings(config: &ConfigDir) -> Result<Settings, Box<dyn Error>> {
    let path = config.settings_file();
    if !path.is_file() {
        return Err(format!(
            "no settings at {}: run the app once, or pass --game DIR --save DIR",
            path.display()
        )
        .into());
    }
    Ok(Settings::parse(&fs::read(&path)?)?)
}
