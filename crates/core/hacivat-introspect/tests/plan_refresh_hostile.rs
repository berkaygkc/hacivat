// ## m4-t21-refresh (fix round 1)
//! The T21 opus review's hostile tests, adopted: H1–H6 failed on `259d00d` and now assert the
//! fixes (I1 `COMMIT AND CHAIN`, I2 maybe-committed failed units, I3 widening across units, I4
//! a scope change under `never`, I5 database DDL in a transaction); P1–P3 passed and stay.

#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use hacivat_introspect::ddl_effects::{
    ExecutedStmt, SessionStateAtExec, Target, TxnState, lower, plan_refresh,
};
use hacivat_introspect::engine::IntroHandle;
use hacivat_introspect::introspector::IntroCaps;
use hacivat_introspect::refresh_coord::{
    CoordEvent, RefreshCoordinator, RefreshInbox, RefreshMsg, RunTracker, TrackedUnit, TxPhase,
};
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::merge::Trigger;
use hacivat_meta::{NsKey, ObjectKey};
use hacivat_proto::{AccessLabel, DbmsKind};
use hacivat_session::session::{
    ExecError, ExecErrorKind, RunId, SchemaCtx, SessionId, SessionSnapshot, TxOutcome, TxState,
    UnitEnd, UnitId,
};
use hacivat_settings::RefreshAfterDdl;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_safety::{ClassifyCtx, Expect, StmtReport, classify};
use tokio::sync::watch;

const DB: &str = "app";
const S: SessionId = SessionId(7);

fn ns(s: &str) -> NsKey {
    pg_ns(DB, s)
}

