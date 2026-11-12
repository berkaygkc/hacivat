//! Probes: the journal mode, and whether the file is writable (HV-CONN-07).

use hacivat_driver::ProbeWarning;
use rusqlite::{Connection, MAIN_DB};

/// The probe findings on `conn`. `read_only` is whether read-only was asked for.
///
/// The journal mode is always reported (`delete`, `wal`, `memory`, ...), in ASCII lowercase.
/// `SqliteFileNotWritable` is reported when read-write was asked for but SQLite opened the
/// main database read-only, which it does when the file is write-protected. A query that fails
/// just leaves its finding out.
pub(crate) fn probe(conn: &Connection, read_only: bool) -> Vec<ProbeWarning> {
    let mut warnings = Vec::new();
    if let Ok(mode) = conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0)) {
        warnings.push(ProbeWarning::SqliteJournal {
            mode: mode.to_ascii_lowercase(),
        });
    }
    if !read_only && conn.is_readonly(MAIN_DB).unwrap_or(false) {
        warnings.push(ProbeWarning::SqliteFileNotWritable);
    }
    warnings
}
