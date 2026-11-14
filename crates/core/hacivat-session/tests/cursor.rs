//! M3a Task 18: console cursors (HV-RES-05): fetching more and all, coalescing, the release
//! of a cursor before anything else runs, Hacivat's own transaction around a PostgreSQL cursor
//! (R-M3-23), the idle bounds (R-M3-20), memory pauses (R-M3-33), a lost connection and
//! liveness. FakeDriver and a paused clock: no socket, no real time.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::runs::*;
use common::*;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeQuery, FakeTypedCall};
use hacivat_driver::{CloseHow, TxStatus, TypedCaps};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_results::{
    CloseReason, Completeness, FetchControl, FetchPolicy, PauseReason, ResultId,
};
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::cursor::{PRODUCTION_IDLE, READ_ONLY_PG_IDLE, idle_bound};
use hacivat_session::session::{
    CancelOutcome, ClosePolicy, ExecErrorKind, Request, SessionEvent, SessionHandle, SessionState,
    TxCommand, TxOwner,
};

/// A PostgreSQL fake that follows `BEGIN`/`COMMIT`/`ROLLBACK` (a suspendable PostgreSQL query
/// needs a transaction block, P5).
fn pg() -> FakeDriver {
    fake_pg().tracks_transactions()
}

/// The statements sent through `execute`/`batch`, without the session's schema probe.
fn statements(f: &FakeDriver) -> Vec<String> {
    f.executed()
        .into_iter()
        .filter(|s| !s.starts_with("SELECT current_database()"))
        .collect()
}

const BIG: &str = "SELECT * FROM big";

/// Runs `BIG` on `h` with `fetch` and expects it suspended at its read-ahead.
async fn suspend_on(h: &SessionHandle, fetch: FetchPolicy) -> Arc<Recorder> {
    let o = Recorder::new();
    run_spec(h, spec_with(&o, &[(BIG, true)], fetch))
        .await
        .expect("the run succeeds");
    assert!(
        matches!(o.snap(0).completeness(), Completeness::Suspended(_)),
        "{:?}",
        o.snap(0).completeness()
    );
    o
}

/// A session over `fake` holding a suspended cursor on a result of `rows` rows.
async fn suspended(fake: FakeDriver, rows: u64) -> (SessionHandle, Arc<FakeDriver>, Arc<Recorder>) {
    let (h, f) = session(fake.queries(vec![select("SELECT * FROM big", rows)])).await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    (h, f, o)
}

async fn suspended_pg(rows: u64) -> (SessionHandle, Arc<FakeDriver>, Arc<Recorder>) {
    suspended(pg(), rows).await
}

fn fetch_more(o: &Recorder, rows: u64) -> Request {
    Request::FetchMore {
        result: o.store(0).id(),
        rows,
    }
}

fn fetch_all(o: &Recorder) -> Request {
    Request::FetchAll {
        result: o.store(0).id(),
    }
}

fn close_cursor(o: &Recorder, reason: CloseReason) -> Request {
    Request::CloseCursor {
        result: o.store(0).id(),
        reason,
    }
}

fn drain() -> FakeTypedCall {
    FakeTypedCall::Close(CloseHow::Drain(ms(250)))
}

fn closed(reason: CloseReason) -> Completeness {
    Completeness::Closed { reason }
}

// ---------------------------------------------------------------------------------------------
// Fetching

#[tokio::test(start_paused = true)]
async fn queued_fetch_mores_coalesce() {
    let (h, f, o) = suspended_pg(1_000_000).await;
    let calls = f.typed_log().len();
    // Two asks sent back to back are one fetch of the sum.
    let a = h.submit(fetch_more(&o, 100));
    let b = h.submit(fetch_more(&o, 200));
    a.reply.await.unwrap().unwrap();
    b.reply.await.unwrap().unwrap();
    assert_eq!(o.rows(), 10_300);
    assert_eq!(f.typed_log()[calls..], [FakeTypedCall::FetchMore(300)]);
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));

    // The same for asks that queue up behind a fetch in flight; a `FetchAll` among them wins.
    let (h, f) = session(pg().queries(vec![FakeQuery {
        chunk_delay: secs(1),
        ..select("SELECT * FROM big", 30_000)
    }]))
    .await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let calls = f.typed_log().len();
    let first = h.submit(fetch_more(&o, 10));
    settle_until("the fetch is in flight", || f.typed_log().len() > calls).await;
    let queued = [h.submit(fetch_more(&o, 20)), h.submit(fetch_more(&o, 30))];
    first.reply.await.unwrap().unwrap();
    for t in queued {
        t.reply.await.unwrap().unwrap();
    }
    assert_eq!(o.rows(), 10_060);
    assert_eq!(
        f.typed_log()[calls..],
        [FakeTypedCall::FetchMore(10), FakeTypedCall::FetchMore(50)]
    );
}

