//! Statement execution on a SQLite connection (Task 11, HV-EXEC-05): one statement per call
//! (the full tail), rows as text, the first keyword as the command tag (Q35), the
//! transaction status from autocommit, and the interrupt.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;

use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use common::*;
use futures::FutureExt as _;
use hacivat_driver::{CancelHandle, ExecOpts, RawErrorKind, TxStatus};
use tokio::time::timeout;

#[tokio::test]
async fn full_tail_required() {
    let mut c = rw(db_with_table()).await;
    assert!(
        c.execute("SELECT 1; DELETE FROM t", simple())
            .await
            .is_err()
    );
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1");
}

#[tokio::test]
async fn a_tail_of_trivia_is_fine_and_a_hidden_second_statement_is_not() {
    let mut c = rw(db_with_table()).await;
    for sql in ["SELECT 1;", "SELECT 1; -- done", "SELECT 1 /* x */ ;  \n"] {
        assert_eq!(text(&mut c, sql).await, "1", "{sql}");
    }
    for sql in [
        "SELECT 1;/* x */DELETE FROM t",
        "SELECT 1;; DELETE FROM t",
        "SELECT 1;\n-- x\nDELETE FROM t;",
    ] {
        assert!(c.execute(sql, simple()).await.is_err(), "{sql}");
    }
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1");
    // Leading empty statements are skipped by SQLite's parser: this is one statement, and
    // its tag is its first keyword.
    let s = c.execute("; ;DELETE FROM t", simple()).await.unwrap();
    assert_eq!(
        (s.command_tag.as_deref(), s.rows_affected),
        (Some("DELETE"), Some(1))
    );
}

#[tokio::test]
async fn tx_status_from_autocommit() {
    let mut c = rw(db_with_table()).await;
    c.batch("BEGIN").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    c.batch("COMMIT").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

#[tokio::test]
async fn execute_reports_the_transaction_status_too() {
    let mut c = rw(db_with_table()).await;
    let s = c.execute("BEGIN", simple()).await.unwrap();
    assert_eq!((s.tx, c.tx_status()), (TxStatus::InTx, TxStatus::InTx));
    c.execute("INSERT INTO t VALUES (2)", simple())
        .await
        .unwrap();
    let s = c.execute("ROLLBACK", simple()).await.unwrap();
    assert_eq!((s.tx, c.tx_status()), (TxStatus::Idle, TxStatus::Idle));
    assert_eq!(text(&mut c, "SELECT count(*) FROM t").await, "1");
}

/// A statement that never ends by itself.
const ENDLESS: &str =
    "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r";

/// A statement that runs long enough to meet the progress handler, then ends.
const COUNTED: &str = "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r WHERE i < 100000) \
                       SELECT count(*) FROM r";

/// The bound on any wait for a statement in these tests.
const LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Awaits `run`, whose statement never ends unless cancelled, for at most [`LIMIT`]. A lost
/// cancel fails the test instead of hanging it: before the panic the statement (running by
/// then, so the latch takes) is cancelled again, so the runtime's blocking thread can end.
async fn ended<T>(run: impl Future<Output = T>, h: &Arc<dyn CancelHandle>, what: &str) -> T {
    match timeout(LIMIT, run).await {
        Ok(r) => r,
        Err(_) => {
            let _ = h.cancel().await;
            panic!("{what}: the cancel was lost")
        }
    }
}

/// Plan-mandated (Task 11 Step 1), adapted in fix round 1 (review I2): the cancel is sent as
/// soon as the statement is submitted (`biased` polls the execute first), with no sleep, and
/// the whole exchange is bounded by a timeout.
#[tokio::test]
async fn interrupt_cancels_a_long_recursive_cte() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    let start = Instant::now();
    let run = async {
        tokio::join!(biased; c.execute(ENDLESS, ExecOpts::default()), async { h.cancel().await })
    };
    let (r, _) = ended(run, &h, "plan test").await;
    assert!(r.is_err());
    assert!(start.elapsed() < secs(2));
    assert!(c.execute("SELECT 1", simple()).await.is_ok());
}

