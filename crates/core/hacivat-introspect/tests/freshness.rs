//! Freshness (M4 Task 22; HV-INTRO-15): state checks compared by equality, background
//! validation of cached namespaces, periodic validation on a clock seam, the negative cache's
//! probe by name, and the carries (identity, split by connection, recorded marks, the
//! mass-deletion guard across validation passes, held names).
//!
//! Every engine test runs on the fakes; the Docker facts are in `docker_freshness.rs`.
//! Periodic validation runs on `ValidationClock::manual()` (no `start_paused`: the crate's
//! tokio has no `test-util`), so nothing here waits on wall-clock time.

#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::time::Duration;

use common::*;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::EngineLimits;
use hacivat_introspect::freshness::{
    self, CheckPlan, NsCheck, NsVerdict, Stored, compare, plan, validate_interval,
};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::scope::CurrentCtx;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_introspect::{IntroError, Introspector, Staleness};
use hacivat_meta::merge::Trigger;
use hacivat_meta::name::{FoldRule, NamePart};
use hacivat_meta::namespace::NsMeta;
use hacivat_meta::snapshot::{CatalogChange, ContainerKey, LoadState};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView};
use hacivat_meta::{
    Catalog, Level, NamespaceSnapshot, NsKey, Obj, ObjectKey, ObjectKind, ServerFacts, StateMark,
    Subtree, Watermark,
};
use hacivat_proto::{DbmsKind, EnvTag};
use hacivat_session::meta::MetaConn;
use hacivat_settings::MetadataSettings;
use sql_dialect::DialectId;

const TEN_MIN: Duration = Duration::from_secs(600);

fn ns(schema: &str) -> NsKey {
    pg_ns("db", schema)
}

fn state(r: &Rig, n: &NsKey) -> LoadState {
    r.catalog
        .current()
        .state(&ContainerKey::Namespace(n.clone()))
        .clone()
}

fn tables_in(r: &Rig, n: &NsKey) -> Vec<String> {
    let snap = r.catalog.current();
    let Some(a) = snap.namespaces.get(n) else {
        return Vec::new();
    };
    let mut v: Vec<String> = a
        .children(0)
        .filter(|(_, o)| o.kind() == ObjectKind::Table)
        .map(|(_, o)| o.name().to_string())
        .collect();
    v.sort();
    v
}

fn stamp_calls(r: &Rig) -> Vec<Vec<ObjectKey>> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::CheckState { keys } => Some(keys),
            _ => None,
        })
        .collect()
}

fn fragments(r: &Rig) -> Vec<Vec<ObjectKey>> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Fragment { keys, .. } => Some(keys),
            _ => None,
        })
        .collect()
}

fn ns_checks(r: &Rig) -> Vec<NsKey> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::CheckNamespace { ns } => Some(ns),
            _ => None,
        })
        .collect()
}

fn passes(r: &Rig) -> Vec<(NsKey, Mode, Level)> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Introspect { ns, mode, level } => Some((ns, mode, level)),
            _ => None,
        })
        .collect()
}

/// Loads `n` with a user refresh (a full pass).
async fn load(r: &Rig, n: &NsKey) {
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
}

/// The keys of every top-level table of `n` in the catalog (with their oids).
fn table_keys(r: &Rig, n: &NsKey) -> Vec<ObjectKey> {
    let snap = r.catalog.current();
    let a = &snap.namespaces[n];
    a.children(0).filter_map(|(i, _)| a.object_key(i)).collect()
}

/// A periodic round has run once the engine asked whether the user is in a transaction
/// (each round asks once) and has gone idle again.
async fn round(r: &Rig, hand: &freshness::ManualClock, by: Duration) {
    let before = r.meta.tx_checks.load(SeqCst);
    hand.advance(by);
    bounded(wait_until(Duration::from_secs(60), || {
        r.meta.tx_checks.load(SeqCst) > before
    }))
    .await;
    idle(&r.handle).await;
}

// ---- the plan's list ---------------------------------------------------------------------------

