//! The window-free text and composition model behind [`TextInput`](super::TextInput).
//!
//! Every offset the platform sees is UTF-16 (`NSTextInputClient`, TSF); internally the model
//! keeps byte offsets into the text. It re-implements the M0 editor spike's `ImeCore`
//! (`spikes/spike-editor/src/input.rs`), including its app-switch rule in [`ImeState::blur`].

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// Text, selection and IME composition (marked range). The caret (the selection's moving end,
/// its head) is `selection.end`, or `selection.start` when the selection is reversed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImeState {
    text: String,
    /// Byte offsets.
    selection: Range<usize>,
    /// Byte offsets of the composition in progress.
    marked: Option<Range<usize>>,
    // ## inputnav-1
    /// The head is `selection.start` (a selection extended leftwards from its anchor).
    reversed: bool,
}

impl ImeState {
    /// `text` with the caret at its end (M0 ruling R4).
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            selection: text.len()..text.len(),
            marked: None,
            reversed: false,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the whole text, ends any composition and puts the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        *self = Self::new(text);
    }

    pub fn is_composing(&self) -> bool {
        self.marked.is_some()
    }

    /// Selection in bytes (for painting).
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// Composition in bytes (for painting).
    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Commits `text` over the target range (the platform's `range_utf16`, else the
    /// composition, else the selection) and puts the caret after it.
    pub fn replace(&mut self, range_utf16: Option<Range<usize>>, text: &str) {
        let target = self.target(range_utf16);
        self.text.replace_range(target.clone(), text);
        let caret = target.start + text.len();
        self.selection = caret..caret;
        self.marked = None;
        self.reversed = false;
    }

    /// Replaces the target range with in-progress composition text and marks it; an empty
    /// `text` ends the composition. `selected_utf16` is relative to the start of `text`.
    pub fn replace_and_mark(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
    ) {
        let target = self.target(range_utf16);
        self.text.replace_range(target.clone(), text);
        let end = target.start + text.len();
        self.marked = (!text.is_empty()).then_some(target.start..end);
        self.reversed = false;
        self.selection = match selected_utf16 {
            Some(sel) => {
                let s = target.start + utf16_to_byte(text, sel.start);
                let e = target.start + utf16_to_byte(text, sel.end);
                s..e.max(s)
            }
            None => end..end,
        };
    }

    /// Ends the composition and keeps its text.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Focus left the input. Commits the composition only while the window is still active
    /// and returns whether it did.
    ///
    /// When the window itself resigned key (app switch), the platform input context keeps the
    /// composition and the IME resumes it on return, as NSTextView does. A local commit would
    /// orphan the IME's copy, and its next delivery would land beside the committed text. Only
    /// a focus move inside a still-active window commits locally (M0, PO-verified on macOS).
    pub fn blur(&mut self, window_active: bool) -> bool {
        let committed = window_active && self.marked.is_some();
        if committed {
            self.unmark();
        }
        committed
    }

    pub fn marked_utf16(&self) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|m| self.byte_to_utf16(m.start)..self.byte_to_utf16(m.end))
    }

    pub fn selection_utf16(&self) -> Range<usize> {
        self.byte_to_utf16(self.selection.start)..self.byte_to_utf16(self.selection.end)
    }

    /// Selects a UTF-16 range (the platform moving the selection).
    pub fn set_selection_utf16(&mut self, range: Range<usize>) {
        let start = self.utf16_to_byte_at(range.start);
        let end = self.utf16_to_byte_at(range.end).max(start);
        self.selection = start..end;
        self.reversed = false;
    }

    /// Puts the caret at byte `offset` (floored to a char boundary).
    pub fn set_caret(&mut self, offset: usize) {
        let at = self.floor_char(offset);
        self.selection = at..at;
        self.reversed = false;
    }

    pub fn len_utf16(&self) -> usize {
        self.byte_to_utf16(self.text.len())
    }

    /// The text of a UTF-16 range clamped to the text; `adjusted` receives the range returned.
    pub fn text_for_range_utf16(
        &self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> String {
        let len = self.len_utf16();
        let start = range.start.min(len);
        let end = range.end.clamp(start, len);
        *adjusted = Some(start..end);
        self.text[self.utf16_to_byte_at(start)..self.utf16_to_byte_at(end)].to_string()
    }

    pub fn byte_to_utf16(&self, byte: usize) -> usize {
        self.text[..byte.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum()
    }

    pub fn utf16_to_byte_at(&self, utf16: usize) -> usize {
        utf16_to_byte(&self.text, utf16)
    }

    /// Deletes the selection, else the grapheme before the caret. Returns whether the text
    /// changed. A composition in progress is committed first.
    pub fn backspace(&mut self) -> bool {
        self.snap_to_clusters();
        let caret = self.head();
        self.delete_towards(prev_grapheme(&self.text, caret))
    }

    /// Collapses the selection to its start, else moves the caret one grapheme left.
    pub fn move_left(&mut self) {
        self.snap_to_clusters();
        let at = if self.selection.is_empty() {
            prev_grapheme(&self.text, self.head())
        } else {
            self.selection.start
        };
        self.collapse_to(at);
    }

    /// Collapses the selection to its end, else moves the caret one grapheme right.
    pub fn move_right(&mut self) {
        self.snap_to_clusters();
        let at = if self.selection.is_empty() {
            next_grapheme(&self.text, self.head())
        } else {
            self.selection.end
        };
        self.collapse_to(at);
    }

    pub fn select_all(&mut self) {
        self.marked = None;
        self.selection = 0..self.text.len();
        self.reversed = false;
    }

    // ## inputnav-1
    /// The caret: the end of the selection that moves when it is extended.
    pub fn head(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }

    /// The fixed end of the selection.
    pub fn anchor(&self) -> usize {
        if self.reversed {
            self.selection.end
        } else {
            self.selection.start
        }
    }

    /// Whether the head is the selection's start (it was extended leftwards).
    pub fn is_reversed(&self) -> bool {
        self.reversed && !self.selection.is_empty()
    }

    /// Moves the head to byte `offset` (floored to a char boundary) and keeps the anchor: a
    /// Shift move, a Shift click or a drag.
    pub fn extend_to(&mut self, offset: usize) {
        self.marked = None;
        let head = self.floor_char(offset);
        let anchor = self.anchor();
        self.reversed = head < anchor;
        self.selection = anchor.min(head)..anchor.max(head);
    }

    /// Selects `range` (bytes, floored to char boundaries) with the head at its end.
    pub fn select_range(&mut self, range: Range<usize>) {
        self.marked = None;
        let start = self.floor_char(range.start);
        let end = self.floor_char(range.end).max(start);
        self.selection = start..end;
        self.reversed = false;
    }

    /// Extends the selection one grapheme left.
    pub fn select_left(&mut self) {
        self.snap_to_clusters();
        self.extend_to(prev_grapheme(&self.text, self.head()));
    }

    /// Extends the selection one grapheme right.
    pub fn select_right(&mut self) {
        self.snap_to_clusters();
        self.extend_to(next_grapheme(&self.text, self.head()));
    }

    /// Moves the caret to the start of the word before it (from a selection: its start).
    pub fn move_word_left(&mut self) {
        self.snap_to_clusters();
        let at = prev_word_start(&self.text, self.selection.start);
        self.collapse_to(at);
    }

    /// Moves the caret to the end of the word after it (from a selection: its end).
    pub fn move_word_right(&mut self) {
        self.snap_to_clusters();
        let at = next_word_end(&self.text, self.selection.end);
        self.collapse_to(at);
    }

    /// Extends the selection to the start of the word before the head.
    pub fn select_word_left(&mut self) {
        self.snap_to_clusters();
        self.extend_to(prev_word_start(&self.text, self.head()));
    }

    /// Extends the selection to the end of the word after the head.
    pub fn select_word_right(&mut self) {
        self.snap_to_clusters();
        self.extend_to(next_word_end(&self.text, self.head()));
    }

    /// Moves the caret to the start of the line (the whole value).
    pub fn move_line_start(&mut self) {
        self.collapse_to(0);
    }

    /// Moves the caret to the end of the line.
    pub fn move_line_end(&mut self) {
        self.collapse_to(self.text.len());
    }

    /// Extends the selection to the start of the line.
    pub fn select_line_start(&mut self) {
        self.snap_to_clusters();
        self.extend_to(0);
    }

    /// Extends the selection to the end of the line.
    pub fn select_line_end(&mut self) {
        self.snap_to_clusters();
        self.extend_to(self.text.len());
    }

    /// Deletes the selection, else the grapheme after the caret. Returns whether the text
    /// changed.
    pub fn delete_forward(&mut self) -> bool {
        self.snap_to_clusters();
        let caret = self.head();
        self.delete_towards(next_grapheme(&self.text, caret))
    }

    /// Deletes the selection, else back to the start of the word before the caret.
    pub fn delete_word_backward(&mut self) -> bool {
        self.snap_to_clusters();
        let caret = self.head();
        self.delete_towards(prev_word_start(&self.text, caret))
    }

    /// Deletes the selection, else up to the end of the word after the caret.
    pub fn delete_word_forward(&mut self) -> bool {
        self.snap_to_clusters();
        let caret = self.head();
        self.delete_towards(next_word_end(&self.text, caret))
    }

    /// Deletes the selection, else back to the start of the line.
    pub fn delete_to_line_start(&mut self) -> bool {
        self.snap_to_clusters();
        self.delete_towards(0)
    }

    /// Deletes the selection, else up to the end of the line (Control-K).
    pub fn delete_to_line_end(&mut self) -> bool {
        self.snap_to_clusters();
        let end = self.text.len();
        self.delete_towards(end)
    }

    // ## inputnav-1 fix round 1 (I2): the Windows word moves.
    /// Moves the caret to the start of the next word (from a selection: its end), as Ctrl+→
    /// does on Windows.
    pub fn move_next_word_start(&mut self) {
        self.snap_to_clusters();
        let at = next_word_start(&self.text, self.selection.end);
        self.collapse_to(at);
    }

    /// Extends the selection to the start of the word after the head.
    pub fn select_next_word_start(&mut self) {
        self.snap_to_clusters();
        self.extend_to(next_word_start(&self.text, self.head()));
    }

    /// Deletes the selection, else up to the start of the next word, with the white space
    /// before it (Ctrl+Delete on Windows).
    pub fn delete_to_next_word_start(&mut self) -> bool {
        self.snap_to_clusters();
        let caret = self.head();
        self.delete_towards(next_word_start(&self.text, caret))
    }

    // ## inputnav-1 fix round 1 (m4): undo.
    /// The text and selection, without the composition (what undo restores).
    pub fn snapshot(&self) -> ImeState {
        ImeState {
            marked: None,
            ..self.clone()
        }
    }

    /// Puts back a [`snapshot`](Self::snapshot).
    pub fn restore(&mut self, snapshot: ImeState) {
        *self = snapshot;
    }

    /// Ends any composition and puts the caret at `at`.
    fn collapse_to(&mut self, at: usize) {
        self.marked = None;
        self.selection = at..at;
        self.reversed = false;
    }

    /// Deletes the selection, else the text between the caret and `to`; a composition in
    /// progress is committed first. Returns whether the text changed.
    fn delete_towards(&mut self, to: usize) -> bool {
        self.marked = None;
        let range = if self.selection.is_empty() {
            let caret = self.selection.start;
            caret.min(to)..caret.max(to)
        } else {
            self.selection.clone()
        };
        if range.is_empty() {
            return false;
        }
        self.text.replace_range(range.clone(), "");
        self.collapse_to(range.start);
        true
    }

    // ## inputnav-1 fix round 1 (m6)
    /// Moves both ends of the selection that lie inside a grapheme cluster (the platform, the
    /// IME or assistive technology may set one there) to that cluster's start, so a move or a
    /// delete never splits a cluster.
    fn snap_to_clusters(&mut self) {
        let len = self.text.len();
        let floor = |at: usize| {
            self.text
                .grapheme_indices(true)
                .map(|(start, _)| start)
                .chain(std::iter::once(len))
                .take_while(|&start| start <= at)
                .last()
                .unwrap_or(0)
        };
        let (start, end) = (floor(self.selection.start), floor(self.selection.end));
        self.selection = start..end.max(start);
    }

    /// `offset` clamped to the text and floored to a char boundary.
    fn floor_char(&self, offset: usize) -> usize {
        let mut at = offset.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// The range an edit targets: the platform's range, else the composition, else the
    /// selection (the `NSTextInputClient` contract).
    fn target(&self, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let range = match range_utf16 {
            Some(r) => self.utf16_to_byte_at(r.start)..self.utf16_to_byte_at(r.end),
            None => self.marked.clone().unwrap_or(self.selection.clone()),
        };
        let len = self.text.len();
        let start = range.start.min(len);
        start..range.end.clamp(start, len)
    }
}

/// The byte offset of UTF-16 offset `utf16` in `text`, clamped to its end. An offset inside a
/// surrogate pair rounds up to the end of that character.
fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

// ## inputnav-1
/// The grapheme boundary before byte `at` (0 at the start). The clusters come from the whole
/// text, so an `at` inside a cluster (a selection the platform set) moves to that cluster's
/// start, never to another offset inside it (fix round 1, m6).
fn prev_grapheme(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .take_while(|&start| start < at)
        .last()
        .unwrap_or(0)
}

/// The grapheme boundary after byte `at` (the end of the text at its end), from the whole
/// text's clusters (m6).
fn next_grapheme(text: &str, at: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(start, g)| start + g.len())
        .find(|&end| end > at)
        .unwrap_or(text.len())
}

// ## inputnav-1 fix round 1 (ruling R-IN-1)
/// The separators that end a word between two letters although UAX #29 (WB6/WB7) joins them:
/// full stop, colon, middle dot and their variants (Greek ano teleia, one dot leader, the
/// presentation, small and full-width forms). `schema.table`, `public.orders.id` and `a:b` are
/// separate words, as in Cocoa fields and DB tools; between digits they still join (`3.14`).
const SPLITS_LETTERS: [char; 10] = [
    '.', ':', '\u{b7}', '\u{387}', '\u{2024}', '\u{fe13}', '\u{fe52}', '\u{fe55}', '\u{ff0e}',
    '\u{ff1a}',
];

/// The word-boundary segments of `text`: UAX #29's, with every segment split at a
/// [`SPLITS_LETTERS`] separator unless the nearest letter or digit on each side is a digit
/// (R-IN-1). Apostrophes (`İstanbul'da`, `don’t`), `_` and the digit joins (`3.14`, `1,000`)
/// stay as UAX #29 has them. A split never falls inside a grapheme cluster: the separators
/// are single, unextended characters inside a segment UAX #29 already joined around them.
fn word_segments(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for (start, seg) in text.split_word_bound_indices() {
        let mut from = start;
        for (i, c) in seg.char_indices() {
            if !SPLITS_LETTERS.contains(&c) {
                continue;
            }
            let at = start + i;
            let before = seg[..i].chars().rev().find(|c| c.is_alphanumeric());
            let after = seg[i + c.len_utf8()..]
                .chars()
                .find(|c| c.is_alphanumeric());
            let digits =
                before.is_some_and(char::is_numeric) && after.is_some_and(char::is_numeric);
            // A separator that starts its segment already stands alone.
            if digits || i == 0 {
                continue;
            }
            if at > from {
                out.push(from..at);
            }
            out.push(at..at + c.len_utf8());
            from = at + c.len_utf8();
        }
        if from < start + seg.len() {
            out.push(from..start + seg.len());
        }
    }
    out
}

/// Whether a word-boundary segment (UAX #29) is a word a word move stops at: it holds a
/// letter, a digit or a symbol such as an emoji, not only white space and punctuation.
fn is_word(segment: &str) -> bool {
    segment
        .chars()
        .any(|c| !(c.is_whitespace() || is_punctuation(c) || c.is_control()))
}

/// Punctuation a word move skips: ASCII punctuation and symbols, the Latin-1 marks
/// (`¡ § « ¶ · » ¿`), the General and Supplemental Punctuation blocks (with the zero-width
/// joiners and other format characters met on their own), CJK punctuation and the
/// full-width forms of ASCII punctuation.
fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{a1}'
                | '\u{a7}'
                | '\u{ab}'
                | '\u{b6}'
                | '\u{b7}'
                | '\u{bb}'
                | '\u{bf}'
                // ## inputnav-1 fix round 1 (R-IN-1): the Greek ano teleia.
                | '\u{387}'
                | '\u{2000}'..='\u{206f}'
                | '\u{2e00}'..='\u{2e7f}'
                | '\u{3000}'..='\u{303f}'
                | '\u{fe10}'..='\u{fe1f}'
                | '\u{fe30}'..='\u{fe4f}'
                // ## inputnav-1 fix round 1 (R-IN-1): the small form variants (`﹒`, `﹕`).
                | '\u{fe50}'..='\u{fe6b}'
                | '\u{ff01}'..='\u{ff0f}'
                | '\u{ff1a}'..='\u{ff20}'
                | '\u{ff3b}'..='\u{ff40}'
                | '\u{ff5b}'..='\u{ff65}'
        )
}