#[tokio::test(start_paused = true)]
async fn the_result_streams_while_a_fetch_is_served_and_suspends_again() {
    let (h, f) = session(pg().queries(vec![FakeQuery {
        chunk_delay: secs(1),
        ..select("SELECT * FROM big", 1_000_000)
    }]))
    .await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let Completeness::Suspended(before) = *o.snap(0).completeness() else {
        unreachable!()
    };
    let version = o.snap(0).version();
    let t = h.submit(fetch_more(&o, 5_000));
    settle_until("the fetch started", || {
        *o.snap(0).completeness() == Completeness::Streaming
    })
    .await;
    // A cursor that is being fetched is not asked again (`has_cursor` is false meanwhile).
    assert!(matches!(snap(&h).state, SessionState::Busy { .. }));
    t.reply.await.unwrap().unwrap();
    assert_eq!(o.rows(), 15_000);
    let Completeness::Suspended(after) = *o.snap(0).completeness() else {
        panic!("{:?}", o.snap(0).completeness())
    };
    assert_eq!(after.opened_at, before.opened_at);
    assert!(after.last_fetch > before.last_fetch);
    assert!(o.snap(0).version() > version);
    // Batches never exceed a chunk, whatever was asked for.
    assert_eq!(
        f.typed_log()[f.typed_log().len() - 2..],
        [
            FakeTypedCall::FetchMore(4_096),
            FakeTypedCall::FetchMore(904)
        ]
    );
    assert_eq!(snap(&h).cursor.map(|c| c.rows), Some(15_000));
}

#[tokio::test(start_paused = true)]
async fn a_late_fetch_is_a_no_op() {
    // After completion.
    let (h, f, o) = suspended_pg(12_000).await;
    ask(&h, fetch_all(&o)).await.unwrap();
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    let (calls, version) = (f.call_log().len(), o.snap(0).version());
    ask(&h, fetch_more(&o, 100)).await.unwrap();
    ask(&h, fetch_all(&o)).await.unwrap();
    ask(&h, close_cursor(&o, CloseReason::UserClosed))
        .await
        .unwrap();
    assert_eq!(f.call_log().len(), calls);
    assert_eq!(o.snap(0).version(), version);

    // For a result that is not the open cursor's: the cursor is untouched.
    let (h, f, o) = suspended_pg(1_000_000).await;
    let calls = f.call_log().len();
    let other = ResultId(u64::MAX);
    for request in [
        Request::FetchMore {
            result: other,
            rows: 10,
        },
        Request::FetchAll { result: other },
        Request::CloseCursor {
            result: other,
            reason: CloseReason::TabClosed,
        },
    ] {
        ask(&h, request).await.unwrap();
    }
    assert_eq!(f.call_log().len(), calls);
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert!(o.releases().is_empty());
}

#[tokio::test(start_paused = true)]
async fn completed_cursor_commits() {
    let (h, f, o) = suspended_pg(12_000).await;
    assert_eq!(snap(&h).tx.owner, TxOwner::Cursor);
    assert_eq!(snap(&h).cursor.map(|c| c.holds_tx), Some(true));
    ask(&h, fetch_all(&o)).await.unwrap();
    assert_eq!(o.rows(), 12_000);
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.result_ends(), vec![Completeness::Complete]);
    assert!(o.releases().is_empty());
    assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT"]);
    assert_eq!(snap(&h).cursor, None);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).tx.owner, TxOwner::None);
}

