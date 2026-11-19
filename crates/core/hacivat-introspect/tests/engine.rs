//! The introspection engine (M4 Task 15; alg/02 HV-INTRO-01 test cases 1–4 and edge cases
//! 1–14 with fakes; the Docker cases 5–6 and 8 run in Tasks 16, 17 and 28).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures::future::join_all;
use hacivat_introspect::IntroError;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::levels::NsRole::{Current, Other, System};
use hacivat_introspect::engine::{
    EngineEvent, EngineLimits, LevelPolicy, LevelSource, NsRole, Phase, Progress, TaskProgress,
    auto_level,
};
use hacivat_introspect::pack::Mode;
use hacivat_introspect::scope::{NsInfo, parse_scope};
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::merge::Trigger;
use hacivat_meta::persist::{CacheStore, Encryption, WriterId};
use hacivat_meta::snapshot::{CatalogEvent, ContainerKey, IntroErrorCode, LoadState};
use hacivat_meta::view::{Lookup, ModelView, SnapshotView, UnknownReason};
use hacivat_meta::{Level, NsKey, ObjectKind};
use hacivat_session::meta::MetaError;
use sql_dialect::DialectId;

fn ns(schema: &str) -> NsKey {
    pg_ns("db", schema)
}

fn key(schema: &str, t: &str) -> hacivat_meta::ObjectKey {
    table_key(&ns(schema), t, None)
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
    a.children(0)
        .filter(|(_, o)| o.kind() == ObjectKind::Table)
        .map(|(_, o)| o.name().to_string())
        .collect()
}

fn introspects(r: &Rig) -> Vec<(NsKey, Mode, Level)> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Introspect { ns, mode, level } => Some((ns, mode, level)),
            _ => None,
        })
        .collect()
}

fn fragments(r: &Rig) -> Vec<(Vec<hacivat_meta::ObjectKey>, Level)> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Fragment { keys, level, .. } => Some((keys, level)),
            _ => None,
        })
        .collect()
}

fn is_introspect_of(n: &NsKey, m: Mode) -> impl Fn(&FakeCall) -> bool + Send + Sync + 'static {
    let n = n.clone();
    move |c| matches!(c, FakeCall::Introspect { ns, mode, .. } if *ns == n && *mode == m)
}

fn loaded_level(s: &LoadState) -> Option<Level> {
    match s {
        LoadState::Loaded { level, .. } => Some(*level),
        _ => None,
    }
}

// ---- HV-INTRO-01 test cases 1–4 -------------------------------------------------------------

/// Test case 1: three children requests within 200 ms in a 50,000-table namespace: the first
/// reads one object, the third upgrades to one bulk pass; at most 4 catalog queries in all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bulk_upgrade_after_third_request_total_queries_le_4() {
    let big = ns("big");
    let fake = FakeIntrospector::new(hacivat_proto::DbmsKind::Postgres);
    fake.put_ns(big.clone(), FakeIntrospector::tables(50_000));
    let r = rig_with(fake, RigOpts::default());
    // Names first: 50,000 objects stay at `Names` (D-038), so no full pass follows.
    bounded(r.handle.expand(big.clone())).await.unwrap();
    assert_eq!(loaded_level(&state(&r, &big)), Some(Level::Names));
    let before = r.fake.queries();
    let calls_before = r.fake.calls().len();
    let t = |i: u32| table_key(&big, &format!("t{i}"), Some(20_000 + i));
    let got = bounded(join_all([
        r.handle.load_children(t(1)),
        r.handle.load_children(t(2)),
        r.handle.load_children(t(3)),
    ]))
    .await;
    assert!(got.iter().all(Result::is_ok), "{got:?}");
    let queries = r.fake.queries() - before;
    assert!(queries <= 4, "{queries} catalog queries");
    let after: Vec<FakeCall> = r.fake.calls()[calls_before..].to_vec();
    // One bulk pass at `Structure`, never one query per object.
    assert_eq!(
        after
            .iter()
            .filter(|c| matches!(c, FakeCall::Introspect { mode: Mode::Full, level: Level::Structure, .. }))
            .count(),
        1,
        "{after:?}"
    );
    assert!(
        after
            .iter()
            .filter(|c| matches!(c, FakeCall::Fragment { .. }))
            .count()
            <= 2
    );
    let snap = r.catalog.current();
    let arena = &snap.namespaces[&big];
    let t1 = arena.find(&t(1)).expect("t1");
    assert_eq!(arena.children(t1).count(), 1, "t1's column is loaded");
}

