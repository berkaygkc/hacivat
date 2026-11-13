//! The connect pipeline (HV-CONN-01): validate, credential files, resolve, transport, TLS,
//! sign-in, driver connect and post-connect checks, with a step trail, cancellation, cleanup
//! guards and classified errors.
//!
//! [`connect`] validates the profile (step 0), then runs an ordered list of
//! [`ConnectStep`] values: the built-in steps of [`steps`] merged with
//! `ConnectDeps::extra_steps` by `order`. A step may leave a [`CleanupGuard`]; on failure or
//! cancel the open connection is closed and the guards run LIFO, and a driver result that
//! arrives after the pipeline gave up is closed by the task that awaited it.

mod interaction;
// ## m2b-t01-interfaces: Task 14 fills them.
mod prompts;
mod ssh_step;
mod steps;
mod trail;
mod validate;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::InsecureAuth;
use hacivat_driver::{
    DriverRegistry, EngineCatalog, RawConn, ServerInfo, ServerMessage, Support, fixes_for,
};
use hacivat_guard::RoleWarning;
use hacivat_net::ssh::{
    HopSecrets, HostKey, KnownHosts, KnownSource, TransportGeneration, TunnelManager,
};
use hacivat_net::{
    AddressClass, CertPin, CertReview, CleanupGuard, StreamFactory, TlsMode, TlsSummary,
    TlsVerified,
};
use hacivat_proto::{AccessLabel, DbmsKind, Department, ErrClass, ErrFamily, FixId};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, Retention, SecretKey, SecretSlot, SecretSource, SecretStore,
};
use secrecy::SecretString;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::manager::PathKind;

pub use hacivat_driver::errors::Phase;
pub use interaction::{Answer, Interaction, Question, SecretPrompt, UiPort};
pub use steps::{ConnectStep, StepCx};
pub use trail::{Progress, StepOutcome, StepTrace, TrailStep};
pub use validate::{ApprovedRoots, IssueCode, ValidationIssue, validate, validate_with};

pub(crate) use steps::{CLOSE_BUDGET, error, short_version};
use steps::{Deadline, MAX_REJECTIONS, StepState, builtin_steps, close_bounded};
use trail::Recorder;

use crate::profile::{Profile, ProfileEndpoint};

/// Why a connect runs (HV-CONN-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    /// A session for the user's work.
    Main,
    /// The dialog's Test button: nothing is stored.
    Test,
    /// Background introspection.
    Probe,
}

/// The arguments of an error's Fluent messages; never English text (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrArgs {
    pub name: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub file: Option<String>,
    pub engine: String,
    pub version: Option<String>,
    // ## m2b-t01-interfaces
    /// The SSH hop the error happened at (from 0), and its host.
    pub hop: Option<u8>,
    pub hop_host: Option<String>,
}

// ## m2b-t01-interfaces

/// A change to the saved profile that a connect asks for (Task 14) or a review makes (Task 19);
/// the app applies it through `ConnectionsModel::apply_changes`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProfileChange {
    AddPin(CertPin),
    /// Built only by `cert_dialog::review` (Q26); the pipeline never builds it.
    ReplacePin {
        old: [u8; 32],
        new: CertPin,
    },
    SetCaFile(PathBuf),
    AllowPublicPlaintext,
}

/// The secrets that worked for a live connection, kept in the session overlay for its
/// reconnects and side connections, never persisted (Q17). `Debug` only (redacted).
#[derive(Debug, Default)]
pub struct SessionSecrets {
    pub db: Option<SecretString>,
    pub hops: Vec<HopSecrets>,
    pub tls_key_passphrase: Option<SecretString>,
}

/// A changed SSH host key, for the "Review host key" fix (Task 21).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostKeyMismatch {
    pub hop: u8,
    pub host: String,
    pub port: u16,
    pub old: HostKey,
    pub new: HostKey,
    pub source: KnownSource,
}

/// Whether the drivers may send a password by an insecure method for `profile` over an
/// address of `class` with TLS `mode` (B8, Q20, Q23). Task 1 stub: always refuses unless the
/// channel is verified (the strictest answer) until Task 14.
pub fn insecure_auth_for(_profile: &Profile, _class: AddressClass, _mode: TlsMode) -> InsecureAuth {
    InsecureAuth::RefuseUnlessVerified
}

