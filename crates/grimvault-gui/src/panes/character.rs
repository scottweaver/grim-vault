//! The character section: a picker over `main/*/player.gdc`, and the
//! chosen character's sacks, equipped items, and personal stash. A
//! character whose every block is typed is editable — its sacks and
//! stash tabs are drag sources and drop targets, its iron bits can be
//! set, and its attributes or masteries can be reset behind a
//! confirmation — while one with an opaque block is shown read-only,
//! since an edit before that block could never be re-keyed
//! (ARCHITECTURE.md "Data flow"). Equipped items are shown but never
//! moved.

use egui::{Id, RichText, Ui, Vec2};
use grimvault_core::gdc::{EquippedItem, InventoryState, PlayerFile};
use grimvault_core::respec::Reset;
use grimvault_core::settings::AutoMoveTab;
use grimvault_core::transfer::{SackIndex, TabIndex};
use univault_ui::components::scroll_strip::{self, ScrollStrip, StripInk};
use univault_ui::theme::Theme;

use super::{
    DragFrame, GridEntry, GridSpec, Interaction, PaneCtx, auto_move_toggle, container_tab, extent,
    grid_surface, item_tooltip, sack_entries, stash_entries,
};
use crate::documents::{Backup, CharacterDoc, CharacterEntry, CharacterSlot, Edits, Writable};
use crate::drag::Container;
use crate::theme::{FITS, UNKNOWN_RARITY, rarity_color};
use grimvault_core::gdc::Realm;

/// Which of the character's containers is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharacterTab {
    Sack(usize),
    Equipped,
    Stash(usize),
}

/// The picker's choice, the container tab, and the reset awaiting
/// the user's confirmation, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterView {
    pub selected: usize,
    pub tab: CharacterTab,
    pub confirm: Option<Reset>,
}

impl Default for CharacterView {
    fn default() -> Self {
        Self::opening_on(None)
    }
}

impl CharacterView {
    /// A fresh view with the picker on `slot` — the character the game
    /// wrote last — or on the first entry when there is none.
    #[must_use]
    pub fn opening_on(slot: Option<CharacterSlot>) -> Self {
        Self {
            selected: slot.map_or(0, CharacterSlot::value),
            tab: CharacterTab::Sack(0),
            confirm: None,
        }
    }

    /// What the strip's selected tab is, for revealing it when it
    /// changes: the tab, under the character showing it — picking
    /// another character is a new selection even on the same tab.
    fn selection(self) -> (usize, CharacterTab) {
        (self.selected, self.tab)
    }
}

/// Equipment slots in file order — GD Stash's order, checked against
/// the classes of the items the user's characters wear.
pub const EQUIPMENT_SLOTS: [&str; 12] = [
    "Head",
    "Amulet",
    "Chest",
    "Legs",
    "Feet",
    "Hands",
    "Ring 1",
    "Ring 2",
    "Belt",
    "Shoulders",
    "Medal",
    "Relic",
];
/// The two slots of each weapon set.
pub const WEAPON_SLOTS: [&str; 2] = ["Main hand", "Off hand"];

/// The rendered size of a sack: the game's fixed size for that bag,
/// grown to its contents rather than clipped.
#[must_use]
pub fn sack_dims(sack: SackIndex, entries: &[GridEntry<'_>]) -> (i32, i32) {
    let dims = sack.dimensions();
    let cols = i32::try_from(dims.width).unwrap_or(0);
    let rows = i32::try_from(dims.height).unwrap_or(0);
    let (used_cols, used_rows) = extent(entries);
    (cols.max(used_cols), rows.max(used_rows))
}

const EDITABLE_WHY: &str = "Every block of this player.gdc is typed, so edits re-key correctly. Moves and the iron \
     bits are written by autosave, backup-first, and verified by re-reading. Close the game before \
     editing a character: the game overwrites player.gdc when it saves.";

pub fn show(
    ui: &mut Ui,
    characters: &[CharacterEntry],
    view: &mut CharacterView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    ui.horizontal(|ui| {
        ui.label(theme.heading("Characters"));
        let selected_label = characters
            .get(view.selected)
            .map_or_else(|| "none".to_string(), picker_label);
        let before = view.selected;
        egui::ComboBox::from_id_salt("character-picker")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (slot, entry) in characters.iter().enumerate() {
                    ui.selectable_value(&mut view.selected, slot, picker_label(entry));
                }
            });
        if view.selected != before {
            view.tab = CharacterTab::Sack(0);
            view.confirm = None;
            if let Some(CharacterEntry::Loaded(doc)) = characters.get(view.selected)
                && doc.writable() == Writable::Yes
            {
                frame.touched = Some(Container::Sack {
                    character: CharacterSlot::new(view.selected),
                    sack: SackIndex::MAIN,
                });
            }
        }
        if let Some(CharacterEntry::Loaded(doc)) = characters.get(view.selected) {
            access_badge(ui, doc, cx);
        }
    });
    match characters.get(view.selected) {
        None => {
            ui.weak("No characters under main/ or user/.");
        }
        Some(CharacterEntry::Failed { path, error, .. }) => {
            ui.colored_label(cx.palette.error, format!("{}: {error}", path.display()));
        }
        Some(CharacterEntry::Loaded(doc)) => body(
            ui,
            CharacterSlot::new(view.selected),
            doc,
            view,
            theme,
            cx,
            frame,
        ),
    }
}

