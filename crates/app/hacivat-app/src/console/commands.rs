//! The editor commands as `ActionDef`s (Task 32): every `hacivat_ui_editor::commands()` entry,
//! context `Editor`, enabled while an editor is focused, run on the `FocusedEditor`; the editor's
//! focus handle registered with `register_focus_context`. The Edit menu then shows them.
//!
//! Task 1b stub: registers nothing (so the Edit menu stays hidden).

use hacivat_ui_core::gpui_surface::App;

/// Registers the editor commands (Task 32).
pub fn register(cx: &mut App) {
    let _ = cx;
}