/// A failed connect, classified (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectError {
    pub class: ErrClass,
    pub phase: Phase,
    pub trail: Vec<StepTrace>,
    pub args: ErrArgs,
    /// The server's message, shown verbatim (I18N-14).
    pub server: Option<ServerMessage>,
    pub fixes: Vec<FixId>,
    /// Names to offer (`UseDatabase`).
    pub suggestions: Vec<String>,
    pub issues: Vec<ValidationIssue>,
    pub elapsed: Duration,
    pub attempts: u8,
    /// A headless connect stopped where it would have asked the user.
    pub needs_interaction: bool,
    // ## m2b-t01-interfaces
    /// The certificate to review (`tls.untrusted`, `tls.pin_changed`; Task 14).
    pub certificate: Option<CertReview>,
    /// The changed host key (`ssh.host_key_changed`; Task 14).
    pub host_key: Option<HostKeyMismatch>,
    /// Changes the failed connect still asks the profile for.
    pub profile_changes: Vec<ProfileChange>,
}

/// One connect (HV-CONN-01). `department` is `Console` for Main, `Unknown` for Test and
/// `Introspection` for Probe (§5.8, P44). Holds a secret, so `Debug` only.
#[derive(Debug)]
pub struct ConnectRequest {
    pub profile: Arc<Profile>,
    pub draft_password: Option<SecretString>,
    pub purpose: Purpose,
    pub department: Department,
    /// Server rejections earlier in the same connect gesture: the error dialog's password
    /// re-entry passes the failed connect's `attempts`, so the gesture keeps one budget of 3
    /// (P8, R8). 0 for a new gesture.
    pub prior_rejections: u8,
    // ## m2b-t01-interfaces
    /// `profile.read_only` or a read-only session; the connect is read-only if either is.
    pub read_only: bool,
    /// The SSH secrets typed in the dialog, for Test (Q23).
    pub draft_ssh: Vec<HopSecrets>,
    /// The session overlay's secrets, used before the store (reconnects, side connections;
    /// Q17).
    pub overlay_secrets: Option<SessionSecrets>,
    /// The overlay's one-gesture consent to the public-plaintext question (Q17).
    pub plaintext_consent: bool,
}

/// An open connection with what the pipeline learned (HV-CONN-01).
pub struct LiveConnection {
    pub server: ServerInfo,
    pub support: Support,
    pub tls: TlsSummary,
    pub address: AddressClass,
    pub auth: AuthProviderId,
    pub user: Option<String>,
    pub access: AccessLabel,
    pub trail: Vec<StepTrace>,
    // ## m2b-t01-interfaces
    /// Profile changes the connect made (a pin, a CA file, the plaintext consent; Task 14).
    pub profile_changes: Vec<ProfileChange>,
    /// The SSH chain generation the connection runs over.
    pub transport: Option<TransportGeneration>,
    pub path: PathKind,
    /// The bastion, as the capsule names an SSH path.
    pub via: Option<String>,
    /// What the role probe found (Task 18).
    pub access_warnings: Vec<RoleWarning>,
    /// The secrets that worked, for the session overlay (Q17).
    pub secrets_used: SessionSecrets,
    pub plaintext_consent: bool,
    conn: Box<dyn RawConn>,
    guards: Vec<CleanupGuard>,
    forwarded: bool,
}

impl fmt::Debug for LiveConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveConnection")
            .field("server", &self.server)
            .field("support", &self.support)
            .field("tls", &self.tls)
            .field("address", &self.address)
            .field("auth", &self.auth)
            .field("user", &self.user)
            .field("access", &self.access)
            .field("trail", &self.trail)
            .field("profile_changes", &self.profile_changes)
            .field("transport", &self.transport)
            .field("path", &self.path)
            .field("via", &self.via)
            .field("access_warnings", &self.access_warnings)
            .field("secrets_used", &self.secrets_used)
            .field("plaintext_consent", &self.plaintext_consent)
            .field("guards", &self.guards.len())
            .field("forwarded", &self.forwarded)
            .finish_non_exhaustive()
    }
}

impl LiveConnection {
    pub fn raw(&mut self) -> &mut dyn RawConn {
        &mut *self.conn
    }

