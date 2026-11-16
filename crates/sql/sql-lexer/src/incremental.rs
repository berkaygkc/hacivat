//! Incremental relexing (SQL-LEX-03; Task 5): public checkpoints, and a relex that restarts at
//! the last exact state before an edit and splices until the new tokens re-synchronise.
//!
//! **Why the result equals a full lex.** A token the lexer scans in `Initial` depends only on
//! the bytes from its start on (the lexer never looks behind a token's start; the one rule that
//! reads the absolute offset, a byte-order mark at offset 0, is kept by lexing the whole new
//! text with one cursor). So:
//! - lexing from an exact point before the edit (a token start lexed in `Initial`: a stored
//!   checkpoint, the start of the edit's line when the old lexing has a token there, or offset
//!   0 in the lexing's start state) gives the full lexing's tokens from there;
//! - once that lexing reaches a token start at or after the edit's end whose old position (the
//!   same suffix bytes) is also a token start of the old lexing, every later token is the old
//!   one, moved by the edit's length change. That point is where the splice stops.
//!
//! The edit's own line is the usual restart point, so typing relexes the tokens of one line.
//! An edit that opens a string, comment or dollar quote re-synchronises nowhere and relexes to
//! the end (linear, `memchr`-driven: the "< 10 ms for 1 MB" case of SQL-LEX-03). An edit that
//! touches a client command, a `DELIMITER` line or a lexical-mode setting (`sql_mode`,
//! `standard_conforming_strings`) relexes to the end on purpose (R-M3-65: they change how the
//! rest of the script is read), and says so (`RelexDelta::full`).

use sql_dialect::{LexDialect, LexModes};
use sql_text::{TextEdit, TextRange, TextSize};

use crate::cursor::{CHECKPOINT_BYTES, Cursor, lex_with_client, text_size};
use crate::relex::RelexDelta;
use crate::state::LexState;
use crate::token::{Lexed, TokenKind};

impl Lexed {
    /// The last offset at or before `at` where lexing can restart, with the state there: a
    /// checkpoint (a token start at a line start, lexed in `Initial`, about every 2 KiB), or
    /// offset 0 in the state this lexing began in. Lexing the text from there in that state
    /// gives this lexing's tokens from there on.
    pub fn checkpoint_before(&self, at: TextSize) -> (TextSize, LexState) {
        let i = self.checkpoints.partition_point(|c| *c <= at);
        match i.checked_sub(1) {
            Some(i) => (self.checkpoints[i], LexState::Initial),
            None => (TextSize::from(0), self.start.clone()),
        }
    }
}

/// `s` moved by `delta` bytes.
fn shift(s: TextSize, delta: isize) -> TextSize {
    text_size(usize::from(s).wrapping_add_signed(delta))
}

