//! M4 Task 27: refresh after DDL in the app (design/m4-explorer surfaces 9 and 11): the run's
//! refresh sink, the Ask toast, the "waits for COMMIT" stamp, the stale badge for Never, the
//! broad-refresh notice for dynamic SQL, the R-W4-8 badge of a held `Partial` namespace, the
//! DDL view's Reload on a stale namespace (T25 m-9) and the T24 residual nits.

mod support;

use std::sync::Arc;

use gpui::{TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer::{self, ShownRow};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::explorer::adapter;
use hacivat_app::explorer::live::{self as hlive, CatalogSource};
use hacivat_app::explorer::refresh_prompt;
use hacivat_app::settings_ui::rows;
use hacivat_driver::fake::FakeDriver;
use hacivat_introspect::testing::fake::FakeIntrospector;
use hacivat_meta::snapshot::ContainerKey;
use hacivat_meta::{LoadState, NsKey, ObjectKind};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_session::profile::Profile;
use hacivat_settings::RefreshAfterDdl;
use hacivat_ui::toast::ToastQueue;
use hacivat_ui::tree::TreeKey;
use support::live::*;
use support::*;

// ---- helpers ----------------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn rows_of(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(explorer::shown_rows)
}

fn row(vcx: &VisualTestContext, label: &str) -> ShownRow {
    rows_of(vcx)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .unwrap_or_else(|| panic!("`{label}` is shown"))
}

fn key_of(vcx: &VisualTestContext, label: &str) -> TreeKey {
    row(vcx, label).key
}

fn set_expanded(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, true, cx));
    });
    vcx.run_until_parked();
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

fn runtime(vcx: &VisualTestContext) -> tokio::runtime::Handle {
    vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime())
}

fn public() -> NsKey {
    NsKey::schema(["app", "public"])
}

