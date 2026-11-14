//! The paint order (Task 19), adapted from harman to `design/m3a-query/NOTES.md` surface 1:
//! the `panel` ground (under the gutter too: the mock gives the gutter no fill of its own),
//! the gutter (`gutter::paint`), the line backgrounds (the Run preview band, `preview::paint_band`),
//! the selection (`blue_tint_strong` focused, `seam_strong` unfocused), the text runs (control
//! chips on `seam_soft`), the IME underline, the marks (`marks::paint`), the carets (2 × 16 px
//! `blue`, 1 px below the row's top, only while focused), the scrollbar markers and the
//! horizontal thumb. Everything inside the text area is clipped to it, so text scrolled sideways
//! never covers the gutter. **No red quad**: Production Red is reserved (DESIGN.md).
//!
//! Also here: the caret blink's clock ([`Blink`]): the platform half-period (530 ms; GPUI
//! exposes no platform blink rate at the pin), solid while typing (every input restarts it), solid
//! again and stopped after 10 s without input, and static under reduced motion (NOTES open
//! question 3).

use std::ops::Range;
use std::time::{Duration, Instant};

use hacivat_text::Snapshot;
use hacivat_ui_core::gpui_surface::{
    App, Bounds, ContentMask, Corners, Pixels, Task, TextAlign, Window, fill, point, px, size,
};

use crate::element::{FrameLayout, PieceKind, RowLayout, TEXT_INSET, VisibleRows};
use crate::preview::{self, BandState};
use crate::theme::EditorTheme;

/// A control chip's inner padding and outer margin (NOTES "Control chip": padding 0 2 px,
/// margin 0 1 px), its height and corner radius.
pub(crate) const CHIP_PAD: f32 = 2.0;
pub(crate) const CHIP_MARGIN: f32 = 1.0;
const CHIP_HEIGHT: f32 = 14.0;
const CHIP_RADIUS: f32 = 3.0;
/// The caret: 2 px wide, the row's height less 2 px (16 px on 18 px rows), 1 px below its top.
pub(crate) const CARET_WIDTH: f32 = 2.0;
/// The horizontal thumb: 4 px tall, 2 px above the bottom, radius 3, at least 24 px wide.
const THUMB_HEIGHT: f32 = 4.0;
const THUMB_BOTTOM: f32 = 2.0;
const THUMB_MIN: f32 = 24.0;
/// The IME underline sits this far under the baseline (NOTES surface 1).
const IME_UNDERLINE_GAP: f32 = 3.0;

/// The code area's ground.
pub(crate) fn background(bounds: Bounds<Pixels>, theme: &EditorTheme, window: &mut Window) {
    window.paint_quad(fill(bounds, theme.background));
}

/// The window x of content x `x` (from a line's start).
fn wx(layout: &FrameLayout, x: f32) -> f32 {
    (layout.origin_x + f64::from(x)) as f32
}

/// The text area, clipped to itself: the band, the selection, the text, the IME underline.
#[allow(
    clippy::too_many_arguments,
    reason = "one paint pass over the frame's parts"
)]
pub(crate) fn text_area(
    layout: &FrameLayout,
    snapshot: &Snapshot,
    band: &BandState,
    theme: &EditorTheme,
    focused: bool,
    marked: Option<Range<usize>>,
    window: &mut Window,
    cx: &mut App,
) {
    let area = layout.rows().bounds;
    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
        preview::paint_band(band, layout.rows(), snapshot, window, cx);
        selections(layout, snapshot, theme, focused, window);
        text(layout, theme, window, cx);
        if let Some(m) = marked {
            ime_underline(layout, snapshot, m, theme, window);
        }
    });
}

/// The content x span of `r` on `row` (the selected line break counts half an advance).
pub(crate) fn span_on_row(
    layout: &FrameLayout,
    row: &RowLayout,
    r: &Range<usize>,
) -> Option<(f32, f32)> {
    let lb = &row.line_bytes;
    if r.start > lb.end || r.end < lb.start {
        return None;
    }
    let x0 = if r.start >= lb.start {
        layout.row_x(row, r.start)
    } else {
        0.0
    };
    let x1 = if r.end <= lb.end {
        layout.row_x(row, r.end)
    } else {
        row.width + layout.advance / 2.0
    };
    (x1 > x0).then_some((x0, x1))
}

