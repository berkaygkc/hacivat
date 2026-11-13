//! Shared fixtures of the pipeline and manager tests: `FakeDriver` deps, profiles, a scripted
//! `UiPort` and a per-thread secret store (HV-CONN-01, P23).
//!
//! Hygiene (P46, P23): every host is an IP literal (`127.0.0.1` local, `203.0.113.10` public,
//! TEST-NET-3), and every deps sets `transport_wrap` to a stub factory over
//! `tokio::io::duplex`, so no test sends a DNS query or opens a socket.

#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]
#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{
    DriverRegistry, EngineCatalog, RawError, RawErrorKind, SemVerLoose, ServerInfo, ServerMessage,
};
use hacivat_net::{
    BoxedStream, CleanupGuard, StreamFactory, TlsFailure, TlsMode, TlsSummary, TlsVerified,
};
use hacivat_proto::{DbmsKind, Department, ProfileId};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretError,
    SecretKey, SecretSlot, SecretStore, SecretString,
};
use hacivat_session::pipeline::{
    Answer, ConnectDeps, ConnectError, ConnectRequest, ConnectStep, Interaction, LiveConnection,
    Phase, Progress, Purpose, Question, StepCx, StepOutcome, TrailStep, UiPort, connect,
};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint, TransportSpec};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, TestReport, test_connection};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// How long a wait for core work may take before the test fails: generous, since CI runners
/// can be much slower than a laptop; a passing wait ends as soon as its condition holds.
pub const WAIT: Duration = Duration::from_secs(10);

/// Polls `done` every 5 ms (letting the runtime's tasks run) until it holds; panics naming
/// `what` after [`WAIT`] of real time. Not for `start_paused` tests.
pub async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out after {WAIT:?} waiting for {what}"
        );
        tokio::time::sleep(ms(5)).await;
    }
}

/// [`eventually`] for a plain thread (a test that owns its `CoreRuntime`).
#[track_caller]
pub fn eventually_blocking(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out after {WAIT:?} waiting for {what}"
        );
        std::thread::sleep(ms(5));
    }
}

// ---------------------------------------------------------------------------------------------
// Secrets: one store per test thread, so a profile's stored password and the deps agree.

thread_local! {
    static BACKEND: RefCell<Option<Arc<MemoryBackend>>> = const { RefCell::new(None) };
    static STORE: RefCell<Option<Arc<SecretStore>>> = const { RefCell::new(None) };
}

fn backend_of_thread() -> Arc<MemoryBackend> {
    BACKEND.with(|b| {
        b.borrow_mut()
            .get_or_insert_with(|| Arc::new(MemoryBackend::new()))
            .clone()
    })
}

/// This test thread's secret store, over its memory backend.
pub fn thread_store() -> Arc<SecretStore> {
    STORE.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| {
                let backend: Arc<dyn SecretBackend> = backend_of_thread();
                Arc::new(SecretStore::new(Some(backend)))
            })
            .clone()
    })
}

/// The memory backend behind `deps`' store (the test thread's).
pub fn backend(_deps: &ConnectDeps) -> Arc<MemoryBackend> {
    backend_of_thread()
}

/// How many backend calls this test thread made.
pub fn backend_calls() -> u64 {
    backend_of_thread().calls()
}

/// The password `deps` keeps for the test profile, `Forever`.
pub fn stored(deps: &ConnectDeps) -> String {
    use hacivat_secrets::ExposeSecret;
    deps.secrets
        .get(&db_key(pid()), Retention::Forever)
        .unwrap()
        .map(|s| s.expose_secret().to_string())
        .unwrap_or_default()
}

pub fn db_key(id: ProfileId) -> SecretKey {
    SecretKey {
        profile: id,
        slot: SecretSlot::DbPassword,
    }
}

/// A backend whose `get` blocks for `delay`, then returns `password` (a slow keychain).
struct SlowBackend {
    delay: Duration,
    password: String,
}

impl SecretBackend for SlowBackend {
    fn get(&self, _key: &SecretKey) -> Result<Option<SecretString>, SecretError> {
        std::thread::sleep(self.delay);
        Ok(Some(SecretString::from(self.password.clone())))
    }
    fn put(&self, _key: &SecretKey, _value: &SecretString) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _key: &SecretKey) -> Result<(), SecretError> {
        Ok(())
    }
    fn name(&self) -> &'static str {
        "slow"
    }
}

// ---------------------------------------------------------------------------------------------
// Credential files: one `.pgpass` for every test (0600, a line for the test user).

fn pgpass_file() -> PathBuf {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pgpass");
        std::fs::write(&path, "127.0.0.1:5432:*:deploy:from-pgpass\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        dir
    });
    dir.path().join("pgpass")
}

// ---------------------------------------------------------------------------------------------
// Deps

/// A factory over `tokio::io::duplex`: no socket is ever opened.
fn stub_wrap() -> hacivat_session::pipeline::TransportWrap {
    Arc::new(|_direct: StreamFactory| {
        StreamFactory::new(|| {
            Box::pin(async {
                let (near, _far) = tokio::io::duplex(64);
                Ok(Box::new(near) as BoxedStream)
            })
        })
    })
}

fn deps_over(driver: Arc<FakeDriver>, secrets: Arc<SecretStore>) -> ConnectDeps {
    let mut drivers = DriverRegistry::new();
    drivers.register(driver);
    let (tunnels, known_hosts) = ssh_stubs();
    ConnectDeps {
        drivers: Arc::new(drivers),
        catalog: EngineCatalog::embedded(),
        secrets,
        files: CredentialFiles {
            pgpass: Some(pgpass_file()),
            pg_service: None,
        },
        transport_wrap: Some(stub_wrap()),
        extra_steps: Vec::new(),
        approved_roots: hacivat_session::pipeline::ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

/// Deps over `fake`, which the caller keeps to read its counters.
pub fn deps_with(fake: FakeDriver) -> (ConnectDeps, Arc<FakeDriver>) {
    let fake = Arc::new(fake);
    (deps_over(fake.clone(), thread_store()), fake)
}

pub fn deps_with_fake(fake: FakeDriver) -> ConnectDeps {
    deps_with(fake).0
}

/// `FakeDriver` presented as a driver that cannot take a stream (MySQL's descriptor shape):
/// it dials a direct TCP target itself and reaches anything else through its forwarder.
struct DialsItself {
    fake: Arc<FakeDriver>,
    descriptor: hacivat_driver::DriverDescriptor,
}

impl hacivat_driver::Driver for DialsItself {
    fn descriptor(&self) -> &hacivat_driver::DriverDescriptor {
        &self.descriptor
    }
    fn connect(
        &self,
        req: hacivat_driver::DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn hacivat_driver::RawConn>, RawError>> {
        self.fake.connect(req)
    }
}

/// Deps over a driver without stream injection; `wrapped` sets the stub transport (so the
/// endpoint has no direct target, P13), else the endpoint is a direct TCP target that the
/// driver would dial (the fake never does).
pub fn deps_dialing_itself(fake: FakeDriver, wrapped: bool) -> (ConnectDeps, Arc<FakeDriver>) {
    let fake = Arc::new(fake);
    let mut descriptor = hacivat_driver::Driver::descriptor(fake.as_ref()).clone();
    descriptor.caps.stream_injection = false;
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(DialsItself {
        fake: fake.clone(),
        descriptor,
    }));
    let mut deps = deps_over(fake.clone(), thread_store());
    deps.drivers = Arc::new(drivers);
    if !wrapped {
        deps.transport_wrap = None;
    }
    (deps, fake)
}

fn deps_shared(fake: &Arc<FakeDriver>) -> ConnectDeps {
    deps_over(fake.clone(), thread_store())
}

/// Deps whose secret backend takes `delay` to answer every read with `password`.
pub fn deps_with_slow_backend(delay: Duration, password: &str) -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(SlowBackend {
        delay,
        password: password.into(),
    });
    deps_over(Arc::new(ok_pg()), Arc::new(SecretStore::new(Some(backend))))
}

// ## m2a-finalfix
/// Deps over `fake` whose secrets live in `secrets` (a test's own backend).
pub fn deps_with_store(fake: FakeDriver, secrets: Arc<SecretStore>) -> ConnectDeps {
    deps_over(Arc::new(fake), secrets)
}

/// An extra step at `order` whose guard records `name` into `sink` (P23).
struct GuardStep {
    order: i32,
    name: &'static str,
    sink: Arc<Mutex<Vec<&'static str>>>,
}

impl ConnectStep for GuardStep {
    fn phase(&self) -> Phase {
        Phase::Transport
    }
    fn order(&self) -> i32 {
        self.order
    }
    fn applies(&self, _p: &Profile) -> bool {
        true
    }
    fn apply<'a>(
        &'a self,
        _cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        let (name, sink) = (self.name, self.sink.clone());
        Box::pin(async move {
            Ok(Some(CleanupGuard::new(move || {
                Box::pin(async move { sink.lock().unwrap().push(name) })
            })))
        })
    }
}

