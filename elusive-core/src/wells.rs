//! Fraction tube → plate well mapping.
//!
//! The collector reports a 1-based `TubeNumber` and a `RackType`/`CollectionPattern`
//! pair; turning that into a physical `A1..H12` position is the whole job of this
//! module. It is deliberately pure so the mapping can be tested against a
//! hand-drawn grid without an instrument file (`design.md` §8).

use crate::model::Well;
use serde::{Deserialize, Serialize};

/// Physical geometry of a fraction rack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RackGeometry {
    pub rows: u8,
    pub cols: u8,
}

impl RackGeometry {
    pub const HEP96: RackGeometry = RackGeometry { rows: 8, cols: 12 };

    pub fn capacity(self) -> u32 {
        self.rows as u32 * self.cols as u32
    }

    /// Resolve from the `RackType` string in the fraction record.
    ///
    /// Only `HEP96` is confirmed against a real export (`design.md` §15). Other
    /// rack names return `None` so the caller can warn rather than silently
    /// placing fractions in the wrong wells.
    pub fn from_rack_type(rack_type: &str) -> Option<Self> {
        match rack_type.trim().to_ascii_uppercase().as_str() {
            "HEP96" | "HEP-96" | "96" | "PLATE96" => Some(RackGeometry::HEP96),
            _ => None,
        }
    }
}

/// The order in which the collector walks the rack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionPattern {
    /// Row-major with alternate rows reversed (boustrophedon). Confirmed for HEP96.
    Serpentine,
    /// Plain row-major: left-to-right on every row.
    Rows,
    /// Column-major: top-to-bottom down each column.
    Columns,
}

impl CollectionPattern {
    pub fn from_str_lenient(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "serpentine" | "boustrophedon" | "snake" => Some(CollectionPattern::Serpentine),
            "rows" | "row" | "rowmajor" | "row-major" => Some(CollectionPattern::Rows),
            "columns" | "column" | "columnmajor" | "column-major" => {
                Some(CollectionPattern::Columns)
            }
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CollectionPattern::Serpentine => "Serpentine",
            CollectionPattern::Rows => "Rows",
            CollectionPattern::Columns => "Columns",
        }
    }
}

/// Map a 1-based tube number to a zero-based plate position.
///
/// Returns `None` for tube 0 (the collector's "no tube" sentinel) or for a tube
/// number past the end of the rack — an out-of-range tube is a data problem worth
/// surfacing, not something to wrap around silently.
pub fn well_for_tube(
    tube: u32,
    geometry: RackGeometry,
    pattern: CollectionPattern,
) -> Option<Well> {
    if tube == 0 || geometry.rows == 0 || geometry.cols == 0 || tube > geometry.capacity() {
        return None;
    }
    let index = tube - 1;
    let (rows, cols) = (geometry.rows as u32, geometry.cols as u32);

    let (row, col) = match pattern {
        CollectionPattern::Rows => (index / cols, index % cols),
        CollectionPattern::Serpentine => {
            let row = index / cols;
            let pos = index % cols;
            // Even rows run left→right, odd rows right→left.
            let col = if row % 2 == 0 { pos } else { cols - 1 - pos };
            (row, col)
        }
        CollectionPattern::Columns => (index % rows, index / rows),
    };

    Some(Well::new(row as u8, col as u8))
}

