//! Repository discovery (HV-SCRIPT-11 step 1, R-M4b-10).
//!
//! A plain upward walk from the canonical library root to the first entry named `.git` (a
//! directory, a gitfile, or a symlink, which `trust` refuses). The walk never considers the
//! user's home directory or anything above it (a `$HOME/.git` dotfiles repository would
//! otherwise claim every library under home), and stops where the filesystem changes
//! (`st_dev` on Unix; the path prefix, a drive or share, on Windows). It reads metadata only;
//! `gix`'s own `discover` is not used (it honours environment overrides).

// ## m4b-t07-git
use std::path::{Component, Path, PathBuf};

/// What the walk found for one library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// The directory that holds the `.git` entry: the worktree root.
    pub(crate) worktree: PathBuf,
    /// The `.git` entry itself.
    pub(crate) dot_git: PathBuf,
    pub(crate) kind: DotGit,
    /// The library root relative to `worktree`, `/`-separated (empty: the worktree itself).
    pub(crate) prefix: String,
}

/// What kind of entry `.git` is (from `lstat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DotGit {
    Dir,
    /// A gitfile (`gitdir: <path>`): a linked worktree, a submodule checkout, or a redirect.
    File,
    Symlink,
}

/// The outcome of a walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Discovery {
    Found(Found),
    NotARepo,
    /// The library root, relative to the worktree, is not valid UTF-8.
    NonUtf8Prefix,
}

/// Walks up from `root` (canonical) with the home directory as the ceiling.
pub(crate) fn find(root: &Path) -> Discovery {
    let home = std::env::home_dir().map(|h| std::fs::canonicalize(&h).unwrap_or(h));
    find_with(root, home.as_deref(), &device_of)
}

/// [`find`] with the ceiling and the device lookup given (tests inject both).
pub(crate) fn find_with(
    root: &Path,
    home: Option<&Path>,
    device: &dyn Fn(&Path) -> Option<u64>,
) -> Discovery {
    let Some(root_dev) = device(root) else {
        return Discovery::NotARepo;
    };
    let mut dir = root.to_path_buf();
    loop {
        if home.is_some_and(|h| h == dir) {
            return Discovery::NotARepo;
        }
        let candidate = dir.join(".git");
        if let Ok(md) = std::fs::symlink_metadata(&candidate) {
            let kind = if md.file_type().is_symlink() {
                Some(DotGit::Symlink)
            } else if md.is_dir() {
                Some(DotGit::Dir)
            } else if md.is_file() {
                Some(DotGit::File)
            } else {
                // A FIFO, socket or device named `.git` is not a repository marker.
                None
            };
            if let Some(kind) = kind {
                return match relative(root, &dir) {
                    Some(prefix) => Discovery::Found(Found {
                        worktree: dir,
                        dot_git: candidate,
                        kind,
                        prefix,
                    }),
                    None => Discovery::NonUtf8Prefix,
                };
            }
        }
        let Some(parent) = dir.parent().map(Path::to_path_buf) else {
            return Discovery::NotARepo;
        };
        if home.is_some_and(|h| h == parent) || device(&parent) != Some(root_dev) {
            return Discovery::NotARepo;
        }
        dir = parent;
    }
}

/// `root` relative to `worktree`, `/`-separated, or `None` when a component is not UTF-8.
fn relative(root: &Path, worktree: &Path) -> Option<String> {
    let rel = root.strip_prefix(worktree).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?.to_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// The filesystem a path lives on.
#[cfg(unix)]
fn device_of(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

/// The filesystem a path lives on: Windows offers no stable device id in `std`, so the path's
/// prefix (`C:`, `\\?\C:`, a UNC share) stands in for it.
#[cfg(windows)]
fn device_of(p: &Path) -> Option<u64> {
    use std::hash::{Hash as _, Hasher as _};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match p.components().next() {
        Some(Component::Prefix(pre)) => pre.as_os_str().hash(&mut h),
        _ => 0u8.hash(&mut h),
    }
    std::fs::metadata(p).ok().map(|_| h.finish())
}

#[cfg(not(any(unix, windows)))]
fn device_of(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|_| 0)
}
// ## m4b-t07-git

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(t.path()).unwrap();
        std::fs::create_dir_all(top.join("repo/.git")).unwrap();
        std::fs::create_dir_all(top.join("repo/a/b/lib")).unwrap();
        (t, top)
    }

    fn same_device(_: &Path) -> Option<u64> {
        Some(7)
    }

    #[test]
    fn walks_up_to_the_first_dot_git_and_keeps_the_prefix() {
        let (_t, top) = tree();
        let lib = top.join("repo/a/b/lib");
        let Discovery::Found(f) = find_with(&lib, None, &same_device) else {
            panic!("not found");
        };
        assert_eq!(f.worktree, top.join("repo"));
        assert_eq!(f.kind, DotGit::Dir);
        assert_eq!(f.prefix, "a/b/lib");
        let Discovery::Found(f) = find_with(&top.join("repo"), None, &same_device) else {
            panic!("not found");
        };
        assert_eq!(f.prefix, "");
    }

    #[test]
    fn home_is_a_ceiling_both_as_the_worktree_and_above_it() {
        let (_t, top) = tree();
        let lib = top.join("repo/a/b/lib");
        // Home is the worktree: its `.git` is never considered.
        assert_eq!(
            find_with(&lib, Some(&top.join("repo")), &same_device),
            Discovery::NotARepo
        );
        // Home is below the repository: the walk stops at home.
        assert_eq!(
            find_with(&lib, Some(&top.join("repo/a")), &same_device),
            Discovery::NotARepo
        );
        // Home elsewhere: found.
        assert!(matches!(
            find_with(&lib, Some(&top.join("elsewhere")), &same_device),
            Discovery::Found(_)
        ));
    }

    #[test]
    fn a_filesystem_boundary_stops_the_walk() {
        let (_t, top) = tree();
        let lib = top.join("repo/a/b/lib");
        let mount = top.join("repo/a");
        // `repo/a` and below are another filesystem than `repo`.
        let device = |p: &Path| Some(if p.starts_with(&mount) { 2 } else { 1 });
        assert_eq!(find_with(&lib, None, &device), Discovery::NotARepo);
        // The same tree on one filesystem: found.
        assert!(matches!(
            find_with(&lib, None, &same_device),
            Discovery::Found(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_named_dot_git_is_no_marker() {
        let t = tempfile::tempdir().unwrap();
        let top = std::fs::canonicalize(t.path()).unwrap();
        std::fs::create_dir_all(top.join("lib")).unwrap();
        std::fs::create_dir_all(top.join(".git")).unwrap();
        // No FIFO helper without a subprocess here: a socket is the same kind of non-regular
        // entry and needs only `std`. The walk passes it and finds the real one above.
        let _sock = std::os::unix::net::UnixListener::bind(top.join("lib/.git")).unwrap();
        let Discovery::Found(f) = find_with(&top.join("lib"), None, &same_device) else {
            panic!("not found");
        };
        assert_eq!(f.worktree, top);
        assert_eq!(f.prefix, "lib");
    }
}
