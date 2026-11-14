//! The paint layers (HV-GRID-03; Task 20), in order inside each region: 1 background, 2 zebra
//! (off in M3a), 5 search matches (empty in M3a), 6 selection fill and the selected rows'
//! numbers, 7 content (text, the `NULL` marker in synthesized small caps at `text-3`, chips),
//! 8 `seam-soft` hairlines snapped to device pixels, 9 the hover outline and the 1 px `blue`
//! selection box with its 5 px handle in a 1 px `panel` ring, 10 the resize guide.
//!
//! The text roles follow `design/m3a-query/NOTES.md` surface 10: body 13 `text` for text,
//! numbers and booleans; Fragment Mono 11.5 `text` for UUIDs, bytes and JSON and `text-2` for
//! timestamps; `''` in mono 11.5 `text-3`; `NULL` in `label-sc` small caps `text-3`; a decode
//! error as the ink triangle-alert and its message in `text-2`; chips in Fragment Mono 11
//! `text-2`; row numbers in micro 11.5 `text-3` (`blue` when selected, `text-2` on hover); the
//! header's name in `body-strong` and its type in `mono-kbd` `text-3`.

use std::sync::Arc;

use hacivat_ui::icon::IconName;
use hacivat_ui::text::{RenderedText, TextRegistry, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, BorderStyle, Bounds, ContentMask, Corners, DecorationRun, Edges, Hsla, LineLayout, Pixels,
    SharedString, TextAlign, Window, fill, hsla, point, px, quad, size,
};
use hacivat_ui_core::metrics::snap_to_device_px;
use hacivat_ui_core::theme::{Colors, ThemeTokens};

use super::text_cache::{
    Budget, CHIP_PAD_PX, ColorClass, FrameLru, PieceKind, Role, Scratch, Shaped, TextKey,
    with_shape,
};
use crate::source::{Align, CellKind, ColumnKind, NULL_MARKER};

/// An empty string's marker (`''`, mono `text-3`): data notation, like `NULL`, never Fluent.
pub const EMPTY_MARKER: &str = "''";

/// A cell's horizontal padding (`spacing.lg`).
pub const CELL_PAD_PX: f32 = 10.0;

/// A control chip's well height (Fragment Mono 11/14).
pub const CHIP_H_PX: f32 = 14.0;

/// The trailing chip's (`… 5 MB`) padding inside its border, and its gap after the text.
pub const TRAIL_PAD_PX: f32 = 4.0;
pub const TRAIL_GAP_PX: f32 = 4.0;

/// The decode-error glyph's size and its gap before the message.
pub const ERROR_ICON_PX: f32 = 12.0;
pub const ERROR_GAP_PX: f32 = 5.0;

/// A placeholder row's skeleton bar height (NOTES "Skeleton bar").
pub const SKELETON_H_PX: f32 = 7.0;

/// Where the registry's records point (the cell and header paint).
const CELL_PATH: &str = "crates/ui/hacivat-ui-grid/src/element/layers.rs#cell";
const HEADER_PATH: &str = "crates/ui/hacivat-ui-grid/src/element/layers.rs#header";
const ROW_NUMBER_PATH: &str = "crates/ui/hacivat-ui-grid/src/element/layers.rs#row-number";

/// Every role the grid paints with, and the row numbers' digit layouts.
#[derive(Debug, Clone)]
pub struct Roles {
    pub body: Role,
    pub mono: Role,
    pub temporal: Role,
    pub empty: Role,
    pub null: Role,
    pub error: Role,
    pub chip: Role,
    pub row_number: Role,
    pub header_name: Role,
    pub header_type: Role,
    /// `0` … `9` in the row-number role: row numbers paint digit by digit at the tabular
    /// advance, so scrolling shapes no row number.
    pub digits: [Arc<LineLayout>; 10],
    pub digit_advance: f32,
}

