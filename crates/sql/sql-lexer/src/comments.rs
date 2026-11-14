//! Comments (`alg/10` LEX-02 rule 3, LEX-05) and psql `COPY` data.
//!
//! - Line comments: `--` (MySQL/MariaDB only before whitespace, a control character or the
//!   end) and MySQL's `#`, to before the line feed (PostgreSQL: also before a carriage return).
//! - Block comments: PostgreSQL's nest, counted with a `u32` (a depth-10⁶ comment is one
//!   linear scan, nothing recurses); the other dialects' close at the first `*/`. SQLite's may
//!   run to the end without an error, and its `/*` with nothing after is two operators.
//! - MySQL/MariaDB optimizer hints `/*+ … */` are `HintComment` wherever they stand (the
//!   parser decides whether one follows its statement keyword); the token ends at the first
//!   `*/`.
//! - Executable comments `/*! … */` (MariaDB also `/*M! … */`, an ordinary comment on MySQL)
//!   are one `ExecutableComment` token that may hold **one** nested comment, ending where a
//!   server that *skips* it ends it: `consume_comment(lip, 1)` (ADR 0004 step 1a, fix round 2).
//!   A deeper `/*` is text. `exec_comment_gate` reads the version gate.
//!
//! **Two readings of one body (review I1).** A server that *runs* an executable comment (an
//! unversioned one always, a versioned one from its version on) lexes the body as code: a `*/`
//! inside a quoted string or identifier, or inside a `#`/`-- ` comment, does not close it,
//! while a `/*` inside quotes opens no nested comment. In
//! `SELECT 1 /*!50000 , '*/' */ ; DROP TABLE t; -- '` the skipped reading ends at the first
//! `*/` and sees one statement; MySQL 8.4 runs the comment, ends it after `'*/' */`, and runs
//! `DROP TABLE t` as a second statement. Hints may have the same question (a quoted `SET_VAR`
//! value): the controller ruled that they fail closed the same way.
//!
//! So each executable comment and hint is also read the running way (`read_running`: quotes
//! and identifiers by the session's modes, `#` and `-- ` to the line feed, one nested
//! `/*…*/`), within the token, linearly. When that reading does not close at the token's own
//! `*/` (it closes earlier, is still open there, or meets an inner executable comment), the
//! token is flagged `ERROR` with `AmbiguousCommentEnd`. The token keeps the skipped extent, so
//! the plan's pinned end and exact resumption hold; **Task 8 treats a statement holding such a
//! token as `Ambiguous`** (execution mode never sends it; Run asks). The comment states carry
//! the running reading's place (`BodyReading`), so a resumed lex reaches the same flag.

use crate::cursor::{Cursor, Scan};
use crate::state::{BodyReading, LexState};
use crate::token::{LexErrorCode, TokenKind};
use sql_dialect::lex_profiles::ExecVendor;

/// The line comment starting at `pos` (`--` or `#`), to before its line break.
pub(crate) fn line(c: &mut Cursor, pos: usize) -> Scan {
    let end = if c.p.comments.line_ends_at_cr {
        c.find2(pos, b'\n', b'\r')
    } else {
        c.find(pos, b'\n')
    };
    Scan::new(TokenKind::LineComment, end.unwrap_or(c.len()))
}

/// The comment opened by the `/*` at `pos`; `None` when it is no comment here (SQLite's `/*`
/// with nothing after it).
pub(crate) fn block(c: &mut Cursor, pos: usize) -> Option<Scan> {
    let comments = c.p.comments;
    let third = c.at(pos + 2);
    if third.is_none() && comments.open_at_end_is_operators {
        return None;
    }
    let executable = match (comments.executable, third) {
        (Some(_), Some(b'!')) => true,
        (Some(ExecVendor::MariaDb), Some(b'M')) => c.at(pos + 3) == Some(b'!'),
        _ => false,
    };
    Some(if executable {
        executable_from(c, pos + 2, 1, BodyReading::Code)
    } else if comments.hints && third == Some(b'+') {
        hint_from(c, pos + 2, BodyReading::Code)
    } else {
        block_from(c, pos + 2, 1)
    })
}

