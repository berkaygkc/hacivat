// ## m4-t01c-core (seam) / ## m4-t21-refresh (bodies)
//! The refresh-after-DDL coordinator (HV-INTRO-12; Task 21): per profile, applies a
//! [`RefreshPlan`] with the per-engine timing, keeps pending targets per session until
//! `COMMIT`/`ROLLBACK` (R-M4-42; exposed for M5's overlays, R-M4-11) and honours
//! `metadata.refresh_after_ddl`.
//!
//! ## m4-t21-refresh (ADR 0008 `## m4-t21 — refresh after DDL`)
//!
//! - **Input.** A run's sink (`hacivat_exec::audit::CatalogRefreshSink`, installed per run by
//!   Task 27) feeds a [`RunTracker`], which turns each finished unit into [`RefreshMsg`]s and
//!   pushes them into the profile's [`RefreshInbox`]: O(1) under a short lock, never awaited,
//!   so a slow coordinator never slows the run. The inbox holds [`INBOX_CAPACITY`] messages; a
//!   full one drops its **oldest** and counts it (the HV-EXEC-06 pattern). A dropped message
//!   is never a silent loss: its session's pending targets are refreshed at once and the
//!   session's whole database is refreshed now and again at its next commit (refresh more,
//!   never less).
//! - **What a unit means** ([`RunTracker`]): a successful unit's statements, with the
//!   session's namespace and search path read from the session's state **when the unit
//!   ended** (execution time) and the transaction state after it ([`TxnState`]); a failed,
//!   cancelled or skipped unit plans nothing, unless it may have committed DDL (a MySQL or
//!   SQLite batch stopped partway, an answer lost outside a transaction; fix round 1 I2).
//!   A transaction end comes from the server's transaction status, never the text: a unit
//!   that moved the status from in-transaction to idle committed, unless it was a rollback,
//!   the transaction had failed (PostgreSQL answers `COMMIT` there with `ROLLBACK`), the
//!   server's tag said `ROLLBACK`, or the unit failed; a `COMMIT`/`ROLLBACK` that left the
//!   status in a transaction (`AND CHAIN`, I1) ended one too; a lost connection after
//!   `COMMIT` is [`TxOutcome::Unknown`] (counts as committed). The
//!   names of `SAVEPOINT`, `RELEASE` and `ROLLBACK TO` come from the unit's text, read when the
//!   tracker is built ([`tx_control`]); only the folded name is kept.
//! - **Timing** ([`plan_refresh_in`]): targets of a statement that ran in a transaction on an
//!   engine with transactional DDL (PostgreSQL, SQLite) wait in the session's pending list;
//!   `COMMIT` (or an unknown end) refreshes them, `ROLLBACK` drops them, `ROLLBACK TO s` drops
//!   those after `s` (no compensating refresh: the metadata connection never saw them,
//!   R-M4-42). A `ROLLBACK TO` whose savepoint the coordinator never saw keeps everything.
//! - **Preference** (`metadata.refresh_after_ddl`): `always` refreshes; `never` marks the
//!   targets' namespaces stale (the explorer's badge) and refreshes nothing; `ask` marks them
//!   stale and announces [`CoordEvent::Ask`], answered by [`RefreshCoordinator::answer`].
//!   [`RefreshCoordinator::refresh_now`] ignores the preference (M7's Modify Object).
//! - **A refresh** lowers its targets on the catalog of that moment ([`lower`]), runs them on
//!   the engine at background priority, then checks the objects it refreshed
//!   ([`IntroHandle::check_state`], alg/02 step 6). A refresh that failed for want of a
//!   connection is **owed** and runs again on [`RefreshCoordinator::reconnected`] (edge case
//!   13: the pending list survives a reconnect after the commit was acknowledged).
//! - Transaction ends outside a run (the toolbar's Commit and Roll back, a closed session) are
//!   told by their caller: [`RefreshCoordinator::tx_ended`],
//!   [`RefreshCoordinator::session_closed`].

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures::future::join_all;
use hacivat_meta::NsKey;
use hacivat_meta::merge::Trigger;
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaError;
use hacivat_session::session::{
    ExecErrorKind, SessionId, SessionSnapshot, TxOutcome, UnitEnd, UnitId,
};
use hacivat_settings::schema::RefreshAfterDdl;
use sql_dialect::DialectId;
use sql_safety::{StmtKind, StmtReport};
use tokio::sync::{Notify, broadcast, watch};
use tokio_util::sync::CancellationToken;

use crate::ddl_effects::{
    ExecutedStmt, RefreshPlan, SessionStateAtExec, Target, TxnState, lower, normalize,
    plan_refresh_in,
};
use crate::engine::IntroHandle;
use crate::introspector::{IntroCaps, IntroError};
use crate::ticket::RefreshTicket;

