//! Recovery copies of unsaved script text under `LocalPaths::script_recovery_dir`, never
//! inside a library (HV-SCRIPT-07 step 5; R-M4b-5; 0700, files 0600). Task 5 writes it.
//!
//! ## m4b-t01a-seams: declared by Task 1a so later tasks add no `lib.rs` line.
//!
//! ## m4b-t05-fileio
//! Task 5. Scripts save only on ⌘S (Q1), so the text of a dirty tab is copied here after 5 s
//! idle (Task 14 runs the timer) and offered back on the next launch.
//!
//! ## m4b-t05-fileio fix round 1 (review I3, m7; rulings R-T5-3, R-T5-4)
//! - **One copy per tab, per app instance.** A copy is keyed by a [`RecoveryKey`] (library,
//!   path, profile, database: the tab's binding, so two tabs of one file under two connections,
//!   Q2, never share a copy) and by the [`Instance`] that wrote it. The file stem is the
//!   XXH3-128 of those parts, each length-prefixed, in hex: `<stem>.sql` holds the text (UTF-8)
//!   and `<stem>.meta` (TOML) says what it belongs to. The names say nothing about the script.
//! - **Instances.** Each launch takes an [`Instance`]: a random id and an exclusive advisory
//!   lock (`File::try_lock`, no process id) on `<data>/script-recovery/instances/<id>.lock`,
//!   held until the instance is dropped. [`take_all`] offers only copies whose instance holds
//!   no lock any more (it crashed or quit), never this instance's own and never those of
//!   another instance still running; [`discard`] removes only this instance's copy of one key.
//! - **Every copy is offered.** Two tabs of one file come back as two entries, each with its
//!   binding, encoding, BOM and line ending (m7), the stamp its edits started from, whether
//!   the caller flagged a credential-looking literal in it (R-T5-4: no redaction, Task 17's
//!   notice says so) and whether it is the text of a save that damaged the file
//!   ([`CopyMeta::damaged`], R-T5-1).
//! - **Hygiene (R-T5-4).** "Don't save" and the restore notice's "Discard" call [`discard`] or
//!   [`discard_entry`]; detaching a library calls [`discard_library`]; a copy older than
//!   [`MAX_AGE`] (14 days) is deleted instead of offered.
//! - Both files go through `fsio::atomic_write` (0600), the text before its sidecar, so a
//!   sidecar never names a half-written copy, into folders set to 0700 (also when they already
//!   existed). Entries that do not check out are skipped, never trusted. When
//!   `script_recovery_dir` answers `None`, nothing is written or read: there is no fallback
//!   folder (carries-w1 T5, m5).

use std::fmt;
use std::fs::{self, File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;
use hacivat_settings::fsio;
use hacivat_settings::paths::LocalPaths;
use serde::{Deserialize, Serialize};
use twox_hash::XxHash3_128;
use uuid::Uuid;

use crate::encoding;
use crate::fileio::{DiskStamp, FileMeta, LineEnding};
use crate::names::DatabaseName;
use crate::pathsafe::RelPath;
use crate::registry::LibraryId;

/// The sidecar format this build writes and reads.
const FORMAT: u32 = 2;
const TEXT_EXT: &str = "sql";
const META_EXT: &str = "meta";
const LOCK_EXT: &str = "lock";
/// The folder beside the libraries' folders that holds the instance locks.
const INSTANCES: &str = "instances";
/// The largest sidecar read; ours are a few hundred bytes.
const META_MAX_BYTES: u64 = 16 << 10;
/// A copy older than this is deleted, not offered (R-T5-4).
pub const MAX_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Which tab a copy belongs to: the script and the connection the tab is bound to (R-M4b-4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecoveryKey {
    pub library: LibraryId,
    pub rel: RelPath,
    /// `None` for an unbound tab.
    pub profile: Option<ProfileId>,
    pub database: Option<DatabaseName>,
}

/// What a copy records besides its text.
// ## m4b-t14-tabs (named exception, fix round 3 m-fr2-1): no longer `Copy` (it may carry the
// SQLite file's path).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CopyMeta {
    /// How the file is saved (encoding, BOM, dominant line ending).
    pub meta: FileMeta,
    /// The file's stamp when the unsaved edits began; `None` for a file that was new.
    pub base: Option<DiskStamp>,
    /// The caller's secret scan flagged a credential-looking literal (Task 17's notice).
    pub secret_suspected: bool,
    /// The text of a save that failed and could not restore the file
    /// (`SaveError::Damaged`): the file on disk is not what it was.
    pub damaged: bool,
    /// ## m4b-t14-tabs (named exception, fix round 3 m-fr2-1): the database file a SQLite
    /// script tab was bound to when the copy was written, so a copy recovered after the profile
    /// was repointed is never silently run on the new file.
    pub db_file: Option<DbFile>,
}

/// A database file as a SQLite script tab is bound to it: its canonical path and, where the
/// platform has them, its device and inode numbers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbFile {
    pub path: String,
    pub dev_ino: Option<(u64, u64)>,
}

