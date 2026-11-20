// ## m4-t32-shots
//! M4 Task 32 (G3, moved from Task 30): the explorer offline. A cached catalog of a server
//! whose version Hacivat ships a system snapshot for shows its Built-in group with the
//! snapshot's version, and lookups of built-in objects resolve from the embedded snapshots,
//! with no connection and no network (the shell's drivers are fakes that are never dialled).

mod support;

use std::sync::Arc;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::explorer;
use hacivat_app::explorer::adapter;
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::name::{IdentCtx, NameClass, match_key};
use hacivat_meta::system::SystemCatalogs;
use hacivat_meta::view::{Lookup, SnapshotView};
use hacivat_meta::{CatalogSnapshot, ObjFlags};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::LiveState;
use hacivat_session::profile::Profile;
use hacivat_ui::tree::TreeRowData;
use sql_catalog::view::CatalogView;
use support::catalogs::*;
use support::*;

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn pg(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

/// A cached PostgreSQL 17.2 catalog of `app` (`public`, and `pg_catalog` read from the
/// shipped snapshot, flagged as such), with the embedded snapshot's layers, as the engine
/// opens one from the cache before any connect.
fn cached_pg17() -> Arc<CatalogSnapshot> {
    cached_pg17_with(true)
}

/// [`cached_pg17`]; `live_pg_catalog`: whether the cache holds a live read of `pg_catalog`
/// (which then answers for it, R-W5-6) or only `public`.
fn cached_pg17_with(live_pg_catalog: bool) -> Arc<CatalogSnapshot> {
    let system = SystemCatalogs::from_embedded();
    let overlays = system.overlays_for(DbmsKind::Postgres, "17.2");
    assert!(overlays.system.is_some(), "PostgreSQL 17 is shipped");
    let mut snap = (*catalog_gen(
        DbmsKind::Postgres,
        3,
        [
            schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![table("orders", Some(16_400), &[("id", "bigint")])],
            ),
            schema(
                DbmsKind::Postgres,
                &["app", "pg_catalog"],
                true,
                vec![table("pg_class", Some(1259), &[])],
            ),
        ]
        .into_iter()
        .take(if live_pg_catalog { 2 } else { 1 })
        .collect(),
        if live_pg_catalog {
            Vec::new()
        } else {
            // Not read live: the scope leaves the system schema to the snapshot.
            vec![(
                hacivat_meta::NsKey::schema(["app", "pg_catalog"]),
                hacivat_meta::LoadState::OutOfScope,
            )]
        },
    ))
    .clone();
    let mut facts = (*snap.server).clone();
    facts.version = Arc::from("17.2");
    snap.server = Arc::new(facts);
    snap.overlays = Arc::new(overlays);
    Arc::new(snap)
}

fn shell(cx: &mut TestAppContext) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("shop"));
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cached_pg17()), None, cx));
    vcx.run_until_parked();
    (vcx, id)
}

fn row(vcx: &VisualTestContext, label: &str) -> (hacivat_ui::tree::TreeKey, TreeRowData) {
    vcx.run_until_parked();
    vcx.cx
        .read(explorer::shown_rows)
        .into_iter()
        .find(|r| r.status.is_none() && strip(&r.data.label) == label)
        .map(|r| (r.key, r.data))
        .unwrap_or_else(|| panic!("`{label}` is shown"))
}

fn expand(vcx: &VisualTestContext, label: &str) {
    let (key, _) = row(vcx, label);
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, true, cx));
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn offline_explorer_shows_the_built_in_group_with_its_version_badge(cx: &mut TestAppContext) {
    let (vcx, _id) = shell(cx);
    expand(vcx, "shop");
    expand(vcx, "app");
    let (_, builtin) = row(vcx, "Built-in");
    assert_eq!(
        builtin.badge.as_deref().map(strip).as_deref(),
        Some("version 17")
    );
    let tip = strip(builtin.tooltip.as_deref().expect("a tooltip"));
    assert!(tip.contains("version 17"), "{tip}");
    assert!(
        builtin
            .a11y_description
            .as_deref()
            .is_some_and(|d| strip(d).contains("version 17")),
        "{:?}",
        builtin.a11y_description
    );
    // Its schemas are there, without a connection.
    expand(vcx, "Built-in");
    row(vcx, "pg_catalog");
    assert_eq!(live(vcx, "shop"), LiveState::Disconnected);
    // In Turkish too.
    switch_to(vcx, "tr");
    let label = vcx
        .cx
        .read(explorer::shown_rows)
        .into_iter()
        .find(|r| {
            r.data.badge.is_some() && r.data.icon == Some(hacivat_ui::icon::IconName::Library)
        })
        .map(|r| strip(r.data.badge.as_deref().unwrap_or_default()))
        .expect("the Built-in row");
    assert!(label.contains("17"), "{label}");
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn offline_lookup_resolves_built_ins_without_a_connection(cx: &mut TestAppContext) {
    let (vcx, id) = shell(cx);
    // A cache with no live read of `pg_catalog`: the embedded snapshot answers for it.
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cached_pg17_with(false)), None, cx));
    vcx.run_until_parked();
    let cat = vcx
        .cx
        .read(|cx| adapter::catalog_of(id, cx))
        .expect("the cached catalog");
    // The database and search path the user's session would report (none here: no session;
    // the cached profile's database).
    let view = SnapshotView::new(cat.clone())
        .with_layers(cat.overlays.clone())
        .with_search_path(hacivat_meta::search_path::SearchPath::for_pg(
            &Arc::from("app"),
            &[Arc::from("public")],
            None,
            None,
        ));
    let ctx = IdentCtx::new(hacivat_meta::name::DialectId::Postgres);
    let key =
        |text: &str, class| match_key(ctx.dialect, &ctx, text, false, class).expect("a pinned key");
    let found = view.relation(
        &key("pg_catalog", NameClass::Schema),
        &key("pg_stat_activity", NameClass::Table),
    );
    let Lookup::Found(r) = found else {
        panic!("pg_catalog.pg_stat_activity resolves from the embedded snapshot: {found:?}");
    };
    assert_eq!(&*r.name.name, "pg_stat_activity");
    // The cached user table resolves too; the snapshot's objects are flagged as shipped.
    assert!(matches!(
        view.relation(
            &key("public", NameClass::Schema),
            &key("orders", NameClass::Table)
        ),
        Lookup::Found(_)
    ));
    let system = cat.overlays.system.as_ref().expect("the snapshot layer");
    assert_eq!(&*system.line, "17");
    assert!(
        system
            .namespaces
            .values()
            .all(|ns| ns.root().flags().contains(ObjFlags::SYSTEM)
                || ns.root().flags().contains(ObjFlags::PRE_INTROSPECTED)),
    );
    // Nothing connected (no driver call, no session).
    assert_eq!(live(vcx, "shop"), LiveState::Disconnected);
    stop_core_runtime_test(cx);
}
