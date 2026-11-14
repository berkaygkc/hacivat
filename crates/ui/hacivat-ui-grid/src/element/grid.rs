//! The grid element (HV-GRID-01, -02, -03; Task 20).
//!
//! **Frame lifecycle (HV-GRID-01).**
//! 1. `request_layout`: the grid takes its parent's size; the accessibility declaration is built
//!    only while assistive technology is active (R-M3-32: role `Grid`, the label, the row and
//!    column counts; no synthetic children in M3a).
//! 2. `prepaint` ([`prepaint_frame`]): one `begin_frame` (one snapshot load); generation and
//!    row-count changes; the geometry (header 36 px, the row-number gutter ≥ 46 px, 9 px
//!    tracks); the frame's coalesced scroll deltas and the velocity (fast-scroll); the visible
//!    rows × columns; at most one `ensure_rows`; the hitbox; the frame's events.
//! 3. `paint` ([`paint_frame`]): the regions body → header → gutter → corner → scrollbars, each
//!    in its content mask, the layers of [`super::layers`] inside each; the pointer listeners.

use std::panic::Location;
use std::rc::Rc;

use hacivat_ui::text::{TextRegistry, TextSource};
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, Element, ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, LayoutId, Pixels, Role, SharedString, Style, Window, accesskit, fill,
    relative,
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
use crate::model::ViewMap;
use crate::source::{Align, CellKind, ColumnKind, GridFrame, ModelRow, ViewRow};
use crate::view::{GridEvent, GridView, SKELETON_ROWS};

/// The grid's accessibility declaration, built only while assistive technology is active.
#[derive(Debug, Clone)]
struct A11y {
    role: Role,
    label: SharedString,
    rows: usize,
    cols: usize,
}

/// The grid's element.
pub(crate) struct GridElement {
    view: Entity<GridView>,
    id: ElementId,
    a11y: Option<A11y>,
}

impl GridElement {
    pub(crate) fn new(view: Entity<GridView>) -> Self {
        let id = ElementId::from(("hv-grid", view.entity_id()));
        GridElement {
            view,
            id,
            a11y: None,
        }
    }

    /// Builds the frame's accessibility declaration when assistive technology is `active`
    /// (`request_layout` passes `window.is_a11y_active()`), and drops it otherwise: no label is
    /// read without assistive technology. GPUI reads it through `a11y_role` and
    /// `write_a11y_info`.
    fn refresh_a11y(&mut self, active: bool, cx: &mut App) {
        self.a11y = active.then(|| {
            self.view.update(cx, |v, cx| {
                let (role, label) = v.a11y_declaration(cx);
                let rows = v.data.known_total().unwrap_or(v.data.count.loaded);
                A11y {
                    role,
                    label,
                    rows: usize::try_from(rows).unwrap_or(usize::MAX),
                    cols: v.layout.len(),
                }
            })
        });
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
        self.refresh_a11y(window.is_a11y_active(), cx);
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

    fn a11y_role(&self) -> Option<Role> {
        self.a11y.as_ref().map(|a| a.role)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        if let Some(a) = &self.a11y {
            node.set_label(a.label.to_string());
            node.set_row_count(a.rows);
            node.set_column_count(a.cols);
        }
    }
}

/// The rows the grid scrolls over for `count`: the exact total when known, the skeleton rows
/// while the first rows are arriving, the loaded rows otherwise.
fn scroll_rows(count: &crate::source::RowCount) -> u64 {
    match count.exact_total {
        Some(total) => total.max(count.loaded),
        None if count.loaded == 0 && count.more => SKELETON_ROWS,
        None => count.loaded,
    }
}

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
        // An append keeps the selection and places a pending focus.
        let map = ViewMap::identity(count.loaded);
        if v.selection.on_rows_loaded(&map, v.layout.visible()) {
            v.selection_dirty = true;
            cx.emit(GridEvent::FocusChanged(v.selection.focus()));
        }
    }
    v.data.count = count;
    v.data.rows = scroll_rows(&count);
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
    let autosize_measures = super::autosize::step(v, &*frame, body_w, window, cx);
    if v.autosize.pending() && autosize_measures > 0 {
        let view = cx.entity();
        window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()));
    }
    let content_w = v.layout.total_width();
    let htrack_h = if content_w > f64::from(body_w) {
        SCROLLBAR_PX
    } else {
        0.0
    };
    let body_h = (h - header_h - htrack_h).max(0.0);
    let body_x = x0 + gutter_w;
    let body_y = y0 + header_h;

    v.scroll.set_viewport(body_w, body_h);
    v.scroll.on_row_count(v.data.rows);
    v.scroll.set_content_width(content_w);
    v.scroll.apply_pending();
    let velocity = v.scroll.velocity_rows_per_frame();
    let fast = velocity > FAST_SCROLL_ROWS_PER_FRAME;

    let rows = v.scroll.visible_rows();
    let cols = v.layout.visible_range(v.scroll.left_x, body_w);
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

    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
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

