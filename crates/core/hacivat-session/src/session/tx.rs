//! Transaction state and modes (HV-SESS-03, HV-EXEC-09/-10, B22; Task 7).
//!
//! The status always comes from the protocol (`RawConn::tx_status()` after every round trip);
//! the mode is the session's. A read-only session is always Auto (Q14).

use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{DbmsKind, EnvTag};

use crate::session::schema::{Family, family};

/// How a session handles transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TxMode {
    /// Every statement commits on its own.
    #[default]
    Auto,
    /// The user begins, commits and rolls back.
    Manual,
    /// Auto, but a write opens a transaction (Task 7).
    Smart,
}

impl TxMode {
    pub const ALL: [TxMode; 3] = [TxMode::Auto, TxMode::Manual, TxMode::Smart];

    /// The stable ID: `auto`, `manual`, `smart`.
    pub fn id(self) -> &'static str {
        match self {
            TxMode::Auto => "auto",
            TxMode::Manual => "manual",
            TxMode::Smart => "smart",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }
}

/// A transaction's isolation level; `Default` keeps the server's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IsolationLevel {
    #[default]
    Default,
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    /// The SQL name of the level; `None` keeps the server's.
    fn sql(self) -> Option<&'static str> {
        match self {
            IsolationLevel::Default => None,
            IsolationLevel::ReadUncommitted => Some("READ UNCOMMITTED"),
            IsolationLevel::ReadCommitted => Some("READ COMMITTED"),
            IsolationLevel::RepeatableRead => Some("REPEATABLE READ"),
            IsolationLevel::Serializable => Some("SERIALIZABLE"),
        }
    }
}

/// A session's transaction (its snapshot part).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TxState {
    pub mode: TxMode,
    pub isolation: IsolationLevel,
    pub status: TxStatus,
    /// When the open transaction began.
    pub since: Option<Instant>,
    /// Statements in the open transaction.
    pub statements: u32,
    // ## m3a-t01b-seams
    /// Whose transaction is open (Task 18 keeps it; the console UX shows only the user's).
    pub owner: TxOwner,
}

// ## m3a-t01b-seams
/// Who opened the session's transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TxOwner {
    /// No transaction, or one nobody claimed.
    #[default]
    None,
    /// The user (`BEGIN`, Manual mode).
    User,
    /// Hacivat, around a PostgreSQL console cursor (R-M3-23).
    Cursor,
    /// The read-only guard's plan.
    Guard,
    // ## m3b-t01c-core-seams
    /// Hacivat's own `BEGIN` around a confirmed statement on an idle Auto session (P-M3b-1,
    /// Task 19): Commit/Roll back, the D-014 deadline rolls it back.
    Confirm,
}

// ## m3b-t01c-core-seams (P-M3b-12)
impl TxOwner {
    /// Whether Hacivat opened the transaction itself (around a cursor, the guard's, or a
    /// confirmed statement's): it is never the user's, never refuses a close and is no idle
    /// transaction of the user's. Every ownership check asks this, so a new owner of
    /// Hacivat's is one line here.
    pub fn is_hacivats(self) -> bool {
        matches!(self, Self::Cursor | Self::Guard | Self::Confirm)
    }
}

impl TxState {
    // ## m3a-f1-followups
    /// Whether a transaction of the user's is open (`InTx` or `Failed`, not Hacivat's own
    /// around a cursor or the guard's).
    pub(crate) fn user_open(&self) -> bool {
        // ## m3b-t01c-core-seams (P-M3b-12)
        !self.owner.is_hacivats() && matches!(self.status, TxStatus::InTx | TxStatus::Failed)
    }

    /// Takes the protocol's `status` after a round trip; `statement` counts a user statement
    /// that ran in the open transaction.
    pub(crate) fn observe(&mut self, status: TxStatus, statement: bool) {
        if status == TxStatus::Idle {
            self.since = None;
            self.statements = 0;
        } else {
            if self.since.is_none() {
                self.since = Some(Instant::now());
                self.statements = 0;
            }
            if statement {
                self.statements = self.statements.saturating_add(1);
            }
        }
        self.status = status;
    }
}

/// A transaction request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxCommand {
    Begin,
    Commit,
    Rollback,
    SetMode(TxMode),
    SetIsolation(IsolationLevel),
}

