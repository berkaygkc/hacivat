//! `format_cell` (Task 16).

use crate::valfmt::{ColumnFacts, Display, DisplaySettings, Purpose};
use crate::value::ValueRef;

/// The display text of `v` in column `col` under `s`, for `p` (HV-GRID-10, L3).
pub fn format_cell(
    v: ValueRef<'_>,
    col: &ColumnFacts<'_>,
    s: &DisplaySettings,
    p: Purpose,
) -> Display {
    let _ = (v, col, s, p);
    unimplemented!("valfmt::format_cell: Task 16")
}
