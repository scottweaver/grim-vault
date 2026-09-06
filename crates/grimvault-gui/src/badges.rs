//! The game's own inventory-tile symbols — monster infrequent, double
//! rare, ascended — painted over an item's tile the way the game does,
//! with a vector diamond standing in when the symbol's texture is not
//! at hand. The textures come out of the install's `UI.arc` at load
//! ([`crate::loader`]); an absent archive or entry is a recorded
//! absence, not a retry per frame.

use std::collections::HashMap;

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use grimvault_core::facets::Symbol;
use univault_ui::theme::Palette;

use crate::icons::{Icon, IconProblem, upload};

/// The `.tex` bytes of each symbol the load could find, or why not.
#[derive(Clone, Debug, Default)]
pub struct SymbolTextures {
    sources: HashMap<Symbol, Result<Vec<u8>, IconProblem>>,
}

impl SymbolTextures {
    /// Records a symbol's bytes, or the reason there are none.
    pub fn insert(&mut self, symbol: Symbol, source: Result<Vec<u8>, IconProblem>) {
        self.sources.insert(symbol, source);
    }

    /// How many symbols have their texture.
    #[must_use]
    pub fn found(&self) -> usize {
        self.sources
            .values()
            .filter(|source| source.is_ok())
            .count()
    }

    /// Why a symbol has no texture, for the load transcript.
    pub fn problems(&self) -> impl Iterator<Item = (Symbol, &IconProblem)> {
        Symbol::ALL
            .into_iter()
            .filter_map(|symbol| match self.sources.get(&symbol) {
                Some(Ok(_)) => None,
                Some(Err(problem)) => Some((symbol, problem)),
                None => Some((symbol, &IconProblem::NoBitmap)),
            })
    }
}

/// The symbols as GPU textures, uploaded on first sight.
#[derive(Default)]
pub struct SymbolCache {
    sources: SymbolTextures,
    uploaded: HashMap<Symbol, Icon>,
}

impl SymbolCache {
    #[must_use]
    pub fn new(sources: SymbolTextures) -> Self {
        Self {
            sources,
            uploaded: HashMap::new(),
        }
    }

    /// The symbol's texture, or the reason it is missing.
    pub fn icon(&mut self, ctx: &egui::Context, symbol: Symbol) -> Icon {
        if let Some(icon) = self.uploaded.get(&symbol) {
            return icon.clone();
        }
        let icon = match self.sources.sources.get(&symbol) {
            Some(Ok(bytes)) => upload(ctx, symbol.variable(), bytes),
            Some(Err(problem)) => Icon::Missing(problem.clone()),
            None => Icon::Missing(IconProblem::NoBitmap),
        };
        self.uploaded.insert(symbol, icon.clone());
        icon
    }
}

/// A symbol to paint on a tile, with its texture when there is one.
pub struct Badge<'a> {
    pub symbol: Symbol,
    pub icon: &'a Icon,
}

