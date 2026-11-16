//! M3b Task 28: conformance of M3b's execution over the Docker matrix (the E-rows of the M3b
//! gate: E2, E4, E5 and the Docker legs of E1/E8). Every case goes the app's M3b way
//! (`common::t28`):
//!
//! 1. a console document in the context the console read at bind;
//! 2. `resolve_units` (Execute3), or one unit sent as written (Run as one);
//! 3. `prepare_units`, every prompt answered by name (`prepare_units_with`);
//! 4. the real `ClassifyPool` over exactly the prepared bytes, in the session's own context
//!    (lexing modes, search path or current database, server version; the T24 carry);
//! 5. `gate` on the live connection, its `Confirm`/`Warn` answered as the dialogs answer;
//! 6. `ExecService::run`, a real session from a `ConnectionManager` (M2b's configuration), the
//!    typed driver;
//!
//! and an admin side channel checks what reached the server after each case (M3a T31's
//! `matrix::Side`).
//!
//! - **Lines.** `matrix::lines()`: PostgreSQL 14–18, MySQL 8.0 and 8.4, MariaDB 10.6, 10.11 and
//!   11.4 under `full` (pg17 and mysql84 under `pr`), from the targets' properties (never a
//!   list), and SQLite (a file per test) in every set. Each test asserts at its end that every
//!   line ran (`Table::finish`) and writes its engine × version × case rows to the Docker
//!   artifact `t28-<test>.tsv`. Per-engine differences are explicit expectations, never a skip.
//! - **The deadline.** The run service sends D-014's 5 minutes; the 2 s deadline legs send the
//!   same prepared units with the gate's verdicts straight to the session
//!   (`Con::run_with_deadline`, the T31 precedent).
//! - **Production writes** come from `ManagerConfig::m2b()` (production writable, as Task 26
//!   leaves the app); the production policy flag is never named (P-M3b-7).
//! - Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//!   docker_m3b_matrix`. Fixture names carry `hv_t31_m3b` (Task 31's sweep drops what a red run
//!   left) and are dropped at the end.
//! - Case 12 of the plan (`physical_memory_bytes_is_known`) is not here: `hacivat-exec` cannot
//!   reach `hacivat-ui-core` (layering); `hacivat-ui-core`'s
//!   `physical_memory_is_known_on_this_platform` runs in `build-test` on all three OSes.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::matrix::{Line, Side, admin, cell, docker_enabled, ended, lines, same, secs, unique};
use common::t28::{AllowAll, Con, Planned, Port, Spec, Table, shape};
use hacivat_driver::TxStatus;
use hacivat_exec::classify_pool::ClassifyPool;
use hacivat_exec::{
    ConfirmAnswer, ConfirmPort, CountRequest, ErrorPolicy, LogMsg, RunChoice, RunTicket,
    count_prepared, count_sql,
};
use hacivat_proto::DbmsKind;
use hacivat_session::session::{
    CommitAnswer, RunEvent, StatementClassifier, TxMode, TxOwner, UnitVerdict,
};
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, RawText, raw_text, type_label};
use hacivat_values::{BindValue, LogicalType, Value};

const DEADLINE: Duration = Duration::from_secs(2);

fn pool() -> Arc<ClassifyPool> {
    ClassifyPool::new(2)
}

fn typed(hint: LogicalType, text: &str) -> BindValue {
    BindValue::Typed {
        hint,
        text: Arc::from(text),
    }
}

fn int_hint() -> LogicalType {
    LogicalType::Int {
        bits: 64,
        signed: true,
    }
}

fn int(v: &Value) -> i64 {
    match v {
        Value::Int(i) => *i,
        Value::UInt(u) => i64::try_from(*u).expect("an int"),
        other => panic!("not an integer: {other:?}"),
    }
}

/// Collected mismatches: a test reports every line's failure at once.
#[derive(Default)]
struct Bad(Vec<String>);

impl Bad {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            let w = what();
            eprintln!("mismatch: {w}");
            self.0.push(w);
        }
    }

    fn eq<T: PartialEq + std::fmt::Debug>(&mut self, got: T, want: T, what: &str) {
        if got != want {
            let w = format!("{what}: got {got:?}, want {want:?}");
            eprintln!("mismatch: {w}");
            self.0.push(w);
        }
    }

    fn assert_none(self, test: &str) {
        assert!(self.0.is_empty(), "{test}:\n{}", self.0.join("\n"));
    }
}

/// `CREATE TABLE` text of `name` with columns `cols` (MySQL family: InnoDB, utf8mb4).
fn create(line: &Line, name: &str, cols: &str) -> String {
    if line.is_mysql_family() {
        format!("CREATE TABLE {name} ({cols}) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4")
    } else {
        format!("CREATE TABLE {name} ({cols})")
    }
}

/// Every row of `tab` now.
fn rows_of(tab: &hacivat_exec::ResultTab) -> Vec<Vec<Value>> {
    let snap = tab.source.snapshot();
    let width = u32::try_from(snap.schema().columns.len()).unwrap();
    (0..snap.row_count())
        .map(|r| (0..width).map(|c| snap.cell(r, c).to_value()).collect())
        .collect()
}

// ---------------------------------------------------------------------------------------------
// 1. Binds

struct Col {
    name: &'static str,
    ddl: &'static str,
    hint: LogicalType,
    bound: &'static str,
    literal: &'static str,
}

fn col(
    name: &'static str,
    ddl: &'static str,
    hint: LogicalType,
    bound: &'static str,
    literal: &'static str,
) -> Col {
    Col {
        name,
        ddl,
        hint,
        bound,
        literal,
    }
}

