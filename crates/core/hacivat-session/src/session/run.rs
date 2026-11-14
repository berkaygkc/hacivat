//! `Request::Run` (HV-EXEC-05; Task 18): typed units in order, streamed into result stores in
//! chunk steps (the actor reads its commands between chunks, R-M3-16), `StopOnError`
//! (R-M3-26), one result set kept per unit (R-M3-25), and the last suspendable unit kept as a
//! console cursor (R-M3-23). Read-only sessions route each unit through the typed guard path
//! (`readonly::guarded_query`, Task 22; it fails closed until then).
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

use std::time::Instant;

use hacivat_driver::{QuerySpec, RawError, RawErrorKind, TxStatus};
use hacivat_results::{CloseReason, Completeness};

use crate::session::actor::{Core, TX_CONTROL};
use crate::session::cursor::{Fire, Hold, Pumped, SAVEPOINT, Step, Stream, Watch, millis};
use crate::session::readonly::{GuardFailure, guarded_query};
use crate::session::recovery::{autocommit_set, track_statement};
use crate::session::schema::{Family, family, first_keyword};
use crate::session::{
    ExecError, ExecErrorKind, ObserverRef, Reply, RowsOutcome, RunEvent, RunSpec, RunSummary,
    RunUnit, TxError, TxMode, TxOwner, UnitEnd, UnitId, needs_schema_reprobe,
};

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
        for (i, unit) in spec.units.iter().enumerate() {
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
            match &ended.outcome {
                UnitEnd::Failed(e) => failure = Some(e.clone()),
                UnitEnd::Cancelled => failure = Some(cancelled()),
                UnitEnd::Succeeded { .. } | UnitEnd::Skipped => {}
            }
            tally.end(ended);
            if failure.is_some() {
                break;
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

    /// The transaction status the user sees: Hacivat's own transaction (around a cursor, or
    /// the guard's) is not theirs.
    fn shown_status(&self) -> TxStatus {
        match self.tx.owner {
            TxOwner::Cursor | TxOwner::Guard => TxStatus::Idle,
            TxOwner::User | TxOwner::None => self.tx.status,
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
        let keyword = first_keyword(&unit.sql);
        let control = TX_CONTROL.contains(&keyword.as_str());
        let before = self.shown_status();
        let mysql = family(self.kind) == Family::MySql;
        // A user's "autocommit off" counts whatever happens next (it errs toward "off").
        if mysql && autocommit_set(&unit.sql) == Some(false) {
            self.autocommit_off = true;
        }
        let ended = self.unit_body(spec, unit, last, &keyword, control, w).await;
        if matches!(ended.outcome, UnitEnd::Succeeded { .. }) {
            track_statement(&mut self.tracked, &unit.sql);
            if mysql && let Some(on) = autocommit_set(&unit.sql) {
                self.autocommit_off = !on;
            }
            // Never under an open cursor: nothing else runs there.
            if self.cursor.is_none() && self.broken.is_none() && needs_schema_reprobe(&unit.sql) {
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
            binds: &[],
            protocol: unit.protocol,
            suspendable,
            first_rows: spec.fetch.first_rows,
            chunk_rows: spec.fetch.chunk_rows,
            chunk_bytes: spec.fetch.chunk_bytes,
            cell_cap: spec.fetch.cell_cap,
            max_result_sets: 1,
        }
    }

    async fn unit_body(
        &mut self,
        spec: &RunSpec,
        unit: &RunUnit,
        last: bool,
        keyword: &str,
        control: bool,
        w: &mut Watch,
    ) -> Ended {
        // Advisory (first keyword only): the server refuses anyway.
        if self.tx.status == TxStatus::Failed && keyword != "ROLLBACK" {
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
        let mut suspendable = unit.suspendable && caps.suspend && engine != Family::Other;
        let mut hold = Hold::None;
        let mut s;

        if self.read_only {
            // The guard opens its own read-only transaction and sends the query (Task 22).
            self.holder = TxOwner::Guard;
            s = Stream::new(unit.id, spec.observer.clone(), spec.fetch, suspendable);
            let q = Self::query_spec(spec, unit, suspendable);
            let kind = self.kind;
            let opened = match self.live.as_mut() {
                Some(live) => {
                    w.guard(guarded_query(live.raw(), kind, q), Fire::OnStop)
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
            s = Stream::new(unit.id, spec.observer.clone(), spec.fetch, suspendable);
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
                let (_, _) = self.end_hold(hold, false).await;
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
            Step::Complete => self.completed(s, hold, control).await,
            Step::Suspend if last => {
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
                // needs the session.
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
                let (completed, failure) = self.stop_stream(&mut s, hold, w).await;
                self.observe(!control);
                let outcome = match failure {
                    Some(e) => UnitEnd::Failed(e),
                    // The statement ran to its end before the stop reached it: it applied
                    // (in autocommit it committed), and is reported so.
                    None if completed => UnitEnd::Succeeded {
                        rows: (s.columns > 0).then_some(RowsOutcome::Fetched {
                            rows: s.rows,
                            more: s.seen > s.rows,
                        }),
                    },
                    None => UnitEnd::Cancelled,
                };
                Ended::of(&s, outcome)
            }
            Step::Failed(e) => self.failed(s, hold, &e, control, w).await,
        }
    }

    /// The unit's statement ran to its end.
    async fn completed(&mut self, mut s: Stream, hold: Hold, control: bool) -> Ended {
        // First what Hacivat opened (the commit, the guard's checks): a result is complete
        // only once that held.
        let (_, ended) = self.end_hold(hold, true).await;
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
        if let Some(skipped) = s.done.map(|d| d.skipped_result_sets)
            && skipped > 0
        {
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
