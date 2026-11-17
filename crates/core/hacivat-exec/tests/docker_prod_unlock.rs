//! M3b Task 26: production consoles are writable behind confirmations, over the Docker matrix
//! (R-M3-6 REVISED ends; R-M3-29, R-M3-52, R-M3-55, D-036, D-014). Every production case goes
//! the app's M3b way (`common::t28`: the console's document, `resolve_units`, `prepare_units`,
//! the real `ClassifyPool` over exactly the prepared bytes in the session's own context, `gate`
//! on the live connection, `ExecService::run`) on a real session of `ManagerConfig::m2b()`, the
//! app's configuration since Task 26; an admin side channel checks what the server holds.
//!
//! - **Lines.** `matrix::lines()`: pg17, mysql84 and SQLite under `pr`; PostgreSQL 14–18,
//!   MySQL 8.0/8.4, MariaDB 10.6/10.11/11.4 and SQLite under `full`. Each test asserts that every
//!   line ran (`Table::finish`, artifact `t28-t26-<test>.tsv`).
//! - **E2 of the gate:** nothing of a production statement that asks first is sent before an
//!   answer; a DML that runs then asks keeps nothing visible to anyone else before Keep and the
//!   user's `COMMIT`, and the D-014 deadline (2 s here, injected into the run service:
//!   `ExecService::with_confirm_deadline`) undoes it.
//! - **The T26 carries:** MySQL/MariaDB joined and comma `UPDATE` typed names on the servers
//!   (MUST 1); Run's context read on read-write sessions: PostgreSQL 14–18, MySQL with
//!   `DATABASE()` NULL, and a read inside the user's open transaction that never aborts it
//!   (MUST 4).
//!
//! Run with `<main>/target/docker/run-docker-tests.sh --set pr|full -p hacivat-exec --test
//! docker_prod_unlock`. Fixture names carry `hv_t31_` (Task 31's sweep drops what a red run
//! left) and are dropped at the end.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::matrix::{Line, admin, docker_enabled, ended, lines, pg_lines, unique};
use common::t28::{Con, Port, Spec, Table};
use hacivat_driver::TxStatus;
use hacivat_exec::classify_pool::ClassifyPool;
use hacivat_exec::{
    ConfirmAnswer, ConfirmPort, ErrorPolicy, ExecService, GateDecision, apply_answer,
};
use hacivat_proto::DbmsKind;
use hacivat_results::MemoryBudget;
use hacivat_session::session::{CommitAnswer, Request, TxCommand, TxMode, TxOwner};

const DEADLINE: Duration = Duration::from_secs(2);

fn pool() -> Arc<ClassifyPool> {
    ClassifyPool::new(2)
}

/// `CREATE TABLE` text of `name` with columns `cols` (MySQL family: InnoDB, utf8mb4).
fn create(line: &Line, name: &str, cols: &str) -> String {
    if line.is_mysql_family() {
        format!("CREATE TABLE {name} ({cols}) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4")
    } else {
        format!("CREATE TABLE {name} ({cols})")
    }
}

/// A production console of `line` whose run service carries `deadline` (the D-014 deadline,
/// injected for the test).
async fn production(line: &Line, pool: &Arc<ClassifyPool>, deadline: Duration) -> Con {
    let mut c = Con::open(line, Spec::production(), pool).await;
    c.svc = ExecService::new(MemoryBudget::new(4 << 30, 256 << 20))
        .with_classifier(pool.clone())
        .with_confirm_deadline(deadline);
    let snap = c.snap();
    assert!(!snap.read_only, "{}: production is read-write", line.id);
    assert_eq!(snap.tx.mode, TxMode::Manual, "{}: D-036", line.id);
    c
}

/// The user's Commit (the transaction badge's, `Request::Tx`).
async fn user_commit(c: &Con) {
    let r =
        c.s.submit(Request::Tx(TxCommand::Commit))
            .reply
            .await
            .expect("the actor replies");
    assert!(r.is_ok(), "{}: COMMIT: {r:?}", c.line.id);
}

/// The ids of the console's log lines.
fn log_ids(c: &Con) -> Vec<&'static str> {
    c.log.lines().iter().map(|l| l.msg.fluent_id()).collect()
}

