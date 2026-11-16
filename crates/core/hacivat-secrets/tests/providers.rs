//! Auth provider resolution order (HV-SEC-04). Credential files are temp fixtures; the user's
//! own `~/.pgpass` is never read. Secret comparisons never print either value.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use hacivat_secrets::{
    AuthError, AuthMaterial, AuthOutcome, AuthProviderId, AuthRequest, CredentialFiles,
    ExposeSecret, MemoryBackend, Retention, SecretBackend, SecretError, SecretKey, SecretSlot,
    SecretSource, SecretStore, SecretString, resolve_auth,
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

thread_local! {
    /// Keeps each fixture's directory alive until the test thread ends.
    static DIRS: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

/// A fresh temp file holding `contents`, with permission bits `mode` on Unix.
fn file(mode: u32, contents: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("pgpass");
    std::fs::write(&path, contents).expect("the fixture is written");
    set_mode(&path, mode);
    DIRS.with(|d| d.borrow_mut().push(dir));
    path
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

fn sec(s: &str) -> SecretString {
    SecretString::from(s)
}

/// A store over a fresh memory backend, and that backend.
fn mem_store() -> (SecretStore, Arc<MemoryBackend>) {
    let b = Arc::new(MemoryBackend::new());
    (SecretStore::new(Some(b.clone())), b)
}

/// The key every request below uses.
fn k() -> SecretKey {
    SecretKey {
        profile: "0b0f5a3c-2f7e-4a54-9d3b-6c1e8f2a7d10"
            .parse()
            .expect("a valid UUID"),
        slot: SecretSlot::DbPassword,
    }
}

/// `deploy@h:5432/d` for `k()`'s profile, retention Forever.
fn req(provider: AuthProviderId, draft: Option<&SecretString>) -> AuthRequest<'_> {
    AuthRequest {
        profile: k().profile,
        provider,
        engine: DbmsKind::Postgres,
        host: "h",
        port: 5432,
        database: Some("d"),
        user: Some("deploy"),
        retention: Retention::Forever,
        draft,
    }
}

/// No credential files.
fn files() -> CredentialFiles {
    CredentialFiles::default()
}

fn files_with_pgpass(contents: &str) -> CredentialFiles {
    CredentialFiles {
        pgpass: Some(file(0o600, contents)),
        pg_service: None,
    }
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
fn resolution_order() {
    let (store, _) = mem_store();
    store.put(&k(), sec("stored"), Retention::Forever).unwrap();
    assert!(matches!(
        resolve_auth(
            &req(AuthProviderId::Password, Some(&sec("draft"))),
            &store,
            &files()
        ),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::Draft,
            ..
        })
    ));
    assert!(matches!(
        resolve_auth(&req(AuthProviderId::Password, None), &store, &files()),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::Store(Retention::Forever),
            ..
        })
    ));
    let (empty, _) = mem_store();
    assert!(matches!(
        resolve_auth(&req(AuthProviderId::Password, None), &empty, &files()),
        AuthOutcome::NeedsPrompt { ref user } if user == "deploy"
    ));
    assert!(matches!(
        resolve_auth(
            &req(AuthProviderId::PgPass, None),
            &empty,
            &files_with_pgpass("h:5432:d:deploy:pp\n")
        ),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::PgPass { line: 1, .. },
            ..
        })
    ));
    assert!(matches!(
        resolve_auth(
            &req(AuthProviderId::PgPass, None),
            &empty,
            &files_with_pgpass("other:5432:d:deploy:pp\n")
        ),
        AuthOutcome::Failed(AuthError::PgPassNoMatch)
    ));
    assert!(matches!(
        resolve_auth(&req(AuthProviderId::None, None), &empty, &files()),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::NotNeeded,
            password: None,
            ..
        })
    ));
}

#[test]
fn ready_material_carries_the_user_and_the_password() {
    let (store, _) = mem_store();
    store.put(&k(), sec("stored"), Retention::Forever).unwrap();
    let AuthOutcome::Ready(m) = resolve_auth(
        &req(AuthProviderId::Password, Some(&sec("draft"))),
        &store,
        &files(),
    ) else {
        panic!("expected Ready");
    };
    assert_eq!(m.user.as_deref(), Some("deploy"));
    assert_secret!(m.password.as_ref().unwrap(), "draft");

    let AuthOutcome::Ready(m) =
        resolve_auth(&req(AuthProviderId::Password, None), &store, &files())
    else {
        panic!("expected Ready");
    };
    assert_eq!(m.user.as_deref(), Some("deploy"));
    assert_secret!(m.password.as_ref().unwrap(), "stored");

    let files = files_with_pgpass("h:5432:d:deploy:pp\n");
    let AuthOutcome::Ready(m) = resolve_auth(&req(AuthProviderId::PgPass, None), &store, &files)
    else {
        panic!("expected Ready");
    };
    assert_eq!(m.user.as_deref(), Some("deploy"));
    assert_secret!(m.password.as_ref().unwrap(), "pp");
    assert_eq!(
        m.source,
        SecretSource::PgPass {
            path: files.pgpass.clone().unwrap(),
            line: 1
        }
    );
}

