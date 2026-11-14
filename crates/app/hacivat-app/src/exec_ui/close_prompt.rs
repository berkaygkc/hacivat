//! The close and quit prompts (Task 33): closing a console with an open user transaction, or
//! quitting with one, asks to commit, roll back or cancel; a failed transaction offers only roll
//! back and cancel (P16). On `Defer` the prompt closes the tab later through
//! `Workspace::force_close_tab`, or quits through `cx.quit()`.
//!
//! Task 1b stub: never asks.

use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::{App, Window};

use crate::shell::CloseDecision;

/// The `tab_close_guard` hook.
pub fn tab_close_guard(tab: TabId, window: &mut Window, cx: &mut App) -> CloseDecision {
    let _ = (tab, window, cx);
    CloseDecision::Proceed
}

/// The `quit_guard` hook.
pub fn quit_guard(cx: &mut App) -> CloseDecision {
    let _ = cx;
    CloseDecision::Proceed
}
