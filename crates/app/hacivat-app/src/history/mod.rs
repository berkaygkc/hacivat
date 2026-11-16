//! The query history dialog (HV-EXEC-12 UI, D-061; M3b Task 27, `design/m3b-safe`): search,
//! filter by connection and outcome, pin, and insert a statement at the caret.
//!
//! ## m3b-t01b-seams: module lines, `install` and the stubs (Task 1b). `history.open` is
//! registered (chord ⌘⇧H / Ctrl+Shift+H, context `Workspace`) but disabled, so the key passes
//! through and the palette does not list it; [`open`] opens nothing. Task 27 fills `dialog`
//! and `rows`, [`open`] and the command's `enabled`.

pub mod dialog;
pub mod rows;

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;

/// Registers `history.open`.
pub fn install(cx: &mut App) {
    let _ = cx.default_global::<ActionRegistry>().register(ActionDef {
        id: "history.open",
        title_id: "hv-cmd-history-open",
        context: "Workspace",
        run: Rc::new(|_, cx| open(cx)),
        enabled: can_open,
    });
}

/// Opens the history dialog over the focused console. STUB: nothing.
pub fn open(cx: &mut App) {
    let _ = cx;
}

/// Whether `history.open` can run. STUB: `false`.
fn can_open(cx: &App) -> bool {
    let _ = cx;
    false
}
