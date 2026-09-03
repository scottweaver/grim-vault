// Vendored from tq-univault crates/univault-gui/src/components/gilded_border.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! A frame painted nine-patch style over the edges of a rect —
//! corners at native scale, edges stretched between them — and
//! nothing else: the interior is left untouched, so whatever lies
//! under the frame shows through.
//!
//! # Art layout
//!
//! The texture holds the frame as one closed image: the four corners
//! in its four corners, straight edge runs between them, a
//! transparent interior. [`NinePatch::symmetric`] over the whole
//! texture describes that layout from two numbers — the corner size
//! (large enough to cover any corner ornament and its transition
//! back into the straight line) and the edge band thickness (the
//! line plus whatever it wanders across). tq-univault's hand-drawn
//! double gold line with hatched corner brackets is 40 px corners
//! over an 8 px band, with a 14 px content margin keeping widgets
//! clear of the line.

use egui::{Color32, Margin, Rect, TextureHandle};

use crate::slice::NinePatch;

/// The uploaded frame art and its layout. The handle is an `Arc`,
/// so cloning is cheap.
#[derive(Clone)]
pub struct GildedBorder {
    texture: TextureHandle,
    frame: NinePatch,
    margin: Margin,
}

impl GildedBorder {
    /// `frame` slices `texture`; `margin` is the content inset
    /// [`Self::show`] lays out inside.
    #[must_use]
    pub const fn new(texture: TextureHandle, frame: NinePatch, margin: Margin) -> Self {
        Self {
            texture,
            frame,
            margin,
        }
    }

    #[must_use]
    pub const fn margin(&self) -> Margin {
        self.margin
    }

    /// Lays `content` out inside the margin, then paints the frame
    /// over the allocated rect. The background is whatever was
    /// already there.
    pub fn show<R>(
        &self,
        ui: &mut egui::Ui,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> egui::InnerResponse<R> {
        let inner = egui::Frame::new()
            .inner_margin(self.margin)
            .show(ui, content);
        self.paint(ui.painter(), inner.response.rect);
        inner
    }

    /// Paints the frame along the edges of `rect`; corners keep the
    /// art's native size, edges stretch. Rects narrower than two
    /// corners drop the edge pieces rather than fold them over.
    pub fn paint(&self, painter: &egui::Painter, rect: Rect) {
        self.frame
            .paint(painter, &self.texture, rect, Color32::WHITE);
    }
}
