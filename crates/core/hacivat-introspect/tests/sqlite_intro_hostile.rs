// ## m4-t18-sqlite (fix round 1): the opus review's hostile tests (`task-m4-18-review-hostile.rs`),
// committed as written except where a test asserted a finding: H1 (I-1), H2 (I-2), H3 (I-3,
// ruling R-W3-10) and H7 (m1) now assert the fixes, and the calls take R-W3-2's filter.
#![allow(dead_code, unused_imports)]
#![allow(
    clippy::result_large_err,
    reason = "the session's `Reply` and the seam's `IntroError` are other crates' types"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use hacivat_driver::{
    Driver, DriverConnect, DriverRegistry, Endpoint, EngineCatalog, InsecureAuth,
};
use hacivat_introspect::engine::MetaSource;
use hacivat_introspect::filter::{CompiledFilter, FilterRole, parse_filter};
use hacivat_introspect::introspector::{FacetRows, IntroError, Introspector, Staleness};
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

/// The filter that keeps everything.
fn all() -> &'static CompiledFilter {
    static ALL: std::sync::LazyLock<CompiledFilter> = std::sync::LazyLock::new(|| {
        parse_filter("", DialectId::Sqlite, FilterRole::Introspect).expect("the empty filter")
    });
    &ALL
}

/// The engine's connection source over the profile's session manager (T15's `MetaSource`),
/// counting what it opens and what comes back.
struct Readers {
    sm: Arc<SessionManager>,
    profile: ProfileId,
    opened: std::sync::atomic::AtomicUsize,
    released: std::sync::atomic::AtomicUsize,
}

impl MetaSource for Readers {
    fn acquire<'a>(
        &'a self,
        database: Option<Name>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<MetaConn, MetaError>> {
        Box::pin(async move {
            let c = self
                .sm
                .meta_conn(
                    MetaConnSpec {
                        profile: self.profile,
                        database: database.map(|d| d.to_string()),
                        statement_timeout: Duration::from_secs(60),
                    },
                    cancel,
                )
                .await?;
            self.opened
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(c)
        })
    }

