//! The harman workspace, one module per region.

pub mod data;
pub mod editor;
pub mod explorer;
pub mod grid;
pub mod inspector;
pub mod statusbar;
pub mod titlebar;
pub mod ui;

use hacivat_ui_core::gpui_surface::*;

use ui::{Ui, col, cp_fill_count, reset_cp_fill_count, row};

/// Logical window size of the mock.
pub const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The whole 1440 × 900 workspace for the given UI context. `advance` is the editor font's
/// glyph advance (used to anchor the caret and the completion popup).
pub fn workspace(ui: &Ui, advance: f32) -> AnyElement {
    reset_cp_fill_count();
    let c = &ui.c;
    let left = col()
        .w(px(276.0))
        .h_full()
        .flex_none()
        .gap(px(1.0))
        .bg(c.seam.to_hsla())
        .child(div().flex_1().min_h_0().overflow_hidden().child(explorer::explorer(ui)))
        .child(div().h(px(205.0)).flex_none().overflow_hidden().child(explorer::connection_sheet(ui)));
    let center = col()
        .flex_1()
        .min_w_0()
        .h_full()
        .gap(px(1.0))
        .bg(c.seam.to_hsla())
        .child(div().h(px(322.0)).flex_none().child(editor::editor(ui, advance)))
        .child(div().flex_1().min_h_0().overflow_hidden().child(grid::results(ui)));
    let tiles = row()
        .w_full()
        .flex_1()
        .min_h_0()
        .items_start()
        .gap(px(1.0))
        .bg(c.seam.to_hsla())
        .child(left)
        .child(center);
    let root = col()
        .size_full()
        .bg(c.ground.to_hsla())
        .child(titlebar::titlebar(ui))
        .child(tiles)
        .child(statusbar::statusbar(ui));
    debug_assert_eq!(cp_fill_count(), 1, "exactly one cp-fill quad per frame");
    root.into_any_element()
}
