//! The OS keyring backend: macOS Keychain, Windows Credential Manager, the Linux Secret Service
//! (HV-SEC-05). Body in Task 4.

use secrecy::SecretString;

use super::{SecretBackend, SecretError};
use crate::store::SecretKey;

/// Secrets in the OS keyring under `SecretKey::service` (HV-SEC-05, R7).
pub struct KeyringBackend {
    #[allow(dead_code, reason = "Task 4")]
    max_blob: Option<usize>,
}

impl KeyringBackend {
    /// A backend refusing values over `max_blob` bytes (Windows: 2560).
    pub fn new(max_blob: Option<usize>) -> Self {
        KeyringBackend { max_blob }
    }

    /// Whether a keyring is reachable, without prompting: true on macOS and Windows, a D-Bus
    /// name probe for `org.freedesktop.secrets` on Linux (P36). Never call it on the UI thread.
    pub fn available() -> bool {
        false
    }

    /// Whether keyring's default store persists until deleted, i.e. it is not keyring's mock
    /// store (Task 4 tests this). The stub answers `false`.
    pub fn persistent() -> bool {
        false
    }
}

impl SecretBackend for KeyringBackend {
    fn get(&self, _key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        Err(SecretError::Unavailable)
    }

    fn put(&self, _key: &SecretKey, _value: &SecretString) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    fn name(&self) -> &'static str {
        "keyring"
    }
}
