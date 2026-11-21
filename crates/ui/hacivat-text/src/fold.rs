//! Folding (UI-EDIT-10; M3c Task 32): the foldable ranges a language service reports, and the
//! set of folded ranges, mapped through every change and turned into hidden lines for the
//! chunked height map (`display::HeightMap::chunked`).
//!
//! ## m3b-t01b-seams: types (Task 1b). ## m3c-t32-fold: the bodies (Task 32).
//!
//! **Lines, not bytes.** A fold keeps its first line (the *header*) and hides the lines after
//! it up to the line its range ends on; when only blanks stand before the range's end on that
//! line (a block's closing `)` or `END` at the start of its line), that line stays visible too.
//! A range must span at least [`MIN_FOLD_LINES`] lines (NOTES ruling M3c-43) and hide one.
//!
//! **Through edits** ([`FoldSet::map`], then [`FoldSet::prune`] with the new text). Text typed
//! at a fold's start goes above it (the header stays the header) and text typed at its end stays
//! outside it (a block's visible closing line never joins the fold: Enter or a key before its
//! `)` keeps the fold as it is, fix round 1 I2); an edit inside it keeps it. An edit that deletes
//! its header unfolds it: the range's first character (which one edit deleting all of it also
//! does), or the header line's break (joining the next line, deleting the header line). A fold
//! left on fewer than three lines, or hiding nothing, is dropped by `prune`. This replaces
//! UI-EDIT-10 step 4's reconciliation after a reparse. A dropped fold does not come back on
//! undo.
//!
//! **Cost** (fix round 1, I3). Each fold keeps its header and last line. `map` leaves the folds
//! before the change as they are, shifts the ones after it, and maps the few around it in one
//! pass; `prune` recomputes only those the change touched (the others shift by the change's
//! line delta) and checks nesting with a stack; `reveal_selection` binary-searches the sorted
//! caret lines. A keystroke costs O(folds) plain arithmetic and O(touched · log n) text reads.
//!
//! **Nesting.** Folded ranges are disjoint or nested; a range that crosses a folded one's edge
//! is refused. The outer one hides the inner one, which keeps its state for when the outer one
//! opens.

use std::ops::Range;

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeDesc, MapMode};
use crate::selection::SelectionSet;

/// The fewest lines a foldable range spans (NOTES ruling M3c-43: a 2-line comment does not
/// fold).
pub const MIN_FOLD_LINES: u32 = 3;

/// What a foldable range is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldKind {
    /// One statement.
    Statement,
    /// A bracketed or `BEGIN … END` block.
    Block,
    /// A comment spanning lines.
    Comment,
    /// A `-- region` … `-- endregion` pair.
    Region,
}

/// A range the language service says can fold (byte offsets).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FoldRange {
    pub range: Range<usize>,
    pub kind: FoldKind,
}

/// What the last `map` changed, for the next `prune`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Pending {
    /// Every fold's lines are those of the text the set last saw.
    #[default]
    Clean,
    /// One change touched these bytes of the new text.
    Changed(Range<usize>),
    /// More than one change, or an unknown one: `prune` reads every fold.
    Unknown,
}

/// The folded ranges of a document: sorted, each either disjoint from or nested in another,
/// mapped through every change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldSet {
    /// Sorted by start, the outer one first on a shared start.
    folded: Vec<Range<usize>>,
    /// Per fold, the offset of its header line's end (its break), in the text the set was last
    /// pruned against (`map` reads it to see an edit delete the break).
    header_ends: Vec<usize>,
    /// Per fold, its header line and its last hidden line, in that text.
    lines: Vec<(u32, u32)>,
    /// The line count of that text.
    text_lines: usize,
    pending: Pending,
    /// Bumped by every change of the set.
    generation: u64,
}

