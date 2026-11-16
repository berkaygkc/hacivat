//! MySQL and MariaDB execution, status flags, read-only capabilities, `COM_RESET_CONNECTION`,
//! `KILL QUERY`, Hacivat's rustls configuration and the outcome-based password guard against
//! the Docker matrix (Task 13; B2, B3, B5, B8, B10, B21, Q2, Q20). Every Docker test is
//! `#[ignore = "docker"]` and takes its targets only from `require_target(..)` or
//! `targets(Some(engine))` (Q6). Run with
//! `cargo xtask docker test --set pr|full -p hacivat-driver-mysql`.
//!
//! - The fixtures (`hv_probe`, `hv_ro`) are created idempotently through the admin connection
//!   (Q7). The matrix's most privileged account with a known password is the test account:
//!   the servers' root passwords are random.
//! - Failures print target ids and error kinds only, never a password.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, RawConn, RawError, RawErrorKind,
    TxStatus,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn, guard_allows};
use hacivat_net::{
    AddressClass, CertPin, ClientCertPaths, Target, TlsFailure, TlsMode, TlsParams, TlsSettings,
    TlsVerified, build_tls_params, cert_sha256, direct, install_crypto_provider, pin_endpoint,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, artifact, require_target, targets};
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

/// The read-only account the fixtures grant `SELECT, SHOW VIEW` on `hv_test` (Q7).
const RO_USER: &str = "hv_ro";

/// Serialises every sign-in as [`RO_USER`] in this binary: the full-authentication tests empty
/// its `caching_sha2_password` cache entry and rely on nobody filling it before they sign in.
static RO_SIGN_IN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Require where the server's certificate is self-signed, else Auto (P12).
fn default_mode(t: &DockerTarget) -> TlsMode {
    match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    }
}

fn mode(m: TlsMode) -> TlsSettings {
    TlsSettings {
        mode: m,
        ..TlsSettings::default()
    }
}

/// Auto with these pins.
fn auto_with(pins: &[CertPin]) -> TlsSettings {
    TlsSettings {
        pins: pins.to_vec(),
        ..mode(TlsMode::Auto)
    }
}

