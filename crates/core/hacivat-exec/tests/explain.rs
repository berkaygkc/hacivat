//! M3b Task 15: the D-062 wrap (R-M3-21, R-M3-55): `EXPLAIN ANALYZE` of a non-read runs
//! inside a rolled-back transaction (the session builds it, Task 19), refused for `Routine` and
//! `Admin` inner classes and, on MySQL/MariaDB, for a DDL inner (its implicit commit escapes
//! the rollback). An `Unknown` inner gets the wrap.

mod common;

use common::gate_fx::{
    conn, conn_dev, conn_prod, conn_prod_user_tx, conn_ro, decision, fake_stmt, item_of, over,
    prepared_on, reason, stmt,
};
use hacivat_exec::{ConnSafety, GateDecision, SuppressSet, explain_analyze_inner, gate};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::UnitVerdict;
use sql_dialect::DialectId;
use sql_safety::{EffectSet, GateClass, ReasonCode, RiskFlags, StmtKind};

fn refused_id(d: &GateDecision) -> &'static str {
    match d {
        GateDecision::Deny(r) => r.fluent_id,
        other => panic!("not denied: {other:?}"),
    }
}

#[test]
fn explain_analyze_of_delete_is_wrapped_and_of_call_refused() {
    for c in [conn_dev(), conn_prod(), conn_prod_user_tx()] {
        let item = item_of("EXPLAIN ANALYZE DELETE FROM t WHERE id = 1", &c);
        assert_eq!(item.verdict, UnitVerdict::ExplainAnalyzeRollback);
        assert_eq!(item.class, GateClass::Dml);
        // ## m3b-t26-prod-unlock (fix round 1, B-M2): on production it asks first, naming what
        // the rollback does not undo (row locks, bloat, a trigger's side effects); elsewhere
        // wrapped at once. Even a DELETE without WHERE: nobody types a name.
        if c.env == Some(EnvTag::Production) {
            assert_confirms_not_undone(&item.decision);
            assert_confirms_not_undone(&decision("EXPLAIN ANALYZE DELETE FROM t", c));
        } else {
            assert_eq!(item.decision, GateDecision::ExplainAnalyzeRollback);
            assert_eq!(
                decision("EXPLAIN ANALYZE DELETE FROM t", c),
                GateDecision::ExplainAnalyzeRollback
            );
        }
        assert_eq!(
            refused_id(&decision("EXPLAIN ANALYZE CALL p()", c)),
            "exec-explain-analyze-refused"
        );
    }
}

#[test]
fn admin_inners_are_refused_and_unknown_inners_wrapped() {
    for c in [conn_dev(), conn_prod()] {
        assert_eq!(
            refused_id(&decision("EXPLAIN ANALYZE COPY t TO PROGRAM 'x'", c)),
            "exec-explain-analyze-refused"
        );
        // A function Hacivat cannot classify: wrapped; on production it asks first (fix round
        // 1, I2), because the rollback does not undo everything it may do.
        let item = item_of("EXPLAIN ANALYZE SELECT f_write()", &c);
        assert_eq!(item.verdict, UnitVerdict::ExplainAnalyzeRollback);
        assert_eq!(item.class, GateClass::Unknown);
        if c.env == Some(EnvTag::Production) {
            assert_confirms_not_undone(&item.decision);
        } else {
            assert_eq!(item.decision, GateDecision::ExplainAnalyzeRollback);
        }
        // PostgreSQL rolls DDL back: `CREATE TABLE … AS` is wrapped (## m3b-t26-prod-unlock
        // fix round 1, B-M2: asked first on production).
        let d = decision("EXPLAIN ANALYZE CREATE TABLE x AS SELECT 1", c);
        if c.env == Some(EnvTag::Production) {
            assert_confirms_not_undone(&d);
        } else {
            assert_eq!(d, GateDecision::ExplainAnalyzeRollback);
        }
    }
    // MySQL/MariaDB commit before DDL, so a DDL inner would escape the rollback.
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let c = conn(engine, Some(EnvTag::Dev));
        assert_eq!(
            refused_id(&decision("EXPLAIN ANALYZE CREATE TABLE x AS SELECT 1", c)),
            "exec-explain-analyze-refused"
        );
        assert_eq!(
            decision("EXPLAIN ANALYZE UPDATE t SET a = 1 WHERE id = 1", c),
            GateDecision::ExplainAnalyzeRollback
        );
        // On production MySQL/MariaDB every wrapped write asks first (M7: a rollback does not
        // undo writes to non-transactional tables).
        let prod = conn(engine, Some(EnvTag::Production));
        let item = item_of("EXPLAIN ANALYZE UPDATE t SET a = 1 WHERE id = 1", &prod);
        assert_confirms_not_undone(&item.decision);
        assert_eq!(item.verdict, UnitVerdict::ExplainAnalyzeRollback);
    }
}