/// The rest of a block comment `depth` levels deep whose text continues at `from`.
pub(crate) fn block_from(c: &mut Cursor, from: usize, depth: u32) -> Scan {
    let end = if c.p.comments.nested {
        nested_end(c, from, depth, u32::MAX)
    } else {
        c.find_seq(from, b"*/").map(|f| f + 2).ok_or(1)
    };
    match end {
        Ok(end) => Scan::new(TokenKind::BlockComment, end),
        Err(depth) => {
            let error =
                (!c.p.comments.unterminated_ok).then_some(LexErrorCode::UnterminatedComment);
            Scan::unterminated(
                TokenKind::BlockComment,
                c.len(),
                LexState::InBlockComment { depth },
                error,
            )
        }
    }
}

/// The rest of an optimizer hint whose text continues at `from`, where a running reading of
/// its body stands at `body`.
pub(crate) fn hint_from(c: &mut Cursor, from: usize, body: BodyReading) -> Scan {
    let skipped = c.find_seq(from, b"*/").map(|f| f + 2);
    let (running, ambiguous) = running_reading(c, from, skipped, body);
    let scan = match skipped {
        Some(end) => Scan::new(TokenKind::HintComment, end),
        None => Scan::unterminated(
            TokenKind::HintComment,
            c.len(),
            LexState::InHintComment { body: running },
            Some(LexErrorCode::UnterminatedComment),
        ),
    };
    flag_ambiguous(scan, ambiguous)
}

/// The rest of an executable comment `depth` levels deep (1, or 2 inside its one nested
/// comment) whose text continues at `from`, where a running reading of its body stands at
/// `body`.
pub(crate) fn executable_from(c: &mut Cursor, from: usize, depth: u32, body: BodyReading) -> Scan {
    let skipped = nested_end(c, from, depth, 2);
    let (running, ambiguous) = running_reading(c, from, skipped.ok(), body);
    let scan = match skipped {
        Ok(end) => Scan::new(TokenKind::ExecutableComment, end),
        Err(depth) => Scan::unterminated(
            TokenKind::ExecutableComment,
            c.len(),
            LexState::InExecutableComment {
                depth,
                body: running,
            },
            Some(LexErrorCode::UnterminatedComment),
        ),
    };
    flag_ambiguous(scan, ambiguous)
}

fn flag_ambiguous(scan: Scan, ambiguous: bool) -> Scan {
    if ambiguous {
        scan.rejected(LexErrorCode::AmbiguousCommentEnd)
    } else {
        scan
    }
}

/// Reads a body the running way from `from` in `body`, within the token (to the skipped end
/// `skipped`, or the text's end when the token is unterminated). Returns the running reading's
/// state at the token's end (`Diverged` once it has closed) and whether the two readings
/// disagree about where the comment ends.
fn running_reading(
    c: &mut Cursor,
    from: usize,
    skipped: Option<usize>,
    body: BodyReading,
) -> (BodyReading, bool) {
    let limit = skipped.unwrap_or(c.len());
    let running = if body == BodyReading::Diverged {
        Err(BodyReading::Diverged)
    } else {
        read_running(c, from, limit, body)
    };
    match (skipped, running) {
        // Both readings end at the token's `*/`.
        (Some(end), Ok(r)) if r == end => (BodyReading::Code, false),
        // The running reading closed first.
        (_, Ok(_)) => (BodyReading::Diverged, true),
        // Still open at the token's `*/`: a server that runs the body ends it later, or never.
        (Some(_), Err(state)) => (state, true),
        // Both still open at the text's end; ambiguous only once the readings have parted.
        (None, Err(state)) => (state, state == BodyReading::Diverged),
    }
}

