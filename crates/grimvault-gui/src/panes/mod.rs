//! The Ready phase's surfaces: the stash with the component /
//! crafting-material storage beside it, the store, and the
//! characters, over one shared cell-grid renderer. Each pane reads the
//! documents, paints, and *reports* what the user did this frame
//! through [`DragFrame`]; the app turns those reports into moves after
//! every pane has drawn, so no pane ever mutates a document.

pub mod character;
pub mod crafting;
pub mod inspector;
pub mod reagents;
pub mod stash;
pub mod store;

use std::path::PathBuf;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use grimvault_core::block::StashTab;
use grimvault_core::gamedata::{GameData, Rarity};
use grimvault_core::gdc::Sack;
use grimvault_core::item::Item;
use grimvault_core::respec::Reset;
use grimvault_core::settings::{
    AutoMoveTab, BlueprintSync, BulkDuplicates, ReagentSync, Settings, StandingOrder,
};
use grimvault_core::socket::Socket;
use grimvault_core::transfer::{Footprints, ItemIndex};
use univault_engine::grid::CellRect;
use univault_engine::ids::GridPos;
use univault_ui::theme::{Palette, Theme};

use crate::automove::OrderRequest;
use crate::badges::{Badge, paint_badge};
use crate::documents::CharacterSlot;
use crate::drag::{self, Container, DragSource, DragState, DropTarget, Fit, Mode};
use crate::facts::FactsCache;
use crate::grid::{
    CELL_PX, FootprintSource, GridGeometry, cells_of, footprint_or_unit, native_size, occupant_at,
};
use crate::icons::{Icon, IconCache};
use crate::theme::{BLOCKED, FITS, UNKNOWN_RARITY, rarity_color};

/// What every pane needs to paint.
pub struct PaneCtx<'a> {
    pub game: &'a GameData,
    pub facts: &'a mut FactsCache,
    pub icons: &'a mut IconCache,
    pub palette: &'a Palette,
    /// The standing orders, for the toggles that show them.
    pub settings: &'a Settings,
    pub drag: Option<&'a DragState>,
    /// Whether a drop this frame would move or copy, from the
    /// modifier keys held.
    pub mode: Mode,
}

/// Where a drop would land this frame, as the surface under the
/// pointer computed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropCandidate {
    pub target: DropTarget,
    pub fit: Fit,
}

/// What the panes reported this frame.
#[derive(Default)]
pub struct DragFrame {
    pub begin: Option<DragState>,
    pub candidate: Option<DropCandidate>,
    pub double_click: Option<DragSource>,
    /// An item right-clicked: a quick move between the game and the
    /// vault, a copy with Shift held.
    pub right_click: Option<DragSource>,
    /// A game-side container the user selected this frame — its tab,
    /// or the character whose sack now shows.
    pub touched: Option<Container>,
    /// An item clicked without dragging: the inspector's selection.
    pub select: Option<DragSource>,
    /// A socket edit the inspector asked for.
    pub socket: Option<crate::sockets::Request>,
    /// A character's iron bits, as the user set them.
    pub set_money: Option<(CharacterSlot, u32)>,
    /// A GD Stash export the user picked to import into the store.
    pub import_gds: Option<PathBuf>,
    /// A reset of a character's attributes or masteries the user
    /// confirmed.
    pub respec: Option<(CharacterSlot, Reset)>,
    /// An add, export, or import on a crafting list.
    pub crafting: Option<crate::crafting::Request>,
    /// A tab nominated for a standing order, or withdrawn from it.
    pub standing_order: Option<OrderRequest>,
    /// The component-storage sync switched on or off.
    pub reagent_sync: Option<ReagentSync>,
    /// The learned-blueprint sync switched on or off.
    pub blueprint_sync: Option<BlueprintSync>,
    /// A whole container moved or copied into the store, or emptied
    /// — the last only once confirmed.
    pub bulk: Option<BulkRequest>,
    /// The bulk-duplicates rule switched.
    pub bulk_duplicates: Option<BulkDuplicates>,
}

/// What a container's header buttons ask for: every item into the
/// store — moved or copied, under the bulk-duplicates rule — or the
/// container emptied, which touches no store. The two are distinct
/// variants so the transfer path can never be handed a clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkOp {
    Transfer(Mode),
    Clear,
}