/// The messages an inbox holds before it drops its oldest.
pub const INBOX_CAPACITY: usize = 1024;

/// What a run tells its profile's coordinator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshMsg {
    /// Statements that ran successfully, with the session's state when they ran.
    Executed {
        session: SessionId,
        stmts: Vec<ExecutedStmt>,
        state: SessionStateAtExec,
        txn: TxnState,
    },
    /// `SAVEPOINT name` succeeded (the name folded by the dialect's rule).
    Savepoint { session: SessionId, name: Arc<str> },
    /// `RELEASE SAVEPOINT name` succeeded.
    Released { session: SessionId, name: Arc<str> },
    /// The session's transaction (or part of it) ended.
    TxEnded {
        session: SessionId,
        outcome: TxOutcome,
    },
}

impl RefreshMsg {
    fn session(&self) -> SessionId {
        match self {
            RefreshMsg::Executed { session, .. }
            | RefreshMsg::Savepoint { session, .. }
            | RefreshMsg::Released { session, .. }
            | RefreshMsg::TxEnded { session, .. } => *session,
        }
    }
}

/// A profile's bounded, drop-oldest message queue (the module docs). Cheap to clone.
#[derive(Clone)]
pub struct RefreshInbox {
    inner: Arc<InboxInner>,
}

struct InboxInner {
    queue: Mutex<Queue>,
    notify: Notify,
    dropped: AtomicU64,
    cap: usize,
}

#[derive(Default)]
struct Queue {
    msgs: VecDeque<RefreshMsg>,
    /// The sessions whose messages were dropped, with the last state (and transaction state) a
    /// dropped message held.
    lost: BTreeMap<SessionId, Option<Lost>>,
}

/// What a dropped `Executed` message said about its session.
type Lost = (SessionStateAtExec, TxnState);

impl std::fmt::Debug for RefreshInbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshInbox")
            .field("dropped", &self.dropped())
            .finish_non_exhaustive()
    }
}

impl RefreshInbox {
    /// An inbox of `cap` messages (at least one).
    pub fn with_capacity(cap: usize) -> Self {
        RefreshInbox {
            inner: Arc::new(InboxInner {
                queue: Mutex::new(Queue::default()),
                notify: Notify::new(),
                dropped: AtomicU64::new(0),
                cap: cap.max(1),
            }),
        }
    }

    /// Queues `msg`; when full, drops the oldest message and counts it. Never waits.
    pub fn push(&self, msg: RefreshMsg) {
        {
            let mut q = self.lock();
            if q.msgs.len() >= self.inner.cap
                && let Some(old) = q.msgs.pop_front()
            {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
                let session = old.session();
                let state = match old {
                    RefreshMsg::Executed { state, txn, .. } => Some((state, txn)),
                    _ => None,
                };
                let slot = q.lost.entry(session).or_default();
                if state.is_some() {
                    *slot = state;
                }
            }
            q.msgs.push_back(msg);
        }
        self.inner.notify.notify_one();
    }

