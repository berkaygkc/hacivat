//! `ExplorerModel`: the explorer's [`TreeModel`] over catalog snapshots (Task 14): profiles →
//! contributed nodes → databases → schemas → kind groups → objects → child groups → leaves.
//! `TreeKey`s are derived from `ObjectKey` with a domain-separated XXH3-128, never from a
//! display string.
//!
//! ## m4-t01b-seams: stub. The model has no roots and knows no key, so a tree over it is empty;
//! an unknown key's children answer `NotLoaded` (never a `Leaf` it cannot know). Nothing shows
//! it until Task 14 wires the tree.

use std::sync::Arc;

use hacivat_ui::tree::{ChildState, TreeKey, TreeModel, TreeRowData};

/// The explorer's tree model. Stub (Task 14 fills it): empty.
#[derive(Debug, Clone, Default)]
pub struct ExplorerModel {
    generation: u64,
}

impl TreeModel for ExplorerModel {
    fn roots(&self) -> Arc<[TreeKey]> {
        Arc::from([])
    }

    fn row(&self, _key: TreeKey) -> Option<TreeRowData> {
        None
    }

    fn children(&self, _key: TreeKey) -> ChildState {
        ChildState::NotLoaded
    }

    fn generation(&self) -> u64 {
        self.generation
    }
}
