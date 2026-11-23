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

// ## m4-t25-ddl-view (named exception, T25 review m-6)
/// The editor's Run commands.
const RUN_COMMANDS: [&str; 3] = ["editor.run", "editor.run_2", "editor.run_3"];

/// Run is enabled while an editor that is not read-only has the focus (a read-only editor, the
/// DDL view's text, has no run host).
fn writable_editor_focused(cx: &App) -> bool {
    focused_editor(cx).is_some_and(|e| !e.read(cx).config().read_only)
}
// ## m4-t25-ddl-view

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
            // ## m4-t25-ddl-view (named exception, T25 review m-6): Run is not offered in a
            // read-only editor (the DDL view's text), where nothing would run it.
            } else if RUN_COMMANDS.contains(&id) {
                writable_editor_focused
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
    // ## m4b-t18-run-as-script fix round 1 (named exception; I1): never on a T2 preview (its
    // gutter ▶ included): only Run as script streams it.
    if state
        .editor
        .as_ref()
        .is_some_and(|e| e.read(cx).preview().is_some())
    {
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

// ## m4-t32-shots (fix round 1, review M-1; named exception)
/// Forgets the recalls of every console on `profile` (its history was cleared): ⌃⌥↑ / ⌃⌥↓
/// no longer step through statements that are gone.
pub(crate) fn forget_recalls(profile: hacivat_proto::ProfileId, cx: &mut App) {
    // ## m4-t33b-carries (T32 re-review RR-1): a read already on its way is dropped too.
    note_cleared(profile, cx);
    let editors: Vec<EntityId> = Consoles::ids(cx)
        .into_iter()
        .filter_map(|id| Consoles::get(cx, id))
        .filter(|s| s.profile == Some(profile))
        .filter_map(|s| s.editor.as_ref().map(|e| e.entity_id()))
        .collect();
    if editors.is_empty() || !cx.has_global::<Recalls>() {
        return;
    }
    let recalls = cx.global_mut::<Recalls>();
    for id in editors {
        recalls.0.remove(&id);
    }
}

// ## m4-t33b-carries (named exception; T32 re-review RR-1)
/// How many times each profile's history was (being) cleared: a recall whose read started
/// under an older count shows nothing, so a read that was already on its way when a clear
/// started never brings the cleared statements back.
#[derive(Default)]
struct ClearedGenerations(HashMap<hacivat_proto::ProfileId, u64>);

impl Global for ClearedGenerations {}

/// A clear of `profile`'s history starts or ended: recalls read before now are stale.
pub(crate) fn note_cleared(profile: hacivat_proto::ProfileId, cx: &mut App) {
    *cx.default_global::<ClearedGenerations>()
        .0
        .entry(profile)
        .or_default() += 1;
}

fn cleared_generation(profile: hacivat_proto::ProfileId, cx: &App) -> u64 {
    cx.try_global::<ClearedGenerations>()
        .and_then(|g| g.0.get(&profile).copied())
        .unwrap_or(0)
}

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
    // ## m4-t33b-carries (RR-1): the clear count the read starts under.
    let generation = cleared_generation(profile, cx);
    cx.spawn(async move |cx| {
        let Ok(list) = read.await else {
            return;
        };
        if list.is_empty() {
            return;
        }
        let list: Arc<[Arc<str>]> = list.into();
        cx.update(|cx| {
            // A clear started since the read did: the list may hold cleared statements.
            if cleared_generation(profile, cx) != generation {
                return;
            }
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

/// A console Open data opened, with the text it must run (fix round 1, I-2).
struct Queued {
    tab: hacivat_ui::panes::TabId,
    /// The generated text: the run happens only while the editor holds exactly it.
    text: String,
    /// The console was seen built (a later absence means its tab closed).
    seen: bool,
    // ## m4-t27-refresh-ui (named exception, T24 residual nit)
    /// The profile the console is opened on.
    profile: hacivat_proto::ProfileId,
    /// A connect of the profile was seen under way (or done): one that then ends
    /// disconnected or failed drops the run, so a later connect never runs it.
    connecting: bool,
    /// The profile had already failed (or was lost) when the run was queued: only a later
    /// attempt's failure drops it (review m-7).
    failed_at_queue: bool,
}

/// Consoles opened with a text to run once they can.
#[derive(Default)]
struct OpenRuns {
    queued: Vec<Queued>,
    observing: bool,
}

impl Global for OpenRuns {}

/// Opens a new console on `profile` over `text` (focused, in the focused pane) and, with `run`,
/// runs it through the normal Run path once the console can run (its session connected and its
/// SQL context read): the gate, the classifier and the confirmations decide as for any Run.
///
/// Fix round 1:
/// - **I-2.** The run is of exactly `text`: it happens only while the editor still holds it
///   (an edit before the session is ready drops the run; the user presses Run), and the queued
///   run is dropped when the console's session breaks or its tab closes.
/// - **I-3.** `database` is the database `text` was written for. A console's session is on the
///   profile's database, so when `database` is not known to be that one (the user's session
///   reports another, or nothing tells which), no console opens and a notice says why: the text
///   never runs against another database.
///
/// Answers the new tab.
pub fn open_with_text(
    profile: hacivat_proto::ProfileId,
    database: Option<&str>,
    text: &str,
    run: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<hacivat_ui::panes::TabId> {
    if let Some(db) = database {
        let current = crate::explorer::live::session_database(profile, cx).or_else(|| {
            cx.try_global::<crate::connections::ConnectionsModel>()
                .and_then(|m| m.get(profile))
                .and_then(|p| p.database.clone())
        });
        if current.as_deref() != Some(db) {
            refuse_other_database(db, current, cx);
            return None;
        }
    }
    let ws = window.root::<crate::shell::Workspace>().flatten()?;
    crate::console::view::queue_text(profile, text.to_string(), cx);
    let tab = ws.update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx))?;
    if run {
        // Review m-7: a connect under way when queued counts as seen.
        let (connecting, failed_at_queue) =
            (connect_under_way(profile, cx), connect_failed(profile, cx));
        let runs = cx.default_global::<OpenRuns>();
        runs.queued.push(Queued {
            tab,
            text: text.to_string(),
            seen: false,
            // ## m4-t27-refresh-ui (named exception)
            profile,
            connecting,
            failed_at_queue,
        });
        if !runs.observing {
            runs.observing = true;
            cx.observe_global::<Consoles>(try_open_runs).detach();
            cx.observe_global::<crate::exec_ui::ExecUi>(try_open_runs)
                .detach();
            // ## m4-t27-refresh-ui (named exception): a connect that ends is seen too.
            cx.observe_global::<crate::connections::ConnectionsModel>(try_open_runs)
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

/// The notice of an Open data refused for another (or an unknown) database.
fn refuse_other_database(database: &str, current: Option<String>, cx: &mut App) {
    let database = database.to_string();
    let text: hacivat_ui::text::TextFn = Rc::new(move |cx: &App| match &current {
        Some(current) => crate::explorer::live::text(
            cx,
            "hv-explorer-open-data-other-db",
            vec![
                ("database", ArgValue::Text(database.clone())),
                ("current", ArgValue::Text(current.clone())),
            ],
        ),
        None => crate::explorer::live::text(
            cx,
            "hv-explorer-open-data-db-unknown",
            vec![("database", ArgValue::Text(database.clone()))],
        ),
    });
    hacivat_ui::toast::ToastQueue::push(
        cx,
        hacivat_ui::toast::Toast::new(hacivat_ui::toast::ToastKind::Info, text),
    );
}

/// What becomes of a queued run now.
enum Fate {
    Wait { seen: bool },
    Run(Entity<EditorView>),
    Drop,
}

/// The fate of `q`'s run.
fn fate(q: &Queued, cx: &App) -> Fate {
    // ## m4-t27-refresh-ui (named exception, T24 residual nit): a connect that ended without a
    // connection (disconnected, failed, lost) drops the run.
    // Review m-7: a failure (or loss) after the run was queued ends an attempt even when the
    // attempt was never seen under way (Connecting and Failed within one cycle).
    if (q.connecting && !connect_under_way(q.profile, cx))
        || (!q.failed_at_queue && connect_failed(q.profile, cx))
    {
        return Fate::Drop;
    }
    let Some(console) = Consoles::by_tab(cx, q.tab) else {
        // Not built yet, or built and since closed.
        return if q.seen {
            Fate::Drop
        } else {
            Fate::Wait { seen: false }
        };
    };
    let Some(state) = Consoles::get(cx, console) else {
        return Fate::Drop;
    };
    let broken = state
        .session
        .as_ref()
        .is_some_and(|s| matches!(s.state().borrow().state, SessionState::Broken(_)));
    if broken {
        return Fate::Drop;
    }
    let Some(editor) = state.editor.clone() else {
        return Fate::Wait { seen: true };
    };
    if editor.read(cx).snapshot().buffer.to_string() != q.text {
        // Edited before it could run: the user's later text never runs by itself.
        return Fate::Drop;
    }
    if can_run_now(console, cx) {
        Fate::Run(editor)
    } else {
        Fate::Wait { seen: true }
    }
}

// ## m4-t27-refresh-ui (named exception)
/// Whether `profile`'s last connect failed or its connection was lost.
fn connect_failed(profile: hacivat_proto::ProfileId, cx: &App) -> bool {
    use hacivat_session::LiveState;
    cx.try_global::<crate::connections::ConnectionsModel>()
        .is_some_and(|m| {
            matches!(
                m.live(profile),
                LiveState::Failed(_) | LiveState::Lost { .. }
            )
        })
}

/// Whether `profile` is connecting, reconnecting or connected.
fn connect_under_way(profile: hacivat_proto::ProfileId, cx: &App) -> bool {
    use hacivat_session::LiveState;
    cx.try_global::<crate::connections::ConnectionsModel>()
        .is_some_and(|m| {
            matches!(
                m.live(profile),
                LiveState::Connecting(_) | LiveState::Reconnecting { .. } | LiveState::Connected(_)
            )
        })
}

/// The runs Open data still waits to start (tests).
pub fn queued_runs(cx: &App) -> usize {
    cx.try_global::<OpenRuns>().map_or(0, |r| r.queued.len())
}

/// Runs every queued console that can run now and drops the runs that may no longer happen.
fn try_open_runs(cx: &mut App) {
    let Some(runs) = cx.try_global::<OpenRuns>() else {
        return;
    };
    if runs.queued.is_empty() {
        return;
    }
    let fates: Vec<Fate> = runs.queued.iter().map(|q| fate(q, cx)).collect();
    // ## m4-t27-refresh-ui (named exception)
    let under_way: Vec<bool> = runs
        .queued
        .iter()
        .map(|q| connect_under_way(q.profile, cx))
        .collect();
    let queued = std::mem::take(&mut cx.global_mut::<OpenRuns>().queued);
    let mut start = Vec::new();
    let mut keep = Vec::new();
    for ((mut q, f), under_way) in queued.into_iter().zip(fates).zip(under_way) {
        match f {
            Fate::Wait { seen } => {
                q.seen |= seen;
                q.connecting |= under_way;
                keep.push(q);
            }
            Fate::Run(editor) => start.push(editor),
            Fate::Drop => {}
        }
    }
    cx.global_mut::<OpenRuns>().queued = keep;
    for editor in start {
        crate::exec_ui::in_workspace_window(cx, move |_, window, cx| {
            editor.update(cx, |e, cx| {
                e.run_command("editor.run", window, cx);
            });
        });
    }
}
