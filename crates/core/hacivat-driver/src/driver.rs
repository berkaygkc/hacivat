//! The driver traits: what an engine crate implements, and the raw connection it returns
//! (HV-DRV-01, HV-CONN-01). Complete since Task 1.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::{StreamFactory, Target, TlsParams, TlsSummary};
use hacivat_proto::DbmsKind;
use secrecy::SecretString;

use crate::errors::RawError;
use crate::server::ServerInfo;

/// How a running statement is cancelled (HV-DRV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelMechanism {
    /// PostgreSQL's CancelRequest on a new connection.
    ProtocolCancelRequest,
    /// MySQL's `KILL QUERY` on a second connection.
    KillQuery,
    /// SQLite's `sqlite3_interrupt`.
    Interrupt,
    None,
}

/// What a driver can do (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriverCaps {
    /// The driver speaks over a caller-supplied stream (else the loopback forwarder, R18).
    pub stream_injection: bool,
    /// TLS is negotiated inside the protocol (PostgreSQL `SSLRequest`, MySQL `CLIENT_SSL`).
    pub in_protocol_tls: bool,
    pub cancel: CancelMechanism,
    /// The endpoint is a file (SQLite).
    pub file_based: bool,
}

/// A driver's identity and capabilities (HV-DRV-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriverDescriptor {
    pub kind: DbmsKind,
    pub crate_name: &'static str,
    pub caps: DriverCaps,
}

/// Progress a driver reports during `connect`, for the step trail (HV-CONN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriverEvent {
    TlsStarted,
    TlsDone,
    AuthStarted,
    AuthDone,
}

/// The callback a driver reports [`DriverEvent`]s to (HV-CONN-01).
pub type DriverEvents = Arc<dyn Fn(DriverEvent) + Send + Sync>;

/// Where a driver connects (HV-CONN-01, P10, P13).
#[derive(Debug, Clone)]
pub enum Endpoint {
    Network {
        factory: StreamFactory,
        /// The direct target, for a driver that dials itself (MySQL Direct). `None` when a
        /// transport wraps the factory (P13).
        direct: Option<Target>,
        /// The pipeline's Resolve result, passed on as `resolved_ips` by a driver that dials
        /// itself (P10).
        resolved: Vec<SocketAddr>,
        /// The name TLS verifies and sends as SNI.
        tls_host: String,
    },
    File {
        path: PathBuf,
        create_if_missing: bool,
        read_only: bool,
    },
}

// ## m2b-t01-interfaces

/// A connection's transaction status, as the protocol reports it (HV-SESS-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TxStatus {
    #[default]
    Idle,
    InTx,
    /// In a transaction that failed: only a rollback is accepted (PostgreSQL).
    Failed,
}

/// Which wire protocol a statement uses (PostgreSQL's extended or simple query protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Protocol {
    #[default]
    Extended,
    Simple,
}

/// How one statement runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecOpts {
    /// The most rows kept; the rest are counted (`ExecSummary::truncated`).
    pub max_rows: usize,
    pub protocol: Protocol,
}

impl Default for ExecOpts {
    /// 1,000 rows, the extended protocol.
    fn default() -> Self {
        ExecOpts {
            max_rows: 1000,
            protocol: Protocol::Extended,
        }
    }
}

/// What one statement did. Values are text as the server sent them (`None` for SQL NULL).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct ExecSummary {
    pub command_tag: Option<String>,
    pub rows_affected: Option<u64>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub row_count: u64,
    pub truncated: bool,
    /// SQLSTATE codes of the notices the statement raised.
    pub notice_codes: Vec<String>,
    /// The transaction status after the statement.
    pub tx: TxStatus,
}

/// Whether a driver may send a password by an insecure method (cleartext or MD5 over an
/// unverified channel) (B8, Q20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InsecureAuth {
    /// Refuse unless the channel is verified TLS (or local).
    #[default]
    RefuseUnlessVerified,
    Allow,
}

/// One connect request to a driver (HV-CONN-01). Holds the password: its `Debug` redacts it.
pub struct DriverConnect {
    pub endpoint: Endpoint,
    pub tls: Option<TlsParams>,
    pub user: Option<String>,
    pub password: Option<SecretString>,
    pub database: Option<String>,
    pub application_name: String,
    pub read_only: bool,
    /// Sanitized driver options (HV-DRV-05).
    pub options: Vec<(String, String)>,
    pub connect_timeout: Duration,
    pub events: DriverEvents,
    // ## m2b-t01-interfaces
    /// The insecure-auth guard (B8, Q20); the drivers enforce it from Tasks 10 and 13.
    pub insecure_auth: InsecureAuth,
}

impl fmt::Debug for DriverConnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverConnect")
            .field("endpoint", &self.endpoint)
            .field("tls", &self.tls)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("database", &self.database)
            .field("application_name", &self.application_name)
            .field("read_only", &self.read_only)
            .field("options", &self.options)
            .field("connect_timeout", &self.connect_timeout)
            .field("insecure_auth", &self.insecure_auth)
            .finish_non_exhaustive()
    }
}

/// An engine driver (HV-DRV-01).
pub trait Driver: Send + Sync + 'static {
    fn descriptor(&self) -> &DriverDescriptor;
    fn connect(&self, req: DriverConnect)
    -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>>;
}

/// A finding of the post-connect probe, shown as a warning (HV-CONN-07).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProbeWarning {
    Replica,
    ServerReadOnly,
    SuperReadOnly,
    LowWaitTimeout { seconds: u32 },
    LocalInfileOn,
    SqliteFileNotWritable,
    SqliteJournal { mode: String },
}

/// Cancels the statement running on a connection (HV-DRV-01). `None` in M2a.
pub trait CancelHandle: Send + Sync {
    fn cancel(&self) -> BoxFuture<'static, Result<(), RawError>>;
}

/// An open engine connection (HV-CONN-01).
pub trait RawConn: Send + 'static {
    fn kind(&self) -> DbmsKind;
    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>>;
    /// One round trip; returns its duration.
    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>>;
    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary>;
    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>>;
    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>>;
    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>>;
    fn close(self: Box<Self>) -> BoxFuture<'static, ()>;
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>>;
    // ## m2b-t01-interfaces
    /// Runs one user statement (HV-EXEC-05).
    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>>;
    /// Runs trusted control text (guard plans, environment), with the simple protocol.
    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>>;
    /// The transaction status after the last statement.
    fn tx_status(&self) -> TxStatus;
    /// Returns the connection to a clean session state: PostgreSQL the `DISCARD ALL` sequence
    /// minus `DEALLOCATE ALL` (Q10), MySQL `COM_RESET_CONNECTION`, SQLite nothing.
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>>;
    /// The server's ID of this connection: the PostgreSQL backend PID (from `BackendKeyData`,
    /// Q11), the MySQL thread ID.
    fn backend_id(&self) -> Option<u64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_connect_debug_redacts_the_password() {
        let req = DriverConnect {
            endpoint: Endpoint::File {
                path: PathBuf::from("db.sqlite"),
                create_if_missing: false,
                read_only: true,
            },
            tls: None,
            user: Some("deploy".into()),
            password: Some(SecretString::from("s3cret")),
            database: None,
            application_name: "Hacivat".into(),
            read_only: true,
            options: vec![],
            connect_timeout: Duration::from_secs(15),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        };
        let s = format!("{req:?}");
        assert!(!s.contains("s3cret"), "{s}");
        assert!(s.contains("<redacted>") && s.contains("deploy"), "{s}");
    }
}
