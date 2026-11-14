//! Statement ranges from PostgreSQL's own grammar (Task 9): `hacivat-exec` resolves an
//! ambiguous PostgreSQL boundary with it (R-M3-24), on the parser stack (R-M3-35).
//!
//! Every call into libpg_query goes through `guarded`. It refuses a NUL character and an
//! oversized text before anything runs, then moves to the parser thread
//! ([`on_parser_stack`]), scans the text with PostgreSQL's own scanner, refuses brackets
//! nested deeper than [`MAX_ORACLE_NESTING`], and only then calls the parser. The scanner is
//! flex-generated and iterative, so no input can exhaust its stack.
//!
//! Every public function here blocks its caller until the parser thread has run the call, and
//! every call queued before it ([`on_parser_stack`]): never call one on the UI thread.

use std::borrow::Cow;
use std::sync::OnceLock;

use pg_query::protobuf::ScanToken;
use sql_text::TextRange;

use crate::error::{self, PgError, size};
use crate::stack::{MAX_ORACLE_NESTING, on_parser_stack};

/// Scanner token codes this crate reads (`pg_query::protobuf::Token`).
pub(crate) mod kind {
    use pg_query::protobuf::Token;

    pub(crate) const LPAREN: i32 = Token::Ascii40 as i32;
    pub(crate) const RPAREN: i32 = Token::Ascii41 as i32;
    pub(crate) const LBRACKET: i32 = Token::Ascii91 as i32;
    pub(crate) const RBRACKET: i32 = Token::Ascii93 as i32;
    pub(crate) const COLON: i32 = Token::Ascii58 as i32;
    pub(crate) const SEMICOLON: i32 = Token::Ascii59 as i32;
    pub(crate) const IDENT: i32 = Token::Ident as i32;
    pub(crate) const UIDENT: i32 = Token::Uident as i32;
    pub(crate) const SCONST: i32 = Token::Sconst as i32;
    pub(crate) const USCONST: i32 = Token::Usconst as i32;
    pub(crate) const SQL_COMMENT: i32 = Token::SqlComment as i32;
    pub(crate) const C_COMMENT: i32 = Token::CComment as i32;
    /// The keywords libpg_query's lookahead filter recodes by the token after them
    /// (`base_yylex`: `NOT_LA`, `NULLS_LA`, `WITH_LA`, `WITHOUT_LA`, `FORMAT_LA`).
    pub(crate) const RECODED: [i32; 5] = [
        Token::Not as i32,
        Token::NullsP as i32,
        Token::With as i32,
        Token::Without as i32,
        Token::Format as i32,
    ];
}

/// One scanner token: byte offsets into the scanned text and its code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tok {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: i32,
    pub(crate) keyword: bool,
}

impl Tok {
    pub(crate) fn is_comment(&self) -> bool {
        matches!(self.kind, kind::SQL_COMMENT | kind::C_COMMENT)
    }

    /// Whether the parser may see this token under another code than the scanner reports.
    pub(crate) fn recoded_by_lookahead(&self) -> bool {
        kind::RECODED.contains(&self.kind)
    }
}

/// What PostgreSQL's scanner makes of a text.
pub(crate) struct Scanned {
    /// The tokens in order, comments included; with a lexical error, only those before it.
    pub(crate) toks: Vec<Tok>,
    /// The scanner's message for the first lexical error.
    pub(crate) lex_error: Option<String>,
    /// Whether `toks` holds every token the parser can read before it stops. False only when
    /// a lexical error could not be placed, so the tokens before it are unknown.
    pub(crate) covers_parse: bool,
}

/// Scans `text`. On a lexical error the tokens before it come from scanning the text up to
/// the error's proven position (`error::locate_lexical`): nothing before that position is a
/// lexical error, and the scanner never reads past it.
pub(crate) fn scan(text: &str) -> Scanned {
    match scan_tokens(text) {
        Ok(toks) => Scanned {
            covers_parse: toks.is_some(),
            toks: toks.unwrap_or_default(),
            lex_error: None,
        },
        Err(message) => {
            let before = error::locate_lexical(text, &message)
                .and_then(|at| text.get(..at))
                .and_then(|prefix| scan_tokens(prefix).ok().flatten());
            Scanned {
                covers_parse: before.is_some(),
                toks: before.unwrap_or_default(),
                lex_error: Some(message),
            }
        }
    }
}

