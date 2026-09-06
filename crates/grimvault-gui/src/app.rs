//! The shell: the phases, the world the Ready phase holds, and the two
//! drivers — autosave and the external-change guard — that run from
//! eframe's `logic` hook. That hook runs even while the window is
//! hidden, so a covered window still saves and still notices the
//! game's writes (a stall tq-univault paid for in its paint-driven
//! refresh).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, LayerId, Order, Rect, RichText, Stroke, StrokeKind,
    Ui, pos2, vec2,
};
use grimvault_core::gamedata::GameData;
use grimvault_core::gds;
use grimvault_core::respec::{Reset, RespecRules};
use grimvault_core::store::Timestamp;
use univault_io::read_verified;
use univault_ui::theme::{Palette, Theme};

use crate::autosave::{Activity, Autosave, AutosaveState, Gate, Pending, Verdict};
use crate::crafting::{self, Blueprints, CraftingFiles, FormulasOpenError, IllusionCollection};
use crate::documents::{
    Backup, CharacterDoc, CharacterEntry, CharacterOpenError, CharacterSlot, Doc, Document, Edits,
    FileStamp, GstOpenError, Optional, ReagentDoc, Reagents, SaveError, SaveOutcome, StashDoc,
    StoreDoc, StoreOpenError,
};
use crate::drag::{
    self, Applied, Containers, DragSource, DragState, DropTarget, Fit, Landing, Mode, Move,
    OpenCharacter,
};
use crate::facts::FactsCache;
use crate::grid::CELL_PX;
use crate::icons::{Icon, IconCache};
use crate::loader::{
    self, LoadFailure, LoadJob, LoadOutcome, LoadReport, LoadedWorld, WorldPaths, open_shared,
};
use crate::panes::character::CharacterView;
use crate::panes::stash::StashView;
use crate::panes::store::StoreView;
use crate::panes::{self, DragFrame, PaneCtx};
use crate::settings::{self, ConfigDir, Settings};
use crate::setup::{DirProblem, GameDir, SaveDir, SetupState};
use crate::theme::FITS;
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

/// Everything the Ready phase holds.
pub struct World {
    paths: WorldPaths,
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
        if let Err(error) = &watcher {
            toasts.error(format!(
                "external-change watching is off: the watcher thread could not start ({error})"
            ));
        }
        for warning in loaded.warnings {
            toasts.error(warning);
        }
        let write_order = WriteOrder::new(SHARED_DOCS.into_iter().chain(
            (0..loaded.characters.len()).map(|slot| Doc::Character(CharacterSlot::new(slot))),
        ));
        let world = Self {
            paths,
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
            icons: IconCache::default(),
            stash_view: StashView::default(),
            store_view: StoreView::default(),
            character_view: CharacterView::default(),
            drag: None,
            autosave: Autosave::default(),
            watcher,
            refresh: RefreshTracker::default(),
            conflicts: Vec::new(),
            write_order,
        };
        world.rewatch();
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

    /// Swaps the shared files for another campaign's. Unsaved edits
    /// are written first, backup-first as ever; nothing changes when
    /// that write, a pending external change, or the open stands in
    /// the way.
    fn switch_campaign(&mut self, next: Campaign, toasts: &mut Toasts) {
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
        for warning in shared.warnings {
            toasts.error(warning);
        }
        self.rewatch();
        toasts.info(format!("showing the {} files", self.campaign));
    }

    /// Every document, in default write order.
    fn docs(&self) -> Vec<Doc> {
        SHARED_DOCS
            .into_iter()
            .chain((0..self.characters.len()).map(|slot| Doc::Character(CharacterSlot::new(slot))))
            .collect()
    }

    fn show(&mut self, ui: &mut Ui, theme: &Theme, toasts: &mut Toasts) {
        let mut frame = DragFrame::default();
        let mode = mode_of(ui.input(|input| input.modifiers));
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
                    drag: self.drag.as_ref(),
                    mode,
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
        let switch = egui::CentralPanel::default()
            .show(ui, |ui| {
                let mut cx = PaneCtx {
                    game: &self.game,
                    facts: &mut self.facts,
                    icons: &mut self.icons,
                    palette: &theme.palette,
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
        self.show_conflict_modal(ui.ctx(), theme, toasts);
        self.finish_frame(ui.ctx(), frame, mode, &theme.palette, toasts);
        if let Some(next) = switch {
            self.switch_campaign(next, toasts);
        }
    }

    fn status_bar(&self, ui: &mut Ui, theme: &Theme, toasts: &Toasts) {
        ui.horizontal_wrapped(|ui| {
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
            ui.weak(format!(
                "game data: {} layers, {} item archives, {} mods",
                self.report.databases, self.report.item_archives, self.report.mods
            ));
            if let Some(error) = toasts.last_error() {
                ui.separator();
                ui.colored_label(theme.palette.error, error);
            }
        });
    }

    /// Adopts a drag the panes began, paints the lifted item at the
    /// pointer, and commits or snaps back on release. Double-clicks
    /// are moves too, and an iron-bits edit or a confirmed reset is
    /// applied here.
    fn finish_frame(
        &mut self,
        ctx: &egui::Context,
        frame: DragFrame,
        mode: Mode,
        palette: &Palette,
        toasts: &mut Toasts,
    ) {
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
        if self.drag.is_none()
            && let Some(source) = frame.double_click
        {
            let mv = drag::double_click(source, mode, self.stash_view.tab);
            self.perform(mv, toasts);
        }
        if self.drag.is_none() {
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
        self.warm_for(mv);
        let now = now();
        let World {
            campaign,
            stash,
            store,
            reagents,
            characters,
            facts,
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
        let carried = drag::peek(&containers, mv.source)
            .ok()
            .map(|(item, _)| item);
        let outcome = drag::apply(mv, &mut containers, &*facts, now);
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
    /// pending a decision — then nothing is overwritten.
    fn flush_on_exit(&mut self) {
        if self.gate() == Gate::Open {
            let mut discard = Toasts::default();
            if let Err(error) = self.flush(&mut discard) {
                eprintln!("grim-vault: final save failed: {error}");
            }
        }
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
                        Ok(()) => toasts.info(format!("reloaded the {label}: it changed on disk")),
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
            for doc in std::mem::take(&mut self.conflicts) {
                let label = self.doc_label(doc);
                match self.reload(doc) {
                    Ok(()) => {
                        self.forget_refresh(doc);
                        toasts.info(format!("reloaded the {label} from disk"));
                    }
                    Err(error) => {
                        self.push_conflict(doc);
                        toasts.error(format!("could not reload the {label}: {error}"));
                    }
                }
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
