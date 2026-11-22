// ## m4b-t13-explorer
//! M4b Task 13: the Scripts node through M4's real explorer (HV-SCRIPT-05, -11; NOTES M4b
//! surfaces 1, 2 and 11). A real `LibraryService` over fixture folders (fake event sources,
//! a test trash) feeds the contributor; the explorer shows what `view(env)` returns for each
//! connection. Texts are read with `support::full_frame`, keys pressed with `support::press_key`,
//! and every wait is on a condition.
//!
//! The git CLI runs only in the `git_*` tests, on fixture repositories they create.

mod support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui::{Focusable as _, TestAppContext, VisualTestContext};
use hacivat_app::connections::explorer::{self as conn_explorer, ShownRow};
use hacivat_app::explorer::adapter;
use hacivat_app::explorer::contrib::{
    self, ContributedChildren, ContributedNode, ExplorerContributor, register_contributor,
};
use hacivat_app::explorer::node_menu;
use hacivat_app::scripts::{file_ops, projection};
use hacivat_driver::fake::FakeDriver;
use hacivat_meta::{CatalogSnapshot, NsKey};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::scan::{Fs, Meta, RealFs};
use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::{GitSummary, LibraryId, LibraryState, RawHint};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_ui::menu::MenuItemSpec;
use hacivat_ui::text::TextSource;
use hacivat_ui::tree::{ChildState, StatusKind, TreeKey, TreeRowData};
use hacivat_ui_core::gpui_surface::App;
use support::catalogs::*;
use support::*;

// ---- the rig ---------------------------------------------------------------------------------

/// Counts the scan's listings of library roots (a rescan lists its root).
struct CountingFs {
    roots: Arc<AtomicUsize>,
    watched: std::sync::Mutex<Vec<PathBuf>>,
}

impl Fs for CountingFs {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        if self
            .watched
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.as_path() == dir)
        {
            self.roots.fetch_add(1, Ordering::SeqCst);
        }
        RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<Meta> {
        RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> std::io::Result<PathBuf> {
        RealFs.read_link(link)
    }
}

struct Rig {
    vcx: &'static mut VisualTestContext,
    svc: Arc<LibraryService>,
    fake: FakeHandle,
    scans: Arc<AtomicUsize>,
    fs: Arc<CountingFs>,
    trash: PathBuf,
}

fn opts(fs: Arc<CountingFs>, trash: &Path) -> ServiceOptions {
    ServiceOptions {
        settle: Duration::from_millis(20),
        git_debounce: Duration::from_millis(40),
        recheck: Duration::from_millis(60),
        home: None,
        fs,
        trasher: Arc::new(FolderTrash(trash.to_path_buf())),
        ..ServiceOptions::default()
    }
}

/// A shell with `profiles` saved and a library service over fake event sources.
fn rig_in(vcx: &'static mut VisualTestContext, profiles: Vec<Profile>) -> Rig {
    for p in profiles {
        add_profile(vcx, p);
    }
    let paths = paths(vcx);
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .runtime()
    });
    let fake = FakeHandle::default();
    let scans = Arc::new(AtomicUsize::new(0));
    let fs = Arc::new(CountingFs {
        roots: scans.clone(),
        watched: std::sync::Mutex::new(Vec::new()),
    });
    let trash = temp_root();
    let svc = Arc::new(LibraryService::open_with(
        &paths,
        &rt,
        fake.sources(),
        opts(fs.clone(), &trash),
    ));
    let s = svc.clone();
    vcx.cx.update(|cx| projection::set_service(s, cx));
    wait_until(vcx, "the service to load", |_| {
        svc.snapshot().state == LibraryState::Watching
    });
    Rig {
        vcx,
        svc,
        fake,
        scans,
        fs,
        trash,
    }
}

fn rig(cx: &mut TestAppContext, profiles: Vec<Profile>) -> Rig {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    rig_in(vcx, profiles)
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

impl Rig {
    /// Attaches a folder named `name` holding `files` (it binds to the database `name`) and
    /// waits for its first scan and status.
    fn library(&self, name: &str, files: &[&str]) -> (LibraryId, PathBuf) {
        let dir = temp_root().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        write_files(&dir, files);
        let dir = std::fs::canonicalize(&dir).unwrap();
        self.fs.watched.lock().unwrap().push(dir.clone());
        let svc = self.svc.clone();
        let d = dir.clone();
        let id =
            block(self.vcx, async move { svc.attach(d, None, false).await }).expect("attached");
        let svc = self.svc.clone();
        wait_until(self.vcx, "the first scan and status", |_| {
            svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
            })
        });
        let svc = self.svc.clone();
        block(self.vcx, async move { svc.settled().await });
        (id, dir)
    }

    /// Hands the service the hint an OS watcher would give for `files` of the library at
    /// `root`.
    fn hint(&self, root: &Path, files: &[&str]) {
        let batch = files.iter().map(|f| RawHint::Path(root.join(f))).collect();
        assert!(self.fake.emit(root, batch), "nothing watches {root:?}");
    }

    /// Drops the service before the core runtime stops (its actor ends first).
    fn finish(self) {
        let Rig { vcx, svc, .. } = self;
        let taken = vcx.cx.update(projection::take_service);
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

// ---- reading the tree --------------------------------------------------------------------------

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{2068}' | '\u{2069}'))
        .collect()
}

fn rows(vcx: &VisualTestContext) -> Vec<ShownRow> {
    vcx.run_until_parked();
    vcx.cx.read(conn_explorer::shown_rows)
}

fn lines(vcx: &VisualTestContext) -> Vec<String> {
    rows(vcx)
        .into_iter()
        .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), strip(&r.data.label)))
        .collect()
}

fn set_expanded(vcx: &VisualTestContext, key: TreeKey, on: bool) {
    vcx.cx.update(|cx| {
        let tree = conn_explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.set_expanded(key, on, cx));
    });
    vcx.run_until_parked();
}

fn is_expanded(vcx: &VisualTestContext, key: TreeKey) -> bool {
    vcx.cx.read(|cx| {
        conn_explorer::tree(cx)
            .expect("the tree")
            .read(cx)
            .is_expanded(key)
    })
}

fn put_cursor(vcx: &VisualTestContext, key: TreeKey) {
    vcx.cx.update(|cx| {
        let tree = conn_explorer::tree(cx).expect("the tree");
        tree.update(cx, |t, cx| t.select(key, cx));
    });
    vcx.run_until_parked();
}

/// The row of `key`, when it is shown.
fn row(vcx: &VisualTestContext, key: TreeKey) -> Option<ShownRow> {
    rows(vcx)
        .into_iter()
        .find(|r| r.key == key && r.status.is_none())
}

/// The rows directly below `parent`, as shown.
fn children(vcx: &VisualTestContext, parent: TreeKey) -> Vec<ShownRow> {
    let all = rows(vcx);
    let Some(ix) = all
        .iter()
        .position(|r| r.key == parent && r.status.is_none())
    else {
        return Vec::new();
    };
    let depth = all[ix].depth;
    all[ix + 1..]
        .iter()
        .take_while(|r| r.depth > depth)
        .filter(|r| r.depth == depth + 1)
        .cloned()
        .collect()
}