/// Review I1: a cancel sent after the statement is submitted but before the connection's
/// thread starts it is latched, not cleared with SQLite's interrupt flag. 50 rounds, each
/// bounded.
#[tokio::test]
async fn an_early_cancel_is_latched() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    for round in 0..50 {
        let mut run = c.execute(ENDLESS, ExecOpts::default());
        // Submitted: the job is on its way to the connection's thread.
        assert!((&mut run).now_or_never().is_none());
        h.cancel().await.unwrap();
        let e = ended(run, &h, &format!("round {round}")).await.unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Cancelled, "round {round}");
        // SQLITE_INTERRUPT
        assert_eq!(e.server.and_then(|s| s.vendor_code), Some(9));
    }
    assert_eq!(text(&mut c, "SELECT 2").await, "2");
}

/// The trusted `batch` path is latched the same way; the interrupted statement leaves nothing.
#[tokio::test]
async fn an_early_cancel_ends_a_batch() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    let mut run = c.batch(
        "CREATE TABLE big AS WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) \
         SELECT i FROM r",
    );
    assert!((&mut run).now_or_never().is_none());
    h.cancel().await.unwrap();
    let e = ended(run, &h, "batch").await.unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Cancelled);
    assert_eq!(
        text(
            &mut c,
            "SELECT count(*) FROM sqlite_schema WHERE name = 'big'"
        )
        .await,
        "0"
    );
}

/// A cancel while nothing runs (before any statement, or after one completed) does nothing,
/// and never reaches the next statement, which runs long enough to meet the progress handler.
#[tokio::test]
async fn a_cancel_while_nothing_runs_never_reaches_the_next_statement() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    h.cancel().await.unwrap();
    for round in 0..20 {
        assert_eq!(text(&mut c, "SELECT 1").await, "1");
        h.cancel().await.unwrap();
        let s = timeout(LIMIT, c.execute(COUNTED, simple()))
            .await
            .expect("bounded")
            .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        assert_eq!(s.rows[0][0].as_deref(), Some("100000"), "round {round}");
        h.cancel().await.unwrap();
        timeout(LIMIT, c.batch("CREATE TEMP TABLE IF NOT EXISTS x(a)"))
            .await
            .expect("bounded")
            .unwrap();
    }
}

/// The cancel of a request whose future was dropped before it ran does not reach the next
/// request: each request takes its own ticket.
#[tokio::test]
async fn a_dropped_request_does_not_pass_its_cancel_on() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    {
        let mut run = c.execute("SELECT 1", simple());
        assert!((&mut run).now_or_never().is_none());
        h.cancel().await.unwrap();
    }
    let s = timeout(LIMIT, c.execute(COUNTED, simple()))
        .await
        .expect("bounded")
        .unwrap();
    assert_eq!(s.rows[0][0].as_deref(), Some("100000"));
}

/// Review R1-M1: a request cancelled and then dropped before its job ran keeps its cancel.
/// The next request's ticket does not reset it, so the orphaned endless statement stops and
/// the next request runs. 20 rounds, each bounded.
#[tokio::test]
async fn a_cancelled_then_dropped_request_still_stops() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    for round in 0..20 {
        {
            let mut run = c.execute(ENDLESS, ExecOpts::default());
            assert!((&mut run).now_or_never().is_none());
            h.cancel().await.unwrap();
        }
        let s = ended(c.execute(COUNTED, simple()), &h, &format!("round {round}"))
            .await
            .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        assert_eq!(s.rows[0][0].as_deref(), Some("100000"), "round {round}");
    }
}

/// Review R1-I1: `close` stops a statement whose request future was dropped without a cancel
/// (the session's abandoned path), and a job queued behind it, so it returns within the
/// bound and the connection's thread exits (`close` resolves only after the thread dropped
/// the connection). On both profiles.
#[tokio::test]
async fn close_stops_a_statement_whose_request_was_dropped() {
    for read_only in [false, true] {
        let mut c = if read_only {
            ro(db_with_table()).await
        } else {
            rw(db_with_table()).await
        };
        let h = c.cancel_handle().unwrap();
        for _ in 0..2 {
            let mut run = c.execute(ENDLESS, ExecOpts::default());
            assert!((&mut run).now_or_never().is_none());
        }
        if timeout(LIMIT, c.close()).await.is_err() {
            let _ = h.cancel().await;
            panic!("read_only {read_only}: close waited on the abandoned statement");
        }
    }
}

