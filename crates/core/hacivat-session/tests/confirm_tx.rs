//! M3b Task 19: confirm-in-transaction (HV-EXEC-04 session side; D-014, R-M3-55, P-M3b-1,
//! P-M3b-5, P-M3b-12). The session picks the form at execution time from the live transaction
//! state: Hacivat's own transaction only when it is truly idle in Auto (`BEGIN` /
//! `START TRANSACTION` / `BEGIN IMMEDIATE`, Commit/Roll back), the savepoint `hv_confirm`
//! (Keep/Undo) whenever a transaction is open, so no confirm path ever commits the user's
//! transaction. The D-014 deadline, a cancel and a close roll back; a failure rolls back only
//! the statement; a MySQL DDL meeting an open transaction the gate did not predict is refused.
//! Each P-M3b-1 shape on the PostgreSQL, MySQL and SQLite fakes. FakeDriver and a paused clock:
//! asserted on `call_log()` (P-M3b-18), no socket, no real time.

mod common;

use std::sync::Arc;

use common::confirm::*;
use common::runs::Recorder;
use common::runs_m3b::{answering, asked, failing, nth_question, spec_units};
use common::*;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeExec, FakeQuery};
use hacivat_driver::{ExecSummary, Protocol, TxStatus};
use hacivat_proto::DbmsKind;
use hacivat_session::session::{
    ClosePolicy, CommitAnswer, DenyLayer, ErrorPolicy, ExecError, ExecErrorKind, IsolationLevel,
    RunChoice, RunEvent, RunSpec, SessionEffects, SessionEvent, SessionHandle, SessionState,
    TxCommand, TxError, TxMode, TxOwner, UnitEnd, UnitVerdict,
};

const UPDATE: &str = "UPDATE t SET a=1 WHERE id=3";

/// A fake of `kind` that follows `BEGIN`/`COMMIT`/`ROLLBACK`.
fn fake(kind: DbmsKind) -> FakeDriver {
    FakeDriver::new(kind).tracks_transactions()
}

const ENGINES: [DbmsKind; 3] = [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::Sqlite];

/// A mark in the fake's log once the session has done its connect steps: a request that sends
/// nothing, queued behind them.
async fn mark(h: &SessionHandle, f: &FakeDriver) -> usize {
    tx_cmd(h, TxCommand::SetIsolation(IsolationLevel::Default))
        .await
        .expect("an idle session takes the isolation level");
    f.call_log().len()
}

/// A MySQL statement after which the server reports a transaction open (`autocommit = 0` opens
/// one with the first statement; the fake does not model it on its own).
fn opens(prefix: &'static str) -> FakeExec {
    FakeExec {
        prefix,
        result: Ok(ExecSummary::default()),
        delay: std::time::Duration::ZERO,
        tx_after: Some(TxStatus::InTx),
        panics: false,
    }
}

/// The user's own transaction start on `kind`, as a run unit.
fn user_begin(kind: DbmsKind) -> &'static str {
    match kind {
        DbmsKind::MySql => "START TRANSACTION",
        _ => "BEGIN",
    }
}

/// The calls whose text starts a transaction (`BEGIN …`, `START TRANSACTION`).
fn starts(calls: &[FakeCall]) -> Vec<FakeCall> {
    calls
        .iter()
        .filter(|c| match c {
            FakeCall::Statement(s) => s.starts_with("BEGIN") || s.starts_with("START"),
            _ => false,
        })
        .cloned()
        .collect()
}

