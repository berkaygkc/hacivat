//! `SqlLanguageService` (Task 24): one per app, every console document's analysis.
//!
//! **Threading (R-M3-22).** Per document, [`open`](LanguageService::open) starts two tasks: the
//! worker on the GPUI background executor (`worker::run`), fed by a `tokio::sync::watch` of the
//! latest job, and a foreground task that awaits each published analysis, stores it and calls
//! the editor's `on_update` notifier (from its own task, never inside another call of the
//! trait). `change` records the change and replaces the job: an O(1) rope clone and a channel
//! write, nothing lexed, no text copied on the UI thread. Closing a document drops both tasks.
//!
//! **Which analysis answers what.**
//! - Highlighting and statements use the latest analysis, rebased to the editor's version.
//! - The token oracle, the Run preview and [`SqlLanguageService::doc_snapshot`] use only an
//!   analysis of exactly the editor's version made under the console's current SQL context
//!   ([`SqlLanguageService::set_sql_context`]; until it is set a connected console has no
//!   preview and Run analyses its own snapshot).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hacivat_text::changelog::CHANGE_LOG_LEN;
use hacivat_text::{ChangeDesc, Snapshot, TokenOracle};
use hacivat_ui_core::gpui_surface::{App, Task};
use hacivat_ui_editor::{
    DecorationSource, DocContext, DocId, ExecAction, ExecAt, ExecPreview, InvalidateFn,
    LanguageService, NoTokens, PreviewState, StatementSpan,
};
use smallvec::SmallVec;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::DocSnapshot;
use sql_split::exec_range::CandidateKind;
use tokio::sync::watch;

use crate::highlight::HighlightSource;
use crate::preview::{PreviewKey, off_thread};
// ## m3b-t24-exec-ui (named exception)
use crate::preview::{RunPolicy, preview_with};
use crate::statements;
use crate::words::TokenWords;
use crate::worker::{self, Analysis, Faults, Job, PreviewJob};

/// The client flavor a document is analysed with: the dialect's own client whenever it has one
/// (`alg/10` SPLIT-01: PostgreSQL and its wire-compatible servers → psql, MySQL/MariaDB →
/// mysql, SQLite → sqlite3), whatever `flavor` says (review M3: another client's rules are
/// never applied); `flavor` for the other dialects. A PostgreSQL console never splits without
/// psql's rules: with them a pg_dump `COPY` row such as `DROP TABLE users` is data; without them
/// it would be a certain statement. Run must use the same flavor (Task 33).
pub fn effective_flavor(ctx: &DialectCtx, flavor: ClientFlavor) -> ClientFlavor {
    match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => ClientFlavor::Psql,
        DialectId::MySql | DialectId::MariaDb => ClientFlavor::MySqlCli,
        DialectId::Sqlite => ClientFlavor::Sqlite3,
        _ => flavor,
    }
}

/// One document's state, shared by the service, its highlight source, its token oracle and its
/// foreground task (UI thread only).
pub(crate) struct DocShared {
    /// The editor's version, as the last `open` or `change` said.
    editor_version: Cell<u64>,
    /// The newest analysis that landed.
    latest: RefCell<Option<Arc<Analysis>>>,
    /// The changes after `latest`'s version, `(the version each made, the change)`, consecutive.
    pending: RefCell<VecDeque<(u64, ChangeDesc)>>,
    /// Bumped by every change of the SQL context.
    generation: Cell<u64>,
    /// The console's SQL context has been set.
    sql_set: Cell<bool>,
    doc_ctx: Cell<DocContext>,
    remembered: Cell<Option<CandidateKind>>,
    notify: RefCell<Option<InvalidateFn>>,
    statements: RefCell<Option<StatementsCache>>,
    /// The last worker-computed selection preview (review I1), for exactly its key.
    selection: RefCell<Option<(PreviewKey, ExecPreview)>>,
    /// The key last handed to the preview worker (asked once, not per frame).
    asked: RefCell<Option<PreviewKey>>,
    // ## m3b-t24-exec-ui (named exception: CHOOSE-06)
    /// The console's run policy ("Customize…"): the preview asks Run's question under it.
    policy: Cell<RunPolicy>,
}

