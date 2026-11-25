//! Run as script on a large file (HV-SCRIPT-12, R-M4b-20; NOTES surface 9): the file streamed
//! from disk through the splitter, every unit classified and gated exactly as Run does, on the
//! tab's own connection only, with the progress strip and Stop.
//!
//! ## m4b-t01b-seams: nothing yet (Task 18; the runner is Task 12's).
//!
//! ## m4b-t18-run-as-script
//! **What streams.** A console whose editor is a T2 preview offers **Run as script** in the
//! toolbar's Run slot instead of Run (NOTES ruling 8: plain Run and ⌘↵ are absent there, so a
//! reflex never streams 148 MB). Two previews stream:
//! - a **script tab's** T2 preview: the file, read from disk by the library service through
//!   [`FileSource`] (T10's `ScriptStream` and its `PassRecord`), in the tab's environment as it
//!   is now (R-T5-6), on the tab's own session;
//! - a **pasted preview** (the 20–100 MB paste, Task 11's `PasteAsPreview`): a new console tab
//!   on the source console's connection shows the first 2.5 MB, and the whole pasted text
//!   lives **in memory only**, held here per console ([`PastedSource`]): never in an autosave,
//!   a recovery copy or any file, and gone when the tab closes (a paste can hold anything,
//!   credentials included; L14).
//!
//! **The run** is Task 12's `ScriptRunner::run_checked`, on the core runtime: pass 1 reads the
//! whole file and sends nothing; a refusal anywhere refuses the file (R-M4b-47, R-T12-1,
//! R-T12-2). Between the passes a script tab runs **Task 14's per-Run database check** on its
//! own session (`tabs::run_gate_streamed`: the same probe, the MySQL switch back once, the
//! refusal notice), bound to the session's context generation, which the first unit must still
//! be under (R-M4b-27, R-T14-2); the check comes after pass 1 because pass 1 can take minutes.
//! Every unit then goes through the classifier, the gate and the M3b confirmation dialogs (the
//! port says Execute all and Skip all for an open-ended run, T12 m4). The profile's live
//! environment tag reaches the runner through a watch channel this module keeps current, and
//! the tab's modes reach it as known or not (T12 fr1). A user's No stops the run and the units
//! confirmed before it stay applied; the end says so (R-T12-2).
//!
//! **The strip** (26 px, raised, under the notices): the amber run marker, "Checking the whole
//! file" in pass 1 or "Running as script", the statements sent, the line, the bytes read of the
//! file's size, the elapsed time, and **Stop** (Default, a Tab stop). Stop is the M3 cancel
//! path: it stops between units and cancels the unit in flight (`exec.cancel` stops it too);
//! Escape never does. At the end the strip says how the run ended, with **Close**; the runner
//! writes its lines to the output log. The strip sits in its own focus home whose primary is
//! the editor, so a Stop or Close that removes itself hands the focus to the code (R-M4b-43).
//! Trust signals stay where they are: the strip is under the toolbar and never covers the
//! capsule, the hairline, the environment mark or the "Read-only preview" segment.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_exec::{
    ChunkError, DatabaseCheck, ScriptChunks, ScriptGate, ScriptHistory, ScriptPhase,
    ScriptProgress, ScriptRunOutcome, ScriptRunRequest, ScriptRunner, ScriptStopReason,
};
use hacivat_l10n::ArgValue;
use hacivat_proto::EnvTag;
use hacivat_scripts::{LibraryId, LibraryService, ReadError, RelPath, ScriptStream, TextEncoding};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::focus_home::{HomeSlot, focus_home};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::Tip;
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Task, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::Theme;
use hacivat_ui_editor::{EditorView, PastedDocument, PreviewSpec};
use tokio::sync::{mpsc, oneshot, watch};

use crate::console::{ConsoleId, Consoles};
use crate::scripts::tabs::{self, Load, ScriptDoc, Switch, msg};

// ---- the sources ---------------------------------------------------------------------------------

/// A library file as the runner reads it: pass 1 over T10's [`ScriptStream`], and pass 2
/// reopened with pass 1's `PassRecord` (R-T10-3: refused unless the file is the one pass 1 read,
/// stopped at the first block that differs). The environment is the tab's as it is at each
/// open (the app keeps it current).
pub struct FileSource {
    service: Arc<LibraryService>,
    library: LibraryId,
    rel: RelPath,
    env: watch::Receiver<Option<EnvTag>>,
    encoding: TextEncoding,
    stream: ScriptStream,
}

impl FileSource {
    /// Opens pass 1 of `rel` (on the core runtime).
    pub async fn open(
        service: Arc<LibraryService>,
        library: LibraryId,
        rel: RelPath,
        env: watch::Receiver<Option<EnvTag>>,
        encoding: TextEncoding,
    ) -> Result<FileSource, ChunkError> {
        let now = *env.borrow();
        let stream = service
            .open_stream(library, &rel, now, encoding, None)
            .await
            .map_err(read_error)?;
        Ok(FileSource {
            service,
            library,
            rel,
            env,
            encoding,
            stream,
        })
    }
}

impl ScriptChunks for FileSource {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        hacivat_scripts::ScriptChunks::next_chunk(&mut self.stream).map(|r| r.map_err(chunk_error))
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
        Box::pin(async move {
            let this = *self;
            // The T10 re-review carry: only a pass read to its end is reopened.
            let record = this.stream.record();
            if !record.is_complete() {
                return Err(ChunkError::Changed);
            }
            let env = *this.env.borrow();
            let stream = this
                .service
                .open_stream(this.library, &this.rel, env, this.encoding, Some(record))
                .await
                .map_err(read_error)?;
            Ok(Box::new(FileSource { stream, ..this }) as Box<dyn ScriptChunks>)
        })
    }
}

/// A source that fails at its first read (a file that could not be opened): the runner
/// refuses with its reason and logs it, so the log says why as for any other refusal.
struct Failed(Option<ChunkError>);

impl ScriptChunks for Failed {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        self.0.take().map(Err)
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
        Box::pin(async { Err(ChunkError::Changed) })
    }
}

/// A pasted preview's whole text, held in memory: both passes read the same `Arc`, so pass 2
/// is pass 1's text by construction. Handed out in 64 KiB pieces cut at a character boundary.
pub struct PastedSource {
    text: Arc<str>,
    at: usize,
}

impl PastedSource {
    pub fn new(text: Arc<str>) -> PastedSource {
        PastedSource { text, at: 0 }
    }
}

