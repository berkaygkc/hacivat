//! The per-profile SQLite catalog cache (HV-MODEL-03, R-M4-15, R-M4-25, D-016): one file per
//! profile (`<profile>.v<major>.sqlite`, mode 0600), one row per namespace, optional row
//! encryption, a content-addressed source store and a server identity check.
//!
//! M4 Task 1a seam (spike origin `d86313b`, declarations only): [`CacheStore::open`] answers
//! `Err(CacheError::Disabled)`, so the cache is memory-only until Task 11. Nothing is written.

pub mod crypt;

use std::fmt;
use std::path::{Path, PathBuf};

use hacivat_proto::ProfileId;

use crate::codec::{DecodeError, FORMAT_MAJOR};
use crate::ids::{NsKey, SourceRef};
use crate::namespace::NamespaceSnapshot;

/// The cache file name of `profile` (under the settings `catalog/` directory).
pub fn cache_file_name(profile: &ProfileId) -> String {
    format!("{}.v{FORMAT_MAJOR}.sqlite", profile.0)
}

/// A 32-byte per-profile data key (secret store slot `CatalogKey`). Its `Debug` is redacted
/// and it has no `PartialEq`: compare through the cipher, never by bytes.
#[derive(Clone)]
pub struct RowKey([u8; 32]);

impl RowKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        RowKey(bytes)
    }
}

impl zeroize::Zeroize for RowKey {
    fn zeroize(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

impl Drop for RowKey {
    /// The key bytes are wiped through `zeroize` (volatile writes the optimizer keeps; fix
    /// round 1, m4).
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(self);
    }
}

impl zeroize::ZeroizeOnDrop for RowKey {}

impl fmt::Debug for RowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RowKey(<redacted>)")
    }
}

/// How rows are stored.
#[derive(Debug, Clone)]
pub enum Encryption {
    /// Plaintext rows (the file itself is 0600, D-016).
    None,
    /// XChaCha20-Poly1305 per row (`RowCipher`, domain `hv-catalog-v1`).
    Row(RowKey),
}

/// Which process writes the file (one writer per file; another writer finds it `Locked`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WriterId(pub u64);

/// Why the cache cannot be used.
#[derive(Debug)]
pub enum CacheError {
    /// Persistence is off (memory-only): the M4 Task 1a stub, or no key for an encrypted
    /// profile (the keyring is unavailable).
    Disabled,
    Io(std::io::Error),
    /// The file was corrupt and was moved aside.
    Corrupt {
        renamed_to: PathBuf,
    },
    /// Another writer holds the file.
    Locked,
    /// Another major format.
    Incompatible,
    /// A temp namespace is never persisted.
    TempNamespace,
    /// The row comes from a newer minor: read, never rewritten.
    NewerMinor,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Disabled => write!(f, "the catalog cache is memory-only"),
            CacheError::Io(e) => write!(f, "cache i/o: {e}"),
            CacheError::Corrupt { renamed_to } => {
                write!(f, "corrupt cache moved to {}", renamed_to.display())
            }
            CacheError::Locked => write!(f, "the cache is locked by another writer"),
            CacheError::Incompatible => write!(f, "incompatible cache format"),
            CacheError::TempNamespace => write!(f, "temp namespaces are not persisted"),
            CacheError::NewerMinor => write!(f, "row from a newer format minor"),
        }
    }
}

impl std::error::Error for CacheError {}

/// Why an opened cache was rebuilt empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildReason {
    Corrupt,
    MajorChanged,
    WrongKey,
    ServerChanged,
}

/// An opened cache, and whether it had to be rebuilt.
#[derive(Debug)]
pub struct Opened {
    pub store: CacheStore,
    pub rebuilt: Option<RebuildReason>,
}

/// Every namespace a cache holds, plus the rows that could not be decoded (dropped and
/// re-introspected; only the affected row).
#[derive(Debug)]
pub struct LoadAll {
    pub namespaces: Vec<NamespaceSnapshot>,
    pub dropped: Vec<(NsKey, DecodeError)>,
}

/// The identity of the server a cache was filled from (a restore, a swapped host).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerFingerprint(pub u128);

/// Whether the server behind a profile is the one the cache was filled from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityCheck {
    Same,
    /// Every namespace is marked stale; rows are kept for offline use.
    Changed,
}

/// One profile's catalog cache.
#[derive(Debug)]
pub struct CacheStore {
    path: PathBuf,
}

impl CacheStore {
    /// Opens (or creates) the cache at `path`.
    ///
    /// Task 1a stub: `Err(CacheError::Disabled)`; nothing is created.
    pub fn open(
        path: &Path,
        encryption: Encryption,
        writer: WriterId,
    ) -> Result<Opened, CacheError> {
        let _ = (path, encryption, writer);
        Err(CacheError::Disabled)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Compares `server` with the fingerprint the cache was filled from.
    /// Task 1a stub: `Changed` (every namespace stale).
    pub fn check_identity(
        &mut self,
        server: ServerFingerprint,
    ) -> Result<IdentityCheck, CacheError> {
        let _ = server;
        Ok(IdentityCheck::Changed)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn load_namespace(&self, key: &NsKey) -> Result<Option<NamespaceSnapshot>, CacheError> {
        let _ = key;
        Err(CacheError::Disabled)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn load_all(&self) -> Result<LoadAll, CacheError> {
        Err(CacheError::Disabled)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn save_namespace(&mut self, ns: &NamespaceSnapshot) -> Result<(), CacheError> {
        let _ = ns;
        Err(CacheError::Disabled)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn remove_namespace(&mut self, key: &NsKey) -> Result<bool, CacheError> {
        let _ = key;
        Err(CacheError::Disabled)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn put_source(&mut self, text: &str) -> Result<SourceRef, CacheError> {
        let _ = text;
        Err(CacheError::Disabled)
    }

    /// Task 1a stub: `Err(CacheError::Disabled)`.
    pub fn source(&self, r: SourceRef) -> Result<Option<String>, CacheError> {
        let _ = r;
        Err(CacheError::Disabled)
    }

    /// Deletes every cache file of `profile` under `dir` ("Clear cached metadata").
    /// Task 1a stub: `Err(CacheError::Disabled)`; nothing is deleted.
    pub fn delete_profile_files(dir: &Path, profile: &ProfileId) -> Result<(), CacheError> {
        let _ = (dir, profile);
        Err(CacheError::Disabled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fix round 1 (m4): `zeroize` clears every key byte (the `Drop` impl calls exactly this),
    /// and the type is `ZeroizeOnDrop`.
    #[test]
    fn row_key_zeroize_clears_every_byte() {
        fn on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        let mut key = RowKey::from_bytes([0x5A; 32]);
        on_drop(&key);
        zeroize::Zeroize::zeroize(&mut key);
        assert_eq!(key.0, [0; 32]);
    }
}
