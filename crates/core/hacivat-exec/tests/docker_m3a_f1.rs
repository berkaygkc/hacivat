//! Follow-up F1 (M3a) over the Docker matrix. Run with
//! `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//! docker_m3a_f1`.
//!
//! - F1-c: a production profile connects with a bound console on every server line
//!   (PostgreSQL, MySQL, MariaDB of the active set). The console binds as the app's does
//!   (`Console::open`: connect under the app's configuration, then the SQL-context probe as
//!   an internal statement, `exec_ui::context`); the probe reads the server's value on the
//!   session, and the profile and the session stay up for the user's statements.
//!   ## m3b-t26-prod-unlock: the session is read-write in Manual mode (D-036), and a write goes
//!   through the gate (`common::t28`): it is confirmed before it is sent, and a confirmation
//!   nobody answered sends nothing (before Task 26 the read-only guard refused it).
//! - F1-a: PostgreSQL rejects `)COPY t FROM STDIN` as a syntax error (42601) and takes the next
//!   statement normally, so it never entered `COPY` mode: the lines after it are never read
//!   as `COPY` data.
//!
//! The exec path stands in for the app's: an app-level Docker test would need the GPUI app over
//! a real core, which no test target builds (the app's tests run over `FakeDriver`).

mod common;

use common::matrix::*;
// ## m3b-t26-prod-unlock
use common::t28::{Con, Spec};
use hacivat_exec::classify_pool::ClassifyPool;
use hacivat_exec::{ErrorPolicy, console_ux, sqlctx};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::LiveState;
use hacivat_session::session::SessionState;
use hacivat_session::session::TxMode;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn a_production_profile_connects_with_a_bound_console_on_every_server_line() {
    if !docker_enabled() {
        return;
    }
    let record = Record::new();
    let lines = server_lines();
    for line in &lines {
        let id = &line.id;
        let prod = Console::open(
            line,
            Opts {
                env: Some(EnvTag::Production),
                ..Opts::default()
            },
        )
        .await;
        let ux = console_ux(Some(EnvTag::Production), &prod.snap(), None, None);
        // ## m3b-t26-prod-unlock: a read-write session in Manual mode (D-036).
        assert!(ux.production && !ux.read_only, "{id}: {ux:?}");
        assert!(!prod.snap().read_only, "{id}: a read-write session");
        assert_eq!(prod.snap().tx.mode, TxMode::Manual, "{id}");

        // The bound console's probe read the server's value (not the defaults), on the
        // session.
        let probe = sqlctx::probe_sql(line.engine).expect("a server line has a probe");
        let value = prod
            .internal(probe)
            .await
            .unwrap_or_else(|| panic!("{id}: the probe read no value"));
        if line.engine == DbmsKind::Postgres {
            assert_eq!(value, "on", "{id}");
        }
        assert_eq!(
            prod.ctx,
            sqlctx::dialect_ctx(line.engine, prod.version.as_ref(), Some(&value)),
            "{id}: the console's context is the probe's"
        );

        // The profile and its session are up, and the user's statements run.
        assert!(
            matches!(prod.m.state(prod.profile.id), LiveState::Connected(_)),
            "{id}: {:?}",
            prod.m.state(prod.profile.id)
        );
        assert_eq!(prod.snap().state, SessionState::Idle, "{id}");
        prod.ok("SELECT 1").await;
        // ## m3b-t26-prod-unlock: a write meets the gate's confirmation; unanswered, the run
        // service sends nothing, and the server never sees it.
        let pool = ClassifyPool::new(2);
        let con = Con::open(line, Spec::production(), &pool).await;
        let planned = con.plan("CREATE TABLE hv_f1_never (a int)", &[]).await;
        assert_eq!(planned.shapes(), ["confirm"], "{id}: asked first");
        let refused = ended(con.send(&planned, ErrorPolicy::StopOnError, None)).await;
        assert_eq!(refused.names(), ["Skipped"], "{id}: not sent unanswered");
        let mut side = admin(line).await;
        assert!(
            side.try_batch("SELECT 1 FROM hv_f1_never").await.is_err(),
            "{id}: the table was never created"
        );
        side.close().await;
        con.shutdown().await;
        assert!(
            matches!(prod.m.state(prod.profile.id), LiveState::Connected(_)),
            "{id}: {:?}",
            prod.m.state(prod.profile.id)
        );
        record.add(
            line,
            prod.version.as_ref(),
            "production connect, bound console probe",
            &format!("{value:?}, write {} unanswered", planned.shapes()[0]),
        );
        prod.shutdown().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("f1-production-connect");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn a_parenthesis_before_copy_is_a_syntax_error_on_postgresql() {
    if !docker_enabled() {
        return;
    }
    let record = Record::new();
    let lines = pg_lines();
    for line in &lines {
        let id = &line.id;
        let dev = Console::dev(line).await;
        // What Run sends for the seed's statement (the splitter's plain statement).
        let ran = dev.run(")COPY t FROM STDIN;").await;
        assert_eq!(ran.names(), ["Failed"], "{id}");
        assert_eq!(ran.outcome(), "42601", "{id}: a syntax error at `)`");
        // Not in COPY mode: the next statement, and a "row" sent as SQL, are statements.
        // The run's end can reach the test before the session publishes `Idle`: wait for it.
        assert_eq!(dev.settled().await.state, SessionState::Idle, "{id}");
        dev.ok("SELECT 1").await;
        let row = dev.run("a\tb").await;
        assert_eq!(row.outcome(), "42601", "{id}: a row is SQL to the server");
        record.add(
            line,
            dev.version.as_ref(),
            ")COPY t FROM STDIN",
            &ran.outcome(),
        );
        dev.shutdown().await;
    }
    assert_every_line_ran(&record, &lines);
    record.write("f1-paren-copy");
}
