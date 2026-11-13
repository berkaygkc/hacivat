//! A profile's `SessionManager` (HV-SESS-01, -02, -08; D-035; Task 15): budgeted sessions,
//! reset-on-release pools with idle close, single-session borrows (Q19), side connections
//! outside the budget, and the session overlay (Q17).

mod common;

use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use hacivat_driver::fake::FakeDriver;
use hacivat_driver::{
    CancelHandle, Driver, DriverConnect, DriverDescriptor, DriverRegistry, ExecOpts, ExecSummary,
    ProbeWarning, RawConn, RawError, ServerInfo, TxStatus,
};
use hacivat_net::{TlsMode, TlsSummary};
use hacivat_proto::{DbmsKind, Department};
use hacivat_session::budget_for;
use hacivat_session::pipeline::ConnectDeps;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    CancelOutcome, ClientId, ClosePolicy, Request, SessionHandle, SessionKind, SessionManager,
    SessionSpec, SessionState,
};
use tokio::time::timeout;

use common::*;

// ---------------------------------------------------------------------------------------------
// Fixtures

/// A PostgreSQL fake whose reset verify probe (`SELECT current_user = session_user`) answers
/// true for the first eight resets: the verify fails closed on no row (Task 7), and an
/// unscripted fake statement has none. Shadows `common::fake_pg` in this file only.
fn fake_pg() -> FakeDriver {
    common::fake_pg().exec(
        (0..8)
            .map(|_| rows("SELECT current_user = session_user", &[&["t"]]))
            .collect(),
    )
}

/// A user statement whose duration the driver's script sets.
fn exec_slow(sql: &str) -> Request {
    exec(sql)
}

fn console_spec(n: u64) -> SessionSpec {
    SessionSpec {
        kind: SessionKind::Console(ClientId(n)),
        read_only: false,
        department: Department::Console,
    }
}

fn profile_for(kind: DbmsKind) -> Profile {
    match kind {
        DbmsKind::MySql | DbmsKind::MariaDb => mysql_profile(TlsMode::Disable),
        DbmsKind::Sqlite => sqlite_profile(),
        _ => pg_profile(TlsMode::Disable),
    }
}

/// A manager of `profile` over `fake` (wrapped so a side `KILL QUERY` reaches the statement
/// it names, as a server's does), with the profile's own budget.
fn manager_over(fake: FakeDriver, profile: Profile) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let (deps, fake) = deps_with(fake);
    let deps = linked(deps, &fake);
    let budget = Arc::new(budget_for(&profile));
    let sm = SessionManager::new(
        tokio::runtime::Handle::current(),
        Arc::new(profile),
        Arc::new(deps),
        budget,
    );
    (sm, fake)
}

fn manager_n(fake: FakeDriver, n: u8) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let kind = Driver::descriptor(&fake).kind;
    let profile = Profile {
        max_connections: n,
        ..profile_for(kind)
    };
    manager_over(fake, profile)
}

fn manager4(fake: FakeDriver) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    manager_n(fake, 4)
}

fn manager_single(fake: FakeDriver) -> (Arc<SessionManager>, Arc<FakeDriver>) {
    let kind = Driver::descriptor(&fake).kind;
    let profile = Profile {
        single_session: true,
        ..profile_for(kind)
    };
    manager_over(fake, profile)
}

fn manager_sqlite() -> (Arc<SessionManager>, Arc<FakeDriver>) {
    manager_over(FakeDriver::new(DbmsKind::Sqlite), sqlite_profile())
}

/// Waits until `n` resets ran, then lets the releases park their sessions.
async fn parked(f: &FakeDriver, n: usize) {
    settle_until("the releases reset their sessions", || f.reset_calls() >= n).await;
    settle().await;
}

/// The profile of the test fixtures, moved to another server.
fn moved(p: Profile) -> Profile {
    Profile {
        endpoint: hacivat_session::profile::ProfileEndpoint::Server {
            host: "10.9.9.9".into(),
            port: None,
        },
        ..p
    }
}

/// The host the last connect went to.
fn last_host(f: &FakeDriver) -> String {
    match f.last_request().and_then(|r| r.endpoint) {
        Some(hacivat_driver::fake::FakeEndpoint::Network { tls_host, .. }) => tls_host,
        other => panic!("no network connect: {other:?}"),
    }
}