/// Where a word move left from byte `at` stops, as on macOS and Windows: back over white space
/// and punctuation, then to the start of the word before them (0 when there is none).
/// Boundaries are [`word_segments`] (UAX #29 with R-IN-1), which never split a grapheme
/// cluster; an `at` inside a cluster counts as its start (m6).
pub fn prev_word_start(text: &str, at: usize) -> usize {
    word_segments(text)
        .into_iter()
        .rev()
        .find(|r| r.start < at && is_word(&text[r.clone()]))
        .map_or(0, |r| r.start)
}

/// Where a word move right from byte `at` stops on macOS and Linux: over white space and
/// punctuation, then to the end of the word after them (the end of the text when there is
/// none).
pub fn next_word_end(text: &str, at: usize) -> usize {
    word_segments(text)
        .into_iter()
        .find(|r| r.end > at && is_word(&text[r.clone()]))
        .map_or(text.len(), |r| r.end)
}

// ## inputnav-1 fix round 1 (I2)
/// Where a word move right from byte `at` stops on Windows: past the rest of the segment that
/// holds `at` (a word, or a run of punctuation), then past white space only, to the start of
/// what follows (`foo bar` from 0: 4, before `bar`; the end of the text when nothing does).
pub fn next_word_start(text: &str, at: usize) -> usize {
    let segments = word_segments(text);
    let mut i = segments
        .iter()
        .position(|r| r.end > at)
        .unwrap_or(segments.len());
    let space = |r: &Range<usize>| text[r.clone()].chars().all(char::is_whitespace);
    if i < segments.len() && !space(&segments[i]) {
        i += 1;
    }
    while i < segments.len() && space(&segments[i]) {
        i += 1;
    }
    segments.get(i).map_or(text.len(), |r| r.start)
}

