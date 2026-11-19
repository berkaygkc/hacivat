// ## m4-t15-engine
//! The scheduler: one actor per profile, the catalog's only writer (alg/02 HV-INTRO-01
//! "Scheduler"). It owns the queue, the running tasks and the publisher; workers
//! ([`super::work`]) do the I/O and the merge and send their answer back here, where it is
//! published in one step. Requests and answers are messages: nothing here blocks, and every
//! cache read or write runs on the blocking pool.
//!
//! **Validation tasks** (periodic, R-M4-20) are queued by [`Actor::validate_due`]; Task 22
//! replaces their body with the cheap namespace check (named lines, File ownership T22).

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use hacivat_meta::delta::CatalogDelta;
use hacivat_meta::merge::{self, Deletions, MergeOutcome, MergePolicy, NamespaceResult, Trigger};
use hacivat_meta::name::{FoldRule, NameClass};
use hacivat_meta::persist::{CacheError, SaveDebouncer};
use hacivat_meta::snapshot::{CatalogChange, ContainerKey, LoadState};
use hacivat_meta::{
    Catalog, KindGroup, Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey, Publisher,
    Watermark,
};
use sql_dialect::DialectId;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::TaskProgress;
use super::levels::{LevelPolicy, NsRole};
use super::work::{self, Merged, Output, Spec, Work, WorkerCtx};
use super::{CacheHandle, EngineConfig, EngineEvent, EngineLimits, Phase, Priority, Progress};
use crate::ddl_effects::Target;
use crate::introspector::{IntroCaps, IntroError, Staleness};
use crate::pack::Mode;
use crate::scope::{CurrentCtx, NsInfo, ScopePattern, parse_scope};
use crate::ticket::TicketSender;

/// A request from a handle.
#[derive(Debug)]
pub(crate) enum Request {
    Ready,
    OnConnect,
    /// Load a namespace (an expansion, a lookup that needed it).
    Load(NsKey),
    /// A lookup answered `Missing` there: check it for changes.
    Probe(NsKey),
    Children(ObjectKey),
    Elevate(ObjectKey, Level),
    Refresh(Target, Trigger),
    CheckState(Vec<ObjectKey>),
    Confirm {
        ns: NsKey,
        accept: bool,
    },
}

/// A message to the actor.
#[derive(Debug)]
pub(crate) enum Cmd {
    Request {
        req: Request,
        prio: Priority,
        reply: TicketSender,
    },
    SetScope(ScopePattern),
    SetCurrent(CurrentCtx),
    LevelChanged(ContainerKey),
    Cancel(NsKey),
    Shutdown,
}

/// Someone waiting for a task.
enum Waiter {
    Ticket(TicketSender),
    /// One part of a request split over several tasks (a state check that re-reads several
    /// namespaces): resolved when every part is.
    Join(Arc<Mutex<Join>>),
}

struct Join {
    left: usize,
    generation: u64,
    error: Option<IntroError>,
    waiters: Vec<Waiter>,
}

impl Waiter {
    fn resolve(self, result: Result<u64, IntroError>) {
        match self {
            Waiter::Ticket(t) => t.send(result),
            Waiter::Join(j) => {
                let done = {
                    let mut j = j.lock().unwrap_or_else(PoisonError::into_inner);
                    match result {
                        Ok(g) => j.generation = j.generation.max(g),
                        Err(e) => {
                            j.error.get_or_insert(e);
                        }
                    }
                    j.left = j.left.saturating_sub(1);
                    (j.left == 0).then(|| {
                        let r = match j.error.take() {
                            Some(e) => Err(e),
                            None => Ok(j.generation),
                        };
                        (std::mem::take(&mut j.waiters), r)
                    })
                };
                if let Some((waiters, r)) = done {
                    resolve_all(waiters, r);
                }
            }
        }
    }
}

fn resolve_all(waiters: Vec<Waiter>, result: Result<u64, IntroError>) {
    for w in waiters {
        w.resolve(result.clone());
    }
}

/// What follows a task when it succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Nothing,
    /// The on-connect algorithm's step 2–3 (after the scope refresh).
    OnConnect,
    /// A names-first pass: then a full pass at the namespace's auto level.
    FullAtAuto,
}

/// A task in the queue or running.
struct Job {
    id: u64,
    seq: u64,
    prio: Priority,
    work: Work,
    trigger: Trigger,
    /// Waiters, and whether each waits for the continuation (`then`) too.
    waiters: Vec<(Waiter, bool)>,
    not_before: Option<Instant>,
    then: Then,
    /// Made by a follow-up: it schedules no further follow-up.
    follow_up: bool,
    /// A load (not a refresh): skipped when the catalog already holds what it would read.
    load_only: bool,
}

impl Job {
    fn add(&mut self, w: Waiter, through_then: bool) {
        self.waiters.push((w, through_then));
    }

    fn resolve(self, result: Result<u64, IntroError>) {
        resolve_all(self.waiters.into_iter().map(|(w, _)| w).collect(), result);
    }
}

struct Running {
    job: Job,
    cancel: CancellationToken,
}

/// A merge the mass-deletion guard held back (D-039).
struct Pending {
    outcome: MergeOutcome,
    keep: Option<NamespaceResult>,
    prev_ptr: Option<usize>,
    level: Level,
    pass: bool,
}

/// The pass mode's strength for coalescing.
fn rank(m: Mode) -> u8 {
    match m {
        Mode::Brief => 0,
        Mode::Incremental => 1,
        Mode::Full | Mode::System => 2,
    }
}

/// Whether a running task's answer serves a new (non-refresh) request too.
fn covers(running: &Work, new: &Work) -> bool {
    match (running, new) {
        (Work::ScopeRefresh, Work::ScopeRefresh) => true,
        (
            Work::Pass {
                ns: a,
                mode: ma,
                level: la,
            },
            Work::Pass {
                ns: b,
                mode: mb,
                level: lb,
            },
        ) => a == b && la >= lb && rank(*ma) >= rank(*mb),
        (
            Work::Pass {
                ns: a,
                mode,
                level: la,
            },
            Work::Fragment {
                ns: b, level: lb, ..
            },
        ) => a == b && *mode != Mode::Brief && la >= lb,
        (
            Work::Fragment {
                ns: a,
                keys: ka,
                level: la,
            },
            Work::Fragment {
                ns: b,
                keys: kb,
                level: lb,
            },
        ) => a == b && la >= lb && kb.iter().all(|k| ka.contains(k)),
        _ => false,
    }
}

