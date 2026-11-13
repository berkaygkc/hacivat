//! The tunnel manager (HV-SEC-01, B11, B20, Task 9): opens a chain of SSH hops (russh, ring
//! only), shares it between connections of the same chain, hands out `direct-tcpip` stream
//! factories, keeps it alive and releases it after its last lease.
//!
//! **Connecting.** Hop 0 is a TCP connect ([`resolve`] and [`direct`]); hop *i* > 0 runs SSH
//! over a `direct-tcpip` channel opened on hop *i − 1*. The database host is resolved by the
//! bastion (it goes into the last `direct-tcpip` request), never locally.
//!
//! **Host keys** are checked inside russh's `check_server_key`, during the key exchange, before
//! any authentication byte: `Known` passes; `Changed` and `Revoked` refuse in every policy;
//! `Unknown` asks (`Ask` with a prompter), trusts (`AcceptNew`) or refuses (`Strict`, `Ask`
//! headless). A host the stores list only with keys of *other* types is not "new": the
//! negotiation offers only the known types, and a key of another type is a change (T4 carry-in;
//! OpenSSH would otherwise call it unknown and `AcceptNew` would take it). Certificate host-key
//! algorithms are never offered, a presented certificate is refused, and SHA-1 `ssh-rsa`
//! signatures are not negotiated. The connect only proceeds when the handler recorded a passed
//! check.
//!
//! **Authentication order (B11)**, per hop: the agent, the key files (the hop's own, the
//! bastion's for a jump host, the imported `IdentityFile`s), a password, keyboard-interactive.
//! With `IdentitiesOnly` the key files come before the agent. A hop whose method is a password
//! or keyboard-interactive tries only those. Every secret has a budget of three attempts (the
//! stored one first, then the prompter), and a method the server stops offering is skipped.
//!
//! **Time.** `TunnelRequest.connect_timeout` bounds each hop's connect, handshake and
//! authentication; the time a prompt is open never counts (P23, Q44). Cancelling the token, or
//! dropping the `acquire` future, aborts every prompt of that acquire (prompts are
//! per-acquire: one that never answers holds no other profile's tunnel).
//!
//! **Sharing and release.** Leases of one [`ChainKey`] share a chain unless the request says
//! `share = false` (test connections). A shared chain closes `idle_grace` after its last lease,
//! an unshared one as soon as its lease goes; both close in reverse hop order, as does a chain
//! that failed part-way. Every hop's transport has a kill switch, tripped unless the hop
//! verified, authenticated and joined its chain, and again when it closes: a session stuck in a
//! key exchange never outlives its acquire or its chain. Keep-alives run per hop; when any hop's session ends on its own, the
//! chain's generation is retired, it is never handed out again, and [`TunnelEvent::Dead`] is
//! sent once.
//!
//! **Channel hygiene (Q46).** The client opens only `direct-tcpip` channels, with `127.0.0.1:0`
//! as the originator; it never requests a pty, a shell, exec, X11 or agent forwarding, and
//! refuses every channel the server opens.
//!
//! **Secrets back (Q17).** [`TunnelLease::take_secrets`] hands the caller the passwords and
//! passphrases that authenticated, once, for its session overlay; never keyboard answers.
//!
//! `hacivat-net` has no logger: the caller logs [`TunnelLease::trusted_keys`] (keys stored
//! without a question) and `KnownHosts::skipped_lines`.

// `SshError` is Task 1's interface: `HostKeyChanged` carries both keys by value, and one error
// travels per acquire, so its size is by design.
#![allow(
    clippy::result_large_err,
    reason = "SshError is a fixed interface; one per acquire"
)]

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::future::{BoxFuture, Either, select};
use futures::task::AtomicWaker;
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{
    Algorithm, EcdsaCurve, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey,
    PublicKeyOrCertificate,
};
use russh::{AlgorithmKind, Channel, ChannelOpenFailure, Disconnect, MethodKind, MethodSet};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Notify, broadcast, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::known_hosts::KnownHosts;
use super::{
    ChainKey, HopSecrets, HopSpec, HostKey, HostKeyCheck, HostKeyPolicy, KbdPrompt, KbdRequest,
    KnownSource, SshAuth, SshError, SshPrompter, SshSecretKind, SshSpec, TransportGeneration,
    TunnelEvent, TunnelRequest,
};
use crate::direct::{NetError, Target, direct, resolve};
use crate::stream::{BoxedStream, StreamFactory};

/// The most hops a chain has: `hacivat_secrets::MAX_SSH_HOP` (8) is the last hop index.
const MAX_HOPS: usize = 9;
/// Attempts per secret (a stored one counts) and keyboard-interactive logins per hop.
const ATTEMPTS: u8 = 3;
/// Keyboard-interactive rounds in one login before Hacivat gives up on a looping server.
const KBD_ROUNDS: usize = 8;
/// The largest key file read.
const MAX_KEY_FILE: u64 = 1 << 20;
/// How long a release waits for one hop's session to end before going on.
const RELEASE_WAIT: Duration = Duration::from_secs(2);
/// The `direct-tcpip` originator Hacivat sends (Q46): never a real local address.
const ORIGINATOR: (&str, u32) = ("127.0.0.1", 0);
/// Every key type a `known_hosts` line may name for a host, for the "other types" rule.
const KNOWN_KINDS: [&str; 8] = [
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "ssh-rsa",
    "ssh-dss",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Tunnel timings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TunnelConfig {
    /// How long a chain stays open after its last lease.
    pub idle_grace: Duration,
    pub keepalive: Duration,
    /// Unanswered keepalives before the chain counts as dead.
    pub keepalive_max: u32,
}

impl Default for TunnelConfig {
    /// 30 s grace, 30 s keepalive, 3 unanswered keepalives.
    fn default() -> Self {
        TunnelConfig {
            idle_grace: Duration::from_secs(30),
            keepalive: Duration::from_secs(30),
            keepalive_max: 3,
        }
    }
}

/// Where the SSH agent listens. Taken from the environment ([`AgentEndpoint::from_env`]) or
/// injected by tests; never from a profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentEndpoint {
    /// A Unix socket (`SSH_AUTH_SOCK`).
    Unix(PathBuf),
    /// A Windows named pipe (OpenSSH's `\\.\pipe\openssh-ssh-agent`).
    Pipe(PathBuf),
}

impl AgentEndpoint {
    /// The environment's agent: `SSH_AUTH_SOCK` on Unix (`None` when unset or empty), the
    /// OpenSSH agent pipe on Windows.
    pub fn from_env() -> Option<AgentEndpoint> {
        if cfg!(windows) {
            Some(AgentEndpoint::Pipe(PathBuf::from(
                r"\\.\pipe\openssh-ssh-agent",
            )))
        } else {
            std::env::var_os("SSH_AUTH_SOCK")
                .filter(|v| !v.is_empty())
                .map(|v| AgentEndpoint::Unix(PathBuf::from(v)))
        }
    }
}

type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

async fn connect_agent(endpoint: &AgentEndpoint) -> Option<Agent> {
    match endpoint {
        #[cfg(unix)]
        AgentEndpoint::Unix(path) => AgentClient::connect_uds(path)
            .await
            .ok()
            .map(AgentClient::dynamic),
        #[cfg(windows)]
        AgentEndpoint::Pipe(path) => AgentClient::connect_named_pipe(path)
            .await
            .ok()
            .map(AgentClient::dynamic),
        #[allow(unreachable_patterns, reason = "the other platform's endpoint")]
        _ => None,
    }
}

/// The app's SSH chains.
pub struct TunnelManager {
    known: Arc<KnownHosts>,
    config: TunnelConfig,
    agent: Option<AgentEndpoint>,
    registry: Arc<Registry>,
}

impl fmt::Debug for TunnelManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TunnelManager")
            .field("sessions_open", &self.sessions_open())
            .finish_non_exhaustive()
    }
}

/// The shared chains, their generations and the open-session count.
struct Registry {
    state: Mutex<State>,
    open: Arc<AtomicUsize>,
    events: broadcast::Sender<TunnelEvent>,
    idle_grace: Duration,
}

#[derive(Default)]
struct State {
    shared: HashMap<ChainKey, Slot>,
    generations: HashMap<ChainKey, u64>,
}

struct Slot {
    chain: Arc<Chain>,
    users: usize,
    /// Bumped at every lease and release: a grace timer only closes the epoch it started in.
    epoch: u64,
}

