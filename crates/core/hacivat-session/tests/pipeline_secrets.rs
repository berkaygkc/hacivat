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
use std::time::Duration;

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

/// Deps over `fake` and `secrets`, with their own tunnel manager (no agent, 0 s grace). A
/// direct profile keeps the stub transport (no socket); an SSH profile uses the tunnel.
fn deps_over_store(fake: FakeDriver, secrets: Arc<SecretStore>) -> ConnectDeps {
    let mut deps = deps_with_store(fake, secrets);
    let known = known_empty();
    deps.tunnels = TunnelManager::with_agent(
        known.clone(),
        TunnelConfig {
            idle_grace: Duration::ZERO,
            ..TunnelConfig::default()
        },
        None,
    );
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

/// Waits until `deps`' tunnel manager has no session open (the 0 s grace ran).
async fn tunnels_closed(deps: &ConnectDeps) {
    eventually("the tunnel closed", || deps.tunnels.sessions_open() == 0).await;
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
    // A 0 s grace: the next connect opens (and authenticates) its own tunnel.
    tunnels_closed(&deps).await;
    connect(
        &deps,
        req(ssh_forever(&s)),
        ui_secret("pw"),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert_eq!(
        stored_slot(&deps, SecretSlot::SshPassword { hop: 0 }).as_deref(),
        Some("pw")
    );
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
