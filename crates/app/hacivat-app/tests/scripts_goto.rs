// ## m4b-t15-goto
//! M4b Task 15: Go to script and the connection chooser (HV-SCRIPT-09, R-M4b-9, R-M4b-31;
//! NOTES surface 7). A real `LibraryService` over fixture folders (fake event sources) feeds
//! the layer; profiles are saved through the model, and each library folder binds to the
//! database of its name. The safety rules: production is never preselected unless it is the
//! active tab's connection, in every entry path (Go to script, the tree row's and the tab's
//! "Open under another connection…"); a hidden-scope file never shows for a connection that
//! does not show it; the chooser offers only connections whose view lists the file; one tab
//! per (file, connection).
//!
//! Keys go through `support::press_key` (down and up), chords are the host's (`cmd` on macOS,
//! `ctrl` elsewhere), window text is read with `support::full_frame`, and every wait is on a
//! condition.

mod support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, FocusHandle, TestAppContext, VisualTestContext};
use hacivat_app::connections::ConnectionsModel;
use hacivat_app::connections::explorer as conn_explorer;
use hacivat_app::explorer::adapter;
use hacivat_app::scripts::goto::{self, GotoHost, Seen};
use hacivat_app::scripts::open::{open_script, open_under_another};
use hacivat_app::scripts::tabs::{self, Load, ScriptTab};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath, projection};
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::CatalogSnapshot;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::LibraryState;
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_session::profile::{Profile, ProfileDraft};
use hacivat_ui::panes::TabId;
use hacivat_ui::tree::TreeKey;
use support::catalogs::*;
use support::*;

// ---- the rig ---------------------------------------------------------------------------------

struct Rig {
    vcx: &'static mut VisualTestContext,
    svc: Arc<LibraryService>,
    fake: FakeHandle,
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

/// A shell in `lang` with `profiles` saved and a library service over fake event sources.
fn rig_lang(cx: &mut TestAppContext, lang: &str, profiles: Vec<Profile>) -> Rig {
    let vcx = shell_with_fake(cx, lang, answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    in_window(vcx, |window, _| window.activate_window());
    for p in profiles {
        add_profile(vcx, p);
    }
    let paths = paths(vcx);
    let rt = vcx.cx.read(|cx| cx.global::<ConnectionsModel>().runtime());
    let fake = FakeHandle::default();
    let svc = Arc::new(LibraryService::open_with(
        &paths,
        &rt,
        fake.sources(),
        opts(),
    ));
    let s = svc.clone();
    vcx.cx.update(|cx| projection::set_service(s, cx));
    wait_until(vcx, "the service to load", |_| {
        svc.snapshot().state == LibraryState::Watching
    });
    Rig { vcx, svc, fake }
}

fn rig(cx: &mut TestAppContext, profiles: Vec<Profile>) -> Rig {
    rig_lang(cx, "en", profiles)
}

/// Runs `fut` on the UI executor and waits for its output.
fn block<T: 'static>(
    vcx: &VisualTestContext,
    fut: impl std::future::Future<Output = T> + 'static,
) -> T {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    vcx.cx.update(|cx| {
        cx.spawn(async move |_| {
            *slot.borrow_mut() = Some(fut.await);
        })
        .detach();
    });
    wait_until(vcx, "the future to finish", |_| out.borrow().is_some());
    out.borrow_mut().take().expect("its output")
}

fn write_files(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("SELECT 1; -- {f}")).unwrap();
    }
}

impl Rig {
    /// Attaches a folder named `name` holding `files` (it binds to the database `name`) and
    /// waits for its first scan.
    fn library(&self, name: &str, files: &[&str]) -> LibraryId {
        self.library_at(name, files).0
    }

    /// [`Rig::library`], with the library's folder.
    fn library_at(&self, name: &str, files: &[&str]) -> (LibraryId, PathBuf) {
        self.library_with(name, files, None)
    }

    /// [`Rig::library_at`], with `.hacivat/workspace.toml` holding `manifest` (fix round 1: the
    /// reviewer's alias probes).
    fn library_with(
        &self,
        name: &str,
        files: &[&str],
        manifest: Option<&str>,
    ) -> (LibraryId, PathBuf) {
        let dir = temp_root().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        write_files(&dir, files);
        if let Some(text) = manifest {
            std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
            std::fs::write(dir.join(".hacivat/workspace.toml"), text).unwrap();
        }
        let dir = std::fs::canonicalize(&dir).unwrap();
        let svc = self.svc.clone();
        let root = dir.clone();
        let id =
            block(self.vcx, async move { svc.attach(dir, None, false).await }).expect("attached");
        let svc = self.svc.clone();
        wait_until(self.vcx, "the first scan", |_| {
            svc.snapshot()
                .view(None)
                .library(id)
                .is_some_and(|l| l.library().state == LibraryState::Watching)
        });
        let svc = self.svc.clone();
        block(self.vcx, async move { svc.settled().await });
        (id, root)
    }

    /// Opens `rel` of `lib` under `profile` and `db` and waits until its tab shows the file.
    fn open(&self, lib: LibraryId, rel: &str, profile: Option<ProfileId>, db: &str) -> TabId {
        let rel_path = RelPath::parse(rel).unwrap();
        let database = (!db.is_empty()).then(|| DatabaseName::new(db));
        self.vcx.cx.update(|cx| {
            open_script(cx, lib, rel_path, profile, database);
        });
        let want = rel.to_string();
        wait_until(self.vcx, "the script tab shows its file", |vcx| {
            active_spec(vcx)
                .is_some_and(|(s, ready)| ready && s.rel.as_str() == want && s.profile == profile)
        });
        self.active()
    }

    fn active(&self) -> TabId {
        self.vcx.run_until_parked();
        let ws = workspace(self.vcx);
        self.vcx
            .cx
            .read(|cx| ws.read(cx).active_tab())
            .expect("a tab is active")
    }

    /// Drops the service before the core runtime stops (its actor ends first).
    fn finish(self) {
        let Rig { vcx, svc, .. } = self;
        let taken = vcx.cx.update(projection::take_service);
        if let Some(s) = &taken {
            s.shutdown();
        }
        drop(taken);
        drop(svc);
        vcx.run_until_parked();
    }
}

