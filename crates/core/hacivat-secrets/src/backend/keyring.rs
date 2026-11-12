//! The OS keyring backend: macOS Keychain, Windows Credential Manager, the Linux Secret Service
//! (HV-SEC-05, R6, R7). This module shadows the `keyring` crate, so the crate is `::keyring`.

use std::collections::HashMap;

use ::keyring::Entry;
use ::keyring::Error as KeyringError;
use ::keyring::credential::CredentialPersistence;
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

use super::{SecretBackend, SecretError};
use crate::store::SecretKey;

/// The account of every Hacivat entry (R7).
const ACCOUNT: &str = "hacivat";

/// Windows Credential Manager's blob limit in bytes (`CRED_MAX_CREDENTIAL_BLOB_SIZE`). A
/// password is stored as UTF-16, so it holds at most 1,280 UTF-16 code units.
const WINDOWS_MAX_BLOB: usize = 2560;

/// Secrets in the OS keyring under `SecretKey::service`, account `hacivat` (HV-SEC-05, R7).
/// Every call blocks and may show an OS prompt: run it on the core runtime's blocking pool.
pub struct KeyringBackend {
    max_blob: Option<usize>,
}

impl KeyringBackend {
    /// A backend refusing values over `max_blob` bytes. On Windows the limit is never above
    /// 2,560 bytes of UTF-16, whatever is passed.
    pub fn new(max_blob: Option<usize>) -> Self {
        let max_blob = if cfg!(windows) {
            Some(max_blob.map_or(WINDOWS_MAX_BLOB, |m| m.min(WINDOWS_MAX_BLOB)))
        } else {
            max_blob
        };
        KeyringBackend { max_blob }
    }

    /// Whether a keyring is reachable, without prompting: true on macOS and Windows; on Linux
    /// (and the BSDs) a Secret Service probe (P36). Keyring's mock store is never available.
    /// Never call it on the UI thread.
    pub fn available() -> bool {
        if !Self::persistent() {
            false
        } else if cfg!(any(target_os = "macos", target_os = "windows")) {
            true
        } else if cfg!(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd"
        )) {
            secret_service_reachable()
        } else {
            false
        }
    }

    /// Whether keyring's default store persists until deleted, i.e. it is not keyring's mock
    /// store, which keyring silently uses when built without its native-store features.
    pub fn persistent() -> bool {
        matches!(
            ::keyring::default::default_credential_builder().persistence(),
            CredentialPersistence::UntilDelete
        )
    }

    fn entry(key: &SecretKey) -> Result<Entry, SecretError> {
        Entry::new(&key.service(), ACCOUNT).map_err(map_error)
    }

    /// The value's stored size in bytes: UTF-16 on Windows, UTF-8 elsewhere.
    fn blob_len(value: &str) -> usize {
        if cfg!(windows) {
            value.encode_utf16().count() * 2
        } else {
            value.len()
        }
    }
}

impl SecretBackend for KeyringBackend {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        match Self::entry(key)?.get_password() {
            Ok(mut password) => {
                let secret = SecretString::from(password.as_str());
                password.zeroize();
                Ok(Some(secret))
            }
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(map_error(e)),
        }
    }

    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        if let Some(limit) = self.max_blob
            && Self::blob_len(value.expose_secret()) > limit
        {
            return Err(SecretError::TooLarge { limit });
        }
        Self::entry(key)?
            .set_password(value.expose_secret())
            .map_err(map_error)
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(map_error(e)),
        }
    }

    fn name(&self) -> &'static str {
        "keyring"
    }

    fn probe_available(&self) -> bool {
        Self::available()
    }

    /// The Windows credential's comment, or the Secret Service item's label. macOS keyring 3
    /// keeps no label, so it is a no-op there (P44).
    fn set_label(&self, key: &SecretKey, label: &str) -> Result<(), SecretError> {
        let attribute = if cfg!(windows) {
            "comment"
        } else if cfg!(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd"
        )) {
            "label"
        } else {
            return Ok(());
        };
        if label.is_empty() {
            return Ok(());
        }
        Self::entry(key)?
            .update_attributes(&HashMap::from([(attribute, label)]))
            .map_err(map_error)
    }
}

/// Maps a keyring error, never carrying a secret. `NoStorageAccess` (a locked store, access
/// rules) becomes `Denied`; `PlatformFailure` (no reachable store) becomes `Unavailable`;
/// `TooLong` becomes `TooLarge`. A non-UTF-8 value's bytes are zeroized, never shown.
fn map_error(e: KeyringError) -> SecretError {
    match e {
        KeyringError::NoStorageAccess(_) => SecretError::Denied,
        KeyringError::PlatformFailure(_) => SecretError::Unavailable,
        KeyringError::TooLong(_, limit) => SecretError::TooLarge {
            limit: limit as usize,
        },
        KeyringError::BadEncoding(mut bytes) => {
            bytes.zeroize();
            SecretError::Backend("bad-encoding".to_string())
        }
        KeyringError::Invalid(attribute, _) => SecretError::Backend(format!("invalid-{attribute}")),
        KeyringError::Ambiguous(_) => SecretError::Backend("ambiguous".to_string()),
        KeyringError::NoEntry => SecretError::Backend("no-entry".to_string()),
        _ => SecretError::Backend("keyring".to_string()),
    }
}

/// Whether the Secret Service (`org.freedesktop.secrets`) answers on the D-Bus session bus. It
/// looks up an entry that is never written: the search opens a session but unlocks nothing,
/// so it never prompts.
fn secret_service_reachable() -> bool {
    let probe = format!(
        "{}/probe",
        if cfg!(debug_assertions) {
            "hacivat-dev"
        } else {
            "hacivat"
        }
    );
    match Entry::new(&probe, ACCOUNT).and_then(|entry| entry.get_password()) {
        Ok(mut password) => {
            password.zeroize();
            true
        }
        Err(KeyringError::NoEntry | KeyringError::Ambiguous(_)) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_without_the_secret() {
        assert_eq!(
            map_error(KeyringError::TooLong("password".into(), 2560)),
            SecretError::TooLarge { limit: 2560 }
        );
        assert_eq!(
            map_error(KeyringError::NoStorageAccess("locked".into())),
            SecretError::Denied
        );
        assert_eq!(
            map_error(KeyringError::PlatformFailure("no bus".into())),
            SecretError::Unavailable
        );
        let mapped = map_error(KeyringError::BadEncoding(b"hv_test_pw".to_vec()));
        assert!(!format!("{mapped:?}").contains("hv_test_pw"));
    }

    #[test]
    fn a_blob_over_the_limit_is_refused_before_the_keyring() {
        let b = KeyringBackend::new(Some(4));
        let key = SecretKey {
            profile: hacivat_proto::ProfileId::new(),
            slot: crate::store::SecretSlot::DbPassword,
        };
        assert_eq!(
            b.put(&key, &SecretString::from("12345")),
            Err(SecretError::TooLarge { limit: 4 })
        );
        assert_eq!(b.name(), "keyring");
    }
}
