//! The search bar: a name field and the facet toggles bound to the
//! core [`Query`]. The pane that shows it applies the query itself and
//! reports what the filter hid, so an item whose facets could not be
//! resolved is counted rather than silently dropped.

use egui::{TextEdit, Ui};
use grimvault_core::search::{AscensionFilter, Constraint, Query};

/// Draws the bar; `true` when the query changed this frame.
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
        changed |= toggle(ui, &mut query.monster_infrequent, "Monster infrequent");
        changed |= toggle(ui, &mut query.double_rare, "Double rare");
        let before = query.ascension;
        egui::ComboBox::from_id_salt("ascension-filter")
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
        changed |= query.ascension != before;
        if !query.is_empty() && ui.small_button("clear").clicked() {
            *query = Query::default();
            changed = true;
        }
    });
    changed
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

/// What the filter did, for the line under the bar: how many of the
/// bucket's items it kept, and how many it could not decide about.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_what_the_filter_hid() {
        assert_eq!(summary(5, 5, 0), "5 shown");
        assert_eq!(summary(2, 5, 0), "2 of 5 match");
        assert!(summary(2, 5, 1).starts_with("2 of 5 match · 1 hidden"));
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
}