fn prof(name: &str, env: Option<EnvTag>, database: &str) -> Profile {
    let mut p = test_profile(name, env, "127.0.0.1");
    p.database = Some(database.to_string());
    p
}

/// The active tab's script binding, and whether its file is read.
fn active_spec(vcx: &VisualTestContext) -> Option<(ScriptTab, bool)> {
    let ws = workspace(vcx);
    let tab = vcx.cx.read(|cx| ws.read(cx).active_tab())?;
    vcx.cx.read(|cx| {
        let doc = tabs::doc_of_tab(tab, cx)?;
        let d = doc.read(cx);
        Some((d.spec(cx)?, *d.load() == Load::Ready))
    })
}

/// The host chord for `key` (`cmd-…` on macOS, `ctrl-…` elsewhere).
fn secondary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

fn host(vcx: &VisualTestContext) -> Entity<GotoHost> {
    vcx.cx
        .read(goto::open_host)
        .expect("the Go-to-script layer")
}

fn is_open(vcx: &VisualTestContext) -> bool {
    layer_ids(vcx).contains(&goto::GOTO_LAYER)
}

/// Opens Go to script with its command and types `q`; waits for a result row `want`.
fn goto_query(vcx: &mut VisualTestContext, q: &str, want: &str) {
    assert!(run(vcx, "scripts.goto"), "scripts.goto ran");
    assert!(is_open(vcx));
    vcx.simulate_input(q);
    let want = want.to_string();
    wait_until(vcx, "the result row", |vcx| {
        let h = host(vcx);
        vcx.cx.read(|cx| h.read(cx).result_rows().contains(&want))
    });
}

type Rows = (Vec<(String, String)>, Option<usize>);

fn chooser(vcx: &VisualTestContext) -> Option<Rows> {
    vcx.run_until_parked();
    let h = vcx.cx.read(goto::open_host)?;
    vcx.cx.read(|cx| h.read(cx).chooser_rows())
}

fn names(rows: &Rows) -> Vec<&str> {
    rows.0.iter().map(|(n, _)| n.as_str()).collect()
}

/// The name of the chooser's active row.
fn active_name(rows: &Rows) -> Option<&str> {
    rows.1.map(|i| rows.0[i].0.as_str())
}

fn env_of(vcx: &VisualTestContext, id: ProfileId) -> Option<EnvTag> {
    vcx.cx.read(|cx| {
        cx.global::<ConnectionsModel>()
            .get(id)
            .and_then(|p| p.environment)
    })
}

fn retag(vcx: &VisualTestContext, id: ProfileId, env: Option<EnvTag>) {
    let p = vcx
        .cx
        .read(|cx| cx.global::<ConnectionsModel>().get(id).unwrap());
    let mut edited = (*p).clone();
    edited.environment = env;
    save_draft(vcx, ProfileDraft::edit(edited));
    assert_eq!(env_of(vcx, id), env);
}

fn pg_catalog(databases: &[&str]) -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::Postgres,
        databases
            .iter()
            .map(|db| {
                schema(
                    DbmsKind::Postgres,
                    &[db, "public"],
                    false,
                    vec![table("orders", Some(16_400), &[("id", "bigint")])],
                )
            })
            .collect(),
    )
}

fn show(vcx: &VisualTestContext, profile: ProfileId, cat: Arc<CatalogSnapshot>) {
    vcx.cx
        .update(|cx| adapter::set_catalog(profile, Some(cat), None, cx));
    vcx.run_until_parked();
}

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

/// The tree row of file `label` under `profile`'s Scripts node of database `db`.
fn file_row(vcx: &VisualTestContext, profile: ProfileId, db: &str, label: &str) -> TreeKey {
    file_row_under(vcx, profile, adapter::database_key(profile, db), label)
}

