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
    Catalog, CatalogSnapshot, KindGroup, Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey,
    ObjectKind, Publisher, StateMark, Watermark,
};
use sql_dialect::DialectId;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::TaskProgress;
use super::filters::Filters;
use super::levels::{LevelPolicy, NsRole};
use super::work::{self, FactsRead, IdentityRead, Listing, Merged, Output, Spec, Work, WorkerCtx};
use super::{CacheHandle, EngineConfig, EngineEvent, EngineLimits, Phase, Priority, Progress};
use crate::ddl_effects::Target;
use crate::freshness::{NsVerdict, ValidationClock};
use crate::introspector::{IntroCaps, IntroError, Staleness};
use crate::pack::Mode;
use crate::scope::{CurrentCtx, NsInfo, ScopePattern, parse_scope};
use crate::ticket::TicketSender;
use hacivat_proto::DbmsKind;

/// A request from a handle.
#[derive(Debug)]
pub(crate) enum Request {
    Ready,
    OnConnect,
    /// Load a namespace (an expansion, a lookup that needed it).
    Load(NsKey),
    /// A lookup answered `Missing` there: check it for changes.
    // ## m4-t22-state (named exception): with the missing name (a state check by name).
    Probe(NsKey, Option<Name>),
    Children(ObjectKey),
    Elevate(ObjectKey, Level),
    Refresh(Target, Trigger),
    CheckState(Vec<ObjectKey>),
    // ## m4-t25-ddl-view (named exception): a state check that also reports its per-key
    // verdicts (the DDL view, surface 11).
    CheckStateVerdicts(Vec<ObjectKey>, VerdictSink),
    // ## m4-t25-ddl-view
    Confirm {
        ns: NsKey,
        accept: bool,
    },
    // ## m4-t15b-facets (named exception in Task 15's file)
    Facet(Vec<ObjectKey>, hacivat_meta::props::FacetKind),
}

// ## m4-t25-ddl-view (named exception)
/// Where a state check reports its per-key verdicts: the jobs of one request (one per
/// database) add theirs; the last one sends them all. A job that fails drops its share, so the
/// receiver then sees the sender dropped (the check could not decide).
#[derive(Debug, Clone)]
pub(crate) struct VerdictSink(Arc<Mutex<VerdictJoin>>);

#[derive(Debug)]
struct VerdictJoin {
    left: usize,
    got: Vec<(ObjectKey, Staleness)>,
    tx: Option<tokio::sync::oneshot::Sender<Vec<(ObjectKey, Staleness)>>>,
}

impl VerdictSink {
    pub(crate) fn new(tx: tokio::sync::oneshot::Sender<Vec<(ObjectKey, Staleness)>>) -> Self {
        VerdictSink(Arc::new(Mutex::new(VerdictJoin {
            left: 0,
            got: Vec::new(),
            tx: Some(tx),
        })))
    }

    fn expect(&self, jobs: usize) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).left = jobs;
    }

    fn add(&self, keys: &[ObjectKey], st: &[Staleness]) {
        let mut j = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        j.got.extend(keys.iter().cloned().zip(st.iter().copied()));
        j.left = j.left.saturating_sub(1);
        if j.left == 0
            && let Some(tx) = j.tx.take()
        {
            let got = std::mem::take(&mut j.got);
            let _ = tx.send(got);
        }
    }

    /// A job of the request failed: nobody gets a verdict (the receiver sees `Err`).
    fn fail(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).tx = None;
    }
}
// ## m4-t25-ddl-view

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
    SetFilters(Filters),
    LevelChanged(ContainerKey),
    Cancel(NsKey),
    Shutdown,
    // ## m4-t21-refresh (named exception)
    MarkStale {
        namespaces: Vec<NsKey>,
        scope: bool,
    },
    // ## m4-t27-refresh-ui (named exception: the overlays' user layers and live choices)
    SetUserData(Arc<hacivat_meta::userdata::ProfileUserData>),
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
    /// A re-planned confirmation (review I-3): the user's answer to the guard, `true` "Remove
    /// from cache", `false` "Keep cached".
    decision: Option<bool>,
    // ## m4-t22-state (named exception)
    /// A fragment's stamps read by the state check that asked for it (stored as marks).
    marks: Vec<(ObjectKey, StateMark)>,
    /// A state check by name for the negative cache's probe of this namespace (an
    /// undecidable answer falls back to an incremental pass of it).
    probe: Option<NsKey>,
    // ## m4-t25-ddl-view (named exception)
    /// Where a state check reports its per-key verdicts.
    verdicts: Option<VerdictSink>,
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
        // ## m4-t15b-facets (named exception): single flight per (object, facet).
        (
            Work::Facet {
                ns: a,
                keys: ka,
                kind: fa,
            },
            Work::Facet {
                ns: b,
                keys: kb,
                kind: fb,
            },
        ) => a == b && fa == fb && kb.iter().all(|k| ka.contains(k)),
        _ => false,
    }
}

/// Whether two queued tasks coalesce into one.
fn same_key(a: &Work, b: &Work) -> bool {
    match (a, b) {
        (Work::ScopeRefresh, Work::ScopeRefresh) => true,
        (Work::Pass { ns: x, .. }, Work::Pass { ns: y, .. })
        | (Work::Fragment { ns: x, .. }, Work::Fragment { ns: y, .. })
        | (Work::Validate { ns: x }, Work::Validate { ns: y }) => x == y,
        // ## m4-t15b-facets (named exception)
        // ## m4-t15b-facets (review I-2, T17b m-6): `Sources` reads are per key: one refused
        // key never fails another caller's.
        (
            Work::Facet {
                ns: x,
                kind: a,
                keys: ka,
            },
            Work::Facet {
                ns: y,
                kind: b,
                keys: kb,
            },
        ) => x == y && a == b && (*a != hacivat_meta::props::FacetKind::Sources || ka == kb),
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

// ## m4-t30-gates (named exception in Task 15's file; T26 carry, R-M4-6, PO-Q7)
/// Whether `facts` name a PostgreSQL, MySQL or MariaDB version that the attached snapshot set
/// has no snapshot for (exact selection by line, never the closest one): its system schemas
/// are then introspected live. Any non-empty version text counts, a release or not: an
/// unparsable one (`19devel`, `PostgreSQL 17`) has no line, so it reads live too, which is
/// truthful (live data, no layer). An empty version (not connected yet, a failed facts read),
/// no attached set, SQLite and the forks: never.
pub(crate) fn live_system_schemas(
    facts: &hacivat_meta::ServerFacts,
    set: Option<&Arc<hacivat_meta::system::SystemCatalogs>>,
) -> bool {
    matches!(
        facts.dbms,
        DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb
    ) && !facts.version.trim().is_empty()
        && set.is_some_and(|s| s.for_server(facts.dbms, &facts.version).is_none())
}

/// The listed system namespaces a snapshot would have covered (`minicat::system_schemas`) in
/// the databases `in_scope` reads (PostgreSQL; MySQL/MariaDB have one level). Names are
/// compared by the server's schema identity rule (review m-4).
fn live_system_keys<'a>(
    infos: &'a [NsInfo],
    in_scope: &BTreeSet<NsKey>,
    server: &hacivat_meta::ServerFacts,
) -> Vec<&'a NsInfo> {
    let ident = &server.ident;
    let key = |s: &str| {
        hacivat_meta::name::match_key(ident.dialect, ident, s, true, NameClass::Schema).ok()
    };
    let names: Vec<_> = crate::minicat::system_schemas(server.dbms)
        .iter()
        .filter_map(|n| key(n))
        .collect();
    let dbs: HashSet<&Name> = in_scope
        .iter()
        .filter(|k| k.kind == NsKind::Schema && k.path.len() == 2)
        .map(|k| &k.path[0])
        .collect();
    infos
        .iter()
        .filter(|i| i.system && i.key.kind == NsKind::Schema)
        .filter(|i| match i.key.path.as_slice() {
            [db, s] => dbs.contains(db) && key(s).is_some_and(|k| names.contains(&k)),
            [s] => key(s).is_some_and(|k| names.contains(&k)),
            _ => false,
        })
        .collect()
}

/// Review I-2 and m-1: a live system namespace is read by the system sections (`40-system`,
/// R-M4-44, plan Task 16) at `Names`, whatever an override of its database says; a fragment
/// there becomes that pass (a user-mode fragment would replace system objects by unflagged
/// ones).
fn live_system_work(work: &mut Work) {
    let ns = match work {
        Work::Pass { ns, .. } | Work::Fragment { ns, .. } => ns.clone(),
        _ => return,
    };
    *work = Work::Pass {
        ns,
        mode: Mode::System,
        level: Level::Names,
    };
}
// ## m4-t30-gates

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

/// What the engine shares with its handles.
pub(crate) struct Shared {
    pub progress: Arc<watch::Sender<Progress>>,
    pub events: broadcast::Sender<EngineEvent>,
    pub levels: Arc<Mutex<LevelPolicy>>,
    pub roles: Arc<Mutex<Roles>>,
    /// Cancelled when the actor has stopped (after the last cache write).
    pub stopped: CancellationToken,
    // ## m4-t22-state (named exception)
    /// The top-level objects each held mass deletion would remove (D-039; T24's dialog).
    pub pending: PendingNames,
}

// ## m4-t22-state (named exception)
/// What `IntroHandle::pending_confirmation` reads.
pub(crate) type PendingNames = Arc<Mutex<HashMap<NsKey, Vec<ObjectKey>>>>;

/// The current and the system namespaces, for `IntroHandle::level_of` (review m8).
#[derive(Debug, Default)]
pub(crate) struct Roles {
    pub current: Option<NsKey>,
    pub system: HashSet<NsKey>,
}

/// Starts the actor of `cfg` on its runtime.
pub(super) fn spawn(
    cfg: EngineConfig,
    rx: mpsc::UnboundedReceiver<Cmd>,
    shared: Shared,
    clock: ValidationClock,
) {
    let rt = cfg.runtime.clone();
    let stopped = shared.stopped.clone();
    let actor = Actor::new(cfg, shared, clock);
    rt.spawn(async move {
        actor.run(rx).await;
        stopped.cancel();
    });
}

