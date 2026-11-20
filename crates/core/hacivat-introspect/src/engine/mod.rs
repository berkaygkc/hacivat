// ## m4-t01c-core
//! The introspection engine (HV-INTRO-01; Task 15, facet loading Task 15b): one scheduler per
//! profile on the `CoreRuntime`, single-flight requests, levels, atomic publish per namespace,
//! progress. [`IntroHandle`] is how the hub, the explorer and later milestones talk to it.
//!
//! ## m4-t15-engine (M4 Task 15; ADR 0008 `## m4-t15 — engine and hub`)
//!
//! [`Engine::start`] spawns one actor per profile on the runtime it is given (the
//! `CoreRuntime`); the actor is the catalog's only writer ([`Publisher`]). Nothing here runs on
//! a GPUI thread: every [`IntroHandle`] method only sends a message (or reads a lock that I/O
//! never holds) and answers a [`RefreshTicket`].
//!
//! - **Tasks** (alg/02 HV-INTRO-01): `ScopeRefresh`, namespace passes (`Full`, `Incremental`,
//!   `Brief`), `Fragment` (also `Elevate` and the per-object children read), `CheckState`.
//! - **Priorities:** user-visible > completion-triggered > background > periodic
//!   ([`Priority`]). Queued tasks of one namespace **coalesce** (one pass, the stronger mode
//!   and the higher level; one fragment with the union of the keys); a load request joins a
//!   running pass of its namespace (**single flight**), a refresh queues a new one after it.
//! - **One task per namespace at a time** (and one scope refresh), at most
//!   [`EngineLimits::workers`] at once. Each task takes its own `MetaConn` and drops it when it
//!   ends (it holds a budget slot; Task 5 concern 1).
//! - **Bulk policy:** [`IntroHandle::load_children`] reads one object at once; ≥ 3 distinct
//!   objects of one namespace within 500 ms, or a namespace under 2,000 relations, upgrade to
//!   one bulk pass of the namespace (one query per kind, never one per object).
//! - **Atomic publish:** a task's whole result is merged (Task 10) and published in one
//!   [`Publisher::publish_with_delta`]; on cancel or error nothing is published, and only a
//!   namespace with no previous state becomes `Failed`. A kind whose existence query failed
//!   makes its namespace `Partial` (lookups answer `Unknown(Failed)`, never `Missing`) and its
//!   `Children` container `Failed`.
//! - **Levels** ([`LevelPolicy`], R-M4-21): the auto level from the namespace's object count,
//!   overrides inherited by children, and back-pressure: a namespace whose pass timed out
//!   twice is read one level lower ([`EngineEvent::LevelLowered`], `intro-hint-level-lowered`).
//!   The merge never lowers a level; the published `LoadState` carries the level the pass read.
//! - **On connect** (steps 1–3): cached namespaces were published stale when the engine
//!   started; then `ScopeRefresh`; then the current namespace `Incremental` if cached, else
//!   `Brief` (names first) then `Full` at its auto level; other namespaces on first expansion.
//! - **Retries:** a lost connection restarts the task once from the beginning; a
//!   `CatalogRace` (PG `XX000`) is retried once ([`EngineEvent::CatalogRaceRetried`], log only).
//! - **Mass-deletion guard** (D-039): a guarded merge is kept pending, nothing of it applies,
//!   the namespace is marked stale and `PendingConfirmation` is announced;
//!   [`IntroHandle::confirm_pending`] applies or drops it.
//! - **Persistence:** every published namespace is offered to Task 11's debounced save, run on
//!   the blocking pool.
//! - **Periodic validation** (R-M4-20): an incremental pass of every loaded namespace every
//!   `validate_every`, skipped while the user's session is in a transaction on a
//!   single-connection profile ([`MetaSource::user_in_transaction`]).
//!
//! ## m4-t22-state (Task 22; ADR 0008 `## m4-t22 — freshness`)
//!
//! - A validation round (periodic, on [`Engine::start_with_clock`]'s clock, and on connect for
//!   every cached in-scope namespace but the current one) is the cheap check
//!   (`freshness::check_namespace`): unchanged clears the stale mark, changed queues an
//!   incremental (or full) pass. A round inside the user's transaction is deferred, not lost.
//! - `check_state` runs `freshness::check_state` per database of its keys and re-reads what
//!   changed or was dropped, storing the stamps it read as marks.
//! - Validation passes never remove more than the guard allows across passes (D-039);
//!   [`IntroHandle::pending_confirmation`] names what a held deletion would remove.

