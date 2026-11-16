//! A console's run UI (Task 33): Run resolves the execution units and submits one run; the
//! range chooser; the end-of-line marks; cancel; the transaction segment and the close and quit
//! prompts.
//!
//! Module lines, re-exports and [`install`] are Task 1b's (P12); Task 33 fills the submodules.
//!
//! **What is sent is what the user sees (R-M3-8).** Run resolves on the background executor
//! with `hacivat_exec::resolve_units_with` and **no parser oracle** ([`NoOracle`]): the Run
//! preview (Task 24's `preview_of`) never consults the PostgreSQL oracle, so a region only the
//! oracle could settle previews `Uncertain`, and Run refuses it the same way (R-M3-24: the
//! chooser's "boundary uncertain" state, nothing sent). One rule, one answer: Task 33's
//! property test checks `preview_of` against `resolve_units` over the same snapshot.
//!
//! **Threading (Q4).** Resolution and the document snapshot are built on the background
//! executor; every session reply and every run status is a `tokio::sync` receiver awaited
//! from a GPUI foreground task. Nothing here blocks the UI thread.

pub mod cancel;
pub mod chooser;
pub mod close_prompt;
// ## m3b-t01b-seams: the M3b run UI (stubs that draw nothing and answer the safe value):
// confirmation and confirm-in-transaction (Task 21), cancel escalation and the error-policy bar
// (Task 23), the parameter prompt and the run-policy popover (Task 24). Task 1c's types come
// from `hacivat_exec` (ruling I2).
pub mod commit_prompt;
pub mod confirm;
pub mod error_bar;
pub mod escalation;
pub mod params_prompt;
pub mod policy_popover;
// ## m3a-t33-exec-ui (fix round 1): the SQL context, known or unknown.
pub mod context;
// ## m3a-t35-shots (named exception): the gutter marks' shots fixtures.
#[cfg(feature = "shots")]
pub mod fixtures;
pub mod marks;
pub mod orchestrator;
pub mod tx;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{ExecService, ExecUnit, SplitOracle};
use hacivat_results::{FetchPolicy, MemoryBudget};
use hacivat_session::session::{RunId, SessionHandle, UnitId};
use hacivat_ui::menu::PopupMenu;
use hacivat_ui_core::gpui_surface::{
    AnyView, AnyWindowHandle, App, Entity, Global, Subscription, Task, WeakEntity, Window,
};
use hacivat_ui_editor::{EditorView, MarkKey};
use sql_split::exec_range::{CandidateKind, TextRange};

pub use orchestrator::{agrees, attach, handle_results_event, rebind, resolve_for_run};
// ## m3b-t01b-seams
pub use confirm::AppConfirmPort;
pub use error_bar::{ErrorAsk, on_choice};

use crate::console::ConsoleId;
use crate::shell::{Workspace, set_quit_guard, set_tab_close_guard};

/// Installs the close and quit guards.
pub fn install(cx: &mut App) {
    set_tab_close_guard(cx, Rc::new(close_prompt::tab_close_guard));
    set_quit_guard(cx, Rc::new(close_prompt::quit_guard));
    // ## m3a-t33-exec-ui: the run state of every console, and the console commands.
    cx.default_global::<ExecUi>();
    cancel::register(cx);
    tx::register(cx);
}

// ## m3a-t33-exec-ui

/// The parser oracle Run uses: none (module docs). Every `Ambiguous` boundary stays uncertain,
/// exactly as the preview draws it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOracle;

impl SplitOracle for NoOracle {
    fn split(&self, text: &str) -> Option<Vec<TextRange>> {
        let _ = text;
        None
    }
}

/// The run state of every console, and the app's run service.
#[derive(Default)]
pub(crate) struct ExecUi {
    pub(crate) consoles: HashMap<ConsoleId, ConsoleExec>,
    service: Option<Arc<ExecService>>,
    /// The open-transaction prompt, while one is open (one at a time).
    pub(crate) prompt: Option<close_prompt::OpenPrompt>,
}

impl Global for ExecUi {}

