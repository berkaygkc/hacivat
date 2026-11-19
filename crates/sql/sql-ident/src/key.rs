//! Identifier keys (HV-INTRO-14, R-M4-28, R-M4-41): how a catalog name and a typed probe turn
//! into the one lookup key the model, the resolver, DDL and the tools compare.
//!
//! An [`IdentKey`] is built only by [`match_key`]; it is never displayed and never persisted.
//! The fold is the server's own rule, never the process locale (ASCII-only casing, INV-3).
//!
//! M4 Task 1a seam: [`match_key`] fails closed (always [`ServerDependent`], so every lookup is
//! `Unknown`, never a wrong `Found` or `Missing`) until Task 3 writes the per-dialect rules.

use std::sync::Arc;

/// The dialect a key belongs to (re-exported: [`IdentCtx`] names it).
pub use sql_dialect::DialectId;

/// A lookup key: a name folded by its dialect's rule for its [`NameClass`]. Built only by
/// [`match_key`]; never displayed, never persisted.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentKey(Arc<str>);

impl IdentKey {
    /// The folded text (for map lookups and tests; never for display).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// `Hash`/`Eq` of `IdentKey` are those of the `str` it holds, so maps keyed by it can be probed
// with a borrowed `&str` of already-folded text.
impl std::borrow::Borrow<str> for IdentKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// What the matching rule of one server depends on besides its dialect: MySQL/MariaDB
/// `lower_case_table_names` (0, 1 or 2) and the server encoding (PostgreSQL non-UTF-8 encodings
/// make non-ASCII input `ServerDependent`, HV-INTRO-14 rule 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentCtx {
    pub dialect: DialectId,
    pub lower_case_table_names: u8,
    pub encoding: Option<Arc<str>>,
}

impl IdentCtx {
    /// The context of `dialect` with `lower_case_table_names = 0` and no known encoding.
    pub fn new(dialect: DialectId) -> Self {
        IdentCtx {
            dialect,
            lower_case_table_names: 0,
            encoding: None,
        }
    }
}

/// The class of the name being matched: MySQL folds database and table names by
/// `lower_case_table_names` and everything else ASCII-caselessly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameClass {
    Database,
    Schema,
    Table,
    Column,
    Routine,
    Other,
}

/// The stored per-namespace fold policy of catalog names (the model spike's rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldRule {
    /// Byte equality.
    Exact,
    /// ASCII letters compare case-insensitively; every other character exactly.
    AsciiCaseless,
}

/// The server's matching rule for this text is not pinned (R-M4-41): a lookup answers `Found`
/// only on an exact stored-name match and `Unknown(ServerDependent)` otherwise, never `Missing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerDependent;

/// The key of `text` (a catalog name, or user input with its `quoted` flag) of `class` under
/// `d` and `ctx`.
///
/// Task 1a stub: always `Err(ServerDependent)`.
pub fn match_key(
    d: DialectId,
    ctx: &IdentCtx,
    text: &str,
    quoted: bool,
    class: NameClass,
) -> Result<IdentKey, ServerDependent> {
    let _ = (d, ctx, text, quoted, class);
    Err(ServerDependent)
}