pub mod facet;
pub mod filters;
pub mod levels;
mod scheduler;
mod work;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_meta::merge::Trigger;
use hacivat_meta::persist::CacheStore;
use hacivat_meta::props::FacetKind;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::view::LookupHooks;
use hacivat_meta::{Catalog, Level, Name, NsKey, ObjectKey, Publisher};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_session::meta::{MetaConn, MetaError};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::ddl_effects::Target;
use crate::introspector::Introspector;
use crate::scope::{CurrentCtx, ScopePattern};
use crate::ticket::RefreshTicket;

pub use filters::{Filters, MAX_PUSHED_RUNS_PER_SECTION, ScopeFilterAdapter, bounded_pushdown};
pub use levels::{LevelChoice, LevelPolicy, LevelSource, NsRole, Thresholds, auto_level};

/// What a running task is doing (a closed set: the UI shows a static phase label and a
/// determinate count, never ambient motion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    ListNamespaces,
    Tables,
    Columns,
    Indexes,
    Constraints,
    Triggers,
    Routines,
    Types,
    Sequences,
    Comments,
    Sources,
    Facets,
    Existence,
    Validate,
}

/// One running task's progress.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskProgress {
    pub ns: NsKey,
    pub phase: Phase,
    pub done: u64,
    /// `None` until the total is known.
    pub total: Option<u64>,
}

/// The engine's progress: one entry per running task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Progress {
    pub tasks: Vec<TaskProgress>,
}

// ## m4-t15-engine

/// Where the engine gets its metadata connections: the profile's `SessionManager` (the hub's
/// adapter), or a test double.
pub trait MetaSource: Send + Sync {
    /// A read-only metadata connection to `database` (`None`: the profile's). `Busy` while a
    /// single-connection profile's user session holds a transaction: the engine waits and asks
    /// again (alg/02 HV-INTRO-01 edge 10).
    fn acquire<'a>(
        &'a self,
        database: Option<Name>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>>;

    /// Whether periodic validation must wait: the profile shares one connection with the user
    /// and the user's session is inside a transaction.
    fn user_in_transaction(&self) -> bool {
        false
    }

    // ## m4-t15-engine (fix round 2, RR-3)
    /// The engine closed a connection `acquire` handed out (it is dropped first). A test
    /// double counts open connections with it; the product's connections give their budget
    /// slot back on drop.
    fn released(&self) {}
}

/// A request's priority (alg/02 HV-INTRO-01 "Scheduler"), lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Priority {
    /// Periodic validation.
    Periodic,
    /// Background validation, refresh after DDL, follow-ups.
    Background,
    /// A lookup (completion) needed the namespace.
    Completion,
    /// Something the user looks at (expand, open source, a user refresh).
    User,
}