impl BulkOp {
    /// The verb as a toast reports it done.
    #[must_use]
    pub fn done(self) -> &'static str {
        match self {
            Self::Transfer(Mode::Move) => "moved",
            Self::Transfer(Mode::Copy) => "copied",
            Self::Clear => "deleted",
        }
    }
}

impl std::fmt::Display for BulkOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Transfer(Mode::Move) => "move all",
            Self::Transfer(Mode::Copy) => "copy all",
            Self::Clear => "delete all",
        })
    }
}

/// A bulk operation on one game-side container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BulkRequest {
    pub container: Container,
    pub op: BulkOp,
}

/// A "Delete all" awaiting the user's confirmation: the tab and how
/// many items it held when asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingClear {
    pub container: Container,
    pub count: usize,
}

/// Whether a confirmation is still up after this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirmation {
    Pending,
    Settled,
}

/// Why an editable-looking character takes no edits.
pub const READ_ONLY_WHY: &str = "This character is read-only: a block of its player.gdc is not typed, so nothing before it \
     can be edited.";

const AUTO_MOVE_WHY: &str = "Every item in this tab is moved into the vault store whenever the app loads or \
     reloads the file — after the game writes it, too — leaving the tab empty. Under the store's \
     \"Skip duplicates in bulk moves\" an item the store already holds under the same record and \
     roll seed is left in place; stacks are never treated as duplicates.";
const PURGE_WHY: &str = "Whenever the app loads or reloads this tab — after the game writes it, too — every \
     item whose record and roll seed the vault store already holds is deleted from the tab, \
     whatever the store's duplicates setting says. Stacks are never duplicates. The file is \
     backed up once per load before the first write.";
const MOVE_ALL_WHY: &str = "Moves every item here into the vault store through the same moves a drag makes. Under the \
     store's \"Skip duplicates in bulk moves\" an item the store already holds under the same \
     record and roll seed stays here.";
const COPY_ALL_WHY: &str = "Copies every item here into the vault store and leaves this container as it is. The \
     store's \"Skip duplicates in bulk moves\" applies.";
const DELETE_ALL_WHY: &str =
    "Deletes every item here after a confirmation. Nothing enters the vault store.";
const EMPTY_WHY: &str = "There is nothing here to act on.";

/// The "Move all" and "Copy all" buttons on a container's header,
/// reporting a click through the frame. `container` is `None` for a
/// read-only character; that and an empty container disable the
/// buttons, each saying why on hover.
pub fn bulk_buttons(
    ui: &mut Ui,
    container: Option<Container>,
    count: usize,
    frame: &mut DragFrame,
) {
    for (mode, label, why) in [
        (Mode::Move, "Move all to vault", MOVE_ALL_WHY),
        (Mode::Copy, "Copy all to vault", COPY_ALL_WHY),
    ] {
        if let Some(container) = header_button(ui, container, count, label, why) {
            frame.bulk = Some(BulkRequest {
                container,
                op: BulkOp::Transfer(mode),
            });
        }
    }
}

/// The "Delete all…" button on a stash tab's header: a click asks for
/// confirmation rather than acting, so the request comes back to the
/// caller to hold until [`confirm_clear`] settles it.
pub fn clear_button(
    ui: &mut Ui,
    container: Option<Container>,
    count: usize,
) -> Option<PendingClear> {
    header_button(ui, container, count, "Delete all…", DELETE_ALL_WHY)
        .map(|container| PendingClear { container, count })
}

fn header_button(
    ui: &mut Ui,
    container: Option<Container>,
    count: usize,
    label: &str,
    why: &str,
) -> Option<Container> {
    let disabled_why = match container {
        None => READ_ONLY_WHY,
        Some(_) => EMPTY_WHY,
    };
    let enabled = container.is_some() && count > 0;
    let clicked = ui
        .add_enabled(enabled, egui::Button::new(label))
        .on_hover_text(why)
        .on_disabled_hover_text(disabled_why)
        .clicked();
    clicked.then_some(container).flatten()
}

