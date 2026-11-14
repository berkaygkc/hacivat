//! M3a Task 18: `Request::Run` (HV-EXEC-05): typed units streamed into result stores in chunk
//! steps, stop fetching (R-M3-16, P16), `StopOnError` (R-M3-26), update counts, and what a run
//! reports to its observer. FakeDriver and a paused clock: no socket, no real time.

mod common;

use common::runs::*;
use common::*;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeQuery, FakeTypedCall};
use hacivat_driver::{CloseHow, TxStatus};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_results::{CloseReason, Completeness, FetchPolicy, TruncReason};
use hacivat_session::session::{
    CancelOutcome, CancelState, Escalation, ExecErrorKind, Request, RowsOutcome, RunEvent,
    RunSummary, SessionState, TxCommand, TxError, TxOwner, UnitEnd,
};

/// A PostgreSQL fake that follows `BEGIN`/`COMMIT`/`ROLLBACK` (a suspendable PostgreSQL query
/// needs a transaction block, P5).
fn pg() -> FakeDriver {
    fake_pg().tracks_transactions()
}

fn typed(f: &FakeDriver) -> Vec<FakeTypedCall> {
    f.typed_log()
}

/// The statements sent through `execute`/`batch`, without the session's schema probe (it
/// follows a user's transaction control, Q27).
fn statements(f: &FakeDriver) -> Vec<String> {
    f.executed()
        .into_iter()
        .filter(|s| !s.starts_with("SELECT current_database()"))
        .collect()
}

fn fetched(rows: u64, more: bool) -> UnitEnd {
    UnitEnd::Succeeded {
        rows: Some(RowsOutcome::Fetched { rows, more }),
    }
}

// ---------------------------------------------------------------------------------------------
// Streaming mechanics

#[tokio::test(start_paused = true)]
async fn never_pending_stream_is_cancelled_within_one_chunk() {
    // The driver is always ready: only the actor's turn between chunks lets the cancel in.
    let endless = FakeQuery {
        never_pending: true,
        ..select("SELECT", u64::MAX)
    };
    let (h, f) = session(pg().queries(vec![endless])).await;
    let o = Recorder::new();
    let t = h.run(spec_with(
        &o,
        &[("SELECT * FROM big", false)],
        FetchPolicy::default(),
    ));
    settle_until("the first chunks", || o.rows() >= 4_096).await;
    let before = f.chunks_sent();
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert!(
        f.chunks_sent() - before <= 1,
        "{} chunks after the cancel",
        f.chunks_sent() - before
    );
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    // The rows stay, the result says fetching was stopped.
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert!(o.snap(0).row_count() >= 4_096);
    assert_eq!(o.unit_ends(), vec![UnitEnd::Cancelled]);
    assert_eq!(o.result_ends(), vec![Completeness::Cancelled]);
    // `Execute(0)` cannot be closed on the wire: the protocol cancel runs beside the close.
    assert_eq!(typed(&f).last(), Some(&FakeTypedCall::Close(CloseHow::Now)));
    assert_eq!(f.cancel_calls(), 1);
    assert_eq!(snap(&h).state, SessionState::Idle);
    assert_eq!(snap(&h).cancel, CancelState::Idle);
}

#[tokio::test(start_paused = true)]
async fn first_chunk_is_published_before_the_second_is_requested() {
    let slow = FakeQuery {
        chunk_delay: secs(5),
        ..select("SELECT", 1_000_000)
    };
    let (h, f) = session(pg().queries(vec![slow])).await;
    let o = Recorder::new();
    let start = tokio::time::Instant::now();
    let t = h.run(spec_of(&o, &["SELECT * FROM big"]));
    settle_until("the first chunk", || o.rows() > 0).await;
    // 500 rows are on screen while the server still works on what follows: nothing more was
    // asked for, and no time passed.
    assert_eq!(o.rows(), 500);
    assert_eq!(f.chunks_sent(), 1);
    assert_eq!(
        typed(&f),
        vec![FakeTypedCall::Query("SELECT * FROM big".into())]
    );
    assert_eq!(tokio::time::Instant::now(), start);
    assert_eq!(*o.snap(0).completeness(), Completeness::Streaming);
    t.reply.await.unwrap().unwrap();
    assert_eq!(o.rows(), 10_000);
}

