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

// ## m4-t24-live ---------------------------------------------------------------------------------
//
// The menus of the explorer's rows (mock 5), the commands behind them and the default open-data
// handler (R-M4-13). Every command acts on the tree's cursor row: a right click (or Shift-F10)
// first puts the cursor on the row the event names (a group header included, R-W1-5), then the
// menu opens.
//
// - **Names.** "Copy name" copies the stored name verbatim; "Copy qualified name" and Open data
//   quote through `sql_ident::quote` only where the dialect needs it (`public."Users"`,
//   `api.\`order\``, `main."a b"`); a PostgreSQL name never carries its database. A name the
//   dialect cannot quote (NUL) copies and opens nothing.
// - **Open data** opens a new console on the same profile with `SELECT * FROM <qualified name>`
//   and runs it through the normal Run path (`console::commands::open_with_text`): the gate,
//   the classifier, the cursor, the first page. On a PostgreSQL database other than the
//   profile's own the console opens with the text but does not run (the console's session is on
//   the profile's database).
// - **Hide objects like this** adds a negative item for the name's prefix (`yedek_fatura_*`) to
//   the namespace's view filter in the profile's data file; the explorer hides those rows and
//   the group header shows the filter glyph. The catalog is untouched: lookups still find them.
// - **Modify…** is a disabled placeholder whose hint names a later version (PO-Q13); a provider
//   that fills the `Modify` slot replaces it. Other later actions appear only when provided
//   (PO-Q16). View DDL and Copy DDL appear once Task 25 enables their commands.

pub use t24::*;

mod t24 {
    use std::collections::HashSet;
    use std::rc::Rc;

    use hacivat_introspect::filter::{FilterRole, parse_filter};
    use hacivat_meta::kind::{DbLevel, database_level};
    use hacivat_meta::name::quote::{QuoteCtx, quote_qualified};
    use hacivat_meta::{NsKey, ObjectKey, ObjectKind};
    use hacivat_proto::{DbmsKind, ProfileId};
    use hacivat_ui::actions::ActionRegistry;
    use hacivat_ui::menu::MenuItemSpec;
    use hacivat_ui::text::TextFn;
    use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
    use hacivat_ui::tree::{ChildState, TreeKey, TreeModel as _};
    use hacivat_ui_core::gpui_surface::{App, ClipboardItem, Pixels, Point, Window};

    use super::{
        ActionSlot, ActionSurface, NodeRef, OpenData, open_data, provided_actions,
        set_open_data_handler,
    };
    use crate::explorer::ExplorerModel;
    use crate::explorer::adapter::{self, NodeInfo};
    use crate::explorer::level_pill::{self, namespace_object_key};
    use crate::explorer::{live, userdata};
    use crate::l10n_global::{L10nGlobal, text_fn_id};
    use crate::shell::keymap_state::shortcut_label;

    /// The kinds Open data (and a double click) opens.
    pub fn opens_data(kind: ObjectKind) -> bool {
        matches!(
            kind,
            ObjectKind::Table
                | ObjectKind::View
                | ObjectKind::MaterializedView
                | ObjectKind::ForeignTable
        )
    }

    /// The parts of `key`'s qualified name on `dbms`, outermost first: a column keeps its
    /// table, any other object its schema; the database of a database-level engine is dropped.
    pub fn qualified_parts(dbms: DbmsKind, key: &ObjectKey) -> Vec<&str> {
        let skip = usize::from(database_level(dbms) == DbLevel::Database && key.path.len() > 1);
        let path: Vec<&str> = key.path.iter().skip(skip).map(|n| &**n).collect();
        match key.kind {
            ObjectKind::Database | ObjectKind::Schema => path.last().copied().into_iter().collect(),
            ObjectKind::Column => path,
            _ if path.len() > 2 => vec![path[0], path[path.len() - 1]],
            _ => path,
        }
    }