/// The picker's entry: the name, the realm when it is not the main
/// campaign (the same name can exist in both), and whether the file
/// could be read.
fn picker_label(entry: &CharacterEntry) -> String {
    let realm = match entry.realm() {
        Realm::Main => "",
        Realm::Custom => " · custom game",
    };
    let state = match entry {
        CharacterEntry::Loaded(_) => "",
        CharacterEntry::Failed { .. } => " (unreadable)",
    };
    format!("{}{realm}{state}", entry.label())
}

fn access_badge(ui: &mut Ui, doc: &CharacterDoc, cx: &PaneCtx<'_>) {
    match doc.writable() {
        Writable::Yes => {
            let state = match (doc.tracking().edits(), doc.tracking().backup()) {
                (Edits::Unsaved, _) => "editable · unsaved",
                (Edits::Saved, Backup::Taken) => "editable · backed up",
                (Edits::Saved, Backup::Armed) => "editable",
            };
            ui.label(RichText::new(state).small().color(FITS))
                .on_hover_text(EDITABLE_WHY);
        }
        Writable::OpaqueBlock(block) => {
            ui.label(
                RichText::new(format!("read-only: block {block} is not typed"))
                    .small()
                    .color(cx.palette.warn),
            )
            .on_hover_text(
                "A save block the parser cannot type cannot be re-keyed, so nothing before it can \
                 be edited; this character is shown but never written.",
            );
        }
    }
}

fn body(
    ui: &mut Ui,
    slot: CharacterSlot,
    doc: &CharacterDoc,
    view: &mut CharacterView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let file = doc.file();
    let editable = doc.writable() == Writable::Yes;
    ui.horizontal_wrapped(|ui| {
        ui.label(header_line(file, cx));
        money_field(ui, slot, file, editable, frame);
        respec_buttons(ui, editable, view);
        ui.label(theme.path_text(doc.path().display().to_string()));
    });
    confirm_reset(ui, slot, file.character_name(), view, theme, frame);
    let sacks = file
        .inventory()
        .map_or(&[][..], |inventory| inventory.sacks());
    let stash_tabs = file.stash().map_or(&[][..], |stash| &stash.tabs[..]);
    ScrollStrip::new("character-tabs", StripInk::from_palette(cx.palette)).show(ui, |ui| {
        for (index, sack) in sacks.iter().enumerate() {
            let label = format!("Sack {} ({})", index + 1, sack.items.len());
            let container =
                editable
                    .then(|| sack_index(index))
                    .flatten()
                    .map(|sack| Container::Sack {
                        character: slot,
                        sack,
                    });
            container_or_plain_tab(
                ui,
                view,
                CharacterTab::Sack(index),
                label,
                container,
                cx,
                frame,
            );
        }
        let worn = file
            .inventory()
            .map_or(0, |inventory| inventory.equipped().count());
        let equipped = view.tab == CharacterTab::Equipped;
        let response = ui.selectable_label(equipped, format!("Equipped ({worn})"));
        if equipped {
            scroll_strip::reveal_selected(ui, view.selection(), &response);
        }
        if response.clicked() {
            view.tab = CharacterTab::Equipped;
        }
        for (index, stash_tab) in stash_tabs.iter().enumerate() {
            let label = format!("Stash {} ({})", index + 1, stash_tab.items.len());
            let container =
                editable
                    .then(|| tab_index(index))
                    .flatten()
                    .map(|tab| Container::CharacterStash {
                        character: slot,
                        tab,
                    });
            container_or_plain_tab(
                ui,
                view,
                CharacterTab::Stash(index),
                label,
                container,
                cx,
                frame,
            );
        }
    });
    show_container(ui, slot, doc, view.tab, cx, frame);
}

