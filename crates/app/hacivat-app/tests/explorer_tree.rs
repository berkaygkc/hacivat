//! M4 Task 14: the explorer tree over catalog snapshots (design/m4-explorer surfaces 1–5, 12;
//! R-M4-18, R-M4-19, R-M4-49, R-M4-50, D-074).

mod support;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer::{self, ShownRow};
use hacivat_app::explorer::adapter::{self, NodeInfo};
use hacivat_app::explorer::contrib::{
    ContributedChildren, ContributedNode, ExplorerContributor, register_contributor,
};
use hacivat_app::explorer::rows::DisplayGroup;
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::delta::{CatalogDelta, Confidence};
use hacivat_meta::{CatalogSnapshot, LoadState, NsKey, ObjectKey, ObjectKind, ServerId};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::icon::IconName;
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, MetaMark, RowFlag, TreeKey, TreeModel, TreeRowData};
use support::catalogs::*;
use support::*;

// ---- helpers ----------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// A connections shell in `lang` with `profiles` saved.
fn shell_with(
    cx: &mut TestAppContext,
    lang: &str,
    profiles: Vec<Profile>,
) -> &'static mut VisualTestContext {
    let vcx = shell_with_fake(cx, lang, answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    for p in profiles {
        add_profile(vcx, p);
    }
    vcx
}

fn pg(name: &str, database: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some(database.to_string());
    p
}

fn mysql(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.engine = DbmsKind::MySql;
    p
}

fn sqlite(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Local), "127.0.0.1");
    p.engine = DbmsKind::Sqlite;
    p.endpoint = ProfileEndpoint::File {
        path: "/data/app.db".into(),
        create_if_missing: false,
    };
    p
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Option<Arc<CatalogSnapshot>>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, cat, None, cx));
    vcx.run_until_parked();
}

fn rows(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(explorer::shown_rows)
}

/// The shown rows as `indent label [meta] (type) «badge»`, isolation marks stripped.
fn lines(vcx: &VisualTestContext) -> Vec<String> {
    rows(vcx)
        .into_iter()
        .map(|r| {
            let mut s = format!("{}{}", "  ".repeat(r.depth as usize), strip(&r.data.label));
            if let Some(m) = &r.data.meta {
                s.push_str(&format!(" [{}]", strip(m)));
            }
            if let Some(t) = &r.data.type_text {
                s.push_str(&format!(" ({})", strip(t)));
            }
            if let Some(b) = &r.data.badge {
                s.push_str(&format!(" «{b}»"));
            }
            s
        })
        .collect()
}

fn model(vcx: &VisualTestContext) -> hacivat_app::explorer::ExplorerModel {
    vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .model()
            .clone()
    })
}

/// The key of the shown row labelled `label` (isolation marks stripped), depth-first.
fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown in {:#?}", lines(vcx)))
        .key
}

fn set_expanded(vcx: &VisualTestContext, key: TreeKey, on: bool) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, on, cx));
    });
    vcx.run_until_parked();
}

fn is_expanded(vcx: &VisualTestContext, key: TreeKey) -> bool {
    vcx.cx.read(|cx| {
        explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .is_expanded(key)
    })
}

/// Expands the shown rows labelled `labels`, in order.
fn open(vcx: &VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key_of(vcx, l);
        set_expanded(vcx, k, true);
    }
}

fn row_of(vcx: &VisualTestContext, label: &str) -> TreeRowData {
    rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown"))
        .data
}

fn filter(vcx: &mut VisualTestContext, text: &str) {
    in_window(vcx, explorer::focus_filter);
    vcx.run_until_parked();
    vcx.simulate_input(text);
    vcx.run_until_parked();
}

/// Erases `typed` from the filter with Backspace.
fn clear_filter(vcx: &mut VisualTestContext, typed: &str) {
    in_window(vcx, explorer::focus_filter);
    vcx.run_until_parked();
    for _ in typed.chars() {
        vcx.simulate_keystrokes("backspace");
    }
    vcx.run_until_parked();
}

fn click_tolerant(vcx: &mut VisualTestContext) {
    click(vcx, "explorer-tolerant");
}

/// The PostgreSQL shop: `app.public` with two tables, a view and a function.
fn pg_shop() -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::Postgres,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![
                table(
                    "orders",
                    Some(16_400),
                    &[("id", "bigint"), ("total", "numeric(12,2)")],
                ),
                table("users", Some(16_410), &[("id", "integer")]),
                leaf(ObjectKind::View, "v_open"),
                leaf(ObjectKind::Routine, "f_total"),
            ],
        )],
    )
}

// ---- shape ------------------------------------------------------------------------------------

#[gpui::test]
fn pg_tree_shape_database_schema_groups_objects(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(vcx, id, Some(pg_shop()));
    open(
        vcx,
        &["shop", "app", "public", "TABLES", "orders", "COLUMNS"],
    );
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    public",
            "      TABLES [2]",
            "        orders",
            "          COLUMNS [2]",
            "            id (bigint)",
            "            total (numeric(12,2))",
            "        users",
            "      VIEWS [1]",
            "      FUNCTIONS [1]",
        ]
    );
    // Kind icons per kind; group headers draw none (R-M4-49 headers, DESIGN.md Explorer Tree).
    assert_eq!(row_of(vcx, "app").icon, Some(IconName::Cylinder));
    assert_eq!(row_of(vcx, "public").icon, Some(IconName::FolderTree));
    assert_eq!(row_of(vcx, "orders").icon, Some(IconName::Table2));
    assert_eq!(row_of(vcx, "id").icon, Some(IconName::Columns3));
    let header = row_of(vcx, "TABLES");
    assert!(header.group_header && header.icon.is_none());
    assert_eq!(row_of(vcx, "orders").label_source, TextSource::Catalog);
    // The group's key is its own: derived from its owner, never a display string.
    let m = model(vcx);
    assert_eq!(
        m.info(key_of(vcx, "TABLES")),
        Some(NodeInfo::Group {
            profile: id,
            group: DisplayGroup::Tables
        })
    );
}

