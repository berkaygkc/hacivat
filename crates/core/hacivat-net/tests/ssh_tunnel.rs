//! The SSH tunnel manager (HV-SEC-01, B11, B20, Task 9) against the in-process testkit SSH
//! server and agent: host-key policies checked before any authentication byte, the
//! authentication order, jump chains, shared sessions, idle release, keep-alive death events and
//! channel hygiene (Q46). Every server and agent runs in this process on loopback; every
//! `known_hosts` file lives in a temp directory. Real sockets set the pace, so no test uses a
//! paused clock (Q25); waits are bounded condition polls.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_net::ssh::tunnel::classify_channel_err;
use hacivat_net::ssh::{
    HopSecrets, HopSpec, HostKey, HostKeyCheck, HostKeyPolicy, KbdRequest, KnownHosts, KnownSource,
    SshAuth, SshError, SshPrompter, SshSecretKind, SshSpec, TunnelConfig, TunnelEvent, TunnelLease,
    TunnelManager, TunnelRequest,
};
use hacivat_testkit::ssh_server::{ForwardPolicy, ForwardTarget, TestSshConfig, TestSshServer};
use russh::keys::PrivateKey;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::ssh_key::{Cipher, Kdf, LineEnding};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Time, servers and stores

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn tok() -> CancellationToken {
    CancellationToken::new()
}

fn cfg() -> TestSshConfig {
    TestSshConfig::default()
}

async fn ssh(config: TestSshConfig) -> TestSshServer {
    TestSshServer::start(config)
}

/// Polls `f` every 10 ms until it holds; callers bound it with `timeout`.
async fn wait_until(f: impl Fn() -> bool) {
    while !f() {
        tokio::time::sleep(ms(10)).await;
    }
}

/// Waits (bounded) until `f` holds, then asserts it.
async fn settle_until(what: &str, f: impl Fn() -> bool) {
    let _ = timeout(secs(5), wait_until(&f)).await;
    assert!(f(), "{what}");
}

/// One temp directory for every store of this test binary.
fn dir() -> &'static Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// The `known_hosts` of the manager built for `s` (one file per server port).
fn known_path(s: &TestSshServer) -> PathBuf {
    dir().join(format!("known_hosts-{}", s.addr.port()))
}

fn known_of(s: &TestSshServer) -> Arc<KnownHosts> {
    KnownHosts::load(Vec::new(), known_path(s))
}

fn host(s: &TestSshServer) -> String {
    s.host()
}

fn port(s: &TestSshServer) -> u16 {
    s.addr.port()
}

/// A store whose only line gives `s`'s host and port another key.
fn known_with(s: &TestSshServer, other: &HostKey) -> Arc<KnownHosts> {
    let path = dir().join(format!("known_other-{}", port(s)));
    let line = format!(
        "{} {}\n",
        hacivat_net::ssh::known_hosts::host_pattern(&host(s), port(s)),
        other.openssh
    );
    std::fs::write(&path, line).unwrap();
    KnownHosts::load(vec![path], dir().join(format!("known_own-{}", port(s))))
}

fn known_empty() -> Arc<KnownHosts> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    KnownHosts::load(Vec::new(), dir().join(format!("known_empty-{n}")))
}

fn mgr_known(known: Arc<KnownHosts>, config: TunnelConfig) -> Arc<TunnelManager> {
    TunnelManager::with_agent(known, config, None)
}

/// A manager over `s`'s own empty store, without an agent.
fn mgr(s: &TestSshServer) -> Arc<TunnelManager> {
    mgr_known(known_of(s), TunnelConfig::default())
}

/// The manager's policy lives in the request; this only picks the store.
fn mgr_with(known: Arc<KnownHosts>, _policy: HostKeyPolicy) -> Arc<TunnelManager> {
    mgr_known(known, TunnelConfig::default())
}

/// 500 ms grace, 100 ms keep-alive (Q25: a short real grace, never a paused clock; M5: wide
/// enough for a starved runner).
fn mgr_fast(s: &TestSshServer) -> Arc<TunnelManager> {
    mgr_known(
        known_of(s),
        TunnelConfig {
            idle_grace: ms(500),
            keepalive: ms(100),
            keepalive_max: 2,
        },
    )
}

fn mgr_fast_keepalive(s: &TestSshServer) -> Arc<TunnelManager> {
    mgr_fast(s)
}

#[cfg(unix)]
fn mgr_agent(s: &TestSshServer, socket: &Path) -> Arc<TunnelManager> {
    TunnelManager::with_agent(
        known_of(s),
        TunnelConfig::default(),
        Some(hacivat_net::ssh::tunnel::AgentEndpoint::Unix(
            socket.to_path_buf(),
        )),
    )
}

// ---------------------------------------------------------------------------------------------
// Requests

