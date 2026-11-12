//! Connections in the shell (M2a): the profile model, the connection dialog, the explorer
//! list, the live capsule and status bar, test and error surfaces. Signatures since Task 1;
//! each module names its owner task.
//!
//! [`install`] registers actions only, so an M1 shell without a [`ConnectionsBoot`] renders
//! exactly as in M1. [`boot`] sets the model and the shell hooks; `shell::start` calls it before
//! the workspace opens (P17).

pub mod boot;
pub mod capsule;
pub mod commands;
pub mod delete;
pub mod dialog;
pub mod error_view;
pub mod explorer;
pub mod fixes;
pub mod home;
pub mod import;
pub mod interaction;
pub mod live;
pub mod model;
pub mod password_prompt;
pub mod prod_prompt;
pub mod report;
pub mod sheet;
pub mod status;
pub mod test_strip;

pub use boot::ConnectionsBoot;
pub use model::{ConnectionsModel, SaveError};

use hacivat_ui_core::gpui_surface::App;

/// Registers the connection actions (called from `crate::init`). Actions only, never hooks.
pub fn install(_cx: &mut App) {
    // ## registrations (append-only: Tasks 17, 18 and 20 add one line each)
    commands::register(_cx); // ## m2a-t18-explorer
}

/// Boots the connections: the `ConnectionsModel` Global and its store diagnostics (Task 12),
/// then the mirror task and every shell hook (Task 18). Every hook view tolerates a missing
/// model by rendering the M1 element (P17).
pub fn boot(cx: &mut App, boot: ConnectionsBoot) {
    boot::install(cx, boot);
    live::install(cx);
}
