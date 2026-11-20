// ## m4-t17b-mysql-facets
//! MySQL and MariaDB facets on real servers (M4 Task 17b, R-M4-30; rulings R-W4-3, R-W4-4).
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-introspect
//! --test docker_mysql_facets` after `cargo test -p hacivat-introspect --no-run`. Every test
//! is `#[ignore = "docker"]` and returns early unless `HACIVAT_DOCKER=1`; targets come only
//! from `require_target` (`mysql84` in the PR set; `mysql80` and `mariadb114` in the full
//! set). Each test works in its own database `hvm4_t17b_<slug>` (`hv_test` has `ALL` on
//! `hvm4\_%`, R-M4-43) and its own `hvm4_t17b_*` account (created and granted through
//! `hv_fixture`'s definer procedures), each dropped by a guard, also on panic. Grants are read
//! as three logins: the account itself and `hv_test` (no `SELECT` on the grant tables: the
//! read is refused) and `hv_catalog` (`SELECT` on `mysql.*`: the read is complete). Server
//! facts are printed as `HV-FACT` lines on stdout (ruling R-W3-14).
//!
//! **Harness.** Task 17's (`docker_mysql_intro.rs`), copied: each metadata connection is Task
//! 5's `meta_conn` on its own connected `ConnectionManager`; fixture SQL runs on a writable
//! `hv_test` (or `hv_fixture`) connection, never a metadata one.

#![cfg(feature = "session")]

use std::sync::Arc;

use hacivat_introspect::mysql::facets::{self, Coverage, name as sections};
use hacivat_introspect::mysql::pass::{SectionReader, read_server};
use hacivat_introspect::mysql::{MySqlIntrospector, TxnReader};
use hacivat_introspect::pack::{Mode, Pack, Params};
use hacivat_introspect::{FacetRows, IntroError, Introspector};
use hacivat_meta::merge::{ExistSet, MergeOutcome, MergePolicy, Trigger, base, merge};
use hacivat_meta::name::{DialectId, FoldRule, IdentCtx, NamePart};
use hacivat_meta::props::{DepsKind, FacetKind, Grant};
use hacivat_meta::snapshot::{Catalog, CatalogChange, ContainerKey, LoadState, ServerFacts};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_meta::{KindGroup, Level, Name, NsKey, ObjectKey, ObjectKind, Ref};
use hacivat_proto::DbmsKind;
use hacivat_session::meta::MetaConn;
use hacivat_testkit::{
    CATALOG_PROBE_DATABASE, DockerTarget, docker_enabled, fixture_sql, require_target,
};
use smallvec::smallvec;
use tokio_util::sync::CancellationToken;

const MYSQL: [&str; 2] = ["mysql84", "mysql80"];
const ALL: [&str; 3] = ["mysql84", "mysql80", "mariadb114"];
const MARIADB: [&str; 1] = ["mariadb114"];

mod harness {
    use std::ops::{Deref, DerefMut};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use hacivat_driver::{EngineCatalog, ServerMessage};
    use hacivat_net::TlsMode;
    use hacivat_proto::Department;
    use hacivat_secrets::{
        AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
        SecretString,
    };
    use hacivat_session::meta::MetaConnSpec;
    use hacivat_session::pipeline::{
        ApprovedRoots, ConnectDeps, ConnectRequest, Interaction, LiveConnection, Progress, Purpose,
        connect,
    };
    use hacivat_session::profile::{Profile, ProfileEndpoint};
    use hacivat_session::{ConnectionManager, LiveState, ManagerConfig, default_drivers};
    use hacivat_testkit::TlsDefault;
    use tokio::sync::watch;

    use super::*;

