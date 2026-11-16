//! M3a Task 1b: the session's typed seams fail closed, the FakeDriver's typed scripting works
//! and keeps the driver contract (P5), and production consoles are read-only (R-M3-29, P4).

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    fake_mysql, fake_pg, manager_with, p, pg_profile, ro_profile, session, settle, wait_connected,
};
use futures::future::BoxFuture;
use hacivat_driver::fake::{
    FakeCall, FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_driver::{
    CancelHandle, CloseHow, Closed, Driver, DriverConnect, Endpoint, ExecOpts, ExecSummary,
    InsecureAuth, ProbeWarning, Protocol, QueryEvent, QuerySpec, RawConn, RawError, RawErrorKind,
    ServerInfo, TxStatus, TypedCaps,
};
use hacivat_net::{TlsMode, TlsSummary, TlsVerified};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_results::{FetchPolicy, ResultId, ResultWriter};
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    ErrorPolicy, ExecErrorKind, ObserverRef, RunEvent, RunId, RunObserver, RunSpec, RunUnit, UnitId,
};
use hacivat_session::{
    ConnectionManager, ManagerConfig, effective_read_only, pipeline::Interaction,
};
use hacivat_values::{LogicalType, ResultSchema, Value};

// ---------------------------------------------------------------------------------------------
// Fixtures

/// Records every event; a run that opens a result fails its test.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<RunEvent>>,
}

impl RunObserver for Recorder {
    fn open_result(&self, unit: UnitId, _: u32, _: Arc<ResultSchema>) -> ResultWriter {
        panic!("a result was opened for {unit:?}")
    }

    fn event(&self, ev: &RunEvent) {
        self.events.lock().unwrap().push(ev.clone());
    }
}

fn spec_of(sqls: &[&str], observer: Arc<Recorder>) -> RunSpec {
    let run = RunId(1);
    let units: Vec<RunUnit> = sqls
        .iter()
        .enumerate()
        .map(|(i, sql)| RunUnit {
            id: UnitId {
                run,
                ordinal: i as u32,
            },
            sql: (*sql).into(),
            protocol: Protocol::Extended,
            suspendable: true,
        })
        .collect();
    RunSpec {
        run,
        units: units.into(),
        policy: ErrorPolicy::StopOnError,
        fetch: FetchPolicy::default(),
        observer: ObserverRef(observer),
    }
}

/// A typed query as the session sends it: 501 first rows, 4,096 per chunk.
fn spec(sql: &str, suspendable: bool) -> QuerySpec<'_> {
    QuerySpec {
        sql,
        binds: &[],
        protocol: Protocol::Extended,
        suspendable,
        first_rows: 501,
        chunk_rows: 4_096,
        chunk_bytes: 4 << 20,
        cell_cap: 1 << 20,
        max_result_sets: 1,
    }
}

fn id_cell(row: u64, _: usize) -> Value {
    Value::Int(row as i64)
}

