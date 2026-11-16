//! M3b Task 1b: the grid's M3b seams. The view maps work (Tasks 35–37 build on them); the go-to
//! parser and frozen columns are fail-closed stubs; the new commands either tell the host or do
//! nothing; `set_transposed` stores the flag. The `#[gpui::test]`s need `test-support`.

use std::sync::Arc;

use hacivat_ui_grid::model::{
    ColumnLayout, GotoError, SortKey, SortSpec, ViewMap, apply_command, parse_goto,
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
    let m = ViewMap::Permuted {
        order: Arc::from([2u64, 0, 1]),
        tail_from: 3,
        loaded: 3,
    };
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
    let m = ViewMap::Permuted {
        order: Arc::from([1u64, 2, 0]),
        tail_from: 3,
        loaded: 5,
    };
    assert_eq!(m.view_to_model(ViewRow(3)), Some(ModelRow(3)));
    assert_eq!(m.view_to_model(ViewRow(4)), Some(ModelRow(4)));
    assert_eq!(m.model_to_view(ModelRow(4)), Some(ViewRow(4)));
    assert_eq!(m.model_to_view(ModelRow(0)), Some(ViewRow(2)));
    assert_eq!(m.len(), 5);
}

#[test]
fn filtered_view_map_shows_only_its_rows() {
    let m = ViewMap::Filtered {
        rows: Arc::from([4u64, 7, 9]),
        source_loaded: 10,
    };
    assert_eq!(m.len(), 3);
    assert_eq!(m.view_to_model(ViewRow(1)), Some(ModelRow(7)));
    assert_eq!(m.view_to_model(ViewRow(3)), None);
    assert_eq!(m.model_to_view(ModelRow(9)), Some(ViewRow(2)));
    assert_eq!(m.model_to_view(ModelRow(5)), None);
}

#[test]
fn goto_and_frozen_stubs_fail_closed() {
    assert_eq!(parse_goto("12", &[], '.', ','), Err(GotoError::Empty));
    let mut layout = ColumnLayout::new([(ColId(0), 80.0), (ColId(1), 80.0)]);
    layout.set_frozen(1);
    assert_eq!(layout.frozen(), 0);
    let spec = SortSpec {
        keys: [SortKey {
            col: ColId(1),
            desc: true,
        }]
        .into_iter()
        .collect(),
    };
    assert_eq!(spec.keys.len(), 1);
}

#[test]
fn m3b_grid_commands_are_in_the_table() {
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
    }
}

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
        let selection = view.read_with(vcx, |v, _| v.selection().clone());
        for cmd in [
            GridCommand::GrowSelection,
            GridCommand::ShrinkSelection,
            GridCommand::AutoFit,
            GridCommand::Freeze,
            GridCommand::Unfreeze,
        ] {
            run(&view, vcx, cmd);
        }
        assert!(events.borrow().is_empty(), "{:?}", events.borrow());
        assert_eq!(view.read_with(vcx, |v, _| v.selection().clone()), selection);
        run(&view, vcx, GridCommand::Find);
        run(&view, vcx, GridCommand::GotoRow);
        run(&view, vcx, GridCommand::OpenValue);
        run(&view, vcx, GridCommand::ContextMenu);
        let got = events.borrow().clone();
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!(got[0], GridEvent::FindRequested);
        assert_eq!(got[1], GridEvent::GotoRequested);
        assert_eq!(got[2], GridEvent::ValueRequested(focus));
        let GridEvent::ContextMenuRequested { at, cell, header } = &got[3] else {
            panic!("{:?}", got[3]);
        };
        assert_eq!((*cell, *header), (Some(focus), None));
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
        let sorted = ViewMap::Permuted {
            order: Arc::from((0..20u64).rev().collect::<Vec<_>>()),
            tail_from: 20,
            loaded: 20,
        };
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
        view.update(vcx, |v, cx| {
            v.set_highlights(Some(Rc::new(|row: ModelRow, _| row.0 == 1)), cx)
        });
        vcx.run_until_parked();
    }
}
