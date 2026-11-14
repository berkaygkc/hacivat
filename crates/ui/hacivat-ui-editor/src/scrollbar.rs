//! Scrollbar markers for execution marks and the current statement (Task 28).

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{App, Bounds, Pixels, Window};

use crate::marks::MarkStore;

/// What the scrollbar paints in one frame (Task 19 builds it).
#[derive(Debug, Clone, PartialEq)]
pub struct ScrollbarCtx {
    pub track: Bounds<Pixels>,
    pub total_lines: u32,
    pub first_line: u32,
    pub visible_lines: u32,
    /// The current statement's lines.
    pub current_statement: Option<Range<u32>>,
}

/// Paints the markers (Task 28). Task 1b: nothing.
pub fn paint(s: &ScrollbarCtx, m: &MarkStore, window: &mut Window, cx: &mut App) {
    let _ = (s, m, window, cx);
}