    /// `key`'s qualified name on `dbms`, quoted where needed; `None` when a part cannot be
    /// quoted (a NUL).
    pub fn qualified_name(dbms: DbmsKind, key: &ObjectKey) -> Option<String> {
        let parts = qualified_parts(dbms, key);
        if parts.is_empty() {
            return None;
        }
        quote_qualified(live::dialect_of(dbms), &parts, &QuoteCtx::default()).ok()
    }

    /// The statement Open data runs for `key` on `dbms`.
    pub fn open_data_sql(dbms: DbmsKind, key: &ObjectKey) -> Option<String> {
        if !opens_data(key.kind) {
            return None;
        }
        Some(format!("SELECT * FROM {}", qualified_name(dbms, key)?))
    }

    /// The default open-data handler (R-M4-13).
    fn default_open_data(profile: ProfileId, key: ObjectKey, window: &mut Window, cx: &mut App) {
        let Some(p) = cx
            .try_global::<crate::connections::ConnectionsModel>()
            .and_then(|m| m.get(profile))
        else {
            return;
        };
        let Some(sql) = open_data_sql(p.engine, &key) else {
            return;
        };
        // Fix round 1 (I-3): on a database-level engine the object's database goes with the text;
        // `open_with_text` refuses (with a notice) unless the console's session is on it.
        let database = (database_level(p.engine) == DbLevel::Database)
            .then(|| key.path.first().map(|d| d.to_string()))
            .flatten();
        crate::console::open_with_text(profile, database.as_deref(), &sql, true, window, cx);
    }

    /// Installs the default open-data handler unless one is set.
    pub fn install_default_open_data(cx: &mut App) {
        if cx.try_global::<OpenData>().is_some_and(|h| h.0.is_some()) {
            return;
        }
        set_open_data_handler(Box::new(default_open_data), cx);
    }

    // ---- the cursor's row ---------------------------------------------------------------------

    /// The tree's cursor row and what it is.
    fn cursor(cx: &App) -> Option<(TreeKey, NodeInfo)> {
        let tree = crate::connections::explorer::tree(cx)?;
        let t = tree.read(cx);
        let key = t.cursor()?;
        Some((key, t.model().info(key)?))
    }

    fn cursor_object(cx: &App) -> Option<(ProfileId, ObjectKey)> {
        match cursor(cx)?.1 {
            NodeInfo::Object { profile, key } => Some((profile, key)),
            _ => None,
        }
    }

    fn engine(profile: ProfileId, cx: &App) -> Option<DbmsKind> {
        cx.try_global::<crate::connections::ConnectionsModel>()?
            .get(profile)
            .map(|p| p.engine)
    }

    /// The text a copy of row `info` puts on the clipboard.
    pub fn copy_text(info: &NodeInfo, qualified: bool, cx: &App) -> Option<String> {
        let (profile, key) = match info {
            NodeInfo::Object { profile, key } => (*profile, key.clone()),
            NodeInfo::Schema { profile, ns } => (*profile, namespace_object_key(ns)),
            NodeInfo::Database { profile, name } => (
                *profile,
                namespace_object_key(&NsKey::database(name.clone())),
            ),
            _ => return None,
        };
        if !qualified {
            return key.path.last().map(|n| n.to_string());
        }
        qualified_name(engine(profile, cx)?, &key)
    }

    fn copy(qualified: bool, cx: &mut App) {
        let Some((_, info)) = cursor(cx) else {
            return;
        };
        let Some(text) = copy_text(&info, qualified, cx) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
        let label: TextFn = Rc::new(move |cx: &App| match cx.try_global::<L10nGlobal>() {
            Some(g) => g
                .l10n
                .t_dynamic(
                    "hv-explorer-copied",
                    vec![("name", hacivat_l10n::ArgValue::Text(text.clone()))],
                )
                .into(),
            None => text.clone().into(),
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Info, label));
    }

