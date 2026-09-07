//! The Loading phase: reading the layered game data exactly as
//! `grimvault-core`'s examples do (the shipped layers, missing
//! expansion files skipped, then every installed mod under `mods/` as
//! a fill layer), then opening the transfer stash, the component /
//! crafting-material storage, the vault store, and every character.
//! [`load_world`] is the whole path as one function, so the window
//! and the headless `--check` run the same code; [`start`] moves it
//! onto a thread and reports progress over a channel.
//!
//! The archives — over a gigabyte, mostly off a network mount — are
//! read and parsed [`READERS`] at a time and assembled in layer order,
//! so the composition ([`GameData::layered`]) sees exactly what a
//! one-at-a-time read would have: the same layers in the same order,
//! and the first failure in that order as the load's failure. The
//! steps are reported in that order too, each as the loader turns to
//! its archive, so the progress panel and the `--check` transcript
//! read the same as before.
//!
//! The tile symbols are the one thing read by entry rather than whole:
//! each layer's `UI.arc` runs to a quarter gigabyte, and the eleven
//! 4 KB textures the app wants are found through the archive's
//! directory ([`ArcIndex`]) and read as byte ranges
//! ([`univault_io::read_ranges`]).

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
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
    CharacterEntry, CharacterSlot, FileStamp, GstOpenError, Reagents, StashDoc, StoreDoc,
    StoreOpenError, open_characters,
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

/// One step of the load, reported as the loader turns to it: for the
/// archives, which are read ahead in parallel, that is when their
/// bytes are awaited, in layer order.
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
    /// Why `campaign` is the one opened.
    pub campaign_choice: CampaignChoice,
    pub stash: StashDoc,
    pub reagents: Reagents,
    pub blueprints: Blueprints,
    pub illusions: IllusionCollection,
    pub store: StoreDoc,
    pub characters: Vec<CharacterEntry>,
    /// The character the game wrote last — the one being played — so
    /// the picker opens on it; `None` only when there are none.
    pub newest_character: Option<CharacterSlot>,
    /// Cross-checks that failed without stopping the load, for the
    /// shell to show.
    pub warnings: Vec<String>,
}

/// Why the campaign opened first is the one it is: the user's last
/// selection when it is still there, else the newest stash — the best
/// witness of what is being played, since neither a character file nor
/// the game names a character's mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CampaignChoice {
    /// The campaign selected last time, still in the save directory.
    Remembered,
    /// Nothing was remembered.
    Newest,
    /// The remembered campaign has no folder in the save directory
    /// any more.
    Missing(Campaign),
}

impl fmt::Display for CampaignChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Remembered => f.write_str("selected last time"),
            Self::Newest => f.write_str("nothing remembered: its transfer.gst was written last"),
            Self::Missing(gone) => write!(
                f,
                "the remembered {gone} is not in the save directory: its transfer.gst was written last"
            ),
        }
    }
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

/// The campaign to open: `remembered` when the save directory still
/// holds it, else the one the game wrote most recently by its stash's
/// modification time (the main campaign when nothing has a time or on
/// a tie).
fn default_campaign(
    remembered: Option<&Campaign>,
    stamped: impl IntoIterator<Item = (Campaign, Option<SystemTime>)>,
) -> (Campaign, CampaignChoice) {
    let stamped: Vec<(Campaign, Option<SystemTime>)> = stamped.into_iter().collect();
    match remembered {
        Some(wanted) if stamped.iter().any(|(campaign, _)| campaign == wanted) => {
            (wanted.clone(), CampaignChoice::Remembered)
        }
        Some(wanted) => (
            newest(stamped).unwrap_or(Campaign::Main),
            CampaignChoice::Missing(wanted.clone()),
        ),
        None => (
            newest(stamped).unwrap_or(Campaign::Main),
            CampaignChoice::Newest,
        ),
    }
}