fn failed_kind(o: &Recorder, i: usize) -> ExecErrorKind {
    match &o.unit_ends()[i] {
        UnitEnd::Failed(e) => e.kind.clone(),
        other => panic!("unit {i} did not fail: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// The own-transaction form (truly idle in Auto)

#[tokio::test(start_paused = true)]
async fn auto_idle_confirm_uses_hacivats_own_tx() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(
        &h,
        "UPDATE t SET a=1 WHERE id=3",
        UnitVerdict::TxThenConfirm,
    );
    answer_commit(&h, &t).await;
    // call_log, not executed() (P-M3b-18)
    assert_eq!(
        &f.call_log()[before..],
        [
            FakeCall::Statement("BEGIN".into()),
            FakeCall::Typed(hacivat_driver::fake::FakeTypedCall::Query(
                "UPDATE t SET a=1 WHERE id=3".into()
            )),
            FakeCall::Statement("COMMIT".into())
        ]
    );
}

/// P-M3b-1 on every engine: `BEGIN` (PostgreSQL), `START TRANSACTION` (MySQL), `BEGIN
/// IMMEDIATE` (SQLite), owner `Confirm` while the question waits (shown as no transaction of
/// the user's), Commit → `COMMIT`; the question carries the affected rows and the own form.
#[tokio::test(start_paused = true)]
async fn own_form_on_every_engine() {
    for kind in ENGINES {
        let (h, f) = session(fake(kind).queries(vec![affects("UPDATE", 1)])).await;
        let before = mark(&h, &f).await;
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        let asked = nth_commit(&t.o, 1).await;
        assert!(!asked.savepoint, "{kind:?}");
        assert_eq!(asked.affected, Some(1), "{kind:?}");
        let tx = snap(&h).tx;
        assert_eq!(
            (tx.status, tx.owner),
            (TxStatus::InTx, TxOwner::Confirm),
            "{kind:?}: Hacivat's transaction while the question waits"
        );
        assert!(tx.owner.is_hacivats());
        answer_commit(&h, &t).await;
        let begin = match kind {
            DbmsKind::Postgres => "BEGIN",
            DbmsKind::MySql => "START TRANSACTION",
            _ => "BEGIN IMMEDIATE",
        };
        assert_eq!(
            sent(&f, before),
            [st(begin), q(UPDATE), st("COMMIT")],
            "{kind:?}"
        );
        assert_eq!(decided(&t.o), [(CommitAnswer::Commit, false)], "{kind:?}");
        t.ticket.reply.await.unwrap().unwrap();
        let tx = snap(&h).tx;
        assert_eq!((tx.status, tx.owner), (TxStatus::Idle, TxOwner::None));
        assert!(
            !t.o.events()
                .iter()
                .any(|e| matches!(e, RunEvent::TxChanged { .. })),
            "{kind:?}: Hacivat's own transaction is not the user's"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn own_form_roll_back_sends_rollback() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    answer_rollback(&h, &t).await;
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

// ---------------------------------------------------------------------------------------------
// The savepoint form (any transaction open)

/// P-M3b-1: a Manual session opens the user's transaction first (PostgreSQL `BEGIN`, SQLite
/// `BEGIN IMMEDIATE`, MySQL nothing: `autocommit = 0` keeps one open), then the savepoint; the
/// owner stays `User` and Keep releases the savepoint only.
#[tokio::test(start_paused = true)]
async fn manual_idle_confirm_opens_the_user_tx_then_a_savepoint() {
    for kind in ENGINES {
        let (h, f) = session(fake(kind).exec(vec![opens("SAVEPOINT")])).await;
        tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
            .await
            .unwrap();
        let before = mark(&h, &f).await;
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        let asked = nth_commit(&t.o, 1).await;
        assert!(asked.savepoint, "{kind:?}");
        let expected: Vec<FakeCall> = match kind {
            DbmsKind::Postgres => vec![st("BEGIN"), st("SAVEPOINT hv_confirm"), q(UPDATE)],
            DbmsKind::MySql => vec![st("SAVEPOINT hv_confirm"), q(UPDATE)],
            _ => vec![st("BEGIN IMMEDIATE"), st("SAVEPOINT hv_confirm"), q(UPDATE)],
        };
        assert_eq!(sent(&f, before), expected, "{kind:?}");
        assert_eq!(snap(&h).tx.owner, TxOwner::User, "{kind:?}");
        answer_commit(&h, &t).await;
        let mut expected = expected;
        expected.push(st("RELEASE SAVEPOINT hv_confirm"));
        assert_eq!(sent(&f, before), expected, "{kind:?}: Keep");
        let tx = snap(&h).tx;
        assert_eq!(
            (tx.status, tx.owner, tx.mode),
            (TxStatus::InTx, TxOwner::User, TxMode::Manual),
            "{kind:?}: the user's transaction stays open"
        );
        assert!(starts(&sent(&f, before)).len() <= 1, "{kind:?}");
    }
}

/// P-M3b-1: the user's `BEGIN` earlier in the same run is seen at execution time: the
/// confirmed unit takes the savepoint, and no second `BEGIN` goes out.
#[tokio::test(start_paused = true)]
async fn user_begin_earlier_in_the_run_forces_the_savepoint_form() {
    for kind in ENGINES {
        let begin = user_begin(kind);
        let (h, f) = session(fake(kind).queries(vec![leaves(
            if kind == DbmsKind::MySql {
                "START TRANSACTION"
            } else {
                "BEGIN"
            },
            TxStatus::InTx,
        )]))
        .await;
        let before = mark(&h, &f).await;
        let t = run_units(
            &h,
            vec![
                verdict_unit(0, begin, UnitVerdict::Write),
                verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
            ],
            RunSpec::CONFIRM_DEADLINE,
        );
        assert!(nth_commit(&t.o, 1).await.savepoint, "{kind:?}");
        answer_commit(&h, &t).await;
        assert_eq!(
            sent(&f, before),
            [
                q(begin),
                st("SAVEPOINT hv_confirm"),
                q(UPDATE),
                st("RELEASE SAVEPOINT hv_confirm")
            ],
            "{kind:?}"
        );
        assert_eq!(snap(&h).tx.status, TxStatus::InTx, "{kind:?}");
        assert_eq!(snap(&h).tx.owner, TxOwner::User, "{kind:?}");
    }
}

/// MySQL/MariaDB never get `START TRANSACTION` (or `BEGIN`) while a transaction is open: the
/// user's (earlier in the run), Manual's (`autocommit = 0`), the user's own `autocommit = 0`
/// in Auto, and one open in Smart mode.
#[tokio::test(start_paused = true)]
async fn mysql_never_starts_a_transaction_inside_one() {
    {
        let kind = DbmsKind::MySql;
        // The user's `SET autocommit = 0` in Auto: a transaction is always open.
        let (h, f) = session(fake(kind).exec(vec![opens("SAVEPOINT")])).await;
        run(&h, "SET autocommit = 0").await.unwrap();
        let before = f.call_log().len();
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        assert!(nth_commit(&t.o, 1).await.savepoint, "{kind:?}");
        answer_commit(&h, &t).await;
        assert_eq!(
            sent(&f, before),
            [
                st("SAVEPOINT hv_confirm"),
                q(UPDATE),
                st("RELEASE SAVEPOINT hv_confirm")
            ],
            "{kind:?}: autocommit off"
        );

        // Smart with the user's transaction open: no second start before the write.
        let (h, f) =
            session(fake(kind).queries(vec![leaves("START TRANSACTION", TxStatus::InTx)])).await;
        tx_cmd(&h, TxCommand::SetMode(TxMode::Smart)).await.unwrap();
        let before = f.call_log().len();
        let t = run_units(
            &h,
            vec![
                verdict_unit(0, "START TRANSACTION", UnitVerdict::Write),
                verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
                verdict_unit(2, "UPDATE u SET b=2", UnitVerdict::Write),
            ],
            RunSpec::CONFIRM_DEADLINE,
        );
        answer_commit(&h, &t).await;
        let calls = sent(&f, before);
        assert!(starts(&calls).is_empty(), "{kind:?}: {calls:?}");
        assert!(
            !calls.contains(&st("COMMIT")),
            "{kind:?}: the confirm path never commits the user's transaction"
        );
    }
}

/// P-M3b-1: a MySQL DDL whose gate verdict did not predict the open transaction (an earlier
/// unit opened it) is refused by the session, nothing of it sent; with the `ImplicitCommit`
/// verdict (the user confirmed it) it runs; PostgreSQL ignores the bit.
#[tokio::test(start_paused = true)]
async fn unpredicted_open_tx_refuses_mysql_ddl() {
    const DDL: &str = "CREATE TABLE x (a int)";
    let ddl = |verdict| {
        let mut u = verdict_unit(1, DDL, verdict);
        u.effects = SessionEffects::DDL | SessionEffects::COMMITS_FIRST;
        u
    };
    let mut start = verdict_unit(0, "START TRANSACTION", UnitVerdict::Write);
    start.effects = SessionEffects::COMMITS_FIRST;

    let (h, f) =
        session(fake(DbmsKind::MySql).queries(vec![leaves("START TRANSACTION", TxStatus::InTx)]))
            .await;
    let before = mark(&h, &f).await;
    let t = run_units(
        &h,
        vec![start.clone(), ddl(UnitVerdict::Write)],
        RunSpec::CONFIRM_DEADLINE,
    );
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(
        e.kind,
        ExecErrorKind::Tx(TxError::ImplicitCommitUnconfirmed)
    );
    assert_eq!(
        failed_kind(&t.o, 1),
        ExecErrorKind::Tx(TxError::ImplicitCommitUnconfirmed)
    );
    assert_eq!(
        sent(&f, before),
        [q("START TRANSACTION")],
        "CREATE never sent"
    );
    assert_eq!(snap(&h).tx.status, TxStatus::InTx, "the transaction stays");

    // Confirmed as an implicit commit: it runs.
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![ddl(UnitVerdict::ImplicitCommit)],
        RunSpec::CONFIRM_DEADLINE,
    );
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [q(DDL)]);

    // A Manual MySQL session always counts as open.
    let (h, f) = session(fake(DbmsKind::MySql)).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = mark(&h, &f).await;
    let t = run_units(
        &h,
        vec![{
            let mut u = ddl(UnitVerdict::Write);
            u.id.ordinal = 0;
            u
        }],
        RunSpec::CONFIRM_DEADLINE,
    );
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::ImplicitCommitUnconfirmed)
    );
    assert!(sent(&f, before).is_empty());

    // PostgreSQL's DDL is transactional: the bit means nothing there.
    let (h, f) =
        session(fake(DbmsKind::Postgres).queries(vec![leaves("BEGIN", TxStatus::InTx)])).await;
    let before = f.call_log().len();
    let mut begin = verdict_unit(0, "BEGIN", UnitVerdict::Write);
    begin.effects = SessionEffects::empty();
    let t = run_units(
        &h,
        vec![begin, ddl(UnitVerdict::Write)],
        RunSpec::CONFIRM_DEADLINE,
    );
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [q("BEGIN"), q(DDL)]);
}

