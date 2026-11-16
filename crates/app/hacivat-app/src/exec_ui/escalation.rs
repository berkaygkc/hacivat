//! Cancel escalation (HV-EXEC-09; M3b Task 23, `design/m3b-safe` surface 6): a cancel that is
//! slow to land offers Terminate (and Deactivate where the engine has it); Escape never sends
//! either (R-M3-38).
//!
//! ## m3b-t01b-seams: the seams (Task 1b). Task 23 fills them.
//!
//! ## m3b-t23-escalation
//!
//! - **Following a cancel.** `exec.cancel` ([`follow`], before the cancel goes out) follows the
//!   cancelled request: the session's `SessionEvent::SlowCancel` for it (10 s), and the request's
//!   end, which takes the toast and the dialog away. Nothing escalates by itself.
//! - **The slow-cancel toast** ([`on_slow_cancel`]): the wide sticky toast with the hourglass,
//!   "Cancelling is taking long" / "The server hasn't stopped the statement on orders-dev after
//!   11 s.", and one Default button, **Show options…**, which opens the dialog. The toast never
//!   terminates or disconnects in one click: the consequences have one source, the dialog. The
//!   unit in flight shows the amber hourglass (`ExecMarkState::Escalated`, "cancel not answered
//!   · 14 s", once a second; Task 34 paints the glyph).
//! - **The dialog** ([`open_dialog`]: a second `exec.cancel` while cancelling, or Show
//!   options…): a 420 px alertdialog, "The statement isn't answering the cancel", the clock
//!   (once a second), then each action's consequence before the action: Terminate on server
//!   (only where the engine can end a session from a side connection: PostgreSQL, MySQL,
//!   MariaDB; the role may always end its own session) and Disconnect, worded from R-M3-73: the
//!   profile's default console closes every connection of the profile (the session's
//!   `Escalation::Deactivate` breaks the default session, and `ConnectionManager` ends the
//!   profile on a `Broken(cancelled)` default session), naming each other console's open
//!   transaction (`SessionManager::open_transactions`, consoles with a request in flight
//!   included, worded "may"); any other console closes only its own session. **Keep waiting is
//!   Primary and has the focus**; Disconnect is Danger, alone on the far left. The word
//!   "Deactivate" appears nowhere in the UI.
//! - **Escape, ⌘W and the close button answer Keep waiting**: they never send Terminate or
//!   Deactivate. Enter acts on the focused button only, on key down.
//! - **A failed terminate** keeps the dialog: the ink alert line and the server's own error in
//!   its own box, verbatim (`SessionEvent::CancelFailed`, queued before the escalation's reply);
//!   the focus returns to Keep waiting.
//! - **Afterwards** the console's Output says what happened: "Session terminated on the
//!   server…" (octagon-x), "Couldn't terminate…", "Disconnected orders-dev. The statement may
//!   still be running on the server." (unplug).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{ServerMessage, TxStatus};
use hacivat_exec::LogMsg;
use hacivat_l10n::ArgValue;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::profile::ProfileEndpoint;
use hacivat_session::session::{
    CancelOutcome, Escalation, SessionEvent, SessionHandle, SessionState, TxMode, TxOwner,
};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::DialogSize;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, ClickEvent, Context, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString, Styled, Task, WeakEntity,
    Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::{ExecMarkState, MarkKey};
use tokio::sync::broadcast::error::RecvError;

use super::confirm::{closes_dialog, console_facts, grouped_footer, scroll_body, subtitle};
use super::marks::{clock_text, message};
use super::{console_exec, in_workspace_window};
use crate::connections::ConnectionsModel;
use crate::connections::prompt_kit::{
    acting_enter, card, cycle_focus, header, inset, push_modal, register_part, remove_modal,
    trapped_tab, visible,
};
use crate::console::tx_badge::TxView;
use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::orchestrator::profile_of;

/// The dialog's layer ID.
pub const ESCALATION_LAYER_ID: &str = "cancel-escalation";

/// At most this many characters of the server's text are drawn.
const SERVER_TEXT_MAX_CHARS: usize = 2_000;

/// At most this many characters of a name are drawn.
const NAME_MAX_CHARS: usize = 200;

// ---- following a cancel -------------------------------------------------------------------

/// One console's cancel in flight, as the escalation follows it.
struct Cancel {
    req: u64,
    session: SessionHandle,
    /// When the cancel was asked (executor clock): the dialog's and the gutter's clock.
    since: Instant,
    /// The slow-cancel toast, once shown.
    toast: Option<u64>,
    /// The session said the cancel is slow: the unit in flight shows the hourglass.
    slow: bool,
    dialog: Option<WeakEntity<EscalationDialog>>,
    // ## m3b-t23-escalation (fix round 1, I1)
    /// A terminate went out and its outcome is not known yet: the statements of this console's
    /// open transaction it would roll back (for the "terminated" line).
    terminate: Option<Option<u32>>,
    _tasks: (Task<()>, Task<()>),
}

/// The cancels the escalation follows, by console.
#[derive(Default)]
struct Escalations {
    cancels: HashMap<ConsoleId, Cancel>,
}