/// Why a transaction request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxError {
    /// A transaction is already open.
    InProgress,
    /// Smart mode needs the statement classifier (M3).
    NeedsClassifier,
    /// Not on this engine, or not on a read-only session (Q14).
    Unsupported,
    /// The transaction failed: only a rollback is accepted.
    FailedNeedsRollback,
    // ## m3b-t01c-core-seams (P-M3b-1)
    /// MySQL/MariaDB: a DDL met an open transaction the gate did not confirm it for (an earlier
    /// unit of the run opened it); it would commit that transaction, so nothing was sent
    /// (Task 19).
    ImplicitCommitUnconfirmed,
    // ## m3b-t19-confirm-tx (fix round 1, C1, C2, m5)
    /// The confirmed statement was committed by the server on its own (`autocommit` was on):
    /// nothing was asked, nothing could be undone, and the rest of the run was not sent.
    ConfirmAlreadyCommitted,
    /// Nobody answered the confirmed statement before the D-014 deadline: it was rolled back
    /// and the rest of the run was not sent.
    ConfirmTimedOut,
    /// The user rolled back (undid) the confirmed statement: it did not take effect, so the
    /// run's error policy decides what follows.
    ConfirmUndone,
    // ## m3b-t19-confirm-tx (fix round 2, R1-m3)
    /// An `EXPLAIN ANALYZE` ran outside a transaction (nothing was open after it), so what it
    /// changed could not be rolled back; the rest of the run was not sent.
    ExplainNotRolledBack,
    /// MySQL/MariaDB: the user's `LOCK TABLES` locks are held, and the transaction this needed
    /// would release them; nothing was sent.
    TableLocksHeld,
    // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1)
    /// MySQL/MariaDB: the rollback of the confirmed (or wrapped) statement ran, but the server
    /// warned that a table does not support transactions (1196 and kin): its changes stay.
    ConfirmNotUndone,
}

/// The transaction mode a profile starts with: Manual on production, else Auto.
pub fn default_mode(env: Option<EnvTag>) -> TxMode {
    match env {
        Some(EnvTag::Production) => TxMode::Manual,
        _ => TxMode::Auto,
    }
}

/// The statements that begin a transaction on `kind` at `iso`:
/// - PostgreSQL: one `BEGIN [ISOLATION LEVEL …] [READ ONLY]`;
/// - MySQL and MariaDB: `SET TRANSACTION ISOLATION LEVEL …` (when not `Default`), then
///   `START TRANSACTION [READ ONLY]`;
/// - SQLite: `BEGIN` (always serializable; the actor uses `BEGIN IMMEDIATE` before a write).
pub fn begin_sql(kind: DbmsKind, iso: IsolationLevel, read_only: bool) -> Vec<String> {
    match family(kind) {
        Family::Sqlite => vec!["BEGIN".to_string()],
        Family::MySql => {
            let start = if read_only {
                "START TRANSACTION READ ONLY"
            } else {
                "START TRANSACTION"
            };
            let mut out = Vec::new();
            if let Some(level) = iso.sql() {
                out.push(format!("SET TRANSACTION ISOLATION LEVEL {level}"));
            }
            out.push(start.to_string());
            out
        }
        Family::Postgres | Family::Other => {
            let mut sql = String::from("BEGIN");
            if let Some(level) = iso.sql() {
                sql.push_str(" ISOLATION LEVEL ");
                sql.push_str(level);
            }
            if read_only {
                sql.push_str(" READ ONLY");
            }
            vec![sql]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_ids_and_defaults() {
        for m in TxMode::ALL {
            assert_eq!(TxMode::from_id(m.id()), Some(m));
        }
        assert_eq!(default_mode(Some(EnvTag::Production)), TxMode::Manual);
        assert_eq!(default_mode(Some(EnvTag::Staging)), TxMode::Auto);
        assert_eq!(default_mode(None), TxMode::Auto);
        assert_eq!(TxState::default().status, TxStatus::Idle);
    }

    #[test]
    fn observe_tracks_the_open_transaction() {
        let mut t = TxState::default();
        t.observe(TxStatus::InTx, false);
        assert!(t.since.is_some());
        t.observe(TxStatus::InTx, true);
        t.observe(TxStatus::Failed, true);
        assert_eq!((t.status, t.statements), (TxStatus::Failed, 2));
        t.observe(TxStatus::Idle, false);
        assert_eq!((t.since, t.statements), (None, 0));
    }
}
