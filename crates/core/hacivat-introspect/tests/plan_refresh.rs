// ## m4-t21-refresh
//! Refresh after DDL (M4 Task 21; alg/02 HV-INTRO-12 test cases 1–9 and 11, edge cases 1–14,
//! the SQL Server rows excluded): the planner over `sql-safety`'s own reports, the run tracker
//! that turns a run's events into messages, and the coordinator over the engine and its fakes.
//! The Docker cases (alg/02 tests 6–9 on real servers) are `docker_refresh_after_ddl.rs`.

#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::*;
use hacivat_introspect::IntroError;
use hacivat_introspect::ddl_effects::{
    ExecutedStmt, RefreshPlan, SessionStateAtExec, Target, TxnState, WIDEN_OVER, lower,
    plan_refresh, plan_refresh_in,
};
use hacivat_introspect::engine::IntroHandle;
use hacivat_introspect::introspector::IntroCaps;
use hacivat_introspect::refresh_coord::{
    CoordEvent, RefreshCoordinator, RefreshInbox, RefreshMsg, RunTracker, TrackedUnit, TxControl,
    TxPhase, tx_control,
};
use hacivat_introspect::scope::NsInfo;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::fixture::{ns_pg, ns_sqlite, table_subtree};
use hacivat_meta::merge::Trigger;
use hacivat_meta::name::IdentCtx;
use hacivat_meta::props::{ColumnProps, Props, TableProps, TypeRef};
use hacivat_meta::snapshot::{CatalogChange, CatalogSnapshot, ContainerKey, LoadState};
use hacivat_meta::{
    Catalog, Level, Name, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, ServerFacts,
    ServerId, Subtree, Watermark,
};
use hacivat_proto::{AccessLabel, DbmsKind};
use hacivat_session::meta::MetaError;
use hacivat_session::session::{
    ExecError, ExecErrorKind, RunId, SchemaCtx, SessionId, SessionSnapshot, TxOutcome, TxState,
    UnitEnd, UnitId,
};
use hacivat_settings::RefreshAfterDdl;
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_safety::{ClassifyCtx, Expect, StmtReport, classify};
use tokio::sync::watch;

// ---- helpers ---------------------------------------------------------------------------------

/// The database of every PostgreSQL case (the `hacivat-meta` fixtures' own).
const DB: &str = "app";
const S: SessionId = SessionId(7);

fn ns(schema: &str) -> NsKey {
    pg_ns(DB, schema)
}

/// The key of PostgreSQL table `schema.table` (no server id).
fn k(dotted: &str) -> ObjectKey {
    let (s, t) = dotted.split_once('.').expect("schema.table");
    table_key(&ns(s), t, None)
}

/// [`k`] with its OID.
fn kid(dotted: &str, oid: u32) -> ObjectKey {
    let (s, t) = dotted.split_once('.').expect("schema.table");
    table_key(&ns(s), t, Some(oid))
}

fn dialect(dbms: DbmsKind) -> DialectId {
    match dbms {
        DbmsKind::MySql => DialectId::MySql,
        DbmsKind::MariaDb => DialectId::MariaDb,
        DbmsKind::Sqlite => DialectId::Sqlite,
        _ => DialectId::Postgres,
    }
}

/// `sql-safety`'s reports of `sql` (one or more statements).
fn reports(dbms: DbmsKind, sql: &str) -> Vec<StmtReport> {
    let ctx = ClassifyCtx::new(
        DialectCtx::new(dialect(dbms), None),
        ClientFlavor::None,
        Expect::Script,
    );
    let r = classify(sql, &ctx);
    assert!(r.pre_decision.is_none(), "{sql}: {:?}", r.pre_decision);
    r.items
}

fn stmts(dbms: DbmsKind, sql: &str) -> Vec<ExecutedStmt> {
    reports(dbms, sql)
        .iter()
        .map(ExecutedStmt::from_report)
        .collect()
}

/// Where and how statements ran.
#[derive(Clone)]
struct Ctx {
    dbms: DbmsKind,
    txn: TxnState,
    state: SessionStateAtExec,
}

fn at(schema: &str, path: &[&str]) -> SessionStateAtExec {
    SessionStateAtExec {
        current: ns(schema),
        search_path: path.iter().map(|s| ns(s)).collect(),
    }
}

fn pg_autocommit() -> Ctx {
    Ctx {
        dbms: DbmsKind::Postgres,
        txn: TxnState::AutoCommit,
        state: at("public", &["public"]),
    }
}

fn pg_in_tx() -> Ctx {
    Ctx {
        txn: TxnState::InTransaction,
        ..pg_autocommit()
    }
}

fn shop() -> NsKey {
    NsKey::schema(["shop"])
}

fn mysql_in_tx() -> Ctx {
    Ctx {
        dbms: DbmsKind::MySql,
        txn: TxnState::InTransaction,
        state: SessionStateAtExec {
            current: shop(),
            search_path: vec![shop()],
        },
    }
}

fn main_ns() -> NsKey {
    NsKey::schema(["main"])
}

fn sqlite(txn: TxnState) -> Ctx {
    Ctx {
        dbms: DbmsKind::Sqlite,
        txn,
        state: SessionStateAtExec {
            current: main_ns(),
            search_path: vec![main_ns()],
        },
    }
}

/// The key of table `t` of namespace `ns`.
fn key_in(ns: &NsKey, t: &str) -> ObjectKey {
    table_key(ns, t, None)
}

fn plan(sqls: &[&str], c: Ctx) -> RefreshPlan {
    plan_in(sqls, c, None)
}

fn plan_in(sqls: &[&str], c: Ctx, snap: Option<&CatalogSnapshot>) -> RefreshPlan {
    let all: Vec<ExecutedStmt> = sqls.iter().flat_map(|s| stmts(c.dbms, s)).collect();
    plan_refresh_in(&all, &c.state, c.txn, &IntroCaps::planned(c.dbms), snap)
}

/// A PostgreSQL catalog holding `namespaces`, each loaded at `Structure`.
fn snapshot(namespaces: Vec<NamespaceSnapshot>) -> Arc<CatalogSnapshot> {
    snapshot_of(DbmsKind::Postgres, namespaces)
}

/// A catalog of `dbms` holding `namespaces`, each loaded at `Structure`.
fn snapshot_of(dbms: DbmsKind, namespaces: Vec<NamespaceSnapshot>) -> Arc<CatalogSnapshot> {
    let cat = Catalog::new(Arc::new(ServerFacts::new(
        dbms,
        IdentCtx::new(dialect(dbms)),
    )));
    let p = cat.publisher().expect("the only writer");
    let mut changes = Vec::new();
    for n in namespaces {
        let key = n.key().clone();
        changes.push(CatalogChange::Put(Arc::new(n)));
        changes.push(CatalogChange::SetState(
            ContainerKey::Namespace(key),
            LoadState::Loaded {
                level: Level::Structure,
                at: SystemTime::UNIX_EPOCH,
                watermark: Watermark::None,
                stale: false,
            },
        ));
    }
    p.publish(changes).expect("published");
    cat.current()
}

