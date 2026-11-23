//! M4b Task 19: Docker conformance of context-from-location (gate E1) and the credential audit
//! (gate E6), with the library service, the binding and real sessions of a `ConnectionManager`
//! (M2b's configuration), the way a script tab finds and runs its file:
//!
//! 1. the library is attached (its `.hacivat/workspace.toml` written by the service);
//! 2. a database node `(profile, database)` binds it through `hacivat_scripts::resolve`;
//! 3. the file is read through `LibraryService::read` under the profile's environment;
//! 4. its text runs on a session of that profile, opened on that database, after the tab's
//!    database check (R-M4b-27: MySQL switches its own session back once; the check is mirrored
//!    here from `hacivat-app`'s `scripts::tabs`, which this crate cannot reach).
//!
//! What only Docker proves: the run lands on the server and database of the connection the
//! file was opened under, checked on the server by `current_database()`/`DATABASE()`, the
//! server version and a marker row that only the right server holds.
//!
//! - **E1** `same_library_runs_on_each_connection`: the same library under orders-prod and
//!   orders-staging on PostgreSQL (pg17 / pg16) and MySQL (mysql84 / mysql80), and on two SQLite
//!   files (`sqlite_file_stem_binds`, no server). The pairs need the `full` set; a pair whose
//!   staging server is not in the active set is skipped with a line on stderr, never silently
//!   under `full`. Facts: `HV-FACT t19 e1 …`.
//! - **MySQL's database level** (`mysql_schema_is_the_database_level`,
//!   `mysql_switch_is_reapplied_after_a_reconnect`): a script tab's own session switched to the
//!   library's database, also after the server killed it.
//! - **E6** `no_credentials_reach_the_library_on_real_servers` (Docker) and
//!   `no_credentials_reach_the_library_with_a_fake` (no Docker, runs everywhere): a full scripted
//!   session (attach, map with a shared alias, save, save as, rename, new folder, new script,
//!   trash, a recovery copy, a git refresh, a two-pass read and a run on prod and staging, quit),
//!   then every byte and every name under the library folder is searched for the connections'
//!   host, port, user, password, names, profile ids (in each spelling) and keyring service
//!   names. `.git/` is byte-identical before and after (Hacivat writes nothing there), and the
//!   only paths in the library are the user's scripts and `.hacivat/workspace.toml`, whose keys
//!   are exactly `format`, `database`, `aliases`.
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-scripts --test
//! docker_m4b`. Server fixtures are `hvm4b_t19_*` tables, unique to the process and dropped by a
//! guard (also when a test fails). Every wait is on a condition with a ceiling.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hacivat_driver::{DriverRegistry, EngineCatalog, ExecOpts, ExecSummary};
use hacivat_net::TlsMode;
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_net::{BoxedStream, StreamFactory};
use hacivat_proto::{DbmsKind, Department, EnvTag, ProfileId};
use hacivat_scripts::binding::logical_for_sqlite;
use hacivat_scripts::recovery::{self, CopyMeta, RecoveryKey};
use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
use hacivat_scripts::testing::{FakeHandle, logical};
use hacivat_scripts::{
    DatabaseName, DbNode, EntryKind, GitSummary, LibraryId, LibraryState, LocalMapping, RelPath,
    SaveText, ScriptChunks as _, TextEncoding, resolve,
};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretKey,
    SecretSlot, SecretStore, SecretString,
};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ClientId, ExecOrigin, Request, SessionEvent, SessionHandle, SessionKind, SessionSpec, TxMode,
    parse_schema_probe, schema_probe_sql,
};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
use hacivat_settings::LocalPaths;
use hacivat_testkit::{DockerTarget, TlsDefault, require_target};

const CEILING: Duration = Duration::from_secs(30);

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

/// A library service over app folders in `app`, fed by a fake event source (the scenario's
/// own calls refresh the listing), trashing into `trash` (outside the library).
struct Lib {
    svc: LibraryService,
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
        svc,
        paths,
        _fake: fake,
    }
}