/// The tree row of file `label` under the Scripts node below `parent` (a database row, or a
/// MySQL schema row) of `profile`.
fn file_row_under(
    vcx: &VisualTestContext,
    profile: ProfileId,
    parent: TreeKey,
    label: &str,
) -> TreeKey {
    let set_expanded = |key: TreeKey| {
        vcx.cx.update(|cx| {
            let tree = conn_explorer::tree(cx).expect("the tree");
            tree.update(cx, |t, cx| t.set_expanded(key, true, cx));
        });
        vcx.run_until_parked();
    };
    set_expanded(adapter::profile_key(profile));
    set_expanded(parent);
    // The rows below `key` (its subtree), as shown.
    let below = |key: TreeKey| {
        vcx.run_until_parked();
        let all = vcx.cx.read(conn_explorer::shown_rows);
        let ix = all
            .iter()
            .position(|r| r.key == key && r.status.is_none())
            .expect("the row is shown");
        let depth = all[ix].depth;
        all[ix + 1..]
            .iter()
            .take_while(|r| r.depth > depth)
            .cloned()
            .collect::<Vec<_>>()
    };
    let scripts = below(parent)
        .into_iter()
        .find(|r| strip(&r.data.label) == "Scripts" && r.status.is_none())
        .expect("a Scripts node");
    set_expanded(scripts.key);
    below(scripts.key)
        .into_iter()
        .find(|r| strip(&r.data.label) == label && r.status.is_none())
        .map(|r| r.key)
        .unwrap_or_else(|| panic!("`{label}` under the Scripts node"))
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = conn_explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

fn focused_handle(vcx: &VisualTestContext) -> Option<FocusHandle> {
    focused(vcx)
}

// ---- the plan's tests ----------------------------------------------------------------------

#[gpui::test]
fn goto_finds_by_turkish_tolerant_name(cx: &mut TestAppContext) {
    let r = rig_lang(
        cx,
        "tr",
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    r.library(
        "orders",
        &[
            "reports/müşteri_segment.sql",
            "reports/daily.sql",
            "İSTANBUL.sql",
        ],
    );
    goto_query(r.vcx, "musteri", "reports/müşteri_segment.sql · orders");
    // The window shows the name, with the folder and the library.
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame.iter().any(|t| t == "müşteri_segment.sql"),
        "{frame:?}"
    );
    assert!(frame.iter().any(|t| t == "reports/"), "{frame:?}");
    // `istanbul` finds `İSTANBUL.sql` (I18N-09 folding).
    let input = r.vcx.cx.read(|cx| host(r.vcx).read(cx).input().clone());
    r.vcx.cx.update(|cx| {
        input.update(cx, |i, cx| {
            i.set_value("");
            cx.notify();
        })
    });
    r.vcx.simulate_input("x");
    press_key(r.vcx, "backspace");
    r.vcx.simulate_input("istanbul");
    wait_until(r.vcx, "İSTANBUL.sql", |vcx| {
        let h = host(vcx);
        vcx.cx.read(|cx| {
            h.read(cx)
                .result_rows()
                .first()
                .is_some_and(|row| row == "İSTANBUL.sql · orders")
        })
    });
    r.finish();
}

#[gpui::test]
fn single_non_production_candidate_opens_directly(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    let staging = profile_id(r.vcx, "orders-staging");
    r.library("orders", &["a.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "a.sql opens under staging", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| ready && s.profile == Some(staging))
    });
    assert!(!is_open(r.vcx), "no chooser, the layer closed");
    assert_eq!(tab_count(r.vcx), before + 1);
    let (spec, _) = active_spec(r.vcx).unwrap();
    assert_eq!(spec.database, Some(DatabaseName::new("orders")));
    r.finish();
}

#[gpui::test]
fn single_production_candidate_still_asks(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    r.library("orders", &["a.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser asks");
    assert_eq!(names(&rows), ["orders-prod"]);
    assert_eq!(rows.1, None, "production is not the active row");
    // The footer says why.
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame
            .iter()
            .any(|t| t == "Production is never chosen for you."),
        "{frame:?}"
    );
    // ↵ does nothing until the user moves onto it.
    press_key(r.vcx, "enter");
    assert!(is_open(r.vcx));
    assert_eq!(tab_count(r.vcx), before);
    press_key(r.vcx, "down");
    assert_eq!(active_name(&chooser(r.vcx).unwrap()), Some("orders-prod"));
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "a.sql opens under prod", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| ready && s.profile == Some(prod))
    });
    assert!(!is_open(r.vcx));
    r.finish();
}

#[gpui::test]
fn chooser_never_preselects_production(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            prof("billing-dev", Some(EnvTag::Dev), "billing"),
        ],
    );
    let dev = profile_id(r.vcx, "billing-dev");
    let orders = r.library("orders", &["a.sql"]);
    let billing = r.library("billing", &["b.sql"]);
    let _ = orders;
    // The active tab is on a dev connection that does not show a.sql.
    r.open(billing, "b.sql", Some(dev), "billing");
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("two candidates ask");
    assert_eq!(names(&rows), ["orders-staging", "orders-prod"]);
    assert_eq!(active_name(&rows), Some("orders-staging"));
    r.finish();
}

#[gpui::test]
fn chooser_preselects_the_focused_connection_even_if_production(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    let lib = r.library("orders", &["a.sql", "b.sql"]);
    // The user works on production: its tab is the active one.
    r.open(lib, "a.sql", Some(prod), "orders");
    goto_query(r.vcx, "b", "b.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(names(&rows), ["orders-prod", "orders-staging"]);
    assert_eq!(active_name(&rows), Some("orders-prod"));
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(frame.iter().any(|t| t == "current"), "{frame:?}");
    // Production is the focused connection: the footer note is not shown.
    assert!(
        !frame
            .iter()
            .any(|t| t == "Production is never chosen for you."),
        "{frame:?}"
    );
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "b.sql opens under prod", |vcx| {
        active_spec(vcx)
            .is_some_and(|(s, ready)| ready && s.rel.as_str() == "b.sql" && s.profile == Some(prod))
    });
    r.finish();
}

#[gpui::test]
fn production_scoped_file_offers_only_production(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    r.library("orders", &["@production/purge.sql", "a.sql"]);
    goto_query(r.vcx, "purge", "@production/purge.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(names(&rows), ["orders-prod"], "staging does not list it");
    assert_eq!(rows.1, None);
    r.finish();
}

#[gpui::test]
fn escape_opens_nothing(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let lib = r.library("orders", &["a.sql", "b.sql"]);
    let staging = profile_id(r.vcx, "orders-staging");
    r.open(lib, "b.sql", Some(staging), "orders");
    let before_focus = focused_handle(r.vcx).expect("the editor has the focus");
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    assert!(chooser(r.vcx).is_some());
    // Escape: back to the results, with the query.
    press_key(r.vcx, "escape");
    assert!(is_open(r.vcx));
    let h = host(r.vcx);
    assert!(!r.vcx.cx.read(|cx| h.read(cx).is_choosing()));
    let input = r.vcx.cx.read(|cx| h.read(cx).input().clone());
    assert_eq!(r.vcx.cx.read(|cx| input.read(cx).value().to_string()), "a");
    // Escape again: closed, nothing opened, the focus back where it was.
    press_key(r.vcx, "escape");
    assert!(!is_open(r.vcx));
    assert_eq!(tab_count(r.vcx), before);
    assert_focus_on(r.vcx, "the editor it came from", &before_focus);
    r.finish();
}

#[gpui::test]
fn no_candidate_offers_unbound_open(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    // `misc` binds to no connection's database.
    r.library("misc", &["notes.sql", "@staging/s.sql"]);
    goto_query(r.vcx, "notes", "notes.sql · misc");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert!(rows.0.is_empty());
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame.iter().any(|t| t == "No connection shows this file."),
        "{frame:?}"
    );
    assert!(
        frame.iter().any(|t| t == "Open without a connection"),
        "{frame:?}"
    );
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "an unbound tab", |vcx| {
        active_spec(vcx)
            .is_some_and(|(s, ready)| ready && s.rel.as_str() == "notes.sql" && s.profile.is_none())
    });
    // A scoped file of a library no connection shows is never offered (an unbound tab cannot
    // show it).
    assert!(run(r.vcx, "scripts.goto"));
    r.vcx.simulate_input("s");
    wait_until(r.vcx, "the results", |vcx| {
        let h = host(vcx);
        vcx.cx.read(|cx| !h.read(cx).result_rows().is_empty())
    });
    let h = host(r.vcx);
    let rows = r.vcx.cx.read(|cx| h.read(cx).result_rows());
    assert!(rows.iter().all(|r| !r.contains("@staging")), "{rows:?}");
    r.finish();
}

