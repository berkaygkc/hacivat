//! The window-free IME core (UI-EDIT-12 model; Task 10): marked text is real buffer text inside
//! an IME bracket; UTF-16 ranges convert through the buffer; an app-switch blur leaves the
//! composition with the platform, an in-window focus move commits it; a programmatic edit
//! during a composition commits it first.
//!
//! Task 1b: no composition is ever open (`is_composing` is false, `marked_utf16` `None`,
//! `unmark` does nothing); `text_for_range` reads the buffer; `mark`, `commit` and `blur`
//! arrive in Task 10.

use std::ops::Range;

use crate::buffer::TextBuffer;
use crate::model::{BracketId, Committed, EditorModel};

/// The composition state of one editor.
#[derive(Debug, Clone, Default)]
pub struct ImeCore {
    /// The marked text's bytes, while composing.
    composition: Option<Range<usize>>,
    bracket: Option<BracketId>,
}

impl ImeCore {
    /// Sets the marked text (Task 10).
    pub fn mark(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
        sel16: Option<Range<usize>>,
    ) -> Committed {
        let _ = (m, r16, text, sel16);
        unimplemented!("ImeCore::mark: Task 10")
    }

    /// Commits `text` in place of the composition (or `r16`) (Task 10).
    pub fn commit(
        &mut self,
        m: &mut EditorModel,
        r16: Option<Range<usize>>,
        text: &str,
    ) -> Committed {
        let _ = (m, r16, text);
        unimplemented!("ImeCore::commit: Task 10")
    }

    /// Ends the composition, keeping its text.
    pub fn unmark(&mut self, m: &mut EditorModel) {
        let _ = m;
        self.composition = None;
        self.bracket = None;
    }

    /// The editor lost focus: `true` when the composition was committed (an in-window focus
    /// move), `false` when it stays with the platform (an app switch; Task 10).
    pub fn blur(&mut self, m: &mut EditorModel, window_active: bool) -> bool {
        let _ = (m, window_active);
        unimplemented!("ImeCore::blur: Task 10")
    }

    /// The text of the UTF-16 range `r16`; `adjusted` gets the range actually returned when
    /// `r16` splits a surrogate pair (Task 10 adjusts; Task 1b reads whole characters).
    pub fn text_for_range(
        &self,
        b: &TextBuffer,
        r16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> String {
        let start = b.utf16_to_byte(r16.start);
        let end = b.utf16_to_byte(r16.end).max(start);
        *adjusted = Some(b.byte_to_utf16(start)..b.byte_to_utf16(end));
        b.slice(start..end).into_owned()
    }

    /// The marked text as UTF-16, while composing.
    pub fn marked_utf16(&self, b: &TextBuffer) -> Option<Range<usize>> {
        self.composition
            .as_ref()
            .map(|r| b.byte_to_utf16(r.start)..b.byte_to_utf16(r.end))
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }
}
