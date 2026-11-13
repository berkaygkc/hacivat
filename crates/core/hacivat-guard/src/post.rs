//! Post-statement checks: did a guarded statement end the read-only transaction, get a
//! transaction ID or write anyway (SQL-SAFE-05)? Task 5 fills [`check_post`].

use hacivat_proto::DbmsKind;

/// The transaction status after a statement, as the guard sees it (the guard cannot depend on
/// `hacivat-driver`'s `TxStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxStatusLite {
    Idle,
    InTx,
    Failed,
}

/// Why a guarded statement escaped the guard (SQL-SAFE-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GapReason {
    CanaryAssigned,
    TransactionEndedByStatement,
    WriteSucceeded,
}

/// The verdict of [`check_post`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostCheck {
    Clean,
    Gap(GapReason),
}

/// What the session saw around one guarded statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct PostFacts {
    pub tx_after_statement: Option<TxStatusLite>,
    /// The canary's row: `None` when no canary ran, `Some(None)` for SQL NULL.
    pub canary: Option<Option<String>>,
    pub rollback_notice_codes: Vec<String>,
    pub command_tag: Option<String>,
    pub rows_affected: Option<u64>,
}

/// Whether `facts` show a gap in `kind`'s guard. Task 1 stub: fails closed, so every
/// statement counts as an escape (`TransactionEndedByStatement`) until Task 5.
pub fn check_post(_kind: DbmsKind, _facts: &PostFacts) -> PostCheck {
    PostCheck::Gap(GapReason::TransactionEndedByStatement)
}