#[gpui::test]
fn no_catalog_and_generation_zero_show_not_loaded(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    open(vcx, &["shop"]);
    assert_eq!(lines(vcx), ["DEV [1]", "shop [PostgreSQL]", "  Not loaded"]);
    // Generation 0 (a catalog with nothing read, T1c m-B5): "Not loaded", never empty.
    show(
        vcx,
        id,
        Some(catalog_gen(DbmsKind::Postgres, 0, Vec::new(), Vec::new())),
    );
    assert_eq!(lines(vcx), ["DEV [1]", "shop [PostgreSQL]", "  Not loaded"]);
    // A database listed with nothing read: its row says so too.
    show(
        vcx,
        id,
        Some(catalog_gen(DbmsKind::Postgres, 3, Vec::new(), Vec::new())),
    );
    open(vcx, &["app"]);
    assert_eq!(
        lines(vcx),
        ["DEV [1]", "shop [PostgreSQL]", "  app", "    Not loaded"]
    );
    // A namespace whose read failed shows the failure with Retry; one read at Names shows a
    // table's columns as not loaded, never as none.
    let failed = LoadState::Failed {
        at: std::time::SystemTime::UNIX_EPOCH,
        error: hacivat_meta::snapshot::IntroErrorCode::PermissionDenied,
        retry_after: std::time::Duration::ZERO,
    };
    show(
        vcx,
        id,
        Some(catalog_gen(
            DbmsKind::Postgres,
            4,
            vec![schema_at(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                hacivat_meta::Level::Names,
                vec![table("orders", Some(16_400), &[])],
            )],
            vec![(NsKey::schema(["app", "audit"]), failed)],
        )),
    );
    open(vcx, &["audit", "public", "TABLES", "orders"]);
    let shown = lines(vcx);
    assert!(
        shown.contains(&"      Couldn't read audit".to_string()),
        "{shown:#?}"
    );
    assert!(
        shown.contains(&"          Not loaded".to_string()),
        "{shown:#?}"
    );
}

#[gpui::test]
fn mysql_schemas_are_labelled_databases(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![mysql("billing")]);
    let id = profile_id(vcx, "billing");
    show(
        vcx,
        id,
        Some(catalog_gen(
            DbmsKind::MySql,
            1,
            vec![
                schema(
                    DbmsKind::MySql,
                    &["shop"],
                    false,
                    vec![table("orders", None, &[("id", "int")])],
                ),
                schema(
                    DbmsKind::MySql,
                    &["mysql"],
                    true,
                    vec![table("user", None, &[])],
                ),
            ],
            vec![(NsKey::schema(["crm"]), LoadState::NotLoaded)],
        )),
    );
    open(vcx, &["billing", "crm", "shop"]);
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "billing [MySQL]",
            "  crm",
            "    Loads when you open it",
            "  shop",
            "    TABLES [1]",
            "  Built-in",
        ]
    );
    // No schema level: each one is a database (`model-kind-schema-mysql`).
    for name in ["crm", "shop"] {
        let d = row_of(vcx, name).a11y_description.expect("a description");
        assert!(d.contains("database"), "{name}: {d}");
        assert!(!d.contains("schema"), "{name}: {d}");
    }
}

#[gpui::test]
fn sqlite_shows_main_temp_and_attached(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![sqlite("app.db")]);
    let id = profile_id(vcx, "app.db");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Sqlite,
            vec![
                schema(DbmsKind::Sqlite, &["aux"], false, vec![]),
                schema(DbmsKind::Sqlite, &["temp"], false, vec![]),
                schema(
                    DbmsKind::Sqlite,
                    &["main"],
                    false,
                    vec![table("notes", None, &[])],
                ),
            ],
        )),
    );
    open(vcx, &["app.db"]);
    assert_eq!(
        lines(vcx),
        ["LOCAL [1]", "app.db [SQLite]", "  main", "  temp", "  aux"]
    );
    // SQLite never uses a snapshot: no Built-in group (R-M4-6).
    assert!(!lines(vcx).iter().any(|l| l.contains("Built-in")));
}

#[gpui::test]
fn kind_group_counts_follow_the_format_locale(cx: &mut TestAppContext) {
    // The count follows the regional format (L3, L15(d)); the label the UI language.
    for (lang, region, header) in [
        ("en", "en-US", "      TABLES [1,204]"),
        ("en", "tr-TR", "      TABLES [1.204]"),
        ("tr", "tr-TR", "      TABLOLAR [1.204]"),
    ] {
        let vcx = shell_with_fake_prepared(
            cx,
            lang,
            answers_probe(FakeDriver::new(DbmsKind::Postgres)),
            |paths| {
                write(
                    paths.settings_toml(),
                    &format!("[ui]\nregional_format = \"{region}\"\n"),
                )
            },
        );
        add_profile(vcx, pg("shop", "app"));
        let id = profile_id(vcx, "shop");
        let ns = wide_pg_schema("app", "public", 1_204, 1);
        show(vcx, id, Some(catalog(DbmsKind::Postgres, vec![ns])));
        open(vcx, &["shop", "app", "public"]);
        let shown = lines(vcx);
        assert_eq!(
            shown.get(4).map(String::as_str),
            Some(header),
            "{lang} {region}: {shown:#?}"
        );
        close_all_windows(vcx);
    }
}