fn hop(s: &TestSshServer, user: &str, auth: SshAuth) -> HopSpec {
    HopSpec {
        host: host(s),
        port: port(s),
        user: user.to_string(),
        auth,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

fn request(hops: Vec<HopSpec>, policy: HostKeyPolicy) -> TunnelRequest {
    TunnelRequest {
        spec: SshSpec {
            hops,
            host_keys: policy,
            keepalive_s: None,
        },
        secrets: Vec::new(),
        share: true,
        connect_timeout: secs(10),
    }
}

/// One hop to `s` as `u`, `Ask` policy, no secrets.
fn req(s: &TestSshServer, auth: SshAuth) -> TunnelRequest {
    request(vec![hop(s, "u", auth)], HostKeyPolicy::Ask)
}

/// One password hop with the password `pw` and the given policy.
fn req_policy(s: &TestSshServer, policy: HostKeyPolicy) -> TunnelRequest {
    request(vec![hop(s, "u", SshAuth::Password)], policy).secret_pw("pw")
}

/// One password hop as `user` with `pw`, `AcceptNew` (no prompt needed).
fn req_user(s: &TestSshServer, user: &str) -> TunnelRequest {
    request(
        vec![hop(s, user, SshAuth::Password)],
        HostKeyPolicy::AcceptNew,
    )
    .secret_pw("pw")
}

fn req_key(s: &TestSshServer, path: &Path) -> TunnelRequest {
    request(
        vec![hop(
            s,
            "u",
            SshAuth::KeyFile {
                path: path.to_path_buf(),
            },
        )],
        HostKeyPolicy::AcceptNew,
    )
}

/// Two password hops (`a` the jump host, `b` the bastion), `AcceptNew`, `pw` on both.
fn chain_req(a: &TestSshServer, b: &TestSshServer) -> TunnelRequest {
    let mut r = request(
        vec![
            hop(a, "u", SshAuth::Password),
            hop(b, "u", SshAuth::Password),
        ],
        HostKeyPolicy::AcceptNew,
    );
    r.secrets = (0..2).map(|_| secrets(Some("pw"), None)).collect();
    r
}

fn secrets(password: Option<&str>, passphrase: Option<&str>) -> HopSecrets {
    HopSecrets {
        password: password.map(SecretString::from),
        passphrase: passphrase.map(SecretString::from),
    }
}

trait ReqExt: Sized {
    fn secret_pw(self, pw: &str) -> Self;
    fn no_share(self) -> Self;
    fn identities_only(self, on: bool) -> Self;
    fn bad_password_at(self, hop: usize) -> Self;
    fn timeout(self, d: Duration) -> Self;
}

impl ReqExt for TunnelRequest {
    fn secret_pw(mut self, pw: &str) -> Self {
        self.secrets = vec![secrets(Some(pw), None)];
        self
    }

    fn no_share(mut self) -> Self {
        self.share = false;
        self
    }

    fn identities_only(mut self, on: bool) -> Self {
        for h in &mut self.spec.hops {
            h.identities_only = on;
        }
        self
    }

    fn bad_password_at(mut self, hop: usize) -> Self {
        self.secrets[hop] = secrets(Some("wrong"), None);
        self
    }

    fn timeout(mut self, d: Duration) -> Self {
        self.connect_timeout = d;
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Keys

/// A fresh Ed25519 key (seeded from a counter: test keys need no real entropy).
fn ed25519() -> PrivateKey {
    static N: AtomicUsize = AtomicUsize::new(1);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&(n as u64).to_le_bytes());
    seed[8..16].copy_from_slice(&u64::from(std::process::id()).to_le_bytes());
    seed[31] = 0x5a;
    PrivateKey::from(Ed25519Keypair::from_seed(&seed))
}

static KEY_A: LazyLock<PrivateKey> = LazyLock::new(ed25519);
static KEY_A_FILE: LazyLock<PathBuf> = LazyLock::new(|| write_key(&KEY_A, None, "key_a"));

/// Writes `key` in OpenSSH form (encrypted with `passphrase` when given), mode `0600`.
fn write_key(key: &PrivateKey, passphrase: Option<&str>, name: &str) -> PathBuf {
    let key = match passphrase {
        Some(p) => key
            .encrypt_with(
                Cipher::Aes256Ctr,
                Kdf::Bcrypt {
                    salt: vec![7; 16],
                    rounds: 2,
                },
                0x1234_5678,
                p,
            )
            .unwrap(),
        None => key.clone(),
    };
    let path = dir().join(name);
    std::fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    path
}

/// A host key of `s`'s type that is not `s`'s.
static OTHER_KEY: LazyLock<HostKey> =
    LazyLock::new(|| HostKey::from_openssh(&ed25519().public_key().to_openssh().unwrap()).unwrap());

/// A syntactically valid ECDSA blob (`known_hosts` checks compare types and blobs only).
fn fake_ecdsa_key() -> HostKey {
    use base64::Engine as _;
    let mut blob = Vec::new();
    for part in [&b"ecdsa-sha2-nistp256"[..], b"nistp256", &[4u8; 65][..]] {
        blob.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
        blob.extend_from_slice(part);
    }
    let line = format!(
        "ecdsa-sha2-nistp256 {}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    );
    HostKey::from_openssh(&line).unwrap()
}

// ---------------------------------------------------------------------------------------------
// A scripted prompter

#[derive(Default)]
struct TestPrompter {
    trusts: AtomicBool,
    host_key_delay: Mutex<Option<Duration>>,
    host_key_never: AtomicBool,
    passphrases: Mutex<VecDeque<String>>,
    keyboard: Mutex<VecDeque<String>>,
    host_key_asks: AtomicUsize,
    secret_asks: Mutex<Vec<(u8, &'static str)>>,
    kbd_requests: Mutex<Vec<KbdRequest>>,
}

fn prompter() -> Arc<TestPrompter> {
    Arc::new(TestPrompter::default())
}

trait PrompterExt {
    fn trusts(self) -> Self;
    fn passphrases<const N: usize>(self, p: [&str; N]) -> Self;
    fn keyboard<const N: usize>(self, a: [&str; N]) -> Self;
    fn slow_host_key(self, d: Duration) -> Self;
    fn never_answers(self) -> Self;
    fn dyn_(&self) -> Option<Arc<dyn SshPrompter>>;
}

impl PrompterExt for Arc<TestPrompter> {
    fn trusts(self) -> Self {
        self.trusts.store(true, Ordering::SeqCst);
        self
    }

    fn passphrases<const N: usize>(self, p: [&str; N]) -> Self {
        self.passphrases
            .lock()
            .unwrap()
            .extend(p.iter().map(|s| s.to_string()));
        self
    }

    fn keyboard<const N: usize>(self, a: [&str; N]) -> Self {
        self.keyboard
            .lock()
            .unwrap()
            .extend(a.iter().map(|s| s.to_string()));
        self
    }

    fn slow_host_key(self, d: Duration) -> Self {
        *self.host_key_delay.lock().unwrap() = Some(d);
        self
    }

    fn never_answers(self) -> Self {
        self.host_key_never.store(true, Ordering::SeqCst);
        self
    }

    fn dyn_(&self) -> Option<Arc<dyn SshPrompter>> {
        Some(self.clone())
    }
}

impl TestPrompter {
    fn host_key_asks(&self) -> usize {
        self.host_key_asks.load(Ordering::SeqCst)
    }

    fn secret_asks(&self) -> Vec<(u8, &'static str)> {
        self.secret_asks.lock().unwrap().clone()
    }

    fn kbd_requests(&self) -> Vec<KbdRequest> {
        self.kbd_requests.lock().unwrap().clone()
    }
}

impl SshPrompter for TestPrompter {
    fn host_key(
        &self,
        _hop: u8,
        _host: &str,
        _port: u16,
        _key: &HostKey,
    ) -> BoxFuture<'static, bool> {
        self.host_key_asks.fetch_add(1, Ordering::SeqCst);
        let trusts = self.trusts.load(Ordering::SeqCst);
        let delay = *self.host_key_delay.lock().unwrap();
        let never = self.host_key_never.load(Ordering::SeqCst);
        Box::pin(async move {
            if never {
                std::future::pending::<()>().await;
            }
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            trusts
        })
    }

    fn secret(
        &self,
        hop: u8,
        kind: SshSecretKind,
        _attempt: u8,
    ) -> BoxFuture<'static, Option<SecretString>> {
        let name = match kind {
            SshSecretKind::Password { .. } => "password",
            SshSecretKind::Passphrase { .. } => "passphrase",
        };
        self.secret_asks.lock().unwrap().push((hop, name));
        let answer = self.passphrases.lock().unwrap().pop_front();
        Box::pin(async move { answer.map(SecretString::from) })
    }

    fn keyboard(
        &self,
        _hop: u8,
        req: KbdRequest,
        _attempt: u8,
    ) -> BoxFuture<'static, Option<Vec<SecretString>>> {
        let n = req.prompts.len();
        self.kbd_requests.lock().unwrap().push(req);
        let mut queue = self.keyboard.lock().unwrap();
        let answers: Option<Vec<SecretString>> = (0..n)
            .map(|_| queue.pop_front().map(SecretString::from))
            .collect();
        Box::pin(async move { answers })
    }
}

// ---------------------------------------------------------------------------------------------
// Targets, streams and logs

/// A TCP echo server on loopback.
async fn echo_target() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    addr
}

/// Sends `ping` through a `direct-tcpip` channel to `t` and returns what comes back.
async fn echo_through(lease: &TunnelLease, t: SocketAddr) -> Vec<u8> {
    let mut stream = lease.factory("127.0.0.1", t.port()).open().await.unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    timeout(secs(5), stream.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    buf.to_vec()
}

/// Every `log` record (russh logs through `log`; `hacivat-net` itself has no logger) at any
/// level, on any thread, while `f`'s future runs. Global: installed once per test binary.
async fn capture_tracing<F, Fut, T>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = T>,
{
    use tracing_log::log;
    struct Capture(Mutex<String>);
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            let line = format!("{} {} {}\n", record.level(), record.target(), record.args());
            self.0.lock().unwrap().push_str(&line);
        }
        fn flush(&self) {}
    }
    static LOGS: OnceLock<&'static Capture> = OnceLock::new();
    let logs = LOGS.get_or_init(|| {
        let capture: &'static Capture = Box::leak(Box::new(Capture(Mutex::new(String::new()))));
        log::set_logger(capture).expect("no other logger in this test binary");
        log::set_max_level(log::LevelFilter::Trace);
        capture
    });
    let _ = f().await;
    logs.0.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------------------------
// The brief's tests (Task 9 Step 1)

#[tokio::test]
async fn password_auth_and_direct_tcpip_relays() {
    let t = echo_target().await;
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter().trusts();
    let lease = mgr(&s)
        .acquire(req(&s, SshAuth::Password).secret_pw("pw"), p.dyn_(), tok())
        .await
        .unwrap();
    assert_eq!(echo_through(&lease, t).await, b"ping");
    assert_eq!(lease.hops(), 1);
    assert_eq!(lease.bastion(), "127.0.0.1");
    assert_eq!(s.channels_opened(), 1);
}

#[tokio::test]
async fn host_key_is_checked_before_any_auth() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let r = req_policy(&s, HostKeyPolicy::Strict);
    let e = mgr(&s).acquire(r, None, tok()).await.unwrap_err();
    assert!(
        matches!(e, SshError::HostKeyUnknown { hop: 0, .. }),
        "{e:?}"
    );
    assert_eq!(s.auth_attempts(), 0);
    // Ask without a prompter refuses the same way, and sends nothing either.
    let e = mgr(&s)
        .acquire(req(&s, SshAuth::Password).secret_pw("pw"), None, tok())
        .await
        .unwrap_err();
    assert!(
        matches!(e, SshError::HostKeyUnknown { hop: 0, .. }),
        "{e:?}"
    );
    assert_eq!(s.auth_attempts(), 0);
}

