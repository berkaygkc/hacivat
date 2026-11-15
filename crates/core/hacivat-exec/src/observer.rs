//! The service's `RunObserver` (Task 23): result stores on the shared budget, run status,
//! result tabs and log lines, O(1) per event and never blocking.
//!
//! **Status publishes.** The run's status lives in its `watch`; every change edits it in place
//! (`Arc::make_mut` copies the unit list only while a reader holds the previous one), so an
//! event costs O(1) unless the UI kept a snapshot. A unit's phase changes (started, ended,
//! suspended, its result ended) publish at once; progress inside a unit (its result opened,
//! rows fetched) is coalesced to one publish per 50 ms, and the next phase change carries it.
//!
//! **What the log says** (one line each, the unit's range as its source):
//! - a unit that ends: rows retrieved (with "more are available" while its cursor stays open),
//!   rows affected, completed, "Statement N failed" with the error view, a read-only denial,
//!   or cancelled (the unit in flight only; units a stop never reached say nothing);
//! - a suspended result that completes later: its rows retrieved; a released cursor: the
//!   reason, with "Hacivat's transaction was rolled back" when it was (R-M3-23);
//! - more result sets drained (R-M3-25); a user transaction committed or rolled back by
//!   `COMMIT`/`END` or `ROLLBACK`/`ABORT`; a summary when the run had more than one unit;
//! - a later fetch's failure (the result source's commands go through [`ExecControl`], which
//!   awaits the session's reply): "Statement N failed" with the server's text (plan amendment;
//!   the session ends the result with `CursorReleased` and `ResultEnded` without the text).
//!
//! **A run that never started** (its reply came without `RunEnded`: queued behind a cancel, a
//! closed or lost session) ends here: every unit `Skipped` (`Cancelled` after a cancel), an
//! aborted summary, and the reply's error in the log. Without a runtime to await the reply on,
//! the observer's drop does the same without the error.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use hacivat_driver::TxStatus;
use hacivat_proto::DbmsKind;
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, MemoryBudget, ResultId, ResultStore,
    ResultWriter, StoreSource,
};
use hacivat_session::session::{
    ExecError, ExecErrorKind, Request, RequestTicket, RowsOutcome, RunEvent, RunId, RunObserver,
    RunSummary, SessionHandle, UnitEnd, UnitId,
};
use hacivat_values::ResultSchema;
use tokio::runtime::Handle;
use tokio::sync::{oneshot, watch};

use crate::log::{LogMsg, SessionLog};
use crate::service::{ResultTab, TabKey};
use crate::sqlctx::first_word;
use crate::status::{RunStatus, UnitStatus, UnitStatusEntry, UnitSummary, error_view};
use crate::titles::result_title;
use crate::units::ExecUnit;

/// Progress publishes inside a unit are at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(50);

/// Result IDs, unique in the process.
static NEXT_RESULT: AtomicU64 = AtomicU64::new(1);

/// Each tab key's latest generation (a re-run into the same tab bumps it), shared by every run
/// of the service.
pub(crate) type Generations = Arc<Mutex<HashMap<TabKey, u64>>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a unit's log lines and errors need.
#[derive(Debug, Clone)]
struct UnitInfo {
    id: UnitId,
    text: Arc<str>,
    range: Range<usize>,
}

impl UnitInfo {
    /// The statement's number in its run, from 1 ("Statement N failed").
    fn n(&self) -> u32 {
        self.id.ordinal.saturating_add(1)
    }
}

struct ResultInfo {
    ordinal: u32,
    store: Arc<ResultStore>,
    ended: bool,
}

#[derive(Default)]
struct State {
    done: Option<oneshot::Sender<RunSummary>>,
    tabs: Vec<ResultTab>,
    results: HashMap<ResultId, ResultInfo>,
    /// The unit in flight.
    current: Option<u32>,
    last_publish: Option<tokio::time::Instant>,
    /// `RunEnded` was seen, or the run was ended as never started.
    ended: bool,
}

/// The parts a run shares with its observer.
pub(crate) struct RunParts {
    pub console: u64,
    pub run: RunId,
    pub engine: DbmsKind,
    pub units: Vec<ExecUnit>,
    pub budget: Arc<MemoryBudget>,
    pub generations: Generations,
    pub log: Arc<SessionLog>,
    pub session: SessionHandle,
    pub fetch: FetchPolicy,
}

