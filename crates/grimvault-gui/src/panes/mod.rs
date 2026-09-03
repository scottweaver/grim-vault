//! The Ready phase's surfaces: the stash with the component /
//! crafting-material storage beside it, the store, and the
//! characters, over one shared cell-grid renderer. Each pane reads the
//! documents, paints, and *reports* what the user did this frame
//! through [`DragFrame`]; the app turns those reports into moves after
//! every pane has drawn, so no pane ever mutates a document.

pub mod character;
pub mod reagents;
pub mod stash;
pub mod store;

use egui::{
    Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, RichText, Sense, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};
use grimvault_core::block::StashTab;
use grimvault_core::gamedata::{GameData, Rarity};
use grimvault_core::gdc::Sack;
use grimvault_core::item::Item;
use grimvault_core::transfer::{Footprints, ItemIndex, TabIndex};
use univault_engine::grid::CellRect;
use univault_engine::ids::GridPos;
use univault_ui::theme::Palette;

use crate::drag::{self, DragSource, DragState, DropTarget, Fit};
use crate::facts::FactsCache;
use crate::grid::{FootprintSource, GridGeometry, cells_of, footprint_or_unit, occupant_at};
use crate::icons::{Icon, IconCache};
use crate::theme::{BLOCKED, FITS, UNKNOWN_RARITY, rarity_color};

/// What every pane needs to paint.
pub struct PaneCtx<'a> {
    pub game: &'a GameData,
    pub facts: &'a mut FactsCache,
    pub icons: &'a mut IconCache,
    pub palette: &'a Palette,
    pub drag: Option<&'a DragState>,
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
}

/// How a grid takes part in drag-and-drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// A transfer-stash tab: a drag source and a drop target.
    Editable { tab: TabIndex },
    /// A character container: hover and tooltips only.
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
        Interaction::Editable { .. } => Sense::click_and_drag(),
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
            Interaction::Editable { tab } => Some(DragSource::Stash {
                tab,
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

    if let Interaction::Editable { tab } = interaction {
        report_gestures(ui, &response, &geometry, &rects, entries, tab, cx, frame);
        if let Some(drag) = cx.drag
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
            && rect.contains(pointer)
        {
            let cell = geometry.snap(pointer - drag.grab, drag.footprint);
            let lifted_index = match drag.source {
                DragSource::Stash { tab: from, index } if from == tab => Some(index),
                DragSource::Stash { .. } | DragSource::Store(_) | DragSource::Reagent { .. } => {
                    None
                }
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
                target: DropTarget::StashCell { tab, cell },
                fit,
            });
        }
    }
    response
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
    tab: TabIndex,
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
            source: DragSource::Stash {
                tab,
                index: entry.index,
            },
            item: entry.item.clone(),
            footprint,
            grab: origin - geometry.cell_rect(entry.cells).min,
        });
    }
    if response.double_clicked()
        && cx.drag.is_none()
        && let Some(slot) = response
            .hover_pos()
            .and_then(|pointer| geometry.cell_at(pointer))
            .and_then(|cell| occupant_at(rects, cell))
    {
        frame.double_click = Some(DragSource::Stash {
            tab,
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
    hovered: bool,
    lifted: bool,
    cx: &mut PaneCtx<'_>,
) {
    let facts = cx.facts.facts(cx.game, item);
    let rarity = facts.base.rarity;
    let initials = facts.initials();
    let bitmap = facts.base.bitmap.clone();
    let icon = cx.icons.icon(ctx, cx.game, bitmap.as_ref());
    paint_tile(
        painter,
        rect,
        &TileLook {
            rarity,
            initials: &initials,
            footprint,
            stack: item.stack_count,
            icon: &icon,
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

/// Item details on hover: name in its rarity colour, then rarity,
/// class, level gate and stack, then the base record in a muted
/// monospace.
pub fn item_tooltip(ui: &mut Ui, cx: &mut PaneCtx<'_>, item: &Item) {
    let facts = cx.facts.facts(cx.game, item);
    let colour = facts.base.rarity.map_or(UNKNOWN_RARITY, rarity_color);
    ui.label(RichText::new(facts.display_name()).color(colour).strong());
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
    ui.label(
        RichText::new(&item.base_name)
            .monospace()
            .size(10.5)
            .color(cx.palette.text_weak),
    );
    let bitmap = facts.base.bitmap.clone();
    if let Icon::Missing(problem) = cx.icons.icon(ui.ctx(), cx.game, bitmap.as_ref()) {
        ui.label(
            RichText::new(format!("no icon: {problem}"))
                .small()
                .color(cx.palette.text_weak),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
