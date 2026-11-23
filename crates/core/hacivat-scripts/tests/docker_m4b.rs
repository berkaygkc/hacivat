//! M4b Task 19: Docker conformance of context-from-location (gate E1) and the credential audit
//! (gate E6), with the library service, the binding, `hacivat_exec`'s `ScriptRunner` (the run
//! path, its gate and its confirmations) and real sessions of a `ConnectionManager` (M2b's
//! configuration), the way a script tab finds and runs its file:
//!
//! 1. the library is attached (its `.hacivat/workspace.toml` written by the service);
//! 2. a database node `(profile, database)` binds it through `hacivat_scripts::resolve`;
//! 3. the file is streamed through `LibraryService::open_stream` under the profile's
//!    environment (both passes, pass 2 checked against pass 1's record, R-T10-3);
//! 4. the runner sends it unit by unit on the tab's session, gated under the environment the
//!    profile carries (`ScriptGate.env`), its questions answered by a recording port.
//!
//! What only Docker proves: the run lands on the server and database of the connection the
//! file was opened under, and what the gate let through depends on that connection's
//! environment, both checked on the server through separate admin sessions (marker rows that
//! only the right server holds, its `current_database()`/`DATABASE()` and server version).
//!
//! - **E1** `same_library_runs_on_each_connection`: the same library under orders-prod and
//!   orders-staging on PostgreSQL (pg17 / pg16) and MySQL (mysql84 / mysql80); production asks
//!   (a "no" keeps every row; a "yes", recorded, lets the file run), staging does not ask.
//!   `sqlite_file_stem_binds` does the same on two SQLite files (no server). The pairs need the
//!   `full` set; a pair whose staging server is not in the active set is skipped with a line on
//!   stderr, never under `full`. `e1_gate_follows_the_profiles_environment_on_fakes` pins the
//!   premise without Docker.
//! - **MySQL's database level** (`mysql_schema_is_the_database_level`, and
//!   `mysql_tab_session_reopened_after_a_kill_is_switched_back` /
//!   `mysql_default_session_reconnect_replays_its_schema`): a script tab's own session switched
//!   to the library's database (R-M4b-27; the switch is the one `hacivat-app`'s `scripts::tabs`
//!   sends, which this crate cannot call), also when the server killed it and the app opened a
//!   new one, and what the default session's reconnect replays.
//! - **E6** `no_credentials_reach_the_library_on_real_servers` (Docker) and
//!   `no_credentials_reach_the_library_with_a_fake` (no Docker): a full scripted session, then
//!   every byte and every name under the library's parent folder and the trash is searched for
//!   the connections' host, port, user, password, passphrases, names, profile ids (in every
//!   spelling) and keyring service names. `.git/` is byte-identical before and after, the only
//!   paths are the user's scripts and `.hacivat/workspace.toml` (keys exactly `format`,
//!   `database`, `aliases`). `mapping_lifecycle_writes_no_profile_id_near_the_library` is the
//!   T19 reviewer's hostile test of the mapping lifecycle (adopted).
//!
//! Facts go to stdout through `fact!` (libtest captures only the `print!` family). Run with
//! `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-scripts --test
//! docker_m4b`. Server fixtures are `hvm4b_t19_*` tables, unique to the process and dropped by
//! a guard (also when a test fails). Every wait is on a condition with a ceiling.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hacivat_driver::{DriverRegistry, EngineCatalog, ExecOpts, ExecSummary};
use hacivat_exec::gate::CommitQuestion;
use hacivat_exec::{
    ClassifyPool, CommitAnswer, ConfirmAnswer, ConfirmPort, ConfirmQuestion, ExecService,
    ScriptGate, ScriptProgress, ScriptRunOutcome, ScriptRunRequest, ScriptRunner, SessionLog,
    SuppressSet, run_file,
};
use hacivat_net::TlsMode;
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_net::{BoxedStream, StreamFactory};
use hacivat_proto::{DbmsKind, Department, EnvTag, ProfileId};
use hacivat_results::MemoryBudget;
use hacivat_scripts::binding::logical_for_sqlite;
use hacivat_scripts::recovery::{self, CopyMeta, RecoveryKey};
use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
use hacivat_scripts::testing::{FakeHandle, logical};
use hacivat_scripts::{
    DatabaseName, DbNode, EntryKind, GitSummary, LibraryId, LibraryState, LocalMapping, ReadError,
    RelPath, SaveText, ScriptChunks as _, ScriptStream, TextEncoding, resolve,
};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, ExposeSecret as _, MemoryBackend, Retention, SecretBackend,
    SecretKey, SecretSlot, SecretStore, SecretString,
};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ClientId, ExecOrigin, Request, SessionEvent, SessionHandle, SessionKind, SessionSpec,
    SessionState, TxMode, parse_schema_probe, schema_probe_sql,
};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_testkit::{DockerTarget, TlsDefault, require_target};
use sql_dialect::ClientFlavor;
use tokio::sync::{oneshot, watch};

const CEILING: Duration = Duration::from_secs(30);

/// Prints one `HV-FACT …` line straight to the process's stdout, on one line: libtest captures
/// only the `print!` family, so the line reaches the CI log of a passing test too (R-W3-14,
/// the `hacivat-meta` tests' macro). Facts never hold a secret.
macro_rules! fact {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let line = format!($($arg)*).replace('\n', " ");
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }};
}

// ---------------------------------------------------------------------------------------------
// Support: names, waits, the library service

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).expect("a valid relative path")
}

/// A name unique to this process and call (`[a-z0-9_]`), prefixed `hvm4b_t19_`.
fn unique(slug: &str) -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "hvm4b_t19_{slug}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    )
}

/// Waits until `f` holds (5 ms steps); panics naming `what` after the ceiling.
async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + CEILING;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

/// A library service over app folders in `app`, fed by a fake event source (the scenario's
/// own calls refresh the listing), trashing into `trash` (outside the library).
struct Lib {
    svc: Arc<LibraryService>,
    paths: LocalPaths,
    _fake: FakeHandle,
}

fn service(app: &Path, trash: &Path) -> Lib {
    let paths = LocalPaths::under(app);
    let fake = FakeHandle::default();
    let svc = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        fake.sources(),
        ServiceOptions {
            settle: Duration::from_millis(20),
            git_debounce: Duration::from_millis(40),
            recheck: Duration::from_millis(60),
            home: None,
            trasher: Arc::new(FolderTrash(trash.to_path_buf())),
            ..ServiceOptions::default()
        },
    );
    Lib {
        svc: Arc::new(svc),
        paths,
        _fake: fake,
    }
}

impl Lib {
    /// Waits for library `id`'s scan and status.
    async fn ready(&self, id: LibraryId) {
        wait_until("the scan and status", || {
            self.svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
            })
        })
        .await;
    }

    /// Attaches `dir` bound to `db`, writing its `workspace.toml`, and waits for its first scan
    /// and status.
    async fn attach(&self, dir: &Path, db: &str) -> LibraryId {
        let id = self
            .svc
            .attach(dir.to_path_buf(), Some(logical(db)), true)
            .await
            .expect("the library attaches");
        self.ready(id).await;
        id
    }

    /// The libraries `node` binds, as the explorer resolves them.
    fn bound(&self, node: &DbNode) -> Vec<LibraryId> {
        let snap = self.svc.snapshot();
        let libs: Vec<_> = snap.libraries.iter().map(|l| (**l).clone()).collect();
        resolve(node, &libs, &snap.mappings)
    }

    /// Whether a connection whose environment is `env` lists `r` in library `id`.
    fn lists(&self, id: LibraryId, env: Option<EnvTag>, r: &str) -> bool {
        self.svc
            .snapshot()
            .view(env)
            .library(id)
            .is_some_and(|l| l.entry(&rel(r)).is_some())
    }

    /// The runner's source for `r` under `env`: pass 1 opened now.
    async fn source(&self, id: LibraryId, r: &str, env: Option<EnvTag>) -> Box<LibSource> {
        let stream = self
            .svc
            .open_stream(id, &rel(r), env, TextEncoding::UTF_8, None)
            .await
            .unwrap_or_else(|e| panic!("{r}: pass 1 opens: {e:?}"));
        Box::new(LibSource {
            svc: self.svc.clone(),
            id,
            rel: rel(r),
            env,
            stream,
        })
    }
}

