//! M2b Task 7: the cancel ladder (HV-SESS-05, B23, Q18, Q37).

mod common;

use std::sync::Arc;

use common::*;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_session::session::{
    CancelOutcome, CancelState, Escalation, ExecErrorKind, SessionEvent, SessionState,
};

#[tokio::test]
async fn cancel_resolves_running_and_drops_queue() {
    let (h, f) = session(fake_pg().exec(vec![slow("select pg_sleep(60)", secs(60))])).await;
    let a = h.submit(exec("select pg_sleep(60)"));
    let b = h.submit(exec("select 2"));
    settle().await;
    assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        a.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(
        b.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(f.cancel_calls(), 1);
    assert_eq!(snap(&h).state, SessionState::Idle);
    assert_eq!(snap(&h).cancel, CancelState::Idle);
    assert!(!f.executed().contains(&"select 2".to_string()));
}

#[tokio::test]
async fn a_cancel_sent_right_after_submit_still_cancels() {
    // No settle: the actor starts the request before it reads the cancel (commands are FIFO).
    let (h, f) = session(fake_pg().exec(vec![slow("select pg_sleep(60)", secs(60))])).await;
    let a = h.submit(exec("select pg_sleep(60)"));
    assert_eq!(h.cancel(a.seq).await.unwrap(), CancelOutcome::Cancelled);
    assert_eq!(
        a.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(f.cancel_calls(), 1);
}

#[tokio::test]
async fn stale_cancel_never_hits_the_next_request() {
    let (h, f) = session(fake_pg()).await;
    let a = h.submit(exec("select 1"));
    a.reply.await.unwrap().unwrap();
    assert_eq!(
        h.cancel(a.seq).await.unwrap(),
        CancelOutcome::AlreadyFinished
    );
    assert_eq!(f.cancel_calls(), 0);
}

#[tokio::test]
async fn stale_cancel_with_old_seq_does_not_cancel_the_next_request() {
    // Q18
    let (h, f) = session(fake_pg().exec(vec![slow("select 2", ms(200))])).await;
    let a = h.submit(exec("select 1"));
    let b = h.submit(exec("select 2"));
    a.reply.await.unwrap().unwrap();
    assert_eq!(
        h.cancel(a.seq).await.unwrap(),
        CancelOutcome::AlreadyFinished
    );
    assert!(b.reply.await.unwrap().is_ok());
    assert_eq!(f.cancel_calls(), 0);
}

#[tokio::test(start_paused = true)]
async fn cancel_waits_two_seconds_then_still_running_and_slow_at_ten() {
    let (h, _) = session(
        fake_pg()
            .ignores_cancel()
            .exec(vec![slow("select x", secs(60))]),
    )
    .await;
    let a = h.submit(exec("select x"));
    let mut ev = h.events();
    settle().await;
    let c = h.cancel(a.seq);
    // The actor reads the cancel at t = 0, so its 2 s wait is pinned (review M7).
    settle().await;
    tokio::time::advance(secs(2)).await;
    assert_eq!(c.await.unwrap(), CancelOutcome::StillRunning);
    assert!(matches!(
        snap(&h).state,
        SessionState::Cancelling { req, .. } if req == a.seq
    ));
    tokio::time::advance(secs(8)).await;
    assert!(matches!(
        next(&mut ev).await,
        SessionEvent::SlowCancel { .. }
    ));
    assert!(matches!(snap(&h).cancel, CancelState::SlowCancel { .. }));
}

#[tokio::test]
async fn mysql_cancel_uses_kill_query_on_a_side_connection() {
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql));
    let (h, _) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select sleep(60)", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select sleep(60)"));
    settle().await;
    let _c = h.cancel(a.seq);
    eventually("the kill on the side connection", || {
        side.executed() == ["KILL QUERY 42"]
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn mysql_side_connection_is_prewarmed_at_500ms() {
    // Q37
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql));
    let (h, _) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select sleep(60)", secs(60))]),
        &side,
    )
    .await;
    let _a = h.submit(exec("select sleep(60)"));
    // The request starts now (the clock only moves by `advance`).
    settle().await;
    tokio::time::advance(ms(400)).await;
    settle().await;
    assert_eq!(side.connect_calls(), 0);
    tokio::time::advance(ms(200)).await;
    settle_until("the side connection to be pre-opened", || {
        side.connect_calls() == 1
    })
    .await;
    assert!(side.executed().is_empty(), "pre-opened, nothing run on it");
}

