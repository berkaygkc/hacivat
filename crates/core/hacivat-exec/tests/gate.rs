//! M3b Task 15: the exec gate (HV-EXEC-04, R-M3-55, R-M3-84; R-M3-23 ANDed). Reports are built
//! by hand over the exact final text (P-M3b-16, `common::gate_fx`); the end-to-end test over
//! the real classifier was `#[ignore]`d until Task 28 turned it on (m3b-t28, named exception).

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
    // `EXPLAIN ANALYZE <x>`: the wrap off production; asked first on production
    // (## m3b-t26-prod-unlock fix round 1, B-M2).
    for (c, want) in [
        (conn_dev(), "explain-wrap"),
        (conn_staging(), "explain-wrap"),
        (conn_prod(), "confirm"),
    ] {
        assert_eq!(
            shape(&decision("EXPLAIN ANALYZE DELETE FROM t WHERE id = 1", c)),
            want
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
/// Fix round 1, I3: the statement's own words say `DROP DATABASE`/`DROP SCHEMA`, so a backend
/// that names the target's kind wrongly (or names no destructive pattern) cannot drop the
/// typed name on production.
#[test]
fn drop_database_asks_for_the_name_from_the_words() {
    for engine in [DbmsKind::MySql, DbmsKind::Postgres] {
        let prod = conn(engine, Some(EnvTag::Production));
        // A target the backend called a table.
        let mut s = stmt(StmtKind::Drop, EffectSet::DDL);
        s.destructive.push(DestructiveReason {
            kind: DestructiveKind::Drop { cascade: false },
            range: TextRange::default(),
            target: Some(object("shop", ObjectKind::Table)),
        });
        for text in ["DROP DATABASE shop", "drop schema if exists shop"] {
            let item = gated(text, vec![s.clone()], &prod);
            assert!(
                matches!(&item.decision, GateDecision::Confirm { type_to_confirm: Some(t), suppressible: false, .. } if &**t == "shop"),
                "{engine:?} {text}: {:?}",
                item.decision
            );
        }
        // No destructive pattern at all: the name comes from the words, as written.
        let bare = stmt(StmtKind::Drop, EffectSet::DDL);
        let item = gated("DROP DATABASE `Shop`", vec![bare.clone()], &prod);
        if engine == DbmsKind::MySql {
            assert!(
                matches!(&item.decision, GateDecision::Confirm { type_to_confirm: Some(t), .. } if &**t == "Shop"),
                "{:?}",
                item.decision
            );
        }
        let item = gated("DROP SCHEMA Sales", vec![bare.clone()], &prod);
        assert!(
            matches!(&item.decision, GateDecision::Confirm { type_to_confirm: Some(t), .. } if &**t == "Sales"),
            "{engine:?}: {:?}",
            item.decision
        );
        // Words that name nothing: nothing to type, denied.
        let item = gated("DROP DATABASE", vec![bare], &prod);
        assert_eq!(deny_id(&item.decision), "exec-unsafe-no-target");
        // Off production nobody types.
        let dev = conn(engine, Some(EnvTag::Dev));
        assert_eq!(
            gated("DROP DATABASE shop", vec![s.clone()], &dev).decision,
            GateDecision::Run
        );
    }
}

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
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): MySQL asks before it sends,
    // in a user transaction or not (a non-transactional table keeps the write; was
    // `RunInTxThenConfirm` inside a user transaction).
    for (status, owner) in [
        (TxStatus::InTx, TxOwner::User),
        (TxStatus::Idle, TxOwner::None),
    ] {
        let my = ConnSafety {
            tx_status: status,
            tx_owner: owner,
            ..mysql(EnvTag::Production)
        };
        let item = item_of("UPDATE t SET a = 1 WHERE id = 3", &my);
        assert!(
            matches!(
                item.decision,
                GateDecision::Confirm {
                    type_to_confirm: None,
                    ..
                }
            ),
            "{:?}",
            item.decision
        );
        assert_eq!(item.verdict, UnitVerdict::Write);
    }
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
            assert!(item.effects.contains(SessionEffects::COMMITS_FIRST), "{s}");
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
    // cannot be rolled back (## m3b-t26-prod-unlock, fix round 2: never suppressible on
    // production).
    let prod = item_of("CREATE TABLE x (a int)", &mysql(EnvTag::Production));
    assert!(matches!(
        prod.decision,
        GateDecision::Confirm {
            irreversible: true,
            suppressible: false,
            ..
        }
    ));
    assert_eq!(prod.verdict, UnitVerdict::Write);
    // PostgreSQL DDL in a transaction: no implicit commit, nothing irreversible (and, since
    // T26 fix round 2, never suppressible on production).
    let pg = item_of("CREATE TABLE x (a int)", &conn_prod_user_tx());
    assert!(matches!(
        pg.decision,
        GateDecision::Confirm {
            irreversible: false,
            suppressible: false,
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
    // A pre-decided report (other than the stub's) is denied on production; off production a
    // quiet one runs and one that may write asks (fix round 1, I1; the full table is
    // `pre_decided_reports_are_denied_on_production_and_ask_before_writes_elsewhere`).
    let unit = prepared_on(DbmsKind::Postgres, "SELECT 1", &[]);
    let mut r = base;
    r.pre_decision = Some(vec![reason(ReasonCode::TooComplex, None)]);
    r.items.clear();
    assert_eq!(
        deny_id(&one(unit.clone(), r.clone(), &conn_prod()).decision),
        "safety-reason-too-complex"
    );
    let dev = one(unit, r.clone(), &conn_dev());
    assert_eq!(dev.decision, GateDecision::Run);
    assert_eq!(dev.verdict, UnitVerdict::Unknown);
    let unit = prepared_on(DbmsKind::Postgres, "DELETE FROM orders", &[]);
    let mut r = report_over("DELETE FROM orders");
    r.items.clear();
    r.pre_decision = Some(vec![reason(ReasonCode::TooComplex, None)]);
    assert!(matches!(
        one(unit, r, &conn_dev()).decision,
        GateDecision::Confirm {
            suppressible: false,
            ..
        }
    ));
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
/// `Run`, `Warn` or `Confirm`; on production a routine, administrative or unknown unit always
/// asks or is denied (EXPLAIN ANALYZE included, fix round 1 I2); off production an unsafe-DML
/// pattern never runs silently while the guard is on; a report that analysed nothing (I1) is
/// denied on production and read-only, and asks off production when its words may write.
#[test]
fn no_path_turns_deny_into_run() {
    // ## m3b-gate-explain: `PROPTEST_CASES` raises the count for a soak run.
    let cfg = ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6_000),
        ..ProptestConfig::default()
    };
    let texts = [
        "SELECT 1",
        "DELETE FROM t",
        "UPDATE t SET a = 1",
        "/*!40101 SET NAMES utf8 */",
        "CALL p()",
        "BEGIN",
    ];
    proptest!(cfg, |(
        items in prop::collection::vec(arb_stmt(), 1..4),
        ci in 0usize..9,
        ti in 0usize..6,
        predecided in prop::option::of(prop::sample::select(ReasonCode::ALL.to_vec())),
    )| {
        let conns = [
            conn_ro(),
            ConnSafety { read_only: true, ..conn_prod() },
            conn_prod_auto(),
            conn_prod_user_tx(),
            ConnSafety { read_only: true, ..mysql(EnvTag::Staging) },
            ConnSafety { tx_mode: TxMode::Manual, ..mysql(EnvTag::Production) },
            conn_dev(),
            ConnSafety { tx_mode: TxMode::Manual, ..mysql(EnvTag::Dev) },
            conn(DbmsKind::Sqlite, None),
        ];
        let c = conns[ci];
        let text = texts[ti];
        let unit = prepared_on(c.engine, text, &[]);
        let mut report = over(text, dialect_of(c.engine), items);
        if let Some(code) = predecided.filter(|c| *c != ReasonCode::NotImplemented) {
            report.items.clear();
            report.pre_decision = Some(vec![reason(code, None)]);
        }
        let item = one(unit, report.clone(), &c);
        let production = c.env == Some(EnvTag::Production);
        let denied = matches!(item.decision, GateDecision::Deny(_));
        let confirms = matches!(item.decision, GateDecision::Confirm { .. });
        if report.items.is_empty() {
            if c.read_only || production {
                prop_assert!(denied, "{:?}", item);
            } else {
                let writes = matches!(ti, 1 | 2 | 4);
                let commits = ti == 5
                    && matches!(c.engine, DbmsKind::MySql)
                    && c.tx_mode == TxMode::Manual;
                if writes || commits {
                    let unsuppressible = matches!(
                        item.decision,
                        GateDecision::Confirm { suppressible: false, .. }
                    );
                    prop_assert!(unsuppressible, "{:?}", item);
                } else {
                    prop_assert_eq!(&item.decision, &GateDecision::Run);
                }
            }
            return Ok(());
        }
        let max = report.items.iter().map(gate_class).max().unwrap();
        if c.read_only {
            let rope = decide(&report, Profile::ReadOnlyPlusExplain, Actor::Human);
            if !rope.allows() {
                prop_assert!(denied, "{:?}", item);
            }
            // The stricter `ReadOnly` profile differs only for EXPLAIN ANALYZE of a read,
            // which the human read-only profile allows (R-M3-56).
            if !decide(&report, Profile::ReadOnly, Actor::Human).allows() && !denied {
                prop_assert!(rope.allows());
                let explains = report.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
                    || report.items.iter().any(|s| s.reasons.iter().any(|r| r.code == ReasonCode::ExplainAnalyze))
                    || report.items.iter().any(|s| matches!(s.kind, StmtKind::Explain { analyze: true }));
                prop_assert!(explains);
            }
            if item.decision == GateDecision::Run {
                prop_assert!(max <= GateClass::Session);
            }
            prop_assert!(denied || item.decision == GateDecision::Run, "a read-only connection never asks: {:?}", item);
        } else if production {
            let admin = decide(&report, Profile::Admin, Actor::Human);
            if matches!(admin.verdict, sql_safety::Verdict::Deny(_)) {
                prop_assert!(denied, "{:?}", item);
            }
            if item.decision == GateDecision::Run {
                // ## m3b-gate-explain: above `Session`, only the B-M6 read runs unasked.
                prop_assert!(max <= GateClass::Session || b_m6_read(&report), "{:?}", item);
                prop_assert!(!matches!(admin.verdict, sql_safety::Verdict::Deny(_)));
            }
            if matches!(max, GateClass::Routine | GateClass::Admin | GateClass::Unknown) {
                prop_assert!(confirms || denied, "production never runs these unasked: {:?}", item);
            }
            // "Don't ask again" never covers these on production (controller ruling).
            if let GateDecision::Confirm { suppressible: true, .. } = item.decision {
                prop_assert!(!matches!(
                    item.class,
                    GateClass::DdlDestructive | GateClass::Admin | GateClass::Routine | GateClass::Unknown
                ));
                prop_assert!(report.items.len() == 1 || !report.items.iter().any(sql_safety::opens_write));
            }
        } else {
            // Off production the unsafe-WHERE guard (on in these connections) is never skipped.
            let unsafe_dml = report
                .items
                .iter()
                .flat_map(|s| &s.destructive)
                .any(|d| d.kind.is_unsafe_dml());
            if unsafe_dml {
                prop_assert_ne!(&item.decision, &GateDecision::Run);
            }
        }
        // A `Read` verdict always reads only (R-M3-23), or is the B-M6 read of a plan.
        if item.verdict == UnitVerdict::Read {
            prop_assert!(reads_only(&report) || b_m6_read(&report));
        }
    });
}

// ## m3b-gate-explain
/// The B-M6 ruling as the property states it: every statement a plain `EXPLAIN` of a data
/// change (gate class exactly `Dml`) with no evidence of code or of `ANALYZE` (no
/// `ROUTINE_CALL` bit on a statement or the request, no touched call, no destructive pattern,
/// no `EXPLAIN ANALYZE` kind, flag or reason), and `decide(.., Admin, Human)` allows it. The
/// class is the unit's, as the gate reads it: the statements' and the request-level floor's
/// (its class, effects and flags; review M-3), exactly `Dml`.
fn b_m6_read(report: &SafetyReport) -> bool {
    let analyzed = |s: &StmtReport| {
        s.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
            || s.reasons
                .iter()
                .any(|r| r.code == ReasonCode::ExplainAnalyze)
    };
    // The request-level floor, read as one statement of kind `Other` (as the gate does).
    let mut request = stmt(StmtKind::Other, report.request_effects);
    request.flags = report.flags;
    request.class = report.request_class;
    let class = report
        .items
        .iter()
        .map(gate_class)
        .max()
        .map(|c| c.max(gate_class(&request)));
    report.pre_decision.is_none()
        && !report.items.is_empty()
        && class == Some(GateClass::Dml)
        && !report.request_effects.contains(EffectSet::ROUTINE_CALL)
        && !report.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
        && report.items.iter().all(|s| {
            s.kind == (StmtKind::Explain { analyze: false })
                && !analyzed(s)
                && !s.effects.contains(EffectSet::ROUTINE_CALL)
                && s.touched.calls.is_empty()
                && s.destructive.is_empty()
        })
        && decide(report, Profile::Admin, Actor::Human).allows()
}

/// Review M-2: the production connections the routine pins run on.
fn production_conns() -> [ConnSafety; 6] {
    [
        conn_prod_auto(),
        conn_prod_manual_idle(),
        conn_prod_user_tx(),
        mysql(EnvTag::Production),
        ConnSafety {
            tx_mode: TxMode::Manual,
            ..mysql(EnvTag::Production)
        },
        conn(DbmsKind::Sqlite, Some(EnvTag::Production)),
    ]
}

/// Review M-2: routine evidence (the `ROUTINE_CALL` bit, a `RoutineCall` or `ProceduralCode`
/// reason) under a data change, a read or a plan asks before anything is sent on production:
/// `Confirm` (or `Deny`), never `Run`, `Warn` or the run-then-ask `RunInTxThenConfirm`. The
/// routine class sits below `Dml` in the order (unchanged); what raises the unit is the policy
/// (`profiles.toml` `[routine] admin = "confirm"`), which step 4 turns into `Unknown`. This test
/// pins that dependency.
#[test]
fn dml_with_routine_evidence_asks_before_sending_on_production() {
    let none = SuppressSet::default();
    // The policy dependency itself.
    let bit = over(
        "UPDATE t SET a = f(1) WHERE id = 1",
        DialectId::Postgres,
        vec![stmt(
            StmtKind::Update,
            EffectSet::READ | EffectSet::WRITE_DATA | EffectSet::ROUTINE_CALL,
        )],
    );
    assert!(
        matches!(
            decide(&bit, Profile::Admin, Actor::Human).verdict,
            sql_safety::Verdict::Confirm(_)
        ),
        "`[routine] admin` must stay `confirm`"
    );
    let evidence: [fn(&mut StmtReport); 4] = [
        |s| s.effects |= EffectSet::ROUTINE_CALL,
        |s| s.reasons.push(reason(ReasonCode::RoutineCall, Some("f"))),
        |s| {
            s.reasons
                .push(reason(ReasonCode::ProceduralCode, Some("f")))
        },
        |s| {
            s.effects |= EffectSet::ROUTINE_CALL;
            s.touched.calls.push(object("f", ObjectKind::Function));
        },
    ];
    for (kind, effects) in [
        (StmtKind::Update, EffectSet::READ | EffectSet::WRITE_DATA),
        (StmtKind::Insert, EffectSet::WRITE_DATA),
        (StmtKind::Delete, EffectSet::READ | EffectSet::WRITE_DATA),
        (StmtKind::Merge, EffectSet::READ | EffectSet::WRITE_DATA),
        (StmtKind::Select, EffectSet::READ),
        (StmtKind::Select, EffectSet::READ | EffectSet::SEQUENCE),
        (
            StmtKind::Explain { analyze: false },
            EffectSet::READ | EffectSet::WRITE_DATA,
        ),
    ] {
        for (i, add) in evidence.iter().enumerate() {
            let mut s = stmt(kind, effects);
            add(&mut s);
            for c in production_conns() {
                let text = "UPDATE t SET a = f(1) WHERE id = 1";
                let unit = prepared_on(c.engine, text, &[]);
                let report = over(&unit.final_text, dialect_of(c.engine), vec![s.clone()]);
                let item = gate(&[unit], &[report], &c, &none, 1).items.remove(0);
                assert!(
                    matches!(
                        item.decision,
                        GateDecision::Confirm { .. } | GateDecision::Deny(_)
                    ),
                    "{kind:?} {effects:?} evidence {i} on {:?} {:?} {:?}: {:?}",
                    c.engine,
                    c.tx_mode,
                    c.tx_owner,
                    item.decision
                );
                assert_ne!(item.verdict, UnitVerdict::Read);
            }
        }
    }
}

/// Review M-2, as a property: any data-change, read or plan kind, any effects at or below
/// `Dml`, touched calls or not, with routine evidence, on any production connection: asked
/// before anything is sent (or denied).
#[test]
fn routine_evidence_never_runs_unasked_on_production() {
    let bits = [
        EffectSet::READ,
        EffectSet::WRITE_DATA,
        EffectSet::WRITE_TEMP,
        EffectSet::SEQUENCE,
        EffectSet::XID_CONSUME,
        EffectSet::LOCK_ROW,
        EffectSet::LOCK_TABLE,
        EffectSet::ADVISORY_LOCK,
        EffectSet::SESSION_STATE,
        EffectSet::NOTIFY,
    ];
    let kinds = vec![
        StmtKind::Update,
        StmtKind::Insert,
        StmtKind::Delete,
        StmtKind::Merge,
        StmtKind::Select,
        StmtKind::Explain { analyze: false },
        StmtKind::Other,
    ];
    let cfg = ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4_000),
        ..ProptestConfig::default()
    };
    let shape = (
        prop::sample::select(kinds),
        prop::collection::vec(any::<bool>(), bits.len()),
        any::<bool>(),
        0u8..3,
    );
    proptest!(cfg, |(
        shapes in prop::collection::vec(shape, 1..3),
        routine_at in 0usize..2,
        ci in 0usize..6,
    )| {
        let c = production_conns()[ci];
        let n = shapes.len();
        let items: Vec<StmtReport> = shapes
            .iter()
            .enumerate()
            .map(|(i, (kind, on, call, evidence))| {
                let mut effects = EffectSet::empty();
                for (b, on) in bits.iter().zip(on) {
                    if *on {
                        effects |= *b;
                    }
                }
                let mut s = stmt(*kind, effects);
                if *call {
                    s.touched.calls.push(object("f", ObjectKind::Function));
                }
                // At least one statement carries routine evidence.
                if i == routine_at % n {
                    match evidence {
                        0 => s.effects |= EffectSet::ROUTINE_CALL,
                        1 => s.reasons.push(reason(ReasonCode::RoutineCall, Some("f"))),
                        _ => s.reasons.push(reason(ReasonCode::ProceduralCode, Some("f"))),
                    }
                }
                s
            })
            .collect();
        let text = "UPDATE t SET a = f(1) WHERE id = 1";
        let unit = prepared_on(c.engine, text, &[]);
        let report = over(&unit.final_text, dialect_of(c.engine), items);
        let item = one(unit, report, &c);
        prop_assert!(
            matches!(item.decision, GateDecision::Confirm { .. } | GateDecision::Deny(_)),
            "{:?}",
            item
        );
        prop_assert_ne!(item.verdict, UnitVerdict::Read);
    });
}

