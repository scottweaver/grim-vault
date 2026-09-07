//! The Loading phase: reading the layered game data exactly as
//! `grimvault-core`'s examples do (the shipped layers, missing
//! expansion files skipped, then every installed mod under `mods/` as
//! a fill layer), then opening the transfer stash, the component /
//! crafting-material storage, the vault store, and every character.
//! [`load_world`] is the whole path as one function, so the window
//! and the headless `--check` run the same code; [`start`] moves it
//! onto a thread and reports progress over a channel.
//!
//! The tile symbols are the one thing read by entry rather than whole:
//! each layer's `UI.arc` runs to a quarter gigabyte, and the eleven
//! 4 KB textures the app wants are found through the archive's
//! directory ([`ArcIndex`]) and read as byte ranges
//! ([`univault_io::read_ranges`]).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use grimvault_core::campaign::Campaign;
use grimvault_core::facets::Symbol;
use grimvault_core::gamedata::{
    GameData, LayerFiles, LayerSet, ModListing, mod_layers, shipped_layers,
};
use thiserror::Error;
use univault_engine::arc::{ArcError, ArcFile, ArcHeader, ArcIndex, Located};
use univault_engine::arz::{ArzDialect, ArzError, ArzFile};
use univault_engine::codec::Codec;

use crate::badges::SymbolTextures;
use crate::crafting::{Blueprints, IllusionCollection};
use crate::documents::{
    CharacterEntry, GstOpenError, Reagents, StashDoc, StoreDoc, StoreOpenError, open_characters,
};
use crate::icons::IconProblem;
use crate::setup::{GameDir, SaveDir};

/// Where everything lives, validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldPaths {
    pub game: GameDir,
    pub save: SaveDir,
    pub store: PathBuf,
    /// The shell's persisted view state, beside the store.
    pub ui_state: PathBuf,
}

/// One step of the load, reported as it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadStep {
    Database(PathBuf),
    TextArchive(PathBuf),
    ItemArchive(PathBuf),
    Localization,
    UiArchive(PathBuf),
    Stash,
    Reagents,
    Blueprints,
    Illusions,
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
            Self::UiArchive(relative) => {
                write!(f, "reading tile symbols from {}", relative.display())
            }
            Self::Stash => f.write_str("opening transfer.gst"),
            Self::Reagents => f.write_str("opening reagents.gst"),
            Self::Blueprints => f.write_str("opening formulas.gst"),
            Self::Illusions => f.write_str("opening transmutes.gst"),
            Self::Store => f.write_str("opening the vault store"),
            Self::Characters => f.write_str("opening characters"),
        }
    }
}

/// What the game-data half of the load found: the shipped layers by
/// kind, how many mods contributed a database, and how many of the
/// tile symbols have their texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub databases: usize,
    pub text_archives: usize,
    pub item_archives: usize,
    pub mods: usize,
    pub symbols: usize,
    pub elapsed: Duration,
}

/// Everything the Ready phase needs.
pub struct LoadedWorld {
    pub game: GameData,
    pub report: LoadReport,
    pub symbols: SymbolTextures,
    /// Every campaign the save directory holds, main first.
    pub campaigns: Vec<Campaign>,
    /// Whose shared files `stash`, `reagents`, `blueprints`, and
    /// `illusions` are.
    pub campaign: Campaign,
    pub stash: StashDoc,
    pub reagents: Reagents,
    pub blueprints: Blueprints,
    pub illusions: IllusionCollection,
    pub store: StoreDoc,
    pub characters: Vec<CharacterEntry>,
    /// Cross-checks that failed without stopping the load, for the
    /// shell to show.
    pub warnings: Vec<String>,
}

/// One campaign's shared files as opened.
pub struct SharedDocs {
    pub stash: StashDoc,
    pub reagents: Reagents,
    pub blueprints: Blueprints,
    pub illusions: IllusionCollection,
    /// A file whose own `mod_name` disagrees with its folder is
    /// opened all the same, but the disagreement is reported: the
    /// folder is context, the file's own word is the fact.
    pub warnings: Vec<String>,
}

