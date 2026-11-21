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
//! All of the above is the default policy, [`StreamPolicy::Exact`].
//!
//! [`StreamPolicy::Eager`] is for M4b "Run as script" (R-M4b-46, R-M4b-47; ADR 0005
//! `m4b-t08`), which must run in constant memory:
//! - on MySQL/MariaDB, statements are given out without the block hold, and the stream stops
//!   at the first block mismatch with one `UnknownBlock` item instead of refusing back;
//! - an anchor is looked for at every line start, after a lone `\r` too;
//! - at most [`STREAM_HELD_MAX`] bytes and [`STREAM_HELD_ITEMS_MAX`] unsettled items are held:
//!   past either the stream stops with `TooLong` or `NoBoundary`.
//!
//! Every eager stream either gives out an item that refuses ([`StreamItem::refuses`]) or
//! equals the default stream, which is the whole split. R-M4b-47's first pass relies on it:
//! "Run as script" streams the file once without running anything, refuses the file if any
//! item refuses, and only then streams it again to run it.

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
    // ## m4b-t08-stream-hold
    /// Under [`StreamPolicy::Eager`]: [`STREAM_HELD_MAX`] bytes (or [`STREAM_HELD_ITEMS_MAX`]
    /// items) from `offset` hold no place where the stream can settle (statements with no line
    /// start between them, an open comment, comments only). The stream stops here.
    NoBoundary {
        offset: u64,
    },
    // ## end m4b-t08-stream-hold
}

// ## m4b-t08-stream-hold
/// The most text the eager stream holds (16 MiB). One statement may come close to it:
/// mysqldump writes an extended `INSERT` no longer than its `--net-buffer-length` (about 1 MiB
/// by default), so a dump stays far below it. A pass over one long statement costs about 4× its
/// size, so the cap costs well under 100 MB whatever the file's size.
pub const STREAM_HELD_MAX: usize = 16 << 20;

/// The most items the eager stream holds unsettled (65,536). A pass costs a few hundred bytes
/// per item: 16 MiB of short statements on one line cost about 600 MB (measured), so their
/// count is capped too, at about 25 MB.
pub const STREAM_HELD_ITEMS_MAX: usize = 1 << 16;

impl StreamItem {
    /// Whether the item refuses the script (R-M4b-47's first pass refuses the file when any
    /// item does): an uncertain boundary (`Ambiguous`, which M3's whole split refuses), a
    /// statement too long to hold (`TooLong`), held text with no boundary (`NoBoundary`), or a
    /// statement that is not `Exact` (never without an `Ambiguous`; checked to fail closed).
    /// A client command does not refuse: the runner reports it.
    pub fn refuses(&self) -> bool {
        match self {
            StreamItem::Statement { confidence, .. } => *confidence != Confidence::Exact,
            StreamItem::Client { .. } => false,
            StreamItem::Ambiguous { .. }
            | StreamItem::TooLong { .. }
            | StreamItem::NoBoundary { .. } => true,
        }
    }
}

/// Whether any of `items` refuses ([`StreamItem::refuses`]).
pub fn has_stop(items: &[StreamItem]) -> bool {
    items.iter().any(StreamItem::refuses)
}

