//! The grid element (HV-GRID-01, -02, -03; Task 20).
//!
//! **Frame lifecycle (HV-GRID-01).**
//! 1. `request_layout`: the grid takes its parent's size. The accessibility declaration (R-M3-32:
//!    role `Grid`, the label, the row and column counts; no synthetic children in M3a) belongs to
//!    the view's root, the element that tracks the focus (`GridView::a11y_node`, Task 30).
//! 2. `prepaint` ([`prepaint_frame`]): one `begin_frame` (one snapshot load); generation and
//!    row-count changes; the geometry (header 36 px, the row-number gutter ≥ 46 px, 9 px
//!    tracks); the frame's coalesced scroll deltas and the velocity (fast-scroll); the visible
//!    rows × columns; at most one `ensure_rows`; the hitbox; the frame's events.
//! 3. `paint` ([`paint_frame`]): the regions body → header → gutter → corner → scrollbars, each
//!    in its content mask, the layers of [`super::layers`] inside each; the pointer listeners.
//!
//! ## m3c-t35 (M3c Task 35)
//! - **The view map.** Rows are painted in view order: view row `r` shows model row
//!   `GridView::view_map().view_to_model(r)`; the gutter numbers view rows; a filter's kept rows
//!   are all the grid scrolls over. A re-run drops the map; rows appended after a sort join its
//!   unsorted tail.
//! - **The frozen layer.** The body and the header are painted in two segments with disjoint
//!   clips: the frozen columns (never scrolled, clipped to the frozen width), then the scrolling
//!   columns (scrolled by `left_x`, clipped right of the frozen width), with a 1 px
//!   `seam-strong` edge on the last frozen column (header and rows). Each segment runs the same
//!   layers; with nothing frozen there is one segment and the frame is M3a's. The fast-scroll
//!   budget is shared (frozen cells are body cells), the frozen ones first, so the columns kept
//!   in view are the last to fall back to skeleton bars; both segments reuse the same kept
//!   column buffer, so a frame of cache hits still allocates nothing.
//! - **Resize.** The hovered header edge shows a 1 px `text-2` handle; while dragging, the
//!   guide is dashed `text-2` down the header and the body (the view adds the width tag).

use std::ops::Range;
use std::panic::Location;
use std::rc::Rc;

// ## m3c-t36: the sort arrows.
use hacivat_ui::icon::IconName;
use hacivat_ui::text::{TextRegistry, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, Element, ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, LayoutId, Pixels, SharedString, Style, Window, fill, relative,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, ThemeTokens};

use super::layers::{
    self, CELL_PAD_PX, CellBox, CellPaint, Palette, Roles, Snap, cell_text, masked, outline,
    paint_cell_content, paint_row_number, paint_skeleton, rect,
};
use super::pointer;
use super::scrollbar;
use super::text_cache::{Budget, with_shape};
use super::{
    FAST_SCROLL_ROWS_PER_FRAME, FAST_SHAPES_PER_FRAME, FrameStats, Geometry, ROW_NUMBER_MIN_DIGITS,
    ROW_NUMBER_MIN_PX, SCROLLBAR_PX,
};
use crate::model::{ColumnLayout, ScrollState, ViewMap};
use crate::source::{Align, CellKind, ColumnKind, GridFrame, ViewRow};
use crate::view::{GridEvent, GridView};

/// The grid's element. Its accessibility node is not here: the view's root, the element that
/// tracks the focus, declares it (`GridView::a11y_node`; ## m3a-t30-results-view).
pub(crate) struct GridElement {
    view: Entity<GridView>,
    id: ElementId,
}

impl GridElement {
    pub(crate) fn new(view: Entity<GridView>) -> Self {
        let id = ElementId::from(("hv-grid", view.entity_id()));
        GridElement { view, id }
    }
}

/// One frame's state from prepaint to paint.
pub(crate) struct FramePrepaint {
    frame: Rc<dyn GridFrame>,
    geom: Geometry,
    hitbox: Hitbox,
    fast: bool,
    tokens: &'static ThemeTokens,
    light: bool,
    stats: FrameStats,
}

impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<FramePrepaint>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FramePrepaint> {
        Some(
            self.view
                .update(cx, |v, cx| prepaint_frame(v, bounds, window, cx)),
        )
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        prepaint: &mut Option<FramePrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(frame) = prepaint.take() else {
            return;
        };
        let hitbox = frame.hitbox.clone();
        let degraded = self
            .view
            .update(cx, |v, cx| paint_frame(v, frame, window, cx));
        pointer::register(&self.view, &hitbox, window, cx);
        if degraded {
            // A fast-scroll frame painted below full quality: the next frame repaints, at full
            // quality once the velocity is back under the threshold.
            let view = self.view.clone();
            window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
        }
    }
}

// ## m3c-t36: the sort indicator (NOTES surface 18): the arrow's size, its gap after the name
// (the mock's 6 px), and the gap between the arrow and the level number.
const SORT_ARROW_PX: f32 = 12.0;
const SORT_GAP_PX: f32 = 6.0;
const SORT_NUMBER_GAP_PX: f32 = 1.0;

/// Generation and row-count changes (HV-GRID-05 lifecycle).
fn sync_data(v: &mut GridView, frame: &dyn GridFrame, cx: &mut Context<GridView>) {
    let (generation, _version) = frame.generation();
    let count = frame.row_count();
    let first = v.data.generation.is_none() && !v.data.replaced;
    let new_generation = v.data.replaced || v.data.generation != Some(generation);
    if new_generation {
        let columns = v.source.columns();
        v.adopt_columns(columns, true);
        if !first {
            // A re-run: both models start over; a focus is kept pending until rows arrive.
            // ## m3c-t35: the new rows have no order yet; the host sorts them again.
            v.drop_view_map();
            v.scroll.on_generation_changed();
            let map = ViewMap::identity(count.loaded);
            v.selection.on_generation_changed(&map, v.layout.visible());
            v.selection_dirty = true;
            if let Some(focus) = v.selection.focus() {
                cx.emit(GridEvent::FocusChanged(Some(focus)));
            }
        }
        v.data.generation = Some(generation);
        v.data.replaced = false;
    } else if count.loaded > v.data.count.loaded {
        // An append keeps the selection and places a pending focus (## m3c-t35: through the
        // view map, which takes the new rows into a sort's unsorted tail).
        v.note_loaded(count.loaded);
        let map = v.view_map().clone();
        if v.selection.on_rows_loaded(&map, v.layout.visible()) {
            v.selection_dirty = true;
            cx.emit(GridEvent::FocusChanged(v.selection.focus()));
        }
    }
    // ## m3a-t30-results-view: the view draws "No rows" from the frame's state; a flip asks
    // for a render, so the label follows the frame it belongs to.
    let showed_no_rows = v.shows_no_rows();
    v.data.count = count;
    // ## m3b-t01b-seams: `GridView::view_map` answers the identity over the loaded rows.
    v.note_loaded(count.loaded);
    // ## m3c-t35: a filter scrolls over its kept rows only.
    v.data.rows = v.scroll_rows();
    if v.shows_no_rows() != showed_no_rows {
        cx.notify();
    }
}

