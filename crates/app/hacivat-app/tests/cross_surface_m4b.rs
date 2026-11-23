// ## m4b-t21-shots
//! M4b Task 21, cross-surface flows (`design/m4b-scripts`).
//!
//! - (a) A script tab on production, connected, with the T2 banner, the Run-as-script strip,
//!   the find bar and the "Changed on disk" bar open at 960 × 600 keeps the capsule, the
//!   hairline and the environment cell whole (the M3c narrow-toolbar rule), and the rows stack
//!   as the mock shows: notice, banner, strip, find bar, code.
//! - (b) Focus order (NOTES surface 9, R-T33-1): toolbar → the strip's Stop → the code; the
//!   strip sits outside the cached editor, so its Stop comes before the code area.
//! - (c) Focus order of the notices T21 shoots (NOTES surfaces 10 and 12): the encoding
//!   notice's buttons in order, then the code; the recovered tab's Save, then Discard….
//!
//! The rows' accessible text is covered beside each surface's machinery: the Scripts tree's
//! `a11y_description`s in `scripts_projection.rs`, the bars' buttons as Tab stops in
//! `scripts_tabs.rs`, the dialogs' focus traps in `scripts_attach.rs`, `scripts_secrets.rs` and
//! `scripts_tabs.rs`, and the chooser's environment words in `scripts_goto.rs`
//! (`the_chooser_names_each_rows_environment`).

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::console::{ConsoleId, Consoles};
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::run_as_script;
use hacivat_app::scripts::tabs::{self, Load, Notice, ScriptDoc};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{FakeDriver, FakeExec};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::LogicalDb;
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "10.0.0.1";

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

/// A PostgreSQL fake on database `db` (the session's probes, answered many times).
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
}

/// A script past the 20 MB the editor reads whole: its tab opens it as a T2 preview.
fn big_script() -> String {
    let line = "INSERT INTO orders (id, city, total) VALUES (1204, 'İzmir', 1234567);\n";
    let n = (21usize << 20) / line.len() + 1;
    format!("-- orders dump\n{}", line.repeat(n))
}

struct Rig {
    vcx: &'static mut VisualTestContext,
    service: Arc<LibraryService>,
    lib: LibraryId,
    _root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.service.shutdown();
    }
}

/// A shell with orders-prod (production, connected over the fake) and a library bound to
/// `orders` holding `files`.
fn prod_rig(cx: &mut TestAppContext, files: &[(&str, Vec<u8>)]) -> (Rig, ProfileId) {
    prod_rig_in(cx, "en", files)
}

/// [`prod_rig`] in `lang` (fix round 2, m-a: the pseudo-locales too).
fn prod_rig_in(cx: &mut TestAppContext, lang: &str, files: &[(&str, Vec<u8>)]) -> (Rig, ProfileId) {
    let (vcx, _fakes) = shell_with_routes(
        cx,
        lang,
        vec![(DbmsKind::Postgres, PROD, pg_fake("orders"))],
    );
    in_window(vcx, |window, _| window.activate_window());
    let root = temp_root().join("orders");
    std::fs::create_dir_all(&root).unwrap();
    for (rel, bytes) in files {
        std::fs::write(root.join(rel), bytes).unwrap();
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
        ServiceOptions {
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
            .is_some_and(|l| l.entries().len() >= n)
    });
    let mut p = test_profile("orders-prod", Some(EnvTag::Production), PROD);
    p.database = Some("orders".to_string());
    let prod = add_profile(vcx, p);
    connect(vcx, "orders-prod");
    wait_until(vcx, "connected", |vcx| {
        matches!(
            live(vcx, "orders-prod"),
            hacivat_session::LiveState::Connected(_)
        )
    });
    (
        Rig {
            vcx,
            service,
            lib,
            _root: root,
        },
        prod,
    )
}

