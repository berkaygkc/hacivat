//! M2b Task 18: guarded read-only execution and the role probe over `FakeDriver`
//! (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1; B21).
//!
//! The clean fakes script a guard that behaves like a server: the guard's `BEGIN` opens a
//! transaction, its canary answers SQL NULL and its `ROLLBACK` ends the transaction. A test's
//! own entries go ahead of that base (`RoFake::exec`), so a test can change one answer without
//! losing the rest.

mod common;

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::*;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{
    Driver as _, DriverConnect, Endpoint, ExecOpts, ExecSummary, InsecureAuth, RawConn, RawError,
    SemVerLoose, ServerInfo, TxStatus,
};
use hacivat_guard::{MAX_STATEMENT_BYTES, plan_for, role_probe_sql};
use hacivat_net::TlsMode;
use hacivat_proto::{AccessLabel, DbmsKind, Department};
use hacivat_session::pipeline::connect;
use hacivat_session::session::readonly::{access_for, remediation_for_server, run_guarded};
use hacivat_session::session::{
    DenyLayer, ExecErrorKind, ExecOrigin, Request, SessionEvent, SessionHandle, SessionState,
};
use hacivat_session::{GapReason, RoleWarning};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// A `FakeDriver` with a clean read-only base script that `exec` keeps.
struct RoFake {
    driver: FakeDriver,
    base: Vec<FakeExec>,
}

impl RoFake {
    fn new(kind: DbmsKind, base: Vec<FakeExec>) -> Self {
        RoFake {
            driver: FakeDriver::new(kind).exec(base.clone()),
            base,
        }
    }

    /// Puts `extra` ahead of the base script (the first matching entry is used).
    fn exec(self, extra: Vec<FakeExec>) -> Self {
        let script = extra.into_iter().chain(self.base.iter().cloned()).collect();
        RoFake {
            driver: self.driver.exec(script),
            base: self.base,
        }
    }

    fn into_driver(self) -> FakeDriver {
        self.driver
    }
}

impl Deref for RoFake {
    type Target = FakeDriver;

    fn deref(&self) -> &FakeDriver {
        &self.driver
    }
}

const CANARY: &str = "SELECT pg_catalog.txid_current_if_assigned()";
/// Fix round 1 (I-3): the probe's identity check.
const IDENTITY: &str = "SELECT current_user, session_user";

/// The identity check's answer for a session that did not switch roles.
fn same_role() -> FakeExec {
    rows(IDENTITY, &[&["hv_ro", "hv_ro"]])
}
const SCHEMA_PROBE: &str = "SELECT current_database()";

