//! Hacivat's widgets, keymap data and UI text provider. Reaches GPUI only through
//! `hacivat_ui_core::gpui_surface`; never depends on `crates/core` (§5.1).

pub mod accesskey;
pub mod actions;
pub mod button;
pub mod capsule;
pub mod checkbox;
pub mod choice_list;
pub mod dialog;
pub mod disclosure;
pub mod explain;
// ## m2b-t24-shots (fix round 1, I2): scroll containers that follow the focus.
pub mod follow;
pub mod form;
pub mod icon;
pub mod input;
pub mod keymaps;
pub mod l10n;
pub mod menu;
pub mod notice;
pub mod palette;
pub mod panes;
pub mod popover;
pub mod segmented;
pub mod select;
pub mod statusbar;
pub mod step_trail;
pub mod text;
pub mod titlebar;
pub mod toast;
// ## m3a-t01b-seams: the Overflow Rule toolbar (Task 21).
pub mod toolbar;
