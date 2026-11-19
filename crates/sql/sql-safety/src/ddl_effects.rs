// ## m4-t01c-core (seam) / ## m4-t06-ddl-effects (bodies)
//! The catalog effects of one statement (`StmtReport.ddl`, the input of HV-INTRO-12's
//! `plan_refresh`; M4 Task 6).
//!
//! Each backend reads, from its own parse of a statement, the objects it creates, alters,
//! renames, moves, comments on, grants on or drops (`backend::ddl_effects::pg` from
//! libpg_query's tree, `backend::ddl_effects::interim` from `sqlparser`'s for MySQL, MariaDB
//! and SQLite; parser types stay inside `backend/`, R-M3-54). Names are exactly as written:
//! the parts' text and whether each was quoted, never folded (the consumer folds with
//! `sql_ident::fold`).
//! A name the PostgreSQL text does not show as a plain name (`U&"…"`, a comment between its
//! parts) is the server's reading of it, every part quoted: the same identity, never folded
//! twice.
//!
//! [`finish`] then decides, from the statement's final kind, effects and flags (all of which
//! `classify` compares and reports), whether an **opaque** effect leads the list: the statement
//! may change objects it does not name for certain, so the consumer refreshes the session's
//! current namespace too (refresh more, never less). It is added when:
//! - the statement hides what it runs: kind `Do`, `Call`, `Execute`, `Prepare`, `Other` (a
//!   statement the backend did not name: MySQL `CREATE EVENT`, `ALTER USER`, `RENAME USER`,
//!   SQLite `ATTACH`/`DETACH`, …), `Explain { analyze: true }`, or PostgreSQL `DROP OWNED`
//!   (`DropOwned`, M3c P2-07: it names a role, not the objects it drops); flags `EXEC_COMMENT`,
//!   `DYNAMIC_SQL` or `EXPLAIN_ANALYZE`; effects `UNKNOWN`, `ROUTINE_CALL` or `PREPARE`, or none
//!   (nothing proved; this covers every statement the backend could not read, and a later
//!   statement of a script after one that may change what names mean);
//! - the backend read no named effect from a statement in the T1c scope (a DDL-class kind
//!   `CreateTable`, `CreateOther`, `Alter`, `Drop`, `Grant`, `Revoke`, or effects `DDL`,
//!   `DDL_TEMP`, `PRIVILEGE`, `ADMIN`), e.g. `REASSIGN OWNED`, `ALTER DEFAULT PRIVILEGES`,
//!   `CREATE CAST`. **Named exception (U-3):** `TRUNCATE` (plain or `CASCADE`) changes no
//!   catalog object and has no effect.
//!
//! Every other statement has none. The field never changes a verdict: it is computed after the
//! statement's report, from fields `classify` already compares, and nothing in `decide`,
//! `gate_class`, the policy or the exec gate reads it. `classify` recomputes it ([`refresh`])
//! after each step that may change the kind, effects or flags (the union of two readings, the
//! context-taint pass), keeping the named effects the backends read.
//!
//! Not detected (ADR 0005 `m4-t06`): DDL that a function called by a plain `SELECT f()` runs,
//! when the classifier reads `f` as a read (an unknown `f` is `UNKNOWN`, so opaque). Only
//! background validation (M4 Task 22) sees it.

use crate::effects::{EffectSet, RiskFlags};
use crate::report::{DdlEffect, StmtKind, StmtReport};

/// The effects that may change the catalog (the T1c stub's scope).
const CATALOG_EFFECTS: EffectSet = EffectSet::DDL
    .union(EffectSet::DDL_TEMP)
    .union(EffectSet::PRIVILEGE)
    .union(EffectSet::ADMIN)
    .union(EffectSet::PREPARE)
    .union(EffectSet::ROUTINE_CALL)
    .union(EffectSet::UNKNOWN);

/// The effects that hide what a statement runs.
const HIDDEN_EFFECTS: EffectSet = EffectSet::UNKNOWN
    .union(EffectSet::ROUTINE_CALL)
    .union(EffectSet::PREPARE);

/// The flags that hide what a statement runs.
const CATALOG_FLAGS: RiskFlags = RiskFlags::EXEC_COMMENT
    .union(RiskFlags::DYNAMIC_SQL)
    .union(RiskFlags::EXPLAIN_ANALYZE);

/// The effects of a statement of `kind` with `effects` and `flags` whose backend read the
/// effects `named` (the module docs): the named effects, each once, in order, after one
/// opaque effect when the statement may change what it does not name.
pub(crate) fn finish(
    kind: StmtKind,
    effects: EffectSet,
    flags: RiskFlags,
    named: Vec<DdlEffect>,
) -> Vec<DdlEffect> {
    let mut opaque = hides_what_it_runs(kind, effects, flags);
    let mut out: Vec<DdlEffect> = Vec::with_capacity(named.len() + 1);
    for e in named {
        if e.opaque {
            opaque = true;
        } else if !out.contains(&e) {
            out.push(e);
        }
    }
    if out.is_empty() && kind != StmtKind::Truncate && in_scope(kind, effects, flags) {
        opaque = true;
    }
    if opaque {
        out.insert(0, DdlEffect::opaque());
    }
    out
}

