//! An in-process SSH server for tunnel tests (Task 9, Q23, Q46): password, public-key and
//! keyboard-interactive users, `direct-tcpip` forwarding (allowed, denied or redirected to one
//! address), and probes for the channel-hygiene tests.
//!
//! The server is russh's, on loopback (`127.0.0.1:0`, or `[::1]:0` with `ipv6()`), with a fresh
//! Ed25519 host key. It counts what a test asserts on: authentication requests of any method,
//! sessions open and accepted, and `direct-tcpip` channels opened. `max_auth_tries` follows
//! OpenSSH's `MaxAuthTries`: every rejected request counts, public-key probes included, and the
//! connection is dropped at the limit. Nothing here logs.
//!
//! `start` needs a running tokio runtime (it spawns the accept loop). Dropping the server stops
//! it and cuts every session.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hacivat_net::ssh::HostKey;
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PublicKey};
use russh::server::{Auth, Handler, Msg, PublicKeyOffer, Response, Session, VerifiedPublicKey};
use russh::{Channel, ChannelOpenFailure, MethodKind, MethodSet};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The methods the server offers, before and after a failure.
const OFFERED: &[MethodKind] = &[
    MethodKind::PublicKey,
    MethodKind::Password,
    MethodKind::KeyboardInteractive,
];

/// A boxed `Send` future, as `futures::future::BoxFuture` (the testkit has no `futures`).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Whether `direct-tcpip` channels are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ForwardPolicy {
    #[default]
    Allow,
    Deny,
}

/// Where an allowed `direct-tcpip` channel goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ForwardTarget {
    /// The host and port the client asked for.
    #[default]
    Any,
    /// Always this address (for names that do not resolve, such as `db.internal`).
    Redirect(SocketAddr),
}

/// One keyboard-interactive prompt and the answer the server expects. Holds a secret (the
/// answer), so `Debug` redacts it and there is no `PartialEq`.
#[derive(Clone)]
pub struct KbdStep {
    pub prompt: String,
    pub echo: bool,
    pub answer: String,
}

impl fmt::Debug for KbdStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KbdStep")
            .field("prompt", &self.prompt)
            .field("echo", &self.echo)
            .field("answer", &"<redacted>")
            .finish()
    }
}

/// A test user. Holds secrets, so `Debug` redacts them and there is no `PartialEq`.
#[derive(Clone)]
pub enum TestUser {
    Password { name: String, password: String },
    Key { name: String, key: PublicKey },
    Keyboard { name: String, steps: Vec<KbdStep> },
}

impl fmt::Debug for TestUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TestUser::Password { name, .. } => f
                .debug_struct("Password")
                .field("name", name)
                .field("password", &"<redacted>")
                .finish(),
            TestUser::Key { name, key } => f
                .debug_struct("Key")
                .field("name", name)
                .field("key", key)
                .finish(),
            TestUser::Keyboard { name, steps } => f
                .debug_struct("Keyboard")
                .field("name", name)
                .field("steps", steps)
                .finish(),
        }
    }
}

/// The server's configuration, built with the methods below. `Debug` redacts the users'
/// secrets.
#[derive(Debug, Clone, Default)]
pub struct TestSshConfig {
    pub users: Vec<TestUser>,
    pub forward: ForwardPolicy,
    pub forward_to: ForwardTarget,
    pub max_auth_tries: Option<u32>,
    pub slow_banner: Option<Duration>,
    /// How long an allowed `direct-tcpip` open waits before it is confirmed.
    pub forward_delay: Option<Duration>,
    pub ipv6: bool,
}

impl TestSshConfig {
    pub fn user_pw(mut self, name: &str, pw: &str) -> Self {
        self.users.push(TestUser::Password {
            name: name.to_string(),
            password: pw.to_string(),
        });
        self
    }

    pub fn user_key(mut self, name: &str, key: PublicKey) -> Self {
        self.users.push(TestUser::Key {
            name: name.to_string(),
            key,
        });
        self
    }

    /// A keyboard-interactive user: `(prompt, echo, answer)` per round.
    pub fn user_kbd(mut self, name: &str, steps: &[(&str, bool, &str)]) -> Self {
        self.users.push(TestUser::Keyboard {
            name: name.to_string(),
            steps: steps
                .iter()
                .map(|(prompt, echo, answer)| KbdStep {
                    prompt: (*prompt).to_string(),
                    echo: *echo,
                    answer: (*answer).to_string(),
                })
                .collect(),
        });
        self
    }

    pub fn forward(mut self, policy: ForwardPolicy) -> Self {
        self.forward = policy;
        self
    }