/// One column per type hint the engine has, with the text the prompt gives and the same
/// value written as a literal.
fn bind_columns(engine: DbmsKind) -> Vec<Col> {
    use LogicalType as L;
    let text = L::Text {
        max_chars: None,
        fixed: false,
    };
    let uint = L::Int {
        bits: 64,
        signed: false,
    };
    let dec = L::Decimal {
        precision: Some(10),
        scale: Some(2),
    };
    let float = L::Float { bits: 64 };
    let bytes = L::Bytes { max: None };
    let time = L::Time {
        precision: 2,
        tz: false,
    };
    let ts = L::Timestamp {
        precision: 6,
        tz: false,
    };
    const TEXT: &str = "O'Reilly ş İ";
    const TEXT_LIT: &str = "'O''Reilly ş İ'";
    const STAMP: &str = "2024-02-29 13:14:15.123456";
    const STAMP_LIT: &str = "'2024-02-29 13:14:15.123456'";
    const JSON: &str = r#"{"a": 1}"#;
    const JSON_LIT: &str = r#"'{"a": 1}'"#;
    match engine {
        DbmsKind::Postgres => vec![
            col("b", "bool", L::Bool, "true", "'true'"),
            col(
                "i",
                "int8",
                int_hint(),
                "9223372036854775807",
                "'9223372036854775807'",
            ),
            col(
                "u",
                "numeric(20,0)",
                uint,
                "18446744073709551615",
                "'18446744073709551615'",
            ),
            col("n", "numeric(10,2)", dec, "-1.50", "'-1.50'"),
            col("f", "float8", float, "0.5", "'0.5'"),
            col("tx", "text", text, TEXT, TEXT_LIT),
            col("bt", "bytea", bytes, r"\x00ff", r"'\x00ff'"),
            col(
                "uu",
                "uuid",
                L::Uuid,
                "6f1e9a0c-0000-4000-8000-000000000001",
                "'6f1e9a0c-0000-4000-8000-000000000001'",
            ),
            col("d", "date", L::Date, "2024-02-29", "'2024-02-29'"),
            col("tm", "time(2)", time, "13:14:15.25", "'13:14:15.25'"),
            col("ts", "timestamp(6)", ts, STAMP, STAMP_LIT),
            col(
                "tz",
                "timestamptz",
                L::Timestamp {
                    precision: 6,
                    tz: true,
                },
                "2024-02-29 13:14:15+00",
                "'2024-02-29 13:14:15+00'",
            ),
            col(
                "iv",
                "interval",
                L::Interval,
                "1 day 02:00:00",
                "'1 day 02:00:00'",
            ),
            col("j", "jsonb", L::Json { binary: true }, JSON, JSON_LIT),
        ],
        DbmsKind::Sqlite => vec![
            col("b", "BOOLEAN", L::Bool, "TRUE", "1"),
            col(
                "i",
                "INTEGER",
                int_hint(),
                "9223372036854775807",
                "9223372036854775807",
            ),
            col("n", "NUMERIC", dec, "1.25", "1.25"),
            col("f", "REAL", float, "0.5", "0.5"),
            col("tx", "TEXT", text, TEXT, TEXT_LIT),
            col("bt", "BLOB", bytes, r"\x00ff", "x'00ff'"),
            col("d", "DATE", L::Date, "2024-02-29", "'2024-02-29'"),
            col("ts", "TEXT", ts, STAMP, STAMP_LIT),
            col("j", "TEXT", L::Json { binary: false }, JSON, JSON_LIT),
        ],
        _ => vec![
            col("b", "TINYINT(1)", L::Bool, "1", "1"),
            col(
                "i",
                "BIGINT",
                int_hint(),
                "9223372036854775807",
                "9223372036854775807",
            ),
            col(
                "u",
                "BIGINT UNSIGNED",
                uint,
                "18446744073709551615",
                "18446744073709551615",
            ),
            col("n", "DECIMAL(10,2)", dec, "-1.50", "-1.50"),
            col("f", "DOUBLE", float, "0.5", "0.5"),
            col("tx", "VARCHAR(40)", text, TEXT, TEXT_LIT),
            col("bt", "VARBINARY(8)", bytes, r"\x00ff", "X'00FF'"),
            col("d", "DATE", L::Date, "2024-02-29", "'2024-02-29'"),
            col("tm", "TIME(2)", time, "13:14:15.25", "'13:14:15.25'"),
            col("ts", "DATETIME(6)", ts, STAMP, STAMP_LIT),
            col("j", "JSON", L::Json { binary: false }, JSON, JSON_LIT),
        ],
    }
}

/// Every type hint binds and reads back the value its literal gives (row 1 bound, row 2 the
/// literals, through the same `INSERT`), a NULL bind is NULL, and what is sent holds only
/// placeholders (E4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn binds_round_trip_every_type_hint() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_binds");
        let cols = bind_columns(line.engine);
        let mut a = admin(line).await;
        let ddl: Vec<String> = cols
            .iter()
            .map(|c| format!("{} {}", c.name, c.ddl))
            .collect();
        a.batch(&create(line, &t, &format!("id INT, {}", ddl.join(", "))))
            .await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let names: Vec<&str> = cols.iter().map(|c| c.name).collect();
        let marks: Vec<String> = names.iter().map(|n| format!(":{n}")).collect();
        let literals: Vec<&str> = cols.iter().map(|c| c.literal).collect();
        let script = format!(
            "INSERT INTO {t} (id, {list}) VALUES (1, {marks});\n\
             INSERT INTO {t} (id, {list}) VALUES (2, {literals});\n\
             INSERT INTO {t} (id, tx) VALUES (3, :nothing)",
            list = names.join(", "),
            marks = marks.join(", "),
            literals = literals.join(", "),
        );
        let mut values: Vec<(&str, BindValue)> = cols
            .iter()
            .map(|c| (c.name, typed(c.hint.clone(), c.bound)))
            .collect();
        values.push(("nothing", BindValue::Null));
        let mut p = c.plan(&script, &values).await;
        bad.eq(p.shapes(), vec!["run"; 3], &format!("{id}: the plan"));
        let sent = &p.prepared[0].final_text;
        bad.check(!sent.contains("Reilly") && !sent.contains("2024"), || {
            format!("{id}: a value was spliced into `{sent}`")
        });
        let mark = if line.is_pg() { "$1" } else { "?" };
        bad.check(sent.contains(mark) && !sent.contains(":b"), || {
            format!("{id}: `{sent}` holds no driver placeholder")
        });
        bad.eq(
            p.prepared[0].binds.len(),
            cols.len(),
            &format!("{id}: binds"),
        );
        p.answer(ConfirmAnswer::Execute);
        let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
        bad.eq(
            ran.outcome(),
            "ran".to_string(),
            &format!("{id}: the inserts"),
        );
        let back = c.ok(&format!("SELECT * FROM {t} ORDER BY id")).await;
        let rows = rows_of(back.tab(0));
        bad.eq(rows.len(), 3, &format!("{id}: rows"));
        if rows.len() == 3 {
            for (k, col) in cols.iter().enumerate() {
                let (b, l) = (&rows[0][k + 1], &rows[1][k + 1]);
                bad.check(same(b, l) && *b != Value::Null, || {
                    format!("{id}: {} bound {b:?}, literal {l:?}", col.name)
                });
            }
            let at = |n: &str| names.iter().position(|x| *x == n).unwrap() + 1;
            bad.eq(
                rows[0][at("i")].clone(),
                Value::Int(i64::MAX),
                &format!("{id}: int8"),
            );
            bad.eq(
                rows[0][at("tx")].clone(),
                Value::Text("O'Reilly ş İ".into()),
                &format!("{id}: text"),
            );
            bad.eq(
                rows[2][at("tx")].clone(),
                Value::Null,
                &format!("{id}: a NULL bind"),
            );
        }
        table.add(
            line,
            c.version.as_ref(),
            "binds",
            &format!("{} hints + NULL", cols.len()),
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("binds");
    table.finish("binds", &lines);
}

/// A value holding a quote and a statement is bound, never spliced: the server stores it as
/// written and the table survives (E4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn value_with_quote_is_bound_not_spliced() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_quote");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "id INT, v VARCHAR(200)")).await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let hostile = format!("x'); DROP TABLE {t}; --");
        let v = typed(
            LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            &hostile,
        );
        let insert = format!("INSERT INTO {t} (id, v) VALUES (1, :v)");
        let p = c.plan(&insert, &[("v", v.clone())]).await;
        bad.check(!p.prepared[0].final_text.contains("DROP"), || {
            format!("{id}: spliced: {}", p.prepared[0].final_text)
        });
        let ran = c.run_with(&insert, &[("v", v.clone())]).await;
        bad.eq(ran.outcome(), "ran".to_string(), &format!("{id}: insert"));
        let found = c
            .run_with(&format!("SELECT v FROM {t} WHERE v = :v"), &[("v", v)])
            .await;
        bad.eq(found.outcome(), "ran".to_string(), &format!("{id}: select"));
        if found.outcome() == "ran" {
            bad.eq(
                rows_of(found.tab(0)),
                vec![vec![Value::Text(hostile.as_str().into())]],
                &format!("{id}: found by the bound value"),
            );
        }
        bad.eq(
            a.scalar(&format!("SELECT v FROM {t}")).await,
            hostile.clone(),
            &format!("{id}: the side channel"),
        );
        table.add(line, c.version.as_ref(), "quote", "bound, table kept");
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("quote");
    table.finish("quote", &lines);
}

