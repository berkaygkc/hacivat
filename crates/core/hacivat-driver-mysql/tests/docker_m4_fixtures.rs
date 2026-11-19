//! The M4 MySQL/MariaDB accounts of `docker/m2a/init/mysql.sql` (M4 Task 1c, R-M4-43, carry
//! R-C1-1) against the Docker matrix. They exist only on volumes created from that file
//! (CI's fresh stack; locally after controller step C-2). Every test is `#[ignore = "docker"]`,
//! returns early unless `HACIVAT_DOCKER=1` and takes its targets from `targets(..)`; run them
//! through `run-docker-tests.sh --set pr|full -p hacivat-driver-mysql --test docker_m4_fixtures`.
//!
//! - `hv_catalog` tells a missing function from a built-in on every MySQL and MariaDB line
//!   (R-C1-1), reads the system schemas and writes nothing;
//! - `hv_test` owns `hvm4_*` databases and may create functions and triggers there;
//! - `hv_fixture` sets the global time zone and manages `hvm4_*` accounts, and nothing else.
//!
//! Every object a test creates is named `hvm4_t01c_*` and dropped by a guard, also on panic.
//! Failures print target ids and server codes only, never a password.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, RawConn, RawError,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{
    AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct, install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{CATALOG_PROBE_DATABASE, DockerTarget, TlsDefault, docker_enabled, targets};
use secrecy::SecretString;

fn simple() -> ExecOpts {
    ExecOpts {
        max_rows: 10_000,
        protocol: Protocol::Simple,
    }
}

/// Every MySQL and MariaDB target of the active set.
fn mysql_family() -> Vec<DockerTarget> {
    let mut ts = targets(Some(DbmsKind::MySql));
    ts.extend(targets(Some(DbmsKind::MariaDb)));
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

/// `t`'s account and database, in the target's default TLS mode.
async fn connect(t: &DockerTarget) -> MySqlConn {
    install_crypto_provider();
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    let tls = build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None)
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let target = Target::Tcp {
        host: t.host.clone(),
        port: t.port,
    };
    let req = DriverConnect {
        endpoint: Endpoint::Network {
            factory: direct(target.clone(), vec![addr], Duration::from_secs(5)),
            direct: Some(target),
            resolved: vec![addr],
            tls_host: t.host.clone(),
        },
        tls,
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat tests".into(),
        read_only: false,
        options: vec![],
        connect_timeout: Duration::from_secs(20),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    };
    connect_conn(req)
        .await
        .unwrap_or_else(|e| panic!("{} as {}: {:?} {:?}", t.id, t.user, e.kind, vendor(&e)))
}

fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

async fn ok(c: &mut MySqlConn, t: &DockerTarget, sql: &str) -> Vec<Vec<Option<String>>> {
    match c.execute(sql, simple()).await {
        Ok(s) => s.rows,
        Err(e) => panic!(
            "{} as {}: {sql}: {:?} {:?}",
            t.id,
            t.user,
            e.kind,
            vendor(&e)
        ),
    }
}

/// The server's error code for `sql`, which must fail.
async fn refused(c: &mut MySqlConn, t: &DockerTarget, sql: &str) -> i64 {
    match c.execute(sql, simple()).await {
        Ok(_) => panic!("{} as {}: {sql} succeeded", t.id, t.user),
        Err(e) => vendor(&e).unwrap_or_else(|| panic!("{}: {sql}: {:?}", t.id, e.kind)),
    }
}

/// Runs `statements` as `target`'s account when dropped (a test's clean-up, also on panic),
/// on a runtime of its own: a drop cannot await.
struct Cleanup {
    target: DockerTarget,
    statements: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let target = self.target.clone();
        let statements = std::mem::take(&mut self.statements);
        let done = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("a clean-up runtime");
            rt.block_on(async {
                let mut c = connect(&target).await;
                for sql in &statements {
                    let _ = c.execute(sql, simple()).await;
                }
                Box::new(c).close().await;
            });
        })
        .join();
        if done.is_err() && !std::thread::panicking() {
            panic!("{}: clean-up failed", self.target.id);
        }
    }
}

/// Codes that mean "no such function" to the probe (`ER_SP_DOES_NOT_EXIST`,
/// `ER_FUNC_INEXISTENT_NAME_COLLISION`).
const NO_SUCH_FUNCTION: [i64; 2] = [1305, 1630];
/// `ER_PROCACCESS_DENIED_ERROR`: what a missing function answered without EXECUTE on the
/// default database (C-1, run 44505051940).
const EXECUTE_DENIED: i64 = 1370;

