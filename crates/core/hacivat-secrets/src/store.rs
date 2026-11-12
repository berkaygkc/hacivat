//! The secret store: keys, retention and the store that routes each retention to a backend or
//! an in-process map (HV-SEC-05, R7, R8).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

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
    /// The keyring service name `hacivat/<profile-uuid>/<slot>`, with the `hacivat-dev/`
    /// prefix in dev builds (R7).
    pub fn service(&self) -> String {
        let prefix = if cfg!(debug_assertions) {
            "hacivat-dev"
        } else {
            "hacivat"
        };
        format!("{prefix}/{}/{}", self.profile, self.slot.id())
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

/// The highest SSH hop whose reserved slots `delete_all` and `copy_all` visit in the backend
/// (R7: the SSH and proxy slots are reserved until M2b).
const MAX_SSH_HOP: u8 = 8;

/// Every slot a backend may hold for one profile. A backend cannot list its entries, so
/// `delete_all` and `copy_all` visit each of these.
fn backend_slots() -> impl Iterator<Item = SecretSlot> {
    [
        SecretSlot::DbPassword,
        SecretSlot::TlsKeyPassphrase,
        SecretSlot::ProxyPassword,
    ]
    .into_iter()
    .chain((0..=MAX_SSH_HOP).flat_map(|hop| {
        [
            SecretSlot::SshPassword { hop },
            SecretSlot::SshPassphrase { hop },
        ]
    }))
}

/// An in-process secret map. `SecretString` zeroizes a value when it is removed or dropped.
type SecretMap = Mutex<HashMap<SecretKey, SecretString>>;

/// Where a stored secret lives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Nowhere,
    Backend,
    Process,
    Session,
}

/// Routes each retention to its storage (HV-SEC-05, R8). Blocking: callers run it on the core
/// runtime's blocking pool, never on the UI thread.
///
/// - `Forever`: the backend only. When the backend is missing, or refuses with `Unavailable`,
///   `Denied` or `TooLarge`, the secret is kept in the process map instead and `put` reports
///   `FellBackTo { retention: UntilRestart, reason }`. A `Forever` get reads that fallback
///   first.
/// - `UntilRestart`: the process map.
/// - `ForSession`: the session map, cleared per profile by `forget_session`.
/// - `Never`: nothing is kept.
pub struct SecretStore {
    backend: Option<Arc<dyn SecretBackend>>,
    process: SecretMap,
    session: SecretMap,
    keyring_available: OnceLock<bool>,
}

impl SecretStore {
    /// A store over `backend` (`None`: no persistent backend; `Forever` falls back).
    pub fn new(backend: Option<Arc<dyn SecretBackend>>) -> Self {
        SecretStore {
            backend,
            process: Mutex::new(HashMap::new()),
            session: Mutex::new(HashMap::new()),
            keyring_available: OnceLock::new(),
        }
    }

    /// The secret kept for `key` under `retention`, if any.
    pub fn get(
        &self,
        key: &SecretKey,
        retention: Retention,
    ) -> Result<Option<SecretString>, SecretError> {
        match retention {
            Retention::Never => Ok(None),
            Retention::UntilRestart => Ok(lock(&self.process).get(key).cloned()),
            Retention::ForSession => Ok(lock(&self.session).get(key).cloned()),
            Retention::Forever => {
                if let Some(fallback) = lock(&self.process).get(key) {
                    return Ok(Some(fallback.clone()));
                }
                match &self.backend {
                    Some(backend) => backend.get(key),
                    None => Ok(None),
                }
            }
        }
    }