#[tokio::test]
async fn unknown_host_key_asks_in_ask_policy_and_trust_stores() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter().trusts();
    mgr(&s)
        .acquire(req(&s, SshAuth::Password).secret_pw("pw"), p.dyn_(), tok())
        .await
        .unwrap();
    assert_eq!(p.host_key_asks(), 1);
    assert!(matches!(
        known_of(&s).check(&host(&s), port(&s), &s.host_key),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
}

#[tokio::test]
async fn declined_host_key_is_unknown_and_never_stored() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter();
    let e = mgr(&s)
        .acquire(req(&s, SshAuth::Password).secret_pw("pw"), p.dyn_(), tok())
        .await
        .unwrap_err();
    assert!(
        matches!(e, SshError::HostKeyUnknown { hop: 0, .. }),
        "{e:?}"
    );
    assert_eq!(p.host_key_asks(), 1);
    assert_eq!(s.auth_attempts(), 0);
    assert_eq!(
        known_of(&s).check(&host(&s), port(&s), &s.host_key),
        HostKeyCheck::Unknown
    );
}

#[tokio::test]
async fn accept_new_stores_without_prompt() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter();
    let known = known_empty();
    let lease = mgr_with(known.clone(), HostKeyPolicy::AcceptNew)
        .acquire(req_policy(&s, HostKeyPolicy::AcceptNew), p.dyn_(), tok())
        .await
        .unwrap();
    assert_eq!(p.host_key_asks(), 0);
    assert!(matches!(
        known.check(&host(&s), port(&s), &s.host_key),
        HostKeyCheck::Known(KnownSource::Hacivat)
    ));
    // The caller logs what was trusted without a question (hacivat-net has no logger).
    assert_eq!(lease.trusted_keys(), [(0, s.host_key.clone())]);
}