#[tokio::test]
async fn the_cancel_handle_outlives_the_connection() {
    let c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    drop(c);
    assert!(h.cancel().await.is_ok());
}

#[tokio::test]
async fn rows_come_back_as_text() {
    let mut c = rw(db_with_table()).await;
    let s = c
        .execute(
            "SELECT 42 AS i, 1.5 AS r, 2.0 AS w, 'x' AS s, NULL AS n, x'00ff' AS b",
            simple(),
        )
        .await
        .unwrap();
    assert_eq!(s.columns, ["i", "r", "w", "s", "n", "b"]);
    let row: Vec<Option<&str>> = s.rows[0].iter().map(|v| v.as_deref()).collect();
    assert_eq!(
        row,
        [
            Some("42"),
            Some("1.5"),
            Some("2.0"),
            Some("x"),
            None,
            Some("\\x00ff")
        ]
    );
    assert_eq!((s.row_count, s.truncated), (1, false));
    assert_eq!(s.command_tag.as_deref(), Some("SELECT"));
    assert_eq!(s.rows_affected, None);
}

#[tokio::test]
async fn rows_beyond_max_rows_are_counted_not_kept() {
    let mut c = rw(db_with_table()).await;
    let opts = ExecOpts {
        max_rows: 3,
        ..simple()
    };
    let s = c
        .execute(
            "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r WHERE i < 10) \
             SELECT i FROM r",
            opts,
        )
        .await
        .unwrap();
    assert_eq!(s.rows.len(), 3);
    assert_eq!((s.row_count, s.truncated), (10, true));
    assert_eq!(s.rows[2][0].as_deref(), Some("3"));
}

#[tokio::test]
async fn command_tag_is_the_first_keyword_and_writes_count_rows() {
    let mut c = rw(db_with_table()).await;
    let s = c
        .execute("  /* c */ -- x\n insert into t values (2), (3)", simple())
        .await
        .unwrap();
    assert_eq!(s.command_tag.as_deref(), Some("INSERT"));
    assert_eq!(s.rows_affected, Some(2));
    let s = c.execute("update t set a = a + 1", simple()).await.unwrap();
    assert_eq!(
        (s.command_tag.as_deref(), s.rows_affected),
        (Some("UPDATE"), Some(3))
    );
    let s = c
        .execute("Delete From t Where a > 2 Returning a", simple())
        .await
        .unwrap();
    assert_eq!(
        (s.command_tag.as_deref(), s.rows_affected),
        (Some("DELETE"), Some(2))
    );
    assert_eq!(s.row_count, 2);
    // A schema change reports its tag and no rows.
    let s = c.execute("create table u(x)", simple()).await.unwrap();
    assert_eq!(
        (s.command_tag.as_deref(), s.rows_affected),
        (Some("CREATE"), Some(0))
    );
}

#[tokio::test]
async fn errors_keep_the_engine_text_and_code() {
    let mut c = rw(db_with_table()).await;
    let e = err(&mut c, "SELECT nope FROM t").await;
    assert_eq!(e.kind, RawErrorKind::Db);
    let server = e.server.unwrap();
    assert_eq!(server.vendor_code, Some(1));
    assert!(server.text.contains("no such column"), "{}", server.text);
}

#[tokio::test]
async fn an_empty_statement_runs_nothing() {
    let mut c = rw(db_with_table()).await;
    for sql in ["", "  -- nothing\n", " ; /* x */ ;"] {
        let s = c.execute(sql, simple()).await.unwrap();
        assert_eq!((s.command_tag, s.row_count), (None, 0), "{sql:?}");
    }
    let mut c = ro(db_with_table()).await;
    assert_eq!(c.execute(";", simple()).await.unwrap().row_count, 0);
}

#[tokio::test]
async fn reset_and_backend_id() {
    let mut c = rw(db_with_table()).await;
    assert!(c.reset().await.is_ok());
    assert_eq!(c.backend_id(), None);
}
