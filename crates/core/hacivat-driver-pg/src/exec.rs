//! Statement execution on a PostgreSQL connection (HV-EXEC-05, Task 10): `execute`, `batch`,
//! the transaction status from `ReadyForQuery`, the reset sequence (`DISCARD ALL` without
//! `DEALLOCATE ALL`, Q10) and the backend PID (Q11).
//!
//! Task 1 stubs refuse every statement (`RawErrorKind::Unsupported`), so nothing runs before
//! Task 10 and the read-only guard (Task 18) are in place.

use hacivat_driver::{RawError, RawErrorKind};

/// The error of an operation Task 10 has not filled in.
pub(crate) fn not_yet() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}
