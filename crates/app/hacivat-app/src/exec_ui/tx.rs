//! The transaction state of a console (Task 33): the user transaction segment and its failed
//! form.
//!
//! - **Whose transaction (R-M3-27).** Only the user's (`TxOwner::User`) counts: Hacivat's own
//!   transaction around a PostgreSQL cursor and the read-only guard's are never offered to
//!   commit or roll back, and a session without a connection holds none (the T34 review's
//!   rule; `hacivat_exec::session_live`).
//! - **Commands.** `tx.commit` and `tx.rollback` (palette, context `Console`) send `Request::Tx`
//!   on the focused console's session and log `exec-tx-committed` / `exec-tx-rolled-back` in
//!   its output when the transaction ended. Commit is offered only on an open transaction, never
//!   on a failed one (R-M3-16 amended, P16): the session refuses it anyway
//!   (`Tx(FailedNeedsRollback)`, whose own sentence `exec-error-tx-failed` is shown).

use std::rc::Rc;

use hacivat_driver::TxStatus;
use hacivat_exec::{LogMsg, error_title_id, session_live};
use hacivat_session::session::{ExecError, Request, SessionHandle, TxCommand, TxOwner};
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui::text::TextFn;
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Task, Window};

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::cancel::{CONSOLE, focused_console};
use crate::exec_ui::marks::message;
use crate::exec_ui::orchestrator::console_name;

/// The user's transaction on a console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserTx {
    /// The transaction failed: only a rollback ends it.
    pub failed: bool,
    pub statements: u32,
}

/// The user's open or failed transaction on `session`, if it holds one.
pub fn user_tx_of(session: &SessionHandle) -> Option<UserTx> {
    let snap = session.state().borrow().clone();
    if !session_live(&snap.state) || snap.tx.owner != TxOwner::User {
        return None;
    }
    match snap.tx.status {
        TxStatus::InTx => Some(UserTx {
            failed: false,
            statements: snap.tx.statements,
        }),
        TxStatus::Failed => Some(UserTx {
            failed: true,
            statements: snap.tx.statements,
        }),
        TxStatus::Idle => None,
    }
}

/// The user's transaction on console `console`.
pub fn user_tx(console: ConsoleId, cx: &App) -> Option<UserTx> {
    user_tx_of(Consoles::get(cx, console)?.session.as_ref()?)
}

// ## m3b-t22-tx-ui (named exception, review I3): the palette's Commit and Roll back wait
// as the badge's do: never while the console's session runs the user's request or a question
// of its run is open (T11 I1, T19 carry 3).
fn answer_waits(console: ConsoleId, cx: &App) -> bool {
    crate::console::tx_badge::answer_waits(console, cx)
}

fn can_commit(cx: &App) -> bool {
    focused_console(cx)
        .filter(|c| !answer_waits(*c, cx))
        .and_then(|c| user_tx(c, cx))
        .is_some_and(|t| !t.failed)
}

fn can_roll_back(cx: &App) -> bool {
    focused_console(cx)
        .filter(|c| !answer_waits(*c, cx))
        .and_then(|c| user_tx(c, cx))
        .is_some()
}

/// Ends console `console`'s user transaction (`commit`, else roll back). The task answers
/// whether the transaction ended: the session answered without an error and holds no user
/// transaction any more. The console's output says so; a failure is a toast.
pub fn end_tx(console: ConsoleId, commit: bool, cx: &mut App) -> Task<bool> {
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return Task::ready(false);
    };
    let Some(session) = state.session.clone() else {
        return Task::ready(false);
    };
    let command = if commit {
        TxCommand::Commit
    } else {
        TxCommand::Rollback
    };
    let ticket = session.submit(Request::Tx(command));
    let log = state.log.clone();
    let results = state.results.clone();
    let name = console_name(cx, console);
    cx.spawn(async move |cx| {
        let reply = ticket.reply.await;
        // Ended: the session answered without an error, and the connection's transaction
        // status after it is idle (the reply's own, never a guess from a later snapshot).
        let (error, ended): (Option<ExecError>, bool) = match &reply {
            Ok(Ok(summary)) => (None, summary.tx == TxStatus::Idle),
            Ok(Err(e)) => (Some(e.clone()), false),
            Err(_) => (
                Some(ExecError::of(
                    hacivat_session::session::ExecErrorKind::NotConnected,
                )),
                false,
            ),
        };
        drop(session);
        cx.update(|cx| {
            if ended {
                log.append(
                    if commit {
                        LogMsg::TxCommitted
                    } else {
                        LogMsg::TxRolledBack
                    },
                    None,
                    None,
                );
                if let Some(results) = results {
                    results.update(cx, |r, cx| r.note_output(cx));
                }
                return;
            }
            // Why it did not end: the session's own sentence (a failed transaction takes only
            // a rollback, `exec-error-tx-failed`), then that nothing was closed.
            if let Some(e) = &error {
                let id = error_title_id(&e.kind);
                ToastQueue::push(
                    cx,
                    Toast::new(ToastKind::Error, crate::l10n_global::text_fn_id(id)),
                );
            }
            let text: TextFn = Rc::new(move |cx: &App| {
                message(
                    cx,
                    "hv-tx-end-failed",
                    vec![("name", hacivat_l10n::ArgValue::Text(name.clone()))],
                )
            });
            ToastQueue::push(cx, Toast::new(ToastKind::Error, text));
        });
        ended
    })
}

fn run_commit(_window: &mut Window, cx: &mut App) {
    if let Some(console) = focused_console(cx)
        .filter(|c| !answer_waits(*c, cx) && user_tx(*c, cx).is_some_and(|t| !t.failed))
    {
        end_tx(console, true, cx).detach();
    }
}

fn run_roll_back(_window: &mut Window, cx: &mut App) {
    if let Some(console) =
        focused_console(cx).filter(|c| !answer_waits(*c, cx) && user_tx(*c, cx).is_some())
    {
        end_tx(console, false, cx).detach();
    }
}

/// Registers `tx.commit` and `tx.rollback`.
pub(crate) fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    let _ = reg.register(ActionDef {
        id: "tx.commit",
        title_id: "hv-cmd-tx-commit",
        context: CONSOLE,
        run: Rc::new(run_commit),
        enabled: can_commit,
    });
    let _ = reg.register(ActionDef {
        id: "tx.rollback",
        title_id: "hv-cmd-tx-rollback",
        context: CONSOLE,
        run: Rc::new(run_roll_back),
        enabled: can_roll_back,
    });
}
