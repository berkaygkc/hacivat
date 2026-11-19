//! M3b Task 13 (HV-EXEC-03 driver half; R-M3-60, R-M3-66, R-M3-77, R-M3-79) on every
//! PostgreSQL line of the active set (pg17 on `pr`, 14 to 18 on `full`): text-format binds (a
//! hostile value stored and read back exactly, the table intact; the server parsing each
//! parameter by its type), the parameter count, `RAISE` notice text as `QueryEvent::Notice`
//! (capped, and before an exception), and Run as one over the simple protocol.
//! `#[ignore = "docker"]`; run with
//! `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-driver-pg --test docker_typed_m3b`.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{
    CloseHow, CloseReport, DriverConnect, Endpoint, InsecureAuth, Notice, NoticeSeverity, Protocol,
    QueryDone, QueryEvent, QuerySpec, RawConn, RawError, RawErrorKind, ResultEnd, TxStatus,
};
use hacivat_driver_pg::{PgConn, connect_conn};
use hacivat_net::{AddressClass, Target, TlsMode, TlsSettings, build_tls_params, direct};
use hacivat_proto::DbmsKind;
use hacivat_testkit::{DockerTarget, TlsDefault, targets};
use hacivat_values::{BindValue, LogicalType, NativeType, ResultSchema, Value};
use secrecy::SecretString;

// ## m3b-t13-drivers

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

async fn connect(t: &DockerTarget) -> PgConn {
    match connect_conn(request(t)).await {
        Ok(c) => c,
        Err(e) => panic!("{}: {:?}", t.id, e.kind),
    }
}