/// Deps with two extra steps at orders 41 and 42 whose guards push `guard-a` and `guard-b`.
pub fn deps_with_guards(
    order: Arc<Mutex<Vec<&'static str>>>,
    fake: FakeDriver,
) -> (ConnectDeps, Arc<FakeDriver>) {
    let (mut deps, fake) = deps_with(fake);
    deps.extra_steps = vec![
        Arc::new(GuardStep {
            order: 41,
            name: "guard-a",
            sink: order.clone(),
        }),
        Arc::new(GuardStep {
            order: 42,
            name: "guard-b",
            sink: order,
        }),
    ];
    (deps, fake)
}

// ---------------------------------------------------------------------------------------------
// Profiles

/// The test profiles' fixed ID.
pub fn pid() -> ProfileId {
    "6f1c2a4e-1d16-4a16-8a16-000000000016".parse().unwrap()
}

fn base(engine: DbmsKind, host: &str) -> Profile {
    Profile {
        id: pid(),
        name: "orders".into(),
        engine,
        endpoint: ProfileEndpoint::Server {
            host: host.into(),
            port: None,
        },
        user: Some("deploy".into()),
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    }
}

fn with_mode(mut p: Profile, mode: TlsMode) -> Profile {
    p.tls.mode = mode;
    if mode == TlsMode::VerifyCa {
        p.tls.ca_file = Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/test-ca.pem"
        )));
    }
    p
}

pub fn pg_profile(mode: TlsMode) -> Profile {
    with_mode(base(DbmsKind::Postgres, "127.0.0.1"), mode)
}

pub fn mysql_profile(mode: TlsMode) -> Profile {
    with_mode(base(DbmsKind::MySql, "127.0.0.1"), mode)
}

pub fn mariadb_profile(mode: TlsMode) -> Profile {
    with_mode(base(DbmsKind::MariaDb, "127.0.0.1"), mode)
}

pub fn public_auto_profile() -> Profile {
    with_mode(base(DbmsKind::Postgres, "203.0.113.10"), TlsMode::Auto)
}

pub fn pgpass_profile() -> Profile {
    Profile {
        auth: AuthProviderId::PgPass,
        ..pg_profile(TlsMode::Auto)
    }
}

pub fn sqlite_profile() -> Profile {
    Profile {
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: PathBuf::from(":memory:"),
            create_if_missing: false,
        },
        user: None,
        ..base(DbmsKind::Sqlite, "")
    }
}

pub fn no_host() -> Profile {
    base(DbmsKind::Postgres, "")
}

pub fn ssh_profile() -> Profile {
    Profile {
        transport: TransportSpec::SshTunnel(Default::default()),
        ..pg_profile(TlsMode::Disable)
    }
}

pub fn pg_db(database: &str) -> Profile {
    Profile {
        database: Some(database.into()),
        ..pg_profile(TlsMode::Disable)
    }
}

pub fn pg_profile_timeout(seconds: u32) -> Profile {
    Profile {
        connect_timeout_s: seconds,
        ..pg_profile(TlsMode::Disable)
    }
}

/// A password profile that keeps nothing (`Never`).
pub fn never_profile() -> Profile {
    Profile {
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..pg_profile(TlsMode::Disable)
    }
}

pub fn never_profile_timeout(seconds: u32) -> Profile {
    Profile {
        connect_timeout_s: seconds,
        ..never_profile()
    }
}

/// A password profile that keeps its password `Forever`; nothing stored yet.
pub fn forever_profile() -> Profile {
    Profile {
        auth: AuthProviderId::Password,
        retention: Retention::Forever,
        ..pg_profile(TlsMode::Disable)
    }
}

/// [`forever_profile`] with `password` stored in this thread's store.
pub fn forever_profile_with(password: &str) -> Profile {
    thread_store()
        .put(
            &db_key(pid()),
            SecretString::from(password.to_string()),
            Retention::Forever,
        )
        .unwrap();
    forever_profile()
}

pub fn ro(p: Profile) -> Profile {
    Profile {
        read_only: true,
        ..p
    }
}

pub fn rw(p: Profile) -> Profile {
    Profile {
        read_only: false,
        ..p
    }
}

pub fn allow_unsupported(p: Profile) -> Profile {
    Profile {
        allow_unsupported_server: true,
        ..p
    }
}

pub fn draft(p: Profile, password: Option<&str>) -> ProfileDraft {
    ProfileDraft {
        password: password.map(|pw| SecretString::from(pw.to_string())),
        ..ProfileDraft::edit(p)
    }
}

// ---------------------------------------------------------------------------------------------
// Drivers and their answers

pub fn ok_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

pub fn pg_info(version: &str) -> ServerInfo {
    ServerInfo {
        kind: DbmsKind::Postgres,
        product: "PostgreSQL".into(),
        version: SemVerLoose::parse(version),
        raw_version: version.into(),
        is_replica: Some(false),
    }
}

pub fn plaintext() -> TlsSummary {
    TlsSummary {
        encrypted: false,
        version: None,
        cipher: None,
        verified: TlsVerified::No,
    }
}

fn raw(kind: RawErrorKind) -> RawError {
    RawError { kind, server: None }
}

pub fn dns() -> RawError {
    raw(RawErrorKind::Dns)
}

pub fn refused() -> RawError {
    raw(RawErrorKind::Io(io::ErrorKind::ConnectionRefused))
}

pub fn io_reset() -> RawError {
    raw(RawErrorKind::Io(io::ErrorKind::ConnectionReset))
}

pub fn tls_unknown() -> RawError {
    raw(RawErrorKind::Tls(TlsFailure::UnknownIssuer))
}

/// A PostgreSQL server error with `sqlstate`.
pub fn db(sqlstate: &str) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some(sqlstate.into()),
            vendor_code: None,
            severity: Some("FATAL".into()),
            text: format!("server error {sqlstate}"),
            detail: None,
            hint: None,
            lang_hint: None,
        }),
    }
}

// ---------------------------------------------------------------------------------------------
// The scripted UI

struct UiScript {
    answers: Mutex<VecDeque<String>>,
    asked: Mutex<Vec<u8>>,
    delay: Duration,
}

/// A `UiPort` that answers password questions from a list, after `delay`.
#[derive(Clone)]
pub struct ScriptedUi(Arc<UiScript>);

impl ScriptedUi {
    pub fn answers<const N: usize>(answers: [&str; N]) -> ScriptedUi {
        Self::delayed(Duration::ZERO, answers)
    }

    pub fn delayed<const N: usize>(delay: Duration, answers: [&str; N]) -> ScriptedUi {
        ScriptedUi(Arc::new(UiScript {
            answers: Mutex::new(answers.iter().map(|a| a.to_string()).collect()),
            asked: Mutex::new(Vec::new()),
            delay,
        }))
    }

    pub fn ix(&self) -> Interaction {
        Interaction::Interactive(Arc::new(self.clone()))
    }

    /// The `attempt` of every question asked, in order.
    pub fn asked_attempts(&self) -> Vec<u8> {
        self.0.asked.lock().unwrap().clone()
    }
}

