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
//! [`StreamPolicy::Eager`] is for M4b "Run as script" (R-M4b-46, R-M4b-47, R-T8-4; ADR 0005
//! `m4b-t08`), which must run in constant memory. It differs from `Exact` on MySQL/MariaDB
//! only; on every other dialect it is `Exact`, code path and items alike. On MySQL/MariaDB:
//! - statements are given out without the block hold, and the stream stops at the first block
//!   mismatch with one `UnknownBlock` item instead of refusing back;
//! - an anchor is looked for at every line start after a `\n` (never after a lone `\r`);
//! - two consecutive anchors may lie at most [`STREAM_HELD_MAX`] bytes and
//!   [`STREAM_HELD_ITEMS_MAX`] items apart (so about that much is held at most): past either
//!   the stream stops with `NoBoundary`. The rule reads the text alone.
//!
//! Every eager stream either gives out an item that refuses ([`StreamItem::refuses`]) or
//! equals the default stream, which is the whole split. R-M4b-47's first pass relies on it:
//! "Run as script" streams the file once without running anything, refuses the file if any
//! item refuses ([`any_refuses`]), and only then streams it again, in the same chunks, to run
//! it.

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
    /// Under [`StreamPolicy::Eager`] on MySQL/MariaDB: the next anchor after `offset` lies more than
    /// [`STREAM_HELD_MAX`] bytes (or [`STREAM_HELD_ITEMS_MAX`] items) away (statements with no
    /// line start between them, an open comment, comments only, a statement longer than the
    /// cap). The stream stops here.
    NoBoundary {
        offset: u64,
    },
    // ## end m4b-t08-stream-hold
}

// ## m4b-t08-stream-hold
/// The byte cap of the eager stream: two consecutive anchors (statement boundaries at a line
/// start) may lie at most this far apart, so this much text is held at most (R-T8-2: the same
/// 64 MiB as [`STREAM_STATEMENT_MAX`], so every statement `Exact` runs fits, such as a large
/// mysqldump `--hex-blob` row).
pub const STREAM_HELD_MAX: usize = STREAM_STATEMENT_MAX;

/// The item cap of the eager stream: two consecutive anchors may hold at most this many items
/// (65,536). A pass costs a few hundred bytes per item, so 64 MiB of short statements on one
/// line would cost gigabytes; the item cap keeps that to about 25 MB.
pub const STREAM_HELD_ITEMS_MAX: usize = 1 << 16;

/// How long the last item must be before its line start counts as an anchor whose status is
/// final: its first token can no longer become a comment (the splitter's own cut margin).
const ESTABLISHED: usize = 16;

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

/// Whether any of `items` refuses ([`StreamItem::refuses`]): what R-M4b-47's first pass checks
/// on every `feed` and on `finish`. A refusing item does not always stop the stream (an
/// `Ambiguous` boundary mid-file, a `Heuristic` statement), so [`StreamSplitter::stopped`] is
/// not enough.
pub fn any_refuses(items: &[StreamItem]) -> bool {
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
    /// Constant memory on MySQL/MariaDB (module documentation): no block hold, a stop at the
    /// first block mismatch, anchors at every line start after a `\n`, consecutive anchors at
    /// most [`STREAM_HELD_MAX`] bytes and [`STREAM_HELD_ITEMS_MAX`] items apart. The stream
    /// either gives out an item that refuses or equals the whole split. On every other dialect
    /// it is `Exact`.
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
    /// The eager caps ([`STREAM_HELD_MAX`], [`STREAM_HELD_ITEMS_MAX`]; smaller in tests).
    held_max: usize,
    items_max: usize,
    // ## end m4b-t08-stream-hold
}