/// One `TlsParams` per connect attempt (T3 carry-in): never shared between two handshakes.
fn params(t: &DockerTarget, settings: &TlsSettings) -> Option<TlsParams> {
    install_crypto_provider();
    build_tls_params(settings, &t.host, t.port, AddressClass::Loopback, None)
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

fn request(
    t: &DockerTarget,
    tls: Option<TlsParams>,
    user: &str,
    read_only: bool,
    insecure_auth: InsecureAuth,
) -> DriverConnect {
    install_crypto_provider();
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let target = Target::Tcp {
        host: t.host.clone(),
        port: t.port,
    };
    DriverConnect {
        endpoint: Endpoint::Network {
            factory: direct(target.clone(), vec![addr], secs(5)),
            direct: Some(target),
            resolved: vec![addr],
            tls_host: t.host.clone(),
        },
        tls,
        user: Some(user.to_string()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat tests".into(),
        read_only,
        options: vec![],
        connect_timeout: secs(20),
        events: Arc::new(|_| {}),
        insecure_auth,
    }
}

async fn connect_or_panic(t: &DockerTarget, r: DriverConnect) -> MySqlConn {
    match connect_conn(r).await {
        Ok(c) => c,
        Err(e) => panic!("{}: {:?}", t.id, e.kind),
    }
}

/// The test account, read-write, in the target's default TLS mode. Loopback, so any sign-in
/// method is allowed, as the pipeline decides for loopback (B8).
async fn mysql(t: &DockerTarget) -> MySqlConn {
    let tls = params(t, &mode(default_mode(t)));
    connect_or_panic(t, request(t, tls, &t.user, false, InsecureAuth::Allow)).await
}

/// The admin connection the fixtures are created through (Q7).
async fn admin(t: &DockerTarget) -> MySqlConn {
    mysql(t).await
}

/// The test account on a read-only connection.
async fn mysql_read_only(t: &DockerTarget) -> MySqlConn {
    let tls = params(t, &mode(default_mode(t)));
    connect_or_panic(t, request(t, tls, &t.user, true, InsecureAuth::Allow)).await
}

/// `hv_ro` (`SELECT, SHOW VIEW` only) on a read-only connection.
async fn mysql_ro(t: &DockerTarget) -> MySqlConn {
    let _signing_in = RO_SIGN_IN.lock().await;
    let tls = params(t, &mode(default_mode(t)));
    connect_or_panic(t, request(t, tls, RO_USER, true, InsecureAuth::Allow)).await
}

/// `hv_ro` with these TLS settings and policy; the caller holds [`RO_SIGN_IN`].
async fn ro_as(
    t: &DockerTarget,
    settings: &TlsSettings,
    policy: InsecureAuth,
) -> Result<MySqlConn, RawError> {
    connect_conn(request(t, params(t, settings), RO_USER, false, policy)).await
}

/// Connects the test account with `settings` and returns the attempt's `TlsParams` too, so the
/// test can read the presented chain and the outcome.
async fn mysql_tls_keep(
    t: &DockerTarget,
    settings: TlsSettings,
) -> (Result<MySqlConn, RawError>, TlsParams) {
    let tls = params(t, &settings).expect("a TLS mode");
    let kept = tls.clone();
    let r = connect_conn(request(t, Some(tls), &t.user, false, InsecureAuth::Allow)).await;
    (r, kept)
}

/// The pin of the leaf the server presented to `params`.
fn pin_from(params: &TlsParams, host: &str, port: u16) -> CertPin {
    let presented = params.presented.lock().unwrap().clone();
    let chain = presented.expect("the verifier records the presented chain");
    CertPin {
        endpoint: pin_endpoint(host, port),
        sha256: cert_sha256(chain.0[0].as_ref()),
    }
}

/// The pin of `t`'s current leaf, read over a Require connection.
async fn pin_of_target(t: &DockerTarget) -> CertPin {
    let (r, params) = mysql_tls_keep(t, mode(TlsMode::Require)).await;
    let c = r.unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
    Box::new(c).close().await;
    pin_from(&params, &t.host, t.port)
}

/// The first cell of the first row of `sql`, as text.
async fn text(c: &mut MySqlConn, sql: &str) -> String {
    let s = c
        .execute(sql, simple())
        .await
        .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    s.rows[0][0].clone().expect("not NULL")
}

fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

async fn table_exists(admin: &mut MySqlConn, name: &str) -> bool {
    let sql = format!(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'hv_test' AND table_name = '{name}'"
    );
    text(admin, &sql).await == "1"
}

/// `hv_probe(a int)` in `hv_test`, created when missing (Q7).
async fn ensure_probe_table(admin: &mut MySqlConn) {
    admin
        .batch("CREATE TABLE IF NOT EXISTS hv_test.hv_probe (a int)")
        .await
        .unwrap_or_else(|e| panic!("hv_probe: {:?}", e.kind));
}

/// `hv_ro` with `SELECT, SHOW VIEW` on `hv_test`, created when it cannot sign in (Q7). The
/// fixtures of `docker/m2a/init/mysql.sql` create it on a fresh volume; otherwise the admin
/// connection needs `CREATE USER` and the grant option.
async fn ensure_ro_role(admin: &mut MySqlConn, t: &DockerTarget) {
    {
        let _signing_in = RO_SIGN_IN.lock().await;
        let tls = params(t, &mode(default_mode(t)));
        if let Ok(c) = connect_conn(request(t, tls, RO_USER, true, InsecureAuth::Allow)).await {
            Box::new(c).close().await;
            return;
        }
    }
    let create = format!(
        "CREATE USER IF NOT EXISTS '{RO_USER}'@'%' IDENTIFIED BY '{}'",
        t.password
    );
    for sql in [
        create,
        format!("GRANT SELECT, SHOW VIEW ON hv_test.* TO '{RO_USER}'@'%'"),
    ] {
        if let Err(e) = admin.batch(&sql).await {
            panic!(
                "{}: cannot create {RO_USER}: {:?} {:?}",
                t.id,
                e.kind,
                vendor(&e)
            );
        }
    }
}

/// Empties `hv_ro`'s `caching_sha2_password` cache entry, so its next sign-in runs full
/// authentication (the server's "perform full authentication", where the password guard
/// decides). `FLUSH PRIVILEGES` needs `RELOAD`, which no account with a known password has, so
/// `hv_ro` sets its own password again (the same throwaway value): changing an account's
/// password clears its entry. The caller holds [`RO_SIGN_IN`].
async fn flush_auth_cache(t: &DockerTarget) {
    let mut c = ro_as(t, &mode(TlsMode::Require), InsecureAuth::Allow)
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
    let set = format!("SET PASSWORD = '{}'", t.password);
    if let Err(e) = c.batch(&set).await {
        panic!("{}: SET PASSWORD: {:?} {:?}", t.id, e.kind, vendor(&e));
    }
    Box::new(c).close().await;
}

/// Every MySQL and MariaDB target of the active set.
fn mysql_family() -> Vec<DockerTarget> {
    let mut ts = targets(Some(DbmsKind::MySql));
    ts.extend(targets(Some(DbmsKind::MariaDb)));
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

#[tokio::test]
#[ignore = "docker"]
async fn execute_and_status_flags() {
    for t in mysql_family() {
        let mut c = mysql(&t).await;
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        c.batch("START TRANSACTION").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::InTx, "{}", t.id);
        // An ERR keeps the transaction (the server reports no status in it).
        let e = c
            .execute("SELECT * FROM no_such_table", simple())
            .await
            .unwrap_err();
        assert_eq!(vendor(&e), Some(1146), "{}", t.id);
        assert_eq!(c.tx_status(), TxStatus::InTx, "{}", t.id);
        c.batch("ROLLBACK").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        // The status comes from the statement's OK packet, in both protocols.
        let s = c
            .execute("START TRANSACTION", ExecOpts::default())
            .await
            .unwrap();
        assert_eq!(s.tx, TxStatus::InTx, "{}", t.id);
        let s = c.execute("COMMIT", simple()).await.unwrap();
        assert_eq!(s.tx, TxStatus::Idle, "{}", t.id);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn a_failed_multi_statement_reports_the_transaction_it_opened() {
    // The ERR packet carries no status: a transaction an earlier statement of the same text
    // opened is still reported (read-write connections only run several statements).
    for t in mysql_family() {
        let mut c = mysql(&t).await;
        let e = c
            .execute("START TRANSACTION; SELECT * FROM no_such_table", simple())
            .await
            .unwrap_err();
        assert_eq!(vendor(&e), Some(1146), "{}", t.id);
        assert_eq!(c.tx_status(), TxStatus::InTx, "{}", t.id);
        c.batch("ROLLBACK").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        Box::new(c).close().await;
    }
}

// ## m2b-t13-mysql fix round 1 (review I1)

/// The `(Level, Code)` rows of `SHOW WARNINGS`.
async fn warnings(c: &mut MySqlConn) -> Vec<(String, String)> {
    let s = c.execute("SHOW WARNINGS", simple()).await.unwrap();
    s.rows
        .into_iter()
        .map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap()))
        .collect()
}

#[tokio::test]
#[ignore = "docker"]
async fn a_failed_statement_keeps_its_diagnostics() {
    // The status re-read after an error is `COM_PING`, not a statement, so `SHOW WARNINGS` still
    // describes the failed statement, in and out of a transaction.
    for t in mysql_family() {
        let mut c = mysql(&t).await;
        for in_tx in [false, true] {
            if in_tx {
                c.batch("START TRANSACTION").await.unwrap();
            }
            let e = c
                .execute("SELECT * FROM no_such_table", ExecOpts::default())
                .await
                .unwrap_err();
            assert_eq!(vendor(&e), Some(1146), "{}", t.id);
            let want = if in_tx {
                TxStatus::InTx
            } else {
                TxStatus::Idle
            };
            assert_eq!(c.tx_status(), want, "{}", t.id);
            assert_eq!(
                warnings(&mut c).await,
                [("Error".to_string(), "1146".to_string())],
                "{} in_tx={in_tx}",
                t.id
            );
        }
        c.batch("ROLLBACK").await.unwrap();
        // A warning of a successful statement survives too (no status re-read runs then).
        c.execute("SELECT CAST('1x' AS SIGNED)", simple())
            .await
            .unwrap();
        assert_eq!(
            warnings(&mut c).await,
            [("Warning".to_string(), "1292".to_string())],
            "{}",
            t.id
        );
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn a_failing_ddl_inside_a_transaction_commits_it() {
    // MySQL and MariaDB commit implicitly before a DDL statement runs, also when it then fails
    // (1050: the table exists): the server is Idle, and so is the status the driver reports.
    for t in mysql_family() {
        let mut a = admin(&t).await;
        ensure_probe_table(&mut a).await;
        let mut c = mysql(&t).await;
        c.batch("START TRANSACTION").await.unwrap();
        assert_eq!(c.tx_status(), TxStatus::InTx, "{}", t.id);
        let e = c
            .execute("CREATE TABLE hv_test.hv_probe (a int)", simple())
            .await
            .unwrap_err();
        assert_eq!(vendor(&e), Some(1050), "{}", t.id);
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        // The next statement's own OK packet agrees.
        let s = c.execute("SELECT 1", simple()).await.unwrap();
        assert_eq!(s.tx, TxStatus::Idle, "{}", t.id);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn simple_rows_are_text_and_extended_rows_are_counted() {
    for t in mysql_family() {
        let mut c = mysql(&t).await;
        let s = c
            .execute("SELECT 1 AS a, 'ş' AS b, NULL AS c", simple())
            .await
            .unwrap();
        assert_eq!(s.columns, ["a", "b", "c"], "{}", t.id);
        assert_eq!(
            s.rows,
            [vec![Some("1".into()), Some("ş".into()), None]],
            "{}",
            t.id
        );
        assert_eq!(s.command_tag.as_deref(), Some("SELECT"), "{}", t.id);
        // A result set reports 0 affected rows in its final OK packet, never the row count.
        assert_eq!(s.rows_affected, Some(0), "{}", t.id);
        let many = "SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4";
        let s = c
            .execute(
                many,
                ExecOpts {
                    max_rows: 2,
                    protocol: Protocol::Simple,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            (s.rows.len(), s.row_count, s.truncated),
            (2, 4, true),
            "{}",
            t.id
        );
        assert_eq!(s.rows_affected, Some(0), "{}", t.id);
        let e = c.execute(many, ExecOpts::default()).await.unwrap();
        assert_eq!((e.rows.len(), e.row_count), (0, 4), "{}", t.id);
        assert_eq!(e.columns.len(), 1, "{}", t.id);
        assert_eq!(e.rows_affected, Some(0), "{}", t.id);
        // Several statements on a read-write connection: the last result set's rows.
        let s = c.execute("SELECT 1; SELECT 2", simple()).await.unwrap();
        assert_eq!(s.rows, [vec![Some("2".into())]], "{}", t.id);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn command_tag_is_the_first_keyword() {
    // Q35: MySQL has no command tags; the guard's post-check reads the first keyword and the
    // OK packet's affected rows.
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_probe_table(&mut a).await;
    let mut c = mysql(&t).await;
    let s = c
        .execute("insert into hv_probe values (1)", simple())
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("INSERT"));
    assert_eq!(s.rows_affected, Some(1));
    let s = c
        .execute(
            "/* why */ DELETE FROM hv_probe WHERE a = 1",
            ExecOpts::default(),
        )
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("DELETE"));
    assert!(s.rows_affected.is_some_and(|n| n >= 1));
}

#[tokio::test]
#[ignore = "docker"]
async fn read_only_connection_has_no_multi_statements() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_probe_table(&mut a).await;
    ensure_ro_role(&mut a, &t).await;
    let mut c = mysql_ro(&t).await;
    assert!(
        c.execute("SELECT 1; DROP TABLE hv_probe", simple())
            .await
            .is_err()
    );
    assert!(table_exists(&mut a, "hv_probe").await);
    // Not the missing privilege: an account that could run both statements still gets a
    // syntax error, because a read-only connection never asks for multi-statements.
    let mut rw_account = mysql_read_only(&t).await;
    let e = rw_account
        .execute("SELECT 1; SELECT 2", simple())
        .await
        .unwrap_err();
    assert_eq!(vendor(&e), Some(1064));
    assert_eq!(
        text(&mut rw_account, "SELECT @@session.transaction_read_only").await,
        "1"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn read_only_extended_never_falls_back() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_ro_role(&mut a, &t).await;
    let mut c = mysql_ro(&t).await;
    assert_eq!(
        c.execute("PREPARE s FROM 'SELECT 1'", ExecOpts::default())
            .await
            .unwrap_err()
            .kind,
        RawErrorKind::Unsupported
    );
    // A read-write connection runs it as text instead (ER_UNSUPPORTED_PS, 1295).
    let mut rw = mysql(&t).await;
    let s = rw
        .execute("PREPARE s FROM 'SELECT 1'", ExecOpts::default())
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("PREPARE"));
}

#[tokio::test]
#[ignore = "docker"]
async fn read_only_prepared_call_cannot_write() {
    // Review probe: a prepared CALL runs, but the session's read-only mode and the account's
    // privileges stop its writes.
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_probe_table(&mut a).await;
    ensure_ro_role(&mut a, &t).await;
    a.batch(
        "CREATE PROCEDURE IF NOT EXISTS hv_test.hv_t13_write() \
         INSERT INTO hv_test.hv_probe VALUES (13)",
    )
    .await
    .unwrap_or_else(|e| panic!("procedure: {:?} {:?}", e.kind, vendor(&e)));
    let before = text(&mut a, "SELECT COUNT(*) FROM hv_test.hv_probe WHERE a = 13").await;
    let mut ro_session = mysql_read_only(&t).await;
    let e = ro_session
        .execute("CALL hv_test.hv_t13_write()", ExecOpts::default())
        .await
        .unwrap_err();
    // ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION
    assert_eq!(vendor(&e), Some(1792));
    let mut ro_account = mysql_ro(&t).await;
    let e = ro_account
        .execute("CALL hv_test.hv_t13_write()", ExecOpts::default())
        .await
        .unwrap_err();
    // ER_PROCACCESS_DENIED_ERROR: hv_ro has no EXECUTE
    assert_eq!(vendor(&e), Some(1370));
    let after = text(&mut a, "SELECT COUNT(*) FROM hv_test.hv_probe WHERE a = 13").await;
    assert_eq!(before, after);
}

#[tokio::test]
#[ignore = "docker"]
async fn kill_query_side_connection_cancels_sleep() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = mysql(&t).await;
    let id = a.backend_id().unwrap();
    let mut b = mysql(&t).await;
    let start = Instant::now();
    let (r, killed) = tokio::join!(a.execute("SELECT SLEEP(60)", simple()), async {
        tokio::time::sleep(ms(200)).await;
        b.batch(&format!("KILL QUERY {id}")).await
    });
    assert!(killed.is_ok());
    // An interrupted SLEEP answers 1 without an error on some versions (Q32).
    assert!(
        matches!(
            r,
            Ok(_)
                | Err(RawError {
                    kind: RawErrorKind::Cancelled,
                    ..
                })
        ),
        "{:?}",
        r.as_ref().err().map(|e| &e.kind)
    );
    assert!(start.elapsed() < secs(2));
    assert!(a.execute("SELECT 1", simple()).await.is_ok());
}

#[tokio::test]
#[ignore = "docker"]
async fn max_execution_time_interruption_is_cancelled_and_leaves_the_connection_usable() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut c = mysql(&t).await;
    c.batch("SET SESSION cte_max_recursion_depth = 4294967295")
        .await
        .unwrap();
    let e = c
        .execute(
            "SELECT /*+ MAX_EXECUTION_TIME(100) */ COUNT(*) FROM (WITH RECURSIVE n(i) AS \
             (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000000000) SELECT i FROM n) x",
            simple(),
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Cancelled);
    assert_eq!(vendor(&e), Some(3024));
    assert_eq!(text(&mut c, "SELECT 1").await, "1");
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

#[tokio::test]
#[ignore = "docker"]
async fn reset_connection_clears_user_vars_and_temp_tables() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut c = mysql(&t).await;
    c.batch("USE orders").await.unwrap();
    c.batch("SET @x = 1").await.unwrap();
    c.batch("CREATE TEMPORARY TABLE tt(a int)").await.unwrap();
    c.batch("START TRANSACTION").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    c.reset().await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    assert_eq!(text(&mut c, "SELECT @x IS NULL").await, "1");
    assert!(c.execute("SELECT * FROM tt", simple()).await.is_err());
    // Q15: the current database survives COM_RESET_CONNECTION.
    assert_eq!(text(&mut c, "SELECT DATABASE()").await, "orders");
}

#[tokio::test]
#[ignore = "docker"]
async fn reset_keeps_a_read_only_session_read_only() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut c = mysql_read_only(&t).await;
    c.batch("SET SESSION TRANSACTION READ WRITE").await.unwrap();
    assert_eq!(
        text(&mut c, "SELECT @@session.transaction_read_only").await,
        "0"
    );
    c.reset().await.unwrap();
    assert_eq!(
        text(&mut c, "SELECT @@session.transaction_read_only").await,
        "1"
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn get_lock_after_reset_is_released() {
    // Q32: COM_RESET_CONNECTION releases the session's user-level locks.
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut c = mysql(&t).await;
    c.batch("DO GET_LOCK('hv_t13', 0)").await.unwrap();
    let mut other = admin(&t).await;
    assert_eq!(text(&mut other, "SELECT IS_FREE_LOCK('hv_t13')").await, "0");
    c.reset().await.unwrap();
    assert_eq!(text(&mut other, "SELECT IS_FREE_LOCK('hv_t13')").await, "1");
}

#[tokio::test]
#[ignore = "docker"]
async fn backend_id_is_the_thread_id() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut c = mysql(&t).await;
    let id = c.backend_id().unwrap();
    assert_eq!(text(&mut c, "SELECT CONNECTION_ID()").await, id.to_string());
}

#[tokio::test]
#[ignore = "docker"]
async fn verify_ca_with_container_ca_skips_the_name() {
    // P11 superseded: VerifyCa runs through Hacivat's `ChainOnly` verifier (Q2).
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let settings = TlsSettings {
        ca_file: Some(artifact("mysql84-ca.pem")),
        ..mode(TlsMode::VerifyCa)
    };
    let (r, params) = mysql_tls_keep(&t, settings).await;
    let mut c = r.unwrap_or_else(|e| panic!("{:?}", e.kind));
    assert_eq!(*params.outcome.lock().unwrap(), Some(TlsVerified::CaOnly));
    let s = c.tls_summary().await;
    assert!(s.encrypted);
    assert_eq!(s.verified, TlsVerified::CaOnly);
}

#[tokio::test]
#[ignore = "docker"]
async fn pin_flow_on_mysql84() {
    // Q2: the presented chain is captured on UnknownIssuer; a retry with the pin gives Pinned.
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let (e, params) = mysql_tls_keep(&t, auto_with(&[])).await;
    assert_eq!(
        e.err().map(|e| e.kind),
        Some(RawErrorKind::Tls(TlsFailure::UnknownIssuer))
    );
    let pin = pin_from(&params, &t.host, t.port);
    let (r, p2) = mysql_tls_keep(&t, auto_with(&[pin])).await;
    let mut c = r.unwrap_or_else(|e| panic!("{:?}", e.kind));
    assert_eq!(*p2.outcome.lock().unwrap(), Some(TlsVerified::Pinned));
    assert_eq!(c.tls_summary().await.verified, TlsVerified::Pinned);
    // Another certificate under the same pin is a pin mismatch.
    let wrong = CertPin {
        sha256: [0; 32],
        ..pin_from(&params, &t.host, t.port)
    };
    let (r, _) = mysql_tls_keep(&t, auto_with(&[wrong])).await;
    assert_eq!(
        r.err().map(|e| e.kind),
        Some(RawErrorKind::Tls(TlsFailure::PinMismatch))
    );
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Require with the fixture client certificate and its encrypted PKCS #8 key (PBES2,
/// PBKDF2-HMAC-SHA-256, AES-256-CBC, as OpenSSL 3 writes it), decrypted by Hacivat (B25).
fn tls_params_with_encrypted_client_key(t: &DockerTarget, passphrase: &str) -> TlsParams {
    install_crypto_provider();
    let settings = TlsSettings {
        client_cert: Some(ClientCertPaths {
            cert: fixture("client-cert.pem"),
            key: fixture("client-key-enc.pem"),
        }),
        ..mode(TlsMode::Require)
    };
    let passphrase = SecretString::from(passphrase);
    build_tls_params(
        &settings,
        &t.host,
        t.port,
        AddressClass::Loopback,
        Some(&passphrase),
    )
    .unwrap_or_else(|e| panic!("{e:?}"))
    .expect("Require is a TLS mode")
}

/// The TLS alert the server sent, from a failed `mysql_async` connect.
fn alert_of(e: &mysql_async::Error) -> Option<rustls::AlertDescription> {
    use mysql_async::{Error as E, IoError, TlsError};
    let r = match e {
        E::Io(IoError::Io(io)) => io.get_ref()?.downcast_ref::<rustls::Error>()?,
        E::Io(IoError::Tls(TlsError::Tls(r))) => r,
        _ => return None,
    };
    match r {
        rustls::Error::AlertReceived(a) => Some(*a),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn encrypted_pkcs8_client_key() {
    // Q2: the ClientConfig carries the client certificate Hacivat decrypted, and it reaches the
    // server; `mysql_async` never reads the key file. The matrix's MySQL verifies a presented
    // client certificate against its own auto-generated CA (`ssl_ca = ca.pem`, whose key never
    // leaves the container), so it answers this fixture with `unknown_ca`: that alert is the
    // proof the identity was sent. Signing in with it needs a target whose CA trusts a test CA
    // (Task 16).
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let params = tls_params_with_encrypted_client_key(&t, "hv_test_pw");
    assert!(params.config.client_auth_cert_resolver.has_certs());
    let req = request(&t, Some(params), &t.user, false, InsecureAuth::Allow);
    let e = match mysql_async::Conn::new(hacivat_driver_mysql::opts_for(&req)).await {
        Ok(_) => panic!("the server trusts only client certificates from its own CA"),
        Err(e) => e,
    };
    assert_eq!(alert_of(&e), Some(rustls::AlertDescription::UnknownCA));
    // Through the driver it is a TLS failure, and the same server without the identity signs in.
    let r = connect_conn(request(
        &t,
        Some(tls_params_with_encrypted_client_key(&t, "hv_test_pw")),
        &t.user,
        false,
        InsecureAuth::Allow,
    ))
    .await;
    assert!(matches!(
        r.err().map(|e| e.kind),
        Some(RawErrorKind::Tls(TlsFailure::Protocol))
    ));
    let mut c = mysql(&t).await;
    assert!(c.tls_summary().await.encrypted);
}

#[tokio::test]
#[ignore = "docker"]
async fn insecure_password_refused_over_plaintext_full_auth() {
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_ro_role(&mut a, &t).await;
    let _signing_in = RO_SIGN_IN.lock().await;
    flush_auth_cache(&t).await;
    let e = ro_as(
        &t,
        &mode(TlsMode::Disable),
        InsecureAuth::RefuseUnlessVerified,
    )
    .await
    .err()
    .expect("full authentication without TLS is refused");
    assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused);
    let c = ro_as(&t, &mode(TlsMode::Disable), InsecureAuth::Allow)
        .await
        .unwrap_or_else(|e| panic!("{:?}", e.kind));
    Box::new(c).close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn require_with_pin_full_auth_is_allowed() {
    // Q20: outcome-based. Require without a pin is unverified (`No`) and refused; with the
    // server's pin the outcome is `Pinned` and the password may go.
    let Some(t) = require_target("mysql84") else {
        return;
    };
    let mut a = admin(&t).await;
    ensure_ro_role(&mut a, &t).await;
    let pin = pin_of_target(&t).await;
    let _signing_in = RO_SIGN_IN.lock().await;
    flush_auth_cache(&t).await;
    let e = ro_as(
        &t,
        &mode(TlsMode::Require),
        InsecureAuth::RefuseUnlessVerified,
    )
    .await
    .err()
    .expect("an unverified Require refuses full authentication");
    assert_eq!(e.kind, RawErrorKind::InsecureAuthRefused);
    let pinned = TlsSettings {
        mode: TlsMode::Require,
        ..auto_with(&[pin])
    };
    let c = ro_as(&t, &pinned, InsecureAuth::RefuseUnlessVerified)
        .await
        .unwrap_or_else(|e| panic!("{:?}", e.kind));
    Box::new(c).close().await;
}

/// Every TLS mode × outcome × policy, with the expected answer: `Allow` always signs in; with
/// `RefuseUnlessVerified`, only a TLS connection whose server was verified (`Full`, `CaOnly`
/// or `Pinned`), whatever the mode. A plaintext attempt (`Disable`) never reads an outcome.
fn insecure_matrix() -> Vec<(TlsMode, Option<TlsVerified>, InsecureAuth, bool)> {
    let outcomes = [
        None,
        Some(TlsVerified::No),
        Some(TlsVerified::Full),
        Some(TlsVerified::CaOnly),
        Some(TlsVerified::Pinned),
    ];
    let mut out = Vec::new();
    for mode in TlsMode::ALL {
        for outcome in outcomes {
            out.push((mode, outcome, InsecureAuth::Allow, true));
            let verified = !matches!(outcome, None | Some(TlsVerified::No));
            let want = mode != TlsMode::Disable && verified;
            out.push((mode, outcome, InsecureAuth::RefuseUnlessVerified, want));
        }
    }
    out
}

#[test]
fn password_guard_matrix() {
    let matrix = insecure_matrix();
    assert_eq!(matrix.len(), 50);
    for (mode, outcome, ia, want) in matrix {
        assert_eq!(
            guard_allows(mode, outcome, ia),
            want,
            "{mode:?} {outcome:?} {ia:?}"
        );
    }
    // The cases the review asks about, spelled out: never with `No` or no outcome.
    for mode in TlsMode::ALL {
        for outcome in [None, Some(TlsVerified::No)] {
            assert!(!guard_allows(
                mode,
                outcome,
                InsecureAuth::RefuseUnlessVerified
            ));
        }
    }
    assert!(guard_allows(
        TlsMode::Require,
        Some(TlsVerified::Pinned),
        InsecureAuth::RefuseUnlessVerified
    ));
}

// ## m2b-fix-f2
#[tokio::test]
#[ignore = "docker"]
async fn a_refused_set_global_is_a_server_error_on_a_live_connection() {
    // F2: MySQL 8.0/8.4 answer the prepare of a `SET GLOBAL`/`SET PERSIST` the account may not
    // run with a bare OK packet, which `mysql_async` cannot read as a prepare reply, so the
    // connection ended. Asked for as a prepared statement, a `SET` now runs over `COM_QUERY`:
    // the refusal is the server's `1227` (MariaDB: `SET PERSIST` is a syntax error there), on
    // the same connection, read-only or not, and nothing changed.
    if !hacivat_testkit::docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let mut a = admin(&t).await;
        ensure_ro_role(&mut a, &t).await;
        let before = text(&mut a, "SELECT @@global.max_connections").await;
        for read_only in [false, true] {
            let mut c = {
                let _signing_in = RO_SIGN_IN.lock().await;
                let settings = mode(default_mode(&t));
                connect_conn(request(
                    &t,
                    params(&t, &settings),
                    RO_USER,
                    read_only,
                    InsecureAuth::Allow,
                ))
                .await
                .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind))
            };
            let id = c.backend_id();
            // A version-gated executable comment the server runs: MariaDB skips MySQL's
            // `/*!80000 …*/` (it is then an empty statement) and runs its own `/*M!…*/`.
            let gated = if t.engine == DbmsKind::MariaDb {
                "/*M!100000 SET @@global.max_connections = 999 */"
            } else {
                "/*!80000 SET @@global.max_connections = 999 */"
            };
            for sql in [
                "SET GLOBAL max_connections = 999",
                "SET PERSIST max_connections = 999",
                "/*!SET @@global.max_connections = 999 */",
                gated,
            ] {
                let Err(e) = c.execute(sql, ExecOpts::default()).await else {
                    panic!("{} read_only={read_only} {sql}: it ran", t.id);
                };
                let want = if t.engine == DbmsKind::MariaDb && sql.contains("PERSIST") {
                    1064
                } else {
                    1227
                };
                assert_eq!(
                    (e.kind.clone(), vendor(&e)),
                    (RawErrorKind::Db, Some(want)),
                    "{} read_only={read_only} {sql}",
                    t.id
                );
                assert_eq!(c.tx_status(), TxStatus::Idle, "{} {sql}", t.id);
                assert_eq!(c.backend_id(), id, "{}: the same connection", t.id);
                assert_eq!(text(&mut c, "SELECT 7").await, "7", "{} {sql}", t.id);
            }
            // A SET the account may run still runs, and keeps its effect.
            let s = c
                .execute("SET @hv_f2 = 2", ExecOpts::default())
                .await
                .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
            assert_eq!(s.command_tag.as_deref(), Some("SET"));
            assert_eq!(text(&mut c, "SELECT @hv_f2").await, "2", "{}", t.id);
            Box::new(c).close().await;
        }
        assert_eq!(
            text(&mut a, "SELECT @@global.max_connections").await,
            before,
            "{}",
            t.id
        );
    }
}
