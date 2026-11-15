//! The close and quit prompts (Task 33): closing a console with an open user transaction, or
//! quitting with one, asks to commit, roll back or cancel; a failed transaction offers only roll
//! back and cancel (P16). On `Defer` the prompt closes the tab later through
//! `Workspace::force_close_tab`, or quits through `cx.quit()`.
//!
//! - **Never silently (D-014, R-M3-27).** A console whose session holds the user's transaction
//!   (`TxOwner::User`, open or failed) is not closed, and the app does not quit, until Commit or
//!   Roll back has ended it. The prompt is a 420 px alertdialog: Commit and close, Roll back and
//!   close, **Cancel (focused)**; Enter acts on key-down and presses the focused button (Cancel
//!   at first); Escape, the close button and closing the window all answer Cancel. On a failed
//!   transaction there is no Commit. If Commit or Roll back does not end the transaction,
//!   nothing closes and a toast says so.
//! - **Quit** asks for each console holding a user transaction in turn, then quits.
//! - **One prompt at a time.** While one is open the guards answer `Defer` (a second close or
//!   quit asks nothing more), and a window close answers the open prompt with Cancel.
//! - **Closing a console** (once nothing is open) releases its cursor (`CloseReason::TabClosed`)
//!   and closes its session when it is not the profile's default one; the console's run state
//!   and registry entry go with it.

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_results::CloseReason;
use hacivat_session::session::{ClientId, ClosePolicy, Request};
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{Dialog, DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::panes::TabId;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, Entity, Window};

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::marks::message;
use crate::exec_ui::orchestrator::console_name;
use crate::exec_ui::tx::{UserTx, end_tx, user_tx};
use crate::exec_ui::{ExecUi, chooser, in_workspace_window};
use crate::shell::{CloseDecision, Workspace};

/// What the prompt closes when the transaction ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    Tab(TabId),
    Quit,
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

fn prompt_open(cx: &App) -> bool {
    cx.try_global::<ExecUi>()
        .is_some_and(|e| e.prompt.is_some())
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
            let ws = window.root::<Workspace>().flatten();
            if let Some(ws) = ws {
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

/// The `quit_guard` hook.
pub fn quit_guard(cx: &mut App) -> CloseDecision {
    if prompt_open(cx) {
        // Any close while the prompt is open answers Cancel (NOTES surface 13).
        answer_cancel(cx);
        return CloseDecision::Defer;
    }
    match next_open_tx(cx) {
        // Without a window there is nobody to ask: the session's own close decides.
        Some(_) if crate::exec_ui::workspace_window(cx).is_none() => CloseDecision::Proceed,
        Some((console, tx)) => {
            // Taken now, so a second close before the prompt opens asks nothing more.
            cx.default_global::<ExecUi>().prompt = Some(OpenPrompt { dialog: None });
            in_workspace_window(cx, move |ws, window, cx| {
                open_prompt(console, tx, Closing::Quit, ws, window, cx);
            });
            CloseDecision::Defer
        }
        None => CloseDecision::Proceed,
    }
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
    let quit = closing == Closing::Quit;
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
    let body: TextFn = if tx.failed {
        Rc::new(move |cx: &App| message(cx, "hv-tx-failed-body", vec![("n", ArgValue::Count(n))]))
    } else if quit {
        crate::l10n_global::text_fn_id("hv-tx-quit-body")
    } else {
        crate::l10n_global::text_fn_id("hv-tx-close-body")
    };
    let (commit_id, rollback_id) = if quit {
        ("hv-tx-commit-quit", "hv-tx-rollback-quit")
    } else {
        ("hv-tx-commit-close", "hv-tx-rollback-close")
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
    // The workspace closes the layer on any event; the answer is ours.
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        let answer = match event {
            DialogEvent::Pressed(i) => answers.get(*i).copied().unwrap_or(Answer::Cancel),
            DialogEvent::Cancelled => Answer::Cancel,
        };
        answered(console, closing, answer, cx);
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
    // A failed transaction is never committed (P16), whatever asked.
    if commit && user_tx(console, cx).is_some_and(|t| t.failed) {
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
                in_workspace_window(cx, move |ws, _, cx| {
                    ws.update(cx, |ws, cx| ws.force_close_tab(tab, cx));
                });
            }
            // The next console holding a transaction asks in turn; then the app quits.
            Closing::Quit => {
                let _ = crate::commands::request_quit(cx);
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