fn sqlstate(e: &RawError) -> Option<&str> {
    e.server.as_ref().and_then(|s| s.sqlstate.as_deref())
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

fn int(s: &str) -> BindValue {
    typed(
        LogicalType::Int {
            bits: 32,
            signed: true,
        },
        s,
    )
}

/// One result of a query.
struct Set {
    schema: Arc<ResultSchema>,
    rows: Vec<Vec<Value>>,
    end: ResultEnd,
}

/// Every result, notice and the end of a query; `Err` is the query's error with the notices
/// that came before it.
async fn run(
    t: &DockerTarget,
    c: &mut PgConn,
    s: QuerySpec<'_>,
) -> Result<(Vec<Set>, Vec<Notice>, QueryDone), (RawError, Vec<Notice>)> {
    // A Bind the server refuses (a parameter its type cannot parse) fails `query` itself.
    c.query(s).await.map_err(|e| (e, Vec::new()))?;
    let mut sets = Vec::new();
    let mut notices = Vec::new();
    let mut current: Option<(Arc<ResultSchema>, Vec<Vec<Value>>)> = None;
    loop {
        match c.next_event().await {
            Ok(QueryEvent::ResultStart(schema)) => current = Some((schema, Vec::new())),
            Ok(QueryEvent::Rows(chunk)) => {
                let (schema, rows) = current.as_mut().expect("rows inside a result");
                let width = schema.columns.len() as u32;
                for r in 0..chunk.rows() {
                    rows.push(
                        (0..width)
                            .map(|col| chunk.cell(r, col).to_value())
                            .collect(),
                    );
                }
            }
            Ok(QueryEvent::ResultEnd(end)) => {
                let (schema, rows) = current.take().expect("an end after a start");
                sets.push(Set { schema, rows, end });
            }
            Ok(QueryEvent::Notice(n)) => notices.push(n),
            Ok(QueryEvent::Done(done)) => return Ok((sets, notices, done)),
            Ok(QueryEvent::Suspended) => panic!("{}: suspended", t.id),
            Err(e) => return Err((e, notices)),
        }
    }
}

async fn ok_run(
    t: &DockerTarget,
    c: &mut PgConn,
    s: QuerySpec<'_>,
) -> (Vec<Set>, Vec<Notice>, QueryDone) {
    match run(t, c, s).await {
        Ok(r) => r,
        Err((e, _)) => panic!("{}: {:?}: {:?} {:?}", t.id, s.sql, e.kind, sqlstate(&e)),
    }
}

fn txt(s: &str) -> Value {
    Value::Text(Arc::from(s))
}

#[tokio::test]
#[ignore = "docker"]
async fn value_with_quote_is_bound_not_spliced() {
    let hostile = "x'); DROP TABLE hv_m3b_t13_q; --";
    for t in version_targets() {
        let mut c = connect(&t).await;
        assert!(c.typed_caps().binds, "{}", t.id);
        c.batch("CREATE TEMP TABLE hv_m3b_t13_q (a int4, b text, c bytea, d timestamptz)")
            .await
            .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        let binds = [
            int("7"),
            text(hostile),
            typed(LogicalType::Bytes { max: None }, "\\x00ff"),
            text("2024-02-29 12:34:56+03"),
        ];
        let (sets, _, _) = ok_run(
            &t,
            &mut c,
            spec("INSERT INTO hv_m3b_t13_q VALUES ($1, $2, $3, $4)", &binds),
        )
        .await;
        assert_eq!(sets[0].end.rows_affected, Some(1), "{}", t.id);
        // Read back exactly; each parameter was parsed by its column's type.
        let (sets, _, _) = ok_run(
            &t,
            &mut c,
            spec(
                "SELECT a, b, c, d = '2024-02-29 09:34:56Z'::timestamptz FROM hv_m3b_t13_q",
                &[],
            ),
        )
        .await;
        assert_eq!(
            sets[0].rows,
            vec![vec![
                Value::Int(7),
                txt(hostile),
                Value::Bytes(Arc::from(&[0x00_u8, 0xff][..])),
                Value::Bool(true),
            ]],
            "{}",
            t.id
        );
        // The table is still there, and a tautology bound as a value matches nothing.
        let binds = [text("x' OR '1'='1")];
        let (sets, _, _) = ok_run(
            &t,
            &mut c,
            spec("SELECT count(*) FROM hv_m3b_t13_q WHERE b = $1", &binds),
        )
        .await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(0)]], "{}", t.id);
        // A NULL bind.
        let binds = [BindValue::Null];
        let (sets, _, _) = ok_run(&t, &mut c, spec("SELECT $1::int4 IS NULL", &binds)).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Bool(true)]], "{}", t.id);
        // A value the parameter's type refuses: the server's error, nothing stored.
        let binds = [int("seven"), text("y"), BindValue::Null, BindValue::Null];
        let Err((e, _)) = run(
            &t,
            &mut c,
            spec("INSERT INTO hv_m3b_t13_q VALUES ($1, $2, $3, $4)", &binds),
        )
        .await
        else {
            panic!("{}: an int4 parameter took 'seven'", t.id);
        };
        assert_eq!(sqlstate(&e), Some("22P02"), "{}", t.id);
        let (sets, _, _) = ok_run(&t, &mut c, spec("SELECT count(*) FROM hv_m3b_t13_q", &[])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(1)]], "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_param_count_mismatch_is_refused_before_send() {
    for t in version_targets() {
        let mut c = connect(&t).await;
        for binds in [vec![], vec![int("1"), int("2")]] {
            let e = c.query(spec("SELECT $1::int4", &binds)).await.unwrap_err();
            assert_eq!(e.kind, RawErrorKind::Unsupported, "{}", t.id);
        }
        let (sets, _, _) = ok_run(&t, &mut c, spec("SELECT $1::int4 + 1", &[int("41")])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(42)]], "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_raise_notice_text_reaches_the_event() {
    for t in version_targets() {
        let mut c = connect(&t).await;
        let (_, notices, done) = ok_run(
            &t,
            &mut c,
            spec(
                "DO $$ BEGIN RAISE NOTICE 'hv notice %', 42; RAISE WARNING 'hv warning'; \
                 RAISE NOTICE '%', repeat('é', 10000); END $$",
                &[],
            ),
        )
        .await;
        assert_eq!(notices.len(), 3, "{}", t.id);
        assert_eq!(
            (
                notices[0].severity,
                notices[0].code.as_deref(),
                &*notices[0].text
            ),
            (NoticeSeverity::Notice, Some("00000"), "hv notice 42"),
            "{}",
            t.id
        );
        assert_eq!(
            (
                notices[1].severity,
                notices[1].code.as_deref(),
                &*notices[1].text
            ),
            (NoticeSeverity::Warning, Some("01000"), "hv warning"),
            "{}",
            t.id
        );
        // 20,000 bytes of text, cut at 16 KiB on a character boundary.
        assert_eq!(notices[2].text.len(), 16 * 1024, "{}", t.id);
        assert!(notices[2].text.chars().all(|ch| ch == 'é'), "{}", t.id);
        assert_eq!(done.warnings, 1, "{}", t.id);

        // At most 100 of a statement's notices.
        let (_, notices, _) = ok_run(
            &t,
            &mut c,
            spec(
                "DO $$ BEGIN FOR i IN 1..150 LOOP RAISE NOTICE 'n%', i; END LOOP; END $$",
                &[],
            ),
        )
        .await;
        let texts: Vec<String> = notices.iter().map(|n| n.text.to_string()).collect();
        let want: Vec<String> = (1..=100).map(|i| format!("n{i}")).collect();
        assert_eq!(texts, want, "{}", t.id);

        // Notices before an exception come out before the error (PL/pgSQL debugging).
        let Err((e, notices)) = run(
            &t,
            &mut c,
            spec(
                "DO $$ BEGIN RAISE NOTICE 'x = %', 41; RAISE EXCEPTION 'hv boom'; END $$",
                &[],
            ),
        )
        .await
        else {
            panic!("{}: the exception was lost", t.id);
        };
        assert_eq!(sqlstate(&e), Some("P0001"), "{}", t.id);
        let texts: Vec<&str> = notices.iter().map(|n| &*n.text).collect();
        assert_eq!(texts, ["x = 41"], "{}", t.id);

        // The next query starts with none of them.
        let (_, notices, _) = ok_run(&t, &mut c, spec("SELECT 1", &[])).await;
        assert!(notices.is_empty(), "{}", t.id);
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_simple_protocol_yields_one_result_per_statement_untyped() {
    let sql = "SELECT 1 AS a, 'x'::text AS b; CREATE TEMP TABLE hv_m3b_t13_s (n int4); \
               INSERT INTO hv_m3b_t13_s VALUES (1), (2), (3); \
               SELECT ARRAY[1, 2]::int4[] AS arr, '192.168.0.1/24'::inet AS ip; \
               DO $$ BEGIN RAISE NOTICE 'as one'; END $$";
    for t in version_targets() {
        let mut c = connect(&t).await;
        let mut s = spec(sql, &[]);
        s.protocol = Protocol::Simple;
        s.max_result_sets = 100;
        let (sets, notices, done) = ok_run(&t, &mut c, s).await;
        assert_eq!(sets.len(), 5, "{}", t.id);
        for set in &sets {
            for col in set.schema.columns.iter() {
                assert_eq!(
                    col.logical,
                    LogicalType::Text {
                        max_chars: None,
                        fixed: false
                    },
                    "{}",
                    t.id
                );
                assert_eq!(col.native, NativeType::Unknown, "{}", t.id);
            }
        }
        assert_eq!(sets[0].rows, vec![vec![txt("1"), txt("x")]], "{}", t.id);
        assert!(sets[1].schema.columns.is_empty(), "{}", t.id);
        assert_eq!(sets[2].end.rows_affected, Some(3), "{}", t.id);
        // The server's text, never read as binary (T7 m-10).
        assert_eq!(
            sets[3].rows,
            vec![vec![txt("{1,2}"), txt("192.168.0.1/24")]],
            "{}",
            t.id
        );
        let texts: Vec<&str> = notices.iter().map(|n| &*n.text).collect();
        assert_eq!(texts, ["as one"], "{}", t.id);
        assert_eq!(done.skipped_result_sets, 0, "{}", t.id);

        // Past the cap: kept up to it, the rest run, read and counted.
        let mut s = spec(
            "SELECT 1; SELECT 2; INSERT INTO hv_m3b_t13_s VALUES (4); SELECT 4",
            &[],
        );
        s.protocol = Protocol::Simple;
        s.max_result_sets = 2;
        let (sets, _, done) = ok_run(&t, &mut c, s).await;
        assert_eq!(sets.len(), 2, "{}", t.id);
        assert_eq!(done.skipped_result_sets, 2, "{}", t.id);
        let (sets, _, _) = ok_run(&t, &mut c, spec("SELECT count(*) FROM hv_m3b_t13_s", &[])).await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(4)]], "{}", t.id);
    }
}

