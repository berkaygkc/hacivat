//! Statement execution on a MySQL or MariaDB connection (HV-EXEC-05, Task 13): `execute`,
//! `batch`, the transaction status from the server status flags and `COM_RESET_CONNECTION`.
//!
//! - `Protocol::Simple` runs `COM_QUERY` (`query_iter`): the rows as text, at most `max_rows`
//!   kept, the rest counted. Several statements run only on a read-write connection (a
//!   read-only one never asks for `CLIENT_MULTI_STATEMENTS`, patch §3); the columns and rows are
//!   then the last result set's.
//! - `Protocol::Extended` runs a prepared statement (`exec_iter(sql, ())`), counting the rows and
//!   dropping their values. A statement the server cannot prepare (`ER_UNSUPPORTED_PS`, 1295) is
//!   `RawErrorKind::Unsupported` on a read-only connection, never re-sent as `COM_QUERY`; on a
//!   read-write connection it runs through `query_iter` instead.
//! - `rows_affected` and the transaction status come from the last OK packet the statement drew
//!   (T5 carry-in): a result set reports 0 affected rows there, never its row count, so the
//!   guard's post-check can read a positive count on `SELECT`/`WITH`/`CALL`/`DO` as a write.
//! - The command tag is the statement's first keyword in ASCII upper case ([`command_tag`];
//!   MySQL has no command tags, Q35).
//! - An ERR packet carries no status flags, so it never sets the status by itself. When the
//!   server answered with an error, the connection is still in step and one `DO 0` reads the
//!   status again: an earlier statement of the same text may have opened a transaction, and a
//!   deadlock (1213) rolls one back. If that fails too, the previous status stays.

use hacivat_driver::{ExecOpts, ExecSummary, Protocol, RawError, RawErrorKind, TxStatus};
use mysql_async::consts::StatusFlags;
use mysql_async::prelude::{Protocol as WireProtocol, Queryable};
use mysql_async::{Conn, QueryResult, Row, Value};

use crate::connect::raw_error;

/// `ER_UNSUPPORTED_PS`: "This command is not supported in the prepared statement protocol yet".
const UNSUPPORTED_PS: u16 = 1295;

/// Reads the transaction status again after a server error; changes nothing.
const STATUS_PROBE: &str = "DO 0";

/// The transaction status in a server's status flags (`SERVER_STATUS_IN_TRANS`).
#[doc(hidden)]
pub fn tx_from_status(flags: StatusFlags) -> TxStatus {
    if flags.contains(StatusFlags::SERVER_STATUS_IN_TRANS) {
        TxStatus::InTx
    } else {
        TxStatus::Idle
    }
}

/// The statement's first keyword in ASCII upper case, `None` when it does not start with one
/// (Q35). Leading whitespace and comments (`-- `, `#`, `/* */`) are skipped. A MySQL or MariaDB
/// executable comment (`/*!…`, `/*!50000 …`, `/*M!100000 …`) is code the server runs, so the
/// scan continues inside it after its version number, and past its end when it is empty
/// (`/*!*/ DELETE …`). An unterminated comment has no keyword.
#[doc(hidden)]
pub fn command_tag(sql: &str) -> Option<String> {
    let mut rest = sql;
    let mut executable_open = false;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("--").or_else(|| rest.strip_prefix('#')) {
            rest = r.find('\n').map_or("", |i| &r[i + 1..]);
        } else if let Some(r) = rest.strip_prefix("*/").filter(|_| executable_open) {
            executable_open = false;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("/*") {
            let executable = r.strip_prefix('!').or_else(|| r.strip_prefix("M!"));
            rest = match executable {
                Some(code) => {
                    executable_open = true;
                    code.trim_start_matches(|c: char| c.is_ascii_digit())
                }
                None => &r[r.find("*/")? + 2..],
            };
        } else {
            break;
        }
    }
    let keyword: String = rest
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect::<String>()
        .to_ascii_uppercase();
    (!keyword.is_empty()).then_some(keyword)
}

/// What a statement's rows are kept as.
#[derive(Clone, Copy)]
enum Keep {
    /// Text, at most this many rows.
    Text(usize),
    /// Counted only.
    Count,
}

/// A text-protocol cell: SQL NULL is `None`, bytes are text (a byte that is not UTF-8 becomes
/// U+FFFD, never a panic).
fn cell(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        other => Some(other.as_sql(true).trim_matches('\'').to_owned()),
    }
}

fn cells(row: &Row) -> Vec<Option<String>> {
    (0..row.len()).map(|i| cell(row.as_ref(i))).collect()
}

