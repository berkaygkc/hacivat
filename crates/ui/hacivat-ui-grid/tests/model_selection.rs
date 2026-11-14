//! M3a Task 17: the selection model (HV-GRID-05) and the key semantics of `GridCommand`:
//! anchor and focus, rectangles in view coordinates, row and column sets, select-all, `u128`
//! counts by inclusion–exclusion, copy normalisation, and the generation lifecycle.

// Selections are lists of ranges: a one-element `vec![a..b]` or `&[a..b]` is meant as a list of
// one range, never as `(a..b).collect()`.
#![allow(clippy::single_range_in_vec_init)]

use hacivat_ui_grid::model::{
    CellRect, CopyRefusal, CopyRegion, Dir, KeyCtx, KeyOutcome, MAX_COPY_CELLS, Selection, ViewMap,
    apply_command,
};
use hacivat_ui_grid::{ColId, GridCommand, ModelCell, ModelRow, ViewRow};

fn c(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

fn rect(rows: std::ops::Range<u64>, cols: std::ops::Range<u32>) -> CellRect {
    CellRect { rows, cols }
}

fn ident() -> ViewMap {
    ViewMap::identity(100)
}

fn cols(n: u32) -> Vec<ColId> {
    (0..n).map(ColId).collect()
}

fn all() -> Selection {
    let mut s = Selection::default();
    s.select_all();
    s
}

/// r[0,2) c[0,2) and r[5,7) c[2,4): neither the same columns nor the same rows.
fn two_rects() -> Selection {
    let m = ident();
    let mut s = Selection::default();
    s.select_cell(c(0, 0), &m);
    s.extend_to(c(1, 1), &m);
    s.add_cell(c(5, 2), &m);
    s.extend_to(c(6, 3), &m);
    s
}

fn region(rows: Vec<std::ops::Range<u64>>, cols: &[u32]) -> CopyRegion {
    CopyRegion {
        rows,
        cols: cols.iter().map(|c| ColId(*c)).collect(),
    }
}

#[test]
fn click_shift_click_and_ctrl_click() {
    let m = ident();
    let mut s = Selection::default();
    assert!(s.is_empty());
    s.select_cell(c(2, 3), &m);
    assert_eq!(s.rects(), &[rect(2..3, 3..4)]);
    s.extend_to(c(5, 1), &m);
    assert_eq!(s.rects(), &[rect(2..6, 1..4)]);
    assert_eq!(s.anchor(), Some(c(2, 3)));
    assert_eq!(s.focus(), Some(c(5, 1)));

    s.add_cell(c(10, 10), &m);
    assert_eq!(s.rects(), &[rect(2..6, 1..4), rect(10..11, 10..11)]);
    assert_eq!(s.anchor(), Some(c(10, 10)));
    // The second rectangle is the active one: Shift extends it and leaves the first alone.
    s.extend_to(c(12, 11), &m);
    assert_eq!(s.rects(), &[rect(2..6, 1..4), rect(10..13, 10..12)]);
    assert_eq!(s.focus(), Some(c(12, 11)));

    // A plain click collapses everything to one cell.
    s.select_cell(c(1, 1), &m);
    assert_eq!(s.rects(), &[rect(1..2, 1..2)]);
    assert_eq!(s.cell_count(100, 20), 1);
}

#[test]
fn shift_click_without_an_anchor_selects_the_cell() {
    let m = ident();
    let mut s = Selection::default();
    s.extend_to(c(4, 2), &m);
    assert_eq!(s.rects(), &[rect(4..5, 2..3)]);
    assert_eq!(s.anchor(), Some(c(4, 2)));
}

#[test]
fn gutter_rows_by_click_shift_and_ctrl() {
    let mut s = Selection::default();
    s.select_row(ModelRow(7), ColId(2));
    s.extend_rows_to(ModelRow(9));
    s.add_row(ModelRow(20), ColId(2));
    assert_eq!(s.rows(), &[7..10, 20..21]);
    assert!(s.rects().is_empty());
    assert_eq!(s.focus(), Some(c(20, 2)));
    assert!(s.row_selected(ModelRow(8)));
    assert!(!s.row_selected(ModelRow(10)));
    // Shift from the new anchor extends only the active range, upwards too.
    s.extend_rows_to(ModelRow(18));
    assert_eq!(s.rows(), &[7..10, 18..21]);
    // Overlapping ranges merge in the set.
    s.add_row(ModelRow(9), ColId(0));
    s.extend_rows_to(ModelRow(12));
    assert_eq!(s.rows(), &[7..13, 18..21]);
}

#[test]
fn header_columns_by_click_shift_and_ctrl() {
    let mut s = Selection::default();
    s.select_column(ColId(2), ModelRow(5));
    s.extend_columns_to(ColId(4));
    s.add_column(ColId(7), ModelRow(5));
    assert_eq!(s.cols(), &[2..5, 7..8]);
    assert!(s.col_selected(ColId(3)));
    assert!(!s.col_selected(ColId(5)));
    assert_eq!(s.focus(), Some(c(5, 7)));
    assert_eq!(s.cell_count(10, 10), 40);
}

#[test]
fn a_shift_click_on_the_gutter_turns_the_active_rect_into_rows() {
    let m = ident();
    let mut s = Selection::default();
    s.select_cell(c(3, 1), &m);
    s.extend_to(c(4, 2), &m);
    s.extend_rows_to(ModelRow(6));
    assert!(s.rects().is_empty());
    assert_eq!(s.rows(), &[3..7]);
    s.extend_columns_to(ColId(5));
    assert!(s.rows().is_empty());
    assert_eq!(s.cols(), &[1..6]);
}

#[test]
fn select_all_on_a_huge_grid_counts_in_u128() {
    assert_eq!(all().cell_count(1_000_000_000, 10_000), 10_000_000_000_000);
    assert_eq!(
        all().cell_count(u64::MAX, u32::MAX),
        u128::from(u64::MAX) * u128::from(u32::MAX)
    );
    assert!(all().is_all());
    assert!(all().contains(ViewRow(999_999_999), ColId(9_999), &ident()));
}

#[test]
fn rows_columns_and_rects_count_by_inclusion_exclusion() {
    let m = ident();
    let mut s = Selection::default();
    s.select_row(ModelRow(0), ColId(0));
    s.extend_rows_to(ModelRow(9));
    s.add_column(ColId(0), ModelRow(0));
    s.extend_columns_to(ColId(1));
    s.add_cell(c(5, 1), &m);
    s.extend_to(c(19, 4), &m);
    // 10 rows × 10 + 2 columns × 100 − 10 × 2, plus rows 10..20 × columns 2..5.
    assert_eq!(s.cell_count(100, 10), 280 + 30);
    // Overlapping rectangles count once.
    let mut s = Selection::default();
    s.select_cell(c(0, 0), &m);
    s.extend_to(c(9, 9), &m);
    s.add_cell(c(5, 5), &m);
    s.extend_to(c(14, 14), &m);
    assert_eq!(s.cell_count(100, 100), 100 + 100 - 25);
    // Everything is clipped to the grid.
    assert_eq!(s.cell_count(10, 10), 100);
    assert_eq!(s.cell_count(0, 10), 0);
}

/// SplitMix64, for seeded random selections (no `proptest` in this crate's manifest).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// The count matches a brute-force walk over `contains`, for random mixes of every kind.
#[test]
fn cell_count_matches_membership_on_random_selections() {
    const R: u64 = 24;
    const C: u32 = 9;
    let m = ViewMap::identity(R);
    let mut rng = Rng(0x7_17_05);
    for case in 0..2_000 {
        let mut s = Selection::default();
        for _ in 0..1 + rng.below(6) {
            // Some coordinates fall outside the grid, to exercise the clipping.
            let row = rng.below(R + 4);
            let col = rng.below(u64::from(C) + 2) as u32;
            match rng.below(9) {
                0 => s.select_cell(c(row, col), &m),
                1 | 2 => s.add_cell(c(row, col), &m),
                3 | 4 => s.extend_to(c(row, col), &m),
                5 => s.add_row(ModelRow(row), ColId(col)),
                6 => s.extend_rows_to(ModelRow(row)),
                7 => s.add_column(ColId(col), ModelRow(row)),
                _ => s.extend_columns_to(ColId(col)),
            }
        }
        let brute = (0..R)
            .flat_map(|r| (0..C).map(move |k| (r, k)))
            .filter(|&(r, k)| s.contains(ViewRow(r), ColId(k), &m))
            .count() as u128;
        assert_eq!(s.cell_count(R, C), brute, "case {case}: {s:?}");
    }
}

#[test]
fn copy_of_disjoint_rects_is_refused() {
    assert_eq!(
        two_rects().normalize_for_copy(&ident(), &cols(4)),
        Err(CopyRefusal::MultipleSelection)
    );
}

#[test]
fn rects_with_the_same_columns_stack() {
    let m = ident();
    let mut s = Selection::default();
    s.select_cell(c(5, 1), &m);
    s.extend_to(c(6, 2), &m);
    s.add_cell(c(0, 1), &m);
    s.extend_to(c(1, 2), &m);
    s.add_cell(c(1, 1), &m);
    s.extend_to(c(2, 2), &m);
    assert_eq!(
        s.normalize_for_copy(&m, &cols(4)),
        Ok(region(vec![0..3, 5..7], &[1, 2]))
    );
}

#[test]
fn rects_with_the_same_rows_concatenate() {
    let m = ident();
    let mut s = Selection::default();
    s.select_cell(c(0, 2), &m);
    s.extend_to(c(2, 3), &m);
    s.add_cell(c(0, 0), &m);
    s.extend_to(c(2, 0), &m);
    assert_eq!(
        s.normalize_for_copy(&m, &cols(4)),
        Ok(region(vec![0..3], &[0, 2, 3]))
    );
}

#[test]
fn row_and_column_sets_copy_whole_rows_and_columns() {
    let m = ViewMap::identity(50);
    let mut s = Selection::default();
    s.select_row(ModelRow(7), ColId(0));
    s.extend_rows_to(ModelRow(9));
    s.add_row(ModelRow(20), ColId(0));
    assert_eq!(
        s.normalize_for_copy(&m, &cols(3)),
        Ok(region(vec![7..10, 20..21], &[0, 1, 2]))
    );
    // A rectangle over every visible column stacks with whole rows.
    s.add_cell(c(30, 0), &m);
    s.extend_to(c(31, 2), &m);
    assert_eq!(
        s.normalize_for_copy(&m, &cols(3)),
        Ok(region(vec![7..10, 20..21, 30..32], &[0, 1, 2]))
    );

    let mut s = Selection::default();
    s.select_column(ColId(2), ModelRow(0));
    s.add_column(ColId(0), ModelRow(0));
    assert_eq!(
        s.normalize_for_copy(&m, &cols(3)),
        Ok(region(vec![0..50], &[0, 2]))
    );

    // Rows and columns together are neither stacked nor concatenable.
    s.add_row(ModelRow(3), ColId(0));
    assert_eq!(
        s.normalize_for_copy(&m, &cols(3)),
        Err(CopyRefusal::MultipleSelection)
    );
}

#[test]
fn select_all_copies_the_loaded_grid_and_is_refused_over_a_million_cells() {
    assert_eq!(
        all().normalize_for_copy(&ViewMap::identity(4), &cols(2)),
        Ok(region(vec![0..4], &[0, 1]))
    );
    assert_eq!(
        all().normalize_for_copy(&ViewMap::identity(1_000), &cols(1_000)),
        Ok(CopyRegion {
            rows: vec![0..1_000],
            cols: cols(1_000)
        })
    );
    assert_eq!(MAX_COPY_CELLS, 1_000_000);
    assert_eq!(
        all().normalize_for_copy(&ViewMap::identity(1_001), &cols(1_000)),
        Err(CopyRefusal::TooMany { cells: 1_001_000 })
    );
    assert_eq!(
        all().normalize_for_copy(&ViewMap::identity(1_000_000_000), &cols(10_000)),
        Err(CopyRefusal::TooMany {
            cells: 10_000_000_000_000
        })
    );
    let r = region(vec![0..3, 10..12], &[1, 4, 5]);
    assert_eq!((r.row_count(), r.cell_count()), (5, 15));
}

#[test]
fn copy_takes_only_loaded_rows_and_an_append_keeps_the_selection() {
    let m = ViewMap::identity(10);
    let mut s = Selection::default();
    s.select_cell(c(5, 0), &m);
    s.extend_to(c(19, 1), &m);
    assert_eq!(
        s.normalize_for_copy(&m, &cols(2)),
        Ok(region(vec![5..10], &[0, 1]))
    );
    // Rows stream in: the selection is untouched and now covers them.
    let before = s.clone();
    let grown = ViewMap::identity(30);
    assert_eq!(s, before);
    assert_eq!(
        s.normalize_for_copy(&grown, &cols(2)),
        Ok(region(vec![5..20], &[0, 1]))
    );
    assert!(s.contains(ViewRow(19), ColId(1), &grown));
}

#[test]
fn a_hidden_column_inside_a_rect_is_left_out_of_the_copy() {
    let m = ident();
    let mut s = Selection::default();
    s.select_cell(c(0, 1), &m);
    s.extend_to(c(2, 3), &m);
    let visible = [ColId(0), ColId(1), ColId(3)];
    assert_eq!(
        s.normalize_for_copy(&m, &visible),
        Ok(region(vec![0..3], &[1, 3]))
    );
}

#[test]
fn nothing_selected_copies_nothing() {
    assert_eq!(
        Selection::default().normalize_for_copy(&ident(), &cols(3)),
        Ok(region(vec![], &[]))
    );
}

#[test]
fn generation_change_clears_selection_and_clamps_focus() {
    let m = ViewMap::identity(1_000);
    let mut s = Selection::default();
    s.select_cell(c(900, 2), &m);
    s.extend_to(c(950, 5), &m);
    s.add_row(ModelRow(3), ColId(5));
    s.add_column(ColId(7), ModelRow(950));
    s.select_all();
    assert_eq!(s.focus(), Some(c(950, 7)));
    assert!(s.is_all());

    // A re-run with 10 rows and 4 columns: one focused cell, clamped into the new grid.
    let new = ViewMap::identity(10);
    s.on_generation_changed(&new, &cols(4));
    assert!(!s.is_all());
    assert!(s.rows().is_empty() && s.cols().is_empty());
    assert_eq!(s.focus(), Some(c(9, 3)));
    assert_eq!(s.anchor(), Some(c(9, 3)));
    assert_eq!(s.rects(), &[rect(9..10, 3..4)]);
    assert_eq!(s.cell_count(10, 4), 1);
    assert_eq!(
        s.normalize_for_copy(&new, &cols(4)),
        Ok(region(vec![9..10], &[3]))
    );

    // A focus inside the new grid stays where it is.
    s.on_generation_changed(&ViewMap::identity(500), &cols(9));
    assert_eq!(s.focus(), Some(c(9, 3)));

    // No rows or no columns: no cell to focus.
    s.on_generation_changed(&ViewMap::identity(0), &cols(4));
    assert_eq!(s.focus(), None);
    assert!(s.is_empty());
    let mut t = Selection::default();
    t.select_cell(c(1, 1), &m);
    t.on_generation_changed(&m, &[]);
    assert!(t.is_empty() && t.focus().is_none());
}

#[test]
fn membership_by_kind() {
    let m = ident();
    let mut s = Selection::default();
    s.select_row(ModelRow(8), ColId(0));
    s.add_column(ColId(3), ModelRow(0));
    s.add_cell(c(20, 1), &m);
    assert!(s.contains(ViewRow(8), ColId(7), &m), "row set");
    assert!(s.contains(ViewRow(50), ColId(3), &m), "column set");
    assert!(s.contains(ViewRow(20), ColId(1), &m), "rect");
    assert!(!s.contains(ViewRow(20), ColId(2), &m));
    assert!(!s.contains(ViewRow(9), ColId(0), &m));
}

#[test]
fn collapse_keeps_only_the_focused_cell() {
    let m = ident();
    let mut s = Selection::default();
    assert!(!s.collapse(&m), "nothing to collapse");
    s.select_cell(c(2, 2), &m);
    s.extend_to(c(6, 4), &m);
    s.add_row(ModelRow(30), ColId(4));
    assert!(s.collapse(&m));
    assert_eq!(s.rects(), &[rect(30..31, 4..5)]);
    assert!(s.rows().is_empty());
    assert_eq!(s.anchor(), s.focus());
    assert!(!s.collapse(&m), "already collapsed");
}

// ---- keys -------------------------------------------------------------------------------------

fn ctx<'a>(map: &'a ViewMap, order: &'a [ColId]) -> KeyCtx<'a> {
    KeyCtx {
        map,
        order,
        page_rows: 20,
    }
}

fn mv(dir: Dir, extend: bool) -> GridCommand {
    GridCommand::Move { dir, extend }
}

fn moved(row: u64, col: u32) -> KeyOutcome {
    KeyOutcome::Moved {
        focus: c(row, col),
        follow_tail: false,
    }
}

#[test]
fn arrows_move_the_focus_and_stop_at_the_edges() {
    let m = ViewMap::identity(5);
    let order = cols(3);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    // The first key focuses the first cell.
    assert_eq!(apply_command(mv(Dir::Down, false), &mut s, &k), moved(0, 0));
    assert_eq!(
        apply_command(mv(Dir::Right, false), &mut s, &k),
        moved(0, 1)
    );
    assert_eq!(
        apply_command(mv(Dir::Right, false), &mut s, &k),
        moved(0, 2)
    );
    assert_eq!(
        apply_command(mv(Dir::Right, false), &mut s, &k),
        KeyOutcome::Unchanged
    );
    assert_eq!(apply_command(mv(Dir::Down, false), &mut s, &k), moved(1, 2));
    assert_eq!(apply_command(mv(Dir::Left, false), &mut s, &k), moved(1, 1));
    assert_eq!(apply_command(mv(Dir::Up, false), &mut s, &k), moved(0, 1));
    assert_eq!(
        apply_command(mv(Dir::Up, false), &mut s, &k),
        KeyOutcome::Unchanged
    );
    assert_eq!(s.rects(), &[rect(0..1, 1..2)]);
}

#[test]
fn shift_arrows_extend_and_a_plain_arrow_collapses() {
    let m = ident();
    let order = cols(6);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(2, 3), &m);
    apply_command(mv(Dir::Down, true), &mut s, &k);
    assert_eq!(apply_command(mv(Dir::Down, true), &mut s, &k), moved(4, 3));
    assert_eq!(s.rects(), &[rect(2..5, 3..4)]);
    assert_eq!(s.anchor(), Some(c(2, 3)));
    apply_command(mv(Dir::Left, true), &mut s, &k);
    assert_eq!(s.rects(), &[rect(2..5, 2..4)]);
    // A plain arrow moves on from the focus and collapses the selection.
    assert_eq!(apply_command(mv(Dir::Down, false), &mut s, &k), moved(5, 2));
    assert_eq!(s.rects(), &[rect(5..6, 2..3)]);
}

