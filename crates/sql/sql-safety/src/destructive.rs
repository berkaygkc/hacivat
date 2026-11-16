//! Destructive patterns (`alg/13` SAFE-04; Task 4): what a confirmation names even when the
//! effects alone would not ask (`DELETE` without `WHERE`, `DROP … CASCADE`, …).

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

/// One destructive pattern in a statement, with the range it covers and its target.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DestructiveReason {
    pub kind: DestructiveKind,
    pub range: TextRange,
    pub target: Option<ObjectRef>,
}
