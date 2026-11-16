//! M3a Task 23: the console UX table (production, read-only, access, the user's
//! transaction only, its Failed form, the cursor badge, cancelling, running, waiting).
//! ## m3b-t26-prod-unlock: the production read-only policy is gone (`forced` and
//! `forced_read_only` with it); its two rows now pin a writable production console in Manual
//! mode and a production profile that is read-only by its own flag.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hacivat_driver::TxStatus;
use hacivat_exec::{
    CancelUx, ConsoleUx, RunStatus, TxBadge, UnitStatus, UnitStatusEntry, console_ux,
};
use hacivat_proto::{AccessLabel, EnvTag};
use hacivat_results::ResultId;
use hacivat_session::session::{
    CancelState, CursorBadge, RunId, RunSummary, SessionSnapshot, SessionState, TxOwner, TxState,
    UnitId,
};

/// One row of the table: what the console knows.
#[derive(Clone)]
struct Inputs {
    env: Option<EnvTag>,
    snap: SessionSnapshot,
    run: Option<RunStatus>,
    waiting: Option<(u8, u8)>,
}

fn console_ux_of(i: Inputs) -> ConsoleUx {
    console_ux(i.env, &i.snap, i.run.as_ref(), i.waiting)
}

fn t0() -> Instant {
    // A fixed point well inside the clock's range; every case is relative to it.
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T0.get_or_init(Instant::now)
}

fn at(s: u64) -> Instant {
    t0() + Duration::from_secs(s)
}

fn snap() -> SessionSnapshot {
    SessionSnapshot {
        state: SessionState::Idle,
        tx: TxState::default(),
        schema: None,
        cancel: CancelState::Idle,
        read_only: false,
        access: AccessLabel::ReadWrite,
        generation: 1,
        cursor: None,
    }
}

fn base() -> Inputs {
    Inputs {
        env: Some(EnvTag::Dev),
        snap: snap(),
        run: None,
        waiting: None,
    }
}

fn idle_ux(env: Option<EnvTag>) -> ConsoleUx {
    ConsoleUx {
        env,
        production: false,
        read_only: false,
        access: AccessLabel::ReadWrite,
        tx: TxBadge::Auto,
        cursor: None,
        cancel: CancelUx::Idle,
        running: None,
        waiting_for_connection: None,
    }
}

fn tx(owner: TxOwner, status: TxStatus, since: Option<Instant>, statements: u32) -> TxState {
    TxState {
        status,
        since,
        statements,
        owner,
        ..TxState::default()
    }
}

fn cursor() -> CursorBadge {
    CursorBadge {
        result: ResultId(4),
        rows: 10_000,
        since: at(3),
        holds_tx: true,
    }
}

fn run_status(started: Option<Instant>, ended: bool) -> RunStatus {
    let unit = UnitId {
        run: RunId(9),
        ordinal: 0,
    };
    RunStatus {
        run: Some(RunId(9)),
        started,
        units: Arc::from(vec![UnitStatusEntry {
            unit,
            doc_range: 0..8,
            status: if started.is_some() {
                UnitStatus::Running { since: at(5) }
            } else {
                UnitStatus::Queued { position: 1 }
            },
        }]),
        summary: ended.then(RunSummary::default),
    }
}