struct StatementsCache {
    analysis: Arc<Analysis>,
    version: u64,
    range: Range<usize>,
    spans: Arc<[StatementSpan]>,
}

impl DocShared {
    fn new() -> Self {
        DocShared {
            editor_version: Cell::new(0),
            latest: RefCell::new(None),
            pending: RefCell::new(VecDeque::new()),
            generation: Cell::new(0),
            sql_set: Cell::new(false),
            doc_ctx: Cell::new(DocContext::default()),
            remembered: Cell::new(None),
            notify: RefCell::new(None),
            statements: RefCell::new(None),
            selection: RefCell::new(None),
            asked: RefCell::new(None),
            policy: Cell::new(RunPolicy::default()),
        }
    }

    fn notifier(&self) -> Option<InvalidateFn> {
        self.notify.borrow().clone()
    }

    /// Stores a worker-computed preview when it still answers the editor's version under the
    /// current context; returns the notifier to call then.
    fn land_preview(&self, key: PreviewKey, p: ExecPreview) -> Option<InvalidateFn> {
        if self.asked.borrow().as_ref() == Some(&key) {
            *self.asked.borrow_mut() = None;
        }
        if key.version != self.editor_version.get() || key.generation != self.generation.get() {
            return None;
        }
        *self.selection.borrow_mut() = Some((key, p));
        self.notifier()
    }

    pub(crate) fn latest(&self) -> Option<Arc<Analysis>> {
        self.latest.borrow().clone()
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// The analysis of exactly the editor's version, under the current SQL context generation.
    pub(crate) fn current(&self) -> Option<Arc<Analysis>> {
        self.latest().filter(|a| {
            a.version == self.editor_version.get() && a.generation == self.generation.get()
        })
    }

    /// `f` over the changes that made versions `from + 1 ..= to`, oldest first; `None` when
    /// they are not all known.
    pub(crate) fn with_descs<R>(
        &self,
        from: u64,
        to: u64,
        f: impl FnOnce(&[&ChangeDesc]) -> R,
    ) -> Option<R> {
        let pending = self.pending.borrow();
        let descs: Vec<&ChangeDesc> = pending
            .iter()
            .filter(|(v, _)| *v > from && *v <= to)
            .map(|(_, d)| d)
            .collect();
        let first = pending.iter().find(|(v, _)| *v > from).map(|(v, _)| *v);
        let whole = u64::try_from(descs.len()).ok() == to.checked_sub(from)
            && (descs.is_empty() || first == Some(from + 1));
        whole.then(|| f(&descs))
    }

    pub(crate) fn cached_statements(
        &self,
        a: &Arc<Analysis>,
        version: u64,
        range: &Range<usize>,
    ) -> Option<Arc<[StatementSpan]>> {
        let c = self.statements.borrow();
        let c = c.as_ref()?;
        // A new SQL context clears the cache (its answer demotes to `Heuristic`).
        (Arc::ptr_eq(&c.analysis, a) && c.version == version && c.range == *range)
            .then(|| c.spans.clone())
    }

    pub(crate) fn cache_statements(
        &self,
        analysis: Arc<Analysis>,
        version: u64,
        range: Range<usize>,
        spans: Arc<[StatementSpan]>,
    ) {
        *self.statements.borrow_mut() = Some(StatementsCache {
            analysis,
            version,
            range,
            spans,
        });
    }

    /// Records a change that made `version` (from the editor's previous version).
    fn record(&self, version: u64, desc: &ChangeDesc) {
        let prev = self.editor_version.replace(version);
        let mut pending = self.pending.borrow_mut();
        if version == prev + 1 {
            pending.push_back((version, desc.clone()));
            if pending.len() > CHANGE_LOG_LEN {
                pending.pop_front();
            }
        } else {
            // A gap: the chain from the analysis is unknown until the next one lands.
            pending.clear();
        }
    }

    /// Stores a landed analysis (never an older one than stored) and returns the notifier to
    /// call once every borrow is released.
    fn land(&self, a: Arc<Analysis>) -> Option<InvalidateFn> {
        if self
            .latest
            .borrow()
            .as_ref()
            .is_some_and(|old| old.version > a.version)
        {
            return None;
        }
        self.pending.borrow_mut().retain(|(v, _)| *v > a.version);
        *self.latest.borrow_mut() = Some(a);
        *self.statements.borrow_mut() = None;
        self.notify.borrow().clone()
    }
}

/// The running worker of an open document.
struct Worker {
    jobs: watch::Sender<Option<Job>>,
    /// Selection previews too long for the UI thread (review I1).
    previews: watch::Sender<Option<PreviewJob>>,
    /// The analysis worker, the preview worker, and the two foreground tasks that land their
    /// results; dropping them stops all four.
    _tasks: [Task<()>; 4],
}

/// One document: its shared state, its sources, its SQL context and its worker once opened.
struct DocEntry {
    shared: Rc<DocShared>,
    highlight: Rc<HighlightSource>,
    words: Rc<TokenWords>,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    worker: Option<Worker>,
}

impl DocEntry {
    fn new() -> Self {
        let shared = Rc::new(DocShared::new());
        DocEntry {
            highlight: Rc::new(HighlightSource::new(shared.clone())),
            words: Rc::new(TokenWords::new(shared.clone())),
            shared,
            // Until the console's context is set: the generic dialect, for highlighting only.
            ctx: DialectCtx::new(DialectId::Generic, None),
            flavor: ClientFlavor::None,
            worker: None,
        }
    }

