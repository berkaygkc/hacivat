//! Destructive patterns (`alg/13` SAFE-07; Task 4): what a confirmation names even when the
//! effects alone would not ask (`DELETE` without `WHERE`, `DROP … CASCADE`, …). The backends
//! (Tasks 9, 10) detect them; [`DestructiveKind::is_unsafe_dml`] and
//! [`DestructiveKind::is_destructive_ddl`] are how the gate class and the policy read them.

use sql_text::TextRange;

use crate::report::ObjectRef;

/// A destructive pattern.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DestructiveKind {
    NoWhere,
    AlwaysTrue,
    JoinWithoutOn,
    Drop { cascade: bool },
    Truncate,
    DropColumn,
    DropConstraint,
    AlterType,
    MergeDelete,
    UpdateLimitNoOrder,
}

impl DestructiveKind {
    /// Every kind (both `Drop` values), in declaration order.
    pub const ALL: [DestructiveKind; 11] = [
        DestructiveKind::NoWhere,
        DestructiveKind::AlwaysTrue,
        DestructiveKind::JoinWithoutOn,
        DestructiveKind::Drop { cascade: false },
        DestructiveKind::Drop { cascade: true },
        DestructiveKind::Truncate,
        DestructiveKind::DropColumn,
        DestructiveKind::DropConstraint,
        DestructiveKind::AlterType,
        DestructiveKind::MergeDelete,
        DestructiveKind::UpdateLimitNoOrder,
    ];

    /// A data change that may touch every row: `GateClass::DmlUnsafe` (`alg/20` :266-278).
    pub fn is_unsafe_dml(self) -> bool {
        match self {
            DestructiveKind::NoWhere
            | DestructiveKind::AlwaysTrue
            | DestructiveKind::JoinWithoutOn
            | DestructiveKind::UpdateLimitNoOrder
            | DestructiveKind::MergeDelete => true,
            DestructiveKind::Drop { .. }
            | DestructiveKind::Truncate
            | DestructiveKind::DropColumn
            | DestructiveKind::DropConstraint
            | DestructiveKind::AlterType => false,
        }
    }

    /// A schema change that destroys data or objects: `GateClass::DdlDestructive`.
    pub fn is_destructive_ddl(self) -> bool {
        !self.is_unsafe_dml()
    }

    /// An unbounded `UPDATE`/`DELETE` that an AI agent may never run directly (`alg/13`
    /// SAFE-04 step 5, SAFE-07 step 5).
    pub fn denied_for_ai(self) -> bool {
        matches!(
            self,
            DestructiveKind::NoWhere | DestructiveKind::AlwaysTrue | DestructiveKind::JoinWithoutOn
        )
    }

    /// The Fluent message id, a literal per arm (P24): `safety-destructive-<kebab>`. Each
    /// message takes `$table`, the target's name as written (Fluent isolates it, FSI/PDI).
    pub fn fluent_id(self) -> &'static str {
        match self {
            DestructiveKind::NoWhere => "safety-destructive-no-where",
            DestructiveKind::AlwaysTrue => "safety-destructive-always-true",
            DestructiveKind::JoinWithoutOn => "safety-destructive-join-without-on",
            DestructiveKind::Drop { cascade: false } => "safety-destructive-drop",
            DestructiveKind::Drop { cascade: true } => "safety-destructive-drop-cascade",
            DestructiveKind::Truncate => "safety-destructive-truncate",
            DestructiveKind::DropColumn => "safety-destructive-drop-column",
            DestructiveKind::DropConstraint => "safety-destructive-drop-constraint",
            DestructiveKind::AlterType => "safety-destructive-alter-type",
            DestructiveKind::MergeDelete => "safety-destructive-merge-delete",
            DestructiveKind::UpdateLimitNoOrder => "safety-destructive-update-limit-no-order",
        }
    }
}

/// One destructive pattern in a statement, with the range it covers and its target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DestructiveReason {
    pub kind: DestructiveKind,
    pub range: TextRange,
    pub target: Option<ObjectRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compile-time proof that `ALL` lists every variant: adding one breaks this match.
    fn index(k: DestructiveKind) -> usize {
        match k {
            DestructiveKind::NoWhere => 0,
            DestructiveKind::AlwaysTrue => 1,
            DestructiveKind::JoinWithoutOn => 2,
            DestructiveKind::Drop { cascade: false } => 3,
            DestructiveKind::Drop { cascade: true } => 4,
            DestructiveKind::Truncate => 5,
            DestructiveKind::DropColumn => 6,
            DestructiveKind::DropConstraint => 7,
            DestructiveKind::AlterType => 8,
            DestructiveKind::MergeDelete => 9,
            DestructiveKind::UpdateLimitNoOrder => 10,
        }
    }

    #[test]
    fn all_lists_every_kind_once_in_order() {
        for (i, k) in DestructiveKind::ALL.iter().enumerate() {
            assert_eq!(index(*k), i, "{k:?}");
        }
    }

    #[test]
    fn ai_denials_are_unsafe_dml() {
        for k in DestructiveKind::ALL {
            if k.denied_for_ai() {
                assert!(k.is_unsafe_dml(), "{k:?}");
            }
            assert_ne!(k.is_unsafe_dml(), k.is_destructive_ddl(), "{k:?}");
        }
    }
}
