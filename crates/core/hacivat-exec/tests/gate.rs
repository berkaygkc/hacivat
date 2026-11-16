//! M3b Task 15: the exec gate (HV-EXEC-04, R-M3-55, R-M3-84; R-M3-23 ANDed). Reports are built
//! by hand over the exact final text (P-M3b-16, `common::gate_fx`); the end-to-end test over
//! the real classifier is `#[ignore]`d until Task 28 turns it on.

mod common;

use std::sync::Arc;

use common::gate_fx::{
    conn, conn_dev, conn_prod, conn_prod_auto, conn_prod_manual_idle, conn_prod_user_tx, conn_ro,
    conn_staging, decision, dialect_of, fake_classify, fake_stmt, item_of, object, over,
    prepared_on, reason, report, stmt,
};
use hacivat_driver::TxStatus;
use hacivat_exec::{
    ConnSafety, GateDecision, GateItem, GatePlan, PreparedUnit, SafetyReport, SuppressSet,
    apply_read_verdicts, gate, reads_only, suspendable_for,
};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::session::{SessionEffects, TxMode, TxOwner, UnitVerdict};
use proptest::prelude::*;
use sql_dialect::DialectId;
use sql_safety::{
    Actor, CoarseClass, DestructiveKind, DestructiveReason, EffectSet, GateClass, ObjectKind,
    Profile, ReasonCode, RiskFlags, StmtKind, StmtReport, decide, gate_class,
};
use sql_text::TextRange;

// ---------------------------------------------------------------------------------------------
// Helpers

fn one(unit: PreparedUnit, report: SafetyReport, conn: &ConnSafety) -> GateItem {
    gate(&[unit], &[report], conn, &SuppressSet::default(), 1)
        .items
        .remove(0)
}

/// `items` over `text` on PostgreSQL, gated on `conn`.
fn gated(text: &str, items: Vec<StmtReport>, conn: &ConnSafety) -> GateItem {
    let unit = prepared_on(conn.engine, text, &[]);
    let report = over(&unit.final_text, dialect_of(conn.engine), items);
    one(unit, report, conn)
}

/// A short name for a decision, for table assertions.
fn shape(d: &GateDecision) -> &'static str {
    match d {
        GateDecision::Run => "run",
        GateDecision::Warn(_) => "warn",
        GateDecision::Confirm {
            type_to_confirm: Some(_),
            ..
        } => "confirm+type",
        GateDecision::Confirm { .. } => "confirm",
        GateDecision::RunInTxThenConfirm { .. } => "tx-confirm",
        GateDecision::ExplainAnalyzeRollback => "explain-wrap",
        GateDecision::Deny(_) => "deny",
    }
}

fn deny_id(d: &GateDecision) -> &'static str {
    match d {
        GateDecision::Deny(r) => r.fluent_id,
        other => panic!("not denied: {other:?}"),
    }
}

fn reason_ids(d: &GateDecision) -> Vec<&'static str> {
    match d {
        GateDecision::Warn(r)
        | GateDecision::Confirm { reasons: r, .. }
        | GateDecision::RunInTxThenConfirm { reasons: r } => {
            r.iter().map(|r| r.fluent_id).collect()
        }
        _ => Vec::new(),
    }
}

fn mysql(env: EnvTag) -> ConnSafety {
    conn(DbmsKind::MySql, Some(env))
}

// ---------------------------------------------------------------------------------------------
// The plan's tests (Task 15, Step 1)

/// `alg/20` HV-EXEC-04 :266-278, every row and level, as R-M3-55 rules it (R-M3-55 wins where
/// they differ: dev `DmlUnsafe` asks the calm confirmation, dev/staging DDL, routines and
/// unknown statements run, production `Session` runs).
#[test]
fn hv_exec_04_table_per_level() {
    let rows: [(&str, [&str; 3]); 11] = [
        // statement → (dev, staging, production)
        ("SELECT 1", ["run", "run", "run"]),
        ("SELECT * FROM t FOR UPDATE", ["run", "run", "warn"]),
        (
            "UPDATE t SET a = 1 WHERE id = 3",
            ["run", "run", "tx-confirm"],
        ),
        ("DELETE FROM orders", ["confirm", "confirm", "confirm+type"]),
        (
            "UPDATE orders SET a = 1",
            ["confirm", "confirm", "confirm+type"],
        ),
        ("DROP TABLE orders", ["run", "run", "confirm"]),
        ("DROP SCHEMA sales CASCADE", ["run", "run", "confirm+type"]),
        ("CREATE TABLE x (a int)", ["run", "run", "confirm"]),
        ("CALL p()", ["run", "run", "confirm"]),
        ("DO $$ BEGIN END $$", ["run", "run", "confirm"]),
        ("SET search_path = public", ["run", "run", "run"]),
    ];
    for (sql, expected) in rows {
        for (conn, want) in [conn_dev(), conn_staging(), conn_prod()]
            .iter()
            .zip(expected)
        {
            assert_eq!(
                shape(&decision(sql, *conn)),
                want,
                "{sql} on {:?}",
                conn.env
            );
        }
    }
    // Transaction control asks on production, runs elsewhere.
    assert_eq!(shape(&decision("BEGIN", conn_prod())), "confirm");
    assert_eq!(shape(&decision("BEGIN", conn_dev())), "run");
    // `EXPLAIN ANALYZE <x>`: the wrap at every level.
    for c in [conn_dev(), conn_staging(), conn_prod()] {
        assert_eq!(
            shape(&decision("EXPLAIN ANALYZE DELETE FROM t WHERE id = 1", c)),
            "explain-wrap"
        );
    }
    // A connection with no environment tag is not production.
    assert_eq!(
        shape(&decision(
            "UPDATE t SET a = 1 WHERE id = 3",
            conn(DbmsKind::Postgres, None)
        )),
        "run"
    );
}