/// Table `name` (OID `oid`) with one column `m` of type `ty_schema.ty`.
fn typed_table(name: &str, oid: u32, ty_schema: &str, ty: &str) -> Subtree {
    let mut t = Subtree::new(
        Obj::new(ObjectKind::Table, name)
            .with_server_id(ServerId::Oid(oid))
            .with_props(Props::Table(Arc::new(TableProps::default()))),
    );
    let root = t.root();
    t.add(
        root,
        Obj::new(ObjectKind::Column, "m").with_props(Props::Column(Arc::new(ColumnProps {
            ordinal: 1,
            raw_type: Name::from(ty),
            type_ref: Some(TypeRef {
                schema: Some(Name::from(ty_schema)),
                name: Name::from(ty),
            }),
            nullable: true,
            ..Default::default()
        }))),
    );
    t
}

/// An engine over a fake PostgreSQL server whose `public` holds `objects`, with `public`
/// loaded.
async fn loaded_rig(objects: Vec<FakeObject>) -> Rig {
    let fake = FakeIntrospector::new(DbmsKind::Postgres);
    fake.put_ns(ns("public"), objects);
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current(DB, "public"),
            ..RigOpts::default()
        },
    );
    // One user refresh loads it: its ticket resolves once the pass is merged (an expansion
    // answers after the names and may start the full pass a moment later).
    bounded(
        r.handle
            .refresh(Target::Namespace(ns("public")), Trigger::User),
    )
    .await
    .unwrap();
    idle(&r.handle).await;
    r
}

fn coordinator(h: &IntroHandle, dbms: DbmsKind, pref: RefreshAfterDdl) -> RefreshCoordinator {
    RefreshCoordinator::new(h.clone(), dbms, pref, tokio::runtime::Handle::current())
}

fn ns_state(r: &Rig, n: &NsKey) -> LoadState {
    r.catalog
        .current()
        .state(&ContainerKey::Namespace(n.clone()))
        .clone()
}

/// The keys of every fragment the engine asked the fake for.
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

fn names(keys: &[ObjectKey]) -> Vec<String> {
    keys.iter().map(|k| k.name().to_string()).collect()
}

// ---- alg/02 HV-INTRO-12 test cases and edge cases: the planner -------------------------------

/// Test case 1.
#[test]
fn create_table_autocommit_refreshes_object_and_names_now() {
    let p = plan(&["create table s1.t(x int)"], pg_autocommit());
    assert_eq!(
        p.now,
        vec![Target::Object(k("s1.t")), Target::NamespaceNames(ns("s1"))]
    );
    assert!(p.pending.is_empty());
}

/// Test case 2: the namespace list is refreshed, and the scope refresh removes the dropped
/// schema from the catalog; with `CASCADE` every loaded namespace of the database too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_schema_removes_the_namespace_and_refreshes_scope() {
    let p = plan(&["drop schema s2"], pg_autocommit());
    assert_eq!(p.now, vec![Target::ScopeRefresh]);
    assert_eq!(
        plan(&["drop schema s2 cascade"], pg_autocommit()).now,
        vec![Target::Database(Name::from(DB)), Target::ScopeRefresh]
    );
    let fake = FakeIntrospector::new(DbmsKind::Postgres);
    fake.put_ns(ns("public"), vec![FakeObject::table("t", &["a"])]);
    fake.put_ns(ns("s2"), vec![FakeObject::table("x", &["a"])]);
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current(DB, "public"),
            ..RigOpts::default()
        },
    );
    bounded(r.handle.expand(ns("s2"))).await.unwrap();
    assert!(r.catalog.current().namespaces.contains_key(&ns("s2")));
    // The server dropped it.
    r.fake.set_listing(vec![NsInfo {
        key: ns("public"),
        system: false,
    }]);
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    bounded(c.submit(S, p)).await.unwrap();
    assert!(!r.catalog.current().namespaces.contains_key(&ns("s2")));
}

/// Test case 3 and edge case 9: an unqualified name follows the search path **at execution
/// time** (captured with the statement), not the session's later state; the catalog's own key
/// (with its OID) is refreshed, so the rename keeps its identity.
#[test]
fn rename_uses_the_search_path_at_execution() {
    let snap = snapshot(vec![
        ns_pg(DB, "s2").table_oid("t", 501, &["a"]).finish(),
        ns_pg(DB, "public").table_oid("other", 600, &["a"]).finish(),
    ]);
    let in_s2 = Ctx {
        state: at("s2", &["s2", "public"]),
        ..pg_autocommit()
    };
    assert_eq!(
        plan_in(&["alter table t rename to u"], in_s2, Some(&snap)).now,
        vec![
            Target::Object(kid("s2.t", 501)),
            Target::NamespaceNames(ns("s2")),
            Target::Object(k("s2.u")),
        ]
    );
    // `public` first on the path: `public` holds no `t`, `s2` still does.
    let public_first = Ctx {
        state: at("public", &["public", "s2"]),
        ..pg_autocommit()
    };
    assert_eq!(
        plan_in(&["alter table t rename to u"], public_first, Some(&snap)).now,
        vec![
            Target::Object(kid("s2.t", 501)),
            Target::NamespaceNames(ns("s2")),
            Target::Object(k("s2.u")),
        ]
    );
    // Only `public` on the path, which holds no relation `t`: nothing in the catalog matches,
    // so its namespace is refreshed (T6 carry: widen when nothing matches).
    let only_public = Ctx {
        state: at("public", &["public"]),
        ..pg_autocommit()
    };
    assert_eq!(
        plan_in(&["alter table t rename to u"], only_public, Some(&snap)).now,
        vec![Target::Namespace(ns("public"))]
    );
}

/// Test case 4: `CREATE DATABASE` refreshes the server's database list, at once even inside a
/// session that reports a transaction (it cannot run in one).
#[test]
fn create_database_refreshes_the_data_source_scope() {
    assert_eq!(
        plan(&["create database d"], pg_autocommit()).now,
        vec![Target::ScopeRefresh]
    );
    let p = plan(&["create database d"], pg_in_tx());
    assert_eq!((p.now, p.pending), (vec![Target::ScopeRefresh], vec![]));
}

/// Test case 5; DML and `TRUNCATE` (U-3) too.
#[test]
fn select_has_no_targets() {
    for sql in [
        "select 1",
        "insert into t values (1)",
        "update t set a = 1 where id = 1",
        "truncate table a, b cascade",
    ] {
        assert_eq!(
            plan(&[sql], pg_autocommit()),
            RefreshPlan::default(),
            "{sql}"
        );
    }
}