#[gpui::test]
fn names_are_verbatim_never_case_mapped(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "tr", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![
                    table("İSTANBUL_ŞUBE", Some(1), &[]),
                    table("ıspanak", Some(2), &[]),
                    table("users", Some(3), &[]),
                    table("Users", Some(4), &[]),
                ],
            )],
        )),
    );
    open(vcx, &["shop", "app", "public", "TABLOLAR"]);
    let names: Vec<String> = lines(vcx)
        .into_iter()
        .skip(5)
        .map(|l| l.trim().to_string())
        .collect();
    // Verbatim (never case-mapped); `users` and `Users` are two rows; Turkish collation for
    // display only (`ı` before `İ`, lower case before upper).
    assert_eq!(names, ["ıspanak", "İSTANBUL_ŞUBE", "users", "Users"]);
    // Each label is the stored name between FSI and PDI (bidi isolation).
    assert_eq!(
        row_of(vcx, "İSTANBUL_ŞUBE").label.as_ref(),
        "\u{2068}İSTANBUL_ŞUBE\u{2069}"
    );
    // The drawn text is the same.
    enable_registry(vcx);
    let drawn: Vec<String> = registry_frame(vcx)
        .into_iter()
        .filter(|r| r.path.ends_with("#hv-tree-label"))
        .map(|r| r.text.to_string())
        .collect();
    assert!(drawn.contains(&"İSTANBUL_ŞUBE".to_string()), "{drawn:?}");
    assert!(drawn.contains(&"ıspanak".to_string()), "{drawn:?}");
}

// ---- expansion --------------------------------------------------------------------------------

#[gpui::test]
fn expansion_survives_publish_and_rename_by_oid(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(vcx, id, Some(pg_shop()));
    open(vcx, &["shop", "app", "public", "TABLES", "orders"]);
    let before = key_of(vcx, "orders");
    // The next generation renames `orders` (same OID) and adds a column.
    let renamed = catalog_gen(
        DbmsKind::Postgres,
        2,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![
                table(
                    "orders_v2",
                    Some(16_400),
                    &[
                        ("id", "bigint"),
                        ("total", "numeric(12,2)"),
                        ("note", "text"),
                    ],
                ),
                table("users", Some(16_410), &[("id", "integer")]),
            ],
        )],
        Vec::new(),
    );
    show(vcx, id, Some(renamed));
    let after = key_of(vcx, "orders_v2");
    assert_eq!(before, after, "an OID keeps the key through a rename");
    assert!(is_expanded(vcx, after));
    assert!(
        lines(vcx).contains(&"          COLUMNS [3]".to_string()),
        "{:#?}",
        lines(vcx)
    );
}

#[gpui::test]
fn mysql_rename_hint_keeps_expansion(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![mysql("billing")]);
    let id = profile_id(vcx, "billing");
    let generation = |g: u64, name: &str| {
        catalog_gen(
            DbmsKind::MySql,
            g,
            vec![schema(
                DbmsKind::MySql,
                &["shop"],
                false,
                vec![table(name, None, &[("id", "int"), ("sku", "varchar(32)")])],
            )],
            Vec::new(),
        )
    };
    show(vcx, id, Some(generation(1, "items")));
    open(vcx, &["billing", "shop", "TABLES", "items"]);
    let old_key = key_of(vcx, "items");
    let object = |name: &str| ObjectKey {
        kind: ObjectKind::Table,
        path: ["shop", name].into_iter().map(Into::into).collect(),
        server_id: None,
        overload_sig: None,
        raw_kind: None,
    };
    let delta = CatalogDelta {
        renamed: vec![(object("items"), object("articles"), Confidence::Heuristic)],
        ..CatalogDelta::default()
    };
    let next = generation(2, "articles");
    vcx.cx
        .update(|cx| adapter::set_catalog(id, Some(next), Some(&delta), cx));
    vcx.run_until_parked();
    let new_key = key_of(vcx, "articles");
    assert_ne!(old_key, new_key, "no server id: the key follows the path");
    assert!(
        is_expanded(vcx, new_key),
        "the rename hint moved the expansion"
    );
    assert!(
        lines(vcx).contains(&"        COLUMNS [2]".to_string()),
        "{:#?}",
        lines(vcx)
    );
}

#[gpui::test]
fn tree_key_differs_across_profiles_for_the_same_oid(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("a", "app"), pg("b", "app")]);
    let (a, b) = (profile_id(vcx, "a"), profile_id(vcx, "b"));
    let key = ObjectKey {
        kind: ObjectKind::Table,
        path: ["app", "public", "orders"]
            .into_iter()
            .map(Into::into)
            .collect(),
        server_id: Some(ServerId::Oid(16_400)),
        overload_sig: None,
        raw_kind: None,
    };
    assert_ne!(
        adapter::object_tree_key(a, &key),
        adapter::object_tree_key(b, &key)
    );
    // In the tree: both profiles show the same catalog, their rows never share a key.
    show(vcx, a, Some(pg_shop()));
    show(vcx, b, Some(pg_shop()));
    for p in ["a", "b"] {
        let root = adapter::profile_key(profile_id(vcx, p));
        set_expanded(vcx, root, true);
    }
    let apps: Vec<TreeKey> = rows(vcx)
        .into_iter()
        .filter(|r| strip(&r.data.label) == "app")
        .map(|r| r.key)
        .collect();
    assert_eq!(apps.len(), 2);
    assert_ne!(apps[0], apps[1]);
    // A different OID is a different object even at the same path.
    let other = ObjectKey {
        server_id: Some(ServerId::Oid(16_401)),
        ..key.clone()
    };
    assert_ne!(
        adapter::object_tree_key(a, &key),
        adapter::object_tree_key(a, &other)
    );
}