/// A library file as "Run as script" reads it (T12's adapter sketch over T10's
/// `ScriptStream`/`PassRecord`): pass 2 reopens with pass 1's record, only after a complete
/// pass 1, and stops at the first block that differs.
struct LibSource {
    svc: Arc<LibraryService>,
    id: LibraryId,
    rel: RelPath,
    env: Option<EnvTag>,
    stream: ScriptStream,
}

fn chunk_error(e: hacivat_scripts::ChunkError) -> hacivat_exec::ChunkError {
    match e {
        hacivat_scripts::ChunkError::Changed => hacivat_exec::ChunkError::Changed,
        hacivat_scripts::ChunkError::Decode => hacivat_exec::ChunkError::Decode,
        hacivat_scripts::ChunkError::Io(k) => hacivat_exec::ChunkError::Io(k),
    }
}

fn read_error(e: ReadError) -> hacivat_exec::ChunkError {
    match e {
        ReadError::Changed => hacivat_exec::ChunkError::Changed,
        ReadError::Io(k) => hacivat_exec::ChunkError::Io(k),
        ReadError::NeedsEncoding { .. }
        | ReadError::Binary
        | ReadError::UnknownEncoding
        | ReadError::Lossy
        | ReadError::Undecodable => hacivat_exec::ChunkError::Decode,
        ReadError::NotImplemented | ReadError::PathRefused | ReadError::OutOfScope => {
            hacivat_exec::ChunkError::Io(std::io::ErrorKind::PermissionDenied)
        }
    }
}

type Reopened = Pin<
    Box<
        dyn Future<Output = Result<Box<dyn hacivat_exec::ScriptChunks>, hacivat_exec::ChunkError>>
            + Send,
    >,
>;

impl hacivat_exec::ScriptChunks for LibSource {
    fn next_chunk(&mut self) -> Option<Result<String, hacivat_exec::ChunkError>> {
        self.stream.next_chunk().map(|r| r.map_err(chunk_error))
    }

    fn reopen_verified(self: Box<Self>) -> Reopened {
        Box::pin(async move {
            let record = self.stream.record();
            if !record.is_complete() {
                return Err(hacivat_exec::ChunkError::Changed);
            }
            let stream = self
                .svc
                .open_stream(
                    self.id,
                    &self.rel,
                    self.env,
                    TextEncoding::UTF_8,
                    Some(record),
                )
                .await
                .map_err(read_error)?;
            Ok(Box::new(LibSource { stream, ..*self }) as Box<dyn hacivat_exec::ScriptChunks>)
        })
    }
}

/// A library folder named `orders-scripts` (the folder name is not the database: the binding
/// comes from `workspace.toml`) holding `files`.
fn library_dir(parent: &Path, files: &[(&str, &str)]) -> PathBuf {
    let dir = parent.join("orders-scripts");
    std::fs::create_dir_all(&dir).expect("the library folder");
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("a folder");
        std::fs::write(&p, text).expect("a script");
    }
    dir
}

// ---------------------------------------------------------------------------------------------
// Support: the runner and its questions

/// A port that answers every confirmation `answer` and every commit question `commit`,
/// recording each question (the dialog the app would show).
#[derive(Debug)]
struct Port {
    answer: ConfirmAnswer,
    commit: CommitAnswer,
    asked: Mutex<Vec<ConfirmQuestion>>,
    commits: Mutex<Vec<CommitQuestion>>,
}

impl Port {
    /// The user says yes to everything (one statement at a time: production never takes
    /// Execute all).
    fn yes() -> Arc<Port> {
        Port::new(ConfirmAnswer::Execute, CommitAnswer::Commit)
    }

    /// The user says no (Cancel; a commit question rolls back).
    fn no() -> Arc<Port> {
        Port::new(ConfirmAnswer::Cancel, CommitAnswer::RollBack)
    }

    fn new(answer: ConfirmAnswer, commit: CommitAnswer) -> Arc<Port> {
        Arc::new(Port {
            answer,
            commit,
            asked: Mutex::default(),
            commits: Mutex::default(),
        })
    }

    /// How many questions the run asked (confirmations and commit questions).
    fn questions(&self) -> usize {
        self.asked.lock().unwrap().len() + self.commits.lock().unwrap().len()
    }

    /// Whether every confirmation said "production" (the gate's context, ConfirmQuestion).
    fn all_production(&self) -> bool {
        self.asked.lock().unwrap().iter().all(|q| q.production)
    }
}

impl ConfirmPort for Port {
    fn confirm(&self, q: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer> {
        self.asked.lock().unwrap().push(q);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(self.answer);
        rx
    }

    fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer> {
        self.commits.lock().unwrap().push(q);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(self.commit);
        rx
    }
}

/// A runner as the app builds one: the run service and a classifier pool.
fn runner() -> ScriptRunner {
    ScriptRunner::new(
        Arc::new(ExecService::new(MemoryBudget::new(1 << 30, 256 << 20))),
        ClassifyPool::new(2),
    )
}

fn flavor(engine: DbmsKind) -> ClientFlavor {
    match engine {
        DbmsKind::Postgres => ClientFlavor::Psql,
        DbmsKind::MySql | DbmsKind::MariaDb => ClientFlavor::MySqlCli,
        _ => ClientFlavor::None,
    }
}

/// Runs `source` as script on `s`, the session of `profile`, gated under the environment the
/// profile carries; the outcome and the last progress.
async fn run_script(
    s: &SessionHandle,
    profile: &Profile,
    source: Box<dyn hacivat_exec::ScriptChunks>,
    port: Arc<Port>,
) -> (ScriptRunOutcome, ScriptProgress) {
    let (env, _) = watch::channel(profile.environment);
    let (cancel, _) = watch::channel(false);
    let req = ScriptRunRequest {
        session: s.clone(),
        source,
        console: 19,
        profile: profile.id,
        engine: profile.engine,
        dialect: hacivat_exec::sqlctx::dialect_ctx(profile.engine, None, None),
        modes_known: false,
        flavor: flavor(profile.engine),
        gate: ScriptGate {
            env: env.subscribe(),
            confirm_unsafe_where: true,
            allowlist: Arc::from(Vec::new()),
            suppressed: SuppressSet::default(),
            connection: Arc::from(profile.name.as_str()),
        },
        confirm: port,
        log: SessionLog::new(),
        cancel: cancel.subscribe(),
        history: None,
        refresh: None,
    };
    let (tx, rx) = watch::channel(ScriptProgress::default());
    let out = run_file(&runner(), req, tx).await;
    let last = rx.borrow().clone();
    (out, last)
}

fn finished(out: &ScriptRunOutcome) -> bool {
    matches!(out, ScriptRunOutcome::Finished(_))
}

// ---------------------------------------------------------------------------------------------
// Support: connections

/// Connect dependencies over `drivers`; `seeds` go into the keyring first (the secrets a
/// session of the app holds). `wrap`: a fake answers in memory, no socket is opened.
fn deps(drivers: DriverRegistry, wrap: bool, seeds: &[(SecretKey, &str)]) -> ConnectDeps {
    let backend = MemoryBackend::new();
    for (key, value) in seeds {
        backend
            .put(key, &SecretString::from(value.to_string()))
            .expect("a seeded secret");
    }
    let backend: Arc<dyn SecretBackend> = Arc::new(backend);
    let known_hosts = KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    ConnectDeps {
        drivers: Arc::new(drivers),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: wrap.then(|| {
            Arc::new(|_direct: StreamFactory| {
                StreamFactory::new(|| {
                    Box::pin(async {
                        let (near, _far) = tokio::io::duplex(64);
                        Ok(Box::new(near) as BoxedStream)
                    })
                })
            }) as _
        }),
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

/// A connected profile: its manager, the profile, its default session, its keyring.
struct Conn {
    m: Arc<ConnectionManager>,
    profile: Arc<Profile>,
    s: SessionHandle,
    secrets: Arc<SecretStore>,
}

/// Connects `profile` (with `password`) on a manager of its own over `deps`.
async fn connect(deps: ConnectDeps, profile: Profile, password: Option<&str>) -> Conn {
    let secrets = deps.secrets.clone();
    let m = ConnectionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(deps),
        ManagerConfig::m2b(),
    );
    let profile = Arc::new(profile);
    m.connect(
        profile.clone(),
        password.map(|p| SecretString::from(p.to_string())),
        Interaction::Headless,
    );
    let mut changes = m.changes();
    let deadline = tokio::time::Instant::now() + CEILING;
    loop {
        match m.state(profile.id) {
            LiveState::Connected(_) => break,
            LiveState::Failed(e) => panic!("{}: the connect failed: {:?}", profile.name, e.class),
            _ => {}
        }
        tokio::time::timeout_at(deadline, changes.changed())
            .await
            .unwrap_or_else(|_| panic!("{}: the connect timed out", profile.name))
            .expect("the manager lives");
    }
    let s = m
        .sessions(profile.id)
        .expect("connected")
        .default_session()
        .expect("a default session");
    Conn {
        m,
        profile,
        s,
        secrets,
    }
}

/// The profiles' transaction mode: autocommit, so what a run wrote is committed when its reply
/// comes and the side channel sees it (production and staging default to manual transactions,
/// `session::default_mode`, where the row would wait for the user's Commit).
const AUTOCOMMIT: Option<TxMode> = Some(TxMode::Auto);

/// The profile `name` of server target `t`, on `database`, tagged `env`.
fn server_profile(t: &DockerTarget, name: &str, env: Option<EnvTag>, database: &str) -> Profile {
    let mut p = Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        engine: t.engine,
        environment: env,
        endpoint: ProfileEndpoint::Server {
            host: t.host.clone(),
            port: Some(t.port),
        },
        database: Some(database.to_string()),
        user: Some(t.user.clone()),
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        allow_unsupported_server: true,
        tx_mode: AUTOCOMMIT,
        ..Profile::default()
    };
    p.tls.mode = match t.tls_default {
        TlsDefault::SelfSigned => TlsMode::Require,
        TlsDefault::None => TlsMode::Auto,
    };
    p
}

/// Connects `profile` of server target `t`, its keyring seeded with `seeds`.
async fn on_server_as(t: &DockerTarget, profile: Profile, seeds: &[(SecretKey, &str)]) -> Conn {
    connect(
        deps(default_drivers(), false, seeds),
        profile,
        Some(&t.password),
    )
    .await
}

/// Connects to `t` as `name`, on `database`.
async fn on_server(t: &DockerTarget, name: &str, env: Option<EnvTag>, database: &str) -> Conn {
    on_server_as(t, server_profile(t, name, env, database), &[]).await
}

/// A SQLite profile `name` on `file`.
fn sqlite_profile(file: &Path, name: &str, env: Option<EnvTag>) -> Profile {
    Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        engine: DbmsKind::Sqlite,
        environment: env,
        endpoint: ProfileEndpoint::File {
            path: file.to_path_buf(),
            create_if_missing: true,
        },
        user: None,
        auth: AuthProviderId::None,
        max_connections: 1,
        tx_mode: AUTOCOMMIT,
        ..Profile::default()
    }
}

