//! The gutter (UI-EDIT-19; Task 28), after `design/m3a-query/NOTES.md` surfaces 1, 3 and 5
//! (binding): line numbers from the digit cache (`layout_cache::paint_line_number`), right
//! aligned with 14 px of padding, the current line's number in `text` and the others in
//! `text_3`, on the panel (the gutter has no fill of its own); the Run preview's bars at x = 4,
//! 1 px wide (`blue` for what Run executes, dashed `text_2` when a boundary is uncertain, solid
//! `text_3` with no connection; x = 2 when an executed bar is already at x = 4; units of a
//! multi-statement selection leave a 2 px gap). The width (58 px + (digits − 3) × advance, from
//! the document's last line number, so it never jitters while typing) is the element's.
//!
//! [`Overlay`] is what the gutter, the marks and their end-of-line statuses take from one
//! frame's layout: the painted rows (their tops and where their text ends), the text area's
//! right edge, and the band's bars and hints. The element builds it in prepaint ([`prepaint`]).

use std::ops::Range;

use hacivat_text::Snapshot;
use hacivat_text::display::ScrollAnchor;
use hacivat_ui_core::gpui_surface::{
    App, BorderStyle, Bounds, Corners, Edges, Hsla, Pixels, Point, Window, fill, point, px, quad,
    size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use smallvec::SmallVec;

use crate::element::FrameLayout;
use crate::layout_cache::paint_line_number;
use crate::marks::MarkStore;
use crate::preview::{self, BandState, BandStyle, GutterBar, PreviewHint};

/// The numbers' right padding.
pub(crate) const NUMBER_PAD: f32 = 14.0;
/// A range bar's x and width; the preview bar's x when an executed bar is already there.
pub(crate) const BAR_X: f32 = 4.0;
pub(crate) const BAR_SHIFTED_X: f32 = 2.0;

/// What the gutter paints in one frame (Task 19 builds it at its paint point; the call site is
/// `element::paint_frame` since Task 1b, fix round 1).
#[derive(Debug, Clone)]
pub struct GutterCtx<'a> {
    pub bounds: Bounds<Pixels>,
    pub line_height: Pixels,
    pub first_line: u32,
    pub last_line: u32,
    /// Digits of the widest line number.
    pub digits: u8,
    pub current_line: u32,
    pub scroll: ScrollAnchor,
    /// The frame's text: byte offsets (marks, the band) to lines through `snapshot.buffer`.
    pub snapshot: &'a Snapshot,
    /// The lines (0-based, end exclusive) of the statement at the caret
    /// (`preview::current_statement`); the scrollbar's marker.
    pub current_statement: Option<Range<u32>>,
    // ## m3a-t28-gutter
    /// The frame's rows, text ends and the band's bars and hints ([`prepaint`]).
    pub overlay: &'a Overlay,
}

impl GutterCtx<'_> {
    /// The top of `line`'s row, in window coordinates (rows are uniform in M3a).
    pub fn top_of(&self, line: u32) -> Pixels {
        self.overlay.top_of(line).unwrap_or_else(|| {
            let lh = f64::from(f32::from(self.line_height));
            let rel = (f64::from(line) - f64::from(self.scroll.line)) * lh
                - f64::from(self.scroll.offset_in_row);
            self.bounds.origin.y + px(rel as f32)
        })
    }
}

/// One painted row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayRow {
    pub line: u32,
    pub top: Pixels,
    /// The window x where the row's text ends.
    pub text_end: Pixels,
}

/// What the gutter, the marks and the end-of-line statuses take from one frame's layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// The painted rows, in order.
    pub rows: Vec<OverlayRow>,
    pub line_height: Pixels,
    /// The text area (right of the gutter; the scrollbar track sits over its right edge).
    pub text_area: Bounds<Pixels>,
    /// The band's gutter bars and end-of-line hints.
    pub band: preview::BandFrame,
}

impl Overlay {
    /// The top of `line`, extrapolated from the painted rows; `None` when nothing was painted.
    pub fn top_of(&self, line: u32) -> Option<Pixels> {
        let first = self.rows.first()?;
        let rel =
            (f64::from(line) - f64::from(first.line)) * f64::from(f32::from(self.line_height));
        Some(first.top + px(rel as f32))
    }

    /// The painted row of `line`.
    pub fn row(&self, line: u32) -> Option<&OverlayRow> {
        let first = self.rows.first()?.line;
        let row = self.rows.get(line.checked_sub(first)? as usize)?;
        (row.line == line).then_some(row)
    }
}

/// Builds the frame's [`Overlay`] (prepaint): the rows from `layout`, the band's rectangles into
/// `band` and its bars and hints (an executed bar from `marks` moves a preview bar to x = 2).
pub fn prepaint(
    band: &mut BandState,
    marks: &MarkStore,
    layout: &FrameLayout,
    snap: &Snapshot,
) -> Overlay {
    let rows = layout
        .lines
        .iter()
        .map(|r| OverlayRow {
            line: r.line,
            top: r.top,
            text_end: px((layout.origin_x + f64::from(r.width)) as f32),
        })
        .collect();
    Overlay {
        rows,
        line_height: layout.rows().line_height,
        text_area: layout.rows().bounds,
        band: preview::layout(band, marks, layout, snap),
    }
}

