//! Scrolling: a pixel offset into the flat rows, the viewport, and scroll anchoring.
//!
//! - On a generation change the first visible node keeps its screen position: it is located
//!   again by key after the rebuild ([`TreeView::anchor`], used by `sync`).
//! - Expanding or collapsing above the viewport shifts the offset by the spliced rows, so the
//!   rows on screen stay put; collapsing the subtree that holds the top row makes the collapsed
//!   node the top row.

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{Bounds, Context, Pixels, point, px, size};

use super::TreeView;
use crate::tree::{TreeKey, TreeModel};

impl<M: TreeModel> TreeView<M> {
    /// The first visible node row and how far the viewport's top is into it.
    pub(super) fn anchor(&self) -> Option<(TreeKey, f64)> {
        let h = f64::from(self.row_h);
        let top = (self.scroll_y / h).floor().max(0.0) as usize;
        let (ix, row) = self
            .flat
            .rows()
            .iter()
            .enumerate()
            .skip(top)
            .find(|(_, r)| r.is_node())?;
        Some((row.key, self.scroll_y - ix as f64 * h))
    }

    /// Scroll anchoring for `n` rows inserted at `at`.
    pub(super) fn shift_for_insert(&mut self, at: usize, n: usize) {
        let h = f64::from(self.row_h);
        if n > 0 && (at as f64) * h <= self.scroll_y {
            self.scroll_y += n as f64 * h;
        }
    }

    /// Scroll anchoring for the `n` rows after row `ix` removed.
    pub(super) fn shift_for_remove(&mut self, ix: usize, n: usize) {
        let h = f64::from(self.row_h);
        let (start, end) = ((ix + 1) as f64 * h, (ix + 1 + n) as f64 * h);
        if n == 0 || self.scroll_y < start {
            return;
        }
        if self.scroll_y >= end {
            self.scroll_y -= n as f64 * h;
        } else {
            // The top row was removed: the collapsed node becomes the top row.
            self.scroll_y = ix as f64 * h;
        }
        self.clamp_scroll();
    }

    pub fn scroll_offset(&self) -> f64 {
        self.scroll_y
    }

    /// Scrolls by `dy` px (positive towards later rows).
    pub fn scroll_by(&mut self, dy: f64, cx: &mut Context<Self>) {
        self.scroll_y += dy;
        self.clamp_scroll();
        cx.notify();
    }

    /// Where node `key`'s row is in window coordinates at the current scroll (it may lie
    /// outside the viewport); `None` before the first frame or when the row is not shown.
    pub fn row_bounds(&self, key: TreeKey) -> Option<Bounds<Pixels>> {
        let ix = self.flat.index_of(key)?;
        self.bounds_of(ix)
    }

    pub(super) fn bounds_of(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let b = self.bounds?;
        let top = ix as f64 * f64::from(self.row_h) - self.scroll_y;
        Some(Bounds::new(
            point(b.left(), b.top() + px(top as f32)),
            size(b.size.width, px(self.row_h)),
        ))
    }

    pub(super) fn viewport_h(&self) -> f64 {
        self.bounds
            .map_or(0.0, |b| f64::from(f32::from(b.size.height)))
    }

    fn content_h(&self) -> f64 {
        self.flat.len() as f64 * f64::from(self.row_h)
    }

    pub(super) fn clamp_scroll(&mut self) {
        let max = (self.content_h() - self.viewport_h()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    /// Scrolls the least that shows row `ix` whole.
    pub(super) fn reveal(&mut self, ix: usize) {
        let h = f64::from(self.row_h);
        let (top, bottom) = (ix as f64 * h, (ix + 1) as f64 * h);
        let view = self.viewport_h();
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + view && view > 0.0 {
            self.scroll_y = bottom - view;
        }
        self.clamp_scroll();
    }

    /// Rows a PageUp/PageDown moves: the rows that fit the viewport, less one.
    pub(super) fn page_rows(&self) -> usize {
        ((self.viewport_h() / f64::from(self.row_h)).floor() as usize).max(2) - 1
    }

    /// The rows intersecting the viewport.
    pub(super) fn visible(&self) -> Range<usize> {
        let h = f64::from(self.row_h);
        let first = (self.scroll_y / h).floor().max(0.0) as usize;
        let last = ((self.scroll_y + self.viewport_h()) / h).ceil() as usize;
        first.min(self.flat.len())..last.min(self.flat.len())
    }
}
