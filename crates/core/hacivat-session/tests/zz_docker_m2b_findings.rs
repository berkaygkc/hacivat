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
use hacivat_session::session::ExecErrorKind;
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
