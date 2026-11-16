//! M3b Task 16: the run service's M3b path (HV-EXEC-06/-07/-08; R-M3-61, R-M3-63, R-M3-66,
//! R-M3-84): prepared units with a checked gate plan, up to 100 result sets with a tab each,
//! the error policy's question on the ticket, the user's pre-run skips, the history sink, and
//! M3a's path unchanged.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::m3b::{item, prepared, request, run_all, text_bind, unit, units};
use common::{Bench, fake_pg, generated, pg_error, service_over};
use hacivat_driver::fake::{FakeQuery, FakeTypedCall};
use hacivat_exec::{
    ConfirmAnswer, ErrorPolicy, ExecService, GateDecision, GatePlan, GateReason, HistoryMeta,
    HistoryOrigin, HistoryRecord, LogMsg, RunChoice, RunTicket, apply_answer, copy_invariant,
};
use hacivat_proto::ProfileId;
use hacivat_results::MemoryBudget;
use hacivat_session::session::RunEvent;
use tokio::sync::mpsc;

/// A query starting with `prefix` that fails at once with `code` and `text`.
fn failing(prefix: &'static str, code: &str, text: &str) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![generated(1)],
        fail_after: Some((0, pg_error(code, text))),
        ..Default::default()
    }
}

/// The tabs a run has opened.
fn tabs(t: &RunTicket) -> Vec<String> {
    t.results
        .borrow()
        .iter()
        .map(|r| r.title.to_string())
        .collect()
}

fn log_has(b: &Bench, id: &str) -> bool {
    b.log_ids().contains(&id)
}

/// A `CALL` that returns `n` result sets of one row each, run as a prepared unit.
async fn run_fake_call_sets(n: usize) -> (Bench, RunTicket) {
    let fake = fake_pg().queries(vec![FakeQuery {
        prefix: "CALL",
        sets: vec![generated(1); n],
        ..Default::default()
    }]);
    let (b, s, _f) = service_over(fake).await;
    let req = request(&b, vec![unit("CALL p()", 0, Some("çok küme"))], run_all(1));
    let mut t = b.svc.run(&s, req);
    (&mut t.done).await.expect("the run ends");
    (b, t)
}

#[tokio::test]
async fn hundred_and_fifty_sets_give_a_hundred_tabs() {
    let (b, t) = run_fake_call_sets(150).await;
    assert_eq!(tabs(&t).len(), 100);
    assert!(log_has(&b, "exec-result-set-limit"));
    let limit = b
        .log_msgs()
        .into_iter()
        .find(|m| matches!(m, LogMsg::ResultSetLimit { .. }));
    assert_eq!(
        limit,
        Some(LogMsg::ResultSetLimit {
            total: 150,
            limit: 100
        })
    );
    // Each set its own tab and key; the title is the unit's, numbered from the second set.
    let titles = tabs(&t);
    assert_eq!(titles[0], "çok küme");
    assert_eq!(titles[1], "çok küme (2)");
    assert_eq!(titles[99], "çok küme (100)");
    let keys: Vec<u32> = t
        .results
        .borrow()
        .iter()
        .map(|r| r.key.result_ordinal)
        .collect();
    assert_eq!(keys, (0..100).collect::<Vec<u32>>());
}

#[tokio::test]
async fn error_policy_choice_flows_through_the_ticket() {
    let fake = fake_pg().queries(vec![failing(
        "UPDATE nope",
        "42P01",
        "relation \"nope\" does not exist",
    )]);
    let (b, s, f) = service_over(fake).await;
    let mut req = request(
        &b,
        units(&[
            "UPDATE a SET x = 1",
            "UPDATE nope SET x = 1",
            "UPDATE c SET x = 1",
        ]),
        run_all(3),
    );
    req.policy = ErrorPolicy::Ask;
    let mut t = b.svc.run(&s, req);
    let (seq, question) = {
        let pending = t
            .choices
            .wait_for(Option::is_some)
            .await
            .expect("the run asks");
        let (seq, ev) = pending.clone().expect("a question");
        let RunEvent::NeedsChoice { unit, question, .. } = ev else {
            panic!("expected NeedsChoice, got {ev:?}");
        };
        assert_eq!(unit.ordinal, 1);
        (seq, question)
    };
    assert_eq!(seq, t.seq, "the question names the run's own request");
    // An answer to another run is refused; the right one goes on.
    assert!(
        !s.choose(seq + 1, question, RunChoice::Ignore)
            .await
            .unwrap()
    );
    assert!(s.choose(seq, question, RunChoice::Ignore).await.unwrap());
    let summary = (&mut t.done).await.unwrap();
    assert_eq!((summary.executed, summary.failed), (2, 1));
    assert!(
        t.choices.borrow().is_none(),
        "nothing waits once the run ended"
    );
    let sent: Vec<String> = f
        .typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(q) => Some(q),
            _ => None,
        })
        .collect();
    assert!(sent.iter().any(|q| q == "UPDATE c SET x = 1"), "{sent:?}");
}

