//! The transfer stash pane: one tab strip, the selected tab as an
//! editable grid. Tab buttons double as drop targets (first fit in
//! that tab) while a drag is in flight.

use egui::{CornerRadius, Stroke, StrokeKind, Ui, Vec2};
use grimvault_core::block::StashTab;
use grimvault_core::transfer::TabIndex;
use univault_ui::theme::Theme;

use super::{
    DragFrame, DropCandidate, GridSpec, Interaction, PaneCtx, grid_surface, stash_entries,
};
use crate::documents::StashDoc;
use crate::drag::{DropTarget, Fit};
use crate::grid::FootprintSource;
use crate::theme::FITS;

/// Which tab the pane shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StashView {
    pub tab: TabIndex,
}

impl Default for StashView {
    fn default() -> Self {
        Self {
            tab: TabIndex::new(0),
        }
    }
}

pub fn show(
    ui: &mut Ui,
    doc: &StashDoc,
    view: &mut StashView,
    theme: &Theme,
    cx: &mut PaneCtx<'_>,
    frame: &mut DragFrame,
) {
    ui.label(theme.heading("Transfer stash"));
    ui.label(theme.path_text(doc.path().display().to_string()));
    let stash = doc.stash();
    ui.horizontal_wrapped(|ui| {
        for (slot, tab) in stash.tabs.iter().enumerate() {
            let Ok(index) = u32::try_from(slot).map(TabIndex::new) else {
                continue;
            };
            let response = ui.selectable_label(view.tab == index, tab_label(slot, tab));
            if response.clicked() {
                view.tab = index;
            }
            if cx.drag.is_some() && response.contains_pointer() {
                ui.painter().rect_stroke(
                    response.rect,
                    CornerRadius::same(2),
                    Stroke::new(2.0, FITS),
                    StrokeKind::Outside,
                );
                frame.candidate = Some(DropCandidate {
                    target: DropTarget::StashTab(index),
                    fit: Fit::Fits,
                });
            }
        }
    });
    let Some(tab) = usize::try_from(view.tab.value())
        .ok()
        .and_then(|slot| stash.tabs.get(slot))
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

fn tab_label(slot: usize, tab: &StashTab) -> String {
    let name = if tab.decoration.button_name.is_empty() {
        format!("Tab {}", slot + 1)
    } else {
        tab.decoration.button_name.clone()
    };
    format!("{name} ({})", tab.items.len())
}
