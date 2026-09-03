// Vendored from tq-univault crates/univault-gui/src/chrome.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Game-chrome mechanics with the art injected: three-sliced plate
//! buttons and nameplates, an eight-tile tooltip border, and a
//! seam-free mirrored collage for stretched backdrops. Which textures
//! to load, where to slice them, and what to fall back to when the
//! art is absent all stay with the app.

use egui::{Color32, ColorImage, FontId, Painter, Rect, Response, Sense, TextureHandle, Ui, vec2};

use crate::slice::{Src, ThreeSlice, blit};

/// Room around a label on its plate.
const BUTTON_PAD: f32 = 24.0;
const NAMEPLATE_PAD: f32 = 56.0;

/// Dims a disabled button's plate; the label ink is the art's own.
const DISABLED_TINT: Color32 = Color32::from_gray(170);

/// A plate button's art: one whole-texture strip per pointer state,
/// each with `caps`-wide end caps, painted `height` tall; `ink` and
/// `disabled_ink` letter the label.
#[derive(Clone)]
pub struct ButtonArt {
    pub up: TextureHandle,
    pub over: TextureHandle,
    pub down: TextureHandle,
    pub caps: f32,
    pub height: f32,
    pub ink: Color32,
    pub disabled_ink: Color32,
}

/// A plate button in the three pointer states. A disabled button
/// shows the resting plate dimmed and senses hover only.
pub fn button(ui: &mut Ui, art: &ButtonArt, enabled: bool, text: &str) -> Response {
    let ink = if enabled { art.ink } else { art.disabled_ink };
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        ink,
    );
    let size = vec2(galley.size().x + BUTTON_PAD, art.height);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let (plate, tint) = if !enabled {
        (&art.up, DISABLED_TINT)
    } else if response.is_pointer_button_down_on() {
        (&art.down, Color32::WHITE)
    } else if response.hovered() {
        (&art.over, Color32::WHITE)
    } else {
        (&art.up, Color32::WHITE)
    };
    ThreeSlice::whole(plate, art.caps).paint(ui.painter(), plate, rect, tint);
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, ink);
    response
}

/// A title plate's art: one whole-texture strip with `caps`-wide end
/// caps, painted `height` tall, lettered in `ink`.
#[derive(Clone)]
pub struct NameplateArt {
    pub plate: TextureHandle,
    pub caps: f32,
    pub height: f32,
    pub ink: Color32,
}

/// A title on its plate, set in `font` (normally the theme's heading
/// face).
pub fn nameplate(ui: &mut Ui, art: &NameplateArt, font: FontId, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, art.ink);
    let size = vec2(galley.size().x + NAMEPLATE_PAD, art.height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ThreeSlice::whole(&art.plate, art.caps).paint(ui.painter(), &art.plate, rect, Color32::WHITE);
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, art.ink);
}

/// A border assembled from eight whole textures — four corners and
/// four edges — each stretched into a `band`-thick strip.
#[derive(Clone)]
pub struct TooltipArt {
    pub top_left: TextureHandle,
    pub top: TextureHandle,
    pub top_right: TextureHandle,
    pub left: TextureHandle,
    pub right: TextureHandle,
    pub bottom_left: TextureHandle,
    pub bottom: TextureHandle,
    pub bottom_right: TextureHandle,
    pub band: f32,
}

/// The thin tooltip border along the edges of `rect`.
pub fn tooltip_frame(painter: &Painter, art: &TooltipArt, rect: Rect) {
    let band = art.band;
    let (min, max) = (rect.min, rect.max);
    let pieces = [
        (&art.top_left, Rect::from_min_size(min, vec2(band, band))),
        (
            &art.top_right,
            Rect::from_min_size(egui::pos2(max.x - band, min.y), vec2(band, band)),
        ),
        (
            &art.bottom_left,
            Rect::from_min_size(egui::pos2(min.x, max.y - band), vec2(band, band)),
        ),
        (
            &art.bottom_right,
            Rect::from_min_size(max - vec2(band, band), vec2(band, band)),
        ),
        (
            &art.top,
            Rect::from_min_max(
                egui::pos2(min.x + band, min.y),
                egui::pos2(max.x - band, min.y + band),
            ),
        ),
        (
            &art.bottom,
            Rect::from_min_max(
                egui::pos2(min.x + band, max.y - band),
                egui::pos2(max.x - band, max.y),
            ),
        ),
        (
            &art.left,
            Rect::from_min_max(
                egui::pos2(min.x, min.y + band),
                egui::pos2(min.x + band, max.y - band),
            ),
        ),
        (
            &art.right,
            Rect::from_min_max(
                egui::pos2(max.x - band, min.y + band),
                egui::pos2(max.x, max.y - band),
            ),
        ),
    ];
    for (texture, dest) in pieces {
        blit(painter, texture, Src::full(texture), dest, Color32::WHITE);
    }
}

/// A `tiles[0]`×`tiles[1]` collage of the `block`-sized rectangle at
/// `origin` in `image`, alternate tiles mirrored so the result
/// stretches with no hard seams. `None` when the block does not lie
/// inside the image or is empty.
#[must_use]
pub fn mirror_collage(
    image: &ColorImage,
    origin: [usize; 2],
    block: [usize; 2],
    tiles: [usize; 2],
) -> Option<ColorImage> {
    let [x0, y0] = origin;
    let [w, h] = block;
    let [cols, rows] = tiles;
    if w == 0 || h == 0 || image.width() < x0 + w || image.height() < y0 + h {
        return None;
    }
    let (out_w, out_h) = (cols * w, rows * h);
    let mirror = |offset: usize, tile: usize, len: usize| {
        if tile.is_multiple_of(2) {
            offset
        } else {
            len - 1 - offset
        }
    };
    let pixels = (0..out_h)
        .flat_map(|y| (0..out_w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let sx = mirror(x % w, x / w, w);
            let sy = mirror(y % h, y / h, h);
            image.pixels[(y0 + sy) * image.width() + x0 + sx]
        })
        .collect();
    Some(ColorImage::new([out_w, out_h], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Color32 = Color32::from_rgb(1, 0, 0);
    const B: Color32 = Color32::from_rgb(2, 0, 0);
    const C: Color32 = Color32::from_rgb(3, 0, 0);
    const D: Color32 = Color32::from_rgb(4, 0, 0);

    #[test]
    fn collage_mirrors_alternate_tiles_on_both_axes() {
        let image = ColorImage::new([2, 2], vec![A, B, C, D]);
        let collage = mirror_collage(&image, [0, 0], [2, 2], [2, 2]).unwrap();
        assert_eq!(collage.size, [4, 4]);
        assert_eq!(
            collage.pixels,
            vec![A, B, B, A, C, D, D, C, C, D, D, C, A, B, B, A]
        );
    }

    #[test]
    fn collage_samples_the_block_at_its_origin() {
        let image = ColorImage::new([3, 1], vec![A, B, C]);
        let collage = mirror_collage(&image, [1, 0], [2, 1], [2, 1]).unwrap();
        assert_eq!(collage.pixels, vec![B, C, C, B]);
    }

    #[test]
    fn collage_refuses_a_block_outside_the_image() {
        let image = ColorImage::new([3, 1], vec![A, B, C]);
        assert!(mirror_collage(&image, [2, 0], [2, 1], [1, 1]).is_none());
        assert!(mirror_collage(&image, [0, 0], [0, 1], [1, 1]).is_none());
    }
}
