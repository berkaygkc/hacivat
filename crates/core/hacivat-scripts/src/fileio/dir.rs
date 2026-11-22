// ## m4b-t05-fileio fix round 1 (review m1, m4, m12; carries-w1 T5)
//! Folder handles without `openat`: Task 3's `Dir` (ADR 0009 `m4b-t03`, ruling R-T3-1), moved
//! here unchanged in behaviour so the manifest writer and the script saver share one copy.
//!
//! A [`Dir`] is a folder opened without following a link (`O_DIRECTORY` plus `O_NOFOLLOW`;
//! macOS `O_NOFOLLOW_ANY`; Windows `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`,
//! held without `FILE_SHARE_DELETE`), with the folders from the root down to it and their
//! identities, re-checked around every operation. On Linux every operation goes through
//! `/proc/self/fd/<n>/`, and without procfs every write refuses (`Unsupported`, T3 review m2).
//! `std` has no `openat`, and this crate takes no `unsafe`.
//!
//! Added for Task 5: [`Dir::open_canonical`] (the root by its stored canonical path, never
//! canonicalized again), [`Dir::open_write`], a `private` flag on [`Dir::create_temp`], the
//! temp predicate as an argument of [`Dir::sweep_stale_temps`], and the Windows retry in
//! [`Dir::rename`].

use std::fs::{self, File, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How old a leftover temp file must be before a write sweeps it.
pub(crate) const STALE_TEMP_AGE: Duration = Duration::from_secs(60);
/// The most entries a sweep looks at.
const SWEEP_MAX_ENTRIES: usize = 4096;

/// What went wrong under a [`Dir`].
#[derive(Debug)]
pub(crate) enum FsError {
    /// A link (or junction) where a folder or file was expected.
    Link,
    NotDir,
    NotFile,
    TooLarge,
    /// The path no longer names the folder that was opened.
    Changed,
    Io(io::Error),
}

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> Self {
        #[cfg(unix)]
        {
            match e.raw_os_error() {
                Some(libc::ELOOP) => return FsError::Link,
                Some(libc::ENOTDIR) => return FsError::NotDir,
                _ => {}
            }
        }
        FsError::Io(e)
    }
}