// ## m4b-t08-stream-hold
/// What the eager cap rule decides on a pass ([`StreamSplitter::eager_check`]).
enum Check {
    /// A gap breaks a cap: stop at this anchor.
    Stop { at: usize },
    /// Cut at the latest anchor, if any.
    Cut(Option<usize>),
}
// ## end m4b-t08-stream-hold

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
            held_max: STREAM_HELD_MAX,
            items_max: STREAM_HELD_ITEMS_MAX,
        }
    }

    /// The splitter with other eager caps (tests reach the cap rule with small texts; never in
    /// product code: the feature is the test suite's alone).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn with_caps(mut self, held_max: usize, items_max: usize) -> Self {
        self.held_max = held_max.max(1);
        self.items_max = items_max.max(1);
        self
    }

    /// The bytes held, not given out yet (the memory tests).
    #[doc(hidden)]
    pub fn held_len(&self) -> usize {
        self.held.len()
    }

    /// Whether the stream has stopped: an item that refuses ([`StreamItem::refuses`]) ended it,
    /// and nothing more is given out. Not enough for R-M4b-47's first pass: an `Ambiguous`
    /// boundary mid-file refuses without stopping; check [`any_refuses`] on every batch.
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
        // ## m4b-t08-stream-hold: the eager policy on MySQL/MariaDB has its own path; the rest
        // (every other dialect under either policy) is `Exact`.
        if self.eager_blocks() {
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
        // ## m4b-t08-stream-hold: the eager policy on MySQL/MariaDB has its own path.
        if self.eager_blocks() {
            return self.finish_eager();
        }
        // ## end m4b-t08-stream-hold
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

    // ## m4b-t08-stream-hold
    /// Whether the eager path applies (MySQL/MariaDB under `Eager`; R-T8-4).
    fn eager_blocks(&self) -> bool {
        self.policy == StreamPolicy::Eager && split_profile(&self.ctx).family == BlockFamily::MySql
    }

    /// [`Self::feed`] under [`StreamPolicy::Eager`] (see [`Self::eager_check`]). The held text
    /// is split again when it has doubled since a split that gave out nothing, and, past the
    /// byte cap, every sixteenth of the cap.
    fn feed_eager(&mut self, chunk: &str) -> Vec<StreamItem> {
        self.held.push_str(chunk);
        let n = self.held.len();
        let step = (self.held_max / 16).max(1);
        let due = n >= self.tried.saturating_mul(2)
            || (n > self.held_max && n >= self.tried.saturating_add(step));
        if !due {
            return Vec::new();
        }
        let a = self.analysis();
        let mut out = Vec::new();
        match self.eager_check(&a, false) {
            Check::Stop { at } => self.stop_at_gap(&a, at, &mut out),
            Check::Cut(Some(x)) => {
                self.release(&a, x, &mut out);
                self.tried = 0;
            }
            Check::Cut(None) => self.tried = n,
        }
        out
    }

    /// [`Self::finish`] under [`StreamPolicy::Eager`]: the end of the text is the last anchor.
    fn finish_eager(&mut self) -> Vec<StreamItem> {
        let a = self.analysis();
        let mut out = Vec::new();
        match self.eager_check(&a, true) {
            Check::Stop { at } => self.stop_at_gap(&a, at, &mut out),
            Check::Cut(_) => {
                let n = self.held.len();
                self.release(&a, n.saturating_add(1), &mut out);
            }
        }
        out
    }

    /// The cap rule of [`StreamPolicy::Eager`], a function of the text alone (R-T8-1): the
    /// stream refuses where two consecutive anchors (the stream's start, the anchors, the end
    /// of the text) are more than the byte cap apart or hold more than the item cap of items,
    /// counting text already given out. It stops at the first anchor of the first such gap.
    ///
    /// Only anchors whose status no later text can change count ([`Self::anchor_status`]). A
    /// gap is decided once the text seen proves it, by a bound on where the next anchor can
    /// lie: at the last item's line if that item is still pending, or else no earlier than
    /// [`Self::span`] (plus a token's margin) before the end of the text held, since an
    /// anchor's item starts within that span of its line. Otherwise the stream cuts at the
    /// latest anchor (or holds on).
    fn eager_check(&self, a: &Analysis, at_end: bool) -> Check {
        let (anchors, pending) = self.eager_anchors(a, at_end);
        let n = self.held.len();
        let mut prev = (0usize, 0usize);
        for &(x, j) in &anchors {
            if x - prev.0 > self.held_max || j - prev.1 > self.items_max {
                return Check::Stop { at: prev.0 };
            }
            prev = (x, j);
        }
        // The last item may still change, unless the text has ended.
        let settled_items = if at_end {
            a.items.len()
        } else {
            a.items.len().saturating_sub(1)
        };
        let next = if at_end {
            n
        } else {
            pending.unwrap_or_else(|| n.saturating_sub(self.span() + ESTABLISHED).max(prev.0))
        };
        if settled_items.saturating_sub(prev.1) > self.items_max || next - prev.0 > self.held_max {
            return Check::Stop { at: prev.0 };
        }
        Check::Cut(anchors.last().map(|(x, _)| *x))
    }

    /// How far an anchor's item may start from its line (after leading comments and blanks)
    /// before the line is no anchor: a sixteenth of the byte cap, 4 MiB. It bounds where a
    /// later anchor can lie, so a gap is decided without waiting for the end of the text.
    fn span(&self) -> usize {
        (self.held_max / 16).max(ESTABLISHED)
    }

    /// The anchors of `a` whose status is final, in text order, as (line start, item index),
    /// and where the last item's line starts if its status is still pending. Each item's
    /// status comes from [`Self::anchor_status`], the one rule for streaming and the end alike.
    fn eager_anchors(&self, a: &Analysis, at_end: bool) -> (Vec<(usize, usize)>, Option<usize>) {
        let (mut anchors, mut pending) = (Vec::new(), None);
        for j in 1..a.items.len() {
            match self.anchor_status(a, j, at_end) {
                Status::Anchor(x) => anchors.push((x, j)),
                Status::Pending(q) => pending = Some(q),
                Status::Not => {}
            }
        }
        (anchors, pending)
    }

    /// Whether item `j`'s line is an anchor (R-T8-4: after a `\n` only, never after a lone
    /// `\r`):
    /// - only blanks stand between the item and a `\n`, and [`anchor`] accepts the line (the
    ///   text before the item is read to its end; no byte-order mark starts the line);
    /// - the item's strict start lies within [`Self::span`] of the line;
    /// - the line lies before any executable comment with an ambiguous end, outside a psql
    ///   `\if`.
    ///
    /// All of it reads the text before the item's strict start. The one thing later text can
    /// change is whether the last item is an item at all (its first token may still become a
    /// comment), so while the text goes on, an unterminated last item shorter than 16 bytes is
    /// `Pending`; at the end, and in every other case, the same rule gives the same answer.
    fn anchor_status(&self, a: &Analysis, j: usize, at_end: bool) -> Status {
        let text = &self.held;
        let b = text.as_bytes();
        let mut q = usize::from(item_start(&a.items[j]));
        while q > 0 && matches!(b[q - 1], b' ' | b'\t') {
            q -= 1;
        }
        let start = match &a.items[j] {
            RawItem::Stmt(s) => usize::from(s.strict.start()),
            RawItem::Empty(r) => usize::from(r.start()),
        };
        if q == 0 || b[q - 1] != b'\n' || start - q > self.span() {
            return Status::Not;
        }
        let open = !at_end
            && j + 1 == a.items.len()
            && matches!(&a.items[j], RawItem::Stmt(s) if s.terminator.is_none());
        if open && text.len() - start < ESTABLISHED {
            return Status::Pending(q);
        }
        let ambiguous = a
            .ambiguous
            .iter()
            .find(|x| x.reason == AmbiguityReason::AmbiguousCommentEnd)
            .map_or(usize::MAX, |x| usize::from(x.at.start()));
        let cp = client_profile(self.flavor);
        match anchor(text, a, j) {
            Some(x) if x <= ambiguous && if_depth(text, cp, &a.client, x) == Some(0) => {
                Status::Anchor(x)
            }
            _ => Status::Not,
        }
    }

    /// The stream stops at `at`, an anchor of `a`, where a gap breaks a cap: what precedes
    /// `at` is given out (a block mismatch there stops the stream first), then `NoBoundary` at
    /// `at`. A statement longer than the cap is such a gap too; it is not told apart as
    /// `TooLong`, which would need its end, and so the chunking.
    fn stop_at_gap(&mut self, a: &Analysis, at: usize, out: &mut Vec<StreamItem>) {
        self.release(a, at, out);
        if !self.stopped {
            out.push(StreamItem::NoBoundary { offset: self.base });
        }
        self.stopped = true;
        self.held = String::new();
    }

    /// Gives out the items of `a` before `upto` as the policy allows: under
    /// [`StreamPolicy::Exact`] all of them ([`Self::give_out`]).
    ///
    /// Under [`StreamPolicy::Eager`] (MySQL/MariaDB) a `TooLong` statement stops the stream
    /// after it, and a block mismatch (`UnknownBlock`) may also refuse statements already given
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
/// What [`StreamSplitter::anchor_status`] says of an item's line.
enum Status {
    /// The line is an anchor, at this offset.
    Anchor(usize),
    /// The line is no anchor, whatever text comes later.
    Not,
    /// The last item may still turn out not to be an item: its line, if it is one.
    Pending(usize),
}

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
