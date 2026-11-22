//! M4b Task 14: script tabs (HV-SCRIPT-06, -07, -08). Where you open a script decides where it
//! runs: a tab is bound to the (connection, database) it was opened under for its whole life,
//! runs only there, saves only on ⌘S, follows the file on disk and survives a restart.
//!
//! Each connection has its own `FakeDriver` (`support::shell_with_routes`, routed by host), so a
//! test can tell which connection a statement reached. The library service runs on the app's
//! core runtime over a temp library, with a fake event source the test drives (deterministic),
//! except `external_edit_appears_without_reload`, which uses the OS watcher (10 s ceiling).
//! Keys go through `support::press_key`; window text through `support::full_frame` (the script
//! editor is a cached view).

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, ConsoleSource, Consoles};
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::tabs::{self, Load, Notice, RunRefused, ScriptDoc, ScriptTab};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_app::shell::TabKind;
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::service::{LibraryService, ServiceOptions, Sources};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_scripts::{LogicalDb, RawHint};
use hacivat_session::profile::Profile;
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "10.0.0.1";
const STAGING: &str = "10.0.0.2";
const MYSQL: &str = "10.0.0.3";

// ---- fakes ------------------------------------------------------------------------------------

fn answer(prefix: &'static str, cells: &[Option<&str>]) -> FakeExec {
    FakeExec {
        prefix,
        result: Ok(ExecSummary {
            rows: vec![cells.iter().map(|c| c.map(str::to_string)).collect()],
            row_count: 1,
            ..ExecSummary::default()
        }),
        delay: Duration::ZERO,
        tx_after: None,
        panics: false,
    }
}

/// A PostgreSQL fake on database `db`: the SQL-context probe, the read-only guard's canary and
/// the schema probe, answered as a server does (many times: every session and re-probe).
fn pg_fake(db: &'static str) -> FakeDriver {
    let exec = (0..48)
        .flat_map(|_| {
            [
                answer("SHOW standard_conforming_strings", &[Some("on")]),
                answer("SELECT pg_catalog.txid_current_if_assigned()", &[None]),
                answer(
                    "SELECT current_database()",
                    &[
                        Some(db),
                        Some("public"),
                        Some("public"),
                        Some("deploy"),
                        Some("public"),
                    ],
                ),
            ]
        })
        .collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(exec)
        .queries(scs_queries())
}

/// The typed form of the SQL-context probe (read-only sessions run it inside the guard).
fn scs_queries() -> Vec<FakeQuery> {
    (0..48)
        .map(|_| FakeQuery {
            prefix: "SHOW standard_conforming_strings",
            sets: vec![FakeResultSet {
                columns: vec![(
                    "standard_conforming_strings",
                    hacivat_values::LogicalType::Text {
                        max_chars: None,
                        fixed: false,
                    },
                )],
                rows: FakeRows::Literal(vec![vec![hacivat_values::Value::Text("on".into())]]),
            }],
            ..Default::default()
        })
        .collect()
}

/// A MySQL fake whose schema probes answer `dbs` in order (the last one again after that).
fn mysql_fake(dbs: &[&'static str]) -> FakeDriver {
    let mut exec: Vec<FakeExec> = (0..48)
        .map(|_| answer("SELECT @@SESSION.sql_mode", &[Some("")]))
        .collect();
    let last = *dbs.last().expect("a database");
    for db in dbs.iter().chain(std::iter::repeat_n(&last, 32)) {
        exec.push(answer("SELECT DATABASE()", &[Some(db), Some("deploy@%")]));
    }
    FakeDriver::new(DbmsKind::MySql)
        .tracks_transactions()
        .exec(exec)
}

fn pg_profile(name: &str, env: EnvTag, host: &str, db: &str) -> Profile {
    let mut p = test_profile(name, Some(env), host);
    p.database = Some(db.to_string());
    p
}

fn mysql_profile(name: &str, env: EnvTag, db: &str) -> Profile {
    let mut p = test_profile(name, Some(env), MYSQL);
    p.engine = DbmsKind::MySql;
    p.database = Some(db.to_string());
    p
}

// ---- the rig ----------------------------------------------------------------------------------

/// A shell, its fakes, and one library `orders` attached under a temp folder.
struct Rig {
    vcx: &'static mut VisualTestContext,
    fakes: Vec<Arc<FakeDriver>>,
    service: Arc<LibraryService>,
    watch: hacivat_scripts::testing::FakeHandle,
    lib: LibraryId,
    root: PathBuf,
}

impl Drop for Rig {
    /// The service stops with the test, so the core runtime's shutdown never waits for it.
    fn drop(&mut self) {
        self.service.shutdown();
    }
}

fn opts() -> ServiceOptions {
    ServiceOptions {
        settle: Duration::from_millis(20),
        git_debounce: Duration::from_millis(40),
        recheck: Duration::from_millis(60),
        home: None,
        ..ServiceOptions::default()
    }
}

fn rig_with(
    cx: &mut TestAppContext,
    routes: Vec<(DbmsKind, &'static str, FakeDriver)>,
    files: &[(&str, &str)],
    os_watcher: bool,
) -> Rig {
    let (vcx, fakes) = shell_with_routes(cx, "en", routes);
    in_window(vcx, |window, _| window.activate_window());
    let root = temp_root().join("orders");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    let root = std::fs::canonicalize(&root).unwrap();
    let paths = paths(vcx);
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (_, watch) = FakeEventSource::new();
    let sources = if os_watcher {
        Sources::Os
    } else {
        watch.sources()
    };
    let service = Arc::new(LibraryService::open_with(&paths, &rt, sources, opts()));
    {
        let service = service.clone();
        vcx.cx.update(|cx| install_service(service, cx));
    }
    let lib = futures::executor::block_on(service.attach(
        root.clone(),
        Some(LogicalDb::parse("orders").unwrap()),
        false,
    ))
    .expect("the library attaches");
    let n = files.len();
    wait_until(vcx, "the library is listed", |_| {
        let snap = service.snapshot();
        snap.view(Some(EnvTag::Production))
            .library(lib)
            .is_some_and(|l| {
                l.entries()
                    .iter()
                    .filter(|e| e.kind != hacivat_scripts::EntryKind::Folder)
                    .count()
                    + hidden(files)
                    >= n
            })
    });
    Rig {
        vcx,
        fakes,
        service,
        watch,
        lib,
        root,
    }
}

/// Files the production view does not show (other `@env` folders).
fn hidden(files: &[(&str, &str)]) -> usize {
    files
        .iter()
        .filter(|(rel, _)| {
            rel.split('/')
                .any(|c| c.starts_with('@') && c != "@production")
        })
        .count()
}

/// Prod (PostgreSQL, database `orders`) and staging (the same), each with its own fake.
fn prod_staging(cx: &mut TestAppContext, files: &[(&str, &str)]) -> (Rig, ProfileId, ProfileId) {
    let r = rig_with(
        cx,
        vec![
            (DbmsKind::Postgres, PROD, pg_fake("orders")),
            (DbmsKind::Postgres, STAGING, pg_fake("orders")),
        ],
        files,
        false,
    );
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    let staging = add_profile(
        r.vcx,
        pg_profile("orders-staging", EnvTag::Staging, STAGING, "orders"),
    );
    (r, prod, staging)
}

impl Rig {
    fn rel(&self, s: &str) -> RelPath {
        RelPath::parse(s).unwrap()
    }

    /// Opens `rel` under `profile` and `db` and waits until its tab shows the file; its tab.
    fn open(&mut self, rel: &str, profile: Option<ProfileId>, db: Option<&str>) -> TabId {
        let (lib, rel_path) = (self.lib, self.rel(rel));
        self.vcx.cx.update(|cx| {
            open_script(cx, lib, rel_path, profile, db.map(DatabaseName::new));
        });
        let tab = self.active();
        wait_until(self.vcx, "the script tab shows its file", |vcx| {
            redraw_full(vcx);
            doc(vcx, tab).is_some_and(|d| vcx.cx.read(|cx| *d.read(cx).load() == Load::Ready))
        });
        tab
    }

    fn active(&self) -> TabId {
        self.vcx.run_until_parked();
        let ws = workspace(self.vcx);
        self.vcx
            .cx
            .read(|cx| ws.read(cx).active_tab())
            .expect("a tab is active")
    }

    fn doc(&self, tab: TabId) -> Entity<ScriptDoc> {
        doc(self.vcx, tab).expect("a script doc")
    }

    fn console(&self, tab: TabId) -> ConsoleId {
        self.vcx
            .cx
            .read(|cx| Consoles::by_tab(cx, tab))
            .expect("the tab's console")
    }

    fn editor(&self, tab: TabId) -> Entity<EditorView> {
        let console = self.console(tab);
        self.vcx
            .cx
            .read(|cx| Consoles::get(cx, console).and_then(|s| s.editor.clone()))
            .expect("an editor")
    }

    fn text(&self, tab: TabId) -> String {
        let editor = self.editor(tab);
        self.vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string())
    }

    fn spec(&self, tab: TabId) -> ScriptTab {
        let doc = self.doc(tab);
        self.vcx
            .cx
            .read(|cx| doc.read(cx).spec(cx))
            .expect("a binding")
    }

    fn focus(&self, tab: TabId) {
        let ws = workspace(self.vcx);
        in_window(self.vcx, |window, cx| {
            ws.update(cx, |w, cx| w.focus_tab(tab, cx));
            let editor = Consoles::by_tab(cx, tab)
                .and_then(|c| Consoles::get(cx, c))
                .and_then(|s| s.editor.clone());
            if let Some(editor) = editor {
                let f = gpui::Focusable::focus_handle(editor.read(cx), cx);
                window.focus(&f, cx);
            }
        });
        self.vcx.run_until_parked();
    }

    /// Types `s` at the end of tab `tab`'s text.
    fn type_text(&self, tab: TabId, s: &str) {
        self.focus(tab);
        let editor = self.editor(tab);
        let s = s.to_string();
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| {
                let len = e.snapshot().buffer.len_bytes();
                e.select_range(len..len, cx);
                e.replace_text_in_range(None, &s, window, cx);
            });
        });
        self.vcx.run_until_parked();
    }

    fn dirty(&self, tab: TabId) -> bool {
        let doc = self.doc(tab);
        self.vcx.cx.read(|cx| doc.read(cx).is_dirty(cx))
    }

    fn notices(&self, tab: TabId) -> Vec<Notice> {
        let doc = self.doc(tab);
        self.vcx.cx.read(|cx| doc.read(cx).notices().to_vec())
    }

    /// Runs the editor's Run on tab `tab`.
    fn run(&self, tab: TabId) {
        self.focus(tab);
        let editor = self.editor(tab);
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| {
                e.run_command("editor.run", window, cx);
            });
        });
        self.vcx.run_until_parked();
    }

    /// Saves tab `tab` (`file.save` in its context).
    fn save(&self, tab: TabId) {
        self.focus(tab);
        assert!(run(self.vcx, "file.save"), "file.save ran");
        self.vcx.run_until_parked();
    }

    /// Tells the service `rel` changed on disk (as the OS watcher would).
    fn hint(&self, rel: &str) {
        let path = self.root.join(rel);
        assert!(self.watch.emit(&self.root, vec![RawHint::Path(path)]));
    }

    fn write_file(&self, rel: &str, text: &str) {
        std::fs::write(self.root.join(rel), text).unwrap();
        self.hint(rel);
    }

    fn session_db(&self, tab: TabId) -> Option<String> {
        let console = self.console(tab);
        self.vcx.cx.read(|cx| {
            let s = Consoles::get(cx, console)?.session.clone()?;
            let state = s.state();
            let snap = state.borrow();
            snap.schema.as_ref()?.database.clone()
        })
    }

    fn wait_session(&self, tab: TabId, what: &str) {
        let console = self.console(tab);
        wait_until(self.vcx, what, |vcx| {
            vcx.cx.read(|cx| {
                Consoles::get(cx, console).is_some_and(|s| s.session.is_some())
                    && hacivat_app::exec_ui::sql_context(console, cx)
                        != hacivat_app::exec_ui::context::SqlContext::Pending
            })
        });
    }
}

