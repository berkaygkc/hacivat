//! Local sort (HV-GRID-07, R-M3-70; M3c Task 36): the sort job off the UI thread by the column's
//! collation, publishing `ViewMap::Permuted`, the "Sorted locally" banners, and the sort
//! commands.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). The sort commands are registered (palette-only, no
//! keymap row) but disabled, so the palette does not list them and nothing sorts; Task 36
//! fills the bodies and their `enabled`.

use std::rc::Rc;

use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::App;
use hacivat_ui_grid::GRID_CONTEXT;
use hacivat_ui_grid::model::SortSpec;

use crate::console::ConsoleId;

/// The sort commands: `(id, title)`.
const COMMANDS: [(&str, &str); 3] = [
    ("grid.sort_asc", "hv-cmd-grid-sort-asc"),
    ("grid.sort_desc", "hv-cmd-grid-sort-desc"),
    ("grid.clear_sort", "hv-cmd-grid-clear-sort"),
];

/// The grid asked for a sort of console `console`'s active result (the tile calls it on
/// `GridEvent::SortRequested`). STUB: nothing (the source order stays).
pub fn on_requested(console: ConsoleId, spec: SortSpec, cx: &mut App) {
    let _ = (console, spec, cx);
}

/// Registers the sort commands. STUB: disabled, no-op bodies.
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
