//! Connect steps: the ordered list the pipeline runs; each may leave a cleanup guard
//! (HV-CONN-01, P4, P23). The built-in steps live here, one value per HV-CONN-01 phase:
//!
//! | order | phase | step | trail |
//! |---|---|---|---|
//! | 20 | CredentialFiles | `.pgpass` lookup (provider PgPass) | `PgPass` |
//! | 30 | NameResolution | resolve the original host | `Resolve` (skipped for a socket) |
//! | 40 | Transport | Direct factory, wrapped by `transport_wrap`; opens the first stream when the driver consumes the factory | `Connect` |
//! | 50 | Tls | build `TlsParams` (not for Disable or a socket) | — |
//! | 60 | Auth | `resolve_auth` on the blocking pool, the password prompt | — |
//! | 70 | DriverConnect | the driver, with the P8 retries and unknown-database suggestions | `Tls`, `SignIn` (from `DriverEvent`s), `OpenFile` |
//! | 90 | PostConnect | server check, R3 plaintext refusal, R11 version gate, R10 access label | `CheckServer` |
//!
//! The Karagöz transport is reserved at order 40 and rejected at Validate (R4). Only the
//! network steps (resolve, transport, driver, post-connect, extra steps) spend the connect
//! timeout; a keyring call or a prompt never does (P23).

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::errors::classify_tls_setup;
use hacivat_driver::{
    Driver, DriverConnect, DriverEvent, Endpoint, EngineCatalog, RawConn, RawError, RawErrorKind,
    ServerInfo, Support, classify, support_status,
};
use hacivat_net::{
    AddressClass, BoxedStream, CleanupGuard, NetError, StreamFactory, Target, TlsMode, TlsParams,
    TlsSettings, TlsSetupError, TlsSummary, build_tls_params, classify_host, direct, resolve,
};
use hacivat_proto::{AccessLabel, DbmsKind, ErrClass, FixId};
use hacivat_secrets::{
    AuthError, AuthMaterial, AuthOutcome, AuthProviderId, AuthRequest, CredentialFiles,
    SecretError, SecretSource, SecretStore, resolve_auth,
};
use secrecy::SecretString;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::trail::Recorder;
use super::validate::tls_issue;
use super::{
    Answer, ConnectError, ErrArgs, Interaction, Phase, Question, StepOutcome, TrailStep,
    TransportWrap, ValidationIssue,
};
// ## m2b-t14-pipeline
use super::prompts::{self, Retries};
use super::ssh_step::{SshState, SshStep};
use super::{ApprovedRoots, ProfileChange, Purpose, SessionSecrets};
use crate::profile::{Profile, ProfileEndpoint, TransportSpec};
use hacivat_net::ssh::{HopSecrets, KnownHosts, TunnelManager};

/// What a step works on (HV-CONN-01).
pub struct StepCx {
    pub profile: Arc<Profile>,
    pub cancel: CancellationToken,
    pub(crate) state: StepState,
}

impl StepCx {
    pub(crate) fn new(profile: Arc<Profile>, cancel: CancellationToken, state: StepState) -> Self {
        StepCx {
            profile,
            cancel,
            state,
        }
    }
}

/// One connect step, merged into the ordered list by `order` (HV-CONN-01 data structures).
pub trait ConnectStep: Send + Sync {
    fn phase(&self) -> Phase;
    fn order(&self) -> i32;
    fn applies(&self, p: &Profile) -> bool;
    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>>;
}

/// How long closing a connection may take when the pipeline gives it up.
pub(crate) const CLOSE_BUDGET: Duration = Duration::from_secs(2);

/// The most server rejections of a password per connect gesture (P8).
pub(crate) const MAX_REJECTIONS: u8 = 3;

/// A pre-opened stream older than this is not handed to the driver: a server or forwarder may
/// have dropped it while idle (MySQL's handshake waits about 10 s).
const STALE_STREAM: Duration = Duration::from_secs(5);

/// The slot holding the transport step's first stream until the factory hands it out.
pub(crate) type FirstStream = Arc<Mutex<Option<BoxedStream>>>;

/// How many database names the unknown-database probe reads, and how many it suggests.
const PROBE_LIST: usize = 50;
const SUGGESTIONS: usize = 3;

/// The connect timeout still left. Only network work spends it (P23).
pub(crate) struct Deadline {
    remaining: Duration,
}

impl Deadline {
    pub(crate) fn new(total: Duration) -> Self {
        Deadline { remaining: total }
    }

    pub(crate) fn remaining(&self) -> Duration {
        self.remaining
    }

    pub(crate) fn spend(&mut self, d: Duration) {
        self.remaining = self.remaining.saturating_sub(d);
    }

    /// Runs `fut` within the time left and spends what it took; `None` when time ran out.
    pub(crate) async fn run<T>(&mut self, fut: impl Future<Output = T>) -> Option<T> {
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(self.remaining, fut).await.ok();
        self.spend(started.elapsed());
        result
    }
}

