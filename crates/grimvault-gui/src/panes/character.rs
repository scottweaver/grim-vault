//! The character section: a picker over `main/*/player.gdc`, and the
//! chosen character's worn gear, sacks, and personal stash. A
//! character whose every block is typed is editable — its sacks and
//! stash tabs are drag sources and drop targets, its worn gear can be
//! taken off into the vault, a sack, or a stash tab (never equipped
//! from here: the game's slot rules are its own), its iron bits can be
//! set, and its attributes or masteries can be reset behind a
//! confirmation — while one with an opaque block is shown read-only,
//! since an edit before that block could never be re-keyed
//! (ARCHITECTURE.md "Data flow").

use egui::{
    Align2, CornerRadius, FontId, Id, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};
use grimvault_core::gamedata::Footprint;
use grimvault_core::gdc::{
    EquipSlot, EquippedItem, Inventory, InventoryContents, PlayerFile, WeaponSet,
};
use grimvault_core::item::Item;
use grimvault_core::respec::Reset;
use grimvault_core::settings::AutoMoveTab;
use grimvault_core::transfer::{SackIndex, TabIndex};
use univault_ui::components::scroll_strip::{self, ScrollStrip, StripInk};
use univault_ui::theme::Theme;

use super::{
    Confirmation, DragFrame, GridEntry, GridSpec, Interaction, PaneCtx, PendingClear,
    READ_ONLY_WHY, bulk_buttons, clear_button, confirm_clear, container_tab, extent, grid_surface,
    item_tooltip, order_toggles, paint_item, sack_entries, stash_entries,
};
use crate::documents::{Backup, CharacterDoc, CharacterEntry, CharacterSlot, Edits, Writable};
use crate::drag::{Container, DragSource, DragState};
use crate::grid::{CELL_PX, cells, footprint_or_unit};
use crate::theme::FITS;
use grimvault_core::gdc::Realm;

/// Which of the character's containers is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharacterTab {
    Sack(usize),
    Equipped,
    Stash(usize),
}

/// An edit awaiting the user's confirmation: a reset of the
/// character, or the emptying of one of its stash tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Reset(Reset),
    Clear(PendingClear),
}

/// The picker's choice, the container tab, and the edit awaiting
/// the user's confirmation, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterView {
    pub selected: usize,
    pub tab: CharacterTab,
    pub confirm: Option<Confirm>,
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

/// The worn slots as the character sheet lays them out: armour across
/// the top with the first weapon set at its end, accessories below
/// with the second — two rows that fit the pane at its default height.
const ARMOR_ROW: [EquipSlot; 6] = [
    EquipSlot::Head,
    EquipSlot::Shoulders,
    EquipSlot::Chest,
    EquipSlot::Hands,
    EquipSlot::Legs,
    EquipSlot::Feet,
];
const ACCESSORY_ROW: [EquipSlot; 6] = [
    EquipSlot::Amulet,
    EquipSlot::Ring1,
    EquipSlot::Ring2,
    EquipSlot::Belt,
    EquipSlot::Medal,
    EquipSlot::Relic,
];