fn tables_of(vcx: &VisualTestContext, id: ProfileId) -> Vec<String> {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx)
            .and_then(|c| c.namespaces.get(&public()).cloned())
            .map(|n| {
                n.children(0)
                    .filter(|(_, o)| o.kind() == ObjectKind::Table)
                    .map(|(_, o)| o.name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn state_of(vcx: &VisualTestContext, id: ProfileId) -> Option<LoadState> {
    vcx.cx.read(|cx| {
        adapter::catalog_of(id, cx)
            .and_then(|c| c.states.get(&ContainerKey::Namespace(public())).cloned())
    })
}

fn pg_profile(name: &str) -> Profile {
    let mut p = test_profile(name, Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    p
}

/// A connected PostgreSQL profile `app-dev` whose `app.public` holds `tables`, loaded in the
/// explorer (root expanded), and its fake source.
fn connected(
    cx: &mut TestAppContext,
    tables: &[&str],
) -> (&'static mut VisualTestContext, Arc<FakeSource>, ProfileId) {
    connected_with(cx, tables, Vec::new())
}

/// [`connected`], with `extra` typed queries scripted on the user's connections.
fn connected_with(
    cx: &mut TestAppContext,
    tables: &[&str],
    extra: Vec<hacivat_driver::fake::FakeQuery>,
) -> (&'static mut VisualTestContext, Arc<FakeSource>, ProfileId) {
    let intro = FakeIntrospector::new(DbmsKind::Postgres);
    intro.put_ns(public(), fake_tables(tables));
    let vcx = shell_with_fake_prepared(
        cx,
        "en",
        answers_probe_with(FakeDriver::new(DbmsKind::Postgres), extra),
        |_| {},
    );
    in_window(vcx, |window, _| window.activate_window());
    let dir = temp_dir(vcx).join("fake-cache");
    let src = Arc::new(FakeSource::new(runtime(vcx), intro, dir));
    let as_source: Arc<dyn CatalogSource> = src.clone();
    vcx.cx.update(|cx| hlive::set_source(as_source, cx));
    let id = add_profile(vcx, pg_profile("app-dev"));
    set_expanded(vcx, adapter::profile_key(id));
    connect(vcx, "app-dev");
    let want = tables.len();
    wait_until(vcx, "the current schema loaded", |vcx| {
        tables_of(vcx, id).len() == want
            && matches!(
                state_of(vcx, id),
                Some(LoadState::Loaded { stale: false, .. })
            )
    });
    wait_until(vcx, "the coordinator", |vcx| {
        vcx.cx
            .read(|cx| refresh_prompt::coordinator(id, cx))
            .is_some()
    });
    (vcx, src, id)
}

/// The fake server now holds `audit` in `app.public` too (as a DDL run would have made it).
fn server_creates_audit(src: &FakeSource, tables: &[&str]) {
    let mut all: Vec<&str> = tables.to_vec();
    all.push("audit");
    src.intro.put_ns(public(), fake_tables(&all));
}

/// Opens a console on `id` with `sql` and runs it through the normal Run path.
fn open_and_run(vcx: &VisualTestContext, id: ProfileId, sql: &str) -> ConsoleId {
    let before: Vec<ConsoleId> = vcx.cx.read(Consoles::ids);
    in_window(vcx, |window, cx| {
        hacivat_app::console::open_with_text(id, None, sql, true, window, cx)
            .expect("a console opens");
    });
    wait_until(vcx, "the console", |vcx| {
        vcx.cx.read(|cx| {
            Consoles::ids(cx).into_iter().any(|c| {
                !before.contains(&c) && Consoles::get(cx, c).is_some_and(|s| s.editor.is_some())
            })
        })
    });
    let console = vcx.cx.read(|cx| {
        Consoles::ids(cx)
            .into_iter()
            .find(|c| !before.contains(c))
            .expect("the new console")
    });
    wait_ran(vcx, sql);
    wait_until(vcx, "the run ended", |vcx| {
        vcx.cx
            .read(|cx| !hacivat_app::exec_ui::orchestrator::run_busy(console, cx))
    });
    console
}

/// Runs `sql` in `console` (its editor's whole text).
fn run_in(vcx: &VisualTestContext, console: ConsoleId, sql: &str) {
    let editor = vcx.cx.read(|cx| {
        Consoles::get(cx, console)
            .and_then(|s| s.editor.clone())
            .expect("the console's editor")
    });
    in_window(vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            e.set_text(sql, cx);
            e.run_command("editor.run", window, cx);
        });
    });
    wait_ran(vcx, sql);
    wait_until(vcx, "the run ended", |vcx| {
        vcx.cx
            .read(|cx| !hacivat_app::exec_ui::orchestrator::run_busy(console, cx))
    });
}

fn sent(vcx: &VisualTestContext) -> Vec<String> {
    let f = fake(vcx);
    let mut all = f.executed();
    all.extend(f.typed_log().into_iter().filter_map(|c| match c {
        hacivat_driver::fake::FakeTypedCall::Query(sql) => Some(sql),
        _ => None,
    }));
    all
}

fn wait_ran(vcx: &VisualTestContext, sql: &str) {
    wait_until(vcx, sql, |vcx| sent(vcx).iter().any(|q| q.trim() == sql));
}

fn set_preference(vcx: &VisualTestContext, value: &str) {
    vcx.cx
        .update(|cx| rows::apply_choice(cx, rows::REFRESH_AFTER_DDL, value));
    vcx.run_until_parked();
}

fn public_row(vcx: &VisualTestContext) -> ShownRow {
    row(vcx, "public")
}

fn open_public(vcx: &VisualTestContext) {
    set_expanded(vcx, key_of(vcx, "app"));
}

fn toasts(vcx: &VisualTestContext) -> Vec<String> {
    toast_texts(vcx).into_iter().map(|t| strip(&t)).collect()
}

/// Clicks action `i` of the newest visible toast.
fn press_toast_action(vcx: &mut VisualTestContext, i: usize) {
    let id = vcx
        .cx
        .read(ToastQueue::visible_ids)
        .into_iter()
        .max()
        .expect("a toast");
    let selector: &'static str =
        Box::leak(format!("button-toast-action-{id}-{i}").into_boxed_str());
    click(vcx, selector);
}

// ---- the tests (plan Task 27 Step 1) ---------------------------------------------------------

#[gpui::test]
fn create_table_in_a_console_appears_in_the_explorer(cx: &mut TestAppContext) {
    let tables = ["users", "orders"];
    let (vcx, src, id) = connected(cx, &tables);
    server_creates_audit(&src, &tables);
    open_and_run(vcx, id, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "audit in the explorer", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
    });
    assert!(matches!(
        state_of(vcx, id),
        Some(LoadState::Loaded { stale: false, .. })
    ));
    assert_eq!(vcx.cx.read(refresh_prompt::asks), 0, "Always asks nothing");
}

