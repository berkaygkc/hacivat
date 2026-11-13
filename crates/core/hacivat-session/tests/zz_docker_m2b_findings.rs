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
use hacivat_session::session::ExecErrorKind;
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
