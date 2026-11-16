//! The exec gate's class of one statement (`alg/20` HV-EXEC-04 :266-278; Task 4, P-M3b-4).
//!
//! Total order (the most critical wins): `Unknown > Admin > DdlDestructive > DdlOther >
//! DmlUnsafe > Dml > Routine > Lock > Tx > Session > Read`. Every named effect bit has a row
//! in [`GATE_OF_BIT`] (a test proves it covers [`EffectSet::all`]); an empty set, a bit the
//! table does not name, an unnamed risk flag or `DYNAMIC_SQL` is `Unknown`, never `Read`.
//! Destructive reasons raise the class: an unsafe-DML kind to at least `DmlUnsafe`, a
//! destructive-DDL kind to at least `DdlDestructive`, whatever the effects say (a destructive
//! reason is evidence of a write even when a backend failed to set its bit).

use crate::effects::{CoarseClass, EffectSet, RiskFlags, class_of, flags_are_named};
use crate::report::StmtReport;

/// The gate's class of a statement, declared from the least to the most critical (the
/// derived `Ord` is the total order of P-M3b-4). The default is `Unknown` (fix round 1, M3),
/// so a defaulted carrier never reads as `Read`.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
pub enum GateClass {
    Read,
    Session,
    Tx,
    Lock,
    Routine,
    Dml,
    DmlUnsafe,
    DdlOther,
    DdlDestructive,
    Admin,
    #[default]
    Unknown,
}

impl GateClass {
    /// Every class, from the least to the most critical.
    pub const ALL: [GateClass; 11] = [
        GateClass::Read,
        GateClass::Session,
        GateClass::Tx,
        GateClass::Lock,
        GateClass::Routine,
        GateClass::Dml,
        GateClass::DmlUnsafe,
        GateClass::DdlOther,
        GateClass::DdlDestructive,
        GateClass::Admin,
        GateClass::Unknown,
    ];
}

/// The gate class of every named effect bit (P-M3b-4; `alg/20` :266-278). `WRITE_DATA` and
/// `DDL` are refined by destructive reasons in [`gate_class`].
pub const GATE_OF_BIT: [(EffectSet, GateClass); 20] = [
    (EffectSet::READ, GateClass::Read),
    (EffectSet::WRITE_DATA, GateClass::Dml),
    (EffectSet::WRITE_TEMP, GateClass::Dml),
    (EffectSet::DDL, GateClass::DdlOther),
    (EffectSet::DDL_TEMP, GateClass::DdlOther),
    (EffectSet::TXN_CONTROL, GateClass::Tx),
    (EffectSet::SESSION_STATE, GateClass::Session),
    (EffectSet::LOCK_ROW, GateClass::Lock),
    (EffectSet::LOCK_TABLE, GateClass::Lock),
    (EffectSet::ADVISORY_LOCK, GateClass::Lock),
    (EffectSet::PRIVILEGE, GateClass::Admin),
    (EffectSet::ADMIN, GateClass::Admin),
    (EffectSet::EXTERNAL_IO, GateClass::Admin),
    (EffectSet::PROCESS_EXEC, GateClass::Admin),
    (EffectSet::SEQUENCE, GateClass::Dml),
    (EffectSet::XID_CONSUME, GateClass::Dml),
    (EffectSet::PREPARE, GateClass::Session),
    (EffectSet::ROUTINE_CALL, GateClass::Routine),
    (EffectSet::NOTIFY, GateClass::Session),
    (EffectSet::UNKNOWN, GateClass::Unknown),
];

/// The gate's class of the effect set `effects` alone (no destructive reasons): the most
/// critical row of its bits; empty or holding an unnamed bit → `Unknown`.
fn class_of_effects(effects: EffectSet) -> GateClass {
    if effects.is_empty() {
        return GateClass::Unknown;
    }
    let mut named = EffectSet::empty();
    let mut class = GateClass::Read;
    for (bit, c) in GATE_OF_BIT {
        if effects.intersects(bit) {
            named |= bit;
            class = class.max(c);
        }
    }
    if named.bits() != effects.bits() {
        return GateClass::Unknown;
    }
    class
}

/// The gate's class of `r` (P-M3b-4): the most critical of its effect bits' rows, raised by
/// its destructive reasons; `Unknown` for an empty or unnamed effect set, an unnamed risk flag
/// or `DYNAMIC_SQL`.
pub fn gate_class(r: &StmtReport) -> GateClass {
    if r.flags.contains(RiskFlags::DYNAMIC_SQL) || !flags_are_named(r.flags) {
        return GateClass::Unknown;
    }
    let mut class = class_of_effects(r.effects);
    for d in &r.destructive {
        let floor = if d.kind.is_unsafe_dml() {
            GateClass::DmlUnsafe
        } else {
            GateClass::DdlDestructive
        };
        class = class.max(floor);
    }
    class
}

/// Whether `r` may write, so Smart mode opens a transaction before it: its coarse class (the
/// most critical of its `class` field and `class_of` over its effects) is neither `Read` nor
/// `Session`, or its gate class is above `Tx` (a destructive reason, `DYNAMIC_SQL` or an
/// unnamed flag the effects do not show). An empty or unnamed effect set is `Unknown`, so it
/// opens one.
pub fn opens_write(r: &StmtReport) -> bool {
    let c = r.class.max(class_of(r.effects));
    (c != CoarseClass::Read && c != CoarseClass::Session) || gate_class(r) > GateClass::Tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_unknown() {
        assert_eq!(GateClass::default(), GateClass::Unknown);
    }

    #[test]
    fn the_table_names_every_named_bit_exactly_once() {
        let mut union = EffectSet::empty();
        for (bit, _) in GATE_OF_BIT {
            assert_eq!(bit.bits().count_ones(), 1, "{bit:?}");
            assert!(!union.intersects(bit), "{bit:?} twice");
            union |= bit;
        }
        assert_eq!(union, EffectSet::all());
    }

    /// The two lattices agree on the bits they share a meaning for: nothing the gate calls
    /// `Read` or `Session` is a write in the coarse class, and vice versa.
    #[test]
    fn gate_and_coarse_rows_agree() {
        for (bit, g) in GATE_OF_BIT {
            let c = class_of(bit);
            match g {
                GateClass::Read => assert_eq!(c, CoarseClass::Read, "{bit:?}"),
                GateClass::Session | GateClass::Tx => {
                    assert_eq!(c, CoarseClass::Session, "{bit:?}")
                }
                GateClass::Lock | GateClass::Dml => assert_eq!(c, CoarseClass::Write, "{bit:?}"),
                GateClass::DdlOther => assert_eq!(c, CoarseClass::Ddl, "{bit:?}"),
                GateClass::Admin => assert_eq!(c, CoarseClass::Admin, "{bit:?}"),
                GateClass::Routine | GateClass::Unknown => {
                    assert_eq!(c, CoarseClass::Unknown, "{bit:?}")
                }
                GateClass::DmlUnsafe | GateClass::DdlDestructive => {
                    panic!("{bit:?}: no bit maps to a destructive class")
                }
            }
        }
    }
}
