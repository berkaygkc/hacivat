//! `FakeDriver`: a scripted in-process driver for pipeline, manager and UI tests (F4).
//! Working since Task 1 (feature `fake`).
//!
//! Each `connect` counts the call, records the request (password included, as a plain string,
//! for asserts; its `Debug` redacts it), waits the configured delay, then returns the next
//! scripted result or the default one. A successful connect reports `TlsStarted`/`TlsDone`
//! (with TLS params) and `AuthStarted`/`AuthDone` to the request's event callback. A gated
//! driver (`gated`) holds every connect until `release`, so a test can observe a connect in
//! flight without racing a delay.
//!
//! M2b (Task 1, Q23): connections run statements from a script (`exec`). The first unconsumed
//! [`FakeExec`] whose prefix matches is used once; an unmatched statement succeeds with an
//! empty summary and leaves the transaction status unchanged. `executed` records `execute` and
//! `batch` text in order. A cancel (through `cancel_handle`) ends the running statement with
//! `RawErrorKind::Cancelled` unless the driver `ignores_cancel`. `kill_connections` makes every
//! live connection fail with `Io(ConnectionReset)`.
//!
//! M2b (Task 7, T1 review M10): a statement counts as running from the `execute`/`batch` call,
//! so a cancel sent before its future is first polled is latched and still ends it; a cancel
//! sent while nothing runs is a no-op (as on a real server) and never reaches a later statement.
//!
//! M2b (Task 17 fix round 1, review I4): a connection that holds its endpoint's stream
//! (`consumes_factory`) fails its ping with `Io(ConnectionReset)` once that stream has ended,
//! so a test sees an SSH channel die with its tunnel, as a real driver would.
//!
//! M2b (finding F3): `execute` with [`Protocol::Extended`] answers a scripted result without
//! its row values (the row count stays), as the PostgreSQL and MySQL drivers do. A statement
//! whose values the caller reads must run with `ExecOpts::values`, or its script is not seen.

use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::{BoxedStream, PresentedChain, Target, TlsSettings, TlsSummary, TlsVerified};
use hacivat_proto::DbmsKind;
use secrecy::ExposeSecret;
use tokio::sync::Notify;

use crate::driver::{
    CancelHandle, CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor,
    DriverEvent, Endpoint, ExecOpts, ExecSummary, InsecureAuth, ProbeWarning, Protocol, RawConn,
    TxStatus,
};
use crate::errors::{RawError, RawErrorKind};
use crate::server::ServerInfo;

// ## m2b-t01-interfaces
/// One scripted statement (Q23): used once, by the first statement starting with `prefix`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FakeExec {
    pub prefix: &'static str,
    pub result: Result<ExecSummary, RawError>,
    /// How long the statement runs (a cancel ends it early).
    pub delay: Duration,
    /// The connection's transaction status after the statement.
    pub tx_after: Option<TxStatus>,
    /// The statement panics (an actor's panic handling).
    pub panics: bool,
}

/// The statement script and log, shared by the driver and its connections.
#[derive(Default)]
struct ExecState {
    script: Mutex<Vec<Option<FakeExec>>>,
    executed: Mutex<Vec<String>>,
}

impl ExecState {
    /// Logs `sql` and takes the first unconsumed entry whose prefix matches it.
    fn take(&self, sql: &str) -> Option<FakeExec> {
        lock(&self.executed).push(sql.to_string());
        let text = sql.trim_start();
        lock(&self.script)
            .iter_mut()
            .find(|e| e.as_ref().is_some_and(|e| text.starts_with(e.prefix)))
            .and_then(Option::take)
    }
}

/// A [`DriverConnect`] endpoint without its stream factory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FakeEndpoint {
    Network {
        direct: Option<Target>,
        resolved: Vec<SocketAddr>,
        tls_host: String,
    },
    File {
        path: PathBuf,
        create_if_missing: bool,
        read_only: bool,
    },
}

/// The TLS part of a [`DriverConnect`] without the rustls configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FakeTls {
    pub settings: TlsSettings,
    pub server_name: String,
    pub plaintext_allowed: bool,
}

/// What the last `connect` received: a [`DriverConnect`] without the factory and the event
/// callback, with the password as a plain string for asserts. `Debug` prints
/// `password: <redacted>` (P33).
#[derive(Clone, Default)]
pub struct FakeRequest {
    pub endpoint: Option<FakeEndpoint>,
    pub tls: Option<FakeTls>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub database: Option<String>,
    pub application_name: String,
    pub read_only: bool,
    pub options: Vec<(String, String)>,
    pub connect_timeout: Duration,
    // ## m2b-t01-interfaces
    pub insecure_auth: InsecureAuth,
}