/// `sql` on `s` as the user's statement; the reply.
async fn exec(s: &SessionHandle, sql: &str) -> Result<ExecSummary, String> {
    s.submit(Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::values(100),
        origin: ExecOrigin::User,
    })
    .reply
    .await
    .expect("the actor replies")
    .map_err(|e| format!("{e:?}"))
}

/// `sql`'s first row on `s`; panics naming `who` when it fails.
async fn row(s: &SessionHandle, who: &str, sql: &str) -> Vec<String> {
    let answer = exec(s, sql)
        .await
        .unwrap_or_else(|e| panic!("{who}: `{sql}`: {e}"));
    answer
        .rows
        .first()
        .map(|r| r.iter().map(|c| c.clone().unwrap_or_default()).collect())
        .unwrap_or_default()
}

/// `sql`'s single integer on `s`.
async fn count(s: &SessionHandle, who: &str, sql: &str) -> i64 {
    let v = row(s, who, sql)
        .await
        .into_iter()
        .next()
        .unwrap_or_default();
    v.parse()
        .unwrap_or_else(|_| panic!("{who}: `{sql}`: `{v}` is not an integer"))
}

/// The database `s` is on now, asked of the server (the tab's probe, R-T14-2).
async fn current_database(s: &SessionHandle, engine: DbmsKind) -> Option<String> {
    let answer = exec(s, schema_probe_sql(engine)).await.ok()?;
    parse_schema_probe(engine, &answer)
        .database
        .filter(|d| !d.is_empty())
}

/// The switch a MySQL script tab sends on its own session when the session is not on the
/// tab's database (R-M4b-27; `hacivat-app::scripts::tabs::run_gate`, which this crate cannot
/// call): one `SwitchSchema`, then the probe again.
async fn switch_back(s: &SessionHandle, engine: DbmsKind, bound: &str) -> Option<String> {
    let switched = s
        .submit(Request::SwitchSchema(bound.to_string()))
        .reply
        .await
        .expect("the actor replies");
    assert!(switched.is_ok(), "switching to {bound}: {switched:?}");
    current_database(s, engine).await
}

/// A script tab's own session on `c`'s profile (`SessionKind::Console(ClientId(n))`, n far
/// above any console's, as `hacivat-app::scripts::tabs::client_number`).
async fn own_session(c: &Conn, n: u64) -> SessionHandle {
    c.m.sessions(c.profile.id)
        .expect("connected")
        .open(
            SessionSpec {
                kind: SessionKind::Console(ClientId((1 << 30) + n)),
                read_only: false,
                department: Department::Console,
            },
            Interaction::Headless,
        )
        .await
        .unwrap_or_else(|e| panic!("{}: a script session: {:?}", c.profile.name, e.class))
}

/// Server fixtures dropped when the guard goes (also on a failed assertion): the drops run on
/// a runtime of their own on another thread, so they work while unwinding. A drop that fails
/// is said in a fact (review m8).
struct Fixtures {
    drops: Vec<(DockerTarget, String, String)>,
}

impl Fixtures {
    fn new() -> Fixtures {
        Fixtures { drops: Vec::new() }
    }

    /// Creates `table` (columns `cols`) in `database` of `t` through `admin`, to be dropped.
    async fn table(
        &mut self,
        t: &DockerTarget,
        admin: &Conn,
        database: &str,
        table: &str,
        cols: &str,
    ) {
        let ddl = format!("CREATE TABLE {table} ({cols})");
        exec(&admin.s, &ddl)
            .await
            .unwrap_or_else(|e| panic!("{}: `{ddl}`: {e}", t.id));
        self.drops.push((
            t.clone(),
            database.to_string(),
            format!("DROP TABLE IF EXISTS {table}"),
        ));
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        let drops = std::mem::take(&mut self.drops);
        if drops.is_empty() {
            return;
        }
        let joined = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the drops");
            rt.block_on(async {
                for (t, db, sql) in drops {
                    let admin = on_server(&t, "hvm4b-t19-drop", None, &db).await;
                    if let Err(e) = exec(&admin.s, &sql).await {
                        fact!(
                            "HV-FACT t19 fixture_drop_failed target={} sql={sql} error={e}",
                            t.id
                        );
                    }
                    admin.m.disconnect(admin.profile.id);
                }
            });
        })
        .join();
        if joined.is_err() {
            fact!("HV-FACT t19 fixture_drop_failed reason=drop-thread-panicked");
        }
    }
}

/// The target `id` when the active set has it and Docker tests run.
fn docker_target(id: &str) -> Option<DockerTarget> {
    if !hacivat_testkit::docker_enabled() {
        return None;
    }
    require_target(id)
}

/// Whether the active set is `full` (or every target): then every pair must run.
fn full_set() -> bool {
    std::env::var("HACIVAT_DOCKER_SET")
        .ok()
        .is_none_or(|s| s == "full")
}

// ---------------------------------------------------------------------------------------------
// E1: the same library under orders-prod and orders-staging

