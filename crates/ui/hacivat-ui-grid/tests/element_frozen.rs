//! M3c Task 35: the grid element and view over the M3c model: frozen columns in their own
//! layer, a permuted view map painted in view order, ⌘/Ctrl-click and -drag ranges, grow and
//! shrink from the on-screen columns, auto-fit, and edge autoscroll while the pointer is still.
//! Needs `--features test-support` (the fixture sources); without it the file is empty.
#![cfg(feature = "test-support")]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Point, TestAppContext,
    VisualTestContext, point, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::model::{CellRect, ViewMap};
use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config};
use hacivat_ui_grid::{
    Align, CellKind, ColId, GridCommand, GridDataSource, GridEvent, GridView, ModelCell, ModelRow,
};

type Events = Rc<RefCell<Vec<GridEvent>>>;

fn numbered(cols: u32, rows: u64) -> FixtureSource {
    FixtureSource::generated(cols, rows, Some(rows), false, |r, c| {
        cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
    })
}

fn open(
    cx: &mut TestAppContext,
    source: Rc<dyn GridDataSource>,
    w: f32,
    h: f32,
) -> (Entity<GridView>, &mut VisualTestContext, Events) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (view, vcx) =
        cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
    vcx.simulate_resize(size(px(w), px(h)));
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

fn run(view: &Entity<GridView>, vcx: &mut VisualTestContext, cmd: GridCommand) {
    vcx.update(|window, cx| view.update(cx, |v, cx| v.dispatch(cmd, window, cx)));
    vcx.run_until_parked();
}

fn frame(vcx: &mut VisualTestContext) {
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
}

/// The left edge each painted column of view row `row` was drawn at, by column.
fn lefts(view: &Entity<GridView>, vcx: &mut VisualTestContext, row: u64) -> Vec<(u32, f32)> {
    view.read_with(vcx, |v, _| {
        let mut out: Vec<(u32, f32)> = v
            .painted_cells()
            .iter()
            .filter(|c| c.row == row)
            .map(|c| (c.col.0, c.left))
            .collect();
        out.sort_by_key(|(c, _)| *c);
        out
    })
}

/// A point inside the body at view row `row`, `x` px right of the body's left edge.
fn body_point(
    view: &Entity<GridView>,
    vcx: &mut VisualTestContext,
    row: u64,
    x: f32,
) -> Point<gpui::Pixels> {
    view.read_with(vcx, |v, _| {
        let g = v.geometry().expect("painted");
        let s = v.scroll_state();
        point(
            px(f32::from(g.body.left()) + x),
            px(f32::from(g.body.top()) + s.row_top(row) as f32 + 10.0),
        )
    })
}

fn mc(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

#[gpui::test]
fn freeze_keeps_the_columns_up_to_the_focus_in_place(cx: &mut TestAppContext) {
    // Six 120 px columns in a body about 440 px wide.
    let (view, vcx, events) = open(cx, Rc::new(numbered(6, 50)), 500.0, 300.0);
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Right,
            extend: false,
        },
    );
    // The first key focused (0, 0); Right moves to column 1.
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Right,
            extend: false,
        },
    );
    run(&view, vcx, GridCommand::Freeze);
    assert_eq!(view.read_with(vcx, |v, _| v.column_layout().frozen()), 2);
    assert!(
        events.borrow().contains(&GridEvent::FrozenChanged(2)),
        "{:?}",
        events.borrow()
    );
    view.update(vcx, |v, _| v.set_paint_recording(true));
    frame(vcx);
    let before = lefts(&view, vcx, 0);
    view.update(vcx, |v, cx| v.scroll_px(0.0, 150.0, cx));
    frame(vcx);
    let after = lefts(&view, vcx, 0);
    let left_of = |cells: &[(u32, f32)], col: u32| {
        cells
            .iter()
            .find(|(c, _)| *c == col)
            .map(|(_, x)| *x)
            .unwrap_or_else(|| panic!("column {col} not painted: {cells:?}"))
    };
    // The frozen columns did not move; column 3 moved 150 px to the left, and column 2 went
    // wholly under the frozen ones, so it is not painted at all.
    assert_eq!(left_of(&before, 0), left_of(&after, 0));
    assert_eq!(left_of(&before, 1), left_of(&after, 1));
    assert_eq!(left_of(&before, 3) - 150.0, left_of(&after, 3));
    assert!(before.iter().any(|(c, _)| *c == 2));
    assert!(after.iter().all(|(c, _)| *c != 2), "{after:?}");
    let geometry = view.read_with(vcx, |v, _| v.geometry().cloned().unwrap());
    assert_eq!(geometry.frozen, 2);
    assert_eq!(geometry.frozen_w, 240.0);
    assert_eq!(geometry.cols, 3..5);

    // A click left of the pane's edge lands in a frozen column whatever the scroll.
    let at = body_point(&view, vcx, 3, 130.0);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selection().focus()),
        Some(mc(3, 1))
    );
    // And right of it, in the scrolled column under the pointer (x 250 + 150 scrolled: col 3).
    let at = body_point(&view, vcx, 3, 250.0);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selection().focus()),
        Some(mc(3, 3))
    );

    // Moving to a frozen column never scrolls; to a scrolled column hidden under the pane, it
    // scrolls it out from under the pane.
    let left_x = view.read_with(vcx, |v, _| v.scroll_state().left_x);
    run(&view, vcx, GridCommand::Home { extend: false });
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().left_x), left_x);
    run(&view, vcx, GridCommand::End { extend: false });
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    // Column 2 starts at content x 240, right at the pane's edge: fully shown.
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().left_x), 0.0);

    events.borrow_mut().clear();
    run(&view, vcx, GridCommand::Unfreeze);
    assert_eq!(view.read_with(vcx, |v, _| v.column_layout().frozen()), 0);
    assert_eq!(events.borrow().as_slice(), &[GridEvent::FrozenChanged(0)]);
    // Unfreezing again changes nothing and says nothing.
    run(&view, vcx, GridCommand::Unfreeze);
    assert_eq!(events.borrow().len(), 1);
}