#[gpui::test]
fn goto_is_reachable_from_palette_and_keys(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    r.library("orders", &["a.sql"]);
    // ⇧⌘O / Ctrl+Shift+O.
    r.vcx.simulate_keystrokes(&secondary("shift-o"));
    r.vcx.run_until_parked();
    assert!(is_open(r.vcx), "the chord opens it");
    press_key(r.vcx, "escape");
    assert!(!is_open(r.vcx));
    // The command palette lists it.
    assert!(run(r.vcx, "workspace.command_palette"));
    r.vcx.simulate_input("Go to script");
    let palette = r
        .vcx
        .cx
        .read(hacivat_app::commands::palette::open_palette_view)
        .expect("the palette");
    let ids: Vec<String> = r.vcx.cx.read(|cx| {
        palette
            .read(cx)
            .items()
            .iter()
            .map(|i| i.id.to_string())
            .collect()
    });
    assert_eq!(
        ids.first().map(String::as_str),
        Some("scripts.goto"),
        "{ids:?}"
    );
    press_key(r.vcx, "enter");
    assert!(is_open(r.vcx), "the palette entry opens it");
    r.finish();
}

#[gpui::test]
fn goto_offers_a_profile_never_expanded_on_its_configured_database(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            prof("orders-dev", Some(EnvTag::Dev), "orders"),
        ],
    );
    let (staging, dev) = (
        profile_id(r.vcx, "orders-staging"),
        profile_id(r.vcx, "orders-dev"),
    );
    r.library("orders", &["a.sql"]);
    // Neither profile was ever expanded: no catalog, so the tree shows no Scripts node.
    for id in [staging, dev] {
        assert!(
            r.vcx
                .cx
                .read(|cx| adapter::catalog_of(id, cx).is_none_or(|c| c.generation == 0))
        );
    }
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        rows.0,
        [
            ("orders-dev".to_string(), "orders".to_string()),
            ("orders-staging".to_string(), "orders".to_string()),
        ]
    );
    assert_eq!(active_name(&rows), Some("orders-dev"));
    r.finish();
}

#[gpui::test]
fn chooser_keeps_the_focus_when_its_candidates_change(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let staging = profile_id(r.vcx, "orders-staging");
    r.library("orders", &["a.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(active_name(&rows), Some("orders-staging"));
    let h = host(r.vcx);
    let chooser_focus = r.vcx.cx.read(|cx| h.read(cx).chooser_focus().clone());
    assert_focus_on(r.vcx, "the chooser", &chooser_focus);
    // The user moves onto staging on purpose (down to prod, back up).
    press_key(r.vcx, "down");
    press_key(r.vcx, "up");
    assert_eq!(
        active_name(&chooser(r.vcx).unwrap()),
        Some("orders-staging")
    );
    // Staging becomes production while the chooser is open: the row the user chose was a
    // staging row, so it is not kept active as a production one.
    retag(r.vcx, staging, Some(EnvTag::Production));
    redraw_full(r.vcx);
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(
        names(&rows),
        ["orders-prod", "orders-staging"],
        "recomputed"
    );
    assert_eq!(rows.1, None, "production is still not preselected");
    assert_focus_on(r.vcx, "the chooser", &chooser_focus);
    // Enter opens nothing.
    press_key(r.vcx, "enter");
    assert_eq!(tab_count(r.vcx), before);
    // The user moves onto staging (now production) on purpose; a retag back to staging is
    // another place in another environment: the preselection rule applies again.
    press_key(r.vcx, "down");
    press_key(r.vcx, "down");
    assert_eq!(
        active_name(&chooser(r.vcx).unwrap()),
        Some("orders-staging")
    );
    retag(r.vcx, staging, Some(EnvTag::Staging));
    redraw_full(r.vcx);
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(names(&rows), ["orders-staging", "orders-prod"]);
    assert_eq!(active_name(&rows), Some("orders-staging"));
    assert_focus_on(r.vcx, "the chooser", &chooser_focus);
    r.finish();
}

// ---- safety: scope, entry paths, one tab per (file, connection) ------------------------------

#[gpui::test]
fn hidden_scope_files_never_show_for_a_connection_that_does_not_show_them(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            prof("billing-prod", Some(EnvTag::Production), "billing"),
        ],
    );
    // Only staging shows `orders`; production shows `billing`.
    r.library(
        "orders",
        &[
            "a.sql",
            "@production/p.sql",
            "@staging/s.sql",
            "@prod/x.sql",
        ],
    );
    r.library("billing", &["b.sql", "@production/q.sql"]);
    assert!(run(r.vcx, "scripts.goto"));
    wait_until(r.vcx, "the empty query's rows", |vcx| {
        let h = host(vcx);
        vcx.cx.read(|cx| h.read(cx).result_rows().len() >= 4)
    });
    let h = host(r.vcx);
    let mut rows = r.vcx.cx.read(|cx| h.read(cx).result_rows());
    rows.sort();
    assert_eq!(
        rows,
        [
            "@production/q.sql · billing",
            "@staging/s.sql · orders",
            "a.sql · orders",
            "b.sql · billing",
        ]
    );
    // The window draws no hidden name either.
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        !frame.iter().any(|t| t == "p.sql" || t == "x.sql"),
        "{frame:?}"
    );
    // Typed, it is still not found.
    r.vcx.simulate_input("p.sql");
    r.vcx.run_until_parked();
    let rows = r.vcx.cx.read(|cx| h.read(cx).result_rows());
    assert!(
        rows.iter()
            .all(|r| !r.ends_with("· orders") || !r.contains("p.sql")),
        "{rows:?}"
    );
    r.finish();
}