/// R-M4-3: at most 1,000 keys per round trip, and one kind per call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keys_over_1000_are_chunked() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    let mut objects = FakeIntrospector::tables(2_500);
    objects
        .extend((0..10).map(|i| FakeObject::view(&format!("v{i}"), &["a"]).with_oid(90_000 + i)));
    fake.put_ns(n.clone(), objects);
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let keys = table_keys(&r, &n);
    assert_eq!(keys.len(), 2_510);
    bounded(r.handle.check_state(keys)).await.unwrap();
    let calls = stamp_calls(&r);
    let mut sizes: Vec<usize> = calls.iter().map(Vec::len).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, vec![10, 500, 1_000, 1_000], "{sizes:?}");
    for c in &calls {
        assert!(c.iter().all(|k| k.kind == c[0].kind), "one kind per call");
    }
    // Nothing changed: nothing is re-read.
    assert!(fragments(&r).is_empty());
}

/// A permission error is `Unknown`, never `Dropped`, and re-reads nothing; on MySQL a stamp
/// the server no longer shows is `Unknown` too (a lost privilege hides it).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_loss_is_unknown_never_dropped() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let keys = table_keys(&r, &n);
    // The server dropped every object, and the login lost the privilege to read stamps.
    r.fake.edit_ns(&n, |s| s.objects.clear());
    r.fake.fail(
        |c| matches!(c, FakeCall::CheckState { .. }),
        IntroError::Permission {
            kind: ObjectKind::Table,
        },
        1,
    );
    let snap = r.catalog.current();
    let mut conn = MetaConn::offline(DbmsKind::Postgres);
    let got = freshness::check_state(&r.fake, &mut conn, &snap, &keys)
        .await
        .unwrap();
    assert_eq!(got.staleness, vec![Staleness::Unknown; 3]);
    // Through the engine: nothing is re-read, nothing removed.
    r.fake.fail(
        |c| matches!(c, FakeCall::CheckState { .. }),
        IntroError::Permission {
            kind: ObjectKind::Table,
        },
        1,
    );
    bounded(r.handle.check_state(keys)).await.unwrap();
    assert!(fragments(&r).is_empty());
    assert_eq!(tables_in(&r, &n).len(), 3);

    // MySQL: a missing stamp may be a lost privilege.
    let m = NsKey::schema([name("app")]);
    let fake = FakeIntrospector::new(DbmsKind::MySql);
    fake.put_ns(m.clone(), vec![FakeObject::table("t", &["a"])]);
    let r = rig_with(
        fake,
        RigOpts {
            dbms: DbmsKind::MySql,
            scope: "*",
            ..RigOpts::default()
        },
    );
    load(&r, &m).await;
    let keys = table_keys(&r, &m);
    r.fake.edit_ns(&m, |s| s.objects.clear());
    let snap = r.catalog.current();
    let mut conn = MetaConn::offline(DbmsKind::MySql);
    let got = freshness::check_state(&r.fake, &mut conn, &snap, &keys)
        .await
        .unwrap();
    assert_eq!(got.staleness, vec![Staleness::Unknown]);
}

/// `Changed` and `Dropped` keys are re-read by one fragment (the ticket resolves with the
/// generation that holds it); `Current` and `Unknown` ones are not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_and_dropped_enqueue_a_fragment() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(4));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let keys = table_keys(&r, &n);
    r.fake.touch(&n, "t1");
    r.fake
        .edit_ns(&n, |s| s.objects.retain(|o| &*o.name != "t3"));
    // A key of a namespace the catalog does not hold is Unknown (no mark to compare).
    let elsewhere = table_key(&ns("other"), "x", Some(1));
    let mut asked = keys.clone();
    asked.push(elsewhere);
    let snap = r.catalog.current();
    let mut conn = MetaConn::offline(DbmsKind::Postgres);
    let got = freshness::check_state(&r.fake, &mut conn, &snap, &asked)
        .await
        .unwrap();
    assert_eq!(
        got.staleness,
        vec![
            Staleness::Current,
            Staleness::Changed,
            Staleness::Current,
            Staleness::Dropped,
            Staleness::Unknown
        ]
    );
    let g = bounded(r.handle.check_state(asked)).await.unwrap();
    assert_eq!(r.catalog.current().namespaces[&n].generation(), g);
    let frags = fragments(&r);
    assert_eq!(frags.len(), 1, "{frags:?}");
    let mut names: Vec<&str> = frags[0].iter().map(|k| k.name()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["t1", "t3"]);
    assert_eq!(tables_in(&r, &n), vec!["t0", "t1", "t2"]);
}

