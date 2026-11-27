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
    AnyElement, App, ElementId, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Role, SharedString, StatefulInteractiveElement, Styled, TextRun, Window, div, px,
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
    // ## m4b-t21-shots (named item; ruling (d), R-T2-5 d)
    /// The title drawn in two parts ([`TitleParts`]). `title` stays the tab's accessible name.
    /// `None`: one ellipsizing title, as before.
    pub parts: Option<TitleParts>,
}

// ## m4b-t21-shots fix round 2 (named item; N-I2, ruling R-T21-1)
/// A two-part tab title. The head (a script's file name) ellipsizes down to its first four
/// visible characters; the tail (" · connection") never ellipsizes. Only when the active tab
/// would still overflow the strip with its head at that floor does it show `tail_short` (the
/// connection shortened in the middle, both ends kept) instead. An inactive tab that does not
/// fit collapses to its icon and marks.
#[derive(Debug, Clone, PartialEq)]
pub struct TitleParts {
    pub head: SharedString,
    pub tail: SharedString,
    pub tail_short: Option<SharedString>,
}

/// The width each pane's tab strip had in the last frame (the room the next frame plans with).
// ## m4b-t22-pre (T21 r2): keyed by (window, pane); a closed window's widths go
// (`on_window_closed`, hooked on the first width), and a closed pane's when its workspace next
// draws ([`keep_strip_widths`]).
#[derive(Default)]
struct StripWidths {
    widths: std::collections::HashMap<(u64, usize), f32>,
    hooked: bool,
}

impl hacivat_ui_core::gpui_surface::Global for StripWidths {}

// ## m4b-t22-pre (named items, T21 r2)
/// Drops the recorded strip widths of `window`'s panes that are not in `live` (closed panes).
/// The pane tree's owner calls it as it draws.
pub fn keep_strip_widths(window: &Window, live: impl IntoIterator<Item = PaneId>, cx: &mut App) {
    if !cx.has_global::<StripWidths>() {
        return;
    }
    let win = window.window_handle().window_id().as_u64();
    let live: std::collections::HashSet<usize> = live.into_iter().map(|p| p.0 as usize).collect();
    cx.global_mut::<StripWidths>()
        .widths
        .retain(|(w, pane), _| *w != win || live.contains(pane));
}

/// How many strip widths are recorded (tests: none is kept for a closed window or pane).
pub fn strip_widths_held(cx: &App) -> usize {
    cx.try_global::<StripWidths>().map_or(0, |g| g.widths.len())
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

// ## m3c-narrow (F12)
/// A tab title's id (`#tab-title` in the text registry's paths).
pub const TAB_TITLE_ID: &str = "tab-title";
// ## m4b-t21-shots (named item)
/// The never-ellipsized tail of a two-part title (`#tab-title-conn`).
pub const TAB_TITLE_TAIL_ID: &str = "tab-title-conn";
/// How many visible characters of a two-part title's head always show (ruling (d)).
const HEAD_FLOOR_CHARS: usize = 4;
// ## m4b-t22-pre (T21 r3): one source for the tab's layout and the strip's plan.
/// A tab's padding, each side.
const TAB_PAD_X: f32 = 12.0;
/// The gap between a tab's children.
const TAB_GAP: f32 = 7.0;
/// A tab's right border.
const TAB_BORDER: f32 = 1.0;
/// The dirty dot.
const DIRTY_W: f32 = 6.0;
/// The tab's icon and its close button's icon.
const TAB_ICON: IconSize = IconSize::Chrome;
/// An inactive tab's flex shrink against the active tab's 1: in a strip too narrow for its tabs
/// the inactive tabs give their width first.
const INACTIVE_TAB_SHRINK: f32 = 1000.0;

// ## m4b-t21-shots (named items; ruling (d), R-T2-5 d)
/// The width `text` shapes to on one line in `style`.
fn shaped_width(text: &str, style: TypeStyle, ink: Hsla, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: hacivat_ui_core::fonts::font_for(&style),
        color: ink,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(
        window
            .text_system()
            .shape_line(
                SharedString::from(text.to_string()),
                px(style.size_px),
                &[run],
                None,
            )
            .width,
    )
}

/// A two-part title's measured widths: the whole head, its floor, the tail (and its short form).
#[derive(Debug, Clone, Copy)]
struct Measured {
    head: f32,
    floor: f32,
    tail: f32,
    tail_short: Option<f32>,
}

impl Measured {
    fn of(parts: &TitleParts, style: TypeStyle, ink: Hsla, window: &Window) -> Measured {
        let head = shaped_width(&parts.head, style, ink, window);
        let floor = crate::text::floor_prefix(&parts.head, HEAD_FLOOR_CHARS)
            .map_or(head, |f| shaped_width(&f, style, ink, window).min(head));
        // One pixel of slack: the label measures what it paints, rounding included.
        Measured {
            head,
            floor,
            tail: shaped_width(&parts.tail, style, ink, window) + 1.0,
            tail_short: parts
                .tail_short
                .as_ref()
                .map(|t| shaped_width(t, style, ink, window) + 1.0),
        }
    }
}

/// How the strip draws one tab this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fit {
    /// As built: a two-part title whose head gives way first, or one ellipsizing title.
    Title,
    /// An inactive two-part tab that does not fit: its icon and marks only.
    Collapsed,
    /// The active two-part tab that overflows even with its head at the floor: the short tail.
    Short,
}