    fn job(&self, snap: &Snapshot) -> Job {
        Job {
            version: snap.version,
            generation: self.shared.generation.get(),
            buffer: snap.buffer.clone(),
            ctx: self.ctx,
            flavor: self.flavor,
        }
    }
}

/// The SQL language service: one per app, every console's documents.
#[derive(Default)]
pub struct SqlLanguageService {
    docs: RefCell<HashMap<DocId, DocEntry>>,
    /// Documents closed and not opened again: calls for them are ignored (review M6).
    closed: RefCell<HashSet<DocId>>,
    analyses: Arc<AtomicUsize>,
    faults: Arc<Faults>,
}

impl fmt::Debug for SqlLanguageService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlLanguageService")
            .field("docs", &self.docs.borrow().len())
            .field("analyses", &self.analyses())
            .finish_non_exhaustive()
    }
}

impl SqlLanguageService {
    pub fn new() -> Rc<Self> {
        Rc::new(SqlLanguageService::default())
    }

    /// The dialect and client flavor `doc` is analysed with: the console's SQL context
    /// (R-M3-39). `ClientFlavor::None` takes the dialect's own client ([`effective_flavor`]).
    /// A new context re-analyses the document; until that analysis lands, no preview, token
    /// words or `doc_snapshot` come from the old one. May be called before `open`.
    ///
    /// A change tells the editor (its `on_update` notifier, deferred to the end of the current
    /// update, never called inside this one; review I2), so a band drawn under the old context
    /// is asked again and finds no preview until the new analysis lands. Ignored for a closed
    /// document.
    pub fn set_sql_context(&self, doc: DocId, ctx: DialectCtx, flavor: ClientFlavor, cx: &mut App) {
        if self.closed.borrow().contains(&doc) {
            return;
        }
        let flavor = effective_flavor(&ctx, flavor);
        let mut docs = self.docs.borrow_mut();
        let e = docs.entry(doc).or_insert_with(DocEntry::new);
        if e.shared.sql_set.get() && e.ctx == ctx && e.flavor == flavor {
            return;
        }
        if let Some(notify) = e.shared.notifier() {
            cx.defer(move |cx| notify(cx));
        }
        e.ctx = ctx;
        e.flavor = flavor;
        e.shared.sql_set.set(true);
        let generation = e.shared.generation.get() + 1;
        e.shared.generation.set(generation);
        *e.shared.statements.borrow_mut() = None;
        if let Some(w) = &e.worker {
            w.jobs.send_modify(|job| {
                if let Some(job) = job {
                    job.ctx = ctx;
                    job.flavor = flavor;
                    job.generation = generation;
                }
            });
        }
    }