/// What the engine tells its listeners besides the catalog's own events. Codes only: the UI
/// maps them to Fluent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    /// Back-pressure lowered `ns`'s auto level to `level` (`intro-hint-level-lowered`).
    LevelLowered { ns: NsKey, level: Level },
    /// A pass over `ns` met a concurrent drop and was retried once (`intro-catalog-race-retried`,
    /// log only).
    CatalogRaceRetried { ns: NsKey },
    /// The server no longer lists `ns`; it was removed (an explicit drop is not guarded, but
    /// shown, R-M4-22).
    NamespaceDropped { ns: NsKey },
    /// A cache write failed for lack of space (`model-cache-write-failed`, shown once).
    CacheWriteFailed,
    /// A mass deletion held for `ns` (D-039) no longer applies: a later pass of the namespace
    /// replaced it. The UI withdraws its prompt (review I-3).
    PendingWithdrawn { ns: NsKey },
    /// The scope refresh could not list the other PostgreSQL database `db` (no `CONNECT`, a
    /// refused login): its schemas are unknown until a later refresh (review RR-2).
    DatabaseUnlisted { db: Name },
    /// A narrower scope filter removed `count` objects of `ns` (R-W3-2): a scope change, not a
    /// server deletion; the delta lists them in `out_of_scope`.
    ScopeNarrowed { ns: NsKey, count: u32 },
    /// A publish the engine made on its own (a scope or filter change) failed; the catalog is
    /// unchanged (review m9).
    PublishFailed,
}

/// The engine's knobs; [`EngineLimits::default`] is alg/02's numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineLimits {
    /// Tasks run at once (each on its own metadata connection; the per-database pool may make
    /// some wait).
    pub workers: usize,
    /// The bulk policy's window (500 ms) and its distinct-object count (3).
    pub bulk_window: Duration,
    pub bulk_distinct: usize,
    /// A namespace with fewer relations than this goes bulk at once (2,000).
    pub bulk_small_ns: u32,
    /// Fragments of one namespace requested within this window run (and publish) once (50 ms).
    pub fragment_window: Duration,
    /// A lookup that answered `Missing` re-checks its namespace at most this often (30 s;
    /// review m7).
    pub probe_interval: Duration,
    /// How long to wait before asking a busy single-connection profile again.
    pub busy_retry: Duration,
    /// `LoadState::Failed::retry_after` of a namespace that failed with no previous state.
    pub failed_retry_after: Duration,
    /// The cache save delay per namespace (Task 11's `SAVE_DEBOUNCE`).
    pub save_debounce: Duration,
}

impl Default for EngineLimits {
    fn default() -> Self {
        EngineLimits {
            workers: 2,
            bulk_window: Duration::from_millis(500),
            bulk_distinct: 3,
            bulk_small_ns: 2_000,
            fragment_window: Duration::from_millis(50),
            probe_interval: Duration::from_secs(30),
            busy_retry: Duration::from_millis(250),
            failed_retry_after: Duration::from_secs(30),
            save_debounce: hacivat_meta::persist::SAVE_DEBOUNCE,
        }
    }
}

/// A profile's cache store, shared with the hub (every call blocks: the engine makes them on
/// the blocking pool).
pub type CacheHandle = Arc<Mutex<CacheStore>>;

/// What [`Engine::start`] needs.
pub struct EngineConfig {
    pub profile: ProfileId,
    pub dbms: DbmsKind,
    pub meta: Arc<dyn MetaSource>,
    pub introspector: Arc<dyn Introspector>,
    pub catalog: Arc<Catalog>,
    /// The catalog's single writer: the engine is the only one.
    pub catalog_publisher: Publisher,
    pub cache: Option<CacheHandle>,
    pub levels: LevelPolicy,
    pub scope: ScopePattern,
    /// The session's current database and schema (what `@` means; the current namespace).
    pub current: CurrentCtx,
    pub filters: Filters,
    pub validate_every: Option<Duration>,
    pub limits: EngineLimits,
    /// The runtime the engine runs on (the `CoreRuntime`).
    pub runtime: tokio::runtime::Handle,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("profile", &self.profile)
            .field("dbms", &self.dbms)
            .finish_non_exhaustive()
    }
}

/// The engine (one per profile).
#[derive(Debug)]
pub struct Engine;

