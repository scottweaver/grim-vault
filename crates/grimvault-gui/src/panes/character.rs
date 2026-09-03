//! The character section: a picker over `main/*/player.gdc`, and the
//! chosen character's sacks, equipped items, and personal stash as
//! read-only surfaces: this build never writes `player.gdc` — the
//! vault loop edits only `transfer.gst`, and a block the parser cannot
//! type cannot be re-keyed (ARCHITECTURE.md "Data flow") — so nothing
//! here is a drag source or target.

use egui::{RichText, Ui, Vec2};
use grimvault_core::gdc::{EquippedItem, InventoryState, PlayerFile};
use univault_ui::theme::Theme;

use super::{
    DragFrame, GridEntry, GridSpec, Interaction, PaneCtx, extent, grid_surface, item_tooltip,
    sack_entries, stash_entries,
};
use crate::documents::{CharacterDoc, CharacterEntry};
use crate::theme::{UNKNOWN_RARITY, rarity_color};

/// Which of the character's containers is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterTab {
    Sack(usize),
    Equipped,
    Stash(usize),
}

/// The picker's choice and the container tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterView {
    pub selected: usize,
    pub tab: CharacterTab,
}

impl Default for CharacterView {
    fn default() -> Self {
        Self {
            selected: 0,
            tab: CharacterTab::Sack(0),
        }
    }
}

/// The game's inventory bag sizes: the main bag and each purchased
/// bag. The file carries neither, and a sack whose items reach past
/// these edges is grown to fit rather than clipped.
pub const MAIN_SACK: (i32, i32) = (12, 8);
pub const EXTRA_SACK: (i32, i32) = (8, 8);

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

/// The rendered size of a sack: its fixed size, grown to its
/// contents.
#[must_use]
pub fn sack_dims(index: usize, entries: &[GridEntry<'_>]) -> (i32, i32) {
    let (cols, rows) = if index == 0 { MAIN_SACK } else { EXTRA_SACK };
    let (used_cols, used_rows) = extent(entries);
    (cols.max(used_cols), rows.max(used_rows))
}

const READ_ONLY_WHY: &str = "Characters are read-only: this build never writes player.gdc. The vault loop edits \
     only transfer.gst, and a save block the parser cannot type makes a re-encode of that file unsafe, \
     so nothing here is a drag source or a drop target.";

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
            .map_or_else(|| "none".to_string(), CharacterEntry::label);
        let before = view.selected;
        egui::ComboBox::from_id_salt("character-picker")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                for (slot, entry) in characters.iter().enumerate() {
                    let label = match entry {
                        CharacterEntry::Loaded(_) => entry.label(),
                        CharacterEntry::Failed { .. } => format!("{} (unreadable)", entry.label()),
                    };
                    ui.selectable_value(&mut view.selected, slot, label);
                }
            });
        if view.selected != before {
            view.tab = CharacterTab::Sack(0);
        }
        ui.label(RichText::new("read-only").small().color(cx.palette.warn))
            .on_hover_text(READ_ONLY_WHY);
    });
    match characters.get(view.selected) {
        None => {
            ui.weak("No characters under main/.");
        }
        Some(CharacterEntry::Failed { path, error }) => {
            ui.colored_label(cx.palette.error, format!("{}: {error}", path.display()));
        }
        Some(CharacterEntry::Loaded(doc)) => body(ui, doc, view, theme, cx, frame),
    }
}

fn body(
    ui: &mut Ui,
    doc: &CharacterDoc,
    view: &mut CharacterView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let file = doc.file();
    ui.horizontal_wrapped(|ui| {
        ui.label(header_line(file, cx));
        ui.label(theme.path_text(doc.path().display().to_string()));
    });
    let sacks = file
        .inventory()
        .map_or(&[][..], |inventory| inventory.sacks());
    let stash_tabs = file.stash().map_or(&[][..], |stash| &stash.tabs[..]);
    ui.horizontal_wrapped(|ui| {
        for (slot, sack) in sacks.iter().enumerate() {
            let label = format!("Sack {} ({})", slot + 1, sack.items.len());
            ui.selectable_value(&mut view.tab, CharacterTab::Sack(slot), label);
        }
        let worn = file
            .inventory()
            .map_or(0, |inventory| inventory.equipped().count());
        ui.selectable_value(
            &mut view.tab,
            CharacterTab::Equipped,
            format!("Equipped ({worn})"),
        );
        for (slot, tab) in stash_tabs.iter().enumerate() {
            let label = format!("Stash {} ({})", slot + 1, tab.items.len());
            ui.selectable_value(&mut view.tab, CharacterTab::Stash(slot), label);
        }
    });
    let available: Vec2 = ui.available_size();
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| match view.tab {
            CharacterTab::Sack(slot) => match sacks.get(slot) {
                Some(sack) => {
                    let entries = sack_entries(sack, cx);
                    let (cols, rows) = sack_dims(slot, &entries);
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    grid_surface(ui, spec, &entries, Interaction::ReadOnly, cx, frame);
                }
                None => {
                    ui.weak("This character has never entered the game, so it has no inventory yet.");
                }
            },
            CharacterTab::Equipped => equipped(ui, file, cx),
            CharacterTab::Stash(slot) => match stash_tabs.get(slot) {
                Some(tab) => {
                    let entries = stash_entries(tab, cx);
                    let cols = i32::try_from(tab.width).unwrap_or(0);
                    let rows = i32::try_from(tab.height).unwrap_or(0);
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    grid_surface(ui, spec, &entries, Interaction::ReadOnly, cx, frame);
                }
                None => {
                    ui.weak("No such stash tab.");
                }
            },
        });
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
    use grimvault_core::transfer::ItemIndex;
    use univault_engine::grid::CellRect;

    use super::*;
    use crate::grid::FootprintSource;

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
        assert_eq!(sack_dims(0, &inside), MAIN_SACK);
        assert_eq!(sack_dims(1, &inside), (12, 8));
        assert_eq!(sack_dims(1, &[]), EXTRA_SACK);
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
        assert_eq!(sack_dims(0, &past), (13, 10));
    }
}