/// The width of a tab's chrome: padding, icon, border, its marks and the gaps between its
/// children (`title`: whether a title is one of them). Read from the constants the tab is laid
/// out with (T21 r3).
fn chrome(tab: &TabSpec, active: bool, title: bool) -> f32 {
    let marks = [
        (tab.env.is_some(), crate::capsule::MARK_PX),
        (tab.dirty, DIRTY_W),
        (active, TAB_ICON.px()),
    ];
    let mut w = 2.0 * TAB_PAD_X + TAB_ICON.px() + TAB_BORDER;
    let mut children = 1;
    if title {
        children += 1;
    }
    for (shown, width) in marks {
        if shown {
            w += width;
            children += 1;
        }
    }
    w + TAB_GAP * (children - 1) as f32
}

/// A tab's width with its whole title: its chrome and its title (a two-part title with a
/// pixel of slack per part, as measured).
fn natural_width(tab: &TabSpec, active: bool, measured: Option<Measured>, title: f32) -> f32 {
    chrome(tab, active, true) + measured.map_or(title, |m| m.head + 1.0 + m.tail)
}

// ## m4b-t22-pre (named item, T21 r3)
/// The width the strip's plan gives `tab` drawn whole (`active`: with its close button), for
/// tests that hold it against the laid-out tab.
pub fn planned_tab_width(tab: &TabSpec, active: bool, window: &Window, cx: &App) -> f32 {
    let t = Theme::get(cx).tokens;
    let style = tab_style(tab.title_source, t);
    let ink = t.colors.text.to_hsla();
    let measured = tab
        .parts
        .as_ref()
        .map(|p| Measured::of(p, style, ink, window));
    natural_width(
        tab,
        active,
        measured,
        shaped_width(&tab.title, style, ink, window),
    )
}

/// The type style a tab title of `source` is drawn in.
fn tab_style(source: TextSource, t: &hacivat_ui_core::theme::ThemeTokens) -> TypeStyle {
    match source {
        TextSource::Data | TextSource::Catalog => TypeStyle {
            size_px: 12.0,
            line_height: 12.0 * 1.35,
            ..t.type_ramp.code
        },
        _ => t.type_ramp.body,
    }
}

/// Plans each tab's [`Fit`] for a strip `room` px wide (the last frame's; `None`: everything
/// as built). Ruling (d) and R-T21-1: the tabs fit whole, else the inactive tabs give way
/// (their file names first, by flex), else the inactive two-part tabs collapse to their icons,
/// and only then does the active tab show its short connection.
fn plan(
    tabs: &[TabSpec],
    active: usize,
    measured: &[Option<Measured>],
    titles: &[f32],
    room: Option<f32>,
) -> Vec<Fit> {
    let mut fits = vec![Fit::Title; tabs.len()];
    let Some(room) = room else {
        return fits;
    };
    let natural = |i: usize| natural_width(&tabs[i], i == active, measured[i], titles[i]);
    let min = |i: usize| {
        let t = &tabs[i];
        match measured[i] {
            Some(m) => chrome(t, i == active, true) + m.floor + m.tail,
            None if i == active => chrome(t, true, true) + titles[i],
            None => chrome(t, false, false),
        }
    };
    if (0..tabs.len()).map(natural).sum::<f32>() <= room {
        return fits;
    }
    let others_min: f32 = (0..tabs.len()).filter(|&i| i != active).map(min).sum();
    let active_min = if active < tabs.len() {
        min(active)
    } else {
        0.0
    };
    if active_min + others_min <= room {
        return fits;
    }
    let mut others = 0.0;
    for i in (0..tabs.len()).filter(|&i| i != active) {
        if measured[i].is_some() {
            fits[i] = Fit::Collapsed;
            others += chrome(&tabs[i], false, false);
        } else {
            others += min(i);
        }
    }
    if let Some(m) = measured.get(active).copied().flatten()
        && m.tail_short.is_some()
        && active_min + others > room
    {
        fits[active] = Fit::Short;
    }
    fits
}

