//! The gutter (UI-EDIT-19; Task 28): line numbers from the digit cache, 58 px + (digits − 3) ×
//! advance, the current line's number at `text`, the others `text-3`, and the 1 px `blue`
//! current-statement bar.

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_text::display::ScrollAnchor;
use hacivat_ui_core::gpui_surface::{App, Bounds, Pixels, Window};

/// What the gutter paints in one frame (Task 19 builds it at its paint point; the call site is
/// `element::paint_frame` since Task 1b, fix round 1).
#[derive(Debug, Clone)]
pub struct GutterCtx<'a> {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// Digits of the widest line number.
    pub digits: u8,
    pub current_line: u32,
    pub scroll: ScrollAnchor,
    /// The frame's text: byte offsets (marks, the band) to lines through `snapshot.buffer`.
    pub snapshot: &'a Snapshot,
    /// The lines (0-based, end exclusive) of the statement at the caret, for the 1 px `blue`
    /// bar (`preview::current_statement`); `None`: no bar.
    pub current_statement: Option<Range<u32>>,
}

/// Paints the line numbers and the current-statement bar (Task 28). Task 1b: nothing.
pub fn paint(g: &GutterCtx<'_>, window: &mut Window, cx: &mut App) {
    let _ = (g, window, cx);
}
