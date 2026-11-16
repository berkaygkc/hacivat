//! M3a Task 22: the typed read-only (guarded) path, HV-SESS-04 on `Request::Run` (R-M3-17).
//! A read-only session runs every typed unit inside the guard's read-only transaction: its
//! `BEGIN … READ ONLY` before the query, PostgreSQL's canary after the first chunk, and the
//! post steps (canary, `ROLLBACK`, post-check, reset, replay) when the cursor ends, never
//! before. A gap discards the connection and keeps the rows. FakeDriver and a paused clock (the
//! SQLite case runs the real driver on real time); the Docker legs are
//! `docker_m3a_readonly.rs`.
//!
//! The fakes follow `BEGIN`/`START`/`ROLLBACK` (`tracks_transactions`) and answer the canary
//! from a script; an unscripted canary answers no row, which the guard cannot read, so it
//! fails closed (a test that forgets one sees `Internal`).

mod common;

use std::sync::Arc;

use common::runs::*;
use common::*;
use hacivat_driver::fake::{FakeCall, FakeDriver, FakeExec, FakeQuery, FakeTypedCall};
use hacivat_driver::{CloseHow, ExecSummary, RawError, RawErrorKind, ServerMessage, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_results::{CloseReason, Completeness, FetchPolicy};
use hacivat_session::GapReason;
use hacivat_session::profile::Profile;
use hacivat_session::session::{
    DenyLayer, ExecErrorKind, Request, SessionEvent, SessionHandle, SessionState, TxOwner, UnitEnd,
};

const CANARY: &str = "SELECT pg_catalog.txid_current_if_assigned()";
const BIG: &str = "SELECT * FROM big";

// ---------------------------------------------------------------------------------------------
// Fixtures

/// The canary's answer: SQL NULL (no transaction ID), or an ID.
fn canary(id: Option<&str>) -> FakeExec {
    FakeExec {
        prefix: CANARY,
        result: Ok(ExecSummary {
            rows: vec![vec![id.map(str::to_string)]],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: std::time::Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// `n` canaries that find no transaction ID.
fn clean(n: usize) -> Vec<FakeExec> {
    (0..n).map(|_| canary(None)).collect()
}

/// A PostgreSQL fake that follows the guard's transaction.
fn pg(script: Vec<FakeExec>) -> FakeDriver {
    fake_pg().tracks_transactions().exec(script)
}

fn mysql() -> FakeDriver {
    fake_mysql().tracks_transactions()
}

fn ro_pg() -> Profile {
    ro_profile()
}

fn ro_mysql() -> Profile {
    mysql_profile(TlsMode::Disable)
}

/// A read-only session over `fake`, and where its call log stood once it was connected (the
/// role probe and the setup are behind it).
async fn ro(fake: FakeDriver, profile: Profile) -> (SessionHandle, Arc<FakeDriver>, usize) {
    let (h, f) = ro_session(fake, profile).await;
    assert!(snap(&h).read_only);
    let base = f.call_log().len();
    (h, f, base)
}

/// One step of the guard, as the call log shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// The plan's `before` (`BEGIN … READ ONLY` and its `SELECT 1`, `START TRANSACTION READ
    /// ONLY`).
    PlanBefore,
    Query,
    Fetch,
    Close(CloseHow),
    Canary,
    Rollback,
    Reset,
    /// The session's environment again (one or more `SET`s).
    Replay,
    Other(String),
}

/// The guard's steps since `from`; runs of `PlanBefore` and of `Replay` count once.
fn steps(f: &FakeDriver, from: usize) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::new();
    for call in &f.call_log()[from..] {
        let step = match call {
            FakeCall::Statement(s)
                if s.starts_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    || s == "START TRANSACTION READ ONLY"
                    || s == "SELECT 1" =>
            {
                Step::PlanBefore
            }
            FakeCall::Statement(s) if s == CANARY => Step::Canary,
            FakeCall::Statement(s) if s == "ROLLBACK" => Step::Rollback,
            FakeCall::Statement(s) if s.starts_with("SET ") => Step::Replay,
            FakeCall::Statement(s) => Step::Other(s.clone()),
            FakeCall::Typed(FakeTypedCall::Query(_)) => Step::Query,
            FakeCall::Typed(FakeTypedCall::FetchMore(_)) => Step::Fetch,
            FakeCall::Typed(FakeTypedCall::Close(how)) => Step::Close(*how),
            // ## m3b-t01c-core-seams: a new fake call (no M3a query has binds).
            FakeCall::Typed(FakeTypedCall::Binds(b)) => Step::Other(format!("binds {}", b.len())),
            FakeCall::Reset => Step::Reset,
        };
        let repeat = matches!(step, Step::PlanBefore | Step::Replay) && out.last() == Some(&step);
        if !repeat {
            out.push(step);
        }
    }
    out
}

/// A run of `sql` (suspendable or not) with the default policy.
async fn run_one(h: &SessionHandle, sql: &str, suspendable: bool) -> (Arc<Recorder>, Reply) {
    let o = Recorder::new();
    let reply = run_spec(
        h,
        spec_with(&o, &[(sql, suspendable)], FetchPolicy::default()),
    )
    .await;
    (o, reply)
}

type Reply = hacivat_session::session::Reply;

fn closed(reason: CloseReason) -> Completeness {
    Completeness::Closed { reason }
}

fn fetch_all(o: &Recorder) -> Request {
    Request::FetchAll {
        result: o.store(0).id(),
    }
}

fn pg_error(sqlstate: &str) -> RawError {
    RawError {
        kind: RawErrorKind::Db,
        server: Some(ServerMessage {
            sqlstate: Some(sqlstate.to_string()),
            vendor_code: None,
            severity: Some("ERROR".to_string()),
            text: "refused".to_string(),
            detail: None,
            hint: None,
            lang_hint: None,
            position: None,
        }),
    }
}

/// D-019, one request = one transaction: every guard's read-only transaction (from its
/// `BEGIN`/`START` to its `ROLLBACK`) holds exactly one user statement (a typed query with its
/// fetches and close, or one M2b statement) besides the guard's own (its `before`, its canary,
/// a release's absorbing `DO 0`). Panics naming what broke it.
fn assert_nothing_else_inside(f: &FakeDriver, from: usize) {
    let log = f.call_log();
    let mut inside = false;
    let mut users = 0;
    for call in &log[from..] {
        match call {
            FakeCall::Statement(s)
                if s.starts_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                    || s == "START TRANSACTION READ ONLY" =>
            {
                assert!(
                    !inside,
                    "a guard's transaction opened inside another: {log:?}"
                );
                inside = true;
                users = 0;
            }
            FakeCall::Statement(s) if s == "ROLLBACK" => {
                assert_eq!(
                    users, 1,
                    "a guard's transaction held {users} statements: {log:?}"
                );
                inside = false;
            }
            FakeCall::Statement(s) if s == "SELECT 1" || s == CANARY || s == "DO 0" => {}
            FakeCall::Statement(_) | FakeCall::Typed(FakeTypedCall::Query(_)) if inside => {
                users += 1;
            }
            FakeCall::Reset if inside => panic!("a reset inside the guard's transaction: {log:?}"),
            _ => {}
        }
    }
    assert!(!inside, "a guard's transaction was left open: {log:?}");
}

// ---------------------------------------------------------------------------------------------
// PostgreSQL

#[tokio::test(start_paused = true)]
async fn guard_begin_precedes_the_query_and_rollback_ends_it() {
    // 1,200 rows: the first chunk (500) suspends, the canary runs, the read-ahead completes the
    // result, and the post steps follow its end.
    let (h, f, base) = ro(
        pg(clean(2)).queries(vec![select("SELECT * FROM t", 1_200)]),
        ro_pg(),
    )
    .await;
    let (o, reply) = run_one(&h, "SELECT * FROM t", true).await;
    reply.expect("the guarded run succeeds");
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.rows(), 1_200);
    assert_eq!(
        steps(&f, base),
        [
            Step::PlanBefore,
            Step::Query,
            Step::Canary,
            Step::Fetch,
            Step::Canary,
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
    assert!(!f.executed().iter().any(|s| s.contains("COMMIT")));
    let s = snap(&h);
    assert_eq!((s.state, s.tx.status), (SessionState::Idle, TxStatus::Idle));
}

#[tokio::test(start_paused = true)]
async fn a_unit_that_cannot_suspend_runs_inside_the_guard_too() {
    // `Execute(0)`: no suspension, so no canary in the middle; the post steps follow `Done`.
    let (h, f, base) = ro(
        pg(clean(1)).queries(vec![select("SELECT * FROM t", 3)]),
        ro_pg(),
    )
    .await;
    let (o, reply) = run_one(&h, "SELECT * FROM t", false).await;
    reply.expect("the guarded run succeeds");
    assert_eq!(o.rows(), 3);
    assert_eq!(
        steps(&f, base),
        [
            Step::PlanBefore,
            Step::Query,
            Step::Canary,
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn post_steps_wait_for_the_cursor() {
    let (h, f, base) = ro(pg(clean(2)).queries(vec![select(BIG, 30_000)]), ro_pg()).await;
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    // Suspended at its read-ahead: the guard's transaction stays open, only the first canary
    // ran, nothing of the end yet.
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    let suspended = steps(&f, base);
    assert_eq!(
        suspended[..3],
        [Step::PlanBefore, Step::Query, Step::Canary]
    );
    assert!(
        !suspended
            .iter()
            .any(|s| matches!(s, Step::Rollback | Step::Reset | Step::Replay)),
        "{suspended:?}"
    );
    let s = snap(&h);
    assert_eq!((s.tx.status, s.tx.owner), (TxStatus::InTx, TxOwner::Guard));
    assert!(s.cursor.is_some_and(|c| c.holds_tx));

    // The fetch that reaches the end runs them.
    ask(&h, fetch_all(&o)).await.expect("the fetch runs");
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    assert_eq!(o.rows(), 30_000);
    let ended = steps(&f, base);
    assert_eq!(
        ended[ended.len() - 4..],
        [Step::Canary, Step::Rollback, Step::Reset, Step::Replay]
    );
    let s = snap(&h);
    assert_eq!((s.tx.status, s.cursor), (TxStatus::Idle, None));
}

#[tokio::test(start_paused = true)]
async fn canary_after_the_first_chunk_catches_a_writing_function() {
    // A read head whose function got a transaction ID in its first rows.
    let (h, f, base) = ro(
        pg(vec![canary(Some("731"))]).queries(vec![select(BIG, 30_000)]),
        ro_pg(),
    )
    .await;
    let mut events = h.events();
    let (o, reply) = run_one(&h, BIG, true).await;
    assert_eq!(
        reply.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned)
    );
    // The first chunk's rows stay, flagged.
    assert_eq!(o.rows(), 500);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::EnforcementGap)
    );
    assert_eq!(o.releases(), vec![(CloseReason::EnforcementGap, false)]);
    assert!(matches!(&o.unit_ends()[..], [UnitEnd::Failed(e)]
        if e.kind == ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned)));
    // Discarded unreset: nothing more was sent on it, no rollback, no reset, no fetch.
    assert_eq!(
        steps(&f, base),
        [Step::PlanBefore, Step::Query, Step::Canary]
    );
    loop {
        if next(&mut events).await == SessionEvent::EnforcementGap(GapReason::CanaryAssigned) {
            break;
        }
    }
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}

#[tokio::test(start_paused = true)]
async fn gap_discards_the_connection_and_flags_the_rows() {
    // The canary at the end finds what the one after the first chunk did not: a function that
    // wrote on a later row, caught at the end of a cursor fetched to its end (`FetchAll`; a
    // release's gap is `a_gap_found_at_a_release_discards_the_connection`).
    let (h, f, base) = ro(
        pg(vec![canary(None), canary(Some("732"))]).queries(vec![select(BIG, 30_000)]),
        ro_pg(),
    )
    .await;
    let mut events = h.events();
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the first canary is clean");
    let gap = ask(&h, fetch_all(&o)).await.unwrap_err();
    assert_eq!(
        gap.kind,
        ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned)
    );
    // Every row stays, the result says why it is not complete.
    assert_eq!(o.rows(), 30_000);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::EnforcementGap)
    );
    // As on M2b: the canary, the rollback (which undoes what the transaction can), then the
    // verdict; the connection is discarded unreset.
    let ended = steps(&f, base);
    assert_eq!(
        ended[ended.len() - 2..],
        [Step::Canary, Step::Rollback],
        "{ended:?}"
    );
    assert!(!ended.contains(&Step::Reset));
    loop {
        if next(&mut events).await == SessionEvent::EnforcementGap(GapReason::CanaryAssigned) {
            break;
        }
    }
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
    assert!(!f.executed().iter().any(|s| s.contains("COMMIT")));
}