/// A two-part tab title: `head` ellipsizes to its floor (its first [`HEAD_FLOOR_CHARS`]
/// visible characters), `tail` never ellipsizes. An inactive tab's parts sit in a wrapping row
/// behind a zero-width first item, so if the plan was wrong and the tab is narrower than the
/// floor plus the tail, the parts wrap onto a second line that the tab clips whole: never a
/// cut connection name.
fn two_part_title(
    head: SharedString,
    tail: SharedString,
    m: Measured,
    source: TextSource,
    style: TypeStyle,
    ink: Hsla,
    active: bool,
) -> AnyElement {
    let natural = m.head + 1.0 + m.tail;
    let min = m.floor + m.tail;
    let parts = div()
        .flex()
        .flex_row()
        .items_center()
        .min_w(px(min))
        .max_w(px(natural))
        .child(
            div()
                .flex_shrink(1.0)
                .min_w(px(m.floor))
                .overflow_hidden()
                .child(
                    Label::new(head, source, style, ink)
                        .truncate(true)
                        .id(TAB_TITLE_ID),
                ),
        )
        .child(
            div()
                .flex_none()
                .child(Label::new(tail, source, style, ink).id(TAB_TITLE_TAIL_ID)),
        );
    if active {
        // The other tabs give their width first (they shrink 1000 to 1).
        return parts.flex_shrink(1.0).into_any_element();
    }
    let line = px(style.line_height.ceil() + 2.0);
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .h(line)
        .min_w(px(0.0))
        .flex_shrink(1.0)
        .overflow_hidden()
        .child(div().w(px(0.0)).h(line))
        .child(parts.flex_basis(px(min)).flex_grow(1.0))
        .into_any_element()
}

