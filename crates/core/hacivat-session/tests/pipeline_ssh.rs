//! The pipeline's SSH transport step (Task 14; B11, Q17, Q23, Q44): the trail, the database
//! host as SNI, no local DNS lookup, prompt time outside the deadline, hop-named failures, the
//! changed-key error, MySQL through the forwarder and Test never sharing a tunnel. Against the
//! in-process testkit SSH server on loopback; every `known_hosts` file lives in a temp
//! directory. Real sockets set the pace, so no test uses a paused clock (Q25).

// Unix-only tests (file modes, Unix sockets, ssh-agent) leave their helpers unused on other
// systems; the lints still run in full on Unix.
#![cfg_attr(not(unix), allow(dead_code, unused_imports, unused_variables))]
#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

mod common;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use common::*;
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeDriver, FakeEndpoint};
use hacivat_driver::{Driver, DriverConnect, DriverDescriptor, DriverRegistry, RawConn, RawError};
use hacivat_net::ssh::{
    HopSecrets, HopSpec, HostKeyPolicy, KnownHosts, SshAuth, SshSpec, TunnelConfig, TunnelManager,
};
use hacivat_net::{AddressClass, TlsMode, ssh_address_class};
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_session::pipeline::{
    Answer, ConnectDeps, ConnectError, LiveConnection, Question, SessionSecrets, TrailStep, connect,
};
use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, PathKind, test_connection};
use hacivat_testkit::ssh_server::{ForwardTarget, TestSshConfig, TestSshServer};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------------------------
// Servers

fn cfg() -> TestSshConfig {
    TestSshConfig::default()
}

/// A TCP server that accepts and holds every connection: the database behind the bastion.
async fn echo_pg_like() -> SocketAddr {
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

/// A test SSH server; channels for names that do not resolve (`db.internal`) go to a held
/// local socket unless the config says where.
async fn ssh(mut config: TestSshConfig) -> TestSshServer {
    if config.forward_to == ForwardTarget::Any {
        config.forward_to = ForwardTarget::Redirect(echo_pg_like().await);
    }
    TestSshServer::start(config)
}

/// A jump host and a bastion, both password users `u` / `pw`.
async fn jump_pair() -> (TestSshServer, TestSshServer) {
    let b = ssh(cfg().user_pw("u", "pw")).await;
    // The jump host forwards to the bastion as asked (no redirect).
    let a = TestSshServer::start(cfg().user_pw("u", "pw"));
    (a, b)
}

fn dir() -> &'static std::path::Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// An empty, private `known_hosts` store.
fn known_empty() -> Arc<KnownHosts> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    KnownHosts::load(Vec::new(), dir().join(format!("known-{n}")))
}

/// A store whose OpenSSH file lists `s`'s host and port with another key.
async fn known_other(s: &TestSshServer) -> Arc<KnownHosts> {
    let other = TestSshServer::start(cfg());
    let path = dir().join(format!("openssh-{}", s.addr.port()));
    let line = format!(
        "{} {}\n",
        hacivat_net::ssh::known_hosts::host_pattern(&s.host(), s.addr.port()),
        other.host_key.openssh
    );
    std::fs::write(&path, line).unwrap();
    KnownHosts::load(vec![path], dir().join(format!("own-{}", s.addr.port())))
}

// ---------------------------------------------------------------------------------------------
// Deps and profiles

/// `FakeDriver` without stream injection (MySQL's descriptor shape): through SSH it is reached
/// through its loopback forwarder.
struct NoInjection {
    fake: Arc<FakeDriver>,
    descriptor: DriverDescriptor,
}

impl Driver for NoInjection {
    fn descriptor(&self) -> &DriverDescriptor {
        &self.descriptor
    }
    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        self.fake.connect(req)
    }
}

/// Deps over `driver`, with their own tunnel manager over `known` (no agent, the default
/// grace).
fn deps_on(driver: Arc<dyn Driver>, known: Arc<KnownHosts>) -> ConnectDeps {
    let (mut deps, _) = deps_with(FakeDriver::new(DbmsKind::Postgres));
    let mut drivers = DriverRegistry::new();
    drivers.register(driver);
    deps.drivers = Arc::new(drivers);
    deps.transport_wrap = None;
    deps.tunnels = TunnelManager::with_agent(known.clone(), TunnelConfig::default(), None);
    deps.known_hosts = known;
    deps
}

