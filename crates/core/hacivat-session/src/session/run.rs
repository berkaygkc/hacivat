//! `Request::Run` (HV-EXEC-05; Task 18): typed units in order, streamed into result stores in
//! chunk steps (the actor reads its commands between chunks, R-M3-16), `StopOnError`
//! (R-M3-26), one result set kept per unit (R-M3-25), and the last suspendable unit kept as a
//! console cursor (R-M3-23). Read-only sessions route each unit through the typed guard path
//! (`readonly::guarded_query`, Task 22: the guard's read-only transaction around the query, its
//! canary after the first chunk, its post steps when the hold ends).
//!
//! What a run tells its observer (ADR 0004 `m3a-t18`):
//! - per unit `UnitStarted`, then for a result with columns `ResultOpened` (the observer's
//!   `open_result` gives the store's writer; a statement without columns opens none),
//!   `RowsFetched` at most every 50 ms, `UpdateCount` when the server reported affected rows or
//!   the result has no columns, `MoreResultSets`, `TxChanged` when the user's transaction
//!   status moved, and `UnitEnded`;
//! - `ResultEnded` exactly once per result, when it reaches a final state (never for a result
//!   that stays `Suspended`: that one ends later, on a fetch or a release), and
//!   `CursorReleased` just before it when a suspendable query was closed before its end;
//! - `RunEnded` once the run started, whatever ends it (also when its future is dropped by a
//!   deactivate); the reply follows it. A reply without a `RunEnded` means the run never
//!   started (it was still queued when the session was cancelled, closed or lost).
//!
//! The reply is `Ok` when every unit ran, else the error of the unit that stopped the run
//! (`Cancelled` for a stop).
//!
//! ## m3b-t11-session-runs (ADR 0004 `m3b-t11`)
//! - **Binds** (HV-EXEC-03): `RunUnit::binds` reach the driver in slot order, only when its
//!   typed path binds (`TypedCaps::binds`) and never with a unit run "as one"; otherwise the
//!   unit is refused (`Internal`), nothing sent (fix round 1, I2).
//! - **Result sets** (R-M3-66): a unit keeps `max_result_sets` sets (at most 100), each its own
//!   result (`open_result(unit, ordinal, ..)`, the ordinal is the set's place in the unit); an
//!   earlier set's `ResultEnded` comes when the unit ends. Only the first set of the last unit
//!   may stay suspended.
//! - **Run as one** (R-M3-60): a `Protocol::Simple` unit is sent whole, never suspends, and is
//!   refused on a read-only session (`ReadOnlyDenied(Protocol)`), nothing sent.
//! - **Notices** (R-M3-77): `RunEvent::Notice`, at most 100 per unit; the rest are counted in a
//!   log line (`notices_dropped`), their text never logged.
//! - **The `Ask` policy** (R-M3-61, P-M3b-3, P-M3b-19): a failed unit with later units emits
//!   `NeedsChoice` and waits on the run gate while the actor serves cancels and closes; `Retry`
//!   runs it again (`UnitStarted`/`UnitEnded` again; the summary counts its last attempt),
//!   `Ignore` goes on, `IgnoreAll` goes on without asking again, `Stop` skips the rest (the
//!   reply is the unit's error), `RollbackAndContinue` sends `ROLLBACK`, keeps the mode and
//!   goes on. A cancel or a close answers `Stop`. A run that went on to its end replies `Ok`
//!   unless its last unit failed. Before it asks, and after the `ROLLBACK` an answer sends,
//!   the session publishes its transaction state (`Core::publish_tx`; still `Busy`).
//! - **A cancelled queued run** reports every unit `Cancelled` and `RunEnded` (ledger 318);
//!   one dropped from the queue for any other reason still never started.
//!
//! ## m3b-t19-confirm-tx (ADR 0004 `m3b-t19`)
//! - **Before anything of a unit is sent** (`Core::refused_before_send`): the implicit-commit
//!   backstop (a MySQL/MariaDB `COMMITS_FIRST` unit meeting an open transaction its verdict
//!   did not confirm: `Tx(ImplicitCommitUnconfirmed)`), and a wrapped unit run as one or on a
//!   read-only session.
//! - **After a Manual session's `BEGIN`**: Smart's `BEGIN` (`smart.rs`), then the wrap of a
//!   `TxThenConfirm` or `ExplainAnalyzeRollback` unit, its form chosen from the live state
//!   (`confirm.rs`, P-M3b-1). A wrapped unit never suspends.
//! - **After the statement**: `Core::finish_wrap` rolls back what failed, rolls back an
//!   `EXPLAIN ANALYZE`, or asks Commit/Roll back (Keep/Undo) and waits (D-014). An
//!   `ImplicitCommit` unit that ended the open transaction is logged.

