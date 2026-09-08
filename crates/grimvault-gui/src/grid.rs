//! Cell-grid geometry for the stash and inventory panes: how many
//! points a cell gets, where an item's cells land on screen, and which
//! cell a pointer is over. Pure — the panes paint with these numbers
//! and the drop logic snaps with them.

use egui::{Pos2, Rect, Vec2, vec2};
use grimvault_core::gamedata::Footprint;
use univault_engine::grid::CellRect;
use univault_engine::ids::GridPos;

/// Native cell size, matching the game's 32-pixel icon cells.
pub const CELL_PX: f32 = 32.0;
/// The smallest cell the panes shrink to before they overflow.
pub const MIN_CELL_PX: f32 = 20.0;

/// A grid placed on screen: `cols` × `rows` cells of `cell` points
/// from `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridGeometry {
    origin: Pos2,
    cell: f32,
    cols: i32,
    rows: i32,
}

/// The cells `n` spans, in points at `cell` per cell. Grid dimensions
/// are small integers that `f32` holds exactly.
#[expect(
    clippy::cast_precision_loss,
    reason = "cell counts are bounded far below f32's exact-integer range"
)]
fn span(n: i32, cell: f32) -> f32 {
    n as f32 * cell
}

/// The footprint at the game's native cell size, as the tooltip shows
/// an item.
#[must_use]
pub fn native_size(footprint: Footprint) -> Vec2 {
    vec2(
        span(footprint.width, CELL_PX),
        span(footprint.height, CELL_PX),
    )
}

impl GridGeometry {
    /// The largest cell in `MIN_CELL_PX..=CELL_PX` that fits the grid
    /// into `available`, floored to whole points so cell edges stay
    /// crisp.
    #[must_use]
    pub fn fit(origin: Pos2, available: Vec2, cols: i32, rows: i32) -> Self {
        let by_width = available.x / span(cols.max(1), 1.0);
        let by_height = available.y / span(rows.max(1), 1.0);
        let cell = by_width.min(by_height).floor().clamp(MIN_CELL_PX, CELL_PX);
        Self {
            origin,
            cell,
            cols,
            rows,
        }
    }

    #[must_use]
    pub fn cols(&self) -> i32 {
        self.cols
    }

    #[must_use]
    pub fn rows(&self) -> i32 {
        self.rows
    }

    #[must_use]
    pub fn size(&self) -> Vec2 {
        vec2(span(self.cols, self.cell), span(self.rows, self.cell))
    }

    #[must_use]
    pub fn rect(&self) -> Rect {
        Rect::from_min_size(self.origin, self.size())
    }

    /// The screen rectangle of a run of cells.
    /// Points per cell.
    #[must_use]
    pub fn cell(&self) -> f32 {
        self.cell
    }

    #[must_use]
    pub fn cell_rect(&self, cells: CellRect) -> Rect {
        Rect::from_min_size(
            self.origin + vec2(span(cells.x, self.cell), span(cells.y, self.cell)),
            vec2(span(cells.width, self.cell), span(cells.height, self.cell)),
        )
    }

    /// The cell under a point, `None` outside the grid.
    #[must_use]
    pub fn cell_at(&self, point: Pos2) -> Option<GridPos> {
        if !self.rect().contains(point) {
            return None;
        }
        let relative = point - self.origin;
        Some(GridPos {
            x: containing_cell(relative.x / self.cell).clamp(0, self.cols - 1),
            y: containing_cell(relative.y / self.cell).clamp(0, self.rows - 1),
        })
    }

    /// The cell a dragged footprint whose top-left corner is at
    /// `top_left` snaps to: rounded to the nearest cell and clamped so
    /// the footprint stays inside the grid.
    #[must_use]
    pub fn snap(&self, top_left: Pos2, footprint: Footprint) -> GridPos {
        let relative = top_left - self.origin;
        let clamp = |value: f32, cells: i32, extent: i32| {
            whole_cells(value / self.cell).clamp(0, (cells - extent).max(0))
        };
        GridPos {
            x: clamp(relative.x, self.cols, footprint.width),
            y: clamp(relative.y, self.rows, footprint.height),
        }
    }
}

/// Rounds a cell coordinate — the nearest cell edge, for snapping a
/// dragged footprint; values are small, so the cast is exact.
#[expect(
    clippy::cast_possible_truncation,
    reason = "cell coordinates are bounded by on-screen grid sizes"
)]
fn whole_cells(value: f32) -> i32 {
    value.round() as i32
}

/// The cell a coordinate falls inside — floored, for hit-testing.
#[expect(
    clippy::cast_possible_truncation,
    reason = "cell coordinates are bounded by on-screen grid sizes"
)]
fn containing_cell(value: f32) -> i32 {
    value.floor() as i32
}

/// The cells an item at `pos` with `footprint` covers.
#[must_use]
pub fn cells_of(pos: GridPos, footprint: Footprint) -> CellRect {
    CellRect {
        x: pos.x,
        y: pos.y,
        width: footprint.width,
        height: footprint.height,
    }
}

