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
//!   server answered with an error, the connection is still in step and one `COM_PING` reads
//!   the status again: an earlier statement of the same text may have opened a transaction, a
//!   deadlock (1213) rolls one back, and a failing DDL commits implicitly first. `COM_PING` is
//!   not a statement, so the diagnostics area (`SHOW WARNINGS`, `SHOW ERRORS`,
//!   `GET DIAGNOSTICS`, `@@warning_count`) still describes the failed statement (review I1). If
//!   the ping fails too, the previous status stays.

use hacivat_driver::{ExecOpts, ExecSummary, Protocol, RawError, RawErrorKind, TxStatus};
use mysql_async::consts::StatusFlags;
use mysql_async::prelude::{Protocol as WireProtocol, Queryable};
use mysql_async::{Conn, QueryResult, Row, Value};

use crate::connect::raw_error;

/// `ER_UNSUPPORTED_PS`: "This command is not supported in the prepared statement protocol yet".
const UNSUPPORTED_PS: u16 = 1295;

/// The transaction status in a server's status flags (`SERVER_STATUS_IN_TRANS`).
#[doc(hidden)]
pub fn tx_from_status(flags: StatusFlags) -> TxStatus {
    if flags.contains(StatusFlags::SERVER_STATUS_IN_TRANS) {
        TxStatus::InTx
    } else {
        TxStatus::Idle
    }
}

// ## m2b-t17-liveness (Task 17 fix round 4, I9/R5)
/// Whether a server's status flags say the session autocommits (`SERVER_STATUS_AUTOCOMMIT`).
#[doc(hidden)]
pub fn autocommit_from_status(flags: StatusFlags) -> bool {
    flags.contains(StatusFlags::SERVER_STATUS_AUTOCOMMIT)
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

// ## m2b-fix-f2
/// How [`execute`] sends a statement the caller asked to run as a prepared statement
/// (`Protocol::Extended`), decided from its text before anything is sent (finding F2).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtendedRoute {
    /// `COM_STMT_PREPARE` and `COM_STMT_EXECUTE`, no parameters.
    Prepared,
    /// `COM_QUERY`: the statement is a `SET`.
    Text,
    /// Refused before anything is sent, as a statement the read-only connection will not
    /// prepare (`Unsupported`): a read of the text the server may make is a `SET`, another is
    /// not.
    Refused,
}

/// What a reading of a statement starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lead {
    /// The keyword `SET`.
    Set,
    /// Anything else: another keyword, or a character that starts no keyword (`(`, `-`).
    Other,
    /// Nothing: only whitespace and comments (the server runs no statement).
    Nothing,
}

/// More readings than any statement written by hand has (one per leading version-gated
/// executable comment that holds a keyword, plus one). Past it the readings are incomplete.
const MAX_LEADS: usize = 64;

/// Skips a MySQL line comment at the start of `s`: `#`, or `--` followed by whitespace, a
/// control character or the end (MySQL reads `--x` as two minus signs).
fn skip_line_comment(s: &str) -> Option<&str> {
    let after = s.strip_prefix('#').or_else(|| {
        s.strip_prefix("--").filter(|r| {
            r.chars()
                .next()
                .is_none_or(|c| c.is_ascii_whitespace() || c.is_ascii_control())
        })
    })?;
    Some(after.find('\n').map_or("", |i| &after[i + 1..]))
}

/// What `s` starts with after whitespace, line comments and ordinary block comments. An
/// executable comment is not looked into: [`leads`] handles those.
fn lead_of(mut s: &str) -> Lead {
    loop {
        s = s.trim_start();
        if let Some(r) = skip_line_comment(s) {
            s = r;
        } else if let Some(r) = s.strip_prefix("/*") {
            let Some(end) = r.find("*/") else {
                return Lead::Nothing;
            };
            s = &r[end + 2..];
        } else {
            break;
        }
    }
    let word: String = s.chars().take_while(char::is_ascii_alphabetic).collect();
    if s.is_empty() {
        Lead::Nothing
    } else if word.eq_ignore_ascii_case("SET") {
        Lead::Set
    } else {
        Lead::Other
    }
}

