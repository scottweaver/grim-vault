//! The vault store pane, in one of two modes: the bucket view — a
//! Group → Bucket selector over the computed type view, a search bar,
//! then the selected bucket's matching items as a flow of tiles — or
//! the search view, the whole store as one filtered, sorted table
//! (`crate::search`). The whole pane is a drop target for stash items
//! and storage rows in either mode.

use std::collections::HashMap;

use egui::{
    Align, CornerRadius, FontId, Key, KeyboardShortcut, Modifiers, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui, pos2, vec2,
};
use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::search::{Query, Verdict};
use grimvault_core::settings::{BulkDuplicates, Settings};
use grimvault_core::stats;
use grimvault_core::store::{StoredItem, StoredItemId};
use serde::{Deserialize, Serialize};
use univault_ui::theme::Theme;

use super::{DragFrame, DropCandidate, PaneCtx, TileLook, item_tooltip, paint_tile};
use crate::badges::Badge;
use crate::documents::StoreDoc;
use crate::drag::{self, DragSource, DragState, DropTarget, Fit};
use crate::grid::{CELL_PX, footprint_or_unit};
use crate::search::{self, SearchCache, SearchView};
use crate::theme::FITS;

/// Which surface the pane shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoreMode {
    #[default]
    Buckets,
    Search,
}

/// The pane's view state — what survives a restart: the bucket in
/// view and its bar, the mode, and the search view's bar and sort.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StoreView {
    pub group: Group,
    pub bucket: Bucket,
    pub query: Query,
    pub mode: StoreMode,
    pub search: SearchView,
}

impl Default for StoreView {
    fn default() -> Self {
        Self {
            group: Group::Weapons,
            bucket: Bucket::OneHanded,
            query: Query::default(),
            mode: StoreMode::Buckets,
            search: SearchView::default(),
        }
    }
}

/// An item a search double-click asked the bucket view to show at
/// home: highlighted, and scrolled to once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub id: StoredItemId,
    pub scrolled: bool,
}

/// The shortcut that opens the search view.
pub const SEARCH_SHORTCUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);

const TILE: egui::Vec2 = vec2(96.0, 108.0);
const ICON_BOX: f32 = 64.0;

/// The selected bucket's items that answer the query, by name, and
/// what the query hid: the bucket's size and how many of its items a
/// required facet could not be decided for.
struct Filtered<'a> {
    shown: Vec<(String, &'a StoredItem)>,
    in_bucket: usize,
    unresolved: usize,
}

fn filtered<'a>(items: &'a [StoredItem], view: &StoreView, cx: &mut PaneCtx<'_>) -> Filtered<'a> {
    let needs_details = view.query.needs_details();
    let mut in_bucket = 0;
    let mut unresolved = 0;
    let mut shown: Vec<(String, &StoredItem)> = items
        .iter()
        .filter_map(|stored| {
            let item = stored.item();
            let details = needs_details.then(|| stats::item_details(cx.game, item));
            let facts = cx.facts.facts(cx.game, item);
            if facts.base.bucket != view.bucket {
                return None;
            }
            in_bucket += 1;
            let name = facts.display_name();
            match view
                .query
                .verdict(&facts.subject(item, &name, details.as_ref()))
            {
                Verdict::Matches => Some((name, stored)),
                Verdict::Excluded => None,
                Verdict::Unresolved => {
                    unresolved += 1;
                    None
                }
            }
        })
        .collect();
    shown.sort_by(|(a, left), (b, right)| a.cmp(b).then(left.id().cmp(&right.id())));
    Filtered {
        shown,
        in_bucket,
        unresolved,
    }
}

const SKIP_DUPLICATES_WHY: &str = "Bulk moves and copies into the vault — the buttons on a tab and the auto-move standing \
     order — pass over an item whose record and roll seed the store already holds. Drags, \
     double-clicks and right-clicks always land.";

/// The heading, the store's path, its size, the mode switch, the one
/// action that brings items in from outside the game — importing a
/// GD Stash export — and the bulk-duplicates rule.
fn header(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut StoreView,
    settings: &Settings,
    theme: &Theme,
    frame: &mut DragFrame,
) {
    ui.label(theme.heading("Vault store"));
    ui.label(theme.path_text(doc.path().display().to_string()));
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} items", doc.store().len()));
        ui.separator();
        ui.selectable_value(&mut view.mode, StoreMode::Buckets, "Buckets");
        ui.selectable_value(&mut view.mode, StoreMode::Search, "Search")
            .on_hover_text(format!(
                "Everything in the store as one filtered, sorted table ({})",
                ui.ctx().format_shortcut(&SEARCH_SHORTCUT)
            ));
        ui.separator();
        if ui
            .button("Import GD Stash export…")
            .on_hover_text(
                "Adds every item of a .gds file GD Stash exported; \
                 entries this store already imported are skipped.",
            )
            .clicked()
        {
            frame.import_gds = rfd::FileDialog::new()
                .add_filter("GD Stash export", &["gds"])
                .pick_file();
        }
        ui.separator();
        let mut skipping = settings.bulk_duplicates == BulkDuplicates::Skip;
        if ui
            .checkbox(&mut skipping, "Skip duplicates in bulk moves")
            .on_hover_text(SKIP_DUPLICATES_WHY)
            .changed()
        {
            frame.bulk_duplicates = Some(if skipping {
                BulkDuplicates::Skip
            } else {
                BulkDuplicates::Allow
            });
        }
    });
}

