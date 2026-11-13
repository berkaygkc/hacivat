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
pub fn deps_with_guards(order: Arc<Mutex<Vec<&'static str>>>, fake: FakeDriver) -> ConnectDeps {
    let mut deps = deps_with_fake(fake);
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
    deps
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
        transport: TransportSpec::SshTunnel,
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
        let Question::Password { attempt, .. } = q;
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
        profile: Arc::new(p),
        draft_password: None,
        purpose: Purpose::Main,
        department: Department::Console,
        prior_rejections: 0,
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
