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

// ## m4b-t14-tabs (named exception, controller note): a thin accessor behind Task 13's names
// (`service`, `ensure_service`, `set_service`), so the script tabs use the app's one library
// service; Task 13's own bodies replace these at the merge.

/// The app's one library service.
#[derive(Clone)]
struct ServiceSlot(std::sync::Arc<hacivat_scripts::service::LibraryService>);

impl hacivat_ui_core::gpui_surface::Global for ServiceSlot {}

/// The library service, when one is open.
pub fn service(cx: &App) -> Option<std::sync::Arc<hacivat_scripts::service::LibraryService>> {
    cx.try_global::<ServiceSlot>().map(|s| s.0.clone())
}

/// The library service, opened now if it is not open yet. `None` without settings or a
/// connections model.
pub fn ensure_service(
    cx: &mut App,
) -> Option<std::sync::Arc<hacivat_scripts::service::LibraryService>> {
    if let Some(s) = service(cx) {
        return Some(s);
    }
    let paths = cx.try_global::<crate::shell::AppState>()?.paths.clone();
    let rt = cx
        .try_global::<crate::connections::ConnectionsModel>()?
        .runtime();
    let svc = std::sync::Arc::new(hacivat_scripts::service::LibraryService::open(&paths, &rt));
    set_service(svc.clone(), cx);
    Some(svc)
}

/// Makes `service` the app's library service (tests pass one over fake event sources).
pub fn set_service(
    service: std::sync::Arc<hacivat_scripts::service::LibraryService>,
    cx: &mut App,
) {
    cx.set_global(ServiceSlot(service));
}
// ## m4b-t14-tabs