/// Test case 6 (the planner part): dynamic SQL refreshes the current namespace, and a namespace
/// the text names literally.
#[test]
fn do_block_refreshes_the_current_namespace() {
    assert_eq!(
        plan(
            &["DO $$ BEGIN EXECUTE 'create table z(a int)'; END $$"],
            pg_autocommit()
        )
        .now,
        vec![Target::Namespace(ns("public"))]
    );
    assert_eq!(
        plan(&["call s2.p()"], pg_autocommit()).now,
        vec![Target::Namespace(ns("public")), Target::Namespace(ns("s2"))]
    );
}

/// Edge case 1: only successful statements count. The run tracker sends nothing for a failed
/// unit.
#[test]
fn failed_statement_produces_no_target() {
    let (tx, rx) = watch::channel(session_at("public", &["public"]));
    let _keep = tx;
    let inbox = RefreshInbox::with_capacity(16);
    let mut t = RunTracker::new(
        S,
        DbmsKind::Postgres,
        vec![
            tracked(0, DbmsKind::Postgres, "create table public.a(x int)"),
            tracked(1, DbmsKind::Postgres, "create table public.b(x int)"),
        ],
        rx,
        TxPhase::Idle,
        inbox.clone(),
    );
    t.unit_started(unit(0));
    t.unit_ended(unit(0), &UnitEnd::Failed(ExecError::of(ExecErrorKind::Db)));
    assert!(inbox.is_empty(), "{:?}", inbox.queued());
    t.unit_started(unit(1));
    t.unit_ended(unit(1), &UnitEnd::Succeeded { rows: None });
    let q = inbox.queued();
    assert_eq!(q.len(), 1);
    let RefreshMsg::Executed { stmts, txn, .. } = &q[0] else {
        panic!("{q:?}");
    };
    assert_eq!(*txn, TxnState::AutoCommit);
    let p = plan_refresh(
        stmts,
        &at("public", &["public"]),
        *txn,
        &IntroCaps::planned(DbmsKind::Postgres),
    );
    assert_eq!(
        p.now,
        vec![
            Target::Object(k("public.b")),
            Target::NamespaceNames(ns("public"))
        ]
    );
}

/// Edge case 2: dropping a table that did not exist refreshes harmlessly: the fragment reads
/// nothing, the namespace keeps its objects and stays loaded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_table_if_exists_missing_is_harmless() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let p = plan(&["drop table if exists public.x"], pg_autocommit());
    assert_eq!(
        p.now,
        vec![
            Target::Object(k("public.x")),
            Target::NamespaceNames(ns("public"))
        ]
    );
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    bounded(c.submit(S, p)).await.unwrap();
    idle(&r.handle).await;
    let snap = r.catalog.current();
    let arena = snap.namespaces.get(&ns("public")).expect("still loaded");
    assert!(arena.find(&kid("public.t", 501)).is_some());
    assert!(matches!(
        ns_state(&r, &ns("public")),
        LoadState::Loaded { .. }
    ));
}

/// alg/02 step 2: a PostgreSQL `DROP INDEX i` names no table; the catalog finds it, else the
/// namespace is refreshed.
#[test]
fn pg_drop_index_resolves_its_table_from_the_snapshot_or_widens() {
    let snap = snapshot(vec![
        ns_pg(DB, "public")
            .subtree(table_subtree("public", "orders", 3, 700, None))
            .finish(),
    ]);
    assert_eq!(
        plan_in(
            &["drop index orders_created_at_idx"],
            pg_autocommit(),
            Some(&snap)
        )
        .now,
        vec![
            Target::NamespaceNames(ns("public")),
            Target::Object(kid("public.orders", 700)),
        ]
    );
    // An index the catalog does not hold, or no catalog at all: the namespace.
    assert_eq!(
        plan_in(&["drop index nope"], pg_autocommit(), Some(&snap)).now,
        vec![Target::Namespace(ns("public"))]
    );
    assert_eq!(
        plan(
            &["drop index public.orders_created_at_idx"],
            pg_autocommit()
        )
        .now,
        vec![Target::Namespace(ns("public"))]
    );
    // `CREATE INDEX … ON t` names its table.
    assert_eq!(
        plan_in(
            &["create index i2 on orders (a)"],
            pg_autocommit(),
            Some(&snap)
        )
        .now,
        vec![
            Target::Object(kid("public.orders", 700)),
            Target::NamespaceNames(ns("public")),
        ]
    );
}

/// alg/02 step 2: `ADD FOREIGN KEY … REFERENCES u` refreshes `u` too (its referencing set).
#[test]
fn fk_refreshes_both_tables() {
    let snap = snapshot(vec![
        ns_pg(DB, "public")
            .table_oid("orders", 700, &["id", "cid"])
            .table_oid("customers", 800, &["id"])
            .finish(),
    ]);
    assert_eq!(
        plan_in(
            &["alter table orders add constraint f foreign key (cid) references customers(id)"],
            pg_autocommit(),
            Some(&snap)
        )
        .now,
        vec![
            Target::Object(kid("public.customers", 800)),
            Target::Object(kid("public.orders", 700)),
        ]
    );
}

/// alg/02 step 2: `ALTER TYPE` refreshes the type's namespace and every table of the database
/// with a column of that type.
#[test]
fn alter_type_refreshes_dependent_tables() {
    let snap = snapshot(vec![
        ns_pg(DB, "s1")
            .subtree(typed_table("t1", 901, "s1", "mood"))
            .finish(),
        ns_pg(DB, "s2")
            .subtree(typed_table("t2", 902, "s1", "mood"))
            .subtree(typed_table("t3", 903, "s2", "other"))
            .finish(),
    ]);
    assert_eq!(
        plan_in(
            &["alter type s1.mood add value 'c'"],
            pg_autocommit(),
            Some(&snap)
        )
        .now,
        vec![
            Target::Namespace(ns("s1")),
            Target::Object(kid("s2.t2", 902)),
        ]
    );
}

/// alg/02 step 3: more than 50 object targets in one namespace become the namespace.
#[test]
fn more_than_50_targets_widen_to_namespace() {
    let script = |n: usize| {
        (0..n)
            .map(|i| format!("create table s1.t{i}(a int)"))
            .collect::<Vec<_>>()
    };
    let many = script(WIDEN_OVER + 1);
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(
        plan(&refs, pg_autocommit()).now,
        vec![Target::Namespace(ns("s1"))]
    );
    let fifty = script(WIDEN_OVER);
    let refs: Vec<&str> = fifty.iter().map(String::as_str).collect();
    let now = plan(&refs, pg_autocommit()).now;
    assert_eq!(now.len(), WIDEN_OVER + 1);
    assert!(now.contains(&Target::NamespaceNames(ns("s1"))));
}