impl hacivat_ui_core::gpui_surface::Global for Escalations {}

/// `exec.cancel` is about to cancel request `req` of console `console` (Task 33's `cancel`):
/// follows its slow cancel and its end. A request already followed is left as it is.
pub(crate) fn follow(console: ConsoleId, req: u64, session: &SessionHandle, cx: &mut App) {
    if cx
        .try_global::<Escalations>()
        .and_then(|e| e.cancels.get(&console))
        .is_some_and(|c| c.req == req)
    {
        return;
    }
    let mut events = session.events();
    let mut state = session.state();
    // Awaits `tokio::sync` receivers only (Q4).
    let slow = cx.spawn(async move |cx| {
        loop {
            match events.recv().await {
                Ok(SessionEvent::SlowCancel { req: r }) if r == req => {
                    cx.update(|cx| on_slow_cancel(console, cx));
                }
                // Fix round 1 (I1): a terminate's refusal, however late it comes.
                Ok(SessionEvent::CancelFailed { req: r, server }) if r == req => {
                    cx.update(|cx| terminate_refused(console, req, server, cx));
                }
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        }
    });
    let ends = cx.spawn(async move |cx| {
        // The state the request left the session in: a terminate that ended it leaves no
        // working connection (broken, or reconnecting), a cancel that landed leaves it idle.
        let left = state
            .wait_for(|s| {
                !matches!(
                    s.state,
                    SessionState::Busy { req: r, .. } | SessionState::Cancelling { req: r, .. }
                        if r == req
                )
            })
            .await
            .map(|s| s.state.clone())
            .ok();
        cx.update(|cx| {
            let ended_by_terminate =
                !matches!(left, Some(SessionState::Idle | SessionState::Busy { .. }));
            finish(console, req, ended_by_terminate, cx);
        });
    });
    let since = cx.background_executor().now();
    let old = cx.default_global::<Escalations>().cancels.insert(
        console,
        Cancel {
            req,
            session: session.clone(),
            since,
            toast: None,
            slow: false,
            dialog: None,
            terminate: None,
            _tasks: (slow, ends),
        },
    );
    if let Some(old) = old {
        drop_cancel(old, cx);
    }
}

/// Whether console `console` cancels request `req` (a second `exec.cancel` then escalates).
pub(crate) fn cancelling(console: ConsoleId, req: u64, cx: &App) -> bool {
    cx.try_global::<Escalations>()
        .and_then(|e| e.cancels.get(&console))
        .is_some_and(|c| c.req == req)
}

/// Request `req` of console `console` ended: its toast and its dialog go. A terminate still
/// waiting for its outcome ended it when the session lost its connection with it
/// (`by_terminate`): its line is written now (fix round 1, I1: a late success too).
fn finish(console: ConsoleId, req: u64, by_terminate: bool, cx: &mut App) {
    let ended = {
        let e = cx.default_global::<Escalations>();
        if e.cancels.get(&console).is_some_and(|c| c.req == req) {
            e.cancels.remove(&console)
        } else {
            None
        }
    };
    if let Some(c) = ended {
        if let Some(statements) = c.terminate.filter(|_| by_terminate) {
            log(console, LogMsg::Terminated { statements }, cx);
        }
        drop_cancel(c, cx);
    }
}

// ## m3b-t23-escalation (fix round 1, I1)
/// The terminate of request `req` of console `console` was refused (its reply, or the
/// session's `CancelFailed`, in either order): its line once, and the dialog shows the server's
/// text (when one came) with the focus back on Keep waiting.
fn terminate_refused(console: ConsoleId, req: u64, server: Option<ServerMessage>, cx: &mut App) {
    let Some(c) = cx
        .default_global::<Escalations>()
        .cancels
        .get_mut(&console)
        .filter(|c| c.req == req)
    else {
        return;
    };
    let first = c.terminate.take().is_some();
    let dialog = c.dialog.as_ref().and_then(|d| d.upgrade());
    if first {
        log(console, LogMsg::TerminateFailed, cx);
    }
    let Some(dialog) = dialog else {
        return;
    };
    dialog.update(cx, |d, cx| {
        let known = d.refused.take().and_then(|r| r.server);
        d.refused = Some(Refused {
            server: server.or(known),
        });
        cx.notify();
    });
    if first {
        let keep = dialog.read(cx).keep.clone();
        in_workspace_window(cx, move |_, window, cx| window.focus(&keep, cx));
    }
}

fn drop_cancel(c: Cancel, cx: &mut App) {
    if let Some(toast) = c.toast {
        ToastQueue::dismiss(cx, toast);
    }
    if let Some(view) = c.dialog.and_then(|d| d.upgrade()) {
        let id = view.entity_id();
        in_workspace_window(cx, move |_, window, cx| remove_modal(id, window, cx));
    }
}

/// The session reported a slow cancel on console `console` (`SessionEvent::SlowCancel`): the
/// toast (unless the dialog is open), and the hourglass on the unit in flight.
pub fn on_slow_cancel(console: ConsoleId, cx: &mut App) {
    let Some((since, shown)) = cx
        .try_global::<Escalations>()
        .and_then(|e| e.cancels.get(&console))
        .map(|c| {
            let dialog = c.dialog.as_ref().and_then(|d| d.upgrade()).is_some();
            (c.since, c.toast.is_some() || dialog)
        })
    else {
        return;
    };
    if let Some(c) = cx.default_global::<Escalations>().cancels.get_mut(&console) {
        c.slow = true;
    }
    mark_escalated(console, since, cx);
    if shown {
        return;
    }
    let elapsed = cx
        .background_executor()
        .now()
        .saturating_duration_since(since);
    let name = super::orchestrator::console_name(cx, console);
    let toast = ToastQueue::push_id(cx, slow_toast(console, name, elapsed));
    if let Some(c) = cx.default_global::<Escalations>().cancels.get_mut(&console) {
        c.toast = Some(toast);
    }
    #[cfg(feature = "test-support")]
    cx.default_global::<Record>().toasts.push((console, toast));
}

/// The slow-cancel toast of console `console` (connection `name`, cancelling for `elapsed`).
fn slow_toast(console: ConsoleId, name: String, elapsed: Duration) -> Toast {
    let ms = millis(elapsed);
    let body: TextFn = Rc::new(move |cx: &App| {
        message(
            cx,
            "hv-cancel-slow-body",
            vec![
                ("name", ArgValue::Text(visible(&name, NAME_MAX_CHARS))),
                ("duration", ArgValue::Text(clock_text(cx, ms))),
            ],
        )
    });
    let mut toast = Toast::new(ToastKind::Info, body);
    toast.sticky = true;
    toast.wide = true;
    toast.title = Some(Rc::new(|cx: &App| {
        message(cx, "hv-cancel-slow-title", vec![])
    }));
    toast.icon = Some(IconName::Hourglass);
    toast.actions = vec![ToastAction {
        label: Rc::new(|cx: &App| message(cx, "hv-cancel-options", vec![])),
        // It only opens the dialog (the action dismisses the toast after it runs).
        run: Rc::new(move |window, cx| open_dialog(console, window, cx)),
    }];
    toast
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// The end-of-line words of a cancel not answered since `since` (read on every repaint).
fn escalated_eol(since: Instant) -> TextFn {
    Rc::new(move |cx: &App| {
        let ms = millis(
            cx.background_executor()
                .now()
                .saturating_duration_since(since),
        );
        message(
            cx,
            "hv-editor-mark-escalated",
            vec![("duration", ArgValue::Text(clock_text(cx, ms)))],
        )
    })
}

/// The mark the unit in flight of console `console` shows while its slow cancel waits (Task 33's
/// status follower keeps it), or `None` when no slow cancel waits.
pub(crate) fn escalated_mark(console: ConsoleId, cx: &App) -> Option<(ExecMarkState, TextFn)> {
    let c = cx.try_global::<Escalations>()?.cancels.get(&console)?;
    c.slow.then(|| {
        (
            ExecMarkState::Escalated { since: c.since },
            escalated_eol(c.since),
        )
    })
}

/// The units of console `console`'s run that are in flight (or already cancelling) show the
/// hourglass.
fn mark_escalated(console: ConsoleId, since: Instant, cx: &mut App) {
    let Some(exec) = console_exec(cx, console) else {
        return;
    };
    let (Some(run), Some(editor)) = (exec.run.as_ref(), exec.editor.upgrade()) else {
        return;
    };
    let marks = editor.read(cx).marks();
    let keys: Vec<MarkKey> = run
        .keys
        .iter()
        .copied()
        .filter(|k| {
            matches!(
                marks.state(*k),
                Some(
                    ExecMarkState::Running { .. }
                        | ExecMarkState::Cancelling { .. }
                        | ExecMarkState::Escalated { .. }
                )
            )
        })
        .collect();
    let eol = escalated_eol(since);
    editor.update(cx, |e, cx| {
        for key in keys {
            e.set_exec_state(
                key,
                ExecMarkState::Escalated { since },
                Some(eol.clone()),
                Some(eol.clone()),
                cx,
            );
            e.set_exec_short(key, None, cx);
        }
    });
}

// ---- the dialog ---------------------------------------------------------------------------

/// Whose transaction the console's Terminate or Disconnect rolls back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnTx {
    None,
    /// The user's open transaction and its statements.
    Open(u32),
    // ## m3b-t23-escalation (fix round 1, m3)
    /// A transaction Hacivat opened for the console (a confirmed change waiting for its
    /// answer, a cursor), in any mode.
    Hacivat,
    /// The running statement may have opened one (Manual or Smart mode).
    Maybe,
}

/// Another console's transaction a profile-wide Disconnect rolls back (fix round 1, m3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtherTx {
    /// The user's open transaction and its statements.
    Open(u32),
    /// A transaction Hacivat opened for it.
    Hacivat,
    /// A request of the user's runs on it: it may have opened one.
    Maybe,
}

