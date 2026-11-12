//! The connections model (Task 12): profile CRUD through the one ordered writer, secrets by
//! retention on the core runtime, store diagnostics at boot (HV-SEC-05, R8, P29, P39, P48).
//! Shells come from `ConnectionsBoot::for_tests` with a FakeDriver registry and a memory (or
//! test) secret backend.

mod support;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Task, TestAppContext, VisualTestContext};
use hacivat_app::connections::{ConnectionsBoot, ConnectionsModel, SaveError};
use hacivat_driver::DriverRegistry;
use hacivat_proto::{EnvTag, Origin, ProfileId, TagSource};
use hacivat_secrets::{
    ExposeSecret, MemoryBackend, Retention, SecretBackend, SecretError, SecretKey, SecretSlot,
    SecretString,
};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_settings::LocalPaths;
use support::*;

// ---- helpers ---------------------------------------------------------------------------------

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn backend(vcx: &VisualTestContext) -> Arc<dyn SecretBackend> {
    secret_backend(vcx)
}

fn db_key_of(profile: ProfileId) -> SecretKey {
    SecretKey {
        profile,
        slot: SecretSlot::DbPassword,
    }
}

/// The password key of the shell's only profile.
fn db_key(vcx: &VisualTestContext) -> SecretKey {
    let ids = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>()
    });
    assert_eq!(ids.len(), 1, "one profile expected");
    db_key_of(ids[0])
}

fn profile_names(vcx: &VisualTestContext) -> Vec<String> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .iter()
            .map(|p| p.name.clone())
            .collect()
    })
}

fn named_profile(name: &str) -> Profile {
    Profile {
        name: name.into(),
        endpoint: ProfileEndpoint::Server {
            host: "db.acme.io".into(),
            port: None,
        },
        ..Profile::default()
    }
}

fn named_draft(name: &str) -> ProfileDraft {
    let mut d = ProfileDraft::new();
    d.profile = named_profile(name);
    d
}

/// A draft with a password, built from a fixture value (secrets are not `Clone`).
fn named_draft_with_password(name: &str, password: &str, retention: Retention) -> ProfileDraft {
    let mut d = named_draft(name);
    d.profile.retention = retention;
    d.password = Some(SecretString::from(password));
    d
}

fn draft_with_password(password: &str, retention: Retention) -> ProfileDraft {
    named_draft_with_password("orders", password, retention)
}

/// An edit of `id` renamed to `name`.
fn edit_named(id: ProfileId, name: &str) -> ProfileDraft {
    ProfileDraft::edit(Profile {
        id,
        ..named_profile(name)
    })
}

/// Waits (in real time, P22) until `task` is done and returns its output.
fn wait<T: 'static>(vcx: &VisualTestContext, task: Task<T>) -> T {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    vcx.cx.update(|cx| {
        cx.spawn(async move |_| {
            *slot.borrow_mut() = Some(task.await);
        })
        .detach();
    });
    for _ in 0..400 {
        if let Some(v) = out.borrow_mut().take() {
            return v;
        }
        settle(vcx, ms(5));
    }
    panic!("the task did not finish in 2 s");
}

fn try_save(vcx: &VisualTestContext, draft: ProfileDraft) -> Result<ProfileId, SaveError> {
    let task = vcx.cx.update(|cx| ConnectionsModel::save(cx, draft));
    wait(vcx, task)
}

fn save(vcx: &VisualTestContext, draft: ProfileDraft) -> ProfileId {
    try_save(vcx, draft).expect("the profile saves")
}

fn save_no_wait(vcx: &VisualTestContext, draft: ProfileDraft) {
    vcx.cx
        .update(|cx| ConnectionsModel::save(cx, draft).detach());
}

fn delete(vcx: &VisualTestContext, id: ProfileId) {
    let task = vcx.cx.update(|cx| ConnectionsModel::delete(cx, id));
    wait(vcx, task).expect("the profile is deleted");
}

fn duplicate(vcx: &VisualTestContext, id: ProfileId) -> ProfileId {
    let task = vcx.cx.update(|cx| ConnectionsModel::duplicate(cx, id));
    wait(vcx, task).expect("the profile is duplicated")
}

