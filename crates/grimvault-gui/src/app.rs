//! The shell: the phases, the world the Ready phase holds, and the two
//! drivers — autosave and the external-change guard — that run from
//! eframe's `logic` hook. That hook runs even while the window is
//! hidden, so a covered window still saves and still notices the
//! game's writes (a stall tq-univault paid for in its paint-driven
//! refresh).

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{
    Align2, Color32, CornerRadius, Id, LayerId, Order, Rect, RichText, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use grimvault_core::gamedata::GameData;
use grimvault_core::store::Timestamp;
use univault_ui::theme::{Palette, Theme};

use crate::autosave::{Activity, Autosave, AutosaveState, Gate, Pending, Verdict};
use crate::documents::{
    Backup, CharacterEntry, Doc, Edits, FileStamp, SaveError, SaveOutcome, StashDoc,
    StashOpenError, StoreDoc, StoreOpenError,
};
use crate::drag::{self, Applied, DragState, DropTarget, Fit, Move};
use crate::facts::FactsCache;
use crate::grid::CELL_PX;
use crate::icons::{Icon, IconCache};
use crate::loader::{self, LoadFailure, LoadJob, LoadOutcome, LoadReport, LoadedWorld, WorldPaths};
use crate::panes::character::CharacterView;
use crate::panes::stash::StashView;
use crate::panes::store::StoreView;
use crate::panes::{self, DragFrame, PaneCtx};
use crate::settings::{self, ConfigDir, Settings};
use crate::setup::{DirProblem, GameDir, SaveDir, SetupState};
use crate::theme::FITS;
use crate::watch::{Observation, RefreshTracker, Watcher};

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
            Ok(paths) => Phase::Loading(loader::start(paths, ctx.clone())),
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
        store: config.store_file(),
    })
}

fn settings_of(paths: &WorldPaths) -> Settings {
    Settings {
        game_dir: paths.game.path().to_path_buf(),
        save_dir: paths.save.path().to_path_buf(),
    }
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
            Phase::Ready(world) => {
                world.show(ui, &self.theme, &mut self.toasts);
                None
            }
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
            if let Err(error) = settings::save(config, &state.settings()) {
                toasts.error(format!("settings were not saved: {error}"));
            }
            let paths = WorldPaths {
                game,
                save,
                store: config.store_file(),
            };
            next = Some(Phase::Loading(loader::start(paths, ui.ctx().clone())));
        }
        ui.add_space(8.0);
        ui.label(theme.path_text(format!(
            "settings and the vault store live in {}",
            config.path().display()
        )));
    });
    next
}

fn dir_field(ui: &mut Ui, label: &str, field: &mut String, candidates: &[PathBuf], theme: &Theme) {
    ui.label(theme.section(label));
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(field).desired_width(560.0));
        if ui.button("Browse…").clicked() {
            let start = PathBuf::from(field.as_str());
            let dialog = if start.is_dir() {
                rfd::FileDialog::new().set_directory(&start)
            } else {
                rfd::FileDialog::new()
            };
            if let Some(picked) = dialog.pick_folder() {
                *field = picked.display().to_string();
            }
        }
    });
    if !candidates.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.weak("found here:");
            for candidate in candidates {
                let text = candidate.display().to_string();
                if ui.small_button(&text).clicked() {
                    *field = text;
                }
            }
        });
    }
}

