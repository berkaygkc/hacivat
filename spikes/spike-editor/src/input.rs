//! IME and text input for the editor spike.
//!
//! Plain typing and IME composition share one path: GPUI delivers every text input to
//! `EntityInputHandler::replace_text_in_range`, and the key handler no longer inserts
//! printable characters itself. The composition logic lives in the window-free `ImeCore`
//! so unit tests can drive it. Every offset the platform sees is UTF-16; internally the
//! editor uses char indices.

use std::ops::Range;
use std::time::Instant;

use hacivat_ui_core::gpui_surface::{
    Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window,
};

use crate::buffer::{Buffer, is_identifier_char};
use crate::element::{EditKind, EditorView};

/// The text operations the IME logic needs, in char indices and UTF-16 offsets.
pub trait TextBuf {
    fn len_chars(&self) -> usize;
    fn replace_chars(&mut self, range: Range<usize>, text: &str);
    fn chars_to_string(&self, range: Range<usize>) -> String;
    fn char_to_utf16(&self, char_idx: usize) -> usize;
    fn utf16_to_char(&self, utf16_idx: usize) -> usize;
}

impl TextBuf for Buffer {
    fn len_chars(&self) -> usize {
        Buffer::len_chars(self)
    }
    fn replace_chars(&mut self, range: Range<usize>, text: &str) {
        self.replace(range, text);
    }
    fn chars_to_string(&self, range: Range<usize>) -> String {
        self.text_in(range)
    }
    fn char_to_utf16(&self, char_idx: usize) -> usize {
        Buffer::char_to_utf16(self, char_idx)
    }
    fn utf16_to_char(&self, utf16_idx: usize) -> usize {
        Buffer::utf16_to_char(self, utf16_idx)
    }
}

/// Selection and composition state, char indices. The caret is `selection.end`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImeCore {
    pub selection: Range<usize>,
    pub marked: Option<Range<usize>>,
}

impl ImeCore {
    /// An empty selection (the caret) at `char_idx`.
    pub fn caret_at(char_idx: usize) -> Self {
        Self {
            selection: char_idx..char_idx,
            marked: None,
        }
    }

    /// The range an edit targets: the platform's range, else the composition, else the
    /// selection (the NSTextInputClient contract).
    fn target<T: TextBuf>(&self, text: &T, range_utf16: Option<Range<usize>>) -> Range<usize> {
        let len = text.len_chars();
        let range = match range_utf16 {
            Some(r) => text.utf16_to_char(r.start)..text.utf16_to_char(r.end),
            None => self.marked.clone().unwrap_or(self.selection.clone()),
        };
        let start = range.start.min(len);
        start..range.end.clamp(start, len)
    }

    /// Commits `new_text` over the target range and puts the caret after it.
    pub fn replace<T: TextBuf>(
        &mut self,
        text: &mut T,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
    ) {
        let target = self.target(text, range_utf16);
        text.replace_chars(target.clone(), new_text);
        let caret = target.start + new_text.chars().count();
        self.selection = caret..caret;
        self.marked = None;
    }

    /// Replaces the target range with in-progress composition text and marks it.
    /// `new_selected_utf16` is relative to the start of `new_text`.
    pub fn replace_and_mark<T: TextBuf>(
        &mut self,
        text: &mut T,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_utf16: Option<Range<usize>>,
    ) {
        let target = self.target(text, range_utf16);
        let start_utf16 = text.char_to_utf16(target.start);
        text.replace_chars(target.clone(), new_text);
        let inserted = new_text.chars().count();
        let end = target.start + inserted;
        self.marked = (inserted > 0).then_some(target.start..end);
        self.selection = match new_selected_utf16 {
            Some(sel) => {
                let len16 = new_text.encode_utf16().count();
                let s = text
                    .utf16_to_char(start_utf16 + sel.start.min(len16))
                    .min(end);
                let e = text
                    .utf16_to_char(start_utf16 + sel.end.min(len16))
                    .clamp(s, end);
                s..e
            }
            None => end..end,
        };
    }

    /// Ends the composition and keeps its text (the commit on focus loss).
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Focus left the editor. Returns whether the composition was committed.
    ///
    /// When the window itself resigned key (app switch), the platform input context keeps
    /// the composition and the IME resumes it on return (NSTextView behaves the same), so
    /// the marked range must survive: a local commit would orphan the IME's composition and
    /// its next `insertText`/`setMarkedText` would land beside the committed copy. Only a
    /// focus move inside a still-active window commits locally; GPUI 912159c gives the app
    /// no way to make the platform discard its composition (see ime-fix-report.md).
    pub fn blur(&mut self, window_active: bool) -> bool {
        let committed = window_active && self.marked.is_some();
        if committed {
            self.unmark();
        }
        committed
    }

