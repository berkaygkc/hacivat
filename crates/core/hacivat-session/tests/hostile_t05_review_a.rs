// ## m4-t05-meta (fix round 1)
//! Security review A of M4 Task 5: the reviewer's hostile tests (`review_t05_a.rs`), flipped
//! so each asserts the fix (rulings R-W1-9, I2-A, I4-A; ADR 0004 `## m4-t05`).
//!
//! `MetaQuery::new` is called here directly: test files are exempt from check-deps
//! `MetaQueryOnlyFromPacks`, and the hostile texts must reach the guards.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::runs::{Recorder, run_spec, spec_of as run_spec_of};
use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_net::TlsMode;
use hacivat_proto::Department;
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConnSpec, MetaError, MetaQuery, check_read_text};
use hacivat_session::pipeline::{ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ClientId, ExecErrorKind, JobKind, SessionHandle, SessionKind, SessionManager, SessionSpec,
    TxCommand, TxMode,
};
use hacivat_values::{LogicalType, Value};
use tokio_util::sync::CancellationToken;

fn manager_over(deps: ConnectDeps, profile: Profile) -> Arc<SessionManager> {
    let budget = Arc::new(budget_for(&profile));
    SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile),
        Arc::new(deps),
        budget,
    )
}

fn single_pg() -> Profile {
    Profile {
        single_session: true,
        max_connections: 3,
        ..pg_profile(TlsMode::Disable)
    }
}

fn spec(profile: &Profile, database: Option<&str>) -> MetaConnSpec {
    MetaConnSpec {
        profile: profile.id,
        database: database.map(str::to_string),
        statement_timeout: Duration::from_secs(60),
    }
}

fn answers(prefix: &'static str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: vec![(
                "relname",
                LogicalType::Text {
                    max_chars: None,
                    fixed: false,
                },
            )],
            rows: FakeRows::Literal(vec![vec![Value::Text(Arc::from("orders"))]]),
        }],
        ..FakeQuery::default()
    }
}

