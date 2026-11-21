//! T15 review: hostile tests the implementer did not write. Each asserts what the task
//! promises; a failing test is a finding. Adopted in fix round 1 (I-3 and m1 now pass and
//! assert the fixes).

#![cfg(feature = "session")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use futures::future::join_all;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::EngineLimits;
use hacivat_introspect::pack::Mode;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::FakeCall;
use hacivat_meta::merge::Trigger;
use hacivat_meta::snapshot::{ContainerKey, LoadState};
use hacivat_meta::{Level, NsKey};

fn ns(schema: &str) -> NsKey {
    pg_ns("db", schema)
}

fn passes(r: &Rig) -> Vec<(NsKey, Mode)> {
    r.fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Introspect { ns, mode, .. } => Some((ns, mode)),
            _ => None,
        })
        .collect()
}

/// Starvation / priority: with one worker busy, a user expansion that arrives after three
/// queued background refreshes runs before them (and so does its names-first follow-up).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_user_request_overtakes_queued_background_work() {
    let fake = FakeIntrospector::default();
    for s in ["a", "b", "c", "d", "e"] {
        fake.put_ns(ns(s), FakeIntrospector::tables(3));
    }
    let gate =
        fake.gate(|c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == pg_ns("db", "a")));
    let r = rig_with(
        fake,
        RigOpts {
            limits: EngineLimits {
                workers: 1,
                ..EngineLimits::default()
            },
            ..RigOpts::default()
        },
    );
    let mut tickets = vec![
        r.handle
            .refresh(Target::Namespace(ns("a")), Trigger::Background),
    ];
    bounded(gate.entered()).await;
    for s in ["b", "c", "d"] {
        tickets.push(
            r.handle
                .refresh(Target::Namespace(ns(s)), Trigger::Background),
        );
    }
    tickets.push(r.handle.expand(ns("e")));
    // A barrier: `Ready` is answered in order, so every request above is queued now.
    bounded(r.handle.ready()).await.unwrap();
    gate.release();
    let got = bounded(join_all(tickets)).await;
    assert!(got.iter().all(Result::is_ok), "{got:?}");
    idle(&r.handle).await;
    let order: Vec<NsKey> = passes(&r).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        order,
        vec![ns("a"), ns("e"), ns("e"), ns("b"), ns("c"), ns("d")],
        "{:?}",
        passes(&r)
    );
}

/// D-039: a held (guarded) merge that a later fragment of the same namespace silently drops.
/// "Remove from cache" then answers Ok, removes nothing, and the namespace stays stale.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_confirm_after_a_fragment_is_not_a_silent_no_op() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(30));
    let r = rig_with(fake, RigOpts::default());
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    // 25 of 30 vanish: a background pass holds the merge.
    r.fake.put_ns(n.clone(), FakeIntrospector::tables(5));
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::Background),
    )
    .await
    .unwrap();
    assert!(matches!(
        r.catalog
            .current()
            .state(&ContainerKey::Namespace(n.clone())),
        LoadState::Loaded { stale: true, .. }
    ));
    // The user expands a surviving table meanwhile (a fragment of the namespace).
    bounded(
        r.handle
            .fragment(n.clone(), vec![table_key(&n, "t1", Some(20_001))]),
    )
    .await
    .unwrap();
    // Then answers the prompt with "Remove from cache".
    let confirmed = bounded(r.handle.confirm_pending(n.clone(), true)).await;
    let snap = r.catalog.current();
    let tables = snap.namespaces[&n].root().child_count();
    let stale = matches!(
        snap.state(&ContainerKey::Namespace(n.clone())),
        LoadState::Loaded { stale: true, .. }
    );
    // Fix round 1 (I-3, flipped): the held merge survives the fragment; the answer re-plans a
    // pass that carries it, and the ticket resolves with the generation that applied it.
    let g = confirmed.expect("the decision applied");
    assert_eq!(snap.generation, g);
    assert_eq!(tables, 5, "the vanished tables were removed");
    assert!(!stale);
    // Nothing is held any more: a second answer applies nothing and says so.
    assert_eq!(
        bounded(r.handle.confirm_pending(n.clone(), true)).await,
        Err(hacivat_introspect::IntroError::Cancelled)
    );
}

