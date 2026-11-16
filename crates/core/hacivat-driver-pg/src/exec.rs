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
//!   is then exact (Q12). When the wait expires the connection is broken, never trusted with a
//!   stale status (fix round 1, I2).
//! - The command tag, rows and columns of a multi-statement simple query are the last
//!   statement's: a caller that decides from the tag (the read-only post-check) runs single
//!   statements through the extended protocol (review M2).

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::{StreamExt, TryStreamExt};
use hacivat_driver::{
    ExecOpts, ExecSummary, Notice, NoticeSeverity, Protocol, RawError, RawErrorKind,
};
use tokio_postgres::error::{DbError, Severity};
use tokio_postgres::types::ToSql;
use tokio_postgres::{AsyncMessage, Client, Connection, SimpleQueryMessage};

use crate::connect::pg_error;
use crate::wire::{SETTLE, Settled, WireState};

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

/// The notices the connection task collects (Q12): their codes for `execute` and the warning
/// count, and (m3b-t13) their text for the open typed query.
pub(crate) type Notices = Arc<Mutex<NoticeBuf>>;

// ## m3b-t13-drivers
/// What the connection task keeps of the server's notices.
///
/// - `codes`: every notice's SQLSTATE, the newest [`NOTICE_CAP`] (M2b's buffer, unchanged).
/// - `texts`: the notices of the open typed query as [`Notice`]s (R-M3-77: severity, code and
///   the server's message verbatim, cut at [`Notice::MAX_TEXT_BYTES`] by [`Notice::new`]): the
///   first [`NOTICE_CAP`] of the query only (`taken` counts those already handed out), and
///   only while a typed query collects them (`collect`): never during a statement run beside a
///   suspended query, never between queries. The text is never written to a log; it leaves the
///   driver only as `QueryEvent::Notice`.
#[derive(Default)]
pub(crate) struct NoticeBuf {
    codes: VecDeque<String>,
    texts: VecDeque<Notice>,
    taken: usize,
    collect: bool,
    // ## m3b-t13-drivers (fix round 1, M-3)
    /// The query's notices past the cap: counted, never kept (`take_notices_dropped`).
    dropped: u32,
    /// Wakes the open typed query when a notice is kept, so a statement that runs long (a
    /// PL/pgSQL loop raising progress notices) hands them out as they come, not at its end.
    wake: Option<UnboundedSender<()>>,
}

/// The open typed query's end of [`NoticeBuf`]'s wake signal.
pub(crate) type NoticeWake = UnboundedReceiver<()>;

// The buffer holds server text: its `Debug` prints counts only.
impl std::fmt::Debug for NoticeBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoticeBuf")
            .field("codes", &self.codes.len())
            .field("texts", &self.texts.len())
            .finish_non_exhaustive()
    }
}

fn lock(notices: &Notices) -> std::sync::MutexGuard<'_, NoticeBuf> {
    notices.lock().unwrap_or_else(PoisonError::into_inner)
}

fn push_notice(notices: &Notices, code: &str) {
    let mut q = lock(notices);
    if q.codes.len() == NOTICE_CAP {
        q.codes.pop_front();
    }
    q.codes.push_back(code.to_string());
}

// ## m3b-t13-drivers
/// A notice's severity as the server labels it (the non-localised field when the server sent
/// it, else the word itself); anything else is `Notice`.
fn severity_of(notice: &DbError) -> NoticeSeverity {
    let parsed = notice.parsed_severity().or(match notice.severity() {
        "WARNING" => Some(Severity::Warning),
        "DEBUG" => Some(Severity::Debug),
        "LOG" => Some(Severity::Log),
        "INFO" => Some(Severity::Info),
        _ => None,
    });
    match parsed {
        Some(Severity::Warning) => NoticeSeverity::Warning,
        Some(Severity::Debug) => NoticeSeverity::Debug,
        Some(Severity::Log) => NoticeSeverity::Log,
        Some(Severity::Info) => NoticeSeverity::Info,
        _ => NoticeSeverity::Notice,
    }
}

