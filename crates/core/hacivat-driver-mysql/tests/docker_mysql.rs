//! MySQL and MariaDB against the Docker matrix (R17). Every test is `#[ignore = "docker"]` and
//! also returns early unless `HACIVAT_DOCKER=1`; run them with
//! `cargo xtask docker test --set full -p hacivat-driver-mysql`. Failures print the target id
//! and the error kind only, never a password.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, Endpoint, ProbeWarning, RawConn, RawError, RawErrorKind,
};
use hacivat_driver_mysql::{MySqlConn, MySqlDriver, connect_conn};
use hacivat_net::{
    BoxedStream, StreamFactory, Target, TlsFailure, TlsMode, TlsSettings, TlsVerified,
    build_tls_params, classify_host, direct, install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, docker_enabled, targets};
use secrecy::SecretString;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const TIMEOUT: Duration = Duration::from_secs(20);

fn target(id: &str) -> Option<DockerTarget> {
    hacivat_testkit::target(id)
}

fn targets_of(kinds: &[DbmsKind]) -> Vec<DockerTarget> {
    let ts: Vec<DockerTarget> = targets(None)
        .into_iter()
        .filter(|t| kinds.contains(&t.engine))
        .collect();
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

/// Require where the server's certificate is self-signed, else Auto (P12, as Task 19).
fn auto_or_require(t: &DockerTarget) -> TlsMode {
    match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    }
}

fn resolved(t: &DockerTarget) -> Vec<SocketAddr> {
    vec![SocketAddr::new(t.host.parse().unwrap(), t.port)]
}

fn direct_factory(t: &DockerTarget) -> StreamFactory {
    direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        resolved(t),
        TIMEOUT,
    )
}

/// Counts the bytes the driver reads through the factory's streams.
struct Counting {
    inner: BoxedStream,
    count: Arc<AtomicUsize>,
}

impl AsyncRead for Counting {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        let read = buf.filled().len() - before;
        self.count.fetch_add(read, Ordering::SeqCst);
        polled
    }
}

impl AsyncWrite for Counting {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn counting(factory: StreamFactory) -> (StreamFactory, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let total = count.clone();
    let wrapped = StreamFactory::new(move || {
        let open = factory.open();
        let count = total.clone();
        Box::pin(async move {
            let inner = open.await?;
            Ok(Box::new(Counting { inner, count }) as BoxedStream)
        })
    });
    (wrapped, count)
}

struct Spec<'a> {
    mode: TlsMode,
    ca: Option<&'a Path>,
    tls_host: Option<&'a str>,
    direct: bool,
    factory: Option<StreamFactory>,
    user: Option<&'a str>,
    password: Option<&'a str>,
    database: Option<&'a str>,
    read_only: bool,
    /// What the pipeline resolved the host to (the target's own address by default).
    resolved: Option<Vec<SocketAddr>>,
}

impl Spec<'_> {
    fn new(mode: TlsMode) -> Self {
        Spec {
            mode,
            ca: None,
            tls_host: None,
            direct: true,
            factory: None,
            user: None,
            password: None,
            database: None,
            read_only: false,
            resolved: None,
        }
    }
}

fn build(t: &DockerTarget, s: Spec<'_>) -> DriverConnect {
    install_crypto_provider();
    let tls_host = s.tls_host.unwrap_or(&t.host).to_string();
    let resolved = s.resolved.unwrap_or_else(|| resolved(t));
    let class = classify_host(&tls_host, &resolved);
    let settings = TlsSettings {
        mode: s.mode,
        ca_file: s.ca.map(Path::to_path_buf),
        client_cert: None,
        ..Default::default()
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: s.factory.unwrap_or_else(|| direct_factory(t)),
            direct: s.direct.then(|| Target::Tcp {
                host: t.host.clone(),
                port: t.port,
            }),
            resolved,
            tls_host: tls_host.clone(),
        },
        tls: build_tls_params(&settings, &tls_host, 3306, class, None).unwrap(),
        user: Some(s.user.unwrap_or(&t.user).to_string()),
        password: Some(SecretString::from(s.password.unwrap_or(&t.password))),
        database: Some(s.database.unwrap_or(&t.database).to_string()),
        application_name: "Hacivat tests".into(),
        read_only: s.read_only,
        options: vec![],
        connect_timeout: TIMEOUT,
        events: Arc::new(|_| {}),
        insecure_auth: Default::default(),
    }
}

fn req(t: &DockerTarget, mode: TlsMode) -> DriverConnect {
    build(t, Spec::new(mode))
}