#[test]
fn jumps_go_to_the_edge_never_to_the_next_non_empty_cell() {
    // 500 rows loaded while more stream in: the last row is the last loaded one.
    let m = ViewMap::identity(500);
    let order = cols(8);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(10, 3), &m);
    let jump = |dir| GridCommand::JumpEdge { dir, extend: false };
    assert_eq!(apply_command(jump(Dir::Right), &mut s, &k), moved(10, 7));
    assert_eq!(apply_command(jump(Dir::Down), &mut s, &k), moved(499, 7));
    assert_eq!(apply_command(jump(Dir::Left), &mut s, &k), moved(499, 0));
    assert_eq!(apply_command(jump(Dir::Up), &mut s, &k), moved(0, 0));
    let extend = GridCommand::JumpEdge {
        dir: Dir::Down,
        extend: true,
    };
    apply_command(extend, &mut s, &k);
    assert_eq!(s.rects(), &[rect(0..500, 0..1)]);
}

#[test]
fn first_and_last_row_keep_the_column_and_last_row_follows_the_tail() {
    let m = ViewMap::identity(500);
    let order = cols(8);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(10, 3), &m);
    assert_eq!(
        apply_command(GridCommand::LastRow { extend: false }, &mut s, &k),
        KeyOutcome::Moved {
            focus: c(499, 3),
            follow_tail: true
        }
    );
    assert_eq!(
        apply_command(GridCommand::FirstRow { extend: true }, &mut s, &k),
        moved(0, 3)
    );
    assert_eq!(s.rects(), &[rect(0..500, 3..4)]);
}

