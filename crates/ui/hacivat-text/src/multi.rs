//! Multi-cursor selection building (UI-EDIT-05 multi; M3b Task 31): add a caret, add one above
//! or below, select the next or every occurrence, column (box) selection, split a selection
//! into lines, the IME's plain-text check, and the selection-only history (the spec's "Ctrl+U
//! undoes the selection"). Nothing here touches the text.
//!
//! - **The main range is the last one added** (a caret added, the next occurrence, the head's
//!   line of a box): the IME, the OS and Run follow it.
//! - **Occurrences are byte-exact** (case- and Turkish-exact: `İ`, `I`, `ı` and `i` are four
//!   different letters, INV-3). From a caret the search first selects the word under it, and a
//!   search that started that way matches whole words only, so `id` never matches inside
//!   `idx` (UI-EDIT-05 step 5; the caller keeps that bit, `select_next_occurrence_with`); a
//!   range selected by hand matches as a substring. The search reads the rope in 64 KB
//!   windows, runs forward from the main range and wraps; occurrences already selected are
//!   skipped. Select all occurrences replaces the selection with at most [`MAX_OCCURRENCES`]
//!   ranges, the asked-from one always among them, and counts them all.
//! - **Column selection** is a box in display columns: a tab fills to the next stop, a wide
//!   character or an emoji takes 2 cells, a visible control 1 (the display model's counting). A
//!   tab or a wide character the box cuts is taken whole; a line shorter than the box gets a
//!   caret at its end (no virtual space, NOTES surface 14).
//! - **Vertical carets** keep the goal column ([`SelectionRange::moved_vertically`]); the
//!   display-free [`add_caret_vertical`] counts the goal in display columns, the view passes
//!   its layout's x through [`add_caret_vertical_with`].
//!
//! Selection history: `model.rs` keeps the selections before each selection-only change (the
//! version each is for), and [`EditorModel::undo_selection`] restores the newest, mapped through
//! the text changes since; text changes add no entry.

use std::ops::Range;

use smallvec::SmallVec;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::buffer::{Bias, TextBuffer};
use crate::display::{cluster_cells, is_visible_control};
use crate::model::{Committed, EditorModel};
use crate::selection::{SelectionRange, SelectionSet};
use crate::units::{CharClass, TokenOracle, char_class, word_at};

/// The most ranges [`select_all_occurrences`] makes (spec: 10,000).
pub const MAX_OCCURRENCES: usize = 10_000;

/// The character classes' words (no tokens).
struct Classes;

impl TokenOracle for Classes {
    fn words_on_line(&self, _: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        None
    }
}

type Ranges = SmallVec<[SelectionRange; 4]>;

/// `sel` with a caret added at byte `at`, as the main range (merged with a range it touches).
pub fn add_caret(sel: &SelectionSet, at: usize) -> SelectionSet {
    let mut r: Ranges = sel.ranges().iter().copied().collect();
    let main = r.len();
    r.push(SelectionRange::caret(at));
    SelectionSet::normalized(r, main)
}

/// `sel` with a caret added on the line above (`down`: below) the main range's head, at its
/// goal column in display columns (`tab_size` wide tabs), as the main range. Unchanged on the
/// first (last) line.
pub fn add_caret_vertical(
    buf: &TextBuffer,
    sel: &SelectionSet,
    down: bool,
    tab_size: u8,
) -> SelectionSet {
    add_caret_vertical_with(
        buf,
        sel,
        down,
        |at| display_column(buf, at, tab_size) as f32,
        |line, x| offset_at_display_column(buf, line, x.round().max(0.0) as usize, tab_size),
    )
}

/// [`add_caret_vertical`] with the caller's geometry: `x_of(byte)` and `offset_at(line, x)`
/// as [`SelectionRange::moved_vertically`] takes them (the view's layout).
pub fn add_caret_vertical_with(
    buf: &TextBuffer,
    sel: &SelectionSet,
    down: bool,
    x_of: impl Fn(usize) -> f32,
    offset_at: impl Fn(usize, f32) -> usize,
) -> SelectionSet {
    let main = *sel.main();
    let line = buf.byte_to_line(main.head);
    let last = buf.len_lines().saturating_sub(1);
    if (down && line >= last) || (!down && line == 0) {
        return sel.clone();
    }
    let added = main.moved_vertically(buf, if down { 1 } else { -1 }, false, x_of, offset_at);
    let mut r: Ranges = sel.ranges().iter().copied().collect();
    let at = r.len();
    r.push(added);
    SelectionSet::normalized(r, at)
}