fn entry(prefix: &'static str, result: Result<ExecSummary, RawError>) -> FakeExec {
    FakeExec {
        prefix,
        result,
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// A statement that answers one row of one SQL NULL.
fn null_row(prefix: &'static str) -> FakeExec {
    entry(
        prefix,
        Ok(ExecSummary {
            rows: vec![vec![None]],
            row_count: 1,
            ..ExecSummary::default()
        }),
    )
}

/// A statement the server refuses with `sqlstate` (the status stays as it was).
fn err(prefix: &'static str, sqlstate: &str) -> FakeExec {
    entry(prefix, Err(db(sqlstate)))
}

/// A statement that reports `tag` with `n` affected rows.
fn wrote(prefix: &'static str, tag: &str, n: u64) -> FakeExec {
    entry(
        prefix,
        Ok(ExecSummary {
            command_tag: Some(tag.to_string()),
            rows_affected: Some(n),
            ..ExecSummary::default()
        }),
    )
}

/// The guard's answers for `rounds` PostgreSQL requests, and the schema probe's.
fn pg_clean_script(rounds: usize) -> Vec<FakeExec> {
    let mut script = Vec::new();
    for _ in 0..rounds {
        script.push(tx("BEGIN ISOLATION LEVEL", TxStatus::InTx));
        script.push(null_row(CANARY));
        script.push(tx("ROLLBACK", TxStatus::Idle));
    }
    for _ in 0..rounds {
        script.push(rows(
            SCHEMA_PROBE,
            &[&["hv_test", "app", "app", "hv_ro", "app"]],
        ));
    }
    script
}

fn fake_pg_ro_clean() -> RoFake {
    RoFake::new(DbmsKind::Postgres, pg_clean_script(4))
}

fn fake_mysql_ro_clean() -> RoFake {
    let mut script = Vec::new();
    for _ in 0..4 {
        script.push(tx("START TRANSACTION READ ONLY", TxStatus::InTx));
        script.push(tx("ROLLBACK", TxStatus::Idle));
    }
    RoFake::new(DbmsKind::MySql, script)
}

fn fake_mariadb() -> FakeDriver {
    FakeDriver::new(DbmsKind::MariaDb)
}

/// A PostgreSQL role the probe reads as least privilege, with TEMP (Q21: still enforced).
fn fake_pg_probe_least() -> FakeDriver {
    let probe = role_probe_sql(DbmsKind::Postgres);
    FakeDriver::new(DbmsKind::Postgres).exec(vec![
        same_role(),
        rows(probe[0], &[&["f", "f", "f", "f"]]),
        rows(probe[2], &[&["f"]]),
        rows(probe[3], &[&["t"]]),
    ])
}

/// What a PostgreSQL read-only connect runs: the identity check, then the probe.
fn pg_probe_lines() -> Vec<String> {
    std::iter::once(IDENTITY)
        .chain(role_probe_sql(DbmsKind::Postgres).iter().copied())
        .map(String::from)
        .collect()
}

fn statements(kind: DbmsKind) -> Vec<String> {
    plan_for(kind)
        .expect("a guard plan")
        .session_setup
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn pg_session_setup() -> Vec<String> {
    statements(DbmsKind::Postgres)
}

fn mysql_session_setup() -> Vec<String> {
    statements(DbmsKind::MySql)
}

/// A raw connection of `f`, outside any pipeline (so nothing ran on it yet).
async fn conn(f: &FakeDriver) -> Box<dyn RawConn> {
    f.connect(DriverConnect {
        endpoint: Endpoint::File {
            path: PathBuf::from("t18.db"),
            create_if_missing: true,
            read_only: true,
        },
        tls: None,
        user: None,
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only: true,
        options: Vec::new(),
        connect_timeout: Duration::from_secs(5),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::default(),
    })
    .await
    .expect("the fake connects")
}

/// A read-only PostgreSQL session over `f`, connected through the pipeline (which probes the
/// role: the clean fake answers nothing, so the label is guarded).
async fn ro_session(f: RoFake) -> (SessionHandle, Arc<FakeDriver>) {
    spawn_over(f.into_driver(), ro_profile(), true, Arc::new(fake_pg())).await
}

fn user_exec(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    }
}

async fn guarded_on(
    f: &FakeDriver,
    kind: DbmsKind,
    sql: &str,
) -> Result<ExecSummary, ExecErrorKind> {
    let mut c = conn(f).await;
    run_guarded(&mut *c, kind, sql, ExecOpts::default(), &[])
        .await
        .map_err(|e| e.kind)
}

// ---------------------------------------------------------------------------------------------
// The brief's tests

#[tokio::test]
async fn pg_guarded_exec_sequence() {
    let f = fake_pg_ro_clean();
    let mut c = conn(&f).await;
    // replay = the four session_setup statements (Q15)
    run_guarded(
        &mut *c,
        DbmsKind::Postgres,
        "select * from t",
        ExecOpts::default(),
        &pg_session_setup(),
    )
    .await
    .unwrap();
    assert_eq!(
        f.executed(),
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SELECT 1",
            "select * from t",
            "SELECT pg_catalog.txid_current_if_assigned()",
            "ROLLBACK",
            "SET default_transaction_read_only = on",
            "SET standard_conforming_strings = on",
            "SET client_encoding = 'UTF8'",
            "SET idle_in_transaction_session_timeout = '30s'"
        ]
    );
    assert_eq!(f.reset_calls(), 1);
}

#[tokio::test]
async fn smuggled_commit_is_an_enforcement_gap_and_discards_the_connection() {
    let (h, f) =
        ro_session(fake_pg_ro_clean().exec(vec![tx("SELECT 1; COMMIT", TxStatus::Idle)])).await;
    let mut events = h.events();
    // The connect's role probe reset once (fix round 2, I-3b); the gap adds none.
    let resets = f.reset_calls();
    let r = h.submit(user_exec("SELECT 1; COMMIT")).reply.await.unwrap();
    // smuggled past 1a
    assert_eq!(
        r.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement)
    );
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
    // The owner hears about it, and the connection was never reset for reuse.
    let mut saw_gap = false;
    while let Ok(e) = events.try_recv() {
        saw_gap |= e == SessionEvent::EnforcementGap(GapReason::TransactionEndedByStatement);
    }
    assert!(saw_gap);
    assert_eq!(f.reset_calls(), resets);
    let later = h.submit(user_exec("select 1")).reply.await.unwrap();
    assert_eq!(later.unwrap_err().kind, ExecErrorKind::NotConnected);
}

#[tokio::test]
async fn canary_non_null_is_a_gap() {
    let f = fake_pg_ro_clean().exec(vec![rows(CANARY, &[&["731"]])]);
    let mut c = conn(&f).await;
    assert_eq!(
        run_guarded(
            &mut *c,
            DbmsKind::Postgres,
            "select 1",
            ExecOpts::default(),
            &[]
        )
        .await
        .unwrap_err()
        .kind,
        ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned)
    );
}

#[tokio::test]
async fn transaction_keywords_are_denied_before_io() {
    // Q35
    for sql in [
        "COMMIT",
        "  /* x */ begin",
        "SAVEPOINT a",
        "release a",
        "END",
    ] {
        let f = fake_pg_ro_clean();
        let mut c = conn(&f).await;
        assert_eq!(
            run_guarded(&mut *c, DbmsKind::Postgres, sql, ExecOpts::default(), &[])
                .await
                .unwrap_err()
                .kind,
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{sql}"
        );
        assert!(f.executed().is_empty());
    }
}

#[tokio::test]
async fn schema_survives_a_guarded_request() {
    // Q15
    let (h, f) = ro_session(fake_pg_ro_clean()).await;
    h.submit(Request::SwitchSchema("app".into()))
        .reply
        .await
        .unwrap()
        .unwrap();
    run(&h, "select 1").await.unwrap();
    assert_eq!(f.executed().last().unwrap(), "SET search_path TO \"app\"");
}