    pub fn marked_utf16<T: TextBuf>(&self, text: &T) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|m| text.char_to_utf16(m.start)..text.char_to_utf16(m.end))
    }

    pub fn selection_utf16<T: TextBuf>(&self, text: &T) -> Range<usize> {
        text.char_to_utf16(self.selection.start)..text.char_to_utf16(self.selection.end)
    }

    /// The text of a UTF-16 range, clamped to the document. `adjusted` receives the range
    /// actually returned.
    pub fn text_for_range<T: TextBuf>(
        &self,
        text: &T,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> String {
        let len16 = text.char_to_utf16(text.len_chars());
        let start = range_utf16.start.min(len16);
        let end = range_utf16.end.clamp(start, len16);
        *adjusted = Some(start..end);
        text.chars_to_string(text.utf16_to_char(start)..text.utf16_to_char(end))
    }
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.ime.text_for_range(&self.buffer, range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.ime.selection_utf16(&self.buffer),
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.ime.marked_utf16(&self.buffer)
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ime.unmark();
        // The underline is part of the cached line shape; the next frame redraws it.
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_at = Instant::now();
        self.ime.replace(&mut self.buffer, range, text);
        let kind = if text.is_empty() {
            EditKind::Delete
        } else if text.chars().all(is_identifier_char) {
            EditKind::Identifier
        } else {
            EditKind::Other
        };
        self.after_edit(kind, input_at, cx);
        window.notify_text_input_changed(&self.focus_handle);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_at = Instant::now();
        self.ime
            .replace_and_mark(&mut self.buffer, range, new_text, new_selected_range);
        // A composition never offers completions.
        self.after_edit(EditKind::Other, input_at, cx);
        window.notify_text_input_changed(&self.focus_handle);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.buffer.utf16_to_char(range_utf16.start)
            ..self.buffer.utf16_to_char(range_utf16.end);
        // The cached line layout is exact; right after an edit, before the next frame, it
        // is stale and the monospace estimate from the last layout stands in.
        self.bounds_for_char_range(range.clone())
            .or_else(|| self.estimated_bounds_for_char_range(range))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.char_at_point(point)
            .map(|c| self.buffer.char_to_utf16(c))
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime.selection = self.buffer.utf16_to_char(range_utf16.start)
            ..self.buffer.utf16_to_char(range_utf16.end);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.buffer.char_to_utf16(self.buffer.len_chars()))
    }
}

/// A window-free text store for tests.
#[cfg(test)]
struct StringBuf(String);

#[cfg(test)]
impl StringBuf {
    fn byte(&self, char_idx: usize) -> usize {
        self.0
            .char_indices()
            .nth(char_idx)
            .map_or(self.0.len(), |(b, _)| b)
    }
}

#[cfg(test)]
impl TextBuf for StringBuf {
    fn len_chars(&self) -> usize {
        self.0.chars().count()
    }
    fn replace_chars(&mut self, range: Range<usize>, text: &str) {
        let (s, e) = (self.byte(range.start), self.byte(range.end));
        self.0.replace_range(s..e, text);
    }
    fn chars_to_string(&self, range: Range<usize>) -> String {
        self.0[self.byte(range.start)..self.byte(range.end)].to_string()
    }
    fn char_to_utf16(&self, char_idx: usize) -> usize {
        self.0.chars().take(char_idx).map(char::len_utf16).sum()
    }
    fn utf16_to_char(&self, utf16_idx: usize) -> usize {
        let mut units = 0;
        for (i, c) in self.0.chars().enumerate() {
            if units >= utf16_idx {
                return i;
            }
            units += c.len_utf16();
        }
        self.len_chars()
    }
}

#[cfg(test)]
struct ImeState {
    buf: StringBuf,
    core: ImeCore,
}

