//! An in-process backend for tests and `HACIVAT_SECRETS=memory` (HV-SEC-05). Working since
//! Task 1.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use secrecy::{ExposeSecret, SecretString};

use super::{SecretBackend, SecretError};
use crate::store::SecretKey;

/// Secrets in a process map, with an optional size limit and a call counter (HV-SEC-05, P4).
pub struct MemoryBackend {
    map: Mutex<HashMap<SecretKey, SecretString>>,
    max_blob: Option<usize>,
    calls: AtomicU64,
}

impl MemoryBackend {
    pub fn new() -> Self {
        MemoryBackend {
            map: Mutex::new(HashMap::new()),
            max_blob: None,
            calls: AtomicU64::new(0),
        }
    }

    /// A backend whose `put` fails with `TooLarge { limit }` above `limit` bytes.
    pub fn with_max_blob(limit: usize) -> Self {
        MemoryBackend {
            max_blob: Some(limit),
            ..Self::new()
        }
    }

    /// How many `get`, `put` and `delete` calls it served.
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<SecretKey, SecretString>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for MemoryBackend {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        Ok(self.map().get(key).cloned())
    }

    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        let mut map = self.map();
        if let Some(limit) = self.max_blob
            && value.expose_secret().len() > limit
        {
            return Err(SecretError::TooLarge { limit });
        }
        map.insert(*key, value.clone());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.map().remove(key);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "memory"
    }

    // ## m2b-t01-interfaces
    fn choice(&self) -> Option<super::BackendChoice> {
        Some(super::BackendChoice::Memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SecretSlot;
    use hacivat_proto::ProfileId;

    fn key() -> SecretKey {
        SecretKey {
            profile: ProfileId::new(),
            slot: SecretSlot::DbPassword,
        }
    }

    #[test]
    fn put_get_delete_and_count() {
        let b = MemoryBackend::new();
        let k = key();
        assert!(b.get(&k).unwrap().is_none());
        b.put(&k, &SecretString::from("hv_test_pw")).unwrap();
        let got = b.get(&k).unwrap().unwrap();
        assert_eq!(got.expose_secret(), "hv_test_pw");
        b.delete(&k).unwrap();
        assert!(b.get(&k).unwrap().is_none());
        assert_eq!(b.calls(), 5);
        assert_eq!(b.name(), "memory");
    }

    #[test]
    fn a_blob_over_the_limit_is_refused() {
        let b = MemoryBackend::with_max_blob(4);
        let k = key();
        assert_eq!(
            b.put(&k, &SecretString::from("12345")),
            Err(SecretError::TooLarge { limit: 4 })
        );
        assert!(b.put(&k, &SecretString::from("1234")).is_ok());
    }
}