#[test]
fn read_only_denies_every_non_read_class_including_call() {
    for s in [
        "INSERT INTO t VALUES (1)",
        "CALL p()",
        "SET x = 1",
        "CREATE TEMP TABLE x (a int)",
    ] {
        assert!(
            matches!(decision(s, conn_ro()), GateDecision::Deny(_)),
            "{s}"
        );
    }
    // Every class but `Read` (and the harmless SET list), named in the denial.
    for (s, class) in [
        ("SELECT * FROM t FOR UPDATE", "lock"),
        ("UPDATE t SET a = 1 WHERE id = 1", "dml"),
        ("DELETE FROM t", "dml"),
        ("DROP TABLE t", "ddl"),
        ("GRANT SELECT ON t TO bob", "admin"),
        ("CALL p()", "routine"),
        ("BEGIN", "tx"),
        ("DO $$ BEGIN END $$", "unknown"),
        ("SELECT f_write()", "unknown"),
        ("EXPLAIN ANALYZE DELETE FROM t WHERE id = 1", "dml"),
    ] {
        match decision(s, conn_ro()) {
            GateDecision::Deny(r) => {
                assert_eq!(r.fluent_id, "exec-gate-read-only", "{s}");
                assert_eq!(r.args, vec![("class", Arc::from(class))], "{s}");
            }
            other => panic!("{s}: {other:?}"),
        }
    }
    // Production read-only (every production console until Task 26) is read-only first.
    let prod_ro = ConnSafety {
        read_only: true,
        ..conn_prod()
    };
    assert!(matches!(
        decision("UPDATE t SET a = 1 WHERE id = 3", prod_ro),
        GateDecision::Deny(_)
    ));
    let read = item_of("SELECT 1", &prod_ro);
    assert_eq!(read.decision, GateDecision::Run);
    assert_eq!(read.verdict, UnitVerdict::Read);
}

#[test]
fn production_delete_without_where_asks_to_type_the_table() {
    let d = decision("DELETE FROM müşteri", conn_prod());
    assert!(
        matches!(d, GateDecision::Confirm { type_to_confirm: Some(ref t), suppressible: false, .. } if &**t == "müşteri"),
        "{d:?}"
    );
    // The plan's builder, as written.
    let item = gated(
        "DELETE FROM müşteri",
        vec![report(
            StmtKind::Delete,
            EffectSet::WRITE_DATA,
            &[DestructiveKind::NoWhere],
            "müşteri",
        )],
        &conn_prod(),
    );
    assert!(matches!(
        item.decision,
        GateDecision::Confirm { type_to_confirm: Some(ref t), irreversible: false, .. } if &**t == "müşteri"
    ));
    assert_eq!(item.class, GateClass::DmlUnsafe);
    assert_eq!(item.verdict, UnitVerdict::Write);
    assert_eq!(
        reason_ids(&item.decision)[0],
        "safety-destructive-no-where",
        "one reason per destructive kind, first"
    );
}

/// The name to type is the user's spelling, byte for byte, even when the classifier carries
/// the server-resolved (folded) name (T9 carry): never case-folded, quotes removed.
#[test]
fn type_to_confirm_is_the_name_as_written() {
    for (sql, folded, want) in [
        ("DELETE FROM Müşteri", "müşteri", "Müşteri"),
        ("DELETE FROM Sales.Orders", "sales.orders", "Sales.Orders"),
        ("DELETE FROM \"Orders\"", "Orders", "Orders"),
        ("UPDATE İller SET a = 1", "İller", "İller"),
    ] {
        let mut s = stmt(StmtKind::Delete, EffectSet::WRITE_DATA);
        let mut target = object("x", ObjectKind::Table);
        target.name = sql_ident::QualifiedName {
            parts: folded
                .split('.')
                .map(|p| sql_ident::Ident {
                    text: Arc::from(p),
                    quoted: true,
                })
                .collect(),
        };
        s.destructive.push(DestructiveReason {
            kind: DestructiveKind::NoWhere,
            range: TextRange::default(),
            target: Some(target),
        });
        let item = gated(sql, vec![s], &conn_prod());
        match &item.decision {
            GateDecision::Confirm {
                type_to_confirm: Some(t),
                ..
            } => assert_eq!(&**t, want, "{sql}"),
            other => panic!("{sql}: {other:?}"),
        }
    }
    // Off production nobody types.
    assert!(matches!(
        decision("DELETE FROM müşteri", conn_dev()),
        GateDecision::Confirm {
            type_to_confirm: None,
            ..
        }
    ));
}

/// From the destructive kinds, not the class: a `DROP` in the same unit raises the class to
/// `DdlDestructive`, which must not hide the unsafe `DELETE`'s typed name.
#[test]
fn ddl_does_not_hide_an_unsafe_dml_name() {
    let text = "DELETE FROM orders; DROP TABLE logs";
    let items = vec![
        fake_stmt("DELETE FROM orders"),
        fake_stmt("DROP TABLE logs"),
    ];
    let item = gated(text, items, &conn_prod());
    assert_eq!(item.class, GateClass::DdlDestructive);
    assert!(matches!(
        item.decision,
        GateDecision::Confirm { type_to_confirm: Some(ref t), suppressible: false, .. } if &**t == "orders"
    ));
    // Two unsafe tables in one unit: typing one name must not approve the other.
    let items = vec![fake_stmt("DELETE FROM a"), fake_stmt("DELETE FROM b")];
    let item = gated("DELETE FROM a; DELETE FROM b", items, &conn_prod());
    assert_eq!(deny_id(&item.decision), "exec-unsafe-several-targets");
    // An unsafe pattern with no target names nothing to type: denied.
    let mut s = stmt(StmtKind::Delete, EffectSet::WRITE_DATA);
    s.destructive.push(DestructiveReason {
        kind: DestructiveKind::AlwaysTrue,
        range: TextRange::default(),
        target: None,
    });
    let item = gated("DELETE FROM t WHERE 1 = 1", vec![s], &conn_prod());
    assert_eq!(deny_id(&item.decision), "exec-unsafe-no-target");
}

#[test]
fn production_dml_is_run_in_tx_then_confirm_in_every_tx_state() {
    for c in [
        conn_prod_auto(),
        conn_prod_manual_idle(),
        conn_prod_user_tx(),
    ] {
        assert!(matches!(
            decision("UPDATE t SET a = 1 WHERE id = 3", c),
            GateDecision::RunInTxThenConfirm { .. }
        ));
        let item = item_of("INSERT INTO t VALUES (1)", &c);
        assert!(matches!(
            item.decision,
            GateDecision::RunInTxThenConfirm { .. }
        ));
        // The form is the session's (P-M3b-1): the verdict carries none.
        assert_eq!(item.verdict, UnitVerdict::TxThenConfirm);
        assert!(item.effects.contains(SessionEffects::OPENS_WRITE));
    }
    // MySQL too, inside a user transaction (the session takes the savepoint form).
    let my = ConnSafety {
        tx_status: TxStatus::InTx,
        tx_owner: TxOwner::User,
        ..mysql(EnvTag::Production)
    };
    assert!(matches!(
        decision("UPDATE t SET a = 1 WHERE id = 3", my),
        GateDecision::RunInTxThenConfirm { .. }
    ));
}

