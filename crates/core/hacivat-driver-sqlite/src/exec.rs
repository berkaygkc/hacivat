//! Statement execution on a SQLite connection (HV-EXEC-05, Task 11): `execute` and `batch` on
//! the connection's own thread, the transaction status from `sqlite3_get_autocommit`, and the
//! interrupt.
//!
//! Task 1 stubs refuse every statement (`RawErrorKind::Unsupported`), so nothing runs before
//! Task 11 and the read-only hardening are in place.

use hacivat_driver::{RawError, RawErrorKind};

/// The error of an operation Task 11 has not filled in.
pub(crate) fn not_yet() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}
