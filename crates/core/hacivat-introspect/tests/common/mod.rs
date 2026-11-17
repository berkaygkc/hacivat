//! Shared helpers of the Docker tests: connections to the `docker/m2a` PostgreSQL targets
//! and a uniquely named schema per test, dropped after the test (also when it panics).

#![allow(dead_code, reason = "each test binary uses a subset")]

use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use hacivat_driver::{DriverConnect, Endpoint, ExecOpts, InsecureAuth, RawConn};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct};
use hacivat_testkit::DockerTarget;
use secrecy::SecretString;

/// The PostgreSQL versions the introspector supports (spec §6 M4; PG 13 kept for the spike).
pub const PG_IDS: [&str; 6] = ["pg13", "pg14", "pg15", "pg16", "pg17", "pg18"];

pub async fn connect(t: &DockerTarget) -> PgConn {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let factory = direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        vec![addr],
        Duration::from_secs(5),
    );
    let settings = TlsSettings {
        mode: TlsMode::Disable,
        ..TlsSettings::default()
    };
    let req = DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: t.host.clone(),
        },
        tls: build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None).unwrap(),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat (metadata)".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(10),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    };
    connect_conn(req)
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

/// A schema name no other test run uses.
pub fn unique_name(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}_{}_{}", std::process::id(), nanos % 1_000_000_000)
}

pub async fn run(c: &mut PgConn, sql: &str) {
    c.batch(sql)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
}

pub async fn scalar(c: &mut PgConn, sql: &str) -> String {
    let s = c.execute(sql, ExecOpts::values(1)).await.unwrap();
    s.rows[0][0].clone().unwrap()
}

pub async fn schema_oid(c: &mut PgConn, name: &str) -> u32 {
    scalar(
        c,
        &format!("SELECT oid FROM pg_catalog.pg_namespace WHERE nspname = '{name}'"),
    )
    .await
    .parse()
    .unwrap()
}

/// Runs `body`, then `cleanup` (also when `body` panics), then re-raises the panic.
pub async fn guarded<B, C>(body: B, cleanup: C)
where
    B: Future<Output = ()>,
    C: Future<Output = ()>,
{
    let out = AssertUnwindSafe(body).catch_unwind().await;
    cleanup.await;
    if let Err(p) = out {
        std::panic::resume_unwind(p);
    }
}