#[tokio::test]
async fn changed_key_is_never_accepted_in_any_policy() {
    for policy in [
        HostKeyPolicy::Ask,
        HostKeyPolicy::AcceptNew,
        HostKeyPolicy::Strict,
    ] {
        let s = ssh(cfg().user_pw("u", "pw")).await;
        let known = known_with(&s, &OTHER_KEY);
        let p = prompter().trusts();
        let e = mgr_with(known, policy)
            .acquire(req_policy(&s, policy), p.dyn_(), tok())
            .await
            .unwrap_err();
        assert!(
            matches!(e, SshError::HostKeyChanged { hop: 0, .. }),
            "{policy:?}: {e:?}"
        );
        assert_eq!(p.host_key_asks(), 0, "{policy:?}");
        assert_eq!(s.auth_attempts(), 0, "{policy:?}");
    }
}

#[tokio::test]
async fn encrypted_key_without_passphrase_prompts_per_hop() {
    let ka = ed25519();
    let kb = ed25519();
    let a = ssh(cfg().user_key("u", ka.public_key().clone())).await;
    let b = ssh(cfg().user_key("u", kb.public_key().clone())).await;
    let fa = write_key(&ka, Some("pa"), &format!("enc_a-{}", port(&a)));
    let fb = write_key(&kb, Some("pb"), &format!("enc_b-{}", port(&b)));
    let r = request(
        vec![
            hop(&a, "u", SshAuth::KeyFile { path: fa }),
            hop(&b, "u", SshAuth::KeyFile { path: fb }),
        ],
        HostKeyPolicy::Ask,
    );
    let p = prompter().passphrases(["pa", "pb"]).trusts();
    let lease = mgr(&a).acquire(r, p.dyn_(), tok()).await.unwrap();
    assert_eq!(p.secret_asks(), [(0, "passphrase"), (1, "passphrase")]);
    assert_eq!(p.host_key_asks(), 2);
    assert_eq!(lease.hops(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn agent_auth_with_in_process_agent() {
    use hacivat_testkit::agent::TestAgent;
    let k = ed25519();
    let agent = TestAgent::start(vec![k.clone()]).await;
    let s = ssh(cfg().user_key("u", k.public_key().clone())).await;
    let mut r = req(&s, SshAuth::Agent);
    r.spec.host_keys = HostKeyPolicy::AcceptNew;
    assert!(
        mgr_agent(&s, &agent.socket)
            .acquire(r, None, tok())
            .await
            .is_ok()
    );
}

#[cfg(unix)]
async fn agent_with_wrong_keys(n: usize) -> hacivat_testkit::agent::TestAgent {
    hacivat_testkit::agent::TestAgent::start((0..n).map(|_| ed25519()).collect()).await
}

#[cfg(unix)]
#[tokio::test]
async fn identities_only_tries_the_key_file_first() {
    let s = ssh(cfg()
        .user_key("u", KEY_A.public_key().clone())
        .max_auth_tries(1))
    .await;
    let agent = agent_with_wrong_keys(5).await;
    let r = req_key(&s, &KEY_A_FILE).identities_only(true);
    assert!(
        mgr_agent(&s, &agent.socket)
            .acquire(r, None, tok())
            .await
            .is_ok()
    );
    // Without IdentitiesOnly the agent goes first and exhausts MaxAuthTries.
    let r = req_key(&s, &KEY_A_FILE).identities_only(false).no_share();
    let e = mgr_agent(&s, &agent.socket)
        .acquire(r, None, tok())
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 0, .. }), "{e:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn many_agent_keys_without_identities_only_reports_auth_with_hop() {
    // MaxAuthTries (Q33)
    let s = ssh(cfg()
        .user_key("u", KEY_A.public_key().clone())
        .max_auth_tries(2))
    .await;
    let agent = agent_with_wrong_keys(5).await;
    let mut r = req(&s, SshAuth::Agent);
    r.spec.host_keys = HostKeyPolicy::AcceptNew;
    let e = mgr_agent(&s, &agent.socket)
        .acquire(r, None, tok())
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 0, .. }), "{e:?}");
}

#[tokio::test]
async fn agent_auth_without_an_agent_is_agent_unavailable() {
    let s = ssh(cfg().user_key("u", KEY_A.public_key().clone())).await;
    let mut r = req(&s, SshAuth::Agent);
    r.spec.host_keys = HostKeyPolicy::AcceptNew;
    let e = mgr(&s).acquire(r, None, tok()).await.unwrap_err();
    assert_eq!(e, SshError::AgentUnavailable { hop: 0 });
}

#[tokio::test]
async fn keyboard_interactive_prompts_are_forwarded_verbatim() {
    let s = ssh(cfg().user_kbd("u", &[("Doğrulama kodu: ", false, "123456")])).await;
    let p = prompter().keyboard(["123456"]).trusts();
    mgr(&s)
        .acquire(req(&s, SshAuth::KeyboardInteractive), p.dyn_(), tok())
        .await
        .unwrap();
    assert_eq!(p.kbd_requests()[0].prompts[0].text, "Doğrulama kodu: ");
    assert!(!p.kbd_requests()[0].prompts[0].echo);
    assert_eq!(p.kbd_requests()[0].host, "127.0.0.1");
}

#[tokio::test]
async fn keyboard_interactive_gives_up_after_three_attempts() {
    let s = ssh(cfg().user_kbd("u", &[("Code: ", false, "123456")])).await;
    let p = prompter().keyboard(["1", "2", "3", "4"]).trusts();
    let e = mgr(&s)
        .acquire(req(&s, SshAuth::KeyboardInteractive), p.dyn_(), tok())
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 0, .. }), "{e:?}");
    assert_eq!(p.kbd_requests().len(), 3);
}

#[tokio::test]
async fn wrong_stored_password_prompts_then_gives_up_after_three() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter().passphrases(["x", "y", "z"]).trusts();
    let e = mgr(&s)
        .acquire(
            req(&s, SshAuth::Password).secret_pw("wrong"),
            p.dyn_(),
            tok(),
        )
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 0, .. }), "{e:?}");
    // The stored password is attempt 1; two prompts make three.
    assert_eq!(p.secret_asks(), [(0, "password"), (0, "password")]);
}

#[tokio::test]
async fn cancelled_password_prompt_is_cancelled() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter().trusts();
    let e = mgr(&s)
        .acquire(req(&s, SshAuth::Password), p.dyn_(), tok())
        .await
        .unwrap_err();
    assert_eq!(e, SshError::Cancelled);
}