// ---- groups and badges ------------------------------------------------------------------------

#[gpui::test]
fn built_in_group_is_collapsed_and_last(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![
                schema(DbmsKind::Postgres, &["app", "public"], false, vec![]),
                schema(
                    DbmsKind::Postgres,
                    &["app", "pg_catalog"],
                    true,
                    vec![table("pg_class", Some(1259), &[])],
                ),
                schema(
                    DbmsKind::Postgres,
                    &["app", "information_schema"],
                    true,
                    vec![],
                ),
                schema(DbmsKind::Postgres, &["app", "zeta"], false, vec![]),
            ],
        )),
    );
    open(vcx, &["shop", "app"]);
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    public",
            "    zeta",
            "    Built-in"
        ]
    );
    let builtin = key_of(vcx, "Built-in");
    assert!(!is_expanded(vcx, builtin));
    assert_eq!(row_of(vcx, "Built-in").icon, Some(IconName::Library));
    // Inside: the system schemas.
    set_expanded(vcx, builtin, true);
    assert_eq!(
        &lines(vcx)[6..],
        ["      information_schema", "      pg_catalog"]
    );
}

#[gpui::test]
fn invisible_chars_badge(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![
                    table("fatura\u{200D}lar", Some(9), &[]),
                    table("users", Some(10), &[]),
                ],
            )],
        )),
    );
    open(vcx, &["shop", "app", "public", "TABLES"]);
    let row = row_of(vcx, "fatura\u{200D}lar");
    assert_eq!(row.badge.as_deref(), Some("invisible characters"));
    assert_eq!(row.badge_glyph, Some(IconName::EyeOff));
    let tip = strip(row.tooltip.as_deref().expect("a tooltip"));
    assert!(tip.contains("U+200D ZERO WIDTH JOINER"), "{tip}");
    // The words are the row's accessible description (the widget adds none).
    assert_eq!(
        row.a11y_description.as_deref(),
        Some("invisible characters")
    );
    // The name is kept exactly; a plain name carries no badge.
    assert!(row.label.contains("fatura\u{200D}lar"));
    assert!(row_of(vcx, "users").badge.is_none());
}

#[gpui::test]
fn unknown_kind_row_shows_its_raw_kind(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![
                    table("orders", Some(1), &[]),
                    unknown("mystery", "relkind q"),
                ],
            )],
        )),
    );
    open(vcx, &["shop", "app", "public"]);
    // Kinds without a group follow the groups; nothing is hidden.
    assert_eq!(
        &lines(vcx)[4..],
        ["      TABLES [1]", "      mystery (relkind q)"]
    );
}

// ---- contributed nodes ------------------------------------------------------------------------

struct Scripts;

fn scripts_key(n: u128) -> TreeKey {
    TreeKey(0x5c_0000 + n)
}

fn node(key: TreeKey, label: &str, children: ContributedChildren) -> ContributedNode {
    let mut row = TreeRowData::new(label.to_string(), TextSource::Data);
    row.icon = Some(IconName::FolderOpen);
    ContributedNode {
        key,
        row,
        children,
        filter_text: Some(label.to_string()),
    }
}

impl ExplorerContributor for Scripts {
    fn database_children(
        &self,
        _profile: ProfileId,
        database: &str,
        _cx: &gpui::App,
    ) -> Vec<ContributedNode> {
        if database != "app" {
            return Vec::new();
        }
        vec![node(
            scripts_key(1),
            "Scripts",
            ContributedChildren::Lazy(Arc::new(|_| {
                ChildState::Loaded(Arc::from([scripts_key(2)]))
            })),
        )]
    }
    fn children(&self, _key: TreeKey, _cx: &gpui::App) -> ChildState {
        ChildState::Leaf
    }
    fn activate(&self, _key: TreeKey, _window: &mut gpui::Window, _cx: &mut gpui::App) {}
    fn context_menu(&self, _key: TreeKey, _cx: &gpui::App) -> Vec<MenuItemSpec> {
        Vec::new()
    }
    fn generation(&self) -> u64 {
        1
    }
    fn child_node(&self, key: TreeKey, _cx: &gpui::App) -> Option<ContributedNode> {
        (key == scripts_key(2)).then(|| node(key, "report.sql", ContributedChildren::Leaf))
    }
}

#[gpui::test]
fn contributor_nodes_sit_above_schemas(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    vcx.cx
        .update(|cx| register_contributor(Rc::new(Scripts), cx));
    let id = profile_id(vcx, "shop");
    show(vcx, id, Some(pg_shop()));
    open(vcx, &["shop", "app"]);
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    Scripts",
            "    public"
        ]
    );
    assert_eq!(
        model(vcx).info(scripts_key(1)),
        Some(NodeInfo::Contributed { profile: id })
    );
}

#[gpui::test]
fn contributed_node_with_lazy_children_expands_and_filters(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    vcx.cx
        .update(|cx| register_contributor(Rc::new(Scripts), cx));
    let id = profile_id(vcx, "shop");
    show(vcx, id, Some(pg_shop()));
    open(vcx, &["shop", "app", "Scripts"]);
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    Scripts",
            "      report.sql",
            "    public",
        ]
    );
    // Collapse everything, then filter: the loaded child matches and its ancestors open.
    for l in ["Scripts", "app", "shop"] {
        let k = key_of(vcx, l);
        set_expanded(vcx, k, false);
    }
    filter(vcx, "report");
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    Scripts",
            "      report.sql"
        ]
    );
}

