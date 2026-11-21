//! M3b Task 12: the M2 carries of a session. C3 (the profile's startup script after connect
//! and after every reset, read-write only), C6 (a Manual MySQL session follows the server's
//! `autocommit` to Auto), C10 (a MySQL session keeps one warm side connection), C11 (the
//! settings' time zone on PostgreSQL, in the plan and its replay), the final transaction state
//! a closed or broken session publishes, and the notices a driver dropped past its own cap.
//! FakeDriver and a paused clock: no socket, no real time; every wait is on a condition.

mod common;

use std::sync::Arc;

use common::runs::*;
use common::runs_m3b::*;
use common::*;
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery};
use hacivat_driver::{ExecSummary, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::{DbmsKind, ErrClass};
use hacivat_results::FetchPolicy;
use hacivat_session::pipeline::Interaction;
use hacivat_session::profile::Profile;
use hacivat_session::session::env::{build_env_plan_with, startup_statements};
use hacivat_session::session::{
    CancelOutcome, ClosePolicy, EnvStep, Escalation, Request, SessionEvent, SessionHandle,
    SessionState, TxMode, TxOwner, schema_probe_sql,
};
use hacivat_session::{ConnectionManager, LiveState, ManagerConfig};
use hacivat_settings::TimeZonePref;

const SCRIPT: &str = "SET search_path = app; SET work_mem = '64MB';";

/// The test profile of `kind` with `script` as its startup script.
fn profile_script(kind: DbmsKind, script: &str) -> Profile {
    let base = match kind {
        DbmsKind::MySql => mysql_profile(TlsMode::Disable),
        _ => pg_profile(TlsMode::Disable),
    };
    Profile {
        startup_script: Some(script.to_string()),
        ..base
    }
}

/// Waits (letting the paused clock run, 10 ms steps) until `done`; panics after [`WAIT`] of
/// real time.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !done() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(ms(10)).await;
    }
}

/// What `f` was sent, without the session's schema probes.
fn sent(f: &FakeDriver) -> Vec<String> {
    let probes = [
        schema_probe_sql(DbmsKind::Postgres),
        schema_probe_sql(DbmsKind::MySql),
    ];
    f.executed()
        .into_iter()
        .filter(|s| !probes.contains(&s.as_str()))
        .collect()
}

/// How many times `sql` was sent.
fn count(f: &FakeDriver, sql: &str) -> usize {
    f.executed().iter().filter(|s| *s == sql).count()
}

/// A read-write session of `profile` over `fake`, once its after-connect statements ran (a
/// first request queues behind them).
async fn session_with_profile(
    fake: FakeDriver,
    profile: Profile,
) -> (SessionHandle, Arc<FakeDriver>) {
    let side = Arc::new(FakeDriver::new(profile.engine));
    let (h, f) = spawn_over(fake, profile, false, side).await;
    // The actor sends its after-connect statements before it reads any request.
    let _ = h.submit(Request::Ping).reply.await;
    (h, f)
}

/// Resets `h` and expects it to succeed.
async fn reset(h: &SessionHandle) {
    let reply = h
        .submit(Request::Reset)
        .reply
        .await
        .expect("the actor replies");
    assert!(reply.is_ok(), "{reply:?}");
}