/// One run's observer.
pub(crate) struct ExecObserver {
    console: u64,
    engine: DbmsKind,
    units: Arc<[UnitInfo]>,
    budget: Arc<MemoryBudget>,
    generations: Generations,
    log: Arc<SessionLog>,
    session: SessionHandle,
    fetch: FetchPolicy,
    status: watch::Sender<RunStatus>,
    results: watch::Sender<Arc<[ResultTab]>>,
    state: Mutex<State>,
}

/// The receivers a ticket holds.
pub(crate) struct Receivers {
    pub status: watch::Receiver<RunStatus>,
    pub results: watch::Receiver<Arc<[ResultTab]>>,
    pub done: oneshot::Receiver<RunSummary>,
}

impl ExecObserver {
    pub(crate) fn new(p: RunParts) -> (Arc<ExecObserver>, Receivers) {
        let units: Arc<[UnitInfo]> = p
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| UnitInfo {
                id: UnitId {
                    run: p.run,
                    ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                },
                text: u.text.clone(),
                range: u.doc_range.clone(),
            })
            .collect();
        let entries: Arc<[UnitStatusEntry]> = units
            .iter()
            .map(|u| UnitStatusEntry {
                unit: u.id,
                doc_range: u.range.clone(),
                status: UnitStatus::Queued { position: u.n() },
            })
            .collect();
        let (status, status_rx) = watch::channel(RunStatus {
            run: Some(p.run),
            started: None,
            units: entries,
            summary: None,
        });
        let (results, results_rx) = watch::channel::<Arc<[ResultTab]>>(Arc::from(Vec::new()));
        let (done_tx, done) = oneshot::channel();
        let observer = Arc::new(ExecObserver {
            console: p.console,
            engine: p.engine,
            units,
            budget: p.budget,
            generations: p.generations,
            log: p.log,
            session: p.session,
            fetch: p.fetch,
            status,
            results,
            state: Mutex::new(State {
                done: Some(done_tx),
                ..State::default()
            }),
        });
        let receivers = Receivers {
            status: status_rx,
            results: results_rx,
            done,
        };
        (observer, receivers)
    }

    fn unit(&self, ordinal: u32) -> Option<&UnitInfo> {
        self.units.get(usize::try_from(ordinal).ok()?)
    }

    fn line(&self, msg: LogMsg, unit: Option<&UnitInfo>) {
        self.log
            .append(msg, unit.map(|u| u.range.clone()), unit.map(|u| u.id));
    }

    /// Edits the status; publishes at once when `phase`, else at most every 50 ms.
    fn update(&self, phase: bool, edit: impl FnOnce(&mut RunStatus)) {
        let now = tokio::time::Instant::now();
        let notify = {
            let mut st = lock(&self.state);
            let notify = phase
                || st
                    .last_publish
                    .is_none_or(|at| now.saturating_duration_since(at) >= PROGRESS_EVERY);
            if notify {
                st.last_publish = Some(now);
            }
            notify
        };
        self.status.send_if_modified(|s| {
            edit(s);
            notify
        });
    }

    /// Sets unit `ordinal`'s status.
    fn set_unit(&self, ordinal: u32, phase: bool, status: UnitStatus) {
        let Ok(i) = usize::try_from(ordinal) else {
            return;
        };
        self.update(phase, |s| {
            if let Some(e) = Arc::make_mut(&mut s.units).get_mut(i) {
                e.status = status;
            }
        });
    }

    fn unit_status(&self, ordinal: u32) -> Option<UnitStatus> {
        let i = usize::try_from(ordinal).ok()?;
        self.status.borrow().units.get(i).map(|e| e.status.clone())
    }

    /// Whether the run has ended for this observer.
    pub(crate) fn ended(&self) -> bool {
        lock(&self.state).ended
    }

    /// Ends a run that never started (its reply came without `RunEnded`), with the reply's
    /// error when there is one.
    pub(crate) fn never_started(&self, error: Option<&ExecError>) {
        let done = {
            let mut st = lock(&self.state);
            if st.ended {
                return;
            }
            st.ended = true;
            st.done.take()
        };
        let cancelled = error.is_some_and(|e| e.kind == ExecErrorKind::Cancelled);
        let total = u32::try_from(self.units.len()).unwrap_or(u32::MAX);
        let summary = RunSummary {
            executed: 0,
            total,
            failed: 0,
            skipped: total,
            aborted: true,
            duration_ms: 0,
        };
        self.update(true, |s| {
            for e in Arc::make_mut(&mut s.units).iter_mut() {
                e.status = if cancelled {
                    UnitStatus::Cancelled
                } else {
                    UnitStatus::Skipped
                };
            }
            s.summary = Some(summary);
        });
        let first = self.units.first();
        match (error, first) {
            (Some(_), _) if cancelled => self.line(LogMsg::Cancelled { ms: 0 }, first),
            (Some(e), Some(u)) => self.line(self.failure(e, u), Some(u)),
            _ => {}
        }
        if let Some(done) = done {
            let _ = done.send(summary);
        }
    }

    /// The log line of `e`, a failure of unit `u`.
    fn failure(&self, e: &ExecError, u: &UnitInfo) -> LogMsg {
        match e.kind {
            ExecErrorKind::ReadOnlyDenied(layer) => LogMsg::ReadOnlyDenied(layer),
            _ => LogMsg::Error {
                n: u.n(),
                view: error_view(self.engine, e, &u.text, u.range.start, None),
            },
        }
    }

    fn result_info<R>(&self, result: ResultId, f: impl FnOnce(&mut ResultInfo) -> R) -> Option<R> {
        lock(&self.state).results.get_mut(&result).map(f)
    }

    /// Whether unit `ordinal`'s result is still open (it stays suspended after the unit).
    fn result_open(&self, ordinal: u32) -> bool {
        lock(&self.state)
            .results
            .values()
            .any(|r| r.ordinal == ordinal && !r.ended)
    }

    fn on_unit_ended(&self, unit: UnitId, outcome: &UnitEnd, exec_ms: u64, fetch_ms: u64) {
        let Some(u) = self.unit(unit.ordinal).cloned() else {
            return;
        };
        let was_in_flight = matches!(
            self.unit_status(unit.ordinal),
            Some(UnitStatus::Running { .. } | UnitStatus::Fetching { .. } | UnitStatus::Cancelling)
        );
        let summary = |rows| {
            UnitStatus::Succeeded(UnitSummary {
                rows,
                exec_ms,
                fetch_ms,
            })
        };
        let ms = exec_ms.saturating_add(fetch_ms);
        match outcome {
            UnitEnd::Succeeded { rows } => {
                let status = match rows {
                    Some(RowsOutcome::Fetched {
                        rows: n,
                        more: true,
                    }) if self.result_open(unit.ordinal) => UnitStatus::Suspended { rows: *n },
                    _ => summary(*rows),
                };
                self.set_unit(unit.ordinal, true, status);
                let msg = match rows {
                    Some(RowsOutcome::Fetched { rows, more }) => LogMsg::RowsRetrieved {
                        rows: *rows,
                        more: *more,
                        exec_ms,
                        fetch_ms,
                    },
                    Some(RowsOutcome::Affected(rows)) => LogMsg::RowsAffected { rows: *rows, ms },
                    None => LogMsg::Completed { ms },
                };
                self.line(msg, Some(&u));
            }
            UnitEnd::Failed(e) => {
                let view = error_view(self.engine, e, &u.text, u.range.start, None);
                self.set_unit(unit.ordinal, true, UnitStatus::Failed(view));
                self.line(self.failure(e, &u), Some(&u));
            }
            UnitEnd::Cancelled => {
                self.set_unit(unit.ordinal, true, UnitStatus::Cancelled);
                if was_in_flight {
                    self.line(LogMsg::Cancelled { ms }, Some(&u));
                }
            }
            UnitEnd::Skipped => self.set_unit(unit.ordinal, true, UnitStatus::Skipped),
        }
        let mut st = lock(&self.state);
        if st.current == Some(unit.ordinal) {
            st.current = None;
        }
    }

    fn on_result_ended(
        &self,
        result: ResultId,
        rows: u64,
        completeness: Completeness,
        exec_ms: u64,
        fetch_ms: u64,
    ) {
        let Some(ordinal) = self.result_info(result, |r| {
            r.ended = true;
            r.ordinal
        }) else {
            return;
        };
        // A result that stayed suspended after its unit ended: the unit's status follows it.
        if !matches!(
            self.unit_status(ordinal),
            Some(UnitStatus::Suspended { .. })
        ) {
            return;
        }
        let complete = completeness == Completeness::Complete;
        self.set_unit(
            ordinal,
            true,
            UnitStatus::Succeeded(UnitSummary {
                rows: Some(RowsOutcome::Fetched {
                    rows,
                    more: !complete,
                }),
                exec_ms,
                fetch_ms,
            }),
        );
        if complete {
            let u = self.unit(ordinal).cloned();
            self.line(
                LogMsg::RowsRetrieved {
                    rows,
                    more: false,
                    exec_ms,
                    fetch_ms,
                },
                u.as_ref(),
            );
        }
    }

    fn on_rows(&self, result: ResultId, total: u64) {
        let Some(ordinal) = self.result_info(result, |r| r.ordinal) else {
            return;
        };
        let status = match self.unit_status(ordinal) {
            Some(UnitStatus::Suspended { .. }) => UnitStatus::Suspended { rows: total },
            Some(UnitStatus::Running { .. } | UnitStatus::Fetching { .. }) => {
                UnitStatus::Fetching { rows: total }
            }
            _ => return,
        };
        self.set_unit(ordinal, false, status);
    }

    fn on_released(&self, result: ResultId, rows: u64, reason: CloseReason, rolled_back: bool) {
        let Some((ordinal, store)) = self.result_info(result, |r| (r.ordinal, r.store.clone()))
        else {
            return;
        };
        // A failure while its unit is in flight is that unit's "Statement N failed" line.
        let failure = matches!(
            reason,
            CloseReason::ServerTerminated
                | CloseReason::ConnectionLost
                | CloseReason::EnforcementGap
        );
        if failure && lock(&self.state).current == Some(ordinal) {
            return;
        }
        let idle_ms = match (reason, store.snapshot().completeness()) {
            (CloseReason::IdleTimeout, Completeness::Suspended(info)) => {
                u64::try_from(info.idle_bound.as_millis()).unwrap_or(u64::MAX)
            }
            _ => 0,
        };
        let u = self.unit(ordinal).cloned();
        self.line(
            LogMsg::CursorClosed {
                rows,
                reason,
                rolled_back,
                idle_ms,
            },
            u.as_ref(),
        );
    }

    fn on_tx_changed(&self, from: TxStatus, to: TxStatus) {
        if to != TxStatus::Idle || from == TxStatus::Idle {
            return;
        }
        let current = lock(&self.state).current;
        let Some(u) = current.and_then(|c| self.unit(c)).cloned() else {
            return;
        };
        let word = first_word(&u.text);
        let is = |w: &str| word.eq_ignore_ascii_case(w);
        let msg = if is("ROLLBACK") || is("ABORT") || (from == TxStatus::Failed && is("COMMIT")) {
            LogMsg::TxRolledBack
        } else if is("COMMIT") || is("END") {
            LogMsg::TxCommitted
        } else {
            // An implicit end (a MySQL DDL, a loss) is not announced: better silent than wrong.
            return;
        };
        self.line(msg, Some(&u));
    }

    fn on_run_ended(&self, summary: RunSummary) {
        let done = {
            let mut st = lock(&self.state);
            st.ended = true;
            st.done.take()
        };
        self.update(true, |s| s.summary = Some(summary));
        if summary.total > 1 {
            self.line(LogMsg::Summary(summary), None);
        }
        if let Some(done) = done {
            let _ = done.send(summary);
        }
    }
}