#[gpui::test]
fn chooser_offers_only_connections_whose_view_lists_the_file(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            prof("orders-dev", Some(EnvTag::Dev), "orders"),
            prof("orders-untagged", None, "orders"),
        ],
    );
    let staging = profile_id(r.vcx, "orders-staging");
    r.library("orders", &["@staging/s.sql", "a.sql"]);
    // A `@staging` file: only staging lists it, so it opens there directly.
    goto_query(r.vcx, "s.sql", "@staging/s.sql · orders");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "s.sql opens under staging", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| ready && s.profile == Some(staging))
    });
    // An unscoped file: every connection, in R-M4b-9's order (untagged after production).
    goto_query(r.vcx, "a.sql", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        names(&rows),
        [
            "orders-staging",
            "orders-dev",
            "orders-prod",
            "orders-untagged"
        ],
        "the active tab's connection first, then dev, then production, then untagged"
    );
    assert_eq!(active_name(&rows), Some("orders-staging"));
    r.finish();
}

#[gpui::test]
fn first_row_production_by_sort_is_never_active(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-untagged", None, "orders"),
        ],
    );
    r.library("orders", &["a.sql"]);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(names(&rows), ["orders-prod", "orders-untagged"]);
    assert_eq!(active_name(&rows), Some("orders-untagged"));
    r.finish();
}

#[gpui::test]
fn picking_a_connection_with_the_file_open_focuses_that_tab(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let staging = profile_id(r.vcx, "orders-staging");
    let lib = r.library("orders", &["a.sql", "b.sql"]);
    let a_tab = r.open(lib, "a.sql", Some(staging), "orders");
    let b_tab = r.open(lib, "b.sql", Some(staging), "orders");
    assert_ne!(a_tab, b_tab);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(
        active_name(&rows),
        Some("orders-staging"),
        "the active tab's"
    );
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the open tab takes the focus", |_| {
        r.active() == a_tab
    });
    assert_eq!(
        tab_count(r.vcx),
        before,
        "no second tab for (a.sql, staging)"
    );
    r.finish();
}

#[gpui::test]
fn tree_open_under_another_connection_leaves_out_its_row_and_never_preselects_production(
    cx: &mut TestAppContext,
) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            prof("orders-dev", Some(EnvTag::Dev), "orders"),
        ],
    );
    let (prod, staging, dev) = (
        profile_id(r.vcx, "orders-prod"),
        profile_id(r.vcx, "orders-staging"),
        profile_id(r.vcx, "orders-dev"),
    );
    for id in [prod, staging, dev] {
        show(r.vcx, id, pg_catalog(&["orders"]));
    }
    let lib = r.library("orders", &["a.sql", "b.sql"]);
    // The active tab is on dev; the row is under prod.
    r.open(lib, "b.sql", Some(dev), "orders");
    let row = file_row(r.vcx, prod, "orders", "a.sql");
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, row);
    assert!(run(r.vcx, "scripts.open_under"), "enabled on the file row");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        names(&rows),
        ["orders-dev", "orders-staging"],
        "prod left out"
    );
    assert_eq!(active_name(&rows), Some("orders-dev"));
    // Escape closes it (it did not come from Go to script's results).
    press_key(r.vcx, "escape");
    assert!(!is_open(r.vcx));
    // The row under staging: prod is offered, never preselected; dev (the active tab's) is.
    let row = file_row(r.vcx, staging, "orders", "a.sql");
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, row);
    let menu = r
        .vcx
        .cx
        .read(|cx| hacivat_app::explorer::node_menu(row, cx))
        .expect("the row's menu");
    let item = menu
        .iter()
        .find(|i| i.action == "scripts.open_under")
        .expect("Open under another connection…");
    assert!(!item.disabled, "enabled: other connections show the file");
    assert!(run(r.vcx, "scripts.open_under"));
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        names(&rows),
        ["orders-dev", "orders-prod"],
        "staging left out"
    );
    assert_eq!(active_name(&rows), Some("orders-dev"));
    press_key(r.vcx, "escape");
    r.finish();
}

#[gpui::test]
fn tree_open_under_with_only_production_elsewhere_has_no_active_row(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let (prod, staging) = (
        profile_id(r.vcx, "orders-prod"),
        profile_id(r.vcx, "orders-staging"),
    );
    for id in [prod, staging] {
        show(r.vcx, id, pg_catalog(&["orders"]));
    }
    r.library("orders", &["a.sql"]);
    // No tab is open: the tree's selection (a row under staging) is not "the focused
    // connection".
    let row = file_row(r.vcx, staging, "orders", "a.sql");
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, row);
    let before = tab_count(r.vcx);
    assert!(run(r.vcx, "scripts.open_under"));
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(names(&rows), ["orders-prod"]);
    assert_eq!(rows.1, None);
    press_key(r.vcx, "enter");
    assert_eq!(tab_count(r.vcx), before, "↵ opens nothing");
    press_key(r.vcx, "escape");
    assert!(!is_open(r.vcx));
    r.finish();
}

#[gpui::test]
fn tab_open_under_another_connection_leaves_out_its_own_connection(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let (prod, staging) = (
        profile_id(r.vcx, "orders-prod"),
        profile_id(r.vcx, "orders-staging"),
    );
    let lib = r.library("orders", &["a.sql"]);
    let on_staging = r.open(lib, "a.sql", Some(staging), "orders");
    let spec = active_spec(r.vcx).unwrap().0;
    let before = tab_count(r.vcx);
    r.vcx.cx.update(|cx| open_under_another(&spec, cx));
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        names(&rows),
        ["orders-prod"],
        "its own connection is left out"
    );
    assert_eq!(rows.1, None, "production elsewhere is never preselected");
    press_key(r.vcx, "enter");
    assert_eq!(
        tab_count(r.vcx),
        before,
        "↵ with no active row opens nothing"
    );
    press_key(r.vcx, "down");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "a second tab under prod", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| ready && s.profile == Some(prod))
    });
    assert_eq!(tab_count(r.vcx), before + 1);
    // The first tab is still on staging: nothing rebinds a tab.
    let first = r
        .vcx
        .cx
        .read(|cx| tabs::doc_of_tab(on_staging, cx).and_then(|d| d.read(cx).spec(cx)));
    assert_eq!(first.and_then(|s| s.profile), Some(staging));
    r.finish();
}

