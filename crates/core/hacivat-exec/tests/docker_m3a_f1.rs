//! Follow-up F1 (M3a) over the Docker matrix. Run with
//! `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//! docker_m3a_f1`.
//!
//! - F1-c: a production profile connects with a bound console on every server line
//!   (PostgreSQL, MySQL, MariaDB of the active set). The console binds as the app's does
//!   (`Console::open`: connect under the app's production policy, then the SQL-context probe as
//!   an internal statement, `exec_ui::context`); the probe reads the server's value on the
//!   read-only session, and the profile and the session stay up for the user's statements,
//!   which the guard still runs.
//! - F1-a: PostgreSQL rejects `)COPY t FROM STDIN` as a syntax error (42601) and takes the next
//!   statement normally, so it never entered `COPY` mode: the lines after it are never read
//!   as `COPY` data.
//!
//! The exec path stands in for the app's: an app-level Docker test would need the GPUI app over
//! a real core, which no test target builds (the app's tests run over `FakeDriver`).

mod common;

use common::matrix::*;
use hacivat_exec::{console_ux, sqlctx};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::LiveState;
use hacivat_session::session::SessionState;

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
                production_policy: true,
                ..Opts::default()
            },
        )
        .await;
        let ux = console_ux(Some(EnvTag::Production), true, &prod.snap(), None, None);
        assert!(
            ux.production && ux.read_only && ux.forced_read_only,
            "{id}: {ux:?}"
        );
        assert!(prod.snap().read_only, "{id}: a read-only session");

        // The bound console's probe read the server's value (not the defaults), on the
        // read-only session.
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

        // The profile and its session are up, and the user's statements run (guarded).
        assert!(
            matches!(prod.m.state(prod.profile.id), LiveState::Connected(_)),
            "{id}: {:?}",
            prod.m.state(prod.profile.id)
        );
        assert_eq!(prod.snap().state, SessionState::Idle, "{id}");
        prod.ok("SELECT 1").await;
        let refused = prod.run("CREATE TABLE hv_f1_never (a int)").await;
        assert_eq!(refused.names(), ["Failed"], "{id}: the guard still refuses");
        // ## m3a-f5-demo (DEMO-7): refused by the console, before anything is sent.
        assert_eq!(refused.outcome(), "input", "{id}");
        assert!(
            matches!(prod.m.state(prod.profile.id), LiveState::Connected(_)),
            "{id}: {:?}",
            prod.m.state(prod.profile.id)
        );
        record.add(
            line,
            prod.version.as_ref(),
            "production connect, bound console probe",
            &format!("{value:?}, write {}", refused.outcome()),
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
