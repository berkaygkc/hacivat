//! Effects, risk flags and the coarse class (`alg/13` SQL-SAFE-01; Task 4).
//!
//! [`class_of`] maps an effect set to its [`CoarseClass`]: every named bit has a class in
//! [`COARSE_OF_BIT`] (a test proves the table covers [`EffectSet::all`]), the most critical
//! class wins (`Unknown > Admin > Ddl > Write > Session > Read`), and an empty set, or a set
//! holding any bit the table does not name, is `Unknown`, never `Read` (fail closed).

use bitflags::bitflags;

bitflags! {
    /// What a statement may do. `UNKNOWN` is critical: the classifier could not prove the
    /// statement's effects (L9, L10).
    ///
    /// **Empty is not `READ`.** `EffectSet::default()` is the empty set, which says nothing
    /// was proved: every mapping (`gate_class`, `class_of`, the policy) must treat an empty set
    /// as `Unknown`, never as a read (fix round 1, M3; Tasks 4, 14, 15).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct EffectSet: u32 {
        const READ = 1;
        const WRITE_DATA = 1 << 1;
        const WRITE_TEMP = 1 << 2;
        const DDL = 1 << 3;
        const DDL_TEMP = 1 << 4;
        const TXN_CONTROL = 1 << 5;
        const SESSION_STATE = 1 << 6;
        const LOCK_ROW = 1 << 7;
        const LOCK_TABLE = 1 << 8;
        const ADVISORY_LOCK = 1 << 9;
        const PRIVILEGE = 1 << 10;
        const ADMIN = 1 << 11;
        const EXTERNAL_IO = 1 << 12;
        const PROCESS_EXEC = 1 << 13;
        const SEQUENCE = 1 << 14;
        const XID_CONSUME = 1 << 15;
        const PREPARE = 1 << 16;
        const ROUTINE_CALL = 1 << 17;
        const NOTIFY = 1 << 18;
        const UNKNOWN = 1 << 31;
    }
}

bitflags! {
    /// What makes a statement riskier than its effects say.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct RiskFlags: u16 {
        const EXEC_COMMENT = 1;
        const DYNAMIC_SQL = 2;
        const LEX_MODE_CHANGE = 4;
        const EXPLAIN_ANALYZE = 8;
        const VOLATILE_FN = 16;
    }
}

/// The coarse class of a statement or request (L9's operation classes); ordered from the
/// least to the most critical.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub enum CoarseClass {
    Read,
    Session,
    Write,
    Ddl,
    Admin,
    Unknown,
}

impl CoarseClass {
    /// Every class, from the least to the most critical.
    pub const ALL: [CoarseClass; 6] = [
        CoarseClass::Read,
        CoarseClass::Session,
        CoarseClass::Write,
        CoarseClass::Ddl,
        CoarseClass::Admin,
        CoarseClass::Unknown,
    ];

    /// The Fluent message id, a literal per arm (P24): `safety-class-<kebab>`.
    pub fn fluent_id(self) -> &'static str {
        match self {
            CoarseClass::Read => "safety-class-read",
            CoarseClass::Session => "safety-class-session",
            CoarseClass::Write => "safety-class-write",
            CoarseClass::Ddl => "safety-class-ddl",
            CoarseClass::Admin => "safety-class-admin",
            CoarseClass::Unknown => "safety-class-unknown",
        }
    }
}

