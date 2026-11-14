//! Scrolling (UI-EDIT-08; Task 19): the row anchor and `scroll_x`, rows painted relative to the
//! anchor.
//!
//! - **Row-anchored.** The view's top is a [`ScrollAnchor`] (a buffer line and how far into
//!   it), not an absolute y: rows are painted at `y_of(line) − y_of(anchor.line)` in `f64` and
//!   only then turned into a viewport-relative `f32`, so a million 18 px lines (1.8·10⁷ px,
//!   beyond what `f32` keeps exact) still land on whole pixels. The absolute y exists for the
//!   scrollbar only.
//! - **Top padding.** The code starts [`TOP_PAD`] (6 px) below the top of the code area (NOTES
//!   surface 1). The padding scrolls with the text: the anchor's `offset_in_row` is negative
//!   (down to `−TOP_PAD`) while the view shows it, and the document's end can scroll up to the
//!   same padding above the bottom edge.
//! - **Edits above the anchor** re-pin it ([`ScrollAnchor::rebase`]), so the visible content
//!   stays still while lines are inserted or removed above it.
//! - **Reveal.** After a commit that moved the main caret or edited at it, the next frame
//!   scrolls just enough to show the caret (with a margin sideways).

use hacivat_text::display::{HeightMap, ScrollAnchor};
use hacivat_text::{ChangeDesc, TextBuffer};

/// The padding above the first line (and below the last), in pixels.
pub(crate) const TOP_PAD: f32 = 6.0;
/// Columns kept between the caret and the viewport's side when revealing it.
pub(crate) const REVEAL_MARGIN_COLS: f32 = 8.0;

/// The view's scroll position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScrollState {
    pub anchor: ScrollAnchor,
    /// Pixels scrolled sideways (`f64`: a long line's x outgrows `f32`'s exact range).
    pub scroll_x: f64,
    /// The next frame shows the main caret.
    pub reveal: bool,
    /// The widest row painted since the text was last replaced: `scroll_x` is clamped to it,
    /// so a long line scrolling out of view vertically does not pull the view back sideways.
    pub known_width: f64,
}

impl Default for ScrollState {
    /// The document's top, its padding shown.
    fn default() -> Self {
        ScrollState {
            anchor: ScrollAnchor {
                line: 0,
                wrap_index: 0,
                offset_in_row: -TOP_PAD,
            },
            scroll_x: 0.0,
            reveal: false,
            known_width: 0.0,
        }
    }
}

impl ScrollState {
    /// The content y of the view's top (the top padding is above 0: down to `−TOP_PAD`).
    pub fn top(&self, map: &HeightMap) -> f64 {
        self.anchor.y(map)
    }

    /// The highest top the view can scroll to: the document's end `TOP_PAD` above the bottom.
    pub fn max_top(map: &HeightMap, view_h: f64) -> f64 {
        (map.total_height() + f64::from(TOP_PAD) - view_h).max(-f64::from(TOP_PAD))
    }

    /// Scrolls so that the content y `y` is at the view's top, clamped.
    pub fn set_top(&mut self, map: &HeightMap, y: f64, view_h: f64) {
        let y = y.clamp(-f64::from(TOP_PAD), Self::max_top(map, view_h));
        self.anchor = if y < 0.0 {
            ScrollAnchor {
                line: 0,
                wrap_index: 0,
                offset_in_row: y as f32,
            }
        } else {
            ScrollAnchor::from_y(map, y)
        };
    }

    /// Keeps the position inside the document (it shrank, or the view grew).
    pub fn clamp(&mut self, map: &HeightMap, view_h: f64) {
        let top = self.top(map);
        if top < -f64::from(TOP_PAD) || top > Self::max_top(map, view_h) {
            self.set_top(map, top, view_h);
        }
    }

    /// Scrolls by `dy` pixels (down when positive).
    pub fn scroll_by(&mut self, map: &HeightMap, dy: f64, view_h: f64) {
        let top = self.top(map);
        self.set_top(map, top + dy, view_h);
    }

    /// The top of row `line`, relative to the view's top, in pixels: exact for any document
    /// size, because only the difference to the anchor's line is turned into `f32`.
    pub fn row_offset(&self, map: &HeightMap, line: u32) -> f32 {
        (map.y_of(line) - map.y_of(self.anchor.line) - f64::from(self.anchor.offset_in_row)) as f32
    }

    /// The first row the view shows (partly, perhaps).
    pub fn first_line(&self, map: &HeightMap) -> u32 {
        if self.anchor.offset_in_row < 0.0 {
            0
        } else {
            self.anchor.line.min(map.lines().saturating_sub(1))
        }
    }

    /// Scrolls the least so that row `line` is fully inside a view `view_h` tall.
    pub fn reveal_line(&mut self, map: &HeightMap, line: u32, view_h: f64) {
        let row_top = map.y_of(line);
        let row_bottom = map.y_of(line.saturating_add(1));
        let top = self.top(map);
        // The first and last lines reveal their padding too.
        let want_top = if line == 0 {
            row_top - f64::from(TOP_PAD)
        } else {
            row_top
        };
        let want_bottom = if line + 1 >= map.lines() {
            row_bottom + f64::from(TOP_PAD)
        } else {
            row_bottom
        };
        if want_top < top {
            self.set_top(map, want_top, view_h);
        } else if want_bottom > top + view_h {
            self.set_top(map, want_bottom - view_h, view_h);
        }
    }

    /// Scrolls the least so that the content x `x` (from the text's left edge) sits at least
    /// `margin` inside a view `view_w` wide.
    pub fn reveal_x(&mut self, x: f64, view_w: f64, margin: f64) {
        let margin = margin.min(view_w / 4.0);
        if x < self.scroll_x + margin {
            self.scroll_x = (x - margin).max(0.0);
        } else if x > self.scroll_x + view_w - margin {
            self.scroll_x = x - view_w + margin;
        }
    }