/// The pipeline's working state between steps.
pub(crate) struct StepState {
    pub(crate) catalog: &'static EngineCatalog,
    pub(crate) secrets: Arc<SecretStore>,
    pub(crate) files: CredentialFiles,
    pub(crate) transport_wrap: Option<TransportWrap>,
    pub(crate) ix: Interaction,
    pub(crate) trail: Recorder,
    pub(crate) clock: Deadline,
    pub(crate) driver: Arc<dyn Driver>,
    /// Sanitized driver options (DRV-05).
    pub(crate) options: Vec<(String, String)>,
    /// The user the connect signs in as: the profile's, else the OS login (libpq parity).
    pub(crate) user: Option<String>,
    /// The profile's port, else the catalog default.
    pub(crate) port: Option<u16>,
    pub(crate) draft: Option<SecretString>,
    pub(crate) resolved: Vec<SocketAddr>,
    pub(crate) address: AddressClass,
    pub(crate) endpoint: Option<Endpoint>,
    /// The stream the transport step opened for the first attempt, and when.
    pub(crate) first_stream: Option<(FirstStream, tokio::time::Instant)>,
    /// The auth step waited on the user (a prompt) or the keyring after the transport opened
    /// its stream, so that stream may have idled out.
    pub(crate) interacted: bool,
    pub(crate) forwarded: bool,
    pub(crate) tls: Option<TlsParams>,
    pub(crate) auth: Option<AuthMaterial>,
    pub(crate) conn: Option<Box<dyn RawConn>>,
    pub(crate) tls_summary: Option<TlsSummary>,
    pub(crate) server: Option<ServerInfo>,
    pub(crate) support: Option<Support>,
    pub(crate) current_user: Option<String>,
    pub(crate) access: AccessLabel,
    /// Server rejections of a password so far (P8).
    pub(crate) rejections: u8,
    // ## m2b-t01-interfaces
    /// The profile or the session is read-only (`ConnectRequest::read_only`).
    pub(crate) read_only: bool,
    /// What the role probe found (Task 18).
    pub(crate) access_warnings: Vec<hacivat_guard::RoleWarning>,
    // ## m2b-t14-pipeline
    pub(crate) purpose: Purpose,
    /// The connect timeout: each SSH hop's own bound (Q44).
    pub(crate) timeout: Duration,
    pub(crate) approved_roots: ApprovedRoots,
    pub(crate) tunnels: Arc<TunnelManager>,
    pub(crate) known_hosts: Arc<KnownHosts>,
    /// The session overlay's secrets, used before the store (Q17); emptied as they are used.
    pub(crate) overlay: SessionSecrets,
    /// The dialog's typed SSH secrets (Test, Q23).
    pub(crate) draft_ssh: Vec<HopSecrets>,
    /// The TLS host (the database host, also under SSH; B11) and the effective TLS settings:
    /// the profile's plus this connect's changes (a pin, a CA file).
    pub(crate) tls_host: String,
    pub(crate) tls_settings: TlsSettings,
    /// The TLS client key passphrase in use, and whether the user typed it.
    pub(crate) tls_passphrase: Option<SecretString>,
    pub(crate) tls_passphrase_typed: bool,
    /// The TLS parameters of the last driver attempt (their recorded chain, Q20).
    pub(crate) attempt_tls: Option<TlsParams>,
    /// Consent to plaintext toward a public host: the overlay's (Q17) or this connect's answer.
    pub(crate) consent: bool,
    /// The consent was applied: the TLS parameters now allow the plaintext retry (review I1:
    /// only that retry, under Auto without a pin, sends `InsecureAuth::Allow` for it).
    pub(crate) plaintext_retry: bool,
    /// The profile changes this connect made.
    pub(crate) changes: Vec<ProfileChange>,
    pub(crate) retries: Retries,
    /// The vault unlock was offered in this connect (at most once).
    pub(crate) vault_tried: bool,
    pub(crate) ssh: SshState,
}

impl StepState {
    /// The auth source of the password in use, if any.
    pub(crate) fn source(&self) -> Option<&SecretSource> {
        self.auth.as_ref().map(|a| &a.source)
    }
}

/// A bare error of `class` in `phase`; the pipeline fills in the trail, arguments, fixes,
/// elapsed time and attempts when it returns it.
pub(crate) fn error(class: ErrClass, phase: Phase) -> ConnectError {
    ConnectError {
        class,
        phase,
        trail: Vec::new(),
        args: ErrArgs {
            name: String::new(),
            host: None,
            port: None,
            user: None,
            database: None,
            file: None,
            engine: String::new(),
            version: None,
            hop: None,
            hop_host: None,
        },
        server: None,
        fixes: Vec::new(),
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
        certificate: None,
        host_key: None,
        profile_changes: Vec::new(),
    }
}

/// The error of a raw driver or transport failure, classified by code (HV-CONN-03).
fn raw_failure(kind: DbmsKind, phase: Phase, raw: &RawError) -> ConnectError {
    let mut e = error(classify(kind, phase, raw).class, phase);
    e.server = raw.server.clone();
    e
}

fn timeout_error(phase: Phase) -> ConnectError {
    error(ErrClass::NetworkTimeout, phase)
}

/// A resolver or transport failure as a driver-shaped error (HV-CONN-03, T10 note 4).
pub(crate) fn net_raw(e: &NetError) -> RawError {
    use std::io::ErrorKind as K;
    let kind = match e {
        NetError::Dns { .. } => RawErrorKind::Dns,
        NetError::Refused => RawErrorKind::Io(K::ConnectionRefused),
        NetError::Timeout => RawErrorKind::Timeout,
        NetError::Unreachable => RawErrorKind::Io(K::HostUnreachable),
        NetError::Reset => RawErrorKind::Io(K::ConnectionReset),
        NetError::Io { kind, .. } => RawErrorKind::Io(*kind),
    };
    RawError { kind, server: None }
}

/// An I/O failure opening a stream.
fn io_raw(e: &std::io::Error) -> RawError {
    let kind = match e.kind() {
        std::io::ErrorKind::TimedOut => RawErrorKind::Timeout,
        k => RawErrorKind::Io(k),
    };
    RawError { kind, server: None }
}

/// `network.unreachable` toward a host that only has IPv6 addresses is
/// `network.ipv6_unavailable`, which `classify` never produces (T10 note 2).
fn ipv6_aware(mut e: ConnectError, resolved: &[SocketAddr]) -> ConnectError {
    if e.class == ErrClass::NetworkUnreachable
        && !resolved.is_empty()
        && resolved.iter().all(SocketAddr::is_ipv6)
    {
        e.class = ErrClass::NetworkIpv6Unavailable;
    }
    e
}

/// Closes `conn`, waiting at most [`CLOSE_BUDGET`].
pub(crate) async fn close_bounded(conn: Box<dyn RawConn>) {
    let _ = tokio::time::timeout(CLOSE_BUDGET, conn.close()).await;
}

/// The host of a server endpoint, trimmed.
pub(crate) fn server_host(p: &Profile) -> Option<&str> {
    match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => Some(host.trim()),
        _ => None,
    }
}

