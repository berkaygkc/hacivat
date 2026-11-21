//! M3b Task 19 (fix round 1): the Output-log lines and error titles of confirm-in-transaction
//! exist in English and Turkish (the `exec-confirm-*`, `exec-implicit-commit` lines Task 16
//! left to Task 19, and the round-1 additions), and a statement that failed while the server
//! rolled back the user's whole transaction says so in the log (I1).

mod common;

use std::collections::HashSet;

use common::m3b::{request, run_all, units};
use common::{fake_pg, generated, pg_error, service_over};
use hacivat_driver::TxStatus;
use hacivat_driver::fake::FakeQuery;
use hacivat_exec::{LogMsg, error_title_id};
use hacivat_session::session::{ExecErrorKind, TxError};

/// The message IDs of `locale`'s `hacivat-core.ftl`.
fn ftl_ids(locale: &str) -> HashSet<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../locales")
        .join(locale)
        .join("hacivat-core.ftl");
    let text = std::fs::read_to_string(&path).expect("the FTL file");
    text.lines()
        .filter_map(|l| {
            let (id, _) = l.split_once(" =")?;
            (!id.starts_with(['#', ' ']) && !id.is_empty()).then(|| id.to_string())
        })
        .collect()
}

#[test]
fn every_confirm_line_and_error_title_has_its_strings() {
    let lines = [
        LogMsg::ConfirmCommitted { rows: None },
        LogMsg::ConfirmRolledBack {
            by_deadline: false,
            savepoint: false,
        },
        LogMsg::ConfirmRolledBack {
            by_deadline: true,
            savepoint: false,
        },
        LogMsg::ConfirmRolledBack {
            by_deadline: false,
            savepoint: true,
        },
        LogMsg::ConfirmStatementFailed,
        LogMsg::ImplicitCommit,
        LogMsg::TxRolledBackByServer,
        LogMsg::TxEndedByStatement,
    ];
    let titles = [
        TxError::ImplicitCommitUnconfirmed,
        TxError::ConfirmAlreadyCommitted,
        TxError::ConfirmTimedOut,
        TxError::ConfirmUndone,
        TxError::TableLocksHeld,
        TxError::ExplainNotRolledBack,
    ]
    .map(|e| error_title_id(&ExecErrorKind::Tx(e)));
    for locale in ["en", "tr"] {
        let ids = ftl_ids(locale);
        for msg in &lines {
            assert!(msg.args().is_empty(), "{msg:?} takes no argument");
            assert!(
                ids.contains(msg.fluent_id()),
                "{locale}: {}",
                msg.fluent_id()
            );
        }
        for id in titles {
            assert!(ids.contains(id), "{locale}: {id}");
        }
    }
}

/// I1: a statement fails and the server rolls back the user's whole transaction with it (a
/// MySQL deadlock, a SQLite interrupt): the log says so after the statement's error.
#[tokio::test]
async fn a_whole_transaction_rolled_back_by_the_server_is_logged() {
    let deadlock = FakeQuery {
        prefix: "UPDATE",
        sets: vec![generated(1)],
        fail_after: Some((0, pg_error("40P01", "deadlock detected"))),
        tx_after_error: Some(TxStatus::Idle),
        ..FakeQuery::default()
    };
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..FakeQuery::default()
    };
    let (b, s, _f) = service_over(fake_pg().queries(vec![begin, deadlock])).await;
    let mut t = b.svc.run(
        &s,
        request(&b, units(&["BEGIN", "UPDATE t SET a = 1"]), run_all(2)),
    );
    let summary = (&mut t.done).await.unwrap();
    assert_eq!(summary.failed, 1, "{:?} {:?}", summary, b.log_ids());
    let ids = b.log_ids();
    let at = ids
        .iter()
        .position(|id| *id == "exec-tx-rolled-back-by-server")
        .unwrap_or_else(|| panic!("{ids:?}"));
    assert_eq!(
        ids[at - 1],
        "exec-error",
        "after the statement's error: {ids:?}"
    );
}

// ## m3b-t24-exec-ui (fix round 1, I6.3): the lines the observer derives from the gate's verdict.

/// A MySQL statement confirmed although it commits the open transaction did commit it: the log
/// says `exec-implicit-commit`, not the generic "the transaction ended" line.
#[tokio::test]
async fn a_confirmed_implicit_commit_is_logged_as_one() {
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..FakeQuery::default()
    };
    let create = FakeQuery {
        prefix: "CREATE",
        tx_after: Some(TxStatus::Idle),
        ..FakeQuery::default()
    };
    let fake = hacivat_driver::fake::FakeDriver::new(hacivat_proto::DbmsKind::MySql)
        .tracks_transactions()
        .queries(vec![begin, create]);
    let (b, s, _f) = service_over(fake).await;
    let mut plan = run_all(2);
    plan.items[1].verdict = hacivat_session::session::UnitVerdict::ImplicitCommit;
    let mut t = b.svc.run(
        &s,
        request(&b, units(&["BEGIN", "CREATE TABLE x (a INT)"]), plan),
    );
    let summary = (&mut t.done).await.unwrap();
    assert_eq!(summary.failed, 0, "{summary:?} {:?}", b.log_ids());
    let ids = b.log_ids();
    assert!(ids.contains(&"exec-implicit-commit"), "{ids:?}");
}

/// A confirm-in-transaction statement that failed in the user's open transaction before any
/// question ran in the savepoint form, which rolled back only it: `exec-confirm-statement-failed`.
#[tokio::test]
async fn a_confirm_in_transaction_statement_that_failed_says_only_it_was_undone() {
    let begin = FakeQuery {
        prefix: "BEGIN",
        tx_after: Some(TxStatus::InTx),
        ..FakeQuery::default()
    };
    let fails = FakeQuery {
        prefix: "UPDATE",
        sets: vec![generated(1)],
        fail_after: Some((0, pg_error("23505", "duplicate key value"))),
        ..FakeQuery::default()
    };
    let (b, s, _f) = service_over(fake_pg().queries(vec![begin, fails])).await;
    let mut plan = run_all(2);
    plan.items[1].decision = hacivat_exec::GateDecision::RunInTxThenConfirm {
        reasons: Vec::new(),
    };
    plan.items[1].verdict = hacivat_session::session::UnitVerdict::TxThenConfirm;
    let mut t = b.svc.run(
        &s,
        request(&b, units(&["BEGIN", "UPDATE t SET a = 1"]), plan),
    );
    let summary = (&mut t.done).await.unwrap();
    assert_eq!(summary.failed, 1, "{summary:?} {:?}", b.log_ids());
    let ids = b.log_ids();
    assert!(ids.contains(&"exec-confirm-statement-failed"), "{ids:?}");
}
