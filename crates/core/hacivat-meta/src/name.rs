//! Server names and lookup keys (HV-MODEL-01 "Names", HV-INTRO-14, R-M4-28).
//!
//! A [`Name`] is the exact text the server reported: never normalized, never case-mapped,
//! displayed verbatim. Lookup keys ([`IdentKey`]) are built only by
//! [`sql_ident::key::match_key`], re-exported here so the model, the resolver and DDL share one
//! fold table.

use std::sync::Arc;

pub use sql_ident::key::{
    DialectId, FoldRule, IdentCtx, IdentKey, NameClass, ServerDependent, match_key,
};

/// An object name exactly as the server reported it. `Arc<str>`: the cache decoder interns the
/// strings of a namespace, so 160k columns share their few dozen type names (R-M4-26).
pub type Name = Arc<str>;

/// One part of a typed path (`schema.table`): its text without quotes and whether it was
/// quoted. The same type `sql_ident` parses.
pub type NamePart = sql_ident::Ident;

/// Whether `name` may hold a character a reader cannot see (bidi controls, zero-width
/// characters, U+202E …): the explorer shows a badge (`model-name-has-invisible-chars`).
///
/// M4 Task 1a stub, fail-closed until Task 3's exact table: flags every character that is
/// neither alphanumeric nor printable ASCII (so it never misses a bidi control or a zero-width
/// character, at the cost of also badging punctuation such as `€` or combining marks).
pub fn has_invisible_chars(name: &str) -> bool {
    name.chars()
        .any(|c| !(c.is_alphanumeric() || (c.is_ascii() && !c.is_ascii_control())))
}