#[tokio::test]
async fn two_leases_same_chain_share_one_session_and_different_user_does_not() {
    let s = ssh(cfg().user_pw("u", "pw").user_pw("v", "pw")).await;
    let m = mgr(&s);
    let a = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    let b = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    assert_eq!(s.sessions_open(), 1);
    assert_eq!(a.generation(), b.generation());
    let _c = m.acquire(req_user(&s, "v"), None, tok()).await.unwrap();
    assert_eq!(s.sessions_open(), 2);
    assert_eq!(m.sessions_open(), 2);
}

#[tokio::test]
async fn test_connection_never_shares() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr(&s);
    let _a = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    let t = m
        .acquire(req_user(&s, "u").no_share(), None, tok())
        .await
        .unwrap();
    assert_eq!(s.sessions_open(), 2);
    // An unshared chain is never reused, and closes as soon as its lease goes.
    drop(t);
    settle_until("the unshared session closed", || s.sessions_open() == 1).await;
    let _b = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    assert_eq!(s.sessions_accepted(), 2);
}

#[tokio::test]
async fn idle_session_closes_after_grace() {
    // real time (Q25): never start_paused with real sockets
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr_fast(&s); // TunnelConfig { idle_grace: ms(500), keepalive: ms(100), .. }
    drop(m.acquire(req_user(&s, "u"), None, tok()).await.unwrap());
    tokio::time::sleep(ms(100)).await;
    assert_eq!(m.sessions_open(), 1);
    timeout(secs(3), wait_until(|| m.sessions_open() == 0))
        .await
        .unwrap();
    settle_until("the server saw the close", || s.sessions_open() == 0).await;
}

#[tokio::test]
async fn reacquire_within_grace_reuses_the_session() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr_fast(&s);
    let first = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    let g = first.generation();
    drop(first);
    let again = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    assert_eq!(again.generation(), g);
    // The first release's timer must not close the re-used chain.
    tokio::time::sleep(ms(1000)).await;
    assert_eq!(m.sessions_open(), 1);
    assert_eq!(s.sessions_accepted(), 1);
}

async fn jump_pair() -> (TestSshServer, TestSshServer) {
    let a = ssh(cfg().user_pw("u", "pw")).await;
    let b = ssh(cfg().user_pw("u", "pw")).await;
    (a, b)
}

#[tokio::test]
async fn jump_chain_relays_through_both_hops() {
    let t = echo_target().await;
    let (a, b) = jump_pair().await;
    let lease = mgr(&a)
        .acquire(chain_req(&a, &b), None, tok())
        .await
        .unwrap();
    assert_eq!(echo_through(&lease, t).await, b"ping");
    assert_eq!(lease.hops(), 2);
    assert_eq!(lease.bastion(), "127.0.0.1");
    assert_eq!(a.channels_opened(), 1, "the jump host carries hop 1");
    assert_eq!(b.channels_opened(), 1, "the bastion carries the database");
}

#[tokio::test]
async fn partial_chain_failure_releases_in_reverse() {
    let (a, b) = jump_pair().await;
    let e = mgr(&a)
        .acquire(chain_req(&a, &b).bad_password_at(1), None, tok())
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Auth { hop: 1, .. }), "{e:?}");
    settle_until("hop 0 released", || a.sessions_open() == 0).await;
    settle_until("hop 1 released", || b.sessions_open() == 0).await;
}

#[tokio::test]
async fn jump_hop_host_key_is_checked_before_its_auth() {
    let (a, b) = jump_pair().await;
    let known = known_with(&b, &OTHER_KEY);
    let e = mgr_known(known, TunnelConfig::default())
        .acquire(chain_req(&a, &b), None, tok())
        .await
        .unwrap_err();
    assert!(
        matches!(e, SshError::HostKeyChanged { hop: 1, .. }),
        "{e:?}"
    );
    assert_eq!(b.auth_attempts(), 0);
    settle_until("hop 0 released", || a.sessions_open() == 0).await;
}

#[tokio::test]
async fn forwarding_refused_names_the_hop() {
    let s = ssh(cfg().user_pw("u", "pw").forward(ForwardPolicy::Deny)).await;
    let lease = mgr(&s)
        .acquire(req_user(&s, "u"), None, tok())
        .await
        .unwrap();
    let e = lease.factory("db", 5432).open().await.err().unwrap();
    // the classified error only (Q32)
    assert!(
        matches!(
            classify_channel_err(&e),
            Some(SshError::ChannelOpen { hop: 0, .. })
        ),
        "{e:?}"
    );
}

#[tokio::test]
async fn database_host_is_resolved_by_the_bastion() {
    let t = echo_target().await;
    let s = ssh(cfg()
        .user_pw("u", "pw")
        .forward_to(ForwardTarget::Redirect(t)))
    .await;
    let lease = mgr(&s)
        .acquire(req_user(&s, "u"), None, tok())
        .await
        .unwrap();
    let mut stream = lease.factory("db.internal", 5432).open().await.unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
}

#[tokio::test]
async fn server_initiated_channels_are_refused() {
    // Q46
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let _lease = mgr(&s)
        .acquire(req_user(&s, "u"), None, tok())
        .await
        .unwrap();
    assert_eq!(s.sessions_open(), 1);
    let e = s.open_server_channel().await.unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e:?}");
}

#[tokio::test]
async fn direct_tcpip_originator_is_loopback_zero() {
    // Q46
    let t = echo_target().await;
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let lease = mgr(&s)
        .acquire(req_user(&s, "u"), None, tok())
        .await
        .unwrap();
    let _ = echo_through(&lease, t).await;
    assert_eq!(s.last_originator(), Some(("127.0.0.1".into(), 0)));
}

#[tokio::test]
async fn keepalive_failure_emits_dead_once_with_generation() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr_fast_keepalive(&s);
    let mut ev = m.events();
    let lease = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    let g = lease.generation();
    s.kill_sessions();
    assert_eq!(
        timeout(secs(5), ev.recv()).await.unwrap().unwrap(),
        TunnelEvent::Dead {
            key: g.key,
            generation: g.generation
        }
    );
    assert!(timeout(ms(300), ev.recv()).await.is_err());
    // The dead chain is never handed out again: the next lease is a new generation.
    let next = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    assert_eq!(next.generation().key, g.key);
    assert_eq!(next.generation().generation, g.generation + 1);
    // A released lease is not a death.
    drop(lease);
    drop(next);
    assert!(timeout(ms(500), ev.recv()).await.is_err());
}

