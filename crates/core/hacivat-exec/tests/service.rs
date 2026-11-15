//! M3a Task 23: the run service (HV-EXEC-05) over a FakeDriver session: what is sent, run
//! status, result tabs, generations, log lines, errors, cancel, cursors, later-fetch failures
//! and a run that never started. A paused clock: no socket, no real time.

mod common;

use std::time::Duration;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{FakeQuery, FakeTypedCall};
use hacivat_exec::{LogMsg, UnitStatus};
use hacivat_results::{CloseReason, Completeness, FetchPolicy};
use hacivat_session::session::{CancelOutcome, ClosePolicy, RowsOutcome};

fn queries(f: &hacivat_driver::fake::FakeDriver) -> Vec<String> {
    f.typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(sql) => Some(sql),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn select_1_event_order_and_status() {
    let (svc, h, f) = service_over(fake_pg().queries(vec![rows_q(1)])).await;
    let mut t = svc.svc.run(&h, svc.req(&["SELECT 1"]));
    assert_eq!(
        status_trail(&mut t).await,
        ["Queued", "Running", "Succeeded"]
    );
    assert_eq!(svc.log_ids(), ["exec-rows-retrieved"]);
    let summary = t.done.await.unwrap();
    assert_eq!(
        (summary.executed, summary.total, summary.aborted),
        (1, 1, false)
    );
    assert_eq!(queries(&f), ["SELECT 1"]);
    let s = t.status.borrow();
    assert!(s.started.is_some());
    assert_eq!(s.run, Some(t.run));
    assert!(matches!(
        s.units[0].status,
        UnitStatus::Succeeded(u) if u.rows == Some(RowsOutcome::Fetched { rows: 1, more: false })
    ));
}

#[tokio::test(start_paused = true)]
async fn two_units_two_tabs_titled_result_n() {
    let (svc, h, f) =
        service_over(fake_pg().queries(vec![rows_of("SELECT 1", 1), rows_of("SELECT 2", 2)])).await;
    let t = svc.svc.run(&h, svc.req(&["SELECT 1", "SELECT 2"]));
    let results = t.results.clone();
    let summary = t.done.await.unwrap();
    assert_eq!((summary.executed, summary.total), (2, 2));
    let tabs = results.borrow().clone();
    assert_eq!(
        tabs.iter().map(|t| &*t.title).collect::<Vec<_>>(),
        ["Result 1", "Result 2"]
    );
    assert_eq!(
        tabs.iter()
            .map(|t| (t.key.console, t.key.unit_ordinal, t.key.result_ordinal))
            .collect::<Vec<_>>(),
        [(7, 0, 0), (7, 1, 0)]
    );
    assert_eq!(tabs[1].source.snapshot().row_count(), 2);
    assert_eq!(
        *tabs[1].source.snapshot().completeness(),
        Completeness::Complete
    );
    // Units in order, each its exact text; a summary line for a run of several.
    assert_eq!(queries(&f), ["SELECT 1", "SELECT 2"]);
    assert_eq!(
        svc.log_ids(),
        ["exec-rows-retrieved", "exec-rows-retrieved", "exec-summary"]
    );
    let lines = svc.log.lines();
    assert_eq!(
        lines[1].source,
        Some(10..18),
        "the line points at its statement"
    );
    assert_eq!(lines[1].unit.map(|u| u.ordinal), Some(1));
}

#[tokio::test(start_paused = true)]
async fn rerun_reuses_the_tab_key_and_bumps_generation() {
    let (svc, h, _f) = service_over(fake_pg().queries(vec![rows_q(1), rows_q(3)])).await;
    let first = svc.svc.run(&h, svc.req(&["SELECT 1"]));
    let first_tabs = first.results.clone();
    first.done.await.unwrap();
    let second = svc.svc.run(&h, svc.req(&["SELECT 1"]));
    let second_tabs = second.results.clone();
    second.done.await.unwrap();
    assert_ne!(first.run, second.run);
    let (a, b) = (
        first_tabs.borrow()[0].clone(),
        second_tabs.borrow()[0].clone(),
    );
    assert_eq!(a.key, b.key);
    assert_eq!(a.source.snapshot().generation(), 1);
    assert_eq!(b.source.snapshot().generation(), 2);
    assert_eq!(b.source.snapshot().row_count(), 3);
    // Another console's tab of the same unit is its own.
    let mut other = svc.req(&["SELECT 1"]);
    other.console = 8;
    let (svc2, h2, _f2) = service_over(fake_pg().queries(vec![rows_q(1)])).await;
    let third = svc2.svc.run(&h2, other);
    let third_tabs = third.results.clone();
    third.done.await.unwrap();
    assert_eq!(third_tabs.borrow()[0].source.snapshot().generation(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_unit_keeps_the_server_text_verbatim_and_stops_the_run() {
    let failing = FakeQuery {
        fail_after: Some((0, pg_error("42P01", "relation \"İŞLEM\" does not exist"))),
        ..rows_of("SELECT * FROM", 10)
    };
    let (svc, h, f) = service_over(fake_pg().queries(vec![rows_q(1), failing])).await;
    let t = svc.svc.run(
        &h,
        svc.req(&["SELECT 1", "SELECT * FROM İŞLEM", "UPDATE t SET a = 1"]),
    );
    let status = t.status.clone();
    let summary = t.done.await.unwrap();
    assert_eq!(
        (
            summary.executed,
            summary.failed,
            summary.skipped,
            summary.aborted
        ),
        (1, 1, 1, true)
    );
    let s = status.borrow();
    assert_eq!(
        s.units.iter().map(|u| u.status.name()).collect::<Vec<_>>(),
        ["Succeeded", "Failed", "Skipped"]
    );
    let UnitStatus::Failed(view) = &s.units[1].status else {
        unreachable!()
    };
    assert_eq!(view.title_id, "exec-error-server");
    assert_eq!(view.code.as_deref(), Some("42P01"));
    assert_eq!(
        view.server_text.as_deref(),
        Some("relation \"İŞLEM\" does not exist")
    );
    assert_eq!(view.hint_id, None);
    // The fake (like the driver) reports no error cursor: no offset is made up.
    assert_eq!(view.doc_offset, None);
    assert_eq!(
        svc.log_msgs()[1],
        LogMsg::Error {
            n: 2,
            view: view.clone()
        }
    );
    assert_eq!(
        svc.log_ids(),
        ["exec-rows-retrieved", "exec-error", "exec-summary"]
    );
    // StopOnError: the third unit was never sent.
    assert_eq!(queries(&f), ["SELECT 1", "SELECT * FROM İŞLEM"]);
}

#[test]
fn failed_unit_maps_server_position_to_the_document() {
    use hacivat_exec::{doc_offset_of, error_view};
    use hacivat_session::session::{ExecError, ExecErrorKind};
    // PostgreSQL's cursor is 1-based in characters: `x` is the 9th character of the unit,
    // after the two-byte `ç`; the unit starts at byte 20 of the document.
    let unit = "SELECT çx FROM t";
    assert_eq!(doc_offset_of(unit, 20, 9), Some(20 + 9));
    assert_eq!(doc_offset_of(unit, 20, 1), Some(20));
    // One past the last character is the end of the text; further, or 0, is no position.
    let chars = u32::try_from(unit.chars().count()).unwrap();
    assert_eq!(doc_offset_of(unit, 20, chars + 1), Some(20 + unit.len()));
    assert_eq!(doc_offset_of(unit, 20, chars + 2), None);
    assert_eq!(doc_offset_of(unit, 20, 0), None);
    let e = ExecError {
        kind: ExecErrorKind::Db,
        server: pg_error("42703", "column \"x\" does not exist").server,
    };
    let view = error_view(hacivat_proto::DbmsKind::Postgres, &e, unit, 20, Some(9));
    assert_eq!(view.doc_offset, Some(29));
    assert_eq!(view.code.as_deref(), Some("42703"));
}

#[tokio::test(start_paused = true)]
async fn more_result_sets_are_logged() {
    let call = FakeQuery {
        prefix: "CALL",
        sets: vec![generated(2), generated(5), generated(7)],
        ..Default::default()
    };
    let (svc, h, _f) = service_over(
        hacivat_driver::fake::FakeDriver::new(hacivat_proto::DbmsKind::MySql).queries(vec![call]),
    )
    .await;
    let t = svc.svc.run(&h, svc.req(&["CALL p()"]));
    let results = t.results.clone();
    t.done.await.unwrap();
    assert_eq!(svc.log_msgs()[0], LogMsg::MoreResultSets { skipped: 2 });
    assert_eq!(
        svc.log_ids(),
        ["exec-more-result-sets", "exec-rows-retrieved"]
    );
    assert_eq!(results.borrow().len(), 1, "one tab per unit (R-M3-25)");
}

#[tokio::test(start_paused = true)]
async fn dml_reports_rows_affected_and_ddl_completed() {
    let insert = FakeQuery {
        prefix: "INSERT",
        rows_affected: Some(3),
        ..Default::default()
    };
    let create = FakeQuery {
        prefix: "CREATE",
        ..Default::default()
    };
    let (svc, h, _f) = service_over(fake_pg().queries(vec![insert, create])).await;
    let t = svc.svc.run(
        &h,
        svc.req(&[
            "INSERT INTO t VALUES (1), (2), (3)",
            "CREATE TABLE u (a int)",
        ]),
    );
    let results = t.results.clone();
    t.done.await.unwrap();
    let msgs = svc.log_msgs();
    assert!(
        matches!(msgs[0], LogMsg::RowsAffected { rows: 3, .. }),
        "{msgs:?}"
    );
    assert!(matches!(msgs[1], LogMsg::Completed { .. }), "{msgs:?}");
    assert!(results.borrow().is_empty(), "no columns, no result tab");
}

#[tokio::test(start_paused = true)]
async fn user_transactions_are_logged_when_they_end() {
    let tx = |prefix, after| FakeQuery {
        prefix,
        tx_after: Some(after),
        ..Default::default()
    };
    let script = vec![
        tx("BEGIN", TxStatus::InTx),
        tx("COMMIT", TxStatus::Idle),
        tx("BEGIN", TxStatus::InTx),
        tx("ROLLBACK", TxStatus::Idle),
    ];
    let (svc, h, _f) = service_over(fake_pg().queries(script)).await;
    let t = svc
        .svc
        .run(&h, svc.req(&["BEGIN", "UPDATE t SET a = 1", "COMMIT"]));
    t.done.await.unwrap();
    assert_eq!(
        svc.log_ids(),
        [
            "exec-completed",
            "exec-completed",
            "exec-tx-committed",
            "exec-completed",
            "exec-summary"
        ]
    );
    svc.log.clear();
    let t = svc.svc.run(&h, svc.req(&["BEGIN", "ROLLBACK"]));
    t.done.await.unwrap();
    assert!(svc.log_ids().contains(&"exec-tx-rolled-back"));
    assert!(!svc.log_ids().contains(&"exec-tx-committed"));
    assert_eq!(h.state().borrow().tx.status, TxStatus::Idle);
}

/// A PostgreSQL result of `rows` that suspends after its first read-ahead.
fn small_fetch() -> FetchPolicy {
    FetchPolicy {
        first_rows: 10,
        chunk_rows: 10,
        read_ahead_rows: 20,
        ..FetchPolicy::default()
    }
}

#[tokio::test(start_paused = true)]
async fn a_suspended_cursor_says_so_and_its_release_rolls_back() {
    let (svc, h, f) = service_over(fake_pg().queries(vec![
        rows_of("SELECT * FROM big", 1_000_000),
        rows_of("SELECT 2", 1),
    ]))
    .await;
    let first = svc
        .svc
        .run(&h, svc.req_with(&["SELECT * FROM big"], small_fetch()));
    let status = first.status.clone();
    let summary = first.done.await.unwrap();
    assert_eq!(summary.executed, 1);
    let rows = match status.borrow().units[0].status {
        UnitStatus::Suspended { rows } => rows,
        ref other => panic!("{other:?}"),
    };
    assert!(rows > 0 && rows < 1_000_000);
    assert_eq!(svc.log_ids(), ["exec-rows-retrieved-more"]);
    assert!(h.state().borrow().cursor.is_some());
    // Another statement releases the cursor first: Hacivat's own transaction rolls back
    // (R-M3-23), and the log says so.
    let second = svc.svc.run(&h, svc.req(&["SELECT 2"]));
    second.done.await.unwrap();
    let msgs = svc.log_msgs();
    assert!(
        msgs.contains(&LogMsg::CursorClosed {
            rows,
            reason: CloseReason::SessionNeeded,
            rolled_back: true,
            idle_ms: 0,
        }),
        "{msgs:?}"
    );
    assert_eq!(
        svc.log_ids(),
        [
            "exec-rows-retrieved-more",
            "cursor-closed-session-needed-rolled-back",
            "exec-rows-retrieved"
        ]
    );
    // The first run's unit follows its result: it ended with more rows than it kept.
    assert!(matches!(
        status.borrow().units[0].status,
        UnitStatus::Succeeded(u) if u.rows == Some(RowsOutcome::Fetched { rows, more: true })
    ));
    assert!(f.executed().iter().any(|s| s == "ROLLBACK"));
}

#[tokio::test(start_paused = true)]
async fn a_suspended_result_that_completes_later_is_logged() {
    let (svc, h, _f) =
        service_over(fake_pg().queries(vec![rows_of("SELECT * FROM big", 45)])).await;
    let t = svc
        .svc
        .run(&h, svc.req_with(&["SELECT * FROM big"], small_fetch()));
    let (status, results) = (t.status.clone(), t.results.clone());
    t.done.await.unwrap();
    assert_eq!(status.borrow().units[0].status.name(), "Suspended");
    let source = results.borrow()[0].source.clone();
    source.fetch_all();
    eventually("the result completes", || {
        *source.snapshot().completeness() == Completeness::Complete
    })
    .await;
    eventually("the unit follows", || {
        status.borrow().units[0].status.name() == "Succeeded"
    })
    .await;
    assert_eq!(
        svc.log_ids(),
        ["exec-rows-retrieved-more", "exec-rows-retrieved"]
    );
    assert!(matches!(
        svc.log_msgs()[1],
        LogMsg::RowsRetrieved {
            rows: 45,
            more: false,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn a_later_fetch_failure_is_logged_with_the_server_text() {
    let failing = FakeQuery {
        fail_after: Some((30, pg_error("22012", "division by zero"))),
        ..rows_of("SELECT * FROM big", 1_000)
    };
    let (svc, h, _f) = service_over(fake_pg().queries(vec![failing])).await;
    let t = svc
        .svc
        .run(&h, svc.req_with(&["SELECT * FROM big"], small_fetch()));
    let results = t.results.clone();
    t.done.await.unwrap();
    assert_eq!(svc.log_ids(), ["exec-rows-retrieved-more"]);
    let source = results.borrow()[0].source.clone();
    source.fetch_all();
    eventually("the failure is logged", || {
        svc.log_ids().contains(&"exec-error")
    })
    .await;
    let msgs = svc.log_msgs();
    let Some(LogMsg::Error { n, view }) = msgs.iter().find(|m| m.fluent_id() == "exec-error")
    else {
        unreachable!()
    };
    assert_eq!(*n, 1);
    assert_eq!(view.server_text.as_deref(), Some("division by zero"));
    assert_eq!(view.code.as_deref(), Some("22012"));
    // The session's own account of it stays: the cursor closed, the rows kept.
    assert!(
        svc.log_ids()
            .iter()
            .any(|id| id.starts_with("cursor-closed-server-terminated")),
        "{:?}",
        svc.log_ids()
    );
    assert!(matches!(
        source.snapshot().completeness(),
        Completeness::Closed { .. }
    ));
}

#[tokio::test(start_paused = true)]
async fn status_order_under_cancel() {
    let slow = FakeQuery {
        first_delay: Duration::from_secs(60),
        ..rows_of("SELECT pg_sleep", 1)
    };
    let (svc, h, f) = service_over(fake_pg().queries(vec![slow])).await;
    let mut t = svc.svc.run(
        &h,
        svc.req(&["SELECT pg_sleep(60)", "SELECT 2", "SELECT 3"]),
    );
    t.status.changed().await.unwrap();
    assert_eq!(unit_status(&t.status.borrow_and_update(), 0), "Running");
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    let summary = (&mut t.done).await.unwrap();
    assert!(summary.aborted);
    let s = t.status.borrow();
    assert_eq!(
        s.units.iter().map(|u| u.status.name()).collect::<Vec<_>>(),
        ["Cancelled", "Cancelled", "Cancelled"]
    );
    // Only the unit in flight says it was cancelled; the run's summary follows.
    assert_eq!(svc.log_ids(), ["exec-cancelled", "exec-summary"]);
    assert_eq!(queries(&f), ["SELECT pg_sleep(60)"]);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_never_started_ends_with_its_reason() {
    let (svc, h, f) = service_over(fake_pg()).await;
    h.close(ClosePolicy::RollBack).await.unwrap();
    let t = svc.svc.run(&h, svc.req(&["SELECT 1", "SELECT 2"]));
    let status = t.status.clone();
    let summary = t.done.await.unwrap();
    assert_eq!(
        (
            summary.executed,
            summary.skipped,
            summary.total,
            summary.aborted
        ),
        (0, 2, 2, true)
    );
    eventually("the reason is logged", || !svc.log_ids().is_empty()).await;
    let s = status.borrow();
    assert!(s.units.iter().all(|u| u.status == UnitStatus::Skipped));
    assert!(s.summary.is_some() && s.started.is_none());
    let msgs = svc.log_msgs();
    assert!(
        matches!(&msgs[..], [LogMsg::Error { n: 1, view }] if view.title_id == "exec-error-not-connected"),
        "{msgs:?}"
    );
    assert!(f.typed_log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn an_empty_request_sends_nothing() {
    let (svc, h, f) = service_over(fake_pg()).await;
    let t = svc.svc.run(&h, svc.req(&[]));
    let summary = t.done.await.unwrap();
    assert_eq!((summary.executed, summary.total), (0, 0));
    assert!(f.typed_log().is_empty());
    assert!(svc.log_ids().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_text_sent_is_the_units_text_byte_for_byte() {
    let text = "SELECT 'ş;\u{202E}x' /* ; */";
    let (svc, h, f) = service_over(fake_pg().queries(vec![rows_q(1)])).await;
    let t = svc.svc.run(&h, svc.req(&[text]));
    t.done.await.unwrap();
    assert_eq!(queries(&f), [text]);
}
