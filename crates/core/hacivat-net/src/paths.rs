//! Local file paths a connection names (TLS files, approved roots): the one rule for a path
//! that is never a local file (M2a final-fix round 2, N2), and a probe tests use to prove
//! such a path is never touched.

use std::path::{Component, Path, Prefix};

/// Whether `path` names another machine or the NT object namespace, on any OS: it starts with
/// two separators (`\\host\share`, `//host/share`, which covers `\\?\UNC\` and `\\.\`), with
/// `\??\`, or it has a UNC, device or `GLOBALROOT` prefix. Opening one on Windows can send the
/// user's credentials (an NTLM hash) to a host, so it is refused before any file system call:
/// no open, no `metadata`, no `canonicalize`.
pub fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let sep = |c: Option<char>| matches!(c, Some('/' | '\\'));
    let mut chars = text.chars();
    let (a, b, c, d) = (chars.next(), chars.next(), chars.next(), chars.next());
    if (sep(a) && sep(b)) || (sep(a) && b == Some('?') && c == Some('?') && sep(d)) {
        return true;
    }
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::DeviceNS(_) | Prefix::UNC(..) | Prefix::VerbatimUNC(..) => true,
            Prefix::Verbatim(name) => name.eq_ignore_ascii_case("GLOBALROOT"),
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
        },
        _ => false,
    }
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
}
