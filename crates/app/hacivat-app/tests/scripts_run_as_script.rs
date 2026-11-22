//! M4b Task 18: Run as script in the app (HV-SCRIPT-12; NOTES surface 9). A T2 preview offers
//! Run as script instead of Run; the file streams from disk through Task 12's two-pass runner on
//! the tab's own connection, after Task 14's database check; every statement is classified,
//! gated and confirmed as Run's are; the strip shows the progress and Stop; the 20–100 MB paste
//! asks and opens a pasted preview.
//!
//! Each connection has its own `FakeDriver` (`support::shell_with_routes`). A script tab's
//! preview needs a file past 20 MB ([`big_script`]: the statements under test, then comment
//! lines with a cheap statement every 1.4 MB, so no stretch of comments reaches T8's 4 MiB cap);
//! a pasted preview (`run_as_script::fixtures::pasted_preview`, what "Paste as preview" opens)
//! needs no such file. Docker is Task 19's. Keys go through `support::press_key`; window text
//! through `support::full_frame` / `redraw_full` (the editor is a cached view).

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::exec_ui::confirm;
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::run_as_script;
use hacivat_app::scripts::tabs::{self, Load, Notice, RunRefused, ScriptDoc};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::LogicalDb;
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_session::profile::Profile;
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "10.0.0.1";
const DEV: &str = "10.0.0.2";
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

/// A statement that runs for a minute unless cancelled (`SELECT 99`; a plain read, so even
/// production runs it without asking).
fn slow() -> FakeQuery {
    FakeQuery {
        prefix: "SELECT 99",
        first_delay: Duration::from_secs(60),
        ..Default::default()
    }
}

/// A PostgreSQL fake whose schema probes answer `db` (every session, every Run's check).
fn pg_fake(db: &'static str) -> FakeDriver {
    let exec = (0..64)
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
        .queries(scs_queries().into_iter().chain([slow(), slow()]).collect())
}

