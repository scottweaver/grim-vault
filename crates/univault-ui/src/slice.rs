// Vendored from tq-univault crates/univault-gui/src/chrome.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Texture slicing, expressed once: [`Src`] names a pixel rectangle
//! of a texture, [`Piece`] orients it, and the two shapes every
//! component paints with are values built from them — [`ThreeSlice`],
//! the caps-preserving strip under plates and buttons, and
//! [`NinePatch`], the frame under borders and rails. An app describes
//! its own art's layout with these and paints over any
//! [`TextureHandle`]; nothing here knows which texture it is.

use egui::{Color32, Painter, Rect, TextureHandle, Vec2, pos2, vec2};

/// A pixel rectangle inside a source texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Src {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Src {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// The whole of `texture`.
    #[must_use]
    pub fn full(texture: &TextureHandle) -> Self {
        let size = texture.size_vec2();
        Self::new(0.0, 0.0, size.x, size.y)
    }

    #[must_use]
    pub const fn size(self) -> Vec2 {
        vec2(self.w, self.h)
    }

    /// This rectangle in normalised texture coordinates of a
    /// texture `texture_size` pixels large.
    #[must_use]
    pub fn uv(self, texture_size: Vec2) -> Rect {
        self.oriented(Flip::None).uv(texture_size)
    }

    #[must_use]
    pub const fn oriented(self, flip: Flip) -> Piece {
        Piece { src: self, flip }
    }
}

/// How a piece is mirrored when painted, so one drawn corner or
/// strip can serve every side of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flip {
    None,
    Horizontal,
    Vertical,
    Both,
}

/// A source rectangle with the orientation it is painted in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub src: Src,
    pub flip: Flip,
}

impl Piece {
    /// The texture coordinates this piece samples: the source
    /// rectangle normalised by `texture_size`, with the mirrored
    /// axes swapped end for end.
    #[must_use]
    pub fn uv(self, texture_size: Vec2) -> Rect {
        let Src { x, y, w, h } = self.src;
        let (u0, u1) = (x / texture_size.x, (x + w) / texture_size.x);
        let (v0, v1) = (y / texture_size.y, (y + h) / texture_size.y);
        let (u0, u1) = match self.flip {
            Flip::None | Flip::Vertical => (u0, u1),
            Flip::Horizontal | Flip::Both => (u1, u0),
        };
        let (v0, v1) = match self.flip {
            Flip::None | Flip::Horizontal => (v0, v1),
            Flip::Vertical | Flip::Both => (v1, v0),
        };
        Rect::from_min_max(pos2(u0, v0), pos2(u1, v1))
    }

    #[must_use]
    pub const fn size(self) -> Vec2 {
        self.src.size()
    }
}

impl From<Src> for Piece {
    fn from(src: Src) -> Self {
        src.oriented(Flip::None)
    }
}

/// Paints `piece` of `texture` stretched over `dest`.
pub fn blit(
    painter: &Painter,
    texture: &TextureHandle,
    piece: impl Into<Piece>,
    dest: Rect,
    tint: Color32,
) {
    let piece = piece.into();
    painter.image(texture.id(), dest, piece.uv(texture.size_vec2()), tint);
}

/// A horizontal strip sliced in three: `caps` source pixels at each
/// end keep their proportions while the middle stretches to fit. The
/// strip is painted at its destination's height, so the caps scale
/// with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThreeSlice {
    pub src: Src,
    pub caps: f32,
}

impl ThreeSlice {
    #[must_use]
    pub const fn new(src: Src, caps: f32) -> Self {
        Self { src, caps }
    }

    /// A strip that is the whole of `texture`.
    #[must_use]
    pub fn whole(texture: &TextureHandle, caps: f32) -> Self {
        Self::new(Src::full(texture), caps)
    }