/// How far Disconnect reaches (R-M3-73).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// The profile's default console: every connection of the profile.
    Profile,
    /// Any other console: only its own session.
    Console,
    /// SQLite: the one connection to file `name` that the profile's consoles share
    /// (single-session, fix round 1, m4).
    File(String),
}

/// What the dialog says: read once when it opens (the shots fixtures give their own).
#[derive(Debug, Clone)]
pub struct Facts {
    /// The connection's name (data).
    pub name: String,
    /// The console's title (data), e.g. "orders-dev (2)".
    pub console: String,
    // ## m3b-t23-escalation (fix round 1, I2): the subtitle's environment mark and engine.
    pub env: Option<EnvTag>,
    pub engine: Option<SharedString>,
    /// The server function or statement Terminate uses; `None`: no Terminate.
    pub terminate: Option<&'static str>,
    pub own: OwnTx,
    pub reach: Reach,
    /// The other consoles whose transaction a profile-wide Disconnect rolls back: their titles
    /// and transactions.
    pub others: Vec<(String, OtherTx)>,
}

impl Facts {
    /// The statements of this console's open transaction, if one is open.
    fn own_statements(&self) -> Option<u32> {
        match self.own {
            OwnTx::Open(n) => Some(n),
            _ => None,
        }
    }
}