use std::time::Instant;

use hacivat_driver::{QuerySpec, RawError, RawErrorKind, TxStatus};
use hacivat_results::{CloseReason, Completeness};

use crate::session::actor::{Core, TX_CONTROL};
use crate::session::cursor::{Fire, Hold, Pumped, SAVEPOINT, Step, Stream, Watch, millis};
use crate::session::readonly::{GuardFailure, guarded_query, may_write};
use crate::session::recovery::{autocommit_set, track_statement};
use crate::session::schema::{Family, family, first_keyword};
use crate::session::{
    DenyLayer, ExecError, ExecErrorKind, ObserverRef, Reply, RowsOutcome, RunEvent, RunSpec,
    RunSummary, RunUnit, TxError, TxMode, TxOwner, UnitEnd, UnitId, needs_schema_reprobe,
};
// ## m3b-t01c-core-seams
use crate::session::verdict::UnitVerdict;
// ## m3b-t19-confirm-tx
use crate::session::confirm::{Wrap, WrapKind};
// ## m3b-t11-session-runs
use crate::session::actor::ends_failed_tx;
use crate::session::cursor::MAX_RESULT_SETS;
use crate::session::run_gate::Answer;
use crate::session::{ErrorPolicy, RunChoice};
use hacivat_driver::Protocol;

/// How one unit ended.
struct Ended {
    outcome: UnitEnd,
    exec_ms: u64,
    fetch_ms: u64,
}

impl Ended {
    /// A unit that failed before a statement of its own was sent.
    fn refused(e: ExecError) -> Ended {
        Ended {
            outcome: UnitEnd::Failed(e),
            exec_ms: 0,
            fetch_ms: 0,
        }
    }

    fn of(s: &Stream, outcome: UnitEnd) -> Ended {
        Ended {
            outcome,
            exec_ms: s.exec_ms,
            fetch_ms: s.fetch_ms(),
        }
    }
}

fn cancelled() -> ExecError {
    ExecError::of(ExecErrorKind::Cancelled)
}

/// A run's bookkeeping. It always ends the run for the observer: a run whose future is dropped
/// (a deactivate drops the request with its connection; a panic unwinds it) still reports the
/// unit in flight, the units it never reached and `RunEnded`.
struct Tally {
    observer: ObserverRef,
    units: Vec<UnitId>,
    /// The first unit that has not ended.
    next: usize,
    /// The unit in flight.
    open: bool,
    summary: RunSummary,
    started: tokio::time::Instant,
    finished: bool,
}

impl Tally {
    fn new(spec: &RunSpec) -> Tally {
        Tally {
            observer: spec.observer.clone(),
            units: spec.units.iter().map(|u| u.id).collect(),
            next: 0,
            open: false,
            summary: RunSummary {
                total: u32::try_from(spec.units.len()).unwrap_or(u32::MAX),
                ..RunSummary::default()
            },
            started: tokio::time::Instant::now(),
            finished: false,
        }
    }

    fn emit(&self, event: &RunEvent) {
        self.observer.0.event(event);
    }

    fn start(&mut self, unit: UnitId, at: Instant) {
        self.open = true;
        self.emit(&RunEvent::UnitStarted { unit, at });
    }

    fn end(&mut self, ended: Ended) {
        let Some(unit) = self.units.get(self.next).copied() else {
            return;
        };
        match &ended.outcome {
            UnitEnd::Succeeded { .. } => self.summary.executed += 1,
            UnitEnd::Failed(_) => self.summary.failed += 1,
            UnitEnd::Skipped => self.summary.skipped += 1,
            UnitEnd::Cancelled => {}
        }
        self.open = false;
        self.next += 1;
        self.emit(&RunEvent::UnitEnded {
            unit,
            outcome: ended.outcome,
            exec_ms: ended.exec_ms,
            fetch_ms: ended.fetch_ms,
        });
    }

    // ## m3b-t11-session-runs
    /// The unit that just ended `Failed` runs again (`RunChoice::Retry`): its failed attempt is
    /// superseded by the next one, which reports `UnitStarted` and `UnitEnded` again.
    fn retry(&mut self) {
        self.next = self.next.saturating_sub(1);
        self.summary.failed = self.summary.failed.saturating_sub(1);
    }