/// The thread names a backend's calls ran on.
type Names = Arc<Mutex<Vec<Option<String>>>>;

/// A memory backend that records the thread of every call.
struct RecordingBackend {
    inner: MemoryBackend,
    names: Names,
}

impl RecordingBackend {
    fn record(&self) {
        let name = std::thread::current().name().map(str::to_string);
        self.names.lock().unwrap().push(name);
    }
}

impl SecretBackend for RecordingBackend {
    fn get(&self, key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        self.record();
        self.inner.get(key)
    }
    fn put(&self, key: &SecretKey, value: &SecretString) -> Result<(), SecretError> {
        self.record();
        self.inner.put(key, value)
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.record();
        self.inner.delete(key)
    }
    fn name(&self) -> &'static str {
        "recording"
    }
    fn set_label(&self, _key: &SecretKey, _label: &str) -> Result<(), SecretError> {
        self.record();
        Ok(())
    }
}

fn shell_with_recording_backend(
    cx: &mut TestAppContext,
    lang: &str,
) -> (&'static mut VisualTestContext, Names) {
    let names = Names::default();
    let backend = Arc::new(RecordingBackend {
        inner: MemoryBackend::new(),
        names: names.clone(),
    });
    (shell_with_backend(cx, lang, None, backend), names)
}

/// A backend whose every `put` fails with one error.
struct FailingBackend(SecretError);

impl SecretBackend for FailingBackend {
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
        "failing"
    }
}

fn shell_with_failing_backend(
    cx: &mut TestAppContext,
    lang: &str,
    error: SecretError,
) -> &'static mut VisualTestContext {
    shell_with_backend(cx, lang, None, Arc::new(FailingBackend(error)))
}

/// Asserts a stored secret without ever printing it.
fn assert_secret(value: Option<SecretString>, expected: &str) {
    let value = value.expect("a secret is stored");
    assert!(
        value.expose_secret() == expected,
        "secret mismatch (values not shown)"
    );
}

// ---- tests -----------------------------------------------------------------------------------

#[gpui::test]
fn connections_toml_never_contains_a_secret(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    save(vcx, draft_with_password("s3cret-pw", Retention::Forever));
    let text = read(&paths(vcx).connections_toml());
    assert!(!text.contains("s3cret-pw"));
    assert!(!text.contains("password"));
    assert_secret(backend(vcx).get(&db_key(vcx)).unwrap(), "s3cret-pw");
}

#[gpui::test]
fn delete_removes_profile_and_every_slot(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = save(
        vcx,
        named_draft_with_password("a", "pw", Retention::Forever),
    );
    delete(vcx, id);
    settle(vcx, ms(50));
    assert!(profile_names(vcx).is_empty());
    assert!(backend(vcx).get(&db_key_of(id)).unwrap().is_none());
    assert!(!read(&paths(vcx).connections_toml()).contains(&id.to_string()));
}

#[gpui::test]
fn duplicate_copies_secret_and_names_dash_2(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = save(
        vcx,
        named_draft_with_password("a", "pw", Retention::Forever),
    );
    let b = duplicate(vcx, a);
    settle(vcx, ms(50));
    assert_eq!(profile_names(vcx), ["a", "a-2"]);
    for id in [a, b] {
        assert_secret(backend(vcx).get(&db_key_of(id)).unwrap(), "pw");
    }
    assert_eq!(toast_texts(vcx), ["Duplicated as a-2"]);
}

#[gpui::test]
fn save_on_read_only_store_fails_without_writing(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", "version = 2\n");
    assert!(matches!(
        try_save(vcx, named_draft("a")),
        Err(SaveError::StoreReadOnly)
    ));
    assert_eq!(read(&paths(vcx).connections_toml()), "version = 2\n");
    assert!(profile_names(vcx).is_empty());
}