/// A gear slot's box: two cells wide and two and a half tall, the
/// larger worn footprints scaled down to fit; the label sits under it.
const SLOT_BOX: Vec2 = vec2(2.0 * CELL_PX, 2.5 * CELL_PX);
const SLOT_LABEL: f32 = 16.0;

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
    confirm_pending(ui, slot, file.character_name(), view, theme, frame);
    let sacks = file
        .inventory()
        .map_or(&[][..], |inventory| inventory.sacks());
    let stash_tabs = file.stash().map_or(&[][..], |stash| &stash.tabs[..]);
    ScrollStrip::new("character-tabs", StripInk::from_palette(cx.palette)).show(ui, |ui| {
        let worn = file
            .inventory()
            .map_or(0, |inventory| inventory.equipped().count());
        container_or_plain_tab(
            ui,
            view,
            CharacterTab::Equipped,
            format!("Equipped ({worn})"),
            None,
            cx,
            frame,
        );
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
    show_container(ui, slot, doc, view, cx, frame);
}

/// The selected container: a sack or own-stash tab as a grid under a
/// header of bulk buttons, editable when the character is, or the
/// equipment list.
fn show_container(
    ui: &mut Ui,
    slot: CharacterSlot,
    doc: &CharacterDoc,
    view: &mut CharacterView,
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
        .show(ui, |ui| match view.tab {
            CharacterTab::Sack(index) => match (sacks.get(index), sack_index(index)) {
                (Some(sack), Some(sack_index)) => {
                    let container = editable.then_some(Container::Sack {
                        character: slot,
                        sack: sack_index,
                    });
                    let entries = sack_entries(sack, cx);
                    let (cols, rows) = sack_dims(sack_index, &entries);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{cols}×{rows} cells · {} items", sack.items.len()));
                        ui.separator();
                        bulk_buttons(ui, container, sack.items.len(), frame);
                    });
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    let interaction =
                        container.map_or(Interaction::ReadOnly, Interaction::Editable);
                    grid_surface(ui, spec, &entries, interaction, cx, frame);
                }
                (None, _) | (_, None) => {
                    ui.weak("This character has never entered the game, so it has no inventory yet.");
                }
            },
            CharacterTab::Equipped => gear(ui, slot, file, editable, cx, frame),
            CharacterTab::Stash(index) => match (stash_tabs.get(index), tab_index(index)) {
                (Some(stash_tab), Some(tab_index)) => {
                    let container = editable.then_some(Container::CharacterStash {
                        character: slot,
                        tab: tab_index,
                    });
                    ui.horizontal_wrapped(|ui| {
                        if editable {
                            order_toggles(
                                ui,
                                &AutoMoveTab::CharacterStash {
                                    realm: doc.realm(),
                                    name: doc.name().to_owned(),
                                    tab: tab_index,
                                },
                                cx,
                                frame,
                            );
                            ui.separator();
                        }
                        bulk_buttons(ui, container, stash_tab.items.len(), frame);
                        if let Some(pending) = clear_button(ui, container, stash_tab.items.len()) {
                            view.confirm = Some(Confirm::Clear(pending));
                        }
                    });
                    let entries = stash_entries(stash_tab, cx);
                    let cols = i32::try_from(stash_tab.width).unwrap_or(0);
                    let rows = i32::try_from(stash_tab.height).unwrap_or(0);
                    let spec = GridSpec {
                        available,
                        cols,
                        rows,
                    };
                    let interaction =
                        container.map_or(Interaction::ReadOnly, Interaction::Editable);
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
            view.confirm = Some(Confirm::Reset(reset));
        }
    }
}

/// The confirmation a reset or a clear needs before it is reported:
/// nothing is edited until the user confirms, and Esc, a click
/// outside, or Cancel drops the request.
fn confirm_pending(
    ui: &Ui,
    slot: CharacterSlot,
    name: &str,
    view: &mut CharacterView,
    theme: &Theme,
    frame: &mut DragFrame,
) {
    match view.confirm {
        None => {}
        Some(Confirm::Reset(reset)) => confirm_reset(ui, slot, name, reset, view, theme, frame),
        Some(Confirm::Clear(pending)) => {
            let label = container_name(pending.container, name);
            if confirm_clear(ui, pending, &label, theme, frame) == Confirmation::Settled {
                view.confirm = None;
            }
        }
    }
}

/// A container as the confirmation names it.
fn container_name(container: Container, name: &str) -> String {
    match container {
        Container::TransferStash(tab) => format!("transfer stash tab {}", tab.value() + 1),
        Container::Sack { sack, .. } => format!("{name}'s sack {}", sack.value() + 1),
        Container::CharacterStash { tab, .. } => format!("{name}'s stash {}", tab.value() + 1),
    }
}

fn confirm_reset(
    ui: &Ui,
    slot: CharacterSlot,
    name: &str,
    reset: Reset,
    view: &mut CharacterView,
    theme: &Theme,
    frame: &mut DragFrame,
) {
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

/// The worn gear as slot boxes: armour and the first weapon set on one
/// row, accessories and the second on the next, the set in hand
/// marked. Each occupied slot is a tile with the item's tooltip and, on
/// an editable character, a drag source for taking the item off; no
/// slot takes a drop.
fn gear(
    ui: &mut Ui,
    character: CharacterSlot,
    file: &PlayerFile,
    editable: bool,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let Some(contents) = file.inventory().and_then(Inventory::contents) else {
        ui.weak("This character has never entered the game, so it wears nothing yet.");
        return;
    };
    if editable {
        ui.weak(
            "Drag or right-click worn gear to take it off into the vault, a sack, or a stash tab; \
             equip in-game.",
        );
    } else {
        ui.weak("Worn gear is shown only: this character is read-only.");
    }
    for (row, set) in [
        (ARMOR_ROW, WeaponSet::First),
        (ACCESSORY_ROW, WeaponSet::Second),
    ] {
        ui.horizontal_top(|ui| {
            for slot in row {
                slot_box(
                    ui,
                    character,
                    slot,
                    contents.slot(slot),
                    editable,
                    cx,
                    frame,
                );
            }
            ui.add_space(SLOT_BOX.x / 2.0);
            weapon_set(ui, character, set, contents, editable, cx, frame);
        });
    }
}

/// A weapon set's two hands with the set named under them.
fn weapon_set(
    ui: &mut Ui,
    character: CharacterSlot,
    set: WeaponSet,
    contents: &InventoryContents,
    editable: bool,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            for slot in EquipSlot::hands(set) {
                slot_box(
                    ui,
                    character,
                    slot,
                    contents.slot(slot),
                    editable,
                    cx,
                    frame,
                );
            }
        });
        let caption = if contents.active_weapon_set() == set {
            RichText::new(format!("Weapon set {} · in hand", set.number()))
                .small()
                .color(cx.palette.heading)
        } else {
            RichText::new(format!("Weapon set {}", set.number()))
                .small()
                .color(cx.palette.text_weak)
        };
        ui.label(caption);
    });
}