impl fmt::Debug for FakeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeRequest")
            .field("endpoint", &self.endpoint)
            .field("tls", &self.tls)
            .field("user", &self.user)
            .field(
                "password",
                &format_args!(
                    "{}",
                    if self.password.is_some() {
                        "<redacted>"
                    } else {
                        "None"
                    }
                ),
            )
            .field("database", &self.database)
            .field("application_name", &self.application_name)
            .field("read_only", &self.read_only)
            .field("options", &self.options)
            .field("connect_timeout", &self.connect_timeout)
            .field("insecure_auth", &self.insecure_auth)
            .finish()
    }
}

impl FakeRequest {
    fn of(req: &DriverConnect) -> Self {
        let endpoint = match &req.endpoint {
            Endpoint::Network {
                direct,
                resolved,
                tls_host,
                ..
            } => FakeEndpoint::Network {
                direct: direct.clone(),
                resolved: resolved.clone(),
                tls_host: tls_host.clone(),
            },
            Endpoint::File {
                path,
                create_if_missing,
                read_only,
            } => FakeEndpoint::File {
                path: path.clone(),
                create_if_missing: *create_if_missing,
                read_only: *read_only,
            },
        };
        FakeRequest {
            endpoint: Some(endpoint),
            tls: req.tls.as_ref().map(|t| FakeTls {
                settings: t.settings.clone(),
                server_name: t.server_name.to_str().into_owned(),
                plaintext_allowed: t.plaintext_allowed,
            }),
            user: req.user.clone(),
            password: req.password.as_ref().map(|p| p.expose_secret().to_string()),
            database: req.database.clone(),
            application_name: req.application_name.clone(),
            read_only: req.read_only,
            options: req.options.clone(),
            connect_timeout: req.connect_timeout,
            insecure_auth: req.insecure_auth,
        }
    }
}

#[derive(Default)]
struct Counters {
    connect: AtomicUsize,
    close: AtomicUsize,
    /// Connection objects alive: made for a connect, not yet dropped (closed or abandoned,
    /// e.g. with a pending connect when its runtime stops). M2a Task 18.
    alive: AtomicUsize,
    // ## m2b-t01-interfaces
    cancel: AtomicUsize,
    reset: AtomicUsize,
    /// Bumped by `kill_connections`: a connection made before the bump is dead.
    killed: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ## m2b-t17-liveness
/// The statements that set a fake connection's `autocommit` flag: a prefix and the value.
type AutocommitEffects = Arc<Vec<(&'static str, bool)>>;

/// A scripted driver (F4). Build it, register an `Arc` clone, and read its counters.
pub struct FakeDriver {
    descriptor: DriverDescriptor,
    server: ServerInfo,
    result: Result<(), RawError>,
    script: Mutex<VecDeque<Result<(), RawError>>>,
    delay: Duration,
    /// `Some` for a gated driver: connects wait until it reads `true` (`release`).
    gate: Option<tokio::sync::watch::Sender<bool>>,
    pings: Vec<Result<Duration, RawError>>,
    tls: TlsSummary,
    databases: Vec<String>,
    counters: Arc<Counters>,
    last: Mutex<Option<FakeRequest>>,
    // ## m2b-t01-interfaces
    exec: Arc<ExecState>,
    requests: Mutex<Vec<FakeRequest>>,
    backend: Option<u64>,
    presented: Option<Vec<Vec<u8>>>,
    tls_outcome: Option<TlsVerified>,
    ignores_cancel: bool,
    reset_fails: bool,
    consumes_factory: bool,
    // ## m2b-t17-liveness
    /// `Some`: connections report `autocommit` (from `true`), and these statements set it.
    autocommit: Option<AutocommitEffects>,
}

impl FakeDriver {
    /// A driver for `kind` that connects at once to a server reporting `kind`'s product name,
    /// without TLS.
    pub fn new(kind: DbmsKind) -> Self {
        FakeDriver {
            descriptor: DriverDescriptor {
                kind,
                crate_name: "hacivat-driver-fake",
                caps: DriverCaps {
                    stream_injection: true,
                    in_protocol_tls: kind != DbmsKind::Sqlite,
                    cancel: CancelMechanism::None,
                    file_based: kind == DbmsKind::Sqlite,
                },
            },
            server: ServerInfo {
                kind,
                product: kind.display_name().to_string(),
                version: None,
                raw_version: String::new(),
                is_replica: Some(false),
            },
            result: Ok(()),
            script: Mutex::new(VecDeque::new()),
            delay: Duration::ZERO,
            gate: None,
            pings: Vec::new(),
            tls: TlsSummary {
                encrypted: false,
                version: None,
                cipher: None,
                verified: TlsVerified::No,
            },
            databases: Vec::new(),
            counters: Arc::default(),
            last: Mutex::new(None),
            exec: Arc::default(),
            requests: Mutex::new(Vec::new()),
            backend: None,
            presented: None,
            tls_outcome: None,
            ignores_cancel: false,
            reset_fails: false,
            consumes_factory: false,
            autocommit: None,
        }
    }

    /// What a connection's `server_info` reports.
    pub fn server(mut self, info: ServerInfo) -> Self {
        self.server = info;
        self
    }

    /// The result of every connect without a scripted one.
    pub fn connect_result(mut self, result: Result<(), RawError>) -> Self {
        self.result = result;
        self
    }

    /// Results for the next connects, in order; then `connect_result` applies.
    pub fn script(self, results: Vec<Result<(), RawError>>) -> Self {
        *lock(&self.script) = results.into();
        self
    }

    /// How long each connect takes.
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Holds every connect (after it is counted and recorded, before its delay and result)
    /// until [`release`](Self::release): a test sees the connect in flight for as long as it
    /// needs, however slow the machine, instead of racing a delay.
    pub fn gated(mut self) -> Self {
        self.gate = Some(tokio::sync::watch::Sender::new(false));
        self
    }

    /// Lets every held connect (and every later one) go on. Nothing for an ungated driver.
    pub fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.send_replace(true);
        }
    }