impl Roles {
    pub fn new(t: &ThemeTokens, window: &Window) -> Self {
        let r = &t.type_ramp;
        let row_number = Role::new(&r.micro, ColorClass::Text3, window);
        let digits: [Arc<LineLayout>; 10] = std::array::from_fn(|d| {
            let s = [b'0' + d as u8];
            row_number.layout(std::str::from_utf8(&s).unwrap_or("0"), window)
        });
        let digit_advance = digits
            .iter()
            .map(|l| f32::from(l.width))
            .fold(0.0, f32::max);
        Roles {
            body: Role::new(&r.body, ColorClass::Text, window),
            mono: Role::new(&r.mono_data, ColorClass::Text, window),
            temporal: Role::new(&r.mono_data, ColorClass::Text2, window),
            empty: Role::new(&r.mono_data, ColorClass::Text3, window),
            null: Role::new(&r.label_sc, ColorClass::Text3, window),
            error: Role::new(&r.body, ColorClass::Text2, window),
            chip: Role::new(&r.mono_micro, ColorClass::Text2, window),
            header_name: Role::new(&r.body_strong, ColorClass::Text, window),
            header_type: Role::new(&r.mono_kbd, ColorClass::Text3, window),
            row_number,
            digits,
            digit_advance,
        }
    }

    /// The role of a cell of `kind` in a column of `column`.
    pub fn for_cell(&self, kind: CellKind, column: ColumnKind) -> &Role {
        match kind {
            CellKind::Text => match column {
                ColumnKind::Uuid | ColumnKind::Json | ColumnKind::Binary => &self.mono,
                _ => &self.body,
            },
            CellKind::Number | CellKind::Bool | CellKind::Placeholder => &self.body,
            CellKind::Null => &self.null,
            CellKind::Empty => &self.empty,
            CellKind::Temporal => &self.temporal,
            CellKind::Binary | CellKind::Json => &self.mono,
            CellKind::Error => &self.error,
        }
    }
}

/// The frame's colours.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub text: Hsla,
    pub text2: Hsla,
    pub text3: Hsla,
    pub blue: Hsla,
    pub panel: Hsla,
    pub head: Hsla,
    pub gutter: Hsla,
    pub hover: Hsla,
    pub seam: Hsla,
    pub seam_soft: Hsla,
    pub seam_strong: Hsla,
    pub blue_tint: Hsla,
    pub blue_tint_strong: Hsla,
    pub green_fill: Hsla,
    /// The skeleton bar: `seam-soft` on dark, `seam` on light (NOTES).
    pub skeleton: Hsla,
}

impl Palette {
    pub fn new(c: &Colors, light: bool) -> Self {
        Palette {
            text: c.text.to_hsla(),
            text2: c.text_2.to_hsla(),
            text3: c.text_3.to_hsla(),
            blue: c.blue.to_hsla(),
            panel: c.panel.to_hsla(),
            head: c.head.to_hsla(),
            gutter: c.gutter.to_hsla(),
            hover: c.hover.to_hsla(),
            seam: c.seam.to_hsla(),
            seam_soft: c.seam_soft.to_hsla(),
            seam_strong: c.seam_strong.to_hsla(),
            blue_tint: c.blue_tint.to_hsla(),
            blue_tint_strong: c.blue_tint_strong.to_hsla(),
            green_fill: c.green_fill.to_hsla(),
            skeleton: if light {
                c.seam.to_hsla()
            } else {
                c.seam_soft.to_hsla()
            },
        }
    }

    pub fn of(&self, class: ColorClass) -> Hsla {
        match class {
            ColorClass::Text => self.text,
            ColorClass::Text2 => self.text2,
            ColorClass::Text3 => self.text3,
            ColorClass::Blue => self.blue,
        }
    }
}

/// `x, y, w, h` as bounds (negative sizes clamp to 0).
pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w.max(0.0)), px(h.max(0.0))))
}

/// A device-pixel snapper at `scale`.
#[derive(Debug, Clone, Copy)]
pub struct Snap(pub f32);

impl Snap {
    pub fn v(self, v: f32) -> f32 {
        snap_to_device_px(v, self.0)
    }