/// The server function or statement that ends a session of `engine` from a side connection
/// (the session's `terminate_sql`): none on SQLite and the engines Hacivat can't terminate on.
fn terminate_with(engine: DbmsKind) -> Option<&'static str> {
    match engine {
        DbmsKind::Postgres => Some("pg_terminate_backend"),
        DbmsKind::MySql | DbmsKind::MariaDb => Some("KILL CONNECTION"),
        _ => None,
    }
}

/// Whether `status` is an open (or failed) transaction.
fn tx_open(status: TxStatus) -> bool {
    matches!(status, TxStatus::InTx | TxStatus::Failed)
}

/// What console `console`'s dialog says now.
fn facts_of(console: ConsoleId, session: &SessionHandle, cx: &App) -> Option<Facts> {
    let profile = profile_of(cx, console)?;
    let base = console_facts(cx, console)?;
    let snap = session.state().borrow().clone();
    let own = match TxView::of(&snap).user {
        Some(u) => OwnTx::Open(u.statements),
        None if tx_open(snap.tx.status) && snap.tx.owner != TxOwner::None => OwnTx::Hacivat,
        None if snap.tx.mode != TxMode::Auto => OwnTx::Maybe,
        None => OwnTx::None,
    };
    let sessions = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.manager().sessions(profile.id));
    let default = sessions
        .as_ref()
        .and_then(|s| s.default_session())
        .is_some_and(|d| d.id() == session.id());
    let reach = match &profile.endpoint {
        ProfileEndpoint::File { path, .. } => Reach::File(
            path.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        ),
        _ if default => Reach::Profile,
        _ => Reach::Console,
    };
    let others = match (&reach, sessions) {
        (Reach::Profile, Some(sessions)) => sessions
            .open_transactions()
            .into_iter()
            .filter(|o| o.session != session.id())
            .map(|o| {
                let title = Consoles::ids(cx)
                    .into_iter()
                    .find(|c| {
                        Consoles::get(cx, *c)
                            .and_then(|s| s.session.as_ref())
                            .is_some_and(|h| h.id() == o.session)
                    })
                    .and_then(|c| console_facts(cx, c))
                    .map_or_else(|| profile.name.clone(), |f| f.title.to_string());
                let tx = match o.tx.owner {
                    TxOwner::User if tx_open(o.tx.status) => OtherTx::Open(o.tx.statements),
                    TxOwner::None | TxOwner::User => OtherTx::Maybe,
                    _ if tx_open(o.tx.status) => OtherTx::Hacivat,
                    _ => OtherTx::Maybe,
                };
                (title, tx)
            })
            .collect(),
        _ => Vec::new(),
    };
    Some(Facts {
        name: profile.name.clone(),
        console: base.title.to_string(),
        env: base.env,
        engine: base.engine.clone(),
        terminate: terminate_with(profile.engine),
        own,
        reach,
        others,
    })
}

/// Opens the escalation dialog for console `console` (a second `exec.cancel` while
/// cancelling, or the toast's Show options…). An open one takes the focus again; a console
/// that cancels nothing opens none.
pub fn open_dialog(console: ConsoleId, window: &mut Window, cx: &mut App) {
    let Some((session, since, open)) = cx
        .try_global::<Escalations>()
        .and_then(|e| e.cancels.get(&console))
        .map(|c| {
            (
                c.session.clone(),
                c.since,
                c.dialog.as_ref().and_then(|d| d.upgrade()),
            )
        })
    else {
        return;
    };
    if let Some(open) = open {
        let keep = open.read(cx).keep.clone();
        window.focus(&keep, cx);
        return;
    }
    let Some(facts) = facts_of(console, &session, cx) else {
        return;
    };
    // The toast's work is done: the dialog says the same and more.
    let toast = cx
        .default_global::<Escalations>()
        .cancels
        .get_mut(&console)
        .and_then(|c| c.toast.take());
    if let Some(toast) = toast {
        ToastQueue::dismiss(cx, toast);
    }
    let view = show(console, facts, since, None, window, cx);
    if let Some(c) = cx.default_global::<Escalations>().cancels.get_mut(&console) {
        c.dialog = view.map(|v| v.downgrade());
    }
}