    /// Ping results per connection, in order; then 1 ms.
    pub fn pings(mut self, pings: Vec<Result<Duration, RawError>>) -> Self {
        self.pings = pings;
        self
    }

    pub fn tls(mut self, summary: TlsSummary) -> Self {
        self.tls = summary;
        self
    }

    pub fn databases(mut self, names: Vec<String>) -> Self {
        self.databases = names;
        self
    }

    pub fn connect_calls(&self) -> usize {
        self.counters.connect.load(Ordering::SeqCst)
    }

    pub fn close_calls(&self) -> usize {
        self.counters.close.load(Ordering::SeqCst)
    }

    /// Connections opened (or being opened) and not yet dropped: closed connections and
    /// abandoned connects (their future dropped) both count as gone (M2a Task 18).
    pub fn open_connections(&self) -> usize {
        self.counters.alive.load(Ordering::SeqCst)
    }

    pub fn last_request(&self) -> Option<FakeRequest> {
        lock(&self.last).clone()
    }

    // ## m2b-t01-interfaces

    /// The statement script (see the module docs).
    pub fn exec(self, script: Vec<FakeExec>) -> Self {
        *lock(&self.exec.script) = script.into_iter().map(Some).collect();
        self
    }

    /// What every connection's `backend_id` reports.
    pub fn backend(mut self, id: u64) -> Self {
        self.backend = Some(id);
        self
    }

    /// The chain (DER, leaf first) written into `TlsParams.presented` when a connect with TLS
    /// fails with a TLS error (M2b Task 14: or with `InsecureAuthRefused`, after its handshake).
    pub fn presented(mut self, der: Vec<Vec<u8>>) -> Self {
        self.presented = Some(der);
        self
    }

    /// The outcome written into `TlsParams.outcome` when a connect with TLS succeeds (M2b Task
    /// 14: or fails with `InsecureAuthRefused`).
    pub fn tls_outcome(mut self, v: TlsVerified) -> Self {
        self.tls_outcome = Some(v);
        self
    }

    /// The text of every `execute` and `batch`, in order.
    pub fn executed(&self) -> Vec<String> {
        lock(&self.exec.executed).clone()
    }

    /// A cancel no longer ends the running statement.
    pub fn ignores_cancel(mut self) -> Self {
        self.ignores_cancel = true;
        self
    }

    /// Every `reset` fails.
    pub fn reset_fails(mut self) -> Self {
        self.reset_fails = true;
        self
    }

