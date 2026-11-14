//! Table-driven lexer profiles, one per dialect (SQL-LEX-02; Task 4).
//!
//! Task 1b stub: every supported dialect gets the same empty placeholder profile (the lexer
//! stub reads nothing from it); `SqlServer`, `ClickHouse` and `Oracle` are unsupported in M3a.

mod generic;
mod mariadb;
mod mysql;
mod pg;
mod sqlite;

use crate::id::{DialectCtx, DialectId};

/// One dialect's lexer tables (opaque; Task 4 fills it).
#[derive(Debug)]
pub struct LexDialect {
    name: &'static str,
}

impl LexDialect {
    /// The profile's name, for diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// A dialect without a lexer profile in M3a (R19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsupportedDialect(pub DialectId);

/// The placeholder profile of Task 1b.
static PLACEHOLDER: LexDialect = LexDialect {
    name: "placeholder",
};

/// The lexer profile for `ctx` (its dialect, version and modes; Task 4).
pub fn lex_profile(ctx: &DialectCtx) -> Result<&'static LexDialect, UnsupportedDialect> {
    match ctx.id {
        DialectId::SqlServer | DialectId::ClickHouse | DialectId::Oracle => {
            Err(UnsupportedDialect(ctx.id))
        }
        _ => Ok(&PLACEHOLDER),
    }
}
