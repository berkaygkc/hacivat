//! M3b Task 4: `GateClass` is total, ordered and fails closed (P-M3b-4; `alg/20` :266-278),
//! and the coarse class lattice (`alg/13` SQL-SAFE-01).

mod common;

use common::{
    all_effect_combinations, arb_parts, destructive_variants, explain_analyze, flags, reason,
    report, request, stmt_full,
};
use proptest::prelude::*;
use sql_safety::effects::class_of;
use sql_safety::{
    Actor, CoarseClass, DestructiveKind, EffectSet, GateClass, Profile, ReasonCode, RiskFlags,
    StmtKind, decide, gate_class, opens_write,
};

#[test]
fn gate_class_is_total() {
    let mut seen = 0u64;
    for e in all_effect_combinations(12) {
        for d in destructive_variants() {
            let _ = gate_class(&report(e, &d));
            seen += 1;
        }
    }
    // 2^20 subsets of the 20 named bits, minus those with more than 12 bits set, times the
    // 12 destructive variants (none + 11 kinds).
    assert!(seen > 10_000_000, "{seen}");
}

#[test]
fn read_only_effects_only_are_read() {
    for e in all_effect_combinations(12) {
        if !(e - EffectSet::READ).is_empty() {
            assert_ne!(gate_class(&report(e, &[])), GateClass::Read, "{e:?}");
        }
    }
    // The empty set says nothing was proved: Unknown, never Read (T1a review M3).
    assert_eq!(
        gate_class(&report(EffectSet::empty(), &[])),
        GateClass::Unknown
    );
    assert_eq!(gate_class(&report(EffectSet::READ, &[])), GateClass::Read);
}

#[test]
fn copy_to_program_is_admin() {
    assert_eq!(
        gate_class(&report(EffectSet::READ | EffectSet::PROCESS_EXEC, &[])),
        GateClass::Admin
    );
}

#[test]
fn lock_row_is_write_class() {
    assert_eq!(
        class_of(EffectSet::READ | EffectSet::LOCK_ROW),
        CoarseClass::Write
    );
    assert_eq!(
        gate_class(&report(EffectSet::READ | EffectSet::LOCK_ROW, &[])),
        GateClass::Lock
    );
}

#[test]
fn delete_without_where_is_dml_unsafe() {
    assert_eq!(
        gate_class(&report(EffectSet::WRITE_DATA, &[DestructiveKind::NoWhere])),
        GateClass::DmlUnsafe
    );
}

#[test]
fn the_most_critical_class_wins() {
    assert_eq!(
        class_of(EffectSet::READ | EffectSet::WRITE_DATA),
        CoarseClass::Write
    );
    assert_eq!(
        class_of(EffectSet::READ | EffectSet::UNKNOWN),
        CoarseClass::Unknown
    );
    assert_eq!(
        gate_class(&report(EffectSet::WRITE_DATA | EffectSet::DDL, &[])),
        GateClass::DdlOther
    );
    assert_eq!(
        gate_class(&report(EffectSet::PRIVILEGE | EffectSet::UNKNOWN, &[])),
        GateClass::Unknown
    );
}

