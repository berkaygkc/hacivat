// ## m4b-t16-attach
//! M4b Task 16: the attach dialog, manual mapping and Settings › Script libraries
//! (HV-SCRIPT-01, -02; R-M4b-7, R-M4b-8, R-M4b-21, Q5, Q6; NOTES M4b surfaces 2, 3 and 4).
//!
//! A real `LibraryService` over fixture folders (fake event sources) behind the real shell.
//! Safety (L14): every write into a library folder is compared byte for byte with what the
//! dialog showed, and the folder's other bytes are compared before and after. Texts are read
//! with `support::full_frame`/`redraw_full`, keys pressed with `support::press_key`, and every
//! wait is on a condition.

mod support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::explorer::{adapter, node_menu};
use hacivat_app::scripts::{attach_dialog, mapping, projection};
use hacivat_app::settings_ui::{self, Section};
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::{CatalogSnapshot, NsKey};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::manifest::ManifestV1;
use hacivat_scripts::service::{LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::{
    DatabaseName, GitSummary, LibraryId, LibraryState, LocalMapping, LogicalDb, PollReason,
    UnavailableReason, WatchMode,
};
use hacivat_session::profile::Profile;
use hacivat_ui::menu::MenuItemSpec;
use support::catalogs::*;
use support::*;

const WAIT: Duration = Duration::from_secs(10);

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

/// A shell with `profiles` saved and a library service over fake event sources.
fn rig(cx: &mut TestAppContext, profiles: Vec<Profile>) -> Rig {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    for p in profiles {
        add_profile(vcx, p);
    }
    let paths = paths(vcx);
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .runtime()
    });
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
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
}