    // ## m2b-t17-liveness
    /// Connections report `RawConn::autocommit` as MySQL's status flags would: on at connect,
    /// then set by every statement starting with one of `effects`' prefixes, whatever its
    /// result (a batch's `SET` applies even when a later statement fails).
    pub fn autocommit_flag(mut self, effects: Vec<(&'static str, bool)>) -> Self {
        self.autocommit = Some(Arc::new(effects));
        self
    }

    /// Each connect opens the network endpoint's stream (as the PostgreSQL driver does) and
    /// fails with its I/O error. The connection holds the stream: once it ends (an SSH channel
    /// whose tunnel died), every ping fails with `Io(ConnectionReset)`.
    pub fn consumes_factory(mut self) -> Self {
        self.consumes_factory = true;
        self
    }

    /// `first` for the next connects, then `then` for every other one.
    pub fn script_then(self, first: Vec<Result<(), RawError>>, then: Result<(), RawError>) -> Self {
        self.script(first).connect_result(then)
    }

    /// Every request, in order.
    pub fn requests(&self) -> Vec<FakeRequest> {
        lock(&self.requests).clone()
    }

    pub fn cancel_calls(&self) -> usize {
        self.counters.cancel.load(Ordering::SeqCst)
    }

    pub fn reset_calls(&self) -> usize {
        self.counters.reset.load(Ordering::SeqCst)
    }

    /// Every live connection fails from now on with `Io(ConnectionReset)`; later connections
    /// are fine.
    pub fn kill_connections(&self) {
        self.counters.killed.fetch_add(1, Ordering::SeqCst);
    }
}

impl Driver for FakeDriver {
    fn descriptor(&self) -> &DriverDescriptor {
        &self.descriptor
    }

    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        // Recorded before it is counted: a test that saw the count also sees the request.
        let recorded = FakeRequest::of(&req);
        lock(&self.requests).push(recorded.clone());
        *lock(&self.last) = Some(recorded);
        self.counters.connect.fetch_add(1, Ordering::SeqCst);
        let scripted = lock(&self.script).pop_front();
        let result = match scripted {
            Some(result) => result,
            None => self.result.clone(),
        };
        let mut conn = FakeConn {
            kind: self.descriptor.kind,
            server: self.server.clone(),
            pings: self.pings.clone().into(),
            tls: self.tls.clone(),
            databases: self.databases.clone(),
            user: req.user.clone(),
            counters: self.counters.clone(),
            exec: self.exec.clone(),
            tx: TxStatus::Idle,
            born: self.counters.killed.load(Ordering::SeqCst),
            cancel: Arc::default(),
            ignores_cancel: self.ignores_cancel,
            reset_fails: self.reset_fails,
            backend: self.backend,
            stream: None,
            autocommit: self
                .autocommit
                .as_ref()
                .map(|effects| (true, effects.clone())),
        };
        self.counters.alive.fetch_add(1, Ordering::SeqCst);
        let delay = self.delay;
        let gate = self
            .gate
            .as_ref()
            .map(tokio::sync::watch::Sender::subscribe);
        let with_tls = req.tls.is_some();
        let tls_slots = req
            .tls
            .as_ref()
            .map(|t| (t.presented.clone(), t.outcome.clone()));
        let (presented, outcome) = (self.presented.clone(), self.tls_outcome);
        let factory = match &req.endpoint {
            Endpoint::Network { factory, .. } if self.consumes_factory => Some(factory.clone()),
            _ => None,
        };
        let events = req.events.clone();
        drop(req);
        Box::pin(async move {
            if let Some(mut gate) = gate {
                // The driver (the sender) outlives its connects in tests; if it is gone, go on.
                let _ = gate.wait_for(|open| *open).await;
            }
            if let Some(factory) = factory {
                let stream = factory.open().await.map_err(|e| RawError {
                    kind: RawErrorKind::Io(e.kind()),
                    server: None,
                })?;
                conn.stream = Some(stream);
            }
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            // ## m2b-t14-pipeline: an insecure-auth refusal comes after a handshake that
            // completed (Q20), so its chain and outcome are recorded too.
            let refused_after_tls = matches!(
                &result,
                Err(RawError {
                    kind: RawErrorKind::InsecureAuthRefused,
                    ..
                })
            );
            if let (Err(e), Some((slot, _)), Some(der)) = (&result, &tls_slots, presented)
                && (matches!(e.kind, RawErrorKind::Tls(_)) || refused_after_tls)
            {
                *lock(slot) = Some(PresentedChain::from_der(der));
            }
            if let (true, Some((_, slot)), Some(v)) = (refused_after_tls, &tls_slots, outcome) {
                *lock(slot) = Some(v);
            }
            result?;
            if let (Some((_, slot)), Some(v)) = (&tls_slots, outcome) {
                *lock(slot) = Some(v);
            }
            if with_tls {
                events(DriverEvent::TlsStarted);
                events(DriverEvent::TlsDone);
            }
            events(DriverEvent::AuthStarted);
            events(DriverEvent::AuthDone);
            Ok(Box::new(conn) as Box<dyn RawConn>)
        })
    }
}

/// A connection of [`FakeDriver`].
struct FakeConn {
    kind: DbmsKind,
    server: ServerInfo,
    pings: VecDeque<Result<Duration, RawError>>,
    tls: TlsSummary,
    databases: Vec<String>,
    user: Option<String>,
    counters: Arc<Counters>,
    // ## m2b-t01-interfaces
    exec: Arc<ExecState>,
    tx: TxStatus,
    /// `Counters::killed` when the connection was made.
    born: u64,
    cancel: Arc<CancelLatch>,
    ignores_cancel: bool,
    reset_fails: bool,
    backend: Option<u64>,
    /// The endpoint's stream, held open (`consumes_factory`).
    stream: Option<BoxedStream>,
    // ## m2b-t17-liveness
    /// What `autocommit` reports, and the statements that change it (`autocommit_flag`).
    autocommit: Option<(bool, AutocommitEffects)>,
}

