// ## m3b-t01c-core-seams (P-M3b-17)
//! `Purpose::A11y` (Task 40): what the grid's screen-reader subtree reads for a cell. Its own
//! file, so Task 40 never shares `display.rs` with Task 38.
//!
//! STUB until Task 40: the cell's text (M3a formats every purpose alike).

use crate::valfmt::{ColumnFacts, Display, DisplaySettings};
use crate::value::ValueRef;

/// The screen-reader text of `v` in column `col` under `s`. STUB: the cell's.
pub(super) fn text(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) -> Display {
    super::display::cell(v, col, s)
}
