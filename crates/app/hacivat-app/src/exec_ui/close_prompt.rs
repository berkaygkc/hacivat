//! The close and quit prompts (Task 33): closing a console with an open user transaction, or
//! quitting with one, asks to commit, roll back or cancel; a failed transaction offers only roll
//! back and cancel (P16). On `Defer` the prompt closes the tab later through
//! `Workspace::force_close_tab`, or quits through `cx.quit()`.
//!
//! - **Never silently (D-014, R-M3-27).** A console whose session holds the user's transaction
//!   (`TxOwner::User`, open or failed) is not closed, the app does not quit or restart, and the
//!   console's session is not released or swapped ([`confirm_release`]), until Commit or Roll
//!   back has ended it. The prompt is a 420 px alertdialog: Commit and close, Roll back and
//!   close, **Cancel (focused)**; Enter acts on key-down and presses the focused button (Cancel
//!   at first); Escape, the close button and closing the window all answer Cancel. On a failed
//!   transaction there is no Commit. If Commit or Roll back does not end the transaction,
//!   nothing closes and a toast says so.
//! - **Quit and restart** (`app.quit`, a window close, `app.restart`, the settings' "Restart
//!   now": `crate::commands::request_restart`) ask for each console holding a user transaction
//!   in turn, then quit or restart.
//! - **One prompt at a time.** While one is open the guards answer `Defer` (a second close or
//!   quit asks nothing more), and a window close answers the open prompt with Cancel. Without a
//!   window to ask in, nothing that would lose a transaction proceeds.
//! - **Closing a console** (once nothing is open) releases its cursor (`CloseReason::TabClosed`)
//!   and closes its session when it is not the profile's default one; the console's run state
//!   and registry entry go with it.

use std::cell::RefCell;
use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_results::CloseReason;
use hacivat_session::session::{ClientId, ClosePolicy, Request};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::panes::TabId;
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Entity, Role, Window};

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::message;
use crate::exec_ui::orchestrator::console_name;
use crate::exec_ui::tx::{UserTx, end_tx, user_tx};
use crate::exec_ui::{ExecUi, chooser, workspace_window};
use crate::shell::{CloseDecision, Workspace};

/// What runs after a release prompt ended the transaction.
type Then = Rc<RefCell<Option<Box<dyn FnOnce(&mut App)>>>>;

/// What the prompt closes when the transaction ends.
#[derive(Clone)]
enum Closing {
    Tab(TabId),
    Quit,
    Restart,
    /// The console's session is released or swapped (Task 32): `then` does it.
    Release(Then),
}

impl Closing {
    /// The body and the Commit / Roll back labels of an open (not failed) transaction.
    fn ids(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Closing::Tab(_) => (
                "hv-tx-close-body",
                "hv-tx-commit-close",
                "hv-tx-rollback-close",
            ),
            Closing::Quit => (
                "hv-tx-quit-body",
                "hv-tx-commit-quit",
                "hv-tx-rollback-quit",
            ),
            Closing::Restart => (
                "hv-tx-restart-body",
                "hv-tx-commit-restart",
                "hv-tx-rollback-restart",
            ),
            Closing::Release(_) => (
                "hv-tx-release-body",
                "hv-tx-commit-continue",
                "hv-tx-rollback-continue",
            ),
        }
    }
}

/// The open prompt.
pub(crate) struct OpenPrompt {
    dialog: Option<Entity<Dialog>>,
}

/// What a button of the prompt does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Commit,
    RollBack,
    Cancel,
}

// ## m3a-t32-console (fix round 2, M15; named exception): crate-visible, so Task 32's
// release gate tells "asked" from "another prompt is open".
pub(crate) fn prompt_open(cx: &App) -> bool {
    cx.try_global::<ExecUi>()
        .is_some_and(|e| e.prompt.is_some())
}

/// The open prompt's dialog (tests read its role).
pub fn open_prompt_dialog(cx: &App) -> Option<Entity<Dialog>> {
    cx.try_global::<ExecUi>()?.prompt.as_ref()?.dialog.clone()
}

/// The `tab_close_guard` hook.
pub fn tab_close_guard(tab: TabId, window: &mut Window, cx: &mut App) -> CloseDecision {
    let Some(console) = Consoles::by_tab(cx, tab) else {
        return CloseDecision::Proceed;
    };
    if prompt_open(cx) {
        return CloseDecision::Defer;
    }
    match user_tx(console, cx) {
        Some(tx) => {
            if let Some(ws) = window.root::<Workspace>().flatten() {
                open_prompt(console, tx, Closing::Tab(tab), &ws, window, cx);
            }
            CloseDecision::Defer
        }
        None => {
            release_console(console, cx);
            CloseDecision::Proceed
        }
    }
}

