//! Reference cards: floating windows of game facts beside the item
//! panes, opened from the status bar's Reference menu. The first is
//! the affix card — [`AffixTable`] as a searchable table: name, kind,
//! level span and what it grants, one row per name; a click on a row
//! unfolds its records. Whether a card is open and what its bar asks
//! for is view state in `ui-state.json`; which rows are unfolded and
//! the rows the last filter admitted are a session convenience kept
//! beside it, never written.

use std::collections::BTreeSet;

use egui::{Context, RichText, Sense, TextEdit, Ui, vec2};
use egui_extras::{Column, TableBuilder};
use grimvault_core::gamedata::Rarity;
use grimvault_core::reference::{AffixEntry, AffixQuery, AffixTable, Coverage, Position, Tier};
use serde::{Deserialize, Serialize};
use univault_ui::theme::Palette;

use crate::stat_lines::stat_text;
use crate::theme::{UNKNOWN_RARITY, rarity_color};

/// Which cards are open and what their bars ask for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReferenceView {
    pub affixes: AffixCard,
}

/// The affix card's view state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AffixCard {
    pub open: bool,
    pub query: AffixQuery,
}

/// What the cards remember within a session: the rows the last query
/// admitted and the rows unfolded, both as table indices.
#[derive(Debug, Default)]
pub struct ReferenceCache {
    rows: RowCache,
    expanded: BTreeSet<usize>,
}

/// The table indices the last query admitted, refiltered only when
/// the query changes.
#[derive(Debug, Default)]
struct RowCache(Option<(AffixQuery, Vec<usize>)>);

impl RowCache {
    fn rows(&mut self, table: &AffixTable, query: &AffixQuery) -> &[usize] {
        if self.0.as_ref().is_none_or(|(cached, _)| cached != query) {
            let rows = table.matching(query).map(|(index, _)| index).collect();
            self.0 = Some((query.clone(), rows));
        }
        self.0.as_ref().map_or(&[], |(_, rows)| rows.as_slice())
    }
}

/// The status bar's Reference menu: one checkbox per card.
pub fn menu(ui: &mut Ui, view: &mut ReferenceView) {
    ui.menu_button("Reference", |ui| {
        ui.checkbox(&mut view.affixes.open, "Affix names")
            .on_hover_text("Every named prefix and suffix and what it grants");
    });
}

/// Draws every open card.
pub fn show(
    ctx: &Context,
    table: &AffixTable,
    view: &mut ReferenceView,
    cache: &mut ReferenceCache,
    palette: &Palette,
) {
    if !view.affixes.open {
        return;
    }
    let mut open = true;
    egui::Window::new("Affix names")
        .open(&mut open)
        .default_size(vec2(880.0, 520.0))
        .resizable(true)
        .show(ctx, |ui| {
            affix_card(ui, table, &mut view.affixes, cache, palette);
        });
    view.affixes.open = open;
}

fn affix_card(
    ui: &mut Ui,
    table: &AffixTable,
    card: &mut AffixCard,
    cache: &mut ReferenceCache,
    palette: &Palette,
) {
    filter_bar(ui, table, &mut card.query);
    let ReferenceCache { rows, expanded } = cache;
    let rows = rows.rows(table, &card.query);
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!("{} of {} affixes", rows.len(), table.len()));
        ui.separator();
        ui.weak("the records' nominal values; every roll jitters them");
        ui.separator();
        ui.weak("click a row to see its records");
    });
    affix_table(ui, table, rows, expanded, palette);
}

fn filter_bar(ui: &mut Ui, table: &AffixTable, query: &mut AffixQuery) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Search");
        ui.add(
            TextEdit::singleline(&mut query.text)
                .hint_text("name or stat")
                .desired_width(220.0),
        );
        ui.separator();
        ui.selectable_value(&mut query.position, None, "Both");
        for position in Position::ALL {
            ui.selectable_value(
                &mut query.position,
                Some(position),
                position_label(position),
            );
        }
        ui.separator();
        egui::ComboBox::from_id_salt("affix-rarity")
            .selected_text(query.rarity.map_or("Any rarity", Rarity::label))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut query.rarity, None, "Any rarity");
                for rarity in rarities(table) {
                    ui.selectable_value(&mut query.rarity, Some(rarity), rarity.label());
                }
            });
        if !query.is_empty() && ui.small_button("clear").clicked() {
            *query = AffixQuery::default();
        }
    });
}

/// The rarities the table has, in tier order.
fn rarities(table: &AffixTable) -> BTreeSet<Rarity> {
    table
        .entries()
        .iter()
        .filter_map(|entry| entry.rarity)
        .collect()
}

const fn position_label(position: Position) -> &'static str {
    match position {
        Position::Prefix => "Prefixes",
        Position::Suffix => "Suffixes",
    }
}

/// Height budget per line at the card's text size, with slack: over-
/// estimating pads, under-estimating clips the cell.
const LINE_HEIGHT: f32 = 17.0;
const ROW_PAD: f32 = 8.0;
const TEXT_SIZE: f32 = 12.0;

/// A row holds its grants and, unfolded, a heading plus one line per
/// record.
fn row_height(grants: usize, records: usize, expanded: bool) -> f32 {
    let lines = grants.max(1) + if expanded { records + 1 } else { 0 };
    count(lines).mul_add(LINE_HEIGHT, ROW_PAD)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "line counts are small; f32 represents them exactly"
)]
fn count(n: usize) -> f32 {
    n as f32
}