impl Registry {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl TunnelManager {
    /// A manager whose agent is the environment's ([`AgentEndpoint::from_env`]).
    pub fn new(known: Arc<KnownHosts>, config: TunnelConfig) -> Arc<TunnelManager> {
        Self::with_agent(known, config, AgentEndpoint::from_env())
    }

    /// A manager with an explicit agent (tests; `None`: no agent).
    pub fn with_agent(
        known: Arc<KnownHosts>,
        config: TunnelConfig,
        agent: Option<AgentEndpoint>,
    ) -> Arc<TunnelManager> {
        let registry = Arc::new(Registry {
            state: Mutex::new(State::default()),
            open: Arc::new(AtomicUsize::new(0)),
            events: broadcast::Sender::new(16),
            idle_grace: config.idle_grace,
        });
        Arc::new(TunnelManager {
            known,
            config,
            agent,
            registry,
        })
    }

    /// Opens (or shares) the chain of `req`, asking `prompter` for unknown keys and missing
    /// secrets. Cancelling `cancel` (or dropping the future) aborts the acquire and its prompts
    /// and releases every hop already up.
    pub fn acquire(
        self: &Arc<Self>,
        req: TunnelRequest,
        prompter: Option<Arc<dyn SshPrompter>>,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<TunnelLease, SshError>> {
        let this = self.clone();
        Box::pin(async move {
            // Ends this acquire's prompts when the acquire ends, however it ends.
            let scope = cancel.child_token();
            let _scope_guard = scope.clone().drop_guard();
            scope
                .run_until_cancelled(this.open(req, prompter, scope.clone()))
                .await
                .unwrap_or(Err(SshError::Cancelled))
        })
    }

    /// Open SSH sessions, all chains and hops counted (connecting ones included).
    pub fn sessions_open(&self) -> usize {
        self.registry.open.load(Ordering::SeqCst)
    }

    pub fn events(&self) -> broadcast::Receiver<TunnelEvent> {
        self.registry.events.subscribe()
    }

    async fn open(
        &self,
        req: TunnelRequest,
        prompter: Option<Arc<dyn SshPrompter>>,
        cancel: CancellationToken,
    ) -> Result<TunnelLease, SshError> {
        let hops = &req.spec.hops;
        if hops.is_empty() {
            return Err(SshError::Protocol { hop: 0 });
        }
        if hops.len() > MAX_HOPS {
            return Err(SshError::Protocol {
                hop: MAX_HOPS as u8,
            });
        }
        let key = chain_key(&req.spec);
        if req.share
            && let Ok(lease) = self.share(key, Vec::new(), Vec::new())
        {
            return Ok(lease);
        }

        let generation = {
            let mut state = self.registry.lock();
            let g = state.generations.entry(key).or_insert(0);
            *g += 1;
            *g
        };
        let last = hops.len() - 1;
        let chain = Arc::new(Chain {
            key,
            generation,
            shared: req.share,
            bastion: hops[last].host.clone(),
            hop_count: hops.len() as u8,
            channel_timeout: req.connect_timeout,
            hops: Mutex::new(Vec::new()),
            closing: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            established: AtomicBool::new(false),
            registry: Arc::downgrade(&self.registry),
        });
        let build = Build {
            manager: self,
            req: &req,
            prompter,
            cancel,
            chain: chain.clone(),
            trusted: Arc::new(Mutex::new(Vec::new())),
            used: Mutex::new(
                (0..hops.len())
                    .map(|_| HopSecrets {
                        password: None,
                        passphrase: None,
                    })
                    .collect(),
            ),
        };
        for i in 0..hops.len() {
            if let Err(e) = build.hop(i).await {
                // Partial failure: release the hops already up, last first, at once. The
                // secrets collected so far drop here (zeroized).
                chain.close().await;
                return Err(e);
            }
        }
        let trusted = std::mem::take(&mut *lock(&build.trusted));
        let secrets = std::mem::take(&mut *lock(&build.used));
        // `established` first, then `dead` (M2): a session that ends in between either sees
        // `established` (and sends `Dead`) or is seen here.
        chain.established.store(true, Ordering::SeqCst);
        if chain.dead.load(Ordering::SeqCst) {
            chain.close().await;
            return Err(SshError::Protocol { hop: last as u8 });
        }
        if !req.share {
            return Ok(TunnelLease::new(chain, trusted, secrets));
        }
        // Another acquire of the same chain may have finished meanwhile: keep one session.
        let (trusted, secrets) = match self.share(key, trusted, secrets) {
            Ok(lease) => {
                chain.close_detached();
                return Ok(lease);
            }
            Err(back) => back,
        };
        self.registry.lock().shared.insert(
            key,
            Slot {
                chain: chain.clone(),
                users: 1,
                epoch: 0,
            },
        );
        Ok(TunnelLease::new(chain, trusted, secrets))
    }

    /// A lease on the live shared chain of `key`, if there is one; else the notes back.
    #[allow(
        clippy::type_complexity,
        reason = "the two notes a lease carries, handed back"
    )]
    fn share(
        &self,
        key: ChainKey,
        trusted: Vec<(u8, HostKey)>,
        secrets: Vec<HopSecrets>,
    ) -> Result<TunnelLease, (Vec<(u8, HostKey)>, Vec<HopSecrets>)> {
        let mut state = self.registry.lock();
        let Some(slot) = state.shared.get_mut(&key).filter(|s| s.chain.alive()) else {
            return Err((trusted, secrets));
        };
        slot.users += 1;
        slot.epoch += 1;
        Ok(TunnelLease::new(slot.chain.clone(), trusted, secrets))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------
// Chains and leases

/// One incarnation of a chain of hops.
struct Chain {
    key: ChainKey,
    generation: u64,
    shared: bool,
    bastion: String,
    hop_count: u8,
    /// Bounds each `direct-tcpip` open.
    channel_timeout: Duration,
    /// The hops in order; emptied when the chain closes.
    hops: Mutex<Vec<HopConn>>,
    /// Set before a deliberate close: a session ending then is no death.
    closing: AtomicBool,
    dead: AtomicBool,
    /// Set once a lease was handed out: only then does a death send an event.
    established: AtomicBool,
    registry: Weak<Registry>,
}

/// An authenticated hop: its session handle, a receiver that resolves when the session's task
/// has ended (its handler, which holds the sender, is dropped then), and its transport's kill
/// switch.
struct HopConn {
    handle: Arc<Handle<HopHandler>>,
    ended: oneshot::Receiver<()>,
    kill: Arc<KillSwitch>,
}

/// Ends a hop's transport from outside the session (I1). russh does not notice a dropped
/// `Handle` while a key exchange is running (it stops reading its handle channel then), so a
/// hop that stalls mid-exchange, or a re-key that never finishes, would keep its task and socket
/// forever. Once tripped, the hop's stream reads end of file and refuses writes, so the session
/// task ends and drops the socket.
#[derive(Default)]
struct KillSwitch {
    tripped: AtomicBool,
    waker: AtomicWaker,
}

impl KillSwitch {
    fn trip(&self) {
        self.tripped.store(true, Ordering::SeqCst);
        self.waker.wake();
    }

    fn tripped(&self) -> bool {
        self.tripped.load(Ordering::SeqCst)
    }
}

/// Trips its switch when dropped, unless disarmed: a hop is kept only once its handshake
/// passed the host-key check, it authenticated and it joined its chain.
struct KillGuard(Option<Arc<KillSwitch>>);

impl KillGuard {
    fn disarm(mut self) -> Arc<KillSwitch> {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for KillGuard {
    fn drop(&mut self) {
        if let Some(kill) = self.0.take() {
            kill.trip();
        }
    }
}

/// A hop's transport behind its kill switch.
struct Killable {
    inner: BoxedStream,
    kill: Arc<KillSwitch>,
}

fn killed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "ssh hop closed")
}

impl AsyncRead for Killable {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.kill.tripped() {
            return Poll::Ready(Ok(()));
        }
        self.kill.waker.register(cx.waker());
        if self.kill.tripped() {
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Killable {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.kill.tripped() {
            return Poll::Ready(Err(killed()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.kill.tripped() {
            return Poll::Ready(Err(killed()));
        }
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.kill.tripped() {
            return Poll::Ready(Err(killed()));
        }
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Chain {
    fn alive(&self) -> bool {
        !self.dead.load(Ordering::SeqCst)
            && !self.closing.load(Ordering::SeqCst)
            && lock(&self.hops)
                .last()
                .is_some_and(|h| !h.handle.is_closed())
    }

    fn last_handle(&self) -> Option<Arc<Handle<HopHandler>>> {
        if self.closing.load(Ordering::SeqCst) {
            return None;
        }
        lock(&self.hops).last().map(|h| h.handle.clone())
    }

    /// Closes every hop, last first, waiting (bounded) for each session to end. Idempotent.
    async fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let hops = std::mem::take(&mut *lock(&self.hops));
        close_hops(hops).await;
    }

    /// [`Chain::close`] from synchronous code: on the runtime when there is one, else by
    /// dropping the handles (each session then ends on its own, inner hops first).
    fn close_detached(self: &Arc<Self>) {
        self.closing.store(true, Ordering::SeqCst);
        let hops = std::mem::take(&mut *lock(&self.hops));
        if hops.is_empty() {
            return;
        }
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn(close_hops(hops));
            }
            Err(_) => drop_hops(hops),
        }
    }

    /// A hop's session ended. Unless the chain is closing, this generation is dead: it leaves
    /// the registry, the rest of the chain closes and `Dead` is sent once.
    fn session_ended(self: &Arc<Self>) {
        if self.closing.load(Ordering::SeqCst) || self.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        if !self.established.load(Ordering::SeqCst) {
            // Still being built: `open` sees `dead` (or the next hop fails) and cleans up.
            return;
        }
        if let Some(registry) = self.registry.upgrade() {
            {
                let mut state = registry.lock();
                if state
                    .shared
                    .get(&self.key)
                    .is_some_and(|slot| Arc::ptr_eq(&slot.chain, self))
                {
                    state.shared.remove(&self.key);
                }
            }
            let _ = registry.events.send(TunnelEvent::Dead {
                key: self.key,
                generation: self.generation,
            });
        }
        self.close_detached();
    }

    /// One lease is gone.
    fn release(self: &Arc<Self>) {
        if !self.shared {
            self.close_detached();
            return;
        }
        let Some(registry) = self.registry.upgrade() else {
            self.close_detached();
            return;
        };
        let mut state = registry.lock();
        let Some(slot) = state
            .shared
            .get_mut(&self.key)
            .filter(|slot| Arc::ptr_eq(&slot.chain, self))
        else {
            // Dead or replaced: nobody else can lease it.
            drop(state);
            self.close_detached();
            return;
        };
        slot.users = slot.users.saturating_sub(1);
        slot.epoch += 1;
        if slot.users > 0 {
            return;
        }
        let epoch = slot.epoch;
        drop(state);
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            registry.lock().shared.remove(&self.key);
            self.close_detached();
            return;
        };
        let weak = Arc::downgrade(&registry);
        let chain = self.clone();
        let grace = registry.idle_grace;
        rt.spawn(async move {
            tokio::time::sleep(grace).await;
            if let Some(registry) = weak.upgrade() {
                let mut state = registry.lock();
                let idle = state.shared.get(&chain.key).is_some_and(|slot| {
                    Arc::ptr_eq(&slot.chain, &chain) && slot.users == 0 && slot.epoch == epoch
                });
                if !idle {
                    return;
                }
                state.shared.remove(&chain.key);
            }
            chain.close().await;
        });
    }
}

impl Drop for Chain {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        drop_hops(std::mem::take(&mut *lock(&self.hops)));
    }
}

/// Disconnects hops last first, each one's session ending (or `RELEASE_WAIT`) before the next.
async fn close_hops(mut hops: Vec<HopConn>) {
    while let Some(hop) = hops.pop() {
        close_hop(hop).await;
    }
}

/// Sends a disconnect, waits (bounded) for the session to end, then trips the hop's transport
/// so a session stuck in a key exchange ends too.
async fn close_hop(hop: HopConn) {
    let HopConn {
        handle,
        ended,
        kill,
    } = hop;
    let _ = tokio::time::timeout(
        RELEASE_WAIT,
        handle.disconnect(Disconnect::ByApplication, "", "en"),
    )
    .await;
    drop(handle);
    let _ = tokio::time::timeout(RELEASE_WAIT, ended).await;
    kill.trip();
}

/// Ends hops from synchronous code, last first: each transport is tripped, so every session
/// ends whatever it is doing.
fn drop_hops(mut hops: Vec<HopConn>) {
    while let Some(hop) = hops.pop() {
        hop.kill.trip();
        drop(hop);
    }
}

/// A use of an open chain. Dropping it releases the chain (in reverse hop order, after the
/// grace period at zero users; at once for an unshared chain).
pub struct TunnelLease {
    generation: TransportGeneration,
    bastion: String,
    hops: u8,
    chain: Arc<Chain>,
    trusted: Vec<(u8, HostKey)>,
    /// Never in `Debug`.
    secrets: Vec<HopSecrets>,
}

impl fmt::Debug for TunnelLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TunnelLease")
            .field("generation", &self.generation)
            .field("bastion", &self.bastion)
            .field("hops", &self.hops)
            .finish_non_exhaustive()
    }
}

impl TunnelLease {
    fn new(
        chain: Arc<Chain>,
        trusted: Vec<(u8, HostKey)>,
        secrets: Vec<HopSecrets>,
    ) -> TunnelLease {
        TunnelLease {
            generation: TransportGeneration {
                key: chain.key,
                generation: chain.generation,
            },
            bastion: chain.bastion.clone(),
            hops: chain.hop_count,
            chain,
            trusted,
            secrets,
        }
    }

