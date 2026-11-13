//! Post-statement checks: did a guarded statement end the read-only transaction, get a
//! transaction ID or write anyway (SQL-SAFE-05)? [`check_post`] is a tripwire run after the
//! server's own read-only enforcement, never instead of it.

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

/// PostgreSQL's "there is no transaction in progress" (SQLSTATE 25P01), as a notice on the
/// guard's own `ROLLBACK`: the user statement already ended the guard's transaction.
const NO_TRANSACTION_IN_PROGRESS: &str = "25P01";

/// First keywords of statements that change data and report a row count (PostgreSQL command
/// tags; MySQL and SQLite report the statement's first keyword, Q35). `COPY` is both
/// directions: the tag cannot tell `COPY … TO` from `COPY … FROM`, so the check fails closed.
/// `REPLACE` and `LOAD` (`LOAD DATA`) are MySQL's row-writing verbs.
const ROW_WRITE_TAGS: [&str; 7] = [
    "INSERT", "UPDATE", "DELETE", "MERGE", "COPY", "REPLACE", "LOAD",
];

/// First keywords of statements that change the schema or privileges. They never carry a row
/// count, so one that succeeded inside the read-only transaction is an escape on its own.
const SCHEMA_WRITE_TAGS: [&str; 7] = [
    "CREATE", "DROP", "ALTER", "TRUNCATE", "RENAME", "GRANT", "REVOKE",
];

/// Whether `facts` show a gap in `kind`'s guard (SQL-SAFE-05), checked in this order:
///
/// 1. PostgreSQL, MySQL and MariaDB: the statement left the guard's transaction (`Idle`, or
///    an unknown state, which fails closed), or the guard's `ROLLBACK` drew `25P01` →
///    `TransactionEndedByStatement`.
/// 2. A write tag succeeded: a row-writing tag with `rows_affected > 0`, or any schema or
///    privilege tag → `WriteSucceeded`.
/// 3. The canary returned a transaction ID (`Some(Some(_))`) → `CanaryAssigned`.
///
/// SQLite runs in autocommit (its layers are connection-level), so only step 2 applies. An
/// engine without a guard plan has no read-only guard at all, so every statement is a gap.
pub fn check_post(kind: DbmsKind, facts: &PostFacts) -> PostCheck {
    match kind {
        DbmsKind::Postgres | DbmsKind::MySql | DbmsKind::MariaDb => {
            if transaction_ended(facts) {
                return PostCheck::Gap(GapReason::TransactionEndedByStatement);
            }
            if write_succeeded(facts) {
                return PostCheck::Gap(GapReason::WriteSucceeded);
            }
            if matches!(facts.canary, Some(Some(_))) {
                return PostCheck::Gap(GapReason::CanaryAssigned);
            }
            PostCheck::Clean
        }
        DbmsKind::Sqlite => {
            if write_succeeded(facts) {
                PostCheck::Gap(GapReason::WriteSucceeded)
            } else {
                PostCheck::Clean
            }
        }
        _ => PostCheck::Gap(GapReason::TransactionEndedByStatement),
    }
}

fn transaction_ended(facts: &PostFacts) -> bool {
    let left = !matches!(
        facts.tx_after_statement,
        Some(TxStatusLite::InTx | TxStatusLite::Failed)
    );
    left || facts
        .rollback_notice_codes
        .iter()
        .any(|c| c.trim().eq_ignore_ascii_case(NO_TRANSACTION_IN_PROGRESS))
}

fn write_succeeded(facts: &PostFacts) -> bool {
    let Some(first) = facts
        .command_tag
        .as_deref()
        .and_then(|t| t.split_ascii_whitespace().next())
    else {
        return false;
    };
    let is = |tags: &[&str]| tags.iter().any(|t| first.eq_ignore_ascii_case(t));
    is(&SCHEMA_WRITE_TAGS) || (is(&ROW_WRITE_TAGS) && facts.rows_affected.is_some_and(|n| n > 0))
}