/// Every file under `dir` with its bytes, sorted (the folder as a test compares it).
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                let name = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((name, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A fixture folder `name` holding `files`, canonical (macOS `/var` → `/private/var`).
fn folder(name: &str, files: &[&str]) -> PathBuf {
    let dir = temp_root().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    write_files(&dir, files);
    std::fs::canonicalize(&dir).unwrap()
}

fn prof(name: &str, env: Option<EnvTag>, database: &str) -> Profile {
    let mut p = test_profile(name, env, "10.20.30.40");
    p.database = Some(database.to_string());
    p.user = Some("hv_test_user".to_string());
    p
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

fn select_all() -> String {
    if cfg!(target_os = "macos") {
        "cmd-a".to_string()
    } else {
        "ctrl-a".to_string()
    }
}

impl Rig {
    /// Attaches `dir` as it is (no manifest written) and waits for its first scan.
    fn attach(&self, dir: &Path) -> LibraryId {
        let svc = self.svc.clone();
        let d = dir.to_path_buf();
        let id =
            block(self.vcx, async move { svc.attach(d, None, false).await }).expect("attached");
        self.up(id);
        id
    }

    fn up(&self, id: LibraryId) {
        let svc = self.svc.clone();
        wait_until(self.vcx, "the first scan and status", |_| {
            svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending
                    && matches!(
                        l.library().state,
                        LibraryState::Watching | LibraryState::Polling(_)
                    )
            })
        });
        let svc = self.svc.clone();
        block(self.vcx, async move { svc.settled().await });
    }

    fn library_at(&self, dir: &Path) -> Option<LibraryId> {
        self.svc
            .snapshot()
            .libraries
            .iter()
            .find(|l| l.root.canonical == dir)
            .map(|l| l.id)
    }

    /// Opens the attach dialog through the folder picker, answering it with `dir`.
    fn open_attach(&mut self, dir: &Path, origin: Option<attach_dialog::Origin>) {
        in_window(self.vcx, |w, cx| attach_dialog::start(origin, w, cx));
        let vcx = &*self.vcx;
        settle_until(vcx, WAIT, || vcx.cx.did_prompt_for_paths());
        let answer = dir.to_path_buf();
        self.vcx
            .cx
            .simulate_path_prompt_response(move |_| Some(vec![answer]));
        self.wait_dialog_ready();
    }

    fn wait_dialog_ready(&self) {
        wait_until(self.vcx, "the attach dialog to read the folder", |vcx| {
            vcx.cx
                .read(|cx| attach_dialog::open_dialog(cx).is_some_and(|d| d.read(cx).is_ready()))
        });
        redraw_full(self.vcx);
    }

    fn dialog(&self) -> gpui::Entity<attach_dialog::AttachDialog> {
        self.vcx
            .cx
            .read(attach_dialog::open_dialog)
            .expect("the attach dialog")
    }

    fn finish(self) {
        let Rig { vcx, svc, .. } = self;
        let taken = vcx.cx.update(projection::take_service);
        drop(taken);
        drop(svc);
        vcx.run_until_parked();
    }
}

/// The file the dialog shows it will write: its path line and its text, line by line as drawn.
fn shown_file(vcx: &VisualTestContext) -> Option<(String, String)> {
    let _ = full_frame(vcx);
    let path = text_of(vcx, "scripts-attach-file-path")?;
    let mut text = String::new();
    for i in 0.. {
        match text_of(vcx, &format!("scripts-attach-file-line-{i}")) {
            Some(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            None => break,
        }
    }
    Some((path, text))
}

fn layer_open(vcx: &VisualTestContext, id: &str) -> bool {
    layer_ids(vcx).contains(&id)
}

fn labels(items: &[MenuItemSpec], cx: &gpui::App) -> Vec<String> {
    items.iter().map(|i| (i.label)(cx).to_string()).collect()
}

// ---- the plan's tests ---------------------------------------------------------------------------

#[gpui::test]
fn attach_shows_the_exact_file_it_will_write(cx: &mut TestAppContext) {
    let mut r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let dir = folder("orders-scripts", &["daily.sql", "@production/purge.sql"]);
    let before = tree(&dir);
    r.open_attach(&dir, None);
    assert!(layer_open(r.vcx, attach_dialog::ATTACH_LAYER));
    // Prefilled with the folder name (opened from the palette, not a database).
    let d = r.dialog();
    assert_eq!(
        r.vcx
            .cx
            .read(|cx| d.read(cx).database_input().read(cx).value().to_string()),
        "orders-scripts"
    );
    // Typing the name and aliases rewrites the file shown.
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_input("orders");
    let aliases = r
        .vcx
        .cx
        .read(|cx| d.read(cx).aliases_input().focus_handle(cx));
    in_window(r.vcx, |w, cx| w.focus(&aliases, cx));
    r.vcx.simulate_input("orders_p, sipariş");
    let (path, text) = shown_file(r.vcx).expect("the file is shown");
    let expected = ManifestV1::new(
        LogicalDb::parse("orders").unwrap(),
        vec![
            LogicalDb::parse("orders_p").unwrap(),
            LogicalDb::parse("sipariş").unwrap(),
        ],
    )
    .to_toml();
    assert_eq!(text, expected, "exactly format, database and aliases");
    assert_eq!(
        path,
        dir.join(".hacivat")
            .join("workspace.toml")
            .display()
            .to_string()
    );
    // Attach, with Enter on the focused Attach button.
    let attach = handle_of(r.vcx, "button-scripts-attach-ok");
    assert_focus_on(r.vcx, "Attach", &attach);
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the library", |_| r.library_at(&dir).is_some());
    assert!(!layer_open(r.vcx, attach_dialog::ATTACH_LAYER));
    // The bytes written are the bytes shown; nothing else in the folder changed.
    let written = std::fs::read(dir.join(".hacivat/workspace.toml")).unwrap();
    assert_eq!(written, text.as_bytes());
    let mut after = tree(&dir);
    after.retain(|(n, _)| !n.starts_with(".hacivat"));
    assert_eq!(after, before);
    // Only format, database and aliases; no connection detail of any kind.
    // (`toml` is not a dependency of this crate: every line of the file is `key = value`.)
    let mut keys: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split_once(" = ").expect("a key line").0)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["aliases", "database", "format"]);
    let read = hacivat_scripts::manifest::read_bytes(text.as_bytes(), "orders-scripts");
    assert!(read.diags.is_empty(), "{:?}", read.diags);
    let written = String::from_utf8(written).unwrap();
    let pid = profile_id(r.vcx, "orders-prod").to_string();
    for leak in [
        "orders-prod",
        "10.20.30.40",
        "hv_test_user",
        "production",
        pid.as_str(),
    ] {
        assert!(!written.contains(leak), "{leak:?} reached the library");
    }
    r.finish();
}

