//! The grid adapter over a result store (Task 26): one `snapshot()` per frame, columns from
//! the `ResultSchema`, cells through `format_cell` and the display cache, `ensure_rows`
//! forwarded to the result's source.
//!
//! Task 1b stub: zero rows, no columns.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_grid::{
    Align, CellContent, CellKind, ColId, ColumnSpec, GridCellFlags, GridDataSource, GridFrame,
    ModelRow, RowCount, SourceCaps,
};

/// A result as a grid source.
#[derive(Debug, Default)]
pub struct GridAdapter {
    _private: (),
}

impl GridAdapter {
    /// An adapter with no result (Task 26 builds it over a `ResultStore`).
    pub fn empty() -> Self {
        GridAdapter::default()
    }
}

struct EmptyFrame;

impl GridFrame for EmptyFrame {
    fn generation(&self) -> (u64, u64) {
        (0, 0)
    }

    fn row_count(&self) -> RowCount {
        RowCount {
            loaded: 0,
            more: false,
            exact_total: Some(0),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        let _ = (row, col);
        CellContent {
            text: "".into(),
            kind: CellKind::Placeholder,
            flags: GridCellFlags::empty(),
            align: Align::Start,
            chip: None,
        }
    }
}

impl GridDataSource for GridAdapter {
    fn begin_frame(&self, cx: &App) -> Rc<dyn GridFrame> {
        let _ = cx;
        Rc::new(EmptyFrame)
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        Arc::from(Vec::new())
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps::default()
    }

    fn ensure_rows(&self, upto: u64, cx: &mut App) {
        let _ = (upto, cx);
    }
}
