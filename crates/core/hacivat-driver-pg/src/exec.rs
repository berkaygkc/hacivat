//! Statement execution on a PostgreSQL connection (HV-EXEC-05, Task 10): `execute`, `batch`,
//! the reset sequence (`DISCARD ALL` without `DEALLOCATE ALL`, Q10) and the notice buffer.
//!
//! - `Protocol::Simple` runs `simple_query`: the rows of the last statement as text, at most
//!   `max_rows` kept, the rest counted.
//! - `Protocol::Extended` prepares the statement and runs it with no parameters, counting the
//!   rows and dropping their values (one statement only: the server refuses several with
//!   `42601`).
//! - The command tag is the last `CommandComplete` the wire observer read; `rows_affected` its
//!   count.
//! - After an error, the call waits (at most 1 s) until every `ReadyForQuery` it is owed has
//!   been read, because `tokio-postgres` returns at the `ErrorResponse`: the transaction status
//!   is then exact (Q12).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use futures::{StreamExt, TryStreamExt};
use hacivat_driver::{ExecOpts, ExecSummary, Protocol, RawError};
use tokio_postgres::types::ToSql;
use tokio_postgres::{AsyncMessage, Client, Connection, SimpleQueryMessage};

use crate::connect::pg_error;
use crate::wire::{SETTLE, WireState};

/// The most notice codes a connection keeps between two statements; the oldest go first.
pub const NOTICE_CAP: usize = 100;

/// The `DISCARD ALL` sequence without its `DEALLOCATE ALL` step (Q10). `DEALLOCATE ALL` would
/// also drop `tokio-postgres`'s cached type-lookup statements, and the next extended query on an
/// enum, domain or composite column would fail with `26000`; SQL-level prepared statements are
/// deallocated by name afterwards instead ([`deallocate_sql`]).
pub const RESET_SQL: &str = "CLOSE ALL; SET SESSION AUTHORIZATION DEFAULT; RESET ALL; \
     UNLISTEN *; SELECT pg_advisory_unlock_all(); DISCARD PLANS; DISCARD TEMP; \
     DISCARD SEQUENCES;";

/// The SQL-level prepared statements of the session (`PREPARE`), never the protocol-level
/// ones `tokio-postgres` keeps.
pub const SQL_PREPARED: &str = "SELECT name FROM pg_prepared_statements WHERE from_sql";