/// The word a caret at `at` selects (identifier or punctuation, never a run of spaces).
pub fn word_for_caret(buf: &TextBuffer, at: usize) -> Option<Range<usize>> {
    let w = word_at(buf, at, &Classes);
    let first = buf.slice(w.clone()).chars().next()?;
    matches!(char_class(first), CharClass::Ident | CharClass::Punct).then_some(w)
}

fn is_ident(c: Option<char>) -> bool {
    c.is_some_and(|c| char_class(c) == CharClass::Ident)
}

/// The character that starts at byte `b` (a char boundary), if any.
fn char_after(buf: &TextBuffer, b: usize) -> Option<char> {
    if b >= buf.len_bytes() {
        return None;
    }
    buf.char_before(buf.snap_to_char(b + 1, Bias::After))
}

/// No identifier character touches `r` from outside.
fn bounded(buf: &TextBuffer, r: &Range<usize>) -> bool {
    !is_ident(buf.char_before(r.start)) && !is_ident(char_after(buf, r.end))
}

// ## m3c-t31-multi (fix round 1, M7): the search reads the rope in windows, never the whole
// document at once.
/// How much of the text one search step reads.
const SEARCH_WINDOW: usize = 64 * 1024;

/// Calls `f` with each occurrence of `needle` that starts in `from..to`, in order and never
/// overlapping, until `f` answers `false`; `false` when it was stopped. Reads at most
/// [`SEARCH_WINDOW`] plus the needle's length per step.
fn each_occurrence(
    buf: &TextBuffer,
    from: usize,
    to: usize,
    needle: &str,
    mut f: impl FnMut(Range<usize>) -> bool,
) -> bool {
    let len = buf.len_bytes();
    let n = needle.len();
    if n == 0 {
        return true;
    }
    let mut s = from;
    let mut last_end = from;
    while s < to {
        let w_end = buf
            .snap_to_char((s + SEARCH_WINDOW).min(to), Bias::Before)
            .max(buf.snap_to_char(s + 1, Bias::After))
            .min(to);
        let hi = buf.snap_to_char((w_end + n).min(len), Bias::After);
        let text = buf.slice(s..hi);
        for (i, _) in text.match_indices(needle) {
            let start = s + i;
            if start >= w_end {
                break;
            }
            if start < last_end {
                continue;
            }
            last_end = start + n;
            if !f(start..start + n) {
                return false;
            }
        }
        s = w_end;
    }
    true
}

/// Whether `r` overlaps a range of `sel` (sorted, never overlapping).
fn overlaps_any(sel: &SelectionSet, r: &Range<usize>) -> bool {
    let ranges = sel.ranges();
    let i = ranges.partition_point(|x| x.range().end <= r.start);
    ranges.get(i).is_some_and(|x| x.range().start < r.end)
}

/// `sel` with the next occurrence of the main range's text added as the main range (module
/// docs); from a caret, the word under it first. Matches are substrings, as for a selection
/// made by hand; [`select_next_occurrence_with`] matches whole words when the search started
/// from a caret. `None` when there is nothing to add.
pub fn select_next_occurrence(buf: &TextBuffer, sel: &SelectionSet) -> Option<SelectionSet> {
    select_next_occurrence_with(buf, sel, false)
}