/// Periodic validation waits while the user's session holds a transaction on a
/// single-connection profile, and runs once it is over (without waiting for the next period).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deferred_during_user_transaction_on_single_connection() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let (r, hand) = rig_with_clock(
        fake,
        RigOpts {
            validate_every: Some(TEN_MIN),
            ..RigOpts::default()
        },
    );
    load(&r, &n).await;
    r.meta.in_tx.store(true, SeqCst);
    round(&r, &hand, TEN_MIN).await;
    // Still inside the transaction a little later: still nothing.
    round(&r, &hand, EngineLimits::default().busy_retry).await;
    assert!(ns_checks(&r).is_empty(), "{:?}", r.fake.calls());
    r.meta.in_tx.store(false, SeqCst);
    round(&r, &hand, EngineLimits::default().busy_retry).await;
    assert_eq!(ns_checks(&r), vec![n.clone()]);
    // The server did not change: the validation read state only, no pass.
    assert_eq!(passes(&r).len(), 1, "{:?}", passes(&r));
}

/// R-M4-20: `auto` validates every 10 minutes for `local` and `dev` profiles and never for
/// the others; the period runs on the engine's clock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn periodic_default_on_for_local_and_dev_off_otherwise() {
    let m = MetadataSettings::default();
    for tag in EnvTag::ALL {
        let want = matches!(tag, EnvTag::Local | EnvTag::Dev).then_some(TEN_MIN);
        assert_eq!(validate_interval(&m, Some(tag)), want, "{tag:?}");
    }
    assert_eq!(validate_interval(&m, None), None, "untagged");

    for tag in [EnvTag::Local, EnvTag::Production] {
        let n = ns("public");
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), FakeIntrospector::tables(3));
        let (r, hand) = rig_with_clock(
            fake,
            RigOpts {
                validate_every: validate_interval(&m, Some(tag)),
                ..RigOpts::default()
            },
        );
        load(&r, &n).await;
        // A minute before the period nothing can fire (the manual clock does not reach it).
        hand.advance(TEN_MIN - Duration::from_secs(60));
        bounded(r.handle.ready()).await.unwrap();
        idle(&r.handle).await;
        assert!(ns_checks(&r).is_empty(), "{tag:?}");
        if tag == EnvTag::Local {
            round(&r, &hand, Duration::from_secs(60)).await;
            assert_eq!(ns_checks(&r), vec![n.clone()], "{tag:?}");
            // And again one period later.
            round(&r, &hand, TEN_MIN).await;
            assert_eq!(ns_checks(&r).len(), 2, "{tag:?}");
        } else {
            hand.advance(Duration::from_secs(24 * 3_600));
            bounded(r.handle.ready()).await.unwrap();
            idle(&r.handle).await;
            assert!(ns_checks(&r).is_empty(), "{tag:?}");
            assert_eq!(r.meta.tx_checks.load(SeqCst), 0, "no round ever ran");
        }
    }
}