impl ScriptChunks for PastedSource {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        if self.at >= self.text.len() {
            return None;
        }
        let mut end = (self.at + hacivat_exec::SCRIPT_CHUNK_BYTES).min(self.text.len());
        while !self.text.is_char_boundary(end) {
            end -= 1;
        }
        let chunk = self.text[self.at..end].to_string();
        self.at = end;
        Some(Ok(chunk))
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
        Box::pin(async move {
            // Only a pass read to its end has a second one.
            if self.at < self.text.len() {
                return Err(ChunkError::Changed);
            }
            Ok(Box::new(PastedSource::new(self.text)) as Box<dyn ScriptChunks>)
        })
    }
}

/// The library's read error in the runner's terms (exhaustive: a new variant must be mapped
/// here, never fall through; carries T1a review, T12 m6).
pub fn chunk_error(e: hacivat_scripts::ChunkError) -> ChunkError {
    match e {
        hacivat_scripts::ChunkError::Changed => ChunkError::Changed,
        hacivat_scripts::ChunkError::Decode => ChunkError::Decode,
        hacivat_scripts::ChunkError::Io(kind) => ChunkError::Io(kind),
    }
}

/// Why the library could not open the file, in the runner's terms (exhaustive; T12 m6): a
/// file that is not the one pass 1 read is `Changed`; anything about its text is `Decode`; a
/// path the library refuses, or one this connection no longer shows, cannot be read.
pub fn read_error(e: ReadError) -> ChunkError {
    match e {
        ReadError::Changed => ChunkError::Changed,
        ReadError::NeedsEncoding { .. }
        | ReadError::Binary
        | ReadError::UnknownEncoding
        | ReadError::Lossy
        | ReadError::Undecodable => ChunkError::Decode,
        ReadError::PathRefused | ReadError::OutOfScope => {
            ChunkError::Io(std::io::ErrorKind::PermissionDenied)
        }
        ReadError::Io(kind) => ChunkError::Io(kind),
    }
}

// ---- what a console streams ----------------------------------------------------------------------

/// What console `console` would stream.
enum Streamed {
    /// A script tab's T2 preview (its doc).
    File(Entity<ScriptDoc>),
    /// A pasted preview's whole text, and the script tab it was pasted from.
    Pasted(PastedText),
}

// ## m4b-t18-run-as-script fix round 1 (I3, R-T18-1)
/// A pasted preview's whole text (memory only; module docs) and, when it was pasted into a
/// script tab, that tab's doc: the preview runs only where the tab is bound (its database),
/// through Task 14's per-Run check on the pasted console's own session.
#[derive(Clone)]
struct PastedText {
    text: Arc<str>,
    from: Option<Entity<ScriptDoc>>,
}

/// The pasted previews' texts, by console.
#[derive(Default)]
struct Pasted(HashMap<ConsoleId, PastedText>);

impl Global for Pasted {}

/// The pasted previews waiting for their console tab to be built, by tab.
#[derive(Default)]
struct PendingPreviews(HashMap<hacivat_ui::panes::TabId, PastedText>);

impl Global for PendingPreviews {}

/// The running (or ended, until closed) strips, by console.
#[derive(Default)]
struct Runs(HashMap<ConsoleId, Entity<ScriptRunView>>);

impl Global for Runs {}

fn editor_of(console: ConsoleId, cx: &App) -> Option<Entity<EditorView>> {
    Consoles::get(cx, console)?.editor.clone()
}

// ## m4b-t21-shots (T18 concern: the strip under the T2 banner)
/// Console `console`'s strip came or went: its editor (a cached view) draws again, so the
/// banner moves between its column and the console host's (`EditorView::set_host_banner`).
fn strip_changed(console: ConsoleId, cx: &mut App) {
    if let Some(editor) = editor_of(console, cx) {
        cx.notify(editor.entity_id());
    }
}

fn streamed(console: ConsoleId, cx: &App) -> Option<Streamed> {
    let editor = editor_of(console, cx)?;
    editor.read(cx).preview()?;
    if let Some(doc) = tabs::doc_of_console(console, cx) {
        return (doc.read(cx).tier() == hacivat_scripts::SizeTier::Preview)
            .then_some(Streamed::File(doc));
    }
    cx.try_global::<Pasted>()?
        .0
        .get(&console)
        .cloned()
        .map(Streamed::Pasted)
}

// ## m4b-t18-run-as-script fix round 1 (I1)
/// Whether console `console` shows a T2 preview (a script tab's, or a pasted one), read
/// without the editor (the gutter asks from inside the editor's own update).
pub fn is_preview(console: ConsoleId, cx: &App) -> bool {
    if let Some(doc) = tabs::doc_of_console(console, cx) {
        return doc.read(cx).tier() == hacivat_scripts::SizeTier::Preview;
    }
    cx.try_global::<Pasted>()
        .is_some_and(|p| p.0.contains_key(&console))
}

/// Whether plain Run must not start on console `console`: a streamed run is under way on it, or
/// on the session it shares (two runs never interleave on one session).
pub fn blocks_run(console: ConsoleId, cx: &App) -> bool {
    if running(console, cx) {
        return true;
    }
    let Some(session) = Consoles::get(cx, console)
        .and_then(|s| s.session.as_ref())
        .map(|s| s.id())
    else {
        return false;
    };
    cx.try_global::<Runs>().is_some_and(|runs| {
        runs.0.values().any(|v| {
            let v = v.read(cx);
            v.ended.is_none() && v.session == session
        })
    })
}

/// Whether console `console` offers Run as script (its editor is a T2 preview of a script file
/// or of a paste): the toolbar's Run slot shows it instead of Run.
pub fn offered(console: ConsoleId, cx: &App) -> bool {
    streamed(console, cx).is_some()
}

/// Whether a run is under way on console `console` (its strip is not at its end).
pub fn running(console: ConsoleId, cx: &App) -> bool {
    view_of(console, cx).is_some_and(|v| v.read(cx).ended.is_none())
}

fn view_of(console: ConsoleId, cx: &App) -> Option<Entity<ScriptRunView>> {
    cx.try_global::<Runs>()?.0.get(&console).cloned()
}