/// Keeps one notice: its code always, its text while a typed query collects and fewer than
/// [`NOTICE_CAP`] of the query's were kept.
fn keep_notice(notices: &Notices, notice: &DbError) {
    push_notice(notices, notice.code().code());
    let mut q = lock(notices);
    if !q.collect {
        return;
    }
    if q.texts.len() + q.taken < NOTICE_CAP {
        let kept = Notice::new(
            severity_of(notice),
            Some(notice.code().code()),
            notice.message(),
        );
        q.texts.push_back(kept);
        if let Some(wake) = &q.wake {
            let _ = wake.unbounded_send(());
        }
    } else {
        q.dropped = q.dropped.saturating_add(1);
    }
}

/// A typed query starts: earlier texts (and their drop count) are dropped and this query's are
/// collected; the answer wakes it when one is kept.
pub(crate) fn collect_notice_texts(notices: &Notices) -> NoticeWake {
    let (wake, woken) = unbounded();
    let mut q = lock(notices);
    q.texts.clear();
    q.taken = 0;
    q.dropped = 0;
    q.collect = true;
    q.wake = Some(wake);
    woken
}

// ## m3b-t13-drivers (fix round 1, M-3)
/// The notices of the current or last typed query dropped past the cap, taken once.
pub(crate) fn take_notices_dropped(notices: &Notices) -> u32 {
    std::mem::take(&mut lock(notices).dropped)
}

/// The typed query ended: nothing more is collected or kept.
pub(crate) fn stop_notice_texts(notices: &Notices) {
    let mut q = lock(notices);
    q.texts.clear();
    q.collect = false;
    q.wake = None;
}

/// The oldest notice of the open typed query not yet handed out.
pub(crate) fn next_notice_text(notices: &Notices) -> Option<Notice> {
    let mut q = lock(notices);
    let n = q.texts.pop_front()?;
    q.taken += 1;
    Some(n)
}

pub(crate) fn drain_notices(notices: &Notices) -> Vec<String> {
    lock(notices).codes.drain(..).collect()
}

// ## m3a-t22-typed-readonly
/// Puts `held` back in front of what the buffer holds now, as the older codes, within
/// [`NOTICE_CAP`] (the oldest go first). A statement run beside a suspended typed query (the
/// read-only guard's canary, R-M3-17) takes `execute`'s usual fresh start, so the codes the
/// open query raised before it are held aside and restored: they still reach the query's
/// `QueryDone::warnings` (ledger T22 note).
pub(crate) fn restore_notices(notices: &Notices, held: Vec<String>) {
    if held.is_empty() {
        return;
    }
    let mut q = lock(notices);
    let newer: Vec<String> = q.codes.drain(..).collect();
    q.codes.extend(held);
    q.codes.extend(newer);
    while q.codes.len() > NOTICE_CAP {
        q.codes.pop_front();
    }
}