#[tokio::test(start_paused = true)]
async fn a_row_stream_that_ends_the_transaction_partway_is_a_gap() {
    // Review probe: the statement leaves the guard's transaction behind (the fake's query
    // reports the connection idle once it is done). The canary then runs outside any
    // transaction and answers NULL; the post-check reads the status and finds the gap.
    let ended = FakeQuery {
        tx_after: Some(TxStatus::Idle),
        ..select("SELECT * FROM t", 3)
    };
    let (h, f, base) = ro(pg(clean(1)).queries(vec![ended]), ro_pg()).await;
    let (o, reply) = run_one(&h, "SELECT * FROM t", false).await;
    assert_eq!(
        reply.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(GapReason::TransactionEndedByStatement)
    );
    assert_eq!(o.rows(), 3);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::EnforcementGap)
    );
    assert!(!steps(&f, base).contains(&Step::Reset));
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}

#[tokio::test(start_paused = true)]
async fn a_canary_that_cannot_be_read_fails_closed() {
    // No scripted answer: the canary after the first chunk answers no row. The guard cannot
    // tell, so the connection is discarded and the rows stay, closed.
    let (h, f, base) = ro(pg(Vec::new()).queries(vec![select(BIG, 30_000)]), ro_pg()).await;
    let (o, reply) = run_one(&h, BIG, true).await;
    assert_eq!(reply.unwrap_err().kind, ExecErrorKind::Internal);
    assert_eq!(o.rows(), 500);
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Closed { .. }
    ));
    assert_eq!(
        steps(&f, base),
        [Step::PlanBefore, Step::Query, Step::Canary]
    );
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}