/// `count` generated rows of one `id int8` column.
fn generated(count: u64) -> FakeResultSet {
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

fn select(count: u64) -> FakeQuery {
    FakeQuery {
        prefix: "SELECT",
        sets: vec![generated(count)],
        ..Default::default()
    }
}

/// A connection straight from the fake driver (no session, no pipeline).
async fn connect_fake(f: &FakeDriver) -> Box<dyn RawConn> {
    let req = DriverConnect {
        endpoint: Endpoint::File {
            path: PathBuf::from("hv-test.db"),
            create_if_missing: false,
            read_only: false,
        },
        tls: None,
        user: Some("hv_test".into()),
        password: None,
        database: None,
        application_name: "Hacivat".into(),
        read_only: false,
        options: Vec::new(),
        connect_timeout: Duration::from_secs(15),
        events: Arc::new(|_| {}),
        insecure_auth: InsecureAuth::default(),
    };
    f.connect(req).await.expect("the fake connects")
}

fn rows_of(ev: &QueryEvent) -> u32 {
    match ev {
        QueryEvent::Rows(chunk) => chunk.rows(),
        other => panic!("expected rows, got {other:?}"),
    }
}

/// A raw connection with only the required methods: every typed method is the trait default.
#[derive(Default)]
struct BareConn;

impl RawConn for BareConn {
    fn kind(&self) -> DbmsKind {
        DbmsKind::Postgres
    }
    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        unreachable!("not called")
    }
    fn ping(&mut self) -> BoxFuture<'_, Result<Duration, RawError>> {
        unreachable!("not called")
    }
    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        Box::pin(async {
            TlsSummary {
                encrypted: false,
                version: None,
                cipher: None,
                verified: TlsVerified::No,
            }
        })
    }
    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        Box::pin(async { Vec::new() })
    }
    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        unreachable!("not called")
    }
    fn list_databases(&mut self, _: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        unreachable!("not called")
    }
    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        None
    }
    fn execute<'a>(
        &'a mut self,
        _: &'a str,
        _: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        unreachable!("not called")
    }
    fn batch<'a>(&'a mut self, _: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        unreachable!("not called")
    }
    fn tx_status(&self) -> TxStatus {
        TxStatus::InTx
    }
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        unreachable!("not called")
    }
    fn backend_id(&self) -> Option<u64> {
        None
    }
}

fn profile_env(env: EnvTag) -> Profile {
    Profile {
        environment: Some(env),
        ..pg_profile(TlsMode::Disable)
    }
}

// ---------------------------------------------------------------------------------------------
// Fail-closed stubs

// ## m3a-t18-session-run: Task 18 filled the run and cursor bodies, so the two stub tests of
// this place now pin what still fails closed on the same inputs (the runs themselves are in
// `tests/run.rs` and `tests/cursor.rs`).

#[tokio::test]
async fn a_portal_the_driver_refuses_fails_the_unit_and_opens_nothing() {
    // This fake does not follow `BEGIN` (no `tracks_transactions`), so its status stays idle
    // after Hacivat's own `BEGIN` and the driver refuses the suspendable query (P5). The
    // session reports the unit failed, opens no result, and ends its own transaction.
    let (h, f) = session(fake_pg().queries(vec![select(1)])).await;
    let observer = Arc::new(Recorder::default());
    let t = h.run(spec_of(&["SELECT 1"], observer.clone()));
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Internal
    );
    assert_eq!(&f.executed()[..], ["BEGIN", "ROLLBACK"]);
    // The refused query delivered nothing (`open_result` would have panicked), and the run
    // ended for its observer.
    assert_eq!(f.chunks_sent(), 0);
    let events = observer.events.lock().unwrap().clone();
    assert!(
        matches!(
            &events[..],
            [
                RunEvent::UnitStarted { .. },
                RunEvent::UnitEnded {
                    outcome: hacivat_session::session::UnitEnd::Failed(e),
                    ..
                },
                RunEvent::RunEnded(summary),
            ] if e.kind == ExecErrorKind::Internal && summary.failed == 1 && summary.aborted
        ),
        "{events:?}"
    );
}

#[tokio::test]
async fn cursor_requests_without_a_cursor_send_nothing() {
    use hacivat_results::{CloseReason, FetchControl};
    use hacivat_session::session::Request;
    let (h, f) = session(fake_pg()).await;
    let result = ResultId(7);
    for req in [
        Request::FetchMore { result, rows: 10 },
        Request::FetchAll { result },
        Request::CloseCursor {
            result,
            reason: CloseReason::UserClosed,
        },
    ] {
        // A late request (its result completed or was released) is a no-op, not an error.
        let reply = h.submit(req.clone()).reply.await.unwrap();
        assert!(reply.is_ok(), "{req:?}: {reply:?}");
    }
    // `FetchControl` submits the same requests and drops their tickets.
    h.fetch_more(result, 10);
    h.fetch_all(result);
    FetchControl::close(&h, result, CloseReason::TabClosed); // the inherent `close` closes the session
    settle().await;
    assert!(f.typed_log().is_empty());
    assert!(f.executed().is_empty());
}

