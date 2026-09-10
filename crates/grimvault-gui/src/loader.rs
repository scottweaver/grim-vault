//! The Loading phase: reading the layered game data through
//! `grimvault_io` (the shipped layers, missing expansion files
//! skipped, then every installed mod under `mods/` as a fill layer),
//! then opening the transfer stash, the component / crafting-material
//! storage, the vault store, and every character. [`load_world`] is
//! the whole path as one function, so the window and the headless
//! `--check` run the same code; [`start`] moves it onto a thread and
//! reports progress over a channel.
//!
//! The tile symbols and the item icons outside `Items.arc` are the
//! things read by entry rather than whole: each layer's `UI.arc` runs
//! to a quarter gigabyte, and the eleven 4 KB textures the app wants
//! are found through the archive's directory and read as byte ranges
//! ([`grimvault_io::layers::read_arc_entry`]).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime};

use grimvault_core::campaign::Campaign;
use grimvault_core::facets::Symbol;
use grimvault_core::gamedata::{
    ArchiveName, BitmapPath, GameData, LayerFiles, mod_layers, shipped_layers,
};
use grimvault_core::reference::AffixTable;
use grimvault_core::settings::Settings;
use grimvault_io::layers::{
    ArchiveFailure, ArchiveRead, LayerFile, find_archive, list_mods, open_arc_index,
    read_arc_entry, read_layers,
};
use thiserror::Error;
use univault_engine::arc::ArcError;

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
    IconArchive(PathBuf),
    Reference,
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
            Self::IconArchive(relative) => {
                write!(f, "reading item icons from {}", relative.display())
            }
            Self::Reference => f.write_str("building the affix reference"),
            Self::Stash => f.write_str("opening transfer.gst"),
            Self::Reagents => f.write_str("opening reagents.gst"),
            Self::Blueprints => f.write_str("opening formulas.gst"),
            Self::Illusions => f.write_str("opening transmutes.gst"),
            Self::Store => f.write_str("opening the vault store"),
            Self::Characters => f.write_str("opening characters"),
        }
    }
}

impl From<ArchiveRead> for LoadStep {
    fn from(read: ArchiveRead) -> Self {
        match read.file {
            LayerFile::Database => Self::Database(read.relative),
            LayerFile::Text => Self::TextArchive(read.relative),
            LayerFile::Items => Self::ItemArchive(read.relative),
        }
    }
}

/// What the game-data half of the load found: the shipped layers by
/// kind, how many mods contributed a database, how many of the tile
/// symbols have their texture, and how many item icons outside
/// `Items.arc` were found of those the records name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadReport {
    pub databases: usize,
    pub text_archives: usize,
    pub item_archives: usize,
    pub mods: usize,
    pub symbols: usize,
    pub foreign_icons: Found,
    pub elapsed: Duration,
}

/// How many of a wanted set turned up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub found: usize,
    pub wanted: usize,
}

impl fmt::Display for Found {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} of {}", self.found, self.wanted)
    }
}

