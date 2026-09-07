// Vendored from tq-univault crates/univault-gui/src/components/tabbed_panel.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! A tabbed panel: plates on a strip above a mitered rail frame
//! around a flat interior, painted from one sheet the app supplies
//! (the layout is documented on [`TabbedPanelArt`]). Plates are
//! 3-sliced so any label width fits; the active plate's slice carries
//! wings into the rail band, reproducing the art's curl where the
//! open tab merges through the rail.
//!
//! When the strip outgrows the pane it scrolls instead of clipping
//! tabs away: triangular chevrons appear at either end and slide the
//! plates while the pointer rests on them — position-based, so an
//! item dragged onto a chevron scrolls too and can reach an
//! off-screen tab. The scroll offset lives in egui temp memory under
//! the panel's id; selecting a tab scrolls it into view.

use egui::{Color32, CursorIcon, Margin, Rect, Sense, TextureHandle, pos2, vec2};

use super::chevron::{self, Side};
use crate::slice::{NinePatch, ThreeSlice};

const TAB_GAP: f32 = 12.0;
const TAB_PAD: f32 = 18.0;
const TAB_MIN_W: f32 = 44.0;

/// The panel's sheet layout and the colours that go with it. Every
/// measurement is in source pixels, painted at native scale.
///
/// # Art layout
///
/// - `inactive`: one closed plate, 3-sliced by its caps. Its height
///   is the plate height above the rail.
/// - `active`: the open plate, cut through the rail band so the merge
///   comes along — its height is the plate height plus the rail's top
///   thickness. It is painted `active_wing` px wider than the plate
///   on each side so the merge's curl reaches past the plate's edges;
///   keep the wings clear of neighbouring art that LINEAR sampling
///   would bleed in.
/// - `rail`: the mitered frame under the strip, with no center
///   ([`NinePatch::mirrored`] from one corner and two clean strips is
///   the usual way to draw it). Its top edge's thickness is the band
///   the open plate crosses; its corners' width is the quiet zone the
///   strip starts past, where the rail's inner line stops short of
///   the edge and a wing crossing it would read as a stray line.
/// - `interior`: the flat fill inside the rail, under the content.
/// - `ink`: label colours on the plates and the scroll chevrons.
/// - `margin`: the content inset — past the strip and the rails, with
///   breathing room inside the interior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabbedPanelArt {
    pub inactive: ThreeSlice,
    pub active: ThreeSlice,
    pub active_wing: f32,
    pub rail: NinePatch,
    pub interior: Color32,
    pub ink: LabelInk,
    pub margin: Margin,
}

impl TabbedPanelArt {
    fn plate_height(&self) -> f32 {
        self.inactive.src.h
    }

    fn rail_top(&self) -> f32 {
        self.rail.top.size().y
    }

    fn left_corner(&self) -> f32 {
        self.rail.top_left.size().x
    }

    fn right_corner(&self) -> f32 {
        self.rail.top_right.size().x
    }
}

/// Label colours by plate state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelInk {
    pub inactive: Color32,
    pub active: Color32,
    pub disabled: Color32,
}

impl LabelInk {
    /// A chevron's colour: the active ink while the pointer rests on
    /// its zone, the inactive ink otherwise.
    fn chevron(self, lit: bool) -> Color32 {
        if lit { self.active } else { self.inactive }
    }
}

/// One plate on the strip. A disabled plate renders dim, reports
/// neither clicks nor hover, and offers `disabled_hint` as its
/// tooltip.
pub struct Tab {
    title: String,
    enabled: bool,
    disabled_hint: Option<String>,
}

impl Tab {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            enabled: true,
            disabled_hint: None,
        }
    }

    pub fn disabled(title: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            enabled: false,
            disabled_hint: Some(hint.into()),
        }
    }
}

/// What [`TabbedPanel::show`] reports back: the clicked tab, the
/// enabled tab under the pointer (for callers that switch tabs
/// mid-drag) — the caller owns the selection — and the content's
/// result.
pub struct TabbedPanelResponse<R> {
    pub clicked: Option<usize>,
    pub hovered: Option<usize>,
    pub inner: R,
}

/// The uploaded panel sheet and its layout. The handle is an `Arc`,
/// so cloning is cheap.
#[derive(Clone)]
pub struct TabbedPanel {
    texture: TextureHandle,
    art: TabbedPanelArt,
}

impl TabbedPanel {
    #[must_use]
    pub const fn new(texture: TextureHandle, art: TabbedPanelArt) -> Self {
        Self { texture, art }
    }

    #[must_use]
    pub const fn art(&self) -> &TabbedPanelArt {
        &self.art
    }

