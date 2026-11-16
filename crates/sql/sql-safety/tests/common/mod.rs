//! Report builders shared by Task 4's tests (`policy`, `gate_class`, `reasons`). Each test
//! binary uses part of them.
#![allow(dead_code, reason = "each test binary uses a subset of the builders")]

use std::sync::Arc;

use smallvec::SmallVec;
use sql_dialect::{DialectCtx, DialectId};
use sql_ident::{Ident, QualifiedName};
use sql_safety::effects::class_of;
use sql_safety::{
    Confidence, DestructiveKind, DestructiveReason, EffectSet, ObjectKind, ObjectRef, Reason,
    ReasonCode, RiskFlags, SafetyReport, SetTarget, StmtKind, StmtReport, TouchedObjects,
    classified_hash,
};
use sql_text::{TextRange, TextSize};

/// Every named effect flag.
pub fn flags() -> Vec<EffectSet> {
    EffectSet::all().iter().collect()
}

/// Every subset of the named flags with at most `max_bits` bits set (the empty set included).
pub fn all_effect_combinations(max_bits: u32) -> impl Iterator<Item = EffectSet> {
    let flags = flags();
    let n = flags.len();
    (0u64..(1u64 << n))
        .filter(move |m| m.count_ones() <= max_bits)
        .map(move |m| {
            let mut e = EffectSet::empty();
            for (i, f) in flags.iter().enumerate() {
                if m & (1 << i) != 0 {
                    e |= *f;
                }
            }
            e
        })
}

/// Every destructive kind (both `Drop` values).
pub fn destructive_kinds() -> Vec<DestructiveKind> {
    DestructiveKind::ALL.to_vec()
}

/// No destructive reason, then each kind alone.
pub fn destructive_variants() -> Vec<Vec<DestructiveKind>> {
    let mut v = vec![Vec::new()];
    v.extend(destructive_kinds().into_iter().map(|k| vec![k]));
    v
}

pub fn range() -> TextRange {
    TextRange::new(TextSize::from(0), TextSize::from(1))
}

pub fn name(text: &str) -> QualifiedName {
    QualifiedName {
        parts: SmallVec::from_iter([Ident {
            text: Arc::from(text),
            quoted: false,
        }]),
    }
}

pub fn target(text: &str) -> ObjectRef {
    ObjectRef {
        name: name(text),
        kind: ObjectKind::Table,
    }
}

/// One statement of `kind` with `effects`, `flags`, destructive `kinds` (target `t`) and
/// `reasons`; its class is `class_of(effects)` (what the backends write).
pub fn stmt_full(
    kind: StmtKind,
    effects: EffectSet,
    flags: RiskFlags,
    kinds: &[DestructiveKind],
    reasons: Vec<Reason>,
) -> StmtReport {
    StmtReport {
        range: range(),
        kind,
        effects,
        flags,
        class: class_of(effects),
        touched: TouchedObjects::default(),
        reasons,
        destructive: kinds
            .iter()
            .map(|k| DestructiveReason {
                kind: *k,
                range: range(),
                target: Some(target("t")),
            })
            .collect(),
        confidence: Confidence::Exact,
        set_targets: Vec::new(),
    }
}

/// A plausible harmless value for `name` (what a user would write).
pub fn value_for(name: &str) -> &'static str {
    match name {
        "search_path" => "public",
        "statement_timeout" | "STATEMENT_TIMEOUT" => "5000",
        "work_mem" => "64MB",
        _ => "x",
    }
}

/// A statement with `effects` and destructive `kinds`, kind `Other`.
pub fn report(effects: EffectSet, kinds: &[DestructiveKind]) -> StmtReport {
    stmt_full(
        StmtKind::Other,
        effects,
        RiskFlags::empty(),
        kinds,
        Vec::new(),
    )
}

pub fn reason(code: ReasonCode, arg: Option<&str>) -> Reason {
    Reason {
        code,
        range: Some(range()),
        arg: arg.map(Arc::from),
    }
}

/// A request of `items` on dialect `d`, merged the way the orchestrator merges.
pub fn request_in(d: Option<DialectId>, items: Vec<StmtReport>) -> SafetyReport {
    let (request_class, request_effects, flags) = sql_safety::report::merge(&items);
    let ctx = DialectCtx::new(d.unwrap_or(DialectId::Postgres), None);
    SafetyReport {
        items,
        request_class,
        request_effects,
        flags,
        fingerprint: sql_split::hv_norm1("x", &ctx),
        backends: SmallVec::new(),
        pre_decision: None,
        classified_len: 1,
        classified_hash: classified_hash("x"),
        dialect: d,
    }
}

