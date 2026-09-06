//! The Loading phase: reading the layered game data exactly as
//! `grimvault-core`'s examples do (the shipped layers, missing
//! expansion files skipped, then every installed mod under `mods/` as
//! a fill layer), then opening the transfer stash, the component /
//! crafting-material storage, the vault store, and every character.
//! [`load_world`] is the whole path as one function, so the window
//! and the headless `--check` run the same code; [`start`] moves it
//! onto a thread and reports progress over a channel.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use grimvault_core::gamedata::{
    GameData, LayerFiles, LayerSet, ModListing, mod_layers, shipped_layers,
};
use thiserror::Error;
use univault_engine::arc::{ArcError, ArcFile};
use univault_engine::arz::{ArzDialect, ArzError, ArzFile};
use univault_engine::codec::Codec;

use crate::documents::{
    CharacterEntry, GstOpenError, Reagents, StashDoc, StoreDoc, StoreOpenError, open_characters,
};
use crate::setup::{GameDir, SaveDir};

/// Where everything lives, validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldPaths {
    pub game: GameDir,
    pub save: SaveDir,
    pub store: PathBuf,
}

/// One step of the load, reported as it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadStep {
    Database(PathBuf),
    TextArchive(PathBuf),
    ItemArchive(PathBuf),
    Localization,
    Stash,
    Reagents,
    Store,
    Characters,
}

impl fmt::Display for LoadStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(relative) => {
                write!(f, "reading record database {}", relative.display())
            }
            Self::TextArchive(relative) => {
                write!(f, "reading text archive {}", relative.display())
            }
            Self::ItemArchive(relative) => {
                write!(f, "reading item archive {}", relative.display())
            }
            Self::Localization => f.write_str("building the localization table"),
            Self::Stash => f.write_str("opening transfer.gst"),
            Self::Reagents => f.write_str("opening reagents.gst"),
            Self::Store => f.write_str("opening the vault store"),
            Self::Characters => f.write_str("opening characters"),
        }
    }
}

/// What the game-data half of the load found: the shipped layers by
/// kind, and how many mods contributed a database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub databases: usize,
    pub text_archives: usize,
    pub item_archives: usize,
    pub mods: usize,
    pub elapsed: Duration,
}

/// Everything the Ready phase needs.
pub struct LoadedWorld {
    pub game: GameData,
    pub report: LoadReport,
    pub stash: StashDoc,
    pub reagents: Reagents,
    pub store: StoreDoc,
    pub characters: Vec<CharacterEntry>,
}

