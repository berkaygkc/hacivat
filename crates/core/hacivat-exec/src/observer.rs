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
//!
//! ## m3b-t16-run-service (ADR 0006 `m3b-t16`)
//! - **The auditor chain** (R-M3-63, `audit.rs`) sees every event first; its history sink
//!   records one statement per ended unit (masked text only), and what its sinks dropped ends
//!   the run as one `Dropped { n }` line.
//! - **Tabs** (R-M3-66): one per result set, titled by the unit's comment (else `Result N`),
//!   ` (k)` for a unit's further sets; a prepared unit keeps up to 100 sets and a unit with more
//!   says so (`exec-result-set-limit`) instead of `exec-more-result-sets`.
//! - **Lines**: server notices and MySQL warnings (`Notice`, the server's text verbatim in its
//!   own field), notices past the cap (`NoticesDropped`), a Run as one stopped before its end
//!   (`ClosedEarly`), a PostgreSQL Run as one that failed (`RunAsOneRolledBack`: one implicit
//!   transaction), a retried statement, the user's skips (`SkippedByUser`), an eviction of one
//!   of the run's results (`Evicted`, through the service's [`Evictions`]); a DDL statement's
//!   end is `exec-completed` on every engine (ledger 340, PO); a line keeps its statement's
//!   command tag (ledger 340).
//! - **Questions**: a `NeedsChoice` or `NeedsCommit` is published on the ticket's `choices`
//!   with the run's request number until the run moves on; a `NeedsCommit` also goes to the
//!   request's `ConfirmPort` (none: `RollBack`), whose answer is sent with
//!   `SessionHandle::commit_decision` for exactly that question.

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
use crate::status::{ErrorView, RunStatus, UnitStatus, UnitStatusEntry, UnitSummary, error_view};
use crate::units::ExecUnit;
// ## m3b-t16-run-service
use std::sync::Weak;

use hacivat_results::evict::Eviction;
use hacivat_session::session::CommitAnswer;

use crate::audit::AuditorChain;
use crate::gate::{CommitQuestion, ConfirmPort};
use crate::params::OffsetMap;
use crate::titles::tab_title;

/// The result sets a prepared unit keeps (R-M3-66).
pub(crate) const MAX_RESULT_SETS: u32 = 100;

/// Progress publishes inside a unit are at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(50);

/// Result IDs, unique in the process.
static NEXT_RESULT: AtomicU64 = AtomicU64::new(1);

// ## m3b-t16-run-service
/// A result ID unique in the process (a count's store too).
pub(crate) fn next_result_id() -> ResultId {
    ResultId(NEXT_RESULT.fetch_add(1, Ordering::Relaxed))
}

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
    /// The text sent (a server's error position is in it).
    text: Arc<str>,
    range: Range<usize>,
    // ## m3b-t16-run-service
    /// The comment title of its tabs.
    title: Option<Arc<str>>,
    /// From the text sent to the document (a prepared unit's rewrite); `None`: as written.
    offsets: Option<OffsetMap>,
    /// It keeps up to [`MAX_RESULT_SETS`] result sets (a prepared unit).
    multi: bool,
    /// It was sent as one (`Protocol::Simple`, R-M3-60).
    run_as_one: bool,
    /// A DDL statement: its end says `exec-completed` on every engine (ledger 340).
    ddl: bool,
}

impl UnitInfo {
    /// The statement's number in its run, from 1 ("Statement N failed").
    fn n(&self) -> u32 {
        self.id.ordinal.saturating_add(1)
    }

    // ## m3b-t16-run-service
    /// The view of `e`, a failure of this unit, its position mapped to the document.
    fn error_view(&self, engine: DbmsKind, e: &ExecError) -> ErrorView {
        match &self.offsets {
            None => error_view(engine, e, &self.text, self.range.start, None),
            Some(map) => {
                let mut view = error_view(engine, e, &self.text, 0, None);
                view.doc_offset = view.doc_offset.map(|at| map.to_doc(at));
                view
            }
        }
    }
}

// ## m3b-t16-run-service
/// Whether `sql` is a DDL statement by its first word (ledger 340's rule: its end is
/// `exec-completed` on every engine, whatever count the server reported).
pub(crate) fn is_ddl(sql: &str) -> bool {
    let word = first_word(sql.trim_start());
    ["CREATE", "ALTER", "DROP", "TRUNCATE", "RENAME", "COMMENT"]
        .iter()
        .any(|w| word.eq_ignore_ascii_case(w))
}

/// Whether `kind` speaks PostgreSQL's protocol (its Run as one is one implicit transaction).
fn pg_wire(kind: DbmsKind) -> bool {
    matches!(
        kind,
        DbmsKind::Postgres
            | DbmsKind::Redshift
            | DbmsKind::Cockroach
            | DbmsKind::Yugabyte
            | DbmsKind::Timescale
    )
}

/// Where an eviction of a result is logged: the console's log and the result's unit.
struct EvictTarget {
    log: Weak<SessionLog>,
    store: Weak<ResultStore>,
    range: Range<usize>,
    unit: UnitId,
}

