// ## m3b-t01c-core-seams
//! Confirm-in-transaction (HV-EXEC-04 session side, D-014, R-M3-55, P-M3b-1, P-M3b-5;
//! Task 19): a `TxThenConfirm` unit runs inside a transaction, then the run waits for
//! `SessionHandle::commit_decision` through the `RunGate` (`RunEvent::NeedsCommit`), with the
//! D-014 deadline (`RunSpec::confirm_deadline`). The session picks the form from the live
//! transaction state: `SAVEPOINT hv_confirm` (Keep/Undo) whenever a transaction is open,
//! Hacivat's own `BEGIN` (`TxOwner::Confirm`, Commit/Roll back) only when it is truly idle in
//! Auto. The D-062 wrap of `EXPLAIN ANALYZE` lives here too.
//!
//! ## m3b-t19-confirm-tx (ADR 0004 `m3b-t19`)
//! - **The form, per unit, at execution time** (P-M3b-1). After a Manual session's `BEGIN` and
//!   Smart's (`run.rs`), a transaction is open when the protocol says so, and on MySQL/MariaDB
//!   also in Manual mode and when the user turned `autocommit` off (a transaction is then
//!   always open, MySQL 8.4 §15.3.1): the **savepoint form** (`SAVEPOINT hv_confirm`, the
//!   statement, `NeedsCommit { savepoint: true }`, Keep → `RELEASE SAVEPOINT hv_confirm`, Undo
//!   → `ROLLBACK TO SAVEPOINT hv_confirm` then `RELEASE SAVEPOINT hv_confirm`; the owner stays
//!   the user's and the transaction stays open in every branch). Otherwise the **own form**:
//!   `BEGIN` (`START TRANSACTION` on MySQL/MariaDB, `BEGIN IMMEDIATE` on SQLite) with
//!   `holder = Confirm`, the statement, `NeedsCommit { savepoint: false }`, Commit → `COMMIT`,
//!   anything else → `ROLLBACK`. So MySQL/MariaDB never get `START TRANSACTION` inside a
//!   transaction, and no path of this module ever sends `COMMIT` while the user's transaction
//!   is open.
//! - **The wait** asks the gate first (`RunGate::ask`), emits `NeedsCommit` with nothing
//!   awaited in between, and selects (biased) on the stop signal, the answer and the deadline.
//!   A cancel beats an answer that raced it; a cancel, the deadline and a close (the gate
//!   answers it, `RunGate::close`) roll back. `CommitDecided { by_deadline }` follows the
//!   rollback or the commit once the server answered it.
//! - **A failure or cancel of the statement** (P-M3b-5) rolls back only the statement (the
//!   savepoint form; the user's transaction returns to `InTx`) or Hacivat's own transaction; the
//!   unit keeps its error. No question is asked.
//! - **A lost connection** at the end (the commit or the rollback cannot be sent): the server
//!   rolled back, `CommitDecided` is not emitted and the unit ends `Failed(ConnectionLost)`.
//! - **The D-062 wrap** (`ExplainAnalyzeRollback`): the same two forms with `hv_explain`, and
//!   always the rollback: `BEGIN … ROLLBACK` when idle, `SAVEPOINT hv_explain … ROLLBACK TO
//!   SAVEPOINT hv_explain; RELEASE SAVEPOINT hv_explain` inside a transaction.
//! - **Never silent:** every rollback is logged (never the statement's text) and the
//!   transaction state is published (`Core::publish_tx`, `SessionEvent::TxChanged`).
//! - **Fail closed:** a wrapped unit run "as one" (`Protocol::Simple`, which could end the
//!   transaction itself) or on a read-only session is refused before anything is sent; a
//!   transaction a statement ended itself is reported, never asked about.
//!
//! Fix round 1 (controller rulings on the review):
//! - **C1.** "Open" reads MySQL's own `autocommit` flag first, so a Manual session whose
//!   server turned `autocommit` back on takes the own form. After a statement that succeeded
//!   with nothing open (it committed on its own), nothing is asked and nothing is rolled back
//!   to: the unit ends `Tx(ConfirmAlreadyCommitted)` (`exec-confirm-already-committed`) and the
//!   run stops.
//! - **C2.** The deadline ends the unit `Tx(ConfirmTimedOut)` and the run stops whatever its
//!   policy; the user's Undo (and a close) ends it `Tx(ConfirmUndone)`, so the error policy
//!   decides. Commit/Keep leaves it `Succeeded`; a cancel `Cancelled`.
//! - **I1.** A statement that failed with nothing open any more (the server rolled back the
//!   whole transaction: a SQLite interrupt, a MySQL deadlock) sends no rollback, publishes the
//!   state and keeps the session (`exec-tx-rolled-back-by-server`); "intact"
//!   (`exec-confirm-statement-failed`) is logged only after an undo that worked.
//! - **m1.** A refused `RELEASE` (after Keep, or after a `ROLLBACK TO` that worked) is logged and
//!   the session stays; only a refused `ROLLBACK TO` breaks it. **m3.** A stop asked for while
//!   the statement ran rolls back without a question. **m5.** No own-form `START TRANSACTION`
//!   over MySQL table locks (`Tx(TableLocksHeld)`, nothing sent).

