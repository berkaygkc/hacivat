//! M3b Task 19 over Docker: confirm-in-transaction on real servers (HV-EXEC-04 session side;
//! D-014, R-M3-55, P-M3b-1). With the deadline injected at 2 s, on pg17, mysql84 and SQLite (in
//! process), both forms:
//! - **own transaction** (Auto, idle): an admin side channel sees the row only after Commit;
//!   the deadline rolls it back;
//! - **savepoint** (inside the user's transaction, and a Manual MySQL session with
//!   `autocommit = 0`, which needs no `START TRANSACTION`): the row is visible only after Keep
//!   and the user's own `COMMIT`; the deadline undoes the confirmed statement only, and an
//!   earlier statement of the user's transaction is committed by the user's `COMMIT`.
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr -p hacivat-session --test
//! docker_m3b_confirm`. Targets come only from `require_target` (Q6); fixtures have names unique
//! to the run and are dropped at the end (Q7); waits are on conditions bounded by real time.

mod common;

use std::time::Duration;

use common::confirm::*;
use common::conformance::*;
use hacivat_driver::TxStatus;
use hacivat_proto::DbmsKind;
use hacivat_session::pipeline::{Interaction, LiveConnection, connect};
use hacivat_session::session::{
    CommitAnswer, Request, SessionEffects, SessionHandle, TxCommand, TxMode, UnitVerdict,
};
use hacivat_testkit::{DockerTarget, require_target};

const DEADLINE: Duration = Duration::from_secs(2);
const LIMIT: Duration = Duration::from_secs(30);

/// A confirmed write of `sql` on `s`.
fn confirmed(s: &SessionHandle, sql: &str) -> Ran {
    run_units(
        s,
        vec![verdict_unit(0, sql, UnitVerdict::TxThenConfirm)],
        DEADLINE,
    )
}

/// Waits (real time) for the run's question: whether it is in the savepoint form.
async fn asked(id: &str, ran: &Ran) -> bool {
    wait_until(&format!("{id}: the question"), LIMIT, || {
        !commit_questions(&ran.o).is_empty() || run_ended(&ran.o)
    })
    .await;
    let questions = commit_questions(&ran.o);
    assert!(
        !questions.is_empty(),
        "{id}: the run ended without asking: {:?}",
        ran.o.unit_ends()
    );
    questions[0].savepoint
}

/// Answers the run's question `a` and waits for its end.
async fn answer(id: &str, s: &SessionHandle, ran: &Ran, a: CommitAnswer) {
    let q = commit_questions(&ran.o)[0];
    assert!(
        s.commit_decision(ran.ticket.seq, q.unit, q.question, a)
            .await
            .unwrap_or(false),
        "{id}: the answer was taken"
    );
    ended(id, ran).await;
}

async fn ended(id: &str, ran: &Ran) {
    wait_until(&format!("{id}: the run ends"), LIMIT, || run_ended(&ran.o)).await;
}

/// A user statement on `s` that must succeed.
async fn user(id: &str, s: &SessionHandle, sql: &str) {
    if let Err(e) = run(s, sql).await {
        panic!(
            "{id}: `{sql}`: {:?} {:?}",
            e.kind,
            code_of(e.server.as_ref())
        );
    }
}

/// Rows of `table` with `a = v`, as the side channel sees them.
async fn seen(side: &mut LiveConnection, table: &str, v: i64) -> i64 {
    count(side, &format!("SELECT count(*) FROM {table} WHERE a = {v}")).await
}