    /// Closes the connection, then runs its cleanup guards, last first.
    pub fn close(self) -> BoxFuture<'static, ()> {
        let LiveConnection { conn, guards, .. } = self;
        Box::pin(async move {
            conn.close().await;
            for guard in guards.into_iter().rev() {
                guard.run().await;
            }
        })
    }

    /// Whether the driver went through the loopback forwarder (P13).
    pub fn forwarded(&self) -> bool {
        self.forwarded
    }
}

/// Wraps a stream factory in a transport (SSH, proxies; tests).
pub type TransportWrap = Arc<dyn Fn(StreamFactory) -> StreamFactory + Send + Sync>;

/// What the pipeline needs (HV-CONN-01, P23).
pub struct ConnectDeps {
    pub drivers: Arc<DriverRegistry>,
    pub catalog: &'static EngineCatalog,
    pub secrets: Arc<SecretStore>,
    pub files: CredentialFiles,
    pub transport_wrap: Option<TransportWrap>,
    /// Extra steps merged into the ordered list by `order` (the cleanup-guard test seam).
    pub extra_steps: Vec<Arc<dyn ConnectStep>>,
    /// Where files named by an untrusted-origin profile may live (DRV-05): the user's home and
    /// the paths picked in the dialog ([`ApprovedRoots::for_user`], then `add`).
    pub approved_roots: ApprovedRoots,
    /// Replaces the profile's connect timeout; `None` in the app, set only by tests.
    #[doc(hidden)]
    pub timeout_override: Option<Duration>,
    // ## m2b-t01-interfaces
    /// The SSH chains (Task 9), shared by every connect.
    pub tunnels: Arc<TunnelManager>,
    /// The known-host stores (Task 4).
    pub known_hosts: Arc<KnownHosts>,
}

/// Runs the pipeline for `req` (HV-CONN-01 steps 0–10).
///
/// - **Validate** (step 0, [`validate`]) runs before any I/O; a problem is `config.invalid`
///   with the issues and no trail (R25). A Test ignores a missing name (the dialog suggests
///   one at save).
/// - The steps run in `order`; the trail follows the mock's `stepsFor` (P43).
/// - The connect timeout (the profile's, default 15 s) covers the network steps only: a
///   keyring read or a prompt never counts (P23). Running out is `network.timeout`.
/// - Retries (P8): a password from this connect's prompt that the server rejects is asked for
///   again, up to 3 rejections in the gesture (counting `prior_rejections`); a stored or draft
///   password that is rejected returns `auth.invalid` at once. `attempts` counts the gesture's
///   server rejections. A gesture that already has 3 fails at once, before any I/O.
/// - On `config.unknown_database` a probe connection lists up to 50 databases, and the 3
///   closest names become `suggestions` (none for SQLite).
/// - A password that worked (prompted, or the draft) is stored with the profile's retention
///   only after success, never by a Test, and never once the connect was cancelled (R8).
/// - On failure or cancel the connection is closed and the cleanup guards run LIFO.
pub async fn connect(
    deps: &ConnectDeps,
    req: ConnectRequest,
    ix: Interaction,
    cancel: CancellationToken,
    progress: watch::Sender<Progress>,
) -> Result<LiveConnection, ConnectError> {
    let span = tracing::debug_span!(
        "connect",
        profile = %req.profile.id,
        engine = req.profile.engine.id(),
        purpose = ?req.purpose,
        department = ?req.department,
    );
    run(deps, req, ix, cancel, progress).instrument(span).await
}

