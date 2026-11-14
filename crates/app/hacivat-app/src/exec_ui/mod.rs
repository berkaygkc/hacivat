//! A console's run UI (Task 33): Run resolves the execution units and submits one run; the
//! range chooser; the end-of-line marks; cancel; the transaction segment and the close and quit
//! prompts.
//!
//! Module lines, re-exports and [`install`] are Task 1b's (P12); Task 33 fills the submodules.
//! Task 1b's stubs: Run says it is not available yet and sends nothing; closing a tab and
//! quitting never ask.

pub mod cancel;
pub mod chooser;
pub mod close_prompt;
pub mod marks;
pub mod orchestrator;
pub mod tx;

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::App;

pub use orchestrator::attach;

use crate::shell::{set_quit_guard, set_tab_close_guard};

/// Installs the close and quit guards.
pub fn install(cx: &mut App) {
    set_tab_close_guard(cx, Rc::new(close_prompt::tab_close_guard));
    set_quit_guard(cx, Rc::new(close_prompt::quit_guard));
}
