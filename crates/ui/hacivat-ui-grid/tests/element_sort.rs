//! M3c Task 36: the grid's side of a local sort (NOTES surface 18). The header's indicator
//! after a sorted column's name (at the end of an end-aligned header), the level numbers with
//! two or more keys only (M3c-19), the indicator's room taken from the name, the dashed
//! boundary above the unsorted tail, a re-run clearing both, and `request_sort` telling the
//! host. Needs `--features test-support` (the fixture sources); without it the file is empty.
#![cfg(feature = "test-support")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, SharedString, TestAppContext, VisualTestContext, size};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::model::{SortKey, SortSpec, ViewMap};
use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config, text_column};
use hacivat_ui_grid::{Align, CellKind, ColId, GridDataSource, GridEvent, GridView};

type Events = Rc<RefCell<Vec<GridEvent>>>;

/// Three columns: a short name, a long one (end-aligned), a third.
fn source(rows: u64) -> FixtureSource {
    let mut long = text_column(1, "a_rather_long_column_name_here", 140.0);
    long.align = Align::End;
    FixtureSource::generated(3, rows, Some(rows), false, |r, c| {
        cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
    })
    .with_columns(vec![
        text_column(0, "id", 120.0),
        long,
        text_column(2, "c2", 120.0),
    ])
}

fn open(
    cx: &mut TestAppContext,
    source: Rc<dyn GridDataSource>,
) -> (Entity<GridView>, &mut VisualTestContext, Events) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (view, vcx) =
        cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
    vcx.simulate_resize(size(gpui::px(800.0), gpui::px(400.0)));
    vcx.run_until_parked();
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &GridEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach()
    });
    (view, vcx, events)
}

fn frame(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

fn tip(text: &'static str) -> TextFn {
    Rc::new(move |_| SharedString::from(text))
}

fn set_sort(view: &Entity<GridView>, vcx: &mut VisualTestContext, spec: SortSpec) {
    let tips = spec.keys.iter().map(|_| tip("level")).collect();
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_sort(spec, tips, cx)));
    frame(vcx);
}

/// The left and right of visible column `ix` in the header, window x.
fn column_span(view: &Entity<GridView>, vcx: &mut VisualTestContext, ix: usize) -> (f32, f32) {
    view.read_with(vcx, |v, _| {
        let g = v.geometry().expect("a frame was painted");
        let layout = v.column_layout();
        let left = f32::from(g.header.left()) + layout.x_of(ix) as f32;
        (left, left + layout.width_at(ix))
    })
}

#[gpui::test]
fn the_indicator_follows_the_name_and_ends_an_end_aligned_header(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(source(5)));
    set_sort(&view, vcx, SortSpec::by(ColId(0), false));
    let marks = view.read_with(vcx, |v, _| v.painted_sort_marks().to_vec());
    assert_eq!(marks.len(), 1);
    let (left, right) = column_span(&view, vcx, 0);
    // After "id" (10 px of padding, the name, a 6 px gap): inside the column, left half.
    assert_eq!(
        (marks[0].col, marks[0].level, marks[0].desc),
        (ColId(0), 0, false)
    );
    assert!(
        marks[0].x > left + 10.0 && marks[0].x < left + 60.0,
        "{marks:?}"
    );
    assert!(marks[0].x + 12.0 <= right - 10.0 + 0.5);
    assert_eq!(marks[0].number, None);

    // The long end-aligned name: the arrow ends the header (12 px at the padding), and the
    // name gives way to it (ellipsized; its tooltip carries the whole name and the sort).
    set_sort(&view, vcx, SortSpec::by(ColId(1), true));
    let marks = view.read_with(vcx, |v, _| v.painted_sort_marks().to_vec());
    let (_, right) = column_span(&view, vcx, 1);
    assert_eq!(marks.len(), 1);
    assert!(
        (marks[0].x - (right - 10.0 - 12.0)).abs() < 0.5,
        "{marks:?}"
    );
    assert!(marks[0].desc);
    let tips = view.read_with(vcx, |v, _| v.header_tips().to_vec());
    let tip = tips.iter().find(|t| t.col == ColId(1)).expect("a tooltip");
    assert!(tip.ellipsized);
    assert_eq!(tip.sort, Some(0));
    // An unsorted, unfrozen, whole name has no tooltip.
    assert!(!tips.iter().any(|t| t.col == ColId(2)));
}

#[gpui::test]
fn level_numbers_only_with_two_or_more_keys(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(source(5)));
    let mut spec = SortSpec::by(ColId(2), false);
    spec.keys.push(SortKey {
        col: ColId(0),
        desc: true,
    });
    set_sort(&view, vcx, spec);
    let mut marks = view.read_with(vcx, |v, _| v.painted_sort_marks().to_vec());
    marks.sort_by_key(|m| m.level);
    let summary: Vec<_> = marks
        .iter()
        .map(|m| (m.col, m.level, m.desc, m.number.clone()))
        .collect();
    assert_eq!(
        summary,
        [
            (ColId(2), 0, false, Some("1".into())),
            (ColId(0), 1, true, Some("2".into())),
        ]
    );
    // Clearing the sort clears the header.
    set_sort(&view, vcx, SortSpec::default());
    assert!(view.read_with(vcx, |v, _| v.painted_sort_marks().is_empty()));
}

#[gpui::test]
fn the_tail_boundary_sits_above_the_first_unsorted_row(cx: &mut TestAppContext) {
    let fixture = source(6);
    let (view, vcx, _) = open(cx, Rc::new(fixture.clone()));
    let order: Arc<[u64]> = Arc::from([5u64, 4, 3, 2, 1, 0]);
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_view_map(ViewMap::permuted(order, 6, 6), cx);
            v.set_sort(SortSpec::by(ColId(0), true), vec![tip("level")], cx);
        })
    });
    frame(vcx);
    assert_eq!(view.read_with(vcx, |v, _| v.tail_boundary()), None);
    assert_eq!(view.read_with(vcx, |v, _| v.painted_tail_y()), None);
    // Four rows arrive after the sort: they join the tail, behind the line.
    fixture.set_count(10, false, Some(10));
    frame(vcx);
    assert_eq!(view.read_with(vcx, |v, _| v.tail_boundary()), Some(6));
    let (y, expected) = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        (
            v.painted_tail_y(),
            f32::from(g.body.top()) + v.scroll_state().row_top(6) as f32,
        )
    });
    assert_eq!(y, Some(expected));
    // A re-run drops the order and the indicator (the host sorts again only when asked).
    fixture.rerun(10, false, Some(10));
    frame(vcx);
    assert!(view.read_with(vcx, |v, _| v.view_map().is_identity()));
    assert!(view.read_with(vcx, |v, _| v.sort().is_empty()));
    assert_eq!(view.read_with(vcx, |v, _| v.tail_boundary()), None);
    assert!(view.read_with(vcx, |v, _| v.painted_sort_marks().is_empty()));
}

#[gpui::test]
fn request_sort_tells_the_host(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(source(3)));
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.request_sort(SortSpec::by(ColId(1), false), cx)
        })
    });
    vcx.run_until_parked();
    assert!(
        events
            .borrow()
            .contains(&GridEvent::SortRequested(SortSpec::by(ColId(1), false)))
    );
    // Asking changes nothing by itself: the host publishes the order.
    assert!(view.read_with(vcx, |v, _| v.view_map().is_identity()));
    assert!(view.read_with(vcx, |v, _| v.sort().is_empty()));
}
