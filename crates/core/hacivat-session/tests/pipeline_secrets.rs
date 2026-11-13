//! The pipeline's secrets and prompts (Task 14; R8, Q17, Q33, Q38, B13, B25): prompted SSH
//! secrets stored only after the whole connect succeeded, prompts per hop, the vault's unlock
//! and create questions, the session overlay, the TLS key passphrase, and no secret in any log.
//! SSH tests run against the in-process testkit server on loopback (no paused clock, Q25).

#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

mod common;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use common::*;
use hacivat_driver::fake::FakeDriver;
use hacivat_net::ClientCertPaths;
use hacivat_net::ssh::{
    HopSpec, HostKeyPolicy, KnownHosts, SshAuth, SshSpec, TunnelConfig, TunnelManager,
};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_secrets::{
    ExposeSecret, FileVault, Retention, SecretBackend, SecretKey, SecretSlot, SecretStore,
    VaultBackend, VaultParams,
};
use hacivat_session::pipeline::{
    Answer, ConnectDeps, ConnectError, ConnectRequest, LiveConnection, Question, SecretPrompt,
    SessionSecrets, connect,
};
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};
use hacivat_session::test_connection;
use hacivat_testkit::ssh_server::{ForwardTarget, TestSshConfig, TestSshServer};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------------------------
// Fixtures

fn cfg() -> TestSshConfig {
    TestSshConfig::default()
}

/// The database behind the bastion: accepts and holds every connection.
async fn db_sink() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held.push(stream);
        }
    });
    addr
}

/// A bastion: channels for `db.internal` go to a held local socket.
async fn ssh(config: TestSshConfig) -> TestSshServer {
    TestSshServer::start(config.forward_to(ForwardTarget::Redirect(db_sink().await)))
}

fn dir() -> &'static std::path::Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

fn fresh_path(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    dir().join(format!("{name}-{}", N.fetch_add(1, Ordering::SeqCst)))
}

fn known_empty() -> Arc<KnownHosts> {
    KnownHosts::load(Vec::new(), fresh_path("known_hosts"))
}

/// Deps over `fake` and `secrets`, with their own tunnel manager (no agent, the default 30 s
/// grace: fix round 1, review I4). A direct profile keeps the stub transport (no socket); an
/// SSH profile uses the tunnel.
fn deps_over_store(fake: FakeDriver, secrets: Arc<SecretStore>) -> ConnectDeps {
    let mut deps = deps_with_store(fake, secrets);
    let known = known_empty();
    deps.tunnels = TunnelManager::with_agent(known.clone(), TunnelConfig::default(), None);
    deps.known_hosts = known;
    deps
}

fn deps_ssh(_s: &TestSshServer, fake: FakeDriver) -> ConnectDeps {
    deps_over_store(
        fake,
        Arc::new(SecretStore::new(Some(Arc::new(
            hacivat_secrets::MemoryBackend::new(),
        )))),
    )
}

fn hop(s: &TestSshServer, auth: SshAuth) -> HopSpec {
    HopSpec {
        host: s.host(),
        port: s.addr.port(),
        user: "u".into(),
        auth,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

fn through(mut p: Profile, hops: Vec<HopSpec>, policy: HostKeyPolicy) -> Profile {
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops,
        host_keys: policy,
        keepalive_s: None,
    });
    p
}

fn db_internal(mut p: Profile) -> Profile {
    p.endpoint = ProfileEndpoint::Server {
        host: "db.internal".into(),
        port: Some(5432),
    };
    p
}

/// A profile through `s` (password hop, `AcceptNew`) whose SSH secrets are kept `Forever`.
fn ssh_forever(s: &TestSshServer) -> Profile {
    let mut p = through(
        db_internal(pg_profile(hacivat_net::TlsMode::Disable)),
        vec![hop(s, SshAuth::Password)],
        HostKeyPolicy::AcceptNew,
    );
    p.ssh_retention = Retention::Forever;
    p
}

/// The secret `deps` keeps in `slot` for the test profile, `Forever`.
fn stored_slot(deps: &ConnectDeps, slot: SecretSlot) -> Option<String> {
    deps.secrets
        .get(
            &SecretKey {
                profile: pid(),
                slot,
            },
            Retention::Forever,
        )
        .unwrap()
        .map(|s| s.expose_secret().to_string())
}

fn expose(s: &hacivat_secrets::SecretString) -> &str {
    s.expose_secret()
}

/// Argon2 parameters small enough for tests (the vault opens a header equal to its own).
fn fast() -> VaultParams {
    VaultParams {
        m_kib: 8,
        t: 1,
        p: 1,
    }
}