/// Pushes the dialog over `window`'s workspace.
fn show(
    console: ConsoleId,
    facts: Facts,
    since: Instant,
    frozen: Option<Duration>,
    window: &mut Window,
    cx: &mut App,
) -> Option<hacivat_ui_core::gpui_surface::Entity<EscalationDialog>> {
    let view = cx.new(|cx| EscalationDialog::new(console, facts, since, frozen, window, cx));
    let keep = view.read(cx).keep.clone();
    if !push_modal(ESCALATION_LAYER_ID, view.clone().into(), &keep, window, cx) {
        return None;
    }
    for (name, handle) in view.read(cx).parts() {
        register_part(cx, name, &handle, None);
    }
    Some(view)
}

/// What a terminate the server refused left: its own error, when it sent one.
#[derive(Debug, Clone)]
struct Refused {
    server: Option<ServerMessage>,
}

/// The escalation dialog (module docs).
pub struct EscalationDialog {
    console: ConsoleId,
    facts: Facts,
    /// When the cancel was asked (executor clock).
    since: Instant,
    /// The shots' fixed clock.
    frozen: Option<Duration>,
    refused: Option<Refused>,
    root: FocusHandle,
    /// Keep waiting (Primary, focused).
    keep: FocusHandle,
    terminate: FocusHandle,
    disconnect: FocusHandle,
    _tick: Task<()>,
}

impl Focusable for EscalationDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.root.clone()
    }
}