/// Opens a campaign's transfer stash, component storage, blueprint
/// list, and illusion collection.
///
/// # Errors
/// [`GstOpenError`] for the stash; the other three files being
/// absent or untypeable is reported inside their [`Optional`]
/// instead.
///
/// [`Optional`]: crate::documents::Optional
pub fn open_shared(
    save: &SaveDir,
    campaign: &Campaign,
    progress: &mut dyn FnMut(LoadStep),
) -> Result<SharedDocs, GstOpenError> {
    progress(LoadStep::Stash);
    let stash = StashDoc::open(save.transfer_stash(campaign))?;
    progress(LoadStep::Reagents);
    let reagents = Reagents::open(save.reagent_storage(campaign));
    progress(LoadStep::Blueprints);
    let blueprints = Blueprints::open(save.blueprints(campaign));
    progress(LoadStep::Illusions);
    let illusions = IllusionCollection::open(save.illusions(campaign));
    let mut warnings = Vec::new();
    if !campaign.owns_file_naming(&stash.stash().mod_name) {
        warnings.push(misfiled(stash.path(), &stash.stash().mod_name, campaign));
    }
    if let Some(doc) = reagents.doc()
        && !campaign.owns_file_naming(&doc.storage().mod_name)
    {
        warnings.push(misfiled(doc.path(), &doc.storage().mod_name, campaign));
    }
    if let Some(doc) = illusions.doc()
        && !campaign.owns_file_naming(&doc.illusions().mod_name)
    {
        warnings.push(misfiled(doc.path(), &doc.illusions().mod_name, campaign));
    }
    Ok(SharedDocs {
        stash,
        reagents,
        blueprints,
        illusions,
        warnings,
    })
}

fn misfiled(path: &Path, mod_name: &str, campaign: &Campaign) -> String {
    let claims = if mod_name.is_empty() {
        "the main campaign".to_string()
    } else {
        format!("mod {mod_name:?}")
    };
    format!(
        "{} says it belongs to {claims}, not the {campaign}: showing it anyway",
        path.display()
    )
}

/// The campaign the game wrote most recently, by its stash's
/// modification time: the best witness of what is being played, since
/// neither a character file nor the game names a character's mod. The
/// first candidate (the main campaign) when nothing has a time or on a
/// tie.
fn newest_campaign(stamped: impl IntoIterator<Item = (Campaign, Option<SystemTime>)>) -> Campaign {
    stamped
        .into_iter()
        .fold(
            (Campaign::Main, None::<SystemTime>),
            |best, (campaign, time)| match (best.1, time) {
                (None, Some(_)) => (campaign, time),
                (Some(held), Some(seen)) if seen > held => (campaign, time),
                _ => best,
            },
        )
        .0
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
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
    let shipped_files = shipped_layers();
    let mod_files = mod_layers(list_mods(game_dir));
    let shipped = load_layers(game_dir, &shipped_files, progress)?;
    let mods = load_layers(game_dir, &mod_files, progress)?;
    progress(LoadStep::Localization);
    let counts = (
        shipped.databases.len(),
        shipped.text_archives.len(),
        shipped.item_archives.len(),
        mods.databases.len(),
    );
    let game = GameData::layered(shipped, mods).map_err(LoadFailure::Localization)?;
    let (symbols, mut warnings) = load_symbols(
        game_dir,
        mod_files.iter().chain(&shipped_files),
        &game,
        progress,
    );
    let report = LoadReport {
        databases: counts.0,
        text_archives: counts.1,
        item_archives: counts.2,
        mods: counts.3,
        symbols: symbols.found(),
        elapsed: started.elapsed(),
    };

    let campaigns = paths.save.campaigns();
    let campaign = newest_campaign(campaigns.iter().map(|campaign| {
        (
            campaign.clone(),
            modified(&paths.save.transfer_stash(campaign)),
        )
    }));
    let SharedDocs {
        stash,
        reagents,
        blueprints,
        illusions,
        warnings: shared_warnings,
    } = open_shared(&paths.save, &campaign, progress)?;
    warnings.extend(shared_warnings);
    progress(LoadStep::Store);
    let store = StoreDoc::open(paths.store.clone())?;
    progress(LoadStep::Characters);
    let characters = open_characters(&paths.save);
    Ok(LoadedWorld {
        game,
        report,
        symbols,
        campaigns,
        campaign,
        stash,
        reagents,
        blueprints,
        illusions,
        store,
        characters,
        warnings,
    })
}