/// The database row of `profile` named `db` (PostgreSQL: the database; MySQL: the schema).
fn database_row(
    vcx: &VisualTestContext,
    profile: ProfileId,
    db: &str,
    engine: DbmsKind,
) -> TreeKey {
    match engine {
        DbmsKind::MySql | DbmsKind::MariaDb => {
            adapter::namespace_key(profile, &NsKey::schema([db]))
        }
        DbmsKind::Sqlite => adapter::profile_key(profile),
        _ => {
            let _ = vcx;
            adapter::database_key(profile, db)
        }
    }
}

/// The Scripts node under `profile`'s `db`, its row expanded (and the rows above it).
fn scripts_node(
    vcx: &VisualTestContext,
    profile: ProfileId,
    db: &str,
    engine: DbmsKind,
) -> Option<ShownRow> {
    set_expanded(vcx, adapter::profile_key(profile), true);
    let parent = database_row(vcx, profile, db, engine);
    set_expanded(vcx, parent, true);
    children(vcx, parent)
        .into_iter()
        .find(|r| strip(&r.data.label) == "Scripts")
}

/// Everything below the Scripts node of `profile`'s `db`, every folder expanded, as indented
/// labels.
fn scripts_rows(vcx: &VisualTestContext, profile: ProfileId, db: &str) -> Vec<String> {
    scripts_rows_of(vcx, profile, db, DbmsKind::Postgres)
}

fn scripts_rows_of(
    vcx: &VisualTestContext,
    profile: ProfileId,
    db: &str,
    engine: DbmsKind,
) -> Vec<String> {
    let Some(node) = scripts_node(vcx, profile, db, engine) else {
        return Vec::new();
    };
    set_expanded(vcx, node.key, true);
    // Expand every folder below, until nothing new opens.
    loop {
        let below = subtree(vcx, node.key);
        let closed: Vec<TreeKey> = below
            .iter()
            .filter(|r| {
                r.data.icon == Some(hacivat_ui::icon::IconName::Folder) && !is_expanded(vcx, r.key)
            })
            .map(|r| r.key)
            .collect();
        if closed.is_empty() {
            break;
        }
        for k in closed {
            set_expanded(vcx, k, true);
        }
    }
    subtree(vcx, node.key)
        .into_iter()
        .map(|r| {
            format!(
                "{}{}",
                "  ".repeat((r.depth - node.depth - 1) as usize),
                strip(&r.data.label)
            )
        })
        .collect()
}

fn subtree(vcx: &VisualTestContext, parent: TreeKey) -> Vec<ShownRow> {
    let all = rows(vcx);
    let Some(ix) = all
        .iter()
        .position(|r| r.key == parent && r.status.is_none())
    else {
        return Vec::new();
    };
    let depth = all[ix].depth;
    all[ix + 1..]
        .iter()
        .take_while(|r| r.depth > depth)
        .cloned()
        .collect()
}

/// The row below `parent`'s subtree labelled `label`.
fn below(vcx: &VisualTestContext, parent: TreeKey, label: &str) -> ShownRow {
    subtree(vcx, parent)
        .into_iter()
        .find(|r| strip(&r.data.label) == label)
        .unwrap_or_else(|| panic!("`{label}` below {parent:?} in {:#?}", lines(vcx)))
}

// ---- placement and scoping ------------------------------------------------------------------

#[gpui::test]
fn same_library_under_prod_and_staging(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    show(r.vcx, staging, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql"]);
    for p in [prod, staging] {
        assert_eq!(scripts_rows(r.vcx, p, "orders"), ["a.sql"]);
    }
    r.finish();
}

#[gpui::test]
fn rows_under_two_connections_have_different_keys(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    show(r.vcx, staging, pg_catalog(&["orders"]));
    let (lib, _) = r.library("orders", &["reports/a.sql"]);
    scripts_rows(r.vcx, prod, "orders");
    scripts_rows(r.vcx, staging, "orders");
    let prod_node = scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).unwrap();
    let staging_node = scripts_node(r.vcx, staging, "orders", DbmsKind::Postgres).unwrap();
    assert_ne!(prod_node.key, staging_node.key);
    let (pa, sa) = (
        below(r.vcx, prod_node.key, "a.sql").key,
        below(r.vcx, staging_node.key, "a.sql").key,
    );
    assert_ne!(pa, sa, "R-M4b-26: one file, two connections, two keys");
    // Each key maps back to its own connection (the contributor's place map).
    let place = |k| r.vcx.cx.read(|cx| projection::describe_key(k, cx)).unwrap();
    assert_eq!(
        place(pa),
        (
            prod,
            "orders".into(),
            Some(lib),
            Some("reports/a.sql".into())
        )
    );
    assert_eq!(
        place(sa),
        (
            staging,
            "orders".into(),
            Some(lib),
            Some("reports/a.sql".into())
        )
    );
    // Collapsing the folder under prod leaves the one under staging open.
    let (pf, sf) = (
        below(r.vcx, prod_node.key, "reports").key,
        below(r.vcx, staging_node.key, "reports").key,
    );
    assert_ne!(pf, sf);
    set_expanded(r.vcx, pf, false);
    assert!(!is_expanded(r.vcx, pf));
    assert!(is_expanded(r.vcx, sf));
    assert!(row(r.vcx, sa).is_some() && row(r.vcx, pa).is_none());
    r.finish();
}

#[gpui::test]
fn scripts_node_sits_before_schemas(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    show(r.vcx, prod, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql"]);
    set_expanded(r.vcx, adapter::profile_key(prod), true);
    let db = adapter::database_key(prod, "orders");
    set_expanded(r.vcx, db, true);
    let labels: Vec<String> = children(r.vcx, db)
        .iter()
        .map(|c| strip(&c.data.label))
        .collect();
    assert_eq!(labels[0], "Scripts", "{labels:?}");
    assert!(labels.contains(&"public".to_string()), "{labels:?}");
    r.finish();
}

/// Gate E3: a `@production` file never appears under a staging connection.
#[gpui::test]
fn production_file_hidden_on_staging(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    show(r.vcx, staging, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql", "@production/drop_old.sql"]);
    assert_eq!(
        scripts_rows(r.vcx, prod, "orders"),
        ["@production", "  drop_old.sql", "a.sql"]
    );
    assert_eq!(scripts_rows(r.vcx, staging, "orders"), ["a.sql"]);
    // The counts are the connection's own.
    let meta = |p| {
        scripts_node(r.vcx, p, "orders", DbmsKind::Postgres)
            .and_then(|n| n.data.meta)
            .map(|m| strip(&m))
    };
    assert_eq!(meta(prod).as_deref(), Some("2"));
    assert_eq!(meta(staging).as_deref(), Some("1"));
    // And nothing the staging window draws names the production file.
    enable_registry(r.vcx);
    redraw_full(r.vcx);
    let staging_node = scripts_node(r.vcx, staging, "orders", DbmsKind::Postgres).unwrap();
    set_expanded(r.vcx, adapter::profile_key(prod), false);
    let drawn: Vec<String> = full_frame(r.vcx).iter().map(|t| strip(&t.text)).collect();
    assert!(drawn.iter().any(|t| t == "a.sql"), "{drawn:?}");
    assert!(!drawn.iter().any(|t| t.contains("drop_old")), "{drawn:?}");
    assert!(
        subtree(r.vcx, staging_node.key)
            .iter()
            .all(|row| !strip(&row.data.label).contains("drop_old"))
    );
    r.finish();
}

#[gpui::test]
fn env_change_refilters_without_rescan(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-x", Some(EnvTag::Staging), "orders")]);
    let id = profile_id(r.vcx, "orders-x");
    show(r.vcx, id, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql", "@production/drop_old.sql"]);
    assert_eq!(scripts_rows(r.vcx, id, "orders"), ["a.sql"]);
    let scans = r.scans.load(Ordering::SeqCst);
    let generation = r.svc.snapshot().generation;
    // The profile becomes production.
    let p = r.vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .get(id)
            .unwrap()
    });
    let mut edited = (*p).clone();
    edited.environment = Some(EnvTag::Production);
    save_draft(r.vcx, ProfileDraft::edit(edited));
    wait_until(r.vcx, "the production folder to show", |vcx| {
        scripts_rows(vcx, id, "orders") == ["@production", "  drop_old.sql", "a.sql"]
    });
    assert_eq!(r.scans.load(Ordering::SeqCst), scans, "no rescan");
    assert_eq!(r.svc.snapshot().generation, generation, "the same snapshot");
    r.finish();
}

#[gpui::test]
fn no_library_no_scripts_node(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    show(r.vcx, prod, pg_catalog(&["orders"]));
    // A library for another database only.
    r.library("billing", &["a.sql"]);
    assert!(scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).is_none());
    r.finish();
}