/// One console's run state.
pub(crate) struct ConsoleExec {
    pub(crate) editor: WeakEntity<EditorView>,
    /// The chooser kind this console remembers for the session (R-M3-36).
    pub(crate) remembered: Option<CandidateKind>,
    /// The latest Run request; a resolution that lands after a newer one is dropped.
    pub(crate) request: u64,
    /// The SQL-context probe in flight (its request number): `exec.cancel` leaves it alone.
    pub(crate) probe: Option<u64>,
    /// What the console knows of its SQL context.
    pub(crate) context: context::SqlContext,
    pub(crate) run: Option<ActiveRun>,
    /// The open chooser, and its subscriptions (kept until it closes).
    pub(crate) chooser: Option<Entity<PopupMenu>>,
    pub(crate) chooser_subscriptions: Vec<Subscription>,
    /// The chooser's layer view (the menu under its key filter).
    pub(crate) chooser_host: Option<AnyView>,
    /// The unit behind each result tab (`TabKey::unit_ordinal`) and its mark, for Re-run.
    pub(crate) tab_units: HashMap<UnitId, (ExecUnit, MarkKey)>,
    /// The session watcher (SQL context, connected state) and the display setup.
    pub(crate) _tasks: Vec<Task<()>>,
}

impl ConsoleExec {
    fn new(editor: WeakEntity<EditorView>) -> Self {
        ConsoleExec {
            editor,
            remembered: None,
            request: 0,
            probe: None,
            context: context::SqlContext::Pending,
            run: None,
            chooser: None,
            chooser_subscriptions: Vec::new(),
            chooser_host: None,
            tab_units: HashMap::new(),
            _tasks: Vec::new(),
        }
    }
}

/// A submitted run of a console: its request (to cancel it), its session, its marks, and the
/// foreground tasks that follow its status and its result tabs.
pub(crate) struct ActiveRun {
    pub(crate) run: RunId,
    pub(crate) seq: u64,
    pub(crate) session: SessionHandle,
    pub(crate) units: Vec<ExecUnit>,
    pub(crate) keys: Vec<MarkKey>,
    /// The run's summary arrived (a suspended cursor may still follow).
    pub(crate) ended: bool,
    /// `exec.cancel` was asked for this run.
    pub(crate) cancel_asked: bool,
    /// The run's status (`exec.cancel` reads which unit is in flight).
    pub(crate) status: tokio::sync::watch::Receiver<hacivat_exec::RunStatus>,
    pub(crate) _tasks: Vec<Task<()>>,
}

/// Console `console`'s run state, if it is attached.
pub(crate) fn console_exec(cx: &App, console: ConsoleId) -> Option<&ConsoleExec> {
    cx.try_global::<ExecUi>()?.consoles.get(&console)
}

/// Changes console `console`'s run state (nothing when it is not attached).
pub(crate) fn update_console_exec<R>(
    cx: &mut App,
    console: ConsoleId,
    f: impl FnOnce(&mut ConsoleExec) -> R,
) -> Option<R> {
    cx.default_global::<ExecUi>()
        .consoles
        .get_mut(&console)
        .map(f)
}

/// The app's run service: one, on a result budget of a quarter of the machine's memory (at
/// most 4 GiB, R-M3-19), awaiting its replies on the core runtime (`with_runtime`).
pub(crate) fn exec_service(cx: &mut App) -> Arc<ExecService> {
    if let Some(s) = cx.try_global::<ExecUi>().and_then(|e| e.service.clone()) {
        return s;
    }
    let budget = MemoryBudget::from_ram(hacivat_ui_core::platform::physical_memory_bytes());
    let mut service = ExecService::new(budget);
    if let Some(m) = cx.try_global::<crate::connections::ConnectionsModel>() {
        service = service.with_runtime(m.runtime());
    }
    let service = Arc::new(service);
    cx.default_global::<ExecUi>().service = Some(service.clone());
    service
}

/// The marks of console `console`'s latest run, in unit order (`None` for a mark that is gone):
/// what the gutter shows for it (tests, and the toolbar's run state).
pub fn last_run_marks(
    console: ConsoleId,
    cx: &App,
) -> Vec<Option<hacivat_ui_editor::ExecMarkState>> {
    let Some(exec) = console_exec(cx, console) else {
        return Vec::new();
    };
    let (Some(run), Some(editor)) = (exec.run.as_ref(), exec.editor.upgrade()) else {
        return Vec::new();
    };
    let marks = editor.read(cx).marks();
    run.keys.iter().map(|k| marks.state(*k).cloned()).collect()
}

// ## m3a-f3-ci (fix round 1)
/// Whether the UI has seen the summary of console `console`'s latest run (tests).
pub fn last_run_ended(console: ConsoleId, cx: &App) -> bool {
    console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .is_some_and(|r| r.ended)
}

/// The workspace window (the active one first).
pub(crate) fn workspace_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.active_window()
        .into_iter()
        .chain(cx.windows())
        .find(|w| w.downcast::<Workspace>().is_some())
}

