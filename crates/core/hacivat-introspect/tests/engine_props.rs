//! Random interleavings of engine requests, failures and cancels never publish a partial
//! namespace (M4 Task 15; alg/02 HV-INTRO-01 "atomic publish per namespace").
//!
//! Every namespace of the fake server holds a fixed set of tables of two columns each; only
//! their versions change. Whatever the order of requests, injected failures (timeouts, lost
//! connections, catalog races) and cancels, every generation a subscriber sees, and the last
//! one, satisfies:
//! - a namespace whose state is `Loaded` holds exactly the server's tables (never some);
//! - every table holds none of its columns (names level) or all of them, never some.

#![cfg(feature = "session")]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use hacivat_introspect::IntroError;
use hacivat_introspect::ddl_effects::Target;
use hacivat_introspect::engine::EngineLimits;
use hacivat_introspect::pack::Mode;
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::merge::Trigger;
use hacivat_meta::snapshot::{CatalogSnapshot, ContainerKey, LoadState};
use hacivat_meta::{Level, NsKey, ObjectKind};
use hacivat_session::meta::MetaError;
use proptest::prelude::*;

const NAMESPACES: usize = 3;
const TABLES: usize = 6;

#[derive(Debug, Clone)]
enum Op {
    Expand(usize),
    Children(usize, usize),
    Refresh(usize, bool),
    Fragment(usize, usize),
    Elevate(usize, usize),
    Cancel(usize),
    Fail(usize, u8),
    Touch(usize, usize),
}

fn engine_ops() -> impl Strategy<Value = Vec<Op>> {
    let ns = 0..NAMESPACES;
    let t = 0..TABLES;
    let op = prop_oneof![
        ns.clone().prop_map(Op::Expand),
        (ns.clone(), t.clone()).prop_map(|(n, t)| Op::Children(n, t)),
        (ns.clone(), any::<bool>()).prop_map(|(n, u)| Op::Refresh(n, u)),
        (ns.clone(), t.clone()).prop_map(|(n, t)| Op::Fragment(n, t)),
        (ns.clone(), t.clone()).prop_map(|(n, t)| Op::Elevate(n, t)),
        ns.clone().prop_map(Op::Cancel),
        (ns.clone(), 0u8..3).prop_map(|(n, k)| Op::Fail(n, k)),
        (ns, t).prop_map(|(n, t)| Op::Touch(n, t)),
    ];
    prop::collection::vec(op, 1..16)
}

fn key(n: &NsKey, t: usize) -> hacivat_meta::ObjectKey {
    table_key(n, &format!("t{t}"), Some(30_000 + t as u32))
}

/// The invariants of one generation (see the module docs).
fn check(snap: &CatalogSnapshot, spaces: &[NsKey]) -> Result<(), String> {
    let want: BTreeSet<String> = (0..TABLES).map(|t| format!("t{t}")).collect();
    for n in spaces {
        let Some(arena) = snap.namespaces.get(n) else {
            continue;
        };
        let mut seen = BTreeSet::new();
        for (i, o) in arena.children(0) {
            if o.kind() != ObjectKind::Table {
                continue;
            }
            let cols = arena.children(i).count();
            if cols != 0 && cols != 2 {
                return Err(format!(
                    "gen {}: {n:?}.{} has {cols} of 2 columns",
                    snap.generation,
                    o.name()
                ));
            }
            seen.insert(o.name().to_string());
        }
        if matches!(
            snap.state(&ContainerKey::Namespace(n.clone())),
            LoadState::Loaded { .. }
        ) && seen != want
        {
            return Err(format!(
                "gen {}: {n:?} is Loaded with {seen:?}",
                snap.generation
            ));
        }
    }
    Ok(())
}

