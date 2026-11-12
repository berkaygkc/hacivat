//! The grid, painted with a GPUI `canvas`: only visible rows x visible columns are shaped and
//! painted. Shaped lines live in a frame-generation LRU so a row that stays on screen is
//! shaped once.

use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::time::Instant;

use hacivat_ui_core::fonts::{SYNTH_SCALE, font_for};
use hacivat_ui_core::gpui_surface::{
    App, Bounds, ContentMask, Font, FontId, Hsla, IntoElement, Pixels, ShapedLine, SharedString,
    Styled, TextAlign, TextRun, Window, canvas, fill, point, px,
};
use hacivat_ui_core::metrics::snap_to_device_px;
use hacivat_ui_core::theme::{Caps, ThemeTokens, TypeStyle};

use crate::scroll::{ScrollPos, scrollbar_thumb, visible_rows};
use crate::source::{ColumnMeta, GridDataSource, NULL};

/// Capacity of the shaped-line cache.
pub const LINE_CACHE_CAPACITY: usize = 4_096;

/// DESIGN.md `components.grid-row-number.width`. The component block is not part of the
/// generated tokens, so the value is restated here; the column widens to fit 8-digit numbers.
const ROW_NUMBER_MIN_WIDTH: f32 = 46.0;

/// An LRU keyed by `(style, text)` whose recency unit is the frame: every entry touched in
/// a frame carries that frame's generation, and eviction drops the oldest generations first.
/// Entries touched in the current frame are never evicted. Lookups take `&str`, so a hit
/// allocates nothing.
pub struct FrameLru<S, V> {
    map: HashMap<S, HashMap<Box<str>, (V, u64)>>,
    len: usize,
    capacity: usize,
    generation: u64,
}

impl<S: Hash + Eq + Copy, V> FrameLru<S, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            len: 0,
            capacity,
            generation: 1,
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Starts a new frame generation.
    pub fn begin_frame(&mut self) {
        self.generation += 1;
    }

    /// Returns the cached value, building it on a miss. The bool is true on a miss.
    pub fn get_or_insert_with(
        &mut self,
        style: S,
        text: &str,
        build: impl FnOnce() -> V,
    ) -> (&V, bool) {
        let generation = self.generation;
        let hit = match self.map.get_mut(&style).and_then(|m| m.get_mut(text)) {
            Some(entry) => {
                entry.1 = generation;
                true
            }
            None => false,
        };
        if !hit {
            if self.len >= self.capacity {
                self.evict();
            }
            self.map
                .entry(style)
                .or_default()
                .insert(text.into(), (build(), generation));
            self.len += 1;
        }
        let value = &self.map[&style][text].0;
        (value, !hit)
    }

    /// Drops the least recently used generations until the cache is at 7/8 of capacity.
    fn evict(&mut self) {
        let target = self.capacity - self.capacity / 8;
        if self.len <= target {
            return;
        }
        let mut gens: Vec<u64> = self
            .map
            .values()
            .flat_map(|m| m.values().map(|e| e.1))
            .collect();
        let drop = gens.len() - target;
        let (_, nth, _) = gens.select_nth_unstable(drop - 1);
        let cutoff = (*nth).min(self.generation - 1);
        let mut len = 0;
        for m in self.map.values_mut() {
            m.retain(|_, e| e.1 > cutoff);
            len += m.len();
        }
        self.map.retain(|_, m| !m.is_empty());
        self.len = len;
    }
}

/// Cache style key: a resolved font, its size and the text colour baked into the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineStyle {
    font: FontId,
    size_bits: u32,
    color_bits: [u32; 4],
}

/// A text role in the grid: token typography plus a colour.
struct Role {
    font: Font,
    font_id: Option<FontId>,
    size: f32,
    color: Hsla,
}

