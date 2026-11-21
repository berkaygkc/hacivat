//! The streaming splitter (SQL-SPLIT-05; Task 5): statements out of a script fed in chunks
//! (file execution, "Run as script" in M4b), with the client delimiter carried across chunks.
//!
//! Whatever the chunking, the stream yields exactly the execution split of the whole script,
//! in text order (`tests/stream.rs`): every statement (its strict text, offset and
//! confidence), client command and uncertain boundary. To do so it holds the text from the
//! last place where the pass carries nothing forward but the delimiter (an anchor,
//! `crate::incremental`), splits what it holds, and gives out the items before the latest
//! anchor that later text can no longer change:
//! - before an executable comment or hint whose end is ambiguous (its refusal runs forward to
//!   where the server ends it, maybe beyond the text held);
//! - on MySQL/MariaDB, before the last statement that tracked a block (a later stray `END`
//!   refuses every statement back to it; with none held, back to the script's start, so
//!   nothing is given out before the end of the stream);
//! - outside a psql `\if` branch.
//!
//! The text held is split again whenever it has doubled since the last split that gave out
//! nothing, so a script costs O(n) splitting overall.
//!
//! One statement longer than [`STREAM_STATEMENT_MAX`] is [`StreamItem::TooLong`]. When the
//! statement is still open at that size the splitter does not look for its end (it would have
//! to hold it): it yields the `TooLong` and nothing more, and drops the rest of the stream.
//!
//! All of the above is the default policy, [`StreamPolicy::Exact`]. Under
//! [`StreamPolicy::Eager`] (M4b "Run as script", R-M4b-46) MySQL/MariaDB statements are given
//! out without the block hold, and the stream stops at the first block mismatch with one
//! `UnknownBlock` item instead of refusing back: it equals the default stream for every script
//! whose blocks match (ADR 0005 `m4b-t08`).

use std::sync::Arc;

use sql_dialect::split_profile::{BlockFamily, client_profile};
use sql_dialect::{ClientFlavor, DialectCtx, split_profile};
use sql_lexer::LexState;
use sql_text::TextSize;

use crate::incremental::{anchor, if_depth};
use crate::item::{AmbiguityReason, Confidence, Item, SplitMode};
use crate::splitter::{Analysis, RawItem, SplitStart, analyze, size};

/// The longest statement the streaming splitter holds (64 MiB); a longer one is
/// [`StreamItem::TooLong`].
pub const STREAM_STATEMENT_MAX: usize = 64 << 20;

/// One item of a stream; offsets are byte offsets from the start of the stream.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StreamItem {
    Statement {
        text: String,
        offset: u64,
        confidence: Confidence,
    },
    Client {
        text: String,
        offset: u64,
        supported: bool,
    },
    Ambiguous {
        offset: u64,
        reason: AmbiguityReason,
    },
    TooLong {
        offset: u64,
    },
}

// ## m4b-t08-stream-hold
/// What the stream may give out before the end of the script (M4b Task 8, R-M4b-46).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StreamPolicy {
    /// The stream equals the whole script's execution split for every chunking (module
    /// documentation); on MySQL/MariaDB it holds everything after the last block-tracked
    /// statement.
    #[default]
    Exact,
    /// MySQL/MariaDB statements are given out before the latest anchor even with no block held,
    /// and the stream stops at the first block mismatch.
    Eager,
}

/// How far back from the latest item an anchor is looked for.
const ANCHOR_SCAN: usize = 256;

/// Splits a script fed in chunks (module documentation).
#[derive(Debug)]
pub struct StreamSplitter {
    ctx: DialectCtx,
    flavor: ClientFlavor,
    /// The text not given out yet, from an anchor (or the stream's start).
    held: String,
    /// The stream offset of `held`'s start.
    base: u64,
    /// The client delimiter in force at `held`'s start.
    delimiter: Option<Arc<str>>,
    /// `held`'s length at the last split that gave out nothing.
    tried: usize,
    /// A statement too long to hold was met: nothing more is split.
    stopped: bool,
    // ## m4b-t08-stream-hold
    policy: StreamPolicy,
}

impl StreamSplitter {
    pub fn new(ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        Self::with_policy(ctx, flavor, StreamPolicy::Exact)
    }

    // ## m4b-t08-stream-hold
    /// A splitter that gives out under `policy` ([`StreamPolicy`]).
    pub fn with_policy(ctx: DialectCtx, flavor: ClientFlavor, policy: StreamPolicy) -> Self {
        StreamSplitter {
            ctx,
            flavor,
            held: String::new(),
            base: 0,
            delimiter: None,
            tried: 0,
            stopped: false,
            policy,
        }
    }

    /// The bytes held, not given out yet (the memory test).
    #[doc(hidden)]
    pub fn held_len(&self) -> usize {
        self.held.len()
    }

