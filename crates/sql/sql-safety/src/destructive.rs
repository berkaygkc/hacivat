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

// ## m3b-t10-interim (fix round 1, I2; named exception in Task 4's file): the constant
// predicate rule of SAFE-07 (`alg/13` :529, UNSAFE-06), shared by the backends. Each backend
// maps its own tree to a [`Predicate`]; this module never sees a parser type.

/// A `WHERE` predicate as SAFE-07 reads it: literals and the logic between them; anything that
/// depends on the row is [`Predicate::Row`].
#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    /// `TRUE` / `FALSE`.
    Bool(bool),
    /// `NULL`.
    Null,
    /// A numeric literal (MySQL and SQLite read a non-zero number as true).
    Number(f64),
    /// A string literal, exactly as written between its quotes.
    Text(String),
    Not(Box<Predicate>),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    /// `a <op> b`.
    Cmp(CmpOp, Box<Predicate>, Box<Predicate>),
    /// `x IS [NOT] NULL`.
    IsNull {
        expr: Box<Predicate>,
        negated: bool,
    },
    /// Row-independent, but its value is not modelled (`1 + 1 = 2`, `@v`, a bound parameter).
    Const,
    /// Depends on the row (a column, a call, a subquery): not constant.
    Row,
}

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    /// MySQL's NULL-safe `<=>` (and SQL's `IS NOT DISTINCT FROM`).
    NullSafeEq,
}

/// A three-valued truth, plus the two "not decided here" outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Null,
    /// Constant, value not modelled.
    Const,
    Row,
}

fn truth(p: &Predicate) -> Truth {
    match p {
        Predicate::Bool(true) => Truth::True,
        Predicate::Bool(false) => Truth::False,
        Predicate::Null => Truth::Null,
        Predicate::Number(n) if n.is_nan() => Truth::Const,
        Predicate::Number(n) => {
            if *n == 0.0 {
                Truth::False
            } else {
                Truth::True
            }
        }
        // A string's truth is its numeric prefix (MySQL) or an error (PG): not modelled.
        Predicate::Text(_) | Predicate::Const => Truth::Const,
        Predicate::Row => Truth::Row,
        Predicate::Not(x) => match truth(x) {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            t => t,
        },
        Predicate::And(a, b) => {
            let (a, b) = (truth(a), truth(b));
            if a == Truth::False || b == Truth::False {
                Truth::False
            } else if a == Truth::Row || b == Truth::Row {
                Truth::Row
            } else if a == Truth::Const || b == Truth::Const {
                Truth::Const
            } else if a == Truth::Null || b == Truth::Null {
                Truth::Null
            } else {
                Truth::True
            }
        }
        Predicate::Or(a, b) => {
            let (a, b) = (truth(a), truth(b));
            if a == Truth::True || b == Truth::True {
                Truth::True
            } else if a == Truth::Row || b == Truth::Row {
                Truth::Row
            } else if a == Truth::Const || b == Truth::Const {
                Truth::Const
            } else if a == Truth::Null || b == Truth::Null {
                Truth::Null
            } else {
                Truth::False
            }
        }
        Predicate::IsNull { expr, negated } => {
            let is_null = match &**expr {
                Predicate::Null => Some(true),
                Predicate::Bool(_) | Predicate::Number(_) | Predicate::Text(_) => Some(false),
                other => match truth(other) {
                    Truth::Row => return Truth::Row,
                    Truth::Null => Some(true),
                    Truth::True | Truth::False => Some(false),
                    Truth::Const => None,
                },
            };
            match is_null {
                Some(v) if v != *negated => Truth::True,
                Some(_) => Truth::False,
                None => Truth::Const,
            }
        }
        Predicate::Cmp(op, a, b) => compare(*op, a, b),
    }
}