// ---------------------------------------------------------------------------------------------
// Failures, cancels, closes, the deadline (P-M3b-5, P-M3b-12, D-014)

/// P-M3b-5: the confirmed statement fails inside the user's transaction: only it is rolled
/// back, the transaction returns to `InTx`, the unit keeps the server's error, nothing is
/// asked. In the own form the session rolls back its own transaction.
#[tokio::test(start_paused = true)]
async fn failed_statement_in_savepoint_form_restores_the_user_tx() {
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx), failing("UPDATE")]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert!(e.server.is_some(), "the server's text is kept");
    assert_eq!(
        sent(&f, before),
        [
            q("BEGIN"),
            st("SAVEPOINT hv_confirm"),
            q(UPDATE),
            closed_now(),
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm"),
        ]
    );
    let tx = snap(&h).tx;
    assert_eq!(
        (tx.status, tx.owner),
        (TxStatus::InTx, TxOwner::User),
        "the user's transaction is intact and usable"
    );
    assert!(commit_questions(&t.o).is_empty());
    assert!(decided(&t.o).is_empty());

    // The own form: Hacivat's transaction is rolled back.
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![failing("UPDATE")]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Db
    );
    assert_eq!(
        sent(&f, before),
        [st("BEGIN"), q(UPDATE), closed_now(), st("ROLLBACK")]
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::Idle, TxOwner::None));
}

/// P-M3b-5: a cancel while the confirmed statement runs (before the question) rolls back only
/// the statement; the user's transaction stays open and nothing is asked.
#[tokio::test(start_paused = true)]
async fn cancel_before_the_question_rolls_back_only_the_statement() {
    let slow = FakeQuery {
        first_delay: secs(30),
        ..answering("UPDATE")
    };
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx), slow]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    settle_until("the confirmed statement runs", || {
        f.call_log()[before..].contains(&q(UPDATE))
    })
    .await;
    drop(h.cancel(t.ticket.seq));
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Cancelled);
    assert_eq!(t.o.unit_ends()[1], UnitEnd::Cancelled);
    let calls = sent(&f, before);
    assert_eq!(
        &calls[calls.len() - 2..],
        [
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm")
        ],
        "{calls:?}"
    );
    assert!(
        !calls.contains(&st("ROLLBACK")),
        "never the user's transaction"
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    assert!(commit_questions(&t.o).is_empty());
}

