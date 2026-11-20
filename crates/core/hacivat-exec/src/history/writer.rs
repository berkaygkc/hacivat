// ## m3b-t01c-core-seams
//! The history's one writer on the blocking pool (Task 17): batches records from the sink,
//! applies retention, never blocks the actor.
// ## m3b-t17-history
//!
//! One task owns the connection. It waits on the sink and on the store's commands and runs
//! every database call on the blocking pool (`spawn_blocking`, the connection moved in and
//! back), so neither the actor nor the UI thread ever touches the file. Before a command it
//! writes every record already queued, so a search sees what was sent before it. Records are
//! written in batches of up to [`BATCH`] in one `BEGIN IMMEDIATE` transaction (the busy
//! timeout covers another process's writer; WAL keeps readers apart). A batch that fails
//! (disk full, a lock held past the timeout) is dropped, never retried into the actor; a
//! corrupt file stops the writer (the next open moves it aside).
//!
//! Each record is redacted first ([`super::redact`]); the text hash (SHA-256) and the
//! fingerprint (`HvNorm1`, generic dialect) are computed from the redacted text, never taken
//! from the record (a hash of a statement holding a password is a guessing oracle). The
//! statement is capped at `statement_cap` bytes (the hash is of the whole text), the error
//! message at 4 KiB, both at a character boundary. The FTS5 index reads the folded statement
//! (`search_key`) and the folded error message (`error_key`), so a Turkish error text is found
//! the same way as a statement.
//!
//! Retention (D-061): after each batch, the touched data sources keep their newest
//! `per_source` unpinned entries and every entry older than `max_age_days` that is not pinned
//! goes; the same runs at open and every 10 minutes. Pinned entries never expire and do not
//! count against the cap. A limit of 0 is no limit.

use std::collections::{BTreeSet, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use hacivat_proto::ProfileId;
use hacivat_session::session::{RunId, TxMode};
use rusqlite::types::Value;
use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};
use sha2::{Digest, Sha256};
use sql_dialect::{DialectCtx, DialectId};
use sql_split::hv_norm1;
use tokio::sync::{mpsc, oneshot};

use super::redact::{redact, scrub};
use super::schema::{is_corrupt, sql};
use super::search::{fold, match_expr};
use super::{
    HistoryConfig, HistoryError, HistoryHit, HistoryOrigin, HistoryQuery, HistoryRecord, Outcome,
};

/// The most records written in one transaction.
pub(crate) const BATCH: usize = 1_000;
/// How often retention runs without new records.
const RETENTION_EVERY: Duration = Duration::from_secs(10 * 60);
/// The most bytes of an error message kept.
pub(crate) const ERROR_CAP: usize = 4 << 10;
/// What a search answers at most when its query gives no limit.
const DEFAULT_LIMIT: u32 = 200;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What the store asks its writer.
pub(crate) enum Cmd {
    Search(
        HistoryQuery,
        oneshot::Sender<Result<Vec<HistoryHit>, HistoryError>>,
    ),
    Recall {
        profile: ProfileId,
        prefix: String,
        n: u32,
        reply: oneshot::Sender<Result<Vec<Arc<str>>, HistoryError>>,
    },
    Pin {
        id: i64,
        pinned: bool,
        reply: oneshot::Sender<Result<(), HistoryError>>,
    },
    // ## m4-t32-shots (named exception; PO R-W5-14 b)
    Clear {
        profile: ProfileId,
        reply: oneshot::Sender<Result<u64, HistoryError>>,
    },
    Shutdown(oneshot::Sender<()>),
}

