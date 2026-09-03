//! The Loading phase: reading the layered game data exactly as
//! `grimvault-core`'s examples do (three database layers, three
//! `Text_EN.arc`, three `Items.arc`, missing expansion files skipped),
//! then opening the transfer stash, the component / crafting-material
//! storage, the vault store, and every character. [`load_world`] is the whole path as one function, so the
//! window and the headless `--check` run the same code; [`start`] moves
//! it onto a thread and reports progress over a channel.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use grimvault_core::gamedata::{GameData, text_from_archives};
use thiserror::Error;
use univault_engine::arc::{ArcError, ArcFile};
use univault_engine::arz::{ArzDialect, ArzError, ArzFile};
use univault_engine::codec::Codec;

use crate::documents::{
    CharacterEntry, GstOpenError, Reagents, StashDoc, StoreDoc, StoreOpenError, open_characters,
};
use crate::setup::{GameDir, SaveDir};

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
    Database(&'static str),
    TextArchive(&'static str),
    ItemArchive(&'static str),
    Localization,
    Stash,
    Reagents,
    Store,
    Characters,
}

impl fmt::Display for LoadStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(relative) => write!(f, "reading record database {relative}"),
            Self::TextArchive(relative) => write!(f, "reading text archive {relative}"),
            Self::ItemArchive(relative) => write!(f, "reading item archive {relative}"),
            Self::Localization => f.write_str("building the localization table"),
            Self::Stash => f.write_str("opening transfer.gst"),
            Self::Reagents => f.write_str("opening reagents.gst"),
            Self::Store => f.write_str("opening the vault store"),
            Self::Characters => f.write_str("opening characters"),
        }
    }
}

/// What the game-data half of the load found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub databases: usize,
    pub text_archives: usize,
    pub item_archives: usize,
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
    let mut databases = Vec::new();
    for relative in DATABASES {
        let path = game_dir.join(relative);
        if !path.is_file() {
            continue;
        }
        progress(LoadStep::Database(relative));
        let bytes = read(&path)?;
        let database = ArzFile::parse(bytes, ArzDialect::grim_dawn())
            .map_err(|source| LoadFailure::Database { path, source })?;
        databases.push(database);
    }
    let text_archives = load_archives(game_dir, &TEXT_ARCHIVES, progress, LoadStep::TextArchive)?;
    let item_archives = load_archives(game_dir, &ITEM_ARCHIVES, progress, LoadStep::ItemArchive)?;
    progress(LoadStep::Localization);
    let text = text_from_archives(&text_archives).map_err(LoadFailure::Localization)?;
    let report = LoadReport {
        databases: databases.len(),
        text_archives: text_archives.len(),
        item_archives: item_archives.len(),
        elapsed: started.elapsed(),
    };
    let game = GameData::from_parts(databases, text, item_archives);

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

fn load_archives(
    game_dir: &Path,
    relatives: &[&'static str],
    progress: &mut dyn FnMut(LoadStep),
    step: fn(&'static str) -> LoadStep,
) -> Result<Vec<ArcFile>, LoadFailure> {
    let mut archives = Vec::new();
    for relative in relatives {
        let path = game_dir.join(relative);
        if !path.is_file() {
            continue;
        }
        progress(step(relative));
        let bytes = read(&path)?;
        let archive = ArcFile::parse(bytes, Codec::Lz4Block)
            .map_err(|source| LoadFailure::Archive { path, source })?;
        archives.push(archive);
    }
    Ok(archives)
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
