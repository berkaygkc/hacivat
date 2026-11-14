//! The data grid (HV-GRID-01..05, -10 use, -16 role): a database-agnostic model (no GPUI in
//! `model/`), one custom element with layered regions and fast-scroll, the view, and a virtual
//! list. Data arrives through a [`GridDataSource`] (the app's result adapter); reaches GPUI only
//! through `hacivat_ui_core::gpui_surface`.
//!
//! Types complete since M3a Task 1b. Bodies: Task 17 (`model`), Task 20 (`element`, `view`,
//! `list`, `testing`, the synthetic B1 example). The paint path's per-frame statistics
//! ([`element::FrameStats`], [`element::FrameLog`]) are public, for the benches and the app's
//! tests.

pub mod commands;
pub mod element;
pub mod list;
pub mod model;
mod source;
#[cfg(feature = "test-support")]
pub mod testing;
mod view;

pub use commands::{GRID_COMMANDS, grid_command};
pub use list::{RowRenderer, VirtualList};
pub use source::{
    Align, CellContent, CellKind, ColId, ColumnKind, ColumnSpec, GRID_CONTEXT, GridCellFlags,
    GridDataSource, GridFrame, ModelCell, ModelRow, NULL_MARKER, RowCount, SourceCaps, ViewRow,
};
pub use view::{GridCommand, GridConfig, GridEvent, GridLabels, GridView};