/// Why the load stopped.
#[derive(Debug, Error)]
pub enum LoadFailure {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("record database {}: {source}", path.display())]
    Database { path: PathBuf, source: ArzError },
    #[error("archive {}: {source}", path.display())]
    Archive { path: PathBuf, source: ArcError },
    #[error("localization text: {0}")]
    Localization(ArcError),
    #[error("transfer stash: {0}")]
    Stash(#[from] GstOpenError),
    #[error("vault store: {0}")]
    Store(#[from] StoreOpenError),
}

/// The whole load, reporting each step to `progress` as it begins.
///
/// # Errors
/// [`LoadFailure`] for the first fatal step; a character that fails
/// to open, or a `reagents.gst` that is absent or cannot be typed, is
/// not fatal and is reported inside the world instead.
pub fn load_world(
    paths: &WorldPaths,
    progress: &mut dyn FnMut(LoadStep),
) -> Result<LoadedWorld, LoadFailure> {
    let started = Instant::now();
    let game_dir = paths.game.path();
    let shipped = load_layers(game_dir, &shipped_layers(), progress)?;
    let mods = load_layers(game_dir, &mod_layers(list_mods(game_dir)), progress)?;
    progress(LoadStep::Localization);
    let report = LoadReport {
        databases: shipped.databases.len(),
        text_archives: shipped.text_archives.len(),
        item_archives: shipped.item_archives.len(),
        mods: mods.databases.len(),
        elapsed: started.elapsed(),
    };
    let game = GameData::layered(shipped, mods).map_err(LoadFailure::Localization)?;

    progress(LoadStep::Stash);
    let stash = StashDoc::open(paths.save.transfer_stash())?;
    progress(LoadStep::Reagents);
    let reagents = Reagents::open(paths.save.reagent_storage());
    progress(LoadStep::Store);
    let store = StoreDoc::open(paths.store.clone())?;
    progress(LoadStep::Characters);
    let characters = open_characters(&paths.save);
    Ok(LoadedWorld {
        game,
        report,
        stash,
        reagents,
        store,
        characters,
    })
}

/// Reads whichever of each layer's files exist, in layer order.
fn load_layers(
    game_dir: &Path,
    layers: &[LayerFiles],
    progress: &mut dyn FnMut(LoadStep),
) -> Result<LayerSet, LoadFailure> {
    let mut set = LayerSet::default();
    for layer in layers {
        if let Some(bytes) =
            read_if_present(game_dir, &layer.database, progress, LoadStep::Database)?
        {
            let database = ArzFile::parse(bytes, ArzDialect::grim_dawn()).map_err(|source| {
                LoadFailure::Database {
                    path: game_dir.join(&layer.database),
                    source,
                }
            })?;
            set.databases.push(database);
        }
        if let Some(archive) = read_archive(game_dir, &layer.text, progress, LoadStep::TextArchive)?
        {
            set.text_archives.push(archive);
        }
        if let Some(archive) =
            read_archive(game_dir, &layer.items, progress, LoadStep::ItemArchive)?
        {
            set.item_archives.push(archive);
        }
    }
    Ok(set)
}

fn read_archive(
    game_dir: &Path,
    relative: &Path,
    progress: &mut dyn FnMut(LoadStep),
    step: fn(PathBuf) -> LoadStep,
) -> Result<Option<ArcFile>, LoadFailure> {
    read_if_present(game_dir, relative, progress, step)?
        .map(|bytes| {
            ArcFile::parse(bytes, Codec::Lz4Block).map_err(|source| LoadFailure::Archive {
                path: game_dir.join(relative),
                source,
            })
        })
        .transpose()
}

/// The bytes of `relative` under the game directory, reported as a
/// step; `None` when the file is not there (an expansion or mod
/// resource that does not exist is not an error).
fn read_if_present(
    game_dir: &Path,
    relative: &Path,
    progress: &mut dyn FnMut(LoadStep),
    step: fn(PathBuf) -> LoadStep,
) -> Result<Option<Vec<u8>>, LoadFailure> {
    let path = game_dir.join(relative);
    if !path.is_file() {
        return Ok(None);
    }
    progress(step(relative.to_path_buf()));
    read(&path).map(Some)
}

/// Every folder under `mods/` with the names its `database/` and
/// `resources/` hold; an absent `mods/` is simply no mods.
fn list_mods(game_dir: &Path) -> Vec<ModListing> {
    let Ok(folders) = std::fs::read_dir(game_dir.join("mods")) else {
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
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Archives are read-only reference data on a possibly stale mount:
/// the verified read turns a short read into an error instead of a
/// corrupt parse.
fn read(path: &Path) -> Result<Vec<u8>, LoadFailure> {
    univault_io::read_verified(path).map_err(|source| LoadFailure::Read {
        path: path.to_path_buf(),
        source,
    })
}

/// What the loader thread sends.
pub enum LoadEvent {
    Step(LoadStep),
    Done(Box<LoadedWorld>),
    Failed(LoadFailure),
}

/// A load in flight: the steps reported so far and the channel the
/// outcome arrives on.
pub struct LoadJob {
    pub paths: WorldPaths,
    pub steps: Vec<LoadStep>,
    events: Receiver<LoadEvent>,
}

/// How a load ended.
pub enum LoadOutcome {
    Done(Box<LoadedWorld>),
    Failed(LoadFailure),
}

impl LoadJob {
    /// Drains progress into `steps`; the outcome once it has arrived.
    pub fn poll(&mut self) -> Option<LoadOutcome> {
        while let Ok(event) = self.events.try_recv() {
            match event {
                LoadEvent::Step(step) => self.steps.push(step),
                LoadEvent::Done(world) => return Some(LoadOutcome::Done(world)),
                LoadEvent::Failed(failure) => return Some(LoadOutcome::Failed(failure)),
            }
        }
        None
    }
}

/// Runs [`load_world`] on a thread; every event asks `wake` to
/// repaint so the progress panel advances without pointer motion.
#[must_use]
pub fn start(paths: WorldPaths, wake: egui::Context) -> LoadJob {
    let (sender, events) = channel::<LoadEvent>();
    let job_paths = paths.clone();
    let spawned = std::thread::Builder::new()
        .name("grimvault-load".into())
        .spawn(move || run(&job_paths, &sender, &wake));
    if let Err(error) = spawned {
        let (fallback, events) = channel::<LoadEvent>();
        let _ = fallback.send(LoadEvent::Failed(LoadFailure::Read {
            path: paths.game.path().to_path_buf(),
            source: error,
        }));
        return LoadJob {
            paths,
            steps: Vec::new(),
            events,
        };
    }
    LoadJob {
        paths,
        steps: Vec::new(),
        events,
    }
}

fn run(paths: &WorldPaths, sender: &Sender<LoadEvent>, wake: &egui::Context) {
    let mut report = |step: LoadStep| {
        let _ = sender.send(LoadEvent::Step(step));
        wake.request_repaint();
    };
    let outcome = match load_world(paths, &mut report) {
        Ok(world) => LoadEvent::Done(Box::new(world)),
        Err(failure) => LoadEvent::Failed(failure),
    };
    let _ = sender.send(outcome);
    wake.request_repaint();
}
