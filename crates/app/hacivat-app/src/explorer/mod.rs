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

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;

pub use actions::{
    ActionSlot, ActionSurface, ExplorerActionProvider, NodeRef, OpenDataHandler, ProvidedAction,
    open_data, provided_actions, register_action_provider, set_open_data_handler,
};
pub use adapter::ExplorerModel;
pub use contrib::{
    ContributedChildren, ContributedNode, ExplorerContributor, contributors, register_contributor,
};
pub use live::{DatabaseRow, RevealOutcome, database_rows, reveal};

/// The key context of the focused explorer (the M2a list's, kept by the tree, R20).
pub const EXPLORER_CONTEXT: &str = crate::connections::explorer::EXPLORER_CONTEXT;

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
pub const STUB_COMMANDS: [&str; 12] = [
    "explorer.view_ddl",
    "explorer.copy_qualified_name",
    "explorer.collapse_all",
    "explorer.open_data",
    "explorer.refresh",
    "explorer.expand_all_children",
    "explorer.edit_scope",
    "explorer.set_level.auto",
    "explorer.set_level.names",
    "explorer.set_level.structure",
    "explorer.set_level.sources",
    "metadata.clear_cache",
];

/// A stub command is never enabled.
fn not_yet(_: &App) -> bool {
    false
}

/// Registers the M4 explorer commands (one line in `crate::init`).
pub fn install(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id) in COMMANDS {
        // A duplicate keeps the first definition (registration order is the palette's order).
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: EXPLORER_CONTEXT,
            run: Rc::new(|_, _| {}),
            enabled: not_yet,
        });
    }
}