/// Whether two queued tasks coalesce into one.
fn same_key(a: &Work, b: &Work) -> bool {
    match (a, b) {
        (Work::ScopeRefresh, Work::ScopeRefresh) => true,
        (Work::Pass { ns: x, .. }, Work::Pass { ns: y, .. })
        | (Work::Fragment { ns: x, .. }, Work::Fragment { ns: y, .. }) => x == y,
        _ => false,
    }
}

fn is_timeout(e: &IntroError) -> bool {
    matches!(
        e,
        IntroError::Timeout | IntroError::Meta(hacivat_session::meta::MetaError::Timeout)
    )
}

fn two_level(d: DialectId) -> bool {
    matches!(d, DialectId::Postgres | DialectId::PgWire(_))
}

/// The major objects of a namespace (its top-level objects): the auto level's count.
fn major_count(ns: &NamespaceSnapshot) -> u32 {
    ns.root().child_count()
}

/// The relations of a namespace: the bulk policy's count.
fn relation_count(ns: &NamespaceSnapshot) -> u32 {
    let dbms = ns.dbms();
    let n = ns
        .children(0)
        .filter(|(_, o)| o.kind().group(dbms) == Some(KindGroup::Relations))
        .count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the actor of `cfg` on its runtime.
pub(super) fn spawn(
    cfg: EngineConfig,
    rx: mpsc::UnboundedReceiver<Cmd>,
    progress: Arc<watch::Sender<Progress>>,
    events: broadcast::Sender<EngineEvent>,
    levels: Arc<Mutex<LevelPolicy>>,
) {
    let rt = cfg.runtime.clone();
    let actor = Actor::new(cfg, progress, events, levels);
    rt.spawn(actor.run(rx));
}

struct Actor {
    catalog: Arc<Catalog>,
    publisher: Publisher,
    ctx: Arc<WorkerCtx>,
    caps: IntroCaps,
    dialect: DialectId,
    cache: Option<CacheHandle>,
    limits: EngineLimits,
    rt: tokio::runtime::Handle,
    levels: Arc<Mutex<LevelPolicy>>,
    progress: Arc<watch::Sender<Progress>>,
    events: broadcast::Sender<EngineEvent>,
    scope: ScopePattern,
    /// The empty pattern (`@:@`): what selects the current namespace.
    current_only: Option<ScopePattern>,
    current: CurrentCtx,
    /// The last scope refresh's listing, the namespaces in scope, the current one, the system
    /// ones (`None` until the first scope refresh).
    listed: Option<Vec<NsInfo>>,
    in_scope: Option<BTreeSet<NsKey>>,
    current_ns: Option<NsKey>,
    system_ns: HashSet<NsKey>,
    queue: Vec<Job>,
    running: HashMap<u64, Running>,
    deferred_confirms: Vec<(NsKey, bool, Waiter)>,
    pending: HashMap<NsKey, Pending>,
    timeouts: HashMap<NsKey, u8>,
    recent_children: HashMap<NsKey, Vec<(Instant, ObjectKey)>>,
    debouncer: SaveDebouncer,
    validate_every: Option<Duration>,
    next_validate: Option<Instant>,
    root_cancel: CancellationToken,
    done_tx: mpsc::UnboundedSender<(u64, Output)>,
    done_rx: mpsc::UnboundedReceiver<(u64, Output)>,
    next_id: u64,
}

impl Actor {
    fn new(
        cfg: EngineConfig,
        progress: Arc<watch::Sender<Progress>>,
        events: broadcast::Sender<EngineEvent>,
        levels: Arc<Mutex<LevelPolicy>>,
    ) -> Actor {
        let server = cfg.catalog.current().server.clone();
        let dialect = cfg.scope.dialect();
        let fold = FoldRule::for_names(server.ident.dialect, &server.ident, NameClass::Table)
            .unwrap_or(FoldRule::Exact);
        let caps = cfg.introspector.capabilities();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        let ctx = Arc::new(WorkerCtx {
            meta: cfg.meta.clone(),
            intro: cfg.introspector.clone(),
            dbms: cfg.dbms,
            fold,
            filters: cfg.filters.clone(),
            limits: cfg.limits,
            events: events.clone(),
        });
        Actor {
            catalog: cfg.catalog,
            publisher: cfg.catalog_publisher,
            ctx,
            caps,
            dialect,
            cache: cfg.cache,
            limits: cfg.limits,
            rt: cfg.runtime,
            levels,
            progress,
            events,
            current_only: parse_scope("", dialect).ok(),
            scope: cfg.scope,
            current: cfg.current,
            listed: None,
            in_scope: None,
            current_ns: None,
            system_ns: HashSet::new(),
            queue: Vec::new(),
            running: HashMap::new(),
            deferred_confirms: Vec::new(),
            pending: HashMap::new(),
            timeouts: HashMap::new(),
            recent_children: HashMap::new(),
            debouncer: SaveDebouncer::new(cfg.limits.save_debounce),
            validate_every: cfg.validate_every,
            next_validate: cfg.validate_every.map(|d| Instant::now() + d),
            root_cancel: CancellationToken::new(),
            done_tx,
            done_rx,
            next_id: 0,
        }
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        self.load_cache().await;
        loop {
            self.pump();
            let wake = self.next_wake();
            tokio::select! {
                biased;
                Some((id, out)) = self.done_rx.recv() => self.on_done(id, out),
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Shutdown) | None => break,
                    Some(cmd) => self.on_cmd(cmd),
                },
                () = sleep_until(wake) => self.on_timer(),
            }
        }
        self.stop().await;
        // `rx` drops here: `IntroHandle::closed` resolves after the flush.
        drop(rx);
    }

    // ---- start and stop ---------------------------------------------------------------------

    /// HV-INTRO-01 step 1 / HV-MODEL-03 step 3: the cached namespaces, published stale.
    async fn load_cache(&mut self) {
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let loaded = tokio::task::spawn_blocking(move || lock(&cache).load_all()).await;
        if let Ok(Ok(all)) = loaded {
            let changes = all.into_changes();
            if !changes.is_empty() {
                // Not offered for saving: these rows are what the cache holds.
                let _ = self.publisher.publish(changes);
            }
        }
    }

    async fn stop(&mut self) {
        self.root_cancel.cancel();
        for (_, r) in self.running.drain() {
            r.job.resolve(Err(IntroError::Cancelled));
        }
        for j in self.queue.drain(..) {
            j.resolve(Err(IntroError::Cancelled));
        }
        for (_, _, w) in self.deferred_confirms.drain(..) {
            w.resolve(Err(IntroError::Cancelled));
        }
        self.progress.send_replace(Progress::default());
        let all = self.debouncer.take_all();
        if let Some(save) = self.save(all) {
            let _ = save.await;
        }
    }

    // ---- the loop ---------------------------------------------------------------------------

    fn next_wake(&self) -> Option<Instant> {
        let now = Instant::now();
        let queued = self
            .queue
            .iter()
            .filter_map(|j| j.not_before)
            .filter(|t| *t > now);
        let save = self.debouncer.next_due().map(Instant::from_std);
        queued.chain(save).chain(self.next_validate).min()
    }

    fn on_timer(&mut self) {
        let now = Instant::now();
        let due = self.debouncer.take_due(now.into_std());
        drop(self.save(due));
        if self.next_validate.is_some_and(|t| t <= now) {
            self.validate_due();
        }
    }

    fn on_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Request { req, prio, reply } => self.on_request(req, prio, Waiter::Ticket(reply)),
            Cmd::SetScope(p) => {
                self.scope = p;
                self.rescope();
            }
            Cmd::SetCurrent(c) => {
                self.current = c;
                self.rescope();
            }
            Cmd::LevelChanged(container) => self.level_changed(&container),
            Cmd::Cancel(ns) => self.cancel(&ns),
            Cmd::Shutdown => {}
        }
    }

    fn new_job(&mut self, work: Work, prio: Priority, trigger: Trigger) -> Job {
        self.next_id += 1;
        Job {
            id: self.next_id,
            seq: self.next_id,
            prio,
            work,
            trigger,
            waiters: Vec::new(),
            not_before: None,
            then: Then::Nothing,
            follow_up: false,
            load_only: false,
        }
    }

    /// Queues `job`. A load (`fresh == false`) joins a running task that already reads what it
    /// wants (single flight); any request coalesces with a queued task of the same key.
    fn enqueue(&mut self, mut job: Job, fresh: bool) {
        if !fresh
            && let Some(r) = self
                .running
                .values_mut()
                .find(|r| covers(&r.job.work, &job.work))
        {
            r.job.waiters.append(&mut job.waiters);
            return;
        }
        if let Some(q) = self.queue.iter_mut().find(|q| same_key(&q.work, &job.work)) {
            q.prio = q.prio.max(job.prio);
            if job.trigger == Trigger::User {
                q.trigger = Trigger::User;
            }
            q.follow_up &= job.follow_up;
            q.load_only &= job.load_only;
            q.waiters.append(&mut job.waiters);
            match (&mut q.work, job.work) {
                (
                    Work::Pass { mode, level, .. },
                    Work::Pass {
                        mode: m2,
                        level: l2,
                        ..
                    },
                ) => {
                    if rank(m2) > rank(*mode) {
                        *mode = m2;
                        q.then = Then::Nothing;
                    }
                    *level = (*level).max(l2);
                }
                (
                    Work::Fragment { keys, level, .. },
                    Work::Fragment {
                        keys: k2,
                        level: l2,
                        ..
                    },
                ) => {
                    for k in k2 {
                        if !keys.contains(&k) {
                            keys.push(k);
                        }
                    }
                    *level = (*level).max(l2);
                }
                _ => {}
            }
            if q.then == Then::Nothing && job.then != Then::Nothing {
                q.then = job.then;
            }
            return;
        }
        self.queue.push(job);
    }

    /// Starts as many ready tasks as there are free workers, best first; each namespace (and
    /// the scope refresh) runs one task at a time.
    fn pump(&mut self) {
        self.run_deferred_confirms();
        let now = Instant::now();
        while self.running.len() < self.limits.workers.max(1) {
            let pick = self
                .queue
                .iter()
                .enumerate()
                .filter(|(_, j)| j.not_before.is_none_or(|t| t <= now) && self.free(&j.work))
                .max_by(|(_, a), (_, b)| a.prio.cmp(&b.prio).then(b.seq.cmp(&a.seq)))
                .map(|(i, _)| i);
            let Some(i) = pick else {
                break;
            };
            let job = self.queue.remove(i);
            self.start(job);
        }
    }

    fn busy(&self, ns: &NsKey) -> bool {
        self.running.values().any(|r| r.job.work.ns() == Some(ns))
    }

    fn free(&self, w: &Work) -> bool {
        match w {
            Work::ScopeRefresh => !self
                .running
                .values()
                .any(|r| r.job.work == Work::ScopeRefresh),
            Work::Pass { ns, .. } | Work::Fragment { ns, .. } => !self.busy(ns),
            Work::CheckState { .. } => true,
        }
    }

    fn out_of_scope(&self, ns: &NsKey) -> bool {
        ns.kind != NsKind::Server && self.in_scope.as_ref().is_some_and(|s| !s.contains(ns))
    }

    fn start(&mut self, mut job: Job) {
        let snap = self.catalog.current();
        if let Some(ns) = job.work.ns()
            && self.out_of_scope(ns)
        {
            job.resolve(Err(IntroError::Cancelled));
            return;
        }
        let prev = job
            .work
            .ns()
            .and_then(|ns| snap.namespaces.get(ns).cloned());
        match &mut job.work {
            Work::Pass { mode, .. } => {
                let incremental_ok = self.caps.incremental
                    && prev
                        .as_ref()
                        .is_some_and(|p| p.meta().watermark != Watermark::None);
                if *mode == Mode::Incremental && !incremental_ok {
                    *mode = Mode::Full;
                }
            }
            Work::Fragment { ns, keys, level } if job.load_only && self.holds(ns, keys, *level) => {
                job.resolve(Ok(snap.generation));
                return;
            }
            _ => {}
        }
        let system = job.work.ns().is_some_and(|ns| self.system_ns.contains(ns));
        let spec = Spec {
            work: job.work.clone(),
            prev,
            system,
            trigger: job.trigger,
        };
        let cancel = self.root_cancel.child_token();
        let (id, ctx, done) = (job.id, self.ctx.clone(), self.done_tx.clone());
        let token = cancel.clone();
        self.rt.spawn(async move {
            let out = work::run(ctx, spec, token).await;
            let _ = done.send((id, out));
        });
        self.running.insert(id, Running { job, cancel });
        self.update_progress();
    }

    /// Whether the catalog holds every key of `ns` at `level` already.
    fn holds(&self, ns: &NsKey, keys: &[ObjectKey], level: Level) -> bool {
        let snap = self.catalog.current();
        let loaded = matches!(
            snap.state(&ContainerKey::Namespace(ns.clone())),
            LoadState::Loaded { level: l, .. } if *l >= level
        );
        loaded
            && snap
                .namespaces
                .get(ns)
                .is_some_and(|a| keys.iter().all(|k| a.find(k).is_some()))
    }

    fn update_progress(&self) {
        let mut tasks: Vec<TaskProgress> = self
            .running
            .values()
            .map(|r| TaskProgress {
                ns: r.job.work.ns().cloned().unwrap_or_else(NsKey::server),
                phase: match &r.job.work {
                    Work::ScopeRefresh => Phase::ListNamespaces,
                    Work::Pass { .. } => Phase::Tables,
                    Work::Fragment { .. } => Phase::Columns,
                    Work::CheckState { .. } => Phase::Validate,
                },
                done: 0,
                total: match &r.job.work {
                    Work::Fragment { keys, .. } | Work::CheckState { keys } => {
                        Some(keys.len() as u64)
                    }
                    _ => None,
                },
            })
            .collect();
        tasks.sort_by(|a, b| a.ns.cmp(&b.ns));
        self.progress.send_if_modified(|p| {
            let changed = p.tasks != tasks;
            p.tasks = tasks;
            changed
        });
    }

    // ---- requests ---------------------------------------------------------------------------

    fn on_request(&mut self, req: Request, prio: Priority, w: Waiter) {
        match req {
            Request::Ready => w.resolve(Ok(self.catalog.current().generation)),
            Request::OnConnect => {
                let mut job = self.new_job(Work::ScopeRefresh, prio, Trigger::Background);
                job.then = Then::OnConnect;
                job.add(w, true);
                self.enqueue(job, true);
            }
            Request::Load(ns) => self.load(ns, prio, w),
            Request::Probe(ns) => {
                if self.catalog.current().namespaces.contains_key(&ns) && !self.out_of_scope(&ns) {
                    let level = self.level_of(&ns);
                    let job = self.pass(ns, Mode::Incremental, level, prio, Trigger::Background, w);
                    self.enqueue(job, false);
                } else {
                    self.load(ns, prio, w);
                }
            }
            Request::Children(key) => self.children(key, prio, w),
            Request::Elevate(key, level) => {
                let ns = self.ns_of(&key);
                if self.out_of_scope(&ns) {
                    return w.resolve(Err(IntroError::Cancelled));
                }
                let mut job = self.fragment_job(ns, vec![key], level, prio, Trigger::Background);
                job.load_only = true;
                job.add(w, false);
                self.enqueue(job, false);
            }
            Request::Refresh(target, trigger) => self.refresh(target, trigger, prio, w),
            Request::CheckState(keys) => {
                let mut job = self.new_job(Work::CheckState { keys }, prio, Trigger::Background);
                job.add(w, false);
                self.queue.push(job);
            }
            Request::Confirm { ns, accept } => self.deferred_confirms.push((ns, accept, w)),
        }
    }

    fn pass(
        &mut self,
        ns: NsKey,
        mode: Mode,
        level: Level,
        prio: Priority,
        trigger: Trigger,
        w: Waiter,
    ) -> Job {
        let mut job = self.new_job(Work::Pass { ns, mode, level }, prio, trigger);
        job.add(w, false);
        job
    }

    fn fragment_job(
        &mut self,
        ns: NsKey,
        keys: Vec<ObjectKey>,
        level: Level,
        prio: Priority,
        trigger: Trigger,
    ) -> Job {
        let mut job = self.new_job(Work::Fragment { ns, keys, level }, prio, trigger);
        job.not_before = Some(Instant::now() + self.limits.fragment_window);
        job
    }

    /// An expansion or a lookup's load: nothing when loaded, an incremental pass of a stale
    /// cached namespace, else names first then the auto level.
    fn load(&mut self, ns: NsKey, prio: Priority, w: Waiter) {
        if self.out_of_scope(&ns) {
            return w.resolve(Err(IntroError::Cancelled));
        }
        let snap = self.catalog.current();
        let state = snap.state(&ContainerKey::Namespace(ns.clone())).clone();
        let has_arena = snap.namespaces.contains_key(&ns);
        match state {
            LoadState::Loaded { stale: false, .. } | LoadState::Partial { .. } if has_arena => {
                w.resolve(Ok(snap.generation));
            }
            _ if has_arena => {
                let level = self.level_of(&ns);
                let job = self.pass(ns, Mode::Incremental, level, prio, Trigger::Background, w);
                self.enqueue(job, false);
            }
            _ => {
                let mut job =
                    self.pass(ns, Mode::Brief, Level::Names, prio, Trigger::Background, w);
                job.then = Then::FullAtAuto;
                self.enqueue(job, false);
            }
        }
    }

    /// The bulk policy (alg/02 HV-INTRO-01): a per-object fragment, or one bulk pass of the
    /// namespace when ≥ `bulk_distinct` distinct objects were asked for within `bulk_window`,
    /// or the namespace has fewer than `bulk_small_ns` relations.
    fn children(&mut self, key: ObjectKey, prio: Priority, w: Waiter) {
        let ns = self.ns_of(&key);
        if self.out_of_scope(&ns) {
            return w.resolve(Err(IntroError::Cancelled));
        }
        if self.holds(&ns, std::slice::from_ref(&key), Level::Structure) {
            return w.resolve(Ok(self.catalog.current().generation));
        }
        let now = Instant::now();
        let window = self.limits.bulk_window;
        let recent = self.recent_children.entry(ns.clone()).or_default();
        recent.retain(|(t, _)| now.duration_since(*t) <= window);
        if !recent.iter().any(|(_, k)| *k == key) {
            recent.push((now, key.clone()));
        }
        let distinct = recent.len();
        let small = self
            .catalog
            .current()
            .namespaces
            .get(&ns)
            .is_some_and(|a| relation_count(a) < self.limits.bulk_small_ns);
        if small || distinct >= self.limits.bulk_distinct {
            self.recent_children.remove(&ns);
            let level = self.level_of(&ns).max(Level::Structure);
            let mut job = self.pass(ns.clone(), Mode::Full, level, prio, Trigger::Background, w);
            // A queued per-object read of the namespace is subsumed by the bulk pass.
            if let Some(i) = self.queue.iter().position(|q| {
                q.load_only && matches!(&q.work, Work::Fragment { ns: n, .. } if *n == ns)
            }) {
                let q = self.queue.remove(i);
                job.waiters.extend(q.waiters);
            }
            self.enqueue(job, false);
        } else {
            let mut job =
                self.fragment_job(ns, vec![key], Level::Structure, prio, Trigger::Background);
            job.load_only = true;
            job.add(w, false);
            self.enqueue(job, false);
        }
    }

    fn refresh(&mut self, target: Target, trigger: Trigger, prio: Priority, w: Waiter) {
        match target {
            Target::Namespace(ns) => {
                if self.out_of_scope(&ns) {
                    return w.resolve(Err(IntroError::Cancelled));
                }
                let level = self.level_of(&ns);
                let job = self.pass(ns, Mode::Incremental, level, prio, trigger, w);
                self.enqueue(job, true);
            }
            Target::Objects { ns, keys } => {
                if self.out_of_scope(&ns) {
                    return w.resolve(Err(IntroError::Cancelled));
                }
                let level = self.state_level(&ns);
                let mut job = self.fragment_job(ns, keys, level, prio, trigger);
                job.add(w, false);
                self.enqueue(job, true);
            }
            Target::ScopeRefresh => {
                let mut job = self.new_job(Work::ScopeRefresh, prio, trigger);
                job.add(w, false);
                self.enqueue(job, true);
            }
        }
    }

    fn cancel(&mut self, ns: &NsKey) {
        for r in self.running.values() {
            if r.job.work.ns() == Some(ns) {
                r.cancel.cancel();
            }
        }
        let (gone, keep): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|j| j.work.ns() == Some(ns));
        self.queue = keep;
        for j in gone {
            j.resolve(Err(IntroError::Cancelled));
        }
    }

    // ---- levels -----------------------------------------------------------------------------

    fn role_of(&self, ns: &NsKey) -> NsRole {
        if self.current_ns.as_ref() == Some(ns) {
            NsRole::Current
        } else if self.system_ns.contains(ns) {
            NsRole::System
        } else {
            NsRole::Other
        }
    }

    /// The level `ns` is read at now (its arena's object count; 0 when not loaded).
    fn level_of(&self, ns: &NsKey) -> Level {
        let count = self
            .catalog
            .current()
            .namespaces
            .get(ns)
            .map_or(0, |a| major_count(a));
        lock(&self.levels)
            .level_for(ns, self.role_of(ns), count)
            .level
    }

    /// The level `ns` was published at (a fragment re-reads at it).
    fn state_level(&self, ns: &NsKey) -> Level {
        let snap = self.catalog.current();
        match snap.state(&ContainerKey::Namespace(ns.clone())) {
            LoadState::Loaded { level, .. } => *level,
            _ => snap
                .namespaces
                .get(ns)
                .map_or(Level::Structure, |a| a.meta().level),
        }
    }

    /// A pill changed: every loaded namespace below `container` now under its level is re-read.
    fn level_changed(&mut self, container: &ContainerKey) {
        let ContainerKey::Namespace(top) = container else {
            return;
        };
        let snap = self.catalog.current();
        let below = |ns: &NsKey| {
            top.kind == NsKind::Server
                || ns == top
                || (top.kind == NsKind::Database && ns.path.first() == top.path.first())
        };
        let stale: Vec<(NsKey, Level)> = snap
            .namespaces
            .keys()
            .filter(|ns| below(ns) && !self.out_of_scope(ns))
            .filter_map(|ns| {
                let want = self.level_of(ns);
                let have = match snap.state(&ContainerKey::Namespace(ns.clone())) {
                    LoadState::Loaded { level, .. } => *level,
                    _ => return None,
                };
                (want > have).then(|| (ns.clone(), want))
            })
            .collect();
        for (ns, level) in stale {
            let job = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Full,
                    level,
                },
                Priority::User,
                Trigger::Background,
            );
            self.enqueue(job, true);
        }
    }

    /// The namespace an object lives in: the longest listed prefix of its path, else the
    /// dialect's schema depth.
    fn ns_of(&self, key: &ObjectKey) -> NsKey {
        let snap = self.catalog.current();
        for k in (0..key.path.len()).rev() {
            for kind in [NsKind::Schema, NsKind::Database, NsKind::Server] {
                let ns = NsKey {
                    kind,
                    path: key.path[..k].iter().cloned().collect(),
                };
                if snap.namespaces.contains_key(&ns)
                    || snap
                        .states
                        .contains_key(&ContainerKey::Namespace(ns.clone()))
                {
                    return ns;
                }
            }
        }
        let depth = if two_level(self.dialect) { 2 } else { 1 };
        NsKey::schema(
            key.path
                .iter()
                .take(depth.min(key.path.len().saturating_sub(1)))
                .cloned(),
        )
    }

    // ---- scope ------------------------------------------------------------------------------

    /// The scope or the current namespace changed: recompute what is in scope and publish it.
    fn rescope(&mut self) {
        if self.listed.is_none() {
            return;
        }
        let changes = self.scope_changes();
        if let Err(e) = self.publish(changes, None) {
            let _ = e;
        }
        // What left the scope: running results are discarded, queued work is dropped.
        let out: Vec<NsKey> = self
            .running
            .values()
            .filter_map(|r| r.job.work.ns().cloned())
            .chain(self.queue.iter().filter_map(|j| j.work.ns().cloned()))
            .filter(|ns| self.out_of_scope(ns))
            .collect();
        for ns in out {
            self.cancel(&ns);
        }
    }

    /// The changes of the last listing under the current scope: listed namespaces in scope get
    /// a state (`NotLoaded`) so a lookup knows they exist; out-of-scope ones are removed and
    /// marked `OutOfScope`; namespaces the server no longer lists are removed; the schema lists
    /// are loaded.
    fn scope_changes(&mut self) -> Vec<CatalogChange> {
        let Some(infos) = self.listed.clone() else {
            return Vec::new();
        };
        let in_scope = self.scope.select(&infos, &self.current);
        self.current_ns = self.current_only.as_ref().and_then(|p| {
            p.select(&infos, &self.current)
                .into_iter()
                .find(|k| k.kind == NsKind::Schema)
        });
        self.system_ns = infos
            .iter()
            .filter(|i| i.system)
            .map(|i| i.key.clone())
            .collect();
        let snap = self.catalog.current();
        let mut changes = Vec::new();
        for info in &infos {
            let k = &info.key;
            let key = ContainerKey::Namespace(k.clone());
            let state = snap.states.get(&key);
            let has = snap.namespaces.contains_key(k);
            if in_scope.contains(k) {
                if (!has && state.is_none()) || matches!(state, Some(LoadState::OutOfScope)) {
                    changes.push(CatalogChange::SetState(key, LoadState::NotLoaded));
                }
            } else {
                if has {
                    changes.push(CatalogChange::Remove(k.clone()));
                }
                if has || !matches!(state, Some(LoadState::OutOfScope)) {
                    changes.push(CatalogChange::SetState(key, LoadState::OutOfScope));
                }
            }
        }
        let listed: HashSet<&NsKey> = infos.iter().map(|i| &i.key).collect();
        let known: BTreeSet<NsKey> = snap
            .namespaces
            .keys()
            .cloned()
            .chain(snap.states.keys().filter_map(|k| match k {
                ContainerKey::Namespace(ns) => Some(ns.clone()),
                _ => None,
            }))
            .filter(|k| k.kind != NsKind::Server)
            .collect();
        for k in known {
            if !listed.contains(&k) {
                changes.push(CatalogChange::Remove(k.clone()));
                let _ = self.events.send(EngineEvent::NamespaceDropped { ns: k });
            }
        }
        let listed_state = LoadState::Loaded {
            level: Level::Names,
            at: SystemTime::now(),
            watermark: Watermark::None,
            stale: false,
        };
        if two_level(self.dialect) {
            let dbs: BTreeSet<Name> = infos
                .iter()
                .filter(|i| i.key.kind == NsKind::Schema && i.key.path.len() == 2)
                .map(|i| i.key.path[0].clone())
                .collect();
            for db in dbs {
                changes.push(CatalogChange::SetState(
                    ContainerKey::schema_list(Some(&db)),
                    listed_state.clone(),
                ));
            }
        } else {
            changes.push(CatalogChange::SetState(
                ContainerKey::schema_list(None),
                listed_state,
            ));
        }
        self.in_scope = Some(in_scope);
        changes
    }

    // ---- answers ----------------------------------------------------------------------------

    fn on_done(&mut self, id: u64, out: Output) {
        let Some(Running { job, .. }) = self.running.remove(&id) else {
            return;
        };
        self.update_progress();
        match out {
            Output::Namespaces(infos) => self.listed_done(job, infos),
            Output::Merged(m) => self.merged(job, *m),
            Output::Staleness(keys, st) => self.staleness(job, keys, st),
            Output::Failed(e) => self.failed(job, e),
        }
    }

    fn listed_done(&mut self, job: Job, infos: Vec<NsInfo>) {
        self.listed = Some(infos);
        let changes = self.scope_changes();
        let generation = match self.publish(changes, None) {
            Ok(g) => g,
            Err(e) => return job.resolve(Err(e)),
        };
        if job.then != Then::OnConnect {
            return job.resolve(Ok(generation));
        }
        // On connect, steps 2–3: the current namespace (others load on first expansion).
        let (now, later): (Vec<_>, Vec<_>) = job.waiters.into_iter().partition(|(_, t)| !*t);
        resolve_all(now.into_iter().map(|(w, _)| w).collect(), Ok(generation));
        let waiters: Vec<Waiter> = later.into_iter().map(|(w, _)| w).collect();
        let Some(ns) = self.current_ns.clone().filter(|ns| !self.out_of_scope(ns)) else {
            return resolve_all(waiters, Ok(generation));
        };
        let cached = self.catalog.current().namespaces.contains_key(&ns);
        let mut next = if cached {
            let level = self.level_of(&ns);
            self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Incremental,
                    level,
                },
                job.prio,
                Trigger::Background,
            )
        } else {
            let mut j = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Brief,
                    level: Level::Names,
                },
                job.prio,
                Trigger::Background,
            );
            j.then = Then::FullAtAuto;
            j
        };
        for w in waiters {
            next.add(w, true);
        }
        self.enqueue(next, true);
    }

    fn merged(&mut self, mut job: Job, m: Merged) {
        let ns = m.ns.clone();
        if self.out_of_scope(&ns) {
            return job.resolve(Err(IntroError::Cancelled));
        }
        let snap = self.catalog.current();
        let now_ptr = snap.namespaces.get(&ns).map(|a| Arc::as_ptr(a) as usize);
        if now_ptr != m.prev_ptr {
            // Published by something else since the task started: read again.
            self.next_id += 1;
            job.seq = self.next_id;
            job.not_before = None;
            self.queue.push(job);
            return;
        }
        if m.outcome.is_pending() {
            return self.hold_pending(job, m);
        }
        let (level, pass) = (m.level, m.pass);
        let (follow, follow_ns) = (m.follow, m.follow_ns);
        let (next, delta, report) = m.outcome.into_parts();
        let changes = self.applied_changes(&ns, next, &report, level, pass);
        let generation = match self.publish(changes, Some(delta)) {
            Ok(g) => g,
            Err(e) => return job.resolve(Err(e)),
        };
        self.timeouts.remove(&ns);
        self.pending.remove(&ns);
        if !job.follow_up {
            self.follow_up(&ns, follow, follow_ns);
        }
        self.then(job, &ns, generation);
    }

    /// What follows a published pass: the continuation, then the waiters.
    fn then(&mut self, job: Job, ns: &NsKey, generation: u64) {
        if job.then != Then::FullAtAuto {
            return job.resolve(Ok(generation));
        }
        let level = self.level_of(ns);
        let (later, now): (Vec<_>, Vec<_>) = job.waiters.into_iter().partition(|(_, t)| *t);
        resolve_all(now.into_iter().map(|(w, _)| w).collect(), Ok(generation));
        let later: Vec<Waiter> = later.into_iter().map(|(w, _)| w).collect();
        if level == Level::Names {
            // A large namespace stays at names (D-038): the brief pass was its pass.
            return resolve_all(later, Ok(generation));
        }
        let mut next = self.new_job(
            Work::Pass {
                ns: ns.clone(),
                mode: Mode::Full,
                level,
            },
            job.prio,
            job.trigger,
        );
        for w in later {
            next.add(w, false);
        }
        self.enqueue(next, false);
    }

    /// Re-reads after a merge: the objects it asked for, and the namespace after a top-level
    /// rename (T10 carries M-2). Follow-ups never schedule follow-ups.
    fn follow_up(&mut self, ns: &NsKey, keys: Vec<ObjectKey>, whole: bool) {
        if !keys.is_empty() {
            let level = self.state_level(ns);
            let mut job = self.fragment_job(
                ns.clone(),
                keys,
                level,
                Priority::Background,
                Trigger::Background,
            );
            job.follow_up = true;
            self.enqueue(job, true);
        }
        if whole {
            let level = self.level_of(ns);
            let mut job = self.new_job(
                Work::Pass {
                    ns: ns.clone(),
                    mode: Mode::Incremental,
                    level,
                },
                Priority::Background,
                Trigger::Background,
            );
            job.follow_up = true;
            self.enqueue(job, true);
        }
    }

    /// The changes that publish a merged namespace: the arena (unique: it is moved, never
    /// copied, when stamped), its state when a pass read it, and the `Children` state of every
    /// kind whose existence query failed (and of every kind that read fine again).
    fn applied_changes(
        &self,
        ns: &NsKey,
        next: NamespaceSnapshot,
        report: &merge::MergeReport,
        level: Level,
        pass: bool,
    ) -> Vec<CatalogChange> {
        let snap = self.catalog.current();
        let root = next.object_key(0);
        let at = SystemTime::now();
        let mut states = Vec::new();
        if pass {
            let state = match report.stale_kinds.values().next() {
                Some(code) => LoadState::Partial { reason: *code },
                None => LoadState::Loaded {
                    // Objects re-read below the namespace's level hold only what this pass
                    // read: the namespace is at the pass's level now (the merge never lowers).
                    level: if report.below_level.is_empty() {
                        next.meta().level
                    } else {
                        level.min(next.meta().level)
                    },
                    at,
                    watermark: next.meta().watermark.clone(),
                    stale: false,
                },
            };
            states.push(CatalogChange::SetState(
                ContainerKey::Namespace(ns.clone()),
                state,
            ));
        }
        if let Some(root) = root {
            for (kind, code) in &report.stale_kinds {
                states.push(CatalogChange::SetState(
                    ContainerKey::Children {
                        ns: ns.clone(),
                        parent: root.clone(),
                        kind: *kind,
                    },
                    LoadState::Failed {
                        at,
                        error: *code,
                        retry_after: self.limits.failed_retry_after,
                    },
                ));
            }
            if pass {
                for (k, st) in snap.states.iter() {
                    if let ContainerKey::Children {
                        ns: n,
                        parent,
                        kind,
                    } = k
                        && n == ns
                        && *parent == root
                        && matches!(st, LoadState::Failed { .. })
                        && !report.stale_kinds.contains_key(kind)
                    {
                        states.push(CatalogChange::SetState(
                            k.clone(),
                            LoadState::Loaded {
                                level,
                                at,
                                watermark: Watermark::None,
                                stale: false,
                            },
                        ));
                    }
                }
            }
        }
        let mut changes = vec![CatalogChange::Put(Arc::new(next))];
        changes.extend(states);
        changes
    }

    /// The guard fired (D-039, R-M4-22): nothing of the pass applies; the namespace is marked
    /// stale and the merge kept until the user decides.
    fn hold_pending(&mut self, job: Job, m: Merged) {
        let ns = m.ns.clone();
        let vanished = match &m.outcome {
            MergeOutcome::PendingConfirmation { vanished, .. } => *vanished,
            MergeOutcome::Applied { .. } => 0,
        };
        let snap = self.catalog.current();
        let mut changes = Vec::new();
        if let LoadState::Loaded {
            level,
            at,
            watermark,
            ..
        } = snap.state(&ContainerKey::Namespace(ns.clone())).clone()
        {
            changes.push(CatalogChange::SetState(
                ContainerKey::Namespace(ns.clone()),
                LoadState::Loaded {
                    level,
                    at,
                    watermark,
                    stale: true,
                },
            ));
        }
        let generation = self.publish(changes, None);
        self.pending.insert(
            ns.clone(),
            Pending {
                outcome: m.outcome,
                keep: m.keep,
                prev_ptr: m.prev_ptr,
                level: m.level,
                pass: m.pass,
            },
        );
        self.publisher.announce_pending_confirmation(ns, vanished);
        job.resolve(generation);
    }

    /// Confirmations wait until their namespace runs nothing.
    fn run_deferred_confirms(&mut self) {
        let all = std::mem::take(&mut self.deferred_confirms);
        for (ns, accept, w) in all {
            if self.busy(&ns) {
                self.deferred_confirms.push((ns, accept, w));
            } else {
                let r = self.confirm(&ns, accept);
                w.resolve(r);
            }
        }
    }

    /// "Remove from cache" publishes the held merge; "Keep cached" re-merges the pass with its
    /// deletions suppressed (I-5). A held merge whose namespace changed since is dropped.
    fn confirm(&mut self, ns: &NsKey, accept: bool) -> Result<u64, IntroError> {
        let snap = self.catalog.current();
        let Some(p) = self.pending.remove(ns) else {
            return Ok(snap.generation);
        };
        let prev = snap.namespaces.get(ns).cloned();
        if prev.as_ref().map(|a| Arc::as_ptr(a) as usize) != p.prev_ptr {
            return Ok(snap.generation);
        }
        let outcome = if accept {
            p.outcome
        } else {
            let (Some(r), Some(prev)) = (p.keep, prev) else {
                return Ok(snap.generation);
            };
            let policy = MergePolicy {
                trigger: Trigger::User,
                deletions: Deletions::Suppress,
                ..MergePolicy::default()
            };
            merge::merge(Some(&prev), r, &policy)
                .map_err(|_| IntroError::Protocol(Name::from("merge")))?
        };
        let (next, delta, report) = outcome.into_parts();
        let changes = self.applied_changes(ns, next, &report, p.level, p.pass);
        let generation = self.publish(changes, Some(delta))?;
        self.timeouts.remove(ns);
        Ok(generation)
    }

    /// A state check answered: what changed or was dropped is re-read (the ticket resolves
    /// with the generation that holds it).
    fn staleness(&mut self, job: Job, keys: Vec<ObjectKey>, st: Vec<Staleness>) {
        let mut by_ns: BTreeMap<NsKey, Vec<ObjectKey>> = BTreeMap::new();
        for (k, s) in keys.into_iter().zip(st) {
            if matches!(s, Staleness::Changed | Staleness::Dropped) {
                let ns = self.ns_of(&k);
                by_ns.entry(ns).or_default().push(k);
            }
        }
        if by_ns.is_empty() {
            return job.resolve(Ok(self.catalog.current().generation));
        }
        let join = Arc::new(Mutex::new(Join {
            left: by_ns.len(),
            generation: 0,
            error: None,
            waiters: job.waiters.into_iter().map(|(w, _)| w).collect(),
        }));
        for (ns, keys) in by_ns {
            let level = self.state_level(&ns);
            let mut f = self.fragment_job(ns, keys, level, job.prio, Trigger::Background);
            f.add(Waiter::Join(join.clone()), false);
            self.enqueue(f, true);
        }
    }

    fn failed(&mut self, job: Job, e: IntroError) {
        if let Some(ns) = job.work.ns().cloned() {
            if is_timeout(&e) {
                self.timed_out(&ns);
            }
            let snap = self.catalog.current();
            let none_before = !snap.namespaces.contains_key(&ns)
                && matches!(
                    snap.state(&ContainerKey::Namespace(ns.clone())),
                    LoadState::NotLoaded | LoadState::Failed { .. }
                );
            if matches!(job.work, Work::Pass { .. })
                && !matches!(e, IntroError::Cancelled)
                && none_before
                && !self.out_of_scope(&ns)
            {
                let state = LoadState::Failed {
                    at: SystemTime::now(),
                    error: e.code(),
                    retry_after: self.limits.failed_retry_after,
                };
                let _ = self.publish(
                    vec![CatalogChange::SetState(ContainerKey::Namespace(ns), state)],
                    None,
                );
            }
        }
        job.resolve(Err(e));
    }

    /// Back-pressure (alg/02 step 7): the second timeout of a namespace lowers its level.
    fn timed_out(&mut self, ns: &NsKey) {
        let n = self.timeouts.entry(ns.clone()).or_insert(0);
        *n += 1;
        if *n < 2 {
            return;
        }
        self.timeouts.remove(ns);
        let count = self
            .catalog
            .current()
            .namespaces
            .get(ns)
            .map_or(0, |a| major_count(a));
        let role = self.role_of(ns);
        let lowered = lock(&self.levels).lower(ns, role, count);
        if let Some(level) = lowered {
            let _ = self.events.send(EngineEvent::LevelLowered {
                ns: ns.clone(),
                level,
            });
        }
    }

    // ---- publish and persist ----------------------------------------------------------------

    /// Publishes `changes` in one step and offers every namespace it put to the cache.
    fn publish(
        &mut self,
        changes: Vec<CatalogChange>,
        delta: Option<CatalogDelta>,
    ) -> Result<u64, IntroError> {
        let published = match delta {
            Some(d) => self.publisher.publish_with_delta(changes, Arc::new(d)),
            None => self.publisher.publish(changes),
        }
        .map_err(|_| IntroError::Protocol(Name::from("publish")))?;
        if self.cache.is_some() {
            let snap = self.catalog.current();
            let now = Instant::now().into_std();
            let mut removed = Vec::new();
            for ns in &published.changed {
                match snap.namespaces.get(ns) {
                    Some(arena) => self.debouncer.offer(arena.clone(), now),
                    None => {
                        self.debouncer.cancel(ns);
                        removed.push(ns.clone());
                    }
                }
            }
            if !removed.is_empty()
                && let Some(cache) = self.cache.clone()
            {
                drop(self.rt.spawn_blocking(move || {
                    let mut c = lock(&cache);
                    for ns in removed {
                        let _ = c.remove_namespace(&ns);
                    }
                }));
            }
        }
        Ok(published.generation)
    }

    /// Writes `due` on the blocking pool.
    fn save(&self, due: Vec<Arc<NamespaceSnapshot>>) -> Option<tokio::task::JoinHandle<()>> {
        let cache = self.cache.clone()?;
        if due.is_empty() {
            return None;
        }
        let events = self.events.clone();
        Some(self.rt.spawn_blocking(move || {
            let mut c = lock(&cache);
            for ns in due {
                if let Err(CacheError::Full { warn: true }) = c.save_namespace(&ns) {
                    let _ = events.send(EngineEvent::CacheWriteFailed);
                }
            }
        }))
    }

    // ## m4-t15-engine (validation tasks; Task 22 replaces the body: named lines)
    /// Periodic validation (R-M4-20): an incremental pass of every loaded namespace in scope,
    /// at periodic priority, unless the user's session is in a transaction on a
    /// single-connection profile (alg/02 step 6).
    fn validate_due(&mut self) {
        let Some(every) = self.validate_every else {
            self.next_validate = None;
            return;
        };
        self.next_validate = Some(Instant::now() + every);
        if self.ctx.meta.user_in_transaction() {
            return;
        }
        let snap = self.catalog.current();
        let loaded: Vec<NsKey> = snap
            .namespaces
            .keys()
            .filter(|ns| !self.out_of_scope(ns))
            .filter(|ns| {
                matches!(
                    snap.state(&ContainerKey::Namespace((*ns).clone())),
                    LoadState::Loaded { .. }
                )
            })
            .cloned()
            .collect();
        for ns in loaded {
            let level = self.level_of(&ns);
            let job = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Incremental,
                    level,
                },
                Priority::Periodic,
                Trigger::Background,
            );
            self.enqueue(job, true);
        }
    }
    // ## m4-t15-engine (validation tasks)
}

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}
