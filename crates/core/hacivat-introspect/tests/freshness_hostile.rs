// ## m4-t22-freshness (fix round 1: the T22 review's hostile tests, adopted as written; all six
// failed at `685b23f`).
//! Task 22 reviewer's hostile tests (not the implementer's). Each one asserts the behaviour the
//! plan and the carries require; a failure is a finding.

#![cfg(feature = "session")]

mod common;
// ## m4-t22-freshness (fix round 3): the T17 in-memory MySQL reader, for a pure stamp test.
mod mysql_fake;

use std::time::Duration;

use common::*;
use hacivat_introspect::Introspector;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::freshness::{self, NsVerdict};
use hacivat_introspect::mysql::MySqlIntrospector;
use hacivat_introspect::mysql::state::{TopMark, ns_check};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::FakeCall;
use hacivat_meta::merge::Trigger;
use hacivat_meta::name::FoldRule;
use hacivat_meta::namespace::NsMeta;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{
    Level, NamespaceSnapshot, NsKey, Obj, ObjectKind, StateMark, Subtree, Watermark,
};
use hacivat_proto::DbmsKind;

const TEN_MIN: Duration = Duration::from_secs(600);

fn tables_in(r: &Rig, n: &NsKey) -> usize {
    r.catalog.current().namespaces.get(n).map_or(0, |a| {
        a.children(0)
            .filter(|(_, o)| o.kind() == ObjectKind::Table)
            .count()
    })
}

fn passes_of(r: &Rig, n: &NsKey) -> Vec<Mode> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Introspect { ns, mode, .. } if &ns == n => Some(mode),
            _ => None,
        })
        .collect()
}

/// A MySQL arena `db` with one table `t` whose mark is `mark`, under `watermark`.
fn mysql_arena(watermark: Watermark, mark: StateMark) -> NamespaceSnapshot {
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, name("db")));
    let root = t.root();
    t.add(
        root,
        Obj::new(ObjectKind::Table, name("t")).with_state(mark),
    );
    NamespaceSnapshot::build(
        NsKey::schema(["db"]),
        DbmsKind::MySql,
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark,
            loaded_at_ms: 1,
        },
        t,
    )
    .unwrap()
}

/// R-W3-3 ("a different server means a full re-read and no trust in cached marks") and R-M4-10
/// (MariaDB: no marker in v1, every pass full). After `forget_watermarks` (or on MariaDB, whose
/// passes store `Watermark::None`), the MySQL cached side must not vouch for the arena: PG and
/// SQLite answer `None` without a watermark, MySQL must too.
#[test]
fn h1_mysql_cached_side_needs_a_watermark() {
    let mark = StateMark::MyTable {
        created: 1_700_000_000_000_000,
        trigger_max: 0,
        trigger_count: 0,
    };
    let snap = mysql_arena(Watermark::None, mark);
    let mysql = MySqlIntrospector {
        flavor: DbmsKind::MySql,
    };
    assert_eq!(
        mysql.cached_check(&snap),
        None,
        "a MySQL arena with no watermark (another server, or MariaDB) is vouched for by marks alone"
    );
}

/// R-M4-39: a table marker inside the one-second window of the last pass is re-read by every
/// incremental pass (two DDLs in the same second leave the same CREATE_TIME). The plan's cost
/// row names background validation as the safety net. Validation must therefore never call such
/// a namespace current.
#[test]
fn h2_mysql_validation_honours_the_one_second_window() {
    let w: i64 = 1_700_000_000_400_000; // the last pass, 0.4 s into the second
    let created: i64 = 1_700_000_000_000_000; // CREATE_TIME (seconds) of a DDL 0.1 s earlier
    let mark = StateMark::MyTable {
        created,
        trigger_max: 0,
        trigger_count: 0,
    };
    let snap = mysql_arena(
        Watermark::MyTime {
            at_utc_micros: w,
            server_uuid: name("u"),
            version: name("8.4.0"),
            priv_hash: 7,
        },
        mark,
    );
    let mysql = MySqlIntrospector {
        flavor: DbmsKind::MySql,
    };
    // The server after a second ALTER TABLE t at w + 0.3 s: CREATE_TIME is still `created`.
    let server = ns_check(&[TopMark {
        kind: ObjectKind::Table,
        raw_kind: None,
        name: name("t"),
        mark,
    }]);
    // The incremental pass would re-read it…
    assert!(hacivat_introspect::mysql::pass::reread(
        Some(&mark),
        &mark,
        true,
        w
    ));
    // …so validation must not clear the namespace without reading it.
    let cached = mysql.cached_check(&snap).expect("a watermark");
    assert_ne!(
        freshness::verdict(&cached, &server),
        NsVerdict::Current,
        "a table changed in the window of the last pass is validated Current"
    );
}

