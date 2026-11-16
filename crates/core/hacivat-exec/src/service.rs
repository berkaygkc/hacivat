//! `ExecService` (HV-EXEC-05; Task 23): one `Request::Run` per run on the console's session,
//! with result stores on the app's shared budget, run status, result tabs and log lines.
//!
//! Each unit is sent as resolved: its strict text, byte for byte, with the extended protocol,
//! `StopOnError` (R-M3-26), and `suspendable` from its cursor eligibility (the session keeps
//! only the run's last unit suspended). One result tab per unit's first result set, keyed
//! `TabKey { console, unit_ordinal, 0 }`: a re-run reuses the key, and its store's generation
//! counts the runs into that tab.
//!
//! ## m3b-t16-run-service (ADR 0006 `m3b-t16`)
//! - **Two paths.** A request without `prepared`, `plan` and `run_as_one` is M3a's run,
//!   unchanged (the app's until Task 24 rewires it). A request with `prepared` *and* `plan`
//!   is an M3b run: each unit is sent as `PreparedUnit::final_text` with its binds, the gate
//!   item's verdict and effects, its masked text, up to 100 result sets (R-M3-66), and as one
//!   (`Protocol::Simple`, never suspended) when `run_as_one`.
//! - **The plan is checked, never assumed** (fail closed; an empty plan never means "allow
//!   all"): it must hold exactly one item per prepared unit, in order, the prepared units must
//!   be the request's units, and every item must be decided: `Run`, `RunInTxThenConfirm` and
//!   `ExplainAnalyzeRollback` are sent; a unit the user skipped ([`apply_answer`]) is not sent
//!   (`Skipped`, one `SkippedByUser` line; R-M3-84); a `Confirm` or `Warn` the user was never
//!   asked, a `Deny`, a plan without prepared units (or the reverse), or `run_as_one` without
//!   them sends nothing at all.
//! - **Questions** (`RunTicket::choices`): a `NeedsChoice` or `NeedsCommit` waits there with
//!   the run's request number; the app answers a choice with `SessionHandle::choose`, and the
//!   service answers a commit question through `RunRequest::confirm` (R-M3-61, R-M3-84).
//! - **Count rows** ([`ExecService::count`], R-M3-76): a separate query only the user's Count
//!   rows sends, never implicitly; classified again and refused unless the read-only check
//!   allows it; on a pooled `Service` session when the console holds no transaction of the
//!   user's, else on the console (after its cursor is released, as for any run); 30 s, then
//!   cancelled; cancellable. A read-only (or production) session's guard applies to it as to
//!   any statement: nothing here unlocks production (R-M3-52).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hacivat_driver::Protocol;
use hacivat_proto::DbmsKind;
use hacivat_results::{FetchPolicy, MemoryBudget, ResultSource};
use hacivat_session::session::{
    ErrorPolicy, ObserverRef, RunEvent, RunId, RunSpec, RunSummary, RunUnit, SessionHandle, UnitId,
};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};

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
// ## m3b-t16-run-service
use crate::audit::{AuditorChain, HistorySink, HistoryUnit};
use crate::gate::{ConfirmAnswer, GateDecision, GateReason};
use crate::history::HistoryRecord;
use crate::observer::{Evictions, MAX_RESULT_SETS, PlannedUnit};
use crate::titles::short_title;
use hacivat_session::session::{
    PooledSession, RoCheck, SafetyDenial, SessionEffects, SessionKind, SessionManager,
    StatementClassifier, UnitVerdict,
};
use hacivat_values::BindValue;

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
    /// `Result N` in M3a; the unit's comment title in M3b, ` (k)` for its further sets, whole
    /// (the tooltip's text; [`ResultTab::short_title`] is what the tab shows).
    pub title: Arc<str>,
    pub unit: UnitId,
    pub source: Arc<dyn ResultSource>,
}

// ## m3b-t16-run-service
impl ResultTab {
    /// The title as the tab shows it: cut after 40 graphemes with `…` (the tooltip keeps
    /// [`title`](Self::title) whole). User text, never translated or case-mapped.
    pub fn short_title(&self) -> Arc<str> {
        short_title(&self.title)
    }