#[tokio::test]
async fn input_over_1mib_rejected_before_io() {
    let f = fake_pg_ro_clean();
    let mut c = conn(&f).await;
    let big = "x".repeat(MAX_STATEMENT_BYTES + 1);
    assert_eq!(
        run_guarded(&mut *c, DbmsKind::Postgres, &big, ExecOpts::default(), &[])
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Input)
    );
    assert!(f.executed().is_empty());
}

#[tokio::test]
async fn statement_error_still_rolls_back_and_resets() {
    let f = fake_pg_ro_clean().exec(vec![err("insert into t values (1)", "25006")]);
    let mut c = conn(&f).await;
    let e = run_guarded(
        &mut *c,
        DbmsKind::Postgres,
        "insert into t values (1)",
        ExecOpts::default(),
        &[],
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    // The server's message travels with it.
    assert_eq!(e.server.and_then(|m| m.sqlstate).as_deref(), Some("25006"));
    assert!(f.executed().contains(&"ROLLBACK".to_string()));
    assert_eq!(f.reset_calls(), 1);
}

#[tokio::test]
async fn mysql_guarded_sequence() {
    let f = fake_mysql_ro_clean();
    let mut c = conn(&f).await;
    run_guarded(
        &mut *c,
        DbmsKind::MySql,
        "select 1",
        ExecOpts::default(),
        &mysql_session_setup(),
    )
    .await
    .unwrap();
    assert_eq!(
        f.executed(),
        [
            "START TRANSACTION READ ONLY",
            "select 1",
            "ROLLBACK",
            "SET SESSION TRANSACTION READ ONLY",
            "SET NAMES utf8mb4"
        ]
    );
    assert_eq!(f.reset_calls(), 1);
}

#[tokio::test]
async fn access_for_probes_only_when_read_only() {
    let f = fake_pg_probe_least();
    let mut c = conn(&f).await;
    assert_eq!(
        access_for(&mut *c, DbmsKind::Postgres, false).await.0,
        AccessLabel::ReadWrite
    );
    assert!(f.executed().is_empty());
    assert_eq!(
        access_for(&mut *c, DbmsKind::Postgres, true).await.0,
        AccessLabel::ReadOnlyEnforced
    );
    assert_eq!(
        access_for(&mut *conn(&fake_mariadb()).await, DbmsKind::MariaDb, true)
            .await
            .0,
        AccessLabel::ReadOnlyGuarded
    );
}

// ---------------------------------------------------------------------------------------------
// Step 1a: the first keyword

#[tokio::test]
async fn mysql_executable_comments_are_statement_text() {
    // T13 carry: MySQL runs `/*!…*/` (and MariaDB `/*M!…*/`), so its content is the statement.
    for (kind, sql) in [
        (DbmsKind::MySql, "/*!COMMIT*/"),
        (DbmsKind::MySql, "/*!50000 commit */"),
        (DbmsKind::MySql, "/*!80000ROLLBACK*/"),
        (DbmsKind::MariaDb, "/*M!100100 BEGIN */"),
        (DbmsKind::MariaDb, "/*M! BEGIN */"),
        (DbmsKind::MySql, "/*! */ /*!START TRANSACTION*/"),
        (DbmsKind::MySql, "-- note\n/*!50000 SAVEPOINT a*/"),
    ] {
        let f = fake_mysql_ro_clean();
        assert_eq!(
            guarded_on(&f, kind, sql).await.unwrap_err(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{kind:?} {sql}"
        );
        assert!(f.executed().is_empty(), "{sql}");
    }
}

#[tokio::test]
async fn step_1a_reads_mysql_comments_as_the_server_does() {
    // Fix round 1 (I-1, review H1): each of these is `COMMIT` to the server, so each is
    // refused before any I/O.
    for (kind, sql) in [
        // A version-gated comment newer than the server is skipped: read both ways.
        (DbmsKind::MySql, "/*!99999 SELECT 1 */ COMMIT"),
        (DbmsKind::MariaDb, "/*!999999 SELECT 1 */ COMMIT"),
        (DbmsKind::MariaDb, "/*M!999999 SELECT 1 */ COMMIT"),
        // `/*M!` is an ordinary comment on MySQL.
        (DbmsKind::MySql, "/*M!100100 SELECT 1 */ COMMIT"),
        // MySQL line comments end at a line feed only.
        (DbmsKind::MySql, "-- x\rSELECT 1\nCOMMIT"),
        (DbmsKind::MySql, "# x\rSELECT 1\nCOMMIT"),
        // Both readings of a version-gated comment, one after another.
        (DbmsKind::MySql, "/*!50000 COMMIT */ SELECT 1"),
        (
            DbmsKind::MySql,
            "/*!50000 SELECT 1 */ /*!50000 ROLLBACK */ SELECT 2",
        ),
        // Fix round 2 (I-1b): a skipped version-gated comment may hold one nested comment
        // (`consume_comment(lip, 1)`), so the server reads `COMMIT` after the second `*/`.
        (DbmsKind::MySql, "/*!99999 /* x */ SELECT 1 */ COMMIT"),
        (DbmsKind::MariaDb, "/*!999999 /* x */ SELECT 1 */ COMMIT"),
        (DbmsKind::MariaDb, "/*M!999999 /* x */ SELECT 1 */ COMMIT"),
        (
            DbmsKind::MySql,
            "/*!99999 /*!40000 x */ SELECT 1 */ ROLLBACK",
        ),
        // Only one level: a second `/*` inside the nested one is text, so its `*/` ends the
        // nested comment and the next ends the version-gated one.
        (DbmsKind::MySql, "/*!99999 /* a /* b */ SELECT 1 */ BEGIN"),
    ] {
        let f = fake_mysql_ro_clean();
        let mut c = conn(&f).await;
        let r = run_guarded(&mut *c, kind, sql, ExecOpts::default(), &[]).await;
        assert_eq!(
            r.unwrap_err().kind,
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{kind:?} {sql:?}"
        );
        assert!(f.executed().is_empty(), "{kind:?} {sql:?}");
    }
    // Too many version-gated comments to follow every reading: refused, never guessed. A
    // statement near the 1 MiB limit made of them is refused the same way.
    for count in [17, 45_000] {
        let sql = "/*!50000 SELECT 1 */ ".repeat(count) + "SELECT 2";
        let f = fake_mysql_ro_clean();
        assert_eq!(
            guarded_on(&f, DbmsKind::MySql, &sql).await.unwrap_err(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{count}"
        );
        assert!(f.executed().is_empty());
    }
    // A chain of version-gated comments is followed once per comment, not once per path
    // (review n-1): sixteen of them are read; seventeen pass the fork bound and are refused.
    let chain = |n: usize| "/*!1 */ ".repeat(n) + "SELECT 1";
    let f = fake_mysql_ro_clean();
    assert!(guarded_on(&f, DbmsKind::MySql, &chain(16)).await.is_ok());
    let f = fake_mysql_ro_clean();
    assert_eq!(
        guarded_on(&f, DbmsKind::MySql, &chain(17))
            .await
            .unwrap_err(),
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction)
    );
    // What the server reads as a read stays a read.
    for (kind, sql) in [
        // MySQL: `/*M!` is a comment, so its `COMMIT` is never run.
        (DbmsKind::MySql, "/*M!100100 COMMIT */ SELECT 1"),
        // A skipped version-gated comment holds one nested level, not two: the server reads
        // `SELECT 2` after it (and `SELECT 1` when it runs it).
        (DbmsKind::MySql, "/*!99999 /* x */ SELECT 1 */ SELECT 2"),
        // A plain comment does not nest on MySQL: `COMMIT */` is never reached as a keyword,
        // the statement is `SELECT 1 */ …` (a syntax error, never a commit).
        (DbmsKind::MySql, "/* /* */ SELECT 1 */ COMMIT"),
        // MySQL: the comment runs to the line feed, so the statement is `SELECT 1`.
        (DbmsKind::MySql, "-- x\rCOMMIT\nSELECT 1"),
        // SQLite ends a line comment at a line feed too.
        (DbmsKind::Sqlite, "-- x\rCOMMIT\nSELECT 1"),
    ] {
        let f = if kind == DbmsKind::Sqlite {
            RoFake::new(kind, Vec::new())
        } else {
            fake_mysql_ro_clean()
        };
        assert!(guarded_on(&f, kind, sql).await.is_ok(), "{kind:?} {sql:?}");
    }
}

#[tokio::test]
async fn comment_rules_follow_the_engine() {
    // PostgreSQL: `/*!` is an ordinary comment, and block comments nest.
    let f = fake_pg_ro_clean();
    assert!(
        guarded_on(&f, DbmsKind::Postgres, "/*! COMMIT */ select 1")
            .await
            .is_ok()
    );
    for sql in [
        "/* a /* nested */ still a comment */ COMMIT",
        ";COMMIT",
        " ; ;\n\t rollback",
        "-- x\rABORT",
        "\u{b}END",
        "begin;",
    ] {
        let f = fake_pg_ro_clean();
        assert_eq!(
            guarded_on(&f, DbmsKind::Postgres, sql).await.unwrap_err(),
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{sql:?}"
        );
    }
    // MySQL: `#` starts a comment; `--` does only before whitespace.
    let f = fake_mysql_ro_clean();
    assert_eq!(
        guarded_on(&f, DbmsKind::MySql, "# x\nCOMMIT")
            .await
            .unwrap_err(),
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction)
    );
    // A word that only starts like a transaction keyword is not one.
    let f = fake_pg_ro_clean();
    assert!(
        guarded_on(&f, DbmsKind::Postgres, "select commit from t")
            .await
            .is_ok()
    );
    let f = fake_pg_ro_clean();
    assert!(
        guarded_on(&f, DbmsKind::Postgres, "endpoints")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn sqlite_runs_steps_1_and_1a_then_the_statement() {
    let f = FakeDriver::new(DbmsKind::Sqlite);
    let mut c = conn(&f).await;
    run_guarded(
        &mut *c,
        DbmsKind::Sqlite,
        "select 1",
        ExecOpts::default(),
        &[],
    )
    .await
    .unwrap();
    assert_eq!(f.executed(), ["select 1"]);
    assert_eq!(f.reset_calls(), 0);
    assert_eq!(
        guarded_on(&f, DbmsKind::Sqlite, "BEGIN IMMEDIATE")
            .await
            .unwrap_err(),
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction)
    );
    // The post-check is still a tripwire: a write that succeeded is a gap.
    let f = FakeDriver::new(DbmsKind::Sqlite).exec(vec![wrote("insert", "INSERT", 1)]);
    assert_eq!(
        guarded_on(&f, DbmsKind::Sqlite, "insert into t values (1)")
            .await
            .unwrap_err(),
        ExecErrorKind::EnforcementGap(GapReason::WriteSucceeded)
    );
}

// ---------------------------------------------------------------------------------------------
// Failing closed

#[tokio::test]
async fn an_engine_without_a_guard_plan_runs_nothing() {
    let f = FakeDriver::new(DbmsKind::Unknown);
    assert_eq!(
        guarded_on(&f, DbmsKind::Unknown, "select 1")
            .await
            .unwrap_err(),
        ExecErrorKind::Internal
    );
    assert!(f.executed().is_empty());
}

#[tokio::test]
async fn a_failed_before_step_runs_nothing_else() {
    let f = fake_pg_ro_clean().exec(vec![err("BEGIN", "53300")]);
    assert_eq!(
        guarded_on(&f, DbmsKind::Postgres, "select 1")
            .await
            .unwrap_err(),
        ExecErrorKind::Db
    );
    assert_eq!(
        f.executed(),
        ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY"]
    );
    assert_eq!(f.reset_calls(), 0);
}

#[tokio::test]
async fn a_before_step_that_leaves_a_transaction_open_breaks_the_session() {
    // `BEGIN` ran, `SELECT 1` failed: the guard's transaction is still open, so the session is
    // never used again.
    let (h, f) =
        ro_session(fake_pg_ro_clean().exec(vec![tx_err("SELECT 1", "57014", TxStatus::Failed)]))
            .await;
    assert!(run(&h, "select 2").await.is_err());
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
    assert!(!f.executed().iter().any(|s| s == "select 2"));
}

#[tokio::test]
async fn the_statement_never_runs_without_the_guard_transaction() {
    // Fix round 1 (m-1, review H3): a `before` that did not open the read-only transaction is a
    // gap, and the user statement is never sent.
    for (kind, begin) in [
        (DbmsKind::MySql, "START TRANSACTION READ ONLY"),
        (DbmsKind::MariaDb, "START TRANSACTION READ ONLY"),
        (DbmsKind::Postgres, "BEGIN ISOLATION LEVEL"),
    ] {
        let f = FakeDriver::new(kind).exec(vec![tx(begin, TxStatus::Idle)]);
        assert_eq!(
            guarded_on(&f, kind, "delete from t").await.unwrap_err(),
            ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement),
            "{kind:?}"
        );
        assert!(
            !f.executed().iter().any(|s| s == "delete from t"),
            "{kind:?}: {:?}",
            f.executed()
        );
        assert_eq!(f.reset_calls(), 0, "{kind:?}");
    }
}

#[tokio::test]
async fn a_failed_mysql_call_that_ended_the_transaction_is_a_gap() {
    // Fix round 1 (I-2, review H2): a procedure may commit, write, commit and then fail. The
    // failed statement is still judged: no transaction afterwards is a gap.
    for kind in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let f = fake_mysql_ro_clean().exec(vec![tx_err("CALL", "45000", TxStatus::Idle)]);
        assert_eq!(
            guarded_on(&f, kind, "CALL p()").await.unwrap_err(),
            ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement),
            "{kind:?}"
        );
        assert_eq!(f.reset_calls(), 0, "discarded unreset");
    }
    // Any keyword outside the implicit-commit list is judged, and so is a statement one of
    // whose readings is outside it.
    for sql in [
        "DO p()",
        "/*!99999 CREATE TABLE x (a int) */ CALL p()",
        "(SELECT 1)",
        "SET autocommit = 1",
    ] {
        let f = mysql_failing("45000");
        assert_eq!(
            guarded_on(&f, DbmsKind::MySql, sql).await.unwrap_err(),
            ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement),
            "{sql}"
        );
    }
}

