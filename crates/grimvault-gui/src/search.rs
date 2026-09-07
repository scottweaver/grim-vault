// The filter bar, suggesting field, sort header and table layout are
// ported in shape from tq-univault crates/univault-gui/src/search.rs @ 36e7774
// (MIT OR Apache-2.0, same author), over grim-vault's typed Query.
//! The store's two search surfaces over the core [`Query`]: the bar
//! the bucket view carries (a name and the facet toggles) and the
//! search view — everything in the store as one filtered, sorted table
//! (icon | name | rarity | requirements | type | stats) shown in the
//! store pane's place so the game-file panes stay live beside it.
//! Rows are drag sources like the tiles, double-click reveals the item
//! in its bucket, and hovering shows the item tooltip. The view state
//! ([`SearchView`]) persists; the rows ([`SearchCache`]) are derived
//! from the store and rebuilt when it, the query, or the sort change.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::str::FromStr;

use egui::{Id, RichText, Sense, TextEdit, Ui, vec2};
use egui_extras::{Column, TableBuilder, TableRow};
use grimvault_core::bucket::{Bucket, Group};
use grimvault_core::gamedata::Rarity;
use grimvault_core::search::{
    AscensionFilter, CategoryFilter, Constraint, Criterion, Query, Scope, SetFilter, SocketFilter,
    SortKey, SortRank, Verdict,
};
use grimvault_core::stats::{self, Block, BlockSource, ItemDetails, Requirement};
use grimvault_core::store::{StoredItem, StoredItemId};
use serde::{Deserialize, Serialize};
use univault_ui::sort::SortDirection;

use crate::documents::StoreDoc;
use crate::drag::{DragSource, DragState};
use crate::grid::{CELL_PX, footprint_or_unit};
use crate::panes::store::{Reveal, stored_tooltip};
use crate::panes::{DragFrame, PaneCtx, paint_item};
use crate::theme::{UNKNOWN_RARITY, rarity_color};

/// Draws the bucket view's bar; `true` when the query changed this
/// frame.
pub fn bar(ui: &mut Ui, query: &mut Query) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Search");
        changed |= ui
            .add(
                TextEdit::singleline(&mut query.name)
                    .hint_text("name")
                    .desired_width(160.0),
            )
            .changed();
        changed |= facet_toggles(ui, query);
        if !query.is_empty() && ui.small_button("clear").clicked() {
            *query = Query::default();
            changed = true;
        }
    });
    changed
}

/// The monster-infrequent and double-rare toggles and the ascension
/// choice; `true` when one changed.
fn facet_toggles(ui: &mut Ui, query: &mut Query) -> bool {
    let mut changed = toggle(ui, &mut query.monster_infrequent, "Monster infrequent");
    changed |= toggle(ui, &mut query.double_rare, "Double rare");
    let before = query.ascension;
    egui::ComboBox::from_id_salt(ui.id().with("ascension-filter"))
        .selected_text(ascension_label(query.ascension))
        .show_ui(ui, |ui| {
            for choice in [
                AscensionFilter::Any,
                AscensionFilter::Upgradeable,
                AscensionFilter::Ascended,
            ] {
                ui.selectable_value(&mut query.ascension, choice, ascension_label(choice));
            }
        });
    changed || query.ascension != before
}

fn toggle(ui: &mut Ui, constraint: &mut Constraint, label: &str) -> bool {
    let clicked = ui
        .selectable_label(constraint.is_required(), label)
        .clicked();
    if clicked {
        *constraint = constraint.toggled();
    }
    clicked
}

const fn ascension_label(filter: AscensionFilter) -> &'static str {
    match filter {
        AscensionFilter::Any => "Any ascension",
        AscensionFilter::Upgradeable => "Upgradeable",
        AscensionFilter::Ascended => "Ascended",
    }
}