/// Whether a dialog titled `title` is open.
fn has_dialog(vcx: &VisualTestContext, title: &str) -> bool {
    layer_ids(vcx).contains(&"dialog") && dialog_title(vcx) == title
}

/// A debug selector built at run time (`handle_of` takes a `'static` one).
fn selector(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn doc(vcx: &VisualTestContext, tab: TabId) -> Option<Entity<ScriptDoc>> {
    vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx))
}

/// The statements `fake` ran as typed queries (Run's units), in order.
fn queries(fake: &FakeDriver) -> Vec<String> {
    fake.typed_log()
        .into_iter()
        .filter_map(|c| match c {
            FakeTypedCall::Query(sql) => Some(sql),
            _ => None,
        })
        .filter(|sql| !sql.starts_with("SHOW standard_conforming_strings"))
        .collect()
}

/// The capsule's connection name and environment.
fn capsule(vcx: &VisualTestContext) -> Option<(String, EnvKind)> {
    match capsule_model(vcx) {
        CapsuleModel::Connection { name, env, .. }
        | CapsuleModel::Connecting { name, env, .. }
        | CapsuleModel::Idle { name, env, .. }
        | CapsuleModel::Failed { name, env, .. }
        | CapsuleModel::Reconnecting { name, env, .. } => Some((name.to_string(), env)),
        CapsuleModel::Placeholder { .. } => None,
    }
}

fn connected(r: &Rig, name: &str) {
    connect(r.vcx, name);
    wait_until(r.vcx, "connected", |vcx| {
        matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
    });
}

// ---- where it runs (gate E1, app half) ---------------------------------------------------------

#[gpui::test]
fn tab_runs_on_the_connection_it_was_opened_under(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    connected(&r, "orders-prod");
    connected(&r, "orders-staging");
    let on_prod = r.open("a.sql", Some(prod), Some("orders"));
    let on_staging = r.open("a.sql", Some(staging), Some("orders"));
    assert_ne!(on_prod, on_staging, "two tabs, one per connection");
    assert_eq!(r.spec(on_prod).profile, Some(prod));
    assert_eq!(r.spec(on_staging).profile, Some(staging));
    r.wait_session(on_prod, "the prod tab's session");
    r.wait_session(on_staging, "the staging tab's session");
    r.run(on_prod);
    wait_until(r.vcx, "prod ran it", |_| {
        queries(&r.fakes[0]) == ["SELECT 1"]
    });
    assert!(queries(&r.fakes[1]).is_empty(), "staging saw nothing yet");
    r.run(on_staging);
    wait_until(r.vcx, "staging ran it", |_| {
        queries(&r.fakes[1]) == ["SELECT 1"]
    });
    assert_eq!(
        queries(&r.fakes[0]),
        ["SELECT 1"],
        "prod saw exactly its own"
    );
    // The capsule follows the focused tab, with each connection's environment.
    r.focus(on_prod);
    redraw_full(r.vcx);
    assert_eq!(
        capsule(r.vcx),
        Some(("orders-prod".to_string(), EnvKind::Production))
    );
    r.focus(on_staging);
    redraw_full(r.vcx);
    assert_eq!(
        capsule(r.vcx),
        Some(("orders-staging".to_string(), EnvKind::Staging))
    );
    // The titles name the connection.
    assert_eq!(tab_title(r.vcx), "a.sql · orders-staging");
}

