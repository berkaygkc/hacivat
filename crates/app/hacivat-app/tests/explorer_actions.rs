//! M4 Task 24: the explorer's actions (design/m4-explorer surface 5; R-M4-13, R-M4-19,
//! PO-Q13, PO-Q16): copy names with the dialect's quoting, Open data through the Run path, hide
//! objects like this, the Modify… placeholder, the replaceable open-data handler and provided
//! actions in their slots.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::explorer::{self, ShownRow};
use hacivat_app::console::Consoles;
use hacivat_app::explorer::adapter;
use hacivat_app::explorer::{
    ActionSlot, ActionSurface, ExplorerActionProvider, NodeRef, ProvidedAction, node_menu,
    register_action_provider, set_open_data_handler,
};
use hacivat_driver::fake::{FakeDriver, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall};
use hacivat_meta::{CatalogSnapshot, NsKey, ObjectKind};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::{Profile, ProfileEndpoint};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::tree::TreeKey;
use hacivat_ui_core::gpui_surface::App;
use support::catalogs::*;
use support::*;

// ---- helpers ----------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn rows(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(explorer::shown_rows)
}

fn labels(vcx: &VisualTestContext) -> Vec<String> {
    rows(vcx)
        .into_iter()
        .map(|r| strip(&r.data.label))
        .collect()
}

/// The key of the last shown row labelled `label` (the newest profile's "TABLES").
fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    rows(vcx)
        .into_iter()
        .rfind(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown in {:?}", labels(vcx)))
        .key
}

fn open(vcx: &VisualTestContext, labels: &[&str]) {
    for l in labels {
        let k = key_of(vcx, l);
        vcx.cx.update(|cx| {
            let tree = explorer::tree(cx).expect("the tree");
            tree.update(cx, |t, cx| t.set_expanded(k, true, cx));
        });
        vcx.run_until_parked();
    }
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

fn clipboard(vcx: &VisualTestContext) -> String {
    vcx.cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default()
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Arc<CatalogSnapshot>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, Some(cat), None, cx));
    vcx.run_until_parked();
}

fn pg(name: &str, env: Option<EnvTag>) -> Profile {
    let mut p = test_profile(name, env, "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

fn pg_tables(names: &[&str]) -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::Postgres,
        vec![schema(
            DbmsKind::Postgres,
            &["app", "public"],
            false,
            names
                .iter()
                .enumerate()
                .map(|(i, n)| table(n, Some(16_400 + i as u32), &[("id", "integer")]))
                .chain([leaf(ObjectKind::View, "v_open")])
                .collect(),
        )],
    )
}

fn copy_of(vcx: &VisualTestContext, label: &str) -> String {
    put_cursor(vcx, key_of(vcx, label));
    assert!(run(vcx, "explorer.copy_qualified_name"), "copy runs on {label}");
    clipboard(vcx)
}

fn menu_of(vcx: &VisualTestContext, label: &str) -> Vec<MenuItemSpec> {
    let key = key_of(vcx, label);
    // The menu opens on the row it belongs to (`open_node_menu` moves the cursor there first).
    put_cursor(vcx, key);
    vcx.cx
        .read(|cx| node_menu(key, cx))
        .expect("the row has a menu")
}

/// The menu's rows as `label`, `(disabled)` marked, separators `—`.
fn menu_lines(vcx: &VisualTestContext, items: &[MenuItemSpec]) -> Vec<String> {
    vcx.cx.read(|cx| {
        items
            .iter()
            .map(|i| {
                if i.separator {
                    return "—".to_string();
                }
                let mut s = strip(&(i.label)(cx));
                if i.disabled {
                    s.push_str(" (disabled)");
                }
                s
            })
            .collect()
    })
}

// ---- names ------------------------------------------------------------------------------------

#[gpui::test]
fn copy_qualified_name_quotes_per_dialect(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let pg_id = add_profile(vcx, pg("app-dev", Some(EnvTag::Dev)));
    show(vcx, pg_id, pg_tables(&["Users", "users"]));
    let mut my = test_profile("api-dev", Some(EnvTag::Dev), "127.0.0.1");
    my.engine = DbmsKind::MySql;
    let my_id = add_profile(vcx, my);
    show(
        vcx,
        my_id,
        catalog(
            DbmsKind::MySql,
            vec![schema(
                DbmsKind::MySql,
                &["api"],
                false,
                vec![table("order", None, &[("id", "int")])],
            )],
        ),
    );
    let mut lite = test_profile("local-file", Some(EnvTag::Local), "127.0.0.1");
    lite.engine = DbmsKind::Sqlite;
    lite.endpoint = ProfileEndpoint::File {
        path: "/data/app.db".into(),
        create_if_missing: false,
    };
    let lite_id = add_profile(vcx, lite);
    show(
        vcx,
        lite_id,
        catalog(
            DbmsKind::Sqlite,
            vec![schema(
                DbmsKind::Sqlite,
                &["main"],
                false,
                vec![table("a b", None, &[("id", "integer")])],
            )],
        ),
    );
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    assert_eq!(copy_of(vcx, "Users"), "public.\"Users\"");
    assert_eq!(copy_of(vcx, "users"), "public.users");
    assert!(toast_texts(vcx).contains(&"Copied public.users".to_string()));
    open(vcx, &["api-dev", "api", "TABLES"]);
    assert_eq!(copy_of(vcx, "order"), "api.`order`");
    open(vcx, &["local-file", "main", "TABLES"]);
    assert_eq!(copy_of(vcx, "a b"), "main.\"a b\"");
    // "Copy name" copies the name verbatim.
    put_cursor(vcx, key_of(vcx, "a b"));
    assert!(run(vcx, "explorer.copy_name"));
    assert_eq!(clipboard(vcx), "a b");
}