    /// Whether the tab shows its title cut (a tooltip then shows it whole).
    pub fn title_cut(&self) -> bool {
        *self.short_title() != *self.title
    }
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
    /// The prepared units (params, final text; Task 15), one per unit of `units`, in order.
    /// Sent only with a `plan` (the module docs).
    pub prepared: Option<Vec<PreparedUnit>>,
    /// The gate's plan with the user's answers applied ([`apply_answer`]; Task 24): one item
    /// per prepared unit, every one decided (the module docs).
    pub plan: Option<GatePlan>,
    /// `Ask` for a multi-unit run (R-M3-61); the session asks from Task 11 on.
    pub policy: ErrorPolicy,
    /// Send the units as one (R-M3-60): only with `prepared` and `plan`.
    pub run_as_one: bool,
    /// Where the history records the run (Task 17); `None` records nothing, and so does a
    /// private console (`HistoryMeta::private`). A profile that records no history gets
    /// `None` (Task 24; the store drops its records too).
    pub history: Option<HistoryMeta>,
    /// Where confirm-in-transaction questions go (Task 21's port); `None` rolls back.
    pub confirm: Option<Arc<dyn ConfirmPort>>,
    // ## m3b-t24-exec-ui (fix round 2, named exception)
    /// The session's context generation the units were classified under
    /// (`SessionHandle::run_context`): the session refuses the run, sending nothing, when it
    /// has moved since (`RunSpec::context_generation`). `None`: not checked.
    pub context_generation: Option<u64>,
}

// ## m3b-t16-run-service

/// The reason a plan item carries once the user skipped its unit ([`apply_answer`]): the unit
/// is not sent, and the log counts it (`exec-skipped-by-user`).
pub const SKIPPED_BY_USER: &str = "exec-skipped-by-user";

/// Applies the user's `answer` to the `Confirm` or `Warn` item of unit `unit` in `plan`
/// (R-M3-84): `Execute`, `ExecuteAll` and `ExecuteAndSuppress` make it `Run`; `Skip` and
/// `SkipAll` mark it skipped (never sent). Any other item (a `Deny` above all) is left as it
/// is, whatever the answer: an answer never turns a denial into a run. Returns `false` for
/// `Cancel`: the caller then sends nothing at all.
///
/// One unit per call (review m14): `ExecuteAll` and `SkipAll` answer only `unit`; the caller
/// applies them to every later `Confirm`/`Warn` itself. A `unit` the plan does not hold
/// changes nothing (and returns `true`); the run then still refuses any item left unanswered.
pub fn apply_answer(plan: &mut GatePlan, unit: u32, answer: ConfirmAnswer) -> bool {
    if answer == ConfirmAnswer::Cancel {
        return false;
    }
    let Some(item) = plan.items.iter_mut().find(|i| i.unit == unit) else {
        return true;
    };
    if !matches!(
        item.decision,
        GateDecision::Confirm { .. } | GateDecision::Warn(_)
    ) {
        return true;
    }
    item.decision = match answer {
        ConfirmAnswer::Execute | ConfirmAnswer::ExecuteAll | ConfirmAnswer::ExecuteAndSuppress => {
            GateDecision::Run
        }
        ConfirmAnswer::Skip | ConfirmAnswer::SkipAll | ConfirmAnswer::Cancel => {
            GateDecision::Deny(GateReason::of(SKIPPED_BY_USER))
        }
    };
    true
}

/// What an M3b request sends per unit, or why it sends nothing.
enum Planned {
    /// M3a's run.
    M3a,
    M3b(Vec<(PlannedUnit, UnitVerdict, SessionEffects)>),
    Refused,
}