/// Edge case 5: PostgreSQL `CREATE INDEX CONCURRENTLY` cannot run in a transaction block, so it
/// is refreshed at once; a plain `CREATE INDEX` in a transaction waits for the commit.
#[test]
fn create_index_concurrently_is_immediate() {
    let p = plan(&["create index concurrently i on s1.t (a)"], pg_in_tx());
    assert!(!p.now.is_empty());
    assert!(p.pending.is_empty());
    let p = plan(&["create index i on s1.t (a)"], pg_in_tx());
    assert!(p.now.is_empty());
    assert_eq!(
        p.pending,
        vec![Target::Object(k("s1.t")), Target::NamespaceNames(ns("s1"))]
    );
}

/// Edge case 4: `SAVEPOINT s; CREATE TABLE b; ROLLBACK TO s; COMMIT` refreshes what ran
/// before the savepoint only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn savepoint_rollback_discards_later_targets() {
    let r = loaded_rig(vec![
        FakeObject::table("t1", &["a"]).with_oid(501),
        FakeObject::table("t2", &["a"]).with_oid(502),
    ])
    .await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    let inbox = c.inbox();
    let exec = |sql: &str| RefreshMsg::Executed {
        session: S,
        stmts: stmts(DbmsKind::Postgres, sql),
        state: at("public", &["public"]),
        txn: TxnState::InTransaction,
    };
    let before = r.fake.calls().len();
    inbox.push(exec("alter table public.t1 add column z int"));
    inbox.push(RefreshMsg::Savepoint {
        session: S,
        name: Arc::from("s"),
    });
    inbox.push(exec("alter table public.t2 add column z int"));
    inbox.push(RefreshMsg::TxEnded {
        session: S,
        outcome: TxOutcome::RolledBackTo(Arc::from("s")),
    });
    let want = vec![Target::Object(kid("public.t1", 501))];
    wait_until(Duration::from_secs(30), || {
        inbox.is_empty() && c.pending(S) == want
    })
    .await;
    // Nothing was read while the transaction was open.
    assert_eq!(r.fake.calls().len(), before);
    inbox.push(RefreshMsg::TxEnded {
        session: S,
        outcome: TxOutcome::Committed,
    });
    wait_until(Duration::from_secs(30), || {
        names(&fragment_keys(&r)).contains(&"t1".to_string())
    })
    .await;
    idle(&r.handle).await;
    assert!(!names(&fragment_keys(&r)).contains(&"t2".to_string()));
    assert!(c.pending(S).is_empty());
}

/// Edge case 14: `never` marks the targets' namespace stale and reads nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_preference_marks_targets_stale() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"])]).await;
    let calls = r.fake.calls().len();
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Never);
    let mut events = c.events();
    bounded(c.submit(S, plan(&["create table public.n(a int)"], pg_autocommit())))
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || {
        matches!(
            ns_state(&r, &ns("public")),
            LoadState::Loaded { stale: true, .. }
        )
    })
    .await;
    assert_eq!(
        bounded(events.recv()).await.unwrap(),
        CoordEvent::Stale {
            namespaces: vec![ns("public")],
            scope: false,
        }
    );
    idle(&r.handle).await;
    assert_eq!(r.fake.calls().len(), calls);
}

/// Edge case 13: the commit was acknowledged, the refresh lost its connection: the targets are
/// owed and refreshed after the reconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_survives_a_reconnect_after_the_commit_ack() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    let p = plan_in(
        &["alter table public.t add column z int"],
        pg_in_tx(),
        Some(&r.catalog.current()),
    );
    assert!(p.now.is_empty());
    assert_eq!(p.pending, vec![Target::Object(kid("public.t", 501))]);
    bounded(c.submit(S, p.clone())).await.unwrap();
    assert_eq!(c.pending(S), p.pending);
    // The engine restarts a lost task once: both attempts fail.
    r.fake.fail(
        |call| matches!(call, FakeCall::Fragment { .. }),
        IntroError::Meta(MetaError::ConnectionLost),
        2,
    );
    let got = bounded(c.tx_ended(S, TxOutcome::Committed)).await;
    assert_eq!(got, Err(IntroError::Meta(MetaError::ConnectionLost)));
    assert!(c.pending(S).is_empty());
    assert_eq!(c.owed(), p.pending);
    let before = fragment_keys(&r).len();
    bounded(c.reconnected()).await.unwrap();
    assert!(c.owed().is_empty());
    assert!(fragment_keys(&r)[before..].contains(&kid("public.t", 501)));
}

/// alg/02 step 6: the refreshed objects are checked afterwards (a pooler or a lagging node).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_is_followed_by_check_state() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    let p = plan_in(
        &["alter table public.t add column z int"],
        pg_autocommit(),
        Some(&r.catalog.current()),
    );
    assert_eq!(p.now, vec![Target::Object(kid("public.t", 501))]);
    let before = r.fake.calls().len();
    bounded(c.submit(S, p)).await.unwrap();
    let after: Vec<FakeCall> = r.fake.calls()[before..].to_vec();
    let fragment = after
        .iter()
        .position(
            |c| matches!(c, FakeCall::Fragment { keys, .. } if *keys == vec![kid("public.t", 501)]),
        )
        .expect("a fragment of t");
    let check = after
        .iter()
        .position(
            |c| matches!(c, FakeCall::CheckState { keys } if *keys == vec![kid("public.t", 501)]),
        )
        .expect("a state check of t");
    assert!(fragment < check, "{after:?}");
}

/// R-M4-42: SQLite DDL inside an explicit transaction waits for the commit (the `mode=ro`
/// metadata connection cannot see it before), then is refreshed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_ddl_in_explicit_txn_appears_after_commit() {
    let p = plan(&["create table q(a)"], sqlite(TxnState::InTransaction));
    assert!(p.now.is_empty());
    assert_eq!(
        p.pending,
        vec![
            Target::Object(key_in(&main_ns(), "q")),
            Target::NamespaceNames(main_ns())
        ]
    );
    let r = sqlite_rig().await;
    let c = coordinator(&r.handle, DbmsKind::Sqlite, RefreshAfterDdl::Always);
    let before = r.fake.calls().len();
    bounded(c.submit(S, p.clone())).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(r.fake.calls().len(), before, "nothing before the commit");
    assert_eq!(c.pending(S), p.pending);
    bounded(c.tx_ended(S, TxOutcome::Committed)).await.unwrap();
    assert!(
        r.fake.calls()[before..]
            .iter()
            .any(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == main_ns())),
        "{:?}",
        r.fake.calls()
    );
}

/// R-M4-42: SQLite DDL rolled back queries nothing (no compensating refresh).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_rolled_back_ddl_queries_nothing() {
    let r = sqlite_rig().await;
    let c = coordinator(&r.handle, DbmsKind::Sqlite, RefreshAfterDdl::Always);
    let before = r.fake.calls().len();
    let p = plan(&["create table q(a)"], sqlite(TxnState::InTransaction));
    bounded(c.submit(S, p)).await.unwrap();
    bounded(c.tx_ended(S, TxOutcome::RolledBack)).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(r.fake.calls().len(), before);
    assert!(c.pending(S).is_empty());
}