#[gpui::test]
fn ddl_in_an_open_pg_transaction_appears_after_commit(cx: &mut TestAppContext) {
    let tables = ["users"];
    // The run's statements go through the typed path, whose fake follows no transaction: the
    // server's status after each is scripted.
    let in_tx = |prefix: &'static str| hacivat_driver::fake::FakeQuery {
        prefix,
        tx_after: Some(hacivat_driver::TxStatus::InTx),
        ..Default::default()
    };
    let (vcx, src, id) = connected_with(cx, &tables, vec![in_tx("BEGIN"), in_tx("CREATE TABLE")]);
    open_public(vcx);
    let console = open_and_run(vcx, id, "BEGIN");
    wait_until(vcx, "the transaction", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::tx::user_tx(console, cx))
            .is_some()
    });
    server_creates_audit(&src, &tables);
    run_in(vcx, console, "CREATE TABLE public.audit (id int)");
    // The namespace carries the stamp; nothing was read (the metadata connection could not see
    // the uncommitted table anyway, R-M4-42).
    wait_until(vcx, "the COMMIT stamp", |vcx| {
        public_row(vcx).data.meta_mark == Some(hacivat_ui::tree::MetaMark::Running)
    });
    let tip = strip(&public_row(vcx).data.tooltip.unwrap_or_default());
    assert_eq!(
        tip,
        "1 change from this transaction waits for COMMIT. ROLLBACK discards it and the \
         explorer stays as it is."
    );
    assert!(!tables_of(vcx, id).iter().any(|t| t == "audit"), "not yet");
    let log = vcx.cx.read(|cx| {
        Consoles::get(cx, console)
            .expect("the console")
            .log
            .lines()
            .iter()
            .map(|l| l.msg.fluent_id())
            .collect::<Vec<_>>()
    });
    assert!(log.contains(&"hv-metadata-log-pending"), "{log:?}");
    // The toolbar's Commit: the waiting changes are read, the stamp goes.
    let ended = vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::tx::end_tx(console, true, cx));
    assert!(wait_task(vcx, ended), "committed");
    wait_until(vcx, "audit after COMMIT", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
    });
    wait_until(vcx, "the stamp goes", |vcx| {
        public_row(vcx).data.meta_mark.is_none()
    });
}

#[gpui::test]
fn ask_preference_prompts_with_the_count(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected(cx, &tables);
    set_preference(vcx, "ask");
    server_creates_audit(&src, &tables);
    open_and_run(vcx, id, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "the Ask toast", |vcx| {
        vcx.cx.read(refresh_prompt::asks) == 1
    });
    let shown = toasts(vcx);
    // The count is the coordinator target count (T21): 2 for this one CREATE TABLE.
    assert!(
        shown
            .iter()
            .any(|t| t == "Refresh the database explorer to reflect 2 changed objects?"),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .any(|t| t == "Your script in app-dev changed them."),
        "{shown:?}"
    );
    // Nothing was refreshed: the namespace is marked out of date meanwhile.
    assert!(!tables_of(vcx, id).iter().any(|t| t == "audit"));
    assert!(matches!(
        state_of(vcx, id),
        Some(LoadState::Loaded { stale: true, .. })
    ));
    // A run is never blocked by the question: another statement runs while it is shown.
    let other = open_and_run(vcx, id, "SELECT 1");
    let _ = other;
    // Refresh answers it.
    press_toast_action(vcx, 0);
    wait_until(vcx, "audit after Refresh", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
            && matches!(
                state_of(vcx, id),
                Some(LoadState::Loaded { stale: false, .. })
            )
    });
}

#[gpui::test]
fn always_refresh_in_the_ask_toast_sets_the_preference(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected(cx, &tables);
    set_preference(vcx, "ask");
    server_creates_audit(&src, &tables);
    open_and_run(vcx, id, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "the Ask toast", |vcx| {
        vcx.cx.read(refresh_prompt::asks) == 1
    });
    press_toast_action(vcx, 1);
    let pref = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::AppState>()
            .settings
            .metadata
            .refresh_after_ddl
    });
    assert_eq!(pref, RefreshAfterDdl::Always);
    wait_until(vcx, "audit", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
    });
}

#[gpui::test]
fn never_preference_shows_the_stale_badge(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected(cx, &tables);
    set_preference(vcx, "never");
    open_public(vcx);
    server_creates_audit(&src, &tables);
    open_and_run(vcx, id, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "the out-of-date badge", |vcx| {
        public_row(vcx).data.badge.as_deref() == Some("out of date")
    });
    assert!(!tables_of(vcx, id).iter().any(|t| t == "audit"));
    assert_eq!(vcx.cx.read(refresh_prompt::asks), 0, "Never asks nothing");
    // Review probe "a stale badge that never clears": a refresh clears it.
    put_cursor(vcx, key_of(vcx, "public"));
    assert!(run(vcx, "explorer.refresh"));
    wait_until(vcx, "the badge clears", |vcx| {
        public_row(vcx).data.badge.is_none() && tables_of(vcx, id).iter().any(|t| t == "audit")
    });
}

