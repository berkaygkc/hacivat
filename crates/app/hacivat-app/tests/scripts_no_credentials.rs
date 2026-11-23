//! M4b Task 19, gate E6 (spec §6 M4b exit 6, L14): no credentials are ever written into a
//! library folder. A whole scripted session runs in the app, with the keyring holding the
//! connections' secrets (the shell's memory backend) and one `FakeDriver` per connection
//! (`support::shell_with_routes`, routed by host):
//!
//! attach (the attach dialog's exact `workspace.toml`, `attach_writing`), map another database
//! with a shared alias, open a script under orders-prod and under orders-staging, run it on
//! both, edit (a recovery copy after five quiet seconds), save, new folder, new script, rename,
//! trash, a git refresh, the layout saved, quit, a relaunch that restores the tabs, quit.
//!
//! Then every name and every byte under the library folder (`.hacivat/` included) is searched
//! for the connections' host, port, user, password, TLS key passphrase, names, profile ids (in
//! each spelling) and keyring service names; `.git/` is byte-identical to what the git CLI
//! wrote (Hacivat never writes a repository); the only paths are the user's scripts and
//! `.hacivat/workspace.toml`, whose keys are `format`, `database` and `aliases`. The recovery
//! copy and the app's state files live under the app's own folders, private.
//!
//! The same audit runs on the core over real servers (`hacivat-scripts` `docker_m4b`).

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};
use hacivat_app::console::Consoles;
use hacivat_app::scripts::open::{install_service, open_script};
use hacivat_app::scripts::tabs::{self, Load, ScriptDoc};
use hacivat_app::scripts::{DatabaseName, LibraryId, RelPath};
use hacivat_app::shell::TabKind;
use hacivat_driver::ExecSummary;
use hacivat_driver::fake::{FakeDriver, FakeExec, FakeQuery, FakeResultSet, FakeRows, FakeTypedCall};
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_scripts::manifest::ManifestV1;
use hacivat_scripts::service::{FolderTrash, LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeEventSource;
use hacivat_scripts::{EntryKind, LocalMapping, LogicalDb};
use hacivat_secrets::{Retention, SecretKey, SecretSlot, SecretString};
use hacivat_session::profile::{Profile, ProfileDraft, ProfileEndpoint};
use hacivat_ui::panes::TabId;
use hacivat_ui_editor::EditorView;
use support::*;

const PROD: &str = "10.0.0.1";
const STAGING: &str = "10.0.0.2";
const PORT: u16 = 6543;
const USER: &str = "hv_user";
const PASSWORD: &str = "hv_test_pw";
const PASSPHRASE: &str = "s3cret";

// ---- fakes (as `scripts_tabs.rs`) ------------------------------------------------------------

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

/// A PostgreSQL fake on database `orders`: the SQL-context probe, the read-only guard's canary
/// and the schema probe, answered many times (every session and re-probe, two launches).
fn pg_fake() -> FakeDriver {
    let exec = (0..96)
        .flat_map(|_| {
            [
                answer("SHOW standard_conforming_strings", &[Some("on")]),
                answer("SELECT pg_catalog.txid_current_if_assigned()", &[None]),
                answer(
                    "SELECT current_database()",
                    &[
                        Some("orders"),
                        Some("public"),
                        Some("public"),
                        Some(USER),
                        Some("public"),
                    ],
                ),
            ]
        })
        .collect();
    let scs = (0..96)
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

fn routes() -> Vec<Route> {
    vec![
        (DbmsKind::Postgres, PROD, pg_fake()),
        (DbmsKind::Postgres, STAGING, pg_fake()),
    ]
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

/// Profile `name` at `host`: the plan's port and user, signing in with a password the keyring
/// keeps (`Retention::Forever`), TLS off (the fakes speak none).
fn profile(name: &str, env: EnvTag, host: &str) -> Profile {
    let mut p = test_profile(name, Some(env), host);
    p.endpoint = ProfileEndpoint::Server {
        host: host.to_string(),
        port: Some(PORT),
    };
    p.user = Some(USER.to_string());
    p.database = Some("orders".to_string());
    p.auth = hacivat_secrets::AuthProviderId::Password;
    p.retention = Retention::Forever;
    p.tls.mode = hacivat_net::TlsMode::Disable;
    p
}

/// Saves `p` with its password and a TLS key passphrase, both into the keyring.
fn add_with_secrets(vcx: &VisualTestContext, p: Profile) -> ProfileId {
    save_draft(
        vcx,
        ProfileDraft {
            is_new: true,
            password: Some(SecretString::from(PASSWORD.to_string())),
            tls_key_passphrase: Some(SecretString::from(PASSPHRASE.to_string())),
            ..ProfileDraft::edit(p)
        },
    )
}

// ---- the library -----------------------------------------------------------------------------

fn opts(trash: &Path) -> ServiceOptions {
    ServiceOptions {
        settle: Duration::from_millis(20),
        git_debounce: Duration::from_millis(40),
        recheck: Duration::from_millis(60),
        home: None,
        trasher: Arc::new(FolderTrash(trash.to_path_buf())),
        ..ServiceOptions::default()
    }
}

/// `git` on the fixture repository `dir` (a repository this test made), with fixed identities
/// and dates.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=hvm4b", "-c", "user.email=hvm4b@example.invalid"])
        .args(["-c", "init.defaultBranch=main", "-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// Every path under `root` (relative, `/`-separated) with its bytes (`None` for a folder).
fn walk(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut todo = vec![root.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("a readable folder").flatten() {
            let p = e.path();
            let name = p
                .strip_prefix(root)
                .expect("under the root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if std::fs::symlink_metadata(&p).expect("metadata").is_dir() {
                out.insert(name, None);
                todo.push(p);
            } else {
                out.insert(name, Some(std::fs::read(&p).expect("its bytes")));
            }
        }
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Every spelling of each profile's identifiers and every secret of the session.
fn needles(profiles: &[ProfileId], names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = [
        PASSWORD, PASSPHRASE, USER, PROD, STAGING, "password =", "host =",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    out.push(PORT.to_string());
    out.extend(names.iter().map(|s| s.to_string()));
    for id in profiles {
        let u = id.0;
        out.extend([
            u.to_string(),
            u.simple().to_string(),
            u.to_string().to_ascii_uppercase(),
            format!("{id:?}"),
        ]);
        for slot in [SecretSlot::DbPassword, SecretSlot::TlsKeyPassphrase] {
            out.push(SecretKey { profile: *id, slot }.service());
        }
    }
    out
}

// ---- the session ------------------------------------------------------------------------------

fn doc(vcx: &VisualTestContext, tab: TabId) -> Option<Entity<ScriptDoc>> {
    vcx.cx.read(|cx| tabs::doc_of_tab(tab, cx))
}

fn active(vcx: &VisualTestContext) -> TabId {
    vcx.run_until_parked();
    let ws = workspace(vcx);
    vcx.cx
        .read(|cx| ws.read(cx).active_tab())
        .expect("a tab is active")
}

/// Opens `rel` under `profile` (database `orders`) and waits until its tab shows the file.
fn open(vcx: &VisualTestContext, lib: LibraryId, rel: &str, profile: ProfileId) -> TabId {
    let rel_path = RelPath::parse(rel).expect("a path");
    vcx.cx.update(|cx| {
        open_script(
            cx,
            lib,
            rel_path,
            Some(profile),
            Some(DatabaseName::new("orders")),
        );
    });
    let want = rel.to_string();
    wait_until(vcx, "the script tab shows its file", |vcx| {
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
    active(vcx)
}

fn editor(vcx: &VisualTestContext, tab: TabId) -> Entity<EditorView> {
    let console = vcx
        .cx
        .read(|cx| Consoles::by_tab(cx, tab))
        .expect("the tab's console");
    vcx.cx
        .read(|cx| Consoles::get(cx, console).and_then(|s| s.editor.clone()))
        .expect("an editor")
}

fn focus(vcx: &VisualTestContext, tab: TabId) {
    let ws = workspace(vcx);
    let ed = editor(vcx, tab);
    in_window(vcx, |window, cx| {
        ws.update(cx, |w, cx| w.focus_tab(tab, cx));
        let f = gpui::Focusable::focus_handle(ed.read(cx), cx);
        window.focus(&f, cx);
    });
    vcx.run_until_parked();
}

fn type_text(vcx: &VisualTestContext, tab: TabId, s: &str) {
    focus(vcx, tab);
    let ed = editor(vcx, tab);
    let s = s.to_string();
    in_window(vcx, |window, cx| {
        ed.update(cx, |e, cx| {
            let len = e.snapshot().buffer.len_bytes();
            e.select_range(len..len, cx);
            e.replace_text_in_range(None, &s, window, cx);
        });
    });
    vcx.run_until_parked();
}

fn run_tab(vcx: &VisualTestContext, tab: TabId) {
    focus(vcx, tab);
    let ed = editor(vcx, tab);
    in_window(vcx, |window, cx| {
        ed.update(cx, |e, cx| e.run_command("editor.run", window, cx));
    });
    vcx.run_until_parked();
}

fn dirty(vcx: &VisualTestContext, tab: TabId) -> bool {
    let d = doc(vcx, tab).expect("a script doc");
    vcx.cx.read(|cx| d.read(cx).is_dirty(cx))
}

fn connected(vcx: &VisualTestContext, name: &str) {
    connect(vcx, name);
    wait_until(vcx, "connected", |vcx| {
        matches!(live(vcx, name), hacivat_session::LiveState::Connected(_))
    });
}

fn block<T>(f: impl std::future::Future<Output = T>) -> T {
    futures::executor::block_on(f)
}

/// The recovery copies (`.sql`) of library `lib` under the app's folders.
fn copies(vcx: &VisualTestContext, lib: LibraryId) -> Vec<PathBuf> {
    let dir = paths(vcx)
        .script_recovery_dir(&lib.to_string())
        .expect("a recovery folder");
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "sql"))
                .collect()
        })
        .unwrap_or_default()
}

#[gpui::test]
fn no_credentials_are_ever_written_into_a_library_folder(cx: &mut TestAppContext) {
    let (vcx, fakes) = shell_with_routes(cx, "en", routes());
    in_window(vcx, |window, _| window.activate_window());
    let app_paths = paths(vcx);
    let app_root = app_paths.config.parent().expect("the app root").to_path_buf();
    let base = temp_root();
    let (root, trash) = (base.join("orders-scripts"), base.join("trash"));
    std::fs::create_dir_all(&root).expect("the library folder");
    std::fs::create_dir_all(&trash).expect("the trash folder");
    std::fs::write(root.join("report.sql"), "SELECT 1").expect("a script");
    let root = std::fs::canonicalize(&root).expect("canonical");
    // A repository the user made: Hacivat only ever reads it.
    git(&root, &["init", "-q", "--template="]);
    git(&root, &["add", "report.sql"]);
    git(&root, &["commit", "-q", "-m", "init"]);
    let git_before = walk(&root.join(".git"));

    let prod = add_with_secrets(vcx, profile("orders-prod", EnvTag::Production, PROD));
    let staging = add_with_secrets(vcx, profile("orders-staging", EnvTag::Staging, STAGING));
    // The secrets are in the keyring, so the session really holds them.
    let backend = secret_backend(vcx);
    for id in [prod, staging] {
        for slot in [SecretSlot::DbPassword, SecretSlot::TlsKeyPassphrase] {
            let key = SecretKey { profile: id, slot };
            wait_until(vcx, "the secret is in the keyring", |_| {
                backend.get(&key).ok().flatten().is_some()
            });
        }
    }

    // The library service on the app's core runtime.
    let rt = vcx.cx.read(|cx| {
        cx.global::<hacivat_app::connections::ConnectionsModel>()
            .core_runtime()
            .handle()
    });
    let (_, watch) = FakeEventSource::new();
    let service = Arc::new(LibraryService::open_with(
        &app_paths,
        &rt,
        watch.sources(),
        opts(&trash),
    ));
    {
        let service = service.clone();
        vcx.cx.update(|cx| install_service(service, cx));
    }
    // 1. Attach, writing exactly what the attach dialog shows.
    let orders = LogicalDb::parse("orders").expect("a name");
    let lib = block(service.attach_writing(root.clone(), ManifestV1::new(orders, Vec::new())))
        .expect("the library attaches");
    let listed = |rel: &str, env: Option<EnvTag>| {
        let rel = RelPath::parse(rel).expect("a path");
        service
            .snapshot()
            .view(env)
            .library(lib)
            .is_some_and(|l| l.entry(&rel).is_some())
    };
    wait_until(vcx, "the library is listed", |_| {
        listed("report.sql", Some(EnvTag::Production))
    });
    // 2. Map another database of prod's, sharing the alias.
    block(service.map(
        LocalMapping {
            profile: prod,
            database: DatabaseName::new("orders_archive"),
            library: lib,
        },
        true,
    ))
    .expect("the mapping");

    // 3. Open under both connections and run on both.
    connected(vcx, "orders-prod");
    connected(vcx, "orders-staging");
    let on_prod = open(vcx, lib, "report.sql", prod);
    let on_staging = open(vcx, lib, "report.sql", staging);
    for (tab, fake) in [(on_prod, &fakes[0]), (on_staging, &fakes[1])] {
        wait_until(vcx, "the tab's session", |vcx| {
            let console = vcx.cx.read(|cx| Consoles::by_tab(cx, tab)).expect("console");
            vcx.cx.read(|cx| {
                Consoles::get(cx, console).is_some_and(|s| s.session.is_some())
                    && hacivat_app::exec_ui::sql_context(console, cx)
                        != hacivat_app::exec_ui::context::SqlContext::Pending
            })
        });
        run_tab(vcx, tab);
        wait_until(vcx, "the run", |_| queries(fake) == ["SELECT 1"]);
    }

    // 4. Edit: a recovery copy after five quiet seconds, under the app's folders; then save.
    type_text(vcx, on_prod, ";\nSELECT 2");
    vcx.executor()
        .advance_clock(tabs::RECOVERY_IDLE + Duration::from_millis(10));
    wait_until(vcx, "the recovery copy", |vcx| copies(vcx, lib).len() == 1);
    let copy = copies(vcx, lib).remove(0);
    assert!(copy.starts_with(&app_root), "the copy is under the app's folders");
    assert!(!copy.starts_with(&root), "never inside the library");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| std::fs::metadata(p).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode(&copy), 0o600, "the copy's mode");
        assert_eq!(mode(copy.parent().expect("its folder")), 0o700, "its folder's mode");
    }
    focus(vcx, on_prod);
    assert!(run(vcx, "file.save"), "file.save ran");
    wait_until(vcx, "saved", |vcx| !dirty(vcx, on_prod));
    wait_until(vcx, "the copy goes", |vcx| copies(vcx, lib).is_empty());

    // 5. The tree's file operations, under prod.
    let env = Some(EnvTag::Production);
    let rel = |s: &str| RelPath::parse(s).expect("a path");
    block(service.create(lib, &rel("drafts"), EntryKind::Folder, env)).expect("new folder");
    block(service.create(lib, &rel("drafts/new.sql"), EntryKind::Sql, env)).expect("new script");
    wait_until(vcx, "drafts/new.sql listed", |_| listed("drafts/new.sql", env));
    block(service.create(lib, &rel("monthly.sql"), EntryKind::Sql, env)).expect("new script");
    wait_until(vcx, "monthly.sql listed", |_| listed("monthly.sql", env));
    block(service.rename(lib, &rel("monthly.sql"), &rel("weekly.sql"), env)).expect("rename");
    block(service.trash(lib, &rel("drafts/new.sql"), env)).expect("trash");
    // 6. Git refresh.
    block(service.refresh(lib));

    // 7. The layout is saved; quit; relaunch, which restores both tabs; quit.
    let state = app_paths.data.join("workspace.json");
    wait_until(vcx, "the layout is saved", |vcx| {
        vcx.executor().advance_clock(Duration::from_millis(600));
        std::fs::read_to_string(&state).is_ok_and(|t| t.matches("\"script\"").count() == 2)
    });
    service.shutdown();
    close_all_windows(vcx);
    drop(service);
    let (vcx2, _) = shell_with_routes_at(cx, &app_root, "en", routes());
    let ws = workspace(vcx2);
    let (panes, _) = layout(vcx2);
    let restored = vcx2.cx.read(|cx| {
        panes
            .panes
            .values()
            .flat_map(|p| p.tabs.clone())
            .filter(|t| matches!(ws.read(cx).tab_kind(*t), Some(TabKind::Script { .. })))
            .count()
    });
    assert_eq!(restored, 2, "both script tabs restored");
    if let Some(service) = vcx2.cx.read(hacivat_app::scripts::open::service) {
        // Let the restored tabs load before the quit.
        let _ = &service;
        wait_until(vcx2, "the restored library", |_| {
            service
                .snapshot()
                .view(env)
                .library(lib)
                .is_some_and(|l| l.entry(&rel("report.sql")).is_some())
        });
        service.shutdown();
    }
    close_all_windows(vcx2);

    // The audit.
    let all = walk(&root);
    let git_after: BTreeMap<String, Option<Vec<u8>>> = all
        .iter()
        .filter_map(|(n, b)| n.strip_prefix(".git/").map(|n| (n.to_string(), b.clone())))
        .collect();
    assert_eq!(git_after, git_before, ".git/ changed: Hacivat wrote into the repository");
    let mine: BTreeMap<&String, &Option<Vec<u8>>> = all
        .iter()
        .filter(|(n, _)| n.as_str() != ".git" && !n.starts_with(".git/"))
        .collect();
    let needles = needles(&[prod, staging], &["orders-prod", "orders-staging"]);
    let mut found = Vec::new();
    for (name, bytes) in &mine {
        for n in &needles {
            if name.contains(n.as_str()) {
                found.push(format!("{name}: its name holds a credential or identifier"));
            }
            if bytes.as_deref().is_some_and(|b| contains(b, n.as_bytes())) {
                found.push(format!("{name}: holds a credential or identifier"));
            }
        }
    }
    assert!(found.is_empty(), "{found:?}");
    let names: BTreeSet<&str> = mine.keys().map(|n| n.as_str()).collect();
    let want: BTreeSet<&str> = [
        ".hacivat",
        ".hacivat/workspace.toml",
        "drafts",
        "report.sql",
        "weekly.sql",
    ]
    .into_iter()
    .collect();
    assert_eq!(names, want, "the only paths are the user's scripts and the manifest");
    assert_eq!(
        mine[&"report.sql".to_string()].as_deref(),
        Some(&b"SELECT 1;\nSELECT 2"[..]),
        "the saved text"
    );
    let manifest = String::from_utf8(
        mine[&".hacivat/workspace.toml".to_string()]
            .clone()
            .expect("a file"),
    )
    .expect("UTF-8");
    let keys: BTreeSet<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, _)| k.trim()))
        .collect();
    assert_eq!(keys, BTreeSet::from(["aliases", "database", "format"]), "{manifest}");
    assert!(manifest.contains("\"orders_archive\""), "the shared alias: {manifest}");
    // The app's own state never names the library's contents, and lives outside it.
    for f in [
        app_paths.libraries_toml(),
        app_paths.workspace_json(),
        app_paths.connections_toml(),
    ] {
        assert!(!f.starts_with(&root), "{} is inside the library", f.display());
    }
}