#[tokio::test]
async fn ipv6_bastion() {
    let s = ssh(cfg().user_pw("u", "pw").ipv6()).await;
    assert_eq!(s.host(), "::1");
    assert!(
        mgr(&s)
            .acquire(req_user(&s, "u"), None, tok())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn cancel_during_handshake_releases_everything() {
    let s = ssh(cfg().user_pw("u", "pw").slow_banner(ms(500))).await;
    let t = tok();
    let h = tokio::spawn(mgr(&s).acquire(req_user(&s, "u"), None, t.clone()));
    tokio::time::sleep(ms(50)).await;
    t.cancel();
    assert!(matches!(h.await.unwrap(), Err(SshError::Cancelled)));
    settle_until("the server saw the close", || s.sessions_open() == 0).await;
}

#[tokio::test]
async fn cancel_during_host_key_prompt_releases_everything() {
    // A prompter that never answers must not hold the session (per-acquire, never global).
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr(&s);
    let p = prompter().never_answers();
    let t = tok();
    let h = tokio::spawn(m.acquire(
        req(&s, SshAuth::Password).secret_pw("pw"),
        p.dyn_(),
        t.clone(),
    ));
    timeout(secs(5), wait_until(|| p.host_key_asks() == 1))
        .await
        .unwrap();
    // Another profile's tunnel to another server is not blocked meanwhile.
    let other = ssh(cfg().user_pw("u", "pw")).await;
    assert!(m.acquire(req_user(&other, "u"), None, tok()).await.is_ok());
    t.cancel();
    assert!(matches!(h.await.unwrap(), Err(SshError::Cancelled)));
    settle_until("the server saw the close", || s.sessions_open() == 0).await;
    assert_eq!(s.auth_attempts(), 0);
}

#[tokio::test]
async fn prompt_time_never_counts_toward_the_connect_timeout() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let p = prompter().trusts().slow_host_key(ms(2000));
    let r = req(&s, SshAuth::Password).secret_pw("pw").timeout(ms(1000));
    assert!(mgr(&s).acquire(r, p.dyn_(), tok()).await.is_ok());
}

#[tokio::test]
async fn slow_server_times_out_with_the_hop() {
    let s = ssh(cfg().user_pw("u", "pw").slow_banner(secs(3))).await;
    let r = req_user(&s, "u").timeout(ms(200));
    let e = mgr(&s).acquire(r, None, tok()).await.unwrap_err();
    assert_eq!(e, SshError::Timeout { hop: 0 });
}

#[tokio::test]
async fn ssh_secrets_never_logged() {
    let s = ssh(cfg().user_pw("u", "s3cret-ssh")).await;
    let logs = capture_tracing(|| async {
        let p = prompter().trusts();
        mgr(&s)
            .acquire(
                req(&s, SshAuth::Password).secret_pw("s3cret-ssh"),
                p.dyn_(),
                tok(),
            )
            .await
            .unwrap();
    })
    .await;
    assert!(!logs.is_empty(), "the capture sees russh's log records");
    assert!(!logs.contains("s3cret-ssh"));
}

// ---------------------------------------------------------------------------------------------
// Carry-ins and review probes

/// T4 carry-in: a host known only by a key of another type is never "new": `AcceptNew` must
/// not accept (and store) a key of a type the stores do not list for it.
#[tokio::test]
async fn key_of_another_type_than_the_known_one_is_changed_not_new() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let known = known_with(&s, &fake_ecdsa_key());
    for policy in HostKeyPolicy::ALL {
        let e = mgr_known(known.clone(), TunnelConfig::default())
            .acquire(req_policy(&s, policy), prompter().trusts().dyn_(), tok())
            .await
            .unwrap_err();
        assert!(
            matches!(&e, SshError::HostKeyChanged { hop: 0, new, .. } if *new == s.host_key),
            "{policy:?}: {e:?}"
        );
    }
    assert_eq!(s.auth_attempts(), 0);
    assert_eq!(
        known_of(&s).check(&host(&s), port(&s), &s.host_key),
        HostKeyCheck::Unknown,
        "nothing was trusted"
    );
}

#[tokio::test]
async fn revoked_key_is_refused_in_every_policy() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let path = dir().join(format!("known_revoked-{}", port(&s)));
    std::fs::write(
        &path,
        format!(
            "@revoked {} {}\n",
            hacivat_net::ssh::known_hosts::host_pattern(&host(&s), port(&s)),
            s.host_key.openssh
        ),
    )
    .unwrap();
    let known = KnownHosts::load(
        vec![path],
        dir().join(format!("known_rev_own-{}", port(&s))),
    );
    for policy in HostKeyPolicy::ALL {
        let e = mgr_known(known.clone(), TunnelConfig::default())
            .acquire(req_policy(&s, policy), prompter().trusts().dyn_(), tok())
            .await
            .unwrap_err();
        assert!(
            matches!(e, SshError::HostKeyRevoked { hop: 0, .. }),
            "{policy:?}: {e:?}"
        );
    }
    assert_eq!(s.auth_attempts(), 0);
}

#[tokio::test]
async fn a_chain_longer_than_nine_hops_is_refused() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let hops = (0..10).map(|_| hop(&s, "u", SshAuth::Password)).collect();
    let e = mgr(&s)
        .acquire(request(hops, HostKeyPolicy::AcceptNew), None, tok())
        .await
        .unwrap_err();
    assert!(matches!(e, SshError::Protocol { .. }), "{e:?}");
    assert_eq!(s.sessions_accepted(), 0);
}

#[tokio::test]
async fn a_factory_outliving_its_lease_fails_cleanly() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr(&s);
    let lease = m
        .acquire(req_user(&s, "u").no_share(), None, tok())
        .await
        .unwrap();
    let factory = lease.factory("127.0.0.1", 9);
    drop(lease);
    settle_until("released", || m.sessions_open() == 0).await;
    let e = factory.open().await.err().unwrap();
    assert_eq!(e.kind(), io::ErrorKind::NotConnected);
}

