//! M3a Task 1b: the grid's seams that later tasks build on: a `GridDataSource` answers one
//! frame per snapshot (`SyntheticSource`, the shape Task 26's adapter follows), and the key
//! context the keymap rows name.

use std::rc::Rc;

use gpui::TestAppContext;
use hacivat_ui_grid::testing::SyntheticSource;
use hacivat_ui_grid::{CellKind, ColId, GRID_CONTEXT, GridDataSource, ModelRow};

#[gpui::test]
fn a_source_answers_rows_and_cells_through_one_frame(cx: &mut TestAppContext) {
    let source: Rc<dyn GridDataSource> = Rc::new(SyntheticSource::new(3, 2));
    let columns = source.columns();
    assert_eq!(columns.len(), 2);
    assert_eq!(columns[0].id, ColId(0));
    let (loaded, more, exact, cell) = cx.update(|cx| {
        let frame = source.begin_frame(cx);
        let count = frame.row_count();
        (
            count.loaded,
            count.more,
            count.exact_total,
            frame.cell(ModelRow(2), ColId(1)),
        )
    });
    assert_eq!((loaded, more, exact), (3, false, Some(3)));
    assert_eq!(cell.text.as_ref(), "r2c1");
    assert_eq!(cell.kind, CellKind::Text);
}

#[test]
fn the_keymap_context_is_grid() {
    assert_eq!(GRID_CONTEXT, "Grid");
}
