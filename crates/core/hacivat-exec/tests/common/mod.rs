//! Shared fixtures of the run-service tests (append-only: each task adds one block headed
//! `// ## m3a-tNN-<slug>`; Task 23's helpers, then Task 31's matrix helpers).
#![allow(dead_code, reason = "each test file uses a subset of the fixtures")]
#![cfg_attr(not(unix), allow(unused_imports, unused_variables))]

// ## m3a-t01b-seams

use hacivat_proto::DbmsKind;
use hacivat_secrets::{AuthProviderId, Retention};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes};
use sql_split::exec_range::CaretOrSelection;
use sql_split::{DocSnapshot, SqlDocument};
use sql_text::TextSize;

/// A profile of `engine` that a `FakeDriver` session connects without a password or a socket.
pub fn profile_of(engine: DbmsKind) -> Profile {
    let endpoint = match engine {
        DbmsKind::Sqlite => ProfileEndpoint::File {
            path: std::env::temp_dir().join("hv-exec-test.db"),
            create_if_missing: true,
        },
        _ => ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: None,
        },
    };
    Profile {
        name: "orders".into(),
        engine,
        endpoint,
        user: Some("deploy".into()),
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    }
}

/// A PostgreSQL 18 context with the default modes.
pub fn pg18() -> DialectCtx {
    DialectCtx {
        id: DialectId::Postgres,
        version: Some(sql_dialect::ServerVersion {
            major: 18,
            minor: 0,
            patch: 0,
        }),
        modes: LexModes::default_for(DialectId::Postgres),
    }
}

/// A PostgreSQL document's snapshot.
pub fn doc(text: &str) -> DocSnapshot {
    SqlDocument::new(text.to_string(), pg18(), ClientFlavor::None).snapshot()
}

/// A caret at byte `at`.
pub fn caret(at: u32) -> CaretOrSelection {
    CaretOrSelection::Caret(TextSize::from(at))
}

// ## m3a-t23-exec

use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows};
use hacivat_driver::{Driver, RawError, RawErrorKind, ServerMessage};
use hacivat_exec::{
    ExecService, ExecUnit, LogMsg, Resolution, RunAction, RunRequest, RunStatus, RunTicket,
    SessionLog, resolve_units,
};
use hacivat_results::{FetchPolicy, MemoryBudget};
use hacivat_session::session::SessionHandle;
use hacivat_session::testing::fake_session;
use hacivat_values::{LogicalType, Value};
use sql_text::TextRange;

/// A document of `text` in `ctx` (psql conventions on the PostgreSQL family, mysql's on the
/// MySQL family).
pub fn doc_in(text: &str, ctx: DialectCtx) -> DocSnapshot {
    let flavor = match ctx.id {
        DialectId::Postgres | DialectId::PgWire(_) => ClientFlavor::Psql,
        DialectId::MySql | DialectId::MariaDb => ClientFlavor::MySqlCli,
        _ => ClientFlavor::None,
    };
    SqlDocument::new(text.to_string(), ctx, flavor).snapshot()
}

/// A PostgreSQL 18 document with psql conventions.
pub fn pg_doc(text: &str) -> DocSnapshot {
    doc_in(text, pg18())
}

/// A MySQL 8.4 document with the default `sql_mode`.
pub fn mysql_doc(text: &str) -> DocSnapshot {
    let version = sql_dialect::ServerVersion::new(8, 4, 0);
    doc_in(text, DialectCtx::new(DialectId::MySql, Some(version)))
}

/// A selection of bytes `r`.
pub fn sel(r: std::ops::Range<u32>) -> CaretOrSelection {
    CaretOrSelection::Selection(TextRange::new(
        TextSize::from(r.start),
        TextSize::from(r.end),
    ))
}

/// What Run `action` at `at` runs in the PostgreSQL document `text`.
pub fn resolve_doc(text: &str, at: CaretOrSelection, action: RunAction) -> Resolution {
    resolve_units(&pg_doc(text), &[at], action, None)
}

/// The units of `r`; panics naming `r` otherwise.
pub fn units(r: &Resolution) -> &[ExecUnit] {
    match r {
        Resolution::Units(u) => u,
        other => panic!("expected units, got {other:?}"),
    }
}

/// A server error with SQLSTATE `code` and the server's text `text`.
pub fn pg_error(code: &str, text: &str) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some(code.into()),
            vendor_code: None,
            severity: Some("ERROR".into()),
            text: text.into(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        }),
    }
}

fn id_cell(row: u64, _: usize) -> Value {
    Value::Int(row as i64)
}

/// `count` generated rows of one `id int8` column.
pub fn generated(count: u64) -> FakeResultSet {
    FakeResultSet {
        columns: vec![(
            "id",
            LogicalType::Int {
                bits: 64,
                signed: true,
            },
        )],
        rows: FakeRows::Generated {
            count,
            cell: id_cell,
        },
    }
}

/// A scripted query: `count` rows for the first text starting with `prefix`, answered after
/// 10 ms (on a paused clock: the run is seen running before it ends).
pub fn rows_of(prefix: &'static str, count: u64) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![generated(count)],
        first_delay: Duration::from_millis(10),
        ..Default::default()
    }
}

/// `SELECT …`'s script: `count` rows.
pub fn rows_q(count: u64) -> FakeQuery {
    rows_of("SELECT", count)
}

/// A PostgreSQL fake that follows `BEGIN`/`COMMIT`/`ROLLBACK` (a suspendable PostgreSQL query
/// needs a transaction block, P5).
pub fn fake_pg() -> FakeDriver {
    FakeDriver::new(DbmsKind::Postgres).tracks_transactions()
}

/// A run service over a console session of a fake, with the console's output log.
pub struct Bench {
    pub svc: ExecService,
    pub log: Arc<SessionLog>,
    pub engine: DbmsKind,
}

impl Bench {
    /// A request running `sqls` as units, in order, with `fetch`. A unit is suspendable when
    /// its text starts with `SELECT` (the tests' stand-in for `cursor_eligible`).
    pub fn req_with(&self, sqls: &[&str], fetch: FetchPolicy) -> RunRequest {
        let mut at = 0;
        let units = sqls
            .iter()
            .map(|s| {
                let u = ExecUnit {
                    text: (*s).into(),
                    doc_range: at..at + s.len(),
                    kind: sql_split::exec_range::UnitKind::Statement,
                    suspendable: s.starts_with("SELECT"),
                    // ## m3b-t16-run-service (named exception)
                    title: None,
                };
                at += s.len() + 2;
                u
            })
            .collect();
        RunRequest {
            console: 7,
            engine: self.engine,
            units,
            fetch,
            log: self.log.clone(),
            // ## m3b-t01c-core-seams: M3a's run (P-M3b-22).
            prepared: None,
            plan: None,
            policy: hacivat_session::session::ErrorPolicy::StopOnError,
            run_as_one: false,
            history: None,
            confirm: None,
            context_generation: None,
        }
    }

    pub fn req(&self, sqls: &[&str]) -> RunRequest {
        self.req_with(sqls, FetchPolicy::default())
    }

    /// The Fluent IDs of the console's log lines, oldest first.
    pub fn log_ids(&self) -> Vec<&'static str> {
        self.log.lines().iter().map(|l| l.msg.fluent_id()).collect()
    }

    pub fn log_msgs(&self) -> Vec<LogMsg> {
        self.log.lines().iter().map(|l| l.msg.clone()).collect()
    }
}

/// The run service over a console session of `fake` (`testing::fake_session`, P6): the bench,
/// the session and the fake.
pub async fn service_over(fake: FakeDriver) -> (Bench, SessionHandle, Arc<FakeDriver>) {
    let engine = fake.descriptor().kind;
    let (h, f) = fake_session(tokio::runtime::Handle::current(), fake, profile_of(engine)).await;
    let bench = Bench {
        svc: ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)),
        log: SessionLog::new(),
        engine,
    };
    (bench, h, f)
}

/// The name of unit `i`'s status.
pub fn unit_status(s: &RunStatus, i: usize) -> &'static str {
    s.units[i].status.name()
}

/// Unit 0's status names as the ticket publishes them, until the run ends (each publish the
/// test task sees; a repeat of the last name once).
pub async fn status_trail(t: &mut RunTicket) -> Vec<&'static str> {
    let first = unit_status(&t.status.borrow_and_update(), 0);
    let mut trail = vec![first];
    while t.status.borrow().summary.is_none() && t.status.changed().await.is_ok() {
        let name = unit_status(&t.status.borrow_and_update(), 0);
        if trail.last() != Some(&name) {
            trail.push(name);
        }
    }
    trail
}

/// Waits until `done` holds, sleeping 5 ms of the (paused) clock between looks.
pub async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..10_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never happened: {what}");
}

// ## m3a-f3-ci (fix round 1)
/// `h`'s snapshot once the session has left its request (CI run 43779216264). A run's `done`
/// follows its `RunEnded`, which the session sends while still `Busy` with the run's request;
/// it publishes `Idle` with its transaction and cursor just before it replies (`hacivat-session`
/// test `run_ended_comes_while_busy_and_the_reply_after_idle`). So a test that checks the state
/// a run left waits here on the snapshot watch (no polling) instead of reading it at once.
/// Panics after 10 s.
pub async fn settled(
    h: &hacivat_session::session::SessionHandle,
) -> hacivat_session::session::SessionSnapshot {
    use hacivat_session::session::SessionState;
    let mut state = h.state();
    let left = |s: &hacivat_session::session::SessionSnapshot| {
        !matches!(
            s.state,
            SessionState::Busy { .. } | SessionState::Cancelling { .. }
        )
    };
    match tokio::time::timeout(Duration::from_secs(10), state.wait_for(left)).await {
        Ok(Ok(snap)) => snap.clone(),
        Ok(Err(_)) => panic!("the session ended"),
        Err(_) => panic!("the session stayed busy: {:?}", h.state().borrow().clone()),
    }
}

// ## m3a-t31-matrix
// Task 31's helpers: consoles on the Docker matrix and on SQLite files, driven exactly as the
// app drives them (document → `resolve_units` → `ExecService` → session → typed driver), with
// an admin side channel that checks what a statement did. Docker only (`docker_m3a_matrix`).

pub mod matrix {
    //! - Lines come only from `hacivat_testkit::targets` (Q6): the server lines of the active
    //!   set plus one SQLite file per test. Every test needs a line of each engine family
    //!   (PostgreSQL, MySQL/MariaDB, SQLite) in any set; under `full` every major the engine
    //!   catalog tests must be there, and a target nobody classified fails ([`classify`]; the
    //!   M2b ruling: a target is never skipped silently).
    //! - Fixtures carry `hv_t31` names unique to the run and are dropped at the end (Q7).
    //! - Waits are on conditions, bounded by real time; failure messages carry line ids and
    //!   codes, never a password.

    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use hacivat_driver::{EngineCatalog, ExecOpts, Protocol, SemVerLoose, ServerMessage};
    use hacivat_exec::{
        ExecService, ExecUnit, ResultTab, RunAction, RunRequest, RunStatus, RunTicket, SessionLog,
        UnitStatus, resolve_units, sqlctx,
    };
    use hacivat_net::TlsMode;
    use hacivat_proto::{DbmsKind, Department, EnvTag};
    use hacivat_results::{FetchPolicy, MemoryBudget};
    use hacivat_secrets::{
        AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
        SecretString,
    };
    use hacivat_session::pipeline::{
        ApprovedRoots, ConnectDeps, ConnectRequest, Interaction, LiveConnection, Progress, Purpose,
        connect,
    };
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::session::{
        ExecOrigin, Request, RunSummary, SessionHandle, SessionSnapshot, SessionState,
    };
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
    use hacivat_testkit::{DockerTarget, TlsDefault, targets};
    use hacivat_values::Value;
    use sql_dialect::DialectCtx;
    use sql_split::DocSnapshot;
    use tokio::sync::watch;
    use tokio_util::sync::CancellationToken;