#[tokio::test]
async fn typed_defaults_are_unsupported() {
    let mut c = BareConn;
    assert_eq!(c.typed_caps(), TypedCaps::NONE);
    assert_eq!(
        c.query(spec("SELECT 1", false)).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert_eq!(
        c.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert_eq!(
        c.fetch_more(10).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
}

// ---------------------------------------------------------------------------------------------
// FakeDriver typed scripting

#[tokio::test]
async fn fake_driver_scripts_a_suspending_query() {
    let f = fake_pg().tracks_transactions().queries(vec![select(1_200)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Rows(ch) if ch.rows() == 501));
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::Suspended
    ));
    assert_eq!(
        f.typed_log(),
        vec![FakeTypedCall::Query("SELECT * FROM t".into())]
    );
}

#[tokio::test]
async fn fake_pg_refuses_a_suspendable_query_outside_a_transaction() {
    let f = fake_pg().queries(vec![select(10)]);
    let mut c = connect_fake(&f).await;
    assert_eq!(
        c.query(spec("SELECT * FROM t", true))
            .await
            .unwrap_err()
            .kind,
        RawErrorKind::Unsupported
    ); // P5
    // Nothing is open: a non-suspendable query runs in autocommit.
    c.query(spec("SELECT * FROM t", false)).await.unwrap();
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultStart(_)
    ));
}

