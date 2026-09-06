//! The component / crafting-material storage as two tabs beside the
//! transfer stash's: the entries of one tab as rows — icon, name,
//! count — sorted by name. Rows are drag sources; the list is a drop
//! target for storable stash and store items.

use egui::{CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use grimvault_core::gst::{ReagentEntry, ReagentStorage};
use grimvault_core::item::Item;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::transfer::ReagentIndex;

use super::{DragFrame, DropCandidate, PaneCtx, TileLook, item_tooltip, paint_tile};
use crate::documents::Reagents;
use crate::drag::{self, DragSource, DragState, DropTarget, Fit};
use crate::grid::{CELL_PX, footprint_or_unit};
use crate::theme::FITS;

const ROW_HEIGHT: f32 = 36.0;
const ICON: f32 = 30.0;

/// One entry as the pane lists it.
pub struct Row<'a> {
    pub index: ReagentIndex,
    pub entry: &'a ReagentEntry,
    pub item: Item,
    pub name: String,
}

/// The item a storage entry stands for: its record and count, nothing
/// else — which is all the storage keeps.
#[must_use]
pub fn entry_item(entry: &ReagentEntry) -> Item {
    Item {
        base_name: entry.record.clone(),
        stack_count: entry.count,
        ..Item::default()
    }
}

/// The entries shown under `kind`, sorted by resolved name then file
/// order.
pub fn rows<'a>(
    storage: &'a ReagentStorage,
    kind: ReagentKind,
    cx: &mut PaneCtx<'_>,
) -> Vec<Row<'a>> {
    let mut rows: Vec<Row<'a>> = storage
        .entries
        .iter()
        .enumerate()
        .filter_map(|(slot, entry)| {
            let item = entry_item(entry);
            let base = cx.facts.base(cx.game, &item);
            (ReagentKind::in_storage(base.reagent) == kind).then(|| Row {
                index: ReagentIndex::new(slot),
                entry,
                name: base.name.clone(),
                item,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name).then(a.index.cmp(&b.index)));
    rows
}

/// How many of each entry a drag from `kind`'s list carries: the whole
/// entry unless `amount` is set and smaller.
#[must_use]
pub fn drag_count(entry_count: u32, amount: u32) -> u32 {
    if amount == 0 || amount > entry_count {
        entry_count
    } else {
        amount
    }
}

/// Paints one tab of the storage, or why it cannot be shown.
pub fn show(
    ui: &mut Ui,
    reagents: &Reagents,
    kind: ReagentKind,
    amount: &mut u32,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let doc = match reagents {
        Reagents::Open(doc) => doc,
        Reagents::Absent { path } => {
            ui.weak(format!(
                "{} does not exist yet: the game writes it the first time the storage is used.",
                path.display()
            ));
            return;
        }
        Reagents::Failed { path, error, .. } => {
            ui.colored_label(
                cx.palette.error,
                format!("{} cannot be edited: {error}", path.display()),
            );
            return;
        }
    };
    let storage = doc.storage();
    let rows = rows(storage, kind, cx);
    ui.horizontal_wrapped(|ui| {
        ui.label(format!(
            "{} entries · {} items",
            rows.len(),
            rows.iter()
                .map(|row| u64::from(row.entry.count))
                .sum::<u64>()
        ));
        ui.separator();
        ui.label("per drag:");
        ui.add(
            egui::DragValue::new(amount)
                .range(0..=u32::MAX)
                .custom_formatter(|value, _| {
                    if value == 0.0 {
                        "whole stack".to_string()
                    } else {
                        format!("{value}")
                    }
                }),
        )
        .on_hover_text(
            "How many a drag or double-click takes from a row: the whole stack, or this many.",
        );
    });

    let zone = ui.available_rect_before_wrap();
    if let Some(drag) = cx.drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && zone.contains(pointer)
    {
        let item_kind = cx.facts.base(cx.game, &drag.item).reagent;
        let fit = drag::fit_in_reagents(drag.source, item_kind);
        if fit == Fit::Fits {
            ui.painter().rect_stroke(
                zone.shrink(1.0),
                CornerRadius::same(3),
                Stroke::new(2.0, FITS),
                StrokeKind::Inside,
            );
        }
        frame.candidate = Some(DropCandidate {
            target: DropTarget::Reagents(kind),
            fit,
        });
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.weak(format!(
                    "No {} in the storage.",
                    kind.label().to_lowercase()
                ));
            }
            for row in &rows {
                reagent_row(ui, row, *amount, cx, frame);
            }
        });
}

