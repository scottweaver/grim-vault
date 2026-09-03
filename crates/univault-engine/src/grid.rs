// Vendored from tq-univault crates/univault-core/src/grid.rs @ 36e7774; adapted per docs/engine-extraction.md.
//! Grid placement for item containers — a port of `TQVaultAE`'s
//! `ItemMovementService.FindOpenCells` (MIT): search each column top
//! to bottom and take the first rectangle of free cells that fits.

use crate::ids::GridPos;

/// An occupied rectangle of cells (position plus footprint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl CellRect {
    #[must_use]
    pub fn overlaps(&self, other: &CellRect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// Whether `candidate` lies fully inside the grid and overlaps no
/// occupied rectangle — the exact-position check behind drop
/// previews and placements.
#[must_use]
pub fn fits(occupied: &[CellRect], candidate: CellRect, sack_width: i32, sack_height: i32) -> bool {
    candidate.x >= 0
        && candidate.y >= 0
        && candidate.width > 0
        && candidate.height > 0
        && candidate.x + candidate.width <= sack_width
        && candidate.y + candidate.height <= sack_height
        && !occupied.iter().any(|rect| rect.overlaps(&candidate))
}

/// First free spot for an `item_width` × `item_height` footprint in a
/// `sack_width` × `sack_height` grid, scanning columns left to right
/// and each column top to bottom. `None` when nothing fits.
#[must_use]
pub fn find_open_cells(
    occupied: &[CellRect],
    item_width: i32,
    item_height: i32,
    sack_width: i32,
    sack_height: i32,
) -> Option<GridPos> {
    if item_width <= 0 || item_height <= 0 || item_width > sack_width {
        return None;
    }
    for x in 0..=(sack_width - item_width) {
        let mut y = 0;
        while y + item_height <= sack_height {
            let candidate = CellRect {
                x,
                y,
                width: item_width,
                height: item_height,
            };
            match occupied.iter().find(|rect| rect.overlaps(&candidate)) {
                None => return Some(GridPos { x, y }),
                // Skip past the blocking item, like TQVaultAE.
                Some(blocking) => y = (blocking.y + blocking.height).max(y + 1),
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> CellRect {
        CellRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn empty_grid_places_at_origin() {
        assert_eq!(
            find_open_cells(&[], 2, 3, 18, 20),
            Some(GridPos { x: 0, y: 0 })
        );
    }

    #[test]
    fn fills_columns_top_to_bottom() {
        let occupied = [rect(0, 0, 1, 2)];
        assert_eq!(
            find_open_cells(&occupied, 1, 1, 4, 4),
            Some(GridPos { x: 0, y: 2 })
        );
    }

    #[test]
    fn moves_to_next_column_when_one_is_full() {
        let occupied = [rect(0, 0, 1, 4)];
        assert_eq!(
            find_open_cells(&occupied, 1, 2, 4, 4),
            Some(GridPos { x: 1, y: 0 })
        );
    }

    #[test]
    fn full_grid_returns_none() {
        let occupied = [rect(0, 0, 4, 4)];
        assert_eq!(find_open_cells(&occupied, 1, 1, 4, 4), None);
    }

    #[test]
    fn item_wider_than_the_sack_never_fits() {
        assert_eq!(find_open_cells(&[], 3, 1, 2, 10), None);
    }

    #[test]
    fn footprint_must_fit_before_the_grid_edge() {
        let occupied = [rect(0, 0, 2, 3)];
        // A 2x3 item can only start at y<=1 in a 4-tall grid; column 0
        // is blocked until y=3, so it must move right.
        assert_eq!(
            find_open_cells(&occupied, 2, 3, 4, 4),
            Some(GridPos { x: 2, y: 0 })
        );
    }

    #[test]
    fn fits_checks_bounds_and_overlap() {
        let occupied = [rect(0, 0, 2, 2)];
        assert!(fits(&occupied, rect(2, 0, 1, 1), 4, 4));
        assert!(!fits(&occupied, rect(1, 1, 1, 1), 4, 4));
        assert!(!fits(&occupied, rect(3, 3, 2, 1), 4, 4));
        assert!(!fits(&occupied, rect(-1, 0, 1, 1), 4, 4));
    }
}