// ## m3b-t13-drivers (fix round 1, I-1)

/// Closes a Run as one of `<sleep 30 s>; DELETE …` while its first statement sleeps: the
/// driver cancels at once, so the answer is 57014 and the DELETE never runs. A driver that read
/// on without cancelling would answer only after the sleep, with the DELETE run. (The backend
/// flushes its output only at a statement's end or on a notice, so the sleeping statement
/// announces itself with a `RAISE NOTICE` first: the Stop then provably lands mid-sleep.)
async fn stop_during_sleep(t: &DockerTarget, c: &mut PgConn) {
    let mut s = spec(
        "DO $$ BEGIN RAISE NOTICE 'hv sleeping'; PERFORM pg_sleep(30); END $$; \
         DELETE FROM hv_m3b_t13_stop",
        &[],
    );
    s.protocol = Protocol::Simple;
    s.max_result_sets = 100;
    c.query(s)
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
    match c.next_event().await {
        Ok(QueryEvent::Notice(n)) if &*n.text == "hv sleeping" => {}
        other => panic!("{}: {other:?}", t.id),
    }
    let e = c.close_query(CloseHow::Now).await.unwrap_err();
    assert_eq!(sqlstate(&e), Some("57014"), "{}: {e:?}", t.id);
    assert_eq!(
        c.take_close_report(),
        Some(CloseReport {
            statements_ran: 0,
            rest_not_run: true
        }),
        "{}",
        t.id
    );
}

