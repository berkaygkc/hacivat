//! M4b Task 17: the secret warning (HV-SCRIPT-10, R-M4b-17, Q7; `design/m4b-scripts`
//! surface 8). A script tab marks password- and token-like literals in its gutter as you type
//! (debounced, scanned off the UI thread) and asks before a save writes one into the library:
//! the dialog lists lines and kinds, never the values; Cancel is the default and Escape keeps
//! the tab unsaved; "Save anyway" is remembered for the tab until quit; a line with
//! `-- hacivat:allow-secret` is not reported; a scan that cannot run fails closed
//! ("couldn't be checked"). Consoles never mark or ask.
//!
//! Each test owns a temp library on a fake watcher (the T14 rig, trimmed). Keys go through
//! `support::press_key`; the window through `support::full_frame` / `redraw_full` (the script
//! editor is a cached view). The debounce is GPUI time: tests advance the executor clock.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};
use hacivat_app::console::Consoles;
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::secret_marks::{self, DEBOUNCE};
use hacivat_app::scripts::tabs::{self, Load, Notice, ScriptDoc};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::secrets::SecretFinding;
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_scripts::{GitSummary, LogicalDb};
use hacivat_session::profile::Profile;
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const DEV: &str = "10.0.0.7";

/// The fixture password: it must never appear in what Hacivat draws, says or logs.
const PW: &str = "s3cret";

/// The platform's Save chord (the keymap rows of `file.save`).
const SAVE: &str = if cfg!(target_os = "macos") {
    "cmd-s"
} else {
    "ctrl-s"
};

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

/// A PostgreSQL fake on database `orders` (nothing in these tests runs SQL; a connect would
/// get its probes answered).
fn pg_fake() -> FakeDriver {
    let exec = (0..8)
        .flat_map(|_| {
            [
                answer("SHOW standard_conforming_strings", &[Some("on")]),
                answer("SELECT pg_catalog.txid_current_if_assigned()", &[None]),
            ]
        })
        .collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(exec)
}

/// A shell with one development profile and a library `orders` under a temp folder.
struct Rig {
    vcx: &'static mut VisualTestContext,
    service: Arc<LibraryService>,
    lib: LibraryId,
    root: PathBuf,
    dev: ProfileId,
}

impl Drop for Rig {
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

/// Runs the git CLI on a fixture repository this test made (the T13 helper's isolation).
fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn rig_with(cx: &mut TestAppContext, files: &[(&str, &str)], repo: bool) -> Rig {
    let (vcx, _) = shell_with_routes(cx, "en", vec![(DbmsKind::Postgres, DEV, pg_fake())]);
    in_window(vcx, |window, _| window.activate_window());
    let root = temp_root().join("orders");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    if repo {
        git(&root, &["init", "-q", "-b", "main"]);
    }
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
        opts(),
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
            .view(Some(EnvTag::Dev))
            .library(lib)
            .is_some_and(|l| l.entries().len() >= n)
    });
    let mut p: Profile = test_profile("orders-dev", Some(EnvTag::Dev), DEV);
    p.database = Some("orders".into());
    let dev = add_profile(vcx, p);
    Rig {
        vcx,
        service,
        lib,
        root,
        dev,
    }
}

fn rig(cx: &mut TestAppContext, files: &[(&str, &str)]) -> Rig {
    rig_with(cx, files, false)
}