/// Test case 2: a children request in a 100-table namespace goes bulk at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_namespace_goes_bulk_immediately() {
    let small = ns("small");
    let fake = FakeIntrospector::default();
    fake.put_ns(small.clone(), FakeIntrospector::tables(100));
    let mut levels = LevelPolicy::default();
    levels.set_override(ContainerKey::Namespace(small.clone()), Some(Level::Names));
    let r = rig_with(
        fake,
        RigOpts {
            levels,
            ..RigOpts::default()
        },
    );
    bounded(r.handle.expand(small.clone())).await.unwrap();
    let calls_before = r.fake.calls().len();
    bounded(r.handle.load_children(table_key(&small, "t7", Some(20_007))))
        .await
        .unwrap();
    let after = r.fake.calls()[calls_before..].to_vec();
    assert_eq!(
        after,
        vec![FakeCall::Introspect {
            ns: small,
            mode: Mode::Full,
            level: Level::Structure
        }]
    );
}

/// Test case 3: 100 concurrent loads of one namespace during its first load share one query.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hundred_concurrent_children_calls_one_query() {
    let n = ns("wide");
    let fake = FakeIntrospector::default();
    // Above 5,000 objects an `Other` namespace stays at names: one brief pass is the load.
    fake.put_ns(n.clone(), FakeIntrospector::tables(6_000));
    let gate = fake.gate(is_introspect_of(&n, Mode::Brief));
    let r = rig_with(fake, RigOpts::default());
    let tickets: Vec<_> = (0..100).map(|_| r.handle.expand(n.clone())).collect();
    bounded(gate.entered()).await;
    gate.release();
    let got = bounded(join_all(tickets)).await;
    let first = got[0].clone().unwrap();
    assert!(got.iter().all(|g| *g == Ok(first)));
    assert_eq!(r.fake.queries(), 1);
    assert_eq!(introspects(&r), vec![(n, Mode::Brief, Level::Names)]);
}

/// Test case 4 (edge 1): a cancel between the two batches of a pass publishes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_between_batches_publishes_nothing() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(10));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let before = r.catalog.current();
    let state_before = state(&r, &n);
    r.fake.touch(&n, "t1");
    let gate = r.fake.gate(is_introspect_of(&n, Mode::Full));
    let queries = r.fake.queries();
    let t = r.handle.refresh(Target::Namespace(n.clone()), Trigger::User);
    bounded(gate.entered()).await;
    // The first batch ran; cancel before the second.
    assert_eq!(r.fake.queries(), queries + 1);
    r.handle.cancel(n.clone());
    assert_eq!(bounded(t).await, Err(IntroError::Cancelled));
    let after = r.catalog.current();
    assert_eq!(after.generation, before.generation);
    assert!(Arc::ptr_eq(&after.namespaces[&n], &before.namespaces[&n]));
    assert_eq!(state(&r, &n), state_before);
    assert_eq!(r.fake.queries(), queries + 1, "the second batch never ran");
}

// ---- edge cases ------------------------------------------------------------------------------

/// Edge 3: a legitimately empty namespace is loaded with zero objects.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_namespace_completes_with_zero_objects() {
    let n = ns("empty");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), Vec::new());
    let r = rig_with(fake, RigOpts::default());
    let g = bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let snap = r.catalog.current();
    assert_eq!(snap.generation, g);
    assert_eq!(snap.namespaces[&n].len(), 1, "the root only");
    assert!(matches!(
        state(&r, &n),
        LoadState::Loaded { level: Level::Structure, stale: false, .. }
    ));
    // A complete empty namespace: its table children are a fact (no table).
    let root = snap.namespaces[&n].object_key(0).unwrap();
    let view = SnapshotView::new(snap);
    assert!(matches!(view.children(&root, ObjectKind::Table), Lookup::Found(v) if v.is_empty()));
}