    /// Keeps `value` for `key` under `retention`. A refused `Forever` falls back to
    /// `UntilRestart` (see the type docs); any other backend error is returned.
    pub fn put(
        &self,
        key: &SecretKey,
        value: SecretString,
        retention: Retention,
    ) -> Result<Stored, SecretError> {
        match retention {
            Retention::Never => {}
            Retention::UntilRestart => {
                lock(&self.process).insert(*key, value);
            }
            Retention::ForSession => {
                lock(&self.session).insert(*key, value);
            }
            Retention::Forever => {
                let reason = match &self.backend {
                    None => SecretError::Unavailable,
                    Some(backend) => match backend.put(key, &value) {
                        Ok(()) => {
                            // A newer value supersedes an earlier fallback.
                            lock(&self.process).remove(key);
                            return Ok(Stored::AsRequested);
                        }
                        Err(
                            e @ (SecretError::Unavailable
                            | SecretError::Denied
                            | SecretError::TooLarge { .. }),
                        ) => e,
                        Err(e) => return Err(e),
                    },
                };
                lock(&self.process).insert(*key, value);
                return Ok(Stored::FellBackTo {
                    retention: Retention::UntilRestart,
                    reason,
                });
            }
        }
        Ok(Stored::AsRequested)
    }

    /// Moves `key`'s secret from `from` to `to` (R8): it is stored under `to` first, then
    /// removed from where `from` kept it, so lowering from `Forever` deletes the backend entry.
    /// With nothing stored it only clears `from`.
    pub fn change_retention(
        &self,
        key: &SecretKey,
        from: Retention,
        to: Retention,
    ) -> Result<Stored, SecretError> {
        if from == to {
            return Ok(Stored::AsRequested);
        }
        let (stored, landed) = match self.get(key, from)? {
            Some(value) => {
                let stored = self.put(key, value, to)?;
                let landed = match &stored {
                    Stored::AsRequested => place_of(to),
                    Stored::FellBackTo { .. } => Place::Process,
                };
                (stored, landed)
            }
            None => (Stored::AsRequested, Place::Nowhere),
        };
        self.clear(key, from, landed)?;
        Ok(stored)
    }

    /// Forgets the profile's `ForSession` secrets.
    pub fn forget_session(&self, profile: ProfileId) {
        lock(&self.session).retain(|k, _| k.profile != profile);
    }

