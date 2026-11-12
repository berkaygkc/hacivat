//! The secret store's retention routing, its fallbacks, the keyring service names and the
//! keyring backend's native store (HV-SEC-05, R7, R8). Fixture secrets are obvious test values,
//! and secret comparisons never print either value.

use std::sync::Arc;

use hacivat_proto::ProfileId;
use hacivat_secrets::{
    AuthMaterial, AuthOutcome, ExposeSecret, KeyringBackend, MemoryBackend, PgPassMatch, Retention,
    SecretBackend, SecretError, SecretKey, SecretSlot, SecretSource, SecretStore, SecretString,
    Stored,
};

/// Compares a secret without printing either value on failure.
macro_rules! assert_secret {
    ($got:expr, $want:expr) => {
        assert!(
            $got.expose_secret() == $want,
            "secret mismatch (values not shown)"
        )
    };
}

/// A store over a fresh memory backend, and that backend.
fn store() -> (SecretStore, Arc<MemoryBackend>) {
    let b = Arc::new(MemoryBackend::new());
    (SecretStore::new(Some(b.clone())), b)
}

/// The same key on every call, so a test can refer to it twice.
fn k() -> SecretKey {
    key(
        "0b0f5a3c-2f7e-4a54-9d3b-6c1e8f2a7d10",
        SecretSlot::DbPassword,
    )
}

fn key(uuid: &str, slot: SecretSlot) -> SecretKey {
    SecretKey {
        profile: uuid.parse().expect("a valid UUID"),
        slot,
    }
}

fn sec(s: &str) -> SecretString {
    SecretString::from(s)
}

/// A backend whose every call fails with one error.
struct Failing(SecretError);

impl SecretBackend for Failing {
    fn get(&self, _key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        Err(self.0.clone())
    }
    fn put(&self, _key: &SecretKey, _value: &SecretString) -> Result<(), SecretError> {
        Err(self.0.clone())
    }
    fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Err(self.0.clone())
    }
    fn name(&self) -> &'static str {
        "failing"
    }
}

#[test]
fn forever_uses_backend_only() {
    let (s, b) = store();
    s.put(&k(), sec("s3cret"), Retention::Forever).unwrap();
    assert_secret!(b.get(&k()).unwrap().unwrap(), "s3cret");
    assert!(s.get(&k(), Retention::UntilRestart).unwrap().is_none());
}

#[test]
fn until_restart_never_touches_backend() {
    let (s, b) = store();
    s.put(&k(), sec("x"), Retention::UntilRestart).unwrap();
    assert_eq!(b.calls(), 0);
    assert_secret!(s.get(&k(), Retention::UntilRestart).unwrap().unwrap(), "x");
}

#[test]
fn for_session_is_forgotten() {
    let (s, _) = store();
    s.put(&k(), sec("x"), Retention::ForSession).unwrap();
    s.forget_session(k().profile);
    assert!(s.get(&k(), Retention::ForSession).unwrap().is_none());
}

#[test]
fn never_keeps_nothing() {
    let (s, b) = store();
    assert_eq!(
        s.put(&k(), sec("x"), Retention::Never).unwrap(),
        Stored::AsRequested
    );
    assert_eq!(b.calls(), 0);
    assert!(s.get(&k(), Retention::Never).unwrap().is_none());
}

#[test]
fn lowering_retention_deletes_the_keyring_entry() {
    let (s, b) = store();
    s.put(&k(), sec("x"), Retention::Forever).unwrap();
    s.change_retention(&k(), Retention::Forever, Retention::ForSession)
        .unwrap();
    assert!(b.get(&k()).unwrap().is_none());
    assert_secret!(s.get(&k(), Retention::ForSession).unwrap().unwrap(), "x");
}

#[test]
fn delete_all_removes_every_slot() {
    let (s, b) = store();
    let p = k().profile;
    let (k_db, k_tls) = (
        SecretKey {
            profile: p,
            slot: SecretSlot::DbPassword,
        },
        SecretKey {
            profile: p,
            slot: SecretSlot::TlsKeyPassphrase,
        },
    );
    s.put(&k_db, sec("a"), Retention::Forever).unwrap();
    s.put(&k_tls, sec("b"), Retention::Forever).unwrap();
    s.put(&k_db, sec("c"), Retention::ForSession).unwrap();
    s.delete_all(p).unwrap();
    assert!(b.get(&k_db).unwrap().is_none());
    assert!(b.get(&k_tls).unwrap().is_none());
    assert!(s.get(&k_db, Retention::ForSession).unwrap().is_none());
}

#[test]
fn copy_all_duplicates_to_new_profile() {
    let (s, b) = store();
    let (p1, p2) = (ProfileId::new(), ProfileId::new());
    let (k1, k2) = (
        SecretKey {
            profile: p1,
            slot: SecretSlot::DbPassword,
        },
        SecretKey {
            profile: p2,
            slot: SecretSlot::DbPassword,
        },
    );
    s.put(&k1, sec("x"), Retention::Forever).unwrap();
    s.copy_all(p1, p2, Retention::Forever).unwrap();
    assert_secret!(b.get(&k2).unwrap().unwrap(), "x");
    assert_secret!(b.get(&k1).unwrap().unwrap(), "x");
}

#[test]
fn keyring_denied_falls_back_to_session_with_reason() {
    let s = SecretStore::new(Some(Arc::new(Failing(SecretError::Denied))));
    assert_eq!(
        s.put(&k(), sec("x"), Retention::Forever).unwrap(),
        Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::Denied
        }
    );
    assert_secret!(s.get(&k(), Retention::Forever).unwrap().unwrap(), "x");
}

