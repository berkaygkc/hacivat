//! [`CacheStore`]: the SQLite file of one profile (alg/02 HV-MODEL-03 steps 1–7).
//!
//! Tables: `header` (format, mode, key check, server fingerprint and epoch), `namespace` (one
//! row per namespace: lookup tag, key, minor, generation, writer, epoch, watermark, payload),
//! `usage` (the LRU clock, apart from the payload rows so a load never rewrites a payload) and
//! `source` (content-addressed source texts). In an encrypted store every column that could
//! carry a name is sealed and padded, and `tag` columns are keyed lookup tags.
//!
//! **The file's schema is untrusted** (fix round 1, B-I1): each connection runs with
//! `trusted_schema = OFF` and `SQLITE_DBCONFIG_DEFENSIVE`, and before any other read or write
//! `sqlite_master` must equal exactly the objects this build creates (same tables, same
//! `CREATE` text, the four primary-key autoindexes, no trigger, view or extra index) and every
//! table's `pragma_table_info` must match. Anything else is `Corrupt`: the file is rotated and
//! rebuilt. A `format_major` of the wrong type is `Corrupt` too.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use rusqlite::config::DbConfig;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};

use super::crypt::{CATALOG_DOMAIN, RowCipher, pad, unpad};
use super::{
    CacheCaps, CacheError, Encryption, IdentityCheck, LoadAll, Opened, RebuildReason, RowError,
    ServerFingerprint, WriterId, files,
};
use crate::codec::{
    self, Decoded, FORMAT_MAJOR, FORMAT_MINOR, decode_ns_key, decode_watermark, encode_ns_key,
    encode_watermark,
};
use crate::ids::{NsKey, SourceRef};
use crate::namespace::{NamespaceSnapshot, NsMeta, Watermark};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One expected column: `(name, type, notnull, pk)` as `pragma_table_info` reports it.
type Column = (&'static str, &'static str, bool, i64);
/// One expected table: its name, its exact `CREATE` text and its columns.
type Table = (&'static str, &'static str, &'static [Column]);
/// One `sqlite_master` row: `(type, name, tbl_name, sql)`.
type MasterRow = (String, String, String, Option<String>);

/// The tables this build creates, each with its exact `CREATE` text (as `sqlite_master`
/// stores it) and its `pragma_table_info` rows: `(name, type, notnull, pk)`.
const TABLES: [Table; 4] = [
    (
        "header",
        "CREATE TABLE header(key TEXT PRIMARY KEY, value BLOB NOT NULL)",
        &[("key", "TEXT", false, 1), ("value", "BLOB", true, 0)],
    ),
    (
        "namespace",
        "CREATE TABLE namespace(tag BLOB PRIMARY KEY, ns_key BLOB NOT NULL, \
         minor INTEGER NOT NULL, generation INTEGER NOT NULL, writer_id INTEGER NOT NULL, \
         epoch INTEGER NOT NULL, watermark BLOB NOT NULL, payload BLOB NOT NULL)",
        &[
            ("tag", "BLOB", false, 1),
            ("ns_key", "BLOB", true, 0),
            ("minor", "INTEGER", true, 0),
            ("generation", "INTEGER", true, 0),
            ("writer_id", "INTEGER", true, 0),
            ("epoch", "INTEGER", true, 0),
            ("watermark", "BLOB", true, 0),
            ("payload", "BLOB", true, 0),
        ],
    ),
    (
        "usage",
        "CREATE TABLE usage(tag BLOB PRIMARY KEY, last_used INTEGER NOT NULL)",
        &[("tag", "BLOB", false, 1), ("last_used", "INTEGER", true, 0)],
    ),
    (
        "source",
        "CREATE TABLE source(tag BLOB PRIMARY KEY, text BLOB NOT NULL)",
        &[("tag", "BLOB", false, 1), ("text", "BLOB", true, 0)],
    ),
];

/// The largest payload one row may have; a longer row is never read (B-M3). A real namespace of
/// a million objects is about 40 MB.
pub const MAX_ROW_BYTES: i64 = 256 << 20;

// Padding floors of the sealed columns (fix round 1, I2/m2): a ciphertext length shows only its
// Padmé bucket, and every namespace key below the floor seals to the same length.
const NS_KEY_FLOOR: usize = 256;
const WATERMARK_FLOOR: usize = 128;
const SOURCE_FLOOR: usize = 256;
const PAYLOAD_FLOOR: usize = 4096;

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

/// The bookkeeping a sealed row is bound to: its minor, generation, writer and epoch (the
/// server identity it was saved under).
#[derive(Debug, Clone, Copy)]
struct RowMeta {
    minor: u16,
    generation: i64,
    writer: i64,
    epoch: i64,
}

/// A payload's associated data: `1 ‖ minor ‖ generation ‖ writer ‖ epoch ‖ ns_key` (fixed
/// widths first, the key last, so the split is unambiguous). Editing any of these columns
/// makes the row fail authentication (fix round 1, A-m4/B-M1).
fn payload_ad(key_bytes: &[u8], m: RowMeta) -> Vec<u8> {
    ad(
        AD_PAYLOAD,
        &[
            &m.minor.to_le_bytes(),
            &m.generation.to_le_bytes(),
            &m.writer.to_le_bytes(),
            &m.epoch.to_le_bytes(),
            key_bytes,
        ],
    )
}

/// A watermark's associated data: `3 ‖ generation ‖ ns_key` (it cannot be replayed apart from
/// its row).
fn watermark_ad(key_bytes: &[u8], generation: i64) -> Vec<u8> {
    ad(AD_WATERMARK, &[&generation.to_le_bytes(), key_bytes])
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
    /// Saved under an earlier server epoch (a server change since): the namespace loads with
    /// `Watermark::None`.
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
    /// The read limit: a load reads at most `caps.per_profile` payload bytes (B-M3).
    caps: CacheCaps,
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
    Crypt(super::CryptError),
}

impl From<rusqlite::Error> for WriteFail {
    fn from(e: rusqlite::Error) -> Self {
        WriteFail::Sql(e)
    }
}

/// The columns of one row to write (plaintext; sealed inside the transaction, once the
/// generation and epoch are known).
struct RowWrite<'a> {
    tag: &'a [u8],
    key_bytes: &'a [u8],
    minor: u16,
    writer: WriterId,
    watermark: &'a Watermark,
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

/// The server identity recorded in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerState {
    /// None recorded yet (epoch 0).
    None,
    Known {
        fp: u128,
        epoch: i64,
    },
    /// Present but not readable (tampered, or sealed under another key): every row is stale.
    Unreadable,
}

