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
            let col = if row.is_multiple_of(2) {
                pos
            } else {
                cols - 1 - pos
            };
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

/// Split a collection-ordered well list into contiguous runs, each returned as
/// its inclusive `(first, last)` pair. A singleton run has `first == last`.
///
/// A run is only formed from wells in the same plate row whose columns step by a
/// constant ±1, so `D5, D6, D7, D8` collapses and so does the descending
/// `B12, B11, B10` that serpentine collection produces on odd rows. Nothing is
/// collapsed across a row change: under serpentine, `A11, A12, B12, B11` is four
/// consecutive tubes but no sane range notation covers it.
///
/// Runs shorter than three wells are left expanded — `D5–D6` is no shorter than
/// `D5, D6` and reads as though something were being hidden between the two.
pub fn well_runs(wells: &[Well]) -> Vec<(Well, Well)> {
    let mut runs: Vec<(Well, Well)> = Vec::new();
    let mut i = 0;
    while i < wells.len() {
        let start = wells[i];
        let mut end = start;
        let mut step: Option<i32> = None;
        let mut j = i + 1;
        while let Some(&next) = wells.get(j) {
            if next.row != end.row {
                break;
            }
            let delta = next.col as i32 - end.col as i32;
            if delta != 1 && delta != -1 {
                break;
            }
            if *step.get_or_insert(delta) != delta {
                break;
            }
            end = next;
            j += 1;
        }
        if j - i >= 3 {
            runs.push((start, end));
            i = j;
        } else {
            runs.push((start, start));
            i += 1;
        }
    }
    runs
}

/// Human-readable well list for a table cell, e.g. `D5–D8` or `A11, A12, B12`.
///
/// `max_entries` caps how many comma-separated entries are rendered; the rest are
/// summarised as `+N more` so one wide peak cannot blow the column out. Pass
/// `None` for the complete list. An empty input yields an empty string — the
/// caller decides how to say "nothing", since "no fraction overlaps" and "this
/// format carries no fractions" are different answers.
pub fn format_well_list(wells: &[Well], max_entries: Option<usize>) -> String {
    let runs = well_runs(wells);
    let shown = max_entries.unwrap_or(runs.len()).min(runs.len());
    let mut out = runs[..shown]
        .iter()
        .map(|&(a, b)| {
            if a == b {
                a.label()
            } else {
                // En dash for a span, matching the numeric ranges elsewhere.
                format!("{}–{}", a.label(), b.label())
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let counted: usize = runs[..shown]
        .iter()
        .map(|&(a, b)| (b.col as i32 - a.col as i32).unsigned_abs() as usize + 1)
        .sum();
    if counted < wells.len() {
        out.push_str(&format!(" +{} more", wells.len() - counted));
    }
    out
}

/// Every well spelled out, comma-separated. Used where a range would have to be
/// re-expanded by whatever reads it — CSV export and hover text.
pub fn join_well_labels(wells: &[Well]) -> String {
    wells.iter().map(Well::label).collect::<Vec<_>>().join(", ")
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

    /// `"D5"` → `Well{row: 3, col: 4}`, for readable test fixtures.
    fn w(label: &str) -> Well {
        let (row, col) = label.split_at(1);
        Well::new(
            row.as_bytes()[0] - b'A',
            col.parse::<u8>().expect("column number") - 1,
        )
    }

    #[test]
    fn an_ascending_run_of_wells_collapses_to_a_range() {
        let wells = [w("D5"), w("D6"), w("D7"), w("D8")];
        assert_eq!(format_well_list(&wells, None), "D5–D8");
    }

    #[test]
    fn a_descending_serpentine_run_collapses_in_collection_order() {
        // Row B is odd, so the collector fills it B12 → B1.
        let wells = [w("B12"), w("B11"), w("B10"), w("B9")];
        assert_eq!(format_well_list(&wells, None), "B12–B9");
    }

    #[test]
    fn a_run_is_never_collapsed_across_a_row_change() {
        // Four consecutive serpentine tubes, but no range notation covers them.
        let wells = [w("A11"), w("A12"), w("B12"), w("B11")];
        assert_eq!(format_well_list(&wells, None), "A11, A12, B12, B11");
    }

    #[test]
    fn two_adjacent_wells_stay_spelled_out() {
        assert_eq!(format_well_list(&[w("D5"), w("D6")], None), "D5, D6");
    }

    #[test]
    fn a_long_list_is_truncated_with_a_count_of_what_is_hidden() {
        let wells = [w("A1"), w("A3"), w("A5"), w("A7"), w("A9"), w("A11")];
        assert_eq!(format_well_list(&wells, Some(3)), "A1, A3, A5 +3 more");
    }

    #[test]
    fn truncation_counts_wells_not_ranges() {
        // One four-well range plus two singletons, shown two entries deep.
        let wells = [w("C1"), w("C2"), w("C3"), w("C4"), w("E7"), w("G2")];
        assert_eq!(format_well_list(&wells, Some(2)), "C1–C4, E7 +1 more");
    }

    #[test]
    fn an_empty_well_list_renders_as_an_empty_string() {
        assert_eq!(format_well_list(&[], None), "");
        assert_eq!(format_well_list(&[], Some(4)), "");
        assert_eq!(join_well_labels(&[]), "");
    }

    #[test]
    fn joined_labels_never_collapse_a_range() {
        let wells = [w("D5"), w("D6"), w("D7"), w("D8")];
        assert_eq!(join_well_labels(&wells), "D5, D6, D7, D8");
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
