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
pub mod marks;
pub mod orchestrator;
pub mod tx;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_exec::{ExecService, ExecUnit, SplitOracle};
use hacivat_results::MemoryBudget;
use hacivat_session::session::{RunId, SessionHandle};
use hacivat_ui::menu::PopupMenu;
use hacivat_ui_core::gpui_surface::{
    AnyView, AnyWindowHandle, App, Entity, Global, Subscription, Task, WeakEntity, Window,
};
use hacivat_ui_editor::{EditorView, MarkKey};
use sql_split::exec_range::{CandidateKind, TextRange};

pub use orchestrator::{agrees, attach, handle_results_event, rebind, resolve_for_run};

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
    pub(crate) run: Option<ActiveRun>,
    /// The open chooser, and its subscriptions (kept until it closes).
    pub(crate) chooser: Option<Entity<PopupMenu>>,
    pub(crate) chooser_subscriptions: Vec<Subscription>,
    /// The chooser's layer view (the menu under its key filter).
    pub(crate) chooser_host: Option<AnyView>,
    /// The unit behind each result tab (`TabKey::unit_ordinal`) and its mark, for Re-run.
    pub(crate) tab_units: HashMap<u32, (ExecUnit, MarkKey)>,
    /// The session watcher (SQL context, connected state) and the display setup.
    pub(crate) _tasks: Vec<Task<()>>,
}

impl ConsoleExec {
    fn new(editor: WeakEntity<EditorView>) -> Self {
        ConsoleExec {
            editor,
            remembered: None,
            request: 0,
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