    /// Whether Docker tests run (`HACIVAT_DOCKER=1`, set by `cargo xtask docker test`).
    pub fn docker_enabled() -> bool {
        hacivat_testkit::docker_enabled()
    }

    pub fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    // -----------------------------------------------------------------------------------------
    // Lines

    /// The server engines of the matrix. A Docker target of any other engine is unclassified,
    /// and fails every test until someone classifies it.
    pub const SERVER_ENGINES: [DbmsKind; 3] =
        [DbmsKind::Postgres, DbmsKind::MySql, DbmsKind::MariaDb];

    /// How the Docker targets of a set split, by their properties (fix round 1, review I2).
    #[derive(Debug, Default)]
    pub struct Classified {
        /// One target per engine and major at or above the engine's floor: the matrix.
        pub lines: Vec<DockerTarget>,
        /// A second target of the same engine and major (`-tls`, `-auth`, `-tr`, `-x509`):
        /// its id extends its line's id. Twins are lines of their own tests.
        pub twins: Vec<DockerTarget>,
        /// Majors below the catalog's floor (pg13): not supported, so not a line.
        pub below_floor: Vec<DockerTarget>,
    }

    /// `"10.11"` as `(10, 11)`.
    fn major_key(major: &str) -> (u32, u32) {
        let mut parts = major.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
    }

    /// Splits `all` (the active set's targets) into lines, twins and below-floor targets.
    /// Every target must land in one of them: one of another engine, or a second target of an
    /// engine and major whose id does not extend the line's, is an error naming it.
    pub fn classify(all: &[DockerTarget], catalog: &EngineCatalog) -> Result<Classified, String> {
        let mut out = Classified::default();
        for t in all {
            if !SERVER_ENGINES.contains(&t.engine) {
                return Err(format!("{}: engine {:?} is not classified", t.id, t.engine));
            }
        }
        for engine in SERVER_ENGINES {
            let floor = catalog
                .get(engine)
                .and_then(|s| s.floor.as_deref())
                .map(major_key)
                .ok_or_else(|| format!("{engine:?}: the catalog has no floor"))?;
            let mut of_engine: Vec<&DockerTarget> =
                all.iter().filter(|t| t.engine == engine).collect();
            // The base of a major is its shortest id; the others must extend it.
            of_engine.sort_by_key(|t| (major_key(&t.major), t.id.len()));
            let mut base: Option<&DockerTarget> = None;
            for t in of_engine {
                if major_key(&t.major) < floor {
                    out.below_floor.push(t.clone());
                    continue;
                }
                match base {
                    Some(b) if b.major == t.major => {
                        // A twin's id is its line's id and a `-variant` suffix.
                        let extends =
                            t.id.strip_prefix(b.id.as_str())
                                .is_some_and(|rest| rest.len() > 1 && rest.starts_with('-'));
                        if !extends {
                            return Err(format!(
                                "{}: a second {engine:?} {} target that is no twin of {}",
                                t.id, t.major, b.id
                            ));
                        }
                        out.twins.push(t.clone());
                    }
                    _ => {
                        out.lines.push(t.clone());
                        base = Some(t);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Whether the lines hold every tested major of each server engine (the catalog's
    /// floor-to-latest list); the missing ones otherwise.
    pub fn missing_tested(lines: &[DockerTarget], catalog: &EngineCatalog) -> Vec<String> {
        let mut missing = Vec::new();
        for engine in SERVER_ENGINES {
            for major in catalog
                .get(engine)
                .map(|s| s.tested.clone())
                .unwrap_or_default()
            {
                if !lines.iter().any(|t| t.engine == engine && t.major == major) {
                    missing.push(format!("{engine:?} {major}"));
                }
            }
        }
        missing
    }

    fn active_set() -> Option<String> {
        std::env::var("HACIVAT_DOCKER_SET").ok()
    }

    /// Whether every named line must exist: under `full`, or with no set (every target).
    pub fn must_exist() -> bool {
        active_set().is_none_or(|s| s == "full")
    }

    /// One line of the matrix: a Docker server, or a SQLite file of its own.
    #[derive(Debug, Clone)]
    pub struct Line {
        pub id: String,
        pub engine: DbmsKind,
        pub target: Option<DockerTarget>,
        sqlite: Option<Arc<tempfile::TempDir>>,
    }

    impl Line {
        pub fn is_pg(&self) -> bool {
            self.engine == DbmsKind::Postgres
        }

        pub fn is_mysql_family(&self) -> bool {
            matches!(self.engine, DbmsKind::MySql | DbmsKind::MariaDb)
        }

        pub fn is_sqlite(&self) -> bool {
            self.engine == DbmsKind::Sqlite
        }

        /// The SQLite line's database file.
        pub fn sqlite_path(&self) -> PathBuf {
            self.sqlite
                .as_ref()
                .expect("a SQLite line")
                .path()
                .join("hv_t31.db")
        }

        pub fn target(&self) -> &DockerTarget {
            self.target.as_ref().expect("a server line")
        }
    }

    /// The server lines of the active set, from `targets(Some(engine))` per server engine
    /// ([`classify`]), checked: PostgreSQL and the MySQL family each have one in any set; under
    /// `full` every major the catalog tests is there; an unclassified target fails.
    pub fn server_lines() -> Vec<Line> {
        let all: Vec<DockerTarget> = SERVER_ENGINES
            .iter()
            .flat_map(|e| targets(Some(*e)))
            .collect();
        // The reverse check: no target of the set is left out unseen.
        let others: Vec<String> = targets(None)
            .into_iter()
            .filter(|t| !all.contains(t))
            .map(|t| t.id)
            .collect();
        assert!(others.is_empty(), "unclassified Docker targets: {others:?}");
        let catalog = EngineCatalog::embedded();
        let found = classify(&all, catalog)
            .unwrap_or_else(|e| panic!("{e}"))
            .lines;
        if must_exist() {
            let missing = missing_tested(&found, catalog);
            assert!(
                missing.is_empty(),
                "the full set has no line for {missing:?}"
            );
        }
        assert!(
            found.iter().any(|t| t.engine == DbmsKind::Postgres),
            "the active set has no PostgreSQL line"
        );
        assert!(
            found
                .iter()
                .any(|t| matches!(t.engine, DbmsKind::MySql | DbmsKind::MariaDb)),
            "the active set has no MySQL or MariaDB line"
        );
        found.into_iter().map(line_of).collect()
    }

    /// The line of server target `t`.
    pub fn line_of(t: DockerTarget) -> Line {
        Line {
            id: t.id.clone(),
            engine: t.engine,
            target: Some(t),
            sqlite: None,
        }
    }

    /// A SQLite line on a fresh file (no server: in every set).
    pub fn sqlite_line() -> Line {
        Line {
            id: "sqlite".into(),
            engine: DbmsKind::Sqlite,
            target: None,
            sqlite: Some(Arc::new(
                tempfile::tempdir().expect("a temporary directory"),
            )),
        }
    }

    /// Every line of the active set: the server lines, then SQLite.
    pub fn lines() -> Vec<Line> {
        let mut all = server_lines();
        all.push(sqlite_line());
        all
    }

    /// The PostgreSQL server lines of the active set.
    pub fn pg_lines() -> Vec<Line> {
        server_lines().into_iter().filter(Line::is_pg).collect()
    }

    // -----------------------------------------------------------------------------------------
    // Connecting

    /// The real drivers, a memory secret store, no credential files, no SSH.
    pub fn deps() -> ConnectDeps {
        let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
        let known_hosts = hacivat_net::ssh::KnownHosts::load(
            Vec::new(),
            PathBuf::from("/nonexistent/hacivat/known_hosts"),
        );
        let tunnels = hacivat_net::ssh::TunnelManager::new(known_hosts.clone(), Default::default());
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
            tunnels,
            known_hosts,
        }
    }

    /// `Require` where the server offers a self-signed certificate, `Auto` elsewhere.
    fn tls_mode(t: &DockerTarget) -> TlsMode {
        match t.tls_default {
            TlsDefault::SelfSigned => TlsMode::Require,
            TlsDefault::None => TlsMode::Auto,
        }
    }

    /// The profile of `line`: user and database `hv_test` on the server lines (password
    /// sign-in that keeps nothing), the file on the SQLite line; a unique `application_name`.
    pub fn profile_of(line: &Line) -> Profile {
        let mut p = match &line.target {
            Some(t) => {
                let mut p = Profile {
                    name: format!("t31 {}", t.id),
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
                p.tls.mode = tls_mode(t);
                p
            }
            None => Profile {
                name: "t31 sqlite".into(),
                engine: DbmsKind::Sqlite,
                endpoint: ProfileEndpoint::File {
                    path: line.sqlite_path(),
                    create_if_missing: true,
                },
                user: None,
                auth: AuthProviderId::None,
                max_connections: 1,
                ..Profile::default()
            },
        };
        p.application_name = unique("hv_t31_app");
        p
    }

    fn password(line: &Line) -> Option<SecretString> {
        line.target
            .as_ref()
            .map(|t| SecretString::from(t.password.clone()))
    }

    fn request(line: &Line, p: Profile) -> ConnectRequest {
        ConnectRequest {
            read_only: p.read_only,
            profile: Arc::new(p),
            draft_password: password(line),
            purpose: Purpose::Main,
            department: Department::Console,
            prior_rejections: 0,
            draft_ssh: Vec::new(),
            overlay_secrets: None,
            plaintext_consent: false,
        }
    }

    fn progress() -> watch::Sender<Progress> {
        watch::Sender::new(Progress {
            trail: Vec::new(),
            current: None,
            started: Instant::now(),
        })
    }

    // -----------------------------------------------------------------------------------------
    // The admin side channel

    /// The test account, read-write and direct, on `line`'s own database: the fixtures' admin
    /// and the side channel that checks what a console did. It first sweeps the `hv_t31_*`
    /// objects another process left behind (a red run's fixtures; Q7 across runs, review M9).
    pub async fn admin(line: &Line) -> Side {
        let mut a = Side::open(line, None).await;
        a.sweep(line).await;
        a
    }

    /// A direct connection that is not a console.
    pub struct Side {
        id: String,
        live: Option<LiveConnection>,
    }

    impl Side {
        /// [`admin`], on database `db` when given (server lines).
        pub async fn open(line: &Line, db: Option<&str>) -> Side {
            let mut p = profile_of(line);
            if let Some(db) = db {
                p.database = Some(db.to_string());
            }
            p.allow_unsupported_server = true;
            let live = connect(
                &deps(),
                request(line, p),
                Interaction::Headless,
                CancellationToken::new(),
                progress(),
            )
            .await
            .unwrap_or_else(|e| panic!("{} admin: {:?}", line.id, e.class));
            Side {
                id: line.id.clone(),
                live: Some(live),
            }
        }

        fn live(&mut self) -> &mut LiveConnection {
            self.live.as_mut().expect("open")
        }

        /// The first value of the first row of `sql` (simple protocol), `None` for NULL or
        /// no row.
        pub async fn value(&mut self, sql: &str) -> Option<String> {
            let id = self.id.clone();
            let answer = self
                .live()
                .raw()
                .execute(
                    sql,
                    ExecOpts {
                        max_rows: 1000,
                        protocol: Protocol::Simple,
                    },
                )
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "{id}: `{sql}`: {:?} {:?}",
                        e.kind,
                        code_of(e.server.as_ref())
                    )
                });
            answer
                .rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .flatten()
        }

        /// The first column of every row of `sql` (NULL as `""`).
        pub async fn column(&mut self, sql: &str) -> Vec<String> {
            let id = self.id.clone();
            let answer = self
                .live()
                .raw()
                .execute(
                    sql,
                    ExecOpts {
                        max_rows: 10_000,
                        protocol: Protocol::Simple,
                    },
                )
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "{id}: `{sql}`: {:?} {:?}",
                        e.kind,
                        code_of(e.server.as_ref())
                    )
                });
            answer
                .rows
                .into_iter()
                .map(|r| r.into_iter().next().flatten().unwrap_or_default())
                .collect()
        }

        /// Runs `sql` as a batch; the failure's code (or kind) when it fails.
        pub async fn try_batch(&mut self, sql: &str) -> Result<(), String> {
            self.live()
                .raw()
                .batch(sql)
                .await
                .map_err(|e| code_of(e.server.as_ref()).unwrap_or_else(|| format!("{:?}", e.kind)))
        }

        /// The server's message for `sql`, which must fail.
        pub async fn error_of(&mut self, sql: &str) -> ServerMessage {
            let id = self.id.clone();
            let answer = self
                .live()
                .raw()
                .execute(
                    sql,
                    ExecOpts {
                        max_rows: 10,
                        protocol: Protocol::Simple,
                    },
                )
                .await;
            match answer {
                Ok(_) => panic!("{id}: `{sql}` ran; an error was expected"),
                Err(e) => e
                    .server
                    .unwrap_or_else(|| panic!("{id}: `{sql}`: {:?} without a message", e.kind)),
            }
        }

        /// Drops the `hv_t31_*` objects of `line` that another process made: their names carry
        /// that process's id ([`unique`]), so this run's own fixtures are never touched.
        pub async fn sweep(&mut self, line: &Line) {
            let mine = format!("\\_{}\\_", std::process::id());
            let drops: Vec<String> = match line.engine {
                DbmsKind::Postgres => {
                    let found = self
                        .column(&format!(
                            "SELECT kind || ' ' || name FROM ( \
                             SELECT 1 AS o, 'ROUTINE' AS kind, proname::text AS name \
                               FROM pg_proc WHERE pronamespace = 'public'::regnamespace \
                             UNION ALL SELECT 2, 'TABLE', relname::text FROM pg_class \
                               WHERE relkind = 'r' AND relnamespace = 'public'::regnamespace \
                             UNION ALL SELECT 3, 'SEQUENCE', relname::text FROM pg_class \
                               WHERE relkind = 'S' AND relnamespace = 'public'::regnamespace \
                             UNION ALL SELECT 4, 'TYPE', typname::text FROM pg_type \
                               WHERE typtype = 'e' AND typnamespace = 'public'::regnamespace \
                             UNION ALL SELECT 5, 'COLLATION', collname::text FROM pg_collation \
                               WHERE collnamespace = 'public'::regnamespace) x \
                             WHERE name LIKE 'hv\\_t31\\_%' AND name NOT LIKE '%{mine}%' \
                             ORDER BY o, name"
                        ))
                        .await;
                    found
                        .into_iter()
                        .map(|f| {
                            let (kind, name) = f.split_once(' ').unwrap_or_default();
                            let cascade = if kind == "COLLATION" { "" } else { " CASCADE" };
                            format!("DROP {kind} IF EXISTS {name}{cascade}")
                        })
                        .collect()
                }
                DbmsKind::MySql | DbmsKind::MariaDb => self
                    .column(&format!(
                        "SELECT table_name FROM information_schema.tables \
                         WHERE table_schema = DATABASE() AND table_name LIKE 'hv\\_t31\\_%' \
                         AND table_name NOT LIKE '%{mine}%'"
                    ))
                    .await
                    .into_iter()
                    .map(|name| format!("DROP TABLE IF EXISTS {name}"))
                    .collect(),
                // A SQLite line is a fresh file per test.
                _ => Vec::new(),
            };
            for sql in drops {
                self.batch(&sql).await;
            }
        }

        /// The first value of the first row of `sql`, `""` for NULL or no row.
        pub async fn scalar(&mut self, sql: &str) -> String {
            self.value(sql).await.unwrap_or_default()
        }

        /// `sql`'s single integer.
        pub async fn count(&mut self, sql: &str) -> i64 {
            let v = self.scalar(sql).await;
            v.parse()
                .unwrap_or_else(|_| panic!("{}: `{sql}`: `{v}` is not an integer", self.id))
        }

        /// Runs fixture text. A failure names the statement up to its first literal and the
        /// server's code.
        pub async fn batch(&mut self, sql: &str) {
            let id = self.id.clone();
            if let Err(e) = self.live().raw().batch(sql).await {
                let shown = sql.split('\'').next().unwrap_or_default();
                panic!(
                    "{id}: fixture `{shown}…`: {:?} {:?}",
                    e.kind,
                    code_of(e.server.as_ref())
                );
            }
        }

        pub async fn close(mut self) {
            if let Some(live) = self.live.take() {
                live.close().await;
            }
        }
    }

