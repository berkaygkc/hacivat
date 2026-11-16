//! What [`crate::classify`] answers: one [`StmtReport`] per statement and the request's
//! [`SafetyReport`] (`alg/13`; Task 4 fills the reports, Tasks 9, 10 and 14 produce them).

use smallvec::SmallVec;
use sql_ident::QualifiedName;
use sql_split::Fingerprint;
use sql_text::TextRange;

use crate::destructive::DestructiveReason;
use crate::effects::{CoarseClass, EffectSet, RiskFlags};
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