#[tokio::test(start_paused = true)]
async fn read_ahead_stops_at_10k_rows_or_16_mib() {
    // Rows: 500 first, then batches of at most a chunk up to the read-ahead.
    let (h, f) = session(pg().queries(vec![select("SELECT", 1_000_000)])).await;
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &["SELECT * FROM big"]))
        .await
        .unwrap();
    assert_eq!(o.rows(), 10_000);
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(
        typed(&f),
        vec![
            FakeTypedCall::Query("SELECT * FROM big".into()),
            FakeTypedCall::FetchMore(4_096),
            FakeTypedCall::FetchMore(4_096),
            FakeTypedCall::FetchMore(1_308),
        ]
    );
    assert_eq!(o.unit_ends(), vec![fetched(10_000, true)]);
    // A result that stays suspended has not ended.
    assert!(o.result_ends().is_empty());

    // Bytes: the first chunk already holds the (tiny) byte limit, so nothing more is read.
    let (h, f) = session(pg().queries(vec![select("SELECT", 1_000_000)])).await;
    let o = Recorder::new();
    let fetch = FetchPolicy {
        read_ahead_bytes: 1,
        ..FetchPolicy::default()
    };
    run_spec(&h, spec_with(&o, &[("SELECT * FROM big", true)], fetch))
        .await
        .unwrap();
    assert_eq!(o.rows(), 500);
    assert_eq!(typed(&f).len(), 1, "{:?}", typed(&f));
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
}

#[tokio::test(start_paused = true)]
async fn a_small_result_completes_and_commits_hacivats_transaction() {
    let (h, f) = session(pg().queries(vec![select("SELECT", 3)])).await;
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &["SELECT 3"])).await.unwrap();
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.rows(), 3);
    assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT"]);
    assert_eq!(o.unit_ends(), vec![fetched(3, false)]);
    assert_eq!(o.result_ends(), vec![Completeness::Complete]);
    // A read reports no update count (`rows_affected` is `None` for a result with rows).
    assert!(
        !o.events()
            .iter()
            .any(|e| matches!(e, RunEvent::UpdateCount { .. }))
    );
    assert_eq!(snap(&h).cursor, None);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).tx.owner, TxOwner::None);
}