#[tokio::test]
#[ignore = "docker"]
async fn hv_catalog_probe_tells_a_missing_function_from_a_builtin() {
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let cat = t.as_catalog().expect("a MySQL-family target");
        let mut c = connect(&cat).await;
        let db = ok(&mut c, &cat, "SELECT DATABASE()").await;
        assert_eq!(
            db[0][0].as_deref(),
            Some(CATALOG_PROBE_DATABASE),
            "{}",
            t.id
        );
        // The sentinel: a name no server has must come back absent, never "denied".
        let code = refused(
            &mut c,
            &cat,
            "PREPARE hv_m4_probe FROM 'SELECT hv_m4_t01c_no_such_function()'",
        )
        .await;
        assert_ne!(code, EXECUTE_DENIED, "{}: the probe cannot tell", t.id);
        assert!(NO_SUCH_FUNCTION.contains(&code), "{}: {code}", t.id);
        // A built-in prepares (nothing runs) ...
        ok(&mut c, &cat, "PREPARE hv_m4_probe FROM 'SELECT NOW()'").await;
        ok(&mut c, &cat, "DEALLOCATE PREPARE hv_m4_probe").await;
        // ... and one called with the wrong argument count is present too (1582).
        let code = refused(&mut c, &cat, "PREPARE hv_m4_probe FROM 'SELECT ABS()'").await;
        assert_eq!(code, 1582, "{}", t.id);
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn hv_catalog_reads_system_schemas_and_writes_nothing() {
    if !docker_enabled() {
        return;
    }
    for t in mysql_family() {
        let cat = t.as_catalog().expect("a MySQL-family target");
        let mut c = connect(&cat).await;
        let users = ok(&mut c, &cat, "SELECT COUNT(*) FROM mysql.user").await;
        assert!(users[0][0].is_some(), "{}", t.id);
        ok(
            &mut c,
            &cat,
            "SELECT COUNT(*) FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = 'sys'",
        )
        .await;
        // No write anywhere, and nothing of the fixture databases.
        assert_eq!(
            refused(
                &mut c,
                &cat,
                "CREATE TABLE hv_catalog_probe.hvm4_t01c_t (a int)"
            )
            .await,
            1142,
            "{}",
            t.id
        );
        assert_eq!(
            refused(&mut c, &cat, "SELECT * FROM hv_test.hv_probe").await,
            1142,
            "{}",
            t.id
        );
        let seen: Vec<String> = ok(&mut c, &cat, "SHOW DATABASES")
            .await
            .into_iter()
            .filter_map(|r| r.into_iter().next().flatten())
            .collect();
        for fixture in ["hv_test", "orders", "orders_archive"] {
            assert!(!seen.iter().any(|d| d == fixture), "{}: {seen:?}", t.id);
        }
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn hv_test_owns_hvm4_databases_with_functions_and_triggers() {
    if !docker_enabled() {
        return;
    }
    const DB: &str = "hvm4_t01c_owned";
    for t in mysql_family() {
        let _cleanup = Cleanup {
            target: t.clone(),
            statements: vec![format!("DROP DATABASE IF EXISTS {DB}")],
        };
        let mut c = connect(&t).await;
        ok(&mut c, &t, &format!("DROP DATABASE IF EXISTS {DB}")).await;
        ok(&mut c, &t, &format!("CREATE DATABASE {DB}")).await;
        ok(&mut c, &t, &format!("CREATE TABLE {DB}.t (a int)")).await;
        // Binary logging is on in every image: without --log-bin-trust-function-creators
        // (MySQL) this is error 1419.
        ok(
            &mut c,
            &t,
            &format!("CREATE FUNCTION {DB}.f() RETURNS int DETERMINISTIC RETURN 1"),
        )
        .await;
        ok(
            &mut c,
            &t,
            &format!("CREATE TRIGGER {DB}.tr BEFORE INSERT ON {DB}.t FOR EACH ROW SET NEW.a = 1"),
        )
        .await;
        let one = ok(&mut c, &t, &format!("SELECT {DB}.f()")).await;
        assert_eq!(one[0][0].as_deref(), Some("1"), "{}", t.id);
        ok(&mut c, &t, &format!("DROP DATABASE {DB}")).await;
        Box::new(c).close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn hv_fixture_sets_the_time_zone_and_manages_hvm4_accounts_only() {
    if !docker_enabled() {
        return;
    }
    const DB: &str = "hvm4_t01c_grants";
    const USER: &str = "'hvm4_t01c_user'@'%'";
    for t in mysql_family() {
        let fx = t.as_fixture().expect("a MySQL-family target");
        let _cleanup = Cleanup {
            target: fx.clone(),
            statements: vec![
                format!("DROP USER IF EXISTS {USER}"),
                format!("DROP DATABASE IF EXISTS {DB}"),
            ],
        };
        let mut c = connect(&fx).await;
        // The privilege, without changing the server: the global zone set to itself.
        ok(&mut c, &fx, "SET GLOBAL time_zone = @@GLOBAL.time_zone").await;
        ok(&mut c, &fx, &format!("DROP USER IF EXISTS {USER}")).await;
        ok(&mut c, &fx, &format!("DROP DATABASE IF EXISTS {DB}")).await;
        ok(&mut c, &fx, &format!("CREATE DATABASE {DB}")).await;
        // Not through `ok`: its failure message would echo the statement and its password.
        let create = format!("CREATE USER {USER} IDENTIFIED BY '{}'", fx.password);
        if let Err(e) = c.execute(&create, simple()).await {
            panic!("{}: CREATE USER: {:?} {:?}", t.id, e.kind, vendor(&e));
        }
        ok(&mut c, &fx, &format!("GRANT SELECT ON {DB}.* TO {USER}")).await;
        let grants = ok(&mut c, &fx, &format!("SHOW GRANTS FOR {USER}")).await;
        assert!(
            grants
                .iter()
                .any(|r| r[0].as_deref().is_some_and(|g| g.contains(DB))),
            "{}: {grants:?}",
            t.id
        );
        ok(&mut c, &fx, &format!("REVOKE SELECT ON {DB}.* FROM {USER}")).await;
        // Nothing beyond its two jobs: no fixture data, no grant on a fixture database.
        assert_eq!(
            refused(&mut c, &fx, "SELECT * FROM hv_test.hv_probe").await,
            1142,
            "{}",
            t.id
        );
        refused(&mut c, &fx, &format!("GRANT SELECT ON hv_test.* TO {USER}")).await;
        ok(&mut c, &fx, &format!("DROP USER {USER}")).await;
        ok(&mut c, &fx, &format!("DROP DATABASE {DB}")).await;
        Box::new(c).close().await;
    }
}