// ---- filter -----------------------------------------------------------------------------------

#[gpui::test]
fn filter_reveals_ancestors_of_matches(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app"), pg("other", "app")]);
    let id = profile_id(vcx, "shop");
    show(vcx, id, Some(pg_shop()));
    filter(vcx, "total");
    // `total` is a column of `orders` and the name of `f_total`; every row above them opens.
    assert_eq!(
        lines(vcx),
        [
            "DEV [1]",
            "shop [PostgreSQL]",
            "  app",
            "    public",
            "      TABLES [1 of 2]",
            "        orders",
            "          COLUMNS [1 of 2]",
            "            total (numeric(12,2))",
            "      FUNCTIONS [1 of 1]",
            "        f_total",
        ]
    );
    enable_registry(vcx);
    assert_eq!(
        text_of(vcx, "explorer-filter-count").as_deref(),
        Some("2 matches")
    );
    // Clearing the filter restores the expansion it replaced (nothing was open).
    clear_filter(vcx, "total");
    assert_eq!(
        lines(vcx),
        ["DEV [2]", "shop [PostgreSQL]", "other [PostgreSQL]"]
    );
}

#[gpui::test]
fn filter_no_match_shows_the_empty_state(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(pg_shop()));
    filter(vcx, "zzz");
    enable_registry(vcx);
    assert_eq!(
        text_of(vcx, "explorer-filter-none").as_deref(),
        Some("No objects match")
    );
    assert_eq!(
        text_of(vcx, "explorer-filter-none-body").as_deref(),
        Some("Nothing loaded matches “zzz”.")
    );
    click(vcx, "button-explorer-filter-clear");
    assert_eq!(text_of(vcx, "explorer-filter-none"), None);
    assert_eq!(lines(vcx), ["DEV [1]", "shop [PostgreSQL]"]);
}

/// A catalog with `İSTANBUL_ŞUBE`.
fn istanbul() -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::Postgres,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            vec![
                table("İSTANBUL_ŞUBE", Some(1), &[]),
                table("ıspanak", Some(2), &[]),
            ],
        )],
    )
}

#[gpui::test]
fn filter_is_turkish_tolerant_for_tr_only(cx: &mut TestAppContext) {
    // D-074: Turkish UI → tolerant by default: `istanbul` finds `İSTANBUL_ŞUBE`.
    let vcx = shell_with(cx, "tr", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(istanbul()));
    filter(vcx, "istanbul");
    assert!(
        lines(vcx).iter().any(|l| l.trim() == "İSTANBUL_ŞUBE"),
        "{:#?}",
        lines(vcx)
    );
    // The toggle off: exact (ASCII case only): no match.
    click_tolerant(vcx);
    enable_registry(vcx);
    assert_eq!(
        text_of(vcx, "explorer-filter-none").as_deref(),
        Some("Eşleşen nesne yok")
    );
    close_all_windows(vcx);
    // English UI → off by default; the toggle turns it on.
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(istanbul()));
    filter(vcx, "istanbul");
    enable_registry(vcx);
    assert_eq!(
        text_of(vcx, "explorer-filter-none").as_deref(),
        Some("No objects match")
    );
    click_tolerant(vcx);
    assert!(
        lines(vcx).iter().any(|l| l.trim() == "İSTANBUL_ŞUBE"),
        "{:#?}",
        lines(vcx)
    );
}

#[gpui::test]
fn typeahead_fold_follows_d074(cx: &mut TestAppContext) {
    let fold = |vcx: &VisualTestContext| {
        vcx.cx
            .read(|cx| format!("{:?}", explorer::tree(cx).expect("the tree").read(cx)))
    };
    // A Turkish UI: Tolerant (I i İ ı one letter).
    let vcx = shell_with(cx, "tr", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(istanbul()));
    assert!(fold(vcx).contains("fold: Tolerant"), "{}", fold(vcx));
    open(vcx, &["shop", "app", "public", "TABLOLAR"]);
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
    // Typing `i` lands on `ıspanak` (dotless) under the tolerant fold.
    vcx.simulate_keystrokes("i");
    let cursor = vcx
        .cx
        .read(|cx| explorer::tree(cx).unwrap().read(cx).cursor());
    assert_eq!(cursor, Some(key_of(vcx, "ıspanak")));
    // The filter's toggle switches it.
    click_tolerant(vcx);
    assert!(
        fold(vcx).contains("fold: Locale { turkic: true }"),
        "{}",
        fold(vcx)
    );
    close_all_windows(vcx);
    // An English UI: the access-key fold.
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    assert!(
        fold(vcx).contains("fold: Locale { turkic: false }"),
        "{}",
        fold(vcx)
    );
    click_tolerant(vcx);
    assert!(fold(vcx).contains("fold: Tolerant"), "{}", fold(vcx));
}

// ---- profile roots ----------------------------------------------------------------------------

fn connected_prod(cx: &mut TestAppContext, name: &str) -> &'static mut VisualTestContext {
    connected_prod_in(cx, name, "en")
}

fn connected_prod_in(
    cx: &mut TestAppContext,
    name: &str,
    lang: &str,
) -> &'static mut VisualTestContext {
    use hacivat_driver::{SemVerLoose, ServerInfo};
    let fake = answers_probe(FakeDriver::new(DbmsKind::Postgres)).server(ServerInfo {
        kind: DbmsKind::Postgres,
        product: "PostgreSQL".to_string(),
        version: SemVerLoose::parse("16.4"),
        raw_version: "16.4".to_string(),
        is_replica: Some(false),
    });
    let vcx = shell_with_fake(cx, lang, fake);
    let mut p = test_profile(name, Some(EnvTag::Production), "10.0.4.12");
    p.read_only = true;
    p.database = Some("app".to_string());
    add_profile(vcx, p);
    connect(vcx, name);
    wait_until(vcx, "the connect to finish", |vcx| {
        matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
    });
    vcx
}

