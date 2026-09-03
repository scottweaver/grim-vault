//! Grim Vault's look: `univault_ui`'s theme machinery over this app's
//! own palette — an ashen dark ground with ember and iron accents — and
//! the game's rarity colours for item names and tile borders. Nothing
//! is bundled: egui's own faces carry the text and every icon is
//! decoded from the player's install at runtime.

use egui::Color32;
use grimvault_core::gamedata::Rarity;
use univault_ui::theme::{Fonts, Palette, Theme};

/// Ember over charcoal, with iron-grey grids so the game's own rarity
/// colours read against them.
#[must_use]
pub const fn palette() -> Palette {
    Palette {
        accent: Color32::from_rgb(214, 108, 46),
        accent_dim: Color32::from_rgb(132, 66, 30),
        accent_faint: Color32::from_rgb(84, 48, 30),
        heading: Color32::from_rgb(232, 150, 84),
        text: Color32::from_rgb(204, 198, 190),
        text_strong: Color32::from_rgb(236, 232, 226),
        text_weak: Color32::from_rgb(140, 134, 128),
        surface: Color32::from_rgb(24, 22, 22),
        surface_raised: Color32::from_rgb(38, 35, 35),
        surface_deep: Color32::from_rgb(14, 13, 13),
        popup: Color32::from_rgb(10, 9, 9),
        grid_bg: Color32::from_rgb(30, 30, 32),
        grid_line: Color32::from_rgb(52, 52, 56),
        tile_bg: Color32::from_rgb(44, 44, 48),
        tile_edge: Color32::from_rgb(72, 72, 78),
        faint_bg: Color32::from_rgb(32, 30, 30),
        selection_bg: Color32::from_rgb(112, 58, 28),
        selection_stroke: Color32::from_rgb(240, 160, 90),
        hover_bg: Color32::from_rgb(52, 44, 40),
        active_bg: Color32::from_rgb(66, 52, 44),
        warn: Color32::from_rgb(235, 180, 76),
        error: Color32::from_rgb(220, 80, 70),
    }
}

/// The whole look, with egui's default faces.
#[must_use]
pub fn theme() -> Theme {
    Theme {
        palette: palette(),
        fonts: Fonts::default(),
    }
}

/// The game's item-quality colours as the tooltip and tile borders
/// use them.
#[must_use]
pub const fn rarity_color(rarity: Rarity) -> Color32 {
    match rarity {
        Rarity::Common => Color32::from_rgb(205, 205, 205),
        Rarity::Magical => Color32::from_rgb(240, 220, 80),
        Rarity::Rare => Color32::from_rgb(96, 220, 96),
        Rarity::Epic => Color32::from_rgb(96, 150, 255),
        Rarity::Legendary => Color32::from_rgb(190, 110, 255),
        Rarity::Quest => Color32::from_rgb(224, 170, 100),
    }
}

/// The colour for an item whose record (and so rarity) the database
/// does not know.
pub const UNKNOWN_RARITY: Color32 = Color32::from_rgb(150, 140, 140);

/// Fits-here green and blocked red for the drop preview.
pub const FITS: Color32 = Color32::from_rgb(64, 230, 96);
pub const BLOCKED: Color32 = Color32::from_rgb(235, 70, 60);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rarity_has_a_distinct_colour() {
        let colours = [
            Rarity::Common,
            Rarity::Magical,
            Rarity::Rare,
            Rarity::Epic,
            Rarity::Legendary,
            Rarity::Quest,
        ]
        .map(rarity_color);
        for (index, colour) in colours.iter().enumerate() {
            assert!(
                !colours[index + 1..].contains(colour),
                "duplicate colour {colour:?}"
            );
            assert_ne!(*colour, UNKNOWN_RARITY);
        }
    }

    #[test]
    fn the_theme_installs() {
        let ctx = egui::Context::default();
        theme().apply(&ctx);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            palette().surface
        );
    }
}
