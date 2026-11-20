// ## m4-t15-engine
//! One task's I/O (HV-INTRO-01 step 4): a metadata connection of its own, the introspector
//! call, the retries, the scope filter and the merge. It never publishes: the scheduler, the
//! catalog's only writer, does that with what this answers. The connection is dropped before
//! the merge (it holds a budget slot; Task 5 concern 1).

#![allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value (MetaError::Server), as the seam does"
)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hacivat_meta::merge::{self, ChangedObject, MergeOutcome, MergePolicy, NamespaceResult};
use hacivat_meta::name::FoldRule;
use hacivat_meta::{
    Level, Name, NamespaceSnapshot, NsKey, NsKind, Obj, ObjFlags, ObjectKey, ServerFacts, Subtree,
};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::filters::Filters;
use super::{CacheHandle, EngineEvent, EngineLimits, MetaSource};
use crate::introspector::{
    IntroError, Introspector, ProgressSink, ServerIdentity, Staleness, with_progress,
};
use crate::pack::Mode;
use crate::scope::{CurrentCtx, NsInfo, ScopePattern};

/// What a task does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Work {
    ScopeRefresh,
    Pass {
        ns: NsKey,
        mode: Mode,
        level: Level,
    },
    Fragment {
        ns: NsKey,
        keys: Vec<ObjectKey>,
        level: Level,
    },
    // ## m4-t22-state (named exception): a state check runs on the connection of its keys'
    // database (PostgreSQL oids are per database; the carry "split CheckState by
    // connection"), and the cheap validation of a cached namespace.
    CheckState {
        keys: Vec<ObjectKey>,
        database: Option<Name>,
    },
    Validate {
        ns: NsKey,
    },
}

impl Work {
    /// The namespace the task reads and publishes (`None`: the scope refresh, a state check).
    pub fn ns(&self) -> Option<&NsKey> {
        match self {
            Work::Pass { ns, .. } | Work::Fragment { ns, .. } | Work::Validate { ns } => Some(ns),
            Work::ScopeRefresh | Work::CheckState { .. } => None,
        }
    }
}

/// What the worker needs besides the task.
pub(super) struct WorkerCtx {
    pub meta: Arc<dyn MetaSource>,
    pub intro: Arc<dyn Introspector>,
    pub dbms: DbmsKind,
    pub limits: EngineLimits,
    pub events: broadcast::Sender<EngineEvent>,
    /// The cache, for the server identity check of `on_connect` (R-W3-3).
    pub cache: Option<CacheHandle>,
    // ## m4-t17-mysql (named exception; T15 carry m11): the progress a pass reports itself.
    pub progress: Arc<tokio::sync::watch::Sender<super::Progress>>,
}

/// What a scope refresh needs besides the listing of the profile's own database.
pub(super) struct Listing {
    /// The scope and `@`, to find the other PostgreSQL databases it may select (review I-2).
    pub scope: ScopePattern,
    pub current: CurrentCtx,
    /// Read the server's identity too, and check it against the cache (R-W3-3).
    pub identity: bool,
}

// ## m4-t17-mysql (T15 carry m4, review I-2)
/// What the scope refresh learned of the server's facts.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FactsRead {
    /// The introspector reads none (the catalog keeps its facts).
    NotRead,
    Read(Box<ServerFacts>),
    /// The introspector reads them, and the read failed: the identifier rule is unknown.
    Failed,
}

/// What the identity step found (R-W3-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IdentityRead {
    /// Not known (a stub, an error): no cached watermark is trusted.
    Unknown,
    /// Known; `cache` is the cache's verdict when there is a cache.
    Known {
        fp: u128,
        same_as_cache: Option<bool>,
    },
}

