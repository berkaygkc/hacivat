//! Identifier quoting and string literals per dialect (alg/03 HV-DDL-07 "identifier quoting
//! only through `needs_quoting`/`quote`"; ALG-I18N-05).
//!
//! The names here are the ones R-M4-28 gives `sql_ident::quote` (`QuoteCtx`, `needs_quoting`,
//! `quote`, `quote_qualified`, `string_literal`), so moving this file into `sql-ident` is a
//! file move. It lives in `hacivat-ddl` for the spike so M3's crate is not edited.
//!
//! Rules (ASCII only, never the process locale, INV-3):
//! - **PostgreSQL** follows the server's own `quote_identifier` (ruleutils.c): a name stays bare
//!   only when it is `[a-z_][a-z0-9_]*` and not a keyword other than an unreserved one.
//!   Everything else, upper case and every non-ASCII letter included, is quoted. So
//!   `MÜŞTERİ` is `"MÜŞTERİ"`: unquoted it would fold ASCII-only to `mÜŞterİ`.
//! - **MySQL / MariaDB** keep a name bare when it is `[A-Za-z_][A-Za-z0-9_]*` and not a
//!   reserved word of that server line (MariaDB reports no reserved flag, so every MariaDB
//!   keyword counts as reserved). Non-ASCII names are quoted (conservative: the connection
//!   character set is not part of the target).
//! - **SQLite**: `[A-Za-z_][A-Za-z0-9_]*` and not a keyword.
//!
//! A bare name is never case-mapped; a quoted one doubles its quote character.

use std::borrow::Cow;

use sql_dialect::keywords::keyword_lookup_at;
use sql_dialect::{DialectId, KwCategory, ServerVersion};

/// What quoting depends on: the dialect and the server line (keyword tables are versioned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteCtx {
    pub dialect: DialectId,
    pub version: Option<ServerVersion>,
}

impl QuoteCtx {
    pub fn new(dialect: DialectId, version: Option<ServerVersion>) -> Self {
        QuoteCtx { dialect, version }
    }
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn is_mysql(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

/// The quote character of `d`'s identifiers.
fn quote_char(d: DialectId) -> char {
    if is_mysql(d) { '`' } else { '"' }
}

/// Whether `name` must be quoted to mean exactly itself in `ctx`.
pub fn needs_quoting(ctx: QuoteCtx, name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(&first) = bytes.first() else {
        return true;
    };
    let pg = is_pg(ctx.dialect);
    let start_ok = if pg {
        first.is_ascii_lowercase() || first == b'_'
    } else {
        first.is_ascii_alphabetic() || first == b'_'
    };
    if !start_ok {
        return true;
    }
    let rest_ok = bytes[1..].iter().all(|&b| {
        if pg {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
        } else {
            b.is_ascii_alphanumeric() || b == b'_'
        }
    });
    if !rest_ok {
        return true;
    }
    match keyword_lookup_at(ctx.dialect, ctx.version, name) {
        None => false,
        Some(kw) => match ctx.dialect {
            d if is_pg(d) => kw.category != KwCategory::Unreserved,
            DialectId::MySql => kw.category == KwCategory::Reserved,
            // MariaDB and SQLite report no reserved flag: every keyword is quoted.
            _ => true,
        },
    }
}

/// `name` quoted unconditionally.
pub fn quote_always(ctx: QuoteCtx, name: &str) -> String {
    let q = quote_char(ctx.dialect);
    let mut out = String::with_capacity(name.len() + 2);
    out.push(q);
    for c in name.chars() {
        if c == q {
            out.push(q);
        }
        out.push(c);
    }
    out.push(q);
    out
}

/// `name` as it must appear in `ctx`: bare when it can be, else quoted.
pub fn quote(ctx: QuoteCtx, name: &str) -> Cow<'_, str> {
    if needs_quoting(ctx, name) {
        Cow::Owned(quote_always(ctx, name))
    } else {
        Cow::Borrowed(name)
    }
}

/// `parts` (outermost first) quoted one by one and joined with `.`.
pub fn quote_qualified(ctx: QuoteCtx, parts: &[&str]) -> String {
    let mut out = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(&quote(ctx, p));
    }
    out
}

/// What a string literal depends on beyond the dialect: the session settings that change how
/// the server reads a backslash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiteralCtx {
    pub dialect: DialectId,
    /// PostgreSQL `standard_conforming_strings` (on by default since 9.1). When off, a text
    /// with a backslash is written as an `E'…'` literal with the backslash doubled.
    pub pg_standard_conforming_strings: bool,
    /// MySQL / MariaDB `sql_mode` contains `NO_BACKSLASH_ESCAPES` (read from the session,
    /// HV-SESS-06). When off, a backslash is doubled.
    pub mysql_no_backslash_escapes: bool,
}

/// A string literal could not be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NulInText;

/// `text` as a string literal of `ctx`'s dialect. A NUL is refused: no supported server
/// stores one in text, and a literal that silently lost it would not be the user's text.
/// Newlines, `--`, `/*` and every other character stay as they are inside the quotes.
pub fn string_literal(ctx: LiteralCtx, text: &str) -> Result<String, NulInText> {
    if text.contains('\0') {
        return Err(NulInText);
    }
    let double_backslash = if is_pg(ctx.dialect) {
        !ctx.pg_standard_conforming_strings && text.contains('\\')
    } else if is_mysql(ctx.dialect) {
        !ctx.mysql_no_backslash_escapes
    } else {
        false
    };
    let mut out = String::with_capacity(text.len() + 3);
    if is_pg(ctx.dialect) && double_backslash {
        out.push('E');
    }
    out.push('\'');
    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' if double_backslash => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    Ok(out)
}
