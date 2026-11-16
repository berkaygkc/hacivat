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
//!
//! ## m3a-t18-session-run: the stop-fetching rung (R-M3-16, P16)
//!
//! For a typed request (`Run`, `FetchMore`, `FetchAll`, `CloseCursor`) the first rung sends
//! nothing: `cancel(seq)` drops the queue and sets the request's stop signal
//! (`cursor::StopSignal`). The request reads it between chunks, stops asking for rows and
//! closes its query the engine's way; the rows it has stay, and the result is `Cancelled`.
//! The server cancel (PostgreSQL's protocol cancel, MySQL's `KILL QUERY`, SQLite's interrupt)
//! goes out only for a call that is `FetchPolicy::overdue` (1 s) late, and beside the close of
//! a stream that cannot be closed early (PostgreSQL's `Execute(0)`, in autocommit only); the
//! request sends it itself and awaits it before its next statement (`cursor::Watch`), at most
//! one per statement (a MySQL kill that raced the statement's end is absorbed by one `DO 0`,
//! never left for the user's next statement). Inside a user transaction
//! that server cancel leaves the transaction `Failed`, as in psql. The caller's 2 s answer,
//! the slow cancel at [`SLOW_CANCEL`] and both escalations are as above. A cancel names the
//! running request, or a fetch merged into it; a run that ended (its cursor still open) and an
//! earlier fetch are `AlreadyFinished`.
//!
//! ## m3b-t12-session-carries: the warm side connection (C10; ADR 0004 `m3b-t12`)
//! A MySQL session's side connections go through a [`WarmSide`], which keeps one open while
//! it may be needed: from the moment a statement has run for [`SIDE_PREWARM`], and while a
//! cursor is suspended. A kill takes it (it was checked with a ping first; a dead one is
//! replaced by a fresh connect), so a cancel never waits for a connect it could have had
//! ready. Once nothing needs it, it closes [`SIDE_KEEP`] after its last use; it closes at once
//! when the session closes, breaks or loses its connection.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_proto::{DbmsKind, ErrClass};
use tokio::task::JoinHandle;

use crate::pipeline::{ConnectError, LiveConnection};
use crate::session::schema::{Family, family};
use crate::session::{ExecError, SideConnector};

/// How long a cancel's caller waits for the request to end before `StillRunning`.
pub const CANCEL_WAIT: Duration = Duration::from_secs(2);

/// When a cancel that has not ended its request becomes a slow cancel (the next rung).
pub const SLOW_CANCEL: Duration = Duration::from_secs(10);

/// How long a MySQL statement runs before its side connection is pre-opened (Q37).
pub const SIDE_PREWARM: Duration = Duration::from_millis(500);

// ## m3b-t12-session-carries (C10)
/// How long a warm MySQL side connection stays open after its last use.
pub const SIDE_KEEP: Duration = Duration::from_secs(30);

/// How long a warm side connection may take to answer the ping before a kill uses it.
const WARM_CHECK: Duration = Duration::from_secs(2);

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
    // ## m3a-t31-docker-conformance (named exception, doc only)
    /// The request ended without the server reporting a cancel: it was not running (Q18), or
    /// the server ended normally a statement Hacivat's kill interrupted (MySQL `SLEEP()`).
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

// ## m3b-t12-session-carries (C10)

type Opening = JoinHandle<Result<LiveConnection, ConnectError>>;

/// A MySQL session's side connections, one of them kept warm (the module docs). It wraps the
/// session's own [`SideConnector`]; every side connection of the session goes through it.
pub(crate) struct WarmSide {
    inner: Arc<dyn SideConnector>,
    slot: Arc<Mutex<WarmSlot>>,
}

#[derive(Default)]
struct WarmSlot {
    /// The warm connection, or its connect in flight.
    conn: Option<Opening>,
    /// Bumped by every use: an expiry armed before it does nothing.
    generation: u64,
}

fn slot_lock(slot: &Mutex<WarmSlot>) -> MutexGuard<'_, WarmSlot> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Closes the connection `task` opens (or opened), on the current runtime. Without one (a drop
/// outside any runtime) the task is left to end with its runtime.
fn close_opening(task: Opening) {
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            if let Ok(Ok(live)) = task.await {
                live.close().await;
            }
        });
    }
}

impl WarmSide {
    pub(crate) fn new(inner: Arc<dyn SideConnector>) -> WarmSide {
        WarmSide {
            inner,
            slot: Arc::default(),
        }
    }
}

impl SideConnector for WarmSide {
    /// The warm connection when it still answers a ping, else a fresh one.
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let warm = {
            let mut slot = slot_lock(&self.slot);
            slot.generation = slot.generation.wrapping_add(1);
            slot.conn.take()
        };
        let inner = self.inner.clone();
        Box::pin(async move {
            if let Some(task) = warm
                && let Ok(Ok(mut live)) = task.await
            {
                let checked = tokio::time::timeout(WARM_CHECK, live.raw().ping()).await;
                if matches!(checked, Ok(Ok(_))) {
                    return Ok(live);
                }
                live.close().await;
            }
            inner.open_side().await
        })
    }

    fn warm(&self) {
        let mut slot = slot_lock(&self.slot);
        // A use: no expiry armed before it closes it.
        slot.generation = slot.generation.wrapping_add(1);
        if slot.conn.is_none() {
            slot.conn = Some(tokio::spawn(self.inner.open_side()));
        }
    }

    fn rest(&self) {
        let generation = {
            let mut slot = slot_lock(&self.slot);
            if slot.conn.is_none() {
                return;
            }
            slot.generation = slot.generation.wrapping_add(1);
            slot.generation
        };
        let slot = Arc::downgrade(&self.slot);
        tokio::spawn(async move {
            tokio::time::sleep(SIDE_KEEP).await;
            let Some(slot) = slot.upgrade() else {
                return;
            };
            let expired = {
                let mut slot = slot_lock(&slot);
                if slot.generation != generation {
                    return;
                }
                slot.conn.take()
            };
            if let Some(task) = expired {
                close_opening(task);
            }
        });
    }

    fn drop_warm(&self) {
        let task = {
            let mut slot = slot_lock(&self.slot);
            slot.generation = slot.generation.wrapping_add(1);
            slot.conn.take()
        };
        if let Some(task) = task {
            close_opening(task);
        }
    }
}

impl Drop for WarmSide {
    fn drop(&mut self) {
        if let Some(task) = slot_lock(&self.slot).conn.take() {
            close_opening(task);
        }
    }
}