/// A recovery copy offered back. Its `Debug` never shows the text.
#[derive(Clone)]
pub struct RecoveryEntry {
    pub key: RecoveryKey,
    pub text: String,
    pub copy: CopyMeta,
    pub saved_at: SystemTime,
    /// ## m4b-t05-fileio fix round 2 (mf3, ruling R-T5-5): whether the instance that wrote it
    /// could not be shown dead (advisory locks unsupported where it ran, or not testable
    /// here). Such a copy is still offered, marked: it may belong to a running instance.
    pub liveness_unknown: bool,
    instance: Uuid,
    stem: String,
}

impl fmt::Debug for RecoveryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecoveryEntry")
            .field("key", &self.key)
            .field("text_len", &self.text.len())
            .field("copy", &self.copy)
            .field("saved_at", &self.saved_at)
            .field("liveness_unknown", &self.liveness_unknown)
            .finish_non_exhaustive()
    }
}

/// Why a copy was not written or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// `script_recovery_dir` has no folder for this library: nothing is written anywhere.
    NoFolder,
    /// ## m4b-t05-fileio fix round 2 (I-fr1-2): the meta is `partial` (a preview, which is
    /// read-only and never has unsaved text); nothing is stored.
    PreviewOnly,
    Io(io::ErrorKind),
}

impl From<io::Error> for RecoveryError {
    fn from(e: io::Error) -> Self {
        RecoveryError::Io(e.kind())
    }
}

/// This app instance: a random id and the advisory lock that says it is alive. Take one at
/// launch and keep it until quit.
pub struct Instance {
    id: Uuid,
    /// Held locked; dropping it (or a crash) releases the lock. `None` where advisory locks
    /// are unsupported (fix round 2, mf3): the instance still writes copies, marked.
    _lock: Option<File>,
}

impl fmt::Debug for Instance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Instance")
            .field("id", &self.id)
            .field("locked", &self._lock.is_some())
            .finish()
    }
}