impl RunRequest {
    /// The request's plan, checked (the module docs).
    fn planned(&self) -> Planned {
        let (prepared, plan) = match (&self.prepared, &self.plan) {
            (None, None) if !self.run_as_one => return Planned::M3a,
            (Some(prepared), Some(plan)) => (prepared, plan),
            _ => return Planned::Refused,
        };
        if prepared.len() != plan.items.len()
            || prepared.len() != self.units.len()
            || prepared.iter().zip(&self.units).any(|(p, u)| p.unit != *u)
        {
            return Planned::Refused;
        }
        let mut out = Vec::with_capacity(prepared.len());
        for (i, (p, item)) in prepared.iter().zip(&plan.items).enumerate() {
            if usize::try_from(item.unit).ok() != Some(i) {
                return Planned::Refused;
            }
            let skipped = match &item.decision {
                GateDecision::Run
                | GateDecision::RunInTxThenConfirm { .. }
                | GateDecision::ExplainAnalyzeRollback => false,
                GateDecision::Deny(r) if r.fluent_id == SKIPPED_BY_USER => true,
                GateDecision::Deny(_) | GateDecision::Confirm { .. } | GateDecision::Warn(_) => {
                    return Planned::Refused;
                }
            };
            out.push((
                PlannedUnit {
                    sent: p.final_text.clone(),
                    offsets: p.offsets.clone(),
                    run_as_one: self.run_as_one,
                    ddl: item.effects.contains(SessionEffects::DDL),
                    // ## m3b-t19-confirm-tx (named exception, fix round 2, R1-I1)
                    commits_first: item.effects.contains(SessionEffects::COMMITS_FIRST),
                    skipped,
                    // ## m3b-t24-exec-ui (named exception): the log derives its lines from it.
                    verdict: item.verdict,
                },
                item.verdict,
                item.effects,
            ));
        }
        Planned::M3b(out)
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
    // ## m3b-t16-run-service
    /// The question the run waits on (`NeedsChoice`, `NeedsCommit`) with the run's request
    /// number (`SessionHandle::choose(seq, question, ..)` names both); `None` once the run
    /// moved on.
    pub choices: watch::Receiver<Option<(u64 /*seq*/, RunEvent)>>,
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
    // ## m3b-t16-run-service
    /// Where the runs' history records go (the store's sink, R-M3-63).
    history_sink: Option<mpsc::Sender<Arc<HistoryRecord>>>,
    /// What decides whether a count statement may run (the pool's read-only check).
    count_check: Option<Arc<dyn StatementClassifier>>,
    evictions: Arc<Evictions>,
}

impl fmt::Debug for ExecService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecService")
            .field("next_run", &self.next_run.load(Ordering::Relaxed))
            .field("runtime", &self.runtime.is_some())
            .field("history", &self.history_sink.is_some())
            .field("classifier", &self.classifier.is_some())
            .finish_non_exhaustive()
    }
}

impl ExecService {
    /// A service whose result stores share `budget`. Built inside a Tokio runtime, it awaits
    /// replies there; elsewhere give it one with [`with_runtime`](Self::with_runtime).
    pub fn new(budget: Arc<MemoryBudget>) -> Self {
        // ## m3b-t16-run-service: evictions on this budget reach their consoles' logs.
        let evictions = Arc::new(Evictions::default());
        let listener = Arc::downgrade(&evictions);
        budget.on_evicted(move |e| {
            if let Some(ev) = listener.upgrade() {
                ev.on_evicted(e);
            }
        });
        ExecService {
            budget,
            next_run: AtomicU64::new(1),
            generations: Arc::new(Mutex::new(HashMap::new())),
            runtime: Handle::try_current().ok(),
            history: None,
            classifier: None,
            history_sink: None,
            count_check: None,
            evictions,
        }
    }

    // ## m3b-t01c-core-seams
    /// The history every run of this service records into (Task 17).
    #[must_use]
    pub fn with_history(mut self, h: Arc<HistoryStore>) -> Self {
        self.history_sink = Some(h.sink());
        self.history = Some(h);
        self
    }

    /// The classifier pool this service classifies with (Tasks 14, 16, 24); it also decides
    /// whether a count statement may run (its read-only check).
    #[must_use]
    pub fn with_classifier(mut self, p: Arc<ClassifyPool>) -> Self {
        self.count_check = Some(p.clone() as Arc<dyn StatementClassifier>);
        self.classifier = Some(p);
        self
    }

    // ## m3b-t16-run-service
    /// Where the runs' history records go: the store's sink ([`with_history`](Self::with_history)
    /// sets it), or any channel (tests).
    #[must_use]
    pub fn with_history_sink(mut self, tx: mpsc::Sender<Arc<HistoryRecord>>) -> Self {
        self.history_sink = Some(tx);
        self
    }