/// The profiles whose statements are not recorded ("do not record history").
pub(crate) type Excluded = Arc<Mutex<HashSet<ProfileId>>>;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The longest prefix of `s` of at most `cap` bytes that ends on a character boundary.
fn cap_at(s: &str, cap: usize) -> &str {
    if s.len() <= cap {
        return s;
    }
    let mut end = cap;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// One record as it is stored.
struct Row {
    ts: i64,
    duration_ms: u32,
    profile: String,
    database: Option<String>,
    schema: Option<String>,
    console: u64,
    origin: HistoryOrigin,
    statement: String,
    search_key: String,
    text_hash: [u8; 32],
    fingerprint: u64,
    params: Option<String>,
    outcome: Outcome,
    rows_affected: Option<u64>,
    rows_fetched: Option<u64>,
    error_code: Option<String>,
    error_message: Option<String>,
    error_key: Option<String>,
    tx_mode: TxMode,
    run: Option<RunId>,
    redacted: bool,
}

impl Row {
    fn of(r: &HistoryRecord, cfg: &HistoryConfig) -> Row {
        let red = redact(&r.statement_masked);
        let text_hash: [u8; 32] = Sha256::digest(red.text.as_bytes()).into();
        let fingerprint = hv_norm1(&red.text, &DialectCtx::new(DialectId::Generic, None)).hash;
        let cap = if cfg.statement_cap == 0 {
            usize::MAX
        } else {
            cfg.statement_cap
        };
        let statement = cap_at(&red.text, cap).to_owned();
        let mut redacted = r.redacted || red.changed;
        let mut clean = |t: &Option<String>, cap: usize| {
            t.as_deref().map(|t| {
                let (s, changed) = scrub(t, &red.secrets);
                redacted |= changed;
                cap_at(&s, cap).to_owned()
            })
        };
        let error_message = clean(&r.error_message, ERROR_CAP);
        let params = clean(&r.params_masked, cap);
        let error_code = clean(&r.error_code, ERROR_CAP);
        Row {
            ts: r.ts_start_ms,
            duration_ms: r.duration_ms,
            profile: r.profile.to_string(),
            database: r
                .database
                .as_deref()
                .map(|s| cap_at(s, ERROR_CAP).to_owned()),
            schema: r.schema.as_deref().map(|s| cap_at(s, ERROR_CAP).to_owned()),
            console: r.console,
            origin: r.origin,
            search_key: fold(&statement),
            statement,
            text_hash,
            fingerprint,
            params,
            outcome: r.outcome,
            rows_affected: r.rows_affected,
            rows_fetched: r.rows_fetched,
            error_code,
            error_key: error_message.as_deref().map(fold),
            error_message,
            tx_mode: r.tx_mode,
            run: r.run,
            redacted,
        }
    }
}

fn origin_str(o: HistoryOrigin) -> &'static str {
    match o {
        HistoryOrigin::Console => "console",
        HistoryOrigin::File => "file",
        HistoryOrigin::GridDml => "grid_dml",
        HistoryOrigin::Mcp => "mcp",
        HistoryOrigin::RunConfig => "run_config",
        HistoryOrigin::Import => "import",
    }
}

fn origin_of(s: &str) -> HistoryOrigin {
    match s {
        "file" => HistoryOrigin::File,
        "grid_dml" => HistoryOrigin::GridDml,
        "mcp" => HistoryOrigin::Mcp,
        "run_config" => HistoryOrigin::RunConfig,
        "import" => HistoryOrigin::Import,
        _ => HistoryOrigin::Console,
    }
}

fn outcome_str(o: Outcome) -> &'static str {
    match o {
        Outcome::Ok => "ok",
        Outcome::Failed => "failed",
        Outcome::Cancelled => "cancelled",
        Outcome::Denied => "denied",
    }
}

fn outcome_of(s: &str) -> Outcome {
    match s {
        "ok" => Outcome::Ok,
        "cancelled" => Outcome::Cancelled,
        "denied" => Outcome::Denied,
        _ => Outcome::Failed,
    }
}

fn tx_mode_str(m: TxMode) -> &'static str {
    match m {
        TxMode::Auto => "auto",
        TxMode::Manual => "manual",
        TxMode::Smart => "smart",
    }
}

fn tx_mode_of(s: &str) -> TxMode {
    match s {
        "manual" => TxMode::Manual,
        "smart" => TxMode::Smart,
        _ => TxMode::Auto,
    }
}

fn opt_i64(v: Option<u64>) -> Option<i64> {
    v.map(|v| i64::try_from(v).unwrap_or(i64::MAX))
}

/// The writer's connection and settings.
pub(crate) struct Db {
    conn: Connection,
    cfg: HistoryConfig,
    /// The file turned out corrupt: nothing more is written or read.
    broken: bool,
}