    /// The chooser kind the console remembers (R-M3-36): Run's chooser opens on it, so the
    /// preview shows that candidate. May be called before `open`; ignored for a closed
    /// document. A change tells the editor, deferred like [`Self::set_sql_context`]'s.
    pub fn set_remembered_kind(&self, doc: DocId, kind: Option<CandidateKind>, cx: &mut App) {
        if self.closed.borrow().contains(&doc) {
            return;
        }
        let mut docs = self.docs.borrow_mut();
        let e = docs.entry(doc).or_insert_with(DocEntry::new);
        if e.shared.remembered.replace(kind) != kind
            && let Some(notify) = e.shared.notifier()
        {
            cx.defer(move |cx| notify(cx));
        }
    }

    // ## m3b-t24-exec-ui (named exception: CHOOSE-06)
    /// The console's run policy (`RunPolicy`, "Customize…"): the preview resolves under it, as
    /// Run does. May be called before `open`; ignored for a closed document. A change tells the
    /// editor, deferred like [`Self::set_remembered_kind`]'s.
    pub fn set_run_policy(&self, doc: DocId, policy: RunPolicy, cx: &mut App) {
        if self.closed.borrow().contains(&doc) {
            return;
        }
        let mut docs = self.docs.borrow_mut();
        let e = docs.entry(doc).or_insert_with(DocEntry::new);
        if e.shared.policy.replace(policy) != policy
            && let Some(notify) = e.shared.notifier()
        {
            cx.defer(move |cx| notify(cx));
        }
    }

    /// How many analyses the workers have run (tests: `open` and `change` analyse nothing on
    /// the calling thread).
    #[doc(hidden)]
    pub fn analyses(&self) -> usize {
        self.analyses.load(Ordering::Relaxed)
    }

    /// Test hook: the next worker step (an analysis or a preview) panics (review M1).
    #[doc(hidden)]
    pub fn inject_worker_panic(&self) {
        self.faults.inject.store(true, Ordering::Relaxed);
    }

    /// How many worker panics were contained.
    #[doc(hidden)]
    pub fn worker_panics(&self) -> usize {
        self.faults.panics.load(Ordering::Relaxed)
    }

    /// How many documents the service holds state for (tests: none after `close`).
    #[doc(hidden)]
    pub fn open_documents(&self) -> usize {
        self.docs.borrow().len()
    }

    /// The analysis of `doc` at exactly `version` under the console's current SQL context, if
    /// there is one (Run reuses it; R-M3-22). `None` before the context is set.
    pub fn doc_snapshot(&self, doc: DocId, version: u64) -> Option<DocSnapshot> {
        let docs = self.docs.borrow();
        let sh = &docs.get(&doc)?.shared;
        let a = sh.latest()?;
        (sh.sql_set.get() && a.version == version && a.generation == sh.generation.get())
            .then(|| a.doc.clone())
    }
}

/// The next analysis the worker publishes, stored; the notifier to call for it. `None` when
/// the worker or the document is gone.
async fn next_landed(
    results: &mut watch::Receiver<Option<Arc<Analysis>>>,
    shared: &Weak<DocShared>,
) -> Option<Option<InvalidateFn>> {
    results.changed().await.ok()?;
    let a = results.borrow_and_update().clone();
    let sh = shared.upgrade()?;
    Some(a.and_then(|a| sh.land(a)))
}