#[gpui::test]
fn no_catalog_no_scripts_node(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    r.library("orders", &["a.sql"]);
    // Never expanded or connected, no cached catalog: the tree invents no database.
    set_expanded(r.vcx, adapter::profile_key(prod), true);
    assert!(
        lines(r.vcx).iter().all(|l| !l.contains("Scripts")),
        "{:?}",
        lines(r.vcx)
    );
    // Its cached catalog lands: the node appears (R-M4b-31).
    show(r.vcx, prod, pg_catalog(&["orders"]));
    assert!(scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).is_some());
    r.finish();
}

#[gpui::test]
fn pg_greyed_database_has_no_scripts_node(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    show(r.vcx, prod, pg_catalog(&["orders", "orders_archive"]));
    r.library("orders", &["a.sql"]);
    r.library("orders_archive", &["old.sql"]);
    assert!(scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).is_some());
    // Another database on the same server: a script tab could not run there (R-M4b-25).
    assert!(scripts_node(r.vcx, prod, "orders_archive", DbmsKind::Postgres).is_none());
    r.finish();
}

#[gpui::test]
fn mysql_lists_scripts_under_every_bound_database(cx: &mut TestAppContext) {
    let mut my = prof("api-dev", Some(EnvTag::Dev), "api");
    my.engine = DbmsKind::MySql;
    let r = rig(cx, vec![my]);
    let id = profile_id(r.vcx, "api-dev");
    show(
        r.vcx,
        id,
        catalog(
            DbmsKind::MySql,
            ["api", "kampanya", "logs"]
                .iter()
                .map(|s| {
                    schema(
                        DbmsKind::MySql,
                        &[s],
                        false,
                        vec![table("t", None, &[("id", "int")])],
                    )
                })
                .collect(),
        ),
    );
    r.library("api", &["a.sql"]);
    r.library("kampanya", &["k.sql"]);
    assert_eq!(
        scripts_rows_of(r.vcx, id, "api", DbmsKind::MySql),
        ["a.sql"]
    );
    assert_eq!(
        scripts_rows_of(r.vcx, id, "kampanya", DbmsKind::MySql),
        ["k.sql"]
    );
    assert!(scripts_node(r.vcx, id, "logs", DbmsKind::MySql).is_none());
    r.finish();
}

#[gpui::test]
fn sqlite_scripts_sit_under_the_profile_root_by_file_stem(cx: &mut TestAppContext) {
    let mut lite = prof("local-file", Some(EnvTag::Local), "main");
    lite.engine = DbmsKind::Sqlite;
    lite.database = None;
    lite.endpoint = ProfileEndpoint::File {
        path: "/x/orders.db".into(),
        create_if_missing: false,
    };
    let r = rig(cx, vec![lite]);
    let id = profile_id(r.vcx, "local-file");
    show(
        r.vcx,
        id,
        catalog(
            DbmsKind::Sqlite,
            vec![
                schema(
                    DbmsKind::Sqlite,
                    &["main"],
                    false,
                    vec![table("t", None, &[])],
                ),
                // ## m4b-cifix-4: an attached schema named `archive`, not `aux`: the library
                // folder below takes its name, and `aux` is a reserved device name on Windows
                // (`create_dir_all` fails there with error 267, CI 45077623376).
                schema(
                    DbmsKind::Sqlite,
                    &["archive"],
                    false,
                    vec![table("u", None, &[])],
                ),
            ],
        ),
    );
    r.library("orders", &["a.sql"]);
    r.library("archive", &["x.sql"]);
    let root = adapter::profile_key(id);
    set_expanded(r.vcx, root, true);
    let labels: Vec<String> = children(r.vcx, root)
        .iter()
        .map(|c| strip(&c.data.label))
        .collect();
    assert_eq!(
        labels.first().map(String::as_str),
        Some("Scripts"),
        "{labels:?}"
    );
    assert_eq!(
        labels.iter().filter(|l| *l == "Scripts").count(),
        1,
        "{labels:?}"
    );
    assert_eq!(
        scripts_rows_of(r.vcx, id, "main", DbmsKind::Sqlite),
        ["a.sql"]
    );
    // The attached schema gets none (R-M4b-32).
    let aux = adapter::namespace_key(id, &NsKey::schema(["archive"]));
    set_expanded(r.vcx, aux, true);
    assert!(
        children(r.vcx, aux)
            .iter()
            .all(|c| strip(&c.data.label) != "Scripts")
    );
    r.finish();
}

#[gpui::test]
fn shared_badge_only_when_projected_twice(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql"]);
    let node = |p| {
        scripts_node(r.vcx, p, "orders", DbmsKind::Postgres)
            .unwrap()
            .data
    };
    assert_eq!(node(prod).badge, None, "only one place shows it");
    // The second connection's catalog lands: both show it.
    show(r.vcx, staging, pg_catalog(&["orders"]));
    let (a, b) = (node(prod), node(staging));
    assert_eq!(a.badge.as_deref(), Some("shared"));
    assert_eq!(b.badge.as_deref(), Some("shared"));
    assert_eq!(a.badge_glyph, None, "outlined, glyph-less (ruling b)");
    let tip = strip(a.tooltip.as_deref().unwrap_or_default());
    assert!(tip.contains("orders-staging"), "{tip}");
    // ## m4b-t13-explorer fix round 1: the tree's tooltip is one line (a newline panics the
    // text layout when it is hovered).
    assert!(!tip.contains('\n'), "{tip:?}");
    assert!(!tip.contains("orders-prod"), "{tip}");
    assert!(strip(b.tooltip.as_deref().unwrap_or_default()).contains("orders-prod"));
    r.finish();
}