/// Deps whose `Forever` backend is a vault under `master` holding the database password
/// `pw`, locked.
fn deps_with_locked_vault(master: &str, pw: &str) -> (ConnectDeps, Arc<FileVault>) {
    let vault = Arc::new(FileVault::with_params(fresh_path("vault.bin"), fast()));
    vault.create(sec(master)).unwrap();
    vault.put(&db_key(pid()), &sec(pw)).unwrap();
    vault.lock();
    let store = Arc::new(SecretStore::new(Some(
        vault.clone() as Arc<dyn SecretBackend>
    )));
    (deps_over_store(ok_pg(), store), vault)
}

/// Deps whose `Forever` backend is a vault that does not exist yet (a Linux machine without
/// Secret Service, Q38).
fn deps_with_absent_vault() -> ConnectDeps {
    let vault = Arc::new(FileVault::with_params(fresh_path("vault.bin"), fast()));
    deps_over_store(ok_pg(), Arc::new(SecretStore::new(Some(vault))))
}

fn vault_of(deps: &ConnectDeps) -> &dyn VaultBackend {
    deps.secrets.vault().expect("a vault backend")
}

fn deps_never_retention() -> ConnectDeps {
    deps_with(ok_pg()).0
}

fn with_overlay(mut r: ConnectRequest, overlay: SessionSecrets) -> ConnectRequest {
    r.overlay_secrets = Some(overlay);
    r
}