#[tokio::test(start_paused = true)]
async fn a_refused_write_is_rolled_back_and_the_session_goes_on() {
    // The server refuses the write inside the read-only transaction (25006); the guard ends
    // its transaction the usual way and the connection is kept.
    // (The fake fails a query when it would send a row: `DELETE … RETURNING` here.)
    let refused = FakeQuery {
        fail_after: Some((0, pg_error("25006"))),
        ..select("DELETE", 5)
    };
    let (h, f, base) = ro(
        pg(clean(1)).queries(vec![refused, select("SELECT * FROM t", 2)]),
        ro_pg(),
    )
    .await;
    let (_, reply) = run_one(&h, "DELETE FROM t", false).await;
    let e = reply.unwrap_err();
    assert_eq!(e.kind, ExecErrorKind::Db);
    assert_eq!(e.server.and_then(|s| s.sqlstate).as_deref(), Some("25006"));
    let after = steps(&f, base);
    // A failed PostgreSQL transaction takes no canary; the rollback, reset and replay follow.
    assert_eq!(
        after[after.len() - 3..],
        [Step::Rollback, Step::Reset, Step::Replay]
    );
    assert!(!after.contains(&Step::Canary));
    assert_eq!(snap(&h).state, SessionState::Idle);
    // The session runs on.
    let (o, reply) = run_one(&h, "SELECT * FROM t", false).await;
    reply.expect("the next run succeeds");
    assert_eq!(o.rows(), 2);
}