/// Everything the Ready phase needs.
pub struct LoadedWorld {
    pub game: GameData,
    pub report: LoadReport,
    pub affixes: AffixTable,
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
    #[error(transparent)]
    Archive(#[from] ArchiveFailure),
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
    let (shipped, mods) = read_layers(
        game_dir,
        &shipped_files,
        &mod_files,
        &LayerFile::ALL,
        &mut |read| progress(read.into()),
    )?;
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
    let (game, foreign_icons, icon_warnings) = load_foreign_bitmaps(
        game_dir,
        mod_files.iter().chain(&shipped_files),
        game,
        progress,
    );
    warnings.extend(icon_warnings);
    progress(LoadStep::Reference);
    let affixes = AffixTable::build(&game);
    let report = LoadReport {
        databases: counts.0,
        text_archives: counts.1,
        item_archives: counts.2,
        mods: counts.3,
        symbols: symbols.found(),
        foreign_icons,
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
        affixes,
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

/// The item icons whose records name an archive other than
/// `Items.arc` ([`GameData::foreign_bitmaps`]: Lokarr's set in
/// `Level Art.arc`, the potion formulas in `UI.arc`), read by entry
/// out of that archive in each layer that has it — mods first as
/// fill layers, so a later layer's copy wins — and handed to the game
/// data. An archive that cannot be opened is a warning; an icon no
/// layer holds stays unknown, as it was.
fn load_foreign_bitmaps<'l>(
    game_dir: &Path,
    layers: impl Iterator<Item = &'l LayerFiles>,
    game: GameData,
    progress: &mut dyn FnMut(LoadStep),
) -> (GameData, Found, Vec<String>) {
    let wanted = game.foreign_bitmaps();
    let mut by_archive: HashMap<ArchiveName, Vec<(BitmapPath, String)>> = HashMap::new();
    for bitmap in wanted {
        if let Some((archive, entry)) = bitmap.archive() {
            let entry = entry.to_string();
            by_archive.entry(archive).or_default().push((bitmap, entry));
        }
    }
    let mut archives: Vec<(ArchiveName, Vec<(BitmapPath, String)>)> =
        by_archive.into_iter().collect();
    archives.sort_by(|(a, _), (b, _)| a.as_str().cmp(b.as_str()));
    let wanted = archives.iter().map(|(_, bitmaps)| bitmaps.len()).sum();
    let mut found: HashMap<BitmapPath, Vec<u8>> = HashMap::new();
    let mut warnings = Vec::new();
    for layer in layers {
        for (archive, bitmaps) in &archives {
            let Some(path) = find_archive(game_dir, &layer.resources, archive) else {
                continue;
            };
            progress(LoadStep::IconArchive(path.clone()));
            let index = match open_arc_index(&game_dir.join(&path)) {
                Ok(index) => index,
                Err(failure) => {
                    warnings.push(format!("item icons: {failure}"));
                    continue;
                }
            };
            for (bitmap, entry) in bitmaps {
                if let Some(located) = index.locate(entry) {
                    match read_arc_entry(&game_dir.join(&path), &index, &located) {
                        Ok(bytes) => {
                            found.insert(bitmap.clone(), bytes);
                        }
                        Err(failure) => warnings.push(format!("item icons: {failure}")),
                    }
                }
            }
        }
    }
    let count = Found {
        found: found.len(),
        wanted,
    };
    (game.with_bitmaps(found), count, warnings)
}

/// The file of `archive` in a layer's resources folder, relative to
/// the game directory, matched case-insensitively so a Linux mount
/// finds `Level Art.arc` however the install spells it.
/// What the loader thread sends.
pub enum LoadEvent {
    Step(LoadStep),
    Done(Box<LoadedWorld>),
    Failed(LoadFailure),
}

/// A load in flight: the settings it was started from (whose
/// remembered campaign it opens, and which ride along so a load that
/// fails hands them back to setup), the steps reported so far, and
/// the channel the outcome arrives on.
pub struct LoadJob {
    pub paths: WorldPaths,
    pub settings: Settings,
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
pub fn start(paths: WorldPaths, settings: Settings, wake: egui::Context) -> LoadJob {
    let (sender, events) = channel::<LoadEvent>();
    let job_paths = paths.clone();
    let job_remembered = settings.campaign.clone();
    let spawned = std::thread::Builder::new()
        .name("grimvault-load".into())
        .spawn(move || run(&job_paths, job_remembered.as_ref(), &sender, &wake));
    if let Err(error) = spawned {
        let (fallback, events) = channel::<LoadEvent>();
        let _ = fallback.send(LoadEvent::Failed(LoadFailure::Archive(
            ArchiveFailure::Read {
                path: paths.game.path().to_path_buf(),
                source: error,
            },
        )));
        return LoadJob {
            paths,
            settings,
            steps: Vec::new(),
            events,
        };
    }
    LoadJob {
        paths,
        settings,
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

    use super::*;

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