impl FakeConn {
    // ## m2b-t17-liveness
    /// `Io(ConnectionReset)` once the held endpoint stream has ended (EOF or an error); a
    /// stream with nothing to read is alive. Polled once, without waiting.
    fn stream_alive(&mut self) -> Result<(), RawError> {
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        use tokio::io::ReadBuf;
        let Some(stream) = self.stream.as_mut() else {
            return Ok(());
        };
        let mut byte = [0u8; 1];
        let mut buf = ReadBuf::new(&mut byte);
        let mut cx = Context::from_waker(Waker::noop());
        match Pin::new(&mut **stream).poll_read(&mut cx, &mut buf) {
            Poll::Ready(Ok(())) if buf.filled().is_empty() => Err(RawError {
                kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
                server: None,
            }),
            Poll::Ready(Err(e)) => Err(RawError {
                kind: RawErrorKind::Io(e.kind()),
                server: None,
            }),
            _ => Ok(()),
        }
    }

    /// `Io(ConnectionReset)` once `kill_connections` ran after this connection was made.
    fn alive(&self) -> Result<(), RawError> {
        if self.counters.killed.load(Ordering::SeqCst) > self.born {
            return Err(RawError {
                kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionReset),
                server: None,
            });
        }
        Ok(())
    }

    /// Runs one statement from the script; it counts as running from this call (M10).
    fn run<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        self.cancel.begin();
        // ## m2b-t17-liveness: the server applies the setting whatever the statement's (or
        // the batch's) outcome.
        if let Some((on, effects)) = self.autocommit.as_mut() {
            let text = sql.trim_start();
            if let Some((_, set)) = effects.iter().find(|(prefix, _)| text.starts_with(prefix)) {
                *on = *set;
            }
        }
        Box::pin(async move {
            let result = self.run_entry(sql).await;
            self.cancel.running.store(false, Ordering::SeqCst);
            result
        })
    }

    async fn run_entry(&mut self, sql: &str) -> Result<ExecSummary, RawError> {
        let entry = self.exec.take(sql);
        self.alive()?;
        let Some(entry) = entry else {
            return Ok(ExecSummary {
                tx: self.tx,
                ..ExecSummary::default()
            });
        };
        assert!(
            !entry.panics,
            "FakeExec `{}` panics (scripted)",
            entry.prefix
        );
        if !entry.delay.is_zero() {
            let cancelled = self.cancel.notify.notified();
            tokio::pin!(cancelled);
            cancelled.as_mut().enable();
            let ignores = self.ignores_cancel;
            // A cancel that came before this first poll (M10).
            if !ignores && self.cancel.latched.swap(false, Ordering::SeqCst) {
                return Err(RawError {
                    kind: RawErrorKind::Cancelled,
                    server: None,
                });
            }
            tokio::select! {
                () = tokio::time::sleep(entry.delay) => {}
                () = &mut cancelled, if !ignores => {
                    return Err(RawError { kind: RawErrorKind::Cancelled, server: None });
                }
            }
            self.alive()?;
        }
        if let Some(tx) = entry.tx_after {
            self.tx = tx;
        }
        let tx = self.tx;
        entry.result.map(|summary| ExecSummary { tx, ..summary })
    }
}

// ## m2b-f3-internal-reads
/// A scripted answer as `opts` returns it: [`Protocol::Extended`] is the count-only path, so
/// its rows are counted and their values dropped, whatever the script says (a value read takes
/// `ExecOpts::values`).
fn count_only(summary: ExecSummary, opts: ExecOpts) -> ExecSummary {
    match opts.protocol {
        Protocol::Extended => ExecSummary {
            rows: Vec::new(),
            truncated: false,
            ..summary
        },
        Protocol::Simple => summary,
    }
}

/// A [`FakeConn`]'s cancel state (M10): whether a statement runs, and a cancel that reached it
/// before it was first polled.
#[derive(Default)]
struct CancelLatch {
    notify: Notify,
    running: AtomicBool,
    latched: AtomicBool,
}

impl CancelLatch {
    /// A statement starts: it runs, and no earlier cancel applies to it.
    fn begin(&self) {
        self.latched.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);
    }
}

/// Cancels the statement running on one [`FakeConn`].
struct FakeCancel {
    latch: Arc<CancelLatch>,
    counters: Arc<Counters>,
}