/// Every way the server may read the start of `sql`; `None` when there are more than
/// [`MAX_LEADS`]. Whitespace, line comments and ordinary block comments are skipped (MySQL's
/// block comments do not nest). An unversioned executable comment (`/*!…*/`) always runs, so a
/// statement inside it is the only reading. A version-gated one (`/*!80000 …*/`), and MariaDB's
/// `/*M!…*/` (an ordinary comment to MySQL), runs or not depending on the server, so one that
/// holds a statement is read both ways: that statement, and what follows the comment. One that
/// holds nothing reads the same either way.
fn leads(sql: &str) -> Option<Vec<Lead>> {
    let mut found = Vec::new();
    let mut rest = sql;
    loop {
        if found.len() > MAX_LEADS {
            return None;
        }
        rest = rest.trim_start();
        if let Some(r) = skip_line_comment(rest) {
            rest = r;
            continue;
        }
        let Some(r) = rest.strip_prefix("/*") else {
            break;
        };
        let Some(end) = r.find("*/") else {
            break;
        };
        let (inside, after) = (&r[..end], &r[end + 2..]);
        let code = match (inside.strip_prefix('!'), inside.strip_prefix("M!")) {
            (Some(code), _) => Some((code, false)),
            (None, Some(code)) => Some((code, true)),
            (None, None) => None,
        };
        if let Some((code, mariadb)) = code {
            let body = code.trim_start_matches(|c: char| c.is_ascii_digit());
            let gated = mariadb || body.len() != code.len();
            match lead_of(body) {
                Lead::Nothing => {}
                inner if !gated => {
                    found.push(inner);
                    return Some(found);
                }
                inner => found.push(inner),
            }
        }
        rest = after;
    }
    found.push(lead_of(rest));
    Some(found)
}

/// How [`execute`] sends `sql` when the caller asked for a prepared statement (finding F2).
///
/// MySQL 8.0 and 8.4 answer the `COM_STMT_PREPARE` of a `SET` that changes a global or
/// persisted variable the account may not change (`SET GLOBAL`, `SET PERSIST`,
/// `SET PERSIST_ONLY`, `SET @@global.…`, `SET GLOBAL TRANSACTION …`) with a bare OK packet
/// instead of the prepare reply or `1227`; `mysql_async` cannot parse it as a prepare reply,
/// and the connection is dropped. Every `SET` therefore runs over `COM_QUERY`, where the
/// server answers `1227`. A `SET` takes no parameters here (none are ever bound), returns no
/// rows and is one statement, so its effect is the same; every other statement stays
/// prepared, so a statement the server will not prepare (`ER_UNSUPPORTED_PS`, 1295) is still
/// found by the server.
///
/// - Read-write: `Text` when any reading of the text is a `SET` (a read-write connection may
///   run a multi-statement text: so does `Protocol::Simple`).
/// - Read-only: `Text` only when every reading that holds a statement is a `SET`, so the
///   prepared protocol's refusal of what it cannot prepare is never bypassed. A text that may
///   be read either as a `SET` or as something else (a version-gated executable comment) is
///   `Refused`; one that holds no `SET` is `Prepared`.
#[doc(hidden)]
pub fn extended_route(sql: &str, read_only: bool) -> ExtendedRoute {
    let Some(found) = leads(sql) else {
        // More readings than any hand-written text: keep the server's own refusal.
        return ExtendedRoute::Prepared;
    };
    let any_set = found.contains(&Lead::Set);
    let any_other = found.contains(&Lead::Other);
    match (any_set, any_other) {
        (false, _) => ExtendedRoute::Prepared,
        (true, false) => ExtendedRoute::Text,
        (true, true) if read_only => ExtendedRoute::Refused,
        (true, true) => ExtendedRoute::Text,
    }
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
/// `COM_PING`, whose OK packet carries the flags (the ERR packet has none) and which leaves the
/// diagnostics area alone; otherwise it stays.
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
            if conn.ping().await.is_ok()
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
        // ## m2b-fix-f2: a `SET` is never prepared (see `extended_route`).
        Protocol::Extended => match extended_route(sql, read_only) {
            ExtendedRoute::Text => text(conn, sql, Keep::Count).await,
            // Nothing was sent, so the status stays as it was.
            ExtendedRoute::Refused => {
                return Err(RawError {
                    kind: RawErrorKind::Unsupported,
                    server: None,
                });
            }
            ExtendedRoute::Prepared => match prepared(conn, sql).await {
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