#[gpui::test]
fn two_libraries_show_as_two_folders(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-prod", Some(EnvTag::Production), "orders")],
    );
    let prod = profile_id(r.vcx, "orders-prod");
    show(r.vcx, prod, pg_catalog(&["orders"]));
    let (_, dir_a) = r.library("orders", &["a.sql"]);
    // A second folder bound to the same database (its own manifest names it).
    let dir_b = temp_root().join("reports");
    std::fs::create_dir_all(dir_b.join(".hacivat")).unwrap();
    std::fs::write(
        dir_b.join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    write_files(&dir_b, &["b.sql", "c.sql"]);
    let svc = r.svc.clone();
    let d = dir_b.clone();
    let id_b = block(r.vcx, async move { svc.attach(d, None, false).await }).unwrap();
    let svc = r.svc.clone();
    wait_until(r.vcx, "the second library", |_| {
        svc.snapshot()
            .view(None)
            .library(id_b)
            .is_some_and(|l| l.file_count() == 2)
    });
    let node = scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).unwrap();
    assert_eq!(node.data.meta.as_deref().map(strip).as_deref(), Some("3"));
    set_expanded(r.vcx, node.key, true);
    let libs: Vec<(String, Option<String>)> = children(r.vcx, node.key)
        .iter()
        .map(|c| (strip(&c.data.label), c.data.meta.as_deref().map(strip)))
        .collect();
    assert_eq!(
        libs,
        [
            ("orders".to_string(), Some("1".to_string())),
            ("reports".to_string(), Some("2".to_string()))
        ]
    );
    let first = children(r.vcx, node.key)[0].clone();
    let tip = strip(first.data.tooltip.as_deref().unwrap_or_default());
    assert!(
        tip.contains(&dir_a.display().to_string()) || tip.contains("orders"),
        "{tip}"
    );
    r.finish();
}

// ---- git badges -------------------------------------------------------------------------------

fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap()
}

/// A repository at `dir` with `a.sql` and `sub/c.sql` committed, then `a.sql` edited and
/// `b.sql`, `sub/new.sql` added untracked.
fn git_fixture(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["add", "a.sql", "sub/c.sql"]);
    git(dir, &["commit", "-q", "-m", "fixture"]);
    std::fs::write(dir.join("a.sql"), b"SELECT 2;").unwrap();
    write_files(dir, &["b.sql", "sub/new.sql"]);
}

/// Waits until the library's status is ready.
fn git_ready(r: &Rig, id: LibraryId) {
    let svc = r.svc.clone();
    wait_until(r.vcx, "the repository status", |_| {
        svc.snapshot().view(None).library(id).is_some_and(|l| {
            l.git() == GitSummary::Ready
                && l.git_status(&hacivat_scripts::RelPath::parse("b.sql").unwrap())
                    .is_some()
        })
    });
}

#[gpui::test]
fn git_badges_follow_the_snapshot(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (lib, dir) = r.library("orders", &["a.sql", "sub/c.sql"]);
    git_fixture(&dir);
    r.hint(&dir, &["a.sql", "b.sql", "sub/new.sql"]);
    git_ready(&r, lib);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let status = |label: &str| below(r.vcx, node.key, label).data.status;
    let a = status("a.sql").expect("a badge");
    assert_eq!((a.kind, a.glyph.as_ref()), (StatusKind::Modified, "M"));
    assert_eq!(a.text.as_ref(), "Modified: differs from the last commit");
    let b = status("b.sql").expect("a badge");
    assert_eq!((b.kind, b.glyph.as_ref()), (StatusKind::New, "N"));
    assert_eq!(b.text.as_ref(), "New: not in the last commit");
    let sub = status("sub").expect("the folder's dot");
    assert_eq!(
        (sub.kind, sub.glyph.as_ref()),
        (StatusKind::ContainsChanges, "•")
    );
    assert_eq!(status("c.sql"), None, "clean files have no badge");
    // The tooltip and the accessible description carry the meaning.
    let a_row = below(r.vcx, node.key, "a.sql").data;
    assert!(
        strip(a_row.tooltip.as_deref().unwrap_or_default())
            .contains("M Modified: differs from the last commit")
    );
    assert_eq!(a_row.a11y_description.as_deref(), Some("modified"));
    r.finish();
}

/// Gate E5: each badge equals `git status --porcelain=v2` for its path.
#[gpui::test]
fn git_badges_match_git_status_on_fixture(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (lib, dir) = r.library("orders", &["a.sql", "sub/c.sql", "keep.sql"]);
    git_fixture(&dir);
    git(&dir, &["add", "keep.sql"]);
    r.hint(&dir, &["a.sql", "b.sql", "sub/new.sql", "keep.sql"]);
    git_ready(&r, lib);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let porcelain = git(&dir, &["status", "--porcelain=v2", "--untracked-files=all"]);
    let mut want: Vec<(String, StatusKind)> = Vec::new();
    for line in porcelain.lines() {
        let mut parts = line.split(' ');
        let kind = parts.next().unwrap_or_default();
        let path = line.rsplit(' ').next().unwrap_or_default().to_string();
        let status = match kind {
            "?" => StatusKind::New,
            "u" => StatusKind::Conflicted,
            "1" | "2" => {
                let xy = parts.next().unwrap_or_default();
                if xy.starts_with('A') {
                    StatusKind::New
                } else {
                    StatusKind::Modified
                }
            }
            _ => continue,
        };
        want.push((path, status));
    }
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(want.len() >= 4, "{porcelain}");
    let mut got: Vec<(String, StatusKind)> = subtree(r.vcx, node.key)
        .into_iter()
        .filter(|row| row.data.icon != Some(hacivat_ui::icon::IconName::Folder))
        .filter_map(|row| {
            let rel = r
                .vcx
                .cx
                .read(|cx| projection::describe_key(row.key, cx))
                .and_then(|d| d.3)?;
            Some((rel, row.data.status?.kind))
        })
        .collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got, want, "porcelain v2:\n{porcelain}");
    println!(
        "HV-FACT T13 git_badges_parity paths={} equal=true",
        want.len()
    );
    r.finish();
}

// ---- activation, menus, operations ----------------------------------------------------------

#[gpui::test]
fn enter_on_a_file_opens_it_under_that_connection(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    show(r.vcx, staging, pg_catalog(&["orders"]));
    let (lib, _) = r.library("orders", &["a.sql"]);
    scripts_rows(r.vcx, staging, "orders");
    let node = scripts_node(r.vcx, staging, "orders", DbmsKind::Postgres).unwrap();
    let file = below(r.vcx, node.key, "a.sql").key;
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, file);
    press_key(r.vcx, "enter");
    let asked = r.vcx.cx.read(projection::opens_asked);
    assert_eq!(
        asked,
        [(
            lib,
            "a.sql".to_string(),
            Some(staging),
            Some("orders".to_string())
        )]
    );
    let _ = prod;
    r.finish();
}

fn actions(items: &[MenuItemSpec]) -> Vec<String> {
    items
        .iter()
        .map(|i| {
            if i.separator {
                "—".to_string()
            } else if i.disabled {
                format!("({})", i.action)
            } else {
                i.action.to_string()
            }
        })
        .collect()
}