impl Rig {
    /// Opens `rel` under orders-dev and waits until its tab shows the file.
    fn open(&mut self, rel: &str) -> TabId {
        let (lib, rel_path, dev) = (self.lib, RelPath::parse(rel).unwrap(), self.dev);
        self.vcx.cx.update(|cx| {
            open_script(
                cx,
                lib,
                rel_path,
                Some(dev),
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
        let tab = self.active();
        // The first scan (at open) lands.
        self.settle_scan(tab);
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

    fn editor(&self, tab: TabId) -> Entity<EditorView> {
        let console = self
            .vcx
            .cx
            .read(|cx| Consoles::by_tab(cx, tab))
            .expect("the tab's console");
        self.vcx
            .cx
            .read(|cx| Consoles::get(cx, console).and_then(|s| s.editor.clone()))
            .expect("an editor")
    }

    fn code_focus(&self, tab: TabId) -> gpui::FocusHandle {
        let editor = self.editor(tab);
        self.vcx
            .cx
            .read(|cx| gpui::Focusable::focus_handle(editor.read(cx), cx))
    }

    fn focus(&self, tab: TabId) {
        let ws = workspace(self.vcx);
        let code = self.code_focus(tab);
        in_window(self.vcx, |window, cx| {
            ws.update(cx, |w, cx| w.focus_tab(tab, cx));
            window.focus(&code, cx);
        });
        self.vcx.run_until_parked();
    }

    /// Types `s` at the end of the tab's text (no wait for the scan).
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

    fn text(&self, tab: TabId) -> String {
        let editor = self.editor(tab);
        self.vcx
            .cx
            .read(|cx| editor.read(cx).snapshot().buffer.to_string())
    }

    fn version(&self, tab: TabId) -> u64 {
        let editor = self.editor(tab);
        self.vcx.cx.read(|cx| editor.read(cx).snapshot().version)
    }

    /// Lets the debounce pass and waits until the tab's scan is for its current text.
    fn settle_scan(&self, tab: TabId) {
        self.vcx.executor().advance_clock(DEBOUNCE);
        let doc = self.doc(tab);
        let v = self.version(tab);
        wait_until(self.vcx, "the scan of the current text", |vcx| {
            vcx.cx.read(|cx| doc.read(cx).secrets().at(v).is_some())
        });
    }

    /// The gutter's marked lines (0-based).
    fn marked(&self, tab: TabId) -> Vec<u32> {
        let editor = self.editor(tab);
        self.vcx.cx.read(|cx| editor.read(cx).line_mark_lines())
    }

    fn dirty(&self, tab: TabId) -> bool {
        let doc = self.doc(tab);
        self.vcx.cx.read(|cx| doc.read(cx).is_dirty(cx))
    }

    /// ⌘S / Ctrl+S on the tab's code.
    fn save(&mut self, tab: TabId) {
        self.focus(tab);
        press_key(self.vcx, SAVE);
    }

    fn on_disk(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(rel)).unwrap()
    }
}

fn warning_open(vcx: &VisualTestContext) -> bool {
    layer_ids(vcx).contains(&"dialog")
        && vcx
            .cx
            .read(hacivat_app::scripts::secret_dialog::open_warning)
            .is_some()
}

/// Every text the window draws now (labels, dialog, notices, toasts), isolation marks stripped.
fn drawn(vcx: &VisualTestContext) -> Vec<String> {
    full_frame(vcx)
        .into_iter()
        .map(|r| r.text.to_string())
        .collect()
}

/// The texts `file` (a path suffix) drew in a fresh full frame.
fn drawn_by(vcx: &VisualTestContext, file: &str) -> Vec<String> {
    let needle = format!("{file}:");
    full_frame(vcx)
        .into_iter()
        .filter(|r| r.path.contains(&needle))
        .map(|r| r.text.to_string())
        .collect()
}

/// The accessibility declarations of a full frame.
fn declared(vcx: &VisualTestContext) -> Vec<hacivat_ui::popover::A11yDecl> {
    use hacivat_ui::popover::{a11y_record, a11y_take};
    a11y_record();
    redraw_full(vcx);
    a11y_take()
}

/// Waits for the warning, then returns the dialog's texts.
fn wait_warning(vcx: &VisualTestContext) -> Vec<String> {
    wait_until(vcx, "the secret warning", warning_open);
    drawn_by(vcx, "hacivat-ui/src/dialog.rs")
        .into_iter()
        .chain(drawn_by(vcx, "hacivat-app/src/scripts/secret_dialog.rs"))
        .collect()
}

// ---- tests ------------------------------------------------------------------------------------

/// Gate E4: a password literal typed into a script tab asks on ⌘S, listing "Line 1"; Escape
/// keeps the file as it was and the tab unsaved, and the focus goes back to the code.
#[gpui::test]
fn secret_literal_triggers_the_warning_on_save(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';"));
    r.settle_scan(tab);
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(
        texts
            .iter()
            .any(|t| t == "1 line in a.sql looks like a secret"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Line 1"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Looks like a password"),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "Line 2"), "one row: {texts:?}");
    press_key(r.vcx, "escape");
    wait_until(r.vcx, "the warning closes", |vcx| !warning_open(vcx));
    r.vcx.executor().advance_clock(Duration::from_secs(1));
    settle(r.vcx, Duration::from_millis(50));
    assert_eq!(r.on_disk("a.sql"), "", "nothing written");
    assert!(r.dirty(tab), "the tab stays unsaved");
    let code = r.code_focus(tab);
    assert_focus_on(r.vcx, "the code", &code);
}

/// The value is in no label, tooltip or accessible name the warning and the gutter make, and in
/// no toast.
#[gpui::test]
fn warning_never_echoes_the_value(cx: &mut TestAppContext) {
    let token = format!("gh{}", "p_0123456789abcdefghijABCDEFGHIJ012345");
    let mut r = rig(cx, &[("a.sql", "SELECT 1;\n")]);
    let tab = r.open("a.sql");
    r.type_text(
        tab,
        &format!(
            "CREATE USER app PASSWORD '{PW}';\n-- prod: postgresql://u:{PW}@db/orders\n-- {token}\n"
        ),
    );
    r.settle_scan(tab);
    assert_eq!(r.marked(tab), vec![1, 2, 3]);
    // The gutter glyphs' accessible names say the kind, never the value.
    let names: Vec<String> = declared(r.vcx)
        .into_iter()
        .filter(|d| d.role == Some(gpui::Role::Image))
        .map(|d| d.name.to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "Looks like a password"),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n == "Looks like an access token"),
        "{names:?}"
    );
    // The hover tooltip of line 2's glyph.
    hover_mark(&r, tab, 1);
    let tip = drawn_by(r.vcx, "hacivat-ui-editor/src/line_marks.rs");
    assert!(tip.iter().any(|t| t == "Looks like a password"), "{tip:?}");
    assert!(
        tip.iter().any(|t| t.contains("hacivat:allow-secret")),
        "{tip:?}"
    );
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(
        texts
            .iter()
            .any(|t| t == "3 lines in a.sql look like secrets"),
        "{texts:?}"
    );
    let all = drawn(r.vcx);
    let decls = declared(r.vcx);
    for secret in [PW, token.as_str(), "postgresql://u:"] {
        assert!(!texts.iter().any(|t| t.contains(secret)), "{texts:?}");
        assert!(!tip.iter().any(|t| t.contains(secret)), "{tip:?}");
        // The editor's own text runs are not labels; every label stays clean.
        assert!(
            !all.iter().any(|t| t.contains(secret)),
            "a drawn label holds a value"
        );
        assert!(
            !decls.iter().any(|d| d.name.contains(secret)
                || d.description.as_ref().is_some_and(|x| x.contains(secret))
                || d.value.as_ref().is_some_and(|x| x.contains(secret))),
            "an accessible name holds a value"
        );
        assert!(
            !toast_texts(r.vcx).iter().any(|t| t.contains(secret)),
            "a toast holds a value"
        );
    }
    // The finding types hold no value either (`Debug`).
    let doc = r.doc(tab);
    let dbg = r.vcx.cx.read(|cx| format!("{:?}", doc.read(cx).secrets()));
    assert!(!dbg.contains(PW) && !dbg.contains(&token), "{dbg}");
    press_key(r.vcx, "escape");
}