#[cfg(test)]
impl ImeState {
    /// The caret starts as an empty selection at the end of `text` (controller ruling R4).
    fn new(text: &str) -> Self {
        let buf = StringBuf(text.to_string());
        let core = ImeCore::caret_at(buf.len_chars());
        Self { buf, core }
    }
    fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        self.core.replace(&mut self.buf, range, text);
    }
    fn replace_and_mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) {
        self.core
            .replace_and_mark(&mut self.buf, range, text, selected);
    }
    fn unmark(&mut self) {
        self.core.unmark();
    }
    fn blur(&mut self, window_active: bool) -> bool {
        self.core.blur(window_active)
    }
    fn text(&self) -> &str {
        &self.buf.0
    }
    fn marked_utf16(&self) -> Option<Range<usize>> {
        self.core.marked_utf16(&self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_mark_sets_marked_range() {
        let mut s = ImeState::new("SELECT ");
        s.replace_and_mark(None, "ç", None);
        assert_eq!(s.marked_utf16(), Some(7..8));
    }

    #[test]
    fn unmark_after_focus_loss_commits_marked_text() {
        let mut s = ImeState::new("x");
        s.replace_and_mark(None, "ş", None);
        s.unmark();
        assert_eq!(s.text(), "xş");
        assert_eq!(s.marked_utf16(), None);
    }

    #[test]
    fn replace_in_marked_range_replaces_composition() {
        let mut s = ImeState::new("");
        s.replace_and_mark(None, "k", None);
        s.replace_and_mark(None, "か", None);
        s.replace(None, "感");
        assert_eq!(s.text(), "感");
    }

    #[test]
    fn dead_key_sequence_produces_single_char() {
        let mut s = ImeState::new("");
        s.replace_and_mark(None, "^", None);
        s.replace(None, "â");
        assert_eq!(s.text(), "â");
    }

    #[test]
    fn utf16_offsets_count_surrogate_pairs() {
        let mut s = ImeState::new("😀");
        s.replace_and_mark(None, "ş", None);
        assert_eq!(s.marked_utf16(), Some(2..3));
        assert_eq!(s.core.selection_utf16(&s.buf), 3..3);
    }

    #[test]
    fn composition_selection_is_relative_to_the_marked_text() {
        let mut s = ImeState::new("ab");
        s.replace_and_mark(None, "かな", Some(0..1));
        assert_eq!(s.core.selection_utf16(&s.buf), 2..3);
    }

    #[test]
    fn explicit_range_wins_over_composition() {
        let mut s = ImeState::new("abc");
        s.replace_and_mark(None, "x", None);
        s.replace(Some(0..1), "Z");
        assert_eq!(s.text(), "Zbcx");
        assert_eq!(s.marked_utf16(), None);
    }

    // Bug 1 (PO manual test, Japanese Romaji + Live Conversion): compose 感じ, click Finder,
    // come back, press Esc. NSTextInputContext still holds the composition, so the IME
    // re-delivers it with replacementRange = NSNotFound (`None`). Our side must still have
    // the marked range, or the re-delivered text lands next to a local commit.
    #[test]
    fn blur_duplication_app_switch_keeps_composition_for_the_platform() {
        let mut s = ImeState::new("SELECT ");
        s.replace_and_mark(None, "感じ", Some(2..2));
        assert!(!s.blur(false));
        assert_eq!(s.marked_utf16(), Some(7..9));
        // Back in the app, Esc: the IME replaces its composition (here, another candidate).
        s.replace_and_mark(None, "幹事", Some(2..2));
        s.replace(None, "幹事");
        assert_eq!(s.text(), "SELECT 幹事");
    }

    #[test]
    fn blur_duplication_ime_commit_on_return_replaces_composition() {
        // The IME may instead commit on return (e.g. on the click that reactivates the
        // window): insertText(sameText, NSNotFound) must not append a second copy.
        let mut s = ImeState::new("");
        s.replace_and_mark(None, "感じ", Some(2..2));
        s.blur(false);
        s.replace(None, "感じ");
        assert_eq!(s.text(), "感じ");
        assert_eq!(s.marked_utf16(), None);
    }

    #[test]
    fn blur_within_active_window_commits_composition() {
        // Focus moved to another view in the same, still-key window: GPUI gives us no way
        // to tell NSTextInputContext, so the editor commits locally (Review Focus 4).
        let mut s = ImeState::new("x");
        s.replace_and_mark(None, "ş", None);
        assert!(s.blur(true));
        assert_eq!(s.text(), "xş");
        assert_eq!(s.marked_utf16(), None);
    }

    // Bug 2 (PO manual test, ABC – Extended, Option+I then A gives "ʼa"): matches native
    // TextEdit. In that layout Option+I is the horn dead key (U+02BC standalone; composes
    // only with o/u into ơ/ư), so UCKeyTranslate itself yields "ʼa". These pin that a dead
    // key's marked text is replaced, never committed-then-appended, however AppKit commits.
    #[test]
    fn dead_key_commit_with_nsnotfound_replaces_the_marked_dead_key() {
        let mut s = ImeState::new("SELECT ");
        s.replace_and_mark(None, "ʼ", Some(1..1));
        s.replace(None, "ʼa");
        assert_eq!(s.text(), "SELECT ʼa");
        assert_eq!(s.marked_utf16(), None);
        assert_eq!(s.core.selection_utf16(&s.buf), 9..9);
    }

    #[test]
    fn dead_key_commit_with_explicit_replacement_range_replaces_the_mark() {
        let mut s = ImeState::new("x");
        s.replace_and_mark(None, "ˆ", Some(1..1));
        s.replace(Some(1..2), "â");
        assert_eq!(s.text(), "xâ");
    }

    #[test]
    fn dead_key_split_commit_mark_then_letter_yields_both_once() {
        // AppKit may also commit the dead key alone and then insert the letter.
        let mut s = ImeState::new("");
        s.replace_and_mark(None, "ʼ", Some(1..1));
        s.replace(None, "ʼ");
        s.replace(None, "a");
        assert_eq!(s.text(), "ʼa");
    }

    #[test]
    fn text_for_range_clamps_and_reports_the_adjusted_range() {
        let s = ImeState::new("İş");
        let mut adjusted = None;
        assert_eq!(s.core.text_for_range(&s.buf, 1..99, &mut adjusted), "ş");
        assert_eq!(adjusted, Some(1..2));
    }
}
