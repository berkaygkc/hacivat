//! The cancel ladder (HV-SESS-05, B23; Task 7): cancel, a slow cancel, then terminate or
//! deactivate.

use std::time::Instant;

use hacivat_proto::ErrClass;

/// Where a session's cancel is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum CancelState {
    #[default]
    Idle,
    Cancelling {
        since: Instant,
    },
    /// The cancel is taking long; the UI offers the next rung.
    SlowCancel {
        since: Instant,
    },
    Terminating,
    Deactivating,
}

/// What a cancel did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelOutcome {
    Cancelled,
    /// The request was not running (Q18).
    AlreadyFinished,
    StillRunning,
    Disconnected,
    Failed(ErrClass),
}

/// The next rung after a slow cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Escalation {
    /// Kill the server session (PostgreSQL `pg_terminate_backend`, MySQL `KILL`).
    Terminate,
    /// Drop the connection on the client side.
    Deactivate,
}