/// I-3: "Keep cached" after a fragment keeps the vanished tables (the re-planned pass
/// suppresses the deletions), and a later pass withdraws a held merge with an event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_cached_after_a_fragment_and_a_later_pass_withdraws() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(30));
    let r = rig_with(fake, RigOpts::default());
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    r.fake.put_ns(n.clone(), FakeIntrospector::tables(5));
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::Background),
    )
    .await
    .unwrap();
    bounded(
        r.handle
            .fragment(n.clone(), vec![table_key(&n, "t1", Some(20_001))]),
    )
    .await
    .unwrap();
    bounded(r.handle.confirm_pending(n.clone(), false))
        .await
        .unwrap();
    let snap = r.catalog.current();
    assert_eq!(snap.namespaces[&n].root().child_count(), 30);
    assert!(matches!(
        snap.state(&ContainerKey::Namespace(n.clone())),
        LoadState::Loaded { stale: false, .. }
    ));
    // Held again, then a user's own pass of the namespace replaces it: the prompt goes.
    // ## m4-t22-freshness (named exception; ruling R-W4-9): after "Keep cached" only the
    // user's refresh is held again (background passes keep the kept objects).
    let mut events = r.handle.events();
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    assert!(r.handle.pending_confirmation(&n).is_some(), "held again");
    r.fake.put_ns(n.clone(), FakeIntrospector::tables(30));
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    let mut withdrawn = false;
    while let Ok(ev) = events.try_recv() {
        withdrawn |=
            ev == hacivat_introspect::engine::EngineEvent::PendingWithdrawn { ns: n.clone() };
    }
    assert!(withdrawn);
    assert_eq!(
        bounded(r.handle.confirm_pending(n.clone(), true)).await,
        Err(hacivat_introspect::IntroError::Cancelled)
    );
}

/// "A task that never ends after the profile disconnects": the engine stops when every handle
/// is dropped (`rx` closes). Holding only the future `closed()` returns must not keep it alive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_closed_future_does_not_keep_a_dropped_engine_alive() {
    // Control: dropping every handle stops the engine (it drops its MetaSource).
    let r = rig_with(FakeIntrospector::default(), RigOpts::default());
    let meta = r.meta.clone();
    drop(r);
    bounded(wait_until(Duration::from_secs(60), || {
        Arc::strong_count(&meta) == 1
    }))
    .await;
    // The same with a `closed()` future held by a watcher.
    let r = rig_with(FakeIntrospector::default(), RigOpts::default());
    let meta = r.meta.clone();
    let closed = r.handle.closed();
    drop(r);
    // Fix round 1 (m1, flipped to pass): `closed` holds a token, not a sender.
    let stopped = tokio::time::timeout(Duration::from_secs(3), closed).await;
    assert!(
        stopped.is_ok(),
        "closed() never resolved: it holds a sender, so the engine (strong count {}) never stops",
        Arc::strong_count(&meta)
    );
}

/// A cancelled namespace publishes nothing, whichever of the cancel and the end of the read
/// the actor sees first: the ticket and the catalog must agree.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_cancel_racing_the_end_of_a_pass_is_consistent() {
    for round in 0..200u32 {
        let n = ns("public");
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), FakeIntrospector::tables(50));
        let r = rig_with(fake, RigOpts::default());
        let before = r.catalog.current().generation;
        let t = r
            .handle
            .refresh(Target::Namespace(n.clone()), Trigger::User);
        if round % 2 == 0 {
            tokio::task::yield_now().await;
        }
        r.handle.cancel(n.clone());
        let got = bounded(t).await;
        idle(&r.handle).await;
        let after = r.catalog.current();
        match got {
            Ok(g) => assert!(after.namespaces.contains_key(&n) && g > before),
            Err(_) => assert!(
                !after.namespaces.contains_key(&n),
                "round {round}: the ticket says {got:?} but the namespace was published"
            ),
        }
        r.handle.shutdown();
        bounded(r.handle.closed()).await;
    }
}