/// Sets `item.ddl` from its final kind, effects and flags, keeping the effects already named.
pub(crate) fn refresh(item: &mut StmtReport) {
    let named = std::mem::take(&mut item.ddl);
    item.ddl = finish(item.kind, item.effects, item.flags, named);
}

/// Whether the statement may run catalog changes its text does not name.
fn hides_what_it_runs(kind: StmtKind, effects: EffectSet, flags: RiskFlags) -> bool {
    effects.is_empty()
        || effects.intersects(HIDDEN_EFFECTS)
        || flags.intersects(CATALOG_FLAGS)
        || matches!(
            kind,
            StmtKind::Do
                | StmtKind::Call
                | StmtKind::Execute
                | StmtKind::Prepare
                | StmtKind::DropOwned
                | StmtKind::Other
                | StmtKind::Explain { analyze: true }
        )
}

/// Whether Task 6 owes the statement an effect (the T1c stub's scope).
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
    use sql_ident::{Ident, QualifiedName};

    use super::*;
    use crate::report::{DdlAction, DdlKind};

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

    fn table(name: &str) -> DdlEffect {
        DdlEffect {
            action: DdlAction::Create,
            kind: DdlKind::Table,
            name: Some(QualifiedName {
                parts: [Ident {
                    text: name.into(),
                    quoted: false,
                }]
                .into_iter()
                .collect(),
            }),
            on: None,
            referenced: Vec::new(),
            temp: false,
            concurrently: false,
            opaque: false,
        }
    }

    #[test]
    fn a_scoped_kind_with_no_named_effect_is_opaque_except_truncate() {
        for kind in IN_SCOPE {
            let want = if kind == StmtKind::Truncate {
                Vec::new()
            } else {
                vec![DdlEffect::opaque()]
            };
            assert_eq!(
                finish(kind, EffectSet::READ, RiskFlags::empty(), Vec::new()),
                want,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn named_effects_stand_alone_only_when_nothing_is_hidden() {
        let named = vec![table("t"), table("t"), table("u")];
        assert_eq!(
            finish(
                StmtKind::CreateTable,
                EffectSet::DDL,
                RiskFlags::empty(),
                named.clone()
            ),
            [table("t"), table("u")]
        );
        for (kind, effects, flags) in [
            (
                StmtKind::CreateTable,
                EffectSet::DDL | EffectSet::UNKNOWN,
                RiskFlags::empty(),
            ),
            (
                StmtKind::CreateTable,
                EffectSet::DDL,
                RiskFlags::EXEC_COMMENT,
            ),
            (
                StmtKind::CreateTable,
                EffectSet::empty(),
                RiskFlags::empty(),
            ),
            (
                StmtKind::Explain { analyze: true },
                EffectSet::DDL,
                RiskFlags::empty(),
            ),
            (StmtKind::Other, EffectSet::DDL, RiskFlags::empty()),
        ] {
            assert_eq!(
                finish(kind, effects, flags, named.clone()),
                [DdlEffect::opaque(), table("t"), table("u")],
                "{kind:?} {effects:?} {flags:?}"
            );
        }
    }

    #[test]
    fn finish_is_idempotent() {
        let once = finish(
            StmtKind::CreateTable,
            EffectSet::DDL | EffectSet::UNKNOWN,
            RiskFlags::empty(),
            vec![table("t")],
        );
        let twice = finish(
            StmtKind::CreateTable,
            EffectSet::DDL | EffectSet::UNKNOWN,
            RiskFlags::empty(),
            once.clone(),
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn plain_kinds_have_none_unless_effects_or_flags_say_otherwise() {
        for kind in PLAIN {
            assert!(
                finish(
                    kind,
                    EffectSet::READ | EffectSet::WRITE_DATA,
                    RiskFlags::empty(),
                    Vec::new()
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
                    finish(kind, EffectSet::READ, flags, Vec::new()),
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
                    finish(kind, effects, RiskFlags::empty(), Vec::new()),
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
    fn both_backends_fill_the_effects_through_classify() {
        use sql_dialect::DialectId;
        let none: Vec<DdlEffect> = Vec::new();
        let opaque = vec![DdlEffect::opaque()];
        for d in [DialectId::Postgres, DialectId::MySql, DialectId::Sqlite] {
            let got = classified(d, "SELECT 1; CREATE TABLE t (a int)");
            assert_eq!(got, [none.clone(), vec![table("t")]], "{d:?}");
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
    /// `Explain { analyze: true }` that change the catalog are never without an opaque effect.
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
            let got = classified(d, sql);
            assert_eq!(got.len(), 1, "{d:?}: {sql}");
            assert_eq!(got[0].first(), Some(&DdlEffect::opaque()), "{d:?}: {sql}");
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
