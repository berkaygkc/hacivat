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
pub mod registry;
pub mod status;
pub mod toolbar;
pub mod view;

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::App;

pub use registry::{ConsoleId, ConsoleState, Consoles};
pub use view::{console_binding, editor_segment, focused_console};

use crate::shell::{set_console_binding, set_console_view, set_tab_decor};

/// Installs the console registry and the console hooks.
pub fn install(cx: &mut App) {
    cx.default_global::<Consoles>();
    set_console_view(cx, Rc::new(view::build));
    set_tab_decor(cx, Rc::new(decor::tab_decor));
    // ## m3a-t01b-seams (fix round 1): the profile a new console tab binds to (R-M3-37).
    set_console_binding(cx, Rc::new(view::console_binding));
    commands::register(cx);
}
