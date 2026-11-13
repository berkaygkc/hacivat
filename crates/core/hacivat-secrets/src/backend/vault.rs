//! The encrypted-file vault (R5, HV-SEC-05, D-016): where no OS keyring is usable, `Forever`
//! secrets live in `vault.bin`, XChaCha20-Poly1305 under an Argon2id key derived from a master
//! password (Task 6, Q30).
//!
//! Task 1 ships the interface and a fail-closed stub: the vault never exists, never unlocks and
//! stores nothing, so no secret is written unencrypted or under an unchecked key.

use std::fmt;
use std::path::PathBuf;

use secrecy::SecretString;

use super::{BackendChoice, SecretBackend, SecretError};
use crate::store::SecretKey;

/// A backend that must be unlocked with a master password first (R5, HV-SEC-05).
pub trait VaultBackend: SecretBackend {
    /// Whether the vault file exists.
    fn exists(&self) -> bool;
    fn is_unlocked(&self) -> bool;
    /// Creates a new, unlocked vault under `master`.
    fn create(&self, master: SecretString) -> Result<(), SecretError>;
    fn unlock(&self, master: SecretString) -> Result<(), SecretError>;
    /// Forgets the key and the decrypted map.
    fn lock(&self);
    /// Deletes the vault and everything in it.
    fn reset(&self) -> Result<(), SecretError>;
}

/// Argon2id parameters: memory in KiB, iterations, lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VaultParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for VaultParams {
    /// 64 MiB, 3 iterations, 1 lane.
    fn default() -> Self {
        VaultParams {
            m_kib: 65536,
            t: 3,
            p: 1,
        }
    }
}

/// The vault file (`LocalPaths::vault_bin`). Opening does no I/O. `Debug` shows the path and
/// the parameters only, never a key or a secret.
pub struct FileVault {
    path: PathBuf,
    params: VaultParams,
}

impl fmt::Debug for FileVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileVault")
            .field("path", &self.path)
            .field("params", &self.params)
            .field("unlocked", &self.is_unlocked())
            .finish()
    }
}

impl FileVault {
    /// The vault at `path` with the default parameters.
    pub fn open(path: PathBuf) -> FileVault {
        Self::with_params(path, VaultParams::default())
    }

    /// The vault at `path` with parameters `p` (tests use small ones).
    pub fn with_params(path: PathBuf, p: VaultParams) -> FileVault {
        FileVault { path, params: p }
    }
}

impl SecretBackend for FileVault {
    /// Task 1 stub: locked.
    fn get(&self, _key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        Err(SecretError::VaultLocked)
    }

    /// Task 1 stub: locked.
    fn put(&self, _key: &SecretKey, _value: &SecretString) -> Result<(), SecretError> {
        Err(SecretError::VaultLocked)
    }

    /// Task 1 stub: locked.
    fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::VaultLocked)
    }

    fn name(&self) -> &'static str {
        "vault"
    }

    fn as_vault(&self) -> Option<&dyn VaultBackend> {
        Some(self)
    }

    fn choice(&self) -> Option<BackendChoice> {
        Some(BackendChoice::Vault)
    }
}

impl VaultBackend for FileVault {
    /// Task 1 stub: never exists.
    fn exists(&self) -> bool {
        false
    }

    /// Task 1 stub: never unlocked.
    fn is_unlocked(&self) -> bool {
        false
    }

    /// Task 1 stub: unavailable.
    fn create(&self, _master: SecretString) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    /// Task 1 stub: unavailable.
    fn unlock(&self, _master: SecretString) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    fn lock(&self) {}

    /// Task 1 stub: unavailable.
    fn reset(&self) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SecretSlot;
    use hacivat_proto::ProfileId;

    #[test]
    fn the_stub_is_locked_and_stores_nothing() {
        let vault = FileVault::open(PathBuf::from("/nonexistent/vault.bin"));
        let key = SecretKey {
            profile: ProfileId::new(),
            slot: SecretSlot::DbPassword,
        };
        assert!(!vault.exists() && !vault.is_unlocked());
        assert_eq!(
            vault.put(&key, &SecretString::from("s3cret")),
            Err(SecretError::VaultLocked)
        );
        assert!(matches!(vault.get(&key), Err(SecretError::VaultLocked)));
        assert!(vault.unlock(SecretString::from("s3cret")).is_err());
        assert!(vault.as_vault().is_some());
        assert_eq!(vault.choice(), Some(BackendChoice::Vault));
        assert_eq!(VaultParams::default().m_kib, 65536);
        assert!(!format!("{vault:?}").contains("s3cret"));
    }
}