/// A MySQL fake on database `db`.
fn mysql_fake(db: &'static str) -> FakeDriver {
    let mut exec: Vec<FakeExec> = (0..48)
        .map(|_| answer("SELECT @@SESSION.sql_mode", &[Some("")]))
        .collect();
    for _ in 0..48 {
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

/// `head`, then comment lines to past 20 MB, with `SELECT 0;` every 1.4 MB (module docs).
fn big_script(head: &str) -> String {
    let pad = "-- padding line of a large script\n";
    let mut s = String::with_capacity(22 << 20);
    s.push_str(head);
    while s.len() < (21 << 20) {
        for _ in 0..40_000 {
            s.push_str(pad);
        }
        s.push_str("SELECT 0;\n");
    }
    s
}

/// The statements `fake` ran as typed queries (Run's units), in order, without the context
/// probe's.
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

/// The statements of the file `fake` ran (the padding's `SELECT 0` left out).
fn sent(fake: &FakeDriver) -> Vec<String> {
    queries(fake)
        .into_iter()
        .map(|q| {
            q.lines()
                .filter(|l| !l.starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|q| q != "SELECT 0")
        .collect()
}

// ---- the rig ----------------------------------------------------------------------------------

struct Rig {
    vcx: &'static mut VisualTestContext,
    fakes: Vec<Arc<FakeDriver>>,
    service: Arc<LibraryService>,
    lib: LibraryId,
    root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.service.shutdown();
    }
}

fn rig(
    cx: &mut TestAppContext,
    routes: Vec<(DbmsKind, &'static str, FakeDriver)>,
    files: &[(&str, &str)],
) -> Rig {
    let (vcx, fakes) = shell_with_routes(cx, "en", routes);
    in_window(vcx, |window, _| window.activate_window());
    let root = temp_root().join("orders");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(&root).unwrap();
    let paths = paths(vcx);
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (_, watch) = FakeEventSource::new();
    let service = Arc::new(LibraryService::open_with(
        &paths,
        &rt,
        watch.sources(),
        ServiceOptions {
            settle: Duration::from_millis(20),
            git_debounce: Duration::from_millis(40),
            recheck: Duration::from_millis(60),
            home: None,
            ..ServiceOptions::default()
        },
    ));
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
        service
            .snapshot()
            .view(Some(EnvTag::Production))
            .library(lib)
            .is_some_and(|l| {
                l.entries()
                    .iter()
                    .filter(|e| e.kind != hacivat_scripts::EntryKind::Folder)
                    .count()
                    >= n
            })
    });
    Rig {
        vcx,
        fakes,
        service,
        lib,
        root,
    }
}

impl Rig {
    /// Opens `rel` under `profile` (database `orders`) and waits until its tab shows the file.
    fn open(&mut self, rel: &str, profile: ProfileId) -> TabId {
        let (lib, rel_path) = (self.lib, RelPath::parse(rel).unwrap());
        self.vcx.cx.update(|cx| {
            open_script(
                cx,
                lib,
                rel_path,
                Some(profile),
                Some(DatabaseName::new("orders")),
            );
        });
        let want = rel.to_string();
        wait_until(self.vcx, "the script tab shows its file", |vcx| {
            redraw_full(vcx);
            let ws = workspace(vcx);
            let tab = vcx.cx.read(|cx| ws.read(cx).active_tab());
            tab.and_then(|t| vcx.cx.read(|cx| tabs::doc_of_tab(t, cx)))
                .is_some_and(|d| {
                    vcx.cx.read(|cx| {
                        let d = d.read(cx);
                        *d.load() == Load::Ready
                            && d.spec(cx).is_some_and(|s| s.rel.as_str() == want)
                    })
                })
        });
        self.active()
    }

    /// Opens a console on `profile` and waits for its session and SQL context.
    fn console_on(&mut self, profile: ProfileId) -> TabId {
        let tab = with_ws(self.vcx, |ws, window, cx| {
            ws.open_console_tab(Some(profile), window, cx)
        })
        .expect("a console tab");
        wait_until(self.vcx, "the console", |vcx| {
            redraw_full(vcx);
            vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).is_some()
        });
        self.wait_session(tab);
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
        self.vcx
            .cx
            .read(|cx| tabs::doc_of_tab(tab, cx))
            .expect("a script doc")
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

    fn notices(&self, tab: TabId) -> Vec<Notice> {
        let doc = self.doc(tab);
        self.vcx.cx.read(|cx| doc.read(cx).notices().to_vec())
    }

    fn wait_session(&self, tab: TabId) {
        let console = self.console(tab);
        wait_until(self.vcx, "the session and its SQL context", |vcx| {
            vcx.cx.read(|cx| {
                Consoles::get(cx, console).is_some_and(|s| s.session.is_some())
                    && hacivat_app::exec_ui::sql_context(console, cx)
                        != hacivat_app::exec_ui::context::SqlContext::Pending
            })
        });
    }

    fn focus_editor(&self, tab: TabId) {
        let editor = self.editor(tab);
        in_window(self.vcx, |window, cx| {
            let f = gpui::Focusable::focus_handle(editor.read(cx), cx);
            window.focus(&f, cx);
        });
        self.vcx.run_until_parked();
    }

    fn editor_focus(&self, tab: TabId) -> gpui::FocusHandle {
        let editor = self.editor(tab);
        self.vcx
            .cx
            .read(|cx| gpui::Focusable::focus_handle(editor.read(cx), cx))
    }

    /// Makes console tab `tab` a pasted preview of `text`.
    fn pasted(&self, tab: TabId, text: &str) {
        let console = self.console(tab);
        let text = text.to_string();
        self.vcx
            .cx
            .update(|cx| run_as_script::fixtures::pasted_preview(console, &text, cx));
        redraw_full(self.vcx);
    }

    /// Presses the toolbar's Run as script.
    fn run_as_script(&mut self) {
        redraw_full(self.vcx);
        click(self.vcx, "button-console-run-as-script");
    }

    fn running(&self, tab: TabId) -> bool {
        let console = self.console(tab);
        self.vcx.cx.read(|cx| run_as_script::running(console, cx))
    }

    /// Waits until the run on `tab` ended; the strip's end sentence.
    fn ended(&self, tab: TabId) -> String {
        wait_until(self.vcx, "the run ends", |vcx| {
            redraw_full(vcx);
            !self.running(tab) && text_of(vcx, "script-run-end").is_some()
        });
        text_of(self.vcx, "script-run-end").expect("the end sentence")
    }

    /// Clicks the open confirmation's `id` button (`confirm-cancel`, …), its run buttons armed.
    fn answer(&mut self, id: &str) {
        self.vcx
            .executor()
            .advance_clock(confirm::ARM_DELAY + Duration::from_millis(50));
        self.vcx.run_until_parked();
        let selector: &'static str = Box::leak(format!("button-{id}").into_boxed_str());
        click(self.vcx, selector);
    }

    fn wait_confirm(&self) {
        wait_until(self.vcx, "the confirmation", |vcx| {
            vcx.cx.read(confirm::confirm_open)
        });
        redraw_full(self.vcx);
    }

    fn texts(&self) -> Vec<String> {
        full_frame(self.vcx)
            .into_iter()
            .map(|t| t.text.to_string())
            .collect()
    }

    /// The ids of the runner's lines in console `tab`'s output log.
    fn log_ids(&self, tab: TabId) -> Vec<&'static str> {
        let console = self.console(tab);
        self.vcx.cx.read(|cx| {
            Consoles::get(cx, console)
                .map(|s| {
                    s.log
                        .lines()
                        .iter()
                        .filter_map(|l| match &l.msg {
                            hacivat_exec::LogMsg::RunUi { id, .. } => Some(*id),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
    }
}

fn connected(r: &Rig, name: &str) {
    connect(r.vcx, name);
    wait_until(r.vcx, "connected", |vcx| {
        matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
    });
}

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

/// A shell with prod (PostgreSQL) and its fake, and `files` in the library.
fn prod(cx: &mut TestAppContext, files: &[(&str, &str)]) -> (Rig, ProfileId) {
    let r = rig(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake("orders"))],
        files,
    );
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    connected(&r, "orders-prod");
    (r, prod)
}

/// A shell with dev (PostgreSQL) and a console on it, its session ready.
fn dev_console(cx: &mut TestAppContext) -> (Rig, TabId) {
    let mut r = rig(cx, vec![(DbmsKind::Postgres, DEV, pg_fake("orders"))], &[]);
    let dev = add_profile(r.vcx, pg_profile("orders-dev", EnvTag::Dev, DEV, "orders"));
    connected(&r, "orders-dev");
    let tab = r.console_on(dev);
    (r, tab)
}

// ---- the plan's tests -------------------------------------------------------------------------

#[gpui::test]
fn run_as_script_asks_on_production_destructive_units(cx: &mut TestAppContext) {
    let big = big_script("SELECT 1;\nDELETE FROM t;\nSELECT 2;\n");
    let (mut r, prod) = prod(cx, &[("big.sql", big.as_str())]);
    let tab = r.open("big.sql", prod);
    r.wait_session(tab);
    let console = r.console(tab);
    assert!(r.vcx.cx.read(|cx| run_as_script::offered(console, cx)));
    // NOTES ruling 8: Run as script replaces Run on a preview; the toolbar has no Run.
    redraw_full(r.vcx);
    let labels = button_labels(r.vcx);
    assert!(labels.iter().any(|l| l == "Run as script"), "{labels:?}");
    assert!(!labels.iter().any(|l| l == "Run"), "{labels:?}");
    r.run_as_script();
    r.wait_confirm();
    // SELECT 1 ran; the DELETE waits for its answer, asked as Run asks it on production.
    assert_eq!(sent(&r.fakes[0]), ["SELECT 1"]);
    assert_eq!(
        r.vcx.cx.read(confirm::open_dialog_env),
        Some(Some(EnvTag::Production))
    );
    let buttons = r.vcx.cx.read(confirm::open_dialog_buttons);
    let ids: Vec<&str> = buttons.iter().map(|b| b.id).collect();
    assert!(
        !ids.contains(&"confirm-execute-all"),
        "never on production: {ids:?}"
    );
    assert!(ids.contains(&"confirm-skip-all"), "{ids:?}");
    let execute = buttons
        .iter()
        .find(|b| b.id == "confirm-execute")
        .expect("Execute");
    assert!(!execute.enabled, "Execute waits for the typed name");
    // The open-ended words (T12 m4): no count of later statements.
    assert_eq!(
        text_of(r.vcx, "confirm-open-ended-scope").as_deref(),
        Some(
            "Skip all skips every later statement in the file that would ask on this connection (production)."
        )
    );
    r.answer("confirm-cancel");
    let end = r.ended(tab);
    assert_eq!(
        end,
        "Stopped at statement 2: you didn't confirm it. The statements before it ran and stay applied; nothing after it was sent."
    );
    assert_eq!(
        sent(&r.fakes[0]),
        ["SELECT 1"],
        "no DELETE, nothing after it"
    );
    assert!(r.log_ids(tab).contains(&"exec-script-confirm-cancelled"));
}

#[gpui::test]
fn stop_ends_the_run_and_reports_progress(cx: &mut TestAppContext) {
    let (mut r, tab) = dev_console(cx);
    r.pasted(tab, "SELECT 1;\nSELECT 99;\nSELECT 3;\n");
    r.run_as_script();
    wait_until(r.vcx, "the slow statement runs", |vcx| {
        redraw_full(vcx);
        queries(&r.fakes[0]).iter().any(|q| q == "SELECT 99")
            && text_of(vcx, "script-run-sent").as_deref() == Some("1 statement sent")
    });
    let texts = r.texts();
    assert!(texts.iter().any(|t| t == "Running as script"), "{texts:?}");
    assert_eq!(text_of(r.vcx, "script-run-line").as_deref(), Some("line 2"));
    assert!(r.running(tab));
    click(r.vcx, "button-script-run-stop");
    let end = r.ended(tab);
    assert_eq!(end, "Stopped after 1 statement. Nothing after it was sent.");
    assert!(
        !queries(&r.fakes[0]).iter().any(|q| q == "SELECT 3"),
        "nothing after the stop was sent"
    );
    assert!(r.log_ids(tab).contains(&"exec-script-cancelled"));
    // Close takes the strip away.
    click(r.vcx, "button-script-run-close");
    redraw_full(r.vcx);
    assert!(text_of(r.vcx, "script-run-end").is_none());
}

#[gpui::test]
fn escape_never_cancels_the_run(cx: &mut TestAppContext) {
    let (mut r, tab) = dev_console(cx);
    r.pasted(tab, "SELECT 1;\nSELECT 99;\nSELECT 3;\n");
    r.run_as_script();
    wait_until(r.vcx, "the slow statement runs", |_| {
        queries(&r.fakes[0]).iter().any(|q| q == "SELECT 99")
    });
    // Escape in the code, then on Stop itself: the run goes on (M3 rule).
    r.focus_editor(tab);
    press_key(r.vcx, "escape");
    redraw_full(r.vcx);
    let stop = handle_of(r.vcx, "button-script-run-stop");
    press_key(r.vcx, "escape");
    redraw_full(r.vcx);
    assert!(r.running(tab), "Escape never stops a run");
    assert_focus_on(r.vcx, "Stop", &stop);
    // The M3 cancel path (`exec.cancel`, from the code) stops the whole run, not one unit.
    r.focus_editor(tab);
    assert!(run(r.vcx, "exec.cancel"), "exec.cancel ran");
    let end = r.ended(tab);
    assert_eq!(end, "Stopped after 1 statement. Nothing after it was sent.");
    assert!(!queries(&r.fakes[0]).iter().any(|q| q == "SELECT 3"));
}

#[gpui::test]
fn paste_confirmation_as_preview_opens_a_preview(cx: &mut TestAppContext) {
    let (mut r, tab) = dev_console(cx);
    let source = r.console(tab);
    let editor = r.editor(tab);
    // Small limits stand in for 20 and 100 MB (Task 11's test does the same).
    r.vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let mut cfg = *e.config();
            cfg.tiers.reduced_max_bytes = 16;
            cfg.tiers.paste_max_bytes = 4096;
            e.set_config(cfg, cx);
        })
    });
    let pasted = "SELECT 1;\nSELECT 2;\n-- pasted from somewhere\n";
    r.vcx
        .write_to_clipboard(gpui::ClipboardItem::new_string(pasted.to_string()));
    r.focus_editor(tab);
    assert!(run(r.vcx, "editor.paste"), "editor.paste ran");
    wait_until(r.vcx, "the question", |vcx| {
        layer_ids(vcx).contains(&"dialog")
    });
    assert!(
        dialog_title(r.vcx).starts_with("Paste "),
        "{}",
        dialog_title(r.vcx)
    );
    // Cancel is focused; "Paste as preview" is before it.
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the preview tab", |vcx| {
        redraw_full(vcx);
        let active = r.active();
        active != tab
            && vcx.cx.read(|cx| {
                Consoles::by_tab(cx, active).is_some_and(|c| run_as_script::offered(c, cx))
            })
    });
    let preview = r.active();
    let shown = r.editor(preview);
    assert!(
        r.vcx.cx.read(|cx| shown.read(cx).preview().is_some()),
        "a T2 preview"
    );
    assert_eq!(
        r.vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string()),
        "",
        "the source console's text stays"
    );
    assert!(
        r.vcx
            .cx
            .read(|cx| Consoles::get(cx, source).is_some_and(|s| s.editor.is_some())),
        "the source console is still open"
    );
    r.wait_session(preview);
    r.run_as_script();
    let end = r.ended(preview);
    assert_eq!(end, "Ran 2 statements from Pasted text");
    assert_eq!(queries(&r.fakes[0]), ["SELECT 1", "SELECT 2"]);
    // The pasted text lives in memory only: no console file holds it.
    let consoles = paths(r.vcx)
        .consoles_dir("x")
        .parent()
        .expect("the consoles folder")
        .to_path_buf();
    if consoles.exists() {
        for entry in walk(&consoles) {
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            assert!(!text.contains("pasted from somewhere"), "{entry:?}");
        }
    }
}