/// Whether Run as script can be pressed on console `console`: offered, nothing running, and a
/// script tab may run where it is bound (loaded, its plan, still listed, a connection, not a
/// note, its open switch landed).
pub fn enabled(console: ConsoleId, cx: &App) -> bool {
    match streamed(console, cx) {
        None => false,
        Some(_) if running(console, cx) => false,
        Some(Streamed::File(doc)) => {
            let d = doc.read(cx);
            *d.load() == Load::Ready
                && d.plan.is_ok()
                && !d.unlisted()
                && !d.runs_nowhere(cx)
                && !d.is_markdown(cx)
                && !matches!(d.switch_state(), Switch::Pending | Switch::Failed)
        }
        Some(Streamed::Pasted(_)) => {
            Consoles::get(cx, console).is_some_and(|s| s.profile.is_some())
                && !blocks_run(console, cx)
        }
    }
}

/// Run as script on console `console` (the toolbar's button): checks what Run checks before it
/// starts (the orchestrator's hook: a connection, the SQL context, nothing else running), then
/// streams. Nothing is sent before pass 1 and the tab's database check agreed.
pub fn start(console: ConsoleId, cx: &mut App) {
    if running(console, cx) {
        return;
    }
    let Some(what) = streamed(console, cx) else {
        return;
    };
    if let Streamed::File(doc) = &what {
        if doc.read(cx).runs_nowhere(cx) {
            tabs::say_runs_nowhere(cx);
            return;
        }
        if !enabled(console, cx) {
            return;
        }
    }
    let Some(parts) = crate::exec_ui::orchestrator::script_run_parts(console, cx) else {
        return;
    };
    launch(console, what, parts, cx);
}

/// Stops the run on console `console` (Stop, `exec.cancel`): between units, and the unit in
/// flight is cancelled. `false` when nothing runs there.
pub fn stop(console: ConsoleId, cx: &mut App) -> bool {
    let Some(view) = view_of(console, cx).filter(|v| v.read(cx).ended.is_none()) else {
        return false;
    };
    view.update(cx, |v, cx| v.stop(cx));
    true
}

/// The strip of console `console`, while there is one (the console host draws it under the
/// notices).
pub fn strip(console: ConsoleId, cx: &App) -> Option<AnyView> {
    view_of(console, cx).map(AnyView::from)
}

/// Console `console` closed: its run stops and its pasted text goes.
pub(crate) fn console_closed(console: ConsoleId, cx: &mut App) {
    if let Some(view) = cx
        .has_global::<Runs>()
        .then(|| cx.default_global::<Runs>().0.remove(&console))
        .flatten()
    {
        view.update(cx, |v, _| v.cancel.send_replace(true));
    }
    if cx.has_global::<Pasted>() {
        cx.default_global::<Pasted>().0.remove(&console);
    }
    // ## m4b-t18-run-as-script fix round 1 (m6): and pasted texts held for tabs that are gone.
    prune_pending(cx);
}

// ## m4b-t18-run-as-script fix round 1 (m6)
/// Drops the pasted texts held for tabs no workspace shows any more (closed before they were
/// built).
fn prune_pending(cx: &mut App) {
    if !cx.has_global::<PendingPreviews>() {
        return;
    }
    let open: Vec<hacivat_ui::panes::TabId> = cx
        .global::<PendingPreviews>()
        .0
        .keys()
        .copied()
        .filter(|tab| tab_is_open(*tab, cx))
        .collect();
    cx.default_global::<PendingPreviews>()
        .0
        .retain(|tab, _| open.contains(tab));
}

/// Whether a workspace window shows tab `tab`.
fn tab_is_open(tab: hacivat_ui::panes::TabId, cx: &App) -> bool {
    cx.windows().iter().any(|w| {
        w.downcast::<crate::shell::Workspace>()
            .and_then(|w| w.read(cx).ok())
            .is_some_and(|ws| ws.tab_kind(tab).is_some())
    })
}

/// The host of tab `tab` was built: a pasted text still held for it is either adopted
/// (`adopt_pending`) or dropped here (m6).
pub(crate) fn forget_pending(tab: hacivat_ui::panes::TabId, cx: &mut App) {
    if cx.has_global::<PendingPreviews>() {
        cx.default_global::<PendingPreviews>().0.remove(&tab);
    }
}

/// Whether the pasted preview waiting for tab `tab` came from a script tab whose connection
/// gives each tab a session of its own (MySQL/MariaDB, R-T14-3): the console takes one too.
pub(crate) fn pending_wants_own_session(tab: hacivat_ui::panes::TabId, cx: &App) -> bool {
    cx.try_global::<PendingPreviews>()
        .and_then(|p| p.0.get(&tab))
        .and_then(|p| p.from.as_ref())
        .is_some_and(|d| {
            matches!(
                d.read(cx).plan,
                Ok(crate::scripts::open::SessionPlan::Own { .. })
            )
        })
}

fn notify_host(console: ConsoleId, cx: &mut App) {
    let Some(tab) = Consoles::get(cx, console).map(|s| s.tab) else {
        return;
    };
    if let Some(host) = crate::console::view::host_of(cx, tab) {
        host.update(cx, |_, cx| cx.notify());
    }
}

fn note_output(console: ConsoleId, cx: &mut App) {
    if let Some(results) = Consoles::get(cx, console).and_then(|s| s.results.clone()) {
        results.update(cx, |r, cx| r.note_output(cx));
    }
}

// ---- the run ---------------------------------------------------------------------------------

/// What the run needs of the console, gathered by the orchestrator's hook
/// (`exec_ui::orchestrator::script_run_parts`).
pub(crate) struct Parts {
    pub(crate) session: hacivat_session::session::SessionHandle,
    pub(crate) profile: Arc<hacivat_session::profile::Profile>,
    pub(crate) dialect: sql_dialect::DialectCtx,
    pub(crate) modes_known: bool,
    pub(crate) suppressed: hacivat_exec::SuppressSet,
    pub(crate) history: Option<hacivat_exec::HistoryMeta>,
    pub(crate) history_sink: tokio::sync::mpsc::Sender<Arc<hacivat_exec::HistoryRecord>>,
    pub(crate) confirm_unsafe_where: bool,
    pub(crate) log: Arc<hacivat_exec::SessionLog>,
    pub(crate) runner: ScriptRunner,
    pub(crate) refresh: Option<hacivat_introspect::refresh_coord::RefreshInbox>,
}