impl Instance {
    /// A new instance, its lock file created and locked under `<data>/script-recovery/`.
    pub fn start(paths: &LocalPaths) -> Result<Instance, RecoveryError> {
        let dir = instances_dir(paths).ok_or(RecoveryError::NoFolder)?;
        make_private(&dir)?;
        let id = Uuid::new_v4();
        let mut opts = fs::OpenOptions::new();
        opts.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let lock = opts.open(dir.join(format!("{}.{LOCK_EXT}", id.simple())))?;
        let lock = match try_lock(&lock) {
            Ok(()) => Some(lock),
            // A fresh file of our own cannot be held by anyone else.
            Err(TryLockError::WouldBlock) => {
                return Err(RecoveryError::Io(io::ErrorKind::WouldBlock));
            }
            // fr2 mf3 (R-T5-5): no advisory locks here (some NFS or SMB homes). Copies are still
            // written, so a crash loses nothing; their sidecars say the instance's liveness is
            // unknown, and a later launch offers them with that mark. Two instances that start
            // together may both offer one dead instance's copies: the M8 single-instance lock
            // closes that (carry).
            Err(TryLockError::Error(_)) => None,
        };
        sweep_dead_locks(&dir, id);
        Ok(Instance { id, _lock: lock })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Whether this instance holds its advisory lock (`false` where locks are unsupported).
    pub fn is_locked(&self) -> bool {
        self._lock.is_some()
    }
}

/// Stores (or replaces) this instance's copy for `key`.
pub fn store(
    paths: &LocalPaths,
    instance: &Instance,
    key: &RecoveryKey,
    text: &str,
    copy: &CopyMeta,
) -> Result<(), RecoveryError> {
    store_in(
        folder(paths, key.library),
        instance,
        key,
        text,
        copy,
        SystemTime::now(),
    )
}

/// Every copy of library `lib` left by an instance that is no longer running, in no particular
/// order. Nothing is deleted except copies older than [`MAX_AGE`].
pub fn take_all(paths: &LocalPaths, instance: &Instance, lib: LibraryId) -> Vec<RecoveryEntry> {
    let Some(dir) = folder(paths, lib) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(META_EXT) {
            continue;
        }
        let Some(found) = read_one(&dir, lib, &path) else {
            continue;
        };
        if found.instance == instance.id {
            continue;
        }
        let mut found = found;
        match liveness(paths, found.instance) {
            Liveness::Alive => continue,
            Liveness::Dead => {}
            Liveness::Unknown => found.liveness_unknown = true,
        }
        let expired = now
            .duration_since(found.saved_at)
            .is_ok_and(|age| age > MAX_AGE);
        if expired {
            let _ = remove_stem(&dir, &found.stem);
            continue;
        }
        out.push(found);
    }
    out
}

/// Removes this instance's copy for `key` (after a save, or "Don't save"), if there is one.
/// Other tabs' copies and other instances' copies stay.
pub fn discard(
    paths: &LocalPaths,
    instance: &Instance,
    key: &RecoveryKey,
) -> Result<(), RecoveryError> {
    let dir = folder(paths, key.library).ok_or(RecoveryError::NoFolder)?;
    remove_stem(&dir, &stem_of(instance.id, key))
}

/// Removes an offered copy (once restored into a tab that stores its own, or on "Discard").
pub fn discard_entry(paths: &LocalPaths, entry: &RecoveryEntry) -> Result<(), RecoveryError> {
    let dir = folder(paths, entry.key.library).ok_or(RecoveryError::NoFolder)?;
    remove_stem(&dir, &entry.stem)
}

