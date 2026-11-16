//! Qualified names from text (Task 3).
//!
//! `parse_qualified` reads `name`, `schema.name` or `db.schema.name` the way the dialect's
//! lexer delimits identifiers: dots split parts only outside quotes, whitespace may surround a
//! dot, and a quoted part keeps its text exactly (doubled quotes undone). Anything else (an
//! empty part, an unterminated quote, an operator character, PostgreSQL's `U&"…"` form) is not
//! a name: `None`, which callers treat as unknown.

use smallvec::SmallVec;
use sql_dialect::DialectId;

use crate::fold::is_pg_family;
use crate::ident::{Ident, QualifiedName};

/// How a dialect quotes identifiers: `(open, close)` pairs; a doubled `close` inside the
/// quotes stands for one.
fn quotes(d: DialectId) -> &'static [(char, char)] {
    match d {
        // A double-quoted MySQL token is a string unless ANSI_QUOTES is on: never a name here.
        DialectId::MySql | DialectId::MariaDb => &[('`', '`')],
        DialectId::Sqlite => &[('"', '"'), ('`', '`'), ('[', ']')],
        DialectId::SqlServer => &[('"', '"'), ('[', ']')],
        _ => &[('"', '"')],
    }
}

/// A character an unquoted identifier may hold: ASCII letters, digits, `_`, `$`, or any
/// non-ASCII character (the lexers accept letters above U+007F).
fn is_bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$' || !c.is_ascii()
}

/// `text` read as a (possibly quoted, possibly qualified) name in `d`; `None` when it is not
/// one. The parts keep their text as written: [`crate::fold`] resolves case.
pub fn parse_qualified(d: DialectId, text: &str) -> Option<QualifiedName> {
    let quotes = quotes(d);
    let mut parts: SmallVec<[Ident; 3]> = SmallVec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let first = *chars.peek()?;
        let part = if let Some(&(_, close)) = quotes.iter().find(|(open, _)| *open == first) {
            chars.next();
            let mut s = String::new();
            loop {
                let c = chars.next()?;
                if c == close {
                    if chars.next_if_eq(&close).is_some() {
                        s.push(close);
                    } else {
                        break;
                    }
                } else {
                    s.push(c);
                }
            }
            if s.is_empty() {
                return None;
            }
            Ident {
                text: s.into(),
                quoted: true,
            }
        } else {
            let mut s = String::new();
            while let Some(c) = chars.next_if(|c| is_bare(*c)) {
                s.push(c);
            }
            if s.is_empty() {
                return None;
            }
            // PostgreSQL's `U&"…"` and `u&"…"` carry escapes: not read here.
            if is_pg_family(d) && chars.peek() == Some(&'&') {
                return None;
            }
            Ident {
                text: s.into(),
                quoted: false,
            }
        };
        parts.push(part);
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.next() {
            None => return Some(QualifiedName { parts }),
            Some('.') => {}
            Some(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_to_three_parts() {
        let q = parse_qualified(DialectId::Postgres, "public.t").unwrap();
        assert_eq!(q.parts.len(), 2);
        let q = parse_qualified(DialectId::Postgres, "db.s.t").unwrap();
        assert_eq!(q.parts.len(), 3);
        assert_eq!(parse_qualified(DialectId::Postgres, "u&\"x\""), None);
    }
}