#[test]
fn home_end_and_page_keys() {
    let m = ViewMap::identity(100);
    let order = cols(8);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(5, 3), &m);
    assert_eq!(
        apply_command(GridCommand::End { extend: false }, &mut s, &k),
        moved(5, 7)
    );
    assert_eq!(
        apply_command(GridCommand::Home { extend: true }, &mut s, &k),
        moved(5, 0)
    );
    assert_eq!(s.rects(), &[rect(5..6, 0..8)]);
    let page = |up, extend| GridCommand::Page { up, extend };
    assert_eq!(apply_command(page(false, false), &mut s, &k), moved(25, 0));
    assert_eq!(apply_command(page(true, false), &mut s, &k), moved(5, 0));
    assert_eq!(apply_command(page(true, false), &mut s, &k), moved(0, 0));
    for _ in 0..10 {
        apply_command(page(false, true), &mut s, &k);
    }
    assert_eq!(s.focus(), Some(c(99, 0)));
    assert_eq!(s.rects(), &[rect(0..100, 0..1)]);
}

#[test]
fn tab_wraps_rows_inside_the_grid_and_leaves_from_the_last_cell() {
    let m = ViewMap::identity(2);
    let order = cols(3);
    let k = ctx(&m, &order);
    let tab = GridCommand::NextCell { back: false };
    let back = GridCommand::NextCell { back: true };
    let mut s = Selection::default();
    s.select_cell(c(0, 1), &m);
    assert_eq!(apply_command(tab, &mut s, &k), moved(0, 2));
    assert_eq!(apply_command(tab, &mut s, &k), moved(1, 0));
    assert_eq!(apply_command(tab, &mut s, &k), moved(1, 1));
    assert_eq!(apply_command(tab, &mut s, &k), moved(1, 2));
    assert_eq!(
        apply_command(tab, &mut s, &k),
        KeyOutcome::Leave { back: false }
    );
    assert_eq!(s.focus(), Some(c(1, 2)), "leaving keeps the focus");
    assert_eq!(apply_command(back, &mut s, &k), moved(1, 1));
    s.select_cell(c(1, 0), &m);
    assert_eq!(apply_command(back, &mut s, &k), moved(0, 2));
    s.select_cell(c(0, 0), &m);
    assert_eq!(
        apply_command(back, &mut s, &k),
        KeyOutcome::Leave { back: true }
    );
}