/// Removes every copy of library `lib` (it was detached, R-T5-4).
pub fn discard_library(paths: &LocalPaths, lib: LibraryId) -> Result<(), RecoveryError> {
    let dir = folder(paths, lib).ok_or(RecoveryError::NoFolder)?;
    match fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ## m4b-t10-service (named exception; T5 mf4)
/// How recently a library's copies must all be older than for [`sweep_unregistered`] to
/// remove them: another instance that registered the library after this one read the registry
/// writes its copies every 5 s of idle, so a folder touched this recently may be its.
pub const SWEEP_MIN_AGE: Duration = Duration::from_secs(10 * 60);

/// Removes the copy folders of libraries for which `registered` answers `false` (T5 mf4: a
/// library detached or lost without its copies being discarded), except folders holding a copy
/// whose instance is alive or of unknown liveness (fix round 1, I2: tested through that
/// instance's advisory lock, as [`take_all`] does; R-T5-5), and folders holding a file modified
/// in the last [`SWEEP_MIN_AGE`]. Only folders named by a library ID are touched (never
/// `instances` or anything else), links are never followed, and an entry that cannot be read is
/// left alone. Returns how many folders were removed. The caller runs it at launch, and only
/// over a registry it could read (a read-only or set-aside registry lists nothing to keep).
pub fn sweep_unregistered(paths: &LocalPaths, registered: &dyn Fn(LibraryId) -> bool) -> usize {
    let Some(root) = instances_dir(paths).and_then(|d| d.parent().map(Path::to_path_buf)) else {
        return 0;
    };
    let Ok(entries) = fs::read_dir(&root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|n| {
                Uuid::parse_str(n)
                    .ok()
                    .filter(|u| u.hyphenated().to_string() == n)
            })
            .map(LibraryId)
        else {
            continue;
        };
        let is_dir = fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_dir());
        if !is_dir || registered(id) {
            continue;
        }
        let Some(dir) = folder(paths, id) else {
            continue;
        };
        // Fix round 1 (I2): a copy whose instance is alive, or whose liveness cannot be told
        // (R-T5-5), keeps the folder; so does a sidecar that cannot be read. The age is a second
        // line only.
        if has_live_copy(paths, &dir) != Some(false) {
            continue;
        }
        if touched_since(&dir, now, SWEEP_MIN_AGE) != Some(false) {
            continue;
        }
        if discard_library(paths, id).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Whether a copy in `dir` belongs to an instance that is alive or whose liveness cannot be
/// tested (fix round 1, I2); `None` when the folder or a sidecar cannot be read (kept too).
fn has_live_copy(paths: &LocalPaths, dir: &Path) -> Option<bool> {
    for entry in fs::read_dir(dir).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|e| e.to_str()) != Some(META_EXT) {
            continue;
        }
        let lstat = fs::symlink_metadata(&path).ok()?;
        if !lstat.is_file() || lstat.len() > META_MAX_BYTES {
            return None;
        }
        let sidecar: Sidecar = toml::from_str(&fs::read_to_string(&path).ok()?).ok()?;
        let instance = Uuid::parse_str(&sidecar.instance).ok()?;
        if liveness(paths, instance) != Liveness::Dead {
            return Some(true);
        }
    }
    Some(false)
}

/// Whether a file directly in `dir` was modified within `age` of `now`; `None` when the folder
/// cannot be read.
fn touched_since(dir: &Path, now: SystemTime, age: Duration) -> Option<bool> {
    for entry in fs::read_dir(dir).ok()? {
        let meta = fs::symlink_metadata(entry.ok()?.path()).ok()?;
        let modified = meta.modified().ok()?;
        if now.duration_since(modified).map_or(true, |d| d < age) {
            return Some(true);
        }
    }
    Some(false)
}
// ## m4b-t10-service

fn folder(paths: &LocalPaths, lib: LibraryId) -> Option<PathBuf> {
    paths.script_recovery_dir(&lib.to_string())
}

/// `<data>/script-recovery/instances`, beside the libraries' folders (a UUID is never
/// `instances`).
fn instances_dir(paths: &LocalPaths) -> Option<PathBuf> {
    paths.script_recovery_dir(INSTANCES)
}

/// Whether an instance is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Liveness {
    Alive,
    Dead,
    /// Its lock cannot be tested here (fix round 2, mf3): offered, marked.
    Unknown,
}

/// Whether the instance `id` still holds its lock. A lock file nobody holds, or a missing one,
/// means it is gone.
fn liveness(paths: &LocalPaths, id: Uuid) -> Liveness {
    let Some(dir) = instances_dir(paths) else {
        return Liveness::Dead;
    };
    let path = dir.join(format!("{}.{LOCK_EXT}", id.simple()));
    let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path) else {
        return Liveness::Dead;
    };
    match try_lock(&file) {
        Ok(()) => Liveness::Dead,
        Err(TryLockError::WouldBlock) => Liveness::Alive,
        // A lock that cannot be tested: neither deleted nor hidden, offered with the mark
        // (R-T5-5; before fix round 2 such copies were never offered).
        Err(TryLockError::Error(_)) => Liveness::Unknown,
    }
}

