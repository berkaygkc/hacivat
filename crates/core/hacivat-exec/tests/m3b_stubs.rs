//! M3b Task 1c: the run service's M3b seams fail closed (F31) until their tasks: the gate
//! denies, parameter preparation refuses placeholders, the history is disabled, the
//! classifier pool answers `sql-safety`'s stub and denies on the read-only path, Count rows
//! answers `Internal`, and a request with M3b-only fields sends nothing. M3a's run shape
//! still builds and runs.

mod common;

use std::sync::Arc;

use common::{fake_pg, pg18, service_over};
use hacivat_driver::TxStatus;
use hacivat_exec::{
    ClassifyCtx, ClassifyPool, ErrorPolicy, ExecUnit, Expect, GateDecision, GatePlan,
    HistoryConfig, HistoryError, HistoryStore, LogMsg, OffsetMap, ParamValueStore, Prepared,
    PreparedUnit, RunRequest, SessionLog, SuppressSet, count_sql, explain_analyze_inner, gate,
    prepare_units,
};
use hacivat_exec::{ConnSafety, GateItem};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::session::{RoCheck, StatementClassifier, TxMode, TxOwner, UnitVerdict};
use hacivat_values::BindValue;
use sql_dialect::ClientFlavor;
use sql_split::exec_range::UnitKind;
use sql_split::{ParamDiag, ParamSettings};

// ---------------------------------------------------------------------------------------------
// Fixtures

fn unit_at(text: &str, start: usize) -> ExecUnit {
    ExecUnit {
        text: text.into(),
        doc_range: start..start + text.len(),
        kind: UnitKind::Statement,
        suspendable: false,
        // ## m3b-t16-run-service (named exception)
        title: None,
    }
}

fn units(texts: &[&str]) -> Vec<ExecUnit> {
    let mut at = 0;
    texts
        .iter()
        .map(|t| {
            let u = unit_at(t, at);
            at += t.len() + 2;
            u
        })
        .collect()
}

fn prepared(text: &str) -> PreparedUnit {
    match prepare_units(
        units(&[text]),
        &pg18(),
        &ParamSettings::default(),
        &NoValues,
        &pid(),
    ) {
        Prepared::Ready(mut u) => u.remove(0),
        other => panic!("{other:?}"),
    }
}

fn ctx() -> ClassifyCtx {
    ClassifyCtx::new(pg18(), ClientFlavor::None, Expect::SingleStatement)
}

fn report_of(text: &str) -> hacivat_exec::SafetyReport {
    sql_safety::classify(text, &ctx())
}

fn stub_report() -> hacivat_exec::SafetyReport {
    report_of("SELECT 1")
}

fn conn_dev() -> ConnSafety {
    ConnSafety {
        engine: DbmsKind::Postgres,
        env: Some(EnvTag::Dev),
        read_only: false,
        tx_status: TxStatus::Idle,
        tx_owner: TxOwner::None,
        tx_mode: TxMode::Auto,
        confirm_unsafe_where: true,
    }
}

fn pid() -> ProfileId {
    ProfileId::new()
}

struct NoValues;

impl ParamValueStore for NoValues {
    fn get(&self, _: &ProfileId, _: &str) -> Option<BindValue> {
        None
    }
    fn put(&self, _: &ProfileId, _: &str, _: BindValue) {}
}

fn tmp() -> std::path::PathBuf {
    tempfile::tempdir().unwrap().keep().join("history.sqlite")
}

// ---------------------------------------------------------------------------------------------
// The gate (Task 15)

#[test]
fn gate_stub_denies_every_unit() {
    let p = gate(
        &[prepared("SELECT 1")],
        &[stub_report()],
        &conn_dev(),
        &SuppressSet::default(),
        1,
    );
    assert!(matches!(p.items[0].decision, GateDecision::Deny(_)));
    // Production, read-only, a dev console with every unit's report: all denied, nothing run.
    let both = [prepared("SELECT 1"), prepared("DELETE FROM t")];
    let reports = [report_of("SELECT 1"), report_of("DELETE FROM t")];
    for conn in [
        conn_dev(),
        ConnSafety {
            env: Some(EnvTag::Production),
            ..conn_dev()
        },
        ConnSafety {
            read_only: true,
            ..conn_dev()
        },
    ] {
        let plan = gate(&both, &reports, &conn, &SuppressSet::default(), 1);
        assert_eq!(plan.items.len(), 2);
        assert!(plan.items.iter().all(|i: &GateItem| {
            matches!(i.decision, GateDecision::Deny(_)) && i.verdict == UnitVerdict::Unknown
        }));
        assert_eq!(plan.first_denied().map(|i| i.unit), Some(0));
        assert!(!plan.needs_prompt());
    }
}