fn deps_fake(f: &Arc<FakeDriver>) -> ConnectDeps {
    deps_on(f.clone(), known_empty())
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

/// `p` through `hops`, with the host-key `policy`.
fn through(mut p: Profile, hops: Vec<HopSpec>, policy: HostKeyPolicy) -> Profile {
    p.transport = TransportSpec::SshTunnel(SshSpec {
        hops,
        host_keys: policy,
        keepalive_s: None,
    });
    p
}

fn with_host(mut p: Profile, host: &str, port: u16) -> Profile {
    p.endpoint = ProfileEndpoint::Server {
        host: host.into(),
        port: Some(port),
    };
    p
}

fn pg_auto(host: &str, port: u16) -> Profile {
    with_host(pg_profile(TlsMode::Auto), host, port)
}

fn mysql_auto(host: &str, port: u16) -> Profile {
    with_host(mysql_profile(TlsMode::Auto), host, port)
}

/// `p` through `s` (password hop, `Ask`), answered with `trust` then the SSH password `pw`.
async fn connect_ssh(
    f: &Arc<FakeDriver>,
    s: &TestSshServer,
    p: Profile,
    trust: Answer,
    pw: &str,
) -> Result<LiveConnection, ConnectError> {
    let ui = ScriptedUi::answers_m2b([trust, Answer::Secret(sec(pw))]);
    let p = through(p, vec![hop(s, SshAuth::Password)], HostKeyPolicy::Ask);
    connect(&deps_fake(f), req(p), ui.ix(), token(), progress()).await
}

async fn connect_ssh_with_deadline(
    f: &Arc<FakeDriver>,
    s: &TestSshServer,
    p: Profile,
    ui: &ScriptedUiM2b,
    deadline: Duration,
) -> Result<LiveConnection, ConnectError> {
    let mut deps = deps_fake(f);
    deps.timeout_override = Some(deadline);
    let p = through(p, vec![hop(s, SshAuth::Password)], HostKeyPolicy::Ask);
    connect(&deps, req(p), ui.ix(), token(), progress()).await
}

/// The chain `a` → `b`, headless (`AcceptNew`), with the overlay's passwords: `pw` everywhere
/// except a wrong one at hop `bad`.
async fn connect_chain_bad_pw_at(
    a: &TestSshServer,
    b: &TestSshServer,
    bad: usize,
) -> Result<LiveConnection, ConnectError> {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(a, SshAuth::Password), hop(b, SshAuth::Password)],
        HostKeyPolicy::AcceptNew,
    );
    let mut r = req(p);
    r.overlay_secrets = Some(SessionSecrets {
        hops: (0..2)
            .map(|i| HopSecrets {
                password: Some(sec(if i == bad { "wrong" } else { "pw" })),
                passphrase: None,
            })
            .collect(),
        ..SessionSecrets::default()
    });
    connect(&deps_fake(&f), r, ix_none(), token(), progress()).await
}

async fn connect_ssh_known_other(s: &TestSshServer) -> Result<LiveConnection, ConnectError> {
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(s, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    let deps = deps_on(f, known_other(s).await);
    let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Secret(sec("pw"))]);
    connect(&deps, req(p), ui.ix(), token(), progress()).await
}

fn steps(live: &LiveConnection) -> Vec<TrailStep> {
    live.trail.iter().map(|t| t.step).collect()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------------------------------------------------------------------------------------------
// Tests

#[tokio::test]
async fn ssh_profile_trail_is_ssh_connect_tls_signin_check() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let live = connect_ssh(
        &f,
        &s,
        pg_auto("db.internal", 5432),
        Answer::TrustHostKey,
        "pw",
    )
    .await
    .unwrap();
    assert_eq!(
        steps(&live),
        [
            TrailStep::Ssh,
            TrailStep::Connect,
            TrailStep::Tls,
            TrailStep::SignIn,
            TrailStep::CheckServer
        ]
    );
    assert_eq!(live.path, PathKind::Ssh);
    assert_eq!(live.via.as_deref(), Some(s.host().as_str()));
    assert!(live.transport.is_some());
    // The tunnel's own address class for the database host (B11).
    assert_eq!(live.address, AddressClass::Private);
    // The password that authenticated, for the session overlay (Q17).
    assert!(live.secrets_used.hops[0].password.is_some());
}