impl UiPort for ScriptedUi {
    fn ask(&self, q: Question) -> BoxFuture<'static, Answer> {
        // M2b: this UI only answers password questions; any other is cancelled.
        let Question::Password { attempt, .. } = q else {
            return Box::pin(async { Answer::Cancelled });
        };
        self.0.asked.lock().unwrap().push(attempt);
        let answer = self.0.answers.lock().unwrap().pop_front();
        let delay = self.0.delay;
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            match answer {
                Some(pw) => Answer::Password(SecretString::from(pw)),
                None => Answer::Cancelled,
            }
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Running the pipeline

pub fn ix_none() -> Interaction {
    Interaction::Headless
}

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

pub fn req(p: Profile) -> ConnectRequest {
    ConnectRequest {
        read_only: p.read_only,
        profile: Arc::new(p),
        draft_password: None,
        purpose: Purpose::Main,
        department: Department::Console,
        prior_rejections: 0,
        draft_ssh: Vec::new(),
        overlay_secrets: None,
        plaintext_consent: false,
    }
}

pub async fn connect_owned(
    deps: ConnectDeps,
    req: ConnectRequest,
    ix: Interaction,
    cancel: CancellationToken,
) -> Result<LiveConnection, ConnectError> {
    connect(&deps, req, ix, cancel, progress()).await
}

/// Connects `p` headless over a fake of its engine; it must succeed.
pub async fn connect_ok(p: Profile) -> LiveConnection {
    let (deps, _) = deps_with(FakeDriver::new(p.engine));
    connect(&deps, req(p), ix_none(), token(), progress())
        .await
        .expect("the connect succeeds")
}

pub async fn err_ix(p: Profile, ix: Interaction) -> ConnectError {
    let (deps, _) = deps_with(FakeDriver::new(p.engine));
    connect(&deps, req(p), ix, token(), progress())
        .await
        .expect_err("the connect fails")
}

pub async fn err(p: Profile) -> ConnectError {
    err_ix(p, ix_none()).await
}

pub async fn err_with(fake: &Arc<FakeDriver>, p: Profile) -> ConnectError {
    connect(&deps_shared(fake), req(p), ix_none(), token(), progress())
        .await
        .expect_err("the connect fails")
}

pub async fn ok_with(fake: &Arc<FakeDriver>, p: Profile) -> LiveConnection {
    connect(&deps_shared(fake), req(p), ix_none(), token(), progress())
        .await
        .expect("the connect succeeds")
}

pub async fn test_with(fake: &Arc<FakeDriver>, d: ProfileDraft) -> TestReport {
    test_connection(&deps_shared(fake), d, ix_none(), token(), progress()).await
}

pub fn steps(live: LiveConnection) -> Vec<TrailStep> {
    live.trail.iter().map(|t| t.step).collect()
}

/// The trail step an error failed at.
pub fn failed_step(e: ConnectError) -> TrailStep {
    e.trail
        .iter()
        .find(|t| t.outcome == StepOutcome::Failed)
        .map(|t| t.step)
        .expect("the trail has a failed step")
}

/// Everything logged at any level, on any thread, while `f`'s future runs (and before), through
/// a global `fmt` subscriber. Global, not scoped: with one scoped dispatcher, `tracing` caches
/// a callsite's interest from whichever thread hits it first, so a scoped capture misses
/// events that another test thread registered; and a global one also sees the driver's tasks
/// and the blocking pool.
pub async fn capture_tracing<F, Fut, T>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    static LOGS: OnceLock<Buf> = OnceLock::new();
    let buf = LOGS.get_or_init(|| {
        let buf = Buf::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other global subscriber in this test binary");
        buf
    });
    let _ = f().await;
    let bytes = buf.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

// ---------------------------------------------------------------------------------------------
// The manager

pub fn p() -> Arc<Profile> {
    Arc::new(pg_profile(TlsMode::Disable))
}

pub fn session_profile() -> Profile {
    Profile {
        auth: AuthProviderId::Password,
        retention: Retention::ForSession,
        ..pg_profile(TlsMode::Disable)
    }
}

pub fn three_profiles() -> Vec<Arc<Profile>> {
    (0..3)
        .map(|_| {
            Arc::new(Profile {
                id: ProfileId::new(),
                ..pg_profile(TlsMode::Disable)
            })
        })
        .collect()
}

pub fn manager_on(
    rt: tokio::runtime::Handle,
    fake: FakeDriver,
) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    (
        ConnectionManager::new(rt, Arc::new(deps), ManagerConfig::default()),
        fake,
    )
}

pub fn manager_with(
    fake: FakeDriver,
    config: ManagerConfig,
) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    (
        ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps), config),
        fake,
    )
}

pub fn manager(fake: FakeDriver) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_with(fake, ManagerConfig::default())
}

pub fn manager_with_store(
    fake: FakeDriver,
) -> (Arc<ConnectionManager>, Arc<FakeDriver>, Arc<SecretStore>) {
    let (m, fake) = manager(fake);
    (m, fake, thread_store())
}

/// Waits until `id` is connected; panics on a failure.
pub async fn wait_connected(m: &ConnectionManager, id: ProfileId) {
    let mut changes = m.changes();
    loop {
        match m.state(id) {
            LiveState::Connected(_) => return,
            LiveState::Failed(e) => panic!("the connect failed: {:?}", e.class),
            _ => {}
        }
        changes.changed().await.unwrap();
    }
}

pub async fn connect_and_wait(m: &ConnectionManager) {
    m.connect(p(), None, Interaction::Headless);
    wait_connected(m, p().id).await;
}

/// Lets spawned tasks run to their next wait.
pub async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Instrumented drivers: which stream a driver gets, and when connections open and close.

/// A transport stub whose streams carry their open number (1, 2, …) as their first byte.
fn numbered_wrap(
    opened: Arc<std::sync::atomic::AtomicU8>,
) -> hacivat_session::pipeline::TransportWrap {
    Arc::new(move |_direct: StreamFactory| {
        let opened = opened.clone();
        StreamFactory::new(move || {
            let n = opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Box::pin(async move {
                use tokio::io::AsyncWriteExt;
                let (near, mut far) = tokio::io::duplex(64);
                far.write_all(&[n]).await?;
                Ok(Box::new(near) as BoxedStream)
            })
        })
    })
}

/// Opens the request's stream factory, records the number of the stream it got, then lets
/// the fake connect.
struct ReadsStream {
    fake: Arc<FakeDriver>,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl hacivat_driver::Driver for ReadsStream {
    fn descriptor(&self) -> &hacivat_driver::DriverDescriptor {
        hacivat_driver::Driver::descriptor(self.fake.as_ref())
    }
    fn connect(
        &self,
        req: hacivat_driver::DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn hacivat_driver::RawConn>, RawError>> {
        let factory = match &req.endpoint {
            hacivat_driver::Endpoint::Network { factory, .. } => Some(factory.clone()),
            hacivat_driver::Endpoint::File { .. } => None,
        };
        let (fake, seen) = (self.fake.clone(), self.seen.clone());
        Box::pin(async move {
            if let Some(factory) = factory {
                use tokio::io::AsyncReadExt;
                let mut stream = factory.open().await.expect("a stream");
                let mut n = [0u8; 1];
                stream
                    .read_exact(&mut n)
                    .await
                    .expect("the stream's number");
                seen.lock().unwrap().push(n[0]);
            }
            hacivat_driver::Driver::connect(fake.as_ref(), req).await
        })
    }
}

/// Deps whose driver records the number of the stream each attempt read (1 is the stream the
/// transport step pre-opened).
pub fn deps_reading_streams(fake: FakeDriver) -> (ConnectDeps, Arc<Mutex<Vec<u8>>>) {
    let fake = Arc::new(fake);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(ReadsStream {
        fake: fake.clone(),
        seen: seen.clone(),
    }));
    let mut deps = deps_over(fake, thread_store());
    deps.drivers = Arc::new(drivers);
    deps.transport_wrap = Some(numbered_wrap(Arc::default()));
    (deps, seen)
}

/// An extra step (after the transport, order 41) that waits `delay`.
struct WaitStep(Duration);

impl ConnectStep for WaitStep {
    fn phase(&self) -> Phase {
        Phase::Transport
    }
    fn order(&self) -> i32 {
        41
    }
    fn applies(&self, _p: &Profile) -> bool {
        true
    }
    fn apply<'a>(
        &'a self,
        _cx: &'a mut StepCx,
    ) -> BoxFuture<'a, Result<Option<CleanupGuard>, ConnectError>> {
        let d = self.0;
        Box::pin(async move {
            tokio::time::sleep(d).await;
            Ok(None)
        })
    }
}

pub fn wait_step(delay: Duration) -> Arc<dyn ConnectStep> {
    Arc::new(WaitStep(delay))
}

/// A connection log: `open`, `ping`, `close`, in order.
pub type Log = Arc<Mutex<Vec<&'static str>>>;

/// A connection whose close takes `close_delay` and whose pings may hang, logging both.
struct LoggedConn {
    inner: Box<dyn hacivat_driver::RawConn>,
    log: Log,
    close_delay: Duration,
    hang_pings: bool,
}