/// Elevation of an object of a namespace loaded at `Names` (a 6,000-table `Other` schema) goes
/// to the requested level and leaves the namespace's own state at its level.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_elevate_in_a_names_namespace_does_not_claim_the_namespace() {
    let n = ns("wide");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(6_000));
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.expand(n.clone())).await.unwrap();
    let k = table_key(&n, "t7", Some(20_007));
    bounded(r.handle.elevate(k.clone(), Level::Structure))
        .await
        .unwrap();
    let snap = r.catalog.current();
    let a = &snap.namespaces[&n];
    let at = a.find(&k).expect("t7");
    assert_eq!(a.children(at).count(), 1, "t7's column");
    assert!(matches!(
        snap.state(&ContainerKey::Namespace(n.clone())),
        LoadState::Loaded {
            level: Level::Names,
            ..
        }
    ));
    assert_eq!(a.root().child_count(), 6_000);
}

fn tables_of(r: &Rig, n: &NsKey) -> Vec<String> {
    let snap = r.catalog.current();
    let Some(a) = snap.namespaces.get(n) else {
        return Vec::new();
    };
    let mut v: Vec<String> = a.children(0).map(|(_, o)| o.name().to_string()).collect();
    v.sort();
    v
}

/// Edge 11 (scope filter on the client): objects the scope filter drops are neither stored
/// nor asked for in a follow-up fragment; a later pass keeps it that way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_scope_filter_drops_objects_without_follow_ups() {
    use hacivat_introspect::engine::Filters;
    use hacivat_introspect::filter::{FilterRole, parse_filter};
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(12));
    let filter = parse_filter(
        "t1*",
        sql_dialect::DialectId::Postgres,
        FilterRole::Introspect,
    )
    .expect("a filter");
    let r = rig_with(
        fake,
        RigOpts {
            filters: Filters {
                scope: Some(filter),
            },
            ..RigOpts::default()
        },
    );
    for _ in 0..2 {
        bounded(
            r.handle
                .refresh(Target::Namespace(n.clone()), Trigger::User),
        )
        .await
        .unwrap();
        idle(&r.handle).await;
    }
    assert_eq!(tables_of(&r, &n), vec!["t1", "t10", "t11"]);
    assert!(
        !r.fake
            .calls()
            .iter()
            .any(|c| matches!(c, FakeCall::Fragment { .. })),
        "{:?}",
        r.fake.calls()
    );
}

/// `check_state`: a changed and a dropped object are re-read and the ticket resolves with the
/// generation that holds the result; an unchanged one is not re-read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_check_state_rereads_changed_and_dropped_only() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(4));
    let r = rig_with(fake, RigOpts::default());
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    r.fake.touch(&n, "t1");
    r.fake
        .edit_ns(&n, |s| s.objects.retain(|o| &*o.name != "t3"));
    let k = |t: &str, i: u32| table_key(&n, t, Some(20_000 + i));
    let g = bounded(
        r.handle
            .check_state(vec![k("t0", 0), k("t1", 1), k("t3", 3)]),
    )
    .await
    .unwrap();
    assert_eq!(r.catalog.current().namespaces[&n].generation(), g);
    assert_eq!(tables_of(&r, &n), vec!["t0", "t1", "t2"]);
    let frags: Vec<Vec<hacivat_meta::ObjectKey>> = r
        .fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Fragment { keys, .. } => Some(keys),
            _ => None,
        })
        .collect();
    assert_eq!(frags.len(), 1, "{frags:?}");
    assert!(!frags[0].contains(&k("t0", 0)));
}

