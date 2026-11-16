//! M3b Task 13 (HV-EXEC-03 driver half; R-M3-60, R-M3-66, R-M3-77, R-M3-79) on every
//! MySQL-family line of the active set (`mysql84` on `pr`; `mysql80` and the MariaDB lines too
//! on `full`): binds through the binary protocol (a hostile value stored and read back exactly,
//! the table intact; integer hints as integers), a `CALL` of three result sets under the
//! result-set cap, Run as one (read-write only) and `SHOW WARNINGS` after `Done`.
//! `#[ignore = "docker"]`; run with
//! `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-mysql --test docker_typed_m3b`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, InsecureAuth, NoticeSeverity, Protocol, QueryDone, QueryEvent,
    QuerySpec, RawConn, RawError, RawErrorKind, ResultEnd,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{
    AddressClass, Target, TlsMode, TlsParams, TlsSettings, build_tls_params, direct,
    install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, targets};
use hacivat_values::{BindValue, LogicalType, Value};
use secrecy::SecretString;

// ## m3b-t13-drivers

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Every MySQL and MariaDB target of the active set.
fn mysql_family() -> Vec<DockerTarget> {
    let mut ts = targets(Some(DbmsKind::MySql));
    ts.extend(targets(Some(DbmsKind::MariaDb)));
    assert!(!ts.is_empty(), "no MySQL-family target in the selected set");
    ts
}

fn params(t: &DockerTarget) -> Option<TlsParams> {
    install_crypto_provider();
    let mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    let settings = TlsSettings {
        mode,
        ..TlsSettings::default()
    };
    build_tls_params(&settings, &t.host, t.port, AddressClass::Loopback, None)
        .unwrap_or_else(|e| panic!("{}: {e:?}", t.id))
}

