//! M2b conformance over Docker (Task 23; exit criteria E2, E3; Review Focus 1): cancel within
//! 1 s on every engine, directly and through SSH; a terminated backend with an open
//! transaction reports what was lost; an `INSERT` cut mid-flight reports "outcome unknown" and
//! is never replayed; a server outage reconnects within the backoff; carried cases: a read-only
//! session whose guard `BEGIN` is answered by a `FATAL` reconnects (T17 P1), an SSH chain's
//! death closes the parked pooled sessions (T17 R2), and finding F3: a released pooled session
//! is parked and reused, and the session's probes read the server's values. Run with `cargo
//! xtask docker test --set pr|full -p hacivat-session`.
//!
//! - Targets come only from `require_target` (Q6); through SSH a test dials the target's
//!   in-network endpoint.
//! - Real servers and sockets set the pace, so waits are on conditions bounded by real time
//!   (`ChaosProxy` cuts are the only sleeps that matter, and they are the scenario).
//! - `ro_fatal_on_the_guards_begin_reconnects_over_a_fake` needs no server: it runs in plain
//!   `cargo test`.

mod common;

use std::sync::Arc;
use std::time::Instant;

use common::conformance::*;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{ExecSummary, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_session::LiveState;
use hacivat_session::pipeline::Interaction;
use hacivat_session::session::{
    CancelOutcome, ExecErrorKind, Request, SessionKind, SessionManager, SessionState,
};
use hacivat_testkit::chaos::ChaosProxy;
use hacivat_testkit::require_target;

/// Every read-write session of the matrix, M2b manager (B18 presets, B19 reconnects).
fn rw(t: &hacivat_testkit::DockerTarget) -> hacivat_session::profile::Profile {
    profile_for(t, auto_or_require(t))
}

// ---------------------------------------------------------------------------------------------
// Cancel (E3)

/// Waits (a condition, 20 ms steps, 10 s at most) until request `seq` has been running on `s`
/// for `d`, by the session's own `Busy { since }`: the scenario's time (MySQL pre-opens its
/// `KILL QUERY` side connection 500 ms into a statement, Q37), never a guess.
async fn running_for(
    s: &hacivat_session::session::SessionHandle,
    seq: u64,
    d: std::time::Duration,
    what: &str,
) {
    let deadline = Instant::now() + secs(10);
    loop {
        if let SessionState::Busy { req, since } = s.state().borrow().state.clone()
            && req == seq
            && since.elapsed() >= d
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: request {seq} is not running: {:?}",
            s.state().borrow().state
        );
        tokio::time::sleep(ms(20)).await;
    }
}

/// Cancels `sql` (a long sleep) on `s` once it has run for `after`: `Cancelled` within 1 s, the
/// request ends `Cancelled`, and the session runs the next statement.
async fn cancels_within_a_second(
    s: &hacivat_session::session::SessionHandle,
    sql: &str,
    after: std::time::Duration,
    what: &str,
) {
    let q = s.submit(user_exec(sql));
    running_for(s, q.seq, after, what).await;
    let start = Instant::now();
    let outcome = q.cancel(s).await.expect("the cancel answers");
    let took = start.elapsed();
    if outcome != CancelOutcome::Cancelled {
        let state = s.state().borrow().state.clone();
        let reply = tokio::time::timeout(secs(5), q.reply).await.map(|r| {
            r.map(|r| {
                r.map(|summary| summary.row_count)
                    .map_err(|e| (e.kind, code_of(e.server.as_ref())))
            })
        });
        panic!("{what}: the cancel answered {outcome:?} after {took:?}; {state:?}; {reply:?}");
    }
    assert!(took < secs(1), "{what}: the cancel took {took:?}");
    let e = q
        .reply
        .await
        .expect("the actor replies")
        .expect_err("the statement was cancelled");
    assert_eq!(e.kind, ExecErrorKind::Cancelled, "{what}");
    let next = run(s, "SELECT 1").await;
    assert!(next.is_ok(), "{what}: {:?}", next.map_err(|e| e.kind));
}