#[gpui::test]
fn trust_signals_stay_visible_during_a_run(cx: &mut TestAppContext) {
    let big = big_script("SELECT 1;\nSELECT 99;\n");
    let (mut r, prod) = prod(cx, &[("big.sql", big.as_str())]);
    let tab = r.open("big.sql", prod);
    r.wait_session(tab);
    r.run_as_script();
    wait_until(r.vcx, "the slow statement runs", |vcx| {
        redraw_full(vcx);
        sent(&r.fakes[0]).iter().any(|q| q == "SELECT 99")
            && text_of(vcx, "script-run-state").is_some()
    });
    // The capsule, the environment mark, the hairline and the read-only segment all show.
    assert_eq!(
        capsule(r.vcx),
        Some(("orders-prod".to_string(), EnvKind::Production))
    );
    let texts = r.texts();
    assert!(texts.iter().any(|t| t == "Read-only preview"), "{texts:?}");
    assert!(
        r.vcx.debug_bounds("console-env-swatch").is_some(),
        "the toolbar's mark"
    );
    let decor = r.vcx.cx.read(|cx| {
        let id = r.doc(tab).read(cx).script_id();
        tabs::tab_decor(tab, id, cx)
    });
    assert!(decor.hairline, "the production hairline");
    // The strip sits under the toolbar and covers none of them.
    let strip = r.vcx.debug_bounds("button-script-run-stop").expect("Stop");
    let swatch = r.vcx.debug_bounds("console-env-swatch").expect("the mark");
    assert!(strip.top() >= swatch.bottom(), "{strip:?} under {swatch:?}");
    let console = r.console(tab);
    r.vcx.cx.update(|cx| run_as_script::stop(console, cx));
    let _ = r.ended(tab);
}