impl CancelHandle for FakeCancel {
    fn cancel(&self) -> BoxFuture<'static, Result<(), RawError>> {
        self.counters.cancel.fetch_add(1, Ordering::SeqCst);
        if self.latch.running.load(Ordering::SeqCst) {
            self.latch.latched.store(true, Ordering::SeqCst);
        }
        self.latch.notify.notify_waiters();
        Box::pin(async { Ok(()) })
    }
}

impl Drop for FakeConn {
    fn drop(&mut self) {
        self.counters.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RawConn for FakeConn {
    fn kind(&self) -> DbmsKind {
        self.kind
    }

    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        let info = self.server.clone();
        Box::pin(async move { Ok(info) })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        let next = self
            .alive()
            .and_then(|()| self.stream_alive())
            .and_then(|()| {
                self.pings
                    .pop_front()
                    .unwrap_or(Ok(Duration::from_millis(1)))
            });
        Box::pin(async move { next })
    }

    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        let tls = self.tls.clone();
        Box::pin(async move { tls })
    }

    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        Box::pin(async { Vec::new() })
    }

    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        let user = self.user.clone();
        Box::pin(async move { Ok(user) })
    }

    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        let names = self.databases.iter().take(limit).cloned().collect();
        Box::pin(async move { Ok(names) })
    }

    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.counters.close.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {})
    }

    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        Some(Arc::new(FakeCancel {
            latch: self.cancel.clone(),
            counters: self.counters.clone(),
        }))
    }

    // ## m2b-t01-interfaces

    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        let run = self.run(sql);
        // ## m2b-f3-internal-reads: the count-only path keeps no values, as the PostgreSQL
        // and MySQL drivers run it, so a script can never answer a value read sent there.
        Box::pin(async move { run.await.map(|summary| count_only(summary, opts)) })
    }

    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        let run = self.run(sql);
        Box::pin(async move { run.await.map(|_| ()) })
    }

    fn tx_status(&self) -> TxStatus {
        self.tx
    }

    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        self.counters.reset.fetch_add(1, Ordering::SeqCst);
        let result = match self.alive() {
            Ok(()) if self.reset_fails => Err(RawError {
                kind: RawErrorKind::Other,
                server: None,
            }),
            alive => alive,
        };
        if result.is_ok() {
            self.tx = TxStatus::Idle;
        }
        Box::pin(async move { result })
    }

    fn backend_id(&self) -> Option<u64> {
        self.backend
    }

    // ## m2b-t17-liveness
    fn autocommit(&self) -> Option<bool> {
        self.autocommit.as_ref().map(|(on, _)| *on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RawErrorKind;
    use secrecy::SecretString;

    fn request(password: Option<&str>, events: crate::driver::DriverEvents) -> DriverConnect {
        DriverConnect {
            endpoint: Endpoint::File {
                path: PathBuf::from("shop.db"),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: Some("deploy".into()),
            password: password.map(SecretString::from),
            database: Some("shop".into()),
            application_name: "Hacivat".into(),
            read_only: false,
            options: vec![("k".into(), "v".into())],
            connect_timeout: Duration::from_secs(15),
            events,
            insecure_auth: InsecureAuth::default(),
        }
    }

    #[tokio::test]
    async fn scripted_results_then_the_default_and_counters() {
        let refused = RawError {
            kind: RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused),
            server: None,
        };
        let fake = Arc::new(
            FakeDriver::new(DbmsKind::Postgres)
                .script(vec![Err(refused.clone())])
                .databases(vec!["a".into(), "b".into(), "c".into()])
                .pings(vec![Ok(Duration::from_millis(7))]),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let events: crate::driver::DriverEvents = Arc::new(move |e| lock(&sink).push(e));

        let first = fake
            .connect(request(Some("hv_test_pw"), events.clone()))
            .await;
        assert_eq!(first.err(), Some(refused));
        assert!(lock(&seen).is_empty());

        let mut conn = fake
            .connect(request(None, events))
            .await
            .expect("the default result is Ok");
        assert_eq!(
            *lock(&seen),
            [DriverEvent::AuthStarted, DriverEvent::AuthDone]
        );
        assert_eq!(conn.kind(), DbmsKind::Postgres);
        assert_eq!(conn.server_info().await.unwrap().product, "PostgreSQL");
        assert_eq!(conn.ping().await.unwrap(), Duration::from_millis(7));
        assert_eq!(conn.ping().await.unwrap(), Duration::from_millis(1));
        assert_eq!(conn.list_databases(2).await.unwrap(), ["a", "b"]);
        assert_eq!(
            conn.current_user().await.unwrap().as_deref(),
            Some("deploy")
        );
        assert!(!conn.tls_summary().await.encrypted);
        conn.close().await;
        assert_eq!((fake.connect_calls(), fake.close_calls()), (2, 1));
    }

    #[tokio::test]
    async fn a_gated_connect_waits_for_the_release() {
        use futures::FutureExt as _;
        let fake = FakeDriver::new(DbmsKind::Postgres).gated();
        let mut held = fake.connect(request(None, Arc::new(|_| {})));
        assert_eq!(fake.connect_calls(), 1, "counted before the gate");
        for _ in 0..10 {
            assert!((&mut held).now_or_never().is_none(), "held at the gate");
            tokio::task::yield_now().await;
        }
        fake.release();
        assert!(held.await.is_ok());
        // Released for good: a later connect goes straight through.
        assert!(fake.connect(request(None, Arc::new(|_| {}))).await.is_ok());
        assert_eq!(fake.open_connections(), 0, "both connections dropped");
    }

    #[tokio::test]
    async fn the_last_request_keeps_the_password_for_asserts_but_never_prints_it() {
        let fake = FakeDriver::new(DbmsKind::Sqlite);
        let _ = fake
            .connect(request(Some("s3cret"), Arc::new(|_| {})))
            .await;
        let last = fake.last_request().expect("a request was recorded");
        assert_eq!(last.password.as_deref(), Some("s3cret"));
        assert_eq!(last.database.as_deref(), Some("shop"));
        assert_eq!(
            last.endpoint,
            Some(FakeEndpoint::File {
                path: PathBuf::from("shop.db"),
                create_if_missing: true,
                read_only: false
            })
        );
        let printed = format!("{last:?}");
        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("password: <redacted>"), "{printed}");
    }

    // ## m2b-t01-interfaces

    fn slow(prefix: &'static str, delay: Duration) -> FakeExec {
        FakeExec {
            prefix,
            result: Ok(ExecSummary::default()),
            delay,
            tx_after: None,
            panics: false,
        }
    }

    #[tokio::test]
    async fn a_cancel_ends_the_running_statement_unless_ignored() {
        // An hour: only the cancel can end it within the test.
        let fake = FakeDriver::new(DbmsKind::Postgres)
            .exec(vec![slow("SELECT pg_sleep", Duration::from_secs(3600))])
            .backend(4242);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        assert_eq!(conn.backend_id(), Some(4242));
        let cancel = conn.cancel_handle().expect("a cancel handle");
        let (result, ()) = tokio::join!(
            conn.execute("SELECT pg_sleep(60)", ExecOpts::default()),
            async {
                tokio::task::yield_now().await;
                cancel.cancel().await.unwrap();
            }
        );
        assert_eq!(result.unwrap_err().kind, RawErrorKind::Cancelled);
        assert_eq!(fake.cancel_calls(), 1);

        let stubborn = FakeDriver::new(DbmsKind::Postgres)
            .exec(vec![slow("SELECT", Duration::from_millis(20))])
            .ignores_cancel();
        let mut conn = stubborn
            .connect(request(None, Arc::new(|_| {})))
            .await
            .unwrap();
        let cancel = conn.cancel_handle().unwrap();
        let (result, ()) = tokio::join!(conn.execute("SELECT 1", ExecOpts::default()), async {
            tokio::task::yield_now().await;
            cancel.cancel().await.unwrap();
        });
        assert!(result.is_ok(), "the statement ran to its end");
    }

    #[tokio::test]
    async fn killed_connections_fail_and_reset_is_counted() {
        let fake = FakeDriver::new(DbmsKind::MySql).exec(vec![FakeExec {
            prefix: "START TRANSACTION",
            result: Ok(ExecSummary::default()),
            delay: Duration::ZERO,
            tx_after: Some(TxStatus::InTx),
            panics: false,
        }]);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        conn.batch("START TRANSACTION READ ONLY").await.unwrap();
        assert_eq!(conn.tx_status(), TxStatus::InTx);
        conn.reset().await.unwrap();
        assert_eq!((conn.tx_status(), fake.reset_calls()), (TxStatus::Idle, 1));

        fake.kill_connections();
        let reset = RawErrorKind::Io(std::io::ErrorKind::ConnectionReset);
        assert_eq!(
            conn.execute("SELECT 1", ExecOpts::default())
                .await
                .unwrap_err()
                .kind,
            reset
        );
        assert_eq!(conn.ping().await.unwrap_err().kind, reset);
        let mut fresh = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        assert!(fresh.execute("SELECT 1", ExecOpts::default()).await.is_ok());
        assert_eq!(
            fake.executed(),
            ["START TRANSACTION READ ONLY", "SELECT 1", "SELECT 1"]
        );
        assert_eq!(fake.requests().len(), 2);
        assert_eq!(
            fake.requests()[0].insecure_auth,
            InsecureAuth::RefuseUnlessVerified
        );

        let failing = FakeDriver::new(DbmsKind::MySql).reset_fails();
        let mut conn = failing
            .connect(request(None, Arc::new(|_| {})))
            .await
            .unwrap();
        assert!(conn.reset().await.is_err());
    }

    // ## m2b-t17-liveness
    #[tokio::test]
    async fn a_held_stream_that_ends_fails_the_ping() {
        let far = Arc::new(Mutex::new(None));
        let keep = far.clone();
        let factory = hacivat_net::StreamFactory::new(move || {
            let keep = keep.clone();
            Box::pin(async move {
                let (near, far) = tokio::io::duplex(64);
                *lock(&keep) = Some(far);
                Ok(Box::new(near) as BoxedStream)
            })
        });
        let fake = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
        let mut req = request(None, Arc::new(|_| {}));
        req.endpoint = Endpoint::Network {
            factory,
            direct: None,
            resolved: Vec::new(),
            tls_host: "db".into(),
        };
        let mut conn = fake.connect(req).await.unwrap();
        assert!(conn.ping().await.is_ok(), "an open, quiet stream is alive");
        drop(lock(&far).take());
        assert_eq!(
            conn.ping().await.unwrap_err().kind,
            RawErrorKind::Io(std::io::ErrorKind::ConnectionReset)
        );
    }

    #[tokio::test]
    async fn a_factory_consumer_fails_with_the_streams_error() {
        let refused = hacivat_net::StreamFactory::new(|| {
            Box::pin(async { Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)) })
        });
        let fake = FakeDriver::new(DbmsKind::Postgres).consumes_factory();
        let mut req = request(None, Arc::new(|_| {}));
        req.endpoint = Endpoint::Network {
            factory: refused,
            direct: None,
            resolved: Vec::new(),
            tls_host: "db".into(),
        };
        let err = fake
            .connect(req)
            .await
            .err()
            .expect("the stream was refused");
        assert_eq!(
            err.kind,
            RawErrorKind::Io(std::io::ErrorKind::ConnectionRefused)
        );
    }

    // ## m2b-t07-session-actor

    /// T1 review M10: a cancel sent after `execute` but before its first poll still ends the
    /// statement; a cancel sent while nothing runs never reaches the next statement.
    #[tokio::test]
    async fn an_early_cancel_is_latched_and_an_idle_cancel_is_a_no_op() {
        let fake = FakeDriver::new(DbmsKind::Postgres).exec(vec![
            slow("SELECT pg_sleep", Duration::from_secs(3600)),
            // Short: it runs to its end unless the idle cancel wrongly reaches it.
            slow("SELECT later", Duration::from_millis(20)),
        ]);
        let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
        let cancel = conn.cancel_handle().unwrap();
        let early = conn.execute("SELECT pg_sleep(60)", ExecOpts::default());
        cancel.cancel().await.unwrap();
        assert_eq!(early.await.unwrap_err().kind, RawErrorKind::Cancelled);

        // Nothing runs now: this cancel is lost, as on a real server.
        cancel.cancel().await.unwrap();
        assert!(
            conn.execute("SELECT later", ExecOpts::default())
                .await
                .is_ok(),
            "the idle cancel did not reach the next statement"
        );
        assert_eq!(fake.cancel_calls(), 2);
    }

    // ## m2b-f3-internal-reads

    /// Finding F3: a scripted answer reaches a value read only through the values path; the
    /// count-only path counts its rows and drops their values, as the real drivers do, so a
    /// probe sent there cannot pass over the fake and fail on a server.
    #[tokio::test]
    async fn the_count_only_path_keeps_no_values() {
        let answer = || FakeExec {
            prefix: "SELECT current_user = session_user",
            result: Ok(ExecSummary {
                rows: vec![vec![Some("t".into())]],
                row_count: 1,
                ..ExecSummary::default()
            }),
            delay: Duration::ZERO,
            tx_after: None,
            panics: false,
        };
        for kind in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::Sqlite] {
            let fake = FakeDriver::new(kind).exec(vec![answer(), answer()]);
            let mut conn = fake.connect(request(None, Arc::new(|_| {}))).await.unwrap();
            let counted = conn
                .execute("SELECT current_user = session_user", ExecOpts::default())
                .await
                .unwrap();
            assert_eq!((counted.row_count, counted.rows.len()), (1, 0), "{kind:?}");
            let read = conn
                .execute("SELECT current_user = session_user", ExecOpts::values(1))
                .await
                .unwrap();
            assert_eq!(read.rows, [vec![Some("t".to_string())]], "{kind:?}");
        }
    }
}