/// HV-MODEL-02 step 6 / Task 9's hook: a name the loaded namespace answered `Missing` for is
/// probed with a state check by name (MySQL stamps by name), and a name that now exists is
/// read by one fragment — no pass of the namespace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_name_probe_uses_check_state_by_name() {
    let m = NsKey::schema([name("app")]);
    let fake = FakeIntrospector::new(DbmsKind::MySql);
    fake.put_ns(m.clone(), vec![FakeObject::table("t", &["a"])]);
    let r = rig_with(
        fake,
        RigOpts {
            dbms: DbmsKind::MySql,
            scope: "*",
            current: CurrentCtx {
                database: Some(name("app")),
                schema: Some(name("app")),
                ident: None,
            },
            // Every miss probes (the engine's own per-namespace limit is review m7's).
            limits: EngineLimits {
                probe_interval: Duration::ZERO,
                ..EngineLimits::default()
            },
            ..RigOpts::default()
        },
    );
    bounded(r.handle.refresh(Target::ScopeRefresh, Trigger::User))
        .await
        .unwrap();
    load(&r, &m).await;
    idle(&r.handle).await;
    let before = passes(&r).len();
    // Another session creates `fresh`.
    r.fake
        .edit_ns(&m, |s| s.objects.push(FakeObject::table("fresh", &["id"])));
    let part = |s: &str| NamePart {
        text: name(s),
        quoted: false,
    };
    let view = SnapshotView::new(r.catalog.current()).with_hooks(Arc::new(r.handle.lookup_hooks()));
    assert!(matches!(
        view.lookup(
            &[part("app"), part("fresh")],
            hacivat_meta::KindGroup::Relations
        ),
        Lookup::Missing(_)
    ));
    bounded(wait_until(Duration::from_secs(60), || {
        tables_in(&r, &m).contains(&"fresh".to_string())
    }))
    .await;
    idle(&r.handle).await;
    let probes = stamp_calls(&r);
    assert!(
        probes
            .iter()
            .flatten()
            .any(|k| k.name() == "fresh" && k.kind == ObjectKind::Table && k.server_id.is_none()),
        "{probes:?}"
    );
    assert_eq!(passes(&r).len(), before, "a fragment, not a pass");
    let frags = fragments(&r);
    assert_eq!(frags.len(), 1, "{frags:?}");
    assert_eq!(frags[0].len(), 1);
    assert_eq!(frags[0][0].name(), "fresh");

    // A name still missing reads its stamps and nothing more.
    let view = SnapshotView::new(r.catalog.current()).with_hooks(Arc::new(r.handle.lookup_hooks()));
    assert!(matches!(
        view.lookup(
            &[part("app"), part("nope")],
            hacivat_meta::KindGroup::Relations
        ),
        Lookup::Missing(_)
    ));
    bounded(wait_until(Duration::from_secs(60), || {
        stamp_calls(&r).iter().flatten().any(|k| k.name() == "nope")
    }))
    .await;
    idle(&r.handle).await;
    assert_eq!(fragments(&r).len(), 1);
    assert_eq!(passes(&r).len(), before);
}

/// The stamps `check_state` compares are exactly the marks the snapshot stores for each stamp
/// owner: an unchanged server is `Current` everywhere and nothing is re-read; a child key is
/// answered by its top-level owner's stamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stamps_equal_check_state_inputs() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(5));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let snap = r.catalog.current();
    let arena = &snap.namespaces[&n];
    let tables = table_keys(&r, &n);
    // A column key too: its owner is its table.
    let col_idx = arena
        .children(arena.find(&tables[2]).unwrap())
        .next()
        .map(|(i, _)| i)
        .unwrap();
    let column = arena.object_key(col_idx).unwrap();
    let mut asked = tables.clone();
    asked.push(column);
    let p: CheckPlan = plan(DbmsKind::Postgres, &snap, &asked);
    assert_eq!(p.probes.len(), 5, "the column shares its table's probe");
    let mut conn = MetaConn::offline(DbmsKind::Postgres);
    let owners: Vec<ObjectKey> = p.probes.iter().map(|(k, _)| k.clone()).collect();
    let stamps = r.fake.stamps(&mut conn, &owners).await.unwrap();
    for ((owner, stored), stamp) in p.probes.iter().zip(&stamps) {
        let held = arena.obj(arena.find(owner).unwrap()).unwrap().state();
        assert_eq!(*stored, Stored::Mark(held));
        assert_eq!(
            Some(held),
            *stamp,
            "the stamp is the stored mark: {owner:?}"
        );
    }
    let got = freshness::run(&r.fake, &mut conn, &p).await.unwrap();
    assert_eq!(got.staleness, vec![Staleness::Current; 6]);
    assert!(got.marks.is_empty());
    bounded(r.handle.check_state(asked)).await.unwrap();
    assert!(fragments(&r).is_empty());
}

