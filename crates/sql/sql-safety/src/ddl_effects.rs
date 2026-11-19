// ## m4-t01c-core
//! The catalog effects of one statement (`StmtReport.ddl`, the input of HV-INTRO-12's
//! `plan_refresh`). Task 6 reads names, kinds and actions from each backend's parse.
//!
//! **Fail-closed stub (M4 Task 1c).** Every statement that may change the catalog carries
//! exactly one [`DdlEffect::opaque`] effect, so the refresh planner widens it to the session's
//! current namespace (refresh more, never less). A statement is in scope when:
//! - its kind is DDL-class: `CreateTable`, `CreateOther`, `Alter`, `Drop`, `DropOwned` (M3c
//!   P2-07), `Truncate`, `Grant`, `Revoke`;
//! - its kind may run DDL the text does not show: `Do`, `Call`, `Execute`, `Prepare`;
//! - its kind is `Other` (a statement the backend did not name: MySQL `PREPARE … FROM`,
//!   `CREATE EVENT`, `ALTER USER`, `RENAME USER`, SQLite `ATTACH`/`DETACH`, …) or
//!   `Explain { analyze: true }` (it runs the statement: `EXPLAIN ANALYZE CREATE TABLE … AS`,
//!   `EXPLAIN (ANALYZE) SELECT … INTO`) (T1c fix round 1, I2);
//! - its flags hold `EXEC_COMMENT` (a MySQL or MariaDB executable comment), `DYNAMIC_SQL` or
//!   `EXPLAIN_ANALYZE`;
//! - its effects hold `DDL`, `DDL_TEMP`, `PRIVILEGE`, `ADMIN`, `PREPARE`, `ROUTINE_CALL` (a
//!   routine may run DDL) or `UNKNOWN`, or are empty (nothing proved).
//!
//! Every other statement has none. The effects are a function of the statement's kind, effects
//! and flags only, all of which `classify` compares and reports, so two backends (or two
//! lexical readings) that agree on those agree on the effects, and the field never changes a
//! verdict: no verdict path reads it. `classify` recomputes it once more after the
//! context-taint pass (`refresh`), which may add `UNKNOWN` to a later statement.

use crate::effects::{EffectSet, RiskFlags};
use crate::report::{DdlEffect, StmtKind, StmtReport};

/// The effects that may change the catalog.
const CATALOG_EFFECTS: EffectSet = EffectSet::DDL
    .union(EffectSet::DDL_TEMP)
    .union(EffectSet::PRIVILEGE)
    .union(EffectSet::ADMIN)
    .union(EffectSet::PREPARE)
    .union(EffectSet::ROUTINE_CALL)
    .union(EffectSet::UNKNOWN);

/// The flags that hide what a statement runs.
const CATALOG_FLAGS: RiskFlags = RiskFlags::EXEC_COMMENT
    .union(RiskFlags::DYNAMIC_SQL)
    .union(RiskFlags::EXPLAIN_ANALYZE);

/// The stub's effects for a statement of `kind` with `effects` and `flags` (the module docs).
pub(crate) fn stub_effects(kind: StmtKind, effects: EffectSet, flags: RiskFlags) -> Vec<DdlEffect> {
    if in_scope(kind, effects, flags) {
        vec![DdlEffect::opaque()]
    } else {
        Vec::new()
    }
}

/// Sets `item.ddl` from its final kind, effects and flags.
pub(crate) fn refresh(item: &mut StmtReport) {
    item.ddl = stub_effects(item.kind, item.effects, item.flags);
}