const UPSERT: &str = "
INSERT INTO history(ts_start, last_run_ts, duration_ms, data_source_id, database, schema,
  client_id, origin, statement_text, search_key, text_hash, fingerprint, params_json, outcome,
  rows_affected, rows_fetched, error_code, error_message, error_key, tx_mode, redacted,
  script_run_id)
VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
  ?19, ?20, ?21)
ON CONFLICT(data_source_id, text_hash) DO UPDATE SET
  last_run_ts = excluded.last_run_ts, duration_ms = excluded.duration_ms,
  database = excluded.database, schema = excluded.schema, client_id = excluded.client_id,
  origin = excluded.origin, params_json = excluded.params_json, outcome = excluded.outcome,
  rows_affected = excluded.rows_affected, rows_fetched = excluded.rows_fetched,
  error_code = excluded.error_code, error_message = excluded.error_message,
  error_key = excluded.error_key, tx_mode = excluded.tx_mode, redacted = max(redacted, excluded.redacted),
  script_run_id = excluded.script_run_id, run_count = run_count + 1";

const COLUMNS: &str = "h.id, h.last_run_ts, h.duration_ms, h.data_source_id, h.database, \
  h.schema, h.client_id, h.origin, h.statement_text, h.text_hash, h.fingerprint, h.params_json, \
  h.outcome, h.rows_affected, h.rows_fetched, h.error_code, h.error_message, h.tx_mode, \
  h.script_run_id, h.redacted, h.run_count, h.pinned";

impl Db {
    pub(crate) fn new(conn: Connection, cfg: HistoryConfig) -> Self {
        Db {
            conn,
            cfg,
            broken: false,
        }
    }

    fn note(&mut self, e: &rusqlite::Error) {
        if is_corrupt(e) {
            self.broken = true;
        }
    }

    /// Writes `records` (those of excluded profiles dropped) and applies retention to the
    /// data sources they touched. A failed batch is dropped.
    pub(crate) fn write(&mut self, records: &[Arc<HistoryRecord>], excluded: &HashSet<ProfileId>) {
        if self.broken {
            return;
        }
        let rows: Vec<Row> = records
            .iter()
            .filter(|r| !excluded.contains(&r.profile))
            .map(|r| Row::of(r, &self.cfg))
            .collect();
        if rows.is_empty() {
            return;
        }
        if let Err(e) = self.upsert(&rows) {
            self.note(&e);
            return;
        }
        let touched: BTreeSet<&str> = rows.iter().map(|r| r.profile.as_str()).collect();
        let r = touched
            .into_iter()
            .try_for_each(|p| self.cap_source(p))
            .and_then(|()| self.purge_age());
        if let Err(e) = r {
            self.note(&e);
        }
    }

