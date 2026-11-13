//! M2b Task 7: transaction state and modes (HV-SESS-03, HV-EXEC-09/-10, B22, Q14).

mod common;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{AccessLabel, DbmsKind, EnvTag};
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ExecErrorKind, ExecOrigin, IsolationLevel, Request, TxCommand, TxError, TxMode, begin_sql,
    default_mode,
};

#[tokio::test]
async fn tx_state_follows_protocol_status() {
    let (h, _) = session(fake_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        tx_err("SELECT 1/0", "22012", TxStatus::Failed),
    ]))
    .await;
    run(&h, "BEGIN").await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    assert!(snap(&h).tx.since.is_some());
    let _ = run(&h, "SELECT 1/0").await;
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
}

#[tokio::test]
async fn manual_mode_begins_before_next_statement() {
    let (h, f) = session(fake_pg()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    run(&h, "update t set a = 1").await.unwrap();
    assert_eq!(f.executed(), ["BEGIN", "update t set a = 1"]);
}

#[tokio::test]
async fn manual_mode_uses_the_isolation_level() {
    let (h, f) = session(fake_pg()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    tx_cmd(&h, TxCommand::SetIsolation(IsolationLevel::Serializable))
        .await
        .unwrap();
    run(&h, "update t set a = 1").await.unwrap();
    assert_eq!(
        f.executed(),
        ["BEGIN ISOLATION LEVEL SERIALIZABLE", "update t set a = 1"]
    );
}

#[tokio::test]
async fn sqlite_manual_begins_immediate_unless_the_statement_reads() {
    let (h, f) = session(FakeDriver::new(DbmsKind::Sqlite).exec(vec![
        tx("BEGIN IMMEDIATE", TxStatus::InTx),
        tx("COMMIT", TxStatus::Idle),
    ]))
    .await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    run(&h, "insert into t values (1)").await.unwrap();
    tx_cmd(&h, TxCommand::Commit).await.unwrap();
    run(&h, "  with x as (select 1) select * from x")
        .await
        .unwrap();
    let ran = f.executed();
    assert_eq!(ran[..2], ["BEGIN IMMEDIATE", "insert into t values (1)"]);
    assert!(ran.contains(&"COMMIT".to_string()));
    let tail = &ran[ran.len() - 2..];
    assert_eq!(tail, ["BEGIN", "  with x as (select 1) select * from x"]);
}

#[tokio::test]
async fn mysql_manual_uses_autocommit() {
    let (h, f) = session(fake_mysql()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    assert_eq!(f.executed(), ["SET autocommit = 0"]);
    tx_cmd(&h, TxCommand::SetMode(TxMode::Auto)).await.unwrap();
    assert_eq!(f.executed(), ["SET autocommit = 0", "SET autocommit = 1"]);
}

#[tokio::test]
async fn a_mysql_profile_that_starts_manual_turns_autocommit_off() {
    // The UI must never show Manual while the server autocommits.
    let profile = Profile {
        tx_mode: Some(TxMode::Manual),
        ..mysql_profile(hacivat_net::TlsMode::Disable)
    };
    let side = std::sync::Arc::new(fake_mysql());
    let (h, f) = spawn_over(fake_mysql(), profile, false, side).await;
    assert_eq!(snap(&h).tx.mode, TxMode::Manual);
    run(&h, "select 1").await.unwrap();
    assert_eq!(f.executed(), ["SET autocommit = 0", "select 1"]);
}

#[tokio::test]
async fn manual_to_auto_in_transaction_is_refused() {
    let (h, _) = in_tx_session().await;
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Auto))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::InProgress)
    );
}

#[tokio::test]
async fn smart_mode_needs_classifier() {
    let (h, _) = session(fake_pg()).await;
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Smart))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::NeedsClassifier)
    );
}

#[tokio::test]
async fn a_failed_transaction_takes_only_a_rollback() {
    let (h, f) = session(fake_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        tx_err("SELECT 1/0", "22012", TxStatus::Failed),
        // The fake matches case-sensitively; the advisory check folds case.
        tx("rollback", TxStatus::Idle),
    ]))
    .await;
    run(&h, "BEGIN").await.unwrap();
    let _ = run(&h, "SELECT 1/0").await;
    assert_eq!(
        run(&h, "select 2").await.unwrap_err().kind,
        ExecErrorKind::Tx(TxError::FailedNeedsRollback)
    );
    assert!(!f.executed().contains(&"select 2".to_string()));
    run(&h, "  rollback").await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

#[tokio::test]
async fn begin_twice_is_in_progress() {
    let (h, f) = session(fake_pg().exec(vec![tx("BEGIN", TxStatus::InTx)])).await;
    tx_cmd(&h, TxCommand::Begin).await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    assert_eq!(
        tx_cmd(&h, TxCommand::Begin).await.unwrap_err().kind,
        ExecErrorKind::Tx(TxError::InProgress)
    );
    assert_eq!(f.executed(), ["BEGIN"]);
}