/// Relexes `lexed` for `text`, the text after `edit` (see the module documentation).
pub(crate) fn relex_text(
    lexed: &mut Lexed,
    text: &str,
    edit: &TextEdit,
    profile: &LexDialect,
    modes: LexModes,
) -> RelexDelta {
    let old_len = usize::from(lexed.len);
    let (e0, e1_old) = (
        usize::from(edit.delete.start()),
        usize::from(edit.delete.end()),
    );
    let e1_new = e0 + edit.insert.len();
    let fits = e0 <= e1_old
        && e1_old <= old_len
        && old_len - (e1_old - e0) + edit.insert.len() == text.len()
        && text.get(e0..e1_new) == Some(edit.insert.as_str());
    if !fits {
        // The edit does not describe this change: lex the whole text.
        let start = lexed.start.clone();
        *lexed = lex_with_client(text, profile, modes, &start, lexed.client);
        return RelexDelta {
            changed: 0..lexed.len(),
            full: true,
        };
    }
    let delta = e1_new as isize - e1_old as isize;
    let (cp, state) = restart_point(lexed, text, e0);
    let i0 = lexed.starts.partition_point(|s| usize::from(*s) < cp);
    let mut forced = touches_tail_rules(lexed, text, e0, e1_old, e1_new);

    let mut cursor = Cursor::with_client(text, profile, modes, lexed.client);
    let mut out = Lexed::with_capacity(64, text_size(text.len()));
    let mut pos = cp;
    if cp == 0 && state != LexState::Initial {
        if text.is_empty() {
            out.end_state = state;
        } else {
            let scan = cursor.resume(&state);
            pos = cursor.emit(&mut out, 0, scan);
        }
    }
    let mut next_checkpoint = cp;
    let mut resync = None;
    while pos < text.len() {
        if !forced && pos >= e1_new && pos > 0 {
            // The suffix from `pos` is the old text's suffix from `old`: a token start there in
            // both lexings re-synchronises them.
            let old = pos.wrapping_add_signed(-delta);
            if old > 0
                && let Ok(j) = lexed.starts.binary_search(&text_size(old))
            {
                resync = Some(j);
                break;
            }
        }
        if pos >= next_checkpoint && cursor.checkpoint_at(pos) {
            out.checkpoints.push(text_size(pos));
            next_checkpoint = pos + CHECKPOINT_BYTES;
        }
        let scan = cursor.scan(pos);
        if scan.kind == TokenKind::ClientCommand && pos <= e1_new && scan.end >= e0 {
            forced = true;
        }
        pos = cursor.emit(&mut out, pos, scan);
    }

    let j = resync.unwrap_or(lexed.kinds.len());
    let old_end = resync.map_or(old_len, |j| usize::from(lexed.starts[j]));
    let unchanged = same_prefix(lexed, i0, j, &out, e0);
    let fresh = out.kinds.len();
    let tail = i0 + fresh;
    lexed.kinds.splice(i0..j, out.kinds);
    lexed.flags.splice(i0..j, out.flags);
    lexed.keywords.splice(i0..j, out.keywords);
    lexed.starts.splice(i0..j, out.starts);
    for s in &mut lexed.starts[tail..] {
        *s = shift(*s, delta);
    }
    let lo = lexed
        .errors
        .partition_point(|e| usize::from(e.range.start()) < cp);
    let hi = lexed
        .errors
        .partition_point(|e| usize::from(e.range.start()) < old_end);
    let moved = lo + out.errors.len();
    lexed.errors.splice(lo..hi, out.errors);
    for e in &mut lexed.errors[moved..] {
        e.range = TextRange::new(shift(e.range.start(), delta), shift(e.range.end(), delta));
    }
    // ## m3b-t05-incremental fix round 1 (C1): which checkpoints stay exact. One before `cp`
    // keeps its bytes and its line feed (both lie before the edit, and the token before it ends
    // at that line feed). One after the old resync point `old_end` keeps the line feed before it
    // too: that byte is at or after `old_end`, which is at or after the edit's end, so it is
    // part of the unchanged suffix. One *at* `old_end` does not: the new resync point may be the
    // edit's end itself, so the byte before it may be new text (a joined line). It is dropped,
    // as is every one inside the relexed region (the new lexing records its own there).
    let lo = lexed.checkpoints.partition_point(|c| usize::from(*c) < cp);
    let hi = lexed
        .checkpoints
        .partition_point(|c| usize::from(*c) <= old_end);
    let moved = lo + out.checkpoints.len();
    lexed.checkpoints.splice(lo..hi, out.checkpoints);
    for c in &mut lexed.checkpoints[moved..] {
        *c = shift(*c, delta);
    }
    if resync.is_none() {
        lexed.end_state = out.end_state;
    }
    lexed.len = text_size(text.len());
    RelexDelta {
        changed: i0 + unchanged..tail,
        full: resync.is_none(),
    }
}

/// Where relexing for an edit at `e0` restarts, and in which state: before the token that
/// holds the byte before the edit (the edit may merge with it: `-` + `-`, `$a` + `$`, a word and
/// a word), at the start of that token's line when the old lexing has a token there (lexed in
/// `Initial`, so exact), else at the checkpoint before it.
fn restart_point(lexed: &Lexed, text: &str, e0: usize) -> (usize, LexState) {
    if e0 == 0 || lexed.kinds.is_empty() {
        return (0, lexed.start.clone());
    }
    let Some(t) = lexed.token_at(text_size(e0 - 1)) else {
        return (0, lexed.start.clone());
    };
    let token_start = usize::from(lexed.starts[t]);
    let (cp, state) = lexed.checkpoint_before(text_size(token_start));
    let cp = usize::from(cp);
    // Second guard (fix round 1, C1): a checkpoint is a line start by construction; were one
    // not, the token before it could look past it (`U` + `&'`, `-` + `- `), so lexing would
    // restart at the text's start rather than there.
    let (cp, state) = if cp > 0 && text.as_bytes()[cp - 1] != b'\n' {
        debug_assert!(false, "checkpoint {cp} is not a line start");
        (0, lexed.start.clone())
    } else {
        (cp, state)
    };
    // The bytes before the edit are the old text's.
    let line = memchr::memrchr(b'\n', &text.as_bytes()[..token_start]).map_or(0, |lf| lf + 1);
    if line > cp
        && let Ok(i) = lexed.starts.binary_search(&text_size(line))
        && i > 0
    {
        return (line, LexState::Initial);
    }
    (cp, state)
}

