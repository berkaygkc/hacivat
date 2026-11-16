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
    ReasonCode, RiskFlags, SafetyReport, StmtKind, StmtReport, TouchedObjects, classified_hash,
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

/// `SET <names…>` on `d`, one `SessionState` reason per target.
pub fn set_of(d: Option<DialectId>, names: &[&str]) -> SafetyReport {
    request_in(
        d,
        vec![stmt_full(
            StmtKind::Set,
            EffectSet::SESSION_STATE,
            RiskFlags::empty(),
            &[],
            names
                .iter()
                .map(|n| reason(ReasonCode::SessionState, Some(n)))
                .collect(),
        )],
    )
}