/// Moves the pointer over the mark slot of `line`'s row and waits for its tooltip.
fn hover_mark(r: &Rig, tab: TabId, line: u32) {
    use gpui::InputEvent as _;
    let editor = r.editor(tab);
    redraw_full(r.vcx);
    let (x, y) = r
        .vcx
        .cx
        .read(|cx| {
            editor.read(cx).last_layout().map(|l| {
                let rows = l.rows();
                let lh = f32::from(rows.line_height);
                let y = f32::from(rows.first_y) + (line - rows.first_line) as f32 * lh + lh / 2.0;
                (f32::from(rows.bounds.left()) - 58.0 + 14.0, y)
            })
        })
        .expect("a painted frame");
    in_window(r.vcx, |window, cx| {
        window.dispatch_event(
            gpui::MouseMoveEvent {
                position: gpui::point(gpui::px(x), gpui::px(y)),
                pressed_button: None,
                modifiers: gpui::Modifiers::none(),
            }
            .to_platform_input(),
            cx,
        );
    });
    r.vcx.executor().advance_clock(Duration::from_secs(1));
    wait_until(r.vcx, "the mark's tooltip", |vcx| {
        !drawn_by(vcx, "hacivat-ui-editor/src/line_marks.rs").is_empty()
    });
}

/// "Save anyway" writes the file; the same findings do not ask again in this tab, even on
/// other lines after an edit elsewhere. A new tab (as after a quit) asks again.
#[gpui::test]
fn save_anyway_writes_and_is_remembered_until_quit(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    let line = format!("CREATE USER app PASSWORD '{PW}';\n");
    r.type_text(tab, &line);
    r.settle_scan(tab);
    r.save(tab);
    wait_warning(r.vcx);
    // Shift+Tab from Cancel to Save anyway, then Enter.
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "written", |_| r.on_disk("a.sql") == line);
    wait_until(r.vcx, "clean", |_| !r.dirty(tab));
    // An edit above it: the finding moved a line down, and is still the one answered.
    let editor = r.editor(tab);
    in_window(r.vcx, |window, cx| {
        editor.update(cx, |e, cx| {
            e.select_range(0..0, cx);
            e.replace_text_in_range(None, "-- reader\n", window, cx);
        });
    });
    r.vcx.run_until_parked();
    r.settle_scan(tab);
    r.save(tab);
    let want = format!("-- reader\n{line}");
    wait_until(r.vcx, "saved without asking", |vcx| {
        assert!(!warning_open(vcx), "asked again");
        r.on_disk("a.sql") == want
    });
    // Closing the tab forgets the answer; a new tab asks again.
    wait_until(r.vcx, "clean again", |_| !r.dirty(tab));
    let before = tab_count(r.vcx);
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "the tab closes", |vcx| tab_count(vcx) == before - 1);
    let tab = r.open("a.sql");
    r.type_text(tab, "-- more\n");
    r.settle_scan(tab);
    r.save(tab);
    wait_warning(r.vcx);
    press_key(r.vcx, "escape");
}

