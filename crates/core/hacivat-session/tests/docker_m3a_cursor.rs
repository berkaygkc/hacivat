//! M3a Task 18 over Docker: console cursors on real servers. A suspended million-row
//! PostgreSQL result is released by the next statement; the idle bound rolls Hacivat's own
//! transaction back (and what a read head's function wrote with it, R-M3-23) while a cursor
//! that completes commits; a user's own `SAVEPOINT hv_cursor` survives a cursor; stop fetching
//! leaves a user's transaction usable, and a server cancel inside it leaves it failed (P16);
//! MySQL and MariaDB release by `KILL QUERY` on a side connection and run the next statement on
//! the same connection. Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p
//! hacivat-session --test docker_m3a_cursor`.
//!
//! - Targets come only from `require_target` (Q6). Fixtures have names unique to the run and
//!   are dropped at the end (Q7).
//! - Real servers set the pace: waits are on conditions bounded by real time; the idle bound
//!   is injected (2 s), never a real one.
//! - `sqlite_rollback_journal_cursor_has_the_30s_bound` needs no server: it runs in plain
//!   `cargo test`, over the real SQLite driver.

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::conformance::*;
use common::runs::*;
use hacivat_driver::TxStatus;
use hacivat_proto::DbmsKind;
use hacivat_results::{CloseReason, Completeness, FetchPolicy};
use hacivat_session::session::{
    CancelOutcome, ExecErrorKind, Request, SessionHandle, SessionSnapshot, TxOwner, UnitEnd,
};
use hacivat_testkit::{DockerTarget, require_target};
use hacivat_values::Value;

fn snap(s: &SessionHandle) -> SessionSnapshot {
    s.state().borrow().clone()
}

fn closed(reason: CloseReason) -> Completeness {
    Completeness::Closed { reason }
}

/// A suspendable run of `sql` with `fetch`, expected to end suspended at its read-ahead.
async fn suspend(s: &SessionHandle, sql: &str, fetch: FetchPolicy, what: &str) -> Arc<Recorder> {
    let o = Recorder::new();
    run_spec(s, spec_with(&o, &[(sql, true)], fetch))
        .await
        .unwrap_or_else(|e| panic!("{what}: the run failed: {:?}", e.kind));
    assert!(
        matches!(o.snap(0).completeness(), Completeness::Suspended(_)),
        "{what}: {:?}",
        o.snap(0).completeness()
    );
    assert_eq!(o.rows(), 10_000, "{what}: the read-ahead");
    o
}

fn int_at(o: &Recorder, row: u64) -> i64 {
    match o.snap(0).cell(row, 0).to_value() {
        Value::Int(v) => v,
        other => panic!("row {row}: {other:?}"),
    }
}

