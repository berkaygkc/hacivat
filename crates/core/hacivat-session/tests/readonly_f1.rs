//! Follow-up F1-c (M3a; ADR 0004 `m3a-f1`): Hacivat's SQL-context probe on a read-only session
//! the read-only guard cannot verify. A connect binds a console and the console probes its
//! context at once, so a probe that broke the session would take every production connect of
//! such a server to `Lost { Internal }`. The probe, by its exact constant text and internal
//! origin, runs outside the guard (as the schema probe does); user statements, and any other
//! internal text, stay guarded and fail closed.
//!
//! The fakes here answer the guard's `BEGIN … READ ONLY` but never its canary
//! (`txid_current_if_assigned()` is missing, as on a PostgreSQL-wire server without it).

mod common;

use std::time::Duration;

use common::*;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_driver::{ExecOpts, ExecSummary, TxStatus};
use hacivat_net::TlsMode;
use hacivat_proto::DbmsKind;
use hacivat_session::session::{ExecOrigin, Request, SessionHandle, SessionState};

const CANARY: &str = "SELECT pg_catalog.txid_current_if_assigned()";

fn entry(prefix: &'static str, result: Result<ExecSummary, hacivat_driver::RawError>) -> FakeExec {
    FakeExec {
        prefix,
        result,
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// `prefix` answers one row holding `value`.
fn value(prefix: &'static str, value: &str) -> FakeExec {
    entry(
        prefix,
        Ok(ExecSummary {
            rows: vec![vec![Some(value.to_string())]],
            row_count: 1,
            ..ExecSummary::default()
        }),
    )
}

/// A PostgreSQL server whose canary function does not exist (42883), every time.
fn pg_without_canary() -> FakeDriver {
    let mut script = vec![value("SHOW standard_conforming_strings", "on")];
    script.extend((0..4).map(|_| entry(CANARY, Err(db("42883")))));
    fake_pg().tracks_transactions().exec(script)
}

fn internal(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::values(1),
        origin: ExecOrigin::Internal { idempotent: true },
    }
}

fn user(sql: &str) -> Request {
    Request::Exec {
        sql: sql.to_string(),
        opts: ExecOpts::default(),
        origin: ExecOrigin::User,
    }
}

fn state(s: &SessionHandle) -> SessionState {
    s.state().borrow().state.clone()
}

async fn ro_pg(fake: FakeDriver) -> SessionHandle {
    let (s, _) = ro_session(fake, pg_profile(TlsMode::Disable)).await;
    assert!(s.state().borrow().read_only);
    s
}

/// The ruling's test: the probe answers its value and the session stays usable.
#[tokio::test]
async fn the_context_probe_never_breaks_a_session_the_guard_cannot_verify() {
    let s = ro_pg(pg_without_canary()).await;
    let sql = "SHOW standard_conforming_strings";
    let answer = s
        .submit(internal(sql))
        .reply
        .await
        .unwrap()
        .expect("the probe answers");
    assert_eq!(answer.rows, [[Some("on".to_string())]]);
    assert_eq!(state(&s), SessionState::Idle);
    assert_eq!(s.state().borrow().tx.status, TxStatus::Idle);
}

/// The guard stays fail-closed for user statements: the same server breaks the session on
/// the first one (the canary cannot be read), the probe's own text included.
#[tokio::test]
async fn a_user_statement_the_guard_cannot_verify_still_breaks_the_session() {
    for sql in ["SELECT 1", "SHOW standard_conforming_strings"] {
        let s = ro_pg(pg_without_canary()).await;
        let reply = s.submit(user(sql)).reply.await.unwrap();
        assert!(reply.is_err(), "{sql}: {reply:?}");
        assert!(
            matches!(state(&s), SessionState::Broken(_)),
            "{sql}: {:?}",
            state(&s)
        );
    }
}

/// Only the probe's exact text: any other internal statement stays guarded.
#[tokio::test]
async fn other_internal_statements_stay_guarded() {
    for sql in [
        "SELECT 1",
        "SHOW standard_conforming_strings; SELECT 1",
        "show standard_conforming_strings",
        "SHOW standard_conforming_strings ",
    ] {
        let s = ro_pg(pg_without_canary()).await;
        let reply = s.submit(internal(sql)).reply.await.unwrap();
        assert!(reply.is_err(), "{sql}: {reply:?}");
        assert!(
            matches!(state(&s), SessionState::Broken(_)),
            "{sql}: {:?}",
            state(&s)
        );
    }
}

/// MySQL and MariaDB: the guard's extended protocol carries no values, so the probe read
/// nothing on a read-only session before; outside the guard it reads the `sql_mode`.
#[tokio::test]
async fn a_read_only_mysql_session_answers_its_sql_mode() {
    for kind in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let sql = "SELECT @@SESSION.sql_mode";
        let fake = FakeDriver::new(kind)
            .tracks_transactions()
            .exec(vec![value("SELECT @@SESSION.sql_mode", "ANSI_QUOTES")]);
        let profile = match kind {
            DbmsKind::MySql => mysql_profile(TlsMode::Disable),
            _ => mariadb_profile(TlsMode::Disable),
        };
        let (s, _) = ro_session(fake, profile).await;
        let answer = s
            .submit(internal(sql))
            .reply
            .await
            .unwrap()
            .expect("answers");
        assert_eq!(answer.rows, [[Some("ANSI_QUOTES".to_string())]], "{kind:?}");
        assert_eq!(state(&s), SessionState::Idle);
    }
}
