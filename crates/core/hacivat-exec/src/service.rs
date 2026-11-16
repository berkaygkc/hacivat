//! `ExecService` (HV-EXEC-05; Task 23): one `Request::Run` per run on the console's session,
//! with result stores on the app's shared budget, run status, result tabs and log lines.
//!
//! Each unit is sent as resolved: its strict text, byte for byte, with the extended protocol,
//! `StopOnError` (R-M3-26), and `suspendable` from its cursor eligibility (the session keeps
//! only the run's last unit suspended). One result tab per unit's first result set, keyed
//! `TabKey { console, unit_ordinal, 0 }`: a re-run reuses the key, and its store's generation
//! counts the runs into that tab.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hacivat_driver::Protocol;
use hacivat_proto::DbmsKind;
use hacivat_results::{FetchPolicy, MemoryBudget, ResultSource};
use hacivat_session::session::{
    ErrorPolicy, ObserverRef, RunId, RunSpec, RunSummary, RunUnit, SessionHandle, UnitId,
};
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};

use crate::log::SessionLog;
use crate::observer::{ExecObserver, Generations, RunParts};
// ## m3b-t01c-core-seams
use crate::classify_pool::ClassifyPool;
use crate::gate::{ConfirmPort, GatePlan};
use crate::history::{HistoryMeta, HistoryStore};
use crate::params::PreparedUnit;
use crate::status::RunStatus;
use crate::units::ExecUnit;
use hacivat_session::session::{ExecError, ExecErrorKind};

/// Which result tab a result belongs to: a re-run of the same unit reuses its tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TabKey {
    pub console: u64,
    pub unit_ordinal: u32,
    pub result_ordinal: u32,
}

/// One result tab of a run.
#[derive(Clone)]
pub struct ResultTab {
    pub key: TabKey,
    /// `Result N` in M3a.
    pub title: Arc<str>,
    pub unit: UnitId,
    pub source: Arc<dyn ResultSource>,
}

impl fmt::Debug for ResultTab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultTab")
            .field("key", &self.key)
            .field("title", &self.title)
            .field("unit", &self.unit)
            .finish_non_exhaustive()
    }
}

/// One run of a console.
#[derive(Debug, Clone)]
pub struct RunRequest {
    pub console: u64,
    /// The console's engine (error hints are per engine).
    pub engine: DbmsKind,
    pub units: Vec<ExecUnit>,
    pub fetch: FetchPolicy,
    /// The console's own output log.
    pub log: Arc<SessionLog>,
    // ## m3b-t01c-core-seams (P-M3b-22: every literal of M3a sets these to `None`, `false`,
    // `StopOnError`)
    /// The prepared units (params, final text; Task 15). Until Task 16 a request that sets it
    /// is refused, nothing sent.
    pub prepared: Option<Vec<PreparedUnit>>,
    /// The gate's plan with the user's answers applied (Task 24). Until Task 16 a request that
    /// sets it is refused, nothing sent.
    pub plan: Option<GatePlan>,
    /// `Ask` for a multi-unit run (R-M3-61); the session asks from Task 11 on.
    pub policy: ErrorPolicy,
    /// Send the units as one (R-M3-60). Until Task 16 a request that sets it is refused.
    pub run_as_one: bool,
    /// Where the history records the run (Task 17); `None` records nothing.
    pub history: Option<HistoryMeta>,
    /// Where confirm-in-transaction questions go (Task 21's port); `None` rolls back.
    pub confirm: Option<Arc<dyn ConfirmPort>>,
}

// ## m3b-t01c-core-seams
impl RunRequest {
    /// Whether the request asks for something only Task 16's service sends (prepared units, a
    /// gate plan, "Run as one"): fail closed until then (R-M3-52).
    fn needs_task_16(&self) -> bool {
        self.prepared.is_some() || self.plan.is_some() || self.run_as_one
    }
}

/// A submitted run: its request's sequence number (to cancel it), its ID, its status, its
/// result tabs, and its summary when it ends.
#[derive(Debug)]
pub struct RunTicket {
    pub seq: u64,
    pub run: RunId,
    pub status: watch::Receiver<RunStatus>,
    pub results: watch::Receiver<Arc<[ResultTab]>>,
    pub done: oneshot::Receiver<RunSummary>,
}

/// The app's run service (one per app).
pub struct ExecService {
    budget: Arc<MemoryBudget>,
    next_run: AtomicU64,
    generations: Generations,
    /// Where the service awaits replies (a run that never started); the core runtime.
    runtime: Option<Handle>,
    // ## m3b-t01c-core-seams
    /// The history the runs record into (Task 17).
    history: Option<Arc<HistoryStore>>,
    /// The classifier pool `count` classifies with (Tasks 14, 16).
    classifier: Option<Arc<ClassifyPool>>,
}

