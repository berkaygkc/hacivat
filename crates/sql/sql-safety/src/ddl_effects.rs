// ## m4-t01c-core
//! The catalog effects of one statement (`StmtReport.ddl`, the input of HV-INTRO-12's
//! `plan_refresh`). Task 6 reads names, kinds and actions from each backend's parse.
//!
//! **Fail-closed stub (M4 Task 1c).** Every statement in Task 6's scope carries exactly one
//! [`DdlEffect::opaque`] effect, so the refresh planner widens it to the session's current
//! namespace (refresh more, never less). The scope:
//! - the DDL-class kinds `CreateTable`, `CreateOther`, `Alter`, `Drop`, `DropOwned` (M3c
//!   P2-07), `Truncate`, `Grant`, `Revoke`;
//! - statements that may run DDL the text does not show: `DO`, `CALL`, `EXECUTE`, `PREPARE`;
//! - any statement with a MySQL or MariaDB executable comment (`RiskFlags::EXEC_COMMENT`).
//!
//! Every other statement has none. The effects are a function of the statement's kind and flags
//! only, so two backends (or two lexical readings) that agree on those agree on the effects,
//! and the field never changes a verdict.

use crate::effects::RiskFlags;
use crate::report::{DdlEffect, StmtKind};

/// The stub's effects for a statement of `kind` with `flags` (see the module docs).
pub(crate) fn stub_effects(kind: StmtKind, flags: RiskFlags) -> Vec<DdlEffect> {
    if in_scope(kind, flags) {
        vec![DdlEffect::opaque()]
    } else {
        Vec::new()
    }
}

/// Whether Task 6 owes a statement of `kind` with `flags` an effect.
fn in_scope(kind: StmtKind, flags: RiskFlags) -> bool {
    flags.contains(RiskFlags::EXEC_COMMENT)
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
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN_SCOPE: [StmtKind; 12] = [
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
    ];

    #[test]
    fn every_scoped_kind_is_one_opaque_effect() {
        for kind in IN_SCOPE {
            assert_eq!(
                stub_effects(kind, RiskFlags::empty()),
                vec![DdlEffect::opaque()],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn other_kinds_have_none_unless_an_executable_comment() {
        let others = [
            StmtKind::Select,
            StmtKind::Values,
            StmtKind::Table,
            StmtKind::Insert,
            StmtKind::Update,
            StmtKind::Delete,
            StmtKind::Merge,
            StmtKind::Copy,
            StmtKind::Explain { analyze: true },
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
            StmtKind::Other,
        ];
        for kind in others {
            assert!(
                stub_effects(kind, RiskFlags::empty()).is_empty(),
                "{kind:?}"
            );
            assert_eq!(
                stub_effects(kind, RiskFlags::EXEC_COMMENT),
                vec![DdlEffect::opaque()],
                "{kind:?} with an executable comment"
            );
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

    #[test]
    fn opaque_names_nothing() {
        let e = DdlEffect::opaque();
        assert!(e.opaque);
        assert_eq!(e.name, None);
        assert_eq!(e.on, None);
        assert!(e.referenced.is_empty());
    }
}