#[gpui::test]
fn freeze_refuses_to_leave_no_room_to_scroll(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(numbered(6, 20)), 400.0, 300.0);
    // Focus column 3: 480 px of frozen columns in a body narrower than that. (The first key
    // only focuses the first cell.)
    run(&view, vcx, GridCommand::End { extend: false });
    run(&view, vcx, GridCommand::End { extend: false });
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    run(&view, vcx, GridCommand::Freeze);
    assert_eq!(view.read_with(vcx, |v, _| v.column_layout().frozen()), 0);
    assert!(
        events
            .borrow()
            .iter()
            .all(|e| !matches!(e, GridEvent::FrozenChanged(_)))
    );
    // The host may set a count that later stops fitting (a narrower pane): the frame freezes
    // only what fits, and the layout keeps the count.
    view.update(vcx, |v, cx| v.set_frozen(5, cx));
    frame(vcx);
    let g = view.read_with(vcx, |v, _| v.geometry().cloned().unwrap());
    assert!(
        g.frozen < 5 && g.frozen_w < f32::from(g.body.size.width),
        "{g:?}"
    );
    assert_eq!(view.read_with(vcx, |v, _| v.column_layout().frozen()), 5);
}

#[gpui::test]
fn a_permuted_map_paints_the_model_rows_in_view_order(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(2, 20)), 500.0, 300.0);
    // Focus model row 3, then sort the rows in reverse.
    for _ in 0..4 {
        run(
            &view,
            vcx,
            GridCommand::Move {
                dir: hacivat_ui_grid::model::Dir::Down,
                extend: false,
            },
        );
    }
    assert_eq!(
        view.read_with(vcx, |v, _| v.selection().focus()),
        Some(mc(3, 0))
    );
    let sorted = ViewMap::permuted(Arc::from((0..20u64).rev().collect::<Vec<_>>()), 20, 20);
    view.update(vcx, |v, cx| v.set_view_map(sorted.clone(), cx));
    view.update(vcx, |v, _| v.set_paint_recording(true));
    frame(vcx);
    let texts = view.read_with(vcx, |v, _| {
        v.painted_cells()
            .iter()
            .filter(|c| c.col == ColId(0) && c.row < 3)
            .map(|c| (c.row, c.text.to_string()))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        texts,
        vec![
            (0, "r19c0".to_string()),
            (1, "r18c0".to_string()),
            (2, "r17c0".to_string())
        ]
    );
    // The focus is the same data, selected where it shows now (view row 16).
    view.read_with(vcx, |v, _| {
        assert_eq!(v.selection().focus(), Some(mc(3, 0)));
        assert_eq!(
            v.selection().rects(),
            &[CellRect::spanning((16, 0), (16, 0))]
        );
    });
    // A click on view row 1 focuses model row 18.
    let at = body_point(&view, vcx, 1, 20.0);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.selection().focus()),
        Some(mc(18, 0))
    );
    // The identity goes back to the source order.
    view.update(vcx, |v, cx| v.set_view_map(ViewMap::identity(0), cx));
    frame(vcx);
    assert_eq!(
        view.read_with(vcx, |v, _| v.view_map().clone()),
        ViewMap::identity(20)
    );
}