/// The row-number gutter's width for `digits` digits: 46 px up to five, one digit advance more
/// for each digit beyond, on whole device pixels.
fn gutter_width(digits: u32, advance: f32, snap: Snap) -> f32 {
    let extra = digits.saturating_sub(ROW_NUMBER_MIN_DIGITS) as f32 * advance;
    snap.v((ROW_NUMBER_MIN_PX + extra).ceil())
}

/// The prepaint half of a frame.
pub(crate) fn prepaint_frame(
    v: &mut GridView,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut Context<GridView>,
) -> FramePrepaint {
    #[cfg(feature = "bench")]
    {
        v.paint.prepaint_started = Some(std::time::Instant::now());
    }
    let theme = *Theme::get(cx);
    let tokens = theme.tokens;
    let scale = window.scale_factor();
    if v.paint.scale != scale {
        // Shapes are scale-free, but glyph metrics are not exact across scales: start over.
        v.paint.cache.clear();
        v.paint.roles = None;
        v.paint.scale = scale;
    }
    if v.paint.roles.is_none() {
        v.paint.roles = Some(Roles::new(tokens, window));
    }
    let snap = Snap(scale);
    let frame = v.source.begin_frame(cx);
    // ## m3b-t01b-seams: Task 38's axis swap (identity until then).
    let frame = super::transpose::frame(v, frame);
    sync_data(v, &*frame, cx);

    let row_h = v.scroll.row_height();
    let header_h = tokens.spacing.grid_header_height;
    let digit_advance = v.paint.roles.as_ref().map_or(0.0, |r| r.digit_advance);
    let last_number = v.data.known_total().unwrap_or(v.data.count.loaded).max(1);
    let gutter_w = gutter_width(layers::digit_count(last_number), digit_advance, snap);

    let (x0, y0) = (f32::from(bounds.left()), f32::from(bounds.top()));
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let body_w = (w - gutter_w - SCROLLBAR_PX).max(0.0);
    // The column auto-size takes its share of the frame before the columns are laid out.
    let autosize = super::autosize::step(v, &*frame, body_w, window, cx);
    let autosize_measures = autosize.measured;
    if (v.autosize.pending() || v.autosize.fits_pending()) && autosize_measures > 0 {
        let view = cx.entity();
        window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
    }
    // ## m3c-t35: an auto-fit's width is the user's; the host keeps it like a dragged one.
    for (col, width) in autosize.fitted {
        cx.emit(GridEvent::ColumnResized { col, width });
    }
    // ## m3c-t35: the frozen columns that fit this body, then the scroll over the rest.
    v.layout.fit_frozen(body_w);
    let frozen = v.layout.frozen_shown();
    let frozen_w = (v.layout.frozen_width() as f32).min(body_w);
    let content_w = v.layout.total_width();
    let htrack_h = if content_w > f64::from(body_w) {
        SCROLLBAR_PX
    } else {
        0.0
    };
    let body_h = (h - header_h - htrack_h).max(0.0);
    let body_x = x0 + gutter_w;
    let body_y = y0 + header_h;
    let scroll_w = (body_w - frozen_w).max(0.0);

    v.scroll.set_viewport(scroll_w, body_h);
    v.scroll.on_row_count(v.data.rows);
    v.scroll.set_content_width(v.layout.scroll_width());
    v.scroll.apply_pending();
    let velocity = v.scroll.velocity_rows_per_frame();
    let fast = velocity > FAST_SCROLL_ROWS_PER_FRAME;

    let rows = v.scroll.visible_rows();
    let cols = v.layout.scrolling_range(v.scroll.left_x, scroll_w);
    let geom = Geometry {
        bounds,
        corner: rect(x0, y0, gutter_w, header_h),
        header: rect(body_x, y0, body_w, header_h),
        gutter: rect(x0, body_y, gutter_w, body_h),
        body: rect(body_x, body_y, body_w, body_h),
        vtrack: rect(x0 + w - SCROLLBAR_PX, body_y, SCROLLBAR_PX, body_h),
        htrack: (htrack_h > 0.0).then(|| rect(body_x, y0 + h - htrack_h, body_w, htrack_h)),
        row_h,
        rows: rows.clone(),
        cols: cols.clone(),
        scale,
        frozen,
        frozen_w,
    };

    // Rows past the loaded ones are placeholders: ask for them, once per frame.
    let mut ensure_rows = 0;
    let loaded = v.data.count.loaded;
    let total = v.data.known_total();
    let wants_more = v.data.count.more || total.is_some_and(|t| t > loaded);
    let prefetch = v.scroll.prefetch_rows();
    if wants_more && prefetch.end > loaded {
        let upto = total.map_or(prefetch.end, |t| prefetch.end.min(t));
        v.source.ensure_rows(upto, cx);
        ensure_rows = 1;
    }

    // The hover follows the pointer, not the row it was on: a wheel scroll moves the rows under
    // a still pointer.
    if v.pointer.drag.is_none()
        && let Some(p) = v.pointer.position
    {
        (v.pointer.hover, v.pointer.border) = v.hover_at(&geom, p);
    }

    // ## m3c-t35 fix round 1 (I3): a drag ends on blur and on window deactivation.
    v.watch_drag_end(window, cx);
    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
    // ## m3b-t01b-seams: Task 40's AccessKit rows and cells (nothing until then).
    super::a11y_rows::prepaint(v, &*frame, &geom, window, cx);
    v.note_visible_region(rows, cols, cx);
    v.flush_frame_events(cx);
    v.geometry = Some(geom.clone());
    FramePrepaint {
        frame,
        geom,
        hitbox,
        fast,
        tokens,
        light: theme.mode == ThemeMode::Light,
        stats: FrameStats {
            fast,
            velocity: velocity.min(f64::from(f32::MAX)) as f32,
            ensure_rows,
            autosize_measures,
            ..FrameStats::default()
        },
    }
}

