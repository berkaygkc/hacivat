// ## m4-t01c-core
//! Metadata connections (HV-SESS-01's metadata connection; HV-INTRO-01 step 4; R-M4-7, R-M4-29,
//! R-M4-38): the read-only connections the introspectors read the catalog through.
//!
//! - **Trusted text only.** A [`MetaConn`] runs only a [`MetaQuery`], built by the pack renderer
//!   (`hacivat-introspect`'s `RenderedQuery::as_meta_query`; check-deps `MetaQueryOnlyFromPacks`
//!   allows `MetaQuery::new` only there and in this file). Its fields are private and
//!   [`MetaQuery::new`] is its only constructor: no `Default`, `From`, `Deserialize` or builder,
//!   so a struct literal cannot bypass the rule. It never passes through `hacivat-exec`, the
//!   classifier or the history store, and never appears in the user's output log.
//! - **Server-enforced read-only** (Task 5): PostgreSQL `BEGIN ISOLATION LEVEL REPEATABLE READ
//!   READ ONLY` plus `SET LOCAL search_path = pg_catalog, pg_temp`; MySQL and MariaDB a read-only
//!   session (restored after every reconnect, asserted before every snapshot) plus
//!   `START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY`; SQLite a separate `mode=ro`
//!   connection (R-M4-38) that does not count against the profile's single session (U-6(e)).
//! - **Joined connections** (R-M4-29) run `joinable` sections inside a transaction Hacivat owns
//!   on a `Job(DdlApply | DriftCheck)` session; they never begin, commit or roll back, refuse
//!   text whose first keyword is `BEGIN`, `START`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`, `RELEASE`
//!   or `SET`, and are refused on `Console` and `Editor` sessions.
//!
//! **Fail-closed stub (M4 Task 1c; bodies Task 5).** Acquisition ([`SessionManager::meta_conn`])
//! and [`MetaConn::join`] answer [`MetaError::Unavailable`] and open nothing, so every namespace
//! stays `LoadState::Failed` and the explorer shows "Not loaded" (F31).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{ServerInfo, ServerMessage};
use hacivat_proto::{DbmsKind, ProfileId};
use hacivat_values::BindValue;
use tokio_util::sync::CancellationToken;

use crate::session::{SessionHandle, SessionManager};

/// What metadata connection to open.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MetaConnSpec {
    pub profile: ProfileId,
    /// The database to read (PostgreSQL needs one connection per database); `None` is the
    /// profile's.
    pub database: Option<String>,
    /// The server-side limit of each catalog statement.
    pub statement_timeout: Duration,
}

/// Why a metadata read failed. Codes only: the edge maps them to Fluent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MetaError {
    /// No metadata connection can be had (the stub's only answer; the profile is not
    /// connected; the engine has none).
    Unavailable,
    /// The profile's connection budget has no room now.
    Busy,
    ConnectionLost,
    Timeout,
    /// The session cannot be joined (not a Hacivat-owned `Job(DdlApply | DriftCheck)`
    /// transaction), or the query is not `joinable`.
    NotJoinable,
    /// A joined connection was handed transaction-control or `SET` text.
    RefusedText,
    /// The server did not confirm the read-only session or transaction.
    ReadOnlyNotEnforced,
    /// The server refused the statement; its own message, verbatim.
    Server {
        sqlstate: Option<Arc<str>>,
        message: ServerMessage,
    },
    Cancelled,
}

/// One catalog statement: trusted pack text with its binds (names and ids always travel as
/// binds, R-M4-3).
///
/// Its fields are private and [`MetaQuery::new`] is its only constructor, so a struct literal
/// outside this module does not compile. (On a stable toolchain rustdoc does not check the
/// `E0451` code of a `compile_fail` test, so the test would pass on any compile error; the T1c
/// review compiled the literal outside the crate and saw exactly E0451, "fields `sql`, `binds`
/// and `joinable` of struct `MetaQuery` are private". The positive test below proves the rest
/// of the snippet compiles.)
///
/// ```compile_fail,E0451
/// use hacivat_session::meta::MetaQuery;
/// let q: MetaQuery<'static> = MetaQuery { sql: "SELECT 1", binds: &[], joinable: false };
/// ```
///
/// while the constructor does (the same values, through `new`):
///
/// ```
/// use hacivat_session::meta::MetaQuery;
/// let q: MetaQuery<'static> = MetaQuery::new("SELECT 1", &[], false);
/// assert!(!q.joinable());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MetaQuery<'a> {
    sql: &'a str,
    binds: &'a [BindValue],
    joinable: bool,
}

impl<'a> MetaQuery<'a> {
    /// Called only by the pack renderer (`RenderedQuery::as_meta_query`); check-deps
    /// `MetaQueryOnlyFromPacks` refuses the call anywhere else.
    #[doc(hidden)]
    pub fn new(sql: &'a str, binds: &'a [BindValue], joinable: bool) -> Self {
        MetaQuery {
            sql,
            binds,
            joinable,
        }
    }

