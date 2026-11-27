//! M3c Task 35: multi-rectangle selections (HV-GRID-05 multi-rect: ⌘/Ctrl-click and -drag add a
//! range; the M3a copy refusal of a union that neither stacks nor concatenates is kept) and
//! growing and shrinking the selection (cell → row segment → row → all).

// Selections are lists of ranges: a one-element `vec![a..b]` is a list of one range.
#![allow(clippy::single_range_in_vec_init)]

use std::sync::Arc;

use hacivat_ui_grid::model::{CellRect, CopyRefusal, GrowStep, Selection, ViewMap};
use hacivat_ui_grid::{ColId, ModelCell, ModelRow, ViewRow};

fn c(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

fn cols(n: u32) -> Vec<ColId> {
    (0..n).map(ColId).collect()
}

#[test]
fn multi_rect_union_copy_refusal_is_unchanged() {
    let m = ViewMap::identity(20);
    let order = cols(5);
    // r[0,2) c[0,2) and r[5,7) c[2,4): neither the same columns nor the same rows.
    let mut s = Selection::default();
    s.select_cell(c(0, 0), &m);
    s.extend_to(c(1, 1), &m);
    s.add_cell(c(5, 2), &m);
    s.extend_to(c(6, 3), &m);
    assert_eq!(s.ranges(), 2);
    assert_eq!(s.cell_count_in(&m, &order), 8);
    assert_eq!(
        s.normalize_for_copy(&m, &order),
        Err(CopyRefusal::MultipleSelection)
    );
    // The same refusal under a sort: the rectangles are view rows either way.
    let sorted = ViewMap::permuted(Arc::from((0..20u64).rev().collect::<Vec<_>>()), 20, 20);
    let mut t = Selection::default();
    t.select_cell(c(19, 0), &sorted);
    t.extend_to(c(18, 1), &sorted);
    t.add_cell(c(14, 2), &sorted);
    t.extend_to(c(13, 3), &sorted);
    assert_eq!(t.rects(), s.rects());
    assert_eq!(
        t.normalize_for_copy(&sorted, &order),
        Err(CopyRefusal::MultipleSelection)
    );

    // Stacked (the same columns) and concatenated (the same rows) unions still copy.
    let mut stacked = Selection::default();
    stacked.select_cell(c(0, 1), &m);
    stacked.extend_to(c(1, 2), &m);
    stacked.add_cell(c(4, 1), &m);
    stacked.extend_to(c(4, 2), &m);
    let r = stacked.normalize_for_copy(&m, &order).unwrap();
    assert_eq!(
        (r.rows, r.cols),
        (vec![0..2, 4..5], vec![ColId(1), ColId(2)])
    );
    let mut side = Selection::default();
    side.select_cell(c(2, 0), &m);
    side.add_cell(c(2, 3), &m);
    let r = side.normalize_for_copy(&m, &order).unwrap();
    assert_eq!((r.rows, r.cols), (vec![2..3], vec![ColId(0), ColId(3)]));

    // One piece, or everything, is one range.
    let mut one = Selection::default();
    one.select_cell(c(0, 0), &m);
    assert_eq!(one.ranges(), 1);
    one.select_all();
    assert_eq!(one.ranges(), 1);
    assert_eq!(Selection::default().ranges(), 0);
}

#[test]
fn grow_and_shrink_selection_steps() {
    let m = ViewMap::identity(10);
    let order = cols(8);
    // Columns 2..5 are on screen around the focus (the row segment).
    let seg = 2..5;
    let mut s = Selection::default();
    s.select_cell(c(3, 3), &m);
    assert_eq!(s.grow_step(&m, &order, seg.clone()), Some(GrowStep::Cell));

    assert!(s.grow(&m, &order, seg.clone()));
    assert_eq!(
        s.grow_step(&m, &order, seg.clone()),
        Some(GrowStep::RowSegment)
    );
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 3..4,
            cols: 2..5
        }]
    );
    assert_eq!(s.focus(), Some(c(3, 3)));

    assert!(s.grow(&m, &order, seg.clone()));
    assert_eq!(s.grow_step(&m, &order, seg.clone()), Some(GrowStep::Row));
    assert_eq!(s.rows(), &[3..4]);
    assert!(s.rects().is_empty());

    assert!(s.grow(&m, &order, seg.clone()));
    assert_eq!(s.grow_step(&m, &order, seg.clone()), Some(GrowStep::All));
    assert!(s.is_all());
    assert!(!s.grow(&m, &order, seg.clone()), "all is the last step");
    assert_eq!(s.focus(), Some(c(3, 3)));

    // Shrinking walks back down to the focus cell, and stops there.
    assert!(s.shrink(&m, &order, seg.clone()));
    assert_eq!(s.rows(), &[3..4]);
    assert!(s.shrink(&m, &order, seg.clone()));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 3..4,
            cols: 2..5
        }]
    );
    assert!(s.shrink(&m, &order, seg.clone()));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 3..4,
            cols: 3..4
        }]
    );
    assert!(!s.shrink(&m, &order, seg.clone()));
    assert_eq!(s.focus(), Some(c(3, 3)));
}

