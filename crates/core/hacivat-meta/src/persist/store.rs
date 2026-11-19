//! [`CacheStore`]: the SQLite file of one profile (alg/02 HV-MODEL-03 steps 1–7).
//!
//! Tables: `header` (format, mode, key check, server fingerprint), `namespace` (one row per
//! namespace: lookup tag, key, minor, generation, writer, watermark, stale flag, payload),
//! `usage` (the LRU clock, apart from the payload rows so a load never rewrites a payload) and
//! `source` (content-addressed source texts). In an encrypted store every column that could
//! carry a name is sealed, and `tag` columns are keyed lookup tags.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use super::crypt::{CATALOG_DOMAIN, RowCipher};
use super::{
    CacheError, Encryption, IdentityCheck, LoadAll, Opened, RebuildReason, RowError,
    ServerFingerprint, WriterId, files,
};
use crate::codec::{
    self, Decoded, FORMAT_MAJOR, FORMAT_MINOR, decode_ns_key, decode_watermark, encode_ns_key,
    encode_watermark,
};
use crate::ids::{NsKey, SourceRef};
use crate::namespace::{NamespaceSnapshot, NsMeta, Watermark};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DDL: &str = "
CREATE TABLE IF NOT EXISTS header(key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS namespace(
    tag BLOB PRIMARY KEY,
    ns_key BLOB NOT NULL,
    minor INTEGER NOT NULL,
    generation INTEGER NOT NULL,
    writer_id INTEGER NOT NULL,
    watermark BLOB NOT NULL,
    stale INTEGER NOT NULL,
    payload BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS usage(tag BLOB PRIMARY KEY, last_used INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS source(tag BLOB PRIMARY KEY, len INTEGER NOT NULL, text BLOB NOT NULL);
";

// The row type that starts every piece of associated data, so no sealed column can stand in
// for another.
const AD_PAYLOAD: u8 = 1;
const AD_NS_KEY: u8 = 2;
const AD_WATERMARK: u8 = 3;
const AD_SOURCE: u8 = 4;
const AD_KEY_CHECK: u8 = 5;
const AD_SERVER: u8 = 6;
const KEY_CHECK: &[u8] = b"hv-catalog-key-check";

fn ad(kind: u8, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = vec![kind];
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// A payload's associated data: `1 ‖ ns_key ‖ minor` (the minor is fixed-width at the end, so
/// the split is unambiguous).
fn payload_ad(key_bytes: &[u8], minor: u16) -> Vec<u8> {
    ad(AD_PAYLOAD, &[key_bytes, &minor.to_le_bytes()])
}

fn source_id(r: SourceRef) -> [u8; 20] {
    let mut id = [0u8; 20];
    id[..16].copy_from_slice(&r.hash.to_le_bytes());
    id[16..].copy_from_slice(&r.len.to_le_bytes());
    id
}

/// A test fault (feature `test-support`).
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// `save_namespace` aborts the process after writing its row, before `COMMIT` returns
    /// (the kill -9 of HV-MODEL-03 test 1; run it in a child process).
    AbortBeforeCommit,
}

/// One namespace row's bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowInfo {
    /// How many times the row was written (1 after the first save).
    pub generation: u64,
    pub writer: WriterId,
    pub minor: u16,
    /// Marked by a server change: the namespace loads with `Watermark::None`.
    pub stale: bool,
}

/// What [`CacheStore::evict`] removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evicted {
    /// The evicted namespaces, least recently used first (`None`: an unreadable row).
    pub evicted: Vec<Option<NsKey>>,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// An eviction candidate: (key readable, last used, tag, key, bytes); sorted, unreadable rows
/// and then the least recently used come first.
type EvictRow = (bool, i64, Vec<u8>, Option<NsKey>, u64);

enum Fail {
    Corrupt,
    Major,
    Other(CacheError),
}

fn classify(e: rusqlite::Error) -> Fail {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => Fail::Corrupt,
        _ => Fail::Other(sql_err(e)),
    }
}

fn sql_err(e: rusqlite::Error) -> CacheError {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => CacheError::Locked,
        _ => CacheError::Sqlite(e),
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn blob<'r>(v: ValueRef<'r>) -> Option<&'r [u8]> {
    match v {
        ValueRef::Blob(b) => Some(b),
        _ => None,
    }
}

fn int(v: ValueRef<'_>) -> Option<i64> {
    match v {
        ValueRef::Integer(i) => Some(i),
        _ => None,
    }
}

