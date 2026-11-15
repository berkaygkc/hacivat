//! M3a Task 1b: the SQL text layers the run service sits on, the D-011 policies, and the
//! session test helper (`hacivat_session::testing::fake_session`). Task 23 replaced the run
//! service's own stub tests (resolve, run, the log ring) with `units.rs`, `service.rs` and
//! `log.rs`.

mod common;

use common::{doc, profile_of};
use hacivat_driver::fake::{FakeDriver, FakeQuery};
use hacivat_exec::{RunAction, policy_for};
use hacivat_proto::DbmsKind;
use hacivat_session::testing::fake_session;
use sql_lexer::TokenKind;
use sql_split::exec_range::{ExecPolicy, InsidePolicy, OutsidePolicy, SelectionPolicy};
use sql_split::{Confidence, Item, head};
use sql_text::{TextRange, TextSize};

#[test]
fn the_text_stubs_beneath_fail_closed() {
    let d = doc("SELECT 1; DELETE FROM t");
    // The lexer works since Task 4 (lossless: its tokens tile the 23 bytes); the stubs above
    // it still fail closed.
    let l = d.lexed();
    assert_eq!(l.len(), 10);
    assert_eq!(l.kind(3), TokenKind::Semicolon);
    assert_eq!(l.range(l.len() - 1).end(), TextSize::from(23));
    let delete = l.token_at(TextSize::from(12)).unwrap();
    assert_eq!(l.kind(delete), TokenKind::Word);
    assert_eq!(
        l.range(delete),
        TextRange::new(TextSize::from(10), TextSize::from(16))
    );
    // The splitter works since Task 8: two exact statements in both modes, no uncertain
    // boundary; cursor eligibility reads each unit's head.
    for split in [d.editor_split(), d.execution_split()] {
        assert!(matches!(
            split.items.as_slice(),
            [Item::Statement(a), Item::Statement(b)]
                if a.confidence == Confidence::Exact && b.confidence == Confidence::Exact
        ));
        assert!(split.ambiguous.is_empty());
    }
    let units: Vec<TextRange> = d.execution_split().statements().map(|s| s.strict).collect();
    assert_eq!(
        units,
        [
            TextRange::new(TextSize::from(0), TextSize::from(8)),
            TextRange::new(TextSize::from(10), TextSize::from(23))
        ]
    );
    assert!(head::cursor_eligible(&d, units[0]));
    assert!(!head::cursor_eligible(&d, units[1]));
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
async fn the_session_helper_runs_a_real_session() {
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
}