    /// Which account a statement runs as.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Login {
        /// `hv_test`: the fixtures' DDL in `hvm4_*` databases.
        Test,
        /// `hv_fixture`: only the `hv_fixture_admin` definer procedures (`SET GLOBAL`, users).
        Fixture,
    }

    /// The real drivers, a memory secret store, no credential files, no SSH (the
    /// `hacivat-exec` and T16 harness).
    fn deps() -> ConnectDeps {
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

    /// The target `t` signed in as `login` (`hv_fixture` on `information_schema`).
    fn as_login(t: &DockerTarget, login: Login) -> DockerTarget {
        match login {
            Login::Test => t.clone(),
            Login::Fixture => t.as_fixture().expect("a MySQL-family target"),
        }
    }

    /// The profile of `t` on database `db` (`None`: the login's own), a password kept nowhere.
    fn profile_for(t: &DockerTarget, db: Option<&str>) -> Profile {
        let mut p = Profile {
            name: format!("t17b {}", t.id),
            engine: t.engine,
            endpoint: ProfileEndpoint::Server {
                host: t.host.clone(),
                port: Some(t.port),
            },
            database: Some(db.unwrap_or(&t.database).to_string()),
            user: Some(t.user.clone()),
            auth: AuthProviderId::Password,
            retention: Retention::Never,
            ..Profile::default()
        };
        p.tls.mode = match t.tls_default {
            TlsDefault::SelfSigned => TlsMode::Require,
            TlsDefault::None => TlsMode::Auto,
        };
        p.allow_unsupported_server = true;
        p
    }

    fn password(t: &DockerTarget) -> SecretString {
        SecretString::from(t.password.clone())
    }

    /// A metadata connection with the manager it lives on (dropping the manager would
    /// disconnect the profile and make the connection `Unavailable`).
    pub struct Meta {
        conn: MetaConn,
        _m: Arc<ConnectionManager>,
    }

    impl Deref for Meta {
        type Target = MetaConn;
        fn deref(&self) -> &MetaConn {
            &self.conn
        }
    }

    impl DerefMut for Meta {
        fn deref_mut(&mut self) -> &mut MetaConn {
            &mut self.conn
        }
    }

    /// A read-only metadata connection of `t` (`hv_test`) on database `db` (Task 5's
    /// `meta_conn`, on its own connected manager). The connect waits on the profile's state,
    /// 60 s at most.
    pub async fn meta(t: &DockerTarget, db: &str) -> Meta {
        let m = ConnectionManager::new(
            tokio::runtime::Handle::current(),
            Arc::new(deps()),
            ManagerConfig::m2b(),
        );
        let profile = Arc::new(profile_for(t, Some(db)));
        m.connect(profile.clone(), Some(password(t)), Interaction::Headless);
        let mut changes = m.changes();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            match m.state(profile.id) {
                LiveState::Connected(_) => break,
                LiveState::Failed(e) => panic!("{}: connect failed: {:?}", t.id, e.class),
                _ => {}
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{}: connect timed out",
                t.id
            );
            let _ = tokio::time::timeout(Duration::from_millis(200), changes.changed()).await;
        }
        let conn = m
            .sessions(profile.id)
            .expect("connected")
            .meta_conn(
                MetaConnSpec {
                    profile: profile.id,
                    database: None,
                    statement_timeout: Duration::from_secs(120),
                },
                &CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: a metadata connection: {e:?}", t.id));
        Meta { conn, _m: m }
    }

    /// A writable connection of `t` as `login` on `db` (never a metadata connection).
    async fn admin(t: &DockerTarget, login: Login, db: Option<&str>) -> LiveConnection {
        let t = as_login(t, login);
        let db = match login {
            Login::Test => db,
            // `hv_fixture` may sign in only to `information_schema` (testkit docs).
            Login::Fixture => None,
        };
        connect(
            &deps(),
            ConnectRequest {
                read_only: false,
                profile: Arc::new(profile_for(&t, db)),
                draft_password: Some(password(&t)),
                purpose: Purpose::Main,
                department: Department::Console,
                prior_rejections: 0,
                draft_ssh: Vec::new(),
                overlay_secrets: None,
                plaintext_consent: false,
            },
            Interaction::Headless,
            CancellationToken::new(),
            watch::Sender::new(Progress {
                trail: Vec::new(),
                current: None,
                started: Instant::now(),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("{} admin ({login:?}): {:?}", t.id, e.class))
    }

    /// MySQL's error number of a refusal (else the SQLSTATE, else the error kind).
    fn code_of(m: Option<&ServerMessage>, kind: impl std::fmt::Debug) -> String {
        match m {
            Some(ServerMessage {
                vendor_code: Some(c),
                ..
            }) => c.to_string(),
            Some(ServerMessage {
                sqlstate: Some(s), ..
            }) => s.clone(),
            _ => format!("{kind:?}"),
        }
    }

    /// Runs `sql` as `login` on `db` (`None`: the login's default); the server's error code
    /// (`1054`, `1419`, …) when it fails. The statement text is never printed (it may hold a
    /// fixture password).
    pub async fn exec(
        t: &DockerTarget,
        login: Login,
        db: Option<&str>,
        sql: &str,
    ) -> Result<(), String> {
        let mut live = admin(t, login, db).await;
        let out = live
            .raw()
            .batch(sql)
            .await
            .map_err(|e| code_of(e.server.as_ref(), &e.kind));
        drop(live);
        out
    }

    /// [`exec`] on a fresh runtime thread, for drop guards. A failure is reported on stderr
    /// with the target and the server's code only (the statement may hold a fixture value;
    /// review m3: a left-over global must be visible), never a panic inside a drop.
    pub fn exec_blocking(t: &DockerTarget, login: Login, sql: &str) {
        let (t, sql) = (t.clone(), sql.to_string());
        let outcome = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            let got = rt.block_on(exec(&t, login, None, &sql));
            (t.id, got)
        })
        .join();
        match outcome {
            Ok((_, Ok(()))) => {}
            Ok((id, Err(code))) => {
                eprintln!("{id}: a drop guard's {login:?} statement failed: {code}");
            }
            Err(_) => eprintln!("a drop guard's {login:?} statement panicked"),
        }
    }
}