impl RunObserver for ExecObserver {
    fn open_result(&self, unit: UnitId, ordinal: u32, schema: Arc<ResultSchema>) -> ResultWriter {
        let key = TabKey {
            console: self.console,
            unit_ordinal: unit.ordinal,
            result_ordinal: ordinal,
        };
        let generation = {
            let mut g = lock(&self.generations);
            let n = g.entry(key).or_insert(0);
            *n += 1;
            *n
        };
        let id = ResultId(NEXT_RESULT.fetch_add(1, Ordering::Relaxed));
        let (store, writer) = ResultStore::new(id, schema, self.budget.clone(), generation);
        let info = self.unit(unit.ordinal).cloned();
        let control: Arc<dyn FetchControl> = Arc::new(ExecControl {
            session: self.session.clone(),
            log: self.log.clone(),
            // The actor's runtime: the control spawns its reply waits there.
            rt: Handle::try_current().ok(),
            engine: self.engine,
            unit: info,
        });
        let source = StoreSource::new(store.clone(), control, self.fetch);
        let tabs: Arc<[ResultTab]> = {
            let mut st = lock(&self.state);
            let title = result_title(st.tabs.len() + 1);
            st.tabs.push(ResultTab {
                key,
                title,
                unit,
                source,
            });
            st.results.insert(
                id,
                ResultInfo {
                    ordinal: unit.ordinal,
                    store,
                    ended: false,
                },
            );
            st.tabs.iter().cloned().collect()
        };
        self.results.send_replace(tabs);
        writer
    }

