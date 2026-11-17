//! The run orchestrator (Task 33): on `ExecuteRequested`, the doc snapshot, `resolve_units`,
//! the chooser when the range is ambiguous, one `ExecService::run`, the marks and the result
//! tabs.
//!
//! - **Attach.** [`attach`] subscribes to the editor and follows the console's session: when it
//!   holds a connection (at attach, and again after a reconnect) the SQL context is probed
//!   (`exec_ui::context`: a value-returning path on read-write and read-only sessions alike;
//!   no value is an unknown context, never the defaults; R-M3-39) and set on the language
//!   service (`set_sql_context`, then `set_doc_context(connected)`, in that order); while it
//!   holds none the editor is told it is not connected. The console's results
//!   get its format locale, display zone and engine (`ConsoleResults::set_display`).
//! - **Run.** The analysis Run reuses is the language service's `doc_snapshot` at exactly the
//!   request's version, else the request's own snapshot analysed on the background executor
//!   under the console's SQL context and `effective_flavor`. Resolution runs there too, with
//!   [`NoOracle`] (module docs of `exec_ui`). Then, on the UI thread: units run; `Ask` opens
//!   the chooser; nothing to run, an uncertain boundary, an unsupported client command and a
//!   refused unit send nothing and say why. A console whose session holds no connection starts
//!   its profile's connect and runs nothing (R-M3-45). Until its SQL context is read a console
//!   runs nothing ("still reading"); with an unknown context it sends only what every lexing
//!   mode combination resolves alike.
//! - **After a run** whose units hold `SET`, `RESET`, `DISCARD`, `ROLLBACK` or `ABORT`, the SQL
//!   context is probed again (`sqlctx::needs_reprobe`).

use std::sync::Arc;

use hacivat_exec::sqlctx::needs_reprobe;
use hacivat_exec::{
    ErrorView, ExecUnit, LogMsg, RefuseReason, Resolution, RunAction, RunRequest, session_live,
    uncertain_message_id,
};
use hacivat_proto::DbmsKind;
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_session::session::{SessionEvent, SessionHandle, UnitId};
use hacivat_sql_editor::{RopeChunks, effective_flavor, exec_points};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::gpui_surface::{App, Entity, Window};
use hacivat_ui_editor::{DocContext, EditorEvent, EditorView, ExecAction, ExecRequest, MarkKey};
use hacivat_values::valfmt::DisplayZone;
use sql_dialect::{ClientFlavor, DialectCtx};
use sql_split::SqlDocument;

use crate::console::{ConsoleId, Consoles};
use crate::exec_ui::context::{self, SqlContext};
use crate::exec_ui::{
    ActiveRun, ConsoleExec, ExecUi, NoOracle, chooser, console_exec, exec_service, marks,
    update_console_exec,
};
use crate::l10n_global::{L10nGlobal, tr_id};
use crate::results::ResultsEvent;
use crate::results::adapter::ResultDisplay;
// ## m3b-t24-exec-ui
use std::collections::{HashMap, HashSet};

use hacivat_exec::{
    ConnSafety, GateDecision, GatePlan, GateReason, LogArg, ParamAnswers, ParamValueStore,
    Prepared, PreparedUnit, SuppressSet, apply_read_verdicts, gate,
};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_session::session::{RunId, UnitVerdict};
use hacivat_sql_editor::RunPolicy;
use hacivat_ui_core::gpui_surface::Global;
use hacivat_ui_editor::ExecMarkState;
use hacivat_values::BindValue;
use sql_split::ParamSettings;
use tokio::sync::watch;

use crate::exec_ui::params_prompt;

/// Wires console `console`'s editor to the run UI; called once when the console is built.
pub fn attach(console: ConsoleId, editor: &Entity<EditorView>, window: &mut Window, cx: &mut App) {
    let _ = window;
    cx.default_global::<ExecUi>()
        .consoles
        .insert(console, ConsoleExec::new(editor.downgrade()));
    cx.subscribe(
        editor,
        move |_editor, event: &EditorEvent, cx| match event {
            EditorEvent::ExecuteRequested(req) => on_execute(console, req.clone(), cx),
            // An edit under an open chooser makes its candidates stale: it closes, sending nothing.
            EditorEvent::Changed { .. } => chooser::close(console, cx),
            _ => {}
        },
    )
    .detach();
    // ## m3c-t34-editor (named exception): the gutter ▶ shows only while Run would be enabled
    // (NOTES M3c-15), and its click comes back as this Run request.
    let gate: hacivat_ui_editor::gutter::RunGate = std::rc::Rc::new(move |cx: &App| {
        crate::console::commands::run_would_be_enabled(console, cx)
    });
    editor.update(cx, |e, cx| e.set_gutter_run(Some(gate), cx));
    follow_session(console, cx);
    set_display(console, cx);
}

/// Console `console` has a new session (Task 32's rebind, a new connection of its profile): its
/// SQL context is read again from it, and Run waits for that (no split under the old one).
pub fn rebind(console: ConsoleId, cx: &mut App) {
    // Fix round 1 (Minor 11): a chooser or a resolution of the old session's context goes.
    chooser::close(console, cx);
    Consoles::update(cx, console, |s| s.sql = None);
    let had = update_console_exec(cx, console, |e| {
        e.request += 1;
        e.context = SqlContext::Pending;
        e._tasks.clear();
    })
    .is_some();
    if had {
        follow_session(console, cx);
        set_display(console, cx);
    }
}

/// The profile of console `console`, from the connections model.
pub(crate) fn profile_of(cx: &App, console: ConsoleId) -> Option<Arc<Profile>> {
    let id = Consoles::get(cx, console)?.profile?;
    cx.try_global::<crate::connections::ConnectionsModel>()?
        .get(id)
}

/// The console's title: its connection's name (data).
pub(crate) fn console_name(cx: &App, console: ConsoleId) -> String {
    profile_of(cx, console)
        .map(|p| p.name.clone())
        .unwrap_or_default()
}

/// The console's session, while it holds a connection.
pub(crate) fn live_session(cx: &App, console: ConsoleId) -> Option<SessionHandle> {
    let session = Consoles::get(cx, console)?.session.clone()?;
    let live = session_live(&session.state().borrow().state);
    live.then_some(session)
}

// ---- the SQL context and the connected state ---------------------------------------------