/// The progress of the tasks since the engine was last idle (review m11).
#[derive(Debug, Default, Clone, Copy)]
struct Batch {
    done: u64,
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
    roles: Arc<Mutex<Roles>>,
    progress: Arc<watch::Sender<Progress>>,
    events: broadcast::Sender<EngineEvent>,
    scope: ScopePattern,
    filters: Filters,
    /// Other PostgreSQL databases the last scope refresh could not list (review RR-2): their
    /// schemas keep their states and are neither in scope nor `Missing`.
    unlisted: HashSet<Name>,
    /// `on_connect` ran: a late current namespace gets its step 3 (review m6).
    connected: bool,
    /// The server identity of the last connect, when there is no cache to compare with.
    last_identity: Option<u128>,
    /// When each namespace was last probed for a missing name (review m7).
    probed: HashMap<NsKey, Instant>,
    batch: Batch,
    /// The empty pattern (`@:@`): what selects the current namespace.
    current_only: Option<ScopePattern>,
    current: CurrentCtx,
    /// The last scope refresh's listing, the namespaces in scope, the current one, the system
    /// ones (`None` until the first scope refresh).
    listed: Option<Vec<NsInfo>>,
    in_scope: Option<BTreeSet<NsKey>>,
    current_ns: Option<NsKey>,
    system_ns: HashSet<NsKey>,
    // ## m4-t30-gates (named exception in Task 15's file; T26 carry, R-M4-6)
    /// The server's version has no shipped system-catalog snapshot (PostgreSQL 13, MySQL 9.x,
    /// …): its system schemas are introspected live at their role's level (`Names` by
    /// default) instead of being left out of scope.
    live_system: bool,
    // ## m4-t30-gates
    queue: Vec<Job>,
    running: HashMap<u64, Running>,
    deferred_confirms: Vec<(NsKey, bool, Waiter)>,
    pending: HashMap<NsKey, Pending>,
    timeouts: HashMap<NsKey, u8>,
    recent_children: HashMap<NsKey, Vec<(Instant, ObjectKey)>>,
    debouncer: SaveDebouncer,
    validate_every: Option<Duration>,
    // ## m4-t22-state (named lines: validation tasks): the next round on the validation clock.
    next_validate: Option<Duration>,
    clock: ValidationClock,
    /// The last connect's identity was unknown too (an unknown identity costs one full re-read
    /// per session, not one per connect; T16 review carry).
    last_unknown: bool,
    /// Per namespace: the top-level objects when background validation first removed some
    /// since the last user decision, and how many it removed since (D-039 across passes).
    bg_deleted: HashMap<NsKey, (u32, u32)>,
    /// Per namespace: what the user chose to keep cached ("Keep cached", D-039) since their last
    /// refresh: a background pass that would remove any of it asks again (fix round 1 I-4).
    kept: HashMap<NsKey, BTreeSet<ObjectKey>>,
    /// Cache writes in flight: the engine stops only after every one has landed (fix round 1,
    /// the ubuntu flake of `on_connect_with_cache_publishes_stale_then_runs_incremental_not_full`).
    saving: Vec<tokio::task::JoinHandle<()>>,
    pending_names: PendingNames,
    /// Namespaces whose arena is being rebuilt on the blocking pool (a narrower filter, a
    /// forgotten watermark, a new fold rule; T15 m-rr2, T17 fm3): no task of them starts
    /// until it is published, so none reads or merges against the arena being replaced.
    republishing: HashSet<NsKey>,
    bg_tx: mpsc::UnboundedSender<Republished>,
    bg_rx: mpsc::UnboundedReceiver<Republished>,
    root_cancel: CancellationToken,
    done_tx: mpsc::UnboundedSender<(u64, Output)>,
    done_rx: mpsc::UnboundedReceiver<(u64, Output)>,
    next_id: u64,
    // ## m4-t21-refresh (named exception, fix round 1 m5)
    /// Namespaces marked stale while a pass of them was loading: that pass started before the
    /// change, so its result is published stale.
    stale_on_publish: BTreeSet<NsKey>,
    // ## m4-t15b-facets (named exception; review m-2)
    /// Bumped whenever loaded facets may have gone stale outside a facet read's namespace (a
    /// role change, a pass that dropped facets elsewhere); a facet read that saw it move while
    /// it ran publishes its answer stale.
    facet_epoch: u64,
    /// The epoch each running facet read started at, by job id.
    facet_started: HashMap<u64, u64>,
    /// The generation that published a read at `Sources` of each object (a fragment) and of
    /// each namespace (a pass): a `Sources` request whose object was read so since its
    /// namespace last changed has nothing more to read (CI 44709954319: the DDL view's facet
    /// request after its elevation read the view twice).
    sources_read: HashMap<ObjectKey, u64>,
    sources_read_ns: HashMap<NsKey, u64>,
}

