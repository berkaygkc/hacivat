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
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use twox_hash::XxHash3_128;

use crate::fileio::DiskStamp;
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

/// The stamp of the regular file at `path` (whose folders the caller has validated), hashed
/// through a handle that follows no final link and never blocks on a FIFO, reading at most
/// `expect_len + 1` bytes: `None` when it is not that long, or not a regular file.
///
/// The hash and the length are those of `fileio`'s stamps (XXH3-128 of every byte, the length
/// read, the handle's modification time), so the two compare equal.
pub(crate) fn stamp_if_len(path: &Path, expect_len: u64) -> io::Result<Option<DiskStamp>> {
    let file = open_no_follow(path)?;
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

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    /// `FILE_FLAG_OPEN_REPARSE_POINT` (`0x00200000`, `winbase.h`): open a link itself.
    const OPEN_REPARSE_POINT: u32 = 1 << 21;
    OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).open(path)
}
// ## m4b-t09-watcher
