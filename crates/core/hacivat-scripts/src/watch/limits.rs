//! Which backend a library gets (HV-SCRIPT-08 step 1; R-M4b-14).
//!
//! ## m4b-t09-watcher
//! - **Network file systems are polled**: inotify "does not catch remote events that occur on
//!   network filesystems" (`inotify(7)`), and FSEvents and `ReadDirectoryChangesW` report only
//!   this machine's own writes there. Detected on Linux from `/proc/self/mountinfo` (the
//!   longest mount point that holds the root, and its type). On macOS and Windows the type
//!   would need `statfs` / `GetDriveTypeW`, which this crate cannot call without `unsafe`; it
//!   answers [`FsKind::Unknown`] there (watched by the OS; [Unknown] U-09, ADR 0009 `m4b-t09`).
//! - **Linux inotify budget**: inotify needs one watch per folder (it is not recursive), out of
//!   `max_user_watches` per user. The budget is that limit minus the inotify watches this
//!   user's processes already hold (counted from `/proc/<pid>/fdinfo`, the processes we can
//!   read). A library whose folders would take more than half the free budget is polled.

use std::fs;
use std::path::{Path, PathBuf};

use super::Backend;
use crate::service::PollReason;

/// What kind of file system holds a library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    Local,
    /// NFS, SMB/CIFS, AFS, 9p, FUSE network mounts, and the like: polled.
    Network,
    /// Not known on this system (macOS, Windows): treated as local.
    Unknown,
}

/// The file system types (`mountinfo`'s field after ` - `) that are polled.
const NETWORK_TYPES: [&str; 17] = [
    "nfs",
    "nfs4",
    "cifs",
    "smb3",
    "smbfs",
    "ncpfs",
    "afs",
    "9p",
    "ceph",
    "glusterfs",
    "lustre",
    "gpfs",
    "davfs",
    "fuse.sshfs",
    "fuse.rclone",
    "fuse.s3fs",
    "fuse.davfs2",
];

/// The R-M4b-14 rule: a network file system is polled; with an inotify budget, a library
/// whose `dirs` folders would take more than half of the `budget` free watches is polled.
pub fn choose_backend(dirs: u64, budget: Option<u64>, fs: FsKind) -> Backend {
    if fs == FsKind::Network {
        return Backend::Poll(PollReason::NetworkFs);
    }
    if let Some(free) = budget
        && dirs.saturating_mul(2) > free
    {
        return Backend::Poll(PollReason::WatchBudget);
    }
    Backend::Os
}

/// The backend for the library at `root` (canonical), measured now.
pub fn backend_for(root: &Path) -> Backend {
    let fs = fs_kind(root);
    let budget = if fs == FsKind::Network {
        None
    } else {
        inotify_budget()
    };
    // Counting stops once the answer is known.
    let dirs = budget.map_or(0, |free| count_dirs(root, free / 2 + 1));
    choose_backend(dirs, budget, fs)
}

/// The free inotify watches of this user (Linux); `None` elsewhere.
pub fn inotify_budget() -> Option<u64> {
    if cfg!(target_os = "linux") {
        inotify_budget_at(Path::new("/proc"))
    } else {
        None
    }
}

/// [`inotify_budget`] read from a `/proc`-shaped folder (tests give it a made-up one).
pub fn inotify_budget_at(proc_root: &Path) -> Option<u64> {
    let max = fs::read_to_string(proc_root.join("sys/fs/inotify/max_user_watches")).ok()?;
    budget_from(&max, watches_in_use(proc_root))
}

/// `max_user_watches` (its file's text) minus the watches in use.
pub fn budget_from(max_user_watches: &str, in_use: u64) -> Option<u64> {
    let max: u64 = max_user_watches.trim().parse().ok()?;
    Some(max.saturating_sub(in_use))
}

/// Processes and descriptors looked at most when counting watches in use.
const MAX_PIDS: usize = 65_536;
const MAX_FDS: usize = 65_536;

/// The inotify watches held by the processes whose descriptors we can read: one
/// `inotify wd:` line per watch in `fdinfo` of every inotify descriptor.
fn watches_in_use(proc_root: &Path) -> u64 {
    let Ok(pids) = fs::read_dir(proc_root) else {
        return 0;
    };
    let mut total = 0u64;
    for pid in pids.take(MAX_PIDS).flatten() {
        let name = pid.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let fd_dir = pid.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd in fds.take(MAX_FDS).flatten() {
            let is_inotify =
                fs::read_link(fd.path()).is_ok_and(|t| t.as_os_str() == "anon_inode:inotify");
            if !is_inotify {
                continue;
            }
            let info = pid.path().join("fdinfo").join(fd.file_name());
            if let Ok(text) = fs::read_to_string(info) {
                total += text
                    .lines()
                    .filter(|l| l.starts_with("inotify wd:"))
                    .count() as u64;
            }
        }
    }
    total
}

/// The folders under `root` (itself included), never through a link, counting stops at `cap`.
/// Every folder counts, skipped ones too: inotify watches them all.
pub fn count_dirs(root: &Path, cap: u64) -> u64 {
    let mut count = 1u64;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                count += 1;
                if count >= cap {
                    return count;
                }
                stack.push(e.path());
            }
        }
    }
    count
}

/// What kind of file system holds `path` (canonical).
pub fn fs_kind(path: &Path) -> FsKind {
    if cfg!(target_os = "linux") {
        match fs::read_to_string("/proc/self/mountinfo") {
            Ok(text) => fs_kind_from_mountinfo(&text, path),
            Err(_) => FsKind::Unknown,
        }
    } else {
        FsKind::Unknown
    }
}

/// [`fs_kind`] from the text of a `mountinfo` file (`proc_pid_mountinfo(5)`): the type of the
/// longest mount point that holds `path`.
pub fn fs_kind_from_mountinfo(mountinfo: &str, path: &Path) -> FsKind {
    let mut best: Option<(usize, bool)> = None;
    for line in mountinfo.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let Some(point) = left.split(' ').nth(4) else {
            continue;
        };
        let Some(kind) = right.split(' ').next() else {
            continue;
        };
        let point = unescape(point);
        if !path.starts_with(&point) {
            continue;
        }
        let depth = point.len();
        if best.is_none_or(|(d, _)| depth >= d) {
            best = Some((depth, NETWORK_TYPES.contains(&kind)));
        }
    }
    match best {
        Some((_, true)) => FsKind::Network,
        Some((_, false)) => FsKind::Local,
        None => FsKind::Unknown,
    }
}

/// `mountinfo` writes a space, tab, newline and backslash in a path as `\040`, `\011`, `\012`
/// and `\134`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('\\') {
        out.push_str(&rest[..i]);
        let code = rest.get(i + 1..i + 4);
        match code.and_then(|c| u8::from_str_radix(c, 8).ok()) {
            Some(b) => {
                out.push(char::from(b));
                rest = &rest[i + 4..];
            }
            None => {
                out.push('\\');
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}
// ## m4b-t09-watcher