    fn upsert(&mut self, rows: &[Row]) -> rusqlite::Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut st = tx.prepare_cached(UPSERT)?;
            for r in rows {
                st.execute(params![
                    r.ts,
                    r.duration_ms,
                    r.profile,
                    r.database,
                    r.schema,
                    r.console as i64,
                    origin_str(r.origin),
                    r.statement,
                    r.search_key,
                    &r.text_hash[..],
                    &r.fingerprint.to_be_bytes()[..],
                    r.params,
                    outcome_str(r.outcome),
                    opt_i64(r.rows_affected),
                    opt_i64(r.rows_fetched),
                    r.error_code,
                    r.error_message,
                    r.error_key,
                    tx_mode_str(r.tx_mode),
                    r.redacted,
                    r.run.map(|r| r.0 as i64),
                ])?;
            }
        }
        tx.commit()
    }

    /// Keeps the newest `per_source` unpinned entries of `profile`.
    fn cap_source(&self, profile: &str) -> rusqlite::Result<()> {
        if self.cfg.per_source == 0 {
            return Ok(());
        }
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM history WHERE data_source_id = ?1 AND pinned = 0",
            [profile],
            |r| r.get(0),
        )?;
        let mut excess = n - i64::from(self.cfg.per_source);
        while excess > 0 {
            let k = excess.min(BATCH as i64);
            self.conn.execute(
                "DELETE FROM history WHERE id IN (SELECT id FROM history
                   WHERE data_source_id = ?1 AND pinned = 0
                   ORDER BY last_run_ts ASC, id ASC LIMIT ?2)",
                params![profile, k],
            )?;
            excess -= k;
        }
        Ok(())
    }

    /// Removes unpinned entries last run more than `max_age_days` ago.
    fn purge_age(&self) -> rusqlite::Result<()> {
        if self.cfg.max_age_days == 0 {
            return Ok(());
        }
        let cutoff = now_ms().saturating_sub(i64::from(self.cfg.max_age_days) * DAY_MS);
        loop {
            let n = self.conn.execute(
                "DELETE FROM history WHERE id IN (SELECT id FROM history
                   WHERE pinned = 0 AND last_run_ts < ?1 LIMIT ?2)",
                params![cutoff, BATCH as i64],
            )?;
            if n < BATCH {
                return Ok(());
            }
        }
    }

    /// Every data source over its cap, and the age limit (at open and every 10 minutes).
    pub(crate) fn retention(&mut self) {
        if self.broken {
            return;
        }
        let r = (|| {
            let over: Vec<String> = {
                let mut st = self.conn.prepare(
                    "SELECT data_source_id FROM history WHERE pinned = 0
                     GROUP BY data_source_id HAVING count(*) > ?1",
                )?;
                let per = if self.cfg.per_source == 0 {
                    i64::MAX
                } else {
                    i64::from(self.cfg.per_source)
                };
                st.query_map([per], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            for p in &over {
                self.cap_source(p)?;
            }
            self.purge_age()?;
            self.conn.execute_batch("PRAGMA incremental_vacuum")
        })();
        if let Err(e) = r {
            self.note(&e);
        }
    }

    pub(crate) fn search(&self, q: &HistoryQuery) -> Result<Vec<HistoryHit>, HistoryError> {
        if self.broken {
            return Err(HistoryError::Corrupt);
        }
        let mut args: Vec<Value> = Vec::new();
        let mut sql_text = String::new();
        let fts = match q.text.as_deref().filter(|t| !t.trim().is_empty()) {
            None => None,
            Some(t) => match match_expr(t) {
                // Text with no word in it matches nothing.
                None => return Ok(Vec::new()),
                Some(e) => Some(e),
            },
        };
        let has_fts = fts.is_some();
        if let Some(expr) = fts {
            sql_text.push_str(&format!(
                "SELECT {COLUMNS} FROM history_fts JOIN history h ON h.id = history_fts.rowid \
                 WHERE history_fts MATCH ?"
            ));
            args.push(Value::Text(expr));
        } else {
            sql_text.push_str(&format!("SELECT {COLUMNS} FROM history h WHERE 1 = 1"));
        }
        if let Some(p) = &q.profile {
            sql_text.push_str(" AND h.data_source_id = ?");
            args.push(Value::Text(p.to_string()));
        }
        if let Some(o) = q.outcome {
            sql_text.push_str(" AND h.outcome = ?");
            args.push(Value::Text(outcome_str(o).into()));
        }
        if let Some(since) = q.since_ms {
            sql_text.push_str(" AND h.last_run_ts >= ?");
            args.push(Value::Integer(since));
        }
        if let Some(fp) = q.fingerprint {
            sql_text.push_str(" AND h.fingerprint = ?");
            args.push(Value::Blob(fp.to_be_bytes().to_vec()));
        }
        if has_fts {
            sql_text.push_str(" ORDER BY bm25(history_fts), h.last_run_ts DESC, h.id DESC");
        } else {
            sql_text.push_str(" ORDER BY h.last_run_ts DESC, h.id DESC");
        }
        sql_text.push_str(" LIMIT ?");
        let limit = if q.limit == 0 { DEFAULT_LIMIT } else { q.limit };
        args.push(Value::Integer(i64::from(limit)));

        let run = || -> rusqlite::Result<Vec<HistoryHit>> {
            let mut st = self.conn.prepare(&sql_text)?;
            let rows = st.query_map(params_from_iter(args.iter()), hit_of)?;
            let mut out = Vec::new();
            for r in rows {
                if let Some(h) = r? {
                    out.push(h);
                }
            }
            Ok(out)
        };
        match run() {
            Ok(h) => Ok(h),
            // FTS5 refusing an expression ("fts5: syntax error …") is "no match", never an
            // error the user sees. `match_expr` should never build one. Any other FTS5 error
            // (an index out of step with its table) is reported.
            Err(rusqlite::Error::SqliteFailure(_, Some(m)))
                if m.starts_with("fts5: syntax error") =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(sql(e)),
        }
    }

    pub(crate) fn recall(
        &self,
        profile: &ProfileId,
        prefix: &str,
        n: u32,
    ) -> Result<Vec<Arc<str>>, HistoryError> {
        if self.broken {
            return Err(HistoryError::Corrupt);
        }
        let run = || -> rusqlite::Result<Vec<Arc<str>>> {
            let mut st = self.conn.prepare_cached(
                "SELECT statement_text FROM history
                 WHERE data_source_id = ?1 AND substr(statement_text, 1, length(?2)) = ?2
                 ORDER BY last_run_ts DESC, id DESC LIMIT ?3",
            )?;
            st.query_map(params![profile.to_string(), prefix, i64::from(n)], |r| {
                r.get::<_, String>(0).map(Arc::from)
            })?
            .collect()
        };
        run().map_err(sql)
    }

    pub(crate) fn pin(&self, id: i64, pinned: bool) -> Result<(), HistoryError> {
        if self.broken {
            return Err(HistoryError::Corrupt);
        }
        self.conn
            .execute(
                "UPDATE history SET pinned = ?2 WHERE id = ?1",
                params![id, pinned],
            )
            .map(drop)
            .map_err(sql)
    }

    // ## m4-t32-shots (named exception; PO R-W5-14 b)
    /// Deletes every entry of `profile`, pinned ones too (the full-text index follows through
    /// its delete trigger); answers how many were deleted.
    pub(crate) fn clear(&self, profile: &ProfileId) -> Result<u64, HistoryError> {
        if self.broken {
            return Err(HistoryError::Corrupt);
        }
        self.conn
            .execute(
                "DELETE FROM history WHERE data_source_id = ?1",
                params![profile.to_string()],
            )
            .map(|n| n as u64)
            .map_err(sql)
    }

    /// Folds the log into the file (at shutdown).
    fn checkpoint(&self) {
        let _ = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }
}

fn hit_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Option<HistoryHit>> {
    let Ok(profile) = r.get::<_, String>(3)?.parse::<ProfileId>() else {
        return Ok(None);
    };
    let blob = |i: usize| -> rusqlite::Result<Vec<u8>> {
        Ok(r.get::<_, Option<Vec<u8>>>(i)?.unwrap_or_default())
    };
    let hash = blob(9)?;
    let fp = blob(10)?;
    let text_hash: [u8; 32] = hash.as_slice().try_into().unwrap_or([0; 32]);
    let fingerprint = <[u8; 8]>::try_from(fp.as_slice()).map_or(0, u64::from_be_bytes);
    let u64_of = |v: Option<i64>| v.and_then(|v| u64::try_from(v).ok());
    let record = HistoryRecord {
        ts_start_ms: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
        duration_ms: u32::try_from(r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0))
            .unwrap_or(u32::MAX),
        profile,
        database: r.get(4)?,
        schema: r.get(5)?,
        console: r.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
        origin: origin_of(&r.get::<_, Option<String>>(7)?.unwrap_or_default()),
        statement_masked: Arc::from(r.get::<_, String>(8)?),
        text_hash,
        fingerprint,
        params_masked: r.get(11)?,
        outcome: outcome_of(&r.get::<_, Option<String>>(12)?.unwrap_or_default()),
        rows_affected: u64_of(r.get(13)?),
        rows_fetched: u64_of(r.get(14)?),
        error_code: r.get(15)?,
        error_message: r.get(16)?,
        tx_mode: tx_mode_of(&r.get::<_, Option<String>>(17)?.unwrap_or_default()),
        run: r.get::<_, Option<i64>>(18)?.map(|v| RunId(v as u64)),
        redacted: r.get::<_, Option<bool>>(19)?.unwrap_or(false),
    };
    Ok(Some(HistoryHit {
        id: r.get(0)?,
        record,
        run_count: u32::try_from(r.get::<_, i64>(20)?.max(0)).unwrap_or(u32::MAX),
        pinned: r.get(21)?,
    }))
}

