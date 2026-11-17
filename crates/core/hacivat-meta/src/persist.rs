//! The per-profile catalog cache (HV-MODEL-03): one SQLite file per connection profile and
//! format major, `<data>/catalog/<profile-uuid>.v<major>.sqlite` (§5.6), WAL mode, `0600`.
//! Each namespace is one row whose payload is [`crate::encode_namespace`]'s bytes, so a
//! namespace refresh is one transaction (crash-safe without tmp + rename) and loading can be
//! lazy per namespace. The cache is disposable: a corrupt file is renamed
//! `*.corrupt-<unix seconds>` and rebuilt, a corrupt row is dropped.
//!
//! Not in the spike (M4): encryption of payloads for sensitive profiles (HV-SEC-05 data key,
//! wrapping the payload), the server-fingerprint check, the multi-instance watermark
//! compare-and-set, debounced saves and eviction.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, params};

use crate::codec::{
    DecodeError, FORMAT_MAJOR, FORMAT_MINOR, decode_namespace, encode_namespace, encode_ns_key,
};
use crate::ids::{NsKey, SourceRef};
use crate::namespace::{NamespaceSnapshot, ObjFlags};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DDL: &str = "
CREATE TABLE IF NOT EXISTS header(key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS namespace(
    id INTEGER PRIMARY KEY,
    ns_key BLOB NOT NULL UNIQUE,
    minor INTEGER NOT NULL,
    level INTEGER NOT NULL,
    loaded_at INTEGER NOT NULL,
    saved_at INTEGER NOT NULL,
    payload BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS source(hash BLOB PRIMARY KEY, len INTEGER NOT NULL, text BLOB NOT NULL);
";

/// The cache file name of `profile` at the current format major.
pub fn cache_file_name(profile: &ProfileId) -> String {
    format!("{profile}.v{FORMAT_MAJOR}.sqlite")
}

#[derive(Debug)]
pub enum CacheError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    /// The file's header names another format major (never by naming; a hand-copied file).
    Incompatible {
        major: u16,
    },
    /// A temp (session) namespace is never persisted (HV-MODEL-01 edge 8).
    TempNamespace,
    /// The namespace was decoded from a newer minor and is not written back (HV-MODEL-03
    /// edge 4); a fresh introspection replaces it.
    NewerMinor,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Sqlite(e) => write!(f, "catalog cache: {e}"),
            CacheError::Io(e) => write!(f, "catalog cache: {e}"),
            CacheError::Incompatible { major } => {
                write!(f, "catalog cache format {major}, expected {FORMAT_MAJOR}")
            }
            CacheError::TempNamespace => write!(f, "temp namespaces are not cached"),
            CacheError::NewerMinor => write!(f, "namespace is from a newer cache format"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<rusqlite::Error> for CacheError {
    fn from(e: rusqlite::Error) -> Self {
        CacheError::Sqlite(e)
    }
}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        CacheError::Io(e)
    }
}

/// Why [`CacheStore::open`] started an empty cache (`model-cache-rebuilt`, `$reason`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildReason {
    /// The file was not a database or failed `quick_check`; it was renamed to this path.
    Corrupt { moved_to: PathBuf },
}

pub struct Opened {
    pub store: CacheStore,
    pub rebuilt: Option<RebuildReason>,
}

/// Everything [`CacheStore::load_all`] read.
#[derive(Debug)]
pub struct LoadAll {
    pub namespaces: Vec<NamespaceSnapshot>,
    /// Rows that failed to decode (bit flip, foreign major): deleted from the cache; their
    /// namespaces are re-introspected.
    pub dropped: Vec<(Vec<u8>, DecodeError)>,
}

/// One profile's catalog cache. `Send`, not `Sync`: one owner on a blocking thread (§5.4).
pub struct CacheStore {
    conn: Connection,
    path: PathBuf,
}

impl fmt::Debug for CacheStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheStore")
            .field("path", &self.path)
            .finish()
    }
}

enum Fail {
    Corrupt,
    Other(CacheError),
}

