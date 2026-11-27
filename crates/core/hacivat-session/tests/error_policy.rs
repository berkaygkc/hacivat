//! M3b Task 11: the `Ask` error policy (R-M3-61, P-M3b-3, P-M3b-19). A failed unit with later
//! units asks (`RunEvent::NeedsChoice`) and the run waits on the session's run gate while the
//! actor still serves cancels and closes; the answer names its question (T1c review I1) and
//! must be one the question offers. Retry, Ignore, Ignore all, Stop, and Roll back and continue
//! inside a failed PostgreSQL transaction (which keeps the session's mode). With no answer
//! nothing more is sent. FakeDriver and a paused clock: no socket, no real time.

mod common;

use common::runs::*;
use common::runs_m3b::*;
use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{FakeDriver, FakeTypedCall};
use hacivat_session::session::{
    CancelOutcome, ClosePolicy, ErrorPolicy, ExecErrorKind, RowsOutcome, RunChoice, RunEvent,
    SessionState, TxCommand, TxMode, TxOwner, UnitEnd,
};

/// A PostgreSQL fake that follows `BEGIN`/`COMMIT`/`ROLLBACK`.
fn pg() -> FakeDriver {
    fake_pg().tracks_transactions()
}

/// The statements sent through `execute`/`batch`, without the schema probe.
fn statements(f: &FakeDriver) -> Vec<String> {
    f.executed()
        .into_iter()
        .filter(|s| !s.starts_with("SELECT current_database()"))
        .collect()
}

/// The typed queries sent, by text.
fn queries(f: &FakeDriver) -> Vec<String> {
    f.typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(sql) => Some(sql),
            _ => None,
        })
        .collect()
}

fn ok_row() -> UnitEnd {
    UnitEnd::Succeeded {
        rows: Some(RowsOutcome::Fetched {
            rows: 1,
            more: false,
        }),
    }
}

fn is_failed(e: &UnitEnd) -> bool {
    matches!(e, UnitEnd::Failed(err) if err.kind == ExecErrorKind::Db)
}

/// Three units; the second fails.
fn three() -> FakeDriver {
    pg().queries(vec![
        answering("SELECT 1"),
        failing("SELECT * FROM missing"),
        answering("SELECT 3"),
    ])
}

const THREE: [&str; 3] = ["SELECT 1", "SELECT * FROM missing", "SELECT 3"];

