// Originated in grim-vault, 2026-09-07.
//! A single row of arbitrary widgets — tab buttons, separators, drop
//! targets — that scrolls instead of wrapping when it outgrows the
//! pane. Art-free: the chevrons are painted triangles in the colours
//! the caller passes. While the row overflows, a chevron zone at
//! either end slides the content whenever the pointer rests on it,
//! keyed on pointer position so an item being dragged can reach an
//! off-screen tab; the mouse wheel scrolls it too. The offset lives
//! in egui temp memory under the strip's id — view state that costs
//! nothing to lose. A child selected from elsewhere is brought into
//! view through [`reveal_selected`].

use egui::scroll_area::ScrollBarVisibility;
use egui::{AsId, Color32, Id, Rect, Response, ScrollArea, Sense, Ui, vec2};

use super::chevron::{self, Side};
use crate::theme::Palette;

/// The chevron colours: resting, and lit while the pointer rests on
/// its zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripInk {
    pub chevron: Color32,
    pub lit: Color32,
}

impl StripInk {
    /// Weak text at rest, the accent while sliding.
    #[must_use]
    pub const fn from_palette(palette: &Palette) -> Self {
        Self {
            chevron: palette.text_weak,
            lit: palette.accent,
        }
    }
}

/// Where the strip stands: how far it is scrolled and how far it can
/// be. Both come from the last frame — the content is measured only
/// once it is laid out — so the chevron zones appear one frame after
/// the row first overflows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct StripState {
    offset: f32,
    max_offset: f32,
}

impl StripState {
    fn overflowing(self) -> bool {
        self.max_offset > 0.5
    }
}

/// A scrolling row, identified by a salt unique among its siblings.
pub struct ScrollStrip {
    salt: Id,
    ink: StripInk,
}

impl ScrollStrip {
    pub fn new(id_salt: impl AsId, ink: StripInk) -> Self {
        Self {
            salt: Id::new(id_salt),
            ink,
        }
    }

    /// Lays `add_contents` out left to right in one row that never
    /// wraps, scrolled behind end chevrons when it is wider than the
    /// space it has.
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
        let id = ui.id().with(self.salt);
        let before = ui
            .data(|data| data.get_temp::<StripState>(id))
            .unwrap_or_default();
        let zone = vec2(chevron::ZONE, ui.spacing().interact_size.y);
        let pointer = ui.ctx().pointer_latest_pos();
        let over = |rect: Option<Rect>| {
            rect.is_some_and(|rect| pointer.is_some_and(|pos| rect.contains(pos)))
        };
        ui.horizontal(|ui| {
            let left = before
                .overflowing()
                .then(|| ui.allocate_exact_size(zone, Sense::hover()).0);
            let reserve = if before.overflowing() {
                chevron::ZONE + ui.spacing().item_spacing.x
            } else {
                0.0
            };
            let output = ScrollArea::horizontal()
                .id_salt(id)
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                .animated(false)
                .auto_shrink([false, true])
                .max_width((ui.available_width() - reserve).max(0.0))
                .horizontal_scroll_offset(before.offset)
                .show(ui, add_contents);
            let right = before
                .overflowing()
                .then(|| ui.allocate_exact_size(zone, Sense::hover()).0);
            let shown = StripState {
                offset: output.state.offset.x,
                max_offset: max_offset(output.content_size.x, output.inner_rect.width()),
            };
            let hovered = if over(left) {
                Some(Side::Left)
            } else if over(right) {
                Some(Side::Right)
            } else {
                None
            };
            let next = StripState {
                offset: nudged(shown.offset, shown.max_offset, hovered, chevron::step(ui)),
                ..shown
            };
            if next != shown || next.overflowing() != before.overflowing() {
                ui.ctx().request_repaint();
            }
            let colour = |side: Side| {
                if hovered == Some(side) {
                    self.ink.lit
                } else {
                    self.ink.chevron
                }
            };
            if let Some(rect) = left
                && shown.offset > 0.5
            {
                chevron::paint(ui.painter(), rect, Side::Left, colour(Side::Left));
            }
            if let Some(rect) = right
                && shown.offset < shown.max_offset - 0.5
            {
                chevron::paint(ui.painter(), rect, Side::Right, colour(Side::Right));
            }
            ui.data_mut(|data| data.insert_temp(id, next));
            output.inner
        })
        .inner
    }
}

/// Scrolls `response` — the selected child — into view the frame the
/// selection changes, so a tab selected from elsewhere (a reset view,
/// a picker) is never left off-screen. Call it for the selected child
/// every frame with a `key` that identifies the selection; a click on
/// a visible tab changes the key without moving anything.
pub fn reveal_selected(ui: &Ui, key: impl AsId, response: &Response) {
    let slot = ui.id().with("scroll-strip-selection");
    let key = Id::new(key);
    let last = ui.data(|data| data.get_temp::<Id>(slot));
    if last != Some(key) {
        response.scroll_to_me(None);
        ui.data_mut(|data| data.insert_temp(slot, key));
    }
}

/// How far the content can scroll: its overhang past the viewport,
/// never negative.
fn max_offset(content_width: f32, inner_width: f32) -> f32 {
    (content_width - inner_width).max(0.0)
}

/// The offset after one frame on a chevron zone, held within the
/// scrollable range.
fn nudged(offset: f32, max_offset: f32, hovered: Option<Side>, step: f32) -> f32 {
    let moved = match hovered {
        Some(Side::Left) => offset - step,
        Some(Side::Right) => offset + step,
        None => offset,
    };
    moved.clamp(0.0, max_offset.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the arithmetic is a subtraction and a clamp — no drift to tolerate"
    )]
    fn the_overhang_is_the_scrollable_range_and_never_negative() {
        assert_eq!(max_offset(500.0, 300.0), 200.0);
        assert_eq!(max_offset(300.0, 300.0), 0.0);
        assert_eq!(max_offset(100.0, 300.0), 0.0);
        assert!(!StripState::default().overflowing());
        assert!(
            StripState {
                offset: 0.0,
                max_offset: 1.0
            }
            .overflowing()
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the arithmetic is a subtraction and a clamp — no drift to tolerate"
    )]
    fn a_hovered_chevron_slides_one_step_and_stops_at_either_end() {
        assert_eq!(nudged(100.0, 200.0, None, 10.0), 100.0);
        assert_eq!(nudged(100.0, 200.0, Some(Side::Right), 10.0), 110.0);
        assert_eq!(nudged(100.0, 200.0, Some(Side::Left), 10.0), 90.0);
        assert_eq!(nudged(195.0, 200.0, Some(Side::Right), 10.0), 200.0);
        assert_eq!(nudged(5.0, 200.0, Some(Side::Left), 10.0), 0.0);
        assert_eq!(nudged(50.0, 0.0, Some(Side::Right), 10.0), 0.0);
    }
}