/// Edge 4: a child row naming a parent the pass did not return schedules a follow-up read of
/// that parent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_row_with_missing_parent_schedules_a_follow_up() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    fake.edit_ns(&n, |s| s.orphan = Some((name("ghost"), name("c"))));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 3);
    let ghost = table_key(&n, "ghost", None);
    bounded(wait_until(Duration::from_secs(60), || {
        fragments(&r).iter().any(|(keys, _)| keys.contains(&ghost))
    }))
    .await;
    // The follow-up reads the parent once and does not loop.
    idle(&r.handle).await;
    let reads = fragments(&r)
        .iter()
        .filter(|(keys, _)| keys.contains(&ghost))
        .count();
    assert_eq!(reads, 1);
}

/// Edge 5: a lost connection restarts the task once from the beginning; a second loss fails it
/// without publishing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_lost_restarts_the_task_once() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(5));
    fake.fail(
        is_introspect_of(&n, Mode::Full),
        IntroError::Meta(MetaError::ConnectionLost),
        1,
    );
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(introspects(&r).len(), 2, "restarted once");
    assert_eq!(r.meta.acquired.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(tables_in(&r, &n).len(), 5);
    // Lost twice: the task fails and nothing is published.
    r.fake.touch(&n, "t0");
    r.fake.fail(
        is_introspect_of(&n, Mode::Full),
        IntroError::Meta(MetaError::ConnectionLost),
        2,
    );
    let generation = r.catalog.current().generation;
    assert_eq!(
        bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User)).await,
        Err(IntroError::Meta(MetaError::ConnectionLost))
    );
    assert_eq!(r.catalog.current().generation, generation);
}

/// Edge 6: a kind the login cannot read fails alone; the other kinds publish, the namespace is
/// `Partial` and a miss of the failed kind is never a fact.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permission_denied_kind_fails_only_that_kind() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(
        n.clone(),
        vec![
            FakeObject::table("a", &["id"]),
            FakeObject::table("b", &["id"]),
            FakeObject::view("v", &["id"]),
        ],
    );
    fake.edit_ns(&n, |s| {
        s.failed
            .insert(ObjectKind::View, IntroErrorCode::PermissionDenied);
    });
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n), vec!["a", "b"]);
    assert_eq!(
        state(&r, &n),
        LoadState::Partial {
            reason: IntroErrorCode::PermissionDenied
        }
    );
    let snap = r.catalog.current();
    let root = snap.namespaces[&n].object_key(0).unwrap();
    assert!(matches!(
        snap.state(&ContainerKey::Children {
            ns: n.clone(),
            parent: root.clone(),
            kind: ObjectKind::View,
        }),
        LoadState::Failed {
            error: IntroErrorCode::PermissionDenied,
            ..
        }
    ));
    let view = SnapshotView::new(snap);
    assert_eq!(
        view.children(&root, ObjectKind::View).map(|v| v.len()),
        Lookup::Unknown(UnknownReason::Failed)
    );
    // Once the kind reads again, its container is no longer failed.
    r.fake.edit_ns(&n, |s| s.failed.clear());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert!(matches!(state(&r, &n), LoadState::Loaded { .. }));
    let snap = r.catalog.current();
    assert!(matches!(
        snap.state(&ContainerKey::Children {
            ns: n.clone(),
            parent: root,
            kind: ObjectKind::View,
        }),
        LoadState::Loaded { .. }
    ));
}

/// Edge 7 / step 7: a namespace whose pass timed out twice is read one level lower, with the
/// hint. (The fake answers `Timeout` at once: no clock is involved.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_twice_lowers_the_level_and_hints() {
    let n = ns("slow");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(10));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(loaded_level(&state(&r, &n)), Some(Level::Structure));
    let mut events = r.handle.events();
    r.fake
        .fail(is_introspect_of(&n, Mode::Full), IntroError::Timeout, 2);
    for _ in 0..2 {
        assert_eq!(
            bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User)).await,
            Err(IntroError::Timeout)
        );
    }
    assert_eq!(
        events.try_recv().unwrap(),
        EngineEvent::LevelLowered {
            ns: n.clone(),
            level: Level::Names
        }
    );
    let choice = r.handle.level_for(&n, NsRole::Other, 10);
    assert_eq!((choice.level, choice.lowered), (Level::Names, 1));
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(
        introspects(&r).last(),
        Some(&(n.clone(), Mode::Full, Level::Names))
    );
}