impl Engine {
    /// Starts the profile's engine on `cfg.runtime`: it first publishes the cached namespaces
    /// (stale), then serves requests. Exactly one per profile: it owns the catalog's publisher.
    pub fn start(cfg: EngineConfig) -> IntroHandle {
        Engine::start_with_clock(cfg, crate::freshness::ValidationClock::default())
    }

    // ## m4-t22-state (named exception)
    /// [`Engine::start`] with periodic validation (R-M4-20) on `clock`: tokio's in the product,
    /// a manual one in tests (the clock seam; no fixed sleeps, no wall clock).
    pub fn start_with_clock(
        cfg: EngineConfig,
        clock: crate::freshness::ValidationClock,
    ) -> IntroHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        let progress = Arc::new(watch::Sender::new(Progress::default()));
        let (events, _) = broadcast::channel(64);
        let shared = scheduler::Shared {
            progress,
            events,
            levels: Arc::new(Mutex::new(cfg.levels.clone())),
            roles: Arc::new(Mutex::new(scheduler::Roles::default())),
            stopped: CancellationToken::new(),
            // ## m4-t22-state (named exception)
            pending: Arc::default(),
        };
        let handle = IntroHandle {
            inner: Arc::new(Inner {
                catalog: cfg.catalog.clone(),
                progress: shared.progress.clone(),
                scope: Mutex::new(Some(cfg.scope.clone())),
                levels: shared.levels.clone(),
                roles: shared.roles.clone(),
                events: shared.events.clone(),
                stopped: shared.stopped.clone(),
                // ## m4-t22-state (named exception)
                pending: shared.pending.clone(),
                tx: Some(tx),
            }),
        };
        scheduler::spawn(cfg, rx, shared, clock);
        handle
    }
}

/// A profile's handle on its engine (Clone; cheap).
#[derive(Debug, Clone)]
pub struct IntroHandle {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    catalog: Arc<Catalog>,
    progress: Arc<watch::Sender<Progress>>,
    scope: Mutex<Option<ScopePattern>>,
    levels: Arc<Mutex<LevelPolicy>>,
    roles: Arc<Mutex<scheduler::Roles>>,
    events: broadcast::Sender<EngineEvent>,
    /// Cancelled when the engine has stopped (review m1: `closed` holds no sender).
    stopped: CancellationToken,
    // ## m4-t22-state (named exception)
    pending: scheduler::PendingNames,
    /// `None` for a detached handle (no engine).
    tx: Option<mpsc::UnboundedSender<scheduler::Cmd>>,
}

impl IntroHandle {
    /// A handle with no engine behind it: `catalog` is served as it is (cache only) and every
    /// request is refused with `IntroError::NotImplemented`. A test double for the UI.
    pub fn detached(catalog: Arc<Catalog>) -> Self {
        IntroHandle {
            inner: Arc::new(Inner {
                catalog,
                progress: Arc::new(watch::Sender::new(Progress::default())),
                scope: Mutex::new(None),
                levels: Arc::new(Mutex::new(LevelPolicy::default())),
                roles: Arc::new(Mutex::new(scheduler::Roles::default())),
                events: broadcast::channel(1).0,
                stopped: {
                    let t = CancellationToken::new();
                    t.cancel();
                    t
                },
                // ## m4-t22-state (named exception)
                pending: Arc::default(),
                tx: None,
            }),
        }
    }

    /// The catalog the engine publishes into.
    pub fn catalog(&self) -> &Arc<Catalog> {
        &self.inner.catalog
    }

    /// Sends `req` at `prio`; the ticket resolves when the engine has answered it.
    fn request(&self, req: scheduler::Request, prio: Priority) -> RefreshTicket {
        let Some(tx) = &self.inner.tx else {
            return RefreshTicket::not_implemented();
        };
        let (ticket, reply) = RefreshTicket::pending();
        // A closed channel drops `reply`: the ticket resolves to `Cancelled`.
        let _ = tx.send(scheduler::Cmd::Request { req, prio, reply });
        ticket
    }

