//! Pointer handling (HV-GRID-05 pointer; Task 20): hit testing through the inverse prefix
//! sums, clicks on cells (Shift extends, Cmd/Ctrl adds), the gutter (rows), the header
//! (columns) and the corner (everything), drag selection with edge autoscroll capped at 40 rows
//! per frame, scrollbar thumb drags and track jumps, and column resizing from a header's right
//! edge (`ColumnResized` when the drag ends).

use hacivat_ui_core::gpui_surface::{
    App, Context, CursorStyle, Entity, Hitbox, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Window,
};

use super::scrollbar;
use super::{Geometry, RESIZE_GRAB_PX};
use crate::model::ViewMap;
use crate::model::autosize::clamp_user_width;
use crate::source::{ColId, ModelCell, ModelRow};
use crate::view::{GridEvent, GridView};

/// Edge autoscroll while drag-selecting moves at most this many rows per event.
pub const AUTOSCROLL_MAX_ROWS: f64 = 40.0;

/// What a pointer drag is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Drag {
    Cells,
    Rows,
    Cols,
    Resize {
        col: ColId,
        start_x: f32,
        start_w: f32,
        width: f32,
    },
    VThumb {
        grab: f32,
    },
    HThumb {
        grab: f32,
    },
}

/// The pointer's state across frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct PointerState {
    /// The hovered body cell: view row and visible column index.
    pub hover: Option<(u64, usize)>,
    /// The header edge under the pointer (visible column index).
    pub border: Option<usize>,
    pub drag: Option<Drag>,
    /// Where the pointer last moved over the grid: each frame re-derives the hover from it, so
    /// after a wheel scroll the hover follows the pointer, not the row it was on.
    pub position: Option<Point<Pixels>>,
}

impl PointerState {
    /// The x of the resize guide while a column is being resized, in content px.
    pub fn resize_guide(&self, layout: &crate::model::ColumnLayout) -> Option<f64> {
        match self.drag {
            Some(Drag::Resize { col, .. }) => layout.index_of(col).map(|ix| layout.x_of(ix + 1)),
            _ => None,
        }
    }
}

/// What lies under a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Hit {
    Corner,
    /// The resize edge of a header (visible column index).
    Border(usize),
    Header(usize),
    Gutter(u64),
    /// A body cell: view row and visible column index (the row may be past the loaded rows).
    Cell(u64, usize),
    VThumb(f32),
    VTrack(f32),
    HThumb(f32),
    HTrack(f32),
    Nothing,
}

fn contains(b: &hacivat_ui_core::gpui_surface::Bounds<Pixels>, p: Point<Pixels>) -> bool {
    p.x >= b.left() && p.x < b.right() && p.y >= b.top() && p.y < b.bottom()
}

impl GridView {
    /// What lies under `p` in the last frame.
    pub(crate) fn hit_test(&self, g: &Geometry, p: Point<Pixels>) -> Hit {
        let (x, y) = (f32::from(p.x), f32::from(p.y));
        if !contains(&g.bounds, p) {
            return Hit::Nothing;
        }
        if contains(&g.corner, p) {
            return Hit::Corner;
        }
        let content_x = |x: f32| self.scroll.left_x + f64::from(x - f32::from(g.body.left()));
        if contains(&g.header, p) {
            for ix in g.cols.clone() {
                let right = f32::from(g.body.left())
                    + (self.layout.x_of(ix + 1) - self.scroll.left_x) as f32;
                if (x - right).abs() <= RESIZE_GRAB_PX {
                    return Hit::Border(ix);
                }
            }
            return self
                .layout
                .col_at_x(content_x(x))
                .map_or(Hit::Nothing, Hit::Header);
        }
        if contains(&g.gutter, p) {
            return self
                .scroll
                .row_at(y - f32::from(g.body.top()))
                .map_or(Hit::Nothing, Hit::Gutter);
        }
        if contains(&g.body, p) {
            let row = self.scroll.row_at(y - f32::from(g.body.top()));
            let col = self.layout.col_at_x(content_x(x));
            return match (row, col) {
                (Some(r), Some(c)) => Hit::Cell(r, c),
                _ => Hit::Nothing,
            };
        }
        if contains(&g.vtrack, p) {
            let thumb = scrollbar::v_thumb(g.vtrack, &self.scroll);
            if y >= f32::from(thumb.top()) && y < f32::from(thumb.bottom()) {
                return Hit::VThumb(y - f32::from(thumb.top()));
            }
            return Hit::VTrack(y);
        }
        if let Some(track) = g.htrack
            && contains(&track, p)
        {
            let thumb = scrollbar::h_thumb(track, &self.scroll);
            if x >= f32::from(thumb.left()) && x < f32::from(thumb.right()) {
                return Hit::HThumb(x - f32::from(thumb.left()));
            }
            return Hit::HTrack(x);
        }
        Hit::Nothing
    }

