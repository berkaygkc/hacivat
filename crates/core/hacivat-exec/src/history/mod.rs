// ## m3b-t01c-core-seams
//! The query history (HV-EXEC-12, D-061, R-M3-62; Task 17): SQLite + FTS5 at
//! `LocalPaths::history_db()` (`0600`), one writer on the blocking pool fed by the auditor
//! chain's sink; statements masked once in `prepare_units`; retention per data source and by
//! age; private consoles and a profile's "do not record" drop the event. History is never an
//! audit trail.
// ## m3b-t17-history
//!
//! Bodies since Task 17 (ADR 0006 `m3b-t17`). The store is a convenience for finding a
//! statement again: it drops records when it is full, busy or broken, and the user may delete
//! it at any time. Nothing may rely on it as a record of what ran (that is Karagöz's audit
//! log, M8).
//!
//! - **What is kept.** The writer masks secrets in the statement before anything else reads
//!   it (`PASSWORD '…'`, `IDENTIFIED BY …`, connection strings, psql meta-command arguments
//!   through `HvNorm1`), and removes the same secrets from the error message and the parameter
//!   text. Bind values never arrive (the run service sends `PreparedUnit::masked`).
//! - **Who is not recorded.** A private console's runs never reach the sink
//!   (`HistoryMeta::private`, the run service's check), and the writer drops the records of a
//!   profile marked "do not record history" as well ([`HistoryStore::set_do_not_record`]).
//! - **Threads.** [`HistoryStore::open`] and every call run on the blocking pool; the futures
//!   only wait on channels, so the UI thread may await them.

mod redact;
mod schema;
mod search;
mod writer;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_proto::ProfileId;
use hacivat_session::session::{RunId, TxMode};
use tokio::sync::{mpsc, oneshot};

use crate::audit::SINK_CAPACITY;
use writer::{Cmd, Db, Excluded};

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
    /// SHA-256 of the redacted statement, all of it (Task 17: the store computes it; a value
    /// sent in is ignored).
    pub text_hash: [u8; 32],
    /// `HvNorm1` (PARAM-03) of the redacted statement, generic dialect (Task 17: the store
    /// computes it; a value sent in is ignored).
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

/// Retention and caps (D-061). A limit of 0 is no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HistoryConfig {
    /// Unpinned statements kept per data source (pinned ones are kept besides).
    pub per_source: u32,
    /// Unpinned statements last run longer ago than this go.
    pub max_age_days: u32,
    /// The most bytes of one statement kept (a prefix ending on a character boundary; the
    /// text hash is of the whole statement).
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

/// A history search. Hits come newest first; with `text`, best match first, then newest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct HistoryQuery {
    /// Full-text, Turkish-tolerant (Task 17): every word must start a word of the statement
    /// or its error message. Words are letters and digits; anything else only separates them
    /// (the text is never FTS5 syntax). Text with no word in it matches nothing.
    pub text: Option<String>,
    pub profile: Option<ProfileId>,
    pub outcome: Option<Outcome>,
    /// Last run at or after this time.
    pub since_ms: Option<i64>,
    /// The most hits; 0 is 200.
    pub limit: u32,
    // ## m3b-t17-history
    /// "Similar": the entries with this fingerprint (`HistoryRecord::fingerprint`).
    pub fingerprint: Option<u64>,
}

/// One search hit: the entry's latest run (`record.ts_start_ms` is when it last started).
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
pub struct HistoryStore {
    sink: mpsc::Sender<Arc<HistoryRecord>>,
    cmd: mpsc::Sender<Cmd>,
    excluded: Excluded,
    recovered: Option<PathBuf>,
}

impl std::fmt::Debug for HistoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryStore")
            .field("recovered", &self.recovered.is_some())
            .finish_non_exhaustive()
    }
}

