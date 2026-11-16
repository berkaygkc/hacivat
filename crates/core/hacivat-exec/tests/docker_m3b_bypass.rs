//! SQL-SAFE-06 run A′ (M3b Task 18; P-M3b-13): the bypass catalogue (`sql-testkit/bypass/*.toml`)
//! sent to real servers through read-only sessions whose guard asks the real `ClassifyPool`
//! (step 0b, R-M3-83), as the app's boot installs it. Every row the classifier denies must end
//! `SafetyDenied` with the side channel unchanged (nothing reached the server); every row it
//! allows must meet the guard's layers (L3–L6) exactly as run B's rows do, never a classifier
//! denial (no false denial in the session's real context). The context is the connection's own
//! (plan amendment C2(b)/I1): PostgreSQL's `search_path` read at connect lets an unqualified
//! builtin through; MySQL's `sql_mode`, read in the connect's own round trip, reads a backslash
//! escape as the server does. Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p
//! hacivat-exec --test docker_m3b_bypass` (`pr`: pg17 and mysql84; `full`: pg14–18, mysql80/84,
//! mariadb106/1011/114).
//!
//! The offline part (a real pool over a fake server) runs on every PR.

#![allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use common::matrix::{Line, Side, docker_enabled, server_lines};
use common::t18::{RoConsole, safety_denied};
use hacivat_driver::SessionSettings;
use hacivat_driver::fake::{FakeCall, FakeDriver};
use hacivat_exec::classify_pool::ClassifyPool;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{ExecErrorKind, Reply, StatementClassifier};
use hacivat_session::testing::fake_session_classified;
use serde::Deserialize;

// ---------------------------------------------------------------------------------------------
// The catalogue

#[derive(Debug, Deserialize)]
struct File {
    row: Vec<Row>,
}

