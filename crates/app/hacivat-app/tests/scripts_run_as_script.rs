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

// ## m4b-cifix-6-big-script
/// `head`, then comment lines to just past the 20 MiB preview threshold, with `SELECT 0;` every
/// 1.4 MB (module docs); and this test's turn to stream it (the guard, held to the test's end).
///
/// A run reads the whole file twice, and in a debug build every byte costs (CI 45102908608,
/// where the five tests that call this timed out on all three runners): lexing about 2.2 s and
/// SHA-256 about 0.7 s of a fast core for pass 1 alone, measured. So:
/// - the padding is long comment lines: a 2 KB line is one token where 34-byte lines were 60,
///   which halves pass 1 (3.6 s to 1.7 s locally);
/// - the file is no larger than the tier needs;
/// - one test streams at a time. Four whole-file passes at once on a 3- or 4-vCPU runner (each
///   with the UI thread redrawing its progress) is what took the waits past 10 s; on efficiency
///   cores the same four took 43 to 49 s each.
fn big_script(head: &str) -> (String, std::sync::MutexGuard<'static, ()>) {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let pad = format!("-- padding of a large script {}\n", "x".repeat(2_000));
    let past = usize::try_from(hacivat_scripts::fileio::REDUCED_MAX_BYTES).unwrap() + (64 << 10);
    let mut s = String::with_capacity(past + (2 << 20));
    s.push_str(head);
    while s.len() < past {
        for _ in 0..(1_400_000 / pad.len()) {
            s.push_str(&pad);
        }
        s.push_str("SELECT 0;\n");
    }
    (s, turn)
}
// ## end m4b-cifix-6-big-script

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
    let (big, _turn) = big_script("SELECT 1;\nDELETE FROM t;\nSELECT 2;\n");
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
    assert_eq!(
        end,
        "Stopped during statement 2; statements after it were not sent."
    );
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
    assert_eq!(
        end,
        "Stopped during statement 2; statements after it were not sent."
    );
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
    let (big, _turn) = big_script("SELECT 1;\nSELECT 99;\n");
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
    assert_eq!(
        end,
        "Stopped during statement 2; statements after it were not sent."
    );
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
    let (big, _turn) = big_script("SELECT 1;\nSELECT 2;\n");
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
    let (big, _turn) = big_script(&head);
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

// ## m4b-t18-run-as-script fix round 1 -----------------------------------------------------------

