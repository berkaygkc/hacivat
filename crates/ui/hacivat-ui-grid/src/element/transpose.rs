//! The Transposed view's axis swap (HV-GRID-12; M3b Task 38): while
//! `GridConfig::transposed` is on, the frame the element paints shows the source's rows as
//! columns and its columns as rows.
//!
//! ## m3b-t01b-seams: the hook (Task 1b), called once per frame by `grid::prepaint_frame`
//! right after `begin_frame`, so Task 38 edits neither `grid.rs` nor `view.rs`.
//!
//! ## m3c-t38 (M3c Task 38; NOTES surface 21)
//! - **The swap is a source.** A grid's columns, its layout, its selection and every event it
//!   sends (`FocusChanged`, `CopyRequested`, `ValueRequested`) come from its source's
//!   `columns()` and frames, so a swap of the frame alone would leave them in the other axis.
//!   [`TransposedSource`] wraps a table's source: its first column holds each table column's
//!   name, then one column per table row **in the order the table shows them** (the view map
//!   of a sort or a filter, read when it is built), titled by its row number (end-aligned);
//!   one row per **shown** table column. The host gives it a grid of its own whose config is
//!   `transposed` and maps cells back with [`TransposedSource::table_cell`].
//! - **The hook** ([`frame`]) keeps a transposed grid's presentation: the name column stays
//!   frozen (the table's own frozen columns are the table grid's and come back with it; freezing
//!   further row columns works as in a table, unfreezing keeps the names), and no sort
//!   indicator is shown: sorting by a row is not offered (alg/22); the table's sort orders the
//!   columns. A grid whose config is not transposed is left alone.
//! - Only the loaded rows are shown: a transposed source asks its table for nothing
//!   (`ensure_rows` is a no-op); the host rebuilds it when the table's rows or order change.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{App, SharedString};

use crate::model::ViewMap;
use crate::source::{
    Align, CellContent, CellKind, ColId, ColumnKind, ColumnSpec, GridCellFlags, GridDataSource,
    GridFrame, ModelCell, ModelRow, RowCount, SourceCaps, ViewRow,
};
use crate::view::{GridView, SortShown};

/// The name column's width (NOTES surface 21: 150 px).
pub const TRANSPOSED_NAME_WIDTH: f32 = 150.0;

/// A row column's starting width (a transposed grid is not auto-sized past a few rows).
pub const TRANSPOSED_ROW_WIDTH: f32 = 160.0;

/// The frame the element paints for `v` this frame: `frame` itself (the swap is the source's,
/// module docs). While `v` is transposed its name column stays frozen and it shows no sort.
pub(crate) fn frame(v: &mut GridView, frame: Rc<dyn GridFrame>) -> Rc<dyn GridFrame> {
    if v.config.transposed {
        if v.layout.frozen() == 0 && v.layout.len() > 1 {
            v.layout.set_frozen(1);
            v.scroll.set_content_width(v.layout.scroll_width());
        }
        if !v.sort.spec.keys.is_empty() {
            v.sort = SortShown::default();
        }
    }
    frame
}

/// A table's source with its axes swapped (module docs).
pub struct TransposedSource {
    inner: Rc<dyn GridDataSource>,
    /// The table's model rows, in the order the table showed them: transposed column `i + 1`.
    rows: Arc<[ModelRow]>,
    /// The table's shown columns, in order: transposed row `i`.
    cols: Arc<[ColId]>,
    /// The table's spec of each shown column (its name), by transposed row.
    names: Arc<[SharedString]>,
    columns: Arc<[ColumnSpec]>,
}

impl std::fmt::Debug for TransposedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransposedSource")
            .field("rows", &self.rows.len())
            .field("cols", &self.cols.len())
            .finish_non_exhaustive()
    }
}

