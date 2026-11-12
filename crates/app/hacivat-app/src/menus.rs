//! The application menu model, built from the action registry: macOS `set_menus` and the
//! Windows/Linux in-window menu (F14). Task 16.

use hacivat_ui_core::gpui_surface::App;

/// Rebuilds the menus after a language or keymap change (F15). Stub: no-op until Task 16.
pub fn rebuild(cx: &mut App) {
    let _ = cx;
}