/// Review probe "Enter racing the candidate computation": the file moves into `@production` on
/// disk while the chooser shows staging; Enter, dispatched before the service's event reaches
/// the layer, still checks the live snapshot and opens nothing under staging.
#[gpui::test]
fn enter_checks_the_live_snapshot_before_opening(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
        ],
    );
    let staging = profile_id(r.vcx, "orders-staging");
    let (_, root) = r.library_at("orders", &["a.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    assert_eq!(
        active_name(&chooser(r.vcx).unwrap()),
        Some("orders-staging")
    );
    // a.sql becomes @production/a.sql on disk; the service rescans on the core runtime.
    std::fs::create_dir_all(root.join("@production")).unwrap();
    std::fs::rename(root.join("a.sql"), root.join("@production/a.sql")).unwrap();
    let batch = vec![
        hacivat_scripts::RawHint::Path(root.join("a.sql")),
        hacivat_scripts::RawHint::Path(root.join("@production/a.sql")),
    ];
    assert!(r.fake.emit(&root, batch), "the library is watched");
    let svc = r.svc.clone();
    poll_until("the snapshot no longer lists a.sql for staging", || {
        let snap = svc.snapshot();
        let lib = snap.libraries.first().map(|l| l.id);
        lib.is_some_and(|id| {
            snap.view(Some(EnvTag::Staging))
                .library(id)
                .is_some_and(|l| l.entry(&RelPath::parse("a.sql").unwrap()).is_none())
        })
    });
    // The chooser still shows its old rows (the event has not reached the layer yet).
    let h = host(r.vcx);
    let shown = r.vcx.cx.read(|cx| h.read(cx).chooser_rows()).unwrap();
    assert_eq!(active_name(&shown), Some("orders-staging"));
    // Enter, at once (no executor turn in between).
    in_window(r.vcx, |window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx);
    });
    r.vcx.run_until_parked();
    assert_eq!(tab_count(r.vcx), before, "nothing opened on a stale row");
    assert!(
        active_spec(r.vcx).is_none_or(|(s, _)| s.profile != Some(staging)),
        "no tab under staging"
    );
    r.finish();
}

// ## m4b-t15-goto (controller note: `open_script` answers `None` for SQLite and opens the tab
// once the file's identity is read, R-T14-1)

/// A SQLite profile `name` over the file `path` (`main`, bound by the file's stem).
fn sqlite(name: &str, env: Option<EnvTag>, path: &Path) -> Profile {
    let mut p = test_profile(name, env, "127.0.0.1");
    p.engine = DbmsKind::Sqlite;
    p.database = None;
    p.endpoint = hacivat_session::profile::ProfileEndpoint::File {
        path: path.to_path_buf(),
        create_if_missing: false,
    };
    p
}

#[gpui::test]
fn sqlite_candidates_open_from_go_to_script_and_from_a_tab(cx: &mut TestAppContext) {
    let dir = temp_root();
    let file = dir.join("orders.db");
    std::fs::write(&file, b"").unwrap();
    let r = rig(
        cx,
        vec![
            prof("orders-staging", Some(EnvTag::Staging), "orders"),
            sqlite("local-orders", Some(EnvTag::Local), &file),
        ],
    );
    let (staging, local) = (
        profile_id(r.vcx, "orders-staging"),
        profile_id(r.vcx, "local-orders"),
    );
    let lib = r.library("orders", &["a.sql", "@local/l.sql"]);
    // A file only the SQLite connection shows: one non-production candidate, opened directly
    // (the tab opens once the file's identity is read).
    goto_query(r.vcx, "l.sql", "@local/l.sql · orders");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "l.sql opens under the SQLite connection", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| {
            ready && s.profile == Some(local) && s.database == Some(DatabaseName::new("main"))
        })
    });
    assert!(!is_open(r.vcx));
    // From a staging tab, "Open under another connection…" offers the SQLite one.
    r.open(lib, "a.sql", Some(staging), "orders");
    let spec = active_spec(r.vcx).unwrap().0;
    let before = tab_count(r.vcx);
    r.vcx.cx.update(|cx| open_under_another(&spec, cx));
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(rows.0, [("local-orders".to_string(), "main".to_string())]);
    assert_eq!(active_name(&rows), Some("local-orders"));
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "a.sql opens under the SQLite connection", |vcx| {
        active_spec(vcx).is_some_and(|(s, ready)| {
            ready && s.rel.as_str() == "a.sql" && s.profile == Some(local)
        })
    });
    assert_eq!(tab_count(r.vcx), before + 1);
    r.finish();
}

