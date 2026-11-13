//! Shared fixtures of the engine conformance suite (Task 19): the real drivers
//! (`default_drivers()`), profiles for the Docker targets, and the target rules.
//!
//! - Every Docker test is `#[ignore = "docker"]` and returns early unless `HACIVAT_DOCKER=1`
//!   ([`docker_enabled`]), because `cargo xtask docker test` runs every ignored test.
//! - A target the active set should contain is required (T13's `target_required`): under
//!   `--set full`, or with no set (every target), a missing one fails; a narrower set (`pr`)
//!   skips it.
//! - Failure messages carry the target id and the error class only, never a password.

#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]
#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hacivat_driver::EngineCatalog;
use hacivat_net::{StreamFactory, TlsMode};
use hacivat_proto::{DbmsKind, Department};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
    SecretString,
};
use hacivat_session::default_drivers;
use hacivat_session::pipeline::{
    ApprovedRoots, ConnectDeps, ConnectError, ConnectRequest, Interaction, LiveConnection,
    Progress, Purpose, StepTrace, TrailStep, TransportWrap, connect,
};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint, suggest_name};
use hacivat_session::{TestReport, test_connection};
use hacivat_testkit::{DockerTarget, TlsDefault, all_targets_unfiltered, target, targets};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub use hacivat_testkit::docker_enabled;

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

// ---------------------------------------------------------------------------------------------
// Targets

/// Whether every named target must exist in the active set: under `full` (or with no set,
/// which means every target) a missing one is a failure, never a silent skip.
pub fn must_exist(set: Option<&str>) -> bool {
    set.is_none_or(|s| s == "full")
}

fn active_set() -> Option<String> {
    std::env::var("HACIVAT_DOCKER_SET").ok()
}

/// The target `id`, or `None` when the active set leaves it out. Panics when the set should
/// contain it ([`must_exist`]).
pub fn target_required(id: &str) -> Option<DockerTarget> {
    let t = target(id);
    let set = active_set();
    assert!(
        t.is_some() || !must_exist(set.as_deref()),
        "Docker target {id} is missing from the {} set",
        set.as_deref().unwrap_or("full")
    );
    t
}

/// The targets of the active set that connect without an extra step: every one but pg13
/// (below the floor) and pg17-tr (the Turkish-locale twin of pg17). Under `full` the whole
/// supported matrix must be there.
pub fn matrix() -> Vec<DockerTarget> {
    let found: Vec<DockerTarget> = targets(None)
        .into_iter()
        .filter(|t| !matches!(t.id.as_str(), "pg13" | "pg17-tr"))
        .collect();
    if must_exist(active_set().as_deref()) {
        for id in [
            "pg14",
            "pg15",
            "pg16",
            "pg17",
            "pg18",
            "pg17-tls",
            "mysql80",
            "mysql84",
            "mariadb106",
            "mariadb1011",
            "mariadb114",
        ] {
            assert!(
                found.iter().any(|t| t.id == id),
                "Docker target {id} is missing from the full set"
            );
        }
    }
    assert!(!found.is_empty(), "the active set has no target");
    found
}

/// `Require` where the server offers a self-signed certificate (which Auto would refuse to
/// trust), `Auto` elsewhere, so TLS never masks the class under test.
pub fn auto_or_require(t: &DockerTarget) -> TlsMode {
    match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    }
}

/// The throwaway password every Docker target shares (from `hacivat-testkit`).
pub fn docker_password() -> SecretString {
    let t = all_targets_unfiltered()
        .into_iter()
        .next()
        .expect("targets.toml lists a target");
    SecretString::from(t.password)
}

// ---------------------------------------------------------------------------------------------
// Deps: the real drivers, a memory secret store, no credential files.

pub fn deps() -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    ConnectDeps {
        drivers: Arc::new(default_drivers()),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: None,
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
    }
}

/// [`deps`] with a transport wrap that counts the streams opened through it. With a wrap the
/// endpoint has no direct target (P13), so every driver must consume the factory.
pub fn deps_counting() -> (ConnectDeps, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let opened = count.clone();
    let wrap: TransportWrap = Arc::new(move |inner: StreamFactory| {
        let opened = opened.clone();
        StreamFactory::new(move || {
            opened.fetch_add(1, Ordering::SeqCst);
            inner.open()
        })
    });
    let mut deps = deps();
    deps.transport_wrap = Some(wrap);
    (deps, count)
}

// ---------------------------------------------------------------------------------------------
// Profiles

/// A profile for target `t` with TLS `mode`: user and database `hv_test` on 127.0.0.1, a
/// password sign-in that keeps nothing (`Never`); the password travels as the draft.
pub fn profile_for(t: &DockerTarget, mode: TlsMode) -> Profile {
    let mut p = Profile {
        name: format!("conformance {}", t.id),
        engine: t.engine,
        endpoint: ProfileEndpoint::Server {
            host: t.host.clone(),
            port: Some(t.port),
        },
        database: Some(t.database.clone()),
        user: Some(t.user.clone()),
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..Profile::default()
    };
    p.tls.mode = mode;
    p
}