#[tokio::test]
async fn terminate_escalation_uses_a_side_connection() {
    let side = Arc::new(FakeDriver::new(DbmsKind::Postgres));
    let (h, _) = session_with_side(
        fake_pg()
            .backend(7)
            .ignores_cancel()
            .exec(vec![slow("select x", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select x"));
    settle().await;
    let _e = h.escalate(a.seq, Escalation::Terminate);
    eventually("the terminate on the side connection", || {
        side.executed() == ["SELECT pg_terminate_backend(7)"]
    })
    .await;
}

#[tokio::test]
async fn mysql_terminate_kills_the_connection() {
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql));
    let (h, _) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select sleep(60)", secs(60))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select sleep(60)"));
    settle().await;
    let _e = h.escalate(a.seq, Escalation::Terminate);
    eventually("the kill on the side connection", || {
        side.executed() == ["KILL CONNECTION 42"]
    })
    .await;
}

#[tokio::test]
async fn deactivate_closes_and_breaks_the_session() {
    let (h, f) = session(
        fake_pg()
            .ignores_cancel()
            .exec(vec![slow("select x", secs(60))]),
    )
    .await;
    let a = h.submit(exec("select x"));
    let b = h.submit(exec("select 2"));
    settle().await;
    assert_eq!(
        h.escalate(a.seq, Escalation::Deactivate).await.unwrap(),
        CancelOutcome::Cancelled
    );
    assert_eq!(
        a.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(
        b.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::Cancelled));
    assert_eq!(f.open_connections(), 0);
}

#[tokio::test]
async fn escalating_a_request_that_is_not_running_is_already_finished() {
    let (h, f) = session(fake_pg()).await;
    let a = h.submit(exec("select 1"));
    a.reply.await.unwrap().unwrap();
    assert_eq!(
        h.escalate(a.seq, Escalation::Deactivate).await.unwrap(),
        CancelOutcome::AlreadyFinished
    );
    assert_eq!(snap(&h).state, SessionState::Idle);
    assert_eq!(f.open_connections(), 1);
}

// ## m2b-t07 fix round 1

/// Review C1: a kill still in flight when its request ends on its own is aborted after the
/// wait, and the session breaks, so the kill can never reach the next request.
#[tokio::test(start_paused = true)]
async fn a_late_kill_never_reaches_the_next_request() {
    let side = Arc::new(FakeDriver::new(DbmsKind::MySql).gated());
    let (h, f) = session_with_side(
        fake_mysql()
            .backend(42)
            .exec(vec![slow("select a", ms(300))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select a"));
    settle().await;
    let _c = h.cancel(a.seq);
    settle_until("the kill to wait at the side connection's gate", || {
        side.connect_calls() == 1
    })
    .await;
    let b = h.submit(exec("select b"));
    assert!(a.reply.await.unwrap().is_ok(), "a ended on its own");
    assert_eq!(
        b.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::NotConnected
    );
    assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::Cancelled));
    assert_eq!(f.open_connections(), 0);
    // The side connection may now open: the aborted kill is gone.
    side.release();
    settle().await;
    assert!(side.executed().is_empty(), "{:?}", side.executed());
    assert!(!f.executed().contains(&"select b".to_string()));
}

/// Review M1: a terminate that ends the request is a success.
#[tokio::test(start_paused = true)]
async fn a_terminate_that_ends_the_request_reports_cancelled() {
    let side = Arc::new(FakeDriver::new(DbmsKind::Postgres));
    let (h, f) = session_with_side(
        fake_pg().backend(7).exec(vec![slow("select x", secs(1))]),
        &side,
    )
    .await;
    let a = h.submit(exec("select x"));
    settle().await;
    let e = h.escalate(a.seq, Escalation::Terminate);
    settle_until("the terminate on the side connection", || {
        side.executed() == ["SELECT pg_terminate_backend(7)"]
    })
    .await;
    // What the terminate does to the server connection.
    f.kill_connections();
    assert_eq!(e.await.unwrap(), CancelOutcome::Cancelled);
    assert!(matches!(
        a.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::ConnectionLost { .. }
    ));
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}