/// A cancel while the question waits rolls back (Hacivat's own transaction here); the unit
/// ends `Cancelled`, the decision says `RollBack`, not the deadline's.
#[tokio::test(start_paused = true)]
async fn cancel_while_the_question_waits_rolls_back() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    let asked = nth_commit(&t.o, 1).await;
    drop(h.cancel(t.ticket.seq));
    until_ended(&t.o).await;
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    assert_eq!(t.o.unit_ends(), [UnitEnd::Cancelled]);
    // An answer after the cancel is refused.
    assert!(
        !h.commit_decision(
            t.ticket.seq,
            asked.unit,
            asked.question,
            CommitAnswer::Commit
        )
        .await
        .unwrap_or(false)
    );
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert!(!f.call_log()[before..].contains(&st("COMMIT")));
}

/// P-M3b-12: `RefuseIfOpenTx` never refuses because of Hacivat's own confirm transaction: the
/// close rolls it back (it never commits) and goes through.
#[tokio::test(start_paused = true)]
async fn close_refuse_if_open_tx_does_not_refuse_hacivats_confirm_tx() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    nth_commit(&t.o, 1).await;
    assert_eq!(snap(&h).tx.owner, TxOwner::Confirm);
    h.close(ClosePolicy::RefuseIfOpenTx)
        .await
        .expect("Hacivat's own transaction never refuses a close");
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    assert_eq!(snap(&h).state, SessionState::Disconnected);
}

/// A close while the question waits rolls back Hacivat's own transaction once, and only that.
#[tokio::test(start_paused = true)]
async fn close_while_waiting_rolls_back_hacivats_tx_only() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    nth_commit(&t.o, 1).await;
    h.close(ClosePolicy::RollBack).await.unwrap();
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    assert!(run_ended(&t.o));

    // In the savepoint form the close undoes the statement first; the user's transaction is
    // then the close's to refuse (it is never committed).
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx)]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    nth_commit(&t.o, 1).await;
    assert!(h.close(ClosePolicy::RefuseIfOpenTx).await.is_err());
    let calls = sent(&f, before);
    assert_eq!(
        &calls[calls.len() - 2..],
        [
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm")
        ]
    );
    assert!(!calls.contains(&st("COMMIT")));
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
}

/// P-M3b-12: no idle-transaction warning fires for Hacivat's confirm transaction, however long
/// the question waits (here past the profile's idle threshold).
#[tokio::test(start_paused = true)]
async fn no_idle_tx_warning_for_the_confirm_tx() {
    let profile = Arc::new(hacivat_session::profile::Profile {
        idle_disconnect_s: Some(30),
        ..pg_profile(hacivat_net::TlsMode::Disable)
    });
    let (m, f) = manager(fake_pg().tracks_transactions());
    m.connect(
        profile.clone(),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, profile.id).await;
    let h = m.sessions(profile.id).unwrap().default_session().unwrap();
    let mut events = h.events();
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    nth_commit(&t.o, 1).await;
    tokio::time::sleep(secs(120)).await;
    assert!(
        matches!(snap(&h).state, SessionState::Busy { .. }),
        "still waiting, never disconnected: {:?}",
        snap(&h).state
    );
    answer_commit(&h, &t).await;
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, SessionEvent::IdleTransaction { .. })),
        "{seen:?}"
    );
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("COMMIT")]);
}

/// D-014 / R-M3-55: the deadline undoes the confirmed statement only (`ROLLBACK TO SAVEPOINT
/// hv_confirm; RELEASE SAVEPOINT hv_confirm`); the user's transaction stays open and theirs;
/// `CommitDecided { by_deadline: true }`; a late answer is refused. In the own form the
/// deadline rolls back Hacivat's transaction.
#[tokio::test(start_paused = true)]
async fn deadline_rolls_back_only_the_confirmed_statement() {
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx)]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        secs(2),
    );
    let asked = nth_commit(&t.o, 1).await;
    assert!(asked.savepoint);
    tokio::time::sleep(secs(3)).await;
    until_ended(&t.o).await;
    assert_eq!(
        sent(&f, before),
        [
            q("BEGIN"),
            st("SAVEPOINT hv_confirm"),
            q(UPDATE),
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm"),
        ]
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, true)]);
    assert!(
        !h.commit_decision(
            t.ticket.seq,
            asked.unit,
            asked.question,
            CommitAnswer::Commit
        )
        .await
        .unwrap_or(false),
        "a late answer is refused"
    );
    // Fix round 1 (C2): the deadline's rollback is no success.
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::ConfirmTimedOut)
    );

    // The own form.
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![verdict_unit(0, UPDATE, UnitVerdict::TxThenConfirm)],
        secs(2),
    );
    nth_commit(&t.o, 1).await;
    tokio::time::sleep(secs(3)).await;
    until_ended(&t.o).await;
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, true)]);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

/// The deadline's `NeedsCommit::deadline` is the injected one from the question's time.
#[tokio::test(start_paused = true)]
async fn the_question_carries_its_deadline() {
    let (h, _f) = session(fake_pg().tracks_transactions()).await;
    let t = run_units(
        &h,
        vec![verdict_unit(0, UPDATE, UnitVerdict::TxThenConfirm)],
        secs(2),
    );
    nth_commit(&t.o, 1).await;
    let now = tokio::time::Instant::now().into_std();
    let deadline =
        t.o.events()
            .into_iter()
            .find_map(|e| match e {
                RunEvent::NeedsCommit { deadline, .. } => Some(deadline),
                _ => None,
            })
            .unwrap();
    assert_eq!(deadline.duration_since(now), secs(2));
    answer_rollback(&h, &t).await;
}