/// Both forms on one session (`s`) with the side channel `side` reading `table`.
async fn both_forms(
    id: &str,
    engine: DbmsKind,
    s: &SessionHandle,
    side: &mut LiveConnection,
    table: &str,
) {
    // The own form: Commit makes the row visible, not before.
    let ran = confirmed(s, &format!("INSERT INTO {table} (a) VALUES (1)"));
    assert!(!asked(id, &ran).await, "{id}: Hacivat's own transaction");
    assert_eq!(seen(side, table, 1).await, 0, "{id}: not before Commit");
    answer(id, s, &ran, CommitAnswer::Commit).await;
    assert_eq!(seen(side, table, 1).await, 1, "{id}: after Commit");
    assert_eq!(decided(&ran.o), [(CommitAnswer::Commit, false)], "{id}");

    // The own form: the deadline rolls back.
    let ran = confirmed(s, &format!("INSERT INTO {table} (a) VALUES (2)"));
    assert!(!asked(id, &ran).await, "{id}");
    ended(id, &ran).await;
    assert_eq!(decided(&ran.o), [(CommitAnswer::RollBack, true)], "{id}");
    assert_eq!(
        seen(side, table, 2).await,
        0,
        "{id}: rolled back by the deadline"
    );
    assert_eq!(s.state().borrow().tx.status, TxStatus::Idle, "{id}");

    // The savepoint form inside the user's transaction: Keep, then the user's COMMIT.
    let begin = if engine == DbmsKind::MySql {
        "START TRANSACTION"
    } else {
        "BEGIN"
    };
    user(id, s, begin).await;
    let ran = confirmed(s, &format!("INSERT INTO {table} (a) VALUES (3)"));
    assert!(asked(id, &ran).await, "{id}: the savepoint form");
    answer(id, s, &ran, CommitAnswer::Commit).await;
    assert_eq!(seen(side, table, 3).await, 0, "{id}: kept, not committed");
    assert_eq!(s.state().borrow().tx.status, TxStatus::InTx, "{id}");
    user(id, s, "COMMIT").await;
    assert_eq!(
        seen(side, table, 3).await,
        1,
        "{id}: after the user's COMMIT"
    );

    // The deadline undoes the confirmed statement only: the user's earlier statement stays and
    // is committed by the user.
    user(id, s, begin).await;
    user(id, s, &format!("INSERT INTO {table} (a) VALUES (5)")).await;
    let ran = confirmed(s, &format!("INSERT INTO {table} (a) VALUES (4)"));
    assert!(asked(id, &ran).await, "{id}");
    ended(id, &ran).await;
    assert_eq!(decided(&ran.o), [(CommitAnswer::RollBack, true)], "{id}");
    assert_eq!(
        s.state().borrow().tx.status,
        TxStatus::InTx,
        "{id}: the user's transaction stays open"
    );
    user(id, s, "COMMIT").await;
    assert_eq!(
        seen(side, table, 4).await,
        0,
        "{id}: undone by the deadline"
    );
    assert_eq!(
        seen(side, table, 5).await,
        1,
        "{id}: the user's statement kept"
    );
}