#[tokio::test(start_paused = true)]
async fn another_request_releases_and_runs_the_post_steps_first() {
    let (h, f, base) = ro(pg(clean(3)).queries(vec![select(BIG, 30_000)]), ro_pg()).await;
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    // A plain statement on the session: the cursor is released and its guard ended first,
    // then the statement runs under a guard of its own.
    run(&h, "SELECT 2").await.expect("the statement runs");
    assert_eq!(o.releases(), vec![(CloseReason::SessionNeeded, false)]);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::SessionNeeded)
    );
    let all = steps(&f, base);
    let release = all
        .iter()
        .position(|s| matches!(s, Step::Close(CloseHow::Drain(_))))
        .expect("the cursor was closed");
    assert_eq!(
        all[release..],
        [
            Step::Close(CloseHow::Drain(ms(250))),
            Step::Canary,
            Step::Rollback,
            Step::Reset,
            Step::Replay,
            // The statement's own guard.
            Step::PlanBefore,
            Step::Other("SELECT 2".into()),
            Step::Canary,
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
    assert_nothing_else_inside(&f, base);
}

#[tokio::test(start_paused = true)]
async fn ro_bound_is_20s() {
    // A stream that never ends, left idle: the cursor is released at 20 s, well before the
    // server's 30 s `idle_in_transaction_session_timeout` would end the connection; the guard's
    // steps run and the session stays.
    let endless = select(BIG, u64::MAX);
    let (h, f, base) = ro(pg(clean(2)).queries(vec![endless]), ro_pg()).await;
    let start = tokio::time::Instant::now();
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    let Completeness::Suspended(info) = *o.snap(0).completeness() else {
        panic!("{:?}", o.snap(0).completeness());
    };
    assert_eq!(info.idle_bound, secs(20));
    assert!(info.holds_tx);
    tokio::time::sleep(ms(19_900)).await;
    settle().await;
    assert!(o.releases().is_empty());
    let mut state = h.state();
    tokio::time::timeout(secs(60), state.wait_for(|s| s.cursor.is_none()))
        .await
        .expect("released within the wait")
        .unwrap();
    assert_eq!(start.elapsed(), secs(20));
    assert_eq!(o.releases(), vec![(CloseReason::IdleTimeout, false)]);
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::IdleTimeout));
    let all = steps(&f, base);
    assert_eq!(
        all[all.len() - 4..],
        [Step::Canary, Step::Rollback, Step::Reset, Step::Replay]
    );
    let s = snap(&h);
    assert_eq!((s.state, s.tx.status), (SessionState::Idle, TxStatus::Idle));
}