pub fn show(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut StoreView,
    cache: &mut SearchCache,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    header(ui, doc, view, cx.settings, theme, frame);
    ui.separator();
    drop_zone(ui, cx, frame);
    match view.mode {
        StoreMode::Buckets => bucket_view(ui, doc, view, cache, cx, frame),
        StoreMode::Search => {
            if let Some(id) = search::show_view(ui, doc, &mut view.search, cache, cx, frame) {
                reveal(doc, view, cache, cx, id);
            }
        }
    }
}

/// The rest of the pane takes a drop as "into the store", filed by
/// type.
fn drop_zone(ui: &Ui, cx: &PaneCtx<'_>, frame: &mut DragFrame) {
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
}

/// Switches to the bucket view showing `id` at home, with the bar
/// cleared so nothing hides it.
fn reveal(
    doc: &StoreDoc,
    view: &mut StoreView,
    cache: &mut SearchCache,
    cx: &mut PaneCtx<'_>,
    id: StoredItemId,
) {
    let Some(stored) = doc.store().get(id) else {
        return;
    };
    let bucket = cx.facts.base(cx.game, stored.item()).bucket;
    view.group = bucket.group();
    view.bucket = bucket;
    view.query = Query::default();
    view.mode = StoreMode::Buckets;
    cache.reveal = Some(Reveal {
        id,
        scrolled: false,
    });
}

fn bucket_view(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut StoreView,
    cache: &mut SearchCache,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let store = doc.store();
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
                cache.reveal = None;
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
            if ui.selectable_label(view.bucket == *bucket, label).clicked()
                && view.bucket != *bucket
            {
                view.bucket = *bucket;
                cache.reveal = None;
            }
        }
    });
    search::bar(ui, &mut view.query);
    ui.separator();

    let Filtered {
        shown,
        in_bucket,
        unresolved,
    } = filtered(store.items(), view, cx);
    if !view.query.is_empty() {
        ui.weak(search::summary(shown.len(), in_bucket, unresolved));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if shown.is_empty() {
                ui.weak(if view.query.is_empty() {
                    format!("No {} in the store.", view.bucket.label())
                } else {
                    format!("No {} match the search.", view.bucket.label())
                });
            }
            ui.horizontal_wrapped(|ui| {
                for (name, stored) in &shown {
                    store_tile(ui, name, stored, &mut cache.reveal, cx, frame);
                }
            });
        });
}

fn store_tile(
    ui: &mut Ui,
    name: &str,
    stored: &StoredItem,
    reveal: &mut Option<Reveal>,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click_and_drag());
    let revealed = reveal.is_some_and(|reveal| reveal.id == stored.id());
    if revealed && let Some(pending) = reveal.as_mut().filter(|reveal| !reveal.scrolled) {
        ui.scroll_to_rect(rect, Some(Align::Center));
        pending.scrolled = true;
    }
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
    let symbol = facts.facets.symbol();
    let icon = cx.icons.icon(ui.ctx(), cx.game, bitmap.as_ref());
    let badge_icon = symbol.map(|symbol| cx.icons.symbol(ui.ctx(), symbol));

    let painter = ui.painter_at(rect);
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        if revealed {
            cx.palette.selection_bg
        } else {
            cx.palette.faint_bg
        },
    );
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
            cell: icon_cell(footprint.width, footprint.height),
            badge: symbol
                .zip(badge_icon.as_ref())
                .map(|(symbol, icon)| Badge { symbol, icon }),
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
    if hovered || revealed {
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, cx.palette.selection_stroke),
            StrokeKind::Inside,
        );
    }
    if hovered {
        egui::Tooltip::for_enabled(&response)
            .at_pointer()
            .show(|ui| stored_tooltip(ui, cx, stored));
    }
    if response.clicked() || response.drag_started() {
        *reveal = None;
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
    if response.secondary_clicked() && cx.drag.is_none() {
        frame.right_click = Some(source);
    }
    if response.clicked() && cx.drag.is_none() {
        frame.select = Some(source);
    }
}

/// The item tooltip plus the store's own line: the stored id and
/// where the item came from.
pub fn stored_tooltip(ui: &mut Ui, cx: &mut PaneCtx<'_>, stored: &StoredItem) {
    item_tooltip(ui, cx, stored.item());
    ui.label(
        RichText::new(format!(
            "stored item {} — from {}",
            stored.id(),
            stored.origin()
        ))
        .small()
        .color(cx.palette.text_weak),
    );
}

/// The icon area, keeping the footprint's aspect inside a square box.
fn icon_box(tile: Rect, width: i32, height: i32) -> Rect {
    let (w, h) = (cells(width.max(1)), cells(height.max(1)));
    let scale = icon_cell(width, height);
    let size = vec2(w * scale, h * scale);
    Rect::from_center_size(
        pos2(tile.center().x, tile.min.y + 4.0 + ICON_BOX / 2.0),
        size,
    )
}

/// Points per footprint cell inside the icon box: the longer side of
/// the footprint fills the box.
fn icon_cell(width: i32, height: i32) -> f32 {
    ICON_BOX / cells(width.max(height).max(1))
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

    #[test]
    fn the_view_state_round_trips_and_an_older_file_restores_what_it_has() {
        let view = StoreView {
            mode: StoreMode::Search,
            bucket: Bucket::Ring,
            group: Group::Accessories,
            ..StoreView::default()
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"mode\":\"search\""), "{json}");
        assert_eq!(serde_json::from_str::<StoreView>(&json).unwrap(), view);
        let partial: StoreView = serde_json::from_str(r#"{"bucket":"head"}"#).unwrap();
        assert_eq!(partial.bucket, Bucket::Head);
        assert_eq!(partial.mode, StoreMode::Buckets);
        assert_eq!(partial.search, SearchView::default());
    }
}