/// R-W3-3 through the engine, with a cached side shaped like MySQL's (marks only; the fake's):
/// a connect to a different server must re-read every cached namespace, not validate it
/// `Current` by the old server's marks.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h3_a_different_server_is_reread_not_validated_current() {
    let (cur, other) = (pg_ns("db", "public"), pg_ns("db", "other"));
    let fake = FakeIntrospector::default();
    fake.put_ns(cur.clone(), FakeIntrospector::tables(3));
    fake.put_ns(other.clone(), FakeIntrospector::tables(3));
    fake.set_identity(Some(42));
    let r = rig_with(
        fake,
        RigOpts {
            incremental: true,
            current: pg_current("db", "public"),
            ..RigOpts::default()
        },
    );
    for n in [&cur, &other] {
        bounded(
            r.handle
                .refresh(Target::Namespace(n.clone()), Trigger::User),
        )
        .await
        .unwrap();
    }
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    let before = passes_of(&r, &other).len();
    // Another server behind the profile (same object names and versions: a clone, a
    // restored dump, a replica).
    r.fake.set_identity(Some(43));
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    let after = &passes_of(&r, &other)[before..];
    assert!(
        !after.is_empty(),
        "{other:?} was not read again on a different server: {after:?}; state {:?}",
        r.catalog
            .current()
            .state(&ContainerKey::Namespace(other.clone()))
    );
}

/// D-039 across passes (T10 carry M-1), the current namespace: its on-connect incremental pass
/// is the background refresh of the namespace the user works in. Two reconnects that each drop
/// under half must not remove 55 % of it without asking.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h4_connect_passes_do_not_delete_in_small_steps_around_the_guard() {
    let n = pg_ns("db", "public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(40));
    fake.set_identity(Some(1));
    let r = rig_with(
        fake,
        RigOpts {
            incremental: true,
            current: pg_current("db", "public"),
            ..RigOpts::default()
        },
    );
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    let drop_range = |from: usize, to: usize| {
        r.fake.edit_ns(&n, |s| {
            s.objects.retain(|o| {
                let i: usize = o.name[1..].parse().unwrap();
                !(from..to).contains(&i)
            });
        });
    };
    drop_range(0, 12);
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(tables_in(&r, &n), 28, "12 of 40: under the guard");
    drop_range(12, 22);
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    assert!(
        tables_in(&r, &n) == 28 || r.handle.pending_confirmation(&n).is_some(),
        "22 of 40 removed by two background passes without asking: {} left",
        tables_in(&r, &n)
    );
}

/// D-039: "Keep cached" is the user's decision for the held objects. The next periodic round
/// must not remove them anyway in a step under the per-pass guard.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h5_keep_cached_survives_the_next_validation_round() {
    let n = pg_ns("db", "public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(40));
    let (r, hand) = rig_with_clock(
        fake,
        RigOpts {
            validate_every: Some(TEN_MIN),
            ..RigOpts::default()
        },
    );
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    let drop_range = |from: usize, to: usize| {
        r.fake.edit_ns(&n, |s| {
            s.objects.retain(|o| {
                let i: usize = o.name[1..].parse().unwrap();
                !(from..to).contains(&i)
            });
        });
    };
    let round = async |by: Duration| {
        let before = r.meta.tx_checks.load(std::sync::atomic::Ordering::SeqCst);
        hand.advance(by);
        bounded(wait_until(Duration::from_secs(60), || {
            r.meta.tx_checks.load(std::sync::atomic::Ordering::SeqCst) > before
        }))
        .await;
        idle(&r.handle).await;
    };
    drop_range(0, 12);
    round(TEN_MIN).await;
    drop_range(12, 22);
    round(TEN_MIN).await;
    assert!(r.handle.pending_confirmation(&n).is_some(), "held");
    bounded(r.handle.confirm_pending(n.clone(), false))
        .await
        .unwrap();
    idle(&r.handle).await;
    assert_eq!(tables_in(&r, &n), 28, "kept");
    round(TEN_MIN).await;
    assert!(
        tables_in(&r, &n) == 28 || r.handle.pending_confirmation(&n).is_some(),
        "the kept objects were removed by the next round without asking: {} left",
        tables_in(&r, &n)
    );
}