#[tokio::test]
async fn tls_through_ssh_sends_the_database_host_as_sni() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    connect_ssh(
        &f,
        &s,
        pg_auto("db.internal", 5432),
        Answer::TrustHostKey,
        "pw",
    )
    .await
    .unwrap();
    // A String in FakeTls (Q44).
    assert_eq!(
        f.last_request().unwrap().tls.unwrap().server_name,
        "db.internal"
    );
}

#[tokio::test]
async fn ssh_profile_never_resolves_the_database_host_locally() {
    // Q44
    let s = ssh(cfg()
        .user_pw("u", "pw")
        .forward_to(ForwardTarget::Redirect(echo_pg_like().await)))
    .await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let live = connect_ssh(
        &f,
        &s,
        pg_auto("db.internal", 5432),
        Answer::TrustHostKey,
        "pw",
    )
    .await
    .unwrap();
    assert!(!steps(&live).contains(&TrailStep::Resolve));
    // Nothing was resolved locally, and the driver has no direct target to dial.
    let Some(FakeEndpoint::Network {
        direct, resolved, ..
    }) = f.last_request().unwrap().endpoint
    else {
        panic!("a network endpoint")
    };
    assert!(resolved.is_empty());
    assert!(direct.is_none());
}

#[tokio::test]
async fn host_key_prompt_time_does_not_count_toward_the_deadline() {
    // P23, Q44
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let ui =
        ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Password(sec("pw"))]).delay(secs(3));
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    assert!(
        connect_ssh_with_deadline(&f, &s, pg_auto("db.internal", 5432), &ui, secs(2))
            .await
            .is_ok()
    );
}

#[test]
fn ssh_address_class_names_are_private() {
    assert_eq!(ssh_address_class("db.internal"), AddressClass::Private);
    assert_eq!(ssh_address_class("localhost"), AddressClass::Loopback);
    assert_eq!(ssh_address_class("8.8.8.8"), AddressClass::Public);
}

#[tokio::test]
async fn ssh_failure_names_the_hop() {
    let (a, b) = jump_pair().await;
    let e = connect_chain_bad_pw_at(&a, &b, 1).await.unwrap_err();
    assert_eq!(e.class, ErrClass::SshAuth);
    assert_eq!(e.args.hop, Some(1));
    assert_eq!(e.args.hop_host.as_deref(), Some(b.host().as_str()));
    // The trail fails at the tunnel, never at a database step.
    assert_eq!(failed_step(e), TrailStep::Ssh);
}

#[tokio::test]
async fn host_key_changed_error_carries_mismatch() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let e = connect_ssh_known_other(&s).await.unwrap_err();
    assert_eq!(e.class, ErrClass::SshHostKeyChanged);
    assert_eq!(e.fixes, [hacivat_proto::FixId::ReviewHostKey]);
    assert_eq!(e.host_key.unwrap().new, s.host_key);
}

/// A headless connect under `Ask` stops at an unknown host key, saying it needed the user
/// (Q17: a reconnect then becomes Lost with Retry). Nothing is stored or asked.
#[tokio::test]
async fn headless_unknown_host_key_needs_interaction() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let known = known_empty();
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    let e = connect(
        &deps_on(f.clone(), known.clone()),
        req(p),
        ix_none(),
        token(),
        progress(),
    )
    .await
    .unwrap_err();
    assert_eq!(e.class, ErrClass::SshHostKeyUnknown);
    assert!(e.needs_interaction);
    assert_eq!(e.args.hop, Some(0));
    assert_eq!(f.connect_calls(), 0, "the database is never reached");
    assert!(!known.hacivat_path().exists(), "no key was stored");
}

#[tokio::test]
async fn mysql_over_ssh_uses_forwarder() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let fake = Arc::new(FakeDriver::new(DbmsKind::MySql));
    let mut descriptor = fake.descriptor().clone();
    descriptor.caps.stream_injection = false;
    let driver = Arc::new(NoInjection {
        fake: fake.clone(),
        descriptor,
    });
    let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Secret(sec("pw"))]);
    let p = through(
        mysql_auto("db", 3306),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    let live = connect(
        &deps_on(driver, known_empty()),
        req(p),
        ui.ix(),
        token(),
        progress(),
    )
    .await
    .unwrap();
    assert!(live.forwarded());
    let Some(FakeEndpoint::Network { direct, .. }) = fake.last_request().unwrap().endpoint else {
        panic!("a network endpoint")
    };
    assert!(
        direct.is_none(),
        "MySQL never dials the database itself through SSH"
    );
}

