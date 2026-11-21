//! Script tabs (HV-SCRIPT-06, R-M4b-4, R-M4b-22, R-M4b-29): a script tab is the tab kind
//! [`TabKind::Script`](crate::shell::TabKind) whose handle ([`ScriptTabId`]) names the
//! (library, file, connection, database) the tab was opened under, for its whole life.
//!
//! ## m4b-t01b-seams: the table of handles and the tab's title. `TabKind` is `Copy` (its users
//! copy it everywhere), so the four values live behind a handle, as M4's DDL view tabs do
//! (`DdlObjectId`, ruling R-W0-1); `workspace.json` stores the values themselves (`kind:
//! "script"`, `library`, `rel_path`, `profile`, `database`). The same values always get the same
//! handle, so "one tab per (file, connection)" (R-M4b-4) compares handles. Task 14 opens the
//! tabs, builds their console view and fills the title rules (NOTES surface 5); until then a
//! script tab shows nothing and runs nothing (only a restored version-2 file can hold one).

use std::collections::BTreeMap;

use hacivat_l10n::{Text, ids};
use hacivat_proto::ProfileId;
use hacivat_ui_core::gpui_surface::{App, Global, SharedString};

use crate::connections::ConnectionsModel;
use crate::l10n_global::tr;
use crate::scripts::{DatabaseName, LibraryId, RelPath};

/// What a script tab is bound to (R-M4b-4: never changed while the tab lives).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScriptTab {
    pub library: LibraryId,
    pub rel: RelPath,
    /// `None`: an unbound script tab ("Open without a connection", or a restored tab whose
    /// profile was deleted). A connect never binds it (R-M4b-30).
    pub profile: Option<ProfileId>,
    /// `None` with a profile: the profile's own database.
    pub database: Option<DatabaseName>,
}

/// A script tab's handle in [`TabKind::Script`](crate::shell::TabKind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptTabId(pub u64);

/// The handles handed out so far (a global; `crate::scripts::install` sets it).
#[derive(Debug, Default)]
pub struct ScriptTabs {
    by_id: BTreeMap<u64, ScriptTab>,
    next: u64,
}

impl Global for ScriptTabs {}

/// The handle of `tab`: the same values get the same handle.
pub fn intern(tab: ScriptTab, cx: &mut App) -> ScriptTabId {
    let tabs = cx.default_global::<ScriptTabs>();
    if let Some((id, _)) = tabs.by_id.iter().find(|(_, t)| **t == tab) {
        return ScriptTabId(*id);
    }
    tabs.next += 1;
    let id = tabs.next;
    tabs.by_id.insert(id, tab);
    ScriptTabId(id)
}

/// What handle `id` names.
pub fn spec(id: ScriptTabId, cx: &App) -> Option<ScriptTab> {
    cx.try_global::<ScriptTabs>()?.by_id.get(&id.0).cloned()
}

/// The tab's title: "file · connection" (NOTES surface 5, `hv-scripts-tab-title`), or "file · no
/// connection" when it is unbound or its profile is gone. Task 14 adds the database part and
/// the truncation rules (ruling d).
pub fn tab_title(id: ScriptTabId, cx: &App) -> SharedString {
    let Some(tab) = spec(id, cx) else {
        return SharedString::default();
    };
    let file = display_name(crate::scripts::file_name(&tab.rel));
    let conn = tab.profile.and_then(|p| {
        cx.try_global::<ConnectionsModel>()?
            .profiles()
            .iter()
            .find(|q| q.id == p)
            .map(|q| display_name(&q.name))
    });
    match conn {
        Some(conn) => tr(
            cx,
            ids::hv::SCRIPTS_TAB_TITLE,
            ids::hv::HvScriptsTabTitle {
                file: Text(file),
                conn: Text(conn),
            },
        ),
        None => tr(
            cx,
            ids::hv::SCRIPTS_TAB_TITLE_UNBOUND,
            ids::hv::HvScriptsTabTitleUnbound { file: Text(file) },
        ),
    }
}

/// `name` for display (R-M4b-23): control characters, bidi controls and invisible characters
/// show as `<U+XXXX>`. Task 4's `display_name` in `hacivat-scripts` takes over (the controller
/// reconciles them).
pub fn display_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let hidden = c.is_control()
            || matches!(
                c,
                '\u{061C}'
                    | '\u{200B}'..='\u{200F}'
                    | '\u{2028}'..='\u{202E}'
                    | '\u{2060}'..='\u{2069}'
                    | '\u{FEFF}'
            );
        if hidden {
            out.push_str(&format!("<U+{:04X}>", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_show_as_code_points() {
        assert_eq!(display_name("a\u{202E}lqs.sql"), "a<U+202E>lqs.sql");
        assert_eq!(display_name("a\u{0}b\u{7}"), "a<U+0000>b<U+0007>");
        assert_eq!(display_name("x\u{200B}\u{2066}y"), "x<U+200B><U+2066>y");
        assert_eq!(display_name("müşteri_raporu.sql"), "müşteri_raporu.sql");
    }
}
