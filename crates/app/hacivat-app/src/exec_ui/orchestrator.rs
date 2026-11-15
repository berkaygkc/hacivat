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
use hacivat_results::FetchPolicy;
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
    let points = exec_points(at);
    let run =
        hacivat_exec::resolve_units_with(doc, &points, run_action(action), remembered, &NoOracle);
    let band = hacivat_sql_editor::preview_of(doc, at, action, true, remembered);
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
    let remembered = exec.remembered;
    let context = exec.context;
    let Some(profile) = profile_of(cx, console) else {
        return;
    };
    if live_session(cx, console).is_none() {
        connect_first(&profile, cx);
        return;
    }
    // Fix round 1 (Task 32 contract): a production-tagged console waiting for the user's open
    // transaction to end before it reconnects read-only sends nothing meanwhile; Commit and
    // Roll back stay available (tx.commit / tx.rollback do not ask this).
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
        let r = resolve_for_run(&doc, &at, action, remembered);
        if !unknown {
            return (doc, r);
        }
        // Unknown modes: every combination must resolve alike, else nothing is sent.
        let alike = context::variants(&ctx).into_iter().all(|v| {
            let other = analyse(v);
            same_runnable(&r, &resolve_for_run(&other, &at, action, remembered))
        });
        let r = if alike {
            r
        } else {
            Resolution::Uncertain(Vec::new())
        };
        (doc, r)
    });
    let version = req.snapshot.version;
    cx.spawn(async move |cx| {
        let (doc, resolution) = resolve.await;
        cx.update(|cx| {
            let current = console_exec(cx, console).is_some_and(|e| e.request == request);
            if current {
                dispatch(console, version, doc, resolution, cx);
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
        Resolution::NothingToRun => say(console, "hv-editor-nothing-to-run", cx),
        Resolution::Uncertain(boundaries) => {
            let mut reasons: Vec<&'static str> = boundaries
                .iter()
                .map(|b| uncertain_message_id(b.reason))
                .collect();
            reasons.dedup();
            chooser::open_uncertain(console, reasons, cx);
        }
        Resolution::UnsupportedClientCommand { at } => {
            let reason = uncertain_message_id(sql_split::AmbiguityReason::ClientCommand);
            log_not_sent(console, LogMsg::NotStarted(view_of(reason)), at, cx);
            chooser::open_uncertain(console, vec![reason], cx);
        }
        Resolution::Refused { at, reason } => {
            let msg = match reason {
                RefuseReason::CopyStream => LogMsg::CopyUnsupported,
                RefuseReason::RunAsOne => LogMsg::NotStarted(view_of(reason.fluent_id())),
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

/// Submits `units` as one run of console `console`, marks them at `version` and follows the
/// run's status and result tabs.
pub(crate) fn start_run(console: ConsoleId, version: u64, units: Vec<ExecUnit>, cx: &mut App) {
    if units.is_empty() {
        return;
    }
    let Some(state) = Consoles::get(cx, console).cloned() else {
        return;
    };
    let Some(session) = live_session(cx, console) else {
        if let Some(p) = profile_of(cx, console) {
            connect_first(&p, cx);
        }
        return;
    };
    let Some(editor) = console_exec(cx, console).and_then(|e| e.editor.upgrade()) else {
        return;
    };
    let engine = profile_of(cx, console).map_or(DbmsKind::Unknown, |p| p.engine);
    let service = exec_service(cx);
    let ticket = service.run(
        &session,
        RunRequest {
            console: console.0,
            engine,
            units: units.clone(),
            fetch: FetchPolicy::default(),
            log: state.log.clone(),
        },
    );
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
            _tasks: vec![status, results],
        });
    });
}

/// A run ended: probe the SQL context again when one of its units may have changed it.
pub(crate) fn on_run_ended(console: ConsoleId, cx: &mut App) {
    let reprobe = console_exec(cx, console)
        .and_then(|e| e.run.as_ref())
        .is_some_and(|r| r.units.iter().any(|u| needs_reprobe(&u.text)));
    if reprobe {
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
            r.tabs()
                .iter()
                .find(|t| t.key().unit_ordinal == ordinal && t.key().result_ordinal == 0)
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