/// Runs `f` in the workspace window, after the current update (a guard asked inside the
/// window's own update must not update it again).
pub(crate) fn in_workspace_window(
    cx: &mut App,
    f: impl FnOnce(&Entity<Workspace>, &mut Window, &mut App) + 'static,
) {
    cx.defer(move |cx| {
        let Some(handle) = workspace_window(cx) else {
            return;
        };
        let _ = handle.update(cx, |_, window, cx| {
            if let Some(ws) = window.root::<Workspace>().flatten() {
                f(&ws, window, cx);
            }
        });
    });
}

/// What console `console` knows of its SQL context (fix round 1; tests and the toolbar).
pub fn sql_context(console: ConsoleId, cx: &App) -> context::SqlContext {
    console_exec(cx, console).map_or(context::SqlContext::Pending, |e| e.context)
}

// ## m3a-t33-exec-ui (fix round 1, Task 32 contract): pending read-only.
/// The consoles Task 32 marked `pending_read_only`: a production-tagged console waiting for the
/// user's open transaction to end before it reconnects read-only.
#[derive(Default)]
struct PendingReadOnly(std::collections::HashSet<ConsoleId>);

impl Global for PendingReadOnly {}

/// Whether console `console` waits to become read-only: Run then sends nothing (Commit and
/// Roll back stay available). **Merge point (Task 32):** Task 32 publishes the flag per
/// console; it either calls [`set_pending_read_only`] or replaces this body with a read of its
/// own flag. Until then nothing sets it, so this answers `false`.
pub fn pending_read_only(console: ConsoleId, cx: &App) -> bool {
    cx.try_global::<PendingReadOnly>()
        .is_some_and(|p| p.0.contains(&console))
}

// ## m3a-t32-console (fix round 2, M18; named exception)
/// Whether console `console` counts as connected for its editor: its SQL context was read (or
/// found unknown) and its session is live; the rule `set_connected` follows.
pub fn connected(console: ConsoleId, cx: &App) -> bool {
    !matches!(sql_context(console, cx), context::SqlContext::Pending)
        && orchestrator::live_session(cx, console).is_some()
}

// ## m3b-t01b-seams

/// The fetch policy of console `console`'s next run (Task 24 calls it when it builds the run;
/// P-M3b-17): M3a's numbers with `results.max_value_bytes` as the cell cap (R-M3-82; the
/// default 1 MiB without settings).
pub fn fetch_policy_for(console: ConsoleId, cx: &App) -> FetchPolicy {
    // ## m3b-t25-results
    let _ = console;
    let limit = cx
        .try_global::<crate::shell::AppState>()
        .map_or_else(hacivat_settings::ValueLimit::default, |s| {
            s.settings.results.max_value_bytes
        });
    FetchPolicy::default().with_cell_cap(limit.get())
}

// ## m3b-t25-results
// The settings' value-size range and the store's cell-cap range are one range (T8 M8):
// `hacivat-results` cannot name `ValueLimit` (no manifest edge), so the two are checked here.
const _: () = assert!(
    hacivat_results::window::MIN_CELL_CAP == hacivat_settings::ValueLimit::MIN
        && hacivat_results::window::MAX_CELL_CAP == hacivat_settings::ValueLimit::MAX
);

/// A receiver that already holds `answer` (the M3b stubs' answers).
pub(crate) fn answered<T>(answer: T) -> tokio::sync::oneshot::Receiver<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    // The receiver is alive here, so the send cannot fail.
    let _ = tx.send(answer);
    rx
}

/// Sets or clears console `console`'s pending read-only flag (Task 32's seam).
pub fn set_pending_read_only(console: ConsoleId, pending: bool, cx: &mut App) {
    let set = &mut cx.default_global::<PendingReadOnly>().0;
    if pending {
        set.insert(console);
    } else {
        set.remove(&console);
    }
}

// ## m3b-t27-history (named exception): the history dialog's Insert.
/// Replaces console `console`'s editor selection with `text` (the caret after it) as one undo
/// group, through the editor's own edit path (`ImeCore::dispatch` + `apply_committed`,
/// `EditorView::replace_selection`). `false`, with nothing changed, while an IME composition is
/// open (never queued: the M3a T27 rule), without an editor, on a read-only editor or past the
/// document size limit.
pub fn insert_at_caret(console: ConsoleId, text: &str, cx: &mut App) -> bool {
    inserted_at_caret(console, text, cx).is_some()
}

// ## m3b-t27-history (named exception)
/// [`insert_at_caret`], answering the bytes the text now covers (Insert and run selects them).
pub(crate) fn inserted_at_caret(
    console: ConsoleId,
    text: &str,
    cx: &mut App,
) -> Option<std::ops::Range<usize>> {
    let editor = crate::console::Consoles::get(cx, console)?.editor.clone()?;
    editor.update(cx, |v, cx| v.replace_selection(text, cx))
}
