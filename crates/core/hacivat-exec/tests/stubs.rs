//! M3a Task 1b: the run service's stubs fail closed, the SQL text stubs they sit on fail
//! closed, and the session test helper (`hacivat_session::testing::fake_session`) works.

mod common;

use std::sync::Arc;
use std::time::SystemTime;

use common::{caret, doc, profile_of};
use hacivat_driver::fake::{FakeDriver, FakeQuery};
use hacivat_exec::{
    ExecService, ExecUnit, LogLine, LogMsg, Resolution, RunAction, RunRequest, SessionLog,
    Severity, policy_for, resolve_units, units_for_candidate,
};
use hacivat_proto::DbmsKind;
use hacivat_results::FetchPolicy;
use hacivat_session::testing::fake_session;
use sql_lexer::TokenKind;
use sql_split::exec_range::{
    Candidate, CandidateKind, ExecPolicy, InsidePolicy, OutsidePolicy, SelectionPolicy, UnitKind,
};
use sql_split::{Confidence, Item, head};
use sql_text::{TextRange, TextSize};

#[test]
fn resolve_stub_runs_nothing() {
    assert!(matches!(
        resolve_units(&doc("SELECT 1"), &[caret(0)], RunAction::Execute1, None),
        Resolution::NothingToRun
    ));
    let candidate = Candidate {
        kind: CandidateKind::Statement,
        units: Vec::new(),
        label: "SELECT 1".into(),
    };
    assert!(units_for_candidate(&doc("SELECT 1"), &candidate).is_empty());
}

#[test]
fn the_text_stubs_beneath_fail_closed() {
    let d = doc("SELECT 1; DELETE FROM t");
    // One `Unknown` token over the whole input: lossless, and no keyword is recognised.
    assert_eq!(d.lexed().len(), 1);
    assert_eq!(d.lexed().kind(0), TokenKind::Unknown);
    assert_eq!(
        d.lexed().range(0),
        TextRange::new(TextSize::from(0), TextSize::from(23))
    );
    assert_eq!(d.lexed().token_at(TextSize::from(12)), Some(0));
    // One heuristic item in both modes: execution never runs a heuristic boundary.
    for split in [d.editor_split(), d.execution_split()] {
        assert!(matches!(
            split.items.as_slice(),
            [Item::Statement(s)] if s.confidence == Confidence::Heuristic
        ));
    }
    assert!(!head::cursor_eligible(&d, d.lexed().range(0)));
    assert!(sql_oracle_pg::split_with_parser("SELECT 1; SELECT 2").is_err());
    assert_eq!(sql_oracle_pg::on_parser_stack(|| 1), None);
    assert!(doc("").lexed().is_empty());
}

#[test]
fn run_actions_map_to_the_d011_policies() {
    assert_eq!(
        policy_for(RunAction::Execute1),
        ExecPolicy {
            inside: InsidePolicy::Ask,
            outside: OutsidePolicy::Nothing,
            selection: SelectionPolicy::AsSeparate,
        }
    );
    assert_eq!(
        policy_for(RunAction::Execute2).inside,
        InsidePolicy::LargestStatement
    );
    let whole = policy_for(RunAction::Execute3);
    assert_eq!(
        (whole.inside, whole.outside),
        (InsidePolicy::WholeScript, OutsidePolicy::WholeScript)
    );
}

#[tokio::test]
async fn the_service_stub_sends_nothing_and_aborts() {
    let fake = FakeDriver::new(DbmsKind::Postgres).queries(vec![FakeQuery {
        prefix: "SELECT",
        ..Default::default()
    }]);
    let (h, f) = fake_session(
        tokio::runtime::Handle::current(),
        fake,
        profile_of(DbmsKind::Postgres),
    )
    .await;
    assert!(!h.state().borrow().read_only, "the profile is read-write");
    // The helper's session really runs: a statement reaches the fake.
    let reply = h
        .submit(hacivat_session::session::Request::Exec {
            sql: "SELECT 1".into(),
            opts: hacivat_driver::ExecOpts::default(),
            origin: hacivat_session::session::ExecOrigin::User,
        })
        .reply
        .await
        .unwrap();
    assert!(reply.is_ok());
    assert_eq!(f.executed(), ["SELECT 1"]);

    let svc = ExecService::new(hacivat_results::MemoryBudget::new(1 << 30, 1 << 28));
    let unit = ExecUnit {
        text: "SELECT 1".into(),
        doc_range: 0..8,
        kind: UnitKind::Statement,
        suspendable: true,
    };
    let ticket = svc.run(
        &h,
        RunRequest {
            console: 1,
            units: vec![unit],
            fetch: FetchPolicy::default(),
            log: SessionLog::new(),
        },
    );
    let summary = ticket.done.await.unwrap();
    assert!(summary.aborted);
    assert_eq!((summary.executed, summary.total), (0, 1));
    assert!(f.typed_log().is_empty());
    assert_eq!(f.executed(), ["SELECT 1"]);
    assert!(ticket.results.borrow().is_empty());
}

fn line(seq: u64) -> LogLine {
    LogLine {
        seq,
        at: SystemTime::UNIX_EPOCH,
        severity: Severity::Info,
        msg: LogMsg::Completed { ms: seq },
        source: None,
        unit: None,
    }
}

#[test]
fn the_session_log_keeps_the_newest_lines() {
    let log = SessionLog::new();
    let mut changes = log.changes();
    for seq in 0..(SessionLog::CAPACITY as u64 + 5) {
        log.push(line(seq));
    }
    let lines = log.lines();
    assert_eq!(lines.len(), SessionLog::CAPACITY);
    assert_eq!(lines[0].seq, 5);
    assert!(changes.has_changed().unwrap());
    assert_eq!(
        *changes.borrow_and_update(),
        SessionLog::CAPACITY as u64 + 5
    );
    log.clear();
    assert!(log.lines().is_empty());
    assert!(changes.has_changed().unwrap());
    let _: Arc<SessionLog> = log;
}