    /// `explorer.copy_qualified_name` (⌥⌘C).
    pub fn copy_qualified_name(_: &mut Window, cx: &mut App) {
        copy(true, cx);
    }

    /// `explorer.copy_name`.
    pub fn copy_name(_: &mut Window, cx: &mut App) {
        copy(false, cx);
    }

    /// Whether the copy commands can run.
    pub fn can_copy(cx: &App) -> bool {
        cursor(cx).is_some_and(|(_, i)| {
            matches!(
                i,
                NodeInfo::Object { .. } | NodeInfo::Schema { .. } | NodeInfo::Database { .. }
            )
        })
    }

    /// `explorer.open_data`: Open data on the cursor's table or view.
    pub fn open_data_command(window: &mut Window, cx: &mut App) {
        if let Some((profile, key)) = cursor_object(cx).filter(|(_, k)| opens_data(k.kind)) {
            open_data(profile, key, window, cx);
        }
    }

    /// Whether Open data can run: the cursor is on a table or a view and a handler is set.
    pub fn can_open_data(cx: &App) -> bool {
        cursor_object(cx).is_some_and(|(_, k)| opens_data(k.kind))
            && cx.try_global::<OpenData>().is_some_and(|h| h.0.is_some())
    }

    /// `explorer.refresh` (F5): the cursor row's refresh (`Trigger::User`).
    pub fn refresh_command(_: &mut Window, cx: &mut App) {
        if let Some((key, _)) = cursor(cx) {
            live::refresh_key(key, cx);
        }
    }

    /// Whether Refresh can run: the cursor row's profile has an open catalog.
    pub fn can_refresh(cx: &App) -> bool {
        let Some(tree) = crate::connections::explorer::tree(cx) else {
            return false;
        };
        let t = tree.read(cx);
        t.cursor()
            .and_then(|k| t.model().profile_of(k))
            .is_some_and(|p| live::is_open(p, cx))
    }

    /// `explorer.collapse_all` (⌘←): every row collapses.
    pub fn collapse_all(_: &mut Window, cx: &mut App) {
        if let Some(tree) = crate::connections::explorer::tree(cx) {
            tree.update(cx, |t, cx| t.set_expanded_keys(Vec::new(), cx));
        }
    }

    /// Whether something is expanded.
    pub fn can_collapse_all(cx: &App) -> bool {
        crate::connections::explorer::tree(cx).is_some_and(|t| !t.read(cx).expanded().is_empty())
    }

    /// `key` and its loaded descendants that have loaded children (opening them reads
    /// nothing).
    fn loaded_containers(model: &ExplorerModel, key: TreeKey) -> Vec<TreeKey> {
        let mut out = vec![key];
        let mut i = 0;
        while i < out.len() && out.len() < 10_000 {
            if let ChildState::Loaded(kids) = model.children(out[i]) {
                for k in kids.iter() {
                    if matches!(model.children(*k), ChildState::Loaded(ref c) if !c.is_empty()) {
                        out.push(*k);
                    }
                }
            }
            i += 1;
        }
        out
    }

    /// `explorer.expand_all_children`: every loaded container below the cursor row opens; a
    /// row whose children are not loaded stays closed (nothing is read).
    pub fn expand_all_children(_: &mut Window, cx: &mut App) {
        let Some(tree) = crate::connections::explorer::tree(cx) else {
            return;
        };
        let Some(key) = tree.read(cx).cursor() else {
            return;
        };
        let model = tree.read(cx).model().clone();
        let keys = loaded_containers(&model, key);
        tree.update(cx, |t, cx| {
            let mut all: Vec<TreeKey> = t.expanded().iter().copied().collect();
            all.extend(keys);
            t.set_expanded_keys(all, cx);
        });
    }

