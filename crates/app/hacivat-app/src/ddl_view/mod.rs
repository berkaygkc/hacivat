//! The DDL view tab (Task 25; design/m4-explorer surfaces 10 and 11): a read-only editor with the
//! object's script (Task 19's `create_script_with`), the options strip, Copy, Open in console and
//! Refresh, and the stale, gone, unsupported-kind and executable-comment notices.
//!
//! ## m4-t01b-seams: the tab is a real [`TabKind::DdlView`](crate::shell::TabKind) (ruling
//! R-W0-1): one tab per object, titled "{name} · DDL", in the [`DDL_VIEW_CONTEXT`] key context.
//! The shell asks this module for the title ([`tab_title`]) and the view ([`tab_view`]), so
//! Task 25 fills them here and never edits the shell. DDL view tabs are not saved in
//! `workspace.json` (ruling R-W0-5).
//!
//! ## m4-t25-ddl-view
//!
//! - [`open_ddl_view`] opens (or focuses) the object's tab; the explorer's View DDL (F12, the
//!   node menu) calls it for the cursor's object, a column's table.
//! - The view ([`view::DdlView`]) reads the profile's catalog through [`source::DdlCatalog`]:
//!   the snapshot synchronously, every request as a ticket awaited on a GPUI foreground task.
//! - **The object table** (T1b carry m3): a handle per open object, dropped when its view is
//!   released (its tab closed); the title follows a rename (an object with a server id keeps
//!   its handle and its key moves to the new name).
//! - `ddl_view.copy` (palette and toolbar only, ruling R-W0-6) copies the active DDL view's
//!   text.

pub mod model;
pub mod source;
pub mod view;

use std::collections::BTreeMap;
use std::rc::Rc;

use hacivat_l10n::{Text, ids};
use hacivat_meta::ObjectKey;
use hacivat_proto::ProfileId;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{
    AnyView, App, AppContext as _, Entity, Global, SharedString, WeakEntity, Window,
};

use crate::l10n_global::tr;
use crate::shell::workspace::ActiveTab;
use crate::shell::{DdlObjectId, Workspace};

pub use source::{DdlCatalog, set_catalog_provider};
pub use view::{Body, DdlView, DdlViewState, Freshness, GrantsUx};

/// The key context while a DDL view tab is active (`ddl_view.copy`'s row).
pub const DDL_VIEW_CONTEXT: &str = "DdlView";

/// The key context while a DDL view's text has the focus (`ddl_view.reload`'s F5 row; never
/// the tab's, so the chord reaches no other field).
pub const DDL_VIEW_CODE_CONTEXT: &str = "DdlViewCode";

/// The DDL view commands that are still stubs (registered disabled).
// ## m4-t25-ddl-view: `ddl_view.copy` is filled.
pub const STUB_COMMANDS: [&str; 0] = [];

/// Why a DDL view did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdlViewError {
    /// The DDL view is not built yet (Task 25).
    NotYet,
    /// The workspace has no room for another tab (the tab IDs are used up).
    NoTab,
}

/// Registers the DDL view's commands (one line in `crate::init`).
pub fn install(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "ddl_view.copy",
        title_id: "hv-cmd-ddl-view-copy",
        context: DDL_VIEW_CONTEXT,
        run: Rc::new(|window, cx| {
            if let Some(v) = active_view(cx) {
                v.update(cx, |v, cx| v.copy(window, cx));
            }
        }),
        enabled: can_copy,
    });
    // ## m4-t25-ddl-view (T25 round 1): F5 in the text reloads (surface 11).
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "ddl_view.reload",
        title_id: "hv-cmd-ddl-view-reload",
        context: DDL_VIEW_CODE_CONTEXT,
        run: Rc::new(|_, cx| {
            if let Some(v) = active_view(cx) {
                v.update(cx, |v, cx| v.refresh(cx));
            }
        }),
        enabled: |cx| active_view(cx).is_some(),
    });
}

/// The DDL view of the focused pane's active tab, if it is one (the workspace mirrors that tab
/// into `ActiveTab`; the tabs [`open_ddl_view`] opened are known by their id).
fn active_view(cx: &App) -> Option<Entity<DdlView>> {
    let tab = cx.try_global::<ActiveTab>()?.0?;
    let objects = cx.try_global::<DdlObjects>()?;
    let object = *objects.tabs.get(&tab.0)?;
    objects.views.get(&object)?.upgrade()
}

fn can_copy(cx: &App) -> bool {
    active_view(cx).is_some_and(|v| v.read(cx).can_copy())
}

/// The explorer cursor's object (`explorer.view_ddl`).
fn cursor_object(cx: &App) -> Option<(ProfileId, ObjectKey)> {
    let tree = crate::connections::explorer::tree(cx)?;
    let t = tree.read(cx);
    match t.model().info(t.cursor()?)? {
        crate::explorer::adapter::NodeInfo::Object { profile, key } => Some((profile, key)),
        _ => None,
    }
}

/// `explorer.view_ddl` (F12, the node menu's View DDL): the cursor's object (a column's table).
pub fn view_ddl_command(window: &mut Window, cx: &mut App) {
    let Some((profile, key)) = cursor_object(cx) else {
        return;
    };
    let Some(ws) = window.root::<Workspace>().flatten() else {
        return;
    };
    let _ = open_ddl_view(&ws, profile, key, window, cx);
}