    /// The cell a drag at `p` reaches: clamped into the body and the loaded rows.
    fn drag_cell(&self, g: &Geometry, p: Point<Pixels>) -> Option<ModelCell> {
        let loaded = self.data.count.loaded;
        let order = self.layout.visible();
        if loaded == 0 || order.is_empty() {
            return None;
        }
        let body = g.body;
        let y = f32::from(p.y).clamp(f32::from(body.top()), f32::from(body.bottom()) - 1.0);
        let x = f32::from(p.x).clamp(f32::from(body.left()), f32::from(body.right()) - 1.0);
        let row = self
            .scroll
            .row_at(y - f32::from(body.top()))
            .unwrap_or_else(|| self.scroll.row_count().saturating_sub(1))
            .min(loaded - 1);
        let content = self.scroll.left_x + f64::from(x - f32::from(body.left()));
        let ix = self
            .layout
            .col_at_x(content)
            .unwrap_or(order.len() - 1)
            .min(order.len() - 1);
        Some(ModelCell {
            row: ModelRow(row),
            col: order[ix],
        })
    }

    pub(crate) fn mouse_down(
        &mut self,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if e.button != MouseButton::Left {
            return;
        }
        window.focus(&self.focus, cx);
        let Some(g) = self.geometry.clone() else {
            return;
        };
        let loaded = self.data.count.loaded;
        let map = ViewMap::identity(loaded);
        let extend = e.modifiers.shift;
        let add = e.modifiers.secondary(cx);
        let focus_row = self.selection.focus().map_or(ModelRow(0), |f| f.row);
        let first_col = self.layout.visible().first().copied();
        let focus_col = self.selection.focus().map(|f| f.col).or(first_col);
        let before = self.selection.clone();
        match self.hit_test(&g, e.position) {
            Hit::Corner => {
                self.selection.select_all();
                self.pointer.drag = None;
            }
            Hit::Border(ix) => {
                if let Some(col) = self.layout.visible().get(ix).copied() {
                    let w = self.layout.width_at(ix);
                    self.pointer.drag = Some(Drag::Resize {
                        col,
                        start_x: f32::from(e.position.x),
                        start_w: w,
                        width: w,
                    });
                }
            }
            Hit::Header(ix) => {
                if let Some(col) = self.layout.visible().get(ix).copied() {
                    if extend {
                        self.selection.extend_columns_to(col);
                    } else if add {
                        self.selection.add_column(col, focus_row);
                    } else {
                        self.selection.select_column(col, focus_row);
                    }
                    self.pointer.drag = Some(Drag::Cols);
                }
            }
            Hit::Gutter(row) if row < loaded => {
                if let Some(col) = focus_col {
                    if extend {
                        self.selection.extend_rows_to(ModelRow(row));
                    } else if add {
                        self.selection.add_row(ModelRow(row), col);
                    } else {
                        self.selection.select_row(ModelRow(row), col);
                    }
                    self.pointer.drag = Some(Drag::Rows);
                }
            }
            Hit::Cell(row, ix) if row < loaded => {
                if let Some(col) = self.layout.visible().get(ix).copied() {
                    let cell = ModelCell {
                        row: ModelRow(row),
                        col,
                    };
                    if extend {
                        self.selection.extend_to(cell, &map);
                    } else if add {
                        self.selection.add_cell(cell, &map);
                    } else {
                        self.selection.select_cell(cell, &map);
                    }
                    self.pointer.drag = Some(Drag::Cells);
                }
            }
            Hit::VThumb(grab) => self.pointer.drag = Some(Drag::VThumb { grab }),
            Hit::VTrack(y) => {
                let (top, travel) = scrollbar::v_travel(g.vtrack);
                let len = f32::from(scrollbar::v_thumb(g.vtrack, &self.scroll).size.height);
                self.scroll
                    .drag_thumb(f64::from(y - top - len / 2.0), travel);
                self.pointer.drag = Some(Drag::VThumb { grab: len / 2.0 });
            }
            Hit::HThumb(grab) => self.pointer.drag = Some(Drag::HThumb { grab }),
            Hit::HTrack(x) => {
                if let Some(track) = g.htrack {
                    let (left, travel) = scrollbar::h_travel(track);
                    let len = f32::from(scrollbar::h_thumb(track, &self.scroll).size.width);
                    self.scroll
                        .drag_h_thumb(f64::from(x - left - len / 2.0), travel);
                    self.pointer.drag = Some(Drag::HThumb { grab: len / 2.0 });
                }
            }
            Hit::Gutter(_) | Hit::Cell(..) | Hit::Nothing => {}
        }
        if self.selection != before {
            self.mark_selection_changed();
            if self.selection.focus() != before.focus() {
                cx.emit(GridEvent::FocusChanged(self.selection.focus()));
            }
        }
        cx.notify();
    }

