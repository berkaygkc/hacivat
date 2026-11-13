//! Guard plans per engine (SQL-SAFE-05; Q8: Task 1 ships `plan_for`, Task 5 tests its values).

use hacivat_guard::{ResetKind, plan_for};
use hacivat_proto::DbmsKind;

#[test]
fn pg_plan_matches_safe_05() {
    let p = plan_for(DbmsKind::Postgres).unwrap();
    assert_eq!(
        p.before,
        [
            "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "SELECT 1"
        ]
    );
    // T18 review m-3: qualified, so `search_path` cannot shadow it.
    assert_eq!(
        p.canary,
        Some("SELECT pg_catalog.txid_current_if_assigned()")
    );
    assert_eq!(p.after, ["ROLLBACK"]);
    assert_eq!(p.reset, ResetKind::DiscardAll);
    assert!(
        p.session_setup
            .contains(&"SET default_transaction_read_only = on")
    );
}

#[test]
fn pg_session_setup_is_exact() {
    let p = plan_for(DbmsKind::Postgres).unwrap();
    assert_eq!(
        p.session_setup,
        [
            "SET default_transaction_read_only = on",
            "SET standard_conforming_strings = on",
            "SET client_encoding = 'UTF8'",
            "SET idle_in_transaction_session_timeout = '30s'",
        ]
    );
}

#[test]
fn mysql_and_mariadb_plans() {
    for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let p = plan_for(k).unwrap();
        assert_eq!(p.session_setup[0], "SET SESSION TRANSACTION READ ONLY");
        assert_eq!(p.before, ["START TRANSACTION READ ONLY"]);
        assert_eq!(p.reset, ResetKind::ResetConnection);
    }
}

#[test]
fn mysql_and_mariadb_plans_are_exact() {
    for k in [DbmsKind::MySql, DbmsKind::MariaDb] {
        let p = plan_for(k).unwrap();
        assert_eq!(
            p.session_setup,
            ["SET SESSION TRANSACTION READ ONLY", "SET NAMES utf8mb4"],
            "{k:?}"
        );
        assert_eq!(p.canary, None, "{k:?}");
        assert_eq!(p.after, ["ROLLBACK"], "{k:?}");
    }
}

#[test]
fn sqlite_plan_is_connection_level() {
    let p = plan_for(DbmsKind::Sqlite).unwrap();
    assert!(p.session_setup.is_empty() && p.before.is_empty() && p.after.is_empty());
    assert_eq!(p.canary, None);
    assert_eq!(p.reset, ResetKind::None);
}

#[test]
fn engines_without_a_guard_have_no_plan() {
    for k in [
        DbmsKind::Redshift,
        DbmsKind::Cockroach,
        DbmsKind::TiDb,
        DbmsKind::MsSql,
        DbmsKind::ClickHouse,
        DbmsKind::Redis,
        DbmsKind::MongoDb,
    ] {
        assert!(plan_for(k).is_none(), "{k:?}");
    }
}
