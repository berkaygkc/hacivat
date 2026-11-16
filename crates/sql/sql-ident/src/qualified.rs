//! Qualified names from text (Task 3).
//!
//! STUB until Task 3: [`parse_qualified`] answers `None` (nothing is recognised).

use sql_dialect::DialectId;

use crate::ident::QualifiedName;

/// `text` read as a (possibly quoted, possibly qualified) name in `d`. STUB: `None`.
pub fn parse_qualified(d: DialectId, text: &str) -> Option<QualifiedName> {
    let _ = (d, text);
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_recognises_nothing() {
        assert_eq!(parse_qualified(DialectId::Postgres, "public.t"), None);
    }
}
