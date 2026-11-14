//! Numbers (`alg/10` LEX-02 rule 7), by the profile's version gates.
//!
//! The token is the longest valid number: digits, a fraction, an exponent only when a digit
//! follows `e`/`E` (and its sign), radix prefixes where the dialect has them (PostgreSQL 16
//! `0x`/`0o`/`0b`, MySQL lower-case `0x`/`0b`, SQLite `0x`), `_` separators where it has them
//! (`HAS_SEPARATORS`). PostgreSQL reads `1..2` as `1`, `..`, `2`. After the number:
//! - PostgreSQL ≥ 15 and SQLite reject an identifier right after it: the number is flagged
//!   `JUNK` and the identifier is the next token (PostgreSQL 15 `1_000` is `1` then `_000`;
//!   PostgreSQL ≤ 14 reads `123abc` as `123 AS abc`, unflagged);
//! - MySQL and MariaDB read a digit-leading run of identifier characters as a word (`1e5x`,
//!   `123abc`, `0X12`), unless it is all digits.

use sql_dialect::lex_profiles::{Junk, Radix, Separators};

use crate::cursor::{Cursor, Scan};
use crate::token::{TokenFlags, TokenKind};
use crate::words;

/// The number (or MySQL digit-leading word) at `pos`, a digit or a `.` before a digit.
pub(crate) fn scan(c: &mut Cursor, pos: usize) -> Scan {
    let (end, flags) = number_end(c, pos);
    if c.p.numbers.digit_leading_idents
        && c.b[pos].is_ascii_digit()
        && c.b.get(end).is_some_and(|b| c.ident_continue(*b))
    {
        let word = words::word_end(c, pos);
        if word > end && !c.b[pos..word].iter().all(u8::is_ascii_digit) {
            return Scan::new(TokenKind::Word, word);
        }
    }
    Scan::new(TokenKind::Number, end).flagged(flags | junk_after(c, end))
}

/// `JUNK` when what follows a number (or a `$n` parameter) ending at `end` is an error here.
pub(crate) fn junk_after(c: &mut Cursor, end: usize) -> TokenFlags {
    let junk = match (c.p.numbers.junk, c.at(end)) {
        (Junk::IdentStart, Some(b)) => c.ident_start(b),
        (Junk::IdentChar, Some(b)) => c.ident_continue(b),
        _ => false,
    };
    if junk {
        TokenFlags::JUNK
    } else {
        TokenFlags::empty()
    }
}

/// The end of the longest valid number at `pos`, and its flags.
fn number_end(c: &mut Cursor, pos: usize) -> (usize, TokenFlags) {
    let mut flags = TokenFlags::empty();
    if c.b[pos] == b'0'
        && let Some(end) = radix_end(c, pos, &mut flags)
    {
        return (end, flags);
    }
    let mut i = pos;
    if c.b[i].is_ascii_digit() {
        i = digits(c, i, &mut flags);
    }
    if c.at(i) == Some(b'.') {
        if c.p.numbers.dot_dot && i > pos && c.at(i + 1) == Some(b'.') {
            return (i, flags);
        }
        i += 1;
        let loose = c.p.numbers.separators == Separators::Loose;
        if c.at(i)
            .is_some_and(|b| b.is_ascii_digit() || (loose && b == b'_'))
        {
            i = digits(c, i, &mut flags);
        }
    }
    if matches!(c.at(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(c.at(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if c.at(j).is_some_and(|b| b.is_ascii_digit()) {
            i = digits(c, j, &mut flags);
        }
    }
    (i, flags)
}

/// The end of the digit run at `i`, with the dialect's separators.
fn digits(c: &mut Cursor, i: usize, flags: &mut TokenFlags) -> usize {
    match c.p.numbers.separators {
        Separators::None => c.skip_while(i, |b| b.is_ascii_digit()),
        Separators::BetweenDigits => {
            let mut j = c.skip_while(i, |b| b.is_ascii_digit());
            while c.at(j) == Some(b'_') && c.at(j + 1).is_some_and(|b| b.is_ascii_digit()) {
                *flags |= TokenFlags::HAS_SEPARATORS;
                j = c.skip_while(j + 1, |b| b.is_ascii_digit());
            }
            j
        }
        Separators::Loose => {
            let j = c.skip_while(i, |b| b.is_ascii_digit() || b == b'_');
            if c.b[i..j].contains(&b'_') {
                *flags |= TokenFlags::HAS_SEPARATORS;
            }
            j
        }
    }
}

/// The end of a radix-prefixed integer at `pos` (a `0`), if one is there.
fn radix_end(c: &mut Cursor, pos: usize, flags: &mut TokenFlags) -> Option<usize> {
    let x = c.at(pos + 1)?;
    match c.p.numbers.radix {
        Radix::Decimal => None,
        Radix::Postgres => {
            let digit: fn(u8) -> bool = match x.to_ascii_lowercase() {
                b'x' => |b| b.is_ascii_hexdigit(),
                b'o' => |b| matches!(b, b'0'..=b'7'),
                b'b' => |b| matches!(b, b'0' | b'1'),
                _ => return None,
            };
            // `0x(_?hexdigit)+`
            let mut j = pos + 2;
            loop {
                let under = c.at(j) == Some(b'_');
                let k = j + usize::from(under);
                if !c.at(k).is_some_and(digit) {
                    break;
                }
                if under {
                    *flags |= TokenFlags::HAS_SEPARATORS;
                }
                j = c.skip_while(k + 1, digit);
            }
            (j > pos + 2).then_some(j)
        }
        Radix::LowerHexBin => {
            let digit: fn(u8) -> bool = match x {
                b'x' => |b| b.is_ascii_hexdigit(),
                b'b' => |b| matches!(b, b'0' | b'1'),
                _ => return None,
            };
            let end = c.skip_while(pos + 2, digit);
            (end > pos + 2).then_some(end)
        }
        Radix::Hex => {
            if !matches!(x, b'x' | b'X') || !c.at(pos + 2).is_some_and(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let end = c.skip_while(pos + 3, |b| b.is_ascii_hexdigit() || b == b'_');
            if c.b[pos + 3..end].contains(&b'_') {
                *flags |= TokenFlags::HAS_SEPARATORS;
            }
            Some(end)
        }
    }
}
