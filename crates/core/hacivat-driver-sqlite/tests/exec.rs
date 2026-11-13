//! Statement execution on a SQLite connection (Task 11, HV-EXEC-05): one statement per call
//! (the full tail), rows as text, the first keyword as the command tag (Q35), the
//! transaction status from autocommit, and the interrupt.

#![allow(
    clippy::result_large_err,
    reason = "RawError is a fixed interface; one per failed call"
)]

mod common;

use std::time::Instant;

use common::*;
use hacivat_driver::{ExecOpts, RawErrorKind, TxStatus};

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

#[tokio::test]
async fn interrupt_cancels_a_long_recursive_cte() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    let start = Instant::now();
    let (r, _) = tokio::join!(
        c.execute(
            "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT count(*) FROM r",
            ExecOpts::default()
        ),
        async {
            tokio::time::sleep(ms(200)).await;
            h.cancel().await
        }
    );
    assert!(r.is_err());
    assert!(start.elapsed() < secs(2));
    assert!(c.execute("SELECT 1", simple()).await.is_ok());
}

#[tokio::test]
async fn an_interrupted_statement_is_cancelled_and_a_stale_interrupt_is_harmless() {
    let mut c = rw(db_with_table()).await;
    let h = c.cancel_handle().unwrap();
    // No statement runs: the interrupt changes nothing for the next one.
    h.cancel().await.unwrap();
    assert_eq!(text(&mut c, "SELECT 2").await, "2");
    let (r, _) = tokio::join!(
        c.execute(
            "WITH RECURSIVE r(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM r) SELECT max(i) FROM r",
            ExecOpts::default()
        ),
        async {
            tokio::time::sleep(ms(100)).await;
            h.cancel().await
        }
    );
    let e = r.unwrap_err();
    assert_eq!(e.kind, RawErrorKind::Cancelled);
    // SQLITE_INTERRUPT
    assert_eq!(e.server.and_then(|s| s.vendor_code), Some(9));
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