/// How many of this test's own backends sit idle in a transaction (`marker` is in their last
/// statement's text; the asking connection is `active`).
async fn idle_in_tx(t: &DockerTarget, marker: &str) -> i64 {
    let mut admin = admin(t).await;
    let n = count(
        &mut admin,
        &format!(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE state LIKE 'idle in transaction%' AND query LIKE '%{marker}%'"
        ),
    )
    .await;
    admin.close().await;
    n
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_suspended_million_rows_released_by_select_1() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let on = session_on(&t, Path::Direct).await;
    let marker = unique("hv_t18_big");
    let sql = format!("SELECT g FROM generate_series(1, 1000000) g /* {marker} */");
    let o = suspend(&on.s, &sql, FetchPolicy::default(), "pg17").await;
    // The rows are the first 10,000, in order; the server holds the portal in Hacivat's own
    // transaction, which the session never shows as the user's.
    assert_eq!((int_at(&o, 0), int_at(&o, 9_999)), (1, 10_000));
    let s = snap(&on.s);
    assert_eq!((s.tx.status, s.tx.owner), (TxStatus::InTx, TxOwner::Cursor));
    assert_eq!(s.cursor.map(|c| (c.rows, c.holds_tx)), Some((10_000, true)));
    assert_eq!(idle_in_tx(&t, &marker).await, 1);

    // Another statement: the cursor goes first, and the statement runs at once.
    let start = Instant::now();
    run(&on.s, "SELECT 1").await.expect("SELECT 1 runs");
    let took = start.elapsed();
    assert!(took < secs(1), "the release took {took:?}");
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::SessionNeeded)
    );
    assert_eq!(o.rows(), 10_000);
    let s = snap(&on.s);
    assert_eq!((s.tx.status, s.cursor), (TxStatus::Idle, None));
    assert_eq!(idle_in_tx(&t, &marker).await, 0);

    // A result a little past the read-ahead: fetching more and then all completes it, and
    // the connection is in autocommit again.
    let sql = format!("SELECT g FROM generate_series(1, 30000) g /* {marker} */");
    let o = suspend(&on.s, &sql, FetchPolicy::default(), "pg17 fetch").await;
    let result = o.store(0).id();
    ask(
        &on.s,
        Request::FetchMore {
            result,
            rows: 5_000,
        },
    )
    .await
    .expect("the fetch runs");
    assert_eq!(o.rows(), 15_000);
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    ask(&on.s, Request::FetchAll { result })
        .await
        .expect("the fetch runs");
    assert_eq!(o.rows(), 30_000);
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!((int_at(&o, 14_999), int_at(&o, 29_999)), (15_000, 30_000));
    assert!(o.releases().is_empty());
    assert_eq!(snap(&on.s).tx.status, TxStatus::Idle);
    assert_eq!(idle_in_tx(&t, &marker).await, 0);
    on.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_injected_idle_bound_rolls_back_hacivats_transaction() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    // A read head whose function writes: what a cursor that does not complete must undo.
    let table = unique("hv_t18_w");
    let func = unique("hv_t18_f");
    let mut admin = admin(&t).await;
    batch(
        &mut admin,
        &t.id,
        &format!(
            "CREATE TABLE {table}(a int); \
             CREATE FUNCTION {func}(int) RETURNS int LANGUAGE sql VOLATILE AS \
             $$ INSERT INTO {table} VALUES ($1) RETURNING a $$"
        ),
    )
    .await;
    let on = session_on(&t, Path::Direct).await;
    let fetch = FetchPolicy {
        idle_cap: Some(secs(2)),
        ..FetchPolicy::default()
    };
    let sql = format!("SELECT {func}(g::int) FROM generate_series(1, 100000) g");
    let start = Instant::now();
    let o = suspend(&on.s, &sql, fetch, "pg17 idle").await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(2));
    assert_eq!(idle_in_tx(&t, &func).await, 1);
    // Nobody touches the session: the bound releases the cursor and rolls back.
    let mut state = on.s.state();
    tokio::time::timeout(secs(15), state.wait_for(|s| s.cursor.is_none()))
        .await
        .expect("the idle bound released the cursor")
        .expect("the session is alive");
    let took = start.elapsed();
    assert!(took >= secs(2), "released after {took:?}");
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, true)]);
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::IdleTimeout));
    assert_eq!(o.rows(), 10_000);
    assert_eq!(snap(&on.s).tx.status, TxStatus::Idle);
    assert_eq!(idle_in_tx(&t, &func).await, 0);
    assert_eq!(
        count(&mut admin, &format!("SELECT count(*) FROM {table}")).await,
        0,
        "the function's writes were rolled back with Hacivat's transaction"
    );

    // The same read head run to its end commits, exactly like an autocommit statement.
    let sql = format!("SELECT {func}(g::int) FROM generate_series(1, 12000) g");
    let o = suspend(&on.s, &sql, FetchPolicy::default(), "pg17 complete").await;
    let result = o.store(0).id();
    ask(&on.s, Request::FetchAll { result })
        .await
        .expect("the fetch runs");
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(
        count(&mut admin, &format!("SELECT count(*) FROM {table}")).await,
        12_000
    );
    on.shutdown().await;
    batch(
        &mut admin,
        &t.id,
        &format!("DROP FUNCTION {func}(int); DROP TABLE {table}"),
    )
    .await;
    admin.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_a_users_savepoint_named_hv_cursor_survives_a_cursor() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let on = session_on(&t, Path::Direct).await;
    let s = &on.s;
    for sql in [
        "BEGIN",
        "CREATE TEMP TABLE hv_t18_sp(a int) ON COMMIT DROP",
        "INSERT INTO hv_t18_sp VALUES (1)",
        // The user's own savepoint, with the name Hacivat uses.
        "SAVEPOINT hv_cursor",
        "INSERT INTO hv_t18_sp VALUES (2)",
    ] {
        run(s, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    }
    // A cursor inside the transaction: Hacivat's savepoint shadows the user's while it is
    // open.
    let o = suspend(
        s,
        "SELECT g FROM generate_series(1, 1000000) g",
        FetchPolicy::default(),
        "pg17 savepoint",
    )
    .await;
    let tx = snap(s).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    // The next statement releases it: Hacivat's savepoint is rolled back and released.
    run(s, "INSERT INTO hv_t18_sp VALUES (3)")
        .await
        .expect("the insert runs");
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    // The user's savepoint is the one left: rolling back to it undoes rows 2 and 3 and keeps
    // row 1. Had Hacivat's survived, the rollback would have kept row 2.
    run(s, "ROLLBACK TO SAVEPOINT hv_cursor")
        .await
        .expect("the user's savepoint is still there");
    let rows = s
        .submit(simple_exec(
            "SELECT string_agg(a::text, ',' ORDER BY a) FROM hv_t18_sp",
        ))
        .reply
        .await
        .unwrap()
        .expect("the count runs")
        .rows;
    assert_eq!(rows, vec![vec![Some("1".to_string())]]);
    assert_eq!(snap(s).tx.status, TxStatus::InTx);
    run(s, "COMMIT").await.expect("the commit runs");
    assert_eq!(snap(s).tx.status, TxStatus::Idle);
    on.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_stop_fetching_keeps_a_users_transaction_and_a_server_cancel_fails_it() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let on = session_on(&t, Path::Direct).await;
    let s = &on.s;
    for sql in [
        "BEGIN",
        "CREATE TEMP TABLE hv_t18_stop(a int) ON COMMIT DROP",
        "INSERT INTO hv_t18_stop VALUES (1)",
    ] {
        run(s, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    }
    // Stop fetching a stream that flows: between chunks only the portal's close is sent, so
    // the user's transaction is as it was (R-M3-16).
    let o = suspend(
        s,
        "SELECT g FROM generate_series(1, 100000000) g",
        FetchPolicy::default(),
        "pg17 stop",
    )
    .await;
    let result = o.store(0).id();
    let fetch = s.submit(Request::FetchAll { result });
    wait_until("more rows", secs(30), || o.rows() >= 50_000).await;
    let start = Instant::now();
    let outcome = s.cancel(fetch.seq).await.expect("the cancel answers");
    assert_eq!(outcome, CancelOutcome::Cancelled);
    assert!(start.elapsed() < secs(1), "{:?}", start.elapsed());
    assert_eq!(
        fetch.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert!(o.rows() >= 50_000);
    let tx = snap(s).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    run(s, "INSERT INTO hv_t18_stop VALUES (2)")
        .await
        .expect("the transaction is usable after the stop");

    // A statement that cannot suspend and is still computing: the stop waits until its first
    // rows are a second late, then cancels on the server, which fails the transaction, as in
    // psql (P16). Nothing hides it.
    let o = Recorder::new();
    let slow = s.run(spec_with(
        &o,
        &[("SELECT pg_sleep(60), a FROM hv_t18_stop FOR UPDATE", false)],
        FetchPolicy::default(),
    ));
    wait_until("the unit started", secs(30), || !o.events().is_empty()).await;
    tokio::time::sleep(ms(300)).await;
    let start = Instant::now();
    let outcome = s.cancel(slow.seq).await.expect("the cancel answers");
    assert_eq!(outcome, CancelOutcome::Cancelled);
    assert!(start.elapsed() < secs(2), "{:?}", start.elapsed());
    assert_eq!(
        slow.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(o.unit_ends(), vec![UnitEnd::Cancelled]);
    let tx = snap(s).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::Failed, TxOwner::User));
    run(s, "ROLLBACK").await.expect("the rollback runs");
    assert_eq!(snap(s).tx.status, TxStatus::Idle);
    run(s, "SELECT 1").await.expect("the session is usable");
    on.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

/// A million rows with a 100-byte padding column, without recursion: far more than the socket
/// buffers hold, so the server blocks on its write while the cursor is suspended.
const MYSQL_BIG: &str = "WITH d(x) AS (SELECT 0 UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL \
    SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 \
    UNION ALL SELECT 8 UNION ALL SELECT 9) \
    SELECT a.x + b.x * 10 + c.x * 100 + e.x * 1000 + f.x * 10000 + g.x * 100000 AS n, \
    REPEAT('x', 100) AS pad FROM d a, d b, d c, d e, d f, d g";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_release_kills_on_the_side_connection_and_reuses_within_1s() {
    if !docker_enabled() {
        return;
    }
    for id in ["mysql84", "mariadb114"] {
        let Some(t) = require_target(id) else {
            continue;
        };
        let on = session_on(&t, Path::Direct).await;
        let thread = backend_id(&on.s, t.engine).await;
        let o = suspend(&on.s, MYSQL_BIG, FetchPolicy::default(), id).await;
        // No transaction of Hacivat's: the cursor is the unread stream.
        let s = snap(&on.s);
        assert_eq!(s.tx.status, TxStatus::Idle, "{id}");
        assert_eq!(s.cursor.map(|c| c.holds_tx), Some(false), "{id}");
        // The statement still runs on the server (blocked on its write).
        let mut admin = admin(&t).await;
        let running = format!(
            "SELECT COUNT(*) FROM information_schema.processlist \
             WHERE id = {thread} AND command <> 'Sleep'"
        );
        assert_eq!(count(&mut admin, &running).await, 1, "{id}");

        // Another statement: drain for 250 ms, kill on the side connection, read on to the
        // kill's error, then run it on the same connection.
        let start = Instant::now();
        run(&on.s, "SELECT 1")
            .await
            .unwrap_or_else(|e| panic!("{id}: SELECT 1 after the release: {:?}", e.kind));
        let took = start.elapsed();
        assert!(took < secs(1), "{id}: the release took {took:?}");
        assert_eq!(
            o.releases(),
            vec![(CloseReason::SessionNeeded, false)],
            "{id}"
        );
        assert_eq!(
            *o.snap(0).completeness(),
            closed(CloseReason::SessionNeeded),
            "{id}"
        );
        assert_eq!(o.rows(), 10_000, "{id}");
        // The same server thread, idle again: the connection was reused, not replaced.
        assert_eq!(backend_id(&on.s, t.engine).await, thread, "{id}");
        assert_eq!(count(&mut admin, &running).await, 0, "{id}");
        assert_eq!(snap(&on.s).cursor, None, "{id}");

        // And a cursor that is fetched to its end needs no kill.
        let o = Recorder::new();
        run_spec(
            &on.s,
            spec_of(&o, &["SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3"]),
        )
        .await
        .unwrap_or_else(|e| panic!("{id}: {:?}", e.kind));
        assert_eq!(*o.snap(0).completeness(), Completeness::Complete, "{id}");
        assert_eq!(o.rows(), 3, "{id}");
        admin.close().await;
        on.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// SQLite (the real driver; no server)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sqlite_rollback_journal_cursor_has_the_30s_bound() {
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = dir.path().join("hv_t18.sqlite");
    let on = OnManager::connect(deps(), sqlite_profile(&path)).await;
    assert_eq!(on.profile.engine, DbmsKind::Sqlite);
    let s = &on.s;
    for sql in [
        "CREATE TABLE t(a INTEGER)",
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 50000) \
         INSERT INTO t SELECT x FROM c",
    ] {
        run(s, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    }
    // The default journal mode is a rollback journal: a live statement holds a SHARED lock, so
    // the cursor may idle 30 s at most.
    let o = suspend(
        s,
        "SELECT a FROM t ORDER BY a",
        FetchPolicy::default(),
        "sqlite",
    )
    .await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(30));
    assert!(!info.holds_tx);
    // A write on the same session releases the cursor first (single-session: every client
    // shares it), then runs; the statement was finalized, so the write finds no lock.
    let start = Instant::now();
    run(s, "INSERT INTO t VALUES (0)")
        .await
        .expect("the insert runs");
    assert!(start.elapsed() < secs(2), "{:?}", start.elapsed());
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, false)]);
    assert_eq!(o.rows(), 10_000);
    assert_eq!((int_at(&o, 0), int_at(&o, 9_999)), (1, 10_000));

    // The injected bound releases an idle cursor (`finalize`), and the session runs on.
    let fetch = FetchPolicy {
        idle_cap: Some(secs(1)),
        ..FetchPolicy::default()
    };
    let o = suspend(s, "SELECT a FROM t ORDER BY a", fetch, "sqlite idle").await;
    let mut state = s.state();
    tokio::time::timeout(secs(15), state.wait_for(|s| s.cursor.is_none()))
        .await
        .expect("the idle bound released the cursor")
        .expect("the session is alive");
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, false)]);

    // In WAL mode the engine sets no bound: the policy's 10 minutes.
    run(s, "PRAGMA journal_mode = WAL")
        .await
        .expect("the pragma runs");
    let o = suspend(
        s,
        "SELECT a FROM t ORDER BY a",
        FetchPolicy::default(),
        "sqlite wal",
    )
    .await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(600));
    // Fetching to the end completes the result and finalizes the statement.
    let result = o.store(0).id();
    ask(s, Request::FetchAll { result })
        .await
        .expect("the fetch runs");
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.rows(), 50_001);
    on.shutdown().await;
}
