//! Statement execution on a MySQL or MariaDB connection (HV-EXEC-05, Task 13): `execute`,
//! `batch`, the transaction status from the server status flags, `COM_RESET_CONNECTION` and
//! the thread ID.
//!
//! Task 1 stubs refuse every statement (`RawErrorKind::Unsupported`), so nothing runs before
//! Task 13 and the read-only guard (Task 18) are in place.

use hacivat_driver::{RawError, RawErrorKind};

/// The error of an operation Task 13 has not filled in.
pub(crate) fn not_yet() -> RawError {
    RawError {
        kind: RawErrorKind::Unsupported,
        server: None,
    }
}
