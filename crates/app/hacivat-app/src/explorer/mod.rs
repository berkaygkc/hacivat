//! The database explorer tree (M4; design/m4-explorer surfaces 1–6, 8, 9, 11, 12): the M2a
//! profile list becomes the tree's roots (R-M4-18), with databases, schemas, kind groups,
//! objects and their children below them, read from catalog snapshots.
//!
//! ## m4-t01b-seams: every module is a stub until its owner lands (Global Constraints, "No silent
//! weakening"); the explorer on screen is still the M2a list (`connections::explorer`), so no
//! shot changes. Owners:
//! - [`adapter`] (`ExplorerModel`, the `TreeModel` over snapshots), [`rows`], [`badges`],
//!   [`contrib`]: Task 14;
//! - [`live`], [`actions`], [`progress`], [`mass_drop`] and this module's command bodies:
//!   Task 24;
//! - [`level_pill`], [`userdata`]: Task 20; [`refresh_prompt`], [`stale`]: Task 27.
//!
//! **Commands.** Every M4 explorer command is registered here (so its keymap row and its palette
//! title resolve) and stays **disabled** until its task fills it: a disabled command is not in
//! the palette, and its chord passes through, exactly as before M4 ([`STUB_COMMANDS`]).
//!
//! **Extension points (R-M4-19, R-M4-34).** [`ExplorerContributor`] inserts nodes under a
//! database node, above its schemas (M4b's Scripts); [`ExplorerActionProvider`] adds actions to
//! the explorer menu and the DDL view toolbar by slot (M6, M7). M4 registers neither.

pub mod actions;
pub mod adapter;
pub mod badges;
pub mod contrib;
pub mod level_pill;
pub mod live;
pub mod mass_drop;
pub mod progress;
pub mod refresh_prompt;
pub mod rows;
pub mod stale;
pub mod userdata;

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionFn, ActionRegistry};
use hacivat_ui_core::gpui_surface::{App, Window};

pub use actions::{
    ActionSlot, ActionSurface, ExplorerActionProvider, NodeRef, OpenDataHandler, ProvidedAction,
    open_data, provided_actions, register_action_provider, set_open_data_handler,
};
pub use adapter::ExplorerModel;
pub use contrib::{
    ContributedChildren, ContributedNode, ExplorerContributor, contributors, register_contributor,
};
pub use live::{DatabaseRow, RevealOutcome, database_rows, reveal};
// ## m4-t24-live
pub use actions::{node_menu, open_node_menu, qualified_name};

/// The key context of the focused explorer (the M2a list's, kept by the tree, R20).
pub const EXPLORER_CONTEXT: &str = crate::connections::explorer::EXPLORER_CONTEXT;
// ## m4-t14-explorer (fix round 2, ruling R-W2-12b; named exception)
/// The key context of the M4 object-level commands: the focused tree, on every row.
pub const EXPLORER_TREE_CONTEXT: &str = crate::connections::explorer::EXPLORER_TREE_CONTEXT;

/// The M4 explorer commands, each with its title and the task that fills it. Registered
/// disabled (no palette row, the chord passes through) until then.
const COMMANDS: [(&str, &str); 12] = [
    // Task 25 (opens the DDL view; Task 24 puts it in the menu).
    ("explorer.view_ddl", "hv-cmd-explorer-view-ddl"),
    // Task 24.
    (
        "explorer.copy_qualified_name",
        "hv-cmd-explorer-copy-qualified-name",
    ),
    ("explorer.collapse_all", "hv-cmd-explorer-collapse-all"),
    ("explorer.open_data", "hv-cmd-explorer-open-data"),
    ("explorer.refresh", "hv-cmd-explorer-refresh"),
    (
        "explorer.expand_all_children",
        "hv-cmd-explorer-expand-all-children",
    ),
    // Task 20.
    ("explorer.edit_scope", "hv-cmd-explorer-edit-scope"),
    ("explorer.set_level.auto", "hv-cmd-explorer-set-level-auto"),
    (
        "explorer.set_level.names",
        "hv-cmd-explorer-set-level-names",
    ),
    (
        "explorer.set_level.structure",
        "hv-cmd-explorer-set-level-structure",
    ),
    (
        "explorer.set_level.sources",
        "hv-cmd-explorer-set-level-sources",
    ),
    // Task 27.
    ("metadata.clear_cache", "hv-cmd-metadata-clear-cache"),
];

