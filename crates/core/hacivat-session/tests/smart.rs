//! M3b Task 19: Smart mode (C2; HV-EXEC-10, D-036). Auto, but before a unit whose verdict is
//! not `Read` while no transaction is open the session opens the user's transaction; a `Read`
//! never opens one, a unit without a verdict does (the safe direction); after the user's
//! `COMMIT` the mode stays Smart; a confirmed write takes the savepoint form (P-M3b-1). FakeDriver
//! and a paused clock, asserted on `call_log()`.

mod common;

use common::confirm::*;
use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{
    IsolationLevel, RunSpec, SessionEffects, SessionHandle, TxCommand, TxMode, TxOwner, UnitVerdict,
};

const UPDATE: &str = "UPDATE t SET a=1 WHERE id=3";

fn fake(kind: DbmsKind) -> FakeDriver {
    FakeDriver::new(kind).tracks_transactions()
}

/// A Smart session over `fake` and a mark in its log after the mode switch.
async fn smart(fake: FakeDriver) -> (SessionHandle, std::sync::Arc<FakeDriver>, usize) {
    let (h, f) = session(fake).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Smart))
        .await
        .expect("Smart is accepted since Task 19");
    tx_cmd(&h, TxCommand::SetIsolation(IsolationLevel::Default))
        .await
        .unwrap();
    let mark = f.call_log().len();
    (h, f, mark)
}

/// P-M3b-1: Smart opens the user's transaction before the confirmed write, which therefore
/// takes the savepoint form (Keep/Undo); the transaction stays the user's after Keep.
#[tokio::test(start_paused = true)]
async fn smart_write_uses_the_savepoint_form() {
    for kind in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::Sqlite] {
        let (h, f, before) = smart(fake(kind)).await;
        let t = run_verdict(&h, UPDATE, UnitVerdict::TxThenConfirm);
        assert!(nth_commit(&t.o, 1).await.savepoint, "{kind:?}");
        answer_commit(&h, &t).await;
        let begin = match kind {
            DbmsKind::Postgres => "BEGIN",
            DbmsKind::MySql => "START TRANSACTION",
            _ => "BEGIN IMMEDIATE",
        };
        assert_eq!(
            sent(&f, before),
            [
                st(begin),
                st("SAVEPOINT hv_confirm"),
                q(UPDATE),
                st("RELEASE SAVEPOINT hv_confirm")
            ],
            "{kind:?}"
        );
        let tx = snap(&h).tx;
        assert_eq!(
            (tx.status, tx.owner, tx.mode),
            (TxStatus::InTx, TxOwner::User, TxMode::Smart),
            "{kind:?}"
        );
        t.ticket.reply.await.unwrap().unwrap();
    }
}

/// C2: a `Read` never opens a transaction; a write opens one first; the user's `COMMIT` ends
/// it, the mode stays Smart, and the next write opens again.
#[tokio::test(start_paused = true)]
async fn smart_opens_before_write_only() {
    let (h, f, before) = smart(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("COMMIT", TxStatus::Idle)]),
    )
    .await;
    let t = run_verdict(&h, "SELECT 1", UnitVerdict::Read);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [q("SELECT 1")], "a read opens nothing");
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);

    let mark = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, mark), [st("BEGIN"), q(UPDATE)]);
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));

    // A read inside the open transaction opens nothing more; a second write neither.
    let mark = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "SELECT 2", UnitVerdict::Read),
            verdict_unit(1, "UPDATE u SET b=2", UnitVerdict::Write),
            verdict_unit(2, "COMMIT", UnitVerdict::Write),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(
        sent(&f, mark),
        [q("SELECT 2"), q("UPDATE u SET b=2"), q("COMMIT")]
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.mode), (TxStatus::Idle, TxMode::Smart));

    // The next write opens again.
    let mark = f.call_log().len();
    let t = run_verdict(&h, UPDATE, UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, mark), [st("BEGIN"), q(UPDATE)]);
}

/// Without a verdict (`Unknown`: every M3a run) Smart opens a transaction, the safe direction.
#[tokio::test(start_paused = true)]
async fn smart_without_a_verdict_opens_a_tx() {
    let (h, f, before) = smart(fake_pg().tracks_transactions()).await;
    let t = run_verdict(&h, "SELECT f()", UnitVerdict::Unknown);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [st("BEGIN"), q("SELECT f()")]);
    assert_eq!(snap(&h).tx.owner, TxOwner::User);
}

/// The user's own transaction control never gets Smart's `BEGIN` before it.
#[tokio::test(start_paused = true)]
async fn smart_never_opens_before_the_users_transaction_control() {
    let (h, f, before) = smart(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx)]),
    )
    .await;
    let t = run_verdict(&h, "BEGIN", UnitVerdict::Write);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [q("BEGIN")]);
}

/// MySQL: Smart does not open before a statement that commits an open transaction first (it
/// would end the new one at once, and the implicit-commit backstop would refuse the statement).
#[tokio::test(start_paused = true)]
async fn smart_never_opens_before_a_mysql_implicit_commit_statement() {
    let (h, f, before) = smart(fake(DbmsKind::MySql)).await;
    let mut ddl = verdict_unit(0, "CREATE TABLE x (a int)", UnitVerdict::Write);
    ddl.effects = SessionEffects::DDL | SessionEffects::COMMITS_FIRST;
    let t = run_units(&h, vec![ddl], RunSpec::CONFIRM_DEADLINE);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [q("CREATE TABLE x (a int)")]);
}

/// `SetMode(Smart)` is accepted (it answered `NeedsClassifier` before Task 19) like Auto: not
/// while a transaction is open, and a Manual MySQL session turns `autocommit` back on; a
/// profile's Smart mode starts as Smart.
#[tokio::test(start_paused = true)]
async fn set_mode_smart_is_accepted_and_a_smart_profile_starts_smart() {
    let (h, f) = session(fake(DbmsKind::MySql)).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = f.call_log().len();
    tx_cmd(&h, TxCommand::SetMode(TxMode::Smart)).await.unwrap();
    assert_eq!(sent(&f, before), [st("SET autocommit = 1")]);
    assert_eq!(snap(&h).tx.mode, TxMode::Smart);

    let (h, _f) = in_tx_session().await;
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Smart))
            .await
            .unwrap_err()
            .kind,
        hacivat_session::session::ExecErrorKind::Tx(hacivat_session::session::TxError::InProgress)
    );

    let profile = hacivat_session::profile::Profile {
        tx_mode: Some(TxMode::Smart),
        ..pg_profile(hacivat_net::TlsMode::Disable)
    };
    let side = std::sync::Arc::new(FakeDriver::new(DbmsKind::Postgres));
    let (h, _f) = spawn_over(fake_pg(), profile, false, side).await;
    assert_eq!(snap(&h).tx.mode, TxMode::Smart);
}

/// `Request::Exec` of the user carries no verdict: Smart opens before it (never before the
/// user's transaction control, never on MySQL inside an open one).
#[tokio::test(start_paused = true)]
async fn smart_opens_before_a_user_exec() {
    let (h, f, before) = smart(fake_pg().tracks_transactions()).await;
    run(&h, "INSERT INTO t VALUES (1)").await.unwrap();
    assert_eq!(
        sent(&f, before),
        [st("BEGIN"), st("INSERT INTO t VALUES (1)")]
    );
    run(&h, "COMMIT").await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert_eq!(snap(&h).tx.mode, TxMode::Smart);
}
