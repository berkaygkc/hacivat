//! The keyring-or-vault fallback backend and the store over it (HV-SEC-05, D-016, Q23, Q38).
//! The keyring is a memory backend; vaults are temp fixtures with fast Argon2 parameters.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hacivat_secrets::{
    BackendChoice, ExposeSecret, FallbackBackend, FileVault, KeyringBackend, MemoryBackend,
    Retention, SecretBackend, SecretError, SecretKey, SecretSlot, SecretStore, SecretString,
    Stored, VaultBackend, VaultParams,
};

thread_local! {
    /// Keeps each fixture's directory alive until the test thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

fn sec(s: &str) -> SecretString {
    SecretString::from(s)
}

fn key() -> SecretKey {
    SecretKey {
        profile: "0b0f5a3c-2f7e-4a54-9d3b-6c1e8f2a7d10"
            .parse()
            .expect("a valid UUID"),
        slot: SecretSlot::DbPassword,
    }
}

fn vault_fast_tmp() -> FileVault {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("vault.bin");
    DIRS.with(|d| d.borrow_mut().push(dir));
    FileVault::with_params(
        path,
        VaultParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    )
}

/// Counts how often the probe ran.
#[derive(Clone, Default)]
struct Counter(Arc<AtomicUsize>);

impl Counter {
    fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

fn counter() -> Counter {
    Counter::default()
}

fn fallback_counting(
    keyring_usable: bool,
    probes: &Counter,
) -> (FallbackBackend, Arc<MemoryBackend>, Arc<FileVault>) {
    let keyring = Arc::new(MemoryBackend::new());
    let vault = Arc::new(vault_fast_tmp());
    let probes = probes.clone();
    let fb = FallbackBackend::new(
        keyring.clone(),
        vault.clone(),
        Box::new(move || {
            probes.0.fetch_add(1, Ordering::SeqCst);
            keyring_usable
        }),
    );
    (fb, keyring, vault)
}

fn fallback(keyring_usable: bool) -> (FallbackBackend, Arc<MemoryBackend>, Arc<FileVault>) {
    fallback_counting(keyring_usable, &counter())
}

/// A store over the vault (probe false), whose vault exists but is locked.
fn store_with_locked_vault() -> (SecretStore, Arc<FileVault>) {
    let (fb, _, vault) = fallback(false);
    vault.create(sec("correct horse")).unwrap();
    vault.lock();
    (SecretStore::new(Some(Arc::new(fb))), vault)
}

#[test]
fn fallback_uses_keyring_when_probe_true() {
    let (fb, kr, vault) = fallback(true);
    fb.put(&key(), &sec("x")).unwrap();
    assert_eq!(kr.calls(), 1);
    assert_eq!(fb.choice(), Some(BackendChoice::Keyring));
    assert!(fb.as_vault().is_none());
    assert!(!vault.exists(), "the vault is never touched");
    assert!(fb.probe_available());
}

#[test]
fn fallback_uses_vault_when_probe_false_and_probes_once() {
    let probes = counter();
    let (fb, kr, vault) = fallback_counting(false, &probes);
    assert_eq!(fb.choice(), None, "not probed yet");
    assert_eq!(probes.get(), 0);
    vault.create(sec("correct horse")).unwrap();
    fb.put(&key(), &sec("x")).unwrap();
    assert!(fb.get(&key()).unwrap().unwrap().expose_secret() == "x");
    assert!(fb.probe_available());
    assert_eq!(probes.get(), 1);
    assert_eq!(fb.choice(), Some(BackendChoice::Vault));
    assert!(fb.as_vault().is_some_and(|v| v.is_unlocked()));
    assert_eq!(fb.name(), "vault");
    assert_eq!(kr.calls(), 0, "the keyring is never touched");
}

#[test]
#[ignore = "ci-linux"]
fn linux_without_secret_service_uses_vault() {
    // run by CI `vault-linux` (Task 16): no D-Bus session
    let fb = FallbackBackend::new(
        Arc::new(KeyringBackend::new(None)),
        Arc::new(vault_fast_tmp()),
        Box::new(|| KeyringBackend::new(None).probe_available()),
    );
    let _ = fb.get(&key()); // the first call runs the probe
    fb.as_vault()
        .expect("the vault is chosen")
        .create(sec("correct horse"))
        .unwrap();
    fb.put(&key(), &sec("x")).unwrap();
    assert_eq!(fb.choice(), Some(BackendChoice::Vault));
}

#[test]
fn store_reports_backend_choice() {
    let s = SecretStore::new(Some(Arc::new(fallback(false).0)));
    assert_eq!(s.backend_choice(), None);
    let _ = s.keyring_available();
    assert_eq!(s.backend_choice(), Some(BackendChoice::Vault));
    assert!(s.vault().is_some());
    // Q23: the vault keeps Forever enabled
    assert!(s.keyring_available());

    let s = SecretStore::new(Some(Arc::new(fallback(true).0)));
    assert!(s.keyring_available());
    assert_eq!(s.backend_choice(), Some(BackendChoice::Keyring));
    assert!(s.vault().is_none());
}

#[test]
fn locked_vault_forever_put_falls_back_to_until_restart() {
    // Q38
    let (s, _) = store_with_locked_vault();
    assert!(matches!(
        s.put(&key(), sec("x"), Retention::Forever),
        Ok(Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::VaultLocked
        })
    ));
    let kept = s.get(&key(), Retention::Forever).unwrap().unwrap();
    assert!(kept.expose_secret() == "x");
}

#[test]
fn no_vault_yet_forever_put_falls_back_too() {
    // Q38: "Not now" on Create vault keeps the secret until restart
    let (fb, _, vault) = fallback(false);
    let s = SecretStore::new(Some(Arc::new(fb)));
    assert!(s.get(&key(), Retention::Forever).unwrap().is_none());
    assert!(matches!(
        s.put(&key(), sec("x"), Retention::Forever),
        Ok(Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::VaultLocked
        })
    ));
    assert!(!vault.exists());
}