fn host_of(p: &Profile) -> String {
    match &p.endpoint {
        hacivat_session::profile::ProfileEndpoint::Server { host, .. } => host.clone(),
        other => panic!("not a server profile: {other:?}"),
    }
}

async fn open_n(sm: &Arc<SessionManager>, n: u64) -> Vec<SessionHandle> {
    let mut held = Vec::new();
    for i in 1..=n {
        held.push(sm.open(console_spec(i), ix_none()).await.unwrap());
    }
    held
}

// A driver whose `KILL QUERY` cancels the statement running on the other connections: the
// fake's side connection is not linked to the session's, a server's is.

type Peers = Arc<Mutex<Vec<Arc<dyn CancelHandle>>>>;

struct Linked {
    fake: Arc<FakeDriver>,
    peers: Peers,
}

fn linked(mut deps: ConnectDeps, fake: &Arc<FakeDriver>) -> ConnectDeps {
    let mut drivers = DriverRegistry::new();
    drivers.register(Arc::new(Linked {
        fake: fake.clone(),
        peers: Peers::default(),
    }));
    deps.drivers = Arc::new(drivers);
    deps
}

impl Driver for Linked {
    fn descriptor(&self) -> &DriverDescriptor {
        Driver::descriptor(self.fake.as_ref())
    }

    fn connect(
        &self,
        req: DriverConnect,
    ) -> BoxFuture<'static, Result<Box<dyn RawConn>, RawError>> {
        let connecting = self.fake.connect(req);
        let peers = self.peers.clone();
        Box::pin(async move {
            let inner = connecting.await?;
            let me = {
                let mut list = peers.lock().unwrap();
                list.extend(inner.cancel_handle());
                list.len() - 1
            };
            Ok(Box::new(LinkedConn { inner, peers, me }) as Box<dyn RawConn>)
        })
    }
}

struct LinkedConn {
    inner: Box<dyn RawConn>,
    peers: Peers,
    me: usize,
}

impl LinkedConn {
    /// A `KILL QUERY` cancels what runs on every other connection.
    fn kill_peers(&self, sql: &str) {
        if !sql.trim_start().starts_with("KILL QUERY") {
            return;
        }
        let peers = self.peers.lock().unwrap().clone();
        for (i, peer) in peers.iter().enumerate() {
            if i != self.me {
                drop(peer.cancel());
            }
        }
    }
}

impl RawConn for LinkedConn {
    fn kind(&self) -> DbmsKind {
        self.inner.kind()
    }
    fn server_info(&mut self) -> BoxFuture<'_, Result<ServerInfo, RawError>> {
        self.inner.server_info()
    }
    fn ping(&mut self) -> BoxFuture<'_, Result<std::time::Duration, RawError>> {
        self.inner.ping()
    }
    fn tls_summary(&mut self) -> BoxFuture<'_, TlsSummary> {
        self.inner.tls_summary()
    }
    fn probe(&mut self) -> BoxFuture<'_, Vec<ProbeWarning>> {
        self.inner.probe()
    }
    fn current_user(&mut self) -> BoxFuture<'_, Result<Option<String>, RawError>> {
        self.inner.current_user()
    }
    fn list_databases(&mut self, limit: usize) -> BoxFuture<'_, Result<Vec<String>, RawError>> {
        self.inner.list_databases(limit)
    }
    fn close(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.inner.close()
    }
    fn cancel_handle(&self) -> Option<Arc<dyn CancelHandle>> {
        self.inner.cancel_handle()
    }
    fn execute<'a>(
        &'a mut self,
        sql: &'a str,
        opts: ExecOpts,
    ) -> BoxFuture<'a, Result<ExecSummary, RawError>> {
        self.kill_peers(sql);
        self.inner.execute(sql, opts)
    }
    fn batch<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        self.kill_peers(sql);
        self.inner.batch(sql)
    }
    fn tx_status(&self) -> TxStatus {
        self.inner.tx_status()
    }
    fn reset(&mut self) -> BoxFuture<'_, Result<(), RawError>> {
        self.inner.reset()
    }
    fn backend_id(&self) -> Option<u64> {
        self.inner.backend_id()
    }
}

// ---------------------------------------------------------------------------------------------
// The brief's tests (Task 15 Step 1)