    fn released(&self) {
        self.released
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Lite {
    fn readers(&self) -> Arc<Readers> {
        Arc::new(Readers {
            sm: self.sm.clone(),
            profile: self.profile.id,
            opened: Default::default(),
            released: Default::default(),
        })
    }
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

// =============================================================================================
// T18 opus review: hostile tests the implementer did not write.

/// H1 (missed change). A temp view over a `main` table: its columns change when `main.t`
/// changes, but `temp.sqlite_schema` does not, so the temp digest is unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h1_temp_view_over_main_sees_main_alter() {
    let l = lite("CREATE TABLE t(a)").await;
    l.sql("CREATE TEMP VIEW tv AS SELECT * FROM main.t").await;
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
    assert_eq!(children(&temp1, &["tv"], ObjectKind::Column), vec!["a"]);
    drop(tc);
    l.sql("ALTER TABLE main.t ADD COLUMN b").await;
    // The engine's own path: a fresh connection, an incremental pass with the cached snapshot.
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
    let temp2 = merged(&temp_ns(), Some(&temp1), r.clone());
    assert_eq!(
        children(&temp2, &["tv"], ObjectKind::Column),
        vec!["a", "b"],
        "the temp view's columns after ALTER of main.t (missed change): full={} changed={}",
        r.full,
        r.changed.len()
    );
}

/// H2 (temp is per connection). `Work::CheckState` carries no namespace, so T15's engine runs
/// it on the reader (`database_of(None)`). The user's temp table must not come back `Dropped`
/// from a connection that cannot see it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_check_state_of_a_temp_object_on_the_reader_is_not_dropped() {
    let l = lite("CREATE TABLE t(a)").await;
    l.sql("CREATE TEMP TABLE tt(x)").await;
    let key = ObjectKey {
        kind: ObjectKind::Table,
        path: vec![Name::from("temp"), Name::from("tt")].into(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let mut c = l.reader().await;
    let st = intro()
        .check_state(&mut c, std::slice::from_ref(&key))
        .await
        .unwrap();
    assert_eq!(
        st,
        vec![Staleness::Unknown],
        "never Dropped from a connection that cannot see it"
    );
    let stamps = intro().stamps(&mut c, std::slice::from_ref(&key)).await;
    assert!(
        matches!(stamps, Err(IntroError::Meta(MetaError::Unavailable))),
        "never None (gone): {stamps:?}"
    );
    let snap = merged(&temp_ns(), None, NamespaceResult::empty_full());
    let check = intro().check_namespace(&mut c, &temp_ns(), &snap).await;
    assert!(
        matches!(check, Err(IntroError::Meta(MetaError::Unavailable))),
        "{check:?}"
    );
    // On the user's own connection the same keys are not decided either.
    drop(c);
    let mut tc = l.temp_conn().await;
    let st = intro()
        .check_state(&mut tc, std::slice::from_ref(&key))
        .await
        .unwrap();
    assert_eq!(st, vec![Staleness::Unknown]);
}

/// H3. Two attached schemas in one `stamps`/`check_state` call on one reader: the second
/// finds the fixed alias holding the first file.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_stamps_across_two_attached_schemas_on_one_reader() {
    let l = lite("CREATE TABLE t(a)").await;
    let a = file_with(l._dir.path(), "a.db", "CREATE TABLE ta(x)").await;
    let b = file_with(l._dir.path(), "b.db", "CREATE TABLE tb(y)").await;
    let readers = l.readers();
    let i = SqliteIntrospector::default().with_readers(readers.clone());
    let mut c = l.reader().await;
    c.attach_read_only("sa", &a).await.unwrap();
    c.attach_read_only("sb", &b).await.unwrap();
    let k = |s: &str, n: &str| ObjectKey {
        kind: ObjectKind::Table,
        path: vec![Name::from(s), Name::from(n)].into(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let got = i.stamps(&mut c, &[k("sa", "ta"), k("sb", "tb")]).await;
    assert!(
        matches!(&got, Ok(v) if v.iter().all(Option::is_some)),
        "both attached tables exist: {got:?}"
    );
    let st = i
        .check_state(&mut c, &[k("sa", "ta"), k("sb", "gone")])
        .await
        .unwrap();
    assert_eq!(st, vec![Staleness::Unknown, Staleness::Dropped]);
    // Each attached schema had a reader of its own, and none stays open.
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(readers.opened.load(SeqCst), 4);
    assert_eq!(readers.released.load(SeqCst), 4, "no idle reader remains");
    // The connection handed in never took the alias.
    let keys: Vec<NsKey> = i
        .list_namespaces(&mut c)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.key)
        .collect();
    assert_eq!(
        keys,
        vec![
            main_ns(),
            temp_ns(),
            NsKey::schema(["sa"]),
            NsKey::schema(["sb"])
        ]
    );
}

/// H3b (R-W3-10). A user schema that is itself named `hacivat_attached` is a namespace like any
/// other, and does not block the other attached schemas.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3b_a_user_schema_named_like_the_alias() {
    let l = lite("CREATE TABLE t(a)").await;
    let own = file_with(l._dir.path(), "own.db", "CREATE TABLE mine(x)").await;
    let b = file_with(l._dir.path(), "b.db", "CREATE TABLE tb(y)").await;
    let readers = l.readers();
    let i = SqliteIntrospector::default().with_readers(readers.clone());
    let mut c = l.reader().await;
    c.attach_read_only(ATTACH_ALIAS, &own).await.unwrap();
    c.attach_read_only("sb", &b).await.unwrap();
    let keys: Vec<NsKey> = i
        .list_namespaces(&mut c)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.key)
        .collect();
    assert!(keys.contains(&NsKey::schema([ATTACH_ALIAS])), "{keys:?}");
    for (schema, table) in [(ATTACH_ALIAS, "mine"), ("sb", "tb")] {
        let ns = NsKey::schema([schema]);
        let r = pass(&i, &mut c, &ns, Mode::Full, Level::Structure, None).await;
        let s = merged(&ns, None, r);
        assert_eq!(s.lookup(KindGroup::Relations, table).len(), 1, "{schema}");
        assert_eq!(s.key(), &ns, "results keep the user's schema name");
    }
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(readers.opened.load(SeqCst), readers.released.load(SeqCst));
}

/// H4 (R-W3-9). Every SQLite section, rendered with binds, passes T5's guards: the own
/// connection's (`check_own_text`), the SQLite read guard and, as every section is declared
/// `joinable`, the joined guard of every lexer (`check_read_text`).
#[test]
fn h4_every_rendered_sqlite_section_passes_the_t5_guards() {
    use hacivat_introspect::pack::{BindValue, LogicalType, PackCtx, PackFlags, Params};
    use hacivat_session::meta::{check_own_text, check_read_text, check_read_text_for};
    let pack = Pack::builtin(DialectId::Sqlite).unwrap();
    let text = |s: &str| BindValue::Typed {
        hint: LogicalType::Text {
            max_chars: None,
            fixed: false,
        },
        text: Arc::from(s),
    };
    let mut n = 0;
    for flags in [PackFlags::empty(), PackFlags::JOINED] {
        for name in pack.sections() {
            let ctx = PackCtx {
                dialect: DialectId::Sqlite,
                version: 0,
                flavor: DbmsKind::Sqlite,
                mode: Mode::Full,
                level: Level::Sources,
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
            assert!(!qs.is_empty(), "{name}");
            for q in qs {
                let sql = q.sql();
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
                assert_eq!(check_read_text(sql), Ok(()), "{name} (joined): {sql}");
                n += 1;
            }
        }
    }
    assert!(n >= 2 * 25, "{n}");
}

/// H5 (ATTACH path). A crafted path cannot widen the reader's access: a `file:` URI asking
/// `mode=rw`/`rwc` is refused by the engine, and a real file whose name contains `?mode=rw&`
/// is opened as that literal file, read-only, and left byte-identical by a full pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_attach_path_cannot_escape_read_only() {
    let l = lite("CREATE TABLE t(a)").await;
    let plain = file_with(l._dir.path(), "plain.db", "CREATE TABLE p(q)").await;
    let mut c = l.reader().await;
    for mode in ["rw", "rwc"] {
        let uri = PathBuf::from(format!("file:{}?mode={mode}", plain.display()));
        let got = c.attach_read_only("u", &uri).await;
        assert!(got.is_err(), "mode={mode} must be refused: {got:?}");
    }
    // `?` and `&` are not valid in a Windows file name, so the literal-name half runs elsewhere.
    if cfg!(windows) {
        return;
    }
    let odd = file_with(
        l._dir.path(),
        "x.db?mode=rw&cache=shared",
        "CREATE TABLE ox(q)",
    )
    .await;
    let before = std::fs::read(&odd).unwrap();
    c.attach_read_only("odd", &odd).await.unwrap();
    let ns = NsKey::schema(["odd"]);
    for level in [Level::Names, Level::Structure, Level::Sources] {
        let r = pass(&intro(), &mut c, &ns, Mode::Full, level, None).await;
        let s = merged(&ns, None, r);
        assert_eq!(s.lookup(KindGroup::Relations, "ox").len(), 1);
    }
    drop(c);
    assert_eq!(std::fs::read(&odd).unwrap(), before, "byte-identical");
    assert!(
        !l._dir.path().join("x.db").exists(),
        "no file created by URI parsing"
    );
}

/// H6 (untrusted file). A crafted file: an exponential view chain (SQLite caps it at 65535
/// references), a view over a dropped table, a view calling a non-innocuous function, a
/// trigger with a hostile body, CHECK/generated text with parentheses in strings, and a
/// virtual table whose module text is patched to an unknown one. A Structure pass ends, keeps
/// the bad relations BROKEN with no columns, keeps the good ones whole, and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h6_hostile_schema_file_is_read_without_executing_it() {
    let mut ddl = String::from(
        "CREATE TABLE base(id INTEGER PRIMARY KEY, \"we)ird\" TEXT COLLATE NOCASE CHECK (\"we)ird\" <> ')'), \
         g AS (id || ')(' ) STORED); \
         CREATE TABLE gone(x); CREATE VIEW dangling AS SELECT x FROM gone; DROP TABLE gone; \
         CREATE VIEW canary AS SELECT json('not json') AS j; \
         CREATE TRIGGER trg AFTER INSERT ON base WHEN new.id > 0 BEGIN DELETE FROM base; END; \
         CREATE VIRTUAL TABLE vt USING fts5(a, b); \
         CREATE VIEW v0 AS SELECT id AS x FROM base;",
    );
    for i in 1..=20 {
        ddl.push_str(&format!(
            "CREATE VIEW v{i} AS SELECT a.x FROM v{p} a, v{p} b;",
            p = i - 1
        ));
    }
    let dir = tempfile::tempdir().unwrap();
    let path = file_with(dir.path(), "hostile.db", &ddl).await;
    let bytes = std::fs::read(&path).unwrap();
    let (from, to) = (b"USING fts5(a, b)", b"USING zzz9(a, b)");
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
    let mut patched = bytes.clone();
    patched[at..at + to.len()].copy_from_slice(to);
    std::fs::write(&path, &patched).unwrap();
    let l = {
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
            path: path.clone(),
            profile,
            sm,
            user,
        }
    };
    let start = std::time::Instant::now();
    let mut c = l.reader().await;
    let r = pass(
        &intro(),
        &mut c,
        &main_ns(),
        Mode::Full,
        Level::Sources,
        None,
    )
    .await;
    drop(c);
    let took = start.elapsed();
    let ns = merged(&main_ns(), None, r);
    let broken = |n: &str, k| obj(&ns, &[n], k).flags().contains(ObjFlags::BROKEN);
    assert!(broken("dangling", ObjectKind::View));
    assert!(broken("vt", ObjectKind::VirtualTable));
    assert!(
        broken("v20", ObjectKind::View),
        "SQLite's reference cap refuses v16+"
    );
    assert!(!broken("v1", ObjectKind::View));
    assert!(!broken("canary", ObjectKind::View), "prepared, never run");
    assert_eq!(children(&ns, &["canary"], ObjectKind::Column), vec!["j"]);
    assert_eq!(
        children(&ns, &["base"], ObjectKind::Column),
        vec!["id", "we)ird", "g"]
    );
    assert_eq!(
        column(&ns, "base", "we)ird").collation().map(|n| &**n),
        Some("NOCASE")
    );
    assert_eq!(
        &*column(&ns, "base", "g").generated().unwrap().expr,
        "id || ')('"
    );
    assert_eq!(children(&ns, &["base"], ObjectKind::Trigger), vec!["trg"]);
    assert_eq!(std::fs::read(&path).unwrap(), patched, "byte-identical");
    eprintln!("h6 hostile Structure pass took {took:?}");
}

/// H7. SQLite applies a `DEFERRABLE` clause standing alone in a later column to the most
/// recent foreign key (`sqlite3DeferForeignKey`). Verified with the CLI: the INSERT of a
/// dangling `a` succeeds inside a transaction, so the FK on `a` is deferred.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h7_stray_deferrable_applies_to_the_previous_fk() {
    let l = lite(
        "CREATE TABLE p(id INTEGER PRIMARY KEY); \
         CREATE TABLE c(a REFERENCES p(id), b DEFERRABLE INITIALLY DEFERRED)",
    )
    .await;
    let ns = full(&l, Level::Structure).await;
    let fks = children(&ns, &["c"], ObjectKind::ForeignKey);
    let Props::ForeignKey(f) = obj(&ns, &["c", &fks[0]], ObjectKind::ForeignKey).props() else {
        panic!()
    };
    assert_eq!(
        (f.deferrable, f.initially_deferred),
        (true, true),
        "{fks:?}"
    );
}