    /// The items `chunk` settles (possibly none: they come once later text can no longer
    /// change them, at the latest at [`StreamSplitter::finish`]).
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamItem> {
        if self.stopped {
            return Vec::new();
        }
        self.held.push_str(chunk);
        let n = self.held.len();
        let over = n > STREAM_STATEMENT_MAX && self.tried <= STREAM_STATEMENT_MAX;
        if n < self.tried.saturating_mul(2) && !over {
            return Vec::new();
        }
        let a = self.analysis();
        let mut out = Vec::new();
        if let Some(x) = self.settled(&a) {
            self.release(&a, x, &mut out);
            self.tried = 0;
            return out;
        }
        self.tried = n;
        if n > STREAM_STATEMENT_MAX
            && let Some(RawItem::Stmt(s)) = a.items.last()
            && s.terminator.is_none()
            && n - usize::from(s.strict.start()) > STREAM_STATEMENT_MAX
        {
            // A statement still open past the cap: give out what precedes it, then stop.
            let at = usize::from(s.strict.start());
            self.release(&a, at, &mut out);
            if self.stopped {
                return out;
            }
            out.push(StreamItem::TooLong { offset: self.base });
            self.stopped = true;
            self.held = String::new();
        }
        out
    }

    /// The remaining items at the end of the stream.
    pub fn finish(mut self) -> Vec<StreamItem> {
        if self.stopped {
            return Vec::new();
        }
        let a = self.analysis();
        let mut out = Vec::new();
        let n = self.held.len();
        self.release(&a, n.saturating_add(1), &mut out);
        out
    }

    /// The pass over the text held.
    fn analysis(&self) -> Analysis {
        let start = SplitStart {
            lex: LexState::Initial,
            delimiter: self.delimiter.clone(),
        };
        analyze(&self.held, None, &self.ctx, self.flavor, &start)
    }

    /// The latest anchor of `a` before which no later text can change anything (module
    /// documentation).
    fn settled(&self, a: &Analysis) -> Option<usize> {
        let text = &self.held;
        let cp = client_profile(self.flavor);
        let mut limit = a.items.len();
        if self.policy == StreamPolicy::Exact
            && split_profile(&self.ctx).family == BlockFamily::MySql
        {
            limit = a
                .items
                .iter()
                .rposition(|it| matches!(it, RawItem::Stmt(s) if s.block))
                .map_or(0, |b| b + 1);
        }
        let ambiguous = a
            .ambiguous
            .iter()
            .find(|x| x.reason == AmbiguityReason::AmbiguousCommentEnd)
            .map_or(usize::MAX, |x| usize::from(x.at.start()));
        (1..limit)
            .rev()
            .take(ANCHOR_SCAN)
            .filter_map(|j| anchor(text, a, j))
            .find(|x| *x <= ambiguous && if_depth(text, cp, &a.client, *x) == Some(0))
    }

    // ## m4b-t08-stream-hold
    /// Gives out the items of `a` before `upto` as the policy allows: under
    /// [`StreamPolicy::Exact`], and off MySQL/MariaDB, all of them ([`Self::give_out`]).
    ///
    /// Under [`StreamPolicy::Eager`] on MySQL/MariaDB a block mismatch (`UnknownBlock`) may refuse
    /// statements already given out (the stray-`END` backstop refuses back to the last
    /// block-tracked statement, which is no longer held). So when `a` has one, the text before
    /// `upto` is split on its own: without one there (the mismatch lies in the text still held,
    /// and its refusal of earlier statements is the backstop's), its items are given out as if
    /// the mismatch were not there yet; with one, the stream stops at the first statement that
    /// brings one ([`Self::stop_at_mismatch`]).
    fn release(&mut self, a: &Analysis, upto: usize, out: &mut Vec<StreamItem>) {
        let eager = self.policy == StreamPolicy::Eager
            && split_profile(&self.ctx).family == BlockFamily::MySql;
        if !eager || !mismatched(a) {
            self.give_out(a, upto, out);
            return;
        }
        let end = upto.min(self.held.len());
        let prefix = (end < self.held.len()).then(|| self.prefix_analysis(end));
        let p = prefix.as_ref().unwrap_or(a);
        if mismatched(p) {
            self.stop_at_mismatch(p, end, out);
        } else {
            self.give_out(p, upto, out);
        }
    }

    /// The pass over the held text's first `end` bytes (`end` is an item's start or the end).
    fn prefix_analysis(&self, end: usize) -> Analysis {
        let start = SplitStart {
            lex: LexState::Initial,
            delimiter: self.delimiter.clone(),
        };
        analyze(&self.held[..end], None, &self.ctx, self.flavor, &start)
    }

