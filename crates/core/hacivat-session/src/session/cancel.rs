//! The cancel ladder (HV-SESS-05, B23; Task 7): cancel, a slow cancel, then terminate or
//! deactivate.
//!
//! - `cancel(seq)` acts only on the running request (Q18): anything else is `AlreadyFinished`.
//!   It drops the queue (`Cancelled`) and cancels through the connection's `CancelHandle`, or on
//!   MySQL with `KILL QUERY <id>` on a side connection, pre-opened once a statement has run for
//!   [`SIDE_PREWARM`] (Q37). Its outcome comes within [`CANCEL_WAIT`], else `StillRunning`; at
//!   [`SLOW_CANCEL`] the session reports `SlowCancel`.
//! - `escalate(seq, Terminate)` kills the server session from a side connection;
//!   `escalate(seq, Deactivate)` drops the connection and breaks the session.
//! - A cancel still in flight when its request ends is awaited (bounded) before the next
//!   request starts, so it can never reach that request.

use std::time::{Duration, Instant};

use hacivat_proto::{DbmsKind, ErrClass};

use crate::session::ExecError;
use crate::session::schema::{Family, family};

/// How long a cancel's caller waits for the request to end before `StillRunning`.
pub const CANCEL_WAIT: Duration = Duration::from_secs(2);

/// When a cancel that has not ended its request becomes a slow cancel (the next rung).
pub const SLOW_CANCEL: Duration = Duration::from_secs(10);

/// How long a MySQL statement runs before its side connection is pre-opened (Q37).
pub const SIDE_PREWARM: Duration = Duration::from_millis(500);

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

/// The side-connection statement that cancels server connection `id`'s statement: MySQL's
/// `KILL QUERY` (the others cancel through their `CancelHandle`).
pub(crate) fn kill_query_sql(kind: DbmsKind, id: u64) -> Option<String> {
    (family(kind) == Family::MySql).then(|| format!("KILL QUERY {id}"))
}

/// Whether `e` is the server's own answer to a cancel: PostgreSQL `57014` (`query_canceled`),
/// MySQL `1317` (`ER_QUERY_INTERRUPTED`). Read only once a cancel was asked for, since
/// PostgreSQL's statement timeout answers `57014` too.
pub(crate) fn is_cancel_code(kind: DbmsKind, e: &ExecError) -> bool {
    let Some(server) = &e.server else {
        return false;
    };
    match family(kind) {
        Family::Postgres => server.sqlstate.as_deref() == Some("57014"),
        Family::MySql => server.vendor_code == Some(1317),
        Family::Sqlite | Family::Other => false,
    }
}

/// The side-connection statement that ends server connection `id`.
pub(crate) fn terminate_sql(kind: DbmsKind, id: u64) -> Option<String> {
    match family(kind) {
        Family::Postgres => Some(format!("SELECT pg_terminate_backend({id})")),
        Family::MySql => Some(format!("KILL CONNECTION {id}")),
        Family::Sqlite | Family::Other => None,
    }
}