use hacivat_driver::{Protocol, TxStatus};
use hacivat_proto::ErrClass;

use crate::session::actor::Core;
use crate::session::cursor::Watch;
use crate::session::run_gate::{Answer, Ask};
use crate::session::schema::{Family, family};
use crate::session::smart::leading_words;
use crate::session::tx::begin_sql;
use crate::session::verdict::{SessionEffects, UnitVerdict};
use crate::session::{
    CommitAnswer, DenyLayer, ExecError, ExecErrorKind, RowsOutcome, RunEvent, RunSpec, RunUnit,
    TxError, TxMode, TxOwner, UnitEnd,
};

/// What Hacivat wraps around a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WrapKind {
    /// Confirm-in-transaction (`UnitVerdict::TxThenConfirm`).
    Confirm,
    /// The D-062 rollback around `EXPLAIN ANALYZE` (`UnitVerdict::ExplainAnalyzeRollback`).
    Explain,
}

impl WrapKind {
    /// The wrap a unit of `verdict` needs.
    pub(crate) fn of(verdict: UnitVerdict) -> Option<WrapKind> {
        match verdict {
            UnitVerdict::TxThenConfirm => Some(WrapKind::Confirm),
            UnitVerdict::ExplainAnalyzeRollback => Some(WrapKind::Explain),
            UnitVerdict::Read
            | UnitVerdict::Write
            | UnitVerdict::Unknown
            | UnitVerdict::ImplicitCommit => None,
        }
    }

    fn savepoint(self) -> &'static str {
        match self {
            WrapKind::Confirm => "SAVEPOINT hv_confirm",
            WrapKind::Explain => "SAVEPOINT hv_explain",
        }
    }

    fn rollback_to(self) -> &'static str {
        match self {
            WrapKind::Confirm => "ROLLBACK TO SAVEPOINT hv_confirm",
            WrapKind::Explain => "ROLLBACK TO SAVEPOINT hv_explain",
        }
    }

    fn release(self) -> &'static str {
        match self {
            WrapKind::Confirm => "RELEASE SAVEPOINT hv_confirm",
            WrapKind::Explain => "RELEASE SAVEPOINT hv_explain",
        }
    }
}

/// Which transaction holds the wrapped statement (P-M3b-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Form {
    /// Hacivat's own transaction (`TxOwner::Confirm`): the session was truly idle in Auto.
    Own,
    /// A savepoint inside the open transaction, which stays the user's.
    Savepoint,
}

/// The wrap a unit runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Wrap {
    pub(crate) kind: WrapKind,
    pub(crate) form: Form,
    /// The protocol's status once the wrap was open: when it was open then and the statement
    /// left none, the statement ended the transaction itself.
    opened: TxStatus,
}

