//! Credential files read the libpq way: `.pgpass` and `pg_service.conf` (HV-SEC-03).

mod pg_service;
mod pgpass;

pub use pg_service::{PgService, pg_service_path, read_pg_service};
pub use pgpass::{PgPassMatch, PgPassWarning, pgpass_lookup, pgpass_path};

use std::path::{Path, PathBuf};

use secrecy::SecretString;
use zeroize::Zeroizing;

/// What a credential file's metadata allows (HV-SEC-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileCheck {
    /// It does not exist, or its metadata cannot be read.
    Missing,
    /// A directory or another non-plain file.
    NotAFile,
    /// Group or other may access it (`mode & 0o077 != 0`, Unix only); `mode` holds the
    /// permission bits.
    #[cfg_attr(
        not(unix),
        allow(dead_code, reason = "libpq checks permissions on Unix only")
    )]
    Exposed { mode: u32 },
    /// A plain file only its owner may access (always, off Unix).
    Private,
}

/// The libpq permission rule for a file that may hold passwords. Symlinks are followed, as
/// libpq's `stat` does.
fn check_file(path: &Path) -> FileCheck {
    let Ok(meta) = std::fs::metadata(path) else {
        return FileCheck::Missing;
    };
    if !meta.is_file() {
        return FileCheck::NotAFile;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return FileCheck::Exposed { mode };
        }
    }
    FileCheck::Private
}

/// The user's home directory, if the platform reports one.
fn home_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf())
}

/// A secret from file bytes. Invalid UTF-8 is replaced (U+FFFD); every intermediate copy is
/// zeroized.
fn secret_from_bytes(bytes: &[u8]) -> SecretString {
    match std::str::from_utf8(bytes) {
        Ok(s) => SecretString::from(s),
        Err(_) => {
            let lossy = Zeroizing::new(String::from_utf8_lossy(bytes).into_owned());
            SecretString::from(lossy.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn invalid_utf8_is_replaced_not_dropped() {
        assert!(secret_from_bytes(b"ok").expose_secret() == "ok");
        assert!(secret_from_bytes(b"a\xffb").expose_secret() == "a\u{fffd}b");
    }

    #[test]
    fn a_missing_path_and_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_file(&dir.path().join("nope")), FileCheck::Missing);
        assert_eq!(check_file(dir.path()), FileCheck::NotAFile);
    }
}
