//! Row-anchored two-axis scrolling (HV-GRID-02; Task 17): the M0 spike's `ScrollPos` math
//! (`spikes/spike-grid/src/scroll.rs`) as [`ScrollState`], plus follow-tail, shrink clamping,
//! `scroll_into_view`, `f64` thumb drags with exact extremes, deltas coalesced per frame and the
//! fast-scroll velocity.
//!
//! No absolute pixel position is computed in `f32`: the vertical position is a `u64` row plus an
//! `f32` offset inside it, every move is computed relative to that row in `f64`, and only
//! positions inside the viewport (small numbers) are `f32`.

use std::ops::Range;

/// The grid's row height when nothing else is configured (`GridConfig::row_height`).
pub const DEFAULT_ROW_HEIGHT: f32 = 21.0;

/// The shortest scrollbar thumb, in logical px.
pub const MIN_THUMB_PX: f32 = 24.0;

/// Rows past each edge of the viewport that prefetch hints cover (never painted).
pub const OVERSCAN_ROWS: u64 = 8;

/// A thumb this close to either end of its track (in px) is at that end: an `f32` track length
/// widened to `f64` never misses the first or last row by a rounding error.
const THUMB_END_EPSILON: f64 = 1e-3;

/// Where the grid is scrolled, and the geometry it is scrolled over.
///
/// The position is the top row and the offset into it (`[0, row height)`), and the horizontal
/// offset `left_x` over the column prefix sums. `follow_tail` is set while the user is at the
/// bottom (or chose End): rows streaming in then keep the view at the bottom.
///
/// The public fields are the position; every method re-establishes the invariants, so a field
/// written from outside is clamped by the next call.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollState {
    pub top_row: u64,
    pub top_row_offset: f32,
    pub left_x: f64,
    pub follow_tail: bool,
    row_h: f32,
    viewport_w: f32,
    viewport_h: f32,
    rows: u64,
    content_w: f64,
    pending_dy: f64,
    pending_dx: f64,
    /// The position at the last `apply_pending` (the previous frame), for the velocity.
    frame_top: u64,
    frame_offset: f32,
    /// A thumb drag or jump since the last frame: the next frame's velocity is infinite.
    jumped: bool,
    velocity: f64,
}

impl Default for ScrollState {
    fn default() -> Self {
        ScrollState::new(DEFAULT_ROW_HEIGHT)
    }
}

impl ScrollState {
    /// At the top of an empty grid of `row_height` px rows (an invalid height takes the
    /// default).
    pub fn new(row_height: f32) -> Self {
        ScrollState {
            top_row: 0,
            top_row_offset: 0.0,
            left_x: 0.0,
            follow_tail: false,
            row_h: if valid_len(row_height) && row_height > 0.0 {
                row_height
            } else {
                DEFAULT_ROW_HEIGHT
            },
            viewport_w: 0.0,
            viewport_h: 0.0,
            rows: 0,
            content_w: 0.0,
            pending_dy: 0.0,
            pending_dx: 0.0,
            frame_top: 0,
            frame_offset: 0.0,
            jumped: false,
            velocity: 0.0,
        }
    }

    pub fn row_height(&self) -> f32 {
        self.row_h
    }

    /// A new row height (zoom, font, density): the top row stays, the offset into it scales.
    /// Heights that are not finite and positive are ignored.
    pub fn set_row_height(&mut self, height: f32) {
        if !valid_len(height) || height <= 0.0 || height == self.row_h {
            return;
        }
        let ratio = f64::from(height) / f64::from(self.row_h);
        let offset = f64::from(self.sane_offset()) * ratio;
        let frame_offset = f64::from(self.frame_offset) * ratio;
        self.row_h = height;
        self.top_row_offset = 0.0;
        self.move_by(offset);
        self.frame_offset = (frame_offset as f32).clamp(0.0, height);
    }