#[tokio::test]
async fn errors_name_the_hop_and_never_a_secret() {
    let e = SshError::Auth {
        hop: 2,
        user: "u".into(),
        host: "bastion".into(),
    };
    let text = e.to_string();
    assert!(text.contains("hop 2"), "{text}");
    assert!(text.contains("bastion"), "{text}");
    let secret = secrets(Some("s3cret"), Some("s3cret"));
    assert!(!format!("{secret:?}").contains("s3cret"));
    assert!(
        secret
            .password
            .is_some_and(|p| !p.expose_secret().is_empty())
    );
}

// ---------------------------------------------------------------------------------------------
// Fix round 1

/// A server that sends its banner, then reads (and never answers) until the client goes: a
/// peer that stalls inside the key exchange (I1).
struct Stall {
    addr: SocketAddr,
    read: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
}

async fn stall_server() -> Stall {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let read = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let (r, c) = (read.clone(), closed.clone());
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (r, c) = (r.clone(), c.clone());
            tokio::spawn(async move {
                let _ = sock.write_all(b"SSH-2.0-Stall\r\n").await;
                let mut buf = [0u8; 4096];
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            r.fetch_add(n, Ordering::SeqCst);
                        }
                    }
                }
                c.fetch_add(1, Ordering::SeqCst);
            });
        }
    });
    Stall { addr, read, closed }
}

fn stall_hop(st: &Stall) -> HopSpec {
    HopSpec {
        host: "127.0.0.1".into(),
        port: st.addr.port(),
        user: "u".into(),
        auth: SshAuth::Password,
        identities_only: false,
        identity_files: Vec::new(),
    }
}

/// The client's id and KEXINIT have reached the stalling server: its session task is running.
async fn in_kex(st: &Stall) {
    timeout(secs(5), wait_until(|| st.read.load(Ordering::SeqCst) > 100))
        .await
        .unwrap();
}

/// h1 (review): a timeout inside a stalled key exchange ends the session and its socket.
#[tokio::test]
async fn a_key_exchange_that_stalls_is_released_on_timeout() {
    let st = stall_server().await;
    let m = mgr_known(known_empty(), TunnelConfig::default());
    let mut r = request(vec![stall_hop(&st)], HostKeyPolicy::AcceptNew).secret_pw("pw");
    r.connect_timeout = ms(300);
    let e = m.acquire(r, None, tok()).await.unwrap_err();
    assert_eq!(e, SshError::Timeout { hop: 0 });
    settle_until("no session left", || m.sessions_open() == 0).await;
    settle_until("the server saw the close", || {
        st.closed.load(Ordering::SeqCst) == 1
    })
    .await;
}

/// h2 (review): a cancel inside a stalled key exchange ends the session and its socket.
#[tokio::test]
async fn a_key_exchange_that_stalls_is_released_on_cancel() {
    let st = stall_server().await;
    let m = mgr_known(known_empty(), TunnelConfig::default());
    let r = request(vec![stall_hop(&st)], HostKeyPolicy::AcceptNew).secret_pw("pw");
    let t = tok();
    let h = tokio::spawn(m.acquire(r, None, t.clone()));
    in_kex(&st).await;
    t.cancel();
    assert_eq!(h.await.unwrap().unwrap_err(), SshError::Cancelled);
    settle_until("no session left", || m.sessions_open() == 0).await;
    settle_until("the server saw the close", || {
        st.closed.load(Ordering::SeqCst) == 1
    })
    .await;
}

/// A jump chain whose hop 1 stalls in its key exchange: cancel (and timeout) close hop 1's
/// session and hop 0 behind it.
#[tokio::test]
async fn a_stalled_inner_hop_releases_the_hops_before_it() {
    for cancel in [true, false] {
        let st = stall_server().await;
        let a = ssh(cfg().user_pw("u", "pw")).await;
        let m = mgr_known(known_empty(), TunnelConfig::default());
        let mut r = request(
            vec![hop(&a, "u", SshAuth::Password), stall_hop(&st)],
            HostKeyPolicy::AcceptNew,
        );
        r.secrets = (0..2).map(|_| secrets(Some("pw"), None)).collect();
        r.connect_timeout = secs(2);
        let t = tok();
        let h = tokio::spawn(m.acquire(r, None, t.clone()));
        let e = if cancel {
            in_kex(&st).await;
            t.cancel();
            h.await.unwrap().unwrap_err()
        } else {
            timeout(secs(10), h).await.unwrap().unwrap().unwrap_err()
        };
        let want = if cancel {
            SshError::Cancelled
        } else {
            SshError::Timeout { hop: 1 }
        };
        assert_eq!(e, want);
        settle_until("no session left", || m.sessions_open() == 0).await;
        settle_until("hop 1's server saw the close", || {
            st.closed.load(Ordering::SeqCst) == 1
        })
        .await;
        settle_until("hop 0 released", || a.sessions_open() == 0).await;
    }
}

/// I2: a `direct-tcpip` channel the server confirms after the open timed out is closed, not
/// left open on the server.
#[tokio::test]
async fn a_channel_confirmed_after_the_timeout_is_closed() {
    let t = echo_target().await;
    let s = ssh(cfg().user_pw("u", "pw").forward_delay(ms(1500))).await;
    let m = mgr(&s);
    let lease = m
        .acquire(req_user(&s, "u").timeout(ms(500)), None, tok())
        .await
        .unwrap();
    let e = lease
        .factory("127.0.0.1", t.port())
        .open()
        .await
        .err()
        .unwrap();
    assert_eq!(classify_channel_err(&e), Some(SshError::Timeout { hop: 0 }));
    settle_until("the server confirmed it late", || s.channels_opened() == 1).await;
    settle_until("the late channel was closed", || s.channels_live() == 0).await;
    assert_eq!(s.sessions_open(), 1, "the session itself stays");
}

