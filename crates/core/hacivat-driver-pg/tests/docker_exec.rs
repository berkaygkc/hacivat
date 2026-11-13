//! PostgreSQL execution, transaction status, cancel, reset and the insecure-auth guard against
//! the Docker matrix (Task 10; B2, B3, B8, B9, B23, Q10, Q11, Q12). Every test is
//! `#[ignore = "docker"]` and takes its target only from `require_target(..)` (Q6). Run with
//! `cargo xtask docker test --set pr -p hacivat-driver-pg` (`pg17-tls` and `pg17-auth` run in
//! the `full` set).

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, RawConn, RawError, RawErrorKind,
    TxStatus,
};
use hacivat_driver_pg::{ChannelBindingMode, PgConn, connect_conn, connect_conn_with};
use hacivat_net::{AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct};
use hacivat_testkit::{DockerTarget, TlsDefault, require_target};
use secrecy::SecretString;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn simple() -> ExecOpts {
    ExecOpts {
        max_rows: 1000,
        protocol: Protocol::Simple,
    }
}

/// A connect request to `t` as `user`. A self-signed TLS target runs `Require` (outcome `No`),
/// any other `Disable`, so no TLS outcome can vouch for an insecure method.
fn request(t: &DockerTarget, user: &str, policy: InsecureAuth) -> DriverConnect {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let factory = direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        vec![addr],
        secs(5),
    );
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Disable,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory,
            direct: None,
            resolved: vec![],
            tls_host: t.host.clone(),
        },
        tls: build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None).unwrap(),
        user: Some(user.to_string()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat".into(),
        read_only: false,
        options: vec![],
        connect_timeout: secs(10),
        events: Arc::new(|_| {}),
        insecure_auth: policy,
    }
}

/// The test user on `t`; loopback, so any method is allowed, as the pipeline does (B8).
async fn pg_on(t: &DockerTarget) -> PgConn {
    connect_conn(request(t, &t.user, InsecureAuth::Allow))
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

async fn pg_as(t: &DockerTarget, user: &str, policy: InsecureAuth) -> Result<PgConn, RawError> {
    connect_conn(request(t, user, policy)).await
}

async fn scalar(c: &mut PgConn, sql: &str) -> String {
    let s = c.execute(sql, simple()).await.unwrap();
    s.rows[0][0].clone().unwrap()
}

#[tokio::test]
#[ignore = "docker"]
async fn execute_simple_and_extended() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    let s = c
        .execute("SELECT 1 AS a, 'ş' AS b", simple())
        .await
        .unwrap();
    assert_eq!(s.columns, ["a", "b"]);
    assert_eq!(s.rows, [vec![Some("1".into()), Some("ş".into())]]);
    let e = c
        .execute("SELECT generate_series(1, 5)", ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(e.row_count, 5);
    assert!(e.rows.is_empty());
    assert_eq!(e.rows_affected, Some(5));
    assert_eq!(e.command_tag.as_deref(), Some("SELECT 5"));
    // max_rows keeps the first rows and counts the rest.
    let s = c
        .execute(
            "SELECT generate_series(1, 5)",
            ExecOpts {
                max_rows: 2,
                protocol: Protocol::Simple,
            },
        )
        .await
        .unwrap();
    assert_eq!((s.rows.len(), s.row_count, s.truncated), (2, 5, true));
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn command_tags_come_from_the_server() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch("CREATE TEMP TABLE hv_t10_tag(a int)")
        .await
        .unwrap();
    let s = c
        .execute(
            "INSERT INTO hv_t10_tag SELECT generate_series(1, 3)",
            ExecOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("INSERT 0 3"));
    assert_eq!(s.rows_affected, Some(3));
    let s = c
        .execute("CREATE TEMP TABLE hv_t10_tag2(a int)", simple())
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("CREATE TABLE"));
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn extended_rejects_multiple_commands() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    let e = c
        .execute("SELECT 1; SELECT 2", ExecOpts::default())
        .await
        .unwrap_err();
    assert_eq!(e.server.unwrap().sqlstate.as_deref(), Some("42601"));
}

#[tokio::test]
#[ignore = "docker"]
async fn tx_status_follows_protocol() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch("BEGIN").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    let _ = c.execute("SELECT 1/0", ExecOpts::default()).await;
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

