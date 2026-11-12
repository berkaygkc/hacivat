//! Test connection: the pipeline with `Purpose::Test`, a ping median and warnings; nothing is
//! stored (HV-CONN-07, R8). Body in Task 16.

use std::time::Duration;

use hacivat_driver::{ProbeWarning, ServerInfo, Support};
use hacivat_net::TlsSummary;
use hacivat_secrets::AuthProviderId;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::pipeline::{
    ConnectDeps, ConnectError, Interaction, Progress, StepTrace, not_implemented,
};
use crate::profile::ProfileDraft;

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

/// Tests `draft` without storing anything (Task 16). The stub fails with `internal`.
pub async fn test_connection(
    _deps: &ConnectDeps,
    draft: ProfileDraft,
    _ix: Interaction,
    _cancel: CancellationToken,
    _progress: watch::Sender<Progress>,
) -> TestReport {
    TestReport {
        ok: false,
        trail: Vec::new(),
        total: Duration::ZERO,
        server: None,
        support: None,
        tls: None,
        auth: draft.profile.auth,
        user: draft.profile.user.clone(),
        ping_median: None,
        warnings: Vec::new(),
        error: Some(not_implemented(&draft.profile)),
    }
}
