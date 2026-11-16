//! M3b Task 11 over Docker: what a run sends and keeps on real servers. PostgreSQL binds its
//! values as text-format parameters (R-M3-79) and reports a `RAISE NOTICE`'s text as a run
//! event (R-M3-77); a MySQL `CALL` with three result sets gives three results (R-M3-66); a
//! SQLite batch runs "as one" (R-M3-60). Run with `<main>/target/docker/run-docker-tests.sh
//! --set pr -p hacivat-session --test docker_m3b_run`.
//!
//! - Targets come only from `require_target` (Q6). Fixtures have names unique to the run and
//!   are dropped at the end (Q7). The SQLite leg needs no server and runs in a temp directory.
//! - The drivers' halves (binds, the simple protocol, notice text) are Task 13's: on a tree
//!   without them this test fails on those legs, by design (no silent skip).

mod common;

use common::conformance::*;
use common::runs::*;
use common::runs_m3b::*;
use hacivat_driver::Protocol;
use hacivat_results::Completeness;
use hacivat_session::session::{RunEvent, RunUnit};
use hacivat_testkit::require_target;
use hacivat_values::{BindValue, LogicalType, Value};

fn int(text: &str) -> BindValue {
    BindValue::Typed {
        hint: LogicalType::Int {
            bits: 32,
            signed: true,
        },
        text: text.into(),
    }
}

fn many(sql: &str, protocol: Protocol) -> RunUnit {
    let mut u = unit(0, sql);
    u.protocol = protocol;
    u.max_result_sets = 100;
    u
}

fn cell(o: &Recorder, result: usize, row: u64, col: u32) -> Value {
    o.snap(result).cell(row, col).to_value()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn pg_mysql_sqlite_binds_and_multi_results_on_the_matrix() {
    if !docker_enabled() {
        return;
    }

    // PostgreSQL: two binds in slot order, text format; the server parses them by type.
    if let Some(t) = require_target("pg17") {
        let on = session_on(&t, Path::Direct).await;
        let o = Recorder::new();
        let hostile = "x'); DROP TABLE t; --";
        let mut u = unit(0, "SELECT $1::int4 + 1 AS a, $2::text AS b, $3::text AS c");
        u.binds = vec![int("41"), BindValue::Null, text(hostile)].into();
        run_spec(&on.s, spec_units(&o, vec![u]))
            .await
            .unwrap_or_else(|e| panic!("pg17 binds: {:?}", e.kind));
        assert_eq!(cell(&o, 0, 0, 0), Value::Int(42), "pg17");
        assert_eq!(cell(&o, 0, 0, 1), Value::Null, "pg17");
        assert_eq!(cell(&o, 0, 0, 2), Value::Text(hostile.into()), "pg17");

        // RAISE NOTICE: its text reaches the run's observer.
        let o = Recorder::new();
        let marker = unique("hv_t11_notice");
        let sql = format!("DO $$ BEGIN RAISE NOTICE '{marker}'; END $$");
        run_spec(&on.s, spec_units(&o, vec![unit(0, &sql)]))
            .await
            .unwrap_or_else(|e| panic!("pg17 notice: {:?}", e.kind));
        assert_eq!(notice_texts(&o), vec![marker], "pg17");
        on.shutdown().await;
    }

    // MySQL: a procedure with three result sets gives three results.
    if let Some(t) = require_target("mysql84") {
        let proc = unique("hv_t11_sets");
        let mut admin = admin(&t).await;
        batch(
            &mut admin,
            "mysql84",
            &format!(
                "CREATE PROCEDURE {proc}() BEGIN SELECT 1 AS a; SELECT 2 AS b; SELECT 3 AS c; END"
            ),
        )
        .await;
        let on = session_on(&t, Path::Direct).await;
        let o = Recorder::new();
        let ran = run_spec(
            &on.s,
            spec_units(
                &o,
                vec![many(&format!("CALL {proc}()"), Protocol::Extended)],
            ),
        )
        .await;
        batch(&mut admin, "mysql84", &format!("DROP PROCEDURE {proc}")).await;
        admin.close().await;
        ran.unwrap_or_else(|e| panic!("mysql84 CALL: {:?}", e.kind));
        assert_eq!(opened(&o), vec![(0, 0), (0, 1), (0, 2)], "mysql84");
        for (i, v) in [1, 2, 3].into_iter().enumerate() {
            assert_eq!(cell(&o, i, 0, 0), Value::Int(v), "mysql84 set {i}");
        }
        assert!(
            !o.events()
                .iter()
                .any(|e| matches!(e, RunEvent::MoreResultSets { .. })),
            "mysql84"
        );
        assert_eq!(snap_cursor(&on), None, "a multi-result unit never suspends");
        on.shutdown().await;
    }

    // SQLite: a batch run as one; its read is the last set.
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = dir.path().join("hv_t11.sqlite");
    let on = OnManager::connect(deps(), sqlite_profile(&path)).await;
    let o = Recorder::new();
    let sql =
        "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1), (2); SELECT a FROM t ORDER BY a";
    run_spec(&on.s, spec_units(&o, vec![many(sql, Protocol::Simple)]))
        .await
        .unwrap_or_else(|e| panic!("sqlite as one: {:?}", e.kind));
    let reads: Vec<usize> = (0..o.stores())
        .filter(|&i| o.snap(i).row_count() == 2)
        .collect();
    assert_eq!(reads.len(), 1, "sqlite: one read result");
    assert_eq!(cell(&o, reads[0], 1, 0), Value::Int(2), "sqlite");
    assert!(
        o.result_ends().iter().all(|c| *c == Completeness::Complete),
        "sqlite: {:?}",
        o.result_ends()
    );
    on.shutdown().await;
}

fn snap_cursor(on: &OnManager) -> Option<hacivat_session::session::CursorBadge> {
    on.s.state().borrow().cursor
}
