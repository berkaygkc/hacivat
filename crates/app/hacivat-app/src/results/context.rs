//! The status-bar context of the active result tab (Task 26): its connection, environment and
//! access, the rows loaded, timings and completeness.
//!
//! Task 1b stub: no context (the status bar keeps its M2 segments).

use hacivat_proto::{AccessLabel, EnvTag, ProfileId};
use hacivat_ui_core::gpui_surface::{App, Window};

/// How far a result got, as the status bar and the cursor strip show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompletenessView {
    Streaming,
    Suspended,
    Complete,
    Closed,
    Cancelled,
    Truncated,
    Released,
}

/// The active result tab's context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultContext {
    pub profile: ProfileId,
    pub env: Option<EnvTag>,
    pub access: AccessLabel,
    pub loaded: u64,
    pub exact: Option<u64>,
    pub exec_ms: Option<u64>,
    pub fetch_ms: Option<u64>,
    pub completeness: CompletenessView,
}

/// The context of the focused console's active result tab. Task 1b stub: `None`.
pub fn active_result_context(window: &Window, cx: &App) -> Option<ResultContext> {
    let _ = (window, cx);
    None
}
