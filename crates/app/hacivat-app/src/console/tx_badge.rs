//! The open-transaction badge in the console toolbar (D-014; M3b Task 22, `design/m3b-safe`
//! surface 4) and the transaction mirror the chip, the badge and the status segment read.
//!
//! ## m3b-t22-tx-ui
//!
//! - **Whose transaction.** Only the user's (`TxOwner::User`, R-M3-27): Hacivat's own around a
//!   PostgreSQL cursor, the read-only guard's and a confirmed statement's are never shown, and a
//!   session that ended (closed or broken; its final state is published by the session, T12)
//!   shows none. A shared session (single-session profile) shows it in every console bound to
//!   it, because every console mirrors the same session.
//! - **The badge.** A trust signal, pinned on the right of the toolbar after a seam: the 7 px
//!   amber stamp, "Transaction · 3 statements · 2 min" (the age repainted once a minute, "<1 min"
//!   first, on the executor clock), then Commit and Roll back (Default buttons with ink icons).
//!   Failed (PostgreSQL aborted): the ink alert glyph, "Transaction failed", Roll back only. Its
//!   compact form drops the age and makes the buttons icon-only (the words stay their tooltips
//!   and accessible names); the stamp and "Transaction · n statements" never go.
//! - **No answer while a question waits.** Commit and Roll back are disabled while the console's
//!   session runs a request of the user's (a run that waits on its error-policy or
//!   confirm-in-transaction question holds the session; a Commit would only queue behind it,
//!   T11 I1), and while a surface says a question is open ([`set_choice_pending`]).
//! - **The mirror.** One foreground task per console session awaits its `watch` (a
//!   `tokio::sync` receiver, Q4) and repaints the windows only when the projection
//!   ([`TxView`]) changes; nothing reads the session on the paint path. While a user
//!   transaction is open a one-minute ticker repaints the age.
//! - **A read-only session's skipped startup script** (T12 carry m9): when a console takes a
//!   read-only session of a profile that has a startup script, its Output says the script was
//!   not run (`exec-startup-script-skipped-read-only`); the session itself only logs it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_driver::TxStatus;
use hacivat_l10n::{ArgValue, Count, NoArgs, Text, ids};
use hacivat_proto::DbmsKind;
use hacivat_session::session::{
    IsolationLevel, SessionHandle, SessionId, SessionSnapshot, SessionState, TxMode, TxOwner,
};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::Tip;
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Bounds, FocusHandle, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, SharedString, StatefulInteractiveElement as _, Styled as _, Task,
    canvas, div, px,
};
use hacivat_ui_core::theme::Theme;

use crate::connections::ConnectionsModel;
use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::message;
use crate::l10n_global::tr;

/// The user's transaction as the console shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserTxView {
    /// Failed: only a rollback ends it.
    pub failed: bool,
    pub statements: u32,
}

/// What the chip, the badge and the status segment show of a console's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TxView {
    pub mode: TxMode,
    pub isolation: IsolationLevel,
    pub read_only: bool,
    /// The session runs a request of the user's (or cancels one).
    pub busy: bool,
    /// The session closed or broke: it shows no transaction.
    pub ended: bool,
    /// The user's open (or failed) transaction.
    pub user: Option<UserTxView>,
}

impl TxView {
    /// The projection of `snap` (see the module docs).
    pub fn of(snap: &SessionSnapshot) -> TxView {
        let ended = matches!(
            snap.state,
            SessionState::Disconnected | SessionState::Broken(_)
        );
        let busy = matches!(
            snap.state,
            SessionState::Busy { req, .. } if req != 0
        ) || matches!(snap.state, SessionState::Cancelling { .. });
        // Fix round 1 (m4): the status segment's rule (`hacivat_exec::session_live`): a
        // session that is connecting, closed or broken shows no transaction.
        let live = hacivat_exec::session_live(&snap.state);
        let user = (live && snap.tx.owner == TxOwner::User)
            .then_some(snap.tx.status)
            .and_then(|status| match status {
                TxStatus::InTx => Some(UserTxView {
                    failed: false,
                    statements: snap.tx.statements,
                }),
                TxStatus::Failed => Some(UserTxView {
                    failed: true,
                    statements: snap.tx.statements,
                }),
                TxStatus::Idle => None,
            });
        TxView {
            mode: snap.tx.mode,
            isolation: snap.tx.isolation,
            read_only: snap.read_only,
            busy,
            ended,
            user,
        }
    }
}