#[gpui::test]
fn a_rerun_drops_the_view_map(cx: &mut TestAppContext) {
    let source = Rc::new(numbered(2, 20));
    let (view, vcx, _) = open(cx, source.clone(), 500.0, 300.0);
    let sorted = ViewMap::permuted(Arc::from((0..20u64).rev().collect::<Vec<_>>()), 20, 20);
    view.update(vcx, |v, cx| v.set_view_map(sorted, cx));
    frame(vcx);
    // Rows that arrive after the sort extend the map's tail.
    source.set_count(25, false, Some(25));
    frame(vcx);
    let m = view.read_with(vcx, |v, _| v.view_map().clone());
    assert_eq!(m.len(), 25);
    assert_eq!(
        m.view_to_model(hacivat_ui_grid::ViewRow(24)),
        Some(ModelRow(24))
    );
    // A re-run is new data: the old order means nothing for it.
    source.rerun(20, false, Some(20));
    frame(vcx);
    assert_eq!(
        view.read_with(vcx, |v, _| v.view_map().clone()),
        ViewMap::identity(20)
    );
}

#[gpui::test]
fn cmd_click_and_drag_add_ranges(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(4, 50)), 600.0, 400.0);
    let a = body_point(&view, vcx, 1, 20.0);
    vcx.simulate_click(a, Modifiers::none());
    vcx.run_until_parked();
    // ⌘/Ctrl-drag from (4, 1) to (6, 2) adds a second range.
    let from = body_point(&view, vcx, 4, 140.0);
    let to = body_point(&view, vcx, 6, 260.0);
    let add = vcx.update(|_, cx| Modifiers::secondary_key(cx));
    vcx.simulate_mouse_down(from, MouseButton::Left, add);
    vcx.simulate_mouse_move(to, Some(MouseButton::Left), add);
    vcx.simulate_mouse_up(to, MouseButton::Left, add);
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| {
        let s = v.selection();
        assert_eq!(s.ranges(), 2);
        assert_eq!(
            s.rects(),
            &[
                CellRect::spanning((1, 0), (1, 0)),
                CellRect::spanning((4, 1), (6, 2))
            ]
        );
        assert_eq!(s.focus(), Some(mc(6, 2)));
    });
}

#[gpui::test]
fn grow_uses_the_columns_on_screen_as_the_row_segment(cx: &mut TestAppContext) {
    // Eight 120 px columns; about three and a half on screen.
    let (view, vcx, _) = open(cx, Rc::new(numbered(8, 30)), 500.0, 300.0);
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Right,
            extend: false,
        },
    );
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Down,
            extend: false,
        },
    );
    run(&view, vcx, GridCommand::GrowSelection);
    let cols = view.read_with(vcx, |v, _| v.geometry().unwrap().cols.clone());
    view.read_with(vcx, |v, _| {
        let s = v.selection();
        assert_eq!(s.focus(), Some(mc(1, 0)));
        assert_eq!(
            s.rects(),
            &[CellRect {
                rows: 1..2,
                cols: cols.start as u32..cols.end as u32
            }]
        );
    });
    run(&view, vcx, GridCommand::GrowSelection);
    assert_eq!(
        view.read_with(vcx, |v, _| v.selection().rows().to_vec()),
        vec![1..2]
    );
    run(&view, vcx, GridCommand::GrowSelection);
    assert!(view.read_with(vcx, |v, _| v.selection().is_all()));
    run(&view, vcx, GridCommand::ShrinkSelection);
    run(&view, vcx, GridCommand::ShrinkSelection);
    run(&view, vcx, GridCommand::ShrinkSelection);
    view.read_with(vcx, |v, _| {
        assert_eq!(v.selection().rects(), &[CellRect::spanning((1, 0), (1, 0))]);
    });
}

/// A source whose column 1 holds long values.
fn long_column() -> FixtureSource {
    FixtureSource::generated(3, 30, Some(30), false, |r, c| {
        // Every value of column 1 is long (one long value alone is an outlier the fit trims).
        let text = if c.0 == 1 {
            format!("a considerably longer value than the column shows, row {r}")
        } else {
            format!("r{r}")
        };
        cell(text, CellKind::Text, Align::Start)
    })
}