    /// The secrets that authenticated this acquire's hops, indexed by hop and shaped like
    /// `TunnelRequest.secrets` (a reconnect passes them straight back): the password that was
    /// accepted and the passphrase that decrypted the accepted key, stored or typed (Q17: the
    /// session overlay keeps them in memory). Keyboard-interactive answers are never included:
    /// they are often one-time codes and must not be replayed. Empty for a lease that joined an
    /// open chain, and on every call after the first.
    pub fn take_secrets(&mut self) -> Vec<HopSecrets> {
        std::mem::take(&mut self.secrets)
    }

    /// A factory of `direct-tcpip` streams to `host:port` as the bastion sees it: each `open`
    /// opens one channel on the last hop (the bastion resolves `host`). A refused channel fails
    /// with an `io::Error` that [`classify_channel_err`] turns back into the [`SshError`]; a
    /// factory that outlives its chain fails with `NotConnected`. An `open` that times out or
    /// is dropped never leaks the channel: one confirmed after the caller left is closed at once.
    pub fn factory(&self, host: &str, port: u16) -> StreamFactory {
        let chain = Arc::downgrade(&self.chain);
        let host: Arc<str> = Arc::from(host);
        let hop = self.hops.saturating_sub(1);
        StreamFactory::new(move || {
            let chain = chain.clone();
            let host = host.clone();
            Box::pin(async move {
                let gone = || channel_io_error(SshError::Protocol { hop });
                let chain = chain.upgrade().ok_or_else(gone)?;
                let handle = chain.last_handle().ok_or_else(gone)?;
                let limit = chain.channel_timeout;
                drop(chain);
                let channel = open_channel(handle, host.to_string(), port, hop, Some(limit))
                    .await
                    .map_err(channel_io_error)?;
                Ok(Box::new(channel.into_stream()) as BoxedStream)
            })
        })
    }

    pub fn generation(&self) -> TransportGeneration {
        self.generation
    }

    /// The last hop's host, as the capsule names the path.
    pub fn bastion(&self) -> &str {
        &self.bastion
    }

    pub fn hops(&self) -> u8 {
        self.hops
    }

    /// The host keys this acquire stored without a question (`AcceptNew`) or after the user's
    /// yes, by hop. The caller logs them (`hacivat-net` has no logger). Empty for a lease that
    /// joined an open chain.
    pub fn trusted_keys(&self) -> &[(u8, HostKey)] {
        &self.trusted
    }
}

impl Drop for TunnelLease {
    fn drop(&mut self) {
        self.chain.release();
    }
}

/// The [`SshError`] inside an `io::Error` from a [`TunnelLease::factory`] stream open.
pub fn classify_channel_err(e: &io::Error) -> Option<SshError> {
    e.get_ref()?.downcast_ref::<SshError>().cloned()
}

fn channel_io_error(e: SshError) -> io::Error {
    let kind = match e {
        SshError::ChannelOpen { .. } => io::ErrorKind::ConnectionRefused,
        SshError::Timeout { .. } => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::NotConnected,
    };
    io::Error::new(kind, e)
}

/// Opens a `direct-tcpip` channel on `handle` (originator `127.0.0.1:0`). The open runs to its
/// end on its own task (russh fails a pending open when the session ends, so the task is
/// bounded by the session); `limit` bounds only the caller's wait. When the caller gives up
/// first (a timeout, a cancel, a dropped future), a channel confirmed later is closed at once,
/// never left open on the server (I2).
async fn open_channel(
    handle: Arc<Handle<HopHandler>>,
    host: String,
    port: u16,
    hop: u8,
    limit: Option<Duration>,
) -> Result<Channel<Msg>, SshError> {
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let opened = handle
            .channel_open_direct_tcpip(host, u32::from(port), ORIGINATOR.0, ORIGINATOR.1)
            .await;
        drop(handle);
        if let Err(Ok(channel)) = tx.send(opened) {
            let _ = channel.close().await;
        }
    });
    let answer = match limit {
        Some(limit) => tokio::time::timeout(limit, rx)
            .await
            .map_err(|_elapsed| SshError::Timeout { hop })?,
        None => rx.await,
    };
    match answer {
        Ok(Ok(channel)) => Ok(channel),
        Ok(Err(russh::Error::ChannelOpenFailure(reason))) => Err(SshError::ChannelOpen {
            hop,
            reason: channel_failure_id(&reason).to_string(),
        }),
        Ok(Err(_)) | Err(_) => Err(SshError::Protocol { hop }),
    }
}

