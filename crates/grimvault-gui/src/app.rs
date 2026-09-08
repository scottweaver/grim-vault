//! The shell: the phases, the world the Ready phase holds, and the two
//! drivers — autosave and the external-change guard — that run from
//! eframe's `logic` hook. That hook runs even while the window is
//! hidden, so a covered window still saves and still notices the
//! game's writes (a stall tq-univault paid for in its paint-driven
//! refresh).

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Rect, RichText, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use grimvault_core::bulk::{self, Identities};
use grimvault_core::gamedata::GameData;
use grimvault_core::gdc::{PlayerFile, Realm};
use grimvault_core::gds;
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::reference::AffixTable;
use grimvault_core::respec::{Reset, RespecRules};
use grimvault_core::settings::{BlueprintSync, BulkDuplicates, ReagentSync, StandingOrder};
use grimvault_core::store::{Timestamp, VaultStore};
use grimvault_core::transfer::TransferError;
use univault_engine::ids::RecordId;
use univault_io::{read_verified, write_synced};
use univault_ui::theme::{Palette, Theme};

use crate::automove::{self, AutoMoveTarget, OrderRequest};
use crate::autosave::{Activity, Autosave, AutosaveState, Gate, Pending, Verdict};
use crate::crafting::{self, Blueprints, CraftingFiles, FormulasOpenError, IllusionCollection};
use crate::documents::{
    Backup, CharacterDoc, CharacterEntry, CharacterOpenError, CharacterSlot, Doc, Document, Edits,
    FileStamp, GstOpenError, Optional, ReagentDoc, Reagents, SaveError, SaveOutcome, StashDoc,
    StoreDoc, StoreOpenError, Writable,
};
use crate::drag::{
    self, Applied, ApplyError, Container, Containers, DragSource, DragState, DropTarget, Fit,
    Landing, LastActive, Mode, Move, OpenCharacter, Views,
};
use crate::facts::FactsCache;
use crate::grid::CELL_PX;
use crate::icons::{Icon, IconCache};
use crate::loader::{
    self, CampaignChoice, LoadFailure, LoadJob, LoadOutcome, LoadReport, LoadedWorld, WorldPaths,
    open_shared,
};
use crate::panes::character::CharacterView;
use crate::panes::inspector::{self, Selected, Supplies};
use crate::panes::stash::StashView;
use crate::panes::store::{SEARCH_SHORTCUT, StoreMode, StoreView};
use crate::panes::{self, BulkOp, BulkRequest, DragFrame, PaneCtx};
use crate::reference::{self, ReferenceCache, ReferenceView};
use crate::search::SearchCache;
use crate::settings::{self, ConfigDir, Settings};
use crate::settings_dialog::{
    Action as SettingsAction, Change, SettingsDialog, dir_field, verdict_line,
};
use crate::setup::{DirProblem, GameDir, SaveDir, SetupState};
use crate::sockets;
use crate::theme::FITS;
use crate::ui_state::{PersistedUiState, UiState};
use crate::watch::{Observation, RefreshTracker, Watcher};
use grimvault_core::campaign::Campaign;

/// Where the app is.
pub enum Phase {
    Setup(SetupState),
    Loading(LoadJob),
    Failed(LoadFailed),
    Ready(Box<World>),
}

/// A load that stopped, with the settings it used so setup can start
/// from them.
pub struct LoadFailed {
    failure: LoadFailure,
    settings: Settings,
}

/// The application.
pub struct App {
    config: ConfigDir,
    theme: Theme,
    phase: Phase,
    toasts: Toasts,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, config: ConfigDir) -> Self {
        let theme = crate::theme::theme();
        theme.apply(&cc.egui_ctx);
        cc.egui_ctx
            .all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
        let phase = initial_phase(&config, &cc.egui_ctx);
        Self {
            config,
            theme,
            phase,
            toasts: Toasts::default(),
        }
    }
}

/// Straight to loading when the saved settings still validate; setup
/// otherwise, saying why.
fn initial_phase(config: &ConfigDir, ctx: &egui::Context) -> Phase {
    match settings::load(config) {
        Ok(Some(saved)) => match world_paths(&saved, config) {
            Ok(paths) => Phase::Loading(loader::start(paths, saved, ctx.clone())),
            Err(problem) => Phase::Setup(SetupState::discover(
                Some(&saved),
                Some(problem.to_string()),
            )),
        },
        Ok(None) => Phase::Setup(SetupState::discover(None, None)),
        Err(error) => Phase::Setup(SetupState::discover(None, Some(error.to_string()))),
    }
}

fn world_paths(saved: &Settings, config: &ConfigDir) -> Result<WorldPaths, DirProblem> {
    Ok(WorldPaths {
        game: GameDir::parse(&saved.game_dir)?,
        save: SaveDir::parse(&saved.save_dir)?,
        store: saved.store_file(config),
        ui_state: config.ui_state_file(),
    })
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Phase::Ready(world) = &mut self.phase {
            world.drive_refresh(ctx, &mut self.toasts);
            world.drive_autosave(ctx, &mut self.toasts);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let next = match &mut self.phase {
            Phase::Setup(state) => {
                show_setup(ui, state, &self.config, &self.theme, &mut self.toasts)
            }
            Phase::Loading(job) => show_loading(ui, job, &self.theme, &mut self.toasts),
            Phase::Failed(failed) => show_failed(ui, failed, &self.theme),
            Phase::Ready(world) => world.show(ui, &self.theme, &self.config, &mut self.toasts),
        };
        if let Some(next) = next {
            self.phase = next;
        }
        self.toasts.show(ui.ctx(), &self.theme.palette);
    }

    fn on_exit(&mut self) {
        if let Phase::Ready(world) = &mut self.phase {
            world.flush_on_exit();
        }
    }
}

fn show_setup(
    ui: &mut Ui,
    state: &mut SetupState,
    config: &ConfigDir,
    theme: &Theme,
    toasts: &mut Toasts,
) -> Option<Phase> {
    let mut next = None;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.label(theme.heading("Grim Vault"));
        ui.label("Tell Grim Vault where Grim Dawn is installed and where its saves live.");
        if let Some(note) = &state.note {
            ui.colored_label(theme.palette.warn, note);
        }
        ui.add_space(8.0);
        dir_field(
            ui,
            "Game directory",
            &mut state.game_field,
            &state.game_candidates,
            theme,
        );
        let game = state.game_dir();
        verdict_line(
            ui,
            game.as_ref().err(),
            "database/database.arz found",
            theme,
        );
        ui.add_space(8.0);
        dir_field(
            ui,
            "Save directory",
            &mut state.save_field,
            &state.save_candidates,
            theme,
        );
        let save = state.save_dir();
        verdict_line(
            ui,
            save.as_ref().err(),
            "transfer.gst and main/ found",
            theme,
        );
        ui.add_space(12.0);
        let ready = game.is_ok() && save.is_ok();
        if ui.add_enabled(ready, egui::Button::new("Load")).clicked()
            && let (Ok(game), Ok(save)) = (game, save)
        {
            let settings = state.settings();
            if let Err(error) = settings::save(config, &settings) {
                toasts.error(format!("settings were not saved: {error}"));
            }
            let paths = WorldPaths {
                game,
                save,
                store: settings.store_file(config),
                ui_state: config.ui_state_file(),
            };
            next = Some(Phase::Loading(loader::start(
                paths,
                settings,
                ui.ctx().clone(),
            )));
        }
        ui.add_space(8.0);
        ui.label(theme.path_text(format!(
            "settings live in {}; the vault store beside them unless Settings (⚙) points elsewhere",
            config.path().display()
        )));
    });
    next
}

fn show_loading(
    ui: &mut Ui,
    job: &mut LoadJob,
    theme: &Theme,
    toasts: &mut Toasts,
) -> Option<Phase> {
    match job.poll() {
        Some(LoadOutcome::Done(world)) => {
            return Some(Phase::Ready(Box::new(World::new(
                *world,
                job.paths.clone(),
                job.settings.clone(),
                ui.ctx(),
                toasts,
            ))));
        }
        Some(LoadOutcome::Failed(failure)) => {
            return Some(Phase::Failed(LoadFailed {
                failure,
                settings: job.settings.clone(),
            }));
        }
        None => {}
    }
    let mut back = false;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.label(theme.heading("Loading"));
        ui.label(theme.path_text(job.paths.game.path().display().to_string()));
        ui.add_space(6.0);
        let count = job.steps.len();
        for (slot, step) in job.steps.iter().enumerate() {
            ui.horizontal(|ui| {
                if slot + 1 == count {
                    ui.spinner();
                } else {
                    ui.colored_label(FITS, "✔");
                }
                ui.label(step.to_string());
            });
        }
        if count == 0 {
            ui.spinner();
        }
        ui.add_space(8.0);
        back = ui.button("Back to setup").clicked();
    });
    back.then(|| Phase::Setup(SetupState::discover(Some(&job.settings), None)))
}

fn show_failed(ui: &mut Ui, failed: &LoadFailed, theme: &Theme) -> Option<Phase> {
    let mut back = false;
    egui::CentralPanel::default().show(ui, |ui| {
        ui.label(theme.heading("Could not load"));
        ui.colored_label(theme.palette.error, failed.failure.to_string());
        ui.add_space(8.0);
        back = ui.button("Back to setup").clicked();
    });
    back.then(|| {
        Phase::Setup(SetupState::discover(
            Some(&failed.settings),
            Some(failed.failure.to_string()),
        ))
    })
}

/// The order the dirty documents are written in: a move's destination
/// goes first, so a crash between two writes leaves the item
/// duplicated rather than lost. Each move promotes its destination to
/// the front, so the most recent move's rule always holds and earlier
/// moves' rules hold whenever they still can.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WriteOrder(Vec<Doc>);

impl WriteOrder {
    /// The store first, then the game files in load order.
    fn new(docs: impl IntoIterator<Item = Doc>) -> Self {
        Self(docs.into_iter().collect())
    }

    fn prioritize(&mut self, destination: Doc) {
        let Some(slot) = self.0.iter().position(|doc| *doc == destination) else {
            return;
        };
        self.0[..=slot].rotate_right(1);
    }