/// What the stream may give out before the end of the script (M4b Task 8; R-M4b-46,
/// R-M4b-47).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum StreamPolicy {
    /// The stream equals the whole script's execution split for every chunking (module
    /// documentation); on MySQL/MariaDB it holds everything after the last block-tracked
    /// statement.
    #[default]
    Exact,
    /// Constant memory: no block hold on MySQL/MariaDB, where the stream stops at the first
    /// block mismatch; anchors at every line start; at most [`STREAM_HELD_MAX`] bytes held.
    /// The stream either gives out an item that refuses or equals the whole split.
    Eager,
}
// ## end m4b-t08-stream-hold

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

    /// The bytes held, not given out yet (the memory tests).
    #[doc(hidden)]
    pub fn held_len(&self) -> usize {
        self.held.len()
    }

    /// Whether the stream has stopped: an item that refuses ([`StreamItem::refuses`]) ended it,
    /// and nothing more is given out.
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    // ## end m4b-t08-stream-hold

    /// The items `chunk` settles (possibly none: they come once later text can no longer
    /// change them, at the latest at [`StreamSplitter::finish`]).
    pub fn feed(&mut self, chunk: &str) -> Vec<StreamItem> {
        if self.stopped {
            return Vec::new();
        }
        // ## m4b-t08-stream-hold: the eager policy has its own path; the rest is `Exact`.
        if self.policy == StreamPolicy::Eager {
            return self.feed_eager(chunk);
        }
        // ## end m4b-t08-stream-hold
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
        // ## m4b-t08-stream-hold: `release` is `give_out` under `Exact`.
        self.release(&a, n.saturating_add(1), &mut out);
        // ## end m4b-t08-stream-hold
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

    // ## m4b-t08-stream-hold
    /// Whether the block rules of the eager policy apply (MySQL/MariaDB under `Eager`).
    fn eager_blocks(&self) -> bool {
        self.policy == StreamPolicy::Eager && split_profile(&self.ctx).family == BlockFamily::MySql
    }

    /// The longest statement given out as a statement (a longer one is `TooLong`).
    fn statement_max(&self) -> usize {
        match self.policy {
            StreamPolicy::Exact => STREAM_STATEMENT_MAX,
            StreamPolicy::Eager => STREAM_HELD_MAX,
        }
    }

    /// [`Self::feed`] under [`StreamPolicy::Eager`]: no block hold on MySQL/MariaDB, an anchor
    /// at any line start (after a lone `\r` too), and at most [`STREAM_HELD_MAX`] bytes held
    /// once a feed returns: past it, or past [`STREAM_HELD_ITEMS_MAX`] unsettled items, the
    /// stream stops ([`Self::stop_at_cap`]).
    fn feed_eager(&mut self, chunk: &str) -> Vec<StreamItem> {
        self.held.push_str(chunk);
        let n = self.held.len();
        let capped = n > STREAM_HELD_MAX;
        if n < self.tried.saturating_mul(2) && !capped {
            return Vec::new();
        }
        let a = self.analysis();
        let mut out = Vec::new();
        if let Some(x) = self.settled_eager(&a) {
            self.release(&a, x, &mut out);
            self.tried = 0;
            if !self.stopped && self.held.len() > STREAM_HELD_MAX {
                // What is left after the anchor is over the cap on its own.
                let rest = self.analysis();
                self.stop_at_cap(&rest, &mut out);
            }
            return out;
        }
        self.tried = n;
        if capped || a.items.len() > STREAM_HELD_ITEMS_MAX {
            self.stop_at_cap(&a, &mut out);
        }
        out
    }

    /// The held text is over [`STREAM_HELD_MAX`] (or [`STREAM_HELD_ITEMS_MAX`] items) with
    /// nothing settled, and the stream stops:
    /// - a statement still open past the cap: what precedes it is given out, then `TooLong` at
    ///   its start (as under `Exact` at [`STREAM_STATEMENT_MAX`]);
    /// - anything else (statements with no line start between them, an open comment, comments
    ///   only): `NoBoundary` at the held text's start, and nothing of it is given out.
    fn stop_at_cap(&mut self, a: &Analysis, out: &mut Vec<StreamItem>) {
        let n = self.held.len();
        if let Some(RawItem::Stmt(s)) = a.items.last()
            && s.terminator.is_none()
            && n - usize::from(s.strict.start()) > STREAM_HELD_MAX
        {
            let at = usize::from(s.strict.start());
            self.release(a, at, out);
            if !self.stopped {
                out.push(StreamItem::TooLong { offset: self.base });
            }
        } else {
            out.push(StreamItem::NoBoundary { offset: self.base });
        }
        self.stopped = true;
        self.held = String::new();
    }

    /// [`Self::settled`] under [`StreamPolicy::Eager`]: the latest item that starts a line,
    /// among all the items held (not only the last `ANCHOR_SCAN`), with no MySQL block limit.
    /// A line start is found by walking back over blanks to a `\n` (then [`anchor`] decides) or
    /// to a lone `\r` ([`Self::cr_anchor`]), so the scan is linear in the items.
    fn settled_eager(&self, a: &Analysis) -> Option<usize> {
        let text = &self.held;
        let b = text.as_bytes();
        let cp = client_profile(self.flavor);
        let ambiguous = a
            .ambiguous
            .iter()
            .find(|x| x.reason == AmbiguityReason::AmbiguousCommentEnd)
            .map_or(usize::MAX, |x| usize::from(x.at.start()));
        for j in (1..a.items.len()).rev() {
            let mut q = usize::from(item_start(&a.items[j]));
            while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
                q -= 1;
            }
            let x = match q.checked_sub(1).map(|p| b[p]) {
                Some(b'\n') => anchor(text, a, j),
                Some(b'\r') => self.cr_anchor(a, j, q),
                _ => None,
            };
            if let Some(x) = x
                && x <= ambiguous
                && if_depth(text, cp, &a.client, x) == Some(0)
            {
                return Some(x);
            }
        }
        None
    }

    /// An anchor at `line`, a line that a lone `\r` (classic Mac line ends) starts before item
    /// `j`. The pass reads only `\n` as a line break for client commands (a `--` or `#` comment
    /// ends at either), so a pass that starts at `line` sees a line start that the whole text
    /// does not have. `line` is an anchor only when:
    /// - nothing the pass read runs past the break (only white space since the last item,
    ///   client command or `COPY` data), and no byte-order mark starts the line;
    /// - item `j` is not the last item (which may still grow);
    /// - the pass from `line` gives exactly the items, commands and boundaries the pass over
    ///   the held text gives from `line` up to the last item, which it starts at the same place
    ///   and with the same delimiter.
    fn cr_anchor(&self, a: &Analysis, j: usize, line: usize) -> Option<usize> {
        let text = &self.held;
        let b = text.as_bytes();
        let last = a.items.len().checked_sub(1)?;
        if j >= last || b[line..].starts_with(BOM) {
            return None;
        }
        let client = a
            .client
            .iter()
            .filter(|c| usize::from(c.range.start()) < line)
            .map(|c| usize::from(c.range.end()))
            .max()
            .unwrap_or(0);
        let copy = a
            .copies
            .iter()
            .filter(|(_, d)| usize::from(d.rows.start()) < line)
            .map(|(_, d)| usize::from(d.end.map_or(d.rows.end(), |e| e.end())))
            .max()
            .unwrap_or(0);
        let read = usize::from(item_end(&a.items[j - 1])).max(client).max(copy);
        if read > line || !b[read..line].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        let upto = usize::from(item_start(&a.items[last]));
        let start = SplitStart {
            lex: LexState::Initial,
            delimiter: a.delimiter_at(size(line)),
        };
        let w = analyze(&text[line..], None, &self.ctx, self.flavor, &start);
        let max = self.statement_max();
        let whole = collect(a, text, 0, line, upto, max);
        let window = collect(&w, &text[line..], line as u64, 0, upto - line, max);
        let same = whole == window
            && w.items
                .iter()
                .any(|it| usize::from(item_start(it)) == upto - line)
            && w.delimiter_at(size(upto - line)) == a.delimiter_at(size(upto));
        same.then_some(line)
    }

    /// Gives out the items of `a` before `upto` as the policy allows: under
    /// [`StreamPolicy::Exact`] all of them ([`Self::give_out`]).
    ///
    /// Under [`StreamPolicy::Eager`] a `TooLong` statement stops the stream after it. On
    /// MySQL/MariaDB a block mismatch (`UnknownBlock`) may also refuse statements already given
    /// out (the stray-`END` backstop refuses back to the last block-tracked statement, which is
    /// no longer held). So when `a` has one, the text before `upto` is split on its own: without
    /// one there (the mismatch lies in the text still held, and its refusal of earlier
    /// statements is the backstop's), its items are given out as if the mismatch were not there
    /// yet; with one, the stream stops at the first statement that brings one
    /// ([`Self::stop_at_mismatch`]).
    fn release(&mut self, a: &Analysis, upto: usize, out: &mut Vec<StreamItem>) {
        if self.policy == StreamPolicy::Exact {
            self.give_out(a, upto, out);
            return;
        }
        let from = out.len();
        if !self.eager_blocks() || !mismatched(a) {
            self.give_out(a, upto, out);
        } else {
            let end = upto.min(self.held.len());
            let prefix = (end < self.held.len()).then(|| self.prefix_analysis(end));
            let p = prefix.as_ref().unwrap_or(a);
            if mismatched(p) {
                self.stop_at_mismatch(p, end, out);
            } else {
                self.give_out(p, upto, out);
            }
        }
        if let Some(i) = out[from..]
            .iter()
            .position(|it| matches!(it, StreamItem::TooLong { .. }))
        {
            out.truncate(from + i + 1);
            self.stopped = true;
            self.held = String::new();
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
    // ## end m4b-t08-stream-hold

    /// Gives out every item of `a` that starts before `upto` and drops the text before it.
    fn give_out(&mut self, a: &Analysis, upto: usize, out: &mut Vec<StreamItem>) {
        // ## m4b-t08-stream-hold: the item list is `collect`'s (shared with `cr_anchor`).
        out.extend(collect(
            a,
            &self.held,
            self.base,
            0,
            upto,
            self.statement_max(),
        ));
        // ## end m4b-t08-stream-hold
        let cut = upto.min(self.held.len());
        self.delimiter = a.delimiter_at(size(cut));
        self.held.drain(..cut);
        self.base += cut as u64;
    }
}

/// The items of `a` (the pass over `text`, which starts at stream offset `base`) that start in
/// `from..upto`, in text order: by offset, then a client command before a statement before an
/// uncertain boundary. A statement longer than `max` is `TooLong`.
fn collect(
    a: &Analysis,
    text: &str,
    base: u64,
    from: usize,
    upto: usize,
    max: usize,
) -> Vec<StreamItem> {
    let off = |p: TextSize| base + u64::from(u32::from(p));
    let within = |p: TextSize| (from..upto).contains(&usize::from(p));
    let split = a.to_split(text, SplitMode::Execution);
    let mut items: Vec<(u64, u8, StreamItem)> = Vec::new();
    for c in split.client.iter().filter(|c| within(c.range.start())) {
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
        .filter(|s| within(s.strict.start()))
    {
        let offset = off(s.strict.start());
        let item = if usize::from(s.strict.len()) > max {
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
    for x in split.ambiguous.iter().filter(|x| within(x.at.start())) {
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
    items.into_iter().map(|(_, _, it)| it).collect()
}

// ## m4b-t08-stream-hold
/// A byte-order mark: a line that starts with one is never an anchor (as `incremental::anchor`).
const BOM: &[u8] = b"\xEF\xBB\xBF";

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

/// Where item `it` ends: its smart range, terminator and trailing comments included.
fn item_end(it: &RawItem) -> TextSize {
    match it {
        RawItem::Stmt(s) => s
            .smart_end
            .max(s.strict.end())
            .max(s.terminator.map_or(s.strict.end(), |t| t.end())),
        RawItem::Empty(r) => r.end(),
    }
}
// ## end m4b-t08-stream-hold

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