/// The header line and the last hidden line of `r` (`last == header`: it hides nothing). The
/// ends are moved to char boundaries first.
pub fn fold_lines(buf: &TextBuffer, r: &Range<usize>) -> (u32, u32) {
    let start = buf.snap(r.start, Bias::Before);
    let end = buf.snap(r.end, Bias::Before).max(start);
    let header = buf.byte_to_line(start);
    let mut last = buf.byte_to_line(end);
    if last > header {
        let line_start = buf.line_to_byte(last);
        let blank = buf
            .chunks_in(line_start..end)
            .all(|c| c.chars().all(char::is_whitespace));
        if blank {
            last -= 1;
        }
    }
    (to_u32(header), to_u32(last))
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Whether `r` can fold in `buf`: inside the text on char boundaries, spanning at least
/// [`MIN_FOLD_LINES`] lines and hiding one.
pub fn can_fold(buf: &TextBuffer, r: &Range<usize>) -> bool {
    if r.start >= r.end
        || r.end > buf.len_bytes()
        || !buf.is_char_boundary(r.start)
        || !buf.is_char_boundary(r.end)
    {
        return false;
    }
    let span = buf.byte_to_line(r.end) - buf.byte_to_line(r.start) + 1;
    let (header, last) = fold_lines(buf, r);
    span >= MIN_FOLD_LINES as usize && last > header
}

/// Maps a foldable range through `c` as a fold maps (the module docs): its start biased after,
/// its end before; `None` when the change deleted its first character. The language service
/// maps its answer from an older analysis with this, so a candidate and a fold agree.
pub fn map_fold(c: &ChangeDesc, r: &Range<usize>) -> Option<Range<usize>> {
    let mut m = c.mapper();
    let s = m.map(r.start, Bias::After, MapMode::TrackAfter)?;
    let e = m.map(r.end, Bias::Before, MapMode::Simple)?;
    Some(s..e.max(s))
}

/// The end of `line`'s content (its break's offset).
fn line_end(buf: &TextBuffer, line: u32) -> usize {
    buf.line_range(line as usize, false).end
}

fn crosses(a: &Range<usize>, b: &Range<usize>) -> bool {
    let disjoint = a.end <= b.start || b.end <= a.start;
    let nested = (a.start <= b.start && b.end <= a.end) || (b.start <= a.start && a.end <= b.end);
    !disjoint && !nested
}

fn shift(at: usize, delta: isize) -> usize {
    at.saturating_add_signed(delta)
}

fn shift_line(line: u32, delta: i64) -> u32 {
    u32::try_from((i64::from(line) + delta).max(0)).unwrap_or(u32::MAX)
}

impl FoldSet {
    /// Folds `r`; `true` when the set changed. Refused: a range that cannot fold
    /// ([`can_fold`]: too short, outside the text or off a char boundary), one already folded,
    /// one crossing a folded one's edge.
    pub fn fold(&mut self, r: Range<usize>, buf: &TextBuffer) -> bool {
        if !can_fold(buf, &r) || self.folded.contains(&r) {
            return false;
        }
        if self.folded.iter().any(|f| crosses(f, &r)) {
            return false;
        }
        let at = self
            .folded
            .partition_point(|f| f.start < r.start || (f.start == r.start && f.end > r.end));
        let lines = fold_lines(buf, &r);
        self.header_ends.insert(at, line_end(buf, lines.0));
        self.lines.insert(at, lines);
        self.folded.insert(at, r);
        self.text_lines = buf.len_lines();
        self.generation += 1;
        true
    }

    /// The innermost folded range holding `at` (its ends included).
    pub fn fold_at(&self, at: usize) -> Option<Range<usize>> {
        self.folded
            .iter()
            .filter(|f| f.start <= at && at <= f.end)
            .min_by_key(|f| f.len())
            .cloned()
    }

    /// Unfolds the innermost folded range holding `at` (its ends included); `true` when the set
    /// changed.
    pub fn unfold_at(&mut self, at: usize) -> bool {
        match self
            .fold_at(at)
            .and_then(|r| self.folded.iter().position(|f| *f == r))
        {
            Some(i) => {
                self.remove(i);
                true
            }
            None => false,
        }
    }

    /// Unfolds every fold that hides `at`'s line (a caret, a search hit or "go to line" landed
    /// there, UI-EDIT-10 step 5); `true` when the set changed.
    pub fn reveal(&mut self, at: usize, buf: &TextBuffer) -> bool {
        let line = to_u32(buf.byte_to_line(at.min(buf.len_bytes())));
        self.reveal_line(line, buf)
    }

    /// Unfolds every fold that hides `line`; `true` when the set changed.
    pub fn reveal_line(&mut self, line: u32, buf: &TextBuffer) -> bool {
        let _ = buf;
        self.remove_where(|_, &(header, last)| header < line && line <= last)
    }

    /// Unfolds every fold a selection range's end landed in (UI-EDIT-10 step 5): an end (the
    /// head or the anchor) on a line the fold hides, unless the range holds the fold whole
    /// (select all keeps the folds); `true` when the set changed. O((folds + ends) log ends).
    pub fn reveal_selection(&mut self, sel: &SelectionSet, buf: &TextBuffer) -> bool {
        if self.folded.is_empty() {
            return false;
        }
        let len = buf.len_bytes();
        let ranges = sel.ranges();
        let mut ends: Vec<(u32, usize)> = Vec::with_capacity(ranges.len() * 2);
        for (i, r) in ranges.iter().enumerate() {
            ends.push((to_u32(buf.byte_to_line(r.head.min(len))), i));
            if r.anchor != r.head {
                ends.push((to_u32(buf.byte_to_line(r.anchor.min(len))), i));
            }
        }
        ends.sort_unstable();
        self.remove_where(|f, &(header, last)| {
            let from = ends.partition_point(|e| e.0 <= header);
            ends[from..].iter().take_while(|e| e.0 <= last).any(|e| {
                let r = ranges[e.1].range();
                !(r.start <= f.start && f.end <= r.end)
            })
        })
    }

    /// Removes the folds `hit` names; `true` when any went.
    fn remove_where(&mut self, mut hit: impl FnMut(&Range<usize>, &(u32, u32)) -> bool) -> bool {
        let before = self.folded.len();
        let mut i = 0;
        while i < self.folded.len() {
            if hit(&self.folded[i], &self.lines[i]) {
                self.remove(i);
            } else {
                i += 1;
            }
        }
        self.folded.len() != before
    }

    /// The outermost folded range whose header is `line`.
    pub fn fold_on_line(&self, line: u32, buf: &TextBuffer) -> Option<Range<usize>> {
        let _ = buf;
        let i = self.lines.iter().position(|l| l.0 == line)?;
        Some(self.folded[i].clone())
    }

    /// The header and last hidden line of the folded range `r`.
    pub fn lines_of(&self, r: &Range<usize>) -> Option<(u32, u32)> {
        let i = self.folded.iter().position(|f| f == r)?;
        Some(self.lines[i])
    }

    /// Each fold's header and last hidden line, in the order of [`FoldSet::folded`].
    pub fn lines(&self) -> &[(u32, u32)] {
        &self.lines
    }

    /// Unfolds exactly the folded range `r`; `true` when it was folded.
    pub fn unfold_range(&mut self, r: &Range<usize>) -> bool {
        match self.folded.iter().position(|f| f == r) {
            Some(i) => {
                self.remove(i);
                true
            }
            None => false,
        }
    }

    /// Unfolds the outermost fold whose header is `line`; `true` when the set changed.
    pub fn unfold_line(&mut self, line: u32, buf: &TextBuffer) -> bool {
        let _ = buf;
        match self.lines.iter().position(|l| l.0 == line) {
            Some(i) => {
                self.remove(i);
                true
            }
            None => false,
        }
    }

    fn remove(&mut self, i: usize) {
        self.folded.remove(i);
        self.header_ends.remove(i);
        self.lines.remove(i);
        self.generation += 1;
    }

    /// Unfolds everything; `true` when anything was folded.
    pub fn unfold_all(&mut self) -> bool {
        let had = !self.folded.is_empty();
        self.folded.clear();
        self.header_ends.clear();
        self.lines.clear();
        if had {
            self.generation += 1;
        }
        had
    }

    /// Maps the folded ranges through `c` (the module docs): the folds before the change stay,
    /// the ones after it shift, the ones around it map in one pass; dropped when the change
    /// deleted a range's first character or its header line's break. [`FoldSet::prune`] with the
    /// new text follows.
    pub fn map(&mut self, c: &ChangeDesc) {
        let (Some(old), Some(new)) = (c.changed_old_range(), c.changed_range()) else {
            return;
        };
        self.pending = match std::mem::take(&mut self.pending) {
            Pending::Clean => Pending::Changed(new),
            _ => Pending::Unknown,
        };
        if self.folded.is_empty() {
            return;
        }
        let delta = c.len_new() as isize - c.len_old() as isize;
        // The folds the change reaches into: their positions, sorted, mapped in one pass.
        let mut queries: Vec<(usize, u8, usize)> = Vec::new();
        for (i, r) in self.folded.iter().enumerate() {
            if r.end <= old.start || r.start >= old.end {
                continue;
            }
            queries.push((r.start, 0, i));
            queries.push((self.header_ends[i], 1, i));
            queries.push((r.end, 2, i));
        }
        queries.sort_unstable();
        let mut mapped: Vec<[Option<usize>; 3]> = vec![[None; 3]; self.folded.len()];
        let mut m = c.mapper();
        for &(pos, slot, i) in &queries {
            let at = match slot {
                0 | 1 => m.map(pos, Bias::After, MapMode::TrackAfter),
                _ => m.map(pos, Bias::Before, MapMode::Simple),
            };
            mapped[i][usize::from(slot)] = at.or(Some(usize::MAX));
        }
        let mut keep = vec![true; self.folded.len()];
        for i in 0..self.folded.len() {
            let r = &mut self.folded[i];
            if r.end <= old.start {
                continue;
            }
            if r.start >= old.end {
                *r = shift(r.start, delta)..shift(r.end, delta);
                self.header_ends[i] = shift(self.header_ends[i], delta);
                continue;
            }
            let [s, he, e] = mapped[i];
            let (Some(s), Some(he), Some(e)) = (s, he, e) else {
                keep[i] = false;
                continue;
            };
            let header_kept = self.header_ends[i] >= r.end || he != usize::MAX;
            if s == usize::MAX || !header_kept {
                keep[i] = false;
                continue;
            }
            *r = s..e.max(s);
            self.header_ends[i] = if he == usize::MAX { s } else { he };
        }
        if keep.iter().any(|k| !k) {
            let mut k = keep.iter();
            self.folded.retain(|_| *k.next().unwrap_or(&true));
            let mut k = keep.iter();
            self.header_ends.retain(|_| *k.next().unwrap_or(&true));
            let mut k = keep.iter();
            self.lines.retain(|_| *k.next().unwrap_or(&true));
        }
        self.generation += 1;
    }

    /// After a change: drops the folds that cannot fold any more (fewer than three lines,
    /// nothing hidden, a range two folds now share, one crossing another) and records each
    /// fold's header line end and lines in `buf`. Only the folds the last `map`'s change reached
    /// are read from `buf`; the ones after it shift by its line delta.
    pub fn prune(&mut self, buf: &TextBuffer) {
        let changed = match std::mem::take(&mut self.pending) {
            Pending::Changed(r) => Some(r),
            Pending::Clean | Pending::Unknown => None,
        };
        let line_delta = buf.len_lines() as i64 - self.text_lines as i64;
        self.text_lines = buf.len_lines();
        let n = self.folded.len();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let (x, y) = (&self.folded[a], &self.folded[b]);
            x.start.cmp(&y.start).then(y.end.cmp(&x.end))
        });
        let mut folded = Vec::with_capacity(n);
        let mut ends = Vec::with_capacity(n);
        let mut lines = Vec::with_capacity(n);
        // The kept ranges still open at the current start (outer first).
        let mut open: Vec<Range<usize>> = Vec::new();
        for i in order {
            let r = self.folded[i].clone();
            while open.last().is_some_and(|top| top.end <= r.start) {
                open.pop();
            }
            if open.last().is_some_and(|top| *top == r || r.end > top.end) {
                continue; // a duplicate, or it crosses the enclosing fold's end
            }
            let (he, l) = match &changed {
                Some(c) if r.end < c.start => (self.header_ends[i], self.lines[i]),
                Some(c) if r.start > c.end => {
                    let (h, last) = self.lines[i];
                    (
                        self.header_ends[i],
                        (shift_line(h, line_delta), shift_line(last, line_delta)),
                    )
                }
                _ => {
                    if !can_fold(buf, &r) {
                        continue;
                    }
                    let l = fold_lines(buf, &r);
                    (line_end(buf, l.0), l)
                }
            };
            open.push(r.clone());
            folded.push(r);
            ends.push(he);
            lines.push(l);
        }
        if folded.len() != n {
            self.generation += 1;
        }
        self.folded = folded;
        self.header_ends = ends;
        self.lines = lines;
    }

    /// The folded ranges, sorted by start (the outer one first on a shared start).
    pub fn folded(&self) -> &[Range<usize>] {
        &self.folded
    }

    /// Bumped by every change of the set: the editor caches the hidden lines by it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The lines the folds hide (each fold keeps its header), sorted and merged, in the text
    /// the set was last pruned or folded against.
    pub fn hidden_lines(&self, buf: &TextBuffer) -> Vec<Range<u32>> {
        let _ = buf;
        let mut out: Vec<Range<u32>> = Vec::with_capacity(self.folded.len());
        for &(header, last) in &self.lines {
            if last <= header {
                continue;
            }
            let run = header + 1..last + 1;
            match out.last_mut() {
                Some(prev) if run.start <= prev.end => prev.end = prev.end.max(run.end),
                _ => out.push(run),
            }
        }
        out
    }
}
