//! M3a Task 17: the horizontal column layout (HV-GRID-02 step 6): `f64` prefix sums over the
//! visible columns in view order, suffix-only width updates, hide and show anchored to the first
//! visible column.

use hacivat_ui_grid::model::ColumnLayout;
use hacivat_ui_grid::{Align, ColId, ColumnKind, ColumnSpec};

fn layout(widths: &[f32]) -> ColumnLayout {
    ColumnLayout::new(
        widths
            .iter()
            .enumerate()
            .map(|(i, w)| (ColId(i as u32), *w)),
    )
}

fn visible_cols(widths: &[f32], left_x: f64, width: f32) -> std::ops::Range<usize> {
    layout(widths).visible_range(left_x, width)
}

/// A zero-width body: `hide`/`show` then clamp only to the content.
const NO_BODY: f32 = 0.0;

fn edges(l: &ColumnLayout) -> Vec<f64> {
    (0..=l.len()).map(|i| l.x_of(i)).collect()
}

#[test]
fn visible_cols_from_prefix_sums() {
    assert_eq!(
        visible_cols(&[100.0, 200.0, 50.0, 300.0], 250.0, 300.0),
        1..4
    );
}

#[test]
fn visible_range_edges() {
    let w = [100.0, 200.0, 50.0, 300.0];
    // A column ending exactly at left_x is not visible; one starting at the right edge is not.
    assert_eq!(visible_cols(&w, 300.0, 50.0), 2..3);
    assert_eq!(visible_cols(&w, 0.0, 100.0), 0..1);
    assert_eq!(visible_cols(&w, 0.0, 100.5), 0..2);
    assert_eq!(visible_cols(&w, 0.0, 10_000.0), 0..4);
    assert_eq!(visible_cols(&w, 640.0, 300.0), 3..4);
    assert_eq!(visible_cols(&w, 650.0, 300.0), 4..4, "past the content");
    assert_eq!(visible_cols(&w, 0.0, 0.0), 0..0, "no width, no columns");
    assert_eq!(visible_cols(&[], 0.0, 300.0), 0..0);
}

#[test]
fn prefix_sums_are_f64_and_exact_for_many_columns() {
    let l = layout(&vec![123.25; 100_000]);
    assert_eq!(l.len(), 100_000);
    assert_eq!(l.total_width(), 12_325_000.0);
    assert_eq!(l.x_of(99_999), 12_324_876.75);
    // An f32 running sum of the same widths is already wrong by many pixels.
    let f32_sum = (0..100_000).fold(0f32, |acc, _| acc + 123.25);
    assert_ne!(f64::from(f32_sum), l.total_width());
    let mid = l.visible_range(6_000_000.0, 960.0);
    assert_eq!(mid.start, (6_000_000.0 / 123.25) as usize);
    assert_eq!(l.col_at_x(12_324_900.0), Some(99_999));
}

#[test]
fn width_change_updates_only_the_suffix() {
    let mut l = layout(&[100.0, 200.0, 50.0, 300.0, 80.0]);
    let before = edges(&l);
    assert!(l.set_width(ColId(2), 75.0));
    let after = edges(&l);
    assert_eq!(
        &after[..=2],
        &before[..=2],
        "edges up to the changed column stay"
    );
    for i in 3..after.len() {
        assert_eq!(after[i], before[i] + 25.0, "edge {i} moves by the change");
    }
    assert_eq!(l.width_of(ColId(2)), Some(75.0));
    assert_eq!(l.total_width(), 755.0);
    assert!(
        !l.set_width(ColId(9), 10.0),
        "an unknown column changes nothing"
    );
    assert!(
        !l.set_width(ColId(1), f32::NAN),
        "a non-finite width changes nothing"
    );
    assert!(
        !l.set_width(ColId(1), -4.0),
        "a negative width changes nothing"
    );
}

#[test]
fn hit_testing_through_the_inverse_prefix_sums() {
    let l = layout(&[100.0, 200.0, 50.0, 300.0]);
    assert_eq!(l.col_at_x(0.0), Some(0));
    assert_eq!(l.col_at_x(99.9), Some(0));
    assert_eq!(l.col_at_x(100.0), Some(1));
    assert_eq!(l.col_at_x(649.0), Some(3));
    assert_eq!(l.col_at_x(650.0), None);
    assert_eq!(l.col_at_x(-1.0), None);
    assert_eq!(l.col_at_x(f64::NAN), None);
}

