//! M3b Task 4: `GateClass` is total, ordered and fails closed (P-M3b-4; `alg/20` :266-278),
//! and the coarse class lattice (`alg/13` SQL-SAFE-01).

mod common;

use common::{all_effect_combinations, destructive_variants, flags, report, stmt_full};
use proptest::prelude::*;
use sql_safety::effects::class_of;
use sql_safety::{
    CoarseClass, DestructiveKind, EffectSet, GateClass, RiskFlags, StmtKind, gate_class,
    opens_write,
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

fn effect_set() -> impl Strategy<Value = EffectSet> {
    any::<u32>().prop_map(EffectSet::from_bits_retain)
}

fn kinds() -> impl Strategy<Value = Vec<DestructiveKind>> {
    prop::collection::vec(prop::sample::select(DestructiveKind::ALL.to_vec()), 0..3)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20_000))]

    /// Adding effect bits or destructive reasons never lowers the gate's class.
    #[test]
    fn gate_class_is_monotone(a in effect_set(), b in effect_set(), ka in kinds(), kb in kinds()) {
        let mut both = ka.clone();
        both.extend(kb.iter().copied());
        let base = gate_class(&report(a, &ka));
        prop_assert!(gate_class(&report(a | b, &ka)) >= base);
        prop_assert!(gate_class(&report(a, &both)) >= base);
    }

    /// Adding bits never lowers the coarse class.
    #[test]
    fn class_of_is_monotone(a in effect_set(), b in effect_set()) {
        prop_assert!(class_of(a | b) >= class_of(a));
    }

    /// Read only for a non-empty subset of READ with no destructive reason, ever.
    #[test]
    fn read_needs_only_read(e in effect_set(), k in kinds()) {
        if gate_class(&report(e, &k)) == GateClass::Read {
            prop_assert_eq!(e, EffectSet::READ);
            prop_assert!(k.is_empty());
        }
        if class_of(e) == CoarseClass::Read {
            prop_assert_eq!(e, EffectSet::READ);
        }
    }
}