/// `mark.sql` on `engine`: a row naming where it ran (database, server version), then a write
/// the gate confirms on production only (a bounded DELETE of the seed row).
fn mark_sql(engine: DbmsKind, marker: &str) -> String {
    let (db, ver) = match engine {
        DbmsKind::Postgres => (
            "current_database()",
            "current_setting('server_version_num')",
        ),
        DbmsKind::Sqlite => ("'main'", "sqlite_version()"),
        _ => ("DATABASE()", "VERSION()"),
    };
    format!(
        "INSERT INTO {marker} (who, db, ver) VALUES ('mark.sql', {db}, {ver});\n\
         DELETE FROM {marker} WHERE who = 'seed';\n"
    )
}

/// What a marker table holds: (rows by `mark.sql`, seed rows left, the database and version a
/// `mark.sql` row recorded).
async fn marks(admin: &Conn, marker: &str) -> (i64, i64, Option<(String, String)>) {
    let who = &admin.profile.name;
    let ran = count(
        &admin.s,
        who,
        &format!("SELECT COUNT(*) FROM {marker} WHERE who = 'mark.sql'"),
    )
    .await;
    let seed = count(
        &admin.s,
        who,
        &format!("SELECT COUNT(*) FROM {marker} WHERE who = 'seed'"),
    )
    .await;
    let at = if ran > 0 {
        let r = row(
            &admin.s,
            who,
            &format!("SELECT db, ver FROM {marker} WHERE who = 'mark.sql'"),
        )
        .await;
        Some((r[0].clone(), r[1].clone()))
    } else {
        None
    };
    (ran, seed, at)
}

/// Whether server version text `v` (PostgreSQL's `server_version_num`, MySQL's `VERSION()`) is
/// of major `major`.
fn of_major(engine: DbmsKind, major: &str, v: &str) -> bool {
    match engine {
        DbmsKind::Postgres => v.len() == 6 && v.starts_with(major),
        _ => v.starts_with(&format!("{major}.")),
    }
}

fn env_name(env: Option<EnvTag>) -> &'static str {
    match env {
        Some(EnvTag::Production) => "production",
        Some(EnvTag::Staging) => "staging",
        Some(_) => "other",
        None => "none",
    }
}

/// One side of a pair: the connection a tab of `mark.sql` was opened under, and the admin
/// session of its server or file.
struct Side {
    c: Conn,
    admin: Conn,
    /// The session the tab runs on (PostgreSQL and SQLite: the profile's; MySQL: one of its
    /// own, R-T14-3).
    s: SessionHandle,
    target: String,
    major: Option<String>,
}

/// Every side's marker table now.
async fn snapshot(sides: &[Side], marker: &str) -> Vec<(i64, i64, Option<(String, String)>)> {
    let mut v = Vec::new();
    for s in sides {
        v.push(marks(&s.admin, marker).await);
    }
    v
}

/// E1 over `sides` (prod first, then staging), all showing library `id` of `lib`:
/// - every node binds the library (by name, or by file stem for SQLite);
/// - on production the gate asks: a "no" leaves every server's rows as they were; a recorded
///   "yes" lets `mark.sql` run there, and only there;
/// - on staging nothing is asked, and the run lands there only.
async fn e1_runs(lib: &Lib, id: LibraryId, sides: &[Side], marker: &str, node_db: &str) {
    for (k, side) in sides.iter().enumerate() {
        let p = &side.c.profile;
        let name = &p.name;
        let logical_db = match &p.endpoint {
            ProfileEndpoint::File { path, .. } => logical_for_sqlite(path).expect("a stem"),
            _ => logical(node_db),
        };
        assert_eq!(
            logical_db.as_str(),
            "orders",
            "{name}: the node's logical name"
        );
        let node = DbNode {
            profile: p.id,
            database: DatabaseName::new(node_db),
            logical: logical_db,
        };
        assert_eq!(lib.bound(&node), [id], "{name}: the library binds");
        // The `@production/` file is listed only under production (R-M4b-6).
        let prod_only = lib.lists(id, p.environment, "@production/only_prod.sql");
        assert_eq!(
            prod_only,
            p.environment == Some(EnvTag::Production),
            "{name}"
        );
        let before = snapshot(sides, marker).await;
        let production = p.environment == Some(EnvTag::Production);
        let mut said_no = None;
        if production {
            // The user says no: nothing changes anywhere.
            let port = Port::no();
            let src = lib.source(id, "mark.sql", p.environment).await;
            let (out, _) = run_script(&side.s, p, src, port.clone()).await;
            assert!(
                !finished(&out),
                "{name}: a \"no\" on production let the file finish"
            );
            assert!(port.questions() > 0, "{name}: production did not ask");
            assert!(
                port.all_production(),
                "{name}: a question without production"
            );
            assert_eq!(
                snapshot(sides, marker).await,
                before,
                "{name}: a \"no\" changed rows"
            );
            said_no = Some(port.questions());
        }
        let port = Port::yes();
        let src = lib.source(id, "mark.sql", p.environment).await;
        let (out, progress) = run_script(&side.s, p, src, port.clone()).await;
        assert!(finished(&out), "{name}: {out:?}");
        let asked = port.questions();
        if production {
            assert!(
                asked >= 2,
                "{name}: production asked {asked} times for two writes"
            );
            assert!(
                port.all_production(),
                "{name}: a question without production"
            );
        } else {
            assert_eq!(asked, 0, "{name}: off production nothing is asked");
        }
        let after = snapshot(sides, marker).await;
        for (j, (b, a)) in before.iter().zip(&after).enumerate() {
            if j == k {
                assert_eq!(
                    (a.0, a.1),
                    (b.0 + 1, 0),
                    "{name}: its own server ran mark.sql"
                );
            } else {
                assert_eq!(a, b, "{name}: another server changed");
            }
        }
        let (db, ver) = after[k].2.clone().expect("a mark.sql row");
        assert_eq!(
            db,
            if node_db == "main" { "main" } else { "orders" },
            "{name}"
        );
        if let Some(major) = &side.major {
            assert!(
                of_major(p.engine, major, &ver),
                "{name}: {ver} is not {major}"
            );
        }
        fact!(
            "HV-FACT t19 e1 profile={name} target={} database={db} server_version_num={ver} \
             env={} asked={asked} no_asked={} units_run={} marker_prod={} marker_staging={} \
             production_file_listed={prod_only}",
            side.target,
            env_name(p.environment),
            said_no.map_or("-".to_string(), |n| n.to_string()),
            progress.units_run,
            after[0].0,
            after.get(1).map_or(0, |m| m.0),
        );
    }
}

