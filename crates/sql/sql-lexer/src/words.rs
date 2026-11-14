//! Words (`alg/10` LEX-02 rule 9): an identifier start, then identifier characters. `$`
//! continues a word where the dialect says so, so `x$$y` is one word and never opens a
//! dollar quote. Every non-ASCII character is an identifier character, as in the servers (a
//! narrower rule would split `x\u{a0}$$` and open a dollar quote PostgreSQL never sees). The
//! keyword of a word is looked up when the token is emitted (`keyword_lookup`, ASCII-only).

use crate::cursor::{Cursor, Scan};
use crate::strings;
use crate::token::TokenKind;

/// The word (or prefixed string, `E'…'`, `X'…'`, `U&"…"`, …) at `pos`.
pub(crate) fn scan(c: &mut Cursor, pos: usize) -> Scan {
    if let Some(prefixed) = strings::prefixed(c, pos) {
        return prefixed;
    }
    Scan::new(TokenKind::Word, word_end(c, pos))
}

/// The end of the word whose first byte is at `pos` (callers checked it starts a word). A
/// non-ASCII character's continuation bytes are identifier bytes too, so a word always ends on
/// a char boundary.
pub(crate) fn word_end(c: &mut Cursor, pos: usize) -> usize {
    let dollar = c.p.idents.dollar_continue;
    c.skip_while(pos + 1, |b| {
        b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 || (dollar && b == b'$')
    })
}