/// One console session the mirror watches.
struct Watched {
    session: SessionId,
    view: TxView,
    /// When the user's transaction opened, on the executor clock (the badge's age).
    opened: Option<Instant>,
    /// The session's own start of that transaction (fix round 1, m5: a COMMIT and a BEGIN
    /// the watch coalesced start a new age).
    since: Option<Instant>,
    /// The console holds its profile's default session (fix round 1, I1).
    holds_default: bool,
    _task: Task<()>,
}

/// The mirror (a global): every console session's [`TxView`], and the consoles whose question
/// is open.
#[derive(Default)]
struct TxMirror {
    watched: HashMap<ConsoleId, Watched>,
    pending: HashSet<ConsoleId>,
    ticker: Option<Task<()>>,
    /// The Tab stops of each console's chip and status segment, kept across repaints.
    focus: HashMap<(ConsoleId, Focus), FocusHandle>,
    /// Where each chip and status segment was last painted (their menus open from there).
    bounds: PaintedBounds,
}

/// Where each console's chip and status segment was last painted, shared with their painters.
type PaintedBounds = Rc<RefCell<HashMap<(ConsoleId, Focus), Bounds<Pixels>>>>;

/// Which of a console's transaction controls a focus handle belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focus {
    /// The mode chip.
    Chip,
    /// The status bar's transaction segment.
    Segment,
    /// The "Shared session" chip (fix round 1, I2: the shots check it is drawn).
    Shared,
}

/// Where console `console`'s `which` control was last painted.
pub fn bounds_of(console: ConsoleId, which: Focus, cx: &App) -> Option<Bounds<Pixels>> {
    cx.try_global::<TxMirror>()?
        .bounds
        .borrow()
        .get(&(console, which))
        .copied()
}

