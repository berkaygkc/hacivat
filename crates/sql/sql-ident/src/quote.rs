//! Identifier quoting and string literals (alg/03 HV-DDL-07 "identifier quoting only through
//! `needs_quoting`/`quote`", R-M4-28, R-M4-37): one function for the model, DDL, `SHOW CREATE`
//! macros, M6's `valfmt::literal` and M8's tools.
//!
//! M4 Task 1a seam until Task 7: [`needs_quoting`] is always true and [`quote`] always quotes
//! with the dialect's quote character doubled inside, which over-quotes but is never wrong (a
//! quoted identifier keeps its exact case on every dialect). [`string_literal`] answers
//! [`QuoteError::NotImplemented`]. A NUL is refused everywhere: no dialect can quote it.

use sql_dialect::DialectId;

/// The session settings that change quoting: MySQL/MariaDB `ANSI_QUOTES` and
/// `NO_BACKSLASH_ESCAPES` (from `sql_mode`), PostgreSQL `standard_conforming_strings`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuoteCtx {
    pub ansi_quotes: bool,
    pub no_backslash_escapes: bool,
    pub standard_conforming_strings: bool,
}

impl Default for QuoteCtx {
    /// The servers' defaults: no `ANSI_QUOTES`, backslash escapes on (MySQL), standard
    /// conforming strings on (PostgreSQL ≥ 9.1).
    fn default() -> Self {
        QuoteCtx {
            ansi_quotes: false,
            no_backslash_escapes: false,
            standard_conforming_strings: true,
        }
    }
}

/// Why a name or literal cannot be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuoteError {
    /// The text holds a NUL character, which no dialect can quote.
    Nul,
    /// Not written yet (Task 7).
    NotImplemented,
}

/// The identifier quote character of `d` under `ctx`.
fn quote_char(d: DialectId, ctx: &QuoteCtx) -> char {
    match d {
        DialectId::MySql | DialectId::MariaDb if !ctx.ansi_quotes => '`',
        _ => '"',
    }
}

/// Whether `text` must be quoted to name itself exactly on `d`.
///
/// Task 1a stub: always true (over-quoting is never wrong).
pub fn needs_quoting(d: DialectId, text: &str, ctx: &QuoteCtx) -> bool {
    let _ = (d, text, ctx);
    true
}

/// `text` as a quoted identifier of `d`: the quote character around it, doubled inside.
pub fn quote(d: DialectId, text: &str, ctx: &QuoteCtx) -> Result<String, QuoteError> {
    if text.contains('\0') {
        return Err(QuoteError::Nul);
    }
    let q = quote_char(d, ctx);
    let mut out = String::with_capacity(text.len() + 2);
    out.push(q);
    for c in text.chars() {
        if c == q {
            out.push(q);
        }
        out.push(c);
    }
    out.push(q);
    Ok(out)
}

/// The quoted parts of a qualified name, outermost first, joined by `.`.
pub fn quote_qualified(d: DialectId, parts: &[&str], ctx: &QuoteCtx) -> Result<String, QuoteError> {
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&quote(d, part, ctx)?);
    }
    Ok(out)
}

/// `text` as a string literal of `d` under `ctx`.
///
/// Task 1a stub: `Err(NotImplemented)` (a NUL is refused first).
pub fn string_literal(d: DialectId, text: &str, ctx: &QuoteCtx) -> Result<String, QuoteError> {
    let _ = (d, ctx);
    if text.contains('\0') {
        return Err(QuoteError::Nul);
    }
    Err(QuoteError::NotImplemented)
}