/// A production confirmation before the D-062 wrap: names what the rollback does not undo,
/// never suppressible, no typed name.
fn assert_confirms_not_undone(d: &GateDecision) {
    match d {
        GateDecision::Confirm {
            reasons,
            type_to_confirm: None,
            suppressible: false,
            irreversible: false,
        } => assert!(
            reasons
                .iter()
                .any(|r| r.fluent_id == "exec-explain-analyze-not-undone"),
            "{reasons:?}"
        ),
        other => panic!("not the wrap's confirmation: {other:?}"),
    }
}

#[test]
fn explain_analyze_of_a_read_runs_as_a_read() {
    for c in [conn_dev(), conn_prod(), conn_ro()] {
        let item = item_of("EXPLAIN ANALYZE SELECT * FROM t", &c);
        assert_eq!(item.decision, GateDecision::Run, "{c:?}");
        assert_eq!(item.verdict, UnitVerdict::Read);
    }
    // A plain EXPLAIN is a read too (it runs nothing).
    let mut s = stmt(StmtKind::Explain { analyze: false }, EffectSet::READ);
    assert_eq!(explain_analyze_inner(&s), None);
    s.kind = StmtKind::Select;
    assert_eq!(explain_analyze_inner(&s), None);
}

#[test]
fn read_only_denies_explain_analyze_of_a_write() {
    let ro_prod = ConnSafety {
        read_only: true,
        ..conn_prod()
    };
    for c in [conn_ro(), ro_prod] {
        for s in [
            "EXPLAIN ANALYZE DELETE FROM t WHERE id = 1",
            "EXPLAIN ANALYZE SELECT f_write()",
            "EXPLAIN ANALYZE CALL p()",
        ] {
            assert_eq!(refused_id(&decision(s, c)), "exec-gate-read-only", "{s}");
        }
    }
}

#[test]
fn explain_analyze_inner_reads_kind_flag_or_reason() {
    let dml = fake_stmt("EXPLAIN ANALYZE UPDATE t SET a = 1 WHERE id = 1");
    assert_eq!(explain_analyze_inner(&dml), Some(GateClass::Dml));
    // Only the flag, or only the reason, says so too.
    let mut flagged = stmt(StmtKind::Other, EffectSet::WRITE_DATA);
    flagged.flags = RiskFlags::EXPLAIN_ANALYZE;
    assert_eq!(explain_analyze_inner(&flagged), Some(GateClass::Dml));
    let mut reasoned = stmt(StmtKind::Other, EffectSet::ROUTINE_CALL);
    reasoned
        .reasons
        .push(reason(ReasonCode::ExplainAnalyze, None));
    assert_eq!(explain_analyze_inner(&reasoned), Some(GateClass::Routine));
    // An empty inner effect set is Unknown.
    let empty = stmt(StmtKind::Explain { analyze: true }, EffectSet::empty());
    assert_eq!(explain_analyze_inner(&empty), Some(GateClass::Unknown));
}

#[test]
fn explain_analyze_of_a_write_inside_a_script_is_refused() {
    // Wrapping a unit of several statements would roll the others back too.
    let text = "SELECT 1; EXPLAIN ANALYZE DELETE FROM t WHERE id = 1";
    let unit = prepared_on(DbmsKind::Postgres, text, &[]);
    let report = over(
        text,
        DialectId::Postgres,
        vec![
            fake_stmt("SELECT 1"),
            fake_stmt("EXPLAIN ANALYZE DELETE FROM t WHERE id = 1"),
        ],
    );
    for c in [conn_dev(), conn_prod()] {
        let item = gate(
            std::slice::from_ref(&unit),
            std::slice::from_ref(&report),
            &c,
            &SuppressSet::default(),
            1,
        )
        .items
        .remove(0);
        assert_eq!(refused_id(&item.decision), "exec-explain-analyze-refused");
    }
}