#[gpui::test]
fn reopening_focuses_the_existing_tab(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    assert!(run(r.vcx, "workspace.new_console_tab"));
    let count = tab_count(r.vcx);
    assert_ne!(r.active(), tab);
    let again = r.open("a.sql", Some(prod), None);
    assert_eq!(again, tab, "the same file and connection focus the tab");
    assert_eq!(tab_count(r.vcx), count, "no second buffer");
    // R-T1b-1: a tab opened without a database stores the one the profile runs on.
    assert_eq!(
        r.spec(tab).database,
        Some(DatabaseName::new("orders")),
        "the database is resolved at open, never None"
    );
}

#[gpui::test]
fn two_tabs_same_file_two_connections(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let a = r.open("a.sql", Some(prod), Some("orders"));
    let b = r.open("a.sql", Some(staging), Some("orders"));
    // A save in one is an external change for the other: a clean tab reloads.
    r.type_text(a, ";\nSELECT 2");
    r.save(a);
    wait_until(r.vcx, "a saved", |_| !r.dirty(a));
    wait_until(r.vcx, "b reloaded", |_| r.text(b) == "SELECT 1;\nSELECT 2");
    assert!(!r.dirty(b));
    // A dirty tab gets the bar, and keeps its text.
    r.type_text(b, " -- mine");
    r.type_text(a, ";\nSELECT 3");
    r.save(a);
    wait_until(r.vcx, "b's bar", |_| {
        r.notices(b).contains(&Notice::Changed)
    });
    assert_eq!(r.text(b), "SELECT 1;\nSELECT 2 -- mine");
}

#[gpui::test]
fn no_path_rebinds_a_script_tab(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    connected(&r, "orders-staging");
    let tab = r.open("a.sql", Some(staging), Some("orders"));
    let before = r.spec(tab);
    let console = r.console(tab);
    // Every command a script tab's context can run leaves its binding as it is (app-level
    // commands that would end the test are left out).
    let skip = [
        "app.quit",
        "app.restart",
        "app.hide",
        "app.hide_others",
        "workspace.close_window",
        "workspace.close_tab",
        "workspace.new_window",
        "app.show_menu",
    ];
    let ids: Vec<&'static str> = r.vcx.cx.read(|cx| {
        cx.global::<hacivat_ui::actions::ActionRegistry>()
            .all()
            .iter()
            .map(|d| d.id)
            .collect()
    });
    for id in ids {
        if skip.contains(&id) {
            continue;
        }
        r.focus(tab);
        let _ = run(r.vcx, id);
        r.vcx.run_until_parked();
        // Dismiss whatever the command opened.
        for _ in 0..3 {
            if layer_ids(r.vcx).is_empty() {
                break;
            }
            press_key(r.vcx, "escape");
        }
    }
    assert_eq!(
        r.spec(tab),
        before,
        "the handle still names the same binding"
    );
    let profile = r
        .vcx
        .cx
        .read(|cx| Consoles::get(cx, console).map(|s| s.profile));
    assert!(
        matches!(profile, Some(Some(p)) if p == staging) || profile.is_none(),
        "the console is bound to staging, or gone with its tab: {profile:?}"
    );
    // "Open under another connection…" offers prod (the file's other place) and never rebinds
    // this tab: production is never chosen for the user, so it waits for the chooser.
    let others = r
        .vcx
        .cx
        .read(|cx| hacivat_app::scripts::open::other_places(&before, cx));
    assert_eq!(others, [(prod, DatabaseName::new("orders"))]);
    let count = tab_count(r.vcx);
    r.vcx
        .cx
        .update(|cx| hacivat_app::scripts::open::open_under_another(&before, cx));
    r.vcx.run_until_parked();
    assert_eq!(
        tab_count(r.vcx),
        count,
        "production is never opened for the user"
    );
    assert_eq!(r.spec(tab), before);
    // Opened under prod on purpose, it is a second tab; the first stays on staging.
    let on_prod = r.open("a.sql", Some(prod), Some("orders"));
    assert_ne!(on_prod, tab);
    assert_eq!(r.spec(tab).profile, Some(staging));
    assert_eq!(r.spec(on_prod).profile, Some(prod));
}

#[gpui::test]
fn unbound_script_tab_is_never_bound_by_a_connect(cx: &mut TestAppContext) {
    let (mut r, _, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", None, None);
    assert_eq!(r.spec(tab).profile, None);
    assert_eq!(tab_title(r.vcx), "a.sql · no connection");
    // Focused while the user connects prod: it stays unbound (R-M4b-30).
    connected(&r, "orders-prod");
    redraw_full(r.vcx);
    assert_eq!(r.spec(tab).profile, None);
    let console = r.console(tab);
    let state = r
        .vcx
        .cx
        .read(|cx| Consoles::get(cx, console).map(|s| (s.profile, s.session.is_some())));
    assert_eq!(state, Some((None, false)), "no profile, no session");
    // Run does nothing anywhere, and the capsule names no connection for it.
    r.run(tab);
    settle(r.vcx, Duration::from_millis(50));
    assert!(queries(&r.fakes[0]).is_empty());
    assert!(queries(&r.fakes[1]).is_empty());
    assert_eq!(capsule(r.vcx), None, "the capsule shows no connection");
    let doc = r.doc(tab);
    assert!(!r.vcx.cx.read(|cx| doc.read(cx).run_enabled(cx)));
}

#[gpui::test]
fn markdown_tab_has_run_disabled(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("notes.md", "SELECT 1")]);
    connected(&r, "orders-prod");
    let tab = r.open("notes.md", Some(prod), Some("orders"));
    r.wait_session(tab, "the session");
    let doc = r.doc(tab);
    assert!(!r.vcx.cx.read(|cx| doc.read(cx).run_enabled(cx)));
    assert!(r.notices(tab).contains(&Notice::Markdown));
    r.run(tab);
    wait_until(r.vcx, "the refusal", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| t == "Markdown notes do not run")
    });
    assert!(queries(&r.fakes[0]).is_empty(), "a note never runs");
    let shown: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        shown.iter().any(|t| t == "Markdown notes do not run"),
        "{shown:?}"
    );
}

#[gpui::test]
fn pg_script_under_a_database_the_session_is_not_on_opens_nothing(cx: &mut TestAppContext) {
    let (r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let before = tab_count(r.vcx);
    let (lib, rel) = (r.lib, r.rel("a.sql"));
    r.vcx.cx.update(|cx| {
        assert_eq!(
            open_script(
                cx,
                lib,
                rel,
                Some(prod),
                Some(DatabaseName::new("orders_archive"))
            ),
            None
        );
    });
    r.vcx.run_until_parked();
    assert_eq!(tab_count(r.vcx), before, "no tab opens");
    let toasts = toast_texts(r.vcx).join(" | ");
    assert!(
        toasts.contains("orders-prod runs on orders, not orders_archive"),
        "{toasts}"
    );
}

#[gpui::test]
fn run_refuses_when_a_pg_session_is_on_another_database(cx: &mut TestAppContext) {
    // The session reports `orders_v2`: the profile was edited after the tab opened under orders.
    let r = rig_with(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake("orders_v2"))],
        &[("a.sql", "SELECT 1")],
        false,
    );
    let mut r = r;
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    connected(&r, "orders-prod");
    r.wait_session(tab, "the session");
    r.run(tab);
    wait_until(r.vcx, "the refusal", |_| {
        r.notices(tab)
            .iter()
            .any(|n| matches!(n, Notice::RunRefused(RunRefused::Edited { .. })))
    });
    assert!(queries(&r.fakes[0]).is_empty(), "nothing was sent");
    let shown: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        shown
            .iter()
            .any(|t| t == "Run refused: this tab runs on orders, but its session is on orders_v2."),
        "both databases named: {shown:?}"
    );
}