#[test]
fn grow_skips_a_row_segment_that_is_the_cell_or_the_row() {
    let m = ViewMap::identity(10);
    let order = cols(4);
    // Every column on screen: the segment is the row.
    let mut s = Selection::default();
    s.select_cell(c(1, 2), &m);
    assert!(s.grow(&m, &order, 0..4));
    assert_eq!(s.rows(), &[1..2]);
    assert!(s.shrink(&m, &order, 0..4));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 1..2,
            cols: 2..3
        }]
    );
    // The focus scrolled off screen: no segment either.
    assert!(s.grow(&m, &order, 0..2));
    assert_eq!(s.rows(), &[1..2]);
    // A one-column segment is the cell.
    let mut t = Selection::default();
    t.select_cell(c(1, 2), &m);
    assert!(t.grow(&m, &order, 2..3));
    assert_eq!(t.rows(), &[1..2]);
}

#[test]
fn grow_from_other_shapes_and_from_nothing() {
    let m = ViewMap::identity(10);
    let order = cols(6);
    // Nothing focused: the first cell.
    let mut s = Selection::default();
    assert!(s.grow(&m, &order, 0..3));
    assert_eq!(s.focus(), Some(c(0, 0)));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 0..1,
            cols: 0..1
        }]
    );
    // A one-row rectangle inside the segment grows to the segment; across it, to the row.
    let mut one_row = Selection::default();
    one_row.select_cell(c(4, 1), &m);
    one_row.extend_to(c(4, 2), &m);
    assert_eq!(one_row.grow_step(&m, &order, 0..4), None);
    assert!(one_row.grow(&m, &order, 0..4));
    assert_eq!(
        one_row.rects(),
        &[CellRect {
            rows: 4..5,
            cols: 0..4
        }]
    );
    // A block of several rows grows to everything; shrinking it gives the focus cell.
    let mut block = Selection::default();
    block.select_cell(c(2, 1), &m);
    block.extend_to(c(5, 3), &m);
    let mut grown = block.clone();
    assert!(grown.grow(&m, &order, 0..6));
    assert!(grown.is_all());
    assert!(block.shrink(&m, &order, 0..6));
    assert_eq!(
        block.rects(),
        &[CellRect {
            rows: 5..6,
            cols: 3..4
        }]
    );
    assert_eq!(block.focus(), Some(c(5, 3)));
    // An empty grid has nothing to grow to.
    let mut empty = Selection::default();
    assert!(!empty.grow(&ViewMap::identity(0), &order, 0..0));
}

#[test]
fn grow_follows_the_focus_through_a_sort() {
    let sorted = ViewMap::permuted(Arc::from((0..10u64).rev().collect::<Vec<_>>()), 10, 10);
    let order = cols(6);
    let mut s = Selection::default();
    // Model row 2 shows at view row 7.
    s.select_cell(c(2, 1), &sorted);
    assert!(s.grow(&sorted, &order, 0..3));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 7..8,
            cols: 0..3
        }]
    );
    assert!(s.grow(&sorted, &order, 0..3));
    // The row step is the focus's model row, so it follows the data through a re-sort.
    assert_eq!(s.rows(), &[2..3]);
    assert!(s.shrink(&sorted, &order, 0..3));
    assert_eq!(
        s.rects(),
        &[CellRect {
            rows: 7..8,
            cols: 0..3
        }]
    );
}

// ## m3c-int (I4): the rectangles are indexed by row, so a cell test reads only the rectangles
// over its row (Select all occurrences makes up to 10,000).

/// A deterministic pseudo-random sequence (no `rand` in the tests).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % below
    }
}

/// What `contains` answers, read the slow way from the public pieces.
fn contains_by_hand(s: &Selection, row: u64, col: u32, map: &ViewMap) -> bool {
    s.is_all()
        || s.cols().iter().any(|c| c.contains(&col))
        || map
            .view_to_model(ViewRow(row))
            .is_some_and(|m| s.rows().iter().any(|r| r.contains(&m.0)))
        || s.rects().iter().any(|r| r.contains(row, col))
}

