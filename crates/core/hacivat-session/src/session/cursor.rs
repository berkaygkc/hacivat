//! Console cursors (HV-RES-05; Task 18): fetching more, fetching all, releasing a cursor
//! (another request first, the idle bound, the end of its transaction, a lost connection), and
//! the per-engine idle bounds (R-M3-20).
//!
//! M3a Task 1b stub: fails closed. Every cursor request is answered `ExecErrorKind::Internal`
//! (there is no cursor before Task 18), and the idle bound is zero (a cursor would be released
//! at once).

use std::time::Duration;

use hacivat_driver::TypedCaps;
use hacivat_proto::DbmsKind;
use hacivat_results::{CloseReason, FetchPolicy, ResultId};

use crate::session::actor::Core;
use crate::session::{ExecError, ExecErrorKind, Reply};

/// How long a suspended cursor of `kind` may stay idle (R-M3-20): PostgreSQL read-write
/// `policy.idle_timeout`, read-only 20 s; MySQL/MariaDB `caps.idle_bound`; SQLite
/// `caps.idle_bound` (30 s in rollback-journal mode), else `policy.idle_timeout`; production at
/// most 120 s; `policy.idle_cap` caps every bound (Task 18).
pub fn idle_bound(
    kind: DbmsKind,
    read_only: bool,
    production: bool,
    caps: &TypedCaps,
    policy: &FetchPolicy,
) -> Duration {
    let _ = (kind, read_only, production, caps, policy);
    Duration::ZERO
}

impl Core {
    /// `Request::FetchMore` (Task 18).
    pub(crate) async fn fetch_more(&mut self, result: ResultId, rows: u64) -> Reply {
        let _ = (result, rows);
        Err(ExecError::of(ExecErrorKind::Internal))
    }

    /// `Request::FetchAll` (Task 18).
    pub(crate) async fn fetch_all(&mut self, result: ResultId) -> Reply {
        let _ = result;
        Err(ExecError::of(ExecErrorKind::Internal))
    }

    /// `Request::CloseCursor` (Task 18).
    pub(crate) async fn close_cursor(&mut self, result: ResultId, reason: CloseReason) -> Reply {
        let _ = (result, reason);
        Err(ExecError::of(ExecErrorKind::Internal))
    }
}
