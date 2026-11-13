//! M2b Task 7: transaction state and modes (HV-SESS-03, HV-EXEC-09/-10, B22, Q14).

mod common;

use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::FakeDriver;
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ExecErrorKind, IsolationLevel, TxCommand, TxError, TxMode, begin_sql, default_mode,
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
    assert_eq!(
        tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
            .await
            .unwrap_err()
            .kind,
        ExecErrorKind::Tx(TxError::Unsupported)
    );
    assert_eq!(
        run(&h, "delete from t").await.unwrap_err().kind,
        ExecErrorKind::Internal
    );
    assert!(f.executed().is_empty(), "{:?}", f.executed());
}

#[tokio::test]
async fn a_read_only_statement_goes_through_the_guard() {
    // Until Task 18 the guard fails closed: nothing runs unguarded.
    let (h, f) = ro_session(fake_pg(), ro_profile()).await;
    assert_eq!(
        run(&h, "select 1").await.unwrap_err().kind,
        ExecErrorKind::Internal
    );
    assert!(f.executed().is_empty(), "{:?}", f.executed());
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
