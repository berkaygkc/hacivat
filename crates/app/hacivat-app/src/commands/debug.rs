//! Debug builds only: the programmatic focus move for the IME check (Task 19).
//! `HACIVAT_DEBUG_FOCUS_MOVE_MS=<n>` makes the first IME composition schedule a focus move to
//! the palette's query field `n` ms later, opening the palette when needed.
//!
//! M2b (Task 22, Q39): the cancel demo. M2b has no other way to cancel a statement (B1), so the
//! PO's demo uses two palette commands, compiled into debug builds only (this module is
//! `cfg(debug_assertions)`): `debug.run_pg_sleep` runs `SELECT pg_sleep(30)` on the default
//! session of the capsule's PostgreSQL connection, and `debug.cancel` cancels it
//! (`RequestTicket::cancel`, the session's cancel ladder). A toast says how it ended.

use std::rc::Rc;
use std::time::Duration;

use hacivat_driver::ExecOpts;
use hacivat_l10n::ids;
use hacivat_proto::DbmsKind;
use hacivat_session::LiveState;
use hacivat_session::session::{ExecErrorKind, ExecOrigin, Request, SessionHandle};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::input::FocusedInput;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Global, Window};

use crate::commands::palette::open_palette;
use crate::connections::ConnectionsModel;
use crate::connections::capsule::current_profile;
use crate::l10n_global::tr;
use crate::shell::Workspace;

/// The environment variable that turns the focus move on (milliseconds).
pub const FOCUS_MOVE_ENV: &str = "HACIVAT_DEBUG_FOCUS_MOVE_MS";

/// Whether the focus move still waits for its composition.
struct FocusMoveArmed(bool);

impl Global for FocusMoveArmed {}

/// Installs the focus move from `HACIVAT_DEBUG_FOCUS_MOVE_MS`, when it is set to a number.
pub fn install_from_env(cx: &mut App) {
    let delay = std::env::var(FOCUS_MOVE_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok());
    if let Some(ms) = delay {
        install_focus_move(cx, Duration::from_millis(ms));
    }
}

// ## m2b-t22-live: the cancel demo (Q39).

/// The demo's statement.
const DEMO_SQL: &str = "SELECT pg_sleep(30)";

/// The demo statement in flight: its session and its request's sequence number.
#[derive(Default)]
struct CancelDemo {
    running: Option<(SessionHandle, u64)>,
}

impl Global for CancelDemo {}

/// The default session of the capsule's connection, while it is a live PostgreSQL one.
fn demo_session(cx: &App) -> Option<SessionHandle> {
    let p = current_profile(cx)?;
    let m = cx.try_global::<ConnectionsModel>()?;
    if p.engine != DbmsKind::Postgres || !matches!(m.live(p.id), LiveState::Connected(_)) {
        return None;
    }
    m.manager().sessions(p.id)?.default_session()
}

fn demo_running(cx: &App) -> bool {
    cx.try_global::<CancelDemo>()
        .is_some_and(|d| d.running.is_some())
}

fn can_run(cx: &App) -> bool {
    !demo_running(cx) && demo_session(cx).is_some()
}

/// Runs the demo statement; its reply comes back on a `oneshot` the UI awaits (Q4 ruling) and
/// ends in a toast.
fn run_demo(_window: &mut Window, cx: &mut App) {
    let Some(session) = demo_session(cx).filter(|_| !demo_running(cx)) else {
        return;
    };
    let ticket = session.submit(Request::Exec {
        sql: DEMO_SQL.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    });
    cx.default_global::<CancelDemo>().running = Some((session, ticket.seq));
    let reply = ticket.reply;
    cx.spawn(async move |cx| {
        let outcome = match reply.await {
            Ok(Ok(_)) => ids::hv::HvDebugDemoResultOutcome::Done,
            Ok(Err(e)) if e.kind == ExecErrorKind::Cancelled => {
                ids::hv::HvDebugDemoResultOutcome::Cancelled
            }
            _ => ids::hv::HvDebugDemoResultOutcome::Failed,
        };
        cx.update(|cx| {
            cx.default_global::<CancelDemo>().running = None;
            let text = Rc::new(move |cx: &App| {
                tr(
                    cx,
                    ids::hv::DEBUG_DEMO_RESULT,
                    ids::hv::HvDebugDemoResult { outcome },
                )
            });
            ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
        });
    })
    .detach();
}

/// Cancels the demo statement through the session's cancel ladder (Q18); the statement's own
/// reply then says how it ended.
fn cancel_demo(_window: &mut Window, cx: &mut App) {
    let Some((session, seq)) = cx
        .try_global::<CancelDemo>()
        .and_then(|d| d.running.clone())
    else {
        return;
    };
    let outcome = session.cancel(seq);
    cx.spawn(async move |_| {
        tracing::debug!(outcome = ?outcome.await, "debug: the demo statement's cancel");
    })
    .detach();
}

/// Registers the two demo commands (from `connections::install`, debug builds only).
pub fn register_cancel_demo(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    let _ = reg.register(ActionDef {
        id: "debug.run_pg_sleep",
        title_id: ids::hv::CMD_DEBUG_RUN_PG_SLEEP.id,
        context: "Workspace",
        run: Rc::new(run_demo),
        enabled: can_run,
    });
    let _ = reg.register(ActionDef {
        id: "debug.cancel",
        title_id: ids::hv::CMD_DEBUG_CANCEL.id,
        context: "Workspace",
        run: Rc::new(cancel_demo),
        enabled: demo_running,
    });
}

/// On the first `FocusedInput.composing == true`, moves the focus to the palette's query field
/// `delay` later (opening the palette in the workspace window if it is not open).
pub fn install_focus_move(cx: &mut App, delay: Duration) {
    cx.set_global(FocusMoveArmed(true));
    cx.observe_global::<FocusedInput>(move |cx| {
        let composing = cx.global::<FocusedInput>().composing;
        let armed = cx.try_global::<FocusMoveArmed>().is_some_and(|a| a.0);
        if !(composing && armed) {
            return;
        }
        cx.set_global(FocusMoveArmed(false));
        let timer = cx.background_executor().timer(delay);
        cx.spawn(async move |cx| {
            timer.await;
            cx.update(|cx| {
                let window = cx
                    .active_window()
                    .into_iter()
                    .chain(cx.windows())
                    .find(|w| w.downcast::<Workspace>().is_some());
                if let Some(window) = window {
                    let _ = window.update(cx, |_, window, cx| open_palette(window, cx));
                }
            });
        })
        .detach();
    })
    .detach();
}
