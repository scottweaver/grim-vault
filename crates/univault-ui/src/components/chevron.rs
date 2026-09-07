// Originated in grim-vault, 2026-09-07 — extracted from the vendored tabbed_panel.rs so both strips share it.
//! The scroll chevrons an overflowing strip shows at either end: the
//! hover zone reserved for each, the triangle drawn inside it, and
//! how fast a hovered zone slides the content. Scrolling keys on
//! pointer position rather than widget hover so a drag in progress
//! scrolls too and can reach an off-screen tab.

use egui::{Color32, Painter, Rect, Shape, Stroke, Ui, pos2};

/// Width of the hover zone reserved at each end of a scrolling strip.
pub(super) const ZONE: f32 = 22.0;
const WIDTH: f32 = 9.0;
const HEIGHT: f32 = 14.0;
const SCROLL_SPEED: f32 = 280.0;

/// Which end of the strip a chevron scrolls toward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
}

/// How far a hovered chevron slides the content this frame.
pub(super) fn step(ui: &Ui) -> f32 {
    SCROLL_SPEED * ui.input(|input| input.stable_dt).min(0.1)
}

/// One scroll chevron: a triangle pointing off-strip, centred in
/// `zone`.
pub(super) fn paint(painter: &Painter, zone: Rect, side: Side, color: Color32) {
    let center = zone.center();
    let (near, far) = match side {
        Side::Left => (center.x + WIDTH / 2.0, center.x - WIDTH / 2.0),
        Side::Right => (center.x - WIDTH / 2.0, center.x + WIDTH / 2.0),
    };
    painter.add(Shape::convex_polygon(
        vec![
            pos2(near, center.y - HEIGHT / 2.0),
            pos2(far, center.y),
            pos2(near, center.y + HEIGHT / 2.0),
        ],
        color,
        Stroke::NONE,
    ));
}