/// Edge 8: a scope change during a running task discards its result for the namespace that
/// left the scope.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scope_change_discards_out_of_scope_results() {
    let (a, b) = (ns("a"), ns("b"));
    let fake = FakeIntrospector::default();
    fake.put_ns(a.clone(), FakeIntrospector::tables(3));
    fake.put_ns(b.clone(), FakeIntrospector::tables(3));
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current("db", "a"),
            ..RigOpts::default()
        },
    );
    bounded(r.handle.on_connect()).await.unwrap();
    let gate = r.fake.gate(is_introspect_of(&b, Mode::Brief));
    let t = r.handle.expand(b.clone());
    bounded(gate.entered()).await;
    r.handle
        .set_scope(parse_scope("@:@", DialectId::Postgres).unwrap());
    gate.release();
    assert_eq!(bounded(t).await, Err(IntroError::Cancelled));
    bounded(wait_until(Duration::from_secs(60), || {
        state(&r, &b) == LoadState::OutOfScope
    }))
    .await;
    assert!(!r.catalog.current().namespaces.contains_key(&b));
    // An expansion of it reads nothing.
    let calls = r.fake.calls().len();
    assert_eq!(
        bounded(r.handle.expand(b.clone())).await,
        Err(IntroError::Cancelled)
    );
    assert_eq!(r.fake.calls().len(), calls);
    assert!(r.catalog.current().namespaces.contains_key(&a));
}

/// Edge 10: on a single-connection profile a task waits while the user's session holds a
/// transaction (it keeps asking for the connection and reads nothing), then runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_connection_profile_waits_for_user_idle() {
    use std::sync::atomic::Ordering::SeqCst;
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(
        fake,
        RigOpts {
            limits: EngineLimits {
                busy_retry: Duration::from_millis(5),
                ..EngineLimits::default()
            },
            ..RigOpts::default()
        },
    );
    r.meta.busy.store(true, SeqCst);
    let t = tokio::spawn(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User));
    // Asked three times and refused each time: nothing was read.
    bounded(wait_until(Duration::from_secs(60), || {
        r.meta.attempts.load(SeqCst) >= 3
    }))
    .await;
    assert!(r.fake.calls().is_empty());
    assert!(!t.is_finished());
    r.meta.busy.store(false, SeqCst);
    bounded(t).await.unwrap().unwrap();
    assert_eq!(introspects(&r).len(), 1);
    assert_eq!(r.meta.acquired.load(SeqCst), 1);
}

/// Edge 13: two windows asking for one namespace share one task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_windows_share_one_future() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(6_000));
    let gate = fake.gate(is_introspect_of(&n, Mode::Brief));
    let r = rig_with(fake, RigOpts::default());
    let one = r.handle.expand(n.clone());
    bounded(gate.entered()).await;
    let two = r.handle.clone().expand(n.clone());
    gate.release();
    let (a, b) = bounded(futures::future::join(one, two)).await;
    assert_eq!(a, b);
    assert!(a.is_ok());
    assert_eq!(introspects(&r).len(), 1);
}

/// Edge 14: an elevation waits for the namespace's running pass, and is answered by it when
/// that pass reads its level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elevation_waits_for_the_running_namespace_task() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(10));
    let gate = fake.gate(is_introspect_of(&n, Mode::Full));
    let r = rig_with(fake, RigOpts::default());
    let pass = r.handle.refresh(Target::Namespace(n.clone()), Trigger::User);
    bounded(gate.entered()).await;
    let t1 = table_key(&n, "t1", Some(20_001));
    let t2 = table_key(&n, "t2", Some(20_002));
    let covered = r.handle.elevate(t1.clone(), Level::Structure);
    let higher = r.handle.elevate(t2.clone(), Level::Sources);
    // Nothing else reads while the namespace's pass runs.
    tokio::task::yield_now().await;
    assert!(fragments(&r).is_empty());
    gate.release();
    let pass_gen = bounded(pass).await.unwrap();
    assert_eq!(bounded(covered).await, Ok(pass_gen));
    let higher_gen = bounded(higher).await.unwrap();
    assert!(higher_gen > pass_gen);
    // Only the higher level needed a fragment, after the pass.
    assert_eq!(fragments(&r), vec![(vec![t2], Level::Sources)]);
    let calls = r.fake.calls();
    let pos = |f: &dyn Fn(&FakeCall) -> bool| calls.iter().position(f).unwrap();
    assert!(
        pos(&|c| matches!(c, FakeCall::Introspect { .. }))
            < pos(&|c| matches!(c, FakeCall::Fragment { .. }))
    );
}

// ---- levels ----------------------------------------------------------------------------------