// ---------------------------------------------------------------------------------------------
// 2. Multiple results and Run as one

/// The first cell of every tab of `ran`, as `int` when typed or `text` when the result came as
/// text (PostgreSQL's Run as one: the simple protocol names no type, so every column is
/// `Text`; the driver's documented design, T13).
fn firsts(ran: &common::matrix::Ran) -> Vec<String> {
    ran.tabs
        .iter()
        .map(|t| match cell(t, 0, 0) {
            Value::Text(s) => format!("text {s}"),
            v => format!("int {}", int(&v)),
        })
        .collect()
}

fn ints(v: &[i64]) -> Vec<String> {
    v.iter().map(|i| format!("int {i}")).collect()
}

fn texts(v: &[i64]) -> Vec<String> {
    v.iter().map(|i| format!("text {i}")).collect()
}

/// A MySQL/MariaDB `CALL` of a procedure with three result sets gives three tabs; a PostgreSQL
/// `CALL` gives its `INOUT` row; Run as one gives one tab per statement on every engine (E5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn multi_result_call_and_run_as_one() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let proc = unique("hv_t31_m3b_sets");
        let mut a = admin(line).await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        // CALL.
        let call = match line.engine {
            DbmsKind::Postgres => {
                a.batch(&format!(
                    "CREATE PROCEDURE {proc}(INOUT x int) LANGUAGE plpgsql \
                     AS $$ BEGIN x := x + 1; END $$"
                ))
                .await;
                Some((format!("CALL {proc}(41)"), vec![42]))
            }
            DbmsKind::Sqlite => None,
            _ => {
                a.batch(&format!(
                    "CREATE PROCEDURE {proc}() BEGIN SELECT 1 AS a; SELECT 2 AS b; \
                     SELECT 3 AS c; END"
                ))
                .await;
                Some((format!("CALL {proc}()"), vec![1, 2, 3]))
            }
        };
        let call_outcome = match &call {
            Some((sql, want)) => {
                let ran = c.run(sql).await;
                bad.eq(ran.outcome(), "ran".to_string(), &format!("{id}: CALL"));
                if ran.outcome() == "ran" {
                    bad.eq(firsts(&ran), ints(want), &format!("{id}: CALL's tabs"));
                }
                format!("call {} tabs", ran.tabs.len())
            }
            None => "call n/a (no procedures)".to_string(),
        };
        // Run as one.
        let script = if line.is_pg() {
            "SELECT 1 AS a; SELECT 2 AS b; SELECT 3 AS c"
        } else {
            "SELECT 1 AS a; SELECT 2 AS b"
        };
        let mut p = c.plan_units(c.as_one(script), &[], true).await;
        bad.eq(p.shapes(), vec!["run"], &format!("{id}: as one plan"));
        p.answer(ConfirmAnswer::Execute);
        let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
        bad.eq(ran.outcome(), "ran".to_string(), &format!("{id}: as one"));
        let want = if line.is_pg() {
            texts(&[1, 2, 3])
        } else {
            ints(&[1, 2])
        };
        if ran.outcome() == "ran" {
            bad.eq(firsts(&ran), want, &format!("{id}: as one's tabs"));
        }
        table.add(
            line,
            c.version.as_ref(),
            "multi-result",
            &format!("{call_outcome}; as one {} tabs", ran.tabs.len()),
        );
        c.shutdown().await;
        if call.is_some() {
            a.batch(&format!("DROP PROCEDURE {proc}")).await;
        }
        a.close().await;
    }
    bad.assert_none("multi-result");
    table.finish("multi-result", &lines);
}

// ---------------------------------------------------------------------------------------------
// 3. The error policy

/// The run's question: `(run seq, question id, unit ordinal, rollback_and_continue)`.
async fn choice(id: &str, t: &RunTicket) -> (u64, u64, u32, bool) {
    let mut choices = t.choices.clone();
    let pending = tokio::time::timeout(secs(30), choices.wait_for(Option::is_some))
        .await
        .unwrap_or_else(|_| panic!("{id}: the run never asked"))
        .expect("the run is alive");
    let (seq, ev) = pending.clone().expect("a question");
    match ev {
        RunEvent::NeedsChoice {
            unit,
            question,
            rollback_and_continue,
            ..
        } => (seq, question, unit.ordinal, rollback_and_continue),
        other => panic!("{id}: expected NeedsChoice, got {other:?}"),
    }
}

/// The side channel's count of rows of `t` with `a = v`.
async fn count_of(a: &mut Side, t: &str, v: i64) -> i64 {
    a.count(&format!("SELECT COUNT(*) FROM {t} WHERE a = {v}"))
        .await
}

/// Runs `script` under `Ask`, answers its question `answer`; the run's question and end.
async fn asked_run(
    c: &Con,
    script: &str,
    answer: impl Fn(bool) -> RunChoice,
) -> ((u32, bool), common::matrix::Ran) {
    let id = &c.line.id;
    let mut p = c.plan(script, &[]).await;
    p.answer(ConfirmAnswer::Execute);
    let t = c.send(&p, ErrorPolicy::Ask, None);
    let (seq, question, unit, rac) = choice(id, &t).await;
    assert!(
        c.s.choose(seq, question, answer(rac))
            .await
            .expect("the actor replies"),
        "{id}: the answer was taken"
    );
    ((unit, rac), ended(t).await)
}