/// A finding added after "Save anyway" asks again (and the dialog lists both lines).
#[gpui::test]
fn a_new_finding_asks_again(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    r.save(tab);
    wait_warning(r.vcx);
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "clean", |_| !r.dirty(tab));
    r.type_text(tab, "ALTER ROLE web PASSWORD 'tiger';\n");
    r.settle_scan(tab);
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(
        texts
            .iter()
            .any(|t| t == "2 lines in a.sql look like secrets"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Line 2"), "{texts:?}");
    press_key(r.vcx, "escape");
    assert!(r.dirty(tab));
    assert!(!r.on_disk("a.sql").contains("tiger"));
}

/// A line with `-- hacivat:allow-secret` is neither marked nor asked about.
#[gpui::test]
fn allow_comment_saves_without_asking(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    let text = format!("CREATE ROLE dev_tmp LOGIN PASSWORD '{PW}'; -- hacivat:allow-secret\n");
    r.type_text(tab, &text);
    r.settle_scan(tab);
    assert!(r.marked(tab).is_empty(), "{:?}", r.marked(tab));
    r.save(tab);
    wait_until(r.vcx, "saved without asking", |vcx| {
        assert!(!warning_open(vcx), "asked");
        r.on_disk("a.sql") == text
    });
}

/// A console never scans, marks or asks, whatever it holds (Q7).
#[gpui::test]
fn consoles_never_ask(cx: &mut TestAppContext) {
    let r = rig(cx, &[("a.sql", "")]);
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = calls.clone();
        r.vcx.cx.update(|cx| {
            secret_marks::set_scanner_for_tests(
                cx,
                Some(Arc::new(move |t: &str| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    secret_marks::scan_text(t)
                })),
            );
        });
    }
    let ws = workspace(r.vcx);
    let dev = r.dev;
    let tab = in_window(r.vcx, |window, cx| {
        ws.update(cx, |ws, cx| ws.open_console_tab(Some(dev), window, cx))
    })
    .expect("a console tab");
    wait_until(r.vcx, "the console's editor", |vcx| {
        vcx.cx.read(|cx| {
            Consoles::by_tab(cx, tab)
                .and_then(|c| Consoles::get(cx, c))
                .is_some_and(|s| s.editor.is_some())
        })
    });
    assert!(r.vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx)).is_none());
    let editor = r.editor(tab);
    let code = r
        .vcx
        .cx
        .read(|cx| gpui::Focusable::focus_handle(editor.read(cx), cx));
    in_window(r.vcx, |window, cx| {
        window.focus(&code, cx);
        editor.update(cx, |e, cx| {
            e.replace_text_in_range(
                None,
                &format!("CREATE USER app PASSWORD '{PW}';"),
                window,
                cx,
            );
        });
    });
    r.vcx.executor().advance_clock(DEBOUNCE * 4);
    settle(r.vcx, Duration::from_millis(50));
    press_key(r.vcx, SAVE);
    r.vcx.executor().advance_clock(DEBOUNCE * 4);
    settle(r.vcx, Duration::from_millis(50));
    assert!(!layer_ids(r.vcx).contains(&"dialog"), "a console asked");
    assert!(
        r.vcx
            .cx
            .read(|cx| editor.read(cx).line_mark_lines())
            .is_empty()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "a console was scanned");
}