    fn docs(&self) -> Vec<Doc> {
        self.0.clone()
    }
}

/// Why a reload from disk failed.
#[derive(Debug, thiserror::Error)]
enum ReloadError {
    #[error("{0}")]
    Gst(#[from] GstOpenError),
    #[error("{0}")]
    Formulas(#[from] FormulasOpenError),
    #[error("{0}")]
    Store(#[from] StoreOpenError),
    #[error("{0}")]
    Character(#[from] CharacterOpenError),
}

/// The moment of an edit, for the store and the export documents;
/// the core has no clock.
fn now() -> Timestamp {
    Timestamp::from_unix_seconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
    )
}

/// A seed for a socket or a freed part. The game's seeds are
/// arbitrary 32-bit values and nothing this app shows depends on the
/// roll, so the clock mixed with a counter — two edits in one instant
/// still differ — is entropy enough.
fn fresh_seed() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let low_seconds = u32::try_from(elapsed.as_secs() & u64::from(u32::MAX)).unwrap_or(0);
    let tick = COUNTER.fetch_add(1, Ordering::Relaxed);
    (elapsed.subsec_nanos() ^ low_seconds.rotate_left(16) ^ tick.wrapping_mul(0x9E37_79B9))
        .wrapping_mul(2_654_435_761)
}

/// The documents the campaign selector swaps out.
const CAMPAIGN_DOCS: [Doc; 4] = [Doc::Stash, Doc::Reagents, Doc::Blueprints, Doc::Illusions];

/// The non-character documents in default write order: the store
/// first, then the campaign's files in load order.
const SHARED_DOCS: [Doc; 5] = [
    Doc::Store,
    Doc::Stash,
    Doc::Reagents,
    Doc::Blueprints,
    Doc::Illusions,
];

/// Whether the modifier keys ask for a copy: Alt, or the platform's
/// command key (Ctrl, ⌘ on macOS).
fn mode_of(modifiers: egui::Modifiers) -> Mode {
    if modifiers.alt || modifiers.command {
        Mode::Copy
    } else {
        Mode::Move
    }
}

/// Whether a right-click copies: Shift, the gesture's own modifier,
/// or either key that copies on a drop.
fn right_click_mode(modifiers: egui::Modifiers) -> Mode {
    if modifiers.shift {
        Mode::Copy
    } else {
        mode_of(modifiers)
    }
}

/// What a bulk operation did, for the toast and the dirty marks.
enum BulkDone {
    Transferred(bulk::BulkSummary),
    Cleared(bulk::ClearSummary),
}

/// A character's file for editing, with its realm — a free function
/// so the store and the memo can be borrowed beside it.
fn editable_character(
    characters: &mut [CharacterEntry],
    slot: CharacterSlot,
) -> Result<(Realm, &mut PlayerFile), ApplyError> {
    let doc = characters
        .get_mut(slot.value())
        .and_then(CharacterEntry::doc_mut)
        .ok_or(ApplyError::CharacterNotEditable(slot))?;
    let realm = doc.realm();
    doc.file_mut()
        .map(|file| (realm, file))
        .map_err(|_| ApplyError::CharacterNotEditable(slot))
}

/// Which standing orders a (re)load carries out: every one, the
/// open campaign's shared files', or one document's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Everything,
    Campaign,
    Doc(Doc),
}

impl Scope {
    fn covers(self, doc: Doc) -> bool {
        match self {
            Self::Everything => true,
            Self::Campaign => CAMPAIGN_DOCS.contains(&doc),
            Self::Doc(scoped) => scoped == doc,
        }
    }
}

/// Everything the Ready phase holds.
pub struct World {
    paths: WorldPaths,
    settings: Settings,
    game: GameData,
    report: LoadReport,
    campaigns: Vec<Campaign>,
    campaign: Campaign,
    stash: StashDoc,
    reagents: Reagents,
    blueprints: Blueprints,
    illusions: IllusionCollection,
    store: StoreDoc,
    characters: Vec<CharacterEntry>,
    facts: FactsCache,
    icons: IconCache,
    stash_view: StashView,
    store_view: StoreView,
    search_cache: SearchCache,
    ui_state: PersistedUiState,
    character_view: CharacterView,
    last_active: LastActive,
    selected: Option<Selected>,
    drag: Option<DragState>,
    autosave: Autosave,
    watcher: Result<Watcher, std::io::Error>,
    refresh: RefreshTracker,
    conflicts: Vec<Doc>,
    write_order: WriteOrder,
    settings_dialog: Option<SettingsDialog>,
    affixes: AffixTable,
    reference_view: ReferenceView,
    reference_cache: ReferenceCache,
    /// The store revision the stacks were last consolidated at.
    settled_revision: Option<u64>,
}

impl World {
    fn new(
        loaded: LoadedWorld,
        paths: WorldPaths,
        settings: Settings,
        ctx: &egui::Context,
        toasts: &mut Toasts,
    ) -> Self {
        let watcher = Watcher::start(ctx.clone());
        if let Err(error) = &watcher {
            toasts.error(format!(
                "external-change watching is off: the watcher thread could not start ({error})"
            ));
        }
        for warning in loaded.warnings {
            toasts.error(warning);
        }
        if let CampaignChoice::Missing(gone) = &loaded.campaign_choice {
            toasts.info(format!(
                "the remembered {gone} is not in the save directory: showing the {} files",
                loaded.campaign
            ));
        }
        let write_order = WriteOrder::new(SHARED_DOCS.into_iter().chain(
            (0..loaded.characters.len()).map(|slot| Doc::Character(CharacterSlot::new(slot))),
        ));
        let ui_state = PersistedUiState::load(paths.ui_state.clone());
        let store_view = ui_state.on_disk().store.clone();
        let reference_view = ui_state.on_disk().reference.clone();
        let mut world = Self {
            paths,
            settings,
            game: loaded.game,
            report: loaded.report,
            campaigns: loaded.campaigns,
            campaign: loaded.campaign,
            stash: loaded.stash,
            reagents: loaded.reagents,
            blueprints: loaded.blueprints,
            illusions: loaded.illusions,
            store: loaded.store,
            characters: loaded.characters,
            facts: FactsCache::default(),
            icons: IconCache::with_symbols(loaded.symbols),
            stash_view: StashView::default(),
            store_view,
            search_cache: SearchCache::default(),
            ui_state,
            character_view: CharacterView::opening_on(loaded.newest_character),
            last_active: LastActive::default(),
            selected: None,
            drag: None,
            autosave: Autosave::default(),
            watcher,
            refresh: RefreshTracker::default(),
            conflicts: Vec::new(),
            write_order,
            settings_dialog: None,
            affixes: loaded.affixes,
            reference_view,
            reference_cache: ReferenceCache::default(),
            settled_revision: None,
        };
        world.rewatch();
        world.carry_out_orders(Scope::Everything, toasts);
        world
    }

    /// Points the guard at the files now open.
    fn rewatch(&self) {
        if let Ok(watcher) = &self.watcher {
            watcher.watch(
                self.docs()
                    .into_iter()
                    .filter_map(|doc| self.path(doc))
                    .map(std::path::Path::to_path_buf)
                    .collect(),
            );
        }
    }

    /// Swaps the shared files for another campaign's and remembers
    /// the choice in the settings for the next launch. Unsaved edits
    /// are written first, backup-first as ever; nothing changes when
    /// that write, a pending external change, or the open stands in
    /// the way.
    fn switch_campaign(&mut self, next: Campaign, config: &ConfigDir, toasts: &mut Toasts) {
        if next == self.campaign {
            return;
        }
        if self.gate() == Gate::Suspended {
            toasts.error("decide the pending external change before switching campaigns");
            return;
        }
        if let Err(error) = self.flush(toasts) {
            toasts.error(format!(
                "could not save before switching campaigns: {error}"
            ));
            return;
        }
        if self.gate() == Gate::Suspended {
            return;
        }
        let shared = match open_shared(&self.paths.save, &next, &mut |_| {}) {
            Ok(shared) => shared,
            Err(error) => {
                toasts.error(format!("could not open the {next} files: {error}"));
                return;
            }
        };
        for doc in CAMPAIGN_DOCS {
            self.forget_refresh(doc);
        }
        self.stash = shared.stash;
        self.reagents = shared.reagents;
        self.blueprints = shared.blueprints;
        self.illusions = shared.illusions;
        self.campaign = next;
        self.stash_view = StashView::default();
        self.last_active.forget();
        self.selected = None;
        for warning in shared.warnings {
            toasts.error(warning);
        }
        self.rewatch();
        toasts.info(format!("showing the {} files", self.campaign));
        self.settings.campaign = Some(self.campaign.clone());
        self.save_settings(config, toasts);
        self.carry_out_orders(Scope::Campaign, toasts);
    }

    fn save_settings(&self, config: &ConfigDir, toasts: &mut Toasts) {
        if let Err(error) = settings::save(config, &self.settings) {
            toasts.error(format!("settings were not saved: {error}"));
        }
    }

    /// The open characters as a nomination names them.
    fn open_names(&self) -> Vec<automove::OpenName<'_>> {
        automove::open_names(&self.characters)
    }

    /// Carries out the standing orders `scope` covers: every tab
    /// nominated for auto-move among the open documents is emptied
    /// into the store, then every tab nominated for the purge loses
    /// what the store holds, then the component storage is synced.
    /// Nothing runs while an external change awaits the user's
    /// decision.
    fn carry_out_orders(&mut self, scope: Scope, toasts: &mut Toasts) {
        if self.gate() == Gate::Suspended {
            return;
        }
        for order in StandingOrder::ALL {
            for target in self.order_targets(order, scope) {
                self.carry_out(order, target, toasts);
            }
        }
        if scope.covers(Doc::Reagents) {
            self.sync_reagents(toasts);
        }
        if scope.covers(Doc::Blueprints) {
            self.sync_blueprints(toasts);
        }
    }

    /// The open tabs nominated for `order` that `scope` covers.
    fn order_targets(&self, order: StandingOrder, scope: Scope) -> Vec<AutoMoveTarget> {
        automove::targets(
            self.settings.nominations(order),
            &self.campaign,
            &self.open_names(),
        )
        .into_iter()
        .filter(|target| scope.covers(target.doc()))
        .collect()
    }

