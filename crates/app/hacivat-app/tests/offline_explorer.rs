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
    cached_pg17_with(PgCatalog::Snapshot)
}

/// What the cache holds of `pg_catalog`.
#[derive(Clone, Copy, PartialEq)]
enum PgCatalog {
    /// The shipped snapshot's (flagged `PRE_INTROSPECTED`): the snapshot answers for it.
    Snapshot,
    /// A live read (R-W5-6: the live catalog answers for it).
    Live,
    /// Nothing: the scope leaves it out (lookups fall to the snapshot's layer).
    None,
    // ## m4-t33b-carries (T32 re-review RR-3)
    /// A live read, beside an `information_schema` the shipped snapshot answers for.
    Mixed,
}

/// `pg_catalog` as the shipped snapshot puts it in a catalog: a system schema flagged
/// `PRE_INTROSPECTED`.
fn shipped_pg_catalog() -> Arc<hacivat_meta::NamespaceSnapshot> {
    shipped_schema("pg_catalog", "pg_class", 1259)
}

/// System schema `name` as the shipped snapshot puts it in a catalog, holding `table`.
fn shipped_schema(name: &str, table_name: &str, oid: u32) -> Arc<hacivat_meta::NamespaceSnapshot> {
    use hacivat_meta::name::{DialectId, FoldRule};
    use hacivat_meta::namespace::{NsMeta, Subtree};
    use hacivat_meta::{Level, Obj, ObjectKind};
    let root =
        Obj::new(ObjectKind::Schema, name).add_flags(ObjFlags::SYSTEM | ObjFlags::PRE_INTROSPECTED);
    let mut tree = Subtree::new(root);
    let r = tree.root();
    tree.graft_under(r, table(table_name, Some(oid), &[]));
    let fold = FoldRule::for_names(
        DialectId::Postgres,
        &IdentCtx::new(DialectId::Postgres),
        NameClass::Table,
    )
    .unwrap_or(FoldRule::Exact);
    Arc::new(
        hacivat_meta::NamespaceSnapshot::build(
            hacivat_meta::NsKey::schema(["app", name]),
            DbmsKind::Postgres,
            fold,
            NsMeta {
                level: Level::Structure,
                ..NsMeta::default()
            },
            tree,
        )
        .expect("a valid namespace"),
    )
}

/// [`cached_pg17`] with `pg` as the cache's `pg_catalog`.
fn cached_pg17_with(pg: PgCatalog) -> Arc<CatalogSnapshot> {
    let live_pg_catalog = pg != PgCatalog::None;
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
            if pg == PgCatalog::Snapshot {
                shipped_pg_catalog()
            } else {
                schema(
                    DbmsKind::Postgres,
                    &["app", "pg_catalog"],
                    true,
                    vec![table("pg_class", Some(1259), &[])],
                )
            },
            shipped_schema("information_schema", "tables", 13_500),
        ]
        .into_iter()
        .take(match pg {
            PgCatalog::None => 1,
            PgCatalog::Mixed => 3,
            _ => 2,
        })
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

// ## m4-t32-shots (fix round 1, review M-6): a Built-in group whose schemas a live read
// answered for carries no snapshot badge (the tooltip says "shipped with Hacivat").
#[gpui::test]
fn a_live_read_built_in_group_carries_no_snapshot_badge(cx: &mut TestAppContext) {
    let (vcx, id) = shell(cx);
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cached_pg17_with(PgCatalog::Live)), None, cx));
    expand(vcx, "shop");
    expand(vcx, "app");
    let (_, builtin) = row(vcx, "Built-in");
    assert_eq!(builtin.badge, None);
    assert_eq!(builtin.tooltip, None);
    stop_core_runtime_test(cx);
}

// ## m4-t33b-carries (T32 re-review RR-3): a mixed group (`pg_catalog` read live,
// `information_schema` from the shipped snapshot) keeps its badge, and its tooltip names the
// schemas the snapshot answers for instead of claiming the whole group was shipped.
#[gpui::test]
fn a_mixed_built_in_group_names_the_schemas_the_snapshot_serves(cx: &mut TestAppContext) {
    let (vcx, id) = shell(cx);
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cached_pg17_with(PgCatalog::Mixed)), None, cx));
    expand(vcx, "shop");
    expand(vcx, "app");
    let (_, builtin) = row(vcx, "Built-in");
    assert_eq!(
        builtin.badge.as_deref().map(strip).as_deref(),
        Some("version 17")
    );
    assert_eq!(
        strip(builtin.tooltip.as_deref().expect("a tooltip")),
        "Built-in catalog (version 17) for information_schema: shipped with Hacivat and read \
         without a connection. The other built-in schemas were read from the server."
    );
    expand(vcx, "Built-in");
    row(vcx, "pg_catalog");
    row(vcx, "information_schema");
    switch_to(vcx, "tr");
    let (_, builtin) = row(vcx, "Yerleşik");
    let tip = strip(
        builtin
            .tooltip
            .as_deref()
            .expect("the Built-in row's tooltip"),
    );
    assert!(
        tip.contains("information_schema") && tip.contains("sunucudan"),
        "{tip}"
    );
    stop_core_runtime_test(cx);
}

#[gpui::test]
fn offline_lookup_resolves_built_ins_without_a_connection(cx: &mut TestAppContext) {
    let (vcx, id) = shell(cx);
    // A cache with no live read of `pg_catalog`: the embedded snapshot answers for it.
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(cached_pg17_with(PgCatalog::None)), None, cx));
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

// ## m4-t33b-carries (T32 review T-1): offline, through the app's own paths. The explorer's
// reveal (the "Show in explorer" seam) walks the Built-in group down to the shipped
// snapshot's `pg_catalog.pg_class` and selects it (before, the walk stopped at the Built-in
// group: `Pending`), its DDL view opens, and the fake driver was never dialled.
#[gpui::test]
fn offline_built_ins_are_revealed_and_shown_through_the_app_with_no_connect(
    cx: &mut TestAppContext,
) {
    let (vcx, id) = shell(cx);
    let key = hacivat_meta::ObjectKey {
        kind: hacivat_meta::ObjectKind::Table,
        path: ["app", "pg_catalog", "pg_class"]
            .into_iter()
            .map(hacivat_meta::Name::from)
            .collect(),
        server_id: Some(hacivat_meta::ServerId::Oid(1259)),
        overload_sig: None,
        raw_kind: None,
    };
    let outcome = in_window(vcx, |window, cx| {
        hacivat_app::explorer::live::reveal(id, &key, window, cx)
    });
    assert_eq!(
        outcome,
        hacivat_app::explorer::live::RevealOutcome::Revealed
    );
    vcx.run_until_parked();
    let target = adapter::object_tree_key(id, &key);
    let selected = vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .selection()
            .contains(&target)
    });
    assert!(selected, "the revealed row is selected");
    row(vcx, "pg_class");
    // Its DDL view opens (offline it can only say what is not loaded) and dials nothing.
    let ws = workspace(vcx);
    let opened = in_window(vcx, |window, cx| {
        hacivat_app::ddl_view::open_ddl_view(&ws, id, key.clone(), window, cx)
    });
    assert!(opened.is_ok(), "{opened:?}");
    vcx.run_until_parked();
    assert!(
        vcx.cx
            .read(|cx| hacivat_app::ddl_view::state_of(id, &key, cx))
            .is_some(),
        "the DDL view is open"
    );
    // No connection, no dial.
    assert_eq!(live(vcx, "shop"), LiveState::Disconnected);
    assert_eq!(fake(vcx).connect_calls(), 0, "the driver was never dialled");
    stop_core_runtime_test(cx);
}
