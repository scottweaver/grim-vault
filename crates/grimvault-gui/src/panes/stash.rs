//! The left pane: one tab strip over the transfer stash's tabs and,
//! beside them, the component / crafting-material storage's two tabs.
//! A stash tab shows as an editable grid, a storage tab as rows. Tab
//! buttons double as drop targets — first fit in a stash tab, merge by
//! record in a storage tab — while a drag is in flight.

use egui::{CornerRadius, Stroke, StrokeKind, Ui, Vec2};
use grimvault_core::block::StashTab;
use grimvault_core::reagents::ReagentKind;
use grimvault_core::transfer::TabIndex;
use univault_ui::theme::Theme;

use super::{
    DragFrame, DropCandidate, GridSpec, Interaction, PaneCtx, grid_surface, reagents, stash_entries,
};
use crate::documents::{Reagents, StashDoc};
use crate::drag::{self, DropTarget, Fit};
use crate::grid::FootprintSource;
use crate::theme::{BLOCKED, FITS};

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

pub fn show(
    ui: &mut Ui,
    doc: &StashDoc,
    storage: &Reagents,
    view: &mut StashView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    let (heading, path) = match view.showing {
        Showing::Stash => ("Transfer stash", doc.path()),
        Showing::Reagents(_) => ("Component storage", storage.path()),
    };
    ui.label(theme.heading(heading));
    ui.label(theme.path_text(path.display().to_string()));
    let stash = doc.stash();
    ui.horizontal_wrapped(|ui| {
        for (slot, tab) in stash.tabs.iter().enumerate() {
            let Ok(index) = u32::try_from(slot).map(TabIndex::new) else {
                continue;
            };
            let selected = view.showing == Showing::Stash && view.tab == index;
            let response = ui.selectable_label(selected, tab_label(slot, tab));
            if response.clicked() {
                view.tab = index;
                view.showing = Showing::Stash;
            }
            if cx.drag.is_some() && response.contains_pointer() {
                outline(ui, response.rect, Fit::Fits);
                frame.candidate = Some(DropCandidate {
                    target: DropTarget::StashTab(index),
                    fit: Fit::Fits,
                });
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
                Interaction::Editable { tab: view.tab },
                cx,
                frame,
            );
        });
}

fn outline(ui: &Ui, rect: egui::Rect, fit: Fit) {
    let colour = match fit {
        Fit::Fits => FITS,
        Fit::Blocked | Fit::Unresolvable => BLOCKED,
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(2.0, colour),
        StrokeKind::Outside,
    );
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