/// A PostgreSQL request of `items`.
pub fn request(items: Vec<StmtReport>) -> SafetyReport {
    request_in(Some(DialectId::Postgres), items)
}

/// A PostgreSQL request of one statement with `effects` and destructive `kinds`.
pub fn single(effects: EffectSet, kinds: &[DestructiveKind]) -> SafetyReport {
    request(vec![report(effects, kinds)])
}

/// `UNKNOWN` effects (`READ | UNKNOWN`, what a backend writes for an unresolved function).
pub fn unknown() -> SafetyReport {
    request(vec![stmt_full(
        StmtKind::Select,
        EffectSet::READ | EffectSet::UNKNOWN,
        RiskFlags::empty(),
        &[],
        vec![reason(ReasonCode::FnUnknown, Some("my_fn"))],
    )])
}

/// `EXPLAIN ANALYZE SELECT …`: the inner read's effects plus the flag.
pub fn explain_analyze_read() -> SafetyReport {
    request(vec![stmt_full(
        StmtKind::Explain { analyze: true },
        EffectSet::READ,
        RiskFlags::EXPLAIN_ANALYZE,
        &[],
        vec![reason(ReasonCode::ExplainAnalyze, None)],
    )])
}

/// `SET <names…>` on `d` with [`value_for`] values, one `SessionState` reason and one
/// `SetTarget` per target (the producer contract).
pub fn set_of(d: Option<DialectId>, names: &[&str]) -> SafetyReport {
    let targets: Vec<(&str, Option<&str>)> =
        names.iter().map(|n| (*n, Some(value_for(n)))).collect();
    set_with(d, &targets)
}

/// `SET` of `(name, value)` targets on `d`.
pub fn set_with(d: Option<DialectId>, targets: &[(&str, Option<&str>)]) -> SafetyReport {
    let mut s = stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE,
        RiskFlags::empty(),
        &[],
        targets
            .iter()
            .map(|(n, _)| reason(ReasonCode::SessionState, Some(n)))
            .collect(),
    );
    s.set_targets = targets
        .iter()
        .map(|(n, v)| SetTarget {
            name: Arc::from(*n),
            value: v.map(Arc::from),
        })
        .collect();
    request_in(d, vec![s])
}

// ---------------------------------------------------------------------------------------------
// Structured strategies (fix round 1, I6): inputs stay near the READ region where a
// Read-by-accident would live, drawing named bits, real reason codes, kinds, class fields and
// flags; unnamed bits appear rarely and on purpose.

use proptest::prelude::*;
use sql_safety::CoarseClass;

/// Every statement kind of the plan's `StmtKind`.
pub const ALL_KINDS: [StmtKind; 33] = [
    StmtKind::Select,
    StmtKind::Values,
    StmtKind::Table,
    StmtKind::Insert,
    StmtKind::Update,
    StmtKind::Delete,
    StmtKind::Merge,
    StmtKind::Copy,
    StmtKind::Call,
    StmtKind::Do,
    StmtKind::Explain { analyze: false },
    StmtKind::Explain { analyze: true },
    StmtKind::CreateTable,
    StmtKind::CreateOther,
    StmtKind::Alter,
    StmtKind::Drop,
    StmtKind::Truncate,
    StmtKind::Grant,
    StmtKind::Revoke,
    StmtKind::Set,
    StmtKind::Show,
    StmtKind::Begin,
    StmtKind::Commit,
    StmtKind::Rollback,
    StmtKind::Savepoint,
    StmtKind::Lock,
    StmtKind::Prepare,
    StmtKind::Execute,
    StmtKind::Listen,
    StmtKind::Notify,
    StmtKind::Use,
    StmtKind::Pragma,
    StmtKind::Other,
];

/// Kinds whose effects alone decide (no floor): half the draws, so READ-only statements are
/// common.
pub const EFFECT_KINDS: [StmtKind; 8] = [
    StmtKind::Select,
    StmtKind::Values,
    StmtKind::Table,
    StmtKind::Show,
    StmtKind::Copy,
    StmtKind::Pragma,
    StmtKind::Other,
    StmtKind::Explain { analyze: false },
];

/// READ most of the time, plus 0–2 named bits, rarely one unnamed bit.
pub fn arb_effects() -> impl Strategy<Value = EffectSet> {
    (
        prop::bool::weighted(0.7),
        prop::collection::vec(prop::sample::select(flags()), 0..3),
        prop::bool::weighted(0.03),
        19u32..31,
    )
        .prop_map(|(read, bits, raw, n)| {
            let mut e = if read {
                EffectSet::READ
            } else {
                EffectSet::empty()
            };
            for b in bits {
                e |= b;
            }
            if raw {
                e |= EffectSet::from_bits_retain(1 << n);
            }
            e
        })
}

