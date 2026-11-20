//! The editor commands as `ActionDef`s (Task 32): every `hacivat_ui_editor::commands()` entry,
//! context `Editor`, enabled while an editor is focused, run on the `FocusedEditor`; the editor's
//! focus handle registered with `register_focus_context` (by the console host, `view.rs`). The
//! Edit menu then shows them.
//!
//! Also here, because they serve the same commands:
//! - the context menu's key hints (`ShortcutHints`, from the active keymap);
//! - the Edit menu's Undo and Redo labels, keyed by what they would take back ("Undo Typing",
//!   `commands::undo_kind`/`redo_kind`, one Fluent message per kind), through the menus'
//!   title hook (`menus::set_menu_title`); the macOS menu bar is rebuilt only when a label or an
//!   enabled state changes, never per keystroke;
//! - the console host's app-wide observers (`view::install`): Task 1b's `console::install` calls
//!   only this function, and `console/mod.rs` stays Task 1b's (P12).

use std::rc::Rc;
// ## m3c-t34-editor
use std::collections::HashMap;
use std::sync::Arc;

use hacivat_l10n::ids;
// ## m3c-t34-editor
use hacivat_l10n::ArgValue;
use hacivat_session::session::SessionState;
use hacivat_text::UserEvent;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::{App, Entity, Global, SharedString, Subscription, Window};
// ## m3c-t34-editor
use hacivat_ui_core::gpui_surface::EntityId;
use hacivat_ui_editor::commands::{ShortcutHints, can_redo, can_undo, redo_kind, undo_kind};
use hacivat_ui_editor::{EDITOR_CONTEXT, EditorEvent, EditorView, FocusedEditor};

// ## m3c-t34-editor
use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::context::SqlContext;
use crate::shell::keymap_state::shortcut_label;

// ## m3a-f4-ime
/// The command behind Edit > Emoji & Symbols (macOS).
pub const CHARACTER_PALETTE: &str = "app.character_palette";

/// The focused editor, while it is alive.
pub fn focused_editor(cx: &App) -> Option<Entity<EditorView>> {
    cx.try_global::<FocusedEditor>()?.0.as_ref()?.upgrade()
}

/// An editor command is enabled while an editor has the focus.
fn editor_focused(cx: &App) -> bool {
    focused_editor(cx).is_some()
}

// ## m3c-t33-find
/// A find command is enabled while an editor or a console's find field has the focus.
fn editor_or_find_bar_focused(cx: &App) -> bool {
    editor_focused(cx) || crate::console::find_bar::focused_bar(cx).is_some()
}

// ## m3b-t01b-seams (fix round 2, I3)
/// A stub command is never enabled.
fn disabled(_: &App) -> bool {
    false
}

/// Registers the editor commands, the key hints, the Edit menu's labels and the console host's
/// observers.
pub fn register(cx: &mut App) {
    let reg = cx.default_global::<ActionRegistry>();
    for command in hacivat_ui_editor::commands() {
        let id = command.id;
        // A duplicate keeps the first definition (the registry's rule).
        let _ = reg.register(ActionDef {
            id,
            title_id: command.title_id,
            context: EDITOR_CONTEXT,
            run: Rc::new(move |window: &mut Window, cx: &mut App| {
                run_on_focused(id, window, cx);
            }),
            // ## m3b-t01b-seams (fix round 2, I3; named exception): a stub row stays disabled.
            // ## m3c-t33-find (named exception): the find commands run in a focused find bar.
            enabled: if command.stub {
                disabled
            } else if crate::console::find_bar::ROUTED.contains(&id) {
                editor_or_find_bar_focused
            } else {
                editor_focused
            },
        });
    }
    // ## m3a-f4-ime (E7 row 6b): Edit > Emoji & Symbols opens the system character palette
    // (macOS; AppKit adds no such item to a GPUI menu bar). Its text reaches whichever input
    // has the focus, through the platform input handler.
    if cfg!(target_os = "macos") {
        let _ = reg.register(ActionDef {
            id: CHARACTER_PALETTE,
            title_id: ids::hv::CMD_APP_CHARACTER_PALETTE.id,
            context: "Workspace",
            run: Rc::new(|window: &mut Window, _: &mut App| window.show_character_palette()),
            enabled: |_| true,
        });
    }
    cx.set_global(ShortcutHints(Rc::new(|cx: &App, id: &str| {
        shortcut_label(cx, id).map(SharedString::from)
    })));
    crate::menus::set_menu_title(cx, Rc::new(menu_title));
    install_menu_follow(cx);
    crate::console::view::install(cx);
}

