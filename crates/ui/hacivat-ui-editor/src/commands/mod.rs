//! The editor's commands (UI-EDIT-05, -06, -13; Task 25): every one goes through
//! `EditorModel::dispatch` with its `UserEvent`, never through `on_key_down`. Keys reach them
//! as keymap data (context `Editor`, Task 1b's rows); the app registers each as an `ActionDef`
//! (Task 32). Titles are `hv-cmd-editor-*` (Task 25's FTL block).
//!
//! **One path for every edit.** A command builds a `Transaction` and hands it to
//! `EditorView::commit` (`ImeCore::dispatch`, then `EditorView::apply_committed`), so an open
//! IME composition is committed first and the edit never joins its undo group. Undo and redo
//! have no transaction of their own: they end the composition (`ImeCore::unmark`, as the IME
//! core's contract says), ask the model, and hand its `Committed` to `apply_committed`. Nothing
//! here calls `EditorModel::dispatch` itself.
//!
//! **Read-only.** `EditorConfig::read_only` is the view's, not a model filter, so every command
//! that edits refuses by itself ([`change`], undo, redo, cut, paste). Moving, selecting, copy
//! and Run still work.
//!
//! **Single cursor (M3a).** Commands read the main range and leave one range.
//!
//! What the host reads (Tasks 32 and 34), all from this module: [`CommandState`]
//! (`tab_moves_focus`, [`CommandState::menu_open`]), [`status_message`] (the sentence of
//! Describe execution range and of the Tab mode, for the status bar's transient message, on
//! `EditorEvent::StatusChanged`), [`escape_released`] (an Escape that collapsed nothing was not
//! consumed), [`can_undo`] and [`can_redo`] (the Edit menu), [`ShortcutHints`] (the context
//! menu's key hints, which only the app's keymap knows).

mod edit;
mod menu;
mod movement;
mod run;
mod selection;

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::{
    ChangeSet, Origin, SelectionRange, SelectionSet, TokenOracle, Transaction, UserEvent,
};
use hacivat_ui::l10n::{UiArg, UiText};
use hacivat_ui_core::gpui_surface::{AnyElement, App, Context, SharedString, Window};

use crate::view::{EditorEvent, EditorView};

pub use edit::{can_redo, can_undo, redo_kind, undo_kind};
pub use menu::{ShortcutHints, context_menu_entries};
pub use selection::{double_click_range, escape_released};

pub(crate) use selection::double_click;

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

/// Where the editor was when something was noted (an Escape's release, a sentence, a double
/// click): the note holds only while the text and the main selection are still these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Moment {
    version: u64,
    selection: SelectionRange,
}

impl Moment {
    pub(crate) fn of(v: &EditorView) -> Self {
        Moment {
            version: v.model.version(),
            selection: *v.model.selection().main(),
        }
    }
}

/// The commands' module-owned state in `EditorView` (P11): `EditorView::command_state` reads it
/// by reference, the bodies write it.
#[derive(Debug, Default)]
pub struct CommandState {
    /// `editor.toggle_tab_moves_focus` (Ctrl+M): Tab and Shift+Tab move the focus instead of
    /// indenting (UI-EDIT-13).
    pub tab_moves_focus: bool,
    /// The open context menu.
    pub(crate) menu: Option<menu::OpenMenu>,
    /// An Escape that had nothing to collapse: the next Tab leaves the editor.
    pub(crate) released: Option<Moment>,
    /// The last sentence said, and when.
    pub(crate) said: Option<(Moment, SharedString)>,
    /// The string whose content the last double click selected (text version, content bytes):
    /// the next double click in it selects it with its quotes.
    pub(crate) string_content: Option<(u64, Range<usize>)>,
}

impl CommandState {
    /// The context menu is open (it has the focus until it closes).
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
}

/// The commands' overlay, drawn by the view over the text: `editor.context_menu`'s `PopupMenu`
/// (Cut, Copy, Paste, Select all · Run, Describe execution range), embedded in
/// `deferred(anchored(..))` (the `Select` pattern) at the pointer or under the caret.
pub fn overlay(v: &EditorView, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    menu::overlay(v, window, cx)
}

