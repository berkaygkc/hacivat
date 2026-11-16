//! The parse tree, scanner tokens and fingerprint of a PostgreSQL text, for `sql-safety`'s PG
//! backend (Task 9). Every call runs on the oracle's one 16 MiB parser thread behind the same
//! guard as the splitter (NUL refused, 8 MiB cap, bracket depth ≤ 256 from PostgreSQL's own
//! scanner), so no input reaches libpg_query unchecked; a prost decode-depth failure is an
//! `Err`, never a partial tree.
//!
//! [`pb`] is the only re-export of `pg_query` types (`BackendOutsideOwner`): `sql-safety`
//! walks the tree through it, and nothing outside `sql-safety` uses it.
//!
//! - The text is parsed exactly as given: no placeholder normalisation (the classifier sees
//!   the text Run sends, `$n` already rewritten, P-M3b-2), so `SELECT :a` is an `Err` here.
//! - The decode of a full parse stops at prost's nesting limit of 100 messages (47 nested
//!   `ARRAY[`, 100 `UNION`s): that is an `Err`, and the classifier answers `Unknown`.
//! - `pg_query::parse` also builds its own summary (tables, functions) by walking the tree with
//!   `pg_query`'s walker; this crate drops it unread, and nothing in Hacivat classifies with it
//!   (R9: the classifier walks the generated visitor only).
//! - Whitespace-only texts never reach `pg_query::scan_tokens` (`split::scan`).
//!
//! Every function here blocks its caller until the parser thread has run the call (and every
//! call queued before it): never call one on the UI thread.

use sql_text::TextRange;

use crate::error::{self, PgError, size};
use crate::split::{Tok, guarded, kind};

pub use pg_query::protobuf as pb;

/// What a scanner token is, coarsely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScanKind {
    Keyword,
    Ident,
    Literal,
    Operator,
    Param,
    Comment,
    Other,
}

/// libpg_query's parse tree of `sql` (`pg_query_parse_protobuf_opts`, default options:
/// `standard_conforming_strings` on, the server's default). A syntax error, the guard's
/// refusals and a protobuf decode failure are an `Err`.
pub fn parse_tree(sql: &str) -> Result<pb::ParseResult, PgError> {
    guarded(sql, |text, _| {
        pg_query::parse(text, 0)
            .map(|parsed| parsed.protobuf)
            .map_err(|e| error::from_pg(text, e))
    })
}

/// The prefix of a protobuf decode failure's message (`pg_query::Error::Decode`): the tree
/// nests deeper than prost's fixed limit of 100 messages, which no API raises.
const DECODE_FAILURE: &str = "Error decoding result:";

/// Whether `e` is a limit rather than a syntax error: the text is over the 8 MiB cap, its
/// brackets nest deeper than [`crate::MAX_ORACLE_NESTING`], or its tree nests deeper than
/// prost decodes (100 messages: 47 nested `ARRAY[`, 100 `UNION`s, a `||` chain of about 45
/// terms, about 24 nested `CASE`). The classifier reports these as `TooComplex`.
pub fn over_a_limit(e: &PgError) -> bool {
    e.message == crate::split::TOO_LONG
        || e.message == crate::split::TOO_DEEP
        || e.message.starts_with(DECODE_FAILURE)
}

/// libpg_query's scanner tokens of `sql`, comments included, in text order (byte ranges into
/// `sql`). A lexical error is an `Err` (the tokens before it are not returned).
pub fn scan_tokens(sql: &str) -> Result<Vec<(TextRange, ScanKind)>, PgError> {
    guarded(sql, |_, scanned| {
        if let Some(message) = &scanned.lex_error {
            return Err(PgError::own(message, None));
        }
        if !scanned.covers_parse {
            return Err(PgError::own("the scanner's tokens are out of range", None));
        }
        scanned
            .toks
            .iter()
            .map(|t| {
                let range = TextRange::new(size(t.start)?, size(t.end)?);
                Some((range, scan_kind(t)))
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| PgError::own("a token offset is out of range", None))
    })
}

/// libpg_query's fingerprint of `sql` (`pg_query_fingerprint_opts`, default options): equal
/// for texts that differ only in constants, whitespace and comments. A syntax error is an
/// `Err`.
pub fn fingerprint(sql: &str) -> Result<u64, PgError> {
    guarded(sql, |text, _| {
        pg_query::fingerprint(text, 0, 0)
            .map(|f| f.value)
            .map_err(|e| error::from_pg(text, e))
    })
}

/// The coarse kind of a scanner token.
fn scan_kind(t: &Tok) -> ScanKind {
    use pb::Token as T;
    if t.is_comment() {
        return ScanKind::Comment;
    }
    if t.keyword {
        return ScanKind::Keyword;
    }
    if matches!(t.kind, kind::IDENT | kind::UIDENT) {
        return ScanKind::Ident;
    }
    match T::try_from(t.kind) {
        Ok(T::Sconst | T::Usconst | T::Fconst | T::Iconst | T::Bconst | T::Xconst) => {
            ScanKind::Literal
        }
        Ok(T::Param) => ScanKind::Param,
        Ok(
            T::Op
            | T::Typecast
            | T::DotDot
            | T::ColonEquals
            | T::EqualsGreater
            | T::LessEquals
            | T::GreaterEquals
            | T::NotEquals
            | T::Ascii37
            | T::Ascii42
            | T::Ascii43
            | T::Ascii45
            | T::Ascii47
            | T::Ascii60
            | T::Ascii61
            | T::Ascii62
            | T::Ascii94,
        ) => ScanKind::Operator,
        _ => ScanKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_kinds() {
        let toks = scan_tokens("SELECT a, 'x' + $1 /* c */ FROM t -- d").unwrap();
        let kinds: Vec<ScanKind> = toks.iter().map(|(_, k)| *k).collect();
        assert_eq!(
            kinds,
            [
                ScanKind::Keyword,
                ScanKind::Ident,
                ScanKind::Other,
                ScanKind::Literal,
                ScanKind::Operator,
                ScanKind::Param,
                ScanKind::Comment,
                ScanKind::Keyword,
                ScanKind::Ident,
                ScanKind::Comment,
            ]
        );
    }
}