/// Ask: Ignore runs the rest, Stop skips it; in a failed PostgreSQL transaction the question
/// offers Roll back and continue, which rolls back and goes on in the session's mode (Auto
/// and Manual, P-M3b-19); the MySQL family and SQLite keep their transaction (E5).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn error_policy_ask_ignore_stop_and_rollback_and_continue() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_policy");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT")).await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let script = |x: i64, y: i64| {
            format!(
                "INSERT INTO {t} VALUES ({x});\nINSERT INTO {t}_nope VALUES (0);\n\
                 INSERT INTO {t} VALUES ({y})"
            )
        };

        // Ignore.
        let ((unit, rac), ran) = asked_run(&c, &script(1, 2), |_| RunChoice::Ignore).await;
        bad.eq((unit, rac), (1, false), &format!("{id}: Ignore's question"));
        bad.eq(
            (ran.summary.executed, ran.summary.failed),
            (2, 1),
            &format!("{id}: Ignore ran the rest"),
        );
        bad.eq(
            (count_of(&mut a, &t, 1).await, count_of(&mut a, &t, 2).await),
            (1, 1),
            &format!("{id}: Ignore's rows"),
        );

        // Stop.
        let ((unit, _), ran) = asked_run(&c, &script(3, 4), |_| RunChoice::Stop).await;
        bad.eq(unit, 1, &format!("{id}: Stop's question"));
        bad.eq(
            ran.summary.skipped,
            1,
            &format!("{id}: Stop skipped the rest"),
        );
        bad.eq(
            (count_of(&mut a, &t, 3).await, count_of(&mut a, &t, 4).await),
            (1, 0),
            &format!("{id}: Stop's rows"),
        );

        // A failure inside the user's transaction.
        let in_tx = format!("BEGIN;\n{}", script(5, 6));
        let pick = |rac: bool| {
            if rac {
                RunChoice::RollbackAndContinue
            } else {
                RunChoice::Ignore
            }
        };
        let ((unit, rac), _) = asked_run(&c, &in_tx, pick).await;
        bad.eq(
            (unit, rac),
            (2, line.is_pg()),
            &format!("{id}: in a transaction"),
        );
        let snap = c.settled().await;
        if line.is_pg() {
            // Rolled back, then the last statement ran in Auto.
            bad.eq(
                snap.tx.status,
                TxStatus::Idle,
                &format!("{id}: rolled back"),
            );
            bad.eq(
                (count_of(&mut a, &t, 5).await, count_of(&mut a, &t, 6).await),
                (0, 1),
                &format!("{id}: Roll back and continue"),
            );
        } else {
            bad.eq(
                (snap.tx.status, snap.tx.owner),
                (TxStatus::InTx, TxOwner::User),
                &format!("{id}: the user's transaction stays"),
            );
            c.ok("COMMIT").await;
            bad.eq(
                (count_of(&mut a, &t, 5).await, count_of(&mut a, &t, 6).await),
                (1, 1),
                &format!("{id}: the user's COMMIT"),
            );
        }

        // Manual mode (P-M3b-19: Roll back and continue keeps it).
        c.set_mode(TxMode::Manual).await;
        let ((unit, rac), _) = asked_run(&c, &script(7, 8), pick).await;
        bad.eq((unit, rac), (1, line.is_pg()), &format!("{id}: Manual"));
        let snap = c.settled().await;
        bad.eq(snap.tx.mode, TxMode::Manual, &format!("{id}: Manual kept"));
        bad.eq(
            snap.tx.status,
            TxStatus::InTx,
            &format!("{id}: Manual, open"),
        );
        c.ok("COMMIT").await;
        let want = if line.is_pg() { (0, 1) } else { (1, 1) };
        bad.eq(
            (count_of(&mut a, &t, 7).await, count_of(&mut a, &t, 8).await),
            want,
            &format!("{id}: Manual's rows"),
        );
        c.set_mode(TxMode::Auto).await;

        table.add(
            line,
            c.version.as_ref(),
            "error-policy",
            if line.is_pg() {
                "ignore, stop, rollback-and-continue (auto, manual)"
            } else {
                "ignore, stop, ignore in tx (auto, manual)"
            },
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("error-policy");
    table.finish("error-policy", &lines);
}

// ---------------------------------------------------------------------------------------------
// 4. Confirm-in-transaction

/// Plans `sql` on production; its last unit must be confirm-in-transaction; the others run or
/// ask first (a `BEGIN` on production asks: transaction control is no Read or Session class), and
/// every question is answered Execute.
async fn tx_confirm_plan(c: &Con, sql: &str) -> Planned {
    let mut p = c.plan(sql, &[]).await;
    let shapes = p.shapes();
    let (last, rest) = shapes.split_last().expect("a unit");
    assert_eq!(*last, "tx-confirm", "{}: `{sql}`: {shapes:?}", c.line.id);
    assert!(
        rest.iter().all(|s| *s == "run" || *s == "confirm"),
        "{}: `{sql}`: {shapes:?}",
        c.line.id
    );
    p.answer(ConfirmAnswer::Execute);
    p
}