#[tokio::test(start_paused = true)]
async fn fetch_all_pauses_at_the_memory_limit_and_resumes_after_the_raise() {
    let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 100_000)])).await;
    // 1 MiB per result: the read-ahead (10,000 rows) fits, 100,000 rows do not.
    let o = Recorder::with_budget(1 << 30, 1 << 20);
    run_spec(&h, spec_of(&o, &[BIG])).await.unwrap();
    assert_eq!(o.rows(), 10_000);
    assert_eq!(o.snap(0).paused(), None);
    ask(&h, fetch_all(&o)).await.unwrap();
    let rows = o.rows();
    assert!(rows > 10_000 && rows < 100_000, "{rows}");
    // Still a cursor, and the snapshot says why it stopped.
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(o.snap(0).paused(), Some(PauseReason::ResultCap));
    // A refused ask still publishes, so the source can ask again.
    let version = o.snap(0).version();
    let calls = f.typed_log().len();
    ask(&h, fetch_more(&o, 500)).await.unwrap();
    assert!(o.snap(0).version() > version);
    assert_eq!(f.typed_log().len(), calls);
    assert_eq!(o.rows(), rows);
    // "Fetch more anyway": the cap rises, the next `FetchAll` runs to the end.
    o.store(0).raise_cap_to_global();
    ask(&h, fetch_all(&o)).await.unwrap();
    assert_eq!(o.rows(), 100_000);
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.snap(0).paused(), None);
    assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT"]);
}

#[tokio::test(start_paused = true)]
async fn the_fetch_control_reaches_the_cursor() {
    let (h, _f, o) = suspended_pg(1_000_000).await;
    let result = o.store(0).id();
    h.fetch_more(result, 100);
    settle_until("the fetch", || o.rows() == 10_100).await;
    FetchControl::close(&h, result, CloseReason::UserClosed);
    settle_until("the close", || {
        *o.snap(0).completeness() == closed(CloseReason::UserClosed)
    })
    .await;
    assert_eq!(o.releases(), vec![(CloseReason::UserClosed, true)]);
}

// ---------------------------------------------------------------------------------------------
// Release

#[tokio::test(start_paused = true)]
async fn another_statement_releases_the_cursor_first() {
    let (h, f, o) = suspended_pg(1_000_000).await;
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    assert!(f.typed_log().ends_with(&[drain()]));
    // In this order: the query closed, Hacivat's transaction ended, then the statement.
    let log = f.call_log();
    assert_eq!(
        log[log.len() - 3..],
        [
            FakeCall::Typed(drain()),
            FakeCall::Statement("ROLLBACK".into()),
            FakeCall::Statement("SELECT 1".into()),
        ]
    );
    // The rows stay and say why the cursor closed.
    assert_eq!(o.rows(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::SessionNeeded)
    );
    assert_eq!(o.result_ends(), vec![closed(CloseReason::SessionNeeded)]);
    assert_eq!(snap(&h).cursor, None);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

#[tokio::test(start_paused = true)]
async fn every_request_kind_releases_the_cursor() {
    for request in [
        Request::Ping,
        Request::Reset,
        Request::Tx(TxCommand::Begin),
        Request::SwitchSchema("public".into()),
    ] {
        let (h, f, o) = suspended(
            pg().exec(vec![rows("SELECT current_user = session_user", &[&["t"]])]),
            1_000_000,
        )
        .await;
        ask(&h, request.clone()).await.unwrap();
        assert_eq!(
            o.releases(),
            vec![(CloseReason::SessionNeeded, true)],
            "{request:?}"
        );
        let log = f.call_log();
        let closed_at = log
            .iter()
            .position(|c| *c == FakeCall::Typed(drain()))
            .expect("the cursor was closed");
        assert_eq!(
            log[closed_at + 1],
            FakeCall::Statement("ROLLBACK".into()),
            "{request:?}"
        );
        assert_eq!(snap(&h).cursor, None);
    }
    // A new run releases the cursor of the one before it.
    let (h, f) = session(pg().queries(vec![
        select("SELECT * FROM big", 1_000_000),
        select("SELECT 2", 2),
    ]))
    .await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let next = Recorder::new();
    run_spec(&h, spec_of(&next, &["SELECT 2"])).await.unwrap();
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK", "BEGIN", "COMMIT"]);
}

#[tokio::test(start_paused = true)]
async fn incomplete_cursor_rolls_back_hacivats_own_transaction() {
    let (h, f, o) = suspended_pg(1_000_000).await;
    ask(&h, close_cursor(&o, CloseReason::TabClosed))
        .await
        .unwrap();
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    assert_eq!(o.releases(), vec![(CloseReason::TabClosed, true)]);
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::TabClosed));
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).cursor, None);
}

