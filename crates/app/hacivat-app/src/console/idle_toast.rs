//! The idle-transaction toast (D-014, C1; M3b Task 22, `design/m3b-safe` surface 5): a user
//! transaction left open past the idle threshold.
//!
//! ## m3b-t22-tx-ui
//!
//! - **Source.** Each console session's own `SessionEvent::IdleTransaction` (T12: every console
//!   session runs its own liveness and idles on its own clock), read from the session's
//!   broadcast by one foreground task per session (a `tokio::sync` receiver, Q4). A shared
//!   session (single-session profile) is watched once. The manager's profile-level relay of the
//!   default session's event is then the same reminder: `session_toasts::idle_tx` leaves it to
//!   this toast whenever a console shows that session ([`shows_session_of`]).
//! - **The toast.** The M2a sticky toast, wide, no TTL: the clock glyph, "Transaction open for
//!   10 min on orders-prod", "It may hold locks that block other sessions.", then Commit,
//!   Roll back and Keep open. Only while the session holds a user transaction (Hacivat's own
//!   never idles here, T12 `is_hacivats`).
//! - **One per session.** A newer reminder for the same session replaces the older one; a
//!   transaction that ended takes its toast away.
//! - **Nothing ends silently.** Keep open only dismisses the toast; a dismissed or timed-out
//!   toast sends nothing (the toast is sticky: it has no timeout). Commit and Roll back are the
//!   user's answer, through `exec_ui::tx::end_tx`, as the badge's, and wait as the badge's do
//!   (fix round 1, I3): while the session runs the user's request or a question is open the
//!   reminder stays and a line says why. The toast goes when the transaction it was raised for
//!   ends, or another one of the session's takes its place.

use std::collections::HashMap;
use std::rc::Rc;

use hacivat_l10n::{NoArgs, Text, ids};
use hacivat_proto::ProfileId;
use hacivat_session::session::{SessionEvent, SessionHandle, SessionId};
use hacivat_ui::icon::IconName;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastAction, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{AnyElement, App, Global, Task};
use tokio::sync::broadcast::error::RecvError;

use crate::connections::ConnectionsModel;
use crate::console::tx_badge::{self, age_text};
use crate::console::{ConsoleId, Consoles};
use crate::l10n_global::{text_fn, tr};

/// One session whose events the toast follows.
struct Followed {
    _task: (Task<()>, Task<()>),
}

/// The toasts shown, by session, and the sessions followed.
#[derive(Default)]
struct IdleToasts {
    shown: HashMap<SessionId, u64>,
    /// The start of the transaction each toast was raised for (fix round 1, I3): another
    /// transaction of the session takes the toast away.
    raised: HashMap<SessionId, Option<std::time::Instant>>,
    followed: HashMap<SessionId, Followed>,
    /// What each toast said, for the tests: (toast id, session, minutes).
    #[cfg(feature = "test-support")]
    record: Vec<(u64, SessionId, u32)>,
}

impl Global for IdleToasts {}

/// Starts following every console session's idle reminders (from `console::install`).
pub fn install(cx: &mut App) {
    cx.default_global::<IdleToasts>();
    cx.observe_global::<Consoles>(follow_sessions).detach();
    follow_sessions(cx);
}

/// The sessions of every console, once each.
fn sessions(cx: &App) -> Vec<SessionHandle> {
    let mut out: Vec<SessionHandle> = Vec::new();
    for id in Consoles::ids(cx) {
        if let Some(h) = Consoles::get(cx, id).and_then(|s| s.session.clone())
            && !out.iter().any(|o| o.id() == h.id())
        {
            out.push(h);
        }
    }
    out
}

