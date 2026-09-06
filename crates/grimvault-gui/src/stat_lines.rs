//! The stat body of the item tooltip: the blocks `grimvault_core::stats`
//! assembles, drawn in the tooltip's order with the theme's colours —
//! base stats in the strong text colour, bonuses in the game's warm
//! stat tint, block titles and set names as headings, descriptions
//! muted — then the set and the requirement lines.

use egui::{Color32, RichText, Ui};
use grimvault_core::gamedata::GameData;
use grimvault_core::item::Item;
use grimvault_core::stats::{self, Block, BlockSource, Emphasis, Section, StatLine};
use univault_ui::theme::Palette;

/// The warm tint the game gives bonus stat text.
const BONUS: Color32 = Color32::from_rgb(222, 206, 160);

/// The green the game names sets in.
const SET: Color32 = Color32::from_rgb(120, 200, 120);

/// Draws every stat block, the set, and the requirements of `item`.
pub fn stat_body(ui: &mut Ui, game: &GameData, palette: &Palette, item: &Item) {
    let details = stats::item_details(game, item);
    let mut drawn_any = false;
    for block in &details.blocks {
        if block.lines.is_empty() && block.flavor.is_none() {
            continue;
        }
        if drawn_any {
            ui.add_space(4.0);
        }
        drawn_any = true;
        block_title(ui, palette, block);
        let mut section: Option<Section> = None;
        for line in &block.lines {
            if section.is_some_and(|last| last != line.section) {
                ui.add_space(2.0);
            }
            section = Some(line.section);
            ui.label(stat_text(palette, line));
        }
        if let Some(flavor) = &block.flavor {
            ui.label(RichText::new(flavor).italics().color(palette.text_weak));
        }
    }
    if let Some(set) = &details.set {
        ui.add_space(4.0);
        ui.label(RichText::new(&set.name).color(SET).strong());
        for member in &set.members {
            ui.label(RichText::new(format!("    {member}")).color(palette.text_weak));
        }
        for tier in &set.tiers {
            for line in &tier.lines {
                ui.label(RichText::new(format!("({}) {}", tier.pieces, line.text)).color(SET));
            }
        }
    }
    if !details.requirement_lines.is_empty() {
        ui.add_space(4.0);
        for line in &details.requirement_lines {
            ui.label(RichText::new(&line.text).color(palette.text_weak));
        }
    }
}

/// The heading a non-base block carries: the component's or augment's
/// own name, the game's "Ascended Bonus", or the block's label when
/// the record is one the database could not name.
fn block_title(ui: &mut Ui, palette: &Palette, block: &Block) {
    let title = match (block.source, &block.title) {
        (BlockSource::Base | BlockSource::Prefix | BlockSource::Suffix, _) => return,
        (_, Some(title)) => title.clone(),
        (source, None) => source.label().to_string(),
    };
    ui.label(RichText::new(title).color(palette.heading).strong());
}

fn stat_text(palette: &Palette, line: &StatLine) -> RichText {
    let text = RichText::new(&line.text);
    match line.emphasis {
        Emphasis::Base => text.color(palette.text_strong),
        Emphasis::Bonus => text.color(BONUS),
        Emphasis::Heading => text.color(palette.heading).strong(),
        Emphasis::Muted => text.color(palette.text_weak).italics(),
    }
}