/// No flag most of the time, else one or two named flags, rarely an unnamed bit.
pub fn arb_flags() -> impl Strategy<Value = RiskFlags> {
    let named: Vec<RiskFlags> = RiskFlags::all().iter().collect();
    prop_oneof![
        6 => Just(RiskFlags::empty()),
        3 => prop::collection::vec(prop::sample::select(named), 1..3)
            .prop_map(|v| v.into_iter().fold(RiskFlags::empty(), |a, b| a | b)),
        1 => (5u16..16).prop_map(|n| RiskFlags::from_bits_retain(1 << n)),
    ]
}

/// Reason arguments that matter: none, a name, a harmless and a forbidden `SET` target, hint
/// limits.
pub const ARGS: [Option<&str>; 6] = [
    None,
    Some("x"),
    Some("search_path"),
    Some("role"),
    Some("0"),
    Some("1000"),
];

/// No reason half the time, else one or two real codes with an argument.
pub fn arb_reasons() -> impl Strategy<Value = Vec<Reason>> {
    let one = (
        prop::sample::select(ReasonCode::ALL.to_vec()),
        prop::sample::select(ARGS.to_vec()),
    )
        .prop_map(|(c, a)| reason(c, a));
    prop_oneof![
        1 => Just(Vec::new()),
        1 => prop::collection::vec(one, 1..3),
    ]
}

pub fn arb_kind() -> impl Strategy<Value = StmtKind> {
    prop_oneof![
        1 => prop::sample::select(EFFECT_KINDS.to_vec()),
        1 => prop::sample::select(ALL_KINDS.to_vec()),
    ]
}

/// The class field: what the backends write (`class_of`) most of the time, else any class.
pub fn arb_class() -> impl Strategy<Value = Option<CoarseClass>> {
    prop_oneof![
        7 => Just(None),
        3 => prop::sample::select(CoarseClass::ALL.to_vec()).prop_map(Some),
    ]
}

pub fn arb_destructive() -> impl Strategy<Value = Vec<DestructiveKind>> {
    prop_oneof![
        4 => Just(Vec::new()),
        1 => prop::collection::vec(prop::sample::select(DestructiveKind::ALL.to_vec()), 1..3),
    ]
}

/// The parts of one statement, so a test can grow it.
#[derive(Debug, Clone)]
pub struct Parts {
    pub kind: StmtKind,
    pub effects: EffectSet,
    pub flags: RiskFlags,
    pub reasons: Vec<Reason>,
    pub destructive: Vec<DestructiveKind>,
    pub class: Option<CoarseClass>,
}

impl Parts {
    pub fn stmt(&self) -> StmtReport {
        let mut s = stmt_full(
            self.kind,
            self.effects,
            self.flags,
            &self.destructive,
            self.reasons.clone(),
        );
        if let Some(c) = self.class {
            s.class = c;
        }
        s
    }

    /// `self` with `more`'s effects, flags, reasons and destructive kinds added (same kind and
    /// class field).
    pub fn grown(&self, more: &Parts) -> Parts {
        let mut reasons = self.reasons.clone();
        reasons.extend(more.reasons.iter().cloned());
        let mut destructive = self.destructive.clone();
        destructive.extend(more.destructive.iter().copied());
        Parts {
            kind: self.kind,
            effects: self.effects | more.effects,
            flags: self.flags | more.flags,
            reasons,
            destructive,
            class: self.class,
        }
    }
}

pub fn arb_parts() -> impl Strategy<Value = Parts> {
    (
        arb_kind(),
        arb_effects(),
        arb_flags(),
        arb_reasons(),
        arb_destructive(),
        arb_class(),
    )
        .prop_map(
            |(kind, effects, flags, reasons, destructive, class)| Parts {
                kind,
                effects,
                flags,
                reasons,
                destructive,
                class,
            },
        )
}

/// `p` with READ when its effect set is empty: monotonicity starts from a non-empty set (the
/// empty set is `Unknown` because nothing was proved, and a producer never grows a report from
/// it).
pub fn non_empty(mut p: Parts) -> Parts {
    if p.effects.is_empty() {
        p.effects = EffectSet::READ;
    }
    p
}

/// Whether the statement asks for `EXPLAIN ANALYZE` (kind, flag or reason).
pub fn explain_analyze(s: &StmtReport) -> bool {
    s.kind == (StmtKind::Explain { analyze: true })
        || s.flags.contains(RiskFlags::EXPLAIN_ANALYZE)
        || s.reasons
            .iter()
            .any(|r| r.code == ReasonCode::ExplainAnalyze)
}