fn reagent_row(
    ui: &mut Ui,
    row: &Row<'_>,
    amount: u32,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let count = drag_count(row.entry.count, amount);
    let source = DragSource::Reagent {
        index: row.index,
        count,
    };
    let lifted = cx.drag.is_some_and(|drag| drag.source == source);
    let hovered = cx.drag.is_none() && response.hovered();

    let facts = cx.facts.facts(cx.game, &row.item);
    let (footprint, footprint_source) = footprint_or_unit(facts.base.footprint);
    let rarity = facts.base.rarity;
    let initials = facts.initials();
    let bitmap = facts.base.bitmap.clone();
    let icon = cx.icons.icon(ui.ctx(), cx.game, bitmap.as_ref());

    let painter = ui.painter_at(rect);
    if hovered {
        painter.rect_filled(rect, CornerRadius::same(3), cx.palette.faint_bg);
    }
    let icon_rect = Rect::from_min_size(
        rect.min + vec2(4.0, (ROW_HEIGHT - ICON) / 2.0),
        vec2(ICON, ICON),
    );
    paint_tile(
        &painter,
        icon_rect,
        &TileLook {
            rarity,
            initials: &initials,
            footprint: footprint_source,
            stack: 1,
            icon: &icon,
            cell: icon_rect.width(),
            badge: None,
            hovered,
            lifted,
        },
        cx.palette,
    );
    paint_name_and_badge(
        &painter,
        rect,
        icon_rect.max.x + 10.0,
        row,
        lifted,
        cx.palette,
    );

    if hovered {
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, cx.palette.selection_stroke),
            StrokeKind::Inside,
        );
        egui::Tooltip::for_enabled(&response)
            .at_pointer()
            .show(|ui| {
                item_tooltip(ui, cx, &row.item);
                ui.label(
                    RichText::new(format!(
                        "storage entry {} · a drag takes {count}",
                        row.index
                    ))
                    .small()
                    .color(cx.palette.text_weak),
                );
            });
    }
    if response.drag_started() && cx.drag.is_none() && frame.begin.is_none() {
        let ghost = vec2(CELL_PX, CELL_PX) * vec2(cells(footprint.width), cells(footprint.height));
        frame.begin = Some(DragState {
            source,
            item: Item {
                stack_count: count,
                ..row.item.clone()
            },
            footprint,
            grab: ghost / 2.0,
        });
    }
    if response.double_clicked() && cx.drag.is_none() {
        frame.double_click = Some(source);
    }
}

/// The resolved name after the icon and the count badge at the right
/// edge; a lifted row's name is dimmed like a lifted tile.
fn paint_name_and_badge(
    painter: &egui::Painter,
    rect: Rect,
    text_left: f32,
    row: &Row<'_>,
    lifted: bool,
    palette: &univault_ui::theme::Palette,
) {
    painter.text(
        egui::pos2(text_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &row.name,
        egui::FontId::proportional(13.0),
        if lifted {
            palette.text_weak
        } else {
            palette.text
        },
    );
    let galley = painter.layout_no_wrap(
        format!("×{}", row.entry.count),
        egui::FontId::proportional(12.0),
        palette.text_strong,
    );
    let badge_rect = Rect::from_min_size(
        egui::pos2(
            rect.max.x - galley.size().x - 16.0,
            rect.center().y - galley.size().y / 2.0 - 3.0,
        ),
        galley.size() + vec2(12.0, 6.0),
    );
    painter.rect_filled(badge_rect, CornerRadius::same(8), palette.accent_dim);
    painter.galley(badge_rect.min + vec2(6.0, 3.0), galley, palette.text_strong);
}

#[expect(
    clippy::cast_precision_loss,
    reason = "footprints are a handful of cells"
)]
fn cells(n: i32) -> f32 {
    n as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_takes_the_whole_entry_unless_a_smaller_amount_is_set() {
        assert_eq!(drag_count(15, 0), 15);
        assert_eq!(drag_count(15, 4), 4);
        assert_eq!(drag_count(15, 15), 15);
        assert_eq!(drag_count(15, 40), 15);
    }

    #[test]
    fn an_entry_becomes_a_bare_stack_of_its_record() {
        let entry = ReagentEntry {
            record: "records/items/materia/compa_moltenskin.dbr".into(),
            count: 20,
        };
        let item = entry_item(&entry);
        assert_eq!(item.base_name, entry.record);
        assert_eq!(item.stack_count, 20);
        assert_eq!(item.seed, 0);
        assert!(item.prefix_name.is_empty());
    }
}