/// What the filter did, for the line under the bucket bar: how many
/// of the bucket's items it kept, and how many it could not decide
/// about.
#[must_use]
pub fn summary(shown: usize, total: usize, unresolved: usize) -> String {
    let hidden = total.saturating_sub(shown);
    match (hidden, unresolved) {
        (0, _) => format!("{total} shown"),
        (_, 0) => format!("{shown} of {total} match"),
        (_, _) => format!(
            "{shown} of {total} match · {unresolved} hidden because a required facet could not \
             be resolved for them"
        ),
    }
}

/// The search view's line: what the table shows of the store and how
/// many items no constraint could be decided for.
#[must_use]
pub fn search_summary(shown: usize, total: usize, unresolved: usize) -> String {
    if unresolved == 0 {
        format!("{shown} of {total} shown")
    } else {
        format!("{shown} of {total} shown · {unresolved} unresolved")
    }
}

/// The table's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub key: SortKey,
    pub direction: SortDirection,
}

impl Default for Sort {
    fn default() -> Self {
        Self::by(SortKey::Name)
    }
}

impl Sort {
    /// A column in the direction it reads best when first picked:
    /// names and types A→Z, rarity and level highest first.
    #[must_use]
    pub fn by(key: SortKey) -> Self {
        let direction = match key {
            SortKey::Name | SortKey::Category => SortDirection::Ascending,
            SortKey::Rarity | SortKey::Level => SortDirection::Descending,
        };
        Self { key, direction }
    }

    /// A header click: the same column flips, another opens naturally.
    #[must_use]
    pub fn clicked(self, key: SortKey) -> Self {
        if self.key == key {
            Self {
                key,
                direction: self.direction.flipped(),
            }
        } else {
            Self::by(key)
        }
    }
}

/// The search view's persisted state: the bar and the sort.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchView {
    pub query: Query,
    pub sort: Sort,
}

/// One store entry as the table needs it, resolved once per store
/// revision: its name, the facts sorting compares, and its rendered
/// stat body.
struct Indexed {
    id: StoredItemId,
    index: usize,
    name: String,
    rarity: Option<Rarity>,
    bucket: Option<Bucket>,
    rank: SortRank,
    details: ItemDetails,
    height: f32,
}

/// The rows and vocabularies derived from the store — never
/// persisted, rebuilt when the store changes (the index), the query
/// changes (the rows), or the sort changes (their order).
#[derive(Default)]
pub struct SearchCache {
    indexed_at: Option<u64>,
    index: Vec<Indexed>,
    templates: Vec<String>,
    affixes: Vec<String>,
    filtered_for: Option<Query>,
    rows: Vec<usize>,
    unresolved: usize,
    sorted_by: Option<Sort>,
    selected: Option<StoredItemId>,
    /// Set by the shortcut that opens the view; the name field takes
    /// focus on the next frame and clears it.
    pub focus_requested: bool,
    /// The item the last double-click asked the bucket view to show.
    pub reveal: Option<Reveal>,
}

impl SearchCache {
    fn refresh(&mut self, doc: &StoreDoc, view: &SearchView, cx: &mut PaneCtx<'_>) {
        if self.indexed_at != Some(doc.revision()) {
            self.rebuild_index(doc, cx);
            self.filtered_for = None;
        }
        if self.filtered_for.as_ref() != Some(&view.query) {
            self.filter(doc, &view.query, cx);
            self.sorted_by = None;
        }
        if self.sorted_by != Some(view.sort) {
            self.sort(view.sort);
        }
    }