/// The typed name a planned unit asks for.
fn typed(decision: &GateDecision) -> Option<String> {
    match decision {
        GateDecision::Confirm {
            type_to_confirm: Some(t),
            ..
        } => Some(t.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn production_dml_without_an_answer_leaves_nothing_visible() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines = lines();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_t26_dml");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "id int PRIMARY KEY, a int"))
            .await;
        a.batch(&format!("INSERT INTO {t} VALUES (1, 1), (2, 1), (3, 1)"))
            .await;
        let c = production(line, &pool, Duration::from_secs(300)).await;

        // E2: on PostgreSQL the server itself shows what the console's backend last ran.
        let backend = if line.is_pg() {
            c.internal("SELECT pg_backend_pid()").await
        } else {
            None
        };

        // DELETE without WHERE: the typed name; Cancel → nothing sent; unanswered → the run
        // service sends nothing either.
        let delete = format!("DELETE FROM {t}");
        let p = c.plan(&delete, &[]).await;
        assert_eq!(p.shapes(), ["confirm+type"], "{id}: {delete}");
        assert_eq!(
            typed(&p.plan.items[0].decision).as_deref(),
            Some(&*t),
            "{id}"
        );
        let mut cancelled = p.plan.clone();
        assert!(
            !apply_answer(&mut cancelled, 0, ConfirmAnswer::Cancel),
            "{id}: Cancel tells the caller to send nothing"
        );
        let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
        assert_eq!(ran.names(), ["Skipped"], "{id}: not sent unanswered");
        assert_eq!(
            a.count(&format!("SELECT count(*) FROM {t}")).await,
            3,
            "{id}"
        );
        if let Some(pid) = &backend {
            let last = a
                .value(&format!(
                    "SELECT query FROM pg_catalog.pg_stat_activity WHERE pid = {pid}"
                ))
                .await
                .unwrap_or_default();
            assert!(
                !last.contains("DELETE"),
                "{id}: the server never saw the DELETE: {last}"
            );
        }

        let update = format!("UPDATE {t} SET a = 2 WHERE id = 1");
        // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): on the MySQL family the
        // UPDATE is confirmed before it is sent (nothing sent unanswered; Execute, then the
        // user's COMMIT of the Manual transaction makes it visible).
        if line.is_mysql_family() {
            let mut p = c.plan(&update, &[]).await;
            assert_eq!(p.shapes(), ["confirm"], "{id}: {update}");
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.names(), ["Skipped"], "{id}: not sent unanswered");
            p.answer(ConfirmAnswer::Execute);
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.outcome(), "ran", "{id}");
            assert_eq!(
                a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
                "1",
                "{id}: in the user's transaction, not yet committed"
            );
            user_commit(&c).await;
            assert_eq!(
                a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
                "2",
                "{id}: committed by the user"
            );
            table.add(
                line,
                c.version.as_ref(),
                "DELETE unanswered / UPDATE confirmed first + commit",
                "nothing sent before an answer",
            );
            c.shutdown().await;
            a.batch(&format!("DROP TABLE {t}")).await;
            a.close().await;
            continue;
        }
        // UPDATE: runs in the user's transaction under the savepoint and asks; the side
        // channel never sees it; Undo keeps nothing.
        let p = c.plan(&update, &[]).await;
        assert_eq!(p.shapes(), ["tx-confirm"], "{id}: {update}");
        let port = Port::new();
        let run = c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        );
        let q = port.question(id, 1).await;
        assert!(q.savepoint && q.production, "{id}: {q:?}");
        let seen = a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await;
        assert_eq!(seen, "1", "{id}: invisible while asked");
        port.answer(CommitAnswer::RollBack);
        ended(run).await;
        c.settled().await;
        assert_eq!(
            a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
            "1",
            "{id}: Undo"
        );

        // Keep, then the user's COMMIT: only then is the row changed for everyone.
        let p = c.plan(&update, &[]).await;
        let port = Port::new();
        let run = c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        );
        port.question(id, 1).await;
        port.answer(CommitAnswer::Commit);
        ended(run).await;
        let snap = c.settled().await;
        assert_eq!(snap.tx.owner, TxOwner::User, "{id}: the user's transaction");
        assert_ne!(snap.tx.status, TxStatus::Failed, "{id}");
        assert_eq!(
            a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
            "1",
            "{id}: Keep keeps it in the user's transaction, not yet committed"
        );
        user_commit(&c).await;
        assert_eq!(
            a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
            "2",
            "{id}: committed by the user"
        );
        table.add(
            line,
            c.version.as_ref(),
            "DELETE unanswered / UPDATE undo / keep+commit",
            "nothing visible before the user's COMMIT",
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    table.finish("t26-dml-no-answer", &lines);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn production_dml_rolls_back_at_the_deadline() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines = lines();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_t26_deadline");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "id int PRIMARY KEY, a int"))
            .await;
        a.batch(&format!("INSERT INTO {t} VALUES (1, 1)")).await;
        // Through the run service, with the deadline injected there (the T28 carry).
        let c = production(line, &pool, DEADLINE).await;
        let p = c
            .plan(&format!("UPDATE {t} SET a = 7 WHERE id = 1"), &[])
            .await;
        // ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): the MySQL family asks
        // before it sends, so no deadline form exists there: nothing is sent unanswered.
        if line.is_mysql_family() {
            assert_eq!(p.shapes(), ["confirm"], "{id}");
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.names(), ["Skipped"], "{id}");
            assert_eq!(
                a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
                "1",
                "{id}"
            );
            table.add(line, c.version.as_ref(), "deadline", "asked before sending");
            c.shutdown().await;
            a.batch(&format!("DROP TABLE {t}")).await;
            a.close().await;
            continue;
        }
        assert_eq!(p.shapes(), ["tx-confirm"], "{id}");
        let port = Port::new();
        let started = std::time::Instant::now();
        let ran = ended(c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        ))
        .await;
        // Nobody answered: the deadline undid the statement, and only it.
        assert_eq!(port.asked().len(), 1, "{id}: asked once");
        assert!(
            started.elapsed() >= DEADLINE,
            "{id}: waited for the deadline"
        );
        assert!(
            c.log.lines().iter().any(|l| matches!(
                l.msg,
                hacivat_exec::LogMsg::ConfirmRolledBack {
                    by_deadline: true,
                    savepoint: true,
                }
            )),
            "{id}: {:?} ({})",
            log_ids(&c),
            ran.outcome()
        );
        let snap = c.settled().await;
        assert_eq!(
            snap.tx.owner,
            TxOwner::User,
            "{id}: the user's transaction stays"
        );
        assert_ne!(snap.tx.status, TxStatus::Failed, "{id}");
        user_commit(&c).await;
        assert_eq!(
            a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
            "1",
            "{id}: the deadline kept nothing, even after the user's COMMIT"
        );
        table.add(line, c.version.as_ref(), "deadline", "rolled back");
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    table.finish("t26-deadline", &lines);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn production_select_runs_without_a_dialog() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines = lines();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_t26_select");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "id int PRIMARY KEY")).await;
        a.batch(&format!("INSERT INTO {t} VALUES (1), (2)")).await;
        let c = production(line, &pool, Duration::from_secs(300)).await;
        let p = c.plan(&format!("SELECT count(*) FROM {t}"), &[]).await;
        assert_eq!(p.shapes(), ["run"], "{id}: no dialog");
        let port = Port::new();
        let ran = ended(c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        ))
        .await;
        assert_eq!(ran.outcome(), "ran", "{id}");
        assert!(port.asked().is_empty(), "{id}: nothing asked after either");
        table.add(line, c.version.as_ref(), "SELECT", "ran, no dialog");
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    table.finish("t26-select", &lines);
}

