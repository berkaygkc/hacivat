//! Identifier quoting and string literals (alg/03 HV-DDL-07 "identifier quoting only through
//! `needs_quoting`/`quote`", R-M4-28, R-M4-37): one function for the model, DDL, `SHOW CREATE`
//! macros, M6's `valfmt::literal` and M8's tools.
//!
//! M4 Task 7 bodies (spike origin `da358e4`, moved here from `hacivat-ddl` as R-M4-28 asks).
//! Rules, ASCII only and never the process locale (INV-3):
//! - **PostgreSQL** (and the PG-wire dialects) follow the server's own `quote_identifier`
//!   (`ruleutils.c`): a name stays bare only when it is `[a-z_][a-z0-9_]*` and not a keyword
//!   other than an unreserved one. Upper case and every non-ASCII letter are quoted, so
//!   `MÜŞTERİ` is `"MÜŞTERİ"` (unquoted it would fold ASCII-only to `mÜŞterİ`).
//! - **MySQL / MariaDB** keep a name bare when it is `[A-Za-z_][A-Za-z0-9_]*` and not a
//!   reserved word. MariaDB's keyword table has no reserved flag, so every MariaDB keyword is
//!   quoted. The quote character is **always the backtick** (T1a review m2): backticks quote
//!   identifiers in every `sql_mode`, while `"` under a wrongly assumed `ANSI_QUOTES` would
//!   turn a name into a string literal. `QuoteCtx::ansi_quotes` is never trusted for that.
//! - **SQLite**: `[A-Za-z_][A-Za-z0-9_]*` and not a keyword.
//! - Every other dialect: always quoted with `"`.
//!
//! The quoting functions take no server version, so a word counts as a keyword when it is one
//! on **any** server line the keyword tables know (over-quoting is never wrong: a quoted
//! identifier names exactly itself). A bare name is never case-mapped; a quoted one doubles
//! its quote character. A NUL is refused everywhere: no dialect can quote it.

use sql_dialect::keywords::keyword_lookup_at;
use sql_dialect::{DialectId, KwCategory, ServerVersion};

/// The session settings that change quoting, as the server reported them; `None` when they are
/// not known (yet). An unknown setting makes [`string_literal`] pick a form that is safe under
/// either value (T1a review m2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct QuoteCtx {
    /// MySQL / MariaDB `ANSI_QUOTES` in `sql_mode`. Carried for completeness and **never
    /// trusted**: identifiers are always backtick-quoted on MySQL and MariaDB, and string
    /// literals always use `'`, which reads the same in either mode.
    pub ansi_quotes: Option<bool>,
    /// MySQL / MariaDB `NO_BACKSLASH_ESCAPES` in `sql_mode`.
    pub no_backslash_escapes: Option<bool>,
    /// PostgreSQL `standard_conforming_strings`.
    pub standard_conforming_strings: Option<bool>,
}

/// Why a name or literal cannot be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuoteError {
    /// The text holds a NUL character, which no dialect can quote.
    Nul,
    /// Never produced since Task 7 filled the bodies; kept so existing `match`es stay valid.
    NotImplemented,
}