/// Sends `sql` with a port of its own; the run and its first question (`savepoint`).
async fn confirmed(c: &Con, sql: &str) -> (RunTicket, Arc<Port>, bool) {
    let p = tx_confirm_plan(c, sql).await;
    let port = Port::new();
    let t = c.send(
        &p,
        ErrorPolicy::StopOnError,
        Some(port.clone() as Arc<dyn ConfirmPort>),
    );
    // The question, or the run's units if it ended without one.
    let deadline = std::time::Instant::now() + secs(30);
    while port.asked().is_empty() {
        let status = t.status.borrow().clone();
        let names: Vec<&str> = status.units.iter().map(|u| u.status.name()).collect();
        assert!(
            std::time::Instant::now() < deadline
                && !(!status.units.is_empty() && status.units.iter().all(|u| u.status.is_final())),
            "{}: `{sql}` asked nothing: {names:?} {:?}",
            c.line.id,
            status.units.iter().map(|u| &u.status).collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let q = port.question(&c.line.id, 1).await;
    (t, port, q.savepoint)
}

/// The deadline's answer and the form of a run sent with [`Con::run_with_deadline`].
fn by_deadline(events: &[RunEvent]) -> (Option<(CommitAnswer, bool)>, Option<bool>) {
    let decided = events.iter().find_map(|e| match e {
        RunEvent::CommitDecided {
            answer,
            by_deadline,
            ..
        } => Some((*answer, *by_deadline)),
        _ => None,
    });
    let form = events.iter().find_map(|e| match e {
        RunEvent::NeedsCommit { savepoint, .. } => Some(*savepoint),
        _ => None,
    });
    (decided, form)
}

/// Confirm-in-transaction on every engine, both forms (P-M3b-1): Auto idle takes Hacivat's own
/// transaction; a user `BEGIN` earlier in the run, Manual and Smart take the savepoint (the
/// MySQL family never sends `START TRANSACTION` inside one: the user's own earlier row stays
/// uncommitted); the side channel sees a row only after Commit, or after Keep and the user's
/// COMMIT; Undo and the 2 s deadline undo the confirmed statement only; a failure inside the
/// savepoint leaves the user's transaction open (P-M3b-5). Manual Undo on the MySQL family too
/// (the T19 carry) (E2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn confirm_in_tx_both_forms_per_engine() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_confirm");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT PRIMARY KEY")).await;
        let c = Con::open(line, Spec::production(), &pool).await;
        // A production profile starts in Manual (`tx::default_mode`); the own form needs Auto.
        bad.eq(
            c.snap().tx.mode,
            TxMode::Manual,
            &format!("{id}: production's mode"),
        );
        c.set_mode(TxMode::Auto).await;
        let ins = |v: i64| format!("INSERT INTO {t} (a) VALUES ({v})");
        let seen = |v: i64| format!("SELECT COUNT(*) FROM {t} WHERE a = {v}");
        let state = |c: &Con| {
            let s = c.snap();
            (s.tx.status, s.tx.owner)
        };
        let user_tx = (TxStatus::InTx, TxOwner::User);
        let idle = (TxStatus::Idle, TxOwner::None);

        // Auto idle: Hacivat's own transaction. Commit.
        let (run, port, sp) = confirmed(&c, &ins(1)).await;
        bad.eq(sp, false, &format!("{id}: Auto idle form"));
        bad.eq(a.count(&seen(1)).await, 0, &format!("{id}: before Commit"));
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        c.settled().await;
        bad.eq(a.count(&seen(1)).await, 1, &format!("{id}: after Commit"));
        bad.eq(state(&c), idle, &format!("{id}: after Commit"));

        // Roll back.
        let (run, port, _) = confirmed(&c, &ins(2)).await;
        port.answer(CommitAnswer::RollBack);
        ended(run).await;
        c.settled().await;
        bad.eq(a.count(&seen(2)).await, 0, &format!("{id}: rolled back"));
        bad.eq(state(&c), idle, &format!("{id}: after Roll back"));

        // The deadline rolls Hacivat's own transaction back.
        let p = tx_confirm_plan(&c, &ins(3)).await;
        let ev = c.run_with_deadline(&p, DEADLINE).await;
        bad.eq(
            by_deadline(&ev),
            (Some((CommitAnswer::RollBack, true)), Some(false)),
            &format!("{id}: own-form deadline"),
        );
        c.settled().await;
        bad.eq(
            a.count(&seen(3)).await,
            0,
            &format!("{id}: own-form deadline"),
        );
        bad.eq(state(&c), idle, &format!("{id}: after the deadline"));

        // A user BEGIN earlier in the run: the savepoint. Keep, then the user's COMMIT.
        let (run, port, sp) = confirmed(&c, &format!("BEGIN;\n{}", ins(4))).await;
        bad.eq(sp, true, &format!("{id}: BEGIN earlier in the run"));
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        c.settled().await;
        bad.eq(state(&c), user_tx, &format!("{id}: kept"));
        bad.eq(
            a.count(&seen(4)).await,
            0,
            &format!("{id}: kept, not committed"),
        );
        c.ok("COMMIT").await;
        bad.eq(
            a.count(&seen(4)).await,
            1,
            &format!("{id}: the user's COMMIT"),
        );

        // Undo undoes the confirmed statement only; the user's earlier one stays uncommitted
        // (no implicit commit: no START TRANSACTION was sent inside the user's transaction).
        c.ok("BEGIN").await;
        let (run, port, sp) = confirmed(&c, &ins(5)).await;
        bad.eq(sp, true, &format!("{id}: inside the user's transaction"));
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        let (run, port, _) = confirmed(&c, &ins(6)).await;
        port.answer(CommitAnswer::RollBack);
        ended(run).await;
        c.settled().await;
        bad.eq(
            state(&c),
            user_tx,
            &format!("{id}: Undo keeps the user's transaction"),
        );
        bad.eq(
            (a.count(&seen(5)).await, a.count(&seen(6)).await),
            (0, 0),
            &format!("{id}: nothing committed behind the user"),
        );
        c.ok("COMMIT").await;
        bad.eq(
            (a.count(&seen(5)).await, a.count(&seen(6)).await),
            (1, 0),
            &format!("{id}: Undo"),
        );

        // The deadline in the savepoint form undoes the confirmed statement only.
        c.ok("BEGIN").await;
        let (run, port, _) = confirmed(&c, &ins(7)).await;
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        let p = tx_confirm_plan(&c, &ins(8)).await;
        let ev = c.run_with_deadline(&p, DEADLINE).await;
        bad.eq(
            by_deadline(&ev),
            (Some((CommitAnswer::RollBack, true)), Some(true)),
            &format!("{id}: savepoint deadline"),
        );
        c.settled().await;
        bad.eq(
            state(&c),
            user_tx,
            &format!("{id}: the deadline keeps the user's tx"),
        );
        c.ok("COMMIT").await;
        bad.eq(
            (a.count(&seen(7)).await, a.count(&seen(8)).await),
            (1, 0),
            &format!("{id}: savepoint deadline"),
        );

        // A failure inside the savepoint leaves the user's transaction open (P-M3b-5).
        c.ok("BEGIN").await;
        let (run, port, _) = confirmed(&c, &ins(9)).await;
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        let p = tx_confirm_plan(&c, &ins(9)).await;
        let port = Port::new();
        let ran = ended(c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        ))
        .await;
        bad.check(ran.outcome() != "ran", || {
            format!("{id}: the duplicate ran")
        });
        bad.eq(
            port.asked().len(),
            0,
            &format!("{id}: a failed statement asks nothing"),
        );
        c.settled().await;
        bad.eq(
            state(&c),
            user_tx,
            &format!("{id}: failure inside the savepoint"),
        );
        c.ok("COMMIT").await;
        bad.eq(
            a.count(&seen(9)).await,
            1,
            &format!("{id}: the user's work kept"),
        );

        // Manual idle: the session opens the user's transaction, then the savepoint. Keep;
        // then Undo (Manual MySQL/MariaDB Undo: `autocommit = 0`, the savepoint goes straight
        // in).
        c.set_mode(TxMode::Manual).await;
        let (run, port, sp) = confirmed(&c, &ins(10)).await;
        bad.eq(sp, true, &format!("{id}: Manual"));
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        c.settled().await;
        bad.eq(a.count(&seen(10)).await, 0, &format!("{id}: Manual, kept"));
        c.ok("COMMIT").await;
        bad.eq(
            a.count(&seen(10)).await,
            1,
            &format!("{id}: Manual, committed"),
        );
        let (run, port, sp) = confirmed(&c, &ins(11)).await;
        bad.eq(sp, true, &format!("{id}: Manual again"));
        port.answer(CommitAnswer::RollBack);
        ended(run).await;
        c.ok("COMMIT").await;
        bad.eq(a.count(&seen(11)).await, 0, &format!("{id}: Manual Undo"));
        c.set_mode(TxMode::Auto).await;

        // Smart: the write opens the user's transaction; the savepoint.
        c.set_mode(TxMode::Smart).await;
        let (run, port, sp) = confirmed(&c, &ins(12)).await;
        bad.eq(sp, true, &format!("{id}: Smart"));
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        c.settled().await;
        bad.eq(state(&c), user_tx, &format!("{id}: Smart opened"));
        bad.eq(a.count(&seen(12)).await, 0, &format!("{id}: Smart, kept"));
        c.ok("COMMIT").await;
        bad.eq(
            a.count(&seen(12)).await,
            1,
            &format!("{id}: Smart, committed"),
        );
        c.set_mode(TxMode::Auto).await;

        table.add(
            line,
            c.version.as_ref(),
            "confirm-in-tx",
            "own: commit, rollback, deadline; savepoint: begin-in-run, undo, deadline, \
             failure; manual keep+undo; smart",
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("confirm-in-tx");
    table.finish("confirm-in-tx", &lines);
}

// ---------------------------------------------------------------------------------------------
// 5. Smart

/// Smart opens the user's transaction before a write, never before a read (E2 / D-036).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn smart_mode_opens_only_before_writes() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_smart");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT")).await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        c.set_mode(TxMode::Smart).await;
        let read = format!("SELECT a FROM {t}");
        let write = format!("INSERT INTO {t} VALUES (1)");
        bad.eq(
            c.plan(&read, &[]).await.plan.items[0].verdict,
            UnitVerdict::Read,
            &format!("{id}: the read's verdict"),
        );
        bad.eq(
            c.plan(&write, &[]).await.plan.items[0].verdict,
            UnitVerdict::Write,
            &format!("{id}: the write's verdict"),
        );
        c.ok(&read).await;
        let s = c.settled().await;
        bad.eq(
            s.tx.status,
            TxStatus::Idle,
            &format!("{id}: a read opens nothing"),
        );
        c.ok(&write).await;
        let s = c.settled().await;
        bad.eq(
            (s.tx.status, s.tx.owner, s.tx.mode),
            (TxStatus::InTx, TxOwner::User, TxMode::Smart),
            &format!("{id}: a write opens the user's transaction"),
        );
        bad.eq(
            a.count(&format!("SELECT COUNT(*) FROM {t}")).await,
            0,
            &format!("{id}: not committed"),
        );
        c.ok("COMMIT").await;
        let s = c.settled().await;
        bad.eq(
            (s.tx.status, s.tx.mode),
            (TxStatus::Idle, TxMode::Smart),
            &format!("{id}: after COMMIT"),
        );
        bad.eq(
            a.count(&format!("SELECT COUNT(*) FROM {t}")).await,
            1,
            &format!("{id}: committed"),
        );
        c.set_mode(TxMode::Auto).await;
        table.add(
            line,
            c.version.as_ref(),
            "smart",
            "read: none; write: opened",
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("smart");
    table.finish("smart", &lines);
}

// ---------------------------------------------------------------------------------------------
// 6. EXPLAIN ANALYZE

/// What each engine does with `EXPLAIN ANALYZE DELETE … WHERE`: the gate's decision and the
/// run's outcome.
fn explain_expected(line: &Line) -> (&'static str, &'static str) {
    match line.engine {
        // D-062: wrapped in a rollback.
        DbmsKind::Postgres => ("explain-wrap", "ran"),
        // MySQL runs EXPLAIN ANALYZE of the DELETE inside the D-062 wrap.
        DbmsKind::MySql => ("explain-wrap", "ran"),
        // MariaDB spells it `ANALYZE DELETE`: a syntax error (ER_PARSE_ERROR 1064, by its
        // SQLSTATE). The gate still plans the wrap (the interim backend reads MySQL syntax);
        // the server refuses before anything runs.
        DbmsKind::MariaDb => ("explain-wrap", "42000"),
        // SQLite has no EXPLAIN ANALYZE: a syntax error.
        _ => ("explain-wrap", "1"),
    }
}

/// `EXPLAIN ANALYZE` of a DML statement leaves the table as it was (D-062; E2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn explain_analyze_dml_is_rolled_back() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_explain");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT")).await;
        a.batch(&format!("INSERT INTO {t} VALUES (1), (2), (3)"))
            .await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let sql = format!("EXPLAIN ANALYZE DELETE FROM {t} WHERE a > 0");
        let p = c.plan(&sql, &[]).await;
        let got_shape = shape(&p.plan.items[0].decision);
        let ran = c.run(&sql).await;
        let (want_shape, want_outcome) = explain_expected(line);
        bad.eq(
            (got_shape, ran.outcome().as_str()),
            (want_shape, want_outcome),
            &format!("{id}: decision and outcome"),
        );
        if line.is_pg() && ran.outcome() == "ran" {
            bad.check(!ran.tabs.is_empty(), || format!("{id}: no plan tab"));
        }
        bad.eq(
            a.count(&format!("SELECT COUNT(*) FROM {t}")).await,
            3,
            &format!("{id}: the side channel"),
        );
        let s = c.settled().await;
        bad.eq(
            s.tx.status,
            TxStatus::Idle,
            &format!("{id}: nothing left open"),
        );
        table.add(
            line,
            c.version.as_ref(),
            "explain-analyze",
            &format!("{got_shape}, {}, 3 rows kept", ran.outcome()),
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("explain-analyze");
    table.finish("explain-analyze", &lines);
}

// ---------------------------------------------------------------------------------------------
// 7. Read-only

/// A write whose first keyword is not a write, so the read-only input layer (DEMO-7, first
/// keyword only) lets it through to step 0b (the bypass catalogue's P1 / M5 shapes).
fn sneaky(line: &Line, t: &str) -> String {
    match line.engine {
        DbmsKind::Postgres => format!("WITH d AS (DELETE FROM {t} RETURNING *) SELECT * FROM d"),
        DbmsKind::Sqlite => format!("WITH x(v) AS (SELECT 2) INSERT INTO {t} SELECT v FROM x"),
        _ => format!("SELECT * FROM {t} FOR UPDATE"),
    }
}

/// What the server's layers answer [`sneaky`] on a read-only session whose classifier allowed
/// it (the run's outcome: a code or the guard's layer).
fn server_stop(line: &Line) -> &'static str {
    match line.engine {
        DbmsKind::Postgres => "25006",
        // The read-only connection's authorizer (SQLITE_AUTH).
        DbmsKind::Sqlite => "23",
        // ER_CANT_EXECUTE_IN_READ_ONLY_TRANSACTION (1792), reported by its SQLSTATE.
        _ => "25006",
    }
}