use harness::Login;

// ---- fixtures ---------------------------------------------------------------------------------

fn n(s: &str) -> Name {
    Arc::from(s)
}

/// `hvm4_t17b_<slug>`, dropped when the guard goes (also on panic).
struct Db {
    t: DockerTarget,
    name: String,
}

impl Db {
    async fn create(t: &DockerTarget, slug: &str) -> Db {
        let name = format!("hvm4_t17b_{slug}");
        harness::exec(
            t,
            Login::Test,
            None,
            &format!("DROP DATABASE IF EXISTS `{name}`"),
        )
        .await
        .expect("drop leftover");
        harness::exec(t, Login::Test, None, &format!("CREATE DATABASE `{name}`"))
            .await
            .unwrap_or_else(|e| panic!("{}: create {name}: {e}", t.id));
        Db { t: t.clone(), name }
    }

    async fn run(&self, sql: &str) {
        harness::exec(&self.t, Login::Test, Some(&self.name), sql)
            .await
            .unwrap_or_else(|e| panic!("{}: `{sql}`: {e}", self.t.id));
    }

    fn ns(&self) -> NsKey {
        NsKey::schema([self.name.as_str()])
    }

    fn key(&self, kind: ObjectKind, path: &[&str]) -> ObjectKey {
        let mut p = smallvec![n(&self.name)];
        p.extend(path.iter().map(|s| n(s)));
        ObjectKey {
            kind,
            path: p,
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        }
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        harness::exec_blocking(
            &self.t,
            Login::Test,
            &format!("DROP DATABASE IF EXISTS `{}`", self.name),
        );
    }
}

/// An `hvm4_*` account (created through `hv_fixture`), dropped when the guard goes (also on
/// panic: the review probe "the fixture user left behind after a panic").
struct Account {
    t: DockerTarget,
    user: String,
}