/// Keeps one follower per console session; a session no console shows loses its own (and its
/// toast: no console is left to answer it).
fn follow_sessions(cx: &mut App) {
    let wanted = sessions(cx);
    let toasts = cx.default_global::<IdleToasts>();
    toasts
        .followed
        .retain(|id, _| wanted.iter().any(|h| h.id() == *id));
    let gone: Vec<u64> = toasts
        .shown
        .iter()
        .filter(|(id, _)| !wanted.iter().any(|h| h.id() == **id))
        .map(|(_, toast)| *toast)
        .collect();
    toasts
        .shown
        .retain(|id, _| wanted.iter().any(|h| h.id() == *id));
    let new: Vec<SessionHandle> = wanted
        .into_iter()
        .filter(|h| !toasts.followed.contains_key(&h.id()))
        .collect();
    for toast in gone {
        ToastQueue::dismiss(cx, toast);
    }
    for session in new {
        let id = session.id();
        let mut events = session.events();
        let mut state = session.state();
        // Awaits `tokio::sync` receivers only (Q4): the reminders, and the transaction's end.
        let reminders = cx.spawn(async move |cx| {
            loop {
                match events.recv().await {
                    Ok(SessionEvent::IdleTransaction { minutes }) => {
                        cx.update(|cx| show(id, minutes, cx));
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        });
        let ends = cx.spawn(async move |cx| {
            while state.changed().await.is_ok() {
                let (open, since) = {
                    let snap = state.borrow_and_update();
                    (user_open(&snap), snap.tx.since)
                };
                cx.update(|cx| {
                    let other = cx
                        .try_global::<IdleToasts>()
                        .and_then(|t| t.raised.get(&id).copied())
                        .is_some_and(|raised| raised != since);
                    if !open || other {
                        clear(id, cx);
                    }
                });
            }
            cx.update(|cx| clear(id, cx));
        });
        let task = (reminders, ends);
        cx.default_global::<IdleToasts>()
            .followed
            .insert(id, Followed { _task: task });
    }
}

/// Whether `snap` shows a user transaction (open or failed) on a usable session.
fn user_open(snap: &hacivat_session::session::SessionSnapshot) -> bool {
    tx_badge::TxView::of(snap).user.is_some()
}

/// A console showing session `session`, the focused one first.
fn console_of(session: SessionId, cx: &App) -> Option<ConsoleId> {
    let shows = |c: &ConsoleId| {
        Consoles::get(cx, *c)
            .and_then(|s| s.session.as_ref())
            .is_some_and(|h| h.id() == session)
    };
    Consoles::focused(cx)
        .filter(shows)
        .or_else(|| Consoles::ids(cx).into_iter().find(shows))
}

/// Whether a console shows the default session of `profile` (its idle reminder is this
/// toast's, not the profile-level notice's).
pub fn shows_session_of(profile: ProfileId, cx: &App) -> bool {
    let Some(default) = cx
        .try_global::<ConnectionsModel>()
        .and_then(|m| m.manager().sessions(profile))
        .and_then(|s| s.default_session())
    else {
        return false;
    };
    console_of(default.id(), cx).is_some()
}

/// Session `session` has held a user transaction idle for `minutes`: the toast, replacing the
/// session's older one.
pub fn show(session: SessionId, minutes: u32, cx: &mut App) {
    let Some(console) = console_of(session, cx) else {
        return;
    };
    if tx_badge::user_tx(console, cx).is_none() {
        return;
    }
    let name = tx_badge::connection_name(console, cx).unwrap_or_default();
    let title: TextFn = Rc::new(move |cx: &App| {
        tr(
            cx,
            ids::hv::TX_IDLE_TITLE,
            ids::hv::HvTxIdleTitle {
                age: Text(age_text(cx, u64::from(minutes)).to_string()),
                name: Text(name.clone()),
            },
        )
    });
    let mut toast = Toast::new(ToastKind::Info, text_fn(ids::hv::TX_IDLE_BODY));
    toast.sticky = true;
    toast.wide = true;
    toast.title = Some(title);
    toast.icon = Some(IconName::Clock);
    let answer = |commit: bool| -> hacivat_ui::actions::ActionFn {
        Rc::new(move |_, cx| {
            // The console that shows the session now (a console may have closed since).
            let Some(console) = console_of(session, cx) else {
                return;
            };
            // Fix round 1 (I3): as the badge's, never while the session runs the user's
            // request or a question of its run is open; the reminder comes back and says so.
            if tx_badge::answer_waits(console, cx) {
                cx.defer(move |cx| {
                    show(session, minutes, cx);
                    ToastQueue::push(
                        cx,
                        Toast::new(ToastKind::Info, text_fn(ids::hv::TX_IDLE_WAITS)),
                    );
                });
                return;
            }
            if tx_badge::user_tx(console, cx).is_some_and(|u| !(commit && u.failed)) {
                crate::exec_ui::tx::end_tx(console, commit, cx).detach();
            }
        })
    };
    let failed = tx_badge::user_tx(console, cx).is_some_and(|u| u.failed);
    let mut actions = Vec::new();
    if !failed {
        actions.push(ToastAction {
            label: text_fn(ids::hv::TX_IDLE_COMMIT),
            run: answer(true),
        });
    }
    actions.push(ToastAction {
        label: text_fn(ids::hv::TX_IDLE_ROLLBACK),
        run: answer(false),
    });
    actions.push(ToastAction {
        label: text_fn(ids::hv::TX_IDLE_KEEP),
        // An action dismisses its toast after it runs; Keep open sends nothing.
        run: Rc::new(|_, _| {}),
    });
    toast.actions = actions;
    let old = cx.default_global::<IdleToasts>().shown.remove(&session);
    if let Some(old) = old {
        ToastQueue::dismiss(cx, old);
    }
    let since = Consoles::get(cx, console)
        .and_then(|s| s.session.as_ref())
        .and_then(|h| h.state().borrow().tx.since);
    let toast_id = ToastQueue::push_id(cx, toast);
    let toasts = cx.default_global::<IdleToasts>();
    toasts.shown.insert(session, toast_id);
    toasts.raised.insert(session, since);
    #[cfg(feature = "test-support")]
    toasts.record.push((toast_id, session, minutes));
}

/// The session's transaction ended (or the session went): its toast goes.
fn clear(session: SessionId, cx: &mut App) {
    if let Some(old) = cx
        .try_global::<IdleToasts>()
        .and_then(|t| t.shown.get(&session).copied())
    {
        let toasts = cx.default_global::<IdleToasts>();
        toasts.shown.remove(&session);
        toasts.raised.remove(&session);
        ToastQueue::dismiss(cx, old);
    }
}

/// The toast of console `console` is a toast of the queue, never an element of the console:
/// `None` (the slot Task 1b made stays empty).
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let _ = (console, cx, NoArgs);
    None
}

// ## test seam (feature `test-support`)

/// The idle toasts still visible: (session, minutes), oldest first.
#[cfg(feature = "test-support")]
pub fn shown(cx: &App) -> Vec<(SessionId, u32)> {
    let visible = ToastQueue::visible_ids(cx);
    cx.try_global::<IdleToasts>()
        .map(|t| {
            t.record
                .iter()
                .filter(|(id, _, _)| visible.contains(id))
                .map(|(_, s, m)| (*s, *m))
                .collect()
        })
        .unwrap_or_default()
}

/// The queue id of session `session`'s idle toast (its buttons are `toast-action-<id>-<i>`).
#[cfg(feature = "test-support")]
pub fn toast_id(session: SessionId, cx: &App) -> Option<u64> {
    cx.try_global::<IdleToasts>()?.shown.get(&session).copied()
}