    /// Lays `content` out inside the art's margin, then dresses the
    /// allocated rect: flat interior, rail frame, one plate per tab
    /// with `selected` drawn open through the rail. A strip wider
    /// than the pane scrolls behind end chevrons rather than clipping
    /// tabs out of reach. Reports clicks and hover on enabled plates;
    /// the caller applies the selection change.
    pub fn show<R>(
        &self,
        ui: &mut egui::Ui,
        tabs: &[Tab],
        selected: usize,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> TabbedPanelResponse<R> {
        let art = &self.art;
        let fill_slot = ui.painter().add(egui::Shape::Noop);
        let inner = egui::Frame::new()
            .inner_margin(art.margin)
            .show(ui, content);
        let outer = inner.response.rect;
        let frame = Rect::from_min_max(
            pos2(outer.min.x, outer.min.y + art.plate_height()),
            outer.max,
        );
        ui.painter().set(
            fill_slot,
            egui::Shape::rect_filled(frame, 0.0, art.interior),
        );

        let geo = strip_geometry(ui, art, tabs, outer);
        let pointer = ui.ctx().pointer_latest_pos();
        let zones = geo.scrolling.then_some((geo.left_zone, geo.right_zone));
        let offset = strip_offset(
            ui,
            inner.response.id,
            selected,
            &geo.plates,
            zones,
            geo.span,
            geo.max_offset,
        );

        let visible = geo.visible;
        let clip = if geo.scrolling { visible } else { outer };
        let strip = ui.painter().with_clip_rect(clip.intersect(ui.clip_rect()));
        let tab_rects: Vec<Rect> = geo
            .plates
            .rects
            .iter()
            .map(|rect| rect.translate(vec2(geo.strip_left - offset, outer.min.y)))
            .collect();
        for (index, rect) in tab_rects.iter().enumerate() {
            if index != selected {
                art.inactive
                    .paint(&strip, &self.texture, *rect, Color32::WHITE);
            }
        }
        art.rail
            .paint(ui.painter(), &self.texture, frame, Color32::WHITE);
        self.paint_plates(ui, &strip, tabs, &tab_rects, selected);
        let PlateHits { clicked, hovered } =
            plate_hits(ui, inner.response.id, tabs, &tab_rects, visible);
        if geo.scrolling {
            let over = |zone: Rect| pointer.is_some_and(|pos| zone.contains(pos));
            if offset > 0.5 {
                chevron::paint(
                    ui.painter(),
                    geo.left_zone,
                    Side::Left,
                    art.ink.chevron(over(geo.left_zone)),
                );
            }
            if offset < geo.max_offset - 0.5 {
                chevron::paint(
                    ui.painter(),
                    geo.right_zone,
                    Side::Right,
                    art.ink.chevron(over(geo.right_zone)),
                );
            }
        }
        TabbedPanelResponse {
            clicked,
            hovered,
            inner: inner.inner,
        }
    }

    /// The open plate drawn through the rail, then every label in
    /// its state's ink.
    fn paint_plates(
        &self,
        ui: &egui::Ui,
        strip: &egui::Painter,
        tabs: &[Tab],
        rects: &[Rect],
        selected: usize,
    ) {
        let art = &self.art;
        let font = egui::TextStyle::Button.resolve(ui.style());
        for ((index, rect), tab) in rects.iter().enumerate().zip(tabs) {
            if index == selected {
                let open = Rect::from_min_max(
                    pos2(rect.min.x - art.active_wing, rect.min.y),
                    pos2(rect.max.x + art.active_wing, rect.max.y + art.rail_top()),
                );
                art.active.paint(strip, &self.texture, open, Color32::WHITE);
            }
            let ink = if !tab.enabled {
                art.ink.disabled
            } else if index == selected {
                art.ink.active
            } else {
                art.ink.inactive
            };
            strip.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &tab.title,
                font.clone(),
                ink,
            );
        }
    }
}

/// What the plates reported this frame.
struct PlateHits {
    clicked: Option<usize>,
    hovered: Option<usize>,
}

/// One hit area per plate that reaches the visible span. Enabled
/// plates report clicks and pointer-position hover; disabled ones
/// only offer their hint.
fn plate_hits(
    ui: &egui::Ui,
    base: egui::Id,
    tabs: &[Tab],
    rects: &[Rect],
    visible: Rect,
) -> PlateHits {
    let pointer = ui.ctx().pointer_latest_pos();
    let mut hits = PlateHits {
        clicked: None,
        hovered: None,
    };
    for ((index, rect), tab) in rects.iter().enumerate().zip(tabs) {
        let hit = rect.intersect(visible);
        if hit.width() <= 0.0 {
            continue;
        }
        let response = ui.interact(hit, base.with(("tabbed-panel", index)), Sense::click());
        if tab.enabled {
            let response = response.on_hover_cursor(CursorIcon::PointingHand);
            if response.clicked() {
                hits.clicked = Some(index);
            }
            if pointer.is_some_and(|pos| hit.contains(pos)) {
                hits.hovered = Some(index);
            }
        } else if let Some(hint) = &tab.disabled_hint {
            response.on_hover_text(hint.clone());
        }
    }
    hits
}

/// The strip's plate rects, relative to its own left edge, and the
/// total width they occupy.
struct PlateLayout {
    rects: Vec<Rect>,
    total_width: f32,
}