/// Drives the connection, keeping notice codes (and, m3b-t13, the open typed query's notice
/// text, never logged) and dropping notifications, until it ends; then wakes anyone waiting on
/// the wire state.
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
            // ## m3b-t13-drivers: the code, and the text for an open typed query (R-M3-77).
            Ok(AsyncMessage::Notice(notice)) => keep_notice(&notices, &notice),
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
    ///
    /// Fix round 1: when they do not all arrive within [`SETTLE`] (I2), the connection is
    /// marked broken and the error is a lost connection (`Io(TimedOut)`, the server's message
    /// kept), so the session breaks instead of trusting a stale status. When the connection
    /// ended instead (R3), it is broken too, but a server error keeps its class (`Db`, with
    /// the SQLSTATE); `Io(TimedOut)` means only that the wait expired. A `COPY … FROM STDIN`
    /// the observer refused (I1) is `Unsupported`.
    async fn request<R>(
        &self,
        op: impl Future<Output = Result<R, tokio_postgres::Error>>,
    ) -> Result<R, RawError> {
        if self.wire.is_broken() {
            return Err(RawError {
                kind: RawErrorKind::Io(io::ErrorKind::NotConnected),
                server: None,
            });
        }
        drain_notices(self.notices);
        self.wire.clear_tag();
        let _ = self.wire.take_copy_refused();
        // ## m3b-t13-drivers: a statement beside a suspended typed query raises notices of its
        // own; their text is not the query's (R-M3-77), so none is collected meanwhile.
        let collecting = std::mem::replace(&mut lock(self.notices).collect, false);
        let r = op.await;
        lock(self.notices).collect = collecting;
        match r {
            Ok(r) => Ok(r),
            Err(e) => {
                let settled = self.wire.settle(SETTLE).await;
                let mut raw = pg_error(&e);
                match settled {
                    Settled::Synced => {
                        if self.wire.take_copy_refused() {
                            raw = RawError {
                                kind: RawErrorKind::Unsupported,
                                server: None,
                            };
                        }
                    }
                    // The server ended the connection (a FATAL `57P01`, say): its own class
                    // and SQLSTATE stand (R3).
                    Settled::Closed => {
                        self.wire.mark_broken();
                        if !matches!(raw.kind, RawErrorKind::Db | RawErrorKind::Io(_)) {
                            raw.kind = RawErrorKind::Io(io::ErrorKind::ConnectionAborted);
                        }
                    }
                    Settled::Expired => {
                        self.wire.mark_broken();
                        if !matches!(raw.kind, RawErrorKind::Io(_)) {
                            raw.kind = RawErrorKind::Io(io::ErrorKind::TimedOut);
                        }
                    }
                }
                Err(raw)
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
                        SimpleQueryMessage::Row(row) => {
                            Some(row.try_get(0).map(|c| c.map(str::to_string)))
                        }
                        _ => None,
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .flatten()
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
    // The first cell that would not decode. The rest of the result is still read up to the
    // ReadyForQuery, however long it takes, so the connection stays in step and the error is
    // `Other`, never a settle timeout on a large result (R2).
    let mut decode_error = None;
    while let Some(message) = stream.try_next().await? {
        if decode_error.is_some() {
            continue;
        }
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
                    // A cell that is not UTF-8 (a SQL_ASCII database) fails the statement with
                    // a decode error (`RawErrorKind::Other`), never a panic (fix round 1, I3).
                    match (0..row.len())
                        .map(|i| row.try_get(i).map(|c| c.map(str::to_string)))
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(cells) => s.rows.push(cells),
                        Err(e) => decode_error = Some(e),
                    }
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
    match decode_error {
        Some(e) => Err(e),
        None => Ok(s),
    }
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

    // ## m3a-t22-typed-readonly
    #[test]
    fn held_notices_go_back_before_newer_ones_within_the_cap() {
        let notices: Notices = Arc::default();
        push_notice(&notices, "01000");
        push_notice(&notices, "01001");
        let held = drain_notices(&notices);
        push_notice(&notices, "00000");
        restore_notices(&notices, held);
        assert_eq!(drain_notices(&notices), ["01000", "01001", "00000"]);
        // Past the cap the oldest (the held ones first) go.
        let held: Vec<String> = (0..10).map(|i| format!("h{i}")).collect();
        for i in 0..95 {
            push_notice(&notices, &format!("n{i}"));
        }
        restore_notices(&notices, held);
        let codes = drain_notices(&notices);
        assert_eq!(codes.len(), NOTICE_CAP);
        assert_eq!(codes.first().map(String::as_str), Some("h5"));
        assert_eq!(codes.last().map(String::as_str), Some("n94"));
        restore_notices(&notices, Vec::new());
        assert!(drain_notices(&notices).is_empty());
    }
}