async fn run(
    deps: &ConnectDeps,
    req: ConnectRequest,
    ix: Interaction,
    cancel: CancellationToken,
    progress: watch::Sender<Progress>,
) -> Result<LiveConnection, ConnectError> {
    let started = tokio::time::Instant::now();
    let ConnectRequest {
        profile,
        draft_password,
        purpose,
        department: _,
        prior_rejections,
        read_only,
        // ## m2b-t01-interfaces: Task 14 uses the SSH drafts and the overlay.
        draft_ssh: _,
        overlay_secrets: _,
        plaintext_consent,
    } = req;
    // A connect is read-only if the profile or the session is (never weaker than the profile).
    let read_only = read_only || profile.read_only;
    // The typed settings only: an extra option never overrides them (M2a final review I2).
    let port = effective_port(&profile, deps.catalog);
    let user = effective_user(&profile);

    let (mut issues, options) =
        validate::checked(&profile, deps.catalog, &deps.approved_roots.list());
    if purpose == Purpose::Test {
        issues.retain(|i| i.code != IssueCode::NameMissing);
    }
    let fail_early = |mut e: ConnectError| {
        progress.send_replace(fresh_progress());
        e.args = args(&profile, port, user.clone(), None);
        e.fixes = fixes_for(e.class);
        e.elapsed = started.elapsed();
        tracing::debug!(class = e.class.id(), "connect refused before any I/O");
        e
    };
    if !issues.is_empty() {
        let mut e = error(ErrClass::ConfigInvalid, Phase::Validate);
        e.issues = issues;
        return Err(fail_early(e));
    }
    let Some(driver) = deps.drivers.resolve(profile.engine) else {
        return Err(fail_early(error(ErrClass::Internal, Phase::Validate)));
    };
    if prior_rejections >= MAX_REJECTIONS {
        // The gesture's budget of rejections is spent (P8): no more attempts, no I/O.
        let mut e = fail_early(error(ErrClass::AuthInvalid, Phase::Auth));
        e.attempts = prior_rejections;
        return Err(e);
    }

    let file_based = matches!(profile.endpoint, ProfileEndpoint::File { .. });
    let trail = Recorder::new(&plan(&profile), progress);
    let timeout = deps
        .timeout_override
        .unwrap_or_else(|| Duration::from_secs(u64::from(profile.connect_timeout_s.max(1))));
    let state = StepState {
        catalog: deps.catalog,
        secrets: deps.secrets.clone(),
        files: deps.files.clone(),
        transport_wrap: deps.transport_wrap.clone(),
        ix,
        trail,
        clock: Deadline::new(timeout),
        driver,
        options,
        user,
        port,
        draft: draft_password,
        resolved: Vec::new(),
        address: if file_based {
            AddressClass::Loopback
        } else {
            AddressClass::Public
        },
        endpoint: None,
        first_stream: None,
        interacted: false,
        forwarded: false,
        tls: None,
        auth: None,
        conn: None,
        tls_summary: None,
        server: None,
        support: None,
        current_user: None,
        access: AccessLabel::ReadWrite,
        rejections: prior_rejections,
        read_only,
        access_warnings: Vec::new(),
    };
    let mut cx = StepCx::new(profile.clone(), cancel.clone(), state);

    // Built-in steps first on equal orders; `sort_by_key` is stable.
    let mut ordered: Vec<(Arc<dyn ConnectStep>, bool)> = builtin_steps()
        .into_iter()
        .map(|s| (s, false))
        .chain(deps.extra_steps.iter().cloned().map(|s| (s, true)))
        .collect();
    ordered.sort_by_key(|(s, _)| s.order());

    let mut guards: Vec<CleanupGuard> = Vec::new();
    for (step, extra) in &ordered {
        if !step.applies(&profile) {
            continue;
        }
        let phase = step.phase();
        let result = if *extra {
            // Extra steps (transports) spend the connect timeout like the network steps.
            let remaining = cx.state.clock.remaining();
            let t0 = tokio::time::Instant::now();
            let r = tokio::select! {
                biased;
                () = cancel.cancelled() => Err(error(ErrClass::Cancelled, phase)),
                r = tokio::time::timeout(remaining, step.apply(&mut cx)) => {
                    r.unwrap_or_else(|_| Err(error(ErrClass::NetworkTimeout, phase)))
                }
            };
            cx.state.clock.spend(t0.elapsed());
            r
        } else {
            tokio::select! {
                biased;
                () = cancel.cancelled() => Err(error(ErrClass::Cancelled, phase)),
                r = step.apply(&mut cx) => r,
            }
        };
        match result {
            Ok(Some(guard)) => guards.push(guard),
            Ok(None) => {}
            Err(e) => return Err(fail(e, cx, guards, started).await),
        }
    }
    if cancel.is_cancelled() {
        let e = error(ErrClass::Cancelled, Phase::PostConnect);
        return Err(fail(e, cx, guards, started).await);
    }

    let (conn, server, support) = (
        cx.state.conn.take(),
        cx.state.server.take(),
        cx.state.support.take(),
    );
    let (Some(conn), Some(server), Some(support)) = (conn, server, support) else {
        // The driver and post-connect steps always apply; this is a defect, not a server's.
        let e = error(ErrClass::Internal, Phase::PostConnect);
        return Err(fail(e, cx, guards, started).await);
    };
    let StepCx { state: st, .. } = cx;
    remember_password(&profile, purpose, &st.secrets, st.auth.as_ref(), &cancel).await;
    let trail = st.trail.snapshot();
    tracing::debug!(
        trail = ?trail.iter().map(|t| t.step.id()).collect::<Vec<_>>(),
        "connected"
    );
    Ok(LiveConnection {
        server,
        support,
        tls: st.tls_summary.unwrap_or(TlsSummary {
            encrypted: false,
            version: None,
            cipher: None,
            verified: TlsVerified::No,
        }),
        address: st.address,
        auth: profile.auth,
        user: st.current_user,
        access: st.access,
        trail,
        profile_changes: Vec::new(),
        transport: None,
        path: PathKind::Direct,
        via: None,
        access_warnings: st.access_warnings,
        secrets_used: SessionSecrets::default(),
        plaintext_consent,
        conn,
        guards,
        forwarded: st.forwarded,
    })
}