/// The scanner's tokens, `Ok(None)` if an offset is out of range, or the lexical error's
/// message.
fn scan_tokens(text: &str) -> Result<Option<Vec<Tok>>, String> {
    if text.bytes().all(|b| b.is_ascii_whitespace() || b == 0x0b) {
        // No tokens. `pg_query::scan_tokens` must not see such a text: with no tokens the C
        // side returns a null array, which the crate turns into a slice (a debug-build panic,
        // undefined behaviour in release). Any other byte starts a token, a comment (which
        // the scan returns as a token) or a lexical error.
        return Ok(Some(Vec::new()));
    }
    pg_query::scan_tokens(text)
        .map(|tokens| toks_of(&tokens))
        .map_err(error::c_message)
}

/// The scanner's tokens with `usize` offsets; `None` if any offset is out of range (which
/// would make every count over them unreliable).
fn toks_of(tokens: &[ScanToken]) -> Option<Vec<Tok>> {
    tokens
        .iter()
        .map(|t| {
            let start = usize::try_from(t.start).ok()?;
            let end = usize::try_from(t.end).ok()?;
            (start <= end).then_some(Tok {
                start,
                end,
                kind: t.token,
                keyword: t.keyword_kind != 0,
            })
        })
        .collect()
}

/// The crate version Hacivat pins, kept equal to the `pg_query = "=18.1.0"` workspace line.
const PG_QUERY_VERSION: &str = "18.1.0";

/// The longest text sent to the parser; a longer one is refused with an `Err` before any scan,
/// so the caller keeps its boundary uncertain (fail closed).
///
/// The oracle's input is one ambiguous statement region, not a document (Hacivat's editor holds
/// up to 20 MB, R-M3-49). Its peak memory is about 135 bytes per token (libpg_query's token
/// array and its copy, this crate's two token lists, then the raw parse tree): measured, an
/// 8 MiB flat `SELECT 1,1,…` peaked at +1.1 GiB, so the cap bounds one call near that, and a
/// call's time on the one parser thread to seconds.
const MAX_TEXT_BYTES: usize = 8 << 20;

const NUL_MESSAGE: &str = "the statement text contains a NUL character";
const TOO_LONG: &str = "the statement text is too long for the parser";
const TOO_DEEP: &str = "brackets nest too deeply for the parser";
const THREAD_FAILED: &str = "the parser thread failed";
const MISMATCH: &str = "the parser's statement ranges do not match its tokens";

/// `"pg_query 18.1.0 (PostgreSQL 18.6)"`. The PostgreSQL version is the one the linked C
/// library reports (`PG_VERSION_NUM` in its parse result), read once, on the parser thread.
///
/// It comes from a full parse of `SELECT 1` through the shipped, pre-generated protobuf code
/// (R-M3-1, P1), so a decode that does not match the C library shows as `unknown`. Only a
/// successful read is kept: after a failure (the parser thread could not start, say) the next
/// call reads again, so one transient failure never sticks.
pub fn version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    cached_version(&VERSION, read_server_version)
}

/// [`version`]'s text when the server version cannot be read.
const VERSION_UNKNOWN: &str = "pg_query 18.1.0 (PostgreSQL unknown)";

/// `cell`'s version, read with `read` until one read succeeds.
fn cached_version(
    cell: &'static OnceLock<String>,
    read: impl FnOnce() -> Option<i32>,
) -> &'static str {
    if let Some(known) = cell.get() {
        return known;
    }
    match read() {
        Some(num) => cell.get_or_init(|| {
            format!(
                "pg_query {PG_QUERY_VERSION} (PostgreSQL {})",
                pg_version_text(num)
            )
        }),
        None => VERSION_UNKNOWN,
    }
}