impl Account {
    async fn create(t: &DockerTarget, user: &str) -> Account {
        let _ = harness::exec(t, Login::Fixture, None, &fixture_sql::drop_user(user)).await;
        harness::exec(
            t,
            Login::Fixture,
            None,
            &fixture_sql::create_user(user, &t.password),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: create_user: {e}", t.id));
        Account {
            t: t.clone(),
            user: user.to_string(),
        }
    }

    /// A table-level grant (a database-level one reads `_` as a wildcard, wider than the
    /// definer's `hvm4\_%` grant: refused, CI 44645719558).
    async fn grant(&self, privilege: &str, db: &Db, table: &str) {
        harness::exec(
            &self.t,
            Login::Fixture,
            None,
            &fixture_sql::grant(privilege, &db.name, Some(table), &self.user),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: grant {privilege} on {table}: {e}", self.t.id));
    }

    fn literal(&self) -> String {
        format!("'{}'@'%'", self.user)
    }

    fn target(&self) -> DockerTarget {
        self.t
            .as_test_account(&self.user)
            .expect("a MySQL-family target")
    }
}

impl Drop for Account {
    fn drop(&mut self) {
        harness::exec_blocking(&self.t, Login::Fixture, &fixture_sql::drop_user(&self.user));
    }
}

/// Prints one server fact as `HV-FACT <target> <key>=<value>` on stdout (ruling R-W3-14).
/// Values are server facts, never a password or a fixture statement.
fn fact(t: &DockerTarget, kv: &str) {
    use std::io::Write as _;
    let line = format!("HV-FACT {} {}\n", t.id, kv.replace('\n', " "));
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.flush();
}

fn intro(t: &DockerTarget) -> MySqlIntrospector {
    MySqlIntrospector { flavor: t.engine }
}

fn targets(ids: &[&str]) -> Vec<DockerTarget> {
    if !docker_enabled() {
        return Vec::new();
    }
    ids.iter().filter_map(|id| require_target(id)).collect()
}

/// A metadata connection of `hv_catalog` (`SELECT` on `mysql.*`, nothing on `hvm4_*`).
async fn catalog_meta(t: &DockerTarget) -> harness::Meta {
    harness::meta(
        &t.as_catalog().expect("a MySQL-family target"),
        CATALOG_PROBE_DATABASE,
    )
    .await
}

/// A metadata connection of the account `a`.
async fn account_meta(a: &Account) -> harness::Meta {
    let t = a.target();
    let db = t.database.clone();
    harness::meta(&t, &db).await
}

fn g(grantee: &str, privilege: &str) -> Grant {
    Grant {
        grantee: n(grantee),
        privilege: n(privilege),
        columns: Vec::new(),
        grantable: false,
    }
}

/// The server namespace `r` merged and published as the engine does.
fn server_view(r: hacivat_meta::merge::NamespaceResult) -> SnapshotView {
    let b = base(NsKey::server(), DbmsKind::MySql, FoldRule::Exact).expect("a base");
    let policy = MergePolicy {
        trigger: Trigger::User,
        ..MergePolicy::default()
    };
    let level = r.level;
    let next = match merge(Some(&b), r, &policy).expect("merge") {
        MergeOutcome::Applied { next, .. } => next,
        other => panic!("{other:?}"),
    };
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::MySql,
        IdentCtx::new(DialectId::MySql),
    )));
    catalog
        .publisher()
        .expect("a publisher")
        .publish(vec![
            CatalogChange::SetState(
                ContainerKey::Namespace(NsKey::server()),
                LoadState::Loaded {
                    level,
                    at: std::time::SystemTime::UNIX_EPOCH,
                    watermark: hacivat_meta::Watermark::None,
                    stale: false,
                },
            ),
            CatalogChange::Put(Arc::new(next)),
        ])
        .expect("publish");
    SnapshotView::new(catalog.current())
}

fn account_lookup(v: &SnapshotView, name: &str) -> &'static str {
    match v.lookup(
        &[NamePart {
            text: n(name),
            quoted: true,
        }],
        KindGroup::Accounts,
    ) {
        Lookup::Found(_) => "found",
        Lookup::Missing(_) => "missing",
        Lookup::Unknown(_) => "unknown",
    }
}

// ---- grants -----------------------------------------------------------------------------------