    /// What decides whether a count statement may run, in place of the classifier pool's
    /// read-only check (a test's classifier).
    #[must_use]
    pub fn with_count_check(mut self, c: Arc<dyn StatementClassifier>) -> Self {
        self.count_check = Some(c);
        self
    }

    /// "Count rows" (R-M3-76; the module docs): `req.sql` (from `count::count_sql`) checked
    /// again (in the context of the session it runs on, read from that session) and run on a
    /// pooled `Service` session or on the console. Nothing is sent when
    /// the check refuses it, or without a check (fail closed).
    pub fn count(&self, req: CountRequest) -> CountTicket {
        let (tx, done) = oneshot::channel();
        let (cancel, cancelled) = watch::channel(false);
        let ticket = CountTicket { done, cancel };
        let Some(rt) = self.runtime.clone() else {
            let _ = tx.send(Err(CountError::Failed(Box::new(ExecError::of(
                ExecErrorKind::Internal,
            )))));
            return ticket;
        };
        let check = self.count_check.clone();
        let budget = self.budget.clone();
        let run = RunId(self.next_run.fetch_add(1, Ordering::SeqCst));
        rt.spawn(async move {
            // Fix round 1 (reviews m7, m8): the cancel and the 30 s cover the whole count (the
            // check and the pooled session's connect too); a ticket dropped after its `done`
            // was taken cancels nothing.
            let sent: SentCount = Arc::default();
            let mut cancelled = cancelled;
            let cancel = async move {
                if cancelled.wait_for(|c| *c).await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            let answer = tokio::select! {
                biased;
                () = cancel => Err(CountError::Cancelled),
                r = tokio::time::timeout(COUNT_TIMEOUT, count_on(req, check, budget, run, sent.clone())) => {
                    r.unwrap_or(Err(CountError::TimedOut))
                }
            };
            if matches!(answer, Err(CountError::Cancelled | CountError::TimedOut)) {
                let held = lock_sent(&sent).take();
                if let Some((session, pooled, seq)) = held {
                    drop(session.cancel(seq));
                    drop(pooled);
                }
            }
            let _ = tx.send(answer);
        });
        ticket
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
        // ## m3b-t16-run-service: the request's plan, checked (the module docs).
        let planned = req.planned();
        let refused = matches!(planned, Planned::Refused);
        let (units, plan): (Arc<[RunUnit]>, Option<Vec<PlannedUnit>>) = match planned {
            // M3a's path: no binds, no verdict, one result set per unit.
            Planned::M3a | Planned::Refused => (
                req.units
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
                    .collect(),
                None,
            ),
            Planned::M3b(items) => {
                let prepared = req.prepared.as_deref().unwrap_or_default();
                let units = items
                    .iter()
                    .zip(prepared)
                    .enumerate()
                    .filter(|(_, ((p, _, _), _))| !p.skipped)
                    .map(|(i, ((p, verdict, effects), prep))| RunUnit {
                        id: UnitId {
                            run,
                            ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                        },
                        sql: prep.final_text.clone(),
                        protocol: if req.run_as_one {
                            Protocol::Simple
                        } else {
                            Protocol::Extended
                        },
                        // R-M3-23 (T15 carry): a cursor only for a unit the gate found a
                        // Read; never one run as one. T15-merge: `apply_read_verdicts` does
                        // the same on the prepared units; this check stays as the last word.
                        suspendable: prep.unit.suspendable
                            && !p.run_as_one
                            && *verdict == UnitVerdict::Read,
                        binds: prep.binds.clone(),
                        verdict: *verdict,
                        effects: *effects,
                        masked: prep.masked.clone(),
                        max_result_sets: MAX_RESULT_SETS,
                        cell_cap: None,
                    })
                    .collect();
                (units, Some(items.into_iter().map(|(p, _, _)| p).collect()))
            }
        };
        let empty = units.is_empty();
        let policy = req.policy;
        let chain = self.chain_for(console, run, &req);
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
            planned: plan,
            chain,
            confirm: req.confirm,
            evictions: self.evictions.clone(),
            rt: self.runtime.clone(),
        });
        let ticket = |seq| RunTicket {
            seq,
            run,
            status: rx.status,
            results: rx.results,
            done: rx.done,
            choices: rx.choices,
        };
        // Fail closed: a request whose plan does not hold sends nothing; ignoring it could
        // send what the gate denied.
        if refused {
            observer.never_started(Some(&ExecError::of(ExecErrorKind::Internal)));
            return ticket(u64::MAX);
        }
        // ## m3b-t16-run-service: the user's skips (R-M3-84).
        observer.log_skipped_by_user();
        if empty {
            observer.never_started(None);
            // Names no request: a cancel of it answers `AlreadyFinished`.
            return ticket(u64::MAX);
        }
        let sent = console.run(RunSpec {
            run,
            units,
            // ## m3b-t01c-core-seams: the request's policy (`StopOnError` in M3a).
            policy,
            fetch: req.fetch,
            observer: ObserverRef(observer.clone()),
            confirm_deadline: RunSpec::CONFIRM_DEADLINE,
            context_generation: req.context_generation,
        });
        let seq = sent.seq;
        observer.set_seq(seq);
        match &self.runtime {
            Some(rt) => {
                rt.spawn(async move {
                    // The reply follows `RunEnded`; one without it never started.
                    let reply = sent.reply.await;
                    if !observer.ended() {
                        observer.never_started(reply.as_ref().ok().and_then(|r| r.as_ref().err()));
                    }
                });
            }
            // Without a runtime the observer's drop ends a run that never started.
            None => drop(observer),
        }
        ticket(seq)
    }

    // ## m3b-t16-run-service
    /// The auditor chain of a run (R-M3-63): the history's sink when the request records and
    /// its console is not private.
    fn chain_for(&self, console: &SessionHandle, run: RunId, req: &RunRequest) -> AuditorChain {
        let mut chain = AuditorChain::new();
        let (Some(tx), Some(meta)) = (&self.history_sink, &req.history) else {
            return chain;
        };
        if meta.private {
            return chain;
        }
        let units: Arc<[HistoryUnit]> = req
            .units
            .iter()
            .enumerate()
            .map(|(i, u)| {
                let prepared = req.prepared.as_ref().and_then(|p| p.get(i));
                HistoryUnit {
                    id: UnitId {
                        run,
                        ordinal: u32::try_from(i).unwrap_or(u32::MAX),
                    },
                    // Never the text sent: its masked form (M3a's units carry no binds).
                    masked: prepared.map_or_else(|| u.text.clone(), |p| p.masked.clone()),
                    binds: prepared.map_or_else(|| Arc::from(Vec::new()), |p| p.binds.clone()),
                }
            })
            .collect();
        let mut sink = HistorySink {
            meta: meta.clone(),
            run,
            units,
            session: console.clone(),
            started: HashMap::new(),
            run_started: std::time::SystemTime::now(),
        };
        chain.push_sink(tx.clone(), move |ev| sink.map(ev));
        chain
    }
}