    /// Ends the run: the units it never reached end as `rest` (`Skipped`, or `Cancelled` after
    /// a stop; both count as skipped), then `RunEnded`.
    fn finish(&mut self, rest: &UnitEnd, aborted: bool) {
        while self.next < self.units.len() {
            self.summary.skipped += 1;
            let unit = self.units[self.next];
            self.next += 1;
            self.emit(&RunEvent::UnitEnded {
                unit,
                outcome: rest.clone(),
                exec_ms: 0,
                fetch_ms: 0,
            });
        }
        self.summary.aborted = aborted;
        self.summary.duration_ms = millis(self.started.elapsed());
        self.finished = true;
        self.emit(&RunEvent::RunEnded(self.summary));
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        if self.open {
            let outcome = if std::thread::panicking() {
                UnitEnd::Failed(ExecError::of(ExecErrorKind::Internal))
            } else {
                UnitEnd::Cancelled
            };
            self.end(Ended {
                outcome,
                exec_ms: 0,
                fetch_ms: 0,
            });
        }
        self.finish(&UnitEnd::Skipped, true);
    }
}

impl Core {
    /// Runs `spec`; the reply comes after its `RunEvent::RunEnded` (the module docs).
    /// `released`: how the release of the session's earlier cursor went; a failure there means
    /// the connection cannot run anything, so every unit is skipped.
    pub(crate) async fn run(
        &mut self,
        spec: RunSpec,
        released: Result<(), ExecError>,
        w: &mut Watch,
    ) -> Reply {
        w.overdue = spec.fetch.overdue;
        let mut tally = Tally::new(&spec);
        if let Err(e) = released {
            let rest = match e.kind {
                ExecErrorKind::Cancelled => UnitEnd::Cancelled,
                _ => UnitEnd::Skipped,
            };
            tally.finish(&rest, true);
            return Err(e);
        }
        let count = spec.units.len();
        let mut failure = None;
        // ## m3b-t11-session-runs: the error policy (R-M3-61). `i` moves on unless a unit is
        // retried; `ignore_all` continues past later failures without asking.
        let mut ignore_all = false;
        let mut i = 0;
        while let Some(unit) = spec.units.get(i) {
            if w.stop.is_requested() {
                failure = Some(cancelled());
                break;
            }
            if self.broken.is_some() || self.live.is_none() {
                failure = Some(ExecError::of(ExecErrorKind::NotConnected));
                break;
            }
            tally.start(unit.id, tokio::time::Instant::now().into_std());
            let ended = self.run_unit(&spec, unit, i + 1 == count, w).await;
            let error = match &ended.outcome {
                UnitEnd::Failed(e) => Some(e.clone()),
                UnitEnd::Cancelled => {
                    failure = Some(cancelled());
                    None
                }
                UnitEnd::Succeeded { .. } | UnitEnd::Skipped => None,
            };
            tally.end(ended);
            if failure.is_some() {
                break;
            }
            let Some(error) = error else {
                i += 1;
                continue;
            };
            // A failure: `StopOnError`, a last unit, a stop asked for, or a connection that
            // cannot run anything more ends the run here.
            let asks = spec.policy == ErrorPolicy::Ask
                && i + 1 < count
                && !w.stop.is_requested()
                && self.broken.is_none()
                && self.live.is_some();
            if !asks {
                failure = Some(error);
                break;
            }
            let choice = if ignore_all {
                RunChoice::IgnoreAll
            } else {
                self.choice_after(&spec, unit.id, &error, w).await
            };
            match choice {
                RunChoice::Retry => tally.retry(),
                RunChoice::Ignore => i += 1,
                RunChoice::IgnoreAll => {
                    ignore_all = true;
                    i += 1;
                }
                RunChoice::RollbackAndContinue => match self.roll_back_to_continue(&spec).await {
                    Ok(()) => i += 1,
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                },
                RunChoice::Stop => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let stopped = failure
            .as_ref()
            .is_some_and(|e| e.kind == ExecErrorKind::Cancelled);
        let rest = if stopped {
            UnitEnd::Cancelled
        } else {
            UnitEnd::Skipped
        };
        tally.finish(&rest, failure.is_some());
        match failure {
            None => Ok(self.summary()),
            Some(e) => Err(e),
        }
    }

    // ## m3b-t11-session-runs
    /// The error policy's question after `unit` failed with `error` (R-M3-61; P-M3b-3): asks
    /// through the run gate, emits `NeedsChoice` with nothing awaited in between, and waits for
    /// the answer, the stop signal (a cancel) or a close, whichever comes first. A stop and a
    /// close answer `Stop`; a stop that races an answer wins. No cursor is open meanwhile: only
    /// the last unit keeps one, and the last unit never asks.
    async fn choice_after(
        &mut self,
        spec: &RunSpec,
        unit: UnitId,
        error: &ExecError,
        w: &mut Watch,
    ) -> RunChoice {
        let gate = self.gate.clone();
        if gate.closing() || w.stop.is_requested() {
            return RunChoice::Stop;
        }
        debug_assert!(self.cursor.is_none(), "no cursor is open while a run asks");
        // Roll back and continue (instead of Ignore and Retry) only inside the user's own
        // failed PostgreSQL transaction: a rollback never ends one the question did not name.
        let failed_tx = family(self.kind) == Family::Postgres
            && self.tx.status == TxStatus::Failed
            && !self.tx.owner.is_hacivats();
        // Fix round 1 (review I1): the snapshot shows the transaction the question is about
        // (status, owner, statements) while the run waits; the session stays `Busy`.
        self.publish_tx();
        let question = gate.ask_choice(self.request_seq, unit, failed_tx);
        spec.observer.0.event(&RunEvent::NeedsChoice {
            unit,
            question,
            error: error.clone(),
            rollback_and_continue: failed_tx,
        });
        let answer = tokio::select! {
            biased;
            () = w.stop.requested() => None,
            answer = gate.answered() => Some(answer),
        };
        gate.withdraw();
        match answer {
            _ if w.stop.is_requested() => RunChoice::Stop,
            Some(Answer::Choice(choice)) => choice,
            Some(Answer::Commit(_)) | None => RunChoice::Stop,
        }
    }

    /// `RunChoice::RollbackAndContinue` (P-M3b-19): `ROLLBACK` ends the user's failed
    /// transaction (logged, never its text), the session keeps its mode (a Manual session
    /// opens its next transaction before the next unit, as for any unit), and the run goes on.
    async fn roll_back_to_continue(&mut self, spec: &RunSpec) -> Result<(), ExecError> {
        let before = self.shown_status();
        let rolled = self.control("ROLLBACK").await;
        if rolled.is_ok() {
            tracing::info!(
                run = spec.run.0,
                "roll back and continue: the user's failed transaction was rolled back"
            );
            // A rollback may undo a schema switch made inside the transaction (Q27).
            self.reprobe().await;
        }
        let after = self.shown_status();
        if before != after {
            spec.observer.0.event(&RunEvent::TxChanged {
                from: before,
                to: after,
            });
        }
        // Fix round 1 (review I1): what the answer ended is shown at once.
        self.publish_tx();
        rolled
    }

    /// The transaction status the user sees: Hacivat's own transaction (around a cursor, or
    /// the guard's) is not theirs.
    fn shown_status(&self) -> TxStatus {
        // ## m3b-t01c-core-seams (P-M3b-12): every owner of Hacivat's, Confirm included.
        if self.tx.owner.is_hacivats() {
            TxStatus::Idle
        } else {
            self.tx.status
        }
    }

    /// One unit: the statement, its result, and what a user statement leaves behind for the
    /// session (what a reconnect would lose, the schema context), as `Request::Exec` keeps it.
    async fn run_unit(
        &mut self,
        spec: &RunSpec,
        unit: &RunUnit,
        last: bool,
        w: &mut Watch,
    ) -> Ended {
        // ## m3b-t19-confirm-tx: the implicit-commit backstop and the wraps' refusals, before
        // anything of the unit is sent (the Manual `BEGIN` included).
        if let Some(e) = self.refused_before_send(unit) {
            return Ended::refused(e);
        }
        // ## m3b-t11-session-runs: binds go to the driver with the query (`query_spec`); a
        // driver that cannot bind refuses the query before sending it (`Unsupported`). A
        // read-only session never runs "as one" (R-M3-60): its guard judges one statement,
        // through the prepared protocol.
        if self.read_only && unit.protocol == Protocol::Simple {
            return Ended::refused(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                DenyLayer::Protocol,
            )));
        }
        // Fix round 1 (review I2): values are bound only where the driver says its typed path
        // binds, and never to a unit run "as one" (a simple query or a batch carries no
        // parameters: a driver could run its placeholders as NULL). Nothing is sent.
        if !unit.binds.is_empty() && (unit.protocol == Protocol::Simple || !self.typed_caps().binds)
        {
            return Ended::refused(ExecError::of(ExecErrorKind::Internal));
        }
        let keyword = first_keyword(&unit.sql);
        let control = TX_CONTROL.contains(&keyword.as_str());
        let before = self.shown_status();
        let mysql = family(self.kind) == Family::MySql;
        // ## m3b-t11-session-runs: a unit run "as one" is several statements; what the session
        // tracks reads each of them (a naive split on `;` errs toward "seen").
        let parts = statements_of(unit);
        // A user's "autocommit off" counts whatever happens next (it errs toward "off").
        if mysql && parts.iter().any(|p| autocommit_set(p) == Some(false)) {
            self.autocommit_off = true;
        }
        // Fix round 1 (T13 review I-1, ruling (b)): a unit run "as one" is stopped by the
        // server cancel at once, whatever the transaction state: waiting the overdue time
        // would let the server run later statements of the script meanwhile. A zero overdue
        // makes both the wait on a call and the close after a stop send it now.
        let overdue = w.overdue;
        if unit.protocol == Protocol::Simple {
            w.overdue = std::time::Duration::ZERO;
        }
        // ## m3b-t19-confirm-tx: whether a transaction was open before the unit (the implicit
        // commit's log line), and the wrap the unit runs in, if any.
        let open_before = self.tx_open_now();
        let mut wrap = None;
        let mut ended = self
            .unit_body(spec, unit, last, &keyword, control, &mut wrap, w)
            .await;
        w.overdue = overdue;
        if let Some(wrap) = wrap {
            ended.outcome = self.finish_wrap(spec, unit, wrap, ended.outcome, w).await;
        }
        let succeeded = matches!(ended.outcome, UnitEnd::Succeeded { .. });
        if succeeded
            && unit.verdict == UnitVerdict::ImplicitCommit
            && open_before
            && self.tx.status == TxStatus::Idle
        {
            tracing::info!(
                run = spec.run.0,
                unit = unit.id.ordinal,
                notice = "exec-implicit-commit",
                "the confirmed statement committed the open transaction first (implicit commit)"
            );
        }
        // Fix round 1 (review m2): a batch that failed or was stopped partway may have run
        // (and on MySQL committed) its first statements: its bookkeeping errs toward "seen".
        let batch = unit.protocol == Protocol::Simple
            && matches!(ended.outcome, UnitEnd::Failed(_) | UnitEnd::Cancelled);
        // Review m3: a batch's statements count in the user's open transaction (one is
        // counted by `observe`).
        if (succeeded || batch)
            && parts.len() > 1
            && self.tx.status != TxStatus::Idle
            && !self.tx.owner.is_hacivats()
        {
            let more = u32::try_from(parts.len() - 1).unwrap_or(u32::MAX);
            self.tx.statements = self.tx.statements.saturating_add(more);
        }
        if succeeded || batch {
            // ## m3b-t01c-core-seams (P-M3b-11): the gate's effects when it classified the
            // unit; Task 18 makes `track_statement` use them.
            let effects = (unit.verdict != UnitVerdict::Unknown).then_some(unit.effects);
            for part in &parts {
                track_statement(&mut self.tracked, part, effects);
            }
            // One statement says what it set; in a batch only an "off" is believed (a split
            // inside a string must not turn a real "off" back on).
            if succeeded
                && mysql
                && let [part] = parts.as_slice()
                && let Some(on) = autocommit_set(part)
            {
                self.autocommit_off = !on;
            }
            // Never under an open cursor: nothing else runs there.
            if self.cursor.is_none()
                && self.broken.is_none()
                && parts.iter().any(|p| needs_schema_reprobe(p))
            {
                self.reprobe().await;
            }
        }
        let after = self.shown_status();
        if before != after {
            spec.observer.0.event(&RunEvent::TxChanged {
                from: before,
                to: after,
            });
        }
        ended
    }