/// Whether a save with watermark `new` may replace another writer's row at `old` (HV-MODEL-03
/// step 5): never with an older watermark of the same server; a watermark of another server
/// (or kind) is not comparable and wins; a row without a watermark always loses; a save
/// without one never replaces a row that has one.
fn not_older(new: &Watermark, old: &Watermark) -> bool {
    match (new, old) {
        (_, Watermark::None) => true,
        (Watermark::None, _) => false,
        (Watermark::PgXid { w: a, identity: ia }, Watermark::PgXid { w: b, identity: ib }) => {
            ia.system_identifier != ib.system_identifier || a >= b
        }
        (
            Watermark::MyTime {
                at_utc_micros: a,
                server_uuid: ua,
                ..
            },
            Watermark::MyTime {
                at_utc_micros: b,
                server_uuid: ub,
                ..
            },
        ) => ua != ub || a >= b,
        (
            Watermark::Sqlite {
                schema_version: a,
                file_identity: fa,
                ..
            },
            Watermark::Sqlite {
                schema_version: b,
                file_identity: fb,
                ..
            },
        ) => fa != fb || a >= b,
        _ => true,
    }
}

/// One profile's catalog cache. `Send`, not shared: one owner on a blocking thread.
pub struct CacheStore {
    conn: Connection,
    path: PathBuf,
    cipher: Option<RowCipher>,
    writer: WriterId,
    /// The format minor this store reads and writes as (`FORMAT_MINOR`; tests simulate other
    /// builds).
    minor: u16,
    /// Namespaces read from a newer minor, by the metadata they were read with: a save of that
    /// same namespace is refused (a fresh introspection has new metadata and is saved).
    newer: HashMap<NsKey, NsMeta>,
    full_warned: bool,
    #[cfg(feature = "test-support")]
    fault: Option<Fault>,
}

impl fmt::Debug for CacheStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheStore")
            .field("path", &self.path)
            .field("encrypted", &self.cipher.is_some())
            .field("writer", &self.writer)
            .finish()
    }
}

enum WriteFail {
    Sql(rusqlite::Error),
    Superseded,
}

impl From<rusqlite::Error> for WriteFail {
    fn from(e: rusqlite::Error) -> Self {
        WriteFail::Sql(e)
    }
}

/// The columns of one row to write.
struct RowWrite<'a> {
    tag: &'a [u8],
    key_bytes: &'a [u8],
    ns_key: &'a [u8],
    minor: u16,
    writer: WriterId,
    watermark: &'a Watermark,
    watermark_col: &'a [u8],
    payload: &'a [u8],
    abort_before_commit: bool,
}

fn next_use(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(last_used), 0) + 1 FROM usage",
        [],
        |r| r.get(0),
    )
}

/// The stored watermark of another writer's row (unreadable reads as `None`, which loses).
fn stored_watermark(cipher: Option<&RowCipher>, key_bytes: &[u8], col: &[u8]) -> Watermark {
    let plain = match cipher {
        None => Some(col.to_vec()),
        Some(c) => c.open(&ad(AD_WATERMARK, &[key_bytes]), col).ok(),
    };
    plain
        .and_then(|b| decode_watermark(&b).ok())
        .unwrap_or_default()
}

fn write_row(
    conn: &mut Connection,
    cipher: Option<&RowCipher>,
    w: &RowWrite<'_>,
) -> Result<u64, WriteFail> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(i64, i64, Vec<u8>)> = tx
        .query_row(
            "SELECT writer_id, generation, watermark FROM namespace WHERE tag = ?1",
            [w.tag],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let generation = match &existing {
        None => 1,
        Some((writer, generation, wm)) => {
            if *writer != w.writer.0 as i64
                && !not_older(w.watermark, &stored_watermark(cipher, w.key_bytes, wm))
            {
                return Err(WriteFail::Superseded);
            }
            generation.saturating_add(1)
        }
    };
    tx.execute(
        "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, watermark, stale,
             payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
         ON CONFLICT(tag) DO UPDATE SET ns_key = excluded.ns_key, minor = excluded.minor,
             generation = excluded.generation, writer_id = excluded.writer_id,
             watermark = excluded.watermark, stale = 0, payload = excluded.payload",
        params![
            w.tag,
            w.ns_key,
            w.minor,
            generation,
            w.writer.0 as i64,
            w.watermark_col,
            w.payload
        ],
    )?;
    let seq = next_use(&tx)?;
    tx.execute(
        "INSERT INTO usage(tag, last_used) VALUES (?1, ?2)
         ON CONFLICT(tag) DO UPDATE SET last_used = excluded.last_used",
        params![w.tag, seq],
    )?;
    if w.abort_before_commit {
        std::process::abort();
    }
    tx.commit()?;
    Ok(u64::try_from(generation).unwrap_or(0))
}

