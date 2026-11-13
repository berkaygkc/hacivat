//! Test connection: the pipeline with `Purpose::Test`, a ping median and warnings; nothing is
//! stored (HV-CONN-07, R8).

use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::{ProbeWarning, ServerInfo, Support};
use hacivat_net::{TlsSummary, TlsVerified};
use hacivat_proto::Department;
use hacivat_secrets::AuthProviderId;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::pipeline::{
    CLOSE_BUDGET, ConnectDeps, ConnectError, ConnectRequest, Interaction, Progress, Purpose,
    SessionSecrets, StepTrace, connect, short_version,
};
use crate::profile::{ProfileDraft, ProfileEndpoint};

/// A warning a successful test still reports (HV-CONN-07).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TestWarning {
    NoTls,
    TlsNotVerified,
    NotTested { version: String },
    UnsupportedAccepted { version: String },
    Probe(ProbeWarning),
}

/// What a test found (HV-CONN-07).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TestReport {
    pub ok: bool,
    pub trail: Vec<StepTrace>,
    pub total: Duration,
    pub server: Option<ServerInfo>,
    pub support: Option<Support>,
    pub tls: Option<TlsSummary>,
    pub auth: AuthProviderId,
    pub user: Option<String>,
    pub ping_median: Option<Duration>,
    pub warnings: Vec<TestWarning>,
    pub error: Option<ConnectError>,
}

/// How many pings the median is taken over (HV-CONN-07).
const PINGS: usize = 3;

/// How long one ping or the probe may take.
const ROUND_TRIP_BUDGET: Duration = Duration::from_secs(5);

/// Tests `draft` without storing anything (HV-CONN-07): the full pipeline with
/// `Purpose::Test` and fresh resources (no budget permit, nothing shared with a live
/// connection), then the median of 3 pings, the engine's probe warnings plus `NoTls` and
/// `TlsNotVerified`, and the version warnings. The connection is closed before it returns.
/// The draft's password is used, never stored (R8).
pub async fn test_connection(
    deps: &ConnectDeps,
    draft: ProfileDraft,
    ix: Interaction,
    cancel: CancellationToken,
    progress: watch::Sender<Progress>,
) -> TestReport {
    test_connection_continuing(deps, draft, 0, ix, cancel, progress).await
}

/// [`test_connection`] continuing a gesture that already had `prior_rejections` server
/// rejections (the connection dialog's password re-entry, P8): the pipeline's budget of 3
/// includes them, and a gesture that already has 3 fails at once, before any I/O.
pub async fn test_connection_continuing(
    deps: &ConnectDeps,
    draft: ProfileDraft,
    prior_rejections: u8,
    ix: Interaction,
    cancel: CancellationToken,
    progress: watch::Sender<Progress>,
) -> TestReport {
    let started = tokio::time::Instant::now();
    let ProfileDraft {
        profile,
        password,
        ssh_secrets,
        tls_key_passphrase,
        ..
    } = draft;
    let auth = profile.auth;
    let profile_user = profile.user.clone();
    let network = !matches!(profile.endpoint, ProfileEndpoint::File { .. });
    let req = ConnectRequest {
        read_only: profile.read_only,
        profile: Arc::new(profile),
        draft_password: password,
        purpose: Purpose::Test,
        department: Department::Unknown,
        prior_rejections,
        // ## m2b-t01-interfaces: the dialog's typed SSH secrets, for Test (Q23).
        draft_ssh: ssh_secrets,
        // ## m2b-t14-pipeline: the dialog's TLS key passphrase, used (never stored) like the
        // draft password. A Test never shares an SSH tunnel: the pipeline acquires with
        // `share = false` for `Purpose::Test` (CONN-07), and the tunnel closes with the test.
        overlay_secrets: tls_key_passphrase.map(|passphrase| SessionSecrets {
            tls_key_passphrase: Some(passphrase),
            ..SessionSecrets::default()
        }),
        plaintext_consent: false,
    };
    let mut live = match connect(deps, req, ix, cancel.clone(), progress).await {
        Ok(live) => live,
        Err(e) => {
            return TestReport {
                ok: false,
                trail: e.trail.clone(),
                total: started.elapsed(),
                server: None,
                support: None,
                tls: None,
                auth,
                user: e.args.user.clone().or(profile_user),
                ping_median: None,
                warnings: Vec::new(),
                error: Some(e),
            };
        }
    };

    let mut pings = Vec::with_capacity(PINGS);
    for _ in 0..PINGS {
        if cancel.is_cancelled() {
            break;
        }
        if let Ok(Ok(d)) = tokio::time::timeout(ROUND_TRIP_BUDGET, live.raw().ping()).await {
            pings.push(d);
        }
    }
    pings.sort();
    let ping_median = pings.get(pings.len() / 2).copied();

    let mut warnings = Vec::new();
    if network && !live.tls.encrypted {
        warnings.push(TestWarning::NoTls);
    } else if live.tls.encrypted && live.tls.verified == TlsVerified::No {
        warnings.push(TestWarning::TlsNotVerified);
    }
    match &live.support {
        Support::Supported => {}
        Support::NotTested => warnings.push(TestWarning::NotTested {
            version: short_version(&live.server),
        }),
        Support::Unsupported { .. } => warnings.push(TestWarning::UnsupportedAccepted {
            version: short_version(&live.server),
        }),
    }
    if !cancel.is_cancelled()
        && let Ok(found) = tokio::time::timeout(ROUND_TRIP_BUDGET, live.raw().probe()).await
    {
        warnings.extend(found.into_iter().map(TestWarning::Probe));
    }

    let report = TestReport {
        ok: true,
        trail: live.trail.clone(),
        total: started.elapsed(),
        server: Some(live.server.clone()),
        support: Some(live.support.clone()),
        tls: Some(live.tls.clone()),
        auth,
        user: live.user.clone(),
        ping_median,
        warnings,
        error: None,
    };
    let _ = tokio::time::timeout(CLOSE_BUDGET, live.close()).await;
    report
}
