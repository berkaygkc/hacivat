//! M2b Task 7: the session actor (HV-SESS-01, -08): the FIFO queue, the snapshot, panics,
//! reset and close.

mod common;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_proto::ErrClass;
use hacivat_session::session::{ClosePolicy, CloseRefused, ExecErrorKind, Request, SessionState};

#[tokio::test]
async fn requests_run_in_fifo_order() {
    let (h, f) = session(fake_pg().exec(vec![slow("select 1", ms(50))])).await;
    let a = h.submit(exec("select 1"));
    let b = h.submit(exec("select 2"));
    a.reply.await.unwrap().unwrap();
    b.reply.await.unwrap().unwrap();
    assert_eq!(f.executed(), ["select 1", "select 2"]);
    assert!(b.seq > a.seq);
}

#[tokio::test]
async fn a_fresh_session_is_idle_and_ran_nothing() {
    // Q27: neither the environment plan nor a schema probe at spawn.
    let (h, f) = session(fake_pg()).await;
    let s = snap(&h);
    assert_eq!(s.state, SessionState::Idle);
    assert_eq!(s.schema, None);
    assert!(f.executed().is_empty(), "{:?}", f.executed());
}

#[tokio::test]
async fn every_change_bumps_the_generation() {
    let (h, _) = session(fake_pg()).await;
    let before = snap(&h).generation;
    run(&h, "select 1").await.unwrap();
    // Busy, then Idle.
    assert!(
        snap(&h).generation >= before + 2,
        "{before} → {}",
        snap(&h).generation
    );
}

#[tokio::test]
async fn driver_panic_marks_session_broken_internal() {
    let (h, f) = session(fake_pg().exec(vec![panics("select boom")])).await;
    let r = h.submit(exec("select boom")).reply.await.unwrap();
    assert_eq!(r.unwrap_err().kind, ExecErrorKind::Internal);
    assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::Internal));
    assert_eq!(f.open_connections(), 0, "the connection is discarded");
    // A broken session is never reused.
    let later = h.submit(exec("select 1")).reply.await.unwrap();
    assert_eq!(later.unwrap_err().kind, ExecErrorKind::NotConnected);
    assert!(!f.executed().iter().any(|s| s == "select 1"));
}

#[tokio::test]
async fn reset_rolls_back_then_resets_then_reapplies_env() {
    let (h, f) = in_tx_session().await;
    h.submit(Request::Reset).reply.await.unwrap().unwrap();
    assert_eq!(
        &f.executed()[f.executed().len() - 3..],
        [
            "ROLLBACK",
            "SELECT current_user = session_user",
            "SET application_name = 'Hacivat'"
        ]
    );
    assert_eq!(f.reset_calls(), 1);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).state, SessionState::Idle);
}

#[tokio::test]
async fn failed_reset_breaks_and_closes() {
    let (h, f) = session(fake_pg().reset_fails()).await;
    let _ = h.submit(Request::Reset).reply.await;
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
}

#[tokio::test]
async fn a_reset_whose_verify_probe_is_not_true_breaks() {
    // Fail closed: the role check must answer `t`.
    let (h, f) =
        session(fake_pg().exec(vec![rows("SELECT current_user = session_user", &[&["f"]])])).await;
    let r = h.submit(Request::Reset).reply.await.unwrap();
    assert!(r.is_err());
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert_eq!(f.open_connections(), 0);
    assert!(
        !f.executed()
            .iter()
            .any(|s| s.starts_with("SET application_name")),
        "nothing is replayed on a connection that failed its verify"
    );
}

#[tokio::test]
async fn close_with_open_tx() {
    let (h, _) = in_tx_session().await;
    assert_eq!(
        h.close(ClosePolicy::RefuseIfOpenTx).await,
        Err(CloseRefused::OpenTransaction)
    );
    assert_eq!(h.close(ClosePolicy::RollBack).await, Ok(()));
}

#[tokio::test]
async fn close_rolls_back_closes_and_answers_later_requests_not_connected() {
    let (h, f) = in_tx_session().await;
    assert_eq!(h.close(ClosePolicy::RollBack).await, Ok(()));
    assert_eq!(f.executed().last().map(String::as_str), Some("ROLLBACK"));
    assert_eq!(f.open_connections(), 0);
    assert_eq!(snap(&h).state, SessionState::Disconnected);
    let r = h.submit(exec("select 1")).reply.await.unwrap();
    assert_eq!(r.unwrap_err().kind, ExecErrorKind::NotConnected);
}

#[tokio::test]
async fn a_lost_connection_breaks_the_session_with_an_unknown_outcome() {
    let (h, f) = session(fake_pg()).await;
    f.kill_connections();
    let r = run(&h, "update t set a = 1").await;
    assert_eq!(
        r.unwrap_err().kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    assert_eq!(snap(&h).state, SessionState::Broken(ErrClass::NetworkReset));
    assert_eq!(f.open_connections(), 0);
}
