//! Panes, splits and tabs.
//!
//! [`PaneTree`] is the model: a binary tree of splits over leaf panes, each holding tab IDs
//! (not views), serializable so a workspace can persist its layout. [`TabStrip`] draws one
//! pane's tabs: 30 px, the active tab raised to join the toolbar below.
//!
//! `Axis::Horizontal` places the two children side by side (`a` left, `b` right) and
//! `Axis::Vertical` stacks them (`a` above, `b` below).

use std::collections::BTreeMap;
use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};
use serde::{Deserialize, Serialize};

pub use hacivat_ui_core::gpui_surface::Axis;

use crate::button::{hover_listener, interaction_state};
use crate::capsule::{EnvKind, env_swatch};
use crate::icon::{Icon, IconName, IconSize};
use crate::l10n::ui_text;
use crate::popover::Declare;
use crate::text::{Label, TextSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PaneId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TabId(pub u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PaneNode {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// The share of the space the first child takes.
        ratio: f32,
        a: Box<PaneNode>,
        b: Box<PaneNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pane {
    pub tabs: Vec<TabId>,
    pub active: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// Bounds of a split ratio (the view additionally keeps each side at least 120 px).
pub const MIN_RATIO: f32 = 0.1;
pub const MAX_RATIO: f32 = 0.9;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaneTree {
    pub root: PaneNode,
    pub panes: BTreeMap<PaneId, Pane>,
    pub focused: PaneId,
}

impl PaneTree {
    /// One pane (id 0) holding `first_tab`.
    pub fn new(first_tab: TabId) -> Self {
        let id = PaneId(0);
        Self {
            root: PaneNode::Leaf(id),
            panes: BTreeMap::from([(
                id,
                Pane {
                    tabs: vec![first_tab],
                    active: 0,
                },
            )]),
            focused: id,
        }
    }

    /// Splits `pane`: it keeps its place and a new pane holding `new_tab` takes the second half.
    /// The new pane gets the focus. An unknown `pane` changes nothing and returns it.
    pub fn split(&mut self, pane: PaneId, axis: Axis, new_tab: TabId) -> PaneId {
        if !self.panes.contains_key(&pane) {
            return pane;
        }
        let new = PaneId(self.panes.keys().next_back().map_or(0, |id| id.0 + 1));
        self.panes.insert(
            new,
            Pane {
                tabs: vec![new_tab],
                active: 0,
            },
        );
        replace_leaf(&mut self.root, pane, &mut |leaf| PaneNode::Split {
            axis,
            ratio: 0.5,
            a: Box::new(leaf),
            b: Box::new(PaneNode::Leaf(new)),
        });
        self.focused = new;
        new
    }

    /// Closes `tab` in `pane` and returns the pane's active tab afterwards. Closing the active
    /// tab activates its left neighbour (its right one for the first tab). A pane left empty
    /// disappears and its sibling takes the space, unless it is the only pane.
    pub fn close_tab(&mut self, pane: PaneId, tab: TabId) -> Option<TabId> {
        let p = self.panes.get_mut(&pane)?;
        let at = p.tabs.iter().position(|&t| t == tab)?;
        p.tabs.remove(at);
        if p.tabs.is_empty() {
            p.active = 0;
            if self.panes.len() > 1 {
                self.remove_pane(pane);
            }
            return None;
        }
        if at < p.active || (at == p.active && at > 0) {
            p.active -= 1;
        }
        p.active = p.active.min(p.tabs.len() - 1);
        Some(p.tabs[p.active])
    }

    fn remove_pane(&mut self, pane: PaneId) {
        let order = self.leaves();
        self.panes.remove(&pane);
        let root = std::mem::replace(&mut self.root, PaneNode::Leaf(pane));
        if let Some(root) = without_leaf(root, pane) {
            self.root = root;
        }
        if self.focused == pane {
            let at = order.iter().position(|&p| p == pane).unwrap_or(0);
            self.focused = at
                .checked_sub(1)
                .and_then(|i| order.get(i))
                .or_else(|| order.get(at + 1))
                .copied()
                .unwrap_or(self.focused);
        }
    }

    /// The leaves in layout order (left to right, top to bottom).
    pub fn leaves(&self) -> Vec<PaneId> {
        fn walk(node: &PaneNode, out: &mut Vec<PaneId>) {
            match node {
                PaneNode::Leaf(id) => out.push(*id),
                PaneNode::Split { a, b, .. } => {
                    walk(a, out);
                    walk(b, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// Moves the focus to the neighbouring pane in `dir`; nothing happens at an edge.
    pub fn move_focus(&mut self, dir: FocusDir) {
        let rects = self.rects();
        let Some(cur) = rects.get(&self.focused).copied() else {
            return;
        };
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.0);
        const EPS: f32 = 1e-4;
        let mut best: Option<(f32, f32, PaneId)> = None;
        for (&id, r) in &rects {
            if id == self.focused {
                continue;
            }
            let (gap, along) = match dir {
                FocusDir::Right => (r.0 - cur.2, overlap(cur.1, cur.3, r.1, r.3)),
                FocusDir::Left => (cur.0 - r.2, overlap(cur.1, cur.3, r.1, r.3)),
                FocusDir::Down => (r.1 - cur.3, overlap(cur.0, cur.2, r.0, r.2)),
                FocusDir::Up => (cur.1 - r.3, overlap(cur.0, cur.2, r.0, r.2)),
            };
            if gap < -EPS || along <= EPS {
                continue;
            }
            let better = best.is_none_or(|(g, o, _)| gap < g - EPS || (gap < g + EPS && along > o));
            if better {
                best = Some((gap, along, id));
            }
        }
        if let Some((_, _, id)) = best {
            self.focused = id;
        }
    }

    /// Pane rectangles in the unit square as `(x0, y0, x1, y1)`.
    fn rects(&self) -> BTreeMap<PaneId, (f32, f32, f32, f32)> {
        fn walk(
            node: &PaneNode,
            r: (f32, f32, f32, f32),
            out: &mut BTreeMap<PaneId, (f32, f32, f32, f32)>,
        ) {
            match node {
                PaneNode::Leaf(id) => {
                    out.insert(*id, r);
                }
                PaneNode::Split { axis, ratio, a, b } => {
                    let (x0, y0, x1, y1) = r;
                    let (ra, rb) = match axis {
                        Axis::Horizontal => {
                            let x = x0 + (x1 - x0) * ratio;
                            ((x0, y0, x, y1), (x, y0, x1, y1))
                        }
                        Axis::Vertical => {
                            let y = y0 + (y1 - y0) * ratio;
                            ((x0, y0, x1, y), (x0, y, x1, y1))
                        }
                    };
                    walk(a, ra, out);
                    walk(b, rb, out);
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(&self.root, (0.0, 0.0, 1.0, 1.0), &mut out);
        out
    }

    /// Sets the ratio of the split at `path` (`false` = first child, `true` = second, from the
    /// root), clamped to [0.1, 0.9]; a non-finite ratio is ignored. A path that does not end at a split changes nothing.
    pub fn set_ratio(&mut self, path: &[bool], ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        let mut node = &mut self.root;
        for &second in path {
            match node {
                PaneNode::Split { a, b, .. } => node = if second { b } else { a },
                PaneNode::Leaf(_) => return,
            }
        }
        if let PaneNode::Split { ratio: r, .. } = node {
            *r = ratio.clamp(MIN_RATIO, MAX_RATIO);
        }
    }

    /// Activates tab `idx` of `pane` and focuses the pane. An out-of-range index changes nothing.
    pub fn activate(&mut self, pane: PaneId, idx: usize) {
        if let Some(p) = self.panes.get_mut(&pane)
            && idx < p.tabs.len()
        {
            p.active = idx;
            self.focused = pane;
        }
    }

    /// Activates the focused pane's next tab, wrapping around.
    pub fn next_tab(&mut self) {
        self.step_tab(true);
    }

    /// Activates the focused pane's previous tab, wrapping around.
    pub fn prev_tab(&mut self) {
        self.step_tab(false);
    }

    fn step_tab(&mut self, forward: bool) {
        if let Some(p) = self.panes.get_mut(&self.focused) {
            let n = p.tabs.len();
            if n > 0 {
                p.active = if forward {
                    (p.active + 1) % n
                } else {
                    (p.active + n - 1) % n
                };
            }
        }
    }
}

/// Replaces the leaf `id` with `f(leaf)`.
fn replace_leaf(node: &mut PaneNode, id: PaneId, f: &mut impl FnMut(PaneNode) -> PaneNode) {
    match node {
        PaneNode::Leaf(leaf) if *leaf == id => {
            let old = std::mem::replace(node, PaneNode::Leaf(id));
            *node = f(old);
        }
        PaneNode::Leaf(_) => {}
        PaneNode::Split { a, b, .. } => {
            replace_leaf(a, id, f);
            replace_leaf(b, id, f);
        }
    }
}

/// `node` without the leaf `id`; the sibling of a removed leaf takes its parent's place.
fn without_leaf(node: PaneNode, id: PaneId) -> Option<PaneNode> {
    match node {
        PaneNode::Leaf(leaf) => (leaf != id).then_some(PaneNode::Leaf(leaf)),
        PaneNode::Split { axis, ratio, a, b } => {
            match (without_leaf(*a, id), without_leaf(*b, id)) {
                (Some(a), Some(b)) => Some(PaneNode::Split {
                    axis,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (a, b) => a.or(b),
            }
        }
    }
}

/// How the environment of a tab is marked (a 7 px swatch; production is red).
pub type EnvMark = EnvKind;

/// What a tab shows. Only the active or hovered tab shows its close button. The strip is rebuilt every frame from IDs, so `title` may be a rendered
/// string.
#[derive(Debug, Clone)]
pub struct TabSpec {
    pub id: TabId,
    pub title: SharedString,
    /// `Data` and `Catalog` titles (file and table names) are set in mono.
    pub title_source: TextSource,
    /// Unsaved changes: a dot after the title.
    pub dirty: bool,
    pub env: Option<EnvMark>,
}

type TabHandler = Rc<dyn Fn(PaneId, TabId, &mut Window, &mut App)>;

/// One pane's tab strip.
pub struct TabStrip {
    pane: PaneId,
    tabs: Vec<TabSpec>,
    active: usize,
    on_activate: Option<TabHandler>,
    on_close: Option<TabHandler>,
}

impl TabStrip {
    pub fn new(pane: PaneId, tabs: Vec<TabSpec>) -> Self {
        Self {
            pane,
            tabs,
            active: 0,
            on_activate: None,
            on_close: None,
        }
    }

    /// The index of the active tab (the pane's `active`).
    pub fn active(mut self, active: usize) -> Self {
        self.active = active;
        self
    }

    pub fn on_activate(
        mut self,
        handler: impl Fn(PaneId, TabId, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    pub fn on_close(
        mut self,
        handler: impl Fn(PaneId, TabId, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

impl IntoElement for TabStrip {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        crate::popover::lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}

impl TabStrip {
    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let close_name = ui_text(cx, "hui-tab-close");
        let height = px(t.spacing.panel_header_height);
        let pane = self.pane;
        let mut strip = div()
            .id(ElementId::from((
                SharedString::from("tab-strip"),
                pane.0 as usize,
            )))
            .debug_selector(|| "tab-strip".to_string())
            .declare(Role::TabList, ui_text(cx, "hui-tabs"), None)
            .flex()
            .flex_row()
            .items_start()
            .h(height)
            .flex_none()
            .bg(c.head.to_hsla());
        for (index, tab) in self.tabs.into_iter().enumerate() {
            let active = index == self.active;
            let id = ElementId::from((SharedString::from("tab"), tab.id.0 as usize));
            let (hover, _) = interaction_state(&id, window, cx);
            let hovered = *hover.read(cx);
            let ink = if active { c.text } else { c.text_2 }.to_hsla();
            let style = match tab.title_source {
                TextSource::Data | TextSource::Catalog => TypeStyle {
                    size_px: 12.0,
                    line_height: 12.0 * 1.35,
                    ..t.type_ramp.code
                },
                _ => t.type_ramp.body,
            };
            let selector = format!("tab-{}", tab.id.0);
            let tab_id = tab.id;
            let mut el = div()
                .id(id)
                .debug_selector(move || selector)
                .declare(Role::Tab, tab.title.clone(), Some(active))
                .flex()
                .flex_row()
                .items_center()
                .h(height)
                .flex_none()
                .px(px(12.0))
                .gap(px(7.0))
                .border_r_1()
                .border_color(c.seam.to_hsla())
                .cursor_pointer()
                .child(
                    Icon::new(IconName::FileCode)
                        .size(IconSize::Chrome)
                        .color(ink),
                )
                .child(Label::new(tab.title, tab.title_source, style, ink));
            // No left border on the first tab: the strip starts at a tile seam (or the window
            // edge), and a second line there would make a 2 px seam (Task 18 review).
            if active {
                el = el.bg(c.raised.to_hsla());
            } else if hovered {
                el = el.bg(c.hover.to_hsla());
            }
            if let Some(env) = tab.env {
                el = el.child(env_swatch(env, &theme));
            }
            if tab.dirty {
                el = el.child(
                    div()
                        .size(px(6.0))
                        .flex_none()
                        .rounded_full()
                        .bg(c.text_2.to_hsla()),
                );
            }
            let close_id = ElementId::from((SharedString::from("tab-close"), tab.id.0 as usize));
            let close_selector = format!("tab-close-{}", tab.id.0);
            let on_close = self.on_close.clone();
            if active || hovered {
                el = el.child(
                    div()
                        .id(close_id)
                        .debug_selector(move || close_selector)
                        .declare(Role::Button, close_name.clone(), None)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(handler) = &on_close {
                                handler(pane, tab_id, window, cx);
                            }
                        })
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Chrome)
                                .color(c.text_3.to_hsla()),
                        ),
                );
            }
            let on_activate = self.on_activate.clone();
            el = el
                .on_hover(hover_listener(hover))
                .on_click(move |_, window, cx| {
                    if let Some(handler) = &on_activate {
                        handler(pane, tab_id, window, cx);
                    }
                });
            strip = strip.child(el);
        }
        strip.into_any_element()
    }
}