#[gpui::test]
fn stop_by_keyboard_presses_the_button_and_keeps_the_focus(cx: &mut TestAppContext) {
    let (mut r, tab) = dev_console(cx);
    r.pasted(tab, "SELECT 1;\nSELECT 99;\nSELECT 3;\n");
    r.run_as_script();
    wait_until(r.vcx, "the slow statement runs", |vcx| {
        redraw_full(vcx);
        queries(&r.fakes[0]).iter().any(|q| q == "SELECT 99")
    });
    // Stop is a Tab stop (R-T33-1).
    let stop = handle_of(r.vcx, "button-script-run-stop");
    let order = tab_order(r.vcx);
    assert!(order.contains(&stop), "Stop is a Tab stop");
    in_window(r.vcx, |window, cx| window.focus(&stop, cx));
    r.vcx.run_until_parked();
    assert_focus_on(r.vcx, "Stop", &stop);
    press_key(r.vcx, "enter");
    let end = r.ended(tab);
    assert_eq!(end, "Stopped after 1 statement. Nothing after it was sent.");
    redraw_full(r.vcx);
    assert!(
        r.vcx.debug_bounds("button-script-run-stop").is_none(),
        "the running strip went"
    );
    // R-M4b-43: the focus is the code, never the window root.
    let code = r.editor_focus(tab);
    assert_focus_on(r.vcx, "the editor", &code);
}

