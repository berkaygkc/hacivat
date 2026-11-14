//! `GridView` (Task 20): keyboard commands through `dispatch`, the pointer (cells, gutter,
//! header, corner, column resize, the scrollbar), re-runs and appends, follow-tail, events and
//! the accessibility declaration. The target builds only with `--features test-support`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Entity, Modifiers, MouseButton, Point, Role, TestAppContext, VisualTestContext, point, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::model::Dir;
use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config};
use hacivat_ui_grid::{
    Align, CellKind, ColId, GridCommand, GridDataSource, GridEvent, GridView, ModelCell, ModelRow,
};

type Events = Rc<RefCell<Vec<GridEvent>>>;

fn numbered(cols: u32, loaded: u64, total: Option<u64>, more: bool) -> FixtureSource {
    FixtureSource::generated(cols, loaded, total, more, |r, c| {
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

fn focus(view: &Entity<GridView>, vcx: &mut VisualTestContext) -> Option<(u64, u32)> {
    view.read_with(vcx, |v, _| {
        v.selection().focus().map(|f| (f.row.0, f.col.0))
    })
}

fn mv(dir: Dir, extend: bool) -> GridCommand {
    GridCommand::Move { dir, extend }
}

/// The centre of body cell `(row, col)` in the last frame (both visible).
fn cell_center(
    view: &Entity<GridView>,
    vcx: &mut VisualTestContext,
    row: u64,
    col: usize,
) -> Point<gpui::Pixels> {
    view.read_with(vcx, |v, _| {
        let g = v.geometry().expect("painted");
        let layout = v.column_layout();
        let s = v.scroll_state();
        let x = f32::from(g.body.left())
            + (layout.x_of(col) - s.left_x) as f32
            + layout.width_at(col) / 2.0;
        let y = f32::from(g.body.top()) + s.row_top(row) as f32 + 10.0;
        point(px(x), px(y))
    })
}

fn click(vcx: &mut VisualTestContext, at: Point<gpui::Pixels>, modifiers: Modifiers) {
    vcx.simulate_click(at, modifiers);
    vcx.run_until_parked();
}

#[gpui::test]
fn keyboard_moves_and_select_all(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(
        cx,
        Rc::new(numbered(4, 500, Some(500), false)),
        700.0,
        400.0,
    );
    // The first key focuses the first cell.
    run(&view, vcx, mv(Dir::Down, false));
    assert_eq!(focus(&view, vcx), Some((0, 0)));
    run(&view, vcx, mv(Dir::Right, false));
    run(&view, vcx, mv(Dir::Right, false));
    run(&view, vcx, mv(Dir::Down, true));
    assert_eq!(focus(&view, vcx), Some((1, 2)));
    let rects = view.read_with(vcx, |v, _| v.selection().rects().to_vec());
    assert_eq!(rects.len(), 1);
    assert_eq!((rects[0].rows.clone(), rects[0].cols.clone()), (0..2, 2..3));
    run(&view, vcx, GridCommand::SelectAll);
    assert!(view.read_with(vcx, |v, _| v.selection().is_all()));
    assert_eq!(
        view.read_with(vcx, |v, _| v
            .selection()
            .cell_count(500, v.column_layout().visible())),
        2_000
    );
    // One event per frame: each command above ran in its own frame.
    let e = events.borrow().clone();
    let changed = e
        .iter()
        .filter(|e| **e == GridEvent::SelectionChanged)
        .count();
    assert_eq!(changed, 5, "{e:?}");
    // Two commands in one frame: one event; a frame that changes nothing: none.
    events.borrow_mut().clear();
    vcx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.dispatch(GridCommand::Escape, window, cx);
            v.dispatch(mv(Dir::Left, false), window, cx);
        })
    });
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let changed = events
        .borrow()
        .iter()
        .filter(|e| **e == GridEvent::SelectionChanged)
        .count();
    assert_eq!(changed, 1, "{:?}", events.borrow());
    run(&view, vcx, GridCommand::SelectAll);
    assert!(e.contains(&GridEvent::FocusChanged(Some(ModelCell {
        row: ModelRow(1),
        col: ColId(2)
    }))));
    // Last row scrolls the focus into view and follows the tail.
    run(&view, vcx, GridCommand::Escape);
    run(&view, vcx, GridCommand::LastRow { extend: false });
    assert_eq!(focus(&view, vcx), Some((499, 1)));
    let (top, max_top, tail) = view.read_with(vcx, |v, _| {
        let s = v.scroll_state();
        (s.top_row, s.max_top_row(), s.follow_tail)
    });
    assert_eq!(top, max_top);
    assert!(tail);
    // Page up moves a body's worth of rows and keeps the focus on screen.
    run(
        &view,
        vcx,
        GridCommand::Page {
            up: true,
            extend: false,
        },
    );
    let (f, visible) = view.read_with(vcx, |v, _| {
        (
            v.selection().focus().map(|f| f.row.0),
            v.scroll_state().visible_rows(),
        )
    });
    assert!(
        f.is_some_and(|r| visible.contains(&r) && r < 499),
        "{f:?} {visible:?}"
    );
}