/// The service's map from results to their logs, for the budget's eviction listener
/// (`MemoryBudget::on_evicted`, Task 8: `LogMsg::Evicted`). O(1) per eviction; results whose
/// store is gone are pruned as the map grows.
#[derive(Default)]
pub(crate) struct Evictions {
    targets: Mutex<(HashMap<ResultId, EvictTarget>, usize)>,
}

impl Evictions {
    fn register(&self, id: ResultId, target: EvictTarget) {
        let mut g = lock(&self.targets);
        let (map, prune_at) = &mut *g;
        if map.len() >= (*prune_at).max(64) {
            map.retain(|_, t| t.store.strong_count() > 0 && t.log.strong_count() > 0);
            *prune_at = map.len().saturating_mul(2);
        }
        map.insert(id, target);
    }

    /// The budget evicted a result: its console's log says so (never blocks on the result).
    pub(crate) fn on_evicted(&self, e: &Eviction) {
        let target = {
            let g = lock(&self.targets);
            g.0.get(&e.result)
                .map(|t| (t.log.upgrade(), t.range.clone(), t.unit))
        };
        if let Some((Some(log), range, unit)) = target {
            log.append(
                LogMsg::Evicted { rows: e.kept_rows },
                Some(range),
                Some(unit),
            );
        }
    }
}

/// A unit's place in an M3b run (Task 16): what the service sends for it.
#[derive(Debug, Clone)]
pub(crate) struct PlannedUnit {
    /// The text sent (`PreparedUnit::final_text`).
    pub sent: Arc<str>,
    pub offsets: OffsetMap,
    pub run_as_one: bool,
    /// The gate's effects say DDL.
    pub ddl: bool,
    /// The user skipped it at the confirmation (R-M3-84): never sent.
    pub skipped: bool,
}

struct ResultInfo {
    ordinal: u32,
    store: Arc<ResultStore>,
    ended: bool,
    /// The release that ended it was a failure (its unit then fails).
    failed: Option<CloseReason>,
}

#[derive(Default)]
struct State {
    done: Option<oneshot::Sender<RunSummary>>,
    tabs: Vec<ResultTab>,
    results: HashMap<ResultId, ResultInfo>,
    /// The unit in flight.
    current: Option<u32>,
    /// The unit in flight moved the user's transaction to idle: (from, the unit's ordinal).
    /// What that means is decided when the unit ends (review C2).
    tx_ended: Option<(TxStatus, u32)>,
    last_publish: Option<tokio::time::Instant>,
    /// `RunEnded` was seen, or the run was ended as never started.
    ended: bool,
    // ## m3b-t16-run-service
    /// Each unit's `Result N` number, given with its first tab.
    numbered: HashMap<u32, usize>,
    /// Each unit's command tag (its first `UpdateCount`'s), for its end line.
    tags: HashMap<u32, Arc<str>>,
    /// The question the ticket's `choices` shows while the request number is unknown.
    pending_choice: Option<RunEvent>,
    /// The form of each unit's confirm-in-transaction question (`NeedsCommit::savepoint`).
    commit_savepoint: HashMap<u32, bool>,
    // Fix round 1 (review m9)
    /// The user's transaction status now: the session's at the run's start, then each
    /// `TxChanged`.
    tx_now: TxStatus,
    /// The user's transaction status when each unit started.
    tx_at_start: HashMap<u32, TxStatus>,
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
    // ## m3b-t16-run-service
    /// What the service sends per unit (an M3b run); `None`: M3a's run, the units as written.
    pub planned: Option<Vec<PlannedUnit>>,
    pub chain: AuditorChain,
    pub confirm: Option<Arc<dyn ConfirmPort>>,
    pub evictions: Arc<Evictions>,
    /// Where the observer awaits the confirm port's answers (the service's runtime).
    pub rt: Option<Handle>,
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
    // ## m3b-t16-run-service
    run: RunId,
    chain: Mutex<AuditorChain>,
    /// Units the user skipped at the confirmation (never sent).
    skipped_by_user: u32,
    choices: watch::Sender<Option<(u64, RunEvent)>>,
    /// The run's request number, once it is sent.
    seq: watch::Sender<Option<u64>>,
    confirm: Option<Arc<dyn ConfirmPort>>,
    evictions: Arc<Evictions>,
    rt: Option<Handle>,
}

/// The receivers a ticket holds.
pub(crate) struct Receivers {
    pub status: watch::Receiver<RunStatus>,
    pub results: watch::Receiver<Arc<[ResultTab]>>,
    pub done: oneshot::Receiver<RunSummary>,
    // ## m3b-t16-run-service
    pub choices: watch::Receiver<Option<(u64, RunEvent)>>,
}

