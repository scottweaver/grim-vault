//! The vault store pane: a Group → Bucket selector over the computed
//! type view, then the selected bucket's items as a flow of tiles.
//! The whole pane is a drop target for stash items and storage rows.

use std::collections::HashMap;

use egui::{CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::store::StoredItem;
use univault_ui::theme::Theme;

use super::{DragFrame, DropCandidate, PaneCtx, TileLook, item_tooltip, paint_tile};
use crate::documents::StoreDoc;
use crate::drag::{self, DragSource, DragState, DropTarget, Fit};
use crate::grid::{CELL_PX, footprint_or_unit};
use crate::theme::FITS;

/// Which bucket the pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreView {
    pub group: Group,
    pub bucket: Bucket,
}

impl Default for StoreView {
    fn default() -> Self {
        Self {
            group: Group::Weapons,
            bucket: Bucket::OneHanded,
        }
    }
}

const TILE: egui::Vec2 = vec2(96.0, 108.0);
const ICON_BOX: f32 = 64.0;

pub fn show(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut StoreView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    ui.label(theme.heading("Vault store"));
    ui.label(theme.path_text(doc.path().display().to_string()));
    let store = doc.store();
    ui.label(format!("{} items", store.len()));

    let mut counts: HashMap<Bucket, usize> = HashMap::new();
    for stored in store.items() {
        *counts
            .entry(cx.facts.base(cx.game, stored.item()).bucket)
            .or_default() += 1;
    }
    let group_count = |group: Group| -> usize {
        Bucket::ALL
            .iter()
            .filter(|bucket| bucket.group() == group)
            .map(|bucket| counts.get(bucket).copied().unwrap_or(0))
            .sum()
    };
    ui.horizontal_wrapped(|ui| {
        for group in Group::ALL {
            let label = format!("{} ({})", group.label(), group_count(group));
            if ui.selectable_label(view.group == group, label).clicked() && view.group != group {
                view.group = group;
                if let Some(first) = Bucket::ALL.iter().find(|bucket| bucket.group() == group) {
                    view.bucket = *first;
                }
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        for bucket in Bucket::ALL
            .iter()
            .filter(|bucket| bucket.group() == view.group)
        {
            let label = format!(
                "{} ({})",
                bucket.label(),
                counts.get(bucket).copied().unwrap_or(0)
            );
            if ui.selectable_label(view.bucket == *bucket, label).clicked() {
                view.bucket = *bucket;
            }
        }
    });
    ui.separator();

    let zone = ui.available_rect_before_wrap();
    if let Some(drag) = cx.drag
        && let Some(pointer) = ui.ctx().pointer_latest_pos()
        && zone.contains(pointer)
    {
        let fit = drag::fit_in_store(drag.source, cx.mode);
        if fit == Fit::Fits {
            ui.painter().rect_stroke(
                zone.shrink(1.0),
                CornerRadius::same(3),
                Stroke::new(2.0, FITS),
                StrokeKind::Inside,
            );
        }
        frame.candidate = Some(DropCandidate {
            target: DropTarget::Store,
            fit,
        });
    }

    let mut shown: Vec<(String, &StoredItem)> = store
        .items()
        .iter()
        .filter_map(|stored| {
            let facts = cx.facts.facts(cx.game, stored.item());
            (facts.base.bucket == view.bucket).then(|| (facts.display_name(), stored))
        })
        .collect();
    shown.sort_by(|(a, left), (b, right)| a.cmp(b).then(left.id().cmp(&right.id())));
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.weak(format!("No {} in the store.", view.bucket.label()));
            }
            ui.horizontal_wrapped(|ui| {
                for (name, stored) in &shown {
                    store_tile(ui, name, stored, cx, frame);
                }
            });
        });
}

fn store_tile(
    ui: &mut Ui,
    name: &str,
    stored: &StoredItem,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let item = stored.item();
    let source = DragSource::Store(stored.id());
    let lifted = cx.drag.is_some_and(|drag| drag.source == source);
    let hovered = cx.drag.is_none() && response.hovered();

    let facts = cx.facts.facts(cx.game, item);
    let (footprint, footprint_source) = footprint_or_unit(facts.base.footprint);
    let rarity = facts.base.rarity;
    let initials = facts.initials();
    let bitmap = facts.base.bitmap.clone();
    let icon = cx.icons.icon(ui.ctx(), cx.game, bitmap.as_ref());

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(3), cx.palette.faint_bg);
    let icon_rect = icon_box(rect, footprint.width, footprint.height);
    paint_tile(
        &painter,
        icon_rect,
        &TileLook {
            rarity,
            initials: &initials,
            footprint: footprint_source,
            stack: item.stack_count,
            icon: &icon,
            hovered,
            lifted,
        },
        cx.palette,
    );
    let galley = painter.layout(
        name.to_string(),
        FontId::proportional(11.0),
        cx.palette.text,
        TILE.x - 8.0,
    );
    painter.galley(
        pos2(
            rect.center().x - galley.size().x / 2.0,
            rect.min.y + ICON_BOX + 8.0,
        ),
        galley,
        cx.palette.text,
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
                item_tooltip(ui, cx, item);
                ui.label(
                    RichText::new(format!("stored item {}", stored.id()))
                        .small()
                        .color(cx.palette.text_weak),
                );
            });
    }
    if response.drag_started() && cx.drag.is_none() && frame.begin.is_none() {
        let ghost = vec2(CELL_PX, CELL_PX) * vec2(cells(footprint.width), cells(footprint.height));
        frame.begin = Some(DragState {
            source,
            item: item.clone(),
            footprint,
            grab: ghost / 2.0,
        });
    }
    if response.double_clicked() && cx.drag.is_none() {
        frame.double_click = Some(source);
    }
}

/// The icon area, keeping the footprint's aspect inside a square box.
fn icon_box(tile: Rect, width: i32, height: i32) -> Rect {
    let (w, h) = (cells(width.max(1)), cells(height.max(1)));
    let scale = ICON_BOX / w.max(h);
    let size = vec2(w * scale, h * scale);
    Rect::from_center_size(
        pos2(tile.center().x, tile.min.y + 4.0 + ICON_BOX / 2.0),
        size,
    )
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
    fn icon_box_keeps_the_footprint_aspect_inside_the_square() {
        let tile = Rect::from_min_size(pos2(0.0, 0.0), TILE);
        let tall = icon_box(tile, 1, 2);
        assert_eq!(tall.size(), vec2(32.0, 64.0));
        let wide = icon_box(tile, 2, 1);
        assert_eq!(wide.size(), vec2(64.0, 32.0));
        let square = icon_box(tile, 2, 2);
        assert_eq!(square.size(), vec2(64.0, 64.0));
    }
}
