//! Statement execution on a SQLite connection (HV-EXEC-05, Task 11): one statement per call,
//! run on the connection's own thread, rows as text, the transaction status from
//! `sqlite3_get_autocommit`, and the interrupt.
//!
//! - **One statement.** rusqlite's `prepare` refuses a tail that holds another statement
//!   (`MultipleStatement`); a tail of whitespace, comments or `;` is fine.
//! - **Read-only.** On a read-only profile's connection the authorizer has already judged
//!   every action while the statement was prepared; then `sqlite3_stmt_readonly` must also be
//!   true, else the statement never runs (`readonly-required`). The one exemption is the plain
//!   `PRAGMA journal_mode` query (Q16 lists it as readable; SQLite compiles it to the opcode
//!   that also sets the mode, so it is not "read-only" to `sqlite3_stmt_readonly`).
//! - **Command tag.** The first keyword in ASCII upper case (Q35), for the guard's
//!   `WriteSucceeded` post-check.
//! - **Cancel.** [`SqliteCancel`] latches a cancel per request (see there): `sqlite3_interrupt`
//!   alone is lost when it lands before the statement starts, because SQLite clears the
//!   interrupt flag whenever a statement starts with none active.

use std::fmt::Write as _;
use std::sync::{Mutex, PoisonError};

use futures::future::BoxFuture;
use hacivat_driver::{CancelHandle, ExecSummary, RawError, RawErrorKind, ServerMessage, TxStatus};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, InterruptHandle, ffi};

use crate::harden::AuthTrail;
use crate::raw_error;

/// How many virtual-machine steps pass between two checks of the cancel latch.
pub(crate) const PROGRESS_STEPS: i32 = 1000;

/// The request a cancel is for.
#[derive(Debug, Default)]
struct Latch {
    /// The last ticket handed out.
    last: u64,
    /// The ticket of the request submitted and not yet ended.
    running: Option<u64>,
    /// A cancel arrived for `running`.
    requested: bool,
}

/// Cancels the request running on a connection, latched per request (review I1).
///
/// - `execute` and `batch` take a ticket ([`begin`](Self::begin)) on the async side before the
///   job is sent, and the connection's thread ends it ([`end`](Self::end)) when the job is done.
/// - [`cancel`](CancelHandle::cancel) acts only while a ticket is open: it sets the latch and
///   calls `sqlite3_interrupt`, both under the latch's lock. A cancel while nothing runs does
///   nothing.
/// - The job reads the latch right after `prepare`, and the connection's progress handler
///   reads it every [`PROGRESS_STEPS`] steps and interrupts the statement. So a cancel that
///   lands between submit and the first step (whose interrupt SQLite clears when the
///   statement starts) still ends the statement.
/// - `end` clears the latch under the same lock, so a cancel can never reach the next
///   request. An interrupt sent just before `end`, after the last statement stopped, is
///   cleared by SQLite when the next statement starts (no statement is active then).
///
/// Safe from any thread; after the connection closes it does nothing.
pub(crate) struct SqliteCancel {
    interrupt: InterruptHandle,
    latch: Mutex<Latch>,
}

impl SqliteCancel {
    pub(crate) fn new(interrupt: InterruptHandle) -> Self {
        SqliteCancel {
            interrupt,
            latch: Mutex::new(Latch::default()),
        }
    }

    fn latch(&self) -> std::sync::MutexGuard<'_, Latch> {
        self.latch.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Opens the ticket of a request about to be sent; a cancel from now on is for it.
    pub(crate) fn begin(&self) -> u64 {
        let mut latch = self.latch();
        latch.last += 1;
        latch.running = Some(latch.last);
        latch.requested = false;
        latch.last
    }

    /// Ends `ticket`'s request: a later cancel does nothing. A ticket a newer request has
    /// replaced (its future was dropped) leaves the newer one alone.
    pub(crate) fn end(&self, ticket: u64) {
        let mut latch = self.latch();
        if latch.running == Some(ticket) {
            latch.running = None;
            latch.requested = false;
        }
    }

    /// Whether `ticket`'s request was cancelled.
    pub(crate) fn cancelled(&self, ticket: u64) -> bool {
        let latch = self.latch();
        latch.running == Some(ticket) && latch.requested
    }

    /// For the progress handler: the request on the connection was cancelled.
    pub(crate) fn pending(&self) -> bool {
        self.latch().requested
    }
}

impl CancelHandle for SqliteCancel {
    fn cancel(&self) -> BoxFuture<'static, Result<(), RawError>> {
        let mut latch = self.latch();
        if latch.running.is_some() {
            latch.requested = true;
            self.interrupt.interrupt();
        }
        drop(latch);
        Box::pin(async { Ok(()) })
    }
}

/// The error of a cancelled request: `SQLITE_INTERRUPT`, as SQLite reports an interrupted
/// statement (`Cancelled`).
pub(crate) fn interrupted() -> RawError {
    raw_error(rusqlite::Error::SqliteFailure(
        ffi::Error::new(ffi::SQLITE_INTERRUPT),
        None,
    ))
}

/// The transaction status: SQLite is in a transaction exactly when autocommit is off.
pub(crate) fn tx_status(conn: &Connection) -> TxStatus {
    if conn.is_autocommit() {
        TxStatus::Idle
    } else {
        TxStatus::InTx
    }
}