#[gpui::test]
fn copy_requests_the_region_or_says_why_not(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(numbered(3, 10, Some(10), false)), 600.0, 300.0);
    run(&view, vcx, mv(Dir::Down, false));
    run(&view, vcx, mv(Dir::Down, true));
    run(&view, vcx, GridCommand::Copy { headers: true });
    let copied = events.borrow().iter().find_map(|e| match e {
        GridEvent::CopyRequested { region, headers } => Some((region.cell_count(), *headers)),
        _ => None,
    });
    assert_eq!(copied, Some((2, true)));
}

#[gpui::test]
fn grid_declares_role_and_label(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(2, 3, Some(3), false)), 400.0, 200.0);
    // Without assistive technology the frames never read the label.
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.a11y_label_reads()), 0);
    let declared = vcx.update(|_, cx| view.update(cx, |v, cx| v.a11y_declaration(cx)));
    assert_eq!(declared, (Role::Grid, "Result 1".into()));
}

#[gpui::test]
fn pointer_selects_cells_rows_columns_and_everything(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(
        cx,
        Rc::new(numbered(4, 100, Some(100), false)),
        800.0,
        400.0,
    );
    let a = cell_center(&view, vcx, 2, 1);
    click(vcx, a, Modifiers::none());
    assert_eq!(focus(&view, vcx), Some((2, 1)));
    let b = cell_center(&view, vcx, 4, 3);
    click(vcx, b, Modifiers::shift());
    assert_eq!(focus(&view, vcx), Some((4, 3)));
    assert_eq!(
        view.read_with(vcx, |v, _| v
            .selection()
            .cell_count(100, v.column_layout().visible())),
        9
    );
    // The gutter selects rows; the header, a column; the corner, everything.
    let (gutter, header, corner) = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        (
            g.gutter.center(),
            g.header.origin + point(px(30.0), px(18.0)),
            g.corner.center(),
        )
    });
    let row_y = cell_center(&view, vcx, 6, 0).y;
    click(vcx, point(gutter.x, row_y), Modifiers::none());
    assert!(view.read_with(vcx, |v, _| v.selection().row_selected(ModelRow(6))));
    click(vcx, header, Modifiers::none());
    assert!(view.read_with(vcx, |v, _| v.selection().col_selected(ColId(0))));
    click(vcx, corner, Modifiers::none());
    assert!(view.read_with(vcx, |v, _| v.selection().is_all()));
    assert!(events.borrow().contains(&GridEvent::SelectionChanged));
}