/// Paints the badge in the tile's top-left corner: the game's texture
/// when it loaded, else a small diamond in the symbol's colour.
pub fn paint_badge(painter: &Painter, tile: Rect, cell: f32, badge: &Badge<'_>, palette: &Palette) {
    let rect = badge_rect(tile, cell);
    match badge.icon {
        Icon::Texture(texture) => {
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        Icon::Missing(_) => {
            painter.add(Shape::convex_polygon(
                diamond(rect),
                glyph_colour(badge.symbol, palette),
                Stroke::new(1.0, palette.surface_deep),
            ));
        }
    }
}

/// The badge's side as a fraction of one grid cell. The game's symbol
/// is a 32 px texture whose glyph fills it edge to edge (measured
/// 2..=29 of 32 on every symbol), so drawing it a full cell would hide
/// a one-cell item; a little over half a cell keeps it legible on a
/// ring and still reads on a three-cell weapon.
const BADGE_PER_CELL: f32 = 0.625;
/// Smallest badge still recognisable, for panes that shrink cells.
const BADGE_MIN: f32 = 14.0;
/// The texture's native size; never upscaled past it.
const BADGE_MAX: f32 = 32.0;

/// Where the badge sits: the tile's top-left corner, sized from the
/// pane's cell (`cell` points per footprint cell) rather than from the
/// item, so a thin weapon and a square chest piece carry the same mark.
#[must_use]
pub fn badge_rect(tile: Rect, cell: f32) -> Rect {
    let size = (cell * BADGE_PER_CELL).clamp(BADGE_MIN, BADGE_MAX);
    Rect::from_min_size(tile.left_top() + vec2(1.0, 1.0), vec2(size, size))
}

fn diamond(rect: Rect) -> Vec<Pos2> {
    let inset = rect.shrink(rect.width() * 0.15);
    vec![
        pos2(inset.center().x, inset.min.y),
        pos2(inset.max.x, inset.center().y),
        pos2(inset.center().x, inset.max.y),
        pos2(inset.min.x, inset.center().y),
    ]
}

/// The stand-in diamond's colour: silver for a monster infrequent,
/// green for a double rare, gold for both, the accent for anything
/// ascended.
#[must_use]
pub fn glyph_colour(symbol: Symbol, palette: &Palette) -> Color32 {
    match symbol {
        Symbol::MonsterInfrequent => Color32::from_rgb(205, 205, 215),
        Symbol::DoubleRare => Color32::from_rgb(96, 220, 96),
        Symbol::DoubleRareMonsterInfrequent => Color32::from_rgb(235, 190, 70),
        Symbol::CommonAscended
        | Symbol::MagicalAscended
        | Symbol::RareAscended
        | Symbol::DoubleRareAscended
        | Symbol::MonsterDoubleRareAscended
        | Symbol::EpicAscended
        | Symbol::LegendaryAscended
        | Symbol::Awakened => palette.accent,
    }
}

#[cfg(test)]
mod tests {
    use univault_engine::tex::fixture::tex_with_pixels;

    use super::*;
    use crate::theme::palette;

    #[test]
    fn the_badge_scales_with_the_cell_not_the_item() {
        let ring = Rect::from_min_size(pos2(10.0, 10.0), vec2(32.0, 32.0));
        assert_eq!(badge_rect(ring, 32.0).size(), vec2(20.0, 20.0));
        assert_eq!(badge_rect(ring, 32.0).min, pos2(11.0, 11.0));
        let weapon = Rect::from_min_size(Pos2::ZERO, vec2(32.0, 96.0));
        assert_eq!(badge_rect(weapon, 32.0).size(), vec2(20.0, 20.0));
        let shrunk = Rect::from_min_size(Pos2::ZERO, vec2(20.0, 20.0));
        assert_eq!(badge_rect(shrunk, 20.0).size(), vec2(14.0, 14.0));
        let zoomed = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0));
        assert_eq!(badge_rect(zoomed, 200.0).size(), vec2(32.0, 32.0));
    }

    #[test]
    fn a_missing_texture_is_a_recorded_absence_with_its_reason() {
        let mut textures = SymbolTextures::default();
        textures.insert(Symbol::DoubleRare, Err(IconProblem::NotInArchives));
        assert_eq!(textures.found(), 0);
        let problems: Vec<(Symbol, &IconProblem)> = textures.problems().collect();
        assert_eq!(problems.len(), Symbol::ALL.len());
        assert!(problems.contains(&(Symbol::DoubleRare, &IconProblem::NotInArchives)));
        assert!(problems.contains(&(Symbol::MonsterInfrequent, &IconProblem::NoBitmap)));

        let ctx = egui::Context::default();
        let mut cache = SymbolCache::new(textures);
        assert!(matches!(
            cache.icon(&ctx, Symbol::DoubleRare),
            Icon::Missing(IconProblem::NotInArchives)
        ));
        assert!(matches!(
            cache.icon(&ctx, Symbol::MonsterInfrequent),
            Icon::Missing(IconProblem::NoBitmap)
        ));
    }

    #[test]
    fn a_found_texture_uploads_once_and_an_undecodable_one_is_reported() {
        let mut textures = SymbolTextures::default();
        textures.insert(
            Symbol::MonsterInfrequent,
            Ok(tex_with_pixels(2, 2, 32, &[0xFF; 16])),
        );
        textures.insert(Symbol::DoubleRare, Ok(b"not a texture".to_vec()));
        assert_eq!(textures.found(), 2);
        let ctx = egui::Context::default();
        let mut cache = SymbolCache::new(textures);
        assert!(matches!(
            cache.icon(&ctx, Symbol::MonsterInfrequent),
            Icon::Texture(_)
        ));
        assert!(matches!(
            cache.icon(&ctx, Symbol::MonsterInfrequent),
            Icon::Texture(_)
        ));
        assert!(matches!(
            cache.icon(&ctx, Symbol::DoubleRare),
            Icon::Missing(IconProblem::Undecodable(_))
        ));
    }

    #[test]
    fn glyph_colours_tell_the_three_unascended_marks_apart() {
        let palette = palette();
        let colours = [
            Symbol::MonsterInfrequent,
            Symbol::DoubleRare,
            Symbol::DoubleRareMonsterInfrequent,
            Symbol::RareAscended,
        ]
        .map(|symbol| glyph_colour(symbol, &palette));
        for (index, colour) in colours.iter().enumerate() {
            assert!(!colours[index + 1..].contains(colour), "{colour:?}");
        }
    }
}