/// The gutter's inks for one frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Inks {
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub blue: Hsla,
    pub green: Hsla,
    pub amber: Hsla,
    pub seam_strong: Hsla,
    pub seam_soft: Hsla,
    pub panel: Hsla,
}

impl Inks {
    pub fn of(cx: &App) -> Self {
        let theme = cx
            .try_global::<Theme>()
            .copied()
            .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
        let c = &theme.tokens.colors;
        Inks {
            text: c.text.to_hsla(),
            text_2: c.text_2.to_hsla(),
            text_3: c.text_3.to_hsla(),
            blue: c.blue.to_hsla(),
            green: c.green.to_hsla(),
            amber: c.amber.to_hsla(),
            seam_strong: c.seam_strong.to_hsla(),
            seam_soft: c.seam_soft.to_hsla(),
            panel: c.panel.to_hsla(),
        }
    }
}

/// A 1 px dashed vertical line (GPUI's dashed border on the left edge of a 1 px quad).
pub(crate) fn dashed_vline(window: &mut Window, at: Point<Pixels>, height: Pixels, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(at, size(px(1.0), height)),
        Corners::default(),
        Hsla::transparent_black(),
        Edges {
            left: px(1.0),
            ..Edges::default()
        },
        color,
        BorderStyle::Dashed,
    ));
}

/// A 1 px dashed horizontal line (the uncertain exact tint's lower edge).
pub(crate) fn dashed_hline(window: &mut Window, at: Point<Pixels>, width: Pixels, color: Hsla) {
    window.paint_quad(quad(
        Bounds::new(at, size(width, px(1.0))),
        Corners::default(),
        Hsla::transparent_black(),
        Edges {
            bottom: px(1.0),
            ..Edges::default()
        },
        color,
        BorderStyle::Dashed,
    ));
}

/// Paints the line numbers and the band's gutter bars.
pub fn paint(g: &GutterCtx<'_>, window: &mut Window, cx: &mut App) {
    let inks = Inks::of(cx);
    let right = g.bounds.right() - px(NUMBER_PAD);
    let lines: SmallVec<[(u32, Pixels); 64]> = if g.overlay.rows.is_empty() {
        (g.first_line..=g.last_line.max(g.first_line))
            .filter(|&l| (l as usize) < g.snapshot.buffer.len_lines())
            .map(|l| (l, g.top_of(l)))
            .collect()
    } else {
        g.overlay.rows.iter().map(|r| (r.line, r.top)).collect()
    };
    for (line, top) in lines {
        let color = if line == g.current_line {
            inks.text
        } else {
            inks.text_3
        };
        paint_line_number(window, cx, line.saturating_add(1), right, top, color);
    }
    for bar in &g.overlay.band.bars {
        paint_bar(g, bar, &inks, window);
    }
}

/// One of the band's bars: over its lines, inset 1 px at each end of a unit of several (the
/// 2 px gap between neighbours).
fn paint_bar(g: &GutterCtx<'_>, bar: &GutterBar, inks: &Inks, window: &mut Window) {
    let lh = g.line_height;
    let top = g.top_of(bar.lines.start);
    let rows = bar.lines.end.saturating_sub(bar.lines.start) as f32;
    let (top, height) = if bar.gap && !bar.shifted {
        (top + px(1.0), lh * rows - px(3.0))
    } else {
        (top, lh * rows)
    };
    // Clip to the gutter (a bar of a unit that runs past the view).
    let y0 = top.max(g.bounds.top());
    let y1 = (top + height).min(g.bounds.bottom());
    if y1 <= y0 {
        return;
    }
    let x = g.bounds.left() + px(if bar.shifted { BAR_SHIFTED_X } else { BAR_X });
    match bar.style {
        BandStyle::Run => window.paint_quad(fill(
            Bounds::new(point(x, y0), size(px(1.0), y1 - y0)),
            inks.blue,
        )),
        BandStyle::Offline => window.paint_quad(fill(
            Bounds::new(point(x, y0), size(px(1.0), y1 - y0)),
            inks.text_3,
        )),
        BandStyle::Uncertain => dashed_vline(window, point(x, y0), y1 - y0, inks.text_2),
    }
}

/// The hint's glyph and Fluent message (Task 28's block).
pub(crate) fn hint_text(h: PreviewHint) -> (&'static str, Option<(usize, usize)>) {
    match h {
        PreviewHint::Unit { i, n } => ("hv-editor-preview-unit", Some((i, n))),
        PreviewHint::Uncertain => ("hv-editor-preview-uncertain", None),
        PreviewHint::Offline => ("hv-editor-preview-offline", None),
    }
}