impl Role {
    /// A synthesized small-caps style (the NULL marker, pre-cased "NULL") shapes its capitals at
    /// `SYNTH_SCALE` of the size; a shaped line has no per-glyph tracking.
    fn new(style: &TypeStyle, color: Hsla) -> Self {
        let size = match style.caps {
            Caps::SynthesizedSmallCaps => style.size_px * SYNTH_SCALE,
            Caps::Normal => style.size_px,
        };
        Self {
            font: font_for(style),
            font_id: None,
            size,
            color,
        }
    }
}

#[derive(Clone, Copy)]
enum RoleId {
    Cell = 0,
    Null = 1,
    RowNumber = 2,
    HeaderName = 3,
    HeaderType = 4,
}

/// Per-frame CPU samples for the paint path, for the bench report.
#[derive(Default)]
pub struct PaintStats {
    /// Time inside the grid's paint closure, in ms.
    pub paint_ms: Vec<f64>,
    /// From the start of `render` to the end of paint (render + layout + prepaint + paint), in ms.
    pub frame_cpu_ms: Vec<f64>,
    /// Shaped-line cache misses per frame.
    pub misses: Vec<u32>,
    /// Visible cells per frame.
    pub cells: Vec<u32>,
}

/// Everything the canvas needs across frames.
pub struct GridState {
    source: Rc<dyn GridDataSource>,
    columns: Vec<ColumnMeta>,
    /// Left edge of each column in content space, plus the total width at the end.
    col_x: Vec<f32>,
    roles: [Role; 5],
    cache: FrameLru<LineStyle, ShapedLine>,
    cell_buf: String,
    /// Body viewport (excluding header, row numbers and scrollbar) from the last paint.
    pub body_size: (f32, f32),
    pub render_started: Option<Instant>,
    pub stats: PaintStats,
}

impl GridState {
    pub fn new(source: Rc<dyn GridDataSource>, tokens: &ThemeTokens) -> Self {
        let columns: Vec<ColumnMeta> = (0..source.col_count()).map(|c| source.column(c)).collect();
        let mut col_x = Vec::with_capacity(columns.len() + 1);
        let mut x = 0.0;
        for c in &columns {
            col_x.push(x);
            x += c.width_px;
        }
        col_x.push(x);
        let (t, c) = (&tokens.type_ramp, &tokens.colors);
        let roles = [
            Role::new(&t.body, c.text.to_hsla()),
            Role::new(&t.label_sc, c.text_3.to_hsla()),
            Role::new(&t.micro, c.text_3.to_hsla()),
            Role::new(&t.body_strong, c.text.to_hsla()),
            Role::new(&t.mono_kbd, c.text_3.to_hsla()),
        ];
        Self {
            source,
            columns,
            col_x,
            roles,
            cache: FrameLru::new(LINE_CACHE_CAPACITY),
            cell_buf: String::with_capacity(64),
            body_size: (0.0, 0.0),
            render_started: None,
            stats: PaintStats::default(),
        }
    }

    pub fn row_count(&self) -> u64 {
        self.source.row_count()
    }

    pub fn content_width(&self) -> f32 {
        *self.col_x.last().unwrap_or(&0.0)
    }

    /// Shapes `text` in `role` (or returns the cached line).
    fn shaped(&mut self, role: RoleId, text: &str, window: &Window) -> (ShapedLine, bool) {
        let r = &mut self.roles[role as usize];
        let font_id = *r
            .font_id
            .get_or_insert_with(|| window.text_system().resolve_font(&r.font));
        let key = LineStyle {
            font: font_id,
            size_bits: r.size.to_bits(),
            color_bits: [r.color.h, r.color.s, r.color.l, r.color.a].map(f32::to_bits),
        };
        let (font, size, color) = (&r.font, r.size, r.color);
        let (line, miss) = self.cache.get_or_insert_with(key, text, || {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window.text_system().shape_line(
                SharedString::from(text.to_string()),
                px(size),
                &[run],
                None,
            )
        });
        (line.clone(), miss)
    }
}

