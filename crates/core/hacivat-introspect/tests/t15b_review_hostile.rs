// ## m4-t15b-facets (fix round 1: the T15b security review's hostile probes, committed)
//! T15b security review: hostile probes of `fragment::on_session` over a scripted PostgreSQL
//! job session (FakeDriver, R-W5-8; no Docker). Reviewer-written; adapted by the fix round to
//! the fixed seam (`JoinedContext` answers the session's own temp schema; `on_session` takes
//! the catalog's `ServerFacts`). H1, H1b, H2 (foreign temp schema) and H3b failed at review
//! (I-1, m-1) and now assert the fixes (ruling R-W5-5; `MetaConn::join_as`).

#![cfg(feature = "session")]
#![allow(
    clippy::result_large_err,
    reason = "the seam's `IntroError` carries the server's message (T1c's type)"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_driver::{Driver, DriverRegistry, EngineCatalog};
use hacivat_introspect::fragment::{on_session, pg_path_is_safe};
use hacivat_introspect::introspector::IntroError;
use hacivat_meta::name::{DialectId, IdentCtx};
use hacivat_meta::{Level, Name, NsKey, ObjectKey, ObjectKind, ServerFacts};
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_net::{BoxedStream, StreamFactory, TlsMode};
use hacivat_proto::{DbmsKind, Department};
use hacivat_secrets::{
    AuthProviderId, CredentialFiles, MemoryBackend, Retention, SecretBackend, SecretStore,
};
use hacivat_session::budget_for;
use hacivat_session::meta::MetaError;
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction, TransportWrap};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_session::session::{
    ExecOrigin, JobKind, Request, SessionHandle, SessionKind, SessionManager, SessionSpec,
};
use hacivat_values::{LogicalType, Value};
use tokio_util::sync::CancellationToken;

fn wrap() -> TransportWrap {
    Arc::new(|_direct: StreamFactory| {
        StreamFactory::new(|| {
            Box::pin(async {
                let (near, _far) = tokio::io::duplex(64);
                Ok(Box::new(near) as BoxedStream)
            })
        })
    })
}

fn deps(fake: Arc<FakeDriver>) -> ConnectDeps {
    let mut drivers = DriverRegistry::new();
    drivers.register(fake as Arc<dyn Driver>);
    let known_hosts = KnownHosts::load(Vec::new(), PathBuf::from("/nonexistent/hv/known_hosts"));
    let tunnels = TunnelManager::new(known_hosts.clone(), Default::default());
    let backend: Arc<dyn SecretBackend> = Arc::new(MemoryBackend::new());
    ConnectDeps {
        drivers: Arc::new(drivers),
        catalog: EngineCatalog::embedded(),
        secrets: Arc::new(SecretStore::new(Some(backend))),
        files: CredentialFiles {
            pgpass: None,
            pg_service: None,
        },
        transport_wrap: Some(wrap()),
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

fn profile() -> Profile {
    let mut p = Profile {
        name: "rev".into(),
        engine: DbmsKind::Postgres,
        endpoint: ProfileEndpoint::Server {
            host: "127.0.0.1".into(),
            port: None,
        },
        user: Some("deploy".into()),
        auth: AuthProviderId::None,
        retention: Retention::Never,
        max_connections: 4,
        ..Profile::default()
    };
    p.tls.mode = TlsMode::Disable;
    p
}

fn text() -> LogicalType {
    LogicalType::Text {
        max_chars: None,
        fixed: false,
    }
}

fn q(prefix: &'static str, cols: &[&'static str], rows: Vec<Vec<&str>>) -> FakeQuery {
    FakeQuery {
        prefix,
        sets: vec![FakeResultSet {
            columns: cols.iter().map(|c| (*c, text())).collect(),
            rows: FakeRows::Literal(
                rows.into_iter()
                    .map(|r| r.into_iter().map(|c| Value::Text(Arc::from(c))).collect())
                    .collect(),
            ),
        }],
        ..FakeQuery::default()
    }
}

fn facts(dbms: DbmsKind) -> ServerFacts {
    let d = if dbms == DbmsKind::Postgres {
        DialectId::Postgres
    } else {
        DialectId::MySql
    };
    ServerFacts::new(dbms, IdentCtx::new(d))
}

/// A PG fake whose `JoinedContext` answers `path` on database `db` with the session's own temp
/// schema `pg_temp_3`, and whose other joined sections describe schema `s` (oid 2200) with
/// table `t` (oid 16400, column `a int4`).
fn scripted(db: &str, path: &[&str]) -> FakeDriver {
    let ctx: Vec<Vec<&str>> = path.iter().map(|p| vec![db, *p, "pg_temp_3"]).collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .queries(vec![
            q(
                "SELECT CAST(pg_catalog.current_database()",
                &["db", "nsp", "own_temp"],
                ctx,
            ),
            q(
                "SELECT n.oid AS oid",
                &["oid", "name"],
                vec![vec!["2200", "s"]],
            ),
            q(
                "SELECT c.oid AS oid",
                &["oid", "name", "relkind"],
                vec![vec!["16400", "t", "r"]],
            ),
            q(
                "SELECT a.attrelid AS relid",
                &[
                    "relid",
                    "attnum",
                    "name",
                    "type_schema",
                    "type_name",
                    "not_null",
                    "identity_kind",
                ],
                vec![vec!["16400", "1", "a", "pg_catalog", "int4", "f", ""]],
            ),
        ])
}

async fn job(sm: &Arc<SessionManager>) -> SessionHandle {
    sm.open(
        SessionSpec {
            kind: SessionKind::Job(1, JobKind::DdlApply),
            read_only: false,
            department: Department::Executor,
        },
        Interaction::Headless,
    )
    .await
    .unwrap_or_else(|e| panic!("opens: {:?}", e.class))
}

async fn begin(h: &SessionHandle) {
    let got = h
        .submit(Request::Exec {
            sql: "BEGIN".into(),
            opts: hacivat_driver::ExecOpts::default(),
            origin: ExecOrigin::User,
        })
        .reply
        .await
        .expect("reply");
    got.map(drop)
        .unwrap_or_else(|e| panic!("BEGIN: {:?}", e.kind));
    let mut st = h.state();
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        st.wait_for(|s| s.tx.status == hacivat_driver::TxStatus::InTx),
    )
    .await;
}