#[derive(Debug, Clone, Deserialize)]
struct Expected {
    verdict: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Row {
    id: String,
    sql: String,
    #[serde(default)]
    modes: Option<String>,
    #[serde(default)]
    search_path: Option<Vec<String>>,
    #[serde(default)]
    allowlist: Option<Vec<String>>,
    classifier: Expected,
    accounts: Vec<String>,
}

fn rows_of(file: &str) -> Vec<Row> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sql/sql-testkit/bypass")
        .join(format!("{file}.toml"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    toml::from_str::<File>(&text)
        .unwrap_or_else(|e| panic!("{path:?}: {e}"))
        .row
}

fn file_of(engine: DbmsKind) -> &'static str {
    match engine {
        DbmsKind::Postgres => "pg",
        DbmsKind::MariaDb => "mariadb",
        DbmsKind::Sqlite => "sqlite",
        _ => "mysql",
    }
}

/// The rows A′ runs on a server session: those whose context is the session's own (no `modes`
/// or `search_path` of their own; an `allowlist` row needs a profile of its own and is run A's).
fn a_prime_rows(engine: DbmsKind) -> Vec<Row> {
    rows_of(file_of(engine))
        .into_iter()
        .filter(|r| r.modes.is_none() && r.search_path.is_none() && r.allowlist.is_none())
        .collect()
}

/// The statement as it runs: another backend's ID that is never a session's, MySQL's
/// variable name, a scratch directory.
fn sql_of(row: &Row) -> String {
    row.sql
        .replace("{victim}", "999999")
        .replace("{tro}", "transaction_read_only")
        .replace("{d}", "/tmp/hv_t18")
}

// ---------------------------------------------------------------------------------------------
// The side channel

/// What the admin connection sees: server state a bypass would change (advisory and user locks,
/// replication slots, prepared transactions, the global settings run B's rows try), and on
/// PostgreSQL the last statement the console's own backend ran (`pg_stat_activity.query`), which
/// a denied row never becomes.
async fn snapshot(a: &mut Side, engine: DbmsKind, app: &str) -> (String, String) {
    match engine {
        DbmsKind::Postgres => {
            let state = a
                .scalar(
                    "SELECT (SELECT count(*) FROM pg_catalog.pg_locks WHERE locktype = 'advisory') \
                     || '/' || (SELECT count(*) FROM pg_catalog.pg_replication_slots) \
                     || '/' || (SELECT count(*) FROM pg_catalog.pg_prepared_xacts)",
                )
                .await;
            let last = a
                .scalar(&format!(
                    "SELECT coalesce(string_agg(query, ' | '), '') FROM pg_catalog.pg_stat_activity \
                     WHERE application_name = '{}'",
                    app.replace('\'', "''")
                ))
                .await;
            (state, last)
        }
        _ => {
            let state = a
                .scalar(
                    "SELECT CONCAT_WS('/', @@global.max_connections, @@global.sql_mode, \
                     IS_USED_LOCK('hv_t23_m1'), IS_USED_LOCK('hv_t23_m5'), IS_USED_LOCK('hv_m5'), \
                     IS_USED_LOCK('x'), IS_USED_LOCK('a'), \
                     (SELECT COUNT(*) FROM information_schema.tables \
                      WHERE table_schema = DATABASE()))",
                )
                .await;
            (state, String::new())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Run A′

/// What one row came to in A′.
fn outcome(reply: &Reply) -> String {
    match reply {
        Ok(_) => "ran".into(),
        Err(e) => match &e.kind {
            ExecErrorKind::SafetyDenied(_) => "safety-denied".into(),
            ExecErrorKind::Db => {
                common::matrix::code_of(e.server.as_ref()).unwrap_or_else(|| "db".into())
            }
            other => format!("{other:?}"),
        },
    }
}

/// The P20 fixture: a view whose read runs a writing function (`hv_t23_v` over `hv_t18_w()`),
/// in the test database; dropped first and last.
async fn pg_fixture(a: &mut Side, make: bool) {
    a.batch("DROP VIEW IF EXISTS hv_t23_v").await;
    a.batch("DROP FUNCTION IF EXISTS hv_t18_w()").await;
    a.batch("DROP TABLE IF EXISTS hv_t18_w_t").await;
    if make {
        a.batch("CREATE TABLE hv_t18_w_t (a int)").await;
        a.batch(
            "CREATE FUNCTION hv_t18_w() RETURNS int LANGUAGE sql VOLATILE AS \
             'INSERT INTO hv_t18_w_t VALUES (1) RETURNING a'",
        )
        .await;
        a.batch("CREATE VIEW hv_t23_v AS SELECT hv_t18_w() AS a")
            .await;
    }
}

/// A′ on `line` for `user` (the test account when `None`): every row, its outcome recorded;
/// a failure when a denied row was not `SafetyDenied` or changed the side channel, or an
/// allowed row was `SafetyDenied`.
async fn a_prime(
    line: &Line,
    user: Option<&str>,
    pool: &Arc<ClassifyPool>,
    a: &mut Side,
    record: &mut String,
    failures: &mut Vec<String>,
) {
    let who = user.unwrap_or("owner");
    let c = RoConsole::open(
        line,
        user,
        Some(pool.clone() as Arc<dyn StatementClassifier>),
        &[],
    )
    .await;
    // The session's real context reached the classifier: an unqualified builtin passes on
    // PostgreSQL (the search path read at connect names no schema before `pg_catalog`), and a
    // backslash escape reads as the server reads it on MySQL and MariaDB (the `sql_mode` read at
    // connect has no NO_BACKSLASH_ESCAPES): with either unknown, step 0b would deny them.
    let context_probe = if line.engine == DbmsKind::Postgres {
        "SELECT lower('A')"
    } else {
        "SELECT 'a\\' /*!; DO SLEEP(5); */ x'"
    };
    let reply = c.exec(context_probe).await;
    if reply.is_err() {
        failures.push(format!(
            "{} {who}: the context probe `{context_probe}` → {}",
            line.id,
            outcome(&reply)
        ));
    }
    for row in a_prime_rows(line.engine) {
        // The test account runs only the rows run B runs as it (and every catalogue row is
        // `hv_ro`'s).
        if user.is_none() && !row.accounts.iter().any(|a| a == "owner") {
            continue;
        }
        let sql = sql_of(&row);
        let before = snapshot(a, line.engine, c.app()).await;
        let reply = c.exec(&sql).await;
        let after = snapshot(a, line.engine, c.app()).await;
        let got = outcome(&reply);
        let shown = if sql.contains("IDENTIFIED BY") || sql.contains("PASSWORD") {
            format!("{} …", sql.split('\'').next().unwrap_or_default())
        } else {
            sql.clone()
        };
        let _ = writeln!(
            record,
            "{}\t{who}\t{}\t{}\t{got}\t{shown}",
            line.id, row.id, row.classifier.verdict
        );
        if row.classifier.verdict == "deny" {
            if !safety_denied(&reply) {
                failures.push(format!(
                    "{} {who} {} `{shown}`: {got}, not denied by step 0b",
                    line.id, row.id
                ));
            }
            if before != after {
                failures.push(format!(
                    "{} {who} {} `{shown}`: the side channel changed: {before:?} → {after:?}",
                    line.id, row.id
                ));
            }
        } else {
            // An allowed row meets the server's layers: never a classifier denial, and nothing
            // left behind (the view's write is refused by the read-only transaction).
            if safety_denied(&reply) {
                failures.push(format!(
                    "{} {who} {} `{shown}`: a false denial",
                    line.id, row.id
                ));
            }
            if before.0 != after.0 {
                failures.push(format!(
                    "{} {who} {} `{shown}`: {got}, and the server state changed: {} → {}",
                    line.id, row.id, before.0, after.0
                ));
            }
        }
    }
    c.close();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn run_a_prime_classifier_denies_and_side_channel_unchanged() {
    if !docker_enabled() {
        return;
    }
    let _one = SERIAL.lock().await;
    let pool = ClassifyPool::new(2);
    let mut failures = Vec::new();
    let mut record = String::new();
    for line in server_lines() {
        let mut a = Side::open(&line, None).await;
        if line.engine == DbmsKind::Postgres {
            pg_fixture(&mut a, true).await;
        }
        for user in [Some("hv_ro"), None] {
            a_prime(&line, user, &pool, &mut a, &mut record, &mut failures).await;
        }
        if line.engine == DbmsKind::Postgres {
            // P20's write never persisted (the read-only transaction refused it).
            let wrote = a.count("SELECT count(*) FROM hv_t18_w_t").await;
            if wrote != 0 {
                failures.push(format!("{}: P20's view wrote {wrote} rows", line.id));
            }
            pg_fixture(&mut a, false).await;
        }
        a.close().await;
    }
    let path = hacivat_testkit::artifact("t18-bypass-a-prime.tsv");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("the artifact directory");
    }
    std::fs::write(&path, &record).expect("the record is written");
    eprintln!("A′ results: {}", path.display());
    assert!(!record.is_empty(), "no line ran a row");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The tests of this binary that use the same servers take turns.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_sql_mode_is_pinned_after_reset() {
    // R-M3-57: a read-only MySQL/MariaDB session reads its `sql_mode` in the connect's own
    // round trip and pins it after every `COM_RESET_CONNECTION`: the real servers accept the pin
    // (their default modes included), and after resets the session's mode is still the one read
    // at connect. (The test account cannot change the global mode, so a server whose reset
    // would restore another mode is the FakeDriver test's, `readonly_classifier.rs`.)
    if !docker_enabled() {
        return;
    }
    let _one = SERIAL.lock().await;
    let pool = ClassifyPool::new(2);
    let mut failures = Vec::new();
    for line in server_lines() {
        if !line.is_mysql_family() {
            continue;
        }
        let mut a = Side::open(&line, None).await;
        let global = a.scalar("SELECT @@GLOBAL.sql_mode").await;
        a.close().await;
        for classifier in [Some(pool.clone() as Arc<dyn StatementClassifier>), None] {
            let c = RoConsole::open(&line, Some("hv_ro"), classifier.clone(), &[]).await;
            for i in 0..3 {
                let reply = c.exec(&format!("SELECT {i}")).await;
                if let Err(e) = reply {
                    failures.push(format!("{} statement {i}: {:?}", line.id, e.kind));
                }
            }
            // The context probe is Hacivat's own read, outside the guard: after three guarded
            // statements (each ending with the reset and the replay, the pin included).
            let mode = c
                .internal(hacivat_session::session::context_probe_sql(line.engine).unwrap())
                .await
                .unwrap_or_default();
            if mode != global {
                failures.push(format!(
                    "{} (classifier: {}): sql_mode after resets {mode:?}, connect {global:?}",
                    line.id,
                    classifier.is_some()
                ));
            }
            let state = c.s.state().borrow().state.clone();
            assert!(
                !matches!(state, hacivat_session::session::SessionState::Broken(_)),
                "{}: the pinned replay broke the session: {state:?}",
                line.id
            );
            c.close();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// The real pool over a fake server (every PR)

fn settings(lex_modes: Option<&str>, search_path: Option<&str>) -> SessionSettings {
    SessionSettings {
        lex_modes: lex_modes.map(Arc::from),
        search_path: search_path.map(Arc::from),
    }
}

async fn fake_ro(
    kind: DbmsKind,
    s: SessionSettings,
) -> (hacivat_session::session::SessionHandle, Arc<FakeDriver>) {
    let mut p = common::profile_of(kind);
    p.read_only = true;
    let fake = FakeDriver::new(kind)
        .tracks_transactions()
        .session_settings(s);
    fake_session_classified(
        tokio::runtime::Handle::current(),
        fake,
        p,
        ClassifyPool::new(1) as Arc<dyn StatementClassifier>,
    )
    .await
}

async fn exec(h: &hacivat_session::session::SessionHandle, sql: &str) -> Reply {
    h.submit(hacivat_session::session::Request::Exec {
        sql: sql.to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: hacivat_session::session::ExecOrigin::User,
    })
    .reply
    .await
    .expect("the actor replies")
}

fn sent(f: &FakeDriver, mark: usize, sql: &str) -> bool {
    f.call_log()[mark..]
        .iter()
        .any(|c| matches!(c, FakeCall::Statement(s) if s == sql))
}

#[tokio::test]
async fn call_is_denied_by_the_real_classifier_on_read_only() {
    // ER15: `CALL` never runs on a read-only session, whatever it calls; the M2b path has no
    // first-keyword layer, so step 0b with the real pool refuses it, and nothing is sent.
    for (kind, s, sql) in [
        (DbmsKind::MySql, settings(Some(""), None), "CALL hv_proc()"),
        (
            DbmsKind::MySql,
            settings(Some(""), None),
            "/*!50000 CALL hv_proc() */",
        ),
        (
            DbmsKind::Postgres,
            settings(Some("on"), Some("public")),
            "CALL hv_proc()",
        ),
        (
            DbmsKind::Postgres,
            settings(Some("on"), Some("public")),
            "call lower('a')",
        ),
    ] {
        let (h, f) = fake_ro(kind, s).await;
        let mark = f.call_log().len();
        let reply = exec(&h, sql).await;
        assert!(safety_denied(&reply), "{kind:?} {sql}: {reply:?}");
        assert!(f.call_log()[mark..].is_empty(), "{kind:?} {sql}");
    }
}

#[tokio::test]
async fn the_sessions_context_decides_what_the_real_classifier_reads() {
    // PostgreSQL: with the search path read at connect (`pg_catalog` not after another schema)
    // an unqualified builtin passes; unread, it is unknown and denied; a path that puts a schema
    // before `pg_catalog` denies it too (T3 M4).
    let (h, f) = fake_ro(
        DbmsKind::Postgres,
        settings(Some("on"), Some("\"$user\", public")),
    )
    .await;
    let mark = f.call_log().len();
    let reply = exec(&h, "SELECT lower('A')").await;
    assert!(!safety_denied(&reply), "{reply:?}");
    assert!(sent(&f, mark, "SELECT lower('A')"));
    for path in [None, Some("app, pg_catalog")] {
        let (h, f) = fake_ro(DbmsKind::Postgres, settings(Some("on"), path)).await;
        let mark = f.call_log().len();
        assert!(
            safety_denied(&exec(&h, "SELECT lower('A')").await),
            "{path:?}"
        );
        assert!(f.call_log()[mark..].is_empty());
    }
    // MySQL: the server's modes read once, exactly so. Default modes: the text is one string (a
    // read); NO_BACKSLASH_ESCAPES: a string, an executable comment and more (denied); unknown:
    // both readings, denied.
    let text = "SELECT 'a\\' /*!; DO SLEEP(5); */ x'";
    let (h, _f) = fake_ro(DbmsKind::MySql, settings(Some("STRICT_TRANS_TABLES"), None)).await;
    assert!(!safety_denied(&exec(&h, text).await));
    for modes in [Some("NO_BACKSLASH_ESCAPES"), None] {
        let (h, _f) = fake_ro(DbmsKind::MySql, settings(modes, None)).await;
        assert!(safety_denied(&exec(&h, text).await), "{modes:?}");
    }
    // MariaDB `sql_mode=ORACLE` is known but not modelled: every statement is refused (T18
    // carry), never read as the default grammar.
    let (h, _f) = fake_ro(
        DbmsKind::MariaDb,
        settings(
            Some("PIPES_AS_CONCAT,ANSI_QUOTES,IGNORE_SPACE,ORACLE"),
            None,
        ),
    )
    .await;
    assert!(safety_denied(&exec(&h, "SELECT 1").await));
}

#[test]
fn mysql_search_path_is_the_current_database() {
    // The T20 carry: on MySQL and MariaDB `ClassifyCtx.search_path[0]` is the current database
    // (read at connect, refreshed after a context change); unread, the path stays empty.
    use hacivat_exec::classify_pool::guard_ctx_in;
    use hacivat_session::session::ClassifierContext;
    for kind in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let known = ClassifierContext {
            lex_modes: Some(Arc::from("")),
            search_path: Some(Arc::from(vec![Arc::<str>::from("performance_schema")])),
            server_version: None,
        };
        let ctx = guard_ctx_in(kind, Arc::from(Vec::new()), &known);
        assert_eq!(
            ctx.search_path.first().map(|s| &**s),
            Some("performance_schema")
        );
        let unread = ClassifierContext {
            search_path: None,
            ..known
        };
        assert!(
            guard_ctx_in(kind, Arc::from(Vec::new()), &unread)
                .search_path
                .is_empty()
        );
    }
}

#[tokio::test]
async fn copy_to_stdout_is_denied_on_read_only() {
    // `alg/13` S-12: an export is not a query tool's, so a read-only console refuses it (T9
    // carry; `classify` makes it `Unknown`).
    let (h, f) = fake_ro(DbmsKind::Postgres, settings(Some("on"), Some("public"))).await;
    let mark = f.call_log().len();
    assert!(safety_denied(&exec(&h, "COPY t TO STDOUT").await));
    assert!(f.call_log()[mark..].is_empty());
}

#[test]
fn every_a_prime_row_is_a_run_a_row() {
    // A′ runs the rows of run A whose context is the session's own: none is lost on the way.
    let mut counts = BTreeMap::new();
    for engine in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
        let all = rows_of(file_of(engine)).len();
        let a = a_prime_rows(engine).len();
        assert!(a > 0 && a <= all, "{engine:?}");
        counts.insert(file_of(engine), (a, all));
    }
    assert!(counts["pg"].0 >= 100, "{counts:?}");
}
