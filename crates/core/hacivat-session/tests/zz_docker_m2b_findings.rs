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

use common::conformance::*;
use hacivat_proto::{AccessLabel, DbmsKind};
use hacivat_session::LiveState;
use hacivat_session::session::{ExecErrorKind, SessionKind, SessionState};
use hacivat_testkit::{DockerTarget, targets};

/// `t`'s read-only profile as `user`, on its own M2b manager.
async fn read_only_as(t: &DockerTarget, user: &str) -> OnManager {
    let mut p = profile_for(t, auto_or_require(t)).user(user);
    p.read_only = true;
    p.allow_unsupported_server = true;
    OnManager::connect(deps(), p).await
}

fn label(on: &OnManager) -> Option<AccessLabel> {
    match on.m.state(on.id()) {
        LiveState::Connected(info) => Some(info.access),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn f1_pg_enforced_role_cannot_write_wal_messages() {
    // F1 (SQL-SAFE-06 P32; owner: the role probe and remediation, T5/T18). PostgreSQL 13–18
    // grant EXECUTE on `pg_logical_emit_message` to PUBLIC. Its transactional form assigns a
    // transaction ID (L6's canary: a gap, rolled back); the non-transactional one writes its
    // WAL record at once, which no rollback undoes and no canary sees. `hv_ro` (least privilege
    // by the matrix's grants) is labelled "enforced" and can do it: the label claims more than
    // the server enforces (B21). Either the probe must not read such a role as enforced, or the
    // role must be refused (`42501`, e.g. by the remediation's REVOKE).
    if !docker_enabled() {
        return;
    }
    let lines: Vec<DockerTarget> = targets(Some(DbmsKind::Postgres))
        .into_iter()
        .filter(|t| !t.id.contains('-'))
        .collect();
    let mut failures = Vec::new();
    for t in lines {
        let on = read_only_as(&t, "hv_ro").await;
        let label = label(&on);
        let reply = run(
            &on.s,
            "SELECT pg_logical_emit_message(false, 'hv_t23', 'x')",
        )
        .await;
        let refused = matches!(
            &reply,
            Err(e) if e.kind == ExecErrorKind::Db
                && code_of(e.server.as_ref()).as_deref() == Some("42501")
        );
        if label == Some(AccessLabel::ReadOnlyEnforced) && !refused {
            failures.push(format!(
                "{}: hv_ro is labelled enforced, and pg_logical_emit_message(false, …) {}",
                t.id,
                match &reply {
                    Ok(_) => "wrote its WAL record".to_string(),
                    Err(e) => format!("failed with {:?}", e.kind),
                }
            ));
        }
        on.shutdown().await;
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
