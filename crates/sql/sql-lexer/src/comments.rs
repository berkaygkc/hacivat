//! Comments (`alg/10` LEX-02 rule 3, LEX-05) and psql `COPY` data.
//!
//! - Line comments: `--` (MySQL/MariaDB only before whitespace, a control character or the
//!   end) and MySQL's `#`, to before the line feed (PostgreSQL: also before a carriage return).
//! - Block comments: PostgreSQL's nest, counted with a `u32` (a depth-10⁶ comment is one
//!   linear scan, nothing recurses); the other dialects' close at the first `*/`. SQLite's may
//!   run to the end without an error, and its `/*` with nothing after is two operators.
//! - MySQL/MariaDB optimizer hints `/*+ … */` are `HintComment` wherever they stand (the
//!   parser decides whether one follows its statement keyword); they do not nest.
//! - Executable comments `/*! … */` (MariaDB also `/*M! … */`, an ordinary comment on MySQL)
//!   are one `ExecutableComment` token that may hold **one** nested comment: a skipped
//!   version-gated comment is consumed by the server's `consume_comment(lip, 1)` (ADR 0004
//!   step 1a, fix round 2), and a running one reads its inner `/*…*/` as an ordinary,
//!   non-nesting comment before its own `*/`, so both readings end at the same `*/`. A deeper
//!   `/*` is text. `exec_comment_gate` reads the version gate.

use crate::cursor::{Cursor, Scan};
use crate::state::LexState;
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
        executable_from(c, pos + 2, 1)
    } else if comments.hints && third == Some(b'+') {
        hint_from(c, pos + 2)
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

/// The rest of an optimizer hint whose text continues at `from`.
pub(crate) fn hint_from(c: &mut Cursor, from: usize) -> Scan {
    match c.find_seq(from, b"*/") {
        Some(f) => Scan::new(TokenKind::HintComment, f + 2),
        None => Scan::unterminated(
            TokenKind::HintComment,
            c.len(),
            LexState::InHintComment,
            Some(LexErrorCode::UnterminatedComment),
        ),
    }
}

/// The rest of an executable comment `depth` levels deep (1, or 2 inside its one nested
/// comment) whose text continues at `from`.
pub(crate) fn executable_from(c: &mut Cursor, from: usize, depth: u32) -> Scan {
    match nested_end(c, from, depth, 2) {
        Ok(end) => Scan::new(TokenKind::ExecutableComment, end),
        Err(depth) => Scan::unterminated(
            TokenKind::ExecutableComment,
            c.len(),
            LexState::InExecutableComment { depth },
            Some(LexErrorCode::UnterminatedComment),
        ),
    }
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
