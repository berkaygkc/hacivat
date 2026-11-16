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
use crate::status::RunStatus;
use crate::units::ExecUnit;

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
}

impl fmt::Debug for ExecService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecService")
            .field("next_run", &self.next_run.load(Ordering::Relaxed))
            .field("runtime", &self.runtime.is_some())
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
        }
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
        let units: Arc<[RunUnit]> = req
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| RunUnit {
                id: UnitId {
                    run,
                    ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                },
                sql: u.text.clone(),
                protocol: Protocol::Extended,
                suspendable: u.suspendable,
            })
            .collect();
        let empty = units.is_empty();
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
            policy: ErrorPolicy::StopOnError,
            fetch: req.fetch,
            observer: ObserverRef(observer.clone()),
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