/// `PG_VERSION_NUM` from a full parse of `SELECT 1`, if it decodes as one `SelectStmt`.
fn read_server_version() -> Option<i32> {
    guarded("SELECT 1", |text, _| {
        let parsed = pg_query::parse(text, 0).map_err(|e| error::from_pg(text, e))?;
        let stmts = &parsed.protobuf.stmts;
        let select = stmts.len() == 1
            && stmts[0]
                .stmt
                .as_ref()
                .is_some_and(|n| matches!(n.node, Some(pg_query::NodeEnum::SelectStmt(_))));
        if select {
            Ok(parsed.protobuf.version)
        } else {
            Err(PgError::own(MISMATCH, None))
        }
    })
    .ok()
}

/// `180006` → `"18.6"` (the PostgreSQL 10+ numbering).
fn pg_version_text(num: i32) -> String {
    format!("{}.{}", num / 10_000, num % 10_000)
}

/// How many statements PostgreSQL's own grammar finds in `sql`; a syntax error is an `Err`,
/// never a panic. Runs on the parser thread after the nesting check, like every parser call.
///
/// It counts the raw parser's statement list and builds no output tree: the protobuf decode
/// of a full parse stops at 100 nested messages (prost's limit), which ordinary SQL reaches
/// (`UNION` of 100 `SELECT`s, 50 nested `ARRAY[…]`).
///
/// Unlike [`split_with_parser`] it does not normalise placeholders: `SELECT :a` is an `Err`
/// here and one statement there. Only the build proof and `--smoke` use it.
pub fn statement_count(sql: &str) -> Result<usize, PgError> {
    guarded(sql, |text, _| {
        pg_query::split_with_parser(text)
            .map(|stmts| stmts.len())
            .map_err(|e| error::from_pg(text, e))
    })
}

/// The statement ranges PostgreSQL's own grammar finds in `sql`, in order.
///
/// - Each range runs from the statement's first token to the end of its last token: leading
///   and trailing whitespace and comments, and the `;`, are outside it. A `;` inside a
///   statement (a `BEGIN ATOMIC` body, a rule's action list) stays inside.
/// - Empty statements (`;;`) and a text of only comments give no range.
/// - The text is parsed as written first. Only when that fails are named placeholders
///   normalised and the text parsed again: a `:name`, psql's `:'name'` and `:"name"` become
///   `$1` padded with spaces to the same length, so every offset stays the caller's. Only a
///   `:` token directly followed by the name token counts, and not after a word character or
///   `$` (`x$:y` stays as written), so the replacement never joins a neighbouring token; `::`
///   casts and `:=` are other tokens. Array slices (`a[1 :n]`, `a[(1):n]`, `a[:n]`) parse as
///   written, so they never reach the normalisation; a text with both a slice like those and
///   a placeholder is an `Err`.
/// - The ranges are checked against the scanner's tokens: every token outside them must be a
///   `;`, and consecutive statements must be separated by one. Anything else is an `Err`
///   (fail closed: the caller then keeps the boundary uncertain).
/// - A syntax error is an `Err` whose message names the text the parser saw last (a
///   normalised placeholder reads `$1`) and whose cursor is a byte offset into `sql`.
/// - A text over 8 MiB is an `Err` before any scan (the oracle's memory cap).
pub fn split_with_parser(sql: &str) -> Result<Vec<TextRange>, PgError> {
    guarded(sql, |text, scanned| {
        let first = match pg_query::split_with_parser(text) {
            Ok(stmts) => return checked_ranges(text, &stmts, scanned),
            Err(e) => e,
        };
        let normalised = normalise_placeholders(text, &scanned.toks);
        if matches!(normalised, Cow::Borrowed(_)) {
            // No placeholder: the first parse's error is the answer.
            return Err(error::from_pg(text, first));
        }
        let stmts =
            pg_query::split_with_parser(&normalised).map_err(|e| error::from_pg(&normalised, e))?;
        checked_ranges(&normalised, &stmts, scanned)
    })
}

