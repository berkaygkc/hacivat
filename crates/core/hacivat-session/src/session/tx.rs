//! Transaction state and modes (HV-SESS-03, HV-EXEC-09/-10; Task 7).

use std::time::Instant;

use hacivat_driver::TxStatus;
use hacivat_proto::{DbmsKind, EnvTag};

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
}

/// The transaction mode a profile starts with: Manual on production, else Auto.
pub fn default_mode(env: Option<EnvTag>) -> TxMode {
    match env {
        Some(EnvTag::Production) => TxMode::Manual,
        _ => TxMode::Auto,
    }
}

/// The statements that begin a transaction on `kind` at `iso`. Task 1 stub: none (Task 7).
pub fn begin_sql(_kind: DbmsKind, _iso: IsolationLevel, _read_only: bool) -> Vec<String> {
    Vec::new()
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
}