/// One task as the worker gets it.
pub(super) struct Spec {
    pub work: Work,
    /// The namespace's arena when the task started (the merge's `prev`).
    pub prev: Option<Arc<NamespaceSnapshot>>,
    /// The namespace is a system one (the scope filter never applies there).
    pub system: bool,
    /// The merge policy (the trigger, and a user's guard decision on a re-planned
    /// confirmation, I-3).
    pub policy: MergePolicy,
    /// The filters when the task started (R-W3-2).
    pub filters: Filters,
    /// The stored fold rule of table-class names, from the current server facts (m4).
    pub fold: FoldRule,
    /// A scope refresh's extras.
    pub listing: Option<Listing>,
    // ## m4-t22-state (named exception)
    /// A state check's catalog (the marks it compares with).
    pub snapshot: Option<Arc<hacivat_meta::CatalogSnapshot>>,
    /// A fragment's stamps read before it (by a state check): stored as the marks of the
    /// re-read owners that bring none of their own (PostgreSQL).
    pub marks: Vec<(ObjectKey, hacivat_meta::StateMark)>,
    /// What the user chose to keep cached ("Keep cached", D-039) in a background read's
    /// namespace: reported existent, so the read never removes it (fix round 2, R-W4-9).
    pub kept: Vec<ObjectKey>,
}

/// A merged pass or fragment, ready to publish.
pub(super) struct Merged {
    pub ns: NsKey,
    /// `Arc::as_ptr` of the arena it was merged into (`None`: the empty base of a first load).
    pub prev_ptr: Option<usize>,
    pub outcome: MergeOutcome,
    /// The pass's result, kept only when the guard could fire (the "Keep cached" re-merge).
    pub keep: Option<NamespaceResult>,
    /// The level the pass read at.
    pub level: Level,
    /// A namespace pass (it sets the namespace's state); a fragment does not.
    pub pass: bool,
    /// Objects to re-read (merge follow-ups, parents of renamed objects).
    pub follow: Vec<ObjectKey>,
    /// A top-level object was renamed: the namespace gets a follow-up pass.
    pub follow_ns: bool,
    /// The filters the pass read with: a result read under other filters than the engine's
    /// current ones is read again (review m-rr1).
    pub filters: Filters,
}

/// What a task answered.
pub(super) enum Output {
    /// The listing, the identity, and the other databases that could not be listed (unknown:
    /// their schemas keep their states; review RR-2).
    // ## m4-t17-mysql (named exception; T15 carry m4): the server's facts, when read.
    Namespaces(Vec<NsInfo>, Option<IdentityRead>, Vec<Name>, FactsRead),
    Merged(Box<Merged>),
    // ## m4-t22-state (named exception): the stamps of the owners that read `Changed`, and a
    // namespace validation's verdict (with the arena it validated).
    Staleness(
        Vec<ObjectKey>,
        Vec<Staleness>,
        Vec<(ObjectKey, hacivat_meta::StateMark)>,
    ),
    Validated(NsKey, Option<usize>, crate::freshness::NsVerdict),
    Failed(IntroError),
}

/// The database a task on `ns` connects to: PostgreSQL needs one connection per database;
/// SQLite's `temp` schema is read on the user's connection (alg/02 HV-INTRO-08 step 4).
pub(super) fn database_of(dbms: DbmsKind, ns: Option<&NsKey>) -> Option<Name> {
    let ns = ns?;
    match dbms {
        DbmsKind::Postgres if ns.kind != NsKind::Server => ns.path.first().cloned(),
        // ## m4-t22-state (named exception; review m-7): `temp` matched as `freshness` and T18
        // match it (ASCII-caseless), and always asked for by its canonical name.
        DbmsKind::Sqlite if ns.path.len() == 1 && ns.path[0].eq_ignore_ascii_case("temp") => {
            Some(Name::from("temp"))
        }
        _ => None,
    }
}

/// Milliseconds since the Unix epoch.
pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Runs `spec`; a cancelled task answers `Cancelled` whatever the introspector does.
pub(super) async fn run(ctx: Arc<WorkerCtx>, spec: Spec, cancel: CancellationToken) -> Output {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Output::Failed(IntroError::Cancelled),
        out = run_inner(&ctx, spec, &cancel) => out,
    }
}