#[gpui::test]
fn run_as_script_refuses_when_the_session_is_on_another_database(cx: &mut TestAppContext) {
    // R-M4b-27 for streamed runs: the tab was opened under `orders`; its session says `other`.
    let big = big_script("SELECT 1;\nSELECT 2;\n");
    let mut r = rig(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake("other"))],
        &[("big.sql", big.as_str())],
    );
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    connected(&r, "orders-prod");
    let tab = r.open("big.sql", prod);
    r.wait_session(tab);
    r.run_as_script();
    let end = r.ended(tab);
    assert_eq!(
        end,
        "Nothing in the file was run: the connection isn't on this tab's database, or Hacivat couldn't check which database it's on."
    );
    assert!(
        r.notices(tab)
            .iter()
            .any(|n| matches!(n, Notice::RunRefused(RunRefused::Edited { .. }))),
        "Task 14's notice names both databases"
    );
    assert_eq!(sent(&r.fakes[0]), Vec::<String>::new(), "nothing was sent");
}

// ---- the carries (end to end) -----------------------------------------------------------------

#[gpui::test]
fn r1_my_stray_end_sends_nothing_through_the_app(cx: &mut TestAppContext) {
    let mut r = rig(
        cx,
        vec![(DbmsKind::MySql, MYSQL, mysql_fake("orders"))],
        &[],
    );
    let mut p = test_profile("orders-my", Some(EnvTag::Dev), MYSQL);
    p.engine = DbmsKind::MySql;
    p.database = Some("orders".to_string());
    let id = add_profile(r.vcx, p);
    connected(&r, "orders-my");
    let tab = r.console_on(id);
    r.pasted(
        tab,
        "CREATE PROCEDURE p() BEGIN SELECT 1; END; DELETE FROM t; END;\nSELECT 2;",
    );
    r.run_as_script();
    let end = r.ended(tab);
    assert!(end.starts_with("Nothing in the file was run"), "{end}");
    let ran = queries(&r.fakes[0]);
    assert!(
        ran.iter()
            .all(|q| !q.contains("DELETE") && !q.contains("SELECT 2") && !q.contains("PROCEDURE")),
        "{ran:?}"
    );
}

