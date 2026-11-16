//! What [`crate::classify`] answers: one [`StmtReport`] per statement and the request's
//! [`SafetyReport`] (`alg/13`; Task 4 fills the reports, Tasks 9, 10 and 14 produce them).

use smallvec::SmallVec;
use sql_dialect::DialectId;
use sql_ident::QualifiedName;
use sql_split::Fingerprint;
use sql_text::TextRange;

use crate::destructive::DestructiveReason;
use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of};
use crate::fingerprint::classified_hash;
use crate::reason::Reason;

/// The kind of object a statement touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Table,
    View,
    Function,
    Sequence,
    Schema,
    Database,
    Role,
    Other,
}

/// A named object.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub name: QualifiedName,
    pub kind: ObjectKind,
}

/// The objects a statement reads, writes and calls.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TouchedObjects {
    pub reads: Vec<ObjectRef>,
    pub writes: Vec<ObjectRef>,
    pub calls: Vec<ObjectRef>,
}

/// The statement's kind, as its backend recognised it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StmtKind {
    Select,
    Values,
    Table,
    Insert,
    Update,
    Delete,
    Merge,
    Copy,
    Call,
    Do,
    Explain { analyze: bool },
    CreateTable,
    CreateOther,
    Alter,
    Drop,
    Truncate,
    Grant,
    Revoke,
    Set,
    Show,
    Begin,
    Commit,
    Rollback,
    Savepoint,
    Lock,
    Prepare,
    Execute,
    Listen,
    Notify,
    Use,
    Pragma,
    Other,
}

/// Whether the effects are exactly the statement's or a conservative superset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    Exact,
    Conservative,
}

/// One statement's classification.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StmtReport {
    pub range: TextRange,
    pub kind: StmtKind,
    pub effects: EffectSet,
    pub flags: RiskFlags,
    pub class: CoarseClass,
    pub touched: TouchedObjects,
    pub reasons: Vec<Reason>,
    pub destructive: Vec<DestructiveReason>,
    pub confidence: Confidence,
}

/// A backend that took part (`"libpg_query"`/`"18.1.0"`, `"sqlparser"`/`"0.64.0"`,
/// `"lexer-prescan"`/`"1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendVersion {
    pub name: &'static str,
    pub version: &'static str,
}

/// The classification of one request.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SafetyReport {
    pub items: Vec<StmtReport>,
    pub request_class: CoarseClass,
    pub request_effects: EffectSet,
    pub flags: RiskFlags,
    pub fingerprint: Fingerprint,
    pub backends: SmallVec<[BackendVersion; 3]>,
    /// Set when the request was decided before (or instead of) analysis: the reasons it is
    /// `Unknown`.
    pub pre_decision: Option<Vec<Reason>>,
    /// The length of the exact bytes classified (saturating at `u32::MAX`; P-M3b-2).
    pub classified_len: u32,
    /// XXH3-64 of the exact bytes classified ([`classified_hash`]; P-M3b-2).
    pub classified_hash: u64,
    // ## m3b-t04-safety: the dialect the request was classified for. `decide` needs it to
    // check `SET` targets and `SET_VAR` hints against the dialect's harmless list
    // (`set_allowed`, P-M3b-6); `None` (a pre-decided report, or a producer that did not say)
    // allowlists nothing. Task 14's `classify` sets it from `ClassifyCtx.dialect`.
    pub dialect: Option<DialectId>,
}

impl SafetyReport {
    /// An `Unknown` report with no items, decided for `reasons` before any analysis of
    /// `sql`; `classified_len`/`classified_hash` describe `sql`.
    pub fn pre_decided(sql: &str, fingerprint: Fingerprint, reasons: Vec<Reason>) -> Self {
        SafetyReport {
            items: Vec::new(),
            request_class: CoarseClass::Unknown,
            request_effects: EffectSet::UNKNOWN,
            flags: RiskFlags::empty(),
            fingerprint,
            backends: SmallVec::new(),
            pre_decision: Some(reasons),
            classified_len: u32::try_from(sql.len()).unwrap_or(u32::MAX),
            classified_hash: classified_hash(sql),
            dialect: None,
        }
    }