/// Validation of every cached namespace on connect includes the server namespace (accounts,
/// roles: where a renamed account shows, T16b carry). On MySQL its cached side vouches
/// (`Some`), but the server side refuses the namespace kind: the validation fails, the error is
/// only resolved, and the accounts are never read again in the background.
#[tokio::test]
async fn h6_mysql_server_namespace_validation_does_not_fail() {
    let key = NsKey::server();
    let mut t = Subtree::new(Obj::new(key.kind.root_kind(), key.root_name()));
    let root = t.root();
    t.add(root, Obj::new(ObjectKind::User, name("app@%")));
    let snap = NamespaceSnapshot::build(
        NsKey::server(),
        DbmsKind::MySql,
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark: Watermark::None,
            loaded_at_ms: 1,
        },
        t,
    );
    let snap = snap.unwrap();
    let mysql = MySqlIntrospector {
        flavor: DbmsKind::MySql,
    };
    let mut c = hacivat_session::meta::MetaConn::offline(DbmsKind::MySql);
    let got = freshness::check_namespace(&mysql, &mut c, &NsKey::server(), &snap).await;
    assert!(
        matches!(got, Ok(NsVerdict::Changed { .. })),
        "the MySQL server namespace cannot be validated: {got:?}"
    );
}

// ## m4-t22-freshness (fix round 2: the RR-1 probe, adopted; ruling R-W4-9)

fn table_names(r: &Rig, n: &NsKey) -> Vec<String> {
    let s = r.catalog.current();
    let mut v: Vec<String> = s.namespaces.get(n).map_or(Vec::new(), |a| {
        a.children(0)
            .filter(|(_, o)| o.kind() == ObjectKind::Table)
            .map(|(_, o)| o.name().to_string())
            .collect()
    });
    v.sort();
    v
}

/// R-W4-9: after "Keep cached", background reads still bring other changes (a new table), the
/// kept objects stay, and the user is not asked again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn h7_keep_cached_does_not_freeze_the_namespace() {
    use std::sync::atomic::Ordering::SeqCst;
    let n = pg_ns("db", "public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(40));
    let (r, hand) = rig_with_clock(
        fake,
        RigOpts {
            validate_every: Some(TEN_MIN),
            ..RigOpts::default()
        },
    );
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    let round = async |by: Duration| {
        let before = r.meta.tx_checks.load(SeqCst);
        hand.advance(by);
        bounded(wait_until(Duration::from_secs(60), || {
            r.meta.tx_checks.load(SeqCst) > before
        }))
        .await;
        idle(&r.handle).await;
    };
    let mut events = r.handle.events();
    r.fake.edit_ns(&n, |s| {
        s.objects
            .retain(|o| o.name[1..].parse::<usize>().unwrap() >= 22)
    });
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    assert!(r.handle.pending_confirmation(&n).is_some(), "22/40 held");
    bounded(r.handle.confirm_pending(n.clone(), false))
        .await
        .unwrap();
    idle(&r.handle).await;
    assert_eq!(table_names(&r, &n).len(), 40, "kept");
    // A new table appears on the server; the kept ones stay gone.
    r.fake.edit_ns(&n, |s| {
        let mut extra = FakeIntrospector::tables(41).pop().unwrap();
        extra.name = "t_new".into();
        extra.version += 1000;
        s.objects.push(extra);
    });
    let mut asks = 0;
    for _ in 0..3 {
        round(TEN_MIN).await;
        while let Ok(e) = events.try_recv() {
            if format!("{e:?}").contains("Pending") {
                asks += 1;
            }
        }
    }
    let names = table_names(&r, &n);
    assert!(
        names.contains(&"t_new".to_string()),
        "a kept set freezes every background update of the namespace"
    );
    assert_eq!(names.len(), 41, "the 22 kept tables stay: {names:?}");
    assert_eq!(r.handle.pending_confirmation(&n), None, "not asked again");
    assert_eq!(asks, 0);
    // A drop outside the kept set is a background drop like any other (counted, applied).
    r.fake
        .edit_ns(&n, |s| s.objects.retain(|o| &*o.name != "t39"));
    round(TEN_MIN).await;
    let names = table_names(&r, &n);
    assert!(!names.contains(&"t39".to_string()), "{names:?}");
    assert_eq!(names.len(), 40);
    // The user's refresh ends the keep: the kept tables go (held by the per-pass guard: 22 of
    // 40, so the user answers "Remove from cache").
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    if r.handle.pending_confirmation(&n).is_some() {
        bounded(r.handle.confirm_pending(n.clone(), true))
            .await
            .unwrap();
    }
    assert_eq!(table_names(&r, &n).len(), 18);
}

/// Fix round 2 minor: MySQL before 8.0 has no R-M4-39 marker path, so its arena is never
/// vouched for, whatever its watermark says.
#[test]
fn mysql_before_8_0_is_never_vouched_for() {
    let mark = StateMark::MyTable {
        created: 1_000_000,
        trigger_max: 0,
        trigger_count: 0,
    };
    let mysql = MySqlIntrospector {
        flavor: DbmsKind::MySql,
    };
    let at = |version: &str| {
        mysql_arena(
            Watermark::MyTime {
                at_utc_micros: 9_000_000_000,
                server_uuid: name("u"),
                version: name(version),
                priv_hash: 1,
            },
            mark,
        )
    };
    assert_eq!(mysql.cached_check(&at("5.7.44-log")), None);
    assert_eq!(mysql.cached_check(&at("")), None);
    assert!(mysql.cached_check(&at("8.0.40")).is_some());
    assert!(mysql.cached_check(&at("8.4.3")).is_some());
}