#[test]
fn admin_class_confirms_on_production_and_denies_read_only() {
    for s in [
        "GRANT SELECT ON t TO bob",
        "COPY t TO PROGRAM 'gzip > /tmp/x'",
    ] {
        let item = item_of(s, &conn_prod());
        assert_eq!(item.class, GateClass::Admin, "{s}");
        assert!(
            matches!(
                item.decision,
                GateDecision::Confirm {
                    type_to_confirm: None,
                    ..
                }
            ),
            "{s}: {:?}",
            item.decision
        );
        assert_eq!(item.verdict, UnitVerdict::Write, "{s}");
        assert_eq!(deny_id(&decision(s, conn_ro())), "exec-gate-read-only");
        // Off production it runs (R-M3-55; P-M3b-4).
        assert_eq!(decision(s, conn_dev()), GateDecision::Run, "{s}");
    }
}

#[test]
fn read_only_allows_set_search_path_only_from_the_list() {
    let item = item_of("SET search_path = public", &conn_ro());
    assert_eq!(item.decision, GateDecision::Run);
    assert_eq!(item.class, GateClass::Session);
    assert!(item.effects.contains(SessionEffects::SESSION_STATE));
    assert_ne!(item.verdict, UnitVerdict::Read, "a SET is no read");
    for s in [
        "SET statement_timeout = 5000",
        "SET work_mem = '64MB'",
        "SET application_name = 'x'",
    ] {
        assert_eq!(decision(s, conn_ro()), GateDecision::Run, "{s}");
    }
    for s in [
        "SET enable_seqscan = off",
        "SET role = admin",
        "SET default_transaction_read_only = off",
        "SET sql_mode = ''",
    ] {
        assert_eq!(
            deny_id(&decision(s, conn_ro())),
            "exec-gate-read-only",
            "{s}"
        );
    }
    // No dialect on the report: nothing is on the list.
    let unit = prepared_on(DbmsKind::Postgres, "SET search_path = public", &[]);
    let mut report = fake_classify(&unit.final_text, DialectId::Postgres);
    report.dialect = None;
    assert!(matches!(
        one(unit, report, &conn_ro()).decision,
        GateDecision::Deny(_)
    ));
}

#[test]
fn non_production_unsafe_where_is_calm_confirm_and_setting_turns_it_off() {
    for c in [conn_dev(), conn_staging(), conn(DbmsKind::Postgres, None)] {
        for s in ["DELETE FROM orders", "UPDATE orders SET a = 1"] {
            assert!(
                matches!(
                    decision(s, c),
                    GateDecision::Confirm {
                        type_to_confirm: None,
                        suppressible: true,
                        irreversible: false,
                        ..
                    }
                ),
                "{s}"
            );
        }
        let off = ConnSafety {
            confirm_unsafe_where: false,
            ..c
        };
        let item = item_of("DELETE FROM orders", &off);
        assert_eq!(item.decision, GateDecision::Run);
        assert_eq!(item.verdict, UnitVerdict::Write);
    }
    // The setting is the non-production guard only: production still asks to type.
    let prod_off = ConnSafety {
        confirm_unsafe_where: false,
        ..conn_prod()
    };
    assert_eq!(
        shape(&decision("DELETE FROM orders", prod_off)),
        "confirm+type"
    );
}

#[test]
fn non_production_everything_else_runs() {
    for c in [
        conn_dev(),
        conn_staging(),
        conn(DbmsKind::Postgres, Some(EnvTag::Local)),
        conn(DbmsKind::Postgres, Some(EnvTag::Test)),
        conn(DbmsKind::Sqlite, Some(EnvTag::Dev)),
        mysql(EnvTag::Dev),
    ] {
        for s in [
            "SELECT 1",
            "SELECT * FROM t FOR UPDATE",
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a = 1 WHERE id = 3",
            "DELETE FROM t WHERE id = 3",
            "CREATE TABLE x (a int)",
            "DROP TABLE x",
            "TRUNCATE t",
            "CALL p()",
            "DO 1", // an unknown statement (`$$` is a SQLite parameter)
            "GRANT SELECT ON t TO bob",
            "SET x = 1",
            "BEGIN",
            "SELECT f_write()",
        ] {
            assert_eq!(decision(s, c), GateDecision::Run, "{s} on {c:?}");
        }
    }
}

#[test]
fn mysql_ddl_with_open_manual_tx_confirms_implicit_commit() {
    let manual = ConnSafety {
        tx_mode: TxMode::Manual,
        ..mysql(EnvTag::Dev)
    };
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb, DbmsKind::TiDb] {
        let c = ConnSafety { engine, ..manual };
        for s in [
            "CREATE TABLE x (a int)",
            "ALTER TABLE t ADD b int",
            "TRUNCATE t",
            "GRANT SELECT ON t TO bob",
        ] {
            let item = item_of(s, &c);
            match &item.decision {
                GateDecision::Confirm {
                    irreversible: true,
                    suppressible: false,
                    type_to_confirm: None,
                    reasons,
                } => assert!(
                    reasons
                        .iter()
                        .any(|r| r.fluent_id == "exec-gate-implicit-commit"),
                    "{s}"
                ),
                other => panic!("{engine:?} {s}: {other:?}"),
            }
            assert_eq!(item.verdict, UnitVerdict::ImplicitCommit, "{s}");
            assert!(item.effects.contains(SessionEffects::DDL), "{s}");
        }
    }
    // A user transaction in Auto mode counts as open too; an idle Auto session does not.
    let in_tx = ConnSafety {
        tx_status: TxStatus::InTx,
        tx_owner: TxOwner::User,
        ..mysql(EnvTag::Dev)
    };
    assert_eq!(
        item_of("CREATE TABLE x (a int)", &in_tx).verdict,
        UnitVerdict::ImplicitCommit
    );
    let idle = item_of("CREATE TABLE x (a int)", &mysql(EnvTag::Dev));
    assert_eq!(idle.decision, GateDecision::Run);
    assert_eq!(idle.verdict, UnitVerdict::Write);
    // Production MySQL DDL on an idle Auto session: a normal DDL confirmation that says it
    // cannot be rolled back, and may be suppressed.
    let prod = item_of("CREATE TABLE x (a int)", &mysql(EnvTag::Production));
    assert!(matches!(
        prod.decision,
        GateDecision::Confirm {
            irreversible: true,
            suppressible: true,
            ..
        }
    ));
    assert_eq!(prod.verdict, UnitVerdict::Write);
    // PostgreSQL DDL in a transaction: no implicit commit, nothing irreversible.
    let pg = item_of("CREATE TABLE x (a int)", &conn_prod_user_tx());
    assert!(matches!(
        pg.decision,
        GateDecision::Confirm {
            irreversible: false,
            suppressible: true,
            ..
        }
    ));
    assert_eq!(pg.verdict, UnitVerdict::Write);
    // A PostgreSQL GRANT is no DDL for the session; a MySQL one commits like one.
    assert!(
        !item_of("GRANT SELECT ON t TO bob", &conn_dev())
            .effects
            .contains(SessionEffects::DDL)
    );
}

