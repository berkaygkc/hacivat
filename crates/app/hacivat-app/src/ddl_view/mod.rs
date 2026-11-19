//! The DDL view tab (Task 25; design/m4-explorer surfaces 10 and 11): a read-only editor with the
//! object's script (Task 19's `create_script`), the options strip, Copy, Open in console and
//! Refresh, and the stale, gone, unsupported-kind and executable-comment notices.
//!
//! ## m4-t01b-seams: the tab is a real [`TabKind::DdlView`](crate::shell::TabKind) (ruling
//! R-W0-1): one tab per object, titled "{name} · DDL", in the [`DDL_VIEW_CONTEXT`] key context.
//! The shell asks this module for the title ([`tab_title`]) and the view ([`tab_view`]), so
//! Task 25 fills them here and never edits the shell. DDL view tabs are not saved in
//! `workspace.json`.
//!
//! Stubs until Task 25: [`open_ddl_view`] opens nothing and answers `Err(NotYet)`;
//! [`tab_view`] builds no view (the tab shows the empty editor state); `ddl_view.copy` is
//! registered **disabled**.

use std::rc::Rc;

use hacivat_l10n::{Text, ids};
use hacivat_meta::ObjectKey;
use hacivat_proto::ProfileId;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{AnyView, App, Entity, Global, SharedString, Window};

use crate::l10n_global::tr;
use crate::shell::{DdlObjectId, Workspace};

/// The key context while a DDL view tab is active (`ddl_view.copy`'s row).
pub const DDL_VIEW_CONTEXT: &str = "DdlView";

/// The DDL view commands that are still stubs (registered disabled; Task 25 fills them).
pub const STUB_COMMANDS: [&str; 1] = ["ddl_view.copy"];

/// Why a DDL view did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdlViewError {
    /// The DDL view is not built yet (Task 25).
    NotYet,
}

fn not_yet(_: &App) -> bool {
    false
}

/// Registers the DDL view's commands (one line in `crate::init`).
pub fn install(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "ddl_view.copy",
        title_id: "hv-cmd-ddl-view-copy",
        context: DDL_VIEW_CONTEXT,
        run: Rc::new(|_, _| {}),
        enabled: not_yet,
    });
}

/// Opens (or focuses) the DDL view tab of `key` of `profile` in `workspace`. Stub (Task 25):
/// opens nothing and answers `Err(NotYet)`; no command calls it yet.
pub fn open_ddl_view(
    _workspace: &Entity<Workspace>,
    _profile: ProfileId,
    _key: ObjectKey,
    _window: &mut Window,
    _cx: &mut App,
) -> Result<TabId, DdlViewError> {
    Err(DdlViewError::NotYet)
}

// ---- the tab's object handles ---------------------------------------------------------------

/// The objects DDL view tabs were opened for, in interning order (handle = index + 1).
#[derive(Default)]
struct DdlObjects(Vec<(ProfileId, ObjectKey)>);

impl Global for DdlObjects {}

/// The tab handle of `key` of `profile`: the same object (`ObjectKey::same_object`) of the same
/// profile always gets the same handle, so its DDL view opens once.
pub fn object_id(profile: ProfileId, key: &ObjectKey, cx: &mut App) -> DdlObjectId {
    let objects = &mut cx.default_global::<DdlObjects>().0;
    let at = match objects
        .iter()
        .position(|(p, k)| *p == profile && k.same_object(key))
    {
        Some(at) => at,
        None => {
            objects.push((profile, key.clone()));
            objects.len() - 1
        }
    };
    DdlObjectId(at as u64 + 1)
}

/// The profile and object behind a tab handle.
pub fn object_key(object: DdlObjectId, cx: &App) -> Option<(ProfileId, ObjectKey)> {
    let at = usize::try_from(object.0.checked_sub(1)?).ok()?;
    cx.try_global::<DdlObjects>()?.0.get(at).cloned()
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

/// The view of a DDL view tab (built once per tab). Stub (Task 25): none, so the tab shows the
/// empty editor state.
pub fn tab_view(
    _profile: ProfileId,
    _object: DdlObjectId,
    _window: &mut Window,
    _cx: &mut App,
) -> Option<AnyView> {
    None
}
