//! The exec gate's class of one statement (`alg/20` HV-EXEC-04; Task 4). Task 4 adds the total
//! order `Unknown > Admin > DdlDestructive > DdlOther > DmlUnsafe > Dml > Routine > Lock > Tx >
//! Session > Read`, the default (`Unknown`) and the mapping of every effect bit (P-M3b-4).
//!
//! STUB until Task 4 (fail closed): [`gate_class`] answers `Unknown` and [`opens_write`]
//! `true` (Smart would open a transaction: the safe direction).

use crate::report::StmtReport;

/// The gate's class of a statement.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum GateClass {
    Read,
    Lock,
    Dml,
    DmlUnsafe,
    DdlDestructive,
    DdlOther,
    Routine,
    Session,
    Tx,
    Admin,
    Unknown,
}

/// The gate's class of `r`. STUB: `Unknown`.
pub fn gate_class(r: &StmtReport) -> GateClass {
    let _ = r;
    GateClass::Unknown
}

/// Whether `r` may write, so Smart mode opens a transaction before it. STUB: `true`.
pub fn opens_write(r: &StmtReport) -> bool {
    let _ = r;
    true
}