/// The selected container: a sack or own-stash tab as a grid, editable
/// when the character is, or the equipment list.
fn show_container(
    ui: &mut Ui,
    slot: CharacterSlot,
    doc: &CharacterDoc,
    tab: CharacterTab,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let file = doc.file();
    let editable = doc.writable() == Writable::Yes;
    let sacks = file
        .inventory()
        .map_or(&[][..], |inventory| inventory.sacks());
    let stash_tabs = file.stash().map_or(&[][..], |stash| &stash.tabs[..]);
    let available: Vec2 = ui.available_size();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| match tab {
            CharacterTab::Sack(index) => match (sacks.get(index), sack_index(index)) {
                (Some(sack), Some(sack_index)) => {
                    let entries = sack_entries(sack, cx);
                    let (cols, rows) = sack_dims(sack_index, &entries);
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    let interaction = if editable {
                        Interaction::Editable(Container::Sack {
                            character: slot,
                            sack: sack_index,
                        })
                    } else {
                        Interaction::ReadOnly
                    };
                    grid_surface(ui, spec, &entries, interaction, cx, frame);
                }
                (None, _) | (_, None) => {
                    ui.weak("This character has never entered the game, so it has no inventory yet.");
                }
            },
            CharacterTab::Equipped => equipped(ui, file, cx),
            CharacterTab::Stash(index) => match (stash_tabs.get(index), tab_index(index)) {
                (Some(stash_tab), Some(tab_index)) => {
                    if editable {
                        auto_move_toggle(
                            ui,
                            AutoMoveTab::CharacterStash {
                                realm: doc.realm(),
                                name: doc.name().to_owned(),
                                tab: tab_index,
                            },
                            cx,
                            frame,
                        );
                    }
                    let entries = stash_entries(stash_tab, cx);
                    let cols = i32::try_from(stash_tab.width).unwrap_or(0);
                    let rows = i32::try_from(stash_tab.height).unwrap_or(0);
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    let interaction = if editable {
                        Interaction::Editable(Container::CharacterStash {
                            character: slot,
                            tab: tab_index,
                        })
                    } else {
                        Interaction::ReadOnly
                    };
                    grid_surface(ui, spec, &entries, interaction, cx, frame);
                }
                (None, _) | (_, None) => {
                    ui.weak("No such stash tab.");
                }
            },
        });
}

/// A container's tab button — a drop target when the container is
/// editable, a plain selector otherwise.
fn container_or_plain_tab(
    ui: &mut Ui,
    view: &mut CharacterView,
    tab: CharacterTab,
    label: String,
    container: Option<Container>,
    cx: &PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let selected = view.tab == tab;
    let response = match container {
        Some(container) => container_tab(ui, selected, label, container, cx, frame),
        None => ui.selectable_label(selected, label),
    };
    if selected {
        scroll_strip::reveal_selected(ui, view.selection(), &response);
    }
    if response.clicked() {
        view.tab = tab;
    }
}

fn sack_index(index: usize) -> Option<SackIndex> {
    u32::try_from(index).ok().map(SackIndex::new)
}

fn tab_index(index: usize) -> Option<TabIndex> {
    u32::try_from(index).ok().map(TabIndex::new)
}

/// The iron bits: a number the user can drag or type into for an
/// editable character, reported through the frame rather than written
/// here.
fn money_field(
    ui: &mut Ui,
    slot: CharacterSlot,
    file: &PlayerFile,
    editable: bool,
    frame: &mut DragFrame,
) {
    let Some(info) = file.character_info() else {
        return;
    };
    ui.separator();
    ui.label("iron bits:");
    if !editable {
        ui.label(info.money.to_string());
        return;
    }
    let mut money = info.money;
    let response = ui
        .add(
            egui::DragValue::new(&mut money)
                .range(0..=u32::MAX)
                .speed(250.0),
        )
        .on_hover_text("Drag, or click and type, to set the character's iron bits.");
    if response.changed() {
        frame.set_money = Some((slot, money));
    }
}

const READ_ONLY_WHY: &str = "This character is read-only: a block of its player.gdc is not typed, so nothing before it \
     can be edited.";

/// What each reset does, for its button and its confirmation.
fn reset_explanation(reset: Reset) -> &'static str {
    match reset {
        Reset::Attributes => {
            "Returns every spent attribute point to the pool and puts physique, cunning, and \
             spirit — with the health and energy they bought — back to their base values."
        }
        Reset::Masteries => {
            "Removes both masteries and every skill in them, returning the points, so both \
             masteries can be chosen again in-game. Devotions and item skills stay."
        }
    }
}

/// The two resets as buttons; a click asks for confirmation rather
/// than acting, and a read-only character explains itself on hover.
fn respec_buttons(ui: &mut Ui, editable: bool, view: &mut CharacterView) {
    ui.separator();
    for (reset, label) in [
        (Reset::Attributes, "Reset attributes"),
        (Reset::Masteries, "Reset masteries"),
    ] {
        if ui
            .add_enabled(editable, egui::Button::new(label))
            .on_hover_text(reset_explanation(reset))
            .on_disabled_hover_text(READ_ONLY_WHY)
            .clicked()
        {
            view.confirm = Some(reset);
        }
    }
}

