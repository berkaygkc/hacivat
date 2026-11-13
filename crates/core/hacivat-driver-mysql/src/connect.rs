//! Connecting through `mysql_async` `Opts`, directly or via the loopback forwarder (R18).
//!
//! - Direct TCP (`direct: Some(Target::Tcp)`): `mysql_async` dials the TLS host itself, with
//!   the pipeline's resolved addresses (P10).
//! - Anything else (a transport wrap, P13; a Unix socket target): a one-shot TCP forwarder on
//!   `127.0.0.1:0` relays to the endpoint's factory, and TLS verifies the real host name through
//!   `with_danger_tls_hostname_override`. Never a Unix socket: `mysql_async` sends no TLS over
//!   one.
//!
//! Every connection sets `prefer_socket(false)`, so `mysql_async` never reconnects through
//! `@@socket` (which would bypass the forwarder and TLS), installs no `LOCAL INFILE` handler and
//! keeps the cleartext plugin off. The profile's driver options are never read: none of the
//! catalog's (JDBC-named) options has a `mysql_async` meaning, and `mysql_async`'s own names
//! (`enable_cleartext_plugin`, `socket`, ...) must never come from user input.

use std::io;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::{
    CancelHandle, DriverConnect, DriverEvent, Endpoint, ProbeWarning, RawConn, RawError,
    RawErrorKind, ServerInfo, ServerMessage,
};
use hacivat_net::{
    AddressClass, CleanupGuard, ForwarderAddr, ForwarderMode, LoopbackForwarder, StreamFactory,
    Target, TlsFailure, TlsMode, TlsSummary, classify_host,
};
use hacivat_proto::DbmsKind;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, SslOpts};
use secrecy::ExposeSecret;

use crate::probe;
use crate::tls::{full_ssl_opts, read_ca};

/// Runs on the session at connect when the profile is read-only (HV-CONN-07).
const READ_ONLY_SESSION: &str = "SET SESSION TRANSACTION READ ONLY";

/// Where the forwarder listens: the TCP mode binds `127.0.0.1:0` and creates nothing here.
fn forwarder_dir() -> std::path::PathBuf {
    std::env::temp_dir()
}

/// Where `mysql_async` dials, and the name TLS verifies when it differs.
struct Dial {
    host: String,
    port: u16,
    resolved: Option<Vec<IpAddr>>,
    host_override: Option<String>,
}

/// The name to dial and verify: brackets around an IPv6 literal and a trailing dot removed.
fn tls_name(host: &str) -> &str {
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    host.strip_suffix('.').unwrap_or(host)
}

fn tls_host(req: &DriverConnect) -> &str {
    match &req.endpoint {
        Endpoint::Network { tls_host, .. } => tls_name(tls_host),
        Endpoint::File { .. } => "",
    }
}

/// The Direct dial: only for a TCP target, which `mysql_async` reaches itself.
fn direct_dial(req: &DriverConnect) -> Option<Dial> {
    let Endpoint::Network {
        direct: Some(Target::Tcp { port, .. }),
        resolved,
        ..
    } = &req.endpoint
    else {
        return None;
    };
    Some(Dial {
        host: tls_host(req).to_owned(),
        port: *port,
        resolved: (!resolved.is_empty()).then(|| resolved.iter().map(|a| a.ip()).collect()),
        host_override: None,
    })
}

fn forwarded_dial(req: &DriverConnect, port: u16) -> Dial {
    Dial {
        host: "127.0.0.1".to_owned(),
        port,
        resolved: None,
        host_override: Some(tls_host(req).to_owned()),
    }
}