// ---- Open data (R-M4-13) ----------------------------------------------------------------------

/// The typed queries the fake saw.
fn queries(fake: &FakeDriver) -> Vec<String> {
    fake.typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(sql) => Some(sql),
            _ => None,
        })
        .collect()
}

fn users_query() -> FakeQuery {
    FakeQuery {
        prefix: "SELECT * FROM public.users",
        sets: vec![FakeResultSet {
            columns: vec![(
                "id",
                hacivat_values::LogicalType::Int {
                    bits: 32,
                    signed: true,
                },
            )],
            rows: FakeRows::Literal(vec![vec![hacivat_values::Value::Int(1)]]),
        }],
        ..Default::default()
    }
}

/// A connected profile `name` in `env` showing `app.public.users`; its fake.
fn connected_with_users(
    cx: &mut TestAppContext,
    name: &str,
    env: EnvTag,
) -> (&'static mut VisualTestContext, ProfileId) {
    let vcx = shell_with_fake(
        cx,
        "en",
        answers_probe_with(FakeDriver::new(DbmsKind::Postgres), vec![users_query()]),
    );
    in_window(vcx, |window, _| window.activate_window());
    let id = add_profile(vcx, pg(name, Some(env)));
    connect(vcx, name);
    wait_until(vcx, "connected", |vcx| {
        matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
    });
    show(vcx, id, pg_tables(&["users"]));
    open(vcx, &[name, "app", "public", "TABLES"]);
    (vcx, id)
}

fn open_data_on_users(vcx: &mut VisualTestContext) {
    let tabs = tab_count(vcx);
    put_cursor(vcx, key_of(vcx, "users"));
    assert!(run(vcx, "explorer.open_data"), "Open data runs on a table");
    vcx.run_until_parked();
    assert_eq!(tab_count(vcx), tabs + 1, "a new console");
}

fn new_console_text(vcx: &VisualTestContext) -> String {
    let id = vcx.cx.read(|cx| {
        Consoles::ids(cx)
            .into_iter()
            .filter(|c| {
                Consoles::get(cx, *c)
                    .and_then(|s| s.editor.as_ref())
                    .is_some()
            })
            .max_by_key(|c| Consoles::get(cx, *c).map(|s| s.n))
            .expect("a console")
    });
    editor_text(vcx, id)
}

#[gpui::test]
fn open_data_creates_a_console_and_runs_through_the_gate(cx: &mut TestAppContext) {
    let (vcx, _id) = connected_with_users(cx, "app-dev", EnvTag::Dev);
    open_data_on_users(vcx);
    wait_until(vcx, "the console's text", |vcx| {
        new_console_text(vcx) == "SELECT * FROM public.users"
    });
    let f = fake(vcx);
    wait_until(vcx, "the run", |_| {
        queries(&f).iter().any(|q| q == "SELECT * FROM public.users")
    });
}

#[gpui::test]
fn open_data_on_production_is_a_read_and_needs_no_confirmation(cx: &mut TestAppContext) {
    let (vcx, _id) = connected_with_users(cx, "orders-prod", EnvTag::Production);
    open_data_on_users(vcx);
    let f = fake(vcx);
    wait_until(vcx, "the run", |_| {
        queries(&f).iter().any(|q| q == "SELECT * FROM public.users")
    });
    assert!(
        !layer_ids(vcx).contains(&"dialog"),
        "a read on production asks nothing: {:?}",
        layer_ids(vcx)
    );
}