/// A MySQL guard whose user statement (whatever its text) fails with `code` and leaves no
/// transaction, as a statement that committed (implicitly or not) before failing does.
fn mysql_failing(code: &str) -> FakeDriver {
    // First match wins: `START TRANSACTION` takes its own entry, the user statement the
    // empty-prefix one, `ROLLBACK` the last.
    FakeDriver::new(DbmsKind::MySql).exec(vec![
        tx("START TRANSACTION READ ONLY", TxStatus::InTx),
        tx_err("", code, TxStatus::Idle),
        tx("ROLLBACK", TxStatus::Idle),
    ])
}

#[tokio::test]
async fn a_failed_mysql_ddl_that_committed_implicitly_is_only_an_error() {
    // MySQL commits before a DDL statement and then refuses it (1792): the statement failed,
    // nothing escaped, and the session stays usable.
    for sql in [
        "CREATE TABLE x (a int)",
        "drop table t",
        "/*! ALTER TABLE t ADD b int */",
        // Review n-4: skipped, a version-gated comment that holds the whole statement leaves
        // none (an empty query, no commit), so only its DDL reading counts.
        "/*!50000 ALTER TABLE t ADD b int */",
        "TRUNCATE t",
        "GRANT SELECT ON t TO u",
        "RENAME TABLE t TO u",
        "RESET MASTER",
        "CHANGE REPLICATION SOURCE TO SOURCE_HOST = 'h'",
        "STOP REPLICA",
    ] {
        let f = mysql_failing("1792");
        assert_eq!(
            guarded_on(&f, DbmsKind::MySql, sql).await.unwrap_err(),
            ExecErrorKind::Db,
            "{sql}"
        );
        assert_eq!(f.reset_calls(), 1, "{sql}");
    }
}