/// P-M3b-4: every named bit maps explicitly.
#[test]
fn every_bit_maps_as_ruled() {
    use GateClass as G;
    let expect = [
        (EffectSet::READ, G::Read),
        (EffectSet::WRITE_DATA, G::Dml),
        (EffectSet::WRITE_TEMP, G::Dml),
        (EffectSet::DDL, G::DdlOther),
        (EffectSet::DDL_TEMP, G::DdlOther),
        (EffectSet::TXN_CONTROL, G::Tx),
        (EffectSet::SESSION_STATE, G::Session),
        (EffectSet::LOCK_ROW, G::Lock),
        (EffectSet::LOCK_TABLE, G::Lock),
        (EffectSet::ADVISORY_LOCK, G::Lock),
        (EffectSet::PRIVILEGE, G::Admin),
        (EffectSet::ADMIN, G::Admin),
        (EffectSet::EXTERNAL_IO, G::Admin),
        (EffectSet::PROCESS_EXEC, G::Admin),
        (EffectSet::SEQUENCE, G::Dml),
        (EffectSet::XID_CONSUME, G::Dml),
        (EffectSet::PREPARE, G::Session),
        (EffectSet::ROUTINE_CALL, G::Routine),
        (EffectSet::NOTIFY, G::Session),
        (EffectSet::UNKNOWN, G::Unknown),
    ];
    assert_eq!(
        expect.len(),
        flags().len(),
        "a named bit has no expectation"
    );
    for (bit, class) in expect {
        assert_eq!(gate_class(&report(bit, &[])), class, "{bit:?}");
        // With READ alongside, the bit still decides (READ is the least critical).
        assert_eq!(
            gate_class(&report(bit | EffectSet::READ, &[])),
            class,
            "{bit:?}"
        );
    }
}

#[test]
fn bits_outside_the_named_flags_are_unknown() {
    for raw in 19..31 {
        let e = EffectSet::from_bits_retain(1 << raw);
        assert_eq!(gate_class(&report(e, &[])), GateClass::Unknown, "bit {raw}");
        assert_eq!(
            gate_class(&report(e | EffectSet::READ, &[])),
            GateClass::Unknown,
            "bit {raw}"
        );
    }
}