#[test]
fn unknown_on_production_confirms_never_runs() {
    let unknowns: Vec<(&str, Vec<StmtReport>)> = vec![
        ("SELECT f_write()", vec![fake_stmt("SELECT f_write()")]),
        ("DO $$ BEGIN END $$", vec![fake_stmt("DO $$ BEGIN END $$")]),
        ("CALL p()", vec![fake_stmt("CALL p()")]),
        // An empty effect set, an unnamed bit, an unnamed flag.
        ("SELECT 1", vec![stmt(StmtKind::Select, EffectSet::empty())]),
        (
            "SELECT 1",
            vec![stmt(StmtKind::Select, EffectSet::from_bits_retain(1 << 20))],
        ),
        ("SELECT 1", {
            let mut s = stmt(StmtKind::Select, EffectSet::READ);
            s.flags = RiskFlags::from_bits_retain(1 << 9);
            vec![s]
        }),
        // An executable comment (D-058: production asks before hidden code).
        ("SELECT 1", {
            let mut s = stmt(StmtKind::Select, EffectSet::READ);
            s.flags = RiskFlags::EXEC_COMMENT;
            vec![s]
        }),
    ];
    for (text, items) in unknowns {
        for c in [
            conn_prod_auto(),
            conn_prod_manual_idle(),
            conn_prod_user_tx(),
        ] {
            let item = gated(text, items.clone(), &c);
            assert!(
                matches!(item.decision, GateDecision::Confirm { .. }),
                "{text} {items:?}: {:?}",
                item.decision
            );
            assert_eq!(item.verdict, UnitVerdict::Unknown);
            // Unknown session effects: everything but DDL.
            assert!(item.effects.contains(
                SessionEffects::OPENS_WRITE
                    | SessionEffects::SESSION_STATE
                    | SessionEffects::WRITE_TEMP
            ));
        }
        assert!(matches!(
            gated(text, items, &conn_ro()).decision,
            GateDecision::Deny(_)
        ));
    }
    // Doubt the policy denies everywhere (a parse error, a lexical-mode change, a role
    // switch) is denied on production rather than confirmed: decide(Admin) must not deny.
    for code in [
        ReasonCode::ParseError,
        ReasonCode::LexModeChange,
        ReasonCode::PrivilegeChange,
        ReasonCode::AmbiguousBoundary,
    ] {
        let mut s = stmt(StmtKind::Select, EffectSet::READ);
        s.reasons.push(reason(code, None));
        let item = gated("SELECT 1", vec![s.clone()], &conn_prod());
        assert_eq!(deny_id(&item.decision), code.fluent_id(), "{code:?}");
        // Off production the server decides (R-M3-55 "everything else runs").
        assert_eq!(
            gated("SELECT 1", vec![s], &conn_dev()).decision,
            GateDecision::Run
        );
    }
}

/// The AND-check: request-level evidence the statements do not show still counts, and a
/// `Confirm` from `decide(Admin)` on a class whose own row allows makes the unit `Unknown`.
#[test]
fn production_and_check_reads_the_request_level_fields() {
    let unit = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    let base = over(
        "SELECT 1",
        DialectId::Postgres,
        vec![stmt(StmtKind::Select, EffectSet::READ)],
    );
    assert_eq!(
        one(unit.clone(), base.clone(), &conn_prod()).decision,
        GateDecision::Run
    );
    let mut r = base.clone();
    r.request_effects |= EffectSet::UNKNOWN;
    let item = one(unit.clone(), r, &conn_prod());
    assert_eq!(shape(&item.decision), "confirm");
    assert_eq!(item.verdict, UnitVerdict::Unknown);
    let mut r = base.clone();
    r.flags |= RiskFlags::DYNAMIC_SQL;
    assert_eq!(
        shape(&one(unit.clone(), r, &conn_prod()).decision),
        "confirm"
    );
    let mut r = base.clone();
    r.request_class = CoarseClass::Write;
    assert_ne!(
        one(unit.clone(), r.clone(), &conn_prod()).decision,
        GateDecision::Run
    );
    assert!(matches!(
        one(unit.clone(), r, &conn_ro()).decision,
        GateDecision::Deny(_)
    ));
    let mut r = base.clone();
    r.flags |= RiskFlags::LEX_MODE_CHANGE;
    assert_eq!(
        deny_id(&one(unit.clone(), r, &conn_prod()).decision),
        "safety-reason-lex-mode-change"
    );
    // A DML run as one with a SET: decide(Admin) confirms the SET, so the unit asks first
    // (Unknown) instead of running in a transaction.
    let text = "SET x = 1; UPDATE t SET a = 1 WHERE id = 1";
    let items = vec![
        fake_stmt("SET x = 1"),
        fake_stmt("UPDATE t SET a = 1 WHERE id = 1"),
    ];
    let item = gated(text, items, &conn_prod());
    assert_eq!(shape(&item.decision), "confirm");
    assert_eq!(item.class, GateClass::Unknown);
    assert_eq!(item.verdict, UnitVerdict::Unknown);
    // A pre-decided report (other than the stub's) is denied on production, runs elsewhere.
    let unit = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    let mut r = base;
    r.pre_decision = Some(vec![reason(ReasonCode::TooComplex, None)]);
    r.items.clear();
    assert_eq!(
        deny_id(&one(unit.clone(), r.clone(), &conn_prod()).decision),
        "safety-reason-too-complex"
    );
    let dev = one(unit, r, &conn_dev());
    assert_eq!(dev.decision, GateDecision::Run);
    assert_eq!(dev.verdict, UnitVerdict::Unknown);
}

/// The classifier stub's report (`NotImplemented`) never runs, on any connection (F31).
#[test]
fn the_classifier_stub_is_denied_everywhere() {
    let unit = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    let ctx = sql_safety::ClassifyCtx::new(
        sql_dialect::DialectCtx::new(DialectId::Postgres, None),
        sql_dialect::ClientFlavor::None,
        sql_safety::Expect::SingleStatement,
    );
    let mut stub = sql_safety::classify(&unit.final_text, &ctx);
    if stub.pre_decision.is_none() {
        // Task 14 merged: the real classifier; build the stub's answer by hand.
        stub = SafetyReport::pre_decided(
            &unit.final_text,
            stub.fingerprint,
            vec![reason(ReasonCode::NotImplemented, None)],
        );
    }
    for c in [conn_dev(), conn_staging(), conn_prod(), conn_ro()] {
        assert_eq!(
            deny_id(&one(unit.clone(), stub.clone(), &c).decision),
            "exec-gate-not-classified"
        );
    }
}

