//! String literals and quoted identifiers (`alg/10` LEX-02 rules 4–5).
//!
//! Strings: `'…'` (MySQL also `"…"` unless `ANSI_QUOTES`), with doubled quotes; backslash
//! escapes in PostgreSQL `E'…'`, in PostgreSQL plain strings only with
//! `standard_conforming_strings` off, and in MySQL/MariaDB unless `NO_BACKSLASH_ESCAPES`.
//! Prefixes are ASCII case-insensitive and directly before the quote: `E`, `B`, `X`, `N`,
//! `U&`. Quoted identifiers: `"…"`, `` `…` ``, `[…]`, `U&"…"`. PostgreSQL's adjacent-literal
//! continuation is not merged: two strings across a newline are two tokens (the parser joins
//! them). An unterminated literal runs to the end of the text, flagged, with its resume state.

use sql_dialect::lex_profiles::Backslash;

use crate::cursor::{Cursor, Scan};
use crate::state::{LexState, StringRule};
use crate::token::{LexErrorCode, TokenFlags, TokenKind};

/// The rule of a plain string under the profile and the session's modes.
pub(crate) fn plain_rule(c: &Cursor) -> StringRule {
    let escapes = match c.p.quotes.backslash {
        Backslash::Never => false,
        Backslash::UnlessStandardConformingStrings => !c.modes.standard_conforming_strings,
        Backslash::UnlessNoBackslashEscapes => !c.modes.no_backslash_escapes,
    };
    match (escapes, c.p.quotes.backslash) {
        (false, _) => StringRule::Standard,
        (true, Backslash::UnlessNoBackslashEscapes) => StringRule::Backslash,
        (true, _) => StringRule::Escape,
    }
}

/// A plain `'…'` string at `pos`.
pub(crate) fn plain(c: &mut Cursor, pos: usize) -> Scan {
    let rule = plain_rule(c);
    string_at(c, pos, b'\'', rule)
}

/// A `String` whose opening `quote` is at `pos`.
pub(crate) fn string_at(c: &mut Cursor, pos: usize, quote: u8, rule: StringRule) -> Scan {
    string_from(c, pos + 1, quote, rule, TokenKind::String)
}

/// The rest of a string of `kind` whose body continues at `from`.
pub(crate) fn string_from(
    c: &mut Cursor,
    from: usize,
    quote: u8,
    rule: StringRule,
    kind: TokenKind,
) -> Scan {
    match string_end(c, from, quote, rule) {
        Some(end) => Scan::new(kind, end),
        None => Scan::unterminated(
            kind,
            c.len(),
            LexState::InString { rule, quote, kind },
            Some(LexErrorCode::UnterminatedString),
        ),
    }
}

/// The index after the quote that closes a string body starting at `i`.
fn string_end(c: &mut Cursor, mut i: usize, quote: u8, rule: StringRule) -> Option<usize> {
    loop {
        let found = if rule.backslash_escapes() {
            c.find2(i, quote, b'\\')?
        } else {
            c.find(i, quote)?
        };
        if c.b[found] != quote {
            // A backslash: the next character is text (a multi-byte one is skipped by the next
            // search, which only stops at ASCII bytes).
            i = found + 2;
            if i > c.len() {
                return None;
            }
            continue;
        }
        if rule.doubled_quotes() && c.b.get(found + 1) == Some(&quote) {
            i = found + 2;
            continue;
        }
        return Some(found + 1);
    }
}

/// A quoted identifier of `kind` starting at `start`, its opener `open_len` bytes long.
pub(crate) fn ident_at(
    c: &mut Cursor,
    start: usize,
    open_len: usize,
    close: u8,
    kind: TokenKind,
) -> Scan {
    ident_from(c, start + open_len, close, kind)
}

/// The rest of a quoted identifier whose body continues at `from`. A doubled closer is one
/// closer, except for `]` (SQLite's `[…]` has no escape).
pub(crate) fn ident_from(c: &mut Cursor, from: usize, close: u8, kind: TokenKind) -> Scan {
    let doubled = close != b']';
    let mut i = from;
    loop {
        let Some(found) = c.find(i, close) else {
            return Scan::unterminated(
                kind,
                c.len(),
                LexState::InQuotedIdent { close, kind },
                Some(LexErrorCode::UnterminatedQuotedIdent),
            );
        };
        if doubled && c.b.get(found + 1) == Some(&close) {
            i = found + 2;
            continue;
        }
        return Scan::new(kind, found + 1);
    }
}

/// A prefixed string or identifier at `pos` (`E'…'`, `B'…'`, `X'…'`, `N'…'`, `U&'…'`,
/// `U&"…"`), when the profile has that prefix.
pub(crate) fn prefixed(c: &mut Cursor, pos: usize) -> Option<Scan> {
    let prefixes = c.p.quotes.prefixes;
    let lead = c.b[pos].to_ascii_lowercase();
    let next = c.at(pos + 1)?;
    match (lead, next) {
        (b'e', b'\'') if prefixes.escape => Some(string_from(
            c,
            pos + 2,
            b'\'',
            StringRule::Escape,
            TokenKind::String,
        )),
        (b'b', b'\'') if prefixes.bit => Some(string_from(
            c,
            pos + 2,
            b'\'',
            StringRule::Raw,
            TokenKind::BitString,
        )),
        (b'x', b'\'') if prefixes.hex => {
            let scan = string_from(c, pos + 2, b'\'', StringRule::Raw, TokenKind::HexString);
            Some(check_blob(c, pos, scan))
        }
        (b'n', b'\'') if prefixes.national => {
            let rule = plain_rule(c);
            Some(string_from(c, pos + 2, b'\'', rule, TokenKind::String))
        }
        (b'u', b'&') if prefixes.unicode => match c.at(pos + 2)? {
            b'\'' => {
                let scan = string_from(c, pos + 3, b'\'', StringRule::Unicode, TokenKind::String);
                Some(if c.modes.standard_conforming_strings {
                    scan
                } else {
                    // "unsafe use of string constant with Unicode escapes"
                    let code = scan.error.unwrap_or(LexErrorCode::InvalidEscape);
                    Scan {
                        error: Some(code),
                        ..scan.flagged(TokenFlags::ERROR)
                    }
                })
            }
            b'"' => Some(ident_from(c, pos + 3, b'"', TokenKind::UnicodeQuotedIdent)),
            _ => None,
        },
        _ => None,
    }
}

/// SQLite's blob rule: `X'…'` holds an even number of hex digits, else the token is illegal.
fn check_blob(c: &mut Cursor, pos: usize, scan: Scan) -> Scan {
    if !c.p.quotes.hex_blob_checked || scan.open.is_some() {
        return scan;
    }
    let body = &c.b[pos + 2..scan.end - 1];
    c.steps += body.len() as u64;
    if body.len().is_multiple_of(2) && body.iter().all(u8::is_ascii_hexdigit) {
        scan
    } else {
        scan.flagged(TokenFlags::ERROR)
    }
}