/// The candidate with the latest modification time; the first one on
/// a tie or when nothing has a time, `None` only when there are no
/// candidates.
fn newest<T>(stamped: impl IntoIterator<Item = (T, Option<SystemTime>)>) -> Option<T> {
    stamped
        .into_iter()
        .fold(None, |best, (candidate, time)| match best {
            None => Some((candidate, time)),
            Some((_, None)) if time.is_some() => Some((candidate, time)),
            Some((_, Some(held))) if time.is_some_and(|seen| seen > held) => {
                Some((candidate, time))
            }
            Some(kept) => Some(kept),
        })
        .map(|(candidate, _)| candidate)
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

/// The whole load, reporting each step to `progress` as it begins;
/// `remembered` is the campaign the user selected last, if any.
///
/// # Errors
/// [`LoadFailure`] for the first fatal step; a character that fails
/// to open, or a `reagents.gst` that is absent or cannot be typed, is
/// not fatal and is reported inside the world instead.
pub fn load_world(
    paths: &WorldPaths,
    remembered: Option<&Campaign>,
    progress: &mut dyn FnMut(LoadStep),
) -> Result<LoadedWorld, LoadFailure> {
    let started = Instant::now();
    let game_dir = paths.game.path();
    let shipped_files = shipped_layers();
    let mod_files = mod_layers(list_mods(game_dir));
    let plan = plan_reads(game_dir, &shipped_files, &mod_files);
    let (shipped, mods) = read_planned(game_dir, &plan, READERS, progress)?;
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
    let (campaign, campaign_choice) = default_campaign(
        remembered,
        campaigns.iter().map(|campaign| {
            (
                campaign.clone(),
                modified(&paths.save.transfer_stash(campaign)),
            )
        }),
    );
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
    let newest_character = newest(characters.iter().enumerate().map(|(slot, entry)| {
        (
            CharacterSlot::new(slot),
            entry.stamp().map(FileStamp::modified),
        )
    }));
    Ok(LoadedWorld {
        game,
        report,
        symbols,
        campaigns,
        campaign,
        campaign_choice,
        stash,
        reagents,
        blueprints,
        illusions,
        store,
        characters,
        newest_character,
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

/// How many archives are read at once: enough to keep parsing off
/// the critical path, and no more — the bytes themselves are the
/// bound (a network mount's link, or a local disk), and past four
/// readers neither measured any faster (the commit that set this
/// records the numbers).
const READERS: usize = 4;

/// Which whole-file archive of a layer a read is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerFile {
    Database,
    Text,
    Items,
}

impl LayerFile {
    const ALL: [Self; 3] = [Self::Database, Self::Text, Self::Items];

    fn relative(self, layer: &LayerFiles) -> &Path {
        match self {
            Self::Database => &layer.database,
            Self::Text => &layer.text,
            Self::Items => &layer.items,
        }
    }

    fn step(self, relative: PathBuf) -> LoadStep {
        match self {
            Self::Database => LoadStep::Database(relative),
            Self::Text => LoadStep::TextArchive(relative),
            Self::Items => LoadStep::ItemArchive(relative),
        }
    }
}

/// Whose [`LayerSet`] an archive fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Shipped,
    Mod,
}

/// An archive that is on disk and will be read.
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedRead {
    origin: Origin,
    file: LayerFile,
    relative: PathBuf,
}

impl PlannedRead {
    fn step(&self) -> LoadStep {
        self.file.step(self.relative.clone())
    }
}

/// Every archive of the shipped layers and then of the mod layers, in
/// layer order, skipping what is not on disk (an expansion or mod
/// resource that does not exist is not an error).
fn plan_reads(game_dir: &Path, shipped: &[LayerFiles], mods: &[LayerFiles]) -> Vec<PlannedRead> {
    let present = |origin: Origin, layers: &[LayerFiles]| -> Vec<PlannedRead> {
        layers
            .iter()
            .flat_map(|layer| {
                LayerFile::ALL.map(|file| PlannedRead {
                    origin,
                    file,
                    relative: file.relative(layer).to_path_buf(),
                })
            })
            .filter(|read| game_dir.join(&read.relative).is_file())
            .collect()
    };
    let mut plan = present(Origin::Shipped, shipped);
    plan.extend(present(Origin::Mod, mods));
    plan
}

/// An archive read and parsed on the thread that read it.
enum Parsed {
    Database(ArzFile),
    Text(ArcFile),
    Items(ArcFile),
}

/// Why an archive could not be read, without the path — the reader
/// works from the plan's index, and the assembler, which knows the
/// plan, names the file ([`ReadProblem::at`]).
#[derive(Debug)]
enum ReadProblem {
    Read(io::Error),
    Database(ArzError),
    Archive(ArcError),
    /// Every reader stopped without delivering this archive's result.
    NoResult,
}

impl ReadProblem {
    fn at(self, path: PathBuf) -> LoadFailure {
        match self {
            Self::Read(source) => LoadFailure::Read { path, source },
            Self::Database(source) => LoadFailure::Database { path, source },
            Self::Archive(source) => LoadFailure::Archive { path, source },
            Self::NoResult => LoadFailure::Read {
                path,
                source: io::Error::other("no reader delivered it"),
            },
        }
    }
}

/// One archive's bytes and parse. Archives are read-only reference
/// data on a possibly stale mount: the verified read turns a short
/// read into an error instead of a corrupt parse.
fn fetch(game_dir: &Path, read: &PlannedRead) -> Result<Parsed, ReadProblem> {
    let bytes =
        univault_io::read_verified(&game_dir.join(&read.relative)).map_err(ReadProblem::Read)?;
    let archive = |bytes| ArcFile::parse(bytes, Codec::Lz4Block).map_err(ReadProblem::Archive);
    match read.file {
        LayerFile::Database => ArzFile::parse(bytes, ArzDialect::grim_dawn())
            .map(Parsed::Database)
            .map_err(ReadProblem::Database),
        LayerFile::Text => archive(bytes).map(Parsed::Text),
        LayerFile::Items => archive(bytes).map(Parsed::Items),
    }
}

/// A reader's result for the archive at that index of the plan.
type Arrival = (usize, Result<Parsed, ReadProblem>);

/// Reads and parses the planned archives, `readers` at a time in plan
/// order, and assembles the shipped and the mod sets from them.
fn read_planned(
    game_dir: &Path,
    plan: &[PlannedRead],
    readers: usize,
    progress: &mut dyn FnMut(LoadStep),
) -> Result<(LayerSet, LayerSet), LoadFailure> {
    let next = AtomicUsize::new(0);
    let (sender, arrivals) = channel::<Arrival>();
    thread::scope(|scope| {
        for _ in 0..readers.min(plan.len()) {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(read) = plan.get(index) else { break };
                    if sender.send((index, fetch(game_dir, read))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
        assemble(game_dir, plan, arrivals, progress)
    })
}

/// Files the arrivals into their sets in plan order, whatever order
/// they came in: each step is reported as its archive is taken up, so
/// progress reads as a one-at-a-time load's would, and the first
/// failure in plan order is the load's — later archives' outcomes are
/// never looked at.
fn assemble(
    game_dir: &Path,
    plan: &[PlannedRead],
    arrivals: impl IntoIterator<Item = Arrival>,
    progress: &mut dyn FnMut(LoadStep),
) -> Result<(LayerSet, LayerSet), LoadFailure> {
    let mut arrivals = arrivals.into_iter();
    let mut early = BTreeMap::new();
    let mut shipped = LayerSet::default();
    let mut mods = LayerSet::default();
    for (index, read) in plan.iter().enumerate() {
        progress(read.step());
        let parsed = take(index, &mut early, &mut arrivals)
            .unwrap_or(Err(ReadProblem::NoResult))
            .map_err(|problem| problem.at(game_dir.join(&read.relative)))?;
        let set = match read.origin {
            Origin::Shipped => &mut shipped,
            Origin::Mod => &mut mods,
        };
        match parsed {
            Parsed::Database(database) => set.databases.push(database),
            Parsed::Text(archive) => set.text_archives.push(archive),
            Parsed::Items(archive) => set.item_archives.push(archive),
        }
    }
    Ok((shipped, mods))
}

/// The result for `wanted`, holding any other archive's result that
/// arrives first for its own turn; `None` once the arrivals end
/// without it.
fn take(
    wanted: usize,
    early: &mut BTreeMap<usize, Result<Parsed, ReadProblem>>,
    arrivals: &mut impl Iterator<Item = Arrival>,
) -> Option<Result<Parsed, ReadProblem>> {
    loop {
        if let Some(result) = early.remove(&wanted) {
            return Some(result);
        }
        let (index, result) = arrivals.next()?;
        early.insert(index, result);
    }
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

/// What the loader thread sends.
pub enum LoadEvent {
    Step(LoadStep),
    Done(Box<LoadedWorld>),
    Failed(LoadFailure),
}

/// A load in flight: the steps reported so far and the channel the
/// outcome arrives on. `remembered` rides along so a load that fails
/// hands the selection back to setup instead of forgetting it.
pub struct LoadJob {
    pub paths: WorldPaths,
    pub remembered: Option<Campaign>,
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
pub fn start(paths: WorldPaths, remembered: Option<Campaign>, wake: egui::Context) -> LoadJob {
    let (sender, events) = channel::<LoadEvent>();
    let job_paths = paths.clone();
    let job_remembered = remembered.clone();
    let spawned = std::thread::Builder::new()
        .name("grimvault-load".into())
        .spawn(move || run(&job_paths, job_remembered.as_ref(), &sender, &wake));
    if let Err(error) = spawned {
        let (fallback, events) = channel::<LoadEvent>();
        let _ = fallback.send(LoadEvent::Failed(LoadFailure::Read {
            path: paths.game.path().to_path_buf(),
            source: error,
        }));
        return LoadJob {
            paths,
            remembered,
            steps: Vec::new(),
            events,
        };
    }
    LoadJob {
        paths,
        remembered,
        steps: Vec::new(),
        events,
    }
}

fn run(
    paths: &WorldPaths,
    remembered: Option<&Campaign>,
    sender: &Sender<LoadEvent>,
    wake: &egui::Context,
) {
    let mut report = |step: LoadStep| {
        let _ = sender.send(LoadEvent::Step(step));
        wake.request_repaint();
    };
    let outcome = match load_world(paths, remembered, &mut report) {
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
    use univault_engine::arz::fixture::ArzBuilder;

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("grimvault-loader-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, relative: &Path, bytes: &[u8]) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn database_with(record: &str) -> Vec<u8> {
        let mut builder = ArzBuilder::new(ArzDialect::grim_dawn());
        builder.record(record, "ItemRelic", &[]);
        builder.build()
    }

    fn parsed_database(record: &str) -> Parsed {
        Parsed::Database(ArzFile::parse(database_with(record), ArzDialect::grim_dawn()).unwrap())
    }

    fn records(set: &LayerSet) -> Vec<String> {
        set.databases
            .iter()
            .flat_map(|database| database.record_ids().map(ToString::to_string))
            .collect()
    }

    fn database_read(origin: Origin, relative: &str) -> PlannedRead {
        PlannedRead {
            origin,
            file: LayerFile::Database,
            relative: PathBuf::from(relative),
        }
    }

    fn steps_of(plan: &[PlannedRead]) -> Vec<LoadStep> {
        plan.iter().map(PlannedRead::step).collect()
    }

    #[test]
    fn plan_reads_lists_what_is_on_disk_shipped_first_in_layer_order() {
        let scratch = Scratch::new("plan");
        let shipped = shipped_layers();
        let mods = mod_layers([ModListing {
            folder: "m".into(),
            database_files: vec!["m.arz".into()],
            resource_files: vec!["Items.arc".into()],
        }]);
        for relative in [
            &shipped[0].database,
            &shipped[0].text,
            &shipped[1].database,
            &mods[0].database,
            &mods[0].items,
        ] {
            scratch.write(relative, b"");
        }

        let plan = plan_reads(&scratch.0, &shipped, &mods);

        let listed: Vec<(Origin, LayerFile, &Path)> = plan
            .iter()
            .map(|read| (read.origin, read.file, read.relative.as_path()))
            .collect();
        assert_eq!(
            listed,
            vec![
                (
                    Origin::Shipped,
                    LayerFile::Database,
                    shipped[0].database.as_path()
                ),
                (Origin::Shipped, LayerFile::Text, shipped[0].text.as_path()),
                (
                    Origin::Shipped,
                    LayerFile::Database,
                    shipped[1].database.as_path()
                ),
                (Origin::Mod, LayerFile::Database, mods[0].database.as_path()),
                (Origin::Mod, LayerFile::Items, mods[0].items.as_path()),
            ]
        );
    }

    #[test]
    fn assemble_files_arrivals_in_plan_order_whatever_order_they_came() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
            database_read(Origin::Mod, "c.arz"),
        ];
        let arrivals = [
            (2, Ok(parsed_database("records/c.dbr"))),
            (0, Ok(parsed_database("records/a.dbr"))),
            (1, Ok(parsed_database("records/b.dbr"))),
        ];
        let mut reported = Vec::new();

        let (shipped, mods) = assemble(Path::new("/game"), &plan, arrivals, &mut |step| {
            reported.push(step);
        })
        .unwrap();

        assert_eq!(reported, steps_of(&plan));
        assert_eq!(records(&shipped), ["records/a.dbr", "records/b.dbr"]);
        assert_eq!(records(&mods), ["records/c.dbr"]);
    }

    #[test]
    fn the_first_failure_in_plan_order_is_the_loads_whichever_arrived_first() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
            database_read(Origin::Shipped, "c.arz"),
        ];
        let arrivals = [
            (2, Err(ReadProblem::Archive(ArcError::NotArc))),
            (0, Ok(parsed_database("records/a.dbr"))),
            (
                1,
                Err(ReadProblem::Database(ArzError::DialectMismatch {
                    found: 1,
                    expected: 2,
                })),
            ),
        ];
        let mut reported = Vec::new();

        let failure = assemble(Path::new("/game"), &plan, arrivals, &mut |step| {
            reported.push(step);
        })
        .err()
        .unwrap();

        assert!(
            matches!(&failure, LoadFailure::Database { path, .. } if path == Path::new("/game/b.arz")),
            "{failure}"
        );
        assert_eq!(reported, steps_of(&plan[..2]));
    }

    #[test]
    fn an_archive_no_reader_delivered_is_a_failure_not_a_wait() {
        let plan = [
            database_read(Origin::Shipped, "a.arz"),
            database_read(Origin::Shipped, "b.arz"),
        ];
        let arrivals = [(0, Ok(parsed_database("records/a.dbr")))];

        let failure = assemble(Path::new("/game"), &plan, arrivals, &mut |_| {})
            .err()
            .unwrap();

        assert!(
            matches!(&failure, LoadFailure::Read { path, .. } if path == Path::new("/game/b.arz")),
            "{failure}"
        );
    }

    #[test]
    fn read_planned_reads_on_threads_and_keeps_layer_order() {
        let scratch = Scratch::new("threads");
        let shipped = shipped_layers();
        scratch.write(&shipped[0].database, &database_with("records/base.dbr"));
        scratch.write(&shipped[1].database, &database_with("records/x1.dbr"));
        scratch.write(&shipped[2].database, &database_with("records/x2.dbr"));
        let plan = plan_reads(&scratch.0, &shipped, &[]);
        let mut reported = Vec::new();

        let (loaded, mods) =
            read_planned(&scratch.0, &plan, 2, &mut |step| reported.push(step)).unwrap();

        assert_eq!(reported, steps_of(&plan));
        assert_eq!(
            records(&loaded),
            ["records/base.dbr", "records/x1.dbr", "records/x2.dbr"]
        );
        assert!(mods.databases.is_empty());
    }

    #[test]
    fn read_planned_fails_on_the_first_bad_archive_in_layer_order() {
        let scratch = Scratch::new("bad");
        let shipped = shipped_layers();
        scratch.write(&shipped[0].database, &database_with("records/base.dbr"));
        scratch.write(&shipped[1].database, b"not a database");
        scratch.write(&shipped[2].database, b"");
        let plan = plan_reads(&scratch.0, &shipped, &[]);
        let mut reported = Vec::new();

        let failure = read_planned(&scratch.0, &plan, 3, &mut |step| reported.push(step))
            .err()
            .unwrap();

        assert!(
            matches!(&failure, LoadFailure::Database { path, .. } if *path == scratch.0.join(&shipped[1].database)),
            "{failure}"
        );
        assert_eq!(reported, steps_of(&plan[..2]));
    }

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn loot() -> Campaign {
        Campaign::Mod(ModName::parse("LootAscension").unwrap())
    }

    fn zeta() -> Campaign {
        Campaign::Mod(ModName::parse("Zeta").unwrap())
    }

    #[test]
    fn with_nothing_remembered_the_campaign_written_most_recently_opens() {
        assert_eq!(
            default_campaign(
                None,
                [
                    (Campaign::Main, Some(at(100))),
                    (loot(), Some(at(300))),
                    (zeta(), Some(at(200))),
                ]
            ),
            (loot(), CampaignChoice::Newest)
        );
        assert_eq!(
            default_campaign(
                None,
                [(Campaign::Main, Some(at(300))), (loot(), Some(at(300)))]
            ),
            (Campaign::Main, CampaignChoice::Newest)
        );
        assert_eq!(
            default_campaign(None, [(Campaign::Main, None), (zeta(), Some(at(5)))]),
            (zeta(), CampaignChoice::Newest)
        );
        assert_eq!(
            default_campaign(None, [(Campaign::Main, None), (loot(), None)]),
            (Campaign::Main, CampaignChoice::Newest)
        );
        assert_eq!(
            default_campaign(None, []),
            (Campaign::Main, CampaignChoice::Newest)
        );
    }

    #[test]
    fn the_remembered_campaign_opens_while_it_exists_and_the_newest_when_it_is_gone() {
        let stamped = [
            (Campaign::Main, Some(at(100))),
            (loot(), Some(at(300))),
            (zeta(), Some(at(200))),
        ];
        assert_eq!(
            default_campaign(Some(&zeta()), stamped.clone()),
            (zeta(), CampaignChoice::Remembered)
        );
        assert_eq!(
            default_campaign(Some(&Campaign::Main), stamped.clone()),
            (Campaign::Main, CampaignChoice::Remembered)
        );
        let gone = Campaign::Mod(ModName::parse("Uninstalled").unwrap());
        assert_eq!(
            default_campaign(Some(&gone), stamped),
            (loot(), CampaignChoice::Missing(gone))
        );
    }

    #[test]
    fn the_newest_character_is_the_one_written_last_and_the_first_on_a_tie() {
        let slot = CharacterSlot::new;
        assert_eq!(
            newest([
                (slot(0), Some(at(100))),
                (slot(1), Some(at(900))),
                (slot(2), Some(at(500))),
            ]),
            Some(slot(1))
        );
        assert_eq!(
            newest([(slot(0), Some(at(900))), (slot(1), Some(at(900)))]),
            Some(slot(0))
        );
        assert_eq!(
            newest([(slot(0), None), (slot(1), None), (slot(2), Some(at(1)))]),
            Some(slot(2))
        );
        assert_eq!(newest([(slot(0), None), (slot(1), None)]), Some(slot(0)));
        assert_eq!(newest::<CharacterSlot>([]), None);
    }
}