async fn e1_pair(engine: DbmsKind, prod_id: &str, staging_id: &str) {
    let (Some(prod), Some(staging)) = (docker_target(prod_id), docker_target(staging_id)) else {
        if hacivat_testkit::docker_enabled() {
            assert!(
                !full_set(),
                "the full set has no {prod_id} or {staging_id} for E1"
            );
            eprintln!("skip e1 {prod_id}/{staging_id}: not both in the active set");
        }
        return;
    };
    let marker = unique("e1_mark");
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(
        parent.path(),
        &[
            ("mark.sql", &mark_sql(engine, &marker)),
            ("@production/only_prod.sql", "SELECT 1"),
        ],
    );
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;

    let mut fixtures = Fixtures::new();
    let mut sides = Vec::new();
    for (k, (t, name, env)) in [
        (&prod, "orders-prod", EnvTag::Production),
        (&staging, "orders-staging", EnvTag::Staging),
    ]
    .into_iter()
    .enumerate()
    {
        let admin = on_server(t, "hvm4b-t19-admin", None, "orders").await;
        fixtures
            .table(
                t,
                &admin,
                "orders",
                &marker,
                "who VARCHAR(32), db VARCHAR(64), ver VARCHAR(64)",
            )
            .await;
        exec(
            &admin.s,
            &format!("INSERT INTO {marker} (who) VALUES ('seed')"),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: the seed row: {e}", t.id));
        let c = on_server(t, name, Some(env), "orders").await;
        let s = if engine == DbmsKind::Postgres {
            c.s.clone()
        } else {
            let s = own_session(&c, k as u64).await;
            // A new own session starts on the profile's database, the tab's here.
            assert_eq!(
                current_database(&s, engine).await.as_deref(),
                Some("orders")
            );
            s
        };
        sides.push(Side {
            c,
            admin,
            s,
            target: t.id.clone(),
            major: Some(t.major.clone()),
        });
    }
    e1_runs(&lib, id, &sides, &marker, "orders").await;
    for side in &sides {
        side.c.m.disconnect(side.c.profile.id);
        side.admin.m.disconnect(side.admin.profile.id);
    }
    lib.svc.shutdown();
    drop(fixtures);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn same_library_runs_on_each_connection() {
    e1_pair(DbmsKind::Postgres, "pg17", "pg16").await;
    e1_pair(DbmsKind::MySql, "mysql84", "mysql80").await;
}

/// SQLite (no server: runs in every set, also without Docker): a database's logical name is its
/// file's stem (R-M4b-32), so `prod/orders.db` and `staging/orders.db` both bind library
/// `orders`, each run lands in its own file, and production asks.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn sqlite_file_stem_binds() {
    let files = tempdir();
    let marker = "hvm4b_t19_e1_mark";
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(
        parent.path(),
        &[
            ("mark.sql", &mark_sql(DbmsKind::Sqlite, marker)),
            ("@production/only_prod.sql", "SELECT 1"),
        ],
    );
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;
    let mut sides = Vec::new();
    for (folder, name, env) in [
        ("prod", "orders-prod", EnvTag::Production),
        ("staging", "orders-staging", EnvTag::Staging),
    ] {
        let p = files.path().join(folder).join("orders.db");
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("a folder");
        let admin = connect(
            deps(default_drivers(), false, &[]),
            sqlite_profile(&p, "hvm4b-t19-admin", None),
            None,
        )
        .await;
        exec(
            &admin.s,
            &format!("CREATE TABLE {marker} (who TEXT, db TEXT, ver TEXT)"),
        )
        .await
        .expect("the marker table");
        exec(
            &admin.s,
            &format!("INSERT INTO {marker} (who) VALUES ('seed')"),
        )
        .await
        .expect("the seed row");
        let c = connect(
            deps(default_drivers(), false, &[]),
            sqlite_profile(&p, name, Some(env)),
            None,
        )
        .await;
        sides.push(Side {
            s: c.s.clone(),
            c,
            admin,
            target: "sqlite".into(),
            major: None,
        });
    }
    e1_runs(&lib, id, &sides, marker, "main").await;
    for side in &sides {
        side.c.m.disconnect(side.c.profile.id);
        side.admin.m.disconnect(side.admin.profile.id);
    }
    lib.svc.shutdown();
}

/// The E1 premise without Docker: the same file run through the runner on fakes, under each
/// profile's environment: production asks (every question says production), staging and an
/// untagged profile do not. On PostgreSQL and MySQL.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn e1_gate_follows_the_profiles_environment_on_fakes() {
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(
        parent.path(),
        &[("mark.sql", &mark_sql(DbmsKind::MySql, "hvm4b_t19_mark"))],
    );
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;
    for engine in [DbmsKind::Postgres, DbmsKind::MySql] {
        for env in [Some(EnvTag::Production), Some(EnvTag::Staging), None] {
            let fake = hacivat_driver::fake::FakeDriver::new(engine);
            let fake = if engine == DbmsKind::Postgres {
                fake.tracks_transactions()
            } else {
                fake
            };
            let profile = Profile {
                environment: env,
                tx_mode: AUTOCOMMIT,
                ..fake_profile("orders", EnvTag::Dev, engine)
            };
            let mut drivers = DriverRegistry::new();
            drivers.register(Arc::new(fake));
            let c = connect(deps(drivers, true, &[]), profile, Some("hv_test_pw")).await;
            let port = Port::yes();
            let src = lib.source(id, "mark.sql", env).await;
            let (out, _) = run_script(&c.s, &c.profile, src, port.clone()).await;
            assert!(finished(&out), "{engine:?} {env:?}: {out:?}");
            if env == Some(EnvTag::Production) {
                assert!(
                    port.questions() >= 2,
                    "{engine:?}: production asked {}",
                    port.questions()
                );
                assert!(port.all_production());
            } else {
                assert_eq!(port.questions(), 0, "{engine:?} {env:?}: asked");
            }
            c.m.disconnect(c.profile.id);
        }
    }
    lib.svc.shutdown();
}

// ---------------------------------------------------------------------------------------------
// MySQL: the schema is the database level

/// A MySQL pair of marker tables (`orders.<t>`, `orders_archive.<t>`) and a library bound to
/// `orders_archive`; the profile's own database is `orders`.
struct MysqlRig {
    _dirs: [tempfile::TempDir; 3],
    lib: Lib,
    id: LibraryId,
    c: Conn,
    admin: Conn,
    marker: String,
    _fixtures: Fixtures,
}

async fn mysql_rig(t: &DockerTarget) -> MysqlRig {
    let marker = unique("mark");
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(
        parent.path(),
        &[(
            "mark.sql",
            &format!("INSERT INTO {marker} (who, db) VALUES ('mark.sql', DATABASE());\n"),
        )],
    );
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders_archive").await;
    let mut fixtures = Fixtures::new();
    let admin = on_server(t, "hvm4b-t19-admin", None, "orders").await;
    for db in ["orders", "orders_archive"] {
        let a = on_server(t, "hvm4b-t19-admin", None, db).await;
        fixtures
            .table(t, &a, db, &marker, "who VARCHAR(32), db VARCHAR(64)")
            .await;
        a.m.disconnect(a.profile.id);
    }
    let c = on_server(t, "orders-staging", Some(EnvTag::Staging), "orders").await;
    MysqlRig {
        _dirs: [app, parent, trash],
        lib,
        id,
        c,
        admin,
        marker,
        _fixtures: fixtures,
    }
}

impl MysqlRig {
    /// The marker rows in `orders` and in `orders_archive`.
    async fn marks(&self) -> (i64, i64) {
        let q = |db: &str| {
            format!(
                "SELECT COUNT(*) FROM {db}.{} WHERE who = 'mark.sql'",
                self.marker
            )
        };
        (
            count(&self.admin.s, "admin", &q("orders")).await,
            count(&self.admin.s, "admin", &q("orders_archive")).await,
        )
    }

    /// `mark.sql` run as script on `s` (staging: nothing is asked).
    async fn run_mark(&self, s: &SessionHandle) {
        let port = Port::no();
        let src = self
            .lib
            .source(self.id, "mark.sql", Some(EnvTag::Staging))
            .await;
        let (out, _) = run_script(s, &self.c.profile, src, port.clone()).await;
        assert!(finished(&out), "mark.sql: {out:?}");
        assert_eq!(port.questions(), 0);
    }

