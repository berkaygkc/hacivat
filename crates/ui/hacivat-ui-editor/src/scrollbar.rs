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
//!   skipped); at least 2 px tall. No red. A running mark shows nothing for its first 50 ms, as
//!   in the gutter. Each track pixel row shows the worst mark that covers it, and runs of rows
//!   in one hue paint as one marker (Task 35, T28 review N1).

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
    // ## m3a-t35-shots (T28 review N1; replaces fix round 1's merge by overlap, which chained
    // through adjacent markers: one failure in a long run painted the whole stretch in ink).
    // Each mark claims the whole track pixel rows it covers; a row keeps the worst severity
    // that claims it; runs of rows in one hue paint as one quad. So 10,000 marks never paint
    // more quads than the track has rows, and a failure keeps exactly its own rows.
    let buf = &s.snapshot.buffer;
    let now = cx.background_executor().now();
    let height = f32::from(track.size.height);
    let rows = height.ceil() as usize;
    let mut worst: Vec<Option<(u8, Hsla)>> = vec![None; rows];
    for (state, range, stale, severity) in m.markers(now) {
        let (a, b) = crate::preview::unit_lines(buf, range);
        let r = span(track, s.total_lines, a..b + 1);
        let y0 = f32::from(r.top() - track.top());
        let y1 = y0 + f32::from(r.size.height);
        let (first, end) = (y0.floor() as usize, (y1.ceil() as usize).min(rows));
        let color = hue(state, stale, &inks);
        for row in &mut worst[first.min(end)..end] {
            if row.is_none_or(|(sev, _)| severity > sev) {
                *row = Some((severity, color));
            }
        }
    }
    let mut row = 0;
    while row < rows {
        let Some((_, color)) = worst[row] else {
            row += 1;
            continue;
        };
        let start = row;
        while row < rows && worst[row].is_some_and(|(_, c)| c == color) {
            row += 1;
        }
        let y1 = (row as f32).min(height);
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + px(4.0), track.top() + px(start as f32)),
                size(px(3.0), px(y1 - start as f32)),
            ),
            color,
        ));
    }
}