impl TransposedSource {
    /// `inner`'s rows as `map` shows them (every row `map` shows, in view order) as columns, and
    /// its columns `cols` (the table's shown columns, in order) as rows. `name_title` heads the
    /// name column (the app's "Column").
    pub fn new(
        inner: Rc<dyn GridDataSource>,
        map: &ViewMap,
        cols: &[ColId],
        name_title: SharedString,
    ) -> Self {
        let rows: Arc<[ModelRow]> = (0..map.len())
            .filter_map(|v| map.view_to_model(ViewRow(v)))
            .collect();
        let specs = inner.columns();
        let names: Arc<[SharedString]> = cols
            .iter()
            .map(|c| {
                specs
                    .iter()
                    .find(|s| s.id == *c)
                    .map(|s| s.name.clone())
                    .unwrap_or_default()
            })
            .collect();
        let mut columns = Vec::with_capacity(rows.len() + 1);
        columns.push(ColumnSpec {
            id: ColId(0),
            name: name_title,
            type_label: SharedString::default(),
            kind: ColumnKind::Text,
            align: Align::Start,
            default_width: TRANSPOSED_NAME_WIDTH,
            key: false,
        });
        for i in 0..rows.len() {
            columns.push(ColumnSpec {
                id: ColId(u32::try_from(i + 1).unwrap_or(u32::MAX)),
                // The row number as the table's gutter shows it (1-based, data digits).
                name: SharedString::from((i + 1).to_string()),
                type_label: SharedString::default(),
                kind: ColumnKind::Other,
                align: Align::End,
                default_width: TRANSPOSED_ROW_WIDTH,
                key: false,
            });
        }
        TransposedSource {
            inner,
            rows,
            cols: cols.into(),
            names,
            columns: columns.into(),
        }
    }

    /// The table cell transposed cell `cell` shows; `None` for the name column and outside the
    /// table.
    pub fn table_cell(&self, cell: ModelCell) -> Option<ModelCell> {
        let ix = (cell.col.0 as usize).checked_sub(1)?;
        Some(ModelCell {
            row: *self.rows.get(ix)?,
            col: *self.cols.get(usize::try_from(cell.row.0).ok()?)?,
        })
    }

    /// The transposed cell that shows table cell `cell`; `None` when its row or column is not
    /// shown.
    pub fn transposed_cell(&self, cell: ModelCell) -> Option<ModelCell> {
        let row = self.cols.iter().position(|c| *c == cell.col)?;
        let col = self.rows.iter().position(|r| *r == cell.row)?;
        Some(ModelCell {
            row: ModelRow(row as u64),
            col: ColId(u32::try_from(col + 1).ok()?),
        })
    }

    /// The table's rows, in the order shown (transposed column `i + 1` is `table_rows()[i]`).
    pub fn table_rows(&self) -> &[ModelRow] {
        &self.rows
    }

    /// The table's shown columns (transposed row `i` is `table_cols()[i]`).
    pub fn table_cols(&self) -> &[ColId] {
        &self.cols
    }

    /// The table's source.
    pub fn table(&self) -> &Rc<dyn GridDataSource> {
        &self.inner
    }
}

/// One frame of a [`TransposedSource`]: the table's frame read across.
struct TransposedFrame {
    inner: Rc<dyn GridFrame>,
    rows: Arc<[ModelRow]>,
    cols: Arc<[ColId]>,
    names: Arc<[SharedString]>,
}

fn placeholder() -> CellContent {
    CellContent {
        text: SharedString::default(),
        kind: CellKind::Placeholder,
        flags: GridCellFlags::empty(),
        align: Align::Start,
        chip: None,
    }
}

impl GridFrame for TransposedFrame {
    fn generation(&self) -> (u64, u64) {
        self.inner.generation()
    }

    fn row_count(&self) -> RowCount {
        let n = self.cols.len() as u64;
        RowCount {
            loaded: n,
            more: false,
            exact_total: Some(n),
        }
    }

    fn cell(&self, row: ModelRow, col: ColId) -> CellContent {
        let Some(r) = usize::try_from(row.0).ok().filter(|r| *r < self.cols.len()) else {
            return placeholder();
        };
        if col.0 == 0 {
            return CellContent {
                text: self.names[r].clone(),
                kind: CellKind::Text,
                flags: GridCellFlags::empty(),
                align: Align::Start,
                chip: None,
            };
        }
        match self.rows.get(col.0 as usize - 1) {
            Some(table_row) => self.inner.cell(*table_row, self.cols[r]),
            None => placeholder(),
        }
    }
}

impl GridDataSource for TransposedSource {
    fn begin_frame(&self, cx: &App) -> Rc<dyn GridFrame> {
        Rc::new(TransposedFrame {
            inner: self.inner.begin_frame(cx),
            rows: self.rows.clone(),
            cols: self.cols.clone(),
            names: self.names.clone(),
        })
    }

    fn columns(&self) -> Arc<[ColumnSpec]> {
        self.columns.clone()
    }

    fn caps(&self) -> SourceCaps {
        // Nothing to fetch or edit through the transposed view.
        SourceCaps::default()
    }

    fn ensure_rows(&self, _upto: u64, _cx: &mut App) {}
}