fn compare(op: CmpOp, a: &Predicate, b: &Predicate) -> Truth {
    use std::cmp::Ordering;
    if truth(a) == Truth::Row || truth(b) == Truth::Row {
        return Truth::Row;
    }
    let null = |p: &Predicate| matches!(p, Predicate::Null);
    if op == CmpOp::NullSafeEq {
        return match (null(a), null(b)) {
            (true, true) => Truth::True,
            (true, false) | (false, true) if is_literal(a) && is_literal(b) => Truth::False,
            _ => match ordering(a, b) {
                Some(Ordering::Equal) => Truth::True,
                Some(_) => Truth::False,
                None => Truth::Const,
            },
        };
    }
    if null(a) || null(b) {
        return Truth::Null;
    }
    let Some(ord) = ordering(a, b) else {
        return Truth::Const;
    };
    let holds = match op {
        CmpOp::Eq | CmpOp::NullSafeEq => ord == Ordering::Equal,
        CmpOp::NotEq => ord != Ordering::Equal,
        CmpOp::Lt => ord == Ordering::Less,
        CmpOp::LtEq => ord != Ordering::Greater,
        CmpOp::Gt => ord == Ordering::Greater,
        CmpOp::GtEq => ord != Ordering::Less,
    };
    if holds { Truth::True } else { Truth::False }
}

fn is_literal(p: &Predicate) -> bool {
    matches!(
        p,
        Predicate::Bool(_) | Predicate::Null | Predicate::Number(_) | Predicate::Text(_)
    )
}

/// The order of two literals of one kind; `None` when it depends on the server (a collation,
/// a cross-type conversion). Strings compare only when their bytes are equal (equal under
/// every collation); any other string pair is left to the server.
fn ordering(a: &Predicate, b: &Predicate) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Predicate::Number(x), Predicate::Number(y)) => x.partial_cmp(y),
        (Predicate::Bool(x), Predicate::Bool(y)) => Some(x.cmp(y)),
        (Predicate::Text(x), Predicate::Text(y)) if x == y => Some(std::cmp::Ordering::Equal),
        _ => None,
    }
}

/// Whether a `WHERE` with predicate `p` keeps every row (SAFE-07 `AlwaysTrue`): it is true
/// whatever the row (`1`, `TRUE`, `1 = 1`, `'a' = 'a'`, `NULL IS NULL`, `NOT FALSE`,
/// `x OR TRUE`), or it does not depend on the row and its value is not modelled (`1 + 1 = 2`):
/// the safe direction. A predicate that is FALSE or NULL for every row keeps none, and one
/// that depends on the row is not constant: both are `false`.
pub fn always_true(p: &Predicate) -> bool {
    matches!(truth(p), Truth::True | Truth::Const)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_predicates() {
        use Predicate as P;
        let b = Box::new;
        let num = P::Number;
        let text = |s: &str| P::Text(s.to_string());
        let is_null = |e: P, negated| P::IsNull {
            expr: b(e),
            negated,
        };
        assert!(always_true(&P::Bool(true)));
        assert!(always_true(&num(1.0)));
        assert!(!always_true(&num(0.0)));
        assert!(!always_true(&P::Null));
        assert!(always_true(&is_null(P::Null, false)));
        assert!(!always_true(&is_null(P::Null, true)));
        assert!(always_true(&is_null(num(1.0), true)));
        assert!(always_true(&P::Not(b(P::Bool(false)))));
        assert!(always_true(&P::Cmp(CmpOp::Eq, b(num(1.0)), b(num(1.0)))));
        assert!(!always_true(&P::Cmp(CmpOp::Eq, b(num(1.0)), b(num(0.0)))));
        assert!(always_true(&P::Cmp(CmpOp::Eq, b(text("a")), b(text("a")))));
        // A collation decides 'a' = 'A': constant, left to the server, so treated as true.
        assert!(always_true(&P::Cmp(CmpOp::Eq, b(text("a")), b(text("A")))));
        assert!(!always_true(&P::Cmp(CmpOp::Eq, b(P::Null), b(P::Null))));
        assert!(always_true(&P::Cmp(
            CmpOp::NullSafeEq,
            b(P::Null),
            b(P::Null)
        )));
        assert!(always_true(&P::Or(b(P::Row), b(P::Bool(true)))));
        assert!(!always_true(&P::Or(b(P::Row), b(P::Bool(false)))));
        assert!(!always_true(&P::And(b(P::Row), b(P::Bool(true)))));
        assert!(!always_true(&P::And(b(P::Const), b(P::Bool(false)))));
        assert!(always_true(&P::Const));
        assert!(!always_true(&P::Row));
        assert!(!always_true(&P::Cmp(CmpOp::Gt, b(P::Row), b(num(1.0)))));
        assert!(always_true(&P::Cmp(CmpOp::Lt, b(num(1.0)), b(num(2.0)))));
    }

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