/// SQLite state is per schema: one DDL anywhere in a schema moves every object's mark, so every
/// key of it reads `Changed`; `temp` keys are never stamped (per connection) and read
/// `Changed`, so the engine re-reads them on the user's connection.
#[test]
fn sqlite_state_is_schema_level() {
    let main = NsKey::schema([name("main")]);
    let before = StateMark::Hash(0xA);
    let after = StateMark::Hash(0xB);
    let mut tree = Subtree::new(Obj::new(ObjectKind::Schema, name("main")).with_state(before));
    let root = tree.root();
    for t in ["t", "u"] {
        let at = tree.add(
            root,
            Obj::new(ObjectKind::Table, name(t)).with_state(before),
        );
        tree.add(
            at,
            Obj::new(ObjectKind::Column, name("a")).with_state(before),
        );
    }
    let arena = NamespaceSnapshot::build(
        main.clone(),
        DbmsKind::Sqlite,
        FoldRule::AsciiCaseless,
        NsMeta {
            level: Level::Structure,
            watermark: Watermark::None,
            loaded_at_ms: 1,
        },
        tree,
    )
    .unwrap();
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Sqlite,
        hacivat_meta::name::IdentCtx::new(DialectId::Sqlite),
    )));
    let publisher = catalog.publisher().unwrap();
    publisher
        .publish(vec![
            CatalogChange::Put(Arc::new(arena)),
            CatalogChange::SetState(
                ContainerKey::Namespace(main.clone()),
                LoadState::Loaded {
                    level: Level::Structure,
                    at: std::time::SystemTime::now(),
                    watermark: Watermark::None,
                    stale: false,
                },
            ),
        ])
        .unwrap();
    let snap = catalog.current();
    let a = &snap.namespaces[&main];
    let key = |path: &[&str], kind| {
        let mut k = ObjectKey {
            kind,
            path: path.iter().map(|p| name(p)).collect(),
            server_id: None,
            overload_sig: None,
            raw_kind: None,
        };
        k = a.object_key(a.find(&k).unwrap()).unwrap();
        k
    };
    let t = key(&["main", "t"], ObjectKind::Table);
    let u = key(&["main", "u"], ObjectKind::Table);
    let ta = key(&["main", "t", "a"], ObjectKind::Column);
    let temp = ObjectKey {
        kind: ObjectKind::Table,
        path: [name("temp"), name("x")].into_iter().collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let p = plan(DbmsKind::Sqlite, &snap, &[t.clone(), u.clone(), ta, temp]);
    // Two owners (the column is its table's); `temp` is decided without a stamp.
    assert_eq!(
        p.probes,
        vec![(t, Stored::Mark(before)), (u, Stored::Mark(before))]
    );
    // DDL on `u` alone: the schema's mark moved, so `t` (and its column) read Changed too.
    for (_, stored) in &p.probes {
        assert_eq!(
            compare(DbmsKind::Sqlite, *stored, Some(after)),
            Staleness::Changed
        );
        assert_eq!(
            compare(DbmsKind::Sqlite, *stored, Some(before)),
            Staleness::Current
        );
    }
}

// ---- carries -------------------------------------------------------------------------------------

/// T15 carry (R-W3-3) / T16 review: the server identity is checked on connect; a login that
/// cannot read it (`Unknown`) costs one full re-read per session, not one per connect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_identity_rereads_in_full_once_per_session() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    fake.set_identity(None);
    let r = rig_with(
        fake,
        RigOpts {
            incremental: true,
            current: pg_current("db", "public"),
            ..RigOpts::default()
        },
    );
    load(&r, &n).await;
    // Connect 1: the identity is unknown, the watermark is forgotten: a full pass.
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(passes(&r).last().unwrap().1, Mode::Full, "{:?}", passes(&r));
    // Connect 2 in the same session: still unknown, but the watermark now comes from this
    // session's own pass: incremental.
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(
        passes(&r).last().unwrap().1,
        Mode::Incremental,
        "{:?}",
        passes(&r)
    );
    // A known identity that differs from the last one is a different server: full again.
    r.fake.set_identity(Some(42));
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    r.fake.set_identity(Some(43));
    bounded(r.handle.on_connect()).await.unwrap();
    idle(&r.handle).await;
    assert_eq!(passes(&r).last().unwrap().1, Mode::Full, "{:?}", passes(&r));
}