/// R-M4-42: SQLite DDL in autocommit is refreshed at once.
#[test]
fn sqlite_autocommit_ddl_refreshes_now() {
    let p = plan(&["create table q(a)"], sqlite(TxnState::AutoCommit));
    assert_eq!(
        p.now,
        vec![
            Target::Object(key_in(&main_ns(), "q")),
            Target::NamespaceNames(main_ns())
        ]
    );
    assert!(p.pending.is_empty());
    // `temp` objects are the session's own (M5's overlay): nothing.
    assert_eq!(
        plan(&["create temp table q(a)"], sqlite(TxnState::AutoCommit)),
        RefreshPlan::default()
    );
}

/// M7's Modify Object always refreshes: `refresh_now` ignores `never`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_now_ignores_never_preference() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Never);
    bounded(c.refresh_now(vec![Target::Object(kid("public.t", 501))]))
        .await
        .unwrap();
    assert!(fragment_keys(&r).contains(&kid("public.t", 501)));
}

// ---- more of the planner -----------------------------------------------------------------------

/// Edge case 6 (ver/01 `#mysql-ro`): MySQL DDL commits on its own, so it is refreshed at once
/// even inside a transaction.
#[test]
fn mysql_ddl_inside_a_transaction_is_refreshed_now() {
    let p = plan(&["create table m(a int)"], mysql_in_tx());
    assert_eq!(
        p.now,
        vec![
            Target::Object(key_in(&shop(), "m")),
            Target::NamespaceNames(shop())
        ]
    );
    assert!(p.pending.is_empty());
    // Edge case 10: `USE other; CREATE TABLE t` arrives qualified by the session's state; a
    // qualified name goes to its own database.
    let p = plan(&["create table other.t(a int)"], mysql_in_tx());
    assert_eq!(
        p.now,
        vec![
            Target::Object(key_in(&NsKey::schema(["other"]), "t")),
            Target::NamespaceNames(NsKey::schema(["other"]))
        ]
    );
}

/// U-2: `DROP OWNED` and `REASSIGN OWNED` name no object: every loaded namespace of the
/// session's database, and the namespace list.
#[test]
fn drop_owned_refreshes_every_namespace_of_the_database() {
    for sql in ["drop owned by r", "reassign owned by r to s"] {
        assert_eq!(
            plan(&[sql], pg_autocommit()).now,
            vec![Target::Database(Name::from(DB)), Target::ScopeRefresh],
            "{sql}"
        );
    }
    // Lowered on the catalog: the loaded namespaces of `app`, never another database's.
    let snap = snapshot(vec![
        ns_pg(DB, "public").table("t", &["a"]).finish(),
        ns_pg(DB, "s1").table("t", &["a"]).finish(),
        ns_pg("other", "public").table("t", &["a"]).finish(),
    ]);
    assert_eq!(
        lower(&plan(&["drop owned by r"], pg_autocommit()).now, &snap),
        vec![
            Target::ScopeRefresh,
            Target::Namespace(ns("public")),
            Target::Namespace(ns("s1")),
        ]
    );
}

/// T6 review carry: a run the classifier did not analyse (a script over 1 MiB) is opaque as a
/// whole.
#[test]
fn an_unanalysed_run_refreshes_the_whole_database() {
    let p = plan_refresh(
        &[ExecutedStmt::unanalysed()],
        &at("public", &["public"]),
        TxnState::AutoCommit,
        &IntroCaps::planned(DbmsKind::Postgres),
    );
    assert_eq!(
        p.now,
        vec![Target::Database(Name::from(DB)), Target::ScopeRefresh]
    );
}

/// Carry m-B4: a DDL statement naming another schema refreshes that schema, not the current
/// one; temporary objects refresh nothing.
#[test]
fn a_ddl_naming_another_schema_refreshes_that_schema() {
    assert_eq!(
        plan(&["create table s9.t(a int)"], pg_autocommit()).now,
        vec![Target::Object(k("s9.t")), Target::NamespaceNames(ns("s9"))]
    );
    for sql in [
        "create temp table x(a int)",
        "create table pg_temp.x(a int)",
    ] {
        assert_eq!(
            plan(&[sql], pg_autocommit()),
            RefreshPlan::default(),
            "{sql}"
        );
    }
}

/// T6 review carry: grants on a schema and on all its tables refresh the namespace.
#[test]
fn grants_on_a_schema_refresh_the_namespace() {
    for sql in [
        "grant select on all tables in schema s1 to r",
        "grant usage on schema s1 to r",
    ] {
        assert_eq!(
            plan(&[sql], pg_autocommit()).now,
            vec![Target::Namespace(ns("s1"))],
            "{sql}"
        );
    }
    // A column comment refreshes its table.
    assert_eq!(
        plan(&["comment on column s1.t.c is 'x'"], pg_autocommit()).now,
        vec![Target::Object(k("s1.t"))]
    );
}

/// A move to another schema refreshes both schemas.
#[test]
fn set_schema_refreshes_both_namespaces() {
    let snap = snapshot(vec![
        ns_pg(DB, "public").table_oid("t", 501, &["a"]).finish(),
    ]);
    assert_eq!(
        plan_in(
            &["alter table t set schema s2"],
            pg_autocommit(),
            Some(&snap)
        )
        .now,
        vec![
            Target::Object(kid("public.t", 501)),
            Target::NamespaceNames(ns("public")),
            Target::NamespaceNames(ns("s2")),
            Target::Object(k("s2.t")),
        ]
    );
    assert_eq!(
        plan(&["alter function f(int) set schema s2"], pg_autocommit()).now,
        vec![Target::Namespace(ns("public")), Target::Namespace(ns("s2"))]
    );
}

/// SQLite resolves an unqualified name in `main`, then in the attached schemas.
#[test]
fn sqlite_unqualified_names_reach_attached_schemas() {
    let aux = NsKey::schema(["aux"]);
    let snap = snapshot_of(
        DbmsKind::Sqlite,
        vec![
            ns_sqlite("main").table("a", &["x"]).finish(),
            ns_sqlite("aux").table("t", &["x"]).finish(),
        ],
    );
    assert_eq!(
        plan_in(
            &["alter table t add column b int"],
            sqlite(TxnState::AutoCommit),
            Some(&snap)
        )
        .now,
        vec![Target::Object(key_in(&aux, "t"))]
    );
}

/// `lower` keeps only what the catalog holds or is loading: a refresh never loads a namespace
/// nobody opened.
#[test]
fn lower_keeps_only_loaded_namespaces() {
    let snap = snapshot(vec![ns_pg(DB, "public").table("t", &["a"]).finish()]);
    assert_eq!(
        lower(
            &[
                Target::Object(k("public.t")),
                Target::Object(k("nowhere.t")),
                Target::NamespaceNames(ns("nowhere")),
            ],
            &snap
        ),
        vec![Target::Objects {
            ns: ns("public"),
            keys: vec![k("public.t")]
        }]
    );
}