#[tokio::test]
async fn test_connection_with_ssh_does_not_share_tunnel() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let deps = Arc::new(deps_fake(&f));
    let mut p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::AcceptNew,
    );
    p.ssh_retention = hacivat_secrets::Retention::Forever;
    deps.secrets
        .put(
            &hacivat_secrets::SecretKey {
                profile: p.id,
                slot: hacivat_secrets::SecretSlot::SshPassword { hop: 0 },
            },
            sec("pw"),
            hacivat_secrets::Retention::Forever,
        )
        .unwrap();
    let m = ConnectionManager::new(
        tokio::runtime::Handle::current(),
        deps.clone(),
        ManagerConfig::default(),
    );
    m.connect(
        Arc::new(p.clone()),
        None,
        hacivat_session::pipeline::Interaction::Headless,
    );
    wait_connected(&m, p.id).await;
    assert_eq!(s.sessions_accepted(), 1);
    let report = test_connection(
        &deps,
        draft(p.clone(), None),
        ix_none(),
        token(),
        progress(),
    )
    .await;
    assert!(report.ok, "{:?}", report.error.map(|e| e.class));
    // The test opened a session of its own instead of joining the live one.
    assert_eq!(s.sessions_accepted(), 2);
    assert!(matches!(m.state(p.id), LiveState::Connected(_)));
}

/// OpenSSH refuses a private key other users can read (M6); so does the SSH step, before
/// anything connects. Unix only (mode bits).
#[cfg(unix)]
#[tokio::test]
async fn a_key_file_other_users_can_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let key = dir().join("id_open");
    std::fs::write(&key, "not a key").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::KeyFile { path: key.clone() })],
        HostKeyPolicy::AcceptNew,
    );
    let e = connect(&deps_fake(&f), req(p), ix_none(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::ConfigInvalid);
    assert_eq!(
        e.issues[0].code,
        hacivat_session::pipeline::IssueCode::PathRefused
    );
    assert_eq!(s.sessions_accepted(), 0, "nothing connected");
}

/// The hop prompts reach the UI as `Question::HostKey` and `Question::Secret` for hop 0.
#[tokio::test]
async fn hop_prompts_name_the_hop() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Secret(sec("pw"))]);
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    connect(&deps_fake(&f), req(p), ui.ix(), token(), progress())
        .await
        .unwrap();
    let asked = ui.asked();
    assert!(matches!(&asked[0], Question::HostKey { hop: 0, key, .. } if *key == s.host_key));
    assert!(matches!(
        &asked[1],
        Question::Secret {
            prompt: hacivat_session::pipeline::SecretPrompt::SshPassword { hop: 0, .. },
            attempt: 1,
            ..
        }
    ));
}

/// A headless reconnect to a password hop with no password anywhere (overlay, store) stops
/// with `needs_interaction` (Q17); a wrong password it was given does not (asking the user
/// cannot help a reconnect that already had one).
#[tokio::test]
async fn headless_without_a_hop_password_needs_interaction() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::AcceptNew,
    );
    let e = connect(&deps_fake(&f), req(p), ix_none(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::SshAuth);
    assert!(e.needs_interaction);

    let (a, b) = jump_pair().await;
    let e = connect_chain_bad_pw_at(&a, &b, 1).await.unwrap_err();
    assert!(!e.needs_interaction);
}

/// Fix round 1 (review M6): a refused channel names the hop that refused it, with its host.
#[tokio::test]
async fn a_refused_channel_names_its_hop() {
    let s = TestSshServer::start(
        cfg()
            .user_pw("u", "pw")
            .forward(hacivat_testkit::ssh_server::ForwardPolicy::Deny),
    );
    let f = Arc::new(FakeDriver::new(DbmsKind::Postgres).consumes_factory());
    let ui = ScriptedUi::answers_m2b([Answer::TrustHostKey, Answer::Secret(sec("pw"))]);
    let p = through(
        pg_auto("db.internal", 5432),
        vec![hop(&s, SshAuth::Password)],
        HostKeyPolicy::Ask,
    );
    let e = connect(&deps_fake(&f), req(p), ui.ix(), token(), progress())
        .await
        .unwrap_err();
    assert_eq!(e.class, ErrClass::SshChannelOpen);
    assert_eq!(e.args.hop, Some(0));
    assert_eq!(e.args.hop_host.as_deref(), Some(s.host().as_str()));
    assert_eq!(failed_step(e), TrailStep::Connect);
}