/// Builds the grid canvas for the current scroll state.
pub fn grid_canvas(
    state: Rc<std::cell::RefCell<GridState>>,
    pos: ScrollPos,
    scroll_x: f32,
    tokens: &'static ThemeTokens,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            let mut state = state.borrow_mut();
            paint_grid(&mut state, bounds, pos, scroll_x, tokens, window, cx);
        },
    )
    .size_full()
}

#[allow(clippy::too_many_arguments)]
fn paint_grid(
    state: &mut GridState,
    bounds: Bounds<Pixels>,
    pos: ScrollPos,
    scroll_x: f32,
    tokens: &'static ThemeTokens,
    window: &mut Window,
    cx: &mut App,
) {
    let started = Instant::now();
    state.cache.begin_frame();
    let mut misses = 0u32;
    let mut cells = 0u32;

    let (sp, colors) = (&tokens.spacing, &tokens.colors);
    let scale = window.scale_factor();
    let snap = |v: f32| snap_to_device_px(v, scale);
    let row_h = sp.grid_row_height;
    let header_h = sp.grid_header_height;
    let pad = sp.lg;
    let seam_w = 1.0;
    let scrollbar_w = sp.md;
    let total_rows = state.row_count();

    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);

    // Row-number column: token width, widened to fit the largest row number.
    let widest = total_rows.max(1).to_string();
    let (widest_line, miss) = state.shaped(RoleId::RowNumber, &widest, window);
    misses += miss as u32;
    let rn_w = snap(
        ROW_NUMBER_MIN_WIDTH
            .max(f32::from(widest_line.width()) + 2.0 * sp.sm)
            .ceil(),
    );

    let body_left = left + rn_w;
    let body_top = top + header_h;
    let body_w = (width - rn_w - scrollbar_w).max(0.0);
    let body_h = (height - header_h).max(0.0);
    state.body_size = (body_w, body_h);

    let max_x = (state.content_width() - body_w).max(0.0);
    let scroll_x = scroll_x.clamp(0.0, max_x);
    let rows = visible_rows(pos, body_h, row_h, total_rows);

    // Visible columns: binary search over left edges.
    let n_cols = state.columns.len();
    let first_col = state.col_x[1..]
        .partition_point(|&r| r <= scroll_x)
        .min(n_cols);
    let last_col = state.col_x[..n_cols].partition_point(|&l| l < scroll_x + body_w);
    let cols = first_col..last_col.max(first_col);

    let body_mask = ContentMask {
        bounds: Bounds::new(point(px(body_left), px(body_top)), hsize(body_w, body_h)),
    };
    let row_top = |i: u64| body_top - pos.offset_px + (i - rows.start) as f32 * row_h;
    let col_left = |c: usize, col_x: &[f32]| body_left + col_x[c] - scroll_x;

    // Backgrounds: gutter for row numbers and scrollbar track, head for the header band.
    window.paint_quad(fill(
        rect(left, body_top, rn_w, body_h),
        colors.gutter.to_hsla(),
    ));
    window.paint_quad(fill(
        rect(left, top, width, header_h),
        colors.head.to_hsla(),
    ));
    window.paint_quad(fill(
        rect(left + width - scrollbar_w, body_top, scrollbar_w, body_h),
        colors.gutter.to_hsla(),
    ));

    // Horizontal seams under each visible row (body and row-number gutter).
    let seam_soft = colors.seam_soft.to_hsla();
    window.with_content_mask(
        Some(ContentMask {
            bounds: Bounds::new(point(px(left), px(body_top)), hsize(rn_w + body_w, body_h)),
        }),
        |window| {
            for r in rows.clone() {
                let y = snap(row_top(r) + row_h - seam_w);
                window.paint_quad(fill(rect(left, y, rn_w + body_w, seam_w), seam_soft));
            }
        },
    );

    // Vertical seams at each visible column's right edge (header and body).
    window.with_content_mask(
        Some(ContentMask {
            bounds: Bounds::new(point(px(body_left), px(top)), hsize(body_w, height)),
        }),
        |window| {
            for c in cols.clone() {
                let x = snap(col_left(c + 1, &state.col_x) - seam_w);
                window.paint_quad(fill(rect(x, top, seam_w, height), seam_soft));
            }
        },
    );
    // Row-number column edge and header bottom.
    let seam = colors.seam.to_hsla();
    window.paint_quad(fill(
        rect(snap(body_left - seam_w), top, seam_w, height),
        seam,
    ));
    window.paint_quad(fill(
        rect(left, snap(body_top - seam_w), width, seam_w),
        seam,
    ));

    // Cells, clipped per column so long text never bleeds into the neighbour.
    let line_h = px(row_h);
    let text_x_pad = pad;
    for c in cols.clone() {
        let cl = col_left(c, &state.col_x);
        let cw = state.columns[c].width_px;
        let numeric = state.columns[c].numeric;
        let mask = ContentMask {
            bounds: Bounds::new(point(px(cl), px(body_top)), hsize(cw - seam_w, body_h)),
        }
        .intersect(&body_mask);
        window.with_content_mask(Some(mask), |window| {
            for r in rows.clone() {
                let mut buf = std::mem::take(&mut state.cell_buf);
                state.source.write_cell(r, c, &mut buf);
                let role = if buf == NULL {
                    RoleId::Null
                } else {
                    RoleId::Cell
                };
                let (line, miss) = state.shaped(role, &buf, window);
                state.cell_buf = buf;
                misses += miss as u32;
                cells += 1;
                let x = if numeric {
                    cl + cw - text_x_pad - f32::from(line.width())
                } else {
                    cl + text_x_pad
                };
                let _ = line.paint(
                    point(px(x), px(row_top(r))),
                    line_h,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
    }

    // Row numbers (1-based, right-aligned).
    window.with_content_mask(
        Some(ContentMask {
            bounds: Bounds::new(point(px(left), px(body_top)), hsize(rn_w - seam_w, body_h)),
        }),
        |window| {
            let mut label = String::with_capacity(12);
            for r in rows.clone() {
                use std::fmt::Write as _;
                label.clear();
                let _ = write!(label, "{}", r + 1);
                let (line, miss) = state.shaped(RoleId::RowNumber, &label, window);
                misses += miss as u32;
                let x = left + rn_w - sp.sm - f32::from(line.width());
                let _ = line.paint(
                    point(px(x), px(row_top(r))),
                    line_h,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        },
    );

    // Header: column name over its mono type subtitle.
    let name_lh = tokens.type_ramp.body_strong.line_height;
    let type_lh = tokens.type_ramp.mono_kbd.line_height;
    let gap = sp.s_2xs;
    let name_y = top + ((header_h - name_lh - gap - type_lh) / 2.0).max(0.0);
    let type_y = name_y + name_lh + gap;
    window.with_content_mask(
        Some(ContentMask {
            bounds: Bounds::new(point(px(body_left), px(top)), hsize(body_w, header_h)),
        }),
        |window| {
            for c in cols.clone() {
                let cl = col_left(c, &state.col_x);
                let cw = state.columns[c].width_px;
                let name = state.columns[c].name.clone();
                let type_label = state.columns[c].type_label.clone();
                let mask = ContentMask {
                    bounds: Bounds::new(point(px(cl), px(top)), hsize(cw - seam_w, header_h)),
                };
                window.with_content_mask(Some(mask), |window| {
                    let (n, miss_n) = state.shaped(RoleId::HeaderName, &name, window);
                    let (t, miss_t) = state.shaped(RoleId::HeaderType, &type_label, window);
                    misses += miss_n as u32 + miss_t as u32;
                    let x = cl + pad;
                    let _ = n.paint(
                        point(px(x), px(name_y)),
                        px(name_lh),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    let _ = t.paint(
                        point(px(x), px(type_y)),
                        px(type_lh),
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                });
            }
        },
    );

    // Vertical scrollbar thumb.
    let (thumb_top, thumb_h) = scrollbar_thumb(pos, body_h, row_h, total_rows);
    let inset = sp.s_2xs;
    let thumb_w = scrollbar_w - 2.0 * inset;
    window.paint_quad(
        fill(
            rect(
                left + width - scrollbar_w + inset,
                body_top + thumb_top,
                thumb_w,
                thumb_h,
            ),
            colors.seam_strong.to_hsla(),
        )
        // The pill token (9999) clamped to the quad, as CSS does; GPUI does not clamp and
        // an over-sized radius makes the quad disappear.
        .corner_radii(px(tokens.radii.full.min(thumb_w / 2.0))),
    );

    let now = Instant::now();
    let stats = &mut state.stats;
    stats.paint_ms.push(ms(now - started));
    if let Some(r) = state.render_started.take() {
        stats.frame_cpu_ms.push(ms(now - r));
    }
    stats.misses.push(misses);
    stats.cells.push(cells);
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn hsize(w: f32, h: f32) -> hacivat_ui_core::gpui_surface::Size<Pixels> {
    hacivat_ui_core::gpui_surface::Size {
        width: px(w.max(0.0)),
        height: px(h.max(0.0)),
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), hsize(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_marker_uses_synthesized_small_caps() {
        // The NULL literal is pre-cased capitals; label-sc shrinks them like the mock.
        let t = &hacivat_ui_core::theme::DARK.type_ramp;
        let color = hacivat_ui_core::gpui_surface::hsla(0.0, 0.0, 0.0, 1.0);
        let null = Role::new(&t.label_sc, color);
        assert!(
            (null.size - t.label_sc.size_px * hacivat_ui_core::fonts::SYNTH_SCALE).abs() < 1e-6
        );
        assert_eq!(Role::new(&t.body, color).size, t.body.size_px);
    }

    #[test]
    fn lru_hits_do_not_rebuild() {
        let mut lru: FrameLru<u8, usize> = FrameLru::new(8);
        let mut builds = 0;
        for _ in 0..3 {
            lru.begin_frame();
            let (v, _) = lru.get_or_insert_with(0, "a", || {
                builds += 1;
                7
            });
            assert_eq!(*v, 7);
        }
        assert_eq!(builds, 1);
    }

    #[test]
    fn lru_keys_on_style_and_text() {
        let mut lru: FrameLru<u8, &str> = FrameLru::new(8);
        assert!(lru.get_or_insert_with(0, "x", || "s0").1);
        assert!(lru.get_or_insert_with(1, "x", || "s1").1);
        assert_eq!(*lru.get_or_insert_with(1, "x", || "zz").0, "s1");
        assert_eq!(lru.len(), 2);
    }

    #[test]
    fn lru_stays_bounded_and_evicts_oldest_frames_first() {
        let mut lru: FrameLru<u8, u32> = FrameLru::new(64);
        // An "always visible" entry, touched every frame.
        for frame in 0..100u32 {
            lru.begin_frame();
            lru.get_or_insert_with(0, "header", || 0);
            for i in 0..10 {
                lru.get_or_insert_with(1, &format!("{frame}-{i}"), || frame);
            }
            assert!(lru.len() <= 64, "len {}", lru.len());
        }
        // The hot entry survived every eviction; the newest frame is intact.
        assert!(!lru.get_or_insert_with(0, "header", || 1).1);
        assert!(!lru.get_or_insert_with(1, "99-0", || 1).1);
        // A long-gone frame was evicted.
        assert!(lru.get_or_insert_with(1, "0-0", || 1).1);
    }

    #[test]
    fn lru_never_evicts_the_current_frame() {
        let mut lru: FrameLru<u8, u32> = FrameLru::new(4);
        lru.begin_frame();
        for i in 0..10 {
            lru.get_or_insert_with(0, &i.to_string(), || i);
        }
        for i in 0..10 {
            assert!(!lru.get_or_insert_with(0, &i.to_string(), || 99).1, "{i}");
        }
    }
}