#[tokio::test]
#[ignore = "docker"]
async fn stop_during_run_as_one_never_runs_the_rest() {
    for t in version_targets() {
        let mut c = connect(&t).await;
        c.batch(
            "CREATE TEMP TABLE hv_m3b_t13_stop (n int4); \
             INSERT INTO hv_m3b_t13_stop VALUES (1), (2), (3)",
        )
        .await
        .unwrap();
        // Inside a user transaction (a Manual-mode console): failed, the user's to end.
        c.batch("BEGIN").await.unwrap();
        stop_during_sleep(&t, &mut c).await;
        assert_eq!(c.tx_status(), TxStatus::Failed, "{}", t.id);
        c.batch("ROLLBACK").await.unwrap();
        // In autocommit: the DELETE did not run, so the rows are all there.
        stop_during_sleep(&t, &mut c).await;
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        let (sets, _, _) = ok_run(
            &t,
            &mut c,
            spec("SELECT count(*) FROM hv_m3b_t13_stop", &[]),
        )
        .await;
        assert_eq!(sets[0].rows, vec![vec![Value::Int(3)]], "{}", t.id);
    }
}

// ## m4-pgcancel

/// Drops table `name` on `t` when it goes, also on panic, over a connection of its own (the
/// test's may be mid-statement). Needs the multi-thread runtime: it blocks on a scoped thread.
struct DropTable {
    t: DockerTarget,
    name: String,
}

impl Drop for DropTable {
    fn drop(&mut self) {
        let t = self.t.clone();
        let sql = format!("DROP TABLE IF EXISTS {}", self.name);
        let rt = tokio::runtime::Handle::current();
        std::thread::scope(|s| {
            s.spawn(|| {
                rt.block_on(async {
                    if let Ok(mut c) = connect_conn(request(&t)).await {
                        let _ = c.batch(&sql).await;
                        Box::new(c).close().await;
                    }
                });
            });
        });
    }
}

/// [`stop_during_sleep`] whose Run as one would delete from `table`.
async fn stop_during_sleep_on(t: &DockerTarget, c: &mut PgConn, table: &str) {
    let sql = format!(
        "DO $$ BEGIN RAISE NOTICE 'hv sleeping'; PERFORM pg_sleep(30); END $$; \
         DELETE FROM {table}"
    );
    let mut s = spec(&sql, &[]);
    s.protocol = Protocol::Simple;
    s.max_result_sets = 100;
    c.query(s)
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
    match c.next_event().await {
        Ok(QueryEvent::Notice(n)) if &*n.text == "hv sleeping" => {}
        other => panic!("{}: {other:?}", t.id),
    }
    let e = c.close_query(CloseHow::Now).await.unwrap_err();
    assert_eq!(sqlstate(&e), Some("57014"), "{}: {e:?}", t.id);
    assert_eq!(
        c.take_close_report(),
        Some(CloseReport {
            statements_ran: 0,
            rest_not_run: true
        }),
        "{}",
        t.id
    );
}

/// CI run 44595239297 (pg15): the statement right after a Stop of a Run as one failed with
/// 57014, a cancel of the Stop landing late. The close answers only once its cancel ended (the
/// server acted on it), so the next statement, run at once and long enough (100 ms) for a late
/// cancel to land in, always completes. Several rounds per server. A probabilistic reproducer
/// (a late signal needs the canceller preempted between its two kills); the deterministic
/// proof is `typed_m3b.rs`'s scripted close tests.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "docker"]
async fn a_stop_never_cancels_the_next_statement() {
    let name = format!("hvm4_pgcancel_stop_{}", std::process::id());
    for t in version_targets() {
        // Before the connection, so it drops after it: a lock the test's connection holds
        // goes with it first.
        let _drop = DropTable {
            t: t.clone(),
            name: name.clone(),
        };
        let mut c = connect(&t).await;
        c.batch(&format!(
            "CREATE TABLE {name} (n int4); INSERT INTO {name} VALUES (1), (2), (3)"
        ))
        .await
        .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.kind));
        let count = format!("SELECT count(*) FROM {name}, pg_sleep(0.1) AS s");
        for _ in 0..5 {
            stop_during_sleep_on(&t, &mut c, &name).await;
            let (sets, _, _) = ok_run(&t, &mut c, spec(&count, &[])).await;
            assert_eq!(sets[0].rows, vec![vec![Value::Int(3)]], "{}", t.id);
        }
        assert_eq!(c.tx_status(), TxStatus::Idle, "{}", t.id);
        Box::new(c).close().await;
    }
}