/// R-M3-14: in Manual mode a confirmed statement is a per-statement Keep/Undo under a
/// savepoint; Undo never ends the user's transaction, and the user's own `COMMIT` is the only
/// commit sent.
#[tokio::test(start_paused = true)]
async fn manual_mode_confirm_uses_a_savepoint_and_keeps_the_user_tx() {
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("COMMIT", TxStatus::Idle)]),
    )
    .await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    answer_rollback(&h, &t).await;
    assert_eq!(
        sent(&f, before),
        [
            st("BEGIN"),
            st("SAVEPOINT hv_confirm"),
            q(UPDATE),
            st("ROLLBACK TO SAVEPOINT hv_confirm"),
            st("RELEASE SAVEPOINT hv_confirm"),
        ]
    );
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    let tx = snap(&h).tx;
    assert_eq!(
        (tx.status, tx.owner, tx.mode),
        (TxStatus::InTx, TxOwner::User, TxMode::Manual)
    );
    // The user commits: that is the one `COMMIT`.
    let mark = f.call_log().len();
    let t = run_verdict(&h, "COMMIT", UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, mark), [q("COMMIT")]);
    assert!(!f.call_log()[before..mark].contains(&st("COMMIT")));
}

/// A connection drop while the question waits: the server rolled back, so the session never
/// reports a commit (`CommitDecided` is not emitted) and the unit ends `ConnectionLost`.
#[tokio::test(start_paused = true)]
async fn connection_drop_during_confirm_reports_server_rollback() {
    for answer in [CommitAnswer::Commit, CommitAnswer::RollBack] {
        let (h, f) = session(fake_pg().tracks_transactions()).await;
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        nth_commit(&t.o, 1).await;
        f.kill_connections();
        assert!(answer_nth(&h, &t, 1, answer).await);
        until_ended(&t.o).await;
        assert!(
            decided(&t.o).is_empty(),
            "{answer:?}: never reported decided"
        );
        assert!(
            matches!(failed_kind(&t.o, 0), ExecErrorKind::ConnectionLost { .. }),
            "{answer:?}"
        );
        if answer == CommitAnswer::RollBack {
            assert_eq!(
                failed_kind(&t.o, 0),
                ExecErrorKind::ConnectionLost {
                    outcome_unknown: false
                }
            );
        }
        assert!(t.ticket.reply.await.unwrap().is_err());
    }
}

/// PostgreSQL keeps savepoints of one name as a stack: the user's own `hv_confirm` is shadowed
/// while Hacivat's is open and intact after Keep or Undo (replayed from the call log with
/// PostgreSQL's rules: the fake keeps its stack private).
#[tokio::test(start_paused = true)]
async fn savepoint_name_collision_with_a_user_hv_confirm() {
    for answer in [CommitAnswer::Commit, CommitAnswer::RollBack] {
        let (h, f) = session(fake_pg().tracks_transactions()).await;
        run(&h, "BEGIN").await.unwrap();
        run(&h, "SAVEPOINT hv_confirm").await.unwrap();
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        assert!(nth_commit(&t.o, 1).await.savepoint);
        assert_eq!(
            pg_savepoints(&f.call_log()),
            ["hv_confirm", "hv_confirm"],
            "the user's and Hacivat's"
        );
        assert!(answer_nth(&h, &t, 1, answer).await);
        until_ended(&t.o).await;
        assert_eq!(
            pg_savepoints(&f.call_log()),
            ["hv_confirm"],
            "{answer:?}: the user's savepoint remains"
        );
        assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    }
}

/// PostgreSQL's savepoint stack after the `execute`/`batch` statements of `calls`: `SAVEPOINT s`
/// pushes; `RELEASE [SAVEPOINT] s` drops the newest `s` and every later one; `ROLLBACK TO
/// [SAVEPOINT] s` drops every one later than the newest `s`; a transaction's start or end
/// clears it.
fn pg_savepoints(calls: &[FakeCall]) -> Vec<String> {
    let mut stack: Vec<String> = Vec::new();
    for call in calls {
        let FakeCall::Statement(sql) = call else {
            continue;
        };
        for statement in sql.split(';') {
            let words: Vec<String> = statement
                .split_whitespace()
                .map(|w| w.to_ascii_lowercase())
                .collect();
            let w = |i: usize| words.get(i).map_or("", String::as_str);
            let newest = |stack: &Vec<String>, name: &str| stack.iter().rposition(|s| s == name);
            match (w(0), w(1)) {
                ("savepoint", name) => stack.push(name.to_string()),
                ("release", _) => {
                    let name = if w(1) == "savepoint" { w(2) } else { w(1) };
                    if let Some(i) = newest(&stack, name) {
                        stack.truncate(i);
                    }
                }
                ("rollback", "to") => {
                    let name = if w(2) == "savepoint" { w(3) } else { w(2) };
                    if let Some(i) = newest(&stack, name) {
                        stack.truncate(i + 1);
                    }
                }
                ("begin" | "start" | "commit" | "end" | "rollback" | "abort", _) => stack.clear(),
                _ => {}
            }
        }
    }
    stack
}

// ---------------------------------------------------------------------------------------------
// Question identity, refusals