#[gpui::test]
fn profile_writes_are_ordered_last_wins(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = save(vcx, named_draft("w"));
    save_no_wait(vcx, edit_named(id, "x"));
    save_no_wait(vcx, edit_named(id, "y"));
    settle(vcx, ms(100));
    let text = read(&paths(vcx).connections_toml());
    assert!(
        text.contains("name = \"y\"") && !text.contains("name = \"x\""),
        "{text}"
    );
}

#[gpui::test]
fn secret_ops_never_run_on_the_ui_thread(cx: &mut TestAppContext) {
    let (vcx, names) = shell_with_recording_backend(cx, "en");
    save(vcx, draft_with_password("pw", Retention::Forever));
    settle(vcx, ms(50));
    let names = names.lock().unwrap().clone();
    assert!(!names.is_empty());
    assert!(
        names
            .iter()
            .all(|n| n.as_deref().is_some_and(|n| n.starts_with("hv-core"))),
        "{names:?}"
    );
}

#[gpui::test]
fn keyring_fallback_toast_once_per_session(cx: &mut TestAppContext) {
    // Review Focus 4 (P39)
    let vcx = shell_with_failing_backend(cx, "en", SecretError::Denied);
    save(
        vcx,
        named_draft_with_password("a", "pw", Retention::Forever),
    );
    save(
        vcx,
        named_draft_with_password("b", "pw", Retention::Forever),
    );
    settle(vcx, ms(50));
    assert_eq!(
        toast_ids(vcx)
            .iter()
            .filter(|i| **i == "hv-conn-keyring-unavailable")
            .count(),
        1
    );
    // The fallback keeps the password until quit.
    let id = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().profiles()[0].id);
    let kept = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .secrets()
            .get(&db_key_of(id), Retention::Forever)
    });
    assert_secret(kept.unwrap(), "pw");
}

#[gpui::test]
fn boot_reports_store_diagnostics_once(cx: &mut TestAppContext) {
    // P29
    let vcx = shell_with_connections_file(
        cx,
        "en",
        "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"h\"\npassword = \"typed-pw\"\n[[connection]]\nid = \"7f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"b\"\nengine = \"oracle9\"\nhost = \"h\"\n",
    );
    assert_eq!(
        toast_ids(vcx),
        [
            "hv-conn-file-entries-skipped",
            "hv-conn-file-password-ignored"
        ]
    );
    assert!(toast_texts(vcx).iter().all(|t| !t.contains("typed-pw")));
}

#[gpui::test]
fn boot_reports_a_quarantined_file(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", "[[connection");
    assert_eq!(toast_ids(vcx), ["hv-conn-file-quarantined"]);
    let texts = toast_texts(vcx);
    assert!(texts[0].contains("connections.toml.corrupt-"), "{texts:?}");
}

#[gpui::test]
fn boot_reports_a_newer_file_as_read_only(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", "version = 2\n");
    let read_only = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().store_read_only());
    assert!(read_only);
    assert_eq!(toast_ids(vcx), ["hv-conn-store-read-only"]);
}

#[gpui::test]
fn changing_the_retention_moves_the_secret(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = save(
        vcx,
        named_draft_with_password("a", "pw", Retention::Forever),
    );
    let mut edit = edit_named(id, "a");
    edit.profile.retention = Retention::UntilRestart;
    save(vcx, edit);
    assert!(backend(vcx).get(&db_key_of(id)).unwrap().is_none());
    let kept = vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .secrets()
            .get(&db_key_of(id), Retention::UntilRestart)
    });
    assert_secret(kept.unwrap(), "pw");
    // The new retention is saved; the file still holds no secret.
    let text = read(&paths(vcx).connections_toml());
    assert!(text.contains("retention = \"until_restart\""), "{text}");
    assert!(!text.contains("pw\""), "{text}");
}

#[gpui::test]
fn a_tls_key_passphrase_is_stored_in_its_own_slot(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let mut d = named_draft("a");
    d.tls_key_passphrase = Some(SecretString::from("k3y-pass"));
    let id = save(vcx, d);
    let key = SecretKey {
        profile: id,
        slot: SecretSlot::TlsKeyPassphrase,
    };
    assert_secret(backend(vcx).get(&key).unwrap(), "k3y-pass");
    assert!(backend(vcx).get(&db_key_of(id)).unwrap().is_none());
    assert!(!read(&paths(vcx).connections_toml()).contains("k3y-pass"));
}