fn selections(
    layout: &FrameLayout,
    snapshot: &Snapshot,
    theme: &EditorTheme,
    focused: bool,
    window: &mut Window,
) {
    let color = if focused {
        theme.selection
    } else {
        theme.selection_unfocused
    };
    let lh = layout.rows().line_height;
    for sel in snapshot.selection.ranges() {
        let r = sel.range();
        if r.is_empty() {
            continue;
        }
        for row in &layout.lines {
            if let Some((x0, x1)) = span_on_row(layout, row, &r) {
                window.paint_quad(fill(
                    Bounds::new(point(px(wx(layout, x0)), row.top), size(px(x1 - x0), lh)),
                    color,
                ));
            }
        }
    }
}

fn text(layout: &FrameLayout, theme: &EditorTheme, window: &mut Window, cx: &mut App) {
    let area = layout.rows().bounds;
    let lh = layout.rows().line_height;
    for row in &layout.lines {
        for piece in &row.pieces {
            let x = wx(layout, piece.x);
            if x + piece.width < f32::from(area.left()) || x > f32::from(area.right()) {
                continue;
            }
            match &piece.kind {
                PieceKind::Text { shaped, .. } => {
                    let _ =
                        shaped.paint(point(px(x), row.top), lh, TextAlign::Left, None, window, cx);
                }
                PieceKind::Chip { label } => {
                    let chip = Bounds::new(
                        point(px(x + CHIP_MARGIN), row.top + (lh - px(CHIP_HEIGHT)) / 2.0),
                        size(label.width() + px(CHIP_PAD * 2.0), px(CHIP_HEIGHT)),
                    );
                    window.paint_quad(
                        fill(chip, theme.chip_background)
                            .corner_radii(Corners::all(px(CHIP_RADIUS))),
                    );
                    let _ = label.paint(
                        point(px(x + CHIP_MARGIN + CHIP_PAD), row.top),
                        lh,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        }
    }
}

/// The baseline of `row`, from its first shaped piece (GPUI centres a line in its row).
fn baseline(row: &RowLayout, lh: Pixels) -> Pixels {
    row.pieces
        .iter()
        .find_map(|p| match &p.kind {
            PieceKind::Text { shaped, .. } => {
                Some(row.top + (lh - shaped.ascent - shaped.descent) / 2.0 + shaped.ascent)
            }
            PieceKind::Chip { .. } => None,
        })
        .unwrap_or(row.top + lh * 0.75)
}

/// The composition's 1 px `text` underline 3 px under the baseline; the active clause (the
/// selection inside it) 2 px.
fn ime_underline(
    layout: &FrameLayout,
    snapshot: &Snapshot,
    marked: Range<usize>,
    theme: &EditorTheme,
    window: &mut Window,
) {
    let lh = layout.rows().line_height;
    let main = snapshot.selection.main().range();
    let clause =
        (!main.is_empty() && main.start >= marked.start && main.end <= marked.end).then_some(main);
    for row in &layout.lines {
        let base = baseline(row, lh) + px(IME_UNDERLINE_GAP);
        if let Some((x0, x1)) = span_on_row(layout, row, &marked) {
            window.paint_quad(fill(
                Bounds::new(point(px(wx(layout, x0)), base), size(px(x1 - x0), px(1.0))),
                theme.ime_underline,
            ));
        }
        if let Some(c) = &clause
            && let Some((x0, x1)) = span_on_row(layout, row, c)
        {
            window.paint_quad(fill(
                Bounds::new(point(px(wx(layout, x0)), base), size(px(x1 - x0), px(2.0))),
                theme.ime_underline,
            ));
        }
    }
}

/// The caret of `head`: 2 px wide, the row's height less 2 px, 1 px below the row's top,
/// centred on the boundary (window coordinates).
pub(crate) fn caret_rect(layout: &FrameLayout, head: usize) -> Option<Bounds<Pixels>> {
    let line = u32::try_from(layout.buffer.byte_to_line(head)).ok()?;
    let row = layout.row(line)?;
    let x = wx(layout, layout.row_x(row, head)) - CARET_WIDTH / 2.0;
    let lh = layout.rows().line_height;
    Some(Bounds::new(
        point(px(x), row.top + px(1.0)),
        size(px(CARET_WIDTH), lh - px(2.0)),
    ))
}

/// Every caret, clipped to the text area.
pub(crate) fn carets(
    layout: &FrameLayout,
    snapshot: &Snapshot,
    theme: &EditorTheme,
    window: &mut Window,
) {
    let area = layout.rows().bounds;
    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
        for sel in snapshot.selection.ranges() {
            if let Some(r) = caret_rect(layout, sel.head) {
                window.paint_quad(fill(r, theme.caret));
            }
        }
    });
}

/// The horizontal thumb when the widest painted row is wider than the view.
pub(crate) fn horizontal_thumb(
    rows: &VisibleRows,
    content_width: f64,
    scroll_x: f64,
    theme: &EditorTheme,
    window: &mut Window,
) {
    let track = rows.bounds;
    let view_w = f64::from(f32::from(track.size.width) - TEXT_INSET);
    if content_width <= view_w || view_w <= 0.0 {
        return;
    }
    let track_w = f32::from(track.size.width);
    let thumb_w = ((view_w / content_width) as f32 * track_w)
        .max(THUMB_MIN)
        .min(track_w);
    let t = (scroll_x / (content_width - view_w)).clamp(0.0, 1.0) as f32;
    let x = f32::from(track.left()) + (track_w - thumb_w) * t;
    let y = f32::from(track.bottom()) - THUMB_BOTTOM - THUMB_HEIGHT;
    window.paint_quad(
        fill(
            Bounds::new(point(px(x), px(y)), size(px(thumb_w), px(THUMB_HEIGHT))),
            theme.thumb,
        )
        .corner_radii(Corners::all(px(3.0))),
    );
}

/// The blink's half-period: the platform's (macOS: 530 ms, also the fallback).
pub(crate) const BLINK_HALF: Duration = Duration::from_millis(530);
/// After this long without input the caret goes solid and the blink stops.
pub(crate) const BLINK_IDLE: Duration = Duration::from_secs(10);

/// The caret blink's state: hidden half the time while idle, never while typing.
#[derive(Default)]
pub(crate) struct Blink {
    hidden: bool,
    last_input: Option<Instant>,
    /// Bumped by every input and stop: an older timer's tick does nothing.
    epoch: u64,
    /// The running half-period timer (dropping it cancels it).
    pub task: Option<Task<()>>,
}

impl std::fmt::Debug for Blink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blink")
            .field("hidden", &self.hidden)
            .field("epoch", &self.epoch)
            .field("running", &self.task.is_some())
            .finish_non_exhaustive()
    }
}