    fn carry_out(&mut self, order: StandingOrder, target: AutoMoveTarget, toasts: &mut Toasts) {
        match order {
            StandingOrder::AutoMove => self.auto_move(target, toasts),
            StandingOrder::PurgeDuplicates => self.purge(target, toasts),
        }
    }

    fn target_label(&self, target: AutoMoveTarget) -> String {
        match target {
            AutoMoveTarget::TransferStash(tab) => {
                format!("transfer stash tab {}", tab.value() + 1)
            }
            AutoMoveTarget::CharacterStash { character, tab } => format!(
                "{} stash tab {}",
                self.doc_label(Doc::Character(character)),
                tab.value() + 1
            ),
        }
    }

    /// Empties one nominated tab into the store through the same
    /// moves a drag makes; duplicates stay, and both documents join
    /// autosave with the store written first.
    fn auto_move(&mut self, target: AutoMoveTarget, toasts: &mut Toasts) {
        let doc = target.doc();
        self.warm_doc(doc);
        self.warm_doc(Doc::Store);
        let label = self.target_label(target);
        let at = now();
        let rule = self.settings.bulk_duplicates;
        let outcome = match target {
            AutoMoveTarget::TransferStash(tab) => bulk::vault_tab(
                self.stash.stash_mut(),
                &self.campaign,
                tab,
                self.store.store_mut(),
                &self.facts,
                rule,
                at,
            ),
            AutoMoveTarget::CharacterStash { character, tab } => {
                let Some(character_doc) = self
                    .characters
                    .get_mut(character.value())
                    .and_then(CharacterEntry::doc_mut)
                else {
                    return;
                };
                let realm = character_doc.realm();
                match character_doc.file_mut() {
                    Ok(file) => bulk::vault_player_tab(
                        file,
                        realm,
                        tab,
                        self.store.store_mut(),
                        &self.facts,
                        rule,
                        at,
                    ),
                    Err(error) => {
                        toasts.error(format!("auto-move from the {label} skipped: {error}"));
                        return;
                    }
                }
            }
        };
        match outcome {
            Ok(summary) if summary.is_noop() => {
                if summary.duplicates > 0 {
                    toasts.info(format!(
                        "nothing to auto-move from the {label}: {} duplicate(s) left in place",
                        summary.duplicates
                    ));
                }
            }
            Ok(summary) => {
                self.mark_edited(Doc::Store);
                self.mark_edited(doc);
                self.write_order.prioritize(Doc::Store);
                toasts.info(format!(
                    "auto-moved the {label} into the vault store: {summary}"
                ));
            }
            Err(error) => toasts.error(format!("auto-move from the {label} failed: {error}")),
        }
    }

    /// Deletes from one nominated tab every item the store already
    /// holds by record and roll seed; only the tab's document
    /// changes, and it joins autosave.
    fn purge(&mut self, target: AutoMoveTarget, toasts: &mut Toasts) {
        let doc = target.doc();
        self.warm_doc(doc);
        self.warm_doc(Doc::Store);
        let label = self.target_label(target);
        let outcome = match target {
            AutoMoveTarget::TransferStash(tab) => bulk::purge_duplicates(
                &mut self.stash.stash_mut().tabs,
                tab,
                self.store.store(),
                &self.facts,
            ),
            AutoMoveTarget::CharacterStash { character, tab } => {
                let Some(character_doc) = self
                    .characters
                    .get_mut(character.value())
                    .and_then(CharacterEntry::doc_mut)
                else {
                    return;
                };
                match character_doc.file_mut() {
                    Ok(file) => {
                        bulk::purge_player_duplicates(file, tab, self.store.store(), &self.facts)
                    }
                    Err(error) => {
                        toasts.error(format!("purge of the {label} skipped: {error}"));
                        return;
                    }
                }
            }
        };
        match outcome {
            Ok(summary) if summary.is_noop() => {}
            Ok(summary) => {
                self.mark_edited(doc);
                toasts.info(format!("purged the {label}: {summary}"));
            }
            Err(error) => toasts.error(format!("purge of the {label} failed: {error}")),
        }
    }

    /// Raises the store's reagent counts to the open storage's;
    /// only the store changes.
    fn sync_reagents(&mut self, toasts: &mut Toasts) {
        if self.settings.sync_reagents == ReagentSync::Off {
            return;
        }
        let Some(doc) = self.reagents.doc() else {
            return;
        };
        let summary =
            bulk::sync_reagents(doc.storage(), &self.campaign, self.store.store_mut(), now());
        if summary.is_noop() {
            return;
        }
        self.mark_edited(Doc::Store);
        self.write_order.prioritize(Doc::Store);
        toasts.info(format!(
            "synced the component storage into the vault store: {summary}"
        ));
    }

    /// The learned-blueprint sync: every blueprint the campaign's
    /// list holds and the vault does not know yet is recorded as
    /// learned; store only, nothing removed, no item made.
    fn sync_blueprints(&mut self, toasts: &mut Toasts) {
        if self.settings.sync_blueprints == BlueprintSync::Off {
            return;
        }
        let Some(doc) = self.blueprints.doc() else {
            return;
        };
        let summary = bulk::sync_blueprints(
            &doc.formulas().entries,
            &self.campaign,
            self.store.store_mut(),
            now(),
        );
        if summary.is_noop() {
            return;
        }
        self.mark_edited(Doc::Store);
        self.write_order.prioritize(Doc::Store);
        toasts.info(format!("vault: {summary}"));
    }

    /// A tab nominated for a standing order or withdrawn from it; a
    /// nomination is carried out at once when its tab is open.
    fn set_order(&mut self, request: OrderRequest, config: &ConfigDir, toasts: &mut Toasts) {
        let (order, target) = match request {
            OrderRequest::Nominate { order, tab } => {
                let target = automove::resolve(&tab, &self.campaign, &self.open_names());
                self.settings.nominate(order, tab);
                (order, target)
            }
            OrderRequest::Withdraw { order, tab } => {
                self.settings.withdraw(order, &tab);
                (order, None)
            }
        };
        self.save_settings(config, toasts);
        if let Some(target) = target
            && self.gate() == Gate::Open
        {
            self.carry_out(order, target, toasts);
        }
    }

    fn set_reagent_sync(&mut self, sync: ReagentSync, config: &ConfigDir, toasts: &mut Toasts) {
        self.settings.sync_reagents = sync;
        self.save_settings(config, toasts);
        if sync == ReagentSync::On && self.gate() == Gate::Open {
            self.sync_reagents(toasts);
        }
    }

    fn set_blueprint_sync(&mut self, sync: BlueprintSync, config: &ConfigDir, toasts: &mut Toasts) {
        self.settings.sync_blueprints = sync;
        self.save_settings(config, toasts);
        if sync == BlueprintSync::On && self.gate() == Gate::Open {
            self.sync_blueprints(toasts);
        }
    }

    fn set_bulk_duplicates(
        &mut self,
        rule: BulkDuplicates,
        config: &ConfigDir,
        toasts: &mut Toasts,
    ) {
        self.settings.bulk_duplicates = rule;
        self.save_settings(config, toasts);
    }

    /// A container as the bulk toasts name it.
    fn container_label(&self, container: Container) -> String {
        match container {
            Container::TransferStash(tab) => format!("transfer stash tab {}", tab.value() + 1),
            Container::Sack { character, sack } => format!(
                "{} sack {}",
                self.doc_label(Doc::Character(character)),
                sack.value() + 1
            ),
            Container::CharacterStash { character, tab } => format!(
                "{} stash tab {}",
                self.doc_label(Doc::Character(character)),
                tab.value() + 1
            ),
        }
    }

    /// A whole container moved or copied into the store through the
    /// same moves a drag makes, under the bulk-duplicates rule, or a
    /// stash tab emptied after the user confirmed; the documents
    /// touched join autosave with the store written first. Refused
    /// while an external change awaits the user's decision.
    fn bulk(&mut self, request: BulkRequest, toasts: &mut Toasts) {
        if self.gate() == Gate::Suspended {
            toasts.error("decide the pending external change before a bulk operation");
            return;
        }
        let BulkRequest { container, op } = request;
        let doc = container.doc();
        let label = self.container_label(container);
        self.warm_doc(doc);
        self.warm_doc(Doc::Store);
        let outcome = match op {
            BulkOp::Transfer(mode) => self.bulk_transfer(container, mode),
            BulkOp::Clear => self.bulk_clear(container),
        };
        match outcome {
            Ok(BulkDone::Transferred(summary)) if summary.is_noop() => {
                toasts.info(format!(
                    "nothing to {op} from the {label}: {} duplicate(s) left in place",
                    summary.duplicates
                ));
            }
            Ok(BulkDone::Transferred(summary)) => {
                self.mark_edited(Doc::Store);
                if op == BulkOp::Transfer(Mode::Move) {
                    self.mark_edited(doc);
                }
                self.write_order.prioritize(Doc::Store);
                toasts.info(format!(
                    "{} {} item(s) from the {label} into the vault store; {} duplicate(s) \
                     left in place",
                    op.done(),
                    summary.moved.len(),
                    summary.duplicates
                ));
            }
            Ok(BulkDone::Cleared(summary)) if summary.is_noop() => {
                toasts.info(format!("the {label} was already empty"));
            }
            Ok(BulkDone::Cleared(summary)) => {
                self.mark_edited(doc);
                self.write_order.prioritize(doc);
                toasts.info(format!("emptied the {label}: {summary}"));
            }
            Err(error) => toasts.error(format!("{op} on the {label} failed: {error}")),
        }
        self.revalidate_selection();
    }