fn run(ops: Vec<Op>) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime");
    rt.block_on(async move {
        let spaces: Vec<NsKey> = (0..NAMESPACES)
            .map(|i| pg_ns("db", &format!("s{i}")))
            .collect();
        let fake = FakeIntrospector::default();
        for n in &spaces {
            fake.put_ns(
                n.clone(),
                (0..TABLES)
                    .map(|t| {
                        FakeObject::table(&format!("t{t}"), &["a", "b"]).with_oid(30_000 + t as u32)
                    })
                    .collect(),
            );
        }
        let r = rig_with(
            fake,
            RigOpts {
                limits: EngineLimits {
                    workers: 3,
                    ..EngineLimits::default()
                },
                ..RigOpts::default()
            },
        );
        // A watcher checks every generation it hears of.
        let mut events = r.catalog.subscribe();
        let catalog = r.catalog.clone();
        let watched = spaces.clone();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let watcher = tokio::spawn(async move {
            let mut first_error = None;
            loop {
                // Events first: every publish before the stop is checked.
                tokio::select! {
                    biased;
                    ev = events.recv() => match ev {
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            if let Err(e) = check(&catalog.current(), &watched) {
                                first_error.get_or_insert(e);
                            }
                        }
                        Err(_) => return first_error,
                    },
                    _ = &mut stop_rx => return first_error,
                }
            }
        });
        let mut tickets = Vec::new();
        for op in ops {
            match op {
                Op::Expand(n) => tickets.push(r.handle.expand(spaces[n].clone())),
                Op::Children(n, t) => tickets.push(r.handle.load_children(key(&spaces[n], t))),
                Op::Refresh(n, user) => tickets.push(r.handle.refresh(
                    Target::Namespace(spaces[n].clone()),
                    if user {
                        Trigger::User
                    } else {
                        Trigger::Background
                    },
                )),
                Op::Fragment(n, t) => tickets.push(
                    r.handle
                        .fragment(spaces[n].clone(), vec![key(&spaces[n], t)]),
                ),
                Op::Elevate(n, t) => {
                    tickets.push(r.handle.elevate(key(&spaces[n], t), Level::Sources))
                }
                Op::Cancel(n) => r.handle.cancel(spaces[n].clone()),
                Op::Fail(n, kind) => {
                    let error = match kind {
                        0 => IntroError::Timeout,
                        1 => IntroError::Meta(MetaError::ConnectionLost),
                        _ => IntroError::CatalogRace,
                    };
                    let target = spaces[n].clone();
                    r.fake.fail(
                        move |c| match c {
                            FakeCall::Introspect { ns, mode, .. } => {
                                *ns == target && *mode != Mode::Brief
                            }
                            FakeCall::Fragment { ns, .. } => *ns == target,
                            _ => false,
                        },
                        error,
                        2,
                    );
                }
                Op::Touch(n, t) => r.fake.touch(&spaces[n], &format!("t{t}")),
            }
        }
        for t in tickets {
            // Every ticket resolves (success, failure or cancel): none hangs.
            let _ = bounded(t).await;
        }
        idle(&r.handle).await;
        check(&r.catalog.current(), &spaces)?;
        r.handle.shutdown();
        bounded(r.handle.closed()).await;
        drop(r);
        let _ = stop_tx.send(());
        match bounded(watcher).await {
            Ok(Some(e)) => Err(e),
            Ok(None) => Ok(()),
            Err(e) => Err(format!("watcher: {e}")),
        }
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    #[test]
    fn random_request_interleavings_never_publish_partial_state(ops in engine_ops()) {
        let out = run(ops);
        prop_assert!(out.is_ok(), "{}", out.unwrap_err());
    }
}

/// The checker itself catches a namespace published with part of its objects.
#[test]
fn the_checker_rejects_a_partial_namespace() {
    use hacivat_meta::{Catalog, ServerFacts};
    let n = pg_ns("db", "s0");
    let catalog = Catalog::new(Arc::new(ServerFacts::new(
        hacivat_proto::DbmsKind::Postgres,
        hacivat_meta::name::IdentCtx::new(sql_dialect::DialectId::Postgres),
    )));
    let arena = hacivat_meta::fixture::ns_pg("db", "s0")
        .table("t0", &["a", "b"])
        .finish();
    catalog
        .publisher()
        .unwrap()
        .publish(vec![
            hacivat_meta::snapshot::CatalogChange::Put(Arc::new(arena)),
            hacivat_meta::snapshot::CatalogChange::SetState(
                ContainerKey::Namespace(n.clone()),
                LoadState::Loaded {
                    level: Level::Structure,
                    at: std::time::SystemTime::UNIX_EPOCH,
                    watermark: Default::default(),
                    stale: false,
                },
            ),
        ])
        .unwrap();
    assert!(check(&catalog.current(), &[n]).is_err());
}
