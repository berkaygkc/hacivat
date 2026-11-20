// ## m4-t33a-hygiene (a new test file of Task 33a in this crate)
//! The public `FakeIntrospector` reads facets and definition texts (T33 carry for T25 m-7):
//! the engine-driven facet-vouch test of the app's DDL view drives the real engine over this
//! fake, so the fake answers `facet` from scripted rows (only the asked objects, only the
//! scripted kinds, which are also its `IntroCaps::facets`) and hashes a view's definition at
//! `Sources` through `engine::facet::source_ref`, as the real introspectors do.

#![cfg(feature = "session")]

mod common;

use std::time::Duration;

use common::*;
use hacivat_introspect::engine::LevelPolicy;
use hacivat_introspect::introspector::{FacetRows, Introspector};
use hacivat_introspect::testing::FakeIntrospector;
use hacivat_introspect::testing::fake::{FakeCall, FakeObject};
use hacivat_meta::props::{FacetKind, Grant, GrantsFacet, Props};
use hacivat_meta::snapshot::{ContainerKey, FacetValue, LoadState};
use hacivat_meta::{Level, NsKey, ObjectKind};

fn ns() -> NsKey {
    pg_ns("db", "public")
}

fn grants(grantee: &str) -> GrantsFacet {
    GrantsFacet {
        entries: vec![Grant {
            grantee: name(grantee),
            privilege: name("SELECT"),
            columns: Vec::new(),
            grantable: false,
        }],
    }
}

async fn wait_level(r: &Rig, level: Level) {
    let catalog = r.catalog.clone();
    bounded(wait_until(Duration::from_secs(60), move || {
        matches!(
            catalog.current().state(&ContainerKey::Namespace(ns())),
            LoadState::Loaded { level: l, .. } if *l >= level
        )
    }))
    .await;
    idle(&r.handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_fake_answers_scripted_facets_for_the_asked_objects_only() {
    let fake = FakeIntrospector::default();
    fake.put_ns(
        ns(),
        vec![
            FakeObject::table("t", &["a"]).with_oid(100),
            FakeObject::table("u", &["a"]).with_oid(101),
        ],
    );
    let (t, u) = (
        table_key(&ns(), "t", Some(100)),
        table_key(&ns(), "u", Some(101)),
    );
    assert!(fake.capabilities().facets.is_empty(), "nothing scripted");
    fake.put_facet(FacetRows::Grants(vec![
        (t.clone(), grants("reporting")),
        (u.clone(), grants("audit")),
    ]));
    assert_eq!(fake.capabilities().facets, vec![FacetKind::Grants]);
    let r = rig_with(fake, RigOpts::default());
    bounded(r.handle.expand(ns())).await.unwrap();
    wait_level(&r, Level::Structure).await;
    bounded(r.handle.load_facet(vec![t.clone()], FacetKind::Grants))
        .await
        .unwrap();
    let snap = r.catalog.current();
    let (state, value) = snap.facet(&t, FacetKind::Grants);
    assert!(matches!(state, LoadState::Loaded { stale: false, .. }), "{state:?}");
    let Some(FacetValue::Grants(g)) = value else {
        panic!("grants: {value:?}");
    };
    assert_eq!(**g, grants("reporting"));
    // Only `t` was asked, so only `t` was read and published.
    assert_eq!(snap.facet(&u, FacetKind::Grants).0, LoadState::NotLoaded);
    let asked: Vec<_> = r
        .fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Facet { keys, kind, .. } => Some((keys, kind)),
            _ => None,
        })
        .collect();
    assert_eq!(asked, vec![(vec![t.clone()], FacetKind::Grants)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_view_definition_is_hashed_and_published_at_sources() {
    let fake = FakeIntrospector::default();
    fake.put_ns(
        ns(),
        vec![FakeObject::view("v", &["one"]).with_definition("SELECT 1 AS one")],
    );
    let mut levels = LevelPolicy::default();
    levels.set_override(ContainerKey::Namespace(ns()), Some(Level::Sources));
    let r = rig_with(
        fake,
        RigOpts {
            levels,
            ..RigOpts::default()
        },
    );
    bounded(r.handle.expand(ns())).await.unwrap();
    wait_level(&r, Level::Sources).await;
    let snap = r.catalog.current();
    let arena = &snap.namespaces[&ns()];
    let (_, v) = arena
        .children(0)
        .find(|(_, o)| o.kind() == ObjectKind::View)
        .expect("the view");
    let Props::View(p) = v.props() else {
        panic!("view props: {:?}", v.props());
    };
    let source = p.source.expect("a source reference at Sources");
    assert_eq!(snap.source_text(&source).as_deref(), Some("SELECT 1 AS one"));
}
// ## m4-t33a-hygiene