#[gpui::test]
fn profile_rows_keep_their_m2a_content_and_bindings(cx: &mut TestAppContext) {
    let vcx = connected_prod(cx, "orders-prod");
    let id = profile_id(vcx, "orders-prod");
    let row = row_of(vcx, "orders-prod");
    assert_eq!(row.icon, Some(IconName::Database));
    assert_eq!(row.label_source, TextSource::Data);
    assert!(row.strong, "a connected root's name is strong");
    // The M2a flags: the environment mark, the lock (red on production), and lock-open while
    // the live connection is plaintext (the fake's is).
    assert_eq!(
        row.flags,
        [
            RowFlag::Env(hacivat_ui::capsule::EnvKind::Production),
            RowFlag::Lock { production: true },
            RowFlag::Icon {
                icon: IconName::LockOpen,
                text: "unencrypted".into()
            },
        ]
    );
    assert_eq!(row.meta_mark, Some(MetaMark::Ok));
    assert!(
        row.meta.as_deref().is_some_and(|m| m.contains("ms")),
        "{:?}",
        row.meta
    );
    assert_eq!(
        row.a11y_description.as_deref(),
        Some("production, read-only, unencrypted")
    );
    // Bindings: on the root, the M2a commands act (Enter disconnects).
    show(vcx, id, Some(pg_shop()));
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
    assert!(
        explorer_context(vcx),
        "on the root, the Explorer rows apply"
    );
    // On a catalog row the Explorer key context is gone (R-W2-12): the chords reach the tree.
    // Enter toggles, Backspace deletes nothing.
    open(vcx, &["orders-prod", "app"]);
    let app = key_of(vcx, "app");
    put_cursor(vcx, app);
    assert!(!explorer_context(vcx));
    assert_eq!(
        vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected()),
        Some(id),
        "the sheet follows the row's profile"
    );
    assert!(is_expanded(vcx, app));
    vcx.simulate_keystrokes("enter");
    assert!(!is_expanded(vcx, app), "Enter on a database row toggles it");
    assert!(
        matches!(
            live(vcx, "orders-prod"),
            hacivat_session::LiveState::Connected(_)
        ),
        "Enter on a database row never disconnects"
    );
    vcx.simulate_keystrokes("backspace");
    assert!(
        layer_ids(vcx).is_empty(),
        "Backspace on a database row opens no delete dialog"
    );
    // Back on the root, Enter disconnects (the M2a binding).
    let root = adapter::profile_key(id);
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).unwrap();
        tree.update(cx, |t, cx| t.select(root, cx));
    });
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    wait_until(vcx, "the disconnect", |vcx| {
        matches!(
            live(vcx, "orders-prod"),
            hacivat_session::LiveState::Disconnected
        )
    });
}

/// Whether the key context stack holds `Explorer` (the tree is focused in these tests).
fn explorer_context(vcx: &VisualTestContext) -> bool {
    in_window(vcx, |window, _| window.refresh());
    vcx.run_until_parked();
    context_stack(vcx).0.contains(&"Explorer")
}

/// Moves the tree's cursor (and selection) to `key`.
fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).unwrap();
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

#[gpui::test]
fn trust_signals_stay_drawn_on_profile_roots(cx: &mut TestAppContext) {
    // A long production name in the 276 px explorer: the mark and the lock never shrink or
    // clip (NOTES cross-cutting #5); the name gives way. In English at the default size, and
    // in Turkish at the 960 × 600 minimum (fix round 1, m6).
    for (lang, size) in [
        ("en", None),
        ("tr", Some(gpui::size(gpui::px(960.0), gpui::px(600.0)))),
    ] {
        let vcx = connected_prod_in(cx, "orders-reporting-replica-eu-west-1-standby", lang);
        if let Some(size) = size {
            vcx.simulate_resize(size);
            vcx.run_until_parked();
        }
        trust_signals_drawn(vcx);
        close_all_windows(vcx);
    }
}

fn trust_signals_drawn(vcx: &mut VisualTestContext) {
    let id = profile_id(vcx, "orders-reporting-replica-eu-west-1-standby");
    let key = adapter::profile_key(id).0;
    vcx.run_until_parked();
    let row = vcx
        .cx
        .read(explorer::profile_rows)
        .into_iter()
        .find(|(p, _)| *p == id)
        .and_then(|(_, b)| b)
        .expect("the root row is drawn");
    let list = vcx.debug_bounds("explorer-list").expect("the explorer");
    for i in 0..2 {
        let sel: &'static str = Box::leak(format!("hv-tree-flag-{key}-{i}").into_boxed_str());
        let flag = vcx
            .debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} is drawn"));
        assert!(flag.size.width > gpui::px(0.0), "{sel}: {flag:?}");
        assert!(
            flag.origin.x >= row.origin.x
                && flag.origin.x + flag.size.width <= row.origin.x + row.size.width,
            "{sel} inside its row: {flag:?} {row:?}"
        );
        assert!(
            flag.origin.x + flag.size.width <= list.origin.x + list.size.width,
            "{sel} inside the explorer"
        );
    }
    enable_registry(vcx);
    let name = registry_frame(vcx)
        .into_iter()
        .find(|r| r.path.ends_with("#hv-tree-label"))
        .expect("the name is drawn");
    assert!(name.truncated && !name.clipped, "{name:?}");
    // The production capsule keeps its block (M3a trust check).
    let capsule = format!("{:?}", capsule_model(vcx));
    assert!(capsule.contains("Production"), "{capsule}");
}