    fn rebuild_index(&mut self, doc: &StoreDoc, cx: &mut PaneCtx<'_>) {
        let mut templates = BTreeSet::new();
        let mut affixes = BTreeSet::new();
        self.index = doc
            .store()
            .items()
            .iter()
            .enumerate()
            .map(|(index, stored)| {
                let item = stored.item();
                let details = stats::item_details(cx.game, item);
                let facts = cx.facts.facts(cx.game, item);
                let name = facts.display_name();
                let rank = SortRank::of(&facts.subject(item, &name, Some(&details)));
                affixes.extend(
                    facts
                        .prefix
                        .into_iter()
                        .chain(facts.suffix)
                        .map(str::to_string),
                );
                templates.extend(
                    details
                        .blocks
                        .iter()
                        .flat_map(|block| block.lines.iter())
                        .filter(|line| grimvault_core::search::is_searchable(line))
                        .filter(|line| !line.text.trim().is_empty())
                        .map(|line| grimvault_core::search::stat_template(&line.text)),
                );
                Indexed {
                    id: stored.id(),
                    index,
                    name,
                    rarity: facts.facets.displayed_rarity(),
                    bucket: matches!(facts.base.record, crate::facts::RecordStatus::Known)
                        .then_some(facts.base.bucket),
                    rank,
                    height: row_height(&details),
                    details,
                }
            })
            .collect();
        self.templates = templates.into_iter().collect();
        self.affixes = affixes.into_iter().collect();
        self.indexed_at = Some(doc.revision());
    }

    fn filter(&mut self, doc: &StoreDoc, query: &Query, cx: &mut PaneCtx<'_>) {
        let items = doc.store().items();
        let mut unresolved = 0;
        let index = &self.index;
        self.rows = index
            .iter()
            .enumerate()
            .filter_map(|(slot, indexed)| {
                let item = items.get(indexed.index)?.item();
                let facts = cx.facts.facts(cx.game, item);
                let subject = facts.subject(item, &indexed.name, Some(&indexed.details));
                match query.verdict(&subject) {
                    Verdict::Matches => Some(slot),
                    Verdict::Excluded => None,
                    Verdict::Unresolved => {
                        unresolved += 1;
                        None
                    }
                }
            })
            .collect();
        self.unresolved = unresolved;
        self.filtered_for = Some(query.clone());
    }

    fn sort(&mut self, sort: Sort) {
        let index = &self.index;
        self.rows.sort_by(|a, b| {
            let (a, b) = (&index[*a], &index[*b]);
            sort.direction
                .apply(a.rank.compare(&b.rank, sort.key).then(a.id.cmp(&b.id)))
        });
        self.sorted_by = Some(sort);
    }
}

/// The size of the icon column's tile box.
const ICON: f32 = 40.0;
/// Height budget per stat line at the row's text size, with slack:
/// over-estimating pads, under-estimating clips the cell.
const LINE_HEIGHT: f32 = 17.0;
const BLOCK_GAP: f32 = 4.0;
const ROW_PAD: f32 = 8.0;
const TEXT_SIZE: f32 = 12.0;

fn row_height(details: &ItemDetails) -> f32 {
    let shown: Vec<&Block> = details.blocks.iter().filter(|block| shows(block)).collect();
    let lines = shown
        .iter()
        .map(|block| block.lines.len() + usize::from(block_title(block).is_some()))
        .sum::<usize>();
    let gaps = shown.len().saturating_sub(1);
    count(lines)
        .mul_add(LINE_HEIGHT, count(gaps) * BLOCK_GAP + ROW_PAD)
        .max(ICON + ROW_PAD)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "line and block counts are tiny; f32 represents them exactly"
)]
fn count(n: usize) -> f32 {
    n as f32
}

fn shows(block: &Block) -> bool {
    !block.lines.is_empty()
}

/// The heading a non-base block gets in the stats column: the
/// component's or augment's own name, or the block's label.
fn block_title(block: &Block) -> Option<String> {
    match block.source {
        BlockSource::Base | BlockSource::Prefix | BlockSource::Suffix => None,
        BlockSource::Modifier
        | BlockSource::Component
        | BlockSource::CompletionBonus
        | BlockSource::Augment
        | BlockSource::Ascendant
        | BlockSource::AscendantTwoHanded => Some(
            block
                .title
                .clone()
                .unwrap_or_else(|| block.source.label().to_string()),
        ),
    }
}