/// A stable id for a refusal (the server's free text is never echoed).
fn channel_failure_id(reason: &ChannelOpenFailure) -> &'static str {
    match reason {
        ChannelOpenFailure::AdministrativelyProhibited => "administratively_prohibited",
        ChannelOpenFailure::ConnectFailed => "connect_failed",
        ChannelOpenFailure::UnknownChannelType => "unknown_channel_type",
        ChannelOpenFailure::ResourceShortage => "resource_shortage",
        ChannelOpenFailure::Other { .. } => "other",
    }
}

// ---------------------------------------------------------------------------------------------
// Building a chain

/// One acquire's context.
struct Build<'a> {
    manager: &'a TunnelManager,
    req: &'a TunnelRequest,
    prompter: Option<Arc<dyn SshPrompter>>,
    cancel: CancellationToken,
    chain: Arc<Chain>,
    trusted: Arc<Mutex<Vec<(u8, HostKey)>>>,
    /// Per hop, the password and passphrase that authenticated (Q17; handed over on success).
    used: Mutex<Vec<HopSecrets>>,
}

impl Build<'_> {
    /// Connects, verifies and authenticates hop `i`, then adds it to the chain.
    async fn hop(&self, i: usize) -> Result<(), SshError> {
        let spec = &self.req.spec.hops[i];
        let n = i as u8;
        let clock = Clock::new(self.req.connect_timeout);
        let listed = self.listed_keys(n, spec).await?;
        let restricted = !listed.kinds.is_empty();
        let Connected {
            mut handle,
            ended,
            armed,
            guard,
        } = match self.connect(i, &clock, &listed, restricted).await {
            Err(Retry) => self.connect(i, &clock, &listed, false).await,
            done => done,
        }
        .map_err(|Retry| SshError::Protocol { hop: n })??;

        let auth = Auth {
            hop: n,
            spec,
            secrets: self.req.secrets.get(i),
            prompter: self.prompter.as_ref(),
            agent: self.manager.agent.as_ref(),
            clock: &clock,
            cancel: &self.cancel,
            bastion_key: match &self.req.spec.hops.last().map(|h| &h.auth) {
                Some(SshAuth::KeyFile { path }) if i + 1 < self.req.spec.hops.len() => {
                    Some(path.as_path())
                }
                _ => None,
            },
            used: &self.used,
        };
        // Until the hop joins its chain, `guard` trips its transport if this future is dropped.
        if let Err(e) = auth.run(&mut handle).await {
            close_hop(HopConn {
                handle: Arc::new(handle),
                ended,
                kill: guard.disarm(),
            })
            .await;
            return Err(e);
        }
        armed.store(true, Ordering::SeqCst);
        lock(&self.chain.hops).push(HopConn {
            handle: Arc::new(handle),
            ended,
            kill: guard.disarm(),
        });
        Ok(())
    }

    /// What the stores list for this hop (blocking reads, on the blocking pool): the key types
    /// among [`KNOWN_KINDS`] (to restrict the negotiation), and any listed key of any type (an
    /// undecodable probe makes `check` count every listed key as a change, so a type outside
    /// the list is seen too, M1).
    async fn listed_keys(&self, n: u8, spec: &HopSpec) -> Result<Listed, SshError> {
        let known = self.manager.known.clone();
        let (host, port) = (spec.host.clone(), spec.port);
        blocking(move || {
            let kinds = KNOWN_KINDS
                .iter()
                .filter(|kind| {
                    matches!(
                        known.check(&host, port, &kind_probe(kind)),
                        HostKeyCheck::Changed { .. }
                    )
                })
                .map(|kind| (*kind).to_string())
                .collect();
            let any = match known.check(&host, port, &any_kind_probe()) {
                HostKeyCheck::Changed { old, source } => Some((old, source)),
                _ => None,
            };
            Listed { kinds, any }
        })
        .await
        .ok_or(SshError::Protocol { hop: n })
    }

    /// The transport and the SSH handshake (host key included) of hop `i`. `Err(Retry)` when
    /// the server offers none of the known key types: the caller retries offering every type,
    /// so the check sees the presented key and reports the change. The returned guard still
    /// holds the transport's kill switch: every other outcome (an error, a timeout, a cancel,
    /// this future dropped) trips it, so no session task outlives a failed handshake (I1).
    async fn connect(
        &self,
        i: usize,
        clock: &Arc<Clock>,
        listed: &Listed,
        restricted: bool,
    ) -> Result<Result<Connected, SshError>, Retry> {
        let spec = &self.req.spec.hops[i];
        let n = i as u8;
        let stream = match self.transport(i, clock).await {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        let kill = Arc::new(KillSwitch::default());
        let guard = KillGuard(Some(kill.clone()));
        let stream = Killable {
            inner: stream,
            kill,
        };
        let keepalive = self
            .req
            .spec
            .keepalive_s
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(u64::from(s)))
            .unwrap_or(self.manager.config.keepalive);
        let kinds: Vec<&str> = listed.kinds.iter().map(String::as_str).collect();
        let config = Arc::new(client::Config {
            preferred: preferred(if restricted { &kinds } else { &[] }),
            keepalive_interval: Some(keepalive),
            keepalive_max: self.manager.config.keepalive_max as usize,
            inactivity_timeout: None,
            nodelay: true,
            ..Default::default()
        });
        let verified = Arc::new(AtomicBool::new(false));
        let armed = Arc::new(AtomicBool::new(false));
        let (ended_tx, ended) = oneshot::channel();
        let handler = HopHandler {
            hop: n,
            host: spec.host.clone(),
            port: spec.port,
            policy: self.req.spec.host_keys,
            known: self.manager.known.clone(),
            other: listed.any.clone(),
            prompter: self.prompter.clone(),
            cancel: self.cancel.clone(),
            clock: clock.clone(),
            trusted: self.trusted.clone(),
            verified: verified.clone(),
            armed: armed.clone(),
            chain: Arc::downgrade(&self.chain),
            _open: OpenCount::new(&self.manager.registry.open),
            _ended: ended_tx,
        };
        let handle = match clock
            .run(client::connect_stream(config, stream, handler))
            .await
        {
            None => return Ok(Err(SshError::Timeout { hop: n })),
            Some(Ok(handle)) => handle,
            Some(Err(HopError::Refused(e))) => return Ok(Err(e)),
            Some(Err(HopError::Ssh(russh::Error::NoCommonAlgo {
                kind: AlgorithmKind::Key,
                ..
            }))) if restricted => return Err(Retry),
            Some(Err(HopError::Ssh(e))) => return Ok(Err(ssh_failure(n, e))),
        };
        // No code path proceeds without a passed host-key check.
        if !verified.load(Ordering::SeqCst) {
            return Ok(Err(SshError::Protocol { hop: n }));
        }
        Ok(Ok(Connected {
            handle,
            ended,
            armed,
            guard,
        }))
    }

    /// Hop 0: a TCP connect. Hop i > 0: a `direct-tcpip` channel through hop i − 1.
    async fn transport(&self, i: usize, clock: &Clock) -> Result<BoxedStream, SshError> {
        let spec = &self.req.spec.hops[i];
        let n = i as u8;
        if i == 0 {
            let addrs = clock
                .run(resolve(&spec.host, spec.port))
                .await
                .ok_or(SshError::Timeout { hop: n })?
                .map_err(|net| SshError::Unreachable { hop: n, net })?;
            let factory = direct(
                Target::Tcp {
                    host: spec.host.clone(),
                    port: spec.port,
                },
                addrs,
                self.req.connect_timeout,
            );
            return clock
                .run(factory.open())
                .await
                .ok_or(SshError::Timeout { hop: n })?
                .map_err(|e| io_failure(n, &e));
        }
        let previous = lock(&self.chain.hops)
            .get(i - 1)
            .map(|h| h.handle.clone())
            .ok_or(SshError::Protocol { hop: n - 1 })?;
        // `clock.run` is the (pausable) bound; dropping the open closes a late channel.
        let channel = clock
            .run(open_channel(
                previous,
                spec.host.clone(),
                spec.port,
                n - 1,
                None,
            ))
            .await
            .ok_or(SshError::Timeout { hop: n })??;
        Ok(Box::new(channel.into_stream()))
    }
}

