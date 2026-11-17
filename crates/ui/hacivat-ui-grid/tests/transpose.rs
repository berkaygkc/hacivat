//! M3c Task 38 (HV-GRID-12; NOTES surface 21): the Transposed view's axis swap. A
//! `TransposedSource` shows a table's rows as columns and its columns as rows: the first column
//! holds each column's name, then one column per row, in the order the table shows them (a
//! sort's or a filter's view map), titled by its row number. A grid over it marks itself
//! transposed (`GridView::set_transposed`): its name column stays frozen and it shows no sort
//! indicator (sorting by a row is not offered, alg/22). Needs `--features test-support`.
#![cfg(feature = "test-support")]

use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, SharedString, TestAppContext, VisualTestContext, size};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use hacivat_ui_grid::model::{SortSpec, ViewMap};
use hacivat_ui_grid::testing::{FixtureSource, cell, grid_config, text_column};
use hacivat_ui_grid::{
    Align, CellKind, ColId, GridDataSource, GridView, ModelCell, ModelRow, TransposedSource,
};

/// Three columns `id`, `name`, `city` over `rows` rows of `r{row}c{col}`.
fn table(rows: u64) -> FixtureSource {
    FixtureSource::generated(3, rows, Some(rows), false, |r, c| {
        cell(format!("r{r}c{}", c.0), CellKind::Text, Align::Start)
    })
    .with_columns(vec![
        text_column(0, "id", 80.0),
        text_column(1, "name", 120.0),
        text_column(2, "city", 120.0),
    ])
}

fn mc(row: u64, col: u32) -> ModelCell {
    ModelCell {
        row: ModelRow(row),
        col: ColId(col),
    }
}

#[gpui::test]
fn rows_become_columns_in_the_order_shown(cx: &mut TestAppContext) {
    let inner: Rc<dyn GridDataSource> = Rc::new(table(4));
    // A sort shows model rows 2, 0, 3, 1; the `name` column is hidden.
    let map = ViewMap::permuted(Arc::from([2u64, 0, 3, 1]), 4, 4);
    let t = TransposedSource::new(
        inner,
        &map,
        &[ColId(0), ColId(2)],
        SharedString::from("Column"),
    );
    let names: Vec<String> = t.columns().iter().map(|c| c.name.to_string()).collect();
    assert_eq!(names, ["Column", "1", "2", "3", "4"]);
    assert_eq!(
        t.columns()[1].align,
        Align::End,
        "a row number is end-aligned"
    );
    let frame = cx.update(|cx| t.begin_frame(cx));
    let count = frame.row_count();
    assert_eq!(
        (count.loaded, count.more, count.exact_total),
        (2, false, Some(2)),
        "one row per shown column"
    );
    // The first column holds the names; the others the cells of the shown rows.
    assert_eq!(frame.cell(ModelRow(0), ColId(0)).text.as_ref(), "id");
    assert_eq!(frame.cell(ModelRow(1), ColId(0)).text.as_ref(), "city");
    assert_eq!(frame.cell(ModelRow(0), ColId(1)).text.as_ref(), "r2c0");
    assert_eq!(frame.cell(ModelRow(1), ColId(4)).text.as_ref(), "r1c2");
    // Past the table: a placeholder, never a panic.
    assert_eq!(
        frame.cell(ModelRow(5), ColId(1)).kind,
        CellKind::Placeholder
    );
    assert_eq!(
        frame.cell(ModelRow(0), ColId(9)).kind,
        CellKind::Placeholder
    );
}