fn is_file(p: &Profile) -> bool {
    matches!(p.endpoint, ProfileEndpoint::File { .. })
}

// ## m2b-t14-pipeline
/// Whether `p` connects through an SSH tunnel.
pub(crate) fn is_ssh(p: &Profile) -> bool {
    matches!(p.transport, TransportSpec::SshTunnel(_))
}

/// The steps of HV-CONN-01, in order.
pub(crate) fn builtin_steps() -> Vec<Arc<dyn ConnectStep>> {
    vec![
        Arc::new(PgPassStep),
        Arc::new(ResolveStep),
        Arc::new(TransportStep),
        // ## m2b-t14-pipeline
        Arc::new(SshStep),
        Arc::new(TlsStep),
        Arc::new(AuthStep),
        Arc::new(DriverStep),
        Arc::new(PostConnectStep),
    ]
}

// ---------------------------------------------------------------------------------------------
// Auth helpers

/// Runs `resolve_auth` on the blocking pool (it reads files and may call the keyring).
fn resolve_blocking(
    p: &Profile,
    st: &StepState,
    host: String,
) -> impl Future<Output = Result<AuthOutcome, ConnectError>> + Send + 'static {
    let profile = p.id;
    let provider = p.auth;
    let engine = p.engine;
    let port = st.port.unwrap_or_default();
    let database = p.database.clone();
    let user = st.user.clone();
    let retention = p.retention;
    let draft = st.draft.clone();
    let secrets = st.secrets.clone();
    let files = st.files.clone();
    let task = tokio::task::spawn_blocking(move || {
        let req = AuthRequest {
            profile,
            provider,
            engine,
            host: &host,
            port,
            database: database.as_deref(),
            user: user.as_deref(),
            retention,
            draft: draft.as_ref(),
        };
        resolve_auth(&req, &secrets, &files)
    });
    async move {
        task.await
            .map_err(|_| error(ErrClass::Internal, Phase::Auth))
    }
}

/// The host a credential file is matched against: the original host, or the socket path.
fn lookup_host(p: &Profile) -> String {
    match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => host.trim().to_string(),
        ProfileEndpoint::Socket { path, .. } => path.display().to_string(),
        ProfileEndpoint::File { .. } => String::new(),
    }
}

/// Asks the user for the password (`attempt` from 1). Headless fails with
/// `needs_interaction`; a cancelled prompt cancels the connect.
async fn ask_password(
    p: &Profile,
    ix: &Interaction,
    user: String,
    attempt: u8,
) -> Result<SecretString, ConnectError> {
    match ix {
        Interaction::Headless => {
            let mut e = error(ErrClass::AuthInvalid, Phase::Auth);
            e.needs_interaction = true;
            Err(e)
        }
        Interaction::Interactive(ui) => {
            let q = Question::Password {
                profile: p.id,
                name: p.name.clone(),
                user,
                attempt,
            };
            match ui.ask(q).await {
                Answer::Password(pw) => Ok(pw),
                Answer::Cancelled => Err(error(ErrClass::Cancelled, Phase::Auth)),
                // ## m2b-t01-interfaces: any other answer to a password question cancels.
                _ => Err(error(ErrClass::Cancelled, Phase::Auth)),
            }
        }
    }
}

/// Whether `resolve_auth` will read the persistent store (the keyring, which may show a
/// system dialog): a password profile kept `Forever`, with no draft.
fn touches_keyring(p: &Profile, st: &StepState) -> bool {
    p.auth == AuthProviderId::Password
        && st.draft.is_none()
        && p.retention == hacivat_secrets::Retention::Forever
}

/// The error of a credential lookup that failed.
fn auth_error(e: &AuthError, phase: Phase) -> ConnectError {
    match e {
        AuthError::PgPassNoMatch | AuthError::PgPassFile(_) => error(ErrClass::AuthInvalid, phase),
        AuthError::Store(_) => error(ErrClass::Internal, phase),
    }
}

/// Turns a lookup into credentials, prompting when nothing is stored. A keyring that is
/// unavailable or refused counts as nothing stored, so the user is asked instead.
async fn material(
    p: &Profile,
    ix: &Interaction,
    fallback_user: Option<String>,
    outcome: AuthOutcome,
    phase: Phase,
    attempt: u8,
) -> Result<AuthMaterial, ConnectError> {
    let user = match outcome {
        AuthOutcome::Ready(m) => return Ok(m),
        AuthOutcome::NeedsPrompt { user } => user,
        AuthOutcome::Failed(AuthError::Store(
            SecretError::Unavailable
            | SecretError::Denied
            // ## m2b-t14-pipeline: a vault that stayed locked (headless, cancelled, three wrong
            // master passwords) or cannot be read: ask for the password instead (Q33).
            | SecretError::VaultLocked
            | SecretError::VaultCorrupt,
        )) => fallback_user.unwrap_or_default(),
        AuthOutcome::Failed(e) => return Err(auth_error(&e, phase)),
    };
    let password = ask_password(p, ix, user.clone(), attempt).await?;
    Ok(AuthMaterial {
        user: Some(user).filter(|u| !u.is_empty()),
        password: Some(password),
        source: SecretSource::Prompt,
    })
}

// ---------------------------------------------------------------------------------------------
// Order 20: credential files

struct PgPassStep;

impl ConnectStep for PgPassStep {
    fn phase(&self) -> Phase {
        Phase::CredentialFiles
    }

    fn order(&self) -> i32 {
        20
    }

