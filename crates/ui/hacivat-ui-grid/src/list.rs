//! `VirtualList` (Task 20): one column, the grid's scroll model ([`ScrollState`]: a `u64` top
//! row and an offset inside it, coalesced deltas, follow-tail), a fixed row height. Only the
//! visible rows are built: each frame asks the [`RowRenderer`] for the rows that intersect the
//! viewport and nothing else, so a list of a million lines costs what its visible rows cost (the
//! output log, Task 30).

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Element, ElementId, Entity,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement,
    Pixels, Render, ScrollEvent, Style, Styled, Window, div, point, px, relative, size,
};

use crate::model::ScrollState;

/// The rows of a virtual list.
pub trait RowRenderer: 'static {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn row(&self, ix: u64, window: &mut Window, cx: &mut App) -> AnyElement;
}

/// A virtual list over `R` (a view: `cx.new(|_| VirtualList::new(rows, 18.0))`).
pub struct VirtualList<R: RowRenderer> {
    rows: R,
    row_height: f32,
    scroll: ScrollState,
    visible: Range<u64>,
}

impl<R: RowRenderer> std::fmt::Debug for VirtualList<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualList")
            .field("len", &self.rows.len())
            .field("row_height", &self.row_height)
            .field("scroll", &self.scroll)
            .finish()
    }
}

impl<R: RowRenderer> VirtualList<R> {
    /// A list of `rows`, each `row_height` px tall.
    pub fn new(rows: R, row_height: f32) -> Self {
        VirtualList {
            rows,
            row_height,
            scroll: ScrollState::new(row_height),
            visible: 0..0,
        }
    }

    pub fn rows(&self) -> &R {
        &self.rows
    }

    /// The rows, to change them (the next frame reads their new length).
    pub fn rows_mut(&mut self, cx: &mut Context<Self>) -> &mut R {
        cx.notify();
        &mut self.rows
    }

    pub fn row_height(&self) -> f32 {
        self.row_height
    }

    pub fn scroll_state(&self) -> &ScrollState {
        &self.scroll
    }

    /// The rows the last frame built.
    pub fn visible_rows(&self) -> Range<u64> {
        self.visible.clone()
    }

    /// Scrolls by `dy` px (positive towards later rows), with the frame's other deltas.
    pub fn scroll_px(&mut self, dy: f64, cx: &mut Context<Self>) {
        self.scroll.queue(dy, 0.0);
        cx.notify();
    }

    /// The last page, then following rows as they are appended.
    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroll.scroll_to_end();
        cx.notify();
    }

    pub fn scroll_to_top(&mut self, cx: &mut Context<Self>) {
        self.scroll.scroll_to_top();
        cx.notify();
    }

    /// Reveals row `ix`.
    pub fn scroll_into_view(&mut self, ix: u64, cx: &mut Context<Self>) {
        self.scroll.scroll_into_view(ix);
        cx.notify();
    }
}

impl<R: RowRenderer> Render for VirtualList<R> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("hv-virtual-list", cx.entity_id()))
            .size_full()
            .overflow_hidden()
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                // GPUI's deltas are positive when the content should move down.
                let d = e.delta.pixel_delta(px(this.row_height));
                this.scroll_px(-f64::from(f32::from(d.y)), cx);
            }))
            .child(ListElement { view: cx.entity() })
    }
}

/// The list's element: lays out and paints the visible rows only.
struct ListElement<R: RowRenderer> {
    view: Entity<VirtualList<R>>,
}

impl<R: RowRenderer> Element for ListElement<R> {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
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
    ) -> Vec<AnyElement> {
        let (rows, row_h) = self.view.update(cx, |list, _| {
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            list.scroll.set_viewport(w, h);
            list.scroll.on_row_count(list.rows.len());
            list.scroll.apply_pending();
            list.visible = list.scroll.visible_rows();
            (list.visible.clone(), list.row_height)
        });
        let space = size(
            AvailableSpace::Definite(bounds.size.width),
            AvailableSpace::Definite(px(row_h)),
        );
        let mut items = Vec::with_capacity((rows.end - rows.start) as usize);
        for ix in rows {
            let (mut el, top) = self.view.update(cx, |list, cx| {
                (
                    list.rows.row(ix, window, cx),
                    list.scroll.row_top(ix) as f32,
                )
            });
            el.prepaint_as_root(
                point(bounds.left(), bounds.top() + px(top)),
                space,
                window,
                cx,
            );
            items.push(el);
        }
        items
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        items: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for el in items.iter_mut() {
                el.paint(window, cx);
            }
        });
    }
}
