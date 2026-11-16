// ## m3b-t01c-core-seams
//! Confirm-in-transaction (HV-EXEC-04 session side, D-014, R-M3-55, P-M3b-1, P-M3b-5;
//! Task 19): a `TxThenConfirm` unit runs inside a transaction, then the run waits for
//! `SessionHandle::commit_decision` through the `RunGate` (`RunEvent::NeedsCommit`), with the
//! D-014 deadline (`RunSpec::confirm_deadline`). The session picks the form from the live
//! transaction state: `SAVEPOINT hv_confirm` (Keep/Undo) whenever a transaction is open,
//! Hacivat's own `BEGIN` (`TxOwner::Confirm`, Commit/Roll back) only when it is truly idle in
//! Auto. The D-062 wrap of `EXPLAIN ANALYZE` lives here too.
//!
//! STUB until Task 19 (fail closed): a unit whose verdict needs this machinery is refused
//! with `ExecErrorKind::Internal` before anything is sent.

use crate::session::verdict::UnitVerdict;
use crate::session::{ExecError, ExecErrorKind};

/// Why a unit with `verdict` cannot run yet, if it cannot: `TxThenConfirm` and
/// `ExplainAnalyzeRollback` need Task 19. Checked before any statement of the unit is sent
/// (the Manual `BEGIN` included).
pub(crate) fn refused_before_task_19(verdict: UnitVerdict) -> Option<ExecError> {
    match verdict {
        UnitVerdict::TxThenConfirm | UnitVerdict::ExplainAnalyzeRollback => {
            Some(ExecError::of(ExecErrorKind::Internal))
        }
        UnitVerdict::Read
        | UnitVerdict::Write
        | UnitVerdict::Unknown
        | UnitVerdict::ImplicitCommit => None,
    }
}
