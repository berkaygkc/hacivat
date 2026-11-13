//! Task 23's findings against the Docker matrix that are product defects of earlier tasks:
//! each test states the behaviour the product owes and fails until its owner fixes it (the
//! controller routes them). Run with `cargo xtask docker test --set full -p hacivat-session`.
//!
//! The file name sorts last on purpose: cargo stops at the first test binary that fails, and a
//! known-red finding must never hide the rest of the Docker suite.

#![allow(
    clippy::result_large_err,
    reason = "ConnectError is the pipeline's fixed interface"
)]

mod common;

// ## m2b-t23-f1: F1 (an enforced PostgreSQL role could write WAL messages) is fixed; its rows
// live in `docker_m2b_bypass.rs` (`PG_NON_TRANSACTIONAL_TEXT`, P31/P32).

use common::conformance::*;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{ExecErrorKind, SessionKind, SessionState};
use hacivat_testkit::targets;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn f2_mysql_refused_set_global_is_a_refusal_not_a_lost_connection() {
    // F2 (owner: the MySQL driver, T13, and the vendored `mysql_async`). On MySQL 8.0 and 8.4 an
    // account without SYSTEM_VARIABLES_ADMIN that sends `SET GLOBAL …` (or `SET PERSIST …`)
    // through the prepared-statement protocol — every user statement of a session — loses its
    // connection: the client reads EOF and the server counts an aborted client. The same text
    // over `COM_QUERY` is answered `1227`, and so is every other privilege refusal seen (`SET
    // SESSION sql_log_bin`, `FLUSH TABLES`, a `SELECT` on `mysql.user`); MariaDB answers `1227`
    // in both protocols. The user is told "connection lost, outcome unknown" for a statement the
    // server refused, and the session reconnects; nothing changed on the server (the bypass
    // suite's side channel). The session owes the refusal, with the server's message, on a
    // connection that stays up.
    if !docker_enabled() {
        return;
    }
    let mut failures = Vec::new();
    for t in targets(Some(DbmsKind::MySql)) {
        for read_only in [false, true] {
            let mut p = profile_for(&t, auto_or_require(&t)).user("hv_ro");
            p.read_only = read_only;
            let on = OnManager::connect(deps(), p).await;
            for sql in [
                "SET GLOBAL max_connections = 999",
                "SET PERSIST max_connections = 999",
            ] {
                let reply = run(&on.session(), sql).await;
                let refused = matches!(
                    &reply,
                    Err(e) if e.kind == ExecErrorKind::Db
                        && code_of(e.server.as_ref()).as_deref() == Some("1227")
                );
                if !refused {
                    failures.push(format!(
                        "{} read_only={read_only} `{sql}`: {:?}",
                        t.id,
                        reply.map(|s| s.row_count).map_err(|e| e.kind)
                    ));
                }
                // The next statement waits for the session to come back.
                wait_connected_within(&on.m, on.id(), CONNECT_LIMIT).await;
            }
            on.shutdown().await;
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn f3_pg_released_pooled_session_is_parked_and_reused() {
    // F3 (owner: the session reset, T7, and the pool, T15). A pooled session goes back to the
    // pool through `Request::Reset`, whose PostgreSQL step 3 is the verify probe
    // `SELECT current_user = session_user`, run with `ExecOpts::default()` — the extended
    // protocol, whose rows the PostgreSQL driver counts without decoding. The probe therefore
    // never reads `t`: "the reset's role check did not answer true", the session breaks and is
    // closed, and every borrow opens a new connection (the pool never parks on PostgreSQL; the
    // T15 tests script the probe's answer, so they pass). MySQL has no verify probe and parks.
    if !docker_enabled() {
        return;
    }
    let Some(t) = hacivat_testkit::require_target("pg17") else {
        return;
    };
    let on = session_on(&t, Path::Direct).await;
    let sessions = on.m.sessions(on.id()).expect("connected");
    let kind = SessionKind::Introspection { database: None };
    let first = sessions
        .pooled(kind.clone())
        .await
        .expect("a pooled session");
    assert!(run(&first, "SELECT 1").await.is_ok());
    let released = (*first).clone();
    let from = released.state().borrow().generation;
    drop(first);
    // The release's reset ends either way: parked (idle) or broken and closed.
    wait_until("the release's reset", secs(10), || {
        let now = released.state().borrow().clone();
        now.generation >= from + 2 && !matches!(now.state, SessionState::Busy { .. })
    })
    .await;
    let state = released.state().borrow().state.clone();
    let again = sessions.pooled(kind).await.expect("a pooled session");
    let reused = again.id() == released.id();
    drop(again);
    on.shutdown().await;
    assert!(
        reused,
        "pg17: the released session was not reused; after its reset it was {state:?}"
    );
}
