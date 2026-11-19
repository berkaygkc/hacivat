//! File handling of the catalog cache: owner-only modes, corrupt-file rotation (copied from the
//! spike, which copied `hacivat-exec/src/history/schema.rs`), scrubbed deletion and the global
//! size cap. Symbolic links are never followed: a cache path, `-wal` or `-shm` that is a link
//! is refused, a rotation renames the link itself, and a deletion removes the link only.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hacivat_proto::ProfileId;

use super::CacheCaps;

/// `path` with `suffix` appended to its file name.
pub(super) fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The database file and its SQLite sidecars.
fn family(path: &Path) -> [PathBuf; 3] {
    [
        path.to_path_buf(),
        sibling(path, "-wal"),
        sibling(path, "-shm"),
    ]
}

fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// Whether anything (a file or a dangling link) sits at `p`.
fn occupied(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// Creates the cache directory with mode 0700 on every directory it creates (Unix; the same
/// rule as `hacivat_settings::fsio::create_dir_all_private`, which this crate cannot depend
/// on). An existing directory is left as it is.
pub(super) fn ensure_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// Creates the file owner-only, or tightens an existing one and its `-wal`/`-shm`. A path
/// that is a symbolic link is refused.
pub(super) fn prepare_file(path: &Path) -> io::Result<()> {
    for p in family(path) {
        if is_symlink(&p) {
            return Err(io::Error::other(
                "the catalog cache path is a symbolic link",
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    tighten(path)
}

/// Sets mode 0600 on the file and its sidecars where they are looser (Unix). Links are skipped.
pub(super) fn tighten(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in family(path) {
            if let Ok(m) = fs::symlink_metadata(&p)
                && m.file_type().is_file()
                && m.permissions().mode() & 0o077 != 0
            {
                fs::set_permissions(&p, fs::Permissions::from_mode(0o600))?;
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

/// Renames a corrupt file (and its `-wal`, `-shm`) to `<name>.corrupt-<unix seconds>[-n]`, the
/// log first so an old `-wal` never sits beside a fresh file. `rename` moves a link, never its
/// target.
pub(super) fn rotate(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut target = sibling(path, &format!(".corrupt-{secs}"));
    let mut n = 1u32;
    while family(&target).iter().any(|p| occupied(p)) {
        target = sibling(path, &format!(".corrupt-{secs}-{n}"));
        n += 1;
    }
    for suffix in ["-wal", "-shm"] {
        let from = sibling(path, suffix);
        if occupied(&from) {
            fs::rename(&from, sibling(&target, suffix))?;
        }
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Overwrites a regular file with zeros, then removes it; a link is removed without touching
/// its target. Best effort against recovery (an SSD may keep old blocks): it keeps a plaintext
/// cache's names out of a later encrypted profile's directory listing and free space.
fn scrub_one(p: &Path) -> io::Result<()> {
    let Ok(meta) = fs::symlink_metadata(p) else {
        return Ok(());
    };
    if meta.file_type().is_file() {
        let mut f = fs::OpenOptions::new().write(true).open(p)?;
        let zeros = vec![0u8; 64 << 10];
        let mut left = meta.len();
        while left > 0 {
            let n = usize::try_from(left.min(zeros.len() as u64)).unwrap_or(zeros.len());
            f.write_all(&zeros[..n])?;
            left -= n as u64;
        }
        f.sync_all()?;
    }
    match fs::remove_file(p) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Scrubs and removes the database file and its sidecars.
pub(super) fn scrub_family(path: &Path) -> io::Result<()> {
    // The log first: a fresh file must never meet an old `-wal`.
    for p in [
        sibling(path, "-wal"),
        sibling(path, "-shm"),
        path.to_path_buf(),
    ] {
        scrub_one(&p)?;
    }
    Ok(())
}

/// Every cache file of `profile` in `dir` (every major, sidecars and corrupt rotations).
fn profile_files(dir: &Path, profile_prefix: &str) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(profile_prefix))
        {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Scrubs and deletes every cache file of `profile` under `dir`.
pub(super) fn delete_profile(dir: &Path, profile: &ProfileId) -> io::Result<()> {
    for p in profile_files(dir, &format!("{}.v", profile.0))? {
        scrub_one(&p)?;
    }
    Ok(())
}

/// The global cap (R-M4-25): while the cache files under `dir` exceed `caps.global` bytes,
/// deletes the whole cache of the least recently used profile (by the newest modification
/// time of its files; a load writes its usage row, so it touches the `-wal`), never a profile
/// in `keep` (the open ones; their own cap is [`super::CacheStore::evict`]'s). Returns the
/// deleted files.
pub fn evict_global(dir: &Path, caps: CacheCaps, keep: &[ProfileId]) -> io::Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // (profile prefix) -> (files, bytes, newest mtime)
    let mut groups: std::collections::BTreeMap<String, (Vec<PathBuf>, u64, SystemTime)> =
        std::collections::BTreeMap::new();
    let mut total = 0u64;
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(at) = name.find(".v") else { continue };
        if !name.contains(".sqlite") {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path())?;
        if !meta.file_type().is_file() {
            continue;
        }
        let g = groups
            .entry(name[..at].to_string())
            .or_insert_with(|| (Vec::new(), 0, UNIX_EPOCH));
        g.0.push(entry.path());
        g.1 += meta.len();
        g.2 = g.2.max(meta.modified().unwrap_or(UNIX_EPOCH));
        total += meta.len();
    }
    let kept: Vec<String> = keep.iter().map(|p| p.0.to_string()).collect();
    let mut order: Vec<(SystemTime, String)> = groups
        .iter()
        .filter(|(k, _)| !kept.contains(k))
        .map(|(k, g)| (g.2, k.clone()))
        .collect();
    order.sort();
    let mut deleted = Vec::new();
    for (_, k) in order {
        if total <= caps.global {
            break;
        }
        if let Some((files, bytes, _)) = groups.remove(&k) {
            for f in files {
                scrub_one(&f)?;
                deleted.push(f);
            }
            total = total.saturating_sub(bytes);
        }
    }
    Ok(deleted)
}