    /// The hovered body cell and header edge at `p` in the frame `g`.
    pub(crate) fn hover_at(
        &self,
        g: &Geometry,
        p: Point<Pixels>,
    ) -> (Option<(u64, usize)>, Option<usize>) {
        match self.hit_test(g, p) {
            Hit::Cell(r, c) => (Some((r, c)), None),
            Hit::Border(ix) => (None, Some(ix)),
            _ => (None, None),
        }
    }

    pub(crate) fn mouse_move(&mut self, e: &MouseMoveEvent, hovered: bool, cx: &mut Context<Self>) {
        let Some(g) = self.geometry.clone() else {
            return;
        };
        if let Some(stale) = self.pointer.drag
            && !e.dragging()
        {
            // The button went up where we never heard it (outside the window): the drag ended.
            self.pointer.drag = None;
            if let Drag::Resize { col, width, .. } = stale {
                cx.emit(GridEvent::ColumnResized { col, width });
            }
            cx.notify();
        }
        let Some(drag) = self.pointer.drag else {
            self.pointer.position = hovered.then_some(e.position);
            let (hover, border) = if hovered {
                self.hover_at(&g, e.position)
            } else {
                (None, None)
            };
            if (hover, border) != (self.pointer.hover, self.pointer.border) {
                self.pointer.hover = hover;
                self.pointer.border = border;
                cx.notify();
            }
            return;
        };
        let map = ViewMap::identity(self.data.count.loaded);
        let before = self.selection.clone();
        match drag {
            Drag::Cells => {
                self.autoscroll(&g, e.position);
                if let Some(cell) = self.drag_cell(&g, e.position) {
                    self.selection.extend_to(cell, &map);
                }
            }
            Drag::Rows => {
                self.autoscroll(&g, e.position);
                if let Some(cell) = self.drag_cell(&g, e.position) {
                    self.selection.extend_rows_to(cell.row);
                }
            }
            Drag::Cols => {
                if let Some(cell) = self.drag_cell(&g, e.position) {
                    self.selection.extend_columns_to(cell.col);
                }
            }
            Drag::Resize {
                col,
                start_x,
                start_w,
                ..
            } => {
                let width = clamp_user_width(start_w + (f32::from(e.position.x) - start_x));
                if self.layout.set_width(col, width) {
                    self.autosize.mark_user_sized(col);
                    self.scroll.set_content_width(self.layout.total_width());
                    self.pointer.drag = Some(Drag::Resize {
                        col,
                        start_x,
                        start_w,
                        width,
                    });
                }
            }
            Drag::VThumb { grab } => {
                let (top, travel) = scrollbar::v_travel(g.vtrack);
                self.scroll
                    .drag_thumb(f64::from(f32::from(e.position.y) - top - grab), travel);
            }
            Drag::HThumb { grab } => {
                if let Some(track) = g.htrack {
                    let (left, travel) = scrollbar::h_travel(track);
                    self.scroll
                        .drag_h_thumb(f64::from(f32::from(e.position.x) - left - grab), travel);
                }
            }
        }
        if self.selection != before {
            self.mark_selection_changed();
            if self.selection.focus() != before.focus() {
                cx.emit(GridEvent::FocusChanged(self.selection.focus()));
            }
        }
        cx.notify();
    }