/// Inverse of [`well_for_tube`], used when the UI needs "which tube is this well?".
pub fn tube_for_well(
    well: Well,
    geometry: RackGeometry,
    pattern: CollectionPattern,
) -> Option<u32> {
    if well.row >= geometry.rows || well.col >= geometry.cols {
        return None;
    }
    let (rows, cols) = (geometry.rows as u32, geometry.cols as u32);
    let (row, col) = (well.row as u32, well.col as u32);

    let index = match pattern {
        CollectionPattern::Rows => row * cols + col,
        CollectionPattern::Serpentine => {
            let pos = if row % 2 == 0 { col } else { cols - 1 - col };
            row * cols + pos
        }
        CollectionPattern::Columns => col * rows + row,
    };
    Some(index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed serpentine grid for the first two rows of an 8x12 plate.
    /// Row A runs A1..A12 (tubes 1..12), row B runs B12..B1 (tubes 13..24).
    #[test]
    fn serpentine_first_two_rows_match_hand_computed_grid() {
        let g = RackGeometry::HEP96;
        let p = CollectionPattern::Serpentine;

        let expected: [(u32, &str); 26] = [
            (1, "A1"),
            (2, "A2"),
            (3, "A3"),
            (4, "A4"),
            (5, "A5"),
            (6, "A6"),
            (7, "A7"),
            (8, "A8"),
            (9, "A9"),
            (10, "A10"),
            (11, "A11"),
            (12, "A12"),
            (13, "B12"),
            (14, "B11"),
            (15, "B10"),
            (16, "B9"),
            (17, "B8"),
            (18, "B7"),
            (19, "B6"),
            (20, "B5"),
            (21, "B4"),
            (22, "B3"),
            (23, "B2"),
            (24, "B1"),
            (25, "C1"),
            (26, "C2"),
        ];

        for (tube, label) in expected {
            let well = well_for_tube(tube, g, p).expect("tube in range");
            assert_eq!(well.label(), label, "tube {tube}");
        }
    }

    #[test]
    fn serpentine_last_well_is_h1() {
        // 96 tubes over 8 rows: row H (index 7) is odd, so it fills right→left and
        // the final tube lands in H1.
        let w = well_for_tube(96, RackGeometry::HEP96, CollectionPattern::Serpentine).unwrap();
        assert_eq!(w.label(), "H1");
    }

    #[test]
    fn row_major_ignores_alternation() {
        let w = well_for_tube(13, RackGeometry::HEP96, CollectionPattern::Rows).unwrap();
        assert_eq!(w.label(), "B1");
    }

    #[test]
    fn column_major_walks_down_first() {
        let g = RackGeometry::HEP96;
        let p = CollectionPattern::Columns;
        assert_eq!(well_for_tube(1, g, p).unwrap().label(), "A1");
        assert_eq!(well_for_tube(8, g, p).unwrap().label(), "H1");
        assert_eq!(well_for_tube(9, g, p).unwrap().label(), "A2");
    }

    #[test]
    fn out_of_range_tubes_are_rejected_not_wrapped() {
        let g = RackGeometry::HEP96;
        assert_eq!(well_for_tube(0, g, CollectionPattern::Serpentine), None);
        assert_eq!(well_for_tube(97, g, CollectionPattern::Serpentine), None);
    }

    #[test]
    fn tube_and_well_round_trip_for_every_pattern() {
        let g = RackGeometry::HEP96;
        for pattern in [
            CollectionPattern::Serpentine,
            CollectionPattern::Rows,
            CollectionPattern::Columns,
        ] {
            for tube in 1..=g.capacity() {
                let well = well_for_tube(tube, g, pattern).unwrap();
                assert_eq!(tube_for_well(well, g, pattern), Some(tube), "{pattern:?}");
            }
        }
    }

    #[test]
    fn serpentine_covers_every_well_exactly_once() {
        let g = RackGeometry::HEP96;
        let mut seen = std::collections::BTreeSet::new();
        for tube in 1..=g.capacity() {
            let well = well_for_tube(tube, g, CollectionPattern::Serpentine).unwrap();
            assert!(seen.insert(well), "well {well} produced twice");
        }
        assert_eq!(seen.len(), 96);
    }

    #[test]
    fn unknown_rack_types_do_not_guess_a_geometry() {
        assert_eq!(
            RackGeometry::from_rack_type("HEP96"),
            Some(RackGeometry::HEP96)
        );
        assert_eq!(RackGeometry::from_rack_type("TUBES18"), None);
    }
}