// ---- the run tracker ---------------------------------------------------------------------------

fn unit(n: u32) -> UnitId {
    UnitId {
        run: RunId(1),
        ordinal: n,
    }
}

fn tracked(n: u32, dbms: DbmsKind, sql: &str) -> TrackedUnit {
    let r = reports(dbms, sql);
    TrackedUnit::new(unit(n), Some(&r), dialect(dbms), sql)
}

fn session_at(schema: &str, path: &[&str]) -> SessionSnapshot {
    SessionSnapshot {
        state: Default::default(),
        tx: TxState::default(),
        schema: Some(SchemaCtx {
            database: Some(DB.into()),
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

/// The messages a tracker over `units` sends for `steps`.
fn run_steps(dbms: DbmsKind, units: &[&str], start: TxPhase, steps: &[Step]) -> Vec<RefreshMsg> {
    let (tx, rx) = watch::channel(session_at("public", &["public"]));
    let _keep = tx;
    let inbox = RefreshInbox::with_capacity(64);
    let tracked: Vec<TrackedUnit> = units
        .iter()
        .enumerate()
        .map(|(i, sql)| tracked(i as u32, dbms, sql))
        .collect();
    let mut t = RunTracker::new(S, dbms, tracked, rx, start, inbox.clone());
    for s in steps {
        match s {
            Step::Start(n) => t.unit_started(unit(*n)),
            Step::Tx(from, to) => t.tx_changed(*from, *to),
            Step::Tag(n, tag) => t.update_count(unit(*n), Some(tag)),
            Step::Ok(n) => t.unit_ended(unit(*n), &UnitEnd::Succeeded { rows: None }),
            Step::Fail(n, kind) => {
                t.unit_ended(unit(*n), &UnitEnd::Failed(ExecError::of(kind.clone())))
            }
        }
    }
    inbox.queued()
}

enum Step {
    Start(u32),
    Tx(TxPhase, TxPhase),
    Tag(u32, &'static str),
    Ok(u32),
    Fail(u32, ExecErrorKind),
}

fn ended(msgs: &[RefreshMsg]) -> Vec<TxOutcome> {
    msgs.iter()
        .filter_map(|m| match m {
            RefreshMsg::TxEnded { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

fn txns(msgs: &[RefreshMsg]) -> Vec<TxnState> {
    msgs.iter()
        .filter_map(|m| match m {
            RefreshMsg::Executed { txn, .. } => Some(*txn),
            _ => None,
        })
        .collect()
}

use Step::{Fail, Ok as Done, Start, Tag, Tx};
use TxPhase::{Failed, Idle, InTx};

/// Review probe: a DDL inside the user's open PostgreSQL transaction waits for the commit; the
/// end comes from the server's transaction status, never the text.
#[test]
fn the_transaction_end_comes_from_the_server_status() {
    let pg = DbmsKind::Postgres;
    // BEGIN; CREATE TABLE; COMMIT.
    let m = run_steps(
        pg,
        &["begin", "create table public.a(x int)", "commit"],
        Idle,
        &[
            Start(0),
            Tx(Idle, InTx),
            Done(0),
            Start(1),
            Done(1),
            Start(2),
            Tx(InTx, Idle),
            Done(2),
        ],
    );
    assert_eq!(txns(&m), vec![TxnState::InTransaction]);
    assert_eq!(ended(&m), vec![TxOutcome::Committed]);
    // COMMIT of a failed transaction rolls back (PostgreSQL answers `ROLLBACK`).
    let m = run_steps(
        pg,
        &["commit"],
        Failed,
        &[Start(0), Tx(Failed, Idle), Done(0)],
    );
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    let m = run_steps(
        pg,
        &["commit"],
        InTx,
        &[Start(0), Tag(0, "ROLLBACK"), Tx(InTx, Idle), Done(0)],
    );
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    let m = run_steps(
        pg,
        &["rollback"],
        InTx,
        &[Start(0), Tx(InTx, Idle), Done(0)],
    );
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    // The connection went after COMMIT was sent: maybe committed.
    let m = run_steps(
        pg,
        &["commit"],
        InTx,
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
    assert_eq!(ended(&m), vec![TxOutcome::Unknown]);
    // The connection went during a statement inside the transaction: the server rolled back.
    let m = run_steps(
        pg,
        &["create table public.b(x int)"],
        InTx,
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
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    assert!(txns(&m).is_empty());
    // The user chose "roll back and continue" between units.
    let m = run_steps(pg, &["select 1"], Failed, &[Tx(Failed, Idle)]);
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    // Savepoints: the names travel, folded.
    let m = run_steps(
        pg,
        &[
            "savepoint S1",
            "create table public.c(x int)",
            "rollback to savepoint s1",
        ],
        InTx,
        &[Start(0), Done(0), Start(1), Done(1), Start(2), Done(2)],
    );
    assert_eq!(
        m.iter()
            .filter(|m| !matches!(m, RefreshMsg::Executed { .. }))
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            RefreshMsg::Savepoint {
                session: S,
                name: Arc::from("s1")
            },
            RefreshMsg::TxEnded {
                session: S,
                outcome: TxOutcome::RolledBackTo(Arc::from("s1"))
            },
        ]
    );
    // A savepoint outside a transaction means nothing.
    let m = run_steps(pg, &["savepoint s"], Idle, &[Start(0), Done(0)]);
    assert!(m.is_empty(), "{m:?}");
    // MySQL: DDL inside a transaction commits it (implicit commit).
    let m = run_steps(
        DbmsKind::MySql,
        &["create table m(a int)"],
        InTx,
        &[Start(0), Tx(InTx, Idle), Done(0)],
    );
    assert_eq!(txns(&m), vec![TxnState::AutoCommit]);
    assert_eq!(ended(&m), vec![TxOutcome::Committed]);
}

/// Review probe: the namespace and search path are read when the unit ends (execution time),
/// never later when the coordinator plans.
#[test]
fn the_state_is_read_when_the_unit_ends_not_at_refresh_time() {
    let (tx, rx) = watch::channel(session_at("s2", &["s2", "public"]));
    let inbox = RefreshInbox::with_capacity(16);
    let mut t = RunTracker::new(
        S,
        DbmsKind::Postgres,
        vec![tracked(0, DbmsKind::Postgres, "create table t(a int)")],
        rx,
        Idle,
        inbox.clone(),
    );
    t.unit_started(unit(0));
    t.unit_ended(unit(0), &UnitEnd::Succeeded { rows: None });
    // A later `SET search_path` moves the session.
    tx.send_replace(session_at("public", &["public"]));
    let q = inbox.queued();
    let RefreshMsg::Executed {
        stmts, state, txn, ..
    } = &q[0]
    else {
        panic!("{q:?}");
    };
    assert_eq!(state.current, ns("s2"));
    assert_eq!(
        plan_refresh(stmts, state, *txn, &IntroCaps::planned(DbmsKind::Postgres)).now,
        vec![Target::Object(k("s2.t")), Target::NamespaceNames(ns("s2"))]
    );
}

#[test]
fn tx_control_reads_savepoint_statements() {
    let pg = DialectId::Postgres;
    let my = DialectId::MySql;
    assert_eq!(
        tx_control(pg, "SAVEPOINT a"),
        Some(TxControl::Savepoint(Arc::from("a")))
    );
    assert_eq!(
        tx_control(pg, "savepoint \"A b\";"),
        Some(TxControl::Savepoint(Arc::from("A b")))
    );
    assert_eq!(
        tx_control(pg, "release savepoint X"),
        Some(TxControl::Release(Arc::from("x")))
    );
    assert_eq!(
        tx_control(pg, "RELEASE savepoint"),
        Some(TxControl::Release(Arc::from("savepoint")))
    );
    assert_eq!(
        tx_control(pg, "rollback /* c */ to -- d\n savepoint s"),
        Some(TxControl::RollbackTo(Arc::from("s")))
    );
    assert_eq!(
        tx_control(my, "ROLLBACK WORK TO `Sp`"),
        Some(TxControl::RollbackTo(Arc::from("Sp")))
    );
    assert_eq!(
        tx_control(pg, "prepare transaction 'x'"),
        Some(TxControl::PrepareTransaction)
    );
    for not in [
        "rollback",
        "commit",
        "select 1",
        "prepare p as select 1",
        "rollback to a.b",
    ] {
        assert_eq!(tx_control(pg, not), None, "{not}");
    }
}

// ---- the inbox and the coordinator ------------------------------------------------------------

/// The sink never waits: a full inbox drops its oldest message and counts it.
#[test]
fn a_full_inbox_drops_the_oldest_and_counts_it() {
    let inbox = RefreshInbox::with_capacity(2);
    for i in 0..5u64 {
        inbox.push(RefreshMsg::TxEnded {
            session: SessionId(i),
            outcome: TxOutcome::Committed,
        });
    }
    assert_eq!(inbox.len(), 2);
    assert_eq!(inbox.dropped(), 3);
    assert_eq!(
        inbox
            .queued()
            .iter()
            .map(|m| match m {
                RefreshMsg::TxEnded { session, .. } => session.0,
                _ => u64::MAX,
            })
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
}

/// A dropped message is never a silent loss: its session's database is refreshed now and again
/// at the next commit.
#[test]
fn a_dropped_message_refreshes_its_session_widely() {
    // A runtime nobody drives: the coordinator's task and its refreshes never run; the test
    // drains by hand.
    let idle_rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    ))));
    let c = RefreshCoordinator::new(
        IntroHandle::detached(catalog),
        DbmsKind::Postgres,
        RefreshAfterDdl::Always,
        idle_rt.handle().clone(),
    );
    let mut events = c.events();
    let inbox = c.inbox();
    inbox.push(RefreshMsg::Executed {
        session: S,
        stmts: stmts(DbmsKind::Postgres, "create table public.a(x int)"),
        state: at("public", &["public"]),
        txn: TxnState::InTransaction,
    });
    for _ in 0..hacivat_introspect::refresh_coord::INBOX_CAPACITY {
        inbox.push(RefreshMsg::Savepoint {
            session: SessionId(8),
            name: Arc::from("s"),
        });
    }
    assert_eq!(inbox.dropped(), 1);
    c.drain();
    assert_eq!(
        c.pending(S),
        vec![Target::Database(Name::from(DB)), Target::ScopeRefresh]
    );
    assert_eq!(events.try_recv().unwrap(), CoordEvent::Dropped { total: 1 });
}

/// `ask` marks stale and asks; a yes refreshes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_preference_asks_then_refreshes_on_yes() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"]).with_oid(501)]).await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Ask);
    let mut events = c.events();
    bounded(c.submit(
        S,
        RefreshPlan {
            now: vec![Target::Object(kid("public.t", 501))],
            pending: Vec::new(),
        },
    ))
    .await
    .unwrap();
    let mut asked = None;
    for _ in 0..2 {
        if let CoordEvent::Ask { id, count } = bounded(events.recv()).await.unwrap() {
            assert_eq!(count, 1);
            asked = Some(id);
        }
    }
    let id = asked.expect("asked");
    assert!(fragment_keys(&r).is_empty());
    bounded(c.answer(id, true)).await.unwrap();
    assert!(fragment_keys(&r).contains(&kid("public.t", 501)));
}

/// A fake SQLite server with `main` loaded.
async fn sqlite_rig() -> Rig {
    let fake = FakeIntrospector::new(DbmsKind::Sqlite);
    fake.put_ns(main_ns(), vec![FakeObject::table("t", &["a"])]);
    let r = rig_with(
        fake,
        RigOpts {
            dbms: DbmsKind::Sqlite,
            scope: "*",
            ..RigOpts::default()
        },
    );
    bounded(
        r.handle
            .refresh(Target::Namespace(main_ns()), Trigger::User),
    )
    .await
    .unwrap();
    idle(&r.handle).await;
    r
}

// ---- fix round 1 ------------------------------------------------------------------------------

/// I1: `ROLLBACK AND CHAIN` ends the transaction (rolled back) though the status stays in one;
/// `ROLLBACK TO` (the status stays too) is not misread as an end.
#[test]
fn chained_transaction_ends_are_read_from_the_statement_kind() {
    let pg = DbmsKind::Postgres;
    let m = run_steps(pg, &["rollback and chain"], InTx, &[Start(0), Done(0)]);
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    let m = run_steps(pg, &["commit and chain"], InTx, &[Start(0), Done(0)]);
    assert_eq!(ended(&m), vec![TxOutcome::Committed]);
    let m = run_steps(
        pg,
        &["commit and chain"],
        Failed,
        &[Start(0), Tx(Failed, InTx), Done(0)],
    );
    assert_eq!(ended(&m), vec![TxOutcome::RolledBack]);
    let m = run_steps(pg, &["rollback to savepoint s"], InTx, &[Start(0), Done(0)]);
    assert_eq!(ended(&m), vec![TxOutcome::RolledBackTo(Arc::from("s"))]);
    // A COMMIT outside any transaction ends nothing.
    let m = run_steps(pg, &["commit"], Idle, &[Start(0), Done(0)]);
    assert!(ended(&m).is_empty(), "{m:?}");
}

/// I2: a MySQL batch cancelled partway may have committed its DDL; a PostgreSQL batch failing
/// partway rolled back (one implicit transaction) and plans nothing.
#[test]
fn a_cancelled_mysql_batch_plans_its_ddl_and_a_failed_pg_batch_does_not() {
    let (tx, rx) = watch::channel(session_at("public", &["public"]));
    let _keep = tx;
    let inbox = RefreshInbox::with_capacity(16);
    let mut t = RunTracker::new(
        S,
        DbmsKind::MySql,
        vec![tracked(
            0,
            DbmsKind::MySql,
            "create table a(x int); select sleep(100)",
        )],
        rx.clone(),
        Idle,
        inbox.clone(),
    );
    t.unit_started(unit(0));
    t.unit_ended(unit(0), &UnitEnd::Cancelled);
    assert_eq!(txns(&inbox.queued()), vec![TxnState::AutoCommit]);
    let inbox = RefreshInbox::with_capacity(16);
    let mut t = RunTracker::new(
        S,
        DbmsKind::Postgres,
        vec![tracked(
            0,
            DbmsKind::Postgres,
            "create table public.a(x int); select * from nope",
        )],
        rx,
        Idle,
        inbox.clone(),
    );
    t.unit_started(unit(0));
    t.unit_ended(unit(0), &UnitEnd::Failed(ExecError::of(ExecErrorKind::Db)));
    assert!(inbox.is_empty(), "{:?}", inbox.queued());
}

/// m4: `COMMIT PREPARED` commits a transaction of unknown content: the whole database.
#[test]
fn commit_prepared_refreshes_the_whole_database() {
    let m = run_steps(
        DbmsKind::Postgres,
        &["commit prepared 'x'"],
        Idle,
        &[Start(0), Done(0)],
    );
    let RefreshMsg::Executed {
        stmts, state, txn, ..
    } = &m[0]
    else {
        panic!("{m:?}");
    };
    assert_eq!(
        plan_refresh(stmts, state, *txn, &IntroCaps::planned(DbmsKind::Postgres)).now,
        vec![Target::Database(Name::from(DB)), Target::ScopeRefresh]
    );
}

/// m3: a dropped message of a session outside a transaction leaves nothing pending.
#[test]
fn a_dropped_autocommit_message_leaves_nothing_pending() {
    let idle_rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        IdentCtx::new(DialectId::Postgres),
    ))));
    let c = RefreshCoordinator::new(
        IntroHandle::detached(catalog),
        DbmsKind::Postgres,
        RefreshAfterDdl::Always,
        idle_rt.handle().clone(),
    );
    let inbox = c.inbox();
    inbox.push(RefreshMsg::Executed {
        session: S,
        stmts: stmts(DbmsKind::Postgres, "create table public.a(x int)"),
        state: at("public", &["public"]),
        txn: TxnState::AutoCommit,
    });
    for _ in 0..hacivat_introspect::refresh_coord::INBOX_CAPACITY {
        inbox.push(RefreshMsg::Savepoint {
            session: SessionId(8),
            name: Arc::from("s"),
        });
    }
    c.drain();
    assert!(c.pending(S).is_empty());
    // The wide refresh was handed over at once.
    assert_eq!(c.applied(), 1);
}

/// m2: an unanswered question is folded into the next one; answering the old id does nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unanswered_question_is_folded_into_the_next() {
    let r = loaded_rig(vec![
        FakeObject::table("t1", &["a"]).with_oid(501),
        FakeObject::table("t2", &["a"]).with_oid(502),
    ])
    .await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Ask);
    let mut events = c.events();
    let mut asks = Vec::new();
    for key in [kid("public.t1", 501), kid("public.t2", 502)] {
        bounded(c.submit(
            S,
            RefreshPlan {
                now: vec![Target::Object(key)],
                pending: Vec::new(),
            },
        ))
        .await
        .unwrap();
        loop {
            if let CoordEvent::Ask { id, count } = bounded(events.recv()).await.unwrap() {
                asks.push((id, count));
                break;
            }
        }
    }
    assert_eq!(asks[1].1, 2, "{asks:?}");
    bounded(c.answer(asks[0].0, true)).await.unwrap();
    assert_eq!(c.applied(), 0);
    bounded(c.answer(asks[1].0, true)).await.unwrap();
    let keys = fragment_keys(&r);
    assert!(keys.contains(&kid("public.t1", 501)) && keys.contains(&kid("public.t2", 502)));
}

