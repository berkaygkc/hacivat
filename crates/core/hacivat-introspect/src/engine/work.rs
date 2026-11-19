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

use hacivat_meta::merge::{self, MergeOutcome, MergePolicy, NamespaceResult, Trigger};
use hacivat_meta::name::FoldRule;
use hacivat_meta::{Level, Name, NamespaceSnapshot, NsKey, NsKind, ObjectKey};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::{MetaConn, MetaError};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::filters::Filters;
use super::{EngineEvent, EngineLimits, MetaSource};
use crate::introspector::{IntroError, Introspector, Staleness};
use crate::pack::Mode;
use crate::scope::NsInfo;

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
    CheckState {
        keys: Vec<ObjectKey>,
    },
}

impl Work {
    /// The namespace the task reads and publishes (`None`: the scope refresh, a state check).
    pub fn ns(&self) -> Option<&NsKey> {
        match self {
            Work::Pass { ns, .. } | Work::Fragment { ns, .. } => Some(ns),
            Work::ScopeRefresh | Work::CheckState { .. } => None,
        }
    }
}

/// What the worker needs besides the task.
pub(super) struct WorkerCtx {
    pub meta: Arc<dyn MetaSource>,
    pub intro: Arc<dyn Introspector>,
    pub dbms: DbmsKind,
    pub fold: FoldRule,
    pub filters: Filters,
    pub limits: EngineLimits,
    pub events: broadcast::Sender<EngineEvent>,
}

/// One task as the worker gets it.
pub(super) struct Spec {
    pub work: Work,
    /// The namespace's arena when the task started (the merge's `prev`).
    pub prev: Option<Arc<NamespaceSnapshot>>,
    /// The namespace is a system one (the scope filter never applies there).
    pub system: bool,
    pub trigger: Trigger,
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
}

/// What a task answered.
pub(super) enum Output {
    Namespaces(Vec<NsInfo>),
    Merged(Box<Merged>),
    Staleness(Vec<ObjectKey>, Vec<Staleness>),
    Failed(IntroError),
}

/// The database a task on `ns` connects to: PostgreSQL needs one connection per database;
/// SQLite's `temp` schema is read on the user's connection (alg/02 HV-INTRO-08 step 4).
pub(super) fn database_of(dbms: DbmsKind, ns: Option<&NsKey>) -> Option<Name> {
    let ns = ns?;
    match dbms {
        DbmsKind::Postgres if ns.kind != NsKind::Server => ns.path.first().cloned(),
        DbmsKind::Sqlite if ns.path.len() == 1 && &*ns.path[0] == "temp" => {
            Some(ns.path[0].clone())
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
    let database = database_of(ctx.dbms, spec.work.ns());
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
            Ok(Read::Namespaces(infos)) => return Output::Namespaces(infos),
            Ok(Read::Staleness(keys, st)) => return Output::Staleness(keys, st),
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
    Namespaces(Vec<NsInfo>),
    Result(NamespaceResult),
    Staleness(Vec<ObjectKey>, Vec<Staleness>),
}

async fn read(
    ctx: &WorkerCtx,
    spec: &Spec,
    conn: &mut MetaConn,
    cancel: &CancellationToken,
) -> Result<Read, IntroError> {
    match &spec.work {
        Work::ScopeRefresh => ctx.intro.list_namespaces(conn).await.map(Read::Namespaces),
        Work::Pass { ns, mode, level } => ctx
            .intro
            .introspect(conn, ns, *mode, *level, spec.prev.as_deref(), cancel)
            .await
            .map(Read::Result),
        Work::Fragment { ns, keys, level } => ctx
            .intro
            .fragment(conn, ns, keys, *level, cancel)
            .await
            .map(Read::Result),
        Work::CheckState { keys } => {
            let st = ctx.intro.check_state(conn, keys).await?;
            if st.len() != keys.len() {
                return Err(IntroError::Protocol(Name::from("check_state")));
            }
            Ok(Read::Staleness(keys.clone(), st))
        }
    }
}

/// Filters `r`, fills the engine's fields and merges it into the task's `prev` (or the empty
/// base of a first load), on the blocking pool (a 160k-column merge is tens of milliseconds).
async fn merge_result(ctx: &Arc<WorkerCtx>, spec: &Spec, mut r: NamespaceResult) -> Output {
    let (ns, level, pass) = match &spec.work {
        Work::Pass { ns, level, .. } => (ns.clone(), *level, true),
        Work::Fragment { ns, level, .. } => (ns.clone(), *level, false),
        _ => return Output::Failed(IntroError::Protocol(Name::from("merge"))),
    };
    ctx.filters.apply(&ns, spec.system, &mut r);
    // R-W2-1: the level the pass read and when (display only).
    r.level = level;
    if r.read_at_ms == 0 {
        r.read_at_ms = now_ms();
    }
    let prev = spec.prev.clone();
    // A fragment read some objects, not the namespace: the namespace keeps its watermark and
    // load time (the merge takes both from the result), so its next incremental pass still
    // starts from where the last pass of the whole namespace did.
    if !pass && let Some(p) = &prev {
        r.new_watermark = p.meta().watermark.clone();
        r.read_at_ms = p.meta().loaded_at_ms;
    }
    let prev_ptr = prev.as_ref().map(|p| Arc::as_ptr(p) as usize);
    let policy = MergePolicy {
        trigger: spec.trigger,
        ..MergePolicy::default()
    };
    let dbms = ctx.dbms;
    let fold = ctx.fold;
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
        })
    })
    .await;
    match joined {
        Ok(Ok(m)) => Output::Merged(Box::new(m)),
        Ok(Err(e)) => Output::Failed(e),
        Err(_) => Output::Failed(IntroError::Protocol(Name::from("merge"))),
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
