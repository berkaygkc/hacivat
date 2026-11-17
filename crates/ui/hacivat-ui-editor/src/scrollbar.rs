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
//!
//! ## m3c-t33-find (sole owner in W9, P-M3b-17)
//!
//! - **By visible row** (T32 carry): every marker sits at its line's visible row out of the
//!   rows the view shows ([`row_span`]); a line a fold hides sits on its fold's header row, and
//!   "taller than the view" counts visible rows.
//! - **Find ticks** (NOTES surface 16, M3c-10): a 2 px tick across the track (1 px in from
//!   each side) per kept match, `text_2`, the current one (the main selection) `blue`, painted
//!   over the marks; one quad per run of pixel rows, so 100,000 matches never paint more quads
//!   than the track has rows.
//! - **The horizontal thumb drags** (moved from Task 34, ledger 207): [`h_thumb`] is the
//!   thumb's box (the paint's too), [`prepaint_thumb`] its hit target (the thumb's width, the
//!   bottom [`THUMB_HIT`] px of the text area: the text under it takes no press there), and
//!   [`listen_thumb`] moves `scroll_x` with the pointer while the left button is down.
//!   The track keeps its 8 px width: the mock's 9 px would move every M3a marker shot (logged).

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_text::display::HeightMap;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, CursorStyle, Entity, Hitbox, HitboxBehavior, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Window, fill, point, px, size,
};

use crate::element::{FramePrepaint, TEXT_INSET};
use crate::gutter::Inks;
use crate::marks::{ExecMarkState, MarkStore};
use crate::paint::{THUMB_BOTTOM, THUMB_HEIGHT, THUMB_MIN};
use crate::view::EditorView;

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
    // ## m3c-t33-find
    /// The frame's height map: markers sit by visible row.
    pub map: &'a HeightMap,
    /// The kept find matches (byte ranges, in order).
    pub find: &'a [Range<usize>],
}

/// The least height of a marker.
const MIN_MARKER: f32 = 2.0;
/// A find tick's height.
const TICK: f32 = 2.0;
/// The thumb's hit target: this tall, from the text area's bottom.
pub const THUMB_HIT: f32 = 10.0;

/// Inserts the track's hit target (prepaint, above the text area's): the pointer stops there,
/// the wheel goes through.
pub fn prepaint(track: Bounds<Pixels>, window: &mut Window) -> Hitbox {
    window.insert_hitbox(track, HitboxBehavior::BlockMouseExceptScroll)
}

/// The track's y span of the lines `lines` (end exclusive), by visible row out of the rows the
/// map shows (a hidden line on its fold's header row); at least [`MIN_MARKER`] tall.
pub fn row_span(track: Bounds<Pixels>, map: &HeightMap, lines: Range<u32>) -> Bounds<Pixels> {
    let h = f32::from(track.size.height);
    let total = map.visible_rows().max(1) as f32;
    let first = map.row_of(lines.start);
    let last = map
        .row_of(lines.end.saturating_sub(1).max(lines.start))
        .max(first);
    let y0 = h * first as f32 / total;
    let y1 = (h * (last + 1) as f32 / total).max(y0 + MIN_MARKER).min(h);
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
        // ## m3b-t01b-seams: Escalated keeps the Cancelling hue.
        ExecMarkState::Running { .. }
        | ExecMarkState::Cancelling { .. }
        | ExecMarkState::Escalated { .. } => inks.amber,
        ExecMarkState::Queued => inks.seam_strong,
        ExecMarkState::Failed { .. } => inks.text,
        ExecMarkState::Cancelled => inks.text_3,
        ExecMarkState::Skipped => inks.text_2,
    }
}

/// Paints the markers.
pub fn paint(s: &ScrollbarCtx<'_>, m: &MarkStore, window: &mut Window, cx: &mut App) {
    // ## m3c-t33-find: "taller than the view" in visible rows.
    if s.map.visible_rows() <= s.visible_lines || s.track.size.height <= px(MIN_MARKER) {
        return;
    }
    let inks = Inks::of(cx);
    let track = s.track;
    if let Some(cur) = &s.current_statement {
        let b = row_span(track, s.map, cur.clone());
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
        let r = row_span(track, s.map, a..b + 1);
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
    paint_ticks(s, &inks, window);
}

/// The find ticks (module docs): one per kept match, the main selection's in blue.
fn paint_ticks(s: &ScrollbarCtx<'_>, inks: &Inks, window: &mut Window) {
    if s.find.is_empty() {
        return;
    }
    let track = s.track;
    let height = f32::from(track.size.height);
    let rows = height.ceil() as usize;
    // 0: nothing, 1: a match, 2: the current one.
    let mut at = vec![0u8; rows];
    let buf = &s.snapshot.buffer;
    let main = s.snapshot.selection.main().range();
    let line_of = |b: usize| u32::try_from(buf.byte_to_line(b)).unwrap_or(u32::MAX);
    let mut last: Option<(u32, usize)> = None;
    for r in s.find {
        let line = line_of(r.start);
        let row = match last {
            Some((l, row)) if l == line => row,
            _ => {
                let b = row_span(track, s.map, line..line + 1);
                let y = f32::from(b.top() - track.top()).min(height - TICK).max(0.0);
                y.floor() as usize
            }
        };
        last = Some((line, row));
        let kind = if *r == main { 2 } else { 1 };
        if let Some(slot) = at.get_mut(row) {
            *slot = (*slot).max(kind);
        }
    }
    let mut row = 0;
    while row < rows {
        let kind = at[row];
        if kind == 0 {
            row += 1;
            continue;
        }
        let start = row;
        while row < rows && at[row] == kind {
            row += 1;
        }
        let color = if kind == 2 { inks.blue } else { inks.text_2 };
        let y1 = ((row - 1) as f32 + TICK).min(height);
        window.paint_quad(fill(
            Bounds::new(
                point(track.left() + px(1.0), track.top() + px(start as f32)),
                size(track.size.width - px(2.0), px(y1 - start as f32)),
            ),
            color,
        ));
    }
}

// ---- the horizontal thumb ---------------------------------------------------------------------

/// The horizontal thumb of one frame: its box, how far it travels, and the scroll it spans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HThumb {
    pub bounds: Bounds<Pixels>,
    /// The track's left edge.
    left: f32,
    /// How far the thumb's left edge travels (the track less the thumb).
    travel: f32,
    /// The scroll range (`content_width` less the view's width).
    range: f64,
}

