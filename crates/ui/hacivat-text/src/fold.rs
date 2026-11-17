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
//! **Through edits** ([`FoldSet::map`], then [`FoldSet::prune`] with the new text). A folded
//! range maps as a container (`RangeKind::Container`): an edit inside it or at its edges keeps
//! it, and it goes when one edit deletes all of it. Text typed at its start goes above it (the
//! header stays the header); text typed at its end joins it. An edit that
//! deletes its header unfolds it: the range's first character, or the header line's break
//! (joining the header with the next line, or deleting the header line). A fold left on fewer
//! than three lines, or hiding nothing, is dropped by `prune`. This replaces UI-EDIT-10 step 4's
//! reconciliation after a reparse: a fold whose opening token is deleted goes at once, not at
//! the next edit inside it.
//!
//! **Nesting.** Folded ranges are disjoint or nested; a range that crosses a folded one's edge
//! is refused. The outer one hides the inner one, which keeps its state for when the outer one
//! opens.

use std::ops::Range;

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeDesc, MapMode, RangeKind};
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

/// The folded ranges of a document: sorted, each either disjoint from or nested in another,
/// mapped through every change.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoldSet {
    /// Sorted by start, the outer one first on a shared start.
    folded: Vec<Range<usize>>,
    /// Per fold, the offset of its header line's end (its break), in the text the set was last
    /// pruned against (`map` reads it to see an edit delete the break).
    header_ends: Vec<usize>,
    /// Bumped by every change of the set.
    generation: u64,
}