#[tokio::test]
async fn a_mysql_write_that_succeeded_is_a_gap() {
    let f = fake_mysql_ro_clean().exec(vec![wrote("CALL", "CALL", 1)]);
    assert_eq!(
        guarded_on(&f, DbmsKind::MySql, "CALL p()")
            .await
            .unwrap_err(),
        ExecErrorKind::EnforcementGap(GapReason::WriteSucceeded)
    );
}

#[tokio::test]
async fn the_rollback_notice_of_an_ended_transaction_is_a_gap() {
    let rollback = entry(
        "ROLLBACK",
        Ok(ExecSummary {
            notice_codes: vec!["25P01".into()],
            ..ExecSummary::default()
        }),
    );
    let f = fake_pg_ro_clean().exec(vec![rollback]);
    assert_eq!(
        guarded_on(&f, DbmsKind::Postgres, "select 1")
            .await
            .unwrap_err(),
        ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement)
    );
    assert_eq!(f.reset_calls(), 0, "a gap discards the connection unreset");
}

#[tokio::test]
async fn an_unreadable_canary_fails_closed() {
    for canary in [
        entry(CANARY, Ok(ExecSummary::default())),
        err(CANARY, "57014"),
        rows(CANARY, &[&["1", "2"]]),
    ] {
        let (h, f) = ro_session(fake_pg_ro_clean().exec(vec![canary])).await;
        assert_eq!(
            run(&h, "select 1").await.unwrap_err().kind,
            ExecErrorKind::Internal
        );
        assert!(matches!(snap(&h).state, SessionState::Broken(_)));
        assert_eq!(f.open_connections(), 0);
    }
}

