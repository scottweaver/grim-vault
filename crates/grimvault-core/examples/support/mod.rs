//! Shared by the examples: loading the layered game data from an
//! install (the shipped layers, then every installed mod as a fill
//! layer) and describing an item with resolved names.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use grimvault_core::gamedata::{
    GameData, LayerFiles, LayerSet, ModListing, mod_layers, shipped_layers,
};
use grimvault_core::item::Item;
use grimvault_core::settings::{ConfigDir, Settings};
use univault_engine::arc::ArcFile;
use univault_engine::arz::{ArzDialect, ArzFile};
use univault_engine::codec::Codec;
use univault_engine::ids::RecordId;

pub fn load_game_data(game_dir: &Path) -> Result<GameData, Box<dyn Error>> {
    let started = Instant::now();
    let shipped = load_layers(game_dir, &shipped_layers())?;
    let mods = load_layers(game_dir, &mod_layers(list_mods(game_dir)))?;
    println!(
        "game data: {} database layers, {} text archives, {} item archives, {} mod layers, \
         loaded in {:.1?}",
        shipped.databases.len(),
        shipped.text_archives.len(),
        shipped.item_archives.len(),
        mods.databases.len(),
        started.elapsed()
    );
    Ok(GameData::layered(shipped, mods)?)
}

fn load_layers(game_dir: &Path, layers: &[LayerFiles]) -> Result<LayerSet, Box<dyn Error>> {
    let mut set = LayerSet::default();
    for layer in layers {
        if let Some(bytes) = read_if_present(game_dir, &layer.database)? {
            set.databases
                .push(ArzFile::parse(bytes, ArzDialect::grim_dawn())?);
        }
        if let Some(bytes) = read_if_present(game_dir, &layer.text)? {
            set.text_archives
                .push(ArcFile::parse(bytes, Codec::Lz4Block)?);
        }
        if let Some(bytes) = read_if_present(game_dir, &layer.items)? {
            set.item_archives
                .push(ArcFile::parse(bytes, Codec::Lz4Block)?);
        }
    }
    Ok(set)
}

fn read_if_present(game_dir: &Path, relative: &Path) -> Result<Option<Vec<u8>>, Box<dyn Error>> {
    let path = game_dir.join(relative);
    if !path.is_file() {
        return Ok(None);
    }
    Ok(Some(fs::read(path)?))
}

/// Every folder under `mods/` with the names its `database/` and
/// `resources/` hold; an absent `mods/` is simply no mods.
fn list_mods(game_dir: &Path) -> Vec<ModListing> {
    let Ok(folders) = fs::read_dir(game_dir.join("mods")) else {
        return Vec::new();
    };
    folders
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| ModListing {
            folder: entry.file_name().to_string_lossy().into_owned(),
            database_files: file_names(&entry.path().join("database")),
            resource_files: file_names(&entry.path().join("resources")),
        })
        .collect()
}

fn file_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
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