#[gpui::test]
fn attach_over_existing_manifest_does_not_rewrite_it(cx: &mut TestAppContext) {
    let mut r = rig(cx, vec![]);
    let dir = folder("team-scripts", &["a.sql"]);
    let theirs =
        "# Our binding \u{202E}keep\nformat = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n";
    std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
    std::fs::write(dir.join(".hacivat/workspace.toml"), theirs).unwrap();
    let before = tree(&dir);
    r.open_attach(&dir, None);
    let d = r.dialog();
    assert!(r.vcx.cx.read(|cx| d.read(cx).found()));
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame
            .iter()
            .any(|t| t == "Found .hacivat/workspace.toml — binds to orders"),
        "{frame:?}"
    );
    assert!(
        frame
            .iter()
            .any(|t| t == "Hacivat reads this file and never changes it.")
    );
    // Its content is shown read-only, comments included, with the bidi control escaped.
    let (_, text) = shown_file(r.vcx).expect("the found file is shown");
    assert_eq!(text, theirs.replace('\u{202E}', "<U+202E>"));
    // No name fields: nothing to write.
    assert!(r.vcx.debug_bounds("scripts-attach-database").is_none());
    // Attach has the initial focus here.
    let attach = handle_of(r.vcx, "button-scripts-attach-ok");
    press_key(r.vcx, "enter");
    let _ = attach;
    wait_until(r.vcx, "the library", |_| r.library_at(&dir).is_some());
    assert_eq!(tree(&dir), before, "the folder is byte for byte the same");
    let id = r.library_at(&dir).unwrap();
    r.up(id);
    assert_eq!(
        r.svc
            .snapshot()
            .library(id)
            .map(|l| l.db.as_str().to_owned()),
        Some("orders".to_string())
    );
    r.finish();
}

#[gpui::test]
fn one_click_mapping_writes_only_libraries_toml(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-local", Some(EnvTag::Local), "orders_berkay")],
    );
    let dir = folder("orders", &["a.sql", "@local/seed.sql"]);
    std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
    std::fs::write(
        dir.join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    let id = r.attach(&dir);
    let before = tree(&dir);
    let p = profile_id(r.vcx, "orders-local");
    in_window(r.vcx, |w, cx| {
        mapping::open_for(p, "orders_berkay".into(), w, cx)
    });
    wait_until(r.vcx, "the mapping dialog", |vcx| {
        vcx.cx
            .read(|cx| mapping::open_dialog(cx).is_some_and(|d| d.read(cx).is_ready()))
    });
    redraw_full(r.vcx);
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame
            .iter()
            .any(|t| t == "Show a script library under orders_berkay")
    );
    assert!(frame.iter().any(|t| t == "The mapping is kept on this computer only. Your teammates don't see it."));
    // The share box is unchecked by default.
    let d = r.vcx.cx.read(mapping::open_dialog).unwrap();
    assert!(!r.vcx.cx.read(|cx| d.read(cx).shares_alias()));
    // One press: Show here has the initial focus.
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the mapping", |_| {
        !r.svc.snapshot().mappings.is_empty()
    });
    assert_eq!(
        r.svc.snapshot().mappings.as_ref(),
        [LocalMapping {
            profile: p,
            database: DatabaseName::new("orders_berkay"),
            library: id,
        }]
    );
    assert_eq!(
        tree(&dir),
        before,
        "the library folder's bytes are identical"
    );
    let registry = std::fs::read_to_string(paths(r.vcx).libraries_toml()).unwrap();
    assert!(registry.contains("orders_berkay"));
    assert!(!layer_open(r.vcx, mapping::MAP_LAYER));
    r.finish();
}