    fn send(&self, cmd: scheduler::Cmd) {
        if let Some(tx) = &self.inner.tx {
            let _ = tx.send(cmd);
        }
    }

    /// Resolves once the engine has published the cached namespaces (on start).
    ///
    /// ## m4-t33a-hygiene (fix round 1, review M4)
    /// It is also an actor-order barrier, and tests rely on that: the engine's loop answers it
    /// in its own step, after every command, timer and validation step handled before it, and
    /// after that step's `pump` and progress publish. A step that was already under way when
    /// it was sent (a validation round that has asked `user_in_transaction`) has queued and
    /// started its work, and `progress` shows it, by the time it resolves. Never answer it
    /// outside the actor (from a cached generation on the handle, say): that would break the
    /// barrier.
    pub fn ready(&self) -> RefreshTicket {
        self.request(scheduler::Request::Ready, Priority::User)
    }

    /// The on-connect algorithm (HV-INTRO-01 steps 1–3). Resolves with the generation that
    /// holds the current namespace's pass (or the scope refresh when there is none).
    pub fn on_connect(&self) -> RefreshTicket {
        self.request(scheduler::Request::OnConnect, Priority::User)
    }

    /// Loads a namespace on its first expansion (names first, then its auto level); a stale
    /// cached namespace is validated incrementally. An out-of-scope namespace answers
    /// `Cancelled` and reads nothing.
    ///
    /// ## m4-t22-state (named exception, doc only)
    /// On a first expansion the ticket resolves after the **names-first** pass; the read at the
    /// namespace's level follows, queued. A caller that needs it waits for the namespace's
    /// `LoadState::Loaded { level, .. }` to reach that level.
    pub fn expand(&self, ns: NsKey) -> RefreshTicket {
        self.request(scheduler::Request::Load(ns), Priority::User)
    }

    /// Reads the children of `key` (an explorer expansion, a column list) under the bulk
    /// policy: one fragment of `key`, or one bulk pass of its namespace.
    pub fn load_children(&self, key: ObjectKey) -> RefreshTicket {
        self.request(scheduler::Request::Children(key), Priority::User)
    }

    /// Reads `key` at `level` (and its children). Waits for a running pass of its namespace and
    /// answers at once when the namespace already holds it at `level`.
    pub fn elevate(&self, key: ObjectKey, level: Level) -> RefreshTicket {
        self.request(scheduler::Request::Elevate(key, level), Priority::User)
    }

    /// Re-reads `keys` of `ns` (refresh after DDL, a heal after a rename). Fragments of one
    /// namespace within 50 ms run and publish once.
    pub fn fragment(&self, ns: NsKey, keys: Vec<ObjectKey>) -> RefreshTicket {
        self.request(
            scheduler::Request::Refresh(Target::Objects { ns, keys }, Trigger::Background),
            Priority::Background,
        )
    }

    /// Refreshes `target` (a user refresh, refresh after DDL, a background check). A user
    /// refresh runs at user priority and asks before a mass deletion.
    pub fn refresh(&self, target: Target, trigger: Trigger) -> RefreshTicket {
        let prio = match trigger {
            Trigger::User => Priority::User,
            Trigger::Background => Priority::Background,
        };
        self.request(scheduler::Request::Refresh(target, trigger), prio)
    }

    // ## m4-t15b-facets (named exception, doc only)
    /// Loads facet `kind` of `keys` (R-M4-30; `engine::facet`): single flight per object and
    /// facet, read on a metadata connection as a task of the object's namespace, published as
    /// a side-table change (`CatalogChange::SetFacet`) that touches no arena. Resolves with the
    /// generation that holds the facets; a read the introspector refused resolves to its error
    /// after the keys were published `Failed`.
    pub fn load_facet(&self, keys: Vec<ObjectKey>, kind: FacetKind) -> RefreshTicket {
        facet::load(self, keys, kind)
    }