fn sent(f: &FakeDriver, from: usize) -> Vec<String> {
    f.call_log()[from..]
        .iter()
        .filter_map(|c| match c {
            FakeCall::Statement(s) => Some(format!("stmt:{s}")),
            FakeCall::Typed(FakeTypedCall::Query(s)) => Some(format!("query:{s}")),
            _ => None,
        })
        .collect()
}

fn ns() -> NsKey {
    NsKey::schema([Name::from("appdb"), Name::from("s")])
}

fn t_key() -> ObjectKey {
    ObjectKey {
        kind: ObjectKind::Table,
        path: ["appdb", "s", "t"].iter().map(|p| Name::from(*p)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    }
}

async fn joined(fake: FakeDriver, dbms: DbmsKind) -> (Result<(), IntroError>, Vec<String>) {
    let fake = Arc::new(fake);
    let p = profile();
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(p.clone()),
        Arc::new(deps(fake.clone())),
        Arc::new(budget_for(&p)),
    );
    let j = job(&sm).await;
    begin(&j).await;
    let from = fake.call_log().len();
    let got = tokio::time::timeout(
        Duration::from_secs(60),
        on_session(
            &j,
            &facts(dbms),
            &ns(),
            &[t_key()],
            Level::Structure,
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("bounded");
    (got.map(drop), sent(&fake, from))
}

// ---- H1: R-W3-11 cannot tell the session's own temp schema from another backend's ----------

/// PostgreSQL never looks up functions or operators in the session's *own* temp schema
/// (`myTempNamespace`), but it does search any *other* `pg_temp_N` named explicitly in the
/// path (namespace.c `OpernameGetOprid`/`OpernameGetCandidates` skip only `myTempNamespace`).
/// A superuser job whose path is `pg_temp_7, pg_catalog` (pg_temp_7 being an attacker's live
/// temp schema, set e.g. by `ALTER DATABASE … SET search_path` by the database owner) resolves
/// the unqualified `=` of the `IN :[oids]` bind in pg_temp_7 first.
#[test]
fn h1_a_foreign_temp_schema_before_pg_catalog_is_not_safe() {
    // The session's own temp schema (pg_temp_3) then a foreign one (pg_temp_7).
    assert!(
        !pg_path_is_safe(&["pg_temp_3", "pg_temp_7", "pg_catalog"], Some("pg_temp_3")),
        "a second pg_temp_N is another backend's: operators there are searched"
    );
}

#[test]
fn h1b_a_lone_foreign_temp_schema_first_is_not_safe() {
    // No own temp schema yet; the first entry is someone else's.
    assert!(!pg_path_is_safe(&["pg_temp_7", "pg_catalog"], None));
}

// ---- H2: the spoofed path end to end ---------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_control_a_user_schema_first_is_refused_before_anything_else() {
    let (got, log) = joined(
        scripted("appdb", &["evil", "pg_catalog"]),
        DbmsKind::Postgres,
    )
    .await;
    assert_eq!(
        got,
        Err(IntroError::Meta(MetaError::NotJoinable)),
        "{log:#?}"
    );
    assert_eq!(log.len(), 1, "only JoinedContext was sent: {log:#?}");
    assert!(log[0].contains("current_schemas"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_control_another_database_is_refused() {
    let (got, log) = joined(scripted("otherdb", &["pg_catalog"]), DbmsKind::Postgres).await;
    assert_eq!(
        got,
        Err(IntroError::Meta(MetaError::NotJoinable)),
        "{log:#?}"
    );
    assert_eq!(log.len(), 1, "{log:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_control_a_safe_path_reads_and_sends_only_joined_selects() {
    let (got, log) = joined(
        scripted("appdb", &["pg_temp_3", "pg_catalog", "public"]),
        DbmsKind::Postgres,
    )
    .await;
    assert_eq!(got, Ok(()), "{log:#?}");
    assert_eq!(log.len(), 4, "{log:#?}");
    for s in &log {
        assert!(s.starts_with("query:SELECT "), "{s}");
        for bad in [
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "SAVEPOINT",
            "SET ",
            "format_type",
            "pg_get_",
        ] {
            assert!(!s.contains(bad), "{bad} in {s}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h2_a_foreign_temp_schema_first_is_refused() {
    let (got, log) = joined(
        scripted("appdb", &["pg_temp_3", "pg_temp_7", "pg_catalog"]),
        DbmsKind::Postgres,
    )
    .await;
    assert_eq!(
        got,
        Err(IntroError::Meta(MetaError::NotJoinable)),
        "the IN-list bind was sent under a path whose first operator schema is pg_temp_7: {log:#?}"
    );
}

// ---- H3: a dbms mismatch ---------------------------------------------------------------------

/// The caller names the engine. On a PostgreSQL job, `DbmsKind::MySql` sends the MySQL
/// INFORMATION_SCHEMA text (bare `=`, text binds, `?` placeholders) to the PostgreSQL session
/// without the R-W3-11 probe. On a real server it is a parse error, which aborts the job's
/// transaction (25P02); nothing tells the caller the engine was wrong.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_a_dbms_mismatch_is_refused_before_anything_is_sent() {
    let (got, log) = joined(scripted("appdb", &["pg_catalog"]), DbmsKind::MySql).await;
    println!("H3 got={got:?}\nH3 sent={log:#?}");
    assert!(got.is_err());
    assert!(
        log.is_empty(),
        "text of another engine reached the job's session: {log:#?}"
    );
}

/// H3 with the namespace shape the named engine expects (one path element for MySQL): the
/// shape check above refused only by accident of the caller's key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3b_a_dbms_mismatch_with_a_mysql_shaped_namespace() {
    let fake = Arc::new(scripted("appdb", &["pg_catalog"]));
    let p = profile();
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(p.clone()),
        Arc::new(deps(fake.clone())),
        Arc::new(budget_for(&p)),
    );
    let j = job(&sm).await;
    begin(&j).await;
    let from = fake.call_log().len();
    let ns1 = NsKey::schema([Name::from("s")]);
    let key = ObjectKey {
        kind: ObjectKind::Table,
        path: ["s", "t"].iter().map(|p| Name::from(*p)).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let got = on_session(
        &j,
        &facts(DbmsKind::MySql),
        &ns1,
        &[key],
        Level::Structure,
        &CancellationToken::new(),
    )
    .await
    .map(drop);
    let log = sent(&fake, from);
    println!("H3b got={got:?}\nH3b sent={log:#?}");
    assert!(got.is_err());
    assert!(
        log.is_empty(),
        "text of another engine reached the job's session: {log:#?}"
    );
}

// ## m4-t15b-facets (implementer addition; CI run 44696885118, T16b m-7)

/// A facet read on a joined connection is refused at once by both network engines, never
/// answered empty (a key without an oid was otherwise left out: `Ok(Grants([]))` on CI), and
/// nothing reaches the job's session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_facet_read_on_a_joined_connection_is_refused_before_anything_is_sent() {
    use hacivat_introspect::Introspector;
    use hacivat_meta::props::FacetKind;
    let fake = Arc::new(scripted("appdb", &["pg_catalog"]));
    let p = profile();
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(p.clone()),
        Arc::new(deps(fake.clone())),
        Arc::new(budget_for(&p)),
    );
    let j = job(&sm).await;
    begin(&j).await;
    let from = fake.call_log().len();
    let mut with_oid = t_key();
    with_oid.server_id = Some(hacivat_meta::ServerId::Oid(16400));
    for kind in [
        FacetKind::Grants,
        FacetKind::Dependencies,
        FacetKind::Statistics,
    ] {
        for keys in [vec![t_key()], vec![with_oid.clone()], Vec::new()] {
            let mut c = hacivat_session::meta::MetaConn::join(&j).expect("joinable");
            let pg = hacivat_introspect::pg::PgIntrospector
                .facet(&mut c, &ns(), &keys, kind)
                .await;
            assert!(
                matches!(pg, Err(IntroError::Meta(MetaError::NotJoinable))),
                "pg {kind:?} {keys:?}: {pg:?}"
            );
            let my = hacivat_introspect::mysql::MySqlIntrospector {
                flavor: DbmsKind::MySql,
            }
            .facet(&mut c, &NsKey::schema([Name::from("s")]), &keys, kind)
            .await;
            assert!(
                matches!(my, Err(IntroError::Meta(MetaError::NotJoinable))),
                "mysql {kind:?} {keys:?}: {my:?}"
            );
        }
    }
    assert!(sent(&fake, from).is_empty(), "{:#?}", sent(&fake, from));
}
// ## m4-t15b-facets
