//! File discovery and reading — the IO half of the server. All
//! format knowledge stays in `grimvault-core` and the discovery in
//! `grimvault-io`; this module decides which files a tool call reads
//! and reads them, uncached and length-checked, because the game and
//! the desktop shell write them at any time and they live on a
//! network mount.

use std::io;
use std::path::{Path, PathBuf};

use grimvault_core::formulas::{Formulas, FormulasError};
use grimvault_core::gamedata::{GameData, mod_layers, shipped_layers};
use grimvault_core::gdc::{GdcError, PlayerFile, Realm};
use grimvault_core::gst::{GstError, GstFile};
use grimvault_core::settings::{ConfigDir, NoConfigDir, Settings};
use grimvault_core::store::{StoreError, VaultStore};
use grimvault_io::characters;
use grimvault_io::dirs::{DirProblem, GameDir, SaveDir};
use grimvault_io::layers::{ArchiveFailure, LayerFile, list_mods, read_layers};
use grimvault_io::settings::SettingsError;
use thiserror::Error;
use univault_engine::arc::ArcError;

/// Where the server reads from: the desktop shell's `settings.json`
/// under the config directory (`GRIMVAULT_CONFIG_DIR` overrides the
/// platform's), so the paths are typed once, in the app.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: ConfigDir,
    pub settings: Settings,
    pub game: GameDir,
    pub save: SaveDir,
    pub store: PathBuf,
}

/// Why the server has nothing to read.
#[derive(Debug, Error)]
pub enum PathsError {
    #[error("{0}")]
    NoConfigDir(#[from] NoConfigDir),
    #[error("{0}")]
    Settings(#[from] SettingsError),
    #[error(
        "no settings at {}: run the Grim Vault app once to choose the game and save \
         directories, or point GRIMVAULT_CONFIG_DIR at a directory holding a settings.json",
        path.display()
    )]
    NoSettings { path: PathBuf },
    #[error("{0}")]
    Dir(#[from] DirProblem),
}

impl Paths {
    /// Resolves the config directory and reads the settings under it.
    ///
    /// # Errors
    /// [`PathsError`] when there is no config directory, no settings
    /// file, or a directory the settings name does not validate.
    pub fn from_settings() -> Result<Self, PathsError> {
        let config = ConfigDir::resolve()?;
        let settings =
            grimvault_io::settings::load(&config)?.ok_or_else(|| PathsError::NoSettings {
                path: config.settings_file(),
            })?;
        let game = GameDir::parse(&settings.game_dir)?;
        let save = SaveDir::parse(&settings.save_dir)?;
        let store = settings.store_file(&config);
        Ok(Self {
            config,
            settings,
            game,
            save,
            store,
        })
    }
}

/// The record database and text of every layer, with the layers named
/// in the order [`GameData`] composed them (mods first, then the
/// shipped layers base-first) so a record can say where it came from.
pub struct Loaded {
    pub game: GameData,
    pub layers: Vec<String>,
}

/// Why the game data could not be loaded.
#[derive(Debug, Error)]
pub enum LoadError {
    #[error(transparent)]
    Archive(#[from] ArchiveFailure),
    #[error("localization text: {0}")]
    Localization(ArcError),
}

/// Reads the record databases and text archives — not the item
/// bitmaps, which nothing here draws — of the shipped and mod layers.
///
/// # Errors
/// [`LoadError`] for the first archive that could not be read.
pub fn load_game_data(game_dir: &GameDir) -> Result<Loaded, LoadError> {
    let dir = game_dir.path();
    let shipped_files = shipped_layers();
    let mod_files = mod_layers(list_mods(dir));
    let (shipped, mods) = read_layers(
        dir,
        &shipped_files,
        &mod_files,
        &LayerFile::HEADLESS,
        &mut |_| {},
    )?;
    let present = |layers: &[grimvault_core::gamedata::LayerFiles]| -> Vec<String> {
        layers
            .iter()
            .filter(|layer| dir.join(&layer.database).is_file())
            .map(|layer| layer.database.display().to_string())
            .collect()
    };
    let mut layers = present(&mod_files);
    layers.extend(present(&shipped_files));
    let game = GameData::layered(shipped, mods).map_err(LoadError::Localization)?;
    Ok(Loaded { game, layers })
}

/// Why a game-owned file or the store could not be read.
#[derive(Debug, Error)]
pub enum ReadError {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}: {source}", path.display())]
    Store { path: PathBuf, source: StoreError },
    #[error("{}: {source}", path.display())]
    Gst { path: PathBuf, source: GstError },
    #[error("{}: {source}", path.display())]
    Formulas {
        path: PathBuf,
        source: FormulasError,
    },
    #[error("{}: {source}", path.display())]
    Character { path: PathBuf, source: GdcError },
}

fn read(path: &Path) -> Result<Vec<u8>, ReadError> {
    univault_io::read_verified(path).map_err(|source| ReadError::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// The vault store, read fresh: the desktop shell autosaves it, so
/// caching would serve stale items. A store file that does not exist
/// yet is an empty store, not an error.
///
/// # Errors
/// [`ReadError`] when the file exists but cannot be read or parsed.
pub fn read_store(path: &Path) -> Result<VaultStore, ReadError> {
    if !path.exists() {
        return Ok(VaultStore::new());
    }
    VaultStore::from_json(&read(path)?).map_err(|source| ReadError::Store {
        path: path.to_path_buf(),
        source,
    })
}

/// A `.gst` file (the transfer stash or the component storage).
///
/// # Errors
/// [`ReadError`].
pub fn read_gst(path: &Path) -> Result<GstFile, ReadError> {
    GstFile::parse(&read(path)?).map_err(|source| ReadError::Gst {
        path: path.to_path_buf(),
        source,
    })
}

/// A campaign's learned-blueprint list; `None` when the game has not
/// written one there yet.
///
/// # Errors
/// [`ReadError`].
pub fn read_formulas(path: &Path) -> Result<Option<Formulas>, ReadError> {
    if !path.is_file() {
        return Ok(None);
    }
    Formulas::parse(&read(path)?)
        .map(Some)
        .map_err(|source| ReadError::Formulas {
            path: path.to_path_buf(),
            source,
        })
}

/// A character's `player.gdc`.
///
/// # Errors
/// [`ReadError`].
pub fn read_character(path: &Path) -> Result<PlayerFile, ReadError> {
    PlayerFile::parse(&read(path)?).map_err(|source| ReadError::Character {
        path: path.to_path_buf(),
        source,
    })
}

/// One discovered character file: the realm its folder is under
/// (the file never names it), the folder's display name, and the
/// path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterFile {
    pub realm: Realm,
    pub name: String,
    pub path: PathBuf,
}

/// Every character under `main/` then `user/`, in folder order.
#[must_use]
pub fn discover_characters(save: &SaveDir) -> Vec<CharacterFile> {
    characters::discover(save)
        .into_iter()
        .map(|(realm, path)| CharacterFile {
            realm,
            name: characters::folder_name(&path),
            path,
        })
        .collect()
}