/// libpg_query's statements of `text` as checked ranges (see [`split_with_parser`]).
fn checked_ranges(
    text: &str,
    stmts: &[&str],
    scanned: &Scanned,
) -> Result<Vec<TextRange>, PgError> {
    if !scanned.covers_parse || scanned.lex_error.is_some() {
        // The parser accepted a text its own scanner rejects: trust neither.
        return Err(PgError::own(MISMATCH, None));
    }
    statement_ranges(text, stmts, &scanned.toks).ok_or_else(|| PgError::own(MISMATCH, None))
}

/// Runs `run` on the parser thread, after the input checks and the nesting check (module
/// docs). `run` gets the text and its scan.
fn guarded<T, F>(sql: &str, run: F) -> Result<T, PgError>
where
    T: Send + 'static,
    F: FnOnce(&str, &Scanned) -> Result<T, PgError> + Send + 'static,
{
    if let Some(at) = sql.find('\0') {
        // libpg_query takes a C string: a NUL would silently cut the text.
        return Err(PgError::own(NUL_MESSAGE, Some(at)));
    }
    if sql.len() > MAX_TEXT_BYTES {
        return Err(PgError::own(TOO_LONG, None));
    }
    let text = sql.to_owned();
    on_parser_stack(move || {
        let scanned = scan(&text);
        check_nesting(&text, &scanned)?;
        run(&text, &scanned)
    })
    .unwrap_or_else(|| Err(PgError::own(THREAD_FAILED, None)))
}

/// Refuses brackets (`(` and `[` together) nested deeper than [`MAX_ORACLE_NESTING`]
/// (R-M3-3, R-M3-35), with the cursor on the first bracket past the limit. Brackets inside
/// strings, quoted identifiers and comments are not tokens, so they do not count. When a
/// lexical error hides the tokens, the bound is the number of bracket bytes in the whole
/// text, an upper bound of any nesting in it.
fn check_nesting(text: &str, scanned: &Scanned) -> Result<(), PgError> {
    let limit = usize::from(MAX_ORACLE_NESTING);
    if !scanned.covers_parse {
        let opens = text.bytes().filter(|b| matches!(b, b'(' | b'[')).count();
        return if opens <= limit {
            Ok(())
        } else {
            Err(PgError::own(
                scanned.lex_error.as_deref().unwrap_or(TOO_DEEP),
                None,
            ))
        };
    }
    let mut depth = 0usize;
    for t in &scanned.toks {
        match t.kind {
            kind::LPAREN | kind::LBRACKET => {
                depth += 1;
                if depth > limit {
                    return Err(PgError::own(TOO_DEEP, Some(t.start)));
                }
            }
            kind::RPAREN | kind::RBRACKET => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// `text` with every named placeholder replaced by `$1` and spaces (see
/// [`split_with_parser`]); borrowed when there is none.
fn normalise_placeholders<'a>(text: &'a str, toks: &[Tok]) -> Cow<'a, str> {
    let bytes = text.as_bytes();
    let mut out: Option<Vec<u8>> = None;
    for pair in toks.windows(2) {
        let (colon, name) = (pair[0], pair[1]);
        if colon.kind != kind::COLON || name.start != colon.end || !placeholder_name(text, &name) {
            continue;
        }
        if colon.start > 0 && word_byte(bytes[colon.start - 1]) {
            continue;
        }
        let buf = out.get_or_insert_with(|| bytes.to_vec());
        buf[colon.start] = b'$';
        buf[colon.start + 1] = b'1';
        buf[colon.start + 2..name.end].fill(b' ');
    }
    match out {
        None => Cow::Borrowed(text),
        // Whole tokens became ASCII, so the bytes stay UTF-8; if not, nothing is normalised.
        Some(buf) => String::from_utf8(buf).map_or(Cow::Borrowed(text), Cow::Owned),
    }
}

/// A byte that would join a following `$1` into a longer token (an identifier, a number, a
/// dollar-quote tag).
fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Whether `t`, right after a `:`, is a placeholder name: an identifier or keyword, or
/// psql's `'name'` and `"name"` (name characters as Hacivat's lexer reads them).
fn placeholder_name(text: &str, t: &Tok) -> bool {
    let Some(s) = text.get(t.start..t.end) else {
        return false;
    };
    let name = |inner: &str| {
        !inner.is_empty()
            && inner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80)
    };
    match t.kind {
        kind::IDENT => match s.strip_prefix('"') {
            Some(rest) => rest.strip_suffix('"').is_some_and(name),
            None => true,
        },
        kind::SCONST => s
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
            .is_some_and(name),
        _ => t.keyword,
    }
}