/// Whether a footprint is the record's or a stand-in for one the
/// database could not supply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FootprintSource {
    Known,
    Assumed,
}

/// A cell count as a length, for sizing a footprint in points.
#[expect(
    clippy::cast_precision_loss,
    reason = "footprints are a handful of cells"
)]
#[must_use]
pub fn cells(n: i32) -> f32 {
    n as f32
}

/// The footprint to paint with: the record's, or 1×1 when unknown so
/// the item is still visible (and marked).
#[must_use]
pub fn footprint_or_unit(footprint: Option<Footprint>) -> (Footprint, FootprintSource) {
    footprint.map_or(
        (
            Footprint {
                width: 1,
                height: 1,
            },
            FootprintSource::Assumed,
        ),
        |footprint| (footprint, FootprintSource::Known),
    )
}

/// Index of the occupant covering `cell`, if any.
#[must_use]
pub fn occupant_at(occupants: &[CellRect], cell: GridPos) -> Option<usize> {
    occupants.iter().position(|rect| {
        cell.x >= rect.x
            && cell.x < rect.x + rect.width
            && cell.y >= rect.y
            && cell.y < rect.y + rect.height
    })
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    fn at(x: i32, y: i32) -> GridPos {
        GridPos { x, y }
    }

    fn footprint(width: i32, height: i32) -> Footprint {
        Footprint { width, height }
    }

    #[test]
    fn fit_picks_the_limiting_axis_within_bounds() {
        let wide = GridGeometry::fit(Pos2::ZERO, vec2(1000.0, 190.0), 10, 19);
        assert_eq!(wide.size(), vec2(200.0, 380.0));
        let roomy = GridGeometry::fit(Pos2::ZERO, vec2(1000.0, 1000.0), 10, 19);
        assert_eq!(roomy.size(), vec2(10.0 * CELL_PX, 19.0 * CELL_PX));
        let cramped = GridGeometry::fit(Pos2::ZERO, vec2(50.0, 50.0), 10, 19);
        assert_eq!(cramped.size(), vec2(10.0 * MIN_CELL_PX, 19.0 * MIN_CELL_PX));
        let fractional = GridGeometry::fit(Pos2::ZERO, vec2(275.0, 1000.0), 10, 19);
        assert_eq!(fractional.size(), vec2(270.0, 513.0));
    }

    #[test]
    fn cell_rects_and_hit_tests_agree() {
        let grid = GridGeometry::fit(pos2(10.0, 20.0), vec2(320.0, 608.0), 10, 19);
        assert_eq!(grid.size(), vec2(320.0, 608.0));
        let rect = grid.cell_rect(cells_of(at(2, 3), footprint(1, 2)));
        assert_eq!(rect.min, pos2(74.0, 116.0));
        assert_eq!(rect.size(), vec2(32.0, 64.0));
        assert_eq!(grid.cell_at(pos2(75.0, 117.0)), Some(at(2, 3)));
        assert_eq!(grid.cell_at(pos2(105.9, 179.9)), Some(at(2, 4)));
        assert_eq!(grid.cell_at(pos2(5.0, 5.0)), None);
        assert_eq!(grid.cell_at(pos2(329.9, 627.9)), Some(at(9, 18)));
    }

    #[test]
    fn snap_rounds_and_keeps_the_footprint_inside() {
        let grid = GridGeometry::fit(Pos2::ZERO, vec2(320.0, 608.0), 10, 19);
        assert_eq!(grid.snap(pos2(47.0, 20.0), footprint(1, 2)), at(1, 1));
        assert_eq!(grid.snap(pos2(49.0, 20.0), footprint(1, 2)), at(2, 1));
        assert_eq!(grid.snap(pos2(-40.0, -40.0), footprint(2, 3)), at(0, 0));
        assert_eq!(grid.snap(pos2(900.0, 900.0), footprint(2, 3)), at(8, 16));
        assert_eq!(grid.snap(pos2(0.0, 0.0), footprint(20, 30)), at(0, 0));
    }

    #[test]
    fn occupants_are_found_by_covered_cell() {
        let occupants = [
            cells_of(at(1, 6), footprint(1, 2)),
            cells_of(at(3, 0), footprint(2, 3)),
        ];
        assert_eq!(occupant_at(&occupants, at(1, 7)), Some(0));
        assert_eq!(occupant_at(&occupants, at(4, 2)), Some(1));
        assert_eq!(occupant_at(&occupants, at(1, 8)), None);
        assert_eq!(occupant_at(&occupants, at(5, 0)), None);
    }

    #[test]
    fn unknown_footprints_paint_as_a_marked_unit() {
        assert_eq!(
            footprint_or_unit(None),
            (footprint(1, 1), FootprintSource::Assumed)
        );
        assert_eq!(
            footprint_or_unit(Some(footprint(2, 4))),
            (footprint(2, 4), FootprintSource::Known)
        );
    }
}
