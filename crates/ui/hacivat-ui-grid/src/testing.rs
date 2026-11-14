//! `SyntheticSource` (feature `test-support`; Task 20 ports the M0 spike's source): `rows` ×
//! `cols` cells of generated text, every row loaded.
//!
//! Task 1b: a minimal working source (`r<row>c<col>` text cells, `Number` for column 0).

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui_core::gpui_surface::App;

use crate::source::{
    Align, CellContent, CellKind, ColId, ColumnKind, ColumnSpec, GridCellFlags, GridDataSource,
    GridFrame, ModelRow, RowCount, SourceCaps,
};

/// A generated source.
#[derive(Debug, Clone)]
pub struct SyntheticSource {
    rows: u64,
    columns: Arc<[ColumnSpec]>,
}

impl SyntheticSource {
    pub fn new(rows: u64, cols: u32) -> Self {
        let columns = (0..cols)
            .map(|c| ColumnSpec {
                id: ColId(c),
                name: format!("c{c}").into(),
                type_label: if c == 0 { "int8" } else { "text" }.into(),
                kind: if c == 0 {
                    ColumnKind::Number
                } else {
                    ColumnKind::Text
                },
                align: if c == 0 { Align::End } else { Align::Start },
                default_width: if c == 0 { 96.0 } else { 160.0 },
                key: c == 0,
            })
            .collect();
        SyntheticSource { rows, columns }
    }
}

struct Frame {
    rows: u64,
}

impl GridFrame for Frame {
    fn generation(&self) -> (u64, u64) {
        (1, 1)
    }

    fn row_count(&self) -> RowCount {
        RowCount {
            loaded: self.rows,
            more: false,
            exact_total: Some(self.rows),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        let (text, kind, align) = if col.0 == 0 {
            (row.0.to_string(), CellKind::Number, Align::End)
        } else {
            (
                format!("r{}c{}", row.0, col.0),
                CellKind::Text,
                Align::Start,
            )
        };
        CellContent {
            text: text.into(),
            kind,
            flags: GridCellFlags::empty(),
            align,
            chip: None,
        }
    }
}

impl GridDataSource for SyntheticSource {
    fn begin_frame(&self, _cx: &App) -> Rc<dyn GridFrame> {
        Rc::new(Frame { rows: self.rows })
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        self.columns.clone()
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps::default()
    }

    fn ensure_rows(&self, upto: u64, cx: &mut App) {
        let _ = (upto, cx);
    }
}