    /// The three source pieces and where each lands in `dest`. A
    /// destination narrower than its two caps loses the middle
    /// rather than folding it over.
    pub fn pieces(self, dest: Rect) -> impl Iterator<Item = (Src, Rect)> {
        let Src { x, y, w, h } = self.src;
        let caps = self.caps;
        let cap = caps * dest.height() / h;
        [
            (
                Src::new(x, y, caps, h),
                Rect::from_min_size(dest.min, vec2(cap, dest.height())),
            ),
            (
                Src::new(x + caps, y, w - 2.0 * caps, h),
                Rect::from_min_max(
                    pos2(dest.min.x + cap, dest.min.y),
                    pos2(dest.max.x - cap, dest.max.y),
                ),
            ),
            (
                Src::new(x + w - caps, y, caps, h),
                Rect::from_min_size(pos2(dest.max.x - cap, dest.min.y), vec2(cap, dest.height())),
            ),
        ]
        .into_iter()
        .filter(|(_, rect)| rect.width() > 0.0)
    }

    pub fn paint(self, painter: &Painter, texture: &TextureHandle, dest: Rect, tint: Color32) {
        for (src, rect) in self.pieces(dest) {
            blit(painter, texture, src, rect, tint);
        }
    }
}

/// The pieces of a nine-patch frame. Corners paint at their source
/// size; each edge stretches between the two corners it joins, at its
/// own source thickness; `center`, when present, stretches over the
/// interior inside the edge bands, and when absent the interior is
/// left untouched so whatever lies beneath shows through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NinePatch {
    pub top_left: Piece,
    pub top_right: Piece,
    pub bottom_left: Piece,
    pub bottom_right: Piece,
    pub top: Piece,
    pub bottom: Piece,
    pub left: Piece,
    pub right: Piece,
    pub center: Option<Piece>,
}

impl NinePatch {
    /// A frame cut from one rectangle of the sheet: square corners
    /// `corner` pixels on a side at its four corners, `band`-thick
    /// edge runs between them, no center. The natural layout for a
    /// frame drawn as a single closed image.
    #[must_use]
    pub fn symmetric(src: Src, corner: f32, band: f32) -> Self {
        let Src { x, y, w, h } = src;
        let (far_x, far_y) = (x + w - corner, y + h - corner);
        let (span_w, span_h) = (w - 2.0 * corner, h - 2.0 * corner);
        Self {
            top_left: Src::new(x, y, corner, corner).into(),
            top_right: Src::new(far_x, y, corner, corner).into(),
            bottom_left: Src::new(x, far_y, corner, corner).into(),
            bottom_right: Src::new(far_x, far_y, corner, corner).into(),
            top: Src::new(x + corner, y, span_w, band).into(),
            bottom: Src::new(x + corner, y + h - band, span_w, band).into(),
            left: Src::new(x, y + corner, band, span_h).into(),
            right: Src::new(x + w - band, y + corner, band, span_h).into(),
            center: None,
        }
    }

    /// A frame drawn once and mirrored: `corner` is the top-left
    /// corner, `top` and `left` are clean runs of those two edges,
    /// and every other corner and edge is a flip of one of them —
    /// symmetric by construction, so the art only has to get one
    /// corner and two strips right.
    #[must_use]
    pub const fn mirrored(corner: Src, top: Src, left: Src) -> Self {
        Self {
            top_left: corner.oriented(Flip::None),
            top_right: corner.oriented(Flip::Horizontal),
            bottom_left: corner.oriented(Flip::Vertical),
            bottom_right: corner.oriented(Flip::Both),
            top: top.oriented(Flip::None),
            bottom: top.oriented(Flip::Vertical),
            left: left.oriented(Flip::None),
            right: left.oriented(Flip::Horizontal),
            center: None,
        }
    }

    #[must_use]
    pub const fn with_center(self, center: Piece) -> Self {
        Self {
            center: Some(center),
            ..self
        }
    }