/// The paint half of a frame; answers whether anything was painted below full quality.
pub(crate) fn paint_frame(
    v: &mut GridView,
    f: FramePrepaint,
    window: &mut Window,
    cx: &mut Context<GridView>,
) -> bool {
    #[cfg(feature = "bench")]
    let paint_started = std::time::Instant::now();
    let palette = Palette::new(&f.tokens.colors, f.light);
    let snap = Snap(f.geom.scale);
    let recording = cx.try_global::<TextRegistry>().is_some_and(|r| r.enabled);
    let mut stats = f.stats;
    // The body's shaping budget (fast-scroll: capped). The header is never in fast-scroll and
    // is not charged to it (review round 1).
    let mut budget = Budget::new(if f.fast {
        FAST_SHAPES_PER_FRAME
    } else {
        u32::MAX
    });
    let mut header_budget = Budget::new(u32::MAX);
    v.paint.cache.begin_frame();
    #[cfg(feature = "test-support")]
    if let Some(cells) = v.paint.recorded.as_mut() {
        cells.clear();
    }
    #[cfg(feature = "test-support")]
    if let Some(headers) = v.paint.recorded_headers.as_mut() {
        headers.clear();
    }
    // ## m3c-t36
    #[cfg(feature = "test-support")]
    {
        v.paint.recorded_sort_marks.clear();
        v.paint.recorded_tail_y = None;
        // ## m3c-t37
        v.paint.recorded_find_marks.clear();
    }
    // Keep the tips of the frame before (in the other kept buffer), to tell whether the
    // view's tooltip areas have to be rebuilt.
    std::mem::swap(&mut v.paint.header_tips, &mut v.paint.header_tips_before);
    v.paint.header_tips.clear();

    let g = &f.geom;
    // Layer 1 under every region: the panel.
    window.paint_quad(fill(g.bounds, palette.panel));
    masked(g.body, window, |window| {
        paint_body(
            v,
            &f,
            &palette,
            snap,
            recording,
            &mut budget,
            &mut stats,
            window,
            cx,
        )
    });
    masked(g.header, window, |window| {
        paint_header(
            v,
            &f,
            &palette,
            snap,
            recording,
            &mut header_budget,
            window,
            cx,
        )
    });
    masked(g.gutter, window, |window| {
        paint_gutter(v, &f, &palette, snap, recording, window, cx)
    });
    masked(g.corner, window, |window| {
        paint_corner(v, &f, &palette, snap, window)
    });
    masked(g.vtrack, window, |window| {
        scrollbar::paint_vertical(
            g.vtrack,
            &v.scroll,
            v.data.count.loaded,
            &palette,
            snap,
            window,
        )
    });
    if let Some(track) = g.htrack {
        masked(track, window, |window| {
            scrollbar::paint_horizontal(track, &v.scroll, &palette, snap, window)
        });
    }

    if v.paint.header_tips != v.paint.header_tips_before {
        // The view's tooltip areas were built from the frame before: render again.
        let view = cx.entity();
        window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
    }
    stats.shapes = budget.shapes;
    stats.header_shapes = header_budget.shapes;
    stats.cell_allocs = budget.allocs + header_budget.allocs;
    let degraded = stats.degraded > 0;
    v.paint.log.push(stats);
    #[cfg(feature = "bench")]
    {
        let now = std::time::Instant::now();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let b = &mut v.paint.bench;
        b.paint_ms.push(ms(now - paint_started));
        if let Some(started) = v.paint.prepaint_started.take() {
            b.frame_cpu_ms.push(ms(now - started));
        }
        b.shapes.push(stats.shapes);
        b.cells.push(stats.cells);
        b.fast.push(stats.fast);
    }
    degraded
}

// ## m3c-t35
/// A run of visible columns painted with one horizontal offset inside one clip: the scrolling
/// columns (scrolled by `left_x`, right of the frozen layer) or the frozen ones (never
/// scrolled).
#[derive(Debug, Clone)]
struct Segment {
    cols: Range<usize>,
    /// Subtracted from a column's content x (`left_x`, or 0 for the frozen columns).
    shift: f64,
    /// The clip's left and right, window x.
    x0: f32,
    x1: f32,
    frozen: bool,
}

/// The frame's segments, in paint order: the frozen columns (none with nothing frozen), then
/// the scrolling ones. Their clips are disjoint, so the order only decides which spends the
/// fast-scroll budget first.
fn segments(v: &GridView, g: &Geometry) -> [Option<Segment>; 2] {
    let (left, right) = (f32::from(g.body.left()), f32::from(g.body.right()));
    let pane = (left + g.frozen_w).min(right);
    [
        (g.frozen > 0).then_some(Segment {
            cols: 0..g.frozen,
            shift: 0.0,
            x0: left,
            x1: pane,
            frozen: true,
        }),
        Some(Segment {
            cols: g.cols.clone(),
            shift: v.scroll.left_x,
            x0: pane,
            x1: right,
            frozen: false,
        }),
    ]
}