/// Whether Task 6 owes the statement an effect.
fn in_scope(kind: StmtKind, effects: EffectSet, flags: RiskFlags) -> bool {
    effects.is_empty()
        || effects.intersects(CATALOG_EFFECTS)
        || flags.intersects(CATALOG_FLAGS)
        || matches!(
            kind,
            StmtKind::CreateTable
                | StmtKind::CreateOther
                | StmtKind::Alter
                | StmtKind::Drop
                | StmtKind::DropOwned
                | StmtKind::Truncate
                | StmtKind::Grant
                | StmtKind::Revoke
                | StmtKind::Do
                | StmtKind::Call
                | StmtKind::Execute
                | StmtKind::Prepare
                | StmtKind::Other
                | StmtKind::Explain { analyze: true }
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN_SCOPE: [StmtKind; 14] = [
        StmtKind::CreateTable,
        StmtKind::CreateOther,
        StmtKind::Alter,
        StmtKind::Drop,
        StmtKind::DropOwned,
        StmtKind::Truncate,
        StmtKind::Grant,
        StmtKind::Revoke,
        StmtKind::Do,
        StmtKind::Call,
        StmtKind::Execute,
        StmtKind::Prepare,
        StmtKind::Other,
        StmtKind::Explain { analyze: true },
    ];

    /// Kinds that change no catalog by themselves.
    const PLAIN: [StmtKind; 20] = [
        StmtKind::Select,
        StmtKind::Values,
        StmtKind::Table,
        StmtKind::Insert,
        StmtKind::Update,
        StmtKind::Delete,
        StmtKind::Merge,
        StmtKind::Copy,
        StmtKind::Explain { analyze: false },
        StmtKind::Set,
        StmtKind::Show,
        StmtKind::Begin,
        StmtKind::Commit,
        StmtKind::Rollback,
        StmtKind::Savepoint,
        StmtKind::Lock,
        StmtKind::Listen,
        StmtKind::Notify,
        StmtKind::Use,
        StmtKind::Pragma,
    ];

    #[test]
    fn every_scoped_kind_is_one_opaque_effect() {
        for kind in IN_SCOPE {
            assert_eq!(
                stub_effects(kind, EffectSet::READ, RiskFlags::empty()),
                vec![DdlEffect::opaque()],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn plain_kinds_have_none_unless_effects_or_flags_say_otherwise() {
        for kind in PLAIN {
            assert!(
                stub_effects(
                    kind,
                    EffectSet::READ | EffectSet::WRITE_DATA,
                    RiskFlags::empty()
                )
                .is_empty(),
                "{kind:?}"
            );
            for flags in [
                RiskFlags::EXEC_COMMENT,
                RiskFlags::DYNAMIC_SQL,
                RiskFlags::EXPLAIN_ANALYZE,
            ] {
                assert_eq!(
                    stub_effects(kind, EffectSet::READ, flags),
                    vec![DdlEffect::opaque()],
                    "{kind:?} {flags:?}"
                );
            }
            for effects in [
                EffectSet::empty(),
                EffectSet::DDL,
                EffectSet::DDL_TEMP,
                EffectSet::PRIVILEGE,
                EffectSet::ADMIN,
                EffectSet::PREPARE,
                EffectSet::ROUTINE_CALL,
                EffectSet::UNKNOWN,
            ] {
                assert_eq!(
                    stub_effects(kind, effects, RiskFlags::empty()),
                    vec![DdlEffect::opaque()],
                    "{kind:?} {effects:?}"
                );
            }
        }
    }

    /// The effects `classify` reports per statement of `sql` in dialect `d`.
    fn classified(d: sql_dialect::DialectId, sql: &str) -> Vec<Vec<DdlEffect>> {
        let ctx = crate::ClassifyCtx::new(
            sql_dialect::DialectCtx::new(d, None),
            sql_dialect::ClientFlavor::None,
            crate::Expect::Script,
        );
        let r = crate::classify(sql, &ctx);
        assert!(r.pre_decision.is_none(), "{sql}: {:?}", r.pre_decision);
        r.items.into_iter().map(|i| i.ddl).collect()
    }

    #[test]
    fn both_backends_fill_the_stub_through_classify() {
        use sql_dialect::DialectId;
        let none: Vec<DdlEffect> = Vec::new();
        let opaque = vec![DdlEffect::opaque()];
        for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
            assert_eq!(
                classified(d, "SELECT 1; CREATE TABLE t (a int); DROP TABLE t"),
                [none.clone(), opaque.clone(), opaque.clone()],
                "{d:?}"
            );
            assert_eq!(
                classified(d, "SELECT 1"),
                std::slice::from_ref(&none),
                "{d:?}"
            );
        }
        assert_eq!(
            classified(DialectId::Postgres, "DO $$ BEGIN END $$; DROP OWNED BY r"),
            [opaque.clone(), opaque.clone()]
        );
        // An executable comment: whatever the reading, refresh the namespace.
        assert_eq!(
            classified(DialectId::MySql, "SELECT 1 /*!50000 , 2 */"),
            [opaque]
        );
    }

    /// The T1c review's probe H1: statements the classifier files under `Other` or
    /// `Explain { analyze: true }` that change the catalog (carry for Task 6's scope).
    #[test]
    fn review_h1_statements_are_opaque() {
        use sql_dialect::DialectId::{MySql, Postgres, Sqlite};
        let cases = [
            (MySql, "PREPARE s FROM 'DROP TABLE t'"),
            (
                MySql,
                "CREATE EVENT e ON SCHEDULE EVERY 1 HOUR DO DELETE FROM t",
            ),
            (MySql, "ALTER USER 'u'@'%' ACCOUNT LOCK"),
            (MySql, "RENAME USER 'a'@'%' TO 'b'@'%'"),
            (Postgres, "EXPLAIN ANALYZE CREATE TABLE t3 AS SELECT 1"),
            (Postgres, "EXPLAIN (ANALYZE) SELECT 1 INTO t4"),
            (Postgres, "PREPARE p AS SELECT 1"),
            (Sqlite, "ATTACH DATABASE 'other.db' AS other"),
            (Sqlite, "DETACH DATABASE other"),
        ];
        for (d, sql) in cases {
            assert_eq!(
                classified(d, sql),
                [vec![DdlEffect::opaque()]],
                "{d:?}: {sql}"
            );
        }
    }

    #[test]
    fn opaque_names_nothing() {
        let e = DdlEffect::opaque();
        assert!(e.opaque);
        assert_eq!(e.name, None);
        assert_eq!(e.on, None);
        assert!(e.referenced.is_empty());
    }
}
