//! Dialect folding and comparison (Task 3).
//!
//! ASCII-only (INV-3): PostgreSQL lowercases unquoted ASCII letters; MySQL, MariaDB and SQLite
//! keep the text (their function names compare ASCII-case-insensitively). Locale casing is
//! never used, so `İ`, `ı` and every other non-ASCII letter stay exactly as written.

use std::sync::Arc;

use sql_dialect::DialectId;

use crate::ident::Ident;

/// PostgreSQL's `NAMEDATALEN - 1`: the server truncates longer identifiers to this many bytes
/// (at a character boundary, `pg_mbcliplen`).
pub const PG_MAX_IDENT_BYTES: usize = 63;

/// The identifier's text as `d` resolves it.
///
/// - PostgreSQL (and the PG-wire dialects): unquoted ASCII letters are lowered, then the text
///   is truncated to [`PG_MAX_IDENT_BYTES`] at a character boundary (quoted or not), as the
///   server's `downcase_truncate_identifier` and `truncate_identifier` do.
/// - Every other dialect: the text unchanged; comparison is [`ident_eq`]'s business.
pub fn fold(d: DialectId, raw: &str, quoted: bool) -> Arc<str> {
    if !is_pg_family(d) {
        return Arc::from(raw);
    }
    let cut = pg_truncated(raw);
    if quoted || !cut.bytes().any(|b| b.is_ascii_uppercase()) {
        Arc::from(cut)
    } else {
        Arc::from(cut.to_ascii_lowercase())
    }
}

/// Whether `a` and `b` name the same object in `d`.
///
/// - PostgreSQL: byte equality after [`fold`].
/// - MySQL, MariaDB, SQLite and the others: ASCII-case-insensitive equality of the texts,
///   quoted or not (column, function and SQLite table names compare this way; a MySQL table
///   name on a case-sensitive file system does not, so callers that must match a table exactly
///   compare bytes themselves).
pub fn ident_eq(d: DialectId, a: &Ident, b: &Ident) -> bool {
    if is_pg_family(d) {
        fold(d, &a.text, a.quoted) == fold(d, &b.text, b.quoted)
    } else {
        a.text.eq_ignore_ascii_case(&b.text)
    }
}

pub(crate) fn is_pg_family(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

/// `raw` cut to at most [`PG_MAX_IDENT_BYTES`] bytes, at a character boundary.
fn pg_truncated(raw: &str) -> &str {
    if raw.len() <= PG_MAX_IDENT_BYTES {
        return raw;
    }
    let mut end = PG_MAX_IDENT_BYTES;
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    &raw[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_ascii_text_is_never_cased() {
        assert_eq!(&*fold(DialectId::Postgres, "MÜŞTERİ", false), "mÜŞterİ");
        assert_eq!(&*fold(DialectId::Postgres, "Müşteri", false), "müşteri");
        let a = Ident {
            text: "T".into(),
            quoted: false,
        };
        let b = Ident {
            text: "t".into(),
            quoted: false,
        };
        assert!(ident_eq(DialectId::MySql, &a, &b));
        assert!(ident_eq(DialectId::Postgres, &a, &b));
    }
}