/// An element that records where it is painted as console `console`'s `which` control (an
/// absolute overlay: put it inside the control).
pub fn record_bounds(console: ConsoleId, which: Focus, cx: &App) -> Option<AnyElement> {
    let store = cx.try_global::<TxMirror>()?.bounds.clone();
    Some(
        canvas(
            move |b, _, _| {
                store.borrow_mut().insert((console, which), b);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
        .into_any_element(),
    )
}

/// The Tab stop of console `console`'s `which` control (made when the mirror first watched the
/// console's session).
pub fn focus_of(console: ConsoleId, which: Focus, cx: &App) -> Option<FocusHandle> {
    cx.try_global::<TxMirror>()?
        .focus
        .get(&(console, which))
        .cloned()
}

impl Global for TxMirror {}

/// Starts the mirror: every console session's transaction (from `console::install`).
pub fn install(cx: &mut App) {
    cx.default_global::<TxMirror>();
    cx.observe_global::<Consoles>(watch_sessions).detach();
    watch_sessions(cx);
    // ## fix round 1 (I1)
    cx.observe_global::<ConnectionsModel>(follow_single_session)
        .detach();
    follow_single_session(cx);
}

/// Keeps one watcher per console session.
fn watch_sessions(cx: &mut App) {
    let wanted: Vec<(ConsoleId, SessionHandle)> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|id| Some((id, Consoles::get(cx, id)?.session.clone()?)))
        .collect();
    let mirror = cx.default_global::<TxMirror>();
    let before = mirror.watched.len();
    mirror.watched.retain(|id, w| {
        wanted
            .iter()
            .any(|(want, h)| want == id && h.id() == w.session)
    });
    mirror
        .focus
        .retain(|(id, _), _| wanted.iter().any(|(want, _)| want == id));
    // Fix round 1 (m10): a console that closed leaves no pending question behind.
    mirror
        .pending
        .retain(|id| wanted.iter().any(|(want, _)| want == id));
    let dropped = mirror.watched.len() != before;
    let new: Vec<(ConsoleId, SessionHandle)> = wanted
        .into_iter()
        .filter(|(id, _)| !mirror.watched.contains_key(id))
        .collect();
    let added = !new.is_empty();
    for (id, session) in new {
        for which in [Focus::Chip, Focus::Segment] {
            let handle = cx.focus_handle().tab_stop(true);
            cx.default_global::<TxMirror>()
                .focus
                .entry((id, which))
                .or_insert(handle);
        }
        let mut rx = session.state();
        let snap = rx.borrow_and_update().clone();
        let view = TxView::of(&snap);
        if snap.read_only {
            note_script_skipped(id, cx);
        }
        let now = cx.background_executor().now();
        let opened = view.user.map(|_| opened_at(now, &snap));
        let holds_default = holds_default_session(id, &session, cx);
        // The foreground task awaits a `tokio::sync::watch` receiver only (Q4 ruling).
        let task = cx.spawn(async move |cx| {
            while rx.changed().await.is_ok() {
                let snap = rx.borrow_and_update().clone();
                cx.update(|cx| note(cx, id, &snap));
            }
        });
        cx.default_global::<TxMirror>().watched.insert(
            id,
            Watched {
                session: session.id(),
                view,
                opened,
                since: snap.tx.since,
                holds_default,
                _task: task,
            },
        );
    }
    tick(cx);
    if dropped || added {
        cx.refresh_windows();
    }
}

// ## fix round 1 (I1)
/// Whether console `console`'s `session` is its profile's default session.
fn holds_default_session(console: ConsoleId, session: &SessionHandle, cx: &App) -> bool {
    Consoles::get(cx, console)
        .and_then(|s| s.profile)
        .and_then(|p| cx.try_global::<ConnectionsModel>()?.manager().sessions(p))
        .and_then(|s| s.default_session())
        .is_some_and(|d| d.id() == session.id())
}

/// Whether console `console` shares its session, as the consoles hold them now (fix round 1,
/// I1; never the saved setting alone, which applies only to sessions opened afterwards):
/// another console shows the same session, or this console holds its profile's default
/// session, the profile gives every console that session (single-session, SQLite), and no
/// other console of the profile holds a session of its own.
pub fn shares_session(console: ConsoleId, cx: &App) -> bool {
    let Some(m) = cx.try_global::<TxMirror>() else {
        return false;
    };
    let Some(me) = m.watched.get(&console) else {
        return false;
    };
    if me.view.ended {
        return false;
    }
    if m.watched
        .iter()
        .any(|(id, w)| *id != console && w.session == me.session)
    {
        return true;
    }
    let profile_of = |c: ConsoleId| Consoles::get(cx, c).and_then(|s| s.profile);
    let Some(profile) = profile_of(console) else {
        return false;
    };
    let single = cx
        .try_global::<ConnectionsModel>()
        .and_then(|model| model.get(profile))
        .is_some_and(|p| p.single_session || p.engine == DbmsKind::Sqlite);
    me.holds_default
        && single
        && !m.watched.iter().any(|(id, w)| {
            *id != console && profile_of(*id) == Some(profile) && w.session != me.session
        })
}

/// The saved single-session setting of each connected profile, as last seen (fix round 1,
/// I1): a change applies to sessions opened after the next reconnect, which a toast says.
#[derive(Default)]
struct SingleSeen(HashMap<hacivat_proto::ProfileId, bool>);

impl Global for SingleSeen {}

/// Says "applies after reconnect" when a connected profile's single-session setting changed.
fn follow_single_session(cx: &mut App) {
    let Some(model) = cx.try_global::<ConnectionsModel>() else {
        return;
    };
    let now: Vec<(hacivat_proto::ProfileId, bool, String, bool)> = model
        .profiles()
        .iter()
        .map(|p| {
            (
                p.id,
                p.single_session,
                p.name.clone(),
                matches!(model.live(p.id), hacivat_session::LiveState::Connected(_)),
            )
        })
        .collect();
    let mut changed: Vec<String> = Vec::new();
    let seen = cx.default_global::<SingleSeen>();
    for (id, single, name, connected) in now {
        let before = seen.0.insert(id, single);
        if connected && before.is_some_and(|b| b != single) {
            changed.push(name);
        }
    }
    for name in changed {
        let text: hacivat_ui::text::TextFn = Rc::new(move |cx: &App| {
            tr(
                cx,
                ids::hv::CONN_SINGLE_SESSION_RECONNECT,
                ids::hv::HvConnSingleSessionReconnect {
                    name: Text(name.clone()),
                },
            )
        });
        hacivat_ui::toast::ToastQueue::push(
            cx,
            hacivat_ui::toast::Toast::new(hacivat_ui::toast::ToastKind::Info, text),
        );
    }
}

/// A read-only session never runs the profile's startup script (T12, C3): the console's Output
/// says so once per session it takes (T12 carry m9; the script's SQL is never shown).
fn note_script_skipped(console: ConsoleId, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let has_script = state
        .profile
        .and_then(|p| cx.try_global::<ConnectionsModel>()?.get(p))
        .is_some_and(|p| {
            p.startup_script
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
        });
    if !has_script {
        return;
    }
    state
        .log
        .append(hacivat_exec::LogMsg::StartupScriptSkipped, None, None);
    if let Some(results) = state.results {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

/// When the transaction of `snap` opened, on the executor clock: now, less what the session's
/// own clock says it has been open.
fn opened_at(now: Instant, snap: &SessionSnapshot) -> Instant {
    let open_for = snap.tx.since.map_or(Duration::ZERO, |s| s.elapsed());
    now.checked_sub(open_for).unwrap_or(now)
}

/// A watched session changed: repaint when what the console shows did.
fn note(cx: &mut App, id: ConsoleId, snap: &SessionSnapshot) {
    let view = TxView::of(snap);
    let now = cx.background_executor().now();
    let Some(w) = cx.default_global::<TxMirror>().watched.get_mut(&id) else {
        return;
    };
    let swapped = view.user.is_some() && snap.tx.since != w.since;
    w.since = snap.tx.since;
    if w.view == view && !swapped {
        return;
    }
    match (w.view.user, view.user) {
        (_, None) => w.opened = None,
        (None, Some(_)) => w.opened = Some(opened_at(now, snap)),
        (Some(_), Some(_)) if swapped => w.opened = Some(opened_at(now, snap)),
        _ => {}
    }
    w.view = view;
    tick(cx);
    cx.refresh_windows();
}

/// Runs the one-minute ticker while some user transaction is open (the badge's age; no
/// ambient motion), and stops it otherwise.
fn tick(cx: &mut App) {
    let mirror = cx.default_global::<TxMirror>();
    let open = mirror.watched.values().any(|w| w.view.user.is_some());
    match (open, mirror.ticker.is_some()) {
        (true, false) => {
            let task = cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_secs(60))
                        .await;
                    cx.update(|cx| cx.refresh_windows());
                }
            });
            cx.default_global::<TxMirror>().ticker = Some(task);
        }
        (false, true) => cx.default_global::<TxMirror>().ticker = None,
        _ => {}
    }
}