/// SQLite (no server): an endless recursive CTE is interrupted within 1 s.
async fn assert_sqlite_long_cte_interrupted() {
    let dir = tempfile::tempdir().expect("a temp directory");
    let on = OnManager::connect(deps(), sqlite_profile(&dir.path().join("db.sqlite"))).await;
    cancels_within_a_second(
        &on.s,
        "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) FROM c",
        ms(300),
        "sqlite",
    )
    .await;
    on.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn cancel_matrix() {
    if !docker_enabled() {
        return;
    }
    // MySQL and MariaDB answer a `KILL QUERY` of a statement that is nothing but `SLEEP()`
    // with its result (1) and no error, so the session rightly reports the request finished,
    // not cancelled (`sleep_only_ends_at_once` below). A `SLEEP` inside a query over a table
    // is answered `1317` (interrupted), which the driver reports as a cancel.
    const MYSQL_SLEEP: &str = "SELECT 1 FROM information_schema.schemata WHERE SLEEP(60) = 0";
    for (id, sql) in [
        ("pg17", "SELECT pg_sleep(60)"),
        ("mysql84", MYSQL_SLEEP),
        ("mariadb114", MYSQL_SLEEP),
    ] {
        let Some(t) = require_target(id) else {
            continue;
        };
        for path in [Path::Direct, Path::Ssh("sshd-a")] {
            if !path.available() {
                continue;
            }
            let on = session_on(&t, path).await;
            let what = format!("{} {path:?}", t.id);
            // 600 ms: past the 500 ms pre-warm of MySQL's `KILL QUERY` side connection (Q37).
            cancels_within_a_second(&on.s, sql, ms(600), &what).await;
            if t.engine != hacivat_proto::DbmsKind::Postgres {
                sleep_only_ends_at_once(&on.s, &what).await;
            }
            on.shutdown().await;
        }
    }
    assert_sqlite_long_cte_interrupted().await;
}

/// MySQL's `SELECT SLEEP(60)` alone: the `KILL QUERY` ends it within 1 s too, and the server
/// answers it with its result (1, no error), so the request finished (`AlreadyFinished`) with
/// that answer; the session runs the next statement.
async fn sleep_only_ends_at_once(s: &hacivat_session::session::SessionHandle, what: &str) {
    let q = s.submit(user_exec("SELECT SLEEP(60)"));
    running_for(s, q.seq, ms(600), what).await;
    let start = Instant::now();
    let outcome = q.cancel(s).await.expect("the cancel answers");
    let reply = tokio::time::timeout(secs(1), q.reply)
        .await
        .unwrap_or_else(|_| panic!("{what}: SLEEP(60) still runs 1 s after the cancel"))
        .expect("the actor replies");
    assert!(start.elapsed() < secs(1), "{what}: {:?}", start.elapsed());
    match reply {
        Ok(summary) => {
            assert_eq!(outcome, CancelOutcome::AlreadyFinished, "{what}");
            assert_eq!(summary.row_count, 1, "{what}");
        }
        Err(e) => {
            assert_eq!(e.kind, ExecErrorKind::Cancelled, "{what}");
            assert_eq!(outcome, CancelOutcome::Cancelled, "{what}");
        }
    }
    let next = run(s, "SELECT 1").await;
    assert!(next.is_ok(), "{what}: {:?}", next.map_err(|e| e.kind));
}

// ---------------------------------------------------------------------------------------------
// Reconnect (E2)

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn terminated_backend_with_open_tx_reports_lost_tx() {
    if !docker_enabled() {
        return;
    }
    for t in ["pg17", "mysql84", "mariadb114"]
        .into_iter()
        .filter_map(require_target)
    {
        // Directly, and for the `pr` lines also through `sshd-a` (alg/01 §5).
        for path in [Path::Direct, Path::Ssh("sshd-a")] {
            if path != Path::Direct && (!in_pr_set(&t) || !path.available()) {
                continue;
            }
            let what = format!("{} {path:?}", t.id);
            // A 1 s keep-alive: the liveness supervisor finds the loss with no statement sent.
            let mut p = profile_on(&t, path, mode_on(&t, path)).expect("checked above");
            p.keepalive_s = Some(1);
            let on = OnManager::connect(matrix_deps(), p).await;
            let mut ev = on.m.events();
            let backend = backend_id(&on.s, t.engine).await;
            for sql in ["BEGIN", "CREATE TEMPORARY TABLE hv_tmp (a int)"] {
                let reply = on.s.submit(simple_exec(sql)).reply.await.unwrap();
                assert!(
                    reply.is_ok(),
                    "{what}: {sql}: {:?}",
                    reply.map_err(|e| e.kind)
                );
            }
            kill_backend(&t, &backend).await;
            let lost = wait_session_lost(&mut ev).await;
            assert!(lost.open_tx && lost.temp_objects, "{what}: {lost:?}");
            // Task 17 ruling C1: a loss inside a transaction is never replayed. The profile is
            // `Lost` (Retry), not silently reconnected. (The brief's `Connected` predates C1.)
            assert!(
                matches!(on.m.state(on.id()), LiveState::Lost { .. }),
                "{what}: {:?}",
                on.m.state(on.id())
            );
            // Retry: a new connection, without the old one's temporary table.
            connect_on(&on.m, &on.profile).await;
            let s = on.session();
            assert!(run(&s, "SELECT 1").await.is_ok(), "{what}");
            let gone = run(&s, "SELECT * FROM hv_tmp").await;
            assert!(
                matches!(&gone, Err(e) if e.kind == ExecErrorKind::Db),
                "{what}: {:?}",
                gone.map_err(|e| e.kind)
            );
            on.shutdown().await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn insert_killed_mid_flight_reports_outcome_unknown_without_duplicate() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("mysql84") else {
        return;
    };
    // A table of this test's own (Q7; the matrix volumes keep no trace of it).
    let mut a = admin(&t).await;
    batch(&mut a, &t.id, "DROP TABLE IF EXISTS hv_t23_insert").await;
    batch(
        &mut a,
        &t.id,
        "CREATE TABLE hv_t23_insert (id int AUTO_INCREMENT PRIMARY KEY, v int)",
    )
    .await;
    let chaos = chaos_for(&t).await;
    let on = OnManager::connect(deps(), profile_via(&t, chaos.addr)).await;
    let q = on.s.submit(user_exec(
        "INSERT INTO hv_t23_insert (v) SELECT SLEEP(1) + 1",
    ));
    // Into the statement's second on the server; the assertions hold wherever the cut lands.
    running_for(&on.s, q.seq, ms(300), "mysql84 INSERT").await;
    chaos.cut(secs(2));
    let e = q
        .reply
        .await
        .expect("the actor replies")
        .expect_err("the connection was cut");
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: true
        }
    );
    // The session was idle around it (autocommit): it reconnects once the cut ends.
    wait_connected_within(&on.m, on.id(), CONNECT_LIMIT).await;
    assert!(run(&on.session(), "SELECT 1").await.is_ok());
    // The server finished (or dropped) the statement while the client was away; nothing ran
    // it again.
    let rows = count(&mut a, "SELECT count(*) FROM hv_t23_insert").await;
    assert!(rows <= 1, "the INSERT ran {rows} times");
    on.shutdown().await;
    batch(&mut a, &t.id, "DROP TABLE hv_t23_insert").await;
    a.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn server_outage_reconnects_within_backoff() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    let chaos = chaos_for(&t).await;
    let mut p = profile_via(&t, chaos.addr);
    p.keepalive_s = Some(1);
    let on = OnManager::connect(deps(), p).await;
    let mut ev = on.m.events();
    let cut = Instant::now();
    chaos.cut(secs(20));
    wait_state_of(&on.m, on.id(), secs(15), |s| {
        matches!(s, LiveState::Reconnecting { .. })
    })
    .await;
    // B19: attempts at 0, 1, 2, 4 and 8 s after the loss, then every 30 s, each ±20 %. A 20 s
    // outage ends between the fifth attempt (≤ 18 s) and the sixth (≤ 54 s after the loss),
    // so 75 s bounds it. (The brief's 45 s is shorter than the schedule's sixth attempt.)
    wait_connected_within(&on.m, on.id(), secs(75)).await;
    assert!(cut.elapsed() >= secs(20), "connected during the outage");
    let lost = wait_session_lost(&mut ev).await;
    assert!(!lost.open_tx, "{lost:?}");
    assert!(run(&on.session(), "SELECT 1").await.is_ok());
    on.shutdown().await;
}

// ---------------------------------------------------------------------------------------------
// Carried: a read-only session's guard `BEGIN` answered by FATAL 57P01 (T17 P1)

#[tokio::test(start_paused = true)]
async fn ro_fatal_on_the_guards_begin_reconnects_over_a_fake() {
    // The server ends the connection (`FATAL 57P01`) as the answer to the guard's first
    // statement: a lost connection (`server.shutdown`, a recovery case), so the idle read-only
    // session reconnects, as a read-write one does (T17 I6). Before: `Lost { internal }`.
    const BEGIN: &str = "BEGIN ISOLATION LEVEL";
    // The guard's answers after the reconnect: its transaction, a NULL canary, the rollback.
    let canary = FakeExec {
        prefix: "SELECT pg_catalog.txid_current_if_assigned()",
        result: Ok(ExecSummary {
            rows: vec![vec![None]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    };
    let fake = FakeDriver::new(DbmsKind::Postgres).exec(vec![
        common::tx_err(BEGIN, "57P01", TxStatus::Failed),
        common::tx(BEGIN, TxStatus::InTx),
        canary,
        common::tx("ROLLBACK", TxStatus::Idle),
    ]);
    let (m, fake) = common::manager_with(fake, hacivat_session::ManagerConfig::m2b());
    let p = Arc::new(common::ro(common::pg_profile(TlsMode::Disable)));
    m.connect(p.clone(), None, Interaction::Headless);
    common::wait_connected(&m, p.id).await;
    let e = run(&default_session(&m, p.id), "SELECT 1")
        .await
        .expect_err("the server ended the connection");
    // The user's statement never ran.
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false
        }
    );
    assert_eq!(code_of(e.server.as_ref()).as_deref(), Some("57P01"));
    wait_connected_within(&m, p.id, secs(30)).await;
    assert_eq!(fake.connect_calls(), 2);
    let again = run(&default_session(&m, p.id), "SELECT 1").await;
    assert!(again.is_ok(), "{:?}", again.map_err(|e| e.kind));
    m.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn ro_fatal_on_the_guards_begin_reconnects() {
    if !docker_enabled() {
        return;
    }
    let Some(t) = require_target("pg17") else {
        return;
    };
    // The proxy holds the server's FATAL until the guard's `BEGIN` goes out, so the FATAL is
    // that statement's answer (an idle client would see the closed socket instead).
    let proxy = HoldProxy::start(published(&t)).await;
    let app = unique("hv_t23_p1");
    let mut p = profile_via(&t, proxy.addr);
    p.read_only = true;
    p.application_name = app.clone();
    // No keep-alive: a ping during the hold would take the FATAL meant for the `BEGIN` (m8).
    p.keepalive_s = Some(0);
    let mut a = admin(&t).await;
    // The write below must stay refused; a table an escaped one left behind goes first and last.
    batch(&mut a, &t.id, "DROP TABLE IF EXISTS hv_t23_p1").await;
    let on = OnManager::connect(deps(), p).await;
    let mut ev = on.m.events();
    assert!(run(&on.s, "SELECT 1").await.is_ok());
    proxy.hold();
    batch(
        &mut a,
        &t.id,
        &format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = '{app}'"
        ),
    )
    .await;
    wait_until("the server's FATAL to be held", secs(10), || {
        proxy.server_ended_while_held()
    })
    .await;
    let e = run(&on.s, "SELECT 1")
        .await
        .expect_err("the server ended the connection");
    assert_eq!(
        e.kind,
        ExecErrorKind::ConnectionLost {
            outcome_unknown: false
        }
    );
    assert_eq!(code_of(e.server.as_ref()).as_deref(), Some("57P01"));
    // A recovery case: the idle read-only session reconnects (never `Lost { internal }`).
    wait_connected_within(&on.m, on.id(), CONNECT_LIMIT).await;
    let lost = wait_session_lost(&mut ev).await;
    assert!(!lost.open_tx, "{lost:?}");
    let s = on.session();
    assert!(run(&s, "SELECT 1").await.is_ok());
    // Still read-only.
    let write = run(&s, "CREATE TABLE hv_t23_p1 (a int)").await;
    on.shutdown().await;
    batch(&mut a, &t.id, "DROP TABLE IF EXISTS hv_t23_p1").await;
    a.close().await;
    assert!(
        matches!(&write, Err(e) if e.kind == ExecErrorKind::Db),
        "{:?}",
        write.map_err(|e| e.kind)
    );
}

// ---------------------------------------------------------------------------------------------
// Finding F3: a released pooled session is parked, and the session's own probes read values

/// F3 (the reset, T7; the pool, T15): a released pooled session is reset and parked, and the
/// next borrow of its kind gets the same session back. On PostgreSQL the reset's verify probe
/// (`SELECT current_user = session_user`) must read its `t`, which only the values path
/// returns: on the count-only extended protocol every release broke the session and every
/// borrow opened a new connection. MySQL has no verify probe.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn a_released_pooled_session_is_parked_and_reused() {
    if !docker_enabled() {
        return;
    }
    for t in ["pg17", "mysql84"].into_iter().filter_map(require_target) {
        let on = session_on(&t, Path::Direct).await;
        let sessions = on.m.sessions(on.id()).expect("connected");
        let kind = SessionKind::Introspection { database: None };
        let first = sessions
            .pooled(kind.clone())
            .await
            .expect("a pooled session");
        assert!(run(&first, "SELECT 1").await.is_ok(), "{}", t.id);
        let released = (*first).clone();
        assert_eq!(sessions.parked_count(), 0, "{}", t.id);
        drop(first);
        // Parked: reset, never broken and closed.
        wait_parked(&sessions, 1, &released, &t.id).await;
        let again = sessions.pooled(kind).await.expect("a pooled session");
        assert_eq!(
            again.id(),
            released.id(),
            "{}: the released session is reused",
            t.id
        );
        let reply = run(&again, "SELECT 1").await;
        assert!(reply.is_ok(), "{}: {:?}", t.id, reply.map_err(|e| e.kind));
        drop(again);
        on.shutdown().await;
    }
}

/// F3's sibling (HV-SESS-07): the schema probe reads values too, so after `SwitchSchema` the
/// session knows the server's database, schema and user (on the count-only path it read no
/// row and forgot the context instead).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn the_schema_probe_reads_the_servers_answer() {
    if !docker_enabled() {
        return;
    }
    for t in ["pg17", "mysql84"].into_iter().filter_map(require_target) {
        let on = session_on(&t, Path::Direct).await;
        let s = on.session();
        let name = match t.engine {
            DbmsKind::Postgres => "public",
            _ => t.database.as_str(),
        };
        let reply = s
            .submit(Request::SwitchSchema(name.to_string()))
            .reply
            .await
            .expect("the actor replies");
        assert!(reply.is_ok(), "{}: {:?}", t.id, reply.map_err(|e| e.kind));
        let ctx = s
            .state()
            .borrow()
            .schema
            .clone()
            .unwrap_or_else(|| panic!("{}: the probe read no context", t.id));
        assert_eq!(
            ctx.database.as_deref(),
            Some(t.database.as_str()),
            "{}",
            t.id
        );
        assert_eq!(ctx.schema.as_deref(), Some(name), "{}", t.id);
        // MySQL's CURRENT_USER() is `user@host`.
        assert!(
            ctx.user.as_deref().is_some_and(|u| u.starts_with(&t.user)),
            "{}: {:?}",
            t.id,
            ctx.user
        );
        if t.engine == DbmsKind::Postgres {
            assert_eq!(ctx.search_path_setting, [name], "{}", t.id);
        }
        on.shutdown().await;
    }
}