/// The M4 explorer commands that are still stubs (registered disabled). A task that fills one
/// removes it here (and in `tests/m4_routing.rs`).
// ## m4-t20-levels: Task 20's five commands are filled (`edit_scope`, `set_level.*`).
// ## m4-t24-live: Task 24's five are filled (copy qualified name, collapse all, open data,
// refresh, expand all children).
// ## m4-t25-ddl-view (named exception): Task 25 filled `explorer.view_ddl`.
pub const STUB_COMMANDS: [&str; 1] = ["metadata.clear_cache"];

// ## m4-t24-live
/// The node-menu commands Task 24 adds (mock 5): no keymap row (palette and menu only); the
/// Modify… placeholder is never enabled (PO-Q13), so it is never a palette row.
const MENU_COMMANDS: [(&str, &str); 6] = [
    ("explorer.copy_name", "hv-cmd-explorer-copy-name"),
    ("explorer.hide_like_this", "hv-cmd-explorer-hide-like-this"),
    ("explorer.show_hidden", "hv-cmd-explorer-show-hidden"),
    (
        "explorer.collapse_children",
        "hv-cmd-explorer-collapse-children",
    ),
    ("explorer.level_menu", "hv-cmd-explorer-level-menu"),
    (actions::MODIFY, "hv-cmd-explorer-modify"),
];

/// A stub command is never enabled.
fn not_yet(_: &App) -> bool {
    false
}

// ## m4-t20-levels
/// A command's `enabled` function.
type Enabled = fn(&App) -> bool;
// ## m4-t24-live
/// A filled command's body.
type Run = fn(&mut Window, &mut App);

/// The run and enabled functions of a filled command (`None`: a stub).
fn filled(id: &str) -> Option<(ActionFn, Enabled)> {
    // ## m4-t25-ddl-view (named exception): View DDL opens the cursor object's DDL view.
    if id == "explorer.view_ddl" {
        let run: ActionFn = Rc::new(crate::ddl_view::view_ddl_command);
        return Some((run, crate::ddl_view::can_view_ddl));
    }
    if id == "explorer.edit_scope" {
        let run: ActionFn = Rc::new(crate::connections::dialog::schemas::edit_scope);
        return Some((run, crate::connections::dialog::schemas::can_edit_scope));
    }
    // ## m4-t24-live
    let t24: Option<(Run, Enabled)> = match id {
        "explorer.copy_qualified_name" | "explorer.copy_name" => Some((
            if id == "explorer.copy_name" {
                actions::copy_name
            } else {
                actions::copy_qualified_name
            },
            actions::can_copy,
        )),
        "explorer.collapse_all" => Some((actions::collapse_all, actions::can_collapse_all)),
        "explorer.open_data" => Some((actions::open_data_command, actions::can_open_data)),
        "explorer.refresh" => Some((actions::refresh_command, actions::can_refresh)),
        "explorer.expand_all_children" => Some((actions::expand_all_children, actions::has_cursor)),
        "explorer.collapse_children" => Some((actions::collapse_children, actions::has_cursor)),
        "explorer.hide_like_this" => Some((actions::hide_like_this, actions::can_hide)),
        "explorer.show_hidden" => Some((actions::show_hidden, actions::can_show_hidden)),
        "explorer.level_menu" => Some((actions::level_menu, level_pill::can_set_level)),
        actions::MODIFY => Some((|_, _| {}, actions::never)),
        _ => None,
    };
    if let Some((run, enabled)) = t24 {
        let run: ActionFn = Rc::new(run);
        return Some((run, enabled));
    }
    let (_, level) = level_pill::LEVEL_COMMANDS
        .iter()
        .find(|(cmd, _)| *cmd == id)?;
    let level = *level;
    let run: ActionFn = Rc::new(move |_, cx| level_pill::set_level(level, cx));
    Some((run, level_pill::can_set_level))
}

/// Registers the M4 explorer commands (one line in `crate::init`).
pub fn install(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    // ## m4-t24-live: the node-menu commands follow the M4 rows (palette order).
    for (id, title_id) in COMMANDS.into_iter().chain(MENU_COMMANDS) {
        let (run, enabled) = filled(id).unwrap_or_else(|| (Rc::new(|_, _| {}), not_yet));
        // A duplicate keeps the first definition (registration order is the palette's order).
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: EXPLORER_TREE_CONTEXT,
            run,
            enabled,
        });
    }
    // ## m4-t24-live: the default open-data handler (R-M4-13; M6 replaces it).
    actions::install_default_open_data(cx);
}
