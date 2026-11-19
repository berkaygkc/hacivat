//! `xtask minicat collect | build | check | verify`: the system-catalog snapshots
//! (HV-INTRO-16, L7, D-017, R-M4-17, R-M4-33). M4 Task 1a skeleton; Task 23 writes the bodies
//! from the minicat spike (`cd27ec1`).
//!
//! - `collect [--reader catalog|test] [--twice] [--require-complete] [--work <dir>] [--out <dir>]`
//! - `build --raw <dir> [--out <dir>] [--require-complete]`
//! - `check [--dir <dir>]` (also `minicat --check`)
//! - `verify [--reader catalog|test] [--twice] [--work <dir>]` (also `minicat --verify`)
//!
//! Until Task 23 every command exits 2 ("not implemented"), except `check`, which passes on an
//! empty manifest (no `[[snapshot]]` entry and no `.hvsnap.zst` file in the directory) and
//! exits 2 on anything else: it never passes a snapshot it cannot verify.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The committed snapshots, relative to the workspace root.
pub const ASSET_DIR: &str = "assets/system-catalogs";
const MANIFEST: &str = "manifest.toml";
const SNAPSHOT_SUFFIX: &str = ".hvsnap.zst";

const USAGE: &str = "usage: xtask minicat collect [--reader catalog|test] [--twice] \
                     [--require-complete] [--work <dir>] [--out <dir>]\n       \
                     xtask minicat build --raw <dir> [--out <dir>] [--require-complete]\n       \
                     xtask minicat check [--dir <dir>]       (also: minicat --check)\n       \
                     xtask minicat verify [--reader catalog|test] [--twice] [--work <dir>]  \
                     (also: minicat --verify)";

/// What `check` found.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The manifest lists no snapshot and the directory holds none.
    Empty,
    /// There are snapshots to verify: Task 23's body.
    NotImplemented,
    /// The manifest is missing or unreadable.
    Invalid(String),
}

/// `check` over `dir`: passes only an empty snapshot set.
pub fn check_dir(dir: &Path) -> CheckOutcome {
    let manifest = match std::fs::read_to_string(dir.join(MANIFEST)) {
        Ok(m) => m,
        Err(e) => return CheckOutcome::Invalid(format!("{}: {e}", dir.join(MANIFEST).display())),
    };
    let table = match manifest.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => return CheckOutcome::Invalid(format!("{MANIFEST}: {e}")),
    };
    // Fix round 1 (m8): an empty manifest holds `format` and `zstd_level` only; any other key
    // (a `[snapshot.x]` table, an empty `snapshot = []`) is not "empty".
    if let Some(key) = table
        .keys()
        .find(|k| !matches!(k.as_str(), "format" | "zstd_level"))
    {
        return if key == "snapshot" {
            CheckOutcome::NotImplemented
        } else {
            CheckOutcome::Invalid(format!("{MANIFEST}: unexpected key `{key}`"))
        };
    }
    if holds_snapshot(dir) {
        CheckOutcome::NotImplemented
    } else {
        CheckOutcome::Empty
    }
}

/// Whether `dir` or any directory under it holds a snapshot file.
fn holds_snapshot(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let p = e.path();
        if p.is_dir() {
            holds_snapshot(&p)
        } else {
            e.file_name().to_string_lossy().ends_with(SNAPSHOT_SUFFIX)
        }
    })
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let command = args.first().map(String::as_str);
    match command {
        Some("check" | "--check") => {
            let dir = args
                .windows(2)
                .find(|w| w[0] == "--dir")
                .map(|w| PathBuf::from(&w[1]))
                .unwrap_or_else(|| crate::workspace_root().join(ASSET_DIR));
            match check_dir(&dir) {
                CheckOutcome::Empty => {
                    println!("minicat check: ok (no snapshots yet)");
                    ExitCode::SUCCESS
                }
                CheckOutcome::NotImplemented => {
                    crate::not_yet("minicat check (snapshots present)", "M4 Task 23")
                }
                CheckOutcome::Invalid(why) => {
                    eprintln!("minicat check: {why}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("collect") => crate::not_yet("minicat collect", "M4 Task 23"),
        Some("build") => crate::not_yet("minicat build", "M4 Task 23"),
        Some("verify" | "--verify") => crate::not_yet("minicat verify", "M4 Task 23"),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_on_the_committed_empty_manifest() {
        assert_eq!(
            check_dir(&crate::workspace_root().join(ASSET_DIR)),
            CheckOutcome::Empty
        );
    }

    #[test]
    fn check_never_passes_snapshots_it_cannot_verify() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST), "format = 1\n").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::Empty);
        std::fs::write(dir.path().join("postgres-17.hvsnap.zst"), b"x").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::NotImplemented);
        std::fs::remove_file(dir.path().join("postgres-17.hvsnap.zst")).unwrap();
        std::fs::write(
            dir.path().join(MANIFEST),
            "format = 1\n\n[[snapshot]]\nfile = \"postgres-17.hvsnap.zst\"\n",
        )
        .unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::NotImplemented);
        assert!(matches!(
            check_dir(&dir.path().join("missing")),
            CheckOutcome::Invalid(_)
        ));
    }

    /// Fix round 1 (m8): a snapshot table, an unknown key or a snapshot in a subdirectory is
    /// not an empty set.
    #[test]
    fn check_sees_snapshot_tables_unknown_keys_and_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join(MANIFEST);
        std::fs::write(&manifest, "format = 1\n\n[snapshot.pg17]\nfile = \"x\"\n").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::NotImplemented);
        std::fs::write(&manifest, "format = 1\nsnapshot = []\n").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::NotImplemented);
        std::fs::write(&manifest, "format = 1\nextra = true\n").unwrap();
        assert!(matches!(check_dir(dir.path()), CheckOutcome::Invalid(_)));
        std::fs::write(&manifest, "format = 1\nzstd_level = 19\n").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::Empty);
        std::fs::create_dir(dir.path().join("pg")).unwrap();
        std::fs::write(dir.path().join("pg/x.hvsnap.zst"), b"x").unwrap();
        assert_eq!(check_dir(dir.path()), CheckOutcome::NotImplemented);
    }
}
