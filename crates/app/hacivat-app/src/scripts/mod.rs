//! The script workspace in the app (M4b, L14; `docs/research/algorithms/28-script-workspace.md`,
//! `design/m4b-scripts/`): the Scripts node in the explorer, script tabs bound to the
//! (connection, database, file) they were opened under, save and the bars for changes on disk,
//! Go to script and its connection chooser, the attach and mapping dialogs, the secret warning,
//! git badges, session restore and Run as script.
//!
//! ## m4b-t01b-seams: every module is a fail-closed stub until its owner lands (Global
//! Constraints, "No silent weakening"), and **none of them is wired into a user path**: the
//! [`ScriptsContributor`] is not registered (Task 13 registers it), [`open_script`] opens
//! nothing, [`database_menu_items`] adds nothing to a menu, and every M4b command is registered
//! **disabled** ([`STUB_COMMANDS`]): a disabled command is not in the palette and its chord
//! passes through, exactly as before M4b. Owners:
//! - [`projection`] (the [`ScriptsContributor`]), [`git_badges`], [`diagnostics`],
//!   [`file_ops`]: Task 13;
//! - [`tabs`], [`open`], [`save`], [`disk_bars`], [`restore`], [`unsaved_prompt`]: Task 14;
//! - [`goto`], [`chooser`]: Task 15; [`attach_dialog`], [`mapping`]: Task 16;
//! - [`secret_marks`], [`secret_dialog`]: Task 17; [`run_as_script`]: Task 18.
//!
//! **Where you open a script decides where it runs** (R-M4b-4): a script tab is a console whose
//! text comes from a file ([`crate::console::ConsoleSource::Script`]), saved in `workspace.json`
//! as its own tab kind ([`crate::shell::TabKind::Script`], R-M4b-29); nothing here rebinds a tab.

pub mod attach_dialog;
pub mod chooser;
pub mod diagnostics;
pub mod disk_bars;
pub mod file_ops;
pub mod git_badges;
pub mod goto;
pub mod mapping;
pub mod open;
pub mod projection;
pub mod restore;
pub mod run_as_script;
pub mod save;
pub mod secret_dialog;
pub mod secret_marks;
pub mod tabs;
pub mod unsaved_prompt;

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui_core::gpui_surface::{App, Global};

use crate::explorer::adapter::NodeInfo;

pub use open::open_script;
pub use projection::ScriptsContributor;
pub use tabs::{ScriptTab, ScriptTabId};

/// The key context of an active script tab (R-M4b-37: `file.save` is bound in it only).
pub const SCRIPT_TAB_CONTEXT: &str = "ScriptTab";

/// The M4b commands, each with its title, the context it is registered in and the task that
/// fills it (R-M4b-37, controller ruling R-T2-1 (c)). Registered disabled until then.
const COMMANDS: [(&str, &str, &str); 6] = [
    // Task 15: Go to script (`cmd-shift-o` / `ctrl-shift-o`).
    ("scripts.goto", "hv-cmd-scripts-goto", "Workspace"),
    // Task 14: save the active script tab (`cmd-s` / `ctrl-s` in `ScriptTab` only) and every
    // dirty one (`cmd-alt-s` on macOS; palette and menu only on Windows and Linux).
    ("file.save", "hv-cmd-file-save", SCRIPT_TAB_CONTEXT),
    ("file.save_all", "hv-cmd-file-save-all", "Workspace"),
    // Task 13: reveal the file in the OS file manager (palette and menu only).
    ("scripts.reveal", "hv-cmd-scripts-reveal", "Workspace"),
    // Task 16: attach a folder as a script library (palette and menu only).
    ("scripts.attach", "hv-cmd-scripts-attach", "Workspace"),
    // Task 13: rename a script file or folder row in the tree (`f2`).
    ("scripts.rename", "hv-cmd-scripts-rename", "ExplorerTree"),
];

/// The M4b commands that are still stubs (registered disabled). A task that fills one removes
/// it here (and lowers the count in `tests/m4b_routing.rs`, a named exception).
pub const STUB_COMMANDS: [&str; 6] = [
    "scripts.goto",
    "file.save",
    "file.save_all",
    "scripts.reveal",
    "scripts.attach",
    "scripts.rename",
];

/// A stub command is never enabled.
fn not_yet(_: &App) -> bool {
    false
}

/// Registers the M4b commands and the script-tab table (one line in `crate::init`). Registers
/// no explorer contributor: the stub contributes nothing, and Task 13 registers the real one.
pub fn install(cx: &mut App) {
    cx.set_global(tabs::ScriptTabs::default());
    let reg = cx.default_global::<ActionRegistry>();
    // Every command is a stub until its task fills it (and gives it its own run and enabled).
    for (id, title_id, context) in COMMANDS {
        // A duplicate keeps the first definition (registration order is the palette's order).
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context,
            run: Rc::new(|_, _| {}),
            enabled: not_yet,
        });
    }
}

// ---- the database menus (R-M4b-34) -------------------------------------------------------------

/// The Scripts items of a database node's menu ("Attach script library…", "Map to a script
/// library…"; NOTES surface 2): appended after M4's items and a separator on a PostgreSQL
/// database row and a MySQL/MariaDB schema row, and placed before the final Delete… on a SQLite
/// profile root (`node` is then [`NodeInfo::Profile`]). Stub: no item, so every M4 menu is
/// unchanged (Task 16 fills it).
pub fn database_menu_items(node: &NodeInfo, cx: &App) -> Vec<MenuItemSpec> {
    match cx.try_global::<DatabaseMenuHook>() {
        Some(hook) => (hook.0)(node, cx),
        None => Vec::new(),
    }
}

