//! SQLite paths, `file:` and `sqlite:` URIs, and `:memory:` (HV-DRV-04).
//!
//! - A `file:` URI is read as SQLite reads it: an authority, if any, must be empty or
//!   `localhost`; `file:///C:/x` is the Windows path `C:/x`; the path is percent-decoded once;
//!   `file::memory:` is an in-memory database. Its query is read like a URI query, and
//!   `mode=ro` maps to read-only.
//! - `sqlite:` (as in `jdbc:sqlite:`) is followed by `:memory:`, a `file:` URI or a path, read
//!   like a `file:` URI.
//! - A plain path is taken only when its last component has a SQLite extension (`.db`,
//!   `.sqlite`, `.sqlite3`, `.db3`, `.s3db`, `.sl3`, ASCII case ignored), so a socket directory
//!   or a host name is not mistaken for a file. It is kept exactly as typed: no decoding, and no
//!   expansion of `~` or `%USERNAME%`.

use hacivat_proto::DbmsKind;

use super::{Builder, decode_plain, query, strip_ci};
use crate::url::FieldId;
use crate::url::parse::ParseOutcome;

/// The in-memory database.
pub(super) const MEMORY: &str = ":memory:";

/// File extensions that mark a plain path as a SQLite database.
const EXTENSIONS: [&str; 6] = ["db", "sqlite", "sqlite3", "db3", "s3db", "sl3"];

pub(super) fn memory() -> Option<ParseOutcome> {
    Builder::new(DbmsKind::Sqlite, "memory").finish()
}

/// What follows `file:`.
pub(super) fn file_uri(rest: &str) -> Option<ParseOutcome> {
    if rest.contains('#') || rest.chars().any(char::is_control) {
        return None;
    }
    let (path, q) = rest.split_once('?').unwrap_or((rest, ""));
    let path = match path.strip_prefix("//") {
        Some(after) => {
            let slash = after.find('/')?;
            let host = &after[..slash];
            if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
                return None;
            }
            let p = &after[slash..];
            if is_drive(&p[1..]) { &p[1..] } else { p }
        }
        None => path,
    };
    let path = decode_plain(path)?;
    if path.is_empty() {
        return None;
    }
    let mut b = if path == MEMORY {
        Builder::new(DbmsKind::Sqlite, "memory")
    } else {
        let mut b = Builder::new(DbmsKind::Sqlite, "file");
        b.field("file", FieldId::File, path);
        b
    };
    for (k, v) in query(q)? {
        b.param(&k, v);
    }
    b.finish()
}

/// What follows `sqlite:`.
pub(super) fn sqlite_scheme(rest: &str) -> Option<ParseOutcome> {
    if rest == MEMORY {
        return memory();
    }
    if let Some(r) = strip_ci(rest, "file:") {
        return file_uri(r);
    }
    // `sqlite:///x` means a relative path to some tools and an absolute one to others.
    if rest.starts_with("//") {
        return None;
    }
    file_uri(rest)
}

/// A plain path to a SQLite file.
pub(super) fn plain_path(text: &str) -> Option<ParseOutcome> {
    if text.chars().any(char::is_control) || text.contains("://") {
        return None;
    }
    let name = text.rsplit(['/', '\\']).next()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || !EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
        return None;
    }
    let mut b = Builder::new(DbmsKind::Sqlite, "file");
    b.field("file", FieldId::File, text.to_string());
    b.finish()
}

/// `C:\…` or `C:/…`.
fn is_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\')
}