    /// Every item of a container into the store — lifted out of it
    /// or cloned — under the bulk-duplicates rule.
    fn bulk_transfer(&mut self, container: Container, mode: Mode) -> Result<BulkDone, ApplyError> {
        let rule = self.settings.bulk_duplicates;
        let at = now();
        let summary = match container {
            Container::TransferStash(tab) => match mode {
                Mode::Move => bulk::vault_tab(
                    self.stash.stash_mut(),
                    &self.campaign,
                    tab,
                    self.store.store_mut(),
                    &self.facts,
                    rule,
                    at,
                ),
                Mode::Copy => bulk::copy_tab(
                    self.stash.stash(),
                    &self.campaign,
                    tab,
                    self.store.store_mut(),
                    &self.facts,
                    rule,
                    at,
                ),
            }?,
            Container::Sack { character, sack } => {
                let (realm, file) = editable_character(&mut self.characters, character)?;
                match mode {
                    Mode::Move => bulk::vault_sack(
                        file,
                        realm,
                        sack,
                        self.store.store_mut(),
                        &self.facts,
                        rule,
                        at,
                    ),
                    Mode::Copy => bulk::copy_sack(
                        file,
                        realm,
                        sack,
                        self.store.store_mut(),
                        &self.facts,
                        rule,
                        at,
                    ),
                }?
            }
            Container::CharacterStash { character, tab } => {
                let (realm, file) = editable_character(&mut self.characters, character)?;
                match mode {
                    Mode::Move => bulk::vault_player_tab(
                        file,
                        realm,
                        tab,
                        self.store.store_mut(),
                        &self.facts,
                        rule,
                        at,
                    ),
                    Mode::Copy => bulk::copy_player_tab(
                        file,
                        realm,
                        tab,
                        self.store.store_mut(),
                        &self.facts,
                        rule,
                        at,
                    ),
                }?
            }
        };
        Ok(BulkDone::Transferred(summary))
    }

    /// Empties a stash tab; a sack is never offered a "Delete all",
    /// so one asked for is refused loudly rather than emptied.
    fn bulk_clear(&mut self, container: Container) -> Result<BulkDone, ApplyError> {
        let summary = match container {
            Container::TransferStash(tab) => {
                bulk::clear_tab(&mut self.stash.stash_mut().tabs, tab)?
            }
            Container::Sack { character, .. } => {
                return Err(ApplyError::NotAnItemContainer(Doc::Character(character)));
            }
            Container::CharacterStash { character, tab } => {
                let (_, file) = editable_character(&mut self.characters, character)?;
                let tabs = &mut file.stash_mut().ok_or(TransferError::NoPlayerStash)?.tabs;
                bulk::clear_tab(tabs, tab)?
            }
        };
        Ok(BulkDone::Cleared(summary))
    }

    /// Every document, in default write order.
    fn docs(&self) -> Vec<Doc> {
        SHARED_DOCS
            .into_iter()
            .chain((0..self.characters.len()).map(|slot| Doc::Character(CharacterSlot::new(slot))))
            .collect()
    }

