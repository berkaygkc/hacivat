//! The Scripts node in the explorer (HV-SCRIPT-05, R-M4b-19, R-M4b-25, R-M4b-26; NOTES surface
//! 1): an [`ExplorerContributor`] that puts a "Scripts" node under every database node a bound
//! library shows under, above its schemas, with the library's folders and files below it.
//!
//! ## m4b-t01b-seams: the stub contributes no node and answers generation 0, so every M4 tree
//! and shot is unchanged. It is not registered: Task 13 fills it (keys scoped by profile and
//! database, R-M4b-26; the PG/MySQL/SQLite placement rules, R-M4b-25, R-M4b-31, R-M4b-32; git
//! badges, R-M4b-11) and registers it, and calls
//! [`contributors_changed`](crate::explorer::contrib::contributors_changed) once per coalesced
//! library event batch (C-M4-7).

use hacivat_proto::ProfileId;
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::tree::{ChildState, TreeKey};
use hacivat_ui_core::gpui_surface::{App, Window};

use crate::explorer::contrib::{ContributedNode, ExplorerContributor};

/// The Scripts contributor (stub: no node).
#[derive(Debug, Default)]
pub struct ScriptsContributor;

impl ExplorerContributor for ScriptsContributor {
    fn database_children(
        &self,
        _profile: ProfileId,
        _database: &str,
        _cx: &App,
    ) -> Vec<ContributedNode> {
        Vec::new()
    }

    fn children(&self, _key: TreeKey, _cx: &App) -> ChildState {
        ChildState::Leaf
    }

    fn child_node(&self, _key: TreeKey, _cx: &App) -> Option<ContributedNode> {
        None
    }

    /// Opens nothing (Task 14's `open_script` is the filled path).
    fn activate(&self, _key: TreeKey, _window: &mut Window, _cx: &mut App) {}

    fn context_menu(&self, _key: TreeKey, _cx: &App) -> Vec<MenuItemSpec> {
        Vec::new()
    }

    fn generation(&self) -> u64 {
        0
    }
}