#[gpui::test]
fn file_and_database_menus_carry_the_scripts_items(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql", "sub/b.sql"]);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let menu_of = |key: TreeKey| -> Vec<String> {
        // The menu acts on the cursor row: the explorer moves the cursor there first.
        put_cursor(r.vcx, key);
        actions(&r.vcx.cx.read(|cx| node_menu(key, cx)).expect("a menu"))
    };
    let file = below(r.vcx, node.key, "a.sql").key;
    assert_eq!(
        menu_of(file),
        [
            "scripts.open",
            "(scripts.open_under)",
            "—",
            "scripts.rename",
            "scripts.trash",
            "—",
            "scripts.reveal",
            "scripts.copy_path",
            "scripts.copy_relative_path",
        ]
    );
    let folder = below(r.vcx, node.key, "sub").key;
    assert_eq!(
        menu_of(folder),
        [
            "scripts.new_script",
            "scripts.new_folder",
            "—",
            "scripts.rename",
            "scripts.trash",
            "—",
            "scripts.reveal",
            "scripts.copy_path",
            "scripts.copy_relative_path",
            "—",
            "scripts.refresh",
        ]
    );
    assert_eq!(
        menu_of(node.key),
        [
            "scripts.new_script",
            "scripts.new_folder",
            "—",
            "scripts.refresh",
            "scripts.reveal",
            "scripts.copy_path",
            "—",
            "(scripts.library_settings)",
            "explorer.collapse_children",
        ]
    );
    // Reveal and Move to Trash are worded for this host ($os).
    let items = r.vcx.cx.read(|cx| node_menu(file, cx)).unwrap();
    let label = |a: &str| {
        let i = items.iter().find(|i| i.action == a).unwrap();
        r.vcx.cx.read(|cx| (i.label)(cx).to_string())
    };
    let (reveal, trash) = match file_ops::host_os() {
        "windows" => ("Reveal in File Explorer", "Move to Recycle Bin"),
        "linux" => ("Open Containing Folder", "Move to Trash"),
        _ => ("Reveal in Finder", "Move to Trash"),
    };
    assert_eq!(label("scripts.reveal"), reveal);
    assert_eq!(label("scripts.trash"), trash);
    // F2 is Rename on script rows only (controller ruling c): enabled on them, never on the
    // database row.
    put_cursor(r.vcx, file);
    assert!(run(r.vcx, "scripts.rename"));
    assert!(layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER));
    press_key(r.vcx, "escape");
    assert!(!layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER));
    put_cursor(r.vcx, adapter::database_key(id, "orders"));
    assert!(
        !run(r.vcx, "scripts.rename"),
        "F2 does nothing on a database row"
    );
    // The palette's Reveal title takes the host's words too.
    let titles: Vec<String> = r.vcx.cx.read(|cx| {
        hacivat_app::commands::palette::command_entries(cx)
            .iter()
            .map(|e| e.title.to_string())
            .collect()
    });
    put_cursor(r.vcx, file);
    let titles_on_file: Vec<String> = r.vcx.cx.read(|cx| {
        hacivat_app::commands::palette::command_entries(cx)
            .iter()
            .map(|e| e.title.to_string())
            .collect()
    });
    let want = match file_ops::host_os() {
        "windows" => "Reveal script in File Explorer",
        "linux" => "Open the script's folder",
        _ => "Reveal script in Finder",
    };
    assert!(!titles.iter().any(|t| t == want));
    assert!(
        titles_on_file.iter().any(|t| t == want),
        "{titles_on_file:?}"
    );
    r.finish();
}

#[cfg(unix)]
#[gpui::test]
fn outside_link_row_is_disabled_and_does_not_open(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let outside = temp_root().join("secret.sql");
    std::fs::write(&outside, b"theirs").unwrap();
    let dir = temp_root().join("orders");
    std::fs::create_dir_all(&dir).unwrap();
    write_files(&dir, &["a.sql"]);
    std::os::unix::fs::symlink(&outside, dir.join("out.sql")).unwrap();
    let svc = r.svc.clone();
    let d = dir.clone();
    let lib = block(r.vcx, async move { svc.attach(d, None, false).await }).unwrap();
    let svc = r.svc.clone();
    wait_until(r.vcx, "the listing", |_| {
        svc.snapshot()
            .view(Some(EnvTag::Dev))
            .library(lib)
            .is_some_and(|l| l.entries().len() == 2)
    });
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let out = below(r.vcx, node.key, "out.sql");
    assert!(out.data.dim);
    assert_eq!(out.data.icon, Some(hacivat_ui::icon::IconName::Link));
    assert!(
        strip(out.data.tooltip.as_deref().unwrap_or_default())
            .contains("Points outside the library, so Hacivat never opens it.")
    );
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, out.key);
    press_key(r.vcx, "enter");
    assert!(
        r.vcx.cx.read(projection::opens_asked).is_empty(),
        "never opened"
    );
    let menu = actions(&r.vcx.cx.read(|cx| node_menu(out.key, cx)).unwrap());
    assert_eq!(menu[0], "(scripts.open)");
    assert!(!menu.contains(&"scripts.rename".to_string()));
    assert!(!run(r.vcx, "scripts.open"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"theirs");
    r.finish();
}

fn name_field(vcx: &VisualTestContext) -> Option<gpui::Entity<file_ops::NameField>> {
    in_window(vcx, |window, cx| file_ops::open_field(window, cx))
}

#[gpui::test]
fn rename_refuses_a_traversal_name(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (_, dir) = r.library("orders", &["a.sql", "b.sql"]);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let file = below(r.vcx, node.key, "a.sql").key;
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, file);
    // F2 opens the name field over the row, its name selected.
    press_key(r.vcx, "f2");
    assert!(
        layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER),
        "{:?}",
        layer_ids(r.vcx)
    );
    let field = name_field(r.vcx).expect("the name field");
    assert_eq!(r.vcx.cx.read(|cx| field.read(cx).value(cx)), "a.sql");
    for (typed, reason) in [
        ("../x.sql", "slash"),
        ("..", "dots"),
        ("b.sql", "exists"),
        ("x.txt", "ext"),
    ] {
        r.vcx.simulate_keystrokes(&select_all());
        r.vcx.simulate_input(typed);
        r.vcx.run_until_parked();
        assert_eq!(
            r.vcx.cx.read(|cx| field.read(cx).error()).map(|e| e.1),
            Some(reason),
            "{typed}"
        );
        // Enter keeps the field and changes nothing.
        press_key(r.vcx, "enter");
        assert!(layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER));
    }
    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    assert_eq!(on_disk, ["a.sql", "b.sql"]);
    // Escape cancels and gives the focus back to the tree.
    press_key(r.vcx, "escape");
    assert!(!layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER));
    let tree_focus = r
        .vcx
        .cx
        .read(|cx| conn_explorer::tree(cx).unwrap().read(cx).focus_handle(cx));
    assert_focus_on(r.vcx, "the tree", &tree_focus);
    // A valid name renames it.
    press_key(r.vcx, "f2");
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_input("müşteri.sql");
    press_key(r.vcx, "enter");
    // ## m4b-cifix-4: a rename is `hard_link(old, new)` then `remove_file(old)` (T13, ADR 0009),
    // so the new name shows on disk a moment before the old one goes; wait for both (the test
    // read the old name in between under parallel load, 9 of 24 stressed runs).
    wait_until(r.vcx, "the renamed file", |_| {
        dir.join("müşteri.sql").exists() && !dir.join("a.sql").exists()
    });
    wait_until(r.vcx, "the renamed row and the field closed", |vcx| {
        scripts_rows(vcx, id, "orders") == ["b.sql", "müşteri.sql"]
            && !layer_ids(vcx).contains(&file_ops::NAME_FIELD_LAYER)
    });
    // ## m4b-t13-explorer fix round 1 (review m12): a committed rename gives the focus back to
    // the tree, as Escape does.
    assert!(!layer_ids(r.vcx).contains(&file_ops::NAME_FIELD_LAYER));
    assert_focus_on(r.vcx, "the tree after a rename", &tree_focus);
    // A case variant of a sibling is taken (on every system, R-T13-1); its own name in another
    // case is not.
    let b = below(r.vcx, node.key, "b.sql").key;
    put_cursor(r.vcx, b);
    press_key(r.vcx, "f2");
    let field = name_field(r.vcx).expect("the name field");
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_input("MÜŞTERİ.SQL");
    r.vcx.run_until_parked();
    assert_eq!(
        r.vcx.cx.read(|cx| field.read(cx).error()).map(|e| e.1),
        None,
        "not an ASCII case variant"
    );
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_input("B.SQL");
    r.vcx.run_until_parked();
    assert_eq!(
        r.vcx.cx.read(|cx| field.read(cx).error()).map(|e| e.1),
        None,
        "its own name in another case"
    );
    press_key(r.vcx, "escape");
    // New script from the Scripts node: created, listed, and the focus back on the tree.
    put_cursor(r.vcx, node.key);
    assert!(run(r.vcx, "scripts.new_script"));
    r.vcx.simulate_keystrokes(&select_all());
    r.vcx.simulate_input("yeni.sql");
    press_key(r.vcx, "enter");
    // The file lands on the worker before the field's own answer is handled on the UI
    // thread: wait for both.
    wait_until(r.vcx, "the new script and the field closed", |vcx| {
        dir.join("yeni.sql").exists() && !layer_ids(vcx).contains(&file_ops::NAME_FIELD_LAYER)
    });
    assert_focus_on(r.vcx, "the tree after a create", &tree_focus);
    r.finish();
}