#[tokio::test(start_paused = true)]
async fn a_release_whose_drain_reads_the_rest_commits_but_claims_no_rows_it_dropped() {
    // 2,000 rows are left: the release's one more `Execute` reads them to the end, so the
    // statement completed (as in autocommit), but those rows never reached the store.
    let (h, f, o) = suspended_pg(12_000).await;
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(&f.executed()[..], ["BEGIN", "COMMIT", "SELECT 1"]);
    assert_eq!(o.rows(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::SessionNeeded)
    );
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, false)]);
}

#[tokio::test(start_paused = true)]
async fn cursor_inside_a_user_transaction_uses_a_savepoint() {
    let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = suspend_on(&h, FetchPolicy::default()).await;
    assert_eq!(statements(&f), ["BEGIN", "SAVEPOINT hv_cursor"]);
    // The transaction is the user's all along.
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    assert_eq!(snap(&h).cursor.map(|c| c.holds_tx), Some(true));
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(
        statements(&f),
        [
            "BEGIN",
            "SAVEPOINT hv_cursor",
            "ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor",
            "SELECT 1",
        ]
    );
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));

    // A cursor that completes releases its savepoint and leaves the transaction open.
    let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 12_000)])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = suspend_on(&h, FetchPolicy::default()).await;
    ask(&h, fetch_all(&o)).await.unwrap();
    assert_eq!(
        statements(&f),
        [
            "BEGIN",
            "SAVEPOINT hv_cursor",
            "RELEASE SAVEPOINT hv_cursor"
        ]
    );
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
}

#[tokio::test(start_paused = true)]
async fn the_end_of_the_users_transaction_releases_its_cursor() {
    for end in [TxCommand::Commit, TxCommand::Rollback] {
        let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
        run(&h, "BEGIN").await.unwrap();
        let o = suspend_on(&h, FetchPolicy::default()).await;
        ask(&h, Request::Tx(end)).await.unwrap();
        assert_eq!(o.releases(), vec![(CloseReason::TxEnded, true)], "{end:?}");
        assert_eq!(*o.snap(0).completeness(), closed(CloseReason::TxEnded));
        // The cursor went before the transaction did.
        let sent = statements(&f);
        assert_eq!(
            sent[2],
            "ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor"
        );
        assert_eq!(sent.len(), 4, "{sent:?}");
        assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    }
    // The same through the user's own text; a `ROLLBACK TO` is not the transaction's end.
    let (h, _f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
    run(&h, "BEGIN").await.unwrap();
    run(&h, "SAVEPOINT mine").await.unwrap();
    let o = suspend_on(&h, FetchPolicy::default()).await;
    run(&h, "ROLLBACK TO SAVEPOINT mine").await.unwrap();
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, true)]);
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
}

#[tokio::test(start_paused = true)]
async fn a_failed_user_transaction_takes_no_savepoint() {
    // The unit is refused before it is sent (only a rollback is accepted), so no savepoint
    // can be put on a transaction that would refuse it.
    let (h, f) = session(pg().exec(vec![tx_err("SELECT boom", "22012", TxStatus::Failed)])).await;
    run(&h, "BEGIN").await.unwrap();
    run(&h, "SELECT boom").await.unwrap_err();
    let o = Recorder::new();
    run_spec(&h, spec_of(&o, &[BIG])).await.unwrap_err();
    assert!(!f.executed().iter().any(|s| s.contains("SAVEPOINT")));
    assert!(f.typed_log().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_failing_cursor_inside_a_user_transaction_leaves_it_usable() {
    // The read fails at row 600, which fails the transaction block; the rollback to
    // Hacivat's savepoint recovers the user's transaction.
    let failing = FakeQuery {
        fail_after: Some((600, db("22012"))),
        ..select("SELECT * FROM big", 1_000_000)
    };
    let (h, f) = session(pg().queries(vec![failing])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let e = run_spec(&h, spec_of(&o, &[BIG])).await.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert_eq!(o.rows(), 600);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::ServerTerminated)
    );
    assert_eq!(o.releases(), vec![(CloseReason::ServerTerminated, true)]);
    assert_eq!(
        statements(&f)[1..],
        [
            "SAVEPOINT hv_cursor",
            "ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor",
        ]
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    run(&h, "SELECT 1").await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_session_close_releases_the_cursor_and_is_not_refused_for_hacivats_transaction() {
    let (h, f, o) = suspended_pg(1_000_000).await;
    // Hacivat's own transaction is not an open transaction of the user's.
    h.close(ClosePolicy::RefuseIfOpenTx).await.unwrap();
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::TabClosed));
    assert_eq!(o.rows(), 10_000);
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    assert_eq!(snap(&h).state, SessionState::Disconnected);
    assert_eq!(snap(&h).cursor, None);

    // A user's transaction still refuses, and the refusal leaves the cursor as it is.
    let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = suspend_on(&h, FetchPolicy::default()).await;
    h.close(ClosePolicy::RefuseIfOpenTx).await.unwrap_err();
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(statements(&f), ["BEGIN", "SAVEPOINT hv_cursor"]);
    h.close(ClosePolicy::RollBack).await.unwrap();
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::TabClosed));
    assert_eq!(statements(&f).last().map(String::as_str), Some("ROLLBACK"));
}

