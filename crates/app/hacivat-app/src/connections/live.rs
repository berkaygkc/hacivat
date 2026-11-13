//! The live mirror: copies `ConnectionManager` states into the model and sets every shell hook
//! (explorer, connection panel, editor empty state, capsule, status bar, Escape) (Task 18).
//!
//! - **Mirror.** A foreground task awaits `ConnectionManager::changes()` (a `tokio::sync::watch`,
//!   never I/O) and copies every profile's state into `ConnectionsModel::set_live`. Every view
//!   that shows a live state reads the model, so the capsule, the status bar, the explorer and
//!   the sheet agree.
//! - **Clocks (P22).** A connect's clock starts at the GPUI executor's `now()` when the attempt
//!   starts (never `Progress.started`). While any profile is connecting a 50 ms timer repaints;
//!   ticking clocks are the only motion.
//! - **Escape.** With no layer open, Escape cancels the current connect and toasts.
//! - **Quit (P15).** GPUI gives quit futures 200 ms, so the quit handler blocks, bounded by the
//!   manager's `shutdown_budget`: the queued secret jobs and profile writes drain first (at most
//!   half the budget), then `ConnectionManager::shutdown_blocking` closes every connection (and
//!   cancels connects) within the rest, then the core runtime stops with its own 200 ms budget.
//!   This is the only place profile writes may block the UI thread.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_l10n::{Text, ids};
use hacivat_proto::ProfileId;
use hacivat_session::LiveState;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, Window};

use crate::connections::model::ConnectionsModel;
use crate::connections::{capsule, explorer, home, sheet, status};
use crate::l10n_global::tr;
use crate::shell;

/// How often the clocks repaint while a profile is connecting.
pub const TICK: Duration = Duration::from_millis(50);
/// How long the core runtime may take to stop at quit (P15).
const RUNTIME_SHUTDOWN: Duration = Duration::from_millis(200);

/// When each connect in progress started, on the GPUI executor clock (P22).
#[derive(Default)]
struct LiveClocks {
    started: HashMap<ProfileId, Instant>,
    /// The repaint timer runs.
    ticking: bool,
}

impl Global for LiveClocks {}

/// Starts the mirror task and sets the shell hooks (Task 18). Called from `connections::boot`
/// after the model is set (P17); without a model it does nothing.
pub fn install(cx: &mut App) {
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let mut changes = model.manager().changes();
    cx.set_global(LiveClocks::default());

    shell::set_explorer(cx, Rc::new(explorer::build));
    shell::set_connection_panel(cx, Rc::new(sheet::build));
    shell::set_editor_empty(cx, Rc::new(home::element));
    shell::set_capsule(cx, Rc::new(capsule::model));
    shell::set_capsule_segment(cx, Rc::new(capsule::on_segment));
    shell::set_status_left(cx, Rc::new(status::segments));
    shell::set_escape(cx, Rc::new(escape));

    // The capsule and the status bar are drawn by the workspace: repaint it on every change.
    cx.observe_global::<ConnectionsModel>(|cx| cx.refresh_windows())
        .detach();
    cx.spawn(async move |cx| {
        while changes.changed().await.is_ok() {
            cx.update(mirror);
        }
    })
    .detach();
    cx.on_app_quit(|cx| {
        quit(cx);
        async {}
    })
    .detach();
}

/// Marks `id`'s connect as started now (the capsule clock's zero): the user started an
/// attempt, so an older start never carries over.
pub fn note_started(cx: &mut App, id: ProfileId) {
    let now = cx.background_executor().now();
    cx.default_global::<LiveClocks>().started.insert(id, now);
}

/// Starts `id`'s clock unless it already runs (the mirror saw a connect it did not start).
fn ensure_started(cx: &mut App, id: ProfileId) {
    let now = cx.background_executor().now();
    cx.default_global::<LiveClocks>()
        .started
        .entry(id)
        .or_insert(now);
}

/// Seconds since `id`'s connect started (0 when it is not running).
pub fn clock_seconds(cx: &App, id: ProfileId) -> f64 {
    let now = cx.background_executor().now();
    cx.try_global::<LiveClocks>()
        .and_then(|c| c.started.get(&id))
        .map_or(0.0, |start| {
            now.saturating_duration_since(*start).as_secs_f64()
        })
}

/// Copies every profile's manager state into the model.
pub(crate) fn mirror(cx: &mut App) {
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let states: Vec<(ProfileId, LiveState)> = m
        .profiles()
        .iter()
        .map(|p| (p.id, m.manager().state(p.id)))
        .filter(|(id, state)| m.live(*id) != *state)
        .collect();
    if states.is_empty() {
        return;
    }
    let mut connecting = false;
    for (id, state) in states {
        if matches!(state, LiveState::Connecting(_)) {
            connecting = true;
            ensure_started(cx, id);
        } else {
            cx.default_global::<LiveClocks>().started.remove(&id);
        }
        cx.global_mut::<ConnectionsModel>().set_live(id, state);
    }
    if connecting {
        start_ticking(cx);
    }
}

/// Whether any profile is connecting.
fn any_connecting(cx: &App) -> bool {
    cx.try_global::<ConnectionsModel>().is_some_and(|m| {
        m.profiles()
            .iter()
            .any(|p| matches!(m.live(p.id), LiveState::Connecting(_)))
    })
}

/// Repaints every [`TICK`] while a profile is connecting.
fn start_ticking(cx: &mut App) {
    let clocks = cx.default_global::<LiveClocks>();
    if std::mem::replace(&mut clocks.ticking, true) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let keep = cx.update(|cx| {
                let keep = any_connecting(cx);
                if keep {
                    cx.refresh_windows();
                } else {
                    cx.default_global::<LiveClocks>().ticking = false;
                }
                keep
            });
            if !keep {
                break;
            }
        }
    })
    .detach();
}

/// The connect Escape cancels: the capsule's profile when it is connecting, else any.
fn connecting_profile(cx: &App) -> Option<ProfileId> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let running = |id: &ProfileId| matches!(m.manager().state(*id), LiveState::Connecting(_));
    capsule::current_profile(cx)
        .map(|p| p.id)
        .filter(running)
        .or_else(|| m.profiles().iter().map(|p| p.id).find(running))
}

/// The `escape` hook: cancels the connect in progress, confirms with a toast and consumes the
/// key; without one it lets the key go on.
fn escape(_window: &mut Window, cx: &mut App) -> bool {
    let Some(id) = connecting_profile(cx) else {
        return false;
    };
    let name = cx
        .global::<ConnectionsModel>()
        .get(id)
        .map(|p| p.name.clone())
        .unwrap_or_default();
    ConnectionsModel::cancel_connect(cx, id);
    mirror(cx);
    let text = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::CONN_TOAST_CANCELLED,
            ids::hv::HvConnToastCancelled {
                name: Text(name.clone()),
            },
        )
    });
    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
    true
}

/// The bounded shutdown at quit (P15): see the module docs.
fn quit(cx: &mut App) {
    let Some(m) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let budget = m.manager().shutdown_budget();
    let started = Instant::now();
    // Profile and secret writes first, with at most half the budget, so a slow close never
    // starves a keyring write; then the connections get what is left.
    let drained = m.drain_writes(budget / 2);
    let closed = m
        .manager()
        .shutdown_blocking(budget.saturating_sub(started.elapsed()));
    m.core_runtime().shutdown(RUNTIME_SHUTDOWN);
    tracing::debug!(closed, drained, "connections: shut down");
}
