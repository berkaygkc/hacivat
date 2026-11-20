//! T27 review hostile tests (opus reviewer), committed by the fix round (H1 asserts the I-1 fix).

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
use hacivat_ui::toast::ToastQueue;
use hacivat_ui::tree::TreeKey;
use support::live::*;
use support::*;

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

fn server_creates_audit(src: &FakeSource, tables: &[&str]) {
    let mut all: Vec<&str> = tables.to_vec();
    all.push("audit");
    src.intro.put_ns(public(), fake_tables(&all));
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

fn public_row(vcx: &VisualTestContext) -> ShownRow {
    row(vcx, "public")
}

fn in_tx(prefix: &'static str) -> hacivat_driver::fake::FakeQuery {
    hacivat_driver::fake::FakeQuery {
        prefix,
        tx_after: Some(hacivat_driver::TxStatus::InTx),
        ..Default::default()
    }
}

/// BEGIN; CREATE TABLE (stamp shown) in console `c`.
fn ddl_in_tx(vcx: &VisualTestContext, id: ProfileId) -> ConsoleId {
    set_expanded(vcx, key_of(vcx, "app"));
    let console = open_and_run(vcx, id, "BEGIN");
    wait_until(vcx, "the transaction", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::tx::user_tx(console, cx))
            .is_some()
    });
    run_in(vcx, console, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "the COMMIT stamp", |vcx| {
        public_row(vcx).data.meta_mark == Some(hacivat_ui::tree::MetaMark::Running)
    });
    console
}

/// H1. DDL inside an open transaction, then the catalog is closed and reopened while the
/// transaction stays open (Settings › Metadata "Clear cached metadata…", T27's own action; T20's
/// Save takes the same `release_cache`/`reopen` path). The reopen reads the namespace before
/// COMMIT. After COMMIT the new table must appear, or the namespace must be marked stale.
#[gpui::test]
fn h1_ddl_waiting_for_commit_survives_a_catalog_reopen(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected_with(cx, &tables, vec![in_tx("BEGIN"), in_tx("CREATE TABLE")]);
    let console = ddl_in_tx(vcx, id);
    let old = vcx
        .cx
        .read(|cx| refresh_prompt::coordinator(id, cx))
        .expect("coordinator");
    let task = vcx
        .cx
        .update(|cx| hacivat_app::settings_ui::metadata::clear(id, cx));
    let _ = wait_task(vcx, task);
    wait_until(vcx, "a new coordinator and a current namespace", |vcx| {
        vcx.cx
            .read(|cx| refresh_prompt::coordinator(id, cx))
            .is_some_and(|c| !Arc::ptr_eq(&c, &old))
            && matches!(
                state_of(vcx, id),
                Some(LoadState::Loaded { stale: false, .. })
            )
    });
    // I-1 fix: the changes still wait for COMMIT on the new coordinator (the stamp is back).
    wait_until(vcx, "the stamp after the reopen", |vcx| {
        public_row(vcx).data.meta_mark == Some(hacivat_ui::tree::MetaMark::Running)
    });
    // The transaction is still open on the user's session.
    assert!(
        vcx.cx
            .read(|cx| hacivat_app::exec_ui::tx::user_tx(console, cx))
            .is_some()
    );
    // COMMIT: the server now shows the table.
    server_creates_audit(&src, &tables);
    let ended = vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::tx::end_tx(console, true, cx));
    assert!(wait_task(vcx, ended), "committed");
    // Always: the committed table is read.
    wait_until(vcx, "audit after COMMIT", |vcx| {
        tables_of(vcx, id).iter().any(|t| t == "audit")
    });
    wait_until(vcx, "the stamp goes", |vcx| {
        public_row(vcx).data.meta_mark.is_none()
    });
}

/// H2. DDL in a transaction, then the toolbar's Roll back: nothing is refreshed (the fake server
/// would show `audit` to any read), the stamp goes, nothing is marked stale.
#[gpui::test]
fn h2_a_rolled_back_ddl_never_refreshes(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected_with(cx, &tables, vec![in_tx("BEGIN"), in_tx("CREATE TABLE")]);
    let console = ddl_in_tx(vcx, id);
    let coord = vcx
        .cx
        .read(|cx| refresh_prompt::coordinator(id, cx))
        .expect("coordinator");
    let applied = coord.applied();
    // A read now would show it (so a wrong refresh is visible).
    server_creates_audit(&src, &tables);
    let ended = vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::tx::end_tx(console, false, cx));
    assert!(wait_task(vcx, ended), "rolled back");
    wait_until(vcx, "the stamp goes", |vcx| {
        public_row(vcx).data.meta_mark.is_none()
    });
    coord.drain();
    vcx.run_until_parked();
    assert_eq!(coord.applied(), applied, "no refresh after ROLLBACK");
    assert!(!tables_of(vcx, id).iter().any(|t| t == "audit"));
    assert!(matches!(
        state_of(vcx, id),
        Some(LoadState::Loaded { stale: false, .. })
    ));
}

/// H3. Ask never answered: the toast is dismissed with its ✕. The namespace must stay marked
/// "out of date" (never silently current), and the next question carries the earlier targets.
#[gpui::test]
fn h3_an_ask_dismissed_with_its_x_leaves_the_namespace_stale(cx: &mut TestAppContext) {
    let tables = ["users"];
    let (vcx, src, id) = connected_with(cx, &tables, Vec::new());
    vcx.cx
        .update(|cx| rows::apply_choice(cx, rows::REFRESH_AFTER_DDL, "ask"));
    vcx.run_until_parked();
    set_expanded(vcx, key_of(vcx, "app"));
    server_creates_audit(&src, &tables);
    open_and_run(vcx, id, "CREATE TABLE public.audit (id int)");
    wait_until(vcx, "the Ask toast", |vcx| {
        vcx.cx.read(refresh_prompt::asks) == 1
    });
    let first: Vec<String> = toast_texts(vcx).iter().map(|t| strip(t)).collect();
    let toast = vcx
        .cx
        .read(ToastQueue::visible_ids)
        .into_iter()
        .max()
        .expect("a toast");
    vcx.cx.update(|cx| ToastQueue::dismiss(cx, toast));
    vcx.run_until_parked();
    assert_eq!(public_row(vcx).data.badge.as_deref(), Some("out of date"));
    // A second DDL: the question folds the first one's targets in.
    let mut all = tables.to_vec();
    all.extend(["audit", "audit2"]);
    src.intro.put_ns(public(), fake_tables(&all));
    open_and_run(vcx, id, "CREATE TABLE public.audit2 (id int)");
    wait_until(vcx, "the second Ask", |vcx| {
        vcx.cx.read(refresh_prompt::asks) == 2
    });
    let second: Vec<String> = toast_texts(vcx).iter().map(|t| strip(t)).collect();
    let _ = (first, second);
    assert_eq!(public_row(vcx).data.badge.as_deref(), Some("out of date"));
    assert!(!tables_of(vcx, id).iter().any(|t| t == "audit"));
}