/// The tile symbols `gameiteminfo.dbr` names, read by entry out of
/// each layer's `UI.arc` in overlay order (mods first, as fill layers)
/// so a later layer's copy wins. An archive that cannot be opened is a
/// warning, not a failure: the tiles fall back to drawn glyphs.
fn load_symbols<'l>(
    game_dir: &Path,
    layers: impl Iterator<Item = &'l LayerFiles>,
    game: &GameData,
    progress: &mut dyn FnMut(LoadStep),
) -> (SymbolTextures, Vec<String>) {
    let wanted = game.symbol_bitmaps();
    let mut textures = SymbolTextures::default();
    let mut warnings = Vec::new();
    for symbol in Symbol::ALL {
        let problem = if wanted.contains_key(&symbol) {
            IconProblem::NotInArchives
        } else {
            IconProblem::NoBitmap
        };
        textures.insert(symbol, Err(problem));
    }
    for layer in layers {
        let path = game_dir.join(&layer.ui);
        if !path.is_file() {
            continue;
        }
        progress(LoadStep::UiArchive(layer.ui.clone()));
        let index = match open_arc_index(&path) {
            Ok(index) => index,
            Err(failure) => {
                warnings.push(format!("tile symbols: {failure}"));
                continue;
            }
        };
        for (symbol, bitmap) in &wanted {
            if let Some(located) = index.locate(bitmap.archive_entry()) {
                let bytes = read_arc_entry(&path, &index, &located)
                    .map_err(|failure| IconProblem::Archive(failure.to_string()));
                textures.insert(*symbol, bytes);
            }
        }
    }
    (textures, warnings)
}

/// The directory of an archive, from its header and table region
/// alone.
fn open_arc_index(path: &Path) -> Result<ArcIndex, LoadFailure> {
    let archive = |source| LoadFailure::Archive {
        path: path.to_path_buf(),
        source,
    };
    let file_len = std::fs::metadata(path)
        .map_err(|source| LoadFailure::Read {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    let header = read_ranges(path, std::slice::from_ref(&(0..ArcHeader::LEN)))?;
    let header = ArcHeader::parse(&header[0]).map_err(archive)?;
    let tables = header
        .tables_range(usize::try_from(file_len).unwrap_or(usize::MAX))
        .map_err(archive)?;
    let tables = read_ranges(path, std::slice::from_ref(&tables))?;
    ArcIndex::parse(header, &tables[0], Codec::Lz4Block).map_err(archive)
}

/// One entry's bytes, read as the ranges the directory names.
fn read_arc_entry(
    path: &Path,
    index: &ArcIndex,
    located: &Located,
) -> Result<Vec<u8>, LoadFailure> {
    let parts = read_ranges(path, &located.ranges())?;
    let slices: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    index
        .assemble(located, &slices)
        .map_err(|source| LoadFailure::Archive {
            path: path.to_path_buf(),
            source,
        })
}

fn read_ranges(
    path: &Path,
    ranges: &[std::ops::Range<usize>],
) -> Result<Vec<Vec<u8>>, LoadFailure> {
    let ranges: Vec<std::ops::Range<u64>> = ranges
        .iter()
        .map(|range| {
            u64::try_from(range.start).unwrap_or(u64::MAX)
                ..u64::try_from(range.end).unwrap_or(u64::MAX)
        })
        .collect();
    univault_io::read_ranges(path, &ranges).map_err(|source| LoadFailure::Read {
        path: path.to_path_buf(),
        source,
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use grimvault_core::campaign::ModName;

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn the_default_campaign_is_the_one_written_most_recently() {
        let loot = Campaign::Mod(ModName::parse("LootAscension").unwrap());
        let zeta = Campaign::Mod(ModName::parse("Zeta").unwrap());
        assert_eq!(
            newest_campaign([
                (Campaign::Main, Some(at(100))),
                (loot.clone(), Some(at(300))),
                (zeta.clone(), Some(at(200))),
            ]),
            loot
        );
        assert_eq!(
            newest_campaign([
                (Campaign::Main, Some(at(300))),
                (loot.clone(), Some(at(300)))
            ]),
            Campaign::Main
        );
        assert_eq!(
            newest_campaign([(Campaign::Main, None), (zeta.clone(), Some(at(5)))]),
            zeta
        );
        assert_eq!(
            newest_campaign([(Campaign::Main, None), (loot, None)]),
            Campaign::Main
        );
    }
}