/// The body region: the zebra, each segment's layers, the frozen edge and the resize guide.
#[allow(clippy::too_many_arguments)]
fn paint_body(
    v: &mut GridView,
    f: &FramePrepaint,
    palette: &Palette,
    snap: Snap,
    recording: bool,
    budget: &mut Budget,
    stats: &mut FrameStats,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let g = &f.geom;
    let (body_x, body_y) = (f32::from(g.body.left()), f32::from(g.body.top()));
    let body_right = f32::from(g.body.right());
    let (top, bottom) = (body_y, f32::from(g.body.bottom()));
    let row_h = g.row_h;

    // Layer 2: zebra (a setting, off in M3a).
    if v.config.zebra {
        for r in g.rows.clone().filter(|r| r % 2 == 1) {
            let y = body_y + v.scroll.row_top(r) as f32;
            window.paint_quad(fill(
                rect(body_x, y, body_right - body_x, row_h),
                palette.hover,
            ));
        }
    }

    // ## m3c-t35: the view map (an `Arc` clone) and the segments.
    let map = v.view_map().clone();
    for seg in segments(v, g).into_iter().flatten() {
        let clip = rect(seg.x0, top, seg.x1 - seg.x0, bottom - top);
        masked(clip, window, |window| {
            paint_body_segment(
                v, f, &seg, &map, palette, snap, recording, budget, stats, window, cx,
            )
        });
    }
    if g.frozen > 0 {
        // The frozen layer's edge: 1 px `seam-strong` on the last frozen column.
        let rows_end = (body_y + v.scroll.row_top(g.rows.end) as f32).min(bottom);
        window.paint_quad(fill(
            snap.vline(body_x + g.frozen_w, top, rows_end),
            palette.seam_strong,
        ));
    }

    // ## m3c-t36: the unsorted tail's boundary, a 1 px dashed `text-3` line above its first
    // row (NOTES surface 18), across the columns.
    if let Some(row) = v.tail_boundary()
        && row >= g.rows.start
        && row <= g.rows.end
    {
        let y = body_y + v.scroll.row_top(row) as f32;
        if y >= top && y <= bottom {
            let x_end = (body_x + v.layout.total_width() as f32 - v.scroll.left_x as f32)
                .max(body_x + g.frozen_w)
                .min(body_right);
            dashed_hline(snap, body_x, x_end, y, palette.text3, window);
            #[cfg(feature = "test-support")]
            {
                v.paint.recorded_tail_y = Some(y);
            }
        }
    }

    // Layer 10: the resize guide (dashed, ## m3c-t35).
    if let Some(ix) = v.pointer.resize_guide(&v.layout) {
        dashed_vline(snap, v.col_right(g, ix), top, bottom, palette.text2, window);
    }
}

/// One segment of the body: selection and hover fills, content, hairlines, outlines.
#[allow(clippy::too_many_arguments)]
fn paint_body_segment(
    v: &mut GridView,
    f: &FramePrepaint,
    seg: &Segment,
    map: &ViewMap,
    palette: &Palette,
    snap: Snap,
    recording: bool,
    budget: &mut Budget,
    stats: &mut FrameStats,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let g = &f.geom;
    let (body_x, body_y) = (f32::from(g.body.left()), f32::from(g.body.top()));
    let row_h = g.row_h;
    let shift = seg.shift;
    let col_left = |layout: &ColumnLayout, ix: usize| body_x + (layout.x_of(ix) - shift) as f32;
    let row_top = |scroll: &ScrollState, r: u64| body_y + scroll.row_top(r) as f32;
    let loaded = v.data.count.loaded;
    // The rows the view shows, and the loaded model row view row `r` shows (`None` past them:
    // a placeholder).
    let shown = map.len();
    let model_of = |r: u64| {
        if r < shown {
            map.view_to_model(ViewRow(r)).filter(|m| m.0 < loaded)
        } else {
            None
        }
    };
    let focused = v.selection.focus();

    // Layer 6: selection fills (the focus cell stronger) and the hovered row, merged per row.
    let hover_row = v.pointer.hover.map(|(r, _)| r);
    if !v.selection.is_empty() || hover_row.is_some() {
        for r in g.rows.clone() {
            let y = row_top(&v.scroll, r);
            let model = model_of(r);
            let mut run: Option<(f32, f32, Hsla)> = None;
            for ix in seg.cols.clone() {
                let col = v.layout.visible()[ix];
                let selected = r < shown && v.selection.contains(ViewRow(r), col, map);
                let color = if selected {
                    let is_focus = focused.is_some_and(|c| Some(c.row) == model && c.col == col);
                    Some(if is_focus {
                        palette.blue_tint_strong
                    } else {
                        palette.blue_tint
                    })
                } else if hover_row == Some(r) && r < shown {
                    Some(palette.hover)
                } else {
                    None
                };
                let (x, x1) = (col_left(&v.layout, ix), col_left(&v.layout, ix + 1));
                match (run, color) {
                    (Some((a, _, c)), Some(n)) if c == n => run = Some((a, x1, c)),
                    (prev, next) => {
                        if let Some((a, b, c)) = prev {
                            window.paint_quad(fill(rect(a, y, b - a, row_h), c));
                        }
                        run = next.map(|n| (x, x1, n));
                    }
                }
            }
            if let Some((a, b, c)) = run {
                window.paint_quad(fill(rect(a, y, b - a, row_h), c));
            }
        }
    }

    // Layer 7: content, row by row (a fast-scroll frame spends its shaping budget on whole rows
    // from the top), each cell clipped to its column.
    let GridView {
        paint,
        layout,
        scroll,
        columns,
        // ## m3c-t37
        highlights,
        ..
    } = &mut *v;
    let mut col_info = std::mem::take(&mut paint.col_scratch);
    col_info.clear();
    for ix in seg.cols.clone() {
        let col = layout.visible()[ix];
        let kind = columns
            .get(col.0 as usize)
            .filter(|s| s.id == col)
            .or_else(|| columns.iter().find(|s| s.id == col))
            .map_or(ColumnKind::Other, |s| s.kind);
        let left = body_x + (layout.x_of(ix) - shift) as f32;
        col_info.push(super::ColPaint {
            ix,
            col,
            kind,
            left,
            width: layout.width_at(ix),
        });
    }
    let roles = paint.roles.as_ref().expect("roles are built in prepaint");
    let cell_paint = CellPaint {
        roles,
        palette,
        fast: f.fast,
        recording,
        row_h,
    };
    let body_h = f32::from(g.body.size.height);
    for r in g.rows.clone() {
        let y = body_y + scroll.row_top(r) as f32;
        let model = model_of(r);
        for c in &col_info {
            let (col, ix, left, width) = (c.col, c.ix, c.left, c.width);
            let mask = rect(left, body_y, (width - 1.0).max(0.0), body_h);
            stats.cells += 1;
            let cell = model.map(|m| f.frame.cell(m, col));
            let kind = cell.as_ref().map_or(CellKind::Placeholder, |c| c.kind);
            let Some(cell) = cell.filter(|_| kind != CellKind::Placeholder) else {
                masked(mask, window, |window| {
                    paint_skeleton(left, width, y, row_h, r, ix, palette, window)
                });
                #[cfg(feature = "test-support")]
                if let Some(cells) = paint.recorded.as_mut() {
                    cells.push(super::PaintedCell::skeleton(r, col, left));
                }
                continue;
            };
            let text = cell_text(kind, &cell.text);
            let source = if kind == CellKind::Error {
                TextSource::Fluent
            } else {
                TextSource::Data
            };
            let size_px = roles.for_cell(kind, c.kind).nominal;
            let at = CellBox {
                left,
                width,
                y,
                row: r,
                col: ix,
            };
            let mut records = 0u32;
            // ## m3c-t37: the find's marks in this cell (data cells only: never a NULL or an
            // error's text).
            let marks = match (highlights.as_ref(), model, kind) {
                (Some(h), Some(m), k) if !matches!(k, CellKind::Null | CellKind::Error) => {
                    h(m, col)
                }
                _ => None,
            };
            #[cfg(feature = "test-support")]
            let marks_sink = &mut paint.recorded_find_marks;
            let outcome = masked(mask, window, |window| {
                paint_cell_content(
                    &cell_paint,
                    &mut paint.cache,
                    budget,
                    &mut paint.scratch,
                    &text,
                    kind,
                    c.kind,
                    cell.align,
                    cell.chip.as_ref(),
                    at,
                    marks.as_ref(),
                    |x0, x1, current| {
                        #[cfg(feature = "test-support")]
                        marks_sink.push(super::PaintedFindMark {
                            row: r,
                            col,
                            x0,
                            x1,
                            current,
                        });
                        #[cfg(not(feature = "test-support"))]
                        let _ = (x0, x1, current);
                    },
                    window,
                    cx,
                    |s, cx| {
                        #[cfg(feature = "test-support")]
                        if let Some(cells) = paint.recorded.as_mut() {
                            records += 1;
                            cells.push(match s {
                                Some(s) => super::PaintedCell::of(r, col, left, s),
                                None => super::PaintedCell::skeleton(r, col, left),
                            });
                        }
                        if recording && let Some(s) = s {
                            records += 1;
                            layers::record_text(
                                cx,
                                s.display.clone(),
                                source,
                                size_px,
                                s.width,
                                s.truncated,
                                // ## m3a-t35-shots: wider than the room inside the
                                // cell's padding (never true of a run that fits).
                                s.width > width - 2.0 * CELL_PAD_PX + 0.5,
                                layers::paths::CELL,
                            );
                        }
                    },
                )
            });
            budget.allocs += records;
            stats.degraded += u32::from(outcome.degraded);
            stats.skeletons += u32::from(outcome.skeleton);
            if recording
                && !outcome.skeleton
                && let Some(trail) = cell.chip.as_ref().filter(|_| !f.fast)
            {
                budget.allocs += 1;
                layers::record_text(
                    cx,
                    trail.clone(),
                    TextSource::Fluent,
                    roles.chip.nominal,
                    0.0,
                    false,
                    false,
                    layers::paths::CELL,
                );
            }
        }
    }
    paint.col_scratch = col_info;

    // Layer 8: hairlines under each row and at each column's right edge.
    let x_end = col_left(&v.layout, seg.cols.end).min(seg.x1);
    for r in g.rows.clone() {
        let bottom = row_top(&v.scroll, r) + row_h;
        window.paint_quad(fill(snap.hline(seg.x0, x_end, bottom), palette.seam_soft));
    }
    let (top, bottom) = (body_y, f32::from(g.body.bottom()));
    let rows_end = row_top(&v.scroll, g.rows.end).min(bottom);
    for ix in seg.cols.clone() {
        let right = col_left(&v.layout, ix + 1);
        window.paint_quad(fill(snap.vline(right, top, rows_end), palette.seam_soft));
    }

    // Layer 9: the hover outline and the selection boxes with the focus handle.
    if let Some((r, ix)) = v.pointer.hover
        && r < shown
        && g.rows.contains(&r)
        && seg.cols.contains(&ix)
    {
        let (x, x1) = (col_left(&v.layout, ix), col_left(&v.layout, ix + 1));
        let y = row_top(&v.scroll, r);
        outline(
            rect(snap.v(x), snap.v(y), snap.v(x1) - snap.v(x), snap.v(row_h)),
            palette.seam_strong,
            window,
        );
    }
    paint_selection_boxes(v, g, seg, map, palette, snap, window);
}