/// The model's children answer only from memory: a key it never showed is "not loaded".
#[gpui::test]
fn the_model_claims_nothing_about_unknown_keys(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(pg_shop()));
    let m = model(vcx);
    assert!(m.row(TreeKey(42)).is_none());
    assert_eq!(m.children(TreeKey(42)), ChildState::NotLoaded);
}

// ---- fix round 1 ------------------------------------------------------------------------------

/// The text the last frame drew with label id `id`.
fn drawn_text(vcx: &VisualTestContext, id: &str) -> Option<String> {
    enable_registry(vcx);
    text_of(vcx, id)
}

fn selected(vcx: &VisualTestContext) -> Option<ProfileId> {
    vcx.cx.read(|cx| cx.global::<ConnectionsModel>().selected())
}

fn enabled(vcx: &VisualTestContext, id: &str) -> bool {
    vcx.cx.read(|cx| {
        cx.global::<hacivat_ui::actions::ActionRegistry>()
            .get(id)
            .is_some_and(|d| (d.enabled)(cx))
    })
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

/// I1, ruling R-W2-12: the palette's and welcome page's connection commands act on the profile
/// that owns the cursor row, wherever it is, and the sheet shows that same profile; the
/// `Explorer` key context exists only on a profile root.
#[gpui::test]
fn commands_follow_the_profile_of_the_cursor_row(cx: &mut TestAppContext) {
    let mut other = pg("other", "app");
    other.endpoint = ProfileEndpoint::Server {
        host: "127.0.0.2".to_string(),
        port: None,
    };
    let vcx = shell_with(cx, "en", vec![pg("shop", "app"), other]);
    let (shop, other) = (profile_id(vcx, "shop"), profile_id(vcx, "other"));
    show(vcx, shop, Some(pg_shop()));
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
    open(vcx, &["shop", "app", "public", "TABLES"]);
    for label in ["app", "TABLES", "orders"] {
        let key = key_of(vcx, label);
        put_cursor(vcx, key);
        assert_eq!(selected(vcx), Some(shop), "{label}: the sheet's profile");
        assert!(
            !explorer_context(vcx),
            "{label}: no Explorer rows off a root"
        );
        for id in [
            "connection.toggle_connect",
            "connection.edit",
            "connection.duplicate",
            "connection.copy_url",
            "connection.delete",
        ] {
            assert!(enabled(vcx, id), "{label}: {id}");
        }
        assert!(run(vcx, "connection.copy_url"));
        assert!(
            clipboard(vcx).contains("127.0.0.1"),
            "{label}: {}",
            clipboard(vcx)
        );
    }
    // Another profile's root: it is the target, and the Explorer rows apply there.
    put_cursor(vcx, adapter::profile_key(other));
    assert_eq!(selected(vcx), Some(other));
    assert!(explorer_context(vcx));
    assert!(run(vcx, "connection.copy_url"));
    assert!(clipboard(vcx).contains("127.0.0.2"), "{}", clipboard(vcx));
}

/// m7: Shift-F10 off a root opens no connection menu; the tree asks for the row's own menu.
#[gpui::test]
fn shift_f10_off_a_root_asks_the_tree(cx: &mut TestAppContext) {
    use std::cell::RefCell;
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(pg_shop()));
    in_window(vcx, explorer::focus_list);
    vcx.run_until_parked();
    open(vcx, &["shop"]);
    let app = key_of(vcx, "app");
    put_cursor(vcx, app);
    let seen: Rc<RefCell<Vec<TreeKey>>> = Rc::default();
    let sink = seen.clone();
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).unwrap();
        cx.subscribe(&tree, move |_, e: &hacivat_ui::tree::TreeEvent, _| {
            if let hacivat_ui::tree::TreeEvent::ContextMenu(k, _) = e {
                sink.borrow_mut().push(*k);
            }
        })
        .detach();
    });
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    assert_eq!(
        seen.borrow().as_slice(),
        [app],
        "the tree's menu event, for the row"
    );
    assert!(layer_ids(vcx).is_empty(), "no connection menu off a root");
    // On the root, the M2a binding opens the connection menu.
    put_cursor(vcx, adapter::profile_key(profile_id(vcx, "shop")));
    vcx.simulate_keystrokes("shift-f10");
    vcx.run_until_parked();
    assert_eq!(layer_ids(vcx), ["menu"]);
}

/// I2: with something not searched, nothing matching is never "No objects match": the roots
/// stay and the "Searches loaded names only" line says why.
fn assert_not_no_match(vcx: &mut VisualTestContext, query: &str, hint: &str) {
    filter(vcx, query);
    assert_eq!(drawn_text(vcx, "explorer-filter-none"), None, "{query}");
    let line = drawn_text(vcx, hint).unwrap_or_else(|| panic!("{hint} is drawn"));
    assert!(line.starts_with("Searches loaded names only."), "{line}");
    assert!(
        lines(vcx).iter().any(|l| l.starts_with("shop ")),
        "the root stays: {:#?}",
        lines(vcx)
    );
}

#[gpui::test]
fn filter_without_a_catalog_is_never_no_match(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    assert_not_no_match(vcx, "orders", "explorer-filter-not-loaded");
}

#[gpui::test]
fn filter_over_a_generation_zero_catalog_is_never_no_match(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog_gen(DbmsKind::Postgres, 0, Vec::new(), Vec::new())),
    );
    assert_not_no_match(vcx, "orders", "explorer-filter-not-loaded");
}

