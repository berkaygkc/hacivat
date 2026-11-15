//! The scrollbar track's markers (Task 28): where the execution marks and the statement Run
//! would execute sit in the whole document, in the 8 px track at the code area's right edge.
//!
//! - **The track's own hit target** ([`prepaint`]): the text area runs under the track (Task
//!   19's geometry), so the track blocks the pointer from the text below it (no caret placed, no
//!   I-beam over the track) but lets the wheel through (`BlockMouseExceptScroll`).
//! - **Markers** only while the document is taller than the view (the gutter shows everything
//!   otherwise): the current statement as a 2 px `blue` strip on the track's left, each mark as
//!   a 3 px strip on its right, in its gutter hue (`green` succeeded, `text_3` stale or
//!   cancelled, `amber` running or cancelling, `seam_strong` queued, `text` failed, `text_2`
//!   skipped); at least 2 px tall. No red.

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Hitbox, HitboxBehavior, Hsla, Pixels, Window, fill, point, px, size,
};

use crate::gutter::Inks;
use crate::marks::{ExecMarkState, MarkStore};

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

/// The least height of a marker.
const MIN_MARKER: f32 = 2.0;

/// Inserts the track's hit target (prepaint, above the text area's): the pointer stops there,
/// the wheel goes through.
pub fn prepaint(track: Bounds<Pixels>, window: &mut Window) -> Hitbox {
    window.insert_hitbox(track, HitboxBehavior::BlockMouseExceptScroll)
}

/// The track's y span of `lines` (end exclusive) out of `total`.
fn span(track: Bounds<Pixels>, total: u32, lines: Range<u32>) -> Bounds<Pixels> {
    let h = f32::from(track.size.height);
    let total = total.max(1) as f32;
    let y0 = h * lines.start as f32 / total;
    let y1 = (h * lines.end as f32 / total).max(y0 + MIN_MARKER).min(h);
    let y0 = y0.min(h - MIN_MARKER).max(0.0);
    Bounds::new(
        point(track.left(), track.top() + px(y0)),
        size(track.size.width, px(y1 - y0)),
    )
}

/// The marker hue of a mark.
fn hue(state: &ExecMarkState, stale: bool, inks: &Inks) -> Hsla {
    match state {
        ExecMarkState::Succeeded { .. } if stale => inks.text_3,
        ExecMarkState::Succeeded { .. } => inks.green,
        ExecMarkState::Running { .. } | ExecMarkState::Cancelling { .. } => inks.amber,
        ExecMarkState::Queued => inks.seam_strong,
        ExecMarkState::Failed { .. } => inks.text,
        ExecMarkState::Cancelled => inks.text_3,
        ExecMarkState::Skipped => inks.text_2,
    }
}

/// Paints the markers.
pub fn paint(s: &ScrollbarCtx<'_>, m: &MarkStore, window: &mut Window, cx: &mut App) {
    if s.total_lines <= s.visible_lines || s.track.size.height <= px(MIN_MARKER) {
        return;
    }
    let inks = Inks::of(cx);
    let track = s.track;
    if let Some(cur) = &s.current_statement {
        let b = span(track, s.total_lines, cur.clone());
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + px(1.0), b.top()),
                size(px(2.0), b.size.height),
            ),
            inks.blue,
        ));
    }
    let buf = &s.snapshot.buffer;
    for (state, range, stale) in m.markers() {
        let (a, b) = crate::preview::unit_lines(buf, range);
        let r = span(track, s.total_lines, a..b + 1);
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + px(4.0), r.top()),
                size(px(3.0), r.size.height),
            ),
            hue(state, stale, &inks),
        ));
    }
}