    pub fn sql(&self) -> &'a str {
        self.sql
    }

    pub fn binds(&self) -> &'a [BindValue] {
        self.binds
    }

    /// The pack section is marked joinable (R-M4-29): it may run on a joined connection.
    pub fn joinable(&self) -> bool {
        self.joinable
    }
}

/// What a catalog statement answered: column names and text cells (`None` for SQL NULL).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Rows {
    pub columns: Vec<Arc<str>>,
    pub rows: Vec<Vec<Option<Arc<str>>>>,
}

/// A read-only metadata connection (or a joined one, R-M4-29). Built only by
/// [`SessionManager::meta_conn`] and [`MetaConn::join`]; the stub builds none.
pub struct MetaConn {
    dbms: DbmsKind,
    server: ServerInfo,
    joined: bool,
}

impl fmt::Debug for MetaConn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetaConn")
            .field("dbms", &self.dbms)
            .field("joined", &self.joined)
            .finish_non_exhaustive()
    }
}

impl MetaConn {
    pub fn dbms(&self) -> DbmsKind {
        self.dbms
    }

    /// What the server reported at connect.
    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// Opens one consistent read-only snapshot (see the module docs per engine).
    ///
    /// Stub: [`MetaError::Unavailable`].
    pub fn begin_snapshot(&mut self) -> BoxFuture<'_, Result<SnapshotTxn<'_>, MetaError>> {
        Box::pin(async { Err(MetaError::Unavailable) })
    }

    /// A connection that reads inside `session`'s open transaction, which Hacivat owns on a
    /// `Job(DdlApply | DriftCheck)` session (R-M4-29); refused on any other session.
    ///
    /// Stub: [`MetaError::Unavailable`].
    pub fn join(session: &SessionHandle) -> Result<MetaConn, MetaError> {
        let _ = session;
        Err(MetaError::Unavailable)
    }

    // ## m4-t01c-core (Part B)
    /// A connection with no server behind it, for tests that must hand an introspector a
    /// `MetaConn` (the seams test, Task 15's engine tests with `FakeIntrospector`). It can never
    /// read: every snapshot answers [`MetaError::Unavailable`], and Task 5 must keep it inert
    /// (no driver connection, no text accepted). Not for product code.
    #[doc(hidden)]
    pub fn offline(dbms: DbmsKind) -> MetaConn {
        MetaConn {
            dbms,
            server: ServerInfo {
                kind: dbms,
                product: String::new(),
                version: None,
                raw_version: String::new(),
                is_replica: None,
            },
            joined: false,
        }
    }

    /// Whether the connection's current transaction has an XID assigned
    /// (`pg_current_xact_id_if_assigned() IS NOT NULL`): the `#pg-canary` check of Task 5's
    /// tests.
    ///
    /// Stub: [`MetaError::Unavailable`].
    #[cfg(any(test, feature = "test-support"))]
    pub fn debug_xid_assigned(&mut self) -> BoxFuture<'_, Result<bool, MetaError>> {
        Box::pin(async { Err(MetaError::Unavailable) })
    }
}

/// One snapshot of a [`MetaConn`]: every [`rows`](Self::rows) call reads the same catalog
/// state; [`commit`](Self::commit) ends it (a read-only transaction's commit writes nothing).
pub struct SnapshotTxn<'c> {
    conn: &'c mut MetaConn,
}

impl fmt::Debug for SnapshotTxn<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotTxn")
            .field("conn", &self.conn)
            .finish()
    }
}

impl<'c> SnapshotTxn<'c> {
    /// Runs one catalog statement.
    ///
    /// Stub: [`MetaError::Unavailable`].
    pub fn rows<'q>(&'q mut self, q: MetaQuery<'q>) -> BoxFuture<'q, Result<Rows, MetaError>> {
        let _ = (q, &self.conn);
        Box::pin(async { Err(MetaError::Unavailable) })
    }

    /// Ends the snapshot.
    ///
    /// Stub: [`MetaError::Unavailable`].
    pub fn commit(self) -> BoxFuture<'c, Result<(), MetaError>> {
        Box::pin(async { Err(MetaError::Unavailable) })
    }
}

impl SessionManager {
    /// A metadata connection for `spec` (a `SessionKind::Introspection` lease through the
    /// profile's budget at `Priority::Introspection`, Task 5).
    ///
    /// Stub: [`MetaError::Unavailable`]; opens no connection.
    pub fn meta_conn<'a>(
        &'a self,
        spec: MetaConnSpec,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        let _ = (spec, cancel);
        Box::pin(async { Err(MetaError::Unavailable) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_query_keeps_what_it_was_given() {
        let binds = [BindValue::Null];
        let q = MetaQuery::new("SELECT 1 WHERE $1 = $1", &binds, true);
        assert_eq!(q.sql(), "SELECT 1 WHERE $1 = $1");
        assert_eq!(q.binds().len(), 1);
        assert!(q.joinable());
    }
}