    /// Deletes every secret of `profile`, whatever its retention. Every backend slot is tried;
    /// an `Unavailable` backend holds nothing, and the first other backend error is returned.
    pub fn delete_all(&self, profile: ProfileId) -> Result<(), SecretError> {
        lock(&self.process).retain(|k, _| k.profile != profile);
        lock(&self.session).retain(|k, _| k.profile != profile);
        let mut first_error = None;
        for slot in backend_slots() {
            if let Err(e) = self.backend_delete(&SecretKey { profile, slot }) {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Copies every secret `from` holds under `retention` to `to`, with the same retention
    /// (profile duplication). A `Forever` copy the backend refuses falls back like `put`. A
    /// backend read that fails with `Unavailable` or `Denied` counts as nothing stored, so a
    /// fallen-back secret still copies on a machine without a keyring.
    pub fn copy_all(
        &self,
        from: ProfileId,
        to: ProfileId,
        retention: Retention,
    ) -> Result<(), SecretError> {
        let mut slots = match retention {
            Retention::Never => return Ok(()),
            Retention::UntilRestart => profile_slots(&self.process, from),
            Retention::ForSession => profile_slots(&self.session, from),
            Retention::Forever => {
                let mut slots = profile_slots(&self.process, from);
                if self.backend.is_some() {
                    slots.extend(backend_slots());
                }
                slots
            }
        };
        slots.sort();
        slots.dedup();
        for slot in slots {
            let source = SecretKey {
                profile: from,
                slot,
            };
            let value = match self.get(&source, retention) {
                Err(SecretError::Unavailable | SecretError::Denied) => None,
                other => other?,
            };
            if let Some(value) = value {
                self.put(&SecretKey { profile: to, slot }, value, retention)?;
            }
        }
        Ok(())
    }

    /// Whether the OS keyring is usable: the backend's `probe_available()` (for the keyring,
    /// `KeyringBackend::available()`) computed once, off the UI thread, then cached (P36).
    /// Without a backend it is `false`; the memory backend is always usable.
    pub fn keyring_available(&self) -> bool {
        *self.keyring_available.get_or_init(|| {
            self.backend
                .as_ref()
                .is_some_and(|backend| backend.probe_available())
        })
    }

    // ## m2a-t12-profiles
    /// Labels `key`'s backend entry with `label`, the profile name (HV-SEC-05, P44). Call it
    /// after a `Forever` put that landed `AsRequested`; without a backend it does nothing.
    pub fn set_label(&self, key: &SecretKey, label: &str) -> Result<(), SecretError> {
        match &self.backend {
            Some(backend) => backend.set_label(key, label),
            None => Ok(()),
        }
    }

    /// Deletes `key`'s backend entry. With no backend, or an `Unavailable` one, nothing is
    /// stored there, so there is nothing to delete.
    fn backend_delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match self.backend.as_ref().map(|backend| backend.delete(key)) {
            None | Some(Ok(()) | Err(SecretError::Unavailable)) => Ok(()),
            Some(Err(e)) => Err(e),
        }
    }

    /// Removes `key` from every place `from` keeps it, except `keep` (where it just landed).
    fn clear(&self, key: &SecretKey, from: Retention, keep: Place) -> Result<(), SecretError> {
        let places: &[Place] = match from {
            Retention::Never => &[],
            Retention::UntilRestart => &[Place::Process],
            Retention::ForSession => &[Place::Session],
            Retention::Forever => &[Place::Process, Place::Backend],
        };
        for &place in places.iter().filter(|&&p| p != keep) {
            match place {
                Place::Nowhere => {}
                Place::Process => {
                    lock(&self.process).remove(key);
                }
                Place::Session => {
                    lock(&self.session).remove(key);
                }
                Place::Backend => self.backend_delete(key)?,
            }
        }
        Ok(())
    }
}

/// Where a successful `put` with `retention` keeps the secret.
fn place_of(retention: Retention) -> Place {
    match retention {
        Retention::Never => Place::Nowhere,
        Retention::UntilRestart => Place::Process,
        Retention::ForSession => Place::Session,
        Retention::Forever => Place::Backend,
    }
}

/// The slots `profile` has in `map`.
fn profile_slots(map: &SecretMap, profile: ProfileId) -> Vec<SecretSlot> {
    lock(map)
        .keys()
        .filter(|k| k.profile == profile)
        .map(|k| k.slot)
        .collect()
}

/// Locks a map, recovering from a poisoned lock: a panicking holder cannot leave a map
/// half-updated.
fn lock(map: &SecretMap) -> MutexGuard<'_, HashMap<SecretKey, SecretString>> {
    map.lock().unwrap_or_else(PoisonError::into_inner)
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

// ## m2a-t12-profiles
#[cfg(test)]
mod label_tests {
    use super::*;
    use crate::backend::MemoryBackend;

    /// A memory backend that records the labels it is given.
    #[derive(Default)]
    struct Labelling {
        inner: MemoryBackend,
        labels: Mutex<Vec<(SecretKey, String)>>,
    }

    impl SecretBackend for Labelling {
        fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
            self.inner.get(key)
        }
        fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
            self.inner.put(key, value)
        }
        fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
            self.inner.delete(key)
        }
        fn name(&self) -> &'static str {
            "labelling"
        }
        fn set_label(&self, key: &SecretKey, label: &str) -> Result<(), SecretError> {
            self.labels.lock().unwrap().push((*key, label.to_string()));
            Ok(())
        }
    }

    #[test]
    fn set_label_reaches_the_backend_and_is_a_no_op_without_one() {
        let backend = Arc::new(Labelling::default());
        let store = SecretStore::new(Some(backend.clone()));
        let key = SecretKey {
            profile: ProfileId::new(),
            slot: SecretSlot::DbPassword,
        };
        store.set_label(&key, "orders-prod").unwrap();
        assert_eq!(
            *backend.labels.lock().unwrap(),
            vec![(key, "orders-prod".to_string())]
        );
        assert_eq!(SecretStore::new(None).set_label(&key, "x"), Ok(()));
    }
}
