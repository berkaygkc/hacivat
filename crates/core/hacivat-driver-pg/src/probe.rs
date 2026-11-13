//! Detection and post-connect probes: version, replica, TLS, user, databases (HV-CONN-07).
//!
//! Probes use the simple-query protocol where they take no parameters, so they also work
//! through a transaction-pooling proxy that cannot keep prepared statements.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelHandle, ExecOpts, ExecSummary, ProbeWarning, RawConn, RawError, SemVerLoose, ServerInfo,
    TxStatus,
};
use hacivat_net::{TlsMode, TlsSummary, TlsVerified};
use hacivat_proto::DbmsKind;
use tokio::task::JoinHandle;
use tokio_postgres::{Client, SimpleQueryMessage};

use crate::cancel::PgCancel;
use crate::connect::pg_error;
use crate::exec::{Exec, Notices};
use crate::tls::LocalTls;
use crate::wire::WireState;

/// How long `close` waits for the server to see the `Terminate` before dropping the socket.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The version of a PostgreSQL server (HV-DRV-01): the numbers from `server_version_num`
/// (`170011` is 17.11, `90624` is 9.6.24), else parsed loosely from `version`. `raw` is always
/// the `version()` text, so a pre-release (`17beta1`, `18devel`, `16rc1`) stays visible to
/// `support_status`, which reports it as not tested.
pub fn server_version(version_num: Option<&str>, version: &str) -> Option<SemVerLoose> {
    let num = version_num
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|n| n.parse::<u32>().ok())
        .filter(|n| *n >= 10_000);
    match num {
        Some(n) if n >= 100_000 => Some(SemVerLoose {
            major: n / 10_000,
            minor: n % 10_000,
            patch: 0,
            raw: version.to_string(),
        }),
        Some(n) => Some(SemVerLoose {
            major: n / 10_000,
            minor: n / 100 % 100,
            patch: n % 100,
            raw: version.to_string(),
        }),
        None => SemVerLoose::parse(version),
    }
}

/// How far the certificate was checked in `mode` (the fallback when the handshake recorded no
/// outcome).
fn verified_for(mode: Option<TlsMode>) -> TlsVerified {
    match mode {
        Some(TlsMode::Auto | TlsMode::VerifyFull) => TlsVerified::Full,
        Some(TlsMode::VerifyCa) => TlsVerified::CaOnly,
        Some(TlsMode::Require | TlsMode::Disable) | None => TlsVerified::No,
    }
}

/// The first row of a simple query, as text cells.
async fn first_row(client: &Client, sql: &str) -> Result<Option<Vec<Option<String>>>, RawError> {
    let messages = client.simple_query(sql).await.map_err(|e| pg_error(&e))?;
    Ok(messages.into_iter().find_map(|m| match m {
        SimpleQueryMessage::Row(row) => Some(
            (0..row.len())
                .map(|i| row.get(i).map(str::to_string))
                .collect(),
        ),
        _ => None,
    }))
}

/// The first cell of a simple query's first row.
async fn scalar(client: &Client, sql: &str) -> Result<Option<String>, RawError> {
    Ok(first_row(client, sql)
        .await?
        .and_then(|row| row.into_iter().next().flatten()))
}

async fn in_recovery(client: &Client) -> Result<Option<bool>, RawError> {
    Ok(scalar(client, "SELECT pg_is_in_recovery()")
        .await?
        .map(|v| v == "t"))
}

/// A live PostgreSQL connection (HV-CONN-01). The connection task runs on the core runtime
/// until [`RawConn::close`] or drop.
pub struct PgConn {
    client: Client,
    task: TaskGuard,
    /// The TLS mode, when this connection runs over TLS.
    tls_mode: Option<TlsMode>,
    /// What the handshake negotiated, when it ran.
    local_tls: Option<LocalTls>,
    // ## m2b-t10-pg
    /// What the wire observer records: PID, transaction status, command tags (Q11, Q12).
    wire: Arc<WireState>,
    /// Notice codes collected by the connection task (Q12).
    notices: Notices,
    cancel: Arc<PgCancel>,
}

impl PgConn {
    pub(crate) fn new(
        client: Client,
        task: JoinHandle<()>,
        tls_mode: Option<TlsMode>,
        local_tls: Option<LocalTls>,
        wire: Arc<WireState>,
        notices: Notices,
        cancel: PgCancel,
    ) -> Self {
        PgConn {
            client,
            task: TaskGuard(Some(task)),
            tls_mode,
            local_tls,
            wire,
            notices,
            cancel: Arc::new(cancel),
        }
    }

    /// A connection whose status could not be resynchronized is cut at once (fix round 1,
    /// I2): no later reply can be matched to the wrong request.
    fn drop_if_broken(&self) {
        if self.wire.is_broken()
            && let Some(task) = &self.task.0
        {
            task.abort();
        }
    }

