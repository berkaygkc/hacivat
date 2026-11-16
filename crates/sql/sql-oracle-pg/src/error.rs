//! libpg_query errors as [`PgError`]: the English message and the byte offset it points at
//! (Task 9).
//!
//! The `pg_query` crate keeps libpg_query's message but drops its cursor (`cursorpos`, a
//! 1-based character number), and reading the C struct directly would take `unsafe`, which
//! the workspace denies outside `hacivat-ui-core/src/platform`. So `locate` finds the
//! position again, from the message and the input, and gives `None` whenever it cannot prove
//! the answer:
//!
//! - `… at end of input`: the end of the text (the scanner reports the end-of-input token).
//! - `unterminated … at or near "<rest>"`: the scanner prints the rest of the text from the
//!   opening quote or comment, so the position is the text's length minus that rest.
//! - `… at or near "<token>"` with that text once in the input: that occurrence, because the
//!   printed text is the input from the error position to the end of the failing token.
//! - Several occurrences: the candidates are the scanner's tokens with exactly that text. The
//!   failing token is the first candidate whose prefix (the text up to its end) fails to parse
//!   with the same message: every earlier candidate ends a viable prefix (it parses, or fails
//!   only "at end of input"), and every later prefix still contains the failing token with the
//!   lookahead it was read with. libpg_query's lookahead filter recodes `NOT`, `NULLS`, `WITH`,
//!   `WITHOUT` and `FORMAT` by the token after them, so for those the prefix also takes the
//!   next token; a failure there would name that token's text, so the argument holds unless
//!   two candidates are neighbours (`NOT NOT`), which gives `None`. The filter merges `U&`
//!   identifiers and strings with the next two tokens, so any `U&` token gives `None`.
//! - Errors raised by grammar actions (`LIMIT #,# syntax is not supported`, …) carry no token
//!   text and give `None`.
//!
//! A scratch C harness that reads `cursorpos` from the same static library compared both
//! sides over 100,000 generated mutations while this was written, with no wrong cursor (Task 9
//! report); `tests/oracle.rs` keeps representative cases.

use std::fmt;

use text_size::TextSize;

use crate::split::{self, Tok, kind};

/// A libpg_query error: the server's English message and, when known, the byte offset it
/// points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgError {
    /// The message as libpg_query reports it, untranslated.
    pub message: String,
    /// The byte offset into the input the error points at, when known.
    pub cursor: Option<TextSize>,
}

impl fmt::Display for PgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PgError {}

impl PgError {
    /// An error Hacivat raises itself (the guard, the parser thread), in English like
    /// libpg_query's.
    pub(crate) fn own(message: &str, cursor: Option<usize>) -> PgError {
        PgError {
            message: message.to_string(),
            cursor: cursor.and_then(size),
        }
    }
}

/// The offset as a [`TextSize`]; `None` past `u32::MAX` (the guard keeps inputs far below).
pub(crate) fn size(at: usize) -> Option<TextSize> {
    u32::try_from(at).ok().map(TextSize::from)
}

/// libpg_query's own message. Private on purpose: a `From` impl would put a `pg_query` type
/// into the public API.
pub(crate) fn c_message(e: pg_query::Error) -> String {
    match e {
        pg_query::Error::Parse(m) | pg_query::Error::Split(m) | pg_query::Error::Scan(m) => m,
        other => other.to_string(),
    }
}

/// The error of parsing `text`, with its cursor located in `text`.
pub(crate) fn from_pg(text: &str, e: pg_query::Error) -> PgError {
    let message = c_message(e);
    let cursor = locate(text, &message).and_then(size);
    PgError { message, cursor }
}

const NEAR: &str = " at or near \"";
const AT_END: &str = " at end of input";

/// Texts longer than this get no cursor when one needs the prefix parses (each up to the whole
/// text, `O(log n)` of them); the oracle's normal input is one statement region.
const DISAMBIGUATE_LIMIT: usize = 1 << 20;