/// Review M-3: a plain `EXPLAIN` of a data change whose request-level fields say more than its
/// statements (a class, effects or flags above `Dml`) is not the B-M6 read.
#[test]
fn plain_explain_with_a_request_floor_above_dml_is_not_the_read() {
    let base = || {
        over(
            "EXPLAIN UPDATE t SET a = 1 WHERE id = 1",
            DialectId::Postgres,
            vec![stmt(
                StmtKind::Explain { analyze: false },
                EffectSet::READ | EffectSet::WRITE_DATA,
            )],
        )
    };
    let unit = prepared_on(
        DbmsKind::Postgres,
        "EXPLAIN UPDATE t SET a = 1 WHERE id = 1",
        &[],
    );
    assert!(b_m6_read(&base()));
    assert_eq!(
        one(unit.clone(), base(), &conn_prod()).verdict,
        UnitVerdict::Read
    );
    let raised: [fn(&mut SafetyReport); 4] = [
        |r| r.request_class = CoarseClass::Ddl,
        |r| r.request_class = CoarseClass::Admin,
        |r| r.request_effects |= EffectSet::DDL,
        |r| r.flags |= RiskFlags::DYNAMIC_SQL,
    ];
    for (i, raise) in raised.iter().enumerate() {
        let mut r = base();
        raise(&mut r);
        assert!(!b_m6_read(&r), "raise {i}");
        for c in [conn_prod(), conn_prod_user_tx(), mysql(EnvTag::Production)] {
            let unit = prepared_on(c.engine, "EXPLAIN UPDATE t SET a = 1 WHERE id = 1", &[]);
            let item = one(unit, r.clone(), &c);
            assert!(
                matches!(
                    item.decision,
                    GateDecision::Confirm { .. } | GateDecision::Deny(_)
                ),
                "raise {i} on {:?}: {:?}",
                c.engine,
                item.decision
            );
            assert_ne!(item.verdict, UnitVerdict::Read, "raise {i}");
        }
    }
}