/// T10 carry: an identity rename (same OID, new name) of a top-level object schedules exactly
/// one follow-up read of its parent (the namespace), and does not loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hostile_rename_schedules_one_heal_and_no_loop() {
    let n = ns("public");
    let fake = FakeIntrospector::default();
    fake.put_ns(n.clone(), FakeIntrospector::tables(3));
    let r = rig_with(fake, RigOpts::default());
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    let before = passes(&r).len();
    r.fake.edit_ns(&n, |s| {
        let o = s.objects.iter_mut().find(|o| &*o.name == "t1").unwrap();
        o.name = hacivat_meta::Name::from("u1");
        o.version += 100;
    });
    bounded(
        r.handle
            .refresh(Target::Namespace(n.clone()), Trigger::User),
    )
    .await
    .unwrap();
    bounded(wait_until(Duration::from_secs(60), || {
        passes(&r).len() >= before + 2
    }))
    .await;
    idle(&r.handle).await;
    assert_eq!(tables_of(&r, &n), vec!["t0", "t2", "u1"]);
    assert_eq!(passes(&r).len(), before + 2, "{:?}", passes(&r));
}

// ---- fix round 1 verification (adopted in fix round 2: RR-1, RR-2, RR-3) ---------------------

mod rr1 {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use hacivat_introspect::IntroError;
    use hacivat_introspect::engine::EngineEvent;
    use hacivat_introspect::scope::NsInfo;

    use super::*;

    fn top_count(r: &Rig, n: &NsKey) -> u32 {
        r.catalog.current().namespaces[n].root().child_count()
    }

