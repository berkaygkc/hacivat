//! The window-free text and composition model behind [`TextInput`](super::TextInput).
//!
//! Every offset the platform sees is UTF-16 (`NSTextInputClient`, TSF); internally the model
//! keeps byte offsets into the text. It re-implements the M0 editor spike's `ImeCore`
//! (`spikes/spike-editor/src/input.rs`), including its app-switch rule in [`ImeState::blur`].

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation as _;

/// Text, selection and IME composition (marked range). The caret is `selection.end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImeState {
    text: String,
    /// Byte offsets.
    selection: Range<usize>,
    /// Byte offsets of the composition in progress.
    marked: Option<Range<usize>>,
}

impl ImeState {
    /// `text` with the caret at its end (M0 ruling R4).
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
            selection: text.len()..text.len(),
            marked: None,
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
    }

    /// Puts the caret at byte `offset` (floored to a char boundary).
    pub fn set_caret(&mut self, offset: usize) {
        let mut at = offset.min(self.text.len());
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        self.selection = at..at;
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
        self.marked = None;
        let range = if self.selection.is_empty() {
            let caret = self.selection.end;
            match self.text[..caret].grapheme_indices(true).next_back() {
                Some((start, _)) => start..caret,
                None => return false,
            }
        } else {
            self.selection.clone()
        };
        self.text.replace_range(range.clone(), "");
        self.selection = range.start..range.start;
        true
    }

    /// Collapses the selection to its start, else moves the caret one grapheme left.
    pub fn move_left(&mut self) {
        self.marked = None;
        let at = if self.selection.is_empty() {
            let caret = self.selection.end;
            self.text[..caret]
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(start, _)| start)
        } else {
            self.selection.start
        };
        self.selection = at..at;
    }

    /// Collapses the selection to its end, else moves the caret one grapheme right.
    pub fn move_right(&mut self) {
        self.marked = None;
        let at = if self.selection.is_empty() {
            let caret = self.selection.end;
            self.text[caret..]
                .graphemes(true)
                .next()
                .map_or(caret, |g| caret + g.len())
        } else {
            self.selection.end
        };
        self.selection = at..at;
    }

    pub fn select_all(&mut self) {
        self.marked = None;
        self.selection = 0..self.text.len();
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