/// The statement passed the authorizer but `sqlite3_stmt_readonly` says it writes: refused
/// before it runs, like an authorizer denial (`SQLITE_AUTH`).
pub(crate) fn readonly_required() -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: None,
            vendor_code: Some(i64::from(ffi::SQLITE_AUTH)),
            severity: None,
            text: "readonly-required".to_string(),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }
}

/// Runs one statement, keeping at most `max_rows` rows and counting the rest. `trail` is the
/// read-only connection's authorizer trail (`None` on a read-write connection). `cancelled`
/// reads the request's cancel latch.
pub(crate) fn execute(
    conn: &Connection,
    sql: &str,
    max_rows: usize,
    trail: Option<&AuthTrail>,
    cancelled: impl Fn() -> bool,
) -> Result<ExecSummary, RawError> {
    if let Some(trail) = trail {
        trail.clear();
    }
    let mut stmt = conn.prepare(sql).map_err(raw_error)?;
    // Text of only whitespace, comments and `;` prepares to no statement: nothing runs.
    // (`sqlite3_expanded_sql` is NULL for no statement, and otherwise only when out of memory.)
    if stmt.expanded_sql().is_none() {
        return match first_keyword(sql) {
            None => Ok(ExecSummary {
                tx: tx_status(conn),
                ..ExecSummary::default()
            }),
            Some(_) => Err(raw_error(rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_NOMEM),
                None,
            ))),
        };
    }
    if let Some(trail) = trail
        && !stmt.readonly()
        && !trail.only_journal_mode_query()
    {
        return Err(readonly_required());
    }
    // A cancel that landed before the statement started (its interrupt is gone by now).
    if cancelled() {
        return Err(interrupted());
    }
    let columns: Vec<String> = stmt.column_names().map(str::to_string).collect();
    let width = columns.len();
    let before = conn.total_changes();
    let mut kept = Vec::new();
    let mut row_count = 0u64;
    {
        let mut rows = stmt.raw_query();
        while let Some(row) = rows.next().map_err(raw_error)? {
            row_count += 1;
            if kept.len() < max_rows {
                let values = (0..width)
                    .map(|i| row.get_ref(i).map(value_text))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(raw_error)?;
                kept.push(values);
            }
        }
    }
    // A statement that writes reports the rows it changed (`sqlite3_changes`, which leaves
    // out trigger rows); one that changed nothing (DDL) reports 0. A read reports none.
    let rows_affected = (!stmt.readonly()).then(|| {
        if conn.total_changes() > before {
            conn.changes()
        } else {
            0
        }
    });
    Ok(ExecSummary {
        command_tag: first_keyword(sql),
        rows_affected,
        columns,
        truncated: row_count > kept.len() as u64,
        rows: kept,
        row_count,
        notice_codes: Vec::new(),
        tx: tx_status(conn),
    })
}

/// A value as text: integers in decimal, reals with at least one fractional digit (`2.0`),
/// text as stored (invalid UTF-8 replaced), blobs as `\x` and lowercase hex digits (the
/// PostgreSQL `bytea` form), and `None` for NULL.
fn value_text(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(format!("{f:?}")),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => {
            let mut s = String::with_capacity(2 + 2 * b.len());
            s.push_str("\\x");
            for byte in b {
                let _ = write!(s, "{byte:02x}");
            }
            Some(s)
        }
    }
}

/// The statement's first keyword in ASCII upper case, after whitespace, comments and empty
/// statements (`;`), which SQLite's parser skips too (Q35); `None` when the text does not
/// start with a word.
pub(crate) fn first_keyword(sql: &str) -> Option<String> {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ';');
        if let Some(r) = rest.strip_prefix("--") {
            rest = r.find('\n').map_or("", |i| &r[i + 1..]);
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
        } else {
            break;
        }
    }
    let end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_keyword_skips_whitespace_and_comments() {
        let k = |s| first_keyword(s);
        assert_eq!(k("select 1").as_deref(), Some("SELECT"));
        assert_eq!(k("  \n\tInsert into t").as_deref(), Some("INSERT"));
        assert_eq!(k("-- a\n/* b */ -- c\nWITH x").as_deref(), Some("WITH"));
        assert_eq!(k("/* a */delete").as_deref(), Some("DELETE"));
        assert_eq!(k("update;").as_deref(), Some("UPDATE"));
        assert_eq!(k(" ; ;-- x\n delete").as_deref(), Some("DELETE"));
        assert_eq!(k("(select 1)"), None);
        assert_eq!(k("-- only a comment"), None);
        assert_eq!(k("/* unterminated"), None);
        assert_eq!(k(""), None);
    }

    #[test]
    fn values_render_as_text() {
        assert_eq!(value_text(ValueRef::Null), None);
        assert_eq!(value_text(ValueRef::Integer(-7)).as_deref(), Some("-7"));
        assert_eq!(value_text(ValueRef::Real(0.25)).as_deref(), Some("0.25"));
        assert_eq!(value_text(ValueRef::Real(3.0)).as_deref(), Some("3.0"));
        assert_eq!(
            value_text(ValueRef::Text(b"h\xffi")).as_deref(),
            Some("h\u{fffd}i")
        );
        assert_eq!(
            value_text(ValueRef::Blob(&[0, 171, 255])).as_deref(),
            Some("\\x00abff")
        );
    }
}
