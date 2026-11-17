//! M3b Task 1b: the grid's M3b seams. The view maps work (Tasks 35–37 build on them); the go-to
//! parser and frozen columns are fail-closed stubs; the new commands either tell the host or do
//! nothing; `set_transposed` stores the flag. The `#[gpui::test]`s need `test-support`.
//!
//! ## m3c-t35 (named exception): Task 35 filled frozen columns, grow/shrink, auto-fit and
//! freeze; the maps are built with `ViewMap::permuted`/`filtered` (they carry an index), and
//! the stub checks below keep only the stubs still open (Tasks 37–39).

use std::sync::Arc;

use hacivat_ui_grid::model::{
    ColumnLayout, GotoError, SortSpec, ViewMap, apply_command, parse_goto,
};
use hacivat_ui_grid::{ColId, GridCommand, ModelRow, ViewRow, grid_command};

#[test]
fn identity_view_map_is_unchanged() {
    let m = ViewMap::identity(10);
    assert_eq!(m.view_to_model(ViewRow(3)), Some(ModelRow(3)));
    assert_eq!(m.model_to_view(ModelRow(3)), Some(ViewRow(3)));
    assert_eq!(m.len(), 10);
}

#[test]
fn permuted_view_map_round_trips() {
    let m = ViewMap::permuted(Arc::from([2u64, 0, 1]), 3, 3);
    assert_eq!(m.view_to_model(ViewRow(0)), Some(ModelRow(2)));
    assert_eq!(m.model_to_view(ModelRow(2)), Some(ViewRow(0)));
    for v in 0..3 {
        let model = m.view_to_model(ViewRow(v)).unwrap();
        assert_eq!(m.model_to_view(model), Some(ViewRow(v)));
    }
    assert_eq!(m.len(), 3);
}

#[test]
fn permuted_view_map_keeps_the_unsorted_tail_in_source_order() {
    // Rows 0..3 were sorted; rows 3 and 4 arrived after the sort.
    let m = ViewMap::permuted(Arc::from([1u64, 2, 0]), 3, 5);
    assert_eq!(m.view_to_model(ViewRow(3)), Some(ModelRow(3)));
    assert_eq!(m.view_to_model(ViewRow(4)), Some(ModelRow(4)));
    assert_eq!(m.model_to_view(ModelRow(4)), Some(ViewRow(4)));
    assert_eq!(m.model_to_view(ModelRow(0)), Some(ViewRow(2)));
    assert_eq!(m.len(), 5);
}

#[test]
fn filtered_view_map_shows_only_its_rows() {
    let m = ViewMap::filtered(Arc::from([4u64, 7, 9]), 10);
    assert_eq!(m.len(), 3);
    assert_eq!(m.view_to_model(ViewRow(1)), Some(ModelRow(7)));
    assert_eq!(m.view_to_model(ViewRow(3)), None);
    assert_eq!(m.model_to_view(ModelRow(9)), Some(ViewRow(2)));
    assert_eq!(m.model_to_view(ModelRow(5)), None);
}

#[test]
fn goto_stub_fails_closed_and_frozen_columns_freeze() {
    // ## m3c-t37 (named exception): Task 37 filled `parse_goto` (`tests/goto.rs`).
    assert_eq!(
        parse_goto("12", &[], '.', ','),
        Ok(hacivat_ui_grid::model::GotoTarget::Row(12))
    );
    assert_eq!(parse_goto("", &[], '.', ','), Err(GotoError::Empty));
    let mut layout = ColumnLayout::new([(ColId(0), 80.0), (ColId(1), 80.0)]);
    layout.set_frozen(1);
    // ## m3c-t35: frozen columns are Task 35's (`tests/model_frozen.rs`).
    assert_eq!(layout.frozen(), 1);
    // An empty sort is the source order.
    assert!(SortSpec::default().keys.is_empty());
}