impl Lib {
    /// Attaches `dir` bound to `db`, writing its `workspace.toml`, and waits for its first scan
    /// and status.
    async fn attach(&self, dir: &Path, db: &str) -> LibraryId {
        let id = self
            .svc
            .attach(dir.to_path_buf(), Some(logical(db)), true)
            .await
            .expect("the library attaches");
        wait_until("the first scan and status", || {
            self.svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
            })
        })
        .await;
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

    /// `r`'s text, read as a script tab reads it under `env`.
    async fn text(&self, id: LibraryId, r: &str, env: Option<EnvTag>) -> String {
        self.svc
            .read(id, &rel(r), env)
            .await
            .unwrap_or_else(|e| panic!("reading {r}: {e:?}"))
            .text
    }

    /// `r` read twice the way "Run as script" reads it (pass 1, then pass 2 checked against
    /// pass 1's record); the text, which both passes must agree on.
    async fn two_passes(&self, id: LibraryId, r: &str, env: Option<EnvTag>) -> String {
        let utf8 = TextEncoding::UTF_8;
        let mut pass1 = self
            .svc
            .open_stream(id, &rel(r), env, utf8, None)
            .await
            .expect("pass 1 opens");
        let one = drain(&mut pass1);
        let mut pass2 = self
            .svc
            .open_stream(id, &rel(r), env, utf8, Some(pass1.record()))
            .await
            .expect("pass 2 opens");
        let two = drain(&mut pass2);
        assert_eq!(one, two, "{r}: the passes read differently");
        one
    }
}

fn drain(s: &mut hacivat_scripts::ScriptStream) -> String {
    let mut out = String::new();
    while let Some(chunk) = s.next_chunk() {
        out.push_str(&chunk.expect("a chunk"));
    }
    out
}

/// A library folder named `orders-scripts` (the folder name is not the database: the binding
/// comes from `workspace.toml`) holding `files`.
fn library_dir(parent: &Path, files: &[(&str, &str)]) -> PathBuf {
    let dir = parent.join("orders-scripts");
    for (name, text) in files {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("a folder");
        std::fs::write(&p, text).expect("a script");
    }
    std::fs::create_dir_all(&dir).expect("the library folder");
    dir
}

// ---------------------------------------------------------------------------------------------
// Support: connections

fn deps(drivers: DriverRegistry, wrap: bool) -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
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
        // A fake answers in memory: no socket is opened for it.
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

/// A connected profile: its manager, the profile, its default session.
struct Conn {
    m: Arc<ConnectionManager>,
    profile: Arc<Profile>,
    s: SessionHandle,
}

/// Connects `profile` (with `password`) on a manager of its own over `deps`.
async fn connect(deps: ConnectDeps, profile: Profile, password: Option<&str>) -> Conn {
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
    Conn { m, profile, s }
}

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