/// `DEALLOCATE` for each name, each a quoted identifier; `None` when there is none.
pub fn deallocate_sql(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| {
        names
            .iter()
            .map(|n| format!("DEALLOCATE \"{}\";", n.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// The notice codes the connection task collects for `execute` (Q12).
pub(crate) type Notices = Arc<Mutex<VecDeque<String>>>;

fn push_notice(notices: &Notices, code: &str) {
    let mut q = notices.lock().unwrap_or_else(PoisonError::into_inner);
    if q.len() == NOTICE_CAP {
        q.pop_front();
    }
    q.push_back(code.to_string());
}

pub(crate) fn drain_notices(notices: &Notices) -> Vec<String> {
    notices
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain(..)
        .collect()
}

/// Drives the connection, keeping notice codes (never their text) and dropping notifications,
/// until it ends; then wakes anyone waiting on the wire state.
pub(crate) async fn drive<S, T>(
    mut connection: Connection<S, T>,
    notices: Notices,
    wire: Arc<WireState>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
    while let Some(message) = messages.next().await {
        match message {
            Ok(AsyncMessage::Notice(notice)) => push_notice(&notices, notice.code().code()),
            Ok(_) => {}
            // The error (if any) reaches the client as a closed connection.
            Err(_) => break,
        }
    }
    wire.close();
}

/// One connection's execution context.
pub(crate) struct Exec<'a> {
    pub client: &'a Client,
    pub wire: &'a WireState,
    pub notices: &'a Notices,
}

impl Exec<'_> {
    /// Runs `op` as one request: stale notices and tag dropped first, and after an error the
    /// outstanding `ReadyForQuery` messages awaited.
    async fn request<R>(
        &self,
        op: impl Future<Output = Result<R, tokio_postgres::Error>>,
    ) -> Result<R, RawError> {
        drain_notices(self.notices);
        self.wire.clear_tag();
        match op.await {
            Ok(r) => Ok(r),
            Err(e) => {
                self.wire.settle(SETTLE).await;
                Err(pg_error(&e))
            }
        }
    }

    pub async fn execute(&self, sql: &str, opts: ExecOpts) -> Result<ExecSummary, RawError> {
        let mut summary = match opts.protocol {
            Protocol::Simple => {
                self.request(simple(self.client, sql, opts.max_rows))
                    .await?
            }
            Protocol::Extended => self.request(extended(self.client, sql)).await?,
        };
        summary.command_tag = self.wire.last_tag();
        summary.notice_codes = drain_notices(self.notices);
        summary.tx = self.wire.tx();
        Ok(summary)
    }

    pub async fn batch(&self, sql: &str) -> Result<(), RawError> {
        self.request(self.client.batch_execute(sql)).await
    }

    /// The reset sequence (Q10), then the SQL-level prepared statements by name.
    pub async fn reset(&self) -> Result<(), RawError> {
        self.batch(RESET_SQL).await?;
        let names = self
            .request(async {
                let messages = self.client.simple_query(SQL_PREPARED).await?;
                Ok(messages
                    .into_iter()
                    .filter_map(|m| match m {
                        SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
                        _ => None,
                    })
                    .collect::<Vec<_>>())
            })
            .await?;
        match deallocate_sql(&names) {
            Some(sql) => self.batch(&sql).await,
            None => Ok(()),
        }
    }
}

/// The simple protocol: the last statement's rows as text, `max_rows` kept.
async fn simple(
    client: &Client,
    sql: &str,
    max_rows: usize,
) -> Result<ExecSummary, tokio_postgres::Error> {
    let stream = client.simple_query_raw(sql).await?;
    let mut stream = std::pin::pin!(stream);
    let mut s = ExecSummary::default();
    // A statement's results start after the previous statement's CommandComplete.
    let mut next_statement = false;
    while let Some(message) = stream.try_next().await? {
        if next_statement {
            s.columns.clear();
            s.rows.clear();
            s.row_count = 0;
            s.truncated = false;
            s.rows_affected = None;
            next_statement = false;
        }
        match message {
            SimpleQueryMessage::RowDescription(columns) => {
                s.columns = columns.iter().map(|c| c.name().to_string()).collect();
            }
            SimpleQueryMessage::Row(row) => {
                s.row_count += 1;
                if s.rows.len() < max_rows {
                    s.rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                } else {
                    s.truncated = true;
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                s.rows_affected = Some(n);
                next_statement = true;
            }
            _ => {}
        }
    }
    Ok(s)
}

/// The extended protocol: one statement, its rows counted and dropped.
async fn extended(client: &Client, sql: &str) -> Result<ExecSummary, tokio_postgres::Error> {
    let statement = client.prepare(sql).await?;
    let columns = statement
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let stream = client
        .query_raw(&statement, std::iter::empty::<&(dyn ToSql + Sync)>())
        .await?;
    let mut stream = std::pin::pin!(stream);
    let mut row_count = 0u64;
    while stream.try_next().await?.is_some() {
        row_count += 1;
    }
    Ok(ExecSummary {
        columns,
        row_count,
        rows_affected: stream.rows_affected(),
        ..ExecSummary::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deallocation_quotes_every_name_and_never_deallocates_all() {
        assert_eq!(deallocate_sql(&[]), None);
        assert_eq!(
            deallocate_sql(&["p".into(), "we\"ird".into()]).as_deref(),
            Some("DEALLOCATE \"p\"; DEALLOCATE \"we\"\"ird\";")
        );
        // A statement literally named ALL is a quoted identifier, never DEALLOCATE ALL.
        assert_eq!(
            deallocate_sql(&["ALL".into()]).as_deref(),
            Some("DEALLOCATE \"ALL\";")
        );
        assert!(!RESET_SQL.to_ascii_uppercase().contains("DEALLOCATE"));
        assert!(!RESET_SQL.to_ascii_uppercase().contains("DISCARD ALL"));
    }

    #[test]
    fn the_notice_buffer_keeps_the_newest_hundred() {
        let notices: Notices = Arc::default();
        for i in 0..150 {
            push_notice(&notices, &format!("{i:05}"));
        }
        let codes = drain_notices(&notices);
        assert_eq!(codes.len(), NOTICE_CAP);
        assert_eq!(codes.first().map(String::as_str), Some("00050"));
        assert_eq!(codes.last().map(String::as_str), Some("00149"));
        assert!(drain_notices(&notices).is_empty());
    }
}