/// libpg_query's statements (slices of `text`) as trimmed ranges, checked against the
/// scanner's tokens (see [`split_with_parser`]); `None` when they disagree.
fn statement_ranges(text: &str, stmts: &[&str], toks: &[Tok]) -> Option<Vec<TextRange>> {
    let base = text.as_ptr() as usize;
    let sig: Vec<&Tok> = toks.iter().filter(|t| !t.is_comment()).collect();
    let mut out = Vec::with_capacity(stmts.len());
    let mut next = 0;
    let mut separated = true;
    for stmt in stmts {
        let start = (stmt.as_ptr() as usize).checked_sub(base)?;
        let end = start.checked_add(stmt.len())?;
        if end > text.len() {
            return None;
        }
        while let Some(t) = sig.get(next).filter(|t| t.end <= start) {
            if t.kind != kind::SEMICOLON {
                return None;
            }
            separated = true;
            next += 1;
        }
        let first = **sig.get(next)?;
        if !separated || first.start < start || first.start >= end {
            return None;
        }
        let mut last = first;
        while let Some(t) = sig.get(next).filter(|t| t.start < end) {
            if t.end > end {
                return None;
            }
            last = **t;
            next += 1;
        }
        if first.kind == kind::SEMICOLON || last.kind == kind::SEMICOLON {
            // A terminator inside the range: not the split this function promises.
            return None;
        }
        out.push(TextRange::new(size(first.start)?, size(last.end)?));
        separated = false;
    }
    sig[next..]
        .iter()
        .all(|t| t.kind == kind::SEMICOLON)
        .then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalised(sql: &str) -> String {
        normalise_placeholders(sql, &scan(sql).toks).into_owned()
    }

    #[test]
    fn placeholders_become_dollar_one_with_the_same_length() {
        assert_eq!(normalised("SELECT :a, :abc"), "SELECT $1, $1  ");
        assert_eq!(normalised("SELECT :'v', :\"w\""), "SELECT $1  , $1  ");
        assert_eq!(normalised("SELECT :select"), "SELECT $1     ");
        assert_eq!(normalised("SELECT :müş"), "SELECT $1    ");
    }

    #[test]
    fn non_placeholders_stay_as_written() {
        for sql in [
            "SELECT x::int, a[1:n], a[lo:hi], x$:y",
            "SELECT ':a', \":a\", $$ :a $$ -- :a",
            "SELECT : a, :'a b', :E'x', :'', :x'1F'",
            "SELECT 1 /* :a */",
        ] {
            assert_eq!(normalised(sql), sql);
        }
    }

    #[test]
    fn a_word_or_dollar_before_the_colon_blocks_the_replacement() {
        // `$t:x` → `$t$1` would open a dollar quote; `a:b` → `a$1` would extend the word.
        assert_eq!(normalised("SELECT $t:x; SELECT 1"), "SELECT $t:x; SELECT 1");
        assert_eq!(normalised("SELECT a:b, (:c)"), "SELECT a:b, ($1)");
    }

    #[test]
    fn a_failed_version_read_is_not_cached() {
        static CELL: OnceLock<String> = OnceLock::new();
        assert_eq!(cached_version(&CELL, || None), VERSION_UNKNOWN);
        assert_eq!(CELL.get(), None);
        let known = "pg_query 18.1.0 (PostgreSQL 18.6)";
        assert_eq!(cached_version(&CELL, || Some(180_006)), known);
        // Once read, the version is never read again.
        assert_eq!(
            cached_version(&CELL, || panic!("read after success")),
            known
        );
        assert!(VERSION_UNKNOWN.starts_with(&format!("pg_query {PG_QUERY_VERSION} ")));
    }
}