/// On a read-only console the real classifier denies a write at the gate (and the run service
/// refuses a denied plan), and again at the session's step 0b when the gate is bypassed; with
/// a classifier that allows everything, and (PostgreSQL) for a read the real classifier
/// allows that writes through a view, the server's layers still stop it. The side channel is
/// unchanged every time (E1 / E8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn read_only_classifier_denies_and_server_layers_hold() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_ro");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT")).await;
        a.batch(&format!("INSERT INTO {t} VALUES (1)")).await;
        if line.is_pg() {
            a.batch(&format!(
                "CREATE FUNCTION {t}_w() RETURNS int LANGUAGE sql VOLATILE \
                 AS $$ INSERT INTO {t} VALUES (100) RETURNING a $$"
            ))
            .await;
            a.batch(&format!("CREATE VIEW {t}_v AS SELECT {t}_w() AS a"))
                .await;
        }
        let rows = format!("SELECT COUNT(*) FROM {t}");
        let write = format!("INSERT INTO {t} VALUES (2)");

        let real = Con::open(
            line,
            Spec::read_only(pool.clone() as Arc<dyn StatementClassifier>),
            &pool,
        )
        .await;
        // The gate denies; a denied plan sends nothing.
        let p = real.plan(&write, &[]).await;
        let denied = p.plan.first_denied().map(|i| match &i.decision {
            hacivat_exec::GateDecision::Deny(r) => r.fluent_id,
            _ => "",
        });
        bad.eq(
            denied,
            Some("exec-gate-read-only"),
            &format!("{id}: the gate"),
        );
        let ran = ended(real.send(&p, ErrorPolicy::StopOnError, None)).await;
        bad.check(ran.outcome() != "ran", || {
            format!("{id}: a denied plan ran")
        });
        bad.eq(a.count(&rows).await, 1, &format!("{id}: after the gate"));
        // The gate bypassed: the session's input layer refuses a write keyword (DEMO-7).
        let ran = ended(real.send(&p.forced(0), ErrorPolicy::StopOnError, None)).await;
        bad.eq(
            ran.outcome(),
            "input".to_string(),
            &format!("{id}: the input layer"),
        );
        // A write that starts like a read: the gate denies it, and with the gate bypassed
        // step 0b's classifier does.
        let p = real.plan(&sneaky(line, &t), &[]).await;
        bad.eq(p.shapes(), vec!["deny"], &format!("{id}: the gate, sneaky"));
        let ran = ended(real.send(&p.forced(0), ErrorPolicy::StopOnError, None)).await;
        bad.eq(
            ran.outcome(),
            "exec-deny-classifier".to_string(),
            &format!("{id}: step 0b"),
        );
        bad.eq(a.count(&rows).await, 1, &format!("{id}: after step 0b"));
        // PostgreSQL: a read the classifier allows writes through a view; the server stops it.
        let mut view = String::new();
        if line.is_pg() {
            let sql = format!("SELECT * FROM {t}_v");
            let p = real.plan(&sql, &[]).await;
            bad.eq(
                p.shapes(),
                vec!["run"],
                &format!("{id}: the view is a read"),
            );
            let ran = real.run(&sql).await;
            bad.eq(
                ran.outcome(),
                "25006".to_string(),
                &format!("{id}: the view's write"),
            );
            bad.eq(a.count(&rows).await, 1, &format!("{id}: after the view"));
            view = format!("; view read: {}", ran.outcome());
        }
        let version = real.version.clone();
        real.shutdown().await;

        // A classifier that allows everything: the server's layers stop the write.
        let open = Con::open(
            line,
            Spec::read_only(Arc::new(AllowAll) as Arc<dyn StatementClassifier>),
            &pool,
        )
        .await;
        let p = open.plan(&sneaky(line, &t), &[]).await.forced(0);
        let ran = ended(open.send(&p, ErrorPolicy::StopOnError, None)).await;
        bad.eq(
            ran.outcome(),
            server_stop(line).to_string(),
            &format!("{id}: the server's layers"),
        );
        bad.eq(a.count(&rows).await, 1, &format!("{id}: after the server"));
        let stopped = ran.outcome();
        open.shutdown().await;

        table.add(
            line,
            version.as_ref(),
            "read-only",
            &format!("gate deny; input layer; step 0b deny; allow-all: {stopped}{view}"),
        );
        if line.is_pg() {
            a.batch(&format!("DROP VIEW {t}_v")).await;
            a.batch(&format!("DROP FUNCTION {t}_w()")).await;
        }
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("read-only");
    table.finish("read-only", &lines);
}

