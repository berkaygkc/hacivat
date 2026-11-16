//! The grid's data seam (HV-GRID-01): what a source gives the grid each frame.

use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{App, SharedString};

/// SQL NULL in a cell (D-065): a constant, never a Fluent message.
pub const NULL_MARKER: &str = "NULL";

/// The GPUI key context of a focused grid (keymap rows `context = "Grid"`).
pub const GRID_CONTEXT: &str = "Grid";

/// A row of the source, in source order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelRow(pub u64);

/// A row as the grid shows it (after a sort, M3b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ViewRow(pub u64);

/// A column by its ordinal (duplicate names keep their order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ColId(pub u32);

/// One cell of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelCell {
    pub row: ModelRow,
    pub col: ColId,
}

/// What a column holds, for alignment, default width and the header subtitle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnKind {
    Text,
    Number,
    Bool,
    Temporal,
    Uuid,
    Binary,
    Json,
    Other,
}

/// Horizontal alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Align {
    #[default]
    Start,
    End,
    Center,
}

/// A column: its name (catalog text, never translated), its type label (`numeric(14,2)`),
/// kind, alignment, default width, and whether it is a key.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnSpec {
    pub id: ColId,
    pub name: SharedString,
    pub type_label: SharedString,
    pub kind: ColumnKind,
    pub align: Align,
    pub default_width: f32,
    pub key: bool,
}

/// What a source can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceCaps {
    pub has_cursor: bool,
    pub editable: bool,
}

/// How many rows the source has loaded, whether more come, and the exact total when known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RowCount {
    pub loaded: u64,
    pub more: bool,
    pub exact_total: Option<u64>,
}

/// How a cell is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CellKind {
    Text,
    Number,
    /// `NULL_MARKER` in synthesized small caps at `text-3`.
    Null,
    Empty,
    Bool,
    Temporal,
    Binary,
    Json,
    /// A decode error: an ink glyph.
    Error,
    /// A row past `loaded`: a static bar.
    Placeholder,
}

bitflags::bitflags! {
    /// What a cell's text carries.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct GridCellFlags: u8 {
        const TRUNCATED = 1;
        const LOSSY = 2;
        const HAS_CONTROLS = 4;
        const MULTILINE = 8;
        const RTL = 16;
    }
}

/// One formatted cell (data text, never pseudo-localised).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellContent {
    pub text: SharedString,
    pub kind: CellKind,
    pub flags: GridCellFlags,
    pub align: Align,
    /// A chip after the text (`… 5 MB`), already rendered through Fluent by the app.
    pub chip: Option<SharedString>,
}

/// One frame's view of a source: one snapshot load per frame, no lock, no I/O.
pub trait GridFrame {
    /// `(generation, version)`: a new generation (a re-run) clears the selection.
    fn generation(&self) -> (u64, u64);
    fn row_count(&self) -> RowCount;
    fn cell(&self, row: ModelRow, col: ColId) -> CellContent;
}

/// A grid's data (the app's result adapter; the synthetic source in tests).
pub trait GridDataSource: 'static {
    /// The frame's snapshot (one load per frame).
    fn begin_frame(&self, cx: &App) -> Rc<dyn GridFrame>;
    fn columns(&self) -> Arc<[ColumnSpec]>;
    fn caps(&self) -> SourceCaps;
    /// Asks for rows up to `upto`: fire-and-forget; the source coalesces.
    fn ensure_rows(&self, upto: u64, cx: &mut App);
}
