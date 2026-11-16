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
}

impl StreamSplitter {
    pub fn new(ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        StreamSplitter {
            ctx,
            flavor,
            held: String::new(),
            base: 0,
            delimiter: None,
            tried: 0,
            stopped: false,
        }
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
            self.give_out(&a, x, &mut out);
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
            self.give_out(&a, at, &mut out);
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
        self.give_out(&a, n.saturating_add(1), &mut out);
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
        if split_profile(&self.ctx).family == BlockFamily::MySql {
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