#[gpui::test]
fn execute_all_in_a_streamed_run_is_open_ended(cx: &mut TestAppContext) {
    let (mut r, tab) = dev_console(cx);
    r.pasted(
        tab,
        "SELECT 1;\nDELETE FROM t;\nDELETE FROM u;\nSELECT 2;\n",
    );
    r.run_as_script();
    r.wait_confirm();
    let ids: Vec<&str> = r
        .vcx
        .cx
        .read(confirm::open_dialog_buttons)
        .iter()
        .map(|b| b.id)
        .collect();
    assert!(ids.contains(&"confirm-execute-all"), "{ids:?}");
    assert!(ids.contains(&"confirm-skip-all"), "{ids:?}");
    assert_eq!(
        text_of(r.vcx, "confirm-open-ended-scope").as_deref(),
        Some(
            "Execute all also runs, without asking, every later statement in the file that would ask on this connection (dev); a statement that needs its name typed still asks. Skip all skips every later statement that would ask."
        )
    );
    r.answer("confirm-execute-all");
    let end = r.ended(tab);
    assert_eq!(end, "Ran 4 statements from Pasted text");
    assert_eq!(
        queries(&r.fakes[0]),
        ["SELECT 1", "DELETE FROM t", "DELETE FROM u", "SELECT 2"],
        "the second DELETE was answered by Execute all"
    );
}