/// The `quit_guard` hook: asked for a quit, a window close and a restart
/// (`crate::commands::request_restart`, which says so through `take_restart_intent`).
pub fn quit_guard(cx: &mut App) -> CloseDecision {
    let restart = crate::commands::take_restart_intent(cx);
    if prompt_open(cx) {
        // Any close while the prompt is open answers Cancel (NOTES surface 13).
        answer_cancel(cx);
        return CloseDecision::Defer;
    }
    let Some((console, tx)) = next_open_tx(cx) else {
        return CloseDecision::Proceed;
    };
    if workspace_window(cx).is_none() {
        // Nobody to ask: the app stays rather than lose the transaction silently.
        tracing::warn!("quit refused: a console holds a transaction and no window can ask");
        return CloseDecision::Defer;
    }
    let closing = if restart {
        Closing::Restart
    } else {
        Closing::Quit
    };
    open_prompt_later(console, tx, closing, cx);
    CloseDecision::Defer
}

/// Before console `console`'s session is released or swapped (Task 32's rebind, a production
/// tag's reconnect, `release_session`): with no user transaction `then` runs at once
/// (`Proceed`); with one, the prompt asks ("Commit and continue", "Roll back and continue",
/// Cancel) and `then` runs only once the transaction ended (`Defer`). While another prompt is
/// open nothing is asked and `then` never runs (`Defer`); the caller acts again later. The
/// session must still be closed with `ClosePolicy::RefuseIfOpenTx`, never `RollBack`.
pub fn confirm_release(
    console: ConsoleId,
    cx: &mut App,
    then: impl FnOnce(&mut App) + 'static,
) -> CloseDecision {
    if prompt_open(cx) {
        return CloseDecision::Defer;
    }
    let Some(tx) = user_tx(console, cx) else {
        then(cx);
        return CloseDecision::Proceed;
    };
    if workspace_window(cx).is_none() {
        tracing::warn!("release refused: the console holds a transaction and no window can ask");
        return CloseDecision::Defer;
    }
    let then: Then = Rc::new(RefCell::new(Some(Box::new(then))));
    open_prompt_later(console, tx, Closing::Release(then), cx);
    CloseDecision::Defer
}

/// Opens the prompt in the workspace window after this update (a guard asked inside the
/// window's own update must not update it again). The prompt counts as open at once, so a
/// second close before it shows asks nothing more; if it cannot open, that is undone.
fn open_prompt_later(console: ConsoleId, tx: UserTx, closing: Closing, cx: &mut App) {
    cx.default_global::<ExecUi>().prompt = Some(OpenPrompt { dialog: None });
    cx.defer(move |cx| {
        let opened = workspace_window(cx).is_some_and(|handle| {
            handle
                .update(cx, |_, window, cx| {
                    let Some(ws) = window.root::<Workspace>().flatten() else {
                        return false;
                    };
                    open_prompt(console, tx, closing, &ws, window, cx);
                    true
                })
                .unwrap_or(false)
        });
        if !opened {
            tracing::warn!("the open-transaction prompt could not open");
            cx.default_global::<ExecUi>().prompt = None;
        }
    });
}

/// The first console (by ID) holding a user transaction.
fn next_open_tx(cx: &App) -> Option<(ConsoleId, UserTx)> {
    let mut ids = Consoles::ids(cx);
    ids.sort();
    ids.into_iter()
        .find_map(|id| user_tx(id, cx).map(|tx| (id, tx)))
}

/// Closes the open prompt as Cancel.
fn answer_cancel(cx: &mut App) {
    let dialog = cx
        .default_global::<ExecUi>()
        .prompt
        .as_ref()
        .and_then(|p| p.dialog.clone());
    if let Some(dialog) = dialog {
        dialog.update(cx, |_, cx| cx.emit(DialogEvent::Cancelled));
    }
}

fn button(label: TextFn, kind: ButtonKind) -> DialogButton {
    DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    }
}

