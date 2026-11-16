//! Effects, risk flags and the coarse class (`alg/13` SQL-SAFE-01; Task 4 maps statements to
//! them).

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