impl Rig {
    /// Opens `rel` under `profile` (database `orders`); waits until its doc reaches `done`.
    fn open_until(&mut self, rel: &str, profile: ProfileId, done: fn(&Load) -> bool) -> TabId {
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
        wait_until(self.vcx, "the script tab", |vcx| {
            redraw_full(vcx);
            self::active_doc(vcx).is_some_and(|d| {
                vcx.cx.read(|cx| {
                    let d = d.read(cx);
                    done(d.load()) && d.spec(cx).is_some_and(|s| s.rel.as_str() == want)
                })
            })
        });
        let ws = workspace(self.vcx);
        self.vcx
            .cx
            .read(|cx| ws.read(cx).active_tab())
            .expect("a tab")
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

    /// Shows `notice` on `tab` and waits until it is drawn.
    fn notice(&self, tab: TabId, notice: Notice) {
        let doc = self.doc(tab);
        let want = notice.clone();
        self.vcx
            .cx
            .update(|cx| tabs::fixtures::show(tab, notice, cx));
        wait_until(self.vcx, "the notice", |vcx| {
            vcx.cx.read(|cx| doc.read(cx).notices().contains(&want))
        });
        redraw_full(self.vcx);
    }

    /// The strip of a run under way on `tab` (no run behind it: nothing is sent).
    fn strip(&self, tab: TabId) {
        let console = self.console(tab);
        self.vcx.cx.update(|cx| {
            run_as_script::fixtures::show_strip(
                console,
                "big.sql",
                21 << 20,
                hacivat_exec::ScriptProgress {
                    phase: hacivat_exec::ScriptPhase::Running,
                    bytes: 9 << 20,
                    units_seen: 1_204,
                    units_total: None,
                    units_run: 1_204,
                    units_failed: 0,
                    line: 48_213,
                },
                None,
                cx,
            );
        });
        wait_until(self.vcx, "the strip", |vcx| {
            vcx.cx.read(|cx| run_as_script::running(console, cx))
        });
        redraw_full(self.vcx);
    }
}

/// A debug selector built at run time.
fn selector(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn active_doc(vcx: &VisualTestContext) -> Option<Entity<ScriptDoc>> {
    let ws = workspace(vcx);
    let tab = vcx.cx.read(|cx| ws.read(cx).active_tab())?;
    vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx))
}

fn ready(load: &Load) -> bool {
    *load == Load::Ready
}

fn needs_encoding(load: &Load) -> bool {
    matches!(load, Load::NeedsEncoding { .. })
}