/// A dashed vertical 1 px line at `right` (exclusive) from `y0` to `y1`: 3 px dashes, 3 px
/// gaps.
fn dashed_vline(snap: Snap, right: f32, y0: f32, y1: f32, color: Hsla, window: &mut Window) {
    const DASH: f32 = 3.0;
    let mut y = y0;
    while y < y1 {
        window.paint_quad(fill(snap.vline(right, y, (y + DASH).min(y1)), color));
        y += 2.0 * DASH;
    }
}

// ## m3c-t36
/// A dashed horizontal 1 px line from `x0` to `x1` whose bottom is `bottom` (exclusive): 3 px
/// dashes, 3 px gaps.
fn dashed_hline(snap: Snap, x0: f32, x1: f32, bottom: f32, color: Hsla, window: &mut Window) {
    const DASH: f32 = 3.0;
    let mut x = x0;
    while x < x1 {
        window.paint_quad(fill(snap.hline(x, (x + DASH).min(x1), bottom), color));
        x += 2.0 * DASH;
    }
}

/// The rows of a selection box `rows` to draw: within the `visible` rows ± 1, so every drawn
/// edge is near the view (precise in `f32`) and a clamped edge falls outside the body.
fn box_rows(rows: std::ops::Range<u64>, visible: &std::ops::Range<u64>) -> std::ops::Range<u64> {
    rows.start.max(visible.start.saturating_sub(1))..rows.end.min(visible.end.saturating_add(1))
}

