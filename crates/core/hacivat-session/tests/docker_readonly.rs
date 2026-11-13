//! M2b Task 18 over Docker: guarded read-only execution and the role probe against the real
//! drivers (HV-SESS-04, SQL-SAFE-05, MCP-09 step 1). Run with
//! `cargo xtask docker test --set pr|full -p hacivat-session`.
//!
//! - Targets come only from `require_target`/`targets(..)` (Q6), after `docker_enabled()`.
//! - The fixtures (`hv_probe`, `hv_ro` and the probe's fixture roles) are created idempotently
//!   through the admin connection (Q7), so no test depends on a recreated volume.
//! - Failure messages carry target ids, users and error classes, never a password.
//! - SQLite needs no server: its tests run in plain `cargo test`.

#![allow(
    clippy::result_large_err,
    reason = "Reply and ConnectError are the session's fixed interfaces"
)]

mod docker_support;

use std::sync::Arc;
use std::time::Instant;

use docker_support::*;
use futures::future::BoxFuture;
use hacivat_driver::{ExecOpts, ExecSummary, Protocol};
use hacivat_proto::{AccessLabel, DbmsKind, Department};
use hacivat_session::RoleWarning;
use hacivat_session::pipeline::{ConnectError, Interaction, LiveConnection, connect};
use hacivat_session::profile::Profile;
use hacivat_session::session::readonly::access_for;
use hacivat_session::session::{
    ClientId, DenyLayer, ExecErrorKind, ExecOrigin, Reply, Request, SessionHandle, SessionHooks,
    SessionKind, SessionSpec, SideConnector, spawn_session,
};
use hacivat_testkit::{DockerTarget, require_target, targets};

/// The read-only role of the fixtures (`docker/m2a/init/*.sql`).
const RO: &str = "hv_ro";
/// Fixture roles that read every table but can do more, each in its own way (T5 carry): a
/// member of the superuser, a column-level `UPDATE`, `CREATE` on `public`.
const RO_SUPERUSER_MEMBER: &str = "hv_ro_su";
const RO_COLUMN_UPDATE: &str = "hv_ro_col";
const RO_CREATE_PUBLIC: &str = "hv_ro_create";

// ---------------------------------------------------------------------------------------------
// Connections and sessions

/// Target `t`'s profile signed in as `user`; read-only when `read_only`. The floor is lifted so
/// PostgreSQL 13 (below it, R11) connects too.
fn profile_as(t: &DockerTarget, user: &str, read_only: bool) -> Profile {
    let mut p = profile_for(t, auto_or_require(t)).user(user);
    p.read_only = read_only;
    p.allow_unsupported_server = true;
    p
}

async fn connect_profile(t: &DockerTarget, p: Profile) -> LiveConnection {
    let user = p.user.clone().unwrap_or_default();
    connect(
        &deps(),
        req_pw(p, &t.password),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("{} as {user}: {:?}", t.id, e.class))
}

/// A read-only connect of `t` as `user` (its label and the probe's warnings).
async fn connect_ro_as(t: &DockerTarget, user: &str) -> LiveConnection {
    connect_profile(t, profile_as(t, user, true)).await
}

/// The test account, read-write: the fixtures' admin.
async fn admin(t: &DockerTarget) -> LiveConnection {
    connect_profile(t, profile_as(t, &t.user, false)).await
}

/// Side connections (cancel, kill) through the pipeline.
struct DockerSide {
    profile: Profile,
    password: String,
}

impl SideConnector for DockerSide {
    fn open_side(&self) -> BoxFuture<'static, Result<LiveConnection, ConnectError>> {
        let mut request = req_pw(self.profile.clone(), &self.password);
        request.department = Department::Cancel;
        Box::pin(async move {
            connect(&deps(), request, Interaction::Headless, token(), progress()).await
        })
    }
}