/// The confirmation a clear needs before it is reported: nothing is
/// edited until the user confirms, and Esc, a click outside, or
/// Cancel drops the request. `label` names the tab as the heading
/// reads it.
pub fn confirm_clear(
    ui: &Ui,
    pending: PendingClear,
    label: &str,
    theme: &Theme,
    frame: &mut DragFrame,
) -> Confirmation {
    let count = pending.count;
    let mut confirmed = None;
    let response = egui::Modal::new(Id::new("clear-confirm")).show(ui.ctx(), |ui| {
        ui.set_max_width(440.0);
        ui.label(theme.heading(format!("Delete all {count} items in {label}?")));
        ui.label(
            "Every item in the tab is deleted; none of them enters the vault store. The file is \
             written by autosave, backup-first, and the game's own copy is backed up once per \
             load, so the pre-session file survives beside it.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(format!("Delete {count} items")).clicked() {
                confirmed = Some(true);
            }
            if ui.button("Cancel").clicked() {
                confirmed = Some(false);
            }
        });
    });
    let answer = match confirmed {
        Some(true) => Answer::Confirmed,
        Some(false) => Answer::Cancelled,
        None if response.should_close() => Answer::Cancelled,
        None => Answer::Waiting,
    };
    let (state, request) = settle_clear(answer, pending);
    if request.is_some() {
        frame.bulk = request;
    }
    state
}

/// What the user did to a confirmation this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Confirmed,
    Cancelled,
    Waiting,
}

/// Whether the confirmation stays up and what it reports: only a
/// confirmed clear becomes a request.
fn settle_clear(answer: Answer, pending: PendingClear) -> (Confirmation, Option<BulkRequest>) {
    match answer {
        Answer::Confirmed => (
            Confirmation::Settled,
            Some(BulkRequest {
                container: pending.container,
                op: BulkOp::Clear,
            }),
        ),
        Answer::Cancelled => (Confirmation::Settled, None),
        Answer::Waiting => (Confirmation::Pending, None),
    }
}

/// The checkbox label and explanation of an order's toggle.
fn order_toggle_text(order: StandingOrder) -> (&'static str, &'static str) {
    match order {
        StandingOrder::AutoMove => ("Auto-move to vault", AUTO_MOVE_WHY),
        StandingOrder::PurgeDuplicates => ("Purge duplicates", PURGE_WHY),
    }
}

/// The standing-order toggles on a tab's header — auto-move, then
/// purge — reporting a change through the frame.
pub fn order_toggles(ui: &mut Ui, tab: &AutoMoveTab, cx: &PaneCtx<'_>, frame: &mut DragFrame) {
    for order in StandingOrder::ALL {
        let (label, why) = order_toggle_text(order);
        let mut nominated = cx.settings.is_nominated(order, tab);
        if ui
            .checkbox(&mut nominated, label)
            .on_hover_text(why)
            .changed()
        {
            let tab = tab.clone();
            frame.standing_order = Some(if nominated {
                OrderRequest::Nominate { order, tab }
            } else {
                OrderRequest::Withdraw { order, tab }
            });
        }
    }
}

/// How a grid takes part in drag-and-drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// A writable container: a drag source and a drop target.
    Editable(Container),
    /// Hover and tooltips only.
    ReadOnly,
}

/// One item on a grid.
pub struct GridEntry<'a> {
    pub item: &'a Item,
    pub cells: CellRect,
    pub footprint: FootprintSource,
    pub index: ItemIndex,
}

const UV_FULL: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// The entries of a stash tab. The game stores cells as whole `f32`s;
/// anything else is drawn at the nearest non-negative cell.
pub fn stash_entries<'a>(tab: &'a StashTab, cx: &mut PaneCtx<'_>) -> Vec<GridEntry<'a>> {
    tab.items
        .iter()
        .enumerate()
        .map(|(index, placed)| {
            let (footprint, source) =
                footprint_or_unit(cx.facts.base(cx.game, &placed.item).footprint);
            GridEntry {
                item: &placed.item,
                cells: cells_of(
                    GridPos {
                        x: stash_cell(placed.x),
                        y: stash_cell(placed.y),
                    },
                    footprint,
                ),
                footprint: source,
                index: ItemIndex::new(index),
            }
        })
        .collect()
}

/// The entries of an inventory sack.
pub fn sack_entries<'a>(sack: &'a Sack, cx: &mut PaneCtx<'_>) -> Vec<GridEntry<'a>> {
    sack.items
        .iter()
        .enumerate()
        .map(|(index, placed)| {
            let (footprint, source) =
                footprint_or_unit(cx.facts.base(cx.game, &placed.item).footprint);
            GridEntry {
                item: &placed.item,
                cells: cells_of(
                    GridPos {
                        x: i32::try_from(placed.x).unwrap_or(0),
                        y: i32::try_from(placed.y).unwrap_or(0),
                    },
                    footprint,
                ),
                footprint: source,
                index: ItemIndex::new(index),
            }
        })
        .collect()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "stash cells are small whole numbers; anything else is clamped into a paintable cell"
)]
fn stash_cell(value: f32) -> i32 {
    value.round().clamp(0.0, 65_536.0) as i32
}

