//! Consoles (Task 32): a console tab bound to a connection, its editor, toolbar, production
//! decor, autosave, the editor status segment and the editor commands.
//!
//! Module lines, re-exports and [`install`] are Task 1b's (P12); Task 32 fills the submodules.
//! Task 1b's stubs: a console tab shows today's M2 home, or the editor of a console registered
//! for its tab (`register_test_console`, so Tasks 33 and 34 test without Task 32); no decor;
//! no commands; no editor segment; a new console tab opens unbound (`console_binding`).

pub mod autosave;
pub mod commands;
pub mod decor;
// ## m3b-t01b-seams: the editor's find bar (Task 33).
pub mod find_bar;
// ## m3b-t01b-seams: the idle-transaction toast (Task 22).
pub mod idle_toast;
pub mod registry;
pub mod status;
pub mod toolbar;
// ## m3b-t01b-seams: the transaction badge and chip (Task 22).
pub mod tx_badge;
pub mod tx_chip;
pub mod view;

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::App;

pub use registry::{ConsoleId, ConsoleState, Consoles};
pub use view::{console_binding, editor_segment, focused_console};
// ## m3c-t41-chrome
pub use view::editor_segment_cells;

use crate::shell::{set_console_binding, set_console_view, set_tab_decor};

/// Installs the console registry and the console hooks.
pub fn install(cx: &mut App) {
    cx.default_global::<Consoles>();
    set_console_view(cx, Rc::new(view::build));
    set_tab_decor(cx, Rc::new(decor::tab_decor));
    // ## m3a-t01b-seams (fix round 1): the profile a new console tab binds to (R-M3-37).
    set_console_binding(cx, Rc::new(view::console_binding));
    commands::register(cx);
    // ## m3b-t22-tx-ui (named exception): the transaction mirror and the idle reminders.
    tx_badge::install(cx);
    idle_toast::install(cx);
}