// ## m3b-t16-run-service

/// How long a count may run before it is cancelled.
pub const COUNT_TIMEOUT: Duration = Duration::from_secs(30);

/// One "Count rows" (R-M3-76).
#[derive(Debug, Clone)]
pub struct CountRequest {
    /// The console whose result is counted.
    pub console: SessionHandle,
    /// The console's connection's sessions: a count with no transaction of the user's open
    /// borrows a pooled `Service` session there; `None` counts on the console.
    pub sessions: Option<Arc<SessionManager>>,
    pub engine: DbmsKind,
    /// The count statement (`count::count_sql`, or `count::count_prepared`'s).
    pub sql: String,
    // Fix round 1 (review m6)
    /// The bind values of a prepared unit's count (`count::count_prepared`), in slot order;
    /// empty for a unit without placeholders.
    pub binds: Arc<[BindValue]>,
}

/// Why a count gave no number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountError {
    /// The read-only check refused the statement (or there is no check): nothing was sent.
    Refused(SafetyDenial),
    /// It ran past [`COUNT_TIMEOUT`] and was cancelled.
    TimedOut,
    /// The user cancelled it.
    Cancelled,
    /// The session or the server failed it.
    Failed(Box<ExecError>),
}

/// A count under way.
#[derive(Debug)]
pub struct CountTicket {
    pub done: oneshot::Receiver<Result<u64, CountError>>,
    cancel: watch::Sender<bool>,
}

