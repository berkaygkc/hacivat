//! M3b Task 19: the D-062 wrap in the session (R-M3-21, R-M3-55). An `EXPLAIN ANALYZE` whose
//! inner statement is not a read (`UnitVerdict::ExplainAnalyzeRollback`) runs inside `BEGIN …
//! ROLLBACK` when no transaction is open (`START TRANSACTION … ROLLBACK` on MySQL), or inside
//! `SAVEPOINT hv_explain … ROLLBACK TO SAVEPOINT hv_explain; RELEASE SAVEPOINT hv_explain` in
//! the user's transaction, which stays open and theirs. The rollback runs whatever the
//! statement's outcome, and nothing is asked. FakeDriver and a paused clock, asserted on
//! `call_log()`.

mod common;

use common::confirm::*;
use common::runs_m3b::{failing, one_row};
use common::*;
use hacivat_driver::TxStatus;
use hacivat_driver::fake::{FakeDriver, FakeQuery};
use hacivat_proto::DbmsKind;
use hacivat_results::Completeness;
use hacivat_session::session::{
    ExecErrorKind, IsolationLevel, RunEvent, RunSpec, TxCommand, TxMode, TxOwner, UnitVerdict,
};

const EXPLAIN: &str = "EXPLAIN ANALYZE UPDATE t SET a=1 WHERE id=3";

/// The plan's rows for the explain.
fn plan_rows() -> FakeQuery {
    FakeQuery {
        prefix: "EXPLAIN",
        sets: vec![one_row(1)],
        ..FakeQuery::default()
    }
}

fn fake(kind: DbmsKind) -> FakeDriver {
    FakeDriver::new(kind)
        .tracks_transactions()
        .queries(vec![plan_rows()])
}

#[tokio::test(start_paused = true)]
async fn explain_analyze_is_wrapped_in_a_rollback() {
    for kind in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::Sqlite] {
        let (h, f) = session(fake(kind)).await;
        tx_cmd(&h, TxCommand::SetIsolation(IsolationLevel::Default))
            .await
            .unwrap();
        let before = f.call_log().len();
        let t = run_verdict(&h, EXPLAIN, UnitVerdict::ExplainAnalyzeRollback);
        t.ticket.reply.await.unwrap().unwrap();
        let begin = match kind {
            DbmsKind::MySql => "START TRANSACTION",
            _ => "BEGIN",
        };
        assert_eq!(
            sent(&f, before),
            [st(begin), q(EXPLAIN), st("ROLLBACK")],
            "{kind:?}"
        );
        let tx = snap(&h).tx;
        assert_eq!((tx.status, tx.owner), (TxStatus::Idle, TxOwner::None));
        assert!(commit_questions(&t.o).is_empty(), "{kind:?}: never asked");
        assert!(decided(&t.o).is_empty());
        // The plan's rows are complete, never a suspended cursor inside the wrap.
        assert_eq!(t.o.rows(), 1, "{kind:?}");
        assert_eq!(t.o.result_ends(), [Completeness::Complete], "{kind:?}");
        assert_eq!(snap(&h).cursor, None);
    }
}

/// Inside the user's transaction: a savepoint of its own, rolled back to and released; the
/// transaction stays open and the user's.
#[tokio::test(start_paused = true)]
async fn explain_inside_a_user_tx_rolls_back_to_its_savepoint() {
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![leaves("BEGIN", TxStatus::InTx), plan_rows()]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_units(
        &h,
        vec![
            verdict_unit(0, "BEGIN", UnitVerdict::Write),
            verdict_unit(1, EXPLAIN, UnitVerdict::ExplainAnalyzeRollback),
        ],
        RunSpec::CONFIRM_DEADLINE,
    );
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(
        sent(&f, before),
        [
            q("BEGIN"),
            st("SAVEPOINT hv_explain"),
            q(EXPLAIN),
            st("ROLLBACK TO SAVEPOINT hv_explain"),
            st("RELEASE SAVEPOINT hv_explain"),
        ]
    );
    let tx = snap(&h).tx;
    assert_eq!((tx.status, tx.owner), (TxStatus::InTx, TxOwner::User));
}

/// A Manual session opens the user's transaction first (as for any statement), then the wrap
/// takes the savepoint form.
#[tokio::test(start_paused = true)]
async fn explain_in_manual_mode_uses_the_savepoint_form() {
    let (h, f) = session(fake(DbmsKind::Postgres)).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Manual))
        .await
        .unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, EXPLAIN, UnitVerdict::ExplainAnalyzeRollback);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(
        sent(&f, before),
        [
            st("BEGIN"),
            st("SAVEPOINT hv_explain"),
            q(EXPLAIN),
            st("ROLLBACK TO SAVEPOINT hv_explain"),
            st("RELEASE SAVEPOINT hv_explain"),
        ]
    );
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
}

/// A failing `EXPLAIN ANALYZE` is rolled back too (its statement may have written before it
/// failed); the unit keeps the server's error.
#[tokio::test(start_paused = true)]
async fn a_failing_explain_is_rolled_back_too() {
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![failing("EXPLAIN")]),
    )
    .await;
    let before = f.call_log().len();
    let t = run_verdict(&h, EXPLAIN, UnitVerdict::ExplainAnalyzeRollback);
    let e = t.ticket.reply.await.unwrap().unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert_eq!(
        sent(&f, before),
        [st("BEGIN"), q(EXPLAIN), closed_now(), st("ROLLBACK")]
    );
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

/// Smart does not open the user's transaction before an `EXPLAIN ANALYZE`: the wrap's own
/// rollback undoes it, and no empty transaction is left open.
#[tokio::test(start_paused = true)]
async fn smart_leaves_the_explain_to_its_wrap() {
    let (h, f) = session(fake(DbmsKind::Postgres)).await;
    tx_cmd(&h, TxCommand::SetMode(TxMode::Smart)).await.unwrap();
    let before = f.call_log().len();
    let t = run_verdict(&h, EXPLAIN, UnitVerdict::ExplainAnalyzeRollback);
    t.ticket.reply.await.unwrap().unwrap();
    assert_eq!(sent(&f, before), [st("BEGIN"), q(EXPLAIN), st("ROLLBACK")]);
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert!(
        !t.o.events()
            .iter()
            .any(|e| matches!(e, RunEvent::NeedsCommit { .. }))
    );
}