impl fmt::Debug for ExecService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecService")
            .field("next_run", &self.next_run.load(Ordering::Relaxed))
            .field("runtime", &self.runtime.is_some())
            .field("history", &self.history.is_some())
            .field("classifier", &self.classifier.is_some())
            .finish_non_exhaustive()
    }
}

impl ExecService {
    /// A service whose result stores share `budget`. Built inside a Tokio runtime, it awaits
    /// replies there; elsewhere give it one with [`with_runtime`](Self::with_runtime).
    pub fn new(budget: Arc<MemoryBudget>) -> Self {
        ExecService {
            budget,
            next_run: AtomicU64::new(1),
            generations: Arc::new(Mutex::new(HashMap::new())),
            runtime: Handle::try_current().ok(),
            history: None,
            classifier: None,
        }
    }

    // ## m3b-t01c-core-seams
    /// The history every run of this service records into (Task 17).
    #[must_use]
    pub fn with_history(mut self, h: Arc<HistoryStore>) -> Self {
        self.history = Some(h);
        self
    }

    /// The classifier pool this service classifies with (Tasks 14, 16, 24).
    #[must_use]
    pub fn with_classifier(mut self, p: Arc<ClassifyPool>) -> Self {
        self.classifier = Some(p);
        self
    }

    /// "Count rows" (R-M3-76; Task 16): `sql` (from `count::count_sql`) classified again and
    /// run on a pooled session or on `session`. STUB until Task 16: `Err(Internal)`, nothing
    /// sent.
    pub fn count(
        &self,
        session: &SessionHandle,
        sql: String,
    ) -> oneshot::Receiver<Result<u64, ExecError>> {
        let _ = (session, sql);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Err(ExecError::of(ExecErrorKind::Internal)));
        rx
    }

    /// The runtime the service awaits session replies on (the app's core runtime).
    #[must_use]
    pub fn with_runtime(mut self, rt: Handle) -> Self {
        self.runtime = Some(rt);
        self
    }

    /// The memory budget every result of this service shares.
    pub fn budget(&self) -> &Arc<MemoryBudget> {
        &self.budget
    }

    /// Runs `req` on `console`: one `Request::Run` with every unit in order. Never blocks; it
    /// may be called from any thread. A request without units sends nothing and ends at once.
    pub fn run(&self, console: &SessionHandle, req: RunRequest) -> RunTicket {
        let run = RunId(self.next_run.fetch_add(1, Ordering::SeqCst));
        // ## m3b-t01c-core-seams: M3a's path: no binds, no verdict, one result set per unit.
        let units: Arc<[RunUnit]> = req
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| {
                RunUnit::plain(
                    UnitId {
                        run,
                        ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                    },
                    u.text.clone(),
                    Protocol::Extended,
                    u.suspendable,
                )
            })
            .collect();
        let empty = units.is_empty();
        let refused = req.needs_task_16();
        let policy = req.policy;
        let (observer, rx) = ExecObserver::new(RunParts {
            console: req.console,
            run,
            engine: req.engine,
            units: req.units,
            budget: self.budget.clone(),
            generations: self.generations.clone(),
            log: req.log,
            session: console.clone(),
            fetch: req.fetch,
        });
        // ## m3b-t01c-core-seams (fail closed until Task 16): a request with prepared units, a
        // plan or "Run as one" sends nothing; ignoring them could send what the gate denied.
        if refused {
            observer.never_started(Some(&ExecError::of(ExecErrorKind::Internal)));
            return RunTicket {
                seq: u64::MAX,
                run,
                status: rx.status,
                results: rx.results,
                done: rx.done,
            };
        }
        if empty {
            observer.never_started(None);
            return RunTicket {
                // Names no request: a cancel of it answers `AlreadyFinished`.
                seq: u64::MAX,
                run,
                status: rx.status,
                results: rx.results,
                done: rx.done,
            };
        }
        let ticket = console.run(RunSpec {
            run,
            units,
            // ## m3b-t01c-core-seams: the request's policy (`StopOnError` in M3a; the session
            // stops on the first failure under `Ask` too until Task 11).
            policy,
            fetch: req.fetch,
            observer: ObserverRef(observer.clone()),
            confirm_deadline: RunSpec::CONFIRM_DEADLINE,
        });
        let seq = ticket.seq;
        match &self.runtime {
            Some(rt) => {
                rt.spawn(async move {
                    // The reply follows `RunEnded`; one without it never started.
                    let reply = ticket.reply.await;
                    if !observer.ended() {
                        observer.never_started(reply.as_ref().ok().and_then(|r| r.as_ref().err()));
                    }
                });
            }
            // Without a runtime the observer's drop ends a run that never started.
            None => drop(observer),
        }
        RunTicket {
            seq,
            run,
            status: rx.status,
            results: rx.results,
            done: rx.done,
        }
    }
}
