// ## m4-t22-freshness (fix round 1: the T22 review's hostile tests, adopted as written; all six
// failed at `685b23f`).
//! Task 22 reviewer's hostile tests (not the implementer's). Each one asserts the behaviour the
//! plan and the carries require; a failure is a finding.

#![cfg(feature = "session")]

mod common;

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