#[tokio::test]
async fn a_pg_error_that_left_no_transaction_is_a_gap() {
    // On PostgreSQL an error inside the guard's transaction leaves it failed; no transaction at
    // all means the statement ended it.
    let f = fake_pg_ro_clean().exec(vec![tx_err("select boom", "XX000", TxStatus::Idle)]);
    assert_eq!(
        guarded_on(&f, DbmsKind::Postgres, "select boom")
            .await
            .unwrap_err(),
        ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement)
    );
}

#[tokio::test]
async fn a_failed_reset_breaks_the_session() {
    let (h, f) = ro_session(RoFake {
        driver: fake_pg().reset_fails().exec(pg_clean_script(2)),
        base: Vec::new(),
    })
    .await;
    assert!(run(&h, "select 1").await.is_err());
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
}

#[tokio::test]
async fn a_cancelled_statement_still_rolls_back_and_resets() {
    let f = fake_pg_ro_clean().exec(vec![tx_err("select pg_sleep", "57014", TxStatus::Failed)]);
    let mut c = conn(&f).await;
    let e = run_guarded(
        &mut *c,
        DbmsKind::Postgres,
        "select pg_sleep(10)",
        ExecOpts::default(),
        &pg_session_setup(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    // A failed transaction cannot run the canary; the rollback, reset and replay still run.
    assert!(!f.executed().iter().any(|s| s == CANARY));
    assert_eq!(&f.executed()[3], "ROLLBACK");
    assert_eq!(f.executed().len(), 8);
    assert_eq!(f.reset_calls(), 1);
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

// ---------------------------------------------------------------------------------------------
// The role probe and the label

#[tokio::test]
async fn a_probe_that_fails_is_unparsed_and_guarded() {
    let probe = role_probe_sql(DbmsKind::Postgres);
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![
        same_role(),
        rows(probe[0], &[&["f", "f", "f", "f"]]),
        err(probe[1], "42501"),
    ]);
    let (label, warnings) = access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await;
    assert_eq!(label, AccessLabel::ReadOnlyGuarded);
    assert_eq!(warnings, [RoleWarning::Unparsed]);
    // It stopped at the failure.
    assert_eq!(f.executed().len(), 3);
}

#[tokio::test]
async fn the_probe_runs_its_fixed_statements_only() {
    let f = fake_pg_probe_least();
    let (label, warnings) = access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await;
    assert_eq!(label, AccessLabel::ReadOnlyEnforced);
    // Q21: TEMP is a warning that keeps the enforced label.
    assert_eq!(warnings, [RoleWarning::TempPrivilege]);
    assert_eq!(f.executed(), pg_probe_lines());
}

#[tokio::test]
async fn a_switched_role_reads_guarded() {
    // Fix round 1 (I-3): with `-c role=…` or a role default, `current_user` is not the session
    // user, and `SET ROLE NONE` returns to the session user, whom the probe never saw. Such a
    // session is `Unparsed` (guarded), and the probe does not run.
    for answer in [
        rows(IDENTITY, &[&["app_reader", "app_login"]]),
        entry(IDENTITY, Ok(ExecSummary::default())),
        err(IDENTITY, "42501"),
        rows(IDENTITY, &[&["hv_ro"]]),
    ] {
        let probe = role_probe_sql(DbmsKind::Postgres);
        let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![
            answer,
            rows(probe[0], &[&["f", "f", "f", "f"]]),
            rows(probe[2], &[&["f"]]),
            rows(probe[3], &[&["f"]]),
        ]);
        let (label, warnings) = access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await;
        assert_eq!(label, AccessLabel::ReadOnlyGuarded);
        assert_eq!(warnings, [RoleWarning::Unparsed]);
        assert_eq!(f.executed(), [IDENTITY]);
    }
    // A NULL is not a name.
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![entry(
        IDENTITY,
        Ok(ExecSummary {
            rows: vec![vec![None, None]],
            row_count: 1,
            ..ExecSummary::default()
        }),
    )]);
    assert_eq!(
        access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await,
        (AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Unparsed])
    );
}