#[tokio::test]
async fn pre_run_skip_is_logged_and_the_unit_skipped() {
    let (b, s, f) = service_over(fake_pg()).await;
    let us = units(&["UPDATE a SET x = 1", "DELETE FROM b", "UPDATE c SET x = 1"]);
    let mut plan = GatePlan {
        items: vec![
            item(0, GateDecision::Run),
            item(
                1,
                GateDecision::Confirm {
                    reasons: Vec::new(),
                    type_to_confirm: None,
                    irreversible: false,
                    suppressible: true,
                },
            ),
            item(2, GateDecision::Warn(Vec::new())),
        ],
    };
    // An unanswered Confirm or Warn sends nothing at all.
    let before = f.call_log().len();
    let t = b.svc.run(&s, request(&b, us.clone(), plan.clone()));
    assert_eq!(t.seq, u64::MAX);
    assert_eq!(t.done.await.unwrap().executed, 0);
    assert!(f.call_log()[before..].is_empty());
    // Skip the second, execute the third.
    assert!(apply_answer(&mut plan, 1, ConfirmAnswer::Skip));
    assert!(apply_answer(&mut plan, 2, ConfirmAnswer::Execute));
    let mut t = b.svc.run(&s, request(&b, us, plan.clone()));
    let summary = (&mut t.done).await.unwrap();
    assert_eq!(
        (summary.executed, summary.skipped, summary.total),
        (2, 1, 3)
    );
    assert_eq!(t.status.borrow().units[1].status.name(), "Skipped");
    let sent: Vec<String> = f
        .typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(q) => Some(q),
            _ => None,
        })
        .collect();
    assert!(!sent.iter().any(|q| q.starts_with("DELETE")), "{sent:?}");
    assert!(
        b.log_msgs().contains(&LogMsg::SkippedByUser { n: 1 }),
        "{:?}",
        b.log_ids()
    );
    // An answer never turns a denial into a run, and Cancel sends nothing.
    let mut denied = GatePlan {
        items: vec![item(0, GateDecision::Deny(GateReason::of("exec-gate-x")))],
    };
    assert!(apply_answer(&mut denied, 0, ConfirmAnswer::Execute));
    assert!(matches!(denied.items[0].decision, GateDecision::Deny(_)));
    assert!(!apply_answer(&mut plan, 0, ConfirmAnswer::Cancel));
}

#[tokio::test]
async fn an_empty_or_short_plan_never_allows_all() {
    let (b, s, f) = service_over(fake_pg()).await;
    let before = f.call_log().len();
    let us = units(&["UPDATE a SET x = 1", "UPDATE b SET x = 1"]);
    let mut wrong_order = run_all(2);
    wrong_order.items.swap(0, 1);
    let mut mismatched = request(&b, us.clone(), run_all(2));
    if let Some(p) = mismatched.prepared.as_mut() {
        p[1] = prepared(&unit("DROP TABLE b", 20, None));
    }
    for req in [
        request(&b, us.clone(), GatePlan::default()),
        request(&b, us.clone(), run_all(1)),
        request(&b, us.clone(), wrong_order),
        mismatched,
    ] {
        let t = b.svc.run(&s, req);
        assert_eq!(t.seq, u64::MAX, "nothing was sent");
        assert_eq!(t.done.await.unwrap().executed, 0);
    }
    assert!(f.call_log()[before..].is_empty());
}

// ## m3b-t19-confirm-tx (named exception): until Task 19 the session refused this unit (the
// test asserted the refusal); now the `NeedsCommit` reaches the console's port, its answer
// returns to the session, and the session acts on it.
/// The console's confirm port, answering every confirm-in-transaction question `answer`.
#[derive(Debug)]
struct Answers(hacivat_exec::CommitAnswer, std::sync::Mutex<Vec<bool>>);