/// (a) The plan's cross-surface case. The "Changed on disk" bar is forced by the fixture
/// (fix round 1, m2: a read-only preview has no edits, and a clean tab reloads in place): the
/// case is the room the rows leave, not how the bar is reached.
#[gpui::test]
fn a_production_script_tab_at_960_keeps_capsule_hairline_and_env_cell_whole(
    cx: &mut TestAppContext,
) {
    let (mut r, prod) = prod_rig(cx, &[("big.sql", big_script().into_bytes())]);
    let tab = r.open_until("big.sql", prod, ready);
    r.vcx
        .simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    r.notice(tab, Notice::Changed);
    r.strip(tab);
    let editor = r.editor(tab);
    in_window(r.vcx, |window, cx| {
        let f = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&f, cx);
        hacivat_ui::actions::ActionRegistry::run("editor.find", window, cx);
    });
    let console = r.console(tab);
    wait_until(r.vcx, "the find bar", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::console::find_bar::is_open(console, cx))
    });
    let frame = full_frame(r.vcx);
    // The capsule names the connection and its environment, and every capsule label is
    // whole (never ellipsized or cut) at 960.
    match capsule_model(r.vcx) {
        CapsuleModel::Connection { name, env, .. } => {
            assert_eq!(name.as_ref(), "orders-prod");
            assert_eq!(env, EnvKind::Production);
        }
        other => panic!("a connected capsule: {other:?}"),
    }
    let capsule: Vec<_> = frame
        .iter()
        .filter(|t| t.visible && t.path.contains("hacivat-ui/src/capsule.rs"))
        .collect();
    assert!(!capsule.is_empty(), "the capsule draws its words");
    assert!(
        capsule.iter().all(|t| !t.truncated && !t.clipped),
        "{capsule:?}"
    );
    // The status bar's environment cell, whole.
    let env: Vec<_> = frame
        .iter()
        .filter(|t| t.path.ends_with("#status-env-label"))
        .collect();
    assert!(
        env.iter()
            .any(|t| t.visible && !t.truncated && !t.clipped && t.painted_px > 0.0),
        "the environment cell: {env:?}"
    );
    // The hairline, and the toolbar's production mark.
    let decor = r.vcx.cx.read(|cx| {
        let id = r.doc(tab).read(cx).script_id();
        tabs::tab_decor(tab, id, cx)
    });
    assert!(decor.hairline, "the production hairline");
    assert!(
        r.vcx.debug_bounds("console-env-swatch").is_some(),
        "the toolbar's mark"
    );
    // Every row is there, none cut, stacked as the mock shows: notice, banner, strip, find
    // bar, code.
    let texts: Vec<&str> = frame.iter().map(|t| t.text.as_ref()).collect();
    assert!(texts.contains(&"big.sql changed on disk."), "{texts:?}");
    assert!(
        frame
            .iter()
            .filter(|t| t.visible)
            .all(|t| !t.clipped || t.source == hacivat_ui::text::TextSource::Data),
        "nothing but data is cut: {:?}",
        frame.iter().filter(|t| t.clipped).collect::<Vec<_>>()
    );
    let reload = r
        .vcx
        .debug_bounds(selector(&format!("button-script-reload-{}", tab.0)))
        .expect("the notice's Reload");
    let banner = r
        .vcx
        .debug_bounds("editor-t2-banner")
        .expect("the T2 banner");
    let stop = r.vcx.debug_bounds("button-script-run-stop").expect("Stop");
    let swatch = r.vcx.debug_bounds("console-env-swatch").expect("the mark");
    assert!(
        reload.bottom() <= banner.top(),
        "{reload:?} above {banner:?}"
    );
    assert!(
        banner.bottom() <= stop.top(),
        "the strip under the banner: {banner:?} {stop:?}"
    );
    assert!(swatch.bottom() <= reload.top(), "all under the toolbar");
    let code = r.vcx.debug_bounds("editor-column").expect("the code");
    assert!(stop.bottom() <= code.top(), "the code under the strip");
    // Fix round 1 (m1), fix round 2 (m-a): two whole lines of code show (`code_lines`).
    assert!(code_lines(&mut r, tab) >= 2, "two whole lines of code");
    #[cfg(feature = "scene-check")]
    {
        // The hairline is drawn: a 1 px red quad across the pane, under the tab strip.
        let red = r.vcx.update(|window, cx| {
            let red = gpui::Rgba::from(
                hacivat_ui_core::theme::Theme::get(cx)
                    .tokens
                    .colors
                    .red
                    .to_hsla(),
            );
            let scale = window.scale_factor();
            window
                .rendered_scene()
                .quads
                .iter()
                .filter(|q| {
                    q.background
                        .as_solid()
                        .is_some_and(|fill| gpui::Rgba::from(fill) == red)
                        && (q.bounds.size.height.0 / scale - 1.0).abs() < 0.01
                })
                .count()
        });
        assert!(red >= 1, "the 1 px production hairline is drawn");
    }
}

/// (b) NOTES surface 9 (toolbar → Stop → code). The strip is the console host's, under the
/// T2 banner and outside the cached editor: its Stop is the Tab stop right after the notice
/// above it (the code area takes Tab itself, so it is no stop), and the code starts under it;
/// the banner is drawn once. (Stop's focus home is
/// `scripts_run_as_script::stop_by_keyboard_presses_the_button_and_keeps_the_focus`.)
#[gpui::test]
fn the_strips_stop_is_a_tab_stop_above_the_code(cx: &mut TestAppContext) {
    let (mut r, prod) = prod_rig(cx, &[("big.sql", big_script().into_bytes())]);
    let tab = r.open_until("big.sql", prod, ready);
    r.strip(tab);
    let stop = handle_of(r.vcx, "button-script-run-stop");
    // A row above the strip: the notice's Keep mine (NOTES: toolbar → the notice's buttons →
    // the strip → the code).
    r.notice(tab, Notice::Changed);
    let keep = handle_of(r.vcx, selector(&format!("button-script-keep-{}", tab.0)));
    let order = tab_order(r.vcx);
    let at = |h: &gpui::FocusHandle| order.iter().position(|o| o == h);
    let (Some(s), Some(k)) = (at(&stop), at(&keep)) else {
        panic!("Stop and Keep mine are Tab stops: {order:?}");
    };
    assert_eq!(k + 1, s, "Tab from the notice's last button is Stop");
    let stop_at = r.vcx.debug_bounds("button-script-run-stop").expect("Stop");
    let banner = r
        .vcx
        .debug_bounds("editor-t2-banner")
        .expect("the T2 banner");
    let code_at = r.vcx.debug_bounds("editor-column").expect("the code");
    assert!(banner.bottom() <= stop_at.top(), "under the banner");
    assert!(
        stop_at.bottom() <= code_at.top(),
        "outside the editor, above the code"
    );
    // The banner is drawn once (the console host's, not also the editor's own).
    let banners = full_frame(r.vcx)
        .into_iter()
        .filter(|t| t.path.ends_with("#editor-t2-title"))
        .count();
    assert_eq!(banners, 1, "one T2 banner");
}

