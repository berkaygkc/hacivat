//! Calls a rollback cannot undo, refused before a read-only statement is sent (SQL-SAFE-05; Task
//! 23 finding F1, controller ruling).
//!
//! A read-only session runs every statement inside a read-only transaction that it rolls back
//! (L3, L6). Some PostgreSQL functions act outside the transaction: they write a WAL record,
//! create or move a replication slot or origin, or change the server's backup or recovery state
//! at once, so no `ROLLBACK` undoes them and the canary does not see them. The worst is
//! `pg_logical_emit_message(false, …)`, which `PUBLIC` may execute on PostgreSQL 13–18. This
//! layer refuses a statement that calls one of them, before any I/O.
//!
//! It reads the statement the way PostgreSQL's lexer does, closely enough to find a call: an
//! identifier (unquoted in any case, quoted, or `U&"…"` with its escapes) that names a listed
//! function and is followed by `(`, after any schema qualification, whitespace or comments. The
//! text of every string literal (`'…'`, `E'…'` and `U&'…'` with their escapes, `$tag$…$tag$`) is
//! read again as SQL, because a `DO` block, a PL/pgSQL `EXECUTE` or `query_to_xml` can run it.
//!
//! It is a tripwire, not a guarantee: SQL built from pieces at run time (`'pg_switch' ||
//! '_wal()'`, `format`, `chr`) names no function this layer can see. What keeps the label
//! honest is the role probe (row 6): a role that may `EXECUTE` one of these functions is not
//! least privilege (ADR 0004 `m2b-t05`, "F1").

use hacivat_proto::DbmsKind;

/// PostgreSQL functions whose effect a `ROLLBACK` does not undo, refused in a read-only session
/// (any schema qualification, quoting or case). Every overload of a name is refused.
///
/// - WAL records: `pg_logical_emit_message` (executable by `PUBLIC` on 13–18), `pg_switch_wal`,
///   `pg_create_restore_point`, `pg_log_standby_snapshot` (16+).
/// - Backup and recovery state: `pg_backup_start`/`pg_backup_stop` (15+),
///   `pg_start_backup`/`pg_stop_backup` (up to 14), `pg_promote`, `pg_wal_replay_pause`,
///   `pg_wal_replay_resume`.
/// - Replication slots (executable by `PUBLIC`, refused at run time without `REPLICATION`):
///   creating, copying, dropping and advancing one, consuming its changes, and
///   `pg_sync_replication_slots` (17+). Peeking (`pg_logical_slot_peek_*`) moves nothing.
/// - Replication origins (superuser before 14, then `EXECUTE` grants): every function that
///   creates, drops, advances or sets one up. Their reads (`_oid`, `_progress`, …) stay allowed.
pub const PG_NON_TRANSACTIONAL_FUNCTIONS: [&str; 27] = [
    "pg_logical_emit_message",
    "pg_switch_wal",
    "pg_create_restore_point",
    "pg_log_standby_snapshot",
    "pg_backup_start",
    "pg_backup_stop",
    "pg_start_backup",
    "pg_stop_backup",
    "pg_promote",
    "pg_wal_replay_pause",
    "pg_wal_replay_resume",
    "pg_create_physical_replication_slot",
    "pg_create_logical_replication_slot",
    "pg_copy_physical_replication_slot",
    "pg_copy_logical_replication_slot",
    "pg_drop_replication_slot",
    "pg_replication_slot_advance",
    "pg_logical_slot_get_changes",
    "pg_logical_slot_get_binary_changes",
    "pg_sync_replication_slots",
    "pg_replication_origin_create",
    "pg_replication_origin_drop",
    "pg_replication_origin_advance",
    "pg_replication_origin_session_setup",
    "pg_replication_origin_session_reset",
    "pg_replication_origin_xact_setup",
    "pg_replication_origin_xact_reset",
];

/// How deep string literals nest before the check stops following them (fails closed).
pub const MAX_STRING_NESTING: usize = 16;

