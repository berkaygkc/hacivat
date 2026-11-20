// ## m4-t18-sqlite
//! The SQLite introspector (M4 Task 18; alg/02 HV-INTRO-08, the SQLite parts of HV-INTRO-13
//! and -15; R-M4-38): real files through the bundled engine of the real driver, no Docker. A
//! pass runs on the session manager's separate `mode=ro` reader, the `temp` schema on the
//! user's own connection, a joined read on the writer (R-M4-29).
//!
//! Fixture writes go through the user's console session (or a raw driver connection before the
//! manager exists); nothing here writes through a `MetaConn`.
// ## m4-t33a-hygiene (named exception): the modules this file uses exist only with the
// `session` feature, so `--no-default-features --tests` builds (T16b/T17 carry).
#![cfg(feature = "session")]

#![allow(
    clippy::result_large_err,
    reason = "the session's `Reply` and the seam's `IntroError` are other crates' types"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hacivat_driver::{
    Driver, DriverConnect, DriverRegistry, Endpoint, EngineCatalog, InsecureAuth,
};
use hacivat_introspect::filter::{CompiledFilter, FilterRole, parse_filter};
use hacivat_introspect::introspector::{
    FacetRows, IntroError, Introspector, ServerIdentity, Staleness,
};
use hacivat_introspect::pack::{DialectId, Mode, Pack, lint};
use hacivat_introspect::sqlite::{ATTACH_ALIAS, PassConn, SqliteIntrospector, ns_check_of};
use hacivat_meta::merge::{ExistSet, MergePolicy, NamespaceResult, base, merge};
use hacivat_meta::name::FoldRule;
use hacivat_meta::props::{
    FacetKind, GeneratedKind, Identity, Persistence, Props, TableExt, TriggerEvents, TriggerTiming,
};
use hacivat_meta::{
    KindGroup, Level, Name, NamespaceSnapshot, NsKey, Obj, ObjFlags, ObjectKey, ObjectKind,
    StateMark, Watermark,
};
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_proto::{DbmsKind, Department, ProfileId};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
};
use hacivat_session::budget_for;
use hacivat_session::meta::{MetaConn, MetaConnSpec, MetaError};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ClientId, ExecOrigin, Reply, Request, SessionHandle, SessionKind, SessionManager, SessionSpec,
};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// Fixtures

fn pid() -> ProfileId {
    "6f1c2a4e-1d16-4a16-8a16-000000000018".parse().unwrap()
}

/// The real drivers, a memory secret store, no credential files (the session crate's Docker
/// support, without Docker).
fn deps() -> ConnectDeps {
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    let known_hosts = KnownHosts::load(
        Vec::new(),
        PathBuf::from("/nonexistent/hacivat/known_hosts"),
    );
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    ConnectDeps {
        drivers: Arc::new(hacivat_session::default_drivers()),
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

fn profile_on(path: &Path, read_only: bool) -> Profile {
    Profile {
        id: pid(),
        name: "lite".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: path.to_path_buf(),
            create_if_missing: false,
        },
        user: None,
        auth: AuthProviderId::None,
        retention: Retention::Never,
        read_only,
        ..Profile::default()
    }
}

/// Creates `dir/name` and runs `sql` on it through the real driver (before any manager).
async fn file_with(dir: &Path, name: &str, sql: &str) -> PathBuf {
    let path = dir.join(name);
    let registry: DriverRegistry = hacivat_session::default_drivers();
    let driver: Arc<dyn Driver> = registry
        .resolve(DbmsKind::Sqlite)
        .expect("the SQLite driver");
    let mut conn = driver
        .connect(DriverConnect {
            endpoint: Endpoint::File {
                path: path.clone(),
                create_if_missing: true,
                read_only: false,
            },
            tls: None,
            user: None,
            password: None,
            database: None,
            application_name: "fixture".into(),
            read_only: false,
            options: Vec::new(),
            connect_timeout: Duration::from_secs(10),
            events: Arc::new(|_| {}),
            insecure_auth: InsecureAuth::default(),
        })
        .await
        .expect("the fixture file opens");
    if !sql.is_empty() {
        conn.batch(sql).await.expect("the fixture SQL runs");
    }
    conn.close().await;
    path
}

/// A file, its manager and the user's console session.
struct Lite {
    _dir: tempfile::TempDir,
    path: PathBuf,
    profile: Profile,
    sm: Arc<SessionManager>,
    user: SessionHandle,
}

async fn lite_with(sql: &str, read_only: bool) -> Lite {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(dir.path(), "main.db", sql).await;
    let profile = profile_on(&path, read_only);
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps()),
        Arc::new(budget_for(&profile)),
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    Lite {
        _dir: dir,
        path,
        profile,
        sm,
        user,
    }
}

async fn lite(sql: &str) -> Lite {
    lite_with(sql, false).await
}

async fn open(sm: &Arc<SessionManager>, kind: SessionKind) -> SessionHandle {
    let department = match kind {
        SessionKind::Job(..) => Department::Executor,
        _ => Department::Console,
    };
    sm.open(
        SessionSpec {
            kind,
            read_only: false,
            department,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("the session opens: {:?}", e.class))
}

/// Runs `sql` on the user's session.
async fn run(h: &SessionHandle, sql: &str) -> Reply {
    h.submit(Request::Exec {
        sql: sql.to_string(),
        opts: hacivat_driver::ExecOpts::default(),
        origin: ExecOrigin::User,
    })
    .reply
    .await
    .expect("the actor replies")
}

async fn exec(h: &SessionHandle, sql: &str) {
    if let Err(e) = run(h, sql).await {
        panic!("{sql}: {:?}", e.kind);
    }
}

impl Lite {
    fn spec(&self, database: Option<&str>) -> MetaConnSpec {
        MetaConnSpec {
            profile: self.profile.id,
            database: database.map(str::to_string),
            statement_timeout: Duration::from_secs(60),
        }
    }

    /// The separate `mode=ro` reader.
    async fn reader(&self) -> MetaConn {
        self.sm
            .meta_conn(self.spec(None), &CancellationToken::new())
            .await
            .expect("a reader")
    }

    /// The user's own connection (the `temp` schema).
    async fn temp_conn(&self) -> MetaConn {
        self.sm
            .meta_conn(self.spec(Some("temp")), &CancellationToken::new())
            .await
            .expect("the user's idle session")
    }

    async fn sql(&self, sql: &str) {
        exec(&self.user, sql).await;
    }

    /// One cell of `sql` run on the user's session.
    async fn scalar(&self, sql: &str) -> String {
        let s = run(&self.user, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
        s.rows[0][0].clone().expect("a value")
    }
}

fn main_ns() -> NsKey {
    NsKey::schema(["main"])
}

fn temp_ns() -> NsKey {
    NsKey::schema(["temp"])
}

fn intro() -> SqliteIntrospector {
    SqliteIntrospector::default()
}

/// The filter that keeps everything (the engine's for a namespace with no scope filter).
fn all() -> &'static CompiledFilter {
    static ALL: std::sync::LazyLock<CompiledFilter> = std::sync::LazyLock::new(|| {
        parse_filter("", DialectId::Sqlite, FilterRole::Introspect).expect("the empty filter")
    });
    &ALL
}

type Trace = Arc<Mutex<Vec<Name>>>;

fn traced() -> (SqliteIntrospector, Trace) {
    let t: Trace = Arc::default();
    (SqliteIntrospector::default().with_trace(t.clone()), t)
}

fn sections(t: &Trace) -> Vec<String> {
    t.lock().unwrap().iter().map(|n| n.to_string()).collect()
}

async fn pass(
    i: &SqliteIntrospector,
    c: &mut MetaConn,
    ns: &NsKey,
    mode: Mode,
    level: Level,
    prev: Option<&NamespaceSnapshot>,
) -> NamespaceResult {
    i.introspect(c, ns, mode, level, all(), prev, &CancellationToken::new())
        .await
        .unwrap_or_else(|e| panic!("{ns:?} {mode:?}: {e:?}"))
}

/// `r` merged into `prev` (or the empty namespace of a first load).
fn merged(ns: &NsKey, prev: Option<&NamespaceSnapshot>, r: NamespaceResult) -> NamespaceSnapshot {
    let start = match prev {
        Some(p) => p.clone(),
        None => base(ns.clone(), DbmsKind::Sqlite, FoldRule::AsciiCaseless).expect("a base"),
    };
    let policy = MergePolicy {
        guard_min_objects: u32::MAX,
        ..MergePolicy::default()
    };
    merge(Some(&start), r, &policy)
        .expect("the merge runs")
        .next()
        .clone()
}

/// A full pass at `level` on a fresh reader, merged into an empty namespace.
async fn full(l: &Lite, level: Level) -> NamespaceSnapshot {
    let mut c = l.reader().await;
    let r = pass(&intro(), &mut c, &main_ns(), Mode::Full, level, None).await;
    merged(&main_ns(), None, r)
}

/// Every object of `ns` as text, sorted: its key, flags, state and props (layout-free).
fn canon(ns: &NamespaceSnapshot) -> Vec<String> {
    let mut out: Vec<String> = (1..ns.len() as u32)
        .map(|i| {
            let o = ns.obj(i).unwrap();
            format!(
                "{:?} {:?} {:?} {:?}",
                ns.object_key(i).unwrap(),
                o.flags(),
                o.state(),
                o.props()
            )
        })
        .collect();
    out.sort();
    out
}

fn idx_of(ns: &NamespaceSnapshot, path: &[&str], kind: ObjectKind) -> Option<u32> {
    let key = ObjectKey {
        kind,
        path: ns
            .key()
            .path
            .iter()
            .cloned()
            .chain(path.iter().map(|s| Name::from(*s)))
            .collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    ns.find(&key)
}

fn obj<'a>(ns: &'a NamespaceSnapshot, path: &[&str], kind: ObjectKind) -> &'a Obj {
    let i = idx_of(ns, path, kind).unwrap_or_else(|| panic!("{path:?} {kind:?} is loaded"));
    ns.obj(i).unwrap()
}