#[tokio::test]
async fn read_only_session_is_always_auto() {
    // Q14
    let (h, _) = ro_session(fake_pg(), production_profile()).await;
    assert_eq!(snap(&h).tx.mode, TxMode::Auto);
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::Unsupported)
    );
    assert_eq!(
        tx_cmd(&h, TxCommand::Begin).await.unwrap_err().kind,
        ExecErrorKind::Tx(TxError::Unsupported)
    );
}

#[tokio::test]
async fn a_production_read_write_session_starts_manual() {
    let (h, _) = spawn_over(
        fake_pg(),
        production_profile(),
        false,
        std::sync::Arc::new(fake_pg()),
    )
    .await;
    assert_eq!(snap(&h).tx.mode, TxMode::Manual);
}

#[tokio::test]
async fn a_read_only_profile_never_yields_a_read_write_session() {
    // Fail safe: the session is read-only when its spec or its profile says so.
    let (h, f) = spawn_over(
        fake_pg(),
        ro_profile(),
        false,
        std::sync::Arc::new(fake_pg()),
    )
    .await;
    assert!(snap(&h).read_only);
    // Review M4: the label follows the session, not the connect.
    assert_eq!(snap(&h).access, AccessLabel::ReadOnlyGuarded);
    // Review M8: no manual transaction control on a read-only session (Q14).
    for cmd in [TxCommand::Commit, TxCommand::Rollback] {
        assert_eq!(
            tx_cmd(&h, cmd).await.unwrap_err().kind,
            ExecErrorKind::Tx(TxError::Unsupported)
        );
    }
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::Unsupported)
    );
    // Task 18: the statement runs only inside the guard's read-only transaction; this fake
    // never opens one, so the guard reports a gap, never sends the statement (fix round 1,
    // m-1) and discards the session.
    assert_eq!(
        run(&h, "delete from t").await.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(hacivat_session::GapReason::TransactionEndedByStatement)
    );
    // After the connect's identity check (the profile is read-only, so the connect is too;
    // the fake answers nothing, so the probe stops there, guarded).
    assert_eq!(
        f.executed(),
        [
            "SELECT current_user, session_user",
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SELECT 1"
        ]
    );
    assert!(matches!(
        snap(&h).state,
        hacivat_session::session::SessionState::Broken(_)
    ));
}

#[tokio::test]
async fn a_read_only_statement_goes_through_the_guard() {
    // Task 18 (T7 carry): the read-only connect runs the role probe first (its identity check;
    // this fake answers nothing, so the probe stops there). Then the statement would run
    // inside the guard, but this fake never opens the guard's transaction: a gap, and the
    // statement is never sent.
    let (h, f) = ro_session(fake_pg(), ro_profile()).await;
    assert_eq!(f.executed(), ["SELECT current_user, session_user"]);
    assert_eq!(
        run(&h, "select 1").await.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(hacivat_session::GapReason::TransactionEndedByStatement)
    );
    assert_eq!(
        f.executed()[1..],
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SELECT 1"
        ]
    );
}

#[test]
fn defaults_and_begin_sql() {
    assert_eq!(default_mode(Some(EnvTag::Production)), TxMode::Manual);
    assert_eq!(default_mode(Some(EnvTag::Dev)), TxMode::Auto);
    assert_eq!(
        begin_sql(DbmsKind::Postgres, IsolationLevel::Serializable, false),
        ["BEGIN ISOLATION LEVEL SERIALIZABLE"]
    );
    assert_eq!(
        begin_sql(DbmsKind::MySql, IsolationLevel::ReadCommitted, false),
        [
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "START TRANSACTION"
        ]
    );
    assert_eq!(
        begin_sql(DbmsKind::MySql, IsolationLevel::Serializable, false),
        [
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "START TRANSACTION"
        ]
    );
    assert_eq!(
        begin_sql(DbmsKind::Sqlite, IsolationLevel::Serializable, false),
        ["BEGIN"]
    );
    assert_eq!(
        begin_sql(DbmsKind::Postgres, IsolationLevel::Default, false),
        ["BEGIN"]
    );
    assert_eq!(
        begin_sql(DbmsKind::Postgres, IsolationLevel::RepeatableRead, true),
        ["BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY"]
    );
    assert_eq!(
        begin_sql(DbmsKind::MariaDb, IsolationLevel::Default, true),
        ["START TRANSACTION READ ONLY"]
    );
}

// ## m2b-t07 fix round 1

/// Review M9: Hacivat's own statements never open the user's Manual transaction.
#[tokio::test]
async fn internal_statements_never_open_a_manual_transaction() {
    let (h, f) = session(fake_pg()).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    h.submit(Request::Exec {
        sql: "select 1".into(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: ExecOrigin::Internal { idempotent: true },
    })
    .reply
    .await
    .unwrap()
    .unwrap();
    assert_eq!(f.executed(), ["select 1"]);
}