#[test]
fn an_empty_grid_lets_tab_leave_and_ignores_moves() {
    let none = ViewMap::identity(0);
    let order = cols(3);
    let k = ctx(&none, &order);
    let mut s = Selection::default();
    assert_eq!(
        apply_command(GridCommand::NextCell { back: false }, &mut s, &k),
        KeyOutcome::Leave { back: false }
    );
    assert_eq!(
        apply_command(GridCommand::NextCell { back: true }, &mut s, &k),
        KeyOutcome::Leave { back: true }
    );
    assert_eq!(
        apply_command(mv(Dir::Down, false), &mut s, &k),
        KeyOutcome::Unchanged
    );
    let m = ViewMap::identity(3);
    let k = ctx(&m, &[]);
    assert_eq!(
        apply_command(GridCommand::LastRow { extend: false }, &mut s, &k),
        KeyOutcome::Unchanged
    );
    assert!(s.is_empty());
}

#[test]
fn hidden_columns_are_skipped() {
    let m = ident();
    let order = [ColId(0), ColId(1), ColId(3)];
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(0, 1), &m);
    assert_eq!(
        apply_command(mv(Dir::Right, false), &mut s, &k),
        moved(0, 3)
    );
    // A focus on a column that was hidden afterwards moves to its visible neighbours.
    s.select_cell(c(4, 2), &m);
    assert_eq!(
        apply_command(mv(Dir::Right, false), &mut s, &k),
        moved(4, 3)
    );
    s.select_cell(c(4, 2), &m);
    assert_eq!(apply_command(mv(Dir::Left, false), &mut s, &k), moved(4, 1));
    s.select_cell(c(4, 2), &m);
    assert_eq!(apply_command(mv(Dir::Down, false), &mut s, &k), moved(5, 3));
}