/// The far edge of the entries, for growing a container whose fixed
/// size the data exceeds.
#[must_use]
pub fn extent(entries: &[GridEntry<'_>]) -> (i32, i32) {
    entries.iter().fold((0, 0), |(width, height), entry| {
        (
            width.max(entry.cells.x + entry.cells.width),
            height.max(entry.cells.y + entry.cells.height),
        )
    })
}

/// A grid's cell count and the room it may take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub available: Vec2,
    pub cols: i32,
    pub rows: i32,
}

/// Paints the grid `spec` describes, its items, hover feedback and
/// tooltips, and — for an editable grid — reports drag starts,
/// double-clicks, and the drop candidate under a drag.
pub fn grid_surface(
    ui: &mut Ui,
    spec: GridSpec,
    entries: &[GridEntry<'_>],
    interaction: Interaction,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Response {
    let GridSpec {
        available,
        cols,
        rows,
    } = spec;
    let size = GridGeometry::fit(Pos2::ZERO, available, cols, rows).size();
    let sense = match interaction {
        Interaction::Editable(_) => Sense::click_and_drag(),
        Interaction::ReadOnly => Sense::hover(),
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let geometry = GridGeometry::fit(rect.min, size, cols, rows);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter_at(rect);
    paint_grid_lines(&painter, &geometry, cx.palette);

    let rects: Vec<CellRect> = entries.iter().map(|entry| entry.cells).collect();
    let hovered = response
        .hover_pos()
        .and_then(|pointer| geometry.cell_at(pointer))
        .and_then(|cell| occupant_at(&rects, cell));
    let dragging = cx.drag.is_some();

    for (slot, entry) in entries.iter().enumerate() {
        let tile = geometry.cell_rect(entry.cells).shrink(1.0);
        let source = match interaction {
            Interaction::Editable(container) => Some(DragSource::Grid {
                container,
                index: entry.index,
            }),
            Interaction::ReadOnly => None,
        };
        let lifted = source.is_some_and(|source| cx.drag.is_some_and(|drag| drag.source == source));
        paint_item(
            ui.ctx(),
            &painter,
            tile,
            entry.item,
            entry.footprint,
            geometry.cell(),
            hovered == Some(slot) && !dragging,
            lifted,
            cx,
        );
    }

    if !dragging && let Some(slot) = hovered {
        let item = entries[slot].item;
        egui::Tooltip::for_enabled(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, cx, item));
    }

    if let Interaction::Editable(container) = interaction {
        report_gestures(
            ui, &response, &geometry, &rects, entries, container, cx, frame,
        );
        if let Some(drag) = cx.drag
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
            && rect.contains(pointer)
        {
            let cell = geometry.snap(pointer - drag.grab, drag.footprint);
            let lifted_index = match (drag.source, cx.mode) {
                (
                    DragSource::Grid {
                        container: from,
                        index,
                    },
                    Mode::Move,
                ) if from == container => Some(index),
                (
                    DragSource::Grid { .. }
                    | DragSource::Store(_)
                    | DragSource::Reagent { .. }
                    | DragSource::Equipped { .. },
                    Mode::Move | Mode::Copy,
                ) => None,
            };
            let others = entries
                .iter()
                .filter(|entry| Some(entry.index) != lifted_index);
            let occupied: Vec<CellRect> = others.clone().map(|entry| entry.cells).collect();
            let resolvable = others
                .clone()
                .all(|entry| entry.footprint == FootprintSource::Known);
            let fit = drag::fit_at(&occupied, resolvable, drag.footprint, cell, cols, rows);
            paint_fit_preview(
                &painter,
                geometry
                    .cell_rect(cells_of(cell, drag.footprint))
                    .shrink(1.0),
                fit,
            );
            frame.candidate = Some(DropCandidate {
                target: DropTarget::Cell { container, cell },
                fit,
            });
        }
    }
    response
}

/// A container's tab button: selects it when clicked and, while a
/// drag is in flight, takes the drop as a first fit into that
/// container.
pub fn container_tab(
    ui: &mut Ui,
    selected: bool,
    label: String,
    container: Container,
    cx: &PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Response {
    let response = ui.selectable_label(selected, label);
    if response.clicked() {
        frame.touched = Some(container);
    }
    if cx.drag.is_some() && response.contains_pointer() {
        outline(ui, response.rect, Fit::Fits);
        frame.candidate = Some(DropCandidate {
            target: DropTarget::Container(container),
            fit: Fit::Fits,
        });
    }
    response
}

/// The drop verdict drawn around a tab button.
pub fn outline(ui: &Ui, rect: Rect, fit: Fit) {
    let colour = match fit {
        Fit::Fits => FITS,
        Fit::Blocked | Fit::Unresolvable => BLOCKED,
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(2.0, colour),
        StrokeKind::Outside,
    );
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call surface inside the grid renderer; the arguments are the frame's borrows"
)]
fn report_gestures(
    ui: &Ui,
    response: &Response,
    geometry: &GridGeometry,
    rects: &[CellRect],
    entries: &[GridEntry<'_>],
    container: Container,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    // The press origin, not the current pointer, decides which item a
    // starting drag lifts: the pointer may already have moved past
    // egui's drag threshold.
    let press_origin = ui.input(|input| input.pointer.press_origin());
    let pressed = press_origin
        .and_then(|origin| geometry.cell_at(origin))
        .and_then(|cell| occupant_at(rects, cell));
    if response.drag_started()
        && cx.drag.is_none()
        && frame.begin.is_none()
        && let Some(slot) = pressed
        && let Some(origin) = press_origin
    {
        let entry = &entries[slot];
        let (footprint, _) = footprint_or_unit(cx.facts.footprint(entry.item));
        frame.begin = Some(DragState {
            source: DragSource::Grid {
                container,
                index: entry.index,
            },
            item: entry.item.clone(),
            footprint,
            grab: origin - geometry.cell_rect(entry.cells).min,
        });
    }
    let occupant_under = |pointer: Option<Pos2>| {
        pointer
            .and_then(|pointer| geometry.cell_at(pointer))
            .and_then(|cell| occupant_at(rects, cell))
    };
    if response.double_clicked()
        && cx.drag.is_none()
        && let Some(slot) = occupant_under(response.hover_pos())
    {
        frame.double_click = Some(DragSource::Grid {
            container,
            index: entries[slot].index,
        });
    }
    if response.secondary_clicked()
        && cx.drag.is_none()
        && let Some(slot) = occupant_under(response.interact_pointer_pos())
    {
        frame.right_click = Some(DragSource::Grid {
            container,
            index: entries[slot].index,
        });
    }
    if response.clicked()
        && cx.drag.is_none()
        && let Some(slot) = occupant_under(response.interact_pointer_pos())
    {
        frame.select = Some(DragSource::Grid {
            container,
            index: entries[slot].index,
        });
    }
}

fn paint_grid_lines(painter: &Painter, geometry: &GridGeometry, palette: &Palette) {
    let rect = geometry.rect();
    painter.rect_filled(rect, CornerRadius::same(2), palette.grid_bg);
    let stroke = Stroke::new(0.5, palette.grid_line);
    for column in 0..=geometry.cols() {
        let x = geometry
            .cell_rect(CellRect {
                x: column,
                y: 0,
                width: 0,
                height: 0,
            })
            .min
            .x;
        painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], stroke);
    }
    for row in 0..=geometry.rows() {
        let y = geometry
            .cell_rect(CellRect {
                x: 0,
                y: row,
                width: 0,
                height: 0,
            })
            .min
            .y;
        painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
    }
}

/// One item's tile: fill, the icon or the initials fallback, a
/// rarity-coloured border, the unknown-footprint mark, the stack
/// badge, and the lifted dimming.
#[expect(
    clippy::too_many_arguments,
    reason = "one call surface inside the grid renderer; the arguments are the frame's borrows"
)]
pub fn paint_item(
    ctx: &egui::Context,
    painter: &Painter,
    rect: Rect,
    item: &Item,
    footprint: FootprintSource,
    cell: f32,
    hovered: bool,
    lifted: bool,
    cx: &mut PaneCtx<'_>,
) {
    let facts = cx.facts.facts(cx.game, item);
    let rarity = facts.base.rarity;
    let initials = facts.initials();
    let bitmap = facts.base.bitmap.clone();
    let symbol = facts.facets.symbol();
    let icon = cx.icons.icon(ctx, cx.game, bitmap.as_ref());
    let badge_icon = symbol.map(|symbol| cx.icons.symbol(ctx, symbol));
    paint_tile(
        painter,
        rect,
        &TileLook {
            rarity,
            initials: &initials,
            footprint,
            stack: item.stack_count,
            icon: &icon,
            cell,
            badge: symbol
                .zip(badge_icon.as_ref())
                .map(|(symbol, icon)| Badge { symbol, icon }),
            hovered,
            lifted,
        },
        cx.palette,
    );
}