/// Runs editor command `id` on the focused editor (nothing without one).
fn run_on_focused(id: &'static str, window: &mut Window, cx: &mut App) {
    if let Some(editor) = focused_editor(cx) {
        editor.update(cx, |v, cx| {
            v.run_command(id, window, cx);
        });
    } else {
        // ## m3c-t33-find (named exception): else the focused find bar's (its field carries
        // the Editor context).
        crate::console::find_bar::run_in_bar(id, window, cx);
    }
}

// ---- the Edit menu's Undo and Redo -----------------------------------------------------------

/// The message of Undo (`redo`: Redo) for an edit of `kind`; the plain command title for a kind
/// without its own word.
pub fn history_title(kind: Option<UserEvent>, redo: bool) -> &'static str {
    let pick = |undo: &'static str, redo_id: &'static str| if redo { redo_id } else { undo };
    match kind {
        Some(UserEvent::InputType | UserEvent::InputTypeCompose) => {
            pick(ids::hv::EDIT_UNDO_TYPING.id, ids::hv::EDIT_REDO_TYPING.id)
        }
        Some(UserEvent::InputPaste) => {
            pick(ids::hv::EDIT_UNDO_PASTE.id, ids::hv::EDIT_REDO_PASTE.id)
        }
        Some(UserEvent::InputDrop) => pick(ids::hv::EDIT_UNDO_DROP.id, ids::hv::EDIT_REDO_DROP.id),
        Some(UserEvent::DeleteBackward | UserEvent::DeleteForward | UserEvent::DeleteSelection) => {
            pick(ids::hv::EDIT_UNDO_DELETE.id, ids::hv::EDIT_REDO_DELETE.id)
        }
        Some(UserEvent::DeleteCut) => pick(ids::hv::EDIT_UNDO_CUT.id, ids::hv::EDIT_REDO_CUT.id),
        Some(UserEvent::Command("indent")) => {
            pick(ids::hv::EDIT_UNDO_INDENT.id, ids::hv::EDIT_REDO_INDENT.id)
        }
        Some(UserEvent::Command("outdent")) => {
            pick(ids::hv::EDIT_UNDO_OUTDENT.id, ids::hv::EDIT_REDO_OUTDENT.id)
        }
        _ => pick(ids::hv::CMD_EDITOR_UNDO.id, ids::hv::CMD_EDITOR_REDO.id),
    }
}

/// The menus' title hook: Undo and Redo of the focused editor by kind; `None` (the command's
/// own title) for every other item.
fn menu_title(id: &str, cx: &App) -> Option<&'static str> {
    let redo = match id {
        "editor.undo" => false,
        "editor.redo" => true,
        _ => return None,
    };
    let editor = focused_editor(cx)?;
    let v = editor.read(cx);
    let kind = if redo { redo_kind(v) } else { undo_kind(v) };
    Some(history_title(kind, redo))
}

