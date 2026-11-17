//! M3c Task 35: the selection under a permuted or filtered `ViewMap` (digest 04 R15: the focus
//! and the row sets are model rows, rectangles are view rows and clear on a re-sort), the map's
//! inverse index, and counts over the rows a filter keeps.

// Selections are lists of ranges: a one-element `vec![a..b]` is a list of one range.
#![allow(clippy::single_range_in_vec_init)]

use std::sync::Arc;

use hacivat_ui_grid::model::{
    CellRect, CopyRegion, Dir, KeyCtx, KeyOutcome, Selection, ViewMap, apply_command,
};
use hacivat_ui_grid::{ColId, GridCommand, ModelCell, ModelRow, ViewRow};

fn c(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

fn cols(n: u32) -> Vec<ColId> {
    (0..n).map(ColId).collect()
}

/// Ten rows sorted in reverse: view row `v` shows model row `9 − v`.
fn reversed(n: u64) -> ViewMap {
    ViewMap::permuted(Arc::from((0..n).rev().collect::<Vec<_>>()), n, n)
}

#[test]
fn constructors_build_the_same_mapping_as_the_variants() {
    let built = ViewMap::permuted(Arc::from([2u64, 0, 1]), 3, 5);
    let ViewMap::Permuted {
        order,
        tail_from,
        loaded,
        ..
    } = &built
    else {
        panic!("{built:?}");
    };
    assert_eq!((&order[..], *tail_from, *loaded), (&[2u64, 0, 1][..], 3, 5));
    for v in 0..5 {
        let m = built.view_to_model(ViewRow(v)).unwrap();
        assert_eq!(built.model_to_view(m), Some(ViewRow(v)), "view {v}");
    }
    // The tail keeps source order.
    assert_eq!(built.view_to_model(ViewRow(4)), Some(ModelRow(4)));
    let kept = ViewMap::filtered(Arc::from([7u64, 2, 9]), 10);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept.model_to_view(ModelRow(2)), Some(ViewRow(1)));
    assert_eq!(kept.model_to_view(ModelRow(3)), None);
    assert_eq!(kept.model_to_view(ModelRow(9_999)), None);
}

#[test]
fn maps_compare_by_their_mapping_not_their_index() {
    let a = reversed(10);
    let b = reversed(10);
    // `a` has answered a lookup (its index is built), `b` has not.
    assert_eq!(a.model_to_view(ModelRow(3)), Some(ViewRow(6)));
    assert_eq!(a, b);
    assert_ne!(a, ViewMap::identity(10));
}

#[test]
fn with_loaded_follows_rows_that_arrived_after_the_sort() {
    let m = ViewMap::permuted(Arc::from([1u64, 0]), 2, 2).with_loaded(4);
    assert_eq!(m.len(), 4);
    assert_eq!(m.view_to_model(ViewRow(3)), Some(ModelRow(3)));
    assert_eq!(m.model_to_view(ModelRow(0)), Some(ViewRow(1)));
    assert_eq!(ViewMap::identity(2).with_loaded(7), ViewMap::identity(7));
    // A filter keeps only its own rows.
    let f = ViewMap::filtered(Arc::from([1u64]), 2).with_loaded(9);
    assert_eq!(f.len(), 1);
}

#[test]
fn focus_follows_its_model_row_through_a_sort() {
    let order = cols(3);
    let ident = ViewMap::identity(10);
    let mut s = Selection::default();
    s.select_cell(c(2, 1), &ident);
    assert_eq!(s.rects(), &[CellRect::spanning((2, 1), (2, 1))]);

    let sorted = reversed(10);
    s.on_view_map_changed(&sorted);
    // The focus is the same data: model row 2, now shown at view row 7.
    assert_eq!(s.focus(), Some(c(2, 1)));
    assert_eq!(sorted.model_to_view(ModelRow(2)), Some(ViewRow(7)));
    assert_eq!(s.rects(), &[CellRect::spanning((7, 1), (7, 1))]);
    assert!(s.contains(ViewRow(7), ColId(1), &sorted));
    assert!(!s.contains(ViewRow(2), ColId(1), &sorted));

    // Down moves one view row: view 8 shows model row 1.
    let ctx = KeyCtx {
        map: &sorted,
        order: &order,
        page_rows: 3,
    };
    let out = apply_command(
        GridCommand::Move {
            dir: Dir::Down,
            extend: false,
        },
        &mut s,
        &ctx,
    );
    assert_eq!(
        out,
        KeyOutcome::Moved {
            focus: c(1, 1),
            follow_tail: false
        }
    );
    // Shift+Up twice from there: view rows 6..=8, whatever their model rows.
    for _ in 0..2 {
        apply_command(
            GridCommand::Move {
                dir: Dir::Up,
                extend: true,
            },
            &mut s,
            &ctx,
        );
    }
    assert_eq!(s.focus(), Some(c(3, 1)));
    assert_eq!(s.rects(), &[CellRect::spanning((8, 1), (6, 1))]);
    // Back to the source order: the focus is still model row 3.
    s.on_view_map_changed(&ViewMap::identity(10));
    assert_eq!(s.focus(), Some(c(3, 1)));
    assert_eq!(s.rects(), &[CellRect::spanning((3, 1), (3, 1))]);
}