    /// Messages dropped so far.
    pub fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    /// Messages waiting.
    pub fn len(&self) -> usize {
        self.lock().msgs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A copy of the messages waiting, oldest first (tests and diagnostics).
    pub fn queued(&self) -> Vec<RefreshMsg> {
        self.lock().msgs.iter().cloned().collect()
    }

    fn take(&self) -> (Vec<RefreshMsg>, Vec<(SessionId, Option<Lost>)>) {
        let mut q = self.lock();
        let msgs = q.msgs.drain(..).collect();
        let lost = std::mem::take(&mut q.lost).into_iter().collect();
        (msgs, lost)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.inner
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the coordinator tells its listeners (codes only: the UI maps them to Fluent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordEvent {
    /// `metadata.refresh_after_ddl = "ask"`: refresh `count` targets? Answer with
    /// [`RefreshCoordinator::answer`] (`intro-refresh-after-ddl-ask`).
    Ask { id: u64, count: usize },
    /// These namespaces were marked stale instead of refreshed (`never`, or `ask` until the
    /// answer). `scope`: the namespace list changed too (a schema or database created, dropped
    /// or renamed); every loaded schema list was marked stale (fix round 1 I4).
    Stale { namespaces: Vec<NsKey>, scope: bool },
    /// Messages were dropped by a full inbox (`total` so far); the sessions concerned were
    /// refreshed widely instead.
    Dropped { total: u64 },
}

/// One entry of a session's pending list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Targets(Vec<Target>),
    Savepoint(Arc<str>),
}

/// One profile's coordinator (the module docs). Dropping it stops its task.
#[derive(Debug)]
pub struct RefreshCoordinator {
    inner: Arc<Inner>,
    stop: CancellationToken,
}

struct Inner {
    handle: IntroHandle,
    caps: IntroCaps,
    runtime: tokio::runtime::Handle,
    pref: Mutex<RefreshAfterDdl>,
    sessions: Mutex<HashMap<SessionId, Vec<Entry>>>,
    owed: Arc<Mutex<Vec<Target>>>,
    /// At most one unanswered question: a new one takes over the targets of the old (fix round
    /// 1 m2).
    asks: Mutex<BTreeMap<u64, Vec<Target>>>,
    next_ask: AtomicU64,
    events: broadcast::Sender<CoordEvent>,
    inbox: RefreshInbox,
    draining: Mutex<()>,
    /// Refreshes handed to the engine so far (diagnostics; the Docker tests read it).
    applied: AtomicU64,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshCoordinator")
            .field("dbms", &self.caps.dbms)
            .field("inbox", &self.inbox)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Drop for RefreshCoordinator {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl RefreshCoordinator {
    /// The coordinator of the profile whose engine `handle` is (`dbms` its engine), with the
    /// preference `pref`; its task runs on `runtime` (the `CoreRuntime`).
    pub fn new(
        handle: IntroHandle,
        dbms: DbmsKind,
        pref: RefreshAfterDdl,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let inner = Arc::new(Inner {
            handle,
            caps: IntroCaps::planned(dbms),
            runtime: runtime.clone(),
            pref: Mutex::new(pref),
            sessions: Mutex::new(HashMap::new()),
            owed: Arc::new(Mutex::new(Vec::new())),
            asks: Mutex::new(BTreeMap::new()),
            next_ask: AtomicU64::new(1),
            events: broadcast::channel(64).0,
            inbox: RefreshInbox::with_capacity(INBOX_CAPACITY),
            draining: Mutex::new(()),
            applied: AtomicU64::new(0),
        });
        let stop = CancellationToken::new();
        let task = inner.clone();
        let stopped = stop.clone();
        runtime.spawn(async move {
            loop {
                tokio::select! {
                    () = stopped.cancelled() => break,
                    () = task.inbox.inner.notify.notified() => task.drain(),
                }
            }
        });
        RefreshCoordinator { inner, stop }
    }

    /// The inbox runs feed (through their [`RunTracker`]s).
    pub fn inbox(&self) -> RefreshInbox {
        self.inner.inbox.clone()
    }

    /// The coordinator's events.
    pub fn events(&self) -> broadcast::Receiver<CoordEvent> {
        self.inner.events.subscribe()
    }

    /// Changes `metadata.refresh_after_ddl`.
    pub fn set_preference(&self, pref: RefreshAfterDdl) {
        *lock(&self.inner.pref) = pref;
    }

    /// Takes the plan of a run on `session`: its `now` part per the preference (the ticket
    /// resolves when that refresh is done), its `pending` part kept until the transaction ends.
    pub fn submit(&self, session: SessionId, plan: RefreshPlan) -> RefreshTicket {
        let mut out = Vec::new();
        self.inner.submit_into(session, plan, &mut out);
        self.inner.deliver(out)
    }

    /// `session`'s transaction ended: its pending targets are refreshed
    /// (`TxOutcome::counts_as_committed`) or dropped; `RolledBackTo` drops those after the
    /// savepoint.
    pub fn tx_ended(&self, session: SessionId, outcome: TxOutcome) -> RefreshTicket {
        let mut out = Vec::new();
        self.inner.tx_ended_into(session, outcome, &mut out);
        self.inner.deliver(out)
    }

    /// How many refreshes the coordinator has handed to the engine (diagnostics).
    pub fn applied(&self) -> u64 {
        self.inner.applied.load(Ordering::Relaxed)
    }

    /// `session` closed: its open transaction was rolled back; its pending targets go.
    pub fn session_closed(&self, session: SessionId) {
        lock(&self.inner.sessions).remove(&session);
    }

    /// The targets waiting for `session`'s `COMMIT` (R-M4-11: M5's overlays read them), widened
    /// as the commit will refresh them (alg/02 step 3 across the transaction's units).
    pub fn pending(&self, session: SessionId) -> Vec<Target> {
        lock(&self.inner.sessions)
            .get(&session)
            .map(|list| normalize(flatten(list)))
            .unwrap_or_default()
    }

    /// Refreshes `targets` now, whatever the preference (M7's Modify Object and source edit).
    pub fn refresh_now(&self, targets: Vec<Target>) -> RefreshTicket {
        self.inner.apply(targets)
    }

    /// Answers the question [`CoordEvent::Ask`] `id`: refresh, or leave the targets stale.
    pub fn answer(&self, id: u64, refresh: bool) -> RefreshTicket {
        let targets = lock(&self.inner.asks).remove(&id);
        match targets {
            Some(t) if refresh => self.inner.apply(t),
            _ => self.inner.ready(),
        }
    }

    /// The profile reconnected: the refreshes owed since a lost connection run now.
    pub fn reconnected(&self) -> RefreshTicket {
        let owed = std::mem::take(&mut *lock(&self.inner.owed));
        self.inner.apply(owed)
    }

    /// The refreshes owed for want of a connection.
    pub fn owed(&self) -> Vec<Target> {
        lock(&self.inner.owed).clone()
    }

    /// Processes what the inbox holds now (the coordinator's task does it on its own; a test
    /// may call it to wait for nothing).
    pub fn drain(&self) {
        self.inner.drain();
    }
}

/// Every target of a pending list, in order.
fn flatten(list: &[Entry]) -> Vec<Target> {
    list.iter()
        .flat_map(|e| match e {
            Entry::Targets(t) => t.clone(),
            Entry::Savepoint(_) => Vec::new(),
        })
        .collect()
}

/// Whether a refresh that failed with `e` is owed until the profile reconnects.
fn owed_on(e: &IntroError) -> bool {
    matches!(
        e,
        IntroError::Timeout
            | IntroError::Meta(
                MetaError::Unavailable
                    | MetaError::Busy
                    | MetaError::ConnectionLost
                    | MetaError::Timeout
            )
    )
}

impl Inner {
    fn ready(&self) -> RefreshTicket {
        RefreshTicket::ready(Ok(self.handle.catalog().current().generation))
    }

    fn same_savepoint(&self, a: &str, b: &str) -> bool {
        match self.caps.dbms {
            DbmsKind::Postgres => a == b,
            _ => a.eq_ignore_ascii_case(b),
        }
    }

    fn drain(&self) {
        // One drain at a time, so a session's messages are handled in the order they came.
        let _one = lock(&self.draining);
        let (msgs, lost) = self.inbox.take();
        // Everything this drain refreshes, delivered once and widened together (fix round 1
        // I3: a script's units arrive as separate messages).
        let mut out = Vec::new();
        for (session, state) in lost {
            self.lost_into(session, state, &mut out);
        }
        for msg in msgs {
            match msg {
                RefreshMsg::Executed {
                    session,
                    stmts,
                    state,
                    txn,
                } => {
                    let snap = self.handle.catalog().current();
                    let plan = plan_refresh_in(&stmts, &state, txn, &self.caps, Some(&snap));
                    self.submit_into(session, plan, &mut out);
                }
                RefreshMsg::Savepoint { session, name } => {
                    lock(&self.sessions)
                        .entry(session)
                        .or_default()
                        .push(Entry::Savepoint(name));
                }
                RefreshMsg::Released { session, name } => {
                    let mut sessions = lock(&self.sessions);
                    if let Some(list) = sessions.get_mut(&session)
                        && let Some(at) = list.iter().rposition(
                            |e| matches!(e, Entry::Savepoint(n) if self.same_savepoint(n, &name)),
                        )
                    {
                        // RELEASE also destroys every savepoint made after it; targets stay.
                        let tail: Vec<Entry> = list
                            .drain(at..)
                            .filter(|e| matches!(e, Entry::Targets(_)))
                            .collect();
                        list.extend(tail);
                    }
                }
                RefreshMsg::TxEnded { session, outcome } => {
                    self.tx_ended_into(session, outcome, &mut out);
                }
            }
        }
        // Dropping a ticket never cancels its refresh (single-flight work).
        drop(self.deliver(out));
    }

    /// Messages of `session` were dropped: refresh its pending targets and its database now,
    /// and, when the dropped state was inside a transaction, again at its next commit (the
    /// module docs; fix round 1 m3).
    fn lost_into(&self, session: SessionId, state: Option<Lost>, out: &mut Vec<Target>) {
        let pending = lock(&self.sessions)
            .remove(&session)
            .map(|l| flatten(&l))
            .unwrap_or_default();
        out.extend(pending);
        if let Some((state, txn)) = state {
            let db = state.current.path.first().cloned().unwrap_or_default();
            let wide = vec![Target::Database(db), Target::ScopeRefresh];
            out.extend(wide.clone());
            if txn == TxnState::InTransaction {
                lock(&self.sessions)
                    .entry(session)
                    .or_default()
                    .push(Entry::Targets(wide));
            }
        }
        let _ = self.events.send(CoordEvent::Dropped {
            total: self.inbox.dropped(),
        });
    }

    /// Keeps `plan`'s pending part for `session` and adds its `now` part to `out`.
    fn submit_into(&self, session: SessionId, plan: RefreshPlan, out: &mut Vec<Target>) {
        if !plan.pending.is_empty() {
            lock(&self.sessions)
                .entry(session)
                .or_default()
                .push(Entry::Targets(plan.pending));
        }
        out.extend(plan.now);
    }

    /// Ends `session`'s transaction (or part of it): what it commits goes into `out`.
    fn tx_ended_into(&self, session: SessionId, outcome: TxOutcome, out: &mut Vec<Target>) {
        if let TxOutcome::RolledBackTo(name) = &outcome {
            let mut sessions = lock(&self.sessions);
            if let Some(list) = sessions.get_mut(&session)
                && let Some(at) = list
                    .iter()
                    .rposition(|e| matches!(e, Entry::Savepoint(n) if self.same_savepoint(n, name)))
            {
                // The savepoint stays; what ran after it is undone.
                list.truncate(at + 1);
            }
            return;
        }
        let list = lock(&self.sessions).remove(&session).unwrap_or_default();
        if outcome.counts_as_committed() {
            out.extend(flatten(&list));
        }
    }

    /// Hands `targets` over per the preference, widened together first (alg/02 step 3 across
    /// units and messages; fix round 1 I3).
    fn deliver(&self, targets: Vec<Target>) -> RefreshTicket {
        let targets = normalize(targets);
        if targets.is_empty() {
            return self.ready();
        }
        let pref = *lock(&self.pref);
        match pref {
            RefreshAfterDdl::Always => self.apply(targets),
            RefreshAfterDdl::Never => {
                self.mark_stale(&targets);
                self.ready()
            }
            RefreshAfterDdl::Ask => {
                self.mark_stale(&targets);
                let id = self.next_ask.fetch_add(1, Ordering::Relaxed);
                // One question at a time: an unanswered one is folded into the new one, so the
                // list never grows without bound (fix round 1 m2).
                let mut asks = lock(&self.asks);
                let mut all: Vec<Target> =
                    std::mem::take(&mut *asks).into_values().flatten().collect();
                all.extend(targets);
                let all = normalize(all);
                let count = all.len();
                asks.insert(id, all);
                drop(asks);
                let _ = self.events.send(CoordEvent::Ask { id, count });
                self.ready()
            }
        }
    }

    /// Marks the namespaces `targets` reach stale, and the namespace lists when a target is the
    /// scope (alg/02 edge 14; fix round 1 I4). Loading namespaces are published stale when
    /// their pass ends (m5).
    fn mark_stale(&self, targets: &[Target]) {
        let snap = self.handle.catalog().current();
        let scope = targets.contains(&Target::ScopeRefresh);
        let mut namespaces: Vec<NsKey> = lower(targets, &snap)
            .into_iter()
            .filter_map(|t| match t {
                Target::Namespace(ns) | Target::Objects { ns, .. } => Some(ns),
                _ => None,
            })
            .collect();
        namespaces.sort();
        namespaces.dedup();
        if namespaces.is_empty() && !scope {
            return;
        }
        self.handle.mark_stale(namespaces.clone(), scope);
        let _ = self.events.send(CoordEvent::Stale { namespaces, scope });
    }

    /// Refreshes `targets` on the engine, then checks the refreshed objects (the module docs).
    fn apply(&self, targets: Vec<Target>) -> RefreshTicket {
        if targets.is_empty() {
            return self.ready();
        }
        self.applied.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = RefreshTicket::pending();
        let handle = self.handle.clone();
        let owed = self.owed.clone();
        self.runtime.spawn(async move {
            reply.send(run(handle, targets, owed).await);
        });
        ticket
    }
}

/// Refreshes `targets` on `handle` and checks the refreshed objects; on a failure for want of
/// a connection, `targets` are owed until the profile reconnects.
#[allow(
    clippy::result_large_err,
    reason = "IntroError carries the server's message by value; one per refresh"
)]
async fn run(
    handle: IntroHandle,
    targets: Vec<Target>,
    owed: Arc<Mutex<Vec<Target>>>,
) -> Result<u64, IntroError> {
    let snap = handle.catalog().current();
    let lowered = lower(&targets, &snap);
    let mut generation = snap.generation;
    if lowered.is_empty() {
        return Ok(generation);
    }
    let results = join_all(
        lowered
            .iter()
            .map(|t| handle.refresh(t.clone(), Trigger::Background)),
    )
    .await;
    for r in results {
        match r {
            Ok(g) => generation = generation.max(g),
            Err(e) => {
                if owed_on(&e) {
                    lock(&owed).extend(targets);
                }
                return Err(e);
            }
        }
    }
    let keys: Vec<_> = lowered
        .into_iter()
        .flat_map(|t| match t {
            Target::Objects { keys, .. } => keys,
            _ => Vec::new(),
        })
        .collect();
    if !keys.is_empty()
        && let Ok(g) = handle.check_state(keys).await
    {
        generation = generation.max(g);
    }
    Ok(generation)
}

// ---- runs ------------------------------------------------------------------------------------

/// A transaction status, as the run reports it (`hacivat_driver::TxStatus`, mapped by the
/// sink: this crate does not see the driver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TxPhase {
    #[default]
    Idle,
    InTx,
    /// A failed transaction: only a rollback is accepted (PostgreSQL).
    Failed,
}

/// A savepoint statement and its name, folded by the dialect's rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxControl {
    Savepoint(Arc<str>),
    Release(Arc<str>),
    RollbackTo(Arc<str>),
    /// PostgreSQL `PREPARE TRANSACTION 'id'`: the transaction leaves the session, neither
    /// committed nor rolled back (its end counts as unknown: refresh more, never less).
    PrepareTransaction,
    /// PostgreSQL `COMMIT PREPARED 'id'`: a prepared transaction of unknown content commits
    /// (fix round 1 m4: the session's whole database is refreshed).
    CommitPrepared,
}

/// The savepoint statement `text` is (`SAVEPOINT s`, `RELEASE [SAVEPOINT] s`, `ROLLBACK [WORK
/// | TRANSACTION] TO [SAVEPOINT] s`), if it is one. Keywords compare ASCII-case-insensitively;
/// comments and SQL whitespace between words are skipped; the name is one identifier, quoted or
/// not, read and folded as the dialect does.
pub fn tx_control(d: DialectId, text: &str) -> Option<TxControl> {
    let (first, mut rest) = take_word(text)?;
    let control: fn(Arc<str>) -> TxControl = match first.as_str() {
        "PREPARE" => {
            return take_word(rest)
                .is_some_and(|(w, _)| w == "TRANSACTION")
                .then_some(TxControl::PrepareTransaction);
        }
        "COMMIT" => {
            return take_word(rest)
                .is_some_and(|(w, _)| w == "PREPARED")
                .then_some(TxControl::CommitPrepared);
        }
        "SAVEPOINT" => TxControl::Savepoint,
        "RELEASE" => {
            rest = optional_word(rest, &["SAVEPOINT"]);
            TxControl::Release
        }
        "ROLLBACK" => {
            rest = optional_word(rest, &["WORK", "TRANSACTION"]);
            let (to, r) = take_word(rest)?;
            if to != "TO" {
                return None;
            }
            rest = optional_word(r, &["SAVEPOINT"]);
            TxControl::RollbackTo
        }
        _ => return None,
    };
    let name = skip_space(rest).trim_end_matches(|c: char| c == ';' || c.is_ascii_whitespace());
    let q = sql_ident::parse_qualified(d, name)?;
    let [ident] = q.parts.as_slice() else {
        return None;
    };
    Some(control(sql_ident::fold(d, &ident.text, ident.quoted)))
}

/// The next bare word of `s` (after whitespace and comments), ASCII-uppercased, and the rest.
fn take_word(s: &str) -> Option<(String, &str)> {
    let s = skip_space(s);
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    (end > 0).then(|| (s[..end].to_ascii_uppercase(), &s[end..]))
}

/// `s` after one of the optional keywords `words`, unless that word is all that is left (then
/// it is the name: `RELEASE savepoint` releases a savepoint named `savepoint`).
fn optional_word<'a>(s: &'a str, words: &[&str]) -> &'a str {
    match take_word(s) {
        Some((w, rest))
            if words.contains(&w.as_str())
                && !skip_space(rest)
                    .trim_end_matches(|c: char| c == ';' || c.is_ascii_whitespace())
                    .is_empty() =>
        {
            rest
        }
        _ => s,
    }
}

