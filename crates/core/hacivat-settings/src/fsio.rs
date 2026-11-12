//! Small file helpers: atomic writes and set-aside renames.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a temporary sibling file and renames it over `path` (`0600` on Unix).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    create_dir_all_private(parent)?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(path.file_name().unwrap_or_default());
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = parent.join(tmp_name);

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Renames `path` to `<name>.<tag>-<unix_secs>` (with a numeric suffix on a collision) and
/// returns the new path.
pub fn quarantine(path: &Path, tag: &str, now: SystemTime) -> io::Result<PathBuf> {
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut target = path.with_file_name(format!("{name}.{tag}-{secs}"));
    let mut n = 1u32;
    while target.exists() {
        target = path.with_file_name(format!("{name}.{tag}-{secs}-{n}"));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

/// `create_dir_all`, with mode `0700` on every directory it creates (Unix). Existing
/// directories are left as they are.
pub fn create_dir_all_private(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Flushes the directory entry of a just-renamed file (Unix; a no-op elsewhere).
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn lazily_created_parents_are_0700() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("a/b/c/file.bin");
        atomic_write(&file, b"x").unwrap();
        for d in ["a", "a/b", "a/b/c"] {
            let m = std::fs::metadata(root.path().join(d))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(m, 0o700, "{d}");
        }
    }
}