// ## m3c-t31-multi (fix round 1, M10): whole words only when the search started from a caret
// (UI-EDIT-05 step 5's `OccurrenceState`; the caller keeps that bit).
/// [`select_next_occurrence`], matching whole words only when `whole_word` (the search began
/// by expanding a caret to its word).
pub fn select_next_occurrence_with(
    buf: &TextBuffer,
    sel: &SelectionSet,
    whole_word: bool,
) -> Option<SelectionSet> {
    let main = *sel.main();
    if main.is_empty() {
        let w = word_for_caret(buf, main.head)?;
        let mut r: Ranges = sel.ranges().iter().copied().collect();
        r[sel.main_index()] = SelectionRange::new(w.start, w.end);
        return Some(SelectionSet::normalized(r, sel.main_index()));
    }
    let range = main.range();
    let needle = buf.slice(range.clone()).into_owned();
    let len = buf.len_bytes();
    let mut found = None;
    let mut take = |r: Range<usize>| {
        if (whole_word && !bounded(buf, &r)) || overlaps_any(sel, &r) {
            return true;
        }
        found = Some(r);
        false
    };
    // Forward from the main range, then from the start (wrapping).
    if each_occurrence(buf, range.end, len, &needle, &mut take) {
        each_occurrence(buf, 0, range.end, &needle, &mut take);
    }
    let found = found?;
    let mut r: Ranges = sel.ranges().iter().copied().collect();
    let at = r.len();
    r.push(SelectionRange::new(found.start, found.end));
    Some(SelectionSet::normalized(r, at))
}

/// Every occurrence of the main range's text (from a caret, the word under it, matched as a
/// whole word), at most `cap` ranges ([`MAX_OCCURRENCES`]), replacing the selection; the
/// asked-from range is the main one and is always kept. `sel` unchanged when there is nothing
/// to search for.
pub fn select_all_occurrences(buf: &TextBuffer, sel: &SelectionSet, cap: usize) -> SelectionSet {
    let whole = sel.main().is_empty();
    select_all_occurrences_with(buf, sel, cap, whole).0
}

// ## m3c-t31-multi (fix round 1, M10, M11)
/// [`select_all_occurrences`] with the whole-word bit given (it applies to a range; a caret
/// always searches its word whole), and the total number of occurrences, kept or not (the
/// "first 10,000 of N" sentence, Task 33).
pub fn select_all_occurrences_with(
    buf: &TextBuffer,
    sel: &SelectionSet,
    cap: usize,
    whole_word: bool,
) -> (SelectionSet, usize) {
    let main = *sel.main();
    let (range, whole) = if main.is_empty() {
        match word_for_caret(buf, main.head) {
            Some(w) => (w, true),
            None => return (sel.clone(), 0),
        }
    } else {
        (main.range(), whole_word)
    };
    let needle = buf.slice(range.clone()).into_owned();
    let cap = cap.max(1);
    let mut found: Vec<Range<usize>> = Vec::new();
    let mut total = 0usize;
    each_occurrence(buf, 0, buf.len_bytes(), &needle, |r| {
        if whole && !bounded(buf, &r) {
            return true;
        }
        total += 1;
        if r != range && found.len() < cap - 1 {
            found.push(r);
        }
        true
    });
    let at = found.partition_point(|r| r.start < range.start);
    found.insert(at, range);
    let set = SelectionSet::normalized(
        found
            .into_iter()
            .map(|r| SelectionRange::new(r.start, r.end)),
        at,
    );
    (set, total.max(1))
}

/// The cells of a grapheme cluster `g` that starts at display column `col`.
fn cells(g: &str, col: usize, tab: usize) -> usize {
    if g == "\t" {
        tab - col % tab
    } else if g.chars().next().is_some_and(is_visible_control) {
        1
    } else {
        cluster_cells(g) as usize
    }
}

/// The display column of byte `at` on its line (tabs to `tab_size` stops, wide characters 2).
pub(crate) fn display_column(buf: &TextBuffer, at: usize, tab_size: u8) -> usize {
    let tab = usize::from(tab_size.max(1));
    let start = buf.line_to_byte(buf.byte_to_line(at));
    let at = buf.snap(at.min(buf.len_bytes()), Bias::Before).max(start);
    let mut col = 0;
    for g in buf.slice(start..at).graphemes(true) {
        col += cells(g, col, tab);
    }
    col
}

/// The boundary on `line` nearest to display column `col` (a tie goes to the earlier one),
/// clamped to the line's text.
fn offset_at_display_column(buf: &TextBuffer, line: usize, col: usize, tab_size: u8) -> usize {
    let tab = usize::from(tab_size.max(1));
    let body = buf.line_range(line, false);
    let mut at = 0;
    for (i, g) in buf.slice(body.clone()).grapheme_indices(true) {
        let w = cells(g, at, tab);
        if col < at + w {
            return body.start + if col - at > w / 2 { i + g.len() } else { i };
        }
        at += w;
    }
    body.end
}