/// Generated reports × connections (R-M3-55, R-M3-56): no path turns a policy `Deny` into a
/// `Run`, `Warn` or `Confirm`.
#[test]
fn no_path_turns_deny_into_run() {
    let cfg = ProptestConfig {
        cases: 4_000,
        ..ProptestConfig::default()
    };
    proptest!(cfg, |(items in prop::collection::vec(arb_stmt(), 1..4), ci in 0usize..6)| {
        let conns = [
            conn_ro(),
            ConnSafety { read_only: true, ..conn_prod() },
            conn_prod_auto(),
            conn_prod_user_tx(),
            ConnSafety { read_only: true, ..mysql(EnvTag::Staging) },
            ConnSafety { tx_mode: TxMode::Manual, ..mysql(EnvTag::Production) },
        ];
        let c = conns[ci];
        let text = "SELECT 1";
        let unit = prepared_on(c.engine, text, &[]);
        let report = over(text, dialect_of(c.engine), items);
        let item = one(unit, report.clone(), &c);
        let max = report.items.iter().map(gate_class).max().unwrap();
        if c.read_only {
            let rope = decide(&report, Profile::ReadOnlyPlusExplain, Actor::Human);
            if !rope.allows() {
                prop_assert!(matches!(item.decision, GateDecision::Deny(_)), "{:?}", item);
            }
            // The stricter `ReadOnly` profile differs only for EXPLAIN ANALYZE of a read,
            // which the human read-only profile allows (R-M3-56).
            if !decide(&report, Profile::ReadOnly, Actor::Human).allows()
                && !matches!(item.decision, GateDecision::Deny(_))
            {
                prop_assert!(rope.allows());
                let explains = report.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
                    || report.items.iter().any(|s| s.reasons.iter().any(|r| r.code == ReasonCode::ExplainAnalyze))
                    || report.items.iter().any(|s| matches!(s.kind, StmtKind::Explain { analyze: true }));
                prop_assert!(explains);
            }
            if item.decision == GateDecision::Run {
                prop_assert!(max <= GateClass::Session);
            }
            let asks = matches!(item.decision, GateDecision::Warn(_) | GateDecision::Confirm { .. } | GateDecision::RunInTxThenConfirm { .. } | GateDecision::ExplainAnalyzeRollback);
            prop_assert!(!asks, "a read-only connection never asks: {:?}", item);
        } else {
            let admin = decide(&report, Profile::Admin, Actor::Human);
            if matches!(admin.verdict, sql_safety::Verdict::Deny(_)) {
                prop_assert!(matches!(item.decision, GateDecision::Deny(_)), "{:?}", item);
            }
            if item.decision == GateDecision::Run {
                prop_assert!(max <= GateClass::Session, "{:?}", item);
                prop_assert!(!matches!(admin.verdict, sql_safety::Verdict::Deny(_)));
            }
            if max == GateClass::Unknown {
                prop_assert_ne!(&item.decision, &GateDecision::Run);
            }
        }
        // A `Read` verdict always reads only (R-M3-23).
        if item.verdict == UnitVerdict::Read {
            prop_assert!(reads_only(&report));
        }
    });
}

fn arb_stmt() -> impl Strategy<Value = StmtReport> {
    let kinds = vec![
        StmtKind::Select,
        StmtKind::Insert,
        StmtKind::Update,
        StmtKind::Delete,
        StmtKind::Call,
        StmtKind::Do,
        StmtKind::Explain { analyze: true },
        StmtKind::Explain { analyze: false },
        StmtKind::CreateTable,
        StmtKind::Alter,
        StmtKind::Drop,
        StmtKind::Truncate,
        StmtKind::Grant,
        StmtKind::Set,
        StmtKind::Begin,
        StmtKind::Lock,
        StmtKind::Copy,
        StmtKind::Other,
    ];
    (
        prop::sample::select(kinds),
        any::<u32>(),
        any::<u16>(),
        prop::sample::select(CoarseClass::ALL.to_vec()),
        prop::collection::vec(prop::sample::select(ReasonCode::ALL.to_vec()), 0..3),
        prop::collection::vec(prop::sample::select(DestructiveKind::ALL.to_vec()), 0..2),
        prop::sample::select(vec!["search_path", "work_mem", "enable_seqscan", "role"]),
        any::<bool>(),
    )
        .prop_map(
            |(kind, bits, flags, class, codes, destructive, set, unnamed)| {
                // Mostly named bits, sometimes an unnamed one.
                let mask = if unnamed {
                    u32::MAX
                } else {
                    EffectSet::all().bits()
                };
                let mut s = stmt(kind, EffectSet::from_bits_retain(bits & mask & 0x8007_ffff));
                s.flags =
                    RiskFlags::from_bits_retain(flags & if unnamed { u16::MAX } else { 0x1f });
                s.class = class;
                s.reasons = codes.into_iter().map(|c| reason(c, Some("f"))).collect();
                s.destructive = destructive
                    .into_iter()
                    .map(|k| DestructiveReason {
                        kind: k,
                        range: TextRange::default(),
                        target: Some(object("t", ObjectKind::Table)),
                    })
                    .collect();
                if kind == StmtKind::Set {
                    s.set_targets.push(sql_safety::SetTarget {
                        name: Arc::from(set),
                        value: Some(Arc::from("1")),
                    });
                }
                s
            },
        )
}