#[tokio::test(start_paused = true)]
async fn nothing_else_runs_inside_the_guard_transaction() {
    // D-019: a cursor, a fetch on it, a plain statement, a new run, a transaction command and a
    // ping: every guard's transaction holds only the guard's own statements and the query.
    let (h, f, base) = ro(
        pg(clean(8)).queries(vec![select(BIG, 100_000), select("SELECT * FROM t", 3)]),
        ro_pg(),
    )
    .await;
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    ask(
        &h,
        Request::FetchMore {
            result: o.store(0).id(),
            rows: 1_000,
        },
    )
    .await
    .expect("the fetch runs");
    run(&h, "SELECT 2").await.expect("the statement runs");
    let (_, reply) = run_one(&h, "SELECT * FROM t", true).await;
    reply.expect("the second run succeeds");
    ask(&h, Request::Ping).await.expect("the ping answers");
    assert_nothing_else_inside(&f, base);
    assert!(!f.executed().iter().any(|s| s.contains("COMMIT")));
}

#[tokio::test(start_paused = true)]
async fn transaction_control_and_bad_input_are_refused_before_anything_is_sent() {
    let (h, f, base) = ro(pg(Vec::new()), ro_pg()).await;
    for (sql, layer) in [
        ("COMMIT", DenyLayer::Transaction),
        ("/* x */ END", DenyLayer::Transaction),
        ("ROLLBACK", DenyLayer::Transaction),
        ("SELECT 1\0", DenyLayer::Input),
        (
            "SELECT pg_logical_emit_message(false, 'p', 'x')",
            DenyLayer::Function,
        ),
    ] {
        for suspendable in [false, true] {
            let (o, reply) = run_one(&h, sql, suspendable).await;
            assert_eq!(
                reply.unwrap_err().kind,
                ExecErrorKind::ReadOnlyDenied(layer),
                "{sql:?}"
            );
            assert_eq!(o.stores(), 0, "{sql:?} opened a result");
        }
    }
    assert_eq!(f.call_log().len(), base, "{:?}", &f.call_log()[base..]);
    assert_eq!(snap(&h).state, SessionState::Idle);
}

// ---------------------------------------------------------------------------------------------
// ## fix round 1 (review I2): a gap found at a release, and stops