/// Reads every result set of `result`: the last set with columns gives `columns` and `rows`.
async fn drain<P: WireProtocol>(
    mut result: QueryResult<'_, 'static, P>,
    keep: Keep,
) -> Result<ExecSummary, mysql_async::Error> {
    let mut s = ExecSummary::default();
    loop {
        if let Some(columns) = result.columns().filter(|c| !c.is_empty()) {
            s.columns = columns.iter().map(|c| c.name_str().into_owned()).collect();
            s.rows.clear();
            s.row_count = 0;
            s.truncated = false;
        }
        while let Some(row) = result.next().await? {
            s.row_count += 1;
            match keep {
                Keep::Text(max) if s.rows.len() < max => s.rows.push(cells(&row)),
                Keep::Text(_) => s.truncated = true,
                Keep::Count => {}
            }
        }
        if result.is_empty() {
            break;
        }
    }
    result.drop_result().await?;
    Ok(s)
}

async fn text(conn: &mut Conn, sql: &str, keep: Keep) -> Result<ExecSummary, mysql_async::Error> {
    let result = conn.query_iter(sql).await?;
    drain(result, keep).await
}

async fn prepared(conn: &mut Conn, sql: &str) -> Result<ExecSummary, mysql_async::Error> {
    let result = conn.exec_iter(sql, ()).await?;
    drain(result, Keep::Count).await
}

fn is_unsupported_ps(e: &mysql_async::Error) -> bool {
    matches!(e, mysql_async::Error::Server(s) if s.code == UNSUPPORTED_PS)
}

/// The status in the connection's last OK packet, if any.
fn last_status(conn: &Conn) -> Option<TxStatus> {
    conn.last_ok_packet()
        .map(|ok| tx_from_status(ok.status_flags()))
}

/// Updates `tx` after a call: from the OK packet on success; after a server error from a
/// `DO 0` (the ERR packet has no flags); otherwise it stays.
async fn settle<T>(
    conn: &mut Conn,
    tx: &mut TxStatus,
    ran: Result<T, mysql_async::Error>,
) -> Result<T, mysql_async::Error> {
    match &ran {
        Ok(_) => {
            if let Some(status) = last_status(conn) {
                *tx = status;
            }
        }
        Err(mysql_async::Error::Server(_)) => {
            if conn.query_drop(STATUS_PROBE).await.is_ok()
                && let Some(status) = last_status(conn)
            {
                *tx = status;
            }
        }
        Err(_) => {}
    }
    ran
}

/// Runs one user statement (HV-EXEC-05).
pub(crate) async fn execute(
    conn: &mut Conn,
    read_only: bool,
    tx: &mut TxStatus,
    sql: &str,
    opts: ExecOpts,
) -> Result<ExecSummary, RawError> {
    let ran = match opts.protocol {
        Protocol::Simple => text(conn, sql, Keep::Text(opts.max_rows)).await,
        Protocol::Extended => match prepared(conn, sql).await {
            // The refused prepare changed nothing, so the status stays as it was.
            Err(e) if is_unsupported_ps(&e) && read_only => {
                return Err(RawError {
                    kind: RawErrorKind::Unsupported,
                    server: None,
                });
            }
            Err(e) if is_unsupported_ps(&e) => text(conn, sql, Keep::Count).await,
            other => other,
        },
    };
    let mut summary = settle(conn, tx, ran).await.map_err(|e| raw_error(&e))?;
    summary.rows_affected = conn.last_ok_packet().map(|ok| ok.affected_rows());
    summary.command_tag = command_tag(sql);
    summary.tx = *tx;
    Ok(summary)
}

/// Runs trusted control text with `COM_QUERY`, every result dropped.
pub(crate) async fn batch(conn: &mut Conn, tx: &mut TxStatus, sql: &str) -> Result<(), RawError> {
    let ran = conn.query_drop(sql).await;
    settle(conn, tx, ran).await.map_err(|e| raw_error(&e))
}

/// `COM_RESET_CONNECTION`, after which `mysql_async` runs the connection's `setup` statements
/// again: `SET SESSION TRANSACTION READ ONLY` on a read-only connection. A server without the
/// command (MySQL before 5.7.3, MariaDB before 10.2.4) is `Unsupported`: the session was not
/// reset.
pub(crate) async fn reset(conn: &mut Conn, tx: &mut TxStatus) -> Result<(), RawError> {
    let ran = conn.reset().await;
    match settle(conn, tx, ran).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(RawError {
            kind: RawErrorKind::Unsupported,
            server: None,
        }),
        Err(e) => Err(raw_error(&e)),
    }
}
