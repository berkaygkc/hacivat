//! M3c Task 35: frozen-left columns in the column layout (HV-GRID-02 frozen): the first `n`
//! visible columns keep their place on screen while the rest scroll under them.

use hacivat_ui_grid::ColId;
use hacivat_ui_grid::model::ColumnLayout;

fn layout(widths: &[f32]) -> ColumnLayout {
    ColumnLayout::new(
        widths
            .iter()
            .enumerate()
            .map(|(i, w)| (ColId(i as u32), *w)),
    )
}

#[test]
fn frozen_columns_never_scroll() {
    let mut l = layout(&[100.0, 50.0, 200.0, 200.0, 200.0, 200.0]);
    l.set_frozen(2);
    assert_eq!(l.frozen(), 2);
    assert_eq!(l.frozen_width(), 150.0);
    assert_eq!(l.scroll_width(), 800.0);
    for left_x in [0.0, 37.5, 300.0, 1_000.0] {
        // Frozen columns sit at their own edges whatever the scroll.
        assert_eq!(l.screen_x(0, left_x), 0.0);
        assert_eq!(l.screen_x(1, left_x), 100.0);
        // The rest move with it.
        assert_eq!(l.screen_x(2, left_x), 150.0 - left_x);
    }
    // A body 450 px wide shows 300 px of scrolling columns right of the 150 frozen ones.
    assert_eq!(l.scrolling_range(0.0, 300.0), 2..4);
    assert_eq!(l.scrolling_range(250.0, 300.0), 3..5);
    assert_eq!(l.scrolling_range(500.0, 300.0), 4..6);
    // Hit testing: under the frozen pane is a frozen column; right of it, a scrolled one.
    assert_eq!(l.col_at_screen_x(20.0, 250.0), Some(0));
    assert_eq!(l.col_at_screen_x(149.0, 250.0), Some(1));
    assert_eq!(l.col_at_screen_x(150.0, 250.0), Some(3));
    assert_eq!(l.col_at_screen_x(450.0, 250.0), Some(4));
    assert_eq!(l.col_at_screen_x(-1.0, 250.0), None);
}

#[test]
fn no_frozen_columns_is_the_m3a_layout() {
    let l = layout(&[100.0, 50.0, 200.0]);
    assert_eq!(l.frozen(), 0);
    assert_eq!(l.frozen_width(), 0.0);
    assert_eq!(l.scroll_width(), l.total_width());
    assert_eq!(l.scrolling_range(60.0, 100.0), l.visible_range(60.0, 100.0));
    assert_eq!(l.screen_x(2, 60.0), 150.0 - 60.0);
    assert_eq!(l.col_at_screen_x(30.0, 60.0), l.col_at_x(90.0));
}

#[test]
fn freezing_clamps_to_the_visible_columns_and_follows_hiding() {
    let mut l = layout(&[100.0, 50.0, 200.0]);
    l.set_frozen(9);
    assert_eq!(l.frozen(), 3);
    assert_eq!(l.scroll_width(), 0.0);
    l.set_frozen(2);
    // Hiding a frozen column leaves the remaining visible ones frozen up to the count.
    let _ = l.hide(ColId(0), 0.0, 400.0);
    assert_eq!(l.visible(), &[ColId(1), ColId(2)]);
    assert_eq!(l.frozen(), 2);
    assert_eq!(l.frozen_width(), 250.0);
    l.set_frozen(0);
    assert_eq!(l.frozen(), 0);
    // A width change inside the frozen columns moves the pane's edge.
    l.set_frozen(1);
    assert!(l.set_width(ColId(1), 80.0));
    assert_eq!(l.frozen_width(), 80.0);
    assert_eq!(l.screen_x(1, 500.0), 80.0 - 500.0);
}
