//! Frozen columns (HV-GRID-02 frozen; M3c Task 35): the first `n` visible columns stay put
//! while the body scrolls horizontally, painted in their own layer.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). ## m3c-t35: the bodies.
//!
//! **Coordinates.** `left_x` keeps its M3a meaning (how far the content is scrolled), but the
//! scroll covers only the columns after the frozen ones: the element gives `ScrollState` a
//! viewport of the body less the frozen width and a content of [`ColumnLayout::scroll_width`],
//! so `left_x` runs over `[0, scroll_width − viewport]`. On screen (relative to the body's left
//! edge) a frozen column sits at its own edge `x_of(ix)`, a scrolling one at `x_of(ix) − left_x`
//! ([`ColumnLayout::screen_x`]); the scrolling columns are clipped to the right of the frozen
//! width. With nothing frozen every function here is M3a's.
//!
//! **Fit.** The count the user set ([`ColumnLayout::frozen`]) is kept, but a frame shows only
//! as many frozen columns as leave [`MIN_WIDTH`] of the body to scroll
//! ([`ColumnLayout::fit_frozen`], [`ColumnLayout::frozen_shown`]): a narrow pane never loses
//! its scrolling columns, and widening it again brings the frozen ones back.

use std::ops::Range;

use crate::model::ColumnLayout;
use crate::model::autosize::MIN_WIDTH;

impl ColumnLayout {
    /// Freezes the first `n` visible columns (`0`: none).
    pub fn set_frozen(&mut self, n: usize) {
        self.frozen = n;
    }

    /// How many leading visible columns the user froze (at most the visible columns).
    pub fn frozen(&self) -> usize {
        self.frozen.min(self.len())
    }

    /// How many frozen columns the frame shows: [`ColumnLayout::frozen`], less any the body
    /// the last [`ColumnLayout::fit_frozen`] noted has no room for. O(frozen): nothing frozen
    /// is O(1) (## m3c-t35 fix round 1, M5).
    pub fn frozen_shown(&self) -> usize {
        match self.frozen_body {
            Some(body_w) => self.frozen_fitting(self.frozen(), body_w),
            None => self.frozen(),
        }
    }

    /// Notes the body width `body_w` of the frame being laid out: from now on at most
    /// [`ColumnLayout::frozen_fitting`]`(frozen, body_w)` frozen columns show. The element
    /// calls it once per frame, before it lays out the scroll.
    pub fn fit_frozen(&mut self, body_w: f32) {
        self.frozen_body = Some(body_w);
    }

    /// The most of the first `n` columns that fit frozen in a body `body_w` wide: they leave
    /// [`MIN_WIDTH`] px to scroll (or, all of them frozen, fit the body whole).
    pub fn frozen_fitting(&self, n: usize, body_w: f32) -> usize {
        let n = n.min(self.len());
        let body = f64::from(body_w);
        let fits = |k: usize| {
            let room = if k < self.len() {
                body - f64::from(MIN_WIDTH)
            } else {
                body
            };
            self.x_of(k) <= room
        };
        (0..=n).rev().find(|k| fits(*k)).unwrap_or(0)
    }

    /// The width of the frozen columns the frame shows.
    pub fn frozen_width(&self) -> f64 {
        self.x_of(self.frozen_shown())
    }

    /// The width the horizontal scroll covers: the visible columns after the frozen ones.
    pub fn scroll_width(&self) -> f64 {
        (self.total_width() - self.frozen_width()).max(0.0)
    }

    /// The left edge of visible column `ix` relative to the body's left edge, scrolled by
    /// `left_x` unless the column is frozen.
    pub fn screen_x(&self, ix: usize, left_x: f64) -> f64 {
        if ix < self.frozen_shown() {
            self.x_of(ix)
        } else {
            self.x_of(ix) - left_x
        }
    }

    /// The scrolling columns a scroll viewport `width` px wide (the body less the frozen width)
    /// shows from `left_x`; never a frozen column. With nothing frozen,
    /// [`ColumnLayout::visible_range`].
    pub fn scrolling_range(&self, left_x: f64, width: f32) -> Range<usize> {
        let f = self.frozen_shown();
        let left = if left_x.is_finite() {
            left_x.max(0.0)
        } else {
            0.0
        };
        let r = self.visible_range(self.frozen_width() + left, width);
        r.start.max(f)..r.end.max(f)
    }

    /// The visible column at `x` px right of the body's left edge, scrolled by `left_x`: a
    /// frozen column left of the frozen width, else a scrolling one.
    pub fn col_at_screen_x(&self, x: f64, left_x: f64) -> Option<usize> {
        if !x.is_finite() || x < 0.0 {
            return None;
        }
        if x < self.frozen_width() {
            return self.col_at_x(x);
        }
        let f = self.frozen_shown();
        self.col_at_x(x + left_x.max(0.0)).filter(|ix| *ix >= f)
    }
}
