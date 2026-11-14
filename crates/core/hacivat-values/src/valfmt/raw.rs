//! `raw_text`: the locale-free canonical text of a value (I18N-11; Task 16).

use std::borrow::Cow;

use crate::valfmt::ColumnFacts;
use crate::value::ValueRef;

/// A value's raw text, or SQL NULL.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RawText<'a> {
    Null,
    Text(Cow<'a, str>),
}

/// The locale-free text of `v` (`1.50`, `2026-11-13 10:00:00.123456+03`, `\x00ff`, `NaN`):
/// copy, sort keys and SQL values use it, never the display text.
pub fn raw_text<'a>(v: ValueRef<'a>, col: &ColumnFacts<'_>) -> RawText<'a> {
    let _ = (v, col);
    unimplemented!("valfmt::raw_text: Task 16")
}