/// Q38: every vault error on a Forever put falls back (a read-only vault was written by a newer
/// Hacivat; a corrupt one cannot be read); any other error stays a hard error (M2a).
#[test]
fn vault_errors_on_forever_put_fall_back_to_until_restart() {
    let s = SecretStore::new(Some(Arc::new(Refusing(SecretError::VaultReadOnly))));
    assert!(matches!(
        s.put(&key(), sec("x"), Retention::Forever),
        Ok(Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::VaultReadOnly
        })
    ));
    let s = SecretStore::new(Some(Arc::new(Refusing(SecretError::VaultCorrupt))));
    assert!(matches!(
        s.put(&key(), sec("x"), Retention::Forever),
        Ok(Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::VaultCorrupt
        })
    ));
    // Other errors stay hard errors (M2a).
    let s = SecretStore::new(Some(Arc::new(Refusing(SecretError::WrongMasterPassword))));
    assert_eq!(
        s.put(&key(), sec("x"), Retention::Forever),
        Err(SecretError::WrongMasterPassword)
    );
}

#[test]
fn unlocked_vault_forever_put_lands_as_requested() {
    let (fb, _, vault) = fallback(false);
    let s = SecretStore::new(Some(Arc::new(fb)));
    vault.create(sec("correct horse")).unwrap();
    assert_eq!(
        s.put(&key(), sec("x"), Retention::Forever),
        Ok(Stored::AsRequested)
    );
    vault.lock();
    vault.unlock(sec("correct horse")).unwrap();
    let got = s.get(&key(), Retention::Forever).unwrap().unwrap();
    assert!(got.expose_secret() == "x");
    s.delete_all(key().profile).unwrap();
    assert!(s.get(&key(), Retention::Forever).unwrap().is_none());
}

/// A backend whose `put` always fails with one error.
struct Refusing(SecretError);

impl SecretBackend for Refusing {
    fn get(&self, _key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        Ok(None)
    }
    fn put(&self, _key: &SecretKey, _value: &SecretString) -> Result<(), SecretError> {
        Err(self.0.clone())
    }
    fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "refusing"
    }
}
