//! Selections (UI-EDIT-05 model; Task 10). A [`SelectionSet`] holds one or many ranges in byte
//! offsets with a main one; M3a's single-cursor commands use the main one, while IME mirroring
//! and the multi-cursor commands (M3b) need the many.
//!
//! **Normalised, always.** Every set is built through [`SelectionSet::normalized`] (or is a
//! single range), so its ranges are sorted by their start and merged: two ranges merge when
//! they overlap, when both are selections and they touch, or when a caret lies inside a
//! selection or at either of its edges; carets at one position collapse to one. The merged
//! range keeps the direction of the range that held the main one (else of the later range; a
//! caret has none, so then the other range's), and the main index follows it (CodeMirror's
//! normalisation, re-implemented).
//!
//! **Mapping** through a change: a caret maps by its `assoc`; a selection maps its start
//! `After` and its end `Before`, so text inserted at its edges stays outside it (a selection
//! never grows by mapping), all `MapMode::Simple`, in one pass over the change (O(k + s)). A
//! mapped set is normalised again (an edit may bring ranges together), and an edit resets every
//! goal column.
//!
//! **Goal column** (step 10): a vertical move sets `goal_x` from the head's x on the first
//! move and keeps it across moves ([`SelectionRange::moved_vertically`]); a horizontal move
//! ([`SelectionRange::moved_to`]) or an edit resets it. The x comes from the caller's layout
//! (the view's line layout; a test's column arithmetic), so this module stays display-free.

use std::ops::Range;

use smallvec::{SmallVec, smallvec};

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeDesc, MapMode, Mapper};

/// One selection: the anchor stays, the head moves; `goal_x` keeps the column of vertical
/// moves; `assoc` the side a caret keeps to at an insertion (and at a wrap or an inlay).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionRange {
    pub anchor: usize,
    pub head: usize,
    pub goal_x: Option<f32>,
    pub assoc: Bias,
}

impl SelectionRange {
    /// A caret at `at`.
    pub fn caret(at: usize) -> Self {
        Self::new(at, at)
    }

    /// The range from `anchor` to `head`, with no goal column, keeping to the text after it.
    pub fn new(anchor: usize, head: usize) -> Self {
        SelectionRange {
            anchor,
            head,
            goal_x: None,
            assoc: Bias::After,
        }
    }

    /// The selected bytes, low to high.
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// The head lies before the anchor (`selected_text_range`'s `reversed`).
    pub fn is_reversed(&self) -> bool {
        self.head < self.anchor
    }

    /// The head moved to `head` by a horizontal move: with `extend` (Shift) the anchor stays,
    /// else the range becomes a caret there. The goal column resets.
    pub fn moved_to(&self, head: usize, extend: bool) -> Self {
        SelectionRange {
            anchor: if extend { self.anchor } else { head },
            head,
            goal_x: None,
            assoc: self.assoc,
        }
    }

    /// The head moved `lines` buffer lines down (negative: up) at the goal column: `goal_x`
    /// when set, else the head's x as `x_of` measures it. `offset_at(line, x)` places the head
    /// on the target line (the layout's hit test); its answer is clamped to that line's text
    /// (never past its break) and to a character boundary. Moving past the first line puts the
    /// head at the start of the text, past the last at its end; the goal is kept either way.
    /// With `extend` (Shift) the anchor stays, else the range becomes a caret.
    pub fn moved_vertically(
        &self,
        buf: &TextBuffer,
        lines: isize,
        extend: bool,
        x_of: impl Fn(usize) -> f32,
        offset_at: impl Fn(usize, f32) -> usize,
    ) -> Self {
        let goal = self.goal_x.unwrap_or_else(|| x_of(self.head));
        let line = buf.byte_to_line(self.head);
        let last = buf.len_lines().saturating_sub(1);
        let head = match line.checked_add_signed(lines) {
            None => 0,
            Some(target) if target > last => buf.len_bytes(),
            Some(target) => {
                let body = buf.line_range(target, false);
                let at = offset_at(target, goal).clamp(body.start, body.end);
                buf.snap(at, Bias::Before)
            }
        };
        SelectionRange {
            anchor: if extend { self.anchor } else { head },
            head,
            goal_x: Some(goal),
            assoc: self.assoc,
        }
    }