    /// One device-snapped hairline thickness (1 logical px rounded to whole device pixels).
    pub fn hair(self) -> f32 {
        snap_to_device_px(1.0, self.0).max(1.0 / self.0.max(1.0))
    }

    /// A horizontal hairline ending at `bottom` (exclusive), from `x0` to `x1`.
    pub fn hline(self, x0: f32, x1: f32, bottom: f32) -> Bounds<Pixels> {
        let h = self.hair();
        let (a, b) = (self.v(x0), self.v(x1));
        rect(a, self.v(bottom - h), b - a, h)
    }

    /// A vertical hairline ending at `right` (exclusive), from `y0` to `y1`.
    pub fn vline(self, right: f32, y0: f32, y1: f32) -> Bounds<Pixels> {
        let w = self.hair();
        let (a, b) = (self.v(y0), self.v(y1));
        rect(self.v(right - w), a, w, b - a)
    }
}

/// Paints `layout` with its baseline centred in `line_h` at `(x, y)` in `color`.
pub fn paint_layout(
    layout: &LineLayout,
    x: f32,
    y: f32,
    line_h: f32,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    if layout.len == 0 {
        return;
    }
    let run = [DecorationRun {
        len: layout.len as u32,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    // Painting fails only when the glyph atlas cannot take a glyph; the cell then misses glyphs
    // for one frame, as GPUI's own text does.
    let _ = layout.paint(
        point(px(x), px(y)),
        px(line_h),
        TextAlign::Left,
        None,
        &run,
        window,
        cx,
    );
}

/// Paints a shaped cell with its text starting at `x`, in a row from `y` `line_h` tall; control
/// chips get their `seam-soft` well (skipped when `chips` is false: fast-scroll).
#[allow(clippy::too_many_arguments)]
pub fn paint_shaped(
    shaped: &Shaped,
    x: f32,
    y: f32,
    line_h: f32,
    palette: &Palette,
    chips: bool,
    window: &mut Window,
    cx: &mut App,
) {
    for p in &shaped.pieces {
        if p.kind == PieceKind::Control {
            if !chips {
                continue;
            }
            let w = f32::from(p.layout.width);
            let well = rect(
                x + p.x - CHIP_PAD_PX,
                y + (line_h - CHIP_H_PX) / 2.0,
                w + 2.0 * CHIP_PAD_PX,
                CHIP_H_PX,
            );
            window.paint_quad(fill(well, palette.seam_soft).corner_radii(px(3.0)));
        }
        paint_layout(
            &p.layout,
            x + p.x,
            y,
            line_h,
            palette.of(p.color),
            window,
            cx,
        );
    }
}

/// Fully transparent (an outline's fill).
pub fn clear() -> Hsla {
    hsla(0.0, 0.0, 0.0, 0.0)
}

/// A 1 px border inside `bounds` with corners of `radius` over `background`.
pub fn bordered(
    bounds: Bounds<Pixels>,
    background: Hsla,
    border: Hsla,
    radius: f32,
    window: &mut Window,
) {
    window.paint_quad(quad(
        bounds,
        Corners::all(px(radius)),
        background,
        Edges::all(px(1.0)),
        border,
        BorderStyle::default(),
    ));
}

/// A square 1 px outline inside `bounds`.
pub fn outline(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    bordered(bounds, clear(), color, 0.0, window);
}

/// What a cell paint needs that does not change across the frame's cells.
pub struct CellPaint<'a> {
    pub roles: &'a Roles,
    pub palette: &'a Palette,
    pub fast: bool,
    pub recording: bool,
    pub row_h: f32,
}

/// The result of painting one cell.
#[derive(Debug, Clone, Copy, Default)]
pub struct CellOutcome {
    /// Painted below full quality: a fast-scroll prefix or the skeleton bar.
    pub degraded: bool,
    /// Painted as the skeleton bar (the frame's shaping budget was spent, or the text has no
    /// safe fast-scroll prefix).
    pub skeleton: bool,
}

/// Where a body cell sits: its column's left edge and width, its row's top, and the row and
/// visible column index (the skeleton bar's width hash).
#[derive(Debug, Clone, Copy)]
pub struct CellBox {
    pub left: f32,
    pub width: f32,
    pub y: f32,
    pub row: u64,
    pub col: usize,
}

/// One cell's content (layer 7): `text` in its role inside the cell, aligned, with its
/// decode-error glyph or trailing chip. In a fast-scroll frame a cell the budget cannot shape,
/// or whose text has no safe prefix, paints the static skeleton bar: never a blank cell.
/// `record` hears of every cell: `Some(shape)` painted text, `None` the skeleton bar.
#[allow(clippy::too_many_arguments)]
pub fn paint_cell_content(
    p: &CellPaint<'_>,
    cache: &mut FrameLru<TextKey, Shaped>,
    budget: &mut Budget,
    scratch: &mut Scratch,
    text: &SharedString,
    kind: CellKind,
    column: ColumnKind,
    align: Align,
    chip: Option<&SharedString>,
    at: CellBox,
    window: &mut Window,
    cx: &mut App,
    mut record: impl FnMut(Option<&Shaped>, &mut App),
) -> CellOutcome {
    let roles = p.roles;
    let role = roles.for_cell(kind, column);
    let (left, width, y) = (at.left, at.width, at.y);
    let mut avail = width - 2.0 * CELL_PAD_PX;
    let mut x0 = left + CELL_PAD_PX;
    let error_icon = (kind == CellKind::Error).then(|| {
        let icon = rect(
            x0,
            y + (p.row_h - ERROR_ICON_PX) / 2.0,
            ERROR_ICON_PX,
            ERROR_ICON_PX,
        );
        x0 += ERROR_ICON_PX + ERROR_GAP_PX;
        avail -= ERROR_ICON_PX + ERROR_GAP_PX;
        icon
    });
    // The trailing chip's box width, measured first (the text gives way to it).
    let trail = match chip {
        Some(chip) if !p.fast => with_shape(
            cache,
            chip,
            &roles.chip,
            &roles.chip,
            avail.max(0.0),
            false,
            budget,
            scratch,
            window,
            cx,
            |s, _, _| s.width,
        ),
        _ => None,
    };
    if let Some(w) = trail {
        avail -= w + 2.0 * TRAIL_PAD_PX + 2.0 + TRAIL_GAP_PX;
    }
    let mut outcome = CellOutcome::default();
    let palette = p.palette;
    let row_h = p.row_h;
    let chips = !p.fast;
    let painted = with_shape(
        cache,
        text,
        role,
        &roles.chip,
        avail.max(0.0),
        p.fast,
        budget,
        scratch,
        window,
        cx,
        |s, window, cx| {
            if s.skeleton {
                return None;
            }
            let x = match align {
                Align::Start => x0,
                Align::End => x0 + avail - s.width,
                Align::Center => x0 + (avail - s.width) / 2.0,
            };
            paint_shaped(s, x, y, row_h, palette, chips, window, cx);
            record(Some(s), cx);
            Some((s.degraded, x + s.width))
        },
    );
    match painted.flatten() {
        Some((degraded, end)) => {
            outcome.degraded = degraded;
            if let Some(icon) = error_icon {
                let _ = window.paint_svg(
                    icon,
                    IconName::TriangleAlert.path().into(),
                    None,
                    Default::default(),
                    palette.text,
                    cx,
                );
            }
            if trail.is_some()
                && let Some(chip) = chip
            {
                let at = end + TRAIL_GAP_PX;
                let _ = with_shape(
                    cache,
                    chip,
                    &roles.chip,
                    &roles.chip,
                    f32::MAX,
                    false,
                    budget,
                    scratch,
                    window,
                    cx,
                    |s, window, cx| {
                        let boxed = rect(
                            at,
                            y + (row_h - CHIP_H_PX - 2.0) / 2.0,
                            s.width + 2.0 * TRAIL_PAD_PX + 2.0,
                            CHIP_H_PX + 2.0,
                        );
                        bordered(boxed, clear(), palette.seam_strong, 3.0, window);
                        let text_x = at + 1.0 + TRAIL_PAD_PX;
                        paint_shaped(s, text_x, y, row_h, palette, true, window, cx);
                    },
                );
            }
        }
        None => {
            paint_skeleton(left, width, y, row_h, at.row, at.col, palette, window);
            record(None, cx);
            outcome.degraded = true;
            outcome.skeleton = true;
        }
    }
    outcome
}

/// The text a cell paints for `kind` (the markers for `NULL` and `''`).
pub fn cell_text(kind: CellKind, text: &SharedString) -> SharedString {
    match kind {
        CellKind::Null => SharedString::new_static(NULL_MARKER),
        CellKind::Empty => SharedString::new_static(EMPTY_MARKER),
        _ => text.clone(),
    }
}

/// A placeholder row's static skeleton bar for column `col` of row `row` (layer 7).
#[allow(clippy::too_many_arguments)]
pub fn paint_skeleton(
    left: f32,
    width: f32,
    y: f32,
    row_h: f32,
    row: u64,
    col: usize,
    palette: &Palette,
    window: &mut Window,
) {
    // The mock's widths: 40 to 129 px by a fixed hash of the cell, at most the column less
    // 28 px, at least 24 px, never past the cell's padding.
    let h = row
        .wrapping_mul(7)
        .wrapping_add(col as u64)
        .wrapping_mul(2_654_435_761)
        % 90;
    let w = (40.0 + h as f32)
        .min(width - 28.0)
        .max(24.0)
        .min(width - 2.0 * CELL_PAD_PX);
    if w <= 0.0 {
        return;
    }
    window.paint_quad(fill(
        rect(
            left + CELL_PAD_PX,
            y + (row_h - SKELETON_H_PX) / 2.0,
            w,
            SKELETON_H_PX,
        ),
        palette.skeleton,
    ));
}

/// Records a painted run in the text registry (only while it is enabled).
#[allow(clippy::too_many_arguments)]
pub fn record_text(
    cx: &mut App,
    text: SharedString,
    source: TextSource,
    size_px: f32,
    painted_px: f32,
    truncated: bool,
    path: &'static str,
) {
    TextRegistry::record(cx, || RenderedText {
        text,
        source,
        size_px,
        painted_px,
        clipped: false,
        truncated,
        path: SharedString::new_static(path),
        visible: true,
        outside_card: false,
    });
}

/// The record paths, for the element.
pub mod paths {
    pub const CELL: &str = super::CELL_PATH;
    pub const HEADER: &str = super::HEADER_PATH;
    pub const ROW_NUMBER: &str = super::ROW_NUMBER_PATH;
}

/// Paints the decimal row number `n` right-aligned at `right` from the digit layouts.
#[allow(clippy::too_many_arguments)]
pub fn paint_row_number(
    roles: &Roles,
    n: u64,
    right: f32,
    y: f32,
    row_h: f32,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let mut digits = [0u8; 20];
    let mut len = 0;
    let mut v = n;
    loop {
        digits[len] = (v % 10) as u8;
        len += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let adv = roles.digit_advance;
    let mut x = right - len as f32 * adv;
    for i in (0..len).rev() {
        paint_layout(
            &roles.digits[digits[i] as usize],
            x,
            y,
            row_h,
            color,
            window,
            cx,
        );
        x += adv;
    }
}

/// The digits of `n` in base 10 (at least 1).
pub fn digit_count(n: u64) -> u32 {
    n.checked_ilog10().map_or(1, |d| d + 1)
}

/// Runs `f` inside `mask` (intersected with the current mask).
pub fn masked<R>(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    f: impl FnOnce(&mut Window) -> R,
) -> R {
    window.with_content_mask(Some(ContentMask { bounds }), f)
}