#[test]
fn auto_level_thresholds_d038() {
    assert_eq!(auto_level(Current, 2_000), Level::Sources);
    assert_eq!(auto_level(Current, 2_001), Level::Structure);
    assert_eq!(auto_level(Current, 20_001), Level::Names);
    assert_eq!(auto_level(Other, 5_000), Level::Structure);
    assert_eq!(auto_level(Other, 5_001), Level::Names);
    assert_eq!(auto_level(System, 10), Level::Names);
}

#[test]
fn level_override_is_inherited_by_children() {
    let mut p = LevelPolicy::default();
    let s = pg_ns("db", "s");
    let t = pg_ns("db", "t");
    let elsewhere = pg_ns("other", "s");
    assert_eq!(p.level_for(&s, Other, 10_000).level, Level::Names);
    // A pill on the database reaches every schema of it, not another database's.
    p.set_override(
        ContainerKey::Namespace(NsKey::database(name("db"))),
        Some(Level::Sources),
    );
    for k in [&s, &t] {
        let c = p.level_for(k, Other, 10_000);
        assert_eq!((c.level, c.source), (Level::Sources, LevelSource::Override));
    }
    assert_eq!(
        p.level_for(&elsewhere, Other, 10_000).source,
        LevelSource::Auto
    );
    // A schema's own pill wins over its database's.
    p.set_override(ContainerKey::Namespace(s.clone()), Some(Level::Names));
    assert_eq!(p.level_for(&s, Other, 10).level, Level::Names);
    assert_eq!(p.level_for(&t, Other, 10).level, Level::Sources);
    // Cleared: back to the auto level.
    p.set_override(ContainerKey::Namespace(NsKey::database(name("db"))), None);
    let c = p.level_for(&t, Other, 10);
    assert_eq!((c.level, c.source), (Level::Structure, LevelSource::Auto));
}

// ---- on connect ------------------------------------------------------------------------------

/// Steps 1–2 with a cache: the cached namespace is published stale before anything is read,
/// then validated incrementally, never re-read in full.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_connect_with_cache_publishes_stale_then_runs_incremental_not_full() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let n = ns("public");
    let open = || {
        let o = CacheStore::open(&path, Encryption::None, WriterId(7)).expect("a cache");
        Arc::new(std::sync::Mutex::new(o.store))
    };
    let opts = || RigOpts {
        incremental: true,
        current: pg_current("db", "public"),
        limits: EngineLimits {
            save_debounce: Duration::ZERO,
            ..EngineLimits::default()
        },
        ..RigOpts::default()
    };
    // A first session fills the cache.
    {
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), FakeIntrospector::tables(4));
        let r = rig_with(
            fake,
            RigOpts {
                cache: Some(open()),
                ..opts()
            },
        );
        bounded(r.handle.on_connect()).await.unwrap();
        r.handle.shutdown();
        bounded(r.handle.closed()).await;
    }
    // The next start publishes it stale before any read.
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(4));
    fake.touch(&n, "t2");
    let r = rig_with(
        fake,
        RigOpts {
            cache: Some(open()),
            ..opts()
        },
    );
    bounded(r.handle.ready()).await.unwrap();
    assert!(r.fake.calls().is_empty());
    assert!(matches!(
        state(&r, &n),
        LoadState::Loaded { stale: true, .. }
    ));
    assert_eq!(tables_in(&r, &n).len(), 4);
    bounded(r.handle.on_connect()).await.unwrap();
    assert_eq!(
        r.fake.calls(),
        vec![
            FakeCall::ListNamespaces,
            FakeCall::Introspect {
                ns: n.clone(),
                mode: Mode::Incremental,
                level: Level::Sources
            }
        ]
    );
    assert!(matches!(
        state(&r, &n),
        LoadState::Loaded { stale: false, .. }
    ));
}