#[test]
fn windows_blob_limit_falls_back() {
    let s = SecretStore::new(Some(Arc::new(MemoryBackend::with_max_blob(2560))));
    assert!(matches!(
        s.put(&k(), sec(&"a".repeat(2561)), Retention::Forever)
            .unwrap(),
        Stored::FellBackTo {
            reason: SecretError::TooLarge { limit: 2560 },
            ..
        }
    ));
}

#[test]
fn raising_retention_moves_the_secret_into_the_backend() {
    let (s, b) = store();
    s.put(&k(), sec("x"), Retention::UntilRestart).unwrap();
    assert_eq!(
        s.change_retention(&k(), Retention::UntilRestart, Retention::Forever)
            .unwrap(),
        Stored::AsRequested
    );
    assert_secret!(b.get(&k()).unwrap().unwrap(), "x");
    assert!(s.get(&k(), Retention::UntilRestart).unwrap().is_none());
}

#[test]
fn a_refused_raise_keeps_the_secret_in_the_process() {
    let s = SecretStore::new(Some(Arc::new(Failing(SecretError::Unavailable))));
    s.put(&k(), sec("x"), Retention::ForSession).unwrap();
    assert_eq!(
        s.change_retention(&k(), Retention::ForSession, Retention::Forever)
            .unwrap(),
        Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::Unavailable
        }
    );
    assert!(s.get(&k(), Retention::ForSession).unwrap().is_none());
    assert_secret!(s.get(&k(), Retention::Forever).unwrap().unwrap(), "x");
}

#[test]
fn no_backend_falls_back_and_other_errors_are_returned() {
    let none = SecretStore::new(None);
    assert_eq!(
        none.put(&k(), sec("x"), Retention::Forever).unwrap(),
        Stored::FellBackTo {
            retention: Retention::UntilRestart,
            reason: SecretError::Unavailable
        }
    );
    assert!(!none.keyring_available());
    let broken = SecretStore::new(Some(Arc::new(Failing(SecretError::Backend("io".into())))));
    assert_eq!(
        broken.put(&k(), sec("x"), Retention::Forever),
        Err(SecretError::Backend("io".into()))
    );
    assert!(broken.get(&k(), Retention::Forever).is_err());
}

#[test]
fn a_successful_forever_put_replaces_an_earlier_fallback() {
    let b = Arc::new(MemoryBackend::with_max_blob(4));
    let s = SecretStore::new(Some(b.clone()));
    assert!(matches!(
        s.put(&k(), sec("too-long"), Retention::Forever).unwrap(),
        Stored::FellBackTo { .. }
    ));
    assert_eq!(
        s.put(&k(), sec("ok"), Retention::Forever).unwrap(),
        Stored::AsRequested
    );
    assert_secret!(s.get(&k(), Retention::Forever).unwrap().unwrap(), "ok");
    assert!(s.get(&k(), Retention::UntilRestart).unwrap().is_none());
    assert!(s.keyring_available(), "a memory backend is always usable");
}

#[test]
fn service_names() {
    assert_eq!(
        key(
            "6f9619ff-8b86-d011-b42d-00c04fc964ff",
            SecretSlot::DbPassword
        )
        .service(),
        if cfg!(debug_assertions) {
            "hacivat-dev/6f9619ff-8b86-d011-b42d-00c04fc964ff/db-password"
        } else {
            "hacivat/6f9619ff-8b86-d011-b42d-00c04fc964ff/db-password"
        }
    );
}

#[test]
fn secrets_never_debug_print() {
    // our types, not secrecy itself (P33); ProfileDraft, ParseOutcome and FakeRequest are
    // covered by Task 1's debug_redaction test
    let m = || AuthMaterial {
        user: Some("u".into()),
        password: Some(sec("s3cret")),
        source: SecretSource::Draft,
    };
    let pg = PgPassMatch {
        line: 1,
        user: None,
        password: sec("s3cret"),
    };
    for s in [
        format!("{:?}", m()),
        format!("{:?}", AuthOutcome::Ready(m())),
        format!("{pg:?}"),
    ] {
        assert!(!s.contains("s3cret"), "{s}");
    }
}

#[test]
fn keyring_backend_is_native_not_mock() {
    // fails if keyring was built without its native store feature (ledger ruling)
    assert!(
        KeyringBackend::persistent(),
        "keyring's default store is the non-persistent mock: check the apple-native / windows-native / async-secret-service features"
    );
    assert!(matches!(
        keyring::default::default_credential_builder().persistence(),
        keyring::credential::CredentialPersistence::UntilDelete
    ));
}

/// Deletes the live test's entry even when an assertion fails first.
struct DeleteOnDrop<'a>(&'a KeyringBackend, SecretKey);

impl Drop for DeleteOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.delete(&self.1);
    }
}

#[test]
#[ignore = "keyring-live"]
fn live_keyring_round_trip() {
    // HACIVAT_KEYRING_LIVE=1; dev-signed binary on macOS. The service is a fresh
    // `hacivat-dev/<random uuid>/db-password`, deleted at the end even on failure.
    if std::env::var("HACIVAT_KEYRING_LIVE").as_deref() != Ok("1") {
        eprintln!("live_keyring_round_trip skipped: set HACIVAT_KEYRING_LIVE=1");
        return;
    }
    let b = KeyringBackend::new(None);
    let key = SecretKey {
        profile: ProfileId::new(),
        slot: SecretSlot::DbPassword,
    };
    let _cleanup = DeleteOnDrop(&b, key);
    b.put(&key, &sec("hv-live")).unwrap();
    assert_secret!(b.get(&key).unwrap().unwrap(), "hv-live");
    b.delete(&key).unwrap();
    assert!(b.get(&key).unwrap().is_none());
}
