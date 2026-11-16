//! The result views (HV-GRID-12; M3c Task 38): Table / Transposed / Text, and the record
//! inspector, with their commands.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). The view commands are registered (palette-only, no
//! keymap row) but disabled, so the palette does not list them and every result stays a table;
//! Task 38 fills the bodies and their `enabled` (the grid's `GridView::set_transposed` is
//! working already).

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_grid::GRID_CONTEXT;

/// The view commands: `(id, title)`.
const COMMANDS: [(&str, &str); 4] = [
    ("grid.view_table", "hv-cmd-grid-view-table"),
    ("grid.view_transposed", "hv-cmd-grid-view-transposed"),
    ("grid.view_text", "hv-cmd-grid-view-text"),
    ("grid.toggle_record", "hv-cmd-grid-toggle-record"),
];

/// Registers the view commands. STUB: disabled, no-op bodies.
pub fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for (id, title_id) in COMMANDS {
        let _ = reg.register(ActionDef {
            id,
            title_id,
            context: GRID_CONTEXT,
            run: Rc::new(|_, _| {}),
            enabled: |_| false,
        });
    }
}