#[test]
fn gate_stub_names_a_stale_classification() {
    // P-M3b-2: a report of other bytes (or none) is stale.
    let plan = gate(
        &[prepared("SELECT 2"), prepared("SELECT 3")],
        &[report_of("SELECT 1")],
        &conn_dev(),
        &SuppressSet::default(),
        1,
    );
    for item in &plan.items {
        match &item.decision {
            GateDecision::Deny(r) => assert_eq!(r.fluent_id, "exec-gate-stale-classification"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(GatePlan::default().first_denied(), None);
}

#[test]
fn suppress_set_and_explain_stub() {
    let mut s = SuppressSet::default();
    s.insert(1, 42);
    assert!(s.contains(1, 42) && !s.contains(2, 42) && !s.contains(1, 43));
    assert!(
        stub_report()
            .items
            .iter()
            .all(|r| explain_analyze_inner(r).is_none())
    );
}

// ---------------------------------------------------------------------------------------------
// Parameters (Task 15)

#[test]
fn prepare_stub_refuses_a_colon() {
    assert!(matches!(
        prepare_units(
            units(&["SELECT :a"]),
            &pg18(),
            &ParamSettings::default(),
            &NoValues,
            &pid()
        ),
        Prepared::Refused { .. }
    ));
    // The first unit holding `:`, `$` or `?`, at its document offset.
    for (text, at) in [("SELECT $1", 7), ("SELECT ?", 7), ("SELECT '10:30'", 10)] {
        match prepare_units(
            units(&["SELECT 1", text]),
            &pg18(),
            &ParamSettings::default(),
            &NoValues,
            &pid(),
        ) {
            Prepared::Refused {
                unit,
                at: range,
                diag,
            } => {
                assert_eq!((unit, diag), (1, ParamDiag::TooManyBinds), "{text}");
                assert_eq!(range, 10 + at..10 + at + 1, "{text}");
            }
            other => panic!("{text}: {other:?}"),
        }
    }
}

#[test]
fn prepare_stub_passes_plain_units_through() {
    let u = prepared("SELECT 1");
    assert_eq!(&*u.final_text, "SELECT 1");
    assert_eq!(&*u.masked, "SELECT 1");
    assert!(u.binds.is_empty());
    assert_eq!(u.offsets, OffsetMap::identity(0));
    assert_eq!(OffsetMap::identity(40).to_doc(3), 43);
}

// ---------------------------------------------------------------------------------------------
// History (Task 17)

#[tokio::test]
async fn history_stub_is_disabled() {
    let path = tmp();
    assert!(matches!(
        HistoryStore::open(path.clone(), HistoryConfig::default()).await,
        Err(HistoryError::Disabled)
    ));
    assert!(!path.exists(), "nothing touched on disk");
    let cfg = HistoryConfig::default();
    assert_eq!(
        (cfg.per_source, cfg.max_age_days, cfg.statement_cap),
        (10_000, 90, 64 << 10)
    );
}

#[test]
fn fts5_is_compiled_in() {
    let c = rusqlite::Connection::open_in_memory().unwrap();
    c.execute_batch("CREATE VIRTUAL TABLE t USING fts5(x)")
        .unwrap();
}

// ---------------------------------------------------------------------------------------------
// The classifier pool (Task 14)

#[tokio::test]
async fn classify_pool_stub_answers_the_safety_stub_and_denies_read_only() {
    let pool = ClassifyPool::new(2);
    assert_eq!(pool.threads(), 2);
    let report = pool.classify("SELECT 1".into(), ctx()).await.unwrap();
    assert_eq!(report, stub_report());
    assert!(report.pre_decision.is_some());
    assert!(matches!(
        pool.check_read_only(DbmsKind::Postgres, "SELECT 1".into(), Arc::from(Vec::new()))
            .await,
        RoCheck::Deny(_)
    ));
    assert_eq!(
        pool.effects(DbmsKind::Postgres, "SET a = 1".into()).await,
        None
    );
}

// ---------------------------------------------------------------------------------------------
// The run service (Task 16)

#[tokio::test]
async fn run_request_m3a_shape_still_builds() {
    let (b, s, f) = service_over(fake_pg()).await;
    let m3a_request = || b.req(&["SELECT 1"]);
    let r = RunRequest {
        prepared: None,
        plan: None,
        ..m3a_request()
    };
    assert_eq!(r.policy, ErrorPolicy::StopOnError);
    let mut t = b.svc.run(&s, r);
    let summary = (&mut t.done).await.unwrap();
    assert_eq!((summary.executed, summary.failed), (1, 0));
    assert!(!f.typed_log().is_empty());
}

#[tokio::test]
async fn a_request_with_m3b_fields_sends_nothing_until_task_16() {
    let (b, s, f) = service_over(fake_pg()).await;
    let before = f.call_log().len();
    let plan_ok = GatePlan::default();
    for r in [
        RunRequest {
            prepared: Some(vec![prepared("SELECT 1")]),
            ..b.req(&["SELECT 1"])
        },
        RunRequest {
            plan: Some(plan_ok.clone()),
            ..b.req(&["SELECT 1"])
        },
        RunRequest {
            run_as_one: true,
            ..b.req(&["SELECT 1"])
        },
    ] {
        let t = b.svc.run(&s, r);
        assert_eq!(t.seq, u64::MAX, "no request was sent");
        let summary = t.done.await.unwrap();
        assert_eq!(summary.executed, 0);
    }
    assert!(f.call_log()[before..].is_empty());
}

// ## m3b-t16-run-service (named exception: Task 16 fills Count rows; the stub test becomes the
// fail-closed half that stays: without a check, nothing is sent)
#[tokio::test]
async fn count_without_a_check_is_refused_and_sends_nothing() {
    let (b, s, f) = service_over(fake_pg()).await;
    let before = f.call_log().len();
    let e = b
        .svc
        .count(hacivat_exec::CountRequest {
            console: s.clone(),
            sessions: None,
            engine: DbmsKind::Postgres,
            sql: "SELECT COUNT(*) FROM t".into(),
        })
        .done
        .await
        .unwrap()
        .unwrap_err();
    assert!(matches!(e, hacivat_exec::CountError::Refused(_)), "{e:?}");
    assert!(f.call_log()[before..].is_empty());
    let doc = common::doc("SELECT * FROM t");
    assert_eq!(
        count_sql(&doc, &unit_at("SELECT * FROM t", 0)).as_deref(),
        Some("SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c")
    );
}

#[test]
fn log_lines_since_reports_a_gap() {
    let log = SessionLog::new();
    for _ in 0..3 {
        log.append(LogMsg::TxCommitted, None, None);
    }
    let (lines, gap) = log.lines_since(1);
    assert_eq!(lines.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![1, 2]);
    assert!(!gap);
    let (lines, gap) = log.lines_since(3);
    assert!(lines.is_empty() && !gap);
    for _ in 0..SessionLog::CAPACITY {
        log.append(LogMsg::TxRolledBack, None, None);
    }
    let (lines, gap) = log.lines_since(0);
    assert_eq!(lines.len(), SessionLog::CAPACITY);
    assert!(gap, "the oldest lines left the ring");
    assert_eq!(
        LogMsg::Evicted { rows: 3 }.fluent_id(),
        "cursor-closed-evicted"
    );
}

/// Review M3: lines a clear removed are no gap; lines the ring pushed out for room still are.
#[test]
fn a_cleared_log_is_no_gap() {
    let log = SessionLog::new();
    for _ in 0..3 {
        log.append(LogMsg::TxCommitted, None, None);
    }
    log.clear();
    let (lines, gap) = log.lines_since(1);
    assert!(lines.is_empty());
    assert!(!gap, "nothing was dropped, the user cleared the log");
    // The ring pushes out lines 3 and 4 (`CAPACITY + 2` lines after the 3 cleared).
    for _ in 0..SessionLog::CAPACITY + 2 {
        log.append(LogMsg::TxRolledBack, None, None);
    }
    log.clear();
    assert!(log.lines_since(4).1, "line 4 was pushed out for room");
    assert!(!log.lines_since(5).1, "line 5 was cleared, not dropped");
}
