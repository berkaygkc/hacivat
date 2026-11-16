//! M3b Task 16: "Count rows" (R-M3-76, P-M3b-27): the count statement from the unit's tokens,
//! a separate query checked again before it runs, on a pooled `Service` session when the
//! console holds no transaction of the user's, else on the console; cancellable.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{fake_pg, profile_of};
use futures::future::BoxFuture;
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall};
use hacivat_driver::{DriverRegistry, EngineCatalog};
use hacivat_exec::{CountError, CountRequest, ExecService, ExecUnit, count_sql};
use hacivat_net::ssh::{KnownHosts, TunnelManager};
use hacivat_net::{BoxedStream, StreamFactory};
use hacivat_proto::DbmsKind;
use hacivat_results::MemoryBudget;
use hacivat_secrets::{CredentialFiles, MemoryBackend, SecretBackend, SecretStore};
use hacivat_session::pipeline::{ApprovedRoots, ConnectDeps, Interaction};
use hacivat_session::session::{
    Request, RoCheck, SafetyDenial, SessionEffects, SessionHandle, StatementClassifier, TxCommand,
};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};
use hacivat_values::{LogicalType, Value};
use sql_split::exec_range::UnitKind;

/// The count statement of the whole of `sql` (its `;` included: the wrap strips it).
fn count_sql_of(sql: &str) -> Option<String> {
    let doc = common::pg_doc(sql);
    let unit = ExecUnit {
        text: sql.into(),
        doc_range: 0..sql.len(),
        kind: UnitKind::Statement,
        suspendable: false,
        title: None,
    };
    count_sql(&doc, &unit)
}

#[test]
fn count_wrap_rules() {
    assert_eq!(
        count_sql_of("SELECT * FROM t ORDER BY a;").unwrap(),
        "SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c"
    );
    assert!(count_sql_of("SELECT * FROM t FOR UPDATE").is_none());
    assert_eq!(
        count_sql_of("SELECT * FROM t ORDER BY a LIMIT 5").unwrap(),
        "SELECT COUNT(*) FROM (\nSELECT * FROM t ORDER BY a LIMIT 5\n) hv_c"
    );
}