/// A PostgreSQL fake that answers the read-only guard `rounds` times (its transaction, a
/// clean canary, its rollback).
fn pg_ro_clean(rounds: usize) -> FakeDriver {
    let null_canary = FakeExec {
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
    let mut script = Vec::new();
    for _ in 0..rounds {
        script.push(tx("BEGIN ISOLATION LEVEL", TxStatus::InTx));
        script.push(null_canary.clone());
        script.push(tx("ROLLBACK", TxStatus::Idle));
    }
    fake_pg().exec(script)
}

/// A PostgreSQL fake whose reset verify probe answers `t` (as often as asked).
fn pg_resettable(resets: usize) -> FakeDriver {
    fake_pg().exec(
        (0..resets)
            .map(|_| rows("SELECT current_user = session_user", &[&["t"]]))
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// C3: startup scripts

#[tokio::test(start_paused = true)]
async fn startup_script_runs_after_connect_and_reset_on_read_write() {
    let (h, f) =
        session_with_profile(pg_resettable(1), profile_script(DbmsKind::Postgres, SCRIPT)).await;
    assert!(f.executed().contains(&"SET search_path = app".into()));
    assert_eq!(count(&f, "SET work_mem = '64MB'"), 1);
    // In the script's order, nothing else before it on a fresh session (Q27); then the
    // schema is probed again, so the session shows what the script set (review I3b).
    assert_eq!(
        f.executed(),
        [
            "SET search_path = app",
            "SET work_mem = '64MB'",
            schema_probe_sql(DbmsKind::Postgres)
        ]
    );
    reset(&h).await;
    assert_eq!(count(&f, "SET work_mem = '64MB'"), 2);
    assert_eq!(count(&f, "SET search_path = app"), 2);
    // Last of the reset's statements (no schema is known to restore), then the probe.
    let executed = sent(&f);
    assert_eq!(
        executed.last().map(String::as_str),
        Some("SET work_mem = '64MB'")
    );
    assert_eq!(
        f.executed().last().map(String::as_str),
        Some(schema_probe_sql(DbmsKind::Postgres))
    );
    assert_eq!(snap(&h).state, SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn startup_script_is_skipped_on_read_only_with_a_notice() {
    let logs = capture_tracing(|| async {
        let profile = ro(profile_script(DbmsKind::Postgres, SCRIPT));
        let (h, f) = ro_session(pg_ro_clean(1), profile).await;
        // A guarded statement replays the environment after its reset: still no script.
        let reply = run(&h, "SELECT 1").await;
        assert!(reply.is_ok(), "{reply:?}");
        assert!(
            !f.executed()
                .iter()
                .any(|s| s.contains("work_mem") || s.contains("search_path = app")),
            "{:?}",
            f.executed()
        );
        assert_eq!(snap(&h).state, SessionState::Idle);
    })
    .await;
    assert_eq!(
        logs.matches("exec-startup-script-skipped-read-only")
            .count(),
        1,
        "one notice: {logs}"
    );
    // The plan of a read-only session never holds it.
    let plan = build_env_plan_with(
        DbmsKind::Postgres,
        &ro(profile_script(DbmsKind::Postgres, SCRIPT)),
        true,
        None,
        None,
    );
    assert!(
        !plan
            .steps
            .iter()
            .any(|s| matches!(s, EnvStep::StartupScript(_)))
    );
}

#[tokio::test(start_paused = true)]
async fn an_uncertain_boundary_refuses_the_whole_script_and_breaks_the_session() {
    // A dollar quote that never ends: the boundary after it is uncertain.
    let script = "SET search_path = app; DO $$ BEGIN PERFORM 1; END";
    assert!(startup_statements(DbmsKind::Postgres, script).is_err());
    let side = Arc::new(fake_pg());
    let (h, f) = spawn_over(
        fake_pg(),
        profile_script(DbmsKind::Postgres, script),
        false,
        side,
    )
    .await;
    let mut state = h.state();
    tokio::time::timeout(WAIT, state.wait_for(|s| s.state != SessionState::Idle))
        .await
        .expect("the session ends")
        .expect("the session publishes");
    assert_eq!(
        snap(&h).state,
        SessionState::Broken(ErrClass::ConfigInvalid)
    );
    assert!(
        f.executed().is_empty(),
        "none of it is sent: {:?}",
        f.executed()
    );
    // Nothing runs on it.
    assert!(run(&h, "SELECT 1").await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_failing_script_breaks_the_session_and_its_text_is_never_logged() {
    let script = "SET app.token = 's3cret_hv'; SET work_mem = '64MB'";
    let logs = capture_tracing(|| async {
        let fake = fake_pg().exec(vec![tx_err("SET app.token", "42704", TxStatus::Idle)]);
        let side = Arc::new(fake_pg());
        let (h, f) = spawn_over(
            fake,
            profile_script(DbmsKind::Postgres, script),
            false,
            side,
        )
        .await;
        let mut state = h.state();
        tokio::time::timeout(WAIT, state.wait_for(|s| s.state != SessionState::Idle))
            .await
            .expect("the session ends")
            .expect("the session publishes");
        assert_eq!(
            snap(&h).state,
            SessionState::Broken(ErrClass::ConfigInvalid)
        );
        // The script stops at its failure.
        assert_eq!(count(&f, "SET work_mem = '64MB'"), 0);
    })
    .await;
    assert!(
        logs.contains("the session environment failed after connect"),
        "{logs}"
    );
    assert!(
        !logs.contains("s3cret_hv"),
        "the script's text is never logged"
    );
}

// ---------------------------------------------------------------------------------------------
// C6: MySQL autocommit in Manual mode

#[tokio::test(start_paused = true)]
async fn mysql_autocommit_on_in_manual_switches_to_auto() {
    let profile = Profile {
        tx_mode: Some(TxMode::Manual),
        ..mysql_profile(TlsMode::Disable)
    };
    let fake = fake_mysql().autocommit_flag(vec![
        ("SET autocommit = 0", false),
        ("SET autocommit = 1", true),
    ]);
    let (h, f) = session_with_profile(fake, profile).await;
    assert_eq!(snap(&h).tx.mode, TxMode::Manual);
    assert_eq!(f.executed(), ["SET autocommit = 0"]);
    let mut ev = h.events();
    let logs = capture_tracing(|| async {
        let reply = run(&h, "SET autocommit = 1").await;
        assert!(reply.is_ok(), "{reply:?}");
    })
    .await;
    assert_eq!(snap(&h).tx.mode, TxMode::Auto);
    let changed = next(&mut ev).await;
    assert!(
        matches!(&changed, SessionEvent::TxChanged(tx) if tx.mode == TxMode::Auto),
        "{changed:?}"
    );
    assert_eq!(
        logs.matches("the server turned autocommit on").count(),
        1,
        "one log line: {logs}"
    );
    // Nothing turns autocommit off again: not a reset, not a later statement.
    reset(&h).await;
    assert_eq!(snap(&h).tx.mode, TxMode::Auto);
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(count(&f, "SET autocommit = 0"), 1, "{:?}", f.executed());
}

#[tokio::test(start_paused = true)]
async fn a_manual_mysql_session_with_autocommit_off_stays_manual() {
    let profile = Profile {
        tx_mode: Some(TxMode::Manual),
        ..mysql_profile(TlsMode::Disable)
    };
    let fake = fake_mysql().autocommit_flag(vec![("SET autocommit = 0", false)]);
    let (h, _f) = session_with_profile(fake, profile).await;
    run(&h, "UPDATE t SET a = 1").await.unwrap();
    assert_eq!(snap(&h).tx.mode, TxMode::Manual);
}

// ---------------------------------------------------------------------------------------------
// C10: the warm MySQL side connection

#[tokio::test(start_paused = true)]
async fn mysql_side_connection_stays_warm_for_30s() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().exec(vec![
            slow("SELECT 1", ms(100)),
            slow("SELECT SLEEP(2)", secs(2)),
            slow("SELECT SLEEP(3)", secs(3)),
        ]),
        &side,
    )
    .await;
    // A statement under 500 ms opens none.
    run(&h, "SELECT 1").await.unwrap();
    assert_eq!(side.connect_calls(), 0);
    // One past 500 ms warms one, which outlives the statement.
    run(&h, "SELECT SLEEP(2)").await.unwrap();
    assert_eq!(side.connect_calls(), 1);
    until("the warm connection", || side.open_connections() == 1).await;
    tokio::time::sleep(secs(29)).await;
    assert_eq!(
        side.open_connections(),
        1,
        "warm within 30 s of its last use"
    );
    // The next long statement uses it again: no new connect, and its 30 s start over.
    run(&h, "SELECT SLEEP(3)").await.unwrap();
    let last_use = tokio::time::Instant::now();
    assert_eq!(side.connect_calls(), 1);
    tokio::time::sleep(secs(29)).await;
    assert_eq!(side.open_connections(), 1);
    until("the warm connection closes", || {
        side.open_connections() == 0
    })
    .await;
    // Review m4: 30 s after its last use, within one 10 ms step of the wait.
    let closed_after = last_use.elapsed();
    assert!(
        closed_after >= secs(30) && closed_after <= secs(30) + ms(20),
        "{closed_after:?}"
    );
    assert_eq!(side.connect_calls(), 1);
    assert_eq!(snap(&h).state, SessionState::Idle);
}

#[tokio::test(start_paused = true)]
async fn a_cancel_kills_on_the_warm_side_connection() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().exec(vec![
            slow("SELECT SLEEP(2)", secs(2)),
            slow("SELECT SLEEP(600)", secs(600)),
        ]),
        &side,
    )
    .await;
    run(&h, "SELECT SLEEP(2)").await.unwrap();
    until("the warm connection", || side.open_connections() == 1).await;
    // A statement held open (600 s on the paused clock): wait until it runs, then cancel.
    let job = h.submit(exec("SELECT SLEEP(600)"));
    until("the statement runs", || {
        matches!(snap(&h).state, SessionState::Busy { .. })
    })
    .await;
    let outcome = h.cancel(job.seq);
    // The kill went out on the warm connection: no second connect. (This side fake is not the
    // session's server, so the statement itself runs on: the caller hears `StillRunning`
    // after its 2 s wait.)
    until("the kill", || side.executed() == ["KILL QUERY 1000"]).await;
    assert_eq!(side.connect_calls(), 1);
    assert_eq!(outcome.await.unwrap(), CancelOutcome::StillRunning);
    let _ = job.reply.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_suspended_mysql_cursor_keeps_the_side_connection_warm() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().queries(vec![select("SELECT * FROM big", 1_000_000)]),
        &side,
    )
    .await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("SELECT * FROM big", true)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    assert!(snap(&h).cursor.is_some());
    until("the warm connection", || side.open_connections() == 1).await;
    // Held while the cursor is suspended, past 30 s (the fake MySQL cursor's idle bound is
    // 45 s).
    tokio::time::sleep(secs(40)).await;
    assert_eq!(side.open_connections(), 1);
    assert!(snap(&h).cursor.is_some());
    // The release kills on it; nothing warm is left afterwards.
    run(&h, "SELECT 2").await.unwrap();
    assert_eq!(side.executed(), ["KILL QUERY 1000"]);
    assert_eq!(side.connect_calls(), 1);
    until("no side connection", || side.open_connections() == 0).await;
}

#[tokio::test(start_paused = true)]
async fn closing_the_session_closes_the_warm_side_connection() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().exec(vec![slow("SELECT SLEEP(2)", secs(2))]),
        &side,
    )
    .await;
    run(&h, "SELECT SLEEP(2)").await.unwrap();
    until("the warm connection", || side.open_connections() == 1).await;
    h.close(ClosePolicy::RollBack).await.unwrap();
    until("the warm connection closes", || {
        side.open_connections() == 0
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// C11: the time zone

#[tokio::test(start_paused = true)]
async fn pg_time_zone_in_plan_and_replay_never_mysql() {
    const ZONE: &str = "SET TIME ZONE 'Europe/Istanbul'";
    let pg = profile_script(DbmsKind::Postgres, "SET work_mem = '64MB'");
    let plan = build_env_plan_with(
        DbmsKind::Postgres,
        &pg,
        false,
        None,
        Some("Europe/Istanbul"),
    );
    assert_eq!(
        plan.statements(),
        [
            "SET application_name = 'Hacivat'",
            ZONE,
            "SET work_mem = '64MB'"
        ]
    );
    assert_eq!(plan.after_connect(), [ZONE, "SET work_mem = '64MB'"]);
    // A read-only session's guard replays it after every reset.
    let ro_plan = build_env_plan_with(
        DbmsKind::Postgres,
        &ro_profile(),
        true,
        None,
        Some("Europe/Istanbul"),
    );
    assert!(ro_plan.replay().contains(&ZONE.to_string()));
    // Never on MySQL or MariaDB (R5).
    for kind in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let my = build_env_plan_with(
            kind,
            &mysql_profile(TlsMode::Disable),
            false,
            None,
            Some("Europe/Istanbul"),
        );
        assert!(
            !my.steps.iter().any(|s| matches!(s, EnvStep::TimeZone(_))),
            "{kind:?}"
        );
        assert!(my.statements().iter().all(|s| !s.contains("time_zone")));
    }

    // A connected profile: after connect, and again after a reset.
    let config = ManagerConfig {
        time_zone: TimeZonePref::Iana("Europe/Istanbul".into()),
        ..ManagerConfig::default()
    };
    let (m, f) = manager_with(pg_resettable(1), config.clone());
    let profile = p();
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    until("the time zone", || count(&f, ZONE) == 1).await;
    let pooled = m
        .sessions(profile.id)
        .expect("connected")
        .pooled(hacivat_session::session::SessionKind::Service)
        .await
        .expect("a pooled session");
    until("its time zone", || count(&f, ZONE) == 2).await;
    drop(pooled);
    // The release resets it, and the reset sends the zone again.
    until("the reset's time zone", || count(&f, ZONE) == 3).await;

    // MySQL: none.
    let (m, f) = manager_with(fake_mysql(), config);
    let profile = Arc::new(mysql_profile(TlsMode::Disable));
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let default = m
        .sessions(profile.id)
        .and_then(|s| s.default_session())
        .expect("the default session");
    run(&default, "SELECT 1").await.unwrap();
    assert!(
        f.executed()
            .iter()
            .all(|s| !s.to_ascii_lowercase().contains("time")),
        "{:?}",
        f.executed()
    );
}

/// The settings' `System` zone sends nothing: the server keeps its own.
#[tokio::test(start_paused = true)]
async fn the_system_time_zone_sends_nothing() {
    let (m, f) = manager_with(fake_pg(), ManagerConfig::default());
    connect_and_wait(&m).await;
    let default = m
        .sessions(p().id)
        .and_then(|s| s.default_session())
        .expect("the default session");
    run(&default, "SELECT 1").await.unwrap();
    assert_eq!(f.executed(), ["SELECT 1"]);
}

// ---------------------------------------------------------------------------------------------
// Close and break publish the final state (ledger 331)

#[tokio::test(start_paused = true)]
async fn closed_session_publishes_idle_tx_and_no_cursor() {
    // A user transaction and a suspended cursor inside it.
    let (h, f) = session(
        fake_pg()
            .tracks_transactions()
            .queries(vec![select("SELECT * FROM big", 1_000_000)]),
    )
    .await;
    run(&h, "BEGIN").await.unwrap();
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("SELECT * FROM big", true)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    let before = snap(&h);
    assert_eq!(
        (before.tx.status, before.tx.owner),
        (TxStatus::InTx, TxOwner::User)
    );
    assert!(before.cursor.is_some());
    let mut ev = h.events();
    h.close(ClosePolicy::RollBack).await.unwrap();
    let after = snap(&h);
    assert_eq!(after.state, SessionState::Disconnected);
    assert_eq!(
        (
            after.tx.status,
            after.tx.owner,
            after.tx.statements,
            after.tx.since
        ),
        (TxStatus::Idle, TxOwner::None, 0, None)
    );
    assert_eq!(after.tx.mode, before.tx.mode, "the mode is kept");
    assert_eq!(after.cursor, None);
    assert!(f.executed().iter().any(|s| s == "ROLLBACK"));
    // Its end is told as a transaction change.
    let mut ended = false;
    while let Ok(event) = ev.try_recv() {
        if let SessionEvent::TxChanged(tx) = event {
            ended |= tx.status == TxStatus::Idle && tx.owner == TxOwner::None;
        }
    }
    assert!(ended, "a TxChanged with the final state");
}

#[tokio::test(start_paused = true)]
async fn broken_session_publishes_idle_tx_and_no_cursor() {
    let (h, _f) = session(
        fake_pg()
            .tracks_transactions()
            .exec(vec![slow("SELECT pg_sleep(600)", secs(600))]),
    )
    .await;
    run(&h, "BEGIN").await.unwrap();
    run(&h, "UPDATE t SET a = 1").await.unwrap();
    assert_eq!(snap(&h).tx.status, TxStatus::InTx);
    let mut ev = h.events();
    // A statement held open, then the session is disconnected under it.
    let job = h.submit(exec("SELECT pg_sleep(600)"));
    until("the statement runs", || {
        matches!(snap(&h).state, SessionState::Busy { .. })
    })
    .await;
    let outcome = h.escalate(job.seq, Escalation::Deactivate).await.unwrap();
    assert_eq!(outcome, CancelOutcome::Cancelled);
    let after = snap(&h);
    assert_eq!(after.state, SessionState::Broken(ErrClass::Cancelled));
    assert_eq!(
        (after.tx.status, after.tx.owner),
        (TxStatus::Idle, TxOwner::None)
    );
    assert_eq!(after.cursor, None);
    let mut events = Vec::new();
    while let Ok(event) = ev.try_recv() {
        events.push(event);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::TxChanged(tx) if tx.status == TxStatus::Idle)),
        "{events:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// C11 fallback: the startup option a pooler refuses

/// The plan's `startup_option_refused_falls_back_to_set` is the driver's (the refusal happens
/// in the PostgreSQL handshake; `hacivat-driver-pg/tests/startup_fallback_m3b.rs`), which
/// reconnects without the options and sends them as `SET`. The session's half: a read-only
/// PostgreSQL session's guard replays the same bound after every reset, so it holds for every
/// guarded statement after the first.
#[tokio::test(start_paused = true)]
async fn the_guard_replays_the_idle_bound_after_every_statement() {
    let (h, f) = ro_session(pg_ro_clean(2), ro_profile()).await;
    run(&h, "SELECT 1").await.unwrap();
    run(&h, "SELECT 2").await.unwrap();
    assert!(
        count(&f, "SET idle_in_transaction_session_timeout = '30s'") >= 2,
        "{:?}",
        f.executed()
    );
}

// ---------------------------------------------------------------------------------------------
// The T13 carry: notices the driver dropped

#[tokio::test(start_paused = true)]
async fn notices_the_driver_dropped_join_the_units_count() {
    let chatty = FakeQuery {
        notices: notices(103),
        ..answering("SELECT")
    };
    let (h, _f) = session(fake_pg().notices_dropped(40).queries(vec![chatty])).await;
    let o = Recorder::new();
    let logs = capture_tracing(|| async {
        run_spec(&h, spec_units(&o, vec![unit(0, "SELECT raise_many()")]))
            .await
            .unwrap();
    })
    .await;
    assert_eq!(notice_texts(&o).len(), 100);
    // 3 past the session's cap and 40 past the driver's.
    assert!(logs.contains("notices_dropped=43"), "{logs}");
}

// ---------------------------------------------------------------------------------------------
// Helpers' own checks

#[test]
fn the_script_splits_into_its_statements() {
    assert_eq!(
        startup_statements(DbmsKind::Postgres, SCRIPT).unwrap(),
        ["SET search_path = app", "SET work_mem = '64MB'"]
    );
    assert_eq!(
        startup_statements(DbmsKind::MySql, "SET sql_mode = 'ANSI';;\n SET @a = 1").unwrap(),
        ["SET sql_mode = 'ANSI'", "SET @a = 1"]
    );
    // A client command refuses it whole.
    assert!(startup_statements(DbmsKind::MySql, "DELIMITER //\nSET @a = 1//").is_err());
    assert!(startup_statements(DbmsKind::Postgres, "\\set x 1\nSET a = 1;").is_err());
}

// ---------------------------------------------------------------------------------------------
// ## Fix round 1 (review I2, I3, m2, m3, m7)

/// A manager with reconnects, over `fake`, with `zone` as the settings' time zone.
fn reconnecting(fake: FakeDriver, zone: TimeZonePref) -> (Arc<ConnectionManager>, Arc<FakeDriver>) {
    manager_with(
        fake,
        ManagerConfig {
            reconnect: true,
            time_zone: zone,
            ..ManagerConfig::default()
        },
    )
}

fn default_of(m: &ConnectionManager, profile: &Profile) -> SessionHandle {
    m.sessions(profile.id)
        .and_then(|s| s.default_session())
        .expect("the default session")
}

/// Review I2: a script that waits holds the session `Busy` (request 0), a request waits its
/// turn, and a close ends it at once.
#[tokio::test(start_paused = true)]
async fn a_waiting_script_holds_the_session_busy_and_a_close_ends_it() {
    let side = Arc::new(fake_pg());
    let (h, f) = spawn_over(
        fake_pg().exec(vec![slow("SELECT pg_advisory_lock", secs(600))]),
        profile_script(DbmsKind::Postgres, "SELECT pg_advisory_lock(1)"),
        false,
        side,
    )
    .await;
    until("the script runs", || {
        matches!(snap(&h).state, SessionState::Busy { req: 0, .. })
    })
    .await;
    // A request waits behind it; a cancel finds nothing of its own running.
    let queued = h.submit(exec("SELECT 1"));
    assert_eq!(
        h.cancel(queued.seq).await.unwrap(),
        CancelOutcome::AlreadyFinished
    );
    let started = tokio::time::Instant::now();
    h.close(ClosePolicy::RollBack).await.unwrap();
    assert!(started.elapsed() < secs(1), "the close is served at once");
    assert_eq!(snap(&h).state, SessionState::Disconnected);
    assert!(queued.reply.await.unwrap().is_err());
    assert!(!f.executed().iter().any(|s| s == "SELECT 1"));
}

/// Review I2: a script still running at the profile's connect timeout fails closed.
#[tokio::test(start_paused = true)]
async fn a_script_past_the_connect_timeout_breaks_the_session() {
    let side = Arc::new(fake_pg());
    let profile = Profile {
        connect_timeout_s: 5,
        ..profile_script(DbmsKind::Postgres, "SELECT pg_advisory_lock(1)")
    };
    let (h, _f) = spawn_over(
        fake_pg().exec(vec![slow("SELECT pg_advisory_lock", secs(600))]),
        profile,
        false,
        side,
    )
    .await;
    let started = tokio::time::Instant::now();
    until("the session ends", || {
        matches!(snap(&h).state, SessionState::Broken(_))
    })
    .await;
    assert_eq!(
        snap(&h).state,
        SessionState::Broken(ErrClass::ConfigInvalid)
    );
    let took = started.elapsed();
    assert!(took >= secs(4) && took < secs(6), "{took:?}");
}

/// Review I3a: the script sets the starting context; the user's own schema is restored after
/// it.
#[test]
fn the_script_runs_before_the_schema_restore() {
    let plan = build_env_plan_with(
        DbmsKind::Postgres,
        &profile_script(DbmsKind::Postgres, SCRIPT),
        false,
        Some(&schema("sales")),
        Some("Europe/Istanbul"),
    );
    assert_eq!(
        plan.statements(),
        [
            "SET application_name = 'Hacivat'",
            "SET TIME ZONE 'Europe/Istanbul'",
            "SET search_path = app",
            "SET work_mem = '64MB'",
            "SET search_path TO \"sales\"",
        ]
    );
}

/// Review I3b: after the script the schema is probed, and the session shows it.
#[tokio::test(start_paused = true)]
async fn the_session_shows_the_schema_the_script_set() {
    let probe = rows(
        "SELECT current_database()",
        &[&["hv_test", "app", "app", "hv_test", "app"]],
    );
    let (h, _f) = session_with_profile(
        fake_pg().exec(vec![probe]),
        profile_script(DbmsKind::Postgres, "SET search_path = app"),
    )
    .await;
    assert_eq!(
        snap(&h).schema.and_then(|s| s.schema).as_deref(),
        Some("app")
    );
    assert_eq!(snap(&h).state, SessionState::Idle);
}

/// Review I3c: a script that leaves a transaction open is refused.
#[tokio::test(start_paused = true)]
async fn a_script_that_leaves_a_transaction_open_is_refused() {
    let side = Arc::new(fake_pg());
    let (h, _f) = spawn_over(
        fake_pg().tracks_transactions(),
        profile_script(DbmsKind::Postgres, "BEGIN; SET LOCAL work_mem = '64MB'"),
        false,
        side,
    )
    .await;
    until("the session ends", || {
        !matches!(
            snap(&h).state,
            SessionState::Idle | SessionState::Busy { .. }
        )
    })
    .await;
    assert_eq!(
        snap(&h).state,
        SessionState::Broken(ErrClass::ConfigInvalid)
    );
    assert_eq!(snap(&h).tx.status, TxStatus::Idle);
}

/// Review I3d and m2: a Manual MySQL session sends `autocommit = 0` after the script, on
/// connect and on a reconnect alike.
#[tokio::test(start_paused = true)]
async fn manual_mysql_turns_autocommit_off_after_the_script_on_connect_and_reconnect() {
    let (m, f) = reconnecting(fake_mysql(), TimeZonePref::System);
    let profile = Arc::new(Profile {
        tx_mode: Some(TxMode::Manual),
        ..profile_script(DbmsKind::MySql, "SET autocommit = 1; SET @app = 1")
    });
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    let once = ["SET autocommit = 1", "SET @app = 1", "SET autocommit = 0"];
    until("the connect's steps", || sent(&f) == once).await;
    let default = default_of(&m, &profile);
    assert_eq!(snap(&default).tx.mode, TxMode::Manual);
    f.kill_connections();
    let _ = default.submit(Request::Ping);
    until("the reconnect's replay", || sent(&f).len() == 6).await;
    assert_eq!(sent(&f)[3..], once);
    until("connected again", || {
        matches!(m.state(profile.id), LiveState::Connected(_))
    })
    .await;
    assert_eq!(snap(&default_of(&m, &profile)).tx.mode, TxMode::Manual);
}

/// Review m2: a reconnect replays the time zone and the script (before the schema restore).
#[tokio::test(start_paused = true)]
async fn a_reconnect_replays_the_time_zone_and_the_script() {
    let (m, f) = reconnecting(fake_pg(), TimeZonePref::Iana("Europe/Istanbul".into()));
    let profile = Arc::new(profile_script(DbmsKind::Postgres, SCRIPT));
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    until("the connect's steps", || {
        count(&f, "SET work_mem = '64MB'") == 1
    })
    .await;
    f.kill_connections();
    let _ = default_of(&m, &profile).submit(Request::Ping);
    until("the replay", || count(&f, "SET work_mem = '64MB'") == 2).await;
    assert_eq!(count(&f, "SET TIME ZONE 'Europe/Istanbul'"), 2);
    let replayed = sent(&f);
    let tz = replayed
        .iter()
        .rposition(|s| s == "SET TIME ZONE 'Europe/Istanbul'")
        .unwrap();
    assert_eq!(
        replayed[tz..tz + 3],
        [
            "SET TIME ZONE 'Europe/Istanbul'",
            "SET search_path = app",
            "SET work_mem = '64MB'"
        ]
    );
}

/// Review m2: a script that fails on a reconnect ends the profile with `config.invalid`, as at
/// connect, and is not retried as a lost connection.
#[tokio::test(start_paused = true)]
async fn a_script_failing_on_reconnect_is_config_invalid() {
    let fake = fake_pg().exec(vec![
        tx("SET app.mode", TxStatus::Idle),
        tx_err("SET app.mode", "22023", TxStatus::Idle),
    ]);
    let (m, f) = reconnecting(fake, TimeZonePref::System);
    let profile = Arc::new(profile_script(DbmsKind::Postgres, "SET app.mode = 'x'"));
    m.connect(profile.clone(), None, Interaction::Headless);
    wait_connected(&m, profile.id).await;
    until("the connect's script", || {
        count(&f, "SET app.mode = 'x'") == 1
    })
    .await;
    let connects = f.connect_calls();
    f.kill_connections();
    let _ = default_of(&m, &profile).submit(Request::Ping);
    until("the profile ends", || {
        matches!(m.state(profile.id), LiveState::Lost { .. })
    })
    .await;
    assert_eq!(
        m.state(profile.id),
        LiveState::Lost {
            class: ErrClass::ConfigInvalid
        }
    );
    assert_eq!(
        f.connect_calls(),
        connects + 1,
        "one reconnect, no backoff loop"
    );
}

/// Review m7: a zone the server refuses breaks one session, says which setting it was, and is
/// never sent again; a name that is no zone is never sent at all.
#[tokio::test(start_paused = true)]
async fn an_unknown_time_zone_is_refused_once() {
    const ZONE: &str = "SET TIME ZONE 'Mars/Olympus'";
    let logs = capture_tracing(|| async {
        let fake = fake_pg().exec(vec![tx_err("SET TIME ZONE", "22023", TxStatus::Idle)]);
        let (m, f) = reconnecting(fake, TimeZonePref::Iana("Mars/Olympus".into()));
        let profile = p();
        m.connect(profile.clone(), None, Interaction::Headless);
        until("the refusal", || {
            matches!(m.state(profile.id), LiveState::Lost { .. })
        })
        .await;
        assert_eq!(
            m.state(profile.id),
            LiveState::Lost {
                class: ErrClass::ConfigInvalid
            }
        );
        // Connecting again works, without the zone.
        m.connect(profile.clone(), None, Interaction::Headless);
        wait_connected(&m, profile.id).await;
        let default = default_of(&m, &profile);
        run(&default, "SELECT 1").await.unwrap();
        assert_eq!(count(&f, ZONE), 1);
    })
    .await;
    assert_eq!(
        logs.matches("the server refused the time zone setting")
            .count(),
        1,
        "{logs}"
    );
    assert!(logs.contains("ui.time_zone"));

    let (m, f) = reconnecting(fake_pg(), TimeZonePref::Iana("../etc/passwd".into()));
    connect_and_wait(&m).await;
    let default = default_of(&m, &p());
    run(&default, "SELECT 1").await.unwrap();
    assert!(f.executed().iter().all(|s| !s.contains("TIME ZONE")));
}

/// Review m3: only a use of the side connection moves its clock; short requests do not keep
/// it open.
#[tokio::test(start_paused = true)]
async fn a_short_request_does_not_keep_the_warm_connection_open() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().exec(vec![slow("SELECT SLEEP(2)", secs(2))]),
        &side,
    )
    .await;
    run(&h, "SELECT SLEEP(2)").await.unwrap();
    let last_use = tokio::time::Instant::now();
    until("the warm connection", || side.open_connections() == 1).await;
    for _ in 0..5 {
        tokio::time::sleep(secs(5)).await;
        run(&h, "SELECT 1").await.unwrap();
    }
    until("the warm connection closes", || {
        side.open_connections() == 0
    })
    .await;
    let closed_after = last_use.elapsed();
    assert!(
        closed_after >= secs(30) && closed_after <= secs(30) + ms(20),
        "{closed_after:?}"
    );
}

