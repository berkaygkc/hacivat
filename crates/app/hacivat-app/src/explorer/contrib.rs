//! Contributed explorer nodes (R-M4-19; M4b's Scripts node, L14): a contributor inserts nodes
//! directly under a database node, above its schemas, with lazy children, activation, a context
//! menu, filter text and a generation.
//!
//! ## m4-t01b-seams: the seam. Task 14 places the nodes in the tree and joins them to the
//! filter; M4 registers no contributor. A contributed node's [`TreeKey`] is domain-separated by
//! its owner (M4b hashes `"hv-scripts\0"` first), so it never equals an object's key.
//!
//! ## m4-t14-explorer: placement (`explorer::adapter`). A PostgreSQL database row lists its
//! contributed nodes first, above its schemas; a MySQL/MariaDB database (a schema) lists them
//! above its kind groups; a SQLite profile (one file) lists them under its root, above `main`,
//! asked for with the database name `main`. A [`ContributedChildren::Lazy`] node's children are
//! asked for when the node is expanded (on the UI thread, then kept until the contributor's
//! generation moves); their rows come from [`ExplorerContributor::child_node`]. The filter
//! matches [`ContributedNode::filter_text`] of every node it knows (loaded names only, as for
//! catalog objects).
//!
//! ## m4b-t01b-seams (R-M4b-34): the explorer calls the owner of a contributed node: Enter or a
//! double click on a node that does not expand calls its [`ExplorerContributor::activate`] (an
//! expandable node still toggles), its menu is its [`ExplorerContributor::context_menu`], and
//! [`contributors_changed`] redraws the explorer. A node's owner is the contributor that listed
//! it ([`ExplorerContributor::database_children`], or the owner of the node it was listed
//! under); no other contributor is ever asked about its key.

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::tree::{ChildState, TreeKey, TreeRowData};
use hacivat_ui_core::gpui_surface::{App, Global, Window};

/// A contributed node's children.
#[derive(Clone)]
pub enum ContributedChildren {
    /// No children (no chevron).
    Leaf,
    /// Asked for when the node is expanded (on the UI thread: answers from memory).
    Lazy(Arc<dyn Fn(&App) -> ChildState>),
}

impl fmt::Debug for ContributedChildren {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContributedChildren::Leaf => f.write_str("Leaf"),
            ContributedChildren::Lazy(_) => f.write_str("Lazy(..)"),
        }
    }
}

/// One node a contributor puts under a database node.
#[derive(Debug, Clone)]
pub struct ContributedNode {
    /// Domain-separated by the contributor (never derived from a display string).
    pub key: TreeKey,
    pub row: TreeRowData,
    pub children: ContributedChildren,
    /// The text the explorer filter matches (M4b: the file name); `None` never matches.
    pub filter_text: Option<String>,
}

/// Adds nodes to the explorer (M4b's Scripts; R-M4-19).
pub trait ExplorerContributor: 'static {
    /// The nodes under `database` of `profile`, above its schemas, in display order.
    fn database_children(
        &self,
        profile: ProfileId,
        database: &str,
        cx: &App,
    ) -> Vec<ContributedNode>;
    /// The children of one of this contributor's nodes.
    fn children(&self, key: TreeKey, cx: &App) -> ChildState;
    // ## m4-t14-explorer (additive): the rows of lazily listed children.
    /// One of this contributor's nodes listed by [`ContributedChildren::Lazy`] or
    /// [`ExplorerContributor::children`]: its row, children and filter text. A key the
    /// contributor does not describe (`None`, the default) is not drawn.
    fn child_node(&self, key: TreeKey, cx: &App) -> Option<ContributedNode> {
        let _ = (key, cx);
        None
    }
    /// Enter or a double click on one of this contributor's nodes.
    fn activate(&self, key: TreeKey, window: &mut Window, cx: &mut App);
    /// The context menu of one of this contributor's nodes.
    fn context_menu(&self, key: TreeKey, cx: &App) -> Vec<MenuItemSpec>;
    /// Changes whenever any answer above changes (the tree re-reads the nodes).
    fn generation(&self) -> u64;
}

/// The registered contributors, in registration order.
// ## m4b-t01b-seams (named exception, R-M4b-34 (3)): public so the explorer observes it; its
// list stays private.
// ## m4b-t13-explorer (named exception, T1b review m7): crate-visible only.
#[derive(Default, Clone)]
pub(crate) struct Contributors(Vec<Rc<dyn ExplorerContributor>>);

impl Global for Contributors {}

// ## m4b-t01b-seams (named lines, R-M4b-34 (3); C-M4-7)
/// A contributor's answers changed (its [`ExplorerContributor::generation`] moved): the
/// explorer re-reads its nodes and redraws, with no other event (M4b's Task 13 calls it once
/// per coalesced batch of library events).
pub fn contributors_changed(cx: &mut App) {
    // Touching the global notifies its observers (`ExplorerView::new` observes it).
    cx.default_global::<Contributors>();
}

/// Registers a contributor (M4b's Task 1; M4 registers none).
pub fn register_contributor(contributor: Rc<dyn ExplorerContributor>, cx: &mut App) {
    cx.default_global::<Contributors>().0.push(contributor);
}

/// The registered contributors, in registration order.
pub fn contributors(cx: &App) -> Vec<Rc<dyn ExplorerContributor>> {
    cx.try_global::<Contributors>()
        .map(|c| c.0.clone())
        .unwrap_or_default()
}