/// The bytes of `line` a box over display columns `[lo, hi)` takes (module docs).
fn box_on_line(buf: &TextBuffer, line: usize, lo: usize, hi: usize, tab: usize) -> Range<usize> {
    let body = buf.line_range(line, false);
    let (mut start, mut end) = (None, None);
    let mut col = 0;
    for (i, g) in buf.slice(body.clone()).grapheme_indices(true) {
        let to = col + cells(g, col, tab);
        if start.is_none() && to > lo {
            start = Some(body.start + i);
        }
        if end.is_none() && col >= hi {
            end = Some(body.start + i);
            break;
        }
        col = to;
    }
    let start = start.unwrap_or(body.end);
    let end = if lo == hi {
        start
    } else {
        end.unwrap_or(body.end).max(start)
    };
    start..end
}

/// The box from `anchor` to `head`, each `(line, display column)` with `tab_size` wide tabs:
/// one range per line, in the drag's direction, the main one on the head's line (module docs).
pub fn column_selection(
    buf: &TextBuffer,
    anchor: (u32, usize),
    head: (u32, usize),
    tab_size: u8,
) -> SelectionSet {
    let tab = usize::from(tab_size.max(1));
    let last = buf.len_lines().saturating_sub(1);
    let line = |l: u32| usize::try_from(l).unwrap_or(usize::MAX).min(last);
    let (a, h) = (line(anchor.0), line(head.0));
    let (lo, hi) = (anchor.1.min(head.1), anchor.1.max(head.1));
    let back = head.1 < anchor.1;
    let ranges = (a.min(h)..=a.max(h)).map(|l| {
        let r = box_on_line(buf, l, lo, hi, tab);
        if back {
            SelectionRange::new(r.end, r.start)
        } else {
            SelectionRange::new(r.start, r.end)
        }
    });
    SelectionSet::normalized(ranges, h - a.min(h))
}

/// Each range of `sel` that spans lines split into one range per line (a selection that ends at
/// a line's start leaves that line out); the main one is the last piece of the main range.
pub fn split_into_lines(buf: &TextBuffer, sel: &SelectionSet) -> SelectionSet {
    let mut out: Ranges = SmallVec::new();
    let mut main = 0;
    for (i, r) in sel.ranges().iter().enumerate() {
        let range = r.range();
        let first = buf.byte_to_line(range.start);
        let end_line = buf.byte_to_line(range.end);
        let last = if end_line > first && buf.line_to_byte(end_line) == range.end {
            end_line - 1
        } else {
            end_line
        };
        if first == end_line {
            out.push(*r);
        } else {
            for l in first..=last {
                let body = buf.line_range(l, false);
                let (s, e) = (range.start.max(body.start), range.end.min(body.end));
                out.push(SelectionRange::new(s, e.max(s)));
            }
        }
        if i == sel.main_index() {
            main = out.len() - 1;
        }
    }
    SelectionSet::normalized(out, main)
}

/// Every caret of `sel` is in plain text: the oracle says, for both ends of every range, that
/// they are outside strings and comments. The IME mirrors a composition to every caret only
/// then (R-M3-64).
// ## m3c-t31-multi (fix round 1, M3): an oracle that does not know (no analysis at this
// version, e.g. right after typing a quote) is not plain text: the fallback is the safe side.
pub fn carets_in_plain_text(sel: &SelectionSet, tok: &dyn TokenOracle) -> bool {
    sel.ranges().iter().all(|r| {
        let range = r.range();
        tok.in_string_or_comment(range.start) == Some(false)
            && (range.is_empty() || tok.in_string_or_comment(range.end) == Some(false))
    })
}

impl EditorModel {
    /// Takes back the newest selection change (the selection-only history beside the text's):
    /// the selection before it, mapped through the text changes since, without touching the
    /// text. `None` when there is none.
    pub fn undo_selection(&mut self) -> Option<Committed> {
        self.step_selection(false)
    }

    /// Re-applies the newest selection change [`EditorModel::undo_selection`] took back.
    pub fn redo_selection(&mut self) -> Option<Committed> {
        self.step_selection(true)
    }
}