/// The next selection preview the preview worker publishes, stored when it is still current;
/// the notifier to call for it. `None` when the worker or the document is gone.
async fn next_preview(
    results: &mut watch::Receiver<Option<(PreviewKey, ExecPreview)>>,
    shared: &Weak<DocShared>,
) -> Option<Option<InvalidateFn>> {
    results.changed().await.ok()?;
    let answer = results.borrow_and_update().clone();
    let sh = shared.upgrade()?;
    Some(answer.and_then(|(key, p)| sh.land_preview(key, p)))
}

impl LanguageService for SqlLanguageService {
    fn open(&self, doc: DocId, snap: &Snapshot, ctx: DocContext, cx: &mut App) {
        self.closed.borrow_mut().remove(&doc);
        let mut docs = self.docs.borrow_mut();
        let e = docs.entry(doc).or_insert_with(DocEntry::new);
        let sh = &e.shared;
        sh.editor_version.set(snap.version);
        sh.doc_ctx.set(ctx);
        sh.pending.borrow_mut().clear();
        *sh.latest.borrow_mut() = None;
        *sh.statements.borrow_mut() = None;
        *sh.selection.borrow_mut() = None;
        *sh.asked.borrow_mut() = None;
        let (jobs, job_rx) = watch::channel(Some(e.job(snap)));
        let (out, results) = watch::channel(None);
        let analysis = cx.background_executor().spawn(worker::run(
            job_rx,
            out,
            self.analyses.clone(),
            self.faults.clone(),
        ));
        let (previews, preview_rx) = watch::channel(None);
        let (preview_out, preview_results) = watch::channel(None);
        let preview = cx.background_executor().spawn(worker::previews(
            preview_rx,
            preview_out,
            self.faults.clone(),
        ));
        // The foreground halves: land each result and call the notifier from their own task,
        // outside every borrow and outside any other call of the trait.
        let weak = Rc::downgrade(sh);
        let mut results = results;
        let landing = cx.spawn(async move |cx| {
            while let Some(notify) = next_landed(&mut results, &weak).await {
                if let Some(notify) = notify {
                    cx.update(|cx| notify(cx));
                }
            }
        });
        let weak = Rc::downgrade(sh);
        let mut preview_results = preview_results;
        let preview_landing = cx.spawn(async move |cx| {
            while let Some(notify) = next_preview(&mut preview_results, &weak).await {
                if let Some(notify) = notify {
                    cx.update(|cx| notify(cx));
                }
            }
        });
        e.worker = Some(Worker {
            jobs,
            previews,
            _tasks: [analysis, preview, landing, preview_landing],
        });
    }

    fn set_context(&self, doc: DocId, ctx: DocContext, cx: &mut App) {
        let _ = cx;
        if let Some(e) = self.docs.borrow().get(&doc) {
            e.shared.doc_ctx.set(ctx);
        }
    }

    fn change(&self, doc: DocId, snap: &Snapshot, changes: &ChangeDesc, cx: &mut App) {
        let _ = cx;
        let docs = self.docs.borrow();
        let Some(e) = docs.get(&doc) else {
            return;
        };
        let Some(w) = &e.worker else {
            return;
        };
        e.shared.record(snap.version, changes);
        // The worker takes the newest job when it is free (latest wins).
        w.jobs.send_replace(Some(e.job(snap)));
    }

    fn close(&self, doc: DocId) {
        self.closed.borrow_mut().insert(doc);
        let entry = self.docs.borrow_mut().remove(&doc);
        if let Some(e) = entry {
            // Sources the editor still holds paint nothing from here on.
            *e.shared.latest.borrow_mut() = None;
            e.shared.pending.borrow_mut().clear();
            *e.shared.selection.borrow_mut() = None;
            drop(e);
        }
    }