/// (c) NOTES surface 10: the encoding notice's four buttons are Tab stops in order, and it
/// stands in place of the tab's body (no toolbar, so no Run, until the text shows).
#[gpui::test]
fn the_encoding_notice_offers_its_encodings_in_order(cx: &mut TestAppContext) {
    let bytes = b"-- m\xfc\xfeteri raporu\nSELECT '\xddzmir';\n".to_vec();
    let (mut r, prod) = prod_rig(cx, &[("tr_rapor.sql", bytes)]);
    let tab = r.open_until("tr_rapor.sql", prod, needs_encoding);
    redraw_full(r.vcx);
    let texts: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        texts.iter().any(|t| t == "tr_rapor.sql isn't UTF-8 text."),
        "{texts:?}"
    );
    let n = tab.0 as usize * 8;
    let buttons: Vec<_> = (0..4)
        .map(|i| handle_of(r.vcx, selector(&format!("button-script-enc-{}", n + i))))
        .collect();
    let order = tab_order(r.vcx);
    let names = ["the first", "the second", "the third", "the fourth"];
    let stops: Vec<(&str, &gpui::FocusHandle)> = names.iter().copied().zip(&buttons).collect();
    assert_consecutive(&order, &stops);
    assert!(
        r.vcx.debug_bounds("button-console-run").is_none()
            && r.vcx.debug_bounds("button-console-run-as-script").is_none(),
        "no Run while the text is not shown"
    );
}

/// (c) NOTES surface 12: the recovered tab's notice: Save, then Discard….
#[gpui::test]
fn the_recovered_notice_offers_save_then_discard(cx: &mut TestAppContext) {
    let (mut r, prod) = prod_rig(cx, &[("a.sql", b"SELECT 1;\n".to_vec())]);
    let tab = r.open_until("a.sql", prod, ready);
    r.notice(
        tab,
        Notice::Recovered {
            at: std::time::UNIX_EPOCH + Duration::from_secs(1_795_259_100),
            unknown: false,
            secret: false,
        },
    );
    let save = handle_of(
        r.vcx,
        selector(&format!("button-script-rec-save-{}", tab.0)),
    );
    let discard = handle_of(
        r.vcx,
        selector(&format!("button-script-rec-discard-{}", tab.0)),
    );
    let order = tab_order(r.vcx);
    assert_consecutive(&order, &[("Save", &save), ("Discard…", &discard)]);
    // The date says when, with the time (DATETIME takes date and time styles only).
    let texts: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    let line = texts
        .iter()
        .find(|t| t.starts_with("Recovered unsaved changes from "))
        .unwrap_or_else(|| panic!("{texts:?}"));
    assert!(
        line.contains(':'),
        "the copy's time is shown, not only its day: {line}"
    );
}

// ## m4b-t21-shots fix round 2 (m-a)
/// How many whole lines of code the editor of `tab` shows in its last frame: rows from the
/// first painted one down to the text area's bottom or the console tile's, whichever is higher
/// (a column taller than its tile is cut at the tile's edge).
fn code_lines(r: &mut Rig, tab: TabId) -> usize {
    let editor = r.editor(tab);
    redraw_full(r.vcx);
    let tile = r
        .vcx
        .debug_bounds("console-tile")
        .expect("the console tile");
    r.vcx
        .cx
        .read(|cx| {
            editor.read(cx).last_layout().map(|l| {
                let rows = l.rows();
                let lh = f32::from(rows.line_height);
                let bottom = f32::from(rows.bounds.bottom()).min(f32::from(tile.bottom()));
                let room = bottom - f32::from(rows.first_y);
                (room / lh + 0.01).floor().max(0.0) as usize
            })
        })
        .unwrap_or(0)
}