    fn applies(&self, p: &Profile) -> bool {
        p.auth == AuthProviderId::PgPass && !is_file(p)
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let st = &mut cx.state;
            st.trail.start(TrailStep::PgPass);
            // The original host, port and database, before any transport rewrite.
            let outcome = resolve_blocking(&cx.profile, st, lookup_host(&cx.profile)).await?;
            if let AuthOutcome::Failed(e) = &outcome {
                return Err(auth_error(e, Phase::CredentialFiles));
            }
            st.trail.done(TrailStep::PgPass);
            let (ix, user) = (st.ix.clone(), st.user.clone());
            // The first prompt of the gesture is attempt 1; a re-entry continues the count.
            let attempt = st.rejections + 1;
            let m = material(
                &cx.profile,
                &ix,
                user,
                outcome,
                Phase::CredentialFiles,
                attempt,
            )
            .await?;
            // A prompt here comes before the transport opens its stream: nothing idles.
            st.interacted |= m.source == SecretSource::Prompt && st.first_stream.is_some();
            st.auth = Some(m);
            Ok(None)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Order 30: name resolution

struct ResolveStep;

impl ConnectStep for ResolveStep {
    fn phase(&self) -> Phase {
        Phase::NameResolution
    }

    fn order(&self) -> i32 {
        30
    }

    fn applies(&self, p: &Profile) -> bool {
        // ## m2b-t14-pipeline: through SSH the bastion resolves the database host; a local
        // lookup would also leak internal names (Q44).
        !is_file(p) && !is_ssh(p)
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let st = &mut cx.state;
            let Some(host) = server_host(&cx.profile) else {
                st.trail.skip(TrailStep::Resolve);
                st.address = AddressClass::UnixSocket;
                return Ok(None);
            };
            st.trail.start(TrailStep::Resolve);
            let port = st.port.unwrap_or_default();
            let resolved = st
                .clock
                .run(resolve(host, port))
                .await
                .ok_or_else(|| timeout_error(Phase::NameResolution))?
                .map_err(|e| raw_failure(cx.profile.engine, Phase::NameResolution, &net_raw(&e)))?;
            st.address = classify_host(host, &resolved);
            st.resolved = resolved;
            st.trail.done(TrailStep::Resolve);
            Ok(None)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Order 40: transport

struct TransportStep;

/// A factory that hands out `first` once, then opens new streams through `inner` (P23).
fn first_then(first: BoxedStream, inner: StreamFactory) -> (StreamFactory, FirstStream) {
    let slot: FirstStream = Arc::new(Mutex::new(Some(first)));
    let first = slot.clone();
    let factory = StreamFactory::new(move || {
        let taken = first
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        match taken {
            Some(stream) => Box::pin(async move { Ok(stream) }),
            None => inner.open(),
        }
    });
    (factory, slot)
}

impl ConnectStep for TransportStep {
    fn phase(&self) -> Phase {
        Phase::Transport
    }

    fn order(&self) -> i32 {
        40
    }

    fn applies(&self, p: &Profile) -> bool {
        // ## m2b-t14-pipeline: the SSH step is the transport of a tunnelled profile.
        !is_file(p) && !is_ssh(p)
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let p = cx.profile.clone();
            let st = &mut cx.state;
            let port = st.port.unwrap_or_default();
            let (target, tls_host) = match &p.endpoint {
                ProfileEndpoint::Server { host, .. } => (
                    Target::Tcp {
                        host: host.trim().to_string(),
                        port,
                    },
                    host.trim().to_string(),
                ),
                ProfileEndpoint::Socket { path, .. } => {
                    (Target::Unix { path: path.clone() }, "localhost".to_string())
                }
                ProfileEndpoint::File { .. } => return Ok(None),
            };
            let mut factory = direct(target.clone(), st.resolved.clone(), st.clock.remaining());
            let direct_target = match &st.transport_wrap {
                Some(wrap) => {
                    factory = wrap(factory);
                    None
                }
                None => Some(target),
            };
            hand_over(&p, st, factory, direct_target, tls_host, false).await?;
            Ok(None)
        })
    }
}

// ## m2b-t14-pipeline (shared by the direct and the SSH transport)
/// Hands the driver `factory` (P23, R18, P13). Only a driver that cannot take a stream dials a
/// direct TCP target itself (MySQL Direct); for every other one the pipeline opens the first
/// stream now (the Connect step's time) and the driver consumes the factory, directly or
/// through its loopback forwarder. Under SSH (`ssh`), a refused channel keeps its class
/// (`ssh.channel_open`, with the hop).
pub(crate) async fn hand_over(
    p: &Profile,
    st: &mut StepState,
    mut factory: StreamFactory,
    direct_target: Option<Target>,
    tls_host: String,
    ssh: bool,
) -> Result<(), ConnectError> {
    let caps = st.driver.descriptor().caps.clone();
    let driver_dials = !caps.stream_injection && matches!(direct_target, Some(Target::Tcp { .. }));
    if !driver_dials {
        // The pipeline opens the first stream itself, so its time is the Connect step.
        st.trail.start(TrailStep::Connect);
        let opened = st
            .clock
            .run(factory.open())
            .await
            .ok_or_else(|| timeout_error(Phase::Transport))?;
        let stream = opened.map_err(|e| {
            if ssh && let Some(ssh_error) = hacivat_net::ssh::classify_channel_err(&e) {
                let mut out = error(
                    hacivat_driver::classify_ssh(&ssh_error).class,
                    Phase::Transport,
                );
                if let hacivat_net::ssh::SshError::ChannelOpen { hop, .. } = ssh_error {
                    out.args.hop = Some(hop);
                    // Fix round 1 (review M6): the hop that refused, not always the bastion.
                    out.args.hop_host = st.ssh.hop_hosts.get(usize::from(hop)).cloned();
                }
                return out;
            }
            ipv6_aware(
                raw_failure(p.engine, Phase::Transport, &io_raw(&e)),
                &st.resolved,
            )
        })?;
        let (handing_out, slot) = first_then(stream, factory);
        factory = handing_out;
        st.first_stream = Some((slot, tokio::time::Instant::now()));
        st.trail.done(TrailStep::Connect);
    }
    st.forwarded = !caps.stream_injection && !driver_dials;
    st.endpoint = Some(Endpoint::Network {
        factory,
        direct: direct_target,
        resolved: st.resolved.clone(),
        tls_host,
    });
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Order 50: TLS

struct TlsStep;

impl ConnectStep for TlsStep {
    fn phase(&self) -> Phase {
        Phase::Tls
    }

    fn order(&self) -> i32 {
        50
    }

    fn applies(&self, p: &Profile) -> bool {
        server_host(p).is_some() && p.tls.mode != TlsMode::Disable
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            // ## m2b-t14-pipeline: the TLS host is the database host (also through SSH, B11),
            // the class is the transport's (`classify_host`, or `ssh_address_class`), the pins
            // come from the profile, and an encrypted client key is opened with the overlay's,
            // the stored or a typed passphrase (B25). The same parameters serve every engine
            // (MySQL through patch §5, Q2).
            let p = cx.profile.clone();
            let st = &mut cx.state;
            let key = p.tls.client_cert.as_ref().map(|c| c.key.clone());
            let mut passphrase = st.overlay.tls_key_passphrase.take();
            let mut built = build_tls(st, passphrase.clone()).await?;
            if matches!(built, Err(TlsSetupError::KeyEncrypted(_))) && passphrase.is_none() {
                let slot = hacivat_secrets::SecretKey {
                    profile: p.id,
                    slot: hacivat_secrets::SecretSlot::TlsKeyPassphrase,
                };
                let stored = prompts::read_store(st, vec![(slot, p.retention)]).await;
                if let Some(Some(stored)) = stored.into_iter().next() {
                    passphrase = Some(stored);
                    built = build_tls(st, passphrase.clone()).await?;
                }
            }
            let mut typed = false;
            let mut attempt = 0;
            while let (
                Err(TlsSetupError::KeyEncrypted(_) | TlsSetupError::KeyPassphraseWrong(_)),
                Some(key),
            ) = (&built, &key)
            {
                if attempt >= prompts::PASSPHRASE_ATTEMPTS {
                    break;
                }
                attempt += 1;
                let ix = st.ix.clone();
                if matches!(ix, Interaction::Headless) {
                    break;
                }
                // A prompt after the transport opened its first stream may let it idle out.
                st.interacted |= st.first_stream.is_some();
                let Some(answer) = prompts::ask_tls_passphrase(&p, &ix, key, attempt).await else {
                    return Err(error(ErrClass::Cancelled, Phase::Tls));
                };
                passphrase = Some(answer);
                typed = true;
                built = build_tls(st, passphrase.clone()).await?;
            }
            match built {
                Ok(params) => {
                    st.tls = params;
                    st.tls_passphrase = passphrase;
                    st.tls_passphrase_typed = typed;
                    Ok(None)
                }
                Err(e) => {
                    let mut err = tls_setup_error(&e);
                    err.needs_interaction = matches!(st.ix, Interaction::Headless)
                        && matches!(e, TlsSetupError::KeyEncrypted(_));
                    Err(err)
                }
            }
        })
    }
}

// ## m2b-t14-pipeline
/// The `config.invalid` error of a TLS setup problem, with its issue.
pub(crate) fn tls_setup_error(e: &TlsSetupError) -> ConnectError {
    let mut err = error(classify_tls_setup(e).class, Phase::Tls);
    let (field, code) = tls_issue(e);
    err.issues = vec![ValidationIssue { field, code }];
    err
}

/// Builds the TLS parameters from `st.tls_settings` toward `st.tls_host` and the transport's
/// address class, with `passphrase` for an encrypted client key; on the blocking pool (small
/// file reads). The outer error is a failed task; the inner one the setup problem. A profile
/// that remembered the public-plaintext answer (`allow_public_plaintext`, B7) may fall back to
/// plaintext under `Auto` toward any address, never on a pinned endpoint (Q13).
pub(crate) fn build_tls(
    st: &StepState,
    passphrase: Option<SecretString>,
) -> impl Future<Output = Result<Result<Option<TlsParams>, TlsSetupError>, ConnectError>> + Send + 'static
{
    let settings = st.tls_settings.clone();
    let host = st.tls_host.clone();
    let port = st.port.unwrap_or_default();
    let class = st.address;
    let task = tokio::task::spawn_blocking(move || {
        let built = build_tls_params(&settings, &host, port, class, passphrase.as_ref());
        built.map(|params| {
            params.map(|mut t| {
                if settings.mode == TlsMode::Auto
                    && settings.allow_public_plaintext
                    && settings.pin_for(&host, port).is_none()
                {
                    t.plaintext_allowed = true;
                }
                t
            })
        })
    });
    async move {
        task.await
            .map_err(|_| error(ErrClass::Internal, Phase::Tls))
    }
}

// ---------------------------------------------------------------------------------------------
// Order 60: auth

struct AuthStep;

impl ConnectStep for AuthStep {
    fn phase(&self) -> Phase {
        Phase::Auth
    }