fn db_secret(pw: &str) -> SessionSecrets {
    SessionSecrets {
        db: Some(sec(pw)),
        ..SessionSecrets::default()
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// `p` with the test client certificate and its encrypted PKCS #8 key `key`.
fn with_client_key(mut p: Profile, key: &str) -> Profile {
    p.tls.client_cert = Some(ClientCertPaths {
        cert: fixture("tls-client.crt"),
        key: fixture(key),
    });
    p
}

async fn connect_with_encrypted_client_key(
    ui: &ScriptedUiM2b,
) -> Result<LiveConnection, ConnectError> {
    let p = with_client_key(
        pg_profile(hacivat_net::TlsMode::Auto),
        "tls-client-key.enc.der",
    );
    connect(
        &deps_never_retention(),
        req(p),
        ui.ix(),
        token(),
        progress(),
    )
    .await
}

/// The `hop` of every SSH secret question, in order.
fn secret_prompt_hops(ui: &ScriptedUiM2b) -> Vec<u8> {
    ui.asked()
        .into_iter()
        .filter_map(|q| match q {
            Question::Secret {
                prompt:
                    SecretPrompt::SshPassword { hop, .. } | SecretPrompt::SshPassphrase { hop, .. },
                ..
            } => Some(hop),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// SSH secrets (R8, Q23)

#[tokio::test]
async fn prompted_ssh_secret_stored_only_after_success() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    // The first connect fails after SSH succeeded.
    let deps = deps_ssh(
        &s,
        FakeDriver::new(DbmsKind::Postgres)
            .consumes_factory()
            .script(vec![Err(db("3D000")), Ok(())]),
    );
    let _ = connect(
        &deps,
        req(ssh_forever(&s)),
        ui_secret("pw"),
        token(),
        progress(),
    )
    .await;
    assert!(stored_slot(&deps, SecretSlot::SshPassword { hop: 0 }).is_none());
    // A connect that types the secret and succeeds stores it (its own tunnel manager, so it
    // opens and authenticates a chain of its own).
    let fresh = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    connect(
        &fresh,
        req(ssh_forever(&s)),
        ui_secret("pw"),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(
        stored_slot(&fresh, SecretSlot::SshPassword { hop: 0 }).as_deref(),
        Some("pw")
    );
}

/// Fix round 1 (review I4, ruling a): with the default 30 s grace, the connect after a failed
/// one joins the chain still open. It is not asked again, and the hop secret that
/// authenticated the chain reaches its `secrets_used` (the session overlay, Q17), so a later
/// headless reconnect can sign in. Only a connect that typed a secret stores it (R8): the
/// joining one did not, so nothing is stored.
#[tokio::test]
async fn a_connect_that_joins_within_the_grace_gets_the_chains_hop_secret() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(
        &s,
        FakeDriver::new(DbmsKind::Postgres)
            .consumes_factory()
            .script(vec![Err(db("57P03"))]),
    );
    let first = ScriptedUi::answers_m2b([Answer::Secret(sec("pw"))]);
    let e = connect(&deps, req(ssh_forever(&s)), first.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::ServerStarting);
    assert_eq!(s.sessions_accepted(), 1);
    let again = ScriptedUi::answers_m2b([Answer::Secret(sec("pw"))]);
    let live = connect(&deps, req(ssh_forever(&s)), again.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(s.sessions_accepted(), 1, "joined the chain in its grace");
    assert!(again.asked().is_empty(), "nothing asked again");
    let hops = &live.secrets_used.hops;
    assert_eq!(expose(hops[0].password.as_ref().unwrap()), "pw");
    assert!(stored_slot(&deps, SecretSlot::SshPassword { hop: 0 }).is_none());
}

/// The per-hop prompts of a jump chain: a jump host and a bastion, each asking for its own
/// secret. Passwords, not key passphrases: the session tests have no SSH key library to write
/// an encrypted key (the per-hop passphrase path is Task 9's `secrets_are_handed_back_by_hop`).
#[tokio::test]
async fn ssh_secret_prompts_per_hop() {
    let b = ssh(cfg().user_pw("u", "pb")).await;
    let a = TestSshServer::start(cfg().user_pw("u", "pa"));
    let ui = ScriptedUi::answers_m2b([
        Answer::TrustHostKey,
        Answer::Secret(sec("pa")),
        Answer::TrustHostKey,
        Answer::Secret(sec("pb")),
    ]);
    let p = through(
        db_internal(pg_profile(hacivat_net::TlsMode::Disable)),
        vec![hop(&a, SshAuth::Password), hop(&b, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    let deps = deps_ssh(&b, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let live = connect(&deps, req(p), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(secret_prompt_hops(&ui), [0, 1]);
    // Both typed passwords, by hop, for the session overlay (Q17).
    let hops = &live.secrets_used.hops;
    assert_eq!(hops.len(), 2);
    assert_eq!(expose(hops[0].password.as_ref().unwrap()), "pa");
    assert_eq!(expose(hops[1].password.as_ref().unwrap()), "pb");
}

/// A Test (from the dialog) uses a typed SSH secret and never stores it (R8).
#[tokio::test]
async fn a_test_never_stores_a_typed_ssh_secret() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let report = test_connection(
        &deps,
        draft(ssh_forever(&s), None),
        ui_secret("pw"),
        token(),
        progress(),
    )
    .await;
    assert!(report.ok, "{:?}", report.error.map(|e| e.class));
    assert!(stored_slot(&deps, SecretSlot::SshPassword { hop: 0 }).is_none());
}

// ---------------------------------------------------------------------------------------------
// The vault (B13, Q33, Q38)

#[tokio::test]
async fn vault_locked_asks_unlock_then_reads() {
    let (deps, vault) = deps_with_locked_vault("correct horse", "pw");
    let ui = ScriptedUi::answers_m2b([
        Answer::Master(sec("wrong horse")),
        Answer::Master(sec("correct horse")),
    ]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(ui.unlock_attempts(), [1, 2]);
    assert!(vault.is_unlocked());
    assert_eq!(
        ui.asked().len(),
        2,
        "the stored password was read, never asked"
    );
}

#[tokio::test]
async fn three_wrong_master_passwords_fall_back_to_password_prompt() {
    // Q33
    let (deps, vault) = deps_with_locked_vault("correct horse", "pw");
    let ui = ScriptedUi::answers_m2b([
        Answer::Master(sec("w1")),
        Answer::Master(sec("w2")),
        Answer::Master(sec("w3")),
        Answer::Password(sec("pw")),
    ]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(ui.unlock_attempts(), [1, 2, 3]);
    assert!(matches!(ui.asked()[3], Question::Password { .. }));
    assert!(!vault.is_unlocked());
}

#[tokio::test]
async fn first_forever_secret_on_vault_machine_asks_create_vault() {
    // Q38
    let deps = deps_with_absent_vault();
    let ui = ScriptedUi::answers_m2b([
        Answer::Password(sec("pw")),
        Answer::Master(sec("correct horse")),
    ]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert!(matches!(ui.asked()[1], Question::CreateVault));
    assert!(vault_of(&deps).exists());
    assert_eq!(
        stored_slot(&deps, SecretSlot::DbPassword).as_deref(),
        Some("pw")
    );
}

/// Not creating the vault keeps the secret until restart (the M2a toast), never an error.
#[tokio::test]
async fn declining_the_vault_keeps_the_secret_until_restart() {
    let deps = deps_with_absent_vault();
    let ui = ScriptedUi::answers_m2b([Answer::Password(sec("pw")), Answer::Cancelled]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert!(!vault_of(&deps).exists());
    let kept = deps
        .secrets
        .get(&db_key(pid()), Retention::UntilRestart)
        .unwrap();
    assert!(kept.is_some_and(|s| s.expose_secret() == "pw"));
}

/// Headless never asks to unlock or create: a locked vault reads as nothing stored, and the
/// connect stops where it would have asked for the password.
#[tokio::test]
async fn headless_never_asks_for_the_vault() {
    let (deps, vault) = deps_with_locked_vault("correct horse", "pw");
    let e = connect(
        &deps,
        req(forever_profile()),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap_err();
    assert!(e.needs_interaction);
    assert_eq!(e.class, ErrClass::AuthInvalid);
    assert!(!vault.is_unlocked());
}

#[tokio::test]
async fn overlay_secrets_are_used_before_the_store_and_returned() {
    // Q17
    let deps = deps_never_retention();
    let live = connect(
        &deps,
        with_overlay(req(never_profile()), db_secret("pw")),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(expose(live.secrets_used.db.as_ref().unwrap()), "pw");
}

#[tokio::test]
async fn vault_unlock_cancel_falls_back_to_password_prompt() {
    let (deps, _) = deps_with_locked_vault("correct horse", "pw");
    let ui = ScriptedUi::answers_m2b([Answer::Cancelled, Answer::Password(sec("pw"))]);
    connect(&deps, req(forever_profile()), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert!(matches!(ui.asked()[1], Question::Password { .. }));
}

// ---------------------------------------------------------------------------------------------
// The TLS key passphrase (B25)

#[tokio::test]
async fn encrypted_tls_key_passphrase_asked_when_missing() {
    let ui = ScriptedUi::answers_m2b([Answer::Secret(sec("pw1"))]);
    let live = connect_with_encrypted_client_key(&ui).await.unwrap();
    assert!(matches!(
        &ui.asked()[0],
        Question::Secret {
            prompt: SecretPrompt::TlsKeyPassphrase { .. },
            ..
        }
    ));
    assert_eq!(
        expose(live.secrets_used.tls_key_passphrase.as_ref().unwrap()),
        "pw1"
    );
    drop(live);
}

/// A wrong passphrase is asked again (3 attempts); headless reports that it needed the user.
#[tokio::test]
async fn a_wrong_tls_key_passphrase_is_asked_again_and_headless_needs_interaction() {
    let ui = ScriptedUi::answers_m2b([Answer::Secret(sec("nope")), Answer::Secret(sec("pw1"))]);
    connect_with_encrypted_client_key(&ui).await.unwrap();
    assert_eq!(ui.asked().len(), 2);

    let p = with_client_key(
        pg_profile(hacivat_net::TlsMode::Auto),
        "tls-client-key.enc.der",
    );
    let e = connect(
        &deps_never_retention(),
        req(p),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.class, ErrClass::ConfigInvalid);
    assert!(e.needs_interaction);
}

/// The dialog's TLS key passphrase reaches a Test through the overlay, used and never asked.
#[tokio::test]
async fn a_test_uses_the_dialogs_tls_key_passphrase() {
    let p = with_client_key(
        pg_profile(hacivat_net::TlsMode::Auto),
        "tls-client-key.enc.der",
    );
    let mut d = draft(p, None);
    d.tls_key_passphrase = Some(sec("pw1"));
    let ui = ScriptedUi::answers_m2b([]);
    let report = test_connection(&deps_never_retention(), d, ui.ix(), token(), progress()).await;
    assert!(report.ok, "{:?}", report.error.map(|e| e.class));
    assert!(ui.asked().is_empty());
}

// ---------------------------------------------------------------------------------------------
// Logs (Q33)

/// Every M2b secret on one connect: the vault master password, an SSH password (hop 0), a
/// keyboard-interactive answer (hop 1), a TLS key passphrase and the database password.
async fn full_m2b_connect_with_secrets(
    db: &str,
    ssh_pw: &str,
    kbd: &str,
    master: &str,
    key: &str,
) -> Result<LiveConnection, ConnectError> {
    let b = ssh(cfg().user_kbd("u", &[("Code: ", false, kbd)])).await;
    let a = TestSshServer::start(cfg().user_pw("u", ssh_pw));
    let vault = Arc::new(FileVault::with_params(fresh_path("vault.bin"), fast()));
    vault.create(sec(master)).unwrap();
    vault.lock();
    let store = Arc::new(SecretStore::new(Some(
        vault.clone() as Arc<dyn SecretBackend>
    )));
    let deps = deps_over_store(
        FakeDriver::new(DbmsKind::Postgres).consumes_factory(),
        store,
    );
    let mut p = through(
        db_internal(with_client_key(
            pg_profile(hacivat_net::TlsMode::Auto),
            "tls-client-key.enc2.der",
        )),
        vec![
            hop(&a, SshAuth::Password),
            hop(&b, SshAuth::KeyboardInteractive),
        ],
        HostKeyPolicy::Ask,
    );
    p.auth = hacivat_secrets::AuthProviderId::Password;
    p.retention = Retention::Forever;
    p.ssh_retention = Retention::Forever;
    let ui = ScriptedUi::answers_m2b([
        Answer::Master(sec(master)),
        Answer::TrustHostKey,
        Answer::Secret(sec(ssh_pw)),
        Answer::TrustHostKey,
        Answer::Keyboard(vec![sec(kbd)]),
        Answer::Secret(sec(key)),
        Answer::Password(sec(db)),
    ]);
    let result = connect(&deps, req(p), ui.ix(), token(), progress()).await;
    // Everything was asked in this order, and the connect used it.
    let kinds: Vec<&'static str> = ui
        .asked()
        .iter()
        .map(|q| match q {
            Question::UnlockVault { .. } => "unlock",
            Question::HostKey { .. } => "host_key",
            Question::Secret { .. } => "secret",
            Question::Keyboard { .. } => "keyboard",
            Question::Password { .. } => "password",
            _ => "other",
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "unlock", "host_key", "secret", "host_key", "keyboard", "secret", "password"
        ]
    );
    if let Ok(live) = &result {
        assert!(!format!("{live:?}").contains("s3cret"));
    }
    if let Err(e) = &result {
        assert!(!format!("{e:?}").contains("s3cret"));
    }
    result
}

#[tokio::test]
async fn connect_logs_never_contain_any_m2b_secret() {
    // Q33: at TRACE, with the `log` bridge, so russh's records are scanned too.
    let _ = tracing_log::LogTracer::init();
    let outcome = Arc::new(Mutex::new(None));
    let seen = outcome.clone();
    let logs = capture_tracing(|| async move {
        let r = full_m2b_connect_with_secrets(
            "s3cret-db",
            "s3cret-ssh",
            "s3cret-kbd",
            "s3cret-master",
            "s3cret-key",
        )
        .await;
        *seen.lock().unwrap() = Some(r.is_ok());
    })
    .await;
    assert_eq!(
        *outcome.lock().unwrap(),
        Some(true),
        "the connect succeeded"
    );
    assert!(!logs.is_empty(), "the capture works");
    for s in [
        "s3cret-db",
        "s3cret-ssh",
        "s3cret-kbd",
        "s3cret-master",
        "s3cret-key",
    ] {
        assert!(!logs.contains(s), "{s}");
    }
}

// ## m2b-t14-pipeline fix round 1 (review M2)

/// A UI that answers from a list, and cancels `token` the moment it is asked to create the
/// vault (the user disconnects while the question is open).
struct CancelsAtCreateVault {
    answers: Mutex<Vec<Answer>>,
    token: tokio_util::sync::CancellationToken,
}

impl hacivat_session::pipeline::UiPort for CancelsAtCreateVault {
    fn ask(&self, q: Question) -> futures::future::BoxFuture<'static, Answer> {
        if matches!(q, Question::CreateVault) {
            self.token.cancel();
        }
        let answer = {
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                Answer::Cancelled
            } else {
                answers.remove(0)
            }
        };
        Box::pin(async move { answer })
    }
}

/// A cancel while the create-vault question is open creates no vault, stores nothing, closes
/// the connection and fails the connect as cancelled.
#[tokio::test]
async fn a_cancel_during_the_create_vault_question_stores_nothing() {
    let f = Arc::new(ok_pg());
    let vault = Arc::new(FileVault::with_params(fresh_path("vault.bin"), fast()));
    let store = Arc::new(SecretStore::new(Some(
        vault.clone() as Arc<dyn SecretBackend>
    )));
    let mut deps = deps_over_store(ok_pg(), store);
    let mut drivers = hacivat_driver::DriverRegistry::new();
    drivers.register(f.clone());
    deps.drivers = Arc::new(drivers);
    let cancel = token();
    let ui = Arc::new(CancelsAtCreateVault {
        answers: Mutex::new(vec![
            Answer::Password(sec("pw")),
            Answer::Master(sec("correct horse")),
        ]),
        token: cancel.clone(),
    });
    let e = connect(
        &deps,
        req(forever_profile()),
        hacivat_session::pipeline::Interaction::Interactive(ui),
        cancel,
        progress(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.class, ErrClass::Cancelled);
    assert!(!vault.exists(), "no vault was created");
    for retention in [Retention::Forever, Retention::UntilRestart] {
        let kept = deps.secrets.get(&db_key(pid()), retention).unwrap();
        assert!(kept.is_none(), "{retention:?}");
    }
    assert_eq!(f.open_connections(), 0, "the connection was closed");
}

// ## m2b-t14-pipeline re-review N1

/// A memory backend that cancels `token` when it is asked to store the TLS key passphrase:
/// the user disconnects while the connect stores what it was given (after the database
/// password, which it stores first).
struct CancelsAtTlsPassphrase {
    inner: hacivat_secrets::MemoryBackend,
    token: tokio_util::sync::CancellationToken,
}

impl SecretBackend for CancelsAtTlsPassphrase {
    fn get(
        &self,
        key: &SecretKey,
    ) -> Result<Option<hacivat_secrets::SecretString>, hacivat_secrets::SecretError> {
        self.inner.get(key)
    }
    fn put(
        &self,
        key: &SecretKey,
        value: &hacivat_secrets::SecretString,
    ) -> Result<(), hacivat_secrets::SecretError> {
        self.inner.put(key, value)?;
        if key.slot == SecretSlot::TlsKeyPassphrase {
            self.token.cancel();
        }
        Ok(())
    }
    fn delete(&self, key: &SecretKey) -> Result<(), hacivat_secrets::SecretError> {
        self.inner.delete(key)
    }
    fn name(&self) -> &'static str {
        "cancels-at-tls-passphrase"
    }
}

/// A cancel while the connect stores its secrets undoes every key it stored: the database
/// password stored first is gone too, not only the one in flight (R8: a cancelled connect
/// stores nothing).
#[tokio::test]
async fn a_cancel_while_storing_undoes_every_stored_secret() {
    let cancel = token();
    let backend = Arc::new(CancelsAtTlsPassphrase {
        inner: hacivat_secrets::MemoryBackend::new(),
        token: cancel.clone(),
    });
    let store = Arc::new(SecretStore::new(Some(
        backend.clone() as Arc<dyn SecretBackend>
    )));
    let deps = deps_over_store(ok_pg(), store);
    let mut p = with_client_key(forever_profile(), "tls-client-key.enc.der");
    p.tls.mode = hacivat_net::TlsMode::Auto;
    let ui = ScriptedUi::answers_m2b([Answer::Secret(sec("pw1")), Answer::Password(sec("pw"))]);
    let e = connect(&deps, req(p), ui.ix(), cancel, progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::Cancelled);
    for slot in [SecretSlot::DbPassword, SecretSlot::TlsKeyPassphrase] {
        let key = SecretKey {
            profile: pid(),
            slot,
        };
        assert!(
            backend.inner.get(&key).unwrap().is_none(),
            "{slot:?} remained"
        );
        for retention in [Retention::Forever, Retention::UntilRestart] {
            assert!(
                deps.secrets.get(&key, retention).unwrap().is_none(),
                "{slot:?} {retention:?}"
            );
        }
    }
}

// ## m2b-t21-prompts (fix round 1): the SSH secret prompt's Remember.

/// The secret `deps` keeps in `slot` of the test profile under `retention`.
fn stored_under(deps: &ConnectDeps, slot: SecretSlot, retention: Retention) -> Option<String> {
    deps.secrets
        .get(
            &SecretKey {
                profile: pid(),
                slot,
            },
            retention,
        )
        .unwrap()
        .map(|s| s.expose_secret().to_string())
}

fn ui_ssh_secret(pw: &str, remember: Retention) -> ScriptedUiM2b {
    ScriptedUi::answers_m2b([Answer::SshSecret {
        secret: sec(pw),
        remember,
    }])
}

/// A Remember that differs from the profile's `ssh_retention`: the typed secret is stored under
/// it, and the successful Main connect reports `SshRetention` so the app re-saves the profile.
#[tokio::test]
async fn a_remember_choice_stores_under_it_and_reports_the_new_retention() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let ui = ui_ssh_secret("pw", Retention::UntilRestart);
    let live = connect(&deps, req(ssh_forever(&s)), ui.ix(), token(), progress())
        .await
        .unwrap();
    let slot = SecretSlot::SshPassword { hop: 0 };
    assert!(
        stored_under(&deps, slot, Retention::UntilRestart).as_deref() == Some("pw"),
        "kept until restart (values not shown)"
    );
    assert!(matches!(
        live.profile_changes[..],
        [hacivat_session::pipeline::ProfileChange::SshRetention(
            Retention::UntilRestart
        )]
    ));
}

/// The typed secret replaces an older one kept under the profile's retention: the old place is
/// cleared, so the app's re-save (which moves stored secrets to the new retention) never brings
/// the stale secret back over the typed one.
#[tokio::test]
async fn a_remember_choice_clears_the_old_place() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let slot = SecretSlot::SshPassword { hop: 0 };
    deps.secrets
        .put(
            &SecretKey {
                profile: pid(),
                slot,
            },
            sec("stale"),
            Retention::Forever,
        )
        .unwrap();
    let ui = ui_ssh_secret("pw", Retention::ForSession);
    connect(&deps, req(ssh_forever(&s)), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert!(
        stored_under(&deps, slot, Retention::Forever).is_none(),
        "the stale one went"
    );
    assert!(
        stored_under(&deps, slot, Retention::ForSession).as_deref() == Some("pw"),
        "the typed one stays (values not shown)"
    );
}

/// The same retention as the profile's reports nothing; a Test never stores and never reports.
#[tokio::test]
async fn a_remember_choice_is_reported_only_when_it_changes_and_never_by_a_test() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let same = ui_ssh_secret("pw", Retention::Forever);
    let live = connect(&deps, req(ssh_forever(&s)), same.ix(), token(), progress())
        .await
        .unwrap();
    assert!(live.profile_changes.is_empty());
    assert!(
        stored_slot(&deps, SecretSlot::SshPassword { hop: 0 }).as_deref() == Some("pw"),
        "stored Forever (values not shown)"
    );

    let fresh = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let test = ui_ssh_secret("pw", Retention::UntilRestart);
    let mut r = req(ssh_forever(&s));
    r.purpose = hacivat_session::pipeline::Purpose::Test;
    let live = connect(&fresh, r, test.ix(), token(), progress())
        .await
        .unwrap();
    assert!(live.profile_changes.is_empty(), "a Test reports nothing");
    let slot = SecretSlot::SshPassword { hop: 0 };
    assert!(stored_under(&fresh, slot, Retention::UntilRestart).is_none());
    assert!(stored_under(&fresh, slot, Retention::Forever).is_none());
}

// ## m2b-t21-prompts (fix round 2, R2): where the prompt's Remember starts.

/// A user who presses Enter on every prompt: trusts host keys, types `passwords[hop]` and keeps
/// the Remember each SSH question starts from, except `pick` on the first one. Records every
/// question.
#[derive(Clone)]
struct BareEnter {
    passwords: Vec<&'static str>,
    pick: Option<Retention>,
    asked: Arc<Mutex<Vec<Question>>>,
}

impl BareEnter {
    fn new(passwords: &[&'static str], pick: Option<Retention>) -> BareEnter {
        BareEnter {
            passwords: passwords.to_vec(),
            pick,
            asked: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn ix(&self) -> hacivat_session::pipeline::Interaction {
        hacivat_session::pipeline::Interaction::Interactive(Arc::new(self.clone()))
    }

    /// The `remember` of every SSH secret question, in order.
    fn remembers(&self) -> Vec<Option<Retention>> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter_map(|q| match q {
                Question::Secret { remember, .. } => Some(*remember),
                _ => None,
            })
            .collect()
    }
}

impl hacivat_session::pipeline::UiPort for BareEnter {
    fn ask(&self, q: Question) -> futures::future::BoxFuture<'static, Answer> {
        let first = self.remembers().is_empty();
        let answer = match &q {
            Question::HostKey { .. } => Answer::TrustHostKey,
            Question::Secret {
                prompt: SecretPrompt::SshPassword { hop, .. },
                remember,
                ..
            } => {
                let secret = sec(self.passwords[usize::from(*hop)]);
                match (remember, self.pick.filter(|_| first)) {
                    (Some(_), Some(pick)) => Answer::SshSecret {
                        secret,
                        remember: pick,
                    },
                    (Some(start), None) => Answer::SshSecret {
                        secret,
                        remember: *start,
                    },
                    (None, _) => Answer::Secret(secret),
                }
            }
            _ => Answer::Cancelled,
        };
        self.asked.lock().unwrap().push(q);
        Box::pin(async move { answer })
    }
}

/// A profile through `s` whose SSH secrets are kept `Never`.
fn ssh_never(s: &TestSshServer) -> Profile {
    let mut p = ssh_forever(s);
    p.ssh_retention = Retention::Never;
    p
}

/// Deps with their own tunnel manager over `deps`'s secrets: a later connect of the same
/// profile asks again instead of reusing a tunnel kept for its grace period.
fn same_secrets(deps: &ConnectDeps) -> ConnectDeps {
    deps_over_store(
        FakeDriver::new(DbmsKind::Postgres).consumes_factory(),
        deps.secrets.clone(),
    )
}

fn retention_reported(live: &LiveConnection, r: Retention) -> bool {
    live.profile_changes
        .iter()
        .any(|c| matches!(c, hacivat_session::pipeline::ProfileChange::SshRetention(x) if *x == r))
}

/// The review's r1-g: a new connection set to Never; Test (Enter); Save; Connect (Enter). The
/// Test's question offers no Remember (it stores nothing); the Connect's starts from the saved
/// Never, so the password is never kept Forever and the profile stays Never.
#[tokio::test]
async fn a_bare_enter_after_a_test_keeps_the_saved_never() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let test = BareEnter::new(&["pw"], None);
    let report = test_connection(
        &deps,
        draft(ssh_never(&s), None),
        test.ix(),
        token(),
        progress(),
    )
    .await;
    assert!(report.ok, "{:?}", report.error.map(|e| e.class));
    assert_eq!(test.remembers(), [None]);

    let later = same_secrets(&deps);
    let ui = BareEnter::new(&["pw"], None);
    let live = connect(&later, req(ssh_never(&s)), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(ui.remembers(), [Some(Retention::Never)]);
    assert!(
        live.profile_changes.is_empty(),
        "nothing to re-save: the profile stays Never"
    );
    let slot = SecretSlot::SshPassword { hop: 0 };
    for r in [
        Retention::Forever,
        Retention::UntilRestart,
        Retention::ForSession,
    ] {
        assert!(stored_under(&later, slot, r).is_none(), "kept nowhere");
    }
}

/// The review's r1-f: a choice applies within its own connect only. Connect 1 picks Forever;
/// the user then sets Never again; connect 2 starts from the saved Never.
#[tokio::test]
async fn the_remember_starts_from_the_saved_retention_not_an_earlier_choice() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let deps = deps_ssh(&s, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let slot = SecretSlot::SshPassword { hop: 0 };
    let first = BareEnter::new(&["pw"], Some(Retention::Forever));
    let live = connect(&deps, req(ssh_never(&s)), first.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(first.remembers(), [Some(Retention::Never)]);
    assert!(retention_reported(&live, Retention::Forever));
    assert!(
        stored_slot(&deps, slot).as_deref() == Some("pw"),
        "values not shown"
    );
    drop(live);
    // The user sets Never again in the dialog: the save moves the secret out of Forever.
    deps.secrets
        .change_retention(
            &SecretKey {
                profile: pid(),
                slot,
            },
            Retention::Forever,
            Retention::Never,
        )
        .unwrap();

    let later = same_secrets(&deps);
    let second = BareEnter::new(&["pw"], None);
    let live = connect(&later, req(ssh_never(&s)), second.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(second.remembers(), [Some(Retention::Never)]);
    assert!(live.profile_changes.is_empty());
    assert!(stored_slot(&later, slot).is_none(), "not kept Forever");
}

/// Within one connect, the next hop's question starts from the choice made on the first.
#[tokio::test]
async fn the_next_hop_of_the_same_connect_starts_from_the_choice() {
    let b = ssh(cfg().user_pw("u", "pb")).await;
    let a = TestSshServer::start(cfg().user_pw("u", "pa"));
    let mut p = through(
        db_internal(pg_profile(hacivat_net::TlsMode::Disable)),
        vec![hop(&a, SshAuth::Password), hop(&b, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    p.ssh_retention = Retention::Forever;
    let deps = deps_ssh(&b, FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let ui = BareEnter::new(&["pa", "pb"], Some(Retention::UntilRestart));
    let live = connect(&deps, req(p), ui.ix(), token(), progress())
        .await
        .unwrap();
    assert_eq!(
        ui.remembers(),
        [Some(Retention::Forever), Some(Retention::UntilRestart)]
    );
    assert!(retention_reported(&live, Retention::UntilRestart));
    let kept = |hop| {
        stored_under(
            &deps,
            SecretSlot::SshPassword { hop },
            Retention::UntilRestart,
        )
    };
    assert!(kept(0).as_deref() == Some("pa"), "values not shown");
    assert!(kept(1).as_deref() == Some("pb"), "values not shown");
}
