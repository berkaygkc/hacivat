//! The secret store: keys, retention and the store that routes each retention to a backend or
//! an in-process map (HV-SEC-05). Bodies in Task 4.

use std::sync::Arc;

use hacivat_proto::ProfileId;
use secrecy::SecretString;

use crate::backend::{SecretBackend, SecretError};

/// Which secret of a profile (HV-SEC-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretSlot {
    DbPassword,
    TlsKeyPassphrase,
    SshPassword { hop: u8 },
    SshPassphrase { hop: u8 },
    ProxyPassword,
}

impl SecretSlot {
    /// The slot's key part: `db-password`, `tls-key-passphrase`, `ssh-<hop>-password`,
    /// `ssh-<hop>-passphrase`, `proxy-password`.
    pub fn id(self) -> String {
        match self {
            SecretSlot::DbPassword => "db-password".to_string(),
            SecretSlot::TlsKeyPassphrase => "tls-key-passphrase".to_string(),
            SecretSlot::SshPassword { hop } => format!("ssh-{hop}-password"),
            SecretSlot::SshPassphrase { hop } => format!("ssh-{hop}-passphrase"),
            SecretSlot::ProxyPassword => "proxy-password".to_string(),
        }
    }
}

/// How long a secret is kept (HV-SEC-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Retention {
    Never,
    UntilRestart,
    ForSession,
    #[default]
    Forever,
}

impl Retention {
    pub const ALL: [Retention; 4] = [
        Retention::Never,
        Retention::UntilRestart,
        Retention::ForSession,
        Retention::Forever,
    ];

    /// The stable ID: `never`, `until_restart`, `for_session`, `forever`.
    pub fn id(self) -> &'static str {
        match self {
            Retention::Never => "never",
            Retention::UntilRestart => "until_restart",
            Retention::ForSession => "for_session",
            Retention::Forever => "forever",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.id() == id)
    }
}

/// One secret's address (HV-SEC-05).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretKey {
    pub profile: ProfileId,
    pub slot: SecretSlot,
}

impl SecretKey {
    /// The keyring service name, with the `hacivat-dev/` prefix in dev builds (R7, Task 4).
    pub fn service(&self) -> String {
        String::new()
    }
}

/// Where a `put` landed (HV-SEC-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Stored {
    AsRequested,
    /// The backend refused; the secret is kept with a shorter retention instead.
    FellBackTo {
        retention: Retention,
        reason: SecretError,
    },
}

/// Routes each retention to its storage (HV-SEC-05). Blocking: callers run it on the core
/// runtime's blocking pool, never on the UI thread.
pub struct SecretStore {
    #[allow(dead_code, reason = "Task 4")]
    backend: Option<Arc<dyn SecretBackend>>,
}

impl SecretStore {
    /// A store over `backend` (`None`: no persistent backend; `Forever` falls back).
    pub fn new(backend: Option<Arc<dyn SecretBackend>>) -> Self {
        SecretStore { backend }
    }

    pub fn get(
        &self,
        _key: &SecretKey,
        _retention: Retention,
    ) -> Result<Option<SecretString>, SecretError> {
        Ok(None)
    }

    pub fn put(
        &self,
        _key: &SecretKey,
        _value: SecretString,
        _retention: Retention,
    ) -> Result<Stored, SecretError> {
        Err(SecretError::Unavailable)
    }

    pub fn change_retention(
        &self,
        _key: &SecretKey,
        _from: Retention,
        _to: Retention,
    ) -> Result<Stored, SecretError> {
        Err(SecretError::Unavailable)
    }

    /// Forgets the profile's `ForSession` secrets.
    pub fn forget_session(&self, _profile: ProfileId) {}

    pub fn delete_all(&self, _profile: ProfileId) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    pub fn copy_all(
        &self,
        _from: ProfileId,
        _to: ProfileId,
        _retention: Retention,
    ) -> Result<(), SecretError> {
        Err(SecretError::Unavailable)
    }

    /// Whether the OS keyring is usable: `KeyringBackend::available()` computed once, off the
    /// UI thread, then cached (P36).
    pub fn keyring_available(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_and_retention_ids() {
        assert_eq!(SecretSlot::DbPassword.id(), "db-password");
        assert_eq!(SecretSlot::TlsKeyPassphrase.id(), "tls-key-passphrase");
        assert_eq!(SecretSlot::SshPassword { hop: 1 }.id(), "ssh-1-password");
        assert_eq!(
            SecretSlot::SshPassphrase { hop: 2 }.id(),
            "ssh-2-passphrase"
        );
        assert_eq!(SecretSlot::ProxyPassword.id(), "proxy-password");
        for r in Retention::ALL {
            assert_eq!(Retention::from_id(r.id()), Some(r));
        }
        assert_eq!(Retention::default(), Retention::Forever);
    }
}