#[tokio::test]
async fn the_probe_resets_before_its_identity_check() {
    // Fix round 2 (I-3b): the identity the probe judges is the one every guarded request runs
    // as after its own reset (`SET SESSION AUTHORIZATION DEFAULT`, `RESET ALL`).
    let f = fake_pg_probe_least();
    let mut c = conn(&f).await;
    assert_eq!(
        access_for(&mut *c, DbmsKind::Postgres, true).await.0,
        AccessLabel::ReadOnlyEnforced
    );
    assert_eq!(f.reset_calls(), 1);
    assert_eq!(f.executed(), pg_probe_lines());
    // A reset that fails leaves the identity unknown: guarded, and nothing else runs.
    let f = FakeDriver::new(DbmsKind::Postgres)
        .reset_fails()
        .exec(vec![same_role()]);
    assert_eq!(
        access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await,
        (AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Unparsed])
    );
    assert!(f.executed().is_empty());
    // MySQL, read-write and side connections reset nothing here.
    let f = FakeDriver::new(DbmsKind::MySql);
    let _ = access_for(&mut *conn(&f).await, DbmsKind::MySql, true).await;
    let _ = access_for(&mut *conn(&f).await, DbmsKind::MySql, false).await;
    assert_eq!(f.reset_calls(), 0);
}

#[tokio::test]
async fn a_read_only_connect_refuses_a_role_switch_in_its_options() {
    // Fix round 2 (I-3b): `-c role=` or `-c session_authorization=` in a read-only connect's
    // options is a visible configuration issue, before any I/O; a read-write connect keeps it.
    let with_role =
        |p: hacivat_session::profile::Profile, value: &str| hacivat_session::profile::Profile {
            extra: vec![("options".into(), value.into())],
            ..p
        };
    let rejected = hacivat_session::pipeline::IssueCode::OptionRejected {
        key: "options".into(),
        reason: hacivat_driver::ViolationReason::AlwaysDenied,
    };
    // Fix round 3 (I-3c): any shape but `-c name=value`, `-cname=value` or `--name=value` too,
    // such as `getopt`-combined flags.
    for value in [
        "-c role=hv_ro",
        "-c session_authorization=hv_ro",
        "-ncrole=hv_ro",
        "-nc role=hv_ro",
        "-n-role=hv_ro",
        "-Pcsession_authorization=hv_ro",
        "-e",
    ] {
        // A read-only profile.
        let fake = Arc::new(fake_pg());
        let e = err_with(&fake, with_role(ro_profile(), value)).await;
        assert_eq!(e.class, hacivat_proto::ErrClass::ConfigInvalid, "{value}");
        assert!(e.issues.iter().any(|i| i.code == rejected), "{value}");
        assert_eq!(fake.connect_calls(), 0, "{value}");
        // A read-only session of a read-write profile.
        let (deps, fake) = deps_with(fake_pg());
        let mut request = req(with_role(rw(pg_profile(TlsMode::Disable)), value));
        request.read_only = true;
        let e = connect(&deps, request, ix_none(), token(), progress())
            .await
            .expect_err("refused");
        assert_eq!(e.class, hacivat_proto::ErrClass::ConfigInvalid, "{value}");
        assert!(e.issues.iter().any(|i| i.code == rejected), "{value}");
        assert_eq!(fake.connect_calls(), 0, "{value}");
    }
    // Read-write keeps `-c role=`.
    let fake = Arc::new(fake_pg());
    let live = ok_with(
        &fake,
        with_role(rw(pg_profile(TlsMode::Disable)), "-c role=hv_ro"),
    )
    .await;
    assert_eq!(
        fake.last_request().unwrap().options,
        [("options".to_string(), "-c role=hv_ro".to_string())]
    );
    live.close().await;
}

#[tokio::test(start_paused = true)]
async fn a_slow_probe_is_unparsed_and_guarded() {
    // Review H4: the 5 s bound (FakeDriver only, so the paused clock is safe).
    let probe = role_probe_sql(DbmsKind::Postgres);
    let slow = FakeExec {
        delay: Duration::from_secs(6),
        ..rows(probe[0], &[&["f", "f", "f", "f"]])
    };
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![same_role(), slow]);
    assert_eq!(
        access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await,
        (AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Unparsed])
    );
}