/// Draws the search view: the filter bar, the summary line, and the
/// results table. Returns the item a double-click asked to reveal in
/// its bucket.
pub fn show_view(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut SearchView,
    cache: &mut SearchCache,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Option<StoredItemId> {
    cache.refresh(doc, view, cx);
    let focus = std::mem::take(&mut cache.focus_requested);
    filter_bar(ui, &mut view.query, &cache.templates, &cache.affixes, focus);
    cache.refresh(doc, view, cx);
    ui.horizontal(|ui| {
        ui.weak(search_summary(
            cache.rows.len(),
            cache.index.len(),
            cache.unresolved,
        ));
        if !view.query.is_empty() && ui.small_button("Clear all").clicked() {
            view.query = Query::default();
        }
    });
    ui.separator();
    table(ui, doc, view, cache, cx, frame)
}

fn filter_bar(
    ui: &mut Ui,
    query: &mut Query,
    templates: &[String],
    affixes: &[String],
    focus: bool,
) {
    ui.horizontal_wrapped(|ui| {
        let name = ui.add(
            TextEdit::singleline(&mut query.name)
                .hint_text("Name contains…")
                .desired_width(180.0),
        );
        if focus {
            name.request_focus();
        }
        set_fields(ui, &mut query.set);
    });
    criteria_rows(ui, &mut query.criteria, templates, affixes);
    ui.horizontal_wrapped(|ui| {
        ui.label("Wearable at ≤");
        for requirement in Requirement::ALL {
            ui.label(requirement.label());
            number_field(
                ui,
                ui.id().with(("cap", requirement.label())),
                query.requirements.slot(requirement),
                "–",
                44.0,
            );
        }
    });
    ui.horizontal_wrapped(|ui| {
        rarity_choice(ui, &mut query.rarity);
        category_choice(ui, &mut query.category);
        socket_choice(ui, &mut query.socket);
        facet_toggles(ui, query);
    });
}

/// The set name field and the "set items only" box: a name means
/// that set, the box alone means any set.
fn set_fields(ui: &mut Ui, set: &mut SetFilter) {
    let (mut text, mut member) = match set {
        SetFilter::Any => (String::new(), false),
        SetFilter::Member => (String::new(), true),
        SetFilter::Named { text } => (text.clone(), true),
    };
    let typed = ui
        .add(
            TextEdit::singleline(&mut text)
                .hint_text("Set name…")
                .desired_width(140.0),
        )
        .changed();
    let ticked = ui.checkbox(&mut member, "set items only").changed();
    if typed || ticked {
        *set = match (text.trim().is_empty(), member) {
            (true, false) => SetFilter::Any,
            (true, true) => SetFilter::Member,
            (false, _) => SetFilter::Named { text },
        };
    }
}

/// The dynamic criteria list: one row per stat or affix conjunct with
/// a scope, a suggesting text, and a value window; rows come and go
/// freely and the bar always shows at least one.
fn criteria_rows(
    ui: &mut Ui,
    criteria: &mut Vec<Criterion>,
    templates: &[String],
    affixes: &[String],
) {
    if criteria.is_empty() {
        criteria.push(Criterion::default());
    }
    let mut remove = None;
    for (index, criterion) in criteria.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut scope = criterion.scope();
            egui::ComboBox::from_id_salt(("criterion-scope", index))
                .selected_text(scope.label())
                .width(96.0)
                .show_ui(ui, |ui| {
                    for choice in Scope::ALL {
                        ui.selectable_value(&mut scope, choice, choice.label());
                    }
                });
            if scope != criterion.scope() {
                *criterion = std::mem::take(criterion).with_scope(scope);
            }
            let vocab = match scope {
                Scope::AnyStat | Scope::AffixStat => templates,
                Scope::AffixName => affixes,
            };
            suggesting_field(
                ui,
                ("criterion-text", index),
                criterion.text_mut(),
                "type or pick…",
                220.0,
                vocab,
            );
            if let Some(bounds) = criterion.bounds_mut() {
                number_field(
                    ui,
                    ui.id().with(("min", index)),
                    &mut bounds.min,
                    "min",
                    44.0,
                );
                ui.label("–");
                number_field(
                    ui,
                    ui.id().with(("max", index)),
                    &mut bounds.max,
                    "max",
                    44.0,
                );
            }
            if ui
                .button("✕")
                .on_hover_text("Remove this criterion")
                .clicked()
            {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        criteria.remove(index);
    }
    if ui.button("＋ Add stat / affix criterion").clicked() {
        criteria.push(Criterion::default());
    }
}

/// How many suggestions the autocomplete popup offers at once.
const SUGGESTION_LIMIT: usize = 8;

/// A text field with an autocomplete popup fed from `vocab`. The
/// popup also renders on the frame focus is lost so a click on a
/// suggestion lands before it closes.
fn suggesting_field(
    ui: &mut Ui,
    id_salt: (&str, usize),
    value: &mut String,
    hint: &str,
    width: f32,
    vocab: &[String],
) {
    let response = ui.add(
        TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(width),
    );
    if !(response.has_focus() || response.lost_focus()) {
        return;
    }
    let needle = value.to_lowercase();
    let matching: Vec<&String> = vocab
        .iter()
        .filter(|entry| entry.to_lowercase().contains(&needle))
        .take(SUGGESTION_LIMIT)
        .collect();
    if matching.is_empty() || (matching.len() == 1 && *matching[0] == *value) {
        return;
    }
    egui::Area::new(Id::new(id_salt).with("suggestions"))
        .order(egui::Order::Foreground)
        .fixed_pos(response.rect.left_bottom() + vec2(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(response.rect.width());
                for entry in matching {
                    if ui.selectable_label(false, entry).clicked() {
                        value.clone_from(entry);
                    }
                }
            });
        });
}