impl hacivat_exec::ConfirmPort for Answers {
    fn confirm(
        &self,
        _: hacivat_exec::ConfirmQuestion,
    ) -> tokio::sync::oneshot::Receiver<ConfirmAnswer> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(ConfirmAnswer::Cancel);
        rx
    }

    fn commit_decision(
        &self,
        q: hacivat_exec::CommitQuestion,
    ) -> tokio::sync::oneshot::Receiver<hacivat_exec::CommitAnswer> {
        self.1.lock().unwrap().push(q.savepoint);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = tx.send(self.0);
        rx
    }
}

#[tokio::test]
async fn needs_commit_reaches_the_confirm_port_and_answer_returns() {
    for (answer, last, id) in [
        (
            hacivat_exec::CommitAnswer::Commit,
            "COMMIT",
            "exec-confirm-committed",
        ),
        (
            hacivat_exec::CommitAnswer::RollBack,
            "ROLLBACK",
            "exec-confirm-rolled-back",
        ),
    ] {
        let (b, s, f) = service_over(fake_pg()).await;
        let mut plan = run_all(1);
        plan.items[0].decision = GateDecision::RunInTxThenConfirm {
            reasons: Vec::new(),
        };
        plan.items[0].verdict = hacivat_session::session::UnitVerdict::TxThenConfirm;
        let port = Arc::new(Answers(answer, std::sync::Mutex::new(Vec::new())));
        let before = f.call_log().len();
        let mut req = request(&b, units(&["UPDATE t SET a = 1"]), plan);
        req.confirm = Some(port.clone());
        let mut t = b.svc.run(&s, req);
        let summary = tokio::time::timeout(Duration::from_secs(10), &mut t.done)
            .await
            .expect("the answer reached the session")
            .unwrap();
        // Fix round 1 (C2): a rolled-back statement is no success.
        let expected = match answer {
            hacivat_exec::CommitAnswer::Commit => (1, 0),
            hacivat_exec::CommitAnswer::RollBack => (0, 1),
        };
        assert_eq!((summary.executed, summary.failed), expected, "{answer:?}");
        assert_eq!(*port.1.lock().unwrap(), [false], "Hacivat's own form");
        assert_eq!(
            f.call_log()[before..],
            [
                hacivat_driver::fake::FakeCall::Statement("BEGIN".into()),
                hacivat_driver::fake::FakeCall::Typed(FakeTypedCall::Query(
                    "UPDATE t SET a = 1".into()
                )),
                hacivat_driver::fake::FakeCall::Statement(last.into()),
            ],
            "{answer:?}"
        );
        assert!(b.log_ids().contains(&id), "{answer:?}: {:?}", b.log_ids());
    }
}

#[tokio::test]
async fn slow_sink_never_slows_the_run_and_counts_drops() {
    let fake = fake_pg();
    let (b, s, _f) = service_over(fake).await;
    // Room for two records, and nobody reads them.
    let (tx, rx) = mpsc::channel::<Arc<HistoryRecord>>(2);
    let svc = ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_history_sink(tx);
    let mut req = request(
        &b,
        units(&[
            "UPDATE a SET x = 1",
            "UPDATE b SET x = 1",
            "UPDATE c SET x = 1",
            "UPDATE d SET x = 1",
            "UPDATE e SET x = 1",
        ]),
        run_all(5),
    );
    req.history = Some(meta(false));
    let mut t = svc.run(&s, req);
    let summary = tokio::time::timeout(Duration::from_secs(10), &mut t.done)
        .await
        .expect("a full sink never holds the run")
        .unwrap();
    assert_eq!(summary.executed, 5);
    assert!(
        b.log_msgs().contains(&LogMsg::Dropped { n: 3 }),
        "{:?}",
        b.log_ids()
    );
    drop(rx);
}

fn meta(private: bool) -> HistoryMeta {
    HistoryMeta {
        profile: ProfileId::new(),
        console: 7,
        private,
        origin: HistoryOrigin::Console,
    }
}