fn req_injected(t: &DockerTarget, factory: StreamFactory) -> DriverConnect {
    build(
        t,
        Spec {
            direct: false,
            factory: Some(factory),
            ..Spec::new(auto_or_require(t))
        },
    )
}

fn req_ca(t: &DockerTarget, mode: TlsMode, ca: &Path) -> DriverConnect {
    build(
        t,
        Spec {
            ca: Some(ca),
            tls_host: Some("localhost"),
            ..Spec::new(mode)
        },
    )
}

fn req_pw(t: &DockerTarget, password: &str) -> DriverConnect {
    build(
        t,
        Spec {
            password: Some(password),
            ..Spec::new(auto_or_require(t))
        },
    )
}

fn req_db(t: &DockerTarget, database: &str) -> DriverConnect {
    build(
        t,
        Spec {
            database: Some(database),
            ..Spec::new(auto_or_require(t))
        },
    )
}

fn req_user(t: &DockerTarget, user: &str) -> DriverConnect {
    build(
        t,
        Spec {
            user: Some(user),
            ..Spec::new(auto_or_require(t))
        },
    )
}

fn vendor(e: RawError) -> i64 {
    e.server
        .and_then(|s| s.vendor_code)
        .unwrap_or_else(|| panic!("no server error: {:?}", e.kind))
}

async fn connect(r: DriverConnect) -> Result<Box<dyn RawConn>, RawError> {
    MySqlDriver::new().connect(r).await
}

async fn connect_ro(id: &str) -> MySqlConn {
    let t = target(id).unwrap();
    let r = build(
        &t,
        Spec {
            read_only: true,
            ..Spec::new(auto_or_require(&t))
        },
    );
    match connect_conn(r).await {
        Ok(c) => c,
        Err(e) => panic!("{id}: {:?}", e.kind),
    }
}

async fn select_i64(c: &mut MySqlConn, sql: &str) -> i64 {
    c.select_i64(sql).await.unwrap().unwrap()
}