impl ExecObserver {
    pub(crate) fn new(p: RunParts) -> (Arc<ExecObserver>, Receivers) {
        let units: Arc<[UnitInfo]> = p
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let id = UnitId {
                    run: p.run,
                    ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                };
                // ## m3b-t16-run-service: an M3b unit is described by what is sent for it.
                match p.planned.as_ref().and_then(|v| v.get(i)) {
                    Some(plan) => UnitInfo {
                        id,
                        text: plan.sent.clone(),
                        range: u.doc_range.clone(),
                        title: u.title.clone(),
                        offsets: Some(plan.offsets.clone()),
                        multi: true,
                        run_as_one: plan.run_as_one,
                        ddl: plan.ddl || is_ddl(&plan.sent),
                    },
                    None => UnitInfo {
                        id,
                        text: u.text.clone(),
                        range: u.doc_range.clone(),
                        title: u.title.clone(),
                        offsets: None,
                        multi: false,
                        run_as_one: false,
                        ddl: is_ddl(&u.text),
                    },
                }
            })
            .collect();
        // ## m3b-t16-run-service: units the user skipped are `Skipped` from the start.
        let skipped: Vec<bool> = (0..units.len())
            .map(|i| {
                p.planned
                    .as_ref()
                    .and_then(|v| v.get(i))
                    .is_some_and(|u| u.skipped)
            })
            .collect();
        let skipped_by_user =
            u32::try_from(skipped.iter().filter(|s| **s).count()).unwrap_or(u32::MAX);
        let entries: Arc<[UnitStatusEntry]> = units
            .iter()
            .zip(&skipped)
            .map(|(u, skipped)| UnitStatusEntry {
                unit: u.id,
                doc_range: u.range.clone(),
                status: if *skipped {
                    UnitStatus::Skipped
                } else {
                    UnitStatus::Queued { position: u.n() }
                },
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
        let (choices, choices_rx) = watch::channel(None);
        // Fix round 1 (review m9): the user's transaction as the run starts (Hacivat's own
        // around a cursor or the guard's is not the user's).
        let tx_now = {
            let snap = p.session.state();
            let tx = &snap.borrow().tx;
            if tx.owner.is_hacivats() {
                TxStatus::Idle
            } else {
                tx.status
            }
        };
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
                tx_now,
                ..State::default()
            }),
            run: p.run,
            chain: Mutex::new(p.chain),
            skipped_by_user,
            choices,
            seq: watch::Sender::new(None),
            confirm: p.confirm,
            evictions: p.evictions,
            rt: p.rt,
        });
        let receivers = Receivers {
            status: status_rx,
            results: results_rx,
            done,
            choices: choices_rx,
        };
        (observer, receivers)
    }

    // ## m3b-t16-run-service
    /// The run was sent as request `seq`: a question asked before that is published now.
    ///
    /// The request number, the pending question and the `choices` publish change under the
    /// state's lock, so a question the actor asks while the caller sets the number is never
    /// lost nor published after the run moved on.
    pub(crate) fn set_seq(&self, seq: u64) {
        let mut st = lock(&self.state);
        self.seq.send_replace(Some(seq));
        if let Some(ev) = st.pending_choice.take() {
            self.choices.send_replace(Some((seq, ev)));
        }
    }

    /// The question `ev` waits on the ticket's `choices`.
    fn publish_choice(&self, ev: &RunEvent) {
        let mut st = lock(&self.state);
        let seq = *self.seq.borrow();
        match seq {
            Some(seq) => {
                self.choices.send_replace(Some((seq, ev.clone())));
            }
            None => st.pending_choice = Some(ev.clone()),
        }
    }

    /// The run moved on: no question waits.
    fn clear_choice(&self) {
        let mut st = lock(&self.state);
        st.pending_choice = None;
        self.choices.send_if_modified(|c| c.take().is_some());
    }

    /// The user skipped units at the confirmation (R-M3-84): one line says how many.
    pub(crate) fn log_skipped_by_user(&self) {
        if self.skipped_by_user > 0 {
            self.line(
                LogMsg::SkippedByUser {
                    n: self.skipped_by_user,
                },
                None,
            );
        }
    }

    /// Forwards a confirm-in-transaction question to the request's port (none: `RollBack`)
    /// and its answer to the session, for exactly that question (review I1).
    fn forward_commit(
        &self,
        unit: UnitId,
        question: u64,
        affected: Option<u64>,
        deadline: std::time::Instant,
        savepoint: bool,
    ) {
        let session = self.session.clone();
        let mut seq = self.seq.subscribe();
        let answer = self.confirm.as_ref().map(|port| {
            port.commit_decision(CommitQuestion {
                run: self.run,
                unit,
                question,
                affected,
                deadline,
                savepoint,
                // Confirm-in-transaction exists on production only (R-M3-55, P-M3b-30); the
                // port, which belongs to its console, names the connection.
                production: true,
                connection: Arc::from(""),
            })
        });
        let send = async move {
            let answer = match answer {
                Some(rx) => rx.await.unwrap_or(CommitAnswer::RollBack),
                None => CommitAnswer::RollBack,
            };
            if let Ok(seq) = seq.wait_for(Option::is_some).await
                && let Some(seq) = *seq
            {
                // The command is queued at the call; its reply is not needed.
                drop(session.commit_decision(seq, unit, question, answer));
            }
        };
        match &self.rt {
            Some(rt) => {
                rt.spawn(send);
            }
            // No runtime to await on: the session's deadline answers (D-014).
            None => drop(send),
        }
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
        // Nothing ran, so the line names no statement and selects nothing.
        match error {
            Some(_) if cancelled => self.line(LogMsg::Cancelled { ms: 0 }, None),
            Some(e) => {
                let view = error_view(self.engine, e, "", 0, None);
                self.line(LogMsg::NotStarted(view), None);
            }
            None => {}
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
                view: u.error_view(self.engine, e),
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
                    // ## m3b-t16-run-service (ledger 340, PO): a DDL statement completed, on
                    // every engine (MySQL and SQLite count 0 rows for it).
                    Some(RowsOutcome::Affected(_)) if u.ddl => LogMsg::Completed { ms },
                    Some(RowsOutcome::Affected(rows)) => LogMsg::RowsAffected { rows: *rows, ms },
                    None => LogMsg::Completed { ms },
                };
                // ## m3b-t16-run-service (ledger 340): with the statement's command tag.
                let tag = lock(&self.state).tags.remove(&unit.ordinal);
                self.log
                    .append_tagged(msg, tag, Some(u.range.clone()), Some(u.id));
            }
            UnitEnd::Failed(e) => {
                let view = u.error_view(self.engine, e);
                self.set_unit(unit.ordinal, true, UnitStatus::Failed(view));
                self.line(self.failure(e, &u), Some(&u));
                if matches!(e.kind, ExecErrorKind::Db | ExecErrorKind::Cancelled) {
                    self.run_as_one_aborted(unit, &u);
                }
            }
            UnitEnd::Cancelled => {
                self.set_unit(unit.ordinal, true, UnitStatus::Cancelled);
                if was_in_flight {
                    self.line(LogMsg::Cancelled { ms }, Some(&u));
                    self.run_as_one_aborted(unit, &u);
                }
            }
            UnitEnd::Skipped => self.set_unit(unit.ordinal, true, UnitStatus::Skipped),
        }
        let tx_ended = {
            let mut st = lock(&self.state);
            if st.current == Some(unit.ordinal) {
                st.current = None;
            }
            st.tx_ended.take_if(|(_, o)| *o == unit.ordinal)
        };
        if let Some((from, _)) = tx_ended
            && let Some(msg) = tx_line(&u.text, from, outcome)
        {
            self.line(msg, Some(&u));
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
        let Some((ordinal, failed)) = self.result_info(result, |r| {
            r.ended = true;
            (r.ordinal, r.failed)
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
        // Released by a failure: the statement failed after all (review I3); the server's
        // text is on the log line the fetch's reply wrote.
        if let Some(reason) = failed {
            let view = ErrorView {
                title_id: match reason {
                    CloseReason::ConnectionLost => "exec-error-connection-lost",
                    CloseReason::EnforcementGap => "exec-error-enforcement-gap",
                    _ => "exec-error-server",
                },
                code: None,
                server_text: None,
                hint_id: None,
                doc_offset: None,
            };
            self.set_unit(ordinal, true, UnitStatus::Failed(view));
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
        if failure {
            self.result_info(result, |r| r.failed = Some(reason));
            if lock(&self.state).current == Some(ordinal) {
                return;
            }
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

    // ## m3b-t16-run-service (T13 review M-2; fix round 1, review m9)
    /// A PostgreSQL Run as one that failed or was cancelled: started outside a transaction of
    /// the user's, it ran as one implicit transaction, so the statements before the failure
    /// were rolled back with it; started inside one, nothing was rolled back yet, but the
    /// user's transaction failed and waits for their rollback.
    fn run_as_one_aborted(&self, unit: UnitId, u: &UnitInfo) {
        if !u.run_as_one || !pg_wire(self.engine) {
            return;
        }
        let at_start = lock(&self.state)
            .tx_at_start
            .get(&unit.ordinal)
            .copied()
            .unwrap_or(TxStatus::Idle);
        let msg = if at_start == TxStatus::Idle {
            LogMsg::RunAsOneRolledBack
        } else {
            LogMsg::RunAsOneTxFailed
        };
        self.line(msg, Some(u));
    }

    /// The user's transaction moved: its end is announced when the unit in flight ends, by
    /// [`tx_line`] (a COMMIT can fail at commit time and still leave the session idle).
    fn on_tx_changed(&self, from: TxStatus, to: TxStatus) {
        lock(&self.state).tx_now = to;
        if to != TxStatus::Idle || from == TxStatus::Idle {
            return;
        }
        let mut st = lock(&self.state);
        if let Some(current) = st.current {
            st.tx_ended = Some((from, current));
        }
    }

    fn on_run_ended(&self, summary: RunSummary) {
        let done = {
            let mut st = lock(&self.state);
            st.ended = true;
            st.done.take()
        };
        // ## m3b-t16-run-service: the units the user skipped belong to the run (R-M3-84), and
        // what the chain's sinks had no room for is said once.
        let summary = RunSummary {
            total: summary.total.saturating_add(self.skipped_by_user),
            skipped: summary.skipped.saturating_add(self.skipped_by_user),
            ..summary
        };
        let dropped = lock(&self.chain).dropped();
        if dropped > 0 {
            self.line(LogMsg::Dropped { n: dropped }, None);
        }
        self.update(true, |s| s.summary = Some(summary));
        if summary.total > 1 {
            self.line(LogMsg::Summary(summary), None);
        }
        if let Some(done) = done {
            let _ = done.send(summary);
        }
    }
}

/// What a unit whose statement `sql` moved the user's transaction from `from` to idle says
/// about it, once the unit ended as `outcome`:
/// - `COMMIT`/`END` that succeeded: committed (unless the transaction had failed, which
///   PostgreSQL ends with a rollback);
/// - `ROLLBACK`/`ABORT`, or any of these four that failed: rolled back (the server ended the
///   transaction without committing it: a failed COMMIT, as a deferred constraint or a
///   serialization failure, rolls it back);
/// - anything else (an implicit end, a loss, a cancel): nothing. Better silent than wrong.
fn tx_line(sql: &str, from: TxStatus, outcome: &UnitEnd) -> Option<LogMsg> {
    let word = first_word(sql);
    let is = |w: &str| word.eq_ignore_ascii_case(w);
    let commit = is("COMMIT") || is("END");
    let rollback = is("ROLLBACK") || is("ABORT");
    match outcome {
        UnitEnd::Succeeded { .. } if commit && from != TxStatus::Failed => {
            Some(LogMsg::TxCommitted)
        }
        UnitEnd::Succeeded { .. } if commit || rollback => Some(LogMsg::TxRolledBack),
        UnitEnd::Failed(_) if commit || rollback => Some(LogMsg::TxRolledBack),
        _ => None,
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
            unit: info.clone(),
        });
        let source = StoreSource::new(store.clone(), control, self.fetch);
        // ## m3b-t16-run-service: the console's log hears of the result's eviction.
        if let Some(u) = &info {
            self.evictions.register(
                id,
                EvictTarget {
                    log: Arc::downgrade(&self.log),
                    store: Arc::downgrade(&store),
                    range: u.range.clone(),
                    unit: u.id,
                },
            );
        }
        let tabs: Arc<[ResultTab]> = {
            let mut st = lock(&self.state);
            // ## m3b-t16-run-service: the comment title, else `Result N` (N counts the run's
            // units that opened a tab), ` (k)` for a further set (R-M3-66).
            let next = st.numbered.len() + 1;
            let n = *st.numbered.entry(unit.ordinal).or_insert(next);
            let title = tab_title(info.as_ref().and_then(|u| u.title.as_deref()), n, ordinal);
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
                    failed: None,
                },
            );
            st.tabs.iter().cloned().collect()
        };
        self.results.send_replace(tabs);
        writer
    }

    fn event(&self, ev: &RunEvent) {
        // ## m3b-t16-run-service: the auditor chain first (R-M3-63).
        lock(&self.chain).on_event(ev);
        match ev {
            RunEvent::UnitStarted { unit, at } => {
                // ## m3b-t16-run-service: an answered question; a failed unit run again.
                self.clear_choice();
                if matches!(self.unit_status(unit.ordinal), Some(UnitStatus::Failed(_))) {
                    let u = self.unit(unit.ordinal).cloned();
                    self.line(
                        LogMsg::Retried {
                            n: unit.ordinal.saturating_add(1),
                        },
                        u.as_ref(),
                    );
                }
                {
                    let mut st = lock(&self.state);
                    st.current = Some(unit.ordinal);
                    let now = st.tx_now;
                    st.tx_at_start.insert(unit.ordinal, now);
                }
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
            // ## m3b-t16-run-service (ledger 340): the first command tag goes on the unit's line.
            RunEvent::UpdateCount {
                unit,
                command_tag: Some(tag),
                ..
            } => {
                lock(&self.state)
                    .tags
                    .entry(unit.ordinal)
                    .or_insert_with(|| Arc::from(tag.as_str()));
            }
            RunEvent::UpdateCount { .. } => {}
            RunEvent::MoreResultSets { unit, skipped } => {
                let u = self.unit(unit.ordinal).cloned();
                // ## m3b-t16-run-service (R-M3-66): past a prepared unit's 100 sets.
                let msg = if u.as_ref().is_some_and(|u| u.multi) {
                    LogMsg::ResultSetLimit {
                        total: MAX_RESULT_SETS.saturating_add(*skipped),
                        limit: MAX_RESULT_SETS,
                    }
                } else {
                    LogMsg::MoreResultSets { skipped: *skipped }
                };
                self.line(msg, u.as_ref());
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
            } => {
                self.clear_choice();
                self.on_unit_ended(*unit, outcome, *exec_ms, *fetch_ms);
            }
            RunEvent::RunEnded(summary) => {
                self.clear_choice();
                self.on_run_ended(*summary);
            }
            // ## m3b-t16-run-service
            RunEvent::NeedsChoice { .. } => self.publish_choice(ev),
            RunEvent::NeedsCommit {
                unit,
                question,
                affected,
                deadline,
                savepoint,
            } => {
                lock(&self.state)
                    .commit_savepoint
                    .insert(unit.ordinal, *savepoint);
                self.publish_choice(ev);
                self.forward_commit(*unit, *question, *affected, *deadline, *savepoint);
            }
            RunEvent::CommitDecided {
                unit,
                answer,
                by_deadline,
            } => {
                self.clear_choice();
                let u = self.unit(unit.ordinal).cloned();
                let savepoint = lock(&self.state)
                    .commit_savepoint
                    .remove(&unit.ordinal)
                    .unwrap_or(false);
                let msg = match answer {
                    CommitAnswer::Commit => LogMsg::ConfirmCommitted { rows: None },
                    CommitAnswer::RollBack => LogMsg::ConfirmRolledBack {
                        by_deadline: *by_deadline,
                        savepoint,
                    },
                };
                self.line(msg, u.as_ref());
            }
            RunEvent::Notice { unit, notice } => {
                let u = self.unit(unit.ordinal).cloned();
                self.line(
                    LogMsg::Notice {
                        severity: notice.severity,
                        code: notice.code.clone(),
                        text: notice.text.clone(),
                    },
                    u.as_ref(),
                );
            }
            RunEvent::NoticesDropped { unit, n } => {
                let u = self.unit(unit.ordinal).cloned();
                self.line(LogMsg::NoticesDropped { n: *n }, u.as_ref());
            }
            RunEvent::ClosedEarly { unit, report } => {
                let u = self.unit(unit.ordinal).cloned();
                // Fix round 1 (review m10): nothing to say when nothing more ran (the stop's
                // own line says the rest did not run).
                if report.statements_ran > 0 {
                    self.line(
                        LogMsg::ClosedEarly {
                            ran: report.statements_ran,
                            rest_not_run: report.rest_not_run,
                        },
                        u.as_ref(),
                    );
                }
            }
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
            // server said something; a command to a closed session (a close after a
            // disconnect) is no statement's failure.
            let quiet = matches!(
                e.kind,
                ExecErrorKind::Cancelled | ExecErrorKind::NotConnected
            ) || (matches!(e.kind, ExecErrorKind::ConnectionLost { .. })
                && e.server.is_none());
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
                    view: u.error_view(engine, &e),
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

#[cfg(test)]
mod tests {
    //! The observer fed synthetic run events: outcomes the FakeDriver cannot produce (a COMMIT
    //! that fails and leaves the connection idle, as PostgreSQL's deferred-constraint and
    //! serialization failures do).

    use std::sync::Arc;
    use std::time::Instant;

    use hacivat_driver::fake::FakeDriver;
    use hacivat_driver::{ServerMessage, TxStatus};
    use hacivat_proto::DbmsKind;
    use hacivat_results::{CloseReason, Completeness, FetchPolicy, MemoryBudget, ResultId};
    use hacivat_secrets::{AuthProviderId, Retention};
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::session::{
        ExecError, ExecErrorKind, RowsOutcome, RunEvent, RunId, RunObserver, RunSummary, UnitEnd,
        UnitId,
    };
    use hacivat_session::testing::fake_session;
    use hacivat_values::{ColumnMeta, LogicalType, NativeType, ResultSchema};
    use sql_split::exec_range::UnitKind;

    use super::{ExecObserver, RunParts};
    use crate::log::{LogMsg, SessionLog};
    use crate::status::UnitStatus;
    use crate::units::ExecUnit;

    const RUN: RunId = RunId(1);

    fn unit(ordinal: u32) -> UnitId {
        UnitId { run: RUN, ordinal }
    }

    async fn observer_of(sqls: &[&str]) -> (Arc<ExecObserver>, Arc<SessionLog>) {
        let profile = Profile {
            name: "orders".into(),
            engine: DbmsKind::Postgres,
            endpoint: ProfileEndpoint::Server {
                host: "127.0.0.1".into(),
                port: None,
            },
            user: Some("deploy".into()),
            auth: AuthProviderId::None,
            retention: Retention::Never,
            ..Profile::default()
        };
        let fake = FakeDriver::new(DbmsKind::Postgres);
        let (session, _f) = fake_session(tokio::runtime::Handle::current(), fake, profile).await;
        let log = SessionLog::new();
        let mut at = 0;
        let units = sqls
            .iter()
            .map(|s| {
                let u = ExecUnit {
                    text: (*s).into(),
                    doc_range: at..at + s.len(),
                    kind: UnitKind::Statement,
                    suspendable: s.starts_with("SELECT"),
                    title: None,
                };
                at += s.len() + 2;
                u
            })
            .collect();
        let (observer, _rx) = ExecObserver::new(RunParts {
            console: 1,
            run: RUN,
            engine: DbmsKind::Postgres,
            units,
            budget: MemoryBudget::new(1 << 30, 256 << 20),
            generations: Arc::default(),
            log: log.clone(),
            session,
            fetch: FetchPolicy::default(),
            // ## m3b-t16-run-service
            planned: None,
            chain: crate::audit::AuditorChain::new(),
            confirm: None,
            evictions: Arc::default(),
            rt: None,
        });
        (observer, log)
    }

    fn db_error(code: &str, text: &str) -> ExecError {
        ExecError {
            kind: ExecErrorKind::Db,
            server: Some(ServerMessage {
                sqlstate: Some(code.into()),
                vendor_code: None,
                severity: Some("ERROR".into()),
                text: text.into(),
                detail: None,
                hint: None,
                lang_hint: None,
                position: None,
            }),
        }
    }

    fn ids(log: &SessionLog) -> Vec<&'static str> {
        log.lines().iter().map(|l| l.msg.fluent_id()).collect()
    }

    /// Runs one unit whose statement moved the transaction from `from` to `to`, ending `end`.
    fn tx_unit(o: &ExecObserver, from: TxStatus, to: TxStatus, end: UnitEnd) {
        o.event(&RunEvent::UnitStarted {
            unit: unit(0),
            at: Instant::now(),
        });
        o.event(&RunEvent::TxChanged { from, to });
        o.event(&RunEvent::UnitEnded {
            unit: unit(0),
            outcome: end,
            exec_ms: 3,
            fetch_ms: 0,
        });
    }

    /// Review C2: a COMMIT that fails at commit time (SERIALIZABLE 40001, a deferred
    /// constraint) leaves PostgreSQL idle with the transaction rolled back: never "committed".
    #[tokio::test]
    async fn a_failed_commit_is_never_announced_as_committed() {
        for (code, text) in [
            (
                "40001",
                "could not serialize access due to read/write dependencies",
            ),
            (
                "23505",
                "duplicate key value violates unique constraint \"t_pkey\"",
            ),
        ] {
            let (o, log) = observer_of(&["COMMIT"]).await;
            tx_unit(
                &o,
                TxStatus::InTx,
                TxStatus::Idle,
                UnitEnd::Failed(db_error(code, text)),
            );
            let got = ids(&log);
            assert!(!got.contains(&"exec-tx-committed"), "{code}: {got:?}");
            assert_eq!(got, ["exec-error", "exec-tx-rolled-back"], "{code}");
        }
        // A COMMIT that succeeded is announced, after its own line.
        let (o, log) = observer_of(&["COMMIT"]).await;
        tx_unit(
            &o,
            TxStatus::InTx,
            TxStatus::Idle,
            UnitEnd::Succeeded { rows: None },
        );
        assert_eq!(ids(&log), ["exec-completed", "exec-tx-committed"]);
        // A failed ROLLBACK that ended the transaction committed nothing either.
        let (o, log) = observer_of(&["ROLLBACK"]).await;
        tx_unit(
            &o,
            TxStatus::InTx,
            TxStatus::Idle,
            UnitEnd::Failed(db_error("XX000", "internal error")),
        );
        assert_eq!(ids(&log), ["exec-error", "exec-tx-rolled-back"]);
        // A COMMIT that failed and left the transaction open says nothing about it.
        let (o, log) = observer_of(&["COMMIT"]).await;
        tx_unit(
            &o,
            TxStatus::InTx,
            TxStatus::InTx,
            UnitEnd::Failed(db_error("57014", "canceling statement")),
        );
        assert_eq!(ids(&log), ["exec-error"]);
    }

    fn schema() -> Arc<ResultSchema> {
        Arc::new(ResultSchema {
            columns: Arc::from(vec![ColumnMeta {
                name: "id".into(),
                logical: LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
                native: NativeType::Unknown,
                origin: None,
            }]),
        })
    }

    /// A suspended unit's result, released for `reason` after its run ended.
    async fn released(reason: CloseReason) -> UnitStatus {
        let (o, _log) = observer_of(&["SELECT * FROM big"]).await;
        o.event(&RunEvent::UnitStarted {
            unit: unit(0),
            at: Instant::now(),
        });
        let writer = o.open_result(unit(0), 0, schema());
        let result: ResultId = writer.id();
        o.event(&RunEvent::ResultOpened {
            unit: unit(0),
            ordinal: 0,
            result,
        });
        o.event(&RunEvent::UnitEnded {
            unit: unit(0),
            outcome: UnitEnd::Succeeded {
                rows: Some(RowsOutcome::Fetched {
                    rows: 20,
                    more: true,
                }),
            },
            exec_ms: 1,
            fetch_ms: 1,
        });
        o.event(&RunEvent::RunEnded(RunSummary {
            executed: 1,
            total: 1,
            ..RunSummary::default()
        }));
        assert_eq!(o.status.borrow().units[0].status.name(), "Suspended");
        o.event(&RunEvent::CursorReleased {
            result,
            rows: 30,
            reason,
            rolled_back: true,
        });
        o.event(&RunEvent::ResultEnded {
            result,
            rows: 30,
            completeness: Completeness::Closed { reason },
            exec_ms: 1,
            fetch_ms: 2,
        });
        drop(writer);
        o.status.borrow().units[0].status.clone()
    }

    /// Review I3: a release by a failure (a later fetch's server error, a loss, the guard's
    /// gap) fails the unit; a release the user or Hacivat chose keeps it succeeded.
    #[tokio::test]
    async fn a_release_by_a_failure_fails_the_unit() {
        for (reason, title) in [
            (CloseReason::ServerTerminated, "exec-error-server"),
            (CloseReason::ConnectionLost, "exec-error-connection-lost"),
            (CloseReason::EnforcementGap, "exec-error-enforcement-gap"),
        ] {
            match released(reason).await {
                UnitStatus::Failed(view) => assert_eq!(view.title_id, title, "{reason:?}"),
                other => panic!("{reason:?}: {other:?}"),
            }
        }
        for reason in [
            CloseReason::UserClosed,
            CloseReason::SessionNeeded,
            CloseReason::IdleTimeout,
            CloseReason::TabClosed,
            CloseReason::MemoryLimit,
            CloseReason::Cancelled,
        ] {
            assert!(
                matches!(
                    released(reason).await,
                    UnitStatus::Succeeded(u)
                        if u.rows == Some(RowsOutcome::Fetched { rows: 30, more: true })
                ),
                "{reason:?}"
            );
        }
    }

    // ## m3b-t16-run-service

    /// A port that records the commit questions and answers each `Commit`.
    #[derive(Debug, Default)]
    struct Port {
        asked: std::sync::Mutex<Vec<crate::gate::CommitQuestion>>,
    }

    impl crate::gate::ConfirmPort for Port {
        fn confirm(
            &self,
            _q: crate::gate::ConfirmQuestion,
        ) -> tokio::sync::oneshot::Receiver<crate::gate::ConfirmAnswer> {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(crate::gate::ConfirmAnswer::Cancel);
            rx
        }
        fn commit_decision(
            &self,
            q: crate::gate::CommitQuestion,
        ) -> tokio::sync::oneshot::Receiver<hacivat_session::session::CommitAnswer> {
            self.asked.lock().unwrap().push(q);
            let (tx, rx) = tokio::sync::oneshot::channel();
            let _ = tx.send(hacivat_session::session::CommitAnswer::Commit);
            rx
        }
    }

    /// Until Task 19 no session asks `NeedsCommit`; the observer's half is tested on a
    /// synthetic event: the question reaches the port and the ticket's `choices` with the
    /// run's request number, and the run's decision clears it and is logged.
    #[tokio::test]
    async fn needs_commit_reaches_the_confirm_port_and_answer_returns() {
        let profile = Profile {
            name: "orders".into(),
            engine: DbmsKind::Postgres,
            endpoint: ProfileEndpoint::Server {
                host: "127.0.0.1".into(),
                port: None,
            },
            auth: AuthProviderId::None,
            retention: Retention::Never,
            ..Profile::default()
        };
        let (session, _f) = fake_session(
            tokio::runtime::Handle::current(),
            FakeDriver::new(DbmsKind::Postgres),
            profile,
        )
        .await;
        let log = SessionLog::new();
        let port = Arc::new(Port::default());
        let (o, mut rx) = ExecObserver::new(RunParts {
            console: 1,
            run: RUN,
            engine: DbmsKind::Postgres,
            units: vec![ExecUnit {
                text: "UPDATE t SET a = 1".into(),
                doc_range: 0..18,
                kind: UnitKind::Statement,
                suspendable: false,
                title: None,
            }],
            budget: MemoryBudget::new(1 << 30, 256 << 20),
            generations: Arc::default(),
            log: log.clone(),
            session,
            fetch: FetchPolicy::default(),
            planned: None,
            chain: crate::audit::AuditorChain::new(),
            confirm: Some(port.clone()),
            evictions: Arc::default(),
            rt: Some(tokio::runtime::Handle::current()),
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(300);
        let ask = RunEvent::NeedsCommit {
            unit: unit(0),
            question: 7,
            affected: Some(3),
            deadline,
            savepoint: true,
        };
        // Asked before the request number is known: published once it is.
        o.event(&ask);
        assert!(rx.choices.borrow().is_none());
        o.set_seq(41);
        assert_eq!(*rx.choices.borrow_and_update(), Some((41, ask.clone())));
        for _ in 0..100 {
            if !port.asked.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let asked = port.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert_eq!(
            (
                asked[0].unit,
                asked[0].question,
                asked[0].affected,
                asked[0].savepoint
            ),
            (unit(0), 7, Some(3), true)
        );
        assert_eq!(asked[0].deadline, deadline);
        o.event(&RunEvent::CommitDecided {
            unit: unit(0),
            answer: hacivat_session::session::CommitAnswer::Commit,
            by_deadline: false,
        });
        assert!(rx.choices.borrow().is_none());
        assert_eq!(ids(&log), ["exec-confirm-committed"]);
    }

    /// A Run as one stopped early says what ran past the closed result (Task 13's report); a
    /// report of nothing says nothing.
    #[tokio::test]
    async fn a_closed_early_report_is_logged() {
        let (o, log) = observer_of(&["SELECT 1; DELETE FROM t"]).await;
        for (ran, rest) in [(2, false), (1, true), (0, false), (0, true)] {
            o.event(&RunEvent::ClosedEarly {
                unit: unit(0),
                report: hacivat_driver::CloseReport {
                    statements_ran: ran,
                    rest_not_run: rest,
                },
            });
        }
        let lines = log.lines();
        let got: Vec<&LogMsg> = lines.iter().map(|l| &l.msg).collect();
        assert_eq!(
            got,
            [
                &LogMsg::ClosedEarly {
                    ran: 2,
                    rest_not_run: false
                },
                &LogMsg::ClosedEarly {
                    ran: 1,
                    rest_not_run: true
                },
            ]
        );
        assert_eq!(lines[0].source, Some(0..23));
    }

    /// A DDL statement's end is `exec-completed` whatever count its server gave (ledger 340).
    #[test]
    fn ddl_is_known_by_its_first_word() {
        for ddl in [
            "CREATE TABLE t (a int)",
            "  alter table t add b int",
            "DROP VIEW v",
        ] {
            assert!(super::is_ddl(ddl), "{ddl}");
        }
        for not in ["INSERT INTO t VALUES (1)", "CREATED", "SELECT 1"] {
            assert!(!super::is_ddl(not), "{not}");
        }
    }
}