    /// A server message's code: MySQL's vendor code, else the SQLSTATE, else none.
    pub fn code_of(m: Option<&ServerMessage>) -> Option<String> {
        let m = m?;
        match (m.vendor_code, &m.sqlstate) {
            (Some(code), _) => Some(code.to_string()),
            (None, Some(state)) => Some(state.clone()),
            (None, None) => None,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Consoles

    /// How a console is opened.
    #[derive(Debug, Clone, Default)]
    pub struct Opts {
        pub read_only: bool,
        pub env: Option<EnvTag>,
        /// The app's boot policy (R-M3-29): production profiles get read-only sessions.
        pub production_policy: bool,
        pub user: Option<String>,
        pub database: Option<String>,
    }

    impl Opts {
        /// A development console under the app's production policy (the app's boot value).
        pub fn dev() -> Opts {
            Opts {
                env: Some(EnvTag::Dev),
                production_policy: true,
                ..Opts::default()
            }
        }
    }

    /// A console: its profile connected on its own manager, its default session, the run
    /// service and the console's output log, and the SQL context the console read at bind
    /// (R-M3-39).
    pub struct Console {
        pub line: Line,
        pub m: Arc<ConnectionManager>,
        pub profile: Arc<Profile>,
        pub s: SessionHandle,
        pub svc: ExecService,
        pub log: Arc<SessionLog>,
        pub ctx: DialectCtx,
        pub version: Option<SemVerLoose>,
        console: u64,
    }

    static CONSOLES: AtomicU64 = AtomicU64::new(1);

    impl Console {
        /// A development console on `line`.
        pub async fn dev(line: &Line) -> Console {
            Console::open(line, Opts::dev()).await
        }

        pub async fn open(line: &Line, opts: Opts) -> Console {
            let mut p = profile_of(line);
            p.read_only = opts.read_only;
            p.environment = opts.env;
            if opts.read_only
                && let ProfileEndpoint::File {
                    create_if_missing, ..
                } = &mut p.endpoint
            {
                // A read-only SQLite profile never creates its file (the fixture made it).
                *create_if_missing = false;
            }
            if let Some(user) = opts.user {
                p.user = Some(user);
            }
            if let Some(db) = opts.database {
                p.database = Some(db);
            }
            let config = ManagerConfig {
                production_read_only: opts.production_policy,
                ..ManagerConfig::m2b()
            };
            let m =
                ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps()), config);
            let profile = Arc::new(p);
            m.connect(profile.clone(), password(line), Interaction::Headless);
            let info = wait_connected(&m, &profile, &line.id).await;
            let s = m
                .sessions(profile.id)
                .expect("the profile is connected")
                .default_session()
                .expect("it has a default session");
            let version = info.server.version.clone();
            let mut console = Console {
                line: line.clone(),
                m,
                profile,
                s,
                svc: ExecService::new(MemoryBudget::new(4 << 30, 256 << 20)),
                log: SessionLog::new(),
                ctx: sqlctx::dialect_ctx(line.engine, version.as_ref(), None),
                version,
                console: CONSOLES.fetch_add(1, Ordering::SeqCst),
            };
            console.reprobe().await;
            console
        }

        /// Reads the session's lexing modes, as the app does when a console binds (R-M3-39).
        pub async fn reprobe(&mut self) {
            let Some(sql) = sqlctx::probe_sql(self.line.engine) else {
                return;
            };
            let row = self.internal(sql).await;
            self.ctx = sqlctx::dialect_ctx(self.line.engine, self.version.as_ref(), row.as_deref());
        }

        /// An internal, idempotent statement's first value (the probe, a backend id).
        pub async fn internal(&self, sql: &str) -> Option<String> {
            let answer = self
                .s
                .submit(Request::Exec {
                    sql: sql.to_string(),
                    opts: ExecOpts {
                        max_rows: 10,
                        protocol: Protocol::Simple,
                    },
                    origin: ExecOrigin::Internal { idempotent: true },
                })
                .reply
                .await
                .expect("the actor replies")
                .unwrap_or_else(|e| panic!("{}: `{sql}`: {:?}", self.line.id, e.kind));
            answer
                .rows
                .first()
                .and_then(|r| r.first())
                .cloned()
                .flatten()
        }

        /// The server-side id of the console's connection (PostgreSQL, MySQL family).
        pub async fn backend_id(&self) -> String {
            let sql = match self.line.engine {
                DbmsKind::Postgres => "SELECT pg_backend_pid()",
                _ => "SELECT CONNECTION_ID()",
            };
            self.internal(sql).await.expect("a backend id")
        }

        /// The console's document of `text` (psql conventions on PostgreSQL, mysql's on the
        /// MySQL family).
        pub fn doc(&self, text: &str) -> DocSnapshot {
            super::doc_in(text, self.ctx)
        }

        /// The units Run `action` with the caret at byte 0 sends for `text`; panics
        /// otherwise.
        pub fn units(&self, text: &str, action: RunAction) -> Vec<ExecUnit> {
            match resolve_units(&self.doc(text), &[super::caret(0)], action, None) {
                hacivat_exec::Resolution::Units(units) => units,
                other => panic!("{}: `{text}` resolved to {other:?}", self.line.id),
            }
        }

        /// Runs the whole script `text`, or says why Run sends nothing for it (the
        /// resolution: `nothing-to-run`, `uncertain`, `refused:<message>`, `ask`,
        /// `client-command`).
        pub async fn try_run(&self, text: &str) -> Result<Ran, String> {
            use hacivat_exec::Resolution as R;
            match resolve_units(
                &self.doc(text),
                &[super::caret(0)],
                RunAction::Execute3,
                None,
            ) {
                R::Units(units) => Ok(ended(self.run_units(units, FetchPolicy::default())).await),
                R::NothingToRun => Err("nothing-to-run".into()),
                R::Uncertain(_) => Err("uncertain".into()),
                R::Refused { reason, .. } => Err(format!("refused:{}", reason.fluent_id())),
                R::Ask { .. } => Err("ask".into()),
                R::UnsupportedClientCommand { .. } => Err("client-command".into()),
            }
        }

        /// Starts a run of `units` with `fetch`.
        pub fn run_units(&self, units: Vec<ExecUnit>, fetch: FetchPolicy) -> RunTicket {
            self.svc.run(
                &self.s,
                RunRequest {
                    console: self.console,
                    engine: self.line.engine,
                    units,
                    fetch,
                    log: self.log.clone(),
                    // ## m3b-t01c-core-seams: M3a's run (P-M3b-22).
                    prepared: None,
                    plan: None,
                    policy: hacivat_session::session::ErrorPolicy::StopOnError,
                    run_as_one: false,
                    history: None,
                    confirm: None,
                    context_generation: None,
                },
            )
        }

        /// Starts Run Execute3 (the whole script) of `text` with `fetch`.
        pub fn start_with(&self, text: &str, fetch: FetchPolicy) -> RunTicket {
            let units = self.units(text, RunAction::Execute3);
            self.svc.run(
                &self.s,
                RunRequest {
                    console: self.console,
                    engine: self.line.engine,
                    units,
                    fetch,
                    log: self.log.clone(),
                    // ## m3b-t01c-core-seams: M3a's run (P-M3b-22).
                    prepared: None,
                    plan: None,
                    policy: hacivat_session::session::ErrorPolicy::StopOnError,
                    run_as_one: false,
                    history: None,
                    confirm: None,
                    context_generation: None,
                },
            )
        }

        pub fn start(&self, text: &str) -> RunTicket {
            self.start_with(text, FetchPolicy::default())
        }

        /// Runs the whole script `text` and waits for the run to end.
        pub async fn run_with(&self, text: &str, fetch: FetchPolicy) -> Ran {
            ended(self.start_with(text, fetch)).await
        }

        pub async fn run(&self, text: &str) -> Ran {
            self.run_with(text, FetchPolicy::default()).await
        }

        /// Runs `text`; every unit must succeed (or stay suspended).
        pub async fn ok(&self, text: &str) -> Ran {
            let ran = self.run(text).await;
            assert!(
                ran.status.units.iter().all(|u| matches!(
                    u.status,
                    UnitStatus::Succeeded(_) | UnitStatus::Suspended { .. }
                )),
                "{}: `{text}`: {:?} {}",
                self.line.id,
                ran.names(),
                ran.outcome()
            );
            ran
        }

        pub fn snap(&self) -> SessionSnapshot {
            self.s.state().borrow().clone()
        }

        /// The session's snapshot once it has left its request (CI run 43779216264). A run's
        /// `done` follows its `RunEnded`, which the session sends while still `Busy` with the
        /// run's request; it publishes `Idle` with its transaction and cursor just before it
        /// replies (`hacivat-session` test `run_ended_comes_while_busy_and_the_reply_after_idle`).
        /// So a test that checks the state a run left waits here on the snapshot watch (no
        /// polling) instead of reading [`snap`](Self::snap) at once. Panics after 10 s.
        pub async fn settled(&self) -> SessionSnapshot {
            let mut state = self.s.state();
            let left = |s: &SessionSnapshot| {
                !matches!(
                    s.state,
                    SessionState::Busy { .. } | SessionState::Cancelling { .. }
                )
            };
            match tokio::time::timeout(secs(10), state.wait_for(left)).await {
                Ok(Ok(snap)) => snap.clone(),
                Ok(Err(_)) => panic!("{}: the session ended", self.line.id),
                Err(_) => panic!(
                    "{}: the session stayed busy: {:?}",
                    self.line.id,
                    self.snap()
                ),
            }
        }

        /// The Fluent IDs of the console's log lines, oldest first.
        pub fn log_ids(&self) -> Vec<&'static str> {
            self.log.lines().iter().map(|l| l.msg.fluent_id()).collect()
        }