/// Plain `EXPLAIN` shapes × every read-write connection: the B-M6 read runs only a plan of
/// code-free statements; a routine bit, a touched call, a destructive pattern, `ANALYZE`
/// evidence or a session-group effect (the policy confirms it under `Admin`) keeps the class,
/// so production asks (or denies) and the verdict is never `Read`.
#[test]
fn plain_explain_runs_as_a_read_only_when_code_free() {
    let bits = [
        EffectSet::READ,
        EffectSet::WRITE_DATA,
        EffectSet::WRITE_TEMP,
        EffectSet::SEQUENCE,
        EffectSet::XID_CONSUME,
        EffectSet::LOCK_ROW,
        EffectSet::LOCK_TABLE,
        EffectSet::ADVISORY_LOCK,
        EffectSet::SESSION_STATE,
        EffectSet::TXN_CONTROL,
        EffectSet::PREPARE,
        EffectSet::NOTIFY,
        EffectSet::ROUTINE_CALL,
    ];
    let code_free = EffectSet::READ
        | EffectSet::WRITE_DATA
        | EffectSet::WRITE_TEMP
        | EffectSet::SEQUENCE
        | EffectSet::XID_CONSUME
        | EffectSet::LOCK_ROW
        | EffectSet::LOCK_TABLE
        | EffectSet::ADVISORY_LOCK;
    let cfg = ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4_000),
        ..ProptestConfig::default()
    };
    let shape = (
        prop::collection::vec(any::<bool>(), bits.len()),
        any::<bool>(),
        any::<bool>(),
        0u8..3,
        prop::option::of(prop::sample::select(vec![
            ReasonCode::RoutineCall,
            ReasonCode::ProceduralCode,
            ReasonCode::FnUnknown,
            ReasonCode::SessionState,
        ])),
    );
    proptest!(cfg, |(
        shapes in prop::collection::vec(shape, 1..3),
        ci in 0usize..6,
    )| {
        let conns = [
            conn_prod_auto(),
            conn_prod_manual_idle(),
            conn_prod_user_tx(),
            mysql(EnvTag::Production),
            conn_dev(),
            mysql(EnvTag::Dev),
        ];
        let c = conns[ci];
        let items: Vec<StmtReport> = shapes
            .iter()
            .map(|(on, call, destroy, analyze, code)| {
                let mut effects = EffectSet::empty();
                for (b, on) in bits.iter().zip(on) {
                    if *on {
                        effects |= *b;
                    }
                }
                let mut s = stmt(StmtKind::Explain { analyze: false }, effects);
                if *call {
                    s.touched.calls.push(object("f", ObjectKind::Function));
                }
                if *destroy {
                    s.destructive.push(DestructiveReason {
                        kind: DestructiveKind::NoWhere,
                        range: TextRange::default(),
                        target: Some(object("t", ObjectKind::Table)),
                    });
                }
                match analyze {
                    1 => s.flags |= RiskFlags::EXPLAIN_ANALYZE,
                    2 => s.reasons.push(reason(ReasonCode::ExplainAnalyze, None)),
                    _ => {}
                }
                if let Some(code) = code {
                    s.reasons.push(reason(*code, Some("f")));
                }
                s
            })
            .collect();
        let text = "EXPLAIN UPDATE t SET a = f(1)";
        let unit = prepared_on(c.engine, text, &[]);
        let report = over(&unit.final_text, dialect_of(c.engine), items.clone());
        let item = one(unit, report.clone(), &c);
        let as_read = item.verdict == UnitVerdict::Read;
        // A plan of a pure read is a read by R-M3-23; anything else read is the B-M6 read.
        if as_read && !reads_only(&report) {
            prop_assert!(b_m6_read(&report), "{:?}", item);
            for s in &items {
                prop_assert!(code_free.contains(s.effects), "{s:?}");
                prop_assert!(s.touched.calls.is_empty() && s.destructive.is_empty());
                prop_assert!(s.flags.is_empty() && s.reasons.is_empty(), "{s:?}");
            }
        }
        let calls_code = items.iter().any(|s| {
            s.effects.contains(EffectSet::ROUTINE_CALL)
                || !s.touched.calls.is_empty()
                || s.reasons.iter().any(|r| {
                    matches!(r.code, ReasonCode::RoutineCall | ReasonCode::ProceduralCode)
                })
        });
        let routine = items.iter().any(|s| {
            s.effects.contains(EffectSet::ROUTINE_CALL)
                || s.reasons.iter().any(|r| {
                    matches!(r.code, ReasonCode::RoutineCall | ReasonCode::ProceduralCode)
                })
        });
        if c.env == Some(EnvTag::Production) {
            // A plan of a data change that calls code never runs unasked on production (the
            // B-M6 domain; at or below `Lock` the table's own row decides); one of a routine
            // asks first.
            let max = items.iter().map(gate_class).max().unwrap_or(GateClass::Unknown);
            if calls_code && max >= GateClass::Dml {
                prop_assert!(
                    !matches!(item.decision, GateDecision::Run | GateDecision::Warn(_)),
                    "a plan of code asks on production: {:?}",
                    item
                );
            }
            if routine {
                prop_assert!(
                    matches!(item.decision, GateDecision::Confirm { .. } | GateDecision::Deny(_)),
                    "a plan of a routine asks first on production: {:?}",
                    item
                );
            }
        }
        if routine {
            prop_assert!(!as_read, "{:?}", item);
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
                // Mostly named bits and flags, sometimes an unnamed one too.
                let (extra_bit, extra_flag) = if unnamed { (1 << 20, 1 << 9) } else { (0, 0) };
                let effects = bits & (EffectSet::all().bits() | extra_bit);
                let mut s = stmt(kind, EffectSet::from_bits_retain(effects));
                s.flags =
                    RiskFlags::from_bits_retain(flags & (RiskFlags::all().bits() | extra_flag));
                s.class = class;
                s.reasons = codes
                    .into_iter()
                    .filter(|c| *c != ReasonCode::NotImplemented)
                    .map(|c| reason(c, Some("f")))
                    .collect();
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

/// Fix round 1, I1: a report that analysed nothing (every pre-decided reason) is denied on
/// production and read-only connections; off production it asks once (never suppressible)
/// when its words may write, else it runs; the classifier stub is denied everywhere.
#[test]
fn pre_decided_reports_are_denied_on_production_and_ask_before_writes_elsewhere() {
    let writes = [
        "DELETE FROM orders",
        "UPDATE orders SET a = 1",
        "INSERT INTO t VALUES (1)",
        "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x",
        "CALL p()",
        "DROP TABLE t",
        "TRUNCATE t",
        "COPY t FROM '/tmp/x'",
        "SELECT * INTO new_t FROM t",
    ];
    let quiet = [
        "SELECT 1",
        "/*!40101 SET NAMES utf8 */",
        "SET search_path = public",
    ];
    for code in ReasonCode::ALL {
        let pre = |text: &str| {
            let unit = prepared_on(DbmsKind::Postgres, text, &[]);
            let mut r = report_over(text);
            r.items.clear();
            r.pre_decision = Some(vec![reason(code, None)]);
            (unit, r)
        };
        for text in writes.iter().chain(&quiet) {
            let (unit, r) = pre(text);
            for c in [conn_prod(), conn_prod_user_tx(), conn_ro()] {
                let item = one(unit.clone(), r.clone(), &c);
                assert!(
                    matches!(item.decision, GateDecision::Deny(_)),
                    "{code:?} {text}"
                );
            }
            let dev = one(unit.clone(), r.clone(), &conn_dev());
            if code == ReasonCode::NotImplemented {
                assert_eq!(deny_id(&dev.decision), "exec-gate-not-classified");
                continue;
            }
            assert_eq!(dev.verdict, UnitVerdict::Unknown);
            if writes.contains(text) {
                match &dev.decision {
                    GateDecision::Confirm {
                        reasons,
                        suppressible: false,
                        type_to_confirm: None,
                        ..
                    } => {
                        assert_eq!(reasons[0].fluent_id, "exec-gate-not-analysed");
                        assert!(reasons.iter().any(|x| x.fluent_id == code.fluent_id()));
                    }
                    other => panic!("{code:?} {text}: {other:?}"),
                }
            } else {
                assert_eq!(dev.decision, GateDecision::Run, "{code:?} {text}");
            }
        }
        // On production the denial names the classifier's reason.
        let (unit, r) = pre("SELECT 1");
        if code != ReasonCode::NotImplemented {
            assert_eq!(
                deny_id(&one(unit, r, &conn_prod()).decision),
                code.fluent_id()
            );
        }
    }
    // A run-as-one unit the splitter could not settle (only `AmbiguousBoundary`) follows the
    // same rule off production (R-M3-60 is denied on production: PO list).
    let text = "DELETE FROM t; SELECT 1";
    let mut unit = prepared_on(DbmsKind::Postgres, text, &[]);
    unit.unit.kind = sql_split::exec_range::UnitKind::Verbatim;
    let mut r = report_over(text);
    r.items.clear();
    r.pre_decision = Some(vec![reason(ReasonCode::AmbiguousBoundary, None)]);
    assert_eq!(
        shape(&one(unit.clone(), r.clone(), &conn_dev()).decision),
        "confirm"
    );
    assert_eq!(
        deny_id(&one(unit, r, &conn_prod()).decision),
        "safety-reason-ambiguous-boundary"
    );
    // MySQL runs executable comments: a write hidden in one asks; a dump's `SET NAMES` runs.
    for (text, asks) in [
        ("SELECT 1 /*!50000 , (DELETE FROM t) */", true),
        ("/*!40101 SET NAMES utf8 */", false),
        ("/*!40000 ALTER TABLE t DISABLE KEYS */", true),
    ] {
        let unit = prepared_on(DbmsKind::MySql, text, &[]);
        let mut r = fake_classify(text, DialectId::MySql);
        r.items.clear();
        r.pre_decision = Some(vec![reason(ReasonCode::ExecutableComment, None)]);
        let item = one(unit, r, &mysql(EnvTag::Dev));
        assert_eq!(shape(&item.decision) == "confirm", asks, "{text}");
    }
    // MySQL: a pre-decided `BEGIN` in an open transaction commits it; it asks.
    let manual = ConnSafety {
        tx_mode: TxMode::Manual,
        ..mysql(EnvTag::Dev)
    };
    let unit = prepared_on(DbmsKind::MySql, "BEGIN", &[]);
    let mut r = fake_classify("BEGIN", DialectId::MySql);
    r.items.clear();
    r.pre_decision = Some(vec![reason(ReasonCode::ParseError, None)]);
    let item = one(unit, r, &manual);
    assert_eq!(item.verdict, UnitVerdict::ImplicitCommit);
    assert!(reason_ids(&item.decision).contains(&"exec-gate-implicit-commit"));
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
    // ## m3b-t26-prod-unlock (fix round 2, N1/R1/A R1-m2; was "a suppressible production
    // confirmation and warning"): on production "don't ask again" is never offered, for any
    // class, and a pair recorded anywhere (on production before, or off it) never silences it.
    for text in [
        "CREATE TABLE x (a int)",
        "SELECT * FROM t FOR UPDATE",
        "BEGIN",
    ] {
        let asked = run(text, &conn_prod(), &SuppressSet::default(), 7);
        assert!(
            matches!(
                asked.decision,
                GateDecision::Confirm {
                    suppressible: false,
                    ..
                } | GateDecision::Warn(_)
            ),
            "{text}: {:?}",
            asked.decision
        );
        let off = run(text, &conn_dev(), &SuppressSet::default(), 7);
        let mut s = SuppressSet::default();
        s.insert(7, asked.fingerprint);
        s.insert(7, off.fingerprint);
        assert_eq!(
            run(text, &conn_prod(), &s, 7).decision,
            asked.decision,
            "{text}: still asks"
        );
    }
    // Fix round 1 (controller ruling): on production "don't ask again" never covers a
    // destructive, administrative, routine or unknown statement, even with its pair recorded.
    for text in [
        "DROP TABLE orders",
        "TRUNCATE orders",
        "GRANT SELECT ON t TO bob",
        "CALL p()",
        "SELECT f_write()",
    ] {
        let asked = run(text, &conn_prod(), &SuppressSet::default(), 7);
        let mut s = SuppressSet::default();
        s.insert(7, asked.fingerprint);
        assert!(
            matches!(
                run(text, &conn_prod(), &s, 7).decision,
                GateDecision::Confirm {
                    suppressible: false,
                    ..
                }
            ),
            "{text}"
        );
    }
    // M4: the key includes the class: a pair recorded at a lower class never silences the
    // same shape at a higher one.
    let unit = prepared_on(DbmsKind::Postgres, "SELECT f(1)", &[]);
    let lock = over(
        &unit.final_text,
        DialectId::Postgres,
        vec![fake_stmt("SELECT * FROM t FOR UPDATE")],
    );
    let unknown = over(
        &unit.final_text,
        DialectId::Postgres,
        vec![fake_stmt("SELECT f_write()")],
    );
    let low = one(unit.clone(), lock, &conn_prod());
    let mut s = SuppressSet::default();
    s.insert(7, low.fingerprint);
    let high = gate(&[unit], &[unknown], &conn_dev(), &s, 7)
        .items
        .remove(0);
    assert_ne!(low.fingerprint, high.fingerprint);
    // M9: a unit of several statements with a write is never suppressible.
    let text = "SELECT 1; DELETE FROM orders";
    let item = gated(
        text,
        vec![fake_stmt("SELECT 1"), fake_stmt("DELETE FROM orders")],
        &conn_dev(),
    );
    assert!(matches!(
        item.decision,
        GateDecision::Confirm {
            suppressible: false,
            ..
        }
    ));
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
        assert!(!item.effects.contains(SessionEffects::COMMITS_FIRST));
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
    // Fix round 2 (R1-I1): `DDL` is "a DDL statement"; `COMMITS_FIRST` is MySQL's implicit commit.
    let grant = e("GRANT SELECT ON t TO bob", mysql(EnvTag::Dev));
    assert!(grant.contains(SessionEffects::COMMITS_FIRST) && !grant.contains(SessionEffects::DDL));
    assert!(!e("GRANT SELECT ON t TO bob", conn_dev()).contains(SessionEffects::COMMITS_FIRST));
    let create = e("CREATE TABLE x (a int)", conn_dev());
    assert!(
        create.contains(SessionEffects::DDL) && !create.contains(SessionEffects::COMMITS_FIRST)
    );
}

// ---------------------------------------------------------------------------------------------
// The verdict table (R-M3-55) for every combination, against an independent oracle

/// One representative statement per gate class, with the text it classifies.
fn base(class: GateClass) -> (&'static str, StmtReport) {
    match class {
        GateClass::Read => ("SELECT 1", stmt(StmtKind::Select, EffectSet::READ)),
        GateClass::Session => ("SET hv_custom = 1", fake_stmt("SET hv_custom = 1")),
        GateClass::Tx => ("BEGIN", stmt(StmtKind::Begin, EffectSet::TXN_CONTROL)),
        GateClass::Lock => (
            "SELECT * FROM t FOR UPDATE",
            fake_stmt("SELECT * FROM t FOR UPDATE"),
        ),
        GateClass::Routine => ("CALL p()", fake_stmt("CALL p()")),
        GateClass::Dml => (
            "UPDATE orders SET a = 1 WHERE id = 1",
            stmt(StmtKind::Update, EffectSet::WRITE_DATA),
        ),
        GateClass::DmlUnsafe => (
            "DELETE FROM orders",
            report(
                StmtKind::Delete,
                EffectSet::WRITE_DATA,
                &[DestructiveKind::NoWhere],
                "orders",
            ),
        ),
        GateClass::DdlOther => (
            "CREATE TABLE x (a int)",
            stmt(StmtKind::CreateTable, EffectSet::DDL),
        ),
        GateClass::DdlDestructive => ("DROP TABLE orders", fake_stmt("DROP TABLE orders")),
        GateClass::Admin => (
            "GRANT SELECT ON t TO bob",
            stmt(StmtKind::Grant, EffectSet::PRIVILEGE),
        ),
        GateClass::Unknown => ("SELECT f(1)", {
            let mut s = stmt(StmtKind::Select, EffectSet::READ);
            s.reasons.push(reason(ReasonCode::FnUnknown, Some("f")));
            s
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Dev,
    Staging,
    Local,
    Test,
    Untagged,
    Production,
    ReadOnly,
    ProductionReadOnly,
}

impl Level {
    const ALL: [Level; 8] = [
        Level::Dev,
        Level::Staging,
        Level::Local,
        Level::Test,
        Level::Untagged,
        Level::Production,
        Level::ReadOnly,
        Level::ProductionReadOnly,
    ];

    fn env(self) -> Option<EnvTag> {
        match self {
            Level::Dev | Level::ReadOnly => Some(EnvTag::Dev),
            Level::Staging => Some(EnvTag::Staging),
            Level::Local => Some(EnvTag::Local),
            Level::Test => Some(EnvTag::Test),
            Level::Untagged => None,
            Level::Production | Level::ProductionReadOnly => Some(EnvTag::Production),
        }
    }

    fn read_only(self) -> bool {
        matches!(self, Level::ReadOnly | Level::ProductionReadOnly)
    }
}

const ENGINES: [DbmsKind; 5] = [
    DbmsKind::Postgres,
    DbmsKind::MySql,
    DbmsKind::MariaDb,
    DbmsKind::TiDb,
    DbmsKind::Sqlite,
];

const TXS: [(TxStatus, TxOwner, TxMode); 6] = [
    (TxStatus::Idle, TxOwner::None, TxMode::Auto),
    (TxStatus::Idle, TxOwner::None, TxMode::Manual),
    (TxStatus::Idle, TxOwner::None, TxMode::Smart),
    (TxStatus::InTx, TxOwner::User, TxMode::Manual),
    (TxStatus::InTx, TxOwner::User, TxMode::Auto),
    (TxStatus::Failed, TxOwner::User, TxMode::Manual),
];

#[derive(Debug, PartialEq, Eq)]
struct Want {
    shape: &'static str,
    typed: Option<&'static str>,
    suppressible: Option<bool>,
    irreversible: Option<bool>,
    verdict: UnitVerdict,
}

fn is_mysql(engine: DbmsKind) -> bool {
    matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb | DbmsKind::TiDb)
}

/// R-M3-55 written out on its own (it reads none of the gate's code), with the fix-round-1
/// rulings: the class column, the connection rows, the AND-check with `decide(Admin)` (the
/// `admin` column of `data/profiles.toml`), type-to-confirm from the destructive kinds, the
/// MySQL implicit commit (DDL, privileges, `BEGIN`), the suppression rules and the verdict.
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
    let mysql = is_mysql(engine);
    let ddl = matches!(class, G::DdlOther | G::DdlDestructive)
        || extra.is_some_and(|(k, _)| !k.is_unsafe_dml());
    // MySQL commits an open transaction before DDL, GRANT and BEGIN.
    let commits = mysql && (ddl || class == G::Admin || class == G::Tx);
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
    if level.read_only() {
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
    let implicit = commits && tx_open;
    let mut shape = if production {
        match eff {
            G::Read | G::Session => "run",
            G::Lock => "warn",
            // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): asked before it is sent
            // on the MySQL family.
            G::Dml if mysql => "confirm",
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
    // ## m3b-t26-prod-unlock (fix round 2, N1/R1): never on production, for any class.
    let never_suppressed = production;
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
        suppressible: (shape == "confirm").then_some(!typed && !implicit && !never_suppressed),
        irreversible: (shape == "confirm").then_some(mysql && (ddl || class == G::Admin)),
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

fn conn_at(
    level: Level,
    engine: DbmsKind,
    tx: (TxStatus, TxOwner, TxMode),
    guard: bool,
) -> ConnSafety {
    ConnSafety {
        engine,
        env: level.env(),
        read_only: level.read_only(),
        tx_status: tx.0,
        tx_owner: tx.1,
        tx_mode: tx.2,
        confirm_unsafe_where: guard,
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
    let mut cases = 0u32;
    for class in GateClass::ALL {
        let (text, rep) = base(class);
        assert_eq!(gate_class(&rep), class, "the representative of {class:?}");
        for extra in &extras {
            let mut s = rep.clone();
            if let Some((kind, object_kind)) = extra {
                s.destructive.push(DestructiveReason {
                    kind: *kind,
                    range: TextRange::default(),
                    target: Some(object("orders", *object_kind)),
                });
            }
            for level in Level::ALL {
                for engine in ENGINES {
                    for tx in TXS {
                        for guard in [true, false] {
                            let c = conn_at(level, engine, tx, guard);
                            let tx_open = tx.0 != TxStatus::Idle || tx.2 == TxMode::Manual;
                            let item = gated(text, vec![s.clone()], &c);
                            let want = oracle(class, *extra, level, engine, tx_open, guard);
                            assert_eq!(
                                observed(&item),
                                want,
                                "{class:?} `{text}` + {extra:?} on {level:?} {engine:?} \
                                 {tx:?} unsafe-where={guard}: {:?}",
                                item.decision
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 11 * 14 * 8 * 5 * 6 * 2);
}

/// Fix round 1 (M1): `EXPLAIN ANALYZE` of every class, at every level and engine, against the
/// D-062 rows: a read runs; `Routine`/`Admin` inners (and MySQL-family DDL inners) are refused;
/// every other inner is wrapped, and on production asks first when the inner is `Unknown` or
/// the engine is MySQL-family (I2, M7); read-only denies every non-read.
#[test]
fn explain_analyze_table_for_every_inner_class() {
    use GateClass as G;
    let mut cases = 0u32;
    for inner in GateClass::ALL {
        let (text, mut s) = base(inner);
        s.kind = StmtKind::Explain { analyze: true };
        s.flags |= RiskFlags::EXPLAIN_ANALYZE;
        let text = format!("EXPLAIN ANALYZE {text}");
        for level in Level::ALL {
            for engine in ENGINES {
                let c = conn_at(level, engine, TXS[0], true);
                let item = gated(&text, vec![s.clone()], &c);
                let mysql = is_mysql(engine);
                let production = level == Level::Production;
                let want = if inner == G::Read {
                    "run"
                } else if level.read_only()
                    || matches!(inner, G::Routine | G::Admin)
                    || (mysql && matches!(inner, G::DdlOther | G::DdlDestructive))
                {
                    "deny"
                } else if production {
                    // ## m3b-t26-prod-unlock (fix round 1, B-M2): every inner on production.
                    "confirm"
                } else {
                    "explain-wrap"
                };
                assert_eq!(
                    shape(&item.decision),
                    want,
                    "{inner:?} on {level:?} {engine:?}"
                );
                if matches!(want, "confirm" | "explain-wrap") {
                    assert_eq!(item.verdict, UnitVerdict::ExplainAnalyzeRollback);
                }
                if want == "confirm" {
                    assert!(matches!(
                        item.decision,
                        GateDecision::Confirm {
                            suppressible: false,
                            type_to_confirm: None,
                            ..
                        }
                    ));
                }
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 11 * 8 * 5);
}

/// Fix round 1 (I3, I5, M1, M3): MySQL and MariaDB rows over the real interim backend's reports
/// (not a builder): `DROP DATABASE`/`DROP SCHEMA` ask for the typed name on production, the
/// implicit-commit list asks in an open transaction, temporary-table DDL does not.
#[test]
fn mysql_rows_over_the_real_backend() {
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let real = |text: &str| {
            let ctx = sql_safety::ClassifyCtx::new(
                common::gate_fx::ctx_of(engine),
                sql_dialect::ClientFlavor::None,
                sql_safety::Expect::SingleStatement,
            );
            sql_safety::classify_interim(text, &ctx)
        };
        let run = |text: &str, c: &ConnSafety| {
            let unit = prepared_on(engine, text, &[]);
            let report = real(&unit.final_text);
            one(unit, report, c)
        };
        let prod = conn(engine, Some(EnvTag::Production));
        for (text, name) in [
            ("DROP DATABASE shop", "shop"),
            ("DROP SCHEMA IF EXISTS Shop", "Shop"),
            ("DROP DATABASE `my shop`", "my shop"),
            ("DELETE FROM orders", "orders"),
            ("UPDATE Orders SET a = 1", "Orders"),
        ] {
            let item = run(text, &prod);
            match &item.decision {
                GateDecision::Confirm {
                    type_to_confirm: Some(t),
                    suppressible: false,
                    ..
                } => assert_eq!(&**t, name, "{engine:?} {text}"),
                other => panic!("{engine:?} {text}: {other:?}"),
            }
        }
        // DROP TABLE: a confirmation that says it cannot be undone; never suppressible on
        // production (DdlDestructive), no typed name.
        let item = run("DROP TABLE orders", &prod);
        assert!(
            matches!(
                item.decision,
                GateDecision::Confirm {
                    type_to_confirm: None,
                    irreversible: true,
                    suppressible: false,
                    ..
                }
            ),
            "{engine:?}: {:?}",
            item.decision
        );
        // The implicit-commit list on a Manual session (a transaction always counts as open).
        let manual = ConnSafety {
            tx_mode: TxMode::Manual,
            ..conn(engine, Some(EnvTag::Dev))
        };
        for text in [
            "CREATE TABLE x (a int)",
            "ALTER TABLE t ADD b int",
            "CREATE INDEX i ON t (a)",
            "GRANT SELECT ON t TO bob",
            "LOCK TABLES t WRITE",
            "BEGIN",
            "START TRANSACTION",
            "SET autocommit = 1",
            "ANALYZE TABLE t",
            "OPTIMIZE TABLE t",
            "FLUSH TABLES",
            "TRUNCATE t",
            "RENAME TABLE a TO b",
            "CREATE USER bob",
            "SET PASSWORD FOR bob = 'x'",
        ] {
            let item = run(text, &manual);
            assert_eq!(
                item.verdict,
                UnitVerdict::ImplicitCommit,
                "{engine:?} {text}: {:?}",
                item.decision
            );
            assert!(
                item.effects.contains(SessionEffects::COMMITS_FIRST),
                "{engine:?} {text}"
            );
            assert!(
                reason_ids(&item.decision).contains(&"exec-gate-implicit-commit"),
                "{engine:?} {text}"
            );
        }
        // These do not commit: temporary tables (M3), autocommit off, plain reads and writes.
        for text in [
            "CREATE TEMPORARY TABLE x (a int)",
            "DROP TEMPORARY TABLE x",
            "SET autocommit = 0",
            "SELECT 1",
            "UPDATE t SET a = 1 WHERE id = 1",
        ] {
            let item = run(text, &manual);
            assert_ne!(
                item.verdict,
                UnitVerdict::ImplicitCommit,
                "{engine:?} {text}"
            );
            assert!(
                !item.effects.contains(SessionEffects::COMMITS_FIRST),
                "{engine:?} {text}"
            );
            // Temporary-table DDL is still DDL (R1-I1), just no implicit commit.
            let ddl = text.contains("TEMPORARY");
            assert_eq!(
                item.effects.contains(SessionEffects::DDL),
                ddl,
                "{engine:?} {text}"
            );
        }
        // `LOAD DATA` commits first on MySQL (some engines) but is no DDL (R1-I1: T16 reads
        // `DDL` as "schema change" and must keep its row count).
        let item = run("LOAD DATA INFILE '/tmp/x' INTO TABLE t", &manual);
        assert!(
            item.effects.contains(SessionEffects::COMMITS_FIRST),
            "{engine:?}"
        );
        assert!(!item.effects.contains(SessionEffects::DDL), "{engine:?}");
        assert_eq!(item.verdict, UnitVerdict::ImplicitCommit, "{engine:?}");
        // An idle Auto session has nothing to commit.
        let idle = conn(engine, Some(EnvTag::Dev));
        assert_ne!(run("BEGIN", &idle).verdict, UnitVerdict::ImplicitCommit);
    }
}

// ---------------------------------------------------------------------------------------------
// The end-to-end gate over the real classifier (P-M3b-16): Task 28 turns it on.

// ## m3b-t28-matrix (named exception): on since Task 14's `classify` merged; the same check
// over the live Docker matrix is `docker_m3b_matrix::gate_end_to_end_over_real_classify`.
#[test]
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

// ---------------------------------------------------------------------------------------------
// Fix round 2

/// R1-M1 and R1-M2: an unanalysed report's scan reads the raw words of the whole text, so a
/// write the default lexer reads as a string (MySQL `NO_BACKSLASH_ESCAPES`) still asks, and the
/// added administrative and write keywords ask; a dump's session `SET`s still run.
#[test]
fn unanalysed_scan_reads_raw_words() {
    let pre = |engine: DbmsKind, text: &str| {
        let unit = prepared_on(engine, text, &[]);
        let mut r = fake_classify(text, dialect_of(engine));
        r.items.clear();
        r.pre_decision = Some(vec![reason(ReasonCode::LexError, None)]);
        one(unit, r, &conn(engine, Some(EnvTag::Dev))).decision
    };
    let asks = |d: &GateDecision| {
        matches!(
            d,
            GateDecision::Confirm {
                suppressible: false,
                ..
            }
        )
    };
    // Under NO_BACKSLASH_ESCAPES the server reads `'a\'` as a whole string and runs the DELETE.
    assert!(asks(&pre(
        DbmsKind::MySql,
        r"SELECT 'a\'; DELETE FROM t; -- '"
    )));
    for text in [
        "COMMENT ON TABLE t IS 'x'",
        "NOTIFY ch",
        "ANALYZE t",
        "CLUSTER t",
        "RESTART",
    ] {
        assert!(asks(&pre(DbmsKind::Postgres, text)), "{text}");
    }
    for text in [
        "XA START 'x'",
        "BINLOG 'abc'",
        "CLONE LOCAL DATA DIRECTORY = '/tmp/x'",
        "SET GLOBAL max_connections = 1",
        "SET PERSIST max_connections = 1",
        "SET PERSIST_ONLY max_connections = 1",
        "SET @@global.max_connections = 1",
    ] {
        assert!(asks(&pre(DbmsKind::MySql, text)), "{text}");
    }
    for text in [
        "/*!40101 SET NAMES utf8 */",
        "/*!40014 SET @OLD_UNIQUE_CHECKS=@@UNIQUE_CHECKS, UNIQUE_CHECKS=0 */",
        "SELECT @@global.version",
    ] {
        assert_eq!(pre(DbmsKind::MySql, text), GateDecision::Run, "{text}");
    }
}

/// R1-M3: a comma-listed `DROP SCHEMA`/`DROP DATABASE` read from the words names no single
/// name to type, so it is denied on production.
#[test]
fn a_listed_drop_schema_from_the_words_is_denied() {
    let bare = stmt(StmtKind::Drop, EffectSet::DDL);
    for text in ["DROP SCHEMA a, b", "DROP SCHEMA IF EXISTS a , b CASCADE"] {
        let item = gated(text, vec![bare.clone()], &conn_prod());
        assert_eq!(deny_id(&item.decision), "exec-unsafe-no-target", "{text}");
    }
    // One name still asks to type it.
    let item = gated("DROP SCHEMA a CASCADE", vec![bare], &conn_prod());
    assert!(matches!(
        &item.decision,
        GateDecision::Confirm { type_to_confirm: Some(t), .. } if &**t == "a"
    ));
}

// ## m3b-t26-prod-unlock (T26 carry MUST 1, the T28 re-check): MySQL/MariaDB multi-table and
// joined `UPDATE`s over the real interim backend. The typed name is the table that changes
// (the `SET` target), never the first table listed; two changed tables are several targets,
// denied on production as a multi-table `DELETE` is; the comma form `UPDATE a, b SET …` gets
// real targets (it was unparsed: `Unknown`, a plain confirmation).
#[test]
fn joined_and_comma_updates_ask_for_the_table_that_changes() {
    for engine in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let real = |text: &str| {
            let ctx = sql_safety::ClassifyCtx::new(
                common::gate_fx::ctx_of(engine),
                sql_dialect::ClientFlavor::None,
                sql_safety::Expect::SingleStatement,
            );
            sql_safety::classify_interim(text, &ctx)
        };
        let run = |text: &str| {
            let unit = prepared_on(engine, text, &[]);
            let report = real(&unit.final_text);
            one(unit, report, &conn(engine, Some(EnvTag::Production)))
        };
        // The table that changes is the one to type.
        for (text, name) in [
            ("UPDATE a JOIN b ON a.id = b.id SET b.y = 2", "b"),
            ("UPDATE a JOIN b ON a.id = b.id SET a.y = 2", "a"),
            ("UPDATE a AS x JOIN b AS z ON x.id = z.id SET z.y = 2", "b"),
            (
                "UPDATE shop.a JOIN shop.b ON a.id = b.id SET shop.b.y = 2",
                "shop.b",
            ),
            ("UPDATE a, b SET b.y = 2", "b"),
            ("UPDATE a x, b z SET z.y = 2, z.w = 3", "b"),
            ("UPDATE LOW_PRIORITY a, b SET a.y = b.y", "a"),
            // A join without `ON` names no target of its own: the table it writes.
            ("UPDATE a JOIN b SET b.y = 2 WHERE a.id = b.id", "b"),
            // A MySQL modifier is never the table (it was read as one before Task 26).
            ("UPDATE LOW_PRIORITY orders SET a = 1", "orders"),
            ("DELETE QUICK FROM orders", "orders"),
            // ## m3b-t26-prod-unlock (fix round 1): nested joins (A-C1), a WITH prefix (A-C2),
            // a multi-table DELETE's alias (B-C1), a WHERE that keeps every row (review B).
            (
                "UPDATE a JOIN (b JOIN c ON b.id = c.id) ON a.id = b.id SET c.y = 2",
                "c",
            ),
            ("UPDATE a, (b JOIN c ON b.id = c.id) SET c.y = 2", "c"),
            (
                "WITH x AS (SELECT 1) UPDATE LOW_PRIORITY orders SET a = 1",
                "orders",
            ),
            ("WITH x AS (SELECT 1) DELETE QUICK FROM orders", "orders"),
            ("DELETE archive FROM orders AS archive", "orders"),
            ("DELETE FROM archive USING orders AS archive", "orders"),
            ("DELETE t FROM t AS u, u AS t", "u"),
            ("DELETE FROM orders WHERE id = 3 || 1", "orders"),
            ("UPDATE orders SET a = 1 WHERE id <=> id", "orders"),
            ("UPDATE (a JOIN b ON a.id = b.id) SET b.y = 2", "b"),
            (
                "UPDATE a AS p JOIN (b AS q JOIN a AS r ON q.id = r.id) ON p.id = q.id SET q.y = 2",
                "b",
            ),
            ("UPDATE LOW_PRIORITY IGNORE a, b SET b.y = 2", "b"),
            (
                "WITH w AS (SELECT 1) UPDATE LOW_PRIORITY a JOIN b ON a.id = b.id SET b.y = 2",
                "b",
            ),
            ("DELETE q FROM a AS p JOIN b AS q ON p.id = q.id", "b"),
            ("DELETE FROM q USING a AS p JOIN b AS q ON p.id = q.id", "b"),
            ("DELETE QUICK IGNORE b FROM a JOIN b ON a.id = b.id", "b"),
            (
                "WITH w AS (SELECT 1) DELETE QUICK q FROM a AS p JOIN b AS q ON p.id = q.id",
                "b",
            ),
        ] {
            let item = run(text);
            match &item.decision {
                GateDecision::Confirm {
                    type_to_confirm: Some(t),
                    suppressible: false,
                    ..
                } => assert_eq!(&**t, name, "{engine:?} {text}"),
                other => panic!("{engine:?} {text}: {other:?}"),
            }
            assert_ne!(item.class, GateClass::Unknown, "{engine:?} {text}: parsed");
        }
        // Two tables change, or a column whose table is unknown: several targets, denied.
        for text in [
            "UPDATE a JOIN b ON a.id = b.id SET a.x = 1, b.y = 2",
            "UPDATE a JOIN b ON a.id = b.id SET y = 2",
            "UPDATE a, b SET a.x = 1, b.y = 2",
            "UPDATE a, b SET y = 2",
            "UPDATE a JOIN b SET a.x = 1, b.y = 2 WHERE a.id = b.id",
        ] {
            let item = run(text);
            assert_eq!(
                deny_id(&item.decision),
                "exec-unsafe-several-targets",
                "{engine:?} {text}"
            );
        }
        // ## m3b-t26-prod-unlock (fix round 1, A-C1, B-C1): a reference that names no table is
        // never guessed: denied on production (was several targets for the comma form).
        for text in [
            "UPDATE a, b SET c.y = 2",
            "UPDATE a JOIN b ON a.id = b.id SET c.y = 2",
            "DELETE q FROM orders AS archive",
        ] {
            let item = run(text);
            assert_eq!(
                deny_id(&item.decision),
                "safety-reason-too-complex",
                "{engine:?} {text}"
            );
        }
        // Bounded by a WHERE: no unsafe pattern, so no typed name; the DML asks before it is
        // sent on the MySQL family (fix round 1, ruling on A-I2/B-I1), never `Unknown`.
        for text in [
            "UPDATE a JOIN b ON a.id = b.id SET b.y = 2 WHERE a.id = 3",
            "UPDATE a, b SET b.y = a.y WHERE a.id = b.id",
        ] {
            let item = run(text);
            assert!(
                matches!(
                    item.decision,
                    GateDecision::Confirm {
                        type_to_confirm: None,
                        ..
                    }
                ),
                "{engine:?} {text}: {:?}",
                item.decision
            );
            assert_eq!(item.class, GateClass::Dml, "{engine:?} {text}");
        }
        // A single-table UPDATE is unchanged.
        let item = run("UPDATE a SET y = 2");
        assert!(
            matches!(&item.decision, GateDecision::Confirm { type_to_confirm: Some(t), .. } if &**t == "a"),
            "{engine:?}: {:?}",
            item.decision
        );
    }
}

// ## m3b-t26-prod-unlock (fix round 1: B-I2, B-M1, B-M2, B-M6) over the real classifier.
fn real_item(engine: DbmsKind, text: &str, c: &ConnSafety, s: &SuppressSet) -> GateItem {
    let dialect = match engine {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        _ => DialectId::Postgres,
    };
    let ctx = sql_safety::ClassifyCtx::new(
        sql_dialect::DialectCtx::new(dialect, None),
        sql_dialect::ClientFlavor::None,
        sql_safety::Expect::SingleStatement,
    );
    let unit = prepared_on(engine, text, &[]);
    let report = sql_safety::classify(&unit.final_text, &ctx);
    gate(&[unit], &[report], c, s, 7).items.remove(0)
}

/// B-I2: a routine definition is never suppressible on production, and its "don't ask again"
/// key holds its whole body elsewhere: a new body is asked again.
#[test]
fn routine_definitions_are_never_suppressed_on_production_and_keyed_by_body() {
    let none = SuppressSet::default();
    let defs = [
        (
            "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE sql AS 'DELETE FROM t RETURNING 1'",
        ),
        (
            "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE sql AS $$SELECT 1$$",
            "CREATE OR REPLACE FUNCTION f() RETURNS int LANGUAGE sql AS $$DELETE FROM t RETURNING 1$$",
        ),
    ];
    for (a, b) in defs {
        let prod = real_item(DbmsKind::Postgres, a, &conn_prod(), &none);
        assert!(
            matches!(
                prod.decision,
                GateDecision::Confirm {
                    suppressible: false,
                    ..
                }
            ),
            "{a}: {:?}",
            prod.decision
        );
        // Elsewhere the key is the body's: suppressing `a` never silences `b`.
        let ka = real_item(DbmsKind::Postgres, a, &conn_staging(), &none).fingerprint;
        let kb = real_item(DbmsKind::Postgres, b, &conn_staging(), &none).fingerprint;
        assert_ne!(ka, kb, "{a} / {b}");
        let mut s = SuppressSet::default();
        s.insert(7, ka);
        let again = real_item(DbmsKind::Postgres, b, &conn_prod(), &s);
        assert_ne!(again.decision, GateDecision::Run, "{b}");
    }
    for text in [
        "CREATE PROCEDURE p() BEGIN DELETE FROM t; END",
        "CREATE TRIGGER tr BEFORE INSERT ON t FOR EACH ROW SET NEW.a = 1",
        "CREATE EVENT e ON SCHEDULE EVERY 1 DAY DO DELETE FROM t",
        "ALTER PROCEDURE p COMMENT 'x'",
    ] {
        let item = real_item(DbmsKind::MySql, text, &mysql(EnvTag::Production), &none);
        assert!(
            matches!(
                item.decision,
                GateDecision::Confirm {
                    suppressible: false,
                    ..
                } | GateDecision::Deny(_)
            ),
            "{text}: {:?}",
            item.decision
        );
    }
}

/// B-M1: on production a statement that turns off later writes' checks or replication, and
/// `NOTIFY`, asks first; the harmless `SET`s still run.
#[test]
fn production_session_settings_that_weaken_later_writes_ask() {
    let none = SuppressSet::default();
    for (engine, c, text) in [
        (
            DbmsKind::Postgres,
            conn_prod(),
            "SET session_replication_role = replica",
        ),
        (DbmsKind::Postgres, conn_prod(), "NOTIFY jobs, 'run'"),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET sql_log_bin = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET SESSION sql_log_bin = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET FOREIGN_KEY_CHECKS = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET @@session.unique_checks = 0",
        ),
        // ## m3b-t26-prod-unlock (fix round 2, R3): quoted names, every spelling, and the
        // replication/durability settings.
        (
            DbmsKind::Postgres,
            conn_prod(),
            "SET \"session_replication_role\" = replica",
        ),
        (
            DbmsKind::Postgres,
            conn_prod(),
            "SET LOCAL session_replication_role = replica",
        ),
        (
            DbmsKind::Postgres,
            conn_prod(),
            "SET synchronous_commit = off",
        ),
        (
            DbmsKind::Postgres,
            conn_prod(),
            "SET SESSION synchronous_commit TO off",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET `foreign_key_checks` = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET @@`foreign_key_checks` = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET foreign_key_checks = OFF",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET @@GLOBAL.FOREIGN_KEY_CHECKS = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET sql_safe_updates = 0",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET gtid_next = 'AUTOMATIC'",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET pseudo_replica_mode = 1",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET pseudo_slave_mode = 1",
        ),
        (
            DbmsKind::MySql,
            mysql(EnvTag::Production),
            "SET a = 1, `unique_checks` = 0",
        ),
    ] {
        let item = real_item(engine, text, &c, &none);
        assert!(
            matches!(
                item.decision,
                GateDecision::Confirm { .. } | GateDecision::Deny(_)
            ),
            "{text}: {:?}",
            item.decision
        );
        assert_ne!(item.decision, GateDecision::Run, "{text}");
    }
    assert_eq!(
        real_item(
            DbmsKind::Postgres,
            "SET search_path = public",
            &conn_prod(),
            &none
        )
        .decision,
        GateDecision::Run
    );
}

/// B-M6: a plain `EXPLAIN` of a data change plans it and runs nothing: a read (no Keep/Undo);
/// B-M2: `EXPLAIN ANALYZE` of it asks first on production.
#[test]
fn plain_explain_of_dml_is_a_read_and_explain_analyze_asks_on_production() {
    let none = SuppressSet::default();
    for (engine, c) in [
        (DbmsKind::Postgres, conn_prod()),
        (DbmsKind::MySql, mysql(EnvTag::Production)),
    ] {
        for text in [
            "EXPLAIN DELETE FROM t WHERE id = 1",
            "EXPLAIN UPDATE t SET a = 1",
        ] {
            let item = real_item(engine, text, &c, &none);
            assert_eq!(item.decision, GateDecision::Run, "{engine:?} {text}");
            assert_eq!(item.verdict, UnitVerdict::Read, "{engine:?} {text}");
        }
    }
    let item = real_item(
        DbmsKind::Postgres,
        "EXPLAIN ANALYZE DELETE FROM t WHERE id = 1",
        &conn_prod(),
        &none,
    );
    assert!(
        matches!(item.decision, GateDecision::Confirm { .. }),
        "{:?}",
        item.decision
    );
    assert_eq!(item.verdict, UnitVerdict::ExplainAnalyzeRollback);
}

// ## m3b-t26-prod-unlock (fix round 2, N1/R1/A R1-m2): on production "don't ask again" never
// applies, whatever the class or engine. Review B's pairs: A answered "don't ask again" (its key
// recorded), then B of the same shape in the same console still asks before anything is sent.
#[test]
fn production_never_suppresses_any_class_on_any_engine() {
    let on = |engine, env| ConnSafety {
        engine,
        ..conn(engine, Some(env))
    };
    let pairs: &[(&str, &str)] = &[
        (
            "DELETE FROM orders WHERE id > 1000000",
            "DELETE FROM orders WHERE id > 0",
        ),
        (
            "DELETE FROM orders WHERE id IN (1)",
            "DELETE FROM orders WHERE id IN (1, 2, 3, 4, 5)",
        ),
        (
            "DELETE FROM orders WHERE id BETWEEN 1 AND 2",
            "DELETE FROM orders WHERE id BETWEEN 0 AND 99999999999",
        ),
        (
            "DELETE FROM orders WHERE created < '2020-01-01'",
            "DELETE FROM orders WHERE created < '2099-01-01'",
        ),
        (
            "UPDATE t SET a = 1 WHERE id = 3",
            "UPDATE t SET a = 1 WHERE id = 4",
        ),
        ("INSERT INTO t VALUES (1)", "INSERT INTO t VALUES (2)"),
        (
            "CREATE OR REPLACE RULE r AS ON INSERT TO t DO ALSO DELETE FROM u WHERE false",
            "CREATE OR REPLACE RULE r AS ON INSERT TO t DO ALSO DELETE FROM u WHERE true",
        ),
        (
            "CREATE POLICY p ON t USING (false)",
            "CREATE POLICY p ON t USING (true)",
        ),
        (
            "ALTER POLICY p ON t USING (false)",
            "ALTER POLICY p ON t USING (true)",
        ),
        (
            "CREATE OR REPLACE VIEW v AS SELECT * FROM t WHERE tenant = 1",
            "CREATE OR REPLACE VIEW v AS SELECT * FROM t WHERE tenant = 2",
        ),
        ("CREATE TABLE x (a int)", "CREATE TABLE y (a int)"),
        (
            "SELECT * FROM t FOR UPDATE",
            "SELECT * FROM t WHERE id = 2 FOR UPDATE",
        ),
    ];
    for engine in [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb] {
        for (a, b) in pairs {
            let pg_only = a.contains("RULE") || a.contains("POLICY");
            if pg_only && engine != DbmsKind::Postgres {
                continue;
            }
            let item =
                |text: &str, env, s: &SuppressSet| real_item(engine, text, &on(engine, env), s);
            let asked = item(a, EnvTag::Production, &SuppressSet::default());
            // Never offered on production.
            assert!(
                !matches!(
                    asked.decision,
                    GateDecision::Confirm {
                        suppressible: true,
                        ..
                    }
                ),
                "{engine:?} `{a}`: {:?}",
                asked.decision
            );
            // A key recorded for A (on production, or off it) never silences B, or A itself.
            let off = item(a, EnvTag::Dev, &SuppressSet::default());
            let mut s = SuppressSet::default();
            s.insert(7, asked.fingerprint);
            s.insert(7, off.fingerprint);
            for text in [a, b] {
                let later = item(text, EnvTag::Production, &s);
                assert_ne!(
                    later.decision,
                    GateDecision::Run,
                    "{engine:?} `{a}` then `{text}`: still asks"
                );
            }
        }
    }
}

// ## m3b-t26-prod-unlock (fix round 2, R1): off production a rule, policy or view keys on its
// whole text, so "don't ask again" for one body never covers another.
#[test]
fn rules_policies_and_views_are_keyed_by_their_body_off_production() {
    for (a, b) in [
        (
            "CREATE OR REPLACE RULE r AS ON INSERT TO t DO ALSO DELETE FROM u WHERE false",
            "CREATE OR REPLACE RULE r AS ON INSERT TO t DO ALSO DELETE FROM u WHERE true",
        ),
        (
            "CREATE POLICY p ON t USING (false)",
            "CREATE POLICY p ON t USING (true)",
        ),
        (
            "ALTER POLICY p ON t USING (false)",
            "ALTER POLICY p ON t USING (true)",
        ),
        (
            "CREATE OR REPLACE VIEW v AS SELECT * FROM t WHERE tenant = 1",
            "CREATE OR REPLACE VIEW v AS SELECT * FROM t WHERE tenant = 2",
        ),
        (
            "ALTER VIEW v AS SELECT * FROM t WHERE tenant = 1",
            "ALTER VIEW v AS SELECT * FROM t WHERE tenant = 2",
        ),
    ] {
        let none = SuppressSet::default();
        let key = |t: &str| real_item(DbmsKind::Postgres, t, &conn_dev(), &none).fingerprint;
        assert_ne!(key(a), key(b), "`{a}` / `{b}`: keyed by the body");
        assert_eq!(key(a), key(a), "`{a}`: stable");
    }
}

// ## m3b-gate-explain: the B-M6 shortcut (a plain `EXPLAIN` of a data change is a read) never
// covers a statement that calls a routine, whatever evidence of the call the report carries:
// the `ROUTINE_CALL` effect bit, a routine reason or a touched call. The seed class
// `no_path_turns_deny_into_run` once found (its seed lost): a lone plain `EXPLAIN` item with a
// DML bit and `ROUTINE_CALL` (gate class `Dml`, the routine hidden under it), no touched call,
// on production → `Run`.

/// The lost seed's class, by hand, on every production shape.
#[test]
fn plain_explain_with_a_routine_call_effect_asks_on_production() {
    let none = SuppressSet::default();
    let text = "EXPLAIN UPDATE t SET a = f(1)";
    for dml in [
        EffectSet::WRITE_DATA,
        EffectSet::WRITE_TEMP,
        EffectSet::SEQUENCE,
        EffectSet::XID_CONSUME,
    ] {
        for extra in [EffectSet::empty(), EffectSet::READ, EffectSet::LOCK_ROW] {
            let effects = dml | extra | EffectSet::ROUTINE_CALL;
            let s = stmt(StmtKind::Explain { analyze: false }, effects);
            assert!(s.touched.calls.is_empty());
            for c in [
                conn_prod_auto(),
                conn_prod_manual_idle(),
                conn_prod_user_tx(),
                ConnSafety {
                    tx_mode: TxMode::Manual,
                    ..mysql(EnvTag::Production)
                },
                mysql(EnvTag::Production),
            ] {
                let unit = prepared_on(c.engine, text, &[]);
                let report = over(&unit.final_text, dialect_of(c.engine), vec![s.clone()]);
                let item = gate(&[unit], &[report], &c, &none, 1).items.remove(0);
                assert!(
                    matches!(
                        item.decision,
                        GateDecision::Confirm { .. } | GateDecision::Deny(_)
                    ),
                    "{effects:?} on {:?} {:?}: {:?}",
                    c.engine,
                    c.tx_mode,
                    item.decision
                );
                assert_ne!(item.verdict, UnitVerdict::Read, "{effects:?}");
            }
        }
    }
}

/// The seed a 30M-case soak of `no_path_turns_deny_into_run` shrank to before the fix (the
/// shortcut ran ahead of the production AND-check): a plain `EXPLAIN` whose effects
/// `decide(Admin)` confirms (session state, NOTIFY, transaction control), on production MySQL
/// in manual mode, ran as a read.
#[test]
fn soaked_seed_plain_explain_with_session_effects_asks_on_production() {
    let s = stmt(
        StmtKind::Explain { analyze: false },
        EffectSet::TXN_CONTROL
            | EffectSet::SESSION_STATE
            | EffectSet::LOCK_ROW
            | EffectSet::SEQUENCE
            | EffectSet::NOTIFY,
    );
    let c = ConnSafety {
        tx_mode: TxMode::Manual,
        ..mysql(EnvTag::Production)
    };
    let item = gated("SELECT 1", vec![s], &c);
    assert!(
        matches!(item.decision, GateDecision::Confirm { .. }),
        "{:?}",
        item.decision
    );
    assert_ne!(item.verdict, UnitVerdict::Read);
}

/// The same class named only by a routine reason (a producer that names the call but sets no
/// bit and touches nothing) keeps its class too.
#[test]
fn plain_explain_with_a_routine_reason_keeps_its_class() {
    for code in [ReasonCode::RoutineCall, ReasonCode::ProceduralCode] {
        let mut s = stmt(StmtKind::Explain { analyze: false }, EffectSet::WRITE_DATA);
        s.reasons.push(reason(code, Some("f")));
        let item = gated("EXPLAIN UPDATE t SET a = f(1)", vec![s], &conn_prod());
        assert_ne!(item.decision, GateDecision::Run, "{code:?}");
        assert_ne!(item.verdict, UnitVerdict::Read, "{code:?}");
    }
}

/// A statement whose kind says plain `EXPLAIN` but whose flag or reason says `ANALYZE` is an
/// `EXPLAIN ANALYZE` (it runs its inner statement): D-062 decides it, never the B-M6 read.
#[test]
fn plain_explain_kind_with_analyze_evidence_is_never_the_read() {
    for analyzed in [
        |s: &mut StmtReport| s.flags |= RiskFlags::EXPLAIN_ANALYZE,
        |s: &mut StmtReport| s.reasons.push(reason(ReasonCode::ExplainAnalyze, None)),
    ] {
        let mut s = stmt(StmtKind::Explain { analyze: false }, EffectSet::WRITE_DATA);
        analyzed(&mut s);
        let text = "EXPLAIN UPDATE t SET a = 1 WHERE id = 1";
        let prod = gated(text, vec![s.clone()], &conn_prod());
        assert!(
            matches!(prod.decision, GateDecision::Confirm { .. }),
            "{:?}",
            prod.decision
        );
        assert_eq!(prod.verdict, UnitVerdict::ExplainAnalyzeRollback);
        let dev = gated(text, vec![s], &conn_dev());
        assert_eq!(dev.decision, GateDecision::ExplainAnalyzeRollback);
    }
}

/// Over the real classifier: a plain `EXPLAIN` of a statement that calls a function (a
/// builtin, a user function, a stored function) is never the B-M6 read on production; a plain
/// `EXPLAIN` of a pure data change, or of a pure read, still runs as a read.
#[test]
fn plain_explain_calling_code_asks_on_production_over_the_real_classifier() {
    let none = SuppressSet::default();
    for (engine, c) in [
        (DbmsKind::Postgres, conn_prod()),
        (DbmsKind::MySql, mysql(EnvTag::Production)),
    ] {
        for text in [
            "EXPLAIN SELECT f_write()",
            "EXPLAIN SELECT * FROM t WHERE a = f_write(1)",
            "EXPLAIN SELECT * FROM (SELECT f_write() AS x) d",
            "EXPLAIN UPDATE t SET a = f_write(1)",
            "EXPLAIN UPDATE t SET a = lower('x') WHERE id = 1",
            "EXPLAIN DELETE FROM t WHERE id = f_write(1)",
            "EXPLAIN INSERT INTO t SELECT f_write()",
        ] {
            let item = real_item(engine, text, &c, &none);
            assert!(
                !matches!(item.decision, GateDecision::Run | GateDecision::Warn(_)),
                "{engine:?} `{text}`: {:?}",
                item.decision
            );
            assert_ne!(item.verdict, UnitVerdict::Read, "{engine:?} `{text}`");
        }
        for text in [
            "EXPLAIN SELECT * FROM t WHERE id = 1",
            "EXPLAIN UPDATE t SET a = 1 WHERE id = 1",
            "EXPLAIN DELETE FROM t WHERE id = 1",
        ] {
            let item = real_item(engine, text, &c, &none);
            assert_eq!(item.decision, GateDecision::Run, "{engine:?} `{text}`");
            assert_eq!(item.verdict, UnitVerdict::Read, "{engine:?} `{text}`");
        }
    }
}