    /// `explorer.collapse_children`: the cursor row and every row below it close.
    pub fn collapse_children(_: &mut Window, cx: &mut App) {
        let Some(tree) = crate::connections::explorer::tree(cx) else {
            return;
        };
        let Some(key) = tree.read(cx).cursor() else {
            return;
        };
        let model = tree.read(cx).model().clone();
        let below: HashSet<TreeKey> = loaded_containers(&model, key).into_iter().collect();
        tree.update(cx, |t, cx| {
            let keep: Vec<TreeKey> = t
                .expanded()
                .iter()
                .copied()
                .filter(|k| !below.contains(k))
                .collect();
            t.set_expanded_keys(keep, cx);
        });
    }

    /// Whether the tree has a cursor row.
    pub fn has_cursor(cx: &App) -> bool {
        cursor(cx).is_some()
    }

    // ---- view filters -------------------------------------------------------------------------

    /// The prefix pattern "Hide objects like this" adds for `name`: up to its last `_`
    /// (`yedek_fatura_2019` → `yedek_fatura_*`), else without its trailing digits (`log2019` →
    /// `log*`), else the name itself. Every character a glob reads specially is escaped.
    pub fn hide_pattern(name: &str) -> String {
        let escape = |s: &str| {
            let mut out = String::new();
            for c in s.chars() {
                if !(c.is_alphanumeric() || c == '_') {
                    out.push('\\');
                }
                out.push(c);
            }
            out
        };
        if let Some(at) = name.rfind('_')
            && at > 0
            && at + 1 < name.len()
        {
            return format!("{}*", escape(&name[..=at]));
        }
        let stem = name.trim_end_matches(|c: char| c.is_ascii_digit());
        if !stem.is_empty() && stem.len() < name.len() {
            return format!("{}*", escape(stem));
        }
        escape(name)
    }

    /// The view-filter item that hides objects like `key` (`table:-yedek_fatura_*`).
    pub fn hide_item(key: &ObjectKey) -> Option<String> {
        let name = key.path.last()?;
        Some(format!("{}:-{}", key.kind.id(), hide_pattern(name)))
    }

    fn ns_of(profile: ProfileId, key: &ObjectKey, cx: &App) -> Option<NsKey> {
        adapter::catalog_of(profile, cx)?
            .find(key)
            .map(|h| h.ns.key().clone())
    }

    /// `explorer.hide_like_this`: adds the item to the namespace's view filter.
    pub fn hide_like_this(_: &mut Window, cx: &mut App) {
        let Some((profile, key)) = cursor_object(cx) else {
            return;
        };
        let Some(item) = hide_item(&key) else {
            return;
        };
        let Some(ns) = ns_of(profile, &key, cx) else {
            return;
        };
        // Fix round 1 (m-1): an item the filter grammar cannot read is never written (it would
        // hide nothing); the user is told.
        let dialect = live::dialect_of(engine(profile, cx).unwrap_or(DbmsKind::Postgres));
        if parse_filter(&item, dialect, FilterRole::View).is_err() {
            let name = key.path.last().map(|n| n.to_string()).unwrap_or_default();
            let label: TextFn = Rc::new(move |cx: &App| {
                live::text(
                    cx,
                    "hv-explorer-hide-refused",
                    vec![("name", hacivat_l10n::ArgValue::Text(name.clone()))],
                )
            });
            ToastQueue::push(cx, Toast::new(ToastKind::Error, label));
            return;
        }
        let slot = namespace_object_key(&ns);
        drop(userdata::update(
            profile,
            move |d| {
                let text = d.view_filters.entry(slot.clone()).or_default();
                if !text.split("||").any(|i| i.trim() == item) {
                    if !text.trim().is_empty() {
                        text.push_str("||");
                    }
                    text.push_str(&item);
                }
            },
            cx,
        ));
    }

    /// Whether "Hide objects like this" can run: the cursor is on a grouped object of a profile
    /// whose data file was read.
    pub fn can_hide(cx: &App) -> bool {
        cursor_object(cx).is_some_and(|(p, k)| {
            userdata::get(p, cx).is_some()
                && k.kind
                    .group(engine(p, cx).unwrap_or(DbmsKind::Postgres))
                    .is_some()
        })
    }

