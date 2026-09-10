//! Shared by the examples: loading the layered game data from an
//! install through `grimvault_io` (the shipped layers, then every
//! installed mod as a fill layer) and describing an item with
//! resolved names.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Instant;

use grimvault_core::campaign::{Campaign, ModName};
use grimvault_core::gamedata::{GameData, mod_layers, shipped_layers};
use grimvault_core::item::Item;
use grimvault_core::settings::{ConfigDir, Settings};
use grimvault_io::layers::{LayerFile, list_mods, read_layers};
use univault_engine::ids::RecordId;

pub fn load_game_data(game_dir: &Path) -> Result<GameData, Box<dyn Error>> {
    let started = Instant::now();
    let (shipped, mods) = read_layers(
        game_dir,
        &shipped_layers(),
        &mod_layers(list_mods(game_dir)),
        &LayerFile::ALL,
        &mut |_| {},
    )?;
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

/// `Prefix Base Suffix xN [Rarity WxH]`, with what the database cannot
/// resolve left out or marked.
#[allow(
    dead_code,
    reason = "shared by every example; each uses its own subset"
)]
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
/// store where its `storeFile` says, beside it by default) so the
/// paths need typing only once, in the app. `--mod NAME` selects a
/// mod's shared files under `save/<NAME>/`; the main campaign's are
/// the default.
pub struct CliPaths {
    pub game_dir: PathBuf,
    #[allow(
        dead_code,
        reason = "shared by every example; each uses its own subset"
    )]
    pub save_dir: PathBuf,
    // Shared by every example; the read-only ones never open the store
    // or the shared files.
    #[allow(dead_code)]
    pub store_path: PathBuf,
    #[allow(dead_code)]
    pub campaign: Campaign,
    /// The arguments that were not path flags, in order.
    pub rest: Vec<String>,
}

pub fn cli_paths(args: &[String]) -> Result<CliPaths, Box<dyn Error>> {
    let (mut game, mut save, mut store, mut mod_name) = (None, None, None, None);
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let slot = match word.as_str() {
            "--game" => &mut game,
            "--save" => &mut save,
            "--store" => &mut store,
            "--mod" => &mut mod_name,
            _ => {
                rest.push(word.clone());
                continue;
            }
        };
        let Some(value) = words.next() else {
            return Err(format!("{word} needs a value").into());
        };
        *slot = Some(value.clone());
    }
    let campaign = mod_name.map_or(Ok(Campaign::Main), |name| {
        ModName::parse(&name).map(Campaign::Mod)
    })?;
    let config = ConfigDir::resolve()?;
    let settings = if game.is_none() || save.is_none() || store.is_none() {
        Some(load_settings(&config)?)
    } else {
        None
    };
    let from_settings = |field: fn(&Settings) -> &PathBuf| settings.as_ref().map(field).cloned();
    Ok(CliPaths {
        game_dir: game
            .map(PathBuf::from)
            .or_else(|| from_settings(|s| &s.game_dir))
            .ok_or("no game dir: pass --game DIR or run the app once")?,
        save_dir: save
            .map(PathBuf::from)
            .or_else(|| from_settings(|s| &s.save_dir))
            .ok_or("no save dir: pass --save DIR or run the app once")?,
        store_path: store.map_or_else(
            || {
                settings
                    .as_ref()
                    .map_or_else(|| config.store_file(), |s| s.store_file(&config))
            },
            PathBuf::from,
        ),
        campaign,
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
    Ok(Settings::parse(&univault_io::read_verified(&path)?)?)
}