/// Closes what the failed connect opened, runs the guards LIFO and completes the error.
async fn fail(
    mut e: ConnectError,
    cx: StepCx,
    guards: Vec<CleanupGuard>,
    started: tokio::time::Instant,
) -> ConnectError {
    let StepCx { profile, state, .. } = cx;
    let StepState {
        conn,
        trail,
        port,
        user,
        current_user,
        server,
        rejections,
        auth,
        ..
    } = state;
    if let Some(conn) = conn {
        close_bounded(conn).await;
    }
    for guard in guards.into_iter().rev() {
        let _ = tokio::time::timeout(CLOSE_BUDGET, guard.run()).await;
    }
    drop(auth); // the secret goes now (step 10)
    e.trail = if e.class == ErrClass::ConfigInvalid {
        trail.clear();
        Vec::new()
    } else {
        trail.fail(failure_step(&e, &profile))
    };
    let user = current_user.or(user);
    e.args = args(&profile, port, user, server.as_ref());
    if e.fixes.is_empty() {
        e.fixes = fixes_for(e.class);
    }
    e.elapsed = started.elapsed();
    e.attempts = rejections;
    tracing::debug!(class = e.class.id(), phase = ?e.phase, attempts = e.attempts, "connect failed");
    e
}

/// The trail step a failure shows at (R25). `None` is the step running when it failed.
fn failure_step(e: &ConnectError, p: &Profile) -> Option<TrailStep> {
    if e.phase == Phase::CredentialFiles {
        return Some(TrailStep::PgPass);
    }
    if matches!(e.class, ErrClass::Cancelled | ErrClass::Internal) {
        // A prompt is part of signing in.
        return (e.phase == Phase::Auth).then_some(TrailStep::SignIn);
    }
    if e.class == ErrClass::ServerUnsupportedVersion {
        return Some(TrailStep::CheckServer);
    }
    if matches!(p.endpoint, ProfileEndpoint::File { .. }) {
        return Some(TrailStep::OpenFile);
    }
    Some(match e.class.family() {
        ErrFamily::Network if e.class == ErrClass::NetworkDns => TrailStep::Resolve,
        ErrFamily::Network | ErrFamily::Ssh | ErrFamily::Karagoz => TrailStep::Connect,
        ErrFamily::Tls => TrailStep::Tls,
        ErrFamily::Auth
        | ErrFamily::Authz
        | ErrFamily::Config
        | ErrFamily::Server
        | ErrFamily::Cancelled
        | ErrFamily::Internal => TrailStep::SignIn,
    })
}

/// The visible steps of `p`'s connect, as the mock's `stepsFor` (P43): TLS is left out when
/// disabled (and over a socket, which never negotiates it).
fn plan(p: &Profile) -> Vec<TrailStep> {
    match &p.endpoint {
        ProfileEndpoint::File { .. } => vec![TrailStep::OpenFile, TrailStep::CheckServer],
        endpoint => {
            let mut steps = Vec::new();
            if p.auth == AuthProviderId::PgPass {
                steps.push(TrailStep::PgPass);
            }
            steps.extend([TrailStep::Resolve, TrailStep::Connect]);
            if matches!(endpoint, ProfileEndpoint::Server { .. })
                && p.tls.mode != hacivat_net::TlsMode::Disable
            {
                steps.push(TrailStep::Tls);
            }
            steps.extend([TrailStep::SignIn, TrailStep::CheckServer]);
            steps
        }
    }
}