#[tokio::test]
#[ignore = "docker"]
async fn every_mysql_family_target_connects_and_detects() {
    if !docker_enabled() {
        return;
    }
    for t in targets_of(&[DbmsKind::MySql, DbmsKind::MariaDb]) {
        let mut c = connect(req(&t, auto_or_require(&t)))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        let i = c.server_info().await.unwrap();
        assert_eq!(i.kind, t.engine, "{}", t.id);
        assert_eq!(c.kind(), t.engine, "{}", t.id);
        assert!(i.version.unwrap().raw.starts_with(&t.major), "{}", t.id);
        c.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_forwarded_stream_is_used() {
    if !docker_enabled() {
        return;
    }
    let t = target("mysql84").unwrap();
    let (factory, count) = counting(direct_factory(&t));
    let mut c = connect(req_injected(&t, factory)).await.unwrap();
    assert!(count.load(Ordering::SeqCst) > 0);
    // TLS runs end to end through the forwarder.
    assert!(c.tls_summary().await.encrypted);
    c.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_default_certificates_are_untrusted_and_require_encrypts_unverified() {
    if !docker_enabled() {
        return;
    }
    let t = target("mysql84").unwrap();
    let e = connect(req(&t, TlsMode::VerifyFull)).await.err().unwrap();
    assert!(
        matches!(
            e.kind,
            RawErrorKind::Tls(TlsFailure::UnknownIssuer | TlsFailure::NameMismatch)
        ),
        "{:?}",
        e.kind
    );
    let mut c = connect(req(&t, TlsMode::Require)).await.unwrap();
    let s = c.tls_summary().await;
    assert!(s.encrypted);
    assert!(s.version.as_deref().is_some_and(|v| v.starts_with("TLSv1")));
    assert!(s.cipher.is_some());
    assert_eq!(s.verified, TlsVerified::No);
    c.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_full_with_the_container_ca_still_checks_the_name() {
    if !docker_enabled() {
        return;
    }
    let t = target("mysql84").unwrap();
    let ca = hacivat_testkit::artifact("mysql84-ca.pem");
    let e = connect(req_ca(&t, TlsMode::VerifyFull, &ca))
        .await
        .err()
        .unwrap();
    // The auto-generated leaf's CN is not "localhost".
    assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::NameMismatch));
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_error_codes() {
    if !docker_enabled() {
        return;
    }
    let t = target("mysql84").unwrap();
    assert_eq!(
        vendor(connect(req_pw(&t, "wrong")).await.err().unwrap()),
        1045
    );
    // A missing database is ER_BAD_DB_ERROR (1049) only for an account with privileges on it;
    // `hv_test` has grants on three named databases, so the server hides whether `orders_v2`
    // exists and answers ER_DBACCESS_DENIED_ERROR (1044).
    assert_eq!(
        vendor(connect(req_db(&t, "orders_v2")).await.err().unwrap()),
        1044
    );
    assert_eq!(
        vendor(connect(req_user(&t, "hv_locked")).await.err().unwrap()),
        3118
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_read_only_session() {
    if !docker_enabled() {
        return;
    }
    let mut c = connect_ro("mysql84").await;
    assert_eq!(
        select_i64(&mut c, "SELECT @@session.transaction_read_only").await,
        1
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn auto_tls_follows_the_target_default() {
    if !docker_enabled() {
        return;
    }
    for t in targets_of(&[DbmsKind::MySql, DbmsKind::MariaDb]) {
        let r = connect(req(&t, TlsMode::Auto)).await;
        match t.tls_default {
            TlsDefault::None => {
                let mut c = r.unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
                assert!(!c.tls_summary().await.encrypted, "{}", t.id);
            }
            TlsDefault::SelfSigned => assert!(
                matches!(r.err().unwrap().kind, RawErrorKind::Tls(_)),
                "{}",
                t.id
            ),
        }
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn require_on_a_server_without_tls_is_not_offered() {
    if !docker_enabled() {
        return;
    }
    for t in targets_of(&[DbmsKind::MySql, DbmsKind::MariaDb]) {
        if t.tls_default != TlsDefault::None {
            continue;
        }
        let e = connect(req(&t, TlsMode::Require)).await.err().unwrap();
        assert_eq!(e.kind, RawErrorKind::TlsNotOffered, "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn every_target_lists_databases_reads_the_user_and_probes() {
    if !docker_enabled() {
        return;
    }
    for t in targets_of(&[DbmsKind::MySql, DbmsKind::MariaDb]) {
        let mut c = connect(req(&t, auto_or_require(&t)))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        let dbs = c.list_databases(10).await.unwrap();
        assert!(dbs.contains(&"orders".to_string()), "{}", t.id);
        assert!(dbs.contains(&"orders_archive".to_string()), "{}", t.id);
        assert!(!dbs.iter().any(|d| d == "mysql" || d == "sys"), "{}", t.id);
        assert_eq!(c.list_databases(1).await.unwrap().len(), 1, "{}", t.id);
        assert_eq!(
            c.current_user().await.unwrap().as_deref(),
            Some("hv_test"),
            "{}",
            t.id
        );
        assert!(c.ping().await.unwrap() < Duration::from_secs(1), "{}", t.id);
        let warnings = c.probe().await;
        assert!(
            !warnings.contains(&ProbeWarning::ServerReadOnly),
            "{}",
            t.id
        );
        if t.engine == DbmsKind::MariaDb {
            // MariaDB has no `super_read_only`.
            assert!(!warnings.contains(&ProbeWarning::SuperReadOnly), "{}", t.id);
        } else {
            // MySQL 8 defaults: writable, `local_infile` off, an 8-hour wait timeout.
            assert_eq!(warnings, Vec::<ProbeWarning>::new(), "{}", t.id);
        }
        c.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn auto_falls_back_to_plaintext_only_toward_loopback() {
    if !docker_enabled() {
        return;
    }
    for t in targets_of(&[DbmsKind::MySql, DbmsKind::MariaDb]) {
        if t.tls_default != TlsDefault::None {
            continue;
        }
        // Loopback: Auto signs in without TLS.
        let mut c = connect(req(&t, TlsMode::Auto))
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        assert!(!c.tls_summary().await.encrypted, "{}", t.id);
        c.close().await;
        // The same server reached as a private-network host (through the forwarder, as a
        // tunnel would): Auto refuses the plaintext sign-in, and opens only one stream.
        let inner = direct_factory(&t);
        let opens = Arc::new(AtomicUsize::new(0));
        let seen = opens.clone();
        let counted = StreamFactory::new(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            inner.open()
        });
        let r = build(
            &t,
            Spec {
                direct: false,
                factory: Some(counted),
                tls_host: Some("db.internal"),
                resolved: Some(vec!["10.0.0.5:3306".parse().unwrap()]),
                ..Spec::new(TlsMode::Auto)
            },
        );
        let e = connect(r).await.err().unwrap();
        assert_eq!(e.kind, RawErrorKind::TlsNotOffered, "{}", t.id);
        assert_eq!(opens.load(Ordering::SeqCst), 1, "{}", t.id);
    }
}
