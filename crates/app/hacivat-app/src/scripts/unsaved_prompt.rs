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
    /// ## m4b-t14-tabs fix round 1 (I4): the tabs answered "Don't save" in this round. The
    /// answer takes effect only when the quit goes on; a cancelled quit leaves them dirty, asked
    /// again, with their recovery copies.
    dont_save: Vec<hacivat_ui_core::gpui_surface::WeakEntity<ScriptDoc>>,
}

impl QuitRound {
    fn skips(&self, doc: &Entity<ScriptDoc>) -> bool {
        self.dont_save.iter().any(|w| {
            w.upgrade()
                .is_some_and(|d| d.entity_id() == doc.entity_id())
        })
    }
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
    // ## m4b-t14-tabs fix round 1 (minor 7): the slot is held only where a window will ask.
    if crate::exec_ui::workspace_window(cx).is_none() {
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
        .filter(|d| cx.try_global::<QuitRound>().is_none_or(|r| !r.skips(d)))
        .collect();
    let Some(doc) = waiting.first().cloned() else {
        // The quit goes on: the "Don't save" answers of this round take effect now (I4); their
        // copies are removed on the core runtime, and the quit hook waits (fix round 2, m-fr1-5).
        let round = std::mem::take(cx.default_global::<QuitRound>());
        for doc in round.dont_save.iter().filter_map(|w| w.upgrade()) {
            doc.update(cx, |d, cx| {
                d.discarded = true;
                d.discard_copy_at_quit(cx);
            });
        }
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
    // ## m4b-t14-tabs fix round 1 (I5): the env, connection and database the text belongs to
    // (one file may be dirty under two connections, Q2).
    let subtitle = subtitle(doc.read(cx), cx);
    let body: TextFn = Rc::new(move |cx: &App| {
        let body = format!(
            "{subtitle}\n{}",
            msg(cx, "hv-scripts-unsaved-body", Vec::new())
        );
        let body: hacivat_ui_core::gpui_surface::SharedString = body.into();
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
        // ## m4b-t14-tabs fix round 1 (I4): at quit, only noted until the quit goes on.
        Answer::DontSave if matches!(what, For::Quit { .. }) => {
            cx.default_global::<QuitRound>()
                .dont_save
                .push(doc.downgrade());
            proceed(&doc, what, cx);
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

/// The prompt's subtitle: environment · connection · database (I5).
fn subtitle(doc: &ScriptDoc, cx: &App) -> String {
    let tab = doc.spec(cx);
    let profile = tab
        .as_ref()
        .and_then(|t| t.profile)
        .and_then(|p| crate::scripts::open::profile_of(p, cx));
    match (profile, tab.and_then(|t| t.database)) {
        (Some(p), Some(db)) => msg(
            cx,
            "hv-scripts-unsaved-subtitle",
            vec![
                (
                    "env",
                    ArgValue::Text(
                        crate::connections::capsule::env_word(cx, p.environment).to_string(),
                    ),
                ),
                ("conn", ArgValue::Text(tabs::display_name(&p.name))),
                ("database", ArgValue::Text(tabs::display_name(db.as_str()))),
            ],
        )
        .to_string(),
        _ => msg(cx, "hv-scripts-unsaved-subtitle-unbound", Vec::new()).to_string(),
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