// ---------------------------------------------------------------------------------------------
// Carried: an SSH chain's death closes the parked pooled sessions (T17 R2, review M6)

/// Waits (a condition, 50 ms steps, 10 s at most) until the pool holds `parked` released
/// sessions: a release resets its session, then parks it (review I1: no fixed sleep). `s` is
/// the released session, whose state the failure message shows.
async fn wait_parked(
    sessions: &SessionManager,
    parked: usize,
    s: &hacivat_session::session::SessionHandle,
    what: &str,
) {
    let deadline = Instant::now() + secs(10);
    while sessions.parked_count() != parked {
        assert!(
            Instant::now() < deadline,
            "{what}: the released session was not parked (pool {}, session {:?})",
            sessions.parked_count(),
            s.state().borrow().state
        );
        tokio::time::sleep(ms(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn ssh_death_closes_parked_pooled_sessions() {
    if !docker_enabled() {
        return;
    }
    let Some(bastion) = require_ssh("sshd-a") else {
        return;
    };
    // MySQL reaches the chain through the loopback forwarder, and an idle MySQL connection does
    // not notice the chain's death by itself, so only the manager's `close_idle` closes its
    // parked session. PostgreSQL runs on the channel's stream (its leg joined with the F3 fix:
    // before it, a released PostgreSQL session was never parked).
    for t in ["mysql84", "pg17"].into_iter().filter_map(require_target) {
        // The bastion behind a chaos proxy, so the test kills exactly its own SSH session.
        let proxy = ChaosProxy::start(
            format!("{}:{}", bastion.host, bastion.port)
                .parse()
                .expect("sshd-a is published on an IP literal"),
        )
        .await;
        let dir = tempfile::tempdir().expect("a temp directory");
        let known = known_hosts_file(
            dir.path(),
            &[(proxy.addr.ip().to_string(), proxy.addr.port(), "sshd-a")],
        );
        let via = hacivat_testkit::SshTarget {
            host: proxy.addr.ip().to_string(),
            port: proxy.addr.port(),
            ..bastion.clone()
        };
        let p = through(&t, rw(&t), vec![hop(&via, key_auth())]);
        let on = OnManager::connect(ssh_deps(known), p).await;
        let sessions = on.m.sessions(on.id()).expect("connected");
        let kind = SessionKind::Introspection { database: None };
        // A pooled session runs, then goes back to the pool: reset, then parked, idle, over the
        // chain. Borrowed again it is the same session (the pool parks it), and back again.
        let pooled = sessions
            .pooled(kind.clone())
            .await
            .expect("a pooled session");
        assert!(run(&pooled, "SELECT 1").await.is_ok(), "{}", t.id);
        let parked = (*pooled).clone();
        drop(pooled);
        wait_parked(&sessions, 1, &parked, &t.id).await;
        let again = sessions
            .pooled(kind.clone())
            .await
            .expect("a pooled session");
        assert_eq!(again.id(), parked.id(), "{}: the pool parks", t.id);
        assert_eq!(sessions.parked_count(), 0, "{}", t.id);
        drop(again);
        wait_parked(&sessions, 1, &parked, &t.id).await;
        assert_eq!(sessions.open_count(), 2, "{}", t.id);
        proxy.cut(secs(3));
        // The chain died: the parked session is closed (never handed out again) and the
        // default session reconnects over a new chain.
        wait_until(
            &format!("{}: the parked session to close", t.id),
            secs(20),
            || {
                matches!(
                    parked.state().borrow().state,
                    SessionState::Disconnected | SessionState::Broken(_)
                )
            },
        )
        .await;
        wait_state_of(&on.m, on.id(), secs(20), |s| {
            !matches!(s, LiveState::Connected(_))
        })
        .await;
        wait_connected_within(&on.m, on.id(), CONNECT_LIMIT).await;
        let fresh = sessions.pooled(kind).await.expect("a pooled session");
        assert_ne!(fresh.id(), parked.id(), "{}", t.id);
        let reply = run(&fresh, "SELECT 1").await;
        assert!(reply.is_ok(), "{}: {:?}", t.id, reply.map_err(|e| e.kind));
        drop(fresh);
        on.shutdown().await;
    }
}
