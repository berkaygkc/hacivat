// ## m4b-t20-adversarial
//! M4b Task 20: Review Focus 1 and 2 end to end, with `FakeDriver` connections (plan Task 20
//! step 1, "App"): where you open decides where it runs, on every path the earlier tasks'
//! tests do not already take. Each connection has its own fake (routed by host), so a test
//! tells which connection a statement reached; the library service runs on the app's core
//! runtime over a temp library, fed by a fake event source.
//!
//! - Run pressed while the connection chooser is open runs nothing, on any connection.
//! - A profile retagged while its script tab is open: the capsule shows the new environment at
//!   once, and the tab still runs only on its own connection.
//! - A file moved into `@production/` while open under staging: a Run press sends nothing to
//!   either connection, and Go to script offers it only where production shows it, never
//!   preselected.
//!
//! Keys go through `support::press_key`; window text through `support::full_frame`; every
//! wait is on a condition.

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::console::Consoles;
use hacivat_app::scripts::goto::{self, GotoHost};
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::tabs::{self, Load, ScriptDoc};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{
    FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall,
};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_scripts::{LogicalDb, RawHint};
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_ui::capsule::{CapsuleModel, EnvKind};
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "10.0.0.1";
const STAGING: &str = "10.0.0.2";

// ---- fakes (as Task 14's) -----------------------------------------------------------------------

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

/// A PostgreSQL fake on database `db` answering the session's probes as a server does.
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
    let scs = (0..48)
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
        .collect();
    FakeDriver::new(DbmsKind::Postgres)
        .tracks_transactions()
        .exec(exec)
        .queries(scs)
}

fn pg_profile(name: &str, env: EnvTag, host: &str, db: &str) -> Profile {
    let mut p = test_profile(name, Some(env), host);
    p.database = Some(db.to_string());
    p
}

/// The statements `fake` ran as Run's units, in order (the session's own probes left out).
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

// ---- the rig -----------------------------------------------------------------------------------

/// A shell with prod and staging (PostgreSQL, database `orders`, one fake each), and the
/// library `orders` of `files` attached.
struct Rig {
    vcx: &'static mut VisualTestContext,
    fakes: Vec<Arc<FakeDriver>>,
    service: Arc<LibraryService>,
    watch: hacivat_scripts::testing::FakeHandle,
    lib: LibraryId,
    root: PathBuf,
    prod: ProfileId,
    staging: ProfileId,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.service.shutdown();
    }
}

fn rig(cx: &mut TestAppContext, files: &[(&str, &str)]) -> Rig {
    let (vcx, fakes) = shell_with_routes(
        cx,
        "en",
        vec![
            (DbmsKind::Postgres, PROD, pg_fake("orders")),
            (DbmsKind::Postgres, STAGING, pg_fake("orders")),
        ],
    );
    in_window(vcx, |window, _| window.activate_window());
    let root = temp_root().join("orders");
    for (rel, text) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    let root = std::fs::canonicalize(&root).unwrap();
    let paths = paths(vcx);
    let rt = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().core_runtime().handle());
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
    // Every file the scan lists (a dot name never is).
    let n = files
        .iter()
        .filter(|(rel, _)| !rel.split('/').any(|c| c.starts_with('.')))
        .count();
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
    let prod = add_profile(
        vcx,
        pg_profile("orders-prod", EnvTag::Production, PROD, "orders"),
    );
    let staging = add_profile(
        vcx,
        pg_profile("orders-staging", EnvTag::Staging, STAGING, "orders"),
    );
    Rig {
        vcx,
        fakes,
        service,
        watch,
        lib,
        root,
        prod,
        staging,
    }
}