    fn order(&self) -> i32 {
        60
    }

    fn applies(&self, p: &Profile) -> bool {
        !is_file(p)
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let st = &mut cx.state;
            if st.auth.is_some() {
                // The credential-files step already found them.
                return Ok(None);
            }
            // ## m2b-t14-pipeline: the session overlay's password (a reconnect or a side
            // connection, Q17) comes before the store; the dialog's draft before both.
            if cx.profile.auth == AuthProviderId::Password
                && st.draft.is_none()
                && let Some(password) = st.overlay.db.take()
            {
                st.auth = Some(AuthMaterial {
                    user: st.user.clone(),
                    password: Some(password),
                    source: SecretSource::Store(cx.profile.retention),
                });
                return Ok(None);
            }
            // Only a wait after the first stream was opened can let it idle out.
            let stream_open = st.first_stream.is_some();
            if stream_open && touches_keyring(&cx.profile, st) {
                st.interacted = true;
            }
            let mut outcome = resolve_blocking(&cx.profile, st, lookup_host(&cx.profile)).await?;
            // ## m2b-t14-pipeline: a locked vault is unlocked first (interactive, at most 3
            // attempts); otherwise the password is asked for as if nothing were stored.
            if matches!(
                outcome,
                AuthOutcome::Failed(AuthError::Store(SecretError::VaultLocked))
            ) && prompts::unlock_vault(st).await
            {
                outcome = resolve_blocking(&cx.profile, st, lookup_host(&cx.profile)).await?;
            }
            let (ix, user) = (st.ix.clone(), st.user.clone());
            let attempt = st.rejections + 1;
            let m = material(&cx.profile, &ix, user, outcome, Phase::Auth, attempt).await?;
            st.interacted |= stream_open && m.source == SecretSource::Prompt;
            st.auth = Some(m);
            Ok(None)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Order 70: driver connect

struct DriverStep;

/// Starts `req` on its own task and waits for it within the time left. A connection that
/// arrives after the pipeline gave up (timeout, cancel) is closed by that task.
async fn dial(
    driver: &Arc<dyn Driver>,
    req: DriverConnect,
    clock: &mut Deadline,
) -> Result<Box<dyn RawConn>, RawError> {
    let fut = driver.connect(req);
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let result = fut.await;
        if let Err(Ok(late)) = tx.send(result) {
            late.close().await;
        }
    });
    match clock.run(rx).await {
        None => Err(RawError {
            kind: RawErrorKind::Timeout,
            server: None,
        }),
        Some(Ok(result)) => result,
        Some(Err(_)) => Err(RawError {
            kind: RawErrorKind::Other,
            server: None,
        }),
    }
}

/// The request of one attempt; `database` overrides the profile's (the unknown-database
/// probe), and `events` reports to the trail.
fn request(
    p: &Profile,
    st: &StepState,
    database: Option<String>,
    events: hacivat_driver::DriverEvents,
) -> DriverConnect {
    let endpoint = match (&p.endpoint, &st.endpoint) {
        (
            ProfileEndpoint::File {
                path,
                create_if_missing,
            },
            _,
        ) => Endpoint::File {
            path: path.clone(),
            create_if_missing: *create_if_missing,
            // Fix round 1 (I1): the profile OR the session (`ConnectRequest::read_only`).
            read_only: st.read_only,
        },
        (_, Some(e)) => e.clone(),
        // The transport step always runs before for a network endpoint.
        (_, None) => Endpoint::File {
            path: PathBuf::new(),
            create_if_missing: false,
            read_only: true,
        },
    };
    let auth = st.auth.as_ref();
    DriverConnect {
        endpoint,
        // ## m2b-t14-pipeline: one verifier record per handshake (T3 carry-in): an attempt
        // never reads the chain, outcome or failure of the one before.
        tls: st.tls.as_ref().map(TlsParams::fork),
        user: auth
            .and_then(|a| a.user.clone())
            .or_else(|| st.user.clone()),
        password: auth.and_then(|a| a.password.clone()),
        database,
        application_name: p.application_name.clone(),
        read_only: st.read_only,
        options: st.options.clone(),
        connect_timeout: st.clock.remaining(),
        events,
        // ## m2b-t01-interfaces: the insecure-auth guard (Task 14 decides the policy).
        insecure_auth: insecure_auth(p, st),
    }
}

// ## m2b-t14-pipeline (fix round 1, review I1)
/// The insecure-auth guard of the next attempt (B8, Q20). The plaintext consent (answered now,
/// or carried by the session overlay) gives `Allow` only on the plaintext retry it answered:
/// under `Auto`, with no pin, and only while the TLS parameters allow plaintext. A pinned
/// endpoint never gets `Allow` from any consent, the remembered one included (Q13); otherwise
/// the profile's policy, [`super::insecure_auth_for`], applies.
fn insecure_auth(p: &Profile, st: &StepState) -> hacivat_driver::InsecureAuth {
    use hacivat_driver::InsecureAuth;
    let pinned = st
        .tls_settings
        .pin_for(&st.tls_host, st.port.unwrap_or_default())
        .is_some();
    let plaintext_attempt = st.tls.as_ref().is_some_and(|t| t.plaintext_allowed);
    let auto = p.tls.mode == TlsMode::Auto;
    if st.plaintext_retry && plaintext_attempt && auto && !pinned {
        return InsecureAuth::Allow;
    }
    let local = matches!(
        st.address,
        AddressClass::Loopback | AddressClass::UnixSocket
    );
    if pinned && !local && p.tls.mode != TlsMode::Disable {
        return InsecureAuth::RefuseUnlessVerified;
    }
    super::insecure_auth_for(p, st.address, p.tls.mode)
}

/// The trail's view of the driver's progress.
fn trail_events(trail: &Recorder) -> hacivat_driver::DriverEvents {
    let trail = trail.clone();
    Arc::new(move |e| {
        // A driver that dialled itself is connected once it reports anything.
        if trail.outcome(TrailStep::Connect) == Some(StepOutcome::Running) {
            trail.done_untimed(TrailStep::Connect);
        }
        match e {
            DriverEvent::TlsStarted => trail.start(TrailStep::Tls),
            DriverEvent::TlsDone => trail.done(TrailStep::Tls),
            DriverEvent::AuthStarted => trail.start(TrailStep::SignIn),
            DriverEvent::AuthDone => trail.done(TrailStep::SignIn),
        }
    })
}

/// Up to [`SUGGESTIONS`] names of `names` closest to `requested` by ASCII-case-insensitive
/// edit distance; ties go alphabetically. `requested` itself is never offered.
pub(crate) fn rank_suggestions(requested: &str, names: Vec<String>) -> Vec<String> {
    let want: Vec<u8> = requested.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let mut scored: Vec<(usize, String, String)> = names
        .into_iter()
        .filter(|n| n != requested)
        .map(|n| {
            let folded = n.to_ascii_lowercase();
            (edit_distance(&want, folded.as_bytes()), folded, n)
        })
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.2 == b.2);
    scored
        .into_iter()
        .take(SUGGESTIONS)
        .map(|(_, _, n)| n)
        .collect()
}