/// Connects to `t` as `name`, on `database`.
async fn on_server(t: &DockerTarget, name: &str, env: Option<EnvTag>, database: &str) -> Conn {
    connect(
        deps(default_drivers(), false),
        server_profile(t, name, env, database),
        Some(&t.password),
    )
    .await
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

/// The profiles' transaction mode: autocommit, so what a run wrote is committed when its reply
/// comes and the side channel sees it (production and staging default to manual transactions,
/// `session::default_mode`, where the row would wait for the user's Commit).
const AUTOCOMMIT: Option<TxMode> = Some(TxMode::Auto);

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

/// The script tab's database check before a Run (R-M4b-27, mirrored from
/// `hacivat-app::scripts::tabs::run_gate`): the session must be on `bound`; a MySQL tab's own
/// session switches back once (`Request::SwitchSchema`) and must then read `bound`. Returns
/// the database the session was on before the check.
async fn database_check(s: &SessionHandle, engine: DbmsKind, bound: &str) -> Option<String> {
    let before = current_database(s, engine).await;
    if before.as_deref() == Some(bound) {
        return before;
    }
    assert!(
        matches!(engine, DbmsKind::MySql | DbmsKind::MariaDb),
        "{engine:?}: the session is on {before:?}, not {bound}: Run refuses"
    );
    let switched = s
        .submit(Request::SwitchSchema(bound.to_string()))
        .reply
        .await
        .expect("the actor replies");
    assert!(switched.is_ok(), "switching to {bound}: {switched:?}");
    assert_eq!(
        current_database(s, engine).await.as_deref(),
        Some(bound),
        "after the one switch back"
    );
    before
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
/// a runtime of their own on another thread, so they work while unwinding.
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
                    let _ = exec(&admin.s, &sql).await;
                    admin.m.disconnect(admin.profile.id);
                }
            });
        })
        .join();
        if joined.is_err() {
            eprintln!("t19: a fixture drop failed; the next run's names differ");
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

/// The SQL of the pair's scripts on `engine`: `a.sql` reads where it runs, `mark.sql` leaves a
/// row naming where it ran.
fn e1_scripts(engine: DbmsKind, marker: &str) -> (String, String) {
    match engine {
        DbmsKind::Postgres => (
            "SELECT current_database(), current_setting('server_version_num')".to_string(),
            format!(
                "INSERT INTO {marker} (who, db, ver) VALUES \
                 ('mark.sql', current_database(), current_setting('server_version_num'))"
            ),
        ),
        _ => (
            "SELECT DATABASE(), VERSION()".to_string(),
            format!(
                "INSERT INTO {marker} (who, db, ver) VALUES ('mark.sql', DATABASE(), VERSION())"
            ),
        ),
    }
}

/// Whether server version text `v` (PostgreSQL's `server_version_num`, MySQL's `VERSION()`) is
/// of major `major`.
fn of_major(engine: DbmsKind, major: &str, v: &str) -> bool {
    match engine {
        DbmsKind::Postgres => v.len() == 6 && v.starts_with(major),
        _ => v.starts_with(&format!("{major}.")),
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
    let (a_sql, mark_sql) = e1_scripts(engine, &marker);
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(
        parent.path(),
        &[
            ("a.sql", &a_sql),
            ("mark.sql", &mark_sql),
            ("@production/only_prod.sql", "SELECT 1"),
        ],
    );
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;

    let mut fixtures = Fixtures::new();
    let mut admins = Vec::new();
    for t in [&prod, &staging] {
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
        admins.push(admin);
    }
    let marks = format!("SELECT COUNT(*) FROM {marker} WHERE who = 'mark.sql' AND db = 'orders'");

    let sides = [
        (&prod, "orders-prod", EnvTag::Production, "production"),
        (&staging, "orders-staging", EnvTag::Staging, "staging"),
    ];
    for (k, (t, name, env, env_name)) in sides.iter().enumerate() {
        let c = on_server(t, name, Some(*env), "orders").await;
        let node = DbNode {
            profile: c.profile.id,
            database: DatabaseName::new("orders"),
            logical: logical("orders"),
        };
        assert_eq!(lib.bound(&node), [id], "{name}: the library binds");
        // The `@production/` file is listed and read only under production (R-M4b-6).
        let prod_only = lib.lists(id, Some(*env), "@production/only_prod.sql");
        assert_eq!(
            prod_only,
            *env == EnvTag::Production,
            "{name}: @production/ listing"
        );
        assert_eq!(
            lib.svc
                .read(id, &rel("@production/only_prod.sql"), Some(*env))
                .await
                .is_ok(),
            *env == EnvTag::Production,
            "{name}: @production/ read"
        );
        // A script tab on MySQL has a session of its own; PostgreSQL's tab runs on the
        // profile's session (R-T14-3).
        let s = if engine == DbmsKind::Postgres {
            c.s.clone()
        } else {
            own_session(&c, k as u64).await
        };
        database_check(&s, engine, "orders").await;
        let text = lib.text(id, "a.sql", Some(*env)).await;
        let got = row(&s, name, &text).await;
        let (db, ver) = (got[0].clone(), got[1].clone());
        assert_eq!(db, "orders", "{name}");
        assert!(
            of_major(engine, &t.major, &ver),
            "{name}: {ver} is not {}",
            t.major
        );
        let text = lib.text(id, "mark.sql", Some(*env)).await;
        exec(&s, &text)
            .await
            .unwrap_or_else(|e| panic!("{name}: mark.sql: {e}"));
        let on: Vec<i64> = {
            let mut v = Vec::new();
            for a in &admins {
                v.push(count(&a.s, name, &marks).await);
            }
            v
        };
        // Prod's run leaves a row on prod only; staging's, one more on staging only.
        let want = if k == 0 { vec![1, 0] } else { vec![1, 1] };
        assert_eq!(on, want, "{name}: marker rows on (prod, staging)");
        println!(
            "HV-FACT t19 e1 profile={name} target={} database={db} server_version={ver} env={env_name} \
             marker_prod={} marker_staging={} production_file_listed={prod_only}",
            t.id, on[0], on[1]
        );
        c.m.disconnect(c.profile.id);
    }
    for a in &admins {
        a.m.disconnect(a.profile.id);
    }
    drop(fixtures);
}

fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary folder")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn same_library_runs_on_each_connection() {
    e1_pair(DbmsKind::Postgres, "pg17", "pg16").await;
    e1_pair(DbmsKind::MySql, "mysql84", "mysql80").await;
}

/// SQLite (no server: runs in every set, also without Docker): a database's logical name is its
/// file's stem (R-M4b-32), so `prod/orders.db` and `staging/orders.db` both bind library
/// `orders`, and each run lands in its own file.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn sqlite_file_stem_binds() {
    let files = tempdir();
    let marker = "hvm4b_t19_e1_mark";
    let mark_sql = format!(
        "INSERT INTO {marker} (who, file) SELECT 'mark.sql', file FROM pragma_database_list \
         WHERE name = 'main'"
    );
    let (app, parent, trash) = (tempdir(), tempdir(), tempdir());
    let dir = library_dir(parent.path(), &[("mark.sql", &mark_sql)]);
    let lib = service(app.path(), trash.path());
    let id = lib.attach(&dir, "orders").await;

    let sides = [
        ("prod", "orders-prod", EnvTag::Production),
        ("staging", "orders-staging", EnvTag::Staging),
    ];
    let mut paths = Vec::new();
    let mut admins = Vec::new();
    for (folder, _, _) in &sides {
        let p = files.path().join(folder).join("orders.db");
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("a folder");
        let admin = connect(
            deps(default_drivers(), false),
            sqlite_profile(&p, "hvm4b-t19-admin", None),
            None,
        )
        .await;
        exec(
            &admin.s,
            &format!("CREATE TABLE {marker} (who TEXT, file TEXT)"),
        )
        .await
        .expect("the marker table");
        paths.push(p);
        admins.push(admin);
    }
    let marks = format!("SELECT COUNT(*) FROM {marker} WHERE who = 'mark.sql'");
    for (k, (_, name, env)) in sides.iter().enumerate() {
        let c = connect(
            deps(default_drivers(), false),
            sqlite_profile(&paths[k], name, Some(*env)),
            None,
        )
        .await;
        let stem = logical_for_sqlite(&paths[k]).expect("a stem");
        assert_eq!(stem.as_str(), "orders");
        let node = DbNode {
            profile: c.profile.id,
            database: DatabaseName::new("main"),
            logical: stem,
        };
        assert_eq!(lib.bound(&node), [id], "{name}: the library binds");
        let text = lib.text(id, "mark.sql", Some(*env)).await;
        exec(&c.s, &text)
            .await
            .unwrap_or_else(|e| panic!("{name}: mark.sql: {e}"));
        let mut on = Vec::new();
        for a in &admins {
            on.push(count(&a.s, name, &marks).await);
        }
        let want = if k == 0 { vec![1, 0] } else { vec![1, 1] };
        assert_eq!(on, want, "{name}: marker rows in (prod, staging)");
        println!(
            "HV-FACT t19 e1 profile={name} target=sqlite logical=orders env={env:?} marker_prod={} marker_staging={}",
            on[0], on[1]
        );
        c.m.disconnect(c.profile.id);
    }
    for a in &admins {
        a.m.disconnect(a.profile.id);
    }
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
            &format!("INSERT INTO {marker} (who, db) VALUES ('mark.sql', DATABASE())"),
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

    async fn run_mark(&self, s: &SessionHandle) {
        let text = self
            .lib
            .text(self.id, "mark.sql", Some(EnvTag::Staging))
            .await;
        exec(s, &text)
            .await
            .unwrap_or_else(|e| panic!("mark.sql: {e}"));
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
    let was = database_check(&script, t.engine, "orders_archive").await;
    let default = current_database(&r.c.s, t.engine).await;
    let now = current_database(&script, t.engine).await;
    println!(
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
    assert_eq!(
        default.as_deref(),
        Some("orders"),
        "the default session was not switched"
    );
    assert_eq!(now.as_deref(), Some("orders_archive"));
    r.run_mark(&script).await;
    assert_eq!(
        r.marks().await,
        (0, 1),
        "the run landed in orders_archive only"
    );
    r.c.m.disconnect(r.c.profile.id);
    r.admin.m.disconnect(r.admin.profile.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_switch_is_reapplied_after_a_reconnect() {
    let Some(t) = docker_target("mysql84") else {
        return;
    };
    let r = mysql_rig(&t).await;
    let script = own_session(&r.c, 2).await;
    database_check(&script, t.engine, "orders_archive").await;
    let mut events = script.events();
    let backend = row(&script, "script", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    exec(&r.c.s, &format!("KILL {backend}"))
        .await
        .unwrap_or_else(|e| panic!("KILL {backend}: {e}"));
    // The next request finds the connection gone; the session recovers in place.
    let _ = script.submit(Request::Ping).reply.await;
    let deadline = tokio::time::Instant::now() + CEILING;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Ok(SessionEvent::Reconnected)) => break,
            Ok(Ok(_)) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("the session's events ended: {e}"),
            Err(_) => panic!("timed out waiting for Reconnected"),
        }
    }
    let after = current_database(&script, t.engine).await;
    let new_backend = row(&script, "script", "SELECT CONNECTION_ID()")
        .await
        .remove(0);
    assert_ne!(new_backend, backend, "a new connection after the kill");
    // The check before the next statement puts the session back on the library's database.
    database_check(&script, t.engine, "orders_archive").await;
    let checked = current_database(&script, t.engine).await;
    println!(
        "HV-FACT t19 mysql_reconnect target={} after_reconnect={} after_check={}",
        t.id,
        after.as_deref().unwrap_or("?"),
        checked.as_deref().unwrap_or("?")
    );
    assert_eq!(checked.as_deref(), Some("orders_archive"));
    r.run_mark(&script).await;
    assert_eq!(
        r.marks().await,
        (0, 1),
        "the run landed in orders_archive only"
    );
    r.c.m.disconnect(r.c.profile.id);
    r.admin.m.disconnect(r.admin.profile.id);
}

// ---------------------------------------------------------------------------------------------
// E6: no credentials ever reach a library folder

/// What must never appear in a library: every spelling of each connection's identifiers and
/// credentials.
fn needles(profiles: &[(&Profile, &str)]) -> Vec<String> {
    let mut out: Vec<String> = vec!["password =".into(), "host =".into(), "s3cret".into()];
    for (p, password) in profiles {
        let u = p.id.0;
        out.extend([
            password.to_string(),
            p.name.clone(),
            u.to_string(),
            u.simple().to_string(),
            u.to_string().to_ascii_uppercase(),
            format!("{:?}", p.id),
            SecretKey {
                profile: p.id,
                slot: SecretSlot::DbPassword,
            }
            .service(),
            SecretKey {
                profile: p.id,
                slot: SecretSlot::SshPassphrase { hop: 0 },
            }
            .service(),
        ]);
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

/// `git` on the fixture repository `dir` (a repository this test made; R-T1a-1), with fixed
/// identities and dates so its files never vary.
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
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// One connection of the scenario: its session, profile and the password it was given.
struct Leg<'a> {
    s: &'a SessionHandle,
    profile: &'a Profile,
    env: EnvTag,
}

/// The full scripted session (plan Task 19), then the audit of the library folder.
async fn credential_audit(legs: &[Leg<'_>], passwords: &[&str]) {
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
    let prod = legs[0].profile;
    // 2. Map another database of prod's, sharing the alias (workspace.toml gains it).
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
        .expect("the mapping");
    let env = Some(legs[0].env);
    let save = |r: &str, text: &str, expect| {
        let r = rel(r);
        let text = SaveText {
            text: text.to_string(),
            meta: Default::default(),
        };
        let svc = &lib.svc;
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
    save("monthly.sql", "SELECT 2", Some(read.stamp))
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
    // 9. A recovery copy of an unsaved edit (it lives under the app's folders).
    let instance = recovery::Instance::start(&lib.paths).expect("a recovery instance");
    recovery::store(
        &lib.paths,
        &instance,
        &RecoveryKey {
            library: id,
            rel: rel("monthly.sql"),
            profile: Some(prod.id),
            database: Some(DatabaseName::new("orders")),
        },
        "SELECT 5 -- unsaved",
        &CopyMeta::default(),
    )
    .expect("a recovery copy");
    // 10. Git refresh.
    lib.svc.refresh(id).await;
    // 11. Run monthly.sql (two passes, as Run as script reads it) on every connection.
    for leg in legs {
        let text = lib.two_passes(id, "monthly.sql", Some(leg.env)).await;
        let got = row(leg.s, &leg.profile.name, &text).await;
        assert_eq!(got, ["2"], "{}: the saved text ran", leg.profile.name);
    }
    wait_until("the git status after the refresh", || {
        lib.svc
            .snapshot()
            .view(env)
            .library(id)
            .is_some_and(|l| l.git() != GitSummary::Pending)
    })
    .await;
    // 12. Quit.
    lib.svc.shutdown();
    drop(lib);

    // The recovery copy is outside the library, private.
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
        let mode = std::fs::metadata(&file)
            .expect("the copy")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the copy's mode");
        let folder = file.parent().expect("its folder");
        let mode = std::fs::metadata(folder)
            .expect("its folder")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "the copy folder's mode");
    }

    // The audit.
    let profiles: Vec<(&Profile, &str)> = legs
        .iter()
        .zip(passwords)
        .map(|(l, p)| (l.profile, *p))
        .collect();
    let needles = needles(&profiles);
    let all = walk(&dir);
    assert_eq!(
        all.iter()
            .filter(|(n, _)| n.as_str() == ".git" || n.starts_with(".git/"))
            .filter(|(n, _)| n.as_str() != ".git")
            .map(|(n, b)| (n.strip_prefix(".git/").unwrap_or(n).to_string(), b.clone()))
            .collect::<BTreeMap<_, _>>(),
        git_before,
        ".git/ changed: Hacivat wrote into the repository"
    );
    let mine: BTreeMap<&String, &Option<Vec<u8>>> = all
        .iter()
        .filter(|(n, _)| n.as_str() != ".git" && !n.starts_with(".git/"))
        .collect();
    let mut found = Vec::new();
    for (name, bytes) in &mine {
        for n in &needles {
            if name.contains(n.as_str()) {
                found.push(format!("{name}: its name holds a credential or identifier"));
            }
            if bytes.as_deref().is_some_and(|b| contains(b, n.as_bytes())) {
                found.push(format!("{name}: holds a credential or identifier"));
            }
        }
    }
    assert!(found.is_empty(), "{found:?}");
    let names: BTreeSet<&str> = mine.keys().map(|n| n.as_str()).collect();
    let want: BTreeSet<&str> = [
        ".hacivat",
        ".hacivat/workspace.toml",
        "@production",
        "@production/cleanup.sql",
        "README.md",
        "copy.sql",
        "drafts",
        "monthly.sql",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        names, want,
        "the only paths are the user's scripts and the manifest"
    );
    let manifest = String::from_utf8(
        mine[&".hacivat/workspace.toml".to_string()]
            .clone()
            .expect("a file"),
    )
    .expect("UTF-8");
    let table: toml::Table = toml::from_str(&manifest).expect("the manifest parses");
    let keys: BTreeSet<&str> = table.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["aliases", "database", "format"]));
    assert_eq!(table["database"].as_str(), Some("orders"));
    // The shared alias of step 2 is the database's name only.
    let aliases: Vec<&str> = table["aliases"]
        .as_array()
        .expect("aliases")
        .iter()
        .filter_map(|a| a.as_str())
        .collect();
    assert_eq!(aliases, ["orders_archive"]);
    println!(
        "HV-FACT t19 e6 connections={} files={} needles={} found=0 git_unchanged=true manifest_keys=aliases,database,format",
        legs.len(),
        names.len(),
        needles.len()
    );
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
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
        let prod = on_server(&t, "orders-prod", Some(EnvTag::Production), "orders").await;
        let staging = on_server(&t, "orders-staging", Some(EnvTag::Staging), "orders").await;
        let legs = [
            Leg {
                s: &prod.s,
                profile: &prod.profile,
                env: EnvTag::Production,
            },
            Leg {
                s: &staging.s,
                profile: &staging.profile,
                env: EnvTag::Staging,
            },
        ];
        credential_audit(&legs, &[&t.password, &t.password]).await;
        println!("HV-FACT t19 e6 target={id} audited=true");
        prod.m.disconnect(prod.profile.id);
        staging.m.disconnect(staging.profile.id);
        ran += 1;
    }
    assert!(ran > 0, "the active set has neither pg17 nor mysql84");
}