fn try_open(path: &Path) -> Result<Connection, Fail> {
    // Not `SQLITE_OPEN_NOFOLLOW`: it refuses a link in any path component, and data
    // directories legitimately sit behind one (macOS `/var` → `/private/var`, a moved home).
    // `files::prepare_file` refuses a link as the file or a sidecar itself.
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(classify)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(classify)?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(classify)?;
    if check != "ok" {
        return Err(Fail::Corrupt);
    }
    // Read the format major before changing anything: a file of another major is moved aside
    // byte for byte (edge 6), never converted.
    let has_header: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'header')",
            [],
            |r| r.get(0),
        )
        .map_err(classify)?;
    let major: Option<i64> = if has_header {
        conn.query_row(
            "SELECT value FROM header WHERE key = 'format_major'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(classify)?
    } else {
        None
    };
    if major.is_some_and(|m| m != i64::from(FORMAT_MAJOR)) {
        return Err(Fail::Major);
    }
    // Before the first table: freed pages can be returned after an eviction.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")
        .map_err(classify)?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))
        .map_err(classify)?;
    // `secure_delete`: a deleted or replaced row leaves zeros, not old names, in free pages.
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON",
    )
    .map_err(classify)?;
    conn.execute_batch(&format!("BEGIN IMMEDIATE; {DDL} COMMIT;"))
        .map_err(classify)?;
    if major.is_none() {
        conn.execute(
            "INSERT OR IGNORE INTO header(key, value) VALUES ('format_major', ?1), \
             ('format_minor', ?2), ('created_at', ?3)",
            params![FORMAT_MAJOR, FORMAT_MINOR, unix_ms()],
        )
        .map_err(classify)?;
    }
    Ok(conn)
}

/// A file just created (or emptied) must open; if it does not, that is an I/O failure.
fn fresh(path: &Path) -> Result<Connection, CacheError> {
    try_open(path).map_err(|f| match f {
        Fail::Other(e) => e,
        Fail::Corrupt | Fail::Major => {
            CacheError::Io(std::io::Error::other("a fresh catalog cache did not open"))
        }
    })
}

fn header_blob(conn: &Connection, key: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row("SELECT value FROM header WHERE key = ?1", [key], |r| {
        Ok(match r.get_ref(0)? {
            ValueRef::Blob(b) | ValueRef::Text(b) => b.to_vec(),
            _ => Vec::new(),
        })
    })
    .optional()
}

/// Whether the file's mode and key match the requested ones. A file that has never recorded a
/// mode matches only while it holds no rows.
fn mode_matches(conn: &Connection, cipher: Option<&RowCipher>) -> rusqlite::Result<bool> {
    let mode = header_blob(conn, "encryption")?;
    match (mode.as_deref(), cipher) {
        (None, _) => {
            let rows: i64 = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM namespace) + (SELECT COUNT(*) FROM source)",
                [],
                |r| r.get(0),
            )?;
            Ok(rows == 0)
        }
        (Some(b"none"), None) => Ok(true),
        (Some(b"row"), Some(c)) => Ok(header_blob(conn, "key_check")?
            .is_some_and(|b| c.open(&[AD_KEY_CHECK], &b).is_ok_and(|p| p == KEY_CHECK))),
        _ => Ok(false),
    }
}

/// Records the mode (and, encrypted, the key check) of a file that has none yet.
fn init_mode(conn: &Connection, cipher: Option<&RowCipher>) -> Result<(), CacheError> {
    if header_blob(conn, "encryption").map_err(sql_err)?.is_some() {
        return Ok(());
    }
    match cipher {
        None => conn.execute(
            "INSERT INTO header(key, value) VALUES ('encryption', 'none')",
            [],
        ),
        Some(c) => {
            let check = c
                .seal(&[AD_KEY_CHECK], KEY_CHECK)
                .map_err(CacheError::Crypt)?;
            conn.execute(
                "INSERT INTO header(key, value) VALUES ('encryption', 'row'), ('key_check', ?1)",
                [check],
            )
        }
    }
    .map_err(sql_err)?;
    Ok(())
}