/// The warning opens on Cancel: Enter answers Cancel and writes nothing. Both buttons are Tab
/// stops, trapped in the card.
#[gpui::test]
fn cancel_is_focused_and_enter_does_not_save(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "SELECT 1;\n")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    r.save(tab);
    wait_warning(r.vcx);
    let dialog = r
        .vcx
        .cx
        .read(hacivat_app::scripts::secret_dialog::open_warning)
        .expect("the warning");
    let focused = |vcx: &VisualTestContext| {
        in_window(vcx, |window, cx| dialog.read(cx).focused_button(window))
    };
    assert_eq!(focused(r.vcx), Some(1), "Cancel has the focus");
    let labels = button_labels(r.vcx);
    assert!(
        labels.windows(2).any(|w| w == ["Save anyway", "Cancel"]),
        "{labels:?}"
    );
    press_key(r.vcx, "tab");
    assert_eq!(focused(r.vcx), Some(0), "Tab: Save anyway");
    press_key(r.vcx, "tab");
    assert_eq!(focused(r.vcx), Some(1), "Tab again: Cancel (trapped)");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "closed", |vcx| !warning_open(vcx));
    r.vcx.executor().advance_clock(Duration::from_secs(1));
    settle(r.vcx, Duration::from_millis(50));
    assert_eq!(r.on_disk("a.sql"), "SELECT 1;\n");
    assert!(r.dirty(tab));
}

/// The scan runs off the UI thread, debounced (one scan for a burst of edits), and never
/// blocks typing: while a slow scan runs, the editor takes keys; an answer for an older text
/// is dropped (latest wins).
#[gpui::test]
fn gutter_marks_appear_off_the_ui_thread(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    let threads = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
    let gate = Arc::new(Mutex::new(gate_rx));
    {
        let (threads, calls, gate) = (threads.clone(), calls.clone(), gate.clone());
        r.vcx.cx.update(|cx| {
            secret_marks::set_scanner_for_tests(
                cx,
                Some(Arc::new(move |t: &str| -> Vec<SecretFinding> {
                    threads.lock().unwrap().push(std::thread::current().id());
                    calls.fetch_add(1, Ordering::SeqCst);
                    // Held until the test lets it go (a slow scan).
                    let _ = gate.lock().unwrap().recv_timeout(Duration::from_secs(10));
                    secret_marks::scan_text(t)
                })),
            );
        });
    }
    // A burst of edits inside the debounce: no scan yet.
    r.type_text(tab, "CREATE USER app ");
    r.vcx.executor().advance_clock(DEBOUNCE / 3);
    r.type_text(tab, &format!("PASSWORD '{PW}';"));
    r.vcx.executor().advance_clock(DEBOUNCE / 3);
    settle(r.vcx, Duration::from_millis(30));
    assert_eq!(calls.load(Ordering::SeqCst), 0, "debounced");
    // Quiet for the debounce: one scan starts, and is held.
    r.vcx.executor().advance_clock(DEBOUNCE);
    wait_until(r.vcx, "the scan starts", |_| {
        calls.load(Ordering::SeqCst) == 1
    });
    // Typing goes on while it runs.
    r.type_text(tab, "\nSELECT 1;");
    assert!(
        r.text(tab).ends_with("\nSELECT 1;"),
        "the editor took the keys"
    );
    let doc = r.doc(tab);
    assert!(r.vcx.cx.read(|cx| doc.read(cx).secrets().pending()));
    // The held scan (of the older text) finishes: its answer is dropped.
    gate_tx.send(()).unwrap();
    settle(r.vcx, Duration::from_millis(50));
    let v_now = r.version(tab);
    assert!(
        r.vcx
            .cx
            .read(|cx| doc.read(cx).secrets().at(v_now).is_none())
    );
    // The newer text's scan lands and marks line 1.
    r.vcx.executor().advance_clock(DEBOUNCE);
    wait_until(r.vcx, "the second scan starts", |_| {
        calls.load(Ordering::SeqCst) == 2
    });
    gate_tx.send(()).unwrap();
    wait_until(r.vcx, "the mark", |_| r.marked(tab) == vec![0]);
    let ui = std::thread::current().id();
    let seen = threads.lock().unwrap().clone();
    assert!(
        !seen.is_empty() && seen.iter().all(|t| *t != ui),
        "{seen:?}"
    );
    r.vcx
        .cx
        .update(|cx| secret_marks::set_scanner_for_tests(cx, None));
}