#[gpui::test]
fn dynamic_sql_shows_the_broad_refresh_notice(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected(cx, &tables);
    server_creates_audit(&src, &tables);
    open_and_run(
        vcx,
        id,
        "DO $$ BEGIN EXECUTE 'CREATE TABLE public.audit (id int)'; END $$",
    );
    wait_until(vcx, "the notice", |vcx| {
        toasts(vcx)
            .iter()
            .any(|t| t.starts_with("Refreshing all of "))
    });
    let shown = toasts(vcx);
    assert!(
        shown.iter().any(|t| t
            == "The script ran dynamic SQL (DO), so Hacivat can't tell which objects changed."),
        "{shown:?}"
    );
    // The whole schema is read: the new table shows.
    wait_until(vcx, "audit", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
    });
}

// ---- carries -------------------------------------------------------------------------------

/// T21 carry: the badge of a schema list marked stale (`CoordEvent::Stale { scope: true }`).
#[gpui::test]
fn never_preference_badges_a_stale_schema_list(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, _src, id) = connected(cx, &tables);
    set_preference(vcx, "never");
    open_and_run(vcx, id, "CREATE SCHEMA audit");
    wait_until(vcx, "the database row's badge", |vcx| {
        row(vcx, "app").data.badge.as_deref() == Some("out of date")
    });
    let tip = strip(&row(vcx, "app").data.tooltip.unwrap_or_default());
    assert_eq!(
        tip,
        "The server changed app after it was read. Refresh to load the changes."
    );
}