#[test]
fn rects_clear_on_resort() {
    let ident = ViewMap::identity(10);
    let mut s = Selection::default();
    s.select_cell(c(0, 0), &ident);
    s.extend_to(c(3, 2), &ident);
    s.add_cell(c(8, 1), &ident);
    // A gutter row (model rows) and a header column (columns) survive a sort.
    s.add_row(ModelRow(5), ColId(0));
    s.add_column(ColId(2), ModelRow(5));
    assert_eq!(s.rects().len(), 2);

    let sorted = reversed(10);
    s.on_view_map_changed(&sorted);
    // Rectangles were view rows of the old order: gone. The focus stays (model row 5, which a
    // header click left the focus on) and the row and column sets stay.
    assert!(s.rects().is_empty(), "{:?}", s.rects());
    assert_eq!(s.focus(), Some(c(5, 2)));
    assert_eq!(s.rows(), &[5..6]);
    assert_eq!(s.cols(), &[2..3]);
    // Model row 5 is view row 4 now.
    assert!(s.contains(ViewRow(4), ColId(0), &sorted));
    assert!(!s.contains(ViewRow(5), ColId(0), &sorted));
    assert!(s.row_selected(ModelRow(5)));

    // A selection of rectangles only keeps its focus cell, selected alone where it now shows.
    let mut t = Selection::default();
    t.select_cell(c(1, 1), &ident);
    t.extend_to(c(4, 2), &ident);
    t.on_view_map_changed(&sorted);
    assert_eq!(t.focus(), Some(c(4, 2)));
    assert_eq!(t.rects(), &[CellRect::spanning((5, 2), (5, 2))]);
}

#[test]
fn a_focus_a_filter_hides_selects_nothing_but_stays() {
    let ident = ViewMap::identity(10);
    let mut s = Selection::default();
    s.select_cell(c(3, 0), &ident);
    let kept = ViewMap::filtered(Arc::from([1u64, 4, 6]), 10);
    s.on_view_map_changed(&kept);
    assert!(s.is_empty());
    assert_eq!(s.focus(), Some(c(3, 0)));
    // The first key lands on a shown row (clamped into the filter's three rows).
    let order = cols(2);
    let ctx = KeyCtx {
        map: &kept,
        order: &order,
        page_rows: 3,
    };
    apply_command(
        GridCommand::Move {
            dir: Dir::Down,
            extend: false,
        },
        &mut s,
        &ctx,
    );
    let f = s.focus().unwrap();
    assert!(kept.model_to_view(f.row).is_some(), "{f:?}");
}

#[test]
fn row_sets_copy_as_the_view_rows_that_show_them() {
    let order = cols(2);
    let mut s = Selection::default();
    s.select_row(ModelRow(2), ColId(0));
    s.add_row(ModelRow(3), ColId(0));
    s.add_row(ModelRow(7), ColId(0));
    let sorted = reversed(10);
    s.on_view_map_changed(&sorted);
    // Model rows 2, 3 and 7 are view rows 7, 6 and 2: copied in view order.
    assert_eq!(
        s.normalize_for_copy(&sorted, &order),
        Ok(CopyRegion {
            rows: vec![2..3, 6..8],
            cols: order.clone(),
        })
    );
    assert_eq!(s.cell_count_in(&sorted, &order), 6);
}

#[test]
fn shift_extending_rows_under_a_sort_takes_the_rows_as_shown() {
    let order = cols(3);
    let sorted = reversed(10);
    let mut s = Selection::default();
    // A gutter click on view row 1 (model row 8), then Shift+click on view row 3 (model row 6).
    s.select_row(ModelRow(8), ColId(0));
    s.extend_rows_to_in(ModelRow(6), &sorted);
    // View rows 1..=3 whole: model rows 8, 7 and 6, not the model range between them.
    let r = s.normalize_for_copy(&sorted, &order).unwrap();
    assert_eq!(r.rows, vec![1..4]);
    assert_eq!(r.cols, order);
    for (v, shown) in [(0, false), (1, true), (2, true), (3, true), (4, false)] {
        assert_eq!(s.contains(ViewRow(v), ColId(2), &sorted), shown, "view {v}");
    }
    // In the source order the same call is the model range (M3a).
    let ident = ViewMap::identity(10);
    let mut t = Selection::default();
    t.select_row(ModelRow(1), ColId(0));
    t.extend_rows_to_in(ModelRow(3), &ident);
    assert_eq!(t.rows(), &[1..4]);
}