impl EscalationDialog {
    fn new(
        console: ConsoleId,
        facts: Facts,
        since: Instant,
        frozen: Option<Duration>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The clock: a repaint on each whole second of the cancel's age, never faster.
        let tick = if frozen.is_some() {
            Task::ready(())
        } else {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    let Ok(age) = this.update(cx, |d, cx| d.age(cx)) else {
                        return;
                    };
                    let next = Duration::from_secs(age.as_secs() + 1).saturating_sub(age);
                    cx.background_executor().timer(next).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
            })
        };
        EscalationDialog {
            console,
            facts,
            since,
            frozen,
            refused: None,
            root: cx.focus_handle(),
            keep: cx.focus_handle().tab_stop(true),
            terminate: cx.focus_handle().tab_stop(true),
            disconnect: cx.focus_handle().tab_stop(true),
            _tick: tick,
        }
    }

    fn parts(&self) -> Vec<(&'static str, FocusHandle)> {
        vec![
            ("escalation-keep", self.keep.clone()),
            ("escalation-terminate", self.terminate.clone()),
            ("escalation-disconnect", self.disconnect.clone()),
        ]
    }

    /// How long the cancel has waited (the shots' fixed value, else the executor clock).
    fn age(&self, cx: &App) -> Duration {
        self.frozen.unwrap_or_else(|| {
            cx.background_executor()
                .now()
                .saturating_duration_since(self.since)
        })
    }

    /// Closes the dialog; nothing is sent (Keep waiting, Escape, ⌘W, the close button).
    fn keep_waiting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let console = self.console;
        if let Some(since) = cx
            .try_global::<Escalations>()
            .and_then(|e| e.cancels.get(&console))
            .map(|c| c.since)
        {
            if let Some(c) = cx.default_global::<Escalations>().cancels.get_mut(&console) {
                c.slow = true;
            }
            mark_escalated(console, since, cx);
        }
        self.close(window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity_id();
        window.defer(cx, move |window, cx| remove_modal(view, window, cx));
    }

    /// The request this dialog escalates, while the console still cancels it.
    fn target(&self, cx: &App) -> Option<(u64, SessionHandle)> {
        let c = cx.try_global::<Escalations>()?.cancels.get(&self.console)?;
        Some((c.req, c.session.clone()))
    }

    /// Whether a terminate of this dialog's request waits for its outcome (fix round 1, I1:
    /// the cancel's follower knows, however late the outcome comes).
    fn terminating(&self, cx: &App) -> bool {
        cx.try_global::<Escalations>()
            .and_then(|e| e.cancels.get(&self.console))
            .is_some_and(|c| c.terminate.is_some())
    }

    /// Terminate on server: the session ends the server session from a side connection. Its
    /// outcome is the follower's (`follow`): a refusal (`CancelFailed`, or a `Failed` reply)
    /// whenever it comes, a success when the request ends with the connection.
    fn terminate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminating(cx) || self.facts.terminate.is_none() {
            return;
        }
        let Some((req, session)) = self.target(cx) else {
            return self.close(window, cx);
        };
        let statements = self.facts.own_statements();
        if let Some(c) = cx
            .default_global::<Escalations>()
            .cancels
            .get_mut(&self.console)
        {
            c.terminate = Some(statements);
        }
        let reply = session.escalate(req, Escalation::Terminate);
        self.refused = None;
        cx.notify();
        let console = self.console;
        // Detached: a refusal without the server's text (no side connection) is a `Failed`
        // reply with no event; the event, when there is one, goes to the follower too.
        cx.spawn(async move |_, cx| {
            if let Ok(CancelOutcome::Failed(_)) = reply.await {
                cx.update(|cx| terminate_refused(console, req, None, cx));
            }
        })
        .detach();
    }

    /// Disconnect: the session drops its connection (`Escalation::Deactivate`); from the
    /// profile's default console the profile disconnects with it (R-M3-73).
    fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((req, session)) = self.target(cx) else {
            return self.close(window, cx);
        };
        // The request ends by the disconnect, not by a terminate still on its way.
        if let Some(c) = cx
            .default_global::<Escalations>()
            .cancels
            .get_mut(&self.console)
        {
            c.terminate = None;
        }
        let reply = session.escalate(req, Escalation::Deactivate);
        let (console, name) = (self.console, self.facts.name.clone());
        let local = matches!(self.facts.reach, Reach::File(_));
        cx.spawn(async move |_, cx| {
            let outcome = reply.await.unwrap_or(CancelOutcome::Disconnected);
            if outcome != CancelOutcome::AlreadyFinished {
                cx.update(|cx| {
                    log(
                        console,
                        LogMsg::Disconnected {
                            name: Arc::from(name.as_str()),
                            local,
                        },
                        cx,
                    );
                });
            }
        })
        .detach();
        self.close(window, cx);
    }

    /// The buttons in visual (and Tab) order: Disconnect, Terminate on server, Keep waiting.
    fn order(&self) -> Vec<FocusHandle> {
        let mut order = vec![self.disconnect.clone()];
        if self.facts.terminate.is_some() {
            order.push(self.terminate.clone());
        }
        order.push(self.keep.clone());
        order
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            // Escape answers the safe value; it never escalates (R-M3-38).
            "escape" => self.keep_waiting(window, cx),
            "enter" => {
                if !acting_enter(event) {
                    return cx.stop_propagation();
                }
                if self.keep.is_focused(window) {
                    self.keep_waiting(window, cx);
                } else if self.terminate.is_focused(window) {
                    self.terminate(window, cx);
                } else if self.disconnect.is_focused(window) {
                    self.disconnect(window, cx);
                }
            }
            "w" if closes_dialog(stroke, cx) => self.keep_waiting(window, cx),
            _ if trapped_tab(event) => {
                cycle_focus(&self.order(), stroke.modifiers.shift, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

/// Appends `msg` to console `console`'s Output.
fn log(console: ConsoleId, msg: LogMsg, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console) else {
        return;
    };
    state.log.append(msg, None, None);
    if let Some(results) = state.results.clone() {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

impl Render for EscalationDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let terminating = self.terminating(cx);
        let f = &self.facts;
        let title = message(cx, "hv-cancel-dlg-title", vec![]);
        // Fix round 1 (I2): the environment mark and the connection, as every dialog's.
        let sub = subtitle(&theme, f.env, &f.name, f.engine.clone(), None);
        let age = clock_text(cx, millis(self.age(cx)));
        let body_text = message(
            cx,
            "hv-cancel-dlg-body",
            vec![("duration", ArgValue::Text(age))],
        );
        let text = |s: &str| ArgValue::Text(visible(s, NAME_MAX_CHARS));
        let line = |text: SharedString, id: &'static str| {
            paragraph(
                text,
                TextSource::Fluent,
                t.type_ramp.body,
                c.text_2.to_hsla(),
            )
            .id(id)
        };
        let own = match f.own {
            OwnTx::Open(n) => Some(message(
                cx,
                "hv-cancel-own-tx",
                vec![("n", ArgValue::Count(u64::from(n)))],
            )),
            OwnTx::Hacivat => Some(message(cx, "hv-cancel-own-tx-hacivat", vec![])),
            OwnTx::Maybe => Some(message(cx, "hv-cancel-own-tx-maybe", vec![])),
            OwnTx::None => None,
        };
        let mut description = vec![body_text.clone()];
        let mut body = div().flex().flex_col().gap(px(t.spacing.md)).child(
            paragraph(
                body_text,
                TextSource::Fluent,
                t.type_ramp.body,
                c.text.to_hsla(),
            )
            .id("escalation-body"),
        );
        let group = || div().flex().flex_col().gap(px(t.spacing.xs));
        if let Some(func) = f.terminate {
            let first = message(
                cx,
                "hv-cancel-terminate-line",
                vec![("fn", ArgValue::Text(func.to_string()))],
            );
            description.push(first.clone());
            description.extend(own.clone());
            body = body.child(
                group()
                    .child(line(first, "escalation-terminate-line"))
                    .children(own.clone().map(|o| line(o, "escalation-terminate-own"))),
            );
        }
        let reach = match &f.reach {
            Reach::Profile => message(
                cx,
                "hv-cancel-disconnect-line-all",
                vec![("name", text(&f.name))],
            ),
            Reach::Console => message(
                cx,
                "hv-cancel-disconnect-line-one",
                vec![("console", text(&f.console))],
            ),
            Reach::File(file) => message(
                cx,
                "hv-cancel-disconnect-line-local",
                vec![("name", text(file))],
            ),
        };
        let others: Vec<SharedString> = f
            .others
            .iter()
            .map(|(console, tx)| match tx {
                OtherTx::Open(n) => message(
                    cx,
                    "hv-cancel-other-tx",
                    vec![
                        ("console", text(console)),
                        ("n", ArgValue::Count(u64::from(*n))),
                    ],
                ),
                OtherTx::Hacivat => message(
                    cx,
                    "hv-cancel-other-tx-hacivat",
                    vec![("console", text(console))],
                ),
                OtherTx::Maybe => message(
                    cx,
                    "hv-cancel-other-tx-maybe",
                    vec![("console", text(console))],
                ),
            })
            .collect();
        description.push(reach.clone());
        description.extend(own.clone());
        description.extend(others.iter().cloned());
        body = body.child(
            group()
                .child(line(reach, "escalation-disconnect-line"))
                .children(own.map(|o| line(o, "escalation-disconnect-own")))
                .children(
                    others
                        .into_iter()
                        .map(|o| line(o, "escalation-other-tx").into_any_element()),
                ),
        );
        if let Some(refused) = &self.refused {
            let failed = message(cx, "hv-cancel-terminate-failed", vec![]);
            description.push(failed.clone());
            body = body.child(wrapping_alert(
                &theme,
                failed,
                "escalation-terminate-failed",
            ));
            if let Some(server) = &refused.server {
                body = body.child(server_box(&theme, server, cx));
            }
        }

        let button = |id: &'static str,
                      label_id: &'static str,
                      kind: ButtonKind,
                      icon: Option<IconName>,
                      handle: &FocusHandle,
                      disabled: bool,
                      act: fn(&mut Self, &mut Window, &mut Context<Self>),
                      cx: &mut Context<Self>| {
            let mut b = Button::new(id, message(cx, label_id, vec![]))
                .kind(kind)
                .on_raised(true)
                .disabled(disabled)
                .focus_handle(handle.clone())
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    // The second click of a double-click never acts.
                    if event.click_count() > 1 {
                        return;
                    }
                    act(this, window, cx);
                }));
            if let Some(icon) = icon {
                b = b.icon(icon);
            }
            b.into_any_element()
        };
        let left = vec![button(
            "escalation-disconnect",
            "hv-cancel-disconnect",
            ButtonKind::Danger,
            Some(IconName::Unplug),
            &self.disconnect,
            false,
            Self::disconnect,
            cx,
        )];
        let mut right = Vec::new();
        if f.terminate.is_some() {
            right.push(button(
                "escalation-terminate",
                "hv-cancel-terminate",
                ButtonKind::Default,
                Some(IconName::OctagonX),
                &self.terminate,
                terminating,
                Self::terminate,
                cx,
            ));
        }
        right.push(button(
            "escalation-keep",
            "hv-cancel-keep-waiting",
            ButtonKind::Primary,
            None,
            &self.keep,
            false,
            Self::keep_waiting,
            cx,
        ));
        let card = card(&theme, DialogSize::Standard, ESCALATION_LAYER_ID)
            .track_focus(&self.root)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .child(header(
                &theme,
                title.clone(),
                "escalation-title",
                Some(sub),
                cx.listener(|this, _, window, cx| this.keep_waiting(window, cx)),
                cx,
            ))
            .child(scroll_body(body))
            .child(grouped_footer(&theme, left, right));
        let description: SharedString = description
            .iter()
            .map(SharedString::as_ref)
            .collect::<Vec<_>>()
            .join(" ")
            .into();
        A11yNode::new("escalation-node", Role::AlertDialog, title, true, card)
            .description(description)
    }
}