impl CacheStore {
    /// Opens (or creates) the cache at `path` (alg/02 step 1): the directory 0700, the file and
    /// its sidecars 0600, WAL, `synchronous=NORMAL`, `quick_check`. A corrupt file (or one of
    /// another major) is moved aside to `*.corrupt-<secs>[-n]` with its `-wal` first, and an
    /// empty store is returned with `rebuilt`. A file of the other encryption mode, or one the
    /// key does not open, is scrubbed and rebuilt (`WrongKey`). Any error: run memory-only.
    pub fn open(
        path: &Path,
        encryption: Encryption,
        writer: WriterId,
    ) -> Result<Opened, CacheError> {
        Self::open_as(path, encryption, writer, FORMAT_MINOR)
    }

    fn open_as(
        path: &Path,
        encryption: Encryption,
        writer: WriterId,
        minor: u16,
    ) -> Result<Opened, CacheError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            files::ensure_dir(dir)?;
        }
        let cipher = match encryption {
            Encryption::None => None,
            Encryption::Row(key) => Some(RowCipher::new(key, CATALOG_DOMAIN)),
        };
        files::prepare_file(path)?;
        let mut rebuilt = None;
        let mut moved_to = None;
        let conn = match try_open(path) {
            Ok(conn) => conn,
            Err(Fail::Other(e)) => return Err(e),
            Err(fail) => {
                rebuilt = Some(match fail {
                    Fail::Major => RebuildReason::MajorChanged,
                    _ => RebuildReason::Corrupt,
                });
                moved_to = Some(files::rotate(path)?);
                files::prepare_file(path)?;
                fresh(path)?
            }
        };
        let conn = if mode_matches(&conn, cipher.as_ref()).map_err(sql_err)? {
            conn
        } else {
            drop(conn);
            files::scrub_family(path)?;
            files::prepare_file(path)?;
            rebuilt = Some(RebuildReason::WrongKey);
            fresh(path)?
        };
        init_mode(&conn, cipher.as_ref())?;
        files::tighten(path)?;
        Ok(Opened {
            store: CacheStore {
                conn,
                path: path.to_path_buf(),
                cipher,
                writer,
                minor,
                newer: HashMap::new(),
                full_warned: false,
                #[cfg(feature = "test-support")]
                fault: None,
            },
            rebuilt,
            moved_to,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether rows are sealed.
    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    /// The lookup tag of a namespace key's canonical bytes.
    fn tag_of(&self, key_bytes: &[u8]) -> Vec<u8> {
        match &self.cipher {
            None => key_bytes.to_vec(),
            Some(c) => c.tag(key_bytes).to_vec(),
        }
    }

    fn source_tag(&self, r: SourceRef) -> Vec<u8> {
        let id = source_id(r);
        match &self.cipher {
            None => id.to_vec(),
            Some(c) => c.tag(&id).to_vec(),
        }
    }

    fn seal_or_copy(&self, ad: &[u8], data: &[u8]) -> Result<Vec<u8>, CacheError> {
        match &self.cipher {
            None => Ok(data.to_vec()),
            Some(c) => c.seal(ad, data).map_err(CacheError::Crypt),
        }
    }

    /// Compares `server` with the fingerprint the cache was filled from (alg/02 step 2). The
    /// first call records it. Another server marks every row stale (it loads with
    /// `Watermark::None`, so its next pass is full) and keeps the rows for offline use.
    pub fn check_identity(
        &mut self,
        server: ServerFingerprint,
    ) -> Result<IdentityCheck, CacheError> {
        let stored = header_blob(&self.conn, "server").map_err(sql_err)?;
        let stored_fp = stored.as_deref().and_then(|b| {
            let plain = match &self.cipher {
                None => Some(b.to_vec()),
                Some(c) => c.open(&[AD_SERVER], b).ok(),
            }?;
            <[u8; 16]>::try_from(plain.as_slice())
                .ok()
                .map(u128::from_le_bytes)
        });
        if stored_fp == Some(server.0) {
            return Ok(IdentityCheck::Same);
        }
        let value = self.seal_or_copy(&[AD_SERVER], &server.0.to_le_bytes())?;
        let first = stored.is_none();
        let tx = self.conn.transaction().map_err(sql_err)?;
        if !first {
            tx.execute("UPDATE namespace SET stale = 1", [])
                .map_err(sql_err)?;
        }
        tx.execute(
            "INSERT INTO header(key, value) VALUES ('server', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )
        .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(if first {
            IdentityCheck::Same
        } else {
            IdentityCheck::Changed
        })
    }

    /// Reads one row: unseal and check the key, unseal the payload, decode (`migrate_minor`).
    fn read_row(
        &self,
        tag: &[u8],
        ns_key_col: &[u8],
        minor: i64,
        stale: bool,
        payload: &[u8],
    ) -> Result<(NsKey, Decoded), (Option<NsKey>, RowError)> {
        let key_bytes = match &self.cipher {
            None => ns_key_col.to_vec(),
            Some(c) => c
                .open(&ad(AD_NS_KEY, &[tag]), ns_key_col)
                .map_err(|e| (None, RowError::Crypt(e)))?,
        };
        let key = decode_ns_key(&key_bytes).map_err(|e| (None, RowError::Decode(e)))?;
        let bad = |key: &NsKey, e: RowError| (Some(key.clone()), e);
        if self.tag_of(&key_bytes) != tag {
            return Err(bad(&key, RowError::Mismatch));
        }
        let minor = u16::try_from(minor).map_err(|_| bad(&key, RowError::Mismatch))?;
        let decoded = match &self.cipher {
            None => codec::decode_with(payload, self.minor, stale),
            Some(c) => {
                let plain = c
                    .open(&payload_ad(&key_bytes, minor), payload)
                    .map_err(|e| bad(&key, RowError::Crypt(e)))?;
                codec::decode_with(&plain, self.minor, stale)
            }
        }
        .map_err(|e| bad(&key, RowError::Decode(e)))?;
        if decoded.ns.key() != &key || decoded.minor != minor {
            return Err(bad(&key, RowError::Mismatch));
        }
        Ok((key, decoded))
    }

    /// Deletes unreadable rows and moves the LRU clock of `touched` (best effort: a cache that
    /// cannot be written still loads).
    fn after_load(&mut self, bad: &[Vec<u8>], touched: Option<&[u8]>) {
        let run = |conn: &mut Connection| -> rusqlite::Result<()> {
            let tx = conn.transaction()?;
            for t in bad {
                tx.execute("DELETE FROM namespace WHERE tag = ?1", [t])?;
                tx.execute("DELETE FROM usage WHERE tag = ?1", [t])?;
            }
            let seq = next_use(&tx)?;
            match touched {
                Some(tag) => tx.execute(
                    "INSERT INTO usage(tag, last_used) VALUES (?1, ?2)
                     ON CONFLICT(tag) DO UPDATE SET last_used = excluded.last_used",
                    params![tag, seq],
                )?,
                None => tx.execute(
                    "INSERT INTO usage(tag, last_used) SELECT tag, ?1 FROM namespace WHERE true
                     ON CONFLICT(tag) DO UPDATE SET last_used = excluded.last_used",
                    [seq],
                )?,
            };
            tx.commit()
        };
        let _ = run(&mut self.conn);
    }

    /// Loads one namespace (alg/02 step 3: decrypt → decode → `migrate_minor`; publish it with
    /// `stale = true`, [`super::cached_state`]). `Ok(None)` when absent. An unreadable row is
    /// deleted and reported as [`CacheError::Row`] (re-introspect that namespace only).
    pub fn load_namespace(&mut self, key: &NsKey) -> Result<Option<NamespaceSnapshot>, CacheError> {
        let tag = self.tag_of(&encode_ns_key(key));
        type Raw = (Option<Vec<u8>>, Option<i64>, Option<i64>, Option<Vec<u8>>);
        let row: Option<Raw> = self
            .conn
            .query_row(
                "SELECT ns_key, minor, stale, payload FROM namespace WHERE tag = ?1",
                [&tag],
                |r| {
                    Ok((
                        blob(r.get_ref(0)?).map(<[u8]>::to_vec),
                        int(r.get_ref(1)?),
                        int(r.get_ref(2)?),
                        blob(r.get_ref(3)?).map(<[u8]>::to_vec),
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let read = match row {
            (Some(k), Some(minor), Some(stale), Some(payload)) => {
                self.read_row(&tag, &k, minor, stale != 0, &payload)
            }
            _ => Err((None, RowError::Mismatch)),
        };
        match read {
            Ok((got, _)) if &got != key => Ok(None),
            Ok((got, d)) => {
                if d.from_newer_minor {
                    self.newer.insert(got, d.ns.meta().clone());
                }
                self.after_load(&[], Some(&tag));
                Ok(Some(d.ns))
            }
            Err((_, e)) => {
                self.after_load(&[tag], None);
                Err(CacheError::Row(e))
            }
        }
    }

    /// Loads every namespace (the connect-time and offline load; the 300 ms budget). Rows that
    /// cannot be read are deleted and listed in [`LoadAll::dropped`].
    pub fn load_all(&mut self) -> Result<LoadAll, CacheError> {
        let mut out = LoadAll::default();
        let mut bad: Vec<Vec<u8>> = Vec::new();
        let mut newer: Vec<(NsKey, NsMeta)> = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT tag, ns_key, minor, stale, payload FROM namespace ORDER BY tag")
                .map_err(sql_err)?;
            let mut rows = stmt.query([]).map_err(sql_err)?;
            while let Some(row) = rows.next().map_err(sql_err)? {
                let cols = (
                    blob(row.get_ref(0).map_err(sql_err)?),
                    blob(row.get_ref(1).map_err(sql_err)?),
                    int(row.get_ref(2).map_err(sql_err)?),
                    int(row.get_ref(3).map_err(sql_err)?),
                    blob(row.get_ref(4).map_err(sql_err)?),
                );
                let (Some(tag), Some(k), Some(minor), Some(stale), Some(payload)) = cols else {
                    if let Some(tag) = cols.0 {
                        bad.push(tag.to_vec());
                    }
                    out.dropped.push((None, RowError::Mismatch));
                    continue;
                };
                match self.read_row(tag, k, minor, stale != 0, payload) {
                    Ok((key, d)) => {
                        if d.from_newer_minor {
                            out.from_newer_minor.push(key.clone());
                            newer.push((key, d.ns.meta().clone()));
                        }
                        out.namespaces.push(d.ns);
                    }
                    Err((key, e)) => {
                        bad.push(tag.to_vec());
                        out.dropped.push((key, e));
                    }
                }
            }
        }
        self.newer.extend(newer);
        self.after_load(&bad, None);
        Ok(out)
    }

    /// Writes `ns` (alg/02 step 4: one `BEGIN IMMEDIATE` transaction, replacing the
    /// namespace's previous row). Refused for a temp namespace, for a namespace read from a
    /// newer minor (until a fresh introspection replaces it), and — compare-and-set, step 5 —
    /// when another writer's row has a newer watermark ([`CacheError::Superseded`]). Callers
    /// debounce ([`super::SaveDebouncer`]) and run it in the background.
    pub fn save_namespace(&mut self, ns: &NamespaceSnapshot) -> Result<(), CacheError> {
        if ns.is_temp() {
            return Err(CacheError::TempNamespace);
        }
        if self.newer.get(ns.key()).is_some_and(|m| m == ns.meta()) {
            return Err(CacheError::NewerMinor);
        }
        let key_bytes = encode_ns_key(ns.key());
        let tag = self.tag_of(&key_bytes);
        let plain = codec::encode_as(ns, self.minor, &[]).map_err(CacheError::Encode)?;
        let watermark = &ns.meta().watermark;
        let wm_plain = encode_watermark(watermark);
        let (payload, ns_key_col, wm_col) = match &self.cipher {
            None => (plain, key_bytes.clone(), wm_plain),
            Some(c) => (
                c.seal(&payload_ad(&key_bytes, self.minor), &plain)
                    .map_err(CacheError::Crypt)?,
                c.seal(&ad(AD_NS_KEY, &[&tag]), &key_bytes)
                    .map_err(CacheError::Crypt)?,
                c.seal(&ad(AD_WATERMARK, &[&key_bytes]), &wm_plain)
                    .map_err(CacheError::Crypt)?,
            ),
        };
        #[cfg(feature = "test-support")]
        let abort_before_commit = self.fault == Some(Fault::AbortBeforeCommit);
        #[cfg(not(feature = "test-support"))]
        let abort_before_commit = false;
        let w = RowWrite {
            tag: &tag,
            key_bytes: &key_bytes,
            ns_key: &ns_key_col,
            minor: self.minor,
            writer: self.writer,
            watermark,
            watermark_col: &wm_col,
            payload: &payload,
            abort_before_commit,
        };
        match write_row(&mut self.conn, self.cipher.as_ref(), &w) {
            Ok(_) => {
                self.newer.remove(ns.key());
                Ok(())
            }
            Err(WriteFail::Superseded) => Err(CacheError::Superseded),
            Err(WriteFail::Sql(e)) if e.sqlite_error_code() == Some(ErrorCode::DiskFull) => {
                let warn = !self.full_warned;
                self.full_warned = true;
                Err(CacheError::Full { warn })
            }
            Err(WriteFail::Sql(e)) => Err(sql_err(e)),
        }
    }

    /// Removes a namespace (a dropped schema). Whether a row was there.
    pub fn remove_namespace(&mut self, key: &NsKey) -> Result<bool, CacheError> {
        let tag = self.tag_of(&encode_ns_key(key));
        let tx = self.conn.transaction().map_err(sql_err)?;
        let n = tx
            .execute("DELETE FROM namespace WHERE tag = ?1", [&tag])
            .map_err(sql_err)?;
        tx.execute("DELETE FROM usage WHERE tag = ?1", [&tag])
            .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        self.newer.remove(key);
        Ok(n > 0)
    }

    /// The bookkeeping of `key`'s row, if any.
    pub fn row_info(&self, key: &NsKey) -> Result<Option<RowInfo>, CacheError> {
        let tag = self.tag_of(&encode_ns_key(key));
        self.conn
            .query_row(
                "SELECT generation, writer_id, minor, stale FROM namespace WHERE tag = ?1",
                [&tag],
                |r| {
                    Ok(RowInfo {
                        generation: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                        writer: WriterId(r.get::<_, i64>(1)? as u64),
                        minor: u16::try_from(r.get::<_, i64>(2)?).unwrap_or(u16::MAX),
                        stale: r.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()
            .map_err(sql_err)
    }

    /// Stores source text under its content address (alg/02 step 6); returns the reference the
    /// model keeps. Sealed in an encrypted store, under a keyed tag.
    pub fn put_source(&mut self, text: &str) -> Result<SourceRef, CacheError> {
        let r = SourceRef::of(text);
        let tag = self.source_tag(r);
        let col = self.seal_or_copy(&ad(AD_SOURCE, &[&source_id(r)]), text.as_bytes())?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO source(tag, len, text) VALUES (?1, ?2, ?3)",
                params![tag, r.len, col],
            )
            .map_err(sql_err)?;
        Ok(r)
    }

    /// The source text for `r`, if cached and intact.
    pub fn source(&self, r: SourceRef) -> Result<Option<String>, CacheError> {
        let tag = self.source_tag(r);
        let col: Option<Option<Vec<u8>>> = self
            .conn
            .query_row("SELECT text FROM source WHERE tag = ?1", [&tag], |row| {
                Ok(blob(row.get_ref(0)?).map(<[u8]>::to_vec))
            })
            .optional()
            .map_err(sql_err)?;
        let Some(Some(col)) = col else {
            return Ok(None);
        };
        let plain = match &self.cipher {
            None => Some(col),
            Some(c) => c.open(&ad(AD_SOURCE, &[&source_id(r)]), &col).ok(),
        };
        Ok(plain
            .and_then(|b| String::from_utf8(b).ok())
            .filter(|t| SourceRef::of(t) == r))
    }

    /// Deletes every source no live namespace references (`live`: the union of
    /// [`super::source_refs`] over the catalog). How many were deleted.
    pub fn gc_sources(&mut self, live: &HashSet<SourceRef>) -> Result<usize, CacheError> {
        let keep: HashSet<Vec<u8>> = live.iter().map(|r| self.source_tag(*r)).collect();
        let tags: Vec<Vec<u8>> = {
            let mut stmt = self
                .conn
                .prepare("SELECT tag FROM source")
                .map_err(sql_err)?;
            let rows = stmt
                .query_map([], |r| Ok(blob(r.get_ref(0)?).map(<[u8]>::to_vec)))
                .map_err(sql_err)?;
            let mut v = Vec::new();
            for t in rows {
                if let Some(t) = t.map_err(sql_err)? {
                    v.push(t);
                }
            }
            v
        };
        let tx = self.conn.transaction().map_err(sql_err)?;
        let mut n = 0;
        for t in tags.iter().filter(|t| !keep.contains(*t)) {
            n += tx
                .execute("DELETE FROM source WHERE tag = ?1", [t])
                .map_err(sql_err)?;
        }
        tx.commit().map_err(sql_err)?;
        Ok(n)
    }

    /// The bytes the cache's rows hold (namespace rows and sources).
    pub fn total_bytes(&self) -> Result<u64, CacheError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE((SELECT SUM(length(tag) + length(ns_key) + length(watermark)
                     + length(payload)) FROM namespace), 0)
                 + COALESCE((SELECT SUM(length(tag) + length(text)) FROM source), 0)",
                [],
                |r| r.get(0),
            )
            .map_err(sql_err)?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// The per-profile cap (R-M4-25, alg/02 step 7): while the rows exceed `cap` bytes, deletes
    /// the least recently loaded namespace for which `protect` is false (the hub protects the
    /// current database's namespaces). Unreadable rows go first. Freed pages are returned.
    pub fn evict(
        &mut self,
        cap: u64,
        protect: &dyn Fn(&NsKey) -> bool,
    ) -> Result<Evicted, CacheError> {
        let before = self.total_bytes()?;
        let mut out = Evicted {
            evicted: Vec::new(),
            bytes_before: before,
            bytes_after: before,
        };
        if before <= cap {
            return Ok(out);
        }
        // (unreadable first, last used, tag) -> (key, bytes)
        let mut rows: Vec<EvictRow> = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT n.tag, n.ns_key, length(n.tag) + length(n.ns_key)
                         + length(n.watermark) + length(n.payload), COALESCE(u.last_used, 0)
                     FROM namespace n LEFT JOIN usage u ON u.tag = n.tag",
                )
                .map_err(sql_err)?;
            let mut q = stmt.query([]).map_err(sql_err)?;
            while let Some(r) = q.next().map_err(sql_err)? {
                let Some(tag) = blob(r.get_ref(0).map_err(sql_err)?).map(<[u8]>::to_vec) else {
                    continue;
                };
                let key = blob(r.get_ref(1).map_err(sql_err)?).and_then(|col| {
                    let bytes = match &self.cipher {
                        None => Some(col.to_vec()),
                        Some(c) => c.open(&ad(AD_NS_KEY, &[&tag]), col).ok(),
                    }?;
                    decode_ns_key(&bytes).ok()
                });
                let size = int(r.get_ref(2).map_err(sql_err)?).unwrap_or(0);
                let used = int(r.get_ref(3).map_err(sql_err)?).unwrap_or(0);
                if key.as_ref().is_some_and(protect) {
                    continue;
                }
                rows.push((
                    key.is_some(),
                    used,
                    tag,
                    key,
                    u64::try_from(size).unwrap_or(0),
                ));
            }
        }
        rows.sort_by(|a, b| (a.0, a.1, &a.2).cmp(&(b.0, b.1, &b.2)));
        let mut total = before;
        let tx = self.conn.transaction().map_err(sql_err)?;
        for (_, _, tag, key, size) in rows {
            if total <= cap {
                break;
            }
            tx.execute("DELETE FROM namespace WHERE tag = ?1", [&tag])
                .map_err(sql_err)?;
            tx.execute("DELETE FROM usage WHERE tag = ?1", [&tag])
                .map_err(sql_err)?;
            total = total.saturating_sub(size);
            out.evicted.push(key);
        }
        tx.commit().map_err(sql_err)?;
        for k in out.evicted.iter().flatten() {
            self.newer.remove(k);
        }
        let _ = self.conn.execute_batch("PRAGMA incremental_vacuum");
        out.bytes_after = self.total_bytes()?;
        Ok(out)
    }

    /// Scrubs and deletes every cache file of `profile` under `dir` (every major, sidecars and
    /// corrupt rotations): profile deletion (edge 8) and "Encrypt cached metadata" changes.
    pub fn delete_profile_files(dir: &Path, profile: &ProfileId) -> Result<(), CacheError> {
        files::delete_profile(dir, profile).map_err(CacheError::Io)
    }
}

#[cfg(feature = "test-support")]
impl CacheStore {
    /// Opens as a build of format minor `minor` would (feature `test-support`).
    pub fn open_as_minor(
        path: &Path,
        encryption: Encryption,
        writer: WriterId,
        minor: u16,
    ) -> Result<Opened, CacheError> {
        Self::open_as(path, encryption, writer, minor)
    }

    /// Arms (or clears) a test fault.
    pub fn set_fault(&mut self, fault: Option<Fault>) {
        self.fault = fault;
    }

    /// Caps the database at its current page count, so the next write that needs a page fails
    /// with `SQLITE_FULL` (a full disk, as SQLite reports it).
    pub fn limit_growth(&self) -> Result<(), CacheError> {
        let pages: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(sql_err)?;
        self.conn
            .query_row(&format!("PRAGMA max_page_count = {pages}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(sql_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::namespace::PgIdentity;

    fn pg(w: u64, sys: u64) -> Watermark {
        Watermark::PgXid {
            w,
            identity: PgIdentity {
                system_identifier: sys,
                start_time_micros: 0,
            },
        }
    }

    #[test]
    fn watermark_order_for_compare_and_set() {
        assert!(not_older(&pg(5, 1), &pg(5, 1)));
        assert!(not_older(&pg(6, 1), &pg(5, 1)));
        assert!(!not_older(&pg(4, 1), &pg(5, 1)));
        // Another cluster: not comparable, the save wins.
        assert!(not_older(&pg(4, 2), &pg(5, 1)));
        assert!(not_older(&pg(1, 1), &Watermark::None));
        assert!(!not_older(&Watermark::None, &pg(1, 1)));
        assert!(not_older(&Watermark::None, &Watermark::None));
    }
}