/// The confirmation a reset needs before it is reported: nothing is
/// edited until the user confirms, and Esc, a click outside, or
/// Cancel drops the request.
fn confirm_reset(
    ui: &Ui,
    slot: CharacterSlot,
    name: &str,
    view: &mut CharacterView,
    theme: &Theme,
    frame: &mut DragFrame,
) {
    let Some(reset) = view.confirm else {
        return;
    };
    let mut confirmed = None;
    let response = egui::Modal::new(Id::new("respec-confirm")).show(ui.ctx(), |ui| {
        ui.set_max_width(440.0);
        ui.label(theme.heading(format!("Reset {name}'s {reset}?")));
        ui.label(reset_explanation(reset));
        ui.label(
            "The change is written by autosave, backup-first, like every other edit. Close the \
             game first; the game overwrites player.gdc when it saves.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(format!("Reset {reset}")).clicked() {
                confirmed = Some(true);
            }
            if ui.button("Cancel").clicked() {
                confirmed = Some(false);
            }
        });
    });
    if confirmed.is_none() && response.should_close() {
        confirmed = Some(false);
    }
    match confirmed {
        Some(true) => {
            frame.respec = Some((slot, reset));
            view.confirm = None;
        }
        Some(false) => view.confirm = None,
        None => {}
    }
}

fn header_line(file: &PlayerFile, cx: &PaneCtx<'_>) -> String {
    let header = file.header();
    let class = cx
        .game
        .tag_text(&header.class_tag)
        .map_or_else(|| header.class_tag.clone(), str::to_string);
    let hardcore = if header.hardcore { " · hardcore" } else { "" };
    let class = if class.is_empty() {
        String::new()
    } else {
        format!(" · {class}")
    };
    format!("{} · level {}{class}{hardcore}", header.name, header.level)
}

fn equipped(ui: &mut Ui, file: &PlayerFile, cx: &mut PaneCtx<'_>) {
    let Some(inventory) = file.inventory() else {
        ui.weak("No inventory block.");
        return;
    };
    let InventoryState::Entered(contents) = &inventory.state else {
        ui.weak("This character has never entered the game.");
        return;
    };
    ui.weak("Equipped items are shown only; unequip in-game to move them.");
    egui::Grid::new("equipped")
        .num_columns(3)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            for (label, slot) in EQUIPMENT_SLOTS.iter().zip(&contents.equipment) {
                equipped_row(ui, label, slot, cx);
            }
            for (set, slots) in [(1, &contents.weapon_set_1), (2, &contents.weapon_set_2)] {
                for (label, slot) in WEAPON_SLOTS.iter().zip(slots) {
                    equipped_row(ui, &format!("Weapon set {set}: {label}"), slot, cx);
                }
            }
        });
}

fn equipped_row(ui: &mut Ui, label: &str, slot: &EquippedItem, cx: &mut PaneCtx<'_>) {
    ui.label(RichText::new(label).color(cx.palette.text_weak));
    if slot.item.is_empty() {
        ui.weak("—");
        ui.label("");
    } else {
        let facts = cx.facts.facts(cx.game, &slot.item);
        let colour = facts.base.rarity.map_or(UNKNOWN_RARITY, rarity_color);
        let name = facts.display_name();
        let class = facts
            .base
            .class
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        ui.label(RichText::new(name).color(colour))
            .on_hover_ui(|ui| item_tooltip(ui, cx, &slot.item));
        ui.label(RichText::new(class).small().color(cx.palette.text_weak));
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use grimvault_core::item::Item;
    use grimvault_core::transfer::{EXTRA_SACK, ItemIndex, MAIN_SACK};
    use univault_engine::grid::CellRect;

    use super::*;
    use crate::grid::FootprintSource;

    fn dims(sack: grimvault_core::transfer::SackDimensions) -> (i32, i32) {
        (
            i32::try_from(sack.width).unwrap(),
            i32::try_from(sack.height).unwrap(),
        )
    }

    #[test]
    fn sacks_keep_their_fixed_size_unless_the_items_reach_past_it() {
        let item = Item::default();
        let inside = [GridEntry {
            item: &item,
            cells: CellRect {
                x: 11,
                y: 7,
                width: 1,
                height: 1,
            },
            footprint: FootprintSource::Known,
            index: ItemIndex::new(0),
        }];
        let extra = SackIndex::new(1);
        assert_eq!(sack_dims(SackIndex::MAIN, &inside), dims(MAIN_SACK));
        assert_eq!(sack_dims(extra, &inside), (12, 8));
        assert_eq!(sack_dims(extra, &[]), dims(EXTRA_SACK));
        let past = [GridEntry {
            item: &item,
            cells: CellRect {
                x: 11,
                y: 7,
                width: 2,
                height: 3,
            },
            footprint: FootprintSource::Known,
            index: ItemIndex::new(0),
        }];
        assert_eq!(sack_dims(SackIndex::MAIN, &past), (13, 10));
    }
}
