//! `FakeDriver`: a scripted in-process driver for pipeline, manager and UI tests (F4).
//! Working since Task 1 (feature `fake`).
//!
//! Each `connect` counts the call, records the request (password included, as a plain string,
//! for asserts; its `Debug` redacts it), waits the configured delay, then returns the next
//! scripted result or the default one. A successful connect reports `TlsStarted`/`TlsDone`
//! (with TLS params) and `AuthStarted`/`AuthDone` to the request's event callback.

use std::collections::VecDeque;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::{Target, TlsSettings, TlsSummary, TlsVerified};
use hacivat_proto::DbmsKind;
use secrecy::ExposeSecret;

use crate::driver::{
    CancelHandle, CancelMechanism, Driver, DriverCaps, DriverConnect, DriverDescriptor,
    DriverEvent, Endpoint, ProbeWarning, RawConn,
};
use crate::errors::RawError;
use crate::server::ServerInfo;

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
        }
    }
}

#[derive(Default)]
struct Counters {
    connect: AtomicUsize,
    close: AtomicUsize,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A scripted driver (F4). Build it, register an `Arc` clone, and read its counters.
pub struct FakeDriver {
    descriptor: DriverDescriptor,
    server: ServerInfo,
    result: Result<(), RawError>,
    script: Mutex<VecDeque<Result<(), RawError>>>,
    delay: Duration,
    pings: Vec<Result<Duration, RawError>>,
    tls: TlsSummary,
    databases: Vec<String>,
    counters: Arc<Counters>,
    last: Mutex<Option<FakeRequest>>,
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

    pub fn last_request(&self) -> Option<FakeRequest> {
        lock(&self.last).clone()
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
        self.counters.connect.fetch_add(1, Ordering::SeqCst);
        *lock(&self.last) = Some(FakeRequest::of(&req));
        let scripted = lock(&self.script).pop_front();
        let result = match scripted {
            Some(result) => result,
            None => self.result.clone(),
        };
        let conn = FakeConn {
            kind: self.descriptor.kind,
            server: self.server.clone(),
            pings: self.pings.clone().into(),
            tls: self.tls.clone(),
            databases: self.databases.clone(),
            user: req.user.clone(),
            counters: self.counters.clone(),
        };
        let delay = self.delay;
        let with_tls = req.tls.is_some();
        let events = req.events.clone();
        drop(req);
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            result?;
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
            .pings
            .pop_front()
            .unwrap_or(Ok(Duration::from_millis(1)));
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
        None
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
}