#[gpui::test]
fn mysql_tab_under_another_database_uses_its_own_session(cx: &mut TestAppContext) {
    // A session reads its database only after a statement that may move it: the one probe is
    // the one after the tab's USE.
    let mut r = rig_with(
        cx,
        vec![(DbmsKind::MySql, MYSQL, mysql_fake(&["orders_archive"]))],
        &[("a.sql", "SELECT 1")],
        false,
    );
    let id = add_profile(r.vcx, mysql_profile("api-dev", EnvTag::Dev, "orders"));
    connected(&r, "api-dev");
    let default_db = r
        .vcx
        .cx
        .read(|cx| hacivat_app::explorer::live::session_database(id, cx));
    let tab = r.open("a.sql", Some(id), Some("orders_archive"));
    assert_eq!(tab_title(r.vcx), "a.sql · api-dev · orders_archive");
    wait_until(r.vcx, "the switch landed", |_| {
        let doc = r.doc(tab);
        r.vcx
            .cx
            .read(|cx| doc.read(cx).switch_state() == tabs::Switch::Done)
    });
    let fake = r.fakes[0].clone();
    let uses: Vec<String> = fake
        .executed()
        .into_iter()
        .filter(|s| s.starts_with("USE"))
        .collect();
    assert_eq!(uses, ["USE `orders_archive`"], "exactly one USE");
    // Its own session, never the profile's default one.
    let console = r.console(tab);
    let (own, default) = r.vcx.cx.read(|cx| {
        let own = Consoles::get(cx, console).and_then(|s| s.session.clone());
        let default = cx
            .global::<hacivat_app::connections::ConnectionsModel>()
            .manager()
            .sessions(id)
            .and_then(|s| s.default_session());
        (own.map(|s| s.id()), default.map(|d| d.id()))
    });
    assert!(own.is_some() && own != default, "{own:?} vs {default:?}");
    assert_eq!(r.session_db(tab).as_deref(), Some("orders_archive"));
    // The explorer's reading of the profile's database is unchanged.
    let after = r
        .vcx
        .cx
        .read(|cx| hacivat_app::explorer::live::session_database(id, cx));
    assert_eq!(after, default_db);
    r.wait_session(tab, "the tab's session");
    r.run(tab);
    wait_until(r.vcx, "ran", |_| queries(&fake) == ["SELECT 1"]);
}

#[gpui::test]
fn mysql_tab_off_its_database_refuses_run_naming_both(cx: &mut TestAppContext) {
    // The tab's session keeps reporting `api` after the switch back: Run is refused.
    let mut r = rig_with(
        cx,
        vec![(
            DbmsKind::MySql,
            MYSQL,
            mysql_fake(&["kampanya", "api", "api"]),
        )],
        &[("a.sql", "SELECT 1")],
        false,
    );
    let id = add_profile(r.vcx, mysql_profile("api-dev", EnvTag::Dev, "api"));
    connected(&r, "api-dev");
    let tab = r.open("a.sql", Some(id), Some("kampanya"));
    wait_until(r.vcx, "the open switch", |_| {
        let doc = r.doc(tab);
        r.vcx
            .cx
            .read(|cx| doc.read(cx).switch_state() == tabs::Switch::Done)
    });
    r.wait_session(tab, "the session");
    // Something moved the session (a reconnect, a `USE` of the user's): it is on `api` now.
    let console = r.console(tab);
    let session = r
        .vcx
        .cx
        .read(|cx| Consoles::get(cx, console).and_then(|s| s.session.clone()))
        .unwrap();
    let _ = futures::executor::block_on(
        session
            .submit(hacivat_session::session::Request::SwitchSchema(
                "api".into(),
            ))
            .reply,
    );
    assert_eq!(r.session_db(tab).as_deref(), Some("api"));
    r.run(tab);
    wait_until(r.vcx, "the refusal", |_| {
        r.notices(tab)
            .iter()
            .any(|n| matches!(n, Notice::RunRefused(RunRefused::Mysql { .. })))
    });
    assert!(queries(&r.fakes[0]).is_empty(), "nothing was sent");
    let shown: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        shown
            .iter()
            .any(|t| t == "Run refused: this tab runs on kampanya, but its session is on api."),
        "{shown:?}"
    );
}

#[gpui::test]
fn mysql_tab_reconnect_reapplies_its_database_before_run(cx: &mut TestAppContext) {
    // Probes: after the open's USE (archive), Run's question after the reconnect (orders),
    // after the switch back (archive).
    let mut r = rig_with(
        cx,
        vec![(
            DbmsKind::MySql,
            MYSQL,
            mysql_fake(&["orders_archive", "orders", "orders_archive"]),
        )],
        &[("a.sql", "SELECT 1")],
        false,
    );
    let id = add_profile(r.vcx, mysql_profile("api-dev", EnvTag::Dev, "orders"));
    connected(&r, "api-dev");
    let tab = r.open("a.sql", Some(id), Some("orders_archive"));
    wait_until(r.vcx, "the open switch", |_| {
        let doc = r.doc(tab);
        r.vcx
            .cx
            .read(|cx| doc.read(cx).switch_state() == tabs::Switch::Done)
    });
    r.wait_session(tab, "the session");
    let fake = r.fakes[0].clone();
    // The connection drops; the session reconnects onto the profile's default database.
    let console = r.console(tab);
    let session = r
        .vcx
        .cx
        .read(|cx| Consoles::get(cx, console).and_then(|s| s.session.clone()))
        .unwrap();
    fake.kill_connections();
    let _ = futures::executor::block_on(
        session
            .submit(hacivat_session::session::Request::Ping)
            .reply,
    );
    // The tab's console holds a live session again that no longer reads `orders_archive`
    // (a fresh connection reads nothing until asked).
    wait_until(r.vcx, "reconnected", |vcx| {
        let live = vcx.cx.read(|cx| {
            Consoles::get(cx, console)
                .and_then(|s| s.session.clone())
                .is_some_and(|s| {
                    s.state().borrow().state == hacivat_session::session::SessionState::Idle
                })
        });
        live && r.session_db(tab).as_deref() != Some("orders_archive")
    });
    let before = fake.executed().len();
    r.run(tab);
    wait_until(r.vcx, "ran after the switch back", |_| {
        queries(&fake) == ["SELECT 1"]
    });
    let after: Vec<String> = fake.executed()[before..]
        .iter()
        .filter(|s| s.starts_with("USE"))
        .cloned()
        .collect();
    assert_eq!(
        after,
        ["USE `orders_archive`"],
        "one USE before the statement: {:?}",
        &fake.executed()[before..]
    );
    assert_eq!(r.session_db(tab).as_deref(), Some("orders_archive"));
}

// ---- dirty, disk and save ----------------------------------------------------------------------

