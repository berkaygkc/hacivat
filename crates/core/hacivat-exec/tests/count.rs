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
use hacivat_exec::{CountError, CountRequest, ExecService, ExecUnit, count_prepared, count_sql};
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
// ## m3b-t18-safe06 (fix round 1, I1)
use hacivat_session::session::{ClassifierContext, RoAnswer};
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
    // ## m3b-t18-safe06 (fix round 1, I1): the trait has no default; this fake ignores the
    // context.
    fn check_read_only_in(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
        _ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let check = self.check_read_only(kind, sql, allow);
        Box::pin(async move {
            RoAnswer {
                check: check.await,
                changes_context: false,
            }
        })
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
    // ## m3b-t18-safe06 (fix round 1, I1): the trait has no default; this fake ignores the
    // context.
    fn check_read_only_in(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
        _ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let check = self.check_read_only(kind, sql, allow);
        Box::pin(async move {
            RoAnswer {
                check: check.await,
                changes_context: false,
            }
        })
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
        binds: Arc::from(Vec::new()),
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
    // The server was told (review m8).
    common::eventually("the statement is cancelled", || f.cancel_calls() > 0).await;
}

// ## m3b-t16-run-service (fix round 1, reviews m6, m7, m8)

/// A check that never answers (a classifier stuck on a pathological statement).
#[derive(Debug)]
struct Stuck;

impl StatementClassifier for Stuck {
    fn check_read_only(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
    ) -> BoxFuture<'static, RoCheck> {
        Box::pin(std::future::pending())
    }
    fn effects(&self, _: DbmsKind, _: Arc<str>) -> BoxFuture<'static, Option<SessionEffects>> {
        Box::pin(async { None })
    }
    // ## m3b-t18-safe06 (fix round 1, I1): the trait has no default; this fake ignores the
    // context.
    fn check_read_only_in(
        &self,
        kind: DbmsKind,
        sql: Arc<str>,
        allow: Arc<[Arc<str>]>,
        _ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let check = self.check_read_only(kind, sql, allow);
        Box::pin(async move {
            RoAnswer {
                check: check.await,
                changes_context: false,
            }
        })
    }
}

fn console_request(console: &SessionHandle) -> CountRequest {
    CountRequest {
        console: console.clone(),
        sessions: None,
        engine: DbmsKind::Postgres,
        sql: "SELECT COUNT(*) FROM (\nSELECT * FROM t\n) hv_c".into(),
        binds: Arc::from(Vec::new()),
    }
}

/// The cancel and the timeout cover the check (and the pool's connect), not only the
/// statement.
#[tokio::test(start_paused = true)]
async fn cancel_and_timeout_cover_the_check() {
    let (_b, s, f) = common::service_over(fake_pg()).await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Stuck));
    let ticket = svc.count(console_request(&s));
    tokio::task::yield_now().await;
    ticket.cancel();
    assert_eq!(ticket.done.await.unwrap(), Err(CountError::Cancelled));
    let e = svc.count(console_request(&s)).done.await.unwrap();
    assert_eq!(e, Err(CountError::TimedOut), "30 s of the paused clock");
    assert_eq!(counted(&f), 0, "nothing was sent");
}

/// Keeping only `done` (the ticket dropped) never cancels the count.
#[tokio::test]
async fn dropping_the_ticket_does_not_cancel() {
    let (_b, s, _f) =
        common::service_over(fake_pg().queries(vec![count_query(Duration::from_millis(50))])).await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Allow));
    let done = svc.count(console_request(&s)).done;
    assert_eq!(done.await.unwrap(), Ok(42));
}

/// A parameterised unit's count carries its binds in slot order; an `ORDER BY` holding a
/// placeholder stays.
#[tokio::test]
async fn a_parameterised_count_carries_its_binds() {
    let doc = common::pg_doc("SELECT * FROM t WHERE a = :a ORDER BY b");
    let unit = ExecUnit {
        text: "SELECT * FROM t WHERE a = :a ORDER BY b".into(),
        doc_range: 0..39,
        kind: UnitKind::Statement,
        suspendable: false,
        title: None,
    };
    assert_eq!(count_sql(&doc, &unit), None, "no values to bind");
    let mut p = common::m3b::prepared(&unit);
    p.final_text = "SELECT * FROM t WHERE a = $1 ORDER BY b".into();
    p.binds = Arc::from(vec![common::m3b::text_bind("7")]);
    let (sql, binds) = count_prepared(&doc, &p).expect("countable");
    assert_eq!(
        sql,
        "SELECT COUNT(*) FROM (\nSELECT * FROM t WHERE a = $1\n) hv_c"
    );
    assert_eq!(&*binds, &[common::m3b::text_bind("7")]);
    p.final_text = "SELECT * FROM t ORDER BY a = $1".into();
    let (sql, _) = count_prepared(&doc, &p).expect("countable");
    assert_eq!(
        sql,
        "SELECT COUNT(*) FROM (\nSELECT * FROM t ORDER BY a = $1\n) hv_c"
    );
    // The binds reach the server with the count.
    let (_b, s, f) =
        common::service_over(fake_pg().queries(vec![count_query(Duration::ZERO)])).await;
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Allow));
    let req = CountRequest {
        sql,
        binds,
        ..console_request(&s)
    };
    assert_eq!(svc.count(req).done.await.unwrap(), Ok(42));
    assert!(
        f.binds_seen()
            .iter()
            .any(|b| b == &[common::m3b::text_bind("7")])
    );
}

