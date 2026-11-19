//! The virtualized element, the row anatomy and the accessibility hand-off.
//!
//! **Virtualization.** Each frame builds, lays out and prepaints only the rows intersecting
//! the viewport plus [`TREE_OVERSCAN_ROWS`] on each side; labels cut by the viewport are
//! reported to the text registry as scrolling.
//!
//! **Geometry** (DESIGN.md Explorer Tree, NOTES M4 surface 1): 23 px rows (`tree_row_height`),
//! 14 px per level from an 8 px base (`tree_indent`), a 12 px chevron (swapped, never rotated),
//! the 12 px kind icon (`text-2`; `blue` while selected), the 13 px label (ellipsized, the full
//! name as its tooltip), then the flags (environment mark, lock: red only on production), the
//! outlined badge, the status glyph and the right-aligned 11 px mono meta at `text-3`. A crowded
//! row gives up the meta first and then the label, down to about six characters; flags, badge
//! and status never shrink. Hover fills `hover`; a selected row fills `blue-tint` with a 1 px
//! `blue` left edge. The keyboard-focus ring is a 1 px `blue` overlay on the cursor row (no
//! layout shift), shown only after keyboard input. GPUI splits a border-only quad into four
//! strips, so the selection edge is a 1 px child fill rather than a left border. A group
//! header is a small-caps label at `text-3`, a `seam-soft` rule and a mono count. Loading and
//! failed child rows show the model's label at `text-2` (a failed row adds `CircleAlert` in the
//! chevron column; never red, no motion). Nothing is drawn below 11 px.
//!
//! **Accessibility without a lag.** The element computes the items in its prepaint and hands
//! them to the root's synthetic-children callback through a shared `Rc`; GPUI runs that
//! callback after the children's prepaint, so the items describe the frame being drawn.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    A11ySubtreeBuilder, AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Element,
    ElementId, Entity, GlobalElementId, Hsla, InspectorElementId, InteractiveElement,
    IntoElement as _, LayoutId, MouseButton, MouseDownEvent, ParentElement, Pixels, SharedString,
    Stateful, StatefulInteractiveElement, Style, Styled, TextRun, Window, accesskit, div, point,
    px, relative, size,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use super::{TREE_OVERSCAN_ROWS, TreeView};
use crate::capsule::env_swatch;
use crate::icon::{Icon, IconName, IconSize};
use crate::text::{Label, TextRegistry, TextSource};
use crate::tree::a11y::{A11Y_OVERSCAN_ROWS, ITEM_ACTIONS, ItemNode, ItemsIn, visible_items};
use crate::tree::flat::RowKind;
use crate::tree::{ChildState, RowFlag, StatusKind, TreeKey, TreeModel};

