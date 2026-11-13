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
    let _ = h.cancel(a.seq).await;
    assert_eq!(side.executed(), ["KILL QUERY 42"]);
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
    let _ = h.escalate(a.seq, Escalation::Terminate).await;
    assert_eq!(side.executed(), ["SELECT pg_terminate_backend(7)"]);
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
    let _ = h.escalate(a.seq, Escalation::Terminate).await;
    assert_eq!(side.executed(), ["KILL CONNECTION 42"]);
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