#[test]
fn m3b_grid_commands_are_in_the_table() {
    // ## m3c-t35: Task 35 filled grow/shrink, auto-fit and freeze/unfreeze.
    let filled = [
        "grid.grow_selection",
        "grid.shrink_selection",
        "grid.auto_fit",
        "grid.freeze",
        "grid.unfreeze",
        // ## m3c-t37 (named exception): Task 37 filled find and go to row.
        "grid.find",
        "grid.goto_row",
        // ## m3c-t38 (named exception): Task 38 filled the value viewer.
        "grid.open_value",
        // ## m3c-t39 (named exception): Task 39 filled the context menu.
        "grid.context_menu",
    ];
    for (id, cmd) in [
        ("grid.grow_selection", GridCommand::GrowSelection),
        ("grid.shrink_selection", GridCommand::ShrinkSelection),
        ("grid.find", GridCommand::Find),
        ("grid.goto_row", GridCommand::GotoRow),
        ("grid.open_value", GridCommand::OpenValue),
        ("grid.context_menu", GridCommand::ContextMenu),
        ("grid.auto_fit", GridCommand::AutoFit),
        ("grid.freeze", GridCommand::Freeze),
        ("grid.unfreeze", GridCommand::Unfreeze),
    ] {
        assert_eq!(grid_command(id), Some(cmd), "{id}");
        // Fix round 2 (I3): every new grid command is a stub until its task fills it.
        assert_eq!(
            hacivat_ui_grid::commands::is_stub(id),
            !filled.contains(&id),
            "{id}"
        );
    }
    // ## m3c-t37 (named exception): 4 → 2.
    // ## m3c-t38 (named exception): 2 → 1.
    // ## m3c-t39 (named exception): 1 → 0.
    assert_eq!(hacivat_ui_grid::commands::STUB_COMMANDS.len(), 0);
    // M3a's commands are not.
    for (id, _) in hacivat_ui_grid::GRID_COMMANDS.iter().take(26) {
        assert!(!hacivat_ui_grid::commands::is_stub(id), "{id}");
    }
}

/// The key model leaves the M3b commands to `GridView::dispatch` (## m3c-t35: grow and shrink
/// need the on-screen columns, freeze and auto-fit the layout), so `apply_command` changes
/// nothing for them.
#[test]
fn the_key_model_ignores_the_m3b_commands() {
    let map = ViewMap::identity(5);
    let order = [ColId(0), ColId(1)];
    let ctx = hacivat_ui_grid::model::KeyCtx {
        map: &map,
        order: &order,
        page_rows: 3,
    };
    let mut sel = hacivat_ui_grid::model::Selection::default();
    apply_command(
        GridCommand::Move {
            dir: hacivat_ui_grid::model::Dir::Down,
            extend: false,
        },
        &mut sel,
        &ctx,
    );
    let before = sel.clone();
    for cmd in [
        GridCommand::GrowSelection,
        GridCommand::ShrinkSelection,
        GridCommand::Freeze,
        GridCommand::Unfreeze,
        GridCommand::AutoFit,
    ] {
        assert_eq!(
            apply_command(cmd, &mut sel, &ctx),
            hacivat_ui_grid::model::KeyOutcome::Unchanged
        );
    }
    assert_eq!(sel, before);
}