/// The header line and the last hidden line of `r` (`last == header`: it hides nothing).
pub fn fold_lines(buf: &TextBuffer, r: &Range<usize>) -> (u32, u32) {
    let len = buf.len_bytes();
    let (start, end) = (r.start.min(len), r.end.min(len).max(r.start.min(len)));
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

/// Whether `r` can fold in `buf`: it spans at least [`MIN_FOLD_LINES`] lines and hides one.
pub fn can_fold(buf: &TextBuffer, r: &Range<usize>) -> bool {
    if r.start >= r.end || r.end > buf.len_bytes() {
        return false;
    }
    let span = buf.byte_to_line(r.end) - buf.byte_to_line(r.start) + 1;
    let (header, last) = fold_lines(buf, r);
    span >= MIN_FOLD_LINES as usize && last > header
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

impl FoldSet {
    /// Folds `r` (clamped to the text); `true` when the set changed. Refused: a range that
    /// cannot fold ([`can_fold`]), one already folded, one crossing a folded one's edge.
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
        let (header, _) = fold_lines(buf, &r);
        self.header_ends.insert(at, line_end(buf, header));
        self.folded.insert(at, r);
        self.generation += 1;
        true
    }

    /// Unfolds the innermost folded range holding `at` (its ends included); `true` when the set
    /// changed.
    pub fn unfold_at(&mut self, at: usize) -> bool {
        let inner = self
            .folded
            .iter()
            .enumerate()
            .filter(|(_, f)| f.start <= at && at <= f.end)
            .min_by_key(|(_, f)| f.len())
            .map(|(i, _)| i);
        match inner {
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
        let before = self.folded.len();
        let mut i = 0;
        while i < self.folded.len() {
            let (header, last) = fold_lines(buf, &self.folded[i]);
            if header < line && line <= last {
                self.remove(i);
            } else {
                i += 1;
            }
        }
        self.folded.len() != before
    }

    /// Unfolds every fold a selection range's end landed in (UI-EDIT-10 step 5): an end (the
    /// head or the anchor) on a line the fold hides, unless the range holds the fold whole
    /// (select all keeps the folds); `true` when the set changed.
    pub fn reveal_selection(&mut self, sel: &SelectionSet, buf: &TextBuffer) -> bool {
        if self.folded.is_empty() {
            return false;
        }
        let len = buf.len_bytes();
        let line_of = |at: usize| to_u32(buf.byte_to_line(at.min(len)));
        let mut changed = false;
        let mut i = 0;
        while i < self.folded.len() {
            let f = self.folded[i].clone();
            let (header, last) = fold_lines(buf, &f);
            let hit = sel.ranges().iter().any(|r| {
                let whole = r.range().start <= f.start && f.end <= r.range().end;
                !whole
                    && [r.head, r.anchor].iter().any(|&p| {
                        let l = line_of(p);
                        header < l && l <= last
                    })
            });
            if hit {
                self.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
        changed
    }

    /// The outermost folded range whose header is `line`.
    pub fn fold_on_line(&self, line: u32, buf: &TextBuffer) -> Option<Range<usize>> {
        self.folded
            .iter()
            .find(|f| fold_lines(buf, f).0 == line)
            .cloned()
    }

    /// Unfolds the outermost fold whose header is `line`; `true` when the set changed.
    pub fn unfold_line(&mut self, line: u32, buf: &TextBuffer) -> bool {
        match self
            .folded
            .iter()
            .position(|f| fold_lines(buf, f).0 == line)
        {
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
        self.generation += 1;
    }

    /// Unfolds everything; `true` when anything was folded.
    pub fn unfold_all(&mut self) -> bool {
        let had = !self.folded.is_empty();
        self.folded.clear();
        self.header_ends.clear();
        if had {
            self.generation += 1;
        }
        had
    }

    /// Maps the folded ranges through `c` (the module docs): kept through edits inside and at
    /// the edges, dropped when one edit deletes all of a range, its first character or its
    /// header line's break. [`FoldSet::prune`] with the new text follows.
    pub fn map(&mut self, c: &ChangeDesc) {
        if self.folded.is_empty() || c.changed_range().is_none() {
            return;
        }
        let mut folded = Vec::with_capacity(self.folded.len());
        let mut ends = Vec::with_capacity(self.folded.len());
        for (r, &he) in self.folded.iter().zip(&self.header_ends) {
            let first_kept = c
                .map_pos(r.start, Bias::After, MapMode::TrackAfter)
                .is_some();
            let break_kept =
                he >= r.end || c.map_pos(he, Bias::After, MapMode::TrackAfter).is_some();
            if !first_kept || !break_kept {
                continue;
            }
            if c.map_range(r.clone(), RangeKind::Container).is_none() {
                continue;
            }
            // Text typed at the start goes above the fold (its header stays its header); text
            // typed at the end joins it.
            let (Some(s), Some(e)) = (
                c.map_pos(r.start, Bias::After, MapMode::Simple),
                c.map_pos(r.end, Bias::After, MapMode::Simple),
            ) else {
                continue;
            };
            let m = s..e.max(s);
            let he = c
                .map_pos(he, Bias::After, MapMode::Simple)
                .unwrap_or(m.start);
            folded.push(m);
            ends.push(he);
        }
        self.folded = folded;
        self.header_ends = ends;
        self.generation += 1;
    }

    /// After a change: drops the folds that cannot fold any more (fewer than three lines,
    /// nothing hidden, a range two folds now share, one crossing another) and records each
    /// header line's end in `buf` for the next [`FoldSet::map`].
    pub fn prune(&mut self, buf: &TextBuffer) {
        let mut keep: Vec<(Range<usize>, usize)> = Vec::with_capacity(self.folded.len());
        let mut order: Vec<Range<usize>> = std::mem::take(&mut self.folded);
        order.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        for r in order {
            if !can_fold(buf, &r) || keep.iter().any(|(k, _)| *k == r || crosses(k, &r)) {
                continue;
            }
            let (header, _) = fold_lines(buf, &r);
            keep.push((r, line_end(buf, header)));
        }
        let changed = keep.len() != self.header_ends.len();
        self.header_ends = keep.iter().map(|(_, e)| *e).collect();
        self.folded = keep.into_iter().map(|(r, _)| r).collect();
        if changed {
            self.generation += 1;
        }
    }

    /// The folded ranges, sorted by start (the outer one first on a shared start).
    pub fn folded(&self) -> &[Range<usize>] {
        &self.folded
    }

    /// Bumped by every change of the set: the editor caches the hidden lines by it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The lines the folds hide (each fold keeps its header), sorted and merged.
    pub fn hidden_lines(&self, buf: &TextBuffer) -> Vec<Range<u32>> {
        let mut out: Vec<Range<u32>> = Vec::with_capacity(self.folded.len());
        for r in &self.folded {
            let (header, last) = fold_lines(buf, r);
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