/// Gate E2 (automated half): the OS watcher (FSEvents, inotify, ReadDirectoryChangesW) brings
/// an edit made by another editor into a clean tab, with no prompt and no bar; in place, then
/// as a write to a temp file renamed over the script (VS Code's save).
#[gpui::test]
fn external_edit_appears_without_reload(cx: &mut TestAppContext) {
    let mut r = rig_with(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake("orders"))],
        &[("a.sql", "SELECT 1")],
        true,
    );
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    // What the window shows: a full frame painted the editor at its current text (the code
    // area is a cached view, so the last frame alone may not have).
    let editor = r.editor(tab);
    let shows = |vcx: &VisualTestContext, text: &str| {
        let _ = full_frame(vcx);
        vcx.cx.read(|cx| {
            let e = editor.read(cx);
            e.snapshot().buffer.to_string() == text
                && e.last_layout()
                    .is_some_and(|l| l.version() == e.snapshot().version)
        })
    };
    std::fs::write(r.root.join("a.sql"), "SELECT 2").unwrap();
    wait_until(r.vcx, "the in-place edit shows", |vcx| {
        shows(vcx, "SELECT 2")
    });
    std::fs::write(r.root.join("a.sql.saving"), "SELECT 3").unwrap();
    std::fs::rename(r.root.join("a.sql.saving"), r.root.join("a.sql")).unwrap();
    wait_until(r.vcx, "the replaced file shows", |vcx| {
        shows(vcx, "SELECT 3")
    });
    assert!(r.notices(tab).is_empty(), "no bar: {:?}", r.notices(tab));
    assert!(!layer_ids(r.vcx).contains(&"dialog"), "no prompt");
    assert!(!r.dirty(tab));
}

#[gpui::test]
fn dirty_tab_is_never_reloaded(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- mine");
    assert!(r.dirty(tab));
    r.write_file("a.sql", "SELECT 99");
    wait_until(r.vcx, "the bar", |_| {
        r.notices(tab).contains(&Notice::Changed)
    });
    assert_eq!(r.text(tab), "SELECT 1 -- mine", "the typed text is kept");
    assert!(r.dirty(tab));
}

#[gpui::test]
fn clean_tab_reloads_in_place_with_a_status_message(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.write_file("a.sql", "SELECT 2");
    wait_until(r.vcx, "reloaded", |_| r.text(tab) == "SELECT 2");
    assert!(!r.dirty(tab));
    assert!(r.notices(tab).is_empty(), "no bar: {:?}", r.notices(tab));
    let doc = r.doc(tab);
    let status = r
        .vcx
        .cx
        .read(|cx| doc.read(cx).status_text())
        .map(|s| s.replace(['\u{2068}', '\u{2069}'], ""));
    assert_eq!(
        status.as_deref(),
        Some("Reloaded a.sql: it changed on disk")
    );
}

/// R-T11-1 (amends R-M4b-38): a composition's marked text is the tab's text, so a change on
/// disk during it never replaces it: the tab shows the bar, the composition ends with the
/// user's text, and Reload (confirmed) then takes the disk text.
#[gpui::test]
fn reload_waits_for_composition_end(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    let editor = r.editor(tab);
    r.focus(tab);
    in_window(r.vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            let len = e.snapshot().buffer.len_bytes();
            e.select_range(len..len, cx);
            e.replace_and_mark_text_in_range(None, "か", None, window, cx);
        });
    });
    r.vcx.run_until_parked();
    assert!(r.vcx.cx.read(|cx| editor.read(cx).status().composing));
    r.write_file("a.sql", "SELECT 2");
    wait_until(r.vcx, "the bar", |_| {
        r.notices(tab).contains(&Notice::Changed)
    });
    assert_eq!(
        r.text(tab),
        "SELECT 1か",
        "the composition is never replaced"
    );
    assert!(r.vcx.cx.read(|cx| editor.read(cx).status().composing));
    in_window(r.vcx, |window, cx| {
        editor.update(cx, |e, cx| e.end_composition(window, cx))
    });
    r.vcx.run_until_parked();
    assert_eq!(r.text(tab), "SELECT 1か", "committed as typed");
    // Reload from the bar (confirmed) takes the disk text.
    redraw_full(r.vcx);
    let reload = handle_of(r.vcx, selector(&format!("button-script-reload-{}", tab.0)));
    in_window(r.vcx, |window, cx| window.focus(&reload, cx));
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the reload asks", |vcx| {
        has_dialog(vcx, "Discard your edits and reload a.sql?")
    });
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "reloaded", |_| r.text(tab) == "SELECT 2");
    assert!(!r.dirty(tab));
}

#[gpui::test]
fn save_over_external_change_asks(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- mine");
    r.write_file("a.sql", "SELECT 99");
    wait_until(r.vcx, "the bar", |_| {
        r.notices(tab).contains(&Notice::Changed)
    });
    // Keep mine: the bar goes, and the focus is the code area (R-M4b-43).
    redraw_full(r.vcx);
    let keep = handle_of(r.vcx, selector(&format!("button-script-keep-{}", tab.0)));
    in_window(r.vcx, |window, cx| window.focus(&keep, cx));
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the bar goes", |_| {
        !r.notices(tab).contains(&Notice::Changed)
    });
    redraw_full(r.vcx);
    let editor = r.editor(tab);
    let code = r
        .vcx
        .cx
        .read(|cx| gpui::Focusable::focus_handle(editor.read(cx), cx));
    assert_focus_on(r.vcx, "the code area", &code);
    r.save(tab);
    wait_until(r.vcx, "Overwrite? asks", |vcx| {
        has_dialog(
            vcx,
            "The file changed on disk since you opened it. Overwrite?",
        )
    });
    // Cancel (Escape) keeps the file on disk as it is.
    press_key(r.vcx, "escape");
    settle(r.vcx, Duration::from_millis(50));
    assert_eq!(
        std::fs::read_to_string(r.root.join("a.sql")).unwrap(),
        "SELECT 99"
    );
    assert!(r.dirty(tab));
    // Overwrite writes it.
    r.save(tab);
    wait_until(r.vcx, "asks again", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "overwritten", |_| {
        std::fs::read_to_string(r.root.join("a.sql")).unwrap() == "SELECT 1 -- mine"
    });
    wait_until(r.vcx, "clean", |_| !r.dirty(tab));
}

#[gpui::test]
fn deleted_on_disk_keeps_text_and_offers_recreate(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    std::fs::remove_file(r.root.join("a.sql")).unwrap();
    r.hint("a.sql");
    wait_until(r.vcx, "the deleted bar", |_| {
        r.notices(tab).contains(&Notice::Deleted)
    });
    assert_eq!(r.text(tab), "SELECT 1", "the text stays");
    r.save(tab);
    wait_until(r.vcx, "written again", |_| {
        std::fs::read_to_string(r.root.join("a.sql"))
            .ok()
            .as_deref()
            == Some("SELECT 1")
    });
    wait_until(r.vcx, "the bar goes", |_| {
        !r.notices(tab).contains(&Notice::Deleted)
    });
}

#[gpui::test]
fn rename_on_disk_follows_path_not_connection(cx: &mut TestAppContext) {
    let (mut r, _, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(staging), Some("orders"));
    std::fs::rename(r.root.join("a.sql"), r.root.join("b.sql")).unwrap();
    let (from, to) = (r.root.join("a.sql"), r.root.join("b.sql"));
    assert!(
        r.watch
            .emit(&r.root, vec![RawHint::Path(from), RawHint::Path(to)])
    );
    wait_until(r.vcx, "followed", |_| r.spec(tab).rel.as_str() == "b.sql");
    let spec = r.spec(tab);
    assert_eq!(spec.profile, Some(staging), "never the connection");
    assert_eq!(spec.database, Some(DatabaseName::new("orders")));
    assert!(
        r.notices(tab)
            .iter()
            .any(|n| matches!(n, Notice::Moved { path } if path == "b.sql")),
        "{:?}",
        r.notices(tab)
    );
    let console = r.console(tab);
    let source = r.vcx.cx.read(|cx| Consoles::source(cx, console));
    assert!(matches!(source, ConsoleSource::Script { rel, .. } if rel.as_str() == "b.sql"));
}