/// How many bytes one check may read in all, counting each nested literal again (fails
/// closed): sixteen times the longest statement a session accepts.
const SCAN_BUDGET: usize = 16 * crate::input::MAX_STATEMENT_BYTES;

/// Why a read-only statement is refused before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionViolation {
    /// It calls this function of [`PG_NON_TRANSACTIONAL_FUNCTIONS`].
    NonTransactional(&'static str),
    /// Its string literals nest deeper than [`MAX_STRING_NESTING`], or reading them would take
    /// more than the check's budget: refused rather than guessed at.
    TooComplex,
}

/// Whether `sql` may be sent on a read-only session of `kind`: on PostgreSQL it must not call a
/// function of [`PG_NON_TRANSACTIONAL_FUNCTIONS`]. Every other engine passes (MySQL's
/// equivalents need privileges outside least privilege, which the probe reports).
pub fn check_functions(kind: DbmsKind, sql: &str) -> Result<(), FunctionViolation> {
    match kind {
        DbmsKind::Postgres => Scan {
            budget: SCAN_BUDGET,
        }
        .text(sql.as_bytes(), 0),
        _ => Ok(()),
    }
}

/// One check: the bytes it may still read.
struct Scan {
    budget: usize,
}

impl Scan {
    /// Reads `b` as SQL text `depth` literals deep.
    fn text(&mut self, b: &[u8], depth: usize) -> Result<(), FunctionViolation> {
        if depth > MAX_STRING_NESTING || b.len() > self.budget {
            return Err(FunctionViolation::TooComplex);
        }
        self.budget -= b.len();
        // A listed name just read, waiting for its `(`.
        let mut pending: Option<&'static str> = None;
        let mut i = 0;
        while let Some(&c) = b.get(i) {
            let next = b.get(i + 1).copied();
            if is_space(c) {
                i += 1;
            } else if c == b'-' && next == Some(b'-') {
                i = line_end(b, i + 2);
            } else if c == b'/' && next == Some(b'*') {
                i = block_end(b, i);
            } else if c == b'(' {
                if let Some(name) = pending {
                    return Err(FunctionViolation::NonTransactional(name));
                }
                i += 1;
            } else if matches!(c, b'e' | b'E') && next == Some(b'\'') {
                let (content, end) = quoted(b, i + 1, true);
                self.text(&unescape_backslashes(&content), depth + 1)?;
                pending = None;
                i = end;
            } else if matches!(c, b'u' | b'U')
                && next == Some(b'&')
                && matches!(b.get(i + 2), Some(b'\'' | b'"'))
            {
                let quote = b[i + 2];
                let (content, end) = quoted(b, i + 2, false);
                let (escape, end) = uescape(b, end);
                let decoded = unescape_unicode(&content, escape);
                pending = None;
                if quote == b'\'' {
                    self.text(&decoded, depth + 1)?;
                } else {
                    pending = listed(&decoded);
                }
                i = end;
            } else if c == b'\'' {
                let (content, end) = quoted(b, i, false);
                self.text(&content, depth + 1)?;
                // Where `standard_conforming_strings` is off when the text runs (a `DO` block
                // may set it before its `EXECUTE`), backslashes escape: read that way too.
                if content.contains(&b'\\') {
                    self.text(&unescape_backslashes(&content), depth + 1)?;
                }
                pending = None;
                i = end;
            } else if c == b'"' {
                let (content, end) = quoted(b, i, false);
                pending = listed(&content);
                i = end;
            } else if c == b'$' {
                match dollar_tag_end(b, i) {
                    Some(open) => {
                        let tag = &b[i..open];
                        let close = find(&b[open..], tag).map(|p| open + p);
                        let body_end = close.unwrap_or(b.len());
                        self.text(&b[open..body_end], depth + 1)?;
                        i = close.map_or(b.len(), |p| p + tag.len());
                    }
                    // A parameter (`$1`) or a stray `$`.
                    None => i += 1,
                }
                pending = None;
            } else if is_ident_start(c) {
                let end = i + b[i..].iter().take_while(|&&d| is_ident_char(d)).count();
                pending = listed(&b[i..end]);
                i = end;
            } else {
                // A number, an operator, `.`, `,`: only the name right before `(` is called.
                pending = None;
                i += 1;
            }
        }
        Ok(())
    }
}