// ---------------------------------------------------------------------------------------------
// The T26 carries

/// MUST 1 (the T28 re-check): a joined or comma `UPDATE` on MySQL and MariaDB asks for the
/// table that changes on the server, two changed tables are denied, and what runs changes
/// exactly the table named.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn mysql_joined_and_comma_updates_name_the_table_that_changes() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines: Vec<Line> = lines().into_iter().filter(Line::is_mysql_family).collect();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let (ta, tb) = (unique("hv_t31_t26_a"), unique("hv_t31_t26_b"));
        let mut a = admin(line).await;
        a.batch(&create(line, &ta, "id int PRIMARY KEY, x int"))
            .await;
        a.batch(&create(line, &tb, "id int PRIMARY KEY, y int"))
            .await;
        a.batch(&format!("INSERT INTO {ta} VALUES (1, 1), (2, 1)"))
            .await;
        a.batch(&format!("INSERT INTO {tb} VALUES (1, 1), (2, 1)"))
            .await;
        let c = production(line, &pool, Duration::from_secs(300)).await;
        // ## m3b-t26-prod-unlock (fix round 1, A-M6): nested and parenthesised joins (A-C1),
        // the modifiers (A-C2; after `WITH` on MySQL, which runs a CTE before UPDATE/DELETE),
        // and aliases (B-C1): the name asked is always the changed table's, never an alias.
        let mut updates = vec![
            format!("UPDATE {ta} JOIN {tb} ON {ta}.id = {tb}.id SET {tb}.y = 2"),
            format!("UPDATE {ta}, {tb} SET {tb}.y = 3"),
            format!("UPDATE ({ta} JOIN {tb} ON {ta}.id = {tb}.id) SET {tb}.y = 4"),
            format!(
                "UPDATE {ta} AS p JOIN ({tb} AS q JOIN {ta} AS r ON q.id = r.id) ON p.id = q.id \
                 SET q.y = 5"
            ),
            format!("UPDATE LOW_PRIORITY IGNORE {ta}, {tb} SET {tb}.y = 6"),
        ];
        if line.engine == DbmsKind::MySql {
            updates.push(format!(
                "WITH w AS (SELECT 1) UPDATE LOW_PRIORITY {ta} JOIN {tb} ON {ta}.id = {tb}.id \
                 SET {tb}.y = 7"
            ));
        }
        for sql in updates {
            let mut p = c.plan(&sql, &[]).await;
            assert_eq!(
                typed(&p.plan.items[0].decision).as_deref(),
                Some(&*tb),
                "{id}: `{sql}` asks for the table that changes"
            );
            // The user typed it and ran it: only that table changes on the server.
            p.answer(ConfirmAnswer::Execute);
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.outcome(), "ran", "{id}: `{sql}`");
            user_commit(&c).await;
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {ta} WHERE x = 1"))
                    .await,
                2,
                "{id}: `{sql}` left {ta} alone"
            );
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {tb} WHERE y = 1"))
                    .await,
                0,
                "{id}: `{sql}` changed {tb}"
            );
        }
        for sql in [
            format!("UPDATE {ta} JOIN {tb} ON {ta}.id = {tb}.id SET {ta}.x = 5, {tb}.y = 5"),
            format!("UPDATE {ta}, {tb} SET {ta}.x = 5, {tb}.y = 5"),
        ] {
            let p = c.plan(&sql, &[]).await;
            assert_eq!(p.shapes(), ["deny"], "{id}: `{sql}`: several targets");
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.names(), ["Skipped"], "{id}: not sent");
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {ta} WHERE x = 5"))
                    .await,
                0,
                "{id}: `{sql}`"
            );
        }
        // Multi-table DELETE through aliases (B-C1): the name asked is the table's.
        let mut deletes = vec![
            format!("DELETE q FROM {ta} AS p JOIN {tb} AS q ON p.id = q.id"),
            format!("DELETE FROM q USING {ta} AS p JOIN {tb} AS q ON p.id = q.id"),
            format!("DELETE QUICK IGNORE {tb} FROM {ta} JOIN {tb} ON {ta}.id = {tb}.id"),
        ];
        if line.engine == DbmsKind::MySql {
            deletes.push(format!(
                "WITH w AS (SELECT 1) DELETE QUICK q FROM {ta} AS p JOIN {tb} AS q ON p.id = q.id"
            ));
        }
        for sql in deletes {
            a.batch(&format!("DELETE FROM {tb}")).await;
            a.batch(&format!("INSERT INTO {tb} VALUES (1, 1), (2, 1)"))
                .await;
            let mut p = c.plan(&sql, &[]).await;
            assert_eq!(
                typed(&p.plan.items[0].decision).as_deref(),
                Some(&*tb),
                "{id}: `{sql}` asks for the table that changes"
            );
            p.answer(ConfirmAnswer::Execute);
            let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
            assert_eq!(ran.outcome(), "ran", "{id}: `{sql}`");
            user_commit(&c).await;
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {tb}")).await,
                0,
                "{id}: `{sql}` emptied {tb}"
            );
            assert_eq!(
                a.count(&format!("SELECT count(*) FROM {ta}")).await,
                2,
                "{id}: `{sql}` left {ta} alone"
            );
        }
        table.add(
            line,
            c.version.as_ref(),
            "joined/comma/nested/aliased UPDATE and DELETE",
            "typed name = the changed table; two denied",
        );
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {ta}")).await;
        a.batch(&format!("DROP TABLE {tb}")).await;
        a.close().await;
    }
    table.finish("t26-mysql-update-targets", &lines);
}

