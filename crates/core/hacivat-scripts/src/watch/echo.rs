//! Hacivat's own writes, so the watcher does not report them (HV-SCRIPT-08 step 6; R-M4b-16).
//!
//! ## m4b-t09-watcher
//! The save path records the stamp of what it wrote before the rename
//! ([`fileio::save_with_echo`](crate::fileio::save_with_echo), HV-SCRIPT-07 step 4). A file
//! the watcher has reconciled is an echo when its full stamp `(len, mtime, xxh3-128)` equals a
//! record younger than [`ECHO_TTL`]. The hash is computed only when `(len, mtime)` already
//! match a record ([`EchoTable::may_be_echo_at`]), so a foreign edit costs no read.
//!
//! The save's temp file (`.<name>.hv-<random>.tmp`) starts with `.`, so the watcher drops its
//! hints by name and the table needs no entry for it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use twox_hash::XxHash3_128;

use crate::fileio::DiskStamp;
// ## m4b-t20-adversarial (named line, T9 m9)
use crate::fileio::dir::{Dir, FsError, is_link};
use crate::pathsafe::RelPath;

/// How long a recorded write stays an echo.
pub const ECHO_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// Records kept per path: two saves in quick succession must both stay echoes, because the
/// watcher may look at the disk between them.
const PER_PATH: usize = 4;

/// Paths kept at most; past it the oldest records go first (a bound, never reached by a person
/// saving files).
const MAX_PATHS: usize = 1_024;

/// The bytes hashed per read.
const HASH_CHUNK: usize = 1 << 20;

/// Recent writes by library-relative path.
#[derive(Debug, Default)]
pub struct EchoTable {
    by_rel: HashMap<RelPath, Vec<(DiskStamp, Instant)>>,
}

impl EchoTable {
    /// Records that Hacivat wrote `stamp` to `rel` now.
    pub fn record(&mut self, rel: RelPath, stamp: DiskStamp) {
        self.record_at(rel, stamp, Instant::now());
    }

    /// [`record`](Self::record) at `now`.
    pub fn record_at(&mut self, rel: RelPath, stamp: DiskStamp, now: Instant) {
        self.expire(now);
        if self.by_rel.len() >= MAX_PATHS && !self.by_rel.contains_key(&rel) {
            let oldest = self
                .by_rel
                .iter()
                .filter_map(|(k, v)| v.iter().map(|(_, at)| *at).max().map(|at| (at, k.clone())))
                .min_by_key(|(at, _)| *at)
                .map(|(_, k)| k);
            if let Some(k) = oldest {
                self.by_rel.remove(&k);
            }
        }
        let list = self.by_rel.entry(rel).or_default();
        if list.len() >= PER_PATH {
            list.remove(0);
        }
        list.push((stamp, now));
    }

    /// Whether `stamp` at `rel` is one of Hacivat's own writes of the last [`ECHO_TTL`].
    pub fn is_echo(&self, rel: &RelPath, stamp: &DiskStamp) -> bool {
        self.is_echo_at(rel, stamp, Instant::now())
    }

    /// [`is_echo`](Self::is_echo) at `now`.
    pub fn is_echo_at(&self, rel: &RelPath, stamp: &DiskStamp, now: Instant) -> bool {
        self.live(rel, now).any(|s| s == stamp)
    }

    /// Whether some live record of `rel` has this length and modification time: only then is
    /// the file worth hashing.
    pub fn may_be_echo_at(&self, rel: &RelPath, len: u64, mtime: SystemTime, now: Instant) -> bool {
        self.live(rel, now)
            .any(|s| s.len == len && s.mtime == mtime)
    }

    /// Drops the records older than [`ECHO_TTL`].
    pub fn expire(&mut self, now: Instant) {
        self.by_rel.retain(|_, list| {
            list.retain(|(_, at)| fresh(*at, now));
            !list.is_empty()
        });
    }

    /// The number of paths with a live record (for tests and diagnostics).
    pub fn len(&self) -> usize {
        self.by_rel.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_rel.is_empty()
    }

    fn live<'a>(&'a self, rel: &RelPath, now: Instant) -> impl Iterator<Item = &'a DiskStamp> {
        self.by_rel
            .get(rel)
            .into_iter()
            .flatten()
            .filter(move |(_, at)| fresh(*at, now))
            .map(|(s, _)| s)
    }
}

fn fresh(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < ECHO_TTL
}

/// The echo table a watcher reads and the save path writes, shared between them.
///
/// The library service records with it from `fileio::save_with_echo`'s callback, under the
/// path the bytes landed at: for a file link, the target's path (`pathsafe::Joined::target`),
/// since that is the path the watcher sees change.
#[derive(Debug, Clone, Default)]
pub struct EchoHandle(Arc<Mutex<EchoTable>>);

impl EchoHandle {
    /// Records that Hacivat is writing `stamp` to `rel`.
    pub fn record(&self, rel: RelPath, stamp: DiskStamp) {
        self.lock().record(rel, stamp);
    }