#[tokio::test(start_paused = true)]
async fn hacivats_own_transaction_is_no_idle_transaction_of_the_users() {
    // A profile that disconnects after 30 idle seconds, with a cursor (and Hacivat's own
    // transaction around it) open for longer than that.
    let profile = Arc::new(Profile {
        idle_disconnect_s: Some(30),
        ..pg_profile(TlsMode::Disable)
    });
    let (m, f) = manager(pg().queries(vec![select("SELECT * FROM big", 1_000_000)]));
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let h = m.sessions(profile.id).unwrap().default_session().unwrap();
    let mut events = h.events();
    let o = suspend_on(&h, FetchPolicy::default()).await;
    assert_eq!(snap(&h).tx.owner, TxOwner::Cursor);
    let mut state = h.state();
    tokio::time::timeout(
        secs(120),
        state.wait_for(|s| s.state == SessionState::Disconnected),
    )
    .await
    .expect("the idle disconnect came")
    .unwrap();
    // No warning about an idle transaction (there is none of the user's), and the disconnect
    // was not held up; the cursor went first, its rows stay.
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(seen.contains(&SessionEvent::IdleDisconnected), "{seen:?}");
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, SessionEvent::IdleTransaction { .. })),
        "{seen:?}"
    );
    assert_eq!(o.releases(), vec![(CloseReason::TabClosed, true)]);
    assert_eq!(o.rows(), 10_000);
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
}

// ---------------------------------------------------------------------------------------------
// MySQL: drain, else kill on a side connection

#[tokio::test(start_paused = true)]
async fn a_mysql_release_kills_on_the_side_connection() {
    let side = Arc::new(fake_mysql());
    let (h, f) = session_with_side(
        fake_mysql().queries(vec![select("SELECT * FROM big", 1_000_000)]),
        &side,
    )
    .await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    // No transaction of Hacivat's on MySQL: the cursor is the unread stream.
    assert!(f.executed().is_empty());
    assert_eq!(snap(&h).cursor.map(|c| c.holds_tx), Some(false));
    run(&h, "SELECT 1").await.unwrap();
    // The drain ran out, the session killed its own thread's statement and read on to the
    // kill's error; then the statement ran on the same connection.
    assert_eq!(&side.executed()[..], ["KILL QUERY 1000"]);
    let typed = f.typed_log();
    assert_eq!(
        typed[typed.len() - 2..],
        [drain(), FakeTypedCall::Close(CloseHow::AfterKill(secs(2))),]
    );
    assert_eq!(&f.executed()[..], ["SELECT 1"]);
    assert_eq!(f.connect_calls(), 1);
    assert_eq!(side.open_connections(), 0, "the side connection is closed");
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, false)]);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::SessionNeeded)
    );
}

#[tokio::test(start_paused = true)]
async fn a_mysql_kill_that_is_ignored_loses_the_connection_and_keeps_the_rows() {
    let side = Arc::new(fake_mysql());
    let stuck = FakeQuery {
        kill_ignored: true,
        ..select("SELECT * FROM big", 1_000_000)
    };
    let (h, f) = session_with_side(fake_mysql().queries(vec![stuck]), &side).await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let e = run(&h, "SELECT 1").await.unwrap_err();
    // The statement never ran: the connection under the cursor could not be freed.
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false
        }
    );
    assert!(f.executed().is_empty());
    assert_eq!(o.rows(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::ConnectionLost)
    );
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_mysql_stop_kills_at_once_and_the_connection_is_reused() {
    let side = Arc::new(fake_mysql());
    let endless = FakeQuery {
        never_pending: true,
        ..select("SELECT * FROM big", u64::MAX)
    };
    let (h, f) = session_with_side(fake_mysql().queries(vec![endless]), &side).await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let t = h.submit(fetch_all(&o));
    settle_until("more rows", || o.rows() >= 20_000).await;
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    t.reply.await.unwrap().unwrap_err();
    assert_eq!(&side.executed()[..], ["KILL QUERY 1000"]);
    let typed = f.typed_log();
    assert_eq!(
        typed[typed.len() - 2..],
        [
            FakeTypedCall::Close(CloseHow::Now),
            FakeTypedCall::Close(CloseHow::AfterKill(secs(2))),
        ]
    );
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(f.connect_calls(), 1);
}