/// Step 2 without a cache: the current namespace's names are published first, then its auto
/// level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_connect_without_cache_paints_brief_names_first() {
    let n = ns("public");
    let other = ns("other");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(10));
    fake.put_ns(other.clone(), FakeIntrospector::tables(10));
    let gate = fake.gate(is_introspect_of(&n, Mode::Full));
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current("db", "public"),
            ..RigOpts::default()
        },
    );
    let t = r.handle.on_connect();
    bounded(gate.entered()).await;
    // While the full pass runs, the names are already there.
    assert_eq!(loaded_level(&state(&r, &n)), Some(Level::Names));
    assert_eq!(tables_in(&r, &n).len(), 10);
    // The other namespace is listed, not read (first expansion).
    assert_eq!(state(&r, &other), LoadState::NotLoaded);
    gate.release();
    bounded(t).await.unwrap();
    assert_eq!(loaded_level(&state(&r, &n)), Some(Level::Sources));
    assert_eq!(
        introspects(&r),
        vec![
            (n.clone(), Mode::Brief, Level::Names),
            (n.clone(), Mode::Full, Level::Sources)
        ]
    );
    // The schema list is loaded: a schema the server does not have is a fact.
    assert!(matches!(
        r.catalog
            .current()
            .state(&ContainerKey::schema_list(Some(&name("db")))),
        LoadState::Loaded { .. }
    ));
}

// ---- the mass-deletion guard -----------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_confirmation_changes_nothing_until_confirmed() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(30));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 30);
    let mut events = r.catalog.subscribe();
    // 25 of 30 vanish (> 50 % of ≥ 20): a background pass holds it.
    r.fake.put_ns(n.clone(), FakeIntrospector::tables(5));
    let arena = r.catalog.current().namespaces[&n].clone();
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::Background))
        .await
        .unwrap();
    let snap = r.catalog.current();
    assert!(Arc::ptr_eq(&snap.namespaces[&n], &arena), "nothing applied");
    assert!(matches!(state(&r, &n), LoadState::Loaded { stale: true, .. }));
    let mut announced = None;
    while let Ok(ev) = events.try_recv() {
        if let CatalogEvent::PendingConfirmation { ns, count } = ev {
            announced = Some((ns, count));
        }
    }
    assert_eq!(announced, Some((n.clone(), 25)));
    // "Keep cached": the vanished tables stay; the namespace is fresh again.
    bounded(r.handle.confirm_pending(n.clone(), false))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 30);
    assert!(matches!(state(&r, &n), LoadState::Loaded { stale: false, .. }));
    // Held again, then "Remove from cache".
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::Background))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 30);
    bounded(r.handle.confirm_pending(n.clone(), true))
        .await
        .unwrap();
    assert_eq!(tables_in(&r, &n).len(), 5);
    // Nothing pending: answered with the current generation.
    let generation = r.catalog.current().generation;
    assert_eq!(
        bounded(r.handle.confirm_pending(n.clone(), true)).await,
        Ok(generation)
    );
}

// ---- periodic validation ---------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn periodic_validation_skips_during_user_tx_on_single_connection() {
    use std::sync::atomic::Ordering::SeqCst;
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(
        fake,
        RigOpts {
            validate_every: Some(Duration::from_millis(20)),
            ..RigOpts::default()
        },
    );
    r.meta.in_tx.store(true, SeqCst);
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let passes = || introspects(&r).len();
    let before = passes();
    // Three validation rounds came due inside the user's transaction: none read anything.
    let checks = r.meta.tx_checks.load(SeqCst);
    bounded(wait_until(Duration::from_secs(60), || {
        r.meta.tx_checks.load(SeqCst) >= checks + 3
    }))
    .await;
    assert_eq!(passes(), before, "no validation inside the user's transaction");
    r.meta.in_tx.store(false, SeqCst);
    bounded(wait_until(Duration::from_secs(60), || passes() > before)).await;
    let (_, mode, level) = introspects(&r).pop().unwrap();
    // No incremental capability: the validation is a full pass at the namespace's level.
    assert_eq!((mode, level), (Mode::Full, Level::Structure));
}

// ---- progress --------------------------------------------------------------------------------

/// The phase label of the UI: an exhaustive match, so a new phase is a compile error here.
fn phase_id(p: Phase) -> &'static str {
    match p {
        Phase::ListNamespaces => "list-namespaces",
        Phase::Tables => "tables",
        Phase::Columns => "columns",
        Phase::Indexes => "indexes",
        Phase::Constraints => "constraints",
        Phase::Triggers => "triggers",
        Phase::Routines => "routines",
        Phase::Types => "types",
        Phase::Sequences => "sequences",
        Phase::Comments => "comments",
        Phase::Sources => "sources",
        Phase::Facets => "facets",
        Phase::Existence => "existence",
        Phase::Validate => "validate",
    }
}