    pub fn forward_to(mut self, target: ForwardTarget) -> Self {
        self.forward_to = target;
        self
    }

    pub fn max_auth_tries(mut self, n: u32) -> Self {
        self.max_auth_tries = Some(n);
        self
    }

    /// The server waits `d` before its banner (timeout tests).
    /// Every allowed `direct-tcpip` open is confirmed `d` late (late-channel tests).
    pub fn forward_delay(mut self, d: Duration) -> Self {
        self.forward_delay = Some(d);
        self
    }

    pub fn slow_banner(mut self, d: Duration) -> Self {
        self.slow_banner = Some(d);
        self
    }

    /// Listen on `[::1]` instead of `127.0.0.1`.
    pub fn ipv6(mut self) -> Self {
        self.ipv6 = true;
        self
    }
}

/// A running test server.
pub struct TestSshServer {
    pub addr: SocketAddr,
    pub host_key: HostKey,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
}

impl fmt::Debug for TestSshServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestSshServer")
            .field("addr", &self.addr)
            .field("host_key", &self.host_key.fingerprint())
            .field("config", &self.shared.config)
            .finish_non_exhaustive()
    }
}

/// What the accept loop and every session share with the test.
struct Shared {
    config: TestSshConfig,
    auth_attempts: AtomicUsize,
    sessions_open: AtomicUsize,
    sessions_accepted: AtomicUsize,
    channels_opened: AtomicUsize,
    channels_live: AtomicUsize,
    next_id: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Per live connection: a duplicate of its socket (to cut it) and, once the SSH session
    /// runs, its handle (to open server channels).
    sessions: HashMap<u64, Live>,
    last_originator: Option<(String, u32)>,
}

struct Live {
    socket: std::net::TcpStream,
    handle: Option<russh::server::Handle>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TestSshServer {
    /// Starts the server on loopback. Needs a tokio runtime (it spawns the accept loop).
    pub fn start(config: TestSshConfig) -> TestSshServer {
        let bind: SocketAddr = if config.ipv6 {
            SocketAddr::from((Ipv6Addr::LOCALHOST, 0))
        } else {
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
        };
        let listener = std::net::TcpListener::bind(bind).expect("bind the test SSH server");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let addr = listener.local_addr().expect("the listener's address");
        let listener = TcpListener::from_std(listener).expect("a tokio runtime is running");

        let key = test_key();
        let host_key = HostKey::from_openssh(
            &key.public_key()
                .to_openssh()
                .expect("an Ed25519 key encodes"),
        )
        .expect("an OpenSSH public key parses");

        let server_config = Arc::new(russh::server::Config {
            keys: vec![key],
            methods: MethodSet::from(OFFERED),
            // The rejection delay is a timing defence; tests want it short.
            auth_rejection_time: Duration::from_millis(1),
            auth_rejection_time_initial: Some(Duration::from_millis(1)),
            // `MaxAuthTries` is counted by the handler (russh does not count key probes).
            max_auth_attempts: 0,
            inactivity_timeout: None,
            nodelay: true,
            ..Default::default()
        });

        let shared = Arc::new(Shared {
            config,
            auth_attempts: AtomicUsize::new(0),
            sessions_open: AtomicUsize::new(0),
            sessions_accepted: AtomicUsize::new(0),
            channels_opened: AtomicUsize::new(0),
            channels_live: AtomicUsize::new(0),
            next_id: AtomicU64::new(0),
            state: Mutex::new(State::default()),
        });
        let accept = tokio::spawn(accept_loop(listener, server_config, shared.clone()));
        TestSshServer {
            addr,
            host_key,
            shared,
            accept,
        }
    }

    /// The host as a profile names it (`127.0.0.1` or `::1`).
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// Authentication requests received, of any method (key probes included).
    pub fn auth_attempts(&self) -> usize {
        self.shared.auth_attempts.load(Ordering::SeqCst)
    }

    /// Connections the server holds now (from accept until the session ends).
    pub fn sessions_open(&self) -> usize {
        self.shared.sessions_open.load(Ordering::SeqCst)
    }

    /// Connections accepted since the start.
    pub fn sessions_accepted(&self) -> usize {
        self.shared.sessions_accepted.load(Ordering::SeqCst)
    }

    /// `direct-tcpip` channels the server opened (allowed and connected).
    pub fn channels_opened(&self) -> usize {
        self.shared.channels_opened.load(Ordering::SeqCst)
    }

    /// `direct-tcpip` channels open now (from confirmation until the relay ends, which is when
    /// the client closes the channel or the target closes its socket).
    pub fn channels_live(&self) -> usize {
        self.shared.channels_live.load(Ordering::SeqCst)
    }

    /// Drops every client session (a network cut): both directions of each socket shut down.
    pub fn kill_sessions(&self) {
        for live in self.shared.lock().sessions.values() {
            let _ = live.socket.shutdown(Shutdown::Both);
        }
    }

    /// Stops accepting and cuts every session.
    pub fn stop(&self) {
        self.accept.abort();
        self.kill_sessions();
    }

    /// Asks the client of the most recent session to open each kind of server-initiated
    /// channel (session, X11, agent, `forwarded-tcpip`, `direct-tcpip`). `Ok` when the client
    /// accepted any of them; `PermissionDenied` when it refused them all (Q46: it must);
    /// `NotConnected` without a session.
    pub fn open_server_channel(&self) -> BoxFuture<'static, io::Result<()>> {
        let handle = {
            let state = self.shared.lock();
            state
                .sessions
                .iter()
                .filter_map(|(id, live)| live.handle.clone().map(|h| (*id, h)))
                .max_by_key(|(id, _)| *id)
                .map(|(_, h)| h)
        };
        Box::pin(async move {
            let Some(h) = handle else {
                return Err(io::ErrorKind::NotConnected.into());
            };
            let accepted = h.channel_open_session().await.is_ok()
                | h.channel_open_x11("127.0.0.1", 6010).await.is_ok()
                | h.channel_open_agent().await.is_ok()
                | h.channel_open_forwarded_tcpip("127.0.0.1", 5432, "127.0.0.1", 40000)
                    .await
                    .is_ok()
                | h.channel_open_direct_tcpip("127.0.0.1", 5432, "127.0.0.1", 40000)
                    .await
                    .is_ok();
            if accepted {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "the client refused every server-initiated channel",
                ))
            }
        })
    }

    /// The originator address and port of the last `direct-tcpip` request (Q46).
    pub fn last_originator(&self) -> Option<(String, u32)> {
        self.shared.lock().last_originator.clone()
    }
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A fresh Ed25519 host key. Seeded from the clock, the process and a counter: a test host key
/// needs to differ between servers, not to resist an attacker.
fn test_key() -> PrivateKey {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut seed = [0u8; 32];
    seed[..16].copy_from_slice(&nanos.to_le_bytes());
    seed[16..24].copy_from_slice(&n.to_le_bytes());
    seed[24..28].copy_from_slice(&std::process::id().to_le_bytes());
    seed[31] = 0xa5;
    PrivateKey::from(Ed25519Keypair::from_seed(&seed))
}

