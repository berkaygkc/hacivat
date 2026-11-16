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
    // ## m3b-t20-differential: the body moved to `tokens_of`, shared with `analyse`.
    guarded(sql, |_, scanned| tokens_of(scanned))
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

// ## m3b-t20-differential (named exception in Task 9's file; T14 perf carry)

/// Everything [`parse_tree`], [`scan_tokens`] and [`fingerprint`] answer for one text, from one
/// trip to the parser thread: one scan (the guard's), one raw parse for both the tree and the
/// fingerprint (`third_party/pg_query` PATCHES §3), and no `pg_query` summary walk.
#[derive(Debug)]
pub struct Analysed {
    /// [`parse_tree`]'s answer.
    pub tree: Result<pb::ParseResult, PgError>,
    /// [`scan_tokens`]'s answer.
    pub tokens: Result<Vec<(TextRange, ScanKind)>, PgError>,
    /// [`fingerprint`]'s value; `None` exactly where [`fingerprint`] is an `Err` (a syntax
    /// error), and also when the tree's encoding failed.
    pub fingerprint: Option<u64>,
}

/// [`parse_tree`], [`scan_tokens`] and [`fingerprint`] of `sql` at once ([`Analysed`]). `Err`
/// only for the guard's refusals (a NUL byte, the 8 MiB cap, brackets nested too deep, the
/// parser thread gone), where each of the three is that same `Err`.
pub fn analyse(sql: &str) -> Result<Analysed, PgError> {
    guarded(sql, |text, scanned| {
        let (tree, fingerprint) = match pg_query::parse_protobuf_with_fingerprint(text, 0, 0) {
            Ok(both) => (
                both.protobuf.map_err(|e| error::from_pg(text, e)),
                both.fingerprint,
            ),
            Err(e) => (Err(error::from_pg(text, e)), None),
        };
        Ok(Analysed {
            tree,
            tokens: tokens_of(scanned),
            fingerprint,
        })
    })
}

/// [`scan_tokens`]'s answer from a scan.
fn tokens_of(scanned: &crate::split::Scanned) -> Result<Vec<(TextRange, ScanKind)>, PgError> {
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

    // ## m3b-t20-differential
    #[test]
    fn analyse_answers_what_the_three_calls_answer() {
        let deep = format!("SELECT {}1{}", "(".repeat(300), ")".repeat(300));
        let decode_limit = format!("SELECT {}1{}", "ARRAY[".repeat(60), "]".repeat(60));
        let texts = [
            "SELECT a FROM t WHERE id = 1",
            "select a\n from t where id = 42 -- x",
            "SELECT 1; INSERT INTO t VALUES (2) RETURNING *; /* c */",
            "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
            "SELECT E'a\\'b', U&'d\\0061t', $q$ x; $q$",
            "SELEC 1",
            "SELECT 'unterminated",
            "SELECT :a",
            "",
            "   ",
            "-- only a comment",
            "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END",
            "SELECT * FROM t WHERE a IN (3, 1, 2) AND b = 'x'",
            "SELECT 1\0",
            deep.as_str(),
            decode_limit.as_str(),
        ];
        for sql in texts {
            let tree = parse_tree(sql);
            let tokens = scan_tokens(sql);
            let fp = fingerprint(sql);
            match analyse(sql) {
                Ok(a) => {
                    assert_eq!(a.tree, tree, "{sql:?}");
                    assert_eq!(a.tokens, tokens, "{sql:?}");
                    assert_eq!(a.fingerprint, fp.ok(), "{sql:?}");
                }
                Err(e) => {
                    assert_eq!(Err(e.clone()), tree, "{sql:?}");
                    assert_eq!(Err(e.clone()), tokens, "{sql:?}");
                    assert_eq!(Err(e), fp, "{sql:?}");
                }
            }
        }
    }

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
