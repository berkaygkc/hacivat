//! M3a Task 18 over the real SQLite driver (no server, so it runs in plain `cargo test`): a
//! console cursor in a rollback-journal database has the 30 s idle bound (a live statement
//! holds a SHARED lock, so no other process can commit), a write on the session releases it
//! first, the injected bound finalizes an idle one, and WAL mode has no engine bound. Real
//! time, so waits are on conditions; no wall-clock budget.

mod common;

use std::sync::Arc;

use common::conformance::*;
use common::runs::*;
use hacivat_proto::DbmsKind;
use hacivat_results::{CloseReason, Completeness, FetchPolicy};
use hacivat_session::session::{Request, SessionHandle};
use hacivat_values::Value;

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
    run(s, "INSERT INTO t VALUES (0)")
        .await
        .expect("the insert runs");
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