#[tokio::test]
#[ignore = "docker"]
async fn tx_status_is_failed_right_after_error() {
    // No sleep (Q12): both protocols.
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    for opts in [ExecOpts::default(), simple()] {
        c.batch("BEGIN").await.unwrap();
        assert!(c.execute("SELECT 1/0", opts).await.is_err());
        assert_eq!(c.tx_status(), TxStatus::Failed, "{opts:?}");
        c.batch("ROLLBACK").await.unwrap();
    }
    // The error of a batch too.
    assert!(c.batch("BEGIN; SELECT 1/0").await.is_err());
    assert_eq!(c.tx_status(), TxStatus::Failed);
}

#[tokio::test]
#[ignore = "docker"]
async fn rollback_outside_tx_reports_25p01_notice() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    let s = c.execute("ROLLBACK", simple()).await.unwrap();
    assert!(s.notice_codes.contains(&"25P01".to_string()), "{s:?}");
    assert_eq!(s.command_tag.as_deref(), Some("ROLLBACK"));
    // Drained: the next statement starts empty.
    assert!(
        c.execute("SELECT 1", simple())
            .await
            .unwrap()
            .notice_codes
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn cancel_pg_sleep_within_one_second() {
    for t in ["pg17", "pg17-tls"].into_iter().filter_map(require_target) {
        let mut c = pg_on(&t).await;
        let h = c.cancel_handle().unwrap();
        let start = Instant::now();
        let (r, cancelled) = tokio::join!(
            c.execute("SELECT pg_sleep(60)", ExecOpts::default()),
            async {
                tokio::time::sleep(ms(200)).await;
                h.cancel().await
            }
        );
        assert!(cancelled.is_ok(), "{}: {cancelled:?}", t.id);
        assert_eq!(
            r.unwrap_err().server.unwrap().sqlstate.as_deref(),
            Some("57014"),
            "{}",
            t.id
        );
        assert!(start.elapsed() < secs(2), "{}", t.id);
        assert!(c.execute("SELECT 1", simple()).await.is_ok(), "{}", t.id);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn reset_discards_all() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch(
        "PREPARE p AS SELECT 1; LISTEN ch; CREATE TEMP TABLE t(a int); \
         SELECT pg_advisory_lock(1); SET search_path = pg_catalog",
    )
    .await
    .unwrap();
    c.reset().await.unwrap();
    // Q10
    assert_eq!(
        scalar(
            &mut c,
            "SELECT count(*) FROM pg_prepared_statements WHERE from_sql"
        )
        .await,
        "0"
    );
    assert_eq!(
        scalar(&mut c, "SELECT count(*) FROM pg_listening_channels()").await,
        "0"
    );
    assert_eq!(
        scalar(
            &mut c,
            "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid()"
        )
        .await,
        "0"
    );
    assert_eq!(
        scalar(
            &mut c,
            "SELECT count(*) FROM pg_class WHERE relname = 't' AND relpersistence = 't'"
        )
        .await,
        "0"
    );
    assert_ne!(scalar(&mut c, "SHOW search_path").await, "pg_catalog");
}

#[tokio::test]
#[ignore = "docker"]
async fn reset_keeps_the_read_only_session_default() {
    // RESET ALL returns to the startup values, so the read-only `-c` option survives it.
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut r = request(&t, &t.user, InsecureAuth::Allow);
    r.read_only = true;
    let mut c = connect_conn(r).await.unwrap();
    c.batch("SET default_transaction_read_only = off")
        .await
        .unwrap();
    c.reset().await.unwrap();
    assert_eq!(
        scalar(&mut c, "SHOW default_transaction_read_only").await,
        "on"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn md5_and_cleartext_users_refused_without_verified_tls() {
    // Pending until Task 16 adds the target (W3).
    let Some(t) = require_target("pg17-auth") else {
        return;
    };
    for user in ["hv_md5", "hv_clear"] {
        let e = pg_as(&t, user, InsecureAuth::RefuseUnlessVerified)
            .await
            .err()
            .unwrap();
        assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused, "{user}");
        assert!(pg_as(&t, user, InsecureAuth::Allow).await.is_ok(), "{user}");
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn scram_channel_binding_require_succeeds_on_pg17_tls() {
    let Some(t) = require_target("pg17-tls") else {
        return;
    };
    let r = request(&t, &t.user, InsecureAuth::RefuseUnlessVerified);
    let c = connect_conn_with(r, |config| {
        config.channel_binding(ChannelBindingMode::Require);
    })
    .await;
    assert!(c.is_ok(), "{:?}", c.err());
}

#[tokio::test]
#[ignore = "docker"]
async fn extended_query_on_enum_column_after_reset() {
    // Q10: the type-lookup statements tokio-postgres caches survive the reset.
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch(
        "DROP TABLE IF EXISTS hv_mood_t; DROP TYPE IF EXISTS hv_mood; \
         CREATE TYPE hv_mood AS ENUM ('ok', 'sad'); CREATE TABLE hv_mood_t(m hv_mood); \
         INSERT INTO hv_mood_t VALUES ('ok')",
    )
    .await
    .unwrap();
    assert!(
        c.execute("SELECT m FROM hv_mood_t", ExecOpts::default())
            .await
            .is_ok()
    );
    c.reset().await.unwrap();
    let r = c
        .execute("SELECT m FROM hv_mood_t", ExecOpts::default())
        .await;
    assert!(r.is_ok(), "{r:?}");
}

#[tokio::test]
#[ignore = "docker"]
async fn backend_id_is_pid() {
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    assert_eq!(
        c.backend_id().unwrap().to_string(),
        scalar(&mut c, "SELECT pg_backend_pid()").await
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn copy_from_stdin_is_refused_and_the_session_stays_in_step() {
    // Fix round 1, I1: every path into copy-in mode (simple inside BEGIN, extended, batch)
    // ends with a clear error, an exact status and a usable connection.
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;
    c.batch("CREATE TEMP TABLE hv_t10_copy(a int)")
        .await
        .unwrap();

    let e = c
        .execute("BEGIN; COPY hv_t10_copy FROM STDIN", simple())
        .await
        .unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Unsupported);
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);

    c.batch("BEGIN").await.unwrap();
    let e = c
        .execute("COPY hv_t10_copy FROM STDIN", ExecOpts::default())
        .await
        .unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Unsupported);
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);

    let e = c.batch("COPY hv_t10_copy FROM STDIN").await.unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Unsupported);
    assert_eq!(c.tx_status(), TxStatus::Idle);

    // Still in step: each reply belongs to its own request.
    assert_eq!(scalar(&mut c, "SELECT 41 + 1").await, "42");
    assert_eq!(
        scalar(&mut c, "SELECT count(*) FROM hv_t10_copy").await,
        "0"
    );
    // COPY TO STDOUT ends with its own ReadyForQuery and is left alone.
    assert!(
        c.execute("COPY hv_t10_copy TO STDOUT", simple())
            .await
            .is_err()
    );
    assert_eq!(scalar(&mut c, "SELECT 7").await, "7");
}

// ## m2b-f3-internal-reads

/// Finding F3: Hacivat's own probes read values, and on PostgreSQL only the values path
/// (`ExecOpts::values`, the simple protocol) decodes them; the extended protocol (the default)
/// counts the rows and drops their values. The session's reset verify probe runs right after
/// the driver's reset sequence, so this does too; the schema probe's five columns all decode.
#[tokio::test]
#[ignore = "docker"]
async fn internal_value_reads_decode_through_the_values_path() {
    const VERIFY: &str = "SELECT current_user = session_user";
    let Some(t) = require_target("pg17") else {
        return;
    };
    let mut c = pg_on(&t).await;

    // The count-only path: the row is there, its value is not (the cause of F3).
    let counted = c.execute(VERIFY, ExecOpts::default()).await.unwrap();
    assert_eq!((counted.row_count, counted.rows.len()), (1, 0));

    c.reset().await.unwrap();
    let verify = c.execute(VERIFY, ExecOpts::values(1)).await.unwrap();
    assert_eq!(verify.rows, [vec![Some("t".to_string())]]);
    assert!(!verify.truncated);

    let schema = c
        .execute(
            "SELECT current_database(), current_schema(), \
             array_to_string(current_schemas(true), ','), current_user, \
             current_setting('search_path')",
            ExecOpts::values(128),
        )
        .await
        .unwrap();
    let [row] = schema.rows.as_slice() else {
        panic!("one row: {:?}", schema.rows);
    };
    assert_eq!(row.len(), 5);
    assert_eq!(row[0].as_deref(), Some(t.database.as_str()));
    assert_eq!(row[3].as_deref(), Some(t.user.as_str()));
    assert!(row.iter().all(Option::is_some), "{row:?}");
    assert_eq!(c.tx_status(), TxStatus::Idle);
    Box::new(c).close().await;
}