#[test]
fn suppression_keyed_by_console_and_fingerprint() {
    let run = |text: &str, c: &ConnSafety, s: &SuppressSet, console: u64| {
        let unit = prepared_on(c.engine, text, &[]);
        let report = fake_classify(&unit.final_text, dialect_of(c.engine));
        gate(&[unit], &[report], c, s, console).items.remove(0)
    };
    let first = run(
        "DELETE FROM orders",
        &conn_dev(),
        &SuppressSet::default(),
        7,
    );
    assert!(matches!(
        first.decision,
        GateDecision::Confirm {
            suppressible: true,
            ..
        }
    ));
    let mut s = SuppressSet::default();
    s.insert(7, first.fingerprint);
    assert_eq!(
        run("DELETE FROM orders", &conn_dev(), &s, 7).decision,
        GateDecision::Run
    );
    // Another console, another shape: still asked.
    assert_ne!(
        run("DELETE FROM orders", &conn_dev(), &s, 8).decision,
        GateDecision::Run
    );
    assert_ne!(
        run("DELETE FROM other", &conn_dev(), &s, 7).decision,
        GateDecision::Run
    );
    // Production: a type-to-confirm unit is never suppressed, even with its pair recorded.
    let prod = run("DELETE FROM orders", &conn_prod(), &s, 7);
    assert!(matches!(
        prod.decision,
        GateDecision::Confirm {
            type_to_confirm: Some(_),
            suppressible: false,
            ..
        }
    ));
    // A suppressible production confirmation and warning ("don't ask again in this console").
    for text in ["DROP TABLE orders", "SELECT * FROM t FOR UPDATE"] {
        let asked = run(text, &conn_prod(), &SuppressSet::default(), 7);
        assert!(matches!(
            asked.decision,
            GateDecision::Confirm {
                suppressible: true,
                ..
            } | GateDecision::Warn(_)
        ));
        let mut s = SuppressSet::default();
        s.insert(7, asked.fingerprint);
        assert_eq!(
            run(text, &conn_prod(), &s, 7).decision,
            GateDecision::Run,
            "{text}"
        );
    }
    // A MySQL implicit commit is never suppressible.
    let manual = ConnSafety {
        tx_mode: TxMode::Manual,
        ..mysql(EnvTag::Dev)
    };
    let ddl = run(
        "CREATE TABLE x (a int)",
        &manual,
        &SuppressSet::default(),
        7,
    );
    let mut s = SuppressSet::default();
    s.insert(7, ddl.fingerprint);
    assert!(matches!(
        run("CREATE TABLE x (a int)", &manual, &s, 7).decision,
        GateDecision::Confirm {
            suppressible: false,
            ..
        }
    ));
    // Production DML runs in a transaction and asks afterwards: suppression does not apply.
    let dml = run(
        "UPDATE t SET a = 1 WHERE id = 1",
        &conn_prod(),
        &SuppressSet::default(),
        7,
    );
    let mut s = SuppressSet::default();
    s.insert(7, dml.fingerprint);
    assert!(matches!(
        run("UPDATE t SET a = 1 WHERE id = 1", &conn_prod(), &s, 7).decision,
        GateDecision::RunInTxThenConfirm { .. }
    ));
}

#[test]
fn stale_report_is_denied() {
    let u = prepared_pg("SELECT :a");
    let rep = report_over("SELECT :a");
    assert!(
        matches!(gate(&[u], &[rep], &conn_dev(), &SuppressSet::default(), 1).items[0].decision, GateDecision::Deny(ref r) if r.fluent_id == "exec-gate-stale-classification")
    ); // final_text is "SELECT $1"
    // The report of the final text passes.
    let u = prepared_pg("SELECT :a");
    assert_eq!(&*u.final_text, "SELECT $1");
    let rep = report_over(&u.final_text);
    assert_eq!(one(u, rep, &conn_dev()).decision, GateDecision::Run);
    // Any byte changed, a missing report, or a count mismatch: stale, for every unit.
    let u = prepared_pg("SELECT 1");
    let item = one(u.clone(), report_over("SELECT 2"), &conn_prod());
    assert_eq!(deny_id(&item.decision), "exec-gate-stale-classification");
    assert_eq!(item.verdict, UnitVerdict::Unknown);
    let plan = gate(
        &[u.clone(), u.clone()],
        &[report_over("SELECT 1")],
        &conn_dev(),
        &SuppressSet::default(),
        1,
    );
    assert!(
        plan.items
            .iter()
            .all(|i| deny_id(&i.decision) == "exec-gate-stale-classification")
    );
    // The trailing `/*M!` case on a MySQL profile: the bytes classified are the bytes sent.
    let my = mysql(EnvTag::Dev);
    let text = "SELECT 1 /*M! , 2 */";
    let unit = prepared_on(DbmsKind::MySql, text, &[]);
    let trimmed = fake_classify("SELECT 1", DialectId::MySql);
    assert_eq!(
        deny_id(&one(unit.clone(), trimmed, &my).decision),
        "exec-gate-stale-classification"
    );
    let exact = fake_classify(text, DialectId::MySql);
    assert_eq!(one(unit, exact, &my).decision, GateDecision::Run);
}

fn prepared_pg(text: &str) -> PreparedUnit {
    prepared_on(DbmsKind::Postgres, text, &[("a", "1")])
}

fn report_over(text: &str) -> SafetyReport {
    fake_classify(text, DialectId::Postgres)
}