/// How ending the wrap went.
enum Undone {
    Done,
    /// Fix round 2 (R1-m2): Keep's `RELEASE` was refused and left the transaction failed
    /// (PostgreSQL): the kept statement will not survive the user's `COMMIT`.
    TxFailed(ExecError),
    /// The connection is gone: the server rolled back.
    Lost(ExecError),
    /// The server refused; the session is broken (nothing may run in a transaction Hacivat
    /// could not end), or, for a refused `COMMIT`, the server rolled back.
    Refused(ExecError),
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
    /// The rollback ran, but the server warned that a table does not support transactions
    /// (MySQL/MariaDB 1196, 1751, 1752): the statement's changes stay.
    ChangesStayed,
}

// ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
/// The MySQL/MariaDB warnings a rollback raises when a table could not be rolled back:
/// `ER_WARNING_NOT_COMPLETE_ROLLBACK` and its two temporary-table forms.
const NOT_COMPLETE_ROLLBACK: [&str; 3] = ["1196", "1751", "1752"];

impl Core {
    /// Whether a transaction is open now, for the form (P-M3b-1), Smart and the implicit-commit
    /// backstop: the protocol's status, and on MySQL/MariaDB the server's own `autocommit`
    /// flag when the driver reports it (off: a transaction is always open, and `START
    /// TRANSACTION` would commit it), else Manual mode or the user's `SET autocommit = 0`.
    /// Fix round 1 (C1): a Manual MySQL session whose server turned `autocommit` back on (the
    /// user's `SET autocommit = 1` earlier in the run) has nothing open.
    pub(crate) fn tx_open_now(&self) -> bool {
        if self.tx.status != TxStatus::Idle {
            return true;
        }
        family(self.kind) == Family::MySql
            && match self.server_autocommit {
                Some(on) => !on,
                None => self.tx.mode == TxMode::Manual || self.autocommit_off,
            }
    }

    // ## m3b-t19-confirm-tx (fix round 1, m5)
    /// MySQL/MariaDB: the user's `LOCK TABLES` locks may be held, and a `START TRANSACTION`
    /// would release them.
    pub(crate) fn table_locks_block(&self) -> bool {
        family(self.kind) == Family::MySql && self.table_locks
    }

    /// Follows the user's table locks after `sql` ran (MySQL/MariaDB): `LOCK TABLE[S]` (or the
    /// gate's `SessionEffects::LOCK_TABLES`) takes them; `UNLOCK TABLE[S]`, `START TRANSACTION`
    /// and `BEGIN` release them. A reconnect and a reset clear them (`actor.rs`). Fix round 2
    /// (R1-m1): the words are read past leading comments and inside an executable comment, and
    /// only those statements release (not `START REPLICA`, `UNLOCK INSTANCE`, `BEGIN NOT
    /// ATOMIC`).
    pub(crate) fn note_table_locks(&mut self, sql: &str, effects: Option<SessionEffects>) {
        if family(self.kind) != Family::MySql {
            return;
        }
        let words = leading_words(sql, 2);
        let w = |i: usize| words.get(i).map_or("", String::as_str);
        match (w(0), w(1)) {
            ("LOCK", "TABLE" | "TABLES") => self.table_locks = true,
            ("UNLOCK", "TABLE" | "TABLES") | ("START", "TRANSACTION") => self.table_locks = false,
            ("BEGIN", second) if second != "NOT" => self.table_locks = false,
            ("UNLOCK" | "START" | "BEGIN", _) => {}
            _ if effects.is_some_and(|e| e.contains(SessionEffects::LOCK_TABLES)) => {
                self.table_locks = true;
            }
            _ => {}
        }
    }