fn verdict_line(ui: &mut Ui, problem: Option<&DirProblem>, ok: &str, theme: &Theme) {
    match problem {
        None => ui.colored_label(FITS, ok),
        Some(problem) => ui.colored_label(theme.palette.error, problem.to_string()),
    };
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
                ui.ctx(),
                toasts,
            ))));
        }
        Some(LoadOutcome::Failed(failure)) => {
            return Some(Phase::Failed(LoadFailed {
                failure,
                settings: settings_of(&job.paths),
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
                    ui.colored_label(FITS, "✓");
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
    back.then(|| Phase::Setup(SetupState::discover(Some(&settings_of(&job.paths)), None)))
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

/// Which document a crash between the two writes of a move may leave
/// duplicated rather than lost: the destination is written first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteOrder {
    StoreFirst,
    StashFirst,
}

/// Why a reload from disk failed.
#[derive(Debug, thiserror::Error)]
enum ReloadError {
    #[error("{0}")]
    Stash(#[from] StashOpenError),
    #[error("{0}")]
    Store(#[from] StoreOpenError),
}

/// Everything the Ready phase holds.
pub struct World {
    paths: WorldPaths,
    game: GameData,
    report: LoadReport,
    stash: StashDoc,
    store: StoreDoc,
    characters: Vec<CharacterEntry>,
    facts: FactsCache,
    icons: IconCache,
    stash_view: StashView,
    store_view: StoreView,
    character_view: CharacterView,
    drag: Option<DragState>,
    autosave: Autosave,
    watcher: Result<Watcher, std::io::Error>,
    refresh: RefreshTracker,
    conflicts: Vec<Doc>,
    write_order: WriteOrder,
}

impl World {
    fn new(
        loaded: LoadedWorld,
        paths: WorldPaths,
        ctx: &egui::Context,
        toasts: &mut Toasts,
    ) -> Self {
        let watcher = Watcher::start(ctx.clone());
        match &watcher {
            Ok(watcher) => watcher.watch(vec![
                loaded.stash.path().to_path_buf(),
                loaded.store.path().to_path_buf(),
            ]),
            Err(error) => toasts.error(format!(
                "external-change watching is off: the watcher thread could not start ({error})"
            )),
        }
        Self {
            paths,
            game: loaded.game,
            report: loaded.report,
            stash: loaded.stash,
            store: loaded.store,
            characters: loaded.characters,
            facts: FactsCache::default(),
            icons: IconCache::default(),
            stash_view: StashView::default(),
            store_view: StoreView::default(),
            character_view: CharacterView::default(),
            drag: None,
            autosave: Autosave::default(),
            watcher,
            refresh: RefreshTracker::default(),
            conflicts: Vec::new(),
            write_order: WriteOrder::StoreFirst,
        }
    }

    fn show(&mut self, ui: &mut Ui, theme: &Theme, toasts: &mut Toasts) {
        let mut frame = DragFrame::default();
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui, theme, toasts));
        egui::Panel::bottom("characters")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| {
                let mut cx = PaneCtx {
                    game: &self.game,
                    facts: &mut self.facts,
                    icons: &mut self.icons,
                    palette: &theme.palette,
                    drag: self.drag.as_ref(),
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
                    drag: self.drag.as_ref(),
                };
                panes::store::show(
                    ui,
                    &self.store,
                    &mut self.store_view,
                    theme,
                    &mut cx,
                    &mut frame,
                );
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let mut cx = PaneCtx {
                game: &self.game,
                facts: &mut self.facts,
                icons: &mut self.icons,
                palette: &theme.palette,
                drag: self.drag.as_ref(),
            };
            panes::stash::show(
                ui,
                &self.stash,
                &mut self.stash_view,
                theme,
                &mut cx,
                &mut frame,
            );
        });
        self.show_conflict_modal(ui.ctx(), theme, toasts);
        self.finish_frame(ui.ctx(), frame, &theme.palette, toasts);
    }

    fn status_bar(&self, ui: &mut Ui, theme: &Theme, toasts: &Toasts) {
        ui.horizontal_wrapped(|ui| {
            ui.label(theme.path_text(format!("saves: {}", self.paths.save.path().display())));
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
                "backup-first: stash {}, store {}",
                backup_label(self.stash.backup()),
                backup_label(self.store.backup())
            ))
            .on_hover_text(
                "The first write of each file since it was loaded takes a grimvault-bak backup beside it; \
                 later autosaves of the same load reuse that backup.",
            );
            ui.separator();
            ui.weak(format!(
                "game data: {} layers, {} item archives",
                self.report.databases, self.report.item_archives
            ));
            if let Some(error) = toasts.last_error() {
                ui.separator();
                ui.colored_label(theme.palette.error, error);
            }
        });
    }

    /// Adopts a drag the panes began, paints the lifted item at the
    /// pointer, and commits or snaps back on release. Double-clicks
    /// are moves too.
    fn finish_frame(
        &mut self,
        ctx: &egui::Context,
        frame: DragFrame,
        palette: &Palette,
        toasts: &mut Toasts,
    ) {
        if self.drag.is_none()
            && let Some(source) = frame.double_click
        {
            let mv = drag::plan_double_click(source, self.stash_view.tab);
            self.perform(mv, toasts);
        }
        if self.drag.is_none() {
            self.drag = frame.begin;
        }
        let Some(state) = self.drag.clone() else {
            return;
        };
        self.paint_ghost(ctx, &state, palette);
        if ctx.input(|input| input.pointer.any_released()) {
            if let Some(candidate) = frame.candidate {
                match (candidate.target, candidate.fit) {
                    (target, Fit::Fits) => self.perform(drag::plan(state.source, target), toasts),
                    (DropTarget::StashCell { .. }, Fit::Blocked) => {
                        toasts.info("no room at that cell; snapped back");
                    }
                    (DropTarget::StashCell { tab, .. }, Fit::Unresolvable) => toasts.error(format!(
                        "tab {tab} holds items with unknown footprints; nothing can be placed there"
                    )),
                    (DropTarget::StashTab(_) | DropTarget::Store, Fit::Blocked | Fit::Unresolvable) => {}
                }
            }
            self.drag = None;
            ctx.request_repaint();
        }
    }

    fn paint_ghost(&mut self, ctx: &egui::Context, state: &DragState, palette: &Palette) {
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
    }

    /// Every move goes through `drag::apply`, hence through
    /// `grimvault_core::transfer`; the shell only marks what changed
    /// and remembers which document is the move's destination.
    fn perform(&mut self, mv: Move, toasts: &mut Toasts) {
        self.warm_for(mv);
        let now = Timestamp::from_unix_seconds(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        );
        let outcome = drag::apply(
            mv,
            self.stash.stash_mut(),
            self.store.store_mut(),
            &self.facts,
            now,
        );
        match outcome {
            Ok(Applied::Vaulted(id)) => {
                self.stash.mark_edited();
                self.store.mark_edited();
                self.write_order = WriteOrder::StoreFirst;
                let name = self
                    .store
                    .store()
                    .get(id)
                    .map(|stored| self.facts.facts(&self.game, stored.item()).display_name())
                    .unwrap_or_default();
                toasts.info(format!("vaulted {name} as stored item {id}"));
            }
            Ok(Applied::Placed(pos)) => {
                self.stash.mark_edited();
                self.store.mark_edited();
                self.write_order = WriteOrder::StashFirst;
                toasts.info(format!("placed in the stash at ({}, {})", pos.x, pos.y));
            }
            Ok(Applied::Rearranged(_)) => self.stash.mark_edited(),
            Ok(Applied::Unmoved) => {}
            Err(error) => toasts.error(error.to_string()),
        }
    }

    /// Footprints are read from the memo without resolving, so every
    /// item a move may touch is resolved first.
    fn warm_for(&mut self, mv: Move) {
        let stash_items = self
            .stash
            .stash()
            .tabs
            .iter()
            .flat_map(|tab| tab.items.iter().map(|placed| &placed.item));
        self.facts.warm_all(&self.game, stash_items);
        let stored = match mv {
            Move::PlaceAt { id, .. } | Move::PlaceFirstFit { id, .. } => Some(id),
            Move::Vault { .. }
            | Move::RearrangeAt { .. }
            | Move::RearrangeFirstFit { .. }
            | Move::Stay => None,
        };
        if let Some(stored) = stored.and_then(|id| self.store.store().get(id)) {
            self.facts.warm(&self.game, stored.item());
        }
    }

    fn pending(&self) -> Pending {
        match (self.stash.edits(), self.store.edits()) {
            (Edits::Saved, Edits::Saved) => Pending::Nothing,
            (Edits::Unsaved, _) | (_, Edits::Unsaved) => Pending::Edits,
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
        let order = match self.write_order {
            WriteOrder::StoreFirst => [Doc::Store, Doc::Stash],
            WriteOrder::StashFirst => [Doc::Stash, Doc::Store],
        };
        for doc in order {
            if self.edits(doc) == Edits::Unsaved {
                match self.save(doc)? {
                    SaveOutcome::Saved {
                        backup: Some(backup),
                    } => {
                        toasts.info(format!("saved the {doc}; backup at {}", backup.display()));
                    }
                    SaveOutcome::Saved { backup: None } => {}
                    SaveOutcome::Conflict => self.push_conflict(doc),
                }
            }
        }
        Ok(())
    }

    /// Unsaved edits at exit are written unless an external change is
    /// pending a decision — then nothing is overwritten.
    fn flush_on_exit(&mut self) {
        if self.gate() == Gate::Open {
            let mut discard = Toasts::default();
            if let Err(error) = self.flush(&mut discard) {
                eprintln!("grim-vault: final save failed: {error}");
            }
        }
    }

    fn save(&mut self, doc: Doc) -> Result<SaveOutcome, SaveError> {
        match doc {
            Doc::Stash => self.stash.save(),
            Doc::Store => self.store.save(),
        }
    }

    fn edits(&self, doc: Doc) -> Edits {
        match doc {
            Doc::Stash => self.stash.edits(),
            Doc::Store => self.store.edits(),
        }
    }

    fn stamp(&self, doc: Doc) -> Option<FileStamp> {
        match doc {
            Doc::Stash => self.stash.stamp(),
            Doc::Store => self.store.stamp(),
        }
    }

    fn path(&self, doc: Doc) -> &std::path::Path {
        match doc {
            Doc::Stash => self.stash.path(),
            Doc::Store => self.store.path(),
        }
    }

    fn reload(&mut self, doc: Doc) -> Result<(), ReloadError> {
        match doc {
            Doc::Stash => self.stash.reload()?,
            Doc::Store => self.store.reload()?,
        }
        Ok(())
    }

    fn keep_mine(&mut self, doc: Doc) {
        match doc {
            Doc::Stash => self.stash.keep_mine(),
            Doc::Store => self.store.keep_mine(),
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
        for poll in &polls {
            for (path, seen) in &poll.stamps {
                for doc in [Doc::Stash, Doc::Store] {
                    if path != self.path(doc) {
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
                Edits::Saved => match self.reload(doc) {
                    Ok(()) => {
                        let path = self.path(doc).to_path_buf();
                        self.refresh.forget(&path);
                        toasts.info(format!("reloaded the {doc}: it changed on disk"));
                    }
                    Err(error) => {
                        let path = self.path(doc).to_path_buf();
                        self.refresh.forget(&path);
                        toasts.error(format!(
                            "the {doc} changed on disk but could not be reloaded: {error}"
                        ));
                    }
                },
            }
        }
    }

    /// A decision is required: Esc and outside clicks are ignored and
    /// autosave stays suspended until one is made.
    fn show_conflict_modal(&mut self, ctx: &egui::Context, theme: &Theme, toasts: &mut Toasts) {
        if self.conflicts.is_empty() {
            return;
        }
        let names: Vec<String> = self.conflicts.iter().map(ToString::to_string).collect();
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
            for doc in std::mem::take(&mut self.conflicts) {
                let path = self.path(doc).to_path_buf();
                match self.reload(doc) {
                    Ok(()) => {
                        self.refresh.forget(&path);
                        toasts.info(format!("reloaded the {doc} from disk"));
                    }
                    Err(error) => {
                        self.push_conflict(doc);
                        toasts.error(format!("could not reload the {doc}: {error}"));
                    }
                }
            }
        } else if keep {
            for doc in std::mem::take(&mut self.conflicts) {
                self.keep_mine(doc);
                let path = self.path(doc).to_path_buf();
                self.refresh.forget(&path);
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