// ## m4-t22-freshness (fix round 3: CI 44667350956, the two `Current` failures)

/// A dropped and recreated PostgreSQL table: the key the caller holds names the old oid; the
/// cache already holds the recreated table (new oid) under the same name. The old key is never
/// `Current`: its stamp (by oid) is gone, so it is `Dropped`.
#[test]
fn a_recreated_oid_is_dropped_for_the_old_key() {
    use hacivat_meta::snapshot::{CatalogChange, LoadState};
    use hacivat_meta::{Catalog, ObjectKey, ServerFacts, ServerId};
    let ns = pg_ns("db", "s");
    let mut t = Subtree::new(Obj::new(ObjectKind::Schema, name("s")));
    let root = t.root();
    // The cache after a re-read: `t` is oid 2 now.
    t.add(
        root,
        Obj::new(ObjectKind::Table, name("t")).with_server_id(ServerId::Oid(2)),
    );
    let arena = NamespaceSnapshot::build(
        ns.clone(),
        DbmsKind::Postgres,
        FoldRule::Exact,
        NsMeta {
            level: Level::Structure,
            watermark: Watermark::None,
            loaded_at_ms: 1,
        },
        t,
    )
    .unwrap();
    let catalog = Catalog::new(std::sync::Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        hacivat_meta::name::IdentCtx::new(sql_dialect::DialectId::Postgres),
    )));
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            CatalogChange::Put(std::sync::Arc::new(arena)),
            CatalogChange::SetState(
                ContainerKey::Namespace(ns.clone()),
                LoadState::Loaded {
                    level: Level::Structure,
                    at: std::time::SystemTime::now(),
                    watermark: Watermark::None,
                    stale: false,
                },
            ),
        ])
        .unwrap();
    let old = ObjectKey {
        kind: ObjectKind::Table,
        path: [name("db"), name("s"), name("t")].into_iter().collect(),
        server_id: Some(ServerId::Oid(1)),
        overload_sig: None,
        raw_kind: None,
    };
    let p = freshness::plan(
        DbmsKind::Postgres,
        &catalog.current(),
        std::slice::from_ref(&old),
    );
    assert_eq!(p.probes.len(), 1);
    let (owner, stored) = &p.probes[0];
    assert_eq!(
        owner.server_id,
        Some(ServerId::Oid(1)),
        "stamped by its own oid"
    );
    // The old oid has no row any more: Dropped, never Current.
    assert_eq!(
        freshness::compare(DbmsKind::Postgres, *stored, None),
        hacivat_introspect::Staleness::Dropped
    );
    // An oid that still exists but the cache lacks: Changed.
    assert_eq!(
        freshness::compare(DbmsKind::Postgres, *stored, Some(StateMark::Hash(7))),
        hacivat_introspect::Staleness::Changed
    );
}

/// ver/01 `#mysql-is-times`: `CREATE TRIGGER` leaves `CREATE_TIME` alone; the table's stamp
/// counts its `INFORMATION_SCHEMA.TRIGGERS` rows (number and newest `CREATED`), so a new
/// trigger, also one created in the same instant as the newest, and a dropped one move it.
#[tokio::test]
async fn a_trigger_moves_the_mysql_table_stamp() {
    use hacivat_introspect::mysql::{pass::read_server, state};
    let mut f = mysql_fake::Fake::mysql();
    let srv = read_server(&mut f, DbmsKind::MySql).await.unwrap();
    let key = hacivat_meta::ObjectKey {
        kind: ObjectKind::Table,
        path: [name("shop"), name("orders")].into_iter().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let stamp = async |f: &mut mysql_fake::Fake| {
        state::stamps(f, &srv, std::slice::from_ref(&key))
            .await
            .unwrap()[0]
            .expect("the table exists")
    };
    let s0 = stamp(&mut f).await;
    let created = f.triggers[0].2.clone();
    f.triggers
        .push(("trg_two".into(), "orders".into(), created));
    let s1 = stamp(&mut f).await;
    assert_ne!(s0, s1, "a second trigger in the same instant");
    assert_eq!(
        freshness::compare(DbmsKind::MySql, freshness::Stored::Mark(s0), Some(s1)),
        hacivat_introspect::Staleness::Changed
    );
    f.triggers.clear();
    let s2 = stamp(&mut f).await;
    assert_ne!(s1, s2, "every trigger dropped");
    assert_ne!(s0, s2);
}