/// MUST 4 (T24/T28): Run's context read on a read-write session (`SessionHandle::
/// run_context`) on PostgreSQL 14–18: the modes and the search path as the server has them,
/// with a generation that moves after the user changes the path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn run_context_reads_on_read_write_postgres() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines = pg_lines();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let c = Con::open(line, Spec::dev(), &pool).await;
        assert!(!c.snap().read_only, "{id}");
        let (ctx, g1) = c.s.run_context().await;
        assert_eq!(ctx.lex_modes.as_deref(), Some("on"), "{id}");
        let path = ctx.search_path.clone().expect("a search path");
        assert!(path.iter().any(|p| &**p == "public"), "{id}: {path:?}");
        let g1 = g1.expect("a generation");
        c.ok("SET search_path = pg_temp, public").await;
        let (ctx, g2) = c.s.run_context().await;
        let path = ctx.search_path.clone().expect("a search path");
        assert_eq!(
            path.first().map(|p| &**p),
            Some("pg_temp"),
            "{id}: {path:?}"
        );
        assert_ne!(g2, Some(g1), "{id}: the generation moved");
        table.add(
            line,
            c.version.as_ref(),
            "run context (read-write)",
            &format!("{path:?}"),
        );
        c.shutdown().await;
    }
    table.finish("t26-run-context-pg", &lines);
}