/// The RunGate rules (T11 I1): an answer names the run, the unit and the question; a second
/// answer, a wrong id or a wrong unit is refused and changes nothing.
#[tokio::test(start_paused = true)]
async fn answers_are_checked_against_the_question() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    let asked = nth_commit(&t.o, 1).await;
    let refused = |seq, unit, question| {
        let rx = h.commit_decision(seq, unit, question, CommitAnswer::Commit);
        async move { !rx.await.unwrap_or(false) }
    };
    assert!(refused(t.ticket.seq + 1, asked.unit, asked.question).await);
    assert!(refused(t.ticket.seq, asked.unit, asked.question + 1).await);
    let mut other = asked.unit;
    other.ordinal += 1;
    assert!(refused(t.ticket.seq, other, asked.question).await);
    assert_eq!(
        sent(&f, before),
        [st("BEGIN"), q(UPDATE)],
        "nothing decided"
    );
    assert!(
        h.commit_decision(
            t.ticket.seq,
            asked.unit,
            asked.question,
            CommitAnswer::RollBack
        )
        .await
        .unwrap()
    );
    assert!(
        refused(t.ticket.seq, asked.unit, asked.question).await,
        "the double click"
    );
    until_ended(&t.o).await;
    assert_eq!(decided(&t.o), [(CommitAnswer::RollBack, false)]);
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
}

/// Fail closed: a wrapped unit run "as one" (its own statements could end the wrapping
/// transaction) and a wrapped unit on a read-only session are refused, nothing sent.
#[tokio::test(start_paused = true)]
async fn wrapped_units_run_as_one_or_read_only_are_refused() {
    for verdict in [
        UnitVerdict::TxThenConfirm,
        UnitVerdict::ExplainAnalyzeRollback,
    ] {
        let (h, f) = session(fake_pg().tracks_transactions()).await;
        let before = f.call_log().len();
        let mut u = verdict_unit(0, "UPDATE t SET a=1; COMMIT", verdict);
        u.protocol = Protocol::Simple;
        let t = run_units(&h, vec![u], RunSpec::CONFIRM_DEADLINE);
        assert_eq!(
            t.ticket.reply.await.unwrap().unwrap_err().kind,
            ExecErrorKind::Internal,
            "{verdict:?}"
        );
        assert!(sent(&f, before).is_empty(), "{verdict:?}");

        let (h, f) = ro_session(fake_pg().tracks_transactions(), ro_profile()).await;
        let before = mark(&h, &f).await;
        let t = run_verdict(&h, UPDATE, verdict);
        assert_eq!(
            t.ticket.reply.await.unwrap().unwrap_err().kind,
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{verdict:?}"
        );
        assert!(sent(&f, before).is_empty(), "{verdict:?}");
    }
}

// ## m3b-t19-confirm-tx (fix round 1)

/// C2: an explicit Undo is no success, so the error policy decides. `StopOnError` stops (the
/// next unit is never sent); `Ask` asks, and `Ignore` goes on.
#[tokio::test(start_paused = true)]
async fn an_undo_follows_the_error_policy() {
    let (h, f) = session(fake_pg().tracks_transactions()).await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, UPDATE, UnitVerdict::TxThenConfirm),
            verdict_unit(1, "SELECT 1", UnitVerdict::Read),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    nth_commit(&t.o, 1).await;
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE)], "unit 2 waits");
    answer_rollback(&h, &t).await;
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::ConfirmUndone)
    );
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE), st("ROLLBACK")]);
    assert_eq!(t.o.unit_ends()[1], UnitEnd::Skipped);

    // Ask: the run asks, and Ignore goes on.
    let before = f.call_log().len();
    let o = Recorder::new();
    let spec = RunSpec {
        policy: ErrorPolicy::Ask,
        ..spec_units(
            &o,
            vec![
                verdict_unit(0, UPDATE, UnitVerdict::TxThenConfirm),
                verdict_unit(1, "SELECT 1", UnitVerdict::Read),
            ],
        )
    };
    let ticket = h.run(spec);
    let t = Ran { o, ticket };
    assert!(answer_nth(&h, &t, 1, CommitAnswer::RollBack).await);
    let question = nth_question(&t.o, 1).await;
    assert_eq!(
        question.error.kind,
        ExecErrorKind::Tx(TxError::ConfirmUndone)
    );
    assert!(
        h.choose(t.ticket.seq, question.question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    until_ended(&t.o).await;
    assert_eq!(
        sent(&f, before),
        [st("BEGIN"), q(UPDATE), st("ROLLBACK"), q("SELECT 1")]
    );
}

/// C2 (the review's data-loss path): the deadline rolls back the copy, and the pre-confirmed
/// `DROP` after it is never sent, whatever the policy (`Ask` asks nothing: the user is away).
#[tokio::test(start_paused = true)]
async fn the_deadline_stops_the_run() {
    const COPY: &str = "INSERT INTO new_t SELECT * FROM old_t";
    for policy in [ErrorPolicy::StopOnError, ErrorPolicy::Ask] {
        let (h, f) = session(fake_pg().tracks_transactions()).await;
        let before = f.call_log().len();
        let o = Recorder::new();
        let spec = RunSpec {
            policy,
            confirm_deadline: secs(2),
            ..spec_units(
                &o,
                vec![
                    verdict_unit(0, COPY, UnitVerdict::TxThenConfirm),
                    verdict_unit(1, "DROP TABLE old_t", UnitVerdict::Write),
                ],
            )
        };
        let ticket = h.run(spec);
        let t = Ran { o, ticket };
        nth_commit(&t.o, 1).await;
        tokio::time::sleep(secs(3)).await;
        until_ended(&t.o).await;
        assert_eq!(
            sent(&f, before),
            [st("BEGIN"), q(COPY), st("ROLLBACK")],
            "{policy:?}: the DROP is never sent"
        );
        assert!(asked(&t.o).is_empty(), "{policy:?}: nobody is asked");
        assert_eq!(
            t.o.unit_ends(),
            [
                UnitEnd::Failed(ExecError::of(ExecErrorKind::Tx(TxError::ConfirmTimedOut))),
                UnitEnd::Skipped
            ],
            "{policy:?}"
        );
    }
}

/// C1: a Manual MySQL session whose script turned `autocommit` back on: the savepoint is a
/// no-op outside a transaction and the statement commits on its own. Nothing is asked, no
/// `ROLLBACK TO` goes out, and the run stops (also under `Ask`).
#[tokio::test(start_paused = true)]
async fn an_already_committed_change_is_never_offered_keep_or_undo() {
    let (h, f) = session(fake(DbmsKind::MySql)).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = mark(&h, &f).await;
    let mut set = verdict_unit(0, "SET autocommit = 1", UnitVerdict::ImplicitCommit);
    set.effects = SessionEffects::COMMITS_FIRST;
    let o = Recorder::new();
    let spec = RunSpec {
        policy: ErrorPolicy::Ask,
        ..spec_units(
            &o,
            vec![
                set,
                verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
                verdict_unit(2, "UPDATE u SET b=2", UnitVerdict::Write),
            ],
        )
    };
    let ticket = h.run(spec);
    let e = ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Tx(TxError::ConfirmAlreadyCommitted));
    assert_eq!(
        sent(&f, before),
        [
            q("SET autocommit = 1"),
            st("SAVEPOINT hv_confirm"),
            q(UPDATE)
        ],
        "never a ROLLBACK TO, and unit 3 is not sent"
    );
    assert!(commit_questions(&o).is_empty());
    assert!(asked(&o).is_empty());
    assert_eq!(o.unit_ends()[2], UnitEnd::Skipped);

    // The own form whose statement ended the transaction itself is reported the same way.
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("UPDATE", TxStatus::Idle)]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::ConfirmAlreadyCommitted)
    );
    assert_eq!(sent(&f, before), [st("BEGIN"), q(UPDATE)]);
    assert!(commit_questions(&t.o).is_empty());
    assert_eq!(snap(&h).tx.owner, TxOwner::None);
}