#[tokio::test]
async fn budget_of_four_queues_the_fifth_console_first() {
    let (sm, _) = manager4(fake_pg());
    let mut held = open_n(&sm, 4).await;
    let intro = tokio::spawn(sm.pooled(SessionKind::Service));
    let console = tokio::spawn(sm.open(console_spec(5), ix_none()));
    // One session ends (its only handle goes); the other three stay.
    drop(held.remove(0));
    // Fix round 1 (review I3, plan amended): a condition wait, not a 200 ms budget.
    let fifth = timeout(WAIT, console).await;
    assert!(fifth.is_ok());
    // Held, so its permit stays taken.
    let _fifth = fifth.unwrap().unwrap().expect("the fifth console opens");
    assert!(timeout(ms(50), intro).await.is_err());
}

#[tokio::test]
async fn pooled_release_resets_and_reuses() {
    let (sm, f) = manager4(fake_pg());
    {
        let s = sm.pooled(SessionKind::Service).await.unwrap();
        run(&s, "SET search_path = x").await.unwrap();
    }
    settle().await;
    let _s2 = sm.pooled(SessionKind::Service).await.unwrap();
    assert_eq!(f.connect_calls(), 1);
    assert_eq!(f.reset_calls(), 1);
}

#[tokio::test(start_paused = true)]
async fn pooled_idle_closes_after_60s() {
    let (sm, f) = manager4(fake_pg());
    drop(sm.pooled(SessionKind::Service).await.unwrap());
    tokio::time::advance(secs(61)).await;
    settle().await;
    assert_eq!(f.open_connections(), 0);
}