#[gpui::test]
fn trashing_the_focused_row_keeps_the_focus_in_the_explorer(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (_, dir) = r.library("orders", &["a.sql", "b.sql"]);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let (a, b) = (
        below(r.vcx, node.key, "a.sql").key,
        below(r.vcx, node.key, "b.sql").key,
    );
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, a);
    let tree_focus = r
        .vcx
        .cx
        .read(|cx| conn_explorer::tree(cx).unwrap().read(cx).focus_handle(cx));
    assert_focus_on(r.vcx, "the tree", &tree_focus);
    assert!(run(r.vcx, "scripts.trash"));
    wait_until(r.vcx, "a.sql in the test trash", |_| {
        r.trash.join("a.sql").exists()
    });
    assert!(!dir.join("a.sql").exists());
    wait_until(r.vcx, "the row to go", |vcx| row(vcx, a).is_none());
    // R-M4b-43: the focus stays in the explorer, on the next sibling.
    assert_focus_on(r.vcx, "the tree", &tree_focus);
    let cursor = r
        .vcx
        .cx
        .read(|cx| conn_explorer::tree(cx).unwrap().read(cx).cursor());
    assert_eq!(cursor, Some(b));
    assert!(
        toast_texts(r.vcx)
            .iter()
            .any(|t| t.contains("a.sql") && (t.contains("Trash") || t.contains("Recycle Bin"))),
        "{:?}",
        toast_texts(r.vcx)
    );
    r.finish();
}

// ---- refresh, persistence, filter and accessibility -------------------------------------------

#[gpui::test]
fn external_change_shows_without_another_event(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (_, dir) = r.library("orders", &["a.sql"]);
    assert_eq!(scripts_rows(r.vcx, id, "orders"), ["a.sql"]);
    enable_registry(r.vcx);
    redraw_full(r.vcx);
    // Another program creates b.sql; the watcher reports it; nothing else happens.
    write_files(&dir, &["b.sql"]);
    r.hint(&dir, &["b.sql"]);
    wait_until(r.vcx, "b.sql drawn", |vcx| {
        full_frame(vcx).iter().any(|t| strip(&t.text) == "b.sql")
    });
    r.finish();
}

#[gpui::test]
fn expanded_scripts_survive_refresh(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (_, dir) = r.library("orders", &["sub/a.sql", "z.sql"]);
    assert_eq!(
        scripts_rows(r.vcx, id, "orders"),
        ["sub", "  a.sql", "z.sql"]
    );
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let sub = below(r.vcx, node.key, "sub").key;
    let z = below(r.vcx, node.key, "z.sql").key;
    put_cursor(r.vcx, z);
    // A change inside the expanded folder: the contributor's generation moves, the explorer
    // re-asks its loaded children at once, and the folder stays open with the new file.
    write_files(&dir, &["sub/b.sql"]);
    r.hint(&dir, &["sub/b.sql"]);
    wait_until(r.vcx, "b.sql listed", |vcx| {
        subtree(vcx, node.key)
            .iter()
            .any(|row| strip(&row.data.label) == "b.sql")
    });
    assert!(is_expanded(r.vcx, node.key));
    assert!(is_expanded(r.vcx, sub));
    let labels: Vec<String> = subtree(r.vcx, node.key)
        .iter()
        .map(|row| strip(&row.data.label))
        .collect();
    assert_eq!(labels, ["sub", "a.sql", "b.sql", "z.sql"]);
    // The cursor stayed on its row (it never passed through "not loaded").
    let cursor = r
        .vcx
        .cx
        .read(|cx| conn_explorer::tree(cx).unwrap().read(cx).cursor());
    assert_eq!(cursor, Some(z));
    // The menu's Refresh lists a change no watcher reported.
    write_files(&dir, &["y.sql"]);
    put_cursor(r.vcx, node.key);
    assert!(run(r.vcx, "scripts.refresh"));
    wait_until(r.vcx, "y.sql listed", |vcx| {
        subtree(vcx, node.key)
            .iter()
            .any(|row| strip(&row.data.label) == "y.sql")
    });
    assert!(is_expanded(r.vcx, sub));
    r.finish();
}

