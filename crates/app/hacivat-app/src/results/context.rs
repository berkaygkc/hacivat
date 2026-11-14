//! The status-bar context of the active result tab (Task 26): its connection, environment and
//! access, the rows loaded, timings and completeness.
//!
//! The context is that of the **focused console's active result tab**; while the Output tab is
//! active, or the console is unbound, there is none and the status bar keeps the console's own
//! segments. Everything is read from state already in memory (the console registry, the
//! connections model's live mirror, one snapshot load): no I/O.

use hacivat_proto::{AccessLabel, EnvTag, ProfileId};
use hacivat_results::Completeness;
use hacivat_session::LiveState;
use hacivat_ui_core::gpui_surface::{App, Entity, Window};

use crate::connections::{ConnectionsModel, session_toasts};
use crate::console::{ConsoleId, Consoles, focused_console};
use crate::results::tabs::ConsoleResults;

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

impl From<&Completeness> for CompletenessView {
    fn from(c: &Completeness) -> Self {
        match c {
            Completeness::Streaming => CompletenessView::Streaming,
            Completeness::Suspended(_) => CompletenessView::Suspended,
            Completeness::Complete => CompletenessView::Complete,
            Completeness::Closed { .. } => CompletenessView::Closed,
            Completeness::Cancelled => CompletenessView::Cancelled,
            Completeness::Truncated(_) => CompletenessView::Truncated,
            Completeness::Released => CompletenessView::Released,
        }
    }
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

/// The context of the focused console's active result tab.
pub fn active_result_context(window: &Window, cx: &App) -> Option<ResultContext> {
    result_context_of(focused_console(window, cx)?, cx)
}

/// The context of `console`'s active result tab: `None` while its Output tab is active, while
/// it has no results, or when it is not bound to a known connection.
///
/// **Access fails closed.** While the profile is connected, the access is the live
/// connection's honest label (`session_toasts::effective_access`), and the tab records it.
/// In every other state (reconnecting, disconnected, lost, failed, connecting) the tab's
/// recorded label is shown: a drop never upgrades the label. A tab with no recorded label
/// reads `ReadOnlyGuarded`, whatever its profile: nothing more than read-only is claimed
/// about a connection that was never seen, and a result without a connection cannot write.
pub fn result_context_of(console: ConsoleId, cx: &App) -> Option<ResultContext> {
    let state = Consoles::get(cx, console)?;
    let profile = state.profile?;
    let results = state.results.as_ref()?.read(cx);
    let tab = results.active_tab()?;
    let model = cx.try_global::<ConnectionsModel>()?;
    let p = model.get(profile)?;
    let access = match model.live(profile) {
        LiveState::Connected(info) => {
            let access = session_toasts::effective_access(cx, profile, info.access);
            tab.note_access(access);
            access
        }
        _ => tab.last_access().unwrap_or(AccessLabel::ReadOnlyGuarded),
    };
    let snapshot = tab.adapter().snapshot();
    let loaded = snapshot.row_count();
    let completeness = CompletenessView::from(snapshot.completeness());
    let (exec_ms, fetch_ms) = tab.timing();
    Some(ResultContext {
        profile,
        env: p.environment,
        access,
        loaded,
        exact: (completeness == CompletenessView::Complete).then_some(loaded),
        exec_ms,
        fetch_ms,
        completeness,
    })
}

/// The access label of the live connection of the console that owns `results`; `None` when no
/// console owns them, it is unbound, or its profile is not connected.
pub(crate) fn live_access_of(results: &Entity<ConsoleResults>, cx: &App) -> Option<AccessLabel> {
    let profile = Consoles::ids(cx).into_iter().find_map(|id| {
        let state = Consoles::get(cx, id)?;
        (state.results.as_ref() == Some(results))
            .then_some(state.profile)
            .flatten()
    })?;
    match cx.try_global::<ConnectionsModel>()?.live(profile) {
        LiveState::Connected(info) => {
            Some(session_toasts::effective_access(cx, profile, info.access))
        }
        _ => None,
    }
}