/// The handshake must be retried offering every host-key type.
struct Retry;

/// A hop after its handshake: the session, its end signal, the flag that hands its end to the
/// chain, and the still-armed kill switch of its transport.
struct Connected {
    handle: Handle<HopHandler>,
    ended: oneshot::Receiver<()>,
    armed: Arc<AtomicBool>,
    guard: KillGuard,
}

/// What the stores list for a hop.
struct Listed {
    /// Types among [`KNOWN_KINDS`] with a listed key.
    kinds: Vec<String>,
    /// A listed key of any type, with its source.
    any: Option<(HostKey, KnownSource)>,
}

/// A probe whose type does not decode: `KnownHosts::check` then counts every listed plain key
/// for the host as a change (it fails closed), so `Changed` answers "is any key listed?".
fn any_kind_probe() -> HostKey {
    HostKey {
        algorithm: String::new(),
        sha256: [0; 32],
        openssh: "x !".to_string(),
    }
}

/// A probe key of `kind` for [`KnownHosts::check`]: the blob names the type and nothing else,
/// and its hash matches no real key, so the answer is `Changed` exactly when a key of that type
/// is listed for the host.
fn kind_probe(kind: &str) -> HostKey {
    let mut blob = (kind.len() as u32).to_be_bytes().to_vec();
    blob.extend_from_slice(kind.as_bytes());
    HostKey {
        algorithm: kind.to_string(),
        sha256: [0; 32],
        openssh: format!("{kind} {}", STANDARD.encode(blob)),
    }
}

/// The algorithms to negotiate: russh's defaults without SHA-1 `ssh-rsa` and without host
/// certificates; with `kinds`, only the host-key algorithms of those key types (an empty match
/// keeps them all, and the check then refuses whatever comes as a change).
fn preferred(kinds: &[&str]) -> russh::Preferred {
    let all = [
        ("ssh-ed25519", Algorithm::Ed25519),
        (
            "ecdsa-sha2-nistp256",
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        ),
        (
            "ecdsa-sha2-nistp384",
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
        ),
        (
            "ecdsa-sha2-nistp521",
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP521,
            },
        ),
        (
            "ssh-rsa",
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
        ),
        (
            "ssh-rsa",
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
        ),
    ];
    let mut key: Vec<Algorithm> = all
        .iter()
        .filter(|(kind, _)| kinds.contains(kind))
        .map(|(_, alg)| alg.clone())
        .collect();
    if key.is_empty() {
        key = all.into_iter().map(|(_, alg)| alg).collect();
    }
    russh::Preferred {
        key: Cow::Owned(key),
        host_key_certificates: Cow::Borrowed(&[]),
        ..russh::Preferred::DEFAULT
    }
}

/// The canonical [`HostKey`] of a server key: built from the key's own type (`ssh-rsa`, never
/// the signature algorithm `rsa-sha2-512`), so `KnownHosts::trust` accepts it.
fn host_key_of(key: &PublicKey) -> Option<HostKey> {
    HostKey::from_openssh(&key.to_openssh().ok()?)
}

fn ssh_failure(hop: u8, e: russh::Error) -> SshError {
    match e {
        russh::Error::IO(e) => io_failure(hop, &e),
        _ => SshError::Protocol { hop },
    }
}

fn io_failure(hop: u8, e: &io::Error) -> SshError {
    use io::ErrorKind as K;
    let net = match e.kind() {
        K::TimedOut => return SshError::Timeout { hop },
        K::ConnectionRefused => NetError::Refused,
        K::HostUnreachable | K::NetworkUnreachable => NetError::Unreachable,
        K::ConnectionReset | K::ConnectionAborted => NetError::Reset,
        kind => NetError::Io {
            kind,
            message: e.to_string(),
        },
    };
    SshError::Unreachable { hop, net }
}

/// Runs `f` on the blocking pool (`known_hosts` reads and writes, key decryption).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

/// A SHA-256 over every hop's host, port, user and authentication identity, and the policy.
fn chain_key(spec: &SshSpec) -> ChainKey {
    fn field(h: &mut Sha256, bytes: &[u8]) {
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }
    let mut h = Sha256::new();
    field(&mut h, b"hacivat-ssh-chain-v1");
    field(&mut h, spec.host_keys.id().as_bytes());
    // Profiles that differ only in keep-alive keep their own interval (M4).
    field(&mut h, &spec.keepalive_s.unwrap_or(0).to_be_bytes());
    for hop in &spec.hops {
        field(&mut h, hop.host.to_ascii_lowercase().as_bytes());
        field(&mut h, &hop.port.to_be_bytes());
        field(&mut h, hop.user.as_bytes());
        match &hop.auth {
            SshAuth::Agent => field(&mut h, b"agent"),
            SshAuth::KeyFile { path } => {
                field(&mut h, b"key");
                field(&mut h, path.as_os_str().as_encoded_bytes());
            }
            SshAuth::Password => field(&mut h, b"password"),
            SshAuth::KeyboardInteractive => field(&mut h, b"keyboard-interactive"),
        }
        field(&mut h, &[u8::from(hop.identities_only)]);
        field(&mut h, &(hop.identity_files.len() as u64).to_be_bytes());
        for file in &hop.identity_files {
            field(&mut h, file.as_os_str().as_encoded_bytes());
        }
    }
    ChainKey(h.finalize().into())
}

// ---------------------------------------------------------------------------------------------
// The per-hop russh handler

/// Counts an open session in the manager from creation to drop.
struct OpenCount(Arc<AtomicUsize>);

impl OpenCount {
    fn new(open: &Arc<AtomicUsize>) -> OpenCount {
        open.fetch_add(1, Ordering::SeqCst);
        OpenCount(open.clone())
    }
}

impl Drop for OpenCount {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
enum HopError {
    Ssh(russh::Error),
    /// The host-key check refused (or was cancelled): this error, not russh's.
    Refused(SshError),
}

impl From<russh::Error> for HopError {
    fn from(e: russh::Error) -> Self {
        HopError::Ssh(e)
    }
}

/// One hop's client handler. Lives on russh's session task; dropped when the session ends.
struct HopHandler {
    hop: u8,
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    known: Arc<KnownHosts>,
    /// A key the stores list for this host under another type than any presented so far.
    other: Option<(HostKey, KnownSource)>,
    prompter: Option<Arc<dyn SshPrompter>>,
    cancel: CancellationToken,
    clock: Arc<Clock>,
    trusted: Arc<Mutex<Vec<(u8, HostKey)>>>,
    /// Set only by a passed host-key check.
    verified: Arc<AtomicBool>,
    /// Set once the hop is part of its chain: from then on its end is the chain's concern.
    armed: Arc<AtomicBool>,
    chain: Weak<Chain>,
    _open: OpenCount,
    _ended: oneshot::Sender<()>,
}

impl Drop for HopHandler {
    fn drop(&mut self) {
        if self.armed.load(Ordering::SeqCst)
            && let Some(chain) = self.chain.upgrade()
        {
            chain.session_ended();
        }
    }
}

impl HopHandler {
    fn unknown(&self, key: HostKey) -> SshError {
        SshError::HostKeyUnknown {
            hop: self.hop,
            host: self.host.clone(),
            port: self.port,
            key,
        }
    }

    fn revoked(&self) -> SshError {
        SshError::HostKeyRevoked {
            hop: self.hop,
            host: self.host.clone(),
            port: self.port,
        }
    }

    fn changed(&self, old: HostKey, new: HostKey, source: KnownSource) -> SshError {
        SshError::HostKeyChanged {
            hop: self.hop,
            host: self.host.clone(),
            port: self.port,
            old,
            new,
            source,
        }
    }