#[test]
fn the_total_order_is_the_ruled_one() {
    use GateClass as G;
    let order = [
        G::Read,
        G::Session,
        G::Tx,
        G::Lock,
        G::Routine,
        G::Dml,
        G::DmlUnsafe,
        G::DdlOther,
        G::DdlDestructive,
        G::Admin,
        G::Unknown,
    ];
    assert_eq!(GateClass::ALL, order);
    assert!(order.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(GateClass::default(), G::Unknown);
}

#[test]
fn destructive_reasons_raise_the_class() {
    use DestructiveKind as K;
    use GateClass as G;
    for k in [
        K::NoWhere,
        K::AlwaysTrue,
        K::JoinWithoutOn,
        K::UpdateLimitNoOrder,
        K::MergeDelete,
    ] {
        assert_eq!(
            gate_class(&report(EffectSet::WRITE_DATA, &[k])),
            G::DmlUnsafe,
            "{k:?}"
        );
        assert_eq!(
            gate_class(&report(EffectSet::READ | EffectSet::WRITE_DATA, &[k])),
            G::DmlUnsafe,
            "{k:?}"
        );
    }
    for k in [
        K::Drop { cascade: false },
        K::Drop { cascade: true },
        K::Truncate,
        K::DropColumn,
        K::DropConstraint,
        K::AlterType,
    ] {
        assert_eq!(
            gate_class(&report(EffectSet::DDL, &[k])),
            G::DdlDestructive,
            "{k:?}"
        );
    }
    // TRUNCATE is WRITE_DATA + DDL (`alg/13`): destructive DDL.
    assert_eq!(
        gate_class(&report(
            EffectSet::WRITE_DATA | EffectSet::DDL,
            &[K::Truncate]
        )),
        G::DdlDestructive
    );
    // A bounded write stays Dml; a plain DDL stays DdlOther.
    assert_eq!(gate_class(&report(EffectSet::WRITE_DATA, &[])), G::Dml);
    assert_eq!(gate_class(&report(EffectSet::DDL, &[])), G::DdlOther);
    // A destructive reason on a report whose effects do not show it (a backend slip) still
    // raises the class: the reason is evidence of a write.
    assert_eq!(
        gate_class(&report(EffectSet::READ, &[K::NoWhere])),
        G::DmlUnsafe
    );
    assert_eq!(
        gate_class(&report(EffectSet::READ, &[K::Drop { cascade: true }])),
        G::DdlDestructive
    );
    // Destructive reasons never lower Admin or Unknown.
    assert_eq!(
        gate_class(&report(EffectSet::ADMIN, &[K::NoWhere])),
        G::Admin
    );
    assert_eq!(
        gate_class(&report(EffectSet::UNKNOWN, &[K::Truncate])),
        G::Unknown
    );
}

#[test]
fn dynamic_sql_is_unknown() {
    let r = stmt_full(
        StmtKind::Select,
        EffectSet::READ,
        RiskFlags::DYNAMIC_SQL,
        &[],
        Vec::new(),
    );
    assert_eq!(gate_class(&r), GateClass::Unknown);
}

#[test]
fn opens_write_only_for_writing_classes() {
    assert!(!opens_write(&report(EffectSet::READ, &[])));
    assert!(!opens_write(&report(EffectSet::SESSION_STATE, &[])));
    assert!(!opens_write(&report(EffectSet::TXN_CONTROL, &[])));
    for e in [
        EffectSet::WRITE_DATA,
        EffectSet::LOCK_ROW | EffectSet::READ,
        EffectSet::DDL,
        EffectSet::ADMIN,
        EffectSet::ROUTINE_CALL,
        EffectSet::UNKNOWN,
        EffectSet::empty(),
        EffectSet::from_bits_retain(1 << 25),
    ] {
        assert!(opens_write(&report(e, &[])), "{e:?}");
    }
    // A report whose class field is more critical than its effects (a backend that knew
    // more) opens a transaction.
    let mut r = report(EffectSet::READ, &[]);
    r.class = CoarseClass::Unknown;
    assert!(opens_write(&r));
}

// ---------------------------------------------------------------------------------------------
// Fix round 1: evidence beyond the effect bits (I1, I2, I5)

/// H1: a class field more critical than the effects raises the gate class, to the least gate
/// class of that coarse class (`Unknown` → `Routine`: a `CALL`'s field is `Unknown`, and
/// R-M3-55 decides `Routine` and `Unknown` alike).
#[test]
fn the_class_field_is_a_floor() {
    use CoarseClass as C;
    use GateClass as G;
    for (c, g) in [
        (C::Read, G::Read),
        (C::Session, G::Session),
        (C::Write, G::Lock),
        (C::Ddl, G::DdlOther),
        (C::Admin, G::Admin),
        (C::Unknown, G::Routine),
    ] {
        let mut r = report(EffectSet::READ, &[]);
        r.class = c;
        assert_eq!(gate_class(&r), g, "{c:?}");
    }
}

/// H2: every reason code on a READ statement: the gate says `Read` only where
/// `decide(ReadOnlyPlusExplain, Human)` allows.
#[test]
fn reasons_are_gate_evidence() {
    use GateClass as G;
    use ReasonCode as R;
    for code in ReasonCode::ALL {
        for arg in [None, Some("x"), Some("0"), Some("1000")] {
            let s = stmt_full(
                StmtKind::Select,
                EffectSet::READ,
                RiskFlags::empty(),
                &[],
                vec![reason(code, arg)],
            );
            let g = gate_class(&s);
            let allowed = decide(
                &request(vec![s]),
                Profile::ReadOnlyPlusExplain,
                Actor::Human,
            );
            if g == G::Read {
                assert!(
                    allowed.allows(),
                    "{code:?} {arg:?}: gate Read, decide {allowed:?}"
                );
            }
        }
    }
    let at = |code| {
        gate_class(&stmt_full(
            StmtKind::Select,
            EffectSet::READ,
            RiskFlags::empty(),
            &[],
            vec![reason(code, Some("x"))],
        ))
    };
    for code in [
        R::FnUnknown,
        R::OperatorUnknown,
        R::CastUnknown,
        R::DynamicSql,
        R::ProceduralCode,
        R::ParseError,
        R::ParserDisagreement,
        R::TooComplex,
        R::LexModeChange,
        R::FnDenylisted,
        R::ExecutableComment,
        R::EffectsUnknown,
        R::VolatileFunction,
    ] {
        assert_eq!(at(code), G::Unknown, "{code:?}");
    }
    for code in [
        R::ProcessExec,
        R::ExternalFile,
        R::ExternalNetwork,
        R::AdminControl,
        R::ExternalIo,
        R::PrivilegeGrant,
        R::PrivilegeChange,
    ] {
        assert_eq!(at(code), G::Admin, "{code:?}");
    }
    assert_eq!(at(R::WriteData), G::Dml);
    assert_eq!(at(R::LockingClause), G::Lock);
    assert_eq!(at(R::SchemaChange), G::DdlOther);
    assert_eq!(at(R::TxnControl), G::Tx);
    assert_eq!(at(R::SessionState), G::Session);
    assert_eq!(at(R::RoutineCall), G::Routine);
    // M3: a bare Destructive reason is at least destructive DDL.
    assert_eq!(at(R::Destructive), G::DdlDestructive);
}

/// H3: a lexical-mode change is `Unknown` for the gate (production confirms it).
#[test]
fn lex_mode_change_is_unknown() {
    let s = stmt_full(
        StmtKind::Set,
        EffectSet::SESSION_STATE,
        RiskFlags::LEX_MODE_CHANGE,
        &[],
        vec![reason(
            ReasonCode::SessionState,
            Some("standard_conforming_strings"),
        )],
    );
    assert_eq!(gate_class(&s), GateClass::Unknown);
}

/// I2: every named flag has a gate rule; `VOLATILE_FN` is `Unknown` (`alg/13` S-04).
#[test]
fn flags_are_gate_evidence() {
    use GateClass as G;
    for (flag, g) in [
        (RiskFlags::EXEC_COMMENT, G::Unknown),
        (RiskFlags::DYNAMIC_SQL, G::Unknown),
        (RiskFlags::LEX_MODE_CHANGE, G::Unknown),
        (RiskFlags::EXPLAIN_ANALYZE, G::Read),
        (RiskFlags::VOLATILE_FN, G::Unknown),
    ] {
        let s = stmt_full(StmtKind::Select, EffectSet::READ, flag, &[], Vec::new());
        assert_eq!(gate_class(&s), g, "{flag:?}");
    }
    assert_eq!(
        RiskFlags::all().iter().count(),
        5,
        "a new flag needs a row above"
    );
}

/// H6 / I5: a statement's kind is evidence whatever its bits say.
#[test]
fn kinds_are_floors() {
    use GateClass as G;
    use StmtKind as K;
    for (k, g) in [
        (K::Set, G::Session),
        (K::Insert, G::Dml),
        (K::Update, G::Dml),
        (K::Delete, G::Dml),
        (K::Merge, G::Dml),
        (K::Drop, G::DdlDestructive),
        (K::Truncate, G::DdlDestructive),
        (K::CreateTable, G::DdlOther),
        (K::CreateOther, G::DdlOther),
        (K::Alter, G::DdlOther),
        (K::Grant, G::Admin),
        (K::Revoke, G::Admin),
        (K::Call, G::Routine),
        (K::Do, G::Routine),
        (K::Execute, G::Unknown),
        (K::Begin, G::Tx),
        (K::Commit, G::Tx),
        (K::Rollback, G::Tx),
        (K::Savepoint, G::Tx),
        (K::Lock, G::Lock),
        (K::Use, G::Session),
        (K::Listen, G::Session),
        (K::Prepare, G::Session),
        (K::Notify, G::Session),
        // The effects decide these.
        (K::Select, G::Read),
        (K::Values, G::Read),
        (K::Table, G::Read),
        (K::Show, G::Read),
        (K::Copy, G::Read),
        (K::Pragma, G::Read),
        (K::Other, G::Read),
        (K::Explain { analyze: false }, G::Read),
        // EXPLAIN ANALYZE: the class of its inner statement (R-M3-55 "class of x").
        (K::Explain { analyze: true }, G::Read),
    ] {
        let s = stmt_full(k, EffectSet::READ, RiskFlags::empty(), &[], Vec::new());
        assert_eq!(gate_class(&s), g, "{k:?}");
    }
    assert_eq!(common::ALL_KINDS.len(), 33);
    let ea = |e| {
        gate_class(&stmt_full(
            StmtKind::Explain { analyze: true },
            e,
            RiskFlags::EXPLAIN_ANALYZE,
            &[],
            Vec::new(),
        ))
    };
    assert_eq!(ea(EffectSet::READ | EffectSet::WRITE_DATA), GateClass::Dml);
    assert_eq!(ea(EffectSet::empty()), GateClass::Unknown);
}

/// Adding bits never lowers the coarse class, over the named subsets. The empty set is the
/// one exception: it is `Unknown` because nothing was proved, and a producer never grows a
/// report from it.
#[test]
fn class_of_is_monotone_over_named_subsets() {
    let all: Vec<EffectSet> = all_effect_combinations(3)
        .filter(|e| !e.is_empty())
        .collect();
    for a in &all {
        for b in flags() {
            assert!(class_of(*a | b) >= class_of(*a), "{a:?} + {b:?}");
        }
    }
}

fn ro_allows(s: &sql_safety::StmtReport, p: Profile) -> bool {
    decide(&request(vec![s.clone()]), p, Actor::Human).allows()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// I1: the gate never calls `Read` what `decide` refuses on the human read-only profile,
    /// and `ReadOnly` refuses a gate-`Read` statement only for `EXPLAIN ANALYZE`
    /// (`ReadOnlyPlusExplain`'s one extra allow). Conversely, whatever the read-only profile
    /// allows the gate calls `Read` (or `Session` for a harmless SET or SET_VAR hint).
    #[test]
    fn gate_and_decide_agree(p in arb_parts()) {
        let s = p.stmt();
        let g = gate_class(&s);
        if g == GateClass::Read {
            prop_assert!(ro_allows(&s, Profile::ReadOnlyPlusExplain), "{p:?}");
            prop_assert!(ro_allows(&s, Profile::ReadOnly) || explain_analyze(&s), "{p:?}");
        }
        if ro_allows(&s, Profile::ReadOnlyPlusExplain) {
            prop_assert!(g <= GateClass::Session, "{p:?}: {g:?}");
        }
        if ro_allows(&s, Profile::ReadOnly) {
            prop_assert!(g <= GateClass::Session, "{p:?}: {g:?}");
            prop_assert!(s.destructive.is_empty());
            prop_assert!(class_of(s.effects) == CoarseClass::Read, "{p:?}");
        }
    }

    /// Adding a bit, flag, reason or destructive kind never lowers the gate class or the
    /// Smart-mode answer (from a non-empty effect set: the empty set is `Unknown` because
    /// nothing was proved, and a producer never grows a report from it).
    #[test]
    fn gate_class_is_monotone(a in arb_parts(), b in arb_parts()) {
        let a = common::non_empty(a);
        let (s, t) = (a.stmt(), a.grown(&b).stmt());
        prop_assert!(gate_class(&t) >= gate_class(&s), "{a:?} + {b:?}");
        prop_assert!(opens_write(&t) || !opens_write(&s), "{a:?} + {b:?}");
    }

    /// `Read` only for exactly READ, with no destructive pattern and no flag but
    /// `EXPLAIN_ANALYZE`, and a reason only of the read-neutral kind.
    #[test]
    fn read_needs_only_read(p in arb_parts()) {
        let s = p.stmt();
        if gate_class(&s) == GateClass::Read {
            prop_assert_eq!(s.effects, EffectSet::READ);
            prop_assert!(s.destructive.is_empty());
            prop_assert!((s.flags - RiskFlags::EXPLAIN_ANALYZE).is_empty());
            prop_assert!(s.class == CoarseClass::Read);
            prop_assert!(!opens_write(&s));
        }
    }
}