/// `File::try_lock`, or, under [`with_locks_unsupported`] (tests), the error a file system
/// without advisory locks answers.
fn try_lock(file: &File) -> Result<(), TryLockError> {
    #[cfg(any(test, feature = "test-support"))]
    if NO_LOCKS.with(std::cell::Cell::get) {
        return Err(TryLockError::Error(io::ErrorKind::Unsupported.into()));
    }
    file.try_lock()
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static NO_LOCKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `f` as if advisory locks were unsupported, on this thread only (fix round 2, mf3).
/// For tests.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn with_locks_unsupported<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            NO_LOCKS.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(NO_LOCKS.with(|c| c.replace(true)));
    f()
}

/// Removes the lock files of instances that are gone. A missing lock file means "not running"
/// just as an unlocked one does, so this never makes a live instance's copies look dead; only
/// files older than a minute are touched, so an instance between creating and locking its
/// file is left alone.
fn sweep_dead_locks(dir: &Path, own: Uuid) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let own = format!("{}.{LOCK_EXT}", own.simple());
    for entry in entries.flatten() {
        let path = entry.path();
        let is_lock = path.extension().and_then(|e| e.to_str()) == Some(LOCK_EXT);
        if !is_lock || entry.file_name().to_str() == Some(own.as_str()) {
            continue;
        }
        let old = fs::symlink_metadata(&path).is_ok_and(|m| {
            m.is_file()
                && m.modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age >= Duration::from_secs(60))
        });
        if !old {
            continue;
        }
        if let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path)
            && try_lock(&file).is_ok()
        {
            let _ = fs::remove_file(&path);
        }
    }
}

fn store_in(
    dir: Option<PathBuf>,
    instance: &Instance,
    key: &RecoveryKey,
    text: &str,
    copy: &CopyMeta,
    now: SystemTime,
) -> Result<(), RecoveryError> {
    if copy.meta.partial {
        return Err(RecoveryError::PreviewOnly);
    }
    let dir = dir.ok_or(RecoveryError::NoFolder)?;
    make_private(&dir)?;
    let stem = stem_of(instance.id, key);
    let meta = Sidecar::new(instance.id, key, copy, now, !instance.is_locked());
    let meta = toml::to_string(&meta).map_err(|_| RecoveryError::Io(io::ErrorKind::InvalidData))?;
    fsio::atomic_write(&dir.join(format!("{stem}.{TEXT_EXT}")), text.as_bytes())?;
    fsio::atomic_write(&dir.join(format!("{stem}.{META_EXT}")), meta.as_bytes())?;
    Ok(())
}

