//! The encrypted-file vault: interface only in M2a (R5, HV-SEC-05).

use secrecy::SecretString;

use super::{SecretBackend, SecretError};

/// A backend that must be unlocked with a master password first (R5, HV-SEC-05).
pub trait VaultBackend: SecretBackend {
    fn is_unlocked(&self) -> bool;
    fn unlock(&self, master: SecretString) -> Result<(), SecretError>;
}