    fn query_spec<'a>(spec: &RunSpec, unit: &'a RunUnit, suspendable: bool) -> QuerySpec<'a> {
        QuerySpec {
            sql: &unit.sql,
            // ## m3b-t11-session-runs: the unit's bind values, in slot order (HV-EXEC-03).
            binds: &unit.binds,
            protocol: unit.protocol,
            suspendable,
            first_rows: spec.fetch.first_rows,
            chunk_rows: spec.fetch.chunk_rows,
            chunk_bytes: spec.fetch.chunk_bytes,
            // ## m3b-t01c-core-seams: the unit's own cap (R-M3-82) when it has one.
            cell_cap: unit.cell_cap.unwrap_or(spec.fetch.cell_cap),
            // ## m3b-t11-session-runs: the unit's result sets, at most 100 (R-M3-66).
            max_result_sets: result_sets(unit),
        }
    }

    // ## m3b-t11-session-runs
    /// A new stream of `unit`, keeping its result sets.
    fn stream(spec: &RunSpec, unit: &RunUnit, suspendable: bool) -> Stream {
        let mut s = Stream::new(unit.id, spec.observer.clone(), spec.fetch, suspendable);
        s.max_sets = result_sets(unit);
        s
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "crate-private; `run_unit` is its one caller (m3b-t19: the wrap it opens)"
    )]
    async fn unit_body(
        &mut self,
        spec: &RunSpec,
        unit: &RunUnit,
        last: bool,
        keyword: &str,
        control: bool,
        wrap: &mut Option<Wrap>,
        w: &mut Watch,
    ) -> Ended {
        // Advisory (first keyword only): the server refuses anyway.
        // ## m3b-t11-session-runs: `ABORT` is PostgreSQL's `ROLLBACK` (R-M3-74).
        if self.tx.status == TxStatus::Failed && !ends_failed_tx(self.kind, keyword) {
            return Ended::refused(ExecError::of(ExecErrorKind::Tx(
                TxError::FailedNeedsRollback,
            )));
        }
        let lost = || RawError {
            kind: RawErrorKind::Io(std::io::ErrorKind::NotConnected),
            server: None,
        };
        let engine = family(self.kind);
        let caps = self
            .live
            .as_mut()
            .map_or(hacivat_driver::TypedCaps::NONE, |l| l.raw().typed_caps());
        // ## m3b-t11-session-runs: a unit run "as one" never suspends (R-M3-60, R-M3-66).
        let mut suspendable = unit.suspendable
            && unit.protocol != Protocol::Simple
            && caps.suspend
            && engine != Family::Other;
        let mut hold = Hold::None;
        let mut s;

        // ## m3a-f5-demo (DEMO-7): a read-only console refuses a statement that can change data
        // or schema before anything is sent; the guard's transaction stays behind this layer.
        if self.read_only && may_write(self.kind, &unit.sql) {
            return Ended::refused(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                DenyLayer::Input,
            )));
        }
        if self.read_only {
            // ## m3b-t01c-core-seams (P-M3b-11): step 0b, the injected classifier, before the
            // guard (a stub until Task 18).
            if let Err(e) = self.step0b(&unit.sql).await {
                return Ended::refused(e);
            }
            // The guard opens its own read-only transaction and sends the query (Task 22).
            self.holder = TxOwner::Guard;
            s = Self::stream(spec, unit, suspendable);
            let q = Self::query_spec(spec, unit, suspendable);
            let kind = self.kind;
            // ## m3a-t22-typed-readonly: the guard ends itself when the query is refused at
            // once, so it needs the session's environment then.
            let replay = self.env_plan().replay();
            let opened = match self.live.as_mut() {
                Some(live) => {
                    w.guard(guarded_query(live.raw(), kind, q, &replay), Fire::OnStop)
                        .await
                }
                None => Err(GuardFailure {
                    error: ExecError::of(ExecErrorKind::NotConnected),
                    discard: None,
                }),
            };
            self.note_settle(w);
            match opened {
                Ok(guard) => hold = Hold::Guard(guard),
                Err(failure) => {
                    self.holder = TxOwner::User;
                    if let Some(class) = failure.discard {
                        self.broken.get_or_insert(class);
                    }
                    self.observe(false);
                    s.on_first_response();
                    return Ended::of(&s, UnitEnd::Failed(failure.error));
                }
            }
        } else {
            // Manual mode opens the user's transaction first, as `Request::Exec` does.
            if self.tx.mode == TxMode::Manual && self.tx.status == TxStatus::Idle && !control {
                for statement in self.manual_begin(keyword) {
                    if let Err(e) = self.control(&statement).await {
                        return Ended::refused(e);
                    }
                }
            }
            // ## m3b-t19-confirm-tx: Smart opens the user's transaction before a unit that is
            // not a read (C2), then a confirmed statement or an `EXPLAIN ANALYZE` gets its wrap,
            // its form chosen from the transaction open now (P-M3b-1). A wrapped statement runs
            // to its end inside the wrap: it never suspends.
            if self.smart_opens_before(unit, control) {
                for statement in self.smart_begin(keyword) {
                    if let Err(e) = self.control(&statement).await {
                        return Ended::refused(e);
                    }
                }
            }
            if let Some(kind) = WrapKind::of(unit.verdict) {
                match self.open_wrap(kind).await {
                    Ok(opened) => *wrap = Some(opened),
                    Err(e) => return Ended::refused(e),
                }
                suspendable = false;
            }
            // A PostgreSQL portal lives in a transaction block (P5): Hacivat's own in
            // autocommit, a savepoint inside the user's (R-M3-23). A failed transaction takes
            // no savepoint: the unit runs as it is and the server refuses it.
            if suspendable && engine == Family::Postgres {
                match self.tx.status {
                    TxStatus::Idle => {
                        self.holder = TxOwner::Cursor;
                        if let Err(e) = self.control("BEGIN").await {
                            self.holder = TxOwner::User;
                            self.observe(false);
                            return Ended::refused(e);
                        }
                        hold = Hold::OwnTx;
                    }
                    TxStatus::InTx => {
                        if let Err(e) = self.control(SAVEPOINT).await {
                            return Ended::refused(e);
                        }
                        hold = Hold::Savepoint;
                    }
                    TxStatus::Failed => suspendable = false,
                }
            }
            s = Self::stream(spec, unit, suspendable);
            let q = Self::query_spec(spec, unit, suspendable);
            // The overdue clock runs from here: a slow statement holds `query` itself until
            // its first header.
            let queried = match self.live.as_mut() {
                Some(live) => w.guard(live.raw().query(q), Fire::OnStop).await,
                None => Err(lost()),
            };
            self.note_settle(w);
            if let Err(e) = queried {
                return self.failed(s, hold, &e, control, w).await;
            }
        }

        let first = self.pump(&mut s, w).await;
        // The guard's canary, once the first rows are in and the query waits (R-M3-17).
        if matches!(first, Pumped::Suspended)
            && let Hold::Guard(guard) = &mut hold
        {
            let checked = match self.live.as_mut() {
                Some(live) => guard.after_first_chunk(live.raw()).await,
                None => Ok(()),
            };
            if let Err(failure) = checked {
                if let Some(class) = failure.discard {
                    self.broken.get_or_insert(class);
                }
                if self.broken.is_none() {
                    let _ = self
                        .close_open(hacivat_driver::CloseHow::Now, Fire::Never, false, w)
                        .await;
                }
                let (_, _) = self.end_hold(hold, false, None).await;
                let reason = match failure.error.kind {
                    ExecErrorKind::EnforcementGap(_) => CloseReason::EnforcementGap,
                    _ if self.broken.is_some() => CloseReason::ConnectionLost,
                    _ => CloseReason::ServerTerminated,
                };
                s.released(reason, false);
                s.finish(Completeness::Closed { reason });
                self.observe(false);
                return Ended::of(&s, UnitEnd::Failed(failure.error));
            }
        }
        let (target, bytes) = if s.suspendable {
            (
                u64::from(spec.fetch.read_ahead_rows),
                Some(spec.fetch.read_ahead_bytes),
            )
        } else {
            (u64::MAX, None)
        };
        match self.fetch_until(&mut s, first, target, bytes, w).await {
            Step::Complete => {
                // ## m3b-t16-run-service (named exception; T13 review M-8)
                self.read_warnings(&mut s, w).await;
                self.completed(s, hold, control).await
            }
            // ## m3b-t11-session-runs: only the first set of the last unit stays suspended (a
            // unit with several sets never does, R-M3-66; the drivers never suspend one).
            Step::Suspend if last && s.ordinal == 0 => {
                let outcome = UnitEnd::Succeeded {
                    rows: Some(RowsOutcome::Fetched {
                        rows: s.rows,
                        more: true,
                    }),
                };
                let ended = Ended::of(&s, outcome);
                self.suspend(s, hold);
                self.observe(!control);
                ended
            }
            Step::Suspend => {
                // Only the last unit keeps its cursor: this one read ahead, and the next unit
                // needs the session (m3b-t11: also a later set that suspended).
                let outcome = UnitEnd::Succeeded {
                    rows: Some(RowsOutcome::Fetched {
                        rows: s.rows,
                        more: true,
                    }),
                };
                let ended = Ended::of(&s, outcome);
                let released = self
                    .release_stream(s, hold, CloseReason::SessionNeeded, w)
                    .await;
                self.observe(!control);
                match released {
                    Ok(()) => ended,
                    Err(e) => Ended {
                        outcome: UnitEnd::Failed(e),
                        ..ended
                    },
                }
            }
            Step::Stopped => {
                let (completed, whole, failure) = self.stop_stream(&mut s, hold, w).await;
                self.observe(!control);
                let outcome = match failure {
                    Some(e) => UnitEnd::Failed(e),
                    // The statement ran to its end before the stop reached it: it applied
                    // (in autocommit it committed), and is reported so.
                    None if completed => UnitEnd::Succeeded {
                        rows: (s.columns > 0).then_some(RowsOutcome::Fetched {
                            rows: s.rows,
                            more: !whole,
                        }),
                    },
                    None => UnitEnd::Cancelled,
                };
                Ended::of(&s, outcome)
            }
            Step::Failed(e) => self.failed(s, hold, &e, control, w).await,
        }
    }

    // ## m3b-t16-run-service (named exception; T13 review M-8, R-M3-77)
    /// MySQL's warnings of the statement that just ended: read (`SHOW WARNINGS`, the driver's
    /// `RawConn::warnings`) only when its `Done` counted some, and never on a read-only
    /// session: the guard's `ROLLBACK` follows at once, and nothing of them reaches the run
    /// (fix round 1, review m11: not even their count). Each one is a `RunEvent::Notice` of
    /// the unit under the same cap as a notice; a failure to read them is traced without any
    /// text and changes nothing else.
    async fn read_warnings(&mut self, s: &mut Stream, w: &mut Watch) {
        let counted = s.done.map_or(0, |d| d.warnings);
        if self.read_only || counted == 0 {
            return;
        }
        let read = match self.live.as_mut() {
            Some(live) => w.guard(live.raw().warnings(), Fire::OnStop).await,
            None => return,
        };
        self.note_settle(w);
        match read {
            Ok(warnings) => {
                for notice in warnings {
                    s.on_notice(notice);
                }
            }
            Err(e) => tracing::debug!(
                kind = ?e.kind,
                counted,
                "the statement's warnings could not be read"
            ),
        }
    }

    /// The unit's statement ran to its end.
    async fn completed(&mut self, mut s: Stream, hold: Hold, control: bool) -> Ended {
        // First what Hacivat opened (the commit, the guard's checks): a result is complete
        // only once that held.
        let (_, ended) = self.end_hold(hold, true, s.judged_end().as_ref()).await;
        self.observe(!control);
        let state = s.state_after(&ended);
        s.finish(state);
        let end = s.end.take();
        // "Rows affected" is the server's count, which no engine reports for a result that
        // returns rows (a read); a statement without columns reports it even when it has none.
        if let Some(end) = &end
            && (end.rows_affected.is_some() || s.columns == 0)
        {
            s.observer.0.event(&RunEvent::UpdateCount {
                unit: s.unit,
                rows: end.rows_affected,
                command_tag: end.command_tag.clone(),
            });
        }
        // ## m3b-t11-session-runs (fix round 1, review m1): the sets the stream did not keep
        // count too.
        let skipped = s.skipped_sets();
        if skipped > 0 {
            s.observer.0.event(&RunEvent::MoreResultSets {
                unit: s.unit,
                skipped,
            });
        }
        let outcome = match ended {
            Err(e) => UnitEnd::Failed(e),
            Ok(()) if s.columns > 0 => UnitEnd::Succeeded {
                rows: Some(RowsOutcome::Fetched {
                    rows: end.as_ref().map_or(s.seen, |e| e.rows.max(s.seen)),
                    more: false,
                }),
            },
            Ok(()) => UnitEnd::Succeeded {
                rows: end.and_then(|e| e.rows_affected).map(RowsOutcome::Affected),
            },
        };
        Ended::of(&s, outcome)
    }

    /// The driver failed the unit's statement.
    async fn failed(
        &mut self,
        mut s: Stream,
        hold: Hold,
        e: &RawError,
        control: bool,
        w: &mut Watch,
    ) -> Ended {
        let error = self.fail_stream(&mut s, hold, e, w).await;
        self.observe(!control);
        let stopped = w.stop.is_requested()
            && (error.kind == ExecErrorKind::Cancelled
                || crate::session::cancel::is_cancel_code(self.kind, &error));
        let outcome = if stopped {
            UnitEnd::Cancelled
        } else {
            UnitEnd::Failed(error)
        };
        Ended::of(&s, outcome)
    }
}