async fn run_inner(ctx: &Arc<WorkerCtx>, spec: Spec, cancel: &CancellationToken) -> Output {
    // ## m4-t22-state (named exception): a state check names its database.
    let database = match &spec.work {
        Work::CheckState { database, .. } => database.clone(),
        w => database_of(ctx.dbms, w.ns()),
    };
    let mut restarted = false;
    let mut raced = false;
    loop {
        let mut conn = match acquire(ctx, database.clone(), cancel).await {
            Ok(c) => c,
            Err(e) => return Output::Failed(e.into()),
        };
        let read = read(ctx, &spec, &mut conn, cancel).await;
        // The connection holds a budget slot: give it back before anything else.
        drop(conn);
        ctx.meta.released();
        match read {
            // HV-INTRO-01 edge 5: reconnect once and restart the task from the beginning.
            Err(IntroError::Meta(MetaError::ConnectionLost)) if !restarted => restarted = true,
            // PG spike finding 8: a concurrent drop; retried once (log only).
            Err(IntroError::CatalogRace) if !raced => {
                raced = true;
                if let Some(ns) = spec.work.ns() {
                    let _ = ctx
                        .events
                        .send(EngineEvent::CatalogRaceRetried { ns: ns.clone() });
                }
            }
            Err(e) => return Output::Failed(e),
            Ok(Read::Namespaces(infos, identity, others, facts)) => {
                // The first connection is closed: list the other databases one connection at
                // a time (review RR-3).
                let (infos, unlisted) = list_others(ctx, infos, others, cancel).await;
                return Output::Namespaces(infos, identity, unlisted, facts);
            }
            Ok(Read::Staleness(keys, st, marks)) => return Output::Staleness(keys, st, marks),
            Ok(Read::Validated(v)) => {
                let ptr = spec.prev.as_ref().map(|p| Arc::as_ptr(p) as usize);
                let ns = spec.work.ns().cloned().unwrap_or_else(NsKey::server);
                return Output::Validated(ns, ptr, v);
            }
            Ok(Read::Result(r)) => return merge_result(ctx, &spec, r).await,
        }
    }
}

/// A metadata connection; a busy single-connection profile is asked again until it is idle.
async fn acquire(
    ctx: &WorkerCtx,
    database: Option<Name>,
    cancel: &CancellationToken,
) -> Result<MetaConn, MetaError> {
    loop {
        match ctx.meta.acquire(database.clone(), cancel).await {
            Err(MetaError::Busy) => {
                tokio::select! {
                    () = cancel.cancelled() => return Err(MetaError::Cancelled),
                    () = tokio::time::sleep(ctx.limits.busy_retry) => {}
                }
            }
            other => return other,
        }
    }
}

enum Read {
    /// The listing, the identity, the other databases still to list, and the server's facts
    /// (m4-t17-mysql).
    Namespaces(Vec<NsInfo>, Option<IdentityRead>, Vec<Name>, FactsRead),
    Result(NamespaceResult),
    // ## m4-t22-state (named exception)
    Staleness(
        Vec<ObjectKey>,
        Vec<Staleness>,
        Vec<(ObjectKey, hacivat_meta::StateMark)>,
    ),
    Validated(crate::freshness::NsVerdict),
}

