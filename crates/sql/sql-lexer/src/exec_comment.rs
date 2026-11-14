//! MySQL/MariaDB executable comments and their version gates (SQL-LEX-05; Task 4).
//!
//! An `ExecutableComment` token is `/*!` or (MariaDB) `/*M!`, an optional version, the body,
//! and `*/`. The gate is for the editor (code colours, a dotted underline when unknown); the
//! classifier never trusts it and reads every body (M3b).

use sql_dialect::{DialectCtx, DialectId};
use sql_text::TextRange;

use crate::cursor::text_size;
use crate::token::{Lexed, TokenFlags, TokenKind};

/// Whether an executable comment's text runs on the connected server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GateStatus {
    Active,
    Inactive,
    /// No server version is known.
    Unknown,
}

/// Which opener an executable comment has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecMarker {
    /// `/*!`: MySQL and MariaDB run it.
    Bang,
    /// `/*M!`: only MariaDB runs it.
    MariaDb,
}

/// The parts of an executable comment token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecComment {
    pub marker: ExecMarker,
    /// The version digits (five, or six on MariaDB's scale), as written: `50110` is 5.1.10,
    /// `100100` is 10.1.0. `None`: the body always runs.
    pub version: Option<u32>,
    /// The opener with its version (`/*!50110`), for delimiter colours.
    pub opener: TextRange,
    /// The text between the opener and the closing `*/` (to the end when unterminated).
    pub body: TextRange,
}

/// The parts of token `i` of `lexed` over `text`; `None` when it is not an executable comment.
pub fn exec_comment(text: &str, lexed: &Lexed, i: usize) -> Option<ExecComment> {
    if lexed.kind(i) != TokenKind::ExecutableComment {
        return None;
    }
    let range = lexed.range(i);
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    // `None` too when `text` is not the text `lexed` was lexed from.
    let token = text.get(start..end)?.as_bytes();
    let (marker, open) = if token.starts_with(b"/*M!") {
        (ExecMarker::MariaDb, 4)
    } else if token.starts_with(b"/*!") {
        (ExecMarker::Bang, 3)
    } else {
        return None;
    };
    let digits = token[open..]
        .iter()
        .take(6)
        .take_while(|b| b.is_ascii_digit())
        .count();
    // Fewer than five digits are body text: the server reads no version there.
    let (version, opener_len) = if digits >= 5 {
        let value = token[open..open + digits]
            .iter()
            .fold(0u32, |v, d| v * 10 + u32::from(d - b'0'));
        (Some(value), open + digits)
    } else {
        (None, open)
    };
    let closed = !lexed.flags(i).contains(TokenFlags::UNTERMINATED)
        && token.len() >= open + 2
        && token.ends_with(b"*/");
    let body_end = if closed { end - 2 } else { end };
    let body_start = (start + opener_len).min(body_end);
    Some(ExecComment {
        marker,
        version,
        opener: TextRange::new(range.start(), text_size(start + opener_len)),
        body: TextRange::new(text_size(body_start), text_size(body_end)),
    })
}

/// The gate of token `i` of `lexed` (an `ExecutableComment`) under `ctx`:
/// - an unversioned `/*!` always runs on MySQL and MariaDB: `Active`;
/// - a versioned one runs when the server is at least that version (`Active`/`Inactive`), and
///   is `Unknown` without a server version;
/// - `/*M!` runs only on MariaDB (by the same version rule);
/// - on any other dialect, and for a token that is no executable comment, `Inactive`.
///
/// MySQL reads five version digits; on a MySQL server a sixth digit is body text.
pub fn exec_comment_gate(text: &str, lexed: &Lexed, i: usize, ctx: &DialectCtx) -> GateStatus {
    let Some(comment) = exec_comment(text, lexed, i) else {
        return GateStatus::Inactive;
    };
    let version = match (ctx.id, comment.marker) {
        (DialectId::MySql, ExecMarker::Bang) => {
            // `/*!` and five or six digits: MySQL reads the first five.
            let six = u32::from(comment.opener.len()) == 3 + 6;
            comment.version.map(|v| if six { v / 10 } else { v })
        }
        (DialectId::MariaDb, _) => comment.version,
        _ => return GateStatus::Inactive,
    };
    let Some(needed) = version else {
        return GateStatus::Active;
    };
    match ctx.version {
        None => GateStatus::Unknown,
        Some(server) => {
            let have = u32::from(server.major) * 10_000
                + u32::from(server.minor) * 100
                + u32::from(server.patch);
            if have >= needed {
                GateStatus::Active
            } else {
                GateStatus::Inactive
            }
        }
    }
}