impl hacivat_driver::RawConn for LoggedConn {
    fn kind(&self) -> DbmsKind {
        self.inner.kind()
    }
    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        self.inner.server_info()
    }
    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        self.log.lock().unwrap().push("ping");
        if self.hang_pings {
            Box::pin(std::future::pending())
        } else {
            self.inner.ping()
        }
    }
    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        self.inner.tls_summary()
    }
    fn probe(&mut self) -> BoxFuture<'_, Vec<hacivat_driver::ProbeWarning>> {
        self.inner.probe()
    }
    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        self.inner.current_user()
    }
    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        self.inner.list_databases(limit)
    }
    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        let LoggedConn {
            inner,
            log,
            close_delay,
            ..
        } = *self;
        Box::pin(async move {
            tokio::time::sleep(close_delay).await;
            inner.close().await;
            log.lock().unwrap().push("close");
        })
    }
    fn cancel_handle(&self) -> Option<Arc<dyn hacivat_driver::CancelHandle>> {
        None
    }
    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: hacivat_driver::ExecOpts,
    ) -> BoxFuture<'a, Result<hacivat_driver::ExecSummary, RawError>> {
        self.inner.execute(sql, opts)
    }
    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        self.inner.batch(sql)
    }
    fn tx_status(&self) -> hacivat_driver::TxStatus {
        self.inner.tx_status()
    }
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        self.inner.reset()
    }
    fn backend_id(&self) -> Option<u64> {
        self.inner.backend_id()
    }
}

struct LoggedDriver {
    fake: Arc<FakeDriver>,
    log: Log,
    close_delay: Duration,
    hang_pings: bool,
}

impl hacivat_driver::Driver for LoggedDriver {
    fn descriptor(&self) -> &hacivat_driver::DriverDescriptor {
        hacivat_driver::Driver::descriptor(self.fake.as_ref())
    }
    fn connect(
        &self,
        req: hacivat_driver::DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn hacivat_driver::RawConn>, RawError>> {
        let fut = hacivat_driver::Driver::connect(self.fake.as_ref(), req);
        let (log, close_delay, hang_pings) = (self.log.clone(), self.close_delay, self.hang_pings);
        Box::pin(async move {
            let inner = fut.await?;
            log.lock().unwrap().push("open");
            Ok(Box::new(LoggedConn {
                inner,
                log,
                close_delay,
                hang_pings,
            }) as Box<dyn hacivat_driver::RawConn>)
        })
    }
}

/// A manager over a logging driver for `kind`.
pub fn logged_manager(
    kind: DbmsKind,
    close_delay: Duration,
    hang_pings: bool,
    config: ManagerConfig,
) -> (Arc<ConnectionManager>, Log) {
    let fake = Arc::new(FakeDriver::new(kind));
    let log: Log = Arc::default();
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(LoggedDriver {
        fake: fake.clone(),
        log: log.clone(),
        close_delay,
        hang_pings,
    }));
    let mut deps = deps_over(fake, thread_store());
    deps.drivers = Arc::new(drivers);
    (
        ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps), config),
        log,
    )
}

// ## m2b-t01-interfaces

/// The SSH stores of a test's `ConnectDeps`: a tunnel manager over a known-hosts store whose
/// files do not exist (loading does no I/O, Q42).
pub fn ssh_stubs() -> (
    Arc<hacivat_net::ssh::TunnelManager>,
    Arc<hacivat_net::ssh::KnownHosts>,
) {
    let known = hacivat_net::ssh::KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = hacivat_net::ssh::TunnelManager::new(known.clone(), Default::default());
    (tunnels, known)
}

// ## m2b-t07-session-actor
// Session actor fixtures (Task 7): sessions over a `FakeDriver` connected through the real
// pipeline, scripted statements, and condition waits.

use hacivat_driver::fake::FakeExec;
use hacivat_driver::{ExecSummary, TxStatus};
use hacivat_proto::EnvTag;
use hacivat_session::session::{
    ClientId, ExecOrigin, Reply, Request, SchemaCtx, SessionEvent, SessionHandle, SessionHooks,
    SessionKind, SessionSnapshot, SessionSpec, SideConnector, TxCommand, spawn_session,
};

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

pub fn fake_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres)
}

pub fn fake_mysql() -> FakeDriver {
    FakeDriver::new(DbmsKind::MySql)
}

fn scripted(prefix: &'static str, result: Result<ExecSummary, RawError>) -> FakeExec {
    FakeExec {
        prefix,
        result,
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// A statement that runs for `delay` (a cancel ends it early).
pub fn slow(prefix: &'static str, delay: Duration) -> FakeExec {
    FakeExec {
        delay,
        ..scripted(prefix, Ok(ExecSummary::default()))
    }
}

/// A statement after which the connection's transaction status is `status`.
pub fn tx(prefix: &'static str, status: TxStatus) -> FakeExec {
    FakeExec {
        tx_after: Some(status),
        ..scripted(prefix, Ok(ExecSummary::default()))
    }
}

/// A statement the server refuses with `sqlstate`, leaving the status `status`.
pub fn tx_err(prefix: &'static str, sqlstate: &str, status: TxStatus) -> FakeExec {
    FakeExec {
        tx_after: Some(status),
        ..scripted(prefix, Err(db(sqlstate)))
    }
}

/// A statement that answers `rows` (text values).
pub fn rows(prefix: &'static str, rows: &[&[&str]]) -> FakeExec {
    let rows: Vec<Vec<Option<String>>> = rows
        .iter()
        .map(|r| r.iter().map(|v| Some((*v).to_string())).collect())
        .collect();
    scripted(
        prefix,
        Ok(ExecSummary {
            row_count: rows.len() as u64,
            rows,
            ..ExecSummary::default()
        }),
    )
}

/// A statement whose driver call panics.
pub fn panics(prefix: &'static str) -> FakeExec {
    FakeExec {
        panics: true,
        ..scripted(prefix, Ok(ExecSummary::default()))
    }
}

/// A user statement.
pub fn exec(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: ExecOrigin::User,
    }
}

fn profile_of(kind: DbmsKind) -> Profile {
    match kind {
        DbmsKind::MySql | DbmsKind::MariaDb => mysql_profile(TlsMode::Disable),
        DbmsKind::Sqlite => sqlite_profile(),
        _ => pg_profile(TlsMode::Disable),
    }
}

/// A read-only PostgreSQL profile.
pub fn ro_profile() -> Profile {
    ro(pg_profile(TlsMode::Disable))
}

/// A PostgreSQL profile tagged production (default mode Manual).
pub fn production_profile() -> Profile {
    Profile {
        environment: Some(EnvTag::Production),
        ..pg_profile(TlsMode::Disable)
    }
}

/// A schema context whose current schema is `name`.
pub fn schema(name: &str) -> SchemaCtx {
    SchemaCtx {
        schema: Some(name.to_string()),
        ..SchemaCtx::default()
    }
}

/// Side connections over a fake (cancel, kill), through the real pipeline.
struct FakeSide {
    fake: Arc<FakeDriver>,
    profile: Profile,
}

impl SideConnector for FakeSide {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let deps = deps_shared(&self.fake);
        let mut request = req(self.profile.clone());
        request.department = Department::Cancel;
        Box::pin(async move { connect(&deps, request, ix_none(), token(), progress()).await })
    }
}

/// Connects `profile` over `fake` and spawns its session; side connections go to `side`.
pub async fn spawn_over(
    fake: FakeDriver,
    profile: Profile,
    read_only: bool,
    side: Arc<FakeDriver>,
) -> (SessionHandle, Arc<FakeDriver>) {
    let fake = Arc::new(fake);
    let mut request = req(profile.clone());
    request.read_only = read_only;
    let live = connect(&deps_shared(&fake), request, ix_none(), token(), progress())
        .await
        .expect("the session's connect succeeds");
    let side: Arc<dyn SideConnector> = Arc::new(FakeSide {
        fake: side,
        profile: profile.clone(),
    });
    let hooks = SessionHooks {
        side,
        reconnect: None,
    };
    let spec = SessionSpec {
        kind: SessionKind::Console(ClientId::DEFAULT),
        read_only,
        department: Department::Console,
    };
    let h = spawn_session(
        &tokio::runtime::Handle::current(),
        spec,
        Arc::new(profile),
        live,
        hooks,
    );
    (h, fake)
}