#[tokio::test]
async fn failed_reset_discards_and_frees_the_permit() {
    let (sm, f) = manager_n(fake_pg().reset_fails(), 1);
    drop(sm.pooled(SessionKind::Service).await.unwrap());
    settle().await;
    assert_eq!(f.open_connections(), 0);
    // Fix round 1 (review I3, plan amended): a condition wait, not a 200 ms budget.
    assert!(
        timeout(WAIT, sm.open(console_spec(1), ix_none()))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn single_session_mode_shares_one_connection() {
    let (sm, f) = manager_single(fake_pg());
    let a = sm.open(console_spec(1), ix_none()).await.unwrap();
    let b = sm.pooled(SessionKind::Service).await.unwrap();
    assert_eq!(a.id(), b.id());
    assert_eq!(f.connect_calls(), 1);
    drop(b);
    settle().await;
    assert_eq!(f.reset_calls(), 0);
}

#[tokio::test]
async fn single_session_pooled_drop_never_resets() {
    // Q19
    let (sm, f) = manager_single(fake_pg().exec(vec![tx("BEGIN", TxStatus::InTx)]));
    let console = sm.open(console_spec(1), ix_none()).await.unwrap();
    run(&console, "BEGIN").await.unwrap();
    drop(sm.pooled(SessionKind::Service).await.unwrap());
    settle().await;
    assert_eq!(f.reset_calls(), 0);
    assert_eq!(snap(&console).tx.status, TxStatus::InTx);
}

#[tokio::test]
#[ignore = "needs Task 14: the pipeline fills LiveConnection.profile_changes and secrets_used"]
async fn overlay_keeps_profile_changes_and_secrets_for_side_connections() {
    // Q17
    let (sm, f) = manager_over(
        fake_pg_with_pin_answer_and_never_retention(),
        pin_never_profile(),
    );
    let _s = sm
        .open(console_spec(1), ix_interactive_pin_and_pw("pw"))
        .await
        .unwrap();
    let o = sm.overlay().unwrap();
    assert_eq!(o.profile.tls.pins.len(), 1);
    assert!(o.secrets.db.is_some());
    sm.update_profile(Arc::new(renamed(&o.profile)));
    assert_eq!(sm.overlay().unwrap().profile.tls.pins.len(), 1);
    assert!(sm.overlay().unwrap().secrets.db.is_some());
    let _ = f;
}

#[tokio::test]
async fn sqlite_is_single_session() {
    let (sm, f) = manager_sqlite();
    let _a = sm.open(console_spec(1), ix_none()).await.unwrap();
    let _b = sm.open(console_spec(2), ix_none()).await.unwrap();
    assert_eq!(f.connect_calls(), 1);
}

#[tokio::test]
async fn read_only_spec_reaches_connect() {
    let (sm, f) = manager4(fake_pg());
    sm.open(
        SessionSpec {
            read_only: true,
            ..console_spec(1)
        },
        ix_none(),
    )
    .await
    .unwrap();
    assert!(f.last_request().unwrap().read_only);
}

#[tokio::test]
async fn cancel_side_connection_is_outside_budget() {
    let (sm, _) = manager_n(
        fake_mysql()
            .backend(5)
            .exec(vec![slow("select sleep", secs(60))]),
        1,
    );
    let s = sm.open(console_spec(1), ix_none()).await.unwrap();
    let q = s.submit(exec_slow("select sleep(60)"));
    settle().await;
    assert_eq!(
        timeout(secs(3), s.cancel(q.seq)).await.unwrap().unwrap(),
        CancelOutcome::Cancelled
    );
}

#[tokio::test]
async fn mcp_sessions_never_share_with_user_console() {
    let (sm, _) = manager4(fake_pg());
    let user = sm.open(console_spec(1), ix_none()).await.unwrap();
    let mcp = sm
        .open(
            SessionSpec {
                kind: SessionKind::Console(ClientId(77)),
                department: Department::Mcp,
                read_only: true,
            },
            ix_none(),
        )
        .await
        .unwrap();
    assert_ne!(user.id(), mcp.id());
}

// ---------------------------------------------------------------------------------------------
// The review probes and the edges of the brief's rules

#[tokio::test]
async fn a_read_only_request_is_never_served_by_a_read_write_pooled_session() {
    let (sm, f) = manager4(fake_pg());
    let rw_id = {
        let s = sm.pooled(SessionKind::Service).await.unwrap();
        assert!(!snap(&s).read_only);
        s.id()
    };
    parked(&f, 1).await;
    // The profile turns read-only (the app saved it): the idle read-write session is not
    // handed out for it.
    sm.update_profile(Arc::new(ro(pg_profile(TlsMode::Disable))));
    let s = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(s.id(), rw_id);
    assert!(snap(&s).read_only);
    assert!(f.last_request().unwrap().read_only);
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test]
async fn a_broken_pooled_session_is_never_handed_out_again() {
    let (sm, f) = manager4(fake_pg());
    let first = {
        let s = sm.pooled(SessionKind::Service).await.unwrap();
        s.id()
    };
    parked(&f, 1).await;
    f.kill_connections();
    let broken = {
        let s = sm.pooled(SessionKind::Service).await.unwrap();
        assert_eq!(s.id(), first, "the idle session is reused");
        assert!(run(&s, "SELECT 1").await.is_err());
        assert!(matches!(snap(&s).state, SessionState::Broken(_)));
        s.id()
    };
    settle_until("the broken session is forgotten", || sm.open_count() == 0).await;
    let s = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(s.id(), broken);
    assert_eq!(f.connect_calls(), 2);
    // The broken one was never reset (its connection is gone) and is closed.
    assert_eq!(f.reset_calls(), 1);
}

#[tokio::test]
async fn a_session_broken_while_idle_in_the_pool_is_discarded_at_borrow() {
    let (sm, f) = manager4(fake_pg());
    let idle = sm.pooled(SessionKind::Service).await.unwrap();
    let id = idle.id();
    // A clone outlives the borrow (Deref hands out the handle): close the session behind the
    // pool's back once it is idle.
    let behind = (*idle).clone();
    drop(idle);
    parked(&f, 1).await;
    behind.close(ClosePolicy::RollBack).await.unwrap();
    let s = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(s.id(), id);
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test]
async fn concurrent_borrowers_get_distinct_sessions_and_each_release_resets_once() {
    let (sm, f) = manager4(fake_pg());
    let a = sm.pooled(SessionKind::Service).await.unwrap();
    let b = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(a.id(), b.id());
    drop(a);
    drop(b);
    parked(&f, 2).await;
    assert_eq!(f.reset_calls(), 2);
    let c = sm.pooled(SessionKind::Service).await.unwrap();
    let d = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(c.id(), d.id());
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test]
async fn pools_are_kept_by_kind() {
    let (sm, f) = manager4(fake_pg());
    drop(sm.pooled(SessionKind::Service).await.unwrap());
    parked(&f, 1).await;
    let _intro = sm
        .pooled(SessionKind::Introspection { database: None })
        .await
        .unwrap();
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test]
async fn an_open_dropped_midway_frees_its_permit_and_its_connection() {
    let (sm, f) = manager_n(fake_pg().gated(), 1);
    let opening = tokio::spawn(sm.open(console_spec(1), ix_none()));
    eventually("the connect is in flight", || f.connect_calls() == 1).await;
    opening.abort();
    let _ = opening.await;
    f.release();
    let s = timeout(WAIT, sm.open(console_spec(2), ix_none()))
        .await
        .expect("the permit came back")
        .unwrap();
    eventually("the abandoned connection is gone", || {
        f.open_connections() == 1
    })
    .await;
    assert_eq!(sm.open_count(), 1);
    drop(s);
}

#[tokio::test]
async fn an_open_waiting_for_a_permit_gives_its_place_up_when_dropped() {
    let (sm, _) = manager_n(fake_pg(), 1);
    let first = sm.open(console_spec(1), ix_none()).await.unwrap();
    let waiting = tokio::spawn(sm.open(console_spec(2), ix_none()));
    settle().await;
    waiting.abort();
    let _ = waiting.await;
    drop(first);
    assert!(
        timeout(WAIT, sm.open(console_spec(3), ix_none()))
            .await
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn a_closed_console_frees_its_permit() {
    let (sm, f) = manager_n(fake_pg(), 1);
    let s = sm.open(console_spec(1), ix_none()).await.unwrap();
    let keep = s.clone();
    s.close(ClosePolicy::RollBack).await.unwrap();
    assert!(
        timeout(WAIT, sm.open(console_spec(2), ix_none()))
            .await
            .unwrap()
            .is_ok()
    );
    assert_eq!(f.connect_calls(), 2);
    drop(keep);
}

#[tokio::test]
async fn close_all_closes_every_session_and_drops_the_overlay() {
    let (sm, f) = manager4(fake_pg());
    let _console = sm.open(console_spec(1), ix_none()).await.unwrap();
    let returned = sm.pooled(SessionKind::Service).await.unwrap();
    let _borrowed = sm.pooled(SessionKind::Service).await.unwrap();
    drop(returned);
    parked(&f, 1).await;
    assert!(sm.overlay().is_some());
    assert_eq!(sm.open_count(), 3);
    sm.close_all().await;
    assert!(sm.overlay().is_none());
    eventually("every connection is closed", || f.open_connections() == 0).await;
    eventually("no session is counted", || sm.open_count() == 0).await;
    assert!(sm.default_session().is_none());
}

#[tokio::test]
async fn a_connect_that_ends_after_close_all_never_brings_the_overlay_back() {
    let (sm, f) = manager4(fake_pg().gated());
    let opening = tokio::spawn(sm.open(console_spec(1), ix_none()));
    eventually("the connect is in flight", || f.connect_calls() == 1).await;
    sm.close_all().await;
    f.release();
    let opened = timeout(WAIT, opening).await.unwrap().unwrap();
    assert!(opened.is_err());
    assert!(sm.overlay().is_none());
    eventually("the late connection is closed", || {
        f.open_connections() == 0
    })
    .await;
    assert_eq!(sm.open_count(), 0);
}

#[tokio::test]
async fn update_profile_keeps_the_overlay_and_reaches_later_connects() {
    let (sm, f) = manager4(fake_pg());
    let _s = sm.open(console_spec(1), ix_none()).await.unwrap();
    assert_eq!(sm.overlay().unwrap().profile.name, "orders");
    sm.update_profile(Arc::new(Profile {
        name: "renamed".into(),
        database: Some("analytics".into()),
        ..pg_profile(TlsMode::Disable)
    }));
    let o = sm.overlay().unwrap();
    assert_eq!(o.profile.name, "renamed");
    let _s2 = sm.open(console_spec(2), ix_none()).await.unwrap();
    assert_eq!(
        f.last_request().unwrap().database.as_deref(),
        Some("analytics")
    );
}

#[tokio::test]
async fn single_mode_default_session_is_reopened_once_it_is_broken() {
    let (sm, f) = manager_single(fake_pg());
    let a = sm.open(console_spec(1), ix_none()).await.unwrap();
    assert_eq!(sm.default_session().map(|d| d.id()), Some(a.id()));
    f.kill_connections();
    assert!(run(&a, "SELECT 1").await.is_err());
    let b = sm.open(console_spec(1), ix_none()).await.unwrap();
    assert_ne!(a.id(), b.id());
    assert_eq!(f.connect_calls(), 2);
}

#[tokio::test(start_paused = true)]
async fn single_mode_borrows_queue_behind_the_console() {
    // Fix round 1 (review I3): the console's statement lasts 60 s on a paused clock, so the
    // borrow's request provably waits behind it; no real-time window.
    let (sm, _) = manager_single(fake_pg().exec(vec![slow("SELECT pg_sleep", secs(60))]));
    let console = sm.open(console_spec(1), ix_none()).await.unwrap();
    let running = console.submit(exec("SELECT pg_sleep(60)"));
    settle_until(
        "the console's statement runs",
        || matches!(snap(&console).state, SessionState::Busy { req, .. } if req == running.seq),
    )
    .await;
    let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
    assert_eq!(borrowed.id(), console.id());
    let mut queued = borrowed.submit(exec("SELECT 1")).reply;
    settle().await;
    // FIFO: nothing can end the console's statement while the clock stands still.
    assert!(matches!(
        queued.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    running.reply.await.unwrap().unwrap();
    queued.await.unwrap().unwrap();
}

#[tokio::test]
async fn single_mode_never_shares_with_mcp() {
    let (sm, f) = manager_single(fake_pg());
    let user = sm.open(console_spec(1), ix_none()).await.unwrap();
    let mcp = sm
        .open(
            SessionSpec {
                kind: SessionKind::Console(ClientId(77)),
                department: Department::Mcp,
                read_only: true,
            },
            ix_none(),
        )
        .await
        .unwrap();
    assert_ne!(user.id(), mcp.id());
    assert!(snap(&mcp).read_only);
    assert_eq!(f.connect_calls(), 2);
}

// ---------------------------------------------------------------------------------------------
// Fix round 1: the review's scenarios (paused clock, FakeDriver; times are virtual)

/// C1 (H8b): a released session goes straight to the borrower of its kind that waits for one.
#[tokio::test(start_paused = true)]
async fn a_released_session_goes_to_a_waiting_borrower_of_its_kind() {
    let (sm, f) = manager_n(fake_pg(), 1);
    let a = sm.pooled(SessionKind::Service).await.unwrap();
    let id = a.id();
    let waiting = tokio::spawn(sm.pooled(SessionKind::Service));
    settle().await;
    let t0 = tokio::time::Instant::now();
    drop(a);
    let b = waiting.await.unwrap().unwrap();
    assert!(t0.elapsed() < secs(1), "waited {:?}", t0.elapsed());
    assert_eq!(b.id(), id);
    assert_eq!((f.connect_calls(), f.reset_calls()), (1, 1));
}

/// C1 (H8): a waiter of another kind gets the permit of a released session.
#[tokio::test(start_paused = true)]
async fn a_released_session_frees_its_permit_for_a_waiter_of_another_kind() {
    let (sm, f) = manager_n(fake_pg(), 2);
    let a = sm.pooled(SessionKind::Service).await.unwrap();
    let b = sm.pooled(SessionKind::Service).await.unwrap();
    let waiting = tokio::spawn(sm.pooled(SessionKind::Introspection { database: None }));
    settle().await;
    let t0 = tokio::time::Instant::now();
    drop(a);
    drop(b);
    let _intro = waiting.await.unwrap().unwrap();
    assert!(t0.elapsed() < secs(1), "waited {:?}", t0.elapsed());
    assert_eq!(f.connect_calls(), 3);
}

/// C1 (H1): idle sessions never starve a console, even when it queued while the borrow was
/// still out.
#[tokio::test(start_paused = true)]
async fn a_console_queued_while_a_borrow_is_out_gets_its_permit_at_release() {
    let (sm, _) = manager_n(fake_pg(), 2);
    let _first = sm.open(console_spec(1), ix_none()).await.unwrap();
    let borrow = sm.pooled(SessionKind::Service).await.unwrap();
    let waiting = tokio::spawn(sm.open(console_spec(2), ix_none()));
    settle().await;
    let t0 = tokio::time::Instant::now();
    drop(borrow);
    let _second = waiting.await.unwrap().unwrap();
    assert!(t0.elapsed() < secs(1), "waited {:?}", t0.elapsed());
}

/// C1: a console outranks a borrower waiting for the same released session.
#[tokio::test(start_paused = true)]
async fn a_waiting_console_outranks_a_waiting_borrower() {
    let (sm, _) = manager_n(fake_pg(), 1);
    let borrow = sm.pooled(SessionKind::Service).await.unwrap();
    let borrower = tokio::spawn(sm.pooled(SessionKind::Service));
    let console = tokio::spawn(sm.open(console_spec(1), ix_none()));
    settle().await;
    let t0 = tokio::time::Instant::now();
    drop(borrow);
    let console = console.await.unwrap().unwrap();
    assert!(t0.elapsed() < secs(1), "waited {:?}", t0.elapsed());
    assert!(!borrower.is_finished());
    drop(console);
    let t0 = tokio::time::Instant::now();
    let _b = borrower.await.unwrap().unwrap();
    assert!(t0.elapsed() < secs(1), "waited {:?}", t0.elapsed());
}

/// I1 (H3, case A): the profile moves to another server while the first connect is in flight;
/// the late connect is refused, and the overlay and the next connect follow the saved profile.
#[tokio::test(start_paused = true)]
async fn a_first_connect_in_flight_across_a_move_is_never_absorbed() {
    let (sm, f) = manager4(fake_pg().delay(secs(5)));
    let opening = tokio::spawn(sm.open(console_spec(1), ix_none()));
    settle_until("the connect is in flight", || f.connect_calls() == 1).await;
    sm.update_profile(Arc::new(moved(pg_profile(TlsMode::Disable))));
    assert!(opening.await.unwrap().is_err());
    assert!(sm.overlay().is_none());
    settle_until("the late connection is closed", || {
        f.open_connections() == 0
    })
    .await;
    assert_eq!(sm.open_count(), 0);
    let _s = sm.open(console_spec(2), ix_none()).await.unwrap();
    assert_eq!(last_host(&f), "10.9.9.9");
    assert_eq!(host_of(&sm.overlay().unwrap().profile), "10.9.9.9");
}

/// I1 (case B): with an overlay already there, a background connect to the old server that
/// lands after the move adds nothing to the new server's overlay.
#[tokio::test(start_paused = true)]
async fn a_background_connect_to_the_old_server_never_reaches_the_new_overlay() {
    let (sm, f) = manager4(fake_pg().delay(secs(5)));
    let _console = sm.open(console_spec(1), ix_none()).await.unwrap();
    assert_eq!(host_of(&sm.overlay().unwrap().profile), "127.0.0.1");
    let late = tokio::spawn(sm.pooled(SessionKind::Service));
    settle_until("the pooled connect is in flight", || f.connect_calls() == 2).await;
    sm.update_profile(Arc::new(moved(pg_profile(TlsMode::Disable))));
    assert!(late.await.unwrap().is_err());
    let o = sm.overlay().unwrap();
    assert_eq!(host_of(&o.profile), "10.9.9.9");
    assert!(o.secrets.db.is_none() && !o.plaintext_consent);
    settle_until("the late connection is closed", || {
        f.open_connections() == 1
    })
    .await;
    assert_eq!(sm.open_count(), 1);
}

/// I2 (H2): Single mode decides on the default session's own state: once the profile is
/// read-only, a read-only request never gets the read-write default.
#[tokio::test]
async fn single_mode_read_only_requests_never_get_a_read_write_default() {
    let (sm, f) = manager_single(fake_pg());
    let rw = sm.open(console_spec(1), ix_none()).await.unwrap();
    assert!(!snap(&rw).read_only);
    // A read-only request while the profile is still read-write.
    let ro_spec = SessionSpec {
        read_only: true,
        ..console_spec(2)
    };
    let own = sm.open(ro_spec.clone(), ix_none()).await.unwrap();
    assert_ne!(own.id(), rw.id());
    assert!(snap(&own).read_only);
    // The app saves the profile read-only.
    sm.update_profile(Arc::new(Profile {
        single_session: true,
        ..ro(pg_profile(TlsMode::Disable))
    }));
    let again = sm.open(ro_spec, ix_none()).await.unwrap();
    assert_ne!(again.id(), rw.id());
    assert!(snap(&again).read_only);
    let borrowed = sm.pooled(SessionKind::Service).await.unwrap();
    assert_ne!(borrowed.id(), rw.id());
    assert!(snap(&borrowed).read_only);
    assert_eq!(f.connect_calls(), 4);
}

/// M1 (H4): a borrow never becomes the default session.
#[tokio::test]
async fn a_borrow_of_the_default_console_kind_is_never_the_default_session() {
    let (sm, f) = manager4(fake_pg());
    let borrowed = sm
        .pooled(SessionKind::Console(ClientId::DEFAULT))
        .await
        .unwrap();
    assert!(sm.default_session().is_none());
    let id = borrowed.id();
    drop(borrowed);
    parked(&f, 1).await;
    let default = sm
        .open(
            SessionSpec {
                kind: SessionKind::Console(ClientId::DEFAULT),
                read_only: false,
                department: Department::Console,
            },
            ix_none(),
        )
        .await
        .unwrap();
    assert_ne!(default.id(), id);
    assert_eq!(sm.default_session().map(|d| d.id()), Some(default.id()));
}

/// M2: on SQLite (budget 1) a session that may not share the default is refused at once,
/// never left waiting forever.
#[tokio::test]
async fn sqlite_refuses_a_session_it_could_never_open() {
    let (sm, f) = manager_sqlite();
    let _default = sm.open(console_spec(1), ix_none()).await.unwrap();
    let mcp = SessionSpec {
        kind: SessionKind::Console(ClientId(77)),
        department: Department::Mcp,
        read_only: true,
    };
    let e = timeout(WAIT, sm.open(mcp, ix_none()))
        .await
        .expect("answered at once")
        .unwrap_err();
    assert_eq!(e.class, hacivat_proto::ErrClass::ServerTooManyConnections);
    let ro_spec = SessionSpec {
        read_only: true,
        ..console_spec(2)
    };
    assert!(
        timeout(WAIT, sm.open(ro_spec, ix_none()))
            .await
            .expect("answered at once")
            .is_err()
    );
    // The default still serves everyone else.
    let _b = timeout(WAIT, sm.pooled(SessionKind::Service))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.connect_calls(), 1);
}

// ---------------------------------------------------------------------------------------------
// The overlay test's fixtures (Task 14's pipeline: a certificate pinned at connect, a password
// with retention `Never` typed once).

fn pin_never_profile() -> Profile {
    Profile {
        auth: hacivat_secrets::AuthProviderId::Password,
        retention: hacivat_secrets::Retention::Never,
        ..pg_profile(TlsMode::VerifyFull)
    }
}

fn fake_pg_with_pin_answer_and_never_retention() -> FakeDriver {
    fake_pg()
        .presented(vec![fixture_ca_der()])
        .script(vec![Err(tls_unknown()), Ok(())])
}

fn renamed(p: &Profile) -> Profile {
    Profile {
        name: "renamed".into(),
        ..p.clone()
    }
}

/// A UI that pins the reviewed certificate and types `password`.
struct PinAndPassword(String);

impl hacivat_session::pipeline::UiPort for PinAndPassword {
    fn ask(
        &self,
        q: hacivat_session::pipeline::Question,
    ) -> BoxFuture<'static, hacivat_session::pipeline::Answer> {
        use hacivat_session::pipeline::{Answer, Question};
        let answer = match q {
            Question::Certificate { .. } => Answer::Pin,
            Question::Password { .. } => {
                Answer::Password(hacivat_secrets::SecretString::from(self.0.clone()))
            }
            _ => Answer::Cancelled,
        };
        Box::pin(async move { answer })
    }
}

fn ix_interactive_pin_and_pw(password: &str) -> hacivat_session::pipeline::Interaction {
    hacivat_session::pipeline::Interaction::Interactive(Arc::new(PinAndPassword(
        password.to_string(),
    )))
}

/// The DER of `tests/fixtures/test-ca.pem`.
fn fixture_ca_der() -> Vec<u8> {
    let pem = include_str!("fixtures/test-ca.pem");
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let bytes: Vec<u8> = body.bytes().filter(|c| *c != b'=').collect();
    let mut der = Vec::new();
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= u32::from(value(*c)) << (18 - 6 * i);
        }
        let out = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        der.extend_from_slice(&out[..chunk.len() - 1]);
    }
    der
}