/// What console `console` shows of its session's transaction; `None` without a session.
pub fn view(console: ConsoleId, cx: &App) -> Option<TxView> {
    cx.try_global::<TxMirror>()?
        .watched
        .get(&console)
        .map(|w| w.view)
}

/// The user's transaction on `console`, as the mirror shows it.
pub fn user_tx(console: ConsoleId, cx: &App) -> Option<UserTxView> {
    view(console, cx)?.user
}

/// How many whole minutes the user's transaction on `console` has been open.
pub fn age_minutes(console: ConsoleId, cx: &App) -> Option<u64> {
    let w = cx.try_global::<TxMirror>()?.watched.get(&console)?;
    let opened = w.opened?;
    let now = cx.background_executor().now();
    Some(now.saturating_duration_since(opened).as_secs() / 60)
}

/// The age as text: "<1 min", then whole minutes (`exec-dur-lt-min`, `exec-dur-min`).
pub fn age_text(cx: &App, minutes: u64) -> SharedString {
    if minutes == 0 {
        message(cx, "exec-dur-lt-min", Vec::new())
    } else {
        message(cx, "exec-dur-min", vec![("min", ArgValue::Count(minutes))])
    }
}

/// Says whether a question of console `console`'s run is open (the error-policy bar, the
/// confirm-in-transaction prompt): the badge's and the menu's Commit and Roll back wait for it.
pub fn set_choice_pending(console: ConsoleId, pending: bool, cx: &mut App) {
    let mirror = cx.default_global::<TxMirror>();
    let changed = if pending {
        mirror.pending.insert(console)
    } else {
        mirror.pending.remove(&console)
    };
    if changed {
        cx.refresh_windows();
    }
}