// ---------------------------------------------------------------------------------------------
// 8. Notices and warnings

/// The texts of the console's notice lines.
fn notices(c: &Con) -> Vec<String> {
    c.log
        .lines()
        .iter()
        .filter_map(|l| match &l.msg {
            LogMsg::Notice { text, .. } => Some(text.to_string()),
            _ => None,
        })
        .collect()
}

/// A PostgreSQL notice and a MySQL/MariaDB warning reach the console's log with their text;
/// SQLite has neither (R-M3-77).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn notices_and_warnings_reach_the_log() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let absent = unique("hv_t31_m3b_absent");
        let c = Con::open(line, Spec::dev(), &pool).await;
        c.ok(&format!("DROP TABLE IF EXISTS {absent}")).await;
        let got = notices(&c);
        if line.is_sqlite() {
            bad.eq(got.len(), 0, &format!("{id}: SQLite has no notices"));
        } else {
            bad.check(got.iter().any(|n| n.contains(&absent)), || {
                format!("{id}: no notice names {absent}: {got:?}")
            });
        }
        if line.is_pg() {
            let marker = unique("hv_t31_m3b_notice");
            c.ok(&format!("DO $$ BEGIN RAISE NOTICE '{marker}'; END $$"))
                .await;
            bad.check(notices(&c).contains(&marker), || {
                format!("{id}: RAISE NOTICE not logged")
            });
        }
        table.add(
            line,
            c.version.as_ref(),
            "notices",
            &format!("{} notice lines", notices(&c).len()),
        );
        c.shutdown().await;
    }
    bad.assert_none("notices");
    table.finish("notices", &lines);
}

// ---------------------------------------------------------------------------------------------
// 9. Count rows

/// "Count rows" (R-M3-76) gives what `COUNT(*)` of the same rows gives, with and without a
/// bound value (E5 / HV-RES-10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn count_rows_matches_count_star() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_m3b_count");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "a INT")).await;
        let values: Vec<String> = (1..=25).map(|v| format!("({v})")).collect();
        a.batch(&format!("INSERT INTO {t} VALUES {}", values.join(", ")))
            .await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let count = |sql: String, binds: Arc<[BindValue]>| CountRequest {
            console: c.s.clone(),
            sessions: c.m.sessions(c.profile.id),
            engine: line.engine,
            sql,
            binds,
        };
        // Plain.
        let text = format!("SELECT * FROM {t} WHERE a > 5");
        let unit = c.units(&text).remove(0);
        let sql = count_sql(&c.doc(&text), &unit).expect("countable");
        let n = c.svc.count(count(sql, Arc::from(Vec::new()))).done.await;
        let want = a
            .count(&format!("SELECT COUNT(*) FROM {t} WHERE a > 5"))
            .await;
        bad.eq(
            n.ok().and_then(|r| r.ok()),
            Some(u64::try_from(want).unwrap()),
            &format!("{id}: count"),
        );
        // With a bound value.
        let text = format!("SELECT * FROM {t} WHERE a > :n");
        let p = c.plan(&text, &[("n", typed(int_hint(), "10"))]).await;
        let (sql, binds) =
            count_prepared(&c.doc(&p.prepared[0].final_text), &p.prepared[0]).expect("countable");
        let n = c.svc.count(count(sql, binds)).done.await;
        let want_b = a
            .count(&format!("SELECT COUNT(*) FROM {t} WHERE a > 10"))
            .await;
        bad.eq(
            n.ok().and_then(|r| r.ok()),
            Some(u64::try_from(want_b).unwrap()),
            &format!("{id}: count with a bind"),
        );
        table.add(
            line,
            c.version.as_ref(),
            "count-rows",
            &format!("{want} and {want_b}"),
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    bad.assert_none("count-rows");
    table.finish("count-rows", &lines);
}

// ---------------------------------------------------------------------------------------------
// 10. Type goldens

const TYPES_PG: &str = include_str!("../../hacivat-driver-pg/tests/data/types_pg.sql");
const TYPES_MYSQL: &str = include_str!("../../hacivat-driver-mysql/tests/data/types_mysql.sql");

/// PostgreSQL's session setup of the T7 golden (`hacivat-driver-pg/tests/docker_types_m3b.rs`).
const PG_SETUP: [&str; 6] = [
    "SET TIME ZONE 'UTC'",
    "SET lc_monetary TO 'C'",
    "SET DateStyle TO ISO, MDY",
    "SET IntervalStyle TO postgres",
    "CREATE TYPE pg_temp.hv_t07_mood AS ENUM ('sad', 'ok', 'happy')",
    "CREATE TYPE pg_temp.hv_t07_pair AS (a int4, b text)",
];

fn pg_kind(l: &LogicalType) -> &'static str {
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

fn my_kind(l: &LogicalType) -> &'static str {
    match l {
        LogicalType::BitString { .. } => "bit",
        LogicalType::Geometry { .. } => "geometry",
        _ => "-",
    }
}