/// The profile's port, else the engine's default.
fn effective_port(p: &Profile, catalog: &EngineCatalog) -> Option<u16> {
    let port = match &p.endpoint {
        ProfileEndpoint::Server { port, .. } | ProfileEndpoint::Socket { port, .. } => *port,
        ProfileEndpoint::File { .. } => return None,
    };
    port.or_else(|| catalog.get(p.engine).and_then(|e| e.default_port))
}

/// The user a network connect signs in as: the profile's, else the OS login name, as libpq
/// and the MySQL client do. SQLite has none.
fn effective_user(p: &Profile) -> Option<String> {
    if matches!(p.endpoint, ProfileEndpoint::File { .. }) || p.engine == DbmsKind::Sqlite {
        return None;
    }
    p.user.clone().filter(|u| !u.trim().is_empty()).or_else(|| {
        ["USER", "USERNAME"]
            .into_iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
    })
}

/// The arguments of `p`'s error messages; the engine is the server's when it is known.
fn args(
    p: &Profile,
    port: Option<u16>,
    user: Option<String>,
    server: Option<&ServerInfo>,
) -> ErrArgs {
    let (host, file) = match &p.endpoint {
        ProfileEndpoint::Server { host, .. } => (Some(host.trim().to_string()), None),
        ProfileEndpoint::Socket { path, .. } => (None, Some(path.display().to_string())),
        ProfileEndpoint::File { path, .. } => (None, Some(path.display().to_string())),
    };
    ErrArgs {
        name: p.name.clone(),
        host,
        port,
        user,
        database: p.database.clone(),
        file,
        engine: server
            .map_or(p.engine, |s| s.kind)
            .display_name()
            .to_string(),
        version: server.map(short_version),
        hop: None,
        hop_host: None,
    }
}

/// Stores the password a successful Main connect used when it came from the prompt or the
/// dialog's draft, with the profile's retention (R8). Never for a Test or a probe, never for
/// `.pgpass` or a stored password, and never for a cancelled connect: `cancel` is checked
/// right before the write, and a secret written while a cancel raced it is removed again
/// (M2a final review M2). A Delete cancels the connect before it queues its own `delete_all`,
/// so either the put lands first and the Delete removes it, or the put sees the cancel here
/// and removes it itself: no `Forever` entry outlives its profile. On the blocking pool.
async fn remember_password(
    p: &Profile,
    purpose: Purpose,
    secrets: &Arc<SecretStore>,
    auth: Option<&hacivat_secrets::AuthMaterial>,
    cancel: &CancellationToken,
) {
    let Some(auth) = auth else { return };
    let worth_storing = purpose == Purpose::Main
        && p.auth == AuthProviderId::Password
        && p.retention != Retention::Never
        && matches!(auth.source, SecretSource::Prompt | SecretSource::Draft);
    let Some(password) = auth.password.clone().filter(|_| worth_storing) else {
        return;
    };
    let key = SecretKey {
        profile: p.id,
        slot: SecretSlot::DbPassword,
    };
    let retention = p.retention;
    if cancel.is_cancelled() {
        return;
    }
    let store = secrets.clone();
    let stored = tokio::task::spawn_blocking(move || store.put(&key, password, retention)).await;
    if !matches!(stored, Ok(Ok(_))) {
        tracing::warn!("the password that worked could not be stored");
    }
    if cancel.is_cancelled() {
        // The connect was cancelled (a disconnect, a Delete) while the write ran: undo it.
        let store = secrets.clone();
        let removed = tokio::task::spawn_blocking(move || store.remove(&key, retention)).await;
        if !matches!(removed, Ok(Ok(()))) {
            tracing::warn!("a password stored by a cancelled connect could not be removed");
        }
    }
}

/// An empty trail starting now.
pub(crate) fn fresh_progress() -> Progress {
    Progress {
        trail: Vec::new(),
        current: None,
        started: Instant::now(),
    }
}
