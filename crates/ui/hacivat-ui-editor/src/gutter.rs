//! The gutter (UI-EDIT-19; Task 28): line numbers from the digit cache, 58 px + (digits − 3) ×
//! advance, the current line's number at `text`, the others `text-3`.

use hacivat_text::display::ScrollAnchor;
use hacivat_ui_core::gpui_surface::{App, Bounds, Pixels, Window};

/// What the gutter paints in one frame (Task 19 builds it at its paint point).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GutterCtx {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// Digits of the widest line number.
    pub digits: u8,
    pub current_line: u32,
    pub scroll: ScrollAnchor,
}

/// Paints the line numbers (Task 28). Task 1b: nothing.
pub fn paint(g: &GutterCtx, window: &mut Window, cx: &mut App) {
    let _ = (g, window, cx);
}
