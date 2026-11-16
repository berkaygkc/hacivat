//! The fallback backend (HV-SEC-05, D-016, Q23): the OS keyring where it is usable, else the
//! encrypted-file vault. The choice is made once, by a non-prompting probe, the first time the
//! backend is used or `probe_available` is asked.
//!
//! `probe_available` is true either way: with the vault chosen, `Forever` stays enabled and
//! the dialog labels it with the vault (Q23). `choice` and `as_vault` follow the probed choice.

use std::sync::{Arc, OnceLock};

use secrecy::SecretString;

use super::vault::{FileVault, VaultBackend};
use super::{BackendChoice, SecretBackend, SecretError};
use crate::store::SecretKey;

/// The keyring, or the vault where no keyring is usable.
pub struct FallbackBackend {
    keyring: Arc<dyn SecretBackend>,
    vault: Arc<FileVault>,
    probe: Box<dyn Fn() -> bool + Send + Sync>,
    chosen: OnceLock<BackendChoice>,
}

impl FallbackBackend {
    /// `probe` answers whether the keyring is usable (without prompting, P36).
    pub fn new(
        keyring: Arc<dyn SecretBackend>,
        vault: Arc<FileVault>,
        probe: Box<dyn Fn() -> bool + Send + Sync>,
    ) -> Self {
        FallbackBackend {
            keyring,
            vault,
            probe,
            chosen: OnceLock::new(),
        }
    }

    /// The choice, probing once.
    fn choose(&self) -> BackendChoice {
        *self.chosen.get_or_init(|| {
            if (self.probe)() {
                BackendChoice::Keyring
            } else {
                BackendChoice::Vault
            }
        })
    }

    fn active(&self) -> &dyn SecretBackend {
        match self.choose() {
            BackendChoice::Vault => self.vault.as_ref(),
            BackendChoice::Keyring | BackendChoice::Memory => self.keyring.as_ref(),
        }
    }
}

impl SecretBackend for FallbackBackend {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        self.active().get(key)
    }

    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.active().put(key, value)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.active().delete(key)
    }

    fn name(&self) -> &'static str {
        match self.chosen.get() {
            Some(_) => self.active().name(),
            None => "fallback",
        }
    }

    fn set_label(&self, key: &SecretKey, label: &str) -> Result<(), SecretError> {
        self.active().set_label(key, label)
    }

    /// Probes once; true with either backend (Q23).
    fn probe_available(&self) -> bool {
        self.choose();
        true
    }

    fn as_vault(&self) -> Option<&dyn VaultBackend> {
        match self.chosen.get() {
            Some(BackendChoice::Vault) => Some(self.vault.as_ref()),
            _ => None,
        }
    }

    fn choice(&self) -> Option<BackendChoice> {
        self.chosen.get().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;
    use std::path::PathBuf;

    fn fallback(keyring_usable: bool) -> FallbackBackend {
        FallbackBackend::new(
            Arc::new(MemoryBackend::new()),
            Arc::new(FileVault::open(PathBuf::from("/nonexistent/vault.bin"))),
            Box::new(move || keyring_usable),
        )
    }

    #[test]
    fn the_probe_chooses_once_and_forever_stays_available() {
        let with_keyring = fallback(true);
        assert_eq!(with_keyring.choice(), None, "not probed yet");
        assert!(with_keyring.probe_available());
        assert_eq!(with_keyring.choice(), Some(BackendChoice::Keyring));
        assert!(with_keyring.as_vault().is_none());

        let without = fallback(false);
        assert!(without.probe_available());
        assert_eq!(without.choice(), Some(BackendChoice::Vault));
        assert!(without.as_vault().is_some());
        assert_eq!(without.name(), "vault");
    }
}