/// A MySQL fake whose schema probes answer `dbs` in order (the last one again after that).
fn mysql_fake_seq(dbs: &[&'static str]) -> FakeDriver {
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

/// [`pg_fake`] whose schema probes take 1.5 s each (a slow server).
fn pg_fake_slow_probe(db: &'static str) -> FakeDriver {
    let exec = (0..64)
        .flat_map(|_| {
            let mut probe = answer(
                "SELECT current_database()",
                &[
                    Some(db),
                    Some("public"),
                    Some("public"),
                    Some("deploy"),
                    Some("public"),
                ],
            );
            probe.delay = Duration::from_millis(1500);
            [
                answer("SHOW standard_conforming_strings", &[Some("on")]),
                answer("SELECT pg_catalog.txid_current_if_assigned()", &[None]),
                probe,
            ]
        })
        .collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(exec)
        .queries(scs_queries().into_iter().chain([slow(), slow()]).collect())
}

impl Rig {
    /// Opens `rel` under `profile` and database `db`, once the tab shows the file.
    fn open_on(&mut self, rel: &str, profile: ProfileId, db: &str) -> TabId {
        let (lib, rel_path) = (self.lib, RelPath::parse(rel).unwrap());
        let database = DatabaseName::new(db);
        self.vcx.cx.update(|cx| {
            open_script(cx, lib, rel_path, Some(profile), Some(database));
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

    /// Pastes `text` into tab `tab` past its (shrunk) 20 MB limit and answers "Paste as
    /// preview"; the preview's tab, once its console has a session.
    fn paste_as_preview(&mut self, tab: TabId, text: &str) -> TabId {
        let editor = self.editor(tab);
        self.vcx.cx.update(|cx| {
            editor.update(cx, |e, cx| {
                let mut cfg = *e.config();
                cfg.tiers.reduced_max_bytes = 16;
                cfg.tiers.paste_max_bytes = 4096;
                e.set_config(cfg, cx);
            })
        });
        self.vcx
            .write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
        self.focus_editor(tab);
        assert!(run(self.vcx, "editor.paste"), "editor.paste ran");
        wait_until(self.vcx, "the question", |vcx| {
            layer_ids(vcx).contains(&"dialog")
        });
        press_key(self.vcx, "shift-tab");
        press_key(self.vcx, "enter");
        wait_until(self.vcx, "the preview tab", |vcx| {
            redraw_full(vcx);
            let active = self.active();
            active != tab
                && vcx.cx.read(|cx| {
                    Consoles::by_tab(cx, active).is_some_and(|c| run_as_script::offered(c, cx))
                })
        });
        let preview = self.active();
        self.wait_session(preview);
        preview
    }

    fn session_of(&self, tab: TabId) -> hacivat_session::session::SessionHandle {
        let console = self.console(tab);
        self.vcx
            .cx
            .read(|cx| Consoles::get(cx, console).and_then(|s| s.session.clone()))
            .expect("a session")
    }
}

/// Sends what the gutter ▶ sends for the statement at byte 0 of tab `tab`'s editor.
fn gutter_run(r: &Rig, tab: TabId) {
    let editor = r.editor(tab);
    r.vcx.cx.update(|cx| {
        editor.update(cx, |e, cx| {
            let request = hacivat_ui_editor::ExecRequest {
                doc: e.doc(),
                action: hacivat_ui_editor::ExecAction::Execute1,
                at: hacivat_ui_editor::ExecAt::Carets(std::iter::once(0).collect()),
                snapshot: e.snapshot(),
            };
            cx.emit(hacivat_ui_editor::EditorEvent::ExecuteRequested(request));
        })
    });
    r.vcx.run_until_parked();
}

#[gpui::test]
fn plain_run_never_runs_on_a_preview_or_beside_a_stream(cx: &mut TestAppContext) {
    // Review rv_b (I1): on a pasted preview the gutter's gate is closed and what a ▶ sends runs
    // nothing, idle and while the stream runs; the stream's session never takes a second run.
    let (mut r, tab) = dev_console(cx);
    r.pasted(tab, "SELECT 1;\nSELECT 99;\nSELECT 3;\n");
    let console = r.console(tab);
    let gate = |r: &Rig| {
        r.vcx
            .cx
            .read(|cx| hacivat_app::console::commands::run_would_be_enabled(console, cx))
    };
    assert!(!gate(&r), "no ▶ on a preview");
    gutter_run(&r, tab);
    assert!(
        queries(&r.fakes[0]).is_empty(),
        "the idle preview ran nothing"
    );
    assert!(
        toast_texts(r.vcx)
            .iter()
            .any(|t| t == "A read-only preview doesn't run. Run as script sends the whole file."),
        "{:?}",
        toast_texts(r.vcx)
    );
    r.run_as_script();
    wait_until(r.vcx, "the slow statement runs", |_| {
        queries(&r.fakes[0]).iter().any(|q| q == "SELECT 99")
    });
    assert!(!gate(&r), "no ▶ while the stream runs");
    assert!(
        r.vcx
            .cx
            .read(|cx| hacivat_app::exec_ui::orchestrator::run_busy(console, cx)),
        "Run is busy while the stream runs"
    );
    gutter_run(&r, tab);
    assert!(
        layer_ids(r.vcx).iter().all(|l| *l != "chooser"),
        "no chooser: {:?}",
        layer_ids(r.vcx)
    );
    assert_eq!(
        queries(&r.fakes[0]),
        ["SELECT 1", "SELECT 99"],
        "no second run interleaved"
    );
    r.vcx.cx.update(|cx| run_as_script::stop(console, cx));
    let _ = r.ended(tab);
    assert!(!queries(&r.fakes[0]).iter().any(|q| q == "SELECT 3"));
}

/// [`pg_fake`] that also answers Run's context reads with a search path (so `=` resolves and
/// a bounded UPDATE is a plain write).
fn pg_fake_ctx(db: &'static str) -> FakeDriver {
    let mut exec: Vec<FakeExec> = (0..64)
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
    exec.extend((0..32).map(|_| {
        answer(
            "SELECT pg_catalog.current_setting('standard_conforming_strings')",
            &[Some("on"), Some("\"$user\", public")],
        )
    }));
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(exec)
        .queries(scs_queries().into_iter().chain([slow(), slow()]).collect())
}

#[gpui::test]
fn a_streamed_commit_prompt_shows_its_statement(cx: &mut TestAppContext) {
    // Review I2: on production in Auto mode (a connection retagged to production keeps its
    // mode, R-M3-78) a plain write runs in a transaction and asks before it is kept; the
    // prompt of a streamed statement shows that statement (its masked text).
    let mut r = rig(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake_ctx("orders"))],
        &[],
    );
    let id = add_profile(r.vcx, pg_profile("orders", EnvTag::Dev, PROD, "orders"));
    connected(&r, "orders");
    let tab = r.console_on(id);
    let mut p = r
        .vcx
        .cx
        .read(|cx| {
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .get(id)
        })
        .expect("the profile")
        .as_ref()
        .clone();
    p.environment = Some(EnvTag::Production);
    save_draft(r.vcx, hacivat_session::profile::ProfileDraft::edit(p));
    r.pasted(tab, "SELECT 1;\nUPDATE t SET a=1 WHERE id=3;\nSELECT 2;\n");
    r.run_as_script();
    // The prompt shows the statement it asks about (its SQL inset, `commit-sql`).
    let prompt_shows = |vcx: &VisualTestContext| {
        redraw_full(vcx);
        layer_ids(vcx).contains(&hacivat_app::exec_ui::commit_prompt::COMMIT_LAYER_ID)
            && full_frame(vcx)
                .iter()
                .any(|t| &*t.text == "UPDATE t SET a=1 WHERE id=3")
    };
    wait_until(
        r.vcx,
        "the confirm-in-transaction prompt with its statement",
        |vcx| prompt_shows(vcx),
    );
    // Escape answers Roll back: the statement is undone and the run stops there.
    press_key(r.vcx, "escape");
    let end = r.ended(tab);
    assert!(end.starts_with("Stopped at statement 2"), "{end}");
    assert!(!queries(&r.fakes[0]).iter().any(|q| q == "SELECT 2"));
}

#[gpui::test]
fn a_streamed_units_commit_prompt_turns_lost_with_its_connection(cx: &mut TestAppContext) {
    // Review I2: the script port follows a streamed unit's run (`ConfirmPort::follow_run`, what
    // the runner calls for every unit): its confirm-in-transaction prompt shows the statement,
    // and when the session reports the connection lost it turns to the "connection lost" form
    // (Close only), as M3's single run's prompt does.
    use hacivat_session::session::{RunEvent, RunId, UnitId};
    let (r, tab) = dev_console(cx);
    let console = r.console(tab);
    let session = r.session_of(tab);
    let port = r
        .vcx
        .cx
        .update(|cx| hacivat_app::exec_ui::AppConfirmPort::for_script_run(console, cx));
    let run = RunId(4242);
    let unit = UnitId { run, ordinal: 0 };
    let (choices_tx, choices) = tokio::sync::watch::channel(None);
    let (_status_tx, status) = tokio::sync::watch::channel(hacivat_exec::RunStatus::default());
    let rt = r.vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .runtime()
    });
    {
        let (port, session) = (port.clone(), session.clone());
        let followed = rt.spawn(async move {
            hacivat_exec::ConfirmPort::follow_run(
                port.as_ref(),
                run,
                choices,
                status,
                Arc::from(vec![Arc::<str>::from("UPDATE t SET a=1 WHERE id=3")]),
                session,
            );
        });
        futures::executor::block_on(followed).expect("followed");
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    choices_tx.send_replace(Some((
        1,
        RunEvent::NeedsCommit {
            unit,
            question: 7,
            affected: Some(1),
            deadline,
            savepoint: false,
        },
    )));
    let answer = hacivat_exec::ConfirmPort::commit_decision(
        port.as_ref(),
        hacivat_exec::CommitQuestion {
            run,
            unit,
            question: 7,
            affected: Some(1),
            deadline,
            savepoint: false,
            production: true,
            connection: Arc::from("orders-dev"),
        },
    );
    wait_until(r.vcx, "the prompt with its statement", |vcx| {
        redraw_full(vcx);
        layer_ids(vcx).contains(&hacivat_app::exec_ui::commit_prompt::COMMIT_LAYER_ID)
            && full_frame(vcx)
                .iter()
                .any(|t| &*t.text == "UPDATE t SET a=1 WHERE id=3")
    });
    // The connection drops inside a transaction (the session then never reconnects silently
    // and says it lost the connection); it finds it at its next request.
    let _ = futures::executor::block_on(
        session
            .submit(hacivat_session::session::Request::Exec {
                sql: "BEGIN".into(),
                opts: hacivat_driver::ExecOpts::values(16),
                origin: hacivat_session::session::ExecOrigin::User,
            })
            .reply,
    );
    r.fakes[0].kill_connections();
    std::mem::drop(session.submit(hacivat_session::session::Request::Ping));
    wait_until(r.vcx, "the lost form", |vcx| {
        redraw_full(vcx);
        text_of(vcx, "commit-dropped-body").is_some()
    });
    let labels = button_labels(r.vcx);
    assert!(!labels.iter().any(|l| l == "Commit"), "{labels:?}");
    drop(answer);
}

#[gpui::test]
fn a_change_between_the_passes_runs_nothing(cx: &mut TestAppContext) {
    // Review rv_a (m8): the file is rewritten (same length) while the between-pass database
    // check waits on a slow server: pass 2 refuses, nothing is sent.
    let (big, _turn) = big_script("SELECT 1;\nSELECT 2;\n");
    let mut r = rig(
        cx,
        vec![(DbmsKind::Postgres, PROD, pg_fake_slow_probe("orders"))],
        &[("big.sql", big.as_str())],
    );
    let prod = add_profile(
        r.vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    connected(&r, "orders-prod");
    let tab = r.open("big.sql", prod);
    r.wait_session(tab);
    let probes = |r: &Rig| {
        r.fakes[0]
            .executed()
            .iter()
            .filter(|s| s.starts_with("SELECT current_database()"))
            .count()
    };
    let before = probes(&r);
    r.run_as_script();
    wait_until(r.vcx, "the check's probe reaches the server", |_| {
        probes(&r) > before
    });
    std::fs::write(
        r.root.join("big.sql"),
        big.replacen("SELECT 1;", "SELECT 7;", 1),
    )
    .unwrap();
    let end = r.ended(tab);
    assert!(end.starts_with("Nothing in the file was run"), "{end}");
    assert_eq!(sent(&r.fakes[0]), Vec::<String>::new(), "nothing was sent");
}

/// A MySQL rig with one script `a.sql` open under `orders_archive` (the profile opens
/// `orders`), its open switch landed; the schema probes answer `dbs`.
fn mysql_archive_tab(cx: &mut TestAppContext, dbs: &[&'static str]) -> (Rig, TabId) {
    let mut r = rig(
        cx,
        vec![(DbmsKind::MySql, MYSQL, mysql_fake_seq(dbs))],
        &[("a.sql", "SELECT 1")],
    );
    let mut p = test_profile("orders-my", Some(EnvTag::Dev), MYSQL);
    p.engine = DbmsKind::MySql;
    p.database = Some("orders".to_string());
    let id = add_profile(r.vcx, p);
    connected(&r, "orders-my");
    let tab = r.open_on("a.sql", id, "orders_archive");
    wait_until(r.vcx, "the open switch", |vcx| {
        let doc = r.doc(tab);
        vcx.cx
            .read(|cx| doc.read(cx).switch_state() == tabs::Switch::Done)
    });
    r.wait_session(tab);
    (r, tab)
}

#[gpui::test]
fn a_paste_from_a_mysql_script_tab_runs_on_its_database_after_one_switch_back(
    cx: &mut TestAppContext,
) {
    // R-T18-1 (I3) and m8. Probes: the tab's open switch (archive); the pasted preview's check
    // on its own fresh session (orders); after the one switch back (archive). It runs there,
    // never on `orders`.
    let (mut r, tab) = mysql_archive_tab(cx, &["orders_archive", "orders", "orders_archive"]);
    let preview = r.paste_as_preview(tab, "SELECT 5;\nSELECT 6;\n-- pasted\n");
    // A session of its own, as the tab's (R-T14-3): never the profile's default one.
    assert_ne!(r.session_of(preview).id(), r.session_of(tab).id());
    let fake = r.fakes[0].clone();
    let before = fake.executed().len();
    r.run_as_script();
    let end = r.ended(preview);
    // The pasted document: the paste at the caret (the start) and the tab's own text.
    assert_eq!(end, "Ran 3 statements from Pasted text");
    let after: Vec<String> = fake.executed()[before..]
        .iter()
        .filter(|s| s.starts_with("USE"))
        .cloned()
        .collect();
    assert_eq!(after, ["USE `orders_archive`"], "one switch back");
    assert_eq!(queries(&fake), ["SELECT 5", "SELECT 6", "SELECT 1"]);
}

#[gpui::test]
fn a_paste_from_a_mysql_script_tab_never_runs_on_the_profiles_database(cx: &mut TestAppContext) {
    // R-T18-1 (I3): the pasted preview's session is still on `orders` after the switch back:
    // refused, nothing sent.
    let (mut r, tab) = mysql_archive_tab(cx, &["orders_archive", "orders", "orders"]);
    let preview = r.paste_as_preview(tab, "SELECT 5;\nSELECT 6;\n-- pasted\n");
    r.run_as_script();
    let end = r.ended(preview);
    assert_eq!(
        end,
        "Nothing in the file was run: the connection isn't on this tab's database, or Hacivat couldn't check which database it's on."
    );
    assert!(queries(&r.fakes[0]).is_empty(), "nothing ran on orders");
}

// ## demofix-1 (PO demo F6): a streamed run's Output stays a few rows long and its memory flat.

/// `n` one-row statements, `pad` bytes of literal each (the PO's dump: about 1.9 KB).
fn updates(n: u64, pad: usize) -> String {
    let x = "x".repeat(pad);
    (1..=n)
        .map(|i| format!("UPDATE demo SET v = '{x}' WHERE id = {i};\n"))
        .collect()
}

impl Rig {
    /// The lines of console `tab`'s log, and the rows of its Output list (when the tile shows
    /// it).
    fn output_size(&self, tab: TabId) -> (usize, Option<usize>) {
        let console = self.console(tab);
        self.vcx.cx.read(|cx| {
            let log = Consoles::get(cx, console).map_or(0, |s| s.log.lines().len());
            let rows = hacivat_app::results::view::hosts(cx)
                .into_iter()
                .find_map(|h| h.read(cx).log_list(console))
                .map(|l| l.read(cx).rows().line_count());
            (log, rows)
        })
    }
}

#[gpui::test]
fn a_streamed_runs_output_counts_quiet_statements_instead_of_listing_them(
    cx: &mut TestAppContext,
) {
    use hacivat_exec::SCRIPT_TAIL_STATEMENTS;
    let (mut r, tab) = dev_console(cx);
    let n = 2_000;
    r.pasted(tab, &updates(n, 8));
    r.run_as_script();
    let end = r.ended(tab);
    assert!(end.starts_with("Ran 2,000 statements"), "{end}");
    redraw_full(r.vcx);
    let (log, rows) = r.output_size(tab);
    // One counted line, the last 50 statements, the run's summary: never a line each.
    assert!(log <= SCRIPT_TAIL_STATEMENTS + 3, "{log} log lines");
    assert!(rows.is_none_or(|rows| rows <= SCRIPT_TAIL_STATEMENTS + 3), "{rows:?}");
    assert!(r.log_ids(tab).contains(&"exec-script-quiet"));
    let texts = r.texts();
    assert!(
        texts.iter().any(|t| t
            == "Statements 1–1950 ran: 1,950 statements, 0 rows affected, not listed one by one."),
        "{texts:?}"
    );
}

/// The PO's F6 measurement, in the app: a streamed run of `HACIVAT_PERF_UNITS` (default
/// 60,000) statements of about 1.9 KB each, from a library file, with a fake that keeps no log
/// of its own. Resident size (`ps`) and `phys_footprint` (`footprint`) before the run, at the
/// run's peak and after it; the Output's size at the end. Self-gated on `HACIVAT_PERF=1`.
#[gpui::test]
#[ignore = "perf"]
fn streamed_run_memory_stays_flat(cx: &mut TestAppContext) {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    let units: u64 = std::env::var("HACIVAT_PERF_UNITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000);
    let text = updates(units, 1_900);
    let bytes = text.len();
    let mut r = rig(
        cx,
        vec![(DbmsKind::Postgres, DEV, pg_fake("orders").unrecorded())],
        &[("big.sql", text.as_str())],
    );
    drop(text);
    let dev = add_profile(r.vcx, pg_profile("orders-dev", EnvTag::Dev, DEV, "orders"));
    connected(&r, "orders-dev");
    let tab = r.open("big.sql", dev);
    r.wait_session(tab);
    redraw_full(r.vcx);
    let pid = std::process::id().to_string();
    let rss_kib = || -> u64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .expect("ps");
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
    };
    let footprint = || -> String {
        std::process::Command::new("footprint")
            .args(["-p", &pid, "-f", "bytes"])
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.contains("phys_footprint") || l.contains("Footprint"))
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default()
    };
    let before = (rss_kib(), footprint());
    r.run_as_script();
    let mut peak = before.0;
    let mut peak_footprint = String::new();
    let started = std::time::Instant::now();
    let mut last_sample = started;
    loop {
        r.vcx.run_until_parked();
        if last_sample.elapsed() >= Duration::from_secs(2) {
            last_sample = std::time::Instant::now();
            let now = rss_kib();
            if now > peak {
                peak = now;
                peak_footprint = footprint();
            }
        }
        redraw_full(r.vcx);
        if !r.running(tab) && text_of(r.vcx, "script-run-end").is_some() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(1_800), "the run ends");
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = (rss_kib(), footprint());
    let (log, rows) = r.output_size(tab);
    println!(
        "HV-FACT demofix-1 f6 units={units} bytes={bytes} secs={} rss_before_mib={} \
         rss_peak_mib={} rss_after_mib={} growth_mib={} log_lines={log} output_rows={rows:?}",
        started.elapsed().as_secs(),
        before.0 / 1024,
        peak / 1024,
        after.0 / 1024,
        peak.saturating_sub(before.0) / 1024
    );
    println!("HV-FACT demofix-1 f6 footprint_before: {}", before.1);
    println!("HV-FACT demofix-1 f6 footprint_peak: {peak_footprint}");
    println!("HV-FACT demofix-1 f6 footprint_after: {}", after.1);
    assert!(log <= hacivat_exec::SCRIPT_TAIL_STATEMENTS + 12, "{log}");
    assert!(
        peak.saturating_sub(before.0) < 64 * 1024,
        "the run grew the app by {} MiB",
        peak.saturating_sub(before.0) / 1024
    );
}
// ## demofix-1