    fn event(&self, ev: &RunEvent) {
        match ev {
            RunEvent::UnitStarted { unit, at } => {
                lock(&self.state).current = Some(unit.ordinal);
                let (ordinal, since) = (unit.ordinal, *at);
                self.update(true, |s| {
                    s.started.get_or_insert(since);
                    if let Some(e) = usize::try_from(ordinal)
                        .ok()
                        .and_then(|i| Arc::make_mut(&mut s.units).get_mut(i))
                    {
                        e.status = UnitStatus::Running { since };
                    }
                });
            }
            RunEvent::ResultOpened { unit, .. } => {
                self.set_unit(unit.ordinal, false, UnitStatus::Fetching { rows: 0 });
            }
            RunEvent::RowsFetched { result, total } => self.on_rows(*result, *total),
            RunEvent::ResultEnded {
                result,
                rows,
                completeness,
                exec_ms,
                fetch_ms,
            } => self.on_result_ended(*result, *rows, *completeness, *exec_ms, *fetch_ms),
            RunEvent::UpdateCount { .. } => {}
            RunEvent::MoreResultSets { unit, skipped } => {
                let u = self.unit(unit.ordinal).cloned();
                self.line(LogMsg::MoreResultSets { skipped: *skipped }, u.as_ref());
            }
            RunEvent::CursorReleased {
                result,
                rows,
                reason,
                rolled_back,
            } => self.on_released(*result, *rows, *reason, *rolled_back),
            RunEvent::TxChanged { from, to } => self.on_tx_changed(*from, *to),
            RunEvent::UnitEnded {
                unit,
                outcome,
                exec_ms,
                fetch_ms,
            } => self.on_unit_ended(*unit, outcome, *exec_ms, *fetch_ms),
            RunEvent::RunEnded(summary) => self.on_run_ended(*summary),
        }
    }
}