/// Plan Task 17b: an account with `SELECT` on one table reads the grants facet as a
/// permission error (the grant tables are refused to it), never as a partial list; so does
/// `hv_test`. `hv_catalog`, which may read the grant tables, gets the complete facet.
#[tokio::test]
#[ignore = "docker"]
async fn mysql_grants_partial_for_unprivileged_account() {
    for t in targets(&ALL) {
        let db = Db::create(&t, "grants").await;
        db.run("CREATE TABLE t (id INT, a INT)").await;
        db.run("CREATE TABLE u (id INT)").await;
        let acc = Account::create(&t, "hvm4_t17b_lim").await;
        acc.grant("SELECT", &db, "t").await;
        acc.grant("INSERT", &db, "t").await;
        let keys = [
            db.key(ObjectKind::Table, &["t"]),
            db.key(ObjectKind::Table, &["u"]),
            db.key(ObjectKind::Schema, &[]),
        ];
        let mut own = account_meta(&acc).await;
        let got = intro(&t)
            .facet(&mut own, &db.ns(), &keys, FacetKind::Grants)
            .await;
        fact(&t, &format!("grants_as_account={}", verdict(&got)));
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::Table
            }),
            "{}",
            t.id
        );
        let mut test = harness::meta(&t, &db.name).await;
        let got = intro(&t)
            .facet(&mut test, &db.ns(), &keys, FacetKind::Grants)
            .await;
        fact(&t, &format!("grants_as_hv_test={}", verdict(&got)));
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::Table
            }),
            "{}",
            t.id
        );
        let mut cat = catalog_meta(&t).await;
        let got = intro(&t)
            .facet(&mut cat, &db.ns(), &keys, FacetKind::Grants)
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        let FacetRows::Grants(got) = got else {
            panic!("{}: {got:?}", t.id)
        };
        fact(&t, &format!("grants_as_hv_catalog_answered={}", got.len()));
        let of = |name: &str| got.iter().find(|(k, _)| k.name() == name).map(|(_, f)| f);
        assert_eq!(
            of("t").map(|f| f.entries.clone()),
            Some(vec![
                g(&acc.literal(), "INSERT"),
                g(&acc.literal(), "SELECT")
            ]),
            "{}",
            t.id
        );
        assert_eq!(of("u").map(|f| f.entries.len()), Some(0), "{}", t.id);
        // The database-level grants: MySQL 8 only. `hv_test`'s `hvm4\_%` pattern grant is a
        // grant on a pattern, not on this database.
        let schema = of(&db.name);
        if t.engine == DbmsKind::MariaDb {
            assert!(schema.is_none(), "{}", t.id);
        } else {
            assert_eq!(schema.map(|f| f.entries.len()), Some(0), "{}", t.id);
        }
    }
}

fn verdict<T>(r: &Result<T, IntroError>) -> String {
    match r {
        Ok(_) => "ok".to_string(),
        Err(IntroError::Permission { kind }) => format!("permission_{kind:?}"),
        Err(e) => format!("{e:?}").replace(' ', "_"),
    }
}

// ---- dependencies -----------------------------------------------------------------------------

fn rref(kind: ObjectKind, s: &str, name: &str) -> Ref {
    Ref {
        kind,
        path: smallvec![n(s), n(name)],
        server_id: None,
    }
}