impl HistoryStore {
    /// Opens (or creates) the history at `path`, on the blocking pool. The directory must
    /// exist (the app creates it private). A corrupt file is moved aside first
    /// ([`HistoryStore::recovered_from`]); retention runs before the store answers. Must be
    /// polled inside the core runtime.
    pub fn open(
        path: PathBuf,
        cfg: HistoryConfig,
    ) -> BoxFuture<'static, Result<Arc<HistoryStore>, HistoryError>> {
        Box::pin(async move {
            let (db, recovered) = tokio::task::spawn_blocking(move || {
                schema::open(&path).map(|o| {
                    let mut db = Db::new(o.conn, cfg);
                    db.retention();
                    (Box::new(db), o.recovered)
                })
            })
            .await
            .map_err(|_| HistoryError::Io("the history could not be opened".into()))??;
            let (sink, records) = mpsc::channel(SINK_CAPACITY);
            let (cmd, cmds) = mpsc::channel(64);
            let excluded: Excluded = Arc::new(Mutex::new(HashSet::new()));
            tokio::spawn(writer::run(db, records, cmds, excluded.clone()));
            Ok(Arc::new(HistoryStore {
                sink,
                cmd,
                excluded,
                recovered,
            }))
        })
    }

    /// Where the auditor chain sends finished statements (`try_send`; R-M3-63). The writer
    /// redacts, caps and deduplicates them.
    pub fn sink(&self) -> mpsc::Sender<Arc<HistoryRecord>> {
        self.sink.clone()
    }

    // ## m3b-t17-history
    /// Where the corrupt file found at open was moved (`history.sqlite.corrupt-<ts>`), so the
    /// app can tell the user once; `None` when the file was healthy.
    pub fn recovered_from(&self) -> Option<&Path> {
        self.recovered.as_deref()
    }

    // ## m3b-t17-history
    /// Marks `profile` "do not record history" (`Profile.record_history == false`): from now
    /// on the writer drops its statements, besides the run service never sending them.
    /// Entries already kept stay.
    pub fn set_do_not_record(&self, profile: ProfileId, on: bool) {
        let mut set = self
            .excluded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if on {
            set.insert(profile);
        } else {
            set.remove(&profile);
        }
    }

    async fn ask<T>(
        cmd: mpsc::Sender<Cmd>,
        make: impl FnOnce(oneshot::Sender<Result<T, HistoryError>>) -> Cmd,
    ) -> Result<T, HistoryError> {
        let (tx, rx) = oneshot::channel();
        cmd.send(make(tx))
            .await
            .map_err(|_| HistoryError::Disabled)?;
        rx.await.map_err(|_| HistoryError::Disabled)?
    }

    /// Searches the history (records sent before the call included). `Disabled` after
    /// [`HistoryStore::shutdown`]; `Corrupt` once the file turned out damaged.
    pub fn search(
        &self,
        q: HistoryQuery,
    ) -> BoxFuture<'static, Result<Vec<HistoryHit>, HistoryError>> {
        let cmd = self.cmd.clone();
        Box::pin(Self::ask(cmd, move |reply| Cmd::Search(q, reply)))
    }

    /// The `n` newest statements of `profile` starting with `prefix` (the editor's recall):
    /// an exact, case-sensitive text prefix (never a pattern); the empty prefix recalls the
    /// newest.
    pub fn recall(
        &self,
        profile: ProfileId,
        prefix: String,
        n: u32,
    ) -> BoxFuture<'static, Result<Vec<Arc<str>>, HistoryError>> {
        let cmd = self.cmd.clone();
        Box::pin(Self::ask(cmd, move |reply| Cmd::Recall {
            profile,
            prefix,
            n,
            reply,
        }))
    }

    /// Pins or unpins hit `id` (a pinned statement outlives retention). An unknown id is
    /// not an error.
    pub fn pin(&self, id: i64, pinned: bool) -> BoxFuture<'static, Result<(), HistoryError>> {
        let cmd = self.cmd.clone();
        Box::pin(Self::ask(cmd, move |reply| Cmd::Pin { id, pinned, reply }))
    }

    /// Writes what is queued, for at most `grace`, then closes the file; later calls answer
    /// `Disabled` and later records are dropped.
    pub fn shutdown(&self, grace: Duration) -> BoxFuture<'static, ()> {
        let cmd = self.cmd.clone();
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            let work = async move {
                if cmd.send(Cmd::Shutdown(tx)).await.is_ok() {
                    let _ = rx.await;
                }
            };
            let _ = tokio::time::timeout(grace, work).await;
        })
    }
}