    async fn check(&self, key: HostKey) -> HostKeyCheck {
        let (known, host, port) = (self.known.clone(), self.host.clone(), self.port);
        // A failed blocking task fails closed: as revoked, the hardest stop.
        blocking(move || known.check(&host, port, &key))
            .await
            .unwrap_or(HostKeyCheck::Revoked {
                source: KnownSource::Hacivat,
            })
    }

    /// The policy (HV-SEC-01): `Ok` only for a known key or one trusted under the policy.
    async fn verify(&self, presented: &PublicKeyOrCertificate) -> Result<(), SshError> {
        // Certificates are never negotiated; one presented anyway is refused.
        let PublicKeyOrCertificate::PublicKey { key, .. } = presented else {
            return Err(SshError::Protocol { hop: self.hop });
        };
        let key = host_key_of(key).ok_or(SshError::Protocol { hop: self.hop })?;
        match self.check(key.clone()).await {
            HostKeyCheck::Known(_) => Ok(()),
            HostKeyCheck::Changed { old, source } => Err(self.changed(old, key, source)),
            HostKeyCheck::Revoked { .. } => Err(self.revoked()),
            HostKeyCheck::Unknown => {
                if let Some((old, source)) = self.other.clone() {
                    // The host is known by a key of another type: a change, never "new".
                    return Err(self.changed(old, key, source));
                }
                match (self.policy, &self.prompter) {
                    (HostKeyPolicy::Strict, _) | (HostKeyPolicy::Ask, None) => {
                        Err(self.unknown(key))
                    }
                    (HostKeyPolicy::AcceptNew, _) => self.trust(key, true).await,
                    (HostKeyPolicy::Ask, Some(prompter)) => {
                        let answer = {
                            let _paused = self.clock.pause();
                            self.cancel
                                .run_until_cancelled(
                                    prompter.host_key(self.hop, &self.host, self.port, &key),
                                )
                                .await
                        };
                        match answer {
                            None => Err(SshError::Cancelled),
                            Some(false) => Err(self.unknown(key)),
                            Some(true) => self.trust(key, false).await,
                        }
                    }
                }
            }
        }
    }

    /// Stores `key` for the host. A revoked or changed key is refused here too (the stores may
    /// have changed since the check). When the write fails otherwise, `AcceptNew` refuses (it is
    /// only safe while the stored key guards the next connect); a key the user accepted is used
    /// for this connect, and the next one asks again.
    async fn trust(&self, key: HostKey, accept_new: bool) -> Result<(), SshError> {
        let (known, host, port, k) = (
            self.known.clone(),
            self.host.clone(),
            self.port,
            key.clone(),
        );
        let stored = blocking(move || known.trust(&host, port, &k)).await;
        match stored {
            Some(Ok(())) => {
                lock(&self.trusted).push((self.hop, key));
                Ok(())
            }
            Some(Err(e)) if e.kind() == io::ErrorKind::PermissionDenied => Err(self.revoked()),
            Some(Err(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
                match self.check(key.clone()).await {
                    HostKeyCheck::Changed { old, source } => Err(self.changed(old, key, source)),
                    HostKeyCheck::Known(_) => Ok(()),
                    HostKeyCheck::Revoked { .. } => Err(self.revoked()),
                    HostKeyCheck::Unknown => Err(self.unknown(key)),
                }
            }
            _ if accept_new => Err(self.unknown(key)),
            _ => Ok(()),
        }
    }
}

impl client::Handler for HopHandler {
    type Error = HopError;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match self.verify(server_public_key).await {
            Ok(()) => {
                self.verified.store(true, Ordering::SeqCst);
                Ok(true)
            }
            Err(e) => Err(HopError::Refused(e)),
        }
    }

    // Q46: every server-initiated channel is refused (dropping the reply rejects it).

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _connected_address: &str,
        _connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        _channel: Channel<Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_direct_tcpip(
        &mut self,
        _channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<Msg>,
        _socket_path: &str,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn server_channel_open_x11(
        &mut self,
        _channel: Channel<Msg>,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        reply
            .reject(ChannelOpenFailure::AdministrativelyProhibited)
            .await;
        Ok(())
    }

    async fn should_accept_unknown_server_channel(
        &mut self,
        _id: russh::ChannelId,
        _channel_type: &str,
    ) -> bool {
        false
    }

    async fn disconnected(
        &mut self,
        _reason: client::DisconnectReason<Self::Error>,
    ) -> Result<(), Self::Error> {
        // The chain learns of the end when this handler drops (it always does).
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Authentication

enum Step<'p> {
    Agent,
    Key(&'p Path),
    Password,
    Keyboard,
}

enum Outcome {
    Success,
    /// Tried, refused (or not usable); the next step may still succeed.
    Failed,
}

struct Auth<'a> {
    hop: u8,
    spec: &'a HopSpec,
    secrets: Option<&'a HopSecrets>,
    prompter: Option<&'a Arc<dyn SshPrompter>>,
    agent: Option<&'a AgentEndpoint>,
    clock: &'a Arc<Clock>,
    cancel: &'a CancellationToken,
    /// For a jump host: the bastion's key file (B11).
    bastion_key: Option<&'a Path>,
    /// Where the secrets that authenticated are recorded (indexed by hop).
    used: &'a Mutex<Vec<HopSecrets>>,
}

/// What the server still accepts (`None` until it said).
struct Offered(Option<MethodSet>);

impl Offered {
    fn allows(&self, kind: MethodKind) -> bool {
        self.0.as_ref().is_none_or(|m| m.contains(&kind))
    }
}

impl Auth<'_> {
    /// The hop's key files, deduplicated: its own, the bastion's (jump hosts), the imported.
    fn key_files(&self) -> Vec<&Path> {
        let mut keys: Vec<&Path> = Vec::new();
        let own = match &self.spec.auth {
            SshAuth::KeyFile { path } => Some(path.as_path()),
            _ => None,
        };
        for path in own
            .into_iter()
            .chain(self.bastion_key)
            .chain(self.spec.identity_files.iter().map(PathBuf::as_path))
        {
            if !keys.contains(&path) {
                keys.push(path);
            }
        }
        keys
    }

    fn steps(&self) -> Vec<Step<'_>> {
        let keys = self.key_files();
        match self.spec.auth {
            SshAuth::Password => vec![Step::Password, Step::Keyboard],
            SshAuth::KeyboardInteractive => vec![Step::Keyboard],
            SshAuth::Agent | SshAuth::KeyFile { .. } => {
                let keys = keys.into_iter().map(Step::Key);
                let mut steps: Vec<Step<'_>> = if self.spec.identities_only {
                    keys.chain([Step::Agent]).collect()
                } else {
                    [Step::Agent].into_iter().chain(keys).collect()
                };
                steps.extend([Step::Password, Step::Keyboard]);
                steps
            }
        }
    }

    /// Records a secret that authenticated this hop.
    fn record(&self, password: Option<SecretString>, passphrase: Option<SecretString>) {
        if let Some(slot) = lock(self.used).get_mut(usize::from(self.hop)) {
            if password.is_some() {
                slot.password = password;
            }
            if passphrase.is_some() {
                slot.passphrase = passphrase;
            }
        }
    }

    fn failed(&self) -> SshError {
        SshError::Auth {
            hop: self.hop,
            user: self.spec.user.clone(),
            host: self.spec.host.clone(),
        }
    }

    async fn run(&self, h: &mut Handle<HopHandler>) -> Result<(), SshError> {
        let mut offered = Offered(None);
        let mut agent_missing = false;
        for step in self.steps() {
            if h.is_closed() {
                break;
            }
            let outcome = match step {
                Step::Agent if offered.allows(MethodKind::PublicKey) => {
                    self.agent(h, &mut offered, &mut agent_missing).await?
                }
                Step::Key(path) if offered.allows(MethodKind::PublicKey) => {
                    self.key(h, path, &mut offered).await?
                }
                Step::Password if offered.allows(MethodKind::Password) => {
                    self.password(h, &mut offered).await?
                }
                Step::Keyboard if offered.allows(MethodKind::KeyboardInteractive) => {
                    self.keyboard(h, &mut offered).await?
                }
                _ => Outcome::Failed,
            };
            if let Outcome::Success = outcome {
                return Ok(());
            }
        }
        if agent_missing && self.spec.auth == SshAuth::Agent {
            return Err(SshError::AgentUnavailable { hop: self.hop });
        }
        Err(self.failed())
    }

    /// Runs one server round trip under the hop's clock.
    async fn timed<T>(&self, fut: impl Future<Output = T>) -> Result<T, SshError> {
        self.clock
            .run(fut)
            .await
            .ok_or(SshError::Timeout { hop: self.hop })
    }

