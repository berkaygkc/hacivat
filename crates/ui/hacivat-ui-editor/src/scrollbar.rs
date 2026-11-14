//! Scrollbar markers for execution marks and the current statement (Task 28).

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{App, Bounds, Pixels, Window};

use crate::marks::MarkStore;

/// What the scrollbar paints in one frame (Task 19 builds it; the call site is
/// `element::paint_frame` since Task 1b, fix round 1).
#[derive(Debug, Clone)]
pub struct ScrollbarCtx<'a> {
    pub track: Bounds<Pixels>,
    pub total_lines: u32,
    pub first_line: u32,
    pub visible_lines: u32,
    /// The current statement's lines (0-based, end exclusive).
    pub current_statement: Option<Range<u32>>,
    /// The frame's text: a mark's byte range to its lines through `snapshot.buffer`.
    pub snapshot: &'a Snapshot,
}

/// Paints the markers (Task 28). Task 1b: nothing.
pub fn paint(s: &ScrollbarCtx<'_>, m: &MarkStore, window: &mut Window, cx: &mut App) {
    let _ = (s, m, window, cx);
}