#[tokio::test(start_paused = true)]
async fn non_suspendable_unit_streams_to_completion_and_counts_past_the_cap() {
    let (h, f) = session(pg().queries(vec![select("WITH", 20_000)])).await;
    // 64 KiB per result: about two chunks of 4,096 eight-byte rows.
    let o = Recorder::with_budget(1 << 30, 64 << 10);
    run_spec(
        &h,
        spec_with(
            &o,
            &[(
                "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d",
                false,
            )],
            FetchPolicy::default(),
        ),
    )
    .await
    .unwrap();
    let kept = o.rows();
    assert!(kept > 0 && kept < 20_000, "kept {kept}");
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Truncated(TruncReason::ResultCap)
    );
    // Every row was read and counted: the statement ran to its end.
    assert_eq!(o.unit_ends(), vec![fetched(20_000, false)]);
    assert_eq!(f.chunks_sent(), 1 + (20_000 - 500_u64).div_ceil(4_096));
    // Never a transaction or a savepoint of Hacivat's around a unit that cannot suspend.
    assert!(f.executed().is_empty(), "{:?}", f.executed());
    assert_eq!(typed(&f).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn only_the_last_unit_stays_suspended() {
    let (h, f) = session(pg().queries(vec![
        select("SELECT * FROM a", 1_000_000),
        select("SELECT * FROM b", 1_000_000),
    ]))
    .await;
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &["SELECT * FROM a", "SELECT * FROM b"]))
        .await
        .unwrap();
    // The first read ahead, then gave the session to the second; its transaction rolled back.
    assert_eq!(o.snap(0).row_count(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Closed {
            reason: CloseReason::SessionNeeded
        }
    );
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    assert!(matches!(
        o.snap(1).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK", "BEGIN"]);
    assert_eq!(
        o.unit_ends(),
        vec![fetched(10_000, true), fetched(10_000, true)]
    );
    assert_eq!(snap(&h).cursor.map(|c| c.result), Some(o.store(1).id()));
    assert_eq!(snap(&h).tx.owner, TxOwner::Cursor);
}

// ---------------------------------------------------------------------------------------------
// Stop fetching (R-M3-16, P16)

#[tokio::test(start_paused = true)]
async fn overdue_chunk_sends_the_server_cancel() {
    // The first chunk comes at once, the next one after a minute.
    let slow = FakeQuery {
        chunk_delay: secs(60),
        ..select("SELECT", 1_000_000)
    };
    let (h, f) = session(pg().queries(vec![slow])).await;
    let o = Recorder::new();
    let t = h.run(spec_of(&o, &["SELECT * FROM big"]));
    settle_until("the first chunk", || o.rows() == 500).await;
    let asked = tokio::time::Instant::now();
    let outcome = h.cancel(t.seq).await.unwrap();
    // Not before the chunk was a second late, and then at once.
    assert_eq!(asked.elapsed(), secs(1));
    assert_eq!(outcome, CancelOutcome::Cancelled);
    assert_eq!(f.cancel_calls(), 1);
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(o.rows(), 500);
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert_eq!(o.unit_ends(), vec![UnitEnd::Cancelled]);
    // Hacivat's own transaction is gone with the cursor.
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    assert_eq!(o.releases(), vec![(CloseReason::Cancelled, true)]);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

#[tokio::test(start_paused = true)]
async fn a_stop_between_prompt_chunks_sends_no_server_cancel() {
    // A suspendable stream that is always ready: the stop lands between chunks, where only
    // the engine's own close is sent.
    let endless = FakeQuery {
        never_pending: true,
        ..select("SELECT", u64::MAX)
    };
    let (h, f) = session(pg().queries(vec![endless])).await;
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &["SELECT * FROM big"]))
        .await
        .unwrap();
    let result = o.store(0).id();
    let t = h.submit(Request::FetchAll { result });
    settle_until("more rows", || o.rows() >= 20_000).await;
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(f.cancel_calls(), 0);
    assert_eq!(typed(&f).last(), Some(&FakeTypedCall::Close(CloseHow::Now)));
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert_eq!(o.releases(), vec![(CloseReason::Cancelled, true)]);
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    assert_eq!(snap(&h).cursor, None);
}

#[tokio::test(start_paused = true)]
async fn a_sqlite_stop_of_a_flowing_stream_never_interrupts() {
    // Fix round 1 (review I1). An interrupt during a write inside an explicit SQLite
    // transaction rolls the user's whole transaction back, so a stop between chunks sends only
    // the close (R-M3-16).
    let endless = FakeQuery {
        never_pending: true,
        ..select("UPDATE", u64::MAX)
    };
    let fake = FakeDriver::new(DbmsKind::Sqlite)
        .tracks_transactions()
        .queries(vec![endless]);
    let (h, f) = session(fake).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let t = h.run(spec_with(
        &o,
        &[("UPDATE t SET a = a + 1 RETURNING a", false)],
        FetchPolicy::default(),
    ));
    settle_until("the first chunks", || o.rows() >= 4_096).await;
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(f.cancel_calls(), 0, "no interrupt for a stream that flows");
    assert_eq!(typed(&f).last(), Some(&FakeTypedCall::Close(CloseHow::Now)));
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
}

#[tokio::test(start_paused = true)]
async fn a_sqlite_stop_of_an_overdue_chunk_interrupts_once_after_a_second() {
    let slow = FakeQuery {
        chunk_delay: secs(60),
        ..select("UPDATE", 1_000_000)
    };
    let fake = FakeDriver::new(DbmsKind::Sqlite)
        .tracks_transactions()
        .queries(vec![slow]);
    let (h, f) = session(fake).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let t = h.run(spec_with(
        &o,
        &[("UPDATE t SET a = a + 1 RETURNING a", false)],
        FetchPolicy::default(),
    ));
    settle_until("the first chunk", || o.rows() == 500).await;
    let asked = tokio::time::Instant::now();
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(asked.elapsed(), secs(1));
    assert_eq!(f.cancel_calls(), 1);
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(f.cancel_calls(), 1, "exactly one interrupt");
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert_eq!(o.rows(), 500);
}

#[tokio::test(start_paused = true)]
async fn a_pg_stop_inside_a_user_transaction_waits_before_it_cancels() {
    // Fix round 1 (review M3, taken): a unit that cannot suspend is read to its end by the
    // close. In autocommit the protocol cancel goes out at once, beside the close
    // (`never_pending_stream_is_cancelled_within_one_chunk`). Inside a user transaction it
    // goes out only when the close is a second late, so a rest that drains meanwhile leaves
    // the user's transaction alive.
    let endless = FakeQuery {
        never_pending: true,
        ..select("UPDATE", u64::MAX)
    };
    let (h, f) = session(pg().queries(vec![endless])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let t = h.run(spec_with(
        &o,
        &[("UPDATE t SET a = a + 1 RETURNING a", false)],
        FetchPolicy::default(),
    ));
    settle_until("the first chunks", || o.rows() >= 4_096).await;
    let asked = tokio::time::Instant::now();
    let _ = h.cancel(t.seq).await.unwrap();
    // The rest was read at once (the fake's close reads it to the end), so the statement ran
    // to its end: no cancel reached the server, and the transaction is as it was.
    t.reply.await.unwrap().unwrap();
    assert_eq!(asked.elapsed(), secs(0));
    assert_eq!(f.cancel_calls(), 0);
    assert_eq!(typed(&f).last(), Some(&FakeTypedCall::Close(CloseHow::Now)));
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    // The rows the close read past the store are not claimed.
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    assert!(matches!(
        &o.unit_ends()[..],
        [UnitEnd::Succeeded {
            rows: Some(RowsOutcome::Fetched { more: true, .. })
        }]
    ));
    run(&h, "COMMIT").await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn cancel_in_a_user_transaction_marks_it_failed() {
    // A unit that cannot suspend (`UPDATE … RETURNING`) whose first rows are a minute away.
    let slow = FakeQuery {
        first_delay: secs(60),
        ..select("UPDATE", 10)
    };
    let (h, f) = session(pg().queries(vec![slow])).await;
    run(&h, "BEGIN").await.unwrap();
    assert_eq!(snap(&h).tx.owner, TxOwner::User);
    let o = Recorder::new();
    let t = h.run(spec_with(
        &o,
        &[("UPDATE t SET a = 1 RETURNING *", false)],
        FetchPolicy::default(),
    ));
    settle_until("the unit started", || !o.events().is_empty()).await;
    let asked = tokio::time::Instant::now();
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(asked.elapsed(), secs(1));
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    // As in psql: the server cancel fails the user's transaction, and nothing hides it.
    assert_eq!(f.cancel_calls(), 1);
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    assert_eq!(snap(&h).tx.owner, TxOwner::User);
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::InTx,
        to: TxStatus::Failed,
    }));
    // No savepoint was put around the unit (P16).
    assert_eq!(statements(&f), ["BEGIN"]);
    // Only a rollback is accepted now: a commit is refused before it is sent, as text and
    // as the transaction command (the server would answer it with a rollback).
    assert_eq!(
        run(&h, "COMMIT").await.unwrap_err().kind,
        ExecErrorKind::Tx(TxError::FailedNeedsRollback)
    );
    assert_eq!(
        ask(&h, Request::Tx(TxCommand::Commit))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::FailedNeedsRollback)
    );
    assert_eq!(statements(&f), ["BEGIN"]);
    ask(&h, Request::Tx(TxCommand::Rollback)).await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(statements(&f), ["BEGIN", "ROLLBACK"]);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_stops_the_rest_of_the_run() {
    let slow = FakeQuery {
        first_delay: secs(60),
        ..select("SELECT 1", 10)
    };
    let (h, f) = session(pg().queries(vec![slow])).await;
    let o = Recorder::new();
    let t = h.run(spec_of(&o, &["SELECT 1", "SELECT 2", "SELECT 3"]));
    settle_until("the unit started", || !o.events().is_empty()).await;
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(
        o.unit_ends(),
        vec![UnitEnd::Cancelled, UnitEnd::Cancelled, UnitEnd::Cancelled]
    );
    let summary = o.summaries()[0];
    assert_eq!(
        (summary.executed, summary.failed, summary.skipped),
        (0, 0, 2)
    );
    assert!(summary.aborted);
    assert_eq!(
        typed(&f)
            .iter()
            .filter(|c| matches!(c, FakeTypedCall::Query(_)))
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn a_stale_cancel_never_stops_a_later_run_or_fetch() {
    let (h, f) = session(pg().queries(vec![
        select("SELECT 1", 3),
        select("SELECT * FROM big", 1_000_000),
    ]))
    .await;
    let o = Recorder::new();
    let first = h.run(spec_of(&o, &["SELECT 1"]));
    first.reply.await.unwrap().unwrap();
    let second = h.run(spec_of(&o, &["SELECT * FROM big"]));
    second.reply.await.unwrap().unwrap();
    // The first run ended; the second ended too and left its cursor open: neither number
    // names anything that runs.
    for seq in [first.seq, second.seq] {
        assert_eq!(h.cancel(seq).await.unwrap(), CancelOutcome::AlreadyFinished);
    }
    assert!(matches!(
        o.snap(1).completeness(),
        Completeness::Suspended(_)
    ));
    // A fetch has its own number: the run's does not stop it.
    let result = o.store(1).id();
    let fetch = h.submit(Request::FetchMore { result, rows: 100 });
    assert_eq!(
        h.cancel(second.seq).await.unwrap(),
        CancelOutcome::AlreadyFinished
    );
    fetch.reply.await.unwrap().unwrap();
    assert_eq!(o.snap(1).row_count(), 10_100);
    assert_eq!(f.cancel_calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_deactivated_run_still_ends_for_its_observer() {
    let stuck = FakeQuery {
        chunk_delay: secs(3_600),
        ignores_cancel: true,
        ..select("SELECT", 1_000_000)
    };
    let (h, _f) = session(pg().queries(vec![stuck])).await;
    let o = Recorder::new();
    let t = h.run(spec_of(&o, &["SELECT * FROM big", "SELECT 2"]));
    settle_until("the first chunk", || o.rows() == 500).await;
    assert_eq!(
        h.escalate(t.seq, Escalation::Deactivate).await.unwrap(),
        CancelOutcome::Cancelled
    );
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    // The request was dropped with its connection: the result is closed, the units and the
    // run are reported ended.
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Closed {
            reason: CloseReason::Cancelled
        }
    );
    assert_eq!(o.unit_ends(), vec![UnitEnd::Cancelled, UnitEnd::Skipped]);
    assert_eq!(
        o.result_ends(),
        vec![Completeness::Closed {
            reason: CloseReason::Cancelled
        }]
    );
    assert_eq!(o.summaries().len(), 1);
    assert!(o.summaries()[0].aborted);
    // In that order: the result, its unit, the rest, the run.
    let events = o.events();
    assert!(matches!(
        &events[events.len() - 4..],
        [
            RunEvent::ResultEnded { .. },
            RunEvent::UnitEnded { .. },
            RunEvent::UnitEnded { .. },
            RunEvent::RunEnded(_),
        ]
    ));
    assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::Cancelled));
}

// ---------------------------------------------------------------------------------------------
// Units and what they report

#[tokio::test(start_paused = true)]
async fn stop_on_error_skips_the_rest() {
    let failing = FakeQuery {
        prefix: "SELECT 1 / 0",
        sets: vec![generated(10)],
        fail_after: Some((0, db("22012"))),
        ..Default::default()
    };
    let (h, f) = session(pg().queries(vec![select("SELECT 1 AS a", 1), failing])).await;
    let o = Recorder::new();
    let reply = run_spec(
        &h,
        spec_of(&o, &["SELECT 1 AS a", "SELECT 1 / 0", "SELECT 3"]),
    )
    .await;
    let e = reply.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert_eq!(
        e.server.as_ref().and_then(|s| s.sqlstate.as_deref()),
        Some("22012")
    );
    let ends = o.unit_ends();
    assert_eq!(ends.len(), 3);
    assert_eq!(ends[0], fetched(1, false));
    assert!(matches!(&ends[1], UnitEnd::Failed(e) if e.kind == ExecErrorKind::Db));
    assert_eq!(ends[2], UnitEnd::Skipped);
    assert_eq!(
        o.summaries(),
        vec![RunSummary {
            executed: 1,
            total: 3,
            failed: 1,
            skipped: 1,
            aborted: true,
            duration_ms: 0,
        }]
    );
    // The third unit never reached the server, and the failed one left nothing open:
    // Hacivat's transaction rolled back, so the session is usable.
    assert!(!typed(&f).contains(&FakeTypedCall::Query("SELECT 3".into())));
    assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT", "BEGIN", "ROLLBACK"]);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    run(&h, "SELECT 4").await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn dml_reports_its_update_count_and_opens_no_result() {
    let insert = FakeQuery {
        prefix: "INSERT",
        rows_affected: Some(3),
        ..Default::default()
    };
    let (h, f) = session(pg().queries(vec![insert])).await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(
            &o,
            &[("INSERT INTO t VALUES (1), (2), (3)", false)],
            FetchPolicy::default(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(o.stores(), 0);
    let unit = spec_of(&o, &["x"]).units[0].id;
    assert!(o.events().contains(&RunEvent::UpdateCount {
        unit,
        rows: Some(3),
        command_tag: None,
    }));
    assert_eq!(
        o.unit_ends(),
        vec![UnitEnd::Succeeded {
            rows: Some(RowsOutcome::Affected(3))
        }]
    );
    assert!(f.executed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn later_result_sets_are_counted_and_noted() {
    let call = FakeQuery {
        prefix: "CALL",
        sets: vec![generated(2), generated(5), generated(7)],
        ..Default::default()
    };
    let (h, _f) = session(fake_mysql().queries(vec![call])).await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("CALL p()", false)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    assert_eq!(o.stores(), 1);
    assert_eq!(o.rows(), 2);
    assert_eq!(
        *o.snap(0).completeness(),
        Completeness::Truncated(TruncReason::FollowingResultSets)
    );
    let unit = spec_of(&o, &["x"]).units[0].id;
    assert!(
        o.events()
            .contains(&RunEvent::MoreResultSets { unit, skipped: 2 })
    );
}

#[tokio::test(start_paused = true)]
async fn rows_fetched_is_reported_at_most_every_50_ms() {
    // 20 chunks, 10 ms apart: about one report per five chunks, never one per chunk.
    let slow = FakeQuery {
        chunk_delay: ms(10),
        ..select("TABLE", 500 + 19 * 4_096)
    };
    let (h, _f) = session(pg().queries(vec![slow])).await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("TABLE big", false)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    let reports: Vec<u64> = o
        .events()
        .iter()
        .filter_map(|e| match e {
            RunEvent::RowsFetched { total, .. } => Some(*total),
            _ => None,
        })
        .collect();
    assert_eq!(reports.first(), Some(&500));
    assert!(reports.len() >= 3 && reports.len() <= 6, "{reports:?}");
    assert!(reports.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(o.rows(), 500 + 19 * 4_096);
}

#[tokio::test(start_paused = true)]
async fn a_run_reports_the_users_transaction_changes() {
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..Default::default()
    };
    let commit = FakeQuery {
        prefix: "COMMIT",
        tx_after: Some(TxStatus::Idle),
        ..Default::default()
    };
    let (h, f) = session(pg().queries(vec![begin, commit])).await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(
            &o,
            &[("BEGIN", false), ("INSERT INTO t VALUES (1)", false)],
            FetchPolicy::default(),
        ),
    )
    .await
    .unwrap();
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::Idle,
        to: TxStatus::InTx,
    }));
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    // The `BEGIN` is transaction control, the `INSERT` a statement of the transaction.
    assert_eq!(tx.statements, 1);
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("COMMIT", false)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::InTx,
        to: TxStatus::Idle,
    }));
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::Idle, TxOwner::None));
    // The user's own transaction: Hacivat added none.
    assert!(statements(&f).is_empty(), "{:?}", statements(&f));
}

