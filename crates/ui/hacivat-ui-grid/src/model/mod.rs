//! The grid model (HV-GRID-02 vertical and horizontal, -04 basic, -05 core; Task 17): no GPUI.
//! Absolute positions are `u64` rows and `f64` pixels; selections count cells in `u128`.
//!
//! - [`ScrollState`]: the row-anchored two-axis scroll position, follow-tail, thumb drags,
//!   deltas coalesced per frame and the fast-scroll velocity.
//! - [`ColumnLayout`]: view order, widths and `f64` prefix sums over the visible columns.
//! - [`Selection`]: anchor and focus, rectangles, row and column sets, select-all, counts and
//!   copy normalisation; [`ViewMap`] maps view rows to model rows.
//! - [`apply_command`]: what each `GridCommand` does to the selection.
//! - [`autosize`]: default widths, the sampled measure, the outlier trim and the fill.

pub mod autosize;
mod columns;
// ## m3b-t01b-seams: filter rows (Tasks 35, 37), frozen columns (Task 35), go to row (Task 37),
// local sort (Task 36).
pub mod filter;
mod frozen;
pub mod goto;
mod keys;
mod scroll;
mod selection;
pub mod sort;

pub use columns::ColumnLayout;
pub use keys::{Dir, KeyCtx, KeyOutcome, apply_command};
pub use scroll::{DEFAULT_ROW_HEIGHT, MIN_THUMB_PX, OVERSCAN_ROWS, ScrollState};
// ## m3c-t35: `GrowStep` and `ViewIndex` added.
pub use selection::{
    CellRect, CopyRefusal, CopyRegion, GrowStep, MAX_COPY_CELLS, Selection, ViewIndex, ViewMap,
};
// ## m3b-t01b-seams
pub use goto::{GotoError, GotoTarget, parse_goto};
pub use sort::{SortKey, SortSpec};
// ## m3c-t36: the local sort's stable, cancellable order.
pub use sort::{CANCEL_EVERY, Cancelled, PARALLEL_ABOVE, stable_order, stable_order_on};