/// Builder-style edits of a test profile (the brief's `|p| p.database("orders_v2")`).
pub trait ProfileExt: Sized {
    fn retention(self, r: Retention) -> Self;
    fn database(self, db: &str) -> Self;
    fn user(self, user: &str) -> Self;
    fn port(self, port: u16) -> Self;
    fn host(self, host: &str) -> Self;
    fn tls(self, mode: TlsMode) -> Self;
}

impl ProfileExt for Profile {
    fn retention(mut self, r: Retention) -> Self {
        self.retention = r;
        self
    }
    fn database(mut self, db: &str) -> Self {
        self.database = Some(db.to_string());
        self
    }
    fn user(mut self, user: &str) -> Self {
        self.user = Some(user.to_string());
        self
    }
    fn port(mut self, port: u16) -> Self {
        if let ProfileEndpoint::Server { port: p, .. } = &mut self.endpoint {
            *p = Some(port);
        }
        self
    }
    fn host(mut self, host: &str) -> Self {
        if let ProfileEndpoint::Server { host: h, .. } = &mut self.endpoint {
            *h = host.to_string();
        }
        self
    }
    fn tls(mut self, mode: TlsMode) -> Self {
        self.tls.mode = mode;
        self
    }
}

// ---------------------------------------------------------------------------------------------
// Requests

pub fn token() -> CancellationToken {
    CancellationToken::new()
}

pub fn progress() -> watch::Sender<Progress> {
    watch::Sender::new(Progress {
        trail: Vec::new(),
        current: None,
        started: Instant::now(),
    })
}

fn main_request(p: Profile, password: Option<SecretString>) -> ConnectRequest {
    ConnectRequest {
        profile: Arc::new(p),
        draft_password: password,
        purpose: Purpose::Main,
        department: Department::Console,
        prior_rejections: 0,
    }
}

/// A Main connect of `p` with the Docker password as the draft.
pub fn req(p: Profile) -> ConnectRequest {
    main_request(p, Some(docker_password()))
}

/// A Main connect of `p` with `password` as the draft.
pub fn req_pw(p: Profile, password: &str) -> ConnectRequest {
    main_request(p, Some(SecretString::from(password.to_string())))
}

/// A Main connect of a draft (the dialog's Connect): its password travels as the draft, and an
/// unnamed draft gets the name the dialog would suggest.
pub fn req_draft(d: ProfileDraft) -> ConnectRequest {
    let ProfileDraft {
        mut profile,
        password,
        ..
    } = d;
    if profile.name.is_empty() {
        profile.name = suggest_name(&profile);
    }
    main_request(profile, password)
}

/// The trail's steps, in order.
pub fn steps(trail: &[StepTrace]) -> Vec<TrailStep> {
    trail.iter().map(|t| t.step).collect()
}

/// Connects `t` with TLS `mode`; it must succeed.
pub async fn connect_target(t: &DockerTarget, mode: TlsMode) -> LiveConnection {
    connect(
        &deps(),
        req(profile_for(t, mode)),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("{}: {:?}", t.id, e.class))
}

/// The error of connecting target `id` (its `auto_or_require` mode) after `edit`, with
/// `password` (the Docker password when `None`); `None` when the active set leaves `id` out.
pub async fn err_of_with(
    id: &str,
    edit: impl FnOnce(Profile) -> Profile,
    password: Option<&str>,
) -> Option<ConnectError> {
    let t = target_required(id)?;
    let p = edit(profile_for(&t, auto_or_require(&t)));
    let request = match password {
        Some(pw) => req_pw(p, pw),
        None => req(p),
    };
    let result = connect(&deps(), request, Interaction::Headless, token(), progress()).await;
    match result {
        Ok(live) => {
            live.close().await;
            panic!("{id}: the connect succeeded, an error was expected");
        }
        Err(e) => Some(e),
    }
}

/// A test connection of target `t` (its `auto_or_require` mode) with the Docker password.
pub async fn test_target(t: &DockerTarget) -> TestReport {
    let draft = ProfileDraft {
        password: Some(docker_password()),
        ..ProfileDraft::edit(profile_for(t, auto_or_require(t)))
    };
    test_connection(&deps(), draft, Interaction::Headless, token(), progress()).await
}

/// A SQLite profile for `path`, created when missing.
pub fn sqlite_profile(path: &std::path::Path) -> Profile {
    Profile {
        name: "conformance sqlite".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: true,
        },
        user: None,
        auth: AuthProviderId::None,
        max_connections: 1,
        ..Profile::default()
    }
}