/// Whether `calls` hold an `execute`/`batch` starting with `prefix`.
fn sends(calls: &[FakeCall], prefix: &str) -> bool {
    calls
        .iter()
        .any(|c| matches!(c, FakeCall::Statement(s) if s.starts_with(prefix)))
}

/// I1: the server rolled back the whole transaction with the failed statement (a SQLite
/// interrupt, a MySQL deadlock 1213): no `ROLLBACK TO` is sent, `TxChanged` says it ended, and
/// the session stays usable.
#[tokio::test(start_paused = true)]
async fn the_server_rolling_back_the_whole_transaction_is_reported_not_undone() {
    // SQLite: a cancel interrupts the statement inside the savepoint.
    let slow = FakeQuery {
        first_delay: secs(30),
        tx_after_error: Some(TxStatus::Idle),
        ..answering("UPDATE")
    };
    let (h, f) =
        session(fake(DbmsKind::Sqlite).queries(vec![leaves("BEGIN", TxStatus::InTx), slow])).await;
    let before = mark(&h, &f).await;
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    settle_until("the confirmed statement runs", || {
        f.call_log()[before..].contains(&q(UPDATE))
    })
    .await;
    drop(h.cancel(t.ticket.seq));
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    let calls = sent(&f, before);
    assert!(
        !sends(&calls, "ROLLBACK") && !sends(&calls, "RELEASE"),
        "{calls:?}"
    );
    assert!(t.o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::InTx,
        to: TxStatus::Idle
    }));
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).state, SessionState::Idle, "the session stays");
    run_verdict(&h, "SELECT 1", UnitVerdict::Read)
        .ticket
        .reply
        .await
        .unwrap()
        .expect("the session is usable");

    // MySQL: a deadlock (1213, SQLSTATE 40001) rolls back the whole InnoDB transaction.
    let deadlock = FakeQuery {
        fail_after: Some((0, db("40001"))),
        tx_after_error: Some(TxStatus::Idle),
        ..answering("UPDATE")
    };
    let (h, f) = session(
        fake(DbmsKind::MySql).queries(vec![leaves("START TRANSACTION", TxStatus::InTx), deadlock]),
    )
    .await;
    let before = mark(&h, &f).await;
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "START TRANSACTION", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db, "the server's error is kept");
    let calls = sent(&f, before);
    assert!(!sends(&calls, "ROLLBACK"), "{calls:?}");
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).state, SessionState::Idle);

    // The own form: Hacivat's transaction is already gone; no `ROLLBACK` is sent.
    let gone = FakeQuery {
        fail_after: Some((0, db("40001"))),
        tx_after_error: Some(TxStatus::Idle),
        ..answering("UPDATE")
    };
    let (h, f) = session(fake(DbmsKind::MySql).queries(vec![gone])).await;
    let before = mark(&h, &f).await;
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Db
    );
    assert!(!sends(&sent(&f, before), "ROLLBACK"));
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::Idle, TxOwner::None));
}