#[gpui::test]
fn share_alias_appends_through_toml_edit(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-p", Some(EnvTag::Staging), "orders_p")],
    );
    let dir = folder("orders-scripts", &["a.sql"]);
    let theirs = "# Team binding: keep this comment.\nformat = 1\ndatabase = \"orders\"\n\n# trailing note\n";
    std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
    std::fs::write(dir.join(".hacivat/workspace.toml"), theirs).unwrap();
    let id = r.attach(&dir);
    let p = profile_id(r.vcx, "orders-p");
    in_window(r.vcx, |w, cx| {
        mapping::open_for(p, "orders_p".into(), w, cx)
    });
    wait_until(r.vcx, "the mapping dialog", |vcx| {
        vcx.cx
            .read(|cx| mapping::open_dialog(cx).is_some_and(|d| d.read(cx).is_ready()))
    });
    redraw_full(r.vcx);
    assert!(
        shown_file(r.vcx).is_none(),
        "nothing to show while unchecked"
    );
    // Space on the box ticks it; the file shows the edit.
    let check = handle_of(r.vcx, "scripts-map-share");
    let _ = check;
    press_key(r.vcx, "space");
    let d = r.vcx.cx.read(mapping::open_dialog).unwrap();
    assert!(r.vcx.cx.read(|cx| d.read(cx).shares_alias()));
    let (path, shown) = shown_file(r.vcx).expect("the edited file is shown");
    assert_eq!(
        path,
        dir.join(".hacivat")
            .join("workspace.toml")
            .display()
            .to_string()
    );
    assert!(shown.starts_with("# Team binding: keep this comment.\n"));
    assert!(shown.contains("aliases = [\"orders_p\"]"), "{shown}");
    assert!(shown.contains("# trailing note"));
    let show_here = handle_of(r.vcx, "button-scripts-map-ok");
    let _ = show_here;
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the mapping", |_| {
        !r.svc.snapshot().mappings.is_empty()
    });
    let written = std::fs::read_to_string(dir.join(".hacivat/workspace.toml")).unwrap();
    assert_eq!(written, shown, "the bytes written are the bytes shown");
    // Nothing about the connection reached the file.
    for leak in ["orders-p\"", "10.20.30.40", "hv_test_user", "staging"] {
        assert!(!written.contains(leak), "{leak:?} reached the library");
    }
    r.up(id);
    r.finish();
}

#[gpui::test]
fn remove_never_touches_the_folder(cx: &mut TestAppContext) {
    let mut r = rig(cx, vec![prof("orders-local", None, "orders_berkay")]);
    let dir = folder("orders", &["a.sql", "notes.md"]);
    std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
    std::fs::write(
        dir.join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    let id = r.attach(&dir);
    let p = profile_id(r.vcx, "orders-local");
    let svc = r.svc.clone();
    block(r.vcx, async move {
        svc.map(
            LocalMapping {
                profile: p,
                database: DatabaseName::new("orders_berkay"),
                library: id,
            },
            false,
        )
        .await
    })
    .unwrap();
    let before = tree(&dir);
    assert!(run(r.vcx, "workspace.open_settings"));
    r.vcx
        .cx
        .update(|cx| settings_ui::fixtures::show_section(Section::Scripts, cx));
    redraw_full(r.vcx);
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(frame.iter().any(|t| t == "orders"), "{frame:?}");
    assert!(
        frame.iter().any(|t| t == "Watching for changes"),
        "{frame:?}"
    );
    assert!(frame.iter().any(|t| t == "Local mappings"), "{frame:?}");
    let remove = handle_of(r.vcx, "button-scripts-set-remove-0");
    let _ = remove;
    press_key(r.vcx, "enter");
    assert_eq!(dialog_title(r.vcx), "Remove orders from Hacivat?",);
    assert!(dialog_body(r.vcx).contains("and so does its 1 local mapping"));
    // Cancel is focused: a stray Enter keeps the library.
    press_key(r.vcx, "enter");
    assert!(r.library_at(&dir).is_some());
    let remove = handle_of(r.vcx, "button-scripts-set-remove-0");
    let _ = remove;
    press_key(r.vcx, "enter");
    // Remove is the first (Default) button.
    let confirm = handle_of(r.vcx, "button-dialog-button-0");
    let _ = confirm;
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "the library to go", |_| r.library_at(&dir).is_none());
    assert!(r.svc.snapshot().mappings.is_empty());
    assert_eq!(tree(&dir), before, "the folder and its files are unchanged");
    // The focus went to the page, never to a vanished button.
    redraw_full(r.vcx);
    assert!(focused(r.vcx).is_some());
    let _ = &mut r;
    r.finish();
}