/// The ink alert line (`prompt_kit::alert_line`), wrapping: the circle-alert glyph and `text`
/// (12 px, text), recorded as `id`.
fn wrapping_alert(theme: &Theme, text: SharedString, id: &'static str) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.sm))
        .child(
            div().flex_none().pt(px(2.0)).child(
                Icon::new(IconName::CircleAlert)
                    .size(IconSize::Chrome)
                    .color(c.text.to_hsla()),
            ),
        )
        .child(
            div().flex_1().min_w(px(0.0)).child(
                paragraph(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                .id(id),
            ),
        )
}

/// The server-message box (NOTES "New primitives"): the caption with the code, then the
/// server's text verbatim in Fragment Mono (made visible, never interpreted).
fn server_box(theme: &Theme, server: &ServerMessage, cx: &App) -> impl IntoElement {
    let t = theme.tokens;
    let c = &t.colors;
    let caption = message(cx, "hv-conn-err-server", vec![]);
    let code = server
        .sqlstate
        .clone()
        .or_else(|| server.vendor_code.map(|v| v.to_string()));
    inset(theme)
        .id("escalation-server")
        .gap(px(t.spacing.xs))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(t.spacing.sm))
                .child(
                    Label::new(
                        caption,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("escalation-server-caption"),
                )
                .children(code.map(|code| {
                    div()
                        .px(px(t.spacing.xs))
                        .border_1()
                        .border_color(c.seam_strong.to_hsla())
                        .rounded(px(t.radii.xs))
                        .child(
                            Label::new(
                                code,
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                            )
                            .id("escalation-server-code"),
                        )
                })),
        )
        .child(
            paragraph(
                visible(&server.text, SERVER_TEXT_MAX_CHARS),
                TextSource::Data,
                t.type_ramp.mono_data,
                c.text.to_hsla(),
            )
            .id("escalation-server-text"),
        )
}