/// One plate rect per tab, laid left-to-right in a single row, each
/// sized to its label. Positions are relative — the caller places
/// the strip past the corner block and applies the scroll offset.
fn plate_layout(ui: &egui::Ui, tabs: &[Tab], plate_height: f32) -> PlateLayout {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut x = 0.0;
    let rects: Vec<Rect> = tabs
        .iter()
        .map(|tab| {
            let label =
                ui.painter()
                    .layout_no_wrap(tab.title.clone(), font.clone(), Color32::WHITE);
            let width = (label.rect.width() + 2.0 * TAB_PAD).max(TAB_MIN_W);
            let rect = Rect::from_min_size(pos2(x, 0.0), vec2(width, plate_height));
            x += width + TAB_GAP;
            rect
        })
        .collect();
    PlateLayout {
        rects,
        total_width: (x - TAB_GAP).max(0.0),
    }
}

/// The dressed strip's working measurements: the plates, whether
/// they outgrow the pane, the visible span between the corner
/// blocks (shrunk behind chevron zones when they do), where that
/// span starts, and the chevron hover zones at either end.
struct StripGeometry {
    plates: PlateLayout,
    scrolling: bool,
    span: f32,
    max_offset: f32,
    strip_left: f32,
    visible: Rect,
    left_zone: Rect,
    right_zone: Rect,
}

fn strip_geometry(ui: &egui::Ui, art: &TabbedPanelArt, tabs: &[Tab], outer: Rect) -> StripGeometry {
    let plates = plate_layout(ui, tabs, art.plate_height());
    let full_span = outer.width() - art.left_corner() - art.right_corner();
    let scrolling = plates.total_width > full_span;
    let reserve = if scrolling { chevron::ZONE } else { 0.0 };
    let span = (full_span - 2.0 * reserve).max(TAB_MIN_W);
    let max_offset = (plates.total_width - span).max(0.0);
    let band = |min_x: f32| {
        Rect::from_min_size(
            pos2(min_x, outer.min.y),
            vec2(chevron::ZONE, art.plate_height()),
        )
    };
    let strip_left = outer.min.x + art.left_corner() + reserve;
    StripGeometry {
        scrolling,
        span,
        max_offset,
        strip_left,
        visible: Rect::from_min_max(
            pos2(strip_left, outer.min.y),
            pos2(strip_left + span, outer.max.y),
        ),
        left_zone: band(outer.min.x + art.left_corner()),
        right_zone: band(outer.max.x - art.right_corner() - chevron::ZONE),
        plates,
    }
}

/// The strip's scroll offset for this frame: persisted in temp
/// memory under the panel's id, scrolled to reveal a newly selected
/// plate, nudged while the pointer rests on a chevron zone (passed
/// only when the strip overflows), and clamped to the scrollable
/// range.
fn strip_offset(
    ui: &egui::Ui,
    base: egui::Id,
    selected: usize,
    plates: &PlateLayout,
    zones: Option<(Rect, Rect)>,
    span: f32,
    max_offset: f32,
) -> f32 {
    let state_id = base.with("tab-strip-scroll");
    let (stored, last_selected) = ui
        .ctx()
        .data(|data| data.get_temp::<(f32, usize)>(state_id))
        .unwrap_or((0.0, selected));
    let mut offset = stored;
    if selected != last_selected
        && let Some(plate) = plates.rects.get(selected)
    {
        offset = reveal_offset(offset, plate.min.x, plate.max.x, span);
    }
    if let Some((left_zone, right_zone)) = zones {
        let pointer = ui.ctx().pointer_latest_pos();
        let step = chevron::step(ui);
        let over = |zone: Rect| pointer.is_some_and(|pos| zone.contains(pos));
        if over(left_zone) && offset > 0.0 {
            offset -= step;
            ui.ctx().request_repaint();
        }
        if over(right_zone) && offset < max_offset {
            offset += step;
            ui.ctx().request_repaint();
        }
    }
    let offset = offset.clamp(0.0, max_offset);
    ui.ctx()
        .data_mut(|data| data.insert_temp(state_id, (offset, selected)));
    offset
}

/// The smallest scroll adjustment that brings a newly selected
/// plate — spanning `min..max` in strip coordinates — fully into a
/// window `span` wide starting at `offset`.
fn reveal_offset(offset: f32, min: f32, max: f32, span: f32) -> f32 {
    if min < offset {
        min
    } else if max > offset + span {
        max - span
    } else {
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::reveal_offset;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "reveal_offset returns its inputs unchanged — no arithmetic drift"
    )]
    fn reveal_scrolls_only_far_enough_to_show_the_plate() {
        assert_eq!(reveal_offset(100.0, 120.0, 200.0, 300.0), 100.0);
        assert_eq!(reveal_offset(150.0, 120.0, 200.0, 300.0), 120.0);
        assert_eq!(reveal_offset(0.0, 500.0, 620.0, 300.0), 320.0);
        assert_eq!(reveal_offset(120.0, 120.0, 420.0, 300.0), 120.0);
    }
}