/// Runs `f` on the blocking pool with the connection; `None` if it panicked (the writer then
/// stops, and the store answers `Disabled`).
async fn blocking<T: Send + 'static>(
    db: Box<Db>,
    f: impl FnOnce(&mut Db) -> T + Send + 'static,
) -> Option<(Box<Db>, T)> {
    tokio::task::spawn_blocking(move || {
        let mut db = db;
        let t = f(&mut db);
        (db, t)
    })
    .await
    .ok()
}

/// Checkpoints and closes the file on the blocking pool.
async fn close(db: Box<Db>) {
    let _ = tokio::task::spawn_blocking(move || {
        db.checkpoint();
        drop(db);
    })
    .await;
}

enum Event {
    Cmd(Option<Cmd>),
    Record(Option<Arc<HistoryRecord>>),
    Tick,
}

/// The writer task (the module docs).
pub(crate) async fn run(
    db: Box<Db>,
    mut records: mpsc::Receiver<Arc<HistoryRecord>>,
    mut cmds: mpsc::Receiver<Cmd>,
    excluded: Excluded,
) {
    let mut db = Some(db);
    let mut records_open = true;
    let mut next_tick = tokio::time::Instant::now() + RETENTION_EVERY;
    while let Some(d) = db.take() {
        let ev = {
            let cmd = cmds.recv().fuse();
            let rec = async {
                if records_open {
                    records.recv().await
                } else {
                    futures::future::pending().await
                }
            }
            .fuse();
            let tick = tokio::time::sleep_until(next_tick).fuse();
            futures::pin_mut!(cmd, rec, tick);
            futures::select_biased! {
                c = cmd => Event::Cmd(c),
                r = rec => Event::Record(r),
                () = tick => Event::Tick,
            }
        };
        // A poisoned lock still holds the set: never record an excluded profile.
        let skip: HashSet<ProfileId> = excluded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match ev {
            Event::Record(None) => {
                records_open = false;
                db = Some(d);
            }
            Event::Record(Some(first)) => {
                let mut batch = vec![first];
                while batch.len() < BATCH {
                    match records.try_recv() {
                        Ok(r) => batch.push(r),
                        Err(_) => break,
                    }
                }
                db = blocking(d, move |db| db.write(&batch, &skip))
                    .await
                    .map(|(d, ())| d);
            }
            Event::Tick => {
                next_tick = tokio::time::Instant::now() + RETENTION_EVERY;
                db = blocking(d, Db::retention).await.map(|(d, ())| d);
            }
            Event::Cmd(cmd) => {
                // Everything sent before the command is written first.
                let mut queued = Vec::new();
                while let Ok(r) = records.try_recv() {
                    queued.push(r);
                }
                let Some((d, ())) = blocking(d, move |db| {
                    for chunk in queued.chunks(BATCH) {
                        db.write(chunk, &skip);
                    }
                })
                .await
                else {
                    return;
                };
                match cmd {
                    None => {
                        // The store is gone.
                        close(d).await;
                        return;
                    }
                    Some(Cmd::Shutdown(ack)) => {
                        close(d).await;
                        let _ = ack.send(());
                        return;
                    }
                    Some(Cmd::Search(q, reply)) => {
                        db = blocking(d, move |db| {
                            let _ = reply.send(db.search(&q));
                        })
                        .await
                        .map(|(d, ())| d);
                    }
                    Some(Cmd::Recall {
                        profile,
                        prefix,
                        n,
                        reply,
                    }) => {
                        db = blocking(d, move |db| {
                            let _ = reply.send(db.recall(&profile, &prefix, n));
                        })
                        .await
                        .map(|(d, ())| d);
                    }
                    Some(Cmd::Pin { id, pinned, reply }) => {
                        db = blocking(d, move |db| {
                            let _ = reply.send(db.pin(id, pinned));
                        })
                        .await
                        .map(|(d, ())| d);
                    }
                    // ## m4-t32-shots (named exception; PO R-W5-14 b)
                    Some(Cmd::Clear { profile, reply }) => {
                        db = blocking(d, move |db| {
                            let _ = reply.send(db.clear(&profile));
                        })
                        .await
                        .map(|(d, ())| d);
                    }
                }
            }
        }
    }
}