impl Actor {
    fn new(cfg: EngineConfig, shared: Shared, clock: ValidationClock) -> Actor {
        let dialect = cfg.scope.dialect();
        let caps = cfg.introspector.capabilities();
        let (done_tx, done_rx) = mpsc::unbounded_channel();
        // ## m4-t22-state (named exception)
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        let Shared {
            progress,
            events,
            levels,
            roles,
            stopped: _,
            pending: pending_names,
        } = shared;
        let ctx = Arc::new(WorkerCtx {
            meta: cfg.meta.clone(),
            intro: cfg.introspector.clone(),
            dbms: cfg.dbms,
            limits: cfg.limits,
            events: events.clone(),
            cache: cfg.cache.clone(),
            // ## m4-t17-mysql (named exception; T15 carry m11)
            progress: progress.clone(),
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
            roles,
            progress,
            events,
            current_only: parse_scope("", dialect).ok(),
            scope: cfg.scope,
            filters: cfg.filters,
            unlisted: HashSet::new(),
            connected: false,
            last_identity: None,
            probed: HashMap::new(),
            batch: Batch::default(),
            current: cfg.current,
            listed: None,
            in_scope: None,
            current_ns: None,
            system_ns: HashSet::new(),
            // ## m4-t30-gates
            live_system: false,
            // ## m4-t30-gates
            queue: Vec::new(),
            running: HashMap::new(),
            deferred_confirms: Vec::new(),
            pending: HashMap::new(),
            timeouts: HashMap::new(),
            recent_children: HashMap::new(),
            debouncer: SaveDebouncer::new(cfg.limits.save_debounce),
            // ## m4-t21-refresh (named exception, fix round 1 m5)
            stale_on_publish: BTreeSet::new(),
            // ## m4-t15b-facets (named exception; review m-2)
            facet_epoch: 0,
            facet_started: HashMap::new(),
            sources_read: HashMap::new(),
            sources_read_ns: HashMap::new(),
            validate_every: cfg.validate_every,
            // ## m4-t22-state (named lines: validation tasks)
            next_validate: cfg.validate_every.map(|d| clock.now() + d),
            clock,
            last_unknown: false,
            bg_deleted: HashMap::new(),
            kept: HashMap::new(),
            saving: Vec::new(),
            pending_names,
            republishing: HashSet::new(),
            bg_tx,
            bg_rx,
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
            // ## m4-t33a-progress (named exception in Task 15's file; T33 carry, the residual
            // race of `background_validation_never_deletes_in_small_steps`): progress is
            // published once per step, after `pump` started what the step queued. A task that
            // finished used to publish its removal first, so a validation whose pass was still
            // to start (or work freed by a republish) showed the engine idle for a moment.
            self.update_progress();
            // ## m4-t33a-progress
            let wake = self.next_wake();
            // ## m4-t22-state (named lines: validation tasks): the round's own clock.
            let (vclock, vnext) = (self.clock.clone(), self.next_validate);
            tokio::select! {
                biased;
                Some((id, out)) = self.done_rx.recv() => self.on_done(id, out),
                // ## m4-t22-state (named exception)
                Some(r) = self.bg_rx.recv() => self.republished(r),
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Shutdown) | None => break,
                    Some(cmd) => self.on_cmd(cmd),
                },
                () = sleep_until(wake) => self.on_timer(),
                () = validation_due(vclock, vnext) => self.validate_due(),
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
            self.saving.push(save);
        }
        // ## m4-t22-state (named exception; fix round 1): every cache write in flight lands
        // before the engine reports it stopped, so the next session's cache holds it.
        for h in std::mem::take(&mut self.saving) {
            let _ = h.await;
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
        queued.chain(save).min()
    }

    fn on_timer(&mut self) {
        let now = Instant::now();
        let due = self.debouncer.take_due(now.into_std());
        // ## m4-t22-state (named exception; fix round 1): kept, so `stop` waits for it.
        self.saving.retain(|h| !h.is_finished());
        if let Some(h) = self.save(due) {
            self.saving.push(h);
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
                let before = self.current_ns.clone();
                self.rescope();
                // A current namespace learned after `on_connect` gets its step 3 (review m6).
                if self.connected && self.current_ns != before {
                    self.connect_step(Vec::new(), Priority::User);
                }
            }
            Cmd::SetFilters(f) => self.set_filters(f),
            Cmd::LevelChanged(container) => self.level_changed(&container),
            Cmd::Cancel(ns) => self.cancel(&ns),
            Cmd::Shutdown => {}
            // ## m4-t21-refresh (named exception)
            Cmd::MarkStale { namespaces, scope } => self.mark_stale(namespaces, scope),
            // ## m4-t27-refresh-ui (named exception): the engine is the catalog's only writer.
            Cmd::SetUserData(data) => {
                let _ = self.publish(vec![CatalogChange::SetUserData(data)], None);
            }
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
            decision: None,
            // ## m4-t22-state (named exception)
            marks: Vec::new(),
            probe: None,
            // ## m4-t25-ddl-view (named exception)
            verdicts: None,
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
            // The user's answer to the guard rides on whatever it coalesces with; the latest
            // answer wins (review RR-1).
            if job.decision.is_some() {
                q.decision = job.decision;
            }
            // ## m4-t22-state (named exception): the stamps ride along.
            q.marks.append(&mut job.marks);
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
                // ## m4-t15b-facets (named exception): one read of the union of the keys.
                (Work::Facet { keys, .. }, Work::Facet { keys: k2, .. }) => {
                    for k in k2 {
                        if !keys.contains(&k) {
                            keys.push(k);
                        }
                    }
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
            // ## m4-t15b-facets (named exception): a facet read is a task of its namespace.
            Work::Pass { ns, .. }
            | Work::Fragment { ns, .. }
            | Work::Validate { ns }
            | Work::Facet { ns, .. } => !self.busy(ns) && !self.republishing.contains(ns),
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
        // ## m4-t30-gates (named exception in Task 15's file; T30 review I-2, m-1)
        if self.live_system && job.work.ns().is_some_and(|ns| self.system_ns.contains(ns)) {
            live_system_work(&mut job.work);
        }
        // ## m4-t30-gates
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
        // A re-planned confirmation carries the user's answer to the guard (review I-3).
        let policy = match job.decision {
            Some(true) => MergePolicy {
                trigger: Trigger::User,
                guard_min_objects: u32::MAX,
                ..MergePolicy::default()
            },
            Some(false) => MergePolicy {
                trigger: Trigger::User,
                deletions: Deletions::Suppress,
                ..MergePolicy::default()
            },
            None => MergePolicy {
                trigger: job.trigger,
                ..MergePolicy::default()
            },
        };
        // The fold rule from the server facts as they are now (review m4).
        let ident = &snap.server.ident;
        let fold =
            FoldRule::for_names(ident.dialect, ident, NameClass::Table).unwrap_or(FoldRule::Exact);
        let listing = (job.work == Work::ScopeRefresh).then(|| Listing {
            scope: self.scope.clone(),
            current: self.current.clone(),
            identity: job.then == Then::OnConnect,
        });
        // ## m4-t22-state (named exception): a validation of a namespace no longer cached has
        // nothing to compare; a state check compares with the catalog as it is now.
        if matches!(job.work, Work::Validate { .. }) && prev.is_none() {
            job.resolve(Ok(snap.generation));
            return;
        }
        let check = matches!(job.work, Work::CheckState { .. }).then(|| snap.clone());
        let marks = std::mem::take(&mut job.marks);
        let spec = Spec {
            work: job.work.clone(),
            prev,
            system,
            policy,
            filters: self.filters.clone(),
            fold,
            listing,
            snapshot: check,
            marks,
            // ## m4-t22-state (named exception; fix round 2, R-W4-9): a background read never
            // removes what the user kept.
            kept: match (job.trigger, job.work.ns()) {
                (Trigger::Background, Some(ns)) => self
                    .kept
                    .get(ns)
                    .map(|k| k.iter().cloned().collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            },
        };
        let cancel = self.root_cancel.child_token();
        let (id, ctx, done) = (job.id, self.ctx.clone(), self.done_tx.clone());
        let token = cancel.clone();
        self.rt.spawn(async move {
            let out = work::run(ctx, spec, token).await;
            let _ = done.send((id, out));
        });
        // ## m4-t15b-facets (named exception; review m-2)
        if matches!(job.work, Work::Facet { .. }) {
            self.facet_started.insert(id, self.facet_epoch);
        }
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

    /// The running tasks' progress. A pass or scope refresh counts the tasks of the current
    /// batch (finished of finished + running + queued, since the engine was last idle); a
    /// fragment or a state check counts its objects (review m11; a count inside one pass needs
    /// a progress sink in the introspector seam, carried to Tasks 16–18).
    fn update_progress(&mut self) {
        if self.running.is_empty() && self.queue.is_empty() {
            self.batch = Batch::default();
        }
        let done = self.batch.done;
        let total = done + (self.running.len() + self.queue.len()) as u64;
        let mut tasks: Vec<TaskProgress> = self
            .running
            .values()
            .map(|r| TaskProgress {
                ns: r.job.work.ns().cloned().unwrap_or_else(NsKey::server),
                phase: match &r.job.work {
                    Work::ScopeRefresh => Phase::ListNamespaces,
                    Work::Pass { .. } => Phase::Tables,
                    Work::Fragment { .. } => Phase::Columns,
                    Work::CheckState { .. } | Work::Validate { .. } => Phase::Validate,
                    // ## m4-t15b-facets (named exception)
                    Work::Facet { .. } => Phase::Facets,
                },
                done: match &r.job.work {
                    Work::Fragment { .. }
                    | Work::CheckState { .. }
                    | Work::Validate { .. }
                    | Work::Facet { .. } => 0,
                    _ => done,
                },
                total: match &r.job.work {
                    Work::Fragment { keys, .. }
                    | Work::CheckState { keys, .. }
                    | Work::Facet { keys, .. } => Some(keys.len() as u64),
                    _ => Some(total),
                },
            })
            .collect();
        // ## m4-t22-state (named exception): an arena rebuilt on the blocking pool is work too.
        tasks.extend(self.republishing.iter().map(|ns| TaskProgress {
            ns: ns.clone(),
            phase: Phase::Validate,
            done: 0,
            total: None,
        }));
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
            Request::Probe(ns, missing) => {
                // At most one probe per namespace per `probe_interval` (review m7).
                let now = Instant::now();
                if self
                    .probed
                    .get(&ns)
                    .is_some_and(|t| now.duration_since(*t) < self.limits.probe_interval)
                {
                    return w.resolve(Ok(self.catalog.current().generation));
                }
                self.probed.insert(ns.clone(), now);
                let loaded =
                    self.catalog.current().namespaces.contains_key(&ns) && !self.out_of_scope(&ns);
                // ## m4-t22-state (named exception): a state check by name first where stamps
                // are by name (MySQL, MariaDB); else (and when it cannot decide) a pass.
                if let Some(missing) = missing
                    && loaded
                    && matches!(self.caps.dbms, DbmsKind::MySql | DbmsKind::MariaDb)
                {
                    let keys = probe_keys(&ns, &missing);
                    let database = work::database_of(self.caps.dbms, Some(&ns));
                    let mut job = self.new_job(
                        Work::CheckState { keys, database },
                        prio,
                        Trigger::Background,
                    );
                    job.probe = Some(ns);
                    job.add(w, false);
                    self.queue.push(job);
                } else if loaded {
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
            // ## m4-t22-state (named exception): one check per database of the keys (the carry
            // "split CheckState by connection"; SQLite `temp` keys are decided without a stamp
            // and re-read on the user's connection).
            // ## m4-t25-ddl-view (named exception): the same check, reporting its verdicts.
            Request::CheckState(_) | Request::CheckStateVerdicts(..) => {
                let (keys, sink) = match req {
                    Request::CheckState(keys) => (keys, None),
                    Request::CheckStateVerdicts(keys, sink) => (keys, Some(sink)),
                    _ => unreachable!("matched above"),
                };
                let mut by_db: BTreeMap<Option<Name>, Vec<ObjectKey>> = BTreeMap::new();
                for k in keys {
                    let ns = self.ns_of(&k);
                    by_db
                        .entry(work::database_of(self.caps.dbms, Some(&ns)))
                        .or_default()
                        .push(k);
                }
                if let Some(sink) = &sink {
                    sink.expect(by_db.len());
                    if by_db.is_empty() {
                        sink.add(&[], &[]);
                    }
                }
                if by_db.is_empty() {
                    return w.resolve(Ok(self.catalog.current().generation));
                }
                let join = Arc::new(Mutex::new(Join {
                    left: by_db.len(),
                    generation: 0,
                    error: None,
                    waiters: vec![w],
                }));
                for (database, keys) in by_db {
                    let mut job = self.new_job(
                        Work::CheckState { keys, database },
                        prio,
                        Trigger::Background,
                    );
                    job.add(Waiter::Join(join.clone()), false);
                    // ## m4-t25-ddl-view (named exception)
                    job.verdicts = sink.clone();
                    self.queue.push(job);
                }
            }
            Request::Confirm { ns, accept } => self.deferred_confirms.push((ns, accept, w)),
            // ## m4-t15b-facets (named exception)
            Request::Facet(keys, kind) => self.facet_request(keys, kind, prio, w),
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
            // ## m4-t21-refresh (named exception): the refresh-after-DDL planner's targets,
            // lowered on the current catalog (`ddl_effects::lower`) into the three above; the
            // request resolves when every part has.
            t @ (Target::Object(_) | Target::NamespaceNames(_) | Target::Database(_)) => {
                let parts = crate::ddl_effects::lower(&[t], &self.catalog.current());
                if parts.is_empty() {
                    return w.resolve(Ok(self.catalog.current().generation));
                }
                let join = Arc::new(Mutex::new(Join {
                    left: parts.len(),
                    generation: 0,
                    error: None,
                    waiters: vec![w],
                }));
                for part in parts {
                    self.refresh(part, trigger, prio, Waiter::Join(join.clone()));
                }
            }
        }
    }

    // ## m4-t21-refresh (named exception)
    /// Marks the loaded namespaces `namespaces` stale (refresh after DDL with the preference
    /// `never` or `ask`, alg/02 HV-INTRO-12 edge 14): nothing is read; their next expansion or
    /// pass validates them. A namespace that is loading is published stale when its pass ends
    /// (fix round 1 m5). `scope`: the namespace list changed too, so every loaded schema list
    /// is marked stale (fix round 1 I4).
    fn mark_stale(&mut self, namespaces: Vec<NsKey>, scope: bool) {
        let snap = self.catalog.current();
        let stale = |key: ContainerKey, state: &LoadState| match state.clone() {
            LoadState::Loaded {
                level,
                at,
                watermark,
                stale: false,
            } => Some(CatalogChange::SetState(
                key,
                LoadState::Loaded {
                    level,
                    at,
                    watermark,
                    stale: true,
                },
            )),
            _ => None,
        };
        let mut changes: Vec<CatalogChange> = Vec::new();
        for ns in namespaces {
            let key = ContainerKey::Namespace(ns.clone());
            let state = snap.state(&key);
            // A pass of it under way (a first load, or a re-read of a loaded namespace) began
            // before the change: its result is published stale too.
            let running = self.running.values().any(|r| r.job.work.ns() == Some(&ns));
            if running || matches!(state, LoadState::Loading { .. }) {
                self.stale_on_publish.insert(ns.clone());
            }
            if let Some(c) = stale(key, state) {
                changes.push(c);
            }
        }
        if scope {
            for (key, state) in snap.states.iter() {
                if matches!(key, ContainerKey::SchemaList { .. })
                    && let Some(c) = stale(key.clone(), state)
                {
                    changes.push(c);
                }
            }
        }
        if !changes.is_empty() {
            let _ = self.publish(changes, None);
        }
    }

    /// Stops and drops the work tied to `ns`: its passes and fragments, and every state check
    /// with a key in it (review m10). Nothing of it is published.
    fn cancel(&mut self, ns: &NsKey) {
        let tied = |this: &Self, w: &Work| match w {
            Work::CheckState { keys, .. } => keys.iter().any(|k| this.ns_of(k) == *ns),
            _ => w.ns() == Some(ns),
        };
        for r in self.running.values() {
            if tied(self, &r.job.work) {
                r.cancel.cancel();
            }
        }
        let queue = std::mem::take(&mut self.queue);
        let (gone, keep): (Vec<Job>, Vec<Job>) =
            queue.into_iter().partition(|j| tied(self, &j.work));
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
        {
            // The user chose a level: back-pressure starts over (review m8).
            let mut levels = lock(&self.levels);
            for ns in snap.namespaces.keys().filter(|ns| below(ns)) {
                levels.reset_lowering(ns);
            }
        }
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
        if self.publish(changes, None).is_err() {
            let _ = self.events.send(EngineEvent::PublishFailed);
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

    /// The filters changed (R-W3-2). What the new scope filter no longer keeps is removed from
    /// every loaded namespace in one publish: a user scope change, so never guarded, and the
    /// delta lists it in `out_of_scope` (not `dropped`). A filter that may keep more re-reads
    /// every loaded namespace in scope.
    fn set_filters(&mut self, filters: Filters) {
        let before = std::mem::replace(&mut self.filters, filters);
        if before == self.filters {
            return;
        }
        let widen = self.filters.may_widen(&before);
        let snap = self.catalog.current();
        // ## m4-t22-state (named exception; T15 m-rr2): the narrowing merges run on the
        // blocking pool; the namespaces wait for them (`republishing`).
        let arenas: Vec<Rebuild> = snap
            .namespaces
            .iter()
            .map(|(k, a)| (k.clone(), a.clone(), self.system_ns.contains(k)))
            .collect();
        self.republish(arenas, BgOp::Narrow(self.filters.clone()));
        if widen {
            let loaded: Vec<NsKey> = snap
                .namespaces
                .keys()
                .filter(|ns| !self.out_of_scope(ns))
                .cloned()
                .collect();
            for ns in loaded {
                let level = self.level_of(&ns);
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
    }

    /// The changes of the last listing under the current scope: listed namespaces in scope get
    /// a state (`NotLoaded`) so a lookup knows they exist; out-of-scope ones are removed and
    /// marked `OutOfScope`; namespaces the server no longer lists are removed; the schema lists
    /// are loaded.
    fn scope_changes(&mut self) -> Vec<CatalogChange> {
        let Some(infos) = self.listed.clone() else {
            return Vec::new();
        };
        let mut in_scope = self.scope.select(&infos, &self.current);
        // ## m4-t30-gates (named exception in Task 15's file; T26 carry, R-M4-6): without a
        // shipped snapshot for the server's version, the snapshot's system schemas of every
        // database the scope reads are in scope too, read live like any other namespace.
        if self.live_system {
            let server = self.catalog.current().server.clone();
            let extra = live_system_keys(&infos, &in_scope, &server);
            // Review I-1: the user's `!` clauses still remove them.
            in_scope.extend(
                extra
                    .into_iter()
                    .filter(|i| !self.scope.excludes(i, &self.current))
                    .map(|i| i.key.clone()),
            );
        }
        // ## m4-t30-gates
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
        *lock(&self.roles) = Roles {
            current: self.current_ns.clone(),
            system: self.system_ns.clone(),
        };
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
        // PostgreSQL databases whose schemas this listing read (the others were not asked).
        let read_dbs: HashSet<&Name> = infos
            .iter()
            .filter(|i| i.key.kind == NsKind::Schema && i.key.path.len() == 2)
            .map(|i| &i.key.path[0])
            .collect();
        for k in known {
            if listed.contains(&k) {
                continue;
            }
            // A database that could not be listed is unknown: nothing of it changes.
            if two_level(self.dialect)
                && k.path.first().is_some_and(|db| self.unlisted.contains(db))
            {
                continue;
            }
            let unread_db = two_level(self.dialect)
                && k.kind == NsKind::Schema
                && k.path.first().is_some_and(|db| !read_dbs.contains(db));
            changes.push(CatalogChange::Remove(k.clone()));
            if unread_db {
                // A schema of a database the scope no longer selects: out of scope, not dropped.
                changes.push(CatalogChange::SetState(
                    ContainerKey::Namespace(k),
                    LoadState::OutOfScope,
                ));
            } else if snap.namespaces.contains_key(&k) {
                // Only a namespace that was loaded is announced (an unknown name someone
                // expanded never existed; review m12).
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
        let Some(Running { job, cancel }) = self.running.remove(&id) else {
            return;
        };
        self.batch.done += 1;
        // ## m4-t33a-progress (named exception): no progress here; the loop publishes it after
        // the follow-ups this answer queues have started.
        // A task cancelled after its read finished publishes nothing (review m2).
        let out = if cancel.is_cancelled() {
            Output::Failed(IntroError::Cancelled)
        } else {
            out
        };
        // ## m4-t15b-facets (named exception): a facet read publishes only its facets.
        if let Work::Facet { .. } = &job.work {
            return match out {
                Output::Facet(rows) => self.facet_done(job, &rows),
                Output::Failed(e) => self.facet_failed(job, e),
                _ => self.facet_failed(job, IntroError::Protocol(Name::from("facet"))),
            };
        }
        match out {
            Output::Namespaces(infos, identity, unlisted, facts) => {
                self.listed_done(job, infos, identity, unlisted, facts)
            }
            Output::Merged(m) => self.merged(job, *m),
            Output::Staleness(keys, st, marks) => self.staleness(job, keys, st, marks),
            // ## m4-t22-state (named exception)
            Output::Validated(ns, ptr, verdict) => self.validated(job, &ns, ptr, verdict),
            // ## m4-t15b-facets (named exception): only a facet task answers rows.
            Output::Facet(_) => job.resolve(Err(IntroError::Protocol(Name::from("facet")))),
            Output::Failed(e) => self.failed(job, e),
        }
    }

    fn listed_done(
        &mut self,
        job: Job,
        infos: Vec<NsInfo>,
        identity: Option<IdentityRead>,
        unlisted: Vec<Name>,
        facts: FactsRead,
    ) {
        for db in &unlisted {
            let _ = self
                .events
                .send(EngineEvent::DatabaseUnlisted { db: db.clone() });
        }
        self.unlisted = unlisted.into_iter().collect();
        self.listed = Some(infos);
        // ## m4-t17-mysql (named exception; T15 carry m4, review I-2): the real server facts
        // replace the catalog's placeholder in the same publish, so every later task's fold
        // rule (read per task from the facts) follows the server's `lower_case_table_names`.
        // A failed read publishes the current facts with an unknown `lower_case_table_names`
        // (no table rule is pinned: a lookup answers `ServerDependent`, never a wrong
        // `Missing`). A fact that changes the table fold rule re-folds every cached arena.
        let cur = self.catalog.current();
        // ## m4-t22-state (named exception; T17 fm3)
        let mut refolds: Option<(Vec<Rebuild>, FoldRule)> = None;
        let next = match facts {
            FactsRead::Read(f) => Some(*f),
            FactsRead::Failed => {
                let mut f = (*cur.server).clone();
                f.ident.lower_case_table_names = u8::MAX;
                // ## m4-t26-overlays (named exception in Task 15's file; review m-1): the
                // server behind the profile may have changed (an upgrade, a host edit), so the
                // version that chose the system layer and the initdb-only fact are unknown
                // too; the layer drops and system lookups answer `Unknown`.
                f.version = hacivat_meta::Name::from("");
                f.version_num = 0;
                f.system_schema_extras = None;
                // ## m4-t26-overlays
                Some(f)
            }
            FactsRead::NotRead => None,
        };
        // ## m4-t30-gates (named exception in Task 15's file; T26 carry, R-M4-6): the scope is
        // computed after the facts so a version without a shipped snapshot brings its system
        // schemas into scope in this same publish.
        self.live_system =
            live_system_schemas(next.as_ref().unwrap_or(&cur.server), self.catalog.system());
        let mut changes = self.scope_changes();
        // ## m4-t30-gates
        if let Some(f) = next
            && *cur.server != f
        {
            let fold = FoldRule::for_names(f.ident.dialect, &f.ident, NameClass::Table)
                .unwrap_or(FoldRule::Exact);
            changes.insert(0, CatalogChange::SetServer(Arc::new(f)));
            // ## m4-t22-state (named exception; T17 fm3): the arenas are rebuilt under the new
            // rule on the blocking pool (O(cached objects)); until then their lookups answer
            // `ServerDependent` (fail-closed) and their tasks wait.
            let arenas: Vec<Rebuild> = cur
                .namespaces
                .iter()
                .filter(|(_, a)| a.fold() != fold)
                .map(|(k, a)| (k.clone(), a.clone(), false))
                .collect();
            refolds = Some((arenas, fold));
        }
        let mut generation = match self.publish(changes, None) {
            Ok(g) => g,
            Err(e) => return job.resolve(Err(e)),
        };
        // ## m4-t22-state (named exception; T17 fm3)
        if let Some((arenas, fold)) = refolds {
            self.republish(arenas, BgOp::Refold(fold));
        }
        if job.then != Then::OnConnect {
            return job.resolve(Ok(generation));
        }
        self.connected = true;
        // R-W3-3: another (or an unknown) server behind the profile: no cached watermark is
        // trusted, so the passes that follow are full.
        if let Some(identity) = identity
            && !self.same_server(identity)
        {
            match self.forget_watermarks() {
                Ok(Some(g)) => generation = g,
                Ok(None) => {}
                Err(e) => return job.resolve(Err(e)),
            }
        }
        // On connect, steps 2–3: the current namespace (others load on first expansion).
        let (now, later): (Vec<_>, Vec<_>) = job.waiters.into_iter().partition(|(_, t)| !*t);
        resolve_all(now.into_iter().map(|(w, _)| w).collect(), Ok(generation));
        let waiters: Vec<Waiter> = later.into_iter().map(|(w, _)| w).collect();
        self.connect_step(waiters, job.prio);
        // ## m4-t22-state (named lines: validation tasks): background validation of every
        // cached namespace in scope but the current one (which has its own step 3).
        let snap = self.catalog.current();
        let cached: Vec<NsKey> = snap
            .namespaces
            .keys()
            .filter(|ns| Some(*ns) != self.current_ns.as_ref() && !self.out_of_scope(ns))
            .cloned()
            .collect();
        for ns in cached {
            self.validate(ns, Priority::Background);
        }
    }

    /// Whether the server behind the connection is the one the cache (or the last connect)
    /// was filled from; an unknown identity is not (fail-closed, R-W3-3).
    fn same_server(&mut self, identity: IdentityRead) -> bool {
        // ## m4-t22-state (named exception; T16 review carry): an unknown identity is cached
        // per session. The first unknown one forgets every watermark (the cache's may be
        // another server's); an unknown one right after it keeps them: they come from this
        // session's own passes on the same login. A known identity after an unknown one is
        // never taken for the same server.
        let was_unknown =
            std::mem::replace(&mut self.last_unknown, identity == IdentityRead::Unknown);
        match identity {
            IdentityRead::Unknown => {
                self.last_identity = None;
                was_unknown
            }
            IdentityRead::Known {
                same_as_cache: Some(same),
                fp,
            } => {
                self.last_identity = Some(fp);
                same && !was_unknown
            }
            IdentityRead::Known {
                fp,
                same_as_cache: None,
            } => self.last_identity.replace(fp).is_none_or(|last| last == fp) && !was_unknown,
        }
    }

    /// Re-publishes every namespace that has a watermark without it (stale), so its next pass
    /// is full (R-W3-3). Answers the generation, when anything changed.
    fn forget_watermarks(&mut self) -> Result<Option<u64>, IntroError> {
        // ## m4-t22-state (named exception; T15 m-rr2): the merges run on the blocking pool;
        // no task of a namespace starts before its watermark is gone (`republishing`), so the
        // passes on-connect queues next are full.
        let snap = self.catalog.current();
        let arenas: Vec<Rebuild> = snap
            .namespaces
            .iter()
            .filter(|(_, a)| a.meta().watermark != Watermark::None)
            .map(|(k, a)| (k.clone(), a.clone(), false))
            .collect();
        self.republish(arenas, BgOp::Forget);
        Ok(None)
    }

    /// On-connect step 3 for the current namespace: `Incremental` if cached, else `Brief`
    /// then the auto level. `waiters` wait for it (none when the current namespace arrived
    /// late, review m6).
    fn connect_step(&mut self, waiters: Vec<Waiter>, prio: Priority) {
        let Some(ns) = self.current_ns.clone().filter(|ns| !self.out_of_scope(ns)) else {
            let generation = self.catalog.current().generation;
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
                prio,
                Trigger::Background,
            )
        } else {
            let mut j = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Brief,
                    level: Level::Names,
                },
                prio,
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
        // Published by something else since the task started, or read under filters that
        // changed since (review m-rr1): read again.
        if now_ptr != m.prev_ptr || m.filters != self.filters {
            // Published by something else since the task started: read again.
            self.next_id += 1;
            job.seq = self.next_id;
            job.not_before = None;
            self.queue.push(job);
            return;
        }
        // ## m4-t22-state (named exception; T17 fm1): a first arena planned under the fold rule
        // of facts a scope refresh replaced since is merged again under the current rule.
        let ident = &snap.server.ident;
        let fold =
            FoldRule::for_names(ident.dialect, ident, NameClass::Table).unwrap_or(FoldRule::Exact);
        if m.prev_ptr.is_none() && m.outcome.next().fold() != fold {
            self.next_id += 1;
            job.seq = self.next_id;
            job.not_before = None;
            self.queue.push(job);
            return;
        }
        if m.outcome.is_pending() {
            return self.hold_pending(job, m);
        }
        // ## m4-t22-state (named exception; T10 carry M-1, fix round 1 I-4): no background
        // pass (validation, on-connect step 3, a probe's fallback, refresh after DDL) removes a
        // namespace's objects in many small steps around the guard, nor removes what the user
        // chose to keep.
        if job.trigger == Trigger::Background
            && job.decision.is_none()
            && m.pass
            && self.over_cumulative_guard(&ns, &m, &snap)
        {
            return self.hold_pending(job, m);
        }
        // A user refresh (not the pass a confirmation re-planned) starts over.
        if job.trigger == Trigger::User && job.decision.is_none() {
            self.bg_deleted.remove(&ns);
            self.kept.remove(&ns);
        }
        let (level, pass) = (m.level, m.pass);
        let (follow, follow_ns) = (m.follow, m.follow_ns);
        // ## m4-t15b-facets (named exception)
        let (reread, texts) = (m.reread, m.texts);
        let (next, delta, report) = m.outcome.into_parts();
        // ## m4-t22-state (named exception): what the delta means beyond this namespace.
        let accounts = (ns.kind == NsKind::Server && accounts_changed(&delta))
            || privileges_moved(snap.namespaces.get(&ns).map(|a| &a.meta().watermark), &next);
        let touched = if matches!(self.caps.dbms, DbmsKind::MySql | DbmsKind::MariaDb) {
            touched_top(&ns, &delta)
        } else {
            BTreeSet::new()
        };
        let mut changes = self.applied_changes(&ns, next, &report, level, pass);
        // ## m4-t15b-facets (named exception): the facets the read made stale go with it, and
        // the definition texts it read come with it.
        let dropped = super::facet::invalidations(&snap, &ns, &reread, &delta);
        if !dropped.is_empty() {
            self.facet_epoch += 1;
        }
        changes.extend(dropped);
        if !texts.is_empty() {
            changes.push(CatalogChange::PutSources {
                ns: ns.clone(),
                texts,
            });
        }
        let generation = match self.publish(changes, Some(delta)) {
            Ok(g) => g,
            Err(e) => return job.resolve(Err(e)),
        };
        // ## m4-t15b-facets (named exception): what this publish read at `Sources`.
        if level >= Level::Sources {
            match &job.work {
                Work::Pass { .. } => {
                    self.sources_read_ns.insert(ns.clone(), generation);
                }
                Work::Fragment { keys, .. } => {
                    for k in keys {
                        self.sources_read.insert(k.clone(), generation);
                    }
                }
                _ => {}
            }
        }
        // ## m4-t22-state (named exception; T16b review carry): a renamed or dropped role
        // leaves loaded grantee names stale.
        let generation = if accounts {
            self.stale_grants().unwrap_or(generation)
        } else {
            generation
        };
        // ## m4-t22-state (named exception; T17 review m5): the foreign keys of other cached
        // MySQL databases that reference a changed table are read again.
        if !job.follow_up && !touched.is_empty() {
            self.reread_referrers(&ns, &touched);
        }
        self.timeouts.remove(&ns);
        // A pass of the namespace replaces a held merge (review I-3): the prompt is withdrawn.
        // A fragment keeps it: the user's answer then re-plans a pass (`confirm`).
        if pass && self.pending.remove(&ns).is_some() && job.decision.is_none() {
            let _ = self
                .events
                .send(EngineEvent::PendingWithdrawn { ns: ns.clone() });
        }
        // ## m4-t22-state (named exception)
        self.sync_pending_names();
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
        // ## m4-t22-state (named exception): a pass held by the cumulative guard is an applied
        // merge; it would remove its delta's top-level drops.
        let vanished = match &m.outcome {
            MergeOutcome::PendingConfirmation { vanished, .. } => *vanished,
            MergeOutcome::Applied { .. } => {
                u32::try_from(top_drops(&ns, m.outcome.delta()).len()).unwrap_or(u32::MAX)
            }
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
        self.sync_pending_names();
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
                self.confirm(&ns, accept, w);
            }
        }
    }

    /// "Remove from cache" publishes the held merge; "Keep cached" re-merges the pass with its
    /// deletions suppressed (I-5). When the namespace changed since (a fragment published),
    /// the answer re-plans a full pass that carries it (review I-3); the ticket resolves with
    /// that pass. Nothing held: `Cancelled` (nothing was applied).
    fn confirm(&mut self, ns: &NsKey, accept: bool, w: Waiter) {
        let snap = self.catalog.current();
        let Some(p) = self.pending.remove(ns) else {
            return w.resolve(Err(IntroError::Cancelled));
        };
        // ## m4-t22-state (named exception; fix round 2, R-W4-9): the user decided; the count
        // starts over, and what they kept stays cached until their next refresh: background
        // reads report it existent (no hold, no new prompt), and only drops outside it count.
        self.sync_pending_names();
        self.bg_deleted.remove(ns);
        if !accept {
            self.kept
                .entry(ns.clone())
                .or_default()
                .extend(top_drops(ns, p.outcome.delta()));
        }
        let prev = snap.namespaces.get(ns).cloned();
        if prev.as_ref().map(|a| Arc::as_ptr(a) as usize) != p.prev_ptr
            || (!accept && p.keep.is_none())
        {
            let level = p.level.max(self.state_level(ns));
            let mut job = self.new_job(
                Work::Pass {
                    ns: ns.clone(),
                    mode: Mode::Full,
                    level,
                },
                Priority::User,
                Trigger::User,
            );
            job.decision = Some(accept);
            job.add(w, false);
            return self.enqueue(job, true);
        }
        let r = self.apply_confirmed(ns, accept, p, prev);
        w.resolve(r);
    }

    fn apply_confirmed(
        &mut self,
        ns: &NsKey,
        accept: bool,
        p: Pending,
        prev: Option<Arc<NamespaceSnapshot>>,
    ) -> Result<u64, IntroError> {
        let outcome = if accept {
            p.outcome
        } else {
            let (Some(r), Some(prev)) = (p.keep, prev) else {
                return Err(IntroError::Cancelled);
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
    fn staleness(
        &mut self,
        mut job: Job,
        keys: Vec<ObjectKey>,
        st: Vec<Staleness>,
        marks: Vec<(ObjectKey, StateMark)>,
    ) {
        // ## m4-t25-ddl-view (named exception): the verdicts, before any re-read.
        if let Some(sink) = job.verdicts.take() {
            sink.add(&keys, &st);
        }
        // ## m4-t22-state (named exception): a probe by name that could not decide falls back
        // to an incremental pass of its namespace (the probe before Task 22).
        if let Some(ns) = job.probe.take()
            && !st.contains(&Staleness::Changed)
            && st.contains(&Staleness::Unknown)
        {
            let level = self.level_of(&ns);
            let mut pass = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Incremental,
                    level,
                },
                job.prio,
                Trigger::Background,
            );
            pass.waiters = job.waiters;
            return self.enqueue(pass, false);
        }
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
            let mut f = self.fragment_job(ns.clone(), keys, level, job.prio, Trigger::Background);
            // ## m4-t22-state (named exception): the stamps of this namespace's owners.
            f.marks = marks
                .iter()
                .filter(|(k, _)| self.ns_of(k) == ns)
                .cloned()
                .collect();
            f.add(Waiter::Join(join.clone()), false);
            self.enqueue(f, true);
        }
    }

    fn failed(&mut self, mut job: Job, e: IntroError) {
        // ## m4-t25-ddl-view (named exception): a failed check decides nothing.
        if let Some(sink) = job.verdicts.take() {
            sink.fail();
        }
        // ## m4-t22-state (named exception; fix round 1 I-5): a validation that cannot decide is
        // never dropped silently: the namespace is read again (the pass reports its own error).
        if let Work::Validate { ns } = &job.work
            && !matches!(e, IntroError::Cancelled)
            && !self.out_of_scope(ns)
        {
            let ns = ns.clone();
            let level = self.level_of(&ns);
            let mut pass = self.new_job(
                Work::Pass {
                    ns,
                    mode: Mode::Incremental,
                    level,
                },
                job.prio,
                Trigger::Background,
            );
            pass.waiters = std::mem::take(&mut job.waiters);
            return self.enqueue(pass, true);
        }
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
        // ## m4-t21-refresh (named exception, fix round 1 m5): a pass that was loading when
        // its namespace was marked stale publishes stale.
        let changes = if self.stale_on_publish.is_empty() {
            changes
        } else {
            changes
                .into_iter()
                .map(|c| match c {
                    CatalogChange::SetState(
                        ContainerKey::Namespace(ns),
                        LoadState::Loaded {
                            level,
                            at,
                            watermark,
                            stale: false,
                        },
                    ) if self.stale_on_publish.remove(&ns) => CatalogChange::SetState(
                        ContainerKey::Namespace(ns),
                        LoadState::Loaded {
                            level,
                            at,
                            watermark,
                            stale: true,
                        },
                    ),
                    c => c,
                })
                .collect()
        };
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
                let h = self.rt.spawn_blocking(move || {
                    let mut c = lock(&cache);
                    for ns in removed {
                        let _ = c.remove_namespace(&ns);
                    }
                });
                // ## m4-t22-state (named exception; fix round 1): awaited by `stop`.
                self.saving.push(h);
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
    // ## m4-t22-state (named lines: validation tasks)
    /// Periodic validation (R-M4-20): the cheap check of every loaded namespace in scope, at
    /// periodic priority. While the user's session holds a transaction on a single-connection
    /// profile the round is deferred, and asked again every `busy_retry` (alg/02 step 6), so it
    /// runs as soon as the transaction is over.
    fn validate_due(&mut self) {
        let Some(every) = self.validate_every else {
            self.next_validate = None;
            return;
        };
        let now = self.clock.now();
        if self.ctx.meta.user_in_transaction() {
            self.next_validate = Some(now + self.limits.busy_retry.max(Duration::from_millis(1)));
            return;
        }
        self.next_validate = Some(now + every);
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
            self.validate(ns, Priority::Periodic);
        }
    }

    /// Queues the cheap validation of the cached namespace `ns`, unless a task of it is
    /// already queued or running (that task reads it anyway).
    fn validate(&mut self, ns: NsKey, prio: Priority) {
        if self.busy(&ns)
            || self.pending.contains_key(&ns)
            || self.queue.iter().any(|j| j.work.ns() == Some(&ns))
        {
            return;
        }
        let job = self.new_job(Work::Validate { ns }, prio, Trigger::Background);
        self.enqueue(job, true);
    }

    /// A validation answered: an unchanged namespace is cleared of its stale mark (nothing
    /// else is read); a changed one is read again, incrementally or in full. An arena published
    /// since the validation started was decided by that publish: nothing to do.
    fn validated(&mut self, job: Job, ns: &NsKey, ptr: Option<usize>, verdict: NsVerdict) {
        let snap = self.catalog.current();
        let now_ptr = snap.namespaces.get(ns).map(|a| Arc::as_ptr(a) as usize);
        if now_ptr != ptr || ptr.is_none() || self.out_of_scope(ns) {
            return job.resolve(Ok(snap.generation));
        }
        match verdict {
            NsVerdict::Current => {
                let key = ContainerKey::Namespace(ns.clone());
                let LoadState::Loaded {
                    level,
                    at,
                    watermark,
                    stale: true,
                } = snap.state(&key).clone()
                else {
                    return job.resolve(Ok(snap.generation));
                };
                if self.pending.contains_key(ns) {
                    return job.resolve(Ok(snap.generation));
                }
                let r = self.publish(
                    vec![CatalogChange::SetState(
                        key,
                        LoadState::Loaded {
                            level,
                            at,
                            watermark,
                            stale: false,
                        },
                    )],
                    None,
                );
                job.resolve(r);
            }
            NsVerdict::Changed { full } => {
                let level = self.level_of(ns);
                let mut pass = self.new_job(
                    Work::Pass {
                        ns: ns.clone(),
                        mode: if full { Mode::Full } else { Mode::Incremental },
                        level,
                    },
                    job.prio,
                    Trigger::Background,
                );
                pass.waiters = job.waiters;
                self.enqueue(pass, true);
            }
        }
    }

    /// Whether the applied background pass `m` of `ns` would bring the top-level objects
    /// background passes removed since the user last decided over the guard (kept objects are
    /// never among them: background reads report them existent, R-W4-9) (D-039: more than `guard_ratio_pct` % of at least `guard_min_objects`). Counts
    /// it when it does not.
    fn over_cumulative_guard(&mut self, ns: &NsKey, m: &Merged, snap: &CatalogSnapshot) -> bool {
        let drops = top_drops(ns, m.outcome.delta());
        let dropped = u32::try_from(drops.len()).unwrap_or(u32::MAX);
        if dropped == 0 {
            return false;
        }
        let policy = MergePolicy::default();
        let before = snap
            .namespaces
            .get(ns)
            .map_or(0, |a| a.root().child_count());
        let (base, gone) = *self.bg_deleted.entry(ns.clone()).or_insert((before, 0));
        let total = gone.saturating_add(dropped);
        if base >= policy.guard_min_objects
            && u64::from(total) * 100 > u64::from(base) * u64::from(policy.guard_ratio_pct)
        {
            return true;
        }
        self.bg_deleted.insert(ns.clone(), (base, total));
        false
    }

    /// Marks every loaded grants facet stale (a role was renamed or dropped: the grantee names
    /// they show may be gone). Answers the generation, when anything was marked.
    fn stale_grants(&mut self) -> Option<u64> {
        // ## m4-t15b-facets (named exception; review m-2): a grants read running now read the
        // old names too.
        self.facet_epoch += 1;
        let snap = self.catalog.current();
        let changes: Vec<CatalogChange> = snap
            .states
            .iter()
            .filter_map(|(k, st)| match (k, st) {
                (
                    ContainerKey::Facet {
                        facet: hacivat_meta::props::FacetKind::Grants,
                        ..
                    },
                    LoadState::Loaded {
                        level,
                        at,
                        watermark,
                        stale: false,
                    },
                ) => Some(CatalogChange::SetState(
                    k.clone(),
                    LoadState::Loaded {
                        level: *level,
                        at: *at,
                        watermark: watermark.clone(),
                        stale: true,
                    },
                )),
                _ => None,
            })
            .collect();
        if changes.is_empty() {
            return None;
        }
        self.publish(changes, None).ok()
    }

    /// Re-reads, in every other cached namespace, the tables whose foreign keys target one of
    /// `touched` (top-level names of `ns`). A follow-up: it schedules none of its own.
    fn reread_referrers(&mut self, ns: &NsKey, touched: &BTreeSet<Name>) {
        let snap = self.catalog.current();
        let mut by_ns: BTreeMap<NsKey, Vec<ObjectKey>> = BTreeMap::new();
        for (other, arena) in snap.namespaces.iter() {
            if other == ns || self.out_of_scope(other) {
                continue;
            }
            for (_, o) in arena.iter_named(&[ObjectKind::ForeignKey]) {
                let hacivat_meta::props::Props::ForeignKey(fk) = o.props() else {
                    continue;
                };
                let p = &fk.target.path;
                if p.len() != ns.path.len() + 1
                    || !p.starts_with(&ns.path)
                    || !touched.contains(&p[ns.path.len()])
                {
                    continue;
                }
                if let Some(table) = o.parent().and_then(|i| arena.object_key(i)) {
                    by_ns.entry(other.clone()).or_default().push(table);
                }
            }
        }
        for (other, mut keys) in by_ns {
            keys.sort();
            keys.dedup();
            let level = self.state_level(&other);
            let mut job = self.fragment_job(
                other,
                keys,
                level,
                Priority::Background,
                Trigger::Background,
            );
            job.follow_up = true;
            self.enqueue(job, true);
        }
    }

    /// Rebuilds `arenas` (key, arena, system) on the blocking pool with `op`; their tasks wait
    /// until [`Actor::republished`] publishes the result (T15 m-rr2, T17 fm3).
    fn republish(&mut self, arenas: Vec<Rebuild>, op: BgOp) {
        if arenas.is_empty() {
            return;
        }
        for (k, _, _) in &arenas {
            self.republishing.insert(k.clone());
        }
        let guard = Abandoned {
            tx: Some(self.bg_tx.clone()),
            keys: arenas.iter().map(|(k, _, _)| k.clone()).collect(),
        };
        drop(self.rt.spawn_blocking(move || {
            let mut guard = guard;
            let done = arenas
                .into_iter()
                .map(|(key, arena, system)| {
                    let ptr = Arc::as_ptr(&arena) as usize;
                    let out = match &op {
                        BgOp::Narrow(f) => narrowed(f, &key, &arena, system),
                        BgOp::Forget => forgotten(&arena),
                        BgOp::Refold(fold) => refold(&arena, *fold).map(|a| (a, Vec::new())),
                    };
                    (key, ptr, out)
                })
                .collect();
            if let Some(tx) = guard.tx.take() {
                let _ = tx.send(Republished {
                    op,
                    done,
                    abandoned: Vec::new(),
                });
            }
        }));
        self.update_progress();
    }

    /// Publishes what [`Actor::republish`] rebuilt. An arena published by something else
    /// meanwhile is rebuilt again from its new version (under the current filters or rule).
    fn republished(&mut self, r: Republished) {
        // A rebuild that died (a panic) gives its namespaces back unchanged (review m-2).
        for k in &r.abandoned {
            self.republishing.remove(k);
        }
        let snap = self.catalog.current();
        let mut changes = Vec::new();
        let mut delta = CatalogDelta::default();
        let mut narrowed_counts = Vec::new();
        let mut again = Vec::new();
        for (key, ptr, out) in r.done {
            match snap.namespaces.get(&key) {
                None => {
                    self.republishing.remove(&key);
                    continue;
                }
                Some(a) if Arc::as_ptr(a) as usize != ptr => {
                    again.push((key.clone(), a.clone(), self.system_ns.contains(&key)));
                    continue;
                }
                Some(_) => {
                    self.republishing.remove(&key);
                }
            }
            let Some((next, dropped)) = out else {
                continue;
            };
            changes.push(CatalogChange::Put(Arc::new(next)));
            match &r.op {
                BgOp::Narrow(_) => {
                    narrowed_counts.push((
                        key.clone(),
                        u32::try_from(dropped.len()).unwrap_or(u32::MAX),
                    ));
                    delta.out_of_scope.extend(dropped);
                }
                BgOp::Forget => {
                    if let LoadState::Loaded { level, at, .. } =
                        snap.state(&ContainerKey::Namespace(key.clone())).clone()
                    {
                        changes.push(CatalogChange::SetState(
                            ContainerKey::Namespace(key.clone()),
                            LoadState::Loaded {
                                level,
                                at,
                                watermark: Watermark::None,
                                stale: true,
                            },
                        ));
                    }
                }
                BgOp::Refold(_) => {}
            }
        }
        if !changes.is_empty() {
            let narrowing = matches!(r.op, BgOp::Narrow(_));
            match self.publish(changes, narrowing.then_some(delta)) {
                Ok(_) => {
                    for (ns, count) in narrowed_counts {
                        let _ = self.events.send(EngineEvent::ScopeNarrowed { ns, count });
                    }
                }
                Err(_) => {
                    let _ = self.events.send(EngineEvent::PublishFailed);
                }
            }
        }
        if !again.is_empty() {
            let op = match r.op {
                BgOp::Narrow(_) => BgOp::Narrow(self.filters.clone()),
                BgOp::Forget => BgOp::Forget,
                BgOp::Refold(_) => {
                    let ident = &self.catalog.current().server.ident;
                    BgOp::Refold(
                        FoldRule::for_names(ident.dialect, ident, NameClass::Table)
                            .unwrap_or(FoldRule::Exact),
                    )
                }
            };
            self.republish(again, op);
        }
        // ## m4-t33a-progress (named exception): the loop publishes progress after the tasks
        // that waited for this rebuild have started.
    }

    /// Writes the names of every held mass deletion where the handles read them.
    fn sync_pending_names(&self) {
        let names: HashMap<NsKey, Vec<ObjectKey>> = self
            .pending
            .iter()
            .map(|(ns, p)| (ns.clone(), top_drops(ns, p.outcome.delta())))
            .collect();
        *lock(&self.pending_names) = names;
    }
    // ## m4-t22-state (named lines: validation tasks)
    // ## m4-t15-engine (validation tasks)
}

// ## m4-t22-state (named lines: validation tasks)
/// Resolves when the validation clock reaches `at` (never without one).
async fn validation_due(clock: ValidationClock, at: Option<Duration>) {
    match at {
        Some(t) => clock.sleep_until(t).await,
        None => std::future::pending().await,
    }
}

/// What [`Actor::republish`] does to an arena.
#[derive(Debug, Clone)]
enum BgOp {
    /// Removes what the filters no longer keep (a scope change, `out_of_scope`).
    Narrow(Filters),
    /// Drops the watermark (another or an unknown server: the next pass is full).
    Forget,
    /// Rebuilds the lookup keys under a new table fold rule.
    Refold(FoldRule),
}

/// Arenas rebuilt on the blocking pool: key, the arena pointer it was built from, and the new
/// arena with the objects it removed (`None`: nothing to change).
struct Republished {
    op: BgOp,
    done: Vec<(NsKey, usize, Option<Rebuilt>)>,
    /// Namespaces of a rebuild that did not finish: released, nothing published.
    abandoned: Vec<NsKey>,
}

/// Releases a rebuild's namespaces when it dies before answering (review m-2).
struct Abandoned {
    tx: Option<mpsc::UnboundedSender<Republished>>,
    keys: Vec<NsKey>,
}

impl Drop for Abandoned {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Republished {
                op: BgOp::Forget,
                done: Vec::new(),
                abandoned: std::mem::take(&mut self.keys),
            });
        }
    }
}

/// An arena to rebuild: its key, the arena, and whether its namespace is a system one.
type Rebuild = (NsKey, Arc<NamespaceSnapshot>, bool);

/// A rebuilt arena and the objects the rebuild removed.
type Rebuilt = (NamespaceSnapshot, Vec<ObjectKey>);

/// `arena` without what `filters` no longer keeps (`None` when it keeps everything or the
/// merge fails).
fn narrowed(
    filters: &Filters,
    key: &NsKey,
    arena: &NamespaceSnapshot,
    system: bool,
) -> Option<Rebuilt> {
    let gone = filters.excluded(arena, system);
    if gone.is_empty() {
        return None;
    }
    let mut existent: BTreeMap<ObjectKind, merge::ExistSet> = BTreeMap::new();
    for k in &gone {
        let set = existent.entry(k.kind).or_insert(merge::ExistSet::Scoped {
            scope: Vec::new(),
            ids: Vec::new(),
        });
        if let merge::ExistSet::Scoped { scope, .. } = set {
            scope.push(k.path[key.path.len()..].to_vec());
        }
    }
    let r = NamespaceResult {
        existent,
        changed: Vec::new(),
        categories_to_reload: Default::default(),
        new_watermark: arena.meta().watermark.clone(),
        full: false,
        level: arena.meta().level,
        read_at_ms: arena.meta().loaded_at_ms,
    };
    let policy = MergePolicy {
        trigger: Trigger::User,
        guard_min_objects: u32::MAX,
        ..MergePolicy::default()
    };
    let (next, d, _) = merge::merge(Some(arena), r, &policy).ok()?.into_parts();
    Some((next, d.dropped))
}

/// `arena` with no watermark and no object marks (R-W3-3: nothing of another or an unknown
/// server is trusted, review m-1); `None` if the rebuild fails.
fn forgotten(arena: &NamespaceSnapshot) -> Option<Rebuilt> {
    let root = arena.obj(0)?.clone().with_state(StateMark::None);
    let mut tree = hacivat_meta::Subtree::with_capacity(root, arena.len());
    let mut stack = vec![(0u32, tree.root())];
    while let Some((idx, node)) = stack.pop() {
        for (child, obj) in arena.children(idx) {
            let at = tree.add(node, obj.clone().with_state(StateMark::None));
            stack.push((child, at));
        }
    }
    let mut meta = arena.meta().clone();
    meta.watermark = Watermark::None;
    let next =
        NamespaceSnapshot::build(arena.key().clone(), arena.dbms(), arena.fold(), meta, tree)
            .ok()?;
    Some((next, Vec::new()))
}

/// Whether a MySQL pass saw the login's privileges change (`MyTime.priv_hash`; T17b carry,
/// fix round 1 I-6a): the grants facets loaded under the old privileges may be wrong.
fn privileges_moved(prev: Option<&Watermark>, next: &NamespaceSnapshot) -> bool {
    match (prev, &next.meta().watermark) {
        (Some(Watermark::MyTime { priv_hash: a, .. }), Watermark::MyTime { priv_hash: b, .. }) => {
            a != b
        }
        _ => false,
    }
}

/// Whether `delta` renamed or dropped an account (a role, user or login).
fn accounts_changed(delta: &CatalogDelta) -> bool {
    let account = |k: &ObjectKey| {
        matches!(
            k.kind,
            ObjectKind::Role | ObjectKind::User | ObjectKind::Login
        )
    };
    delta.renamed.iter().any(|(before, _, _)| account(before)) || delta.dropped.iter().any(account)
}

/// The top-level names of `ns` that `delta` created, altered, renamed (both names) or dropped,
/// or below which it changed anything.
fn touched_top(ns: &NsKey, delta: &CatalogDelta) -> BTreeSet<Name> {
    let depth = ns.path.len();
    let top = |k: &ObjectKey| k.path.get(depth).cloned();
    delta
        .created
        .iter()
        .chain(delta.dropped.iter())
        .chain(delta.altered.iter().map(|(k, _)| k))
        .chain(delta.renamed.iter().flat_map(|(a, b, _)| [a, b]))
        .filter(|k| k.path.starts_with(&ns.path))
        .filter_map(top)
        .collect()
}

/// The top-level objects `delta` removes from `ns` (server deletions, not scope changes).
fn top_drops(ns: &NsKey, delta: &CatalogDelta) -> Vec<ObjectKey> {
    delta
        .dropped
        .iter()
        .filter(|k| k.path.len() == ns.path.len() + 1)
        .cloned()
        .collect()
}

/// The keys a probe of the missing name `name` in `ns` stamps by name: every top-level kind a
/// MySQL or MariaDB schema holds (a lookup's kind group is not known here, and a kind left out
/// would let a wrong `Missing` stand).
fn probe_keys(ns: &NsKey, name: &Name) -> Vec<ObjectKey> {
    [
        ObjectKind::Table,
        ObjectKind::View,
        ObjectKind::Sequence,
        ObjectKind::Routine,
        ObjectKind::Procedure,
        ObjectKind::Event,
    ]
    .into_iter()
    .map(|kind| {
        let mut path: smallvec::SmallVec<[Name; 4]> = ns.path.iter().cloned().collect();
        path.push(name.clone());
        ObjectKey {
            kind,
            path,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    })
    .collect()
}
// ## m4-t22-state

async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

// ## m4-t17-mysql (named exception; review I-2)
/// `arena` rebuilt under the table fold rule `fold`, with its objects, metadata and watermark
/// unchanged (only its lookup keys change). `None` if the rebuild fails (the arena is then
/// kept, and its lookups answer `ServerDependent` against the new facts).
fn refold(arena: &NamespaceSnapshot, fold: FoldRule) -> Option<NamespaceSnapshot> {
    let mut tree = hacivat_meta::Subtree::with_capacity(arena.obj(0)?.clone(), arena.len());
    let mut stack = vec![(0u32, tree.root())];
    while let Some((idx, node)) = stack.pop() {
        for (child, obj) in arena.children(idx) {
            let at = tree.add(node, obj.clone());
            stack.push((child, at));
        }
    }
    NamespaceSnapshot::build(
        arena.key().clone(),
        arena.dbms(),
        fold,
        arena.meta().clone(),
        tree,
    )
    .ok()
}

// ## m4-t15b-facets (named exception in Task 15's file; `engine/facet.rs` has the rules)
impl Actor {
    /// `IntroHandle::load_facet`: one task per namespace of the keys whose facet is not loaded
    /// (or stale); a key no arena holds is not read. A facet the introspector does not read is
    /// published `Failed { Unsupported }` at once, except `Sources`, which a fragment of the
    /// keys at `Level::Sources` reads where the engine reads definitions as a level.
    fn facet_request(
        &mut self,
        keys: Vec<ObjectKey>,
        kind: hacivat_meta::props::FacetKind,
        prio: Priority,
        w: Waiter,
    ) {
        let snap = self.catalog.current();
        let plan = super::facet::plan(&snap, &keys, kind, SystemTime::now());
        if plan.is_empty() {
            return w.resolve(Ok(snap.generation));
        }
        let read = self.caps.facets.contains(&kind);
        if !read && kind != hacivat_meta::props::FacetKind::Sources {
            let changes: Vec<CatalogChange> = plan
                .iter()
                .flat_map(|(ns, ks)| {
                    super::facet::failed(
                        &snap,
                        ns,
                        ks,
                        kind,
                        hacivat_meta::snapshot::IntroErrorCode::Unsupported,
                        self.limits.failed_retry_after,
                    )
                })
                .collect();
            return w.resolve(
                self.publish(changes, None)
                    .and(Err(IntroError::NotImplemented)),
            );
        }
        // Where `Sources` is a level, an object read at it since its namespace last changed has
        // nothing more to give (its text, if it has one, came with that read): no second read.
        let plan = if read {
            plan
        } else {
            let fresh = |ns: &NsKey, k: &ObjectKey| {
                let Some(arena) = snap.namespaces.get(ns) else {
                    return false;
                };
                let g = arena.generation();
                self.sources_read_ns.get(ns) == Some(&g) || self.sources_read.get(k) == Some(&g)
            };
            plan.into_iter()
                .map(|(ns, ks)| {
                    let ks: Vec<ObjectKey> = ks.into_iter().filter(|k| !fresh(&ns, k)).collect();
                    (ns, ks)
                })
                .filter(|(_, ks)| !ks.is_empty())
                .collect()
        };
        if plan.is_empty() {
            return w.resolve(Ok(snap.generation));
        }
        // `Sources` reads are per key (review I-2, T17b m-6): a key the login may not
        // `SHOW CREATE` fails alone, never another key or another caller's.
        let plan: Vec<(NsKey, Vec<ObjectKey>)> =
            if read && kind == hacivat_meta::props::FacetKind::Sources {
                plan.into_iter()
                    .flat_map(|(ns, ks)| ks.into_iter().map(move |k| (ns.clone(), vec![k])))
                    .collect()
            } else {
                plan.into_iter().collect()
            };
        let join = Arc::new(Mutex::new(Join {
            left: plan.len(),
            generation: 0,
            error: None,
            waiters: vec![w],
        }));
        for (ns, ks) in plan {
            let mut job = if read {
                self.new_job(
                    Work::Facet { ns, keys: ks, kind },
                    prio,
                    Trigger::Background,
                )
            } else {
                // Not a load: it reads even when the namespace is held at `Sources` (a cached
                // namespace has its references but none of its texts).
                self.fragment_job(ns, ks, Level::Sources, prio, Trigger::Background)
            };
            job.add(Waiter::Join(join.clone()), false);
            self.enqueue(job, false);
        }
    }

    /// A facet read answered: its keys are published `Loaded` (answered) or `NotLoaded` (left
    /// out), in one side-table publish.
    fn facet_done(&mut self, job: Job, rows: &crate::introspector::FacetRows) {
        let started = self.facet_started.remove(&job.id);
        let Work::Facet { ns, keys, kind } = &job.work else {
            return job.resolve(Err(IntroError::Protocol(Name::from("facet"))));
        };
        if self.out_of_scope(ns) {
            return job.resolve(Err(IntroError::Cancelled));
        }
        let snap = self.catalog.current();
        let level = self.state_level(ns);
        // Review m-2: facets went stale elsewhere while this read ran (a role rename): its
        // answer is published stale, never as fresh.
        let stale = started != Some(self.facet_epoch);
        match super::facet::answered(&snap, ns, keys, *kind, rows, level, stale) {
            Ok(changes) => {
                let r = self.publish(changes, None);
                job.resolve(r);
            }
            Err(e) => self.facet_failed(job, e),
        }
    }

    /// A facet read failed: a definite failure is published `Failed` for its keys (R-W4-3); a
    /// transient one publishes nothing. Never counts towards the namespace's back-pressure.
    fn facet_failed(&mut self, job: Job, e: IntroError) {
        self.facet_started.remove(&job.id);
        if let Work::Facet { ns, keys, kind } = &job.work
            && super::facet::decides(&e)
            && !self.out_of_scope(ns)
        {
            let snap = self.catalog.current();
            let changes = super::facet::failed(
                &snap,
                ns,
                keys,
                *kind,
                super::facet::failure_code(&e),
                self.limits.failed_retry_after,
            );
            let _ = self.publish(changes, None);
        }
        job.resolve(Err(e));
    }
}
// ## m4-t15b-facets

// ## m4-t15b-facets (named exception; T25 review m-7, carried to T15b): `VerdictSink` units.
#[cfg(test)]
mod verdict_tests {
    use super::*;

    fn key(t: &str) -> ObjectKey {
        ObjectKey {
            kind: ObjectKind::Table,
            path: [Name::from("db"), Name::from("s"), Name::from(t)]
                .into_iter()
                .collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }

    /// The verdicts arrive once, when every job of the request has added its share.
    #[test]
    fn verdicts_arrive_when_every_job_added_its_share() {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let sink = VerdictSink::new(tx);
        sink.expect(2);
        sink.add(&[key("a")], &[Staleness::Current]);
        assert!(
            rx.try_recv().is_err(),
            "one job of two answered: nothing yet"
        );
        sink.add(&[key("b")], &[Staleness::Changed]);
        assert_eq!(
            rx.try_recv().expect("both answered"),
            vec![
                (key("a"), Staleness::Current),
                (key("b"), Staleness::Changed)
            ]
        );
    }

    /// A failed job decides nothing for the whole request: the receiver sees the sender gone,
    /// even when another job had answered (the DDL view then says "couldn't check").
    #[test]
    fn a_failed_job_withholds_every_verdict() {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let sink = VerdictSink::new(tx);
        sink.expect(2);
        sink.add(&[key("a")], &[Staleness::Current]);
        sink.fail();
        sink.add(&[key("b")], &[Staleness::Current]);
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }

    /// Jobs cancelled before answering drop their shares: the receiver sees the sender gone.
    #[test]
    fn cancelled_jobs_leave_the_receiver_undecided() {
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        let sink = VerdictSink::new(tx);
        sink.expect(2);
        let other = VerdictSink(sink.0.clone());
        sink.add(&[key("a")], &[Staleness::Dropped]);
        drop(sink);
        drop(other);
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }
}
// ## m4-t15b-facets