/// In a git repository the warning adds the commit sentence; outside one it does not.
#[gpui::test]
fn git_repo_variant_mentions_commits(cx: &mut TestAppContext) {
    const SENTENCE: &str =
        "If you commit this file, the value is shared with everyone who can read the repository.";
    let mut r = rig_with(cx, &[("a.sql", "")], true);
    let svc = r.service.clone();
    let lib = r.lib;
    wait_until(r.vcx, "the repository status", |_| {
        svc.snapshot()
            .view(Some(EnvTag::Dev))
            .library(lib)
            .is_some_and(|l| l.git() != GitSummary::Pending)
    });
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(texts.iter().any(|t| t == SENTENCE), "{texts:?}");
    press_key(r.vcx, "escape");
}

#[gpui::test]
fn a_library_outside_git_has_no_commit_sentence(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let svc = r.service.clone();
    let lib = r.lib;
    wait_until(r.vcx, "the repository status", |_| {
        svc.snapshot()
            .view(Some(EnvTag::Dev))
            .library(lib)
            .is_some_and(|l| l.git() == GitSummary::NotARepo)
    });
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(
        !texts.iter().any(|t| t.starts_with("If you commit")),
        "{texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t == "Save anyway is remembered for this tab until you quit."),
        "{texts:?}"
    );
    press_key(r.vcx, "escape");
}

/// A scan that cannot run (it panics) fails closed: the gutter says the script couldn't be
/// checked, a save asks, and "Save anyway" is never remembered for it.
#[gpui::test]
fn a_scan_error_fails_closed_as_unscanned(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "SELECT 1;\n")]);
    r.vcx.cx.update(|cx| {
        secret_marks::set_scanner_for_tests(
            cx,
            Some(Arc::new(|_: &str| -> Vec<SecretFinding> {
                panic!("the scanner failed (test)")
            })),
        );
    });
    let tab = r.open("a.sql");
    assert_eq!(r.marked(tab), vec![0], "the unscanned mark sits on line 1");
    let names: Vec<String> = declared(r.vcx)
        .into_iter()
        .filter(|d| d.role == Some(gpui::Role::Image))
        .map(|d| d.name.to_string())
        .collect();
    assert!(
        names.iter().any(|n| n == "Couldn't be checked for secrets"),
        "{names:?}"
    );
    r.type_text(tab, "SELECT 2;\n");
    r.settle_scan(tab);
    r.save(tab);
    let texts = wait_warning(r.vcx);
    assert!(
        texts
            .iter()
            .any(|t| t == "a.sql couldn't be checked for secrets"),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|t| t.starts_with("Line ")), "{texts:?}");
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "written", |_| {
        r.on_disk("a.sql") == "SELECT 1;\nSELECT 2;\n"
    });
    wait_until(r.vcx, "clean", |_| !r.dirty(tab));
    // Never remembered: the next save asks again.
    r.type_text(tab, "SELECT 3;\n");
    r.settle_scan(tab);
    r.save(tab);
    wait_warning(r.vcx);
    press_key(r.vcx, "escape");
    r.vcx
        .cx
        .update(|cx| secret_marks::set_scanner_for_tests(cx, None));
}