/// The segment a double click selects: the word, or the run of white space or punctuation,
/// that holds byte `at`. At the very end of the text it is the last word, never the white
/// space after it (fix round 1, m7); an empty text gives `0..0`.
pub fn word_at(text: &str, at: usize) -> Range<usize> {
    let segments = word_segments(text);
    if let Some(r) = segments.iter().find(|r| r.contains(&at)) {
        return r.clone();
    }
    segments
        .iter()
        .rev()
        .find(|r| is_word(&text[(*r).clone()]))
        .or(segments.last())
        .cloned()
        .unwrap_or(0..0)
}

// ## inputnav-1 fix round 1 (m4): a field's undo history.
/// What an edit was, for coalescing: a run of typing, or of deleting in one direction, is one
/// undo step; everything else (a paste, a cut, a word or line delete, an IME commit) is a step
/// of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EditKind {
    Typing,
    DeleteBackward,
    DeleteForward,
    Other,
}

/// The most undo steps a field keeps; the oldest goes first.
pub const HISTORY_MAX: usize = 100;

/// The undo and redo stacks of one field. It lives and dies with its field; a masked field
/// keeps none at all (the field never records), so no secret outlives an edit in it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditHistory {
    undo: Vec<ImeState>,
    redo: Vec<ImeState>,
    /// The kind of the last edit and the caret it left, while a run may go on.
    run: Option<(EditKind, usize)>,
}