/// The thumb over the text area `area` when the widest painted row (`content_width`) is wider
/// than the view; `None` otherwise.
pub fn h_thumb(area: Bounds<Pixels>, content_width: f64, scroll_x: f64) -> Option<HThumb> {
    let view_w = f64::from(f32::from(area.size.width) - TEXT_INSET);
    if content_width <= view_w || view_w <= 0.0 {
        return None;
    }
    let track_w = f32::from(area.size.width);
    let thumb_w = ((view_w / content_width) as f32 * track_w)
        .max(THUMB_MIN)
        .min(track_w);
    let range = content_width - view_w;
    let t = (scroll_x / range).clamp(0.0, 1.0) as f32;
    let left = f32::from(area.left());
    let travel = track_w - thumb_w;
    let x = left + travel * t;
    let y = f32::from(area.bottom()) - THUMB_BOTTOM - THUMB_HEIGHT;
    Some(HThumb {
        bounds: Bounds::new(point(px(x), px(y)), size(px(thumb_w), px(THUMB_HEIGHT))),
        left,
        travel,
        range,
    })
}

/// A drag of the horizontal thumb in progress (`EditorView::hthumb`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ThumbDrag {
    /// The pointer's x less the thumb's left edge at the press, and the thumb's geometry then.
    grab: Option<(f32, HThumb)>,
}

/// The thumb's hit target: its width, the bottom [`THUMB_HIT`] px of the text area. A press
/// there never reaches the text below.
pub(crate) fn prepaint_thumb(
    area: Bounds<Pixels>,
    content_width: f64,
    scroll_x: f64,
    window: &mut Window,
) -> Option<Hitbox> {
    let thumb = h_thumb(area, content_width, scroll_x)?;
    let hit = Bounds::new(
        point(thumb.bounds.left(), area.bottom() - px(THUMB_HIT)),
        size(thumb.bounds.size.width, px(THUMB_HIT)),
    );
    Some(window.insert_hitbox(hit, HitboxBehavior::BlockMouseExceptScroll))
}

/// The thumb's pointer listeners: a left press on it starts a drag, a move with the button down
/// scrolls sideways, the release ends it.
pub(crate) fn listen_thumb(view: &Entity<EditorView>, frame: &FramePrepaint, window: &mut Window) {
    let Some(hit) = frame.thumb_hit.clone() else {
        // No thumb this frame: a drag in progress still ends on release.
        listen_release(view, window);
        return;
    };
    window.set_cursor_style(CursorStyle::Arrow, &hit);
    let thumb = h_thumb(frame.rows.bounds, frame.content_width, frame.scroll_x);
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
        if !phase.bubble() || ev.button != MouseButton::Left || !hit.is_hovered(window) {
            return;
        }
        let Some(thumb) = thumb else {
            return;
        };
        let grab = f32::from(ev.position.x) - f32::from(thumb.bounds.left());
        v.update(cx, |v, _| v.hthumb.grab = Some((grab, thumb)));
        cx.stop_propagation();
    });
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _window, cx| {
        if !phase.bubble() {
            return;
        }
        v.update(cx, |v, cx| {
            let Some((grab, thumb)) = v.hthumb.grab else {
                return;
            };
            if ev.pressed_button != Some(MouseButton::Left) {
                v.hthumb.grab = None;
                return;
            }
            let left = f32::from(ev.position.x) - grab;
            let t = if thumb.travel > 0.0 {
                ((left - thumb.left) / thumb.travel).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let x = f64::from(t) * thumb.range;
            if (v.scroll.scroll_x - x).abs() > f64::EPSILON {
                v.scroll.scroll_x = x;
                cx.notify();
            }
        });
    });
    listen_release(view, window);
}

/// The release that ends a thumb drag.
fn listen_release(view: &Entity<EditorView>, window: &mut Window) {
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseUpEvent, phase, _window, cx| {
        if phase.bubble() && ev.button == MouseButton::Left {
            v.update(cx, |v, _| v.hthumb.grab = None);
        }
    });
}
