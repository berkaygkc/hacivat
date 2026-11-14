//! Parameters and variables (`alg/10` LEX-02 rule 8): `:name`, psql `:'v'`/`:"v"`/`:{?v}`,
//! MySQL `@user`/`@@system`, SQLite `$`/`@`/`:`/`#` variables. `?` and PostgreSQL's `$n` are
//! read by the cursor and `dollar`. Which of them reach the server as parameters is
//! SQL-PARAM-01's decision (M3b); the lexer only names their shape.

use sql_dialect::lex_profiles::Colon;

use crate::cursor::{Cursor, Scan};
use crate::token::{LexErrorCode, TokenKind};

/// A psql variable or `:name` character (no `$`).
fn name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// The token at the `:` at `pos` (PostgreSQL, MySQL, generic).
pub(crate) fn colon(c: &mut Cursor, pos: usize) -> Scan {
    let rule = c.p.sigils.colon;
    let op = |len: usize| Scan::new(TokenKind::Op, pos + len);
    match (rule, c.at(pos + 1)) {
        (Colon::Postgres | Colon::NamedParam, Some(b':')) | (_, Some(b'=')) => op(2),
        (Colon::Postgres, Some(q @ (b'\'' | b'"'))) => psql_quoted(c, pos, q).unwrap_or(op(1)),
        (Colon::Postgres, Some(b'{')) => psql_defined(c, pos).unwrap_or(op(1)),
        (Colon::Postgres | Colon::NamedParam, Some(b))
            if b.is_ascii_alphabetic() || b == b'_' || b >= 0x80 =>
        {
            Scan::new(TokenKind::NamedParam, c.skip_while(pos + 2, name_char))
        }
        _ => op(1),
    }
}

/// psql `:'name'` or `:"name"`.
fn psql_quoted(c: &mut Cursor, pos: usize, quote: u8) -> Option<Scan> {
    let end = c.skip_while(pos + 2, name_char);
    (end > pos + 2 && c.at(end) == Some(quote)).then(|| Scan::new(TokenKind::ClientVar, end + 1))
}

/// psql `:{?name}`.
fn psql_defined(c: &mut Cursor, pos: usize) -> Option<Scan> {
    if c.at(pos + 2) != Some(b'?') {
        return None;
    }
    let end = c.skip_while(pos + 3, name_char);
    (end > pos + 3 && c.at(end) == Some(b'}')).then(|| Scan::new(TokenKind::ClientVar, end + 1))
}

/// MySQL/MariaDB `@@system` and `@user` variables; a bare `@` (as in `'u'@'host'`) is an
/// operator, and the quoted name after it is its own token.
pub(crate) fn mysql_var(c: &mut Cursor, pos: usize) -> Scan {
    let system = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80;
    if c.at(pos + 1) == Some(b'@') {
        let end = c.skip_while(pos + 2, system);
        if end > pos + 2 {
            return Scan::new(TokenKind::SystemVar, end);
        }
    }
    let end = c.skip_while(pos + 1, |b| system(b) || b == b'.');
    if end > pos + 1 {
        Scan::new(TokenKind::UserVar, end)
    } else {
        Scan::new(TokenKind::Op, pos + 1)
    }
}

/// SQLite's variables `$name`, `@name`, `:name`, `#name`: identifier characters, Tcl `::`
/// segments, and a `(…)` suffix that must close before whitespace (else the token is
/// illegal); with no identifier character the token is illegal.
pub(crate) fn tcl(c: &mut Cursor, pos: usize) -> Scan {
    let mut i = pos + 1;
    let mut chars = 0usize;
    loop {
        match c.at(i) {
            Some(b) if c.ident_continue(b) => {
                chars += 1;
                i += 1;
            }
            Some(b'(') if chars > 0 => {
                let stop = c.skip_while(i + 1, |b| {
                    b != b')' && b != 0 && !b.is_ascii_whitespace() && b != 0x0b
                });
                return if c.at(stop) == Some(b')') {
                    Scan::new(TokenKind::NamedParam, stop + 1)
                } else {
                    Scan::new(TokenKind::NamedParam, stop).rejected(LexErrorCode::InvalidLiteral)
                };
            }
            Some(b':') if c.at(i + 1) == Some(b':') => i += 2,
            _ => break,
        }
    }
    if chars == 0 {
        Scan::new(TokenKind::Unknown, i).rejected(LexErrorCode::UnknownCharacter)
    } else {
        Scan::new(TokenKind::NamedParam, i)
    }
}