impl Rig {
    fn connected(&self, name: &str) {
        connect(self.vcx, name);
        wait_until(self.vcx, "connected", |vcx| {
            matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
        });
    }

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
            tab.and_then(|t| doc(vcx, t)).is_some_and(|d| {
                vcx.cx.read(|cx| {
                    let d = d.read(cx);
                    *d.load() == Load::Ready
                        && d.spec(cx)
                            .is_some_and(|s| s.rel.as_str() == want && s.profile == Some(profile))
                })
            })
        });
        self.vcx.run_until_parked();
        let ws = workspace(self.vcx);
        self.vcx
            .cx
            .read(|cx| ws.read(cx).active_tab())
            .expect("a tab is active")
    }

    fn editor(&self, tab: TabId) -> Entity<EditorView> {
        self.vcx
            .cx
            .read(|cx| {
                Consoles::by_tab(cx, tab)
                    .and_then(|c| Consoles::get(cx, c))
                    .and_then(|s| s.editor.clone())
            })
            .expect("an editor")
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

    /// The editor's Run on tab `tab`, as the command runs it.
    fn run(&self, tab: TabId) {
        self.focus(tab);
        let editor = self.editor(tab);
        in_window(self.vcx, |window, cx| {
            editor.update(cx, |e, cx| e.run_command("editor.run", window, cx));
        });
        self.vcx.run_until_parked();
    }

    fn wait_session(&self, tab: TabId) {
        let console = self
            .vcx
            .cx
            .read(|cx| Consoles::by_tab(cx, tab))
            .expect("the tab's console");
        wait_until(self.vcx, "the tab's session", |vcx| {
            vcx.cx.read(|cx| {
                Consoles::get(cx, console).is_some_and(|s| s.session.is_some())
                    && hacivat_app::exec_ui::sql_context(console, cx)
                        != hacivat_app::exec_ui::context::SqlContext::Pending
            })
        });
    }

    fn sent(&self) -> (Vec<String>, Vec<String>) {
        (queries(&self.fakes[0]), queries(&self.fakes[1]))
    }

    // ## fix round 1 (review I2)
    /// Waits until `tab` has no Run preparing or in flight, by the exec service's own signal
    /// (`run_busy` is true from a Run's preparation until the UI has seen its summary, after
    /// the session sent it on the core runtime). On its own this is not a barrier for a press
    /// still queued (a press reaches the console with the next frame), so a "nothing ran" check
    /// also goes through [`Rig::barrier`].
    fn quiesce(&self, tab: TabId) {
        let console = self
            .vcx
            .cx
            .read(|cx| Consoles::by_tab(cx, tab))
            .expect("the tab's console");
        self.vcx.run_until_parked();
        wait_until(self.vcx, "the tab's runs have ended", |vcx| {
            redraw_full(vcx);
            vcx.cx
                .read(|cx| !hacivat_app::exec_ui::orchestrator::run_busy(console, cx))
        });
        self.vcx.run_until_parked();
    }

    /// The barrier of a "nothing ran" check (review I2): Run on `barrier`, another tab of the
    /// same connection whose text no other press sends, and wait until that text is in the
    /// connection's log (the session runs in the order the runs were sent, and a press made
    /// before this one reaches the console before it). Then wait until `tab` is idle too. A
    /// Run that leaked from `tab` is in the log before this returns.
    fn barrier(&self, barrier: TabId, tab: TabId, text: &str) {
        let fake = &self.fakes[1];
        let seen = |f: &FakeDriver| queries(f).iter().filter(|q| *q == text).count();
        let before = seen(fake);
        self.run(barrier);
        wait_until(self.vcx, "the barrier's statement ran", |_| {
            seen(fake) > before
        });
        self.quiesce(barrier);
        self.quiesce(tab);
    }

    /// Everything the two connections ran, without the barrier statements.
    fn leaked(&self, barrier_text: &str) -> (Vec<String>, Vec<String>) {
        let (p, s) = self.sent();
        (p, s.into_iter().filter(|q| q != barrier_text).collect())
    }
}

const BARRIER: &str = "SELECT 'barrier'";

fn doc(vcx: &VisualTestContext, tab: TabId) -> Option<Entity<ScriptDoc>> {
    vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx))
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

fn host(vcx: &VisualTestContext) -> Option<Entity<GotoHost>> {
    vcx.cx.read(goto::open_host)
}

type Rows = (Vec<(String, String)>, Option<usize>);

fn chooser(vcx: &VisualTestContext) -> Option<Rows> {
    vcx.run_until_parked();
    let h = host(vcx)?;
    vcx.cx.read(|cx| h.read(cx).chooser_rows())
}

/// Opens Go to script, types `q` and waits for the result row `want`.
fn goto_query(vcx: &mut VisualTestContext, q: &str, want: &str) {
    assert!(run(vcx, "scripts.goto"), "scripts.goto ran");
    vcx.simulate_input(q);
    let want = want.to_string();
    wait_until(vcx, "the result row", |vcx| {
        host(vcx).is_some_and(|h| vcx.cx.read(|cx| h.read(cx).result_rows().contains(&want)))
    });
}

