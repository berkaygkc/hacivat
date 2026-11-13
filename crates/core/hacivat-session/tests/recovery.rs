//! Recovery after a lost connection (HV-CONN-05, D-085, B19; Task 17): the backoff schedule,
//! which failures reconnect, and what a reconnect loses.

use std::time::Duration;

use hacivat_driver::TxStatus;
use hacivat_proto::ErrClass;
use hacivat_session::session::{
    LostState, RecoveryPolicy, Tracked, is_recovery_case, lost_state, track_statement,
};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn backoff_schedule_with_jitter_bounds() {
    let p = RecoveryPolicy::default();
    let mut lo = || 0.0f32;
    let mut hi = || 1.0f32;
    for (i, base) in [0u64, 1, 2, 4, 8, 30, 30].iter().enumerate() {
        let d =
            |r: &mut dyn FnMut() -> f32| p.delay(i as u32, secs(0), r).unwrap().as_millis() as f64;
        assert!((d(&mut lo) - *base as f64 * 800.0).abs() < 1.0);
        assert!((d(&mut hi) - *base as f64 * 1200.0).abs() < 1.0);
    }
    assert!(p.delay(20, secs(301), &mut lo).is_none());
}

#[test]
fn recovery_cases() {
    assert!(is_recovery_case(ErrClass::NetworkReset));
    assert!(is_recovery_case(ErrClass::ServerShutdown));
    assert!(!is_recovery_case(ErrClass::AuthInvalid));
}

#[test]
fn lost_state_from_tracked() {
    let mut t = Tracked::default();
    for s in [
        "BEGIN",
        "CREATE TEMP TABLE x(a int)",
        "SET work_mem = '1MB'",
        "SELECT pg_advisory_lock(1)",
    ] {
        track_statement(&mut t, s);
    }
    t.tx = TxStatus::InTx;
    assert_eq!(
        lost_state(&t),
        LostState {
            open_tx: true,
            temp_objects: true,
            session_vars: true,
            prepared: 0,
            locks: true
        }
    );
}

// Beyond the brief.

#[test]
fn the_policy_is_b19s_and_gives_up_at_five_minutes() {
    let p = RecoveryPolicy::default();
    assert_eq!(p.schedule, [secs(0), secs(1), secs(2), secs(4), secs(8)]);
    assert_eq!((p.steady, p.give_up_after), (secs(30), secs(300)));
    assert!((p.jitter - 0.2).abs() < f32::EPSILON);
    let mut mid = || 0.5f32;
    assert_eq!(p.delay(3, secs(299), &mut mid), Some(secs(4)));
    assert_eq!(p.delay(0, secs(300), &mut mid), None);
    // The production RNG stays inside the ±20 % band.
    for _ in 0..50 {
        let d = p
            .delay(5, secs(10), &mut RecoveryPolicy::jitter_source())
            .unwrap();
        assert!(d >= secs(24) && d <= secs(36), "{d:?}");
    }
}

#[test]
fn every_recovery_case_and_none_other() {
    let expected = |c: ErrClass| {
        c.id().starts_with("network.")
            || matches!(
                c,
                ErrClass::ServerShutdown
                    | ErrClass::ServerStarting
                    | ErrClass::KaragozUpstreamUnreachable
                    | ErrClass::KaragozResumeExpired
            )
    };
    for c in ErrClass::ALL {
        assert_eq!(is_recovery_case(*c), expected(*c), "{}", c.id());
    }
}

#[test]
fn the_scan_reads_the_first_keyword_only_and_ignores_transaction_scoped_state() {
    let scan = |statements: &[&str]| {
        let mut t = Tracked::default();
        for s in statements {
            track_statement(&mut t, s);
        }
        lost_state(&t)
    };
    // Transaction-scoped: gone with the transaction anyway, never "also gone".
    assert!(
        scan(&[
            "SET LOCAL work_mem = '1MB'",
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SELECT pg_advisory_xact_lock(1)",
            "select 'SET x = 1, pg_advisory_lock(1)' -- LOCK",
            "CREATE TABLE t (a int)",
            "COMMIT",
        ])
        .is_empty()
    );
    // Comments, case and MySQL spellings.
    let lost = scan(&[
        "/* tmp */ create temporary table t (a int)",
        "  -- vars\n set @x = 1",
        "LOCK TABLES t WRITE",
        "PREPARE s1 FROM 'SELECT 1'",
        "prepare s2 as select 2",
    ]);
    assert_eq!(
        lost,
        LostState {
            open_tx: false,
            temp_objects: true,
            session_vars: true,
            prepared: 2,
            locks: true,
        }
    );
    // GET_LOCK and the try variants are session locks too; DEALLOCATE forgets a statement.
    let lost = scan(&[
        "SELECT GET_LOCK('job', 10)",
        "PREPARE s1 FROM 'SELECT 1'",
        "DEALLOCATE PREPARE s1",
    ]);
    assert!(lost.locks && lost.prepared == 0);
    assert!(scan(&["SELECT pg_try_advisory_lock(7)"]).locks);
    let mut t = Tracked::default();
    for s in [
        "PREPARE a AS SELECT 1",
        "PREPARE b AS SELECT 2",
        "DEALLOCATE ALL",
    ] {
        track_statement(&mut t, s);
    }
    assert_eq!(t.prepared, 0);
    // A failed transaction is an open one.
    let failed = Tracked {
        tx: TxStatus::Failed,
        ..Tracked::default()
    };
    assert!(lost_state(&failed).open_tx);
}