#[gpui::test]
fn dragging_a_header_edge_resizes_the_column(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(cx, Rc::new(numbered(3, 20, Some(20), false)), 800.0, 300.0);
    let (edge, w0) = view.read_with(vcx, |v, _| {
        let g = v.geometry().unwrap();
        let l = v.column_layout();
        let x = f32::from(g.body.left()) + l.x_of(1) as f32;
        (point(px(x - 1.0), g.header.center().y), l.width_at(0))
    });
    vcx.simulate_mouse_down(edge, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        edge + point(px(60.0), px(0.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.simulate_mouse_up(
        edge + point(px(60.0), px(0.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    let w1 = view.read_with(vcx, |v, _| v.column_layout().width_at(0));
    assert!((w1 - (w0 + 60.0)).abs() < 1.0, "{w0} → {w1}");
    let resized = events
        .borrow()
        .iter()
        .any(|e| matches!(e, GridEvent::ColumnResized { col: ColId(0), .. }));
    assert!(resized);
}

#[gpui::test]
fn clicking_the_track_jumps_and_dragging_the_thumb_reaches_the_end(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(
        cx,
        Rc::new(numbered(2, 1_000_000, Some(1_000_000), false)),
        600.0,
        400.0,
    );
    let track = view.read_with(vcx, |v, _| v.geometry().unwrap().vtrack);
    click(
        vcx,
        point(track.center().x, track.bottom() - px(3.0)),
        Modifiers::none(),
    );
    let (top, max_top) = view.read_with(vcx, |v, _| {
        (v.scroll_state().top_row, v.scroll_state().max_top_row())
    });
    assert!(top > max_top / 2, "{top} of {max_top}");
    // Drag the thumb to the very top: exactly row 0.
    let thumb = view.read_with(vcx, |v, _| {
        hacivat_ui_grid::element::scrollbar::v_thumb(v.geometry().unwrap().vtrack, v.scroll_state())
    });
    vcx.simulate_mouse_down(thumb.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        point(thumb.center().x, track.top() - px(50.0)),
        Some(MouseButton::Left),
        Modifiers::none(),
    );
    vcx.simulate_mouse_up(
        point(thumb.center().x, track.top() - px(50.0)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), 0);
}

#[gpui::test]
fn a_rerun_clears_the_selection_and_keeps_the_focus_until_rows_arrive(cx: &mut TestAppContext) {
    let source = numbered(3, 200, Some(200), false);
    let (view, vcx, events) = open(cx, Rc::new(source.clone()), 600.0, 300.0);
    run(&view, vcx, mv(Dir::Down, false));
    for _ in 0..120 {
        run(&view, vcx, mv(Dir::Down, false));
    }
    run(&view, vcx, mv(Dir::Right, true));
    assert_eq!(focus(&view, vcx), Some((120, 1)));
    assert!(view.read_with(vcx, |v, _| v.scroll_state().top_row) > 0);
    // A re-run with no rows yet: nothing selected, the focus pending, the view at the top.
    events.borrow_mut().clear();
    source.rerun(0, true, None);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let (empty, pending, top) = view.read_with(vcx, |v, _| {
        (
            v.selection().is_empty(),
            v.selection().pending_focus(),
            v.scroll_state().top_row,
        )
    });
    assert!(empty);
    assert_eq!(pending.map(|f| (f.row.0, f.col.0)), Some((120, 1)));
    assert_eq!(top, 0);
    // The first rows arrive: the focus is placed, clamped, and announced.
    source.set_count(50, true, None);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(focus(&view, vcx), Some((49, 1)));
    let placed = ModelCell {
        row: ModelRow(49),
        col: ColId(1),
    };
    assert!(
        events
            .borrow()
            .contains(&GridEvent::FocusChanged(Some(placed))),
        "{:?}",
        events.borrow()
    );
    // A re-run whose rows are already there starts at the top too, the focus clamped at once.
    source.set_count(400, false, Some(400));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_px(21.0 * 300.0, 0.0, cx)));
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.scroll_state().top_row) >= 250);
    source.rerun(30, false, Some(30));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let (top, f) = view.read_with(vcx, |v, _| {
        (
            v.scroll_state().top_row,
            v.selection().focus().map(|f| (f.row.0, f.col.0)),
        )
    });
    assert_eq!((top, f), (0, Some((29, 1))));
    source.rerun(400, false, Some(400));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_px(21.0 * 300.0, 0.0, cx)));
    vcx.run_until_parked();
    source.rerun(400, false, Some(400));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), 0);
}

#[gpui::test]
fn appends_keep_the_selection_and_follow_the_tail(cx: &mut TestAppContext) {
    let source = numbered(2, 100, None, true);
    let (view, vcx, _) = open(cx, Rc::new(source.clone()), 500.0, 300.0);
    run(&view, vcx, mv(Dir::Down, false));
    run(&view, vcx, GridCommand::LastRow { extend: false });
    assert_eq!(focus(&view, vcx), Some((99, 0)));
    source.set_count(300, true, None);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let (top, max_top, tail, f) = view.read_with(vcx, |v, _| {
        let s = v.scroll_state();
        (
            s.top_row,
            s.max_top_row(),
            s.follow_tail,
            v.selection().focus().map(|f| f.row.0),
        )
    });
    assert!(tail);
    assert_eq!(top, max_top);
    assert_eq!(f, Some(99), "an append keeps the focus");
    // The stream does not trip fast-scroll on a still view.
    source.set_count(5_000, true, None);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |v, _| v.frame_stats()).fast);
}