fn retag(vcx: &VisualTestContext, id: ProfileId, env: Option<EnvTag>) {
    let p = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id).unwrap());
    let mut edited = (*p).clone();
    edited.environment = env;
    save_draft(vcx, ProfileDraft::edit(edited));
}

// ---- Review Focus 1: where you open decides where it runs -----------------------------------------

/// Run pressed while the chooser is open (its chord, and the command itself) runs nothing on
/// any connection: the chooser holds the keys, the tab under it is not the target, and the
/// chooser's own answer is never a run. Once the chooser is gone, the same chord on another tab
/// of the same connection runs that tab's own, distinct text (the control: the presses were
/// live keys, and a late leak of `a.sql` could not hide behind it).
///
/// ## fix round 1 (review I2): every "nothing ran" check waits for the exec service's own end
/// of runs first ([`Rig::quiesce`]).
#[gpui::test]
fn run_pressed_while_the_chooser_is_open_runs_nothing(cx: &mut TestAppContext) {
    let mut r = rig(
        cx,
        &[
            ("a.sql", "SELECT 1"),
            ("b.sql", "SELECT 2"),
            ("control.sql", "SELECT 'control'"),
        ],
    );
    r.connected("orders-prod");
    r.connected("orders-staging");
    let staging = r.staging;
    let control = r.open("control.sql", staging);
    r.wait_session(control);
    let tab = r.open("a.sql", staging);
    r.wait_session(tab);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "b", "b.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser is open");
    assert_eq!(rows.0.len(), 2, "{rows:?}");
    // The Run chord, then the command by name, while the chooser has the focus.
    let chord = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    press_key(r.vcx, chord);
    let _ = run(r.vcx, "editor.run");
    // Escape closes the chooser and opens nothing.
    press_key(r.vcx, "escape");
    assert!(chooser(r.vcx).is_none());
    assert_eq!(tab_count(r.vcx), before);
    // The control and barrier: the same chord on the other tab of the same connection runs
    // that tab's own, distinct text. A Run the presses above leaked (`a.sql`'s `SELECT 1`, or
    // `b.sql`'s `SELECT 2`, on either connection) is in the logs before it.
    r.focus(control);
    press_key(r.vcx, chord);
    wait_until(r.vcx, "the control ran", |_| {
        queries(&r.fakes[1]).contains(&"SELECT 'control'".to_string())
    });
    r.quiesce(control);
    r.quiesce(tab);
    assert_eq!(
        r.sent(),
        (vec![], vec!["SELECT 'control'".to_string()]),
        "exactly the control, on staging: nothing ran under the chooser"
    );
}

/// A profile retagged staging → production while its script tab is open: the capsule shows
/// production now (never the environment the tab was opened under), and Run still reaches the
/// tab's own connection only.
#[gpui::test]
fn a_retag_while_the_tab_is_open_shows_the_new_environment_and_runs_where_bound(
    cx: &mut TestAppContext,
) {
    let mut r = rig(cx, &[("a.sql", "SELECT 1")]);
    r.connected("orders-prod");
    r.connected("orders-staging");
    let staging = r.staging;
    let tab = r.open("a.sql", staging);
    r.wait_session(tab);
    r.focus(tab);
    redraw_full(r.vcx);
    assert_eq!(
        capsule(r.vcx),
        Some(("orders-staging".to_string(), EnvKind::Staging))
    );
    retag(r.vcx, staging, Some(EnvTag::Production));
    r.focus(tab);
    wait_until(r.vcx, "the capsule shows production now", |vcx| {
        redraw_full(vcx);
        capsule(vcx) == Some(("orders-staging".to_string(), EnvKind::Production))
    });
    let spec = doc(r.vcx, tab)
        .and_then(|d| r.vcx.cx.read(|cx| d.read(cx).spec(cx)))
        .expect("a binding");
    assert_eq!(spec.profile, Some(staging), "the binding never moved");
    r.run(tab);
    wait_until(r.vcx, "its own connection ran it", |_| {
        queries(&r.fakes[1]) == ["SELECT 1"]
    });
    assert!(
        queries(&r.fakes[0]).is_empty(),
        "the other connection saw nothing"
    );
}