/// A read-only session of `t` as `user`, whose server-side name is `app` (so the admin can
/// find its backend).
async fn ro_session_as(t: &DockerTarget, user: &str, app: &str) -> SessionHandle {
    let mut p = profile_as(t, user, true);
    p.application_name = app.to_string();
    let live = connect_profile(t, p.clone()).await;
    let hooks = SessionHooks {
        side: Arc::new(DockerSide {
            profile: p.clone(),
            password: t.password.clone(),
        }),
        reconnect: None,
    };
    let spec = SessionSpec {
        kind: SessionKind::Console(ClientId::DEFAULT),
        read_only: true,
        department: Department::Console,
    };
    spawn_session(
        &tokio::runtime::Handle::current(),
        spec,
        Arc::new(p),
        live,
        hooks,
    )
}

async fn run(s: &SessionHandle, sql: &str) -> Reply {
    s.submit(Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    })
    .reply
    .await
    .expect("the actor replies")
}

/// The first value of the first row of `sql` on `live` (simple protocol, so values are kept).
async fn scalar(live: &mut LiveConnection, sql: &str) -> String {
    let answer: ExecSummary = live
        .raw()
        .execute(
            sql,
            ExecOpts {
                max_rows: 10,
                protocol: Protocol::Simple,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    answer
        .rows
        .first()
        .and_then(|row| row.first())
        .cloned()
        .flatten()
        .unwrap_or_default()
}

async fn batch(live: &mut LiveConnection, id: &str, sql: &str) {
    live.raw()
        .batch(sql)
        .await
        .unwrap_or_else(|e| panic!("{id}: fixture `{sql}`: {:?}", e.kind));
}

/// A name unique to this test run, for `application_name`.
fn unique(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{prefix}-{nanos}")
}

// ---------------------------------------------------------------------------------------------
// Fixtures (Q7)

/// `hv_probe`, `hv_ro` and the three fixture roles on PostgreSQL target `t`, idempotently.
/// `hv_ro` reads every table (`pg_read_all_data`, or `SELECT` on `public` before 14), with
/// TEMP and `CREATE` on `public` revoked from `PUBLIC` as the init script does.
async fn ensure_pg_fixtures(t: &DockerTarget) {
    let mut a = admin(t).await;
    let password = t.password.replace('\'', "''");
    let roles = [RO, RO_SUPERUSER_MEMBER, RO_COLUMN_UPDATE, RO_CREATE_PUBLIC];
    let mut create = String::new();
    for role in roles {
        create.push_str(&format!(
            "IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{role}') THEN \
             EXECUTE 'CREATE ROLE {role} LOGIN PASSWORD ''{}'''; END IF; ",
            password.replace('\'', "''")
        ));
    }
    let list = roles.join(", ");
    let statements = [
        "CREATE TABLE IF NOT EXISTS hv_probe (a int)".to_string(),
        format!("DO $$ BEGIN {create}END $$"),
        format!(
            "DO $$ BEGIN \
             IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pg_read_all_data') THEN \
             EXECUTE 'GRANT pg_read_all_data TO {list}'; \
             ELSE EXECUTE 'GRANT SELECT ON ALL TABLES IN SCHEMA public TO {list}'; \
             END IF; END $$"
        ),
        format!("REVOKE TEMP ON DATABASE \"{}\" FROM PUBLIC", t.database),
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC".to_string(),
        format!("GRANT \"{}\" TO {RO_SUPERUSER_MEMBER}", t.user),
        format!("GRANT UPDATE (a) ON hv_probe TO {RO_COLUMN_UPDATE}"),
        format!("GRANT CREATE ON SCHEMA public TO {RO_CREATE_PUBLIC}"),
    ];
    for sql in &statements {
        batch(&mut a, &t.id, sql).await;
    }
    a.close().await;
}

/// `hv_probe` and `hv_ro` (`SELECT, SHOW VIEW` on the test database) on MySQL or MariaDB
/// target `t`, created only when `hv_ro` cannot sign in (the init script makes it).
async fn ensure_mysql_fixtures(t: &DockerTarget) {
    let mut a = admin(t).await;
    batch(
        &mut a,
        &t.id,
        &format!(
            "CREATE TABLE IF NOT EXISTS `{}`.hv_probe (a int)",
            t.database
        ),
    )
    .await;
    let signs_in = connect(
        &deps(),
        req_pw(profile_as(t, RO, true), &t.password),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await;
    match signs_in {
        Ok(live) => live.close().await,
        Err(_) => {
            let password = t.password.replace('\\', "\\\\").replace('\'', "''");
            for sql in [
                format!("CREATE USER IF NOT EXISTS '{RO}'@'%' IDENTIFIED BY '{password}'"),
                format!(
                    "GRANT SELECT, SHOW VIEW ON `{}`.* TO '{RO}'@'%'",
                    t.database
                ),
            ] {
                batch(&mut a, &t.id, &sql).await;
            }
        }
    }
    a.close().await;
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

#[tokio::test]
#[ignore = "docker"]
async fn pg_hv_ro_can_read_a_table() {
    // Q21: TEMP is a warning only; the read itself runs inside the guard.
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    ensure_pg_fixtures(&t).await;
    let s = ro_session_as(&t, RO, &unique("hv-t18-read")).await;
    let read = run(&s, "SELECT count(*) FROM hv_probe").await;
    assert!(read.is_ok(), "{}: {:?}", t.id, read.map_err(|e| e.kind));
    // The server refuses a write inside the guard's read-only transaction.
    let write = run(&s, "INSERT INTO hv_probe VALUES (1)")
        .await
        .unwrap_err();
    assert_eq!(write.kind, ExecErrorKind::Db, "{}", t.id);
    // A transaction keyword never reaches the server; the session stays usable.
    assert_eq!(
        run(&s, "COMMIT").await.unwrap_err().kind,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction)
    );
    assert!(run(&s, "SELECT 1").await.is_ok(), "{}", t.id);
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_hv_ro_is_enforced_and_superuser_is_guarded() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    ensure_pg_fixtures(&t).await;
    let ro = connect_ro_as(&t, RO).await;
    assert_eq!(
        ro.access,
        AccessLabel::ReadOnlyEnforced,
        "{:?}",
        ro.access_warnings
    );
    ro.close().await;
    let su = connect_ro_as(&t, &t.user).await;
    assert_eq!(su.access, AccessLabel::ReadOnlyGuarded);
    assert!(
        su.access_warnings.contains(&RoleWarning::Superuser),
        "{:?}",
        su.access_warnings
    );
    su.close().await;
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_fixture_roles_with_more_than_read_are_guarded() {
    // T5 carry: superuser membership, a column-level UPDATE and CREATE on `public` each read
    // guarded, with the warning that says why.
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    ensure_pg_fixtures(&t).await;
    for (role, warning) in [
        (RO_SUPERUSER_MEMBER, RoleWarning::Superuser),
        (RO_COLUMN_UPDATE, RoleWarning::WritePrivileges),
        (RO_CREATE_PUBLIC, RoleWarning::WritePrivileges),
    ] {
        let live = connect_ro_as(&t, role).await;
        assert_eq!(live.access, AccessLabel::ReadOnlyGuarded, "{role}");
        assert!(
            live.access_warnings.contains(&warning),
            "{role}: {:?}",
            live.access_warnings
        );
        live.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_hv_ro_is_enforced_on_every_postgres_line() {
    // The plan amendment (cd4c538): with CREATE on `public` revoked from PUBLIC, `hv_ro` reads
    // enforced on every PostgreSQL line, 13 and 14 included.
    if !docker_enabled() {
        return;
    }
    let lines: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| t.id == format!("pg{}", t.major))
        .collect();
    for t in lines {
        ensure_pg_fixtures(&t).await;
        let ro = connect_ro_as(&t, RO).await;
        assert_eq!(
            ro.access,
            AccessLabel::ReadOnlyEnforced,
            "{}: {:?}",
            t.id,
            ro.access_warnings
        );
        ro.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_advisory_lock_and_listen_do_not_survive_a_request() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let app = unique("hv-t18-advisory");
    let s = ro_session_as(&t, &t.user, &app).await;
    run(&s, "SELECT pg_advisory_lock(42)").await.unwrap();
    let _ = run(&s, "LISTEN ch").await;
    let mut a = admin(&t).await;
    let held = scalar(
        &mut a,
        &format!(
            "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity s ON s.pid = l.pid \
             WHERE l.locktype = 'advisory' AND s.application_name = '{app}'"
        ),
    )
    .await;
    assert_eq!(held, "0", "{}", t.id);
    // The session's backend still runs under its startup name (the replay leaves it alone).
    let backends = scalar(
        &mut a,
        &format!("SELECT count(*) FROM pg_stat_activity WHERE application_name = '{app}'"),
    )
    .await;
    assert_eq!(backends, "1", "{}", t.id);
    a.close().await;
    // A guarded statement returns its row count, not its values.
    let channels = run(&s, "SELECT * FROM pg_listening_channels()")
        .await
        .unwrap();
    assert_eq!(channels.row_count, 0, "{}", t.id);
}

#[tokio::test]
#[ignore = "docker"]
async fn pg_probe_on_100k_tables_finishes_within_2s() {
    // Review probe: row 3's EXISTS over every relation. The 100k-table database is created once
    // per volume (about a minute), so this runs only under the full set.
    if !docker_enabled() || std::env::var("HACIVAT_DOCKER_SET").ok().as_deref() != Some("full") {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    ensure_pg_fixtures(&t).await;
    const DB: &str = "hv_probe_scale";
    const TABLES: u32 = 100_000;
    let mut a = admin(&t).await;
    if scalar(
        &mut a,
        &format!("SELECT count(*) FROM pg_database WHERE datname = '{DB}'"),
    )
    .await
        == "0"
    {
        batch(&mut a, &t.id, &format!("CREATE DATABASE {DB}")).await;
    }
    a.close().await;
    let mut scale = connect_profile(&t, profile_as(&t, &t.user, false).database(DB)).await;
    batch(&mut scale, &t.id, "CREATE SCHEMA IF NOT EXISTS scale").await;
    let have: u32 = scalar(
        &mut scale,
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'scale' AND c.relkind = 'r'",
    )
    .await
    .parse()
    .unwrap_or(0);
    // In chunks: each table takes a lock until its transaction ends.
    let mut from = have;
    while from < TABLES {
        let to = (from + 2_000).min(TABLES);
        batch(
            &mut scale,
            &t.id,
            &format!(
                "DO $$ BEGIN FOR i IN {from}..{} LOOP \
                 EXECUTE format('CREATE TABLE IF NOT EXISTS scale.t%s (a int)', i); \
                 END LOOP; END $$",
                to - 1
            ),
        )
        .await;
        from = to;
    }
    scale.close().await;

    let mut ro = connect_profile(&t, profile_as(&t, RO, true).database(DB)).await;
    let started = Instant::now();
    let (label, warnings) = access_for(ro.raw(), DbmsKind::Postgres, true).await;
    let took = started.elapsed();
    eprintln!("{}: role probe over {TABLES} tables took {took:?}", t.id);
    assert_eq!(label, AccessLabel::ReadOnlyEnforced, "{warnings:?}");
    assert!(took.as_secs_f64() < 2.0, "{}: {took:?}", t.id);
    ro.close().await;
}

// ---------------------------------------------------------------------------------------------
// MySQL and MariaDB

#[tokio::test]
#[ignore = "docker"]
async fn mysql_hv_ro_enforced_mariadb_guarded() {
    if !docker_enabled() {
        return;
    }
    if let Some(t) = require_target("mysql84") {
        ensure_mysql_fixtures(&t).await;
        let ro = connect_ro_as(&t, RO).await;
        assert_eq!(
            ro.access,
            AccessLabel::ReadOnlyEnforced,
            "{}: {:?}",
            t.id,
            ro.access_warnings
        );
        ro.close().await;
    }
    for t in targets(Some(DbmsKind::MariaDb)) {
        ensure_mysql_fixtures(&t).await;
        let ro = connect_ro_as(&t, RO).await;
        assert_eq!(ro.access, AccessLabel::ReadOnlyGuarded, "{}", t.id);
        // C19: no probe on MariaDB, so no warnings either.
        assert!(ro.access_warnings.is_empty(), "{}", t.id);
        ro.close().await;
    }
}

#[tokio::test]
#[ignore = "docker"]
async fn mysql_guarded_session_reads_and_refuses() {
    if !docker_enabled() {
        return;
    }
    for t in targets(Some(DbmsKind::MySql))
        .into_iter()
        .chain(targets(Some(DbmsKind::MariaDb)))
    {
        ensure_mysql_fixtures(&t).await;
        let s = ro_session_as(&t, RO, &unique("hv-t18-mysql")).await;
        assert!(
            run(&s, "SELECT count(*) FROM hv_probe").await.is_ok(),
            "{}",
            t.id
        );
        assert_eq!(
            run(&s, "INSERT INTO hv_probe VALUES (1)")
                .await
                .unwrap_err()
                .kind,
            ExecErrorKind::Db,
            "{}",
            t.id
        );
        // T13 carry: an executable comment is statement text.
        assert_eq!(
            run(&s, "/*!50000 COMMIT */").await.unwrap_err().kind,
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{}",
            t.id
        );
        assert!(run(&s, "SELECT 1").await.is_ok(), "{}", t.id);
    }
}

// ---------------------------------------------------------------------------------------------
// SQLite (no server)

/// A read-only connect of a fresh SQLite file with one table.
async fn sqlite_ro(dir: &tempfile::TempDir) -> (LiveConnection, Profile) {
    let path = dir.path().join("t18.db");
    let mut rw = connect(
        &deps(),
        req(sqlite_profile(&path)),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("sqlite create: {:?}", e.class));
    rw.raw()
        .batch("CREATE TABLE t (a int); INSERT INTO t VALUES (1)")
        .await
        .unwrap_or_else(|e| panic!("sqlite fixture: {:?}", e.kind));
    rw.close().await;
    let mut p = sqlite_profile(&path);
    p.read_only = true;
    // A read-only file is opened, never created.
    p.endpoint = hacivat_session::profile::ProfileEndpoint::File {
        path,
        create_if_missing: false,
    };
    let live = connect(
        &deps(),
        req(p.clone()),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("sqlite read-only: {:?}", e.class));
    (live, p)
}

#[tokio::test]
async fn sqlite_is_guarded() {
    let dir = tempfile::tempdir().unwrap();
    let (live, _) = sqlite_ro(&dir).await;
    assert_eq!(live.access, AccessLabel::ReadOnlyGuarded);
    assert!(live.access_warnings.is_empty());
    live.close().await;
}

#[tokio::test]
async fn sqlite_guarded_session_reads_and_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let (live, p) = sqlite_ro(&dir).await;
    let hooks = SessionHooks {
        side: Arc::new(DockerSide {
            profile: p.clone(),
            password: String::new(),
        }),
        reconnect: None,
    };
    let spec = SessionSpec {
        kind: SessionKind::Console(ClientId::DEFAULT),
        read_only: true,
        department: Department::Console,
    };
    let s = spawn_session(
        &tokio::runtime::Handle::current(),
        spec,
        Arc::new(p),
        live,
        hooks,
    );
    let read = run(&s, "SELECT a FROM t").await.unwrap();
    assert_eq!(read.row_count, 1);
    assert!(run(&s, "INSERT INTO t VALUES (2)").await.is_err());
    assert_eq!(
        run(&s, "BEGIN IMMEDIATE").await.unwrap_err().kind,
        ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction)
    );
    // Still usable after the refusals.
    assert_eq!(
        run(&s, "SELECT count(*) FROM t").await.unwrap().row_count,
        1
    );
}