/// How much of a name a crowded row keeps: about six characters and the ellipsis (the M2a
/// explorer's rule, which the profile roots keep, R-M4-18).
const NAME_MIN_CHARS: usize = 6;
/// The name gives way only once the meta is gone (NOTES M4 cross-cutting #5).
const NAME_SHRINK: f32 = 0.01;

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
        for ix in rows {
            let (mut el, top) = self.view.update(cx, |tree, cx| {
                let top = ix as f64 * f64::from(tree.row_h) - tree.scroll_y;
                (tree.build_row(ix, ring, window, cx), top as f32)
            });
            el.prepaint_as_root(
                point(bounds.left(), bounds.top() + px(top)),
                space,
                window,
                cx,
            );
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
fn row_id(key: TreeKey, kind: RowKind) -> ElementId {
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(&key.0.to_le_bytes());
    bytes[16] = kind as u8 + 1;
    ElementId::from(bytes)
}

/// The narrowest a label gets: its first [`NAME_MIN_CHARS`] characters and the ellipsis, or
/// the whole label when it is that short (the line layout is cached across frames).
fn name_min_width(name: &SharedString, style: TypeStyle, window: &mut Window) -> Pixels {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(NAME_MIN_CHARS).collect();
    let text: SharedString = if chars.next().is_some() {
        format!("{head}…").into()
    } else {
        name.clone()
    };
    let run = TextRun {
        len: text.len(),
        font: font_for(&style),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(style.size_px), &[run], None)
        .width
}

impl<M: TreeModel> TreeView<M> {
    fn build_row(
        &mut self,
        ix: usize,
        ring: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let row = self.flat.rows()[ix];
        let indent = t.spacing.md + f32::from(row.depth) * t.spacing.tree_indent;
        let chevron_w = IconSize::Chrome.px();
        let is_cursor = ring && self.cursor == Some(row.key) && row.kind == RowKind::Node;
        let base = div()
            .id(row_id(row.key, row.kind))
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(self.row_h))
            .pl(px(indent))
            .pr(px(t.spacing.md))
            .gap(px(t.spacing.xs));
        // The keyboard-focus ring: an overlay, so the row's content never shifts by a border.
        let ring_overlay = || {
            div()
                .absolute()
                .inset_0()
                .border_1()
                .border_color(c.blue.to_hsla())
        };
        if row.kind != RowKind::Node {
            let label = match self.model.children(row.key) {
                ChildState::Loading { label } | ChildState::Failed { label } => label,
                _ => SharedString::default(),
            };
            let mut marker = div().flex_none().w(px(chevron_w));
            if row.kind == RowKind::Failed {
                marker = marker.child(
                    Icon::new(IconName::CircleAlert)
                        .size(IconSize::Chrome)
                        .color(c.text_2.to_hsla()),
                );
            }
            return base
                .child(marker)
                .child(
                    div().min_w(px(0.0)).flex_shrink(1.0).child(
                        Label::new(
                            label,
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true)
                        .id("hv-tree-status-label"),
                    ),
                )
                .into_any_element();
        }
        let Some(data) = self.model.row(row.key) else {
            return base.into_any_element();
        };
        let expandable = self.model.children(row.key).expandable();
        let open = self.expanded.contains(&row.key);
        let key = row.key;
        let chevron = if expandable {
            div()
                .id("hv-tree-chevron")
                .flex_none()
                .w(px(chevron_w))
                .cursor_pointer()
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        this.on_chevron_down(key, window, cx);
                    }),
                )
                .into_any_element()
        } else {
            div().flex_none().w(px(chevron_w)).into_any_element()
        };
        let mouse = || {
            cx.listener(move |this: &mut Self, e: &MouseDownEvent, window, cx| {
                this.on_row_mouse_down(ix, e, window, cx);
            })
        };
        if data.group_header {
            let mut el = base
                .child(chevron)
                .child(div().flex_none().child(Label::new(
                    data.label,
                    data.label_source,
                    t.type_ramp.label_sc,
                    c.text_3.to_hsla(),
                )))
                .child(div().flex_1().h(px(1.0)).bg(c.seam_soft.to_hsla()));
            if let Some(meta) = data.meta {
                el = el.child(div().flex_none().child(Label::new(
                    meta,
                    TextSource::Fluent,
                    t.type_ramp.mono_micro,
                    c.text_3.to_hsla(),
                )));
            }
            if is_cursor {
                el = el.child(ring_overlay());
            }
            return el
                .on_mouse_down(MouseButton::Left, mouse())
                .on_mouse_down(MouseButton::Right, mouse())
                .into_any_element();
        }
        let selected = self.selection.contains(&row.key);
        let ink = if data.dim { c.text_3 } else { c.text };
        let icon_ink = if selected {
            c.blue
        } else if data.dim {
            c.text_3
        } else {
            c.text_2
        };
        let mut el = base.child(chevron);
        if let Some(icon) = data.icon {
            el = el.child(
                div()
                    .debug_selector(move || format!("hv-tree-icon-{}", key.0))
                    .flex_none()
                    .pr(px(t.spacing.s_2xs))
                    .child(
                        Icon::new(icon)
                            .size(IconSize::Chrome)
                            .color(icon_ink.to_hsla()),
                    ),
            );
        }
        let min_w = name_min_width(&data.label, t.type_ramp.body, window);
        el = el.child(
            div().min_w(min_w).flex_shrink(NAME_SHRINK).child(
                Label::new(
                    data.label,
                    data.label_source,
                    t.type_ramp.body,
                    ink.to_hsla(),
                )
                .truncate(true)
                .id("hv-tree-label"),
            ),
        );
        // Trust signals never shrink or clip (Global Constraints; NOTES cross-cutting #5).
        for (i, flag) in data.flags.into_iter().enumerate() {
            let mark = match flag {
                RowFlag::Env(kind) => env_swatch(kind, &theme).into_any_element(),
                RowFlag::Lock { production } => Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(if production { c.red } else { c.text_2 }.to_hsla())
                    .into_any_element(),
                RowFlag::Icon { icon, .. } => Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla())
                    .into_any_element(),
            };
            el = el.child(
                div()
                    .debug_selector(move || format!("hv-tree-flag-{}-{i}", key.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(mark),
            );
        }
        if let Some(badge) = data.badge {
            // Outlined: the capsule keeps the window's one filled badge.
            el = el.child(
                div()
                    .flex_none()
                    .px(px(t.spacing.xs))
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.xs))
                    .child(Label::new(
                        badge,
                        TextSource::Fluent,
                        t.type_ramp.micro,
                        c.text_2.to_hsla(),
                    )),
            );
        }
        if let Some(status) = data.status {
            // Never red (Hue Is Meaning); the text alternative is the item's description.
            let ink = match status.kind {
                StatusKind::Modified | StatusKind::Conflicted => c.amber,
                StatusKind::New => c.green,
                StatusKind::ContainsChanges => c.text_2,
                StatusKind::Unknown => c.text_3,
            };
            el = el.child(div().flex_none().child(Label::new(
                status.glyph,
                TextSource::Fluent,
                t.type_ramp.mono_micro,
                ink.to_hsla(),
            )));
        }
        el = el.child(div().flex_1());
        if let Some(meta) = data.meta {
            el = el.child(
                div().min_w(px(0.0)).flex_shrink(1.0).child(
                    Label::new(
                        meta,
                        TextSource::Data,
                        t.type_ramp.mono_micro,
                        c.text_3.to_hsla(),
                    )
                    .truncate(true)
                    .id("hv-tree-meta"),
                ),
            );
        }
        el = if selected {
            // The 1 px blue left edge: a child quad, so no row carries a border.
            el.bg(c.blue_tint.to_hsla()).child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(1.0))
                    .h_full()
                    .bg(c.blue.to_hsla()),
            )
        } else {
            el.hover(|s| s.bg(c.hover.to_hsla()))
        };
        if is_cursor {
            el = el.child(ring_overlay());
        }
        el.on_mouse_down(MouseButton::Left, mouse())
            .on_mouse_down(MouseButton::Right, mouse())
            .into_any_element()
    }
}