impl EditHistory {
    /// Records an edit of `kind` that turned `before` into `after`. Nothing is recorded when
    /// the text did not change. The edit opens a new undo step unless it goes on with the run
    /// of the last one: the same typing or deleting kind, from the caret that one left, with no
    /// selection. Any edit clears the redo stack.
    pub fn record(&mut self, kind: EditKind, before: &ImeState, after: &ImeState) {
        if before.text() == after.text() {
            return;
        }
        let continues = kind != EditKind::Other
            && before.selection().is_empty()
            && self.run == Some((kind, before.head()));
        if !continues {
            self.undo.push(before.snapshot());
            if self.undo.len() > HISTORY_MAX {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.run = (kind != EditKind::Other).then(|| (kind, after.head()));
    }

    /// Ends the run: the next edit opens a step of its own (a caret move, a click).
    pub fn break_run(&mut self) {
        self.run = None;
    }

    /// Takes the last step back. Returns whether there was one.
    pub fn undo(&mut self, state: &mut ImeState) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(state.snapshot());
        state.restore(previous);
        self.run = None;
        true
    }

    /// Puts the last undone step back. Returns whether there was one.
    pub fn redo(&mut self, state: &mut ImeState) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(state.snapshot());
        state.restore(next);
        self.run = None;
        true
    }

