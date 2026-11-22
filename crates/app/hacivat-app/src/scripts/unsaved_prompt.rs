//! The unsaved-changes prompt (R-M4b-5, R-M4b-39; NOTES surface 6b): Save / Don't save /
//! **Cancel** (focused; Escape and window close answer Cancel), one per dirty script tab on
//! close, quit and restart, after any transaction prompt.
//!
//! ## m4b-t01b-seams: nothing yet.
//!
//! ## m4b-t14-tabs
//! - **In M3's guard chain** (`exec_ui::close_prompt`): the tab close guard and the quit guard
//!   ask about open transactions first; with none (or once it ended) they ask here. The prompt
//!   holds M3's one-prompt slot, so a second close or quit asks nothing more while it is open,
//!   and a window close answers it with Cancel.
//! - **Save** runs the normal save (the secret check and "Overwrite?" can follow); the tab
//!   closes, or the quit goes on, only when it saved. **Don't save** discards the tab's
//!   recovery copy (R-T5-4) and closes it (or lets the quit go on without asking about it
//!   again). **Cancel** keeps everything.
//! - Quit and restart ask for each dirty script tab in turn ("1 of 2 unsaved scripts").

use std::rc::Rc;

use hacivat_l10n::ArgValue;
use hacivat_ui::button::ButtonKind;
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::panes::TabId;
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, Entity, Global, Role, Window};

use crate::exec_ui::close_prompt;
use crate::scripts::save::{self, SaveOutcome};
use crate::scripts::tabs::{self, ScriptDoc, msg};
use crate::shell::{CloseDecision, Workspace};

/// What the prompt is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum For {
    Tab(TabId),
    Quit { restart: bool },
}

/// The prompt's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    DontSave,
    Save,
    Cancel,
}

/// The quit round: how many dirty tabs it asks about, and how many it asked.
#[derive(Debug, Default)]
struct QuitRound {
    total: usize,
    asked: usize,
}

impl Global for QuitRound {}

/// The tab close guard's script part: `Some(Defer)` when tab `tab` is a dirty script tab (the
/// prompt opened, or another prompt is open), `None` to go on closing.
pub(crate) fn close_guard(tab: TabId, window: &mut Window, cx: &mut App) -> Option<CloseDecision> {
    let doc = tabs::doc_of_tab(tab, cx)?;
    if !asks(&doc, cx) {
        return None;
    }
    if close_prompt::prompt_open(cx) {
        return Some(CloseDecision::Defer);
    }
    let ws = window.root::<Workspace>().flatten()?;
    open(&doc, For::Tab(tab), None, &ws, window, cx);
    Some(CloseDecision::Defer)
}

/// Asks about tab `tab` after its transaction prompt ended (the tab closes after the answer);
/// `true` when it asks, `false` when the tab may close now.
pub(crate) fn close_after_tx(tab: TabId, cx: &mut App) -> bool {
    let Some(doc) = tabs::doc_of_tab(tab, cx) else {
        return false;
    };
    if !asks(&doc, cx) {
        return false;
    }
    close_prompt::hold_prompt(None, cx);
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        open(&doc, For::Tab(tab), None, ws, window, cx);
    });
    true
}

/// The quit guard's script part, after every transaction was asked about: `Some(Defer)` while
/// a dirty script tab asks, `None` to quit (or restart).
pub(crate) fn quit_guard(restart: bool, cx: &mut App) -> Option<CloseDecision> {
    let waiting: Vec<Entity<ScriptDoc>> = tabs::all_docs(cx)
        .into_iter()
        .filter(|d| asks(d, cx))
        .collect();
    let Some(doc) = waiting.first().cloned() else {
        cx.set_global(QuitRound::default());
        return None;
    };
    if crate::exec_ui::workspace_window(cx).is_none() {
        tracing::warn!("quit refused: a script tab has unsaved edits and no window can ask");
        return Some(CloseDecision::Defer);
    }
    let round = cx.default_global::<QuitRound>();
    if round.total == 0 {
        round.total = waiting.len();
    }
    round.asked += 1;
    let count = (round.total > 1).then_some((round.asked.min(round.total), round.total));
    close_prompt::hold_prompt(None, cx);
    crate::exec_ui::in_workspace_window(cx, move |ws, window, cx| {
        open(&doc, For::Quit { restart }, count, ws, window, cx);
    });
    Some(CloseDecision::Defer)
}