#[gpui::test]
fn moved_into_production_folder_keeps_binding_and_notices(cx: &mut TestAppContext) {
    let (mut r, _, staging) = prod_staging(
        cx,
        &[("a.sql", "SELECT 1"), ("@production/x.sql", "SELECT 0")],
    );
    let tab = r.open("a.sql", Some(staging), Some("orders"));
    std::fs::rename(r.root.join("a.sql"), r.root.join("@production/a.sql")).unwrap();
    let (from, to) = (r.root.join("a.sql"), r.root.join("@production/a.sql"));
    assert!(
        r.watch
            .emit(&r.root, vec![RawHint::Path(from), RawHint::Path(to)])
    );
    wait_until(r.vcx, "no longer listed", |_| {
        r.notices(tab)
            .iter()
            .any(|n| matches!(n, Notice::Unlisted(_)))
    });
    let spec = r.spec(tab);
    assert_eq!(spec.profile, Some(staging));
    assert_eq!(
        spec.rel.as_str(),
        "a.sql",
        "the production path is never followed"
    );
    let shown: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        shown
            .iter()
            .any(|t| t == "a.sql is no longer listed under orders-staging."),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|t| t.contains("@production")),
        "{shown:?}"
    );
}

#[gpui::test]
fn reload_from_the_bar_keeps_the_focus_in_the_tab(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- mine");
    r.write_file("a.sql", "SELECT 2");
    wait_until(r.vcx, "the bar", |_| {
        r.notices(tab).contains(&Notice::Changed)
    });
    redraw_full(r.vcx);
    let reload = handle_of(r.vcx, selector(&format!("button-script-reload-{}", tab.0)));
    let keep = handle_of(r.vcx, selector(&format!("button-script-keep-{}", tab.0)));
    let order = tab_order(r.vcx);
    assert_consecutive(&order, &[("Reload", &reload), ("Keep mine", &keep)]);
    in_window(r.vcx, |window, cx| window.focus(&reload, cx));
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the reload asks", |vcx| {
        has_dialog(vcx, "Discard your edits and reload a.sql?")
    });
    // The Danger button: Tab back from Cancel, then Enter.
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "reloaded", |_| r.text(tab) == "SELECT 2");
    wait_until(r.vcx, "the bar is gone", |_| {
        !r.notices(tab).contains(&Notice::Changed)
    });
    redraw_full(r.vcx);
    let editor = r.editor(tab);
    let code = r
        .vcx
        .cx
        .read(|cx| gpui::Focusable::focus_handle(editor.read(cx), cx));
    assert_focus_on(r.vcx, "the code area", &code);
}

#[gpui::test]
fn save_never_writes_outside_the_library(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    connected(&r, "orders-prod");
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, ";\nSELECT 2");
    r.save(tab);
    wait_until(r.vcx, "saved", |_| !r.dirty(tab));
    // Wait past the recovery idle too: nothing of it lands in the library.
    settle(r.vcx, Duration::from_millis(100));
    let files = tree(&r.root);
    assert_eq!(files.len(), 1, "only the script itself: {files:?}");
    let (name, bytes) = &files[0];
    assert_eq!(name, "a.sql");
    assert_eq!(bytes, b"SELECT 1;\nSELECT 2");
    // No credential, connection detail or profile id is ever written into the folder.
    let profile = prod.to_string();
    for needle in [
        "orders-prod",
        PROD,
        "deploy",
        profile.as_str(),
        "hv_test_pw",
    ] {
        assert!(
            files
                .iter()
                .all(|(n, b)| !n.contains(needle) && !String::from_utf8_lossy(b).contains(needle)),
            "{needle} found in the library"
        );
    }
}

/// Every file under `dir` with its bytes, recursively, sorted (relative names).
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                let rel = p
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[gpui::test]
fn script_tab_never_autosaves_into_consoles(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- typed");
    r.vcx.executor().advance_clock(Duration::from_secs(2));
    settle(r.vcx, Duration::from_millis(100));
    let consoles = paths(r.vcx).consoles_dir(&prod.to_string());
    let written = std::fs::read_dir(&consoles).map(|d| d.count()).unwrap_or(0);
    assert_eq!(written, 0, "nothing under {}", consoles.display());
    // The text stays out of the library too, until ⌘S.
    assert_eq!(
        std::fs::read_to_string(r.root.join("a.sql")).unwrap(),
        "SELECT 1"
    );
}

#[gpui::test]
fn preview_tab_is_never_saved_or_run(cx: &mut TestAppContext) {
    let big = format!(
        "SELECT 1;\n{}",
        "-- padding line of a large script\n".repeat(700_000)
    );
    let (mut r, prod, _) = prod_staging(cx, &[("big.sql", big.as_str())]);
    connected(&r, "orders-prod");
    let tab = r.open("big.sql", Some(prod), Some("orders"));
    let doc = r.doc(tab);
    assert_eq!(
        r.vcx.cx.read(|cx| doc.read(cx).tier()),
        hacivat_scripts::fileio::SizeTier::Preview
    );
    assert!(
        !r.vcx.cx.read(|cx| doc.read(cx).run_enabled(cx)),
        "Run is off"
    );
    r.focus(tab);
    let enabled = r.vcx.cx.read(|cx| {
        let def = cx
            .global::<hacivat_ui::actions::ActionRegistry>()
            .get("file.save")
            .expect("file.save");
        (def.enabled)(cx)
    });
    assert!(!enabled, "Save is disabled on a preview");
    let shown: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        shown.iter().any(|t| t == "Read-only preview"),
        "the status segment: {shown:?}"
    );
    r.run(tab);
    wait_until(r.vcx, "the refusal", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| t == "A read-only preview doesn't run. Run as script sends the whole file.")
    });
    assert!(queries(&r.fakes[0]).is_empty(), "a preview never runs");
    let on_disk = std::fs::metadata(r.root.join("big.sql")).unwrap().len();
    assert_eq!(on_disk, big.len() as u64, "never cut to the preview");
}

// ---- close, quit, recovery, restore ------------------------------------------------------------

#[gpui::test]
fn close_with_unsaved_edits_asks_and_cancel_is_focused(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- mine");
    let tabs_before = tab_count(r.vcx);
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "the prompt", |vcx| {
        has_dialog(vcx, "Save changes to a.sql?")
    });
    let labels = button_labels(r.vcx);
    assert!(
        labels
            .windows(3)
            .any(|w| w == ["Don't save", "Save", "Cancel"]),
        "{labels:?}"
    );
    // Escape answers Cancel: the tab stays, dirty.
    press_key(r.vcx, "escape");
    assert_eq!(tab_count(r.vcx), tabs_before);
    assert!(r.dirty(tab));
    // Save: written, closed.
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "asks again", |vcx| {
        has_dialog(vcx, "Save changes to a.sql?")
    });
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "saved and closed", |vcx| {
        tab_count(vcx) == tabs_before - 1
    });
    assert_eq!(
        std::fs::read_to_string(r.root.join("a.sql")).unwrap(),
        "SELECT 1 -- mine"
    );
    // Don't save: closed, the file unchanged.
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- dropped");
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "asks", |vcx| {
        has_dialog(vcx, "Save changes to a.sql?")
    });
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "closed", |vcx| tab_count(vcx) == tabs_before - 1);
    assert_eq!(
        std::fs::read_to_string(r.root.join("a.sql")).unwrap(),
        "SELECT 1 -- mine"
    );
}

#[gpui::test]
fn the_prompt_opens_on_cancel(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- mine");
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "the prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    let dialog = r
        .vcx
        .cx
        .read(hacivat_app::exec_ui::close_prompt::open_prompt_dialog)
        .expect("the prompt holds the one-prompt slot");
    let focused = in_window(r.vcx, |window, cx| dialog.read(cx).focused_button(window));
    assert_eq!(focused, Some(2), "Cancel (the third button) has the focus");
}