/// On connect every cached in-scope namespace other than the current one is validated in the
/// background: an unchanged one is cleared of its stale mark with state queries only, a
/// changed one is read again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_connect_validates_cached_namespaces() {
    let (cur, same, moved) = (ns("public"), ns("same"), ns("moved"));
    let fake = FakeIntrospector::default();
    for n in [&cur, &same, &moved] {
        fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    }
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current("db", "public"),
            ..RigOpts::default()
        },
    );
    for n in [&cur, &same, &moved] {
        load(&r, n).await;
    }
    r.handle
        .mark_stale(vec![cur.clone(), same.clone(), moved.clone()], false);
    bounded(r.handle.ready()).await.unwrap();
    r.fake.touch(&moved, "t2");
    let before = passes(&r).len();
    bounded(r.handle.on_connect()).await.unwrap();
    bounded(wait_until(Duration::from_secs(60), || {
        !matches!(state(&r, &moved), LoadState::Loaded { stale: true, .. })
            && !matches!(state(&r, &same), LoadState::Loaded { stale: true, .. })
    }))
    .await;
    idle(&r.handle).await;
    let mut checked = ns_checks(&r);
    checked.sort();
    let mut want = vec![same.clone(), moved.clone()];
    want.sort();
    assert_eq!(checked, want, "the current namespace has its own step");
    let after: Vec<NsKey> = passes(&r)[before..].iter().map(|p| p.0.clone()).collect();
    assert!(!after.contains(&same), "unchanged: state queries only");
    assert!(after.contains(&moved), "changed: read again");
    assert!(matches!(
        state(&r, &same),
        LoadState::Loaded { stale: false, .. }
    ));
}

/// The carry "split `CheckState` by connection": keys of two PostgreSQL databases are stamped
/// on a connection to each database (oids are per database), never on one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_state_runs_on_each_keys_own_database() {
    let (a, b) = (pg_ns("one", "public"), pg_ns("two", "public"));
    let fake = FakeIntrospector::default();
    fake.put_ns(a.clone(), FakeIntrospector::tables(2));
    fake.put_ns(b.clone(), FakeIntrospector::tables(2));
    let r = rig_with(
        fake,
        RigOpts {
            scope: "*:*",
            ..RigOpts::default()
        },
    );
    load(&r, &a).await;
    load(&r, &b).await;
    let mut keys = table_keys(&r, &a);
    keys.extend(table_keys(&r, &b));
    r.meta.databases.lock().unwrap().clear();
    let calls_before = stamp_calls(&r).len();
    bounded(r.handle.check_state(keys)).await.unwrap();
    let calls = &stamp_calls(&r)[calls_before..];
    assert_eq!(calls.len(), 2, "{calls:?}");
    for c in calls {
        let db = &c[0].path[0];
        assert!(c.iter().all(|k| &k.path[0] == db), "one database per call");
    }
    let mut asked = r.meta.databases.lock().unwrap().clone();
    asked.sort();
    assert_eq!(asked, vec![Some(name("one")), Some(name("two"))]);
}

/// PostgreSQL passes store no per-object mark: the first check reads `Changed` and re-reads
/// the object, storing the stamp read before the re-read; the next check is `Current` and
/// reads nothing more.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reread_records_the_stamp_so_the_next_check_is_current() {
    let n = ns("public");
    let fake = FakeIntrospector::default().with_markless_passes(true);
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let keys = table_keys(&r, &n);
    assert!(
        r.catalog.current().namespaces[&n]
            .children(0)
            .all(|(_, o)| o.state() == StateMark::None)
    );
    bounded(r.handle.check_state(keys.clone())).await.unwrap();
    assert_eq!(fragments(&r).len(), 1);
    assert!(
        r.catalog.current().namespaces[&n]
            .children(0)
            .all(|(_, o)| matches!(o.state(), StateMark::Hash(_))),
        "the stamps are stored as marks"
    );
    bounded(r.handle.check_state(keys)).await.unwrap();
    assert_eq!(fragments(&r).len(), 1, "current: nothing re-read");
}

