//! The left pane: one tab strip over the transfer stash's tabs and,
//! beside them, the component / crafting-material storage's two tabs.
//! A stash tab shows as an editable grid, a storage tab as rows. Tab
//! buttons double as drop targets — first fit in a stash tab, merge by
//! record in a storage tab — while a drag is in flight.

use egui::{Ui, Vec2};
use grimvault_core::block::StashTab;
use grimvault_core::campaign::Campaign;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::transfer::TabIndex;
use univault_ui::theme::Theme;

use super::{
    DragFrame, DropCandidate, GridSpec, Interaction, PaneCtx, container_tab, grid_surface, outline,
    reagents, stash_entries,
};
use crate::documents::{Reagents, StashDoc};
use crate::drag::{self, Container, DropTarget, Fit};
use crate::grid::FootprintSource;

/// Which surface the pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Showing {
    Stash,
    Reagents(ReagentKind),
}

/// The pane's selection: the surface showing, the stash tab last
/// chosen (the target of a double-clicked store item even while a
/// storage tab shows), and how many a storage drag carries (0 for the
/// whole entry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StashView {
    pub tab: TabIndex,
    pub showing: Showing,
    pub reagent_amount: u32,
}

impl Default for StashView {
    fn default() -> Self {
        Self {
            tab: TabIndex::new(0),
            showing: Showing::Stash,
            reagent_amount: 0,
        }
    }
}

const CAMPAIGN_WHY: &str = "Whose shared files show here: the main campaign keeps its stash and \
     component storage beside main/, and each mod keeps its own under save/<Mod>/. The app opens \
     on whichever the game wrote last. Characters are not tied to a campaign — the game lists \
     every custom-game character under every mod — so the character list never changes.";

/// The campaign whose files the pane shows, among those the save
/// directory holds.
#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub campaign: &'a Campaign,
    pub campaigns: &'a [Campaign],
}

/// The selected campaign's shared files.
#[derive(Clone, Copy)]
pub struct Shared<'a> {
    pub stash: &'a StashDoc,
    pub reagents: &'a Reagents,
}

/// Draws the pane; `Some` when the user picked another campaign,
/// whose files the shell then opens in place of `shared`.
pub fn show(
    ui: &mut Ui,
    selection: Selection<'_>,
    shared: Shared<'_>,
    view: &mut StashView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) -> Option<Campaign> {
    let Shared {
        stash: doc,
        reagents: storage,
    } = shared;
    let (heading, path) = match view.showing {
        Showing::Stash => ("Transfer stash", doc.path()),
        Showing::Reagents(_) => ("Component storage", storage.path()),
    };
    let mut switch = None;
    ui.horizontal(|ui| {
        ui.label(theme.heading(heading));
        egui::ComboBox::from_id_salt("campaign-picker")
            .selected_text(selection.campaign.to_string())
            .show_ui(ui, |ui| {
                for candidate in selection.campaigns {
                    let current = candidate == selection.campaign;
                    if ui
                        .selectable_label(current, candidate.to_string())
                        .clicked()
                        && !current
                    {
                        switch = Some(candidate.clone());
                    }
                }
            })
            .response
            .on_hover_text(CAMPAIGN_WHY);
    });
    ui.label(theme.path_text(path.display().to_string()));
    let stash = doc.stash();
    ui.horizontal_wrapped(|ui| {
        for (slot, tab) in stash.tabs.iter().enumerate() {
            let Ok(index) = u32::try_from(slot).map(TabIndex::new) else {
                continue;
            };
            let selected = view.showing == Showing::Stash && view.tab == index;
            let response = container_tab(
                ui,
                selected,
                tab_label(slot, tab),
                Container::TransferStash(index),
                cx,
                frame,
            );
            if response.clicked() {
                view.tab = index;
                view.showing = Showing::Stash;
            }
        }
        ui.separator();
        for kind in ReagentKind::ALL {
            let selected = view.showing == Showing::Reagents(kind);
            let response = ui.selectable_label(selected, reagent_tab_label(storage, kind, cx));
            if response.clicked() {
                view.showing = Showing::Reagents(kind);
            }
            if let Some(drag) = cx.drag
                && response.contains_pointer()
            {
                let fit = match storage {
                    Reagents::Open(_) => drag::fit_in_reagents(
                        drag.source,
                        cx.facts.base(cx.game, &drag.item).reagent,
                    ),
                    Reagents::Absent { .. } | Reagents::Failed { .. } => Fit::Blocked,
                };
                outline(ui, response.rect, fit);
                frame.candidate = Some(DropCandidate {
                    target: DropTarget::Reagents(kind),
                    fit,
                });
            }
        }
    });
    match view.showing {
        Showing::Stash => show_tab(ui, stash.tabs.as_slice(), view, cx, frame),
        Showing::Reagents(kind) => {
            reagents::show(ui, storage, kind, &mut view.reagent_amount, cx, frame);
        }
    }
    switch
}

fn show_tab(
    ui: &mut Ui,
    tabs: &[StashTab],
    view: &StashView,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let Some(tab) = usize::try_from(view.tab.value())
        .ok()
        .and_then(|slot| tabs.get(slot))
    else {
        ui.label("The stash has no tabs.");
        return;
    };
    ui.label(format!(
        "{}×{} cells · {} items",
        tab.width,
        tab.height,
        tab.items.len()
    ));
    let entries = stash_entries(tab, cx);
    let unresolved = entries
        .iter()
        .filter(|entry| entry.footprint == FootprintSource::Assumed)
        .count();
    if unresolved > 0 {
        ui.colored_label(
            cx.palette.warn,
            format!("{unresolved} item(s) have no known footprint (drawn 1×1, marked !); nothing can be placed in this tab until the database knows them"),
        );
    }
    let available: Vec2 = ui.available_size();
    let cols = i32::try_from(tab.width).unwrap_or(0);
    let rows = i32::try_from(tab.height).unwrap_or(0);
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            grid_surface(
                ui,
                GridSpec {
                    available,
                    cols,
                    rows,
                },
                &entries,
                Interaction::Editable(Container::TransferStash(view.tab)),
                cx,
                frame,
            );
        });
}

fn tab_label(slot: usize, tab: &StashTab) -> String {
    let name = if tab.decoration.button_name.is_empty() {
        format!("Tab {}", slot + 1)
    } else {
        tab.decoration.button_name.clone()
    };
    format!("{name} ({})", tab.items.len())
}

fn reagent_tab_label(storage: &Reagents, kind: ReagentKind, cx: &mut PaneCtx<'_>) -> String {
    match storage {
        Reagents::Open(doc) => format!(
            "{} ({})",
            kind.label(),
            reagents::rows(doc.storage(), kind, cx).len()
        ),
        Reagents::Absent { .. } | Reagents::Failed { .. } => format!("{} (–)", kind.label()),
    }
}