/// MUST 4: a MySQL/MariaDB session without a database (`DATABASE()` NULL): the context is read
/// (the `sql_mode`), with no search path, never a stale one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn run_context_reads_on_mysql_without_a_database() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines: Vec<Line> = lines().into_iter().filter(Line::is_mysql_family).collect();
    for line in &lines {
        let id = &line.id;
        let (m, s) = common::t26::without_database(line).await;
        assert!(!s.state().borrow().read_only, "{id}");
        let (ctx, g) = s.run_context().await;
        assert!(ctx.lex_modes.is_some(), "{id}: the sql_mode is read");
        assert_eq!(ctx.search_path, None, "{id}: no current database");
        assert!(g.is_some(), "{id}: a generation");
        table.add(line, None, "run context, DATABASE() NULL", "no search path");
        m.shutdown().await;
    }
    table.finish("t26-run-context-mysql-nodb", &lines);
}

/// MUST 4: the context read inside the user's open transaction on PostgreSQL and the MySQL
/// family never aborts it: the transaction stays open and commits what it holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn run_context_read_never_aborts_the_users_transaction() {
    if !docker_enabled() {
        return;
    }
    let table = Table::new();
    let lines: Vec<Line> = lines().into_iter().filter(|l| !l.is_sqlite()).collect();
    let pool = pool();
    for line in &lines {
        let id = &line.id;
        let t = unique("hv_t31_t26_ctx");
        let mut a = admin(line).await;
        a.batch(&create(line, &t, "id int PRIMARY KEY")).await;
        let c = Con::open(line, Spec::dev(), &pool).await;
        let begin = if line.engine == DbmsKind::Postgres {
            "BEGIN"
        } else {
            "START TRANSACTION"
        };
        c.ok(begin).await;
        // A write marks the context stale: the next answer reads it again, inside the
        // transaction.
        c.ok(&format!("INSERT INTO {t} VALUES (1)")).await;
        let (ctx, g) = c.s.run_context().await;
        assert!(ctx.lex_modes.is_some(), "{id}: read inside the transaction");
        assert!(g.is_some(), "{id}");
        let snap = c.settled().await;
        assert_eq!(snap.tx.status, TxStatus::InTx, "{id}: never aborted");
        assert_eq!(snap.tx.owner, TxOwner::User, "{id}");
        c.ok("COMMIT").await;
        assert_eq!(
            a.count(&format!("SELECT count(*) FROM {t}")).await,
            1,
            "{id}"
        );
        table.add(line, c.version.as_ref(), "read in the user's tx", "kept");
        c.shutdown().await;
        a.batch(&format!("DROP TABLE {t}")).await;
        a.close().await;
    }
    table.finish("t26-run-context-in-tx", &lines);
}

// ---------------------------------------------------------------------------------------------
// ## m3b-t26-prod-unlock (fix round 1, ruling on A-I2/B-I1): non-transactional tables (mysql84)

/// The mysql84 line: in every Docker set (`pr` and `full`), so a missing one fails the leg.
fn mysql84() -> Line {
    lines()
        .into_iter()
        .find(|l| l.id == "mysql84")
        .expect("mysql84 is in every Docker set")
}

