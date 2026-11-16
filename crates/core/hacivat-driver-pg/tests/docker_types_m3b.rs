//! M3b Task 7 (R-M3-67): the PostgreSQL type matrix golden against the Docker matrix, one
//! round trip per case on every PostgreSQL line (14 to 18 on `full`, pg17 on `pr`). Every case
//! of `tests/data/types_pg.sql` runs through the typed path (binary for the decoded types); its
//! logical kind and raw text must equal the checked-in expectation and the server's own
//! text of the same expression (its output function, over the simple query protocol).
//! `#[ignore = "docker"]`; run with
//! `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-driver-pg --test docker_types_m3b`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryEvent, QuerySpec, RawConn,
};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, targets};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text};
use hacivat_values::{LogicalType, Value};
use secrecy::SecretString;

const TYPES: &str = include_str!("data/types_pg.sql");

const SETUP: &str = "\
    SET TIME ZONE 'UTC';
    SET lc_monetary TO 'C';
    SET DateStyle TO ISO, MDY;
    SET IntervalStyle TO postgres;
    CREATE TYPE pg_temp.hv_t07_mood AS ENUM ('sad', 'ok', 'happy');
    CREATE TYPE pg_temp.hv_t07_pair AS (a int4, b text);";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn request(t: &DockerTarget) -> DriverConnect {
    let addr: SocketAddr = format!("{}:{}", t.host, t.port).parse().unwrap();
    let factory = direct(
        Target::Tcp {
            host: t.host.clone(),
            port: t.port,
        },
        vec![addr],
        secs(5),
    );
    let settings = TlsSettings {
        mode: TlsMode::Disable,
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
        user: Some(t.user.clone()),
        password: Some(SecretString::from(t.password.clone())),
        database: Some(t.database.clone()),
        application_name: "Hacivat tests".into(),
        read_only: false,
        options: vec![],
        connect_timeout: secs(10),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
    }
}

/// One server per PostgreSQL major from 14 to 18 in the active set (without TLS).
fn version_targets() -> Vec<DockerTarget> {
    let mut majors = BTreeSet::new();
    let list: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| t.major.parse::<u32>().is_ok_and(|m| (14..=18).contains(&m)))
        .filter(|t| t.tls_default == TlsDefault::None)
        .filter(|t| majors.insert(t.major.clone()))
        .collect();
    assert!(
        !list.is_empty(),
        "no PostgreSQL 14-18 target in the active set"
    );
    list
}

/// One case of the data file.
struct Case {
    line: usize,
    expr: String,
    kind: String,
    expected: String,
}

fn cases() -> Vec<Case> {
    TYPES
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.starts_with("--"))
        .map(|(i, l)| {
            let (expr, note) = l
                .rsplit_once(" -- ")
                .unwrap_or_else(|| panic!("line {}: no ` -- `", i + 1));
            let (kind, expected) = note
                .split_once(" =>")
                .unwrap_or_else(|| panic!("line {}: no ` =>`", i + 1));
            Case {
                line: i + 1,
                expr: expr.trim().to_string(),
                kind: kind.trim().to_string(),
                expected: expected.strip_prefix(' ').unwrap_or(expected).to_string(),
            }
        })
        .collect()
}

fn kind_of(l: &LogicalType) -> &'static str {
    match l {
        LogicalType::Array(_) => "array",
        LogicalType::Network(_) => "network",
        LogicalType::BitString { .. } => "bit",
        LogicalType::Money => "money",
        LogicalType::Xml => "xml",
        LogicalType::Composite(_) => "composite",
        LogicalType::Range(_) => "range",
        LogicalType::Geometry { .. } => "geometry",
        LogicalType::Opaque { .. } => "opaque",
        _ => "other",
    }
}

fn spec(sql: &str) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable: false,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

/// The one cell of `SELECT <expr>` through the typed path, and its column's logical type.
async fn typed_cell(c: &mut PgConn, sql: &str) -> (LogicalType, Value) {
    c.query(spec(sql))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    let QueryEvent::ResultStart(schema) = c.next_event().await.unwrap() else {
        panic!("{sql}: ResultStart first");
    };
    let mut cell = None;
    loop {
        match c.next_event().await.unwrap() {
            QueryEvent::Rows(chunk) => cell = Some(chunk.cell(0, 0).to_value()),
            QueryEvent::ResultEnd(_) => break,
            other => panic!("{sql}: {other:?}"),
        }
    }
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Done(_)));
    (
        schema.columns[0].logical.clone(),
        cell.unwrap_or_else(|| panic!("{sql}: no row")),
    )
}

/// The server's own text of `expr`: its type's output function, as a simple query (text
/// format) returns it. Not a `::text` cast, which for `inet` is `text(inet)` (always `/32`).
async fn server_text(c: &mut PgConn, expr: &str) -> String {
    let sql = format!("SELECT {expr}");
    let s = c
        .execute(&sql, ExecOpts::values(1))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
    s.rows
        .into_iter()
        .next()
        .and_then(|r| r.into_iter().next().flatten())
        .unwrap_or_else(|| panic!("{sql}: no text"))
}

fn raw_of(logical: &LogicalType, v: &Value) -> String {
    let facts = ColumnFacts {
        logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Hex,
        engine: DbmsKind::Postgres,
    };
    match v {
        Value::Text(_) | Value::Xml(_) | Value::Opaque(_) => match raw_text(v.view(), &facts) {
            RawText::Text(t) => t.into_owned(),
            RawText::Null => panic!("NULL"),
        },
        other => panic!("not text: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_type_goldens_pg14_to_18() {
    let cases = cases();
    assert!(cases.len() >= 60, "{} cases", cases.len());
    let lines = version_targets();
    // The full set runs every line the plan names.
    if std::env::var("HACIVAT_DOCKER_SET").as_deref() == Ok("full") {
        let majors: Vec<&str> = lines.iter().map(|t| t.major.as_str()).collect();
        assert_eq!(majors, ["14", "15", "16", "17", "18"]);
    }
    for t in lines {
        let id = &t.id;
        let mut c = connect_conn(request(&t))
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        c.batch(SETUP)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        for case in &cases {
            let at = format!("{id} line {}: {}", case.line, case.expr);
            let (logical, value) = typed_cell(&mut c, &format!("SELECT {}", case.expr)).await;
            assert_eq!(kind_of(&logical), case.kind, "{at}: {logical:?}");
            let raw = raw_of(&logical, &value);
            assert_eq!(raw, case.expected, "{at}");
            assert_eq!(
                raw,
                server_text(&mut c, &case.expr).await,
                "{at}: server text"
            );
        }
        Box::new(c).close().await;
    }
}