/// What the Edit menu shows of the focused editor: Undo's and Redo's titles and whether they
/// can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HistoryLabels {
    undo: Option<(&'static str, bool)>,
    redo: Option<(&'static str, bool)>,
}

fn history_labels(cx: &App) -> HistoryLabels {
    let Some(editor) = focused_editor(cx) else {
        return HistoryLabels::default();
    };
    let v = editor.read(cx);
    HistoryLabels {
        undo: Some((history_title(undo_kind(v), false), can_undo(v))),
        redo: Some((history_title(redo_kind(v), true), can_redo(v))),
    }
}

/// The Edit menu as last built, and the focused editor's subscription that keeps it current.
#[derive(Default)]
struct MenuFollow {
    shown: HistoryLabels,
    editor: Option<(Entity<EditorView>, Subscription)>,
}

impl Global for MenuFollow {}

/// Follows the focused editor: when Undo's or Redo's label or enabled state changes, the menus
/// are rebuilt (macOS reads the labels once per build; the in-window menu reads them as it
/// renders). Typing that keeps "Undo Typing" rebuilds nothing.
fn install_menu_follow(cx: &mut App) {
    cx.set_global(MenuFollow::default());
    cx.observe_global::<FocusedEditor>(|cx| {
        let editor = focused_editor(cx);
        let same = cx
            .try_global::<MenuFollow>()
            .and_then(|f| f.editor.as_ref())
            .map(|(e, _)| e.entity_id())
            == editor.as_ref().map(Entity::entity_id);
        if !same {
            let follow = editor.map(|editor| {
                let sub = cx.subscribe(&editor, |editor, event: &EditorEvent, cx| {
                    if matches!(
                        event,
                        EditorEvent::Changed { .. } | EditorEvent::StatusChanged
                    ) {
                        refresh_menu(cx);
                    }
                    // ## m3c-t34-editor: history recall comes from the focused editor's keys.
                    if let EditorEvent::HistoryRecall { older } = event {
                        recall(editor, *older, cx);
                    }
                });
                (editor, sub)
            });
            cx.default_global::<MenuFollow>().editor = follow;
        }
        refresh_menu(cx);
    })
    .detach();
}

/// Rebuilds the menus when the Edit menu's history items changed.
fn refresh_menu(cx: &mut App) {
    let now = history_labels(cx);
    let follow = cx.default_global::<MenuFollow>();
    if follow.shown == now {
        return;
    }
    follow.shown = now;
    crate::menus::rebuild(cx);
}

/// What the Edit menu's Undo and Redo say now, in the UI language (tests; the in-window menu
/// reads the same hook).
pub fn edit_menu_history_titles(cx: &App) -> (SharedString, SharedString) {
    let title = |id: &str| {
        let message = menu_title(id, cx).unwrap_or(if id == "editor.undo" {
            ids::hv::CMD_EDITOR_UNDO.id
        } else {
            ids::hv::CMD_EDITOR_REDO.id
        });
        crate::l10n_global::tr_id(cx, message)
    };
    (title("editor.undo"), title("editor.redo"))
}

// ## m3c-t34-editor
// ---- the gutter ▶'s gate and history recall (NOTES surface 17) ------------------------------

/// Whether Run would be enabled on console `console` now: its session is idle (not running, not
/// waiting for Commit or Roll back, not cancelling, connected), no run of it is still going,
/// and its SQL context is read. The gutter ▶ shows only then (M3c-15); its click still goes
/// through Run's whole path, which decides again.
pub fn run_would_be_enabled(console: ConsoleId, cx: &App) -> bool {
    let Some(state) = Consoles::get(cx, console) else {
        return false;
    };
    // An unbound console has nothing to run on.
    if state.profile.is_none() {
        return false;
    }
    // ## fix round 1 (M1): a disconnected (or broken, or not yet opened) session: Run connects
    // first, so it is enabled; while connecting, running or cancelling it is not.
    let connected = match state
        .session
        .as_ref()
        .map(|s| s.state().borrow().state.clone())
    {
        None | Some(SessionState::Disconnected | SessionState::Broken(_)) => false,
        Some(SessionState::Idle) => true,
        Some(_) => return false,
    };
    // Connected, Run waits for the SQL context until it is read.
    if connected && crate::exec_ui::sql_context(console, cx) == SqlContext::Pending {
        return false;
    }
    // ## m3c-int (M1): Run's whole gate: nor while a run is being prepared (classification, the
    // parameter prompt, the confirmations, the "as one" chooser) or a read-only reconnect is
    // pending.
    !crate::exec_ui::orchestrator::run_busy(console, cx)
        && !crate::exec_ui::pending_read_only(console, cx)
}

/// The most statements one recall reads from the history.
pub const RECALL_LIMIT: u32 = 500;

/// One editor's recall: the statements read (newest first) and the one it shows.
struct Recall {
    list: Arc<[Arc<str>]>,
    index: usize,
}

/// The recalls of the editors, by editor.
#[derive(Default)]
struct Recalls(HashMap<EntityId, Recall>);

impl Global for Recalls {}

/// The console whose editor is `editor`.
fn console_of(editor: &Entity<EditorView>, cx: &App) -> Option<ConsoleId> {
    Consoles::ids(cx).into_iter().find(|id| {
        Consoles::get(cx, *id)
            .and_then(|s| s.editor.as_ref())
            .is_some_and(|e| e.entity_id() == editor.entity_id())
    })
}

/// `editor.history_prev` (`older`) or `_next` at the last line of an empty or recalled console
/// (the editor asked, `EditorEvent::HistoryRecall`): an empty console reads its profile's
/// newest statements from the history (off the UI thread) and shows the newest; a recalled one
/// steps older or newer. Nothing without a history, a profile, or a statement to show.
fn recall(editor: Entity<EditorView>, older: bool, cx: &mut App) {
    let id = editor.entity_id();
    let empty = editor.read(cx).snapshot().buffer.len_bytes() == 0;
    if !empty {
        let Some(r) = cx.try_global::<Recalls>().and_then(|r| r.0.get(&id)) else {
            return;
        };
        let next = if older {
            r.index + 1
        } else {
            match r.index.checked_sub(1) {
                Some(i) => i,
                None => return,
            }
        };
        if next < r.list.len() {
            let list = r.list.clone();
            show_recalled(&editor, list, next, cx);
        }
        return;
    }
    if !older {
        return;
    }
    let Some(console) = console_of(&editor, cx) else {
        return;
    };
    let (Some(profile), Some(store)) = (
        Consoles::get(cx, console).and_then(|s| s.profile),
        crate::history::store(cx),
    ) else {
        return;
    };
    let read = crate::history::recall(&store, profile, RECALL_LIMIT, cx);
    cx.spawn(async move |cx| {
        let Ok(list) = read.await else {
            return;
        };
        if list.is_empty() {
            return;
        }
        let list: Arc<[Arc<str>]> = list.into();
        cx.update(|cx| {
            // Typed into meanwhile: the recall is dropped.
            if editor.read(cx).snapshot().buffer.len_bytes() == 0 {
                show_recalled(&editor, list, 0, cx);
            }
        });
    })
    .detach();
}

/// Puts statement `index` of `list` in `editor`, saying where it is ("From history: statement
/// 2 of 48 · ⌃⌥↑ older · ⌃⌥↓ newer").
fn show_recalled(editor: &Entity<EditorView>, list: Arc<[Arc<str>]>, index: usize, cx: &mut App) {
    let Some(text) = list.get(index).cloned() else {
        return;
    };
    let key = |id: &str| shortcut_label(cx, id).unwrap_or_default();
    let sentence = crate::exec_ui::marks::message(
        cx,
        "hv-history-recall",
        vec![
            ("i", ArgValue::Count(index as u64 + 1)),
            ("n", ArgValue::Count(list.len() as u64)),
            ("older", ArgValue::Text(key("editor.history_prev"))),
            ("newer", ArgValue::Text(key("editor.history_next"))),
        ],
    );
    let id = editor.entity_id();
    let first = cx
        .default_global::<Recalls>()
        .0
        .insert(id, Recall { list, index })
        .is_none();
    // ## fix round 1 (M4): the entry goes with its editor (a closed console).
    if first {
        cx.observe_release(editor, move |_, cx| {
            if cx.has_global::<Recalls>() {
                cx.global_mut::<Recalls>().0.remove(&id);
            }
        })
        .detach();
    }
    editor.update(cx, |e, cx| e.show_recalled(&text, sentence, cx));
}

// ## m4-t24-live (named exception): the console opener of the explorer's Open data (R-M4-13).

/// Consoles opened with a text to run once they can (their tab, until the run starts).
#[derive(Default)]
struct OpenRuns {
    tabs: Vec<hacivat_ui::panes::TabId>,
    observing: bool,
}

impl Global for OpenRuns {}

/// Opens a new console on `profile` over `text` (focused, in the focused pane) and, with `run`,
/// runs it through the normal Run path once the console can run (its session connected and its
/// SQL context read): the gate, the classifier and the confirmations decide as for any Run.
/// `database` is the database the text was written for; the console's session is the
/// profile's own, so callers pass `run: false` for another one. Answers the new tab.
pub fn open_with_text(
    profile: hacivat_proto::ProfileId,
    database: Option<&str>,
    text: &str,
    run: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<hacivat_ui::panes::TabId> {
    let _ = database;
    let ws = window.root::<crate::shell::Workspace>().flatten()?;
    crate::console::view::queue_text(profile, text.to_string(), cx);
    let tab = ws.update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))?;
    if run {
        let runs = cx.default_global::<OpenRuns>();
        runs.tabs.push(tab);
        if !runs.observing {
            runs.observing = true;
            cx.observe_global::<Consoles>(try_open_runs).detach();
            cx.observe_global::<crate::exec_ui::ExecUi>(try_open_runs)
                .detach();
        }
    }
    Some(tab)
}