// ---------------------------------------------------------------------------------------------
// Idle bounds (R-M3-20)

fn caps(idle: Option<u64>) -> TypedCaps {
    TypedCaps {
        suspend: true,
        binds: false,
        close_needs_kill: false,
        idle_bound: idle.map(secs),
    }
}

#[tokio::test(start_paused = true)]
async fn every_idle_bound_of_the_table() {
    let d = FetchPolicy::default();
    let capped = FetchPolicy {
        idle_cap: Some(secs(2)),
        ..d
    };
    use DbmsKind::{MariaDb, MySql, Postgres, Sqlite};
    // (engine, read-only, production, the driver's bound, the policy) → the bound.
    let cases: [(DbmsKind, bool, bool, Option<u64>, FetchPolicy, Duration); 14] = [
        (Postgres, false, false, None, d, secs(600)),
        (Postgres, true, false, None, d, secs(20)),
        (Postgres, false, true, None, d, secs(120)),
        (Postgres, true, true, None, d, secs(20)),
        (MySql, false, false, Some(45), d, secs(45)),
        (MariaDb, false, false, Some(45), d, secs(45)),
        (MySql, false, true, Some(285), d, secs(120)),
        (MySql, true, false, Some(45), d, secs(45)),
        (Sqlite, false, false, Some(30), d, secs(30)),
        (Sqlite, false, false, None, d, secs(600)),
        (Sqlite, false, true, None, d, secs(120)),
        (Postgres, false, false, None, capped, secs(2)),
        (MySql, false, false, Some(45), capped, secs(2)),
        (Sqlite, true, true, Some(30), capped, secs(2)),
    ];
    for (kind, read_only, production, engine, policy, want) in cases {
        assert_eq!(
            idle_bound(kind, read_only, production, &caps(engine), &policy),
            want,
            "{kind:?} read_only={read_only} production={production} engine={engine:?}"
        );
    }
    assert_eq!(READ_ONLY_PG_IDLE, secs(20));
    assert_eq!(PRODUCTION_IDLE, secs(120));
}

#[tokio::test(start_paused = true)]
async fn the_idle_bound_releases_the_cursor_and_rolls_back() {
    let (h, f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
    let fetch = FetchPolicy {
        idle_cap: Some(secs(2)),
        ..FetchPolicy::default()
    };
    let start = tokio::time::Instant::now();
    let o = suspend_on(&h, fetch).await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(2));
    assert!(info.holds_tx);
    // Nothing happens before the bound.
    tokio::time::sleep(ms(1_900)).await;
    settle().await;
    assert!(o.releases().is_empty());
    let mut state = h.state();
    tokio::time::timeout(secs(60), state.wait_for(|s| s.cursor.is_none()))
        .await
        .expect("released within the wait")
        .unwrap();
    assert_eq!(start.elapsed(), secs(2));
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, true)]);
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::IdleTimeout));
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    assert!(f.typed_log().ends_with(&[drain()]));
    assert_eq!(snap(&h).state, SessionState::Idle);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    run(&h, "SELECT 1").await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_fetch_starts_the_idle_bound_over() {
    let (h, _f) = session(pg().queries(vec![select("SELECT * FROM big", 1_000_000)])).await;
    let fetch = FetchPolicy {
        idle_cap: Some(secs(10)),
        ..FetchPolicy::default()
    };
    let o = suspend_on(&h, fetch).await;
    tokio::time::sleep(secs(8)).await;
    ask(&h, fetch_more(&o, 100)).await.unwrap();
    tokio::time::sleep(secs(8)).await;
    settle().await;
    // 16 s after the run, 8 s after the fetch: still open.
    assert!(o.releases().is_empty());
    tokio::time::sleep(secs(3)).await;
    settle_until("the idle release", || !o.releases().is_empty()).await;
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, true)]);
}