    /// RR-1: the user's "Remove from cache" re-plans a full pass; when a pass of the same
    /// namespace is already queued, the two coalesce, and the answer survives the coalescing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rr_confirm_decision_survives_coalescing_with_a_queued_pass() {
        let n = ns("public");
        let busy = ns("busy");
        let fake = FakeIntrospector::default();
        fake.put_ns(n.clone(), FakeIntrospector::tables(30));
        fake.put_ns(busy.clone(), FakeIntrospector::tables(3));
        let r = rig_with(
            fake,
            RigOpts {
                limits: EngineLimits {
                    workers: 1,
                    ..EngineLimits::default()
                },
                ..RigOpts::default()
            },
        );
        bounded(
            r.handle
                .refresh(Target::Namespace(n.clone()), Trigger::User),
        )
        .await
        .unwrap();
        r.fake.put_ns(n.clone(), FakeIntrospector::tables(5));
        bounded(
            r.handle
                .refresh(Target::Namespace(n.clone()), Trigger::Background),
        )
        .await
        .unwrap();
        assert_eq!(top_count(&r, &n), 30, "held");
        // A fragment moves the arena: the held merge no longer applies as is.
        bounded(
            r.handle
                .fragment(n.clone(), vec![table_key(&n, "t1", Some(20_001))]),
        )
        .await
        .unwrap();
        // Occupy the only worker, queue a background pass of `n`, then answer the prompt.
        let gate = r.fake.gate(
            move |c| matches!(c, FakeCall::Introspect { ns, .. } if *ns == pg_ns("db", "busy")),
        );
        let hold = r
            .handle
            .refresh(Target::Namespace(busy.clone()), Trigger::User);
        bounded(gate.entered()).await;
        let queued = r
            .handle
            .refresh(Target::Namespace(n.clone()), Trigger::Background);
        let confirm = r.handle.confirm_pending(n.clone(), true);
        bounded(r.handle.ready()).await.unwrap();
        gate.release();
        bounded(hold).await.unwrap();
        let _ = bounded(queued).await;
        let answered = bounded(confirm).await;
        idle(&r.handle).await;
        let tables = top_count(&r, &n);
        let stale = matches!(
            r.catalog
                .current()
                .state(&ContainerKey::Namespace(n.clone())),
            LoadState::Loaded { stale: true, .. }
        );
        // Fix round 2 (RR-1, flipped): the decision applied.
        assert!(answered.is_ok(), "{answered:?}");
        assert_eq!((tables, stale), (5, false));
    }

    /// RR-2: on a `*:*` scope, a database the login cannot list does not take the whole scope
    /// refresh (and so `on_connect` and the profile's own database) down with it; it is
    /// unknown: its schemas are neither in scope nor `Missing`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rr_one_unlistable_database_does_not_fail_the_scope_refresh() {
        let fake = FakeIntrospector::default();
        fake.put_ns(ns("public"), FakeIntrospector::tables(2));
        let info = |key: NsKey| NsInfo { key, system: false };
        fake.set_listing_for(
            None,
            vec![
                info(ns("public")),
                info(NsKey::database(name("db"))),
                info(NsKey::database(name("locked"))),
            ],
        );
        fake.set_listing_for(Some("locked"), vec![info(pg_ns("locked", "public"))]);
        // The second listing (the other database) is refused.
        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        fake.fail(
            move |c| matches!(c, FakeCall::ListNamespaces) && s.fetch_add(1, Ordering::SeqCst) == 1,
            IntroError::Permission {
                kind: hacivat_meta::ObjectKind::Schema,
            },
            1,
        );
        let r = rig_with(
            fake,
            RigOpts {
                scope: "*:*",
                current: pg_current("db", "public"),
                ..RigOpts::default()
            },
        );
        let mut events = r.handle.events();
        let got = bounded(r.handle.on_connect()).await;
        assert!(
            got.is_ok(),
            "on_connect failed because one other database could not be listed: {got:?}"
        );
        assert!(r.catalog.current().namespaces.contains_key(&ns("public")));
        let mut unlisted = false;
        while let Ok(ev) = events.try_recv() {
            unlisted |= ev == EngineEvent::DatabaseUnlisted { db: name("locked") };
        }
        assert!(unlisted);
        // Unknown, fail-closed: no schema list, no state, and an expansion reads nothing.
        let snap = r.catalog.current();
        assert_eq!(
            snap.state(&ContainerKey::schema_list(Some(&name("locked")))),
            &LoadState::NotLoaded
        );
        let locked = pg_ns("locked", "public");
        assert!(!snap.namespaces.contains_key(&locked));
        assert_eq!(
            bounded(r.handle.expand(locked)).await,
            Err(IntroError::Cancelled)
        );
    }

    /// RR-3: the scope refresh closes its first connection before it opens one to another
    /// database, so a budget of one connection is enough and never more is open at once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn rr_scope_refresh_holds_one_connection_at_a_time() {
        let other = pg_ns("other", "public");
        let fake = FakeIntrospector::default();
        fake.put_ns(ns("public"), FakeIntrospector::tables(2));
        fake.put_ns(other.clone(), FakeIntrospector::tables(2));
        let info = |key: NsKey| NsInfo { key, system: false };
        fake.set_listing_for(
            None,
            vec![
                info(ns("public")),
                info(NsKey::database(name("db"))),
                info(NsKey::database(name("other"))),
            ],
        );
        fake.set_listing_for(Some("other"), vec![info(other.clone())]);
        let r = rig_with(
            fake,
            RigOpts {
                scope: "*:*",
                current: pg_current("db", "public"),
                limits: EngineLimits {
                    workers: 1,
                    busy_retry: Duration::from_millis(5),
                    ..EngineLimits::default()
                },
                ..RigOpts::default()
            },
        );
        r.meta.budget.store(1, std::sync::atomic::Ordering::SeqCst);
        bounded(r.handle.on_connect()).await.unwrap();
        assert_eq!(r.meta.peak.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            r.catalog
                .current()
                .state(&ContainerKey::Namespace(other.clone())),
            &LoadState::NotLoaded
        );
    }
}