#[test]
fn count_wrap_refusals_and_trivia() {
    // Trailing trivia after the `;` and the `ORDER BY` go; an inner `ORDER BY` stays.
    assert_eq!(
        count_sql_of("SELECT * FROM t -- note\nORDER BY a ; -- end\n").unwrap(),
        "SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c"
    );
    assert_eq!(
        count_sql_of("SELECT * FROM (SELECT * FROM u ORDER BY b LIMIT 3) s").unwrap(),
        "SELECT COUNT(*) FROM (\nSELECT * FROM (SELECT * FROM u ORDER BY b LIMIT 3) s\n) hv_c"
    );
    assert_eq!(
        count_sql_of("WITH x AS (SELECT 1) SELECT * FROM x ORDER BY 1 OFFSET 2").unwrap(),
        "SELECT COUNT(*) FROM (\nWITH x AS (SELECT 1) SELECT * FROM x ORDER BY 1 OFFSET 2\n) hv_c"
    );
    for refused in [
        "DELETE FROM t",
        "SELECT * INTO t2 FROM t",
        "SELECT * FROM t FOR SHARE",
        "SELECT * FROM t FOR NO KEY UPDATE",
        "SELECT * FROM t FOR KEY SHARE",
        "SELECT * FROM t LOCK IN SHARE MODE",
        "SELECT 1; SELECT 2",
        "SELECT 'open",
        "",
    ] {
        assert!(count_sql_of(refused).is_none(), "{refused:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// The count over a connection manager on a fake

/// Allows every statement (the read-only check of a classifier that finds it a read).
#[derive(Debug)]
struct Allow;

impl StatementClassifier for Allow {
    fn check_read_only(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        Box::pin(async { RoCheck::Allow })
    }
    fn effects(&self, _: DbmsKind, _: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
        Box::pin(async { None })
    }
}

/// Refuses every statement.
#[derive(Debug)]
struct Deny;

impl StatementClassifier for Deny {
    fn check_read_only(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        Box::pin(async {
            RoCheck::Deny(SafetyDenial {
                class_id: "safety-class-write",
                reasons: Vec::new(),
            })
        })
    }
    fn effects(&self, _: DbmsKind, _: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
        Box::pin(async { None })
    }
}

fn forty_two(_: u64, _: usize) -> Value {
    Value::Int(42)
}

/// The count's answer: one row of `42`, after `delay`.
fn count_query(delay: Duration) -> FakeQuery {
    FakeQuery {
        prefix: "SELECT COUNT(*)",
        sets: vec![FakeResultSet {
            columns: vec![(
                "count",
                LogicalType::Int {
                    bits: 64,
                    signed: true,
                },
            )],
            rows: FakeRows::Generated {
                count: 1,
                cell: forty_two,
            },
        }],
        first_delay: delay,
        ..Default::default()
    }
}

/// Connect dependencies with `fake` as the only driver, over an in-memory transport.
fn deps_over(fake: Arc<FakeDriver>) -> ConnectDeps {
    let mut drivers = DriverRegistry::new();
    drivers.register(fake);
    let known_hosts = KnownHosts::load(Vec::new(), PathBuf::from("hacivat-test-known-hosts"));
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
        transport_wrap: Some(Arc::new(|_direct: StreamFactory| {
            StreamFactory::new(|| {
                Box::pin(async {
                    let (near, _far) = tokio::io::duplex(64);
                    Ok(Box::new(near) as BoxedStream)
                })
            })
        })),
        extra_steps: Vec::new(),
        approved_roots: ApprovedRoots::for_user(),
        timeout_override: None,
        tunnels,
        known_hosts,
    }
}

/// A connected profile over `fake`: the manager, its console (the default session) and the
/// fake.
async fn connected(
    fake: FakeDriver,
) -> (
    Arc<ConnectionManager>,
    SessionHandle,
    Arc<FakeDriver>,
    CountRequest,
) {
    let fake = Arc::new(fake);
    let m = ConnectionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(deps_over(fake.clone())),
        ManagerConfig::default(),
    );
    let profile = Arc::new(profile_of(DbmsKind::Postgres));
    m.connect(profile.clone(), None, Interaction::Headless);
    let mut changes = m.changes();
    loop {
        match m.state(profile.id) {
            LiveState::Connected(_) => break,
            LiveState::Failed(e) => panic!("the connect failed: {:?}", e.class),
            _ => {}
        }
        changes.changed().await.unwrap();
    }
    let sessions = m.sessions(profile.id).expect("connected");
    let console = sessions.default_session().expect("a default session");
    let req = CountRequest {
        console: console.clone(),
        sessions: Some(sessions),
        engine: DbmsKind::Postgres,
        sql: "SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c".into(),
    };
    (m, console, fake, req)
}

fn counted(f: &FakeDriver) -> usize {
    f.typed_log()
        .iter()
        .filter(|c| matches!(c, FakeTypedCall::Query(q) if q.starts_with("SELECT COUNT(*)")))
        .count()
}

#[tokio::test]
async fn count_uses_a_service_session_without_a_tx() {
    let (_m, console, f, req) = connected(fake_pg().queries(vec![
        count_query(Duration::ZERO),
        count_query(Duration::ZERO),
    ]))
    .await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Allow));
    assert_eq!(f.connect_calls(), 1);
    let n = svc.count(req.clone()).done.await.unwrap().expect("a count");
    assert_eq!(n, 42);
    assert_eq!(f.connect_calls(), 2, "a pooled Service session of its own");
    assert_eq!(counted(&f), 1);
    // Inside a transaction of the user's, the count runs on the console (no new session).
    console
        .submit(Request::Tx(TxCommand::Begin))
        .reply
        .await
        .unwrap()
        .expect("BEGIN");
    let n = svc.count(req.clone()).done.await.unwrap().expect("a count");
    assert_eq!(n, 42);
    assert_eq!(f.connect_calls(), 2, "the console's own session");
    assert_eq!(counted(&f), 2);
}

#[tokio::test]
async fn a_refused_count_sends_nothing() {
    let (_m, _console, f, req) =
        connected(fake_pg().queries(vec![count_query(Duration::ZERO)])).await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Deny));
    let e = svc.count(req).done.await.unwrap().unwrap_err();
    assert!(
        matches!(&e, CountError::Refused(d) if d.class_id == "safety-class-write"),
        "{e:?}"
    );
    assert_eq!(counted(&f), 0);
}

#[tokio::test]
async fn a_count_can_be_cancelled() {
    let (_m, _console, f, req) =
        connected(fake_pg().queries(vec![count_query(Duration::from_secs(20))])).await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Allow));
    let ticket = svc.count(req);
    common::eventually("the count is sent", || counted(&f) == 1).await;
    ticket.cancel();
    let e = tokio::time::timeout(Duration::from_secs(10), ticket.done)
        .await
        .expect("a cancel ends the count")
        .unwrap()
        .unwrap_err();
    assert_eq!(e, CountError::Cancelled);
}