#[gpui::test]
fn a_sqlite_file_that_cannot_be_read_opens_nothing_and_says_why(cx: &mut TestAppContext) {
    let dir = temp_root();
    // The profile points at a file that is not there.
    let r = rig(
        cx,
        vec![sqlite(
            "local-orders",
            Some(EnvTag::Local),
            &dir.join("orders.db"),
        )],
    );
    r.library("orders", &["a.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "a", "a.sql · orders");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the refusal", |vcx| {
        toast_texts(vcx).iter().any(|t| t.contains("wasn't opened"))
    });
    assert!(!is_open(r.vcx), "the layer closed");
    assert_eq!(tab_count(r.vcx), before, "no tab");
    r.finish();
}

// ## m4b-t15-goto fix round 1: the review's probes (H1, H2) and minors.

/// MySQL profile `name` on database `db`.
fn mysql(name: &str, env: Option<EnvTag>, db: &str) -> Profile {
    let mut p = prof(name, env, db);
    p.engine = DbmsKind::MySql;
    p
}

fn mysql_catalog(schemas: &[&str]) -> Arc<CatalogSnapshot> {
    catalog(
        DbmsKind::MySql,
        schemas
            .iter()
            .map(|db| {
                schema(
                    DbmsKind::MySql,
                    &[db],
                    false,
                    vec![table("t", None, &[("id", "int")])],
                )
            })
            .collect(),
    )
}

/// Rewrites the library's manifest and waits until the snapshot reads `aliases` from it.
fn rewrite_manifest(r: &Rig, root: &Path, text: &str, aliases: usize) {
    std::fs::write(root.join(".hacivat/workspace.toml"), text).unwrap();
    let batch = vec![hacivat_scripts::RawHint::Path(
        root.join(".hacivat/workspace.toml"),
    )];
    assert!(r.fake.emit(root, batch), "the library is watched");
    let svc = r.svc.clone();
    poll_until("the snapshot reads the new manifest", || {
        svc.snapshot()
            .libraries
            .first()
            .is_some_and(|l| l.aliases.len() == aliases)
    });
}

/// H2 (C1, R-T15-1): "Open under another connection…" from a MySQL production tab on `api`,
/// whose library is also bound to `kampanya` through an alias, never offers `api-prod ·
/// kampanya` (the connection it starts from, every database of it).
#[gpui::test]
fn open_under_another_from_a_mysql_tab_never_offers_its_own_connection(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![
            mysql("api-prod", Some(EnvTag::Production), "api"),
            mysql("api-staging", Some(EnvTag::Staging), "api"),
        ],
    );
    let (prod, staging) = (
        profile_id(r.vcx, "api-prod"),
        profile_id(r.vcx, "api-staging"),
    );
    show(r.vcx, prod, mysql_catalog(&["api", "kampanya"]));
    let (lib, _) = r.library_with(
        "api",
        &["a.sql"],
        Some("format = 1\ndatabase = \"api\"\naliases = [\"kampanya\"]\n"),
    );
    // Both databases of api-prod list the file.
    let all = r.vcx.cx.read(|cx| {
        hacivat_app::scripts::chooser::candidates(lib, &RelPath::parse("a.sql").unwrap(), None, cx)
    });
    assert!(
        all.iter()
            .any(|c| c.profile == prod && c.database.as_str() == "kampanya"),
        "the fixture binds kampanya too: {all:?}"
    );
    r.open(lib, "a.sql", Some(prod), "api");
    let spec = active_spec(r.vcx).unwrap().0;
    r.vcx.cx.update(|cx| open_under_another(&spec, cx));
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(
        rows.0,
        [("api-staging".to_string(), "api".to_string())],
        "no database of api-prod is offered"
    );
    assert_eq!(active_name(&rows), Some("api-staging"));
    // The notice's button (T14's `other_places`) follows the same rule.
    let others = r
        .vcx
        .cx
        .read(|cx| hacivat_app::scripts::open::other_places(&spec, cx));
    assert_eq!(others, [(staging, DatabaseName::new("api"))]);
    press_key(r.vcx, "escape");
    r.finish();
}

/// C1 from the tree: the row's whole connection is left out, and the menu item is disabled when
/// only another database of that connection shows the file.
#[gpui::test]
fn tree_open_under_never_offers_another_database_of_the_rows_connection(cx: &mut TestAppContext) {
    let r = rig(cx, vec![mysql("api-prod", Some(EnvTag::Production), "api")]);
    let prod = profile_id(r.vcx, "api-prod");
    show(r.vcx, prod, mysql_catalog(&["api", "kampanya"]));
    r.library_with(
        "api",
        &["a.sql"],
        Some("format = 1\ndatabase = \"api\"\naliases = [\"kampanya\"]\n"),
    );
    let parent = adapter::namespace_key(prod, &hacivat_meta::NsKey::schema(["api"]));
    let row = file_row_under(r.vcx, prod, parent, "a.sql");
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, row);
    let menu = r
        .vcx
        .cx
        .read(|cx| hacivat_app::explorer::node_menu(row, cx))
        .expect("the row's menu");
    let item = menu
        .iter()
        .find(|i| i.action == "scripts.open_under")
        .expect("Open under another connection…");
    assert!(item.disabled, "only api-prod (kampanya) shows it: disabled");
    assert!(!run(r.vcx, "scripts.open_under"), "the command is disabled");
    assert!(!is_open(r.vcx));
    r.finish();
}