fn launch(console: ConsoleId, what: Streamed, parts: Parts, cx: &mut App) {
    let (cancel, cancel_rx) = watch::channel(false);
    let (env, env_rx) = watch::channel(parts.profile.environment);
    let (progress_tx, progress) = watch::channel(ScriptProgress::default());
    // The database check runs on the UI thread (Task 14's gate); the runner asks for it.
    let (ask, asks) = mpsc::unbounded_channel::<oneshot::Sender<Option<u64>>>();
    let check_via = |ask: mpsc::UnboundedSender<oneshot::Sender<Option<u64>>>| -> DatabaseCheck {
        Box::new(move || {
            Box::pin(async move {
                let (reply, answer) = oneshot::channel();
                ask.send(reply).ok()?;
                answer.await.ok().flatten()
            })
        })
    };
    // ## m4b-t18-run-as-script fix round 1 (I3, m1): the doc whose binding is checked, and
    // the run's own session the check must be made on.
    let via = match &what {
        Streamed::File(doc) => Some(doc.downgrade()),
        Streamed::Pasted(p) => p.from.as_ref().map(Entity::downgrade),
    };
    let session_id = parts.session.id();
    let (file, total, source, check): (SharedString, u64, Opening, Option<DatabaseCheck>) =
        match &what {
            Streamed::File(doc) => {
                let d = doc.read(cx);
                let Some(spec) = d.spec(cx) else {
                    return;
                };
                let Some(rt) = crate::scripts::open::current(cx) else {
                    return;
                };
                let file = SharedString::from(tabs::file_label(d, cx));
                let total = d.stamp.as_ref().map_or(0, |s| s.len);
                let opening = Opening::File {
                    service: rt.service,
                    library: spec.library,
                    rel: spec.rel,
                    encoding: d.meta.encoding,
                };
                (file, total, opening, Some(check_via(ask)))
            }
            Streamed::Pasted(p) => (
                msg(cx, "hv-scripts-pasted-name", Vec::new()),
                p.text.len() as u64,
                Opening::Text(p.text.clone()),
                p.from.is_some().then(|| check_via(ask)),
            ),
        };
    let file_name: Arc<str> = Arc::from(file.as_ref());
    let history = parts.history.map(|mut meta| {
        meta.origin = hacivat_exec::HistoryOrigin::File;
        ScriptHistory {
            sink: parts.history_sink.clone(),
            meta,
            file_name: file_name.clone(),
            database: parts
                .session
                .state()
                .borrow()
                .schema
                .as_ref()
                .and_then(|s| s.database.clone()),
        }
    });
    let confirm = crate::exec_ui::AppConfirmPort::for_script_run(console, cx);
    let allowlist: Arc<[Arc<str>]> = parts
        .profile
        .read_only_fn_allowlist
        .iter()
        .map(|s| Arc::from(s.as_str()))
        .collect();
    let flavor =
        hacivat_sql_editor::effective_flavor(&parts.dialect, sql_dialect::ClientFlavor::None);
    let request = Request {
        session: parts.session,
        console: console.0,
        profile: parts.profile.id,
        engine: parts.profile.engine,
        dialect: parts.dialect,
        modes_known: parts.modes_known,
        flavor,
        gate: ScriptGate {
            env: env_rx.clone(),
            confirm_unsafe_where: parts.confirm_unsafe_where,
            allowlist,
            suppressed: parts.suppressed,
            connection: Arc::from(parts.profile.name.as_str()),
        },
        confirm,
        log: parts.log,
        cancel: cancel_rx,
        history,
        refresh: parts.refresh,
    };
    let runner = parts.runner;
    let run = tabs::on_core(cx, async move {
        let source = source.open(env_rx).await;
        let req = request.with_source(source);
        runner.run_checked(req, progress_tx, check).await
    });
    let started = cx.background_executor().now();
    let view = cx.new(|cx| {
        let mut view = ScriptRunView {
            console,
            file,
            total,
            progress: ScriptProgress::default(),
            ended: None,
            started,
            elapsed: Duration::ZERO,
            cancel,
            env,
            home: HomeSlot::new(),
            session: session_id,
            _tasks: Vec::new(),
            _subscriptions: Vec::new(),
        };
        view.follow(progress, asks, via, run, cx);
        view
    });
    cx.default_global::<Runs>().0.insert(console, view);
    notify_host(console, cx);
    strip_changed(console, cx);
}

/// How a run opens its source (on the core runtime).
enum Opening {
    File {
        service: Arc<LibraryService>,
        library: LibraryId,
        rel: RelPath,
        encoding: TextEncoding,
    },
    Text(Arc<str>),
}

impl Opening {
    async fn open(self, env: watch::Receiver<Option<EnvTag>>) -> Box<dyn ScriptChunks> {
        match self {
            Opening::File {
                service,
                library,
                rel,
                encoding,
            } => match FileSource::open(service, library, rel, env, encoding).await {
                Ok(source) => Box::new(source),
                Err(e) => Box::new(Failed(Some(e))),
            },
            Opening::Text(text) => Box::new(PastedSource::new(text)),
        }
    }
}

/// A [`ScriptRunRequest`] waiting for its source (opened on the core runtime).
struct Request {
    session: hacivat_session::session::SessionHandle,
    console: u64,
    profile: hacivat_proto::ProfileId,
    engine: hacivat_proto::DbmsKind,
    dialect: sql_dialect::DialectCtx,
    modes_known: bool,
    flavor: sql_dialect::ClientFlavor,
    gate: ScriptGate,
    confirm: Arc<crate::exec_ui::AppConfirmPort>,
    log: Arc<hacivat_exec::SessionLog>,
    cancel: watch::Receiver<bool>,
    history: Option<ScriptHistory>,
    refresh: Option<hacivat_introspect::refresh_coord::RefreshInbox>,
}

impl Request {
    fn with_source(self, source: Box<dyn ScriptChunks>) -> ScriptRunRequest {
        ScriptRunRequest {
            session: self.session,
            source,
            console: self.console,
            profile: self.profile,
            engine: self.engine,
            dialect: self.dialect,
            modes_known: self.modes_known,
            flavor: self.flavor,
            gate: self.gate,
            confirm: self.confirm,
            log: self.log,
            cancel: self.cancel,
            history: self.history,
            refresh: self.refresh,
        }
    }
}