/// A read-write session over `fake` (its engine's test profile).
pub async fn session(fake: FakeDriver) -> (SessionHandle, Arc<FakeDriver>) {
    let kind = hacivat_driver::Driver::descriptor(&fake).kind;
    let side = Arc::new(FakeDriver::new(kind));
    spawn_over(fake, profile_of(kind), false, side).await
}

/// A read-only session of `profile` over `fake`.
pub async fn ro_session(fake: FakeDriver, profile: Profile) -> (SessionHandle, Arc<FakeDriver>) {
    let side = Arc::new(FakeDriver::new(profile.engine));
    spawn_over(fake, ro(profile), true, side).await
}

/// A read-write session over `fake` whose side connections go to `side`.
pub async fn session_with_side(
    fake: FakeDriver,
    side: &Arc<FakeDriver>,
) -> (SessionHandle, Arc<FakeDriver>) {
    let kind = hacivat_driver::Driver::descriptor(&fake).kind;
    spawn_over(fake, profile_of(kind), false, side.clone()).await
}

/// A PostgreSQL session inside a transaction (the user ran `BEGIN`); its reset verify probe
/// answers `t`.
pub async fn in_tx_session() -> (SessionHandle, Arc<FakeDriver>) {
    let (h, f) = session(fake_pg().exec(vec![
        tx("BEGIN", TxStatus::InTx),
        rows("SELECT current_user = session_user", &[&["t"]]),
    ]))
    .await;
    run(&h, "BEGIN").await.expect("BEGIN runs");
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    (h, f)
}

/// Runs the user statement `sql` and waits for its reply.
pub async fn run(h: &SessionHandle, sql: &str) -> Reply {
    h.submit(exec(sql)).reply.await.expect("the actor replies")
}

/// Runs a transaction command and waits for its reply.
pub async fn tx_cmd(h: &SessionHandle, cmd: TxCommand) -> Reply {
    h.submit(Request::Tx(cmd))
        .reply
        .await
        .expect("the actor replies")
}

/// The session's current snapshot.
pub fn snap(h: &SessionHandle) -> SessionSnapshot {
    h.state().borrow().clone()
}

/// The next event, within [`WAIT`].
pub async fn next(ev: &mut tokio::sync::broadcast::Receiver<SessionEvent>) -> SessionEvent {
    tokio::time::timeout(WAIT, ev.recv())
        .await
        .expect("an event within the wait")
        .expect("the event channel is open")
}

/// The next `SchemaChanged` event, skipping the others.
pub async fn next_schema_changed(
    ev: &mut tokio::sync::broadcast::Receiver<SessionEvent>,
) -> SessionEvent {
    loop {
        let e = next(ev).await;
        if matches!(e, SessionEvent::SchemaChanged(_)) {
            return e;
        }
    }
}

/// Yields (never sleeps, so a paused clock does not move) until `done` holds; panics naming
/// `what` after [`WAIT`] of real time.
pub async fn settle_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out after {WAIT:?} waiting for {what}"
        );
        tokio::task::yield_now().await;
    }
}

// ## m2b-t14-pipeline
// The M2b scripted UI (Task 14): it answers every question kind from a list, in order, and
// records each question it was asked.

/// A `UiPort` that answers any question with the next scripted [`Answer`] (then `Cancelled`),
/// after `delay` of real time, and records every question.
#[derive(Clone)]
pub struct ScriptedUiM2b(Arc<UiScriptM2b>);

struct UiScriptM2b {
    answers: Mutex<VecDeque<Answer>>,
    asked: Mutex<Vec<Question>>,
    delay: Duration,
}

impl ScriptedUi {
    /// The M2b scripted UI over `answers`.
    pub fn answers_m2b<const N: usize>(answers: [Answer; N]) -> ScriptedUiM2b {
        ScriptedUiM2b(Arc::new(UiScriptM2b {
            answers: Mutex::new(answers.into_iter().collect()),
            asked: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
        }))
    }
}

impl ScriptedUiM2b {
    /// The same script, each answer given after `delay` (a user who takes their time).
    pub fn delay(self, delay: Duration) -> ScriptedUiM2b {
        let script = Arc::try_unwrap(self.0).unwrap_or_else(|_| panic!("a fresh script"));
        ScriptedUiM2b(Arc::new(UiScriptM2b { delay, ..script }))
    }

    pub fn ix(&self) -> Interaction {
        Interaction::Interactive(Arc::new(self.clone()))
    }

    /// Every question asked, in order.
    pub fn asked(&self) -> Vec<Question> {
        self.0.asked.lock().unwrap().clone()
    }

    /// The `attempt` of every `UnlockVault` question, in order.
    pub fn unlock_attempts(&self) -> Vec<u8> {
        self.asked()
            .into_iter()
            .filter_map(|q| match q {
                Question::UnlockVault { attempt } => Some(attempt),
                _ => None,
            })
            .collect()
    }
}

impl UiPort for ScriptedUiM2b {
    fn ask(&self, q: Question) -> BoxFuture<'static, Answer> {
        self.0.asked.lock().unwrap().push(q);
        let answer = self.0.answers.lock().unwrap().pop_front();
        let delay = self.0.delay;
        Box::pin(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            answer.unwrap_or(Answer::Cancelled)
        })
    }
}

/// A secret for an answer (an obvious fixture value).
pub fn sec(s: &str) -> SecretString {
    SecretString::from(s.to_string())
}

/// An interactive UI that answers its first question (an SSH password prompt under
/// `AcceptNew`) with `pw`, and cancels any other.
pub fn ui_secret(pw: &str) -> Interaction {
    ScriptedUi::answers_m2b([Answer::Secret(sec(pw))]).ix()
}

// ## m2b-t23-conformance
// The Docker conformance suite's helpers (Task 23; M2 exit criteria E1–E8). Docker only: the
// hygiene note at the top of this file (IP literals, no sockets) is for the FakeDriver tests.
// These reach the real matrix: targets only through `require_target`/`targets` (Q6), the
// bastions `sshd-a`/`sshd-b` through `require_ssh`, and through SSH always the target's
// in-network endpoint (`DockerTarget.internal`).

/// The engine conformance fixtures of M2a Task 19 (real drivers, Docker profiles), shared. A
/// test file that declares its own `mod docker_support` (Task 17's `manager_m2b`) loads the
/// file twice, deliberately: each copy is private to its module.
#[allow(
    unused_imports,
    clippy::duplicate_mod,
    reason = "each test file uses a subset; manager_m2b loads it too"
)]
#[path = "../docker_support/mod.rs"]
pub mod docker_support;

pub mod conformance {
    //! Paths (direct, one bastion, the jump chain), SSH deps with the bastions' host keys,
    //! managers and sessions on real servers, the admin side channel, `ChaosProxy` profiles and
    //! a proxy that holds the server's last words until the client speaks.
    //!
    //! Failure messages carry target ids, users and error classes; a statement that may hold a
    //! password is never printed.

