// ## m3b-t01c-core-seams
//! The query history (HV-EXEC-12, D-061, R-M3-62; Task 17): SQLite + FTS5 at
//! `LocalPaths::history_db()` (`0600`), one writer on the blocking pool fed by the auditor
//! chain's sink; statements masked once in `prepare_units`; retention per data source and by
//! age; private consoles and a profile's "do not record" drop the event. History is never an
//! audit trail.
//!
//! STUB until Task 17 (fail closed, F31): [`HistoryStore::open`] answers
//! `Err(HistoryError::Disabled)` and nothing is recorded.

mod redact;
mod schema;
mod search;
mod writer;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_proto::ProfileId;
use hacivat_session::session::{RunId, TxMode};
use tokio::sync::mpsc;

/// Where a recorded statement came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HistoryOrigin {
    Console,
    File,
    GridDml,
    Mcp,
    RunConfig,
    Import,
}

/// How a recorded statement ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    Failed,
    Cancelled,
    Denied,
}

/// What a run tells the history about where it ran (`RunRequest::history`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryMeta {
    pub profile: ProfileId,
    pub console: u64,
    /// A private console records nothing.
    pub private: bool,
    pub origin: HistoryOrigin,
}

/// One recorded statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryRecord {
    pub ts_start_ms: i64,
    pub duration_ms: u32,
    pub profile: ProfileId,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub console: u64,
    pub origin: HistoryOrigin,
    /// The statement with its bind values masked, at most `HistoryConfig::statement_cap`.
    pub statement_masked: Arc<str>,
    /// SHA-256 of the statement as sent.
    pub text_hash: [u8; 32],
    /// `HvNorm1` (PARAM-03).
    pub fingerprint: u64,
    pub params_masked: Option<String>,
    pub outcome: Outcome,
    pub rows_affected: Option<u64>,
    pub rows_fetched: Option<u64>,
    pub error_code: Option<String>,
    /// At most 4 KiB.
    pub error_message: Option<String>,
    pub tx_mode: TxMode,
    pub run: Option<RunId>,
    /// Something was masked beyond the bind values.
    pub redacted: bool,
}

/// Retention and caps (D-061).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistoryConfig {
    /// Statements kept per data source.
    pub per_source: u32,
    pub max_age_days: u32,
    /// The most bytes of one statement kept.
    pub statement_cap: usize,
}

impl Default for HistoryConfig {
    /// 10,000 per data source, 90 days, 64 KiB per statement.
    fn default() -> Self {
        HistoryConfig {
            per_source: 10_000,
            max_age_days: 90,
            statement_cap: 64 << 10,
        }
    }
}

/// A history search.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct HistoryQuery {
    /// Full-text, Turkish-tolerant (Task 17).
    pub text: Option<String>,
    pub profile: Option<ProfileId>,
    pub outcome: Option<Outcome>,
    pub since_ms: Option<i64>,
    pub limit: u32,
}

/// One search hit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HistoryHit {
    pub id: i64,
    pub record: HistoryRecord,
    /// How many times the same statement ran.
    pub run_count: u32,
    pub pinned: bool,
}

/// Why the history cannot answer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HistoryError {
    /// The history is off (the setting, or not built yet: the stub).
    Disabled,
    Io(String),
    Corrupt,
}

/// The history database and its one writer (the module docs).
#[derive(Debug)]
pub struct HistoryStore {
    sink: mpsc::Sender<Arc<HistoryRecord>>,
}

impl HistoryStore {
    /// Opens (or creates) the history at `path`. STUB until Task 17:
    /// `Err(HistoryError::Disabled)`, nothing touched on disk.
    pub fn open(
        path: PathBuf,
        cfg: HistoryConfig,
    ) -> BoxFuture<'static, Result<Arc<HistoryStore>, HistoryError>> {
        let _ = (path, cfg);
        Box::pin(async { Err(HistoryError::Disabled) })
    }

    /// Where the auditor chain sends finished statements (`try_send`; R-M3-63).
    pub fn sink(&self) -> mpsc::Sender<Arc<HistoryRecord>> {
        self.sink.clone()
    }

    /// Searches the history. STUB: `Disabled`.
    pub fn search(
        &self,
        q: HistoryQuery,
    ) -> BoxFuture<'static, Result<Vec<HistoryHit>, HistoryError>> {
        let _ = q;
        Box::pin(async { Err(HistoryError::Disabled) })
    }

    /// The `n` newest statements of `profile` starting with `prefix` (the editor's recall).
    /// STUB: `Disabled`.
    pub fn recall(
        &self,
        profile: ProfileId,
        prefix: String,
        n: u32,
    ) -> BoxFuture<'static, Result<Vec<Arc<str>>, HistoryError>> {
        let _ = (profile, prefix, n);
        Box::pin(async { Err(HistoryError::Disabled) })
    }

    /// Pins or unpins hit `id` (a pinned statement outlives retention). STUB: `Disabled`.
    pub fn pin(&self, id: i64, pinned: bool) -> BoxFuture<'static, Result<(), HistoryError>> {
        let _ = (id, pinned);
        Box::pin(async { Err(HistoryError::Disabled) })
    }

    /// Writes what is queued, for at most `grace`, then closes. STUB: nothing to write.
    pub fn shutdown(&self, grace: Duration) -> BoxFuture<'static, ()> {
        let _ = grace;
        Box::pin(async {})
    }
}