    /// Records a refusal; whether the session is still worth another try.
    fn refused(&self, h: &Handle<HopHandler>, offered: &mut Offered, remaining: MethodSet) {
        offered.0 = Some(if h.is_closed() {
            MethodSet::empty()
        } else {
            remaining
        });
    }

    /// Asks the prompter for a secret (paused clock, cancellable). `Ok(None)`: no prompter.
    async fn ask(
        &self,
        kind: SshSecretKind,
        attempt: u8,
    ) -> Result<Option<SecretString>, SshError> {
        let Some(prompter) = self.prompter else {
            return Ok(None);
        };
        let _paused = self.clock.pause();
        match self
            .cancel
            .run_until_cancelled(prompter.secret(self.hop, kind, attempt))
            .await
        {
            Some(Some(secret)) => Ok(Some(secret)),
            None | Some(None) => Err(SshError::Cancelled),
        }
    }

    async fn rsa_hash(
        &self,
        h: &Handle<HopHandler>,
        key: &PublicKey,
    ) -> Result<Option<HashAlg>, SshError> {
        if !key.algorithm().is_rsa() {
            return Ok(None);
        }
        let best = self.timed(h.best_supported_rsa_hash()).await?;
        Ok(best.ok().flatten().unwrap_or(Some(HashAlg::Sha512)))
    }

    async fn agent(
        &self,
        h: &mut Handle<HopHandler>,
        offered: &mut Offered,
        missing: &mut bool,
    ) -> Result<Outcome, SshError> {
        let Some(endpoint) = self.agent else {
            *missing = true;
            return Ok(Outcome::Failed);
        };
        let Some(mut agent) = self.timed(connect_agent(endpoint)).await? else {
            *missing = true;
            return Ok(Outcome::Failed);
        };
        let Ok(identities) = self.timed(agent.request_identities()).await? else {
            *missing = true;
            return Ok(Outcome::Failed);
        };
        // With IdentitiesOnly, only the agent keys of the configured identities (OpenSSH, M6).
        let allowed = if self.spec.identities_only {
            let files: Vec<PathBuf> = self.key_files().iter().map(|p| p.to_path_buf()).collect();
            Some(
                blocking(move || files.iter().filter_map(|f| identity_public(f)).collect())
                    .await
                    .unwrap_or_else(Vec::new),
            )
        } else {
            None
        };
        for identity in identities {
            // Certificates are not offered (v1).
            let AgentIdentity::PublicKey { key, .. } = identity else {
                continue;
            };
            if let Some(allowed) = &allowed
                && !allowed
                    .iter()
                    .any(|k: &PublicKey| k.key_data() == key.key_data())
            {
                continue;
            }
            let hash = self.rsa_hash(h, &key).await?;
            let user = self.spec.user.clone();
            match self
                .timed(h.authenticate_publickey_with(user, key, hash, &mut agent))
                .await?
            {
                Ok(AuthResult::Success) => return Ok(Outcome::Success),
                Ok(AuthResult::Failure {
                    remaining_methods, ..
                }) => {
                    self.refused(h, offered, remaining_methods);
                    if !offered.allows(MethodKind::PublicKey) {
                        break;
                    }
                }
                // The agent declined to sign this key: try the next.
                Err(_) => continue,
            }
        }
        Ok(Outcome::Failed)
    }

    async fn key(
        &self,
        h: &mut Handle<HopHandler>,
        path: &Path,
        offered: &mut Offered,
    ) -> Result<Outcome, SshError> {
        let Some((key, passphrase)) = self.load_key(path).await? else {
            return Ok(Outcome::Failed);
        };
        let hash = self.rsa_hash(h, key.public_key()).await?;
        let user = self.spec.user.clone();
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        match self.timed(h.authenticate_publickey(user, key)).await? {
            Ok(AuthResult::Success) => {
                self.record(None, passphrase);
                Ok(Outcome::Success)
            }
            Ok(AuthResult::Failure {
                remaining_methods, ..
            }) => {
                self.refused(h, offered, remaining_methods);
                Ok(Outcome::Failed)
            }
            Err(_) => Ok(Outcome::Failed),
        }
    }

    /// Reads and decrypts a key file: plain keys at once; encrypted ones with the stored
    /// passphrase, then the prompter, three attempts in all. `None`: unreadable or not unlocked.
    /// With the key, the passphrase that decrypted it.
    async fn load_key(
        &self,
        path: &Path,
    ) -> Result<Option<(PrivateKey, Option<SecretString>)>, SshError> {
        let file = path.to_path_buf();
        let Some(Some(text)) = blocking(move || read_key_file(&file)).await else {
            return Ok(None);
        };
        let plain = text.clone();
        match blocking(move || russh::keys::decode_secret_key(&plain, None)).await {
            Some(Ok(key)) => return Ok(Some((key, None))),
            Some(Err(russh::keys::Error::KeyIsEncrypted)) => {}
            _ if text.contains("ENCRYPTED") => {}
            _ => return Ok(None),
        }
        let stored = self.secrets.and_then(|s| s.passphrase.clone());
        let mut attempt = 0u8;
        while attempt < ATTEMPTS {
            attempt += 1;
            let passphrase = match (attempt, &stored) {
                (1, Some(p)) => p.clone(),
                _ => match self
                    .ask(
                        SshSecretKind::Passphrase {
                            key: path.to_path_buf(),
                        },
                        attempt,
                    )
                    .await?
                {
                    Some(p) => p,
                    None => return Ok(None),
                },
            };
            let text = text.clone();
            let tried = passphrase.clone();
            let decoded = blocking(move || {
                russh::keys::decode_secret_key(&text, Some(tried.expose_secret()))
            })
            .await;
            if let Some(Ok(key)) = decoded {
                return Ok(Some((key, Some(passphrase))));
            }
        }
        Ok(None)
    }

    async fn password(
        &self,
        h: &mut Handle<HopHandler>,
        offered: &mut Offered,
    ) -> Result<Outcome, SshError> {
        let stored = self.secrets.and_then(|s| s.password.clone());
        let mut attempt = 0u8;
        while attempt < ATTEMPTS && offered.allows(MethodKind::Password) {
            attempt += 1;
            let password = match (attempt, &stored) {
                (1, Some(p)) => p.clone(),
                _ => match self
                    .ask(
                        SshSecretKind::Password {
                            host: self.spec.host.clone(),
                            user: self.spec.user.clone(),
                        },
                        attempt,
                    )
                    .await?
                {
                    Some(p) => p,
                    None => return Ok(Outcome::Failed),
                },
            };
            let user = self.spec.user.clone();
            // russh copies the password into its own buffer (not zeroized; ADR 0003).
            match self
                .timed(h.authenticate_password(user, password.expose_secret()))
                .await?
            {
                Ok(AuthResult::Success) => {
                    self.record(Some(password), None);
                    return Ok(Outcome::Success);
                }
                Ok(AuthResult::Failure {
                    remaining_methods, ..
                }) => self.refused(h, offered, remaining_methods),
                Err(_) => return Ok(Outcome::Failed),
            }
        }
        Ok(Outcome::Failed)
    }

    async fn keyboard(
        &self,
        h: &mut Handle<HopHandler>,
        offered: &mut Offered,
    ) -> Result<Outcome, SshError> {
        for attempt in 1..=ATTEMPTS {
            if !offered.allows(MethodKind::KeyboardInteractive) {
                break;
            }
            let user = self.spec.user.clone();
            let Ok(mut response) = self
                .timed(h.authenticate_keyboard_interactive_start(user, None))
                .await?
            else {
                return Ok(Outcome::Failed);
            };
            let mut rounds = 0;
            loop {
                match response {
                    KeyboardInteractiveAuthResponse::Success => return Ok(Outcome::Success),
                    KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods, ..
                    } => {
                        self.refused(h, offered, remaining_methods);
                        break;
                    }
                    KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        rounds += 1;
                        if rounds > KBD_ROUNDS {
                            return Ok(Outcome::Failed);
                        }
                        let Some(answers) =
                            self.answer(name, instructions, prompts, attempt).await?
                        else {
                            return Ok(Outcome::Failed);
                        };
                        // russh takes `String`s (not zeroized; ADR 0003).
                        let answers = answers
                            .iter()
                            .map(|a| a.expose_secret().to_string())
                            .collect();
                        let Ok(next) = self
                            .timed(h.authenticate_keyboard_interactive_respond(answers))
                            .await?
                        else {
                            return Ok(Outcome::Failed);
                        };
                        response = next;
                    }
                }
            }
        }
        Ok(Outcome::Failed)
    }

    /// One keyboard-interactive round: the server's prompts, verbatim, to the prompter.
    /// `None`: no prompter for a round that has prompts.
    async fn answer(
        &self,
        name: String,
        instructions: String,
        prompts: Vec<client::Prompt>,
        attempt: u8,
    ) -> Result<Option<Vec<SecretString>>, SshError> {
        if prompts.is_empty() {
            return Ok(Some(Vec::new()));
        }
        let Some(prompter) = self.prompter else {
            return Ok(None);
        };
        let count = prompts.len();
        let req = KbdRequest {
            host: self.spec.host.clone(),
            name,
            instructions,
            prompts: prompts
                .into_iter()
                .map(|p| KbdPrompt {
                    text: p.prompt,
                    echo: p.echo,
                })
                .collect(),
        };
        let _paused = self.clock.pause();
        match self
            .cancel
            .run_until_cancelled(prompter.keyboard(self.hop, req, attempt))
            .await
        {
            Some(Some(answers)) if answers.len() == count => Ok(Some(answers)),
            _ => Err(SshError::Cancelled),
        }
    }
}