    /// The body's size (the cells' area: no header, gutter or scrollbars).
    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport_w = if valid_len(width) { width } else { 0.0 };
        self.viewport_h = if valid_len(height) { height } else { 0.0 };
        self.clamp();
    }

    /// `(width, height)` of the body.
    pub fn viewport(&self) -> (f32, f32) {
        (self.viewport_w, self.viewport_h)
    }

    /// The rows the grid scrolls over.
    pub fn row_count(&self) -> u64 {
        self.rows
    }

    /// The row count changed. Growth while following the tail keeps the view at the bottom;
    /// otherwise the top row stays. A shrink clamps the view to the new last page.
    pub fn on_row_count(&mut self, rows: u64) {
        let grew = rows > self.rows;
        self.rows = rows;
        if grew && self.follow_tail {
            self.top_row = self.max_top_row();
            self.top_row_offset = 0.0;
        }
        self.clamp();
    }

    /// The width of the scrollable columns (`ColumnLayout::total_width`).
    pub fn content_width(&self) -> f64 {
        self.content_w
    }

    pub fn set_content_width(&mut self, width: f64) {
        self.content_w = if width.is_finite() {
            width.max(0.0)
        } else {
            0.0
        };
        self.left_x = self.clamp_x(self.left_x);
    }

    /// Whole rows that fit in the body (at least one).
    pub fn viewport_rows(&self) -> u64 {
        ((f64::from(self.viewport_h) / f64::from(self.row_h)).floor() as u64).max(1)
    }

    /// The largest top row: the last page shows the final row whole, with offset 0.
    pub fn max_top_row(&self) -> u64 {
        self.rows.saturating_sub(self.viewport_rows())
    }

    /// The largest `left_x`.
    pub fn max_left_x(&self) -> f64 {
        (self.content_w - f64::from(self.viewport_w)).max(0.0)
    }

    /// Rows that intersect the body, a partial last row included.
    pub fn visible_rows(&self) -> Range<u64> {
        let start = self.top_row.min(self.rows);
        let span = ((f64::from(self.sane_offset()) + f64::from(self.viewport_h))
            / f64::from(self.row_h))
        .ceil() as u64;
        start..start.saturating_add(span).min(self.rows)
    }

    /// The visible rows widened by [`OVERSCAN_ROWS`] on each side, for prefetch hints
    /// (`ensure_rows`). The end is not clamped to the row count, so a hint at the bottom of the
    /// loaded rows reaches past them.
    pub fn prefetch_rows(&self) -> Range<u64> {
        let v = self.visible_rows();
        v.start.saturating_sub(OVERSCAN_ROWS)..v.end.saturating_add(OVERSCAN_ROWS)
    }

    /// The top edge of `row` relative to the body's top (negative above it). Exact for every
    /// row: computed from the row distance to the anchor, in `f64`.
    pub fn row_top(&self, row: u64) -> f64 {
        (i128::from(row) - i128::from(self.top_row)) as f64 * f64::from(self.row_h)
            - f64::from(self.sane_offset())
    }

    /// The row under `y` (relative to the body's top); `None` above the body or past the last
    /// row.
    pub fn row_at(&self, y: f32) -> Option<u64> {
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        let rel = ((f64::from(y) + f64::from(self.sane_offset())) / f64::from(self.row_h)).floor();
        let row = self.top_row.saturating_add(rel as u64);
        (row < self.rows).then_some(row)
    }

    /// Scrolls by `dy` px now (positive towards later rows). Non-finite deltas move nothing.
    pub fn scroll_by(&mut self, dy: f64) {
        if dy.is_finite() {
            self.move_by(dy);
            self.after_user_move(true);
        }
    }

    /// Scrolls horizontally by `dx` px now (positive towards later columns).
    pub fn scroll_x_by(&mut self, dx: f64) {
        if dx.is_finite() {
            self.left_x = self.clamp_x(self.left_x + dx);
        }
    }

    pub fn set_left_x(&mut self, x: f64) {
        if x.is_finite() {
            self.left_x = self.clamp_x(x);
        }
    }

    /// Queues a wheel or trackpad delta; every delta queued in one frame is applied together by
    /// [`ScrollState::apply_pending`] (scroll storms at 120 Hz cost one move per frame).
    pub fn queue(&mut self, dy: f64, dx: f64) {
        if dy.is_finite() {
            self.pending_dy = (self.pending_dy + dy).clamp(-f64::MAX, f64::MAX);
        }
        if dx.is_finite() {
            self.pending_dx = (self.pending_dx + dx).clamp(-f64::MAX, f64::MAX);
        }
    }

    /// Applies the frame's queued deltas and measures the frame's velocity. Called once per
    /// frame (prepaint); answers whether the queued deltas moved the view.
    pub fn apply_pending(&mut self) -> bool {
        let before = (self.top_row, self.top_row_offset, self.left_x);
        let (dy, dx) = (self.pending_dy, self.pending_dx);
        self.pending_dy = 0.0;
        self.pending_dx = 0.0;
        if dy != 0.0 {
            self.move_by(dy);
            self.after_user_move(true);
        }
        if dx != 0.0 {
            self.left_x = self.clamp_x(self.left_x + dx);
        }
        self.velocity = if self.jumped {
            f64::INFINITY
        } else {
            let rows = (i128::from(self.top_row) - i128::from(self.frame_top)) as f64
                + f64::from(self.top_row_offset - self.frame_offset) / f64::from(self.row_h);
            rows.abs()
        };
        self.jumped = false;
        self.frame_top = self.top_row;
        self.frame_offset = self.top_row_offset;
        before != (self.top_row, self.top_row_offset, self.left_x)
    }

    /// Rows moved between the last two frames (`apply_pending`), every kind of move included;
    /// infinite for the one frame after a thumb jump. Fast-scroll (Task 20) compares it with its
    /// threshold.
    pub fn velocity_rows_per_frame(&self) -> f64 {
        self.velocity
    }

    /// Home: the first row; the tail is no longer followed.
    pub fn scroll_to_top(&mut self) {
        self.top_row = 0;
        self.top_row_offset = 0.0;
        self.follow_tail = false;
    }

    /// End: the last page, and the view follows rows streaming in.
    pub fn scroll_to_end(&mut self) {
        self.top_row = self.max_top_row();
        self.top_row_offset = 0.0;
        self.follow_tail = true;
    }

    /// Reveals `row` (keyboard, go to row, find): a row above the body becomes the top row; a
    /// row below it ends at the body's bottom edge; a visible row moves nothing. Rows past the
    /// end reveal the last row.
    pub fn scroll_into_view(&mut self, row: u64) {
        if self.rows == 0 {
            return;
        }
        let row = row.min(self.rows - 1);
        let offset = self.sane_offset();
        if row < self.top_row || (row == self.top_row && offset > 0.0) {
            self.top_row = row;
            self.top_row_offset = 0.0;
            self.clamp();
        } else {
            let bottom = self.row_top(row) + f64::from(self.row_h);
            let body = f64::from(self.viewport_h);
            if bottom > body {
                if self.row_h >= self.viewport_h {
                    // Taller than the body: show its start.
                    self.top_row = row;
                    self.top_row_offset = 0.0;
                    self.clamp();
                } else {
                    self.move_by(bottom - body);
                }
            }
        }
        self.after_user_move(false);
    }

    /// Reveals the content span `[left, right)` horizontally; a span wider than the body shows
    /// its left edge.
    pub fn scroll_x_into_view(&mut self, left: f64, right: f64) {
        if !(left.is_finite() && right.is_finite()) {
            return;
        }
        let width = f64::from(self.viewport_w);
        if left < self.left_x || right - left > width {
            self.left_x = self.clamp_x(left);
        } else if right > self.left_x + width {
            self.left_x = self.clamp_x(right - width);
        }
    }

    /// Jumps to the fraction `p` of the scroll range, in `f64`: `p ≤ 0` is exactly row 0 and
    /// `p ≥ 1` exactly the last page. A non-finite `p` moves nothing. Counts as a jump for the
    /// velocity.
    pub fn scroll_to_fraction(&mut self, p: f64) {
        if !p.is_finite() {
            return;
        }
        let max_top = self.max_top_row();
        if p <= 0.0 {
            self.top_row = 0;
            self.top_row_offset = 0.0;
        } else if p >= 1.0 {
            self.top_row = max_top;
            self.top_row_offset = 0.0;
        } else {
            let row_h = f64::from(self.row_h);
            let virtual_y = p * (max_top as f64 * row_h);
            let top = (virtual_y / row_h).floor();
            self.top_row = top as u64;
            self.top_row_offset = 0.0;
            self.move_by(virtual_y - top * row_h);
        }
        self.jumped = true;
        self.after_user_move(true);
    }

    /// The vertical thumb `(top, length)` on a track `track_h` px long.
    pub fn thumb(&self, track_h: f32) -> (f32, f32) {
        let track = if valid_len(track_h) {
            f64::from(track_h)
        } else {
            0.0
        };
        let max_top = self.max_top_row();
        if max_top == 0 {
            return (0.0, track as f32);
        }
        let row_h = f64::from(self.row_h);
        let content = self.rows as f64 * row_h;
        let length = thumb_length(track, f64::from(self.viewport_h), content);
        let scrolled = self.top_row as f64 * row_h + f64::from(self.sane_offset());
        let frac = (scrolled / (max_top as f64 * row_h)).clamp(0.0, 1.0);
        ((frac * (track - length)) as f32, length as f32)
    }

    /// Drags the vertical thumb so its top sits `thumb_top` px down a `track_h` px track. The
    /// ends of the track are exactly the first row and the last page.
    pub fn drag_thumb(&mut self, thumb_top: f64, track_h: f32) {
        let (_, length) = self.thumb(track_h);
        let free = f64::from(track_h) - f64::from(length);
        if let Some(p) = thumb_fraction(thumb_top, free) {
            self.scroll_to_fraction(p);
        }
    }

    /// The horizontal thumb `(left, length)` on a track `track_w` px long.
    pub fn h_thumb(&self, track_w: f32) -> (f32, f32) {
        let track = if valid_len(track_w) {
            f64::from(track_w)
        } else {
            0.0
        };
        let max_x = self.max_left_x();
        if max_x <= 0.0 {
            return (0.0, track as f32);
        }
        let length = thumb_length(track, f64::from(self.viewport_w), self.content_w);
        let frac = (self.left_x / max_x).clamp(0.0, 1.0);
        ((frac * (track - length)) as f32, length as f32)
    }

    /// Drags the horizontal thumb so its left edge sits `thumb_left` px along the track.
    pub fn drag_h_thumb(&mut self, thumb_left: f64, track_w: f32) {
        let (_, length) = self.h_thumb(track_w);
        let free = f64::from(track_w) - f64::from(length);
        if let Some(p) = thumb_fraction(thumb_left, free) {
            self.left_x = if p >= 1.0 {
                self.max_left_x()
            } else {
                self.clamp_x(p * self.max_left_x())
            };
        }
    }

    /// The spike's `scroll_by`: moves by `delta` px relative to the anchor row, in `f64`, and
    /// clamps to `[0, max_top_row]` (offset 0 at either bound).
    fn move_by(&mut self, delta: f64) {
        if !delta.is_finite() {
            return;
        }
        let row_h = f64::from(self.row_h);
        let max_top = self.max_top_row();
        let within = f64::from(self.sane_offset()) + delta;
        let steps = (within / row_h).floor();
        let mut offset = within - steps * row_h;
        // `as` saturates; i128 keeps `top_row + steps` exact for any u64 anchor.
        let mut top = i128::from(self.top_row).saturating_add(steps as i128);
        if offset >= row_h || (offset as f32) >= self.row_h {
            top = top.saturating_add(1);
            offset = 0.0;
        }
        if top < 0 {
            self.top_row = 0;
            self.top_row_offset = 0.0;
        } else if top >= i128::from(max_top) {
            self.top_row = max_top;
            self.top_row_offset = 0.0;
        } else {
            self.top_row = top as u64;
            self.top_row_offset = offset.max(0.0) as f32;
        }
    }

    /// After a move the user made: at the bottom of a grid that scrolls, follow the tail; away
    /// from it, stop. On a grid too short to scroll, `keep` leaves the user's last choice (End
    /// before the rows arrived) in place.
    fn after_user_move(&mut self, keep: bool) {
        let max_top = self.max_top_row();
        if self.top_row < max_top {
            self.follow_tail = false;
        } else if max_top > 0 {
            self.follow_tail = true;
        } else if !keep {
            self.follow_tail = false;
        }
    }

    /// Re-establishes the invariants after the geometry or a public field changed.
    fn clamp(&mut self) {
        let max_top = self.max_top_row();
        if self.top_row >= max_top {
            self.top_row = max_top;
            self.top_row_offset = 0.0;
        } else {
            let offset = self.sane_offset();
            if offset != self.top_row_offset || offset >= self.row_h {
                self.top_row_offset = 0.0;
                self.move_by(f64::from(offset));
            }
        }
        self.left_x = self.clamp_x(self.left_x);
    }

    fn clamp_x(&self, x: f64) -> f64 {
        if x.is_finite() {
            x.clamp(0.0, self.max_left_x())
        } else {
            0.0
        }
    }

    /// The offset, with a non-finite or negative value written from outside read as 0.
    fn sane_offset(&self) -> f32 {
        if self.top_row_offset.is_finite() && self.top_row_offset > 0.0 {
            self.top_row_offset
        } else {
            0.0
        }
    }
}

fn valid_len(v: f32) -> bool {
    v.is_finite() && v >= 0.0
}

/// A thumb's length: the visible share of the content, never shorter than [`MIN_THUMB_PX`] or
/// longer than the track.
fn thumb_length(track: f64, viewport: f64, content: f64) -> f64 {
    (track * viewport / content)
        .max(f64::from(MIN_THUMB_PX))
        .min(track)
}

/// The fraction of the scroll range for a thumb `pos` px along a track with `free` px of travel.
fn thumb_fraction(pos: f64, free: f64) -> Option<f64> {
    if !pos.is_finite() || !free.is_finite() || free <= 0.0 {
        return None;
    }
    Some(if pos <= THUMB_END_EPSILON {
        0.0
    } else if pos >= free - THUMB_END_EPSILON {
        1.0
    } else {
        pos / free
    })
}
