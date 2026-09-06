//! The Blueprints and Illusions tabs beside the stash's: each lists
//! the campaign's entries as rows — icon, name, and for a blueprint
//! its "new" badge — sorted by name, illusions under one category at
//! a time. The toolbar's Add… opens a searchable picker over the
//! records the database admits and the list lacks; Export… and
//! Import… ask the app for a file dialog. Like every pane, this one
//! only *reports* through [`DragFrame`]; the app performs.

use std::collections::HashSet;

use egui::{CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use grimvault_core::blueprint::available_blueprints;
use grimvault_core::formulas::FormulaRead;
use grimvault_core::gamedata::Rarity;
use grimvault_core::illusion::{IllusionCategory, available_illusions, records_of};
use grimvault_core::item::Item;
use univault_engine::ids::{RecordId, normalize};

use super::{DragFrame, PaneCtx, TileLook, item_tooltip, paint_tile};
use crate::crafting::{Blueprints, Crafting, IllusionCollection, Request};
use crate::documents::Optional;
use crate::grid::footprint_or_unit;
use crate::theme::{UNKNOWN_RARITY, rarity_color};

const ROW_HEIGHT: f32 = 36.0;
const ICON: f32 = 30.0;
const PICKER_ROWS: usize = 200;

/// A record the picker offers, resolved once when the picker opens.
struct Candidate {
    record: RecordId,
    name: String,
    rarity: Option<Rarity>,
}

/// The picker over the records a list lacks.
struct Picker {
    list: Crafting,
    query: String,
    candidates: Vec<Candidate>,
}

/// The pane's selection: the illusion category showing, and the
/// picker while it is open.
#[derive(Default)]
pub struct CraftingView {
    pub category: Option<IllusionCategory>,
    picker: Option<Picker>,
}

/// One listed entry.
struct Row {
    item: Item,
    name: String,
    badge: Option<&'static str>,
}

pub fn show_blueprints(
    ui: &mut Ui,
    blueprints: &Blueprints,
    view: &mut CraftingView,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let Some(doc) = editable(ui, blueprints, Crafting::Blueprints, cx) else {
        return;
    };
    let formulas = doc.formulas();
    let unread = formulas
        .entries
        .iter()
        .filter(|entry| entry.read == FormulaRead::Unread)
        .count();
    let opened = toolbar(
        ui,
        Crafting::Blueprints,
        format!("{} known · {unread} new", formulas.entries.len()),
        frame,
    );
    if opened {
        let known: HashSet<String> = formulas
            .entries
            .iter()
            .map(|entry| normalize(&entry.record))
            .collect();
        let candidates = available_blueprints(cx.game)
            .into_iter()
            .filter(|record| !known.contains(&normalize(record.as_str())))
            .collect();
        view.picker = Some(picker_of(Crafting::Blueprints, candidates, cx));
    }
    let mut rows: Vec<Row> = formulas
        .entries
        .iter()
        .map(|entry| {
            let item = record_item(&entry.record);
            let name = cx.facts.base(cx.game, &item).name.clone();
            Row {
                item,
                name,
                badge: (entry.read == FormulaRead::Unread).then_some("NEW"),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    show_picker(ui, view, frame);
    show_rows(ui, &rows, "No blueprints learned in this campaign.", cx);
}

pub fn show_illusions(
    ui: &mut Ui,
    illusions: &IllusionCollection,
    view: &mut CraftingView,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let Some(doc) = editable(ui, illusions, Crafting::Illusions, cx) else {
        return;
    };
    let collection = doc.illusions();
    let opened = toolbar(
        ui,
        Crafting::Illusions,
        format!(
            "{} unlocked in {} categories",
            collection.total_count(),
            collection.slots.len()
        ),
        frame,
    );
    if opened {
        let known: HashSet<String> = collection
            .slots
            .iter()
            .flat_map(|slot| slot.records.iter())
            .map(|record| normalize(record))
            .collect();
        let candidates = available_illusions(cx.game)
            .into_iter()
            .filter(|record| !known.contains(&normalize(record.as_str())))
            .collect();
        view.picker = Some(picker_of(Crafting::Illusions, candidates, cx));
    }
    ui.horizontal_wrapped(|ui| {
        for category in IllusionCategory::ALL {
            let count = records_of(collection, category).len();
            let selected = view.category == Some(category);
            if ui
                .selectable_label(selected, format!("{} ({count})", category.label()))
                .clicked()
            {
                view.category = Some(category);
            }
        }
    });
    let category = view.category.unwrap_or(IllusionCategory::Head);
    let mut rows: Vec<Row> = records_of(collection, category)
        .iter()
        .map(|record| {
            let item = record_item(record);
            let name = cx.facts.base(cx.game, &item).name.clone();
            Row {
                item,
                name,
                badge: None,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    show_picker(ui, view, frame);
    show_rows(
        ui,
        &rows,
        &format!("No {} illusions unlocked.", category.label().to_lowercase()),
        cx,
    );
}

/// The open document, or the reason the list cannot be shown.
fn editable<'d, D: crate::documents::Document>(
    ui: &mut Ui,
    file: &'d Optional<D>,
    list: Crafting,
    cx: &PaneCtx<'_>,
) -> Option<&'d D> {
    match file {
        Optional::Open(doc) => Some(doc),
        Optional::Absent { path } => {
            ui.weak(format!(
                "{} does not exist yet: the game writes it once the first {} is learned in this campaign.",
                path.display(),
                list.one()
            ));
            None
        }
        Optional::Failed { path, error, .. } => {
            ui.colored_label(
                cx.palette.error,
                format!("{} cannot be edited: {error}", path.display()),
            );
            None
        }
    }
}

/// The count line and the three actions; `true` when Add… was clicked.
fn toolbar(ui: &mut Ui, list: Crafting, summary: String, frame: &mut DragFrame) -> bool {
    let mut opened = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(summary);
        ui.separator();
        opened = ui.button(format!("Add {}…", list.one())).clicked();
        if ui.button("Export…").clicked() {
            frame.crafting = Some(Request::Export(list));
        }
        if ui.button("Import…").clicked() {
            frame.crafting = Some(Request::Import(list));
        }
    });
    opened
}

fn picker_of(list: Crafting, records: Vec<RecordId>, cx: &mut PaneCtx<'_>) -> Picker {
    let mut candidates: Vec<Candidate> = records
        .into_iter()
        .map(|record| {
            let base = cx.facts.base(cx.game, &record_item(record.as_str()));
            Candidate {
                name: base.name.clone(),
                rarity: base.rarity,
                record,
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    Picker {
        list,
        query: String::new(),
        candidates,
    }
}

/// The picker window while open: a query over names and record paths,
/// the matches as buttons. Choosing one reports the add and closes.
fn show_picker(ui: &mut Ui, view: &mut CraftingView, frame: &mut DragFrame) {
    let Some(picker) = view.picker.as_mut() else {
        return;
    };
    let mut open = true;
    let mut chosen = None;
    egui::Window::new(format!("Add {}", picker.list.one()))
        .open(&mut open)
        .default_size(vec2(520.0, 480.0))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("search");
                ui.add(
                    egui::TextEdit::singleline(&mut picker.query)
                        .hint_text("name or record path")
                        .desired_width(360.0),
                );
            });
            let query = picker.query.to_lowercase();
            let matches: Vec<&Candidate> = picker
                .candidates
                .iter()
                .filter(|candidate| {
                    query.is_empty()
                        || candidate.name.to_lowercase().contains(&query)
                        || candidate.record.as_str().to_lowercase().contains(&query)
                })
                .collect();
            ui.weak(format!(
                "{} of {} not yet in the list",
                matches.len(),
                picker.candidates.len()
            ));
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for candidate in matches.iter().take(PICKER_ROWS) {
                        let colour = candidate.rarity.map_or(UNKNOWN_RARITY, rarity_color);
                        let response = ui
                            .selectable_label(false, RichText::new(&candidate.name).color(colour));
                        let response = response.on_hover_text(candidate.record.as_str());
                        if response.clicked() {
                            chosen = Some(candidate.record.clone());
                        }
                    }
                    if matches.len() > PICKER_ROWS {
                        ui.weak(format!(
                            "and {} more — narrow the search",
                            matches.len() - PICKER_ROWS
                        ));
                    }
                });
        });
    if let Some(record) = chosen {
        frame.crafting = Some(Request::Add {
            list: picker.list,
            record,
        });
        open = false;
    }
    if !open {
        view.picker = None;
    }
}

