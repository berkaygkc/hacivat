//! Explorer actions (mock 5): the context menus, Open data (R-M4-13), View DDL, copy name and
//! qualified name, refresh, hide objects like this, and the actions later milestones provide.
//!
//! ## m4-t01b-seams: the seams. Task 24 builds the menus, renders provided actions in slot order
//! and installs the default open-data handler. M4 registers no action provider (R-M4-19).

use std::fmt;
use std::rc::Rc;

use hacivat_meta::{ObjectKey, ObjectKind};
use hacivat_proto::ProfileId;
use hacivat_ui::tree::TreeKey;
use hacivat_ui_core::gpui_surface::{App, Global, SharedString, Window};

/// Where provided actions are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionSurface {
    /// The explorer's context menu.
    ExplorerMenu,
    /// The DDL view tab's toolbar (M7's "Edit source").
    DdlViewToolbar,
}

/// The menu group a provided action joins, in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ActionSlot {
    /// Open data and its kin (M6's table editor).
    Data,
    /// Export and import (M6).
    Transfer,
    /// Modify… (M7; the disabled placeholder yields to a provider that fills it, PO-Q13).
    Modify,
    /// Edit source (M7).
    Source,
}

/// One action a provider adds.
#[derive(Debug, Clone, PartialEq)]
pub struct ProvidedAction {
    pub slot: ActionSlot,
    /// The registered command it runs (`ActionRegistry`).
    pub id: &'static str,
    /// The rendered label.
    pub label: SharedString,
    pub enabled: bool,
}

/// The node an action is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRef {
    pub profile: ProfileId,
    /// The node's tree key (an object's, or a contributed node's).
    pub key: TreeKey,
    /// The catalog object the node shows; `None` for a profile, database, kind group or
    /// contributed node.
    pub object: Option<ObjectKey>,
    /// The node's kind: the object's (`object.kind`), or the kind a kind-group header holds
    /// ("Tables"); `None` otherwise. Providers match on it (Task 24's slot test).
    pub kind: Option<ObjectKind>,
}

/// Adds actions to the explorer menu and the DDL view toolbar (M6, M7; R-M4-19).
pub trait ExplorerActionProvider: 'static {
    fn actions(&self, node: &NodeRef, surface: ActionSurface, cx: &App) -> Vec<ProvidedAction>;
}

/// The registered providers, in registration order.
#[derive(Default, Clone)]
struct Providers(Vec<Rc<dyn ExplorerActionProvider>>);

impl Global for Providers {}

impl fmt::Debug for Providers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Providers").field(&self.0.len()).finish()
    }
}

/// Registers a provider (M6, M7; M4 registers none).
pub fn register_action_provider(provider: Rc<dyn ExplorerActionProvider>, cx: &mut App) {
    cx.default_global::<Providers>().0.push(provider);
}

/// Every registered provider's actions for `node` on `surface`, grouped by slot (a stable sort:
/// within a slot, registration order). Empty in M4.
pub fn provided_actions(node: &NodeRef, surface: ActionSurface, cx: &App) -> Vec<ProvidedAction> {
    let Some(providers) = cx.try_global::<Providers>() else {
        return Vec::new();
    };
    let mut out: Vec<ProvidedAction> = providers
        .0
        .iter()
        .flat_map(|p| p.actions(node, surface, cx))
        .collect();
    out.sort_by_key(|a| a.slot);
    out
}

// ---- Open data (R-M4-13) ----------------------------------------------------------------------

/// What "Open data" runs on a table or view: Task 24 installs the M4 default (a console on the
/// same profile and schema running `SELECT * FROM <qualified name>` through the normal Run
/// path); M6 replaces it with the table editor.
pub type OpenDataHandler = Box<dyn Fn(ProfileId, ObjectKey, &mut Window, &mut App)>;

/// The installed handler (unset until Task 24).
#[derive(Default)]
struct OpenData(Option<Rc<OpenDataHandler>>);

impl Global for OpenData {}

/// Installs the open-data handler, replacing the one before it.
pub fn set_open_data_handler(handler: OpenDataHandler, cx: &mut App) {
    cx.default_global::<OpenData>().0 = Some(Rc::new(handler));
}

/// Runs the open-data handler for `key` of `profile`. Returns whether a handler ran (`false`
/// while none is installed: Open data does nothing, and its command stays disabled).
pub fn open_data(profile: ProfileId, key: ObjectKey, window: &mut Window, cx: &mut App) -> bool {
    let Some(handler) = cx.try_global::<OpenData>().and_then(|h| h.0.clone()) else {
        return false;
    };
    handler(profile, key, window, cx);
    true
}