    use std::net::SocketAddr;
    use std::path::{Path as FsPath, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use hacivat_driver::{ExecOpts, ExecSummary, Protocol, ServerMessage};
    use hacivat_net::TlsMode;
    use hacivat_net::ssh::{
        HopSecrets, HopSpec, HostKeyPolicy, KnownHosts, SshAuth, SshSpec, TunnelConfig,
        TunnelManager,
    };
    use hacivat_proto::{DbmsKind, ProfileId};
    use hacivat_secrets::SecretString;
    use hacivat_session::pipeline::{
        ConnectDeps, ConnectError, Interaction, LiveConnection, SessionSecrets, connect,
    };
    use hacivat_session::profile::{Profile, ProfileEndpoint, TransportSpec};
    use hacivat_session::session::{ExecOrigin, LostState, Reply, Request, SessionHandle};
    use hacivat_session::{ConnectionEvent, ConnectionManager, LiveState, ManagerConfig, PathKind};
    use hacivat_testkit::chaos::ChaosProxy;
    use hacivat_testkit::{
        DockerTarget, SSH_CLIENT_KEY_PASSPHRASE, SshTarget, TlsDefault, ssh_client_key,
        ssh_host_pub, ssh_target,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{Notify, broadcast, mpsc};

    pub use super::docker_support::*;

    /// How long a manager may take to (re)connect a Docker target before a test fails.
    pub const CONNECT_LIMIT: Duration = Duration::from_secs(60);

    pub fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // -----------------------------------------------------------------------------------------
    // Paths

    /// How a test reaches its database (`alg/01` §5): directly, through one bastion, or through
    /// the `sshd-a` → `sshd-b` jump chain.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Path {
        Direct,
        Ssh(&'static str),
        Chain(&'static str, &'static str),
    }

    impl Path {
        /// What the capsule shows for this path.
        pub fn kind(self) -> PathKind {
            match self {
                Path::Direct => PathKind::Direct,
                Path::Ssh(_) | Path::Chain(..) => PathKind::Ssh,
            }
        }

        pub fn hops(self) -> u8 {
            match self {
                Path::Direct => 0,
                Path::Ssh(_) => 1,
                Path::Chain(..) => 2,
            }
        }

        /// The bastions of this path, in order; `None` (with the skip line) when one is not in
        /// the active set.
        fn bastions(self) -> Option<Vec<SshTarget>> {
            let ids: Vec<&str> = match self {
                Path::Direct => Vec::new(),
                Path::Ssh(a) => vec![a],
                Path::Chain(a, b) => vec![a, b],
            };
            ids.into_iter().map(require_ssh).collect()
        }

        /// Whether every bastion of this path is in the active set.
        pub fn available(self) -> bool {
            self.bastions().is_some()
        }
    }

    /// The bastion `id` a test needs: `None` (with `skip <id>: …` on stderr) when the active
    /// set leaves it out. One published on the host must accept a TCP connection, else this
    /// panics (a bastion in the set never skips; Q6).
    pub fn require_ssh(id: &str) -> Option<SshTarget> {
        let Some(t) = ssh_target(id) else {
            eprintln!("skip {id}: not in the active set");
            return None;
        };
        if t.via.is_none() {
            let addr: SocketAddr = format!("{}:{}", t.host, t.port)
                .parse()
                .unwrap_or_else(|_| panic!("{id}: the bastion's host is an IP literal"));
            assert!(
                std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)).is_ok(),
                "{id} is in the active set but does not accept a connection"
            );
        }
        Some(t)
    }

    /// The TCP targets of the active set (pg13, below the floor, and pg17-tr, a locale twin,
    /// excepted; Task 19's `matrix`).
    pub fn tcp_targets() -> Vec<DockerTarget> {
        matrix()
    }

    /// Whether `t` is in the `pr` set (the jump chain runs only for those, Task 23 brief).
    pub fn in_pr_set(t: &DockerTarget) -> bool {
        t.sets.iter().any(|s| s == "pr")
    }

    // -----------------------------------------------------------------------------------------
    // SSH: keys, known hosts, deps