    /// Records a frame's widest row (`content_w`) and keeps `scroll_x` within what the widest
    /// row seen needs, plus `slack`.
    pub fn clamp_x(&mut self, content_w: f64, slack: f64, view_w: f64) {
        self.known_width = self.known_width.max(content_w);
        let max = (self.known_width + slack - view_w).max(0.0);
        self.scroll_x = self.scroll_x.clamp(0.0, max);
    }

    /// Follows a committed change from `old` to `new`: the anchor's line keeps its content in
    /// view. A view showing the top padding stays at the top. A change that was not made to
    /// `old` (one that skipped the view) only clamps.
    pub fn rebase(&mut self, old: &TextBuffer, new: &TextBuffer, change: &ChangeDesc) {
        if self.anchor.offset_in_row < 0.0 {
            return;
        }
        if change.len_old() == old.len_bytes() && change.len_new() == new.len_bytes() {
            let offset = self.anchor.offset_in_row;
            self.anchor.rebase(old, new, change);
            self.anchor.offset_in_row = offset;
        } else {
            let last = u32::try_from(new.len_lines().saturating_sub(1)).unwrap_or(u32::MAX);
            self.anchor.line = self.anchor.line.min(last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hacivat_text::ChangeSet;
    use std::sync::Arc;

    const LH: f64 = 18.0;

    #[test]
    fn the_top_shows_the_padding_and_the_end_stops_at_it() {
        let map = HeightMap::uniform(100, LH);
        let mut s = ScrollState::default();
        assert_eq!(s.top(&map), -6.0);
        assert_eq!(s.row_offset(&map, 0), 6.0);
        s.scroll_by(&map, -50.0, 300.0);
        assert_eq!(s.top(&map), -6.0, "no scrolling above the padding");
        s.scroll_by(&map, 1e9, 300.0);
        // 100 lines × 18 = 1800; the last line's bottom sits 6 px above the view's bottom.
        assert_eq!(s.top(&map), 1800.0 + 6.0 - 300.0);
        assert_eq!(s.row_offset(&map, 99) + 18.0, 300.0 - 6.0);
    }

    #[test]
    fn a_million_lines_paint_at_exact_pixels() {
        let map = HeightMap::uniform(1_000_000, LH);
        let mut s = ScrollState::default();
        s.set_top(&map, map.y_of(999_950), 600.0);
        assert_eq!(s.anchor.line, 999_950);
        assert_eq!(s.row_offset(&map, 999_950), 0.0);
        assert_eq!(s.row_offset(&map, 999_951), 18.0);
        // Half a row in: still exact (f32 alone would be off at 1.8e7 px).
        s.scroll_by(&map, 9.0, 600.0);
        assert_eq!(s.row_offset(&map, 999_951), 9.0);
        assert_eq!(s.first_line(&map), 999_950);
    }

    #[test]
    fn reveal_scrolls_the_least() {
        let map = HeightMap::uniform(1_000, LH);
        let mut s = ScrollState::default();
        s.reveal_line(&map, 5, 180.0);
        assert_eq!(s.top(&map), -6.0, "already visible");
        s.reveal_line(&map, 50, 180.0);
        assert_eq!(
            s.top(&map),
            51.0 * LH - 180.0,
            "the row's bottom at the view's bottom"
        );
        s.reveal_line(&map, 10, 180.0);
        assert_eq!(s.top(&map), 10.0 * LH, "the row's top at the view's top");
        s.reveal_line(&map, 0, 180.0);
        assert_eq!(s.top(&map), -6.0, "the first line shows its padding");
        s.reveal_x(1_000.0, 400.0, 60.0);
        assert_eq!(s.scroll_x, 1_000.0 - 400.0 + 60.0);
        s.reveal_x(10.0, 400.0, 60.0);
        assert_eq!(s.scroll_x, 0.0);
    }

    #[test]
    fn sideways_scrolling_is_clamped_to_the_widest_row_seen() {
        let mut s = ScrollState {
            scroll_x: 5_000.0,
            ..ScrollState::default()
        };
        s.clamp_x(3_000.0, 60.0, 400.0);
        assert_eq!(s.scroll_x, 3_000.0 + 60.0 - 400.0);
        // The wide row left the view: the position stays.
        s.clamp_x(200.0, 60.0, 400.0);
        assert_eq!(s.scroll_x, 2_660.0);
        assert_eq!(s.known_width, 3_000.0);
    }

    #[test]
    fn edits_above_keep_the_anchor_on_its_content() {
        let text: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let old = TextBuffer::from_str(&text);
        let map = HeightMap::uniform(old.len_lines() as u32, LH);
        let mut s = ScrollState::default();
        s.set_top(&map, map.y_of(100) + 4.0, 300.0);
        let cs = ChangeSet::from_edits(old.len_bytes(), [(0..0, Arc::from("a\nb\nc\n"))]).unwrap();
        let mut new = old.clone();
        cs.apply(&mut new);
        s.rebase(&old, &new, &cs.desc());
        assert_eq!(s.anchor.line, 103);
        assert_eq!(
            s.anchor.offset_in_row, 4.0,
            "the offset into the row is kept"
        );
        // A change that skipped the view only clamps.
        let short = TextBuffer::from_str("x\ny");
        s.rebase(&new, &short, &cs.desc());
        assert_eq!(s.anchor.line, 1);
        // At the very top, the view stays at the top.
        let mut top = ScrollState::default();
        top.rebase(&old, &new, &cs.desc());
        assert_eq!(top, ScrollState::default());
    }
}