/// Records the strip's width for the next frame's plan; a changed width draws the view that
/// holds the strip once more.
///
/// ## m4b-t22-pre (T21 r2): that view only (a notify), not every window in full: the cached
/// views in it (the editor) stay cached.
fn width_probe(key: (u64, usize)) -> impl IntoElement {
    div().absolute().top_0().left_0().size_full().child(
        hacivat_ui_core::gpui_surface::canvas(
            move |bounds, window, cx| {
                let w = f32::from(bounds.size.width);
                let g = cx.default_global::<StripWidths>();
                let old = g.widths.insert(key, w);
                if !std::mem::replace(&mut g.hooked, true) {
                    cx.on_window_closed(|cx, id| {
                        if cx.has_global::<StripWidths>() {
                            let id = id.as_u64();
                            cx.global_mut::<StripWidths>()
                                .widths
                                .retain(|(w, _), _| *w != id);
                        }
                    })
                    .detach();
                }
                if old.is_none_or(|o| (o - w).abs() > 0.5) {
                    let view = window.current_view();
                    cx.defer(move |cx| cx.notify(view));
                }
            },
            |_, _, _, _| {},
        )
        .size_full(),
    )
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
            // ## m3c-narrow (F12): was `flex_none`; a strip wider than its pane shrinks to it,
            // so its tabs give way (below).
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .bg(c.head.to_hsla());
        // ## m4b-t21-shots fix round 2 (N-I2, m-d): the plan, from the last frame's width.
        let style_of = |source: TextSource| tab_style(source, t);
        let has_parts = self.tabs.iter().any(|t| t.parts.is_some());
        // Keyed by window and pane: two windows' strips never share a width.
        let strip_key = (window.window_handle().window_id().as_u64(), pane.0 as usize);
        let fits = if has_parts {
            let ink = c.text.to_hsla();
            let measured: Vec<Option<Measured>> = self
                .tabs
                .iter()
                .map(|t| {
                    t.parts
                        .as_ref()
                        .map(|p| Measured::of(p, style_of(t.title_source), ink, window))
                })
                .collect();
            let titles: Vec<f32> = self
                .tabs
                .iter()
                .map(|t| shaped_width(&t.title, style_of(t.title_source), ink, window))
                .collect();
            let room = cx
                .try_global::<StripWidths>()
                .and_then(|g| g.widths.get(&strip_key).copied());
            let fits = plan(&self.tabs, self.active, &measured, &titles, room);
            fits.into_iter().zip(measured).collect()
        } else {
            vec![(Fit::Title, None); self.tabs.len()]
        };
        for ((index, tab), (fit, measured)) in self.tabs.into_iter().enumerate().zip(fits) {
            let active = index == self.active;
            let id = ElementId::from((SharedString::from("tab"), tab.id.0 as usize));
            let (hover, _) = interaction_state(&id, window, cx);
            let hovered = *hover.read(cx);
            let ink = if active { c.text } else { c.text_2 }.to_hsla();
            let style = tab_style(tab.title_source, t);
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
                // ## m3c-narrow (F12): was `flex_none`: the tab gives way (below).
                .min_w(px(0.0))
                // ## m4b-t22-pre (T21 r3): the constants the plan reads.
                .px(px(TAB_PAD_X))
                .gap(px(TAB_GAP))
                .border_r(px(TAB_BORDER))
                .border_color(c.seam.to_hsla())
                .cursor_pointer()
                .child(Icon::new(IconName::FileCode).size(TAB_ICON).color(ink))
                // ## m3c-narrow (F12): the title ellipsizes (its tooltip and the tab's
                // accessible name keep it whole) under a stable id.
                // ## m4b-t21-shots (named exception): or two parts (`TabSpec::parts`), as the
                // strip's plan draws them (fix round 2).
                .children(match (tab.parts, measured) {
                    (Some(parts), Some(m)) => match fit {
                        Fit::Collapsed => None,
                        Fit::Short => {
                            let m = Measured {
                                tail: m.tail_short.unwrap_or(m.tail),
                                ..m
                            };
                            let tail = parts.tail_short.unwrap_or(parts.tail);
                            Some(two_part_title(
                                parts.head,
                                tail,
                                m,
                                tab.title_source,
                                style,
                                ink,
                                active,
                            ))
                        }
                        Fit::Title => Some(two_part_title(
                            parts.head,
                            parts.tail,
                            m,
                            tab.title_source,
                            style,
                            ink,
                            active,
                        )),
                    },
                    _ => Some(
                        Label::new(tab.title, tab.title_source, style, ink)
                            .truncate(true)
                            .id(TAB_TITLE_ID)
                            .into_any_element(),
                    ),
                });
            // ## m3c-narrow (F12; design/m3b-safe surface 24): a strip narrower than its tabs
            // (a pane at the split floor) is a declared give-way: the inactive tabs shrink first,
            // their titles ellipsizing, down to nothing (each clips what it no longer has room
            // for; Ctrl+Tab still reaches them), then the active tab's title ellipsizes. A strip
            // with room draws every tab at its natural width.
            el = if active {
                el.flex_shrink(1.0)
            } else if fit == Fit::Collapsed {
                // ## m4b-t21-shots fix round 2 (m-d): its icon and marks, no empty width.
                el.flex_none()
            } else {
                el.flex_shrink(INACTIVE_TAB_SHRINK).overflow_hidden()
            };
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
                        .size(px(DIRTY_W))
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
                            // Raised Ink Rule: the active tab is raised, so its × is text-2;
                            // an inactive tab (head, or hover) keeps text-3.
                            Icon::new(IconName::X)
                                .size(TAB_ICON)
                                .color(if active { c.text_2 } else { c.text_3 }.to_hsla()),
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
        // ## m4b-t21-shots fix round 2 (N-I2): the strip in a row as wide as the header, which
        // records its width (only where a two-part title plans with it).
        div()
            .flex()
            .flex_row()
            .flex_1()
            .min_w(px(0.0))
            .h(height)
            .relative()
            .children(has_parts.then(|| width_probe(strip_key)))
            .child(strip)
            .into_any_element()
    }
}