#[gpui::test]
fn saved_profiles_load_back_in_order(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let a = save(vcx, named_draft("a"));
    save(vcx, named_draft("b"));
    vcx.cx.update(|cx| ConnectionsModel::select(cx, Some(a)));
    let selected = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected());
    assert_eq!(selected, Some(a));
    let paths = paths(vcx);
    let boot = ConnectionsBoot::for_tests(&paths, DriverRegistry::new());
    let names: Vec<String> = boot
        .loaded
        .profiles
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["a", "b"]);
}

#[cfg(unix)]
#[test]
fn private_dir_is_0700_under_the_local_data() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(root.path());
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

    let boot = ConnectionsBoot::for_tests(&paths, DriverRegistry::new());
    assert_ne!(boot.private_dir, std::env::temp_dir());
    assert!(boot.private_dir.ends_with("fwd"), "{:?}", boot.private_dir);
    assert!(
        boot.private_dir.starts_with(&paths.data) || boot.private_dir.starts_with(&paths.cache),
        "{:?}",
        boot.private_dir
    );
    assert_eq!(mode(&boot.private_dir), 0o700);

    // An existing directory with a looser mode is tightened.
    std::fs::set_permissions(&boot.private_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let again = ConnectionsBoot::for_tests(&paths, DriverRegistry::new());
    assert_eq!(mode(&again.private_dir), 0o700);
    again.runtime.shutdown(Duration::from_millis(100));
    boot.runtime.shutdown(Duration::from_millis(100));

    // A data path too long for a socket path falls back to the cache directory.
    let long = LocalPaths {
        data: root.path().join("d".repeat(90)),
        ..LocalPaths::under(root.path())
    };
    let dir = hacivat_app::connections::boot::private_dir_for(&long);
    assert_eq!(dir, long.cache.join("fwd"));
    assert_eq!(mode(&dir), 0o700);
}

#[test]
fn the_secret_backend_follows_hacivat_secrets() {
    use hacivat_app::connections::boot::secret_backend_from;
    assert_eq!(secret_backend_from(Some("memory")).name(), "memory");
    assert_eq!(secret_backend_from(Some("MEMORY")).name(), "memory");
    assert_eq!(secret_backend_from(None).name(), "keyring");
    assert_eq!(secret_backend_from(Some("vault")).name(), "keyring");
}

// ---- fix round 1 -----------------------------------------------------------------------------

const MANAGED: &str = "version = 1\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"managed\"\nengine = \"postgres\"\nhost = \"db.acme.io\"\nenvironment = \"production\"\nenv_source = \"org\"\norigin = { kind = \"perde_managed\", org = \"acme\" }\n";

fn only_profile(vcx: &VisualTestContext) -> Profile {
    vcx.cx
        .read(|cx| (*cx.global::<ConnectionsModel>().profiles()[0]).clone())
}

fn profiles(vcx: &VisualTestContext) -> Vec<Profile> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .profiles()
            .iter()
            .map(|p| (**p).clone())
            .collect()
    })
}

#[gpui::test]
fn an_org_tag_survives_an_edit_and_a_user_cannot_claim_one(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", MANAGED);
    let managed = only_profile(vcx);
    assert_eq!(managed.env_source, TagSource::Org);

    // The draft tries to lower the organization's tag and to hand it to the user.
    let mut edit = ProfileDraft::edit(managed.clone());
    edit.profile.environment = Some(EnvTag::Dev);
    edit.profile.env_source = TagSource::User;
    edit.profile.name = "renamed".into();
    save(vcx, edit);
    let saved = only_profile(vcx);
    assert_eq!(
        (saved.name.as_str(), saved.environment, saved.env_source),
        ("renamed", Some(EnvTag::Production), TagSource::Org)
    );

    // A user's profile can never claim an organization tag.
    let mut claim = named_draft("mine");
    claim.profile.environment = Some(EnvTag::Production);
    claim.profile.env_source = TagSource::Org;
    let id = save(vcx, claim);
    let mine = profiles(vcx).into_iter().find(|p| p.id == id).unwrap();
    assert_eq!(mine.env_source, TagSource::User);
    assert_eq!(mine.environment, Some(EnvTag::Production));
}

