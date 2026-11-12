//! PostgreSQL driver against the Docker matrix (R17, Task 13). Every test is
//! `#[ignore = "docker"]` and also returns early unless `HACIVAT_DOCKER=1`, because
//! `cargo xtask docker test` runs every ignored test. Run with
//! `cargo xtask docker test --set full -p hacivat-driver-pg`.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, DriverEvent, DriverEvents, Endpoint, RawConn, RawError, RawErrorKind,
};
use hacivat_driver_pg::{PgConn, PgDriver, connect_conn};
use hacivat_net::{
    AddressClass, BoxedStream, StreamFactory, Target, TlsFailure, TlsMode, TlsSettings,
    TlsVerified, build_tls_params, direct,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, docker_enabled, target, targets};
use secrecy::SecretString;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Whether every named target must exist in the active set: under `full` (or with no set,
/// which means every target) a missing one is a failure, never a silent skip. A narrower set
/// (`pr`) legitimately leaves targets out.
fn must_exist(set: Option<&str>) -> bool {
    set.is_none_or(|s| s == "full")
}

fn active_set() -> Option<String> {
    std::env::var("HACIVAT_DOCKER_SET").ok()
}

/// The target `id`, or `None` when the active set leaves it out. Panics when the set should
/// contain it (`must_exist`).
fn target_required(id: &str) -> Option<DockerTarget> {
    let t = target(id);
    let set = active_set();
    assert!(
        t.is_some() || !must_exist(set.as_deref()),
        "Docker target {id} is missing from the {} set",
        set.as_deref().unwrap_or("full")
    );
    t
}

fn pg_targets(ids: &[&str]) -> Vec<DockerTarget> {
    let found: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| ids.contains(&t.id.as_str()))
        .collect();
    if must_exist(active_set().as_deref()) {
        for id in ids {
            assert!(
                found.iter().any(|t| t.id == *id),
                "Docker target {id} is missing from the full set"
            );
        }
    }
    found
}

#[test]
fn only_the_full_set_requires_every_target() {
    assert!(must_exist(Some("full")));
    assert!(must_exist(None));
    assert!(!must_exist(Some("pr")));
}

fn direct_factory(t: &DockerTarget) -> StreamFactory {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        vec![addr],
        Duration::from_secs(5),
    )
}

/// Counts every byte that crosses the injected stream, both ways.
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
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.count
            .fetch_add(buf.filled().len() - before, Ordering::SeqCst);
        r
    }
}

impl AsyncWrite for Counting {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        let r = Pin::new(&mut self.inner).poll_write(cx, data);
        if let Poll::Ready(Ok(n)) = &r {
            self.count.fetch_add(*n, Ordering::SeqCst);
        }
        r
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn counting(f: StreamFactory) -> (StreamFactory, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let wrapped = StreamFactory::new(move || {
        let open = f.open();
        let count = c.clone();
        Box::pin(async move {
            let inner = open.await?;
            Ok(Box::new(Counting { inner, count }) as BoxedStream)
        })
    });
    (wrapped, count)
}

/// A request with fresh TLS parameters (a new failure slot per attempt).
fn req_with(
    t: &DockerTarget,
    factory: StreamFactory,
    settings: TlsSettings,
    class: AddressClass,
) -> DriverConnect {
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: t.host.clone(),
        },
        tls: build_tls_params(&settings, &t.host, class).unwrap(),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(10),
        events: Arc::new(|_| {}),
    }
}

fn req(t: &DockerTarget, factory: StreamFactory, mode: TlsMode) -> DriverConnect {
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    req_with(t, factory, settings, AddressClass::Loopback)
}

fn req_pw(t: &DockerTarget, password: &str) -> DriverConnect {
    let mut r = req(t, direct_factory(t), TlsMode::Auto);
    r.password = Some(SecretString::from(password.to_string()));
    r
}

fn req_db(t: &DockerTarget, db: &str) -> DriverConnect {
    let mut r = req(t, direct_factory(t), TlsMode::Auto);
    r.database = Some(db.into());
    r
}

fn sqlstate(e: RawError) -> String {
    assert_eq!(e.kind, RawErrorKind::Db);
    e.server.unwrap().sqlstate.unwrap()
}