/// The `Opts` for one attempt. The password is copied out of its `SecretString` here, at the
/// last moment; the caller hands the value straight to `Conn::new` (P49).
fn build_opts(req: &DriverConnect, dial: Dial, ssl: Option<SslOpts>) -> Opts {
    let setup: Vec<&str> = if req.read_only {
        vec![READ_ONLY_SESSION]
    } else {
        Vec::new()
    };
    OptsBuilder::default()
        .ip_or_hostname(dial.host)
        .tcp_port(dial.port)
        .resolved_ips(dial.resolved)
        .user(req.user.clone())
        .pass(req.password.as_ref().map(|p| p.expose_secret().to_owned()))
        .db_name(req.database.clone())
        .prefer_socket(false)
        .socket(None::<String>)
        .enable_cleartext_plugin(false)
        .secure_auth(true)
        .ssl_opts(ssl)
        .setup(setup)
        .connect_attribute("program_name", req.application_name.clone())
        .into()
}

fn ssl_for(req: &DriverConnect, dial: &Dial, ca: Vec<Vec<u8>>) -> Option<SslOpts> {
    let tls = req.tls.as_ref()?;
    full_ssl_opts(tls, ca, dial.host_override.as_deref())
}

/// The `Opts` the driver would connect with: the Direct ones for a TCP target, else the
/// forwarded ones with port 0. A test seam: never `Debug`-format the result (P49).
#[doc(hidden)]
pub fn opts_for(req: &DriverConnect) -> Opts {
    match direct_dial(req) {
        Some(dial) => {
            let ssl = ssl_for(
                req,
                &dial,
                req.tls.as_ref().map(read_ca).unwrap_or_default(),
            );
            build_opts(req, dial, ssl)
        }
        None => opts_for_forwarded(req, 0),
    }
}

/// The `Opts` for a connect through a forwarder listening on `127.0.0.1:port`. A test seam:
/// never `Debug`-format the result (P49).
#[doc(hidden)]
pub fn opts_for_forwarded(req: &DriverConnect, port: u16) -> Opts {
    let dial = forwarded_dial(req, port);
    let ssl = ssl_for(
        req,
        &dial,
        req.tls.as_ref().map(read_ca).unwrap_or_default(),
    );
    build_opts(req, dial, ssl)
}

fn err(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

/// A `mysql_async` error as a raw error, by type and code only, never by message text
/// (HV-CONN-03). The server's own message is kept verbatim in `server`.
#[doc(hidden)]
pub fn raw_error(e: &mysql_async::Error) -> RawError {
    use mysql_async::{DriverError as D, Error as E, IoError, TlsError};
    let kind = match e {
        E::Server(s) => {
            return RawError {
                kind: RawErrorKind::Db,
                server: Some(ServerMessage {
                    sqlstate: (!s.state.is_empty()).then(|| s.state.clone()),
                    vendor_code: Some(i64::from(s.code)),
                    severity: None,
                    text: s.message.clone(),
                    detail: None,
                    hint: None,
                    lang_hint: None,
                }),
            };
        }
        E::Io(IoError::Io(io)) => io_kind(io),
        E::Io(IoError::Tls(TlsError::Tls(r))) => RawErrorKind::Tls(TlsFailure::from_rustls(r)),
        E::Io(IoError::Tls(_)) => RawErrorKind::Tls(TlsFailure::Other),
        E::Driver(d) => match d {
            D::NoClientSslFlagFromServer => RawErrorKind::TlsNotOffered,
            D::ConnectionClosed => RawErrorKind::Io(io::ErrorKind::UnexpectedEof),
            D::CleartextPluginDisabled
            | D::MysqlOldPasswordDisabled
            | D::UnknownAuthPlugin { .. }
            | D::NamedPipesDisabled => RawErrorKind::Unsupported,
            D::NoKeyFound => RawErrorKind::Tls(TlsFailure::Other),
            _ => RawErrorKind::Protocol,
        },
        E::Url(_) | E::Other(_) => RawErrorKind::Other,
    };
    err(kind)
}

/// An I/O error; a rustls error that tokio-rustls wrapped into it is a TLS failure.
fn io_kind(e: &io::Error) -> RawErrorKind {
    if let Some(r) = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return RawErrorKind::Tls(TlsFailure::from_rustls(r));
    }
    match e.kind() {
        io::ErrorKind::TimedOut => RawErrorKind::Timeout,
        kind => RawErrorKind::Io(kind),
    }
}