/// Task 14's database check for console `console`, answered on `reply` with the context
/// generation it read (`None`: refused). A MySQL tab's switch back, or a probe under way, runs
/// the check again when it lands (the gate keeps `rerun`); a refusal drops it, which answers
/// `None` (fail closed).
fn check_database(
    console: ConsoleId,
    // ## m4b-t18-run-as-script fix round 1 (m1): the run's own session; (I3) the doc whose
    // binding is checked (a pasted preview's source script tab).
    session: hacivat_session::session::SessionId,
    via: Option<WeakEntity<ScriptDoc>>,
    reply: Rc<RefCell<Option<oneshot::Sender<Option<u64>>>>>,
    cx: &mut App,
) {
    // The console still holds the session the run was started on, else no check passes.
    let state = Consoles::get(cx, console);
    if state
        .and_then(|s| s.session.as_ref())
        .is_none_or(|s| s.id() != session)
    {
        return;
    }
    let Some(doc) = via.as_ref().and_then(WeakEntity::upgrade) else {
        return;
    };
    // A pasted preview of a script tab whose connection gives each tab a session of its own
    // (R-T14-3) runs only on a session of its own, never the profile's default one.
    let own = matches!(
        doc.read(cx).plan,
        Ok(crate::scripts::open::SessionPlan::Own { .. })
    );
    let is_file = tabs::doc_of_console(console, cx).is_some();
    if own
        && !is_file
        && state.is_some_and(|s| s.client == hacivat_session::session::ClientId::DEFAULT)
    {
        return;
    }
    let again = reply.clone();
    let via_again = via.clone();
    let rerun =
        Box::new(move |cx: &mut App| check_database(console, session, via_again, again, cx));
    if !tabs::run_gate_streamed_for(&doc, console, rerun, cx) {
        return;
    }
    let generation = tabs::take_checked_generation(console, session, cx);
    if let Some(reply) = reply.borrow_mut().take() {
        let _ = reply.send(generation);
    }
}

// ---- the strip ---------------------------------------------------------------------------------

/// How a run ended, as the strip says it.
#[derive(Debug, Clone)]
enum Ended {
    /// Every statement ran.
    Done { n: u64 },
    /// A statement failed (the server's text is in Output).
    Failed { at: u64 },
    /// Stop. `during`: the statement Stop cancelled in flight (R-T18-2).
    Cancelled { n: u64, during: Option<u64> },
    /// The user's No at statement `at`: the `n` before it ran and stay applied (R-T12-2).
    Declined { at: u64 },
    /// The runner's own sentence (`hacivat-core.ftl`) with its line and count.
    Said {
        id: &'static str,
        line: Option<u64>,
        n: Option<u64>,
    },
}

impl Ended {
    fn of(out: &ScriptRunOutcome) -> Ended {
        let at = |p: &Option<hacivat_exec::ScriptPosition>| {
            (p.map(|p| p.line), p.map(|p| p.unit.saturating_sub(1)))
        };
        match out {
            ScriptRunOutcome::Finished(s) => Ended::Done { n: s.units_run },
            ScriptRunOutcome::Failed(s) => Ended::Failed {
                at: s.failed_at.map_or(s.units_run + 1, |p| p.unit),
            },
            ScriptRunOutcome::Stopped(stop, s) => match &stop.reason {
                ScriptStopReason::Cancelled => Ended::Cancelled {
                    n: s.units_run,
                    during: s
                        .cancelled_in_flight
                        .then(|| stop.at.map(|p| p.unit))
                        .flatten(),
                },
                ScriptStopReason::ConfirmCancelled => Ended::Declined {
                    at: stop.at.map_or(s.units_run + 1, |p| p.unit),
                },
                reason => {
                    let (line, n) = at(&stop.at);
                    Ended::Said {
                        id: reason.fluent_id(),
                        line,
                        n,
                    }
                }
            },
            ScriptRunOutcome::Refused(r) => {
                let (line, n) = at(&r.at);
                Ended::Said {
                    id: r.reason.fluent_id(),
                    line,
                    n,
                }
            }
        }
    }

    fn text(&self, file: &str, cx: &App) -> SharedString {
        let count = |n: u64| ArgValue::Count(n);
        match self {
            Ended::Done { n } => msg(
                cx,
                "hv-scripts-run-done",
                vec![("n", count(*n)), ("file", ArgValue::Text(file.to_string()))],
            ),
            Ended::Failed { at } => msg(cx, "hv-scripts-run-stopped", vec![("n", count(*at))]),
            Ended::Cancelled {
                during: Some(at), ..
            } => msg(
                cx,
                "hv-scripts-run-cancelled-during",
                vec![("n", count(*at))],
            ),
            Ended::Cancelled { n, during: None } => {
                msg(cx, "hv-scripts-run-cancelled", vec![("n", count(*n))])
            }
            Ended::Declined { at } => msg(cx, "hv-scripts-run-declined", vec![("n", count(*at))]),
            Ended::Said { id, line, n } => {
                let mut args = Vec::new();
                if let Some(line) = line {
                    args.push(("line", count(*line)));
                }
                if let Some(n) = n {
                    args.push(("n", count(*n)));
                }
                msg(cx, id, args)
            }
        }
    }
}

/// One console's Run-as-script strip (module docs).
pub struct ScriptRunView {
    console: ConsoleId,
    /// The file's name as shown (data), or the pasted text's name.
    file: SharedString,
    /// The whole file's size in bytes.
    total: u64,
    progress: ScriptProgress,
    ended: Option<Ended>,
    /// When it started (the executor clock).
    started: Instant,
    elapsed: Duration,
    cancel: watch::Sender<bool>,
    /// The profile's live environment tag (the runner reads it before every gate and send).
    env: watch::Sender<Option<EnvTag>>,
    home: HomeSlot,
    // ## m4b-t18-run-as-script fix round 1 (I1, m1): the session the run was started on.
    session: hacivat_session::session::SessionId,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<hacivat_ui_core::gpui_surface::Subscription>,
}

impl Drop for ScriptRunView {
    /// A strip that goes (its tab closed) stops its run: a dropped Stop never cancels.
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}