    /// The table, locked (never held across file I/O).
    pub fn lock(&self) -> MutexGuard<'_, EchoTable> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The stamp of the regular file `rel` under the canonical `root`, hashed through a handle that
/// follows no link and never blocks on a FIFO, reading at most `expect_len + 1` bytes: `None`
/// when it is not that long, or not a regular file.
///
/// ## m4b-t20-adversarial (named exception, T9 review m9): the folders are walked from the
/// stored canonical root with T5's no-follow folder handles (`fileio::dir::Dir`, identities
/// re-checked around every step), never a path-based open after the caller's `join`, so a
/// folder swapped for a link in between is refused instead of followed out of the library.
///
/// The hash and the length are those of `fileio`'s stamps (XXH3-128 of every byte, the length
/// read, the handle's modification time), so the two compare equal.
pub(crate) fn stamp_if_len(
    root: &Path,
    rel: &RelPath,
    expect_len: u64,
) -> io::Result<Option<DiskStamp>> {
    let Some(file) = open_under(root, rel)? else {
        return Ok(None);
    };
    let before = file.metadata()?;
    if !before.is_file() {
        return Ok(None);
    }
    let mut hasher = XxHash3_128::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    let mut total = 0u64;
    let mut reader = (&file).take(expect_len.saturating_add(1));
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.write(&buf[..n]);
                total += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    if total != expect_len {
        return Ok(None);
    }
    let meta = file.metadata()?;
    Ok(Some(DiskStamp {
        len: total,
        mtime: meta.modified().unwrap_or(UNIX_EPOCH),
        hash: hasher.finish_128(),
    }))
}

/// `rel` opened for reading under `root` through no-follow folder handles: `None` when a link,
/// a non-folder on the way or a non-regular file is met (a scan decides what it is now).
fn open_under(root: &Path, rel: &RelPath) -> io::Result<Option<File>> {
    let to_io = |e: FsError| match e {
        FsError::Io(e) => Some(e),
        _ => None,
    };
    let parts: Vec<&str> = rel.components().collect();
    let Some((name, folders)) = parts.split_last() else {
        return Ok(None);
    };
    let mut dir = match Dir::open_canonical(root) {
        Ok(d) => d,
        Err(e) => return to_io(e).map_or(Ok(None), Err),
    };
    for folder in folders {
        dir = match dir.subdir(folder) {
            Ok(d) => d,
            Err(e) => return to_io(e).map_or(Ok(None), Err),
        };
    }
    let lstat = match dir.lstat(name) {
        Ok(m) => m,
        Err(e) => return to_io(e).map_or(Ok(None), Err),
    };
    if is_link(&lstat) || !lstat.is_file() {
        return Ok(None);
    }
    match dir.open_read(name, &lstat) {
        Ok(f) => Ok(Some(f)),
        Err(e) => to_io(e).map_or(Ok(None), Err),
    }
}
// ## m4b-t09-watcher

// ## m4b-t20-adversarial (named exception, carry T9 review m9)
#[cfg(all(test, unix))]
mod t20_tests {
    use super::*;

    /// T9 review m9: the hash read walks the folders from the canonical root with handles that
    /// follow no link, so a folder swapped for a link after the reconciler's `join` is never
    /// followed out of the library, even to a file of the expected length.
    #[test]
    fn hash_read_never_follows_a_folder_swapped_for_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(tmp.path()).unwrap();
        let root = base.join("lib");
        let outside = base.join("outside");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(outside.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.sql"), b"inside").unwrap();
        std::fs::write(outside.join("sub/a.sql"), b"secret").unwrap();
        let rel = RelPath::parse("sub/a.sql").unwrap();
        assert!(stamp_if_len(&root, &rel, 6).unwrap().is_some());
        // The folder becomes a link to a folder outside holding a same-length file.
        std::fs::remove_dir_all(root.join("sub")).unwrap();
        std::os::unix::fs::symlink(outside.join("sub"), root.join("sub")).unwrap();
        assert!(!matches!(stamp_if_len(&root, &rel, 6), Ok(Some(_))));
        // A final link is never followed either.
        std::fs::remove_file(root.join("sub")).unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::os::unix::fs::symlink(outside.join("sub/a.sql"), root.join("sub/a.sql")).unwrap();
        assert!(!matches!(stamp_if_len(&root, &rel, 6), Ok(Some(_))));
        // The root itself replaced by a link is refused too.
        std::fs::rename(&root, base.join("lib-moved")).unwrap();
        std::os::unix::fs::symlink(base.join("lib-moved"), &root).unwrap();
        std::fs::remove_file(base.join("lib-moved/sub/a.sql")).unwrap();
        std::fs::write(base.join("lib-moved/sub/a.sql"), b"inside").unwrap();
        assert!(!matches!(stamp_if_len(&root, &rel, 6), Ok(Some(_))));
    }
}
// ## m4b-t20-adversarial