        /// The request the session is busy with (`None` when idle or releasing an idle
        /// cursor, which is request 0).
        pub fn busy_request(&self) -> Option<u64> {
            match self.snap().state {
                SessionState::Busy { req, .. } if req != 0 => Some(req),
                _ => None,
            }
        }

        pub async fn shutdown(self) {
            self.m.shutdown().await;
        }

        /// After a gap or a lost connection discarded the session's connection: waits until
        /// the profile is connected again (a reconnect, else a fresh connect) and takes its
        /// default session, so the next statement starts on a live session.
        /// (Task 22's sequence: the profile leaves `Connected`, then settles.)
        pub async fn recover(&mut self) {
            let (m, id, line) = (&self.m, self.profile.id, &self.line.id);
            wait_state(m, id, line, secs(10), |s| {
                !matches!(s, LiveState::Connected(_))
            })
            .await;
            let state = wait_state(m, id, line, secs(60), |s| {
                !matches!(s, LiveState::Reconnecting { .. } | LiveState::Connecting(_))
            })
            .await;
            if !matches!(state, LiveState::Connected(_)) {
                m.connect(
                    self.profile.clone(),
                    password(&self.line),
                    Interaction::Headless,
                );
                wait_connected(m, &self.profile, line).await;
            }
            self.s = m
                .sessions(id)
                .expect("the profile is connected")
                .default_session()
                .expect("it has a default session");
            self.reprobe().await;
        }
    }

    /// Waits until the profile's state matches `what`, for at most `limit`.
    async fn wait_state(
        m: &ConnectionManager,
        id: hacivat_proto::ProfileId,
        line: &str,
        limit: Duration,
        what: impl Fn(&LiveState) -> bool,
    ) -> LiveState {
        let mut changes = m.changes();
        let deadline = Instant::now() + limit;
        loop {
            let state = m.state(id);
            if what(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "{line}: the state stayed {state:?}"
            );
            tokio::select! {
                _ = changes.changed() => {}
                () = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    }

    async fn wait_connected(
        m: &ConnectionManager,
        p: &Profile,
        id: &str,
    ) -> Arc<hacivat_session::LiveInfo> {
        let mut changes = m.changes();
        let deadline = Instant::now() + secs(60);
        loop {
            match m.state(p.id) {
                LiveState::Connected(info) => return info,
                LiveState::Failed(e) => panic!("{id}: the connect failed: {:?}", e.class),
                LiveState::Lost { class } => panic!("{id}: lost: {class:?}"),
                _ => {}
            }
            assert!(Instant::now() < deadline, "{id}: never connected");
            tokio::select! {
                _ = changes.changed() => {}
                () = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
        }
    }

    /// A finished run: its status and result tabs when it ended, its summary, and the
    /// watches (a suspended unit follows its result after the run).
    #[derive(Debug)]
    pub struct Ran {
        pub status: RunStatus,
        pub tabs: Arc<[ResultTab]>,
        pub summary: RunSummary,
        pub live: watch::Receiver<RunStatus>,
    }

    impl Ran {
        /// Each unit's status name.
        pub fn names(&self) -> Vec<&'static str> {
            self.status.units.iter().map(|u| u.status.name()).collect()
        }

        pub fn unit(&self, i: usize) -> &UnitStatus {
            &self.status.units[i].status
        }

        /// Unit `i`'s status now.
        pub fn unit_now(&self, i: usize) -> UnitStatus {
            self.live.borrow().units[i].status.clone()
        }

        pub fn tab(&self, i: usize) -> &ResultTab {
            &self.tabs[i]
        }

        /// How the run ended, as the bypass rows judge it: the first failed unit's code (the
        /// SQLSTATE or vendor code) or its read-only layer (`1a`, `protocol`, `function`,
        /// `input`, `engine`), `gap`, another status's name, or `ran` when every unit
        /// succeeded.
        pub fn outcome(&self) -> String {
            for u in self.status.units.iter() {
                match &u.status {
                    UnitStatus::Failed(view) => {
                        if let Some(code) = &view.code {
                            return code.to_string();
                        }
                        return match view.title_id {
                            "exec-deny-read-only-transaction" => "1a",
                            "exec-deny-read-only-protocol" => "protocol",
                            "exec-deny-read-only-function" => "function",
                            "exec-deny-read-only-input" => "input",
                            "exec-deny-read-only-engine" => "engine",
                            "exec-error-enforcement-gap" => "gap",
                            other => other,
                        }
                        .to_string();
                    }
                    UnitStatus::Succeeded(_) | UnitStatus::Suspended { .. } => {}
                    other => return other.name().to_string(),
                }
            }
            "ran".into()
        }
    }

    /// Waits for `t`'s run to end.
    pub async fn ended(t: RunTicket) -> Ran {
        let (live, results) = (t.status.clone(), t.results.clone());
        let summary = t.done.await.expect("the run ends");
        let status = live.borrow().clone();
        let tabs = results.borrow().clone();
        Ran {
            status,
            tabs,
            summary,
            live,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Values and waits

    /// Cell (`row`, `col`) of `tab`'s result now.
    pub fn cell(tab: &ResultTab, row: u64, col: u32) -> Value {
        tab.source.snapshot().cell(row, col).to_value()
    }

    /// Values compare by bits for floats (`-0.0` ≠ `0.0`), and NaN equals NaN.
    pub fn same(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::F64(x), Value::F64(y)) => {
                x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
            }
            (Value::F32(x), Value::F32(y)) => {
                x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
            }
            _ => a == b,
        }
    }

    /// A name unique to this process and call (`[a-z0-9_]`).
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
        let deadline = Instant::now() + limit;
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits (real time, 50 ms steps) until the side channel counts `want` for `sql`, for at
    /// most `limit`.
    pub async fn wait_count(a: &mut Side, what: &str, limit: Duration, sql: &str, want: i64) {
        let deadline = Instant::now() + limit;
        loop {
            let n = a.count(sql).await;
            if n == want {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what} (the count is {n})"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // -----------------------------------------------------------------------------------------
    // The record

    /// The engine × version × case table: one row per line and case, written to the Docker
    /// artifact `t31-<test>.tsv` (ADR 0006's table is built from them).
    #[derive(Debug, Default)]
    pub struct Record {
        rows: std::sync::Mutex<Vec<String>>,
    }

    impl Record {
        pub fn new() -> Record {
            Record::default()
        }

        pub fn add(&self, line: &Line, version: Option<&SemVerLoose>, case: &str, outcome: &str) {
            let v = version.map_or_else(String::new, |v| {
                format!("{}.{}.{}", v.major, v.minor, v.patch)
            });
            self.rows
                .lock()
                .unwrap()
                .push(format!("{}\t{v}\t{case}\t{outcome}", line.id));
        }

        /// Writes the artifact; a test that recorded nothing fails (no line ran).
        pub fn write(&self, test: &str) {
            let rows = self.rows.lock().unwrap();
            assert!(!rows.is_empty(), "{test}: no line of the active set ran");
            let path = hacivat_testkit::artifact(&format!("t31-{test}.tsv"));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).expect("the artifact directory");
            }
            std::fs::write(&path, rows.join("\n") + "\n").expect("the record is written");
        }

        /// The line ids recorded, in order, without repeats.
        pub fn lines(&self) -> Vec<String> {
            let mut ids: Vec<String> = Vec::new();
            for row in self.rows.lock().unwrap().iter() {
                let id = row.split('\t').next().unwrap_or_default().to_string();
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids
        }
    }

    /// Asserts that `record` holds every line of `expected`, in order: no line was skipped.
    pub fn assert_every_line_ran(record: &Record, expected: &[Line]) {
        let want: Vec<String> = expected.iter().map(|l| l.id.clone()).collect();
        assert_eq!(record.lines(), want, "a line of the active set did not run");
    }

    /// Whether `v` is the server line `major` names (`17`, `8.4`, `10.11`).
    pub fn version_matches(major: &str, v: &SemVerLoose) -> bool {
        match major.split_once('.') {
            Some((a, b)) => a == v.major.to_string() && b == v.minor.to_string(),
            None => major == v.major.to_string(),
        }
    }
}

// ## m3b-t16-run-service
// Task 16's helpers: M3b requests (prepared units with a gate plan the user answered) over the
// fake console session, without Task 15's `prepare_units`/`gate` bodies (P-M3b-16: a plan is
// built here item by item).

pub mod m3b {
    use std::sync::Arc;

    use hacivat_exec::{
        ErrorPolicy, ExecUnit, GateClass, GateDecision, GateItem, GatePlan, OffsetMap,
        PreparedUnit, RunRequest,
    };
    use hacivat_session::session::{SessionEffects, UnitVerdict};
    use hacivat_values::{BindValue, LogicalType};
    use sql_split::exec_range::UnitKind;

    use super::Bench;

    /// A unit of `text` at `start`, titled `title`.
    pub fn unit(text: &str, start: usize, title: Option<&str>) -> ExecUnit {
        ExecUnit {
            text: text.into(),
            doc_range: start..start + text.len(),
            kind: UnitKind::Statement,
            suspendable: false,
            title: title.map(Arc::from),
        }
    }

    /// Units of `texts`, two bytes apart, untitled.
    pub fn units(texts: &[&str]) -> Vec<ExecUnit> {
        let mut at = 0;
        texts
            .iter()
            .map(|t| {
                let u = unit(t, at, None);
                at += t.len() + 2;
                u
            })
            .collect()
    }

    /// `unit` prepared as written: no binds, its own text masked.
    pub fn prepared(unit: &ExecUnit) -> PreparedUnit {
        PreparedUnit {
            unit: unit.clone(),
            final_text: unit.text.clone(),
            binds: Arc::from(Vec::new()),
            masked: unit.text.clone(),
            offsets: OffsetMap::identity(unit.doc_range.start),
        }
    }

    /// A text bind value.
    pub fn text_bind(s: &str) -> BindValue {
        BindValue::Typed {
            hint: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            text: s.into(),
        }
    }

    /// A gate item for unit `i` deciding `decision`.
    pub fn item(i: u32, decision: GateDecision) -> GateItem {
        GateItem {
            unit: i,
            decision,
            verdict: UnitVerdict::Unknown,
            effects: SessionEffects::empty(),
            class: GateClass::Read,
            fingerprint: 0,
        }
    }

    /// A plan that runs every one of `n` units.
    pub fn run_all(n: usize) -> GatePlan {
        GatePlan {
            items: (0..n)
                .map(|i| item(u32::try_from(i).unwrap(), GateDecision::Run))
                .collect(),
        }
    }

    /// An M3b request of `units` prepared as written, run by `plan`.
    pub fn request(b: &Bench, units: Vec<ExecUnit>, plan: GatePlan) -> RunRequest {
        let prepared = units.iter().map(prepared).collect();
        RunRequest {
            units,
            prepared: Some(prepared),
            plan: Some(plan),
            policy: ErrorPolicy::StopOnError,
            ..b.req(&[])
        }
    }
}

// ## m3b-t15-gate

/// Task 15's report builders (P-M3b-16): the gate's tests build `SafetyReport`s by hand over
/// the exact final text (dialect and `set_targets` set, as `classify` sets them), so the gate
/// is tested without Task 14's classifier. `fake_stmt` reads a statement's first words the way
/// these tests need; it is no classifier and nothing outside these tests uses it.
pub mod gate_fx {
    use std::sync::Arc;

    use hacivat_driver::TxStatus;
    use hacivat_exec::{
        ConnSafety, ExecUnit, GateDecision, GateItem, ParamAnswers, ParamValueStore, Prepared,
        PreparedUnit, SafetyReport, SuppressSet, gate, prepare_units, prepare_units_with,
    };
    use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
    use hacivat_session::session::{TxMode, TxOwner};
    use hacivat_values::{BindValue, LogicalType};
    use sql_dialect::{DialectCtx, DialectId};
    use sql_ident::parse_qualified;
    use sql_safety::effects::class_of;
    use sql_safety::{
        Confidence, DestructiveKind, DestructiveReason, EffectSet, ObjectKind, ObjectRef, Reason,
        ReasonCode, RiskFlags, SetTarget, StmtKind, StmtReport, TouchedObjects,
    };
    use sql_split::ParamSettings;
    use sql_split::exec_range::UnitKind;
    use sql_text::{TextRange, TextSize};

    /// The dialect of `engine`.
    pub fn dialect_of(engine: DbmsKind) -> DialectId {
        match engine {
            DbmsKind::MySql | DbmsKind::TiDb => DialectId::MySql,
            DbmsKind::MariaDb => DialectId::MariaDb,
            DbmsKind::Sqlite => DialectId::Sqlite,
            _ => DialectId::Postgres,
        }
    }

    pub fn ctx_of(engine: DbmsKind) -> DialectCtx {
        DialectCtx::new(dialect_of(engine), None)
    }

    /// An object named `name` (a qualified name as written) of `kind`.
    pub fn object(name: &str, kind: ObjectKind) -> ObjectRef {
        ObjectRef {
            name: parse_qualified(DialectId::Postgres, name).expect("a name"),
            kind,
        }
    }

    /// A statement of `kind` with `effects` (class `class_of(effects)`, as the backends write
    /// it); `over` makes it cover the whole text.
    pub fn stmt(kind: StmtKind, effects: EffectSet) -> StmtReport {
        StmtReport {
            range: TextRange::default(),
            kind,
            effects,
            flags: RiskFlags::empty(),
            class: class_of(effects),
            touched: TouchedObjects::default(),
            reasons: Vec::new(),
            destructive: Vec::new(),
            confidence: Confidence::Exact,
            set_targets: Vec::new(),
        }
    }

    /// The plan's builder: a statement of `kind` with `effects` and the destructive `kinds`,
    /// each on the table `target` (written exactly so).
    pub fn report(
        kind: StmtKind,
        effects: EffectSet,
        kinds: &[DestructiveKind],
        target: &str,
    ) -> StmtReport {
        let mut s = stmt(kind, effects);
        s.touched.writes.push(object(target, ObjectKind::Table));
        s.destructive = kinds
            .iter()
            .map(|k| DestructiveReason {
                kind: *k,
                range: TextRange::default(),
                target: Some(object(target, ObjectKind::Table)),
            })
            .collect();
        s
    }

    pub fn reason(code: ReasonCode, arg: Option<&str>) -> Reason {
        Reason {
            code,
            range: None,
            arg: arg.map(Arc::from),
        }
    }

    /// The request of `items` over exactly `text` in `dialect`, merged and hashed the way
    /// `classify` does (P-M3b-2); an item whose range is empty covers the whole text.
    pub fn over(text: &str, dialect: DialectId, mut items: Vec<StmtReport>) -> SafetyReport {
        let whole = TextRange::up_to(TextSize::from(u32::try_from(text.len()).unwrap()));
        for s in &mut items {
            if s.range.is_empty() {
                s.range = whole;
            }
        }
        let ctx = DialectCtx::new(dialect, None);
        let mut r = SafetyReport::pre_decided(text, sql_split::hv_norm1(text, &ctx), Vec::new());
        let (class, effects, flags) = sql_safety::report::merge(&items);
        r.items = items;
        r.request_class = class;
        r.request_effects = effects;
        r.flags = flags;
        r.pre_decision = None;
        r.dialect = Some(dialect);
        r
    }

    /// The word after the first of `keys` in `text` (a table, schema or database name).
    fn name_after(text: &str, keys: &[&str]) -> String {
        let words: Vec<&str> = text.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            if keys.iter().any(|k| w.eq_ignore_ascii_case(k)) {
                let mut j = i + 1;
                while words.get(j).is_some_and(|w| {
                    ["IF", "EXISTS", "ONLY", "TABLE"].contains(&w.to_ascii_uppercase().as_str())
                }) {
                    j += 1;
                }
                if let Some(n) = words.get(j) {
                    let n = n.trim_end_matches(';');
                    return n.split('(').next().unwrap_or_default().to_string();
                }
            }
        }
        "t".to_string()
    }

    /// A test-only statement report of `text` by its first words (the module docs).
    pub fn fake_stmt(text: &str) -> StmtReport {
        let up = text.to_ascii_uppercase();
        let has = |w: &str| up.split_whitespace().any(|x| x.trim_end_matches(';') == w);
        let first = up.split_whitespace().next().unwrap_or_default().to_string();
        let table = |keys: &[&str]| name_after(text, keys);
        match first.as_str() {
            "EXPLAIN" if has("ANALYZE") => {
                let at = up.find("ANALYZE").unwrap() + "ANALYZE".len();
                let mut s = fake_stmt(text[at..].trim_start());
                s.kind = StmtKind::Explain { analyze: true };
                s.flags |= RiskFlags::EXPLAIN_ANALYZE;
                s.reasons.push(reason(ReasonCode::ExplainAnalyze, None));
                s
            }
            "SELECT" | "WITH" | "VALUES" | "SHOW" => {
                if up.contains("FOR UPDATE") {
                    let mut s = stmt(StmtKind::Select, EffectSet::READ | EffectSet::LOCK_ROW);
                    s.reasons.push(reason(ReasonCode::LockingClause, None));
                    s
                } else if up.contains("F_WRITE(") {
                    let mut s = stmt(StmtKind::Select, EffectSet::READ);
                    s.flags |= RiskFlags::VOLATILE_FN;
                    s.reasons
                        .push(reason(ReasonCode::FnUnknown, Some("f_write")));
                    s
                } else {
                    stmt(StmtKind::Select, EffectSet::READ)
                }
            }
            "INSERT" => stmt(StmtKind::Insert, EffectSet::WRITE_DATA),
            "UPDATE" if has("WHERE") => stmt(StmtKind::Update, EffectSet::WRITE_DATA),
            "UPDATE" => report(
                StmtKind::Update,
                EffectSet::WRITE_DATA,
                &[DestructiveKind::NoWhere],
                &table(&["UPDATE"]),
            ),
            "DELETE" if has("WHERE") => stmt(StmtKind::Delete, EffectSet::WRITE_DATA),
            "DELETE" => report(
                StmtKind::Delete,
                EffectSet::WRITE_DATA,
                &[DestructiveKind::NoWhere],
                &table(&["FROM"]),
            ),
            "CALL" => {
                let mut s = stmt(StmtKind::Call, EffectSet::ROUTINE_CALL);
                s.reasons.push(reason(ReasonCode::RoutineCall, None));
                s
            }
            "DO" => {
                let mut s = stmt(StmtKind::Do, EffectSet::UNKNOWN);
                s.reasons.push(reason(ReasonCode::ProceduralCode, None));
                s
            }
            "SET" => {
                let rest = text[3..].trim().trim_end_matches(';');
                let (name, value) = match rest.split_once('=') {
                    Some((n, v)) => (n.trim(), v.trim()),
                    None => match rest.split_once(' ') {
                        Some((n, v)) => (n.trim(), v.trim().trim_start_matches("TO ").trim()),
                        None => (rest, ""),
                    },
                };
                let mut s = stmt(StmtKind::Set, EffectSet::SESSION_STATE);
                s.set_targets.push(SetTarget {
                    name: Arc::from(name),
                    value: Some(Arc::from(value)),
                });
                if name.eq_ignore_ascii_case("role") {
                    s.reasons.push(reason(ReasonCode::PrivilegeChange, None));
                } else {
                    s.reasons.push(reason(ReasonCode::SessionState, Some(name)));
                }
                s
            }
            "BEGIN" | "COMMIT" | "ROLLBACK" | "START" => {
                let kind = match first.as_str() {
                    "COMMIT" => StmtKind::Commit,
                    "ROLLBACK" => StmtKind::Rollback,
                    _ => StmtKind::Begin,
                };
                stmt(kind, EffectSet::TXN_CONTROL)
            }
            "LOCK" => stmt(StmtKind::Lock, EffectSet::LOCK_TABLE),
            "CREATE" if has("TEMP") || has("TEMPORARY") => {
                stmt(StmtKind::CreateTable, EffectSet::DDL_TEMP)
            }
            "CREATE" if has("TABLE") => stmt(StmtKind::CreateTable, EffectSet::DDL),
            "CREATE" => stmt(StmtKind::CreateOther, EffectSet::DDL),
            "ALTER" => stmt(StmtKind::Alter, EffectSet::DDL),
            "DROP" => {
                let cascade = has("CASCADE");
                let (kind, key) = if has("SCHEMA") {
                    (ObjectKind::Schema, "SCHEMA")
                } else if has("DATABASE") {
                    (ObjectKind::Database, "DATABASE")
                } else {
                    (ObjectKind::Table, "DROP")
                };
                let mut s = stmt(StmtKind::Drop, EffectSet::DDL);
                s.destructive.push(DestructiveReason {
                    kind: DestructiveKind::Drop { cascade },
                    range: TextRange::default(),
                    target: Some(object(&table(&[key]), kind)),
                });
                s
            }
            "TRUNCATE" => {
                let mut s = stmt(StmtKind::Truncate, EffectSet::DDL);
                s.destructive.push(DestructiveReason {
                    kind: DestructiveKind::Truncate,
                    range: TextRange::default(),
                    target: Some(object(&table(&["TRUNCATE"]), ObjectKind::Table)),
                });
                s
            }
            "GRANT" => stmt(StmtKind::Grant, EffectSet::PRIVILEGE),
            "REVOKE" => stmt(StmtKind::Revoke, EffectSet::PRIVILEGE),
            "COPY" if up.contains("PROGRAM") => stmt(StmtKind::Copy, EffectSet::PROCESS_EXEC),
            _ => {
                let mut s = stmt(StmtKind::Other, EffectSet::UNKNOWN);
                s.reasons.push(reason(ReasonCode::EffectsUnknown, None));
                s
            }
        }
    }

    /// `fake_stmt` over exactly `text` in `dialect`.
    pub fn fake_classify(text: &str, dialect: DialectId) -> SafetyReport {
        over(text, dialect, vec![fake_stmt(text)])
    }

    /// A connection: idle, Auto-commit, unsafe-WHERE confirmation on.
    pub fn conn(engine: DbmsKind, env: Option<EnvTag>) -> ConnSafety {
        ConnSafety {
            engine,
            env,
            read_only: false,
            tx_status: TxStatus::Idle,
            tx_owner: TxOwner::None,
            tx_mode: TxMode::Auto,
            confirm_unsafe_where: true,
        }
    }

    pub fn conn_dev() -> ConnSafety {
        conn(DbmsKind::Postgres, Some(EnvTag::Dev))
    }

    pub fn conn_staging() -> ConnSafety {
        conn(DbmsKind::Postgres, Some(EnvTag::Staging))
    }

    pub fn conn_prod() -> ConnSafety {
        conn(DbmsKind::Postgres, Some(EnvTag::Production))
    }

    pub fn conn_prod_auto() -> ConnSafety {
        conn_prod()
    }

    pub fn conn_prod_manual_idle() -> ConnSafety {
        ConnSafety {
            tx_mode: TxMode::Manual,
            ..conn_prod()
        }
    }

    pub fn conn_prod_user_tx() -> ConnSafety {
        ConnSafety {
            tx_status: TxStatus::InTx,
            tx_owner: TxOwner::User,
            tx_mode: TxMode::Manual,
            ..conn_prod()
        }
    }

    /// A read-only (toggle or guard) development connection.
    pub fn conn_ro() -> ConnSafety {
        ConnSafety {
            read_only: true,
            ..conn_dev()
        }
    }

    /// A store holding `pairs` for every profile (none: a store with no remembered value).
    pub struct Values(pub Vec<(String, BindValue)>);

    impl Values {
        pub fn of(pairs: &[(&str, &str)]) -> Values {
            Values(
                pairs
                    .iter()
                    .map(|(n, v)| (n.to_string(), text_value(v)))
                    .collect(),
            )
        }
    }

    impl ParamValueStore for Values {
        fn get(&self, _: &ProfileId, name: &str) -> Option<BindValue> {
            self.0
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        }
        fn put(&self, _: &ProfileId, _: &str, _: BindValue) {}
    }

    pub fn text_value(v: &str) -> BindValue {
        BindValue::Typed {
            hint: LogicalType::Text {
                max_chars: None,
                fixed: false,
            },
            text: Arc::from(v),
        }
    }

    /// Statement units of `texts`, one after the other in a document (two bytes apart), each
    /// with a read head (`suspendable`).
    pub fn exec_units(texts: &[&str]) -> Vec<ExecUnit> {
        let mut at = 0;
        texts
            .iter()
            .map(|t| {
                let u = ExecUnit {
                    text: Arc::from(*t),
                    doc_range: at..at + t.len(),
                    kind: UnitKind::Statement,
                    suspendable: true,
                    title: None,
                };
                at += t.len() + 2;
                u
            })
            .collect()
    }

    /// `texts` prepared on `engine`, every prompt answered from `values` by its name
    /// (placeholders inside DDL bind): the prompt, then the run with the answers.
    pub fn prepare_answered(
        engine: DbmsKind,
        texts: &[&str],
        values: &[(&str, &str)],
        s: ParamSettings,
    ) -> Prepared {
        let (ctx, profile) = (ctx_of(engine), ProfileId::new());
        let store = Values(Vec::new());
        match prepare_units(exec_units(texts), &ctx, &s, &store, &profile) {
            Prepared::NeedsValues(prompts) => {
                let mut answers = ParamAnswers::default();
                for p in &prompts {
                    if let Some((_, v)) = values.iter().find(|(n, _)| **n == *p.name) {
                        answers.insert(p, text_value(v));
                    }
                }
                prepare_units_with(exec_units(texts), &ctx, &s, &store, &profile, &answers)
            }
            other => other,
        }
    }

    /// `text` prepared on `engine` with `values` (placeholders inside DDL bind).
    pub fn prepared_on(engine: DbmsKind, text: &str, values: &[(&str, &str)]) -> PreparedUnit {
        let s = ParamSettings {
            in_ddl: true,
            ..ParamSettings::default()
        };
        match prepare_answered(engine, &[text], values, s) {
            Prepared::Ready(mut u) => u.remove(0),
            other => panic!("{text}: {other:?}"),
        }
    }

    /// The gate's item for `text` on `conn`, `fake_stmt` over its final text.
    pub fn item_of(text: &str, conn: &ConnSafety) -> GateItem {
        let unit = prepared_on(conn.engine, text, &[]);
        let report = fake_classify(&unit.final_text, dialect_of(conn.engine));
        gate(&[unit], &[report], conn, &SuppressSet::default(), 1)
            .items
            .remove(0)
    }

    /// The gate's decision for `text` on `conn` (`item_of`).
    pub fn decision(text: &str, conn: ConnSafety) -> GateDecision {
        item_of(text, &conn).decision
    }
}

// ## m3b-t18-safe06
/// Task 18's read-only consoles with the read-only guard's classifier (run A′, P-M3b-13):
/// a profile connected read-only on a manager of its own whose `ManagerConfig::classifier` is
/// the one given (the app's boot installs a `ClassifyPool`), as `user`.
#[allow(
    clippy::result_large_err,
    reason = "Reply is the session's fixed interface"
)]
pub mod t18 {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use hacivat_driver::ExecOpts;
    use hacivat_secrets::SecretString;
    use hacivat_session::pipeline::Interaction;
    use hacivat_session::profile::Profile;
    use hacivat_session::session::{
        ExecErrorKind, ExecOrigin, Reply, Request, SessionHandle, StatementClassifier,
    };
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};

    use super::matrix::{Line, deps, profile_of};

    /// A read-only console of `line` as `user`.
    pub struct RoConsole {
        pub m: Arc<ConnectionManager>,
        pub profile: Arc<Profile>,
        pub s: SessionHandle,
    }

    impl RoConsole {
        /// Connects `line` read-only as `user` (the test account when `None`), with
        /// `classifier` (`None`: run B's layers alone) and the profile's function `allowlist`.
        pub async fn open(
            line: &Line,
            user: Option<&str>,
            classifier: Option<Arc<dyn StatementClassifier>>,
            allowlist: &[&str],
        ) -> RoConsole {
            let mut p = profile_of(line);
            p.read_only = true;
            p.allow_unsupported_server = true;
            if let Some(user) = user {
                p.user = Some(user.to_string());
            }
            p.read_only_fn_allowlist = allowlist.iter().map(|s| s.to_string()).collect();
            let config = ManagerConfig {
                classifier,
                ..ManagerConfig::m2b()
            };
            let m =
                ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps()), config);
            let profile = Arc::new(p);
            let password = line
                .target
                .as_ref()
                .map(|t| SecretString::from(t.password.clone()));
            m.connect(profile.clone(), password, Interaction::Headless);
            let mut changes = m.changes();
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                match m.state(profile.id) {
                    LiveState::Connected(_) => break,
                    LiveState::Failed(e) => {
                        panic!("{}: the connect failed: {:?}", line.id, e.class)
                    }
                    LiveState::Lost { class } => panic!("{}: lost: {class:?}", line.id),
                    _ => {}
                }
                assert!(Instant::now() < deadline, "{}: never connected", line.id);
                tokio::select! {
                    _ = changes.changed() => {}
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
            let s = m
                .sessions(profile.id)
                .expect("connected")
                .default_session()
                .expect("a default session");
            assert!(s.state().borrow().read_only, "{}", line.id);
            RoConsole { m, profile, s }
        }

        /// The profile's `application_name` (PostgreSQL's side channel finds the backend by it).
        pub fn app(&self) -> &str {
            &self.profile.application_name
        }

        /// Runs `sql` as a user statement (M2b `Exec`), reading up to 10 rows.
        pub async fn exec(&self, sql: &str) -> Reply {
            self.s
                .submit(Request::Exec {
                    sql: sql.to_string(),
                    opts: ExecOpts::values(10),
                    origin: ExecOrigin::User,
                })
                .reply
                .await
                .expect("the actor replies")
        }

        /// Hacivat's own constant statement (outside the guard), its first value.
        pub async fn internal(&self, sql: &str) -> Option<String> {
            self.s
                .submit(Request::Exec {
                    sql: sql.to_string(),
                    opts: ExecOpts::values(10),
                    origin: ExecOrigin::Internal { idempotent: true },
                })
                .reply
                .await
                .expect("the actor replies")
                .ok()?
                .rows
                .first()?
                .first()
                .cloned()
                .flatten()
        }

        pub fn close(self) {
            self.m.disconnect(self.profile.id);
        }
    }

    /// Whether `reply` is step 0b's denial.
    pub fn safety_denied(reply: &Reply) -> bool {
        matches!(reply, Err(e) if matches!(e.kind, ExecErrorKind::SafetyDenied(_)))
    }
}