/// The H1 fixture: prod (`orders`) and staging (`orders_s`, bound through an alias); a prod tab
/// active; the chooser for a.sql with the user moved onto staging.
fn h1_setup(cx: &mut TestAppContext) -> (Rig, PathBuf, ProfileId, usize) {
    let r = rig(
        cx,
        vec![
            prof("orders-prod", Some(EnvTag::Production), "orders"),
            prof("orders-staging", Some(EnvTag::Staging), "orders_s"),
        ],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    let (lib, root) = r.library_with(
        "orders",
        &["a.sql", "b.sql"],
        Some("format = 1\ndatabase = \"orders\"\naliases = [\"orders_s\"]\n"),
    );
    r.open(lib, "b.sql", Some(prod), "orders");
    goto_query(r.vcx, "a.sql", "a.sql · orders");
    press_key(r.vcx, "enter");
    let rows = chooser(r.vcx).expect("the chooser");
    assert_eq!(names(&rows), ["orders-prod", "orders-staging"]);
    assert_eq!(active_name(&rows), Some("orders-prod"), "the active tab's");
    press_key(r.vcx, "down");
    assert_eq!(
        active_name(&chooser(r.vcx).unwrap()),
        Some("orders-staging")
    );
    let before = tab_count(r.vcx);
    // The alias goes away on disk (a pull, a teammate's edit); the event has not reached the
    // layer yet.
    rewrite_manifest(
        &r,
        &root,
        "format = 1\ndatabase = \"orders\"\naliases = []\n",
        0,
    );
    (r, root, prod, before)
}

/// H1 (I1, R-T15-2): Enter opens only the highlighted row, and only if it survived the re-check
/// unchanged; here staging was dropped, so nothing opens (never the focused production).
#[gpui::test]
fn enter_never_opens_a_row_the_user_did_not_highlight(cx: &mut TestAppContext) {
    let (r, _root, prod, before) = h1_setup(cx);
    in_window(r.vcx, |window, cx| {
        window.dispatch_keystroke(gpui::Keystroke::parse("enter").unwrap(), cx);
    });
    r.vcx.run_until_parked();
    assert_eq!(tab_count(r.vcx), before, "nothing opened");
    assert!(is_open(r.vcx), "the chooser stays, redrawn");
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(names(&rows), ["orders-prod"], "the rows are redrawn");
    // The active tab is still b.sql on prod; a.sql was not opened anywhere.
    let (spec, _) = active_spec(r.vcx).unwrap();
    assert_eq!(spec.rel.as_str(), "b.sql");
    assert_eq!(spec.profile, Some(prod));
    r.finish();
}

/// H1, a click: the clicked row is the one that must survive the re-check.
#[gpui::test]
fn a_click_never_opens_a_row_the_user_did_not_click(cx: &mut TestAppContext) {
    let (r, _root, _prod, before) = h1_setup(cx);
    // The layer still draws its old rows (no frame since the manifest changed): click staging,
    // dispatched at once (no executor turn lets the service's event redraw the rows first).
    let at = r
        .vcx
        .debug_bounds("scripts-choose-row-1")
        .expect("the staging row is drawn")
        .center();
    in_window(r.vcx, |window, cx| {
        use gpui::InputEvent as _;
        window.dispatch_event(
            gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: at,
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: at,
                modifiers: gpui::Modifiers::none(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
    });
    r.vcx.run_until_parked();
    assert_eq!(tab_count(r.vcx), before, "nothing opened");
    let (spec, _) = active_spec(r.vcx).unwrap();
    assert_eq!(spec.rel.as_str(), "b.sql", "a.sql opened nowhere");
    r.finish();
}

/// Review minor 3: a file listed only where the script may not run (a single-session MySQL
/// profile) says why on Enter (R-T14-3) instead of doing nothing.
#[gpui::test]
fn a_hit_only_a_single_session_profile_lists_says_why(cx: &mut TestAppContext) {
    let mut one = mysql("api-one", Some(EnvTag::Dev), "api");
    one.single_session = true;
    let r = rig(cx, vec![one]);
    r.library("api", &["@dev/d.sql"]);
    let before = tab_count(r.vcx);
    goto_query(r.vcx, "d.sql", "@dev/d.sql · api");
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the refusal", |vcx| {
        toast_texts(vcx)
            .iter()
            .any(|t| t.contains("This connection allows one session"))
    });
    assert!(!is_open(r.vcx));
    assert_eq!(tab_count(r.vcx), before);
    r.finish();
}

/// Review minor 6: a library event between a frame and Enter keeps the results' active file.
#[gpui::test]
fn a_refresh_keeps_the_active_result(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    let (_, root) = r.library_at("orders", &["b_report.sql", "c_report.sql"]);
    goto_query(r.vcx, "report", "c_report.sql · orders");
    let h = host(r.vcx);
    let rows = r.vcx.cx.read(|cx| h.read(cx).result_rows());
    let ix = rows
        .iter()
        .position(|r| r == "c_report.sql · orders")
        .unwrap();
    for _ in 0..ix {
        press_key(r.vcx, "down");
    }
    // A new file that sorts first lands.
    std::fs::write(root.join("a_report.sql"), "SELECT 1;").unwrap();
    assert!(r.fake.emit(
        &root,
        vec![hacivat_scripts::RawHint::Path(root.join("a_report.sql"))]
    ));
    wait_until(r.vcx, "the new file is listed", |vcx| {
        vcx.cx
            .read(|cx| h.read(cx).result_rows().len() == rows.len() + 1)
    });
    let now = r.vcx.cx.read(|cx| {
        let h = h.read(cx);
        h.active_result().map(|a| h.result_rows()[a].clone())
    });
    assert_eq!(now.as_deref(), Some("c_report.sql · orders"));
    r.finish();
}

// ## m4b-t15-goto fix round 2 (I-fr1-1, R-T15-2 for clicks)

/// A click carries the identity of the row it was painted for. Driven through the handler's
/// own entry point (`GotoHost::activate`) with a stale identity, so the test window's redraw
/// after every update cannot hide the race: a staging row gone since its frame, and "Open
/// without a connection" painted before a candidate appeared, open nothing, and never the
/// row the preselection rule picks now (the focused production connection).
#[gpui::test]
fn a_click_with_a_stale_row_identity_opens_nothing(cx: &mut TestAppContext) {
    let (r, _root, prod, before) = h1_setup(cx);
    let staging = profile_id(r.vcx, "orders-staging");
    let h = host(r.vcx);
    let click = |seen: Seen| {
        in_window(r.vcx, |window, cx| {
            h.update(cx, |h, cx| h.activate(seen, window, cx));
        });
        r.vcx.run_until_parked();
    };
    // The staging row, as it was painted (now unbound: its alias is gone).
    click(Seen::Row(
        staging,
        DatabaseName::new("orders_s"),
        Some(EnvTag::Staging),
    ));
    assert_eq!(tab_count(r.vcx), before, "nothing opened");
    assert!(is_open(r.vcx), "the chooser stays");
    let rows = chooser(r.vcx).unwrap();
    assert_eq!(names(&rows), ["orders-prod"], "redrawn with the live rows");
    assert_eq!(
        active_name(&rows),
        Some("orders-prod"),
        "the focused connection"
    );
    // "Open without a connection", painted before a candidate appeared.
    click(Seen::Unbound);
    assert_eq!(tab_count(r.vcx), before, "nothing opened");
    // The production row with an environment it no longer has: not the same row.
    click(Seen::Row(
        prod,
        DatabaseName::new("orders"),
        Some(EnvTag::Staging),
    ));
    assert_eq!(tab_count(r.vcx), before, "nothing opened");
    let (spec, _) = active_spec(r.vcx).unwrap();
    assert_eq!(spec.rel.as_str(), "b.sql", "a.sql opened nowhere");
    // The row as it is now opens.
    click(Seen::Row(
        prod,
        DatabaseName::new("orders"),
        Some(EnvTag::Production),
    ));
    wait_until(r.vcx, "a.sql opens under prod", |vcx| {
        active_spec(vcx)
            .is_some_and(|(s, ready)| ready && s.rel.as_str() == "a.sql" && s.profile == Some(prod))
    });
    r.finish();
}