    /// The profile and view-filter slot of the cursor's group header, when it has a filter.
    fn cursor_group_filter(cx: &App) -> Option<(ProfileId, ObjectKey)> {
        let tree = crate::connections::explorer::tree(cx)?;
        let t = tree.read(cx);
        let key = t.cursor()?;
        let NodeInfo::Group { profile, .. } = t.model().info(key)? else {
            return None;
        };
        let flat = t.flat();
        let owner = flat
            .index_of(key)
            .and_then(|ix| flat.parent_of(ix))
            .and_then(|ix| flat.get(ix))
            .map(|r| r.key)?;
        let ns = match t.model().info(owner)? {
            NodeInfo::Schema { ns, .. } => ns,
            NodeInfo::Object { key, .. } => ns_of(profile, &key, cx)?,
            _ => return None,
        };
        let slot = namespace_object_key(&ns);
        userdata::get(profile, cx)?
            .view_filters
            .contains_key(&slot)
            .then_some((profile, slot))
    }

    /// `explorer.show_hidden`: removes the view filter of the cursor group's namespace.
    pub fn show_hidden(_: &mut Window, cx: &mut App) {
        if let Some((profile, slot)) = cursor_group_filter(cx) {
            drop(userdata::update(
                profile,
                move |d| {
                    d.view_filters.remove(&slot);
                },
                cx,
            ));
        }
    }

    /// Whether the cursor's group has a view filter.
    pub fn can_show_hidden(cx: &App) -> bool {
        cursor_group_filter(cx).is_some()
    }

    /// `explorer.level_menu`: Task 20's level list of the cursor's database or schema row,
    /// under the row.
    pub fn level_menu(window: &mut Window, cx: &mut App) {
        let Some(tree) = crate::connections::explorer::tree(cx) else {
            return;
        };
        let Some(key) = tree.read(cx).cursor() else {
            return;
        };
        let at = tree
            .read(cx)
            .row_bounds(key)
            .map(|b| b.bottom_left())
            .unwrap_or_default();
        level_pill::open_from_pill(key, at, window, cx);
    }

    /// A placeholder that never runs.
    pub fn never(_: &App) -> bool {
        false
    }

    // ---- the menus ----------------------------------------------------------------------------

    fn item(label: &'static str, action: &'static str, cx: &App) -> MenuItemSpec {
        let enabled = cx
            .try_global::<ActionRegistry>()
            .and_then(|r| r.get(action))
            .is_some_and(|d| (d.enabled)(cx));
        let mut spec = MenuItemSpec::new(text_fn_id(label), action).disabled(!enabled);
        if shortcut_label(cx, action).is_some() {
            let keys: TextFn =
                Rc::new(move |cx: &App| shortcut_label(cx, action).unwrap_or_default().into());
            spec = spec.keys(keys);
        }
        spec
    }

    /// An item only while its command can run (later-task items stay hidden, PO-Q16).
    fn item_if_enabled(
        label: &'static str,
        action: &'static str,
        cx: &App,
    ) -> Option<MenuItemSpec> {
        let spec = item(label, action, cx);
        (!spec.disabled).then_some(spec)
    }

    fn provided(node: &NodeRef, slot: ActionSlot, cx: &App) -> Vec<MenuItemSpec> {
        provided_actions(node, ActionSurface::ExplorerMenu, cx)
            .into_iter()
            .filter(|a| a.slot == slot)
            .map(|a| {
                let label = a.label.clone();
                MenuItemSpec::new(Rc::new(move |_: &App| label.clone()), a.id).disabled(!a.enabled)
            })
            .collect()
    }