/// Removes a copy's sidecar, then its text (a copy without a sidecar is never offered).
fn remove_stem(dir: &Path, stem: &str) -> Result<(), RecoveryError> {
    for ext in [META_EXT, TEXT_EXT] {
        match fs::remove_file(dir.join(format!("{stem}.{ext}"))) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// `<data>/script-recovery` and the folder in it, 0700 whether new or not.
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

/// The file stem of a copy: the XXH3-128 of the instance, the binding and the path, each part
/// length-prefixed, in 32 hex digits.
fn stem_of(instance: Uuid, key: &RecoveryKey) -> String {
    let mut h = XxHash3_128::new();
    let mut part = |bytes: &[u8]| {
        h.write(&(bytes.len() as u64).to_le_bytes());
        h.write(bytes);
    };
    part(b"hv-recovery");
    part(instance.as_bytes());
    part(key.library.0.as_bytes());
    let profile = key.profile.as_ref().map(|p| p.0);
    part(profile.as_ref().map_or(&[][..], |u| &u.as_bytes()[..]));
    part(
        key.database
            .as_ref()
            .map_or("", DatabaseName::as_str)
            .as_bytes(),
    );
    part(&[
        u8::from(key.profile.is_some()),
        u8::from(key.database.is_some()),
    ]);
    part(key.rel.as_str().as_bytes());
    format!("{:032x}", h.finish_128())
}

fn read_one(dir: &Path, lib: LibraryId, meta_path: &Path) -> Option<RecoveryEntry> {
    let stem = meta_path.file_stem()?.to_str()?;
    let lstat = fs::symlink_metadata(meta_path).ok()?;
    if !lstat.is_file() || lstat.len() > META_MAX_BYTES {
        return None;
    }
    let sidecar: Sidecar = toml::from_str(&fs::read_to_string(meta_path).ok()?).ok()?;
    if sidecar.format != FORMAT {
        return None;
    }
    let instance = Uuid::parse_str(&sidecar.instance).ok()?;
    let key = RecoveryKey {
        library: lib,
        rel: RelPath::parse(&sidecar.rel_path).ok()?,
        profile: match &sidecar.profile {
            None => None,
            Some(p) => Some(ProfileId(Uuid::parse_str(p).ok()?)),
        },
        database: sidecar.database.as_deref().map(DatabaseName::new),
    };
    if stem_of(instance, &key) != stem {
        return None;
    }
    let text_path = dir.join(format!("{stem}.{TEXT_EXT}"));
    if !fs::symlink_metadata(&text_path).ok()?.is_file() {
        return None;
    }
    let text = String::from_utf8(fs::read(&text_path).ok()?).ok()?;
    let copy = CopyMeta {
        meta: FileMeta {
            encoding: encoding::lookup(&sidecar.encoding)?,
            bom: sidecar.bom,
            line_ending: match sidecar.line_ending.as_str() {
                "lf" => LineEnding::Lf,
                "crlf" => LineEnding::CrLf,
                "cr" => LineEnding::Cr,
                _ => return None,
            },
            partial: false,
        },
        base: match sidecar.base {
            None => None,
            Some(b) => Some(b.stamp()?),
        },
        secret_suspected: sidecar.secret_suspected,
        damaged: sidecar.damaged,
        // ## m4b-t14-tabs (fix round 3): an unreadable identity keeps the path alone.
        db_file: sidecar.db_file_path.map(|path| DbFile {
            path,
            dev_ino: sidecar.db_file_id.as_deref().and_then(|t| {
                let (d, i) = t.split_once(':')?;
                Some((d.parse().ok()?, i.parse().ok()?))
            }),
        }),
    };
    Some(RecoveryEntry {
        key,
        text,
        copy,
        saved_at: time_of(sidecar.saved_at_secs, sidecar.saved_at_nanos)?,
        // An unlocked writer's lock file is unlocked while it runs: never read that as dead.
        liveness_unknown: sidecar.unlocked,
        instance,
        stem: stem.to_owned(),
    })
}

/// The sidecar, as written. `u128` and pre-1970 times do not fit TOML integers, so the hash is
/// hex text and times are seconds and nanoseconds since 1970.
#[derive(Serialize, Deserialize)]
struct Sidecar {
    format: u32,
    instance: String,
    rel_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    database: Option<String>,
    encoding: String,
    bom: bool,
    line_ending: String,
    secret_suspected: bool,
    damaged: bool,
    /// fr2 mf3: the writing instance held no advisory lock.
    #[serde(default)]
    unlocked: bool,
    saved_at_secs: u64,
    saved_at_nanos: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<BaseStamp>,
    // ## m4b-t14-tabs (named exception, fix round 3 m-fr2-1): lenient, absent before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    db_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    db_file_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct BaseStamp {
    len: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    hash: String,
}

impl Sidecar {
    fn new(
        instance: Uuid,
        key: &RecoveryKey,
        copy: &CopyMeta,
        now: SystemTime,
        unlocked: bool,
    ) -> Self {
        let since = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok();
        let saved = since(now).unwrap_or_default();
        Sidecar {
            format: FORMAT,
            instance: instance.hyphenated().to_string(),
            rel_path: key.rel.as_str().to_owned(),
            profile: key.profile.as_ref().map(ToString::to_string),
            database: key.database.as_ref().map(|d| d.as_str().to_owned()),
            encoding: copy.meta.encoding.name().to_owned(),
            bom: copy.meta.bom,
            line_ending: match copy.meta.line_ending {
                LineEnding::Lf => "lf",
                LineEnding::CrLf => "crlf",
                LineEnding::Cr => "cr",
            }
            .to_owned(),
            secret_suspected: copy.secret_suspected,
            damaged: copy.damaged,
            unlocked,
            saved_at_secs: saved.as_secs(),
            saved_at_nanos: saved.subsec_nanos(),
            // A file stamped before 1970 keeps no base: the app then treats it as changed.
            base: copy.base.and_then(|b| {
                let m = since(b.mtime)?;
                Some(BaseStamp {
                    len: b.len,
                    mtime_secs: m.as_secs(),
                    mtime_nanos: m.subsec_nanos(),
                    hash: format!("{:032x}", b.hash),
                })
            }),
            db_file_path: copy.db_file.as_ref().map(|f| f.path.clone()),
            db_file_id: copy
                .db_file
                .as_ref()
                .and_then(|f| f.dev_ino)
                .map(|(d, i)| format!("{d}:{i}")),
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

/// For tests: stores a copy as if written at `saved_at` (the 14-day expiry).
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn store_at(
    paths: &LocalPaths,
    instance: &Instance,
    key: &RecoveryKey,
    text: &str,
    copy: &CopyMeta,
    saved_at: SystemTime,
) -> Result<(), RecoveryError> {
    store_in(
        folder(paths, key.library),
        instance,
        key,
        text,
        copy,
        saved_at,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> RecoveryKey {
        RecoveryKey {
            library: LibraryId::new(),
            rel: RelPath::parse("a.sql").unwrap(),
            profile: None,
            database: None,
        }
    }

    /// carries-w1 T5 (m5): no folder → no copy, and no fallback folder anywhere.
    #[test]
    fn no_folder_writes_nothing() {
        assert_eq!(
            store_in(
                None,
                &unlocked(),
                &key(),
                "x",
                &CopyMeta::default(),
                SystemTime::now()
            ),
            Err(RecoveryError::NoFolder)
        );
    }

    fn unlocked() -> Instance {
        Instance {
            id: Uuid::new_v4(),
            _lock: None,
        }
    }

    /// fr2 I-fr1-2: a preview's meta is never stored (and nothing is written).
    #[test]
    fn partial_meta_is_not_stored() {
        let copy = CopyMeta {
            meta: FileMeta {
                partial: true,
                ..FileMeta::default()
            },
            ..CopyMeta::default()
        };
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            store_in(
                Some(dir.path().join("lib")),
                &unlocked(),
                &key(),
                "x",
                &copy,
                SystemTime::now()
            ),
            Err(RecoveryError::PreviewOnly)
        );
        assert!(fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn a_pre_1970_base_is_dropped_not_wrapped() {
        let copy = CopyMeta {
            base: Some(DiskStamp {
                len: 1,
                mtime: UNIX_EPOCH - Duration::from_secs(5),
                hash: 7,
            }),
            ..CopyMeta::default()
        };
        assert!(
            Sidecar::new(Uuid::new_v4(), &key(), &copy, SystemTime::now(), false)
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

    /// The binding is part of the key: no two bindings or instances share a stem, and
    /// `None` is not the empty database.
    #[test]
    fn stems_differ_per_binding_and_instance() {
        let a = key();
        let i = Uuid::new_v4();
        let mut b = a.clone();
        b.profile = Some(ProfileId::new());
        let mut c = b.clone();
        c.database = Some(DatabaseName::new("orders"));
        let mut d = a.clone();
        d.database = Some(DatabaseName::new(""));
        let stems = [
            stem_of(i, &a),
            stem_of(i, &b),
            stem_of(i, &c),
            stem_of(i, &d),
            stem_of(Uuid::new_v4(), &a),
        ];
        for (n, s) in stems.iter().enumerate() {
            assert_eq!(s.len(), 32);
            assert!(!stems[n + 1..].contains(s), "{n}");
        }
    }
}
// ## m4b-t05-fileio