/// On production a DML of a MyISAM table is confirmed before it is sent: unanswered, the server
/// never sees it (the row stays); after Execute it is written at once (MyISAM keeps no
/// transaction), so the confirmation is the only barrier.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn production_myisam_dml_is_confirmed_before_it_is_sent() {
    if !docker_enabled() {
        return;
    }
    let line = mysql84();
    let id = &line.id;
    let pool = pool();
    let t = unique("hv_t31_t26_myisam");
    let mut a = admin(&line).await;
    a.batch(&format!(
        "CREATE TABLE {t} (id int PRIMARY KEY, a int) ENGINE = MyISAM"
    ))
    .await;
    a.batch(&format!("INSERT INTO {t} VALUES (1, 1)")).await;
    let c = production(&line, &pool, Duration::from_secs(300)).await;
    let mut p = c
        .plan(&format!("UPDATE {t} SET a = 2 WHERE id = 1"), &[])
        .await;
    assert_eq!(p.shapes(), ["confirm"], "{id}: asked before it is sent");
    let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
    assert_eq!(ran.names(), ["Skipped"], "{id}: not sent unanswered");
    assert_eq!(
        a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
        "1",
        "{id}: nothing reached the server"
    );
    p.answer(ConfirmAnswer::Execute);
    let ran = ended(c.send(&p, ErrorPolicy::StopOnError, None)).await;
    assert_eq!(ran.outcome(), "ran", "{id}");
    assert_eq!(
        a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
        "2",
        "{id}: MyISAM writes at once, after the answer"
    );
    c.shutdown().await;
    a.batch(&format!("DROP TABLE {t}")).await;
    a.close().await;
}

/// On a development console the session's confirm-in-transaction form over a MyISAM table
/// (the plan forced to it, as the gate never plans it on this engine): Undo runs the rollback,
/// the server warns 1196, and the log says "not undone", never "undone" or "discarded"; the
/// unit fails with `ConfirmNotUndone`; the row keeps the change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "docker"]
async fn myisam_rollback_is_logged_as_not_undone() {
    if !docker_enabled() {
        return;
    }
    let line = mysql84();
    let id = &line.id;
    let pool = pool();
    let t = unique("hv_t31_t26_notundone");
    let mut a = admin(&line).await;
    a.batch(&format!(
        "CREATE TABLE {t} (id int PRIMARY KEY, a int) ENGINE = MyISAM"
    ))
    .await;
    a.batch(&format!("INSERT INTO {t} VALUES (1, 1)")).await;
    let c = Con::open(&line, Spec::dev(), &pool).await;
    for (mode, savepoint) in [(TxMode::Auto, false), (TxMode::Manual, true)] {
        c.set_mode(mode).await;
        let value = if savepoint { "3" } else { "2" };
        let mut p = c
            .plan(&format!("UPDATE {t} SET a = {value} WHERE id = 1"), &[])
            .await;
        let item = &mut p.plan.items[0];
        item.decision = GateDecision::RunInTxThenConfirm {
            reasons: Vec::new(),
        };
        item.verdict = hacivat_session::session::UnitVerdict::TxThenConfirm;
        let port = Port::new();
        let run = c.send(
            &p,
            ErrorPolicy::StopOnError,
            Some(port.clone() as Arc<dyn ConfirmPort>),
        );
        let q = port.question(id, 1).await;
        assert_eq!(q.savepoint, savepoint, "{id} {mode:?}");
        port.answer(CommitAnswer::RollBack);
        let ran = ended(run).await;
        c.settled().await;
        let ids = log_ids(&c);
        assert!(
            ids.contains(&"exec-confirm-not-undone"),
            "{id} {mode:?}: {ids:?} ({})",
            ran.outcome()
        );
        assert!(
            !ids.contains(&"exec-confirm-undone")
                && !ids.contains(&"exec-confirm-rolled-back")
                && !ids.contains(&"exec-confirm-rolled-back-deadline"),
            "{id} {mode:?}: never said undone: {ids:?}"
        );
        assert_eq!(
            ran.outcome(),
            "exec-error-confirm-not-undone",
            "{id} {mode:?}"
        );
        if savepoint {
            user_commit(&c).await;
        }
        assert_eq!(
            a.scalar(&format!("SELECT a FROM {t} WHERE id = 1")).await,
            value,
            "{id} {mode:?}: the change stayed"
        );
    }
    c.shutdown().await;
    a.batch(&format!("DROP TABLE {t}")).await;
    a.close().await;
}