/// What a tile shows.
pub struct TileLook<'a> {
    pub rarity: Option<Rarity>,
    pub initials: &'a str,
    pub footprint: FootprintSource,
    pub stack: u32,
    pub icon: &'a Icon,
    /// Points per footprint cell in the pane painting the tile; the
    /// badge is sized from it.
    pub cell: f32,
    /// The game's tile symbol, when the item's facets earn one.
    pub badge: Option<Badge<'a>>,
    pub hovered: bool,
    pub lifted: bool,
}

pub fn paint_tile(painter: &Painter, rect: Rect, look: &TileLook<'_>, palette: &Palette) {
    painter.rect_filled(rect, CornerRadius::same(2), palette.tile_bg);
    match look.icon {
        Icon::Texture(texture) => {
            painter.image(texture.id(), rect, UV_FULL, Color32::WHITE);
        }
        Icon::Missing(_) => {
            let size = (rect.height() * 0.35).clamp(9.0, 16.0);
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                look.initials,
                FontId::proportional(size),
                palette.text_strong,
            );
        }
    }
    let border = look.rarity.map_or(UNKNOWN_RARITY, rarity_color);
    let stroke = if look.hovered {
        Stroke::new(2.0, palette.selection_stroke)
    } else {
        Stroke::new(1.0, border)
    };
    painter.rect_stroke(rect, CornerRadius::same(2), stroke, StrokeKind::Inside);
    if let Some(badge) = &look.badge {
        paint_badge(painter, rect, look.cell, badge, palette);
    }
    if look.footprint == FootprintSource::Assumed {
        painter.text(
            rect.left_top() + vec2(3.0, 1.0),
            Align2::LEFT_TOP,
            "!",
            FontId::proportional(11.0),
            palette.warn,
        );
    }
    if look.stack > 1 {
        painter.text(
            rect.right_bottom() - vec2(2.0, 1.0),
            Align2::RIGHT_BOTTOM,
            look.stack.to_string(),
            FontId::proportional(10.0),
            palette.text_strong,
        );
    }
    if look.lifted {
        painter.rect_filled(rect, CornerRadius::same(2), Color32::from_black_alpha(150));
    }
}