#[tokio::test(start_paused = true)]
async fn a_failed_user_transaction_refuses_units_before_they_are_sent() {
    let (h, f) = session(pg().exec(vec![tx_err("SELECT boom", "22012", TxStatus::Failed)])).await;
    run(&h, "BEGIN").await.unwrap();
    run(&h, "SELECT boom").await.unwrap_err();
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    let o = Recorder::new();
    let e = run_spec(&h, spec_of(&o, &["SELECT 1"])).await.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Tx(TxError::FailedNeedsRollback));
    assert!(typed(&f).is_empty());
    assert_eq!(o.summaries()[0].failed, 1);
}

#[tokio::test(start_paused = true)]
async fn a_read_only_run_fails_closed_until_the_typed_guard_lands() {
    // Task 22 fills `readonly::guarded_query`; until then a read-only session runs nothing
    // typed, and says so to the observer.
    let (h, f) = ro_session(pg().queries(vec![select("SELECT", 1)]), ro_profile()).await;
    // The connect's own role probe is behind us.
    let sent = f.call_log().len();
    let o = Recorder::new();
    let e = run_spec(&h, spec_of(&o, &["SELECT 1"])).await.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Internal);
    assert_eq!(f.call_log().len(), sent, "{:?}", f.call_log());
    assert!(
        matches!(&o.unit_ends()[..], [UnitEnd::Failed(e)] if e.kind == ExecErrorKind::Internal)
    );
    assert_eq!(o.summaries().len(), 1);
    assert_eq!(snap(&h).state, SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn an_empty_run_ends_at_once() {
    let (h, f) = session(pg()).await;
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &[])).await.unwrap();
    assert_eq!(o.events(), vec![RunEvent::RunEnded(RunSummary::default())]);
    assert!(f.call_log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_run_queued_behind_a_cancel_never_starts() {
    let (h, f) = session(pg().exec(vec![slow("select pg_sleep(60)", secs(60))])).await;
    let a = h.submit(exec("select pg_sleep(60)"));
    let o = Recorder::new();
    let b = h.run(spec_of(&o, &["SELECT 1"]));
    settle().await;
    assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        b.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    // It never ran, so the observer heard nothing: a reply without `RunEnded`.
    assert!(o.events().is_empty());
    assert!(
        !f.call_log()
            .contains(&FakeCall::Typed(FakeTypedCall::Query("SELECT 1".into())))
    );
}