/// Opens the prompt for console `console`'s transaction `tx`, closing `closing` once it ends.
fn open_prompt(
    console: ConsoleId,
    tx: UserTx,
    closing: Closing,
    ws: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    chooser::close(console, cx);
    let name = console_name(cx, console);
    let n = u64::from(tx.statements);
    let title: TextFn = {
        let name = name.clone();
        if tx.failed {
            Rc::new(move |cx: &App| {
                message(
                    cx,
                    "hv-tx-failed-title",
                    vec![("name", ArgValue::Text(name.clone()))],
                )
            })
        } else {
            Rc::new(move |cx: &App| {
                message(
                    cx,
                    "hv-tx-close-title",
                    vec![
                        ("name", ArgValue::Text(name.clone())),
                        ("n", ArgValue::Count(n)),
                    ],
                )
            })
        }
    };
    let (body_id, commit_id, rollback_id) = closing.ids();
    let body: TextFn = if tx.failed {
        Rc::new(move |cx: &App| message(cx, "hv-tx-failed-body", vec![("n", ArgValue::Count(n))]))
    } else {
        crate::l10n_global::text_fn_id(body_id)
    };
    let id = crate::l10n_global::text_fn_id;
    // Commit and Roll back are outlines; Cancel is the primary, focused button.
    let (buttons, answers) = if tx.failed {
        (
            vec![
                button(id(rollback_id), ButtonKind::Default),
                button(id("hv-tx-cancel"), ButtonKind::Primary),
            ],
            vec![Answer::RollBack, Answer::Cancel],
        )
    } else {
        (
            vec![
                button(id(commit_id), ButtonKind::Default),
                button(id(rollback_id), ButtonKind::Default),
                button(id("hv-tx-cancel"), ButtonKind::Primary),
            ],
            vec![Answer::Commit, Answer::RollBack, Answer::Cancel],
        )
    };
    let cancel = answers.len() - 1;
    let spec = DialogSpec {
        title,
        body,
        buttons,
        cancel,
        primary: cancel,
        size: DialogSize::Standard,
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    // An alertdialog: it interrupts to ask about work that would be lost.
    dialog.update(cx, |d, _| d.set_role(Role::AlertDialog));
    // The workspace closes the layer on any event; the answer is ours.
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        let answer = match event {
            DialogEvent::Pressed(i) => answers.get(*i).copied().unwrap_or(Answer::Cancel),
            DialogEvent::Cancelled => Answer::Cancel,
        };
        answered(console, closing.clone(), answer, cx);
    })
    .detach();
    cx.default_global::<ExecUi>().prompt = Some(OpenPrompt {
        dialog: Some(dialog),
    });
}

/// The prompt was answered: Cancel keeps everything; Commit or Roll back ends the transaction
/// first and closes only when it ended.
fn answered(console: ConsoleId, closing: Closing, answer: Answer, cx: &mut App) {
    cx.default_global::<ExecUi>().prompt = None;
    let commit = match answer {
        Answer::Cancel => return,
        Answer::Commit => true,
        Answer::RollBack => false,
    };
    // A failed transaction is never committed (P16), whatever asked: say why nothing closed.
    if commit && user_tx(console, cx).is_some_and(|t| t.failed) {
        ToastQueue::push(
            cx,
            Toast::new(
                ToastKind::Error,
                crate::l10n_global::text_fn_id("exec-error-tx-failed"),
            ),
        );
        let name = console_name(cx, console);
        let text: TextFn = Rc::new(move |cx: &App| {
            message(
                cx,
                "hv-tx-end-failed",
                vec![("name", ArgValue::Text(name.clone()))],
            )
        });
        ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
        return;
    }
    let ended = end_tx(console, commit, cx);
    cx.spawn(async move |cx| {
        if !ended.await {
            return;
        }
        cx.update(|cx| match closing {
            Closing::Tab(tab) => {
                release_console(console, cx);
                crate::exec_ui::in_workspace_window(cx, move |ws, _, cx| {
                    ws.update(cx, |ws, cx| ws.force_close_tab(tab, cx));
                });
            }
            // The next console holding a transaction asks in turn; then the app quits or
            // restarts.
            Closing::Quit => {
                let _ = crate::commands::request_quit(cx);
            }
            Closing::Restart => {
                let _ = crate::commands::request_restart(cx);
            }
            Closing::Release(then) => {
                if let Some(then) = then.borrow_mut().take() {
                    then(cx);
                }
            }
        });
    })
    .detach();
}

/// Console `console` closes: its cursor is released (`TabClosed`), its own session (not the
/// profile's default one) is closed, and its run state and registry entry go.
pub(crate) fn release_console(console: ConsoleId, cx: &mut App) {
    chooser::close(console, cx);
    if let Some(state) = Consoles::get(cx, console).cloned()
        && let Some(session) = state.session
    {
        let cursor = session.state().borrow().cursor;
        if let Some(cursor) = cursor {
            let _ = session.submit(Request::CloseCursor {
                result: cursor.result,
                reason: CloseReason::TabClosed,
            });
        }
        if state.client != ClientId::DEFAULT {
            // Never discards a user transaction: the guard asked first, and the session
            // refuses to close over one.
            let closing = session.close(ClosePolicy::RefuseIfOpenTx);
            cx.spawn(async move |_| {
                if closing.await.is_err() {
                    tracing::warn!("a closed console's session kept its open transaction");
                }
            })
            .detach();
        }
    }
    cx.default_global::<ExecUi>().consoles.remove(&console);
    Consoles::remove(cx, console);
}