fn classify(e: rusqlite::Error) -> Fail {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => Fail::Corrupt,
        _ => Fail::Other(CacheError::Sqlite(e)),
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl CacheStore {
    /// Opens (or creates) the cache file at `path`. A corrupt file is renamed and replaced by
    /// an empty cache.
    pub fn open(path: &Path) -> Result<Opened, CacheError> {
        prepare_file(path)?;
        match try_open(path) {
            Ok(conn) => Ok(Opened {
                store: CacheStore {
                    conn,
                    path: path.to_path_buf(),
                },
                rebuilt: None,
            }),
            Err(Fail::Other(e)) => Err(e),
            Err(Fail::Corrupt) => {
                let moved_to = rotate(path)?;
                prepare_file(path)?;
                let conn = try_open(path).map_err(|f| match f {
                    Fail::Corrupt => {
                        CacheError::Io(std::io::Error::other("a fresh catalog cache is corrupt"))
                    }
                    Fail::Other(e) => e,
                })?;
                Ok(Opened {
                    store: CacheStore {
                        conn,
                        path: path.to_path_buf(),
                    },
                    rebuilt: Some(RebuildReason::Corrupt { moved_to }),
                })
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `ns` (one transaction; replaces the namespace's previous row).
    pub fn save_namespace(&mut self, ns: &NamespaceSnapshot) -> Result<(), CacheError> {
        if ns.root().flags.contains(ObjFlags::TEMP) {
            return Err(CacheError::TempNamespace);
        }
        if ns.from_newer_minor() {
            return Err(CacheError::NewerMinor);
        }
        let payload = encode_namespace(ns);
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO namespace(ns_key, minor, level, loaded_at, saved_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ns_key) DO UPDATE SET minor = excluded.minor, level = excluded.level,
                 loaded_at = excluded.loaded_at, saved_at = excluded.saved_at,
                 payload = excluded.payload",
            params![
                encode_ns_key(ns.key()),
                FORMAT_MINOR,
                i64::from(ns.meta().level.code()),
                ns.meta().loaded_at_ms,
                unix_ms(),
                payload
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Reads one namespace. `Ok(None)` when absent; a row that fails to decode is deleted and
    /// reported as `Ok(None)` with the error in the second slot.
    pub fn load_namespace(
        &self,
        key: &NsKey,
    ) -> Result<(Option<NamespaceSnapshot>, Option<DecodeError>), CacheError> {
        let k = encode_ns_key(key);
        let payload: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT payload FROM namespace WHERE ns_key = ?1",
                [&k],
                |r| r.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok((None, None));
        };
        match decode_namespace(&payload) {
            Ok(ns) => Ok((Some(ns), None)),
            Err(e) => {
                self.conn
                    .execute("DELETE FROM namespace WHERE ns_key = ?1", [&k])?;
                Ok((None, Some(e)))
            }
        }
    }

    /// Reads every cached namespace (the connect-time load of the 160k-column budget).
    pub fn load_all(&self) -> Result<LoadAll, CacheError> {
        let mut stmt = self
            .conn
            .prepare("SELECT ns_key, payload FROM namespace ORDER BY ns_key")?;
        let mut rows = stmt.query([])?;
        let mut namespaces = Vec::new();
        let mut dropped = Vec::new();
        while let Some(row) = rows.next()? {
            let key: Vec<u8> = row.get(0)?;
            let payload = row.get_ref(1)?.as_blob().map_err(rusqlite::Error::from)?;
            match decode_namespace(payload) {
                Ok(ns) => namespaces.push(ns),
                Err(e) => dropped.push((key, e)),
            }
        }
        drop(rows);
        drop(stmt);
        for (k, _) in &dropped {
            self.conn
                .execute("DELETE FROM namespace WHERE ns_key = ?1", [k])?;
        }
        Ok(LoadAll {
            namespaces,
            dropped,
        })
    }

    /// Removes a namespace (a dropped schema).
    pub fn remove_namespace(&mut self, key: &NsKey) -> Result<bool, CacheError> {
        Ok(self.conn.execute(
            "DELETE FROM namespace WHERE ns_key = ?1",
            [encode_ns_key(key)],
        )? > 0)
    }

    /// Stores source text under its content hash; returns the reference the model keeps.
    pub fn put_source(&mut self, text: &str) -> Result<SourceRef, CacheError> {
        let r = SourceRef::of(text);
        self.conn.execute(
            "INSERT OR IGNORE INTO source(hash, len, text) VALUES (?1, ?2, ?3)",
            params![r.hash.to_le_bytes().to_vec(), r.len, text.as_bytes()],
        )?;
        Ok(r)
    }

    /// The source text for `r`, if cached.
    pub fn source(&self, r: SourceRef) -> Result<Option<String>, CacheError> {
        let bytes: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT text FROM source WHERE hash = ?1",
                [r.hash.to_le_bytes().to_vec()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(bytes.and_then(|b| String::from_utf8(b).ok()))
    }
}

fn try_open(path: &Path) -> Result<Connection, Fail> {
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
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))
        .map_err(classify)?;
    conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY")
        .map_err(classify)?;
    conn.execute_batch(&format!("BEGIN IMMEDIATE; {DDL} COMMIT;"))
        .map_err(classify)?;
    let major: Option<i64> = conn
        .query_row(
            "SELECT value FROM header WHERE key = 'format_major'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(classify)?;
    match major {
        None => {
            conn.execute(
                "INSERT INTO header(key, value) VALUES ('format_major', ?1), \
                 ('format_minor', ?2), ('created_at', ?3)",
                params![FORMAT_MAJOR, FORMAT_MINOR, unix_ms()],
            )
            .map_err(classify)?;
        }
        Some(m) if m == i64::from(FORMAT_MAJOR) => {}
        Some(m) => {
            return Err(Fail::Other(CacheError::Incompatible {
                major: u16::try_from(m).unwrap_or(u16::MAX),
            }));
        }
    }
    Ok(conn)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Creates the file owner-only, or tightens an existing one (and its `-wal` / `-shm`).
fn prepare_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        for p in [
            path.to_path_buf(),
            sibling(path, "-wal"),
            sibling(path, "-shm"),
        ] {
            if let Ok(m) = std::fs::metadata(&p)
                && m.permissions().mode() & 0o077 != 0
            {
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
            }
        }
    }
    #[cfg(not(unix))]
    {
        // The per-user application-data folder is owner-only on Windows.
        let _ = path;
    }
    Ok(())
}

/// Renames a corrupt file (and its `-wal`, `-shm`) to `<name>.corrupt-<unix seconds>[-n]`.
fn rotate(path: &Path) -> std::io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut target = sibling(path, &format!(".corrupt-{secs}"));
    let mut n = 1u32;
    while target.exists() {
        target = sibling(path, &format!(".corrupt-{secs}-{n}"));
        n += 1;
    }
    // The log first: an old `-wal` must never sit beside a fresh file.
    for suffix in ["-wal", "-shm"] {
        let from = sibling(path, suffix);
        if from.exists() {
            std::fs::rename(&from, sibling(&target, suffix))?;
        }
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}
