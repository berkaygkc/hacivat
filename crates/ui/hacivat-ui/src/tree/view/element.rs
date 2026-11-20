//! The virtualized element and the accessibility hand-off ([`super::row`] builds the rows).
//!
//! **Virtualization.** Each frame builds, lays out and prepaints only the rows intersecting
//! the viewport plus [`TREE_OVERSCAN_ROWS`] on each side; labels cut by the viewport are
//! reported to the text registry as scrolling.
//!
//! **Accessibility without a lag.** The element computes the items in its prepaint and hands
//! them to the root's synthetic-children callback through a shared `Rc`; GPUI runs that
//! callback after the children's prepaint, so the items describe the frame being drawn.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, AnyElement, App, AvailableSpace, Bounds, ContentMask, Element, ElementId,
    Entity, GlobalElementId, InspectorElementId, LayoutId, Pixels, Stateful,
    StatefulInteractiveElement, Style, Window, accesskit, point, px, relative, size,
};

use super::{TREE_OVERSCAN_ROWS, TreeView};
use crate::text::TextRegistry;
use crate::tree::a11y::{A11Y_OVERSCAN_ROWS, ITEM_ACTIONS, ItemNode, ItemsIn, visible_items};
use crate::tree::flat::RowKind;
use crate::tree::{TreeKey, TreeModel};

/// A rect in logical window px: left, top, right, bottom.
type Rect = (f32, f32, f32, f32);

/// What the element hands the root's accessibility callback within one frame.
#[derive(Default)]
pub(super) struct A11yFrame {
    /// Each item and its window rect (logical px).
    items: RefCell<Vec<(ItemNode, Rect)>>,
    scale: Cell<f32>,
    /// The pushed items' node ids that answer actions.
    ids: RefCell<Vec<(accesskit::NodeId, TreeKey)>>,
}

/// Makes `root` the tree node whose synthetic children are the items `frame` will hold.
pub(super) fn a11y_root(
    root: Stateful<hacivat_ui_core::gpui_surface::Div>,
    frame: Rc<A11yFrame>,
) -> Stateful<hacivat_ui_core::gpui_surface::Div> {
    StatefulInteractiveElement::a11y_synthetic_children(root, move |b: &mut A11ySubtreeBuilder| {
        let items = frame.items.borrow();
        let scale = frame.scale.get();
        let mut ids = frame.ids.borrow_mut();
        ids.clear();
        let mut focus = None;
        for (item, rect) in items.iter() {
            let mut node = item.node();
            let (x0, y0, x1, y1) = *rect;
            node.set_bounds(accesskit::Rect {
                x0: f64::from(x0 * scale),
                y0: f64::from(y0 * scale),
                x1: f64::from(x1 * scale),
                y1: f64::from(y1 * scale),
            });
            let id = b.synthetic_node_id(item.id_key());
            if b.push_child(id, node) {
                if item.focused {
                    focus = Some(id);
                }
                if item.focusable {
                    ids.push((id, item.key));
                }
            }
        }
        if let Some(id) = focus {
            b.parent_node().set_active_descendant(id);
        }
    })
}

/// The tree's element: lays out and paints the visible rows only.
pub(super) struct TreeElement<M: TreeModel> {
    view: Entity<TreeView<M>>,
    a11y: Option<Rc<A11yFrame>>,
}

impl<M: TreeModel> TreeElement<M> {
    pub(super) fn new(view: Entity<TreeView<M>>, a11y: Option<Rc<A11yFrame>>) -> Self {
        TreeElement { view, a11y }
    }
}

impl<M: TreeModel> Element for TreeElement<M> {
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
        let started = Instant::now();
        let (rows, row_h, ring) = self.view.update(cx, |tree, _| {
            tree.bounds = Some(bounds);
            tree.clamp_scroll();
            let visible = tree.visible();
            let rows = visible.start.saturating_sub(TREE_OVERSCAN_ROWS)
                ..(visible.end + TREE_OVERSCAN_ROWS).min(tree.flat.len());
            tree.built = rows.clone();
            let ring = tree.focus.is_focused(window) && window.last_input_was_keyboard();
            (rows, tree.row_h, ring)
        });
        let space = size(
            AvailableSpace::Definite(bounds.size.width),
            AvailableSpace::Definite(px(row_h)),
        );
        let mut items = Vec::with_capacity(rows.len());
        // ## m4-t32-shots (named exception): rows are laid out (and their labels measured)
        // inside the tree's own clip, noted as a scroll viewport first, so a row cut at the
        // tree's top or bottom edge counts as scrolled, not clipped (the 15,000-table shot).
        let mask = window.content_mask().bounds.intersect(&bounds);
        TextRegistry::note_scroll_viewport(cx, mask);
        for ix in rows {
            let (mut el, top) = self.view.update(cx, |tree, cx| {
                let top = ix as f64 * f64::from(tree.row_h) - tree.scroll_y;
                (
                    tree.build_row(ix, ring, f32::from(bounds.size.width), window, cx),
                    top as f32,
                )
            });
            window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
                el.prepaint_as_root(
                    point(bounds.left(), bounds.top() + px(top)),
                    space,
                    window,
                    cx,
                );
            });
            items.push(el);
        }
        if let Some(frame) = &self.a11y {
            let scale = window.scale_factor();
            self.view.update(cx, |tree, _| {
                tree.a11y.builds += 1;
                let items = visible_items(
                    &tree.model,
                    ItemsIn {
                        flat: &tree.flat,
                        visible: tree.visible(),
                        overscan: A11Y_OVERSCAN_ROWS,
                        expanded: &tree.expanded,
                        selection: &tree.selection,
                        cursor: tree.cursor,
                    },
                );
                let (left, right) = (f32::from(bounds.left()), f32::from(bounds.right()));
                let rects = items
                    .iter()
                    .map(|item| {
                        let top = f32::from(bounds.top())
                            + (item.row as f64 * f64::from(tree.row_h) - tree.scroll_y) as f32;
                        (item.clone(), (left, top, right, top + tree.row_h))
                    })
                    .collect();
                *frame.items.borrow_mut() = rects;
                frame.scale.set(scale);
                tree.a11y.items = Rc::new(items);
            });
        }
        let took = started.elapsed();
        self.view.update(cx, |tree, _| tree.last_build = took);
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
        TextRegistry::note_scroll_viewport(cx, bounds);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for el in items.iter_mut() {
                el.paint(window, cx);
            }
        });
        if let Some(frame) = &self.a11y {
            let view = self.view.downgrade();
            for (id, key) in frame.ids.borrow().iter().copied() {
                for action in ITEM_ACTIONS {
                    let view = view.clone();
                    window.on_a11y_action(id, action, move |_, window, cx| {
                        let _ = view.update(cx, |v, cx| v.a11y_action(key, action, window, cx));
                    });
                }
            }
        }
    }
}

/// A row's element id: its key and kind (stable across scrolls and generations).
pub(super) fn row_id(key: TreeKey, kind: RowKind) -> ElementId {
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(&key.0.to_le_bytes());
    bytes[16] = kind as u8 + 1;
    ElementId::from(bytes)
}