/// Calls `f(r0, r1, c0, c1)` for each box of the selection's pieces, in view rows and visible
/// column indices: select-all, the row pieces, the column pieces, the rectangles. Under a sort
/// or a filter (## m3c-t35) a row piece (model rows) shows as the runs of view rows near the
/// view that hold its rows.
fn for_each_box(
    v: &GridView,
    g: &Geometry,
    map: &ViewMap,
    mut f: impl FnMut(u64, u64, usize, usize),
) {
    let shown = map.len();
    let order = v.layout.visible();
    // Column positions (ordinals) to visible indices: [first visible ≥ start, last < end].
    let col_span = |cols: &std::ops::Range<u32>| -> Option<(usize, usize)> {
        let a = order.iter().position(|c| c.0 >= cols.start)?;
        let b = order.iter().rposition(|c| c.0 < cols.end)?;
        (a <= b).then_some((a, b + 1))
    };
    let visible_rows = g.rows.start..g.rows.end.min(shown);
    let all = order.len();
    if v.selection.is_all() {
        f(visible_rows.start, visible_rows.end, 0, all);
        return;
    }
    if map.is_identity() {
        for rows in v.selection.rows() {
            f(rows.start, rows.end.min(shown), 0, all);
        }
    } else if !v.selection.rows().is_empty() {
        let near = g.rows.start.saturating_sub(1)..g.rows.end.saturating_add(1).min(shown);
        let mut run: Option<u64> = None;
        for r in near.clone() {
            let held = map
                .view_to_model(ViewRow(r))
                .is_some_and(|m| v.selection.row_selected(m));
            match (run, held) {
                (None, true) => run = Some(r),
                (Some(start), false) => {
                    f(start, r, 0, all);
                    run = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run {
            f(start, near.end, 0, all);
        }
    }
    for cols in v.selection.cols() {
        if let Some((a, b)) = col_span(cols) {
            f(visible_rows.start, visible_rows.end, a, b);
        }
    }
    for r in v.selection.rects() {
        if let Some((a, b)) = col_span(&r.cols) {
            f(r.rows.start, r.rows.end.min(shown), a, b);
        }
    }
}

/// The 1 px `blue` boxes of the selection's pieces in segment `seg`, and the 5 px handle (in a
/// 1 px `panel` ring) at the bottom right of the box holding the focus (else the first box on
/// screen), drawn by the segment holding that box's last column.
fn paint_selection_boxes(
    v: &GridView,
    g: &Geometry,
    seg: &Segment,
    map: &ViewMap,
    palette: &Palette,
    snap: Snap,
    window: &mut Window,
) {
    if v.selection.is_empty() || map.is_empty() || g.rows.is_empty() {
        return;
    }
    if seg.cols.is_empty() && !seg.frozen {
        return;
    }
    let (body_x, body_y) = (f32::from(g.body.left()), f32::from(g.body.top()));
    let on_rows = |r0: u64, r1: u64| r0 < r1 && r1 > g.rows.start && r0 < g.rows.end;
    let on_screen = |c0: usize, c1: usize| {
        (g.frozen > 0 && c0 < g.frozen) || (c1 > g.cols.start && c0 < g.cols.end)
    };
    let focus = v
        .selection
        .focus()
        .map(|f| (v.view_row_of(f.row), v.layout.index_of(f.col)));
    let mut handle: Option<(u64, u64, usize, usize)> = None;
    for_each_box(v, g, map, |r0, r1, c0, c1| {
        if !on_rows(r0, r1) || !on_screen(c0, c1) {
            return;
        }
        let holds_focus = focus.is_some_and(|(row, ix)| {
            (r0..r1).contains(&row) && ix.is_some_and(|ix| (c0..c1).contains(&ix))
        });
        if holds_focus || handle.is_none() {
            handle = Some((r0, r1, c0, c1));
        }
    });
    let corners = |r0: u64, r1: u64, c0: usize, c1: usize| {
        let x = snap.v(body_x + (v.layout.x_of(c0) - seg.shift) as f32);
        let x1 = snap.v(body_x + (v.layout.x_of(c1) - seg.shift) as f32);
        // Rows far off screen would put an edge ~10⁸ px away, where an `f32` step is several
        // px and the visible edge drifts: draw the box's rows within the visible ones ± 1 (the
        // clamped edges stay outside the body's mask).
        let drawn = box_rows(r0..r1, &g.rows);
        let y = snap.v(body_y + v.scroll.row_top(drawn.start) as f32);
        let y1 = snap.v(body_y + v.scroll.row_top(drawn.end) as f32);
        (x, y, x1, y1)
    };
    for_each_box(v, g, map, |r0, r1, c0, c1| {
        if !on_rows(r0, r1) || c1 <= seg.cols.start || c0 >= seg.cols.end {
            return;
        }
        let (x, y, x1, y1) = corners(r0, r1, c0, c1);
        outline(rect(x, y, x1 - x, y1 - y), palette.blue, window);
    });
    if let Some((r0, r1, c0, c1)) = handle
        && c1 > 0
        && (c1 - 1 < g.frozen) == seg.frozen
    {
        let (_, _, x1, y1) = corners(r0, r1, c0, c1);
        // The 5 px square in its 1 px panel ring, centred a px inside the box's corner.
        layers::bordered(
            rect(x1 - 4.0, y1 - 4.0, 7.0, 7.0),
            palette.blue,
            palette.panel,
            0.0,
            window,
        );
    }
}

/// The header region: background, each segment's headers (the selected columns' tint, names
/// over mono types, the column seams), the bottom border, the frozen edge, the hovered edge's
/// handle and the resize guide.
#[allow(clippy::too_many_arguments)]
fn paint_header(
    v: &mut GridView,
    f: &FramePrepaint,
    palette: &Palette,
    snap: Snap,
    recording: bool,
    budget: &mut Budget,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let g = &f.geom;
    let header = g.header;
    window.paint_quad(fill(header, palette.head));
    let (hx, hy) = (f32::from(header.left()), f32::from(header.top()));
    let hh = f32::from(header.size.height);
    for seg in segments(v, g).into_iter().flatten() {
        let clip = rect(seg.x0, hy, seg.x1 - seg.x0, hh);
        masked(clip, window, |window| {
            paint_header_segment(
                v, f, &seg, clip, palette, snap, recording, budget, window, cx,
            )
        });
    }
    // The bottom border.
    window.paint_quad(fill(
        snap.hline(hx, f32::from(header.right()), hy + hh),
        palette.seam_strong,
    ));
    // ## m3c-t35: the frozen edge, the hovered edge's handle, the dashed guide.
    if g.frozen > 0 {
        window.paint_quad(fill(
            snap.vline(hx + g.frozen_w, hy, hy + hh),
            palette.seam_strong,
        ));
    }
    if v.pointer.drag.is_none()
        && let Some(ix) = v.pointer.border
    {
        window.paint_quad(fill(
            snap.vline(v.col_right(g, ix), hy, hy + hh),
            palette.text2,
        ));
    }
    if let Some(ix) = v.pointer.resize_guide(&v.layout) {
        dashed_vline(snap, v.col_right(g, ix), hy, hy + hh, palette.text2, window);
    }
}

/// One segment of the header.
#[allow(clippy::too_many_arguments)]
fn paint_header_segment(
    v: &mut GridView,
    f: &FramePrepaint,
    seg: &Segment,
    clip: Bounds<Pixels>,
    palette: &Palette,
    snap: Snap,
    recording: bool,
    budget: &mut Budget,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let g = &f.geom;
    let header = g.header;
    let (hx, hy) = (f32::from(header.left()), f32::from(header.top()));
    let hh = f32::from(header.size.height);
    let shift = seg.shift;
    let GridView {
        paint,
        layout,
        columns,
        selection,
        sort,
        ..
    } = &mut *v;
    let roles = paint.roles.as_ref().expect("roles are built in prepaint");
    let name_lh = 15.0;
    let type_lh = 13.0;
    let name_y = hy + ((hh - 1.0 - name_lh - 1.0 - type_lh) / 2.0).max(0.0);
    let type_y = name_y + name_lh + 1.0;
    for ix in seg.cols.clone() {
        let col = layout.visible()[ix];
        let left = hx + (layout.x_of(ix) - shift) as f32;
        let width = layout.width_at(ix);
        // A column selection tints its headers; select-all tints the corner instead (the mock's
        // `colsel` and `allon`).
        if !selection.is_all() && selection.col_selected(col) {
            window.paint_quad(fill(rect(left, hy, width, hh), palette.blue_tint));
        }
        let Some(spec) = columns
            .get(col.0 as usize)
            .filter(|s| s.id == col)
            .or_else(|| columns.iter().find(|s| s.id == col))
        else {
            continue;
        };
        let avail = (width - 2.0 * CELL_PAD_PX).max(0.0);
        let x0 = left + CELL_PAD_PX;
        let end_aligned = spec.align == Align::End;
        let mask = rect(left, hy, (width - 1.0).max(0.0), hh);
        // ## m3c-t36: a sorted column keeps room after its name for the arrow (and the level
        // number with two or more keys): the name gives way first.
        let sorted = sort.spec.level_of(col);
        let number = sorted.and_then(|(level, _)| sort.levels.get(level).cloned());
        let number_w = number.as_ref().map_or(0.0, |n| {
            with_shape(
                &mut paint.cache,
                n,
                &roles.sort_level,
                &roles.chip,
                f32::MAX,
                false,
                budget,
                &mut paint.scratch,
                window,
                cx,
                |s, _, _| s.width,
            )
            .unwrap_or(0.0)
        });
        let mark_w = SORT_ARROW_PX
            + if number.is_some() {
                SORT_NUMBER_GAP_PX + number_w
            } else {
                0.0
            };
        let name_room = if sorted.is_some() {
            (avail - SORT_GAP_PX - mark_w).max(0.0)
        } else {
            avail
        };
        masked(mask, window, |window| {
            let mut name_end = x0;
            for (text, role, y, lh, path, is_name) in [
                (
                    &spec.name,
                    &roles.header_name,
                    name_y,
                    name_lh,
                    layers::paths::HEADER,
                    true,
                ),
                (
                    &spec.type_label,
                    &roles.header_type,
                    type_y,
                    type_lh,
                    layers::paths::HEADER,
                    false,
                ),
            ] {
                if text.is_empty() {
                    continue;
                }
                // Never fast-scroll: a header always paints its full-quality shape (review
                // round 1), on its own budget.
                // ## m3c-t36: the name line ends where the sort indicator begins.
                let line = if is_name { name_room } else { avail };
                let recorded = with_shape(
                    &mut paint.cache,
                    text,
                    role,
                    &roles.chip,
                    line,
                    false,
                    budget,
                    &mut paint.scratch,
                    window,
                    cx,
                    |s, window, cx| {
                        let x = if end_aligned { x0 + line - s.width } else { x0 };
                        if is_name {
                            name_end = x + s.width;
                        }
                        layers::paint_shaped(s, x, y, lh, palette, true, window, cx);
                        if recording {
                            layers::record_text(
                                cx,
                                text.clone(),
                                TextSource::Catalog,
                                role.nominal,
                                s.width,
                                s.truncated,
                                // ## m3a-t35-shots: wider than the header's room.
                                s.width > line + 0.5,
                                path,
                            );
                        }
                        (recording, s.truncated, s.degraded)
                    },
                );
                if let Some((recorded, truncated, _degraded)) = recorded {
                    budget.allocs += u32::from(recorded);
                    if is_name && (truncated || seg.frozen || sorted.is_some()) {
                        // An ellipsized name: the whole name is its tooltip (NOTES rule 7);
                        // ## m3c-t35: a frozen column's header says it is frozen.
                        // ## m3c-t36: a sorted one names its level and collation.
                        paint.header_tips.push(super::HeaderTip {
                            col,
                            name: text.clone(),
                            bounds: mask.intersect(&header).intersect(&clip),
                            ellipsized: truncated,
                            frozen: seg.frozen,
                            sort: sorted.map(|(level, _)| level),
                        });
                    }
                    #[cfg(feature = "test-support")]
                    if let Some(headers) = paint.recorded_headers.as_mut() {
                        budget.allocs += 1;
                        headers.push(super::PaintedHeader {
                            col,
                            text: text.clone(),
                            truncated,
                            degraded: _degraded,
                        });
                    }
                }
            }
            // ## m3c-t36: the sort indicator after the name (at the end of an end-aligned
            // header): a 12 px blue arrow, up ascending, down descending, and the level number
            // in Fragment Mono 11 blue with two or more keys (NOTES surface 18, M3c-19).
            if let Some((level, key)) = sorted {
                let room_end = x0 + avail - mark_w;
                let mark_x = if end_aligned {
                    room_end
                } else {
                    (name_end + SORT_GAP_PX).min(room_end)
                };
                let arrow = if key.desc {
                    IconName::ArrowDown
                } else {
                    IconName::ArrowUp
                };
                let _ = window.paint_svg(
                    rect(
                        mark_x,
                        name_y + (name_lh - SORT_ARROW_PX) / 2.0,
                        SORT_ARROW_PX,
                        SORT_ARROW_PX,
                    ),
                    arrow.path().into(),
                    None,
                    Default::default(),
                    palette.blue,
                    cx,
                );
                if let Some(n) = number.as_ref() {
                    let at = mark_x + SORT_ARROW_PX + SORT_NUMBER_GAP_PX;
                    let _ = with_shape(
                        &mut paint.cache,
                        n,
                        &roles.sort_level,
                        &roles.chip,
                        f32::MAX,
                        false,
                        budget,
                        &mut paint.scratch,
                        window,
                        cx,
                        |s, window, cx| {
                            layers::paint_shaped(s, at, name_y, name_lh, palette, false, window, cx)
                        },
                    );
                }
                #[cfg(feature = "test-support")]
                paint.recorded_sort_marks.push(super::PaintedSortMark {
                    col,
                    level,
                    desc: key.desc,
                    x: mark_x,
                    number: number.clone(),
                });
                #[cfg(not(feature = "test-support"))]
                let _ = level;
            }
        });
        // The seam between columns.
        window.paint_quad(fill(snap.vline(left + width, hy, hy + hh), palette.seam));
    }
}

/// The row-number gutter: background, row numbers (raw digits; `blue` for selected rows,
/// `text-2` on the hovered row), the row hairlines and the `seam` edge. ## m3c-t35: the numbers
/// are view rows (a sorted grid numbers its rows 1…n as shown); a row is selected by its model
/// row.
fn paint_gutter(
    v: &mut GridView,
    f: &FramePrepaint,
    palette: &Palette,
    snap: Snap,
    recording: bool,
    window: &mut Window,
    cx: &mut Context<GridView>,
) {
    let g = &f.geom;
    let gutter = g.gutter;
    window.paint_quad(fill(gutter, palette.gutter));
    let (gx, gy) = (f32::from(gutter.left()), f32::from(gutter.top()));
    let right = f32::from(gutter.right());
    let row_h = g.row_h;
    let map = v.view_map().clone();
    let numbered = match map {
        ViewMap::Filtered { .. } => map.len(),
        _ => v.data.known_total().unwrap_or(v.data.count.loaded),
    };
    let hover_row = v.pointer.hover.map(|(r, _)| r);
    let selection = &v.selection;
    let roles = v.paint.roles.as_ref().expect("roles are built in prepaint");
    let all = selection.is_all();
    for r in g.rows.clone() {
        let y = gy + v.scroll.row_top(r) as f32;
        if r < numbered {
            let selected = all
                || map
                    .view_to_model(ViewRow(r))
                    .is_some_and(|m| selection.row_selected(m))
                || selection.rects().iter().any(|rc| rc.rows.contains(&r));
            let color = if selected {
                palette.blue
            } else if hover_row == Some(r) {
                palette.text2
            } else {
                palette.text3
            };
            // Right-aligned, 9 px from the 1 px edge (the mock's padding).
            paint_row_number(roles, r + 1, right - 10.0, y, row_h, color, window, cx);
            if recording {
                layers::record_text(
                    cx,
                    SharedString::from((r + 1).to_string()),
                    TextSource::Data,
                    roles.row_number.nominal,
                    layers::digit_count(r + 1) as f32 * roles.digit_advance,
                    false,
                    false,
                    layers::paths::ROW_NUMBER,
                );
            }
        }
        window.paint_quad(fill(snap.hline(gx, right, y + row_h), palette.seam_soft));
    }
    window.paint_quad(fill(
        snap.vline(right, gy, f32::from(gutter.bottom())),
        palette.seam,
    ));
}

/// The corner: `head`, `blue-tint-strong` while everything is selected, the gutter's edge and
/// the header's bottom border. A click selects everything.
fn paint_corner(
    v: &GridView,
    f: &FramePrepaint,
    palette: &Palette,
    snap: Snap,
    window: &mut Window,
) {
    let c = f.geom.corner;
    window.paint_quad(fill(c, palette.head));
    if v.selection.is_all() {
        window.paint_quad(fill(c, palette.blue_tint_strong));
    }
    let (top, bottom) = (f32::from(c.top()), f32::from(c.bottom()));
    window.paint_quad(fill(
        snap.vline(f32::from(c.right()), top, bottom),
        palette.seam,
    ));
    window.paint_quad(fill(
        snap.hline(f32::from(c.left()), f32::from(c.right()), bottom),
        palette.seam_strong,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::scroll_rows;
    use crate::source::RowCount;
    use crate::view::SKELETON_ROWS;

    #[test]
    fn scroll_rows_follow_the_total_then_the_loaded_rows() {
        let c = |loaded, more, exact_total| RowCount {
            loaded,
            more,
            exact_total,
        };
        assert_eq!(scroll_rows(&c(1_000, true, Some(10_000))), 10_000);
        assert_eq!(scroll_rows(&c(1_000, true, None)), 1_000);
        assert_eq!(scroll_rows(&c(0, true, None)), SKELETON_ROWS);
        assert_eq!(scroll_rows(&c(0, false, Some(0))), 0);
        assert_eq!(scroll_rows(&c(12, false, None)), 12);
    }

    // ## m3a-t30-results-view: the node moved to the view's root (`GridView::a11y_node`,
    // tested in `tests/view.rs`); the inner element must not declare a second one.
    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn the_element_declares_no_node_of_its_own(cx: &mut gpui::TestAppContext) {
        use crate::testing::{FixtureSource, grid_config};
        use hacivat_ui_core::theme::Theme;
        cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
        let source = FixtureSource::texts(&[&["a", "b", "c"], &["d", "e", "f"]], 100.0);
        let (view, vcx) = cx.add_window_view(|_, cx| {
            GridView::new(std::rc::Rc::new(source), grid_config("Result 1"), cx)
        });
        vcx.run_until_parked();
        let element = GridElement::new(view.clone());
        assert_eq!(element.a11y_role(), None);
        assert_eq!(view.read_with(vcx, |v, _| v.a11y_label_reads()), 0);
    }

    #[test]
    fn selection_boxes_draw_only_rows_near_the_view() {
        let visible = 5_000_000..5_000_030;
        assert_eq!(box_rows(0..10_000_000, &visible), 4_999_999..5_000_031);
        assert_eq!(
            box_rows(5_000_002..5_000_004, &visible),
            5_000_002..5_000_004
        );
        assert_eq!(box_rows(0..5_000_010, &visible), 4_999_999..5_000_010);
        assert_eq!(box_rows(0..3, &(0..30)), 0..3);
    }

    #[test]
    fn the_gutter_is_46_px_up_to_five_digits_then_one_advance_per_digit() {
        let snap = Snap(1.0);
        assert_eq!(gutter_width(1, 6.0, snap), 46.0);
        assert_eq!(gutter_width(5, 6.0, snap), 46.0);
        assert_eq!(gutter_width(8, 6.0, snap), 64.0);
        assert_eq!(gutter_width(8, 6.1, snap), 65.0);
    }
}