/// D-039 across passes (T10 carry M-1): background validation never removes a namespace's
/// objects in many small steps around the guard. Once the passes since the last user decision
/// would remove more than half, the pass is held for the user, with the names it would remove.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_validation_never_deletes_in_small_steps() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(40));
    let (r, hand) = rig_with_clock(
        fake,
        RigOpts {
            validate_every: Some(TEN_MIN),
            ..RigOpts::default()
        },
    );
    load(&r, &n).await;
    let drop_range = |from: usize, to: usize| {
        r.fake.edit_ns(&n, |s| {
            s.objects.retain(|o| {
                let i: usize = o.name[1..].parse().unwrap();
                !(from..to).contains(&i)
            });
        });
    };
    // 12 of 40 (30 %): under the guard; applied.
    drop_range(0, 12);
    round(&r, &hand, TEN_MIN).await;
    assert_eq!(tables_in(&r, &n).len(), 28);
    assert_eq!(r.handle.pending_confirmation(&n), None);
    // 10 of the remaining 28 (36 %): under the per-pass guard, but 22 of 40 since the user last
    // looked (55 %): held.
    drop_range(12, 22);
    round(&r, &hand, TEN_MIN).await;
    assert_eq!(tables_in(&r, &n).len(), 28, "nothing more removed");
    let held = r
        .handle
        .pending_confirmation(&n)
        .expect("held for the user");
    let mut names: Vec<String> = held.iter().map(|k| k.name().to_string()).collect();
    names.sort();
    let mut want: Vec<String> = (12..22).map(|i| format!("t{i}")).collect();
    want.sort();
    assert_eq!(names, want);
    assert!(matches!(
        state(&r, &n),
        LoadState::Loaded { stale: true, .. }
    ));
    // The user removes them: applied, and the count starts over.
    bounded(r.handle.confirm_pending(n.clone(), true))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 18);
    assert_eq!(r.handle.pending_confirmation(&n), None);
}

/// D-039 / T24: a held mass deletion names what it would remove (the dialog's list).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_mass_deletion_names_its_objects() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(30));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    r.fake.edit_ns(&n, |s| s.objects.truncate(5));
    load(&r, &n).await;
    let held = r.handle.pending_confirmation(&n).expect("held");
    assert_eq!(held.len(), 25);
    assert!(held.iter().all(|k| k.kind == ObjectKind::Table));
    bounded(r.handle.confirm_pending(n.clone(), false))
        .await
        .unwrap();
    assert_eq!(r.handle.pending_confirmation(&n), None);
}

/// A namespace verdict compares the introspector's two sides by equality.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn check_namespace_compares_the_cached_and_the_server_side() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    load(&r, &n).await;
    let snap = r.catalog.current();
    let arena = snap.namespaces[&n].clone();
    let mut conn = MetaConn::offline(DbmsKind::Postgres);
    let cached: NsCheck = r.fake.cached_check(&arena).unwrap();
    assert_eq!(cached.objects, 3);
    assert_eq!(
        freshness::check_namespace(&r.fake, &mut conn, &n, &arena)
            .await
            .unwrap(),
        NsVerdict::Current
    );
    r.fake.touch(&n, "t0");
    assert_eq!(
        freshness::check_namespace(&r.fake, &mut conn, &n, &arena)
            .await
            .unwrap(),
        NsVerdict::Changed { full: false }
    );
    // An introspector without a cached side reads nothing and answers Changed.
    let markless = FakeIntrospector::default().with_markless_passes(true);
    markless.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r2 = rig_with(markless, RigOpts::default());
    load(&r2, &n).await;
    let arena2 = r2.catalog.current().namespaces[&n].clone();
    let calls = r2.fake.calls().len();
    assert_eq!(
        freshness::check_namespace(&r2.fake, &mut conn, &n, &arena2)
            .await
            .unwrap(),
        NsVerdict::Changed { full: false }
    );
    assert_eq!(r2.fake.calls().len(), calls, "nothing read");
}