impl CountTicket {
    /// Cancels the count: its check, its pooled session's connect or its statement, whichever
    /// it is at. Dropping the ticket (or keeping only `done`) never cancels it.
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
}

/// The class Fluent ID of a count refused because nothing can check it.
const NO_CHECK_CLASS_ID: &str = "safety-class-unknown";

/// A count's statement once sent: its session (a pooled one is held here until the end), the
/// pooled session itself, and the request number to cancel.
type SentCount = Arc<Mutex<Option<(SessionHandle, Option<PooledSession>, u64)>>>;

fn lock_sent(
    s: &SentCount,
) -> std::sync::MutexGuard<'_, Option<(SessionHandle, Option<PooledSession>, u64)>> {
    s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The count (the caller bounds it with the cancel and the timeout; the module docs).
async fn count_on(
    req: CountRequest,
    check: Option<Arc<dyn StatementClassifier>>,
    budget: Arc<MemoryBudget>,
    run: RunId,
    sent: SentCount,
) -> Result<u64, CountError> {
    let sql: Arc<str> = Arc::from(req.sql);
    let Some(check) = check else {
        return Err(CountError::Refused(SafetyDenial {
            class_id: NO_CHECK_CLASS_ID,
            reasons: vec![sql_safety::ReasonCode::NotImplemented.fluent_id()],
        }));
    };
    // The console's own transaction sees what the count must see; otherwise a pooled session
    // keeps the console free.
    let user_tx = {
        let snap = req.console.state();
        let tx = &snap.borrow().tx;
        !tx.owner.is_hacivats() && tx.status != hacivat_driver::TxStatus::Idle
    };
    let pooled: Option<PooledSession> = match (&req.sessions, user_tx) {
        (Some(sessions), false) => match sessions.pooled(SessionKind::Service).await {
            Ok(p) => Some(p),
            Err(_) => {
                return Err(CountError::Failed(Box::new(ExecError::of(
                    ExecErrorKind::NotConnected,
                ))));
            }
        },
        _ => None,
    };
    let session: SessionHandle = pooled.as_deref().unwrap_or(&req.console).clone();
    // ## m3b-t24-exec-ui (named exception, T18 m12, the T24 I1 ruling): checked in the context
    // of the session the count runs on, as that session reads it now (never another
    // session's, never a stale one).
    let ctx = session.classifier_context().await;
    match check
        .check_read_only_in(req.engine, sql.clone(), Arc::from(Vec::new()), ctx)
        .await
        .check
    {
        RoCheck::Allow => {}
        RoCheck::Deny(d) => return Err(CountError::Refused(d)),
    }
    let observer = Arc::new(crate::count::CountObserver::new(budget));
    let unit = RunUnit {
        verdict: UnitVerdict::Read,
        binds: req.binds.clone(),
        ..RunUnit::plain(UnitId { run, ordinal: 0 }, sql, Protocol::Extended, false)
    };
    let ticket = session.run(RunSpec {
        run,
        units: Arc::from(vec![unit]),
        policy: ErrorPolicy::StopOnError,
        fetch: FetchPolicy::default(),
        observer: ObserverRef(observer.clone()),
        confirm_deadline: RunSpec::CONFIRM_DEADLINE,
        context_generation: None,
    });
    *lock_sent(&sent) = Some((session, pooled, ticket.seq));
    let reply = ticket.reply.await;
    // Done: the pooled session goes back to its pool here.
    drop(lock_sent(&sent).take());
    match reply {
        Ok(Ok(_)) => observer
            .count()
            .ok_or(CountError::Failed(Box::new(ExecError::of(
                ExecErrorKind::Internal,
            )))),
        Ok(Err(e)) => Err(CountError::Failed(Box::new(e))),
        Err(_) => Err(CountError::Failed(Box::new(ExecError::of(
            ExecErrorKind::NotConnected,
        )))),
    }
}
