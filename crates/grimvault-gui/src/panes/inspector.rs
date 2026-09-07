//! The inspector: the last clicked item in full — where it sits, its
//! tooltip body, and its two sockets, each showing the part it holds
//! with a button to free it into the vault, or a picker over the
//! compatible parts the vault store and the component storage hold.
//! It reports what the user asked for through [`DragFrame`]; nothing
//! is edited here.

use std::collections::BTreeMap;

use egui::{Context, Id, RichText, ScrollArea, Ui};
use grimvault_core::gst::ReagentStorage;
use grimvault_core::item::Item;
use grimvault_core::socket::{Slot, Socket};
use grimvault_core::store::VaultStore;
use grimvault_core::transfer::ReagentIndex;
use univault_ui::theme::Theme;

use super::{DragFrame, PaneCtx, item_tooltip, part_name};
use crate::drag::DragSource;
use crate::sockets::{PartSource, Request};
use crate::theme::{UNKNOWN_RARITY, rarity_color};

/// The item the inspector is bound to, as it was when clicked; the
/// binding lapses once the item at `source` is no longer this one.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    pub source: DragSource,
    pub item: Item,
}

/// Where the inspector may offer parts from.
#[derive(Clone, Copy)]
pub struct Supplies<'a> {
    pub store: &'a VaultStore,
    pub reagents: Option<&'a ReagentStorage>,
}

/// Draws the window; `false` once the user closed it.
pub fn show(
    ctx: &Context,
    selected: &Selected,
    place: &str,
    supplies: Supplies<'_>,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> bool {
    let mut open = true;
    egui::Window::new("Item")
        .id(Id::new("inspector"))
        .open(&mut open)
        .default_width(380.0)
        .resizable(true)
        .show(ctx, |ui| {
            body(ui, selected, place, supplies, theme, cx, frame);
        });
    open
}

fn body(
    ui: &mut Ui,
    selected: &Selected,
    place: &str,
    supplies: Supplies<'_>,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let item = &selected.item;
    let (name, colour, class) = {
        let facts = cx.facts.facts(cx.game, item);
        (
            facts.display_name(),
            facts.base.rarity.map_or(UNKNOWN_RARITY, rarity_color),
            facts.base.class.clone(),
        )
    };
    ui.label(RichText::new(name).color(colour).strong());
    ui.label(RichText::new(place).small().color(cx.palette.text_weak));
    ScrollArea::vertical()
        .id_salt("inspector-body")
        .max_height(300.0)
        .show(ui, |ui| item_tooltip(ui, cx, item));
    ui.separator();
    ui.label(theme.section("Sockets"));
    match class.as_ref().and_then(Slot::of_class) {
        None => {
            ui.weak("This item has no sockets: only equipment carries a component and an augment.");
        }
        Some(slot) => {
            for socket in Socket::ALL {
                socket_row(ui, selected, socket, slot, supplies, cx, frame);
            }
        }
    }
}

fn socket_row(
    ui: &mut Ui,
    selected: &Selected,
    socket: Socket,
    slot: Slot,
    supplies: Supplies<'_>,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let record = socket.record_of(&selected.item);
    ui.horizontal(|ui| {
        ui.label(format!("{}:", socket.title()));
        if record.is_empty() {
            ui.weak("none");
            picker(ui, selected.source, socket, slot, supplies, cx, frame);
        } else {
            let name = part_name(cx, record);
            ui.label(RichText::new(name).color(cx.palette.heading));
            if ui
                .button("Remove to vault")
                .on_hover_text(format!(
                    "Frees the {socket} into the vault store as an item of its own; the item \
                     keeps everything else."
                ))
                .clicked()
            {
                frame.socket = Some(Request::Detach {
                    host: selected.source,
                    socket,
                });
            }
        }
    });
}

/// One compatible part the vault or the storage can supply: its
/// record's name and how many are held under it.
struct Candidate {
    source: PartSource,
    name: String,
    count: u32,
}

fn picker(
    ui: &mut Ui,
    host: DragSource,
    socket: Socket,
    slot: Slot,
    supplies: Supplies<'_>,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let candidates = candidates(socket, slot, supplies, cx);
    if candidates.is_empty() {
        ui.weak(format!("no {socket} for a {slot} in the vault"));
        return;
    }
    egui::ComboBox::from_id_salt(("socket-picker", socket.label()))
        .selected_text("Add…")
        .show_ui(ui, |ui| {
            for candidate in &candidates {
                let held_in = match candidate.source {
                    PartSource::Store(_) => "vault",
                    PartSource::Reagent(_) => "storage",
                };
                let label = format!("{} ×{} · {held_in}", candidate.name, candidate.count);
                if ui.selectable_label(false, label).clicked() {
                    frame.socket = Some(Request::Attach {
                        host,
                        part: candidate.source,
                    });
                }
            }
        });
}

/// Every part record the store holds that fits `slot`, one candidate
/// per record with the stacks summed, then the storage's rows for a
/// component; by name.
fn candidates(
    socket: Socket,
    slot: Slot,
    supplies: Supplies<'_>,
    cx: &mut PaneCtx<'_>,
) -> Vec<Candidate> {
    let mut by_record: BTreeMap<String, Candidate> = BTreeMap::new();
    for stored in supplies.store.items() {
        let item = stored.item();
        if !fits(cx, &item.base_name, socket, slot) {
            continue;
        }
        let held = item.stack_count.max(1);
        by_record
            .entry(item.base_name.clone())
            .and_modify(|candidate| candidate.count = candidate.count.saturating_add(held))
            .or_insert_with(|| Candidate {
                source: PartSource::Store(stored.id()),
                name: part_name(cx, &item.base_name),
                count: held,
            });
    }
    if socket == Socket::Component
        && let Some(storage) = supplies.reagents
    {
        for (index, entry) in storage.entries.iter().enumerate() {
            if fits(cx, &entry.record, socket, slot) {
                by_record.insert(
                    format!("storage:{}", entry.record),
                    Candidate {
                        source: PartSource::Reagent(ReagentIndex::new(index)),
                        name: part_name(cx, &entry.record),
                        count: entry.count,
                    },
                );
            }
        }
    }
    let mut candidates: Vec<Candidate> = by_record.into_values().collect();
    candidates.sort_by(|a, b| a.name.cmp(&b.name));
    candidates
}

fn fits(cx: &mut PaneCtx<'_>, record: &str, socket: Socket, slot: Slot) -> bool {
    cx.facts
        .part(cx.game, record)
        .is_some_and(|part| part.socket() == socket && part.fits(slot))
}