// ## m3b-t11-session-runs

/// The result sets `unit` keeps: its own count, at least 1 and at most [`MAX_RESULT_SETS`]
/// (R-M3-66).
fn result_sets(unit: &RunUnit) -> u32 {
    unit.max_result_sets.clamp(1, MAX_RESULT_SETS)
}

/// The statements of `unit` for the session's bookkeeping (what a reconnect would lose, a
/// schema switch, MySQL's `autocommit`): a unit run "as one" (`Protocol::Simple`) split on
/// every `;`, a split inside a quoted string included (it errs toward "seen"); any other unit
/// whole.
fn statements_of(unit: &RunUnit) -> Vec<&str> {
    if unit.protocol != Protocol::Simple {
        return vec![&unit.sql];
    }
    let parts: Vec<&str> = unit
        .sql
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        vec![&unit.sql]
    } else {
        parts
    }
}

/// A run that was still queued when a cancel dropped the queue (ledger 318): its observer
/// hears every unit end `Cancelled` (none started) and `RunEnded`, so it reads "cancelled"
/// whoever awaits its reply.
pub(crate) fn report_cancelled_queued(spec: &RunSpec) {
    Tally::new(spec).finish(&UnitEnd::Cancelled, true);
}

/// A run the session refused to queue (`QueueFull`, C9; fix round 1, review m6): its observer
/// hears its first unit end `Failed` with `error` (none started), the rest `Skipped`, then
/// `RunEnded`, so a run nobody awaits still shows why it never ran.
pub(crate) fn report_refused(spec: &RunSpec, error: &ExecError) {
    let mut tally = Tally::new(spec);
    if !spec.units.is_empty() {
        tally.end(Ended::refused(error.clone()));
    }
    tally.finish(&UnitEnd::Skipped, true);
}