impl ScriptRunView {
    fn follow(
        &mut self,
        mut progress: watch::Receiver<ScriptProgress>,
        mut asks: mpsc::UnboundedReceiver<oneshot::Sender<Option<u64>>>,
        via: Option<WeakEntity<ScriptDoc>>,
        run: impl std::future::Future<Output = Option<ScriptRunOutcome>> + 'static,
        cx: &mut Context<Self>,
    ) {
        let console = self.console;
        let session = self.session;
        // The figures, coalesced to the latest.
        self._tasks.push(cx.spawn(async move |this, cx| {
            while progress.changed().await.is_ok() {
                let now = progress.borrow_and_update().clone();
                let gone = this
                    .update(cx, |v, cx| {
                        v.progress = now;
                        cx.notify();
                    })
                    .is_err();
                if gone {
                    break;
                }
                cx.update(|cx| note_output(console, cx));
            }
        }));
        // The database check, when the runner asks for it.
        self._tasks.push(cx.spawn(async move |_, cx| {
            while let Some(reply) = asks.recv().await {
                let reply = Rc::new(RefCell::new(Some(reply)));
                let via = via.clone();
                cx.update(|cx| check_database(console, session, via, reply, cx));
            }
        }));
        // The end.
        self._tasks.push(cx.spawn(async move |this, cx| {
            let out = run.await;
            let _ = this.update(cx, |v, cx| v.end(out, cx));
        }));
        // The elapsed time, once a second while it runs.
        self._tasks.push(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let running = this
                    .update(cx, |v, cx| {
                        if v.ended.is_none() {
                            v.elapsed = cx
                                .background_executor()
                                .now()
                                .saturating_duration_since(v.started);
                            cx.notify();
                        }
                        v.ended.is_none()
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        }));
        // The live environment tag (T12 fr1 I2): a retag reaches the next gate and send.
        self._subscriptions
            .push(
                cx.observe_global::<crate::connections::ConnectionsModel>(move |v, cx| {
                    // ## m4b-t18-run-as-script fix round 1 (m2): a profile deleted mid-run
                    // stops the run (never "no environment" for the units after it).
                    let Some(profile) = crate::exec_ui::orchestrator::profile_of(cx, console)
                    else {
                        v.cancel.send_replace(true);
                        return;
                    };
                    let tag = profile.environment;
                    v.env.send_if_modified(|now| {
                        let changed = *now != tag;
                        *now = tag;
                        changed
                    });
                }),
            );
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.cancel.send_replace(true);
        cx.notify();
    }

    fn end(&mut self, out: Option<ScriptRunOutcome>, cx: &mut Context<Self>) {
        // The core runtime dropped the run (quitting): nothing more is sent.
        let out = out.unwrap_or(ScriptRunOutcome::Refused(hacivat_exec::ScriptRefusal {
            reason: hacivat_exec::ScriptRefuseReason::Read(ChunkError::Io(
                std::io::ErrorKind::Interrupted,
            )),
            at: None,
        }));
        let suppress = match &out {
            ScriptRunOutcome::Finished(s)
            | ScriptRunOutcome::Failed(s)
            | ScriptRunOutcome::Stopped(_, s) => s.suppress.clone(),
            ScriptRunOutcome::Refused(_) => Vec::new(),
        };
        self.ended = Some(Ended::of(&out));
        self.progress.phase = ScriptPhase::Done;
        let console = self.console;
        cx.defer(move |cx| {
            crate::exec_ui::orchestrator::script_run_ended(console, &suppress, cx);
            note_output(console, cx);
            notify_host(console, cx);
        });
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        let console = self.console;
        cx.defer(move |cx| {
            if cx.has_global::<Runs>() {
                cx.default_global::<Runs>().0.remove(&console);
            }
            notify_host(console, cx);
            strip_changed(console, cx);
        });
    }

    fn running_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let checking = self.progress.phase == ScriptPhase::Checking;
        let state = msg(
            cx,
            if checking {
                "hv-scripts-run-checking"
            } else {
                "hv-scripts-run-running"
            },
            Vec::new(),
        );
        let sent = msg(
            cx,
            "hv-scripts-run-sent",
            vec![("n", ArgValue::Count(self.progress.units_run))],
        );
        let line = msg(
            cx,
            "hv-scripts-run-line",
            vec![("line", ArgValue::Count(self.progress.line.max(1)))],
        );
        let read = msg(
            cx,
            "hv-scripts-run-read",
            vec![
                (
                    "read",
                    ArgValue::Text(
                        crate::console::view::size_text(cx, self.progress.bytes).to_string(),
                    ),
                ),
                (
                    "size",
                    ArgValue::Text(crate::console::view::size_text(cx, self.total).to_string()),
                ),
            ],
        );
        let clock = crate::l10n_global::tr(
            cx,
            hacivat_l10n::ids::hv::CONSOLE_CLOCK,
            hacivat_l10n::ids::hv::HvConsoleClock {
                s: hacivat_l10n::Count(self.elapsed.as_secs()),
            },
        );
        let figure = |text: SharedString, id: &'static str, mono: bool| {
            Label::new(
                text,
                TextSource::Fluent,
                if mono {
                    t.type_ramp.mono_data
                } else {
                    t.type_ramp.caption
                },
                // The Raised Ink Rule: text-2 on the raised strip.
                c.text_2.to_hsla(),
            )
            .id(id)
        };
        let me = cx.weak_entity();
        let stopping = *self.cancel.borrow();
        div()
            .id("script-run-strip")
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md))
            .min_h(px(t.spacing.strip_height))
            .px(px(t.spacing.lg))
            .bg(c.raised.to_hsla())
            .border_b_1()
            .border_color(c.seam.to_hsla())
            .child(run_marker(&theme))
            .child(
                Label::new(
                    state,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text.to_hsla(),
                )
                .id("script-run-state"),
            )
            .child(figure(sent, "script-run-sent", true))
            .children((!checking).then(|| figure(line, "script-run-line", true)))
            .child(figure(read, "script-run-read", false))
            .child(figure(clock, "script-run-clock", true))
            .child(div().flex_1())
            .child(
                Button::new(
                    "script-run-stop",
                    crate::l10n_global::tr(
                        cx,
                        hacivat_l10n::ids::hv::CONSOLE_STOP,
                        hacivat_l10n::NoArgs,
                    ),
                )
                .kind(ButtonKind::Default)
                .icon(IconName::Square)
                .on_raised(true)
                .compact(true)
                .disabled(stopping)
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |v, cx| v.stop(cx));
                }),
            )
            .into_any_element()
    }

    fn ended_row(&self, ended: &Ended, cx: &mut Context<Self>) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let text = ended.text(&self.file, cx);
        let icon = match ended {
            Ended::Done { .. } => IconName::Info,
            _ => IconName::TriangleAlert,
        };
        let me = cx.weak_entity();
        div()
            .id("script-run-strip")
            .flex()
            .flex_row()
            .items_start()
            .gap(px(t.spacing.md))
            .min_h(px(t.spacing.strip_height))
            .px(px(t.spacing.lg))
            .py(px(t.spacing.sm))
            .bg(c.raised.to_hsla())
            .border_b_1()
            .border_color(c.seam.to_hsla())
            // Hue Is Meaning: an ink glyph, never red, for how a run ended.
            .child(
                div().flex_none().pt(px(2.0)).child(
                    Icon::new(icon)
                        .size(IconSize::Chrome)
                        .color(c.text.to_hsla()),
                ),
            )
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    paragraph(
                        text,
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text.to_hsla(),
                    )
                    .id("script-run-end"),
                ),
            )
            .child(
                Button::new(
                    "script-run-close",
                    msg(cx, "hv-scripts-run-close", Vec::new()),
                )
                .kind(ButtonKind::Default)
                .on_raised(true)
                .compact(true)
                .on_click(move |_, _, cx| {
                    let _ = me.update(cx, |v, cx| v.close(cx));
                }),
            )
            .into_any_element()
    }
}