    /// Checks `keys` against the server (HV-INTRO-15) and re-reads those that changed or were
    /// dropped; resolves with the generation that holds them.
    pub fn check_state(&self, keys: Vec<ObjectKey>) -> RefreshTicket {
        self.request(scheduler::Request::CheckState(keys), Priority::User)
    }

    // ## m4-t25-ddl-view (named exception in Task 15's file)
    /// [`IntroHandle::check_state`], also answering each key's verdict (`Current`, `Changed`,
    /// `Dropped`, `Unknown`) as soon as the stamps were compared, before any re-read. The
    /// receiver sees its sender dropped when the check failed or was cancelled (nothing was
    /// decided). The DDL view's freshness (surface 11).
    pub fn check_state_verdicts(
        &self,
        keys: Vec<ObjectKey>,
    ) -> (
        RefreshTicket,
        tokio::sync::oneshot::Receiver<Vec<(ObjectKey, crate::introspector::Staleness)>>,
    ) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let ticket = self.request(
            scheduler::Request::CheckStateVerdicts(keys, scheduler::VerdictSink::new(tx)),
            Priority::User,
        );
        (ticket, rx)
    }
    // ## m4-t25-ddl-view

    /// Replaces the profile's scope (HV-INTRO-02): namespaces leaving it are removed (and their
    /// running results discarded), namespaces entering it are listed as not loaded.
    pub fn set_scope(&self, pattern: ScopePattern) {
        *self
            .inner
            .scope
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(pattern.clone());
        self.send(scheduler::Cmd::SetScope(pattern));
    }

    /// The scope last set.
    pub fn scope(&self) -> Option<ScopePattern> {
        self.inner
            .scope
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Sets the session's current database and schema (what `@` means).
    pub fn set_current(&self, current: CurrentCtx) {
        self.send(scheduler::Cmd::SetCurrent(current));
    }

    /// Sets (or clears, `None`) the level of `container` and the namespaces below it (the
    /// pills). A loaded namespace now below its new level is re-read at it.
    pub fn set_level_override(&self, container: ContainerKey, level: Option<Level>) {
        self.inner
            .levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_override(container.clone(), level);
        self.send(scheduler::Cmd::LevelChanged(container));
    }

    /// The level `ns` is read at and why (the pill; R-M4-21): `role` and `count` (its major
    /// objects) decide the auto level. A lock no I/O holds; safe on any thread.
    pub fn level_for(&self, ns: &NsKey, role: NsRole, count: u32) -> LevelChoice {
        self.inner
            .levels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .level_for(ns, role, count)
    }

    /// The engine's progress.
    pub fn progress(&self) -> watch::Receiver<Progress> {
        self.inner.progress.subscribe()
    }

    /// The engine's events (hints and notices).
    pub fn events(&self) -> broadcast::Receiver<EngineEvent> {
        self.inner.events.subscribe()
    }

    /// Lookup hooks for a `SnapshotView` (completion): a namespace a lookup needed but found
    /// not loaded is loaded, and a name answered `Missing` makes its namespace check for
    /// changes, both at completion priority. The callbacks only send a message.
    pub fn lookup_hooks(&self) -> LookupHooks {
        let unknown = self.clone();
        let probe = self.clone();
        LookupHooks::new(
            move |ns| {
                drop(unknown.request(scheduler::Request::Load(ns.clone()), Priority::Completion));
            },
            // ## m4-t22-state (named exception): the probe carries the missing name (a state
            // check by name, Task 22).
            move |ns, part| {
                drop(probe.request(
                    scheduler::Request::Probe(ns.clone(), Some(part.text.clone())),
                    Priority::Completion,
                ));
            },
        )
    }

    /// Applies (`accept`) or drops a merge the mass-deletion guard held back (D-039):
    /// "Remove from cache" publishes it; "Keep cached" applies the rest of the pass and keeps
    /// the vanished objects. Nothing pending: resolves with the current generation.
    pub fn confirm_pending(&self, ns: NsKey, accept: bool) -> RefreshTicket {
        self.request(scheduler::Request::Confirm { ns, accept }, Priority::User)
    }

    /// Stops what runs on `ns` and drops what is queued for it; nothing of it is published.
    pub fn cancel(&self, ns: NsKey) {
        self.send(scheduler::Cmd::Cancel(ns));
    }

    /// Stops the engine: running tasks are cancelled (nothing of them is published), queued
    /// ones answer `Cancelled`, and pending cache saves are written.
    pub fn shutdown(&self) {
        self.send(scheduler::Cmd::Shutdown);
    }

    /// Resolves when the engine has stopped (at once for a detached handle). It holds no
    /// sender, so it never keeps an engine alive whose handles are all gone (review m1).
    pub fn closed(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.inner.stopped.clone().cancelled_owned()
    }

    /// Replaces the profile's filters (R-W3-2). A narrower scope filter removes the objects it
    /// no longer keeps from every loaded namespace at once (a scope change: never guarded, the
    /// delta lists them in `out_of_scope`); a filter that may keep more re-reads every loaded
    /// namespace in scope.
    pub fn set_filters(&self, filters: Filters) {
        self.send(scheduler::Cmd::SetFilters(filters));
    }

    // ## m4-t21-refresh (named exception)
    /// Marks the loaded namespaces `namespaces` stale without reading anything (refresh after
    /// DDL with the preference `never` or `ask`; the explorer's stale badge). The engine, the
    /// catalog's only writer, publishes it. A namespace still loading is published stale when
    /// its pass ends; `scope` also marks every loaded schema list stale (the namespace list
    /// changed: a schema or database created, dropped or renamed).
    pub fn mark_stale(&self, namespaces: Vec<NsKey>, scope: bool) {
        self.send(scheduler::Cmd::MarkStale { namespaces, scope });
    }

    // ## m4-t27-refresh-ui (named exception; T26 carry)
    /// Hands the profile's user data to the catalog's overlays (`CatalogChange::SetUserData`):
    /// virtual foreign keys, annotations and the "Introspect live" choices. The engine, the
    /// catalog's only writer, publishes it.
    pub fn set_user_data(&self, data: Arc<hacivat_meta::userdata::ProfileUserData>) {
        self.send(scheduler::Cmd::SetUserData(data));
    }
    // ## m4-t27-refresh-ui

    // ## m4-t22-state (named exception)
    /// The top-level objects a mass deletion held for `ns` would remove (D-039; the
    /// confirmation dialog lists them), `None` when nothing is held.
    pub fn pending_confirmation(&self, ns: &NsKey) -> Option<Vec<ObjectKey>> {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(ns)
            .cloned()
    }
    // ## m4-t22-state

    /// The level `ns` is read at and why, as the engine sees it (the pills; review m8): its
    /// role (current, system, other) and its loaded object count. A database namespace answers
    /// its own override, else the auto level of its loaded schemas' objects together. A lock no
    /// I/O holds; safe on any thread.
    pub fn level_of(&self, ns: &NsKey) -> LevelChoice {
        let snap = self.inner.catalog.current();
        let (role, count) = {
            let roles = self
                .inner
                .roles
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let role = if roles.current.as_ref() == Some(ns) {
                NsRole::Current
            } else if roles.system.contains(ns) {
                NsRole::System
            } else {
                NsRole::Other
            };
            let count: u32 = if ns.kind == hacivat_meta::NsKind::Database {
                snap.namespaces
                    .values()
                    .filter(|a| a.key().path.len() == 2 && a.key().path.first() == ns.path.first())
                    .map(|a| a.root().child_count())
                    .fold(0u32, u32::saturating_add)
            } else {
                snap.namespaces
                    .get(ns)
                    .map_or(0, |a| a.root().child_count())
            };
            (role, count)
        };
        self.level_for(ns, role, count)
    }
}
// ## m4-t15-engine
