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
use hacivat_proto::{AccessLabel, DbmsKind, Department, ErrClass, ErrFamily, FixId, ProfileId};
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
pub use validate::{
    ApprovedRoots, IssueCode, ValidationIssue, validate, validate_with, validate_with_passphrase,
};

pub(crate) use steps::{CLOSE_BUDGET, error, short_version};
use steps::{Deadline, MAX_REJECTIONS, StepState, builtin_steps, close_bounded};
// ## m2b-t14-pipeline
use ssh_step::SshState;
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

// ## m2b-t14-pipeline
/// Whether the drivers may send a password by an insecure method (PostgreSQL cleartext or MD5,
/// MySQL's `caching_sha2_password` full authentication) for `profile` over an address of `class`
/// with TLS `mode` (B8, Q20, Q23). `Allow` when any of these holds:
/// - the address is loopback or a Unix socket (the password never crosses a network);
/// - the mode is `Disable` (the user's explicit choice of plaintext);
/// - under `Auto`, the user consented to plaintext toward a public host and remembered it
///   (`tls.allow_public_plaintext`, B7). The one-gesture consent is the pipeline's (it sends
///   `Allow` for the retry it answers).
///
/// Otherwise `RefuseUnlessVerified`: the drivers refuse unless the TLS outcome is `Full`,
/// `CaOnly` or `Pinned` (outcome-based for both engines, Q20). Through SSH, `class` is
/// `ssh_address_class` of the database host (B11: the guard still applies).
pub fn insecure_auth_for(profile: &Profile, class: AddressClass, mode: TlsMode) -> InsecureAuth {
    let local = matches!(class, AddressClass::Loopback | AddressClass::UnixSocket);
    let consented = mode == TlsMode::Auto && profile.tls.allow_public_plaintext;
    if local || mode == TlsMode::Disable || consented {
        InsecureAuth::Allow
    } else {
        InsecureAuth::RefuseUnlessVerified
    }
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
///
/// M2b (Task 14):
/// - **SSH** (order 40, `ssh_step`): a `TransportSpec::SshTunnel` profile acquires its tunnel
///   outside the connect clock (prompt time never counts, Q44), never resolves the database
///   host locally, and shows `Ssh` then `Connect`. Prompted SSH secrets are stored only after
///   the whole connect succeeded, with `ssh_retention`.
/// - **TLS** (order 50): the database host is the TLS host, an encrypted client key is opened
///   with the overlay's, the stored or a typed passphrase (B25), and the profile's pins apply.
/// - **Questions** (order 70, `prompts::ask_and_retry`, at most one retry per kind): the
///   certificate review on an unknown issuer (pin, import CA), the public-plaintext question
///   under `Auto` (never for a pinned endpoint, Q13), and the errors' certificate for
///   `tls.pin_changed` and `auth.insecure_method` (Q20). Headless never asks: it fails with
///   `needs_interaction` where it would have.
/// - **Secrets**: the session overlay (`ConnectRequest.overlay_secrets`) comes before the store,
///   a locked vault is unlocked first (up to 3 master passwords, Q33), and a vault that does
///   not exist yet is offered before the first `Forever` secret is stored (Q38). What worked is
///   returned in `LiveConnection.secrets_used`, the plaintext consent in `plaintext_consent`,
///   and the profile changes in `profile_changes` (also on a later failure, in the error).
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
        department,
        prior_rejections,
        read_only,
        // ## m2b-t14-pipeline: the SSH drafts (Test, Q23) and the session overlay (Q17).
        draft_ssh,
        overlay_secrets,
        plaintext_consent,
    } = req;
    // A connect is read-only if the profile or the session is (never weaker than the profile).
    let read_only = read_only || profile.read_only;
    // The typed settings only: an extra option never overrides them (M2a final review I2).
    let port = effective_port(&profile, deps.catalog);
    let user = effective_user(&profile);

    // ## m2b-t14-pipeline: an encrypted TLS client key is not a problem here: the TLS step
    // opens it with the overlay's, the stored or a typed passphrase (T3 carry-in, B25).
    let (mut issues, options) = validate::checked(
        &profile,
        deps.catalog,
        &deps.approved_roots.list(),
        validate::KeyCheck::Deferred,
    );
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
    // ## m2b-t14-pipeline
    let tls_host = match &profile.endpoint {
        ProfileEndpoint::Server { host, .. } => host.trim().to_string(),
        _ => "localhost".to_string(),
    };
    if profile
        .tls
        .disable_ignores_pin(&tls_host, port.unwrap_or_default())
    {
        // T3 ruling M2: Disable is the user's choice and wins; the dialog shows the flag.
        tracing::warn!("TLS is disabled for an endpoint that has a pin: the pin is not checked");
    }
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
        // ## m2b-t18-guarded-exec (review m-2): a cancel or kill connection runs no user SQL
        // and shows no label, so it skips the role probe.
        probe_role: department != Department::Cancel,
        // ## m2b-t14-pipeline
        purpose,
        timeout,
        approved_roots: deps.approved_roots.clone(),
        tunnels: deps.tunnels.clone(),
        known_hosts: deps.known_hosts.clone(),
        overlay: overlay_secrets.unwrap_or_default(),
        draft_ssh,
        tls_host,
        tls_settings: profile.tls.clone(),
        tls_passphrase: None,
        tls_passphrase_typed: false,
        attempt_tls: None,
        consent: plaintext_consent,
        plaintext_retry: false,
        changes: Vec::new(),
        retries: Default::default(),
        vault_tried: false,
        ssh: SshState::direct(),
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
    // ## m2b-t14-pipeline: every secret a successful Main connect was given by the user is
    // stored now, never before (R8), with the vault created first when needed (Q38). Fix
    // round 1 (review M2): a cancel during that (the create-vault question, a slow keychain)
    // undoes what was stored and fails the connect like any other cancel.
    let todo = typed_secrets(&profile, &cx.state);
    let (ix, store) = (cx.state.ix.clone(), cx.state.secrets.clone());
    let stored = remember_secrets(profile.id, ix, store.clone(), todo, &cancel).await;
    if cancel.is_cancelled() {
        // Re-review N1: a cancelled connect stores nothing (R8), so every key this connect
        // wrote goes again, decided at the same point as the cancelled result.
        forget_stored(&store, stored).await;
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
    let StepCx { state: mut st, .. } = cx;
    let secrets_used = SessionSecrets {
        db: st
            .auth
            .as_ref()
            .filter(|_| profile.auth == AuthProviderId::Password)
            .and_then(|a| a.password.clone()),
        hops: std::mem::take(&mut st.ssh.hops),
        tls_key_passphrase: st.tls_passphrase.take(),
    };
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
        profile_changes: st.changes,
        transport: st.ssh.transport,
        path: st.ssh.path,
        via: st.ssh.via,
        access_warnings: st.access_warnings,
        secrets_used,
        plaintext_consent: st.consent,
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
        changes,
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
    // ## m2b-t14-pipeline: the SSH step names the hop; the changes made before the failure
    // still apply (a pin, a CA file, the plaintext consent).
    let (hop, hop_host) = (e.args.hop, e.args.hop_host.take());
    e.args = args(&profile, port, user, server.as_ref());
    e.args.hop = hop;
    e.args.hop_host = hop_host;
    let mut changes = changes;
    changes.append(&mut e.profile_changes);
    e.profile_changes = changes;
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
    // ## m2b-t14-pipeline: through SSH, a transport failure lands where it happened: the
    // tunnel (`Ssh`) or its first channel (`Connect`).
    if e.phase == Phase::Transport && steps::is_ssh(p) {
        return None;
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
        ErrFamily::Network | ErrFamily::Karagoz => TrailStep::Connect,
        // ## m2b-t14-pipeline
        ErrFamily::Ssh => TrailStep::Ssh,
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
            // ## m2b-t14-pipeline: through SSH the tunnel replaces the local lookup (B11).
            if steps::is_ssh(p) {
                steps.extend([TrailStep::Ssh, TrailStep::Connect]);
            } else {
                steps.extend([TrailStep::Resolve, TrailStep::Connect]);
            }
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

/// Stores what a successful Main connect was given by the user, with the profile's retentions
/// (R8): the password from the prompt or the dialog's draft (`retention`), the SSH passwords
/// and passphrases typed at a hop prompt (`ssh_retention`, by hop), and a typed TLS key
/// passphrase (`retention`). Never for a Test or a probe, never for `.pgpass`, a stored, the
/// overlay's or a draft SSH secret, never a keyboard-interactive answer, and never for a
/// cancelled connect: `cancel` is checked right before each write, and when the connect turns
/// out cancelled the caller removes every key this call wrote (it returns them; M2a final
/// review M2, T14 re-review N1). A Delete cancels the connect before it queues its own
/// `delete_all`, so either the puts land first and the Delete removes them, or the cancel is
/// seen here and they are removed again: no `Forever` entry outlives its profile. Before the
/// first `Forever` secret, a vault that does not exist yet is offered (Q38). On the blocking
/// pool.
async fn remember_secrets(
    profile: ProfileId,
    ix: Interaction,
    secrets: Arc<SecretStore>,
    todo: Vec<(SecretSlot, SecretString, Retention)>,
    cancel: &CancellationToken,
) -> Vec<(SecretKey, Retention)> {
    let mut written = Vec::new();
    if todo.is_empty() || cancel.is_cancelled() {
        return written;
    }
    if todo.iter().any(|(_, _, r)| *r == Retention::Forever) {
        prompts::ensure_vault(&ix, &secrets, cancel).await;
    }
    for (slot, secret, retention) in todo {
        if cancel.is_cancelled() {
            break;
        }
        let key = SecretKey { profile, slot };
        written.push((key, retention));
        let store = secrets.clone();
        let stored = tokio::task::spawn_blocking(move || store.put(&key, secret, retention)).await;
        if !matches!(stored, Ok(Ok(_))) {
            tracing::warn!("a secret that worked could not be stored");
        }
    }
    written
}

/// Removes every key a cancelled connect wrote, wherever its retention keeps it (the backend
/// and its `UntilRestart` fallback for `Forever`). On the blocking pool.
async fn forget_stored(secrets: &Arc<SecretStore>, written: Vec<(SecretKey, Retention)>) {
    if written.is_empty() {
        return;
    }
    let store = secrets.clone();
    let removed = tokio::task::spawn_blocking(move || {
        written
            .iter()
            .all(|(key, retention)| store.remove(key, *retention).is_ok())
    })
    .await;
    if !matches!(removed, Ok(true)) {
        tracing::warn!("a secret stored by a cancelled connect could not be removed");
    }
}

/// What [`remember_secrets`] stores: the user's secrets of a Main connect, by slot.
fn typed_secrets(p: &Profile, st: &StepState) -> Vec<(SecretSlot, SecretString, Retention)> {
    if st.purpose != Purpose::Main {
        return Vec::new();
    }
    let mut todo: Vec<(SecretSlot, SecretString, Retention)> = Vec::new();
    if let Some(auth) = st.auth.as_ref()
        && p.auth == AuthProviderId::Password
        && matches!(auth.source, SecretSource::Prompt | SecretSource::Draft)
        && let Some(password) = auth.password.clone()
    {
        todo.push((SecretSlot::DbPassword, password, p.retention));
    }
    for (hop, slot) in &st.ssh.typed {
        let used = st.ssh.hops.get(usize::from(*hop));
        let (slot, secret) = match slot {
            prompts::SshSlot::Password => (
                SecretSlot::SshPassword { hop: *hop },
                used.and_then(|h| h.password.clone()),
            ),
            prompts::SshSlot::Passphrase => (
                SecretSlot::SshPassphrase { hop: *hop },
                used.and_then(|h| h.passphrase.clone()),
            ),
        };
        // `MAX_SSH_HOP` bounds the slots `delete_all` can reach (T1 fix round 1, M3).
        if let Some(secret) = secret
            && *hop <= hacivat_secrets::MAX_SSH_HOP
        {
            todo.push((slot, secret, p.ssh_retention));
        }
    }
    if st.tls_passphrase_typed
        && let Some(passphrase) = st.tls_passphrase.clone()
    {
        todo.push((SecretSlot::TlsKeyPassphrase, passphrase, p.retention));
    }
    todo.retain(|(_, _, retention)| *retention != Retention::Never);
    todo
}

/// An empty trail starting now.
pub(crate) fn fresh_progress() -> Progress {
    Progress {
        trail: Vec::new(),
        current: None,
        started: Instant::now(),
    }
}