    pub(crate) fn mouse_up(&mut self, e: &MouseUpEvent, cx: &mut Context<Self>) {
        if e.button != MouseButton::Left {
            return;
        }
        if let Some(Drag::Resize { col, width, .. }) = self.pointer.drag.take() {
            cx.emit(GridEvent::ColumnResized { col, width });
            cx.notify();
        } else if self.pointer.drag.is_some() {
            self.pointer.drag = None;
            cx.notify();
        }
    }

    /// Scrolls towards a drag that left the body, in proportion to the distance, capped at
    /// [`AUTOSCROLL_MAX_ROWS`].
    fn autoscroll(&mut self, g: &Geometry, p: Point<Pixels>) {
        let (y, top, bottom) = (
            f32::from(p.y),
            f32::from(g.body.top()),
            f32::from(g.body.bottom()),
        );
        let over = if y < top {
            f64::from(y - top)
        } else if y >= bottom {
            f64::from(y - bottom)
        } else {
            0.0
        };
        if over != 0.0 {
            let cap = AUTOSCROLL_MAX_ROWS * f64::from(self.scroll.row_height());
            self.scroll.scroll_by(over.clamp(-cap, cap));
        }
    }
}

/// Registers the frame's pointer listeners (paint phase) and the resize cursor.
pub(crate) fn register(view: &Entity<GridView>, hitbox: &Hitbox, window: &mut Window, cx: &App) {
    let state = view.read(cx).pointer;
    if state.border.is_some() || matches!(state.drag, Some(Drag::Resize { .. })) {
        window.set_cursor_style(CursorStyle::ResizeLeftRight, hitbox);
    }
    let (down_view, down_hit) = (view.clone(), hitbox.clone());
    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
        if phase.bubble() && down_hit.is_hovered(window) {
            down_view.update(cx, |v, cx| v.mouse_down(e, window, cx));
        }
    });
    let (move_view, move_hit) = (view.clone(), hitbox.clone());
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
        if phase.bubble() {
            let hovered = move_hit.is_hovered(window);
            let interested = hovered || {
                let s = move_view.read(cx).pointer;
                s.drag.is_some() || s.hover.is_some() || s.border.is_some()
            };
            if interested {
                move_view.update(cx, |v, cx| v.mouse_move(e, hovered, cx));
            }
        }
    });
    let up_view = view.clone();
    window.on_mouse_event(move |e: &MouseUpEvent, phase, _window, cx| {
        if phase.bubble() && up_view.read(cx).pointer.drag.is_some() {
            up_view.update(cx, |v, cx| v.mouse_up(e, cx));
        }
    });
}