#[test]
fn hide_keeps_left_x_anchored_to_the_first_visible_column() {
    let mut l = layout(&[100.0, 200.0, 50.0, 300.0]);
    // left_x 320 sits 20 px into column 2.
    let x = l.hide(ColId(1), 320.0, NO_BODY);
    assert_eq!(x, 120.0, "column 2 stays where it was on screen");
    assert_eq!(l.visible(), &[ColId(0), ColId(2), ColId(3)]);
    assert_eq!(l.hidden(), &[ColId(1)]);
    assert_eq!(l.order(), &[ColId(0), ColId(1), ColId(2), ColId(3)]);
    assert_eq!(l.index_of(ColId(3)), Some(2));
    assert_eq!(l.index_of(ColId(1)), None);
    assert_eq!(
        l.width_of(ColId(1)),
        Some(200.0),
        "a hidden column keeps its width"
    );
    assert_eq!(l.total_width(), 450.0);

    // Showing it again puts the view back.
    let x = l.show(ColId(1), x, NO_BODY);
    assert_eq!(x, 320.0);
    assert_eq!(l.visible(), &[ColId(0), ColId(1), ColId(2), ColId(3)]);
    assert!(l.hidden().is_empty());

    // Hiding a column after the anchor moves nothing.
    assert_eq!(l.hide(ColId(3), 320.0, NO_BODY), 320.0);
    let x = l.show_all(320.0, NO_BODY);
    assert_eq!(x, 320.0);
}

#[test]
fn hiding_the_anchor_column_anchors_to_the_next_one() {
    let mut l = layout(&[100.0, 200.0, 50.0, 300.0]);
    // left_x 320 is inside column 2; hiding it leaves column 3 at its old left edge.
    assert_eq!(l.hide(ColId(2), 320.0, NO_BODY), 300.0);
    assert_eq!(l.x_of(l.index_of(ColId(3)).unwrap()), 300.0);
    // Hiding the last visible column anchors to the one before it.
    let mut l = layout(&[100.0, 200.0]);
    assert_eq!(l.hide(ColId(1), 150.0, NO_BODY), 100.0);
}

#[test]
fn at_the_left_edge_a_shown_column_stays_in_view() {
    let mut l = layout(&[100.0, 200.0, 50.0]);
    l.hide(ColId(0), 0.0, NO_BODY);
    assert_eq!(l.show(ColId(0), 0.0, NO_BODY), 0.0);
}

#[test]
fn hide_and_show_clamp_left_x_to_the_new_content() {
    // Widths 100, 200, 50, 300 (650 px) in a 300 px body; left_x 340 is 40 px into column 2.
    let mut l = layout(&[100.0, 200.0, 50.0, 300.0]);
    // Hiding a column before the anchor: 140, inside the new range [0, 150].
    assert_eq!(l.hide(ColId(1), 340.0, 300.0), 140.0);
    l.show(ColId(1), 140.0, 300.0);
    // Hiding the last column leaves 350 px of content: the anchored 340 clamps to 50.
    assert_eq!(l.hide(ColId(3), 340.0, 300.0), 50.0);
    // Content narrower than the body: 0.
    assert_eq!(l.hide(ColId(2), 50.0, 300.0), 0.0);
    assert_eq!(l.show_all(0.0, 300.0), 0.0);
    assert_eq!(l.total_width(), 650.0);
    // A body width that is not finite clamps only to the content.
    assert_eq!(l.hide(ColId(1), 340.0, f32::NAN), 140.0);
}

#[test]
fn hidden_columns_leave_the_prefix_sums() {
    let mut l = layout(&[100.0, 200.0, 50.0, 300.0]);
    l.hide(ColId(1), 0.0, NO_BODY);
    assert_eq!(edges(&l), vec![0.0, 100.0, 150.0, 450.0]);
    assert_eq!(l.visible_range(120.0, 100.0), 1..3);
    assert!(
        l.set_width(ColId(1), 10.0),
        "a hidden column's width can change"
    );
    assert_eq!(edges(&l), vec![0.0, 100.0, 150.0, 450.0]);
    l.show(ColId(1), 0.0, NO_BODY);
    assert_eq!(edges(&l), vec![0.0, 100.0, 110.0, 160.0, 460.0]);
    assert!(!l.is_empty());
    let empty = layout(&[]);
    assert!(empty.is_empty());
    assert_eq!(empty.total_width(), 0.0);
    assert_eq!(empty.x_of(0), 0.0);
}

#[test]
fn from_specs_takes_the_default_widths() {
    let spec = |i: u32, w: f32| ColumnSpec {
        id: ColId(i),
        name: format!("c{i}").into(),
        type_label: "int8".into(),
        kind: ColumnKind::Number,
        align: Align::End,
        default_width: w,
        key: false,
    };
    let l = ColumnLayout::from_specs(&[spec(0, 96.0), spec(1, 160.0)]);
    assert_eq!(l.visible(), &[ColId(0), ColId(1)]);
    assert_eq!(l.total_width(), 256.0);
    assert_eq!(l.width_at(1), 160.0);
}
