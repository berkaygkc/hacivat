//! Go to row (HV-GRID-14; M3b Task 37): what the user typed into the go-to field, parsed.
//!
//! Rows are 1-based **view** rows; the format locale's digits and grouping are accepted (`1.000`
//! in `tr` is row 1000); a column is named after a comma or a colon, by its 1-based index, its
//! exact name, its ASCII case-insensitive name, then the tolerant key.
//!
//! ## m3b-t01b-seams: the types and a fail-closed stub (Task 1b): [`parse_goto`] answers
//! `Err(GotoError::Empty)`, so nothing jumps. Task 37 fills it.

use crate::source::{ColId, ColumnSpec};

/// Where to go: a view row, 1-based as typed (`"1.000"` in `tr` is `Row(1000)`), maybe a
/// column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoTarget {
    Row(u64),
    RowCol(u64, ColId),
}

/// Why the text names no place to go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoError {
    Empty,
    NotANumber,
    /// Past the last row (`last`: the last row when the total is known).
    OutOfRange {
        last: Option<u64>,
    },
    UnknownColumn,
    HiddenColumn(ColId),
}

/// Parses `text` against `columns` with the format locale's `decimal` and `group` separators.
/// STUB: `Err(GotoError::Empty)`.
pub fn parse_goto(
    text: &str,
    columns: &[ColumnSpec],
    decimal: char,
    group: char,
) -> Result<GotoTarget, GotoError> {
    let _ = (text, columns, decimal, group);
    Err(GotoError::Empty)
}