fn children(ns: &NamespaceSnapshot, path: &[&str], kind: ObjectKind) -> Vec<String> {
    let parent = idx_of(ns, path, ObjectKind::Table)
        .or_else(|| idx_of(ns, path, ObjectKind::View))
        .or_else(|| idx_of(ns, path, ObjectKind::VirtualTable))
        .unwrap_or_else(|| panic!("{path:?} is loaded"));
    ns.children_of_kind(parent, kind)
        .map(|(_, o)| o.name().to_string())
        .collect()
}

fn sqlite_wm(ns: &NamespaceSnapshot) -> (u32, Option<(u64, u64)>, u128) {
    match &ns.meta().watermark {
        Watermark::Sqlite {
            schema_version,
            file_identity,
            digest,
        } => (*schema_version, *file_identity, *digest),
        w => panic!("a SQLite watermark: {w:?}"),
    }
}

fn column<'a>(
    ns: &'a NamespaceSnapshot,
    table: &str,
    col: &str,
) -> &'a hacivat_meta::props::ColumnProps {
    let o = obj(ns, &[table, col], ObjectKind::Column);
    o.props().as_column().expect("column props")
}

fn reread(r: &NamespaceResult) -> bool {
    r.full && !r.changed.is_empty()
}

// ---------------------------------------------------------------------------------------------
// alg/02 HV-INTRO-08 tests 1–6

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_skips_the_reread() {
    let l = lite("CREATE TABLE t(id)").await;
    let s1 = full(&l, Level::Structure).await;
    l.sql("INSERT INTO t VALUES (1)").await;
    let (i, trace) = traced();
    let mut c = l.reader().await;
    let r = pass(
        &i,
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    assert!(
        !r.full && r.changed.is_empty() && r.existent.is_empty(),
        "{r:?}"
    );
    let ran = sections(&trace);
    assert!(
        ran.iter().all(|s| !s.contains("Columns")),
        "no table_xinfo in the log: {ran:?}"
    );
    let s2 = merged(&main_ns(), Some(&s1), r);
    assert_eq!(canon(&s2), canon(&s1));
    assert_eq!(sqlite_wm(&s2), sqlite_wm(&s1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_index_bumps_and_rereads() {
    let l = lite("CREATE TABLE t(id)").await;
    let s1 = full(&l, Level::Structure).await;
    l.sql("CREATE INDEX i ON t(id)").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    assert!(reread(&r), "{r:?}");
    let s2 = merged(&main_ns(), Some(&s1), r);
    assert_eq!(sqlite_wm(&s2).0, sqlite_wm(&s1).0 + 1);
    assert_eq!(children(&s2, &["t"], ObjectKind::Index), vec!["i"]);
    // One reader per database at a time (T5's pool): give it back first.
    drop(c);
    assert_eq!(canon(&s2), canon(&full(&l, Level::Structure).await));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rolled_back_ddl_skips() {
    let l = lite("CREATE TABLE t(id)").await;
    let s1 = full(&l, Level::Structure).await;
    l.sql("BEGIN").await;
    l.sql("CREATE TABLE z(a)").await;
    l.sql("ROLLBACK").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    assert!(!r.full && r.changed.is_empty(), "{r:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn temp_table_rereads_only_temp_on_the_users_connection() {
    let l = lite("CREATE TABLE t(id)").await;
    let main1 = full(&l, Level::Structure).await;
    let mut tc = l.temp_conn().await;
    let r = pass(
        &intro(),
        &mut tc,
        &temp_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let temp1 = merged(&temp_ns(), None, r);
    assert!(
        temp1.is_temp(),
        "the temp root is flagged TEMP: never persisted"
    );
    drop(tc);

    l.sql("CREATE TEMP TABLE tt(a)").await;
    // main is untouched: its pass skips.
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&main1),
    )
    .await;
    assert!(!r.full && r.changed.is_empty(), "main skips: {r:?}");
    drop(c);
    // temp is read on the user's connection, where `tt` lives.
    let mut tc = l.temp_conn().await;
    let r = pass(
        &intro(),
        &mut tc,
        &temp_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&temp1),
    )
    .await;
    assert!(reread(&r), "{r:?}");
    let temp2 = merged(&temp_ns(), Some(&temp1), r);
    let tt = obj(&temp2, &["tt"], ObjectKind::Table);
    assert!(tt.flags().contains(ObjFlags::TEMP));
    assert_eq!(
        tt.props().as_table().map(|p| p.persistence),
        Some(Persistence::Temporary)
    );
    assert_eq!(children(&temp2, &["tt"], ObjectKind::Column), vec!["a"]);
    // The reader's own temp schema never shows the user's table.
    let mut c = l.reader().await;
    let mine = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    assert!(
        merged(&main_ns(), None, mine)
            .lookup(KindGroup::Relations, "tt")
            .is_empty()
    );
}

/// Overwrites `dst`'s bytes in place with `src`'s (same inode: a copy-in-place).
fn copy_in_place(src: &Path, dst: &Path) {
    use std::io::Write;
    let bytes = std::fs::read(src).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(dst)
        .unwrap();
    f.write_all(&bytes).unwrap();
    f.sync_all().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_schema_version_replaced_file_detected_on_reconnect_by_digest() {
    let l = lite("CREATE TABLE a1(x)").await;
    let other = file_with(l._dir.path(), "other.db", "CREATE TABLE b1(y)").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let s1 = merged(&main_ns(), None, r);
    let (sv1, id1, _) = sqlite_wm(&s1);
    assert_eq!(sv1, 1);
    // The user's session holds no lock; the file is replaced in place by a database whose
    // schema_version is the same.
    copy_in_place(&other, &l.path);
    // The fast path on the same connection: same schema_version, same identity, no digest.
    let r = intro()
        .pass(
            &mut c,
            &main_ns(),
            Mode::Incremental,
            Level::Structure,
            all(),
            Some(&s1),
            PassConn::Reused,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(
        !r.full && r.changed.is_empty(),
        "no detection on the fast path: {r:?}"
    );
    if let Watermark::Sqlite {
        schema_version,
        file_identity,
        ..
    } = &r.new_watermark
    {
        assert_eq!(
            (*schema_version, *file_identity),
            (sv1, id1),
            "same version and inode"
        );
    }
    drop(c);
    // A reconnect checks the digest: the change is found and the schema re-read.
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    assert!(reread(&r), "{r:?}");
    let s2 = merged(&main_ns(), Some(&s1), r);
    assert!(s2.lookup(KindGroup::Relations, "a1").is_empty());
    assert_eq!(s2.lookup(KindGroup::Relations, "b1").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_adds_a_namespace() {
    let l = lite("CREATE TABLE t(id)").await;
    let aux = file_with(
        l._dir.path(),
        "arsiv.db",
        "CREATE TABLE eski(k INTEGER PRIMARY KEY, v)",
    )
    .await;
    let i = intro();
    let mut c = l.reader().await;
    let before: Vec<NsKey> = i
        .list_namespaces(&mut c)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.key)
        .collect();
    assert_eq!(before, vec![main_ns(), temp_ns()]);
    c.attach_read_only("arsiv", &aux).await.unwrap();
    let infos = i.list_namespaces(&mut c).await.unwrap();
    let keys: Vec<NsKey> = infos.iter().map(|n| n.key.clone()).collect();
    let arsiv = NsKey::schema(["arsiv"]);
    assert_eq!(keys, vec![main_ns(), temp_ns(), arsiv.clone()]);
    assert!(infos.iter().all(|n| !n.system));
    let r = pass(&i, &mut c, &arsiv, Mode::Full, Level::Structure, None).await;
    let ns = merged(&arsiv, None, r);
    assert_eq!(children(&ns, &["eski"], ObjectKind::Column), vec!["k", "v"]);
    assert_eq!(
        column(&ns, "eski", "k").identity(),
        Some(Identity::RowidAlias)
    );
    // The alias the reader attached the file under is not a namespace of its own, and main is
    // still main.
    let keys: Vec<NsKey> = i
        .list_namespaces(&mut c)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.key)
        .collect();
    assert!(!keys.contains(&NsKey::schema([ATTACH_ALIAS])), "{keys:?}");
    let r = pass(&i, &mut c, &main_ns(), Mode::Full, Level::Structure, None).await;
    let m = merged(&main_ns(), None, r);
    assert_eq!(m.lookup(KindGroup::Relations, "t").len(), 1);
    assert!(m.lookup(KindGroup::Relations, "eski").is_empty());
    // A second pass of the attached schema on the same reader reuses the alias.
    let r = pass(
        &i,
        &mut c,
        &arsiv,
        Mode::Incremental,
        Level::Structure,
        Some(&ns),
    )
    .await;
    assert!(!r.full && r.changed.is_empty(), "{r:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn schema_version_sequence_matches_ver01() {
    // ver/01 #sqlite-schema-version: baseline 1 | INSERT 1 | CREATE INDEX 2 | ADD COLUMN 3 |
    // CREATE TEMP TABLE main 3, temp 1 | CREATE VIEW 4 | DROP VIEW 5 | rolled-back CREATE 5 |
    // VACUUM 6. Each step: the main watermark, and an incremental pass equal to a full one.
    let l = lite("CREATE TABLE t(id)").await;
    let mut cur = full(&l, Level::Structure).await;
    assert_eq!(sqlite_wm(&cur).0, 1, "baseline");
    let steps: [(&str, u32); 8] = [
        ("INSERT INTO t VALUES (1)", 1),
        ("CREATE INDEX i ON t(id)", 2),
        ("ALTER TABLE t ADD COLUMN c", 3),
        ("CREATE TEMP TABLE tt(a)", 3),
        ("CREATE VIEW v AS SELECT id FROM t", 4),
        ("DROP VIEW v", 5),
        ("BEGIN; CREATE TABLE z(a); ROLLBACK", 5),
        ("VACUUM", 6),
    ];
    for (sql, want) in steps {
        for stmt in sql.split("; ") {
            l.sql(stmt).await;
        }
        let mut c = l.reader().await;
        let r = pass(
            &intro(),
            &mut c,
            &main_ns(),
            Mode::Incremental,
            Level::Structure,
            Some(&cur),
        )
        .await;
        drop(c);
        cur = merged(&main_ns(), Some(&cur), r);
        assert_eq!(sqlite_wm(&cur).0, want, "{sql}");
        assert_eq!(
            canon(&cur),
            canon(&full(&l, Level::Structure).await),
            "{sql}"
        );
        if sql.starts_with("CREATE TEMP") {
            assert_eq!(l.scalar("PRAGMA temp.schema_version").await, "1");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Shapes (HV-MODEL-01 test 8, HV-INTRO-13 test 5, F10)

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_generated_column_hidden_3() {
    let l = lite("CREATE TABLE g(a INT, b INT GENERATED ALWAYS AS (a * 2) STORED)").await;
    let ns = full(&l, Level::Structure).await;
    let b = column(&ns, "g", "b");
    let g = b.generated().expect("generated");
    assert_eq!((g.kind, &*g.expr), (GeneratedKind::Stored, "a * 2"));
    assert!(!b.hidden, "a generated column is not a hidden one");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn virtual_and_stored_generated_columns_by_hidden_2_and_3() {
    let l = lite(
        "CREATE TABLE g(a INT, v AS (a + 1), s TEXT GENERATED ALWAYS AS (upper('x' || a)) STORED, \
         w INT AS (a*3) VIRTUAL)",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    let kinds: Vec<Option<(GeneratedKind, String)>> = ["a", "v", "s", "w"]
        .iter()
        .map(|c| {
            column(&ns, "g", c)
                .generated()
                .map(|g| (g.kind, g.expr.to_string()))
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            None,
            Some((GeneratedKind::Virtual, "a + 1".to_string())),
            Some((GeneratedKind::Stored, "upper('x' || a)".to_string())),
            Some((GeneratedKind::Virtual, "a*3".to_string())),
        ]
    );
    assert_eq!(column(&ns, "g", "s").raw_type.as_ref(), "TEXT");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unique_autoindex_and_pk() {
    let l = lite("CREATE TABLE u(a TEXT PRIMARY KEY, b UNIQUE, c)").await;
    let ns = full(&l, Level::Structure).await;
    let pk = obj(&ns, &["u", "u_pk"], ObjectKind::PrimaryKey);
    assert!(pk.flags().contains(ObjFlags::SERVER_GENERATED_NAME));
    let Props::Key(k) = pk.props() else {
        panic!("key props")
    };
    assert_eq!(k.backing_index.as_deref(), Some("sqlite_autoindex_u_1"));
    let uk = obj(&ns, &["u", "sqlite_autoindex_u_2"], ObjectKind::UniqueKey);
    assert!(uk.flags().contains(ObjFlags::SERVER_GENERATED_NAME));
    for ix in ["sqlite_autoindex_u_1", "sqlite_autoindex_u_2"] {
        let o = obj(&ns, &["u", ix], ObjectKind::Index);
        assert!(
            o.flags()
                .contains(ObjFlags::IMPLICIT | ObjFlags::SERVER_GENERATED_NAME),
            "{ix}: {:?}",
            o.flags()
        );
    }
    assert_eq!(
        children(&ns, &["u"], ObjectKind::Column),
        vec!["a", "b", "c"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_autoindex_is_implicit() {
    let l = lite(
        "CREATE TABLE k(a, b, c, PRIMARY KEY (b, a), UNIQUE (c)); CREATE UNIQUE INDEX own ON k(c, a)",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    let own = obj(&ns, &["k", "own"], ObjectKind::Index);
    assert!(
        !own.flags()
            .intersects(ObjFlags::IMPLICIT | ObjFlags::SERVER_GENERATED_NAME)
    );
    for ix in ["sqlite_autoindex_k_1", "sqlite_autoindex_k_2"] {
        assert!(
            obj(&ns, &["k", ix], ObjectKind::Index)
                .flags()
                .contains(ObjFlags::IMPLICIT)
        );
    }
    let Props::Key(pk) = obj(&ns, &["k", "k_pk"], ObjectKind::PrimaryKey).props() else {
        panic!("key props")
    };
    let cols: Vec<&str> = pk
        .columns
        .iter()
        .filter_map(|c| c.name.as_deref())
        .collect();
    assert_eq!(cols, vec!["b", "a"], "key order, not column order");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn strict_and_without_rowid_props() {
    let sql = "CREATE TABLE s(a INT PRIMARY KEY, b TEXT) STRICT, WITHOUT ROWID; \
               CREATE TABLE w(a PRIMARY KEY) WITHOUT ROWID; CREATE TABLE p(a INT) STRICT; \
               CREATE TABLE q(a)";
    let l = lite(sql).await;
    let ns = full(&l, Level::Structure).await;
    let ext = |t: &str| {
        obj(&ns, &[t], ObjectKind::Table)
            .props()
            .as_table()
            .unwrap()
            .ext
            .clone()
    };
    let want = |without_rowid, strict| TableExt::Sqlite {
        without_rowid,
        strict,
        module: None,
    };
    assert_eq!(ext("s"), want(true, true));
    assert_eq!(ext("w"), want(true, false));
    assert_eq!(ext("p"), want(false, true));
    assert_eq!(ext("q"), want(false, false));
    // The same answer from the text scan, on the user's connection (`temp`).
    for stmt in sql.replace("CREATE TABLE", "CREATE TEMP TABLE").split("; ") {
        l.sql(stmt).await;
    }
    let mut tc = l.temp_conn().await;
    let r = pass(
        &intro(),
        &mut tc,
        &temp_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let t = merged(&temp_ns(), None, r);
    let ext = |n: &str| {
        obj(&t, &[n], ObjectKind::Table)
            .props()
            .as_table()
            .unwrap()
            .ext
            .clone()
    };
    assert_eq!(ext("s"), want(true, true));
    assert_eq!(ext("w"), want(true, false));
    assert_eq!(ext("p"), want(false, true));
    assert_eq!(ext("q"), want(false, false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vacuum_rereads_harmlessly() {
    let l = lite("CREATE TABLE t(id INTEGER PRIMARY KEY, n TEXT); CREATE INDEX tn ON t(n)").await;
    let s1 = full(&l, Level::Structure).await;
    l.sql("VACUUM").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    assert!(r.full, "VACUUM bumps schema_version: a (spurious) re-read");
    let start = s1.clone();
    let policy = MergePolicy::default();
    let o = merge(Some(&start), r, &policy).unwrap();
    let hacivat_meta::merge::MergeOutcome::Applied {
        next: s2, delta, ..
    } = o
    else {
        panic!("applied")
    };
    assert!(
        delta.created.is_empty() && delta.dropped.is_empty() && delta.renamed.is_empty(),
        "nothing dropped or created: {delta:?}"
    );
    assert_eq!(sqlite_wm(&s2).0, sqlite_wm(&s1).0 + 1);
    // Marks are schema-level (schema_version + digest), so they move; the rest is unchanged.
    let strip = |v: Vec<String>| -> Vec<String> {
        v.into_iter()
            .map(|s| s.split(" Hash(").next().unwrap().to_string())
            .collect()
    };
    assert_eq!(strip(canon(&s2)), strip(canon(&s1)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn virtual_table_with_missing_module_lists_columns_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(
        dir.path(),
        "vt.db",
        "CREATE TABLE ok(a, b); CREATE VIRTUAL TABLE nm USING fts5(x, y)",
    )
    .await;
    // The driver's connections are defensive (no `writable_schema`): the module name is
    // patched in the file instead, to one of the same length that no build has.
    let bytes = std::fs::read(&path).unwrap();
    let (from, to) = (b"USING fts5(x, y)", b"USING hvx5(x, y)");
    let at = bytes
        .windows(from.len())
        .position(|w| w == from)
        .expect("the virtual table's text");
    let mut patched = bytes.clone();
    patched[at..at + to.len()].copy_from_slice(to);
    std::fs::write(&path, patched).unwrap();
    let l = lite_on(dir, path).await;
    let (i, trace) = traced();
    let mut c = l.reader().await;
    let r = pass(&i, &mut c, &main_ns(), Mode::Full, Level::Structure, None).await;
    let ns = merged(&main_ns(), None, r.clone());
    let nm = obj(&ns, &["nm"], ObjectKind::VirtualTable);
    assert!(nm.flags().contains(ObjFlags::BROKEN));
    let Props::Table(t) = nm.props() else {
        panic!("table props")
    };
    assert!(matches!(
        &t.ext,
        TableExt::Sqlite { module: Some(m), .. } if &**m == "hvx5"
    ));
    assert!(
        children(&ns, &["nm"], ObjectKind::Column).is_empty(),
        "no columns"
    );
    assert_eq!(children(&ns, &["ok"], ObjectKind::Column), vec!["a", "b"]);
    // Its columns are not decided (never "missing"): the column set leaves `nm` out of scope.
    let Some(ExistSet::Scoped { scope, .. }) = r.existent.get(&ObjectKind::Column) else {
        panic!(
            "a scoped column set: {:?}",
            r.existent.get(&ObjectKind::Column)
        )
    };
    assert!(!scope.contains(&vec![Name::from("nm")]), "{scope:?}");
    assert!(scope.contains(&vec![Name::from("ok")]));
    let ran = sections(&trace);
    assert_eq!(
        ran.iter().filter(|s| *s == "RelationColumns").count(),
        7,
        "only the tables fell back, one query each (ok, nm and its five shadow tables): {ran:?}"
    );
}

async fn lite_on(dir: tempfile::TempDir, path: PathBuf) -> Lite {
    let profile = profile_on(&path, false);
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps()),
        Arc::new(budget_for(&profile)),
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    Lite {
        _dir: dir,
        path,
        profile,
        sm,
        user,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn locked_database_retries_then_fails_softly() {
    let l = lite("CREATE TABLE t(id)").await;
    let mut c = l.reader().await;
    l.sql("BEGIN EXCLUSIVE").await;
    let (i, trace) = traced();
    let got = i
        .introspect(
            &mut c,
            &main_ns(),
            Mode::Full,
            Level::Structure,
            all(),
            None,
            &CancellationToken::new(),
        )
        .await;
    let Err(IntroError::Meta(MetaError::Server { message, .. })) = &got else {
        panic!("a soft server error: {got:?}")
    };
    assert_eq!(
        message.vendor_code.map(|c| c & 0xff),
        Some(5),
        "SQLITE_BUSY"
    );
    assert_eq!(
        sections(&trace)
            .iter()
            .filter(|s| *s == "DatabaseList")
            .count(),
        2,
        "one retry after the busy timeout"
    );
    l.sql("ROLLBACK").await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    assert_eq!(
        merged(&main_ns(), None, r)
            .lookup(KindGroup::Relations, "t")
            .len(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identifier_case_rules_are_ascii_only() {
    // ver/01 #sqlite-case, F6: SQLite folds ASCII letters only.
    let l = lite("CREATE TABLE \"Müşteri\"(a); CREATE TABLE ITEMS(b)").await;
    let dup = run(&l.user, "CREATE TABLE \"müşteri\"(a)").await;
    assert!(dup.is_err(), "Müşteri and müşteri are the same table");
    l.sql("CREATE TABLE \"MÜŞTERİ\"(c)").await;
    let ns = full(&l, Level::Structure).await;
    let at = |n: &str| ns.lookup(KindGroup::Relations, n).to_vec();
    let mus = at("Müşteri");
    assert_eq!(mus.len(), 1);
    assert_eq!(at("müşteri"), mus, "M and m fold together");
    let upper = at("MÜŞTERİ");
    assert_eq!(upper.len(), 1);
    assert_ne!(upper, mus, "Ü ≠ ü and İ ≠ i: another table");
    assert_eq!(at("items"), at("ITEMS"));
    assert_eq!(
        ns.obj(at("items")[0]).unwrap().name().as_ref(),
        "ITEMS",
        "names verbatim"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn builtins_come_from_function_list_live() {
    let l = lite("").await;
    let mut c = l.reader().await;
    let b = intro().builtins(&mut c).await.unwrap();
    let f = |n: &str| b.functions.iter().find(|f| &*f.name == n);
    let abs = f("abs").expect("abs");
    assert!(abs.builtin && abs.deterministic);
    assert_eq!(abs.narg, 1);
    assert!(f("count").is_some() && f("json_extract").is_some());
    let collations: Vec<&str> = b.collations.iter().map(|n| &**n).collect();
    for want in ["BINARY", "NOCASE", "RTRIM"] {
        assert!(collations.contains(&want), "{collations:?}");
    }
    let modules = b.modules.expect("module_list on the reader");
    assert!(modules.iter().any(|m| &**m == "fts5"), "{modules:?}");
    // Live: no snapshot, nothing in a namespace (one reader at a time: give it back).
    drop(c);
    let ns = full(&l, Level::Sources).await;
    assert_eq!(ns.len(), 1, "an empty main holds no built-in object");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broken_view_falls_back_per_relation_and_is_kept_broken() {
    let l = lite(
        "CREATE TABLE t(a, b); CREATE TABLE gone(x); CREATE VIEW ok AS SELECT a FROM t; \
         CREATE VIEW bad AS SELECT x FROM gone; DROP TABLE gone",
    )
    .await;
    let (i, trace) = traced();
    let mut c = l.reader().await;
    let r = pass(&i, &mut c, &main_ns(), Mode::Full, Level::Structure, None).await;
    let ns = merged(&main_ns(), None, r);
    let bad = obj(&ns, &["bad"], ObjectKind::View);
    assert!(bad.flags().contains(ObjFlags::BROKEN));
    assert!(children(&ns, &["bad"], ObjectKind::Column).is_empty());
    assert!(
        !obj(&ns, &["ok"], ObjectKind::View)
            .flags()
            .contains(ObjFlags::BROKEN)
    );
    assert_eq!(children(&ns, &["ok"], ObjectKind::Column), vec!["a"]);
    assert_eq!(children(&ns, &["t"], ObjectKind::Column), vec!["a", "b"]);
    let ran = sections(&trace);
    assert!(
        ran.contains(&"TableColumnsMain".to_string()),
        "tables in bulk: {ran:?}"
    );
    assert_eq!(
        ran.iter().filter(|s| *s == "RelationColumns").count(),
        2,
        "only the views fell back: {ran:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expression_index_part_has_no_name_and_server_text() {
    let l =
        lite("CREATE TABLE t(a, b TEXT); CREATE INDEX e ON t(lower(b) DESC, a) WHERE a > 0").await;
    let ns = full(&l, Level::Structure).await;
    let Props::Index(ix) = obj(&ns, &["t", "e"], ObjectKind::Index).props() else {
        panic!("index props")
    };
    let names: Vec<Option<&str>> = ix.key.columns.iter().map(|c| c.name.as_deref()).collect();
    assert_eq!(names, vec![None, Some("a")]);
    assert_eq!(ix.key.expressions, vec![Name::from("lower(b)")]);
    assert_eq!(ix.key.desc, vec![true, false]);
    assert_eq!(ix.key.predicate.as_deref(), Some("a > 0"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fts5_shadow_tables_flagged_on_the_metadata_connection() {
    let l = lite("CREATE VIRTUAL TABLE docs USING fts5(title, body)").await;
    let ns = full(&l, Level::Structure).await;
    let docs = obj(&ns, &["docs"], ObjectKind::VirtualTable);
    assert!(matches!(
        &docs.props().as_table().unwrap().ext,
        TableExt::Sqlite { module: Some(m), .. } if &**m == "fts5"
    ));
    let cols = children(&ns, &["docs"], ObjectKind::Column);
    assert_eq!(cols, vec!["title", "body", "docs", "rank"]);
    assert!(
        column(&ns, "docs", "rank").hidden,
        "hidden 1: a virtual table's hidden column"
    );
    assert!(!column(&ns, "docs", "title").hidden);
    for shadow in [
        "docs_data",
        "docs_idx",
        "docs_content",
        "docs_docsize",
        "docs_config",
    ] {
        let o = obj(&ns, &[shadow], ObjectKind::Table);
        assert!(
            o.flags().contains(ObjFlags::IMPLICIT),
            "{shadow}: {:?}",
            o.flags()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_rowid_alias_identity() {
    let l = lite(
        "CREATE TABLE r(id INTEGER PRIMARY KEY, x); \
         CREATE TABLE ra(id integer primary key autoincrement, x); \
         CREATE TABLE rd(id INTEGER PRIMARY KEY DESC, x); \
         CREATE TABLE ri(id INT PRIMARY KEY, x); \
         CREATE TABLE rw(id INTEGER PRIMARY KEY, x) WITHOUT ROWID; \
         CREATE TABLE rk(id INTEGER, x, PRIMARY KEY (id))",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    let id = |t: &str| column(&ns, t, "id").identity();
    assert_eq!(id("r"), Some(Identity::RowidAlias));
    assert_eq!(id("ra"), Some(Identity::AutoIncrement));
    assert_eq!(
        id("rd"),
        None,
        "INTEGER PRIMARY KEY DESC has an index: not an alias"
    );
    assert_eq!(id("ri"), None, "INT is not INTEGER");
    assert_eq!(id("rw"), None, "WITHOUT ROWID has no rowid");
    assert_eq!(
        id("rk"),
        Some(Identity::RowidAlias),
        "a table-level key on one INTEGER column"
    );
    assert!(
        children(&ns, &["r"], ObjectKind::Index).is_empty(),
        "no index"
    );
    assert_eq!(children(&ns, &["r"], ObjectKind::PrimaryKey), vec!["r_pk"]);
    assert_eq!(
        obj(&ns, &["sqlite_sequence"], ObjectKind::Table).flags() & ObjFlags::SYSTEM,
        ObjFlags::SYSTEM
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_joined_read_uses_the_writer_connection() {
    // A SQLite profile has one session: a `Job(DdlApply)` request gets the console, and T5
    // refuses to join it (the writer seam is M7's, `MetaConn::join` docs). What T18 owns is
    // that the introspector reads on the connection it is handed, never opening the `mode=ro`
    // reader itself, and that every section renders for a joined connection (the pack test).
    let l = lite("CREATE TABLE t(id)").await;
    l.sql("BEGIN").await;
    l.sql("CREATE TABLE pending_t(a)").await;
    let mut st = l.user.state();
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        st.wait_for(|s| s.tx.status == hacivat_driver::TxStatus::InTx),
    )
    .await;
    assert!(
        matches!(MetaConn::join(&l.user), Err(MetaError::NotJoinable)),
        "no joinable SQLite writer before M7"
    );
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    assert!(
        merged(&main_ns(), None, r)
            .lookup(KindGroup::Relations, "pending_t")
            .is_empty(),
        "the mode=ro reader sees only committed state"
    );
    drop(c);
    l.sql("ROLLBACK").await;

    // An in-memory profile has no file to open read-only: the metadata connection is the
    // writer itself, and the pass reads there.
    let profile = Profile {
        id: pid(),
        name: "mem".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: PathBuf::from(":memory:"),
            create_if_missing: false,
        },
        user: None,
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    };
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps()),
        Arc::new(budget_for(&profile)),
    );
    let user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    exec(&user, "CREATE TABLE only_in_memory(a INTEGER PRIMARY KEY)").await;
    let mut c = sm
        .meta_conn(
            MetaConnSpec {
                profile: profile.id,
                database: None,
                statement_timeout: Duration::from_secs(60),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let ns = merged(&main_ns(), None, r);
    assert_eq!(
        children(&ns, &["only_in_memory"], ObjectKind::Column),
        vec!["a"],
        "read on the writer's own connection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn temp_schema_read_uses_only_authorizer_allowed_pragmas() {
    // R-M4-38: on the user's own connection only table_xinfo, index_list, index_xinfo,
    // foreign_key_list, database_list, function_list, collation_list, schema_version.
    let allowed = |s: &str| {
        matches!(
            s,
            "DatabaseList"
                | "ObjectsTemp"
                | "TableColumnsTemp"
                | "ViewColumnsTemp"
                | "RelationColumns"
                | "IndexesTemp"
                | "IndexPartsTemp"
                | "ForeignKeysTemp"
                | "FunctionList"
                | "CollationList"
        )
    };
    // A read-only profile: the user's connection carries the read-only authorizer.
    let l = lite_with("CREATE TABLE t(id)", true).await;
    let (i, trace) = traced();
    let mut tc = l.temp_conn().await;
    let r = pass(&i, &mut tc, &temp_ns(), Mode::Full, Level::Sources, None).await;
    assert!(r.full);
    let ran = sections(&trace);
    assert!(ran.iter().all(|s| allowed(s)), "{ran:?}");
    // The built-ins: function and collation lists are allowed, module_list is refused there.
    let b = i
        .builtins(&mut tc)
        .await
        .expect("function and collation lists are allowed");
    assert!(
        b.modules.is_none(),
        "module_list is not on the authorizer's list"
    );
    assert!(!b.functions.is_empty());

    // A read-write profile with temp objects of every shape.
    let l = lite("CREATE TABLE p(id INTEGER PRIMARY KEY)").await;
    l.sql("CREATE TEMP TABLE tt(a INTEGER PRIMARY KEY, b TEXT UNIQUE, c REFERENCES p(id))")
        .await;
    l.sql("CREATE INDEX temp.tb ON tt(b, a)").await;
    l.sql("CREATE TEMP VIEW tv AS SELECT a FROM tt").await;
    l.sql("CREATE TEMP TRIGGER tg AFTER INSERT ON tt BEGIN SELECT 1; END")
        .await;
    let (i, trace) = traced();
    let mut tc = l.temp_conn().await;
    let r = pass(&i, &mut tc, &temp_ns(), Mode::Full, Level::Sources, None).await;
    let ns = merged(&temp_ns(), None, r);
    assert_eq!(children(&ns, &["tt"], ObjectKind::Index).len(), 2);
    assert_eq!(children(&ns, &["tt"], ObjectKind::ForeignKey).len(), 1);
    assert_eq!(children(&ns, &["tt"], ObjectKind::Trigger), vec!["tg"]);
    assert_eq!(children(&ns, &["tv"], ObjectKind::Column), vec!["a"]);
    let ran = sections(&trace);
    assert!(ran.iter().all(|s| allowed(s)), "{ran:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "perf"]
async fn ten_thousand_tables_structure_recorded() {
    // Spike: Names 44 ms, Structure 245 ms, unchanged check 17 ms (release); recorded, not
    // gating.
    if !hacivat_testkit::perf_enabled() {
        return;
    }
    let mut sql = String::from("BEGIN;");
    for t in 0..10_000 {
        sql.push_str(&format!(
            "CREATE TABLE t{t}(id INTEGER PRIMARY KEY, c1 TEXT, c2 TEXT, c3 INT, c4 INT, c5 REAL, \
             c6 BLOB, c7 TEXT, c8 TEXT, c9 INT, c10 INT, c11 TEXT, c12 TEXT, c13 INT, c14 INT, \
             c15 TEXT); CREATE INDEX t{t}_i ON t{t}(c1, c2);"
        ));
    }
    sql.push_str("COMMIT");
    let l = lite(&sql).await;
    let timed = |label: &'static str| {
        let start = std::time::Instant::now();
        move || eprintln!("{label}: {} ms", start.elapsed().as_millis())
    };
    let mut c = l.reader().await;
    let done = timed("Names");
    let r = pass(&intro(), &mut c, &main_ns(), Mode::Full, Level::Names, None).await;
    done();
    drop(r);
    let done = timed("Structure");
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    done();
    let s = merged(&main_ns(), None, r);
    assert_eq!(s.column_count(), 160_000);
    let done = timed("Unchanged check (fresh: digest)");
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s),
    )
    .await;
    done();
    assert!(!r.full);
    let done = timed("Unchanged check (reused: schema_version only)");
    let r = intro()
        .pass(
            &mut c,
            &main_ns(),
            Mode::Incremental,
            Level::Structure,
            all(),
            Some(&s),
            PassConn::Reused,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    done();
    assert!(!r.full);
}

// ---------------------------------------------------------------------------------------------
// Beyond the plan's list: keys, existence, freshness, facets, the pack, the reader's safety

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fk_deferral_and_content_names_come_from_the_table_sql() {
    let l = lite(
        "CREATE TABLE p(id INTEGER PRIMARY KEY, k UNIQUE); \
         CREATE TABLE c(a REFERENCES p(id) DEFERRABLE INITIALLY DEFERRED, b, \
           \"x, y\" INT CONSTRAINT nn NOT NULL, \
           FOREIGN KEY (b) REFERENCES p(k) ON DELETE CASCADE NOT DEFERRABLE INITIALLY DEFERRED, \
           CHECK (b <> 'REFERENCES q DEFERRABLE'))",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    let fks = children(&ns, &["c"], ObjectKind::ForeignKey);
    assert_eq!(fks, vec!["c_a_p_fkey", "c_b_p_fkey"]);
    let fk = |n: &str| match obj(&ns, &["c", n], ObjectKind::ForeignKey).props() {
        Props::ForeignKey(f) => (f.deferrable, f.initially_deferred),
        p => panic!("{p:?}"),
    };
    assert_eq!(fk("c_a_p_fkey"), (true, true));
    assert_eq!(fk("c_b_p_fkey"), (false, false), "NOT DEFERRABLE");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn index_collation_is_kept_only_where_it_differs_from_the_column() {
    let l = lite(
        "CREATE TABLE cc(a TEXT COLLATE NOCASE, \"b c\" TEXT, d TEXT COLLATE rtrim); \
         CREATE INDEX ia ON cc(a); CREATE INDEX ib ON cc(\"b c\" COLLATE NOCASE); \
         CREATE INDEX id ON cc(d COLLATE BINARY, a COLLATE nocase)",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    assert_eq!(
        column(&ns, "cc", "a").collation().map(|c| &**c),
        Some("NOCASE")
    );
    assert_eq!(column(&ns, "cc", "b c").collation(), None);
    let coll = |ix: &str| -> Vec<Option<String>> {
        let Props::Index(p) = obj(&ns, &["cc", ix], ObjectKind::Index).props() else {
            panic!("index props")
        };
        match &p.key.rare {
            None => vec![None; p.key.columns.len()],
            Some(r) if r.collation.is_empty() => vec![None; p.key.columns.len()],
            Some(r) => r
                .collation
                .iter()
                .map(|c| c.as_ref().map(|c| c.to_string()))
                .collect(),
        }
    };
    assert_eq!(coll("ia"), vec![None], "the column's own collation");
    assert_eq!(coll("ib"), vec![Some("NOCASE".to_string())]);
    assert_eq!(coll("id"), vec![Some("BINARY".to_string()), None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn triggers_and_sources_at_the_sources_level() {
    let l = lite(
        "CREATE TABLE t(a, b); CREATE VIEW v AS SELECT a FROM t; \
         CREATE TRIGGER tr_upd BEFORE UPDATE OF a ON t FOR EACH ROW WHEN new.a > 0 BEGIN SELECT 1; END; \
         CREATE TRIGGER tr_v INSTEAD OF DELETE ON v BEGIN SELECT 2; END; \
         CREATE TRIGGER \"after\" AFTER INSERT ON t BEGIN SELECT 3; END",
    )
    .await;
    let s = full(&l, Level::Structure).await;
    let tr =
        |ns: &NamespaceSnapshot, path: &[&str]| match obj(ns, path, ObjectKind::Trigger).props() {
            Props::Trigger(p) => (**p).clone(),
            p => panic!("{p:?}"),
        };
    let upd = tr(&s, &["t", "tr_upd"]);
    assert_eq!(
        (upd.timing, upd.events),
        (TriggerTiming::Before, TriggerEvents::UPDATE)
    );
    assert!(upd.for_each_row);
    assert_eq!(upd.when_expr.as_deref(), Some("new.a > 0"));
    assert_eq!(upd.source, None, "no source text below the Sources level");
    let v = tr(&s, &["v", "tr_v"]);
    assert_eq!(
        (v.timing, v.events),
        (TriggerTiming::InsteadOf, TriggerEvents::DELETE)
    );
    let a = tr(&s, &["t", "after"]);
    assert_eq!(
        (a.timing, a.events),
        (TriggerTiming::After, TriggerEvents::INSERT)
    );
    let src = full(&l, Level::Sources).await;
    let text = "CREATE TRIGGER tr_v INSTEAD OF DELETE ON v BEGIN SELECT 2; END";
    assert_eq!(
        tr(&src, &["v", "tr_v"]).source,
        Some(hacivat_meta::SourceRef::of(text))
    );
    let Props::View(vp) = obj(&src, &["v"], ObjectKind::View).props() else {
        panic!("view props")
    };
    assert_eq!(
        vp.source,
        Some(hacivat_meta::SourceRef::of(
            "CREATE VIEW v AS SELECT a FROM t"
        ))
    );
    // The Sources facet carries the texts themselves.
    let mut c = l.reader().await;
    let keys = [
        src.object_key(idx_of(&src, &["v"], ObjectKind::View).unwrap())
            .unwrap(),
        src.object_key(idx_of(&src, &["t"], ObjectKind::Table).unwrap())
            .unwrap(),
    ];
    let FacetRows::Sources(rows) = intro()
        .facet(&mut c, &main_ns(), &keys, FacetKind::Sources)
        .await
        .unwrap()
    else {
        panic!("sources")
    };
    assert_eq!(rows[0].1.as_ref(), "CREATE VIEW v AS SELECT a FROM t");
    assert_eq!(rows[1].1.as_ref(), "CREATE TABLE t(a, b)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_pass_existence_sets_decide_children_only_under_read_relations() {
    let l = lite("CREATE TABLE t(a); CREATE VIEW v AS SELECT a FROM t").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let rel = |k: ObjectKind| match r.existent.get(&k) {
        Some(ExistSet::Ok(ids)) => ids.iter().map(|i| i.path.clone()).collect::<Vec<_>>(),
        other => panic!("{k:?}: {other:?}"),
    };
    assert_eq!(rel(ObjectKind::Table), vec![vec![Name::from("t")]]);
    assert_eq!(rel(ObjectKind::View), vec![vec![Name::from("v")]]);
    assert_eq!(rel(ObjectKind::VirtualTable), Vec::<Vec<Name>>::new());
    for k in [
        ObjectKind::Column,
        ObjectKind::Index,
        ObjectKind::PrimaryKey,
        ObjectKind::UniqueKey,
        ObjectKind::ForeignKey,
        ObjectKind::Trigger,
    ] {
        assert!(
            matches!(r.existent.get(&k), Some(ExistSet::Scoped { .. })),
            "{k:?}: {:?}",
            r.existent.get(&k)
        );
    }
    assert!(
        !r.existent.contains_key(&ObjectKind::Check),
        "CHECKs are not modelled"
    );
    assert_eq!(r.level, Level::Structure);
    assert!(r.read_at_ms > 0);
    // A Names pass decides only the relations: cached columns stay.
    let s = merged(&main_ns(), None, r);
    let r = pass(&intro(), &mut c, &main_ns(), Mode::Full, Level::Names, None).await;
    assert_eq!(
        r.existent.keys().copied().collect::<Vec<_>>(),
        vec![
            ObjectKind::Table,
            ObjectKind::View,
            ObjectKind::VirtualTable
        ]
    );
    let s2 = merged(&main_ns(), Some(&s), r);
    assert_eq!(children(&s2, &["t"], ObjectKind::Column), vec!["a"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragment_rereads_the_requested_relations_scoped() {
    let l = lite("CREATE TABLE a(x); CREATE TABLE b(y)").await;
    let s1 = full(&l, Level::Structure).await;
    l.sql("ALTER TABLE a ADD COLUMN x2").await;
    l.sql("ALTER TABLE b ADD COLUMN y2").await;
    l.sql("DROP TABLE b").await;
    let key = |t: &str| {
        s1.object_key(idx_of(&s1, &[t], ObjectKind::Table).unwrap())
            .unwrap()
    };
    let mut c = l.reader().await;
    let r = intro()
        .fragment(
            &mut c,
            &main_ns(),
            &[key("a")],
            Level::Structure,
            all(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!r.full);
    assert_eq!(
        r.new_watermark,
        Watermark::None,
        "a fragment never vouches for the namespace"
    );
    let Some(ExistSet::Scoped { scope, .. }) = r.existent.get(&ObjectKind::Table) else {
        panic!("{:?}", r.existent)
    };
    assert_eq!(scope, &vec![vec![Name::from("a")]]);
    let s2 = merged(&main_ns(), Some(&s1), r);
    assert_eq!(children(&s2, &["a"], ObjectKind::Column), vec!["x", "x2"]);
    assert_eq!(
        children(&s2, &["b"], ObjectKind::Column),
        vec!["y"],
        "b was not asked for: kept as cached"
    );
    // A fragment of a dropped table drops it.
    let r = intro()
        .fragment(
            &mut c,
            &main_ns(),
            &[key("b")],
            Level::Structure,
            all(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let s3 = merged(&main_ns(), Some(&s2), r);
    assert!(s3.lookup(KindGroup::Relations, "b").is_empty());
    // The next incremental pass re-reads (the fragment left no watermark).
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s3),
    )
    .await;
    assert!(r.full);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stamps_are_schema_level_and_none_for_what_is_gone() {
    let l = lite("CREATE TABLE t(a, b); CREATE INDEX ti ON t(a)").await;
    let s = full(&l, Level::Structure).await;
    let k = |path: &[&str], kind| s.object_key(idx_of(&s, path, kind).unwrap()).unwrap();
    let keys = vec![
        k(&["t"], ObjectKind::Table),
        k(&["t", "b"], ObjectKind::Column),
        k(&["t", "ti"], ObjectKind::Index),
    ];
    let mut c = l.reader().await;
    let marks = intro().stamps(&mut c, &keys).await.unwrap();
    let mark = obj(&s, &["t"], ObjectKind::Table).state();
    assert!(matches!(mark, StateMark::Hash(_)));
    assert_eq!(
        marks,
        vec![Some(mark); 3],
        "one schema-level mark, as loaded"
    );
    for (i, _) in keys.iter().enumerate() {
        assert_eq!(s.obj(s.find(&keys[i]).unwrap()).unwrap().state(), mark);
    }
    l.sql("DROP INDEX ti").await;
    l.sql("ALTER TABLE t DROP COLUMN b").await;
    let marks = intro().stamps(&mut c, &keys).await.unwrap();
    assert!(
        matches!(marks[0], Some(m) if m != mark),
        "the schema changed: {marks:?}"
    );
    assert_eq!(&marks[1..], &[None, None]);
    let st = intro().check_state(&mut c, &keys).await.unwrap();
    assert_eq!(
        st,
        vec![Staleness::Unknown, Staleness::Dropped, Staleness::Dropped]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_namespace_matches_the_cached_namespace_until_the_schema_changes() {
    let l = lite("CREATE TABLE t(a); CREATE VIEW v AS SELECT a FROM t").await;
    let s = full(&l, Level::Structure).await;
    let mut c = l.reader().await;
    let server = intro()
        .check_namespace(&mut c, &main_ns(), &s)
        .await
        .unwrap();
    assert_eq!(Some(server.clone()), ns_check_of(&s));
    assert_eq!(server.objects, 2);
    l.sql("INSERT INTO t VALUES (1)").await;
    assert_eq!(
        intro()
            .check_namespace(&mut c, &main_ns(), &s)
            .await
            .unwrap(),
        server
    );
    l.sql("CREATE TABLE u(b)").await;
    let after = intro()
        .check_namespace(&mut c, &main_ns(), &s)
        .await
        .unwrap();
    assert_ne!(Some(after), ns_check_of(&s));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn other_facets_need_no_read() {
    let l = lite("CREATE TABLE t(a)").await;
    let s = full(&l, Level::Structure).await;
    let key = s
        .object_key(idx_of(&s, &["t"], ObjectKind::Table).unwrap())
        .unwrap();
    let mut c = MetaConn::offline(DbmsKind::Sqlite);
    let i = intro();
    let FacetRows::Grants(g) = i
        .facet(
            &mut c,
            &main_ns(),
            std::slice::from_ref(&key),
            FacetKind::Grants,
        )
        .await
        .unwrap()
    else {
        panic!("grants")
    };
    assert!(
        g[0].1.entries.is_empty(),
        "SQLite has no privileges: not applicable"
    );
    let FacetRows::Statistics(st) = i
        .facet(
            &mut c,
            &main_ns(),
            std::slice::from_ref(&key),
            FacetKind::Statistics,
        )
        .await
        .unwrap()
    else {
        panic!("statistics")
    };
    assert_eq!(st[0].1.row_estimate, None);
    assert_eq!(
        i.facet(&mut c, &main_ns(), &[key], FacetKind::Dependencies)
            .await,
        Err(IntroError::NotImplemented),
        "dependencies are unsupported on SQLite"
    );
    let caps = i.capabilities();
    assert!(caps.incremental && caps.transactional_ddl);
    assert_eq!(
        caps.facets,
        vec![FacetKind::Sources, FacetKind::Grants, FacetKind::Statistics]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attached_namespace_the_connection_lacks_fails_closed() {
    let l = lite("CREATE TABLE t(a)").await;
    let mut c = l.reader().await;
    let got = intro()
        .introspect(
            &mut c,
            &NsKey::schema(["nowhere"]),
            Mode::Full,
            Level::Structure,
            all(),
            None,
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(got, Err(IntroError::Meta(MetaError::Unavailable))),
        "never an empty namespace: {got:?}"
    );
    // On the user's connection an attached file cannot be re-attached under the alias.
    let aux = file_with(l._dir.path(), "x.db", "CREATE TABLE xt(a)").await;
    l.sql(&format!("ATTACH '{}' AS xa", aux.display())).await;
    let mut tc = l.temp_conn().await;
    let infos = intro().list_namespaces(&mut tc).await.unwrap();
    assert!(infos.iter().any(|n| n.key == NsKey::schema(["xa"])));
    let files = intro().attached_files(&mut tc).await.unwrap();
    // Compare canonical forms on both sides: Windows reports 8.3 short names and `\\?\` prefixes.
    let files: Vec<_> = files
        .into_iter()
        .map(|(n, p)| (n, p.canonicalize().unwrap()))
        .collect();
    assert_eq!(files, vec![(Name::from("xa"), aux.canonicalize().unwrap())]);
    let got = intro()
        .introspect(
            &mut tc,
            &NsKey::schema(["xa"]),
            Mode::Full,
            Level::Structure,
            all(),
            None,
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(got, Err(IntroError::Meta(MetaError::Unavailable))),
        "{got:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_never_writes_the_files() {
    let l = lite("CREATE TABLE t(a INTEGER PRIMARY KEY, b); CREATE INDEX tb ON t(b)").await;
    let aux = file_with(l._dir.path(), "ek.db", "CREATE TABLE z(q)").await;
    let digest = |p: &Path| {
        (
            std::fs::read(p).unwrap(),
            std::fs::metadata(p).unwrap().modified().unwrap(),
        )
    };
    let (m0, a0) = (digest(&l.path), digest(&aux));
    let i = intro();
    let mut c = l.reader().await;
    c.attach_read_only("aux", &aux).await.unwrap();
    for ns in [main_ns(), NsKey::schema(["aux"])] {
        for level in [Level::Names, Level::Structure, Level::Sources] {
            pass(&i, &mut c, &ns, Mode::Full, level, None).await;
        }
    }
    let _ = i.builtins(&mut c).await.unwrap();
    drop(c);
    assert!(
        digest(&l.path) == m0 && digest(&aux) == a0,
        "the files are byte-identical"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn case_sensitive_like_on_the_users_connection_changes_nothing() {
    let l = lite("CREATE TABLE \"Abc\"(a)").await;
    l.sql("CREATE TEMP TABLE \"Xyz\"(b)").await;
    let mut tc = l.temp_conn().await;
    let r1 = pass(
        &intro(),
        &mut tc,
        &temp_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    drop(tc);
    l.sql("PRAGMA case_sensitive_like = ON").await;
    let mut tc = l.temp_conn().await;
    let r2 = pass(
        &intro(),
        &mut tc,
        &temp_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    assert_eq!(
        canon(&merged(&temp_ns(), None, r1)),
        canon(&merged(&temp_ns(), None, r2))
    );
}

#[test]
fn the_sqlite_pack_is_pragma_functions_only_and_joinable() {
    let pack = Pack::builtin(DialectId::Sqlite).expect("the SQLite pack parses");
    assert_eq!(lint(&pack, DialectId::Sqlite), Vec::new());
    assert!(!pack.sections().is_empty());
    for name in pack.sections() {
        let s = pack.section(name).unwrap();
        assert!(s.joinable(), "{name}: a joined read runs the same text");
    }
    for (file, text) in hacivat_introspect::pack::SQLITE_FILES {
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_uppercase();
        for word in [
            "PRAGMA ",
            " LIKE ",
            " GLOB ",
            "COLLATE",
            "LOWER(",
            "UPPER(",
            "SQLITE_MASTER",
        ] {
            assert!(!code.contains(word), "{file}: {word}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_file_fingerprint_survives_ddl_and_vacuum_and_tells_files_apart() {
    let l = lite("CREATE TABLE t(a)").await;
    let i = intro();
    let mut c = l.reader().await;
    let f1 = i.file_fingerprint(&mut c).await.unwrap().expect("a file");
    drop(c);
    l.sql("CREATE TABLE u(b)").await;
    l.sql("VACUUM").await;
    let mut c = l.reader().await;
    assert_eq!(i.file_fingerprint(&mut c).await.unwrap(), Some(f1));
    drop(c);
    l.sql("PRAGMA application_id = 1213486160").await;
    let mut c = l.reader().await;
    assert_ne!(
        i.file_fingerprint(&mut c).await.unwrap(),
        Some(f1),
        "another format"
    );
    drop(c);
    let other = lite("CREATE TABLE t(a)").await;
    let mut c = other.reader().await;
    assert_ne!(
        i.file_fingerprint(&mut c).await.unwrap(),
        Some(f1),
        "another file"
    );
    let mut off = MetaConn::offline(DbmsKind::Sqlite);
    assert!(i.file_fingerprint(&mut off).await.is_err(), "fails closed");
}

// ## m4-t18 (resume after T15: R-W3-2, R-W3-3, the progress sink, R-W3-9)

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_is_the_file_fingerprint_and_unknown_in_memory() {
    let l = lite("CREATE TABLE t(a)").await;
    let i = intro();
    let mut c = l.reader().await;
    let f = i.file_fingerprint(&mut c).await.unwrap().expect("a file");
    assert_eq!(
        i.identity(&mut c).await.unwrap(),
        ServerIdentity::Known(hacivat_meta::persist::ServerFingerprint(f))
    );
    let mut off = MetaConn::offline(DbmsKind::Sqlite);
    assert!(
        i.identity(&mut off).await.is_err(),
        "no read: an error, never a fact"
    );
    // In memory: no file, so no identity.
    let profile = Profile {
        id: pid(),
        name: "mem".into(),
        engine: DbmsKind::Sqlite,
        endpoint: ProfileEndpoint::File {
            path: PathBuf::from(":memory:"),
            create_if_missing: false,
        },
        user: None,
        auth: AuthProviderId::None,
        retention: Retention::Never,
        ..Profile::default()
    };
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile.clone()),
        Arc::new(deps()),
        Arc::new(budget_for(&profile)),
    );
    let _user = open(&sm, SessionKind::Console(ClientId::DEFAULT)).await;
    let mut c = sm
        .meta_conn(
            MetaConnSpec {
                profile: profile.id,
                database: None,
                statement_timeout: Duration::from_secs(60),
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(i.identity(&mut c).await.unwrap(), ServerIdentity::Unknown);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_scope_filter_is_applied_on_the_client() {
    let l = lite(
        "CREATE TABLE keep_a(x); CREATE TABLE drop_b(y); CREATE INDEX drop_b_y ON drop_b(y); \
         CREATE VIEW v AS SELECT x FROM keep_a",
    )
    .await;
    let f = parse_filter("table:keep*", DialectId::Sqlite, FilterRole::Introspect).unwrap();
    let mut c = l.reader().await;
    let r = intro()
        .introspect(
            &mut c,
            &main_ns(),
            Mode::Full,
            Level::Structure,
            &f,
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let tables = match r.existent.get(&ObjectKind::Table) {
        Some(ExistSet::Ok(ids)) => ids.iter().map(|i| i.path.clone()).collect::<Vec<_>>(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        tables,
        vec![vec![Name::from("keep_a")]],
        "drop_b is never listed"
    );
    let ns = merged(&main_ns(), None, r.clone());
    assert!(ns.lookup(KindGroup::Relations, "drop_b").is_empty());
    assert_eq!(ns.lookup(KindGroup::Relations, "keep_a").len(), 1);
    assert_eq!(
        ns.lookup(KindGroup::Relations, "v").len(),
        1,
        "no item applies to views"
    );
    for (kind, set) in &r.existent {
        if let ExistSet::Ok(ids) | ExistSet::Scoped { ids, .. } = set {
            assert!(
                ids.iter().all(|i| &*i.path[0] != "drop_b"),
                "{kind:?}: nothing of drop_b"
            );
        }
    }
    // A fragment of a filtered-out table reads nothing of it either.
    let key = ObjectKey {
        kind: ObjectKind::Table,
        path: [Name::from("main"), Name::from("drop_b")]
            .into_iter()
            .collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let r = intro()
        .fragment(
            &mut c,
            &main_ns(),
            &[key],
            Level::Structure,
            &f,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(r.changed.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_reports_its_counts_to_the_progress_sink() {
    let l = lite(
        "CREATE TABLE a(x); CREATE TABLE b(y REFERENCES a(x)); CREATE INDEX bi ON b(y); \
         CREATE VIEW v AS SELECT x FROM a",
    )
    .await;
    type Seen = Arc<Mutex<Vec<(NsKey, hacivat_introspect::Phase, u64, Option<u64>)>>>;
    let seen: Seen = Arc::default();
    let sink_seen = seen.clone();
    let i = SqliteIntrospector::default().with_progress(Arc::new(move |ns, phase, done, total| {
        sink_seen
            .lock()
            .unwrap()
            .push((ns.clone(), phase, done, total));
    }));
    let mut c = l.reader().await;
    pass(&i, &mut c, &main_ns(), Mode::Full, Level::Structure, None).await;
    let seen = seen.lock().unwrap().clone();
    use hacivat_introspect::Phase as P;
    let last = |p: P| {
        seen.iter()
            .rev()
            .find(|s| s.1 == p)
            .map(|s| (s.2, s.3))
            .unwrap_or_else(|| panic!("{p:?} reported: {seen:?}"))
    };
    assert!(seen.iter().all(|s| s.0 == main_ns()));
    assert_eq!(last(P::Tables), (3, Some(3)), "a, b and v");
    assert_eq!(last(P::Columns), (3, Some(3)));
    assert_eq!(last(P::Indexes), (1, Some(1)));
    assert_eq!(last(P::Constraints), (1, Some(1)));
    // Counts never go backwards within a phase.
    for p in [P::Tables, P::Columns, P::Indexes, P::Constraints] {
        let done: Vec<u64> = seen.iter().filter(|s| s.1 == p).map(|s| s.2).collect();
        assert!(done.windows(2).all(|w| w[0] <= w[1]), "{p:?}: {done:?}");
    }
}

#[test]
fn every_rendered_sqlite_section_passes_the_metadata_guard() {
    // R-W3-9: every section, in every copy (Main, Temp, Attached), rendered as the
    // introspector renders it, passes T5's guard for the reader (`check_own_text`) and for the
    // user's or a joined session (`check_read_text_for`), so nothing reaches a connection
    // that the guard would refuse at run time.
    use hacivat_introspect::pack::{BindValue, LogicalType, PackCtx, PackFlags, Params};
    use hacivat_session::meta::{check_own_text, check_read_text_for};
    let pack = Pack::builtin(DialectId::Sqlite).unwrap();
    let text = |s: &str| BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    };
    let mut copies = std::collections::BTreeMap::<&str, usize>::new();
    for name in pack.sections() {
        for suffix in ["Main", "Temp", "Attached"] {
            if name.ends_with(suffix) {
                *copies.entry(suffix).or_default() += 1;
            }
        }
        for flags in [PackFlags::empty(), PackFlags::JOINED] {
            for level in [Level::Names, Level::Structure, Level::Sources] {
                let ctx = PackCtx {
                    dialect: DialectId::Sqlite,
                    version: 0,
                    flavor: DbmsKind::Sqlite,
                    mode: Mode::Full,
                    level,
                    flags,
                    params: Params {
                        scalars: vec![
                            (Name::from("schema"), text("main")),
                            (Name::from("rel"), text("t")),
                        ],
                        lists: Vec::new(),
                    },
                };
                let qs = pack
                    .render(name, &ctx)
                    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
                assert_eq!(qs.len(), 1, "{name}");
                let sql = qs[0].sql();
                assert_eq!(
                    check_own_text(sql, DbmsKind::Sqlite),
                    Ok(()),
                    "{name}: {sql}"
                );
                assert_eq!(
                    check_read_text_for(sql, DbmsKind::Sqlite),
                    Ok(()),
                    "{name}: {sql}"
                );
                assert_eq!(
                    check_read_text_for(sql, DbmsKind::Unknown),
                    Ok(()),
                    "{name} (every engine's lexer): {sql}"
                );
            }
        }
    }
    // Six sections read `sqlite_schema`, each in three copies (`SchemaVersionMain` and
    // `ApplicationIdMain` are `main`-only and have none).
    assert_eq!(
        copies.into_iter().collect::<Vec<_>>(),
        vec![("Attached", 6), ("Main", 8), ("Temp", 6)]
    );
}

// ## m4-t28-oracle
// Gate G4 (M4 Task 28 step 2, ver/01 #sqlite-schema-version): Task 18's
// `schema_version_sequence_matches_ver01` and
// `same_schema_version_replaced_file_detected_on_reconnect_by_digest` re-run with the G4
// oracle after every step: the incremental namespace equals a fresh full pass compared by
// `g4_canon` (the namespace header and every object, the root included; only `loaded_at`,
// the generation and the watermark left out), and a step that leaves `main` unchanged
// re-reads nothing. Each cell that holds prints `HV-FACT g4 sqlite <case> incremental==full`.

/// The G4 comparison. It must stay in step with `common::g4::canon` of the Docker matrix
/// (this file does not use `mod common`): the same header and the same object fields.
///
/// Known limit (T28 review m-3): a SQLite incremental pass on a change is a full read, so the
/// merge reuses its arena (`Dry::Reuse`). These cells therefore measure change *detection*,
/// not the merge walk; merge coverage comes from the PostgreSQL and MySQL cells.
fn g4_canon(ns: &NamespaceSnapshot) -> Vec<String> {
    let mut v: Vec<String> = (0..ns.len() as u32)
        .map(|i| {
            let o = ns.obj(i).unwrap();
            format!(
                "{:?} | {:?} | {:?} | {:?}",
                ns.object_key(i).unwrap(),
                o.state(),
                o.flags(),
                o.props()
            )
        })
        .collect();
    v.sort();
    v.insert(
        0,
        format!(
            "namespace {:?} {:?} {:?} {:?} objects={}",
            ns.key(),
            ns.dbms(),
            ns.fold(),
            ns.meta().level,
            ns.len()
        ),
    );
    v
}

/// `HV-FACT g4 sqlite <case> incremental==full`, straight to stdout (libtest captures only
/// the `print!` family).
fn g4_cell(case: &str) {
    use std::io::Write as _;
    let line = format!("HV-FACT g4 sqlite {case} incremental==full\n");
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g4_schema_version_sequence_incremental_equals_full_each_step() {
    // (case, statements, main's schema_version after it, whether main changed)
    let steps: [(&str, &str, u32, bool); 8] = [
        ("insert", "INSERT INTO t VALUES (1)", 1, false),
        ("create-index", "CREATE INDEX i ON t(id)", 2, true),
        ("add-column", "ALTER TABLE t ADD COLUMN c", 3, true),
        ("create-temp-table", "CREATE TEMP TABLE tt(a)", 3, false),
        ("create-view", "CREATE VIEW v AS SELECT id FROM t", 4, true),
        ("drop-view", "DROP VIEW v", 5, true),
        (
            "rolled-back-create",
            "BEGIN; CREATE TABLE z(a); ROLLBACK",
            5,
            false,
        ),
        ("vacuum", "VACUUM", 6, true),
    ];
    for level in [Level::Structure, Level::Sources] {
        let l = lite("CREATE TABLE t(id)").await;
        let mut cur = full(&l, level).await;
        assert_eq!(sqlite_wm(&cur).0, 1, "{level:?}: baseline");
        for (case, sql, want, changed) in steps {
            let label = format!("{level:?} {case}");
            for stmt in sql.split("; ") {
                l.sql(stmt).await;
            }
            let mut c = l.reader().await;
            let r = pass(
                &intro(),
                &mut c,
                &main_ns(),
                Mode::Incremental,
                level,
                Some(&cur),
            )
            .await;
            drop(c);
            if changed {
                assert!(!r.changed.is_empty(), "{label}: nothing re-read");
            } else {
                assert!(
                    !r.full && r.changed.is_empty() && r.existent.is_empty(),
                    "{label}: re-read although main is unchanged: {r:?}"
                );
            }
            cur = merged(&main_ns(), Some(&cur), r);
            assert_eq!(sqlite_wm(&cur).0, want, "{label}: schema_version");
            let fresh = full(&l, level).await;
            let (got, wanted) = (g4_canon(&cur), g4_canon(&fresh));
            let only_inc: Vec<_> = got.iter().filter(|x| !wanted.contains(x)).collect();
            let only_full: Vec<_> = wanted.iter().filter(|x| !got.contains(x)).collect();
            assert!(
                only_inc.is_empty() && only_full.is_empty(),
                "{label}: incremental differs from full\n only incremental: {only_inc:#?}\n only full: {only_full:#?}"
            );
            g4_cell(&format!(
                "{}-{case}",
                format!("{level:?}").to_ascii_lowercase()
            ));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g4_replaced_file_with_the_same_schema_version_equals_full_after_reconnect() {
    let l = lite("CREATE TABLE a1(x)").await;
    let other = file_with(l._dir.path(), "other.db", "CREATE TABLE b1(y)").await;
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Structure,
        None,
    )
    .await;
    let s1 = merged(&main_ns(), None, r);
    assert_eq!(sqlite_wm(&s1).0, 1);
    // Replaced in place by a file with the same schema_version. The reused reader's fast
    // path does not look (Task 18: detection is on reconnect, by digest); the cell is the
    // reconnect.
    copy_in_place(&other, &l.path);
    drop(c);
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Incremental,
        Level::Structure,
        Some(&s1),
    )
    .await;
    drop(c);
    assert!(reread(&r), "the digest found the change: {r:?}");
    let s2 = merged(&main_ns(), Some(&s1), r);
    assert_eq!(sqlite_wm(&s2).0, 1, "the same schema_version");
    assert_eq!(g4_canon(&s2), g4_canon(&full(&l, Level::Structure).await));
    assert!(s2.lookup(KindGroup::Relations, "a1").is_empty());
    g4_cell("replaced-file-same-schema-version-reconnect");
}
// ## m4-t28-oracle
