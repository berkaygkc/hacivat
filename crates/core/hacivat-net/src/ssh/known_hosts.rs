//! Known host keys (HV-SEC-01, B11, Task 4): Hacivat's own `known_hosts` file, read and
//! written, and the user's OpenSSH files, only read. Loading does no I/O (Q42): the files are
//! read lazily at the first check or trust, and an OpenSSH file is re-read when its `(mtime,
//! len)` changed (Q24).
//!
//! Task 1 stub: the check refuses every key (`Revoked`), and nothing is ever trusted, so no
//! policy (not even `AcceptNew`) can connect before Task 4.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{HostKey, HostKeyCheck, KnownSource};

/// The known-host stores.
#[derive(Debug)]
pub struct KnownHosts {
    openssh_files: Vec<PathBuf>,
    hacivat_file: PathBuf,
}

impl KnownHosts {
    /// The stores: `openssh_files` (read only) and `hacivat_file` (Hacivat's own). No I/O.
    pub fn load(openssh_files: Vec<PathBuf>, hacivat_file: PathBuf) -> Arc<KnownHosts> {
        Arc::new(KnownHosts {
            openssh_files,
            hacivat_file,
        })
    }

    /// What the stores say about `key` for `host:port`. Task 1 stub: `Revoked` (refuses).
    pub fn check(&self, _host: &str, _port: u16, _key: &HostKey) -> HostKeyCheck {
        HostKeyCheck::Revoked {
            source: KnownSource::Hacivat,
        }
    }

    /// Trusts `key` for `host:port` in Hacivat's store. Task 1 stub: unsupported.
    pub fn trust(&self, _host: &str, _port: u16, _key: &HostKey) -> io::Result<()> {
        Err(unsupported())
    }

    /// Replaces `old` with `new` for `host:port` in Hacivat's store. Task 1 stub: unsupported.
    pub fn replace(
        &self,
        _host: &str,
        _port: u16,
        _old: &HostKey,
        _new: &HostKey,
    ) -> io::Result<()> {
        Err(unsupported())
    }

    /// Hacivat's own `known_hosts` file.
    pub fn hacivat_path(&self) -> &Path {
        &self.hacivat_file
    }

    /// The OpenSSH files the stores read (never write).
    pub fn openssh_files(&self) -> &[PathBuf] {
        &self.openssh_files
    }
}

fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "known_hosts: Task 4")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_loads_without_io_and_refuses_every_key() {
        let missing = PathBuf::from("/nonexistent/hacivat/known_hosts");
        let known = KnownHosts::load(vec![PathBuf::from("/nonexistent/.ssh")], missing.clone());
        assert_eq!(known.hacivat_path(), missing);
        assert_eq!(known.openssh_files(), [PathBuf::from("/nonexistent/.ssh")]);
        let key = HostKey {
            algorithm: "ssh-ed25519".into(),
            sha256: [1; 32],
            openssh: "ssh-ed25519 AAAA".into(),
        };
        assert!(matches!(
            known.check("bastion", 22, &key),
            HostKeyCheck::Revoked { .. }
        ));
        assert!(known.trust("bastion", 22, &key).is_err());
        assert!(known.replace("bastion", 22, &key, &key).is_err());
    }
}