/// C-M4-8 / R-M4b-45: after a restart the Scripts node is expanded again; its folders, whose
/// children were not loaded when the one-shot restore walked the tree, start collapsed.
#[gpui::test]
fn scripts_node_expansion_survives_restart_nested_folders_start_collapsed(cx: &mut TestAppContext) {
    use support::live::*;
    let profile = prof("orders-dev", Some(EnvTag::Dev), "orders");
    let id = profile.id;
    // The library, attached in an earlier session.
    let lib_dir = std::fs::canonicalize({
        let d = temp_root().join("orders");
        std::fs::create_dir_all(&d).unwrap();
        write_files(&d, &["sub/a.sql", "z.sql"]);
        d
    })
    .unwrap();
    // The ids the saved expansion names are only known once the library is attached, so the
    // shell starts first, the live host is held back (as `explorer_live` does), and the saved
    // expansion is given to it before it starts.
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let src = Arc::new(FakeSource::new(
        vcx.cx.read(|cx| {
            cx.global::<hacivat_app::connections::ConnectionsModel>()
                .runtime()
        }),
        hacivat_introspect::testing::fake::FakeIntrospector::new(DbmsKind::Postgres),
        temp_dir(vcx).join("fake-cache"),
    ));
    let as_source: Arc<dyn hacivat_app::explorer::live::CatalogSource> = src.clone();
    vcx.cx
        .update(|cx| hacivat_app::explorer::live::set_source(as_source, cx));
    let r = rig_in(vcx, vec![profile]);
    r.fs.watched.lock().unwrap().push(lib_dir.clone());
    let svc = r.svc.clone();
    let d = lib_dir.clone();
    let lib = block(r.vcx, async move { svc.attach(d, None, false).await }).unwrap();
    let svc = r.svc.clone();
    wait_until(r.vcx, "the listing", |_| {
        svc.snapshot()
            .view(None)
            .library(lib)
            .is_some_and(|l| l.file_count() == 2)
    });
    write_cache(
        &src.cache_path(id),
        None,
        &[cached_schema(
            DbmsKind::Postgres,
            &["orders", "public"],
            &["t"],
        )],
    );
    // What the last session saved: the root, the database, the Scripts node and its folder.
    let scripts = projection::key_for(id, "orders", None, None);
    let sub = projection::key_for(id, "orders", Some(lib), Some(("sub", true)));
    let saved = vec![
        adapter::profile_key(id),
        adapter::database_key(id, "orders"),
        scripts,
        sub,
    ];
    r.vcx.cx.update(|cx| {
        hacivat_app::explorer::live::set_restored(
            std::collections::BTreeMap::from([(id, saved)]),
            cx,
        );
        hacivat_app::explorer::live::start(cx);
    });
    wait_until(r.vcx, "the restored Scripts node", |vcx| {
        row(vcx, scripts).is_some() && is_expanded(vcx, scripts)
    });
    wait_until(r.vcx, "its folder row", |vcx| row(vcx, sub).is_some());
    assert!(!is_expanded(r.vcx, sub), "nested folders start collapsed");
    r.finish();
}

#[gpui::test]
fn explorer_filter_and_a11y_names(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (lib, dir) = r.library("orders", &["a.sql", "sub/c.sql", "orders_report.sql"]);
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "fixture"]);
    std::fs::write(dir.join("orders_report.sql"), b"SELECT 3;").unwrap();
    write_files(&dir, &["b.sql"]);
    r.hint(&dir, &["orders_report.sql", "b.sql"]);
    git_ready(&r, lib);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    // The Scripts node: its count in words.
    assert_eq!(
        node.data.a11y_description.as_deref().map(strip).as_deref(),
        Some("4 scripts")
    );
    let report = below(r.vcx, node.key, "orders_report.sql");
    assert_eq!(
        format!(
            "{}, {}",
            strip(&report.data.label),
            report.data.a11y_description.as_deref().unwrap_or_default()
        ),
        "orders_report.sql, modified"
    );
    // The filter finds a loaded file by its name.
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_filter);
    r.vcx.simulate_input("report");
    wait_until(r.vcx, "the filter's match", |vcx| {
        lines(vcx).iter().any(|l| l.trim() == "orders_report.sql")
    });
    assert!(
        !lines(r.vcx).iter().any(|l| l.trim() == "a.sql"),
        "{:?}",
        lines(r.vcx)
    );
    r.finish();
}

#[gpui::test]
fn diagnostics_row_opens_the_popover(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-staging", Some(EnvTag::Staging), "orders")],
    );
    let id = profile_id(r.vcx, "orders-staging");
    show(r.vcx, id, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql", "@prod/x.sql", "@production/p.sql"]);
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let kids = children(r.vcx, node.key);
    assert_eq!(
        strip(&kids[0].data.label),
        "1 problem",
        "the library's first child"
    );
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, kids[0].key);
    press_key(r.vcx, "enter");
    assert!(
        layer_ids(r.vcx).contains(&hacivat_app::scripts::diagnostics::DIAGNOSTICS_LAYER),
        "{:?}",
        layer_ids(r.vcx)
    );
    enable_registry(r.vcx);
    redraw_full(r.vcx);
    let drawn: Vec<String> = full_frame(r.vcx).iter().map(|t| strip(&t.text)).collect();
    assert!(
        drawn.iter().any(|t| t == "1 problem in orders"),
        "{drawn:?}"
    );
    assert!(
        drawn
            .iter()
            .any(|t| t.contains("@prod is not an environment tag")
                && t.contains("did you mean @production?")),
        "{drawn:?}"
    );
    // Nothing names the production file on a staging connection.
    assert!(!drawn.iter().any(|t| t.contains("p.sql")), "{drawn:?}");
    // Escape closes it and gives the focus back to the tree.
    press_key(r.vcx, "escape");
    assert!(!layer_ids(r.vcx).contains(&hacivat_app::scripts::diagnostics::DIAGNOSTICS_LAYER));
    let tree_focus = r
        .vcx
        .cx
        .read(|cx| conn_explorer::tree(cx).unwrap().read(cx).focus_handle(cx));
    assert_focus_on(r.vcx, "the tree", &tree_focus);
    r.finish();
}

// ---- the contributor seam (T1b review m3, m4) -------------------------------------------------

/// A contributor that lists `top` under database `app` with the lazy children `kids`, each a
/// leaf labelled after its owner.
struct Fake {
    name: &'static str,
    top: TreeKey,
    kids: Vec<TreeKey>,
    activated: RefCell<Vec<TreeKey>>,
}

impl ExplorerContributor for Fake {
    fn database_children(&self, _: ProfileId, database: &str, _: &App) -> Vec<ContributedNode> {
        if database != "app" {
            return Vec::new();
        }
        let kids: Arc<[TreeKey]> = Arc::from(self.kids.clone());
        vec![ContributedNode {
            key: self.top,
            row: TreeRowData::new(self.name, TextSource::Data),
            children: ContributedChildren::Lazy(Arc::new(move |_| {
                ChildState::Loaded(kids.clone())
            })),
            filter_text: None,
        }]
    }
    fn children(&self, _: TreeKey, _: &App) -> ChildState {
        ChildState::Leaf
    }
    fn child_node(&self, key: TreeKey, _: &App) -> Option<ContributedNode> {
        Some(ContributedNode {
            key,
            row: TreeRowData::new(format!("{}-{}", self.name, key.0), TextSource::Data),
            children: ContributedChildren::Leaf,
            filter_text: None,
        })
    }
    fn activate(&self, key: TreeKey, _: &mut gpui::Window, _: &mut App) {
        self.activated.borrow_mut().push(key);
    }
    fn context_menu(&self, _: TreeKey, _: &App) -> Vec<MenuItemSpec> {
        Vec::new()
    }
    fn generation(&self) -> u64 {
        1
    }
}

#[gpui::test]
fn a_child_key_keeps_its_first_owner(cx: &mut TestAppContext) {
    let vcx = shell_with_fake(cx, "en", answers_probe(FakeDriver::new(DbmsKind::Postgres)));
    let mut p = test_profile("shop", Some(EnvTag::Dev), "127.0.0.1");
    p.database = Some("app".to_string());
    let id = add_profile(vcx, p);
    show(vcx, id, pg_catalog(&["app"]));
    let shared = TreeKey(9_001);
    let a = Rc::new(Fake {
        name: "A",
        top: TreeKey(9_000),
        kids: vec![shared],
        activated: RefCell::default(),
    });
    // B lists A's child as its own.
    let b = Rc::new(Fake {
        name: "B",
        top: TreeKey(9_100),
        kids: vec![shared],
        activated: RefCell::default(),
    });
    vcx.cx.update(|cx| {
        register_contributor(a.clone(), cx);
        register_contributor(b.clone(), cx);
        contrib::contributors_changed(cx);
    });
    set_expanded(vcx, adapter::profile_key(id), true);
    set_expanded(vcx, adapter::database_key(id, "app"), true);
    set_expanded(vcx, a.top, true);
    set_expanded(vcx, b.top, true);
    in_window(vcx, |window, _| window.activate_window());
    in_window(vcx, conn_explorer::focus_list);
    put_cursor(vcx, shared);
    press_key(vcx, "enter");
    assert_eq!(
        *a.activated.borrow(),
        [shared],
        "the first owner keeps its child"
    );
    assert!(b.activated.borrow().is_empty());
}
// ## m4b-t13-explorer