/// Whether Commit and Roll back must wait on `console`: a question is open, or the session
/// runs a request of the user's.
pub fn answer_waits(console: ConsoleId, cx: &App) -> bool {
    let Some(m) = cx.try_global::<TxMirror>() else {
        return false;
    };
    m.pending.contains(&console) || m.watched.get(&console).is_some_and(|w| w.view.busy)
}

/// The engine of console `console`'s connection.
pub(crate) fn engine_of(console: ConsoleId, cx: &App) -> Option<DbmsKind> {
    let profile = Consoles::get(cx, console)?.profile?;
    Some(cx.try_global::<ConnectionsModel>()?.get(profile)?.engine)
}

/// The connection name of console `console` (data).
pub(crate) fn connection_name(console: ConsoleId, cx: &App) -> Option<String> {
    let profile = Consoles::get(cx, console)?.profile?;
    Some(
        cx.try_global::<ConnectionsModel>()?
            .get(profile)?
            .name
            .clone(),
    )
}

/// The badge's text: with the age, or (`short`) without it.
pub fn badge_text(console: ConsoleId, short: bool, cx: &App) -> Option<SharedString> {
    let user = user_tx(console, cx)?;
    if user.failed {
        return Some(tr(cx, ids::hv::TX_BADGE_FAILED, NoArgs));
    }
    let n = Count(u64::from(user.statements));
    Some(if short {
        tr(cx, ids::hv::TX_BADGE_SHORT, ids::hv::HvTxBadgeShort { n })
    } else {
        let age = age_text(cx, age_minutes(console, cx).unwrap_or(0));
        tr(
            cx,
            ids::hv::TX_BADGE,
            ids::hv::HvTxBadge {
                n,
                age: Text(age.to_string()),
            },
        )
    })
}

/// The badge of console `console` (its full form); `None` without a user transaction.
pub fn render(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    badge(console, false, cx)
}

/// The badge's compact form: no age, icon-only buttons (the words stay their tooltips).
pub fn render_compact(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    badge(console, true, cx)
}

fn badge(console: ConsoleId, compact: bool, cx: &App) -> Option<AnyElement> {
    let user = user_tx(console, cx)?;
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = &t.colors;
    let text = badge_text(console, compact, cx)?;
    let tip = if user.failed {
        tr(cx, ids::hv::TX_BADGE_FAILED_TIP, NoArgs)
    } else {
        tr(cx, ids::hv::TX_BADGE_TIP, NoArgs)
    };
    let waits = answer_waits(console, cx);
    let glyph = if user.failed {
        Icon::new(IconName::TriangleAlert)
            .size(IconSize::Chrome)
            .color(c.text.to_hsla())
            .into_any_element()
    } else {
        run_marker(&theme).into_any_element()
    };
    let head = div()
        .id("tx-badge-head")
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(t.spacing.sm))
        .child(glyph)
        .child(
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            )
            .id(if user.failed {
                "tx-badge-failed"
            } else {
                "tx-badge-text"
            }),
        )
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx));
    let mut row = div()
        .id("tx-badge")
        .debug_selector(|| "tx-badge".to_string())
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(t.spacing.sm))
        // The seam before the badge.
        .child(div().w(px(1.0)).h(px(16.0)).bg(c.seam.to_hsla()))
        .child(head);
    if !user.failed {
        row = row.child(end_button(console, true, compact, waits, cx));
    }
    row = row.child(end_button(console, false, compact, waits, cx));
    Some(row.into_any_element())
}