async fn open(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let department = match kind {
        SessionKind::Job(..) => Department::Executor,
        _ => Department::Console,
    };
    sm.open(
        SessionSpec {
            kind,
            read_only: false,
            department,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("opens: {:?}", e.class))
}

fn log_since(f: &FakeDriver, from: usize) -> Vec<String> {
    f.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) => Some(format!("stmt:{s}")),
            FakeCall::Typed(FakeTypedCall::Query(s)) => Some(format!("query:{s}")),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// 1. The allowlist corpus.

#[test]
fn h1_allowlist_corpus() {
    // The plain read stays accepted.
    assert_eq!(check_read_text("TABLE pg_catalog.pg_class"), Ok(()));
    // Accepted before the fix; every one is refused now (fix round 1: the token-level guard).
    let now_refused = [
        // PG `--` comments end at \r as well as \n (scan.l: newline [\n\r]); the checker only
        // ends them at \n. PostgreSQL reads `CREATE TABLE evil AS SELECT 1`.
        "-- x\rCREATE TABLE evil AS\nSELECT 1",
        "-- x\rCOPY (\nSELECT 1) TO '/tmp/hv_evil'",
        "-- x\rPREPARE p AS\nSELECT 1",
        "--\rDECLARE c CURSOR WITH HOLD FOR\nSELECT 1",
        // Side-effecting functions behind SELECT/VALUES/WITH.
        "SELECT pg_advisory_lock(1)",
        "SELECT set_config('search_path', 'evil', false)",
        "SELECT nextval('s')",
        "SELECT setval('s', 1)",
        "VALUES (nextval('s'))",
        "WITH x AS (SELECT setval('s', 1)) SELECT * FROM x",
        "SELECT lo_create(0)",
        "SELECT dblink_exec('dbname=x', 'DROP TABLE t')",
        "SELECT txid_current()",
        "SELECT pg_terminate_backend(1)",
        "SELECT * FROM t FOR SHARE",
        "SELECT * FROM t FOR KEY SHARE",
        "SELECT GET_LOCK('x', 0)",
        "SELECT SLEEP(100)",
        "SELECT @x := 1",
        "SELECT * FROM t LOCK IN SHARE MODE",
        // Unicode whitespace before the keyword: not a keyword token.
        "\u{00a0}SELECT 1",
    ];
    for t in now_refused {
        assert_eq!(
            check_read_text(t),
            Err(MetaError::RefusedText),
            "refused now: {t:?}"
        );
    }
    let refused = [
        "COPY t TO STDOUT",
        "WITH x AS (INSERT INTO t VALUES (1) RETURNING 1) SELECT * FROM x",
        "SELECT * FROM t FOR UPDATE",
        "SELECT * FROM t FOR NO KEY UPDATE",
        "SELECT 1 INTO OUTFILE '/tmp/x'",
        "SELECT 1 INTO @x",
        "SELECT $$;$$",
        "SELECT $q$ ; COMMIT; $q$",
        "SELECT 'a;b'",
        "/*!SELECT*/ 1",
        "SELECT /*!80000 SLEEP(1) */ 1",
        "\u{FF33}ELECT 1", // fullwidth S
        "ЅELECT 1",        // Cyrillic DZE
        "SEL\u{200b}ECT 1",
        "/* a /* b */ c */ SELECT 1",
        "# x\nSELECT 1",
        "EXPLAIN ANALYZE SELECT 1",
        "LOCK TABLE t",
        "VACUUM INTO '/tmp/x'",
        "ATTACH 'x' AS y",
        "PRAGMA writable_schema = ON",
        "HANDLER t OPEN",
        "DO $$BEGIN END$$",
        "XA RECOVER",
        "END",
        "ABORT",
        "PREPARE TRANSACTION 'x'",
    ];
    for t in refused {
        assert_eq!(
            check_read_text(t),
            Err(MetaError::RefusedText),
            "refused: {t:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// 2. The CR-comment bypass (I2) never reaches the wire on a joined connection.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_cr_comment_ctas_never_reaches_a_joined_conn() {
    let profile = Profile {
        max_connections: 4,
        ..pg_profile(TlsMode::Disable)
    };
    let (deps, fake) = deps_with(fake_pg().tracks_transactions());
    let sm = manager_over(deps, profile);
    let job = open(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    run(&job, "BEGIN").await.unwrap();
    let mut c = hacivat_session::meta::MetaConn::join(&job).unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    let evil = "-- x\rCREATE TABLE evil AS\nSELECT 1";
    let got = snap.rows(MetaQuery::new(evil, &[], true)).await;
    let sent = log_since(&fake, from);
    assert_eq!(got.map(drop), Err(MetaError::RefusedText));
    assert!(
        sent.is_empty(),
        "nothing reached the job's connection: {sent:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// 3. Smart mode (C1): a metadata read on the user's session opens no user transaction.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_smart_mode_metadata_read_opens_no_user_transaction() {
    let profile = single_pg();
    let (deps, fake) = deps_with(
        fake_pg()
            .tracks_transactions()
            .queries(vec![answers("SELECT relname")]),
    );
    let sm = manager_over(deps, profile.clone());
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    tx_cmd(&user, TxCommand::SetMode(TxMode::Smart))
        .await
        .unwrap();
    assert_eq!(snap(&user).tx.status, TxStatus::Idle);
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();
    let from = fake.call_log().len();
    let mut s = c.begin_snapshot().await.unwrap();
    let got = s
        .rows(MetaQuery::new(
            "SELECT relname FROM pg_catalog.pg_class",
            &[],
            false,
        ))
        .await;
    let sent = log_since(&fake, from);
    assert_eq!(
        snap(&user).tx.status,
        TxStatus::Idle,
        "the metadata read left the user's session outside any transaction"
    );
    assert!(
        !sent.iter().any(|s| s == "stmt:BEGIN"),
        "no user BEGIN: {sent:?}"
    );
    assert_eq!(
        sent,
        [
            "stmt:BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "stmt:SET LOCAL search_path = pg_catalog, pg_temp",
            "stmt:SET LOCAL statement_timeout = 60000",
            "query:SELECT relname FROM pg_catalog.pg_class",
            "stmt:ROLLBACK",
        ],
        "Hacivat's own read-only transaction, rolled back"
    );
    assert!(got.is_ok(), "{got:?}");
}

// ---------------------------------------------------------------------------------------------
// 4. I1: a request queued before the read runs first; the actor then refuses the read
// before sending it (current-thread runtime: deterministic).

#[tokio::test]
async fn h4_a_read_queued_behind_a_user_begin_is_refused() {
    let profile = single_pg();
    let (deps, fake) = deps_with(
        fake_pg()
            .tracks_transactions()
            .queries(vec![answers("SELECT relname")]),
    );
    let sm = manager_over(deps, profile.clone());
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    // The user's BEGIN is queued, not yet taken.
    let begin = user.submit(exec("BEGIN"));
    let got = s
        .rows(MetaQuery::new(
            "SELECT relname FROM pg_catalog.pg_class",
            &[],
            false,
        ))
        .await;
    begin.reply.await.unwrap().unwrap();
    let sent = log_since(&fake, from);
    assert!(
        sent.iter().any(|s| s == "stmt:BEGIN"),
        "the user's BEGIN ran"
    );
    assert!(
        !sent.iter().any(|s| s.starts_with("query:SELECT relname")),
        "the read never ran inside the user's transaction: {sent:?}"
    );
    assert_eq!(
        got.map(drop),
        Err(MetaError::Busy),
        "the actor refused it when it took it"
    );
}

// ---------------------------------------------------------------------------------------------
// 5. I4: a metadata read never moves the user's context generation: a run the console
// classified just before is not refused (`ContextChanged`).

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_metadata_read_keeps_the_users_classified_run() {
    let profile = single_pg();
    let (deps, _fake) = deps_with(
        fake_pg()
            .tracks_transactions()
            .queries(vec![answers("SELECT relname")]),
    );
    let sm = manager_over(deps, profile.clone());
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let (_, generation) = user.run_context().await;
    let generation = generation.expect("a generation");
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    s.rows(MetaQuery::new(
        "SELECT relname FROM pg_catalog.pg_class",
        &[],
        false,
    ))
    .await
    .unwrap();
    s.commit().await.unwrap();
    let o = Recorder::new();
    let mut run = run_spec_of(&o, &["SELECT 1"]);
    run.context_generation = Some(generation);
    let reply = run_spec(&user, run).await;
    assert!(
        !matches!(&reply, Err(e) if matches!(e.kind, ExecErrorKind::ContextChanged)),
        "a metadata read never refuses the user's classified run: {:?}",
        reply.map(drop).map_err(|e| e.kind)
    );
}

// ---------------------------------------------------------------------------------------------
// 6. The SQLite reader: what `mode=ro` (SQLITE_OPEN_READ_ONLY) does not stop.

async fn sqlite_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use hacivat_driver::{Driver, DriverConnect, Endpoint, InsecureAuth};
    let path = dir.join(name);
    let mut conn = hacivat_driver_sqlite::SqliteDriver::new()
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.clone(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "fixture".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: hacivat_driver::InsecureAuth::default(),
        })
        .await
        .expect("fixture");
    let _ = InsecureAuth::default();
    conn.batch("CREATE TABLE t(a)").await.unwrap();
    conn.batch("INSERT INTO t VALUES (1)").await.unwrap();
    conn.close().await;
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h6_sqlite_reader_write_surfaces() {
    let dir = tempfile::tempdir().unwrap();
    let path = sqlite_file(dir.path(), "main.db").await;
    let aux = sqlite_file(dir.path(), "arsiv.db").await;
    let profile = Profile {
        engine: hacivat_proto::DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.clone(),
            create_if_missing: false,
        },
        user: None,
        ..sqlite_profile()
    };
    let sm = manager_over(common::docker_support::deps(), profile.clone());
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();

    // Attach by URI asking for rw, and a file that does not exist: both must fail.
    let uri = format!("file:{}?mode=rw", aux.display());
    let r_uri = c.attach_read_only("rw", std::path::Path::new(&uri)).await;
    let missing = dir.path().join("created.db");
    let r_missing = c.attach_read_only("nf", &missing).await;
    assert!(r_uri.is_err(), "a URI asking for rw is refused: {r_uri:?}");
    assert!(r_missing.is_err(), "{r_missing:?}");
    assert!(
        !missing.exists(),
        "the read-only reader never creates a file by ATTACH"
    );

    // Attach the real aux read-only, then try to write it. Fix round 2 (review B R-M3): the
    // token guard refuses every write on the own connection too; `rows_unguarded` (debug
    // builds with `test-support`) proves the engine refuses it as well.
    c.attach_read_only("aux", &aux).await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    for write in [
        "INSERT INTO aux.t VALUES (9)",
        "PRAGMA writable_schema = ON",
        "UPDATE sqlite_master SET sql = sql",
        "CREATE TEMP TABLE tt(a)",
    ] {
        assert_eq!(
            s.rows(MetaQuery::new(write, &[], false)).await.map(drop),
            Err(MetaError::RefusedText),
            "{write}"
        );
    }
    // Past the guard (fix round 3: only with this crate's `test-support` feature, which
    // `cargo test --workspace` enables through the dev-dependencies), the engine refuses too.
    #[cfg(all(debug_assertions, feature = "test-support"))]
    {
        let w = s
            .rows_unguarded(MetaQuery::new("INSERT INTO aux.t VALUES (9)", &[], false))
            .await;
        assert!(w.is_err());
        // `writable_schema` may be set; `sqlite_master` still refuses the write.
        let _ = s
            .rows_unguarded(MetaQuery::new("PRAGMA writable_schema = ON", &[], false))
            .await;
        let wm = s
            .rows_unguarded(MetaQuery::new(
                "UPDATE sqlite_master SET sql = sql",
                &[],
                false,
            ))
            .await;
        assert!(wm.is_err());
        // M8 (documented in ADR 0004 `## m4-t05`): the reader's own private `temp` schema
        // takes a table when the guard is bypassed; it is never persisted and dies with the
        // connection.
        let tt = s
            .rows_unguarded(MetaQuery::new("CREATE TEMP TABLE tt(a)", &[], false))
            .await;
        assert!(tt.is_ok(), "{tt:?}");
    }
    s.commit().await.unwrap();
    // `VACUUM INTO` never reaches the reader: an own snapshot refuses it (review B M1), and
    // inside its `BEGIN` SQLite could not run it anyway.
    let out = dir.path().join("copied.db");
    let v = {
        let sql = format!("VACUUM INTO '{}'", out.display());
        let mut s = c.begin_snapshot().await.unwrap();
        let in_tx = s.rows(MetaQuery::new(&sql, &[], false)).await;
        drop(s);
        in_tx
    };
    assert_eq!(v.map(drop), Err(MetaError::RefusedText));
    assert!(!out.exists());
    // Original file untouched.
    let mut s = c.begin_snapshot().await.unwrap();
    let n = s
        .rows(MetaQuery::new("SELECT count(*) FROM t", &[], false))
        .await
        .unwrap();
    assert_eq!(n.rows[0][0].as_deref(), Some("1"));
    s.commit().await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Re-review of fix round 1 (`review_t05_a_r1.rs`, R1–R5), asserting fix round 2.

/// R1: the round-0 bypasses stay refused under every lexer.
#[test]
fn r1_round0_bypasses_are_refused() {
    for t in [
        "-- x\rCREATE TABLE evil AS\nSELECT 1",
        "-- x\rCOPY (\nSELECT 1) TO '/tmp/hv_evil'",
        "-- x\rPREPARE p AS\nSELECT 1",
        "--\rDECLARE c CURSOR WITH HOLD FOR\nSELECT 1",
        "SELECT pg_advisory_lock(1)",
        "SELECT set_config('search_path', 'evil', false)",
        "SELECT nextval('s')",
        "SELECT setval('s', 1)",
        "VALUES (nextval('s'))",
        "WITH x AS (SELECT setval('s', 1)) SELECT * FROM x",
        "SELECT lo_create(0)",
        "SELECT dblink_exec('dbname=x', 'DROP TABLE t')",
        "SELECT txid_current()",
        "SELECT pg_terminate_backend(1)",
        "SELECT * FROM t FOR SHARE",
        "SELECT * FROM t FOR KEY SHARE",
        "SELECT GET_LOCK('x', 0)",
        "SELECT SLEEP(100)",
        "SELECT @x := 1",
        "SELECT * FROM t LOCK IN SHARE MODE",
        "SELECT 1INTO x",
        "SELECT $$x$$INTO t",
        "SELECT \"nextval\"('s')",
        "SELECT `GET_LOCK`('x', 0)",
        "SELECT pg_catalog . \"set_config\"('a','b',false)",
        "SELECT GET_LOCK /* c */ ('x', 0)",
        "SELECT {fn sleep(1)}",
        "SELECT 'a\\' INTO x --'",
    ] {
        assert_eq!(check_read_text(t), Err(MetaError::RefusedText), "{t:?}");
    }
}

/// R2: I5 (`U&"…"` names), the administration families, `LAST_INSERT_ID(n)`, lookalike and
/// invisible characters: refused (fix round 2). The harmless readings stay accepted.
#[test]
fn r2_token_guard_probes() {
    use hacivat_proto::DbmsKind;
    use hacivat_session::meta::check_read_text_for;
    let refused: &[(&str, DbmsKind)] = &[
        ("SELECT U&\"nextval\"('s')", DbmsKind::Postgres),
        ("SELECT U&\"pg_advisory_lock\"(1)", DbmsKind::Postgres),
        (
            "SELECT u&\"set_config\"('search_path','evil',false)",
            DbmsKind::Postgres,
        ),
        ("SELECT U&\"\\006Eextval\"('s')", DbmsKind::Postgres),
        (
            "SELECT U&\"!006Eextval\" UESCAPE '!' ('s')",
            DbmsKind::Postgres,
        ),
        ("SELECT pg_catalog.U&\"txid_current\"()", DbmsKind::Postgres),
        ("SELECT pg_reload_conf()", DbmsKind::Postgres),
        (
            "SELECT pg_create_logical_replication_slot('s', 'test_decoding')",
            DbmsKind::Postgres,
        ),
        (
            "SELECT pg_logical_emit_message(true, 'p', 'x')",
            DbmsKind::Postgres,
        ),
        ("SELECT pg_stat_reset()", DbmsKind::Postgres),
        ("SELECT LAST_INSERT_ID(5)", DbmsKind::MySql),
        ("SELECT 1 /* /* */ INTO x */", DbmsKind::MySql),
        ("\u{00a0}SELECT 1", DbmsKind::Postgres),
        ("SELECT 1 \u{2028}", DbmsKind::Postgres),
        ("SELECT n\u{0435}xtval('s')", DbmsKind::Postgres),
        ("SELECT 1 INTO\u{00a0}@x", DbmsKind::MySql),
        ("SELECT $a$ ; $a$", DbmsKind::Postgres),
        ("SELECT * FROM t FOR\tUPDATE", DbmsKind::Postgres),
        ("SELECT 1 FOR/**/SHARE", DbmsKind::MySql),
    ];
    for (t, k) in refused {
        assert_eq!(
            check_read_text_for(t, *k),
            Err(MetaError::RefusedText),
            "{k:?} {t:?}"
        );
    }
    // Harmless: a read-only lock probe, PostgreSQL's nested comment hiding `INTO` (the server
    // reads one comment), MySQL's `: =` (a syntax error there), an escaped quote, a trailing
    // comment, a backtick name.
    let accepted: &[(&str, DbmsKind)] = &[
        ("SELECT IS_USED_LOCK('x')", DbmsKind::MySql),
        ("SELECT 1 /* /* */ INTO x */", DbmsKind::Postgres),
        ("SELECT @x : = 1", DbmsKind::MySql),
        ("SELECT E'\\'' || 'x'", DbmsKind::Postgres),
        ("SELECT 1 -- ;", DbmsKind::Postgres),
        ("SHOW CREATE TABLE `t`", DbmsKind::MySql),
    ];
    for (t, k) in accepted {
        assert_eq!(check_read_text_for(t, *k), Ok(()), "{k:?} {t:?}");
    }
}

/// R3: the `U&` name never reaches a joined connection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r3_unicode_ident_never_reaches_a_joined_conn() {
    let profile = Profile {
        max_connections: 4,
        ..pg_profile(TlsMode::Disable)
    };
    let (deps, fake) = deps_with(fake_pg().tracks_transactions());
    let sm = manager_over(deps, profile);
    let job = open(&sm, SessionKind::Job(1, JobKind::DdlApply)).await;
    run(&job, "BEGIN").await.unwrap();
    let mut c = hacivat_session::meta::MetaConn::join(&job).unwrap();
    let mut snap = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    let evil = "SELECT U&\"nextval\"('s')";
    let got = snap.rows(MetaQuery::new(evil, &[], true)).await;
    assert_eq!(got.map(drop), Err(MetaError::RefusedText));
    assert!(log_since(&fake, from).is_empty());
}

/// R4: a user `Tx(Begin)` command queued before the read: Busy, the read sends nothing.
#[tokio::test]
async fn r4_queued_tx_begin_then_read() {
    let profile = single_pg();
    let (deps, fake) = deps_with(
        fake_pg()
            .tracks_transactions()
            .queries(vec![answers("SELECT relname")]),
    );
    let sm = manager_over(deps, profile.clone());
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let cancel = CancellationToken::new();
    let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();
    let mut s = c.begin_snapshot().await.unwrap();
    let from = fake.call_log().len();
    let begin = user.submit(hacivat_session::session::Request::Tx(TxCommand::Begin));
    let got = s
        .rows(MetaQuery::new(
            "SELECT relname FROM pg_catalog.pg_class",
            &[],
            false,
        ))
        .await;
    begin.reply.await.unwrap().unwrap();
    let sent = log_since(&fake, from);
    assert_eq!(got.map(drop), Err(MetaError::Busy));
    assert!(!sent.iter().any(|s| s.contains("relname")), "{sent:?}");
    assert_eq!(snap(&user).tx.status, TxStatus::InTx);
}

/// R5: Smart and Manual sessions: a read sends only Hacivat's read-only transaction, and the
/// context generation stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r5_smart_and_manual_send_only_hacivats_read_only_txn() {
    for mode in [TxMode::Smart, TxMode::Manual] {
        let profile = single_pg();
        let (deps, fake) = deps_with(
            fake_pg()
                .tracks_transactions()
                .queries(vec![answers("SELECT relname")]),
        );
        let sm = manager_over(deps, profile.clone());
        let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
        tx_cmd(&user, TxCommand::SetMode(mode)).await.unwrap();
        let (_, g0) = user.run_context().await;
        let cancel = CancellationToken::new();
        let mut c = sm.meta_conn(spec(&profile, None), &cancel).await.unwrap();
        let from = fake.call_log().len();
        let mut s = c.begin_snapshot().await.unwrap();
        let got = s
            .rows(MetaQuery::new(
                "SELECT relname FROM pg_catalog.pg_class",
                &[],
                false,
            ))
            .await;
        s.commit().await.unwrap();
        let sent = log_since(&fake, from);
        let (_, g1) = user.run_context().await;
        assert!(got.is_ok(), "{mode:?}: {got:?}");
        assert_eq!(snap(&user).tx.status, TxStatus::Idle);
        assert_eq!(
            sent,
            [
                "stmt:BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
                "stmt:SET LOCAL search_path = pg_catalog, pg_temp",
                "stmt:SET LOCAL statement_timeout = 60000",
                "query:SELECT relname FROM pg_catalog.pg_class",
                "stmt:ROLLBACK",
            ]
        );
        assert_eq!(g0, g1, "no ContextChanged");
    }
}