impl Blink {
    /// The caret is drawn this frame.
    pub fn visible(&self) -> bool {
        !self.hidden
    }

    /// An input at `now`: the caret shows and the half-period starts again. Returns the epoch
    /// the new timer runs under.
    pub fn restart(&mut self, now: Instant) -> u64 {
        self.hidden = false;
        self.last_input = Some(now);
        self.epoch += 1;
        self.epoch
    }

    /// One half-period of `epoch` ended at `now`: `true` to keep blinking.
    pub fn tick(&mut self, epoch: u64, now: Instant) -> bool {
        if epoch != self.epoch {
            return false;
        }
        let idle = self
            .last_input
            .is_none_or(|t| now.saturating_duration_since(t) >= BLINK_IDLE);
        if idle {
            self.hidden = false;
            self.task = None;
            return false;
        }
        self.hidden = !self.hidden;
        true
    }

    /// The caret goes solid and nothing blinks (blur, reduced motion, blink off).
    pub fn stop(&mut self) {
        self.hidden = false;
        self.epoch += 1;
        self.task = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blink_toggles_until_ten_seconds_idle_and_typing_keeps_it_solid() {
        let t0 = Instant::now();
        let mut b = Blink::default();
        let e = b.restart(t0);
        assert!(b.visible());
        assert!(b.tick(e, t0 + BLINK_HALF));
        assert!(!b.visible());
        assert!(b.tick(e, t0 + BLINK_HALF * 2));
        assert!(b.visible());
        // An input restarts it: the old timer's tick does nothing.
        let t1 = t0 + BLINK_HALF * 3;
        let e2 = b.restart(t1);
        assert!(!b.tick(e, t1 + BLINK_HALF), "a stale timer stops");
        assert!(b.visible());
        // Ten seconds after the last input: solid, and the timer stops.
        assert!(!b.tick(e2, t1 + BLINK_IDLE));
        assert!(b.visible());
        b.stop();
        assert!(!b.tick(e2, t1 + BLINK_HALF), "stopped");
    }
}