fn raw_of(logical: &LogicalType, v: &Value, engine: DbmsKind) -> String {
    let facts = ColumnFacts {
        logical,
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

/// `(expression, kind, raw text)` of the PostgreSQL data file.
fn pg_cases() -> Vec<(String, String, String)> {
    TYPES_PG
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("--"))
        .map(|l| {
            let (expr, note) = l.rsplit_once(" -- ").expect("` -- `");
            let (kind, raw) = note.split_once(" =>").expect("` =>`");
            (
                expr.trim().to_string(),
                kind.trim().to_string(),
                raw.strip_prefix(' ').unwrap_or(raw).to_string(),
            )
        })
        .collect()
}

/// `(column, label, kind, raw text)` of a MySQL-family case.
type MyCase = (String, String, String, String);

/// The MySQL-family data file: its setup statements and its cases.
fn my_cases() -> (Vec<String>, Vec<MyCase>) {
    let setup = TYPES_MYSQL
        .lines()
        .filter(|l| !l.starts_with("--") && !l.starts_with('=') && l.ends_with(';'))
        .map(|l| l.trim_end_matches(';').to_string())
        .collect();
    let cases = TYPES_MYSQL
        .lines()
        .filter_map(|l| l.strip_prefix("= "))
        .map(|l| {
            let parts: Vec<&str> = l.split(" | ").collect();
            (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
            )
        })
        .collect();
    (setup, cases)
}

/// Task 7's type tables through Run (the PostgreSQL and MySQL-family data files of the driver
/// goldens, read by path), and the M3a SQLite golden: the logical kind and raw text of each
/// value the M3b path brings back (R-M3-67).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn type_goldens_match() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let mut checked = 0;
        match line.engine {
            DbmsKind::Postgres => {
                for sql in PG_SETUP {
                    c.ok(sql).await;
                }
                let cases = pg_cases();
                assert!(cases.len() >= 60, "{id}: {} cases", cases.len());
                for (expr, kind, raw) in cases {
                    let ran = c.run(&format!("SELECT {expr}")).await;
                    if ran.outcome() != "ran" {
                        bad.0.push(format!("{id}: `{expr}`: {}", ran.outcome()));
                        continue;
                    }
                    let snap = ran.tab(0).source.snapshot();
                    let logical = snap.schema().columns[0].logical.clone();
                    let v = snap.cell(0, 0).to_value();
                    bad.eq(
                        pg_kind(&logical),
                        kind.as_str(),
                        &format!("{id}: `{expr}` kind"),
                    );
                    bad.eq(
                        raw_of(&logical, &v, line.engine),
                        raw,
                        &format!("{id}: `{expr}`"),
                    );
                    checked += 1;
                }
            }
            DbmsKind::MySql | DbmsKind::MariaDb => {
                let (setup, cases) = my_cases();
                for sql in &setup {
                    c.ok(sql).await;
                }
                assert!(cases.len() >= 30, "{id}: {} cases", cases.len());
                for (column, label, kind, raw) in cases {
                    let ran = c
                        .run(&format!("SELECT {column} FROM hv_t07_types WHERE id = 1"))
                        .await;
                    if ran.outcome() != "ran" {
                        bad.0.push(format!("{id}: {column}: {}", ran.outcome()));
                        continue;
                    }
                    let snap = ran.tab(0).source.snapshot();
                    let col = snap.schema().columns[0].clone();
                    let v = snap.cell(0, 0).to_value();
                    bad.eq(
                        type_label(&col.native),
                        label,
                        &format!("{id}: {column} label"),
                    );
                    bad.eq(
                        my_kind(&col.logical),
                        kind.as_str(),
                        &format!("{id}: {column} kind"),
                    );
                    bad.eq(
                        raw_of(&col.logical, &v, line.engine),
                        raw,
                        &format!("{id}: {column}"),
                    );
                    checked += 1;
                }
            }
            _ => {
                let t = unique("hv_t31_m3b_types");
                c.ok(&format!(
                    "CREATE TABLE {t} (id INTEGER PRIMARY KEY, r REAL, t TEXT, b BLOB, \
                     n NUMERIC(10,2), flag BOOLEAN, d DATE, plain)"
                ))
                .await;
                c.ok(&format!(
                    "INSERT INTO {t} VALUES (1, 1.5, 'ş İ ı ğ Ü ç', x'00ff', 1.25, 1, \
                     '2026-11-14', 'x'), (2, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"
                ))
                .await;
                let ran = c.ok(&format!("SELECT * FROM {t} ORDER BY id")).await;
                let snap = ran.tab(0).source.snapshot();
                let want = [
                    Value::Int(1),
                    Value::F64(1.5),
                    Value::Text("ş İ ı ğ Ü ç".into()),
                    Value::Bytes(Arc::from(&[0x00_u8, 0xff][..])),
                    Value::F64(1.25),
                    Value::Bool(true),
                    Value::Text("2026-11-14".into()),
                    Value::Text("x".into()),
                ];
                for (k, w) in want.iter().enumerate() {
                    let g = snap.cell(0, u32::try_from(k).unwrap()).to_value();
                    bad.check(same(&g, w), || format!("{id}: column {k}: {g:?} != {w:?}"));
                    checked += 1;
                }
                let logical: Vec<LogicalType> = snap
                    .schema()
                    .columns
                    .iter()
                    .map(|c| c.logical.clone())
                    .collect();
                bad.eq(
                    (logical[4].clone(), logical[5].clone()),
                    (LogicalType::Float { bits: 64 }, LogicalType::Bool),
                    &format!("{id}: declared types"),
                );
                for k in 1..8u32 {
                    bad.eq(
                        snap.cell(1, k).to_value(),
                        Value::Null,
                        &format!("{id}: NULL column {k}"),
                    );
                }
            }
        }
        table.add(
            line,
            c.version.as_ref(),
            "type-goldens",
            &format!("{checked} values"),
        );
        c.shutdown().await;
    }
    bad.assert_none("type-goldens");
    table.finish("type-goldens", &lines);
}

// ---------------------------------------------------------------------------------------------
// 11. The gate over the real classifier

/// `(statement, production, development)`: the gate's decision on each kind of console of the
/// line's engine, over the real classifier in the session's context (P-M3b-16).
fn gate_rows(line: &Line) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut rows = vec![
        ("SELECT 1", "run", "run"),
        ("UPDATE t SET a = 1 WHERE id = 3", "tx-confirm", "run"),
        ("DELETE FROM müşteri", "confirm+type", "confirm"),
        ("INSERT INTO t VALUES (1)", "tx-confirm", "run"),
    ];
    match line.engine {
        DbmsKind::Postgres => rows.extend([
            ("DROP SCHEMA s CASCADE", "confirm+type", "run"),
            ("DROP TABLE t", "confirm", "run"),
            ("CALL p()", "confirm", "run"),
            ("SELECT * FROM t FOR UPDATE", "warn", "run"),
            ("TRUNCATE t", "confirm", "run"),
        ]),
        DbmsKind::MySql | DbmsKind::MariaDb => rows.extend([
            ("DROP DATABASE d", "confirm+type", "run"),
            ("DROP TABLE t", "confirm", "run"),
            ("CALL p()", "confirm", "run"),
            ("SELECT * FROM t FOR UPDATE", "warn", "run"),
            ("TRUNCATE TABLE t", "confirm", "run"),
        ]),
        _ => rows.extend([("DROP TABLE t", "confirm", "run")]),
    }
    rows
}

/// The gate end to end over the real `classify` (Task 15's `#[ignore]` test, turned on here on
/// the matrix; P-M3b-16): the shapes per engine on a live production console, a live
/// development console and a live read-only console (where every write is denied).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn gate_end_to_end_over_real_classify() {
    if !docker_enabled() {
        return;
    }
    let (pool, table, lines) = (pool(), Table::new(), lines());
    let mut bad = Bad::default();
    for line in &lines {
        let id = &line.id;
        let prod = Con::open(line, Spec::production(), &pool).await;
        let dev = Con::open(line, Spec::dev(), &pool).await;
        let ro = Con::open(
            line,
            Spec::read_only(pool.clone() as Arc<dyn StatementClassifier>),
            &pool,
        )
        .await;
        let rows = gate_rows(line);
        for (sql, on_prod, on_dev) in &rows {
            let p = prod.plan(sql, &[]).await;
            let d = dev.plan(sql, &[]).await;
            let r = ro.plan(sql, &[]).await;
            bad.eq(
                (p.shapes()[0], d.shapes()[0]),
                (*on_prod, *on_dev),
                &format!("{id}: `{sql}` (production, development)"),
            );
            let want_ro = if *sql == "SELECT 1" { "run" } else { "deny" };
            bad.eq(r.shapes()[0], want_ro, &format!("{id}: `{sql}` read-only"));
        }
        table.add(
            line,
            prod.version.as_ref(),
            "gate",
            &format!("{} statements x 3 consoles", rows.len()),
        );
        prod.shutdown().await;
        dev.shutdown().await;
        ro.shutdown().await;
    }
    bad.assert_none("gate");
    table.finish("gate", &lines);
}
