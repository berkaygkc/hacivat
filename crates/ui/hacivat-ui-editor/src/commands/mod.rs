//! The editor's commands (UI-EDIT-05, -06, -13; Task 25): every one goes through
//! `EditorModel::dispatch` with its `UserEvent`, never through `on_key_down`. Keys reach them
//! as keymap data (context `Editor`, Task 1b's rows); the app registers each as an `ActionDef`
//! (Task 32). Titles are `hv-cmd-editor-*` (Task 25's FTL block).
//!
//! Task 1b: the table with no-op bodies; Task 25 fills them.

mod edit;
mod movement;
mod run;
mod selection;

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

/// One editor command.
#[derive(Clone, Copy)]
pub struct EditorCommand {
    /// `editor.*`, as the keymap names it.
    pub id: &'static str,
    /// Its title message (`hv-cmd-editor-*`).
    pub title_id: &'static str,
    pub run: fn(&mut EditorView, &mut Window, &mut Context<EditorView>),
    /// Running it closes the open undo group (Run, select all, …).
    pub seals_history: bool,
}

impl std::fmt::Debug for EditorCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorCommand")
            .field("id", &self.id)
            .field("title_id", &self.title_id)
            .field("seals_history", &self.seals_history)
            .finish_non_exhaustive()
    }
}

/// The commands' module-owned state in `EditorView` (P11; Task 25 adds fields as it needs
/// them; keep `Default`): `EditorView::command_state` reads it, the bodies write it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CommandState {
    /// `editor.toggle_tab_moves_focus` (Ctrl+M): Tab and Shift+Tab move the focus instead of
    /// indenting (UI-EDIT-13).
    pub tab_moves_focus: bool,
}

/// Task 1b's body of every command.
fn pending(_: &mut EditorView, _: &mut Window, _: &mut Context<EditorView>) {}

const fn command(id: &'static str, title_id: &'static str, seals_history: bool) -> EditorCommand {
    EditorCommand {
        id,
        title_id,
        run: pending,
        seals_history,
    }
}

/// Every editor command, in palette order.
static COMMANDS: [EditorCommand; 45] = [
    command("editor.move_left", "hv-cmd-editor-move-left", false),
    command("editor.move_right", "hv-cmd-editor-move-right", false),
    command("editor.move_up", "hv-cmd-editor-move-up", false),
    command("editor.move_down", "hv-cmd-editor-move-down", false),
    command("editor.select_left", "hv-cmd-editor-select-left", false),
    command("editor.select_right", "hv-cmd-editor-select-right", false),
    command("editor.select_up", "hv-cmd-editor-select-up", false),
    command("editor.select_down", "hv-cmd-editor-select-down", false),
    command(
        "editor.move_word_left",
        "hv-cmd-editor-move-word-left",
        false,
    ),
    command(
        "editor.move_word_right",
        "hv-cmd-editor-move-word-right",
        false,
    ),
    command(
        "editor.select_word_left",
        "hv-cmd-editor-select-word-left",
        false,
    ),
    command(
        "editor.select_word_right",
        "hv-cmd-editor-select-word-right",
        false,
    ),
    command(
        "editor.move_line_start",
        "hv-cmd-editor-move-line-start",
        false,
    ),
    command("editor.move_line_end", "hv-cmd-editor-move-line-end", false),
    command(
        "editor.select_line_start",
        "hv-cmd-editor-select-line-start",
        false,
    ),
    command(
        "editor.select_line_end",
        "hv-cmd-editor-select-line-end",
        false,
    ),
    command(
        "editor.move_document_start",
        "hv-cmd-editor-move-document-start",
        false,
    ),
    command(
        "editor.move_document_end",
        "hv-cmd-editor-move-document-end",
        false,
    ),
    command(
        "editor.select_document_start",
        "hv-cmd-editor-select-document-start",
        false,
    ),
    command(
        "editor.select_document_end",
        "hv-cmd-editor-select-document-end",
        false,
    ),
    command("editor.page_up", "hv-cmd-editor-page-up", false),
    command("editor.page_down", "hv-cmd-editor-page-down", false),
    command(
        "editor.select_page_up",
        "hv-cmd-editor-select-page-up",
        false,
    ),
    command(
        "editor.select_page_down",
        "hv-cmd-editor-select-page-down",
        false,
    ),
    command("editor.newline", "hv-cmd-editor-newline", false),
    command("editor.tab", "hv-cmd-editor-tab", false),
    command("editor.outdent", "hv-cmd-editor-outdent", false),
    command(
        "editor.delete_backward",
        "hv-cmd-editor-delete-backward",
        false,
    ),
    command(
        "editor.delete_forward",
        "hv-cmd-editor-delete-forward",
        false,
    ),
    command(
        "editor.delete_word_backward",
        "hv-cmd-editor-delete-word-backward",
        false,
    ),
    command(
        "editor.delete_word_forward",
        "hv-cmd-editor-delete-word-forward",
        false,
    ),
    command(
        "editor.delete_to_line_start",
        "hv-cmd-editor-delete-to-line-start",
        false,
    ),
    command("editor.select_all", "hv-cmd-editor-select-all", true),
    command("editor.undo", "hv-cmd-editor-undo", true),
    command("editor.redo", "hv-cmd-editor-redo", true),
    command("editor.copy", "hv-cmd-editor-copy", false),
    command("editor.cut", "hv-cmd-editor-cut", true),
    command("editor.paste", "hv-cmd-editor-paste", true),
    command("editor.escape", "hv-cmd-editor-escape", false),
    command(
        "editor.toggle_tab_moves_focus",
        "hv-cmd-editor-toggle-tab-moves-focus",
        false,
    ),
    command("editor.context_menu", "hv-cmd-editor-context-menu", false),
    command("editor.run", "hv-cmd-editor-run", true),
    command("editor.run_2", "hv-cmd-editor-run-2", true),
    command("editor.run_3", "hv-cmd-editor-run-3", true),
    command(
        "editor.describe_exec_range",
        "hv-cmd-editor-describe-exec-range",
        false,
    ),
];

/// Every editor command (the app registers them; the keymap's `editor.*` rows name them).
pub fn commands() -> &'static [EditorCommand] {
    &COMMANDS
}

/// The command `id`, if it is one.
pub fn command_by_id(id: &str) -> Option<&'static EditorCommand> {
    COMMANDS.iter().find(|c| c.id == id)
}