#[gpui::test]
fn dialog_is_keyboard_complete_and_escape_cancels(cx: &mut TestAppContext) {
    let mut r = rig(cx, vec![]);
    let dir = folder("orders", &["a.sql"]);
    r.open_attach(&dir, None);
    let d = r.dialog();
    let (database, aliases) = r.vcx.cx.read(|cx| {
        let d = d.read(cx);
        (
            d.database_input().focus_handle(cx),
            d.aliases_input().focus_handle(cx),
        )
    });
    // The database field has the initial focus.
    assert_focus_on(r.vcx, "Database", &database);
    let change = handle_of(r.vcx, "button-scripts-attach-change");
    let cancel = handle_of(r.vcx, "button-scripts-attach-cancel");
    let attach = handle_of(r.vcx, "button-scripts-attach-ok");
    let order = tab_order(r.vcx);
    assert_consecutive(
        &order,
        &[
            ("Change…", &change),
            ("Database", &database),
            ("Aliases", &aliases),
            ("Cancel", &cancel),
            ("Attach", &attach),
        ],
    );
    // Tab is trapped in the card: from Attach it comes back to Change….
    in_window(r.vcx, |w, cx| w.focus(&attach, cx));
    r.vcx.simulate_keystrokes("tab");
    assert_focus_on(r.vcx, "Change…", &change);
    // An empty name disables Attach and says why.
    in_window(r.vcx, |w, cx| w.focus(&database, cx));
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_keystrokes("backspace");
    redraw_full(r.vcx);
    assert!(!r.vcx.cx.read(|cx| d.read(cx).can_attach(cx)));
    assert_eq!(
        text_of(r.vcx, "scripts-attach-error").as_deref(),
        Some("Type the database name.")
    );
    // Enter in the field does not attach an invalid name.
    press_key(r.vcx, "enter");
    assert!(layer_open(r.vcx, attach_dialog::ATTACH_LAYER));
    // Escape in a field cancels: nothing written, nothing registered.
    r.vcx.simulate_input("orders");
    press_key(r.vcx, "escape");
    assert!(!layer_open(r.vcx, attach_dialog::ATTACH_LAYER));
    assert!(!dir.join(".hacivat").exists());
    assert!(r.svc.snapshot().libraries.is_empty());
    // A found file: Change… → Cancel → Attach, the focus on Attach.
    std::fs::create_dir_all(dir.join(".hacivat")).unwrap();
    std::fs::write(
        dir.join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    r.open_attach(&dir, None);
    let attach = handle_of(r.vcx, "button-scripts-attach-ok");
    let change = handle_of(r.vcx, "button-scripts-attach-change");
    let cancel = handle_of(r.vcx, "button-scripts-attach-cancel");
    let order = tab_order(r.vcx);
    assert_consecutive(
        &order,
        &[
            ("Change…", &change),
            ("Cancel", &cancel),
            ("Attach", &attach),
        ],
    );
    // Escape on a button cancels too.
    in_window(r.vcx, |w, cx| w.focus(&cancel, cx));
    press_key(r.vcx, "escape");
    assert!(!layer_open(r.vcx, attach_dialog::ATTACH_LAYER));
    // The mapping dialog: Library → the box → Attach another folder… → Cancel → Show here.
    add_profile(r.vcx, prof("orders-p", None, "orders_p"));
    let id = r.attach(&dir);
    let _ = id;
    let p = profile_id(r.vcx, "orders-p");
    in_window(r.vcx, |w, cx| {
        mapping::open_for(p, "orders_p".into(), w, cx)
    });
    wait_until(r.vcx, "the mapping dialog", |vcx| {
        vcx.cx
            .read(|cx| mapping::open_dialog(cx).is_some_and(|d| d.read(cx).is_ready()))
    });
    redraw_full(r.vcx);
    let show_here = handle_of(r.vcx, "button-scripts-map-ok");
    let library = handle_of(r.vcx, "scripts-map-library");
    let share = handle_of(r.vcx, "scripts-map-share");
    let other = handle_of(r.vcx, "button-scripts-map-other");
    let cancel = handle_of(r.vcx, "button-scripts-map-cancel");
    let order = tab_order(r.vcx);
    assert_consecutive(
        &order,
        &[
            ("Library", &library),
            ("share", &share),
            ("Attach another folder…", &other),
            ("Cancel", &cancel),
            ("Show here", &show_here),
        ],
    );
    in_window(r.vcx, |w, cx| w.focus(&share, cx));
    press_key(r.vcx, "escape");
    assert!(!layer_open(r.vcx, mapping::MAP_LAYER));
    assert!(r.svc.snapshot().mappings.is_empty());
    r.finish();
}

// ---- the carries ---------------------------------------------------------------------------------

#[gpui::test]
fn attach_refuses_a_folder_inside_an_environment_folder(cx: &mut TestAppContext) {
    let mut r = rig(cx, vec![]);
    let outer = folder("team", &["@production/scripts/a.sql"]);
    let dir = outer.join("@production").join("scripts");
    r.open_attach(&dir, None);
    let d = r.dialog();
    assert!(!r.vcx.cx.read(|cx| d.read(cx).can_attach(cx)));
    let error = text_of(r.vcx, "scripts-attach-error").unwrap_or_default();
    assert!(error.contains("@production"), "{error:?}");
    assert!(shown_file(r.vcx).is_none(), "nothing would be written");
    press_key(r.vcx, "escape");
    assert!(r.svc.snapshot().libraries.is_empty());
    assert!(!dir.join(".hacivat").exists());
    r.finish();
}

#[gpui::test]
fn database_menus_offer_the_scripts_items_where_a_scripts_node_can_show(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let p = profile_id(r.vcx, "orders-prod");
    show(r.vcx, p, pg_catalog(&["orders", "billing"]));
    let menu = |key| r.vcx.cx.read(|cx| node_menu(key, cx).unwrap_or_default());
    let tail = |items: &[MenuItemSpec]| {
        r.vcx
            .cx
            .read(|cx| labels(items, cx))
            .into_iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
    };
    // The PostgreSQL database the profile connects to: both items, after a separator.
    let items = menu(adapter::database_key(p, "orders"));
    assert_eq!(
        tail(&items),
        ["Map to a script library…", "Attach script library…"]
    );
    // With no library attached, mapping says why it cannot.
    let map = items.last().unwrap();
    assert!(map.disabled);
    // Another PostgreSQL database (no Scripts node can show there) and a schema row: none.
    let items = menu(adapter::database_key(p, "billing"));
    assert!(
        r.vcx
            .cx
            .read(|cx| labels(&items, cx))
            .iter()
            .all(|l| !l.contains("script library"))
    );
    let items = menu(adapter::namespace_key(
        p,
        &NsKey::schema(["orders", "public"]),
    ));
    assert!(
        r.vcx
            .cx
            .read(|cx| labels(&items, cx))
            .iter()
            .all(|l| !l.contains("script library")),
        "PostgreSQL schema rows get no Scripts items"
    );
    // A library binding another name: mapping is offered.
    let dir = folder("billing-scripts", &["a.sql"]);
    r.attach(&dir);
    let items = menu(adapter::database_key(p, "orders"));
    assert!(!items.last().unwrap().disabled);
    r.finish();
}

#[gpui::test]
fn the_watch_choice_polls_the_library(cx: &mut TestAppContext) {
    let r = rig(cx, vec![]);
    let dir = folder("orders", &["a.sql"]);
    let id = r.attach(&dir);
    assert!(run(r.vcx, "workspace.open_settings"));
    r.vcx
        .cx
        .update(|cx| settings_ui::fixtures::show_section(Section::Scripts, cx));
    redraw_full(r.vcx);
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(frame.iter().any(|t| t == "Automatic"), "{frame:?}");
    r.vcx
        .cx
        .update(|cx| settings_ui::scripts::fixtures::choose_watch(0, WatchMode::Poll, cx));
    wait_until(r.vcx, "polling by choice", |_| {
        r.svc.snapshot().library(id).map(|l| l.state.clone())
            == Some(LibraryState::Polling(PollReason::UserChoice))
    });
    redraw_full(r.vcx);
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame.iter().any(|t| t == "Checking every 2 seconds"),
        "{frame:?}"
    );
    let registry = std::fs::read_to_string(paths(r.vcx).libraries_toml()).unwrap();
    assert!(registry.contains("watch = \"poll\""));
    // A chosen poll is not a library problem.
    let problems = r.vcx.cx.read(|cx| {
        projection::service(cx)
            .unwrap()
            .snapshot()
            .view(None)
            .library(id)
            .map(|l| hacivat_app::scripts::diagnostics::count_for_tests(&l))
    });
    assert_eq!(problems, Some(0));
    r.finish();
}