/// The cross-surface rows at 960 × 600 in `lang`: two whole lines of code still show.
fn two_lines_at_960(cx: &mut TestAppContext, lang: &str) {
    let (mut r, prod) = prod_rig_in(cx, lang, &[("big.sql", big_script().into_bytes())]);
    let tab = r.open_until("big.sql", prod, ready);
    r.vcx
        .simulate_resize(gpui::size(gpui::px(960.0), gpui::px(600.0)));
    r.notice(tab, Notice::Changed);
    r.strip(tab);
    let editor = r.editor(tab);
    in_window(r.vcx, |window, cx| {
        let f = gpui::Focusable::focus_handle(editor.read(cx), cx);
        window.focus(&f, cx);
        hacivat_ui::actions::ActionRegistry::run("editor.find", window, cx);
    });
    let console = r.console(tab);
    wait_until(r.vcx, "the find bar", |vcx| {
        vcx.cx
            .read(|cx| hacivat_app::console::find_bar::is_open(console, cx))
    });
    let n = code_lines(&mut r, tab);
    assert!(n >= 2, "{lang}: {n} whole lines of code at 960");
}

#[gpui::test]
fn two_whole_lines_of_code_at_960_in_english(cx: &mut TestAppContext) {
    two_lines_at_960(cx, "en");
}

#[gpui::test]
fn two_whole_lines_of_code_at_960_in_en_xa(cx: &mut TestAppContext) {
    two_lines_at_960(cx, "en-XA");
}

#[gpui::test]
fn two_whole_lines_of_code_at_960_in_tr_xt(cx: &mut TestAppContext) {
    two_lines_at_960(cx, "tr-XT");
}

// ## m4b-t21-shots fix round 2 (N-I1): the short form of a connection on a tab.
#[test]
fn a_connections_short_form_cuts_whole_clusters_after_nfc_and_escapes_each_part() {
    use hacivat_app::scripts::tabs::tab_conn_short;
    let a = "a".repeat(19);
    let b = "b".repeat(30);
    // A flag at either end, a skin-tone emoji, a ZWJ sequence: whole on their side.
    let s = tab_conn_short(&format!("{a}🇹🇷{b}")).unwrap();
    assert!(s.starts_with(&format!("{a}🇹🇷…")), "{s}");
    let s = tab_conn_short(&format!("{b}🇹🇷{}", "c".repeat(18))).unwrap();
    assert!(s.ends_with(&format!("…🇹🇷{}", "c".repeat(18))), "{s}");
    let s = tab_conn_short(&format!("{a}👍🏽{b}")).unwrap();
    assert!(s.starts_with(&format!("{a}👍🏽…")), "{s}");
    // A combining mark never lands on the "…".
    let s = tab_conn_short(&format!("{b}x\u{0307}{}", "c".repeat(18))).unwrap();
    assert!(!s.contains("…\u{0307}"), "{s:?}");
    // A ZWJ sequence is one cluster; its joiner is shown escaped, whole.
    let s = tab_conn_short(&format!("{a}👩\u{200d}💻{b}")).unwrap();
    assert!(s.contains("<U+200D>"), "{s}");
    assert!(!s.contains("<U+2…"), "{s}");
    // An escaped control at the cut: its token whole (a control is one cluster).
    let s = tab_conn_short(&format!("{a}\u{7}{b}")).unwrap();
    assert!(!s.contains("<U+0…") && !s.contains("<U…"), "{s}");
    // Turkish after NFC: a decomposed ş at the cut is one cluster and shows composed.
    let s = tab_conn_short(&format!("{a}s\u{0327}{b}")).unwrap();
    assert!(s.starts_with(&format!("{a}ş…")), "{s}");
    // 40 clusters or fewer: no short form.
    assert_eq!(tab_conn_short(&"x".repeat(40)), None);
    assert_eq!(tab_conn_short(&format!("{}ş", "x".repeat(39))), None);
}