impl Render for ScriptRunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = match self.ended.clone() {
            None => self.running_row(cx),
            Some(ended) => self.ended_row(&ended, cx),
        };
        // R-M4b-43: Stop and Close remove themselves; the focus goes to the code.
        match editor_of(self.console, cx).map(|e| e.focus_handle(cx)) {
            Some(primary) => focus_home(&self.home, primary, row).into_any_element(),
            None => row,
        }
    }
}

// ---- the toolbar's button ------------------------------------------------------------------------

/// The Run slot of a console that offers Run as script (NOTES surface 9): "Run as script",
/// Primary with the play glyph and its tooltip; disabled while a run is under way (its tooltip
/// says why). `narrow`: the icon alone, its word the accessible name and tooltip.
pub(crate) fn toolbar_button(console: ConsoleId, narrow: bool, cx: &App) -> AnyElement {
    let busy = running(console, cx);
    let label = msg(cx, "hv-scripts-run-as-script", Vec::new());
    let tip = if busy {
        msg(cx, "hv-scripts-run-busy", Vec::new())
    } else {
        msg(cx, "hv-scripts-run-as-script-tip", Vec::new())
    };
    let button = Button::new("console-run-as-script", label)
        .icon_only(narrow)
        .kind(ButtonKind::Primary)
        .icon(IconName::Play)
        .on_raised(true)
        .disabled(!enabled(console, cx))
        .on_click(move |_, _, cx| start(console, cx));
    div()
        .id("console-run-as-script-tip")
        .child(button)
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .into_any_element()
}

// ---- the large paste (Task 11's question, R-T11-2) -----------------------------------------------

/// A 20–100 MB paste into console `console`'s editor asks first (Task 11's dialog: "Paste as
/// preview" Default, Cancel Primary and focused, Escape's answer); the answer goes back to the
/// editor (`confirm_paste`), which hands "Paste as preview"'s document to [`open_pasted`].
pub(crate) fn ask_paste(
    editor: &Entity<EditorView>,
    bytes: u64,
    total: u64,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = window.root::<crate::shell::Workspace>().flatten() else {
        editor.update(cx, |e, cx| {
            e.confirm_paste(hacivat_ui_editor::clipboard::PasteAnswer::Cancel, cx)
        });
        return;
    };
    let spec = hacivat_ui_editor::clipboard::paste_confirm_dialog(bytes, total);
    let dialog = ws.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    dialog.update(cx, |d, _| {
        d.set_role(hacivat_ui_core::gpui_surface::Role::AlertDialog)
    });
    let editor = editor.downgrade();
    let answered = Rc::new(RefCell::new(false));
    cx.subscribe(
        &dialog,
        move |_, event: &hacivat_ui::dialog::DialogEvent, cx| {
            if std::mem::replace(&mut *answered.borrow_mut(), true) {
                return;
            }
            let answer = match event {
                hacivat_ui::dialog::DialogEvent::Pressed(i) => {
                    hacivat_ui_editor::clipboard::paste_answer(*i)
                }
                hacivat_ui::dialog::DialogEvent::Cancelled => {
                    hacivat_ui_editor::clipboard::PasteAnswer::Cancel
                }
            };
            let _ = editor.update(cx, |e, cx| e.confirm_paste(answer, cx));
        },
    )
    .detach();
}

/// "Paste as preview" from console `console` (its editor's text is unchanged): a new console
/// tab on the same connection shows the document's first 2.5 MB as a T2 preview, and its whole
/// text is kept in memory for Run as script (module docs). A console with no connection has
/// nothing to open it on: the paste is refused with a toast.
pub(crate) fn open_pasted(
    console: ConsoleId,
    text: &PastedDocument,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(profile) = Consoles::get(cx, console).and_then(|s| s.profile) else {
        let text = msg(cx, "hv-scripts-paste-preview-unbound", Vec::new());
        hacivat_ui::toast::ToastQueue::push(
            cx,
            hacivat_ui::toast::Toast::new(
                hacivat_ui::toast::ToastKind::Info,
                Rc::new(move |_: &App| text.clone()),
            ),
        );
        return;
    };
    let Some(ws) = window.root::<crate::shell::Workspace>().flatten() else {
        return;
    };
    let full = text.shared();
    let cut = hacivat_ui_editor::large_file::preview_cut(
        &full,
        hacivat_ui_editor::TierLimits::default().preview_bytes,
    );
    // ## m4b-t18-run-as-script fix round 1 (I3, R-T18-1): pasted into a script tab, the
    // preview keeps that tab's binding (its database, checked before every run).
    let from = tabs::doc_of_console(console, cx);
    prune_pending(cx);
    crate::console::view::queue_text(profile, full[..cut].to_string(), cx);
    // Registered before the tab is built, so its host takes the session the binding needs.
    let tab = ws.update(cx, |ws, cx| ws.open_console_tab(Some(profile), window, cx));
    if let Some(tab) = tab {
        cx.default_global::<PendingPreviews>()
            .0
            .insert(tab, PastedText { text: full, from });
    }
}

/// The console host of tab `tab` built its editor: a pasted preview waiting for that tab makes
/// it a T2 preview of its first part, and keeps the whole text for console `console`.
pub(crate) fn adopt_pending(
    tab: hacivat_ui::panes::TabId,
    console: ConsoleId,
    editor: &Entity<EditorView>,
    cx: &mut App,
) {
    let Some(pasted) = cx
        .has_global::<PendingPreviews>()
        .then(|| cx.default_global::<PendingPreviews>().0.remove(&tab))
        .flatten()
    else {
        return;
    };
    adopt(console, editor, pasted, cx);
}

