//! Recovery copies of unsaved script text under `LocalPaths::script_recovery_dir`, never
//! inside a library (HV-SCRIPT-07 step 5; R-M4b-5; 0700, files 0600). Task 5 writes it.
//!
//! ## m4b-t01a-seams: declared by Task 1a so later tasks add no `lib.rs` line.
//!
//! ## m4b-t05-fileio
//! Task 5: scripts save only on ⌘S (Q1), so a dirty tab's text is copied here after 5 s idle
//! (Task 14 drives the timer) and offered back on the next launch. One copy per script:
//! `<data>/script-recovery/<library uuid>/<xxh3-128(rel path) hex>.sql` holds the text as UTF-8,
//! and a `.meta` sidecar beside it (TOML: `format`, `rel_path`, `saved_at`, the stamp the
//! edits started from) says what it belongs to. The file names say nothing about the script.
//! Both are written with `fsio::atomic_write` (0600) into folders made 0700, the text before
//! its sidecar, so a sidecar never names a half-written copy.
//!
//! [`take_all`] reads and validates without deleting, so a second crash before the user
//! answers still loses nothing; [`discard`] removes a copy (after a save, or when the user
//! discards it). Entries that do not check out (a sidecar whose path is not library-relative
//! or does not hash to its file name, a newer format, text that is not UTF-8, a sidecar
//! without its copy) are skipped, never trusted. When `script_recovery_dir` answers `None`,
//! nothing is written and nothing is read: there is no fallback folder (carries-w1 T5, m5).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_settings::fsio;
use hacivat_settings::paths::LocalPaths;
use serde::{Deserialize, Serialize};
use twox_hash::XxHash3_128;

use crate::fileio::DiskStamp;
use crate::pathsafe::RelPath;
use crate::registry::LibraryId;

/// The sidecar format this build writes and reads.
const FORMAT: u32 = 1;
const TEXT_EXT: &str = "sql";
const META_EXT: &str = "meta";
/// The largest sidecar read; ours are a few hundred bytes.
const META_MAX_BYTES: u64 = 16 << 10;

/// A recovery copy offered back. Its `Debug` never shows the text.
#[derive(Clone)]
pub struct Recovered {
    pub rel: RelPath,
    pub text: String,
    pub saved_at: SystemTime,
    /// The file's stamp when the unsaved edits began; `None` for a file that was new.
    pub base: Option<DiskStamp>,
}

impl fmt::Debug for Recovered {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recovered")
            .field("rel", &self.rel)
            .field("text_len", &self.text.len())
            .field("saved_at", &self.saved_at)
            .field("base", &self.base)
            .finish()
    }
}

/// Why a copy was not written or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// `script_recovery_dir` has no folder for this library: nothing is written anywhere.
    NoFolder,
    Io(io::ErrorKind),
}

impl From<io::Error> for RecoveryError {
    fn from(e: io::Error) -> Self {
        RecoveryError::Io(e.kind())
    }
}

/// Stores (or replaces) the recovery copy of `rel` in library `lib`.
pub fn store(
    paths: &LocalPaths,
    lib: LibraryId,
    rel: &RelPath,
    text: &str,
    base: Option<DiskStamp>,
) -> Result<(), RecoveryError> {
    store_in(folder(paths, lib), rel, text, base, SystemTime::now())
}

/// Every recovery copy of library `lib` that checks out, in no particular order. Nothing is
/// deleted.
pub fn take_all(paths: &LocalPaths, lib: LibraryId) -> Vec<Recovered> {
    folder(paths, lib).map_or_else(Vec::new, |dir| read_all(&dir))
}

/// Removes the recovery copy of `rel`, if there is one.
pub fn discard(paths: &LocalPaths, lib: LibraryId, rel: &RelPath) -> Result<(), RecoveryError> {
    let Some(dir) = folder(paths, lib) else {
        return Err(RecoveryError::NoFolder);
    };
    let stem = stem_of(rel);
    // The sidecar first: a copy without one is never offered.
    for ext in [META_EXT, TEXT_EXT] {
        match fs::remove_file(dir.join(format!("{stem}.{ext}"))) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn folder(paths: &LocalPaths, lib: LibraryId) -> Option<PathBuf> {
    paths.script_recovery_dir(&lib.to_string())
}

fn store_in(
    dir: Option<PathBuf>,
    rel: &RelPath,
    text: &str,
    base: Option<DiskStamp>,
    now: SystemTime,
) -> Result<(), RecoveryError> {
    let dir = dir.ok_or(RecoveryError::NoFolder)?;
    make_private(&dir)?;
    let stem = stem_of(rel);
    let meta = Sidecar::new(rel, now, base);
    let meta = toml::to_string(&meta).map_err(|_| RecoveryError::Io(io::ErrorKind::InvalidData))?;
    fsio::atomic_write(&dir.join(format!("{stem}.{TEXT_EXT}")), text.as_bytes())?;
    fsio::atomic_write(&dir.join(format!("{stem}.{META_EXT}")), meta.as_bytes())?;
    Ok(())
}

/// `<data>/script-recovery` and the library's folder in it, 0700 whether new or not.
fn make_private(dir: &Path) -> io::Result<()> {
    fsio::create_dir_all_private(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let private = || fs::Permissions::from_mode(0o700);
        fs::set_permissions(dir, private())?;
        if let Some(parent) = dir.parent() {
            fs::set_permissions(parent, private())?;
        }
    }
    Ok(())
}

/// The file stem of `rel`'s copy: the XXH3-128 of its exact text, 32 hex digits.
fn stem_of(rel: &RelPath) -> String {
    format!("{:032x}", XxHash3_128::oneshot(rel.as_str().as_bytes()))
}

fn read_all(dir: &Path) -> Vec<Recovered> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(META_EXT) {
            continue;
        }
        if let Some(r) = read_one(dir, &path) {
            out.push(r);
        }
    }
    out
}

