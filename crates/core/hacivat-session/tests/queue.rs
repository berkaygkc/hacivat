//! M3b Task 11: a session's queue. At most 256 requests wait behind the running one (C9); the
//! next is answered `QueueFull` at once and never runs. A run still queued when a cancel drops
//! the queue tells its observer that its units were cancelled, not skipped (ledger 318).
//! FakeDriver and a paused clock: the running request is held by its scripted delay, which a
//! paused clock never reaches while the test only yields.

mod common;

use common::runs::*;
use common::runs_m3b::*;
use common::*;
use hacivat_driver::fake::FakeQuery;
use hacivat_session::session::{
    CancelOutcome, ExecErrorKind, Request, RunEvent, RunSummary, SessionState, UnitEnd,
};
use tokio::sync::oneshot::error::TryRecvError;

#[tokio::test(start_paused = true)]
async fn queue_over_256_is_queue_full() {
    let (h, f) = session(fake_pg().exec(vec![slow("SELECT pg_sleep", secs(30))])).await;
    let running = h.submit(exec("SELECT pg_sleep(30)"));
    settle_until(
        "the slow statement holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == running.seq),
    )
    .await;
    let mut queued: Vec<_> = (0..256)
        .map(|i| h.submit(exec(&format!("SELECT {i}"))))
        .collect();
    // The 257th is refused at once, while the first still runs.
    let refused = h.submit(exec("SELECT 256"));
    let e = refused.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::QueueFull);
    // Fix round 1 (review m6): a refused run tells its observer why it never ran.
    let o = Recorder::new();
    let refused_run = h.run(ask_spec(&o, &["SELECT 1", "SELECT 2"]));
    assert_eq!(
        refused_run.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::QueueFull
    );
    assert_eq!(
        o.unit_ends(),
        vec![
            UnitEnd::Failed(hacivat_session::session::ExecError::of(
                ExecErrorKind::QueueFull
            )),
            UnitEnd::Skipped,
        ]
    );
    let summary = o.summaries();
    assert_eq!(
        (summary.len(), summary[0].failed, summary[0].skipped),
        (1, 1, 1)
    );
    assert!(matches!(snap(&h).state, SessionState::Busy { req, .. } if req == running.seq));
    for t in &mut queued {
        assert_eq!(t.reply.try_recv(), Err(TryRecvError::Empty), "still queued");
    }
    // A close is no request: it queues behind them.
    let closed = h.close(hacivat_session::session::ClosePolicy::RollBack);
    // Once the slow one ends the queue drains in order; the refused one never ran.
    running.reply.await.unwrap().unwrap();
    for t in queued {
        t.reply.await.unwrap().unwrap();
    }
    closed.await.unwrap();
    let sent = f.executed();
    assert!(sent.contains(&"SELECT 255".to_string()));
    assert!(!sent.contains(&"SELECT 256".to_string()), "never sent");
}

#[tokio::test(start_paused = true)]
async fn cancelled_queued_run_reports_cancelled() {
    let (h, f) = session(fake_pg().exec(vec![slow("SELECT pg_sleep", secs(30))])).await;
    let running = h.submit(exec("SELECT pg_sleep(30)"));
    settle_until(
        "the slow statement holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == running.seq),
    )
    .await;
    let o = Recorder::new();
    let queued = h.run(ask_spec(&o, &["SELECT 1", "SELECT 2"]));
    assert_eq!(
        h.cancel(running.seq).await.unwrap(),
        CancelOutcome::Cancelled
    );
    assert_eq!(
        queued.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    // Its observer heard it, without the reply's help: every unit cancelled, none started.
    assert_eq!(o.unit_ends(), vec![UnitEnd::Cancelled, UnitEnd::Cancelled]);
    assert!(
        !o.events()
            .iter()
            .any(|e| matches!(e, RunEvent::UnitStarted { .. }))
    );
    let summary = o.summaries();
    assert_eq!(summary.len(), 1);
    assert_eq!(
        RunSummary {
            duration_ms: 0,
            ..summary[0]
        },
        RunSummary {
            executed: 0,
            total: 2,
            failed: 0,
            skipped: 2,
            aborted: true,
            duration_ms: 0,
        }
    );
    assert!(f.typed_log().is_empty(), "nothing of it sent");
}

#[tokio::test(start_paused = true)]
async fn a_deactivate_reports_the_queued_run_cancelled() {
    let held = FakeQuery {
        first_delay: secs(30),
        ..answering("SELECT 1")
    };
    let (h, _f) = session(fake_pg().tracks_transactions().queries(vec![held])).await;
    let o1 = Recorder::new();
    let first = h.run(ask_spec(&o1, &["SELECT 1"]));
    settle_until(
        "the run holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == first.seq),
    )
    .await;
    let o2 = Recorder::new();
    let queued = h.run(ask_spec(&o2, &["SELECT 2"]));
    let deactivated = h.escalate(first.seq, hacivat_session::session::Escalation::Deactivate);
    assert_eq!(
        queued.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(deactivated.await.unwrap(), CancelOutcome::Cancelled);
    // A deactivate drops the queue as a cancel does: the queued run says so.
    assert_eq!(o2.unit_ends(), vec![UnitEnd::Cancelled]);
}

#[tokio::test(start_paused = true)]
async fn a_queued_run_dropped_by_a_loss_never_started() {
    // Only a cancel reports a queued run: one a lost connection drops is answered as before,
    // without `RunEnded` (it never started).
    let (h, f) = session(fake_pg().exec(vec![slow("SELECT pg_sleep", secs(30))])).await;
    let running = h.submit(exec("SELECT pg_sleep(30)"));
    settle_until(
        "the slow statement holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == running.seq),
    )
    .await;
    let o = Recorder::new();
    let queued = h.run(ask_spec(&o, &["SELECT 1"]));
    f.kill_connections();
    assert!(running.reply.await.unwrap().is_err());
    assert_eq!(
        queued.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::NotConnected
    );
    assert!(o.events().is_empty(), "{:?}", o.events());
    let _ = ask(&h, Request::Ping).await;
}