pub(crate) fn is_link(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// A file's identity: device and inode on Unix. Windows has no stable equivalent in `std`; the
/// handles there are held without `FILE_SHARE_DELETE` instead, so the folder cannot change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity(u64, u64);

#[cfg(unix)]
pub(crate) fn identity(meta: &Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity(meta.dev(), meta.ino())
}

#[cfg(not(unix))]
pub(crate) fn identity(_: &Metadata) -> Identity {
    Identity(0, 0)
}

// ## m4b-t03-registry fix round 1 (review m2): the injectable probe of "no procfs".
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static NO_PROCFS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` as if this were Linux without procfs, on this thread only: every folder handle
/// opened inside refuses writes (`Unsupported`), as the real code does there. For tests.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn with_procfs_absent<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            NO_PROCFS.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(NO_PROCFS.with(|c| c.replace(true)));
    f()
}

#[cfg(any(test, feature = "test-support"))]
fn procfs_forced_absent() -> bool {
    NO_PROCFS.with(std::cell::Cell::get)
}

#[cfg(not(any(test, feature = "test-support")))]
fn procfs_forced_absent() -> bool {
    false
}

/// A folder opened without following a link, with the folders from the root down to it and
/// their identities, re-checked around every operation.
#[derive(Debug)]
pub(crate) struct Dir {
    path: PathBuf,
    /// Held for the whole operation (on Windows without `FILE_SHARE_DELETE`).
    handle: File,
    chain: Vec<(PathBuf, Identity)>,
    reach: Reach,
}

/// How operations under a [`Dir`] name their files.
#[derive(Debug)]
enum Reach {
    /// The folder's path, between identity checks (macOS with `O_NOFOLLOW_ANY` opens; Windows,
    /// where the held handles keep the folders in place).
    #[cfg_attr(
        target_os = "linux",
        expect(dead_code, reason = "Linux always goes through the handle or refuses")
    )]
    Path,
    /// Linux: `/proc/self/fd/<n>`, the handle as a path prefix.
    #[cfg_attr(
        not(target_os = "linux"),
        expect(dead_code, reason = "only Linux has a handle-relative path")
    )]
    Handle(PathBuf),
    /// Linux without procfs (T3 fix round 1, review m2): nothing can be relative to the
    /// handle, so writes refuse (`Unsupported`) instead of falling back to plain paths; reads
    /// (which re-check the file's identity) still work.
    WritesRefused,
}

impl Dir {
    /// The library root, canonicalized first (the opens below refuse every link). The manifest
    /// writer's entry.
    pub(crate) fn open_root(path: &Path) -> Result<Dir, FsError> {
        let path = fs::canonicalize(path)?;
        let lstat = fs::symlink_metadata(&path)?;
        Dir::open(path.clone(), &path, &lstat, Vec::new())
    }

    /// The library root by its stored canonical path, never canonicalized again: a root that
    /// became a link is refused, not followed (Task 5).
    pub(crate) fn open_canonical(canonical: &Path) -> Result<Dir, FsError> {
        if !canonical.is_absolute() {
            return Err(FsError::Changed);
        }
        let lstat = fs::symlink_metadata(canonical)?;
        Dir::open(canonical.to_path_buf(), canonical, &lstat, Vec::new())
    }

    /// Opens the folder at `via` (on Linux possibly a `/proc/self/fd` path), known as `path`,
    /// which `lstat` described.
    fn open(
        path: PathBuf,
        via: &Path,
        lstat: &Metadata,
        mut chain: Vec<(PathBuf, Identity)>,
    ) -> Result<Dir, FsError> {
        if is_link(lstat) {
            return Err(FsError::Link);
        }
        if !lstat.is_dir() {
            return Err(FsError::NotDir);
        }
        let handle = open_dir_handle(via)?;
        let meta = handle.metadata()?;
        if is_link(&meta) {
            return Err(FsError::Link);
        }
        if !meta.is_dir() {
            return Err(FsError::NotDir);
        }
        if identity(&meta) != identity(lstat) {
            return Err(FsError::Changed);
        }
        chain.push((path.clone(), identity(&meta)));
        let reach = if procfs_forced_absent() {
            Reach::WritesRefused
        } else {
            platform_reach(&handle, &meta)
        };
        Ok(Dir {
            path,
            handle,
            chain,
            reach,
        })
    }

    /// The path an operation on `name` uses.
    pub(crate) fn child(&self, name: &str) -> PathBuf {
        match &self.reach {
            Reach::Handle(p) => p.join(name),
            Reach::Path | Reach::WritesRefused => self.path.join(name),
        }
    }

    /// Whether this folder may be written (T3 review m2: never through plain paths on Linux).
    pub(crate) fn writable(&self) -> Result<(), FsError> {
        match self.reach {
            Reach::WritesRefused => Err(FsError::Io(io::ErrorKind::Unsupported.into())),
            Reach::Path | Reach::Handle(_) => Ok(()),
        }
    }

    /// Runs the write `op` between two [`Dir::verify`]s, if this folder may be written.
    pub(crate) fn write_checked<T>(
        &self,
        op: impl FnOnce() -> io::Result<T>,
    ) -> Result<T, FsError> {
        self.writable()?;
        self.checked(op)
    }

    /// A new temp file `name` (the caller's random name). Removed again if the after-check
    /// fails, so no temp is left in a committed folder by that path (T3 review m4). 0600 when
    /// `private`, else the umask's mode.
    pub(crate) fn create_temp(&self, name: &str, private: bool) -> Result<File, FsError> {
        self.writable()?;
        self.verify()?;
        let f = self.new_file_options(private).open(self.child(name))?;
        if let Err(e) = self.verify() {
            drop(f);
            self.discard_temp(name);
            return Err(e);
        }
        Ok(f)
    }

    /// Removes a temp file this code made, on every path, without the identity checks: its
    /// random name can only ever be ours, wherever the folder now is.
    pub(crate) fn discard_temp(&self, name: &str) {
        if self.writable().is_ok() {
            let _ = fs::remove_file(self.child(name));
        }
    }

    /// Removes temp files a crash left (T3 review m4): names `is_ours` accepts exactly, regular
    /// files older than [`STALE_TEMP_AGE`]. A younger one may be another writer's. Best effort,
    /// at most [`SWEEP_MAX_ENTRIES`] entries looked at.
    pub(crate) fn sweep_stale_temps(&self, is_ours: &dyn Fn(&str) -> bool) {
        if self.writable().is_err() || self.verify().is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(self.child("")) else {
            return;
        };
        for entry in entries.take(SWEEP_MAX_ENTRIES).flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|n| is_ours(n)) else {
                continue;
            };
            let stale = fs::symlink_metadata(self.child(name)).is_ok_and(|m| {
                m.is_file()
                    && m.modified()
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age >= STALE_TEMP_AGE)
            });
            if stale {
                self.discard_temp(name);
            }
        }
    }

    fn new_file_options(&self, private: bool) -> OpenOptions {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        no_follow(&mut opts);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        opts
    }

    /// Every folder from the root down still is the one that was opened.
    pub(crate) fn verify(&self) -> Result<(), FsError> {
        for (path, id) in &self.chain {
            let m = fs::symlink_metadata(path).map_err(|_| FsError::Changed)?;
            if is_link(&m) || !m.is_dir() || identity(&m) != *id {
                return Err(FsError::Changed);
            }
        }
        Ok(())
    }

    /// Runs `op` between two [`Dir::verify`]s.
    pub(crate) fn checked<T>(&self, op: impl FnOnce() -> io::Result<T>) -> Result<T, FsError> {
        self.verify()?;
        let out = op()?;
        self.verify()?;
        Ok(out)
    }

    pub(crate) fn lstat(&self, name: &str) -> Result<Metadata, FsError> {
        self.checked(|| fs::symlink_metadata(self.child(name)))
    }

    /// The folder `name` under this one; a link or a non-folder is refused.
    pub(crate) fn subdir(&self, name: &str) -> Result<Dir, FsError> {
        let lstat = self.lstat(name)?;
        let dir = Dir::open(
            self.path.join(name),
            &self.child(name),
            &lstat,
            self.chain.clone(),
        )?;
        self.verify()?;
        Ok(dir)
    }

    /// A new file `name` (never an existing one, never through a link), mode from the umask.
    pub(crate) fn create_new(&self, name: &str) -> Result<File, FsError> {
        let opts = self.new_file_options(false);
        self.write_checked(|| opts.open(self.child(name)))
    }

    /// `name` opened with `opts` plus the no-follow flags (and, on Unix, `O_NONBLOCK`, so a
    /// FIFO swapped in never blocks), checked to be the regular file `lstat` saw.
    fn open_file(
        &self,
        name: &str,
        opts: &mut OpenOptions,
        lstat: &Metadata,
    ) -> Result<File, FsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(no_follow_flags() | libc::O_NONBLOCK);
        }
        #[cfg(not(unix))]
        no_follow(opts);
        let f = self.checked(|| opts.open(self.child(name)))?;
        let meta = f.metadata()?;
        if is_link(&meta) {
            return Err(FsError::Link);
        }
        if !meta.is_file() {
            return Err(FsError::NotFile);
        }
        if identity(&meta) != identity(lstat) {
            return Err(FsError::Changed);
        }
        Ok(f)
    }

    /// `name` for reading: not through a link, never blocking on a FIFO, and the same file
    /// `lstat` saw.
    pub(crate) fn open_read(&self, name: &str, lstat: &Metadata) -> Result<File, FsError> {
        self.open_file(name, OpenOptions::new().read(true), lstat)
    }

    /// `name` for reading and writing, without truncating or creating (Task 5: the writable
    /// check and the in-place write), if this folder may be written.
    pub(crate) fn open_write(&self, name: &str, lstat: &Metadata) -> Result<File, FsError> {
        self.writable()?;
        self.open_file(name, OpenOptions::new().read(true).write(true), lstat)
    }

    /// `linkat`: fails when `to` exists, so nothing is ever replaced.
    pub(crate) fn hard_link(&self, from: &str, to: &str) -> Result<(), FsError> {
        self.write_checked(|| fs::hard_link(self.child(from), self.child(to)))
    }

    pub(crate) fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        self.write_checked(|| rename_retrying(&self.child(from), &self.child(to)))
    }

    /// ## m4b-t05-fileio fix round 2 (mf2): runs the write `op` after a [`Dir::verify`], and
    /// answers whether the folders still check out afterwards, instead of failing: once `op`
    /// has published a file, a moved folder is a warning, not an undone write.
    /// `between` runs after `op` and before the after-check (a test hook).
    fn write_noting_after(
        &self,
        op: impl FnOnce() -> io::Result<()>,
        between: impl FnOnce(),
    ) -> Result<bool, FsError> {
        self.writable()?;
        self.verify()?;
        op()?;
        between();
        Ok(self.verify().is_ok())
    }

    /// [`Dir::rename`], answering whether the folders still check out after it (fix round 2).
    pub(crate) fn rename_noting_after(
        &self,
        from: &str,
        to: &str,
        between: impl FnOnce(),
    ) -> Result<bool, FsError> {
        self.write_noting_after(
            || rename_retrying(&self.child(from), &self.child(to)),
            between,
        )
    }

    /// [`Dir::hard_link`] of the temp `from` to `to`, then the temp unlinked in the same step
    /// (so a folder moved right after the publish leaves no temp behind), answering whether the
    /// folders still check out after it.
    pub(crate) fn publish_link_noting_after(
        &self,
        from: &str,
        to: &str,
        between: impl FnOnce(),
    ) -> Result<bool, FsError> {
        self.write_noting_after(
            || {
                fs::hard_link(self.child(from), self.child(to))?;
                let _ = fs::remove_file(self.child(from));
                Ok(())
            },
            between,
        )
    }

    pub(crate) fn remove_file(&self, name: &str) -> Result<(), FsError> {
        self.write_checked(|| fs::remove_file(self.child(name)))
    }

    // ## m4b-t13-explorer (named exception): New folder in the Scripts tree.
    /// A new folder `name` (`mkdir`: never an existing name, never through a link).
    pub(crate) fn create_dir(&self, name: &str) -> Result<(), FsError> {
        self.write_checked(|| fs::create_dir(self.child(name)))
    }
    // ## m4b-t13-explorer

    /// Flushes the folder's entries (Unix; best effort: the file is already in place).
    pub(crate) fn sync(&self) {
        #[cfg(unix)]
        let _ = self.handle.sync_all();
        #[cfg(not(unix))]
        let _ = &self.handle;
    }
}

/// Linux: the handle as a path prefix when procfs answers for it (and names this very folder);
/// otherwise writes are refused.
#[cfg(target_os = "linux")]
fn platform_reach(handle: &File, meta: &Metadata) -> Reach {
    use std::os::fd::AsRawFd;
    let p = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
    match fs::metadata(&p) {
        Ok(m) if m.is_dir() && identity(&m) == identity(meta) => Reach::Handle(p),
        _ => Reach::WritesRefused,
    }
}

#[cfg(not(target_os = "linux"))]
fn platform_reach(_: &File, _: &Metadata) -> Reach {
    Reach::Path
}

/// Windows: a rename over a file another program holds open can fail with a sharing violation
/// for a moment (an indexer, a virus scanner); three retries with backoff (alg/28 edge case).
/// This reaches Task 3's manifest rename too (ADR 0009 `m4b-t05` fix round 2, mf5).
#[cfg(windows)]
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    retrying(
        || fs::rename(from, to),
        is_sharing_error,
        std::thread::sleep,
    )
}

#[cfg(not(windows))]
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// `ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION`, `ERROR_LOCK_VIOLATION`.
#[cfg_attr(
    not(any(windows, test)),
    expect(
        dead_code,
        reason = "the rename retry runs on Windows only; tested everywhere"
    )
)]
fn is_sharing_error(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

/// Runs `op`; on an error `retry_on` accepts, waits 50, 100, then 200 ms between three more
/// tries, then makes one last one (about 350 ms at most). `sleep` is injectable for the test.
#[cfg_attr(
    not(any(windows, test)),
    expect(
        dead_code,
        reason = "the rename retry runs on Windows only; tested everywhere"
    )
)]
fn retrying(
    mut op: impl FnMut() -> io::Result<()>,
    retry_on: fn(&io::Error) -> bool,
    mut sleep: impl FnMut(Duration),
) -> io::Result<()> {
    let mut wait = Duration::from_millis(50);
    for _ in 0..3 {
        match op() {
            Err(e) if retry_on(&e) => {
                sleep(wait);
                wait *= 2;
            }
            other => return other,
        }
    }
    op()
}

#[cfg(unix)]
fn no_follow_flags() -> i32 {
    // No link in any component, the last one included (macOS 11+). The kernel refuses it
    // together with `O_NOFOLLOW` (`EINVAL`), and it implies it.
    #[cfg(target_vendor = "apple")]
    {
        libc::O_NOFOLLOW_ANY
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        libc::O_NOFOLLOW
    }
}

#[cfg(unix)]
fn no_follow(opts: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(no_follow_flags());
}

#[cfg(windows)]
fn no_follow(opts: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    opts.custom_flags(win::FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(unix)]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | no_follow_flags())
        .open(path)
}

#[cfg(windows)]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(win::FILE_SHARE_READ | win::FILE_SHARE_WRITE)
        .custom_flags(win::FILE_FLAG_BACKUP_SEMANTICS | win::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Win32 constants (`winnt.h`, `winbase.h`), written as shifts.
#[cfg(windows)]
mod win {
    pub const FILE_SHARE_READ: u32 = 1;
    pub const FILE_SHARE_WRITE: u32 = 1 << 1;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 1 << 21;
    pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 1 << 25;
}
#[cfg(test)]
mod tests {
    use super::*;

    /// fr2 mf5: the Windows retry, with an injected sharing violation (32) and a recorded sleep.
    #[test]
    fn rename_retry_waits_and_gives_up() {
        let violation = || io::Error::from_raw_os_error(32);
        // Two violations, then success: three tries, two waits.
        let mut left = 2;
        let mut tries = 0;
        let mut waits = Vec::new();
        let got = retrying(
            || {
                tries += 1;
                if left > 0 {
                    left -= 1;
                    Err(violation())
                } else {
                    Ok(())
                }
            },
            is_sharing_error,
            |d| waits.push(d),
        );
        assert!(got.is_ok());
        assert_eq!(tries, 3);
        assert_eq!(
            waits,
            [Duration::from_millis(50), Duration::from_millis(100)]
        );
        // Always violated: four tries, three waits, then the error.
        let mut tries = 0;
        let mut waits = Vec::new();
        let got = retrying(
            || {
                tries += 1;
                Err(violation())
            },
            is_sharing_error,
            |d| waits.push(d),
        );
        assert_eq!(got.unwrap_err().raw_os_error(), Some(32));
        assert_eq!(tries, 4);
        assert_eq!(waits.iter().sum::<Duration>(), Duration::from_millis(350));
        // Another error is not retried.
        let mut tries = 0;
        let got = retrying(
            || {
                tries += 1;
                Err(io::Error::from_raw_os_error(2))
            },
            is_sharing_error,
            |_| panic!("no wait for a non-sharing error"),
        );
        assert!(got.is_err());
        assert_eq!(tries, 1);
        for code in [5, 32, 33] {
            assert!(is_sharing_error(&io::Error::from_raw_os_error(code)));
        }
    }
}
// ## m4b-t05-fileio