/// The listed function `name` is, ASCII case folded (unquoted names fold to lower case; a quoted
/// name in another case is refused too, which fails closed).
fn listed(name: &[u8]) -> Option<&'static str> {
    PG_NON_TRANSACTIONAL_FUNCTIONS
        .iter()
        .find(|f| f.as_bytes().eq_ignore_ascii_case(name))
        .copied()
}

/// PostgreSQL's whitespace (`[ \t\n\r\f\v]`).
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || c == b'$'
}

/// The index after the line comment whose text starts at `from` (PostgreSQL ends one at a line
/// feed or a carriage return).
fn line_end(b: &[u8], from: usize) -> usize {
    b[from.min(b.len())..]
        .iter()
        .position(|c| matches!(c, b'\n' | b'\r'))
        .map_or(b.len(), |p| from + p + 1)
}

/// The index after the block comment that opens at `start`; PostgreSQL's nest. An unterminated
/// one runs to the end.
fn block_end(b: &[u8], start: usize) -> usize {
    let mut depth = 0u32;
    let mut i = start;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'/', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    b.len()
}

/// Skips whitespace and comments from `i`.
fn skip_trivia(b: &[u8], mut i: usize) -> usize {
    loop {
        match (b.get(i).copied(), b.get(i + 1).copied()) {
            (Some(c), _) if is_space(c) => i += 1,
            (Some(b'-'), Some(b'-')) => i = line_end(b, i + 2),
            (Some(b'/'), Some(b'*')) => i = block_end(b, i),
            _ => return i,
        }
    }
}