    /// `p`, the pass over the held text's first `end` bytes, has a block mismatch: gives out
    /// what precedes the first statement that brings one, then one
    /// [`StreamItem::Ambiguous`] `UnknownBlock` at the mismatch, and stops the stream.
    ///
    /// The statement is found by splitting prefixes that end where an item starts (or at `end`):
    /// the first prefix with a mismatch ends just after it. A binary search over the items
    /// always lands on a prefix with a mismatch whose predecessor has none, so what is given
    /// out comes from a pass with no mismatch, and nothing from that statement on is given out.
    fn stop_at_mismatch(&mut self, p: &Analysis, end: usize, out: &mut Vec<StreamItem>) {
        let starts: Vec<usize> = p
            .items
            .iter()
            .map(|it| usize::from(item_start(it)))
            .collect();
        // Prefix `k` holds the items `..=k`; the last is `p` itself.
        let cut = |k: usize| starts.get(k + 1).copied().unwrap_or(end);
        let (mut lo, mut hi) = (0, starts.len().saturating_sub(1));
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if mismatched(&self.prefix_analysis(cut(mid))) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        let first = starts.get(lo).copied().unwrap_or(0);
        let bad = if cut(lo) == end {
            None
        } else {
            Some(self.prefix_analysis(cut(lo)))
        };
        let bad = bad.as_ref().unwrap_or(p);
        let from = match bad.items.get(lo) {
            Some(RawItem::Stmt(s)) => usize::from(s.strict.start()),
            _ => first,
        };
        let at = bad
            .ambiguous
            .iter()
            .filter(|x| x.reason == AmbiguityReason::UnknownBlock)
            .map(|x| usize::from(x.at.start()))
            .filter(|x| *x >= from)
            .min()
            .unwrap_or(from);
        let offset = self.base + at as u64;
        if first > 0 {
            let clean = self.prefix_analysis(first);
            self.give_out(&clean, first, out);
        }
        out.push(StreamItem::Ambiguous {
            offset,
            reason: AmbiguityReason::UnknownBlock,
        });
        self.stopped = true;
        self.held = String::new();
    }

    /// Gives out every item of `a` that starts before `upto` and drops the text before it.
    fn give_out(&mut self, a: &Analysis, upto: usize, out: &mut Vec<StreamItem>) {
        let text = &self.held;
        let base = self.base;
        let off = |p: TextSize| base + u64::from(u32::from(p));
        let split = a.to_split(text, SplitMode::Execution);
        let mut items: Vec<(u64, u8, StreamItem)> = Vec::new();
        for c in split
            .client
            .iter()
            .filter(|c| usize::from(c.range.start()) < upto)
        {
            items.push((
                off(c.range.start()),
                0,
                StreamItem::Client {
                    text: text[c.range].to_owned(),
                    offset: off(c.range.start()),
                    supported: c.supported,
                },
            ));
        }
        for s in split
            .items
            .iter()
            .filter_map(Item::statement)
            .filter(|s| usize::from(s.strict.start()) < upto)
        {
            let offset = off(s.strict.start());
            let item = if usize::from(s.strict.len()) > STREAM_STATEMENT_MAX {
                StreamItem::TooLong { offset }
            } else {
                StreamItem::Statement {
                    text: text[s.strict].to_owned(),
                    offset,
                    confidence: s.confidence,
                }
            };
            items.push((offset, 1, item));
        }
        for x in split
            .ambiguous
            .iter()
            .filter(|x| usize::from(x.at.start()) < upto)
        {
            let offset = off(x.at.start());
            items.push((
                offset,
                2,
                StreamItem::Ambiguous {
                    offset,
                    reason: x.reason,
                },
            ));
        }
        items.sort_by_key(|(o, rank, _)| (*o, *rank));
        out.extend(items.into_iter().map(|(_, _, it)| it));
        let cut = upto.min(self.held.len());
        self.delimiter = a.delimiter_at(size(cut));
        self.held.drain(..cut);
        self.base += cut as u64;
    }
}

// ## m4b-t08-stream-hold
/// Whether the pass found a block mismatch.
fn mismatched(a: &Analysis) -> bool {
    a.ambiguous
        .iter()
        .any(|x| x.reason == AmbiguityReason::UnknownBlock)
}

/// Where item `it` starts, its leading comments included.
fn item_start(it: &RawItem) -> TextSize {
    match it {
        RawItem::Stmt(s) => s
            .leading
            .map_or(s.strict.start(), |l| l.start().min(s.strict.start())),
        RawItem::Empty(r) => r.start(),
    }
}

#[cfg(test)]
mod tests {
    use sql_dialect::DialectId;

    use super::*;

    #[test]
    fn statements_come_out_once_settled() {
        let mut s = StreamSplitter::new(
            DialectCtx::new(DialectId::Postgres, None),
            ClientFlavor::Psql,
        );
        // The second statement may still grow: only the first is settled.
        let first = s.feed("SELECT 1;\nSELECT 2;\n");
        assert_eq!(
            first,
            [StreamItem::Statement {
                text: "SELECT 1".into(),
                offset: 0,
                confidence: Confidence::Exact,
            }]
        );
        assert_eq!(
            s.finish(),
            [StreamItem::Statement {
                text: "SELECT 2".into(),
                offset: 10,
                confidence: Confidence::Exact,
            }]
        );
    }
}