#[test]
fn an_empty_plan_allows_nothing() {
    let plan = gate(&[], &[], &conn_dev(), &SuppressSet::default(), 1);
    assert!(plan.items.is_empty());
    assert!(!plan.covers(0) && !plan.covers(1));
    assert!(!GatePlan::default().covers(1));
    let u = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    let plan = gate(
        &[u.clone(), u],
        &[report_over("SELECT 1"), report_over("SELECT 1")],
        &conn_dev(),
        &SuppressSet::default(),
        1,
    );
    assert!(plan.covers(2) && !plan.covers(3) && !plan.covers(1));
    assert_eq!(
        plan.items.iter().map(|i| i.unit).collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn cursor_needs_a_read_verdict() {
    assert!(!unit_suspendable("SELECT f_write()", report_write()));
    assert!(unit_suspendable(
        "SELECT 1",
        stmt(StmtKind::Select, EffectSet::READ)
    ));
    // Each piece of R-M3-23 alone refuses the cursor.
    let mut lying = stmt(StmtKind::Select, EffectSet::READ | EffectSet::WRITE_DATA);
    lying.class = CoarseClass::Read;
    assert!(!unit_suspendable("SELECT 1", lying));
    assert!(!unit_suspendable(
        "SELECT * FROM t FOR UPDATE",
        fake_stmt("SELECT * FROM t FOR UPDATE")
    ));
    assert!(!unit_suspendable("CALL p()", fake_stmt("CALL p()")));
    // A stale report is no `Read`.
    let p = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    assert!(!suspendable_for(&p, &report_over("SELECT 2")));
    // Without a read head nothing is suspendable, whatever the verdict.
    let mut p = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    p.unit.suspendable = false;
    assert!(!suspendable_for(&p, &report_over("SELECT 1")));
    // `apply_read_verdicts` ANDs the flag in place, and agrees with the gate's verdict.
    let mut units = vec![
        prepared_on(DbmsKind::Postgres, "SELECT 1", &[]),
        prepared_on(DbmsKind::Postgres, "SELECT f_write()", &[]),
    ];
    let reports = vec![report_over("SELECT 1"), report_over("SELECT f_write()")];
    let plan = gate(&units, &reports, &conn_dev(), &SuppressSet::default(), 1);
    apply_read_verdicts(&mut units, &reports);
    assert!(units[0].unit.suspendable && !units[1].unit.suspendable);
    for (u, i) in units.iter().zip(&plan.items) {
        assert_eq!(u.unit.suspendable, i.verdict == UnitVerdict::Read);
    }
    // A count mismatch makes nothing suspendable.
    apply_read_verdicts(&mut units, &reports[..1]);
    assert!(units.iter().all(|u| !u.unit.suspendable));
}

fn unit_suspendable(text: &str, s: StmtReport) -> bool {
    let p = prepared_on(DbmsKind::Postgres, text, &[]);
    let report = over(&p.final_text, DialectId::Postgres, vec![s]);
    suspendable_for(&p, &report)
}

/// `SELECT f_write()` as a classifier that knows the function writes reports it.
fn report_write() -> StmtReport {
    let mut s = stmt(StmtKind::Select, EffectSet::READ | EffectSet::WRITE_DATA);
    s.touched
        .calls
        .push(object("f_write", ObjectKind::Function));
    s
}

/// Routine maps to `Unknown` with unknown session effects, never to `Write`.
#[test]
fn routine_is_unknown_with_unknown_effects() {
    for c in [conn_dev(), conn_prod()] {
        let item = item_of("CALL p()", &c);
        assert_eq!(item.class, GateClass::Routine);
        assert_eq!(item.verdict, UnitVerdict::Unknown);
        assert!(item.effects.contains(
            SessionEffects::OPENS_WRITE
                | SessionEffects::SESSION_STATE
                | SessionEffects::WRITE_TEMP
                | SessionEffects::ADVISORY_LOCK
                | SessionEffects::PREPARE
        ));
        assert!(!item.effects.contains(SessionEffects::DDL));
    }
}

#[test]
fn effects_map_to_session_effects() {
    let e = |s: &str, c: ConnSafety| item_of(s, &c).effects;
    assert_eq!(e("SELECT 1", conn_dev()), SessionEffects::empty());
    assert_eq!(e("SET x = 1", conn_dev()), SessionEffects::SESSION_STATE);
    assert!(
        e("CREATE TEMP TABLE x (a int)", conn_dev()).contains(
            SessionEffects::WRITE_TEMP | SessionEffects::DDL | SessionEffects::OPENS_WRITE
        )
    );
    for s in [
        "CREATE TABLE x (a int)",
        "ALTER TABLE t ADD b int",
        "DROP TABLE t",
        "TRUNCATE t",
    ] {
        assert!(e(s, conn_dev()).contains(SessionEffects::DDL), "{s}");
    }
    assert_eq!(
        e("INSERT INTO t VALUES (1)", conn_dev()),
        SessionEffects::OPENS_WRITE
    );
    assert!(e("LOCK TABLES t WRITE", mysql(EnvTag::Dev)).contains(SessionEffects::LOCK_TABLES));
    assert!(!e("LOCK TABLE t", conn_dev()).contains(SessionEffects::LOCK_TABLES));
    assert!(e("GRANT SELECT ON t TO bob", mysql(EnvTag::Dev)).contains(SessionEffects::DDL));
}

// ---------------------------------------------------------------------------------------------
// The verdict table (R-M3-55) for every combination, against an independent oracle

/// One representative statement per gate class.
fn base(class: GateClass) -> StmtReport {
    match class {
        GateClass::Read => stmt(StmtKind::Select, EffectSet::READ),
        GateClass::Session => fake_stmt("SET hv_custom = 1"),
        GateClass::Tx => stmt(StmtKind::Begin, EffectSet::TXN_CONTROL),
        GateClass::Lock => fake_stmt("SELECT * FROM t FOR UPDATE"),
        GateClass::Routine => fake_stmt("CALL p()"),
        GateClass::Dml => stmt(StmtKind::Update, EffectSet::WRITE_DATA),
        GateClass::DmlUnsafe => report(
            StmtKind::Delete,
            EffectSet::WRITE_DATA,
            &[DestructiveKind::NoWhere],
            "orders",
        ),
        GateClass::DdlOther => stmt(StmtKind::CreateTable, EffectSet::DDL),
        GateClass::DdlDestructive => fake_stmt("DROP TABLE orders"),
        GateClass::Admin => stmt(StmtKind::Grant, EffectSet::PRIVILEGE),
        GateClass::Unknown => {
            let mut s = stmt(StmtKind::Select, EffectSet::READ);
            s.reasons.push(reason(ReasonCode::FnUnknown, Some("f")));
            s
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Dev,
    Staging,
    Production,
    ReadOnly,
    ProductionReadOnly,
}

#[derive(Debug, PartialEq, Eq)]
struct Want {
    shape: &'static str,
    typed: Option<&'static str>,
    suppressible: Option<bool>,
    irreversible: Option<bool>,
    verdict: UnitVerdict,
}

/// R-M3-55 written out on its own (it reads none of the gate's code): the class column, the
/// connection rows, the AND-check with `decide(Admin)` (the `admin` column of
/// `data/profiles.toml`), type-to-confirm from the destructive kinds, the MySQL implicit
/// commit, suppressibility and the verdict.
fn oracle(
    class: GateClass,
    extra: Option<(DestructiveKind, ObjectKind)>,
    level: Level,
    engine: DbmsKind,
    tx_open: bool,
    confirm_unsafe_where: bool,
) -> Want {
    use GateClass as G;
    let extra_class = extra.map(|(k, _)| {
        if k.is_unsafe_dml() {
            G::DmlUnsafe
        } else {
            G::DdlDestructive
        }
    });
    let eff = extra_class.map_or(class, |e| class.max(e));
    let unsafe_dml = class == G::DmlUnsafe || extra.is_some_and(|(k, _)| k.is_unsafe_dml());
    let mysql = matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb);
    let ddl = matches!(class, G::DdlOther | G::DdlDestructive)
        || extra.is_some_and(|(k, _)| !k.is_unsafe_dml())
        || (mysql && class == G::Admin);
    let deny = Want {
        shape: "deny",
        typed: None,
        suppressible: None,
        irreversible: None,
        verdict: UnitVerdict::Unknown,
    };
    let verdict_of = |c: G| match c {
        G::Read => UnitVerdict::Read,
        G::Routine | G::Unknown => UnitVerdict::Unknown,
        _ => UnitVerdict::Write,
    };
    if matches!(level, Level::ReadOnly | Level::ProductionReadOnly) {
        // Only `Read` runs (the `Session` representative is not on the harmless list).
        return if eff == G::Read {
            Want {
                shape: "run",
                typed: None,
                suppressible: None,
                irreversible: None,
                verdict: UnitVerdict::Read,
            }
        } else {
            deny
        };
    }
    let production = level == Level::Production;
    // decide(Admin): the base statement's group and the destructive pattern's group, under the
    // `admin` column (session/tx, routine, admin, unknown confirm; the rest allow).
    let admin_confirms = matches!(
        class,
        G::Session | G::Tx | G::Routine | G::Admin | G::Unknown
    );
    let admin_row_allows = matches!(
        eff,
        G::Read | G::Lock | G::Dml | G::DmlUnsafe | G::DdlOther | G::DdlDestructive
    );
    let eff = if production && admin_confirms && admin_row_allows {
        G::Unknown
    } else {
        eff
    };
    let typed = production
        && (unsafe_dml
            || extra.is_some_and(|(k, o)| {
                matches!(k, DestructiveKind::Drop { .. })
                    && matches!(o, ObjectKind::Schema | ObjectKind::Database)
            }));
    let implicit = mysql && ddl && tx_open;
    let mut shape = if production {
        match eff {
            G::Read | G::Session => "run",
            G::Lock => "warn",
            G::Dml => "tx-confirm",
            _ => "confirm",
        }
    } else if unsafe_dml && confirm_unsafe_where {
        "confirm"
    } else {
        "run"
    };
    if typed || implicit {
        shape = "confirm";
    }
    let verdict = if shape == "tx-confirm" {
        UnitVerdict::TxThenConfirm
    } else if implicit {
        UnitVerdict::ImplicitCommit
    } else {
        verdict_of(eff)
    };
    Want {
        shape: if typed { "confirm+type" } else { shape },
        typed: typed.then_some("orders"),
        suppressible: (shape == "confirm").then_some(!typed && !implicit),
        irreversible: (shape == "confirm").then_some(mysql && ddl),
        verdict,
    }
}

fn observed(item: &GateItem) -> Want {
    let (typed, suppressible, irreversible) = match &item.decision {
        GateDecision::Confirm {
            type_to_confirm,
            suppressible,
            irreversible,
            ..
        } => (
            type_to_confirm.as_deref().map(|t| {
                assert_eq!(t, "orders");
                "orders"
            }),
            Some(*suppressible),
            Some(*irreversible),
        ),
        _ => (None, None, None),
    };
    Want {
        shape: shape(&item.decision),
        typed,
        suppressible,
        irreversible,
        verdict: item.verdict,
    }
}

#[test]
fn verdict_table_matches_r_m3_55_for_every_combination() {
    let mut extras: Vec<Option<(DestructiveKind, ObjectKind)>> = vec![None];
    extras.extend(
        DestructiveKind::ALL
            .iter()
            .map(|k| Some((*k, ObjectKind::Table))),
    );
    extras.push(Some((
        DestructiveKind::Drop { cascade: false },
        ObjectKind::Schema,
    )));
    extras.push(Some((
        DestructiveKind::Drop { cascade: true },
        ObjectKind::Database,
    )));
    let levels = [
        Level::Dev,
        Level::Staging,
        Level::Production,
        Level::ReadOnly,
        Level::ProductionReadOnly,
    ];
    let txs = [
        (TxStatus::Idle, TxOwner::None, TxMode::Auto),
        (TxStatus::Idle, TxOwner::None, TxMode::Manual),
        (TxStatus::Idle, TxOwner::None, TxMode::Smart),
        (TxStatus::InTx, TxOwner::User, TxMode::Manual),
        (TxStatus::InTx, TxOwner::User, TxMode::Auto),
        (TxStatus::Failed, TxOwner::User, TxMode::Manual),
    ];
    let mut cases = 0u32;
    for class in GateClass::ALL {
        assert_eq!(
            gate_class(&base(class)),
            class,
            "the representative of {class:?}"
        );
        for extra in &extras {
            let mut s = base(class);
            if let Some((kind, object_kind)) = extra {
                s.destructive.push(DestructiveReason {
                    kind: *kind,
                    range: TextRange::default(),
                    target: Some(object("orders", *object_kind)),
                });
            }
            for level in levels {
                for engine in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
                    for (tx_status, tx_owner, tx_mode) in txs {
                        for confirm_unsafe_where in [true, false] {
                            let env = match level {
                                Level::Dev | Level::ReadOnly => EnvTag::Dev,
                                Level::Staging => EnvTag::Staging,
                                Level::Production | Level::ProductionReadOnly => EnvTag::Production,
                            };
                            let c = ConnSafety {
                                engine,
                                env: Some(env),
                                read_only: matches!(
                                    level,
                                    Level::ReadOnly | Level::ProductionReadOnly
                                ),
                                tx_status,
                                tx_owner,
                                tx_mode,
                                confirm_unsafe_where,
                            };
                            let tx_open = tx_status != TxStatus::Idle || tx_mode == TxMode::Manual;
                            let item = gated("DELETE FROM orders", vec![s.clone()], &c);
                            let want =
                                oracle(class, *extra, level, engine, tx_open, confirm_unsafe_where);
                            assert_eq!(
                                observed(&item),
                                want,
                                "{class:?} + {extra:?} on {level:?} {engine:?} \
                                 {tx_status:?}/{tx_mode:?} unsafe-where={confirm_unsafe_where}: \
                                 {:?}",
                                item.decision
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 11 * 14 * 5 * 3 * 6 * 2);
}

// ---------------------------------------------------------------------------------------------
// The end-to-end gate over the real classifier (P-M3b-16): Task 28 turns it on.

#[test]
#[ignore = "Task 28 turns it on once Task 14's classify has merged (P-M3b-16)"]
fn gate_end_to_end_over_real_classify() {
    let ctx = sql_safety::ClassifyCtx::new(
        sql_dialect::DialectCtx::new(DialectId::Postgres, None),
        sql_dialect::ClientFlavor::None,
        sql_safety::Expect::SingleStatement,
    );
    let rows = [
        ("SELECT 1", "run"),
        ("UPDATE t SET a = 1 WHERE id = 3", "tx-confirm"),
        ("DELETE FROM müşteri", "confirm+type"),
        ("DROP TABLE t", "confirm"),
        ("CALL p()", "confirm"),
        ("SELECT * FROM t FOR UPDATE", "warn"),
    ];
    for (sql, want) in rows {
        let unit = prepared_on(DbmsKind::Postgres, sql, &[]);
        let report = sql_safety::classify(&unit.final_text, &ctx);
        let item = one(unit, report, &conn_prod());
        assert_eq!(shape(&item.decision), want, "{sql}");
    }
}