/// The literal or quoted identifier whose opening quote is at `open`: its text with doubled
/// quotes undone, and the index after it. `backslash`: an `E'…'` string, where `\` escapes the
/// next byte (kept, for [`unescape_backslashes`]). A `'…'` string continues after a line break
/// (PostgreSQL joins `'a'` newline `'b'` into one constant). An unterminated one runs to the end.
fn quoted(b: &[u8], open: usize, backslash: bool) -> (Vec<u8>, usize) {
    let quote = b[open];
    let mut out = Vec::new();
    let mut i = open + 1;
    while let Some(&c) = b.get(i) {
        if backslash && c == b'\\' {
            out.extend_from_slice(&b[i..(i + 2).min(b.len())]);
            i += 2;
        } else if c == quote && b.get(i + 1) == Some(&quote) {
            out.push(quote);
            i += 2;
        } else if c == quote {
            match continuation(b, i + 1).filter(|_| quote == b'\'') {
                Some(next) => i = next + 1,
                None => return (out, i + 1),
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    (out, b.len())
}

/// Where a string constant continues after its closing quote: the next opening quote, when only
/// whitespace with at least one line break, and line comments after it, come between.
fn continuation(b: &[u8], mut i: usize) -> Option<usize> {
    let mut newline = false;
    loop {
        match (b.get(i).copied(), b.get(i + 1).copied()) {
            (Some(b'\n' | b'\r'), _) => {
                newline = true;
                i += 1;
            }
            (Some(c), _) if is_space(c) => i += 1,
            (Some(b'-'), Some(b'-')) if newline => i = line_end(b, i + 2),
            (Some(b'\''), _) if newline => return Some(i),
            _ => return None,
        }
    }
}

/// A `U&` literal's optional `UESCAPE 'c'` clause from `i`: its escape character (`\` without
/// one) and the index after the literal.
fn uescape(b: &[u8], i: usize) -> (u8, usize) {
    let j = skip_trivia(b, i);
    let keyword = b"UESCAPE";
    let is_clause = b
        .get(j..j + keyword.len())
        .is_some_and(|w| w.eq_ignore_ascii_case(keyword))
        && !b.get(j + keyword.len()).is_some_and(|&c| is_ident_char(c));
    if is_clause {
        let k = skip_trivia(b, j + keyword.len());
        if let (Some(b'\''), Some(&escape), Some(b'\'')) = (b.get(k), b.get(k + 1), b.get(k + 2)) {
            return (escape, k + 3);
        }
    }
    (b'\\', i)
}

/// The text of a `U&` literal with its escapes undone: `<e>XXXX`, `<e>+XXXXXX` and `<e><e>`.
/// A code point that is no character (a lone surrogate) is dropped; the server refuses it.
fn unescape_unicode(s: &[u8], escape: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        if c == escape {
            if s.get(i + 1) == Some(&escape) {
                out.push(escape);
                i += 2;
                continue;
            }
            let (skip, digits) = if s.get(i + 1) == Some(&b'+') {
                (2, 6)
            } else {
                (1, 4)
            };
            if let Some(cp) = hex(s.get(i + skip..).unwrap_or_default(), digits) {
                push_char(&mut out, cp);
                i += skip + digits;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The text of an `E'…'` string with its backslash escapes undone: `\b \f \n \r \t`, octal
/// (1–3 digits), `\x` (1–2 hex digits), `\u` (4), `\U` (8); any other escaped byte is itself.
fn unescape_backslashes(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while let Some(&c) = s.get(i) {
        if c != b'\\' {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(&e) = s.get(i + 1) else {
            out.push(c);
            break;
        };
        i += 2;
        let rest = &s[i..];
        match e {
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'0'..=b'7' => {
                let more = rest
                    .iter()
                    .take(2)
                    .take_while(|d| (b'0'..=b'7').contains(*d))
                    .count();
                let value = std::iter::once(&e)
                    .chain(&rest[..more])
                    .fold(0u32, |v, d| v * 8 + u32::from(d - b'0'));
                // The server keeps the low byte.
                out.push(value.to_le_bytes()[0]);
                i += more;
            }
            b'x' if rest.first().is_some_and(u8::is_ascii_hexdigit) => {
                let more = rest
                    .iter()
                    .take(2)
                    .take_while(|d| d.is_ascii_hexdigit())
                    .count();
                let value = hex(rest, more).unwrap_or_default();
                out.push(value.to_le_bytes()[0]);
                i += more;
            }
            b'u' if hex(rest, 4).is_some() => {
                push_char(&mut out, hex(rest, 4).unwrap_or_default());
                i += 4;
            }
            b'U' if hex(rest, 8).is_some() => {
                push_char(&mut out, hex(rest, 8).unwrap_or_default());
                i += 8;
            }
            other => out.push(other),
        }
    }
    out
}

/// The value of the first `digits` bytes of `s` when all are hex digits.
fn hex(s: &[u8], digits: usize) -> Option<u32> {
    let s = s.get(..digits)?;
    s.iter().try_fold(0u32, |v, &d| {
        let d = char::from(d).to_digit(16)?;
        v.checked_mul(16)?.checked_add(d)
    })
}

/// Appends code point `cp` in UTF-8; nothing when it is no character.
fn push_char(out: &mut Vec<u8>, cp: u32) {
    if let Some(ch) = char::from_u32(cp) {
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
}

/// The index after the opening tag of a dollar-quoted string at `i` (`$$` or `$tag$`, whose
/// tag starts like an identifier and holds no `$`), or `None` for a parameter or a stray `$`.
fn dollar_tag_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if b.get(j).is_some_and(|&c| is_ident_start(c)) {
        j += b[j..]
            .iter()
            .take_while(|&&c| is_ident_char(c) && c != b'$')
            .count();
    }
    (b.get(j) == Some(&b'$')).then_some(j + 1)
}

/// The first position of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