fn is_pg(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

fn is_mysql(d: DialectId) -> bool {
    matches!(d, DialectId::MySql | DialectId::MariaDb)
}

/// The identifier quote character of `d`: the backtick on MySQL and MariaDB whatever
/// `ANSI_QUOTES` says, `"` everywhere else.
fn quote_char(d: DialectId) -> char {
    if is_mysql(d) { '`' } else { '"' }
}

/// The oldest server line of every keyword table: a row gated `until` a version holds there,
/// a row gated `since` one holds on the newest line (`None`). Every gate in the tables is
/// one-sided, so the two probes see every row.
const OLDEST: Option<ServerVersion> = Some(ServerVersion::new(0, 0, 0));

/// Whether `word` is a keyword that `d` will not take as a bare identifier on some server line.
fn reserved_somewhere(d: DialectId, word: &str) -> bool {
    [None, OLDEST].into_iter().any(|v| {
        keyword_lookup_at(d, v, word).is_some_and(|kw| match d {
            d if is_pg(d) => kw.category != KwCategory::Unreserved,
            DialectId::MySql => kw.category == KwCategory::Reserved,
            // MariaDB and SQLite report no reserved flag: every keyword is quoted.
            _ => true,
        })
    })
}

/// Whether `text` must be quoted to name itself exactly on `d`: a reserved word, upper case on
/// PostgreSQL, a character outside the dialect's bare identifier set (non-ASCII, a space, a
/// trailing space, a quote), a leading digit or an empty name. `ctx` changes nothing here: no
/// session setting makes a bare name safe that is otherwise quoted.
pub fn needs_quoting(d: DialectId, text: &str, ctx: &QuoteCtx) -> bool {
    let _ = ctx;
    let pg = is_pg(d);
    if !(pg || is_mysql(d) || d == DialectId::Sqlite) {
        return true;
    }
    let bytes = text.as_bytes();
    let Some(&first) = bytes.first() else {
        return true;
    };
    let start_ok = if pg {
        first.is_ascii_lowercase() || first == b'_'
    } else {
        first.is_ascii_alphabetic() || first == b'_'
    };
    let rest_ok = bytes[1..].iter().all(|&b| {
        if pg {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
        } else {
            b.is_ascii_alphanumeric() || b == b'_'
        }
    });
    !start_ok || !rest_ok || reserved_somewhere(d, text)
}

/// `text` as an identifier of `d`: bare when [`needs_quoting`] allows it, else inside the
/// dialect's quote character with that character doubled. A NUL is refused.
pub fn quote(d: DialectId, text: &str, ctx: &QuoteCtx) -> Result<String, QuoteError> {
    if text.contains('\0') {
        return Err(QuoteError::Nul);
    }
    if !needs_quoting(d, text, ctx) {
        return Ok(text.to_owned());
    }
    let q = quote_char(d);
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

/// The parts of a qualified name, outermost first, each through [`quote`], joined by `.`.
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

/// `'text'` with `'` doubled and, when `double_backslash`, `\` doubled.
fn single_quoted(text: &str, double_backslash: bool) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' if double_backslash => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// A PostgreSQL dollar-quoted string: no escape is processed inside it whatever
/// `standard_conforming_strings` says. The tag is the first of `hv`, `hv1`, `hv2`, … such that
/// `$tag` does not occur in `text`, so the closing `$tag$` is the first one after the opening.
fn dollar_quoted(text: &str) -> String {
    let mut tag = String::from("hv");
    let mut n = 0u32;
    while text.contains(&format!("${tag}")) {
        n += 1;
        tag = format!("hv{n}");
    }
    format!("${tag}${text}${tag}$")
}

/// `text` as a string literal of `d` under `ctx`. Newlines, `--`, `/*` and every other
/// character stay as they are inside the quotes; a NUL is refused.
///
/// - PostgreSQL: `'…'` with `'` doubled. When the text holds a backslash and
///   `standard_conforming_strings` is not known to be on, a dollar-quoted string instead,
///   which reads the same in either mode (`E'…'` is never produced).
/// - MySQL / MariaDB: `'…'` with `'` doubled and `\` doubled unless `NO_BACKSLASH_ESCAPES` is
///   known to be on. Unknown is treated as off: doubling is exact under the server default
///   and, under `NO_BACKSLASH_ESCAPES`, keeps an extra backslash but can never end the literal
///   early.
/// - SQLite and every other dialect: `'…'` with `'` doubled.
pub fn string_literal(d: DialectId, text: &str, ctx: &QuoteCtx) -> Result<String, QuoteError> {
    if text.contains('\0') {
        return Err(QuoteError::Nul);
    }
    if is_pg(d) {
        if text.contains('\\') && ctx.standard_conforming_strings != Some(true) {
            return Ok(dollar_quoted(text));
        }
        return Ok(single_quoted(text, false));
    }
    if is_mysql(d) {
        return Ok(single_quoted(text, ctx.no_backslash_escapes != Some(true)));
    }
    Ok(single_quoted(text, false))
}