/// The fake leg's profile: the plan's port, user and password, and a host that appears nowhere
/// else in the test's library. The plan's `db.example.internal` would stop the connect at its
/// DNS step (the fake's in-memory transport replaces only the socket), so the host is an
/// address of TEST-NET-1 (RFC 5737), which needs no lookup.
fn fake_profile(name: &str, env: EnvTag) -> Profile {
    let mut p = Profile {
        id: ProfileId::new(),
        name: name.to_string(),
        engine: DbmsKind::Postgres,
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
        // The saved text's answer, as the server would give it.
        let fake = Arc::new(
            hacivat_driver::fake::FakeDriver::new(DbmsKind::Postgres).exec(vec![
                hacivat_driver::fake::FakeExec {
                    prefix: "SELECT 2",
                    result: Ok(ExecSummary {
                        rows: vec![vec![Some("2".into())]],
                        row_count: 1,
                        ..ExecSummary::default()
                    }),
                    delay: Duration::ZERO,
                    tx_after: None,
                    panics: false,
                },
            ]),
        );
        let mut drivers = DriverRegistry::new();
        drivers.register(fake);
        let c = connect(
            deps(drivers, true),
            fake_profile(name, env),
            Some("hv_test_pw"),
        )
        .await;
        conns.push((c, env));
    }
    let legs: Vec<Leg<'_>> = conns
        .iter()
        .map(|(c, env)| Leg {
            s: &c.s,
            profile: &c.profile,
            env: *env,
        })
        .collect();
    credential_audit(&legs, &["hv_test_pw", "hv_test_pw"]).await;
}
