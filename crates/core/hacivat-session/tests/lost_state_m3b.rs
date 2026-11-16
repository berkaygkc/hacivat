//! M3b Task 18, C8: what a reconnect would lose is read from the classifier's session effects
//! of each unit when the exec gate classified it (`RunUnit.effects`), with M2b's first-keyword
//! scan as the fallback for a unit without a verdict (it errs toward reporting more).

use hacivat_session::session::{SessionEffects, Tracked, lost_state, track_statement};

fn after(units: &[(&str, Option<SessionEffects>)]) -> Tracked {
    let mut t = Tracked::default();
    for (sql, effects) in units {
        track_statement(&mut t, sql, *effects);
    }
    t
}

#[test]
fn lost_state_from_effects_and_scan_fallback() {
    // The classifier sees what no first keyword shows: a temporary table created by `SELECT …
    // INTO TEMP`, a session lock taken inside a CTE, a variable set by a function call.
    let t = after(&[
        (
            "SELECT * INTO TEMP hv_tmp FROM t",
            Some(SessionEffects::WRITE_TEMP | SessionEffects::OPENS_WRITE),
        ),
        (
            "WITH l AS (SELECT pg_advisory_lock(1)) SELECT * FROM l",
            Some(SessionEffects::ADVISORY_LOCK),
        ),
        (
            "SELECT set_config('work_mem', '64MB', false)",
            Some(SessionEffects::SESSION_STATE),
        ),
    ]);
    let lost = lost_state(&t);
    assert!(
        lost.temp_objects && lost.locks && lost.session_vars,
        "{lost:?}"
    );
    assert_eq!(lost.prepared, 0);
    // The scan alone reads none of them (its first keyword is SELECT, WITH, SELECT).
    let scan = after(&[
        ("SELECT * INTO TEMP hv_tmp FROM t", None),
        ("SELECT set_config('work_mem', '64MB', false)", None),
    ]);
    assert!(lost_state(&scan).is_empty(), "{:?}", lost_state(&scan));

    // Effects that leave nothing behind leave nothing, whatever the first keyword says: a
    // `SET LOCAL` the scan would not count either, a `LOCK TABLE` on PostgreSQL (ends with the
    // transaction), a plain read.
    let t = after(&[
        (
            "SET LOCAL statement_timeout = 0",
            Some(SessionEffects::empty()),
        ),
        ("LOCK TABLE t IN SHARE MODE", Some(SessionEffects::empty())),
        ("SELECT 1", Some(SessionEffects::empty())),
    ]);
    assert!(lost_state(&t).is_empty(), "{:?}", lost_state(&t));
    // MySQL's LOCK TABLES is a session lock.
    let t = after(&[("LOCK TABLES t WRITE", Some(SessionEffects::LOCK_TABLES))]);
    assert!(lost_state(&t).locks);

    // Prepared statements are counted; DEALLOCATE (also PREPARE-effect) lowers the count.
    let t = after(&[
        ("PREPARE a AS SELECT 1", Some(SessionEffects::PREPARE)),
        ("PREPARE b AS SELECT 2", Some(SessionEffects::PREPARE)),
        ("DEALLOCATE a", Some(SessionEffects::PREPARE)),
    ]);
    assert_eq!(lost_state(&t).prepared, 1);
    let t = after(&[
        ("PREPARE a AS SELECT 1", Some(SessionEffects::PREPARE)),
        ("DEALLOCATE PREPARE ALL", Some(SessionEffects::PREPARE)),
    ]);
    assert_eq!(lost_state(&t).prepared, 0);

    // Without effects (no verdict, or an Unknown one) the scan decides, as in M2b.
    let t = after(&[
        ("CREATE TEMP TABLE x (a int)", None),
        ("SET search_path = app", None),
        ("SELECT pg_advisory_lock(7)", None),
        ("PREPARE s AS SELECT 1", None),
    ]);
    let lost = lost_state(&t);
    assert!(
        lost.temp_objects && lost.session_vars && lost.locks,
        "{lost:?}"
    );
    assert_eq!(lost.prepared, 1);

    // A flag once on stays on: a later read does not clear it.
    let t = after(&[
        ("SET search_path = app", Some(SessionEffects::SESSION_STATE)),
        ("SELECT 1", Some(SessionEffects::empty())),
        ("SELECT 2", None),
    ]);
    assert!(lost_state(&t).session_vars);
}