    fn exec(&self) -> Exec<'_> {
        Exec {
            client: &self.client,
            wire: &self.wire,
            notices: &self.notices,
        }
    }

    /// The `tokio-postgres` client, for the driver's own tests.
    #[doc(hidden)]
    pub fn client(&self) -> &Client {
        &self.client
    }

    async fn server_info_inner(&self) -> Result<ServerInfo, RawError> {
        // Older forks may lack either; the version text is the one that must answer.
        let num = scalar(&self.client, "SHOW server_version_num")
            .await
            .ok()
            .flatten();
        let text = scalar(&self.client, "SELECT version()")
            .await?
            .unwrap_or_default();
        let is_replica = in_recovery(&self.client).await.ok().flatten();
        Ok(ServerInfo {
            kind: DbmsKind::Postgres,
            product: "PostgreSQL".into(),
            version: server_version(num.as_deref(), &text),
            raw_version: text,
            is_replica,
        })
    }

    async fn tls_summary_inner(&self) -> TlsSummary {
        let row = first_row(
            &self.client,
            "SELECT ssl, version, cipher FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
        )
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        let cell = |i: usize| row.get(i).cloned().flatten();
        let server_ssl = cell(0).is_some_and(|v| v == "t");
        let local = self.local_tls.as_ref();
        TlsSummary {
            // Our hop is what counts; a proxy in front of the server may terminate TLS.
            encrypted: local.is_some() || server_ssl,
            version: cell(1).or_else(|| local.and_then(|l| l.version.clone())),
            cipher: cell(2).or_else(|| local.and_then(|l| l.cipher.clone())),
            // The handshake's own outcome (a pin is `Pinned` in any mode), else the mode's.
            verified: if local.is_some() {
                self.wire
                    .outcome()
                    .unwrap_or_else(|| verified_for(self.tls_mode))
            } else {
                TlsVerified::No
            },
        }
    }
}

/// Aborts the connection task when the connection is dropped without `close` (a stuck
/// socket must not outlive its connection).
struct TaskGuard(Option<JoinHandle<()>>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}

impl RawConn for PgConn {
    fn kind(&self) -> DbmsKind {
        DbmsKind::Postgres
    }

    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        Box::pin(self.server_info_inner())
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        Box::pin(async move {
            let start = Instant::now();
            // The empty simple query: one round trip, no parse, no plan.
            self.client
                .batch_execute("")
                .await
                .map_err(|e| pg_error(&e))?;
            Ok(start.elapsed())
        })
    }

    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        Box::pin(self.tls_summary_inner())
    }

    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        Box::pin(async move {
            match in_recovery(&self.client).await {
                Ok(Some(true)) => vec![ProbeWarning::Replica],
                _ => vec![],
            }
        })
    }

    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        Box::pin(scalar(&self.client, "SELECT current_user"))
    }

    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        Box::pin(async move {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let rows = self
                .client
                .query(
                    "SELECT datname FROM pg_database WHERE datallowconn AND \
                     has_database_privilege(datname,'CONNECT') ORDER BY 1 LIMIT $1",
                    &[&limit],
                )
                .await
                .map_err(|e| pg_error(&e))?;
            Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
        })
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            let PgConn {
                client, mut task, ..
            } = *self;
            // Dropping the last client sends `Terminate`; the task then ends on its own.
            drop(client);
            if let Some(task) = task.0.take() {
                let abort = task.abort_handle();
                if tokio::time::timeout(CLOSE_GRACE, task).await.is_err() {
                    abort.abort();
                }
            }
        })
    }

    /// The `CancelRequest` over a fresh stream from the connection's factory, with its TLS
    /// settings (B23).
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(self.cancel.clone())
    }

    // ## m2b-t10-pg

    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        Box::pin(async move {
            let r = self.exec().execute(sql, opts).await;
            self.drop_if_broken();
            r
        })
    }

    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        Box::pin(async move {
            let r = self.exec().batch(sql).await;
            self.drop_if_broken();
            r
        })
    }

    fn tx_status(&self) -> TxStatus {
        self.wire.tx()
    }

    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        Box::pin(async move {
            let r = self.exec().reset().await;
            self.drop_if_broken();
            r
        })
    }

    fn backend_id(&self) -> Option<u64> {
        self.wire
            .backend_pid()
            .and_then(|pid| u64::try_from(pid).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_follows_the_mode() {
        assert_eq!(verified_for(Some(TlsMode::VerifyFull)), TlsVerified::Full);
        assert_eq!(verified_for(Some(TlsMode::Auto)), TlsVerified::Full);
        assert_eq!(verified_for(Some(TlsMode::VerifyCa)), TlsVerified::CaOnly);
        assert_eq!(verified_for(Some(TlsMode::Require)), TlsVerified::No);
        assert_eq!(verified_for(None), TlsVerified::No);
    }
}