#[gpui::test]
fn open_data_handler_can_be_replaced(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("app-dev", Some(EnvTag::Dev)));
    show(vcx, id, pg_tables(&["users"]));
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    let seen: Rc<RefCell<Vec<(ProfileId, String)>>> = Rc::default();
    let sink = seen.clone();
    vcx.cx.update(|cx| {
        set_open_data_handler(
            Box::new(move |p, k, _, _| sink.borrow_mut().push((p, k.path[2].to_string()))),
            cx,
        )
    });
    let tabs = tab_count(vcx);
    // A double click (Enter) on a table opens its data through the installed handler.
    put_cursor(vcx, key_of(vcx, "users"));
    let tree = vcx.cx.read(|cx| explorer::tree(cx).expect("the tree"));
    in_window(vcx, |window, cx| {
        let focus = tree.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert_eq!(seen.borrow().as_slice(), [(id, "users".to_string())]);
    assert_eq!(tab_count(vcx), tabs, "the default handler did not run");
}

// ---- hide objects like this -------------------------------------------------------------------

#[gpui::test]
fn hide_like_this_adds_a_view_filter_and_lookup_still_finds_it(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("app-dev", Some(EnvTag::Dev)));
    let cat = pg_tables(&["yedek_fatura_2019", "yedek_fatura_2020", "users"]);
    show(vcx, id, cat.clone());
    wait_until(vcx, "the profile's data", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::explorer::userdata::get(id, cx))
            .is_some()
    });
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    put_cursor(vcx, key_of(vcx, "yedek_fatura_2019"));
    assert!(run(vcx, "explorer.hide_like_this"));
    wait_until(vcx, "the rows hidden", |vcx| {
        !labels(vcx).iter().any(|l| l.starts_with("yedek_"))
    });
    assert!(labels(vcx).contains(&"users".to_string()));
    let header = rows(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == "TABLES")
        .expect("the group")
        .data;
    assert_eq!(header.meta.as_deref().map(strip).as_deref(), Some("1"));
    assert_eq!(
        header.filter_glyph.map(|t| strip(&t)).as_deref(),
        Some(
            "2 objects are hidden by a view filter (table:-yedek_fatura_*). Show hidden \
             objects, in this group's menu, removes it."
        )
    );
    // The catalog still holds them: lookups find them.
    let ns = cat.namespaces.get(&NsKey::schema(["app", "public"])).unwrap();
    let key = ns
        .children(0)
        .find(|(_, o)| &**o.name() == "yedek_fatura_2019")
        .and_then(|(i, _)| ns.object_key(i))
        .expect("the key");
    let shown = vcx
        .cx
        .read(|cx| adapter::catalog_of(id, cx))
        .expect("the catalog");
    assert!(shown.find(&key).is_some(), "lookup still finds a hidden object");
    // Show hidden objects removes the filter.
    put_cursor(vcx, key_of(vcx, "TABLES"));
    assert!(run(vcx, "explorer.show_hidden"));
    wait_until(vcx, "the rows back", |vcx| {
        labels(vcx).contains(&"yedek_fatura_2019".to_string())
    });
}

// ---- menus ------------------------------------------------------------------------------------

#[gpui::test]
fn modify_is_disabled_with_a_tooltip(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("app-dev", Some(EnvTag::Dev)));
    show(vcx, id, pg_tables(&["users"]));
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    let items = menu_of(vcx, "users");
    let modify = items
        .iter()
        .find(|i| i.action.as_ref() == "explorer.modify")
        .expect("the placeholder");
    assert!(modify.disabled);
    let hint = vcx
        .cx
        .read(|cx| modify.hint.as_ref().map(|h| strip(&h(cx))));
    assert_eq!(hint.as_deref(), Some("Arrives in a later version"));
    assert_eq!(
        menu_lines(vcx, &items),
        [
            "Open data",
            "—",
            "Copy name",
            "Copy qualified name",
            "—",
            "Refresh (disabled)",
            "Hide objects like this",
            "—",
            "Modify table… (disabled)",
        ]
    );
    assert!(!run(vcx, "explorer.modify"), "the placeholder never runs");
}

struct Provider;

impl ExplorerActionProvider for Provider {
    fn actions(&self, node: &NodeRef, surface: ActionSurface, _: &App) -> Vec<ProvidedAction> {
        if surface != ActionSurface::ExplorerMenu || node.kind != Some(ObjectKind::Table) {
            return Vec::new();
        }
        [
            (ActionSlot::Source, "fake.source"),
            (ActionSlot::Data, "fake.data"),
            (ActionSlot::Modify, "fake.modify"),
            (ActionSlot::Transfer, "fake.transfer"),
        ]
        .into_iter()
        .map(|(slot, id)| ProvidedAction {
            slot,
            id,
            label: id.into(),
            enabled: true,
        })
        .collect()
    }
}

#[gpui::test]
fn provided_actions_appear_in_their_slot_for_matching_kinds(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let id = add_profile(vcx, pg("app-dev", Some(EnvTag::Dev)));
    show(vcx, id, pg_tables(&["users"]));
    vcx.cx
        .update(|cx| register_action_provider(Rc::new(Provider), cx));
    open(vcx, &["app-dev", "app", "public", "TABLES"]);
    let items = menu_of(vcx, "users");
    assert_eq!(
        menu_lines(vcx, &items),
        [
            "Open data",
            "fake.data",
            "—",
            "Copy name",
            "Copy qualified name",
            "—",
            "Refresh (disabled)",
            "Hide objects like this",
            "—",
            "fake.transfer",
            "—",
            "fake.modify",
            "—",
            "fake.source",
        ],
        "the provider's Modify replaces the placeholder"
    );
    // A view: the provider matches tables only; the placeholder stays.
    open(vcx, &["VIEWS"]);
    let items = menu_of(vcx, "v_open");
    let lines = menu_lines(vcx, &items);
    assert!(!lines.iter().any(|l| l.starts_with("fake.")), "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("Modify view… (disabled)"));
}