#[test]
fn filtered_map_counts_only_kept_rows() {
    let order = cols(4);
    // The filter keeps model rows 1, 4, 6 and 9 of ten: view rows 0..4.
    let kept = ViewMap::filtered(Arc::from([1u64, 4, 6, 9]), 10);
    assert_eq!(kept.len(), 4);

    let mut all = Selection::default();
    all.select_all();
    assert_eq!(all.cell_count_in(&kept, &order), 16);
    let r = all.normalize_for_copy(&kept, &order).unwrap();
    assert_eq!(r.rows, vec![0..4]);

    // A column: the kept rows only.
    let mut col = Selection::default();
    col.select_column(ColId(1), ModelRow(4));
    assert_eq!(col.cell_count_in(&kept, &order), 4);

    // Rows chosen before the filter: only the kept ones count (model rows 4 and 6 of 3..=7).
    let ident = ViewMap::identity(10);
    let mut rows = Selection::default();
    rows.select_row(ModelRow(3), ColId(0));
    rows.extend_rows_to_in(ModelRow(7), &ident);
    rows.on_view_map_changed(&kept);
    assert_eq!(rows.cell_count_in(&kept, &order), 8);
    assert_eq!(
        rows.normalize_for_copy(&kept, &order).unwrap().rows,
        vec![1..3]
    );

    // A rectangle in view rows past the filter's rows is clipped to them.
    let mut rect = Selection::default();
    rect.select_cell(c(6, 0), &kept);
    rect.extend_to(c(9, 1), &kept);
    assert_eq!(rect.rects(), &[CellRect::spanning((2, 0), (3, 1))]);
    assert_eq!(rect.cell_count_in(&kept, &order), 4);

    // In the source order `cell_count_in` is M3a's `cell_count`.
    let mut s = Selection::default();
    s.select_cell(c(0, 0), &ident);
    s.extend_to(c(2, 2), &ident);
    s.add_row(ModelRow(5), ColId(0));
    assert_eq!(s.cell_count_in(&ident, &order), s.cell_count(10, &order));
}

/// A permutation of `n` rows by a seeded generator (SplitMix64, as Task 17's properties).
fn shuffled(n: u64, seed: u64) -> Vec<u64> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut v: Vec<u64> = (0..n).collect();
    for i in (1..v.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

#[test]
fn a_shuffled_map_round_trips_every_row() {
    let order = shuffled(5_000, 7);
    let m = ViewMap::permuted(Arc::from(order.clone()), 5_000, 5_100);
    for v in 0..5_100 {
        let model = m.view_to_model(ViewRow(v)).unwrap();
        assert_eq!(m.model_to_view(model), Some(ViewRow(v)));
    }
    assert_eq!(m.model_to_view(ModelRow(5_100)), Some(ViewRow(5_100)));
    // A map built by hand (the variant, with an index that never matched) still answers right.
    let ViewMap::Permuted { index, .. } = reversed(3) else {
        unreachable!()
    };
    let by_hand = ViewMap::Permuted {
        order: Arc::from([2u64, 0, 1]),
        tail_from: 3,
        loaded: 3,
        index,
    };
    assert_eq!(by_hand.model_to_view(ModelRow(2)), Some(ViewRow(0)));
    assert_eq!(by_hand.model_to_view(ModelRow(1)), Some(ViewRow(2)));
}

#[test]
#[ignore = "perf"]
fn permuted_map_lookup_under_50ns() {
    if std::env::var_os("HACIVAT_PERF").is_none_or(|v| v != "1") {
        return;
    }
    const N: u64 = 1_000_000;
    let order = shuffled(N, 35);
    let probes = shuffled(N, 36);
    let m = ViewMap::permuted(Arc::from(order), N, N);
    // Build the index outside the timing (`ViewMap::permuted` builds it eagerly).
    assert!(m.model_to_view(ModelRow(0)).is_some());
    let started = std::time::Instant::now();
    let mut sum = 0u64;
    for &p in &probes {
        let v = m.model_to_view(ModelRow(p)).map_or(0, |v| v.0);
        let back = m.view_to_model(ViewRow(v)).map_or(0, |r| r.0);
        sum = sum.wrapping_add(v ^ back);
    }
    let per = started.elapsed().as_nanos() as f64 / N as f64;
    println!("permuted map: model→view→model {per:.1} ns per row (sum {sum})");
    assert!(per < 50.0, "{per:.1} ns per lookup");
}