    /// Forgets everything (a value set by the app, a field turned masked).
    pub fn clear(&mut self) {
        *self = EditHistory::default();
    }

    /// The number of undo and redo steps (tests).
    pub fn depth(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

#[cfg(test)]
mod nav_tests {
    use super::*;

    /// A state over `text` with the caret at byte `at`.
    fn at(text: &str, at: usize) -> ImeState {
        let mut s = ImeState::new(text);
        s.set_caret(at);
        s
    }

    fn caret(s: &ImeState) -> usize {
        assert!(
            s.selection().is_empty(),
            "no selection: {:?}",
            s.selection()
        );
        s.head()
    }

    // ---- word boundaries ---------------------------------------------------------------------

    #[test]
    fn word_right_skips_space_and_punctuation_then_stops_at_the_word_end() {
        let t = "select  *, müşteri (ad)";
        assert_eq!(next_word_end(t, 0), 6);
        // From the gap: over "  *, " to the end of "müşteri".
        assert_eq!(next_word_end(t, 6), "select  *, müşteri".len());
        assert_eq!(
            next_word_end(t, "select  *, müşteri".len()),
            "select  *, müşteri (ad".len(),
            "over ` (` to the end of `ad`"
        );
        assert_eq!(next_word_end(t, t.len() - 1), t.len(), "no word after `)`");
        assert_eq!(next_word_end(t, t.len()), t.len());
    }

    #[test]
    fn word_left_skips_space_and_punctuation_then_stops_at_the_word_start() {
        let t = "select  *, müşteri (ad)";
        assert_eq!(prev_word_start(t, t.len()), "select  *, müşteri (".len());
        assert_eq!(
            prev_word_start(t, "select  *, müşteri (".len()),
            "select  *, ".len()
        );
        assert_eq!(prev_word_start(t, "select  *, ".len()), 0);
        assert_eq!(prev_word_start(t, 0), 0);
    }

    #[test]
    fn a_dot_between_letters_ends_the_word_and_an_apostrophe_does_not() {
        // R-IN-1 (fix round 1, I1): UAX #29 joins `müşteri.ad` (WB6, WB7); fields split it at
        // the dot, as Cocoa and DB tools do. The apostrophe and digit joins stay.
        assert_eq!(next_word_end("müşteri.ad x", 0), "müşteri".len());
        assert_eq!(next_word_end("müşteri. ad", 0), "müşteri".len());
        assert_eq!(prev_word_start("3.14 ", 5), 0);
    }

    #[test]
    fn turkish_dotted_capitals_and_apostrophes_stay_one_word() {
        // `İ` is two bytes, `i̇` (i + U+0307 combining dot) three: neither splits a word.
        let t = "İstanbul'da ıi\u{0307}ğş";
        assert_eq!(next_word_end(t, 0), "İstanbul'da".len());
        assert_eq!(next_word_end(t, "İstanbul'da".len()), t.len());
        assert_eq!(prev_word_start(t, t.len()), "İstanbul'da ".len());
        assert_eq!(prev_word_start(t, "İstanbul'da".len()), 0);
    }

    #[test]
    fn emoji_and_zwj_sequences_are_words_and_never_split() {
        // A family (ZWJ sequence), a flag (regional indicators) and a skin-tone modifier.
        let family = "👨\u{200d}👩\u{200d}👧";
        let t = format!("a {family} 🇹🇷 👍🏽 b");
        let after_family = 2 + family.len();
        assert_eq!(next_word_end(&t, 1), after_family);
        assert_eq!(prev_word_start(&t, after_family), 2);
        let flag_end = after_family + 1 + "🇹🇷".len();
        assert_eq!(next_word_end(&t, after_family), flag_end);
        let thumb_end = flag_end + 1 + "👍🏽".len();
        assert_eq!(next_word_end(&t, flag_end), thumb_end);
        assert_eq!(prev_word_start(&t, thumb_end), flag_end + 1);
    }

    #[test]
    fn a_run_of_punctuation_only_goes_to_the_edge() {
        assert_eq!(next_word_end("a ...!?", 1), "a ...!?".len());
        assert_eq!(prev_word_start("...!? a", 5), 0);
        assert_eq!(prev_word_start("  ", 2), 0);
        assert_eq!(next_word_end("  ", 0), 2);
    }

    #[test]
    fn word_at_finds_the_segment_under_a_click() {
        let t = "ab  müşteri, ad";
        assert_eq!(word_at(t, 5), 4.."ab  müşteri".len());
        assert_eq!(word_at(t, 2), 2..4, "the gap selects the gap");
        assert_eq!(word_at(t, t.len()), "ab  müşteri, ".len()..t.len());
        assert_eq!(word_at("", 0), 0..0);
    }

    // ---- moves -------------------------------------------------------------------------------

    #[test]
    fn select_right_and_left_extend_by_grapheme_from_a_fixed_anchor() {
        let t = "ai\u{0307}😀ş";
        let mut s = at(t, 1);
        s.select_right();
        assert_eq!((s.selection(), s.is_reversed()), (1..4, false));
        s.select_right();
        assert_eq!(s.selection(), 1..8);
        s.select_left();
        s.select_left();
        s.select_left();
        // Past the anchor: the selection flips and the head leads.
        assert_eq!((s.selection(), s.is_reversed()), (0..1, true));
        assert_eq!(s.selection_utf16(), 0..1);
    }

    #[test]
    fn left_and_right_collapse_a_selection_to_its_edges() {
        let mut s = at("İzmir", 2);
        s.select_line_end();
        s.move_left();
        assert_eq!(caret(&s), 2);
        s.select_line_start();
        s.move_right();
        assert_eq!(caret(&s), 2);
    }

    #[test]
    fn word_moves_and_selections() {
        let t = "foo bar-baz";
        let mut s = at(t, 0);
        s.move_word_right();
        assert_eq!(caret(&s), 3);
        s.move_word_right();
        assert_eq!(caret(&s), 7);
        s.select_word_left();
        assert_eq!((s.selection(), s.is_reversed()), (4..7, true));
        s.select_word_left();
        assert_eq!(s.selection(), 0..7);
        // With a selection a word move starts from the edge it goes towards.
        s.move_word_right();
        assert_eq!(caret(&s), t.len());
        s.move_word_left();
        assert_eq!(caret(&s), 8);
        s.select_word_right();
        assert_eq!((s.selection(), s.is_reversed()), (8..t.len(), false));
        s.move_word_left();
        assert_eq!(caret(&s), 4);
        s.select_word_left();
        s.move_word_right();
        assert_eq!(caret(&s), 7, "from the selection's end (`bar`)");
    }

    #[test]
    fn line_moves_and_selections() {
        let mut s = at("şehir", 3);
        s.select_line_start();
        assert_eq!((s.selection(), s.is_reversed()), (0..3, true));
        s.select_line_end();
        assert_eq!((s.selection(), s.is_reversed()), (3.."şehir".len(), false));
        s.move_line_start();
        assert_eq!(caret(&s), 0);
        s.move_line_end();
        assert_eq!(caret(&s), "şehir".len());
    }

    // ---- deletes -----------------------------------------------------------------------------

    #[test]
    fn delete_forward_removes_one_grapheme_or_the_selection() {
        let mut s = at("a👨\u{200d}👩b", 1);
        assert!(s.delete_forward());
        assert_eq!((s.text(), caret(&s)), ("ab", 1));
        s.move_line_end();
        assert!(!s.delete_forward());
        s.select_line_start();
        assert!(s.delete_forward());
        assert_eq!(s.text(), "");
    }

    #[test]
    fn word_deletes() {
        let mut s = at("SELECT müşteri, ad", "SELECT müşteri, ad".len());
        assert!(s.delete_word_backward());
        assert_eq!(s.text(), "SELECT müşteri, ");
        assert!(s.delete_word_backward());
        assert_eq!(s.text(), "SELECT ");
        s.move_line_start();
        assert!(s.delete_word_forward());
        assert_eq!((s.text(), caret(&s)), (" ", 0));
        assert!(s.delete_word_forward());
        assert_eq!(s.text(), "");
        assert!(!s.delete_word_backward());
    }

    #[test]
    fn line_deletes() {
        let mut s = at("ğüş", 2);
        assert!(s.delete_to_line_start());
        assert_eq!((s.text(), caret(&s)), ("üş", 0));
        s.move_right();
        assert!(s.delete_to_line_end());
        assert_eq!((s.text(), caret(&s)), ("ü", 2));
        assert!(!s.delete_to_line_end());
        // A selection goes first.
        s.select_line_start();
        assert!(s.delete_to_line_end());
        assert_eq!(s.text(), "");
    }

    #[test]
    fn a_composition_ends_before_a_move_or_delete() {
        let mut s = ImeState::new("x");
        s.replace_and_mark(None, "ş", None);
        s.move_word_left();
        assert!(!s.is_composing());
        s.replace_and_mark(None, "ğ", None);
        s.delete_word_forward();
        assert!(!s.is_composing());
    }

    #[test]
    fn replacing_or_selecting_by_utf16_resets_the_direction() {
        let mut s = at("abc", 2);
        s.select_line_start();
        assert!(s.is_reversed());
        s.set_selection_utf16(1..3);
        assert!(!s.is_reversed());
        s.select_line_start();
        s.replace(None, "z");
        assert_eq!((s.text(), s.is_reversed()), ("zbc", false));
    }

    #[test]
    fn extend_to_moves_the_head_and_keeps_the_anchor() {
        let mut s = at("İstanbul", 2);
        s.extend_to(6);
        assert_eq!((s.selection(), s.is_reversed()), (2..6, false));
        s.extend_to(0);
        assert_eq!((s.selection(), s.is_reversed()), (0..2, true));
        s.select_range(3..5);
        assert_eq!((s.selection(), s.is_reversed()), (3..5, false));
        // A byte inside a character floors to its start.
        s.extend_to(1);
        assert_eq!(s.selection(), 0..3);
    }
}

// ## inputnav-1 fix round 1
#[cfg(test)]
mod fr1_tests {
    use super::*;

    /// The words a word move stops at, left to right.
    fn words(text: &str) -> Vec<&str> {
        word_segments(text)
            .into_iter()
            .map(|r| &text[r])
            .filter(|s| is_word(s))
            .collect()
    }

    /// R-IN-1: a table of what splits and what joins.
    #[test]
    fn r_in_1_splits_qualified_names_and_keeps_the_other_joins() {
        let table: &[(&str, &[&str])] = &[
            ("schema.table", &["schema", "table"]),
            ("public.orders.id", &["public", "orders", "id"]),
            ("müşteri.ad", &["müşteri", "ad"]),
            ("a:b", &["a", "b"]),
            ("a·b", &["a", "b"]),
            ("a\u{387}b", &["a", "b"]),
            ("a\u{2024}b", &["a", "b"]),
            ("a\u{fe13}b", &["a", "b"]),
            ("a\u{fe52}b", &["a", "b"]),
            ("a\u{fe55}b", &["a", "b"]),
            ("a\u{ff0e}b", &["a", "b"]),
            ("a\u{ff1a}b", &["a", "b"]),
            ("İ.ş", &["İ", "ş"]),
            // A combining mark before the dot: the nearest letter decides.
            ("i\u{307}.a", &["i\u{307}", "a"]),
            // The joins that stay.
            ("İstanbul'da", &["İstanbul'da"]),
            ("don\u{2019}t", &["don\u{2019}t"]),
            ("3.14", &["3.14"]),
            ("1,000", &["1,000"]),
            ("v1.2", &["v1.2"]),
            ("müşteri_raporu", &["müşteri_raporu"]),
            ("a.1", &["a", "1"]),
            ("e.g. x", &["e", "g", "x"]),
        ];
        for (text, want) in table {
            assert_eq!(words(text), *want, "{text:?}");
        }
    }

    #[test]
    fn word_moves_stop_at_each_part_of_a_qualified_name() {
        let t = "public.orders.id";
        assert_eq!(next_word_end(t, 0), 6);
        assert_eq!(next_word_end(t, 6), 13);
        assert_eq!(next_word_end(t, 13), 16);
        assert_eq!(prev_word_start(t, 16), 14);
        assert_eq!(prev_word_start(t, 14), 7);
        assert_eq!(prev_word_start(t, 7), 0);
        let mut s = ImeState::new(t);
        assert!(s.delete_word_backward());
        assert_eq!(s.text(), "public.orders.");
        assert_eq!(word_at("schema.table", 8), 7..12);
        assert_eq!(word_at("a:b", 1), 1..2, "the colon alone");
    }

    /// I2: `foo bar` from 0 is 4 on Windows (the next word's start) and 3 elsewhere.
    #[test]
    fn the_windows_word_move_ends_at_the_next_words_start() {
        assert_eq!(next_word_start("foo bar", 0), 4);
        assert_eq!(next_word_end("foo bar", 0), 3);
        assert_eq!(next_word_start("fo|o bar".replace('|', "").as_str(), 2), 4);
        assert_eq!(next_word_start("foo bar", 3), 4);
        assert_eq!(next_word_start("foo   bar", 1), 6);
        assert_eq!(next_word_start("foo bar", 4), 7);
        assert_eq!(next_word_start("foo ", 0), 4);
        assert_eq!(next_word_start("", 0), 0);
        // Punctuation is a stop of its own.
        assert_eq!(next_word_start("foo, bar", 0), 3);
        assert_eq!(next_word_start("foo, bar", 3), 5);
        assert_eq!(next_word_start("schema.table", 0), 6);
        let mut s = ImeState::new("foo bar");
        s.set_caret(0);
        assert!(s.delete_to_next_word_start());
        assert_eq!((s.text(), s.selection()), ("bar", 0..0));
        let mut s = ImeState::new("foo bar");
        s.set_caret(0);
        s.select_next_word_start();
        assert_eq!(s.selection(), 0..4);
        s.move_next_word_start();
        assert_eq!(s.selection(), 7..7);
    }

    /// m6: an offset inside a cluster moves from the cluster, never into the next one.
    #[test]
    fn a_start_inside_a_cluster_snaps_to_its_boundary() {
        let t = "🇹🇷🇺🇸";
        assert_eq!(next_word_end(t, 4), 8);
        assert_eq!(prev_word_start(t, 12), 8);
        let mut s = ImeState::new(t);
        s.set_selection_utf16(2..2); // inside the first flag
        assert_eq!(s.selection(), 4..4);
        s.move_right();
        assert_eq!(s.selection(), 8..8);
        s.set_selection_utf16(2..2);
        s.move_left();
        assert_eq!(s.selection(), 0..0);
        s.set_selection_utf16(6..6); // inside the second flag
        s.select_right();
        assert_eq!(s.selection(), 8..16);
        s.set_selection_utf16(6..6);
        assert!(s.delete_forward());
        assert_eq!(s.text(), "🇹🇷", "the whole flag, never half of it");
        let mut s = ImeState::new(t);
        s.set_selection_utf16(6..6);
        assert!(s.backspace());
        assert_eq!(s.text(), "🇺🇸");
    }

    /// m7: past the end a double click selects the last word, not the white space after it.
    #[test]
    fn word_at_past_the_end_is_the_last_word() {
        assert_eq!(word_at("foo bar  ", 9), 4..7);
        assert_eq!(word_at("foo bar", 7), 4..7);
        assert_eq!(word_at("   ", 3), 0..3);
        assert_eq!(word_at("", 0), 0..0);
        // Inside the text, the white space under the pointer is still what it selects.
        assert_eq!(word_at("foo bar  ", 7), 7..9);
    }

    fn typed(h: &mut EditHistory, s: &mut ImeState, text: &str) {
        let before = s.snapshot();
        s.replace(None, text);
        h.record(EditKind::Typing, &before, s);
    }

    #[test]
    fn typing_runs_coalesce_and_a_move_breaks_them() {
        let mut h = EditHistory::default();
        let mut s = ImeState::new("");
        for c in ["s", "e", "ç"] {
            typed(&mut h, &mut s, c);
        }
        assert_eq!(h.depth(), (1, 0));
        s.move_left();
        h.break_run();
        typed(&mut h, &mut s, "x");
        assert_eq!((s.text(), h.depth()), ("sexç", (2, 0)));
        assert!(h.undo(&mut s));
        assert_eq!((s.text(), s.selection()), ("seç", 2..2));
        assert!(h.undo(&mut s));
        assert_eq!(s.text(), "");
        assert!(!h.undo(&mut s));
        assert!(h.redo(&mut s));
        assert_eq!(s.text(), "seç");
        assert!(h.redo(&mut s));
        assert_eq!(s.text(), "sexç");
        assert!(!h.redo(&mut s));
    }

    #[test]
    fn deletes_coalesce_by_direction_and_other_edits_stand_alone() {
        let mut h = EditHistory::default();
        let mut s = ImeState::new("abcdef");
        for _ in 0..2 {
            let before = s.snapshot();
            s.backspace();
            h.record(EditKind::DeleteBackward, &before, &s);
        }
        assert_eq!((s.text(), h.depth()), ("abcd", (1, 0)));
        // A delete that changes nothing records nothing.
        s.move_line_end();
        let before = s.snapshot();
        s.delete_forward();
        h.record(EditKind::DeleteForward, &before, &s);
        assert_eq!(h.depth(), (1, 0));
        // Two pastes are two steps.
        for p in ["x", "y"] {
            let before = s.snapshot();
            s.replace(None, p);
            h.record(EditKind::Other, &before, &s);
        }
        assert_eq!(h.depth(), (3, 0));
        h.undo(&mut s);
        // A new edit clears the redo stack.
        typed(&mut h, &mut s, "z");
        assert_eq!(h.depth(), (3, 0));
        assert_eq!(s.text(), "abcdxz");
    }

    #[test]
    fn the_history_keeps_at_most_its_cap() {
        let mut h = EditHistory::default();
        let mut s = ImeState::new("");
        for i in 0..HISTORY_MAX + 10 {
            let before = s.snapshot();
            s.replace(None, &i.to_string());
            h.record(EditKind::Other, &before, &s);
        }
        assert_eq!(h.depth(), (HISTORY_MAX, 0));
        h.clear();
        assert_eq!(h.depth(), (0, 0));
    }
}