/// m5: a namespace marked stale while a pass of it is loading is published stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_namespace_loading_when_marked_is_published_stale() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"])]).await;
    let public = ns("public");
    let gate = r
        .fake
        .gate(move |c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == public));
    let pass = r
        .handle
        .refresh(Target::Namespace(ns("public")), Trigger::User);
    bounded(gate.entered()).await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Never);
    let mut events = c.events();
    bounded(c.submit(S, plan(&["create table public.n(a int)"], pg_autocommit())))
        .await
        .unwrap();
    assert!(matches!(
        bounded(events.recv()).await.unwrap(),
        CoordEvent::Stale { .. }
    ));
    gate.release();
    bounded(pass).await.unwrap();
    assert!(matches!(
        ns_state(&r, &ns("public")),
        LoadState::Loaded { stale: true, .. }
    ));
}

/// I4: `never` with a scope change marks the loaded schema lists stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_marks_the_schema_list_stale_on_a_scope_change() {
    let r = loaded_rig(vec![FakeObject::table("t", &["a"])]).await;
    bounded(r.handle.refresh(Target::ScopeRefresh, Trigger::User))
        .await
        .unwrap();
    let list = ContainerKey::schema_list(Some(&Name::from(DB)));
    assert!(matches!(
        r.catalog.current().state(&list),
        LoadState::Loaded { stale: false, .. }
    ));
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Never);
    bounded(c.submit(S, plan(&["create schema s9"], pg_autocommit())))
        .await
        .unwrap();
    wait_until(Duration::from_secs(30), || {
        matches!(
            r.catalog.current().state(&list),
            LoadState::Loaded { stale: true, .. }
        )
    })
    .await;
}

// ## m4-t33b-carries (named exception; the T27 carry "Explorer refreshed N objects", mock 9):
// a refresh after DDL that re-read objects announces how many, per namespace, once it is done.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_object_refresh_announces_its_count() {
    let r = loaded_rig(vec![
        FakeObject::table("t", &["a"]).with_oid(501),
        FakeObject::table("u", &["a"]).with_oid(502),
    ])
    .await;
    let c = coordinator(&r.handle, DbmsKind::Postgres, RefreshAfterDdl::Always);
    let mut events = c.events();
    let p = plan_in(
        &[
            "alter table public.t add column z int",
            "alter table public.u add column z int",
        ],
        pg_autocommit(),
        Some(&r.catalog.current()),
    );
    bounded(c.submit(S, p)).await.unwrap();
    let mut refreshed = None;
    while let Ok(ev) = events.try_recv() {
        if let CoordEvent::Refreshed { objects } = ev {
            refreshed = Some(objects);
        }
    }
    assert_eq!(refreshed, Some(vec![(ns("public"), 2)]));
}
