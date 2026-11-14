//! `ExecService` (HV-EXEC-05; Task 23): one `Request::Run` per run on the console's session,
//! with result stores on the app's shared budget, run status, result tabs and log lines.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use hacivat_results::{FetchPolicy, MemoryBudget, ResultSource};
use hacivat_session::session::{RunId, RunSummary, SessionHandle, UnitId};
use tokio::sync::{oneshot, watch};

use crate::log::SessionLog;
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
#[derive(Debug)]
pub struct ExecService {
    budget: Arc<MemoryBudget>,
    next_run: AtomicU64,
}

impl ExecService {
    /// A service whose result stores share `budget`.
    pub fn new(budget: Arc<MemoryBudget>) -> Self {
        ExecService {
            budget,
            next_run: AtomicU64::new(1),
        }
    }

    /// The memory budget every result of this service shares.
    pub fn budget(&self) -> &Arc<MemoryBudget> {
        &self.budget
    }

    /// Runs `req` on `console` (Task 23).
    ///
    /// Task 1b stub (fail closed): nothing is sent; the ticket's `done` answers an aborted
    /// summary with nothing executed, and its `seq` is `u64::MAX`, which names no request (a
    /// cancel of it answers `AlreadyFinished`).
    pub fn run(&self, console: &SessionHandle, req: RunRequest) -> RunTicket {
        let _ = console;
        let run = RunId(self.next_run.fetch_add(1, Ordering::SeqCst));
        let (done_tx, done) = oneshot::channel();
        let _ = done_tx.send(RunSummary {
            executed: 0,
            total: u32::try_from(req.units.len()).unwrap_or(u32::MAX),
            failed: 0,
            skipped: 0,
            aborted: true,
            duration_ms: 0,
        });
        let (_, status) = watch::channel(RunStatus::default());
        let (_, results) = watch::channel::<Arc<[ResultTab]>>(Arc::from(Vec::new()));
        RunTicket {
            seq: u64::MAX,
            run,
            status,
            results,
            done,
        }
    }
}
