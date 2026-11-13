//! Secret backends: the blocking storage interface and its implementations (HV-SEC-05).

// ## m2b-t01-interfaces
mod fallback;
mod keyring;
mod memory;
mod vault;

pub use fallback::FallbackBackend;
pub use keyring::KeyringBackend;
pub use memory::MemoryBackend;
pub use vault::{FileVault, VaultBackend, VaultParams};

use secrecy::SecretString;

use crate::store::SecretKey;

/// Why a secret could not be read or written (HV-SEC-05). Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretError {
    /// No usable backend (no keyring service, a locked vault).
    Unavailable,
    /// The user or the OS refused access.
    Denied,
    /// The value is larger than the backend stores (Windows: 2560 bytes).
    TooLarge { limit: usize },
    /// Any other backend failure, as the backend describes it.
    Backend(String),
    // ## m2b-t01-interfaces
    /// The vault exists but is not unlocked.
    VaultLocked,
    /// The vault file cannot be read as a vault (Task 6).
    VaultCorrupt,
    /// The master password does not open the vault.
    WrongMasterPassword,
    /// The vault was written by a newer Hacivat, or its file cannot be written.
    VaultReadOnly,
}

// ## m2b-t01-interfaces
/// Which backend holds `Forever` secrets (Q23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendChoice {
    Keyring,
    Vault,
    Memory,
}

/// A secret storage backend (HV-SEC-05). Every method blocks; callers run them on the core
/// runtime's blocking pool.
pub trait SecretBackend: Send + Sync + 'static {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError>;
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError>;
    /// Deletes `key`'s entry; deleting a missing entry succeeds.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
    /// A stable backend name for diagnostics (`memory`, `keyring`).
    fn name(&self) -> &'static str;
    /// Sets the human-readable label of `key`'s existing entry, e.g. the profile name
    /// (HV-SEC-05, P44). Backends without labels ignore it.
    fn set_label(&self, _key: &SecretKey, _label: &str) -> Result<(), SecretError> {
        Ok(())
    }
    /// Whether the backend can store secrets now, without prompting (P36). May block (a D-Bus
    /// probe); `SecretStore::keyring_available` calls it once and caches the answer.
    fn probe_available(&self) -> bool {
        true
    }
    // ## m2b-t01-interfaces
    /// The vault behind this backend, when it is one (or chose one).
    fn as_vault(&self) -> Option<&dyn VaultBackend> {
        None
    }
    /// Which backend this is, once known: the keyring once probed, the memory backend always,
    /// the fallback backend its probed choice (Q23). `None` until then.
    fn choice(&self) -> Option<BackendChoice> {
        None
    }
}