// ---------------------------------------------------------------------------------------------
// ## m3b-t24-exec-ui (fix round 1, I1; the T25 m2 carry)

/// Allows every statement and keeps the context each check was given.
#[derive(Debug, Default)]
struct AllowSeen(std::sync::Mutex<Vec<ClassifierContext>>);

impl StatementClassifier for AllowSeen {
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
    fn check_read_only_in(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
        ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        self.0.lock().unwrap().push(ctx);
        Box::pin(async {
            RoAnswer {
                check: RoCheck::Allow,
                changes_context: false,
            }
        })
    }
}

/// The session's own context read (T18's constant text) answering `on` and `app, public`.
fn context_read() -> hacivat_driver::fake::FakeExec {
    hacivat_driver::fake::FakeExec {
        prefix: "SELECT pg_catalog.current_setting('standard_conforming_strings')",
        result: Ok(hacivat_driver::ExecSummary {
            rows: vec![vec![Some("on".into()), Some("app, public".into())]],
            row_count: 1,
            ..hacivat_driver::ExecSummary::default()
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// A count the check allows runs, and it was checked in the context of the session it runs
/// on (a pooled Service session here), as that session read it.
#[tokio::test]
async fn an_allowed_count_is_checked_in_its_sessions_context() {
    let (_m, _console, f, req) = connected(
        fake_pg()
            .exec(vec![context_read(), context_read()])
            .queries(vec![count_query(Duration::ZERO)]),
    )
    .await;
    let seen = Arc::new(AllowSeen::default());
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(seen.clone());
    let n = svc.count(req).done.await.unwrap().expect("a count");
    assert_eq!(n, 42);
    assert_eq!(counted(&f), 1);
    let seen = seen.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].lex_modes.as_deref(), Some("on"));
    let path: Vec<&str> = seen[0]
        .search_path
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|s| &**s)
        .collect();
    assert_eq!(path, ["app", "public"]);
}

// ## m3b-t26-prod-unlock (T26 carry MUST 2): Count rows carries the context generation it was
// checked under, so the session refuses it when the context moved between the check and the
// send (here: the user's `SET` reaches the session while the check runs), sending nothing.

/// A check that, while it runs, has the user change the session's context (a `SET` the
/// session runs before the count's request), then allows.
#[derive(Debug)]
struct MovesTheContext(SessionHandle);

impl StatementClassifier for MovesTheContext {
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
    fn check_read_only_in(
        &self,
        _kind: DbmsKind,
        _sql: Arc<str>,
        _allow: Arc<[Arc<str>]>,
        _ctx: ClassifierContext,
    ) -> BoxFuture<'static, RoAnswer> {
        let ticket = self.0.submit(Request::Exec {
            sql: "SET search_path = elsewhere".to_string(),
            opts: hacivat_driver::ExecOpts::default(),
            origin: hacivat_session::session::ExecOrigin::User,
        });
        Box::pin(async move {
            let _ = ticket.reply.await;
            RoAnswer {
                check: RoCheck::Allow,
                changes_context: false,
            }
        })
    }
}

#[tokio::test]
async fn a_count_is_refused_when_the_context_moved_after_its_check() {
    let (_b, s, f) = common::service_over(fake_pg().queries(vec![
        count_query(Duration::ZERO),
        count_query(Duration::ZERO),
    ]))
    .await;
    let svc = ExecService::new(MemoryBudget::new(1 << 30, 256 << 20))
        .with_count_check(Arc::new(MovesTheContext(s.clone())));
    let e = svc
        .count(console_request(&s))
        .done
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(&e, CountError::Failed(e) if e.kind == hacivat_session::session::ExecErrorKind::ContextChanged),
        "{e:?}"
    );
    assert_eq!(counted(&f), 0, "nothing was sent");
    // The control: with nothing moving it, the same count runs (its generation holds).
    let svc =
        ExecService::new(MemoryBudget::new(1 << 30, 256 << 20)).with_count_check(Arc::new(Allow));
    let ok = svc.count(console_request(&s)).done.await.unwrap();
    assert_eq!(ok, Ok(42));
    assert_eq!(counted(&f), 1);
}