/// T16b review carry: a role renamed (or dropped) on the server leaves the grantee names of
/// loaded grants stale: they are marked so (re-read on demand), never shown as current.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_role_rename_marks_loaded_grants_stale() {
    use hacivat_introspect::engine::{Engine, EngineConfig, Filters, LevelPolicy};
    use hacivat_introspect::scope::parse_scope;
    use hacivat_introspect::testing::fake::FakeMetaSource;
    let srv = NsKey::server();
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(
        srv.clone(),
        vec![
            FakeObject::role("alice").with_oid(500),
            FakeObject::role("bob").with_oid(501),
        ],
    );
    fake.put_ns(n.clone(), FakeIntrospector::tables(2));
    let meta = FakeMetaSource::new(DbmsKind::Postgres);
    fake.link_meta(meta.clone());
    let catalog = Arc::new(Catalog::new(Arc::new(ServerFacts::new(
        DbmsKind::Postgres,
        hacivat_meta::name::IdentCtx::new(DialectId::Postgres),
    ))));
    let publisher = catalog.publisher().unwrap();
    // A grants facet loaded earlier (Task 15b loads them; the catalog holds its state).
    // ## m4-t15b-facets (named exception in Task 22's test): the facet is of a namespace this
    // test never reads, because a pass that re-reads an object drops that object's loaded
    // grants (T16b's rule, applied by Task 15b) and the first pass of `n` re-reads `t0`.
    let other = ns("other");
    let grants = ContainerKey::Facet {
        ns: other.clone(),
        object: table_key(&other, "t0", Some(20_000)),
        facet: hacivat_meta::props::FacetKind::Grants,
    };
    publisher
        .publish(vec![CatalogChange::SetState(
            grants.clone(),
            LoadState::Loaded {
                level: Level::Structure,
                at: std::time::SystemTime::now(),
                watermark: Watermark::None,
                stale: false,
            },
        )])
        .unwrap();
    let handle = Engine::start(EngineConfig {
        profile: hacivat_proto::ProfileId::new(),
        dbms: DbmsKind::Postgres,
        meta: meta.clone(),
        introspector: Arc::new(fake.clone()),
        catalog: catalog.clone(),
        catalog_publisher: publisher,
        cache: None,
        levels: LevelPolicy::default(),
        scope: parse_scope("@:*", DialectId::Postgres).unwrap(),
        current: CurrentCtx::default(),
        filters: Filters::default(),
        validate_every: None,
        limits: EngineLimits::default(),
        runtime: tokio::runtime::Handle::current(),
    });
    let r = Rig {
        fake,
        meta,
        catalog,
        handle,
    };
    load(&r, &srv).await;
    let st = |r: &Rig| r.catalog.current().state(&grants).clone();
    assert!(matches!(st(&r), LoadState::Loaded { stale: false, .. }));
    // A table changing leaves grants alone.
    r.fake.touch(&n, "t0");
    load(&r, &n).await;
    assert!(matches!(st(&r), LoadState::Loaded { stale: false, .. }));
    // `ALTER ROLE alice RENAME TO alicia`: same oid, new name.
    r.fake.edit_ns(&srv, |s| {
        let o = s.objects.iter_mut().find(|o| &*o.name == "alice").unwrap();
        o.name = name("alicia");
        o.version += 10;
    });
    load(&r, &srv).await;
    assert!(
        matches!(st(&r), LoadState::Loaded { stale: true, .. }),
        "{:?}",
        st(&r)
    );
}

/// T17 review m5: a change of a MySQL table re-reads the foreign keys of other cached
/// databases that reference it (their text names its columns), not only the same schema's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cross_schema_fk_referrer_is_reread() {
    let (shop, crm) = (NsKey::schema([name("shop")]), NsKey::schema([name("crm")]));
    let fake = FakeIntrospector::new(DbmsKind::MySql);
    fake.put_ns(shop.clone(), vec![FakeObject::table("orders", &["id"])]);
    fake.put_ns(
        crm.clone(),
        vec![
            FakeObject::table("notes", &["order_id"]).with_fk("fk_o", &["shop", "orders"]),
            FakeObject::table("plain", &["x"]),
        ],
    );
    let r = rig_with(
        fake,
        RigOpts {
            dbms: DbmsKind::MySql,
            scope: "*",
            ..RigOpts::default()
        },
    );
    load(&r, &shop).await;
    load(&r, &crm).await;
    let before = fragments(&r).len();
    // `ALTER TABLE shop.orders RENAME COLUMN id TO order_no`.
    r.fake.edit_ns(&shop, |s| {
        s.objects[0].columns = vec![name("order_no")];
        s.objects[0].version += 1;
    });
    load(&r, &shop).await;
    bounded(wait_until(Duration::from_secs(60), || {
        fragments(&r).len() > before
    }))
    .await;
    idle(&r.handle).await;
    let frags = &fragments(&r)[before..];
    assert_eq!(frags.len(), 1, "{frags:?}");
    assert_eq!(
        frags[0].iter().map(|k| k.path.to_vec()).collect::<Vec<_>>(),
        vec![vec![name("crm"), name("notes")]]
    );
}