/// A text field over an optional number: the text as typed lives in
/// egui's frame memory while it is being edited, the parsed value is
/// the only state kept, and a value changed elsewhere (a clear) resets
/// the text once the field is not focused.
fn number_field<T>(ui: &mut Ui, id: Id, value: &mut Option<T>, hint: &str, width: f32)
where
    T: Copy + PartialEq + FromStr + Display,
{
    let shown = |value: Option<T>| value.map_or_else(String::new, |value| value.to_string());
    let mut text: String = ui
        .data_mut(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| shown(*value));
    let response = ui.add(
        TextEdit::singleline(&mut text)
            .hint_text(hint)
            .desired_width(width),
    );
    let parsed = text.trim().parse::<T>().ok();
    if response.changed() {
        *value = parsed;
    } else if !response.has_focus() && parsed != *value {
        text = shown(*value);
    }
    ui.data_mut(|data| data.insert_temp(id, text));
}

fn rarity_choice(ui: &mut Ui, rarity: &mut Option<Rarity>) {
    egui::ComboBox::from_id_salt("search-rarity")
        .selected_text(rarity.map_or("Any rarity", Rarity::label))
        .show_ui(ui, |ui| {
            ui.selectable_value(rarity, None, "Any rarity");
            for choice in Rarity::ALL {
                ui.selectable_value(rarity, Some(choice), choice.label());
            }
        });
}

fn category_label(category: CategoryFilter) -> String {
    match category {
        CategoryFilter::Any => "Any type".to_string(),
        CategoryFilter::Group { group } => format!("All {}", group.label()),
        CategoryFilter::Bucket { bucket } => bucket.label().to_string(),
    }
}

/// Every group, each followed by its buckets.
fn category_choice(ui: &mut Ui, category: &mut CategoryFilter) {
    egui::ComboBox::from_id_salt("search-category")
        .selected_text(category_label(*category))
        .show_ui(ui, |ui| {
            ui.selectable_value(category, CategoryFilter::Any, "Any type");
            for group in Group::ALL {
                let all = CategoryFilter::Group { group };
                ui.selectable_value(category, all, RichText::new(category_label(all)).strong());
                for bucket in Bucket::ALL.iter().filter(|bucket| bucket.group() == group) {
                    let one = CategoryFilter::Bucket { bucket: *bucket };
                    ui.selectable_value(category, one, format!("    {}", bucket.label()));
                }
            }
        });
}