#[tokio::test(start_paused = true)]
async fn a_gap_found_at_a_release_discards_the_connection() {
    // A function that wrote after the first chunk, on a cursor that is not fetched to its
    // end: the release (`CloseCursor`, another request, the idle bound) closes the query
    // before its end, so the statement is judged as not completed; the end canary still runs
    // (the guard's transaction is open) and finds the transaction ID.
    for how in ["CloseCursor", "another request", "the idle bound"] {
        let (h, f, base) = ro(
            pg(vec![canary(None), canary(Some("733"))]).queries(vec![select(BIG, 30_000)]),
            ro_pg(),
        )
        .await;
        let mut events = h.events();
        let (o, reply) = run_one(&h, BIG, true).await;
        reply.expect("the first canary is clean");
        assert!(matches!(
            o.snap(0).completeness(),
            Completeness::Suspended(_)
        ));
        match how {
            "CloseCursor" => {
                let e = ask(
                    &h,
                    Request::CloseCursor {
                        result: o.store(0).id(),
                        reason: CloseReason::UserClosed,
                    },
                )
                .await
                .unwrap_err();
                assert_eq!(
                    e.kind,
                    ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned),
                    "{how}"
                );
            }
            "another request" => {
                let e = run(&h, "SELECT 2").await.unwrap_err();
                assert_eq!(
                    e.kind,
                    ExecErrorKind::EnforcementGap(GapReason::CanaryAssigned),
                    "{how}"
                );
                assert!(
                    !f.executed().iter().any(|s| s == "SELECT 2"),
                    "{how}: the request ran on a discarded connection"
                );
            }
            _ => {
                let mut state = h.state();
                tokio::time::timeout(secs(60), state.wait_for(|s| s.cursor.is_none()))
                    .await
                    .expect("released within the wait")
                    .unwrap();
            }
        }
        // The rows read stay, flagged; the release says why.
        assert_eq!(o.rows(), 10_000, "{how}");
        assert_eq!(
            *o.snap(0).completeness(),
            closed(CloseReason::EnforcementGap),
            "{how}"
        );
        assert_eq!(
            o.releases(),
            vec![(CloseReason::EnforcementGap, false)],
            "{how}"
        );
        // The close, the end canary, the rollback, then nothing: discarded unreset.
        let all = steps(&f, base);
        let close = all
            .iter()
            .position(|s| matches!(s, Step::Close(_)))
            .unwrap_or_else(|| panic!("{how}: no close in {all:?}"));
        assert_eq!(
            all[close..],
            [
                Step::Close(CloseHow::Drain(ms(250))),
                Step::Canary,
                Step::Rollback
            ],
            "{how}"
        );
        loop {
            if next(&mut events).await == SessionEvent::EnforcementGap(GapReason::CanaryAssigned) {
                break;
            }
        }
        let mut state = h.state();
        tokio::time::timeout(
            secs(60),
            state.wait_for(|s| matches!(s.state, SessionState::Broken(_))),
        )
        .await
        .unwrap_or_else(|_| panic!("{how}: the session did not break"))
        .unwrap();
        assert!(!f.executed().iter().any(|s| s.contains("COMMIT")), "{how}");
    }
}

/// A suspended endless read-only cursor on `fake`, then a `FetchAll` on it, stopped once it
/// streams: returns the session, its fake, where its log stood, and the recorder.
async fn stopped_fetch(
    fake: FakeDriver,
    profile: Profile,
) -> (SessionHandle, Arc<FakeDriver>, usize, Arc<Recorder>) {
    let endless = FakeQuery {
        never_pending: true,
        ..select(BIG, u64::MAX)
    };
    let (h, f, base) = ro(fake.queries(vec![endless]), profile).await;
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    let t = h.submit(fetch_all(&o));
    settle_until("the fetch streams", || o.rows() > 20_000).await;
    assert_eq!(
        h.cancel(t.seq).await.unwrap(),
        hacivat_session::session::CancelOutcome::Cancelled
    );
    assert_eq!(
        t.reply.await.unwrap().unwrap_err().kind,
        ExecErrorKind::Cancelled
    );
    assert_eq!(*o.snap(0).completeness(), Completeness::Cancelled);
    (h, f, base, o)
}