/// `s` after SQL whitespace and comments (`-- …`, `/* … */`).
fn skip_space(mut s: &str) -> &str {
    loop {
        let t = s.trim_start_matches([' ', '\t', '\n', '\r', '\x0c']);
        if let Some(r) = t.strip_prefix("--") {
            s = r.find('\n').map_or("", |i| &r[i + 1..]);
        } else if let Some(r) = t.strip_prefix("/*") {
            s = r.find("*/").map_or("", |i| &r[i + 2..]);
        } else {
            return t;
        }
    }
}

/// What the tracker knows of one unit of a run (built before the run starts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedUnit {
    pub id: UnitId,
    /// Its statements; `None` when the classifier gave no report (the run is opaque).
    pub stmts: Option<Vec<ExecutedStmt>>,
    /// The kind of its last statement (a transaction end reads it).
    pub kind: Option<StmtKind>,
    pub control: Option<TxControl>,
}

impl TrackedUnit {
    /// Unit `id` with the classifier's `reports` (if any) of its `text`, which is read only
    /// for a savepoint name and never kept.
    pub fn new(id: UnitId, reports: Option<&[StmtReport]>, d: DialectId, text: &str) -> Self {
        let stmts = reports
            .filter(|r| !r.is_empty())
            .map(|r| r.iter().map(ExecutedStmt::from_report).collect::<Vec<_>>());
        let kind = reports.and_then(|r| r.last()).map(|r| r.kind);
        // Only the first words are read; the text is never kept.
        let control = tx_control(d, text);
        TrackedUnit {
            id,
            stmts,
            kind,
            control,
        }
    }
}