#[gpui::test]
fn quit_asks_each_dirty_script_tab_after_open_transactions(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let a = r.open("a.sql", Some(prod), Some("orders"));
    let b = r.open("a.sql", Some(staging), Some("orders"));
    r.type_text(a, " -- a");
    r.type_text(b, " -- b");
    let quit = r.vcx.cx.update(hacivat_app::commands::request_quit);
    assert_eq!(quit, hacivat_app::shell::CloseDecision::Defer);
    wait_until(r.vcx, "the first prompt", |vcx| {
        has_dialog(vcx, "Save changes to a.sql?")
    });
    let body = dialog_body(r.vcx);
    assert!(body.contains("1 of 2 unsaved scripts"), "{body}");
    // Don't save the first: the second asks next.
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the second prompt", |vcx| {
        dialog_body(vcx).contains("2 of 2")
    });
    // Cancel stops the quit; both tabs stay.
    press_key(r.vcx, "escape");
    assert!(tab_count(r.vcx) >= 2);
    assert!(r.dirty(b));
}

#[gpui::test]
fn unsaved_text_is_copied_outside_the_library_after_five_quiet_seconds(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    r.type_text(tab, " -- unsaved");
    let paths = paths(r.vcx);
    let dir = paths.script_recovery_dir(&r.lib.to_string()).unwrap();
    r.vcx.executor().advance_clock(Duration::from_secs(4));
    settle(r.vcx, Duration::from_millis(50));
    let copies = |dir: &Path| {
        std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "sql"))
                    .count()
            })
            .unwrap_or(0)
    };
    assert_eq!(copies(&dir), 0, "not before 5 s of quiet");
    r.vcx.executor().advance_clock(Duration::from_secs(2));
    wait_until(r.vcx, "the copy", |_| copies(&dir) == 1);
    assert!(!dir.starts_with(&r.root), "never inside the library");
    assert_eq!(tree(&r.root).len(), 1, "the library holds only the script");
    // This instance is alive: its copy is not offered to another one.
    let other = hacivat_scripts::recovery::Instance::start(&paths).unwrap();
    assert!(hacivat_scripts::recovery::take_all(&paths, &other, r.lib).is_empty());
    // Saved: the copy goes.
    r.save(tab);
    wait_until(r.vcx, "saved", |_| !r.dirty(tab));
    wait_until(r.vcx, "the copy goes", |_| copies(&dir) == 0);
}

#[gpui::test]
fn recovery_copy_restores_unsaved_text_after_a_crash(cx: &mut TestAppContext) {
    // A crashed instance left a copy of an unbound tab of `a.sql`.
    let lib_dir = temp_root().join("orders");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(lib_dir.join("a.sql"), "SELECT 1").unwrap();
    let lib_dir = std::fs::canonicalize(&lib_dir).unwrap();
    let lib = std::cell::Cell::new(None);
    let (vcx, _) = shell_with_routes_prepared(cx, "en", Vec::new(), |paths| {
        let core = hacivat_session::runtime::CoreRuntime::new().unwrap();
        let (_, watch) = FakeEventSource::new();
        let service = LibraryService::open_with(paths, &core.handle(), watch.sources(), opts());
        let id = futures::executor::block_on(service.attach(
            lib_dir.clone(),
            Some(LogicalDb::parse("orders").unwrap()),
            false,
        ))
        .unwrap();
        drop(service);
        core.shutdown(Duration::from_secs(5));
        let crashed = hacivat_scripts::recovery::Instance::start(paths).unwrap();
        hacivat_scripts::recovery::store(
            paths,
            &crashed,
            &hacivat_scripts::recovery::RecoveryKey {
                library: id,
                rel: RelPath::parse("a.sql").unwrap(),
                profile: None,
                database: None,
            },
            "SELECT 1 -- unsaved",
            &hacivat_scripts::recovery::CopyMeta::default(),
        )
        .unwrap();
        drop(crashed);
        lib.set(Some(id));
    });
    let lib = lib.get().unwrap();
    wait_until(vcx, "the recovered tab", |vcx| {
        redraw_full(vcx);
        let ws = workspace(vcx);
        let tab = vcx.cx.read(|cx| ws.read(cx).active_tab());
        tab.and_then(|t| doc(vcx, t)).is_some_and(|d| {
            vcx.cx
                .read(|cx| *d.read(cx).load() == Load::Ready && d.read(cx).is_dirty(cx))
        })
    });
    let ws = workspace(vcx);
    let tab = vcx.cx.read(|cx| ws.read(cx).active_tab()).unwrap();
    let d = doc(vcx, tab).unwrap();
    let (text, notices, spec) = vcx.cx.read(|cx| {
        let doc = d.read(cx);
        let editor = doc.editor().unwrap().read(cx);
        (
            editor.snapshot().buffer.to_string(),
            doc.notices().to_vec(),
            doc.spec(cx).unwrap(),
        )
    });
    assert_eq!(text, "SELECT 1 -- unsaved");
    assert_eq!(spec.library, lib);
    assert_eq!(spec.profile, None, "bound as it was: unbound");
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, Notice::Recovered { unknown: false, .. })),
        "{notices:?}"
    );
    let toasts = toast_texts(vcx).join(" | ");
    assert!(
        toasts.contains("Recovered unsaved changes to 1 script"),
        "{toasts}"
    );
    // The file on disk is untouched until Save.
    assert_eq!(
        std::fs::read_to_string(lib_dir.join("a.sql")).unwrap(),
        "SELECT 1"
    );
    if let Some(service) = vcx.cx.read(hacivat_app::scripts::open::service) {
        service.shutdown();
    }
}

#[gpui::test]
fn restore_keeps_binding_and_reads_current_env(cx: &mut TestAppContext) {
    let (mut r, _, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(staging), Some("orders"));
    let spec = r.spec(tab);
    // The profile is retagged production while the tab is open: the capsule and the tab's
    // mark read the profile now.
    let mut p = r
        .vcx
        .cx
        .read(|cx| {
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .get(staging)
        })
        .unwrap()
        .as_ref()
        .clone();
    p.environment = Some(EnvTag::Production);
    save_draft(r.vcx, hacivat_session::profile::ProfileDraft::edit(p));
    connected(&r, "orders-staging");
    r.focus(tab);
    redraw_full(r.vcx);
    assert_eq!(
        capsule(r.vcx),
        Some(("orders-staging".to_string(), EnvKind::Production))
    );
    assert_eq!(r.spec(tab), spec, "the same binding");
}