#[tokio::test(start_paused = true)]
async fn rollback_journal_cursor_uses_the_30s_bound() {
    // The real driver reports 30 s for a rollback-journal database (a live statement holds a
    // SHARED lock, so no other process can commit); the fake's SQLite default has no bound.
    let journal = FakeDriver::new(DbmsKind::Sqlite).with_typed_caps(caps(Some(30)));
    let (h, f, o) = suspended(journal, 1_000_000).await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(30));
    assert!(!info.holds_tx);
    assert!(f.executed().is_empty());
    let start = tokio::time::Instant::now();
    let mut state = h.state();
    tokio::time::timeout(secs(120), state.wait_for(|s| s.cursor.is_none()))
        .await
        .expect("released within the wait")
        .unwrap();
    assert_eq!(start.elapsed(), secs(30));
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, false)]);
    assert!(f.typed_log().ends_with(&[drain()]));

    // In WAL mode the driver sets no bound: the policy's 10 minutes.
    let wal = FakeDriver::new(DbmsKind::Sqlite);
    let (_h, _f, o) = suspended(wal, 1_000_000).await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(600));
}

#[tokio::test(start_paused = true)]
async fn a_production_cursor_is_released_after_120_s() {
    let side = Arc::new(pg());
    let (h, _f) = spawn_over(
        pg().queries(vec![select("SELECT * FROM big", 1_000_000)]),
        Profile {
            // Auto, so the cursor is in Hacivat's own transaction.
            tx_mode: Some(hacivat_session::session::TxMode::Auto),
            ..production_profile()
        },
        false,
        side,
    )
    .await;
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        unreachable!()
    };
    assert_eq!(info.idle_bound, secs(120));
}

// ---------------------------------------------------------------------------------------------
// A lost connection, liveness

#[tokio::test(start_paused = true)]
async fn connection_lost_while_suspended_keeps_rows() {
    // Found by the next fetch.
    let (h, f, o) = suspended_pg(1_000_000).await;
    f.kill_connections();
    let e = ask(&h, fetch_more(&o, 100)).await.unwrap_err();
    assert!(matches!(e.kind, ExecErrorKind::ConnectionLost { .. }));
    assert_eq!(o.rows(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::ConnectionLost)
    );
    assert_eq!(o.releases(), vec![(CloseReason::ConnectionLost, false)]);
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(snap(&h).cursor, None);
    // Hacivat's own transaction died with the connection: nothing of the user's is open.
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);

    // Found by the release for another statement: that statement never runs.
    let (h, f, o) = suspended_pg(1_000_000).await;
    f.kill_connections();
    let e = run(&h, "SELECT 1").await.unwrap_err();
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false
        }
    );
    assert!(!f.executed().contains(&"SELECT 1".to_string()));
    assert_eq!(o.rows(), 10_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::ConnectionLost)
    );

    // Found by the idle bound.
    let (h, f, o) = suspended_pg(1_000_000).await;
    f.kill_connections();
    tokio::time::sleep(secs(601)).await;
    settle_until("the idle release", || {
        *o.snap(0).completeness() == closed(CloseReason::ConnectionLost)
    })
    .await;
    assert_eq!(o.rows(), 10_000);
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}

#[tokio::test(start_paused = true)]
async fn liveness_skips_pings_while_suspended() {
    // A keep-alive every 5 s, through the manager (its sessions are supervised).
    let profile = Arc::new(Profile {
        keepalive_s: Some(5),
        ..pg_profile(TlsMode::Disable)
    });
    let (m, f) = manager(pg().queries(vec![select("SELECT * FROM big", 1_000_000)]));
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let h = m.sessions(profile.id).unwrap().default_session().unwrap();
    // Idle: the keep-alive pings.
    tokio::time::sleep(secs(12)).await;
    settle().await;
    let idle_pings = f.ping_calls();
    assert!(idle_pings >= 2, "{idle_pings} pings while idle");
    let o = suspend_on(&h, FetchPolicy::default()).await;
    let before = f.ping_calls();
    tokio::time::sleep(secs(60)).await;
    settle().await;
    assert_eq!(f.ping_calls(), before, "no ping under a suspended cursor");
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(h.state().borrow().state, SessionState::Idle);
    // Once the cursor is gone the keep-alive is back.
    ask(&h, close_cursor(&o, CloseReason::UserClosed))
        .await
        .unwrap();
    tokio::time::sleep(secs(12)).await;
    settle().await;
    assert!(f.ping_calls() > before);
}