/// Whether console `console` can run now (connected, context read, nothing running).
fn can_run_now(console: ConsoleId, cx: &App) -> bool {
    let Some(state) = Consoles::get(cx, console) else {
        return false;
    };
    if state.editor.is_none() {
        return false;
    }
    let idle = state
        .session
        .as_ref()
        .is_some_and(|s| s.state().borrow().state == SessionState::Idle);
    idle && crate::exec_ui::sql_context(console, cx) != SqlContext::Pending
        && !crate::exec_ui::orchestrator::run_busy(console, cx)
        && !crate::exec_ui::pending_read_only(console, cx)
}

/// Runs every queued console that can run now.
fn try_open_runs(cx: &mut App) {
    let Some(runs) = cx.try_global::<OpenRuns>() else {
        return;
    };
    let ready: Vec<(hacivat_ui::panes::TabId, ConsoleId)> = runs
        .tabs
        .iter()
        .filter_map(|tab| Some((*tab, Consoles::by_tab(cx, *tab)?)))
        .filter(|(_, c)| can_run_now(*c, cx))
        .collect();
    if ready.is_empty() {
        return;
    }
    cx.global_mut::<OpenRuns>()
        .tabs
        .retain(|t| !ready.iter().any(|(r, _)| r == t));
    for (_, console) in ready {
        let Some(editor) = Consoles::get(cx, console).and_then(|s| s.editor.clone()) else {
            continue;
        };
        crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.run_command("editor.run", window, cx);
            });
        });
    }
}