/// Save all asks too (the same save path), and Cancel stops the chain.
#[gpui::test]
fn save_all_asks_before_writing_a_secret(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    assert!(run(r.vcx, "file.save_all"));
    wait_warning(r.vcx);
    press_key(r.vcx, "escape");
    r.vcx.executor().advance_clock(Duration::from_secs(1));
    settle(r.vcx, Duration::from_millis(50));
    assert_eq!(r.on_disk("a.sql"), "");
}

/// R-T5-4: the recovery copy's metadata records that its text looked like a secret (never the
/// value), and the restore notice says so.
#[gpui::test]
fn recovery_copy_metadata_flags_a_secret(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "SELECT 1;\n")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    let dir = paths(r.vcx)
        .script_recovery_dir(&r.lib.to_string())
        .unwrap();
    r.vcx.executor().advance_clock(Duration::from_secs(6));
    let metas = |dir: &Path| -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "meta"))
                    .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    wait_until(r.vcx, "the copy", |_| metas(&dir).len() == 1);
    let meta = metas(&dir).remove(0);
    assert!(meta.contains("secret_suspected = true"), "{meta}");
    assert!(!meta.contains(PW), "the metadata holds no value");
}

#[gpui::test]
fn restore_notice_mentions_a_secret_without_the_value(cx: &mut TestAppContext) {
    let lib_dir = temp_root().join("orders");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(lib_dir.join("a.sql"), "SELECT 1").unwrap();
    let lib_dir = std::fs::canonicalize(&lib_dir).unwrap();
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
            &format!("CREATE USER app PASSWORD '{PW}';"),
            &hacivat_scripts::recovery::CopyMeta {
                secret_suspected: true,
                ..Default::default()
            },
        )
        .unwrap();
        drop(crashed);
    });
    wait_until(vcx, "the recovered tab", |vcx| {
        redraw_full(vcx);
        let ws = workspace(vcx);
        let tab = vcx.cx.read(|cx| ws.read(cx).active_tab());
        tab.and_then(|t| vcx.cx.read(|cx| tabs::doc_of_tab(t, cx)))
            .is_some_and(|d| vcx.cx.read(|cx| *d.read(cx).load() == Load::Ready))
    });
    let ws = workspace(vcx);
    let tab = vcx.cx.read(|cx| ws.read(cx).active_tab()).unwrap();
    let d = vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx)).unwrap();
    let notices = vcx.cx.read(|cx| d.read(cx).notices().to_vec());
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, Notice::Recovered { secret: true, .. })),
        "{notices:?}"
    );
    let shown = drawn_by(vcx, "hacivat-ui/src/notice.rs");
    assert!(
        shown
            .iter()
            .any(|t| t
                == "Some lines of this text look like a password or a token. Saving asks first."),
        "{shown:?}"
    );
    assert!(!shown.iter().any(|t| t.contains(PW)), "{shown:?}");
    assert!(!toast_texts(vcx).iter().any(|t| t.contains(PW)));
    if let Some(service) = vcx.cx.read(hacivat_app::scripts::open::service) {
        service.shutdown();
    }
}

/// Closing a dirty tab: the unsaved prompt's Save runs the same save, so the warning asks
/// next; Cancel there keeps the tab open, dirty, and the file as it was.
#[gpui::test]
fn the_unsaved_prompts_save_asks_first(cx: &mut TestAppContext) {
    let mut r = rig(cx, &[("a.sql", "")]);
    let tab = r.open("a.sql");
    r.type_text(tab, &format!("CREATE USER app PASSWORD '{PW}';\n"));
    r.settle_scan(tab);
    let before = tab_count(r.vcx);
    assert!(run(r.vcx, "workspace.close_tab"));
    wait_until(r.vcx, "the unsaved prompt", |vcx| {
        layer_ids(vcx).contains(&"dialog") && dialog_title(vcx) == "Save changes to a.sql?"
    });
    // Save (one Shift+Tab from Cancel).
    press_key(r.vcx, "shift-tab");
    press_key(r.vcx, "enter");
    wait_warning(r.vcx);
    press_key(r.vcx, "escape");
    wait_until(r.vcx, "the warning closes", |vcx| !warning_open(vcx));
    r.vcx.executor().advance_clock(Duration::from_secs(1));
    settle(r.vcx, Duration::from_millis(50));
    assert_eq!(tab_count(r.vcx), before, "the tab stays");
    assert!(r.dirty(tab));
    assert_eq!(r.on_disk("a.sql"), "");
}