// ## m3b-t28-matrix
/// Task 28's helpers (the M3b gate's E-rows over the Docker matrix, `docker_m3b_matrix`): a
/// console driven the app's M3b way. A document goes through `resolve_units`, then
/// `prepare_units` (the prompts answered by name), then the real `ClassifyPool` in the
/// session's own context, then `gate` on the live connection, then `ExecService::run`, a real
/// session and the typed driver. The lines, the side channel and the waits are Task 31's
/// (`matrix`).
pub mod t28 {
    use std::collections::VecDeque;
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use futures::future::BoxFuture;
    use hacivat_driver::{ExecOpts, Protocol, SemVerLoose};
    use hacivat_exec::classify_pool::{ClassifyPool, guard_ctx_in};
    use hacivat_exec::{
        ClassifyCtx, CommitQuestion, ConfirmAnswer, ConfirmPort, ConfirmQuestion, ConnSafety,
        ExecService, ExecUnit, Expect, GateDecision, GatePlan, ParamAnswers, ParamValueStore,
        Prepared, PreparedUnit, Resolution, RunAction, RunRequest, RunTicket, SafetyReport,
        SessionLog, SuppressSet, apply_answer, gate, prepare_units, prepare_units_with,
        resolve_units, sqlctx,
    };
    use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
    use hacivat_results::{FetchPolicy, MemoryBudget, ResultId, ResultStore, ResultWriter};
    use hacivat_secrets::SecretString;
    use hacivat_session::pipeline::Interaction;
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::session::{
        ClassifierContext, CommitAnswer, ErrorPolicy, ExecOrigin, ObserverRef, Request, RoAnswer,
        RoCheck, RunEvent, RunId, RunObserver, RunSpec, RunUnit, SessionEffects, SessionHandle,
        SessionSnapshot, SessionState, StatementClassifier, TxCommand, TxMode, UnitId,
    };
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};
    use hacivat_values::{BindValue, ResultSchema};
    use sql_dialect::DialectCtx;
    use sql_split::exec_range::UnitKind;
    use sql_split::{DocSnapshot, ParamSettings};
    use tokio::sync::oneshot;

    use super::matrix::{Line, Ran, deps, ended, profile_of, secs};

    /// How a console is opened.
    pub struct Spec {
        pub env: Option<EnvTag>,
        pub read_only: bool,
        /// The read-only guard's classifier (`ManagerConfig::classifier`, step 0b).
        pub classifier: Option<Arc<dyn StatementClassifier>>,
    }

    impl Spec {
        pub fn dev() -> Spec {
            Spec {
                env: Some(EnvTag::Dev),
                read_only: false,
                classifier: None,
            }
        }

        /// A production console that writes (as Task 26 leaves it): `ManagerConfig::m2b()`
        /// keeps production sessions writable, so the gate's confirmations are the only
        /// barrier (P-M3b-7: the production policy flag is never named here).
        pub fn production() -> Spec {
            Spec {
                env: Some(EnvTag::Production),
                read_only: false,
                classifier: None,
            }
        }

        /// A read-only development console whose guard asks `classifier`.
        pub fn read_only(classifier: Arc<dyn StatementClassifier>) -> Spec {
            Spec {
                env: Some(EnvTag::Dev),
                read_only: true,
                classifier: Some(classifier),
            }
        }
    }

    /// The values a run remembers (none: every prompt is answered by the test).
    struct NoMemory;

    impl ParamValueStore for NoMemory {
        fn get(&self, _: &ProfileId, _: &str) -> Option<BindValue> {
            None
        }
        fn put(&self, _: &ProfileId, _: &str, _: BindValue) {}
    }

    /// A run of a console, planned: the units, their prepared form (exactly the bytes sent),
    /// the pool's reports of those bytes, and the gate's plan on the live connection.
    #[derive(Debug, Clone)]
    pub struct Planned {
        pub units: Vec<ExecUnit>,
        pub prepared: Vec<PreparedUnit>,
        pub reports: Vec<SafetyReport>,
        pub plan: GatePlan,
        pub run_as_one: bool,
        /// The units that asked before the run (`Confirm`/`Warn`), answered by
        /// [`Planned::answer`].
        pub asked: Vec<u32>,
    }

    impl Planned {
        /// Each unit's decision, as a short name ([`shape`]).
        pub fn shapes(&self) -> Vec<&'static str> {
            self.plan.items.iter().map(|i| shape(&i.decision)).collect()
        }

        /// The user's `answer` to every `Confirm`/`Warn` (the app's dialogs, Task 21).
        pub fn answer(&mut self, answer: ConfirmAnswer) {
            let asking: Vec<u32> = self
                .plan
                .items
                .iter()
                .filter(|i| {
                    matches!(
                        i.decision,
                        GateDecision::Confirm { .. } | GateDecision::Warn(_)
                    )
                })
                .map(|i| i.unit)
                .collect();
            for unit in asking {
                assert!(apply_answer(&mut self.plan, unit, answer), "{answer:?}");
                self.asked.push(unit);
            }
        }

        /// Unit `i`'s plan item turned to `Run` (a gate that missed): what the session's own
        /// layers must still stop.
        pub fn forced(mut self, i: usize) -> Planned {
            self.plan.items[i].decision = GateDecision::Run;
            self
        }
    }

    /// A short name for a decision (the gate tests' table names).
    pub fn shape(d: &GateDecision) -> &'static str {
        match d {
            GateDecision::Run => "run",
            GateDecision::Warn(_) => "warn",
            GateDecision::Confirm {
                type_to_confirm: Some(_),
                ..
            } => "confirm+type",
            GateDecision::Confirm { .. } => "confirm",
            GateDecision::RunInTxThenConfirm { .. } => "tx-confirm",
            GateDecision::ExplainAnalyzeRollback => "explain-wrap",
            GateDecision::Deny(_) => "deny",
        }
    }

    /// PostgreSQL's `SHOW search_path` split into its elements, quotes removed (`$user` kept).
    fn split_search_path(raw: &str) -> Vec<Arc<str>> {
        raw.split(',')
            .map(|s| Arc::<str>::from(s.trim().trim_matches('"')))
            .filter(|s| !s.is_empty())
            .collect()
    }

    static CONSOLES: AtomicU64 = AtomicU64::new(1_000);

    /// A console of the M3b pipeline (the module docs).
    pub struct Con {
        pub line: Line,
        pub m: Arc<ConnectionManager>,
        pub profile: Arc<Profile>,
        pub s: SessionHandle,
        pub svc: ExecService,
        pub log: Arc<SessionLog>,
        pub ctx: DialectCtx,
        pub version: Option<SemVerLoose>,
        pub pool: Arc<ClassifyPool>,
        console: u64,
    }

    impl Con {
        /// Connects `line` as `spec` says; the run service classifies with `pool` (also its
        /// "Count rows" check).
        pub async fn open(line: &Line, spec: Spec, pool: &Arc<ClassifyPool>) -> Con {
            let mut p = profile_of(line);
            p.read_only = spec.read_only;
            p.environment = spec.env;
            if spec.read_only
                && let ProfileEndpoint::File {
                    create_if_missing, ..
                } = &mut p.endpoint
            {
                *create_if_missing = false;
            }
            let config = ManagerConfig {
                classifier: spec.classifier,
                ..ManagerConfig::m2b()
            };
            let m =
                ConnectionManager::new(tokio::runtime::Handle::current(), Arc::new(deps()), config);
            let profile = Arc::new(p);
            let password = line
                .target
                .as_ref()
                .map(|t| SecretString::from(t.password.clone()));
            m.connect(profile.clone(), password, Interaction::Headless);
            let mut changes = m.changes();
            let deadline = Instant::now() + secs(60);
            let version = loop {
                match m.state(profile.id) {
                    LiveState::Connected(info) => break info.server.version.clone(),
                    LiveState::Failed(e) => {
                        panic!("{}: the connect failed: {:?}", line.id, e.class)
                    }
                    LiveState::Lost { class } => panic!("{}: lost: {class:?}", line.id),
                    _ => {}
                }
                assert!(Instant::now() < deadline, "{}: never connected", line.id);
                tokio::select! {
                    _ = changes.changed() => {}
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            };
            let s = m
                .sessions(profile.id)
                .expect("connected")
                .default_session()
                .expect("a default session");
            assert_eq!(s.state().borrow().read_only, spec.read_only, "{}", line.id);
            let mut c = Con {
                line: line.clone(),
                m,
                profile,
                s,
                svc: ExecService::new(MemoryBudget::new(4 << 30, 256 << 20))
                    .with_classifier(pool.clone()),
                log: SessionLog::new(),
                ctx: sqlctx::dialect_ctx(line.engine, version.as_ref(), None),
                version,
                pool: pool.clone(),
                console: CONSOLES.fetch_add(1, Ordering::SeqCst),
            };
            if let Some(sql) = sqlctx::probe_sql(line.engine) {
                let row = c.internal(sql).await;
                c.ctx = sqlctx::dialect_ctx(line.engine, c.version.as_ref(), row.as_deref());
            }
            c
        }

        /// Hacivat's own idempotent statement: its first value (`None` for NULL, no row, or a
        /// failure).
        pub async fn internal(&self, sql: &str) -> Option<String> {
            self.s
                .submit(Request::Exec {
                    sql: sql.to_string(),
                    opts: ExecOpts {
                        max_rows: 10,
                        protocol: Protocol::Simple,
                    },
                    origin: ExecOrigin::Internal { idempotent: true },
                })
                .reply
                .await
                .expect("the actor replies")
                .ok()?
                .rows
                .first()?
                .first()
                .cloned()
                .flatten()
        }

        /// What the run path classifies with (the T24 carry): the session's own context, read
        /// now (its lexing-mode setting, PostgreSQL's search path, the MySQL family's current
        /// database as `search_path[0]`, the server version), modes known when read.
        pub async fn classify_ctx(&self, expect: Expect) -> ClassifyCtx {
            let (lex_modes, search_path) = match self.line.engine {
                DbmsKind::Postgres => (
                    self.internal("SHOW standard_conforming_strings").await,
                    self.internal("SHOW search_path")
                        .await
                        .map(|p| split_search_path(&p)),
                ),
                DbmsKind::MySql | DbmsKind::MariaDb => (
                    self.internal("SELECT @@SESSION.sql_mode").await,
                    self.internal("SELECT DATABASE()")
                        .await
                        .map(|d| vec![Arc::<str>::from(d)]),
                ),
                _ => (None, None),
            };
            let c = ClassifierContext {
                lex_modes: lex_modes.map(Arc::from),
                search_path: search_path.map(Arc::from),
                server_version: self.version.clone(),
            };
            let mut ctx = guard_ctx_in(self.line.engine, Arc::from(Vec::new()), &c);
            ctx.expect = expect;
            ctx
        }

        pub fn snap(&self) -> SessionSnapshot {
            self.s.state().borrow().clone()
        }

        /// The snapshot once the session left its request (the run's reply follows `Idle`).
        pub async fn settled(&self) -> SessionSnapshot {
            let mut state = self.s.state();
            let left = |s: &SessionSnapshot| {
                !matches!(
                    s.state,
                    SessionState::Busy { .. } | SessionState::Cancelling { .. }
                )
            };
            match tokio::time::timeout(secs(10), state.wait_for(left)).await {
                Ok(Ok(snap)) => snap.clone(),
                Ok(Err(_)) => panic!("{}: the session ended", self.line.id),
                Err(_) => panic!("{}: the session stayed busy", self.line.id),
            }
        }

        /// The gate's view of the connection now.
        pub fn conn(&self) -> ConnSafety {
            let snap = self.snap();
            ConnSafety {
                engine: self.line.engine,
                env: self.profile.environment,
                read_only: snap.read_only,
                tx_status: snap.tx.status,
                tx_owner: snap.tx.owner,
                tx_mode: snap.tx.mode,
                confirm_unsafe_where: true,
            }
        }

        pub fn doc(&self, text: &str) -> DocSnapshot {
            super::doc_in(text, self.ctx)
        }

        /// The units Run Execute3 (the whole script) sends for `text`.
        pub fn units(&self, text: &str) -> Vec<ExecUnit> {
            match resolve_units(
                &self.doc(text),
                &[super::caret(0)],
                RunAction::Execute3,
                None,
            ) {
                Resolution::Units(units) => units,
                other => panic!("{}: `{text}` resolved to {other:?}", self.line.id),
            }
        }

        /// `text` as one unit, sent as written (Run as one, R-M3-60).
        pub fn as_one(&self, text: &str) -> Vec<ExecUnit> {
            vec![ExecUnit {
                text: Arc::from(text),
                doc_range: 0..text.len(),
                kind: UnitKind::Verbatim,
                suspendable: false,
                title: None,
            }]
        }

        /// Prepares `units` (every prompt answered from `values` by its name), classifies the
        /// prepared bytes on the pool in the session's context and gates them on the live
        /// connection.
        pub async fn plan_units(
            &self,
            units: Vec<ExecUnit>,
            values: &[(&str, BindValue)],
            run_as_one: bool,
        ) -> Planned {
            let id = &self.line.id;
            let settings = ParamSettings::default();
            let profile = self.profile.id;
            let prepared =
                match prepare_units(units.clone(), &self.ctx, &settings, &NoMemory, &profile) {
                    Prepared::Ready(p) => p,
                    Prepared::NeedsValues(prompts) => {
                        let mut answers = ParamAnswers::default();
                        for p in &prompts {
                            let (_, v) = values
                                .iter()
                                .find(|(n, _)| *n == &*p.name)
                                .unwrap_or_else(|| panic!("{id}: no value for `{}`", p.name));
                            answers.insert(p, v.clone());
                        }
                        match prepare_units_with(
                            units.clone(),
                            &self.ctx,
                            &settings,
                            &NoMemory,
                            &profile,
                            &answers,
                        ) {
                            Prepared::Ready(p) => p,
                            other => panic!("{id}: {other:?}"),
                        }
                    }
                    other => panic!("{id}: {other:?}"),
                };
            let expect = if run_as_one {
                Expect::Script
            } else {
                Expect::SingleStatement
            };
            let texts: Vec<Arc<str>> = prepared.iter().map(|p| p.final_text.clone()).collect();
            let reports = match self
                .pool
                .classify_units(texts, self.classify_ctx(expect).await)
                .await
            {
                Ok(r) => r,
                Err(never) => match never {},
            };
            let plan = gate(
                &prepared,
                &reports,
                &self.conn(),
                &SuppressSet::default(),
                self.console,
            );
            Planned {
                units,
                prepared,
                reports,
                plan,
                run_as_one,
                asked: Vec::new(),
            }
        }

        /// [`Con::plan_units`] of Run Execute3 on `text`.
        pub async fn plan(&self, text: &str, values: &[(&str, BindValue)]) -> Planned {
            self.plan_units(self.units(text), values, false).await
        }

        /// Sends `p` through the run service.
        pub fn send(
            &self,
            p: &Planned,
            policy: ErrorPolicy,
            confirm: Option<Arc<dyn ConfirmPort>>,
        ) -> RunTicket {
            self.svc.run(
                &self.s,
                RunRequest {
                    console: self.console,
                    engine: self.line.engine,
                    units: p.units.clone(),
                    fetch: FetchPolicy::default(),
                    log: self.log.clone(),
                    prepared: Some(p.prepared.clone()),
                    plan: Some(p.plan.clone()),
                    policy,
                    run_as_one: p.run_as_one,
                    history: None,
                    confirm,
                    context_generation: None,
                },
            )
        }

        /// Plans `text` with `values`, answers every confirmation Execute, runs it (stop on
        /// error) and waits for its end.
        pub async fn run_with(&self, text: &str, values: &[(&str, BindValue)]) -> Ran {
            let mut p = self.plan(text, values).await;
            p.answer(ConfirmAnswer::Execute);
            ended(self.send(&p, ErrorPolicy::StopOnError, None)).await
        }

        pub async fn run(&self, text: &str) -> Ran {
            self.run_with(text, &[]).await
        }

        /// [`Con::run`]; every unit must succeed.
        pub async fn ok(&self, text: &str) -> Ran {
            let ran = self.run(text).await;
            assert_eq!(ran.outcome(), "ran", "{}: `{text}`", self.line.id);
            ran
        }

        /// Sets the transaction mode (the chip, Task 22).
        pub async fn set_mode(&self, mode: TxMode) {
            let r = self
                .s
                .submit(Request::Tx(TxCommand::SetMode(mode)))
                .reply
                .await
                .expect("the actor replies");
            assert!(r.is_ok(), "{}: SetMode({mode:?}): {r:?}", self.line.id);
        }

        /// Sends `p`'s units straight to the session with `deadline` as the confirm deadline,
        /// unanswered; the run's events. The run service has no deadline of its own to inject
        /// (D-014's 5 minutes, `RunSpec::CONFIRM_DEADLINE`), so this pass builds the same
        /// `RunUnit`s the service builds (the M3a T31 precedent: a run through
        /// `SessionHandle::run` with an observer of the test's own).
        pub async fn run_with_deadline(&self, p: &Planned, deadline: Duration) -> Vec<RunEvent> {
            static RUNS: AtomicU64 = AtomicU64::new(u64::MAX / 2);
            let run = RunId(RUNS.fetch_add(1, Ordering::SeqCst));
            let units: Arc<[RunUnit]> = p
                .prepared
                .iter()
                .zip(&p.plan.items)
                .enumerate()
                .map(|(i, (prep, item))| {
                    assert!(
                        matches!(
                            item.decision,
                            GateDecision::Run
                                | GateDecision::RunInTxThenConfirm { .. }
                                | GateDecision::ExplainAnalyzeRollback
                        ),
                        "{}: unit {i} is not to be sent: {:?}",
                        self.line.id,
                        item.decision
                    );
                    RunUnit {
                        id: UnitId {
                            run,
                            ordinal: u32::try_from(i).unwrap(),
                        },
                        sql: prep.final_text.clone(),
                        protocol: Protocol::Extended,
                        suspendable: false,
                        binds: prep.binds.clone(),
                        verdict: item.verdict,
                        effects: item.effects,
                        masked: prep.masked.clone(),
                        max_result_sets: 100,
                        cell_cap: None,
                    }
                })
                .collect();
            let events = Arc::new(Events::default());
            let spec = RunSpec {
                run,
                units,
                policy: ErrorPolicy::StopOnError,
                fetch: FetchPolicy::default(),
                observer: ObserverRef(events.clone()),
                confirm_deadline: deadline,
                context_generation: None,
            };
            // The reply of a run whose deadline rolled back is `Tx(ConfirmTimedOut)`; the
            // events say what happened.
            let reply = self.s.run(spec).reply.await.expect("the actor replies");
            if let Err(e) = &reply {
                assert!(
                    matches!(
                        e.kind,
                        hacivat_session::session::ExecErrorKind::Tx(
                            hacivat_session::session::TxError::ConfirmTimedOut
                        )
                    ),
                    "{}: {:?}",
                    self.line.id,
                    e.kind
                );
            }
            events.take()
        }

        pub async fn shutdown(self) {
            self.m.shutdown().await;
        }
    }

    /// The events of a run sent straight to the session.
    #[derive(Default)]
    struct Events {
        events: Mutex<Vec<RunEvent>>,
    }

    impl Events {
        fn take(&self) -> Vec<RunEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl RunObserver for Events {
        fn open_result(&self, _: UnitId, _: u32, schema: Arc<ResultSchema>) -> ResultWriter {
            let budget = MemoryBudget::new(1 << 30, 256 << 20);
            let (_store, writer) = ResultStore::new(ResultId(u64::MAX), schema, budget, 0);
            writer
        }

        fn event(&self, ev: &RunEvent) {
            self.events.lock().unwrap().push(ev.clone());
        }
    }

    /// The confirm-in-transaction port of a test: it records every question and holds it
    /// until the test answers ([`Port::answer`]), so the side channel can look first.
    #[derive(Debug, Default)]
    pub struct Port {
        asked: Mutex<Vec<CommitQuestion>>,
        waiting: Mutex<VecDeque<oneshot::Sender<CommitAnswer>>>,
    }

    impl Port {
        pub fn new() -> Arc<Port> {
            Arc::new(Port::default())
        }

        /// The questions asked so far.
        pub fn asked(&self) -> Vec<CommitQuestion> {
            self.asked.lock().unwrap().clone()
        }

        /// Waits (real time, at most 30 s) until `n` questions were asked; the `n`-th.
        pub async fn question(&self, id: &str, n: usize) -> CommitQuestion {
            let deadline = Instant::now() + secs(30);
            loop {
                if let Some(q) = self.asked.lock().unwrap().get(n - 1) {
                    return q.clone();
                }
                assert!(Instant::now() < deadline, "{id}: question {n} never came");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        /// Answers the oldest question still waiting.
        pub fn answer(&self, a: CommitAnswer) {
            let tx = self
                .waiting
                .lock()
                .unwrap()
                .pop_front()
                .expect("a question waits");
            assert!(tx.send(a).is_ok(), "the run still waits");
        }
    }

    impl ConfirmPort for Port {
        fn confirm(&self, _: ConfirmQuestion) -> oneshot::Receiver<ConfirmAnswer> {
            // Pre-run confirmations are answered on the plan ([`Planned::answer`]).
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(ConfirmAnswer::Cancel);
            rx
        }

        fn commit_decision(&self, q: CommitQuestion) -> oneshot::Receiver<CommitAnswer> {
            let (tx, rx) = oneshot::channel();
            self.waiting.lock().unwrap().push_back(tx);
            self.asked.lock().unwrap().push(q);
            rx
        }
    }

    /// A read-only guard classifier that allows everything (a classifier that missed): what
    /// the server's own layers must still stop.
    #[derive(Debug)]
    pub struct AllowAll;

    impl StatementClassifier for AllowAll {
        fn check_read_only(
            &self,
            _: DbmsKind,
            _: Arc<str>,
            _: Arc<[Arc<str>]>,
        ) -> BoxFuture<'static, RoCheck> {
            Box::pin(async { RoCheck::Allow })
        }

        fn effects(&self, _: DbmsKind, _: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
            Box::pin(async { None })
        }

        fn check_read_only_in(
            &self,
            _: DbmsKind,
            _: Arc<str>,
            _: Arc<[Arc<str>]>,
            _: ClassifierContext,
        ) -> BoxFuture<'static, RoAnswer> {
            Box::pin(async {
                RoAnswer {
                    check: RoCheck::Allow,
                    changes_context: false,
                }
            })
        }
    }

    /// The engine × version × case table of one test, written to the Docker artifact
    /// `t28-<test>.tsv` (ADR 0006 `m3b-t28` tabulates them).
    #[derive(Debug, Default)]
    pub struct Table {
        rows: Mutex<Vec<(String, String, String, String)>>,
    }

    impl Table {
        pub fn new() -> Table {
            Table::default()
        }

        pub fn add(&self, line: &Line, version: Option<&SemVerLoose>, case: &str, outcome: &str) {
            let v = version.map_or_else(String::new, |v| {
                format!("{}.{}.{}", v.major, v.minor, v.patch)
            });
            self.rows.lock().unwrap().push((
                line.id.clone(),
                v,
                case.to_string(),
                outcome.to_string(),
            ));
        }

        /// Asserts that every line of `lines` recorded a row, in order (no engine is skipped
        /// silently), and writes the artifact.
        pub fn finish(&self, test: &str, lines: &[Line]) {
            let rows = self.rows.lock().unwrap();
            let mut seen: Vec<&str> = Vec::new();
            for (id, ..) in rows.iter() {
                if !seen.contains(&id.as_str()) {
                    seen.push(id);
                }
            }
            let want: Vec<&str> = lines.iter().map(|l| l.id.as_str()).collect();
            assert_eq!(seen, want, "{test}: a line of the active set did not run");
            let mut out = String::new();
            for (id, v, case, outcome) in rows.iter() {
                let _ = writeln!(out, "{id}\t{v}\t{case}\t{outcome}");
            }
            let path = hacivat_testkit::artifact(&format!("t28-{test}.tsv"));
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).expect("the artifact directory");
            }
            std::fs::write(&path, out).expect("the record is written");
        }
    }
}
