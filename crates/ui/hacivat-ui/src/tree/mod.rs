//! A virtualized tree (plan-m4 Task 8): the explorer's widget, with no database knowledge
//! (spec §5.1). The host describes the tree through a [`TreeModel`] keyed by opaque
//! [`TreeKey`]s; the widget owns expansion, selection, scrolling, keyboard and accessibility.
//!
//! - **Model.** [`TreeModel::roots`], [`TreeModel::children`] (lazy: [`ChildState`] says
//!   whether children are unknown, loading, loaded or failed) and [`TreeModel::row`] (what a
//!   row shows). The model bumps [`TreeModel::generation`] whenever any answer changes and the
//!   host calls [`TreeView::model_changed`]; the view re-flattens before the next frame.
//! - **Flattening** ([`flat`]): the expanded tree is one `Vec` of 32-byte rows, rebuilt on a
//!   generation change and spliced on expand/collapse. A frame only reads the rows on screen.
//! - **Expansion** is a set of keys owned by the view, so it survives model generations, and a
//!   key that vanishes and returns comes back expanded.
//! - **Scroll anchoring.** The first visible row keeps its screen position when rows are
//!   inserted or removed above it (located again by key after every rebuild).
//! - **Group headers** (R-M4-49) take the keyboard cursor when they can expand, never the
//!   selection; a header that cannot expand (an environment label) is not a cursor stop.
//! - **Events** ([`TreeEvent`]): expand/collapse (an expanded node whose children are
//!   `NotLoaded` is announced once per expansion so the host loads it), activate (Enter,
//!   double click), context menu (right click, Shift-F10, the menu key) and selection.
//! - **Accessibility** ([`a11y`]): while assistive technology is active, a `Tree` node whose
//!   synthetic `TreeItem` children (rows on screen ± overscan) carry level, expanded state,
//!   set size and position; nothing is built otherwise.

// ## m4-t08-tree: the bodies of `m4-spike-tree` @ `2a31b8b` (R-M4-40), on Task 1b's seam.
pub mod a11y;
pub mod flat;
pub mod typeahead;
mod view;

use std::collections::HashSet;
use std::hash::BuildHasherDefault;
use std::sync::Arc;

use hacivat_ui_core::gpui_surface::{Pixels, Point, SharedString};

use crate::capsule::EnvKind;
use crate::icon::IconName;
use crate::text::TextSource;

pub use typeahead::Fold;
pub use view::{TREE_OVERSCAN_ROWS, TreeView};

/// A node's identity: opaque to the widget. The app derives it from its own object identity
/// with a stable hash (`ObjectKey`, Task 14), never from a display string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TreeKey(pub u128);

/// A set of keys (expansion, selection), hashed with [`flat::KeyHasher`]: the keys are already
/// hashes. Build one with `KeySet::default()` (or `collect`).
pub type KeySet = HashSet<TreeKey, BuildHasherDefault<flat::KeyHasher>>;

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

/// The kind of a row's trailing status glyph (git-like state of a contributed node, M4b).
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
/// and its text alternative, which assistive technology reads. Both come from Fluent through
/// the app; the text alternative is a required field, so no status is drawn without one.
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
    /// The kind icon (12 px; blue while selected). None on loading rows and group headers.
    pub icon: Option<IconName>,
    /// Right-aligned mono meta (a preformatted count, an engine, a column type).
    pub meta: Option<SharedString>,
    /// `smallvec` is not a `hacivat-ui` dependency; an empty `Vec` does not allocate.
    pub flags: Vec<RowFlag>,
    /// An outlined badge after the flags ("out of date", "version 17").
    pub badge: Option<SharedString>,
    /// The status slot (M4b git badges).
    pub status: Option<RowStatus>,
    /// A group header (small caps, a seam rule and a mono count): takes the keyboard focus but
    /// never the selection (R-M4-49). A header that cannot expand (no children) is a plain
    /// label the keys pass over.
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
/// press, so each must answer from memory: `row` runs per visible row per frame and answers
/// from the snapshot without I/O (the host loads in the background and bumps the generation).
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
    /// starts loading them; emitted once per expansion (also for a node restored expanded, or
    /// revealed by an ancestor's expansion, whose children were not loaded yet, and again when
    /// the host drops an expanded node's children back to `NotLoaded`).
    Expand(TreeKey),
    Collapse(TreeKey),
    /// Enter or a double click on a row that is not a group header (Enter on a header toggles
    /// it instead).
    Activate(TreeKey),
    /// Right click, Shift-F10 or the menu key: open the menu at this window position. A group
    /// header asks too (its kind-group menu), without becoming the selection.
    ContextMenu(TreeKey, Point<Pixels>),
    /// The selected row changed (`None`: nothing is selected). Never names a group header.
    SelectionChanged(Option<TreeKey>),
}