async fn read(
    ctx: &WorkerCtx,
    spec: &Spec,
    conn: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<Read, IntroError> {
    match &spec.work {
        Work::ScopeRefresh => list(ctx, spec, conn).await,
        Work::Pass { ns, mode, level } => with_progress(
            sink(ctx, ns),
            ctx.intro.introspect(
                conn,
                ns,
                *mode,
                *level,
                spec.filters.for_pass(ns, spec.system),
                spec.prev.as_deref(),
                cancel,
            ),
        )
        .await
        .map(Read::Result),
        Work::Fragment { ns, keys, level } => with_progress(
            sink(ctx, ns),
            ctx.intro.fragment(
                conn,
                ns,
                keys,
                *level,
                spec.filters.for_pass(ns, spec.system),
                cancel,
            ),
        )
        .await
        .map(Read::Result),
        // ## m4-t22-state (named exception): `freshness::check_state` compares the stamps with
        // the snapshot's marks (Task 22); the introspector's own `check_state` has no marks.
        Work::CheckState { keys, .. } => {
            let snap = spec
                .snapshot
                .as_deref()
                .ok_or_else(|| IntroError::Protocol(Name::from("check_state")))?;
            let got = crate::freshness::check_state(&*ctx.intro, conn, snap, keys).await?;
            if got.staleness.len() != keys.len() {
                return Err(IntroError::Protocol(Name::from("check_state")));
            }
            Ok(Read::Staleness(keys.clone(), got.staleness, got.marks))
        }
        Work::Validate { ns } => {
            let Some(prev) = spec.prev.as_deref() else {
                return Err(IntroError::Protocol(Name::from("validate")));
            };
            crate::freshness::check_namespace(&*ctx.intro, conn, ns, prev)
                .await
                .map(Read::Validated)
        }
    }
}

// ## m4-t17-mysql (named exception; T15 carry m11)
/// The sink of a pass over `ns`: the introspector's own phase and counts replace the batch
/// count of `ns`'s entry in the engine's progress (the actor's next update writes the batch
/// count back; the pass reports again as it goes).
fn sink(ctx: &WorkerCtx, ns: &NsKey) -> ProgressSink {
    let progress = ctx.progress.clone();
    let ns = ns.clone();
    ProgressSink::new(move |phase, done, total| {
        progress.send_if_modified(|p| {
            let Some(t) = p.tasks.iter_mut().find(|t| t.ns == ns) else {
                return false;
            };
            let next = (phase, done, total);
            if (t.phase, t.done, t.total) == next {
                return false;
            }
            (t.phase, t.done, t.total) = next;
            true
        });
    })
}

/// The scope refresh on the profile's own connection: its database's namespaces, on connect the
/// server's identity checked against the cache (R-W3-3), and on PostgreSQL the other listed
/// databases the scope may select (`pg_namespace` is per database; review I-2), which
/// [`list_others`] reads after this connection is closed (review RR-3).
async fn list(ctx: &WorkerCtx, spec: &Spec, conn: &mut MetaConn) -> Result<Read, IntroError> {
    let infos = ctx.intro.list_namespaces(conn).await?;
    // ## m4-t17-mysql (T15 carry m4; review I-2): the facts of the server behind the
    // connection. A failed read is reported as such: the scheduler then publishes facts whose
    // `lower_case_table_names` is unknown, never the lctn-0 placeholder (a guess).
    let facts = match ctx.intro.server_facts(conn).await {
        Ok(Some(f)) => FactsRead::Read(Box::new(f)),
        Ok(None) => FactsRead::NotRead,
        Err(_) => FactsRead::Failed,
    };
    let Some(listing) = &spec.listing else {
        return Ok(Read::Namespaces(infos, None, Vec::new(), facts));
    };
    let identity = if listing.identity {
        Some(identity(ctx, conn).await)
    } else {
        None
    };
    let with_schemas: std::collections::BTreeSet<Name> = infos
        .iter()
        .filter(|i| i.key.kind == NsKind::Schema && i.key.path.len() == 2)
        .map(|i| i.key.path[0].clone())
        .collect();
    let others: Vec<Name> = infos
        .iter()
        .filter(|i| i.key.kind == NsKind::Database && i.key.path.len() == 1)
        .filter(|i| !with_schemas.contains(&i.key.path[0]))
        .filter(|i| {
            listing
                .scope
                .may_select_database(&i.key.path[0], i.system, &listing.current)
        })
        .map(|i| i.key.path[0].clone())
        .collect();
    Ok(Read::Namespaces(infos, identity, others, facts))
}

/// The schemas of each other database, each on its own connection, one at a time (the first
/// connection is already closed: no budget slot is held idle; review RR-3). A database that
/// cannot be listed (no `CONNECT`, a refused login) does not fail the refresh: it is answered
/// as unlisted (review RR-2). A cancel stops the loop; what was not listed is unlisted.
async fn list_others(
    ctx: &WorkerCtx,
    mut infos: Vec<NsInfo>,
    others: Vec<Name>,
    cancel: &CancellationToken,
) -> (Vec<NsInfo>, Vec<Name>) {
    let mut unlisted = Vec::new();
    for db in others {
        if cancel.is_cancelled() {
            unlisted.push(db);
            continue;
        }
        let found = match acquire(ctx, Some(db.clone()), cancel).await {
            Ok(mut other) => {
                let found = ctx.intro.list_namespaces(&mut other).await;
                drop(other);
                ctx.meta.released();
                found
            }
            Err(e) => Err(e.into()),
        };
        match found {
            Ok(found) => infos.extend(
                found
                    .into_iter()
                    .filter(|i| i.key.kind == NsKind::Schema && i.key.path.first() == Some(&db)),
            ),
            Err(_) => unlisted.push(db),
        }
    }
    (infos, unlisted)
}

/// The server's identity, checked against the cache when there is one (on the blocking pool).
async fn identity(ctx: &WorkerCtx, conn: &mut MetaConn) -> IdentityRead {
    let fp = match ctx.intro.identity(conn).await {
        Ok(ServerIdentity::Known(fp)) => fp,
        Ok(ServerIdentity::Unknown) | Err(_) => return IdentityRead::Unknown,
    };
    let Some(cache) = ctx.cache.clone() else {
        return IdentityRead::Known {
            fp: fp.0,
            same_as_cache: None,
        };
    };
    let checked = tokio::task::spawn_blocking(move || {
        cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .check_identity(fp)
    })
    .await;
    match checked {
        Ok(Ok(c)) => IdentityRead::Known {
            fp: fp.0,
            same_as_cache: Some(c == hacivat_meta::persist::IdentityCheck::Same),
        },
        // A cache that cannot answer is not trusted.
        _ => IdentityRead::Unknown,
    }
}

// ## m4-t30-gates (named exception in Task 15's file; T30 review I-2)
/// A system-mode read (the `40-system` sections) flags its objects `SYSTEM`; the namespace
/// root gets the flag too (a root update), so the schema sits in the explorer's Built-in group
/// and `CatalogView` reports `is_system`. A root update the read carries keeps its props.
fn flag_system_root(ns: &NsKey, r: &mut NamespaceResult) {
    let root_kind = ns.kind.root_kind();
    let root = r
        .changed
        .iter()
        .position(|c| c.subtree.root_obj().kind() == root_kind && c.subtree.len() == 1);
    let (obj, parent) = match root {
        Some(i) => {
            let c = r.changed.remove(i);
            (c.subtree.root_obj().clone(), c.parent)
        }
        None => (
            Obj::new(root_kind, Name::from(ns.root_name())),
            ObjectKey {
                kind: root_kind,
                path: ns.path.iter().cloned().collect(),
                server_id: None,
                overload_sig: None,
                raw_kind: None,
            },
        ),
    };
    r.changed.insert(
        0,
        ChangedObject {
            parent,
            subtree: Subtree::new(obj.add_flags(ObjFlags::SYSTEM)),
        },
    );
}
// ## m4-t30-gates

/// Filters `r`, fills the engine's fields and merges it into the task's `prev` (or the empty
/// base of a first load), on the blocking pool (a 160k-column merge is tens of milliseconds).
async fn merge_result(ctx: &Arc<WorkerCtx>, spec: &Spec, mut r: NamespaceResult) -> Output {
    let (ns, level, pass) = match &spec.work {
        Work::Pass { ns, level, .. } => (ns.clone(), *level, true),
        Work::Fragment { ns, level, .. } => (ns.clone(), *level, false),
        _ => return Output::Failed(IntroError::Protocol(Name::from("merge"))),
    };
    spec.filters.apply(&ns, spec.system, &mut r);
    // ## m4-t30-gates (named exception in Task 15's file; T30 review I-2)
    if matches!(
        spec.work,
        Work::Pass {
            mode: Mode::System,
            ..
        }
    ) {
        flag_system_root(&ns, &mut r);
    }
    // ## m4-t30-gates
    // ## m4-t22-state (named exception): stamps a state check read before this re-read become
    // the marks of the owners it re-read without a mark of their own.
    if !spec.marks.is_empty() {
        record_marks(&mut r, &spec.marks);
    }
    // ## m4-t22-state (named exception; fix round 2, R-W4-9)
    if !spec.kept.is_empty() {
        protect_kept(&mut r, &ns, &spec.kept);
    }
    // R-W2-1: the level the pass read and when (display only).
    r.level = level;
    if r.read_at_ms == 0 {
        r.read_at_ms = now_ms();
    }
    let prev = spec.prev.clone();
    let filters = spec.filters.clone();
    // A fragment read some objects, not the namespace: the namespace keeps its watermark and
    // load time (the merge takes both from the result), so its next incremental pass still
    // starts from where the last pass of the whole namespace did.
    if !pass && let Some(p) = &prev {
        r.new_watermark = p.meta().watermark.clone();
        r.read_at_ms = p.meta().loaded_at_ms;
    }
    let prev_ptr = prev.as_ref().map(|p| Arc::as_ptr(p) as usize);
    let policy = spec.policy;
    let dbms = ctx.dbms;
    let fold = spec.fold;
    let joined = tokio::task::spawn_blocking(move || {
        let base;
        let into: &NamespaceSnapshot = match &prev {
            Some(p) => p,
            None => {
                base = merge::base(ns.clone(), dbms, fold)
                    .map_err(|_| IntroError::Protocol(Name::from("base")))?;
                &base
            }
        };
        // The guard can only fire over at least `guard_min_objects` cached objects; only then
        // is the result kept for a "Keep cached" re-merge (D-039, I-5).
        let keep = (into.root().child_count() >= policy.guard_min_objects).then(|| r.clone());
        let outcome = merge::merge(Some(into), r, &policy)
            .map_err(|_| IntroError::Protocol(Name::from("merge")))?;
        let (follow, follow_ns) = follow_ups(&outcome);
        Ok::<_, IntroError>(Merged {
            ns,
            prev_ptr,
            outcome,
            keep,
            level,
            pass,
            follow,
            follow_ns,
            filters,
        })
    })
    .await;
    match joined {
        Ok(Ok(m)) => Output::Merged(Box::new(m)),
        Ok(Err(e)) => Output::Failed(e),
        Err(_) => Output::Failed(IntroError::Protocol(Name::from("merge"))),
    }
}

// ## m4-t22-state (named exception; fix round 2, R-W4-9)
/// Reports every kept object of `ns` existent in the existence sets of `r` that would decide it
/// (a complete set, or a scoped one covering it): the merge then deletes none of them, while
/// every other deletion, addition and change of the read applies.
fn protect_kept(r: &mut NamespaceResult, ns: &NsKey, kept: &[ObjectKey]) {
    for k in kept {
        let Some(rel) = k.path.strip_prefix(ns.path.as_slice()) else {
            continue;
        };
        let rel: Vec<Name> = rel.to_vec();
        let id = merge::Identity {
            path: rel.clone(),
            server_id: k.server_id,
            overload_sig: k.overload_sig.clone(),
            raw_kind: k.raw_kind.clone(),
            state: hacivat_meta::StateMark::None,
        };
        let ids = match r.existent.get_mut(&k.kind) {
            Some(merge::ExistSet::Ok(ids)) => ids,
            Some(merge::ExistSet::Scoped { scope, ids })
                if scope.iter().any(|s| rel.starts_with(s)) =>
            {
                ids
            }
            _ => continue,
        };
        if !ids.iter().any(|i| i.path == rel) {
            ids.push(id);
        }
    }
}

// ## m4-t22-state (named exception)
/// Sets `marks` on the top-level objects of `r` they name that the read left without a mark
/// (module docs of `freshness`, "Marks recorded on re-read").
fn record_marks(r: &mut NamespaceResult, marks: &[(ObjectKey, hacivat_meta::StateMark)]) {
    for c in &mut r.changed {
        let obj = c.subtree.root_obj();
        if obj.state() != hacivat_meta::StateMark::None {
            continue;
        }
        let mut path = c.parent.path.clone();
        path.push(obj.name().clone());
        let key = ObjectKey {
            kind: obj.kind(),
            path,
            server_id: obj.server_id(),
            overload_sig: obj.props().overload_sig().cloned(),
            raw_kind: obj.raw_kind().cloned(),
        };
        if let Some((_, m)) = marks.iter().find(|(k, _)| k.same_object(&key)) {
            c.subtree.set_root_state(*m);
        }
    }
}

/// What to re-read after a merge: the merge's own fragments (a child whose parent was not
/// placed, an object only the existence query listed), and the parent of every identity rename
/// (a half re-read name swap heals there; T10 carry). A top-level rename asks for a pass of the
/// namespace instead (its parent is the namespace root).
fn follow_ups(outcome: &MergeOutcome) -> (Vec<ObjectKey>, bool) {
    let next = outcome.next();
    let mut keys = outcome.report().fragments.clone();
    let mut ns = false;
    for (_, after, conf) in &outcome.delta().renamed {
        if !conf.is_identity() {
            continue;
        }
        let parent = next
            .find(after)
            .and_then(|i| next.obj(i))
            .and_then(|o| o.parent());
        match parent {
            Some(0) | None => ns = true,
            Some(p) => keys.extend(next.object_key(p)),
        }
    }
    keys.sort();
    keys.dedup();
    (keys, ns)
}
