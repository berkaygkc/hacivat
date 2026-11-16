//! Local file paths a connection names (TLS files, approved roots): the one rule for a path
//! that is never a local file (M2a final-fix round 2, N2), and a probe tests use to prove
//! such a path is never touched.

use std::path::{Component, Path, PathBuf, Prefix};

/// Whether `path` names another machine or the NT object namespace, on any OS: it starts with
/// two separators (`\\host\share`, `//host/share`, which covers `\\?\UNC\` and `\\.\`), with
/// `\??\`, or it has a UNC, device or `GLOBALROOT` prefix. Opening one on Windows can send the
/// user's credentials (an NTLM hash) to a host, so it is refused before any file system call:
/// no open, no `metadata`, no `canonicalize`. On Windows a parsed prefix decides, so a verbatim
/// disk path (`\\?\C:\…`) is local.
pub fn is_network_path(path: &Path) -> bool {
    // Where the system parses a prefix (Windows), its kind decides: a verbatim disk path
    // (`\\?\C:\…`) is local even though it starts with two separators.
    if let Some(Component::Prefix(p)) = path.components().next() {
        return match p.kind() {
            Prefix::DeviceNS(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
            Prefix::Verbatim(name) => name.eq_ignore_ascii_case("GLOBALROOT"),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
        };
    }
    // No prefix (every path on Unix): a share spelled `\\server` or `//server`, or the NT
    // object namespace `\??\`, is refused by its text.
    let text = path.to_string_lossy();
    let sep = |c: Option<char>| matches!(c, Some('/' | '\\'));
    let mut chars = text.chars();
    let (a, b, c, d) = (chars.next(), chars.next(), chars.next(), chars.next());
    (sep(a) && sep(b)) || (sep(a) && b == Some('?') && c == Some('?') && sep(d))
}

// ## m2b-t12-profiles
/// The local file a profile's path names: the one path every reader opens (TLS files; the SSH
/// key files from Task 14 on) and Validate's approved-roots check canonicalises, so the
/// checked file is the opened file (T12 review M1). A leading `~` component is the user's home
/// directory. `None` for a share or a device, as typed or once `~` is expanded (a home folder
/// on a share; review M2), and for `~` without a home directory. No file system call.
pub fn local_path(path: &Path) -> Option<PathBuf> {
    if is_network_path(path) {
        return None;
    }
    let expanded = match path.strip_prefix("~") {
        Ok(rest) => std::env::home_dir()?.join(rest),
        Err(_) => path.to_path_buf(),
    };
    (!is_network_path(&expanded)).then_some(expanded)
}

/// A record of the paths the connect stack's file readers touched, for tests that prove a
/// refused path is never opened (N2). Off by default: [`note`](fs_probe::note) records nothing
/// until a test calls [`enable`](fs_probe::enable), so the app never accumulates paths.
#[doc(hidden)]
pub mod fs_probe {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, PoisonError};

    static ENABLED: AtomicBool = AtomicBool::new(false);
    static TOUCHED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    /// Starts recording (process-wide; tests filter by their own unique paths).
    pub fn enable() {
        ENABLED.store(true, Ordering::SeqCst);
    }

    /// Records that a reader is about to touch `path` (open, `metadata`, `canonicalize`).
    pub fn note(path: &Path) {
        if ENABLED.load(Ordering::SeqCst) {
            TOUCHED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(path.to_path_buf());
        }
    }

    /// Every path recorded so far.
    pub fn touched() -> Vec<PathBuf> {
        TOUCHED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_and_devices_are_network_paths_local_files_are_not() {
        for p in [
            r"\\attacker\s\ca.pem",
            "//host/s/ca.pem",
            r"\\?\UNC\h\s\x",
            r"\??\x",
            r"\\.\pipe\x",
            r"\/srv/share",
            "/\\host",
        ] {
            assert!(is_network_path(Path::new(p)), "{p}");
        }
        for p in [
            "/etc/ssl/ca.pem",
            "ca.pem",
            "./ca.pem",
            "C:/certs/ca.pem",
            "",
        ] {
            assert!(!is_network_path(Path::new(p)), "{p}");
        }
    }

    /// A verbatim disk path is local on Windows (it starts with two separators, which the
    /// text rule alone took for a share: a Windows CI failure in the SQLite driver).
    #[cfg(windows)]
    #[test]
    fn a_verbatim_disk_path_is_local_on_windows() {
        assert!(!is_network_path(Path::new(r"\\?\C:\data\a.db")));
        assert!(is_network_path(Path::new(r"\\?\UNC\h\s\a.db")));
        assert!(is_network_path(Path::new(r"\\?\GLOBALROOT\Device\x")));
    }

    // ## m2b-t12-profiles
    #[test]
    fn local_paths_expand_the_home_folder_and_refuse_shares() {
        let home = std::env::home_dir();
        assert_eq!(
            local_path(Path::new("~/.ssh/id_ed25519")),
            home.as_ref().map(|h| h.join(".ssh/id_ed25519"))
        );
        assert_eq!(local_path(Path::new("~")), home.map(|h| h.join("")));
        for p in ["/etc/ssl/ca.pem", "ca.pem", "~user/x", "a/~/b"] {
            assert_eq!(local_path(Path::new(p)), Some(PathBuf::from(p)), "{p}");
        }
        for p in [r"\\host\s\x", "//host/s/x", r"\??\x", r"\\.\pipe\x"] {
            assert_eq!(local_path(Path::new(p)), None, "{p}");
        }
    }
}