#[gpui::test]
fn the_visible_region_is_announced_once_it_is_still(cx: &mut TestAppContext) {
    let (view, vcx, events) = open(
        cx,
        Rc::new(numbered(3, 10_000, Some(10_000), false)),
        600.0,
        300.0,
    );
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
    events.borrow_mut().clear();
    for _ in 0..5 {
        vcx.update(|_, cx| view.update(cx, |v, cx| v.scroll_px(210.0, 0.0, cx)));
        vcx.executor().advance_clock(Duration::from_millis(10));
        vcx.run_until_parked();
    }
    let early = events
        .borrow()
        .iter()
        .filter(|e| matches!(e, GridEvent::VisibleRegion { .. }))
        .count();
    assert_eq!(early, 0, "still scrolling");
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
    let regions: Vec<_> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            GridEvent::VisibleRegion { rows, .. } => Some(rows.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(regions.len(), 1, "{regions:?}");
    assert_eq!(regions[0].start, 50);
}

#[gpui::test]
fn wheel_events_scroll_the_grid(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(
        cx,
        Rc::new(numbered(2, 1_000, Some(1_000), false)),
        500.0,
        300.0,
    );
    let at = cell_center(&view, vcx, 1, 0);
    vcx.simulate_event(gpui::ScrollEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-105.0))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), 5);
}

#[gpui::test]
fn set_source_starts_over_and_keeps_widths_when_asked(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(cx, Rc::new(numbered(3, 10, Some(10), false)), 600.0, 300.0);
    let set = vcx.update(|_, cx| view.update(cx, |v, cx| v.set_column_width(ColId(1), 333.0, cx)));
    assert!(set);
    for _ in 0..3 {
        run(&view, vcx, mv(Dir::Down, false));
    }
    assert_eq!(focus(&view, vcx), Some((2, 0)));
    let width =
        |vcx: &mut VisualTestContext| view.read_with(vcx, |v, _| v.column_layout().width_at(1));
    // The same column names: the widths stay; the selection starts over around the focus.
    let next: Rc<dyn GridDataSource> = Rc::new(numbered(3, 4, Some(4), false));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_source(next, true, cx)));
    vcx.run_until_parked();
    assert_eq!(width(vcx), 333.0);
    assert_eq!(focus(&view, vcx), Some((2, 0)));
    assert_eq!(view.read_with(vcx, |v, _| v.selection().rects().len()), 1);
    // Not keeping them: the source's widths.
    let fresh: Rc<dyn GridDataSource> = Rc::new(numbered(3, 4, Some(4), false));
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_source(fresh, false, cx)));
    vcx.run_until_parked();
    assert_eq!(width(vcx), 120.0);
}

/// Review round 1: after a wheel scroll the hover is the cell under the still pointer, not the
/// row it was on.
#[gpui::test]
fn the_hover_follows_the_pointer_through_a_wheel_scroll(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(
        cx,
        Rc::new(numbered(2, 1_000, Some(1_000), false)),
        500.0,
        300.0,
    );
    let at = cell_center(&view, vcx, 2, 0);
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.pointer_state().0),
        Some((2, 0))
    );
    vcx.simulate_event(gpui::ScrollEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-63.0))),
        ..Default::default()
    });
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.scroll_state().top_row), 3);
    assert_eq!(
        view.read_with(vcx, |v, _| v.pointer_state().0),
        Some((5, 0))
    );
}

/// Review round 1: a mouse-up the grid never heard (released outside the window) ends the
/// drag at the next move without a button, and the hover tracks the pointer again.
#[gpui::test]
fn a_missed_mouse_up_ends_the_drag(cx: &mut TestAppContext) {
    let (view, vcx, _) = open(
        cx,
        Rc::new(numbered(3, 100, Some(100), false)),
        600.0,
        300.0,
    );
    let a = cell_center(&view, vcx, 1, 0);
    let b = cell_center(&view, vcx, 3, 0);
    let c = cell_center(&view, vcx, 4, 1);
    vcx.simulate_mouse_down(a, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(b, Some(MouseButton::Left), Modifiers::none());
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.pointer_state().1), "dragging");
    assert_eq!(focus(&view, vcx), Some((3, 0)));
    // No mouse-up: the next move arrives with no button pressed.
    vcx.simulate_mouse_move(c, None, Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        view.read_with(vcx, |v, _| v.pointer_state()),
        (Some((4, 1)), false)
    );
    // The selection stayed where the drag left it.
    assert_eq!(focus(&view, vcx), Some((3, 0)));
}