/// The transient sentence for the status bar ("Run executes lines 12–14: …", the Tab mode),
/// until the text or the selection changes. The host reads it on `EditorEvent::StatusChanged`.
pub fn status_message(v: &EditorView) -> Option<SharedString> {
    let (at, text) = v.commands.said.as_ref()?;
    (*at == Moment::of(v)).then(|| text.clone())
}

/// Says `text`: a polite announcement, and the status bar's transient message.
pub(crate) fn say(v: &mut EditorView, text: SharedString, cx: &mut Context<EditorView>) {
    v.announce(text.clone());
    v.commands.said = Some((Moment::of(v), text));
    cx.emit(EditorEvent::StatusChanged);
    cx.notify();
}

/// The message `id` with `args` from the installed provider (`id` itself when there is none).
pub(crate) fn message(cx: &App, id: &'static str, args: &[(&'static str, UiArg)]) -> SharedString {
    match cx.try_global::<UiText>() {
        Some(provider) => provider.0.text(id, args),
        None => SharedString::from(id),
    }
}

/// The language service's words (tokens when it has them, else character classes).
pub(crate) fn words(v: &EditorView) -> Rc<dyn TokenOracle> {
    v.service.token_oracle(v.doc)
}

/// A command acted: notes that held only until the next command go (the double click's
/// string), whatever the command changed.
fn acted(v: &mut EditorView) {
    v.commands.string_content = None;
}

/// Makes `range` the selection (one range), as `event`; nothing when it already is.
pub(crate) fn select(
    v: &mut EditorView,
    range: SelectionRange,
    event: UserEvent,
    cx: &mut Context<EditorView>,
) {
    acted(v);
    let now = v.model.selection();
    if now.ranges().len() == 1 && *now.main() == range {
        return;
    }
    let len = v.model.buffer().len_bytes();
    let ann = v.annotations(event, Origin::User, cx);
    v.commit(
        Transaction {
            changes: ChangeSet::identity(len),
            selection: Some(SelectionSet::single(range)),
            ann,
        },
        cx,
    );
}

/// Applies `edits` as one transaction of `event`, with `selection` after it (`None`: the
/// selection mapped through the edits). Refused on a read-only editor; nothing when there is no
/// edit. `true` when it committed.
pub(crate) fn change(
    v: &mut EditorView,
    edits: Vec<(Range<usize>, Arc<str>)>,
    selection: Option<SelectionSet>,
    event: UserEvent,
    cx: &mut Context<EditorView>,
) -> bool {
    acted(v);
    if v.config.read_only || edits.is_empty() {
        return false;
    }
    let len = v.model.buffer().len_bytes();
    let Ok(changes) = ChangeSet::from_edits(len, edits) else {
        return false;
    };
    let ann = v.annotations(event, Origin::User, cx);
    v.commit(
        Transaction {
            changes,
            selection,
            ann,
        },
        cx,
    )
    .is_some()
}

const fn command(
    id: &'static str,
    title_id: &'static str,
    run: fn(&mut EditorView, &mut Window, &mut Context<EditorView>),
    seals_history: bool,
) -> EditorCommand {
    EditorCommand {
        id,
        title_id,
        run,
        seals_history,
    }
}