/// Follows the console's session: the SQL context is probed whenever it gains a connection
/// (and after a `Reconnected` event), and the editor's `connected` follows it.
fn follow_session(console: ConsoleId, cx: &mut App) {
    let Some(session) = Consoles::get(cx, console).and_then(|s| s.session.clone()) else {
        return;
    };
    let mut states = session.state();
    let watcher = cx.spawn(async move |cx| {
        let mut was_live = None;
        loop {
            let live = session_live(&states.borrow_and_update().state);
            if was_live != Some(live) {
                was_live = Some(live);
                cx.update(|cx| {
                    if live {
                        probe_context(console, cx);
                    } else {
                        set_connected(console, false, cx);
                    }
                });
            }
            if states.changed().await.is_err() {
                break;
            }
        }
    });
    let mut events = session.events();
    let reconnects = cx.spawn(async move |cx| {
        loop {
            match events.recv().await {
                Ok(SessionEvent::Reconnected) => cx.update(|cx| probe_context(console, cx)),
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    update_console_exec(cx, console, |e| e._tasks.extend([watcher, reconnects]));
}

/// Tells the editor whether its console is connected (the preview's `NoConnection`).
fn set_connected(console: ConsoleId, connected: bool, cx: &mut App) {
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    editor.update(cx, |e, cx| {
        let tier = e.status().tier;
        e.set_doc_context(DocContext { connected, tier }, cx);
    });
}

/// Probes the console's SQL context (R-M3-39) and sets it; a session that answers nothing
/// (closed, lost) leaves the context as it was, to be probed at the next connection.
pub(crate) fn probe_context(console: ConsoleId, cx: &mut App) {
    // Fix round 1 (T31 finding): read through a path that returns values on a read-only
    // session, and an unanswered probe is Unknown, never the defaults (`context`).
    context::probe(console, cx, apply_context);
}

/// Takes a probe's outcome: a known context is set; an unknown one keeps Run fail-closed
/// (`SqlContext::Unknown`: only what every mode combination resolves alike is sent), while the
/// language service draws under the dialect's defaults.
fn apply_context(console: ConsoleId, ctx: SqlContext, cx: &mut App) {
    update_console_exec(cx, console, |e| e.context = ctx);
    match ctx {
        SqlContext::Known(ctx) => set_context(console, ctx, cx),
        SqlContext::Unknown(base) => {
            set_context(console, base, cx);
            Consoles::update(cx, console, |s| s.sql = None);
        }
        SqlContext::Pending => {}
    }
}

/// Sets the console's SQL context: the registry, the language service (before the connected
/// state, Task 24 M7), then the editor's `connected`.
fn set_context(console: ConsoleId, ctx: DialectCtx, cx: &mut App) {
    Consoles::update(cx, console, |s| s.sql = Some(ctx));
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    if let Some(service) = Consoles::get(cx, console).and_then(|s| s.service.clone()) {
        let doc = editor.read(cx).doc();
        service.set_sql_context(doc, ctx, ClientFlavor::None, cx);
    }
    set_connected(console, true, cx);
}

/// The console's results take its format locale, display zone and engine; the zone is loaded
/// on the background executor (it reads the time-zone database).
fn set_display(console: ConsoleId, cx: &mut App) {
    let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) else {
        return;
    };
    let engine = profile_of(cx, console).map_or(DbmsKind::Postgres, |p| p.engine);
    let format = cx
        .try_global::<L10nGlobal>()
        .map(|g| g.l10n.snapshot().format.clone());
    let (locale, zone) = match format {
        Some(f) => (f.format_locale, Some(f.time_zone)),
        None => ("und".to_string(), None),
    };
    // ## m3c-t41-chrome (fix round 1, m8): what the console's results were given, so its host
    // re-applies a change made before its first frame too.
    cx.default_global::<DisplayApplied>()
        .0
        .insert(console, (locale.clone(), zone.clone().unwrap_or_default()));
    let load = cx
        .background_executor()
        .spawn(async move { DisplayZone::load(zone.as_deref()) });
    let task = cx.spawn(async move |cx| {
        let zone = load.await;
        cx.update(|cx| {
            results.update(cx, |r, cx| {
                r.set_display(ResultDisplay::new(&locale, zone, engine), cx);
            });
        });
    });
    update_console_exec(cx, console, |e| e._tasks.push(task));
}

// ---- Run ---------------------------------------------------------------------------------

fn run_action(a: ExecAction) -> RunAction {
    match a {
        ExecAction::Execute1 => RunAction::Execute1,
        ExecAction::Execute2 => RunAction::Execute2,
        ExecAction::Execute3 => RunAction::Execute3,
    }
}

/// What Run does at `at` over `doc` (off the UI thread): `hacivat_exec::resolve_units_with`
/// without the parser oracle, **gated by the preview**: Run sends units, or opens the chooser,
/// only where the band (`hacivat_sql_editor::preview_of` over the same snapshot, connected,
/// with the same remembered kind) shows exactly those ranges as runnable. Anything else is
/// refused as uncertain and sends nothing (R-M3-8: what the user sees is what is sent; the
/// band never shows a range Run would not send, and Run never sends what the band calls
/// uncertain). The one known divergence of the two answers is a chooser whose preselected
/// candidate Run would refuse (`COPY … FROM STDIN` in a remembered whole script): the band says
/// uncertain, so Run refuses too.
pub fn resolve_for_run(
    doc: &sql_split::DocSnapshot,
    at: &hacivat_ui_editor::ExecAt,
    action: ExecAction,
    remembered: Option<sql_split::exec_range::CandidateKind>,
) -> Resolution {
    resolve_for_run_with(doc, at, action, remembered, &RunPolicy::default())
}

// ## m3b-t24-exec-ui
/// [`resolve_for_run`] under the console's run policy (CHOOSE-06): Run and its band ask the
/// same question under one policy table (`hacivat_sql_editor::run_policy`), so the gate by the
/// preview holds for "Customize…" and for a selection run as one (R-M3-60).
pub fn resolve_for_run_with(
    doc: &sql_split::DocSnapshot,
    at: &hacivat_ui_editor::ExecAt,
    action: ExecAction,
    remembered: Option<sql_split::exec_range::CandidateKind>,
    custom: &RunPolicy,
) -> Resolution {
    let points = exec_points(at);
    let policy = hacivat_sql_editor::run_policy(action, custom);
    let run = hacivat_exec::resolve_units_in(
        doc,
        &points,
        run_action(action),
        policy,
        remembered,
        &NoOracle,
    );
    let band = hacivat_sql_editor::preview_with(doc, at, action, true, remembered, custom);
    if agrees(&band, &run) {
        run
    } else {
        tracing::warn!("Run and its preview disagree: nothing is sent");
        Resolution::Uncertain(Vec::new())
    }
}

/// Whether two resolutions of the same place send (or offer) exactly the same text: the same
/// units, or the same chooser over the same ranges with the same preselected row, or both
/// nothing to run. Anything that sends nothing never matches a runnable answer.
fn same_runnable(a: &Resolution, b: &Resolution) -> bool {
    let units = |u: &[ExecUnit]| -> Vec<(std::ops::Range<usize>, std::sync::Arc<str>)> {
        u.iter()
            .map(|u| (u.doc_range.clone(), u.text.clone()))
            .collect()
    };
    let asked =
        |c: &[sql_split::exec_range::Candidate]| -> Vec<Vec<sql_split::exec_range::TextRange>> {
            c.iter()
                .map(|c| c.units.iter().map(|u| u.range).collect())
                .collect()
        };
    match (a, b) {
        (Resolution::Units(x), Resolution::Units(y)) => units(x) == units(y),
        (
            Resolution::Ask {
                candidates: x,
                preselect: px,
            },
            Resolution::Ask {
                candidates: y,
                preselect: py,
            },
        ) => px == py && asked(x) == asked(y),
        (Resolution::NothingToRun, Resolution::NothingToRun) => true,
        (Resolution::Units(_) | Resolution::Ask { .. } | Resolution::NothingToRun, _) => false,
        // Neither sends anything.
        _ => !matches!(
            b,
            Resolution::Units(_) | Resolution::Ask { .. } | Resolution::NothingToRun
        ),
    }
}

/// Whether the band `p` shows exactly what `run` does.
pub fn agrees(p: &hacivat_ui_editor::ExecPreview, run: &Resolution) -> bool {
    use hacivat_ui_editor::PreviewState;
    let ranges = |units: &[ExecUnit]| -> Vec<std::ops::Range<usize>> {
        units.iter().map(|u| u.doc_range.clone()).collect()
    };
    let span = |c: &sql_split::exec_range::Candidate| -> Vec<std::ops::Range<usize>> {
        c.units
            .iter()
            .map(|u| usize::from(u.range.start())..usize::from(u.range.end()))
            .collect()
    };
    match (p.state, run) {
        (PreviewState::Units, Resolution::Units(u)) => p.ranges.as_slice() == ranges(u),
        (
            PreviewState::AskPreselected,
            Resolution::Ask {
                candidates,
                preselect,
            },
        ) => candidates
            .get(*preselect)
            .is_some_and(|c| p.ranges.as_slice() == span(c)),
        (PreviewState::Nothing, Resolution::NothingToRun) => true,
        (
            PreviewState::Uncertain,
            Resolution::Uncertain(_)
            | Resolution::UnsupportedClientCommand { .. }
            | Resolution::Refused { .. },
        ) => true,
        _ => false,
    }
}

/// Run, Run 2 or Run 3 at `req`.
fn on_execute(console: ConsoleId, req: ExecRequest, cx: &mut App) {
    chooser::close(console, cx);
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let Some(exec) = console_exec(cx, console) else {
        return;
    };
    // One run at a time per console; Stop first (the toolbar's Run is Stop meanwhile).
    if exec.run.as_ref().is_some_and(|r| !r.ended) {
        return;
    }
    // ## m3b-t24-exec-ui: nor while the previous Run is still being prepared (a parameter
    // prompt or a confirmation open).
    if preparing(console, cx) {
        return;
    }
    let custom = policy_of(console, cx);
    let remembered = exec.remembered;
    let context = exec.context;
    let Some(profile) = profile_of(cx, console) else {
        return;
    };
    if live_session(cx, console).is_none() {
        connect_first(&profile, cx);
        return;
    }
    // Fix round 1 (Task 32 contract): a console waiting for the user's open transaction to end
    // before it reconnects read-only (its profile was marked read-only; ## m3b-t26-prod-unlock:
    // a production tag no longer does this) sends nothing meanwhile; Commit and Roll back stay
    // available (tx.commit / tx.rollback do not ask this).
    if crate::exec_ui::pending_read_only(console, cx) {
        say(console, "hv-exec-pending-read-only", cx);
        return;
    }
    let (ctx, unknown) = match context {
        SqlContext::Known(ctx) => (ctx, false),
        // Fix round 1 (T31 finding): the modes are unknown; Run sends only what every mode
        // combination resolves alike.
        SqlContext::Unknown(base) => (base, true),
        SqlContext::Pending => {
            // The SQL context is still being read: nothing is split under a guessed dialect.
            say(console, "hv-editor-describe-pending", cx);
            return;
        }
    };
    let request = update_console_exec(cx, console, |e| {
        e.request += 1;
        e.request
    })
    .unwrap_or(0);
    let reused = state
        .service
        .as_ref()
        .and_then(|s| s.doc_snapshot(req.doc, req.snapshot.version));
    let buffer = req.snapshot.buffer.clone();
    let (at, action) = (req.at.clone(), req.action);
    let resolve = cx.background_executor().spawn(async move {
        let analyse = |ctx: DialectCtx| {
            let flavor = effective_flavor(&ctx, ClientFlavor::None);
            SqlDocument::from_chunks(&RopeChunks(buffer.clone()), ctx, flavor).snapshot()
        };
        let doc = reused.unwrap_or_else(|| analyse(ctx));
        // ## m3b-t24-exec-ui: under the console's run policy (CHOOSE-06).
        let r = resolve_for_run_with(&doc, &at, action, remembered, &custom);
        // ## m3b-t24-exec-ui: an uncertain boundary offers "Run as one" over its region
        // (R-M3-60), never preselected. Its region is read from this reading; with unknown modes
        // its classification still reads every mode (fail-closed).
        let as_one = match &r {
            Resolution::Uncertain(b) => run_as_one_unit(&doc, &at, action, b),
            _ => None,
        };
        if !unknown {
            return (doc, r, as_one);
        }
        // Unknown modes: every combination must resolve alike, else nothing is sent.
        let alike = context::variants(&ctx).into_iter().all(|v| {
            let other = analyse(v);
            same_runnable(
                &r,
                &resolve_for_run_with(&other, &at, action, remembered, &custom),
            )
        });
        let r = if alike {
            r
        } else {
            Resolution::Uncertain(Vec::new())
        };
        (doc, r, as_one)
    });
    let version = req.snapshot.version;
    cx.spawn(async move |cx| {
        let (doc, resolution, as_one) = resolve.await;
        cx.update(|cx| {
            let current = console_exec(cx, console).is_some_and(|e| e.request == request);
            if current {
                dispatch(console, version, doc, resolution, as_one, cx);
            }
        });
    })
    .detach();
}

/// What Run does with a resolution, on the UI thread.
fn dispatch(
    console: ConsoleId,
    version: u64,
    doc: sql_split::DocSnapshot,
    resolution: Resolution,
    as_one: Option<ExecUnit>,
    cx: &mut App,
) {
    // Fix round 1 (C1): the chooser and the uncertain state show ranges of the text Run was
    // asked on; after an edit they would sit over other text, and Enter would send what the
    // band no longer shows. They are dropped (nothing is sent; the user asks again). Units
    // still run: they are exactly what the band showed when Run was pressed, and their marks
    // are rebased through the edit.
    let current = console_exec(cx, console)
        .and_then(|e| e.editor.upgrade())
        .map(|e| e.read(cx).snapshot().version);
    if current != Some(version) && !matches!(resolution, Resolution::Units(_)) {
        return;
    }
    match resolution {
        Resolution::Units(units) => start_run(console, version, units, cx),
        Resolution::Ask {
            candidates,
            preselect,
        } => chooser::open(console, version, doc, candidates, preselect, cx),
        // ## m3c-t41-chrome (named line): "Nothing to run here" is the status bar's sentence
        // (the Run preview's), announced, never a toast (design/m3b-safe surface 24).
        Resolution::NothingToRun => say_in_status(console, "hv-editor-nothing-to-run", cx),
        Resolution::Uncertain(boundaries) => {
            let mut reasons: Vec<&'static str> = boundaries
                .iter()
                .map(|b| uncertain_message_id(b.reason))
                .collect();
            reasons.dedup();
            // ## m3b-t24-exec-ui: with "Run as one" over the region when it can be sent so
            // (R-M3-60); disabled on a read-only session, whose guard checks one statement at
            // a time.
            let as_one = as_one.map(|unit| {
                let statements = statements_in(&doc, &unit.doc_range);
                chooser::AsOne {
                    version,
                    unit,
                    statements,
                    read_only: live_session(cx, console)
                        .is_some_and(|s| s.state().borrow().read_only),
                }
            });
            chooser::open_uncertain(console, reasons, as_one, cx);
        }
        Resolution::UnsupportedClientCommand { at } => {
            let reason = uncertain_message_id(sql_split::AmbiguityReason::ClientCommand);
            log_not_sent(console, LogMsg::NotStarted(view_of(reason)), at, cx);
            chooser::open_uncertain(console, vec![reason], None, cx);
        }
        Resolution::Refused { at, reason } => {
            // ## m3b-t15-gate (named exception): `RefuseReason::RunAsOne` is gone (a selection
            // run as one runs as one unit, R-M3-60), so the copy refusal is the only reason.
            let msg = match reason {
                RefuseReason::CopyStream => LogMsg::CopyUnsupported,
            };
            log_not_sent(console, msg, at, cx);
            say(console, reason.fluent_id(), cx);
        }
    }
}

/// An error view that only names why nothing was sent.
fn view_of(title_id: &'static str) -> ErrorView {
    ErrorView {
        title_id,
        code: None,
        server_text: None,
        hint_id: None,
        doc_offset: None,
        // ## m3b-t25-results (named exception)
        vendor_code: None,
    }
}

/// Logs that Run sent nothing, pointing at `at`.
fn log_not_sent(console: ConsoleId, msg: LogMsg, at: std::ops::Range<usize>, cx: &mut App) {
    let Some(state) = Consoles::get(cx, console) else {
        return;
    };
    state.log.append(msg, Some(at), None);
    if let Some(results) = state.results.clone() {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

/// Says `id` (a Fluent message without arguments) to the user: a toast, and the editor's
/// polite announcement for assistive technology.
pub(crate) fn say(console: ConsoleId, id: &'static str, cx: &mut App) {
    if let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) {
        let text = tr_id(cx, id);
        editor.update(cx, |e, cx| e.announce(text, cx));
    }
    ToastQueue::push(
        cx,
        Toast::new(ToastKind::Info, crate::l10n_global::text_fn_id(id)),
    );
}

// ## m3c-t41-chrome
/// Says `id` where the status bar already says it (the Run preview's sentence): announced to a
/// screen reader, no toast.
fn say_in_status(console: ConsoleId, id: &'static str, cx: &mut App) {
    if let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) {
        let text = tr_id(cx, id);
        editor.update(cx, |e, cx| e.announce(text, cx));
    }
}

// ## m3c-t41-chrome
/// Gives console `console`'s results the current format locale and display zone again (the
/// console host calls it when the regional settings changed).
pub fn refresh_display(console: ConsoleId, cx: &mut App) {
    set_display(console, cx);
}

// ## m3c-t41-chrome (fix round 1, m8)
/// The format locale and time zone each console's results were last given.
#[derive(Default)]
struct DisplayApplied(std::collections::HashMap<ConsoleId, (String, String)>);

impl Global for DisplayApplied {}

/// The format locale and time zone console `console`'s results were last given (`None` before
/// its attach).
pub fn display_applied(console: ConsoleId, cx: &App) -> Option<(String, String)> {
    cx.try_global::<DisplayApplied>()?.0.get(&console).cloned()
}

/// Run on a console without a connection: the profile's connect starts, nothing runs
/// (R-M3-45).
fn connect_first(profile: &Profile, cx: &mut App) {
    let id = profile.id;
    let connecting = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .is_some_and(|m| {
            matches!(
                m.live(id),
                LiveState::Connecting(_) | LiveState::Reconnecting { .. }
            )
        });
    if !connecting && cx.has_global::<crate::connections::ConnectionsModel>() {
        crate::connections::ConnectionsModel::connect(cx, id, None);
    }
    let name = profile.name.clone();
    let text: hacivat_ui::text::TextFn = std::rc::Rc::new(move |cx: &App| {
        crate::l10n_global::tr(
            cx,
            hacivat_l10n::ids::hv::CONSOLE_CONNECT_FIRST,
            hacivat_l10n::ids::hv::HvConsoleConnectFirst {
                name: hacivat_l10n::Text(name.clone()),
            },
        )
    });
    ToastQueue::push(cx, Toast::new(ToastKind::Info, text));
}

/// Runs `units` on console `console` (resolved at editor `version`): the M3b pipeline
/// ([`pipeline`]: parameters, classification on the pool, the gate, the confirmations), then
/// one `ExecService::run` ([`submit`]). Nothing is sent before every step agreed; a refusal at
/// any step logs why, marks the gutter and says so (no silent path).
pub(crate) fn start_run(console: ConsoleId, version: u64, units: Vec<ExecUnit>, cx: &mut App) {
    // ## m3a-t32-console (fix round 2, N1; named exception): the single submit point refuses a
    // console whose read-only reconnect is pending, whatever reached it (the chooser's confirm,
    // the dispatch after the background resolve, Re-run from a result tab). `on_execute`
    // keeps its early check, so a pending console never opens a chooser.
    if crate::exec_ui::pending_read_only(console, cx) {
        say(console, "hv-exec-pending-read-only", cx);
        return;
    }
    if units.is_empty() {
        return;
    }
    // ## m3b-t24-exec-ui: one run at a time per console, its preparation included.
    if preparing(console, cx)
        || console_exec(cx, console).is_some_and(|e| e.run.as_ref().is_some_and(|r| !r.ended))
    {
        return;
    }
    let Some(session) = live_session(cx, console) else {
        if let Some(p) = profile_of(cx, console) {
            connect_first(&p, cx);
        }
        return;
    };
    let Some(profile) = profile_of(cx, console) else {
        return;
    };
    let (dialect, modes_known) = match console_exec(cx, console).map(|e| e.context) {
        Some(SqlContext::Known(c)) => (c, true),
        Some(SqlContext::Unknown(c)) => (c, false),
        Some(SqlContext::Pending) | None => {
            say(console, "hv-editor-describe-pending", cx);
            return;
        }
    };
    set_preparing(console, true, cx);
    let job = Job {
        console,
        version,
        units,
        profile,
        session,
        dialect,
        modes_known,
    };
    let prepared = pipeline(job, cx);
    cx.spawn(async move |cx| {
        prepared.await;
        cx.update(|cx| set_preparing(console, false, cx));
    })
    .detach();
}

// ## m3b-t24-exec-ui
/// Sends `prepared` as one run of console `console` with the answered `plan`, marks the units at
/// `version` and follows the run's status, result tabs and questions.
#[allow(clippy::too_many_arguments)]
fn submit(
    console: ConsoleId,
    version: u64,
    session: SessionHandle,
    profile: &Profile,
    prepared: Vec<PreparedUnit>,
    plan: GatePlan,
    terms: RunTerms,
    cx: &mut App,
) {
    let run_as_one = terms.run_as_one;
    if crate::exec_ui::pending_read_only(console, cx) {
        say(console, "hv-exec-pending-read-only", cx);
        return;
    }
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    // The session gated is the one sent to: a console that reconnected meanwhile runs nothing
    // (its environment, transaction and read-only state may differ); the user runs again.
    let Some(live) = live_session(cx, console).filter(|s| s.id() == session.id()) else {
        let ranges = prepared.iter().map(|p| p.unit.doc_range.clone()).collect();
        refuse(
            console,
            version,
            ranges,
            (CONNECTION_CHANGED, Vec::new()),
            NOT_SENT,
            cx,
        );
        return;
    };
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    let units: Vec<ExecUnit> = prepared.iter().map(|p| p.unit.clone()).collect();
    let ask_policy = cx
        .try_global::<crate::AppState>()
        .is_none_or(|s| s.settings.exec.error_policy_ask);
    // R-M3-61: a multi-unit run asks after a failure (a unit run as one is one unit).
    let policy = if units.len() > 1 && !run_as_one && ask_policy && !terms.stop_on_error {
        hacivat_session::session::ErrorPolicy::Ask
    } else {
        hacivat_session::session::ErrorPolicy::StopOnError
    };
    let masked: Arc<[Arc<str>]> = prepared.iter().map(|p| p.masked.clone()).collect();
    // Kept per result tab (Count rows reads its binds, T16 `count_prepared`; Task 25).
    let kept = prepared.clone();
    let history = history_meta(console, profile, cx);
    let port = crate::exec_ui::AppConfirmPort::for_console(console, cx);
    let fetch = crate::exec_ui::fetch_policy_for(console, cx);
    let service = exec_service(cx);
    let ticket = service.run(
        &live,
        RunRequest {
            console: console.0,
            engine: profile.engine,
            units: units.clone(),
            fetch,
            log: state.log.clone(),
            prepared: Some(prepared),
            plan: Some(plan),
            policy,
            run_as_one,
            history,
            confirm: Some(port.clone() as Arc<dyn hacivat_exec::ConfirmPort>),
            context_generation: terms.context_generation,
        },
    );
    // T21: the port follows this run's questions and its losses, whatever order they come in.
    port.follow(
        ticket.run,
        ticket.choices.clone(),
        ticket.status.clone(),
        masked,
    );
    let questions = follow_questions(
        console,
        ticket.run,
        ticket.choices.clone(),
        live.clone(),
        port,
        cx,
    );
    let session = live;
    let keys: Vec<MarkKey> = (0..units.len())
        .map(|i| marks::mark_key(ticket.run, i))
        .collect();
    editor.update(cx, |e, cx| {
        e.add_exec_marks(
            version,
            keys.iter()
                .copied()
                .zip(units.iter().map(|u| u.doc_range.clone()))
                .collect(),
            cx,
        );
    });
    // The units the console's result tabs show now (Re-run looks them up; Fix round 1, C2).
    let shown: Vec<UnitId> = state
        .results
        .as_ref()
        .map(|r| r.read(cx).tabs().iter().map(|t| t.unit()).collect())
        .unwrap_or_default();
    let status_rx = ticket.status.clone();
    let status = marks::follow_status(console, ticket.status, keys.clone(), cx);
    let results = marks::follow_results(console, ticket.results, cx);
    update_console_exec(cx, console, |e| {
        // Fix round 1 (C2): by the unit's own ID, so a result tab's Re-run finds its unit.
        for (i, u) in units.iter().enumerate() {
            let id = UnitId {
                run: ticket.run,
                ordinal: u32::try_from(i).unwrap_or(u32::MAX),
            };
            e.tab_units.insert(id, (u.clone(), keys[i]));
        }
        e.tab_units
            .retain(|id, _| id.run == ticket.run || shown.contains(id));
        e.run = Some(ActiveRun {
            run: ticket.run,
            seq: ticket.seq,
            session,
            units,
            keys,
            ended: false,
            cancel_asked: false,
            status: status_rx,
            policy,
            _tasks: vec![status, results, questions],
        });
    });
    // ## m3b-t24-exec-ui: the prepared unit behind each result tab, by the unit's own ID, kept
    // for as long as `tab_units` keeps the tab's unit.
    let live: Vec<UnitId> = console_exec(cx, console)
        .map(|e| e.tab_units.keys().copied().collect())
        .unwrap_or_default();
    let tabs = cx
        .default_global::<RunPrep>()
        .prepared
        .entry(console)
        .or_default();
    for (i, p) in kept.into_iter().enumerate() {
        tabs.insert(
            UnitId {
                run: ticket.run,
                ordinal: u32::try_from(i).unwrap_or(u32::MAX),
            },
            p,
        );
    }
    tabs.retain(|id, _| live.contains(id));
}

/// The prepared unit (final text, binds, masked text) behind console `console`'s result tab of
/// unit `unit`, while the tab keeps it (Task 25's Count rows: `count_prepared` and
/// `CountRequest::binds`). `None` for a unit M3b did not prepare.
pub fn prepared_unit(console: ConsoleId, unit: UnitId, cx: &App) -> Option<PreparedUnit> {
    cx.try_global::<RunPrep>()
        .and_then(|p| p.prepared.get(&console))
        .and_then(|m| m.get(&unit).cloned())
}

/// A run ended: probe the SQL context again when one of its units may have changed it.
pub(crate) fn on_run_ended(console: ConsoleId, cx: &mut App) {
    let reprobe = console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .is_some_and(|r| r.units.iter().any(|u| needs_reprobe(&u.text)));
    // ## m3b-t24-exec-ui: or when the classifier said a unit may change the session's context
    // (a search path set by a function, a role, code it cannot see): Run's next classification
    // must not read the old one.
    let changed = cx
        .try_global::<RunPrep>()
        .is_some_and(|p| p.reread.contains(&console));
    if changed {
        cx.default_global::<RunPrep>().reread.remove(&console);
    }
    if reprobe || changed {
        probe_context(console, cx);
    }
}

// ---- the results tile's requests ---------------------------------------------------------

/// Handles what the results tile asks of the run UI (Task 30 emits `ResultsEvent`): Go to
/// source selects the range in the editor; Re-run runs a closed result's unit again, only when
/// the editor still holds exactly the text that ran (what is sent is what the user sees).
pub fn handle_results_event(event: &ResultsEvent, window: &mut Window, cx: &mut App) {
    match event {
        ResultsEvent::GoToSource { console, range } => {
            let Some(editor) = console_exec(cx, *console).and_then(|e| e.editor.upgrade()) else {
                return;
            };
            let range = range.clone();
            editor.update(cx, |e, cx| e.select_range(range, cx));
            let handle =
                hacivat_ui_core::gpui_surface::Focusable::focus_handle(editor.read(cx), cx);
            window.focus(&handle, cx);
        }
        ResultsEvent::Rerun {
            console,
            unit_ordinal,
        } => rerun(*console, *unit_ordinal, cx),
    }
}

fn rerun(console: ConsoleId, ordinal: u32, cx: &mut App) {
    let Some(exec) = console_exec(cx, console) else {
        return;
    };
    if exec.run.as_ref().is_some_and(|r| !r.ended) {
        return;
    }
    // Fix round 1 (C2): the unit behind *this* tab (its own `ResultEntry::unit`), never the
    // last run's unit at the same ordinal (which may have made no tab: a DELETE after the
    // SELECT whose rows the tab shows).
    let tab_unit = Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .and_then(|r| {
            let r = r.read(cx);
            // ## m3b-t25-results (named exception): Re-run is asked from the active tab (its
            // strip, `results.rerun`); a pinned tab of an older run may share its ordinal, and
            // a further result set its unit, so the active tab answers first.
            r.active_tab()
                .filter(|t| t.key().unit_ordinal == ordinal)
                .or_else(|| {
                    r.tabs()
                        .iter()
                        .find(|t| t.key().unit_ordinal == ordinal && t.key().result_ordinal == 0)
                })
                .map(|t| t.unit())
        });
    let Some((unit, key)) = tab_unit.and_then(|id| exec.tab_units.get(&id).cloned()) else {
        say(console, "hv-exec-rerun-edited", cx);
        return;
    };
    let Some(editor) = exec.editor.upgrade() else {
        return;
    };
    let view = editor.read(cx);
    let snap = view.snapshot();
    let marks = view.marks();
    // The mark follows the statement through edits; an edit inside it makes it stale.
    let range = marks.range(key).filter(|_| !marks.is_stale(key));
    let same = range.as_ref().is_some_and(|r| {
        r.end <= snap.buffer.len_bytes() && snap.buffer.slice(r.clone()) == *unit.text
    });
    match range {
        Some(range) if same => {
            let unit = ExecUnit {
                doc_range: range,
                ..unit
            };
            start_run(console, snap.version, vec![unit], cx);
        }
        _ => say(console, "hv-exec-rerun-edited", cx),
    }
}

// =================================================================================================
// ## m3b-t24-exec-ui: the M3b run pipeline (HV-EXEC-01/-03/-04 app side; P-M3b-2, R-M3-52,
// R-M3-60, R-M3-84; ADR 0006 `m3b-t24`)
//
// units (as previewed) → `prepare_units` (the parameter prompt on `NeedsValues`; Cancel sends
// nothing) → `classify_units(final_text)` on the classifier pool, off the UI thread, in the
// session's real context (`run_ctx`: known modes, search path, version, the profile's allowlist)
// → `gate` with `ConnSafety` from the live profile and session (production read-only until Task
// 26; since then its production rows) → a denial logs, marks the gutter and says why, nothing
// sent → the confirmations in unit
// order (R-M3-84) → one `ExecService::run` with the answered plan. Classification sees exactly
// the bytes sent (the gate denies a stale report), and Run sends exactly the previewed ranges
// (with parameters, their rewrite).
// =================================================================================================

/// Fluent IDs this pipeline writes (`hacivat-core.ftl`, block `m3b-t24-exec-ui`).
const PARAMS_CANCELLED: &str = "exec-params-cancelled";
const PARAMS_MODES: &str = "exec-param-modes-unknown";
const AS_ONE_REFUSED: &str = "exec-gate-run-as-one-refused";
const CONFIRM_CANCELLED: &str = "exec-confirm-cancelled";
const CONFIRM_SKIPPED: &str = "exec-confirm-skipped";
const CONFIRM_SUPPRESSED: &str = "exec-confirm-suppressed";
// ## fix round 1
/// I4: a unit not sent because another unit of the run was refused.
const RUN_REFUSED: &str = "exec-gate-run-refused";
/// I1: the session reads text in other modes than the console split it in.
const CONTEXT_CHANGED: &str = "exec-run-context-changed";
// ## m3b-t26-prod-unlock (`hacivat-core.ftl`, block `m3b-t26-prod-unlock`)
/// Carry MUST 2: the context read timed out on a production or read-only session.
const CONTEXT_UNCONFIRMED: &str = "exec-run-context-unconfirmed";
/// M4: the console's session changed between the gate and the send.
const CONNECTION_CHANGED: &str = "exec-run-connection-changed";
/// M1: a unit run as one carries no bind values.
const PARAMS_AS_ONE: &str = "exec-param-run-as-one";
/// M2: a unit run as one on a read-only session.
const AS_ONE_READ_ONLY: &str = "exec-gate-run-as-one-read-only";
/// The end-of-line words of a unit not sent (`hacivat-ui.ftl`): M3a's read-only denial, and
/// any other refusal.
const DENIED_RO: &str = "hv-editor-mark-denied";
const NOT_SENT: &str = "hv-editor-mark-not-sent";
/// How long Run waits for the session's context before it classifies without it (fail-closed:
/// unknown modes, and an unqualified builtin is then `Unknown`).
const CONTEXT_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
/// How long after a run that had a confirm-in-transaction question its loss listener still
/// waits for the session's report of a lost connection (fix round 1, I3).
const LOSS_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// One Run on its way through the pipeline.
struct Job {
    console: ConsoleId,
    version: u64,
    units: Vec<ExecUnit>,
    profile: Arc<Profile>,
    session: SessionHandle,
    /// The console's SQL context and whether its modes were read (`SqlContext::Known`).
    dialect: DialectCtx,
    modes_known: bool,
}

/// What the pipeline keeps per console (this module's global).
#[derive(Default)]
struct RunPrep {
    /// Consoles whose Run is being prepared (a prompt or a confirmation may be open): Run
    /// waits, as it does for a run in flight.
    busy: HashSet<ConsoleId>,
    /// "Don't ask again" (R-M3-55): `(console, GateItem::fingerprint)` for the app session.
    suppressed: SuppressSet,
    /// Each console's run policy (CHOOSE-06), until it closes.
    policies: HashMap<ConsoleId, RunPolicy>,
    /// The gutter marks of each console's last refusal (units not sent).
    refused: HashMap<ConsoleId, Vec<MarkKey>>,
    /// Consoles whose last run may have changed the session's context (read it again).
    reread: HashSet<ConsoleId>,
    /// Refusal marks take run IDs counting down from the top (real runs count up from 1).
    next_refusal: u64,
    /// The prepared unit behind each result tab, per console ([`prepared_unit`]).
    prepared: HashMap<ConsoleId, HashMap<UnitId, PreparedUnit>>,
}

impl Global for RunPrep {}

fn preparing(console: ConsoleId, cx: &App) -> bool {
    cx.try_global::<RunPrep>()
        .is_some_and(|p| p.busy.contains(&console))
}

fn set_preparing(console: ConsoleId, on: bool, cx: &mut App) {
    let prep = cx.default_global::<RunPrep>();
    if on {
        prep.busy.insert(console);
        // A new Run: the last one's refusals are no longer the latest.
        prep.refused.remove(&console);
    } else {
        prep.busy.remove(&console);
    }
}

// ---- the classifier pool, the parameter memory, the history -----------------------------

/// The classifier pool Run classifies on (R-M3-53): the one boot also installs into the
/// read-only guard.
struct Classifier(Arc<hacivat_exec::ClassifyPool>);

impl Global for Classifier {}

/// The app's classifier pool (a 2-thread pool when none was installed).
pub(crate) fn classifier(cx: &mut App) -> Arc<hacivat_exec::ClassifyPool> {
    if let Some(c) = cx.try_global::<Classifier>() {
        return c.0.clone();
    }
    let pool = hacivat_exec::ClassifyPool::new(2);
    cx.set_global(Classifier(pool.clone()));
    pool
}

/// Makes `pool` the pool Run classifies on (boot shares its read-only guard's pool; tests
/// install a recording one, `ClassifyPool::observed`). The run service is built again with it.
pub fn install_classifier(pool: Arc<hacivat_exec::ClassifyPool>, cx: &mut App) {
    cx.set_global(Classifier(pool));
    crate::exec_ui::reset_service(cx);
}

/// The parameter values remembered per `(profile, name)` for the app session (R-M3-79): never
/// persisted; named parameters only (the pipeline stores only `rememberable` answers).
#[derive(Clone, Default)]
pub(crate) struct ParamMemory(Arc<std::sync::Mutex<Remembered>>);

/// The remembered values, by profile and parameter name.
type Remembered = HashMap<(ProfileId, Arc<str>), BindValue>;

impl Global for ParamMemory {}

impl ParamValueStore for ParamMemory {
    fn get(&self, profile: &ProfileId, name: &str) -> Option<BindValue> {
        self.0
            .lock()
            .ok()
            .and_then(|m| m.get(&(*profile, Arc::from(name))).cloned())
    }

    fn put(&self, profile: &ProfileId, name: &str, v: BindValue) {
        if let Ok(mut m) = self.0.lock() {
            m.insert((*profile, Arc::from(name)), v);
        }
    }
}

/// Where the runs' history records go: a channel the service writes from the start, forwarded
/// to the store once it has opened ([`open_history`]).
struct HistoryLink {
    tx: tokio::sync::mpsc::Sender<Arc<hacivat_exec::HistoryRecord>>,
    rx: Option<tokio::sync::mpsc::Receiver<Arc<hacivat_exec::HistoryRecord>>>,
    store: Option<Arc<hacivat_exec::HistoryStore>>,
}

impl Global for HistoryLink {}

fn history_link(cx: &mut App) -> &mut HistoryLink {
    if !cx.has_global::<HistoryLink>() {
        let (tx, rx) = tokio::sync::mpsc::channel(hacivat_exec::SINK_CAPACITY);
        cx.set_global(HistoryLink {
            tx,
            rx: Some(rx),
            store: None,
        });
    }
    cx.global_mut::<HistoryLink>()
}

/// The sink the run service records into (`ExecService::with_history_sink`).
pub(crate) fn history_sink(
    cx: &mut App,
) -> tokio::sync::mpsc::Sender<Arc<hacivat_exec::HistoryRecord>> {
    history_link(cx).tx.clone()
}

/// Opens the query history (D-061, R-M3-62; the T17 carries) on the core runtime, never
/// holding the window: the private `data/` directory is created first (`0700`), the store's
/// open (with its integrity check) runs on the blocking pool, and its records start flowing
/// once it answers. A store recovered from a corrupt file says so once. With `history.enabled`
/// off nothing is opened and every record is dropped.
pub fn open_history(cx: &mut App) {
    let Some(state) = cx.try_global::<crate::AppState>() else {
        return;
    };
    if !state.settings.history.enabled {
        return;
    }
    let path = state.paths.history_db();
    let Some(rt) = cx
        .try_global::<crate::connections::ConnectionsModel>()
        .map(|m| m.runtime())
    else {
        return;
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    rt.spawn(async move {
        let dir_ok = path
            .parent()
            .is_some_and(|dir| hacivat_settings::fsio::create_dir_all_private(dir).is_ok());
        // Fix round 1 (M9): a history that can't open says so in the app's log (the run goes
        // on; nothing is recorded).
        let opened = if dir_ok {
            match hacivat_exec::HistoryStore::open(path, hacivat_exec::HistoryConfig::default())
                .await
            {
                Ok(store) => Some(store),
                Err(e) => {
                    tracing::warn!(error = ?e, "the query history could not be opened");
                    None
                }
            }
        } else {
            tracing::warn!("the query history's data directory could not be created");
            None
        };
        let _ = tx.send(opened);
    });
    cx.spawn(async move |cx| {
        let Ok(Some(store)) = rx.await else {
            return;
        };
        cx.update(|cx| {
            if store.recovered_from().is_some() {
                ToastQueue::push(
                    cx,
                    Toast::new(
                        ToastKind::Info,
                        crate::l10n_global::text_fn_id("hv-exec-history-recovered"),
                    ),
                );
            }
            // ## m3b-t27-history (named exception): the history dialog reads the same store.
            crate::history::set_store(Some(store.clone()), cx);
            let link = history_link(cx);
            link.store = Some(store.clone());
            if let Some(mut records) = link.rx.take() {
                let sink = store.sink();
                rt.spawn(async move {
                    while let Some(r) = records.recv().await {
                        // The store's own sink drops what it has no room for (R-M3-63).
                        let _ = sink.try_send(r);
                    }
                });
            }
        });
    })
    .detach();
}

/// Where console `console`'s run records itself (R-M3-62): nowhere when the profile records no
/// history (the store drops its records too: defence in depth) or history is off. M3b has no
/// private consoles.
fn history_meta(
    console: ConsoleId,
    profile: &Profile,
    cx: &mut App,
) -> Option<hacivat_exec::HistoryMeta> {
    let enabled = cx
        .try_global::<crate::AppState>()
        .is_none_or(|s| s.settings.history.enabled);
    if let Some(store) = cx.try_global::<HistoryLink>().and_then(|l| l.store.clone()) {
        store.set_do_not_record(profile.id, !profile.record_history);
    }
    (enabled && profile.record_history).then_some(hacivat_exec::HistoryMeta {
        profile: profile.id,
        console: console.0,
        private: false,
        origin: hacivat_exec::HistoryOrigin::Console,
    })
}

// ---- the console's run policy (CHOOSE-06) -------------------------------------------------

/// Console `console`'s run policy ("Customize…"; M3a's by default).
pub fn policy_of(console: ConsoleId, cx: &App) -> RunPolicy {
    cx.try_global::<RunPrep>()
        .and_then(|p| p.policies.get(&console).copied())
        .unwrap_or_default()
}

/// Sets console `console`'s run policy for as long as it is open; its band follows at once.
pub fn set_policy(console: ConsoleId, policy: RunPolicy, cx: &mut App) {
    cx.default_global::<RunPrep>()
        .policies
        .insert(console, policy);
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    if let Some(service) = Consoles::get(cx, console).and_then(|s| s.service.clone()) {
        let doc = editor.read(cx).doc();
        service.set_run_policy(doc, policy, cx);
    }
}

// ---- the session's search path (Run's classification context) ---------------------------

/// **The one classification context of a run** (T18 amendment C2(b)/I1; the T24 I1 ruling, the
/// T28 M6 carry): the session's own context `sctx` (`SessionHandle::classifier_context`: its
/// modes, search path and version as it reads and refreshes them), read by T18's rules
/// (`hacivat_exec::run_ctx_in`), with `profile`'s function allowlist and the unit's `expect`.
/// Run uses it; tests build the same context through it.
pub fn classify_ctx_in(
    profile: &Profile,
    sctx: &hacivat_session::session::ClassifierContext,
    expect: hacivat_exec::Expect,
) -> hacivat_exec::ClassifyCtx {
    let allow: Arc<[Arc<str>]> = profile
        .read_only_fn_allowlist
        .iter()
        .map(|s| Arc::from(s.as_str()))
        .collect();
    hacivat_exec::run_ctx_in(profile.engine, allow, sctx, expect)
}

/// The gate's view of console `console`'s connection now: the **live** profile's environment
/// (a profile retagged to production gets its confirmations from the next Run, R-M3-78) and the
/// session's read-only flag, transaction state and mode.
/// ## m3b-t26-prod-unlock: production is no longer gated as read-only (R-M3-6 REVISED ends):
/// every unit that is not a read meets the gate's production rows (R-M3-55).
fn conn_safety(profile: &Profile, session: &SessionHandle, cx: &App) -> ConnSafety {
    let snap = session.state().borrow().clone();
    ConnSafety {
        engine: profile.engine,
        env: profile.environment,
        read_only: snap.read_only,
        tx_status: snap.tx.status,
        tx_owner: snap.tx.owner,
        tx_mode: snap.tx.mode,
        confirm_unsafe_where: cx
            .try_global::<crate::AppState>()
            .is_none_or(|s| s.settings.exec.confirm_unsafe_where),
    }
}

// ---- parameters ---------------------------------------------------------------------------

/// What preparing gave, under every reading of unknown modes.
enum Prep {
    Same(Prepared),
    /// The modes are unknown and the units' parameters read differently under them.
    ModesDiffer,
}

/// `prepare_units_with` under `base`, and, with unknown modes, under every other reading
/// (`context::variants`): they must all agree, else nothing is sent (fail-closed: a `:a`
/// inside a string one mode closes early would bind under one reading only).
fn prepare_alike(
    units: &[ExecUnit],
    base: DialectCtx,
    modes_known: bool,
    store: &ParamMemory,
    profile: &ProfileId,
    answers: &ParamAnswers,
) -> Prep {
    let settings = ParamSettings::default();
    let first =
        hacivat_exec::prepare_units_with(units.to_vec(), &base, &settings, store, profile, answers);
    if !modes_known {
        for v in context::variants(&base) {
            let other = hacivat_exec::prepare_units_with(
                units.to_vec(),
                &v,
                &settings,
                store,
                profile,
                answers,
            );
            if other != first {
                return Prep::ModesDiffer;
            }
        }
    }
    Prep::Same(first)
}

/// Whether a unit of `units` holds PostgreSQL's `?` operator where a parameter would be
/// expected (`ParamDiag::QuestionMarkOff`): the prompt says so.
fn pg_question_marks(units: &[ExecUnit], base: &DialectCtx) -> bool {
    let settings = ParamSettings::default();
    units.iter().any(|u| {
        sql_split::scan_params(&u.text, base, &sql_split::StmtHead::default(), &settings)
            .diags
            .iter()
            .any(|(_, d)| *d == sql_split::ParamDiag::QuestionMarkOff)
    })
}

/// What the parameter prompt of console `console` shows beyond its rows.
/// `sql`: the units' text with its secrets masked, computed off the UI thread (it lexes).
fn prompt_context(
    job: &Job,
    sql: Arc<str>,
    notices: Vec<params_prompt::PromptNotice>,
    cx: &App,
) -> params_prompt::PromptContext {
    let locale = cx
        .try_global::<L10nGlobal>()
        .map(|g| g.l10n.snapshot().format.format_locale.clone())
        .unwrap_or_default();
    params_prompt::PromptContext {
        connection: Arc::from(job.profile.name.as_str()),
        env: job.profile.environment,
        engine: job.profile.engine,
        sql,
        notices,
        grouping_dot: params_prompt::groups_with_dot(&locale),
    }
}

// ---- refusals: log, gutter, say ---------------------------------------------------------------

/// `args` as message arguments (a statement number raw, names as data).
fn arg_values(args: &[(&'static str, LogArg)]) -> Vec<(&'static str, hacivat_l10n::ArgValue)> {
    args.iter()
        .map(|(k, v)| {
            let v = match v {
                LogArg::U64(n) => hacivat_l10n::ArgValue::Count(*n),
                LogArg::Str(s) => hacivat_l10n::ArgValue::Text(s.to_string()),
                LogArg::DurationMs(n) | LogArg::SpanMs(n) => hacivat_l10n::ArgValue::Count(*n),
            };
            (*k, v)
        })
        .collect()
}

/// A gate reason's arguments as log arguments.
fn reason_args(r: &GateReason) -> Vec<(&'static str, LogArg)> {
    r.args
        .iter()
        .map(|(k, v)| (*k, LogArg::Str(v.clone())))
        .collect()
}

/// Logs a line of the run UI on console `console` (`warn`: a refusal).
fn log_line(
    console: ConsoleId,
    id: &'static str,
    args: Vec<(&'static str, LogArg)>,
    warn: bool,
    at: Option<std::ops::Range<usize>>,
    cx: &mut App,
) {
    let Some(state) = Consoles::get(cx, console) else {
        return;
    };
    state.log.append(LogMsg::RunUi { id, args, warn }, at, None);
    if let Some(results) = state.results.clone() {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

/// Unit `range` of console `console` was not sent for `id` (`args`): the log says why, the
/// gutter marks it Failed with the same words, and the user is told (toast and announcement).
fn refuse(
    console: ConsoleId,
    version: u64,
    ranges: Vec<std::ops::Range<usize>>,
    (id, args): (&'static str, Vec<(&'static str, LogArg)>),
    short: &'static str,
    cx: &mut App,
) {
    for r in &ranges {
        log_line(console, id, args.clone(), true, Some(r.clone()), cx);
    }
    let values = arg_values(&args);
    let label: hacivat_ui::text::TextFn = {
        let values = values.clone();
        std::rc::Rc::new(move |cx: &App| marks::message(cx, id, values.clone()))
    };
    if let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) {
        let prep = cx.default_global::<RunPrep>();
        let keys: Vec<MarkKey> = ranges
            .iter()
            .enumerate()
            .map(|(i, _)| {
                prep.next_refusal = prep.next_refusal.wrapping_add(1);
                marks::mark_key(RunId(u64::MAX - prep.next_refusal), i)
            })
            .collect();
        let refused = prep.refused.entry(console).or_default();
        let first = refused.is_empty();
        refused.extend(keys.iter().copied());
        let eol: hacivat_ui::text::TextFn =
            std::rc::Rc::new(move |cx: &App| marks::message(cx, short, Vec::new()));
        editor.update(cx, |e, cx| {
            e.add_exec_marks(
                version,
                keys.iter().copied().zip(ranges.iter().cloned()).collect(),
                cx,
            );
            for key in &keys {
                e.set_exec_state(
                    *key,
                    ExecMarkState::Failed { sqlstate: None },
                    Some(label.clone()),
                    Some(eol.clone()),
                    cx,
                );
            }
            let text = label(cx);
            e.announce(text, cx);
        });
        // One toast per Run (the log and the gutter name each unit).
        if first {
            ToastQueue::push(cx, Toast::new(ToastKind::Info, label));
        }
    }
}

/// What the gutter shows for console `console`'s latest Run when it is no run of the session's:
/// nothing yet while the Run is being prepared (`Some(empty)`), the refused units' marks when it
/// was refused; `None` when the latest Run was sent (`exec_ui::last_run_marks` reads the run).
pub(crate) fn latest_marks_override(
    console: ConsoleId,
    cx: &App,
) -> Option<Vec<Option<ExecMarkState>>> {
    if preparing(console, cx) {
        return Some(Vec::new());
    }
    cx.try_global::<RunPrep>()
        .is_some_and(|p| p.refused.contains_key(&console))
        .then(|| last_refused_marks(console, cx))
}

/// The gutter marks of console `console`'s last refusal (tests).
pub fn last_refused_marks(console: ConsoleId, cx: &App) -> Vec<Option<ExecMarkState>> {
    let Some(keys) = cx
        .try_global::<RunPrep>()
        .and_then(|p| p.refused.get(&console).cloned())
    else {
        return Vec::new();
    };
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return Vec::new();
    };
    let marks = editor.read(cx).marks();
    keys.iter().map(|k| marks.state(*k).cloned()).collect()
}

/// A statement number as the log writes it (raw ASCII digits).
fn number(unit: u32) -> LogArg {
    LogArg::Str(Arc::from(unit.saturating_add(1).to_string()))
}

// ---- the pipeline -------------------------------------------------------------------------

/// The pipeline (the section docs), on a foreground task that awaits only channels: the
/// preparation and the gate run on the background executor, the classification on the pool.
/// Every early return sends nothing and has said why.
#[allow(clippy::too_many_lines)]
fn pipeline(job: Job, cx: &mut App) -> hacivat_ui_core::gpui_surface::Task<()> {
    cx.spawn(async move |cx| {
    let console = job.console;
    let store = cx.update(|cx| cx.default_global::<ParamMemory>().clone());
    let profile_id = job.profile.id;
    let ranges = |units: &[ExecUnit]| -> Vec<std::ops::Range<usize>> {
        units.iter().map(|u| u.doc_range.clone()).collect()
    };

    // 0. The session's own context (the T24 I1 ruling): asked of the session, so it is read
    //    after every earlier request on that session (another console's too, when they share
    //    it), and read again there when a request may have changed it. A context the session
    //    can't give in time is unknown (fail-closed), never a stale one. One context per run:
    //    the parameters are scanned and the units classified in it.
    let (sctx, _) =
        context_within(&job.session, cx.background_executor().timer(CONTEXT_WAIT)).await;
    let (session_dialect, session_known) =
        hacivat_exec::classify_pool::modes_of(job.profile.engine, &sctx);
    // The units were split in the console's context: when the session now reads text in other
    // modes they may not be what it would split, so nothing is sent and the console reads its
    // context again.
    if job.modes_known && session_known && session_dialect.modes != job.dialect.modes {
        cx.update(|cx: &mut App| {
            refuse(
                console,
                job.version,
                ranges(&job.units),
                (CONTEXT_CHANGED, Vec::new()),
                NOT_SENT,
                cx,
            );
            probe_context(console, cx);
        });
        return;
    }
    let (dialect, modes_known) = if session_known {
        (session_dialect, true)
    } else {
        (job.dialect, false)
    };

    // 1. Parameters: values asked for once per name, fresh answers each run (T15).
    // Everything that lexes runs here, off the UI thread: the preparation, PostgreSQL's `?`
    // scan and the prompt's masked text.
    let (prepared, qmarks, shown_sql) = {
        let (units, base, known, store) = (job.units.clone(), dialect, modes_known, store.clone());
        cx.background_executor()
            .spawn(async move {
                let prep =
                    prepare_alike(&units, base, known, &store, &profile_id, &ParamAnswers::default());
                let qmarks = matches!(
                    base.id,
                    sql_dialect::DialectId::Postgres | sql_dialect::DialectId::PgWire(_)
                ) && pg_question_marks(&units, &base);
                let sql: Vec<&str> = units.iter().map(|u| &*u.text).collect();
                let shown: Arc<str> =
                    Arc::from(hacivat_exec::mask_secrets(&sql.join(";\n"), &base).as_ref());
                (prep, qmarks, shown)
            })
            .await
    };
    let prepared = match prepared {
        Prep::ModesDiffer => {
            cx.update(|cx| refuse(console, job.version, ranges(&job.units), (PARAMS_MODES, Vec::new()), NOT_SENT, cx));
            return;
        }
        Prep::Same(Prepared::Ready(p)) => p,
        // Fix round 1 (M1): a unit run as one is sent in one simple-query message, which
        // carries no bind values: refused before any prompt, saying so.
        Prep::Same(
            Prepared::NeedsValues(_)
            | Prepared::Refused {
                diag: sql_split::ParamDiag::Unbindable,
                ..
            },
        ) if job.units.len() == 1
                && job.units[0].kind == sql_split::exec_range::UnitKind::Verbatim =>
        {
            cx.update(|cx: &mut App| {
                refuse(console, job.version, ranges(&job.units), (PARAMS_AS_ONE, Vec::new()), NOT_SENT, cx)
            });
            return;
        }
        Prep::Same(Prepared::Refused { unit, at, diag }) => {
            let id = hacivat_exec::param_diag_message_id(diag);
            cx.update(|cx| refuse(console, job.version, vec![at.clone()], (id, Vec::new()), NOT_SENT, cx));
            // A template variable is shown in the prompt too (NOTES surface 8): it can only be
            // cancelled.
            if diag == sql_split::ParamDiag::TemplateVariable {
                let var = job
                    .units
                    .get(unit as usize)
                    .and_then(|u| {
                        let s = at.start.checked_sub(u.doc_range.start)?;
                        let e = at.end.checked_sub(u.doc_range.start)?;
                        u.text.get(s..e).map(Arc::from)
                    })
                    .unwrap_or_else(|| Arc::from(""));
                let notices = vec![params_prompt::PromptNotice::Template(var)];
                let rx = cx.update(|cx: &mut App| {
                    let ctx = prompt_context(&job, shown_sql.clone(), notices, cx);
                    params_prompt::open(Vec::new(), ctx, cx)
                });
                let _ = rx.await;
            }
            return;
        }
        Prep::Same(Prepared::NeedsValues(prompts)) => {
            let mut notices = Vec::new();
            if qmarks {
                notices.push(params_prompt::PromptNotice::PgQuestionMark);
            }
            let rx = cx.update(|cx: &mut App| {
                let ctx = prompt_context(&job, shown_sql.clone(), notices, cx);
                params_prompt::open(prompts, ctx, cx)
            });
            let Some(values) = rx.await.ok().flatten() else {
                cx.update(|cx: &mut App| {
                    log_line(console, PARAMS_CANCELLED, Vec::new(), false, None, cx);
                    say(console, PARAMS_CANCELLED, cx);
                });
                return;
            };
            // A fresh `ParamAnswers` for exactly the units prompted (T15 carry); only named
            // parameters are remembered (ordinal and anonymous ones never are).
            let mut answers = ParamAnswers::default();
            for (p, v) in &values {
                answers.insert(p, v.clone());
                if p.rememberable() {
                    store.put(&profile_id, &p.name, v.clone());
                }
            }
            let again = {
                let (units, base, known, store) = (job.units.clone(), dialect, modes_known, store.clone());
                cx.background_executor()
                    .spawn(async move { prepare_alike(&units, base, known, &store, &profile_id, &answers) })
                    .await
            };
            match again {
                Prep::Same(Prepared::Ready(p)) => p,
                Prep::Same(Prepared::Refused { at, diag, .. }) => {
                    let id = hacivat_exec::param_diag_message_id(diag);
                    cx.update(|cx| refuse(console, job.version, vec![at], (id, Vec::new()), NOT_SENT, cx));
                    return;
                }
                // Answers that do not cover the prompts: never sent unbound.
                Prep::Same(Prepared::NeedsValues(_)) | Prep::ModesDiffer => {
                    cx.update(|cx: &mut App| {
                        refuse(console, job.version, ranges(&job.units), ("exec-param-unbindable", Vec::new()), NOT_SENT, cx)
                    });
                    return;
                }
            }
        }
    };

    // 2. Classification: exactly the bytes sent (P-M3b-2), on the pool, in the session's real
    //    context (T18 amendment C2(b)/I1).
    let run_as_one = prepared.len() == 1
        && prepared[0].unit.kind == sql_split::exec_range::UnitKind::Verbatim;
    let expect = prepared
        .first()
        .map_or(hacivat_exec::Expect::SingleStatement, |p| {
            hacivat_exec::expect_for(&p.unit)
        });
    let ctx = classify_ctx_in(&job.profile, &sctx, expect);
    let texts: Vec<Arc<str>> = prepared.iter().map(|p| p.final_text.clone()).collect();
    let pool = cx.update(classifier);
    let reports: Vec<hacivat_exec::SafetyReport> = match pool.classify_units(texts, ctx).await { Ok(r) => r, Err(never) => match never {} };

    // 3. The gate, with the connection as it is now.
    let (conn, suppressed) = cx.update(|cx: &mut App| {
        (
            // Fix round 1 (M5): the profile as it is now.
            conn_safety(
                &profile_of(cx, console).unwrap_or_else(|| job.profile.clone()),
                &job.session,
                cx,
            ),
            cx.default_global::<RunPrep>().suppressed.clone(),
        )
    });
    // ## m3b-t26-prod-unlock (fix round 1, B-M4/A-M4): what the plan was gated under; the
    // dialogs show it, and the send re-reads it (step 5).
    let gated = (conn.env, conn.read_only);
    let (prepared, mut plan, changes, switches) = {
        let c = console.0;
        cx.background_executor()
            .spawn(async move {
                let mut plan = gate(&prepared, &reports, &conn, &suppressed, c);
                let mut prepared = prepared;
                // R-M3-23: a cursor only for a unit the classifier found a read.
                apply_read_verdicts(&mut prepared, &reports);
                let changes = prepared
                    .iter()
                    .zip(&reports)
                    .any(|(p, r)| hacivat_exec::changes_context(&p.final_text, r));
                // Fix round 2 (R1-C1): the units whose literal context switch the later units
                // were classified under; only those before the last unit matter.
                let texts: Vec<&str> = prepared.iter().map(|p| &*p.final_text).collect();
                let switches: Vec<u32> = hacivat_exec::carried_switches(&texts, &reports)
                    .into_iter()
                    .filter(|&i| i + 1 < prepared.len())
                    .filter_map(|i| u32::try_from(i).ok())
                    .collect();
                // T19: the session refuses a unit run as one that would need its own
                // transaction or the D-062 wrap; the gate never sends one.
                // Fix round 1 (M2): a unit run as one never goes to a read-only session (its
                // guard checks one statement at a time), the chooser's reason.
                if run_as_one && conn.read_only {
                    for item in &mut plan.items {
                        item.decision = GateDecision::Deny(GateReason::of(AS_ONE_READ_ONLY));
                    }
                }
                if run_as_one {
                    for item in &mut plan.items {
                        if matches!(
                            item.verdict,
                            UnitVerdict::TxThenConfirm | UnitVerdict::ExplainAnalyzeRollback
                        ) && !matches!(item.decision, GateDecision::Deny(_))
                        {
                            item.decision = GateDecision::Deny(GateReason::of(AS_ONE_REFUSED));
                        }
                    }
                }
                (prepared, plan, changes, switches)
            })
            .await
    };
    if changes {
        cx.update(|cx: &mut App| {
            cx.default_global::<RunPrep>().reread.insert(console);
        });
    }
    if plan.first_denied().is_some() {
        // Nothing of the run is sent: each denied unit says why, in the log and the gutter.
        cx.update(|cx: &mut App| {
            for item in &plan.items {
                if let GateDecision::Deny(reason) = &item.decision
                    && let Some(p) = prepared.get(item.unit as usize)
                {
                    refuse(
                        console,
                        job.version,
                        vec![p.unit.doc_range.clone()],
                        (reason.fluent_id, reason_args(&reason.clone())),
                        if conn.read_only { DENIED_RO } else { NOT_SENT },
                        cx,
                    );
                }
            }
            // Fix round 1 (I4): every other unit of the run was not sent either, and says
            // which statement refused the run.
            let first = plan.first_denied().map_or(0, |i| i.unit);
            let others: Vec<std::ops::Range<usize>> = plan
                .items
                .iter()
                .filter(|i| !matches!(i.decision, GateDecision::Deny(_)))
                .filter_map(|i| prepared.get(i.unit as usize).map(|p| p.unit.doc_range.clone()))
                .collect();
            for at in others {
                refuse(
                    console,
                    job.version,
                    vec![at],
                    (RUN_REFUSED, vec![("n", number(first))]),
                    NOT_SENT,
                    cx,
                );
            }
        });
        return;
    }
    // "Don't ask again" ran a unit that would have asked: the log says so (T15 carry).
    cx.update(|cx: &mut App| {
        let title = crate::exec_ui::confirm::console_facts(cx, console)
            .map_or_else(|| Arc::from(job.profile.name.as_str()), |f| f.title);
        let prep = cx.default_global::<RunPrep>();
        let lines: Vec<std::ops::Range<usize>> = plan
            .items
            .iter()
            .filter(|i| {
                i.decision == GateDecision::Run && prep.suppressed.contains(console.0, i.fingerprint)
            })
            .filter_map(|i| prepared.get(i.unit as usize).map(|p| p.unit.doc_range.clone()))
            .collect();
        for at in lines {
            log_line(
                console,
                CONFIRM_SUPPRESSED,
                vec![("console", LogArg::Str(title.clone()))],
                false,
                Some(at),
                cx,
            );
        }
    });

    // 4. The confirmations, in unit order, before anything is sent (R-M3-84).
    if plan.needs_prompt() {
        let asked = cx.update(|cx: &mut App| {
            crate::console::tx_badge::set_choice_pending(console, true, cx);
            let facts = crate::exec_ui::confirm::console_facts(cx, console);
            // ## m3b-t26-prod-unlock (fix round 1, A-M4): the live profile's tag the plan was
            // gated under, never the tag the job was started with.
            let production = gated.0 == Some(EnvTag::Production);
            let base = crate::exec_ui::confirm::ConfirmContext {
                env: facts.as_ref().and_then(|f| f.env),
                engine: facts.as_ref().and_then(|f| f.engine.clone()),
                console: facts.as_ref().map(|f| f.title.clone()),
                ..Default::default()
            };
            let name = facts
                .map_or_else(|| Arc::from(job.profile.name.as_str()), |f| f.name);
            let masked = prepared.iter().map(|p| p.masked.clone()).collect();
            crate::exec_ui::confirm::ask_plan_at(RunId(0), plan.clone(), masked, production, name, base, cx)
        });
        let answered = asked.await;
        cx.update(|cx| crate::console::tx_badge::set_choice_pending(console, false, cx));
        match answered {
            Err(unit) => {
                cx.update(|cx: &mut App| {
                    let at = prepared.get(unit as usize).map(|p| p.unit.doc_range.clone());
                    log_line(console, CONFIRM_CANCELLED, vec![("n", number(unit))], false, at, cx);
                });
                return;
            }
            Ok(answers) => {
                plan = answers.plan;
                cx.update(|cx: &mut App| {
                    let prep = cx.default_global::<RunPrep>();
                    for key in answers.suppress {
                        prep.suppressed.insert(console.0, key);
                    }
                    for item in &plan.items {
                        if matches!(&item.decision, GateDecision::Deny(r) if r.fluent_id == hacivat_exec::SKIPPED_BY_USER) {
                            let at = prepared.get(item.unit as usize).map(|p| p.unit.doc_range.clone());
                            log_line(console, CONFIRM_SKIPPED, vec![("n", number(item.unit))], false, at, cx);
                        }
                    }
                });
            }
        }
    }
    // Fix round 2 (R1-C1): the later units were classified as if a carried context switch took
    // effect, so skipping one cancels the run: nothing is sent.
    if let Some(item) = plan.items.iter().find(|i| {
        switches.contains(&i.unit)
            && matches!(&i.decision, GateDecision::Deny(r) if r.fluent_id == hacivat_exec::SKIPPED_BY_USER)
    }) {
        let unit = item.unit;
        cx.update(|cx: &mut App| {
            let at = prepared.get(unit as usize).map(|p| p.unit.doc_range.clone());
            log_line(console, CONFIRM_CANCELLED, vec![("n", number(unit))], false, at, cx);
            say(console, CONFIRM_CANCELLED, cx);
        });
        return;
    }
    // Every unit skipped: nothing to send (the log said each).
    let Some(_) = plan
        .items
        .iter()
        .find(|i| !matches!(&i.decision, GateDecision::Deny(r) if r.fluent_id == hacivat_exec::SKIPPED_BY_USER))
    else {
        return;
    };

    // The context again, after the prompt and the confirmations (another console sharing the
    // session may have changed it meanwhile): a run is sent only in the context it was
    // prepared and classified in.
    let (now, generation) =
        context_within(&job.session, cx.background_executor().timer(CONTEXT_WAIT)).await;
    if now != sctx {
        cx.update(|cx: &mut App| {
            refuse(
                console,
                job.version,
                ranges(&job.units),
                (CONTEXT_CHANGED, Vec::new()),
                NOT_SENT,
                cx,
            );
            probe_context(console, cx);
        });
        return;
    }
    // ## m3b-t26-prod-unlock (T26 carry MUST 2, the T24 round-2 re-check): a read that timed
    // out (both reads did, or `now` would differ from `sctx`) gives no context generation, and
    // the session checks none. On a production (the live tag) or read-only session nothing is
    // sent then: the run could not be proved to run in the context it was classified in.
    let unconfirmed = generation.is_none()
        && cx.update(|cx: &mut App| {
            let live = profile_of(cx, console).unwrap_or_else(|| job.profile.clone());
            live.environment == Some(EnvTag::Production) || job.session.state().borrow().read_only
        });
    if unconfirmed {
        cx.update(|cx: &mut App| {
            refuse(
                console,
                job.version,
                ranges(&job.units),
                (CONTEXT_UNCONFIRMED, Vec::new()),
                NOT_SENT,
                cx,
            );
        });
        return;
    }

    // 5. One run.
    cx.update(|cx: &mut App| {
        // ## m3b-t26-prod-unlock (fix round 1, B-M4/A-M4): the live profile's tag and the
        // session's read-only flag are read again right before the send (the context generation
        // is checked by the session); a plan gated under another tag or flag (a retag while
        // the dialogs were open) sends nothing.
        let live = profile_of(cx, console).unwrap_or_else(|| job.profile.clone());
        if (live.environment, job.session.state().borrow().read_only) != gated {
            refuse(
                console,
                job.version,
                ranges(&job.units),
                (CONNECTION_CHANGED, Vec::new()),
                NOT_SENT,
                cx,
            );
            return;
        }
        submit(
            console,
            job.version,
            job.session.clone(),
            &job.profile,
            prepared,
            plan,
            RunTerms {
                run_as_one,
                // A failed switch must stop the run (fix round 2, R1-C1).
                stop_on_error: !switches.is_empty(),
                // The session refuses the run if its context moved since (fix round 2).
                context_generation: generation,
            },
            cx,
        );
    });
    })
}

/// How [`submit`] sends a run (fix round 2).
#[derive(Debug, Clone, Copy)]
struct RunTerms {
    /// One `Verbatim` unit sent as one (R-M3-60).
    run_as_one: bool,
    /// `ErrorPolicy::StopOnError` whatever the setting says: a unit before the last carries a
    /// context switch the later units were classified under (R1-C1), so the error bar's
    /// Ignore/Skip must not run them after it failed.
    stop_on_error: bool,
    /// The session's context generation the run was classified under.
    context_generation: Option<u64>,
}

// ---- the run's questions ------------------------------------------------------------------

/// The session's classification context (`SessionHandle::classifier_context`), or an unknown
/// one when `timer` ends first (fail-closed; fix round 1, I1).
/// With the context's generation (fix round 2; `None` when unknown).
async fn context_within(
    session: &SessionHandle,
    timer: impl std::future::Future<Output = ()>,
) -> (hacivat_session::session::ClassifierContext, Option<u64>) {
    let ask = session.run_context();
    futures::pin_mut!(ask);
    futures::pin_mut!(timer);
    match futures::future::select(ask, timer).await {
        futures::future::Either::Left((answer, _)) => answer,
        futures::future::Either::Right(_) => {
            (hacivat_session::session::ClassifierContext::default(), None)
        }
    }
}

/// What run `run`'s port is told of a lost connection (fix round 1, I3): every loss of a run
/// that had a confirm-in-transaction question reaches the port (`connection_lost`), whichever
/// of the loss and the question this side sees first (T21's port orders them), with the
/// statements before the confirmed one in the savepoint form (M10).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LossLink {
    /// A question was seen: `Some(statements before)`, the count when the form is the
    /// savepoint's.
    question: Option<Option<u32>>,
    lost: bool,
    told: bool,
}

impl LossLink {
    /// A confirm-in-transaction question; `statements`: the user's transaction's statements
    /// (the confirmed one included) in the savepoint form. `Some(n)` when the port must now
    /// be told of a loss seen earlier.
    pub(crate) fn question(&mut self, savepoint: bool, statements: u32) -> Option<Option<u32>> {
        let before = savepoint.then(|| statements.saturating_sub(1));
        self.question.get_or_insert(before);
        self.tell()
    }

    /// The session's connection was lost. `Some(n)` when the port must now be told.
    pub(crate) fn lost(&mut self) -> Option<Option<u32>> {
        self.lost = true;
        self.tell()
    }

    fn tell(&mut self) -> Option<Option<u32>> {
        let before = self.question?;
        if !self.lost || self.told {
            return None;
        }
        self.told = true;
        Some(before)
    }
}

/// Follows run `run`'s questions: an error-policy question goes to the error bar
/// (`error_bar::on_choice`, Task 23); while a confirm-in-transaction question waits, the
/// transaction badge waits too (`set_choice_pending`) and the unit's gutter mark says what is
/// awaited; a loss of a run that had such a question reaches the port in any order
/// ([`LossLink`]; its prompt then shows the server rolled the changes back).
fn follow_questions(
    console: ConsoleId,
    run: RunId,
    mut choices: watch::Receiver<Option<(u64, hacivat_session::session::RunEvent)>>,
    session: SessionHandle,
    port: Arc<crate::exec_ui::AppConfirmPort>,
    cx: &mut App,
) -> hacivat_ui_core::gpui_surface::Task<()> {
    use hacivat_session::session::RunEvent;
    let waiting = std::rc::Rc::new(std::cell::Cell::new(false));
    let link = std::rc::Rc::new(std::cell::Cell::new(LossLink::default()));
    // The loss listener outlives the run's questions: a loss the session reports after the run
    // ended (or after its question cleared) still reaches the port, for `LOSS_GRACE` after the
    // end of a run that had a question; a run without one stops listening at its end.
    let (ended, ended_rx) = futures::channel::oneshot::channel::<()>();
    {
        let (link, port) = (link.clone(), port.clone());
        let mut events = session.events();
        let timer = cx.background_executor().clone();
        cx.spawn(async move |_| {
            let linger = {
                let link = link.clone();
                async move {
                    let _ = ended_rx.await;
                    if link.get().question.is_some() {
                        timer.timer(LOSS_GRACE).await;
                    }
                }
            };
            futures::pin_mut!(linger);
            loop {
                let next = Box::pin(events.recv());
                let ev = match futures::future::select(next, linger.as_mut()).await {
                    futures::future::Either::Left((ev, _)) => ev,
                    futures::future::Either::Right(_) => break,
                };
                match ev {
                    Ok(SessionEvent::Lost(_)) => {
                        let mut l = link.get();
                        let tell = l.lost();
                        link.set(l);
                        if let Some(before) = tell {
                            port.connection_lost(run, before);
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .detach();
    }
    cx.spawn(async move |cx| {
        let _ended = ended;
        loop {
            let now = choices.borrow_and_update().clone();
            cx.update(|cx| match &now {
                Some((_, ev @ RunEvent::NeedsChoice { .. })) => {
                    crate::exec_ui::error_bar::on_choice(console, ev, cx);
                }
                Some((
                    _,
                    RunEvent::NeedsCommit {
                        unit, savepoint, ..
                    },
                )) => {
                    waiting.set(true);
                    let statements = session.state().borrow().tx.statements;
                    let mut l = link.get();
                    let tell = l.question(*savepoint, statements);
                    link.set(l);
                    if let Some(before) = tell {
                        port.connection_lost(run, before);
                    }
                    crate::console::tx_badge::set_choice_pending(console, true, cx);
                    mark_awaiting(console, *unit, *savepoint, cx);
                }
                _ => {
                    if waiting.replace(false) {
                        crate::console::tx_badge::set_choice_pending(console, false, cx);
                    }
                }
            });
            if choices.changed().await.is_err() {
                break;
            }
        }
        if waiting.replace(false) {
            cx.update(|cx| crate::console::tx_badge::set_choice_pending(console, false, cx));
        }
    })
}

/// Unit `unit`'s gutter mark while its confirm-in-transaction question waits (NOTES ruling 31):
/// the pending run marker with "waiting for Commit or Roll back" (or Keep or Undo).
fn mark_awaiting(console: ConsoleId, unit: UnitId, savepoint: bool, cx: &mut App) {
    let Some(exec) = console_exec(cx, console) else {
        return;
    };
    let Some(key) = exec
        .run
        .as_ref()
        .filter(|r| r.run == unit.run)
        .and_then(|r| r.keys.get(unit.ordinal as usize).copied())
    else {
        return;
    };
    let Some(editor) = exec.editor.upgrade() else {
        return;
    };
    let id = if savepoint {
        "hv-editor-mark-awaiting-sp"
    } else {
        "hv-editor-mark-awaiting"
    };
    let label: hacivat_ui::text::TextFn =
        std::rc::Rc::new(move |cx: &App| marks::message(cx, id, Vec::new()));
    editor.update(cx, |e, cx| {
        // ## m3c-t34-editor (named exception; T32 ruling 5): its own state, the amber stamp,
        // which the fold header ranks first and the gutter ▶ never covers.
        e.set_exec_state(
            key,
            ExecMarkState::Awaiting,
            Some(label.clone()),
            Some(label),
            cx,
        );
        e.set_exec_short(key, None, cx);
    });
}

// ---- Run as one (R-M3-60) -----------------------------------------------------------------

/// Whether two ranges overlap (an empty one at a point inside counts).
fn overlaps(a: std::ops::Range<usize>, b: &std::ops::Range<usize>) -> bool {
    a.start < b.end && b.start < a.end || (a.is_empty() && b.start <= a.start && a.start < b.end)
}

fn span(r: sql_split::exec_range::TextRange) -> std::ops::Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

/// The unit "Run as one" sends from Run's uncertain state at `at` (R-M3-60): the region the
/// uncertain boundaries sit in, sent whole as one unit (`Verbatim`: one simple-query message,
/// classified as a script). A single caret: the execution statements holding the boundaries,
/// from the first one's start to the last one's end (Run 3: the whole text); one non-empty
/// selection: the selection. Without its outer white space; `None` when that text would not
/// be what the document shows (a cut token), would reach a client command or `COPY` rows, or
/// a boundary comes from a client command or several carets.
fn run_as_one_unit(
    doc: &sql_split::DocSnapshot,
    at: &hacivat_ui_editor::ExecAt,
    action: ExecAction,
    boundaries: &[sql_split::AmbiguousBoundary],
) -> Option<ExecUnit> {
    use hacivat_ui_editor::ExecAt;
    use sql_split::AmbiguityReason;
    if boundaries.is_empty()
        || boundaries.iter().any(|b| {
            matches!(
                b.reason,
                AmbiguityReason::ClientCommand | AmbiguityReason::OverlappingUnits
            )
        })
    {
        return None;
    }
    let text = doc.text();
    let exec = doc.execution_split();
    let region: std::ops::Range<usize> = match at {
        ExecAt::Selections(s) if s.len() == 1 && !s[0].is_empty() => {
            let r = &s[0];
            r.start.min(r.end)..r.start.max(r.end)
        }
        ExecAt::Carets(c) if c.len() == 1 => match action {
            ExecAction::Execute3 => 0..text.len(),
            ExecAction::Execute1 | ExecAction::Execute2 => {
                let mut lo = usize::MAX;
                let mut hi = 0;
                for b in boundaries {
                    let holder = exec.statements().find(|s| {
                        s.strict.contains_range(b.at)
                            && (!b.at.is_empty() || s.strict.contains(b.at.start()))
                    })?;
                    lo = lo.min(usize::from(holder.strict.start()));
                    hi = hi.max(usize::from(holder.strict.end()));
                }
                lo..hi
            }
        },
        _ => return None,
    };
    let size =
        |n: usize| sql_split::exec_range::TextSize::from(u32::try_from(n).unwrap_or(u32::MAX));
    let region = span(
        doc.without_comment_parts(sql_split::exec_range::TextRange::new(
            size(region.start),
            size(region.end.min(text.len())),
        )),
    );
    let piece = text.get(region.clone())?;
    // Fix round 1 (M3): only SQL white space is trimmed (a no-break space is part of a name
    // to PostgreSQL, not white space).
    let sql_space = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c');
    let lead = piece.len() - piece.trim_start_matches(sql_space).len();
    let trimmed = piece.trim_matches(sql_space);
    if trimmed.is_empty() {
        return None;
    }
    let range = region.start + lead..region.start + lead + trimmed.len();
    let tr = sql_split::exec_range::TextRange::new(size(range.start), size(range.end));
    if !doc.selection_cuts(tr).is_empty()
        || exec.client.iter().any(|c| overlaps(span(c.range), &range))
        || exec
            .statements()
            .any(|s| s.copy_data.is_some_and(|d| overlaps(span(d.rows), &range)))
    {
        return None;
    }
    Some(ExecUnit {
        text: Arc::from(trimmed),
        doc_range: range,
        kind: sql_split::exec_range::UnitKind::Verbatim,
        suspendable: false,
        title: None,
    })
}

/// How many execution statements `range` holds (the "Run as one" row says it).
fn statements_in(doc: &sql_split::DocSnapshot, range: &std::ops::Range<usize>) -> usize {
    doc.execution_split()
        .statements()
        .filter(|s| overlaps(span(s.strict), range))
        .count()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::LossLink;

    /// Fix round 1 (I3): a loss of a run that had a question reaches the port once, whichever
    /// comes first; a run without a question never tells it.
    #[test]
    fn a_loss_reaches_the_port_in_either_order() {
        let mut first = LossLink::default();
        assert_eq!(first.question(true, 3), None, "no loss yet");
        assert_eq!(
            first.lost(),
            Some(Some(2)),
            "the two statements before the confirmed one"
        );
        assert_eq!(first.lost(), None, "told once");

        let mut later = LossLink::default();
        assert_eq!(later.lost(), None, "no question yet");
        assert_eq!(later.question(false, 0), Some(None), "own form: no count");
        assert_eq!(later.question(false, 0), None, "told once");

        let mut none = LossLink::default();
        assert_eq!(
            none.lost(),
            None,
            "a run without a question never tells the port"
        );
    }
}