/// One run's view for its coordinator (the module docs): fed every run event the sink sees, in
/// order, on the session's actor task. Every call does constant work apart from copying a
/// finished unit's statements into its message, and never waits.
#[derive(Debug)]
pub struct RunTracker {
    session: SessionId,
    dbms: DbmsKind,
    units: HashMap<UnitId, TrackedUnit>,
    state: watch::Receiver<SessionSnapshot>,
    status: TxPhase,
    /// The status when the open unit started.
    start_status: TxPhase,
    /// The status change the open unit reported, if any.
    change: Option<(TxPhase, TxPhase)>,
    /// The open unit's server tag said `ROLLBACK`.
    rollback_tag: bool,
    open: Option<UnitId>,
    inbox: RefreshInbox,
}

impl RunTracker {
    /// The tracker of a run on `session` (of engine `dbms`) whose units are `units`; `state` is
    /// the session's state, `status` its transaction status when the run starts.
    pub fn new(
        session: SessionId,
        dbms: DbmsKind,
        units: Vec<TrackedUnit>,
        state: watch::Receiver<SessionSnapshot>,
        status: TxPhase,
        inbox: RefreshInbox,
    ) -> Self {
        RunTracker {
            session,
            dbms,
            units: units.into_iter().map(|u| (u.id, u)).collect(),
            state,
            status,
            start_status: status,
            change: None,
            rollback_tag: false,
            open: None,
            inbox,
        }
    }