// ## m4b-t13-explorer fix round 1

/// The platform's select-all chord (review I2: never a macOS literal).
fn select_all() -> String {
    if cfg!(target_os = "macos") {
        "cmd-a".to_string()
    } else {
        "ctrl-a".to_string()
    }
}

/// Review H4: a profile retagged from production to staging while the cursor is on a
/// `@production` file: the rows go, nothing drawn names the file, its commands are off, and
/// nothing opens or moves it.
#[gpui::test]
fn a_retag_under_the_cursor_hides_and_disarms_the_production_row(cx: &mut TestAppContext) {
    let r = rig(
        cx,
        vec![prof("orders-x", Some(EnvTag::Production), "orders")],
    );
    let id = profile_id(r.vcx, "orders-x");
    show(r.vcx, id, pg_catalog(&["orders"]));
    let (_, dir) = r.library("orders", &["a.sql", "@production/drop_old.sql"]);
    assert_eq!(
        scripts_rows(r.vcx, id, "orders"),
        ["@production", "  drop_old.sql", "a.sql"]
    );
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let hidden = below(r.vcx, node.key, "drop_old.sql").key;
    in_window(r.vcx, |window, _| window.activate_window());
    in_window(r.vcx, conn_explorer::focus_list);
    put_cursor(r.vcx, hidden);
    let p = r.vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .get(id)
            .unwrap()
    });
    let mut edited = (*p).clone();
    edited.environment = Some(EnvTag::Staging);
    save_draft(r.vcx, ProfileDraft::edit(edited));
    wait_until(r.vcx, "the production rows to go", |vcx| {
        scripts_rows(vcx, id, "orders") == ["a.sql"]
    });
    assert!(
        r.vcx
            .cx
            .read(|cx| projection::describe_key(hidden, cx))
            .is_none()
    );
    put_cursor(r.vcx, hidden);
    for cmd in [
        "scripts.open",
        "scripts.rename",
        "scripts.trash",
        "scripts.copy_relative_path",
    ] {
        assert!(!run(r.vcx, cmd), "{cmd} is off");
    }
    assert!(r.vcx.cx.read(projection::opens_asked).is_empty());
    enable_registry(r.vcx);
    redraw_full(r.vcx);
    let drawn: Vec<String> = full_frame(r.vcx).iter().map(|t| strip(&t.text)).collect();
    assert!(
        !drawn
            .iter()
            .any(|t| t.contains("drop_old") || t.contains("@production")),
        "{drawn:?}"
    );
    for row in rows(r.vcx) {
        let all = format!(
            "{} {:?} {:?} {:?}",
            row.data.label, row.data.tooltip, row.data.a11y_description, row.data.meta
        );
        assert!(
            !all.contains("drop_old") && !all.contains("@production"),
            "{all}"
        );
    }
    assert!(dir.join("@production/drop_old.sql").exists());
    r.finish();
}

/// Review m8 (R-T1a-2 carry): a modified file whose name is NFD on disk shows its badge.
#[gpui::test]
fn an_nfd_named_modified_file_shows_its_badge(cx: &mut TestAppContext) {
    let r = rig(cx, vec![prof("orders-dev", Some(EnvTag::Dev), "orders")]);
    let id = profile_id(r.vcx, "orders-dev");
    show(r.vcx, id, pg_catalog(&["orders"]));
    // "müşteri.sql" decomposed: u + U+0308, s + U+0327.
    let nfd = "mu\u{0308}s\u{0327}teri.sql";
    let (lib, dir) = r.library("orders", &["a.sql"]);
    std::fs::write(dir.join(nfd), b"SELECT 1;").unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "fixture"]);
    std::fs::write(dir.join(nfd), b"SELECT 2;").unwrap();
    write_files(&dir, &["b.sql"]);
    r.hint(&dir, &[nfd, "b.sql"]);
    git_ready(&r, lib);
    let svc = r.svc.clone();
    wait_until(r.vcx, "the NFD file's status", |_| {
        let snap = svc.snapshot();
        let view = snap.view(None);
        view.library(lib).is_some_and(|l| {
            l.entries().iter().any(|e| {
                e.rel.as_str().ends_with("teri.sql")
                    && l.git_status(&e.rel) == Some(hacivat_git::FileStatus::Modified)
            })
        })
    });
    scripts_rows(r.vcx, id, "orders");
    let node = scripts_node(r.vcx, id, "orders", DbmsKind::Postgres).unwrap();
    let row = subtree(r.vcx, node.key)
        .into_iter()
        .find(|row| row.data.label.contains("teri.sql"))
        .expect("the NFD row");
    let st = row.data.status.expect("its badge");
    assert_eq!((st.kind, st.glyph.as_ref()), (StatusKind::Modified, "M"));
    r.finish();
}

/// Review m11 and m10: a shared library's file rows say "shared" to assistive technology, and
/// "Open under another connection…" says when no other connection shows the file.
#[gpui::test]
fn shared_file_rows_and_the_open_under_hint(cx: &mut TestAppContext) {
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
    show(r.vcx, prod, pg_catalog(&["orders"]));
    r.library("orders", &["a.sql", "@production/p.sql"]);
    scripts_rows(r.vcx, prod, "orders");
    let node = scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).unwrap();
    let hint = |key: TreeKey| -> bool {
        put_cursor(r.vcx, key);
        r.vcx
            .cx
            .read(|cx| node_menu(key, cx))
            .unwrap()
            .iter()
            .find(|i| i.action == "scripts.open_under")
            .is_some_and(|i| i.hint.is_some())
    };
    let a = below(r.vcx, node.key, "a.sql");
    assert_eq!(
        a.data.a11y_description, None,
        "the tree lists one place only"
    );
    // The chooser's candidates include staging's configured database even before its catalog
    // loads (R-M4b-31): a.sql has another connection, the @production file none.
    assert!(!hint(a.key), "staging's configured database shows it");
    let p = below(r.vcx, node.key, "p.sql");
    assert!(hint(p.key), "no other connection shows a @production file");
    // Staging's catalog lands: a.sql is shared; p.sql still shows nowhere else.
    show(r.vcx, staging, pg_catalog(&["orders"]));
    scripts_rows(r.vcx, prod, "orders");
    let node = scripts_node(r.vcx, prod, "orders", DbmsKind::Postgres).unwrap();
    let a = below(r.vcx, node.key, "a.sql");
    assert_eq!(
        a.data.a11y_description.as_deref().map(strip).as_deref(),
        Some("shared")
    );
    assert!(!hint(a.key), "staging shows it");
    let p = below(r.vcx, node.key, "p.sql");
    assert!(hint(p.key), "staging never shows a @production file");
    r.finish();
}
// ## m4b-t13-explorer