/// The items of a database node's menu.
type MenuItemsFn = Rc<dyn Fn(&NodeInfo, &App) -> Vec<MenuItemSpec>>;

/// What [`database_menu_items`] answers instead of the stub (tests only).
#[derive(Clone)]
struct DatabaseMenuHook(MenuItemsFn);

impl Global for DatabaseMenuHook {}

/// Makes [`database_menu_items`] answer `items` (the routing tests drive the menu seam with it).
#[cfg(feature = "test-support")]
pub fn set_database_menu_items_for_tests(
    items: impl Fn(&NodeInfo, &App) -> Vec<MenuItemSpec> + 'static,
    cx: &mut App,
) {
    cx.set_global(DatabaseMenuHook(Rc::new(items)));
}

// ---- names from disk and from `workspace.json` --------------------------------------------------

/// The library ID written as `text` (the hyphenated UUID form `workspace.json` stores); `None`
/// for anything else.
pub fn parse_library_id(text: &str) -> Option<LibraryId> {
    serde_json::from_value(serde_json::Value::String(text.to_string())).ok()
}

/// The last component of `rel` (the file or folder name).
pub fn file_name(rel: &RelPath) -> &str {
    rel.components().last().unwrap_or_default()
}

// ---- app-side placeholders for Task 1a's core types ---------------------------------------------
// ## m4b-t01b-seams: placeholders for `hacivat_scripts::{DatabaseName, LibraryId, PathError,
// RelPath}`, which are not on Task 1a's manifests commit this task starts from. They copy the
// API Task 1a merged into `m4b` (`names.rs`, `pathsafe.rs`, `registry.rs`) and nothing else. The
// controller replaces this block with `pub use hacivat_scripts::{DatabaseName, LibraryId,
// PathError, RelPath};`.

/// A registered library's stable ID (a v4 UUID); serialized as its hyphenated text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LibraryId(pub u128);

impl fmt::Display for LibraryId {
    /// The hyphenated lowercase UUID form, as `uuid::Uuid` writes it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let h = format!("{:032x}", self.0);
        write!(
            f,
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    }
}

impl serde::Serialize for LibraryId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for LibraryId {
    /// The hyphenated UUID form (any ASCII case).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let groups: Vec<&str> = s.split('-').collect();
        let lengths: Vec<usize> = groups.iter().map(|g| g.len()).collect();
        let hex = groups
            .iter()
            .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()));
        if lengths != [8, 4, 4, 4, 12] || !hex {
            return Err(serde::de::Error::custom("not a hyphenated UUID"));
        }
        u128::from_str_radix(&groups.concat(), 16)
            .map(LibraryId)
            .map_err(serde::de::Error::custom)
    }
}

/// A path inside a library, relative to its root, `/`-separated; never joined to a path here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(Arc<str>);

/// Why a path was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    NotImplemented,
    /// Empty, absolute, an empty, `.` or `..` component, a `\`, or a control character.
    Malformed,
}

impl RelPath {
    /// A library-relative path from `/`-separated text, checked structurally.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let well_formed = !text.is_empty()
            && text.split('/').all(|c| {
                !c.is_empty()
                    && c != "."
                    && c != ".."
                    && !c.contains('\\')
                    && !c.chars().any(char::is_control)
            });
        if well_formed {
            Ok(RelPath(text.into()))
        } else {
            Err(PathError::Malformed)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The components, root first.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

/// A database's exact name on its server (`"main"` on SQLite, R-M4b-32; C-M4-4's fallback).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DatabaseName(Arc<str>);

impl DatabaseName {
    /// The server's own name; never validated, never joined to a path.
    pub fn new(exact: &str) -> Self {
        DatabaseName(exact.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_ids_round_trip_in_the_uuid_form() {
        let text = "01234567-89ab-cdef-0123-456789abcdef";
        let id = parse_library_id(text).expect("a library id");
        assert_eq!(id.to_string(), text);
        assert_eq!(
            parse_library_id("01234567-89AB-CDEF-0123-456789ABCDEF"),
            Some(id)
        );
        assert_eq!(
            serde_json::to_value(id).unwrap(),
            serde_json::Value::String(text.to_string())
        );
        for bad in [
            "",
            "lib",
            "01234567-89ab-cdef-0123-456789abcde",
            "01234567-89ab-cdef-0123-456789abcdeg",
            "+1234567-89ab-cdef-0123-456789abcdef",
        ] {
            assert_eq!(parse_library_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rel_paths_keep_only_valid_components() {
        let p = RelPath::parse("reports/haftalık_rapor.sql").expect("valid");
        assert_eq!(p.as_str(), "reports/haftalık_rapor.sql");
        assert_eq!(file_name(&p), "haftalık_rapor.sql");
        assert_eq!(file_name(&RelPath::parse("a.sql").unwrap()), "a.sql");
        for bad in [
            "",
            "/a.sql",
            "a//b.sql",
            "a/",
            "./a.sql",
            "../a.sql",
            "a/../b.sql",
            "a\\b.sql",
            "a\u{0}b.sql",
            "a\nb.sql",
        ] {
            assert_eq!(RelPath::parse(bad), Err(PathError::Malformed), "{bad:?}");
        }
    }
}