    /// One frame of the Ready phase; `Some` when the settings applied
    /// call for a full reload.
    fn show(
        &mut self,
        ui: &mut Ui,
        theme: &Theme,
        config: &ConfigDir,
        toasts: &mut Toasts,
    ) -> Option<Phase> {
        let mut frame = DragFrame::default();
        let modifiers = ui.input(|input| input.modifiers);
        let mode = mode_of(modifiers);
        self.search_shortcuts(ui.ctx());
        let gear = egui::Panel::bottom("status")
            .show(ui, |ui| self.status_bar(ui, theme, toasts))
            .inner;
        if gear && self.settings_dialog.is_none() && self.gate() == Gate::Open {
            self.settings_dialog = Some(SettingsDialog::open(&self.settings, config));
        }
        egui::Panel::bottom("characters")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                let mut cx = PaneCtx {
                    game: &self.game,
                    facts: &mut self.facts,
                    icons: &mut self.icons,
                    palette: &theme.palette,
                    settings: &self.settings,
                    drag: self.drag.as_ref(),
                    mode,
                };
                panes::character::show(
                    ui,
                    &self.characters,
                    &mut self.character_view,
                    theme,
                    &mut cx,
                    &mut frame,
                );
            });
        egui::Panel::right("store")
            .resizable(true)
            .default_size(480.0)
            .show(ui, |ui| {
                let mut cx = PaneCtx {
                    game: &self.game,
                    facts: &mut self.facts,
                    icons: &mut self.icons,
                    palette: &theme.palette,
                    settings: &self.settings,
                    drag: self.drag.as_ref(),
                    mode,
                };
                panes::store::show(
                    ui,
                    &self.store,
                    &mut self.store_view,
                    &mut self.search_cache,
                    theme,
                    &mut cx,
                    &mut frame,
                );
            });
        let switch = egui::CentralPanel::default()
            .show(ui, |ui| {
                let mut cx = PaneCtx {
                    game: &self.game,
                    facts: &mut self.facts,
                    icons: &mut self.icons,
                    palette: &theme.palette,
                    settings: &self.settings,
                    drag: self.drag.as_ref(),
                    mode,
                };
                panes::stash::show(
                    ui,
                    panes::stash::Selection {
                        campaign: &self.campaign,
                        campaigns: &self.campaigns,
                    },
                    panes::stash::Shared {
                        stash: &self.stash,
                        reagents: &self.reagents,
                        blueprints: &self.blueprints,
                        illusions: &self.illusions,
                    },
                    &mut self.stash_view,
                    theme,
                    &mut cx,
                    &mut frame,
                )
            })
            .inner;
        self.show_inspector(ui.ctx(), theme, mode, &mut frame);
        self.show_reference_cards(ui.ctx(), &theme.palette);
        self.show_conflict_modal(ui.ctx(), theme, toasts);
        let reload = self.show_settings_dialog(ui.ctx(), theme, config, toasts);
        self.finish_frame(ui.ctx(), frame, modifiers, &theme.palette, config, toasts);
        if let Some(next) = switch {
            self.switch_campaign(next, config, toasts);
        }
        self.settle_store(toasts);
        self.persist_ui_state(ui.ctx());
        reload
    }

    /// Keeps every stackable record to one stack: whenever the store
    /// changed since the last look — a load, a drop, an import, a
    /// standing order, a reload — later stacks of a record are folded
    /// into its first, and the fold is written by autosave like any
    /// edit. Not while a drag is in flight, since the lifted entry
    /// must still be there when it lands.
    fn settle_store(&mut self, toasts: &mut Toasts) {
        if self.drag.is_some() || self.settled_revision == Some(self.store.revision()) {
            return;
        }
        let folded = self
            .store
            .store_mut()
            .consolidate_stacks(|item| self.game.is_stack(item));
        if folded.entries > 0 {
            self.store.tracking_mut().mark_edited();
            self.write_order.prioritize(Doc::Store);
            self.revalidate_selection();
            toasts.info(format!("consolidated the vault's stacks: {folded}"));
        }
        self.settled_revision = Some(self.store.revision());
    }

    /// The gear's modal, while open: export and import act at once;
    /// Apply saves the draft and pays what it costs.
    fn show_settings_dialog(
        &mut self,
        ctx: &egui::Context,
        theme: &Theme,
        config: &ConfigDir,
        toasts: &mut Toasts,
    ) -> Option<Phase> {
        let vault_items = self.store.store().len();
        let action = self
            .settings_dialog
            .as_mut()?
            .show(ctx, theme, config, vault_items)?;
        match action {
            SettingsAction::Export => {
                self.export_store(toasts);
                None
            }
            SettingsAction::Import => {
                self.import_store(toasts);
                None
            }
            SettingsAction::Cancel => {
                self.settings_dialog = None;
                None
            }
            SettingsAction::Apply(next) => {
                self.settings_dialog = None;
                self.apply_settings(next, ctx, config, toasts)
            }
        }
    }

    /// Puts `next` in force: the rules at once; a moved store by
    /// swapping the store document; changed directories by writing
    /// everything and starting over from the loader. A pending
    /// external change, or a save that fails, leaves the old settings
    /// in force.
    fn apply_settings(
        &mut self,
        next: Settings,
        ctx: &egui::Context,
        config: &ConfigDir,
        toasts: &mut Toasts,
    ) -> Option<Phase> {
        match Change::between(&self.settings, &next, config) {
            Change::Nothing => None,
            Change::Rules => {
                let reagents_turned_on = next.sync_reagents == ReagentSync::On
                    && self.settings.sync_reagents == ReagentSync::Off;
                let blueprints_turned_on = next.sync_blueprints == BlueprintSync::On
                    && self.settings.sync_blueprints == BlueprintSync::Off;
                self.settings = next;
                self.save_settings(config, toasts);
                if self.gate() == Gate::Open {
                    if reagents_turned_on {
                        self.sync_reagents(toasts);
                    }
                    if blueprints_turned_on {
                        self.sync_blueprints(toasts);
                    }
                }
                None
            }
            Change::Store => {
                if self.settle_before_switching("switching the vault store", toasts) {
                    self.switch_store(next, config, toasts);
                }
                None
            }
            Change::World => {
                if !self.settle_before_switching("reloading", toasts) {
                    return None;
                }
                let paths = match world_paths(&next, config) {
                    Ok(paths) => paths,
                    Err(problem) => {
                        toasts.error(format!("settings not applied: {problem}"));
                        return None;
                    }
                };
                self.settings = next;
                self.save_settings(config, toasts);
                let current = self.ui_snapshot();
                self.ui_state.flush(current);
                Some(Phase::Loading(loader::start(
                    paths,
                    self.settings.clone(),
                    ctx.clone(),
                )))
            }
        }
    }

    /// Writes every unsaved edit before a switch; `false` when an
    /// external change awaits a decision or the write failed, either
    /// of which means the switch must not happen.
    fn settle_before_switching(&mut self, what: &str, toasts: &mut Toasts) -> bool {
        if self.gate() == Gate::Suspended {
            toasts.error(format!("decide the pending external change before {what}"));
            return false;
        }
        if let Err(error) = self.flush(toasts) {
            toasts.error(format!("could not save before {what}: {error}"));
            return false;
        }
        self.gate() == Gate::Open
    }

    /// Swaps the store document for the one `next` names — an absent
    /// file starts empty and is created by the first save — and runs
    /// the standing orders as any load of the store does. The old
    /// store stays open when the new one cannot be read.
    fn switch_store(&mut self, next: Settings, config: &ConfigDir, toasts: &mut Toasts) {
        let path = next.store_file(config);
        let store = match StoreDoc::open(path.clone()) {
            Ok(store) => store,
            Err(error) => {
                toasts.error(format!("the vault store was not switched: {error}"));
                return;
            }
        };
        self.forget_refresh(Doc::Store);
        self.store = store;
        self.paths.store = path;
        self.search_cache = SearchCache::default();
        self.selected = None;
        self.rewatch();
        self.settings = next;
        self.save_settings(config, toasts);
        toasts.info(format!(
            "opened the vault store at {}: {} items",
            self.store.path().display(),
            self.store.store().len()
        ));
        self.carry_out_orders(Scope::Everything, toasts);
    }

    /// A copy of the store as it is now, to a file the user picks.
    /// The open store's own path is refused: a write there would slip
    /// past the stamp the guard compares against.
    fn export_store(&self, toasts: &mut Toasts) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Grim Vault store", &["json"])
            .set_file_name(grimvault_core::settings::STORE_FILE)
            .save_file()
        else {
            return;
        };
        if path == self.store.path() {
            toasts.error("that is the open vault store itself; choose another file");
            return;
        }
        match write_synced(&path, &self.store.store().to_json()) {
            Ok(()) => toasts.info(format!(
                "exported a copy of the vault ({} items) to {}",
                self.store.store().len(),
                path.display()
            )),
            Err(error) => toasts.error(format!("could not write {}: {error}", path.display())),
        }
    }

    /// Merges another store file into the open one: what this vault
    /// does not hold is added under fresh ids, nothing is removed.
    fn import_store(&mut self, toasts: &mut Toasts) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Grim Vault store", &["json"])
            .pick_file()
        else {
            return;
        };
        let bytes = match read_verified(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                toasts.error(format!("could not read {}: {error}", path.display()));
                return;
            }
        };
        let other = match VaultStore::from_json(&bytes) {
            Ok(other) => other,
            Err(error) => {
                toasts.error(format!(
                    "{} is not a vault store this app reads: {error}",
                    path.display()
                ));
                return;
            }
        };
        let merged = self
            .store
            .store_mut()
            .merge(&other, |item| self.game.is_stack(item));
        if merged.added == 0 && merged.raised == 0 {
            toasts.info(format!("nothing new in {}: {merged}", path.display()));
            return;
        }
        self.store.tracking_mut().mark_edited();
        self.write_order.prioritize(Doc::Store);
        toasts.info(format!("imported {}: {merged}", path.display()));
    }

    /// ⌘F / Ctrl+F opens the search view with its name field focused;
    /// Esc with nothing focused returns to the buckets.
    fn search_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_shortcut(&SEARCH_SHORTCUT)) {
            self.store_view.mode = StoreMode::Search;
            self.search_cache.focus_requested = true;
        }
        if self.store_view.mode == StoreMode::Search
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
            && ctx.memory(|memory| memory.focused().is_none())
        {
            self.store_view.mode = StoreMode::Buckets;
        }
    }

    /// The inspector window over the selected item, when there is one;
    /// closing it drops the selection.
    fn show_inspector(
        &mut self,
        ctx: &egui::Context,
        theme: &Theme,
        mode: Mode,
        frame: &mut DragFrame,
    ) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let place = self.place_label(selected.source);
        let mut cx = PaneCtx {
            game: &self.game,
            facts: &mut self.facts,
            icons: &mut self.icons,
            palette: &theme.palette,
            settings: &self.settings,
            drag: self.drag.as_ref(),
            mode,
        };
        let supplies = Supplies {
            store: self.store.store(),
            reagents: self.reagents.doc().map(ReagentDoc::storage),
        };
        if !inspector::show(ctx, &selected, &place, supplies, theme, &mut cx, frame) {
            self.selected = None;
        }
    }

    /// The reference cards the user has open.
    fn show_reference_cards(&mut self, ctx: &egui::Context, palette: &Palette) {
        reference::show(
            ctx,
            &self.affixes,
            &mut self.reference_view,
            &mut self.reference_cache,
            palette,
        );
    }

    /// Where an item sits, for the inspector's subtitle.
    fn place_label(&self, source: DragSource) -> String {
        match source {
            DragSource::Grid {
                container: Container::TransferStash(tab),
                ..
            } => format!("{} transfer stash, tab {}", self.campaign, tab.value() + 1),
            DragSource::Grid {
                container: Container::Sack { character, sack },
                ..
            } => format!(
                "{}, sack {}",
                self.doc_label(Doc::Character(character)),
                sack.value() + 1
            ),
            DragSource::Grid {
                container: Container::CharacterStash { character, tab },
                ..
            } => format!(
                "{}, stash tab {}",
                self.doc_label(Doc::Character(character)),
                tab.value() + 1
            ),
            DragSource::Store(id) => self.store.store().get(id).map_or_else(
                || format!("vault store, stored item {id}"),
                |stored| format!("vault store, stored item {id} — from {}", stored.origin()),
            ),
            DragSource::Reagent { .. } => Doc::Reagents.to_string(),
        }
    }

    /// The open containers for reading: what a click or a check of
    /// the selection looks through, without counting as an edit.
    fn views(&self) -> Views<'_> {
        Views {
            campaign: &self.campaign,
            stash: self.stash.stash(),
            store: self.store.store(),
            reagents: self.reagents.doc().map(ReagentDoc::storage),
            characters: self
                .characters
                .iter()
                .map(|entry| {
                    entry
                        .doc()
                        .filter(|doc| doc.writable() == Writable::Yes)
                        .map(|doc| (doc.realm(), doc.file()))
                })
                .collect(),
        }
    }

    /// Binds the inspector to the item at `source`, if one is there.
    fn select(&mut self, source: DragSource) {
        self.selected = drag::peek(&self.views(), source)
            .ok()
            .map(|(item, _)| Selected { source, item });
    }

    /// After a move or a reload: the selection stands while the same
    /// item is still at its source, and lapses otherwise.
    fn revalidate_selection(&mut self) {
        let Some(selected) = &self.selected else {
            return;
        };
        let still_there =
            drag::peek(&self.views(), selected.source).is_ok_and(|(item, _)| item == selected.item);
        if !still_there {
            self.selected = None;
        }
    }

    /// After a socket edit: the inspector follows the edited item.
    fn refresh_selection(&mut self) {
        if let Some(source) = self.selected.as_ref().map(|selected| selected.source) {
            self.select(source);
        }
    }

    /// The live view state, as it would be written.
    fn ui_snapshot(&self) -> UiState {
        UiState::of(
            &self.store_view,
            &self.reference_view,
            self.ui_state.on_disk(),
        )
    }

    /// Persists the view state once it has held still; called at the
    /// end of every frame.
    fn persist_ui_state(&mut self, ctx: &egui::Context) {
        let current = self.ui_snapshot();
        if let Some(wait) = self.ui_state.observe(current, Instant::now()) {
            ctx.request_repaint_after(wait);
        }
    }

    /// The bottom strip; `true` when the gear was clicked.
    fn status_bar(&mut self, ui: &mut Ui, theme: &Theme, toasts: &Toasts) -> bool {
        ui.horizontal_wrapped(|ui| {
            let gear = ui
                .button("⚙")
                .on_hover_text("Settings: directories, the vault store file, rules, export and import")
                .clicked();
            ui.separator();
            reference::menu(ui, &mut self.reference_view);
            ui.separator();
            ui.label(theme.path_text(format!("saves: {}", self.paths.save.path().display())));
            ui.separator();
            ui.label(format!("campaign: {}", self.campaign));
            ui.separator();
            ui.label(format!("store: {} items", self.store.store().len()));
            ui.separator();
            let state = self.autosave.state(self.pending(), self.gate());
            let colour = match state {
                AutosaveState::Saved => theme.palette.text_weak,
                AutosaveState::Unsaved | AutosaveState::Saving => theme.palette.warn,
                AutosaveState::Suspended => theme.palette.error,
            };
            ui.colored_label(colour, format!("autosave: {state}"));
            ui.separator();
            ui.weak(format!(
                "backup-first: stash {}, store {}, components {}, blueprints {}, illusions {}, characters {}",
                backup_label(self.stash.tracking().backup()),
                backup_label(self.store.tracking().backup()),
                optional_backup_label(&self.reagents),
                optional_backup_label(&self.blueprints),
                optional_backup_label(&self.illusions),
                characters_backup_label(&self.characters)
            ))
            .on_hover_text(
                "The first write of each file since it was loaded takes a grimvault-bak backup beside it; \
                 later autosaves of the same load reuse that backup.",
            );
            ui.separator();
            ui.weak("hold Alt or ⌘/Ctrl while dropping to copy");
            ui.separator();
            ui.weak("right-click an item to move it between the game and the vault; hold Shift to copy");
            ui.separator();
            ui.weak("click an item to inspect it and its sockets");
            ui.separator();
            ui.weak(format!(
                "game data: {} layers, {} item archives, {} mods, {} of {} tile symbols",
                self.report.databases,
                self.report.item_archives,
                self.report.mods,
                self.report.symbols,
                grimvault_core::facets::Symbol::ALL.len()
            ));
            if let Some(error) = toasts.last_error() {
                ui.separator();
                ui.colored_label(theme.palette.error, error);
            }
            gear
        })
        .inner
    }

    /// Adopts a drag the panes began, paints the lifted item at the
    /// pointer, and commits or snaps back on release. Double-clicks
    /// and right-clicks are moves too, and an iron-bits edit or a
    /// confirmed reset is applied here.
    fn finish_frame(
        &mut self,
        ctx: &egui::Context,
        frame: DragFrame,
        modifiers: egui::Modifiers,
        palette: &Palette,
        config: &ConfigDir,
        toasts: &mut Toasts,
    ) {
        let mode = mode_of(modifiers);
        if let Some(request) = frame.standing_order {
            self.set_order(request, config, toasts);
        }
        if let Some(sync) = frame.reagent_sync {
            self.set_reagent_sync(sync, config, toasts);
        }
        if let Some(sync) = frame.blueprint_sync {
            self.set_blueprint_sync(sync, config, toasts);
        }
        if let Some(rule) = frame.bulk_duplicates {
            self.set_bulk_duplicates(rule, config, toasts);
        }
        if let Some(request) = frame.bulk {
            self.bulk(request, toasts);
        }
        if let Some((slot, money)) = frame.set_money {
            self.set_money(slot, money, toasts);
        }
        if let Some(path) = &frame.import_gds {
            self.import_gds(path, toasts);
        }
        if let Some((slot, reset)) = frame.respec {
            self.respec(slot, reset, toasts);
        }
        if let Some(request) = frame.crafting {
            self.perform_crafting(request, toasts);
        }
        if let Some(request) = frame.socket {
            self.perform_socket(request, toasts);
        }
        if let Some(source) = frame.select {
            self.select(source);
        }
        if let Some(container) = frame.touched {
            self.last_active.touch(container);
        }
        if self.drag.is_none()
            && let Some(source) = frame.double_click
        {
            let home = Container::TransferStash(self.stash_view.tab);
            self.perform(drag::quick_move(source, mode, home, None), toasts);
        }
        if self.drag.is_none()
            && let Some(source) = frame.right_click
        {
            let mode = right_click_mode(modifiers);
            let home = self.last_active.container_or(self.stash_view.tab);
            let storage = self.storage_kind(source);
            self.perform(drag::quick_move(source, mode, home, storage), toasts);
        }
        if self.drag.is_none() {
            if let Some(begin) = &frame.begin {
                self.last_active.touch_source(begin.source);
            }
            self.drag = frame.begin;
        }
        let Some(state) = self.drag.clone() else {
            return;
        };
        self.paint_ghost(ctx, &state, mode, palette);
        if ctx.input(|input| input.pointer.any_released()) {
            if let Some(candidate) = frame.candidate {
                let mv = Move {
                    source: state.source,
                    target: candidate.target,
                    mode,
                };
                match (candidate.target, candidate.fit) {
                    (_, Fit::Fits) => self.perform(mv, toasts),
                    (DropTarget::Cell { .. }, Fit::Blocked) => {
                        toasts.info("no room at that cell; snapped back");
                    }
                    (DropTarget::Cell { .. }, Fit::Unresolvable) => toasts.error(
                        "that grid holds items with unknown footprints; nothing can be placed there",
                    ),
                    (DropTarget::Reagents(_), Fit::Blocked | Fit::Unresolvable) => {
                        match state.source {
                            DragSource::Grid { .. } | DragSource::Store(_) => toasts.info(
                                "only components and crafting materials go in the storage; snapped back",
                            ),
                            DragSource::Reagent { .. } => {}
                        }
                    }
                    (
                        DropTarget::Container(_) | DropTarget::Store,
                        Fit::Blocked | Fit::Unresolvable,
                    ) => {}
                }
            }
            self.drag = None;
            ctx.request_repaint();
        }
    }

    fn paint_ghost(
        &mut self,
        ctx: &egui::Context,
        state: &DragState,
        mode: Mode,
        palette: &Palette,
    ) {
        let Some(cursor) = ctx.pointer_latest_pos() else {
            return;
        };
        let size = vec2(
            CELL_PX * cells(state.footprint.width),
            CELL_PX * cells(state.footprint.height),
        );
        let rect = Rect::from_min_size(cursor - state.grab, size);
        let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("drag-ghost")));
        let bitmap = self.facts.base(&self.game, &state.item).bitmap.clone();
        match self.icons.icon(ctx, &self.game, bitmap.as_ref()) {
            Icon::Texture(texture) => painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::from_rgba_unmultiplied(255, 255, 255, 210),
            ),
            Icon::Missing(_) => {
                painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(140));
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(2),
                    Stroke::new(1.0, palette.accent),
                    StrokeKind::Inside,
                )
            }
        };
        if mode == Mode::Copy {
            let badge =
                Rect::from_center_size(rect.right_top() + vec2(-8.0, 8.0), vec2(16.0, 16.0));
            painter.rect_filled(badge, CornerRadius::same(8), palette.accent);
            painter.text(
                badge.center(),
                Align2::CENTER_CENTER,
                "+",
                FontId::proportional(13.0),
                palette.text_strong,
            );
        }
    }

    /// Every move goes through `drag::apply`, hence through
    /// `grimvault_core::transfer`; the shell only marks what changed
    /// and remembers which document is the move's destination.
    fn perform(&mut self, mv: Move, toasts: &mut Toasts) {
        self.last_active.touch_move(mv);
        self.warm_for(mv);
        let now = now();
        let (carried, outcome) = self.with_containers(|containers, facts, _| {
            let carried = drag::peek(containers, mv.source).ok().map(|(item, _)| item);
            (carried, drag::apply(mv, containers, facts, now))
        });
        let moved = carried.map_or_else(
            || "the item".to_string(),
            |item| {
                let name = self.facts.facts(&self.game, &item).display_name();
                if item.stack_count > 1 {
                    format!("{} × {name}", item.stack_count)
                } else {
                    name
                }
            },
        );
        match outcome {
            Ok(Applied::Changed {
                mode,
                from,
                to,
                landing,
            }) => {
                self.mark_edited(to);
                let verb = match mode {
                    Mode::Move => {
                        self.mark_edited(from);
                        "moved"
                    }
                    Mode::Copy => "copied",
                };
                self.write_order.prioritize(to);
                let destination = self.doc_label(to);
                let where_ = match landing {
                    Landing::Cell(pos) => {
                        format!("into the {destination} at ({}, {})", pos.x, pos.y)
                    }
                    Landing::Stored(id) => format!("into the {destination} as stored item {id}"),
                    Landing::Merged => format!("into the {destination}"),
                };
                toasts.info(format!("{verb} {moved} {where_}"));
            }
            Ok(Applied::Unmoved) => {}
            Err(error) => toasts.error(error.to_string()),
        }
        self.revalidate_selection();
    }

    /// Every open container at once — the ends a move or a socket edit
    /// reaches — with the memo and the game data beside them. Counts
    /// as an edit of the store, so reads go through [`Self::views`].
    fn with_containers<T>(
        &mut self,
        edit: impl FnOnce(&mut Containers<'_>, &FactsCache, &GameData) -> T,
    ) -> T {
        let World {
            campaign,
            stash,
            store,
            reagents,
            characters,
            facts,
            game,
            ..
        } = self;
        let mut containers = Containers {
            campaign,
            stash: stash.stash_mut(),
            store: store.store_mut(),
            reagents: reagents.doc_mut().map(ReagentDoc::storage_mut),
            characters: characters
                .iter_mut()
                .map(|entry| {
                    let doc = entry.doc_mut()?;
                    let realm = doc.realm();
                    doc.file_mut()
                        .ok()
                        .map(|file| OpenCharacter { realm, file })
                })
                .collect(),
        };
        edit(&mut containers, facts, game)
    }

    /// A socket edit from the inspector: the host and the part's
    /// source are edited through the same containers a move uses, the
    /// documents that changed join autosave with the part's
    /// destination written first, and the inspector follows the
    /// edited item.
    fn perform_socket(&mut self, request: sockets::Request, toasts: &mut Toasts) {
        let seed = fresh_seed();
        let now = now();
        let outcome = self.with_containers(|containers, _, game| {
            sockets::apply(request, containers, game, seed, now)
        });
        match outcome {
            Ok(sockets::Applied::Attached {
                socket,
                part,
                host,
                from,
            }) => {
                self.mark_edited(host);
                self.mark_edited(from);
                self.write_order.prioritize(host);
                toasts.info(format!(
                    "put {} in as the {socket} of the item in the {}",
                    self.record_name(&part),
                    self.doc_label(host)
                ));
            }
            Ok(sockets::Applied::Detached {
                socket,
                part,
                host,
                stored,
            }) => {
                self.mark_edited(host);
                self.mark_edited(Doc::Store);
                self.write_order.prioritize(Doc::Store);
                toasts.info(format!(
                    "freed the {socket} {} into the vault store as stored item {stored}",
                    self.record_name(&part)
                ));
            }
            Err(error) => toasts.error(error.to_string()),
        }
        self.refresh_selection();
    }

    /// The database's name for a record, as the panes show it.
    fn record_name(&mut self, record: &RecordId) -> String {
        let item = Item {
            base_name: record.as_str().to_string(),
            ..Item::default()
        };
        self.facts.base(&self.game, &item).name.clone()
    }

    /// The storage tab a right-clicked store item belongs in, `None`
    /// for anything that is not a component or crafting material.
    fn storage_kind(&mut self, source: DragSource) -> Option<ReagentKind> {
        match source {
            DragSource::Store(id) => {
                let item = self.store.store().get(id)?.item();
                self.facts.base(&self.game, item).reagent
            }
            DragSource::Grid { .. } | DragSource::Reagent { .. } => None,
        }
    }

    /// Adds a GD Stash export's items to the store; autosave then
    /// writes the store as after any other edit. A file that cannot be
    /// read or parsed changes nothing.
    fn import_gds(&mut self, path: &Path, toasts: &mut Toasts) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let bytes = match read_verified(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                toasts.error(format!("could not read {name}: {error}"));
                return;
            }
        };
        let export = match gds::parse(&bytes) {
            Ok(export) => export,
            Err(error) => {
                toasts.error(format!(
                    "{name} is not a GD Stash export this app reads: {error}"
                ));
                return;
            }
        };
        let report = gds::import(self.store.store_mut(), &export, path, &self.game, now());
        if report.added.is_empty() {
            toasts.info(format!("nothing new in {name}: {report}"));
            return;
        }
        self.store.tracking_mut().mark_edited();
        self.write_order.prioritize(Doc::Store);
        toasts.info(format!("imported {name}: {report}"));
        if !report.unknown_records.is_empty() {
            let named: Vec<&str> = report
                .unknown_records
                .keys()
                .take(3)
                .map(String::as_str)
                .collect();
            let more = report.unknown_records.len().saturating_sub(named.len());
            let rest = if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            };
            toasts.error(format!(
                "{} record(s) in {name} are unknown to the database (imported anyway): {}{rest}",
                report.unknown_records.len(),
                named.join(", ")
            ));
        }
    }

    /// An add, export, or import on the campaign's blueprint list or
    /// illusion collection; a changed document joins the write order.
    fn perform_crafting(&mut self, request: crafting::Request, toasts: &mut Toasts) {
        let mut files = CraftingFiles {
            blueprints: &mut self.blueprints,
            illusions: &mut self.illusions,
        };
        if let Some(doc) = crafting::perform(
            request,
            &mut files,
            &self.game,
            &self.campaign,
            now(),
            toasts,
        ) {
            self.write_order.prioritize(doc);
            if doc == Doc::Blueprints {
                self.sync_blueprints(toasts);
            }
        }
    }

    /// Sets a character's iron bits; the model is edited only when the
    /// character is writable, and the toast says so otherwise.
    fn set_money(&mut self, slot: CharacterSlot, money: u32, toasts: &mut Toasts) {
        let Some(doc) = self
            .characters
            .get_mut(slot.value())
            .and_then(CharacterEntry::doc_mut)
        else {
            return;
        };
        match doc.file_mut() {
            Ok(file) => match file.character_info_mut() {
                Some(info) if info.money != money => {
                    info.money = money;
                    doc.tracking_mut().mark_edited();
                    self.write_order.prioritize(Doc::Character(slot));
                }
                Some(_) | None => {}
            },
            Err(error) => toasts.error(error.to_string()),
        }
    }

    /// A confirmed reset: the rules come from the loaded game data,
    /// the character is edited through its document — so autosave,
    /// backup-first, and the guard handle the write — and the report
    /// is toasted. A refusal leaves the character untouched.
    fn respec(&mut self, slot: CharacterSlot, reset: Reset, toasts: &mut Toasts) {
        let label = self.doc_label(Doc::Character(slot));
        let rules = match RespecRules::load(&self.game) {
            Ok(rules) => rules,
            Err(error) => {
                toasts.error(format!("cannot reset {reset}: {error}"));
                return;
            }
        };
        let Some(doc) = self
            .characters
            .get_mut(slot.value())
            .and_then(CharacterEntry::doc_mut)
        else {
            return;
        };
        let file = match doc.file_mut() {
            Ok(file) => file,
            Err(error) => {
                toasts.error(error.to_string());
                return;
            }
        };
        match reset.apply(file, &rules) {
            Ok(report) if report.is_noop() => {
                toasts.info(format!("{label}: {report}; nothing to change"));
            }
            Ok(report) => {
                doc.tracking_mut().mark_edited();
                self.write_order.prioritize(Doc::Character(slot));
                toasts.info(format!("{label}: {report}"));
            }
            Err(error) => toasts.error(format!("{label}: reset {reset} refused: {error}")),
        }
    }

    fn mark_edited(&mut self, doc: Doc) {
        match doc {
            Doc::Stash => self.stash.tracking_mut().mark_edited(),
            Doc::Store => self.store.tracking_mut().mark_edited(),
            Doc::Reagents => self.reagents.mark_edited(),
            Doc::Blueprints => self.blueprints.mark_edited(),
            Doc::Illusions => self.illusions.mark_edited(),
            Doc::Character(slot) => {
                if let Some(doc) = self
                    .characters
                    .get_mut(slot.value())
                    .and_then(CharacterEntry::doc_mut)
                {
                    doc.tracking_mut().mark_edited();
                }
            }
        }
    }

    /// A document as the toasts and the conflict modal name it: a
    /// character by name, the rest by role.
    fn doc_label(&self, doc: Doc) -> String {
        match doc {
            Doc::Stash | Doc::Store | Doc::Reagents | Doc::Blueprints | Doc::Illusions => {
                doc.to_string()
            }
            Doc::Character(slot) => self.characters.get(slot.value()).map_or_else(
                || doc.to_string(),
                |entry| format!("character {}", entry.label()),
            ),
        }
    }

    /// Footprints are read from the memo without resolving, so every
    /// item in the documents a move touches is resolved first.
    fn warm_for(&mut self, mv: Move) {
        self.warm_doc(mv.source.doc());
        self.warm_doc(mv.target.doc());
    }

    fn warm_doc(&mut self, doc: Doc) {
        match doc {
            Doc::Stash => {
                let items = self
                    .stash
                    .stash()
                    .tabs
                    .iter()
                    .flat_map(|tab| tab.items.iter().map(|placed| &placed.item));
                self.facts.warm_all(&self.game, items);
            }
            Doc::Store => {
                let items = self
                    .store
                    .store()
                    .items()
                    .iter()
                    .map(grimvault_core::store::StoredItem::item);
                self.facts.warm_all(&self.game, items);
            }
            Doc::Reagents => {
                let entries = self
                    .reagents
                    .doc()
                    .map(|doc| {
                        doc.storage()
                            .entries
                            .iter()
                            .map(panes::reagents::entry_item)
                    })
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                self.facts.warm_all(&self.game, &entries);
            }
            Doc::Blueprints | Doc::Illusions => {}
            Doc::Character(slot) => {
                let Some(file) = self
                    .characters
                    .get(slot.value())
                    .and_then(CharacterEntry::doc)
                    .map(CharacterDoc::file)
                else {
                    return;
                };
                let sacks = file
                    .inventory()
                    .into_iter()
                    .flat_map(grimvault_core::gdc::Inventory::sacks)
                    .flat_map(|sack| sack.items.iter().map(|placed| &placed.item));
                let tabs = file
                    .stash()
                    .into_iter()
                    .flat_map(|stash| stash.tabs.iter())
                    .flat_map(|tab| tab.items.iter().map(|placed| &placed.item));
                self.facts.warm_all(&self.game, sacks.chain(tabs));
            }
        }
    }

    fn pending(&self) -> Pending {
        if self
            .docs()
            .into_iter()
            .any(|doc| self.edits(doc) == Edits::Unsaved)
        {
            Pending::Edits
        } else {
            Pending::Nothing
        }
    }

    fn gate(&self) -> Gate {
        if self.conflicts.is_empty() {
            Gate::Open
        } else {
            Gate::Suspended
        }
    }

    fn activity(&self, ctx: &egui::Context) -> Activity {
        if self.drag.is_some() || ctx.input(|input| input.pointer.any_down()) {
            Activity::Busy
        } else {
            Activity::Idle
        }
    }

    /// Autosave: 600 ms after the last edit with no gesture in flight,
    /// the dirty documents are written destination-first.
    fn drive_autosave(&mut self, ctx: &egui::Context, toasts: &mut Toasts) {
        let now = Instant::now();
        match self
            .autosave
            .observe(self.pending(), self.activity(ctx), self.gate(), now)
        {
            Verdict::Idle => {}
            Verdict::Wait(wait) => ctx.request_repaint_after(wait),
            Verdict::Flush => match self.flush(toasts) {
                Ok(()) => self.autosave.flushed(),
                Err(error) => {
                    toasts.error(format!("autosave failed: {error}"));
                    let wait = self.autosave.flush_failed(Instant::now());
                    ctx.request_repaint_after(wait);
                }
            },
        }
    }

    fn flush(&mut self, toasts: &mut Toasts) -> Result<(), SaveError> {
        for doc in self.write_order.docs() {
            if self.edits(doc) == Edits::Unsaved {
                match self.save(doc)? {
                    SaveOutcome::Saved {
                        backup: Some(backup),
                    } => {
                        toasts.info(format!(
                            "saved the {}; backup at {}",
                            self.doc_label(doc),
                            backup.display()
                        ));
                    }
                    SaveOutcome::Saved { backup: None } => {}
                    SaveOutcome::Conflict => self.push_conflict(doc),
                }
            }
        }
        Ok(())
    }

    /// Unsaved edits at exit are written unless an external change is
    /// pending a decision — then nothing is overwritten. The view
    /// state is written regardless: it is a convenience, not data.
    fn flush_on_exit(&mut self) {
        if self.gate() == Gate::Open {
            let mut discard = Toasts::default();
            if let Err(error) = self.flush(&mut discard) {
                eprintln!("grim-vault: final save failed: {error}");
            }
        }
        let current = self.ui_snapshot();
        self.ui_state.flush(current);
    }

    /// Writes one document; an absent or unusable shared file, or an
    /// unreadable character, has nothing to write and never has edits
    /// to flush.
    fn save(&mut self, doc: Doc) -> Result<SaveOutcome, SaveError> {
        const NOTHING: Result<SaveOutcome, SaveError> = Ok(SaveOutcome::Saved { backup: None });
        match doc {
            Doc::Stash => self.stash.save(),
            Doc::Store => self.store.save(),
            Doc::Reagents => self.reagents.save(),
            Doc::Blueprints => self.blueprints.save(),
            Doc::Illusions => self.illusions.save(),
            Doc::Character(slot) => self.character_mut(slot).map_or(NOTHING, CharacterDoc::save),
        }
    }

    fn character(&self, slot: CharacterSlot) -> Option<&CharacterEntry> {
        self.characters.get(slot.value())
    }

    fn character_mut(&mut self, slot: CharacterSlot) -> Option<&mut CharacterDoc> {
        self.characters
            .get_mut(slot.value())
            .and_then(CharacterEntry::doc_mut)
    }

    fn edits(&self, doc: Doc) -> Edits {
        match doc {
            Doc::Stash => self.stash.tracking().edits(),
            Doc::Store => self.store.tracking().edits(),
            Doc::Reagents => self.reagents.edits(),
            Doc::Blueprints => self.blueprints.edits(),
            Doc::Illusions => self.illusions.edits(),
            Doc::Character(slot) => self
                .character(slot)
                .map_or(Edits::Saved, CharacterEntry::edits),
        }
    }

    fn stamp(&self, doc: Doc) -> Option<FileStamp> {
        match doc {
            Doc::Stash => self.stash.tracking().stamp(),
            Doc::Store => self.store.tracking().stamp(),
            Doc::Reagents => self.reagents.stamp(),
            Doc::Blueprints => self.blueprints.stamp(),
            Doc::Illusions => self.illusions.stamp(),
            Doc::Character(slot) => self.character(slot).and_then(CharacterEntry::stamp),
        }
    }

    fn path(&self, doc: Doc) -> Option<&std::path::Path> {
        match doc {
            Doc::Stash => Some(self.stash.path()),
            Doc::Store => Some(self.store.path()),
            Doc::Reagents => Some(self.reagents.path()),
            Doc::Blueprints => Some(self.blueprints.path()),
            Doc::Illusions => Some(self.illusions.path()),
            Doc::Character(slot) => self.character(slot).map(CharacterEntry::path),
        }
    }

    fn reload(&mut self, doc: Doc) -> Result<(), ReloadError> {
        match doc {
            Doc::Stash => self.stash.reload()?,
            Doc::Store => self.store.reload()?,
            Doc::Reagents => self.reagents.reload()?,
            Doc::Blueprints => self.blueprints.reload()?,
            Doc::Illusions => self.illusions.reload()?,
            Doc::Character(slot) => {
                if let Some(entry) = self.characters.get_mut(slot.value()) {
                    entry.reload()?;
                }
            }
        }
        self.revalidate_selection();
        Ok(())
    }

    fn keep_mine(&mut self, doc: Doc) {
        match doc {
            Doc::Stash => self.stash.tracking_mut().keep_mine(),
            Doc::Store => self.store.tracking_mut().keep_mine(),
            Doc::Reagents => self.reagents.keep_mine(),
            Doc::Blueprints => self.blueprints.keep_mine(),
            Doc::Illusions => self.illusions.keep_mine(),
            Doc::Character(slot) => {
                if let Some(doc) = self.character_mut(slot) {
                    doc.tracking_mut().keep_mine();
                }
            }
        }
    }

    fn push_conflict(&mut self, doc: Doc) {
        if !self.conflicts.contains(&doc) {
            self.conflicts.push(doc);
        }
    }

    /// The guard: every queued poll is evidence; a change believed
    /// (two identical polls) reloads a clean document and raises the
    /// modal for a dirty one. Nothing moves under a gesture.
    fn drive_refresh(&mut self, ctx: &egui::Context, toasts: &mut Toasts) {
        let polls = match &self.watcher {
            Ok(watcher) => watcher.drain(),
            Err(_) => return,
        };
        if polls.is_empty() {
            return;
        }
        if self.activity(ctx) == Activity::Busy {
            return;
        }
        let mut settled: Vec<Doc> = Vec::new();
        let docs = self.docs();
        for poll in &polls {
            for (path, seen) in &poll.stamps {
                for &doc in &docs {
                    if Some(path.as_path()) != self.path(doc) {
                        continue;
                    }
                    let observation = self.refresh.observe(path, *seen, self.stamp(doc));
                    match observation {
                        Observation::Settled => {
                            if !settled.contains(&doc) {
                                settled.push(doc);
                            }
                        }
                        Observation::Unchanged
                        | Observation::Unreachable
                        | Observation::Settling => {}
                    }
                }
            }
        }
        for doc in settled {
            match self.edits(doc) {
                Edits::Unsaved => self.push_conflict(doc),
                Edits::Saved => {
                    let outcome = self.reload(doc);
                    self.forget_refresh(doc);
                    let label = self.doc_label(doc);
                    match outcome {
                        Ok(()) => {
                            toasts.info(format!("reloaded the {label}: it changed on disk"));
                            self.carry_out_orders(Scope::Doc(doc), toasts);
                        }
                        Err(error) => toasts.error(format!(
                            "the {label} changed on disk but could not be reloaded: {error}"
                        )),
                    }
                }
            }
        }
    }

    fn forget_refresh(&mut self, doc: Doc) {
        if let Some(path) = self.path(doc).map(std::path::Path::to_path_buf) {
            self.refresh.forget(&path);
        }
    }

    /// A decision is required: Esc and outside clicks are ignored and
    /// autosave stays suspended until one is made.
    fn show_conflict_modal(&mut self, ctx: &egui::Context, theme: &Theme, toasts: &mut Toasts) {
        if self.conflicts.is_empty() {
            return;
        }
        let names: Vec<String> = self
            .conflicts
            .iter()
            .map(|doc| self.doc_label(*doc))
            .collect();
        let mut reload = false;
        let mut keep = false;
        egui::Modal::new(Id::new("external-change")).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.label(theme.heading("Changed on disk"));
            ui.label(format!(
                "The game (or another tool) changed the {} on disk while you have unsaved edits here. \
                 Saving is paused until you choose.",
                names.join(" and ")
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                reload = ui.button("Reload from disk (discard my edits)").clicked();
                keep = ui.button("Keep mine (back up theirs, then overwrite)").clicked();
            });
        });
        if reload {
            let mut reloaded = Vec::new();
            for doc in std::mem::take(&mut self.conflicts) {
                let label = self.doc_label(doc);
                match self.reload(doc) {
                    Ok(()) => {
                        self.forget_refresh(doc);
                        toasts.info(format!("reloaded the {label} from disk"));
                        reloaded.push(doc);
                    }
                    Err(error) => {
                        self.push_conflict(doc);
                        toasts.error(format!("could not reload the {label}: {error}"));
                    }
                }
            }
            for doc in reloaded {
                self.carry_out_orders(Scope::Doc(doc), toasts);
            }
        } else if keep {
            for doc in std::mem::take(&mut self.conflicts) {
                self.keep_mine(doc);
                self.forget_refresh(doc);
            }
            toasts
                .info("keeping your edits; the external version is backed up before the next save");
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "footprints are a handful of cells"
)]
fn cells(n: i32) -> f32 {
    n as f32
}