/// A file moved into `@production/` while it is open under staging: a Run press on the tab
/// sends nothing to either connection; Go to script offers the moved file only where
/// production shows it, with no row preselected, and Enter there opens nothing.
#[gpui::test]
fn a_file_moved_into_production_while_open_under_staging_runs_nothing(cx: &mut TestAppContext) {
    let mut r = rig(
        cx,
        &[
            ("a.sql", "SELECT 1"),
            ("@production/keep.sql", "SELECT 0"),
            ("barrier.sql", BARRIER),
        ],
    );
    r.connected("orders-prod");
    r.connected("orders-staging");
    let staging = r.staging;
    let barrier = r.open("barrier.sql", staging);
    r.wait_session(barrier);
    let tab = r.open("a.sql", staging);
    r.wait_session(tab);
    std::fs::rename(r.root.join("a.sql"), r.root.join("@production/a.sql")).unwrap();
    let (from, to) = (r.root.join("a.sql"), r.root.join("@production/a.sql"));
    assert!(
        r.watch
            .emit(&r.root, vec![RawHint::Path(from), RawHint::Path(to)])
    );
    wait_until(r.vcx, "no longer listed", |vcx| {
        doc(vcx, tab).is_some_and(|d| {
            vcx.cx.read(|cx| {
                d.read(cx)
                    .notices()
                    .iter()
                    .any(|n| matches!(n, tabs::Notice::Unlisted(_)))
            })
        })
    });
    // Run on the tab, by the command and by the chord: nothing reaches either connection.
    // (Fix round 1, review I2: each press is followed by a barrier run on another staging
    // tab, so a leaked Run is in the log before the check.)
    r.run(tab);
    r.barrier(barrier, tab, BARRIER);
    assert_eq!(
        r.leaked(BARRIER),
        (vec![], vec![]),
        "the command ran nothing"
    );
    r.focus(tab);
    let chord = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    press_key(r.vcx, chord);
    r.barrier(barrier, tab, BARRIER);
    assert_eq!(r.leaked(BARRIER), (vec![], vec![]), "nothing ran");
    // Go to script offers it only under production, never preselected.
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "@production/a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("a production candidate still asks");
    assert_eq!(
        rows.0,
        [("orders-prod".to_string(), "orders".to_string())],
        "only where production shows it"
    );
    assert_eq!(rows.1, None, "production is never preselected");
    press_key(r.vcx, "enter");
    assert_eq!(tab_count(r.vcx), before, "Enter opened nothing");
    r.barrier(barrier, tab, BARRIER);
    assert_eq!(r.leaked(BARRIER), (vec![], vec![]), "still nothing ran");
    let _ = r.prod;
}

// ---- Review Focus 5: a script stranded by an interrupted rename is said, never lost ------------

/// T13 fr1 re-review n5, the app half: the library's diagnostics popover names the script left
/// under a case-only rename's temp name and the folder it is in, so the user can rename it
/// back; the name is shown escaped, and nothing deletes the file.
#[gpui::test]
fn a_stranded_rename_temp_is_named_in_the_diagnostics(cx: &mut TestAppContext) {
    let temp = format!(".hv-rename-{}", "0123456789abcdef".repeat(2));
    let stranded = format!("reports/{temp}");
    let r = rig(
        cx,
        &[
            ("reports/a.sql", "SELECT 1"),
            (stranded.as_str(), "SELECT 2"),
        ],
    );
    let lib = r.lib;
    let (_, lines) = r.vcx.cx.read(|cx| {
        let snap = r.service.snapshot();
        let view = snap.view(None);
        let l = view.library(lib).expect("the library");
        hacivat_app::scripts::diagnostics::content_for_tests(&l, cx)
    });
    let lines: Vec<String> = lines
        .iter()
        .map(|l| {
            l.chars()
                .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
                .collect()
        })
        .collect();
    let want = format!(
        "A rename was interrupted: one of your scripts is in reports under the temporary name \
         {temp}, which isn't listed. Rename it back in your file manager."
    );
    assert!(lines.contains(&want), "{lines:?}");
    assert!(r.root.join(&stranded).is_file(), "the script is kept");
}
// ## m4b-t20-adversarial