/// Levenshtein distance over bytes (names are compared after ASCII folding).
fn edit_distance(a: &[u8], b: &[u8]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(ca != cb);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Lists the databases the user can open, through a probe connection: `postgres`, then
/// `template1` for PostgreSQL; no database for MySQL and MariaDB. SQLite has no list.
async fn suggestions(p: &Profile, st: &mut StepState, requested: &str) -> Vec<String> {
    let candidates: Vec<Option<String>> = match p.engine {
        DbmsKind::Sqlite => return Vec::new(),
        DbmsKind::Postgres => vec![Some("postgres".into()), Some("template1".into())],
        _ => vec![None],
    };
    let quiet: hacivat_driver::DriverEvents = Arc::new(|_| {});
    for database in candidates {
        let req = request(p, st, database, quiet.clone());
        let driver = st.driver.clone();
        let Ok(mut conn) = dial(&driver, req, &mut st.clock).await else {
            continue;
        };
        let listed = st.clock.run(conn.list_databases(PROBE_LIST)).await;
        close_bounded(conn).await;
        return match listed {
            Some(Ok(names)) => rank_suggestions(requested, names),
            _ => Vec::new(),
        };
    }
    Vec::new()
}

impl ConnectStep for DriverStep {
    fn phase(&self) -> Phase {
        Phase::DriverConnect
    }

    fn order(&self) -> i32 {
        70
    }

    fn applies(&self, _p: &Profile) -> bool {
        true
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let p = cx.profile.clone();
            let st = &mut cx.state;
            let file = is_file(&p);
            if file {
                st.trail.start(TrailStep::OpenFile);
            } else if st.trail.outcome(TrailStep::Connect) == Some(StepOutcome::Pending) {
                // The driver dials itself: its connect runs now, untimed (P23).
                st.trail.start(TrailStep::Connect);
            }
            // A first stream that idled through a prompt or a keyring read, or for too long, is
            // dropped: the factory dials a fresh one for the driver (the Connect step keeps the
            // time it measured).
            if let Some((slot, opened)) = st.first_stream.take()
                && (st.interacted || opened.elapsed() > STALE_STREAM)
            {
                drop(
                    slot.lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take(),
                );
            }
            let events = trail_events(&st.trail);
            let conn = loop {
                let req = request(&p, st, p.database.clone(), events.clone());
                let sent_password = req.password.is_some();
                // ## m2b-t14-pipeline: this attempt's TLS parameters (its recorded chain, and
                // what it allowed: the post-connect backstop reads them, review I3).
                st.attempt_tls = req.tls.clone();
                let driver = st.driver.clone();
                let raw = match dial(&driver, req, &mut st.clock).await {
                    Ok(conn) => break conn,
                    Err(raw) => raw,
                };
                // `auth.invalid` includes `PasswordRequired` (the server wanted a password and
                // none was sent: PostgreSQL), and MySQL 1045 "using password: NO".
                let mut e = raw_failure(p.engine, Phase::DriverConnect, &raw);
                if e.class == ErrClass::AuthInvalid {
                    let user = st.auth.as_ref().and_then(|a| a.user.clone());
                    let user = user.or_else(|| st.user.clone()).unwrap_or_default();
                    let reprompt = if sent_password {
                        // A rejection. Only a password from this connect's prompt is asked for
                        // again (P8); a stored or draft one returns at once.
                        st.rejections += 1;
                        matches!(st.source(), Some(SecretSource::Prompt))
                            && st.rejections < MAX_REJECTIONS
                    } else {
                        // The server wants a password that nothing supplied: ask for it.
                        true
                    };
                    if reprompt {
                        let attempt = st.rejections + 1;
                        let ix = st.ix.clone();
                        let password = ask_password(&p, &ix, user.clone(), attempt).await;
                        let password = password.map_err(|mut ask| {
                            ask.server = e.server.clone();
                            ask
                        })?;
                        st.auth = Some(AuthMaterial {
                            user: Some(user).filter(|u| !u.is_empty()),
                            password: Some(password),
                            source: SecretSource::Prompt,
                        });
                        continue;
                    }
                    return Err(e);
                }
                // ## m2b-t14-pipeline: the certificate, plaintext and insecure-auth questions,
                // each with at most one retry per connect (B5–B8).
                if matches!(
                    e.class,
                    ErrClass::TlsUntrusted
                        | ErrClass::TlsPinChanged
                        | ErrClass::TlsNotSupportedByServer
                        | ErrClass::AuthInsecureMethod
                ) {
                    prompts::ask_and_retry(&p, st, &raw, e).await?;
                    continue;
                }
                if e.class == ErrClass::ConfigUnknownDatabase {
                    let requested = p.database.clone().unwrap_or_default();
                    e.suggestions = suggestions(&p, st, &requested).await;
                }
                return Err(ipv6_aware(e, &st.resolved));
            };
            let mut conn = conn;
            let summary = conn.tls_summary().await;
            st.conn = Some(conn);
            if file {
                st.tls_summary = Some(summary);
                st.trail.done(TrailStep::OpenFile);
                return Ok(None);
            }
            // A driver that dialled itself reported no connect time.
            if st.trail.outcome(TrailStep::Connect) != Some(StepOutcome::Done) {
                st.trail.done_untimed(TrailStep::Connect);
            }
            if st.trail.contains(TrailStep::Tls) {
                if !summary.encrypted {
                    // Auto fell back to plaintext (T14: loopback only), or the server said no.
                    st.trail.skip(TrailStep::Tls);
                } else if st.trail.outcome(TrailStep::Tls) != Some(StepOutcome::Done) {
                    st.trail.done_untimed(TrailStep::Tls);
                }
            }
            if st.trail.outcome(TrailStep::SignIn) != Some(StepOutcome::Done) {
                st.trail.done_untimed(TrailStep::SignIn);
            }
            st.tls_summary = Some(summary);
            Ok(None)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Order 90: post-connect

struct PostConnectStep;

/// The version as the error text shows it: the raw version's first word, without MariaDB's
/// `5.5.5-` handshake prefix or a `-suffix` (`9.6.24`, `16.4`, `10.6.12`).
pub(crate) fn short_version(info: &ServerInfo) -> String {
    let raw = info.raw_version.trim();
    let raw = raw.strip_prefix("5.5.5-").unwrap_or(raw);
    let word = raw.split_whitespace().next().unwrap_or_default();
    let word = word.split('-').next().unwrap_or_default();
    if word.is_empty() {
        info.version
            .as_ref()
            .map(|v| format!("{}.{}.{}", v.major, v.minor, v.patch))
            .unwrap_or_default()
    } else {
        word.to_string()
    }
}

impl ConnectStep for PostConnectStep {
    fn phase(&self) -> Phase {
        Phase::PostConnect
    }

    fn order(&self) -> i32 {
        90
    }

    fn applies(&self, _p: &Profile) -> bool {
        true
    }

    fn apply<'a>(
        &'a self,
        cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        Box::pin(async move {
            let p = cx.profile.clone();
            let st = &mut cx.state;
            st.trail.start(TrailStep::CheckServer);
            let Some(conn) = st.conn.as_mut() else {
                return Err(error(ErrClass::Internal, Phase::PostConnect));
            };
            let info = st
                .clock
                .run(conn.server_info())
                .await
                .ok_or_else(|| timeout_error(Phase::PostConnect))?
                .map_err(|raw| raw_failure(p.engine, Phase::PostConnect, &raw))?;
            st.server = Some(info.clone());

            // R3, behind P9: a server that answered in plaintext although TLS was asked for is
            // refused, except Auto's fallback toward a non-public address.
            let summary = st.tls_summary.clone().unwrap_or(TlsSummary {
                encrypted: false,
                version: None,
                cipher: None,
                verified: hacivat_net::TlsVerified::No,
            });
            // ## m2b-t14-pipeline (fix round 1, review I3): the backstop follows what the
            // connect asked for. Any unencrypted connection whose request did not allow
            // plaintext is refused: Require and Verify*, a public host under Auto without
            // consent, and a pinned endpoint at any address, through SSH too (Q13). Auto toward
            // a private address, or with the user's consent, allowed it (P9, B7).
            let plaintext_allowed = st.attempt_tls.as_ref().map(|t| t.plaintext_allowed);
            if !summary.encrypted && plaintext_allowed == Some(false) {
                let mut e = error(ErrClass::TlsNotSupportedByServer, Phase::PostConnect);
                if st
                    .tls_settings
                    .pin_for(&st.tls_host, st.port.unwrap_or_default())
                    .is_some()
                {
                    e.fixes = vec![FixId::EditConnection];
                }
                return Err(e);
            }

            // R11: below the floor stops here unless the user chose to connect anyway.
            let support = support_status(st.catalog, &info);
            if matches!(support, Support::Unsupported { .. }) && !p.allow_unsupported_server {
                return Err(error(
                    ErrClass::ServerUnsupportedVersion,
                    Phase::PostConnect,
                ));
            }
            st.support = Some(support);

            let user = match st.clock.run(conn.current_user()).await {
                Some(Ok(Some(user))) => Some(user),
                _ => None,
            };
            st.current_user = user
                .or_else(|| st.auth.as_ref().and_then(|a| a.user.clone()))
                .or_else(|| st.user.clone());
            // ## m2b-t01-interfaces: the access label and the role probe's warnings (Task 18;
            // until then the M2a R10 label).
            let (access, warnings) =
                crate::session::readonly::access_for(&mut **conn, p.engine, st.read_only).await;
            st.access = access;
            st.access_warnings = warnings;
            st.trail.done(TrailStep::CheckServer);
            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_rank_by_folded_edit_distance_then_name() {
        let names = [
            "postgres",
            "orders_archive",
            "Orders",
            "analytics",
            "orders_v2",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            rank_suggestions("orders_v2", names),
            ["Orders", "orders_archive", "postgres"]
        );
        assert_eq!(edit_distance(b"kitten", b"sitting"), 3);
        assert_eq!(edit_distance(b"", b"abc"), 3);
        // Ties go alphabetically.
        let tied = ["bb", "ab", "ba"].map(String::from).to_vec();
        assert_eq!(rank_suggestions("aa", tied), ["ab", "ba", "bb"]);
    }

    #[test]
    fn versions_read_as_the_error_text_shows_them() {
        let info = |raw: &str| ServerInfo {
            kind: DbmsKind::Postgres,
            product: String::new(),
            version: hacivat_driver::SemVerLoose::parse(raw),
            raw_version: raw.into(),
            is_replica: None,
        };
        assert_eq!(short_version(&info("9.6.24")), "9.6.24");
        assert_eq!(
            short_version(&info("16.4 (Debian 16.4-1.pgdg120+1)")),
            "16.4"
        );
        assert_eq!(short_version(&info("5.5.5-10.5.2-MariaDB")), "10.5.2");
    }

    #[test]
    fn net_errors_map_to_raw_kinds_that_classify() {
        use std::io::ErrorKind as K;
        let cases = [
            (NetError::Dns { host: "x".into() }, ErrClass::NetworkDns),
            (NetError::Refused, ErrClass::NetworkRefused),
            (NetError::Timeout, ErrClass::NetworkTimeout),
            (NetError::Unreachable, ErrClass::NetworkUnreachable),
            (NetError::Reset, ErrClass::NetworkReset),
            (
                NetError::Io {
                    kind: K::ConnectionAborted,
                    message: String::new(),
                },
                ErrClass::NetworkReset,
            ),
        ];
        for (e, class) in cases {
            let c = classify(DbmsKind::Postgres, Phase::Transport, &net_raw(&e));
            assert_eq!(c.class, class, "{e:?}");
        }
        let v6: SocketAddr = "[2001:db8::1]:5432".parse().unwrap();
        let v4: SocketAddr = "192.0.2.1:5432".parse().unwrap();
        let unreachable = || error(ErrClass::NetworkUnreachable, Phase::Transport);
        assert_eq!(
            ipv6_aware(unreachable(), &[v6]).class,
            ErrClass::NetworkIpv6Unavailable
        );
        assert_eq!(
            ipv6_aware(unreachable(), &[v6, v4]).class,
            ErrClass::NetworkUnreachable
        );
    }
}