#[gpui::test]
fn use_this_folder_is_confirmed_and_keeps_the_mappings(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-local", None, "orders_berkay")]);
    let dir = folder("orders", &["a.sql"]);
    let id = r.attach(&dir);
    let p = profile_id(r.vcx, "orders-local");
    let m = LocalMapping {
        profile: p,
        database: DatabaseName::new("orders_berkay"),
        library: id,
    };
    let svc = r.svc.clone();
    let mm = m.clone();
    block(r.vcx, async move { svc.map(mm, false).await }).unwrap();
    // The folder is replaced by another one (a restore).
    std::fs::rename(&dir, dir.with_file_name("orders-old")).unwrap();
    write_files(&dir, &["restored.sql"]);
    assert!(
        r.fake
            .emit(&dir, vec![hacivat_scripts::RawHint::Path(dir.clone())])
    );
    wait_until(r.vcx, "RootChanged", |_| {
        r.svc.snapshot().library(id).map(|l| l.state.clone())
            == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    });
    assert!(run(r.vcx, "workspace.open_settings"));
    r.vcx
        .cx
        .update(|cx| settings_ui::fixtures::show_section(Section::Scripts, cx));
    redraw_full(r.vcx);
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(
        frame
            .iter()
            .any(|t| t == "This isn't the folder that was attached: it was moved or replaced."),
        "{frame:?}"
    );
    let use_it = handle_of(r.vcx, "button-scripts-set-adopt-0");
    let _ = use_it;
    press_key(r.vcx, "enter");
    assert_eq!(dialog_title(r.vcx), "Use this folder as orders?");
    // Cancel is focused: Enter keeps it refused.
    press_key(r.vcx, "enter");
    assert_eq!(
        r.svc.snapshot().library(id).map(|l| l.state.clone()),
        Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    );
    let use_it = handle_of(r.vcx, "button-scripts-set-adopt-0");
    let _ = use_it;
    press_key(r.vcx, "enter");
    let confirm = handle_of(r.vcx, "button-dialog-button-0");
    let _ = confirm;
    press_key(r.vcx, "enter");
    wait_until(r.vcx, "served again", |_| {
        r.svc.snapshot().library(id).map(|l| l.state.clone()) == Some(LibraryState::Watching)
    });
    assert_eq!(r.svc.snapshot().mappings.as_ref(), [m]);
    r.finish();
}

#[gpui::test]
fn the_menu_attach_is_prefilled_with_the_database_and_names_the_connection(
    cx: &mut TestAppContext,
) {
    let mut r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let p = profile_id(r.vcx, "orders-prod");
    let dir = folder("some-folder", &["a.sql"]);
    r.open_attach(
        &dir,
        Some(attach_dialog::Origin {
            profile: p,
            database: "orders".into(),
        }),
    );
    let d = r.dialog();
    assert_eq!(
        r.vcx
            .cx
            .read(|cx| d.read(cx).database_input().read(cx).value().to_string()),
        "orders"
    );
    let frame: Vec<String> = full_frame(r.vcx)
        .into_iter()
        .map(|t| t.text.to_string())
        .collect();
    assert!(frame.iter().any(|t| t.contains("orders-prod")), "{frame:?}");
    press_key(r.vcx, "escape");
    r.finish();
}
// ## m4b-t16-attach
