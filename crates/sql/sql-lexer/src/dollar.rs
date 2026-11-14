//! PostgreSQL's `$` (`alg/10` LEX-02 rule 6): dollar-quoted strings and `$n` parameters.
//!
//! A dollar quote opens with `$tag$`, the tag empty or an identifier without `$` that does not
//! start with a digit (bytes ≥ 0x80 included, as in the server), and closes at the first
//! identical `$tag$`: tags are case-sensitive and a different inner tag is text. `$1` is a
//! parameter, never a tag (`$1$` is a parameter and a lone `$`). A `$` inside a word belongs
//! to the word (`x$$y`, `words`). Anything else after `$` leaves a lone `$`, which the server
//! rejects.

use smallvec::SmallVec;

use crate::cursor::{Cursor, Scan, unknown};
use crate::numbers;
use crate::state::LexState;
use crate::token::{LexErrorCode, TokenKind};

/// The token at the `$` at `pos`.
pub(crate) fn scan(c: &mut Cursor, pos: usize) -> Scan {
    if c.p.quotes.dollar
        && let Some(quoted) = dollar_quote(c, pos)
    {
        return quoted;
    }
    if c.at(pos + 1).is_some_and(|d| d.is_ascii_digit()) {
        let end = c.skip_while(pos + 1, |d| d.is_ascii_digit());
        return numbers::junk_checked(c, Scan::new(TokenKind::PositionalParam, end));
    }
    unknown(pos)
}

/// The dollar-quoted string opened at `pos`, if a `$tag$` opens there.
fn dollar_quote(c: &mut Cursor, pos: usize) -> Option<Scan> {
    let bytes = c.b;
    let mut close = pos + 1;
    match c.at(close)? {
        b'$' => {}
        b if b.is_ascii_alphabetic() || b == b'_' || b >= 0x80 => {
            close = c.skip_while(close + 1, |b| {
                b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
            });
            if c.at(close)? != b'$' {
                return None;
            }
        }
        _ => return None,
    }
    let delim = &bytes[pos..=close];
    Some(match c.find_seq(close + 1, delim) {
        Some(found) => Scan::new(TokenKind::DollarString, found + delim.len()),
        None => unterminated(c, &delim[1..delim.len() - 1]),
    })
}

/// The rest of a dollar-quoted string opened by `$tag$` whose text continues at `from`.
pub(crate) fn body_from(c: &mut Cursor, from: usize, tag: &[u8]) -> Scan {
    let mut delim: SmallVec<[u8; 18]> = SmallVec::with_capacity(tag.len() + 2);
    delim.push(b'$');
    delim.extend_from_slice(tag);
    delim.push(b'$');
    match c.find_seq(from, &delim) {
        Some(found) => Scan::new(TokenKind::DollarString, found + delim.len()),
        None => unterminated(c, tag),
    }
}

fn unterminated(c: &Cursor, tag: &[u8]) -> Scan {
    Scan::unterminated(
        TokenKind::DollarString,
        c.len(),
        LexState::InDollar {
            tag: SmallVec::from_slice(tag),
        },
        Some(LexErrorCode::UnterminatedDollar),
    )
}