async fn accept_loop(
    listener: TcpListener,
    config: Arc<russh::server::Config>,
    shared: Arc<Shared>,
) {
    while let Ok((socket, _)) = listener.accept().await {
        shared.sessions_accepted.fetch_add(1, Ordering::SeqCst);
        shared.sessions_open.fetch_add(1, Ordering::SeqCst);
        let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(serve(socket, id, config.clone(), shared.clone()));
    }
}

/// Decrements `sessions_open` and forgets the connection however `serve` ends.
struct OpenGuard {
    id: u64,
    shared: Arc<Shared>,
}

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.shared.lock().sessions.remove(&self.id);
        self.shared.sessions_open.fetch_sub(1, Ordering::SeqCst);
    }
}

async fn serve(
    socket: TcpStream,
    id: u64,
    config: Arc<russh::server::Config>,
    shared: Arc<Shared>,
) {
    let _guard = OpenGuard {
        id,
        shared: shared.clone(),
    };
    let Some(socket) = register(socket, id, &shared) else {
        return;
    };
    if let Some(d) = shared.config.slow_banner {
        tokio::time::sleep(d).await;
    }
    let handler = Client {
        shared: shared.clone(),
        failures: 0,
        kbd_step: 0,
    };
    let Ok(session) = russh::server::run_stream(config, socket, handler).await else {
        return;
    };
    if let Some(live) = shared.lock().sessions.get_mut(&id) {
        live.handle = Some(session.handle());
    }
    let _ = session.await;
}

/// Keeps a duplicate of the socket so `kill_sessions` can cut it.
fn register(socket: TcpStream, id: u64, shared: &Shared) -> Option<TcpStream> {
    let socket = socket.into_std().ok()?;
    let duplicate = socket.try_clone().ok()?;
    let socket = TcpStream::from_std(socket).ok()?;
    let _ = socket.set_nodelay(true);
    shared.lock().sessions.insert(
        id,
        Live {
            socket: duplicate,
            handle: None,
        },
    );
    Some(socket)
}

