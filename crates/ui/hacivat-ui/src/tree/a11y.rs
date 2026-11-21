//! The tree's AccessKit items (the M3c T40 pattern, `hacivat-ui-grid` `element/a11y_rows.rs`).
//!
//! - **Zero cost without assistive technology.** While `Window::is_a11y_active` is false the
//!   view builds no item, no label and no string ([`TreeView::a11y_builds`] stays 0).
//! - **The subtree.** The view's root is a `Tree` node; its synthetic children are one
//!   `TreeItem` per row on screen ± [`A11Y_OVERSCAN_ROWS`] ([`visible_items`], pure), with
//!   level, expanded state, set size and position, selection, and bounds. The cursor row is
//!   the tree's active descendant (a group header too: it takes the cursor, R-M4-49). Group
//!   headers are items without a selection state; a loading or failed child row, and a header
//!   that cannot expand, are items named by their text that take no action.
//! - **Ids** come from the row's key and kind, so an item keeps its node across a scroll, a
//!   reorder and a model generation.
//! - **Actions** (Focus, Click, Expand, Collapse, ScrollIntoView, ShowContextMenu) act on the
//!   item's node as the keyboard would (Click activates a row and toggles a group header).
//!
//! [`TreeView::a11y_builds`]: super::TreeView::a11y_builds

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{Role, SharedString, accesskit};

use super::KeySet;
use super::flat::{Flat, RowKind};
use super::{ChildState, TreeKey, TreeModel, TreeRowData};

/// Rows built above and below the ones on screen.
pub const A11Y_OVERSCAN_ROWS: usize = 5;

/// The actions an item that takes the cursor answers.
pub const ITEM_ACTIONS: [accesskit::Action; 6] = [
    accesskit::Action::Focus,
    accesskit::Action::Click,
    accesskit::Action::Expand,
    accesskit::Action::Collapse,
    accesskit::Action::ScrollIntoView,
    accesskit::Action::ShowContextMenu,
];

/// One tree item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemNode {
    pub key: TreeKey,
    pub kind: RowKind,
    /// Its row in the flat list.
    pub row: usize,
    /// 1 for a root (ARIA levels are 1-based).
    pub level: usize,
    /// `Some` for a node that can expand.
    pub expanded: Option<bool>,
    pub position: usize,
    pub set_size: usize,
    /// `None` for rows that are never selected (group headers, loading and failed rows).
    pub selected: Option<bool>,
    /// Whether it can take the cursor (and so answers [`ITEM_ACTIONS`]): a row that is not a
    /// group header, or a group header that can expand (R-M4-49).
    pub focusable: bool,
    /// The cursor row (the tree's active descendant).
    pub focused: bool,
    pub name: SharedString,
    /// The status slot's text alternative.
    pub description: Option<SharedString>,
}

impl ItemNode {
    /// The item's AccessKit node (without bounds).
    pub fn node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(Role::TreeItem);
        node.set_label(self.name.to_string());
        node.set_level(self.level);
        node.set_position_in_set(self.position);
        node.set_size_of_set(self.set_size);
        if let Some(expanded) = self.expanded {
            node.set_expanded(expanded);
        }
        if let Some(selected) = self.selected {
            node.set_selected(selected);
        }
        if self.focusable {
            for action in ITEM_ACTIONS {
                node.add_action(action);
            }
        }
        if let Some(d) = &self.description {
            node.set_description(d.to_string());
        }
        node
    }

    /// The key its synthetic node id is derived from.
    pub fn id_key(&self) -> (u128, u8) {
        (self.key.0, self.kind as u8)
    }
}

/// The description of a node's item (fix round 1, R-W1-1): the status text, the host's
/// description (the environment, read-only, the badge), the row's tooltip and the pill's
/// tooltip, in that order, as one list.
fn describe(d: TreeRowData) -> Option<SharedString> {
    let parts: Vec<SharedString> = [
        d.status.map(|s| s.text),
        d.a11y_description,
        d.tooltip,
        d.pill.map(|p| p.tooltip),
    ]
    .into_iter()
    .flatten()
    .filter(|p| !p.is_empty())
    .collect();
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => Some(parts.join(", ").into()),
    }
}

/// What the items read from the view.
#[derive(Debug, Clone)]
pub struct ItemsIn<'a> {
    pub flat: &'a Flat,
    /// The rows on screen.
    pub visible: Range<usize>,
    pub overscan: usize,
    pub expanded: &'a KeySet,
    pub selection: &'a KeySet,
    pub cursor: Option<TreeKey>,
}

/// The items for the rows on screen ± the overscan (pure): see the module docs.
pub fn visible_items<M: TreeModel + ?Sized>(model: &M, input: ItemsIn<'_>) -> Vec<ItemNode> {
    let rows = input.flat.rows();
    let start = input.visible.start.saturating_sub(input.overscan);
    let end = input
        .visible
        .end
        .saturating_add(input.overscan)
        .min(rows.len());
    let mut out = Vec::with_capacity(end.saturating_sub(start));
    for (ix, r) in rows.iter().enumerate().take(end).skip(start) {
        let base = |name: SharedString| ItemNode {
            key: r.key,
            kind: r.kind,
            row: ix,
            level: usize::from(r.depth) + 1,
            expanded: None,
            position: r.pos as usize,
            set_size: r.set_size as usize,
            selected: None,
            focusable: false,
            focused: false,
            name,
            description: None,
        };
        let item = match r.kind {
            RowKind::Loading | RowKind::Failed => match model.children(r.key) {
                ChildState::Loading { label, count, .. } => {
                    let mut item = base(label);
                    item.description = count;
                    item
                }
                ChildState::Failed { label, retry } => {
                    let mut item = base(label);
                    item.description = retry;
                    item
                }
                _ => base(SharedString::default()),
            },
            RowKind::Node => {
                let data = model.row(r.key);
                let header = data.as_ref().is_some_and(|d| d.group_header);
                let expandable = model.children(r.key).expandable();
                let mut item = base(data.as_ref().map(|d| d.label.clone()).unwrap_or_default());
                item.expanded = expandable.then(|| input.expanded.contains(&r.key));
                item.focusable = data.is_some() && (!header || expandable);
                if data.is_some() && !header {
                    item.selected = Some(input.selection.contains(&r.key));
                }
                item.focused = item.focusable && input.cursor == Some(r.key);
                item.description = data.and_then(describe);
                item
            }
        };
        out.push(item);
    }
    out
}