const fn socket_label(socket: SocketFilter) -> &'static str {
    match socket {
        SocketFilter::Any => "Socketed or not",
        SocketFilter::Socketed => "With component",
        SocketFilter::Unsocketed => "Without component",
    }
}

fn socket_choice(ui: &mut Ui, socket: &mut SocketFilter) {
    egui::ComboBox::from_id_salt("search-socket")
        .selected_text(socket_label(*socket))
        .show_ui(ui, |ui| {
            for choice in [
                SocketFilter::Any,
                SocketFilter::Socketed,
                SocketFilter::Unsocketed,
            ] {
                ui.selectable_value(socket, choice, socket_label(choice));
            }
        });
}

/// A column header that sorts by its key and shows the direction in
/// force.
fn sort_header(ui: &mut Ui, label: &str, key: SortKey, sort: &mut Sort) {
    let arrow = if sort.key == key {
        format!(" {}", sort.direction.arrow())
    } else {
        String::new()
    };
    let hint = if sort.key == key {
        sort.direction.flip_hint()
    } else {
        "Sort by this column"
    };
    if ui
        .add(egui::Button::new(RichText::new(format!("{label}{arrow}")).strong()).frame(false))
        .on_hover_text(hint)
        .clicked()
    {
        *sort = sort.clicked(key);
    }
}

