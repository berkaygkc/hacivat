//! A console tab's decoration (Task 32): the connection name as its title (data, ` (n)` from a
//! profile's second console), the environment swatch, the production hairline.
//!
//! Task 1b stub: no decoration (the kind's title, no mark, no hairline).

use hacivat_ui::panes::TabId;
use hacivat_ui_core::gpui_surface::App;

use crate::shell::TabDecor;

/// The `tab_decor` hook.
pub fn tab_decor(tab: TabId, cx: &App) -> TabDecor {
    let _ = (tab, cx);
    TabDecor::default()
}