fn read_one(dir: &Path, meta_path: &Path) -> Option<Recovered> {
    let stem = meta_path.file_stem()?.to_str()?;
    let meta = fs::symlink_metadata(meta_path).ok()?;
    if !meta.is_file() || meta.len() > META_MAX_BYTES {
        return None;
    }
    let sidecar: Sidecar = toml::from_str(&fs::read_to_string(meta_path).ok()?).ok()?;
    if sidecar.format != FORMAT {
        return None;
    }
    let rel = RelPath::parse(&sidecar.rel_path).ok()?;
    if stem_of(&rel) != stem {
        return None;
    }
    let text_path = dir.join(format!("{stem}.{TEXT_EXT}"));
    if !fs::symlink_metadata(&text_path).ok()?.is_file() {
        return None;
    }
    let text = String::from_utf8(fs::read(&text_path).ok()?).ok()?;
    Some(Recovered {
        rel,
        text,
        saved_at: time_of(sidecar.saved_at_secs, sidecar.saved_at_nanos)?,
        base: match sidecar.base {
            None => None,
            Some(b) => Some(b.stamp()?),
        },
    })
}

/// The sidecar, as written. `u128` and pre-1970 times do not fit TOML integers, so the hash
/// is hex text and times are seconds and nanoseconds since 1970.
#[derive(Serialize, Deserialize)]
struct Sidecar {
    format: u32,
    rel_path: String,
    saved_at_secs: u64,
    saved_at_nanos: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<BaseStamp>,
}

#[derive(Serialize, Deserialize)]
struct BaseStamp {
    len: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    hash: String,
}

impl Sidecar {
    fn new(rel: &RelPath, now: SystemTime, base: Option<DiskStamp>) -> Self {
        let since = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok();
        let saved = since(now).unwrap_or_default();
        Sidecar {
            format: FORMAT,
            rel_path: rel.as_str().to_owned(),
            saved_at_secs: saved.as_secs(),
            saved_at_nanos: saved.subsec_nanos(),
            // A file stamped before 1970 keeps no base: the app then treats it as changed.
            base: base.and_then(|b| {
                let m = since(b.mtime)?;
                Some(BaseStamp {
                    len: b.len,
                    mtime_secs: m.as_secs(),
                    mtime_nanos: m.subsec_nanos(),
                    hash: format!("{:032x}", b.hash),
                })
            }),
        }
    }
}

impl BaseStamp {
    fn stamp(&self) -> Option<DiskStamp> {
        if self.hash.len() != 32 {
            return None;
        }
        Some(DiskStamp {
            len: self.len,
            mtime: time_of(self.mtime_secs, self.mtime_nanos)?,
            hash: u128::from_str_radix(&self.hash, 16).ok()?,
        })
    }
}

fn time_of(secs: u64, nanos: u32) -> Option<SystemTime> {
    if nanos >= 1_000_000_000 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::new(secs, nanos))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// carries-w1 T5 (m5): no folder → no copy, and no fallback folder anywhere.
    #[test]
    fn no_folder_writes_nothing() {
        let rel = RelPath::parse("a.sql").unwrap();
        assert_eq!(
            store_in(None, &rel, "x", None, SystemTime::now()),
            Err(RecoveryError::NoFolder)
        );
    }

    #[test]
    fn a_pre_1970_base_is_dropped_not_wrapped() {
        let rel = RelPath::parse("a.sql").unwrap();
        let base = DiskStamp {
            len: 1,
            mtime: UNIX_EPOCH - Duration::from_secs(5),
            hash: 7,
        };
        assert!(
            Sidecar::new(&rel, SystemTime::now(), Some(base))
                .base
                .is_none()
        );
        let bad = BaseStamp {
            len: 1,
            mtime_secs: 1,
            mtime_nanos: 2_000_000_000,
            hash: format!("{:032x}", 7),
        };
        assert!(bad.stamp().is_none());
    }
}
// ## m4b-t05-fileio
