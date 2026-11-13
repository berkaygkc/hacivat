//! Credential files read the libpq way: `.pgpass` and `pg_service.conf` (HV-SEC-03).

mod pg_service;
mod pgpass;

pub use pg_service::{
    PgService, PgServiceFile, ServiceFileWarning, pg_service_path, pg_service_paths,
    read_pg_service,
};
pub use pgpass::{PgPassMatch, PgPassWarning, pgpass_lookup, pgpass_path};

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use zeroize::Zeroizing;

/// The largest credential file read (1 MiB). A larger one is refused, never loaded.
pub const MAX_CREDENTIAL_FILE_BYTES: u64 = 1 << 20;

/// A credential file's contents, read once through one handle.
struct CredentialFile {
    bytes: Zeroizing<Vec<u8>>,
    /// The permission bits when group or other may access the file (`mode & 0o077 != 0`,
    /// Unix only); `None` when only the owner may.
    exposed_mode: Option<u32>,
}

/// Why a credential file was not read.
#[derive(Debug)]
enum ReadError {
    /// A directory, FIFO, device or other non-plain file.
    NotAFile,
    /// Larger than [`MAX_CREDENTIAL_FILE_BYTES`].
    TooLarge,
    /// It could not be opened or read (including a missing file).
    Io(io::Error),
}

/// Opens `path` once, checks the handle's metadata (a plain file; the mode on Unix), then reads
/// at most [`MAX_CREDENTIAL_FILE_BYTES`]. Checking the open handle rather than the path leaves
/// no window between the check and the read. On Unix the open is non-blocking, so a FIFO is
/// refused instead of waiting for a writer. Symlinks are followed, as libpq's `stat` does.
fn read_credential_file(path: &Path) -> Result<CredentialFile, ReadError> {
    let file = match open_nonblocking(path) {
        Ok(file) => file,
        // Windows refuses to open a directory (access denied) before we can look at it; report
        // it as "not a file" like every other platform does after the open.
        Err(_) if std::fs::metadata(path).is_ok_and(|m| !m.is_file()) => {
            return Err(ReadError::NotAFile);
        }
        Err(e) => return Err(ReadError::Io(e)),
    };
    let meta = file.metadata().map_err(ReadError::Io)?;
    if !meta.is_file() {
        return Err(ReadError::NotAFile);
    }
    if meta.len() > MAX_CREDENTIAL_FILE_BYTES {
        return Err(ReadError::TooLarge);
    }
    let exposed_mode = exposed_mode(&meta);
    // Sized up front so the buffer is not reallocated (and copied) while it fills, unless the
    // file grows during the read.
    let capacity = usize::try_from(meta.len()).unwrap_or(0).saturating_add(1);
    let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
    file.take(MAX_CREDENTIAL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > MAX_CREDENTIAL_FILE_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(CredentialFile {
        bytes,
        exposed_mode,
    })
}

/// `O_NONBLOCK` for the platforms Hacivat ships on (no `libc` dependency). It only affects FIFOs
/// and devices, which are refused right after the open, so 0 is a safe fallback: the open of a
/// FIFO may then block until a writer appears, but nothing is ever read from it.
///
/// On Linux and Android the value is per architecture: `0o4000` on x86, x86_64, arm, aarch64,
/// riscv64, powerpc, powerpc64 and s390x; mips, sparc and alpha use other values, so they (and
/// every other unix) get 0 rather than a wrong flag. Apple and the BSDs use 4.
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "s390x"
    )
))]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(any(
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
const O_NONBLOCK: i32 = 4;
#[cfg(all(
    unix,
    not(any(
        all(
            any(target_os = "linux", target_os = "android"),
            any(
                target_arch = "x86",
                target_arch = "x86_64",
                target_arch = "arm",
                target_arch = "aarch64",
                target_arch = "riscv64",
                target_arch = "powerpc",
                target_arch = "powerpc64",
                target_arch = "s390x"
            )
        ),
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
const O_NONBLOCK: i32 = 0;

#[cfg(unix)]
fn open_nonblocking(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(unix)]
fn exposed_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & 0o777;
    (mode & 0o077 != 0).then_some(mode)
}

/// libpq checks permissions on Unix only.
#[cfg(not(unix))]
fn exposed_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
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

    /// The hard-coded flag matches the platform's value on the hosts this test can run on.
    #[cfg(any(
        target_vendor = "apple",
        all(
            target_os = "linux",
            any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")
        )
    ))]
    #[test]
    fn o_nonblock_matches_the_host() {
        let want = if cfg!(target_vendor = "apple") {
            4
        } else {
            0o4000
        };
        assert_eq!(O_NONBLOCK, want);
    }

    /// With a real flag, opening a FIFO that has no writer is refused instead of blocking.
    #[cfg(any(target_vendor = "apple", target_os = "linux"))]
    #[test]
    fn a_fifo_is_refused_without_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) || O_NONBLOCK == 0 {
            return;
        }
        assert!(matches!(
            read_credential_file(&fifo),
            Err(ReadError::NotAFile)
        ));
    }

    #[test]
    fn missing_directory_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_credential_file(&dir.path().join("nope")),
            Err(ReadError::Io(e)) if e.kind() == io::ErrorKind::NotFound
        ));
        assert!(matches!(
            read_credential_file(dir.path()),
            Err(ReadError::NotAFile)
        ));
        let big = dir.path().join("big");
        std::fs::write(&big, vec![b'#'; MAX_CREDENTIAL_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            read_credential_file(&big),
            Err(ReadError::TooLarge)
        ));
        let edge = dir.path().join("edge");
        std::fs::write(&edge, vec![b'#'; MAX_CREDENTIAL_FILE_BYTES as usize]).unwrap();
        assert_eq!(
            read_credential_file(&edge).unwrap().bytes.len() as u64,
            MAX_CREDENTIAL_FILE_BYTES
        );
    }

    #[cfg(unix)]
    #[test]
    fn devices_are_refused_without_reading() {
        assert!(matches!(
            read_credential_file(Path::new("/dev/zero")),
            Err(ReadError::NotAFile)
        ));
    }
}