/// R-W4-8: a deletion held for a `Partial` namespace keeps its badge and its count.
#[gpui::test]
fn a_held_deletion_badges_a_partial_namespace(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..30).map(|i| format!("t{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (vcx, src, id) = connected(cx, &refs);
    open_public(vcx);
    src.intro.edit_ns(&public(), |ns| ns.objects.truncate(5));
    let h = vcx.cx.read(|cx| hlive::handle(id, cx)).expect("the engine");
    drop(h.refresh(
        hacivat_introspect::ddl_effects::Target::Namespace(public()),
        hacivat_meta::merge::Trigger::Background,
    ));
    wait_until(vcx, "the held deletion", |vcx| {
        vcx.cx
            .read(|cx| hlive::pending(id, cx))
            .get(&public())
            .copied()
            == Some(25)
    });
    // The namespace becomes Partial (a facet the login reads partly), the deletion still held.
    vcx.cx.update(|cx| {
        let cat = adapter::catalog_of(id, cx).expect("the catalog");
        let mut states = (*cat.states).clone();
        states.insert(
            ContainerKey::Namespace(public()),
            LoadState::Partial {
                reason: hacivat_meta::snapshot::IntroErrorCode::PermissionDenied,
            },
        );
        let mut next = (*cat).clone();
        next.states = Arc::new(states);
        next.generation += 1;
        adapter::set_catalog(id, Some(Arc::new(next)), None, cx);
    });
    let r = public_row(vcx);
    assert_eq!(r.data.badge.as_deref(), Some("out of date"));
    assert_eq!(
        strip(&r.data.tooltip.unwrap_or_default()),
        "25 objects disappeared in the last background check. Refresh public to review them; \
         nothing was removed."
    );
}

/// T25 m-9: Reload in the DDL view of an object whose namespace is stale reads the namespace
/// again, so a decided verdict can show the text as current.
#[gpui::test]
fn ddl_view_reload_reads_a_stale_namespace_again(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, _src, id) = connected(cx, &tables);
    let h = vcx.cx.read(|cx| hlive::handle(id, cx)).expect("the engine");
    h.mark_stale(vec![public()], false);
    wait_until(vcx, "stale", |vcx| {
        matches!(
            state_of(vcx, id),
            Some(LoadState::Loaded { stale: true, .. })
        )
    });
    let catalog = hacivat_app::ddl_view::source::EngineCatalog { profile: id };
    let key = vcx
        .cx
        .read(|cx| {
            adapter::catalog_of(id, cx).and_then(|c| {
                let ns = c.namespaces.get(&public())?.clone();
                ns.children(0)
                    .find(|(_, o)| o.name().as_ref() == "users")
                    .and_then(|(i, _)| ns.object_key(i))
            })
        })
        .expect("users");
    let ticket = vcx
        .cx
        .read(|cx| hacivat_app::ddl_view::source::DdlCatalog::refresh(&catalog, key, cx));
    let rt = runtime(vcx);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = done.clone();
    rt.spawn(async move {
        let _ = ticket.await;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    wait_until(vcx, "the reload", |_| {
        done.load(std::sync::atomic::Ordering::SeqCst)
    });
    wait_until(vcx, "the namespace current again", |vcx| {
        matches!(
            state_of(vcx, id),
            Some(LoadState::Loaded { stale: false, .. })
        )
    });
}

/// T24 residual nit: a queued Open-data run is dropped when its connect ends disconnected, so a
/// later connect never runs it.
#[gpui::test]
fn a_queued_run_is_dropped_when_its_connect_ends_disconnected(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(
        cx,
        "en",
        answers_probe(FakeDriver::new(DbmsKind::Postgres)).gated(),
    );
    in_window(vcx, |window, _| window.activate_window());
    let id = add_profile(vcx, pg_profile("app-dev"));
    // Held at the driver: the connect is under way while the run waits in the queue.
    connect(vcx, "app-dev");
    wait_until(vcx, "connecting", |vcx| {
        matches!(
            live(vcx, "app-dev"),
            hacivat_session::LiveState::Connecting(_)
        )
    });
    in_window(vcx, |window, cx| {
        hacivat_app::console::open_with_text(id, None, "SELECT 1", true, window, cx)
            .expect("a console opens");
    });
    vcx.run_until_parked();
    assert_eq!(
        vcx.cx.read(hacivat_app::console::commands::queued_runs),
        1,
        "the run waits for the connect"
    );
    vcx.cx.update(|cx| ConnectionsModel::cancel_connect(cx, id));
    wait_until(vcx, "the connect ended", |vcx| {
        !matches!(
            live(vcx, "app-dev"),
            hacivat_session::LiveState::Connecting(_)
        )
    });
    wait_until(vcx, "the queued run dropped", |vcx| {
        vcx.cx.read(hacivat_app::console::commands::queued_runs) == 0
    });
    // A later connect runs nothing.
    fake(vcx).release();
    connect(vcx, "app-dev");
    wait_until(vcx, "connected", |vcx| {
        matches!(
            live(vcx, "app-dev"),
            hacivat_session::LiveState::Connected(_)
        )
    });
    vcx.run_until_parked();
    assert!(
        !sent(vcx).iter().any(|q| q.trim() == "SELECT 1"),
        "{:?}",
        sent(vcx)
    );
}

/// Mock 11: an object a state check found changed (and could not read again) says so in the
/// explorer until its namespace is read again; one found gone says it no longer exists.
#[gpui::test]
fn a_changed_object_says_so_in_the_explorer_until_read_again(cx: &mut TestAppContext) {
    use hacivat_introspect::introspector::Staleness;
    let tables = ["users", "orders"];
    let (vcx, _src, id) = connected(cx, &tables);
    open_public(vcx);
    set_expanded(vcx, key_of(vcx, "public"));
    set_expanded(vcx, key_of(vcx, "TABLES"));
    let key_named = |vcx: &VisualTestContext, name: &str| {
        vcx.cx
            .read(|cx| {
                adapter::catalog_of(id, cx).and_then(|c| {
                    let ns = c.namespaces.get(&public())?.clone();
                    ns.children(0)
                        .find(|(_, o)| o.name().as_ref() == name)
                        .and_then(|(i, _)| ns.object_key(i))
                })
            })
            .expect("the object")
    };
    let users = key_named(vcx, "users");
    let orders = key_named(vcx, "orders");
    vcx.cx.update(|cx| {
        let snap = adapter::catalog_of(id, cx).expect("the catalog");
        hacivat_app::explorer::stale::record(id, &users, Staleness::Changed, false, &snap, cx);
        hacivat_app::explorer::stale::record(id, &orders, Staleness::Dropped, false, &snap, cx);
    });
    let tip = |vcx: &VisualTestContext, name: &str| row(vcx, name).data.tooltip.map(|t| strip(&t));
    assert_eq!(
        tip(vcx, "users").as_deref(),
        Some("users changed on the server since it was loaded")
    );
    assert_eq!(
        tip(vcx, "orders").as_deref(),
        Some("orders no longer exists on the server")
    );
    // A re-read that succeeded marks nothing.
    vcx.cx.update(|cx| {
        let snap = adapter::catalog_of(id, cx).expect("the catalog");
        hacivat_app::explorer::stale::record(id, &orders, Staleness::Changed, true, &snap, cx);
    });
    assert_eq!(tip(vcx, "orders"), None);
    // The namespace read again: the mark no longer describes what is shown.
    put_cursor(vcx, key_of(vcx, "public"));
    assert!(run(vcx, "explorer.refresh"));
    wait_until(vcx, "the tooltip goes", |vcx| tip(vcx, "users").is_none());
}