#[test]
fn progress_phases_are_a_closed_enum_with_counts() {
    assert_eq!(phase_id(Phase::Tables), "tables");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let n = ns("public");
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), FakeIntrospector::tables(3));
        let pass_gate = fake.gate(is_introspect_of(&n, Mode::Full));
        let r = rig_with(fake, RigOpts::default());
        let mut progress = r.handle.progress();
        let pass = r.handle.refresh(Target::Namespace(n.clone()), Trigger::User);
        bounded(pass_gate.entered()).await;
        let p: Progress = progress.borrow_and_update().clone();
        assert_eq!(
            p.tasks,
            vec![TaskProgress {
                ns: n.clone(),
                phase: Phase::Tables,
                done: 0,
                total: None
            }]
        );
        pass_gate.release();
        bounded(pass).await.unwrap();
        // A fragment has a determinate count: its objects.
        let frag_gate = r
            .fake
            .gate(|c| matches!(c, FakeCall::Fragment { .. }));
        let keys = vec![key("public", "t0"), key("public", "t1")];
        let frag = r.handle.fragment(n.clone(), keys);
        bounded(frag_gate.entered()).await;
        let p = r.handle.progress().borrow().clone();
        assert_eq!(p.tasks[0].phase, Phase::Columns);
        assert_eq!((p.tasks[0].done, p.tasks[0].total), (0, Some(2)));
        frag_gate.release();
        bounded(frag).await.unwrap();
        idle(&r.handle).await;
        assert!(r.handle.progress().borrow().tasks.is_empty());
    });
}

// ---- tickets ---------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ticket_resolves_with_the_publishing_generation() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    let g = bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let snap = r.catalog.current();
    assert_eq!(snap.generation, g);
    assert_eq!(snap.namespaces[&n].generation(), g, "the result is in it");
    // A fragment's ticket too.
    r.fake.touch(&n, "t1");
    let g2 = bounded(r.handle.fragment(n.clone(), vec![key("public", "t1")]))
        .await
        .unwrap();
    assert!(g2 > g);
    assert_eq!(r.catalog.current().namespaces[&n].generation(), g2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_ticket_does_not_cancel_the_shared_task() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(6_000));
    let gate = fake.gate(is_introspect_of(&n, Mode::Brief));
    let r = rig_with(fake, RigOpts::default());
    let first = r.handle.expand(n.clone());
    bounded(gate.entered()).await;
    let second = r.handle.expand(n.clone());
    drop(first);
    gate.release();
    let g = bounded(second).await.unwrap();
    assert_eq!(r.catalog.current().namespaces[&n].generation(), g);
    assert_eq!(introspects(&r).len(), 1);
    // A dropped ticket's own task still publishes.
    let n2 = ns("solo");
    r.fake.put_ns(n2.clone(), FakeIntrospector::tables(6_000));
    drop(r.handle.expand(n2.clone()));
    bounded(wait_until(Duration::from_secs(60), || {
        r.catalog.current().namespaces.contains_key(&n2)
    }))
    .await;
}

/// 20 fragments of one namespace sent at once (well within the 50 ms window) run and publish
/// once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fragment_burst_publishes_once_per_window() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(20));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let mut events = r.catalog.subscribe();
    let generation = r.catalog.current().generation;
    let tickets: Vec<_> = (0..20)
        .map(|i| r.handle.fragment(n.clone(), vec![key("public", &format!("t{i}"))]))
        .collect();
    let got = bounded(join_all(tickets)).await;
    let g = got[0].clone().unwrap();
    assert!(got.iter().all(|x| *x == Ok(g)));
    assert_eq!(g, generation + 1, "one publish");
    let frags = fragments(&r);
    assert_eq!(frags.len(), 1);
    assert_eq!(frags[0].0.len(), 20);
    let mut published = 0;
    while let Ok(ev) = events.try_recv() {
        if matches!(ev, CatalogEvent::Published { .. }) {
            published += 1;
        }
    }
    assert_eq!(published, 1);
}

