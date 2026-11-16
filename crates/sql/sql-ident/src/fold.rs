//! Dialect folding and comparison (Task 3).
//!
//! ASCII-only (INV-3): PostgreSQL lowercases unquoted ASCII letters; MySQL, MariaDB and SQLite
//! keep the text (their function names compare ASCII-case-insensitively). Locale casing is
//! never used.
//!
//! STUB until Task 3: [`fold`] returns the text unchanged and [`ident_eq`] is byte equality.

use std::sync::Arc;

use sql_dialect::DialectId;

use crate::ident::Ident;

/// The identifier's text as `d` resolves it. STUB: `raw` unchanged.
pub fn fold(d: DialectId, raw: &str, quoted: bool) -> Arc<str> {
    let _ = (d, quoted);
    Arc::from(raw)
}

/// Whether `a` and `b` name the same object in `d`. STUB: byte equality of the texts.
pub fn ident_eq(d: DialectId, a: &Ident, b: &Ident) -> bool {
    let _ = d;
    a.text == b.text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_change_nothing() {
        assert_eq!(&*fold(DialectId::Postgres, "Müşteri", false), "Müşteri");
        let a = Ident {
            text: "T".into(),
            quoted: false,
        };
        let b = Ident {
            text: "t".into(),
            quoted: false,
        };
        assert!(!ident_eq(DialectId::MySql, &a, &b));
        assert!(ident_eq(DialectId::MySql, &a, &a.clone()));
    }
}