#[cfg(feature = "test-support")]
mod view {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui::{Entity, TestAppContext, VisualTestContext, px, size};
    use hacivat_ui_core::theme::{Theme, ThemeMode};
    use hacivat_ui_grid::model::{Dir, ViewMap};
    use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config};
    use hacivat_ui_grid::{
        Align, CellKind, ColId, GridCommand, GridDataSource, GridEvent, GridView, ModelCell,
        ModelRow,
    };

    type Events = Rc<RefCell<Vec<GridEvent>>>;

    fn open(cx: &mut TestAppContext) -> (Entity<GridView>, &mut VisualTestContext, Events) {
        cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
        let source: Rc<dyn GridDataSource> =
            Rc::new(FixtureSource::generated(3, 20, Some(20), false, |r, c| {
                cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
            }));
        let (view, vcx) =
            cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
        vcx.simulate_resize(size(px(600.0), px(400.0)));
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

    #[gpui::test]
    fn m3b_commands_tell_the_host_or_do_nothing(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(cx);
        // Without a focus, Open value asks nothing.
        run(&view, vcx, GridCommand::OpenValue);
        assert!(events.borrow().is_empty());
        run(
            &view,
            vcx,
            GridCommand::Move {
                dir: Dir::Down,
                extend: false,
            },
        );
        let focus = view.read_with(vcx, |v, _| v.selection().focus()).unwrap();
        events.borrow_mut().clear();
        // ## m3c-t35: grow/shrink, auto-fit and freeze/unfreeze are no longer no-ops (Task 35's
        // `tests/element_frozen.rs`); the commands below still only tell the host.
        run(&view, vcx, GridCommand::Find);
        run(&view, vcx, GridCommand::GotoRow);
        run(&view, vcx, GridCommand::OpenValue);
        run(&view, vcx, GridCommand::ContextMenu);
        let got = events.borrow().clone();
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[0], GridEvent::FindRequested);
        assert_eq!(got[1], GridEvent::GotoRequested);
        assert_eq!(got[2], GridEvent::ValueRequested(focus));
        // ## m3c-t39 (named exception): the request says it came from the keys, not a press.
        let GridEvent::ContextMenuRequested {
            at,
            cell,
            header,
            row_number,
            pointer,
        } = &got[3]
        else {
            panic!("{:?}", got[3]);
        };
        assert_eq!((*cell, *header), (Some(focus), None));
        assert!(!*row_number && !*pointer);
        // Under the focus cell (row 0 of a 21 px body under the 36 px header), inside the body.
        assert!(f32::from(at.y) > 36.0 && f32::from(at.y) < 400.0, "{at:?}");
        assert_eq!(
            focus,
            ModelCell {
                row: ModelRow(0),
                col: ColId(0)
            }
        );
    }

    #[gpui::test]
    fn view_map_transposed_and_highlights_are_stored(cx: &mut TestAppContext) {
        let (view, vcx, _) = open(cx);
        // No map set: the identity over the loaded rows.
        assert_eq!(
            view.read_with(vcx, |v, _| v.view_map().clone()),
            ViewMap::identity(20)
        );
        let sorted = ViewMap::permuted(Arc::from((0..20u64).rev().collect::<Vec<_>>()), 20, 20);
        view.update(vcx, |v, cx| v.set_view_map(sorted.clone(), cx));
        assert_eq!(view.read_with(vcx, |v, _| v.view_map().clone()), sorted);
        view.update(vcx, |v, cx| v.set_view_map(ViewMap::identity(0), cx));
        assert_eq!(
            view.read_with(vcx, |v, _| v.view_map().clone()),
            ViewMap::identity(20)
        );
        assert!(!view.read_with(vcx, |v, _| v.transposed()));
        view.update(vcx, |v, cx| v.set_transposed(true, cx));
        vcx.run_until_parked();
        assert!(view.read_with(vcx, |v, _| v.transposed()));
        assert!(view.read_with(vcx, |v, _| v.highlights().is_none()));
        // ## m3c-t37 (named exception): the marks are each cell's `CellMarks` now.
        view.update(vcx, |v, cx| {
            v.set_highlights(
                Some(Rc::new(|row: ModelRow, _| {
                    (row.0 == 1).then(hacivat_ui_grid::CellMarks::default)
                })),
                cx,
            )
        });
        vcx.run_until_parked();
        let marked = view.read_with(vcx, |v, _| {
            let h = v.highlights().expect("stored");
            (
                h(ModelRow(1), ColId(0)).is_some(),
                h(ModelRow(2), ColId(0)).is_some(),
            )
        });
        assert_eq!(marked, (true, false));
        view.update(vcx, |v, cx| v.set_highlights(None, cx));
        assert!(view.read_with(vcx, |v, _| v.highlights().is_none()));
    }

    // ## m3c-t39 (named exception): the right press and the header's freeze.

    fn press(
        vcx: &mut VisualTestContext,
        at: gpui::Point<gpui::Pixels>,
        button: gpui::MouseButton,
    ) {
        vcx.simulate_event(gpui::MouseDownEvent {
            button,
            position: at,
            modifiers: gpui::Modifiers::none(),
            click_count: 1,
            first_mouse: false,
        });
        vcx.simulate_event(gpui::MouseUpEvent {
            button,
            position: at,
            modifiers: gpui::Modifiers::none(),
            click_count: 1,
        });
        vcx.run_until_parked();
    }

    /// Window points of the last frame: the middle of view row `row`'s cell in visible column
    /// `ix`, of column `ix`'s header, of row `row`'s number, and the corner.
    fn points(
        view: &Entity<GridView>,
        vcx: &mut VisualTestContext,
        row: u64,
        ix: usize,
    ) -> [gpui::Point<gpui::Pixels>; 4] {
        view.read_with(vcx, |v, _| {
            let g = v.geometry().expect("a frame").clone();
            let layout = v.column_layout();
            let x =
                f32::from(g.body.left()) + ((layout.x_of(ix) + layout.x_of(ix + 1)) / 2.0) as f32;
            let y = f32::from(g.body.top()) + g.row_h * (row as f32 + 0.5);
            [
                gpui::point(px(x), px(y)),
                gpui::point(px(x), g.header.center().y),
                gpui::point(g.gutter.center().x, px(y)),
                g.corner.center(),
            ]
        })
    }

    fn menu_requests(events: &Events) -> Vec<GridEvent> {
        events
            .borrow()
            .iter()
            .filter(|e| matches!(e, GridEvent::ContextMenuRequested { .. }))
            .cloned()
            .collect()
    }

    /// A right press asks the host for the menu over what it hits (a cell, a header, a row
    /// number) at the pointer. Outside the selection it first selects what it hits, as a left
    /// click would; inside it the selection stays (the menu acts on it). The corner and a left
    /// press ask for nothing.
    #[gpui::test]
    fn a_right_press_asks_for_the_menu_over_what_it_hits(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(cx);
        let [cell_at, header_at, row_at, corner_at] = points(&view, vcx, 2, 1);
        let right = gpui::MouseButton::Right;

        press(vcx, cell_at, right);
        let r2c1 = ModelCell {
            row: ModelRow(2),
            col: ColId(1),
        };
        assert_eq!(
            menu_requests(&events),
            [GridEvent::ContextMenuRequested {
                at: cell_at,
                cell: Some(r2c1),
                header: None,
                row_number: false,
                pointer: true,
            }]
        );
        assert_eq!(
            view.read_with(vcx, |v, _| v.selection().focus()),
            Some(r2c1)
        );

        // Inside a selection (everything), the selection stays.
        run(&view, vcx, GridCommand::SelectAll);
        events.borrow_mut().clear();
        let [other_cell, ..] = points(&view, vcx, 4, 2);
        press(vcx, other_cell, right);
        assert!(view.read_with(vcx, |v, _| v.selection().is_all()));
        assert_eq!(menu_requests(&events).len(), 1);

        // A header: its column is selected, and the request names it.
        run(&view, vcx, GridCommand::Escape);
        run(
            &view,
            vcx,
            GridCommand::Move {
                dir: Dir::Down,
                extend: false,
            },
        );
        events.borrow_mut().clear();
        press(vcx, header_at, right);
        assert_eq!(
            menu_requests(&events),
            [GridEvent::ContextMenuRequested {
                at: header_at,
                cell: None,
                header: Some(ColId(1)),
                row_number: false,
                pointer: true,
            }]
        );
        assert!(view.read_with(vcx, |v, _| v.selection().col_selected(ColId(1))));

        // A row number: its row is selected; the request carries the row (focus column kept).
        events.borrow_mut().clear();
        press(vcx, row_at, right);
        let got = menu_requests(&events);
        let [
            GridEvent::ContextMenuRequested {
                cell,
                header,
                row_number,
                pointer,
                ..
            },
        ] = got.as_slice()
        else {
            panic!("{got:?}");
        };
        assert_eq!(cell.map(|c| c.row), Some(ModelRow(2)));
        assert_eq!((*header, *row_number, *pointer), (None, true, true));
        assert!(view.read_with(vcx, |v, _| v.selection().row_selected(ModelRow(2))));

        // The corner, and a left press, ask for nothing.
        events.borrow_mut().clear();
        press(vcx, corner_at, right);
        press(vcx, cell_at, gpui::MouseButton::Left);
        assert!(menu_requests(&events).is_empty());
    }

    /// "Freeze up to here" on a header freezes through that column whatever the focus; a
    /// freeze that would leave less than 48 px of the body to scroll is refused, and says so.
    #[gpui::test]
    fn freeze_through_a_column(cx: &mut TestAppContext) {
        let (view, vcx, events) = open(cx);
        view.update(vcx, |v, cx| v.freeze_through(ColId(1), cx));
        vcx.run_until_parked();
        assert_eq!(view.read_with(vcx, |v, _| v.frozen()), 2);
        assert_eq!(events.borrow().last(), Some(&GridEvent::FrozenChanged(2)));
        // Three wide columns in a narrow pane: freezing all of them is refused.
        view.update(vcx, |v, cx| {
            for c in 0..3 {
                v.set_column_width(ColId(c), 260.0, cx);
            }
        });
        vcx.run_until_parked();
        view.update(vcx, |v, cx| v.freeze_through(ColId(2), cx));
        vcx.run_until_parked();
        assert_eq!(events.borrow().last(), Some(&GridEvent::FreezeRefused));
        assert_eq!(view.read_with(vcx, |v, _| v.frozen()), 2);
    }
}