/// A run that ends without `RunEnded` and nobody awaited its reply: ended here, without the
/// reply's error.
impl Drop for ExecObserver {
    fn drop(&mut self) {
        self.never_started(None);
    }
}

/// A result source's commands to its session, whose replies are awaited: a failed fetch or
/// release is logged with the server's text (the plan amendment; `FetchControl for
/// SessionHandle` drops them).
struct ExecControl {
    session: SessionHandle,
    log: Arc<SessionLog>,
    rt: Option<Handle>,
    engine: DbmsKind,
    unit: Option<UnitInfo>,
}

impl ExecControl {
    fn follow(&self, ticket: RequestTicket) {
        let Some(rt) = &self.rt else {
            return;
        };
        let (log, engine, unit) = (self.log.clone(), self.engine, self.unit.clone());
        rt.spawn(async move {
            let Ok(Err(e)) = ticket.reply.await else {
                return;
            };
            // A stop says so through the cursor line; a lost connection too, unless the
            // server said something.
            let quiet = e.kind == ExecErrorKind::Cancelled
                || (matches!(e.kind, ExecErrorKind::ConnectionLost { .. }) && e.server.is_none());
            if quiet {
                return;
            }
            let Some(u) = unit else {
                return;
            };
            let msg = match e.kind {
                ExecErrorKind::ReadOnlyDenied(layer) => LogMsg::ReadOnlyDenied(layer),
                _ => LogMsg::Error {
                    n: u.n(),
                    view: error_view(engine, &e, &u.text, u.range.start, None),
                },
            };
            log.append(msg, Some(u.range.clone()), Some(u.id));
        });
    }
}

impl FetchControl for ExecControl {
    fn fetch_more(&self, result: ResultId, rows: u64) {
        self.follow(self.session.submit(Request::FetchMore { result, rows }));
    }

    fn close(&self, result: ResultId, reason: CloseReason) {
        self.follow(self.session.submit(Request::CloseCursor { result, reason }));
    }

    fn fetch_all(&self, result: ResultId) {
        self.follow(self.session.submit(Request::FetchAll { result }));
    }
}