    fn on_update(&self, doc: DocId, notify: InvalidateFn) {
        if self.closed.borrow().contains(&doc) {
            return;
        }
        let mut docs = self.docs.borrow_mut();
        let e = docs.entry(doc).or_insert_with(DocEntry::new);
        *e.shared.notify.borrow_mut() = Some(notify);
    }

    fn statements(&self, doc: DocId, version: u64, range: Range<usize>) -> Arc<[StatementSpan]> {
        let sh = match self.docs.borrow().get(&doc) {
            Some(e) => e.shared.clone(),
            None => return Arc::from(Vec::new()),
        };
        statements::at(&sh, version, range)
    }

    fn exec_preview(
        &self,
        doc: DocId,
        version: u64,
        at: &ExecAt,
        action: ExecAction,
    ) -> Option<ExecPreview> {
        let docs = self.docs.borrow();
        let e = docs.get(&doc)?;
        let sh = &e.shared;
        if version != sh.editor_version.get() {
            return None;
        }
        let a = sh.latest().filter(|a| a.version == version)?;
        let connected = sh.doc_ctx.get().connected;
        // Connected, the split must be the console's dialect's: no preview until it is.
        if connected && !(sh.sql_set.get() && a.generation == sh.generation.get()) {
            return None;
        }
        if !off_thread(at) {
            // ## m3b-t24-exec-ui (named exception): under the console's run policy.
            return Some(preview_with(
                &a.doc,
                at,
                action,
                connected,
                sh.remembered.get(),
                &sh.policy.get(),
            ));
        }
        // A long selection is lexed and split by the preview worker, never here (review I1):
        // `None` (no band) until its answer for exactly this question lands.
        let key = PreviewKey {
            version: a.version,
            generation: a.generation,
            at: at.clone(),
            action,
            remembered: sh.remembered.get(),
            // ## m3b-t24-exec-ui (named exception)
            policy: sh.policy.get(),
        };
        if let Some((k, p)) = sh.selection.borrow().as_ref()
            && *k == key
        {
            let mut p = p.clone();
            if !connected {
                p.state = PreviewState::NoConnection;
            }
            return Some(p);
        }
        if sh.asked.borrow().as_ref() != Some(&key)
            && let Some(w) = &e.worker
        {
            *sh.asked.borrow_mut() = Some(key.clone());
            w.previews.send_replace(Some(PreviewJob {
                key,
                doc: a.doc.clone(),
            }));
        }
        None
    }

    fn token_oracle(&self, doc: DocId) -> Rc<dyn TokenOracle> {
        match self.docs.borrow().get(&doc) {
            Some(e) => e.words.clone(),
            None => Rc::new(NoTokens),
        }
    }

    // ## m3c-t34-editor (named exception, fix round 1 M9): MySQL and MariaDB need the space
    // after `--`; every other dialect reads SQL's `--` and `/* */`.
    fn comment_tokens(&self, doc: DocId) -> hacivat_ui_editor::CommentTokens {
        let mysql = self
            .docs
            .borrow()
            .get(&doc)
            .is_some_and(|e| matches!(e.ctx.id, DialectId::MySql | DialectId::MariaDb));
        if mysql {
            hacivat_ui_editor::CommentTokens::MYSQL
        } else {
            hacivat_ui_editor::CommentTokens::SQL
        }
    }

    // ## m3c-t32-fold (named exception)
    fn folding_ranges(&self, doc: DocId, version: u64) -> Arc<[hacivat_text::FoldRange]> {
        match self.docs.borrow().get(&doc) {
            Some(e) => crate::folds::at(&e.shared, version),
            None => Arc::from([]),
        }
    }

    fn decoration_sources(&self, doc: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        let mut out: SmallVec<[Rc<dyn DecorationSource>; 2]> = SmallVec::new();
        if let Some(e) = self.docs.borrow().get(&doc) {
            out.push(e.highlight.clone());
        }
        out
    }
}