#[tokio::test(start_paused = true)]
async fn a_stopped_read_only_pg_cursor_rolls_back_and_never_commits() {
    let (h, f, base, _o) = stopped_fetch(pg(clean(2)), ro_pg()).await;
    let all = steps(&f, base);
    let close = all
        .iter()
        .rposition(|s| matches!(s, Step::Close(_)))
        .expect("the stop closed the query");
    assert_eq!(
        all[close..],
        [
            Step::Close(CloseHow::Now),
            Step::Canary,
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
    assert_nothing_else_inside(&f, base);
    assert!(!f.executed().iter().any(|s| s.contains("COMMIT")));
    let s = snap(&h);
    assert_eq!((s.state, s.tx.status), (SessionState::Idle, TxStatus::Idle));
}

#[tokio::test(start_paused = true)]
async fn a_stopped_read_only_mysql_cursor_rolls_back_and_never_commits() {
    // MySQL closes early by a kill on the side connection; then the guard's end.
    let (h, f, base, _o) = stopped_fetch(mysql(), ro_mysql()).await;
    let all = steps(&f, base);
    let close = all
        .iter()
        .position(|s| *s == Step::Close(CloseHow::Now))
        .expect("the stop closed the query");
    assert_eq!(
        all[close..],
        [
            Step::Close(CloseHow::Now),
            Step::Close(CloseHow::AfterKill(secs(2))),
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
    assert_nothing_else_inside(&f, base);
    assert!(!f.executed().iter().any(|s| s.contains("COMMIT")));
    let s = snap(&h);
    assert_eq!((s.state, s.tx.status), (SessionState::Idle, TxStatus::Idle));
}

// MySQL

#[tokio::test(start_paused = true)]
async fn mysql_read_only_runs_reset_after_release() {
    // The release drains, kills on the side connection, reads the kill's answer; then the
    // guard's `ROLLBACK`, `COM_RESET_CONNECTION` and the replay. MySQL takes no statement
    // beside an open query: no canary.
    let (h, f, base) = ro(mysql().queries(vec![select(BIG, 30_000)]), ro_mysql()).await;
    let (o, reply) = run_one(&h, BIG, true).await;
    reply.expect("the guarded run succeeds");
    assert!(matches!(
        o.snap(0).completeness(),
        Completeness::Suspended(_)
    ));
    assert_eq!(snap(&h).tx.owner, TxOwner::Guard);
    ask(
        &h,
        Request::CloseCursor {
            result: o.store(0).id(),
            reason: CloseReason::UserClosed,
        },
    )
    .await
    .expect("the release runs");
    assert_eq!(*o.snap(0).completeness(), closed(CloseReason::UserClosed));
    let all = steps(&f, base);
    assert_eq!(all[..2], [Step::PlanBefore, Step::Query]);
    assert_eq!(
        all[all.len() - 5..],
        [
            Step::Close(CloseHow::Drain(ms(250))),
            Step::Close(CloseHow::AfterKill(secs(2))),
            Step::Rollback,
            Step::Reset,
            Step::Replay,
        ]
    );
    assert!(!all.contains(&Step::Canary));
    assert_nothing_else_inside(&f, base);
    let s = snap(&h);
    assert_eq!((s.state, s.tx.status), (SessionState::Idle, TxStatus::Idle));
}

#[tokio::test(start_paused = true)]
async fn mysql_write_succeeded_is_read_from_the_final_ok_packet() {
    // A positive affected-row count on a result with columns is `WriteSucceeded`. Review M5:
    // this is a fake-only shape, a tripwire. After a `SELECT` a real MySQL final OK packet
    // most likely carries 0 affected rows; the count is meaningful for `CALL` and statements
    // without columns. The server's read-only transaction refuses such a write first.
    let wrote = FakeQuery {
        rows_affected: Some(1),
        ..select("SELECT f()", 1)
    };
    let (h, f, base) = ro(mysql().queries(vec![wrote]), ro_mysql()).await;
    let (o, reply) = run_one(&h, "SELECT f()", true).await;
    assert_eq!(
        reply.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(GapReason::WriteSucceeded)
    );
    assert_eq!(o.rows(), 1);
    assert_eq!(
        *o.snap(0).completeness(),
        closed(CloseReason::EnforcementGap)
    );
    // The rollback ran (it undoes what the transaction can), the reset did not.
    let all = steps(&f, base);
    assert_eq!(all.last(), Some(&Step::Rollback), "{all:?}");
    assert!(matches!(snap(&h).state, SessionState::Broken(_)));
}

#[tokio::test(start_paused = true)]
async fn a_schema_write_behind_a_version_gated_comment_is_a_gap() {
    // T14 review m14: MySQL's command tag is read from the text, so a version-gated comment
    // can name another verb than the one a current server runs. Every reading counts. (A real
    // server refuses the `CREATE` first, 1792; the fake has no server, so this proves the
    // tripwire behind it, never that the write is stopped.)
    let (h, _f, _) = ro(mysql(), ro_mysql()).await;
    let (_, reply) = run_one(&h, "/*!99999 SELECT 1 */ CREATE TABLE x (a int)", false).await;
    assert_eq!(
        reply.unwrap_err().kind,
        ExecErrorKind::EnforcementGap(GapReason::WriteSucceeded)
    );
}

#[tokio::test(start_paused = true)]
async fn mysql_transaction_keywords_in_executable_comments_are_refused() {
    let (h, f, base) = ro(mysql(), ro_mysql()).await;
    for sql in [
        "/*!50000 COMMIT */",
        "/*!99999 SELECT 1 */ COMMIT",
        "# c\nROLLBACK",
    ] {
        let (_, reply) = run_one(&h, sql, false).await;
        assert_eq!(
            reply.unwrap_err().kind,
            ExecErrorKind::ReadOnlyDenied(DenyLayer::Transaction),
            "{sql:?}"
        );
    }
    assert_eq!(f.call_log().len(), base);
}

// ---------------------------------------------------------------------------------------------
// SQLite (the real driver)

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_authorizer_and_stmt_readonly_on_the_typed_path() {
    use common::conformance::{OnManager, deps, sqlite_profile};
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = dir.path().join("hv_t22.sqlite");
    let rw = OnManager::connect(deps(), sqlite_profile(&path)).await;
    for sql in [
        "CREATE TABLE t(a INTEGER)",
        "INSERT INTO t VALUES (1), (2), (3)",
    ] {
        run(&rw.s, sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {:?}", e.kind));
    }
    rw.shutdown().await;

    let before = std::fs::read(&path).expect("the database file");
    let mut p = sqlite_profile(&path);
    p.read_only = true;
    p.endpoint = hacivat_session::profile::ProfileEndpoint::File {
        path: path.clone(),
        create_if_missing: false,
    };
    let on = OnManager::connect(deps(), p).await;
    assert_eq!(on.profile.engine, DbmsKind::Sqlite);
    assert!(snap(&on.s).read_only);
    // A read runs on the typed path, as a cursor too.
    let (o, reply) = run_one(&on.s, "SELECT a FROM t ORDER BY a", true).await;
    reply.expect("the read runs");
    assert_eq!(o.rows(), 3);
    assert_eq!(*o.snap(0).completeness(), Completeness::Complete);
    // The M2b SQLite rows (docker_m2b_bypass.rs) through `Run`, suspendable or not: the
    // read-only open, the hardening, the authorizer and `stmt_readonly` refuse each one, and
    // a statement SQLite cannot prepare is refused too; the session runs on after each.
    let d = dir.path().display();
    let rows = [
        "INSERT INTO t VALUES (2)".to_string(),
        "DELETE FROM t".to_string(),
        format!("ATTACH '{d}/n.db' AS n"),
        format!("VACUUM INTO '{d}/c.db'"),
        "PRAGMA query_only = 0".to_string(),
        "CREATE TEMP TABLE x (a)".to_string(),
        format!("SELECT writefile('{d}/w', 'x')"),
        "SELECT load_extension('hv_t22')".to_string(),
        "PRAGMA writable_schema = 1".to_string(),
        "PRAGMA user_version = 7".to_string(),
        "PRAGMA journal_mode = WAL".to_string(),
        "BEGIN IMMEDIATE".to_string(),
        "SELEC 1".to_string(),
    ];
    for sql in &rows {
        for suspendable in [false, true] {
            let (_, reply) = run_one(&on.s, sql, suspendable).await;
            assert!(reply.is_err(), "`{sql}` ran");
            let (o, read) = run_one(&on.s, "SELECT count(*) FROM t", false).await;
            read.unwrap_or_else(|e| panic!("after `{sql}`: {:?}", e.kind));
            assert_eq!(
                o.snap(0).cell(0, 0).to_value(),
                hacivat_values::Value::Int(3),
                "after `{sql}`"
            );
        }
    }
    on.shutdown().await;
    // The side channel: no file was created, and the database is byte-identical.
    let mut files: Vec<String> = std::fs::read_dir(dir.path())
        .expect("the directory")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    files.sort();
    assert_eq!(files, ["hv_t22.sqlite"]);
    assert_eq!(std::fs::read(&path).expect("the database file"), before);
}