#[gpui::test]
fn a_file_changed_during_the_run_stops_it(cx: &mut TestAppContext) {
    // The DELETE asks; while it waits the file is rewritten. The run reads the file it opened
    // and checks each block: the next block read stops it, and nothing from it is sent.
    let mut head = String::from("SELECT 1;\nDELETE FROM t;\n");
    head.push_str(&"-- the first block\n".repeat(8_000));
    head.push_str("SELECT 42;\n");
    let big = big_script(&head);
    let (mut r, prod) = prod(cx, &[("big.sql", big.as_str())]);
    let tab = r.open("big.sql", prod);
    r.wait_session(tab);
    r.run_as_script();
    r.wait_confirm();
    let changed = big.replace("SELECT 42;", "DROP TABLE t");
    std::fs::write(r.root.join("big.sql"), changed).unwrap();
    // Cancel would end it anyway; type nothing, answer Skip: the run reads on and meets the
    // change.
    r.answer("confirm-skip");
    let end = r.ended(tab);
    assert_eq!(
        end,
        "The file changed on disk, so the run stopped. The statements before the change ran."
    );
    let ran = sent(&r.fakes[0]);
    assert_eq!(ran, ["SELECT 1"], "{ran:?}");
}

#[gpui::test]
fn a_retag_mid_run_reaches_the_runner(cx: &mut TestAppContext) {
    // T12 fr1 I2: the app keeps the runner's environment tag current. The profile is retagged
    // to production while a confirmation is open; the answered statement is not sent under the
    // tag it was gated with.
    let (mut r, tab) = dev_console(cx);
    r.pasted(tab, "SELECT 1;\nDELETE FROM t;\nSELECT 2;\n");
    r.run_as_script();
    r.wait_confirm();
    let console = r.console(tab);
    let mut p = r
        .vcx
        .cx
        .read(|cx| {
            let id = Consoles::get(cx, console)?.profile?;
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .get(id)
        })
        .expect("the console's profile")
        .as_ref()
        .clone();
    p.environment = Some(EnvTag::Production);
    save_draft(r.vcx, hacivat_session::profile::ProfileDraft::edit(p));
    r.answer("confirm-execute");
    let end = r.ended(tab);
    assert!(
        end.contains("environment or read-only setting changed"),
        "{end}"
    );
    assert_eq!(
        queries(&r.fakes[0]),
        ["SELECT 1"],
        "the DELETE was not sent"
    );
}

/// Every file under `dir`.
fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