/// One slot: its box and label, and — when something is worn there —
/// the item's tile scaled to fit, its tooltip, and the gestures of a
/// grid item when the character is editable.
fn slot_box(
    ui: &mut Ui,
    character: CharacterSlot,
    slot: EquipSlot,
    worn: &EquippedItem,
    editable: bool,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let source =
        (editable && !worn.item.is_empty()).then_some(DragSource::Equipped { character, slot });
    let sense = match source {
        Some(_) => Sense::click_and_drag(),
        None => Sense::hover(),
    };
    let (rect, response) = ui.allocate_exact_size(SLOT_BOX + vec2(0.0, SLOT_LABEL), sense);
    if !ui.is_rect_visible(rect) {
        return;
    }
    let box_rect = Rect::from_min_size(rect.min, SLOT_BOX);
    let painter = ui.painter_at(rect);
    painter.rect_filled(box_rect, CornerRadius::same(2), cx.palette.grid_bg);
    painter.rect_stroke(
        box_rect,
        CornerRadius::same(2),
        Stroke::new(0.5, cx.palette.grid_line),
        StrokeKind::Inside,
    );
    painter.text(
        pos2(box_rect.center().x, box_rect.max.y + 2.0),
        Align2::CENTER_TOP,
        slot.label(),
        FontId::proportional(11.0),
        cx.palette.text_weak,
    );
    if worn.item.is_empty() {
        return;
    }
    let (footprint, footprint_source) =
        footprint_or_unit(cx.facts.base(cx.game, &worn.item).footprint);
    let cell = (SLOT_BOX.x / cells(footprint.width.max(1)))
        .min(SLOT_BOX.y / cells(footprint.height.max(1)))
        .min(CELL_PX);
    let size = vec2(
        cells(footprint.width.max(1)),
        cells(footprint.height.max(1)),
    ) * cell;
    let tile = Rect::from_center_size(box_rect.center(), size).shrink(1.0);
    let dragging = cx.drag.is_some();
    let lifted = source.is_some_and(|source| cx.drag.is_some_and(|drag| drag.source == source));
    paint_item(
        ui.ctx(),
        &painter,
        tile,
        &worn.item,
        footprint_source,
        cell,
        response.hovered() && !dragging,
        lifted,
        cx,
    );
    if !dragging {
        egui::Tooltip::for_enabled(&response)
            .at_pointer()
            .show(|ui| item_tooltip(ui, cx, &worn.item));
    }
    if let Some(source) = source {
        report_slot_gestures(
            ui, &response, source, &worn.item, footprint, tile, cx, frame,
        );
    }
}

/// The gestures a grid item has, on a worn one: a drag lifts it, a
/// double-click or right-click asks for the quick move, a click
/// selects it for the inspector.
#[expect(
    clippy::too_many_arguments,
    reason = "one call surface inside the slot renderer; the arguments are the frame's borrows"
)]
fn report_slot_gestures(
    ui: &Ui,
    response: &Response,
    source: DragSource,
    item: &Item,
    footprint: Footprint,
    tile: Rect,
    cx: &PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    if cx.drag.is_some() {
        return;
    }
    if response.drag_started()
        && frame.begin.is_none()
        && let Some(origin) = ui.input(|input| input.pointer.press_origin())
    {
        frame.begin = Some(DragState {
            source,
            item: item.clone(),
            footprint,
            grab: origin - tile.min,
        });
    }
    if response.double_clicked() {
        frame.double_click = Some(source);
    }
    if response.secondary_clicked() {
        frame.right_click = Some(source);
    }
    if response.clicked() {
        frame.select = Some(source);
    }
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
    fn the_confirmation_names_the_container_after_the_character() {
        let zark = CharacterSlot::new(1);
        assert_eq!(
            container_name(
                Container::CharacterStash {
                    character: zark,
                    tab: TabIndex::new(1)
                },
                "Zark"
            ),
            "Zark's stash 2"
        );
        assert_eq!(
            container_name(
                Container::Sack {
                    character: zark,
                    sack: SackIndex::MAIN
                },
                "Zark"
            ),
            "Zark's sack 1"
        );
        assert_eq!(
            container_name(Container::TransferStash(TabIndex::new(0)), "Zark"),
            "transfer stash tab 1"
        );
    }

    #[test]
    fn picking_another_character_drops_a_pending_confirmation() {
        let mut view = CharacterView::opening_on(Some(CharacterSlot::new(2)));
        assert_eq!(view.selected, 2);
        view.confirm = Some(Confirm::Clear(PendingClear {
            container: Container::CharacterStash {
                character: CharacterSlot::new(2),
                tab: TabIndex::new(0),
            },
            count: 3,
        }));
        let fresh = CharacterView::opening_on(Some(CharacterSlot::new(0)));
        assert_eq!(fresh.confirm, None);
        assert_ne!(view.confirm, fresh.confirm);
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