/// `VIEW_TABLE_USAGE` and `VIEW_ROUTINE_USAGE` (MySQL 8.0.13+): the rows as read, and the
/// facet a login without global `SELECT` and `EXECUTE` gets — a permission error, since a
/// used table it has no privilege on would be hidden.
#[tokio::test]
#[ignore = "docker"]
async fn mysql_view_table_usage_read() {
    for t in targets(&MYSQL) {
        let db = Db::create(&t, "usage").await;
        db.run("CREATE TABLE base_t (id INT)").await;
        // `log_bin_trust_function_creators = 1` on mysql80/84 (R-M4-43).
        db.run("CREATE FUNCTION f1() RETURNS INT DETERMINISTIC RETURN 1")
            .await;
        db.run("CREATE VIEW v AS SELECT id, f1() AS x FROM base_t")
            .await;
        db.run("CREATE VIEW v2 AS SELECT id FROM v").await;
        let v = db.key(ObjectKind::View, &["v"]);
        let v2 = db.key(ObjectKind::View, &["v2"]);
        let gone = db.key(ObjectKind::View, &["gone"]);
        let mut c = harness::meta(&t, &db.name).await;
        let (deps, cov) = {
            let mut txn = c.begin_snapshot().await.expect("snapshot");
            let mut r = TxnReader::new(&mut txn, None).expect("reader");
            let srv = read_server(&mut r, t.engine).await.expect("identity");
            let out = facets::dependencies(
                &mut r,
                &srv,
                &db.ns(),
                &[
                    v.clone(),
                    v2.clone(),
                    gone,
                    db.key(ObjectKind::Table, &["base_t"]),
                ],
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
            drop(r);
            txn.commit().await.expect("commit");
            out
        };
        fact(&t, &format!("deps_coverage_hv_test={cov:?}"));
        assert_eq!(deps.len(), 2, "{}: {deps:?}", t.id);
        assert_eq!(deps[0].key, v);
        assert_eq!(deps[0].facet.kind, DepsKind::Usage);
        assert!(!deps[0].unnamed, "{}", t.id);
        assert_eq!(
            deps[0].facet.depends_on,
            vec![
                rref(ObjectKind::Table, &db.name, "base_t"),
                rref(ObjectKind::Routine, &db.name, "f1"),
            ],
            "{}",
            t.id
        );
        assert_eq!(deps[1].key, v2);
        assert_eq!(
            deps[1].facet.depends_on,
            vec![rref(ObjectKind::View, &db.name, "v")],
            "{}",
            t.id
        );
        // `hv_test` holds database-level grants only: the read cannot be complete.
        assert!(matches!(cov, Coverage::Incomplete(_)), "{}", t.id);
        let got = intro(&t)
            .facet(&mut c, &db.ns(), &[v], FacetKind::Dependencies)
            .await;
        assert_eq!(
            got,
            Err(IntroError::Permission {
                kind: ObjectKind::View
            }),
            "{}",
            t.id
        );
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mariadb_dependencies_are_unsupported() {
    for t in targets(&MARIADB) {
        let db = Db::create(&t, "nodeps").await;
        db.run("CREATE TABLE base_t (id INT)").await;
        db.run("CREATE VIEW v AS SELECT id FROM base_t").await;
        assert!(
            !intro(&t)
                .capabilities()
                .facets
                .contains(&FacetKind::Dependencies),
            "{}",
            t.id
        );
        let mut c = harness::meta(&t, &db.name).await;
        let got = intro(&t)
            .facet(
                &mut c,
                &db.ns(),
                &[db.key(ObjectKind::View, &["v"])],
                FacetKind::Dependencies,
            )
            .await;
        assert_eq!(got, Err(IntroError::NotImplemented), "{}", t.id);
        // Even a profile that says MySQL: the flavor is the server's.
        let as_mysql = MySqlIntrospector {
            flavor: DbmsKind::MySql,
        };
        let got = as_mysql
            .facet(
                &mut c,
                &db.ns(),
                &[db.key(ObjectKind::View, &["v"])],
                FacetKind::Dependencies,
            )
            .await;
        assert_eq!(got, Err(IntroError::NotImplemented), "{}", t.id);
    }
}

// ---- roles ------------------------------------------------------------------------------------

/// The server namespace lists the applicable roles and the login, at `Names`: an account it
/// does not list (another login that exists) is `Unknown`, never `Missing`, and nothing is
/// ever an empty "no roles" answer.
#[tokio::test]
#[ignore = "docker"]
async fn mysql_roles_refused_is_unknown_not_empty() {
    for t in targets(&ALL) {
        let acc = Account::create(&t, "hvm4_t17b_roles").await;
        for (who, mut c, me, other) in [
            (
                "hv_test",
                harness::meta(&t, &t.database).await,
                "'hv_test'@'%'".to_string(),
                acc.literal(),
            ),
            (
                "account",
                account_meta(&acc).await,
                acc.literal(),
                "'hv_test'@'%'".to_string(),
            ),
        ] {
            let r = facets::server_namespace(&mut c)
                .await
                .unwrap_or_else(|e| panic!("{} {who}: {e:?}", t.id));
            assert_eq!(r.level, Level::Names, "{}", t.id);
            let roles = match &r.existent[&ObjectKind::Role] {
                ExistSet::Ok(ids) => ids.len().to_string(),
                other => format!("{other:?}"),
            };
            fact(&t, &format!("applicable_roles_{who}={roles}"));
            let v = server_view(r);
            assert_eq!(account_lookup(&v, &me), "found", "{} {who}", t.id);
            assert_eq!(account_lookup(&v, &other), "unknown", "{} {who}", t.id);
        }
        // Through the introspector's seam (`NsKind::Server`).
        let mut c = harness::meta(&t, &t.database).await;
        let all = hacivat_introspect::filter::parse_filter(
            "",
            DialectId::MySql,
            hacivat_introspect::filter::FilterRole::Introspect,
        )
        .expect("the empty filter");
        let r = intro(&t)
            .introspect(
                &mut c,
                &NsKey::server(),
                Mode::Full,
                Level::Names,
                &all,
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        assert!(r.existent.contains_key(&ObjectKind::Role), "{}", t.id);
    }
}

// ---- sources ----------------------------------------------------------------------------------

#[tokio::test]
#[ignore = "docker"]
async fn mysql_sources_facet_reads_show_create() {
    for t in targets(&ALL) {
        let db = Db::create(&t, "sources").await;
        db.run("CREATE TABLE `a``b` (id INT)").await;
        db.run("CREATE VIEW v AS SELECT id FROM `a``b`").await;
        let keys = [
            db.key(ObjectKind::Table, &["a`b"]),
            db.key(ObjectKind::View, &["v"]),
            db.key(ObjectKind::Table, &["gone"]),
            db.key(ObjectKind::Column, &["v", "id"]),
        ];
        let mut c = harness::meta(&t, &db.name).await;
        let got = intro(&t)
            .facet(&mut c, &db.ns(), &keys, FacetKind::Sources)
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
        let FacetRows::Sources(got) = got else {
            panic!("{}: {got:?}", t.id)
        };
        let names: Vec<&str> = got.iter().map(|(k, _)| k.name()).collect();
        assert_eq!(names, ["a`b", "v"], "{}", t.id);
        assert!(got[0].1.starts_with("CREATE TABLE `a``b`"), "{}", t.id);
        assert!(got[1].1.contains("VIEW `v`"), "{}: {}", t.id, got[1].1);
    }
}

// ---- every section ----------------------------------------------------------------------------

/// Every `20-facets` section prepares on the server and returns its declared columns: the
/// grant-table sections as `hv_catalog`, the others as `hv_test`.
#[tokio::test]
#[ignore = "docker"]
async fn every_mysql_facet_section_prepares() {
    let pack = Pack::builtin(DialectId::MySql).expect("pack");
    for t in targets(&ALL) {
        let db = Db::create(&t, "prepare").await;
        db.run("CREATE TABLE base_t (id INT)").await;
        db.run("CREATE VIEW v AS SELECT id FROM base_t").await;
        for login in ["hv_catalog", "hv_test"] {
            let mut c = if login == "hv_catalog" {
                catalog_meta(&t).await
            } else {
                harness::meta(&t, &db.name).await
            };
            let mut txn = c.begin_snapshot().await.expect("snapshot");
            let mut r = TxnReader::new(&mut txn, None).expect("reader");
            let srv = read_server(&mut r, t.engine).await.expect("identity");
            for s in sections::ALL {
                let grant_table = sections::GRANT_TABLES.contains(&s);
                if grant_table != (login == "hv_catalog") || !facets::renders(&srv, s) {
                    continue;
                }
                let names: Vec<_> = if s == sections::GRANTS_SCHEMA {
                    vec![hacivat_introspect::mysql::pass::text(&db.name)]
                } else {
                    ["base_t", "v"]
                        .into_iter()
                        .map(hacivat_introspect::mysql::pass::text)
                        .collect()
                };
                let params = Params {
                    scalars: vec![(n("schema"), hacivat_introspect::mysql::pass::text(&db.name))],
                    lists: vec![(n("names"), names)],
                };
                let rows = r
                    .read(s, srv.pack_ctx(Mode::Full, Level::Structure, params))
                    .await
                    .unwrap_or_else(|e| panic!("{} {login}: {s}: {e:?}", t.id));
                let got: Vec<&str> = rows.columns.iter().map(|c| &**c).collect();
                let want: Vec<&str> = pack
                    .section(s)
                    .unwrap()
                    .returns()
                    .iter()
                    .map(|c| &*c.name)
                    .collect();
                assert_eq!(got, want, "{} {login}: {s}", t.id);
                fact(&t, &format!("facet_section {s} rows={}", rows.rows.len()));
            }
            drop(r);
            txn.commit().await.expect("commit");
        }
    }
}