    /// Whether this report classified exactly `text` (length and XXH3-64): the gate denies a
    /// unit whose report does not match its final text (`exec-gate-stale-classification`,
    /// P-M3b-2).
    pub fn matches_text(&self, text: &str) -> bool {
        u32::try_from(text.len()).unwrap_or(u32::MAX) == self.classified_len
            && classified_hash(text) == self.classified_hash
    }
}

/// The request-level class, effects and flags of `items` (`alg/13` SQL-SAFE-02 step 8): the
/// union of the effects and flags, and the most critical of every statement's class and of
/// `class_of` over its effects (a statement whose `class` field is lower than its effects say
/// does not lower the request). No statement is `(Unknown, UNKNOWN, ∅)`: nothing was proved.
///
/// Public (the plan names it `pub(crate)`) so report builders outside the orchestrator (the
/// gate's tests, P-M3b-16) merge the way `classify` does.
pub fn merge(items: &[StmtReport]) -> (CoarseClass, EffectSet, RiskFlags) {
    if items.is_empty() {
        return (CoarseClass::Unknown, EffectSet::UNKNOWN, RiskFlags::empty());
    }
    let mut class = CoarseClass::Read;
    let mut effects = EffectSet::empty();
    let mut flags = RiskFlags::empty();
    for s in items {
        class = class.max(s.class).max(class_of(s.effects));
        effects |= s.effects;
        flags |= s.flags;
    }
    (class, effects, flags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::CoarseClass as C;

    fn stmt(effects: EffectSet, class: CoarseClass, flags: RiskFlags) -> StmtReport {
        StmtReport {
            range: TextRange::default(),
            kind: StmtKind::Other,
            effects,
            flags,
            class,
            touched: TouchedObjects::default(),
            reasons: Vec::new(),
            destructive: Vec::new(),
            confidence: Confidence::Exact,
        }
    }

    #[test]
    fn merge_unions_and_takes_the_most_critical_class() {
        let a = stmt(EffectSet::READ, C::Read, RiskFlags::VOLATILE_FN);
        let b = stmt(EffectSet::WRITE_DATA, C::Write, RiskFlags::EXEC_COMMENT);
        assert_eq!(
            merge(&[a.clone(), b]),
            (
                C::Write,
                EffectSet::READ | EffectSet::WRITE_DATA,
                RiskFlags::VOLATILE_FN | RiskFlags::EXEC_COMMENT
            )
        );
        // A class field lower than the effects does not lower the request.
        let lying = stmt(
            EffectSet::READ | EffectSet::UNKNOWN,
            C::Read,
            RiskFlags::empty(),
        );
        assert_eq!(merge(&[a.clone(), lying]).0, C::Unknown);
        // A class field higher than the effects raises it.
        let admin = stmt(EffectSet::READ, C::Admin, RiskFlags::empty());
        assert_eq!(merge(&[a, admin]).0, C::Admin);
        // An empty statement effect set is not a read.
        assert_eq!(
            merge(&[stmt(EffectSet::empty(), C::Read, RiskFlags::empty())]).0,
            C::Unknown
        );
    }

    #[test]
    fn merge_of_nothing_is_unknown() {
        assert_eq!(
            merge(&[]),
            (C::Unknown, EffectSet::UNKNOWN, RiskFlags::empty())
        );
    }

    #[test]
    fn pre_decided_reports_name_no_dialect() {
        let ctx = sql_dialect::DialectCtx::new(DialectId::Postgres, None);
        let r = SafetyReport::pre_decided("x", sql_split::hv_norm1("x", &ctx), Vec::new());
        assert_eq!(r.dialect, None);
        assert_eq!(r.request_class, C::Unknown);
    }
}