/// Whether `doc` asks before it closes: dirty, and not answered "Don't save".
fn asks(doc: &Entity<ScriptDoc>, cx: &App) -> bool {
    let d = doc.read(cx);
    d.is_dirty(cx) && !d.discarded
}

/// Opens the prompt for `doc`.
fn open(
    doc: &Entity<ScriptDoc>,
    what: For,
    count: Option<(usize, usize)>,
    ws: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let file = tabs::file_label(doc.read(cx), cx);
    let title: TextFn = Rc::new(move |cx: &App| {
        msg(
            cx,
            "hv-scripts-unsaved-title",
            vec![("file", ArgValue::Text(file.clone()))],
        )
    });
    let body: TextFn = Rc::new(move |cx: &App| {
        let body = msg(cx, "hv-scripts-unsaved-body", Vec::new());
        match count {
            Some((n, total)) => {
                let head = msg(
                    cx,
                    "hv-scripts-unsaved-count",
                    vec![
                        ("n", ArgValue::Count(n as u64)),
                        ("total", ArgValue::Count(total as u64)),
                    ],
                );
                format!("{head}\n{body}").into()
            }
            None => body,
        }
    });
    let button = |id: &'static str, kind| DialogButton {
        label: crate::l10n_global::text_fn_id(id),
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    // Don't save (Default, far left) · Save (Default) · Cancel (Primary, focused); controller
    // ruling (e): the answers to an app-raised question are never Danger.
    let spec = DialogSpec {
        title,
        body,
        buttons: vec![
            button("hv-scripts-unsaved-discard", ButtonKind::Default),
            button("hv-scripts-unsaved-save", ButtonKind::Default),
            button("hv-tx-cancel", ButtonKind::Primary),
        ],
        cancel: 2,
        primary: 2,
        size: DialogSize::Standard,
    };
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    dialog.update(cx, |d, _| d.set_role(Role::AlertDialog));
    close_prompt::hold_prompt(Some(dialog.clone()), cx);
    let doc = doc.clone();
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        let answer = match event {
            DialogEvent::Pressed(0) => Answer::DontSave,
            DialogEvent::Pressed(1) => Answer::Save,
            _ => Answer::Cancel,
        };
        answered(doc.clone(), what, answer, cx);
    })
    .detach();
}

/// The prompt was answered.
fn answered(doc: Entity<ScriptDoc>, what: For, answer: Answer, cx: &mut App) {
    close_prompt::release_prompt(cx);
    match answer {
        Answer::Cancel => {
            cx.set_global(QuitRound::default());
        }
        Answer::DontSave => {
            doc.update(cx, |d, cx| {
                d.discarded = true;
                d.discard_copy(cx);
            });
            proceed(&doc, what, cx);
        }
        Answer::Save => {
            crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
                let target = doc.clone();
                save::save(
                    &doc,
                    window,
                    cx,
                    Some(Box::new(move |outcome, cx| {
                        if outcome == SaveOutcome::Saved {
                            proceed(&target, what, cx);
                        } else {
                            cx.set_global(QuitRound::default());
                        }
                    })),
                );
            });
        }
    }
}

/// The tab closes, or the quit (restart) asks again and goes on.
fn proceed(doc: &Entity<ScriptDoc>, what: For, cx: &mut App) {
    match what {
        For::Tab(tab) => {
            if let Some(console) = doc.read(cx).console() {
                close_prompt::release_console(console, cx);
            }
            crate::exec_ui::in_workspace_window(cx, move |ws, _, cx| {
                ws.update(cx, |ws, cx| ws.force_close_tab(tab, cx));
            });
        }
        For::Quit { restart } => {
            cx.defer(move |cx| {
                let _ = if restart {
                    crate::commands::request_restart(cx)
                } else {
                    crate::commands::request_quit(cx)
                };
            });
        }
    }
}