/// Whether Auto may sign in without TLS after the server offered none: only toward a
/// loopback address, and only where `hacivat-net` allows plaintext at all.
///
/// This is stricter than `TlsParams::plaintext_allowed` (Auto toward any non-public address).
/// Without TLS, `caching_sha2_password` "perform full authentication" makes `mysql_async`
/// request the server's RSA public key and encrypt the password with whatever key comes back;
/// it has no setting to refuse that. An active attacker on a private network can strip
/// `CLIENT_SSL` from the greeting, send its own key and recover the password. Loopback has no
/// such attacker in the path. An explicit Disable remains the user's choice (with a warning).
#[doc(hidden)]
pub fn plaintext_fallback_allowed(req: &DriverConnect) -> bool {
    let Endpoint::Network {
        resolved, tls_host, ..
    } = &req.endpoint
    else {
        return false;
    };
    req.tls.as_ref().is_some_and(|t| {
        t.settings.mode == TlsMode::Auto
            && t.plaintext_allowed
            && classify_host(tls_host, resolved) == AddressClass::Loopback
    })
}

/// Server errors sent instead of the greeting, before TLS or sign-in begin:
/// `ER_CON_COUNT_ERROR`, `ER_HOST_IS_BLOCKED`, `ER_HOST_NOT_PRIVILEGED`.
const PRE_GREETING_ERRORS: [i64; 3] = [1040, 1129, 1130];

/// The trail events for one finished `mysql_async` connect attempt. TLS and sign-in run
/// inside one call (`Conn::new`), so the events are derived from how far it got: after a
/// success all four (or the two sign-in events without TLS); after a TLS failure only
/// `TlsStarted`; after a sign-in error the TLS pair (when TLS was on) and `AuthStarted`;
/// after a transport error, a pre-greeting refusal or `TlsNotOffered`, nothing.
#[doc(hidden)]
pub fn trail_events(tls: bool, outcome: Result<(), &RawError>) -> Vec<DriverEvent> {
    use DriverEvent::*;
    let tls_pair: &[DriverEvent] = if tls { &[TlsStarted, TlsDone] } else { &[] };
    match outcome {
        Ok(()) => [tls_pair, &[AuthStarted, AuthDone]].concat(),
        Err(e) => match &e.kind {
            RawErrorKind::Tls(_) if tls => vec![TlsStarted],
            RawErrorKind::Db
                if e.server
                    .as_ref()
                    .and_then(|s| s.vendor_code)
                    .is_some_and(|c| PRE_GREETING_ERRORS.contains(&c)) =>
            {
                Vec::new()
            }
            RawErrorKind::Db | RawErrorKind::Unsupported => [tls_pair, &[AuthStarted]].concat(),
            _ => Vec::new(),
        },
    }
}

/// [`MySqlDriver::connect`](crate::MySqlDriver) without the box, so a test can reach
/// [`MySqlConn::select_i64`].
#[doc(hidden)]
pub async fn connect_conn(req: DriverConnect) -> Result<MySqlConn, RawError> {
    connect(req).await
}

/// Connects for `req` within its connect timeout.
pub(crate) async fn connect(req: DriverConnect) -> Result<MySqlConn, RawError> {
    let mode = req
        .tls
        .as_ref()
        .map_or(TlsMode::Disable, |t| t.settings.mode);
    if mode == TlsMode::VerifyCa {
        // P11: `mysql_async` cannot skip only the name check with the locked rustls.
        return Err(err(RawErrorKind::Unsupported));
    }
    if matches!(req.endpoint, Endpoint::File { .. }) {
        return Err(err(RawErrorKind::Unsupported));
    }
    match tokio::time::timeout(req.connect_timeout, connect_in_time(&req, mode)).await {
        Ok(result) => result,
        Err(_) => Err(err(RawErrorKind::Timeout)),
    }
}

