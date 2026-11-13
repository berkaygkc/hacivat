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
//! - **M2b (Task 22).** A reconnect waiting for its next attempt (`Reconnecting` with
//!   `next_in`) gets a deadline on the executor clock when the mirror sees it, so the capsule
//!   counts down to it, repainting once a second (when the shown second changes) while it
//!   waits; the 50 ms clock runs only while an attempt runs. A profile that leaves
//!   `Disconnected` is no longer idle (`session_toasts::clear_idle`). The mirror still awaits
//!   its `watch` channel (Q4 ruling): only the new states are added.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_l10n::{Text, ids};
use hacivat_proto::ProfileId;
use hacivat_session::LiveState;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, Window};

use crate::connections::model::ConnectionsModel;
use crate::connections::{capsule, explorer, home, session_toasts, sheet, status};
use crate::l10n_global::tr;
use crate::shell;

/// How often the clocks repaint while a profile is connecting.
pub const TICK: Duration = Duration::from_millis(50);
// ## m2b-t22-live
/// The longest a reconnect countdown waits between repaints (Task 22 ruling: 1 s).
pub const RECONNECT_TICK: Duration = Duration::from_secs(1);
/// How long the core runtime may take to stop at quit (P15).
const RUNTIME_SHUTDOWN: Duration = Duration::from_millis(200);

/// When each connect in progress started, on the GPUI executor clock (P22).
#[derive(Default)]
struct LiveClocks {
    started: HashMap<ProfileId, Instant>,
    /// The repaint timer runs.
    ticking: bool,
    // ## m2b-t22-live
    /// A reconnect waiting for its next attempt: the attempt, its `next_in`, and when it
    /// starts on the executor clock.
    reconnects: HashMap<ProfileId, (u32, Duration, Instant)>,
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

// ## m2b-t22-live
/// Whole seconds until reconnect `attempt` of `id` starts: from the deadline the mirror noted
/// for that `next_in`, else `next_in` itself (rounded up, so a wait under a second says 1).
pub fn reconnect_seconds(cx: &App, id: ProfileId, attempt: u32, next_in: Duration) -> u64 {
    let left = reconnect_left(cx, id, attempt, next_in);
    u64::try_from(left.as_millis().div_ceil(1000)).unwrap_or(u64::MAX)
}

/// How long until reconnect `attempt` of `id` starts (see [`reconnect_seconds`]).
fn reconnect_left(cx: &App, id: ProfileId, attempt: u32, next_in: Duration) -> Duration {
    let now = cx.background_executor().now();
    cx.try_global::<LiveClocks>()
        .and_then(|c| c.reconnects.get(&id))
        .filter(|(a, n, _)| *a == attempt && *n == next_in)
        .map_or(next_in, |(_, _, at)| at.saturating_duration_since(now))
}

/// Notes when `id`'s next reconnect attempt starts, once per `(attempt, next_in)`.
fn note_reconnect(cx: &mut App, id: ProfileId, attempt: u32, next_in: Duration) {
    let now = cx.background_executor().now();
    let clocks = cx.default_global::<LiveClocks>();
    let same = clocks
        .reconnects
        .get(&id)
        .is_some_and(|(a, n, _)| *a == attempt && *n == next_in);
    if !same {
        clocks
            .reconnects
            .insert(id, (attempt, next_in, now + next_in));
    }
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
        // ## m2b-t22-live: the countdown to the next attempt, and the end of an idle state.
        match state {
            LiveState::Reconnecting {
                attempt,
                next_in: Some(next_in),
            } => {
                connecting = true;
                note_reconnect(cx, id, attempt, next_in);
            }
            _ => {
                cx.default_global::<LiveClocks>().reconnects.remove(&id);
            }
        }
        if state != LiveState::Disconnected {
            session_toasts::clear_idle(cx, id);
        }
        cx.global_mut::<ConnectionsModel>().set_live(id, state);
    }
    if connecting {
        start_ticking(cx);
    }
}

/// How long until the next repaint, while one is needed: [`TICK`] while a connect attempt
/// runs (its clock); while a reconnect waits for its next attempt, until its countdown shows
/// the next whole second, at most [`RECONNECT_TICK`] (Task 22 ruling: a 1 s repaint). `None`
/// when nothing ticks.
fn next_tick(cx: &App) -> Option<Duration> {
    let m = cx.try_global::<ConnectionsModel>()?;
    let mut wait: Option<Duration> = None;
    for p in m.profiles() {
        match m.live(p.id) {
            LiveState::Connecting(_) => return Some(TICK),
            LiveState::Reconnecting {
                attempt,
                next_in: Some(next_in),
            } => {
                let left = reconnect_left(cx, p.id, attempt, next_in);
                let to_next_second = Duration::from_nanos(
                    u64::try_from(left.as_nanos() % RECONNECT_TICK.as_nanos()).unwrap_or(0),
                );
                let step = if to_next_second.is_zero() {
                    RECONNECT_TICK
                } else {
                    to_next_second
                };
                wait = Some(wait.map_or(step, |w| w.min(step)));
            }
            _ => {}
        }
    }
    wait
}

/// The interval [`next_tick`] gives now (test seam, feature `test-support`).
#[cfg(feature = "test-support")]
pub fn repaint_interval(cx: &App) -> Option<Duration> {
    next_tick(cx)
}

/// Repaints while something ticks ([`next_tick`]): every [`TICK`] while a profile is
/// connecting, once a second while a reconnect counts down.
fn start_ticking(cx: &mut App) {
    let clocks = cx.default_global::<LiveClocks>();
    if std::mem::replace(&mut clocks.ticking, true) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            let wait = cx.update(|cx| {
                let wait = next_tick(cx);
                if wait.is_none() {
                    cx.default_global::<LiveClocks>().ticking = false;
                }
                wait
            });
            let Some(wait) = wait else {
                break;
            };
            cx.background_executor().timer(wait).await;
            cx.update(|cx| cx.refresh_windows());
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