/// A key file's text in a zeroized buffer (at most 1 MiB, UTF-8).
fn read_key_file(path: &Path) -> Option<Zeroizing<String>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_KEY_FILE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_KEY_FILE {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?;
    Some(Zeroizing::new(text.to_string()))
}

/// The public key of an identity file, without its passphrase: an OpenSSH private key carries
/// it in the clear; other formats are read unencrypted or from `<file>.pub`.
fn identity_public(path: &Path) -> Option<PublicKey> {
    if let Some(text) = read_key_file(path) {
        if let Ok(key) = russh::keys::ssh_key::PrivateKey::from_openssh(text.as_bytes()) {
            return Some(key.public_key().clone());
        }
        if let Ok(key) = russh::keys::decode_secret_key(&text, None) {
            return Some(key.public_key().clone());
        }
    }
    let mut public = path.as_os_str().to_owned();
    public.push(".pub");
    russh::keys::load_public_key(PathBuf::from(public)).ok()
}

// ---------------------------------------------------------------------------------------------
// The connect clock

/// A deadline that stops while a prompt is open (P23, Q44: prompt time never counts).
struct Clock {
    state: Mutex<ClockState>,
    changed: Notify,
}

struct ClockState {
    deadline: Instant,
    paused: usize,
    paused_at: Option<Instant>,
}

/// Holds the clock still until dropped.
struct Paused<'c>(&'c Clock);

impl Drop for Paused<'_> {
    fn drop(&mut self) {
        {
            let mut s = lock(&self.0.state);
            s.paused = s.paused.saturating_sub(1);
            if s.paused == 0
                && let Some(at) = s.paused_at.take()
            {
                s.deadline += at.elapsed();
            }
        }
        self.0.changed.notify_waiters();
    }
}

impl Clock {
    fn new(limit: Duration) -> Arc<Clock> {
        Arc::new(Clock {
            state: Mutex::new(ClockState {
                deadline: Instant::now() + limit,
                paused: 0,
                paused_at: None,
            }),
            changed: Notify::new(),
        })
    }

    fn pause(&self) -> Paused<'_> {
        {
            let mut s = lock(&self.state);
            if s.paused == 0 {
                s.paused_at = Some(Instant::now());
            }
            s.paused += 1;
        }
        self.changed.notify_waiters();
        Paused(self)
    }

    /// Runs `fut` until it finishes (`Some`) or the deadline passes while not paused (`None`).
    async fn run<F: Future>(&self, fut: F) -> Option<F::Output> {
        let mut fut = std::pin::pin!(fut);
        loop {
            let changed = self.changed.notified();
            let mut changed = std::pin::pin!(changed);
            changed.as_mut().enable();
            let (deadline, paused) = {
                let s = lock(&self.state);
                (s.deadline, s.paused > 0)
            };
            let wake = async {
                if paused {
                    changed.await;
                } else {
                    let sleep = std::pin::pin!(tokio::time::sleep_until(deadline));
                    if let Either::Left(_) = select(sleep, changed).await {
                        return true;
                    }
                }
                false
            };
            match select(fut.as_mut(), std::pin::pin!(wake)).await {
                Either::Left((out, _)) => return Some(out),
                Either::Right((true, _)) => {
                    let s = lock(&self.state);
                    if s.paused == 0 && Instant::now() >= s.deadline {
                        return None;
                    }
                }
                Either::Right((false, _)) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::SshSpec;

    fn known() -> Arc<KnownHosts> {
        KnownHosts::load(Vec::new(), PathBuf::from("/nonexistent/known_hosts"))
    }

    #[tokio::test]
    async fn an_empty_or_overlong_chain_is_refused() {
        let tunnels = TunnelManager::with_agent(known(), TunnelConfig::default(), None);
        let req = TunnelRequest {
            spec: SshSpec::default(),
            secrets: Vec::new(),
            share: true,
            connect_timeout: Duration::from_secs(1),
        };
        let lease = tunnels.acquire(req, None, CancellationToken::new()).await;
        assert_eq!(lease.err(), Some(SshError::Protocol { hop: 0 }));
        assert_eq!(tunnels.sessions_open(), 0);
        assert_eq!(TunnelConfig::default().keepalive_max, 3);
    }

    /// T4 fix-round carry-in: an RSA host key is stored under its key type, `ssh-rsa`, which is
    /// what `known_hosts` lines name (never the signature algorithm `rsa-sha2-512`).
    #[test]
    fn rsa_host_keys_are_named_by_their_key_type() {
        let line = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQCpGnuVbYDjMk1Kj4KGuDM/itVvFDUvBrDMxibcekdSYC6iiUNAf66Qqqz+AWfNnl0C0qPQZUNE272SbZI6nFk5igB3d8mbm9xjiwTdAke/dGuxymhQtt2FDPb5clVdO5A6vVybfs2j6JYRyFgQvELyLO6LgJXNY8t3AoJmR5PkUw==";
        let key = russh::keys::parse_public_key_base64(line.split(' ').nth(1).unwrap()).unwrap();
        let host_key = host_key_of(&key).unwrap();
        assert_eq!(host_key.algorithm, "ssh-rsa");
        assert!(host_key.openssh.starts_with("ssh-rsa "));
        assert_eq!(HostKey::from_openssh(line), Some(host_key));
    }

    #[test]
    fn negotiation_offers_only_known_types_and_never_certificates_or_sha1() {
        let only = preferred(&["ssh-ed25519"]);
        assert_eq!(only.key.as_ref(), [Algorithm::Ed25519]);
        assert!(only.host_key_certificates.is_empty());
        let all = preferred(&[]);
        assert!(all.host_key_certificates.is_empty());
        assert!(!all.key.contains(&Algorithm::Rsa { hash: None }));
        assert_eq!(all.key.len(), 6);
        // Known only under a type russh cannot negotiate: offer all, the check refuses.
        assert_eq!(preferred(&["ssh-dss"]).key.len(), 6);
    }

    #[test]
    fn chain_keys_separate_users_auth_and_policy() {
        let hop = |user: &str, auth: SshAuth| HopSpec {
            host: "Bastion".into(),
            port: 22,
            user: user.into(),
            auth,
            identities_only: false,
            identity_files: Vec::new(),
        };
        let spec = |hops: Vec<HopSpec>, host_keys| SshSpec {
            hops,
            host_keys,
            keepalive_s: None,
        };
        let a = chain_key(&spec(vec![hop("u", SshAuth::Agent)], HostKeyPolicy::Ask));
        let mut lower = hop("u", SshAuth::Agent);
        lower.host = "bastion".into();
        assert_eq!(a, chain_key(&spec(vec![lower], HostKeyPolicy::Ask)));
        assert_ne!(
            a,
            chain_key(&spec(vec![hop("v", SshAuth::Agent)], HostKeyPolicy::Ask))
        );
        assert_ne!(
            a,
            chain_key(&spec(vec![hop("u", SshAuth::Password)], HostKeyPolicy::Ask))
        );
        assert_ne!(
            a,
            chain_key(&spec(vec![hop("u", SshAuth::Agent)], HostKeyPolicy::Strict))
        );
    }

    #[tokio::test]
    async fn the_clock_stops_while_paused() {
        let clock = Clock::new(Duration::from_millis(100));
        let slow = async {
            let _p = clock.pause();
            tokio::time::sleep(Duration::from_millis(300)).await;
        };
        assert!(clock.run(slow).await.is_some());
        let never = std::future::pending::<()>();
        assert!(clock.run(never).await.is_none());
    }
}