#[tokio::test]
async fn side_connections_run_no_probe() {
    // Fix round 1 (m-2, review H5): a cancel or kill connection runs no user SQL and shows no
    // label, so it is labelled guarded without a probe.
    let (deps, fake) = deps_with(fake_pg());
    let mut request = req(ro_profile());
    request.read_only = true;
    request.department = Department::Cancel;
    let live = connect(&deps, request, ix_none(), token(), progress())
        .await
        .expect("connects");
    assert!(fake.executed().is_empty(), "{:?}", fake.executed());
    assert_eq!(live.access, AccessLabel::ReadOnlyGuarded);
    assert!(live.access_warnings.is_empty());
    live.close().await;
    // A read-write side connection stays read-write.
    let (deps, fake) = deps_with(fake_pg());
    let mut request = req(rw(pg_profile(TlsMode::Disable)));
    request.department = Department::Cancel;
    let live = connect(&deps, request, ix_none(), token(), progress())
        .await
        .expect("connects");
    assert!(fake.executed().is_empty());
    assert_eq!(live.access, AccessLabel::ReadWrite);
    live.close().await;
}

#[tokio::test]
async fn a_superuser_reads_guarded_with_its_warning() {
    let probe = role_probe_sql(DbmsKind::Postgres);
    let f = FakeDriver::new(DbmsKind::Postgres).exec(vec![
        same_role(),
        rows(probe[0], &[&["t", "t", "f", "f"]]),
        rows(probe[2], &[&["t"]]),
        rows(probe[3], &[&["t"]]),
    ]);
    let (label, warnings) = access_for(&mut *conn(&f).await, DbmsKind::Postgres, true).await;
    assert_eq!(label, AccessLabel::ReadOnlyGuarded);
    assert!(warnings.contains(&RoleWarning::Superuser));
    assert!(warnings.contains(&RoleWarning::WritePrivileges));
}

#[tokio::test]
async fn mysql_probe_reads_show_grants() {
    let least = FakeDriver::new(DbmsKind::MySql).exec(vec![rows(
        "SHOW GRANTS",
        &[
            &["GRANT USAGE ON *.* TO `hv_ro`@`%`"],
            &["GRANT SELECT, SHOW VIEW ON `hv_test`.* TO `hv_ro`@`%`"],
        ],
    )]);
    assert_eq!(
        access_for(&mut *conn(&least).await, DbmsKind::MySql, true).await,
        (AccessLabel::ReadOnlyEnforced, Vec::new())
    );
    let wide = FakeDriver::new(DbmsKind::MySql).exec(vec![rows(
        "SHOW GRANTS",
        &[&["GRANT ALL PRIVILEGES ON *.* TO `root`@`%`"]],
    )]);
    assert_eq!(
        access_for(&mut *conn(&wide).await, DbmsKind::MySql, true).await,
        (AccessLabel::ReadOnlyGuarded, vec![RoleWarning::Superuser])
    );
}

#[tokio::test]
async fn mariadb_and_sqlite_are_guarded_without_a_probe() {
    for kind in [DbmsKind::MariaDb, DbmsKind::Sqlite] {
        let f = FakeDriver::new(kind);
        assert_eq!(
            access_for(&mut *conn(&f).await, kind, true).await,
            (AccessLabel::ReadOnlyGuarded, Vec::new()),
            "{kind:?}"
        );
        assert!(f.executed().is_empty(), "{kind:?}");
    }
    // A MySQL profile whose server turned out to be MariaDB is MariaDB (C19).
    let f = FakeDriver::new(DbmsKind::MariaDb);
    assert_eq!(
        access_for(&mut *conn(&f).await, DbmsKind::MySql, true).await,
        (AccessLabel::ReadOnlyGuarded, Vec::new())
    );
    assert!(f.executed().is_empty());
}

#[tokio::test]
async fn the_connect_labels_a_read_only_session_from_the_probe() {
    let probe = role_probe_sql(DbmsKind::Postgres);
    let fake = fake_pg().exec(vec![
        same_role(),
        rows(probe[0], &[&["f", "f", "f", "f"]]),
        rows(probe[2], &[&["f"]]),
        rows(probe[3], &[&["f"]]),
    ]);
    let (h, f) = spawn_over(fake, ro_profile(), true, Arc::new(fake_pg())).await;
    assert_eq!(snap(&h).access, AccessLabel::ReadOnlyEnforced);
    assert_eq!(f.executed(), pg_probe_lines());
}

#[test]
fn remediation_follows_the_server_version() {
    let info = |kind, raw: &str| ServerInfo {
        kind,
        product: String::new(),
        version: SemVerLoose::parse(raw),
        raw_version: raw.into(),
        is_replica: None,
    };
    let pg13 = remediation_for_server(&info(DbmsKind::Postgres, "13.16")).unwrap();
    assert!(pg13.contains("<schema>") && !pg13.contains("pg_read_all_data"));
    let pg17 = remediation_for_server(&info(DbmsKind::Postgres, "17.2")).unwrap();
    assert!(pg17.contains("pg_read_all_data") && !pg17.contains("<schema>"));
    // An unknown version gets the current form.
    assert_eq!(
        remediation_for_server(&info(DbmsKind::Postgres, "")),
        Some(pg17)
    );
    assert!(remediation_for_server(&info(DbmsKind::MySql, "8.4.3")).is_some());
    assert!(remediation_for_server(&info(DbmsKind::MariaDb, "11.4.4")).is_none());
}