/// The translucent footprint under a drag: green fits, red does not.
pub fn paint_fit_preview(painter: &Painter, preview: Rect, fit: Fit) {
    let colour = match fit {
        Fit::Fits => FITS,
        Fit::Blocked | Fit::Unresolvable => BLOCKED,
    };
    painter.rect_filled(
        preview,
        CornerRadius::same(2),
        Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 60),
    );
    painter.rect_stroke(
        preview,
        CornerRadius::same(2),
        Stroke::new(2.0, colour),
        StrokeKind::Inside,
    );
}

/// Item details on hover: the item's tile at the game's native size
/// with its symbol, the name in its rarity colour, the facets the game
/// would mark, then rarity, class, level gate and stack, the stat
/// blocks with the set and the requirements, then the base record in a
/// muted monospace.
pub fn item_tooltip(ui: &mut Ui, cx: &mut PaneCtx<'_>, item: &Item) {
    let sockets = socket_line(cx, item);
    let facts = cx.facts.facts(cx.game, item);
    let (footprint, footprint_source) = footprint_or_unit(facts.base.footprint);
    let symbol = facts.facets.symbol();
    let icon = cx.icons.icon(ui.ctx(), cx.game, facts.base.bitmap.as_ref());
    let badge_icon = symbol.map(|symbol| cx.icons.symbol(ui.ctx(), symbol));
    let (response, painter) = ui.allocate_painter(native_size(footprint), Sense::hover());
    paint_tile(
        &painter,
        response.rect,
        &TileLook {
            rarity: facts.base.rarity,
            initials: &facts.initials(),
            footprint: footprint_source,
            stack: item.stack_count,
            icon: &icon,
            cell: CELL_PX,
            badge: symbol
                .zip(badge_icon.as_ref())
                .map(|(symbol, icon)| Badge { symbol, icon }),
            hovered: false,
            lifted: false,
        },
        cx.palette,
    );
    let colour = facts.base.rarity.map_or(UNKNOWN_RARITY, rarity_color);
    ui.label(RichText::new(facts.display_name()).color(colour).strong());
    let labels = facts.facets.labels();
    if !labels.is_empty() {
        ui.label(RichText::new(labels.join(" · ")).color(cx.palette.heading));
    }
    let details: Vec<String> = [
        facts.base.rarity.map(|rarity| format!("{rarity:?}")),
        facts.base.class.as_ref().map(ToString::to_string),
        facts
            .base
            .level_requirement
            .map(|level| format!("Level {level}")),
        (item.stack_count > 1).then(|| format!("x{}", item.stack_count)),
        facts
            .base
            .footprint
            .map(|footprint| format!("{}×{}", footprint.width, footprint.height)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !details.is_empty() {
        ui.label(details.join(" · "));
    }
    if let Some(sockets) = sockets {
        ui.label(RichText::new(sockets).color(cx.palette.heading));
    }
    crate::stat_lines::stat_body(ui, cx.game, cx.palette, item);
    ui.label(
        RichText::new(&item.base_name)
            .monospace()
            .size(10.5)
            .color(cx.palette.text_weak),
    );
    if let Icon::Missing(problem) = &icon {
        ui.label(
            RichText::new(format!("no icon: {problem}"))
                .small()
                .color(cx.palette.text_weak),
        );
    }
}

/// "Component: X · Augment: Y" for the sockets the item fills, `None`
/// when both are empty.
fn socket_line(cx: &mut PaneCtx<'_>, item: &Item) -> Option<String> {
    let parts: Vec<String> = Socket::ALL
        .into_iter()
        .filter_map(|socket| {
            let record = socket.record_of(item);
            (!record.is_empty()).then(|| format!("{}: {}", socket.title(), part_name(cx, record)))
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The database's name for a part record, as the base facts resolve
/// it.
pub fn part_name(cx: &mut PaneCtx<'_>, record: &str) -> String {
    let part = Item {
        base_name: record.to_string(),
        ..Item::default()
    };
    cx.facts.base(cx.game, &part).name.clone()
}

#[cfg(test)]
mod tests {
    use grimvault_core::transfer::TabIndex;

    use super::*;

    #[test]
    fn only_a_confirmed_clear_becomes_a_request() {
        let pending = PendingClear {
            container: Container::TransferStash(TabIndex::new(2)),
            count: 5,
        };
        assert_eq!(
            settle_clear(Answer::Waiting, pending),
            (Confirmation::Pending, None)
        );
        assert_eq!(
            settle_clear(Answer::Cancelled, pending),
            (Confirmation::Settled, None)
        );
        assert_eq!(
            settle_clear(Answer::Confirmed, pending),
            (
                Confirmation::Settled,
                Some(BulkRequest {
                    container: Container::TransferStash(TabIndex::new(2)),
                    op: BulkOp::Clear,
                })
            )
        );
    }

    #[test]
    fn bulk_ops_name_themselves_for_the_toasts() {
        assert_eq!(BulkOp::Transfer(Mode::Move).to_string(), "move all");
        assert_eq!(BulkOp::Transfer(Mode::Copy).done(), "copied");
        assert_eq!(BulkOp::Clear.done(), "deleted");
    }

    #[test]
    fn stash_cells_round_and_never_go_negative() {
        assert_eq!(stash_cell(4.0), 4);
        assert_eq!(stash_cell(3.6), 4);
        assert_eq!(stash_cell(-2.0), 0);
        assert_eq!(stash_cell(f32::NAN), 0);
    }

    #[test]
    fn extent_is_the_far_edge_of_the_entries() {
        let item = Item::default();
        let entries = [
            GridEntry {
                item: &item,
                cells: CellRect {
                    x: 1,
                    y: 6,
                    width: 1,
                    height: 2,
                },
                footprint: FootprintSource::Known,
                index: ItemIndex::new(0),
            },
            GridEntry {
                item: &item,
                cells: CellRect {
                    x: 10,
                    y: 0,
                    width: 2,
                    height: 3,
                },
                footprint: FootprintSource::Known,
                index: ItemIndex::new(1),
            },
        ];
        assert_eq!(extent(&entries), (12, 8));
        assert_eq!(extent(&[]), (0, 0));
    }
}
