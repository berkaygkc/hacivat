//! The connect pipeline (HV-CONN-01): validate, credential files, resolve, transport, TLS,
//! sign-in, driver connect and post-connect checks, with a step trail, cancellation, cleanup
//! guards and classified errors. Types are complete since Task 1; logic in Task 16.

mod interaction;
mod steps;
mod trail;
mod validate;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::{DriverRegistry, EngineCatalog, RawConn, ServerInfo, ServerMessage, Support};
use hacivat_net::{AddressClass, CleanupGuard, StreamFactory, TlsSummary};
use hacivat_proto::{AccessLabel, Department, ErrClass, FixId};
use hacivat_secrets::{AuthProviderId, CredentialFiles, SecretStore};
use secrecy::SecretString;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub use hacivat_driver::errors::Phase;
pub use interaction::{Answer, Interaction, Question, UiPort};
pub use steps::{ConnectStep, StepCx};
pub use trail::{Progress, StepOutcome, StepTrace, TrailStep};
pub use validate::{IssueCode, ValidationIssue, validate};

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
}

/// One connect (HV-CONN-01). `department` is `Console` for Main, `Unknown` for Test and
/// `Introspection` for Probe (§5.8, P44). Holds a secret, so `Debug` only.
#[derive(Debug)]
pub struct ConnectRequest {
    pub profile: Arc<Profile>,
    pub draft_password: Option<SecretString>,
    pub purpose: Purpose,
    pub department: Department,
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
    conn: Box<dyn RawConn>,
    #[allow(dead_code, reason = "Task 16")]
    guards: Vec<CleanupGuard>,
    forwarded: bool,
}

impl LiveConnection {
    pub fn raw(&mut self) -> &mut dyn RawConn {
        &mut *self.conn
    }

    /// Closes the connection, then runs its cleanup guards (Task 16).
    pub fn close(self) -> BoxFuture<'static, ()> {
        self.conn.close()
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
    /// Where forwarder sockets are created (`0700`).
    pub private_dir: PathBuf,
    pub transport_wrap: Option<TransportWrap>,
    /// Extra steps merged into the ordered list by `order` (the cleanup-guard test seam).
    pub extra_steps: Vec<Arc<dyn ConnectStep>>,
    /// Replaces the profile's connect timeout; `None` in the app, set only by tests.
    #[doc(hidden)]
    pub timeout_override: Option<Duration>,
}

/// Runs the pipeline for `req` (Task 16). The stub fails with `internal`.
pub async fn connect(
    _deps: &ConnectDeps,
    req: ConnectRequest,
    _ix: Interaction,
    _cancel: CancellationToken,
    _progress: watch::Sender<Progress>,
) -> Result<LiveConnection, ConnectError> {
    Err(not_implemented(&req.profile))
}

/// The stub's error for `p` (Task 16 removes it).
pub(crate) fn not_implemented(p: &Profile) -> ConnectError {
    let (host, port, file) = match &p.endpoint {
        ProfileEndpoint::Server { host, port } => (Some(host.clone()), *port, None),
        ProfileEndpoint::Socket { path, port } => (None, *port, Some(path.display().to_string())),
        ProfileEndpoint::File { path, .. } => (None, None, Some(path.display().to_string())),
    };
    ConnectError {
        class: ErrClass::Internal,
        phase: Phase::Validate,
        trail: Vec::new(),
        args: ErrArgs {
            name: p.name.clone(),
            host,
            port,
            user: p.user.clone(),
            database: p.database.clone(),
            file,
            engine: p.engine.display_name().to_string(),
            version: None,
        },
        server: None,
        fixes: Vec::new(),
        suggestions: Vec::new(),
        issues: Vec::new(),
        elapsed: Duration::ZERO,
        attempts: 0,
        needs_interaction: false,
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