/// How many of the new tokens `out`, from old token `i0` on, are the old tokens unchanged
/// (same start, kind, flags, keyword and end, all before the edit at `e0`).
fn same_prefix(old: &Lexed, i0: usize, j: usize, out: &Lexed, e0: usize) -> usize {
    let mut k = 0;
    while k < out.kinds.len() && i0 + k < j {
        let o = i0 + k;
        let old_end = old.starts.get(o + 1).map_or(old.len, |s| *s);
        let new_end = out.starts.get(k + 1).copied();
        let same = old.starts[o] == out.starts[k]
            && old.kinds[o] == out.kinds[k]
            && old.flags[o] == out.flags[k]
            && old.keywords[o] == out.keywords[k]
            && usize::from(old_end) <= e0
            && new_end == Some(old_end);
        if !same {
            break;
        }
        k += 1;
    }
    k
}

/// How far around an edit its line is searched for a `DELIMITER` command or a lexical-mode
/// setting (those lines are short; a minified one-line script is not searched end to end on
/// every keystroke).
const RULE_WINDOW: usize = 256;

/// Whether the edit touches a client command (in the old lexing; the new one is checked as it
/// is lexed) or a line of the new text that is a `DELIMITER` command or a `SET` of `sql_mode`
/// or `standard_conforming_strings` (its first word is `DELIMITER`, or `SET` with the setting's
/// name on the line): such an edit relexes to the end (R-M3-65). A `SELECT @@sql_mode` does not
/// (fix round 1, M4). This is a signal, never needed for exactness: the lexing does not read
/// those statements (the session's modes are the document's context).
fn touches_tail_rules(lexed: &Lexed, text: &str, e0: usize, e1_old: usize, e1_new: usize) -> bool {
    let old_len = usize::from(lexed.len);
    if !lexed.kinds.is_empty() {
        let first = lexed.token_at(text_size(e0.saturating_sub(1))).unwrap_or(0);
        let last = lexed
            .token_at(text_size(e1_old.min(old_len)))
            .unwrap_or(first);
        if lexed.kinds[first..=last.max(first)].contains(&TokenKind::ClientCommand) {
            return true;
        }
    }
    let b = text.as_bytes();
    let from = e0.saturating_sub(RULE_WINDOW);
    let from = memchr::memrchr(b'\n', &b[from..e0]).map_or(from, |lf| from + lf + 1);
    let to = (e1_new + RULE_WINDOW).min(b.len());
    let to = memchr::memchr(b'\n', &b[e1_new..to]).map_or(to, |lf| e1_new + lf);
    b[from..to].split(|c| *c == b'\n').any(|line| {
        let first = line
            .iter()
            .position(|c| !c.is_ascii_whitespace())
            .map_or(&line[..0], |s| {
                let rest = &line[s..];
                &rest[..rest
                    .iter()
                    .position(|c| !c.is_ascii_alphanumeric() && *c != b'_')
                    .unwrap_or(rest.len())]
            });
        first.eq_ignore_ascii_case(b"delimiter")
            || (first.eq_ignore_ascii_case(b"set")
                && [&b"sql_mode"[..], b"standard_conforming_strings"]
                    .iter()
                    .any(|w| contains_ignore_ascii_case(line, w)))
    })
}

fn contains_ignore_ascii_case(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() >= needle.len()
        && hay
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use sql_dialect::{DialectCtx, DialectId, lex_profile};

    use super::*;

    #[test]
    fn checkpoints_are_line_starts_about_every_2_kib() {
        let ctx = DialectCtx::new(DialectId::Postgres, None);
        let profile = lex_profile(&ctx).unwrap();
        let text = "SELECT 1;\n".repeat(1_000);
        let l = crate::lex(&text, profile, ctx.modes, &LexState::Initial);
        assert_eq!(
            l.checkpoint_before(TextSize::from(12)),
            (TextSize::from(0), LexState::Initial)
        );
        let (cp, state) = l.checkpoint_before(TextSize::from(5_000));
        assert_eq!(state, LexState::Initial);
        let cp = usize::from(cp);
        assert!((2_048..=5_000).contains(&cp) && cp % 10 == 0, "{cp}");
    }
}