#[test]
fn a_focus_past_the_loaded_rows_is_clamped_by_the_next_key() {
    let big = ViewMap::identity(1_000);
    let mut s = Selection::default();
    s.select_cell(c(900, 0), &big);
    let m = ViewMap::identity(100);
    let order = cols(2);
    assert_eq!(
        apply_command(mv(Dir::Down, false), &mut s, &ctx(&m, &order)),
        moved(99, 0)
    );
}

#[test]
fn select_all_row_column_copy_and_escape() {
    let m = ident();
    let order = cols(5);
    let k = ctx(&m, &order);
    let mut s = Selection::default();
    s.select_cell(c(2, 1), &m);
    s.extend_to(c(4, 2), &m);

    assert_eq!(
        apply_command(GridCommand::SelectRow, &mut s, &k),
        KeyOutcome::Changed
    );
    assert_eq!(s.rows(), &[2..5]);
    assert!(s.rects().is_empty());
    assert_eq!(
        apply_command(GridCommand::SelectColumn, &mut s, &k),
        KeyOutcome::Changed
    );
    assert_eq!(s.cols(), &[1..3]);
    assert!(s.rows().is_empty());

    assert_eq!(
        apply_command(GridCommand::Copy { headers: true }, &mut s, &k),
        KeyOutcome::Copy {
            region: Ok(region(vec![0..100], &[1, 2])),
            headers: true
        }
    );

    assert_eq!(
        apply_command(GridCommand::SelectAll, &mut s, &k),
        KeyOutcome::Changed
    );
    assert!(s.is_all());
    assert_eq!(
        apply_command(GridCommand::SelectAll, &mut s, &k),
        KeyOutcome::Unchanged
    );
    assert_eq!(s.focus(), Some(c(4, 2)), "select-all keeps the focus");

    assert_eq!(
        apply_command(GridCommand::Escape, &mut s, &k),
        KeyOutcome::Changed
    );
    assert_eq!(s.rects(), &[rect(4..5, 2..3)]);
    assert_eq!(
        apply_command(GridCommand::Escape, &mut s, &k),
        KeyOutcome::Unchanged
    );
}

#[test]
fn select_all_on_an_unfocused_grid_focuses_the_first_cell() {
    let m = ident();
    let order = cols(5);
    let mut s = Selection::default();
    assert_eq!(
        apply_command(GridCommand::SelectAll, &mut s, &ctx(&m, &order)),
        KeyOutcome::Changed
    );
    assert!(s.is_all());
    assert_eq!(s.focus(), Some(c(0, 0)));
}