    /// Why `unit` may not run here, before anything of it is sent (the Manual `BEGIN`
    /// included):
    /// - a wrapped unit run "as one" (its own statements could end the wrapping transaction) or
    ///   on a read-only session (the guard owns its transaction);
    /// - the implicit-commit backstop (P-M3b-1): a MySQL/MariaDB unit that commits an open
    ///   transaction first (`SessionEffects::COMMITS_FIRST`, the gate's list) meeting an open
    ///   transaction the gate did not confirm it for (its verdict is not `ImplicitCommit`: an
    ///   earlier unit of the run opened it).
    pub(crate) fn refused_before_send(&self, unit: &RunUnit) -> Option<ExecError> {
        if WrapKind::of(unit.verdict).is_some() {
            if self.read_only {
                return Some(ExecError::of(ExecErrorKind::ReadOnlyDenied(
                    DenyLayer::Transaction,
                )));
            }
            if unit.protocol == Protocol::Simple {
                return Some(ExecError::of(ExecErrorKind::Internal));
            }
        }
        if family(self.kind) == Family::MySql
            && unit.effects.contains(SessionEffects::COMMITS_FIRST)
            && unit.verdict != UnitVerdict::ImplicitCommit
            && self.tx_open_now()
        {
            tracing::warn!(
                unit = unit.id.ordinal,
                "a statement that commits the open transaction met one the confirmation did not \
                 name; nothing was sent"
            );
            return Some(ExecError::of(ExecErrorKind::Tx(
                TxError::ImplicitCommitUnconfirmed,
            )));
        }
        None
    }

    /// Opens the wrap of `kind` around the next statement (after the Manual or Smart
    /// `BEGIN`): the savepoint inside an open transaction, else Hacivat's own transaction
    /// (never over MySQL table locks, which it would release: `TableLocksHeld`, nothing sent).
    pub(crate) async fn open_wrap(&mut self, kind: WrapKind) -> Result<Wrap, ExecError> {
        let form = if self.tx_open_now() {
            Form::Savepoint
        } else {
            Form::Own
        };
        match form {
            Form::Savepoint => self.control(kind.savepoint()).await?,
            Form::Own => {
                if self.table_locks_block() {
                    return Err(ExecError::of(ExecErrorKind::Tx(TxError::TableLocksHeld)));
                }
                self.holder = TxOwner::Confirm;
                for statement in self.own_begin(kind) {
                    if let Err(e) = self.control(&statement).await {
                        self.end_own();
                        return Err(e);
                    }
                }
            }
        }
        Ok(Wrap {
            kind,
            form,
            opened: self.tx.status,
        })
    }

    /// What opens Hacivat's own transaction: PostgreSQL `BEGIN`, MySQL/MariaDB `START
    /// TRANSACTION` (after the isolation level, when one is set), SQLite `BEGIN IMMEDIATE`
    /// before a confirmed write (the write lock at once) and `BEGIN` around `EXPLAIN`.
    fn own_begin(&self, kind: WrapKind) -> Vec<String> {
        match (family(self.kind), kind) {
            (Family::Sqlite, WrapKind::Confirm) => vec!["BEGIN IMMEDIATE".to_string()],
            _ => begin_sql(self.kind, self.tx.isolation, false),
        }
    }

    /// Hacivat's own transaction is over (or never opened).
    fn end_own(&mut self) {
        self.holder = TxOwner::User;
        self.observe(false);
    }

    /// The connection is gone with Hacivat's own transaction: the session has no transaction
    /// of the user's to report lost.
    fn forget_own(&mut self) {
        self.holder = TxOwner::User;
        self.tx.status = TxStatus::Idle;
        self.tx.since = None;
        self.tx.statements = 0;
        self.tx.owner = TxOwner::None;
    }