async fn server_leg(t: &DockerTarget) {
    let id = t.id.as_str();
    let table = unique("hv_t19");
    let mut side = admin(t).await;
    let create = match t.engine {
        DbmsKind::MySql | DbmsKind::MariaDb => {
            format!("CREATE TABLE {table} (a INT) ENGINE=InnoDB")
        }
        _ => format!("CREATE TABLE {table} (a INT)"),
    };
    batch(&mut side, id, &create).await;
    let on = session_on(t, Path::Direct).await;
    both_forms(id, t.engine, &on.s, &mut side, &table).await;

    if t.engine == DbmsKind::MySql {
        // Manual: `autocommit = 0` keeps a transaction open; the savepoint goes straight in.
        let mode =
            on.s.submit(Request::Tx(TxCommand::SetMode(TxMode::Manual)))
                .reply
                .await
                .expect("the actor replies");
        assert!(mode.is_ok(), "{id}: Manual");
        let ran = confirmed(&on.s, &format!("INSERT INTO {table} (a) VALUES (6)"));
        assert!(
            asked(id, &ran).await,
            "{id}: Manual takes the savepoint form"
        );
        answer(id, &on.s, &ran, CommitAnswer::Commit).await;
        assert_eq!(seen(&mut side, &table, 6).await, 0, "{id}: Manual, kept");
        user(id, &on.s, "COMMIT").await;
        assert_eq!(
            seen(&mut side, &table, 6).await,
            1,
            "{id}: Manual, committed"
        );

        // ## m3b-t19-confirm-tx (fix round 1, m6): Manual Undo. The savepoint was the first
        // statement under `autocommit = 0`; `ROLLBACK TO` undoes the insert, the user's COMMIT
        // commits nothing of it.
        let ran = confirmed(&on.s, &format!("INSERT INTO {table} (a) VALUES (9)"));
        assert!(asked(id, &ran).await, "{id}: Manual, savepoint form");
        answer(id, &on.s, &ran, CommitAnswer::RollBack).await;
        user(id, &on.s, "COMMIT").await;
        assert_eq!(seen(&mut side, &table, 9).await, 0, "{id}: Manual, undone");

        // Fix round 1 (C1): `SET autocommit = 1; INSERT …` in Manual. The server's own flag
        // says nothing is open after the first statement, so the confirmed one takes Hacivat's
        // own transaction (never a no-op savepoint over an autocommitted change), and Undo
        // undoes it.
        let mut set = verdict_unit(0, "SET autocommit = 1", UnitVerdict::ImplicitCommit);
        set.effects = SessionEffects::COMMITS_FIRST;
        let ran = run_units(
            &on.s,
            vec![
                set,
                verdict_unit(
                    1,
                    &format!("INSERT INTO {table} (a) VALUES (11)"),
                    UnitVerdict::TxThenConfirm,
                ),
            ],
            DEADLINE,
        );
        assert!(
            !asked(id, &ran).await,
            "{id}: autocommit on: Hacivat's own transaction, not a savepoint"
        );
        answer(id, &on.s, &ran, CommitAnswer::RollBack).await;
        assert_eq!(
            seen(&mut side, &table, 11).await,
            0,
            "{id}: the change was undone, never committed behind a Keep/Undo"
        );
    }

    on.shutdown().await;
    batch(&mut side, id, &format!("DROP TABLE {table}")).await;
    side.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn confirm_in_tx_on_the_matrix() {
    if !docker_enabled() {
        return;
    }
    for id in ["pg17", "mysql84"] {
        if let Some(t) = require_target(id) {
            server_leg(&t).await;
        }
    }

    // SQLite, in process: `BEGIN IMMEDIATE` for the own form, a second connection as the side
    // channel.
    let dir = tempfile::tempdir().expect("a temp directory");
    let path = dir.path().join("hv_t19.sqlite");
    let on = OnManager::connect(deps(), sqlite_profile(&path)).await;
    user("sqlite", &on.s, "CREATE TABLE t19 (a INTEGER)").await;
    let mut side = connect(
        &deps(),
        req(sqlite_profile(&path)),
        Interaction::Headless,
        token(),
        progress(),
    )
    .await
    .unwrap_or_else(|e| panic!("sqlite side channel: {:?}", e.class));
    both_forms("sqlite", DbmsKind::Sqlite, &on.s, &mut side, "t19").await;

    // ## m3b-t19-confirm-tx (fix round 1, I1, m6): a cancel interrupts the confirmed statement
    // inside the savepoint; SQLite rolls back the whole transaction with it. Nothing is asked,
    // no `ROLLBACK TO` is needed, the session stays usable, and the user's earlier insert is
    // gone (as the server did it).
    // Fix round 2 (R1-I2): no fixed sleep decides the assertions. The cancel goes out right
    // after `UnitStarted`; if it beat the statement (the transaction is still open: only the
    // statement was cancelled, the user's transaction is intact), the attempt is rolled back
    // and repeated with a short backoff, at most `ATTEMPTS` times. Every outcome is asserted.
    const ATTEMPTS: u64 = 10;
    let s = &on.s;
    let slow = "UPDATE t19 SET a = a WHERE (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL \
                SELECT x + 1 FROM c WHERE x < 1000000000) SELECT count(*) FROM c) > 0";
    let mut interrupted = false;
    for attempt in 0..ATTEMPTS {
        user("sqlite", s, "BEGIN").await;
        user("sqlite", s, "INSERT INTO t19 (a) VALUES (10)").await;
        let ran = confirmed(s, slow);
        wait_until("sqlite: the slow statement starts", LIMIT, || {
            ran.o
                .events()
                .iter()
                .any(|e| matches!(e, hacivat_session::session::RunEvent::UnitStarted { .. }))
        })
        .await;
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
        }
        let _ = s.cancel(ran.ticket.seq).await;
        ended("sqlite", &ran).await;
        assert!(commit_questions(&ran.o).is_empty(), "sqlite: never asked");
        assert_eq!(
            ran.o.unit_ends(),
            [hacivat_session::session::UnitEnd::Cancelled],
            "sqlite"
        );
        let tx = s.state().borrow().tx.clone();
        if tx.status == TxStatus::Idle {
            // The interrupt hit the running statement: SQLite rolled back the whole transaction.
            interrupted = true;
            break;
        }
        // The cancel beat the statement: the user's transaction is intact and theirs.
        assert_eq!(
            (tx.status, tx.owner),
            (TxStatus::InTx, hacivat_session::session::TxOwner::User),
            "sqlite: attempt {attempt}"
        );
        user("sqlite", s, "ROLLBACK").await;
    }
    assert!(
        interrupted,
        "sqlite: the cancel never reached the running statement in {ATTEMPTS} attempts"
    );
    user("sqlite", s, "SELECT 1").await;
    assert_eq!(
        seen(&mut side, "t19", 10).await,
        0,
        "sqlite: rolled back by the server"
    );
    side.close().await;
    on.shutdown().await;
}