#[tokio::test]
async fn fetch_more_resumes_in_batches_and_the_query_ends() {
    let f = fake_pg().tracks_transactions().queries(vec![FakeQuery {
        rows_affected: None,
        tx_after: None,
        ..select(1_200)
    }]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    c.next_event().await.unwrap(); // ResultStart
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 501);
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    // `next_event` while suspended is refused: fetch_more or close first.
    assert_eq!(
        c.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    c.fetch_more(300).await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 300);
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.fetch_more(10_000).await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 399);
    match c.next_event().await.unwrap() {
        QueryEvent::ResultEnd(end) => assert_eq!((end.rows, end.kept), (1_200, 1_200)),
        other => panic!("{other:?}"),
    }
    match c.next_event().await.unwrap() {
        QueryEvent::Done(done) => {
            assert_eq!(done.tx, TxStatus::InTx);
            assert_eq!(done.skipped_result_sets, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(f.chunks_sent(), 3);
    assert_eq!(
        f.typed_log(),
        vec![
            FakeTypedCall::Query("SELECT * FROM t".into()),
            FakeTypedCall::FetchMore(300),
            FakeTypedCall::FetchMore(10_000),
        ]
    );
}

#[tokio::test]
async fn a_non_suspendable_query_streams_to_the_end_and_counts_later_sets() {
    let f = fake_mysql().queries(vec![FakeQuery {
        prefix: "CALL",
        sets: vec![generated(5_000), generated(3), generated(4)],
        ..Default::default()
    }]);
    let mut c = connect_fake(&f).await;
    c.query(spec("CALL p()", true)).await.unwrap();
    c.next_event().await.unwrap(); // ResultStart
    // A multi-result unit never suspends (R-M3-25): 501, then 4,096, then the rest.
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 501);
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 4_096);
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 403);
    assert!(matches!(
        c.next_event().await.unwrap(),
        QueryEvent::ResultEnd(_)
    ));
    match c.next_event().await.unwrap() {
        QueryEvent::Done(done) => assert_eq!(done.skipped_result_sets, 2),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_unscripted_statement_is_one_empty_result_with_its_update_count() {
    let f = fake_pg().queries(vec![FakeQuery {
        prefix: "UPDATE",
        rows_affected: Some(12),
        ..Default::default()
    }]);
    let mut c = connect_fake(&f).await;
    c.query(spec("UPDATE t SET a = 1", false)).await.unwrap();
    match c.next_event().await.unwrap() {
        QueryEvent::ResultStart(schema) => assert!(schema.columns.is_empty()),
        other => panic!("{other:?}"),
    }
    match c.next_event().await.unwrap() {
        QueryEvent::ResultEnd(end) => assert_eq!((end.rows, end.rows_affected), (0, Some(12))),
        other => panic!("{other:?}"),
    }
    assert!(matches!(c.next_event().await.unwrap(), QueryEvent::Done(_)));
    // The query is closed: its events are over.
    assert_eq!(
        c.next_event().await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
}

#[tokio::test]
async fn one_open_query_per_connection_except_a_suspended_pg_portal() {
    // MySQL: no statement and no second query while a query is open.
    let f = fake_mysql().queries(vec![select(10_000)]);
    let mut c = connect_fake(&f).await;
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    assert_eq!(
        c.batch("SELECT 1").await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    assert_eq!(
        c.query(spec("SELECT 2", false)).await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    // PostgreSQL: the canary runs beside a suspended portal (P5), never beside a running one.
    let f = fake_pg()
        .tracks_transactions()
        .queries(vec![select(10_000)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    assert_eq!(
        c.batch("SELECT 1").await.unwrap_err().kind,
        RawErrorKind::Unsupported
    );
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.execute("SELECT txid_current_if_assigned()", ExecOpts::values(1))
        .await
        .unwrap();
    c.fetch_more(10).await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 10);
    assert_eq!(
        f.call_log(),
        vec![
            FakeCall::Statement("BEGIN".into()),
            FakeCall::Typed(FakeTypedCall::Query("SELECT * FROM t".into())),
            FakeCall::Statement("SELECT 1".into()),
            FakeCall::Statement("SELECT txid_current_if_assigned()".into()),
            FakeCall::Typed(FakeTypedCall::FetchMore(10)),
        ]
    );
}

#[tokio::test]
async fn a_cancel_inside_a_pg_transaction_fails_it() {
    let f = fake_pg().tracks_transactions().queries(vec![FakeQuery {
        chunk_delay: Duration::from_secs(30),
        ..select(100_000)
    }]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT * FROM t", false)).await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 501);
    let cancel = c.cancel_handle().expect("the fake can cancel");
    let next = c.next_event();
    let (event, _) = tokio::join!(next, async {
        tokio::task::yield_now().await;
        cancel.cancel().await
    });
    assert_eq!(event.unwrap_err().kind, RawErrorKind::Cancelled);
    assert_eq!(c.tx_status(), TxStatus::Failed); // as psql (P16)
    assert_eq!(f.cancel_calls(), 1);
}

#[tokio::test]
async fn closing_drains_a_small_rest_and_mysql_asks_for_a_kill() {
    let f = fake_pg()
        .tracks_transactions()
        .queries(vec![select(600), select(50_000)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT a", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
            .unwrap(),
        Closed::Completed { rows: 600 }
    );
    c.query(spec("SELECT b", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
            .unwrap(),
        Closed::Closed { rows: 501 }
    );
    let f = fake_mysql().backend(42).queries(vec![FakeQuery {
        kill_needed: true,
        ..select(50_000)
    }]);
    let mut c = connect_fake(&f).await;
    c.query(spec("SELECT c", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
            .unwrap(),
        Closed::NeedsServerKill { thread_id: 42 }
    );
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(1)))
            .await
            .unwrap(),
        Closed::Closed { rows: 501 }
    );
    assert!(c.typed_caps().close_needs_kill);
}

#[tokio::test]
async fn tracked_transactions_follow_the_keywords() {
    let f = fake_pg().tracks_transactions();
    let mut c = connect_fake(&f).await;
    for (sql, want) in [
        ("BEGIN", TxStatus::InTx),
        ("SAVEPOINT hv_cursor", TxStatus::InTx),
        (
            "ROLLBACK TO SAVEPOINT hv_cursor; RELEASE SAVEPOINT hv_cursor",
            TxStatus::InTx,
        ),
        ("COMMIT", TxStatus::Idle),
        ("START TRANSACTION READ ONLY", TxStatus::InTx),
        ("ROLLBACK", TxStatus::Idle),
    ] {
        c.batch(sql).await.unwrap();
        assert_eq!(c.tx_status(), want, "{sql}");
    }
    // Without tracking a statement leaves the status alone (the M2b behaviour).
    let f = fake_pg();
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
}

// ---------------------------------------------------------------------------------------------
// Production consoles are read-only (R-M3-29, P4)

#[test]
fn production_is_read_only_only_with_the_flag() {
    let p = profile_env(EnvTag::Production);
    assert!(effective_read_only(&p, true));
    assert!(!effective_read_only(&p, false));
    assert!(effective_read_only(&ro_profile(), false));
    assert!(!effective_read_only(&profile_env(EnvTag::Staging), true));
}

#[tokio::test]
async fn production_policy_reaches_a_connected_session() {
    let config = ManagerConfig {
        production_read_only: true,
        ..ManagerConfig::m2b()
    };
    let (m, fake) = manager_with(FakeDriver::new(DbmsKind::Postgres), config);
    let prod = Arc::new(profile_env(EnvTag::Production));
    let dev = Arc::new(Profile {
        id: ProfileId::new(),
        ..profile_env(EnvTag::Dev)
    });
    for profile in [&prod, &dev] {
        m.connect(profile.clone(), None, Interaction::Headless);
        wait_connected(&m, profile.id).await;
    }
    let read_only = |m: &ConnectionManager, id| {
        let s = m.sessions(id).unwrap().default_session().unwrap();
        s.state().borrow().read_only
    };
    assert!(read_only(&m, prod.id));
    assert!(!read_only(&m, dev.id));
    // The policy is readable where the app needs it (fix round 1).
    assert!(m.policy().production_read_only);
    assert!(m.sessions(prod.id).unwrap().policy().production_read_only);
    // Both connects went through the pipeline as the session runs: production read-only.
    let requests = fake.requests();
    assert_eq!(requests.iter().filter(|r| r.read_only).count(), 1);
    // Without the policy (the M2b default) the production profile is read-write.
    let (m, _) = manager_with(FakeDriver::new(DbmsKind::Postgres), ManagerConfig::m2b());
    m.connect(prod.clone(), None, Interaction::Headless);
    wait_connected(&m, prod.id).await;
    assert!(!read_only(&m, prod.id));
    assert!(!m.policy().production_read_only);
}

#[tokio::test]
async fn budget_use_reports_in_use_and_max() {
    let (m, _) = manager_with(
        FakeDriver::new(DbmsKind::Postgres),
        ManagerConfig::default(),
    );
    m.connect(p(), None, Interaction::Headless);
    wait_connected(&m, p().id).await;
    let sessions = m.sessions(p().id).unwrap();
    let (in_use, max) = sessions.budget_use();
    assert_eq!(in_use, 1, "the default session holds a permit");
    assert!(max >= in_use);
}

// ---------------------------------------------------------------------------------------------
// ## m3a-t01b-seams (fix round 1): FakeDriver matches PostgreSQL where the session relies on
// it (I5; the first two are the review's hostile probes).

/// The server's SQLSTATE of a failed call.
fn sqlstate(e: &RawError) -> Option<&str> {
    e.server.as_ref().and_then(|m| m.sqlstate.as_deref())
}

/// PostgreSQL destroys a non-holdable portal when its transaction ends: a session that ends
/// Hacivat's transaction before closing the cursor cannot keep fetching.
#[tokio::test]
async fn a_suspended_pg_portal_does_not_survive_commit() {
    let f = fake_pg()
        .tracks_transactions()
        .queries(vec![select(10_000)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.batch("COMMIT").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    let resumed = c.fetch_more(10).await;
    let next = if resumed.is_ok() {
        Some(c.next_event().await)
    } else {
        None
    };
    assert!(
        resumed.is_err() || next.as_ref().is_some_and(|n| n.is_err()),
        "fetch_more after COMMIT: {resumed:?} then {next:?} (real PostgreSQL: portal does not exist)"
    );
    assert_eq!(resumed.as_ref().err().and_then(sqlstate), Some("34000"));
    // Closing the destroyed portal is no error (PostgreSQL's Close of a missing portal), and
    // the connection takes a new query.
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 0 }
    );
    c.query(spec("SELECT 1", false)).await.unwrap();
}

/// `ROLLBACK` destroys the portal too; `ROLLBACK TO SAVEPOINT` inside the transaction keeps it
/// (the transaction goes on).
#[tokio::test]
async fn rollback_destroys_a_suspended_pg_portal_and_rollback_to_keeps_it() {
    let f = fake_pg()
        .tracks_transactions()
        .queries(vec![select(10_000), select(10_000)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    c.query(spec("SELECT a", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.batch("SAVEPOINT s").await.unwrap();
    c.batch("ROLLBACK TO SAVEPOINT s").await.unwrap();
    c.fetch_more(10).await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 10);
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.batch("ROLLBACK").await.unwrap();
    let e = c.fetch_more(10).await.unwrap_err();
    assert_eq!(sqlstate(&e), Some("34000"));
}

/// In a failed PostgreSQL transaction every command errors until ROLLBACK.
#[tokio::test]
async fn a_pg_query_in_a_failed_transaction_is_refused() {
    let f = fake_pg().tracks_transactions().queries(vec![select(10)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    // A cancel inside the transaction fails it (the fake models this, P16).
    let cancel = c.cancel_handle().expect("cancel handle");
    c.query(spec("SELECT * FROM t", false)).await.unwrap();
    cancel.cancel().await.expect("the cancel is sent");
    let _ = c.next_event().await;
    assert_eq!(c.tx_status(), TxStatus::Failed);
    let r = c.query(spec("SELECT * FROM u", true)).await;
    let first = if r.is_ok() {
        Some(c.next_event().await)
    } else {
        None
    };
    assert!(
        r.is_err() || first.as_ref().is_some_and(|e| e.is_err()),
        "query in a Failed tx: {r:?} then {first:?}"
    );
    assert_eq!(r.as_ref().err().and_then(sqlstate), Some("25P02"));
}

/// The same rule for plain statements: refused with 25P02, except the ones that end the
/// transaction or roll back to a savepoint.
#[tokio::test]
async fn a_failed_pg_transaction_takes_only_its_end_or_a_rollback_to() {
    let boom = || hacivat_driver::fake::FakeExec {
        prefix: "SELECT boom",
        result: Err(RawError {
            kind: RawErrorKind::Db,
            server: None,
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let f = fake_pg().tracks_transactions().exec(vec![boom(), boom()]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap();
    assert!(c.execute("SELECT boom", ExecOpts::default()).await.is_err());
    assert_eq!(c.tx_status(), TxStatus::Failed);
    let e = c
        .execute("SELECT 1", ExecOpts::default())
        .await
        .unwrap_err();
    assert_eq!(sqlstate(&e), Some("25P02"));
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::Idle);
    c.batch("SELECT 1").await.unwrap();
    // `ROLLBACK TO SAVEPOINT` recovers a failed transaction, which goes on.
    c.batch("BEGIN").await.unwrap();
    c.batch("SAVEPOINT s").await.unwrap();
    assert!(c.execute("SELECT boom", ExecOpts::default()).await.is_err());
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK TO SAVEPOINT s").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    c.batch("SELECT 1").await.unwrap();
}

/// MySQL closes a suspended query early by a kill: its caps say so, and a drain past what one
/// read takes asks for it without a script flag.
#[tokio::test]
async fn a_mysql_drain_of_a_large_rest_asks_for_a_kill_by_its_caps() {
    let f = fake_mysql()
        .backend(7)
        .queries(vec![select(50_000), select(600)]);
    let mut c = connect_fake(&f).await;
    assert!(c.typed_caps().close_needs_kill);
    c.query(spec("SELECT a", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
            .unwrap(),
        Closed::NeedsServerKill { thread_id: 7 }
    );
    assert_eq!(
        c.close_query(CloseHow::AfterKill(Duration::from_secs(1)))
            .await
            .unwrap(),
        Closed::Closed { rows: 501 }
    );
    // A small rest is drained, no kill.
    c.query(spec("SELECT b", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(
        c.close_query(CloseHow::Drain(Duration::from_millis(250)))
            .await
            .unwrap(),
        Closed::Completed { rows: 600 }
    );
}

/// M3a's typed path is the extended protocol only (`typed.rs`): `Simple` is refused before
/// anything is sent, on every engine.
#[tokio::test]
async fn a_simple_protocol_query_is_refused() {
    for f in [fake_pg().tracks_transactions(), fake_mysql()] {
        let mut c = connect_fake(&f).await;
        let e = c
            .query(QuerySpec {
                protocol: Protocol::Simple,
                ..spec("SELECT 1", false)
            })
            .await
            .unwrap_err();
        assert_eq!(e.kind, RawErrorKind::Unsupported);
        assert_eq!(
            c.next_event().await.unwrap_err().kind,
            RawErrorKind::Unsupported
        );
    }
}

// ## m3a-t01b-seams (round 2): the review's round-1 probe.

/// PostgreSQL closes a portal created inside a savepoint when that savepoint is rolled back
/// (R-M3-23's `SAVEPOINT hv_cursor`); one created before the savepoint survives (above).
#[tokio::test]
async fn a_portal_opened_inside_a_savepoint_does_not_survive_rollback_to_it() {
    let f = fake_pg()
        .tracks_transactions()
        .queries(vec![select(10_000), select(10_000)]);
    let mut c = connect_fake(&f).await;
    c.batch("BEGIN").await.unwrap(); // the user's transaction
    c.batch("SAVEPOINT hv_cursor").await.unwrap(); // Hacivat's, around the cursor
    c.query(spec("SELECT * FROM t", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.batch("ROLLBACK TO SAVEPOINT hv_cursor").await.unwrap();
    let resumed = c.fetch_more(10).await;
    let next = if resumed.is_ok() {
        Some(c.next_event().await)
    } else {
        None
    };
    assert!(
        resumed.is_err() || next.as_ref().is_some_and(|n| n.is_err()),
        "fetch_more after ROLLBACK TO the portal's own savepoint: {resumed:?} then {next:?}"
    );
    assert_eq!(resumed.as_ref().err().and_then(sqlstate), Some("34000"));
    // m3a-t12 fix round 1 (review I4b, named exception): the Execute's 34000 inside the block
    // fails the transaction, as on PostgreSQL (pinned by hacivat-driver-pg's Docker test
    // `a_portal_its_transaction_ended_is_gone_as_the_fake_says`); a ROLLBACK TO recovers it.
    // The transaction goes on, and so does the savepoint: a portal opened inside a later
    // savepoint that is released (not rolled back) survives.
    assert_eq!(c.tx_status(), TxStatus::Failed);
    c.batch("ROLLBACK TO SAVEPOINT hv_cursor").await.unwrap();
    assert_eq!(c.tx_status(), TxStatus::InTx);
    assert_eq!(
        c.close_query(CloseHow::Now).await.unwrap(),
        Closed::Closed { rows: 0 }
    );
    c.batch("SAVEPOINT inner").await.unwrap();
    c.query(spec("SELECT * FROM u", true)).await.unwrap();
    c.next_event().await.unwrap();
    c.next_event().await.unwrap();
    assert_eq!(c.next_event().await.unwrap(), QueryEvent::Suspended);
    c.batch("RELEASE SAVEPOINT inner").await.unwrap();
    c.fetch_more(10).await.unwrap();
    assert_eq!(rows_of(&c.next_event().await.unwrap()), 10);
}