    /// The menu of tree row `key` (mock 5); `None` for a row without one (an environment
    /// header; a profile root keeps its M2a menu, `connections::explorer::menu_items`).
    pub fn node_menu(key: TreeKey, cx: &App) -> Option<Vec<MenuItemSpec>> {
        let tree = crate::connections::explorer::tree(cx)?;
        let info = tree.read(cx).model().info(key)?;
        let sep = MenuItemSpec::separator;
        let mut out: Vec<MenuItemSpec> = Vec::new();
        match &info {
            NodeInfo::Database { .. } | NodeInfo::Schema { .. } => {
                out.push(item("hv-explorer-menu-refresh", "explorer.refresh", cx));
                if matches!(info, NodeInfo::Database { .. }) {
                    out.push(item(
                        "hv-explorer-menu-edit-scope",
                        "explorer.edit_scope",
                        cx,
                    ));
                }
                out.push(item("hv-explorer-menu-level", "explorer.level_menu", cx));
                out.push(sep());
                out.push(item(
                    "hv-explorer-menu-copy-name",
                    "explorer.copy_qualified_name",
                    cx,
                ));
                out.push(item(
                    "hv-explorer-menu-expand-all",
                    "explorer.expand_all_children",
                    cx,
                ));
                out.push(item(
                    "hv-explorer-menu-collapse-children",
                    "explorer.collapse_children",
                    cx,
                ));
            }
            NodeInfo::Group { .. } | NodeInfo::BuiltIn { .. } => {
                if matches!(info, NodeInfo::Group { .. }) {
                    out.push(item("hv-explorer-menu-refresh", "explorer.refresh", cx));
                    out.extend(item_if_enabled(
                        "hv-explorer-menu-show-hidden",
                        "explorer.show_hidden",
                        cx,
                    ));
                }
                out.push(item(
                    "hv-explorer-menu-expand-all",
                    "explorer.expand_all_children",
                    cx,
                ));
                out.push(item(
                    "hv-explorer-menu-collapse-children",
                    "explorer.collapse_children",
                    cx,
                ));
            }
            NodeInfo::Object {
                profile,
                key: object,
            } => {
                let node = NodeRef {
                    profile: *profile,
                    key,
                    object: Some(object.clone()),
                    kind: Some(object.kind),
                };
                let column = object.kind == ObjectKind::Column;
                if opens_data(object.kind) {
                    out.push(item("hv-explorer-menu-open-data", "explorer.open_data", cx));
                }
                out.extend(provided(&node, ActionSlot::Data, cx));
                out.extend(item_if_enabled(
                    if column {
                        "hv-explorer-menu-view-table-ddl"
                    } else {
                        "hv-explorer-menu-view-ddl"
                    },
                    "explorer.view_ddl",
                    cx,
                ));
                if !out.is_empty() {
                    out.push(sep());
                }
                out.push(item(
                    "hv-explorer-menu-copy-plain",
                    "explorer.copy_name",
                    cx,
                ));
                out.push(item(
                    "hv-explorer-menu-copy-name",
                    "explorer.copy_qualified_name",
                    cx,
                ));
                out.extend(item_if_enabled(
                    "hv-explorer-menu-copy-ddl",
                    "explorer.copy_ddl",
                    cx,
                ));
                if !column {
                    out.push(sep());
                    out.push(item("hv-explorer-menu-refresh", "explorer.refresh", cx));
                    let grouped = object
                        .kind
                        .group(engine(*profile, cx).unwrap_or(DbmsKind::Postgres))
                        .is_some();
                    if grouped {
                        out.push(item(
                            "hv-explorer-menu-hide-like",
                            "explorer.hide_like_this",
                            cx,
                        ));
                    }
                }
                let transfer = provided(&node, ActionSlot::Transfer, cx);
                if !transfer.is_empty() {
                    out.push(sep());
                    out.extend(transfer);
                }
                // Modify…: the provider's, else the disabled placeholder (PO-Q13).
                let modify = provided(&node, ActionSlot::Modify, cx);
                let placeholder = match object.kind {
                    ObjectKind::Table | ObjectKind::ForeignTable => {
                        Some("hv-explorer-menu-modify-table")
                    }
                    ObjectKind::View | ObjectKind::MaterializedView => {
                        Some("hv-explorer-menu-modify-view")
                    }
                    ObjectKind::Column => Some("hv-explorer-menu-modify-column"),
                    _ => None,
                };
                if !modify.is_empty() {
                    out.push(sep());
                    out.extend(modify);
                } else if let Some(label) = placeholder {
                    out.push(sep());
                    out.push(
                        MenuItemSpec::new(text_fn_id(label), MODIFY)
                            .disabled(true)
                            .hint(text_fn_id("hv-explorer-menu-modify-tip")),
                    );
                }
                let source = provided(&node, ActionSlot::Source, cx);
                if !source.is_empty() {
                    out.push(sep());
                    out.extend(source);
                }
            }
            NodeInfo::Environment(_) | NodeInfo::Profile(_) | NodeInfo::Contributed { .. } => {
                return None;
            }
        }
        Some(out)
    }