#[test]
fn the_store_is_read_under_the_profile_retention() {
    let (store, _) = mem_store();
    store
        .put(&k(), sec("session"), Retention::ForSession)
        .unwrap();
    let mut r = req(AuthProviderId::Password, None);
    assert!(matches!(
        resolve_auth(&r, &store, &files()),
        AuthOutcome::NeedsPrompt { .. }
    ));
    r.retention = Retention::ForSession;
    assert!(matches!(
        resolve_auth(&r, &store, &files()),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::Store(Retention::ForSession),
            ..
        })
    ));
    r.retention = Retention::Never;
    assert!(matches!(
        resolve_auth(&r, &store, &files()),
        AuthOutcome::NeedsPrompt { .. }
    ));
}

#[test]
fn a_failure_is_never_retried_with_another_source() {
    // A store error is not turned into a prompt.
    let failing = SecretStore::new(Some(Arc::new(Failing(SecretError::Denied))));
    assert!(matches!(
        resolve_auth(&req(AuthProviderId::Password, None), &failing, &files()),
        AuthOutcome::Failed(AuthError::Store(SecretError::Denied))
    ));
    // A pgpass miss does not fall back to a stored password or a prompt.
    let (store, _) = mem_store();
    store.put(&k(), sec("stored"), Retention::Forever).unwrap();
    assert!(matches!(
        resolve_auth(
            &req(AuthProviderId::PgPass, None),
            &store,
            &files_with_pgpass("other:5432:d:deploy:pp\n")
        ),
        AuthOutcome::Failed(AuthError::PgPassNoMatch)
    ));
    // No pgpass file at all is a miss too.
    assert!(matches!(
        resolve_auth(&req(AuthProviderId::PgPass, None), &store, &files()),
        AuthOutcome::Failed(AuthError::PgPassNoMatch)
    ));
}

#[cfg(unix)]
#[test]
fn a_readable_pgpass_fails_with_its_warning() {
    use hacivat_secrets::PgPassWarning;

    let path = file(0o644, "h:5432:d:deploy:pp\n");
    let files = CredentialFiles {
        pgpass: Some(path.clone()),
        pg_service: None,
    };
    let (store, _) = mem_store();
    match resolve_auth(&req(AuthProviderId::PgPass, None), &store, &files) {
        AuthOutcome::Failed(AuthError::PgPassFile(w)) => {
            assert_eq!(w, PgPassWarning::Permissions { path, mode: 0o644 });
        }
        other => panic!("expected a pgpass warning, got {other:?}"),
    }
}

#[test]
fn the_draft_comes_before_pgpass() {
    let (store, _) = mem_store();
    assert!(matches!(
        resolve_auth(
            &req(AuthProviderId::PgPass, Some(&sec("draft"))),
            &store,
            &files_with_pgpass("h:5432:d:deploy:pp\n")
        ),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::Draft,
            ..
        })
    ));
}

#[test]
fn pgpass_without_a_database_uses_the_user_name() {
    let (store, _) = mem_store();
    let mut r = req(AuthProviderId::PgPass, None);
    r.database = None;
    assert!(matches!(
        resolve_auth(&r, &store, &files_with_pgpass("h:5432:deploy:deploy:pp\n")),
        AuthOutcome::Ready(AuthMaterial {
            source: SecretSource::PgPass { line: 1, .. },
            ..
        })
    ));
}

#[test]
fn no_password_providers_ignore_the_draft_and_the_store() {
    let (store, b) = mem_store();
    store.put(&k(), sec("stored"), Retention::Forever).unwrap();
    let before = b.calls();
    for provider in [AuthProviderId::None, AuthProviderId::ClientCert] {
        let AuthOutcome::Ready(m) =
            resolve_auth(&req(provider, Some(&sec("draft"))), &store, &files())
        else {
            panic!("expected Ready");
        };
        assert_eq!(m.source, SecretSource::NotNeeded);
        assert!(m.password.is_none());
        assert_eq!(m.user.as_deref(), Some("deploy"));
    }
    assert_eq!(b.calls(), before, "the store was not consulted");
}

#[test]
fn an_outcome_never_debug_prints_a_password() {
    let (store, _) = mem_store();
    let shown = format!(
        "{:?}",
        resolve_auth(
            &req(AuthProviderId::PgPass, None),
            &store,
            &files_with_pgpass("h:5432:d:deploy:hv_test_pw\n")
        )
    );
    assert!(shown.contains("PgPass"));
    assert!(!shown.contains("hv_test_pw"), "the password was printed");
}
