//! Operators (`alg/10` LEX-02 rule 10).
//!
//! PostgreSQL: maximal munch over its operator characters, cut before an embedded `--` or
//! `/*` (comments win: `+/*c*/1` is `+`, a comment, `1`); a multi-character operator may end in
//! `+` or `-` only if it holds one of `` ~ ! @ # % ^ & | ` ? `` (`=-1` is `=`, `-`, `1`; `@-1`
//! is `@-`, `1`), otherwise its trailing signs are separate operators; 64 bytes or more is an
//! error. The rule reads a run once: the next operator inside the same run reuses the run's
//! end and its last sign keeper, so a megabyte of `+-` stays linear.
//!
//! The other dialects have a fixed set: the longest listed operator, else a listed single
//! character, else the byte is `Unknown`.

use sql_dialect::lex_profiles::OperatorRule;

use crate::cursor::{Cursor, Scan, unknown};
use crate::token::{LexErrorCode, TokenKind};

/// One PostgreSQL operator run read so far: `[start, end)` holds operator characters (cut
/// before a comment start), `keeper` is the last sign keeper before `end - 1`, `plain` the last
/// byte that is not `+`/`-`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RunCache {
    start: usize,
    end: usize,
    keeper: Option<usize>,
    plain: Option<usize>,
}

/// The operator at `pos` (or `Unknown`, when no operator starts there).
pub(crate) fn scan(c: &mut Cursor, pos: usize) -> Scan {
    match c.p.operators {
        OperatorRule::Postgres {
            chars,
            sign_keepers,
            max_len,
        } => postgres(c, pos, chars, sign_keepers, max_len),
        OperatorRule::Table { multi, single } => {
            let rest = &c.b[pos..];
            c.steps += 3;
            if let Some(op) = multi.iter().find(|op| rest.starts_with(op)) {
                Scan::new(TokenKind::Op, pos + op.len())
            } else if single.contains(&c.b[pos]) {
                Scan::new(TokenKind::Op, pos + 1)
            } else {
                unknown(pos)
            }
        }
    }
}

fn postgres(c: &mut Cursor, pos: usize, chars: &[u8], keepers: &[u8], max_len: usize) -> Scan {
    if !chars.contains(&c.b[pos]) {
        return unknown(pos);
    }
    let run = if c.op_run.start <= pos && pos < c.op_run.end {
        c.op_run
    } else {
        read_run(c, pos, chars, keepers)
    };
    let end = run.end;
    let whole = end - pos == 1
        || !matches!(c.b[end - 1], b'+' | b'-')
        || run.keeper.is_some_and(|k| k >= pos);
    let len = if whole {
        end - pos
    } else {
        // Strip the trailing signs, keeping at least one byte.
        run.plain.filter(|p| *p >= pos).map_or(1, |p| p - pos + 1)
    };
    let op = Scan::new(TokenKind::Op, pos + len);
    if len >= max_len {
        op.rejected(LexErrorCode::OperatorTooLong)
    } else {
        op
    }
}

/// Reads the operator run at `pos` once and caches it. The run stops before the first
/// embedded `--` or `/*` (not at `pos`: a comment there was taken first), so its bytes are read
/// once, never past the cut: a run of comments between operators (`+/**/+/**/…`) stays linear.
fn read_run(c: &mut Cursor, pos: usize, chars: &[u8], keepers: &[u8]) -> RunCache {
    let b = c.b;
    let mut end = pos + 1;
    while end < b.len()
        && chars.contains(&b[end])
        && !matches!(b.get(end..end + 2), Some(b"--" | b"/*"))
    {
        end += 1;
    }
    let run = &b[pos..end];
    c.steps += run.len() as u64 + 1;
    let keeper = run[..run.len() - 1]
        .iter()
        .rposition(|b| keepers.contains(b))
        .map(|k| pos + k);
    let plain = run
        .iter()
        .rposition(|b| !matches!(b, b'+' | b'-'))
        .map(|p| pos + p);
    c.op_run = RunCache {
        start: pos,
        end,
        keeper,
        plain,
    };
    c.op_run
}
