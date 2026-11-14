//! The editor element (UI-EDIT-07 paint, -08 scroll; Task 19): one custom element for the text
//! and the gutter area, in the harman paint order, calling `gutter::paint`, `marks::paint`,
//! `preview::paint_band`, `scrollbar::paint` and `a11y::declare` at their paint points; each
//! frame's layout is kept for `EditorView::last_layout` (P11).
//!
//! Since Task 1b's fix round 1, [`paint_frame`] is the one paint path and already calls every
//! module seam with the frame's text, so Tasks 27–29 fill module bodies only. Task 19 replaces
//! its placeholder geometry (a fixed line height, no scrolling, nothing kept for
//! `last_layout`) and keeps the calls.

use hacivat_text::display::ScrollAnchor;
use hacivat_ui_core::gpui_surface::{Bounds, Context, Pixels, Point, Window, point, px, size};

use crate::a11y::{self, A11yFrame};
use crate::gutter::{self, GutterCtx};
use crate::marks;
use crate::preview;
use crate::scrollbar::{self, ScrollbarCtx};
use crate::view::EditorView;

/// The painted rows of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VisibleRows {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// The top of `first_line`, in window coordinates.
    pub first_y: Pixels,
}

/// The last painted frame (Task 19 fills it): what IME bounds and hit testing read.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameLayout {
    rows: VisibleRows,
}

impl FrameLayout {
    /// A frame over `rows` (Task 19 adds the shaped lines).
    pub(crate) fn new(rows: VisibleRows) -> Self {
        FrameLayout { rows }
    }

    /// The rows the frame painted.
    pub fn rows(&self) -> &VisibleRows {
        &self.rows
    }

    /// The x of byte `offset` on a painted line; `None` off the painted lines (Task 19).
    pub fn x_for_offset(&self, offset: usize) -> Option<Pixels> {
        let _ = offset;
        None
    }

    /// The byte offset at `p`; `None` off the painted lines (Task 19).
    pub fn offset_for_point(&self, p: Point<Pixels>) -> Option<usize> {
        let _ = p;
        None
    }
}

/// Task 1b's placeholder geometry (Task 19 measures the font and the gutter digits).
const LINE_HEIGHT: f32 = 20.0;
const GUTTER_WIDTH: f32 = 58.0;
const SCROLLBAR_WIDTH: f32 = 8.0;

/// Paints one frame of `v` into `bounds`, calling every module seam in the harman paint order
/// (Task 19 owns this function and its call sites; the modules own their bodies).
pub(crate) fn paint_frame(
    v: &mut EditorView,
    bounds: Bounds<Pixels>,
    focused: bool,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    crate::ime::on_frame(v, window, cx);
    let snap = v.model.snapshot();
    let line_height = px(LINE_HEIGHT);
    let total_lines = u32::try_from(snap.buffer.len_lines()).unwrap_or(u32::MAX);
    let visible = (f32::from(bounds.size.height) / LINE_HEIGHT)
        .ceil()
        .max(1.0) as u32;
    let last_line = visible.min(total_lines).saturating_sub(1);
    let caret_line =
        u32::try_from(snap.buffer.byte_to_line(snap.selection.main().head)).unwrap_or(u32::MAX);
    let digits = (total_lines.max(1).ilog10() + 1).max(3) as u8;
    let gutter_bounds = Bounds::new(bounds.origin, size(px(GUTTER_WIDTH), bounds.size.height));
    let text_bounds = Bounds::new(
        point(bounds.origin.x + px(GUTTER_WIDTH), bounds.origin.y),
        size(
            (bounds.size.width - px(GUTTER_WIDTH + SCROLLBAR_WIDTH)).max(px(0.0)),
            bounds.size.height,
        ),
    );
    let track = Bounds::new(
        point(
            bounds.origin.x + bounds.size.width - px(SCROLLBAR_WIDTH),
            bounds.origin.y,
        ),
        size(px(SCROLLBAR_WIDTH), bounds.size.height),
    );
    let rows = VisibleRows {
        bounds: text_bounds,
        line_height,
        first_line: 0,
        last_line,
        first_y: bounds.origin.y,
    };
    let layout = FrameLayout::new(rows);
    let current_statement = preview::current_statement(&v.band, &snap);
    let g = GutterCtx {
        bounds: gutter_bounds,
        line_height,
        first_line: 0,
        last_line,
        digits,
        current_line: caret_line,
        scroll: ScrollAnchor::default(),
        snapshot: &snap,
        current_statement: current_statement.clone(),
    };
    // The harman order: gutter, line backgrounds (the band), text (Task 19), marks, scrollbar.
    gutter::paint(&g, window, cx);
    preview::paint_band(&v.band, &rows, &snap, window, cx);
    marks::paint(&v.marks, &g, &layout, window, cx);
    scrollbar::paint(
        &ScrollbarCtx {
            track,
            total_lines,
            first_line: 0,
            visible_lines: visible,
            current_statement,
            snapshot: &snap,
        },
        &v.marks,
        window,
        cx,
    );
    let label = v.a11y_label(cx);
    a11y::declare(
        &mut v.a11y,
        &A11yFrame {
            snapshot: &snap,
            label,
            config: &v.config,
            tier: v.tier.tier,
            focused,
            bounds: text_bounds,
        },
        window,
        cx,
    );
}