/// The coarse class of every named effect bit (`alg/13` SQL-SAFE-01's table, on M3b's bits).
///
/// `ROUTINE_CALL` is `Unknown`: a routine's body is opaque to the classifier (`CallStmt` is
/// `PROCEDURAL_CODE` in `alg/13`; its test 13, `CALL p()`, is Unknown). `PREPARE` and
/// `NOTIFY` are `Session` (`PrepareStmt`, `NotifyStmt`). Locks, temp writes, sequences and
/// XIDs are `Write`; temp DDL is `Ddl`.
pub const COARSE_OF_BIT: [(EffectSet, CoarseClass); 20] = [
    (EffectSet::READ, CoarseClass::Read),
    (EffectSet::WRITE_DATA, CoarseClass::Write),
    (EffectSet::WRITE_TEMP, CoarseClass::Write),
    (EffectSet::DDL, CoarseClass::Ddl),
    (EffectSet::DDL_TEMP, CoarseClass::Ddl),
    (EffectSet::TXN_CONTROL, CoarseClass::Session),
    (EffectSet::SESSION_STATE, CoarseClass::Session),
    (EffectSet::LOCK_ROW, CoarseClass::Write),
    (EffectSet::LOCK_TABLE, CoarseClass::Write),
    (EffectSet::ADVISORY_LOCK, CoarseClass::Write),
    (EffectSet::PRIVILEGE, CoarseClass::Admin),
    (EffectSet::ADMIN, CoarseClass::Admin),
    (EffectSet::EXTERNAL_IO, CoarseClass::Admin),
    (EffectSet::PROCESS_EXEC, CoarseClass::Admin),
    (EffectSet::SEQUENCE, CoarseClass::Write),
    (EffectSet::XID_CONSUME, CoarseClass::Write),
    (EffectSet::PREPARE, CoarseClass::Session),
    (EffectSet::ROUTINE_CALL, CoarseClass::Unknown),
    (EffectSet::NOTIFY, CoarseClass::Session),
    (EffectSet::UNKNOWN, CoarseClass::Unknown),
];

/// The coarse class of `effects`: the most critical class of its bits. An empty set, or one
/// holding a bit [`COARSE_OF_BIT`] does not name (a raw bit outside the named flags, or a flag
/// added without a row), is `Unknown`.
///
/// Public (the plan names it `pub(crate)`) so the integration tests and the exec crates can
/// read the lattice; it is pure.
pub fn class_of(effects: EffectSet) -> CoarseClass {
    if effects.is_empty() {
        return CoarseClass::Unknown;
    }
    let mut named = EffectSet::empty();
    let mut class = CoarseClass::Read;
    for (bit, c) in COARSE_OF_BIT {
        if effects.intersects(bit) {
            named |= bit;
            class = class.max(c);
        }
    }
    if named.bits() != effects.bits() {
        return CoarseClass::Unknown;
    }
    class
}

/// Whether `flags` holds only named risk flags (an unnamed bit is a flag this build does not
/// understand: the callers fail closed).
pub(crate) fn flags_are_named(flags: RiskFlags) -> bool {
    RiskFlags::from_bits(flags.bits()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_names_every_named_bit_exactly_once() {
        let mut union = EffectSet::empty();
        for (bit, _) in COARSE_OF_BIT {
            assert_eq!(bit.bits().count_ones(), 1, "{bit:?}");
            assert!(!union.intersects(bit), "{bit:?} twice");
            union |= bit;
        }
        assert_eq!(union, EffectSet::all());
    }

    #[test]
    fn empty_and_unnamed_bits_are_unknown() {
        assert_eq!(class_of(EffectSet::empty()), CoarseClass::Unknown);
        for raw in 19..31 {
            let e = EffectSet::from_bits_retain(1 << raw);
            assert_eq!(class_of(e), CoarseClass::Unknown, "bit {raw}");
            assert_eq!(
                class_of(e | EffectSet::READ),
                CoarseClass::Unknown,
                "bit {raw}"
            );
        }
        assert!(!flags_are_named(RiskFlags::from_bits_retain(1 << 9)));
        assert!(flags_are_named(RiskFlags::all()));
    }

    #[test]
    fn class_ids_are_distinct_and_all_is_ordered() {
        let ids: std::collections::BTreeSet<_> =
            CoarseClass::ALL.iter().map(|c| c.fluent_id()).collect();
        assert_eq!(ids.len(), CoarseClass::ALL.len());
        assert!(CoarseClass::ALL.windows(2).all(|w| w[0] < w[1]));
    }
}