/// R-M4b-22: after a restart the tab is bound as it was saved (by the profile's ID), and its
/// capsule reads the profile's environment as it is now (staging became production).
#[gpui::test]
fn restore_after_a_restart_keeps_binding_and_reads_current_env(cx: &mut TestAppContext) {
    let (mut r, _, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(staging), Some("orders"));
    let spec = r.spec(tab);
    let paths = paths(r.vcx);
    let root = paths.config.parent().unwrap().to_path_buf();
    let mut p = r
        .vcx
        .cx
        .read(|cx| {
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .get(staging)
        })
        .unwrap()
        .as_ref()
        .clone();
    p.environment = Some(EnvTag::Production);
    save_draft(r.vcx, hacivat_session::profile::ProfileDraft::edit(p));
    let store = paths.connections_toml();
    wait_until(r.vcx, "the retag is on disk", |_| {
        std::fs::read_to_string(&store)
            .is_ok_and(|t| t.matches("environment = \"production\"").count() == 2)
    });
    r.vcx.executor().advance_clock(Duration::from_millis(600));
    let state = paths.data.join("workspace.json");
    wait_until(r.vcx, "the layout is saved", |vcx| {
        vcx.executor().advance_clock(Duration::from_millis(600));
        std::fs::read_to_string(&state).is_ok_and(|t| t.contains("\"script\""))
    });
    close_all_windows(r.vcx);
    let (vcx2, _) = shell_with_routes_at(
        cx,
        &root,
        "en",
        vec![
            (DbmsKind::Postgres, PROD, pg_fake("orders")),
            (DbmsKind::Postgres, STAGING, pg_fake("orders")),
        ],
    );
    let ws = workspace(vcx2);
    let (panes, _) = layout(vcx2);
    let restored: Vec<(TabId, ScriptTab)> = vcx2.cx.read(|cx| {
        panes
            .panes
            .values()
            .flat_map(|p| p.tabs.clone())
            .filter_map(|t| match ws.read(cx).tab_kind(t) {
                Some(TabKind::Script { script }) => Some((t, tabs::spec(script, cx)?)),
                _ => None,
            })
            .collect()
    });
    assert_eq!(restored.len(), 1, "{restored:?}");
    let (tab2, spec2) = restored[0].clone();
    assert_eq!(spec2, spec, "the same binding, never re-resolved");
    connect(vcx2, "orders-staging");
    in_window(vcx2, |_, cx| {
        ws.update(cx, |w, cx| w.focus_tab(tab2, cx));
    });
    wait_until(vcx2, "the restored tab shows its file", |vcx| {
        redraw_full(vcx);
        doc(vcx, tab2).is_some_and(|d| vcx.cx.read(|cx| *d.read(cx).load() == Load::Ready))
    });
    redraw_full(vcx2);
    assert_eq!(
        capsule(vcx2),
        Some(("orders-staging".to_string(), EnvKind::Production)),
        "the environment as it is now"
    );
}

#[gpui::test]
fn a_bad_saved_script_tab_drops_only_itself_and_duplicates_restore_once(cx: &mut TestAppContext) {
    let root = temp_root();
    std::fs::create_dir_all(root.join("data")).unwrap();
    let lib = "01234567-89ab-cdef-0123-456789abcdef";
    let profile = ProfileId::new();
    let good = format!(
        r#"{{"id": 1, "kind": "script", "library": "{lib}", "rel_path": "a.sql", "profile": "{profile}", "database": "orders"}}"#
    );
    let dup = good.replace("\"id\": 1", "\"id\": 2");
    let bad = r#"{"id": 3, "kind": "script", "library": 42, "rel_path": "a.sql", "profile": null, "database": null}"#;
    let json = format!(
        r#"{{"version": 2,
            "tiles": {{"explorer_w": 260.0, "conn_panel_h": 180.0, "editor_h": 420.0,
                      "inspector_w": 300.0, "explorer_visible": true}},
            "panes": {{"root": {{"Leaf": 0}}, "panes": {{"0": {{"tabs": [0, 1, 2, 3], "active": 0}}}},
                      "focused": 0}},
            "tabs": [{{"id": 0, "kind": "console", "n": 1}}, {good}, {dup}, {bad}]}}"#
    );
    write(root.join("data").join("workspace.json"), &json);
    let vcx = shell_at(cx, root.as_path(), "en");
    // The console and one script tab: the wrong-typed one and the duplicate are dropped, the
    // layout is not set aside.
    assert_eq!(tab_count(vcx), 2);
    let set_aside: Vec<String> = std::fs::read_dir(root.join("data"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.contains("corrupt"))
        .collect();
    assert!(set_aside.is_empty(), "{set_aside:?}");
    let ws = workspace(vcx);
    let kind = vcx.cx.read(|cx| ws.read(cx).tab_kind(TabId(1)));
    let Some(TabKind::Script { script }) = kind else {
        panic!("{kind:?}");
    };
    let spec = vcx.cx.read(|cx| tabs::spec(script, cx)).unwrap();
    assert_eq!(
        spec.profile,
        Some(profile),
        "the id as saved, never re-resolved"
    );
}

#[gpui::test]
fn restore_with_deleted_profile_runs_nothing_and_says_so(cx: &mut TestAppContext) {
    let (mut r, prod, _) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let tab = r.open("a.sql", Some(prod), Some("orders"));
    // The profile is deleted while the tab is open.
    r.vcx.cx.update(|cx| {
        hacivat_app::connections::ConnectionsModel::delete(cx, prod).detach();
    });
    wait_until(r.vcx, "the removed notice", |_| {
        r.notices(tab).contains(&Notice::Removed)
    });
    assert_eq!(tab_title(r.vcx), "a.sql · no connection");
    let doc = r.doc(tab);
    assert!(r.vcx.cx.read(|cx| doc.read(cx).runs_nowhere(cx)));
    r.run(tab);
    settle(r.vcx, Duration::from_millis(50));
    assert!(queries(&r.fakes[0]).is_empty());
    assert_eq!(capsule(r.vcx), None, "no connection shown for it");
}

#[gpui::test]
fn status_bar_and_capsule_follow_the_focused_script_tab(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1"), ("b.sql", "SELECT 2")]);
    connected(&r, "orders-prod");
    connected(&r, "orders-staging");
    let a = r.open("a.sql", Some(prod), Some("orders"));
    let b = r.open("b.sql", Some(staging), Some("orders"));
    r.focus(a);
    redraw_full(r.vcx);
    assert_eq!(capsule(r.vcx).map(|c| c.1), Some(EnvKind::Production));
    r.focus(b);
    redraw_full(r.vcx);
    assert_eq!(capsule(r.vcx).map(|c| c.1), Some(EnvKind::Staging));
}

#[gpui::test]
fn the_last_tab_of_a_file_forgets_it(cx: &mut TestAppContext) {
    let (mut r, prod, staging) = prod_staging(cx, &[("a.sql", "SELECT 1")]);
    let a = r.open("a.sql", Some(prod), Some("orders"));
    let _b = r.open("a.sql", Some(staging), Some("orders"));
    let rel = r.rel("a.sql");
    let service = r.service.clone();
    let remembered = |s: &LibraryService, lib, rel: &RelPath| {
        futures::executor::block_on(s.is_remembered(lib, rel))
    };
    assert!(remembered(&service, r.lib, &rel));
    // One tab closes: the other still has it open.
    assert!(run(r.vcx, "workspace.close_tab"));
    r.vcx.run_until_parked();
    assert!(remembered(&service, r.lib, &rel));
    r.focus(a);
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "forgotten", |_| !remembered(&service, r.lib, &rel));
}

#[gpui::test]
fn open_under_another_connection_is_disabled_when_none_shows_the_file(cx: &mut TestAppContext) {
    let (mut r, _, staging) = prod_staging(cx, &[("@staging/a.sql", "SELECT 1")]);
    let tab = r.open("@staging/a.sql", Some(staging), Some("orders"));
    let spec = r.spec(tab);
    let others = r
        .vcx
        .cx
        .read(|cx| hacivat_app::scripts::open::other_places(&spec, cx));
    assert!(
        others.is_empty(),
        "prod does not show a @staging file: {others:?}"
    );
}

#[test]
fn core_and_editor_tiers_agree() {
    let cfg = hacivat_ui_editor::EditorConfig::default();
    assert_eq!(
        cfg.tiers.reduced_max_bytes,
        hacivat_scripts::fileio::REDUCED_MAX_BYTES
    );
    assert_eq!(
        cfg.tiers.full_max_bytes,
        hacivat_scripts::fileio::FULL_MAX_BYTES
    );
    assert_eq!(
        cfg.tiers.preview_bytes,
        hacivat_scripts::fileio::PREVIEW_BYTES as u64
    );
}