async fn connect_in_time(req: &DriverConnect, mode: TlsMode) -> Result<MySqlConn, RawError> {
    let tls_on = mode != TlsMode::Disable;
    let mut ca = Vec::new();
    if tls_on {
        // The OS store is read once, on the blocking pool. A panic there is an internal error,
        // never retried on an async worker.
        tokio::task::spawn_blocking(crate::tls::os_roots)
            .await
            .map_err(|_| err(RawErrorKind::Other))?;
        // The CA certificates the pipeline read and checked; never the file again (M6).
        if let Some(tls) = req.tls.as_ref() {
            ca = read_ca(tls);
        }
    }
    let report = |tls: bool, outcome: Result<(), &RawError>| {
        for e in trail_events(tls, outcome) {
            (req.events)(e);
        }
    };
    let first = attempt(req, tls_on, ca).await;
    report(tls_on, first.as_ref().map(|_| ()));
    let (conn, guard, tls_used) = match first {
        Ok((conn, guard)) => (conn, guard, tls_on),
        // Auto: the server offered no TLS. Plaintext only toward loopback (see
        // `plaintext_fallback_allowed`); elsewhere `TlsNotOffered` (P9). `DriverEvent` has no
        // "continuing without TLS" event: the trail shows no TLS step for the fallback.
        Err(e)
            if mode == TlsMode::Auto
                && e.kind == RawErrorKind::TlsNotOffered
                && plaintext_fallback_allowed(req) =>
        {
            let second = attempt(req, false, Vec::new()).await;
            report(false, second.as_ref().map(|_| ()));
            let (conn, guard) = second?;
            (conn, guard, false)
        }
        Err(e) => return Err(e),
    };
    let kind = if conn.server_version().0 >= 10 {
        DbmsKind::MariaDb
    } else {
        DbmsKind::MySql
    };
    Ok(MySqlConn {
        conn: Some(conn),
        guard,
        kind,
        tls_mode: if tls_used { mode } else { TlsMode::Disable },
    })
}

/// One connect attempt, Direct or through a fresh forwarder (it accepts one connection).
async fn attempt(
    req: &DriverConnect,
    tls: bool,
    ca: Vec<Vec<u8>>,
) -> Result<(Conn, Option<CleanupGuard>), RawError> {
    if let Some(dial) = direct_dial(req) {
        let ssl = if tls { ssl_for(req, &dial, ca) } else { None };
        let opts = build_opts(req, dial, ssl);
        return Conn::new(opts)
            .await
            .map(|c| (c, None))
            .map_err(|e| raw_error(&e));
    }
    let Endpoint::Network { factory, .. } = &req.endpoint else {
        return Err(err(RawErrorKind::Unsupported));
    };
    let (factory, open_error) = recording(factory.clone());
    let (fwd, guard) = LoopbackForwarder::bind(factory, &forwarder_dir(), 1, ForwarderMode::Tcp)
        .await
        .map_err(|e| err(io_kind(&e)))?;
    let ForwarderAddr::Tcp(addr) = fwd.addr() else {
        guard.run().await;
        return Err(err(RawErrorKind::Other));
    };
    let dial = forwarded_dial(req, addr.port());
    let ssl = if tls { ssl_for(req, &dial, ca) } else { None };
    let opts = build_opts(req, dial, ssl);
    match Conn::new(opts).await {
        Ok(conn) => Ok((conn, Some(guard))),
        Err(e) => {
            guard.run().await;
            // The forwarder could not reach the endpoint: report that, not the dropped relay.
            let opened = *open_error.lock().unwrap_or_else(PoisonError::into_inner);
            Err(match opened {
                Some(kind) => err(io_kind(&kind.into())),
                None => raw_error(&e),
            })
        }
    }
}

/// Wraps `factory` so the forwarder's failed open is remembered: without it the driver would
/// only see the relay closing.
fn recording(factory: StreamFactory) -> (StreamFactory, Arc<Mutex<Option<io::ErrorKind>>>) {
    let slot = Arc::new(Mutex::new(None));
    let seen = slot.clone();
    let wrapped = StreamFactory::new(move || {
        let open = factory.open();
        let seen = seen.clone();
        Box::pin(async move {
            let opened = open.await;
            if let Err(e) = &opened {
                *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(e.kind());
            }
            opened
        })
    });
    (wrapped, slot)
}

