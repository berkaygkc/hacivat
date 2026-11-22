//! The script workspace in the app (M4b, L14; `docs/research/algorithms/28-script-workspace.md`,
//! `design/m4b-scripts/`): the Scripts node in the explorer, script tabs bound to the
//! (connection, database, file) they were opened under, save and the bars for changes on disk,
//! Go to script and its connection chooser, the attach and mapping dialogs, the secret warning,
//! git badges, session restore and Run as script.
//!
//! ## m4b-t01b-seams: every module is a fail-closed stub until its owner lands (Global
//! Constraints, "No silent weakening"), and **none of them is wired into a user path**: the
//! [`ScriptsContributor`] is not registered (Task 13 registers it: `## m4b-t13-explorer`, it is
//! now, and fills `scripts.reveal`/`scripts.rename`), [`open_script`] opens
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

use std::rc::Rc;

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
// ## m4b-t13-explorer (named exception): `scripts.reveal` and `scripts.rename` are filled
// (`file_ops::filled`).
// ## m4b-t14-tabs (named exception): `file.save` and `file.save_all` are Task 14's
// (`save::register_commands`).
pub const STUB_COMMANDS: [&str; 2] = ["scripts.goto", "scripts.attach"];

/// A stub command is never enabled.
fn not_yet(_: &App) -> bool {
    false
}

/// Registers the M4b commands and the script-tab table (one line in `crate::init`).
///
/// ## m4b-t13-explorer (named exception): and the Scripts contributor, over the app's library
/// service host (`projection::install_service` opens the service at boot when a library was
/// ever attached); with no service it contributes nothing.
pub fn install(cx: &mut App) {
    cx.set_global(tabs::ScriptTabs::default());
    // ## m4b-t14-tabs (named exception): the script tabs' app-wide parts (the save commands,
    // registered before the stubs so their definitions win; the quit-time recovery copies).
    tabs::install(cx);
    let reg = cx.default_global::<ActionRegistry>();
    // Every command is a stub until its task fills it (and gives it its own run and enabled).
    for (id, title_id, context) in COMMANDS {
        // ## m4b-t13-explorer (named exception): Task 13's filled commands.
        let (run, enabled) = file_ops::filled(id).unwrap_or_else(|| (Rc::new(|_, _| {}), not_yet));
        // A duplicate keeps the first definition (registration order is the palette's order).
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context,
            run,
            enabled,
        });
    }
    // ## m4b-t13-explorer (named lines)
    file_ops::install(cx);
    let contributor = ScriptsContributor::for_app(cx);
    crate::explorer::contrib::register_contributor(Rc::new(contributor), cx);
    // ## m4b-t13-explorer
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

// ---- the core types ------------------------------------------------------------------------
// ## m4b-t01b-seams: the placeholders for Task 1a's core types are replaced by them, as this
// block said the controller would (## m4b-t14-tabs, named exception: Task 14 is the first task
// that hands these values to `hacivat_scripts`).
pub use hacivat_scripts::{DatabaseName, LibraryId, PathError, RelPath};

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