fn dialect(d: DbmsKind) -> DialectId {
    match d {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

fn reports(d: DbmsKind, sql: &str) -> Vec<StmtReport> {
    let ctx = ClassifyCtx::new(
        DialectCtx::new(dialect(d), None),
        ClientFlavor::None,
        Expect::Script,
    );
    classify(sql, &ctx).items
}

fn stmts(d: DbmsKind, sql: &str) -> Vec<ExecutedStmt> {
    reports(d, sql)
        .iter()
        .map(ExecutedStmt::from_report)
        .collect()
}

fn unit(n: u32) -> UnitId {
    UnitId {
        run: RunId(1),
        ordinal: n,
    }
}

fn snap_at(db: Option<&str>, schema: &str, path: &[&str]) -> SessionSnapshot {
    SessionSnapshot {
        state: Default::default(),
        tx: TxState::default(),
        schema: Some(SchemaCtx {
            database: db.map(Into::into),
            schema: Some(schema.into()),
            search_path: path.iter().map(|s| s.to_string()).collect(),
            user: None,
            search_path_setting: Vec::new(),
        }),
        cancel: Default::default(),
        read_only: false,
        access: AccessLabel::ReadWrite,
        generation: 0,
        cursor: None,
    }
}

enum Step {
    Start(u32),
    Tx(TxPhase, TxPhase),
    Ok(u32),
    Fail(u32, ExecErrorKind),
}
use Step::{Fail, Ok as Done, Start, Tx};
use TxPhase::{Idle, InTx};

fn run(
    d: DbmsKind,
    snap: SessionSnapshot,
    units: &[&str],
    start: TxPhase,
    steps: &[Step],
) -> Vec<RefreshMsg> {
    let (_tx, rx) = watch::channel(snap);
    let inbox = RefreshInbox::with_capacity(256);
    let tracked = units
        .iter()
        .enumerate()
        .map(|(i, sql)| {
            let r = reports(d, sql);
            TrackedUnit::new(unit(i as u32), Some(&r), dialect(d), sql)
        })
        .collect();
    let mut t = RunTracker::new(S, d, tracked, rx, start, inbox.clone());
    for s in steps {
        match s {
            Start(n) => t.unit_started(unit(*n)),
            Tx(a, b) => t.tx_changed(*a, *b),
            Done(n) => t.unit_ended(unit(*n), &UnitEnd::Succeeded { rows: None }),
            Fail(n, k) => t.unit_ended(unit(*n), &UnitEnd::Failed(ExecError::of(k.clone()))),
        }
    }
    inbox.queued()
}

fn outcomes(m: &[RefreshMsg]) -> Vec<TxOutcome> {
    m.iter()
        .filter_map(|m| match m {
            RefreshMsg::TxEnded { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

fn executed(m: &[RefreshMsg]) -> usize {
    m.iter()
        .filter(|m| matches!(m, RefreshMsg::Executed { .. }))
        .count()
}

async fn loaded(objects: Vec<FakeObject>) -> Rig {
    let fake = FakeIntrospector::new(DbmsKind::Postgres);
    fake.put_ns(ns("public"), objects);
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current(DB, "public"),
            ..RigOpts::default()
        },
    );
    bounded(
        r.handle
            .refresh(Target::Namespace(ns("public")), Trigger::User),
    )
    .await
    .unwrap();
    idle(&r.handle).await;
    r
}

fn coord(h: &IntroHandle, pref: RefreshAfterDdl) -> RefreshCoordinator {
    RefreshCoordinator::new(
        h.clone(),
        DbmsKind::Postgres,
        pref,
        tokio::runtime::Handle::current(),
    )
}

fn fragment_keys(r: &Rig) -> Vec<ObjectKey> {
    r.fake
        .calls()
        .into_iter()
        .flat_map(|c| match c {
            FakeCall::Fragment { keys, .. } => keys,
            _ => Vec::new(),
        })
        .collect()
}

// ---- H1: PostgreSQL COMMIT AND CHAIN ----------------------------------------------------------

/// `BEGIN; ALTER …; COMMIT AND CHAIN; ROLLBACK`: the ALTER is committed by `COMMIT AND CHAIN`
/// (the server status stays `T`, so no TxChanged), the chained transaction is then rolled back.
/// The committed change must be refreshed.
#[test]
fn h1_tracker_commit_and_chain_reports_a_commit() {
    let kinds: Vec<_> = reports(DbmsKind::Postgres, "commit and chain")
        .iter()
        .map(|r| r.kind)
        .collect();
    eprintln!("classifier kind of COMMIT AND CHAIN: {kinds:?}");
    let m = run(
        DbmsKind::Postgres,
        snap_at(Some(DB), "public", &["public"]),
        &[
            "begin",
            "alter table public.t add column z int",
            "commit and chain",
            "rollback",
        ],
        Idle,
        &[
            Start(0),
            Tx(Idle, InTx),
            Done(0),
            Start(1),
            Done(1),
            Start(2),
            Done(2), // COMMIT AND CHAIN: status InTx -> InTx, no TxChanged
            Start(3),
            Tx(InTx, Idle),
            Done(3),
        ],
    );
    let o = outcomes(&m);
    eprintln!("outcomes: {o:?}");
    assert!(
        o.first().is_some_and(TxOutcome::counts_as_committed),
        "the first transaction end must count as committed, got {o:?}"
    );
}

/// The same through the coordinator: the committed ALTER of `t` is never refreshed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h1_coordinator_commit_and_chain_never_refreshes_the_committed_alter() {
    let r = loaded(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let c = coord(&r.handle, RefreshAfterDdl::Always);
    let m = run(
        DbmsKind::Postgres,
        snap_at(Some(DB), "public", &["public"]),
        &[
            "begin",
            "alter table public.t add column z int",
            "commit and chain",
            "rollback",
        ],
        Idle,
        &[
            Start(0),
            Tx(Idle, InTx),
            Done(0),
            Start(1),
            Done(1),
            Start(2),
            Done(2),
            Start(3),
            Tx(InTx, Idle),
            Done(3),
        ],
    );
    let before = fragment_keys(&r).len();
    let inbox = c.inbox();
    for msg in m {
        inbox.push(msg);
    }
    c.drain();
    // Adopted (fix round 1): the refresh is handed over synchronously at the commit and runs
    // on a spawned task, so wait on the fragment itself rather than on an idle engine.
    assert_eq!(
        c.applied(),
        1,
        "the committed ALTER of t was never handed over"
    );
    assert!(c.pending(S).is_empty());
    wait_until(Duration::from_secs(30), || {
        fragment_keys(&r)[before..].iter().any(|k| k.name() == "t")
    })
    .await;
}

// ---- H2: a unit run as one fails after its DDL committed (SQLite / MySQL) --------------------

/// `create table a; create table b; select * from nope` run as one unit on SQLite in
/// autocommit: a and b are committed (each statement autocommits), the unit fails on the
/// third. The committed tables must be refreshed.
#[test]
fn h2_run_as_one_failing_after_committed_ddl_plans_it() {
    let m = run(
        DbmsKind::Sqlite,
        snap_at(None, "main", &["main"]),
        &["create table a(x int); create table b(x int); select * from nope"],
        Idle,
        &[Start(0), Fail(0, ExecErrorKind::Db)],
    );
    eprintln!("messages: {m:?}");
    assert!(
        executed(&m) > 0,
        "committed DDL of a failed batch planned nothing"
    );
}

// ---- H3: autocommit DDL whose answer was lost -------------------------------------------------

/// An autocommit ALTER whose connection went before the answer (`outcome_unknown`): the server
/// may have committed it. Refresh more, never less.
#[test]
fn h3_autocommit_ddl_with_unknown_outcome_is_planned() {
    let m = run(
        DbmsKind::Postgres,
        snap_at(Some(DB), "public", &["public"]),
        &["alter table public.t add column z int"],
        Idle,
        &[
            Start(0),
            Fail(
                0,
                ExecErrorKind::ConnectionLost {
                    outcome_unknown: true,
                },
            ),
        ],
    );
    eprintln!("messages: {m:?}");
    assert!(executed(&m) > 0, "a maybe-committed DDL planned nothing");
}

// ---- H4: widening across units ------------------------------------------------------------------

/// 60 single-statement ALTER units in one transaction, then COMMIT: more than 50 object targets
/// in `public` (alg/02 step 3, edge 12) must widen to a namespace refresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h4_sixty_units_in_one_transaction_widen_at_commit() {
    let objs = (0..60)
        .map(|i| FakeObject::table(&format!("t{i}"), &["a"]).with_oid(1000 + i))
        .collect();
    let r = loaded(objs).await;
    let c = coord(&r.handle, RefreshAfterDdl::Always);
    let inbox = c.inbox();
    let state = SessionStateAtExec {
        current: ns("public"),
        search_path: vec![ns("public")],
    };
    for i in 0..60 {
        inbox.push(RefreshMsg::Executed {
            session: S,
            stmts: stmts(
                DbmsKind::Postgres,
                &format!("alter table public.t{i} add column z int"),
            ),
            state: state.clone(),
            txn: TxnState::InTransaction,
        });
    }
    c.drain();
    let pending = c.pending(S);
    let lowered = lower(&pending, &r.catalog.current());
    eprintln!(
        "pending: {} targets; lowered at commit: {:?}",
        pending.len(),
        lowered
            .iter()
            .map(|t| match t {
                Target::Objects { keys, .. } => format!("Objects({})", keys.len()),
                o => format!("{o:?}"),
            })
            .collect::<Vec<_>>()
    );
    assert!(
        lowered.contains(&Target::Namespace(ns("public"))),
        "60 object targets in one namespace were not widened"
    );
}

// ---- H5: `never` and a scope change ---------------------------------------------------------

/// `never` + `create schema s9`: the only target is the scope refresh. Something must be
/// marked stale (edge 14), or the explorer stays silently out of date.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_never_marks_a_scope_change_somewhere() {
    let r = loaded(vec![FakeObject::table("t", &["a"])]).await;
    let c = coord(&r.handle, RefreshAfterDdl::Never);
    let mut ev = c.events();
    let p = plan_refresh(
        &stmts(DbmsKind::Postgres, "create schema s9"),
        &SessionStateAtExec {
            current: ns("public"),
            search_path: vec![ns("public")],
        },
        TxnState::AutoCommit,
        &IntroCaps::planned(DbmsKind::Postgres),
    );
    eprintln!("plan: {p:?}");
    bounded(c.submit(S, p)).await.unwrap();
    let got = ev.try_recv();
    eprintln!("event after submit: {got:?}");
    assert!(
        matches!(got, Ok(CoordEvent::Stale { .. })),
        "a scope change under `never` marked nothing stale"
    );
}

// ---- positive probes (expected to pass) -------------------------------------------------------

/// Same-name savepoints and RELEASE: `SAVEPOINT s; alter t1; SAVEPOINT s; alter t2; RELEASE s;
/// ROLLBACK TO s; COMMIT` — RELEASE drops the newer `s`, ROLLBACK TO goes to the older one:
/// neither t1 nor t2 is refreshed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p1_release_then_rollback_to_reaches_the_older_savepoint() {
    let r = loaded(vec![
        FakeObject::table("t0", &["a"]).with_oid(500),
        FakeObject::table("t1", &["a"]).with_oid(501),
        FakeObject::table("t2", &["a"]).with_oid(502),
    ])
    .await;
    let c = coord(&r.handle, RefreshAfterDdl::Always);
    let m = run(
        DbmsKind::Postgres,
        snap_at(Some(DB), "public", &["public"]),
        &[
            "alter table public.t0 add column z int",
            "savepoint s",
            "alter table public.t1 add column z int",
            "savepoint s",
            "alter table public.t2 add column z int",
            "release savepoint s",
            "rollback to savepoint s",
            "commit",
        ],
        InTx,
        &[
            Start(0),
            Done(0),
            Start(1),
            Done(1),
            Start(2),
            Done(2),
            Start(3),
            Done(3),
            Start(4),
            Done(4),
            Start(5),
            Done(5),
            Start(6),
            Done(6),
            Start(7),
            Tx(InTx, Idle),
            Done(7),
        ],
    );
    let before = fragment_keys(&r).len();
    let inbox = c.inbox();
    for msg in m {
        inbox.push(msg);
    }
    c.drain();
    wait_until(Duration::from_secs(30), || {
        fragment_keys(&r)[before..].iter().any(|k| k.name() == "t0")
    })
    .await;
    idle(&r.handle).await;
    let names: Vec<String> = fragment_keys(&r)[before..]
        .iter()
        .map(|k| k.name().to_string())
        .collect();
    assert!(
        !names.contains(&"t1".into()) && !names.contains(&"t2".into()),
        "{names:?}"
    );
}

/// SQLite: `SAVEPOINT s` outside a transaction opens one; `create table q; ROLLBACK TO s;
/// RELEASE s` commits nothing of q: q is never refreshed; `SAVEPOINT s; create table w;
/// RELEASE s` commits w: w is refreshed.
#[test]
fn p2_sqlite_savepoint_opens_a_transaction() {
    let sq = DbmsKind::Sqlite;
    let m = run(
        sq,
        snap_at(None, "main", &["main"]),
        &[
            "savepoint s",
            "create table q(a)",
            "rollback to s",
            "release s",
        ],
        Idle,
        &[
            Start(0),
            Tx(Idle, InTx),
            Done(0),
            Start(1),
            Done(1),
            Start(2),
            Done(2),
            Start(3),
            Tx(InTx, Idle),
            Done(3),
        ],
    );
    eprintln!("{m:?}");
    // Savepoint, Executed(pending), RolledBackTo(s), Committed.
    assert!(matches!(m[0], RefreshMsg::Savepoint { .. }), "{m:?}");
    assert_eq!(
        outcomes(&m),
        vec![
            TxOutcome::RolledBackTo(Arc::from("s")),
            TxOutcome::Committed
        ]
    );
}

/// The search path changes between two units of one run (`SET search_path` reprobed before
/// `UnitEnded`): the second unit's unqualified name resolves in the new schema.
#[test]
fn p3_search_path_changing_mid_run_is_read_per_unit() {
    let (tx, rx) = watch::channel(snap_at(Some(DB), "public", &["public"]));
    let inbox = RefreshInbox::with_capacity(16);
    let units = [
        "create table a(x int)",
        "set search_path to s2",
        "create table b(x int)",
    ]
    .iter()
    .enumerate()
    .map(|(i, sql)| {
        let r = reports(DbmsKind::Postgres, sql);
        TrackedUnit::new(unit(i as u32), Some(&r), DialectId::Postgres, sql)
    })
    .collect();
    let mut t = RunTracker::new(S, DbmsKind::Postgres, units, rx, Idle, inbox.clone());
    t.unit_started(unit(0));
    t.unit_ended(unit(0), &UnitEnd::Succeeded { rows: None });
    t.unit_started(unit(1));
    tx.send_replace(snap_at(Some(DB), "s2", &["s2"]));
    t.unit_ended(unit(1), &UnitEnd::Succeeded { rows: None });
    t.unit_started(unit(2));
    t.unit_ended(unit(2), &UnitEnd::Succeeded { rows: None });
    let currents: Vec<NsKey> = inbox
        .queued()
        .into_iter()
        .filter_map(|m| match m {
            RefreshMsg::Executed { state, .. } => Some(state.current),
            _ => None,
        })
        .collect();
    assert_eq!(currents, vec![ns("public"), ns("s2")]);
}

// ---- H6: database-level DDL that may run inside a transaction -------------------------------

/// `ALTER DATABASE … RENAME`, `COMMENT ON DATABASE` and `GRANT … ON DATABASE` can run inside a
/// PostgreSQL transaction block (only CREATE/DROP DATABASE cannot). Refreshed "now" they are
/// read before the commit and never again.
#[test]
fn h6_database_ddl_in_a_transaction_waits_for_the_commit() {
    let st = SessionStateAtExec {
        current: ns("public"),
        search_path: vec![ns("public")],
    };
    for sql in [
        "alter database app rename to app2",
        "comment on database app is 'x'",
        "grant connect on database app to r",
    ] {
        let s = stmts(DbmsKind::Postgres, sql);
        let p = plan_refresh(
            &s,
            &st,
            TxnState::InTransaction,
            &IntroCaps::planned(DbmsKind::Postgres),
        );
        eprintln!(
            "{sql}: ddl {:?} -> {p:?}",
            s.iter().map(|x| &x.ddl).collect::<Vec<_>>()
        );
        assert!(
            p.now.is_empty(),
            "{sql}: refreshed before the commit, never after: {p:?}"
        );
    }
}