    /// Ends the wrap after the unit's statement ended as `ended` (the module docs): rolls back
    /// what failed, rolls back an `EXPLAIN ANALYZE`, and asks about a confirmed statement.
    pub(crate) async fn finish_wrap(
        &mut self,
        spec: &RunSpec,
        unit: &RunUnit,
        wrap: Wrap,
        ended: UnitEnd,
        w: &mut Watch,
    ) -> UnitEnd {
        let run = spec.run.0;
        let ordinal = unit.id.ordinal;
        if self.broken.is_some() || self.live.is_none() {
            if wrap.form == Form::Own {
                self.forget_own();
            }
            tracing::info!(
                run,
                unit = ordinal,
                "the connection was lost inside Hacivat's wrap; the server rolled back these \
                 changes"
            );
            return match ended {
                UnitEnd::Succeeded { .. } => {
                    UnitEnd::Failed(ExecError::of(ExecErrorKind::ConnectionLost {
                        outcome_unknown: false,
                    }))
                }
                other => other,
            };
        }
        if !matches!(ended, UnitEnd::Succeeded { .. }) {
            return self.after_failure(run, ordinal, wrap, ended).await;
        }
        // Fix round 1 (C1): nothing is open after the statement, so it committed on its own (a
        // savepoint taken outside a transaction is a no-op on MySQL, or the statement ended the
        // transaction itself). Never asked, nothing rolled back to: reported, and the run stops
        // (`run.rs`).
        if self.tx.status == TxStatus::Idle {
            if wrap.form == Form::Own {
                self.end_own();
            }
            // Fix round 2 (R1-m3): an `EXPLAIN ANALYZE` has its own wording.
            let (reason, notice) = match wrap.kind {
                WrapKind::Confirm => (
                    TxError::ConfirmAlreadyCommitted,
                    "exec-confirm-already-committed",
                ),
                WrapKind::Explain => (
                    TxError::ExplainNotRolledBack,
                    "exec-explain-not-rolled-back",
                ),
            };
            tracing::warn!(
                run,
                unit = ordinal,
                savepoint = wrap.form == Form::Savepoint,
                notice,
                "the wrapped statement was committed by the server on its own; nothing was asked \
                 and the rest of the run is not sent"
            );
            self.publish_tx();
            return UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(reason)));
        }
        match wrap.kind {
            WrapKind::Explain => {
                let undone = self.undo(wrap).await;
                tracing::info!(
                    run,
                    unit = ordinal,
                    "EXPLAIN ANALYZE ran inside a transaction that was rolled back (D-062)"
                );
                self.publish_tx();
                match undone {
                    Undone::Done => ended,
                    // ## m3b-t26-prod-unlock (fix round 1)
                    Undone::ChangesStayed => {
                        UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmNotUndone)))
                    }
                    Undone::Lost(e) | Undone::Refused(e) | Undone::TxFailed(e) => {
                        UnitEnd::Failed(e)
                    }
                }
            }
            WrapKind::Confirm => self.decide(spec, unit, wrap, ended, w).await,
        }
    }

    /// The wrapped statement failed or was cancelled (P-M3b-5): only it is undone and the unit
    /// keeps its error. Fix round 1 (I1): when nothing is open any more, the server ended the
    /// transaction itself (a SQLite interrupt and a MySQL deadlock roll back the whole
    /// transaction): nothing is sent, the state is published, and the session stays.
    async fn after_failure(
        &mut self,
        run: u64,
        ordinal: u32,
        wrap: Wrap,
        ended: UnitEnd,
    ) -> UnitEnd {
        if self.tx.status == TxStatus::Idle {
            if wrap.form == Form::Own {
                self.end_own();
            }
            if wrap.opened != TxStatus::Idle {
                tracing::warn!(
                    run,
                    unit = ordinal,
                    savepoint = wrap.form == Form::Savepoint,
                    notice = "exec-tx-rolled-back-by-server",
                    "the wrapped statement failed and the server rolled back the whole \
                     transaction"
                );
            }
            self.publish_tx();
            return ended;
        }
        let undone = self.undo(wrap).await;
        if matches!(undone, Undone::Done) {
            match (wrap.kind, wrap.form) {
                (WrapKind::Confirm, Form::Savepoint) => tracing::info!(
                    run,
                    unit = ordinal,
                    notice = "exec-confirm-statement-failed",
                    "the confirmed statement failed or was cancelled; only it was rolled back, \
                     the transaction is intact"
                ),
                (WrapKind::Confirm, Form::Own) => tracing::info!(
                    run,
                    unit = ordinal,
                    "the confirmed statement failed or was cancelled; Hacivat's transaction \
                     around it was rolled back"
                ),
                (WrapKind::Explain, _) => tracing::info!(
                    run,
                    unit = ordinal,
                    "EXPLAIN ANALYZE failed or was cancelled; its rollback ran (D-062)"
                ),
            }
        }
        self.publish_tx();
        match undone {
            Undone::Done => ended,
            // ## m3b-t26-prod-unlock (fix round 1): the failed statement's error stays first.
            Undone::ChangesStayed => match ended {
                UnitEnd::Failed(first) => UnitEnd::Failed(first),
                _ => UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmNotUndone))),
            },
            Undone::Lost(e) | Undone::Refused(e) | Undone::TxFailed(e) => match ended {
                UnitEnd::Failed(first) => UnitEnd::Failed(first),
                _ => UnitEnd::Failed(e),
            },
        }
    }

    /// Asks about the confirmed statement, waits, and acts on the answer (the module docs).
    async fn decide(
        &mut self,
        spec: &RunSpec,
        unit: &RunUnit,
        wrap: Wrap,
        ended: UnitEnd,
        w: &mut Watch,
    ) -> UnitEnd {
        let run = spec.run.0;
        let ordinal = unit.id.ordinal;
        let savepoint = wrap.form == Form::Savepoint;
        // Fix round 1 (m3): a stop asked for while the statement ran is never asked about.
        if w.stop.is_requested() {
            let undone = self.undo(wrap).await;
            tracing::info!(
                run,
                unit = ordinal,
                savepoint,
                "the confirmed statement was rolled back: the run was cancelled before the \
                 question"
            );
            self.publish_tx();
            return match undone {
                Undone::Done => UnitEnd::Cancelled,
                // ## m3b-t26-prod-unlock (fix round 1)
                Undone::ChangesStayed => {
                    UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmNotUndone)))
                }
                Undone::Lost(e) | Undone::Refused(e) | Undone::TxFailed(e) => UnitEnd::Failed(e),
            };
        }
        let affected = match &ended {
            UnitEnd::Succeeded {
                rows: Some(RowsOutcome::Affected(n)),
            } => Some(*n),
            _ => None,
        };
        let gate = self.gate.clone();
        // The snapshot shows the transaction the question is about while the run waits.
        self.publish_tx();
        let at = tokio::time::Instant::now() + spec.confirm_deadline;
        // Ask before emitting, nothing awaited in between (T11 I1).
        let question = gate.ask(Ask::Commit, self.request_seq, unit.id);
        spec.observer.0.event(&RunEvent::NeedsCommit {
            unit: unit.id,
            question,
            affected,
            deadline: at.into_std(),
            savepoint,
        });
        let (answer, by_deadline) = tokio::select! {
            biased;
            () = w.stop.requested() => (CommitAnswer::RollBack, false),
            answer = gate.answered() => match answer {
                Answer::Commit(answer) => (answer, false),
                Answer::Choice(_) => (CommitAnswer::RollBack, false),
            },
            () = tokio::time::sleep_until(at) => (CommitAnswer::RollBack, true),
        };
        gate.withdraw();
        // A cancel beats an answer that raced it.
        let cancelled = w.stop.is_requested();
        let (answer, by_deadline) = if cancelled {
            (CommitAnswer::RollBack, false)
        } else {
            (answer, by_deadline)
        };
        let closing = gate.closing();
        let done = match answer {
            CommitAnswer::Commit => self.keep(wrap).await,
            CommitAnswer::RollBack => self.undo(wrap).await,
        };
        let outcome = match done {
            // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): the rollback left the
            // changes (a table without transactions): said so, never "undone".
            Undone::ChangesStayed => {
                spec.observer
                    .0
                    .event(&RunEvent::ConfirmNotUndone { unit: unit.id });
                spec.observer.0.event(&RunEvent::CommitDecided {
                    unit: unit.id,
                    answer,
                    by_deadline,
                });
                UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmNotUndone)))
            }
            Undone::Done => {
                match answer {
                    CommitAnswer::Commit => tracing::info!(
                        run,
                        unit = ordinal,
                        savepoint,
                        "the confirmed statement was kept"
                    ),
                    CommitAnswer::RollBack => tracing::info!(
                        run,
                        unit = ordinal,
                        savepoint,
                        by_deadline,
                        cancelled,
                        closing,
                        "the confirmed statement was rolled back"
                    ),
                }
                spec.observer.0.event(&RunEvent::CommitDecided {
                    unit: unit.id,
                    answer,
                    by_deadline,
                });
                // Fix round 1 (C2): an undone statement did not take effect. The deadline's
                // stops the run whatever its policy (the user is away); the user's Undo (or a
                // close) leaves the rest to the error policy.
                match answer {
                    _ if cancelled => UnitEnd::Cancelled,
                    CommitAnswer::Commit => ended,
                    CommitAnswer::RollBack if by_deadline => {
                        UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmTimedOut)))
                    }
                    CommitAnswer::RollBack => {
                        UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmUndone)))
                    }
                }
            }
            // Fix round 2 (R1-m2): not reported as kept; the unit fails with the server's text
            // and the snapshot shows the failed transaction.
            Undone::TxFailed(e) => UnitEnd::Failed(e),
            Undone::Lost(mut e) => {
                // Fix round 1 (m2): worded per answer.
                match answer {
                    CommitAnswer::Commit => tracing::info!(
                        run,
                        unit = ordinal,
                        "the connection was lost at the commit; its outcome is unknown, and a \
                         server rolls back a transaction whose commit did not reach it"
                    ),
                    CommitAnswer::RollBack => {
                        tracing::info!(
                            run,
                            unit = ordinal,
                            "the connection was lost while the confirmed statement waited; the \
                             server rolled back these changes"
                        );
                        // A rollback that met a lost connection rolled back all the same.
                        e.kind = ExecErrorKind::ConnectionLost {
                            outcome_unknown: false,
                        };
                    }
                }
                UnitEnd::Failed(e)
            }
            Undone::Refused(e) if self.broken.is_some() => {
                tracing::warn!(
                    run,
                    unit = ordinal,
                    savepoint,
                    "the server refused to end the confirmed statement; the session is closed \
                     and the server rolls back"
                );
                UnitEnd::Failed(e)
            }
            // A `COMMIT` the server refused (a deferred constraint): it rolled back.
            Undone::Refused(e) => {
                tracing::info!(
                    run,
                    unit = ordinal,
                    "the server refused the commit and rolled back the confirmed statement"
                );
                UnitEnd::Failed(e)
            }
        };
        self.publish_tx();
        outcome
    }

    /// Commit (own form) or Keep (`RELEASE`, the savepoint form).
    async fn keep(&mut self, wrap: Wrap) -> Undone {
        match wrap.form {
            Form::Own => {
                let committed = self.control("COMMIT").await;
                self.after_own(committed)
            }
            Form::Savepoint => {
                let released = self.control(wrap.kind.release()).await;
                match released {
                    Err(e) if self.broken.is_none() && self.tx.status == TxStatus::Failed => {
                        tracing::warn!(
                            kind = ?e.kind,
                            "the server refused to release Hacivat's savepoint after Keep and \
                             failed the transaction; the kept statement is not kept"
                        );
                        Undone::TxFailed(e)
                    }
                    other => self.after_release(other),
                }
            }
        }
    }

    /// Roll back (own form) or Undo (`ROLLBACK TO` then `RELEASE`, the savepoint form).
    async fn undo(&mut self, wrap: Wrap) -> Undone {
        match wrap.form {
            Form::Own => {
                let rolled = self.control("ROLLBACK").await;
                let kept = rolled.is_ok() && self.rollback_left_changes().await;
                match self.after_own(rolled) {
                    Undone::Done if kept => Undone::ChangesStayed,
                    other => other,
                }
            }
            Form::Savepoint => match self.control(wrap.kind.rollback_to()).await {
                Ok(()) => {
                    // ## m3b-t26-prod-unlock (fix round 1): read before `RELEASE` replaces the
                    // warnings.
                    let kept = self.rollback_left_changes().await;
                    let released = self.control(wrap.kind.release()).await;
                    match self.after_release(released) {
                        Undone::Done if kept => Undone::ChangesStayed,
                        other => other,
                    }
                }
                // The statement may still be in the user's transaction: the session is not
                // used again (the server rolls back; the loss is reported like any other).
                Err(e) if self.broken.is_some() => Undone::Lost(e),
                Err(e) => {
                    self.broken.get_or_insert(ErrClass::Internal);
                    Undone::Refused(e)
                }
            },
        }
    }

    /// Hacivat's own transaction after its `COMMIT` or `ROLLBACK`: it is never left open (a
    /// server that still reports it breaks the session, as for a cursor's).
    fn after_own(&mut self, ended: Result<(), ExecError>) -> Undone {
        if self.broken.is_some() {
            self.forget_own();
            return match ended {
                Err(e) => Undone::Lost(e),
                Ok(()) => Undone::Lost(ExecError::of(ExecErrorKind::ConnectionLost {
                    outcome_unknown: false,
                })),
            };
        }
        self.end_own();
        match ended {
            Ok(()) if self.tx.status == TxStatus::Idle => Undone::Done,
            Ok(()) => {
                self.broken.get_or_insert(ErrClass::Internal);
                Undone::Refused(ExecError::of(ExecErrorKind::Internal))
            }
            // A `COMMIT` the server refused (a deferred constraint) ended the transaction
            // with a rollback; anything still open is never used again.
            Err(e) => {
                if self.tx.status != TxStatus::Idle {
                    self.broken.get_or_insert(ErrClass::Internal);
                }
                Undone::Refused(e)
            }
        }
    }

    /// The savepoint's `RELEASE` after Keep, or after a `ROLLBACK TO` that undid the
    /// statement. Fix round 1 (m1): a release the server refused leaves at most an extra
    /// savepoint (or, on PostgreSQL, a failed transaction the snapshot shows as it is): it is
    /// logged and the session stays; only a lost connection is a loss.
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
    /// Whether the rollback just sent left changes behind: on MySQL/MariaDB the server warns
    /// (`NOT_COMPLETE_ROLLBACK`) when a table does not support transactions. Never on other
    /// engines; a warnings read that fails answers `false` (the rollback itself succeeded).
    async fn rollback_left_changes(&mut self) -> bool {
        if family(self.kind) != Family::MySql {
            return false;
        }
        let Ok(conn) = self.conn() else {
            return false;
        };
        let warnings = conn.warnings().await;
        self.observe(false);
        let kept = warnings.is_ok_and(|w| {
            w.iter().any(|n| {
                n.code
                    .as_deref()
                    .is_some_and(|c| NOT_COMPLETE_ROLLBACK.contains(&c))
            })
        });
        if kept {
            tracing::warn!(
                notice = "exec-confirm-not-undone",
                "the rollback ran, but a table does not support transactions: its changes stay"
            );
        }
        kept
    }

    fn after_release(&mut self, released: Result<(), ExecError>) -> Undone {
        match released {
            Ok(()) => Undone::Done,
            Err(e) if self.broken.is_some() => Undone::Lost(e),
            Err(e) => {
                tracing::warn!(
                    kind = ?e.kind,
                    status = ?self.tx.status,
                    "the server refused to release Hacivat's savepoint; the session keeps the \
                     transaction as the server reports it"
                );
                Undone::Done
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_two_wrapping_verdicts_wrap() {
        assert_eq!(
            WrapKind::of(UnitVerdict::TxThenConfirm),
            Some(WrapKind::Confirm)
        );
        assert_eq!(
            WrapKind::of(UnitVerdict::ExplainAnalyzeRollback),
            Some(WrapKind::Explain)
        );
        for v in [
            UnitVerdict::Read,
            UnitVerdict::Write,
            UnitVerdict::Unknown,
            UnitVerdict::ImplicitCommit,
        ] {
            assert_eq!(WrapKind::of(v), None, "{v:?}");
        }
    }

    #[test]
    fn savepoint_statements_name_their_own_savepoint() {
        assert_eq!(WrapKind::Confirm.savepoint(), "SAVEPOINT hv_confirm");
        assert_eq!(
            WrapKind::Confirm.rollback_to(),
            "ROLLBACK TO SAVEPOINT hv_confirm"
        );
        assert_eq!(WrapKind::Confirm.release(), "RELEASE SAVEPOINT hv_confirm");
        assert_eq!(WrapKind::Explain.savepoint(), "SAVEPOINT hv_explain");
        assert_eq!(
            WrapKind::Explain.rollback_to(),
            "ROLLBACK TO SAVEPOINT hv_explain"
        );
        assert_eq!(WrapKind::Explain.release(), "RELEASE SAVEPOINT hv_explain");
    }
}