async fn connect_db(id: &str, db: &str) -> Box<dyn RawConn> {
    let t = target_required(id).unwrap();
    let r = req_db(&t, db);
    match PgDriver::new().connect(r).await {
        Ok(c) => c,
        Err(e) => panic!("{id}: {e:?}"),
    }
}

async fn connect_ro(id: &str) -> PgConn {
    let t = target_required(id).unwrap();
    let mut r = req(&t, direct_factory(&t), TlsMode::Auto);
    r.read_only = true;
    connect_conn(r)
        .await
        .unwrap_or_else(|e| panic!("{id}: {e:?}"))
}

async fn show(c: &mut PgConn, name: &str) -> String {
    let row = c
        .client()
        .query_one(&format!("SHOW {name}"), &[])
        .await
        .unwrap();
    row.get(0)
}

/// Docker tests run only through `cargo xtask docker test`, which sets `HACIVAT_DOCKER=1`.
macro_rules! docker_only {
    () => {
        if !docker_enabled() {
            return;
        }
    };
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_connects_over_injected_stream_counting_bytes() {
    docker_only!();
    let all = pg_targets(&["pg14", "pg15", "pg16", "pg17", "pg18"]);
    assert!(!all.is_empty());
    for t in all {
        let (factory, count) = counting(direct_factory(&t));
        let mut c = PgDriver::new()
            .connect(req(&t, factory, TlsMode::Auto))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        assert!(count.load(Ordering::SeqCst) > 0);
        let info = c.server_info().await.unwrap();
        assert_eq!(info.kind, DbmsKind::Postgres);
        assert_eq!(info.version.unwrap().major.to_string(), t.major);
        assert_eq!(info.is_replica, Some(false));
        assert!(
            info.raw_version.starts_with("PostgreSQL "),
            "{}",
            info.raw_version
        );
        assert!(!c.tls_summary().await.encrypted);
        let before = count.load(Ordering::SeqCst);
        c.ping().await.unwrap();
        assert!(
            count.load(Ordering::SeqCst) > before,
            "the ping crossed the stream"
        );
        c.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_tls_target_modes() {
    docker_only!();
    let Some(t) = target_required("pg17-tls") else {
        return;
    };
    let e = PgDriver::new()
        .connect(req(&t, direct_factory(&t), TlsMode::VerifyFull))
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::UnknownIssuer));
    let mut c = PgDriver::new()
        .connect(req(&t, direct_factory(&t), TlsMode::Require))
        .await
        .unwrap();
    let s = c.tls_summary().await;
    assert!(s.encrypted);
    assert_eq!(s.version.as_deref(), Some("TLSv1.3"));
    assert!(s.cipher.is_some());
    assert_eq!(s.verified, TlsVerified::No);
    c.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_tls_records_the_verifier_failure_per_attempt() {
    docker_only!();
    let Some(t) = target_required("pg17-tls") else {
        return;
    };
    // Auto toward a loopback host still verifies when the server offers TLS (HV-SEC-01).
    let r = req(&t, direct_factory(&t), TlsMode::Auto);
    let failure = r.tls.as_ref().unwrap().failure.clone();
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::Tls(TlsFailure::UnknownIssuer));
    assert_eq!(*failure.lock().unwrap(), Some(TlsFailure::UnknownIssuer));
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_auto_on_a_public_host_refuses_a_plaintext_server() {
    docker_only!();
    let Some(t) = target_required("pg17") else {
        return;
    };
    let settings = TlsSettings::default();
    let r = req_with(&t, direct_factory(&t), settings, AddressClass::Public);
    let e = PgDriver::new().connect(r).await.err().unwrap();
    assert_eq!(e.kind, RawErrorKind::TlsNotOffered);
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_server_errors_keep_sqlstate() {
    docker_only!();
    let Some(t) = target_required("pg17") else {
        return;
    };
    assert_eq!(
        sqlstate(
            PgDriver::new()
                .connect(req_pw(&t, "wrong"))
                .await
                .err()
                .unwrap()
        ),
        "28P01"
    );
    assert_eq!(
        sqlstate(
            PgDriver::new()
                .connect(req_db(&t, "orders_v2"))
                .await
                .err()
                .unwrap()
        ),
        "3D000"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_turkish_server_text_is_kept_verbatim_and_marked() {
    docker_only!();
    let Some(t) = target_required("pg17-tr") else {
        return;
    };
    let e = PgDriver::new()
        .connect(req_pw(&t, "wrong"))
        .await
        .err()
        .unwrap();
    let m = e.server.unwrap();
    assert_eq!(m.sqlstate.as_deref(), Some("28P01"));
    assert_ne!(m.severity.as_deref(), Some("FATAL"));
    assert_eq!(m.lang_hint.as_deref(), Some("und"));
    assert!(!m.text.contains("hv_test_pw"));
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_lists_databases_and_reads_user() {
    docker_only!();
    if target_required("pg17").is_none() {
        return;
    }
    let mut c = connect_db("pg17", "postgres").await;
    let dbs = c.list_databases(10).await.unwrap();
    assert!(
        dbs.contains(&"orders".into())
            && dbs.contains(&"orders_archive".into())
            && !dbs.contains(&"template0".into()),
        "{dbs:?}"
    );
    let mut sorted = dbs.clone();
    sorted.sort();
    assert_eq!(dbs, sorted);
    assert_eq!(c.list_databases(1).await.unwrap().len(), 1);
    assert_eq!(c.current_user().await.unwrap().as_deref(), Some("hv_test"));
    assert!(c.ping().await.unwrap() < Duration::from_secs(1));
    assert!(c.probe().await.is_empty());
    assert!(c.cancel_handle().is_none());
    assert_eq!(c.kind(), DbmsKind::Postgres);
    c.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn pg13_reports_major_13() {
    docker_only!();
    if target_required("pg13").is_none() {
        return;
    }
    assert_eq!(
        connect_db("pg13", "hv_test")
            .await
            .server_info()
            .await
            .unwrap()
            .version
            .unwrap()
            .major,
        13
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_read_only_profile_sets_the_session_default() {
    docker_only!();
    if target_required("pg17").is_none() {
        return;
    }
    let mut c = connect_ro("pg17").await;
    assert_eq!(show(&mut c, "default_transaction_read_only").await, "on");
    let e = c
        .client()
        .batch_execute("CREATE TABLE hv_t13_should_fail (id int)")
        .await
        .unwrap_err();
    assert_eq!(e.code().map(|c| c.code()), Some("25006"));
}

fn recorder() -> (DriverEvents, Arc<Mutex<Vec<DriverEvent>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let sink = seen.clone();
    (Arc::new(move |e| sink.lock().unwrap().push(e)), seen)
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_reports_tls_and_sign_in_events() {
    docker_only!();
    use DriverEvent::*;
    if let Some(t) = target_required("pg17-tls") {
        let mut r = req(&t, direct_factory(&t), TlsMode::Require);
        let (events, seen) = recorder();
        r.events = events;
        PgDriver::new().connect(r).await.unwrap().close().await;
        assert_eq!(
            *seen.lock().unwrap(),
            [TlsStarted, TlsDone, AuthStarted, AuthDone]
        );
    }
    if let Some(t) = target_required("pg17") {
        for mode in [TlsMode::Disable, TlsMode::Auto] {
            let mut r = req(&t, direct_factory(&t), mode);
            let (events, seen) = recorder();
            r.events = events;
            PgDriver::new().connect(r).await.unwrap().close().await;
            assert_eq!(*seen.lock().unwrap(), [AuthStarted, AuthDone], "{mode:?}");
        }
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_close_ends_the_server_session() {
    docker_only!();
    let Some(t) = target_required("pg17") else {
        return;
    };
    let name = format!("hv-t13-close-{}", std::process::id());
    let mut r = req(&t, direct_factory(&t), TlsMode::Auto);
    r.application_name = name.clone();
    let doomed = PgDriver::new().connect(r).await.unwrap();
    let watcher = connect_conn(req(&t, direct_factory(&t), TlsMode::Auto))
        .await
        .unwrap();
    let count = || async {
        let n: i64 = watcher
            .client()
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
                &[&name],
            )
            .await
            .unwrap()
            .get(0);
        n
    };
    assert_eq!(count().await, 1);
    doomed.close().await;
    let mut n = count().await;
    for _ in 0..50 {
        if n == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        n = count().await;
    }
    assert_eq!(n, 0);
}