#[tokio::test(start_paused = true)]
async fn ask_policy_waits_for_a_choice_and_serves_cancel() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert_eq!(q.unit.ordinal, 1);
    assert_eq!(q.error.kind, ExecErrorKind::Db);
    assert!(!q.rollback_and_continue, "outside a transaction");
    // The run waits: unit 3 is not sent, and the session is still busy with the run.
    assert_eq!(queries(&f), &THREE[..2]);
    assert!(matches!(snap(&h).state, SessionState::Busy { req, .. } if req == t.seq));
    // A cancel while it waits answers Stop.
    assert_eq!(h.cancel(t.seq).await.unwrap(), CancelOutcome::Cancelled);
    let e = t.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db, "the reply is the unit's error");
    let ends = o.unit_ends();
    assert_eq!(ends.len(), 3);
    assert_eq!(ends[0], ok_row());
    assert!(is_failed(&ends[1]), "{ends:?}");
    assert_eq!(ends[2], UnitEnd::Skipped);
    assert_eq!(queries(&f), &THREE[..2], "nothing more sent");
    let summary = o.summaries()[0];
    assert_eq!(
        (
            summary.executed,
            summary.failed,
            summary.skipped,
            summary.aborted
        ),
        (1, 1, 1, true)
    );
    // The question is gone with its run.
    assert!(
        !h.choose(t.seq, q.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn choice_reaches_a_run_that_is_waiting() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    // Another run's number, another question id: refused, and the run still waits.
    assert!(
        !h.choose(t.seq + 1, q.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    assert!(
        !h.choose(t.seq, q.question + 1, RunChoice::Ignore)
            .await
            .unwrap()
    );
    assert_eq!(queries(&f).len(), 2);
    assert!(
        h.choose(t.seq, q.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    // A double click answers nothing more.
    assert!(
        !h.choose(t.seq, q.question, RunChoice::IgnoreAll)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().expect("the run went on to its end");
    let ends = o.unit_ends();
    assert_eq!(ends[0], ok_row());
    assert!(is_failed(&ends[1]));
    assert_eq!(ends[2], ok_row());
    assert_eq!(queries(&f), THREE);
    let summary = o.summaries()[0];
    assert_eq!(
        (
            summary.executed,
            summary.failed,
            summary.skipped,
            summary.aborted
        ),
        (2, 1, 0, false)
    );
    assert_eq!(asked(&o).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn stop_skips_the_rest() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert!(h.choose(t.seq, q.question, RunChoice::Stop).await.unwrap());
    assert_eq!(t.reply.await.unwrap().unwrap_err().kind, ExecErrorKind::Db);
    assert_eq!(o.unit_ends()[2], UnitEnd::Skipped);
    assert_eq!(queries(&f).len(), 2, "nothing sent after Stop");
}

#[tokio::test(start_paused = true)]
async fn ignore_all_does_not_ask_again() {
    let (h, f) = session(pg().queries(vec![
        answering("SELECT 1"),
        failing("SELECT * FROM a"),
        failing("SELECT * FROM b"),
        answering("SELECT 4"),
    ]))
    .await;
    let o = Recorder::new();
    let sqls = ["SELECT 1", "SELECT * FROM a", "SELECT * FROM b", "SELECT 4"];
    let t = h.run(ask_spec(&o, &sqls));
    let q = nth_question(&o, 1).await;
    assert!(
        h.choose(t.seq, q.question, RunChoice::IgnoreAll)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().expect("the run went on to its end");
    assert_eq!(asked(&o).len(), 1, "asked once");
    let ends = o.unit_ends();
    assert_eq!(ends[0], ok_row());
    assert!(is_failed(&ends[1]) && is_failed(&ends[2]), "{ends:?}");
    assert_eq!(ends[3], ok_row());
    assert_eq!(queries(&f), sqls);
}

#[tokio::test(start_paused = true)]
async fn retry_runs_the_failed_unit_once_more() {
    let (h, f) = session(pg().queries(vec![
        answering("SELECT 1"),
        failing("SELECT * FROM flaky"),
        answering("SELECT * FROM flaky"),
        answering("SELECT 3"),
    ]))
    .await;
    let o = Recorder::new();
    let sqls = ["SELECT 1", "SELECT * FROM flaky", "SELECT 3"];
    let t = h.run(ask_spec(&o, &sqls));
    let q = nth_question(&o, 1).await;
    assert!(h.choose(t.seq, q.question, RunChoice::Retry).await.unwrap());
    t.reply.await.unwrap().expect("the retry succeeded");
    assert_eq!(
        queries(&f),
        [
            "SELECT 1",
            "SELECT * FROM flaky",
            "SELECT * FROM flaky",
            "SELECT 3"
        ]
    );
    assert_eq!(started(&o), vec![0, 1, 1, 2]);
    let ends = o.unit_ends();
    assert_eq!(ends.len(), 4);
    assert!(is_failed(&ends[1]));
    assert_eq!(ends[2], ok_row());
    let summary = o.summaries()[0];
    assert_eq!(
        (
            summary.executed,
            summary.failed,
            summary.skipped,
            summary.total
        ),
        (3, 0, 0, 3),
        "the summary counts the last attempt"
    );
}

#[tokio::test(start_paused = true)]
async fn a_retry_that_fails_again_asks_a_new_question() {
    let (h, f) = session(pg().queries(vec![
        failing("SELECT * FROM gone"),
        failing("SELECT * FROM gone"),
        answering("SELECT 2"),
    ]))
    .await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &["SELECT * FROM gone", "SELECT 2"]));
    let first = nth_question(&o, 1).await;
    assert!(
        h.choose(t.seq, first.question, RunChoice::Retry)
            .await
            .unwrap()
    );
    let second = nth_question(&o, 2).await;
    assert_ne!(first.question, second.question);
    assert_eq!(first.unit, second.unit);
    // The first question's answer is stale now.
    assert!(
        !h.choose(t.seq, first.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    assert!(
        h.choose(t.seq, second.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().unwrap();
    assert_eq!(queries(&f).len(), 3);
}

#[tokio::test(start_paused = true)]
async fn rollback_and_continue_replaces_ignore_in_a_failed_pg_tx() {
    let (h, f) = session(three()).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert!(q.rollback_and_continue);
    // The run reported the failed transaction before it asked.
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::InTx,
        to: TxStatus::Failed,
    }));
    // Ignore and Retry are not offered there: refused, the question still waits.
    for refused in [RunChoice::Ignore, RunChoice::Retry] {
        assert!(
            !h.choose(t.seq, q.question, refused).await.unwrap(),
            "{refused:?}"
        );
    }
    assert!(!statements(&f).contains(&"ROLLBACK".to_string()));
    assert!(
        h.choose(t.seq, q.question, RunChoice::RollbackAndContinue)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().expect("the run went on");
    assert_eq!(statements(&f), vec!["BEGIN", "ROLLBACK"]);
    assert_eq!(queries(&f), THREE);
    assert_eq!(o.unit_ends()[2], ok_row());
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::Failed,
        to: TxStatus::Idle,
    }));
    // The rollback comes before unit 3 starts.
    let events = o.events();
    let rolled = events
        .iter()
        .position(|e| {
            matches!(
                e,
                RunEvent::TxChanged {
                    to: TxStatus::Idle,
                    ..
                }
            )
        })
        .unwrap();
    let third = events
        .iter()
        .position(|e| matches!(e, RunEvent::UnitStarted { unit, .. } if unit.ordinal == 2))
        .unwrap();
    assert!(rolled < third);
    let s = snap(&h);
    assert_eq!((s.tx.status, s.tx.mode), (TxStatus::Idle, TxMode::Auto));
}

#[tokio::test(start_paused = true)]
async fn rollback_and_continue_is_refused_outside_a_failed_tx() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert!(!q.rollback_and_continue);
    assert!(
        !h.choose(t.seq, q.question, RunChoice::RollbackAndContinue)
            .await
            .unwrap()
    );
    assert!(statements(&f).is_empty(), "no ROLLBACK sent");
    assert!(h.choose(t.seq, q.question, RunChoice::Stop).await.unwrap());
    assert!(t.reply.await.unwrap().is_err());
    assert!(statements(&f).is_empty());
}

#[tokio::test(start_paused = true)]
async fn rollback_and_continue_keeps_manual_mode() {
    let (h, f) = session(three()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert!(q.rollback_and_continue);
    assert!(
        h.choose(t.seq, q.question, RunChoice::RollbackAndContinue)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().expect("the run went on");
    // Unit 1 opened the user's transaction, the rollback ended it, unit 3 opened a new one.
    assert_eq!(statements(&f), vec!["BEGIN", "ROLLBACK", "BEGIN"]);
    assert_eq!(o.unit_ends()[2], ok_row());
    let s = snap(&h);
    assert_eq!(
        (s.tx.mode, s.tx.status, s.tx.owner),
        (TxMode::Manual, TxStatus::InTx, TxOwner::User)
    );
}

#[tokio::test(start_paused = true)]
async fn close_while_waiting_answers_stop() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    // The close waits behind the run and answers its question Stop.
    h.close(ClosePolicy::RollBack).await.unwrap();
    let e = t.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert_eq!(o.unit_ends()[2], UnitEnd::Skipped);
    assert_eq!(queries(&f).len(), 2, "nothing sent after the close");
    assert_eq!(snap(&h).state, SessionState::Disconnected);
    assert!(
        !h.choose(t.seq, q.question, RunChoice::Ignore)
            .await
            .unwrap_or(false)
    );
}

#[tokio::test(start_paused = true)]
async fn a_close_queued_before_the_failure_answers_stop_without_asking() {
    // Unit 1 is slow: the close arrives while it runs, before unit 2 fails.
    let slow = hacivat_driver::fake::FakeQuery {
        first_delay: secs(30),
        ..answering("SELECT 1")
    };
    let (h, f) = session(pg().queries(vec![
        slow,
        failing("SELECT * FROM missing"),
        answering("SELECT 3"),
    ]))
    .await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    settle_until(
        "the run holds the session",
        || matches!(snap(&h).state, SessionState::Busy { req, .. } if req == t.seq),
    )
    .await;
    let closed = h.close(ClosePolicy::RollBack);
    let e = t.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    closed.await.unwrap();
    assert!(asked(&o).is_empty(), "a closing console is not asked");
    assert_eq!(queries(&f).len(), 2);
}

#[tokio::test(start_paused = true)]
async fn stop_on_error_and_a_failed_last_unit_never_ask() {
    // StopOnError stops at the first failure.
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let mut spec = ask_spec(&o, &THREE);
    spec.policy = ErrorPolicy::StopOnError;
    assert!(run_spec(&h, spec).await.is_err());
    assert!(asked(&o).is_empty());
    assert_eq!(queries(&f).len(), 2);
    // Ask with nothing after the failure ends the run.
    let (h, _f) = session(pg().queries(vec![failing("SELECT * FROM missing")])).await;
    let o = Recorder::new();
    assert!(
        run_spec(&h, ask_spec(&o, &["SELECT * FROM missing"]))
            .await
            .is_err()
    );
    assert!(asked(&o).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_cancel_that_races_the_answer_wins() {
    let (h, f) = session(three()).await;
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    // Both reach the actor before the run is polled again: the answer is taken, the stop
    // wins.
    let answered = h.choose(t.seq, q.question, RunChoice::Ignore);
    let cancelled = h.cancel(t.seq);
    assert!(answered.await.unwrap());
    assert_eq!(cancelled.await.unwrap(), CancelOutcome::Cancelled);
    assert!(t.reply.await.unwrap().is_err());
    assert_eq!(o.unit_ends()[2], UnitEnd::Skipped);
    assert_eq!(queries(&f).len(), 2, "unit 3 never sent");
}

// ## m3b-t11-session-runs (fix round 1, review I1)

/// The `SessionEvent::TxChanged` events, in order, received so far.
fn tx_events(
    ev: &mut tokio::sync::broadcast::Receiver<hacivat_session::session::SessionEvent>,
) -> Vec<hacivat_session::session::TxState> {
    let mut out = Vec::new();
    while let Ok(e) = ev.try_recv() {
        if let hacivat_session::session::SessionEvent::TxChanged(tx) = e {
            out.push(tx);
        }
    }
    out
}

/// While a choice waits, the snapshot and the session's events show the transaction the
/// question is about (the badge, the "Roll back and continue" count); the session stays busy.
/// What the answer ends is shown at once, while the run goes on.
#[tokio::test(start_paused = true)]
async fn a_pending_choice_shows_the_failed_transaction() {
    let slow_third = hacivat_driver::fake::FakeQuery {
        first_delay: secs(30),
        ..answering("SELECT 3")
    };
    let (h, _f) = session(pg().queries(vec![
        answering("SELECT 1"),
        failing("SELECT * FROM missing"),
        slow_third,
    ]))
    .await;
    run(&h, "BEGIN").await.unwrap();
    let mut ev = h.events();
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert!(q.rollback_and_continue);
    let s = snap(&h);
    assert!(matches!(s.state, SessionState::Busy { req, .. } if req == t.seq));
    assert_eq!(
        (s.tx.status, s.tx.owner, s.tx.statements),
        (TxStatus::Failed, TxOwner::User, 2)
    );
    let seen = tx_events(&mut ev);
    assert_eq!(seen.last().map(|tx| tx.status), Some(TxStatus::Failed));

    assert!(
        h.choose(t.seq, q.question, RunChoice::RollbackAndContinue)
            .await
            .unwrap()
    );
    // Unit 3 is held 30 s (paused): the rollback is already shown, the run still busy.
    settle_until("the rollback is shown", || {
        snap(&h).tx.status == TxStatus::Idle
    })
    .await;
    assert!(matches!(snap(&h).state, SessionState::Busy { req, .. } if req == t.seq));
    assert_eq!(
        tx_events(&mut ev).last().map(|tx| tx.status),
        Some(TxStatus::Idle)
    );
    t.reply.await.unwrap().unwrap();
    // The end of the run changed nothing more: no second event for the same state.
    assert!(tx_events(&mut ev).is_empty());
}

/// Manual mode: the failed transaction, then its rollback, then the next unit's new
/// transaction, each one event.
#[tokio::test(start_paused = true)]
async fn manual_mode_shows_each_transaction_state_once() {
    let (h, _f) = session(three()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let mut ev = h.events();
    let o = Recorder::new();
    let t = h.run(ask_spec(&o, &THREE));
    let q = nth_question(&o, 1).await;
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    assert!(
        h.choose(t.seq, q.question, RunChoice::RollbackAndContinue)
            .await
            .unwrap()
    );
    t.reply.await.unwrap().unwrap();
    let statuses: Vec<TxStatus> = tx_events(&mut ev).iter().map(|tx| tx.status).collect();
    assert_eq!(
        statuses,
        vec![TxStatus::Failed, TxStatus::Idle, TxStatus::InTx]
    );
}