/// m1: a `RELEASE` the server refuses after a `ROLLBACK TO` that worked (or after Keep) is
/// logged and the session stays; the statement was undone (or kept).
#[tokio::test(start_paused = true)]
async fn a_refused_release_keeps_the_session() {
    for answer in [CommitAnswer::RollBack, CommitAnswer::Commit] {
        let (h, _f) = session(
            fake_pg()
                .tracks_transactions()
                .exec(vec![tx_err("RELEASE", "3B001", TxStatus::InTx)])
                .queries(vec![leaves("BEGIN", TxStatus::InTx)]),
        )
        .await;
        let t = run_units(
            &h,
            vec![
                verdict_unit(0, "BEGIN", UnitVerdict::Write),
                verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
            ],
            RunSpec::CONFIRM_DEADLINE,
        );
        assert!(answer_nth(&h, &t, 1, answer).await);
        until_ended(&t.o).await;
        assert_eq!(decided(&t.o), [(answer, false)], "{answer:?}");
        let _ = t.ticket.reply.await.unwrap();
        assert_eq!(snap(&h).state, SessionState::Idle, "{answer:?}: not broken");
        assert_eq!(snap(&h).tx.status, TxStatus::InTx, "{answer:?}");
    }
}

/// m3: a stop asked for while the statement ran to its end: no question, the statement is
/// rolled back.
#[tokio::test(start_paused = true)]
async fn no_question_after_a_stop() {
    let stubborn = FakeQuery {
        first_delay: secs(30),
        ignores_cancel: true,
        ..affects("UPDATE", 1)
    };
    let (h, f) = session(fake_pg().tracks_transactions().queries(vec![stubborn])).await;
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    settle_until("the confirmed statement runs", || {
        f.call_log()[before..].contains(&q(UPDATE))
    })
    .await;
    drop(h.cancel(t.ticket.seq));
    let _ = t.ticket.reply.await.unwrap();
    until_ended(&t.o).await;
    assert!(commit_questions(&t.o).is_empty(), "never asked");
    let calls = sent(&f, before);
    assert_eq!(calls.last(), Some(&st("ROLLBACK")), "{calls:?}");
    assert!(!calls.contains(&st("COMMIT")));
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

/// m5: MySQL table locks: Hacivat's own `START TRANSACTION` (and Smart's) would release them,
/// so the unit is refused, nothing sent; after `UNLOCK TABLES` it runs.
#[tokio::test(start_paused = true)]
async fn mysql_table_locks_refuse_hacivats_own_transaction() {
    let (h, f) = session(fake(DbmsKind::MySql)).await;
    run(&h, "LOCK TABLES t WRITE").await.unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::TableLocksHeld)
    );
    assert!(sent(&f, before).is_empty(), "nothing sent");

    tx_cmd(&h, TxCommand::SetMode(TxMode::Smart)).await.unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, "UPDATE u SET b=2", UnitVerdict::Write);
    assert_eq!(
        t.ticket.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Tx(TxError::TableLocksHeld)
    );
    assert!(sent(&f, before).is_empty(), "Smart sends nothing either");

    // `UNLOCK TABLES` (a run unit; Smart does not open before it) releases them.
    let t = run_verdict(&h, "UNLOCK TABLES", UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, "UPDATE u SET b=2", UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(
        sent(&f, before),
        [st("START TRANSACTION"), q("UPDATE u SET b=2")]
    );
}

// ## m3b-t19-confirm-tx (fix round 2)

/// R1-m1: table locks are seen past leading and executable comments, and only `UNLOCK
/// TABLE[S]`, `START TRANSACTION`/`BEGIN`, a reset or a reconnect release them (`START
/// REPLICA` and `UNLOCK INSTANCE` do not).
#[tokio::test(start_paused = true)]
async fn table_locks_are_read_past_comments_and_released_only_by_their_statements() {
    // The fake reads any leading START as a transaction start; the server does not.
    let (h, f) =
        session(fake(DbmsKind::MySql).exec(vec![tx("START REPLICA", TxStatus::Idle)])).await;
    run(&h, "/*!40000 LOCK TABLES t WRITE */").await.unwrap();
    for releases_nothing in ["START REPLICA", "UNLOCK INSTANCE"] {
        run(&h, releases_nothing).await.unwrap();
        let before = f.call_log().len();
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        assert_eq!(
            t.ticket.reply.await.unwrap().unwrap_err().kind,
            ExecErrorKind::Tx(TxError::TableLocksHeld),
            "after {releases_nothing}"
        );
        assert!(sent(&f, before).is_empty());
    }
    run(&h, "-- done\nUNLOCK TABLES").await.unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
    assert!(answer_nth(&h, &t, 1, CommitAnswer::Commit).await);
    until_ended(&t.o).await;
    assert_eq!(
        sent(&f, before),
        [st("START TRANSACTION"), q(UPDATE), st("COMMIT")]
    );
}

/// R1-m2: Keep whose `RELEASE` the server refused and left the PostgreSQL transaction failed:
/// the statement is not reported kept (no `CommitDecided`), the unit fails with the server's
/// text, the snapshot shows the failed transaction, and the session stays.
#[tokio::test(start_paused = true)]
async fn keep_with_a_refused_release_that_fails_the_transaction_is_no_success() {
    let (h, _f) = session(
        fake_pg()
            .tracks_transactions()
            .exec(vec![tx_err("RELEASE", "XX000", TxStatus::Failed)])
            .queries(vec![leaves("BEGIN", TxStatus::InTx)]),
    )
    .await;
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, UPDATE, UnitVerdict::TxThenConfirm),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    assert!(answer_nth(&h, &t, 1, CommitAnswer::Commit).await);
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert!(e.server.is_some(), "the server's text");
    assert!(decided(&t.o).is_empty(), "never reported kept");
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    assert_eq!(snap(&h).state, SessionState::Idle, "not broken");
}