    /// The disabled Modify… placeholder's command (never enabled; PO-Q13).
    pub const MODIFY: &str = "explorer.modify";

    /// Opens row `key`'s menu at `at` (the cursor moves to the row first).
    pub fn open_node_menu(key: TreeKey, at: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let Some(tree) = crate::connections::explorer::tree(cx) else {
            return;
        };
        tree.update(cx, |t, cx| t.select(key, cx));
        let Some(items) = node_menu(key, cx) else {
            return;
        };
        let Some(ws) = window.root::<crate::shell::Workspace>().flatten() else {
            return;
        };
        ws.update(cx, |ws, cx| {
            ws.open_popup_menu(at, items, window, cx);
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn key(kind: ObjectKind, path: &[&str]) -> ObjectKey {
            ObjectKey {
                kind,
                path: path.iter().map(|s| hacivat_meta::Name::from(*s)).collect(),
                server_id: None,
                overload_sig: None,
                raw_kind: None,
            }
        }

        #[test]
        fn qualified_names_quote_only_where_needed() {
            let pg = |p: &[&str]| qualified_name(DbmsKind::Postgres, &key(ObjectKind::Table, p));
            assert_eq!(
                pg(&["app", "public", "Users"]).as_deref(),
                Some("public.\"Users\"")
            );
            assert_eq!(
                pg(&["app", "public", "users"]).as_deref(),
                Some("public.users")
            );
            assert_eq!(
                qualified_name(DbmsKind::MySql, &key(ObjectKind::Table, &["api", "order"]))
                    .as_deref(),
                Some("api.`order`")
            );
            assert_eq!(
                qualified_name(DbmsKind::Sqlite, &key(ObjectKind::Table, &["main", "a b"]))
                    .as_deref(),
                Some("main.\"a b\"")
            );
            assert_eq!(
                qualified_name(
                    DbmsKind::Postgres,
                    &key(ObjectKind::Column, &["app", "public", "users", "Id"])
                )
                .as_deref(),
                Some("public.users.\"Id\"")
            );
            assert_eq!(
                pg(&["app", "public", "a\u{0}b"]),
                None,
                "a NUL copies nothing"
            );
        }

        #[test]
        fn hide_patterns_take_the_prefix_and_escape_glob_characters() {
            assert_eq!(hide_pattern("yedek_fatura_2019"), "yedek_fatura_*");
            assert_eq!(hide_pattern("log2019"), "log*");
            assert_eq!(hide_pattern("users"), "users");
            assert_eq!(hide_pattern("a*b_1"), "a\\*b_*");
            assert_eq!(
                hide_item(&key(
                    ObjectKind::Table,
                    &["app", "public", "yedek_fatura_2019"]
                ))
                .as_deref(),
                Some("table:-yedek_fatura_*")
            );
        }
    }
}