fn request(t: &DockerTarget, read_only: bool) -> DriverConnect {
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
        tls: params(t),
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat tests".into(),
        read_only,
        options: vec![],
        connect_timeout: secs(20),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

async fn connect(t: &DockerTarget, read_only: bool) -> MySqlConn {
    match connect_conn(request(t, read_only)).await {
        Ok(c) => c,
        Err(e) => panic!("{}: {:?}", t.id, e.kind),
    }
}

fn vendor(e: &RawError) -> Option<i64> {
    e.server.as_ref().and_then(|s| s.vendor_code)
}

async fn batch(t: &DockerTarget, c: &mut MySqlConn, sql: &str) {
    if let Err(e) = c.batch(sql).await {
        panic!("{}: {sql}: {:?} {:?}", t.id, e.kind, vendor(&e));
    }
}

fn spec<'a>(sql: &'a str, binds: &'a [BindValue]) -> QuerySpec<'a> {
    QuerySpec {
        sql,
        binds,
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

fn typed(hint: LogicalType, text: &str) -> BindValue {
    BindValue::Typed {
        hint,
        text: Arc::from(text),
    }
}

fn text(s: &str) -> BindValue {
    typed(
        LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        s,
    )
}

fn int(signed: bool, s: &str) -> BindValue {
    typed(LogicalType::Int { bits: 64, signed }, s)
}

/// One result of a query.
struct Set {
    rows: Vec<Vec<Value>>,
    end: ResultEnd,
}

/// Every result of a query to `Done`.
async fn run(t: &DockerTarget, c: &mut MySqlConn, s: QuerySpec<'_>) -> (Vec<Set>, QueryDone) {
    if let Err(e) = c.query(s).await {
        panic!("{}: query {:?}: {:?} {:?}", t.id, s.sql, e.kind, vendor(&e));
    }
    let mut sets = Vec::new();
    let mut rows = Vec::new();
    let mut width = 0;
    loop {
        match c.next_event().await {
            Ok(QueryEvent::ResultStart(schema)) => {
                width = schema.columns.len() as u32;
                rows = Vec::new();
            }
            Ok(QueryEvent::Rows(chunk)) => {
                for r in 0..chunk.rows() {
                    rows.push(
                        (0..width)
                            .map(|col| chunk.cell(r, col).to_value())
                            .collect(),
                    );
                }
            }
            Ok(QueryEvent::ResultEnd(end)) => sets.push(Set {
                rows: std::mem::take(&mut rows),
                end,
            }),
            Ok(QueryEvent::Done(done)) => return (sets, done),
            Ok(other) => panic!("{}: {other:?}", t.id),
            Err(e) => panic!("{}: next_event: {:?} {:?}", t.id, e.kind, vendor(&e)),
        }
    }
}

fn txt(s: &str) -> Value {
    Value::Text(Arc::from(s))
}

#[tokio::test]
#[ignore = "docker"]
async fn value_with_quote_is_bound_not_spliced() {
    let hostile = "x'); DROP TABLE hv_m3b_t13_q; --";
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        assert!(c.typed_caps().binds, "{}", t.id);
        batch(
            &t,
            &mut c,
            "CREATE TEMPORARY TABLE hv_m3b_t13_q (a BIGINT, b VARCHAR(100)) DEFAULT CHARSET = utf8mb4",
        )
        .await;
        let binds = [int(true, "7"), text(hostile)];
        let (sets, _) = run(
            &t,
            &mut c,
            spec("INSERT INTO hv_m3b_t13_q VALUES (?, ?)", &binds),
        )
        .await;
        assert_eq!(sets[0].end.rows_affected, Some(1), "{}", t.id);
        let (sets, _) = run(&t, &mut c, spec("SELECT a, b FROM hv_m3b_t13_q", &[])).await;
        assert_eq!(
            sets[0].rows,
            vec![vec![Value::Int(7), txt(hostile)]],
            "{}",
            t.id
        );
        // A tautology bound as a value matches nothing.
        let binds = [text("x' OR '1'='1")];
        let (sets, _) = run(
            &t,
            &mut c,
            spec("SELECT count(*) FROM hv_m3b_t13_q WHERE b = ?", &binds),
        )
        .await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(0)]], "{}", t.id);
        // Integer hints arrive as integers: integer arithmetic (a text would make it DOUBLE),
        // the unsigned range, NULL.
        let binds = [
            int(true, "41"),
            int(false, "18446744073709551615"),
            BindValue::Null,
        ];
        let (sets, _) = run(&t, &mut c, spec("SELECT ? + 1, ? + 0, ? IS NULL", &binds)).await;
        assert_eq!(sets[0].rows[0][0], Value::Int(42), "{}", t.id);
        assert_eq!(sets[0].rows[0][1], Value::UInt(u64::MAX), "{}", t.id);
        assert_eq!(sets[0].rows[0][2], Value::Int(1), "{}", t.id);
        // A missing bind: prepared, never executed; the connection stays in step.
        let one = [int(true, "1")];
        let e = c.query(spec("SELECT ?, ?", &one)).await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Unsupported, "{}", t.id);
        let (sets, _) = run(&t, &mut c, spec("SELECT count(*) FROM hv_m3b_t13_q", &[])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(1)]], "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_call_three_sets() {
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        batch(
            &t,
            &mut c,
            "CREATE PROCEDURE IF NOT EXISTS hv_test.hv_m3b_t13_three() \
             BEGIN SELECT 1 AS n; SELECT 2 AS n; SELECT 3 AS n; END",
        )
        .await;
        // Within the cap: all three, in order.
        let mut s = spec("CALL hv_test.hv_m3b_t13_three()", &[]);
        s.max_result_sets = 100;
        let (sets, done) = run(&t, &mut c, s).await;
        let values: Vec<Vec<Vec<Value>>> = sets.iter().map(|s| s.rows.clone()).collect();
        assert_eq!(
            values,
            vec![
                vec![vec![Value::Int(1)]],
                vec![vec![Value::Int(2)]],
                vec![vec![Value::Int(3)]]
            ],
            "{}",
            t.id
        );
        assert_eq!(done.skipped_result_sets, 0, "{}", t.id);
        // Past the cap: kept up to it, the rest read and counted, never dropped silently.
        s.max_result_sets = 2;
        let (sets, done) = run(&t, &mut c, s).await;
        assert_eq!(sets.len(), 2, "{}", t.id);
        assert_eq!(done.skipped_result_sets, 1, "{}", t.id);
        // Run as one: one `COM_QUERY`, a set per statement.
        let mut as_one = spec("SELECT 1 AS a; SELECT 'b' AS b; SELECT 3 AS c", &[]);
        as_one.protocol = Protocol::Simple;
        as_one.max_result_sets = 100;
        let (sets, done) = run(&t, &mut c, as_one).await;
        let values: Vec<Vec<Vec<Value>>> = sets.iter().map(|s| s.rows.clone()).collect();
        assert_eq!(
            values,
            vec![
                vec![vec![Value::Int(1)]],
                vec![vec![txt("b")]],
                vec![vec![Value::Int(3)]]
            ],
            "{}",
            t.id
        );
        assert_eq!(done.skipped_result_sets, 0, "{}", t.id);
        batch(
            &t,
            &mut c,
            "DROP PROCEDURE IF EXISTS hv_test.hv_m3b_t13_three",
        )
        .await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_multi_statement_refused_on_read_only() {
    for t in mysql_family() {
        let mut c = connect(&t, true).await;
        let mut as_one = spec("SELECT 1; SELECT 2", &[]);
        as_one.protocol = Protocol::Simple;
        as_one.max_result_sets = 100;
        assert_eq!(
            c.query(as_one).await.unwrap_err().kind,
            RawErrorKind::Unsupported,
            "{}",
            t.id
        );
        // In step.
        let (sets, _) = run(&t, &mut c, spec("SELECT 1", &[])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(1)]], "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_show_warnings_after_done() {
    let sql = "SELECT CAST('12abc' AS SIGNED) AS n";
    for t in mysql_family() {
        let mut c = connect(&t, false).await;
        // No statement yet: nothing to read.
        assert!(c.warnings().await.unwrap().is_empty(), "{}", t.id);
        let (sets, done) = run(&t, &mut c, spec(sql, &[])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(12)]], "{}", t.id);
        assert_eq!(done.warnings, 1, "{}", t.id);
        let w = c
            .warnings()
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        assert_eq!(w.len(), 1, "{}", t.id);
        assert_eq!(w[0].severity, NoticeSeverity::Warning, "{}", t.id);
        assert_eq!(w[0].code.as_deref(), Some("1292"), "{}", t.id);
        assert!(w[0].text.contains("12abc"), "{}: {}", t.id, w[0].text);
        // A statement without warnings: none, and no round trip needed.
        run(&t, &mut c, spec("SELECT 1", &[])).await;
        assert!(c.warnings().await.unwrap().is_empty(), "{}", t.id);

        // Read-only: the count only (the guard's ROLLBACK follows at once).
        let mut ro = connect(&t, true).await;
        let (_, done) = run(&t, &mut ro, spec(sql, &[])).await;
        assert_eq!(done.warnings, 1, "{}", t.id);
        assert!(ro.warnings().await.unwrap().is_empty(), "{}", t.id);
    }
}