/// Whether the explorer's cursor is on an object.
pub fn can_view_ddl(cx: &App) -> bool {
    cursor_object(cx).is_some()
}

/// Opens (or focuses) the DDL view tab of `key` of `profile` in `workspace` (a column's opens
/// its table's).
pub fn open_ddl_view(
    workspace: &Entity<Workspace>,
    profile: ProfileId,
    key: ObjectKey,
    _window: &mut Window,
    cx: &mut App,
) -> Result<TabId, DdlViewError> {
    let snap = source::catalog_for(profile, cx).and_then(|c| c.snapshot(cx));
    let key = model::ddl_object(snap.as_deref(), &key);
    let object = object_id(profile, &key, cx);
    let tab = workspace
        .update(cx, |w, cx| w.open_ddl_view_tab(profile, object, cx))
        .ok_or(DdlViewError::NoTab)?;
    cx.default_global::<DdlObjects>()
        .tabs
        .insert(tab.0, object.0);
    Ok(tab)
}

// ---- the tab's object handles ---------------------------------------------------------------

/// The objects DDL view tabs were opened for, by handle (T1b carry m3: an entry is dropped when
/// its view is released, so the table shrinks as tabs close).
#[derive(Default)]
struct DdlObjects {
    by_id: BTreeMap<u64, (ProfileId, ObjectKey)>,
    next: u64,
    views: BTreeMap<u64, WeakEntity<DdlView>>,
    /// The tabs [`open_ddl_view`] opened: tab id → object handle.
    tabs: BTreeMap<u32, u64>,
}

impl Global for DdlObjects {}

/// The tab handle of `key` of `profile`: the same object (`ObjectKey::same_object`) of the same
/// profile gets the same handle while its entry lives, so its DDL view opens once.
pub fn object_id(profile: ProfileId, key: &ObjectKey, cx: &mut App) -> DdlObjectId {
    let objects = cx.default_global::<DdlObjects>();
    if let Some((id, _)) = objects
        .by_id
        .iter()
        .find(|(_, (p, k))| *p == profile && k.same_object(key))
    {
        return DdlObjectId(*id);
    }
    objects.next += 1;
    let id = objects.next;
    objects.by_id.insert(id, (profile, key.clone()));
    DdlObjectId(id)
}

/// The profile and object behind a tab handle.
pub fn object_key(object: DdlObjectId, cx: &App) -> Option<(ProfileId, ObjectKey)> {
    cx.try_global::<DdlObjects>()?.by_id.get(&object.0).cloned()
}

/// How many objects the table holds (tests: it shrinks as tabs close).
pub fn object_count(cx: &App) -> usize {
    cx.try_global::<DdlObjects>().map_or(0, |o| o.by_id.len())
}

/// The object `object`'s key became `key` (a rename the catalog reported for the same object).
pub(crate) fn rekey(object: DdlObjectId, key: ObjectKey, cx: &mut App) {
    if let Some(e) = cx.default_global::<DdlObjects>().by_id.get_mut(&object.0) {
        e.1 = key;
    }
}

/// Forgets `object` (its view was released: the tab closed).
pub(crate) fn forget(object: DdlObjectId, cx: &mut App) {
    let objects = cx.default_global::<DdlObjects>();
    objects.by_id.remove(&object.0);
    objects.views.remove(&object.0);
    objects.tabs.retain(|_, o| *o != object.0);
}

fn view_by_id(object: DdlObjectId, cx: &App) -> Option<Entity<DdlView>> {
    cx.try_global::<DdlObjects>()?
        .views
        .get(&object.0)?
        .upgrade()
}

/// The open DDL view of `key` of `profile`.
pub fn view_of(profile: ProfileId, key: &ObjectKey, cx: &App) -> Option<Entity<DdlView>> {
    let objects = cx.try_global::<DdlObjects>()?;
    let (id, _) = objects
        .by_id
        .iter()
        .find(|(_, (p, k))| *p == profile && k.same_object(key))?;
    objects.views.get(id)?.upgrade()
}

/// What the open DDL view of `key` of `profile` shows (tests, shots).
pub fn state_of(profile: ProfileId, key: &ObjectKey, cx: &App) -> Option<DdlViewState> {
    view_of(profile, key, cx).map(|v| v.read(cx).state(cx))
}

/// The tab's title, "{name} · DDL" with the object's own name (verbatim, isolated by Fluent).
pub fn tab_title(profile: ProfileId, object: DdlObjectId, cx: &App) -> SharedString {
    let name = object_key(object, cx)
        .filter(|(p, _)| *p == profile)
        .and_then(|(_, key)| key.path.last().map(|n| n.to_string()))
        .unwrap_or_default();
    tr(
        cx,
        ids::hv::DDL_VIEW_TAB_TITLE,
        ids::hv::HvDdlViewTabTitle { name: Text(name) },
    )
}

/// The view of a DDL view tab (built once per tab).
pub fn tab_view(
    profile: ProfileId,
    object: DdlObjectId,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    if let Some(v) = view_by_id(object, cx) {
        return Some(v.into());
    }
    let (p, key) = object_key(object, cx)?;
    if p != profile {
        return None;
    }
    let view = cx.new(|cx| DdlView::new(profile, object, key, window, cx));
    cx.default_global::<DdlObjects>()
        .views
        .insert(object.0, view.downgrade());
    Some(view.into())
}