/// Every editor command, in palette order.
#[rustfmt::skip]
static COMMANDS: [EditorCommand; 45] = [
    command("editor.move_left", "hv-cmd-editor-move-left", movement::move_left, false),
    command("editor.move_right", "hv-cmd-editor-move-right", movement::move_right, false),
    command("editor.move_up", "hv-cmd-editor-move-up", movement::move_up, false),
    command("editor.move_down", "hv-cmd-editor-move-down", movement::move_down, false),
    command("editor.select_left", "hv-cmd-editor-select-left", movement::select_left, false),
    command("editor.select_right", "hv-cmd-editor-select-right", movement::select_right, false),
    command("editor.select_up", "hv-cmd-editor-select-up", movement::select_up, false),
    command("editor.select_down", "hv-cmd-editor-select-down", movement::select_down, false),
    command("editor.move_word_left", "hv-cmd-editor-move-word-left", movement::move_word_left, false),
    command("editor.move_word_right", "hv-cmd-editor-move-word-right", movement::move_word_right, false),
    command("editor.select_word_left", "hv-cmd-editor-select-word-left", movement::select_word_left, false),
    command("editor.select_word_right", "hv-cmd-editor-select-word-right", movement::select_word_right, false),
    command("editor.move_line_start", "hv-cmd-editor-move-line-start", movement::move_line_start, false),
    command("editor.move_line_end", "hv-cmd-editor-move-line-end", movement::move_line_end, false),
    command("editor.select_line_start", "hv-cmd-editor-select-line-start", movement::select_line_start, false),
    command("editor.select_line_end", "hv-cmd-editor-select-line-end", movement::select_line_end, false),
    command("editor.move_document_start", "hv-cmd-editor-move-document-start", movement::move_document_start, false),
    command("editor.move_document_end", "hv-cmd-editor-move-document-end", movement::move_document_end, false),
    command("editor.select_document_start", "hv-cmd-editor-select-document-start", movement::select_document_start, false),
    command("editor.select_document_end", "hv-cmd-editor-select-document-end", movement::select_document_end, false),
    command("editor.page_up", "hv-cmd-editor-page-up", movement::page_up, false),
    command("editor.page_down", "hv-cmd-editor-page-down", movement::page_down, false),
    command("editor.select_page_up", "hv-cmd-editor-select-page-up", movement::select_page_up, false),
    command("editor.select_page_down", "hv-cmd-editor-select-page-down", movement::select_page_down, false),
    command("editor.newline", "hv-cmd-editor-newline", edit::newline, false),
    command("editor.tab", "hv-cmd-editor-tab", edit::tab, false),
    command("editor.outdent", "hv-cmd-editor-outdent", edit::outdent, false),
    command("editor.delete_backward", "hv-cmd-editor-delete-backward", edit::delete_backward, false),
    command("editor.delete_forward", "hv-cmd-editor-delete-forward", edit::delete_forward, false),
    command("editor.delete_word_backward", "hv-cmd-editor-delete-word-backward", edit::delete_word_backward, false),
    command("editor.delete_word_forward", "hv-cmd-editor-delete-word-forward", edit::delete_word_forward, false),
    command("editor.delete_to_line_start", "hv-cmd-editor-delete-to-line-start", edit::delete_to_line_start, false),
    command("editor.select_all", "hv-cmd-editor-select-all", selection::select_all, true),
    command("editor.undo", "hv-cmd-editor-undo", edit::undo, true),
    command("editor.redo", "hv-cmd-editor-redo", edit::redo, true),
    command("editor.copy", "hv-cmd-editor-copy", edit::copy, false),
    command("editor.cut", "hv-cmd-editor-cut", edit::cut, true),
    command("editor.paste", "hv-cmd-editor-paste", edit::paste, true),
    command("editor.escape", "hv-cmd-editor-escape", selection::escape, false),
    command("editor.toggle_tab_moves_focus", "hv-cmd-editor-toggle-tab-moves-focus", selection::toggle_tab_moves_focus, false),
    command("editor.context_menu", "hv-cmd-editor-context-menu", menu::context_menu, false),
    command("editor.run", "hv-cmd-editor-run", run::run, true),
    command("editor.run_2", "hv-cmd-editor-run-2", run::run_2, true),
    command("editor.run_3", "hv-cmd-editor-run-3", run::run_3, true),
    command("editor.describe_exec_range", "hv-cmd-editor-describe-exec-range", run::describe_exec_range, false),
];

/// Every editor command (the app registers them; the keymap's `editor.*` rows name them).
pub fn commands() -> &'static [EditorCommand] {
    &COMMANDS
}

/// The command `id`, if it is one.
pub fn command_by_id(id: &str) -> Option<&'static EditorCommand> {
    COMMANDS.iter().find(|c| c.id == id)
}