// ---- test seams ---------------------------------------------------------------------------

/// What the escalation showed (tests).
#[cfg(feature = "test-support")]
#[derive(Default)]
struct Record {
    toasts: Vec<(ConsoleId, u64)>,
}

#[cfg(feature = "test-support")]
impl hacivat_ui_core::gpui_surface::Global for Record {}

/// The slow-cancel toasts still visible, by console (tests).
#[cfg(feature = "test-support")]
pub fn slow_toasts(cx: &App) -> Vec<(ConsoleId, u64)> {
    let visible = ToastQueue::visible_ids(cx);
    cx.try_global::<Record>()
        .map(|r| {
            r.toasts
                .iter()
                .filter(|(_, id)| visible.contains(id))
                .copied()
                .collect()
        })
        .unwrap_or_default()
}

/// Whether console `console`'s escalation dialog is open (tests).
pub fn dialog_open(console: ConsoleId, cx: &App) -> bool {
    cx.try_global::<Escalations>()
        .and_then(|e| e.cancels.get(&console))
        .and_then(|c| c.dialog.as_ref()?.upgrade())
        .is_some()
}

/// The open dialog's facts, server text and whether a terminate is in flight (tests).
pub fn dialog_state(console: ConsoleId, cx: &App) -> Option<(Facts, Option<String>, bool)> {
    let view = cx
        .try_global::<Escalations>()?
        .cancels
        .get(&console)?
        .dialog
        .as_ref()?
        .upgrade()?;
    let d = view.read(cx);
    let server = d
        .refused
        .as_ref()
        .and_then(|r| r.server.as_ref())
        .map(|s| s.text.clone());
    Some((d.facts.clone(), server, d.refused.is_some()))
}

// ---- shots fixtures -----------------------------------------------------------------------

/// Shots fixtures for surface 6 (feature `shots`): the toast, the dialog in each of its forms
/// and the gutter's hourglass, with fixed clocks, without a server that ignores a cancel.
#[cfg(feature = "shots")]
pub mod fixtures {
    use super::*;

    /// Shows the slow-cancel toast of console `console` at `secs` seconds.
    pub fn show_toast(console: ConsoleId, secs: u64, cx: &mut App) {
        let name = super::super::orchestrator::console_name(cx, console);
        ToastQueue::push(cx, slow_toast(console, name, Duration::from_secs(secs)));
    }

    /// Opens console `console`'s dialog over `window` with `facts` at `secs` seconds; with
    /// `refused`, the failed-terminate form with the server's `(code, text)`.
    pub fn open(
        console: ConsoleId,
        facts: Facts,
        secs: u64,
        refused: Option<(&str, &str)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let since = cx.background_executor().now();
        let Some(view) = show(
            console,
            facts,
            since,
            Some(Duration::from_secs(secs)),
            window,
            cx,
        ) else {
            return;
        };
        if let Some((code, text)) = refused {
            view.update(cx, |d, cx| {
                d.refused = Some(Refused {
                    server: Some(ServerMessage {
                        sqlstate: Some(code.to_string()),
                        vendor_code: None,
                        severity: Some("ERROR".to_string()),
                        text: text.to_string(),
                        detail: None,
                        hint: None,
                        lang_hint: None,
                        position: None,
                    }),
                });
                cx.notify();
            });
        }
    }

    /// The facts of console `console` as a live dialog would read them, with `own`, `reach` and
    /// `others` given (the shots have no slow server).
    pub fn facts(
        console: ConsoleId,
        own: OwnTx,
        reach: Reach,
        others: Vec<(String, OtherTx)>,
        cx: &App,
    ) -> Option<Facts> {
        let profile = profile_of(cx, console)?;
        let base = console_facts(cx, console)?;
        Some(Facts {
            name: profile.name.clone(),
            console: base.title.to_string(),
            env: base.env,
            engine: base.engine.clone(),
            terminate: terminate_with(profile.engine),
            own,
            reach,
            others,
        })
    }

    /// Fixture mark `key` shows the hourglass at `secs` seconds (fixed words).
    pub fn escalate_mark(console: ConsoleId, key: MarkKey, secs: u64, cx: &mut App) {
        let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) else {
            return;
        };
        let now = cx.background_executor().now();
        let since = now.checked_sub(Duration::from_secs(secs)).unwrap_or(now);
        let ms = secs * 1_000;
        let eol: TextFn = Rc::new(move |cx: &App| {
            message(
                cx,
                "hv-editor-mark-escalated",
                vec![("duration", ArgValue::Text(clock_text(cx, ms)))],
            )
        });
        editor.update(cx, |e, cx| {
            e.set_exec_state(
                key,
                ExecMarkState::Escalated { since },
                Some(eol.clone()),
                Some(eol),
                cx,
            );
            e.set_exec_short(key, None, cx);
        });
    }
}