/// One connection's handler.
struct Client {
    shared: Arc<Shared>,
    /// Rejected authentication requests on this connection (OpenSSH's `MaxAuthTries`).
    failures: u32,
    /// The next keyboard-interactive round.
    kbd_step: usize,
}

impl Client {
    fn user(&self, name: &str) -> Option<&TestUser> {
        self.shared.config.users.iter().find(|u| match u {
            TestUser::Password { name: n, .. }
            | TestUser::Key { name: n, .. }
            | TestUser::Keyboard { name: n, .. } => n == name,
        })
    }

    fn attempt(&self) {
        self.shared.auth_attempts.fetch_add(1, Ordering::SeqCst);
    }

    /// A rejection; at `max_auth_tries` the connection is dropped (an error ends the session).
    /// Like OpenSSH, every method stays on offer after a failure (russh's default would drop a
    /// failed password method).
    fn reject(&mut self) -> Result<Auth, russh::Error> {
        self.failures += 1;
        match self.shared.config.max_auth_tries {
            Some(max) if self.failures >= max => Err(russh::Error::Disconnect),
            _ => Ok(Auth::Reject {
                proceed_with_methods: Some(MethodSet::from(OFFERED)),
                partial_success: false,
            }),
        }
    }

    fn key_matches(&self, user: &str, key: &PublicKey) -> bool {
        matches!(self.user(user), Some(TestUser::Key { key: k, .. }) if k.key_data() == key.key_data())
    }

    fn kbd_round(&self, steps: &[KbdStep]) -> Auth {
        let step = &steps[self.kbd_step];
        Auth::Partial {
            name: Cow::Borrowed(""),
            instructions: Cow::Borrowed(""),
            prompts: Cow::Owned(vec![(Cow::Owned(step.prompt.clone()), step.echo)]),
        }
    }
}

impl Handler for Client {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        self.attempt();
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.attempt();
        match self.user(user) {
            Some(TestUser::Password { password: pw, .. }) if pw == password => Ok(Auth::Accept),
            _ => self.reject(),
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        self.attempt();
        if self.key_matches(user, offer.public_key) {
            Ok(Auth::Accept)
        } else {
            self.reject()
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if self.key_matches(user, key.public_key) {
            Ok(Auth::Accept)
        } else {
            self.reject()
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        self.attempt();
        let Some(TestUser::Keyboard { steps, .. }) = self.user(user).cloned() else {
            return self.reject();
        };
        if steps.is_empty() {
            return Ok(Auth::Accept);
        }
        let Some(mut response) = response else {
            self.kbd_step = 0;
            return Ok(self.kbd_round(&steps));
        };
        let answer = response.next();
        let right = answer.as_deref() == Some(steps[self.kbd_step].answer.as_bytes())
            && response.next().is_none();
        if !right {
            self.kbd_step = 0;
            return self.reject();
        }
        self.kbd_step += 1;
        if self.kbd_step == steps.len() {
            self.kbd_step = 0;
            Ok(Auth::Accept)
        } else {
            Ok(self.kbd_round(&steps))
        }
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        originator_address: &str,
        originator_port: u32,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.shared.lock().last_originator =
            Some((originator_address.to_string(), originator_port));
        if self.shared.config.forward == ForwardPolicy::Deny {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let target = match self.shared.config.forward_to {
            ForwardTarget::Redirect(addr) => Target::Addr(addr),
            ForwardTarget::Any => match u16::try_from(port_to_connect) {
                Ok(port) => Target::Name(host_to_connect.to_string(), port),
                Err(_) => {
                    reply.reject(ChannelOpenFailure::ConnectFailed).await;
                    return Ok(());
                }
            },
        };
        let shared = self.shared.clone();
        // Connect off the session loop, then accept (or refuse) and relay.
        tokio::spawn(async move {
            let connected = match target {
                Target::Addr(addr) => TcpStream::connect(addr).await,
                Target::Name(host, port) => TcpStream::connect((host.as_str(), port)).await,
            };
            let Ok(mut tcp) = connected else {
                reply.reject(ChannelOpenFailure::ConnectFailed).await;
                return;
            };
            if let Some(d) = shared.config.forward_delay {
                tokio::time::sleep(d).await;
            }
            reply.accept().await;
            shared.channels_opened.fetch_add(1, Ordering::SeqCst);
            shared.channels_live.fetch_add(1, Ordering::SeqCst);
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
            shared.channels_live.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(())
    }
}

enum Target {
    Addr(SocketAddr),
    Name(String, u16),
}