/// Review m3: a warm connect that fails is tried once more.
#[tokio::test(start_paused = true)]
async fn a_failed_warm_connect_is_tried_once_more() {
    let side = Arc::new(fake_mysql().script(vec![Err(refused()), Ok(())]));
    let (h, _f) = session_with_side(
        fake_mysql().exec(vec![slow("SELECT SLEEP(2)", secs(2))]),
        &side,
    )
    .await;
    run(&h, "SELECT SLEEP(2)").await.unwrap();
    until("the second connect", || side.open_connections() == 1).await;
    assert_eq!(side.connect_calls(), 2);
}

/// Review m3: a cursor release that drains in time kills nothing and leaves the warm
/// connection as it was.
#[tokio::test(start_paused = true)]
async fn a_release_that_drains_in_time_leaves_the_warm_connection() {
    let side = Arc::new(fake_mysql());
    let (h, _f) = session_with_side(
        fake_mysql().queries(vec![select("SELECT * FROM small", 12_000)]),
        &side,
    )
    .await;
    let o = Recorder::new();
    run_spec(
        &h,
        spec_with(&o, &[("SELECT * FROM small", true)], FetchPolicy::default()),
    )
    .await
    .unwrap();
    assert!(snap(&h).cursor.is_some());
    until("the warm connection", || side.open_connections() == 1).await;
    run(&h, "SELECT 2").await.unwrap();
    assert!(snap(&h).cursor.is_none());
    assert!(side.executed().is_empty(), "{:?}", side.executed());
    assert_eq!(side.open_connections(), 1, "still warm");
    assert_eq!(side.connect_calls(), 1);
}
