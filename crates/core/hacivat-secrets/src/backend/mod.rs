//! Secret backends: the blocking storage interface and its implementations (HV-SEC-05).

mod keyring;
mod memory;
mod vault;

pub use keyring::KeyringBackend;
pub use memory::MemoryBackend;
pub use vault::VaultBackend;

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
}

/// A secret storage backend (HV-SEC-05). Every method blocks; callers run them on the core
/// runtime's blocking pool.
pub trait SecretBackend: Send + Sync + 'static {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError>;
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError>;
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
    /// A stable backend name for diagnostics (`memory`, `keyring`).
    fn name(&self) -> &'static str;
}