fn show_rows(ui: &mut Ui, rows: &[Row], empty: &str, cx: &mut PaneCtx<'_>) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.weak(empty);
            }
            for row in rows {
                entry_row(ui, row, cx);
            }
        });
}

fn entry_row(ui: &mut Ui, row: &Row, cx: &mut PaneCtx<'_>) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let hovered = response.hovered();
    let facts = cx.facts.facts(cx.game, &row.item);
    let (_, footprint_source) = footprint_or_unit(facts.base.footprint);
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
            badge: None,
            hovered,
            lifted: false,
        },
        cx.palette,
    );
    painter.text(
        egui::pos2(icon_rect.max.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &row.name,
        egui::FontId::proportional(13.0),
        rarity.map_or(cx.palette.text, rarity_color),
    );
    if let Some(badge) = row.badge {
        let galley = painter.layout_no_wrap(
            badge.to_string(),
            egui::FontId::proportional(11.0),
            cx.palette.text_strong,
        );
        let badge_rect = Rect::from_min_size(
            egui::pos2(
                rect.max.x - galley.size().x - 16.0,
                rect.center().y - galley.size().y / 2.0 - 3.0,
            ),
            galley.size() + vec2(12.0, 6.0),
        );
        painter.rect_filled(badge_rect, CornerRadius::same(8), cx.palette.accent_dim);
        painter.galley(
            badge_rect.min + vec2(6.0, 3.0),
            galley,
            cx.palette.text_strong,
        );
    }
    if hovered {
        painter.rect_stroke(
            rect,
            CornerRadius::same(3),
            Stroke::new(1.0, cx.palette.selection_stroke),
            StrokeKind::Inside,
        );
        egui::Tooltip::for_enabled(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, cx, &row.item));
    }
}

/// The item a list entry stands for: its record, nothing else.
fn record_item(record: &str) -> Item {
    Item {
        base_name: record.to_string(),
        ..Item::default()
    }
}

/// The label of a crafting tab: the list and its count, or a dash
/// when the file is absent or unusable.
pub fn tab_label<D: crate::documents::Document>(
    list: Crafting,
    file: &Optional<D>,
    count: impl FnOnce(&D) -> usize,
) -> String {
    match file {
        Optional::Open(doc) => format!("{} ({})", list.label(), count(doc)),
        Optional::Absent { .. } | Optional::Failed { .. } => format!("{} (–)", list.label()),
    }
}
