//! M3b Task 7 (R-M3-67): the MySQL/MariaDB type matrix golden against the Docker matrix:
//! every case of `tests/data/types_mysql.sql` in both protocols on every MySQL-family line
//! (`mysql84` on `pr`; `mysql80`, `mariadb106/1011/114` too on `full`): the header label from
//! the native type (ledger 246), the kind (`BIT(n)` a bit string, `GEOMETRY` typed; P-M3b-26),
//! the raw text (shortest round-trip floats, ledger 177), and the session `time_zone` a
//! `TIMESTAMP` column carries (ledger 194). `#[ignore = "docker"]`; run with
//! `<main>/target/docker/run-docker-tests.sh --set full -p hacivat-driver-mysql --test docker_types_m3b`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    DriverConnect, Endpoint, ExecOpts, InsecureAuth, Protocol, QueryEvent, QuerySpec, RawConn,
};
use hacivat_driver_mysql::{MySqlConn, connect_conn};
use hacivat_net::{
    AddressClass, Target, TlsMode, TlsParams, TlsSettings, build_tls_params, direct,
    install_crypto_provider,
};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, targets};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text, type_label};
use hacivat_values::{ColumnMeta, LogicalType, NativeType, Value};
use secrecy::SecretString;

const TYPES: &str = include_str!("data/types_mysql.sql");

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

fn request(t: &DockerTarget) -> DriverConnect {
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
        read_only: false,
        options: vec![],
        connect_timeout: secs(20),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::Allow,
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

/// One case of the data file.
struct Case {
    line: usize,
    column: String,
    label: String,
    kind: String,
    raw: String,
}

fn setup() -> Vec<&'static str> {
    TYPES
        .lines()
        .filter(|l| !l.starts_with("--") && !l.starts_with('=') && l.ends_with(';'))
        .map(|l| l.trim_end_matches(';'))
        .collect()
}

fn cases() -> Vec<Case> {
    TYPES
        .lines()
        .enumerate()
        .filter_map(|(i, l)| Some((i, l.strip_prefix("= ")?)))
        .map(|(i, l)| {
            let parts: Vec<&str> = l.split(" | ").collect();
            let [column, label, kind, raw] = parts[..] else {
                panic!("line {}: four fields", i + 1);
            };
            Case {
                line: i + 1,
                column: column.to_string(),
                label: label.to_string(),
                kind: kind.to_string(),
                raw: raw.to_string(),
            }
        })
        .collect()
}

fn kind_of(l: &LogicalType) -> &'static str {
    match l {
        LogicalType::BitString { .. } => "bit",
        LogicalType::Geometry { .. } => "geometry",
        _ => "-",
    }
}

/// The one cell of `sql` through the typed path, and its column.
async fn typed_cell(c: &mut MySqlConn, sql: &str) -> (ColumnMeta, Value) {
    c.query(spec(sql))
        .await
        .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
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
        schema.columns[0].clone(),
        cell.unwrap_or_else(|| panic!("{sql}: no row")),
    )
}

fn raw_of(col: &ColumnMeta, v: &Value, engine: DbmsKind) -> String {
    let facts = ColumnFacts {
        logical: &col.logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Hex,
        engine,
    };
    match raw_text(v.view(), &facts) {
        RawText::Text(t) => t.into_owned(),
        RawText::Null => "NULL".into(),
    }
}

fn time_zone_of(native: &NativeType) -> Option<Arc<str>> {
    match native {
        NativeType::MySql { time_zone, .. } => time_zone.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_type_goldens_80_84_and_mariadb() {
    let cases = cases();
    assert!(cases.len() >= 30, "{} cases", cases.len());
    let lines = mysql_family();
    // The full set runs every line the plan names.
    if std::env::var("HACIVAT_DOCKER_SET").as_deref() == Ok("full") {
        for id in [
            "mysql80",
            "mysql84",
            "mariadb106",
            "mariadb1011",
            "mariadb114",
        ] {
            assert!(lines.iter().any(|t| t.id == id), "{id} not in the full set");
        }
    }
    for t in lines {
        let id = &t.id;
        let mut c = connect_conn(request(&t))
            .await
            .unwrap_or_else(|e| panic!("{id}: {:?}", e.kind));
        // The zone the connection read at connect, as the server reports it now.
        let zone = c
            .execute("SELECT @@session.time_zone", ExecOpts::values(1))
            .await
            .unwrap_or_else(|e| panic!("{id}: {:?}", e.kind))
            .rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next().flatten())
            .unwrap_or_else(|| panic!("{id}: no time_zone"));
        for sql in setup() {
            c.batch(sql)
                .await
                .unwrap_or_else(|e| panic!("{id}: {sql}: {:?}", e.kind));
        }
        for text_protocol in [false, true] {
            c.force_text_protocol(text_protocol);
            let protocol = if text_protocol { "text" } else { "binary" };
            for case in &cases {
                let at = format!("{id} {protocol} line {}: {}", case.line, case.column);
                let sql = format!("SELECT {} FROM hv_t07_types WHERE id = 1", case.column);
                let (col, value) = typed_cell(&mut c, &sql).await;
                assert_eq!(type_label(&col.native), case.label, "{at}");
                assert_eq!(kind_of(&col.logical), case.kind, "{at}: {:?}", col.logical);
                assert_eq!(raw_of(&col, &value, t.engine), case.raw, "{at}: {value:?}");
            }
            // A TIMESTAMP column names the session zone; a DATETIME one names none.
            let sql = "SELECT ts3, dt6 FROM hv_t07_types WHERE id = 1";
            c.query(spec(sql)).await.unwrap();
            let QueryEvent::ResultStart(schema) = c.next_event().await.unwrap() else {
                panic!("{id}: ResultStart first");
            };
            while !matches!(c.next_event().await.unwrap(), QueryEvent::Done(_)) {}
            assert_eq!(
                time_zone_of(&schema.columns[0].native).as_deref(),
                Some(zone.as_str()),
                "{id} {protocol}"
            );
            assert_eq!(
                type_label(&schema.columns[0].native),
                "timestamp(3)",
                "{id}"
            );
            assert_eq!(time_zone_of(&schema.columns[1].native), None, "{id}");
        }
        c.force_text_protocol(false);
        Box::new(c).close().await;
    }
}