#[gpui::test]
fn a_transposed_cell_maps_to_its_table_cell_and_back(_cx: &mut TestAppContext) {
    let inner: Rc<dyn GridDataSource> = Rc::new(table(4));
    let map = ViewMap::permuted(Arc::from([2u64, 0, 3, 1]), 4, 4);
    let t = TransposedSource::new(
        inner,
        &map,
        &[ColId(0), ColId(2)],
        SharedString::from("Column"),
    );
    assert_eq!(t.table_cell(mc(1, 3)), Some(mc(3, 2)));
    assert_eq!(t.table_cell(mc(0, 0)), None, "the name column is no cell");
    assert_eq!(t.table_cell(mc(7, 1)), None);
    assert_eq!(t.transposed_cell(mc(3, 2)), Some(mc(1, 3)));
    assert_eq!(
        t.transposed_cell(mc(3, 1)),
        None,
        "a hidden column is not shown"
    );
    assert_eq!(
        t.table_rows(),
        &[ModelRow(2), ModelRow(0), ModelRow(3), ModelRow(1)]
    );
    assert_eq!(t.table_cols(), &[ColId(0), ColId(2)]);
    // A filter shows its kept rows only.
    let inner: Rc<dyn GridDataSource> = Rc::new(table(4));
    let kept = ViewMap::filtered(Arc::from([1u64, 3]), 4);
    let t = TransposedSource::new(inner, &kept, &[ColId(1)], SharedString::from("Column"));
    assert_eq!(t.columns().len(), 3);
    assert_eq!(t.table_cell(mc(0, 2)), Some(mc(3, 1)));
}

fn open(
    cx: &mut TestAppContext,
    source: Rc<dyn GridDataSource>,
) -> (Entity<GridView>, &mut VisualTestContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let (view, vcx) = cx.add_window_view(|_, cx| {
        let mut config = grid_config("Result 1");
        config.transposed = true;
        GridView::new(source, config, cx)
    });
    vcx.simulate_resize(size(gpui::px(800.0), gpui::px(400.0)));
    vcx.update(|_, cx| view.update(cx, |v, _| v.set_paint_recording(true)));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    (view, vcx)
}

#[gpui::test]
fn a_transposed_grid_keeps_its_name_column_frozen_and_shows_no_sort(cx: &mut TestAppContext) {
    let inner: Rc<dyn GridDataSource> = Rc::new(table(3));
    let t = TransposedSource::new(
        inner,
        &ViewMap::identity(3),
        &[ColId(0), ColId(1), ColId(2)],
        SharedString::from("Column"),
    );
    let (view, vcx) = open(cx, Rc::new(t));
    assert_eq!(view.read_with(vcx, |v, _| v.frozen()), 1);
    // Unfreezing leaves the names in place: the next frame freezes them again.
    vcx.update(|_, cx| view.update(cx, |v, cx| v.set_frozen(0, cx)));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.frozen()), 1);
    // A sort's indicator never shows on a transposed grid.
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_sort(SortSpec::by(ColId(1), false), Vec::new(), cx)
        })
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(view.read_with(vcx, |v, _| v.sort().is_empty()));
    assert!(view.read_with(vcx, |v, _| v.painted_sort_marks().is_empty()));
    // Painted: view row 1 (the `name` column) shows its name, then rows 0..3.
    let row1: Vec<(u32, String)> = view.read_with(vcx, |v, _| {
        let mut cells: Vec<(u32, String)> = v
            .painted_cells()
            .iter()
            .filter(|c| c.row == 1)
            .map(|c| (c.col.0, c.text.to_string()))
            .collect();
        cells.sort();
        cells
    });
    assert_eq!(
        row1,
        [
            (0, "name".to_string()),
            (1, "r0c1".to_string()),
            (2, "r1c1".to_string()),
            (3, "r2c1".to_string())
        ]
    );
}

#[gpui::test]
fn a_table_grid_is_left_alone(cx: &mut TestAppContext) {
    cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
    let source: Rc<dyn GridDataSource> = Rc::new(table(3));
    let (view, vcx) =
        cx.add_window_view(|_, cx| GridView::new(source, grid_config("Result 1"), cx));
    vcx.simulate_resize(size(gpui::px(800.0), gpui::px(400.0)));
    vcx.run_until_parked();
    assert_eq!(view.read_with(vcx, |v, _| v.frozen()), 0);
    vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.set_sort(SortSpec::by(ColId(1), false), Vec::new(), cx)
        })
    });
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert!(!view.read_with(vcx, |v, _| v.sort().is_empty()));
}