/// Reads a MySQL/MariaDB comment body as a server that runs it does, from `i` in `state`, no
/// further than `limit`: `Ok(end)` when its `*/` closes it at `end`, else the state at `limit`.
///
/// Code: `'…'` and `"…"` strings (backslash escapes unless `NO_BACKSLASH_ESCAPES`; `"…"` is an
/// identifier under `ANSI_QUOTES`), `` `…` `` identifiers, doubled quotes, `#` and `-- ` to
/// the line feed, one nested `/*…*/` (opaque to its first `*/`). An inner executable comment
/// (`/*!`, MariaDB `/*M!`) is code of its own that this reading does not follow: `Diverged`.
/// Linear: each byte is read once.
fn read_running(
    c: &mut Cursor,
    mut i: usize,
    limit: usize,
    mut state: BodyReading,
) -> Result<usize, BodyReading> {
    let b = c.b;
    let mariadb = c.p.comments.executable == Some(ExecVendor::MariaDb);
    let modes = c.modes;
    while i < limit {
        match state {
            BodyReading::Diverged => break,
            BodyReading::Code => {
                c.steps += 1;
                match b[i] {
                    b'*' if b.get(i + 1) == Some(&b'/') && i + 2 <= limit => return Ok(i + 2),
                    b'/' if b.get(i + 1) == Some(&b'*') => {
                        let inner = b.get(i + 2) == Some(&b'!')
                            || (mariadb && b.get(i + 2..i + 4) == Some(b"M!"));
                        if inner {
                            return Err(BodyReading::Diverged);
                        }
                        state = BodyReading::Nested;
                        i += 2;
                    }
                    quote @ (b'\'' | b'"' | b'`') => {
                        state = BodyReading::Quoted { quote };
                        i += 1;
                    }
                    b'#' => {
                        state = BodyReading::LineComment;
                        i += 1;
                    }
                    b'-' if b.get(i + 1) == Some(&b'-')
                        && b.get(i + 2)
                            .is_none_or(|n| n.is_ascii_whitespace() || n.is_ascii_control()) =>
                    {
                        state = BodyReading::LineComment;
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            BodyReading::Quoted { quote } => {
                let escapes = !modes.no_backslash_escapes
                    && (quote == b'\'' || (quote == b'"' && !modes.ansi_quotes));
                let window = &b[i..limit];
                let found = if escapes {
                    memchr::memchr2(quote, b'\\', window)
                } else {
                    memchr::memchr(quote, window)
                };
                c.steps += found.map_or(window.len(), |f| f + 1) as u64;
                match found.map(|f| i + f) {
                    None => i = limit,
                    Some(f) if b[f] != quote => i = f + 2,
                    Some(f) if f + 1 < limit && b[f + 1] == quote => i = f + 2,
                    Some(f) => {
                        state = BodyReading::Code;
                        i = f + 1;
                    }
                }
            }
            BodyReading::LineComment => {
                let window = &b[i..limit];
                let found = memchr::memchr(b'\n', window);
                c.steps += found.map_or(window.len(), |f| f + 1) as u64;
                match found {
                    None => i = limit,
                    Some(f) => {
                        state = BodyReading::Code;
                        i += f + 1;
                    }
                }
            }
            BodyReading::Nested => {
                let window = &b[i..limit];
                let found = memchr::memmem::find(window, b"*/");
                c.steps += found.map_or(window.len(), |f| f + 2) as u64;
                match found {
                    None => i = limit,
                    Some(f) => {
                        state = BodyReading::Code;
                        i += f + 2;
                    }
                }
            }
        }
    }
    Err(state)
}

/// The index after the `*/` that closes a comment `depth` levels deep, scanning from `from`;
/// a `/*` opens a level while fewer than `max_depth` are open. `Err(depth)` when the text ends
/// first. A counter, never recursion.
fn nested_end(c: &mut Cursor, from: usize, depth: u32, max_depth: u32) -> Result<usize, u32> {
    // A hand-built state of depth 0 still means "inside a comment".
    let mut depth = depth.max(1);
    let mut i = from;
    while let Some(f) = c.find2(i, b'*', b'/') {
        let next = c.at(f + 1);
        if c.b[f] == b'*' && next == Some(b'/') {
            depth -= 1;
            i = f + 2;
            if depth == 0 {
                return Ok(i);
            }
        } else if c.b[f] == b'/' && next == Some(b'*') && depth < max_depth {
            depth += 1;
            i = f + 2;
        } else {
            i = f + 1;
        }
    }
    Err(depth)
}

/// psql `COPY … FROM STDIN` data whose lines start at `from`: whole lines up to and with the
/// line that is exactly `\.` (SQL-SPLIT-01 hands the data segment over in `InCopyData`).
pub(crate) fn copy_data_from(c: &mut Cursor, from: usize) -> Scan {
    let mut line = from;
    while line < c.len() {
        if c.b[line..].starts_with(b"\\.")
            && c.at(line + 2).is_none_or(|b| b == b'\n' || b == b'\r')
        {
            return Scan::new(TokenKind::InlineData, line + 2);
        }
        match c.find(line, b'\n') {
            Some(lf) => line = lf + 1,
            None => break,
        }
    }
    Scan::unterminated(TokenKind::InlineData, c.len(), LexState::InCopyData, None)
}