    fn done(&self) {
        self.c.m.disconnect(self.c.profile.id);
        self.admin.m.disconnect(self.admin.profile.id);
        self.lib.svc.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_schema_is_the_database_level() {
    let Some(t) = docker_target("mysql84") else {
        return;
    };
    let r = mysql_rig(&t).await;
    let node = |db: &str| DbNode {
        profile: r.c.profile.id,
        database: DatabaseName::new(db),
        logical: logical(db),
    };
    assert_eq!(r.lib.bound(&node("orders_archive")), [r.id]);
    assert!(
        r.lib.bound(&node("orders")).is_empty(),
        "the profile's own database binds nothing"
    );
    // The script tab's own session, never the default one.
    let script = own_session(&r.c, 1).await;
    let was = current_database(&script, t.engine).await;
    let now = switch_back(&script, t.engine, "orders_archive").await;
    let default = current_database(&r.c.s, t.engine).await;
    fact!(
        "HV-FACT t19 mysql_switch target={} default={} script_before={} script={}",
        t.id,
        default.as_deref().unwrap_or("?"),
        was.as_deref().unwrap_or("?"),
        now.as_deref().unwrap_or("?")
    );
    assert_eq!(
        was.as_deref(),
        Some("orders"),
        "a new session starts on the profile's database"
    );
    assert_eq!(now.as_deref(), Some("orders_archive"));
    assert_eq!(
        default.as_deref(),
        Some("orders"),
        "the default session was not switched"
    );
    r.run_mark(&script).await;
    assert_eq!(
        r.marks().await,
        (0, 1),
        "the run landed in orders_archive only"
    );
    r.done();
}

/// Waits until `s` has ended (the server killed its connection): an own session is not
/// reconnected in place (`SessionPool::register`: only the default session's reconnects), so
/// it goes `Broken` or `Disconnected`.
async fn wait_ended(s: &SessionHandle) -> SessionState {
    let mut state = s.state();
    let deadline = tokio::time::Instant::now() + CEILING;
    loop {
        let now = state.borrow().state.clone();
        if matches!(now, SessionState::Broken(_) | SessionState::Disconnected) {
            return now;
        }
        // The next request is what finds the connection gone.
        let _ = s.submit(Request::Ping).reply.await;
        if matches!(
            state.borrow().state,
            SessionState::Broken(_) | SessionState::Disconnected
        ) {
            continue;
        }
        if tokio::time::timeout_at(deadline, state.changed())
            .await
            .is_err()
        {
            panic!("timed out waiting for the killed session to end: {now:?}");
        }
    }
}

/// C1 (review), the app's path: a script tab's own session is killed by the server; it ends
/// (never reconnected in place); the app opens a new own session for the tab's slot
/// (`console/view.rs`); that session starts on the profile's database (`orders`), the tab's
/// check switches it, and the run lands in `orders_archive`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_tab_session_reopened_after_a_kill_is_switched_back() {
    let Some(t) = docker_target("mysql84") else {
        return;
    };
    let r = mysql_rig(&t).await;
    let script = own_session(&r.c, 2).await;
    assert_eq!(
        switch_back(&script, t.engine, "orders_archive")
            .await
            .as_deref(),
        Some("orders_archive")
    );
    let backend = row(&script, "script", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    exec(&r.c.s, &format!("KILL {backend}"))
        .await
        .unwrap_or_else(|e| panic!("KILL {backend}: {e}"));
    let ended = wait_ended(&script).await;
    // The app's re-open for the same slot.
    let reopened = own_session(&r.c, 2).await;
    let new_backend = row(&reopened, "script", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    let starts_on = current_database(&reopened, t.engine).await;
    let after_check = switch_back(&reopened, t.engine, "orders_archive").await;
    fact!(
        "HV-FACT t19 mysql_reopen target={} killed_state={ended:?} starts_on={} after_check={} new_connection={}",
        t.id,
        starts_on.as_deref().unwrap_or("?"),
        after_check.as_deref().unwrap_or("?"),
        new_backend != backend
    );
    assert_ne!(new_backend, backend, "a new connection after the kill");
    assert_eq!(
        starts_on.as_deref(),
        Some("orders"),
        "the re-opened session starts on the profile's database, so the check is what moves it"
    );
    assert_eq!(after_check.as_deref(), Some("orders_archive"));
    r.run_mark(&reopened).await;
    assert_eq!(
        r.marks().await,
        (0, 1),
        "the run landed in orders_archive only"
    );
    r.done();
}

/// C1's variant: the **default** session is killed. It reconnects in place (`Reconnected`)
/// and its replay restores the last schema (`session/actor.rs`: "identity, read-only setup,
/// last schema"), so a tab's check after it finds the database already right and switches
/// nothing; the fact records which of the two put it there.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_default_session_reconnect_replays_its_schema() {
    let Some(t) = docker_target("mysql84") else {
        return;
    };
    let r = mysql_rig(&t).await;
    let default = r.c.s.clone();
    assert_eq!(
        switch_back(&default, t.engine, "orders_archive")
            .await
            .as_deref(),
        Some("orders_archive")
    );
    let mut events = default.events();
    let backend = row(&default, "default", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    exec(&r.admin.s, &format!("KILL {backend}"))
        .await
        .unwrap_or_else(|e| panic!("KILL {backend}: {e}"));
    let _ = default.submit(Request::Ping).reply.await;
    let deadline = tokio::time::Instant::now() + CEILING;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(SessionEvent::Reconnected)) => break,
            Ok(Ok(_)) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("the session's events ended: {e}"),
            Err(_) => panic!("timed out waiting for Reconnected"),
        }
    }
    let new_backend = row(&default, "default", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    let after_replay = current_database(&default, t.engine).await;
    // The tab's check: switches only when the probe reads another database.
    let switched = after_replay.as_deref() != Some("orders_archive");
    let after_check = if switched {
        switch_back(&default, t.engine, "orders_archive").await
    } else {
        after_replay.clone()
    };
    fact!(
        "HV-FACT t19 mysql_reconnect target={} after_replay={} check_switched={switched} after_check={} new_connection={}",
        t.id,
        after_replay.as_deref().unwrap_or("?"),
        after_check.as_deref().unwrap_or("?"),
        new_backend != backend
    );
    assert_ne!(new_backend, backend, "a new connection after the kill");
    assert_eq!(
        after_replay.as_deref(),
        Some("orders_archive"),
        "the replay restored the last schema"
    );
    assert!(!switched, "the check had nothing to switch");
    r.run_mark(&default).await;
    assert_eq!(
        r.marks().await,
        (0, 1),
        "the run landed in orders_archive only"
    );
    r.done();
}

// ---------------------------------------------------------------------------------------------
// E6: no credentials ever reach a library folder

const PASSPHRASE: &str = "s3cret";

/// Every spelling of a profile id (review m2: the uppercase simple form too).
fn id_spellings(id: ProfileId) -> Vec<String> {
    let u = id.0;
    vec![
        u.to_string(),
        u.simple().to_string(),
        u.to_string().to_ascii_uppercase(),
        u.simple().to_string().to_ascii_uppercase(),
        format!("{id:?}"),
    ]
}

/// What must never appear near a library: every spelling of each connection's identifiers
/// and credentials.
fn needles(profiles: &[(&Profile, &str)]) -> Vec<String> {
    let mut out: Vec<String> = vec!["password =".into(), "host =".into(), PASSPHRASE.into()];
    for (p, password) in profiles {
        out.push(password.to_string());
        out.push(p.name.clone());
        out.extend(id_spellings(p.id));
        for slot in [
            SecretSlot::DbPassword,
            SecretSlot::TlsKeyPassphrase,
            SecretSlot::SshPassphrase { hop: 0 },
        ] {
            out.push(
                SecretKey {
                    profile: p.id,
                    slot,
                }
                .service(),
            );
        }
        if let Some(user) = &p.user {
            out.push(user.clone());
        }
        if let ProfileEndpoint::Server { host, port } = &p.endpoint {
            out.push(host.clone());
            if let Some(port) = port {
                out.push(port.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The passphrases a profile's keyring holds (review m2): its TLS key and its first SSH hop.
fn passphrase_seeds(id: ProfileId) -> Vec<(SecretKey, &'static str)> {
    vec![
        (
            SecretKey {
                profile: id,
                slot: SecretSlot::TlsKeyPassphrase,
            },
            PASSPHRASE,
        ),
        (
            SecretKey {
                profile: id,
                slot: SecretSlot::SshPassphrase { hop: 0 },
            },
            PASSPHRASE,
        ),
    ]
}

/// Every path under `root` (relative, `/`-separated) with its bytes (`None` for a folder).
fn walk(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("a readable folder").flatten() {
            let p = e.path();
            let name = p
                .strip_prefix(root)
                .expect("under the root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let meta = std::fs::symlink_metadata(&p).expect("its metadata");
            if meta.is_dir() {
                out.insert(name, None);
                todo.push(p);
            } else {
                out.insert(name, Some(std::fs::read(&p).expect("its bytes")));
            }
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Every needle found in a name or the bytes of `tree`, labelled `where_`.
fn findings(
    tree: &BTreeMap<String, Option<Vec<u8>>>,
    needles: &[String],
    where_: &str,
) -> Vec<String> {
    let mut found = Vec::new();
    for (name, bytes) in tree {
        for n in needles {
            if name.contains(n.as_str()) {
                found.push(format!(
                    "{where_}/{name}: its name holds a credential or identifier"
                ));
            }
            if bytes.as_deref().is_some_and(|b| contains(b, n.as_bytes())) {
                found.push(format!("{where_}/{name}: holds a credential or identifier"));
            }
        }
    }
    found
}

/// `git` on the fixture repository `dir` (a repository this test made; R-T1a-1), with fixed
/// identities and dates and no automatic maintenance (review I2: `git commit` starts a
/// detached `maintenance run --auto` holding `objects/maintenance.lock`), so its files never
/// change after the command returns.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=hvm4b",
            "-c",
            "user.email=hvm4b@example.invalid",
        ])
        .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
        .args(["-c", "maintenance.auto=false", "-c", "gc.auto=0"])
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// One connection of the scenario: its session and profile.
struct Leg<'a> {
    c: &'a Conn,
    env: EnvTag,
}

/// The full scripted session (plan Task 19, review m3), then the audit of the library's
/// parent folder and the trash.
async fn credential_audit(legs: &[Leg<'_>], passwords: &[&str], select2: &str) {
    // The keyring really holds every secret of the session (the needles are not vacuous).
    for leg in legs {
        for (key, value) in passphrase_seeds(leg.c.profile.id) {
            let held = leg.c.secrets.get(&key, Retention::Forever).ok().flatten();
            assert!(
                held.is_some_and(|s| s.expose_secret() == value),
                "{}: the keyring holds no passphrase",
                leg.c.profile.name
            );
        }
    }
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(parent.path(), &[("README.md", "Orders scripts\n")]);
    // A repository the user made: Hacivat only ever reads it.
    git(&dir, &["init", "-q", "--template="]);
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    let git_before: BTreeMap<_, _> = walk(&dir.join(".git"));

    let lib = service(app.path(), trash.path());
    // 1. Attach (writes workspace.toml).
    let id = lib.attach(&dir, "orders").await;
    let prod = &legs[0].c.profile;
    let staging = &legs[legs.len() - 1].c.profile;
    // 2. The mapping dialog's path: share the alias, written only over the bytes it showed.
    let before = hacivat_scripts::manifest::preview(&dir).bytes;
    lib.svc
        .map_sharing(
            LocalMapping {
                profile: prod.id,
                database: DatabaseName::new("orders_archive"),
                library: id,
            },
            before,
        )
        .await
        .expect("the shared mapping");
    // A local-only mapping of staging's, then unmapped; and one dropped with its profile.
    lib.svc
        .map(
            LocalMapping {
                profile: staging.id,
                database: DatabaseName::new("orders_hist"),
                library: id,
            },
            false,
        )
        .await
        .expect("a local mapping");
    assert!(
        lib.svc
            .unmap(staging.id, DatabaseName::new("orders_hist"))
            .await
            .expect("unmap")
    );
    lib.svc
        .map(
            LocalMapping {
                profile: staging.id,
                database: DatabaseName::new("orders_old"),
                library: id,
            },
            false,
        )
        .await
        .expect("a local mapping");
    assert_eq!(
        lib.svc
            .drop_profile(staging.id)
            .await
            .expect("drop_profile"),
        1
    );
    let env = Some(legs[0].env);
    let save = |r: &str, text: &str, expect| {
        let r = rel(r);
        let text = SaveText {
            text: text.to_string(),
            meta: Default::default(),
        };
        let svc = lib.svc.clone();
        async move { svc.save(id, &r, env, text, expect).await }
    };
    // 3. New script, saved.
    save("report.sql", "SELECT 1", None).await.expect("save");
    wait_until("report.sql listed", || lib.lists(id, env, "report.sql")).await;
    // 4. Rename.
    lib.svc
        .rename(id, &rel("report.sql"), &rel("monthly.sql"), env)
        .await
        .expect("rename");
    wait_until("monthly.sql listed", || lib.lists(id, env, "monthly.sql")).await;
    // 5. Edit and save over the file read (the tab's stamp).
    let read = lib
        .svc
        .read(id, &rel("monthly.sql"), env)
        .await
        .expect("read");
    save("monthly.sql", select2, Some(read.stamp))
        .await
        .expect("save over");
    // 6. Save as.
    save("copy.sql", "SELECT 3", None).await.expect("save as");
    // 7. New folder, new script in it, then trash it.
    lib.svc
        .create(id, &rel("drafts"), EntryKind::Folder, env)
        .await
        .expect("new folder");
    lib.svc
        .create(id, &rel("drafts/new.sql"), EntryKind::Sql, env)
        .await
        .expect("new script");
    wait_until("drafts/new.sql listed", || {
        lib.lists(id, env, "drafts/new.sql")
    })
    .await;
    lib.svc
        .trash(id, &rel("drafts/new.sql"), env)
        .await
        .expect("trash");
    // 8. A production-only script.
    lib.svc
        .create(
            id,
            &rel("@production"),
            EntryKind::Folder,
            Some(EnvTag::Production),
        )
        .await
        .expect("an environment folder");
    lib.svc
        .save(
            id,
            &rel("@production/cleanup.sql"),
            Some(EnvTag::Production),
            SaveText {
                text: "SELECT 4".into(),
                meta: Default::default(),
            },
            None,
        )
        .await
        .expect("save under @production");
    // 9. A recovery copy left by an instance that crashed, then recovered by this one.
    let key = RecoveryKey {
        library: id,
        rel: rel("monthly.sql"),
        profile: Some(prod.id),
        database: Some(DatabaseName::new("orders")),
    };
    {
        let crashed = recovery::Instance::start(&lib.paths).expect("a recovery instance");
        recovery::store(
            &lib.paths,
            &crashed,
            &key,
            "SELECT 5 -- unsaved",
            &CopyMeta::default(),
        )
        .expect("a recovery copy");
    }
    let app_files = walk(app.path());
    let copies: Vec<&String> = app_files
        .iter()
        .filter(|(_, b)| {
            b.as_deref()
                .is_some_and(|b| contains(b, b"SELECT 5 -- unsaved"))
        })
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        copies.len(),
        1,
        "one recovery copy under the app's folders: {copies:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let file = app.path().join(copies[0]);
        let mode = |p: &Path| std::fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600, "the copy's mode");
        assert_eq!(
            mode(file.parent().expect("its folder")),
            0o700,
            "its folder's mode"
        );
    }
    let instance = recovery::Instance::start(&lib.paths).expect("this instance");
    let recovered = recovery::take_all(&lib.paths, &instance, id);
    assert_eq!(recovered.len(), 1, "the crashed instance's copy is offered");
    assert_eq!(recovered[0].text, "SELECT 5 -- unsaved");
    assert_eq!(recovered[0].key, key);
    recovery::discard_entry(&lib.paths, &recovered[0]).expect("discarded once restored");
    // 10. Git refresh.
    lib.svc.refresh(id).await;
    // 11. Detach, then attach again over the manifest already there (read, never rewritten),
    // and share the alias again (already there: nothing added).
    lib.svc.detach(id).await.expect("detach");
    let id = lib
        .svc
        .attach(dir.clone(), Some(logical("orders")), true)
        .await
        .expect("re-attach");
    lib.ready(id).await;
    lib.svc
        .map(
            LocalMapping {
                profile: prod.id,
                database: DatabaseName::new("orders_archive"),
                library: id,
            },
            true,
        )
        .await
        .expect("the alias again");
    // 12. Run monthly.sql as script on every connection, under its environment.
    for leg in legs {
        let port = Port::yes();
        let src = lib.source(id, "monthly.sql", Some(leg.env)).await;
        let (out, progress) = run_script(&leg.c.s, &leg.c.profile, src, port).await;
        assert!(finished(&out), "{}: {out:?}", leg.c.profile.name);
        assert_eq!(
            progress.units_run, 1,
            "{}: the saved text ran",
            leg.c.profile.name
        );
    }
    lib.ready(id).await;
    // 13. Quit.
    lib.svc.shutdown();
    drop(lib);

    // The audit: the parent holds only the library, the trash only what was trashed, and
    // neither holds anything identifying.
    let profiles: Vec<(&Profile, &str)> = legs
        .iter()
        .zip(passwords)
        .map(|(l, p)| (l.c.profile.as_ref(), *p))
        .collect();
    let needles = needles(&profiles);
    let all = walk(parent.path());
    let git_after: BTreeMap<String, Option<Vec<u8>>> = all
        .iter()
        .filter_map(|(n, b)| {
            n.strip_prefix("orders-scripts/.git/")
                .map(|n| (n.to_string(), b.clone()))
        })
        .collect();
    assert_eq!(
        git_after, git_before,
        ".git/ changed: Hacivat wrote into the repository"
    );
    let mine: BTreeMap<String, Option<Vec<u8>>> = all
        .into_iter()
        .filter(|(n, _)| n != "orders-scripts/.git" && !n.starts_with("orders-scripts/.git/"))
        .collect();
    let trashed = walk(trash.path());
    let mut found = findings(&mine, &needles, "parent");
    found.extend(findings(&trashed, &needles, "trash"));
    assert!(found.is_empty(), "{found:?}");
    let names: BTreeSet<&str> = mine.keys().map(String::as_str).collect();
    let want: BTreeSet<&str> = [
        "orders-scripts",
        "orders-scripts/.hacivat",
        "orders-scripts/.hacivat/workspace.toml",
        "orders-scripts/@production",
        "orders-scripts/@production/cleanup.sql",
        "orders-scripts/README.md",
        "orders-scripts/copy.sql",
        "orders-scripts/drafts",
        "orders-scripts/monthly.sql",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        names, want,
        "the only paths are the user's scripts and the manifest"
    );
    assert_eq!(
        trashed.keys().map(String::as_str).collect::<Vec<_>>(),
        ["new.sql"],
        "the trash holds only the trashed script"
    );
    let manifest = String::from_utf8(
        mine["orders-scripts/.hacivat/workspace.toml"]
            .clone()
            .expect("a file"),
    )
    .expect("UTF-8");
    let table: toml::Table = toml::from_str(&manifest).expect("the manifest parses");
    let keys: BTreeSet<&str> = table.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["aliases", "database", "format"]));
    assert_eq!(table["database"].as_str(), Some("orders"));
    // The shared alias only, once; the local-only databases never became aliases.
    let aliases: Vec<&str> = table["aliases"]
        .as_array()
        .expect("aliases")
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    assert_eq!(aliases, ["orders_archive"]);
    fact!(
        "HV-FACT t19 e6 connections={} paths={} trash={} needles={} found=0 git_unchanged=true \
         manifest_keys=aliases,database,format recovered=1",
        legs.len(),
        names.len(),
        trashed.len(),
        needles.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn no_credentials_reach_the_library_on_real_servers() {
    if !hacivat_testkit::docker_enabled() {
        return;
    }
    let mut ran = 0;
    for id in ["pg17", "mysql84"] {
        let Some(t) = require_target(id) else {
            continue;
        };
        let mut conns = Vec::new();
        for (name, env) in [
            ("orders-prod", EnvTag::Production),
            ("orders-staging", EnvTag::Staging),
        ] {
            let p = server_profile(&t, name, Some(env), "orders");
            let seeds = passphrase_seeds(p.id);
            conns.push((on_server_as(&t, p, &seeds).await, env));
        }
        let legs: Vec<Leg<'_>> = conns.iter().map(|(c, env)| Leg { c, env: *env }).collect();
        credential_audit(&legs, &[&t.password, &t.password], "SELECT 2").await;
        fact!("HV-FACT t19 e6 target={id} audited=true");
        for (c, _) in &conns {
            c.m.disconnect(c.profile.id);
        }
        ran += 1;
    }
    assert!(ran > 0, "the active set has neither pg17 nor mysql84");
}

/// The fake leg's profile: the plan's port, user and password, and a host that appears nowhere
/// else in the test's library. The plan's `db.example.internal` would stop the connect at its
/// DNS step (the fake's in-memory transport replaces only the socket), so the host is an
/// address of TEST-NET-1 (RFC 5737), which needs no lookup.
fn fake_profile(name: &str, env: EnvTag, engine: DbmsKind) -> Profile {
    let mut p = Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        engine,
        environment: Some(env),
        endpoint: ProfileEndpoint::Server {
            host: "192.0.2.77".into(),
            port: Some(6543),
        },
        database: Some("orders".into()),
        user: Some("hv_user".into()),
        auth: AuthProviderId::Password,
        retention: Retention::Never,
        ..Profile::default()
    };
    // The fake speaks no TLS (a non-loopback host requires it by default).
    p.tls.mode = TlsMode::Disable;
    p
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_credentials_reach_the_library_with_a_fake() {
    let mut conns = Vec::new();
    for (name, env) in [
        ("orders-prod", EnvTag::Production),
        ("orders-staging", EnvTag::Staging),
    ] {
        let mut drivers = DriverRegistry::new();
        drivers.register(Arc::new(
            hacivat_driver::fake::FakeDriver::new(DbmsKind::Postgres).tracks_transactions(),
        ));
        let p = fake_profile(name, env, DbmsKind::Postgres);
        let seeds = passphrase_seeds(p.id);
        let c = connect(deps(drivers, true, &seeds), p, Some("hv_test_pw")).await;
        conns.push((c, env));
    }
    let legs: Vec<Leg<'_>> = conns.iter().map(|(c, env)| Leg { c, env: *env }).collect();
    credential_audit(&legs, &["hv_test_pw", "hv_test_pw"], "SELECT 2").await;
}

/// The T19 reviewer's hostile test (adopted, review m3): the mapping lifecycle the first audit
/// skipped (the mapping dialog's `map_sharing`, a local-only map, unmap, `drop_profile`,
/// detach and a re-attach over the existing manifest), audited over the library's **parent**
/// (siblings, leftovers), with a positive control proving the needles find a real profile id
/// where Hacivat may keep one (`libraries.toml`, app config).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mapping_lifecycle_writes_no_profile_id_near_the_library() {
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(parent.path(), &[("a.sql", "SELECT 1")]);
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;
    let (p1, p2) = (ProfileId::new(), ProfileId::new());
    let before = hacivat_scripts::manifest::preview(&dir).bytes;
    lib.svc
        .map_sharing(
            LocalMapping {
                profile: p1,
                database: DatabaseName::new("orders_archive"),
                library: id,
            },
            before,
        )
        .await
        .expect("map_sharing");
    lib.svc
        .map(
            LocalMapping {
                profile: p2,
                database: DatabaseName::new("orders_hist"),
                library: id,
            },
            false,
        )
        .await
        .expect("local map");
    // Positive control: the needles DO find the ids where Hacivat may keep them.
    let reg = std::fs::read(lib.paths.libraries_toml()).expect("libraries.toml");
    for p in [p1, p2] {
        assert!(
            id_spellings(p).iter().any(|n| contains(&reg, n.as_bytes())),
            "positive control: an id is not found in libraries.toml (a needle spelled wrong?)"
        );
    }
    lib.svc
        .save(
            id,
            &rel("b.sql"),
            None,
            SaveText {
                text: "SELECT 2".into(),
                meta: Default::default(),
            },
            None,
        )
        .await
        .expect("save");
    assert!(
        lib.svc
            .unmap(p1, DatabaseName::new("orders_archive"))
            .await
            .expect("unmap")
    );
    lib.svc.drop_profile(p2).await.expect("drop_profile");
    lib.svc.detach(id).await.expect("detach");
    let id2 = lib.attach(&dir, "orders").await;
    lib.svc
        .map(
            LocalMapping {
                profile: p1,
                database: DatabaseName::new("orders_archive"),
                library: id2,
            },
            true,
        )
        .await
        .expect("map again with share (the alias is already there)");
    lib.svc.shutdown();
    let all = walk(parent.path());
    let names: Vec<&str> = all.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "orders-scripts",
            "orders-scripts/.hacivat",
            "orders-scripts/.hacivat/workspace.toml",
            "orders-scripts/a.sql",
            "orders-scripts/b.sql",
        ],
        "paths under the library's parent"
    );
    let needles: Vec<String> = id_spellings(p1)
        .into_iter()
        .chain(id_spellings(p2))
        .collect();
    let found = findings(&all, &needles, "parent");
    assert!(found.is_empty(), "{found:?}");
    let manifest = String::from_utf8(
        all["orders-scripts/.hacivat/workspace.toml"]
            .clone()
            .expect("a file"),
    )
    .expect("UTF-8");
    let table: toml::Table = toml::from_str(&manifest).expect("the manifest parses");
    let keys: BTreeSet<&str> = table.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        BTreeSet::from(["aliases", "database", "format"]),
        "{manifest}"
    );
    let aliases: Vec<&str> = table["aliases"]
        .as_array()
        .expect("aliases")
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    assert_eq!(aliases, ["orders_archive"], "{manifest}");
    assert!(walk(trash.path()).is_empty(), "the trash got nothing");
    fact!(
        "HV-FACT t19 e6 mapping_lifecycle paths={} found=0",
        all.len()
    );
}
