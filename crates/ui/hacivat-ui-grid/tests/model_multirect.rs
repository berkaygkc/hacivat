//! M3c Task 35: multi-rectangle selections (HV-GRID-05 multi-rect: ⌘/Ctrl-click and -drag add a
//! range; the M3a copy refusal of a union that neither stacks nor concatenates is kept) and
//! growing and shrinking the selection (cell → row segment → row → all).

// Selections are lists of ranges: a one-element `vec![a..b]` is a list of one range.
#![allow(clippy::single_range_in_vec_init)]

use std::sync::Arc;

use hacivat_ui_grid::model::{CellRect, CopyRefusal, GrowStep, Selection, ViewMap};
use hacivat_ui_grid::{ColId, ModelCell, ModelRow};

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