fn table(
    ui: &mut Ui,
    doc: &StoreDoc,
    view: &mut SearchView,
    cache: &mut SearchCache,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Option<StoredItemId> {
    let mut sort = view.sort;
    let mut reveal = None;
    let items = doc.store().items();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(Sense::click_and_drag())
        .column(Column::exact(ICON + 8.0))
        .column(Column::initial(190.0).at_least(120.0).clip(true))
        .column(Column::initial(72.0).clip(true))
        .column(Column::initial(84.0).clip(true))
        .column(Column::initial(110.0).clip(true))
        .column(Column::remainder().clip(true))
        .header(22.0, |mut header| {
            header.col(|_| {});
            header.col(|ui| sort_header(ui, "Name", SortKey::Name, &mut sort));
            header.col(|ui| sort_header(ui, "Rarity", SortKey::Rarity, &mut sort));
            header.col(|ui| sort_header(ui, "Requires", SortKey::Level, &mut sort));
            header.col(|ui| sort_header(ui, "Type", SortKey::Category, &mut sort));
            header.col(|ui| {
                ui.strong("Stats");
            });
        })
        .body(|body| {
            let SearchCache {
                index,
                rows,
                selected,
                ..
            } = cache;
            let heights = rows.iter().map(|slot| index[*slot].height);
            body.heterogeneous_rows(heights, |mut table_row| {
                let Some(indexed) = rows.get(table_row.index()).map(|slot| &index[*slot]) else {
                    return;
                };
                let Some(stored) = items
                    .get(indexed.index)
                    .filter(|stored| stored.id() == indexed.id)
                else {
                    return;
                };
                table_row.set_selected(*selected == Some(indexed.id));
                if let Some(id) = row(&mut table_row, indexed, stored, selected, cx, frame) {
                    reveal = Some(id);
                }
            });
        });
    if sort != view.sort {
        view.sort = sort;
    }
    reveal
}

/// One row: the tile, the coloured name, rarity, requirements, type,
/// and the stat lines — plus the gestures: click selects, drag lifts
/// the item like a tile, double-click asks to reveal it in its bucket,
/// right-click moves it back into the game as a tile's would.
fn row(
    table_row: &mut TableRow<'_, '_>,
    indexed: &Indexed,
    stored: &StoredItem,
    selected: &mut Option<StoredItemId>,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Option<StoredItemId> {
    let item = stored.item();
    let source = DragSource::Store(stored.id());
    let lifted = cx.drag.is_some_and(|drag| drag.source == source);
    let (footprint, footprint_source) = footprint_or_unit(cx.facts.base(cx.game, item).footprint);
    let (_, icon_response) = table_row.col(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
        let longest = cells(footprint.width.max(footprint.height).max(1));
        let cell = ICON / longest;
        let size = vec2(
            cells(footprint.width.max(1)),
            cells(footprint.height.max(1)),
        ) * cell;
        let tile = egui::Rect::from_center_size(rect.center(), size);
        paint_item(
            ui.ctx(),
            &ui.painter_at(rect),
            tile,
            item,
            footprint_source,
            cell,
            false,
            lifted,
            cx,
        );
    });
    let colour = indexed.rarity.map_or(UNKNOWN_RARITY, rarity_color);
    let (_, name_response) = table_row.col(|ui| {
        ui.label(RichText::new(&indexed.name).color(colour).size(13.0));
    });
    if cx.drag.is_none() {
        for response in [&icon_response, &name_response] {
            egui::Tooltip::for_enabled(response)
                .at_pointer()
                .show(|ui| stored_tooltip(ui, cx, stored));
        }
    }
    table_row.col(|ui| {
        ui.label(
            RichText::new(indexed.rarity.map_or("?", Rarity::label))
                .color(cx.palette.text_weak)
                .size(TEXT_SIZE),
        );
    });
    table_row.col(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (requirement, value) in &indexed.details.requirements {
            let text = match requirement {
                Requirement::Level => RichText::new(format!("Lv {value}")).size(TEXT_SIZE),
                Requirement::Physique | Requirement::Cunning | Requirement::Spirit => {
                    RichText::new(format!("{} {value}", requirement.label()))
                        .size(TEXT_SIZE - 1.0)
                        .color(cx.palette.text_weak)
                }
            };
            ui.label(text);
        }
    });
    table_row.col(|ui| {
        ui.label(
            RichText::new(indexed.bucket.map_or("unknown record", Bucket::label)).size(TEXT_SIZE),
        );
    });
    table_row.col(|ui| stat_column(ui, &indexed.details, cx));
    let response = table_row.response();
    if response.drag_started() && cx.drag.is_none() && frame.begin.is_none() {
        let ghost = vec2(CELL_PX, CELL_PX) * vec2(cells(footprint.width), cells(footprint.height));
        frame.begin = Some(DragState {
            source,
            item: item.clone(),
            footprint,
            grab: ghost / 2.0,
        });
    }
    if response.secondary_clicked() && cx.drag.is_none() {
        frame.right_click = Some(source);
    }
    if response.double_clicked() && cx.drag.is_none() {
        return Some(stored.id());
    }
    if response.clicked() {
        *selected = Some(stored.id());
    }
    None
}