fn backup_label(backup: Backup) -> &'static str {
    match backup {
        Backup::Armed => "armed",
        Backup::Taken => "taken",
    }
}

fn optional_backup_label<D: Document>(file: &Optional<D>) -> &'static str {
    match file {
        Optional::Open(doc) => backup_label(doc.tracking().backup()),
        Optional::Absent { .. } => "no file",
        Optional::Failed { .. } => "read-only",
    }
}

/// `taken/editable` across the characters, so the bar stays one line
/// however many there are.
fn characters_backup_label(characters: &[CharacterEntry]) -> String {
    let editable: Vec<&CharacterDoc> = characters
        .iter()
        .filter_map(CharacterEntry::doc)
        .filter(|doc| doc.writable() == crate::documents::Writable::Yes)
        .collect();
    let taken = editable
        .iter()
        .filter(|doc| doc.tracking().backup() == Backup::Taken)
        .count();
    format!("{taken}/{} taken", editable.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_order_puts_the_latest_destination_first_and_keeps_the_rest() {
        let sif = Doc::Character(CharacterSlot::new(0));
        let mut order = WriteOrder::new([Doc::Store, Doc::Stash, Doc::Reagents, sif]);
        assert_eq!(order.docs(), [Doc::Store, Doc::Stash, Doc::Reagents, sif]);
        order.prioritize(Doc::Stash);
        assert_eq!(order.docs(), [Doc::Stash, Doc::Store, Doc::Reagents, sif]);
        order.prioritize(sif);
        assert_eq!(order.docs(), [sif, Doc::Stash, Doc::Store, Doc::Reagents]);
        order.prioritize(Doc::Store);
        assert_eq!(order.docs(), [Doc::Store, sif, Doc::Stash, Doc::Reagents]);
        order.prioritize(Doc::Store);
        assert_eq!(order.docs(), [Doc::Store, sif, Doc::Stash, Doc::Reagents]);
        order.prioritize(Doc::Character(CharacterSlot::new(9)));
        assert_eq!(order.docs(), [Doc::Store, sif, Doc::Stash, Doc::Reagents]);
    }

    #[test]
    fn the_campaign_docs_are_the_shared_docs_less_the_store() {
        assert!(SHARED_DOCS.contains(&Doc::Store));
        assert!(!CAMPAIGN_DOCS.contains(&Doc::Store));
        assert!(CAMPAIGN_DOCS.iter().all(|doc| SHARED_DOCS.contains(doc)));
        assert_eq!(CAMPAIGN_DOCS.len() + 1, SHARED_DOCS.len());
    }

    #[test]
    fn alt_or_the_command_key_asks_for_a_copy() {
        assert_eq!(mode_of(egui::Modifiers::NONE), Mode::Move);
        assert_eq!(mode_of(egui::Modifiers::ALT), Mode::Copy);
        assert_eq!(mode_of(egui::Modifiers::COMMAND), Mode::Copy);
        assert_eq!(mode_of(egui::Modifiers::SHIFT), Mode::Move);
    }

    #[test]
    fn shift_or_either_copy_key_makes_a_right_click_copy() {
        assert_eq!(right_click_mode(egui::Modifiers::NONE), Mode::Move);
        assert_eq!(right_click_mode(egui::Modifiers::SHIFT), Mode::Copy);
        assert_eq!(right_click_mode(egui::Modifiers::ALT), Mode::Copy);
        assert_eq!(right_click_mode(egui::Modifiers::COMMAND), Mode::Copy);
    }
}

/// Transient outcome notifications, and the last error for the status
/// bar.
#[derive(Default)]
pub struct Toasts {
    entries: Vec<Toast>,
    last_error: Option<String>,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: Instant,
}

#[derive(Clone, Copy)]
enum ToastKind {
    Info,
    Error,
}

impl ToastKind {
    fn lifetime(self) -> Duration {
        match self {
            Self::Info => Duration::from_secs(4),
            Self::Error => Duration::from_secs(8),
        }
    }
}

const TOAST_STACK: usize = 6;

impl Toasts {
    pub fn info(&mut self, text: impl Into<String>) {
        self.push(text.into(), ToastKind::Info);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.last_error = Some(text.clone());
        self.push(text, ToastKind::Error);
    }

    fn push(&mut self, text: String, kind: ToastKind) {
        self.entries.push(Toast {
            text,
            kind,
            born: Instant::now(),
        });
        if self.entries.len() > TOAST_STACK {
            self.entries.remove(0);
        }
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn show(&mut self, ctx: &egui::Context, palette: &Palette) {
        let now = Instant::now();
        self.entries
            .retain(|toast| now.duration_since(toast.born) < toast.kind.lifetime());
        if self.entries.is_empty() {
            return;
        }
        egui::Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
            .order(Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for toast in &self.entries {
                    let (edge, ink) = match toast.kind {
                        ToastKind::Info => (palette.accent_dim, palette.text_strong),
                        ToastKind::Error => (palette.error, palette.error),
                    };
                    egui::Frame::popup(ui.style())
                        .fill(palette.surface_raised)
                        .stroke(Stroke::new(1.0, edge))
                        .show(ui, |ui| {
                            ui.set_max_width(440.0);
                            ui.label(RichText::new(&toast.text).color(ink));
                        });
                }
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