#[tokio::test]
async fn unmasked_sql_never_reaches_the_history_sink() {
    let fake = fake_pg().queries(vec![failing(
        "SELECT * FROM t WHERE pw",
        "22P02",
        "invalid input syntax for type integer: \"s3cret\"",
    )]);
    let (b, s, f) = service_over(fake).await;
    let (tx, mut rx) = mpsc::channel::<Arc<HistoryRecord>>(16);
    let svc = ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_history_sink(tx);
    let u = unit("SELECT * FROM t WHERE pw = :pw", 0, None);
    let mut p = prepared(&u);
    p.final_text = "SELECT * FROM t WHERE pw = $1".into();
    p.binds = Arc::from(vec![text_bind("s3cret")]);
    p.masked = "SELECT * FROM t WHERE pw = :pw /* masked */".into();
    let mut req = request(&b, vec![u], run_all(1));
    req.prepared = Some(vec![p]);
    req.history = Some(meta(false));
    let mut t = svc.run(&s, req);
    (&mut t.done).await.unwrap();
    // The server got the bind.
    assert!(
        f.binds_seen()
            .iter()
            .flatten()
            .any(|v| *v == text_bind("s3cret"))
    );
    let record = rx.recv().await.expect("one record per unit");
    assert_eq!(
        &*record.statement_masked,
        "SELECT * FROM t WHERE pw = :pw /* masked */"
    );
    assert!(
        !record.statement_masked.contains("$1"),
        "never the text sent"
    );
    let message = record.error_message.as_deref().unwrap_or_default();
    assert!(
        !message.contains("s3cret"),
        "the bind is scrubbed from the message"
    );
    assert!(message.contains("invalid input syntax"));
    assert_eq!(
        record.text_hash, [0; 32],
        "the store hashes its own redacted text"
    );
    assert_eq!(record.fingerprint, 0);
    assert!(record.params_masked.is_none());
    assert!(rx.try_recv().is_err(), "one record per unit");
    // The log copy never holds a statement.
    let copied = copy_invariant(&b.log.lines());
    assert!(!copied.contains("pw = $1") && !copied.contains("pw = :pw"));
    // A private console records nothing.
    let (tx, mut rx) = mpsc::channel::<Arc<HistoryRecord>>(16);
    let svc = ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_history_sink(tx);
    let mut req = request(&b, units(&["UPDATE a SET x = 1"]), run_all(1));
    req.history = Some(meta(true));
    let mut t = svc.run(&s, req);
    (&mut t.done).await.unwrap();
    assert!(rx.try_recv().is_err(), "a private console records nothing");
}

#[tokio::test]
async fn m3a_path_without_prepared_is_unchanged() {
    // Two result sets: M3a keeps the first and says the rest were not shown.
    let fake = fake_pg().queries(vec![
        FakeQuery {
            prefix: "CALL",
            sets: vec![generated(2), generated(3)],
            ..Default::default()
        },
        FakeQuery {
            prefix: "SELECT",
            sets: vec![generated(1)],
            ..Default::default()
        },
    ]);
    let (b, s, f) = service_over(fake).await;
    let mut t = b.svc.run(&s, b.req(&["CALL p()", "SELECT 1"]));
    let summary = (&mut t.done).await.unwrap();
    assert_eq!((summary.executed, summary.total), (2, 2));
    assert_eq!(tabs(&t), ["Result 1", "Result 2"]);
    assert!(log_has(&b, "exec-more-result-sets"));
    assert!(!log_has(&b, "exec-result-set-limit"));
    assert!(t.choices.borrow().is_none());
    let queries: Vec<FakeTypedCall> = f
        .typed_log()
        .into_iter()
        .filter(|c| matches!(c, FakeTypedCall::Query(_) | FakeTypedCall::Binds(_)))
        .collect();
    assert_eq!(
        queries,
        [
            FakeTypedCall::Query("CALL p()".into()),
            FakeTypedCall::Query("SELECT 1".into())
        ]
    );
}

/// R-M3-23 (T15 carry): only a unit whose gate verdict is `Read` may stay open as a console
/// cursor; any other verdict reads its result to the end, however eligible its text.
#[tokio::test]
async fn a_non_read_verdict_never_produces_a_suspendable_unit() {
    use hacivat_session::session::UnitVerdict;
    for (verdict, suspended) in [
        (UnitVerdict::Read, true),
        (UnitVerdict::Write, false),
        (UnitVerdict::Unknown, false),
        (UnitVerdict::ImplicitCommit, false),
    ] {
        let fake = fake_pg().queries(vec![FakeQuery {
            prefix: "SELECT",
            sets: vec![generated(20_000)],
            ..Default::default()
        }]);
        let (b, s, _f) = service_over(fake).await;
        let mut u = unit("SELECT * FROM big", 0, None);
        u.suspendable = true;
        let mut plan = run_all(1);
        plan.items[0].verdict = verdict;
        let mut t = b.svc.run(&s, request(&b, vec![u], plan));
        (&mut t.done).await.unwrap();
        let status = t.status.borrow().units[0].status.name();
        assert_eq!(status == "Suspended", suspended, "{verdict:?}: {status}");
        if !suspended {
            assert_eq!(t.results.borrow()[0].source.snapshot().row_count(), 20_000);
        }
    }
}