/// Commit (`commit`) or Roll back: a Default button with its ink icon; icon-only when
/// `compact` (the word is its tooltip and accessible name).
fn end_button(
    console: ConsoleId,
    commit: bool,
    compact: bool,
    waits: bool,
    cx: &App,
) -> AnyElement {
    let (id, label, icon) = if commit {
        (
            "tx-badge-commit",
            tr(cx, ids::hv::TX_BADGE_COMMIT, NoArgs),
            // NOTES surface 4: Commit (check), Roll back (undo).
            IconName::Check,
        )
    } else {
        (
            "tx-badge-rollback",
            tr(cx, ids::hv::TX_BADGE_ROLLBACK, NoArgs),
            IconName::Undo2,
        )
    };
    let tip = label.clone();
    let button = Button::new(id, label)
        .kind(if compact {
            ButtonKind::Icon
        } else {
            ButtonKind::Default
        })
        .icon(icon)
        .on_raised(true)
        .disabled(waits)
        .on_click(move |_, _, cx| {
            if !answer_waits(console, cx) && user_tx(console, cx).is_some() {
                crate::exec_ui::tx::end_tx(console, commit, cx).detach();
            }
        });
    if compact {
        div()
            .id(SharedString::from(format!("{id}-tip")))
            .child(button)
            .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
            .into_any_element()
    } else {
        button.into_any_element()
    }
}

// ## test seam

/// The session `console` mirrors, and the badge's actions, for tests: Commit (`commit`) or Roll
/// back as a click on the badge would (nothing while an answer waits).
pub fn press(console: ConsoleId, commit: bool, cx: &mut App) -> bool {
    let Some(user) = user_tx(console, cx) else {
        return false;
    };
    if answer_waits(console, cx) || (commit && user.failed) {
        return false;
    }
    crate::exec_ui::tx::end_tx(console, commit, cx).detach();
    true
}

/// The text's id the badge shows (`tx-badge-text` or `tx-badge-failed`), for the shots' trust
/// check.
pub const TRUST_IDS: [&str; 2] = ["#tx-badge-text", "#tx-badge-failed"];

#[cfg(test)]
mod tests {
    use hacivat_proto::AccessLabel;
    use hacivat_session::session::TxState;

    use super::*;

    fn snap(owner: TxOwner, status: TxStatus, state: SessionState) -> SessionSnapshot {
        SessionSnapshot {
            state,
            tx: TxState {
                owner,
                status,
                statements: 2,
                ..TxState::default()
            },
            schema: None,
            cancel: Default::default(),
            read_only: false,
            access: AccessLabel::ReadWrite,
            generation: 1,
            cursor: None,
        }
    }

    /// Only the user's transaction is shown: never Hacivat's own around a cursor, the guard's
    /// or a confirmed statement's (T19's `Confirm`), and none on a session that ended.
    #[test]
    fn only_the_users_transaction_is_shown() {
        let user = TxView::of(&snap(TxOwner::User, TxStatus::InTx, SessionState::Idle));
        assert_eq!(
            user.user,
            Some(UserTxView {
                failed: false,
                statements: 2
            })
        );
        for owner in [
            TxOwner::Cursor,
            TxOwner::Guard,
            TxOwner::Confirm,
            TxOwner::None,
        ] {
            for status in [TxStatus::InTx, TxStatus::Failed] {
                let v = TxView::of(&snap(owner, status, SessionState::Idle));
                assert_eq!(v.user, None, "{owner:?} {status:?}");
            }
        }
        for state in [
            SessionState::Disconnected,
            SessionState::Broken(hacivat_proto::ErrClass::ALL[0]),
        ] {
            let v = TxView::of(&snap(TxOwner::User, TxStatus::InTx, state));
            assert!(v.ended && v.user.is_none());
        }
        let failed = TxView::of(&snap(TxOwner::User, TxStatus::Failed, SessionState::Idle));
        assert!(failed.user.is_some_and(|u| u.failed));
    }
}
