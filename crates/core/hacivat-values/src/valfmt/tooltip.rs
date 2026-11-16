// ## m3b-t01c-core-seams (P-M3b-17)
//! `Purpose::Tooltip` (Task 38): the raw value under a fraction cap for a cell's tooltip. Its
//! own file, so Task 38 never shares `display.rs` with Task 40.
//!
//! STUB until Task 38: the cell's text (M3a formats every purpose alike).

use crate::valfmt::{ColumnFacts, Display, DisplaySettings};
use crate::value::ValueRef;

/// The tooltip text of `v` in column `col` under `s`. STUB: the cell's.
pub(super) fn text(v: ValueRef<'_>, col: &ColumnFacts<'_>, s: &DisplaySettings) -> Display {
    super::display::cell(v, col, s)
}