/// The body region: background, selection and hover fills, content, hairlines, outlines and
/// the resize guide.
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
    let row_h = g.row_h;
    let left_x = v.scroll.left_x;
    let col_left =
        |layout: &crate::model::ColumnLayout, ix: usize| body_x + (layout.x_of(ix) - left_x) as f32;
    let row_top = |scroll: &crate::model::ScrollState, r: u64| body_y + scroll.row_top(r) as f32;
    let loaded = v.data.count.loaded;
    let map = ViewMap::identity(loaded);
    let focused = v.selection.focus();

    // Layer 2: zebra (a setting, off in M3a).
    if v.config.zebra {
        for r in g.rows.clone().filter(|r| r % 2 == 1) {
            let y = row_top(&v.scroll, r);
            window.paint_quad(fill(
                rect(body_x, y, body_right - body_x, row_h),
                palette.hover,
            ));
        }
    }

    // Layer 6: selection fills (the focus cell stronger) and the hovered row, merged per row.
    let hover_row = v.pointer.hover.map(|(r, _)| r);
    if !v.selection.is_empty() || hover_row.is_some() {
        for r in g.rows.clone() {
            let y = row_top(&v.scroll, r);
            let mut run: Option<(f32, f32, Hsla)> = None;
            for ix in g.cols.clone() {
                let col = v.layout.visible()[ix];
                let selected = r < loaded && v.selection.contains(ViewRow(r), col, &map);
                let color = if selected {
                    let is_focus = focused.is_some_and(|c| c.row == ModelRow(r) && c.col == col);
                    Some(if is_focus {
                        palette.blue_tint_strong
                    } else {
                        palette.blue_tint
                    })
                } else if hover_row == Some(r) && r < loaded {
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
        ..
    } = &mut *v;
    let mut col_info = std::mem::take(&mut paint.col_scratch);
    col_info.clear();
    for ix in g.cols.clone() {
        let col = layout.visible()[ix];
        let kind = columns
            .get(col.0 as usize)
            .filter(|s| s.id == col)
            .or_else(|| columns.iter().find(|s| s.id == col))
            .map_or(ColumnKind::Other, |s| s.kind);
        let left = body_x + (layout.x_of(ix) - left_x) as f32;
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
        for c in &col_info {
            let (col, ix, left, width) = (c.col, c.ix, c.left, c.width);
            let mask = rect(left, body_y, (width - 1.0).max(0.0), body_h);
            stats.cells += 1;
            let cell = (r < loaded).then(|| f.frame.cell(ModelRow(r), col));
            let kind = cell.as_ref().map_or(CellKind::Placeholder, |c| c.kind);
            let Some(cell) = cell.filter(|_| kind != CellKind::Placeholder) else {
                masked(mask, window, |window| {
                    paint_skeleton(left, width, y, row_h, r, ix, palette, window)
                });
                #[cfg(feature = "test-support")]
                if let Some(cells) = paint.recorded.as_mut() {
                    cells.push(super::PaintedCell::skeleton(r, col));
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
                    window,
                    cx,
                    |s, cx| {
                        #[cfg(feature = "test-support")]
                        if let Some(cells) = paint.recorded.as_mut() {
                            records += 1;
                            cells.push(match s {
                                Some(s) => super::PaintedCell::of(r, col, s),
                                None => super::PaintedCell::skeleton(r, col),
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
                    layers::paths::CELL,
                );
            }
        }
    }
    paint.col_scratch = col_info;

    // Layer 8: hairlines under each row and at each column's right edge.
    let x_end = col_left(&v.layout, g.cols.end).min(body_right);
    for r in g.rows.clone() {
        let bottom = row_top(&v.scroll, r) + row_h;
        window.paint_quad(fill(snap.hline(body_x, x_end, bottom), palette.seam_soft));
    }
    let (top, bottom) = (body_y, f32::from(g.body.bottom()));
    let rows_end = row_top(&v.scroll, g.rows.end).min(bottom);
    for ix in g.cols.clone() {
        let right = col_left(&v.layout, ix + 1);
        window.paint_quad(fill(snap.vline(right, top, rows_end), palette.seam_soft));
    }

    // Layer 9: the hover outline and the selection boxes with the focus handle.
    if let Some((r, ix)) = v.pointer.hover
        && r < loaded
        && g.rows.contains(&r)
        && g.cols.contains(&ix)
    {
        let (x, x1) = (col_left(&v.layout, ix), col_left(&v.layout, ix + 1));
        let y = row_top(&v.scroll, r);
        outline(
            rect(snap.v(x), snap.v(y), snap.v(x1) - snap.v(x), snap.v(row_h)),
            palette.seam_strong,
            window,
        );
    }
    paint_selection_boxes(v, g, palette, snap, window);

    // Layer 10: the resize guide.
    if let Some(x) = v.pointer.resize_guide(&v.layout) {
        let gx = body_x + (x - left_x) as f32;
        window.paint_quad(fill(snap.vline(gx, top, bottom), palette.text2));
    }
}

/// The rows of a selection box `rows` to draw: within the `visible` rows ± 1, so every drawn
/// edge is near the view (precise in `f32`) and a clamped edge falls outside the body.
fn box_rows(rows: std::ops::Range<u64>, visible: &std::ops::Range<u64>) -> std::ops::Range<u64> {
    rows.start.max(visible.start.saturating_sub(1))..rows.end.min(visible.end.saturating_add(1))
}

/// The 1 px `blue` boxes of the selection's pieces, the 5 px handle (in a 1 px `panel` ring) at
/// the bottom right of the box holding the focus.
fn paint_selection_boxes(
    v: &GridView,
    g: &Geometry,
    palette: &Palette,
    snap: Snap,
    window: &mut Window,
) {
    let loaded = v.data.count.loaded;
    if v.selection.is_empty() || loaded == 0 || g.cols.is_empty() || g.rows.is_empty() {
        return;
    }
    let (body_x, body_y) = (f32::from(g.body.left()), f32::from(g.body.top()));
    let left_x = v.scroll.left_x;
    let order = v.layout.visible();
    // Column positions (ordinals) to visible indices: [first visible ≥ start, last < end].
    let col_span = |cols: &std::ops::Range<u32>| -> Option<(usize, usize)> {
        let a = order.iter().position(|c| c.0 >= cols.start)?;
        let b = order.iter().rposition(|c| c.0 < cols.end)?;
        (a <= b).then_some((a, b + 1))
    };
    let visible_rows = g.rows.start..g.rows.end.min(loaded);
    let all_cols = (0, order.len());
    let mut boxes: smallvec::SmallVec<[(u64, u64, usize, usize); 4]> = smallvec::SmallVec::new();
    if v.selection.is_all() {
        boxes.push((visible_rows.start, visible_rows.end, all_cols.0, all_cols.1));
    } else {
        for rows in v.selection.rows() {
            boxes.push((rows.start, rows.end.min(loaded), all_cols.0, all_cols.1));
        }
        for cols in v.selection.cols() {
            if let Some((a, b)) = col_span(cols) {
                boxes.push((visible_rows.start, visible_rows.end, a, b));
            }
        }
        for r in v.selection.rects() {
            if let Some((a, b)) = col_span(&r.cols) {
                boxes.push((r.rows.start, r.rows.end.min(loaded), a, b));
            }
        }
    }
    let focus = v.selection.focus();
    let mut handle: Option<(f32, f32)> = None;
    for (r0, r1, c0, c1) in boxes {
        if r0 >= r1
            || r1 <= g.rows.start
            || r0 >= g.rows.end
            || c1 <= g.cols.start
            || c0 >= g.cols.end
        {
            continue;
        }
        let x = snap.v(body_x + (v.layout.x_of(c0) - left_x) as f32);
        let x1 = snap.v(body_x + (v.layout.x_of(c1) - left_x) as f32);
        // Rows far off screen would put an edge ~10⁸ px away, where an `f32` step is several
        // px and the visible edge drifts: draw the box's rows within the visible ones ± 1 (the
        // clamped edges stay outside the body's mask).
        let drawn = box_rows(r0..r1, &g.rows);
        let y = snap.v(body_y + v.scroll.row_top(drawn.start) as f32);
        let y1 = snap.v(body_y + v.scroll.row_top(drawn.end) as f32);
        outline(rect(x, y, x1 - x, y1 - y), palette.blue, window);
        let holds_focus = focus.is_some_and(|f| {
            let ix = v.layout.index_of(f.col);
            (r0..r1).contains(&f.row.0) && ix.is_some_and(|ix| (c0..c1).contains(&ix))
        });
        if holds_focus || handle.is_none() {
            handle = Some((x1, y1));
        }
    }
    if let Some((x1, y1)) = handle {
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

/// The header region: background, the selected columns' tint, names over mono types, the
/// column seams, the bottom border and the resize guide.
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
    let left_x = v.scroll.left_x;
    let GridView {
        paint,
        layout,
        columns,
        selection,
        pointer: ptr,
        ..
    } = &mut *v;
    let roles = paint.roles.as_ref().expect("roles are built in prepaint");
    let name_lh = 15.0;
    let type_lh = 13.0;
    let name_y = hy + ((hh - 1.0 - name_lh - 1.0 - type_lh) / 2.0).max(0.0);
    let type_y = name_y + name_lh + 1.0;
    for ix in g.cols.clone() {
        let col = layout.visible()[ix];
        let left = hx + (layout.x_of(ix) - left_x) as f32;
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
        masked(mask, window, |window| {
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
                let recorded = with_shape(
                    &mut paint.cache,
                    text,
                    role,
                    &roles.chip,
                    avail,
                    false,
                    budget,
                    &mut paint.scratch,
                    window,
                    cx,
                    |s, window, cx| {
                        let x = if end_aligned {
                            x0 + avail - s.width
                        } else {
                            x0
                        };
                        layers::paint_shaped(s, x, y, lh, palette, true, window, cx);
                        if recording {
                            layers::record_text(
                                cx,
                                text.clone(),
                                TextSource::Catalog,
                                role.nominal,
                                s.width,
                                s.truncated,
                                path,
                            );
                        }
                        (recording, s.truncated, s.degraded)
                    },
                );
                if let Some((recorded, truncated, _degraded)) = recorded {
                    budget.allocs += u32::from(recorded);
                    if truncated && is_name {
                        // An ellipsized name: the whole name is its tooltip (NOTES rule 7).
                        paint.header_tips.push(super::HeaderTip {
                            col,
                            name: text.clone(),
                            bounds: mask.intersect(&header),
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
        });
        // The seam between columns.
        window.paint_quad(fill(snap.vline(left + width, hy, hy + hh), palette.seam));
    }
    // The bottom border.
    window.paint_quad(fill(
        snap.hline(hx, f32::from(header.right()), hy + hh),
        palette.seam_strong,
    ));
    if let Some(x) = ptr.resize_guide(layout) {
        let gx = hx + (x - left_x) as f32;
        window.paint_quad(fill(snap.vline(gx, hy, hy + hh), palette.text2));
    }
}

/// The row-number gutter: background, row numbers (raw digits; `blue` for selected rows,
/// `text-2` on the hovered row), the row hairlines and the `seam` edge.
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
    let numbered = v.data.known_total().unwrap_or(v.data.count.loaded);
    let hover_row = v.pointer.hover.map(|(r, _)| r);
    let selection = &v.selection;
    let roles = v.paint.roles.as_ref().expect("roles are built in prepaint");
    let all = selection.is_all();
    for r in g.rows.clone() {
        let y = gy + v.scroll.row_top(r) as f32;
        if r < numbered {
            let selected = all
                || selection.row_selected(ModelRow(r))
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
    use crate::source::RowCount;

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

    /// GPUI's test window never activates accessibility (its `a11y_init` is a no-op), so this
    /// drives the element's hooks through the same declaration `request_layout` builds, with
    /// the active branch taken: role `Grid`, the label, the row and column counts (R-M3-32).
    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn the_element_declares_role_grid_while_assistive_technology_is_active(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::testing::{FixtureSource, grid_config};
        use hacivat_ui_core::theme::Theme;
        cx.update(|cx| Theme::install(cx, ThemeMode::Dark));
        let source = FixtureSource::texts(&[&["a", "b", "c"], &["d", "e", "f"]], 100.0);
        let (view, vcx) = cx.add_window_view(|_, cx| {
            GridView::new(std::rc::Rc::new(source), grid_config("Result 1"), cx)
        });
        vcx.run_until_parked();
        let mut element = GridElement::new(view.clone());
        vcx.update(|_, cx| element.refresh_a11y(false, cx));
        assert_eq!(element.a11y_role(), None);
        assert_eq!(view.read_with(vcx, |v, _| v.a11y_label_reads()), 0);
        vcx.update(|_, cx| element.refresh_a11y(true, cx));
        assert_eq!(element.a11y_role(), Some(Role::Grid));
        let mut node = accesskit::Node::new(Role::Grid);
        element.write_a11y_info(&mut node);
        assert_eq!(node.label(), Some("Result 1"));
        assert_eq!(node.row_count(), Some(2));
        assert_eq!(node.column_count(), Some(3));
        assert_eq!(view.read_with(vcx, |v, _| v.a11y_label_reads()), 1);
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