/// The byte offset libpg_query's error `message` points at in `text`, or `None` when it
/// cannot be shown (module docs).
pub(crate) fn locate(text: &str, message: &str) -> Option<usize> {
    if message.ends_with(AT_END) {
        return Some(text.len());
    }
    let (head, near) = near_text(message)?;
    if let Some(found) = by_text(text, head, near) {
        return found;
    }
    if text.len() > DISAMBIGUATE_LIMIT {
        return None;
    }
    let scanned = split::scan(text);
    if !scanned.covers_parse || scanned.lex_error.as_deref() == Some(message) {
        // A lexical error with several occurrences of its text, or tokens that stop before
        // a lexical error the scanner could not place.
        return None;
    }
    if scanned
        .toks
        .iter()
        .any(|t| matches!(t.kind, kind::UIDENT | kind::USCONST))
    {
        return None;
    }
    let sig: Vec<&Tok> = scanned.toks.iter().filter(|t| !t.is_comment()).collect();
    let is_candidate = |t: &Tok| text.get(t.start..t.end) == Some(near);
    // Each candidate with the end of the prefix that decides it.
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for (i, t) in sig.iter().enumerate() {
        if !is_candidate(t) {
            continue;
        }
        let cut = if t.recoded_by_lookahead() {
            match sig.get(i + 1) {
                Some(next) if is_candidate(next) => return None,
                Some(next) => next.end,
                None => text.len(),
            }
        } else {
            t.end
        };
        candidates.push((t.start, cut));
    }
    match candidates.as_slice() {
        [] => None,
        [(only, _)] => Some(*only),
        many => {
            let fails_here = |&(_, cut): &(usize, usize)| {
                text.get(..cut)
                    .and_then(prefix_error)
                    .is_some_and(|m| m == message)
            };
            let first = many.partition_point(|c| !fails_here(c));
            // The search assumes the predicate flips once; check the flip it found.
            let flips = many.get(first).is_some_and(fails_here)
                && (first == 0 || !fails_here(&many[first - 1]));
            flips.then(|| many[first].0)
        }
    }
}

/// The position of a lexical error the scanner reported with `message`, when the text alone
/// proves it (the first three rules of the module docs).
pub(crate) fn locate_lexical(text: &str, message: &str) -> Option<usize> {
    if message.ends_with(AT_END) {
        return Some(text.len());
    }
    let (head, near) = near_text(message)?;
    by_text(text, head, near).flatten()
}

/// `(head, token text)` of `<head> at or near "<token text>"`. The head is one of the
/// scanner's fixed English phrases, so the first ` at or near "` is the separator.
fn near_text(message: &str) -> Option<(&str, &str)> {
    let at = message.find(NEAR)?;
    let near = message[at + NEAR.len()..].strip_suffix('"')?;
    Some((&message[..at], near))
}

/// `Some(answer)` when the text decides: an unterminated construct (its position follows from
/// the printed rest), no occurrence (`Some(None)`) or exactly one; `None` when the text occurs
/// more than once.
fn by_text(text: &str, head: &str, near: &str) -> Option<Option<usize>> {
    if near.is_empty() {
        return Some(None);
    }
    if head.starts_with("unterminated ") {
        return Some(text.ends_with(near).then(|| text.len() - near.len()));
    }
    let Some(first) = text.find(near) else {
        return Some(None);
    };
    // Occurrences may overlap ("tt" in "ttt"): look again one character further on.
    let step = text[first..].chars().next().map_or(1, char::len_utf8);
    match text[first + step..].find(near) {
        None => Some(Some(first)),
        Some(_) => None,
    }
}

/// The message of parsing `prefix` on its own, or `None` when it parses.
fn prefix_error(prefix: &str) -> Option<String> {
    pg_query::split_with_parser(prefix).err().map(c_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_text_takes_the_first_separator_and_one_closing_quote() {
        assert_eq!(
            near_text("zero-length delimited identifier at or near \"\"\"\""),
            Some(("zero-length delimited identifier", "\"\""))
        );
        assert_eq!(
            near_text(r#"syntax error at or near "' at or near "y'""#),
            Some(("syntax error", r#"' at or near "y'"#))
        );
        assert_eq!(near_text("LIMIT #,# syntax is not supported"), None);
    }

    #[test]
    fn by_text_counts_overlapping_occurrences() {
        assert_eq!(by_text("SELECT 1 ttt", "syntax error", "tt"), None);
        assert_eq!(by_text("SELECT 1 tt", "syntax error", "tt"), Some(Some(9)));
        assert_eq!(by_text("SELECT 1", "syntax error", "x"), Some(None));
        assert_eq!(
            by_text("SELECT 'a', 'a", "unterminated quoted string", "'a"),
            Some(Some(12))
        );
        assert_eq!(
            by_text("SELECT 'çğ", "unterminated quoted string", "'çğ"),
            Some(Some(7))
        );
    }
}