/// The stat body, block by block, each non-base block under its
/// heading.
fn stat_column(ui: &mut Ui, details: &ItemDetails, cx: &PaneCtx<'_>) {
    ui.spacing_mut().item_spacing.y = 1.0;
    let mut drawn_any = false;
    for block in details.blocks.iter().filter(|block| shows(block)) {
        if drawn_any {
            ui.add_space(BLOCK_GAP);
        }
        drawn_any = true;
        if let Some(title) = block_title(block) {
            ui.label(
                RichText::new(title)
                    .color(cx.palette.heading)
                    .strong()
                    .size(TEXT_SIZE),
            );
        }
        for line in &block.lines {
            ui.label(crate::stat_lines::stat_text(cx.palette, line).size(TEXT_SIZE));
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

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    /// The order two rows take under a sort, for hand-built ranks.
    fn ordered(sort: Sort, a: &SortRank, b: &SortRank) -> Ordering {
        sort.direction.apply(a.compare(b, sort.key))
    }

    #[test]
    fn the_summaries_name_what_the_filter_hid() {
        assert_eq!(summary(5, 5, 0), "5 shown");
        assert_eq!(summary(2, 5, 0), "2 of 5 match");
        assert!(summary(2, 5, 1).starts_with("2 of 5 match · 1 hidden"));
        assert_eq!(search_summary(3, 10, 0), "3 of 10 shown");
        assert_eq!(search_summary(3, 10, 2), "3 of 10 shown · 2 unresolved");
    }

    #[test]
    fn every_ascension_choice_has_a_distinct_label() {
        let labels = [
            AscensionFilter::Any,
            AscensionFilter::Upgradeable,
            AscensionFilter::Ascended,
        ]
        .map(ascension_label);
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[1], labels[2]);
    }

    #[test]
    fn a_fresh_column_opens_in_its_natural_direction_and_a_repeat_flips() {
        assert_eq!(Sort::by(SortKey::Name).direction, SortDirection::Ascending);
        assert_eq!(
            Sort::by(SortKey::Category).direction,
            SortDirection::Ascending
        );
        assert_eq!(
            Sort::by(SortKey::Rarity).direction,
            SortDirection::Descending
        );
        assert_eq!(
            Sort::by(SortKey::Level).direction,
            SortDirection::Descending
        );
        let sort = Sort::default();
        assert_eq!(
            sort.clicked(SortKey::Name).direction,
            SortDirection::Descending
        );
        assert_eq!(sort.clicked(SortKey::Level), Sort::by(SortKey::Level));
    }

    #[test]
    fn the_direction_orients_a_rank_comparison() {
        use grimvault_core::facets::{AffixEvidence, AscensionTable, BaseEvidence, Facets};
        use grimvault_core::item::Item;
        use grimvault_core::search::{AffixName, Subject};
        let item = Item::default();
        let rank = |name: &str| {
            SortRank::of(&Subject {
                item: &item,
                name,
                prefix: AffixName::Absent,
                suffix: AffixName::Absent,
                base: BaseEvidence::Unresolved,
                facets: Facets::classify(
                    &item,
                    BaseEvidence::Unresolved,
                    AffixEvidence::Absent,
                    AffixEvidence::Absent,
                    &AscensionTable::Absent,
                ),
                details: None,
            })
        };
        let (alpha, bravo) = (rank("Alpha"), rank("Bravo"));
        assert_eq!(
            ordered(Sort::by(SortKey::Name), &alpha, &bravo),
            Ordering::Less
        );
        assert_eq!(
            ordered(
                Sort::by(SortKey::Name).clicked(SortKey::Name),
                &alpha,
                &bravo
            ),
            Ordering::Greater
        );
    }

    #[test]
    fn the_view_state_round_trips_and_defaults() {
        let view = SearchView {
            query: Query {
                name: "x".into(),
                ..Query::default()
            },
            sort: Sort::by(SortKey::Rarity),
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<SearchView>(&json).unwrap(), view);
        assert_eq!(
            serde_json::from_str::<SearchView>("{}").unwrap(),
            SearchView::default()
        );
    }

    #[test]
    fn row_heights_grow_with_the_lines_and_never_shrink_below_the_tile() {
        assert!((row_height(&ItemDetails::default()) - (ICON + ROW_PAD)).abs() < 1e-3);
        let mut details = ItemDetails::default();
        details.blocks.push(Block {
            source: BlockSource::Base,
            record: univault_engine::ids::RecordId::parse("records/a.dbr".into()).unwrap(),
            title: None,
            flavor: None,
            lines: (0..6)
                .map(|_| grimvault_core::stats::StatLine {
                    kind: grimvault_core::stats::LineKind::Conversion,
                    section: grimvault_core::stats::Section::Offense,
                    emphasis: grimvault_core::stats::Emphasis::Bonus,
                    text: "x".into(),
                    values: Vec::new(),
                })
                .collect(),
        });
        assert!((row_height(&details) - 6.0f32.mul_add(LINE_HEIGHT, ROW_PAD)).abs() < 1e-3);
        assert_eq!(category_label(CategoryFilter::Any), "Any type");
        assert_eq!(
            category_label(CategoryFilter::Group {
                group: Group::Armor
            }),
            "All Armor"
        );
    }
}