#[gpui::test]
fn a_duplicate_of_a_managed_profile_is_the_users_own(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(cx, "en", MANAGED);
    let original = only_profile(vcx);
    let copy_id = duplicate(vcx, original.id);
    let copy = profiles(vcx).into_iter().find(|p| p.id == copy_id).unwrap();
    assert_eq!(copy.env_source, TagSource::User);
    assert_eq!(copy.origin, Origin::UserTyped);
    assert_eq!(copy.environment, Some(EnvTag::Production));
    assert_eq!(only_profile(vcx).env_source, TagSource::Org);
}

#[gpui::test]
fn an_edit_of_a_deleted_profile_is_refused_and_not_brought_back(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    let id = save(vcx, named_draft("a"));
    let stale = edit_named(id, "a-edited");
    delete(vcx, id);
    assert!(matches!(try_save(vcx, stale), Err(SaveError::NotFound)));
    assert!(profile_names(vcx).is_empty());
    assert!(!read(&paths(vcx).connections_toml()).contains(&id.to_string()));
    let task = vcx.cx.update(|cx| ConnectionsModel::duplicate(cx, id));
    assert!(matches!(wait(vcx, task), Err(SaveError::NotFound)));
}

#[gpui::test]
fn a_file_broken_while_running_is_set_aside_by_the_next_save_with_a_toast(cx: &mut TestAppContext) {
    let vcx = shell_with_connections(cx, "en");
    save(vcx, named_draft("a"));
    let path = paths(vcx).connections_toml();
    std::fs::write(&path, "[[connection").unwrap();
    save(vcx, named_draft("b"));
    assert_eq!(toast_ids(vcx), ["hv-conn-file-quarantined"]);
    let text = read(&path);
    assert!(
        text.contains("name = \"a\"") && text.contains("name = \"b\""),
        "{text}"
    );
}

#[gpui::test]
fn ignored_values_and_outside_passwords_are_reported_at_boot(cx: &mut TestAppContext) {
    let vcx = shell_with_connections_file(
        cx,
        "en",
        "version = 1\npassword = \"typed-pw\"\n[[connection]]\nid = \"6f9619ff-8b86-d011-b42d-00c04fc964ff\"\nname = \"a\"\nengine = \"postgres\"\nhost = \"h\"\nenvironment = \"uat\"\nenv_source = \"org\"\n",
    );
    assert_eq!(
        toast_ids(vcx),
        [
            "hv-conn-file-values-ignored",
            "hv-conn-file-password-ignored"
        ]
    );
    assert_eq!(
        toast_texts(vcx)[0],
        "2 settings in your connections file couldn't be read and were ignored."
    );
    assert!(toast_texts(vcx).iter().all(|t| !t.contains("typed-pw")));
    let p = only_profile(vcx);
    assert_eq!((p.environment, p.env_source), (None, TagSource::User));
}

#[gpui::test]
fn an_unreadable_file_is_reported_left_alone_and_read_only(cx: &mut TestAppContext) {
    // A connections.toml the app cannot read: a directory in its place is an I/O error, not a
    // parse error, so nothing is set aside.
    let vcx = shell_with_connections_prepared(
        cx,
        "en",
        Arc::new(MemoryBackend::new()),
        |paths: &LocalPaths| std::fs::create_dir_all(paths.connections_toml()).unwrap(),
    );
    assert_eq!(toast_ids(vcx), ["hv-conn-file-unreadable"]);
    let read_only = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().store_read_only());
    assert!(read_only);
    assert!(paths(vcx).connections_toml().is_dir());
    assert!(matches!(
        try_save(vcx, named_draft("a")),
        Err(SaveError::StoreReadOnly)
    ));
}