/// PG spike finding 8: a concurrent drop (`XX000`) is retried once (logged), then the
/// namespace fails without publishing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_race_is_retried_once_then_fails_without_publishing() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let mut events = r.handle.events();
    // Once: retried, and it succeeds.
    r.fake
        .fail(is_introspect_of(&n, Mode::Full), IntroError::CatalogRace, 1);
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    assert_eq!(
        events.try_recv().unwrap(),
        EngineEvent::CatalogRaceRetried { ns: n.clone() }
    );
    // Twice: it fails, nothing is published, one retry was logged.
    r.fake
        .fail(is_introspect_of(&n, Mode::Full), IntroError::CatalogRace, 2);
    let before = r.catalog.current();
    let calls = introspects(&r).len();
    assert_eq!(
        bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User)).await,
        Err(IntroError::CatalogRace)
    );
    assert_eq!(introspects(&r).len(), calls + 2);
    let after = r.catalog.current();
    assert_eq!(after.generation, before.generation);
    assert!(Arc::ptr_eq(&after.namespaces[&n], &before.namespaces[&n]));
    assert_eq!(
        events.try_recv().unwrap(),
        EngineEvent::CatalogRaceRetried { ns: n.clone() }
    );
    assert!(events.try_recv().is_err());
}

// ---- beyond the plan's list: carries ---------------------------------------------------------

/// T15 carry: a first load that fails leaves the namespace `Failed`; one with a previous state
/// keeps it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failure_marks_failed_only_without_previous_state() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    fake.fail(is_introspect_of(&n, Mode::Full), IntroError::Permission { kind: ObjectKind::Table }, 1);
    let r = rig_with(fake, RigOpts::default());
    assert!(bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .is_err());
    assert!(matches!(
        state(&r, &n),
        LoadState::Failed {
            error: IntroErrorCode::PermissionDenied,
            ..
        }
    ));
    bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .unwrap();
    let loaded = state(&r, &n);
    r.fake.fail(
        is_introspect_of(&n, Mode::Full),
        IntroError::Meta(MetaError::Unavailable),
        1,
    );
    assert!(bounded(r.handle.refresh(Target::Namespace(n.clone()), Trigger::User))
        .await
        .is_err());
    assert_eq!(state(&r, &n), loaded);
}

/// The scope refresh lists every namespace (in scope: not loaded; out of it: out of scope),
/// removes the ones the server dropped, and loads the schema list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scope_refresh_lists_and_drops_namespaces() {
    let (a, gone) = (ns("a"), ns("gone"));
    let fake = FakeIntrospector::default();
    fake.put_ns(a.clone(), FakeIntrospector::tables(2));
    fake.put_ns(gone.clone(), FakeIntrospector::tables(2));
    let r = rig_with(
        fake,
        RigOpts {
            current: pg_current("db", "a"),
            ..RigOpts::default()
        },
    );
    bounded(r.handle.expand(gone.clone())).await.unwrap();
    let mut events = r.handle.events();
    r.fake.set_listing(vec![
        NsInfo {
            key: a.clone(),
            system: false,
        },
        NsInfo {
            key: ns("pg_catalog"),
            system: true,
        },
    ]);
    bounded(r.handle.refresh(Target::ScopeRefresh, Trigger::User))
        .await
        .unwrap();
    assert_eq!(state(&r, &a), LoadState::NotLoaded);
    assert_eq!(state(&r, &ns("pg_catalog")), LoadState::OutOfScope);
    assert!(!r.catalog.current().namespaces.contains_key(&gone));
    assert_eq!(
        events.try_recv().unwrap(),
        EngineEvent::NamespaceDropped { ns: gone.clone() }
    );
    // A schema of the database that the server does not list is now a fact.
    let snap = r.catalog.current();
    let view = SnapshotView::new(snap);
    let probe = hacivat_meta::name::NamePart {
        text: name("missing"),
        quoted: false,
    };
    let t = hacivat_meta::name::NamePart {
        text: name("t"),
        quoted: false,
    };
    let db = hacivat_meta::name::NamePart {
        text: name("db"),
        quoted: false,
    };
    assert!(matches!(
        view.lookup(&[db, probe, t], hacivat_meta::KindGroup::Relations),
        Lookup::Missing(_)
    ));
}

/// The engine is the catalog's only writer: nobody else can take the publisher.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_engine_is_the_only_writer() {
    let r = rig_with(FakeIntrospector::default(), RigOpts::default());
    assert!(r.catalog.publisher().is_none());
    r.handle.shutdown();
    bounded(r.handle.closed()).await;
    // After shutdown every request answers Cancelled.
    assert_eq!(
        bounded(r.handle.expand(ns("public"))).await,
        Err(IntroError::Cancelled)
    );
}