/// An open MySQL or MariaDB connection (HV-CONN-01). It owns the forwarder's guard, which
/// `close` runs after the connection says goodbye; dropping the connection drops the guard,
/// which stops the forwarder too. Public only as a test seam ([`connect_conn`]).
#[doc(hidden)]
pub struct MySqlConn {
    /// `None` only after `close` took it.
    conn: Option<Conn>,
    guard: Option<CleanupGuard>,
    kind: DbmsKind,
    /// The mode TLS ran in; `Disable` when the connection is plaintext.
    tls_mode: TlsMode,
}

impl MySqlConn {
    fn conn(&mut self) -> Result<&mut Conn, RawError> {
        self.conn
            .as_mut()
            .ok_or_else(|| err(RawErrorKind::Io(io::ErrorKind::NotConnected)))
    }

    /// The first column of the first row of `sql` as an integer. A test seam: the Docker
    /// tests read session state with it.
    #[doc(hidden)]
    pub async fn select_i64(&mut self, sql: &str) -> Result<Option<i64>, RawError> {
        let row = self
            .conn()?
            .query_first::<mysql_async::Row, _>(sql)
            .await
            .map_err(|e| raw_error(&e))?;
        Ok(row.and_then(|r| probe::int(r.as_ref(0))))
    }
}

impl RawConn for MySqlConn {
    fn kind(&self) -> DbmsKind {
        self.kind
    }

    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        Box::pin(async move {
            let vars = probe::server_vars(self.conn()?).await?;
            let info = probe::server_info(&vars);
            self.kind = info.kind;
            Ok(info)
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        Box::pin(async move {
            let conn = self.conn()?;
            let start = Instant::now();
            conn.ping().await.map_err(|e| raw_error(&e))?;
            Ok(start.elapsed())
        })
    }

    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        Box::pin(async move {
            let mode = self.tls_mode;
            match self.conn() {
                Ok(conn) => probe::tls_summary(conn, mode).await,
                Err(_) => probe::summary(mode, None, None, None),
            }
        })
    }

    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        Box::pin(async move {
            let Ok(conn) = self.conn() else {
                return Vec::new();
            };
            match probe::server_vars(conn).await {
                Ok(v) => probe::probe_warnings(
                    v.read_only,
                    v.super_read_only,
                    v.wait_timeout,
                    v.local_infile,
                ),
                Err(_) => Vec::new(),
            }
        })
    }

    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        Box::pin(async move { probe::current_user(self.conn()?).await })
    }

    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        Box::pin(async move { probe::list_databases(self.conn()?, limit).await })
    }

    fn close(mut self: Box<Self>) -> BoxFuture<'static, ()> {
        let conn = self.conn.take();
        let guard = self.guard.take();
        Box::pin(async move {
            if let Some(conn) = conn {
                let _ = conn.disconnect().await;
            }
            if let Some(guard) = guard {
                guard.run().await;
            }
        })
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        // M2b: `KILL QUERY <id>` on a second connection through the same factory.
        None
    }

    // ## m2b-t01-interfaces: Task 13 (`exec`) fills these; until then nothing runs.

    fn execute<'a>(
        &'a mut self,
        _sql: &'a str,
        _opts: hacivat_driver::ExecOpts,
    ) -> BoxFuture<'a, Result<hacivat_driver::ExecSummary, RawError>> {
        Box::pin(async { Err(crate::exec::not_yet()) })
    }

    fn batch<'a>(&'a mut self, _sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        Box::pin(async { Err(crate::exec::not_yet()) })
    }

    fn tx_status(&self) -> hacivat_driver::TxStatus {
        hacivat_driver::TxStatus::Idle
    }

    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        Box::pin(async { Err(crate::exec::not_yet()) })
    }

    fn backend_id(&self) -> Option<u64> {
        None
    }
}