    /// This range mapped through the change `m` walks (positions at or after the last query).
    fn mapped(&self, m: &mut Mapper<'_>, reset_goal: bool) -> Self {
        let r = self.range();
        let (from, to) = if r.is_empty() {
            let at = m
                .map(r.start, self.assoc, MapMode::Simple)
                .unwrap_or(r.start);
            (at, at)
        } else {
            let from = m
                .map(r.start, Bias::After, MapMode::Simple)
                .unwrap_or(r.start);
            let to = m.map(r.end, Bias::Before, MapMode::Simple).unwrap_or(r.end);
            // A replacement that took the selection's text puts its start after the new text
            // and its end before it: the selection collapses after the new text.
            (from, to.max(from))
        };
        let (anchor, head) = if self.is_reversed() {
            (to, from)
        } else {
            (from, to)
        };
        SelectionRange {
            anchor,
            head,
            goal_x: if reset_goal { None } else { self.goal_x },
            assoc: self.assoc,
        }
    }
}

/// The selections of a document, normalised (module docs), and which is the main one.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    ranges: SmallVec<[SelectionRange; 1]>,
    main: usize,
}

impl SelectionSet {
    /// One caret at `at`.
    pub fn cursor(at: usize) -> Self {
        Self::single(SelectionRange::caret(at))
    }

    pub fn single(r: SelectionRange) -> Self {
        SelectionSet {
            ranges: smallvec![r],
            main: 0,
        }
    }

    /// The set of `r` with `r[main]` as the main range (clamped to the last one), sorted and
    /// merged (module docs). An empty `r` is a caret at 0. O(k log k).
    pub fn normalized(r: impl IntoIterator<Item = SelectionRange>, main: usize) -> Self {
        let mut tagged: SmallVec<[(SelectionRange, bool); 1]> = r
            .into_iter()
            .enumerate()
            .map(|(i, r)| (r, i == main))
            .collect();
        match tagged.len() {
            0 => return Self::cursor(0),
            1 => return Self::single(tagged[0].0),
            n => {
                if main >= n {
                    tagged[n - 1].1 = true;
                }
            }
        }
        // Stable: equal ranges keep their input order.
        tagged.sort_by_key(|(r, _)| (r.range().start, r.range().end));
        let mut out: SmallVec<[(SelectionRange, bool); 1]> = SmallVec::new();
        for (r, is_main) in tagged {
            let Some((prev, prev_main)) = out.last_mut() else {
                out.push((r, is_main));
                continue;
            };
            let (p, c) = (prev.range(), r.range());
            // Sorted by start: `c` overlaps or touches `prev` exactly when it starts by
            // `prev`'s end. That merges overlaps, touching selections, a caret inside or at an
            // edge of a selection, and equal carets.
            if c.start > p.end {
                out.push((r, is_main));
                continue;
            }
            let (from, to) = (p.start, p.end.max(c.end));
            let (lead, other) = if *prev_main && !is_main {
                (*prev, r)
            } else {
                (r, *prev)
            };
            // A caret has no direction: then the other range's counts.
            let reversed = if lead.is_empty() {
                other.is_reversed()
            } else {
                lead.is_reversed()
            };
            let (anchor, head) = if reversed { (to, from) } else { (from, to) };
            *prev = SelectionRange {
                anchor,
                head,
                ..lead
            };
            *prev_main |= is_main;
        }
        let main = out.iter().position(|(_, m)| *m).unwrap_or(0);
        SelectionSet {
            ranges: out.into_iter().map(|(r, _)| r).collect(),
            main,
        }
    }

    /// Every range mapped through `c` (module docs), normalised again; an edit resets the goal
    /// columns, the identity keeps them. O(k + sections).
    pub fn map(&self, c: &ChangeDesc) -> Self {
        let reset_goal = c.changed_range().is_some();
        let mut m = c.mapper();
        let mapped: SmallVec<[SelectionRange; 1]> = self
            .ranges
            .iter()
            .map(|r| r.mapped(&mut m, reset_goal))
            .collect();
        Self::normalized(mapped, self.main)
    }

    pub fn main(&self) -> &SelectionRange {
        &self.ranges[self.main]
    }

    /// The main range's index in [`SelectionSet::ranges`].
    pub fn main_index(&self) -> usize {
        self.main
    }

    /// The ranges, sorted by their start, never overlapping.
    pub fn ranges(&self) -> &[SelectionRange] {
        &self.ranges
    }

    /// One Escape (UI-EDIT-05 step 8): many ranges collapse to the main one, a selection to a
    /// caret at its head; `None` for a single caret (nothing to collapse, so Escape is not
    /// consumed and can leave the editor).
    pub fn collapse(&self) -> Option<Self> {
        if self.ranges.len() > 1 {
            return Some(Self::single(*self.main()));
        }
        let r = self.main();
        (!r.is_empty()).then(|| Self::cursor(r.head))
    }
}