#[gpui::test]
fn filter_over_a_state_only_namespace_is_never_no_match(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog_gen(
            DbmsKind::Postgres,
            2,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![],
            )],
            vec![(NsKey::schema(["app", "audit"]), LoadState::NotLoaded)],
        )),
    );
    assert_not_no_match(vcx, "orders", "explorer-filter-not-loaded");
}

#[gpui::test]
fn filter_over_names_level_relations_is_never_no_match(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![schema_at(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                hacivat_meta::Level::Names,
                vec![
                    table("orders", Some(1), &[]),
                    leaf(ObjectKind::View, "v_open"),
                ],
            )],
        )),
    );
    // A column name: the columns are not loaded at Names.
    assert_not_no_match(vcx, "total", "explorer-filter-unloaded");
    let line = drawn_text(vcx, "explorer-filter-unloaded").unwrap();
    assert!(line.contains(" 2 "), "the table and the view: {line}");
}

/// m1: a profile renamed while a filter is on is matched again.
#[gpui::test]
fn a_renamed_profile_is_filtered_again(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("alpha", "app"), pg("gamma", "app")]);
    for name in ["alpha", "gamma"] {
        show(vcx, profile_id(vcx, name), Some(pg_shop()));
    }
    filter(vcx, "alpha");
    assert_eq!(lines(vcx), ["DEV [1]", "alpha [PostgreSQL]"]);
    let gamma = vcx
        .cx
        .read(|cx| {
            cx.global::<ConnectionsModel>()
                .get(profile_id(vcx, "gamma"))
        })
        .expect("gamma");
    let mut renamed = (*gamma).clone();
    renamed.name = "alpha-2".to_string();
    save_draft(vcx, hacivat_session::profile::ProfileDraft::edit(renamed));
    vcx.run_until_parked();
    assert_eq!(
        lines(vcx),
        ["DEV [2]", "alpha [PostgreSQL]", "alpha-2 [PostgreSQL]"]
    );
}

/// m3: a dim row says why; database rows say what they are.
#[gpui::test]
fn out_of_scope_rows_say_so(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    show(
        vcx,
        id,
        Some(catalog_gen(
            DbmsKind::Postgres,
            2,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![],
            )],
            vec![
                (NsKey::schema(["app", "audit"]), LoadState::OutOfScope),
                (NsKey::database("archive"), LoadState::OutOfScope),
            ],
        )),
    );
    open(vcx, &["shop", "app"]);
    let audit = row_of(vcx, "audit");
    assert!(audit.dim);
    assert_eq!(
        audit.a11y_description.as_deref(),
        Some("schema, out of scope")
    );
    let archive = row_of(vcx, "archive");
    assert!(archive.dim);
    assert_eq!(
        archive.a11y_description.as_deref(),
        Some("database, out of scope")
    );
    assert_eq!(
        row_of(vcx, "app").a11y_description.as_deref(),
        Some("database")
    );
}

/// m4: an index on an expression shows every part, the expression too.
#[gpui::test]
fn an_expression_index_shows_its_expression_part(cx: &mut TestAppContext) {
    use hacivat_meta::props::{IndexProps, KeyColumn, KeyProps, Props};
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    let id = profile_id(vcx, "shop");
    let mut t = table("users", Some(1), &[("a", "text"), ("b", "text")]);
    let root = t.root();
    t.add(
        root,
        hacivat_meta::Obj::new(ObjectKind::Index, "ix_users").with_props(Props::Index(Arc::new(
            IndexProps {
                key: KeyProps {
                    columns: vec![
                        KeyColumn {
                            ordinal: 1,
                            name: Some(Arc::from("a")),
                        },
                        KeyColumn {
                            ordinal: 2,
                            name: None,
                        },
                    ],
                    expressions: vec![Arc::from("lower(b)")],
                    ..KeyProps::default()
                },
                ..IndexProps::default()
            },
        ))),
    );
    show(
        vcx,
        id,
        Some(catalog(
            DbmsKind::Postgres,
            vec![schema(
                DbmsKind::Postgres,
                &["app", "public"],
                false,
                vec![t],
            )],
        )),
    );
    open(
        vcx,
        &["shop", "app", "public", "TABLES", "users", "INDEXES"],
    );
    assert_eq!(
        row_of(vcx, "ix_users").type_text.map(|t| strip(&t)),
        Some("(a, lower(b))".to_string())
    );
}

/// m8 (mock surface 4): Escape clears the field; on an empty field it returns the focus to the
/// tree; ↓ moves into the tree.
#[gpui::test]
fn escape_clears_the_filter_and_down_enters_the_tree(cx: &mut TestAppContext) {
    let vcx = shell_with(cx, "en", vec![pg("shop", "app")]);
    show(vcx, profile_id(vcx, "shop"), Some(pg_shop()));
    let tree_focused = |vcx: &VisualTestContext| {
        in_window(vcx, |window, cx| {
            use gpui::Focusable as _;
            explorer::tree(cx)
                .unwrap()
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        })
    };
    filter(vcx, "total");
    assert!(lines(vcx).len() > 2);
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert_eq!(
        lines(vcx),
        ["DEV [1]", "shop [PostgreSQL]"],
        "the filter is cleared"
    );
    assert!(!tree_focused(vcx), "the field keeps the focus");
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        tree_focused(vcx),
        "Escape on an empty field returns to the tree"
    );
    in_window(vcx, explorer::focus_filter);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("down");
    vcx.run_until_parked();
    assert!(tree_focused(vcx), "↓ moves into the tree");
}