    pub fn unit_started(&mut self, unit: UnitId) {
        self.open = Some(unit);
        self.start_status = self.status;
        self.change = None;
        self.rollback_tag = false;
    }

    /// The transaction status moved. Between units (a rollback the user chose after a failed
    /// statement) a move to idle ends the transaction at once.
    pub fn tx_changed(&mut self, from: TxPhase, to: TxPhase) {
        self.status = to;
        if self.open.is_some() {
            let first = self.change.map_or(from, |(f, _)| f);
            self.change = Some((first, to));
        } else if from != TxPhase::Idle && to == TxPhase::Idle {
            let outcome = if from == TxPhase::Failed {
                TxOutcome::RolledBack
            } else {
                TxOutcome::Unknown
            };
            self.send(RefreshMsg::TxEnded {
                session: self.session,
                outcome,
            });
        }
    }

    /// The server's tag of the open unit (`ROLLBACK` for PostgreSQL's `COMMIT` of a failed
    /// transaction).
    pub fn update_count(&mut self, unit: UnitId, tag: Option<&str>) {
        if self.open == Some(unit) && tag.is_some_and(|t| t.eq_ignore_ascii_case("ROLLBACK")) {
            self.rollback_tag = true;
        }
    }

    /// A unit ended (the module docs). Only the finished unit's statements are copied, into
    /// its message (m7).
    pub fn unit_ended(&mut self, unit: UnitId, end: &UnitEnd) {
        let change = if self.open == Some(unit) {
            self.change.take()
        } else {
            None
        };
        self.open = None;
        let start = self.start_status;
        let Some(u) = self.units.get(&unit) else {
            return;
        };
        let (kind, control) = (u.kind, u.control.clone());
        let succeeded = matches!(end, UnitEnd::Succeeded { .. });
        let lost = matches!(
            end,
            UnitEnd::Failed(e) if matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
        );
        let lost_unknown = matches!(
            end,
            UnitEnd::Failed(e) if e.kind == ExecErrorKind::ConnectionLost { outcome_unknown: true }
        );
        let pg = self.dbms == DbmsKind::Postgres;
        let transactional_ddl = matches!(self.dbms, DbmsKind::Postgres | DbmsKind::Sqlite);
        // Fix round 1 (I2): a unit that did not succeed may still have committed DDL: a batch
        // run as one on an engine whose statements commit one by one (MySQL/MariaDB implicit
        // commit, SQLite autocommit) that failed or was cancelled partway, and a statement
        // whose answer was lost outside a transaction (or on an engine without transactional
        // DDL). Refresh more, never less.
        let batch = u.stmts.as_ref().is_none_or(|s| s.len() > 1);
        let maybe_committed = !succeeded
            && !matches!(end, UnitEnd::Skipped)
            && ((batch && !pg && !lost)
                || (lost_unknown && (start == TxPhase::Idle || !transactional_ddl)));
        // m4: `COMMIT PREPARED` commits a prepared transaction of unknown content.
        let commit_prepared = succeeded && control == Some(TxControl::CommitPrepared);
        if succeeded || maybe_committed {
            let stmts = if commit_prepared {
                vec![ExecutedStmt::unanalysed()]
            } else {
                u.stmts
                    .clone()
                    .unwrap_or_else(|| vec![ExecutedStmt::unanalysed()])
            };
            let catalog = stmts.iter().any(|s| !s.analysed || !s.ddl.is_empty())
                || stmts.iter().any(|s| {
                    matches!(
                        s.kind,
                        StmtKind::CreateTable
                            | StmtKind::CreateOther
                            | StmtKind::Alter
                            | StmtKind::Drop
                            | StmtKind::DropOwned
                            | StmtKind::Grant
                            | StmtKind::Revoke
                            | StmtKind::Other
                            | StmtKind::Explain { analyze: true }
                    )
                });
            if catalog {
                // A session that does not know its namespace: the whole run is opaque (every
                // loaded namespace, refresh more, never less).
                let (state, stmts) = match self.state_now() {
                    Some(s) => (s, stmts),
                    None => (
                        SessionStateAtExec {
                            current: NsKey::server(),
                            search_path: Vec::new(),
                        },
                        vec![ExecutedStmt::unanalysed()],
                    ),
                };
                // A lost connection ended any transaction; what may have committed did so
                // outside one.
                let txn = if self.status == TxPhase::Idle || lost {
                    TxnState::AutoCommit
                } else {
                    TxnState::InTransaction
                };
                self.send(RefreshMsg::Executed {
                    session: self.session,
                    stmts,
                    state,
                    txn,
                });
            }
        }
        let ended = change.is_some_and(|(from, to)| from != TxPhase::Idle && to == TxPhase::Idle);
        // A savepoint means something only inside a transaction.
        if succeeded && !ended && self.status != TxPhase::Idle {
            match &control {
                Some(TxControl::Savepoint(n)) => self.send(RefreshMsg::Savepoint {
                    session: self.session,
                    name: n.clone(),
                }),
                Some(TxControl::Release(n)) => self.send(RefreshMsg::Released {
                    session: self.session,
                    name: n.clone(),
                }),
                Some(TxControl::RollbackTo(n)) => self.send(RefreshMsg::TxEnded {
                    session: self.session,
                    outcome: TxOutcome::RolledBackTo(n.clone()),
                }),
                Some(TxControl::PrepareTransaction | TxControl::CommitPrepared) | None => {}
            }
        }
        // The `COMMIT` was sent and the connection went before its answer: maybe committed.
        let commit_unknown = kind == Some(StmtKind::Commit) && lost_unknown;
        // Fix round 1 (I1): `COMMIT AND CHAIN` / `ROLLBACK AND CHAIN` (and a MySQL `COMMIT`
        // with `autocommit` off) end the transaction and open the next one at once: the status
        // stays in a transaction, yet the transaction ended.
        let chained = succeeded
            && !ended
            && start != TxPhase::Idle
            && self.status != TxPhase::Idle
            && matches!(kind, Some(StmtKind::Commit | StmtKind::Rollback))
            && !matches!(
                control,
                Some(TxControl::RollbackTo(_) | TxControl::PrepareTransaction)
            );
        let outcome = if ended {
            let (from, _) = change.unwrap_or_default();
            Some(
                if commit_unknown || (succeeded && control == Some(TxControl::PrepareTransaction)) {
                    TxOutcome::Unknown
                } else if !succeeded
                    || from == TxPhase::Failed
                    || self.rollback_tag
                    || kind == Some(StmtKind::Rollback)
                {
                    TxOutcome::RolledBack
                } else {
                    TxOutcome::Committed
                },
            )
        } else if chained {
            Some(
                if kind == Some(StmtKind::Rollback) || start == TxPhase::Failed || self.rollback_tag
                {
                    TxOutcome::RolledBack
                } else {
                    TxOutcome::Committed
                },
            )
        } else if lost && self.status != TxPhase::Idle {
            // The connection went with the transaction open: the server rolled it back, unless
            // the lost statement was the `COMMIT` itself.
            self.status = TxPhase::Idle;
            Some(if commit_unknown {
                TxOutcome::Unknown
            } else {
                TxOutcome::RolledBack
            })
        } else {
            None
        };
        if let Some(outcome) = outcome {
            self.send(RefreshMsg::TxEnded {
                session: self.session,
                outcome,
            });
        }
    }

    /// The session's namespace and search path now (when a unit ended: execution time).
    fn state_now(&self) -> Option<SessionStateAtExec> {
        let snap = self.state.borrow();
        let ctx = snap.schema.as_ref()?;
        SessionStateAtExec::from_schema(
            self.dbms,
            ctx.database.as_deref(),
            ctx.schema.as_deref(),
            &ctx.search_path,
        )
    }

    fn send(&self, msg: RefreshMsg) {
        self.inbox.push(msg);
    }
}
