//! Multi-cursor selection building (UI-EDIT-05 multi; M3b Task 31): add a caret, add one above
//! or below, select the next or every occurrence, column (box) selection, split a selection
//! into lines, and the selection-only history (the spec's "Ctrl+U undoes the selection").
//!
//! ## m3b-t01b-seams: signatures and stubs (Task 1b). Every stub keeps M3a's single-cursor
//! behaviour: it answers the selection it was given (or one caret), never a selection the
//! caller could not have had before, and never touches the text. Task 31 fills the bodies.

use crate::buffer::TextBuffer;
use crate::model::{Committed, EditorModel};
use crate::selection::SelectionSet;

/// The most ranges [`select_all_occurrences`] makes (spec: 10,000).
pub const MAX_OCCURRENCES: usize = 10_000;

/// `sel` with a caret added at byte `at`. STUB: `sel` unchanged.
pub fn add_caret(sel: &SelectionSet, at: usize) -> SelectionSet {
    let _ = at;
    sel.clone()
}

/// `sel` with a caret added on the line above (`down`: below) each range's head, at its display
/// column (`tab_size` wide tabs). STUB: `sel` unchanged.
pub fn add_caret_vertical(
    buf: &TextBuffer,
    sel: &SelectionSet,
    down: bool,
    tab_size: u8,
) -> SelectionSet {
    let _ = (buf, down, tab_size);
    sel.clone()
}

/// `sel` with the next occurrence of the main range's text added (the word under a caret
/// first); `None` when there is none. STUB: `None`.
pub fn select_next_occurrence(buf: &TextBuffer, sel: &SelectionSet) -> Option<SelectionSet> {
    let _ = (buf, sel);
    None
}

/// Every occurrence of the main range's text, at most `cap` ranges ([`MAX_OCCURRENCES`]).
/// STUB: `sel` unchanged.
pub fn select_all_occurrences(buf: &TextBuffer, sel: &SelectionSet, cap: usize) -> SelectionSet {
    let _ = (buf, cap);
    sel.clone()
}

/// The box from `anchor` to `head`, each `(line, display column)` with `tab_size` wide tabs:
/// one range per line. STUB: one caret at the start of `head`'s line (clamped to the text).
pub fn column_selection(
    buf: &TextBuffer,
    anchor: (u32, usize),
    head: (u32, usize),
    tab_size: u8,
) -> SelectionSet {
    let _ = (anchor, tab_size);
    let last = buf.len_lines().saturating_sub(1);
    let line = usize::try_from(head.0).unwrap_or(usize::MAX).min(last);
    SelectionSet::cursor(buf.line_to_byte(line))
}

/// Each range of `sel` that spans lines split into one range per line. STUB: `sel` unchanged.
pub fn split_into_lines(buf: &TextBuffer, sel: &SelectionSet) -> SelectionSet {
    let _ = buf;
    sel.clone()
}

impl EditorModel {
    /// Takes back the newest selection change (a selection-only history beside the text's).
    /// STUB: `None` (nothing to take back).
    pub fn undo_selection(&mut self) -> Option<Committed> {
        None
    }

    /// Re-applies the newest selection change [`EditorModel::undo_selection`] took back.
    /// STUB: `None`.
    pub fn redo_selection(&mut self) -> Option<Committed> {
        None
    }
}
