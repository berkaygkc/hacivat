//! M3b Task 11: what a run sends and keeps. Bind values reach the driver in slot order
//! (HV-EXEC-03); a unit keeps up to 100 result sets, each its own result (R-M3-66), and one
//! with several never suspends; "run as one" sends one unit with the simple protocol (R-M3-60)
//! and is refused on a read-only session; `ABORT` ends a failed PostgreSQL transaction like
//! `ROLLBACK` (R-M3-74) and is nothing special elsewhere; server notices become run events, at
//! most 100 per unit (R-M3-77). FakeDriver and a paused clock: no socket, no real time.

mod common;

use common::runs::*;
use common::runs_m3b::*;
use common::*;
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeTypedCall};
use hacivat_driver::{Protocol, TxStatus};
use hacivat_results::{Completeness, TruncReason};
use hacivat_session::session::{
    DenyLayer, ExecErrorKind, Request, RowsOutcome, RunEvent, SessionState, TxError, UnitEnd,
};
use hacivat_values::BindValue;

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

fn more_sets(o: &Recorder) -> Vec<u32> {
    o.events()
        .into_iter()
        .filter_map(|e| match e {
            RunEvent::MoreResultSets { skipped, .. } => Some(skipped),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Binds

#[tokio::test(start_paused = true)]
async fn binds_reach_the_driver_in_slot_order() {
    let (h, f) = session(fake_pg()).await;
    let o = Recorder::new();
    h.run(spec_binds(
        &o,
        "SELECT $1, $2",
        &[text("a"), BindValue::Null],
    ))
    .reply
    .await
    .unwrap()
    .unwrap();
    assert_eq!(f.binds_seen(), vec![vec![text("a"), BindValue::Null]]);
    // With the query, never spliced into its text.
    assert_eq!(
        f.typed_log(),
        vec![
            FakeTypedCall::Query("SELECT $1, $2".into()),
            FakeTypedCall::Binds(vec![text("a"), BindValue::Null]),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_value_with_a_quote_is_bound_as_it_is() {
    let (h, f) = session(fake_pg()).await;
    let o = Recorder::new();
    let hostile = "x'); DROP TABLE t; --";
    run_spec(&h, spec_binds(&o, "SELECT $1", &[text(hostile)]))
        .await
        .unwrap();
    assert_eq!(f.binds_seen(), vec![vec![text(hostile)]]);
    assert_eq!(f.typed_log()[0], FakeTypedCall::Query("SELECT $1".into()));
}

// ---------------------------------------------------------------------------------------------
// Result sets (R-M3-66)

#[tokio::test(start_paused = true)]
async fn hundred_and_fifty_sets_give_a_hundred_results() {
    let call = FakeQuery {
        prefix: "CALL",
        sets: (0..150).map(one_row).collect(),
        ..FakeQuery::default()
    };
    let (h, _f) = session(fake_mysql().queries(vec![call])).await;
    let o = Recorder::new();
    let mut u = unit(0, "CALL many_sets()");
    u.max_result_sets = 100;
    run_spec(&h, spec_units(&o, vec![u])).await.unwrap();
    assert_eq!(o.stores(), 100);
    let ordinals: Vec<u32> = opened(&o).into_iter().map(|(_, set)| set).collect();
    assert_eq!(ordinals, (0..100).collect::<Vec<_>>());
    assert_eq!(more_sets(&o), vec![50]);
    // Each set holds its own row; the last says sets followed it.
    assert_eq!(o.snap(0).row_count(), 1);
    assert_eq!(o.snap(99).row_count(), 1);
    let ends = o.result_ends();
    assert_eq!(ends.len(), 100, "every result ends once");
    assert!(ends[..99].iter().all(|c| *c == Completeness::Complete));
    assert_eq!(
        ends[99],
        Completeness::Truncated(TruncReason::FollowingResultSets)
    );
    assert_eq!(
        o.unit_ends(),
        vec![UnitEnd::Succeeded {
            rows: Some(RowsOutcome::Fetched {
                rows: 1,
                more: false
            })
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn a_unit_keeps_at_most_a_hundred_sets() {
    let call = FakeQuery {
        prefix: "CALL",
        sets: (0..150).map(one_row).collect(),
        ..FakeQuery::default()
    };
    let (h, f) = session(fake_mysql().queries(vec![call])).await;
    let o = Recorder::new();
    let mut u = unit(0, "CALL many_sets()");
    u.max_result_sets = 1_000;
    run_spec(&h, spec_units(&o, vec![u])).await.unwrap();
    assert_eq!(o.stores(), 100);
    assert_eq!(more_sets(&o), vec![50]);
    assert_eq!(f.typed_log().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn multi_result_unit_never_suspends() {
    let two = FakeQuery {
        prefix: "SELECT",
        sets: vec![generated(5_000), generated(5_000)],
        ..FakeQuery::default()
    };
    let (h, f) = session(pg().queries(vec![two])).await;
    let o = Recorder::new();
    let mut spec = spec_of(&o, &["SELECT * FROM a"]);
    let mut units = spec.units.to_vec();
    units[0].max_result_sets = 100;
    spec.units = units.into();
    run_spec(&h, spec).await.unwrap();
    assert_eq!(opened(&o), vec![(0, 0), (0, 1)]);
    assert_eq!(o.snap(0).row_count(), 5_000);
    assert_eq!(o.snap(1).row_count(), 5_000);
    assert_eq!(
        o.result_ends(),
        vec![Completeness::Complete, Completeness::Complete]
    );
    // Read to its end in one pass: no fetch, no cursor, Hacivat's own transaction committed.
    assert!(
        !f.typed_log()
            .iter()
            .any(|c| matches!(c, FakeTypedCall::FetchMore(_)))
    );
    let s = snap(&h);
    assert_eq!((s.cursor, s.tx.status), (None, TxStatus::Idle));
    assert_eq!(statements(&f), vec!["BEGIN", "COMMIT"]);
}

#[tokio::test(start_paused = true)]
async fn a_dml_set_between_two_reads_reports_its_count() {
    let sets = FakeQuery {
        prefix: "CALL",
        sets: vec![
            one_row(1),
            hacivat_driver::fake::FakeResultSet::default(),
            one_row(3),
        ],
        rows_affected: Some(7),
        ..FakeQuery::default()
    };
    let (h, _f) = session(fake_mysql().queries(vec![sets])).await;
    let o = Recorder::new();
    let mut u = unit(0, "CALL mixed()");
    u.max_result_sets = 100;
    run_spec(&h, spec_units(&o, vec![u])).await.unwrap();
    // The set without columns opens no result; the ordinals are the sets' places.
    assert_eq!(opened(&o), vec![(0, 0), (0, 2)]);
    let counts: Vec<Option<u64>> = o
        .events()
        .into_iter()
        .filter_map(|e| match e {
            RunEvent::UpdateCount { rows, .. } => Some(rows),
            _ => None,
        })
        .collect();
    assert_eq!(counts, vec![Some(7), Some(7), Some(7)], "one per set");
}

// ---------------------------------------------------------------------------------------------
// Run as one (R-M3-60)

#[tokio::test(start_paused = true)]
async fn run_as_one_uses_simple_protocol_and_one_unit() {
    let batch = FakeQuery {
        prefix: "INSERT",
        sets: vec![hacivat_driver::fake::FakeResultSet::default(), one_row(1)],
        ..FakeQuery::default()
    };
    let (h, f) = session(pg().queries(vec![batch])).await;
    let o = Recorder::new();
    let sql = "INSERT INTO t VALUES (1); SELECT v FROM t";
    let mut u = unit(0, sql);
    u.protocol = Protocol::Simple;
    // A suspendable unit run as one still never suspends.
    u.suspendable = true;
    u.max_result_sets = 100;
    run_spec(&h, spec_units(&o, vec![u])).await.unwrap();
    assert_eq!(f.typed_log(), vec![FakeTypedCall::Query(sql.into())]);
    assert_eq!(opened(&o), vec![(0, 1)]);
    assert_eq!(o.snap(0).row_count(), 1);
    assert_eq!(o.result_ends(), vec![Completeness::Complete]);
    // No transaction of Hacivat's around it, no cursor.
    assert!(statements(&f).is_empty(), "{:?}", statements(&f));
    assert_eq!(snap(&h).cursor, None);
}

#[tokio::test(start_paused = true)]
async fn run_as_one_is_refused_on_read_only() {
    let (h, f) = ro_session(fake_pg(), pg_profile(hacivat_net::TlsMode::Disable)).await;
    let before = f.call_log().len();
    let o = Recorder::new();
    let mut u = unit(0, "SELECT 1; SELECT 2");
    u.protocol = Protocol::Simple;
    let e = run_spec(&h, spec_units(&o, vec![u])).await.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::ReadOnlyDenied(DenyLayer::Protocol));
    assert!(f.call_log()[before..].is_empty(), "nothing sent");
}

// ---------------------------------------------------------------------------------------------
// ABORT (R-M3-74)

#[tokio::test(start_paused = true)]
async fn abort_in_a_failed_tx_rolls_back() {
    // The `Exec` path.
    let (h, f) = session(pg().exec(vec![tx_err(
        "SELECT * FROM missing",
        "42P01",
        TxStatus::Failed,
    )]))
    .await;
    run(&h, "BEGIN").await.unwrap();
    assert!(run(&h, "SELECT * FROM missing").await.is_err());
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    run(&h, "abort")
        .await
        .expect("ABORT ends the failed transaction");
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert!(statements(&f).contains(&"abort".to_string()));

    // The typed path.
    let abort = FakeQuery {
        prefix: "ABORT",
        tx_after: Some(TxStatus::Idle),
        ..FakeQuery::default()
    };
    let (h, f) = session(pg().queries(vec![failing("SELECT * FROM missing"), abort])).await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    let failed = spec_units(&o, vec![unit(0, "SELECT * FROM missing")]);
    assert!(run_spec(&h, failed).await.is_err());
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    let o = Recorder::new();
    run_spec(&h, spec_units(&o, vec![unit(0, "ABORT")]))
        .await
        .expect("ABORT runs");
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
    assert!(o.events().contains(&RunEvent::TxChanged {
        from: TxStatus::Failed,
        to: TxStatus::Idle,
    }));
    assert!(
        f.typed_log()
            .contains(&FakeTypedCall::Query("ABORT".into()))
    );
}

#[tokio::test(start_paused = true)]
async fn abort_is_no_rollback_on_mysql() {
    // MySQL has no failed transaction state and no `ABORT`: a session that sees one anyway
    // keeps asking for a ROLLBACK.
    let (h, f) = session(fake_mysql().exec(vec![tx_err(
        "SELECT * FROM missing",
        "42S02",
        TxStatus::Failed,
    )]))
    .await;
    assert!(run(&h, "SELECT * FROM missing").await.is_err());
    assert_eq!(snap(&h).tx.status, TxStatus::Failed);
    let e = run(&h, "ABORT").await.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Tx(TxError::FailedNeedsRollback));
    assert!(!f.executed().iter().any(|s| s == "ABORT"), "nothing sent");
}

// ---------------------------------------------------------------------------------------------
// Notices (R-M3-77)

#[tokio::test(start_paused = true)]
async fn notices_become_run_events_capped_at_100() {
    let chatty = FakeQuery {
        notices: notices(150),
        ..answering("SELECT")
    };
    let (h, _f) = session(pg().queries(vec![chatty])).await;
    let o = Recorder::new();
    let logs = capture_tracing(|| async {
        run_spec(&h, spec_units(&o, vec![unit(0, "SELECT raise_many()")]))
            .await
            .unwrap();
    })
    .await;
    let texts = notice_texts(&o);
    assert_eq!(texts.len(), 100);
    assert_eq!(texts[0], "n0");
    assert_eq!(texts[99], "n99");
    assert!(
        o.events().iter().all(|e| match e {
            RunEvent::Notice { unit, .. } => unit.ordinal == 0,
            _ => true,
        }),
        "every notice names its unit"
    );
    // The rest are counted in a log line; no notice text is ever logged.
    assert!(logs.contains("notices_dropped=50"), "{logs}");
    assert!(!logs.contains("n120"), "notice text never logged");
    assert_eq!(snap(&h).state, SessionState::Idle);
}

// ---------------------------------------------------------------------------------------------
// The M3a shapes stay

#[tokio::test(start_paused = true)]
async fn a_plain_unit_keeps_one_set() {
    let two = FakeQuery {
        prefix: "SELECT",
        sets: vec![one_row(1), one_row(2)],
        ..FakeQuery::default()
    };
    let (h, _f) = session(pg().queries(vec![two])).await;
    let o = Recorder::new();
    run_spec(&h, spec_units(&o, vec![unit(0, "SELECT 1")]))
        .await
        .unwrap();
    assert_eq!(o.stores(), 1);
    assert_eq!(more_sets(&o), vec![1]);
    // `Request::Exec` is unchanged.
    assert!(
        ask(&h, Request::Ping).await.is_ok(),
        "the session still serves requests"
    );
}