#[test]
fn contains_with_many_rectangles_matches_every_rectangle() {
    let m = ViewMap::identity(400);
    for seed in 1..=12u64 {
        let mut rng = Lcg(seed);
        let mut s = Selection::default();
        // Overlapping, nested and long rectangles, with a row and a column piece among them.
        for i in 0..(10 + seed * 15) {
            let (r0, c0) = (rng.next(400), rng.next(12) as u32);
            let tall = if i % 7 == 0 { 120 } else { 6 };
            let (r1, c1) = (
                (r0 + rng.next(tall)).min(399),
                (c0 + rng.next(4) as u32).min(11),
            );
            s.add_cell(c(r0, c0), &m);
            s.extend_to(c(r1, c1), &m);
        }
        if seed % 3 == 0 {
            s.add_row(ModelRow(rng.next(400)), ColId(0));
            s.add_column(ColId(rng.next(12) as u32), ModelRow(0));
        }
        for row in 0..400u64 {
            let any = s.rects().iter().any(|r| r.rows.contains(&row));
            assert_eq!(s.row_in_rects(row), any, "seed {seed}, row {row}");
            for col in 0..12u32 {
                assert_eq!(
                    s.contains(ViewRow(row), ColId(col), &m),
                    contains_by_hand(&s, row, col, &m),
                    "seed {seed}, cell ({row}, {col})"
                );
            }
        }
    }
}

#[test]
fn contains_follows_every_edit_of_the_rectangles() {
    let m = ViewMap::identity(100);
    let mut s = Selection::default();
    s.select_cell(c(50, 2), &m);
    s.add_cell(c(10, 1), &m);
    s.extend_to(c(20, 3), &m);
    assert!(s.contains(ViewRow(15), ColId(2), &m));
    assert!(s.contains(ViewRow(50), ColId(2), &m));
    // Extending the active rectangle again shrinks it: its old rows are no longer selected.
    s.extend_to(c(12, 1), &m);
    assert!(!s.contains(ViewRow(15), ColId(1), &m));
    assert!(s.contains(ViewRow(11), ColId(1), &m));
    assert!(!s.row_in_rects(15));
    // Escape: only the focus stays.
    assert!(s.collapse(&m));
    assert!(!s.contains(ViewRow(50), ColId(2), &m));
    assert!(s.contains(ViewRow(12), ColId(1), &m));
    // A re-sort clears the rectangles (view rows of the old order) but the focus's.
    let sorted = ViewMap::permuted(Arc::from((0..100u64).rev().collect::<Vec<_>>()), 100, 100);
    s.on_view_map_changed(&sorted);
    assert!(s.contains(ViewRow(87), ColId(1), &sorted));
    assert!(!s.contains(ViewRow(12), ColId(1), &sorted));
    // Select all occurrences: one-cell rectangles, two on one row.
    let cells = [c(3, 0), c(3, 4), c(90, 2), c(40, 1)];
    s.select_cells(&cells, 2, &m);
    for row in 0..100u64 {
        for col in 0..5u32 {
            let want = cells.contains(&c(row, col));
            assert_eq!(
                s.contains(ViewRow(row), ColId(col), &m),
                want,
                "({row}, {col})"
            );
        }
    }
    assert!(s.row_in_rects(3) && s.row_in_rects(90) && !s.row_in_rects(4));
}

#[test]
#[ignore = "perf"]
fn contains_with_10000_rectangles_under_1ms_per_viewport() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    let map = ViewMap::identity(1_000_000);
    let cells: Vec<ModelCell> = (0..10_000u64).map(|i| c(i * 37, (i % 20) as u32)).collect();
    let mut s = Selection::default();
    s.select_cells(&cells, 0, &map);
    // A 60 × 20 viewport (its cells and its gutter), at bands over and past the rectangles.
    let bands = [
        0u64, 400_000, 369_960, 999_940, 185_000, 12_345, 250_000, 333_333,
    ];
    let started = std::time::Instant::now();
    let mut hits = 0u32;
    for (frame, top) in bands.iter().cycle().take(40).enumerate() {
        for r in 0..60u64 {
            let row = top + r + frame as u64 % 3;
            if s.row_in_rects(row) {
                hits += 1;
            }
            for col in 0..20u32 {
                if s.contains(ViewRow(row), ColId(col), &map) {
                    hits += 1;
                }
            }
        }
    }
    let per = started.elapsed().as_secs_f64() * 1000.0 / 40.0;
    println!("contains over a 60 x 20 viewport with 10,000 rectangles: {per:.4} ms ({hits})");
    assert!(hits > 0);
    assert!(per < 1.0, "{per:.4} ms per viewport");
}
