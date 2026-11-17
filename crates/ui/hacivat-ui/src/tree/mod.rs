//! A virtualized tree (M4 spike, plan-m4 Task 8): the explorer's widget, with no database
//! knowledge (spec §5.1). The host describes the tree through a [`TreeModel`] keyed by opaque
//! [`TreeKey`]s; the widget owns expansion, selection, scrolling, keyboard and accessibility.
//!
//! - **Model.** [`TreeModel::roots`], [`TreeModel::children`] (lazy: [`ChildState`] says
//!   whether children are unknown, loading, loaded or failed) and [`TreeModel::row`] (what a
//!   row shows). The model bumps [`TreeModel::generation`] whenever any answer changes and the
//!   host calls [`TreeView::model_changed`]; the next frame re-flattens.
//! - **Flattening** ([`flat`]): the expanded tree is one `Vec` of 32-byte rows, rebuilt on a
//!   generation change and spliced on expand/collapse. A frame only reads the rows on screen.
//! - **Expansion** is a set of keys owned by the view, so it survives model generations, and a
//!   key that vanishes and returns comes back expanded.
//! - **Scroll anchoring.** The first visible row keeps its screen position when rows are
//!   inserted or removed above it (located again by key after every rebuild).
//! - **Events** ([`TreeEvent`]): expand/collapse (an expanded node whose children are
//!   `NotLoaded` is announced once per expansion so the host loads it), activate (Enter,
//!   double click), context menu (right click, Shift-F10, the menu key) and selection.
//! - **Accessibility** ([`a11y`]): while assistive technology is active, a `Tree` node whose
//!   synthetic `TreeItem` children (rows on screen ± overscan) carry level, expanded state,
//!   set size and position; nothing is built otherwise.

pub mod a11y;
pub mod flat;
pub mod typeahead;
mod view;

use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{Pixels, Point, SharedString};

use crate::capsule::EnvKind;
use crate::icon::IconName;
use crate::text::TextSource;

pub use view::{TREE_OVERSCAN_ROWS, TreeView};

/// A node's identity: opaque to the widget (the app derives it from its own object identity
/// with a stable hash, never from a display string).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeKey(pub u128);

/// What a node's children are.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildState {
    /// The node has no children and never will (no chevron).
    Leaf,
    /// The node may have children nobody asked for yet (a chevron; expanding it emits
    /// [`TreeEvent::Expand`] so the host loads them).
    NotLoaded,
    /// The host is loading them: one child row shows `label` (a rendered message).
    Loading { label: SharedString },
    /// The children, in display order. An empty list draws no chevron.
    Loaded(Arc<[TreeKey]>),
    /// Loading failed: one child row shows `label` (a rendered message).
    Failed { label: SharedString },
}

impl ChildState {
    /// Whether the node draws a chevron.
    pub fn expandable(&self) -> bool {
        match self {
            ChildState::Leaf => false,
            ChildState::Loaded(keys) => !keys.is_empty(),
            _ => true,
        }
    }
}

/// The kind of a row's trailing status glyph (git-like state of a contributed node, M4b C-M4).
/// Never red (Hue Is Meaning: red is production and destructive only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusKind {
    Modified,
    New,
    Conflicted,
    Unknown,
    ContainsChanges,
}

/// A row's status slot: a short glyph (a rendered message, e.g. `M`) drawn after the label,
/// and its text alternative, which assistive technology reads.
#[derive(Debug, Clone, PartialEq)]
pub struct RowStatus {
    pub kind: StatusKind,
    pub glyph: SharedString,
    pub text: SharedString,
}

/// A flag drawn after the label (DESIGN.md Explorer Tree: environment swatch, lock/path flags).
#[derive(Debug, Clone, PartialEq)]
pub enum RowFlag {
    /// The 7 px environment mark.
    Env(EnvKind),
    /// Read-only: red only on production (Hue Is Meaning), `text-2` otherwise.
    Lock { production: bool },
    /// Any other 12 px icon flag, with its text alternative.
    Icon { icon: IconName, text: SharedString },
}

/// What one row shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRowData {
    /// The name, verbatim.
    pub label: SharedString,
    /// Where the label comes from (catalog names are [`TextSource::Catalog`]).
    pub label_source: TextSource,
    /// The kind icon (12 px; blue while selected).
    pub icon: Option<IconName>,
    /// Right-aligned mono meta (a preformatted count, an engine, a column type).
    pub meta: Option<SharedString>,
    pub flags: Vec<RowFlag>,
    /// An outlined badge after the flags ("Out of date", "Built-in catalog (version 17)").
    pub badge: Option<SharedString>,
    /// The status slot (M4b git badges).
    pub status: Option<RowStatus>,
    /// A group header (small caps, a seam rule and a mono count): never selected, skipped by
    /// the arrow keys, still expandable with a click.
    pub group_header: bool,
    /// Greyed (out of scope): `text-3` ink.
    pub dim: bool,
}

impl TreeRowData {
    /// A plain row: `label` from `source`, nothing else.
    pub fn new(label: impl Into<SharedString>, source: TextSource) -> Self {
        TreeRowData {
            label: label.into(),
            label_source: source,
            icon: None,
            meta: None,
            flags: Vec::new(),
            badge: None,
            status: None,
            group_header: false,
            dim: false,
        }
    }
}

/// The tree's data source. Every method is called on the UI thread during a frame or a key
/// press, so each must answer from memory (the host loads in the background and bumps the
/// generation).
pub trait TreeModel: 'static {
    /// The top-level nodes, in display order.
    fn roots(&self) -> Arc<[TreeKey]>;
    /// What `key`'s row shows; `None` for a key the model no longer knows (the row is skipped).
    fn row(&self, key: TreeKey) -> Option<TreeRowData>;
    /// `key`'s children.
    fn children(&self, key: TreeKey) -> ChildState;
    /// Changes whenever any answer above changes.
    fn generation(&self) -> u64;
    /// The text type-ahead matches (the label by default).
    fn search_text(&self, key: TreeKey) -> Option<SharedString> {
        self.row(key).map(|r| r.label)
    }
}

/// What the tree tells its host.
#[derive(Debug, Clone, PartialEq)]
pub enum TreeEvent {
    /// The node is now expanded. When its children are [`ChildState::NotLoaded`] the host
    /// starts loading them; this is emitted once per expansion (also for a node restored
    /// expanded whose children were not loaded yet).
    Expand(TreeKey),
    Collapse(TreeKey),
    /// Enter or a double click.
    Activate(TreeKey),
    /// Right click, Shift-F10 or the menu key: open the menu at this window position.
    ContextMenu(TreeKey, Point<Pixels>),
    /// The focused row (the cursor) changed.
    SelectionChanged(Option<TreeKey>),
}