impl ServerState {
    fn epoch(self) -> i64 {
        match self {
            ServerState::None => 0,
            ServerState::Known { epoch, .. } => epoch,
            ServerState::Unreadable => i64::MIN,
        }
    }
}

fn read_server(conn: &Connection, cipher: Option<&RowCipher>) -> rusqlite::Result<ServerState> {
    let Some(stored) = header_blob(conn, "server")? else {
        return Ok(ServerState::None);
    };
    let plain = match cipher {
        None => Some(stored),
        Some(c) => c.open(&[AD_SERVER], &stored).ok(),
    };
    Ok(plain
        .filter(|p| p.len() == 24)
        .map_or(ServerState::Unreadable, |p| {
            let mut fp = [0u8; 16];
            fp.copy_from_slice(&p[..16]);
            let mut epoch = [0u8; 8];
            epoch.copy_from_slice(&p[16..]);
            ServerState::Known {
                fp: u128::from_le_bytes(fp),
                epoch: i64::from_le_bytes(epoch),
            }
        }))
}

/// The stored watermark of another writer's row (unreadable reads as `None`, which loses).
fn stored_watermark(
    cipher: Option<&RowCipher>,
    key_bytes: &[u8],
    generation: i64,
    col: &[u8],
) -> Watermark {
    let plain = match cipher {
        None => Some(col.to_vec()),
        Some(c) => c
            .open(&watermark_ad(key_bytes, generation), col)
            .ok()
            .and_then(|p| unpad(&p).map(<[u8]>::to_vec)),
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
    let writer = w.writer.0 as i64;
    let generation = match &existing {
        None => 1,
        Some((stored_writer, generation, wm)) => {
            if *stored_writer != writer
                && !not_older(
                    w.watermark,
                    &stored_watermark(cipher, w.key_bytes, *generation, wm),
                )
            {
                return Err(WriteFail::Superseded);
            }
            generation.saturating_add(1)
        }
    };
    let epoch = read_server(&tx, cipher)?.epoch();
    let meta = RowMeta {
        minor: w.minor,
        generation,
        writer,
        epoch,
    };
    let wm_plain = encode_watermark(w.watermark);
    let (ns_key, watermark, payload) = match cipher {
        None => (w.key_bytes.to_vec(), wm_plain, w.payload.to_vec()),
        Some(c) => (
            c.seal(&ad(AD_NS_KEY, &[w.tag]), &pad(w.key_bytes, NS_KEY_FLOOR))
                .map_err(WriteFail::Crypt)?,
            c.seal(
                &watermark_ad(w.key_bytes, generation),
                &pad(&wm_plain, WATERMARK_FLOOR),
            )
            .map_err(WriteFail::Crypt)?,
            c.seal(
                &payload_ad(w.key_bytes, meta),
                &pad(w.payload, PAYLOAD_FLOOR),
            )
            .map_err(WriteFail::Crypt)?,
        ),
    };
    tx.execute(
        "INSERT INTO namespace(tag, ns_key, minor, generation, writer_id, epoch, watermark,
             payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(tag) DO UPDATE SET ns_key = excluded.ns_key, minor = excluded.minor,
             generation = excluded.generation, writer_id = excluded.writer_id,
             epoch = excluded.epoch, watermark = excluded.watermark,
             payload = excluded.payload",
        params![
            w.tag, ns_key, w.minor, generation, writer, epoch, watermark, payload
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

/// The objects of `sqlite_master`: `(type, name, tbl_name, sql)`, sorted.
fn master(conn: &Connection) -> rusqlite::Result<Vec<MasterRow>> {
    let mut stmt =
        conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    rows.collect()
}

/// Whether `objects` are exactly this build's schema (tables with their exact text and
/// columns, the primary-key autoindexes, nothing else).
fn schema_is_exact(conn: &Connection, objects: &[MasterRow]) -> rusqlite::Result<bool> {
    let mut expected: Vec<MasterRow> = Vec::new();
    for (name, sql, _) in TABLES {
        expected.push(("table".into(), name.into(), name.into(), Some(sql.into())));
        expected.push((
            "index".into(),
            format!("sqlite_autoindex_{name}_1"),
            name.into(),
            None,
        ));
    }
    expected.sort();
    if objects != expected.as_slice() {
        return Ok(false);
    }
    for (name, _, columns) in TABLES {
        let mut stmt = conn.prepare(
            "SELECT name, type, \"notnull\", dflt_value IS NULL, pk FROM pragma_table_info(?1)
             ORDER BY cid",
        )?;
        let got: Vec<(String, String, bool, bool, i64)> = stmt
            .query_map([name], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let want: Vec<(String, String, bool, bool, i64)> = columns
            .iter()
            .map(|(n, t, nn, pk)| ((*n).into(), (*t).into(), *nn, true, *pk))
            .collect();
        if got != want {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Opens the file and checks it before any other read or write: the schema (B-I1), then the
/// format major. A fresh (empty) file is given this build's schema.
fn try_open(path: &Path) -> Result<Connection, Fail> {
    // Not `SQLITE_OPEN_NOFOLLOW`: it refuses a link in any path component, and data
    // directories legitimately sit behind one (macOS `/var` → `/private/var`, a moved home).
    // `files::prepare_file` refuses a link as the file or a sidecar itself.
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(classify)?;
    // The file's schema is untrusted: no function or virtual table may run from it, and no
    // write may reach the schema or the file's pages behind SQLite's back.
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(classify)?;
    conn.execute_batch("PRAGMA trusted_schema = OFF")
        .map_err(classify)?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(classify)?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(classify)?;
    if check != "ok" {
        return Err(Fail::Corrupt);
    }
    let mut objects = master(&conn).map_err(classify)?;
    if objects.is_empty() {
        create_schema(&conn).map_err(classify)?;
        objects = master(&conn).map_err(classify)?;
    }
    // The header table first, then its major, then the rest: a file of another major is
    // moved aside byte for byte (edge 6), never read further.
    let (header_name, header_sql, _) = TABLES[0];
    let header_ok = objects
        .iter()
        .any(|(t, n, _, s)| t == "table" && n == header_name && s.as_deref() == Some(header_sql));
    if !header_ok {
        return Err(Fail::Corrupt);
    }
    let major: Option<Option<i64>> = conn
        .query_row(
            "SELECT value FROM header WHERE key = 'format_major'",
            [],
            |r| Ok(int(r.get_ref(0)?)),
        )
        .optional()
        .map_err(classify)?;
    match major {
        Some(Some(m)) if m == i64::from(FORMAT_MAJOR) => {}
        Some(Some(_)) => return Err(Fail::Major),
        // Missing, or of the wrong type: corrupt.
        _ => return Err(Fail::Corrupt),
    }
    if !schema_is_exact(&conn, &objects).map_err(classify)? {
        return Err(Fail::Corrupt);
    }
    Ok(conn)
}

/// Gives an empty file this build's schema and header, in one transaction (another process
/// may be creating it at the same moment: the second finds it done).
fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    // Before the first table: freed pages can be returned after an eviction.
    conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let run = || -> rusqlite::Result<()> {
        let n: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        if n == 0 {
            for (_, sql, _) in TABLES {
                conn.execute_batch(sql)?;
            }
            conn.execute(
                "INSERT INTO header(key, value) VALUES ('format_major', ?1), \
                 ('format_minor', ?2), ('created_at', ?3)",
                params![FORMAT_MAJOR, FORMAT_MINOR, unix_ms()],
            )?;
        }
        Ok(())
    };
    match run() {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// The connection settings of a checked file.
fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))?;
    // `secure_delete`: a deleted or replaced row leaves zeros, not old names, in free pages.
    conn.execute_batch(
        "PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON",
    )
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

/// What the file's recorded mode says about opening it with the requested one.
enum Mode {
    Matches,
    /// Plaintext opened with a key, a wrong key, an unrecorded mode with rows: scrub and
    /// rebuild.
    Rebuild,
    /// An encrypted file opened without a key: refused, memory-only, the file untouched
    /// (fix round 1, A-I1).
    Refuse,
}

/// Whether the file's mode and key match the requested ones. A file that has never recorded a
/// mode matches only while it holds no rows.
fn mode_of(conn: &Connection, cipher: Option<&RowCipher>) -> rusqlite::Result<Mode> {
    let mode = header_blob(conn, "encryption")?;
    Ok(match (mode.as_deref(), cipher) {
        (None, _) => {
            let rows: i64 = conn.query_row(
                "SELECT (SELECT COUNT(*) FROM namespace) + (SELECT COUNT(*) FROM source)",
                [],
                |r| r.get(0),
            )?;
            if rows == 0 {
                Mode::Matches
            } else {
                Mode::Rebuild
            }
        }
        (Some(b"none"), None) => Mode::Matches,
        (Some(b"row"), None) => Mode::Refuse,
        (Some(b"row"), Some(c)) => {
            if header_blob(conn, "key_check")?
                .is_some_and(|b| c.open(&[AD_KEY_CHECK], &b).is_ok_and(|p| p == KEY_CHECK))
            {
                Mode::Matches
            } else {
                Mode::Rebuild
            }
        }
        _ => Mode::Rebuild,
    })
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

/// The columns of one stored namespace row, as read.
struct RawRow<'r> {
    tag: &'r [u8],
    ns_key: &'r [u8],
    meta: RowMeta,
    payload: &'r [u8],
}

impl CacheStore {
    /// Opens (or creates) the cache at `path` (alg/02 step 1): the directory 0700, the file and
    /// its sidecars 0600, WAL, `synchronous=NORMAL`, `quick_check`, the exact schema. A corrupt
    /// file (a foreign schema object included) or one of another major is moved aside to
    /// `*.corrupt-<secs>[-n]` with its `-wal` first, and an empty store is returned with
    /// `rebuilt`. A plaintext file opened with a key, or one the key does not open, is scrubbed
    /// (with its `.corrupt-*` rotations) and rebuilt (`WrongKey`). An encrypted file opened
    /// without a key is refused with [`CacheError::Disabled`] and left untouched. Any error:
    /// run memory-only.
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
            Encryption::Row(key) => {
                Some(RowCipher::new(key, CATALOG_DOMAIN).map_err(CacheError::Crypt)?)
            }
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
        let conn = match mode_of(&conn, cipher.as_ref()).map_err(sql_err)? {
            Mode::Matches => conn,
            Mode::Refuse => return Err(CacheError::Disabled),
            Mode::Rebuild => {
                drop(conn);
                files::scrub_family(path)?;
                files::scrub_rotations(path)?;
                files::prepare_file(path)?;
                rebuilt = Some(RebuildReason::WrongKey);
                fresh(path)?
            }
        };
        configure(&conn).map_err(sql_err)?;
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
                caps: CacheCaps::DEFAULT,
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

    /// Sets the caps; a load reads at most `caps.per_profile` payload bytes, most recently used
    /// rows first (B-M3). Default: [`CacheCaps::DEFAULT`].
    pub fn set_caps(&mut self, caps: CacheCaps) {
        self.caps = caps;
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

    /// Compares `server` with the fingerprint the cache was filled from (alg/02 step 2). The
    /// first call records it. Another server starts a new epoch: every row saved under an
    /// earlier one is stale (it loads with `Watermark::None`, so its next pass is full); the
    /// rows are kept for offline use. The epoch is sealed with the fingerprint and bound into
    /// every row's associated data, so a row cannot be made fresh by editing a column.
    pub fn check_identity(
        &mut self,
        server: ServerFingerprint,
    ) -> Result<IdentityCheck, CacheError> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_err)?;
        let state = read_server(&tx, self.cipher.as_ref()).map_err(sql_err)?;
        let (epoch, result) = match state {
            ServerState::Known { fp, .. } if fp == server.0 => return Ok(IdentityCheck::Same),
            ServerState::None => (0, IdentityCheck::Same),
            ServerState::Known { .. } | ServerState::Unreadable => {
                // Past every epoch a row or the header carries: no row stays fresh.
                let max_row: i64 = tx
                    .query_row("SELECT COALESCE(MAX(epoch), 0) FROM namespace", [], |r| {
                        r.get(0)
                    })
                    .map_err(sql_err)?;
                let floor = max_row.max(state.epoch()).max(0);
                (floor.saturating_add(1), IdentityCheck::Changed)
            }
        };
        let mut plain = server.0.to_le_bytes().to_vec();
        plain.extend_from_slice(&epoch.to_le_bytes());
        let value = match &self.cipher {
            None => plain,
            Some(c) => c.seal(&[AD_SERVER], &plain).map_err(CacheError::Crypt)?,
        };
        tx.execute(
            "INSERT INTO header(key, value) VALUES ('server', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )
        .map_err(sql_err)?;
        tx.commit().map_err(sql_err)?;
        Ok(result)
    }

    /// Reads one row: unseal and check the key, unseal the payload, decode (`migrate_minor`).
    /// `epoch` is the header's current epoch: a row of another epoch is stale.
    fn read_row(
        &self,
        row: &RawRow<'_>,
        epoch: i64,
    ) -> Result<(NsKey, Decoded), (Option<NsKey>, RowError)> {
        let key_bytes = match &self.cipher {
            None => row.ns_key.to_vec(),
            Some(c) => {
                let padded = c
                    .open(&ad(AD_NS_KEY, &[row.tag]), row.ns_key)
                    .map_err(|e| (None, RowError::Crypt(e)))?;
                unpad(&padded).ok_or((None, RowError::Mismatch))?.to_vec()
            }
        };
        let key = decode_ns_key(&key_bytes).map_err(|e| (None, RowError::Decode(e)))?;
        let bad = |key: &NsKey, e: RowError| (Some(key.clone()), e);
        if self.tag_of(&key_bytes) != row.tag {
            return Err(bad(&key, RowError::Mismatch));
        }
        let stale = row.meta.epoch != epoch;
        let decoded = match &self.cipher {
            None => codec::decode_with(row.payload, self.minor, stale),
            Some(c) => {
                let padded = c
                    .open(&payload_ad(&key_bytes, row.meta), row.payload)
                    .map_err(|e| bad(&key, RowError::Crypt(e)))?;
                let plain = unpad(&padded).ok_or_else(|| bad(&key, RowError::Mismatch))?;
                codec::decode_with(plain, self.minor, stale)
            }
        }
        .map_err(|e| bad(&key, RowError::Decode(e)))?;
        if decoded.ns.key() != &key || decoded.minor != row.meta.minor {
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

    fn current_epoch(&self) -> Result<i64, CacheError> {
        Ok(read_server(&self.conn, self.cipher.as_ref())
            .map_err(sql_err)?
            .epoch())
    }

    /// Loads one namespace (alg/02 step 3: decrypt → decode → `migrate_minor`; publish it with
    /// `stale = true`, [`super::cached_state`]). `Ok(None)` when absent. An unreadable or
    /// oversized row is deleted and reported as [`CacheError::Row`] (re-introspect that
    /// namespace only).
    pub fn load_namespace(&mut self, key: &NsKey) -> Result<Option<NamespaceSnapshot>, CacheError> {
        let tag = self.tag_of(&encode_ns_key(key));
        let epoch = self.current_epoch()?;
        let limit = MAX_ROW_BYTES.min(i64::try_from(self.caps.per_profile).unwrap_or(i64::MAX));
        type Raw = (Option<Vec<u8>>, [Option<i64>; 5], Option<Vec<u8>>);
        let row: Option<Raw> = self
            .conn
            .query_row(
                "SELECT ns_key, minor, generation, writer_id, epoch, length(payload),
                     CASE WHEN length(payload) <= ?2 THEN payload END
                 FROM namespace WHERE tag = ?1",
                params![&tag, limit],
                |r| {
                    Ok((
                        blob(r.get_ref(0)?).map(<[u8]>::to_vec),
                        [
                            int(r.get_ref(1)?),
                            int(r.get_ref(2)?),
                            int(r.get_ref(3)?),
                            int(r.get_ref(4)?),
                            int(r.get_ref(5)?),
                        ],
                        blob(r.get_ref(6)?).map(<[u8]>::to_vec),
                    ))
                },
            )
            .optional()
            .map_err(sql_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let read = match row {
            (_, [_, _, _, _, Some(len)], _) if len > limit => Err((None, RowError::TooLarge)),
            (
                Some(k),
                [
                    Some(minor),
                    Some(generation),
                    Some(writer),
                    Some(row_epoch),
                    _,
                ],
                Some(p),
            ) => match u16::try_from(minor) {
                Ok(minor) => self.read_row(
                    &RawRow {
                        tag: &tag,
                        ns_key: &k,
                        meta: RowMeta {
                            minor,
                            generation,
                            writer,
                            epoch: row_epoch,
                        },
                        payload: &p,
                    },
                    epoch,
                ),
                Err(_) => Err((None, RowError::Mismatch)),
            },
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

    /// Loads every namespace (the connect-time and offline load; the 300 ms budget), most
    /// recently used first, reading at most `caps.per_profile` payload bytes in all and no row
    /// over [`MAX_ROW_BYTES`] (B-M3). Rows that cannot be read, or are past those limits, are
    /// deleted and listed in [`LoadAll::dropped`].
    pub fn load_all(&mut self) -> Result<LoadAll, CacheError> {
        let mut out = LoadAll::default();
        let mut bad: Vec<Vec<u8>> = Vec::new();
        let mut newer: Vec<(NsKey, NsMeta)> = Vec::new();
        let epoch = self.current_epoch()?;
        let total_limit = i64::try_from(self.caps.per_profile).unwrap_or(i64::MAX);
        let mut read_bytes = 0i64;
        {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT n.tag, n.ns_key, n.minor, n.generation, n.writer_id, n.epoch,
                         length(n.payload), n.payload
                     FROM namespace n LEFT JOIN usage u ON u.tag = n.tag
                     ORDER BY COALESCE(u.last_used, 0) DESC, n.tag",
                )
                .map_err(sql_err)?;
            let mut rows = stmt.query([]).map_err(sql_err)?;
            while let Some(row) = rows.next().map_err(sql_err)? {
                let tag = blob(row.get_ref(0).map_err(sql_err)?);
                let len = int(row.get_ref(6).map_err(sql_err)?).unwrap_or(i64::MAX);
                if len > MAX_ROW_BYTES || read_bytes.saturating_add(len) > total_limit {
                    if let Some(tag) = tag {
                        bad.push(tag.to_vec());
                    }
                    out.dropped.push((None, RowError::TooLarge));
                    continue;
                }
                read_bytes = read_bytes.saturating_add(len);
                let cols = (
                    tag,
                    blob(row.get_ref(1).map_err(sql_err)?),
                    int(row.get_ref(2).map_err(sql_err)?).and_then(|m| u16::try_from(m).ok()),
                    int(row.get_ref(3).map_err(sql_err)?),
                    int(row.get_ref(4).map_err(sql_err)?),
                    int(row.get_ref(5).map_err(sql_err)?),
                    blob(row.get_ref(7).map_err(sql_err)?),
                );
                let (
                    Some(tag),
                    Some(k),
                    Some(minor),
                    Some(generation),
                    Some(writer),
                    Some(row_epoch),
                    Some(payload),
                ) = cols
                else {
                    if let Some(tag) = cols.0 {
                        bad.push(tag.to_vec());
                    }
                    out.dropped.push((None, RowError::Mismatch));
                    continue;
                };
                let raw = RawRow {
                    tag,
                    ns_key: k,
                    meta: RowMeta {
                        minor,
                        generation,
                        writer,
                        epoch: row_epoch,
                    },
                    payload,
                };
                match self.read_row(&raw, epoch) {
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
    /// newer minor (until a fresh introspection replaces it), for a name over the codec's cap,
    /// and — compare-and-set, step 5 — when another writer's row has a newer watermark
    /// ([`CacheError::Superseded`]). Callers debounce ([`super::SaveDebouncer`]) and run it in
    /// the background.
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
        #[cfg(feature = "test-support")]
        let abort_before_commit = self.fault == Some(Fault::AbortBeforeCommit);
        #[cfg(not(feature = "test-support"))]
        let abort_before_commit = false;
        let w = RowWrite {
            tag: &tag,
            key_bytes: &key_bytes,
            minor: self.minor,
            writer: self.writer,
            watermark: &ns.meta().watermark,
            payload: &plain,
            abort_before_commit,
        };
        match write_row(&mut self.conn, self.cipher.as_ref(), &w) {
            Ok(_) => {
                self.newer.remove(ns.key());
                Ok(())
            }
            Err(WriteFail::Superseded) => Err(CacheError::Superseded),
            Err(WriteFail::Crypt(e)) => Err(CacheError::Crypt(e)),
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
        let epoch = self.current_epoch()?;
        self.conn
            .query_row(
                "SELECT generation, writer_id, minor, epoch FROM namespace WHERE tag = ?1",
                [&tag],
                |r| {
                    Ok(RowInfo {
                        generation: u64::try_from(r.get::<_, i64>(0)?).unwrap_or(0),
                        writer: WriterId(r.get::<_, i64>(1)? as u64),
                        minor: u16::try_from(r.get::<_, i64>(2)?).unwrap_or(u16::MAX),
                        stale: r.get::<_, i64>(3)? != epoch,
                    })
                },
            )
            .optional()
            .map_err(sql_err)
    }

    /// Stores source text under its content address (alg/02 step 6); returns the reference the
    /// model keeps. Sealed (and padded) in an encrypted store, under a keyed tag; no length is
    /// kept in the clear.
    pub fn put_source(&mut self, text: &str) -> Result<SourceRef, CacheError> {
        let r = SourceRef::of(text);
        let tag = self.source_tag(r);
        let col = match &self.cipher {
            None => text.as_bytes().to_vec(),
            Some(c) => c
                .seal(
                    &ad(AD_SOURCE, &[&source_id(r)]),
                    &pad(text.as_bytes(), SOURCE_FLOOR),
                )
                .map_err(CacheError::Crypt)?,
        };
        self.conn
            .execute(
                "INSERT OR IGNORE INTO source(tag, text) VALUES (?1, ?2)",
                params![tag, col],
            )
            .map_err(sql_err)?;
        Ok(r)
    }

    /// The source text for `r`, if cached and intact.
    pub fn source(&self, r: SourceRef) -> Result<Option<String>, CacheError> {
        let tag = self.source_tag(r);
        let col: Option<Option<Vec<u8>>> = self
            .conn
            .query_row(
                "SELECT CASE WHEN length(text) <= ?2 THEN text END FROM source WHERE tag = ?1",
                params![&tag, MAX_ROW_BYTES],
                |row| Ok(blob(row.get_ref(0)?).map(<[u8]>::to_vec)),
            )
            .optional()
            .map_err(sql_err)?;
        let Some(Some(col)) = col else {
            return Ok(None);
        };
        let plain = match &self.cipher {
            None => Some(col),
            Some(c) => c
                .open(&ad(AD_SOURCE, &[&source_id(r)]), &col)
                .ok()
                .and_then(|p| unpad(&p).map(<[u8]>::to_vec)),
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
                        Some(c) => c
                            .open(&ad(AD_NS_KEY, &[&tag]), col)
                            .ok()
                            .and_then(|p| unpad(&p).map(<[u8]>::to_vec)),
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

    #[test]
    fn the_created_schema_is_the_exact_schema() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        let objects = master(&conn).unwrap();
        assert!(schema_is_exact(&conn, &objects).unwrap());
        conn.execute_batch("CREATE INDEX extra ON namespace(minor)")
            .unwrap();
        assert!(!schema_is_exact(&conn, &master(&conn).unwrap()).unwrap());
    }
}