/// I3 (Q17): the typed password that authenticated is handed back once; a rejected one is not.
#[tokio::test]
async fn the_password_that_authenticated_is_handed_back_once() {
    let s = ssh(cfg().user_pw("u", "s3cret-typed")).await;
    let p = prompter()
        .passphrases(["wrong-typed", "s3cret-typed"])
        .trusts();
    let mut lease = mgr(&s)
        .acquire(req(&s, SshAuth::Password), p.dyn_(), tok())
        .await
        .unwrap();
    assert!(!format!("{lease:?}").contains("s3cret-typed"));
    let secrets = lease.take_secrets();
    assert_eq!(secrets.len(), 1);
    assert!(
        secrets[0]
            .password
            .as_ref()
            .is_some_and(|pw| pw.expose_secret() == "s3cret-typed")
    );
    assert!(secrets[0].passphrase.is_none());
    assert!(!format!("{secrets:?}").contains("s3cret-typed"));
    assert!(lease.take_secrets().is_empty(), "handed over once");
}

/// I3: secrets are indexed by hop: a stored password at hop 0, a typed passphrase at hop 1.
#[tokio::test]
async fn secrets_are_handed_back_by_hop() {
    let kb = ed25519();
    let a = ssh(cfg().user_pw("u", "pw")).await;
    let b = ssh(cfg().user_key("u", kb.public_key().clone())).await;
    let fb = write_key(&kb, Some("pb-typed"), &format!("enc_hop1-{}", port(&b)));
    let mut r = request(
        vec![
            hop(&a, "u", SshAuth::Password),
            hop(&b, "u", SshAuth::KeyFile { path: fb }),
        ],
        HostKeyPolicy::AcceptNew,
    );
    r.secrets = vec![secrets(Some("pw"), None), secrets(None, None)];
    let p = prompter().passphrases(["pb-typed"]);
    let mut lease = mgr(&a).acquire(r, p.dyn_(), tok()).await.unwrap();
    let got = lease.take_secrets();
    assert_eq!(got.len(), 2);
    assert!(
        got[0]
            .password
            .as_ref()
            .is_some_and(|s| s.expose_secret() == "pw")
    );
    assert!(got[0].passphrase.is_none());
    assert!(got[1].password.is_none());
    assert!(
        got[1]
            .passphrase
            .as_ref()
            .is_some_and(|s| s.expose_secret() == "pb-typed")
    );
}

/// I3: a lease that joined an open chain carries no secrets; keyboard-interactive answers
/// (one-time codes) are never handed back.
#[tokio::test]
async fn joined_leases_and_keyboard_answers_hand_back_nothing() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr(&s);
    let mut first = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    // m2b-t14 (review I4, named exception): a lease that joins the open chain gets the secrets
    // that authenticated it, once, for its connect's session overlay (Q17).
    let mut joined = m
        .acquire(
            request(
                vec![hop(&s, "u", SshAuth::Password)],
                HostKeyPolicy::AcceptNew,
            ),
            None,
            tok(),
        )
        .await
        .unwrap();
    let got = joined.take_secrets();
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].password.as_ref().map(|p| p.expose_secret()),
        Some("pw")
    );
    assert!(joined.take_secrets().is_empty(), "once");
    assert_eq!(first.take_secrets().len(), 1);

    let k = ssh(cfg().user_kbd("u", &[("Code: ", false, "123456")])).await;
    let p = prompter().keyboard(["123456"]).trusts();
    let mut lease = mgr(&k)
        .acquire(req(&k, SshAuth::KeyboardInteractive), p.dyn_(), tok())
        .await
        .unwrap();
    let got = lease.take_secrets();
    assert_eq!(got.len(), 1);
    assert!(got[0].password.is_none() && got[0].passphrase.is_none());
}

/// h3 (review, M1): a host listed only under a key type outside the probed list is not new:
/// `AcceptNew` must not store a key of another type for it.
#[tokio::test]
async fn a_host_listed_under_an_unprobed_type_is_changed_not_new() {
    use base64::Engine as _;
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let mut blob = Vec::new();
    for part in [&b"ssh-xmss@openssh.com"[..], &[9u8; 40][..]] {
        blob.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
        blob.extend_from_slice(part);
    }
    let xmss = HostKey::from_openssh(&format!(
        "ssh-xmss@openssh.com {}",
        base64::engine::general_purpose::STANDARD.encode(&blob)
    ))
    .unwrap();
    let known = known_with(&s, &xmss);
    let e = mgr_known(known.clone(), TunnelConfig::default())
        .acquire(req_policy(&s, HostKeyPolicy::AcceptNew), None, tok())
        .await
        .unwrap_err();
    assert!(
        matches!(&e, SshError::HostKeyChanged { hop: 0, old, new, .. } if *old == xmss && *new == s.host_key),
        "{e:?}"
    );
    assert_eq!(s.auth_attempts(), 0);
    assert!(!matches!(
        known.check(&host(&s), port(&s), &s.host_key),
        HostKeyCheck::Known(_)
    ));
}

/// M6: with IdentitiesOnly the agent offers only the configured identities' keys (OpenSSH), so
/// unrelated agent keys never burn MaxAuthTries.
#[cfg(unix)]
#[tokio::test]
async fn identities_only_offers_only_the_configured_agent_key() {
    let s = ssh(cfg()
        .user_key("u", KEY_A.public_key().clone())
        .max_auth_tries(2))
    .await;
    let mut keys: Vec<PrivateKey> = (0..5).map(|_| ed25519()).collect();
    keys.push(KEY_A.clone());
    let agent = hacivat_testkit::agent::TestAgent::start(keys).await;
    // The file is encrypted and nothing can unlock it: only the agent can sign for it.
    let locked = write_key(&KEY_A, Some("unknown"), &format!("locked_a-{}", port(&s)));
    let r = req_key(&s, &locked).identities_only(true);
    assert!(
        mgr_agent(&s, &agent.socket)
            .acquire(r, None, tok())
            .await
            .is_ok()
    );
}

/// M4: profiles that differ only in keep-alive do not share a chain.
#[tokio::test]
async fn keepalive_is_part_of_the_chain_key() {
    let s = ssh(cfg().user_pw("u", "pw")).await;
    let m = mgr(&s);
    let a = m.acquire(req_user(&s, "u"), None, tok()).await.unwrap();
    let mut r = req_user(&s, "u");
    r.spec.keepalive_s = Some(15);
    let b = m.acquire(r, None, tok()).await.unwrap();
    assert_ne!(a.generation().key, b.generation().key);
    assert_eq!(s.sessions_open(), 2);
}