#[gpui::test]
fn double_click_on_a_header_edge_fits_the_column(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(long_column()), 700.0, 300.0);
    let (edge, before) = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        let l = v.column_layout();
        let x = f32::from(g.body.left()) + (l.x_of(2) - v.scroll_state().left_x) as f32;
        let y = f32::from(g.header.top()) + 10.0;
        (point(px(x), px(y)), l.width_at(1))
    });
    for (click_count, first) in [(1, true), (2, false)] {
        vcx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: edge,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: first,
        });
        vcx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: edge,
            modifiers: Modifiers::none(),
            click_count,
        });
    }
    vcx.run_until_parked();
    frame(vcx);
    let after = view.read_with(vcx, |v, _| v.column_layout().width_at(1));
    assert!(after > before + 100.0, "{before} -> {after}");
    let resized: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            GridEvent::ColumnResized { col, width } => Some((*col, *width)),
            _ => None,
        })
        .collect();
    assert_eq!(resized.last(), Some(&(ColId(1), after)), "{resized:?}");
}

#[gpui::test]
fn auto_fit_fits_the_focus_column(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(long_column()), 700.0, 300.0);
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Right,
            extend: false,
        },
    );
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Right,
            extend: false,
        },
    );
    let before = view.read_with(vcx, |v, _| v.column_layout().width_at(1));
    run(&view, vcx, GridCommand::AutoFit);
    frame(vcx);
    let after = view.read_with(vcx, |v, _| v.column_layout().width_at(1));
    assert!(after > before + 100.0, "{before} -> {after}");
    assert!(events.borrow().contains(&GridEvent::ColumnResized {
        col: ColId(1),
        width: after
    }));
    // A short column fits down to what it holds (never under the narrowest width).
    run(
        &view,
        vcx,
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Left,
            extend: false,
        },
    );
    run(&view, vcx, GridCommand::AutoFit);
    frame(vcx);
    let w0 = view.read_with(vcx, |v, _| v.column_layout().width_at(0));
    assert!((48.0..120.0).contains(&w0), "{w0}");
}

#[gpui::test]
fn edge_autoscroll_while_still(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(3, 500)), 500.0, 300.0);
    let start = body_point(&view, vcx, 2, 20.0);
    let below = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        point(
            px(f32::from(g.body.left()) + 20.0),
            g.body.bottom() + px(30.0),
        )
    });
    vcx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::none());
    vcx.run_until_parked();
    let top_after_move = view.read_with(vcx, |v, _| v.scroll_state().top_row);
    let focus_after_move = view.read_with(vcx, |v, _| v.selection().focus().unwrap().row.0);
    // The pointer stays still past the bottom edge: the grid keeps scrolling and extending.
    vcx.executor().advance_clock(Duration::from_millis(200));
    vcx.run_until_parked();
    let top_still = view.read_with(vcx, |v, _| v.scroll_state().top_row);
    let focus_still = view.read_with(vcx, |v, _| v.selection().focus().unwrap().row.0);
    assert!(
        top_still > top_after_move,
        "{top_after_move} -> {top_still}"
    );
    assert!(
        focus_still > focus_after_move,
        "{focus_after_move} -> {focus_still}"
    );
    view.read_with(vcx, |v, _| {
        let r = &v.selection().rects()[0];
        assert_eq!(r.rows.start, 2);
        assert_eq!(r.rows.end, focus_still + 1);
    });
    // Back inside the body: it stops.
    let inside = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        point(px(f32::from(g.body.left()) + 20.0), g.body.top() + px(50.0))
    });
    vcx.simulate_mouse_move(inside, Some(MouseButton::Left), Modifiers::none());
    vcx.run_until_parked();
    let top_inside = view.read_with(vcx, |v, _| v.scroll_state().top_row);
    vcx.executor().advance_clock(Duration::from_millis(200));
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.scroll_state().top_row),
        top_inside
    );
    // Past the edge again, then the button goes up: it stops too.
    vcx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::none());
    vcx.simulate_mouse_up(below, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let top_up = view.read_with(vcx, |v, _| v.scroll_state().top_row);
    vcx.executor().advance_clock(Duration::from_millis(200));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), top_up);
}

#[gpui::test]
fn the_resize_tag_shows_the_width_while_hovering_or_dragging_an_edge(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(3, 20)), 700.0, 300.0);
    assert_eq!(view.read_with(vcx, |v, _| v.resize_tag()), None);
    let edge = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        let x = f32::from(g.body.left()) + v.column_layout().x_of(1) as f32;
        point(px(x), px(f32::from(g.header.top()) + 10.0))
    });
    vcx.simulate_mouse_move(edge, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.resize_tag()),
        Some((ColId(0), 120.0))
    );
    vcx.simulate_mouse_down(edge, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        point(edge.x + px(16.0), edge.y),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.resize_tag()),
        Some((ColId(0), 136.0))
    );
    vcx.simulate_mouse_up(
        point(edge.x + px(16.0), edge.y),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(point(px(300.0), px(200.0)), None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.resize_tag()), None);
}
