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

use hacivat_l10n::ids;
use hacivat_text::UserEvent;
use hacivat_ui::actions::{ActionDef, ActionRegistry};
use hacivat_ui_core::gpui_surface::{App, Entity, Global, SharedString, Subscription, Window};
use hacivat_ui_editor::commands::{ShortcutHints, can_redo, can_undo, redo_kind, undo_kind};
use hacivat_ui_editor::{EDITOR_CONTEXT, EditorEvent, EditorView, FocusedEditor};

use crate::shell::keymap_state::shortcut_label;

/// The focused editor, while it is alive.
pub fn focused_editor(cx: &App) -> Option<Entity<EditorView>> {
    cx.try_global::<FocusedEditor>()?.0.as_ref()?.upgrade()
}

/// An editor command is enabled while an editor has the focus.
fn editor_focused(cx: &App) -> bool {
    focused_editor(cx).is_some()
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
            enabled: editor_focused,
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
        // Run sends nothing on a console whose read-only reconnect is pending (fix round 1,
        // C1): the keys and the menus reach Run here.
        if matches!(id, "editor.run" | "editor.run_2" | "editor.run_3")
            && let Some(console) = crate::console::registry::Consoles::ids(cx)
                .into_iter()
                .find(|c| {
                    crate::console::registry::Consoles::get(cx, *c)
                        .and_then(|s| s.editor.as_ref())
                        .is_some_and(|e| e.entity_id() == editor.entity_id())
                })
            && crate::console::view::run_refused(console, cx)
        {
            return;
        }
        editor.update(cx, |v, cx| {
            v.run_command(id, window, cx);
        });
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
                let sub = cx.subscribe(&editor, |_, event: &EditorEvent, cx| {
                    if matches!(
                        event,
                        EditorEvent::Changed { .. } | EditorEvent::StatusChanged
                    ) {
                        refresh_menu(cx);
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