fn adopt(console: ConsoleId, editor: &Entity<EditorView>, pasted: PastedText, cx: &mut App) {
    let full = pasted.text.clone();
    let cut = hacivat_ui_editor::large_file::preview_cut(
        &full,
        editor.read(cx).config().tiers.preview_bytes,
    );
    let shown = full[..cut].to_string();
    editor.update(cx, |e, cx| {
        // What the preview shows is the start of what Run as script sends.
        if e.snapshot().buffer.to_string() != shown {
            e.set_text(&shown, cx);
        }
        e.set_preview(
            PreviewSpec {
                shown_bytes: cut as u64,
                total_bytes: full.len() as u64,
            },
            cx,
        );
    });
    cx.default_global::<Pasted>().0.insert(console, pasted);
}

/// Shots and tests (features `shots`, `test-support`).
#[cfg(any(feature = "shots", feature = "test-support"))]
pub mod fixtures {
    use super::*;

    /// Makes console `console` a pasted preview of `text` (what "Paste as preview" opens),
    /// without a 20 MB clipboard.
    pub fn pasted_preview(console: ConsoleId, text: &str, cx: &mut App) {
        if let Some(editor) = editor_of(console, cx) {
            let pasted = PastedText {
                text: Arc::from(text),
                from: None,
            };
            adopt(console, &editor, pasted, cx);
            notify_host(console, cx);
        }
    }

    /// A strip state on console `console` with no run behind it (shots): `progress` running,
    /// or the end `ended` names: `done`, `failed`, `cancelled`, `declined`, `refused`.
    pub fn show_strip(
        console: ConsoleId,
        file: &str,
        total: u64,
        progress: ScriptProgress,
        ended: Option<&str>,
        cx: &mut App,
    ) {
        let ended = ended.map(|e| match e {
            "done" => Ended::Done {
                n: progress.units_run,
            },
            "failed" => Ended::Failed {
                at: progress.units_run + 1,
            },
            "cancelled" => Ended::Cancelled {
                n: progress.units_run,
                during: None,
            },
            "declined" => Ended::Declined {
                at: progress.units_run + 1,
            },
            _ => Ended::Said {
                id: "exec-script-no-boundary-items",
                line: Some(progress.line),
                n: None,
            },
        });
        let file = SharedString::from(file.to_string());
        let started = cx.background_executor().now();
        let view = cx.new(|_| ScriptRunView {
            console,
            file,
            total,
            progress,
            ended,
            started,
            elapsed: Duration::from_secs(42),
            cancel: watch::channel(false).0,
            env: watch::channel(None).0,
            home: HomeSlot::new(),
            session: hacivat_session::session::SessionId(u64::MAX),
            _tasks: Vec::new(),
            _subscriptions: Vec::new(),
        });
        cx.default_global::<Runs>().0.insert(console, view);
        notify_host(console, cx);
        super::strip_changed(console, cx);
    }

    // ## m4b-t22-pre fix round 1 (T22-pre review I1)
    /// Whether Stop was asked of the run on console `console` (a strip of [`show_strip`] has no
    /// run to end).
    pub fn stop_asked(console: ConsoleId, cx: &App) -> bool {
        view_of(console, cx).is_some_and(|v| *v.read(cx).cancel.borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every read error the library can answer maps to a runner reason, and the ones that mean
    /// "not the file pass 1 read" stay `Changed` (T12 m6; carries-w1 T1a review).
    #[test]
    fn every_library_error_maps_to_a_runner_reason() {
        use std::io::ErrorKind;
        let chunks = [
            (hacivat_scripts::ChunkError::Changed, ChunkError::Changed),
            (hacivat_scripts::ChunkError::Decode, ChunkError::Decode),
            (
                hacivat_scripts::ChunkError::Io(ErrorKind::TimedOut),
                ChunkError::Io(ErrorKind::TimedOut),
            ),
        ];
        for (from, to) in chunks {
            assert_eq!(chunk_error(from), to);
        }
        let reads = [
            (ReadError::Changed, ChunkError::Changed),
            (ReadError::NeedsEncoding { guess: None }, ChunkError::Decode),
            (ReadError::Binary, ChunkError::Decode),
            (ReadError::UnknownEncoding, ChunkError::Decode),
            (ReadError::Lossy, ChunkError::Decode),
            (ReadError::Undecodable, ChunkError::Decode),
            (
                ReadError::PathRefused,
                ChunkError::Io(ErrorKind::PermissionDenied),
            ),
            (
                ReadError::OutOfScope,
                ChunkError::Io(ErrorKind::PermissionDenied),
            ),
            (
                ReadError::Io(ErrorKind::NotFound),
                ChunkError::Io(ErrorKind::NotFound),
            ),
        ];
        for (from, to) in reads {
            assert_eq!(read_error(from.clone()), to, "{from:?}");
        }
        // The match above is exhaustive by construction: a variant added to either enum fails
        // to compile `chunk_error`/`read_error` (no `_` arm), so this list is complete too.
    }

    #[test]
    fn a_pasted_text_reads_alike_twice_in_whole_characters() {
        let text: Arc<str> = Arc::from(format!("SELECT 'ş';\n{}", "é".repeat(70_000)).as_str());
        let mut first = PastedSource::new(text.clone());
        let mut pass1 = Vec::new();
        while let Some(chunk) = first.next_chunk() {
            let chunk = chunk.expect("no error");
            assert!(chunk.len() <= hacivat_exec::SCRIPT_CHUNK_BYTES);
            pass1.push(chunk);
        }
        assert!(pass1.len() > 2);
        assert_eq!(pass1.concat(), &*text);
        let second = futures::executor::block_on(Box::new(first).reopen_verified())
            .expect("the second pass");
        let mut second = second;
        let mut pass2 = Vec::new();
        while let Some(chunk) = second.next_chunk() {
            pass2.push(chunk.expect("no error"));
        }
        assert_eq!(pass1, pass2);
    }

    #[test]
    fn a_pass_not_read_to_its_end_is_never_reopened() {
        let mut half = PastedSource::new(Arc::from("SELECT 1;\n".repeat(20_000).as_str()));
        let _ = half.next_chunk();
        let again = futures::executor::block_on(Box::new(half).reopen_verified());
        assert!(matches!(again, Err(ChunkError::Changed)));
    }
}