fn affix_table(
    ui: &mut Ui,
    table: &AffixTable,
    rows: &[usize],
    expanded: &mut BTreeSet<usize>,
    palette: &Palette,
) {
    let entries = table.entries();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .sense(Sense::click())
        .column(Column::initial(170.0).at_least(110.0).clip(true))
        .column(Column::initial(110.0).clip(true))
        .column(Column::initial(70.0).clip(true))
        .column(Column::remainder().at_least(240.0).clip(true))
        .header(22.0, |mut header| {
            header.col(|ui| {
                ui.strong("Affix");
            });
            header.col(|ui| {
                ui.strong("Kind");
            });
            header.col(|ui| {
                ui.strong("Level");
            });
            header.col(|ui| {
                ui.strong("Grants");
            });
        })
        .body(|body| {
            let heights: Vec<f32> = rows
                .iter()
                .map(|&index| {
                    let entry = &entries[index];
                    row_height(
                        entry.grants.len(),
                        entry.tiers.len(),
                        expanded.contains(&index),
                    )
                })
                .collect();
            body.heterogeneous_rows(heights.into_iter(), |mut row| {
                let Some(&index) = rows.get(row.index()) else {
                    return;
                };
                let entry = &entries[index];
                let unfolded = expanded.contains(&index);
                let colour = entry.rarity.map_or(UNKNOWN_RARITY, rarity_color);
                row.col(|ui| {
                    let marker = if unfolded { "⏷" } else { "⏵" };
                    ui.label(
                        RichText::new(format!("{marker} {}", entry.name))
                            .color(colour)
                            .size(13.0),
                    );
                });
                row.col(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            entry.rarity.map_or("unclassified", Rarity::label),
                            entry.position.label()
                        ))
                        .color(palette.text_weak)
                        .size(TEXT_SIZE),
                    );
                });
                row.col(|ui| {
                    let text = entry
                        .levels
                        .map_or_else(|| "—".to_string(), |range| format!("Lv {range}"));
                    ui.label(RichText::new(text).size(TEXT_SIZE));
                });
                row.col(|ui| grants_cell(ui, entry, unfolded, palette));
                if row.response().clicked() && !expanded.remove(&index) {
                    expanded.insert(index);
                }
            });
        });
}

/// The grants, each partial one marked with how many records carry
/// it, then — unfolded — every record with its level and lines.
fn grants_cell(ui: &mut Ui, entry: &AffixEntry, unfolded: bool, palette: &Palette) {
    ui.spacing_mut().item_spacing.y = 1.0;
    if entry.grants.is_empty() {
        ui.label(
            RichText::new("no stat lines rendered")
                .italics()
                .color(palette.text_weak)
                .size(TEXT_SIZE),
        );
    }
    for grant in &entry.grants {
        match grant.coverage {
            Coverage::Every => {
                ui.label(stat_text(palette, &grant.line).size(TEXT_SIZE));
            }
            Coverage::Some { tiers, of } => {
                ui.horizontal(|ui| {
                    ui.label(stat_text(palette, &grant.line).size(TEXT_SIZE));
                    ui.label(
                        RichText::new(format!("({tiers} of {of} records)"))
                            .italics()
                            .color(palette.text_weak)
                            .size(TEXT_SIZE - 1.0),
                    );
                });
            }
        }
    }
    if unfolded {
        ui.label(
            RichText::new(format!("{} records, lowest level first", entry.tiers.len()))
                .color(palette.heading)
                .size(TEXT_SIZE - 1.0),
        );
        for tier in &entry.tiers {
            tier_line(ui, tier, palette);
        }
    }
}

fn tier_line(ui: &mut Ui, tier: &Tier, palette: &Palette) {
    let level = tier
        .level
        .map_or_else(|| "—".to_string(), |level| format!("Lv {level}"));
    let lines: Vec<&str> = tier
        .stats
        .lines
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(level)
                .color(palette.text_weak)
                .size(TEXT_SIZE - 1.0),
        );
        ui.label(RichText::new(lines.join(" · ")).size(TEXT_SIZE - 1.0))
            .on_hover_text(tier.record.as_str());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_round_trips_and_older_files_default_it() {
        let view = ReferenceView {
            affixes: AffixCard {
                open: true,
                query: AffixQuery {
                    text: "cleric".into(),
                    position: Some(Position::Prefix),
                    rarity: None,
                },
            },
        };
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("\"open\":true"), "{json}");
        assert_eq!(serde_json::from_str::<ReferenceView>(&json).unwrap(), view);
        assert_eq!(
            serde_json::from_str::<ReferenceView>("{}").unwrap(),
            ReferenceView::default()
        );
        let partial: ReferenceView = serde_json::from_str(r#"{"affixes":{"open":true}}"#).unwrap();
        assert!(partial.affixes.open);
        assert_eq!(partial.affixes.query, AffixQuery::default());
    }

    #[test]
    fn a_row_grows_with_its_grants_and_unfolds_to_its_records() {
        assert!(row_height(0, 6, false) > 0.0);
        assert!((row_height(0, 6, false) - row_height(1, 6, false)).abs() < f32::EPSILON);
        assert!(row_height(5, 6, false) > row_height(1, 6, false));
        let folded = row_height(5, 6, false);
        let unfolded = row_height(5, 6, true);
        assert!((unfolded - folded - 7.0 * LINE_HEIGHT).abs() < f32::EPSILON);
    }

    #[test]
    fn the_row_cache_answers_an_empty_table_with_no_rows() {
        let mut cache = RowCache::default();
        let table = AffixTable::default();
        assert!(cache.rows(&table, &AffixQuery::default()).is_empty());
        let query = AffixQuery {
            text: "x".into(),
            ..AffixQuery::default()
        };
        assert!(cache.rows(&table, &query).is_empty());
        assert_eq!(cache.0.as_ref().map(|(cached, _)| cached), Some(&query));
    }
}
