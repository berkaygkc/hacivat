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
        let caret = self.head();
        self.delete_towards(prev_grapheme(&self.text, caret))
    }

    /// Collapses the selection to its start, else moves the caret one grapheme left.
    pub fn move_left(&mut self) {
        let at = if self.selection.is_empty() {
            prev_grapheme(&self.text, self.head())
        } else {
            self.selection.start
        };
        self.collapse_to(at);
    }

    /// Collapses the selection to its end, else moves the caret one grapheme right.
    pub fn move_right(&mut self) {
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
        self.extend_to(prev_grapheme(&self.text, self.head()));
    }

    /// Extends the selection one grapheme right.
    pub fn select_right(&mut self) {
        self.extend_to(next_grapheme(&self.text, self.head()));
    }

    /// Moves the caret to the start of the word before it (from a selection: its start).
    pub fn move_word_left(&mut self) {
        let at = prev_word_start(&self.text, self.selection.start);
        self.collapse_to(at);
    }

    /// Moves the caret to the end of the word after it (from a selection: its end).
    pub fn move_word_right(&mut self) {
        let at = next_word_end(&self.text, self.selection.end);
        self.collapse_to(at);
    }

    /// Extends the selection to the start of the word before the head.
    pub fn select_word_left(&mut self) {
        self.extend_to(prev_word_start(&self.text, self.head()));
    }

    /// Extends the selection to the end of the word after the head.
    pub fn select_word_right(&mut self) {
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
        self.extend_to(0);
    }

    /// Extends the selection to the end of the line.
    pub fn select_line_end(&mut self) {
        self.extend_to(self.text.len());
    }

    /// Deletes the selection, else the grapheme after the caret. Returns whether the text
    /// changed.
    pub fn delete_forward(&mut self) -> bool {
        let caret = self.head();
        self.delete_towards(next_grapheme(&self.text, caret))
    }

    /// Deletes the selection, else back to the start of the word before the caret.
    pub fn delete_word_backward(&mut self) -> bool {
        let caret = self.head();
        self.delete_towards(prev_word_start(&self.text, caret))
    }

    /// Deletes the selection, else up to the end of the word after the caret.
    pub fn delete_word_forward(&mut self) -> bool {
        let caret = self.head();
        self.delete_towards(next_word_end(&self.text, caret))
    }

    /// Deletes the selection, else back to the start of the line.
    pub fn delete_to_line_start(&mut self) -> bool {
        self.delete_towards(0)
    }

    /// Deletes the selection, else up to the end of the line (Control-K).
    pub fn delete_to_line_end(&mut self) -> bool {
        let end = self.text.len();
        self.delete_towards(end)
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
/// The start of the grapheme before byte `at` (0 at the start).
fn prev_grapheme(text: &str, at: usize) -> usize {
    text[..at]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(start, _)| start)
}

/// The end of the grapheme after byte `at` (`at` at the end).
fn next_grapheme(text: &str, at: usize) -> usize {
    text[at..]
        .graphemes(true)
        .next()
        .map_or(at, |g| at + g.len())
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
                | '\u{2000}'..='\u{206f}'
                | '\u{2e00}'..='\u{2e7f}'
                | '\u{3000}'..='\u{303f}'
                | '\u{fe10}'..='\u{fe1f}'
                | '\u{fe30}'..='\u{fe4f}'
                | '\u{ff01}'..='\u{ff0f}'
                | '\u{ff1a}'..='\u{ff20}'
                | '\u{ff3b}'..='\u{ff40}'
                | '\u{ff5b}'..='\u{ff65}'
        )
}

/// Where a word move left from byte `at` stops, as on macOS and Windows: back over white space
/// and punctuation, then to the start of the word before them (0 when there is none).
/// Boundaries are Unicode word boundaries (UAX #29), which never split a grapheme cluster.
pub fn prev_word_start(text: &str, at: usize) -> usize {
    text[..at]
        .split_word_bound_indices()
        .rev()
        .find(|(_, seg)| is_word(seg))
        .map_or(0, |(start, _)| start)
}

/// Where a word move right from byte `at` stops: over white space and punctuation, then to the
/// end of the word after them (the end of the text when there is none).
pub fn next_word_end(text: &str, at: usize) -> usize {
    text[at..]
        .split_word_bound_indices()
        .find(|(_, seg)| is_word(seg))
        .map_or(text.len(), |(start, seg)| at + start + seg.len())
}

/// The word-boundary segment that holds byte `at` (a double click): the word, or the run of
/// white space or punctuation; at the very end, the last segment.
pub fn word_at(text: &str, at: usize) -> Range<usize> {
    let mut last = 0..0;
    for (start, seg) in text.split_word_bound_indices() {
        let range = start..start + seg.len();
        if range.contains(&at) {
            return range;
        }
        last = range;
    }
    last
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
    fn a_dot_or_apostrophe_between_letters_joins_them_as_unicode_says() {
        // UAX #29 (WB6, WB7): `.` and `'` between letters are inside the word, as `e.g.`;
        // with a space after them they are punctuation again.
        assert_eq!(next_word_end("müşteri.ad x", 0), "müşteri.ad".len());
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