fn ux_cases() -> Vec<(Inputs, ConsoleUx)> {
    let mut cases = Vec::new();
    // Idle dev console.
    cases.push((base(), idle_ux(Some(EnvTag::Dev))));
    // ## m3b-t26-prod-unlock: a production console is read-write (Manual mode, D-036; the gate
    // confirms its writes): production, never read-only, the read-write label.
    let mut i = base();
    i.env = Some(EnvTag::Production);
    i.snap.tx.mode = hacivat_session::session::TxMode::Manual;
    cases.push((
        i.clone(),
        ConsoleUx {
            production: true,
            read_only: false,
            access: AccessLabel::ReadWrite,
            ..idle_ux(Some(EnvTag::Production))
        },
    ));
    // A production profile that is read-only by its own flag: read-only, still production.
    let mut j = i.clone();
    j.snap.tx.mode = hacivat_session::session::TxMode::Auto;
    j.snap.read_only = true;
    j.snap.access = AccessLabel::ReadOnlyGuarded;
    cases.push((
        j,
        ConsoleUx {
            production: true,
            read_only: true,
            access: AccessLabel::ReadOnlyGuarded,
            ..idle_ux(Some(EnvTag::Production))
        },
    ));
    // A read-only dev profile.
    let mut k = base();
    k.snap.read_only = true;
    k.snap.access = AccessLabel::ReadOnlyEnforced;
    cases.push((
        k,
        ConsoleUx {
            read_only: true,
            access: AccessLabel::ReadOnlyEnforced,
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // The user's open transaction (R-M3-27).
    let mut u = base();
    u.snap.tx = tx(TxOwner::User, TxStatus::InTx, Some(at(1)), 3);
    cases.push((
        u,
        ConsoleUx {
            tx: TxBadge::UserOpen {
                since: at(1),
                statements: 3,
            },
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // The user's failed transaction (R-M3-16 amended).
    let mut f = base();
    f.snap.tx = tx(TxOwner::User, TxStatus::Failed, Some(at(1)), 2);
    cases.push((
        f,
        ConsoleUx {
            tx: TxBadge::Failed,
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // Hacivat's own transaction around a PostgreSQL cursor is no user transaction.
    let mut c = base();
    c.snap.tx = tx(TxOwner::Cursor, TxStatus::InTx, Some(at(3)), 0);
    c.snap.cursor = Some(cursor());
    cases.push((
        c,
        ConsoleUx {
            tx: TxBadge::CursorSnapshot { since: at(3) },
            cursor: Some(cursor()),
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // The read-only guard's transaction, and one nobody claimed, show nothing.
    for owner in [TxOwner::Guard, TxOwner::None] {
        let mut g = base();
        g.snap.tx = tx(owner, TxStatus::InTx, Some(at(2)), 1);
        cases.push((g, idle_ux(Some(EnvTag::Dev))));
    }
    // Cancelling, at each rung.
    for cancel in [
        CancelState::Cancelling { since: at(7) },
        CancelState::SlowCancel { since: at(7) },
    ] {
        let mut x = base();
        x.snap.cancel = cancel;
        x.snap.state = SessionState::Cancelling {
            req: 4,
            since: at(7),
        };
        cases.push((
            x,
            ConsoleUx {
                cancel: CancelUx::Cancelling { since: at(7) },
                ..idle_ux(Some(EnvTag::Dev))
            },
        ));
    }
    let mut d = base();
    d.snap.cancel = CancelState::Deactivating;
    d.snap.state = SessionState::Cancelling {
        req: 4,
        since: at(8),
    };
    cases.push((
        d,
        ConsoleUx {
            cancel: CancelUx::Cancelling { since: at(8) },
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // A run that started and has not ended.
    let mut r = base();
    r.snap.state = SessionState::Busy {
        req: 4,
        since: at(5),
    };
    r.run = Some(run_status(Some(at(5)), false));
    cases.push((
        r,
        ConsoleUx {
            running: Some((RunId(9), at(5))),
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // Queued (not started) and ended runs are not running.
    for (started, ended) in [(None, false), (Some(at(5)), true)] {
        let mut q = base();
        q.run = Some(run_status(started, ended));
        cases.push((q, idle_ux(Some(EnvTag::Dev))));
    }
    // Busy with request 0: the idle bound releasing a cursor, which no run holds (T18 M4).
    let mut z = base();
    z.snap.state = SessionState::Busy {
        req: 0,
        since: at(9),
    };
    z.snap.cursor = Some(cursor());
    z.run = Some(run_status(Some(at(5)), true));
    cases.push((
        z,
        ConsoleUx {
            cursor: Some(cursor()),
            ..idle_ux(Some(EnvTag::Dev))
        },
    ));
    // Waiting for a free connection (R-M3-43), on a console without a tag.
    let mut w = base();
    w.env = None;
    w.waiting = Some((4, 4));
    cases.push((
        w,
        ConsoleUx {
            waiting_for_connection: Some((4, 4)),
            ..idle_ux(None)
        },
    ));
    cases
}

#[test]
fn console_ux_table() {
    for (n, (inputs, want)) in ux_cases().into_iter().enumerate() {
        assert_eq!(console_ux_of(inputs), want, "case {n}");
    }
}

// ## m3a-t33-exec-ui (named exception, T34 review I3)
/// A transaction lives on its connection: while the session is disconnected, reconnecting or
/// broken, the last published `tx` (the user's open or failed transaction) shows no badge.
#[test]
fn no_transaction_badge_without_a_connection() {
    for status in [TxStatus::InTx, TxStatus::Failed] {
        for state in [
            SessionState::Disconnected,
            SessionState::Connecting,
            SessionState::Broken(hacivat_proto::ErrClass::ALL[0]),
        ] {
            let mut i = base();
            i.snap.tx = tx(TxOwner::User, status, Some(at(1)), 3);
            i.snap.state = state.clone();
            assert_eq!(
                console_ux_of(i).tx,
                TxBadge::Auto,
                "{status:?} while {state:?}"
            );
        }
        let mut live = base();
        live.snap.tx = tx(TxOwner::User, status, Some(at(1)), 3);
        assert_ne!(
            console_ux_of(live).tx,
            TxBadge::Auto,
            "{status:?} while idle"
        );
    }
}
