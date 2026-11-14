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
mod keys;
mod scroll;
mod selection;

pub use columns::ColumnLayout;
pub use keys::{Dir, KeyCtx, KeyOutcome, apply_command};
pub use scroll::{DEFAULT_ROW_HEIGHT, MIN_THUMB_PX, OVERSCAN_ROWS, ScrollState};
pub use selection::{CellRect, CopyRefusal, CopyRegion, MAX_COPY_CELLS, Selection, ViewMap};