    /// A private copy of the generated client key (`encrypted`: the one protected by
    /// [`SSH_CLIENT_KEY_PASSPHRASE`]). The generated files are world-readable for Linux CI,
    /// and Hacivat, like OpenSSH, refuses a private key other users can read (T14 M6), so the
    /// copy is `0600`. Written once per process into the Docker artifacts directory.
    pub fn client_key(encrypted: bool) -> PathBuf {
        static PLAIN: OnceLock<PathBuf> = OnceLock::new();
        static ENC: OnceLock<PathBuf> = OnceLock::new();
        let cell = if encrypted { &ENC } else { &PLAIN };
        cell.get_or_init(|| {
            let from = ssh_client_key(encrypted);
            let name = from
                .file_name()
                .expect("the key has a file name")
                .to_string_lossy()
                .into_owned();
            let to = hacivat_testkit::artifact(&format!("t23-{name}"));
            let tmp = hacivat_testkit::artifact(&format!("t23-{name}.{}", std::process::id()));
            std::fs::copy(&from, &tmp).expect("the generated client key is readable");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                    .expect("the key copy is made private");
            }
            std::fs::rename(&tmp, &to).expect("the key copy is in place");
            to
        })
        .clone()
    }

    /// One `known_hosts` line: bastion `id`'s published host key at `host`:`port`.
    fn known_line(host: &str, port: u16, id: &str) -> String {
        let key = std::fs::read_to_string(ssh_host_pub(id))
            .unwrap_or_else(|e| panic!("{id}: its host key artifact is unreadable: {e}"));
        let mut words = key.split_whitespace();
        let (Some(kind), Some(blob)) = (words.next(), words.next()) else {
            panic!("{id}: its host key artifact is not `<type> <base64>`");
        };
        format!(
            "{} {kind} {blob}\n",
            hacivat_net::ssh::known_hosts::host_pattern(host, port)
        )
    }

    /// An OpenSSH `known_hosts` file in `dir` listing both bastions where the tests dial them
    /// (`sshd-a` on the host, `sshd-b` by its in-network name), plus `extra`: (host, port,
    /// bastion) for an endpoint in front of a bastion (a proxy).
    pub fn known_hosts_file(dir: &FsPath, extra: &[(String, u16, &str)]) -> PathBuf {
        let path = dir.join("known_hosts");
        std::fs::write(&path, known_text(extra)).expect("the known_hosts file is written");
        path
    }

    /// The `known_hosts` text of [`known_hosts_file`].
    fn known_text(extra: &[(String, u16, &str)]) -> String {
        let mut text = String::new();
        for id in ["sshd-a", "sshd-b"] {
            if let Some(t) = ssh_target(id) {
                text.push_str(&known_line(&t.host, t.port, id));
            }
        }
        for (host, port, id) in extra {
            text.push_str(&known_line(host, *port, id));
        }
        text
    }

    /// The matrix's `known_hosts` (both bastions), written once per process into the Docker
    /// artifacts directory (`t23-known-hosts`: the same text from every process, renamed into
    /// place).
    fn matrix_known_hosts() -> PathBuf {
        let to = hacivat_testkit::artifact("t23-known-hosts");
        let tmp = hacivat_testkit::artifact(&format!("t23-known-hosts.{}", std::process::id()));
        std::fs::write(&tmp, known_text(&[])).expect("the known_hosts file is written");
        std::fs::rename(&tmp, &to).expect("the known_hosts file is in place");
        to
    }

    /// The real drivers (Task 19's `deps`) with SSH that trusts exactly the host keys `openssh`
    /// lists (`Strict`: nothing is learned, nothing written) and no agent.
    pub fn ssh_deps(openssh: PathBuf) -> ConnectDeps {
        let known = KnownHosts::load(
            vec![openssh],
            PathBuf::from("/nonexistent/hacivat-t23/known_hosts"),
        );
        let tunnels = TunnelManager::with_agent(known.clone(), TunnelConfig::default(), None);
        ConnectDeps {
            tunnels,
            known_hosts: known,
            ..deps()
        }
    }

    /// [`ssh_deps`] over the matrix's own `known_hosts` (both bastions).
    pub fn matrix_deps() -> ConnectDeps {
        static FILE: OnceLock<PathBuf> = OnceLock::new();
        let file = FILE.get_or_init(matrix_known_hosts);
        ssh_deps(file.clone())
    }

    /// Hop `t` signing in with `auth`, `IdentitiesOnly` (the agent is never asked).
    pub fn hop(t: &SshTarget, auth: SshAuth) -> HopSpec {
        HopSpec {
            host: t.host.clone(),
            port: t.port,
            user: t.user.clone(),
            auth,
            identities_only: true,
            identity_files: Vec::new(),
        }
    }

    /// The private copy of the plain client key.
    pub fn key_auth() -> SshAuth {
        SshAuth::KeyFile {
            path: client_key(false),
        }
    }

    /// The hops of `path`: one bastion signs in with the plain key; the chain's first hop with
    /// the encrypted key (its passphrase from the overlay) and its second with the bastion
    /// password ([`chain_secrets`]), so the chain exercises both secret kinds.
    fn hops_of(path: Path) -> Option<Vec<HopSpec>> {
        let bastions = path.bastions()?;
        Some(match path {
            Path::Direct => Vec::new(),
            Path::Ssh(_) => vec![hop(&bastions[0], key_auth())],
            Path::Chain(..) => vec![
                hop(
                    &bastions[0],
                    SshAuth::KeyFile {
                        path: client_key(true),
                    },
                ),
                hop(&bastions[1], SshAuth::Password),
            ],
        })
    }

    /// The chain's hop secrets, as a session overlay carries them (Q17).
    pub fn chain_secrets() -> SessionSecrets {
        let password = ssh_target("sshd-b")
            .map(|t| t.password)
            .expect("sshd-b is in the active set");
        SessionSecrets {
            hops: vec![
                HopSecrets {
                    password: None,
                    passphrase: Some(SecretString::from(SSH_CLIENT_KEY_PASSPHRASE.to_string())),
                },
                HopSecrets {
                    password: Some(SecretString::from(password)),
                    passphrase: None,
                },
            ],
            ..SessionSecrets::default()
        }
    }

    /// `p` sent over `hops` to `t`'s in-network endpoint (`t.internal`), `Strict` host keys.
    pub fn through(t: &DockerTarget, mut p: Profile, hops: Vec<HopSpec>) -> Profile {
        p.endpoint = ProfileEndpoint::Server {
            host: t.internal.0.clone(),
            port: Some(t.internal.1),
        };
        p.transport = TransportSpec::SshTunnel(SshSpec {
            hops,
            host_keys: HostKeyPolicy::Strict,
            keepalive_s: None,
        });
        p
    }

    /// Target `t`'s Task 19 profile (TLS `mode`) on `path`; `None` when a bastion of the path is
    /// not in the active set.
    pub fn profile_on(t: &DockerTarget, path: Path, mode: TlsMode) -> Option<Profile> {
        let p = profile_for(t, mode);
        if path == Path::Direct {
            return Some(p);
        }
        Some(through(t, p, hops_of(path)?))
    }

    /// The TLS mode a matrix test uses for `t` on `path`: `auto_or_require`, except that a
    /// MySQL-family server without TLS reached through SSH is `Disable`. MySQL's `Auto` falls
    /// back to plaintext on loopback only (M2a; widening it to private addresses is the open PO
    /// question Q41), so through a bastion `Auto` is `tls.not_supported_by_server` there.
    pub fn mode_on(t: &DockerTarget, path: Path) -> TlsMode {
        let mysql_family = matches!(t.engine, DbmsKind::MySql | DbmsKind::MariaDb);
        if path != Path::Direct && mysql_family && t.tls_default == TlsDefault::None {
            TlsMode::Disable
        } else {
            auto_or_require(t)
        }
    }

    /// A headless Main connect of `t` (its [`mode_on`] mode) on `path`, with the Docker
    /// password (and the chain's secrets on the chain). Panics when a bastion of the path is
    /// not in the active set (callers check `Path::available`).
    pub async fn connect_main(
        t: &DockerTarget,
        path: Path,
    ) -> Result<LiveConnection, ConnectError> {
        let p = profile_on(t, path, mode_on(t, path))
            .unwrap_or_else(|| panic!("{path:?}: a bastion is not in the active set"));
        let mut r = req(p);
        if matches!(path, Path::Chain(..)) {
            r.overlay_secrets = Some(chain_secrets());
        }
        connect(
            &matrix_deps(),
            r,
            Interaction::Headless,
            token(),
            progress(),
        )
        .await
    }

    // -----------------------------------------------------------------------------------------
    // Managers and sessions

    /// A `ConnectionManager` with M2b's configuration (B18 presets, B19 reconnects) over `deps`.
    pub fn manager_over(deps: ConnectDeps) -> Arc<ConnectionManager> {
        ConnectionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(deps),
            ManagerConfig::m2b(),
        )
    }

    /// Waits until `id`'s state matches `what`, for at most `limit`; panics with the state.
    pub async fn wait_state_of(
        m: &ConnectionManager,
        id: ProfileId,
        limit: Duration,
        what: impl Fn(&LiveState) -> bool,
    ) -> LiveState {
        let mut changes = m.changes();
        let waited = tokio::time::timeout(limit, async {
            loop {
                let state = m.state(id);
                if what(&state) {
                    return state;
                }
                tokio::select! {
                    _ = changes.changed() => {}
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
        })
        .await;
        waited.unwrap_or_else(|_| panic!("timed out waiting; the state is {:?}", m.state(id)))
    }

    /// Waits until `id` is connected within `limit`; panics on `Failed` or `Lost`.
    pub async fn wait_connected_within(m: &ConnectionManager, id: ProfileId, limit: Duration) {
        wait_state_of(m, id, limit, |s| {
            if let LiveState::Failed(e) = s {
                panic!("the connect failed: {:?}", e.class);
            }
            if let LiveState::Lost { class } = s {
                panic!("the profile is lost: {class:?}");
            }
            matches!(s, LiveState::Connected(_))
        })
        .await;
    }

    /// Connects `p` on `m` with the Docker password and waits until it is connected.
    pub async fn connect_on(m: &ConnectionManager, p: &Arc<Profile>) {
        m.connect(p.clone(), Some(docker_password()), Interaction::Headless);
        wait_connected_within(m, p.id, CONNECT_LIMIT).await;
    }

    pub fn default_session(m: &ConnectionManager, id: ProfileId) -> SessionHandle {
        m.sessions(id)
            .expect("the profile is connected")
            .default_session()
            .expect("it has a default session")
    }

    /// A profile connected on its own manager, and its default session.
    pub struct OnManager {
        pub m: Arc<ConnectionManager>,
        pub profile: Arc<Profile>,
        pub s: SessionHandle,
    }

    impl OnManager {
        /// Connects `p` on a fresh M2b manager over `deps`.
        pub async fn connect(deps: ConnectDeps, p: Profile) -> OnManager {
            let m = manager_over(deps);
            let profile = Arc::new(p);
            connect_on(&m, &profile).await;
            let s = default_session(&m, profile.id);
            OnManager { m, profile, s }
        }

        pub fn id(&self) -> ProfileId {
            self.profile.id
        }

        /// The default session now (a reconnect keeps it; a Retry makes a new one).
        pub fn session(&self) -> SessionHandle {
            default_session(&self.m, self.profile.id)
        }

        pub async fn shutdown(self) {
            self.m.shutdown().await;
        }
    }

    /// Target `t` on `path` (its [`mode_on`] mode), connected on its own manager over
    /// the matrix deps. One bastion signs in with the plain key, so no secret is needed.
    pub async fn session_on(t: &DockerTarget, path: Path) -> OnManager {
        assert!(
            !matches!(path, Path::Chain(..)),
            "a manager connect reads SSH secrets from the store; the chain is not wired here"
        );
        let p = profile_on(t, path, mode_on(t, path))
            .unwrap_or_else(|| panic!("{path:?}: a bastion is not in the active set"));
        OnManager::connect(matrix_deps(), p).await
    }

    /// A user statement (the extended protocol, the session's default).
    pub fn user_exec(sql: &str) -> Request {
        Request::Exec {
            sql: sql.to_string(),
            opts: ExecOpts::default(),
            origin: ExecOrigin::User,
        }
    }

    /// A user statement over the simple (text) protocol: its values come back.
    pub fn simple_exec(sql: &str) -> Request {
        Request::Exec {
            sql: sql.to_string(),
            opts: ExecOpts {
                protocol: Protocol::Simple,
                ..ExecOpts::default()
            },
            origin: ExecOrigin::User,
        }
    }

    pub async fn run(s: &SessionHandle, sql: &str) -> Reply {
        s.submit(user_exec(sql))
            .reply
            .await
            .expect("the actor replies")
    }

    /// The statement that answers a connection's server-side ID.
    pub fn backend_id_sql(engine: DbmsKind) -> &'static str {
        match engine {
            DbmsKind::Postgres => "SELECT pg_backend_pid()",
            _ => "SELECT CONNECTION_ID()",
        }
    }

    /// The server-side ID of read-write session `s`'s connection (an internal, idempotent
    /// statement: no user activity, nothing tracked).
    pub async fn backend_id(s: &SessionHandle, engine: DbmsKind) -> String {
        let answer = s
            .submit(Request::Exec {
                sql: backend_id_sql(engine).to_string(),
                opts: ExecOpts {
                    protocol: Protocol::Simple,
                    ..ExecOpts::default()
                },
                origin: ExecOrigin::Internal { idempotent: true },
            })
            .reply
            .await
            .expect("the actor replies")
            .unwrap_or_else(|e| panic!("the backend ID: {:?}", e.kind));
        answer
            .rows
            .first()
            .and_then(|r| r.first())
            .cloned()
            .flatten()
            .expect("a backend ID")
    }

    /// The next manager event within 30 s.
    pub async fn next_event(ev: &mut broadcast::Receiver<ConnectionEvent>) -> ConnectionEvent {
        tokio::time::timeout(secs(30), ev.recv())
            .await
            .expect("an event within 30 s")
            .expect("the manager is alive")
    }

    /// The next `SessionLost` within 30 s, other events skipped.
    pub async fn wait_session_lost(ev: &mut broadcast::Receiver<ConnectionEvent>) -> LostState {
        loop {
            if let ConnectionEvent::SessionLost { lost, .. } = next_event(ev).await {
                return lost;
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // The admin side channel

    /// The test account (`hv_test`), read-write and direct: the fixtures' admin and the side
    /// channel that checks what a statement did. The floor is lifted for pg13.
    pub async fn admin(t: &DockerTarget) -> LiveConnection {
        admin_on(t, &t.database).await
    }

    /// [`admin`] on database `db`.
    pub async fn admin_on(t: &DockerTarget, db: &str) -> LiveConnection {
        let mut p = profile_for(t, auto_or_require(t)).database(db);
        p.allow_unsupported_server = true;
        connect(&deps(), req(p), Interaction::Headless, token(), progress())
            .await
            .unwrap_or_else(|e| panic!("{} admin on {db}: {:?}", t.id, e.class))
    }

    /// The first value of the first row of `sql` (simple protocol, so values are kept).
    pub async fn scalar(live: &mut LiveConnection, sql: &str) -> String {
        let answer: ExecSummary = live
            .raw()
            .execute(
                sql,
                ExecOpts {
                    max_rows: 1000,
                    protocol: Protocol::Simple,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?} {:?}", e.kind, code_of(e.server.as_ref())));
        answer
            .rows
            .first()
            .and_then(|row| row.first())
            .cloned()
            .flatten()
            .unwrap_or_default()
    }

    /// `sql`'s single integer.
    pub async fn count(live: &mut LiveConnection, sql: &str) -> i64 {
        let v = scalar(live, sql).await;
        v.parse()
            .unwrap_or_else(|_| panic!("{sql}: `{v}` is not an integer"))
    }

    /// Runs fixture text on `live`. A failure names the statement only up to its first literal
    /// (a literal may hold the matrix password) and the server's code.
    pub async fn batch(live: &mut LiveConnection, id: &str, sql: &str) {
        if let Err(e) = live.raw().batch(sql).await {
            let shown = sql.split('\'').next().unwrap_or_default();
            panic!(
                "{id}: fixture `{shown}…`: {:?} {:?}",
                e.kind,
                code_of(e.server.as_ref())
            );
        }
    }

    /// A server message's code: MySQL's vendor code (its SQLSTATEs are coarse), else the
    /// SQLSTATE.
    pub fn code_of(m: Option<&ServerMessage>) -> Option<String> {
        let m = m?;
        match (m.vendor_code, &m.sqlstate) {
            (Some(code), _) => Some(code.to_string()),
            (None, Some(state)) => Some(state.clone()),
            (None, None) => None,
        }
    }

    /// Ends server connection `backend` of `t` from the admin side (PostgreSQL
    /// `pg_terminate_backend`, MySQL/MariaDB `KILL`).
    pub async fn kill_backend(t: &DockerTarget, backend: &str) {
        let sql = match t.engine {
            DbmsKind::Postgres => format!("SELECT pg_terminate_backend({backend})"),
            _ => format!("KILL {backend}"),
        };
        let mut a = admin(t).await;
        batch(&mut a, &t.id, &sql).await;
        a.close().await;
    }

    /// A name unique to this process and call (`[a-z0-9_]`), for `application_name` and
    /// fixture names.
    pub fn unique(prefix: &str) -> String {
        static N: AtomicU64 = AtomicU64::new(0);
        format!(
            "{prefix}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    /// Waits (real time, 50 ms steps) until `done`, for at most `limit`.
    pub async fn wait_until(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + limit;
        while !done() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Proxies

    /// The loopback address of `t`'s published port.
    pub fn published(t: &DockerTarget) -> SocketAddr {
        format!("{}:{}", t.host, t.port)
            .parse()
            .expect("targets listen on an IP literal")
    }

    /// A `ChaosProxy` in front of `t`'s published port.
    pub async fn chaos_for(t: &DockerTarget) -> ChaosProxy {
        ChaosProxy::start(published(t)).await
    }

    /// `t`'s Task 19 profile (its `auto_or_require` mode) through a proxy at `addr`.
    pub fn profile_via(t: &DockerTarget, addr: SocketAddr) -> Profile {
        profile_for(t, auto_or_require(t))
            .host(&addr.ip().to_string())
            .port(addr.port())
    }

    /// A loopback TCP proxy that can hold what the server says until the client speaks again
    /// (T17 carry P1): armed, it keeps the server's bytes (and its end of the stream) back, and
    /// the client's next bytes release them. So a server's `FATAL` sent while the client was
    /// idle arrives as the answer to the client's next statement, as it does when the server
    /// ends the session during that statement's round trip.
    pub struct HoldProxy {
        pub addr: SocketAddr,
        relays: Arc<Mutex<Vec<Arc<HeldRelay>>>>,
        accept: tokio::task::JoinHandle<()>,
    }

    /// One relayed connection's hold.
    struct HeldRelay {
        armed: AtomicBool,
        release: Notify,
        /// The server ended its side while the hold was armed.
        server_ended: AtomicBool,
    }

    enum Down {
        Bytes(Vec<u8>),
        End,
    }

    impl HoldProxy {
        /// Starts the proxy toward `upstream` on a free loopback port.
        pub async fn start(upstream: SocketAddr) -> HoldProxy {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind the hold proxy");
            let addr = listener.local_addr().expect("the proxy address");
            let relays: Arc<Mutex<Vec<Arc<HeldRelay>>>> = Arc::default();
            let list = relays.clone();
            let accept = tokio::spawn(async move {
                while let Ok((client, _)) = listener.accept().await {
                    let Ok(server) = TcpStream::connect(upstream).await else {
                        continue;
                    };
                    let relay = Arc::new(HeldRelay {
                        armed: AtomicBool::new(false),
                        release: Notify::new(),
                        server_ended: AtomicBool::new(false),
                    });
                    list.lock().unwrap().push(relay.clone());
                    tokio::spawn(hold_relay(client, server, relay));
                }
            });
            HoldProxy {
                addr,
                relays,
                accept,
            }
        }

        /// Arms the hold on every connection relayed now.
        pub fn hold(&self) {
            for r in self.relays.lock().unwrap().iter() {
                r.armed.store(true, Ordering::SeqCst);
            }
        }

        /// Whether a held connection's server side has ended (its last words are held).
        pub fn server_ended_while_held(&self) -> bool {
            self.relays
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.armed.load(Ordering::SeqCst) && r.server_ended.load(Ordering::SeqCst))
        }
    }

    impl Drop for HoldProxy {
        fn drop(&mut self) {
            self.accept.abort();
        }
    }

    async fn hold_relay(client: TcpStream, server: TcpStream, relay: Arc<HeldRelay>) {
        let (mut client_read, mut client_write) = client.into_split();
        let (mut server_read, mut server_write) = server.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Down>();
        let reader = relay.clone();
        // Server → queue.
        tokio::spawn(async move {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match server_read.read(&mut buf).await {
                    Ok(0) | Err(_) => {
                        if reader.armed.load(Ordering::SeqCst) {
                            reader.server_ended.store(true, Ordering::SeqCst);
                        }
                        let _ = tx.send(Down::End);
                        return;
                    }
                    Ok(n) => {
                        if tx.send(Down::Bytes(buf[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        // Queue → client, waiting for the release while armed.
        let writer = relay.clone();
        tokio::spawn(async move {
            while let Some(down) = rx.recv().await {
                if writer.armed.load(Ordering::SeqCst) {
                    writer.release.notified().await;
                }
                match down {
                    Down::Bytes(b) => {
                        if client_write.write_all(&b).await.is_err() {
                            return;
                        }
                    }
                    Down::End => {
                        let _ = client_write.shutdown().await;
                        return;
                    }
                }
            }
        });
        // Client → server; the client's first bytes while armed release the hold.
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match client_read.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let _ = server_write.shutdown().await;
                    return;
                }
                Ok(n) => {
                    // The server may be gone already: a failed write changes nothing.
                    let _ = server_write.write_all(&buf[..n]).await;
                    if relay.armed.swap(false, Ordering::SeqCst) {
                        relay.release.notify_one();
                    }
                }
            }
        }
    }
}
