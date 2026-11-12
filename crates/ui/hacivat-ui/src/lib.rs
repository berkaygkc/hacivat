//! Hacivat's widgets, keymap data and UI text provider. Reaches GPUI only through
//! `hacivat_ui_core::gpui_surface`; never depends on `crates/core` (§5.1).

pub mod accesskey;
pub mod actions;
pub mod button;
pub mod capsule;
pub mod dialog;
pub mod icon;
pub mod input;
pub mod keymaps;
pub mod l10n;
pub mod menu;
pub mod palette;
pub mod panes;
pub mod popover;
pub mod segmented;
pub mod statusbar;
pub mod text;
pub mod titlebar;
pub mod toast;