    /// The pieces and where each lands in `dest`. Edges (and the
    /// center) with no room between their corners are omitted rather
    /// than folded over; corners always paint.
    pub fn pieces(self, dest: Rect) -> impl Iterator<Item = (Piece, Rect)> {
        let (tl, tr) = (self.top_left.size(), self.top_right.size());
        let (bl, br) = (self.bottom_left.size(), self.bottom_right.size());
        let corners = [
            (self.top_left, Rect::from_min_size(dest.min, tl)),
            (
                self.top_right,
                Rect::from_min_size(pos2(dest.max.x - tr.x, dest.min.y), tr),
            ),
            (
                self.bottom_left,
                Rect::from_min_size(pos2(dest.min.x, dest.max.y - bl.y), bl),
            ),
            (self.bottom_right, Rect::from_min_size(dest.max - br, br)),
        ];
        let (top, bottom) = (self.top.size().y, self.bottom.size().y);
        let (left, right) = (self.left.size().x, self.right.size().x);
        let edges = [
            (
                self.top,
                Rect::from_min_max(
                    pos2(dest.min.x + tl.x, dest.min.y),
                    pos2(dest.max.x - tr.x, dest.min.y + top),
                ),
            ),
            (
                self.bottom,
                Rect::from_min_max(
                    pos2(dest.min.x + bl.x, dest.max.y - bottom),
                    pos2(dest.max.x - br.x, dest.max.y),
                ),
            ),
            (
                self.left,
                Rect::from_min_max(
                    pos2(dest.min.x, dest.min.y + tl.y),
                    pos2(dest.min.x + left, dest.max.y - bl.y),
                ),
            ),
            (
                self.right,
                Rect::from_min_max(
                    pos2(dest.max.x - right, dest.min.y + tr.y),
                    pos2(dest.max.x, dest.max.y - br.y),
                ),
            ),
        ];
        let center = self.center.map(|piece| {
            (
                piece,
                Rect::from_min_max(
                    pos2(dest.min.x + left, dest.min.y + top),
                    pos2(dest.max.x - right, dest.max.y - bottom),
                ),
            )
        });
        corners.into_iter().chain(
            edges
                .into_iter()
                .chain(center)
                .filter(|(_, rect)| rect.width() > 0.0 && rect.height() > 0.0),
        )
    }

