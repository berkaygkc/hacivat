// ## m3b-t01c-core-seams
//! The history database's schema and its migrations (Task 17): the statements table, the
//! FTS5 index (`unicode61 remove_diacritics 2` over the Turkish pre-fold) and the pins.
// ## m3b-t17-history
//!
//! `alg/20` HV-EXEC-12's schema, version 1 (`PRAGMA user_version`). The file is created
//! owner-only (`0600` on Unix, before SQLite opens it; SQLite gives the `-wal` and `-shm` files
//! the database file's mode) and a wider mode found on an existing file is tightened. The
//! directory is the app's (`LocalPaths::history_db()`'s parent, created private by the app;
//! T1a review M7). Opening runs `PRAGMA quick_check`: a file SQLite reports as not a database or
//! corrupt is renamed `history.sqlite.corrupt-<unix seconds>` (with its `-wal` and `-shm`, so
//! an old log is never replayed into the new file; the log is moved first) and a fresh file
//! takes its place. Nothing is deleted. A busy or unreadable file is an error, never a
//! rotation. Another process that still has the corrupt file open keeps writing to the moved
//! file (its own handle), not to the fresh one, until it reopens.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use super::HistoryError;

/// The schema this build writes.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// How long a write waits for another connection's lock (another window's process).
pub(crate) const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DDL: &str = "
CREATE TABLE IF NOT EXISTS history(
  id INTEGER PRIMARY KEY,
  ts_start INTEGER,
  last_run_ts INTEGER,
  duration_ms INTEGER,
  data_source_id TEXT NOT NULL,
  database TEXT,
  schema TEXT,
  client_id INTEGER,
  file_path TEXT,
  origin TEXT,
  statement_text TEXT NOT NULL,
  search_key TEXT NOT NULL,
  text_hash BLOB NOT NULL,
  fingerprint BLOB,
  params_json TEXT,
  outcome TEXT,
  rows_affected INTEGER,
  rows_fetched INTEGER,
  error_code TEXT,
  error_message TEXT,
  error_key TEXT,
  tx_mode TEXT,
  run_count INTEGER NOT NULL DEFAULT 1,
  pinned INTEGER NOT NULL DEFAULT 0,
  redacted INTEGER NOT NULL DEFAULT 0,
  script_run_id INTEGER,
  UNIQUE(data_source_id, text_hash));
CREATE VIRTUAL TABLE IF NOT EXISTS history_fts USING fts5(
  search_key, error_key, content='history', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS history_ai AFTER INSERT ON history BEGIN
  INSERT INTO history_fts(rowid, search_key, error_key)
    VALUES (new.id, new.search_key, new.error_key);
END;
CREATE TRIGGER IF NOT EXISTS history_ad AFTER DELETE ON history BEGIN
  INSERT INTO history_fts(history_fts, rowid, search_key, error_key)
    VALUES ('delete', old.id, old.search_key, old.error_key);
END;
CREATE TRIGGER IF NOT EXISTS history_au AFTER UPDATE OF search_key, error_key ON history BEGIN
  INSERT INTO history_fts(history_fts, rowid, search_key, error_key)
    VALUES ('delete', old.id, old.search_key, old.error_key);
  INSERT INTO history_fts(rowid, search_key, error_key)
    VALUES (new.id, new.search_key, new.error_key);
END;
CREATE INDEX IF NOT EXISTS history_ds_recent ON history(data_source_id, last_run_ts DESC);
CREATE INDEX IF NOT EXISTS history_fp ON history(fingerprint);
CREATE INDEX IF NOT EXISTS history_age ON history(pinned, last_run_ts);
";

/// An open, checked database and, when the file at the path was corrupt, where it went.
pub(crate) struct Opened {
    pub conn: Connection,
    pub recovered: Option<PathBuf>,
}

enum Fail {
    Corrupt,
    Other(HistoryError),
}

/// Opens (creating or migrating) the history at `path`; a corrupt file is moved aside.
pub(crate) fn open(path: &Path) -> Result<Opened, HistoryError> {
    prepare_file(path).map_err(io)?;
    match try_open(path) {
        Ok(conn) => Ok(Opened {
            conn,
            recovered: None,
        }),
        Err(Fail::Other(e)) => Err(e),
        Err(Fail::Corrupt) => {
            let moved = rotate(path).map_err(io)?;
            prepare_file(path).map_err(io)?;
            match try_open(path) {
                Ok(conn) => Ok(Opened {
                    conn,
                    recovered: Some(moved),
                }),
                Err(Fail::Corrupt) => Err(HistoryError::Corrupt),
                Err(Fail::Other(e)) => Err(e),
            }
        }
    }
}

pub(crate) fn io(e: std::io::Error) -> HistoryError {
    HistoryError::Io(e.to_string())
}

pub(crate) fn sql(e: rusqlite::Error) -> HistoryError {
    if is_corrupt(&e) {
        HistoryError::Corrupt
    } else {
        HistoryError::Io(e.to_string())
    }
}

/// SQLite says the file is not a database or is damaged.
pub(crate) fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    )
}

fn classify(e: rusqlite::Error) -> Fail {
    if is_corrupt(&e) {
        Fail::Corrupt
    } else {
        Fail::Other(sql(e))
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
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(classify)?;
    if version > SCHEMA_VERSION {
        return Err(Fail::Other(HistoryError::Io(format!(
            "history schema {version} is newer than this build's {SCHEMA_VERSION}"
        ))));
    }
    if version == 0 {
        // Before the first table: lets retention give pages back to the file system.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")
            .map_err(classify)?;
    }
    conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get::<_, String>(0))
        .map_err(classify)?;
    // Large sorts stay in memory: no (redacted) text spills into the temporary directory.
    conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY")
        .map_err(classify)?;
    conn.execute_batch(&format!(
        "BEGIN IMMEDIATE; {DDL} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
    ))
    .map_err(classify)?;
    // ## m4-t32-shots (fix round 1, review I-1; named exception): a deleted statement is
    // overwritten, not just unlinked ("Clear query history" deletes it from this computer):
    // SQLite zeroes freed content on this connection, and the full-text index drops a deleted
    // row's tokens from its segments (persistent; FTS5 `secure-delete`, SQLite 3.42+).
    conn.execute_batch(
        "PRAGMA secure_delete = ON; \
         INSERT INTO history_fts(history_fts, rank) VALUES('secure-delete', 1);",
    )
    .map_err(classify)?;
    Ok(conn)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Creates the file owner-only, or tightens an existing one (and its `-wal`/`-shm`).
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

/// Renames the corrupt file (and its `-wal`, `-shm`) to `<name>.corrupt-<unix seconds>[-n]`.
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
    // The log first: a crash between the renames must never leave an old `-wal` beside a
    // fresh file (SQLite would replay it into the new database).
    for suffix in ["-wal", "-shm"] {
        let from = sibling(path, suffix);
        if from.exists() {
            std::fs::rename(&from, sibling(&target, suffix))?;
        }
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}