    pub fn paint(self, painter: &Painter, texture: &TextureHandle, dest: Rect, tint: Color32) {
        for (piece, rect) in self.pieces(dest) {
            blit(painter, texture, piece, rect, tint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: Vec2 = vec2(200.0, 100.0);
    const BORDER: Src = Src::new(0.0, 0.0, 120.0, 120.0);

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    #[test]
    fn uv_normalises_by_texture_size() {
        let uv = Src::new(50.0, 25.0, 100.0, 50.0).uv(SHEET);
        assert_eq!(uv.min, pos2(0.25, 0.25));
        assert_eq!(uv.max, pos2(0.75, 0.75));
    }

    #[test]
    fn flips_swap_the_axes_they_mirror() {
        let src = Src::new(0.0, 0.0, 100.0, 50.0);
        let plain = src.uv(SHEET);
        let horizontal = src.oriented(Flip::Horizontal).uv(SHEET);
        assert_eq!(horizontal.min, pos2(plain.max.x, plain.min.y));
        assert_eq!(horizontal.max, pos2(plain.min.x, plain.max.y));
        let vertical = src.oriented(Flip::Vertical).uv(SHEET);
        assert_eq!(vertical.min, pos2(plain.min.x, plain.max.y));
        let both = src.oriented(Flip::Both).uv(SHEET);
        assert_eq!(both.min, plain.max);
        assert_eq!(both.max, plain.min);
    }

    #[test]
    fn three_slice_caps_scale_with_the_destination_height() {
        let strip = ThreeSlice::new(Src::new(0.0, 0.0, 100.0, 20.0), 10.0);
        let pieces: Vec<_> = strip.pieces(rect(0.0, 0.0, 300.0, 40.0)).collect();
        assert_eq!(pieces.len(), 3);
        assert_eq!(
            pieces[0],
            (Src::new(0.0, 0.0, 10.0, 20.0), rect(0.0, 0.0, 20.0, 40.0))
        );
        assert_eq!(
            pieces[1],
            (
                Src::new(10.0, 0.0, 80.0, 20.0),
                rect(20.0, 0.0, 260.0, 40.0)
            )
        );
        assert_eq!(
            pieces[2],
            (
                Src::new(90.0, 0.0, 10.0, 20.0),
                rect(280.0, 0.0, 20.0, 40.0)
            )
        );
    }

    #[test]
    fn three_slice_narrower_than_its_caps_drops_the_middle() {
        let strip = ThreeSlice::new(Src::new(0.0, 0.0, 100.0, 20.0), 10.0);
        assert_eq!(strip.pieces(rect(0.0, 0.0, 15.0, 20.0)).count(), 2);
    }

    #[test]
    fn symmetric_frame_cuts_corners_and_bands_from_one_rectangle() {
        let frame = NinePatch::symmetric(BORDER, 40.0, 8.0);
        assert_eq!(frame.top_left.src, Src::new(0.0, 0.0, 40.0, 40.0));
        assert_eq!(frame.bottom_right.src, Src::new(80.0, 80.0, 40.0, 40.0));
        assert_eq!(frame.top.src, Src::new(40.0, 0.0, 40.0, 8.0));
        assert_eq!(frame.bottom.src, Src::new(40.0, 112.0, 40.0, 8.0));
        assert_eq!(frame.left.src, Src::new(0.0, 40.0, 8.0, 40.0));
        assert_eq!(frame.right.src, Src::new(112.0, 40.0, 8.0, 40.0));
        assert_eq!(frame.center, None);
    }

    #[test]
    fn mirrored_frame_flips_one_corner_and_two_strips_into_place() {
        let corner = Src::new(0.0, 35.0, 24.0, 24.0);
        let top = Src::new(680.0, 35.0, 60.0, 13.0);
        let left = Src::new(0.0, 100.0, 10.0, 500.0);
        let frame = NinePatch::mirrored(corner, top, left);
        assert_eq!(frame.top_right, corner.oriented(Flip::Horizontal));
        assert_eq!(frame.bottom_left, corner.oriented(Flip::Vertical));
        assert_eq!(frame.bottom_right, corner.oriented(Flip::Both));
        assert_eq!(frame.bottom, top.oriented(Flip::Vertical));
        assert_eq!(frame.right, left.oriented(Flip::Horizontal));
    }

    #[test]
    fn frame_keeps_corners_native_and_stretches_edges_between_them() {
        let frame = NinePatch::symmetric(BORDER, 40.0, 8.0);
        let pieces: Vec<_> = frame.pieces(rect(10.0, 20.0, 300.0, 200.0)).collect();
        assert_eq!(pieces.len(), 8);
        assert_eq!(pieces[0].1, rect(10.0, 20.0, 40.0, 40.0));
        assert_eq!(pieces[3].1, rect(270.0, 180.0, 40.0, 40.0));
        assert_eq!(pieces[4].1, rect(50.0, 20.0, 220.0, 8.0));
        assert_eq!(pieces[5].1, rect(50.0, 212.0, 220.0, 8.0));
        assert_eq!(pieces[6].1, rect(10.0, 60.0, 8.0, 120.0));
        assert_eq!(pieces[7].1, rect(302.0, 60.0, 8.0, 120.0));
    }

    #[test]
    fn frame_narrower_than_two_corners_drops_the_edges_rather_than_folding_them() {
        let frame = NinePatch::symmetric(BORDER, 40.0, 8.0);
        assert_eq!(frame.pieces(rect(0.0, 0.0, 60.0, 60.0)).count(), 4);
    }

    #[test]
    fn a_center_piece_fills_inside_the_bands() {
        let frame = NinePatch::symmetric(BORDER, 40.0, 8.0)
            .with_center(Src::new(40.0, 40.0, 40.0, 40.0).into());
        let (_, center) = frame.pieces(rect(0.0, 0.0, 300.0, 200.0)).last().unwrap();
        assert_eq!(center, rect(8.0, 8.0, 284.0, 184.0));
    }
}
