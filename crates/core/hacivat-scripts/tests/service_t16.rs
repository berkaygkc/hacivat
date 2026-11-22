// ## m4b-t16-attach
//! Task 16's core seams: the attach that writes exactly the manifest its dialog showed
//! (R-M4b-8, Q6), the per-library "Watch: automatic / poll" choice (R-T9-1), the confirmed
//! "Use this folder" for a library stuck in `RootChanged` (T10 m-h), and R-T10-7 (a manifest
//! that is there but unreadable keeps the last good binding).
//!
//! Watch hints come from fake event sources (`testing::FakeHandle::sources`); every wait is on
//! a condition with a 10 s ceiling.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hacivat_proto::ProfileId;
use hacivat_scripts::manifest::{self, ManifestV1};
use hacivat_scripts::service::{AttachError, LibraryDiag, LibraryService, ServiceOptions};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::{
    DatabaseName, GitSummary, LibraryId, LibraryState, LocalMapping, LogicalDb, PollReason,
    RawHint, RegistryError, UnavailableReason, WatchMode,
};
use hacivat_settings::LocalPaths;

const CEILING: Duration = Duration::from_secs(10);

fn opts() -> ServiceOptions {
    ServiceOptions {
        settle: Duration::from_millis(20),
        git_debounce: Duration::from_millis(40),
        recheck: Duration::from_millis(60),
        home: None,
        ..ServiceOptions::default()
    }
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

fn write_files(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
}

/// Every file under `dir` with its bytes, sorted.
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

async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + CEILING;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

struct Rig {
    svc: LibraryService,
    fake: FakeHandle,
    paths: LocalPaths,
}

fn rig(app: &Path) -> Rig {
    let paths = LocalPaths::under(app);
    let fake = FakeHandle::default();
    let svc = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        fake.sources(),
        opts(),
    );
    Rig { svc, fake, paths }
}

impl Rig {
    async fn ready(&self) {
        wait_until("the service to load", || {
            self.svc.snapshot().state == LibraryState::Watching
        })
        .await;
    }

    fn state(&self, id: LibraryId) -> Option<LibraryState> {
        self.svc.snapshot().library(id).map(|l| l.state.clone())
    }

    fn db(&self, id: LibraryId) -> Option<String> {
        self.svc
            .snapshot()
            .library(id)
            .map(|l| l.db.as_str().to_owned())
    }

    async fn up(&self, id: LibraryId) {
        wait_until("the first scan and status", || {
            self.svc.snapshot().view(None).library(id).is_some_and(|l| {
                l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
            })
        })
        .await;
        self.svc.settled().await;
    }

    fn registry_text(&self) -> String {
        std::fs::read_to_string(self.paths.libraries_toml()).unwrap_or_default()
    }

    fn hint(&self, root: &Path, files: &[&str]) {
        let batch = files.iter().map(|f| RawHint::Path(root.join(f))).collect();
        assert!(self.fake.emit(root, batch), "nothing watches {root:?}");
    }
}

fn name(s: &str) -> LogicalDb {
    LogicalDb::parse(s).unwrap()
}

// ---- attach_writing ----------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn attach_writing_writes_exactly_the_text_it_was_given() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("orders-scripts");
    write_files(&lib, &["daily.sql"]);
    let before = tree(&lib);
    let r = rig(app.path());
    r.ready().await;
    let m = ManifestV1::new(name("orders"), vec![name("orders_p"), name("sipariş")]);
    let shown = m.to_toml();
    let id = r.svc.attach_writing(lib.clone(), m).await.unwrap();
    r.up(id).await;
    let written = std::fs::read(lib.join(".hacivat/workspace.toml")).unwrap();
    assert_eq!(written, shown.as_bytes(), "the bytes written are the bytes shown");
    // Only that one file was added to the folder.
    let mut after = tree(&lib);
    after.retain(|(n, _)| !n.starts_with(".hacivat"));
    assert_eq!(after, before);
    let table: toml::Table = shown.parse().unwrap();
    let mut keys: Vec<&str> = table.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["aliases", "database", "format"]);
    assert_eq!(r.db(id).as_deref(), Some("orders"));
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_writing_never_replaces_a_file_that_appeared_meanwhile() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("orders-scripts");
    let theirs = "# the team's file\nformat = 1\ndatabase = \"billing\"\n";
    write_files(&lib, &["a.sql"]);
    std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
    std::fs::write(lib.join(".hacivat/workspace.toml"), theirs).unwrap();
    let r = rig(app.path());
    r.ready().await;
    let got = r
        .svc
        .attach_writing(lib.clone(), ManifestV1::new(name("orders"), Vec::new()))
        .await;
    assert_eq!(
        got,
        Err(AttachError::Manifest(manifest::WriteError::Exists))
    );
    assert_eq!(
        std::fs::read_to_string(lib.join(".hacivat/workspace.toml")).unwrap(),
        theirs
    );
    assert!(r.svc.snapshot().libraries.is_empty(), "nothing registered");
}

#[test]
fn preview_reads_an_existing_file_without_following_links() {
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("lib");
    std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
    // Absent.
    let p = manifest::preview(&lib);
    assert!(!p.present && p.text.is_none());
    // Present: its text and what it binds.
    let text = "# keep\nformat = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n";
    std::fs::write(lib.join(".hacivat/workspace.toml"), text).unwrap();
    let p = manifest::preview(&lib);
    assert!(p.present);
    assert_eq!(p.text.as_deref(), Some(text));
    assert_eq!(p.read.db.as_ref().map(LogicalDb::as_str), Some("orders"));
    // A link is never followed: present, no text, a diagnostic.
    #[cfg(unix)]
    {
        let secret = d.path().join("secret.txt");
        std::fs::write(&secret, "hv_test_pw").unwrap();
        std::fs::remove_file(lib.join(".hacivat/workspace.toml")).unwrap();
        std::os::unix::fs::symlink(&secret, lib.join(".hacivat/workspace.toml")).unwrap();
        let p = manifest::preview(&lib);
        assert!(p.present);
        assert!(p.text.is_none(), "a linked manifest's target is never read");
        assert_eq!(p.read.diags, [manifest::Diag::LinkIgnored]);
    }
}

// ---- the shared alias, exactly as shown (Q5) --------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn map_sharing_writes_exactly_the_shown_edit_and_only_over_what_was_shown() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("orders-scripts");
    write_files(&lib, &["a.sql"]);
    std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
    let file = lib.join(".hacivat/workspace.toml");
    let theirs = "# The team's binding: keep this comment.\nformat = 1\ndatabase = \"orders\"\n";
    std::fs::write(&file, theirs).unwrap();
    let r = rig(app.path());
    r.ready().await;
    let id = r.svc.attach(lib.clone(), None, false).await.unwrap();
    r.up(id).await;
    let p = manifest::preview(&lib);
    let shown = manifest::with_alias(&p.bytes, "orders-scripts", &name("orders_p"))
        .unwrap()
        .expect("an edit");
    let m = LocalMapping {
        profile: ProfileId::new(),
        database: DatabaseName::new("orders_p"),
        library: id,
    };
    // A teammate's pull lands between the dialog's read and the press: refused, unchanged.
    let pulled = format!("{theirs}# pulled\n");
    std::fs::write(&file, &pulled).unwrap();
    assert_eq!(
        r.svc.map_sharing(m.clone(), p.bytes.clone()).await,
        Err(hacivat_scripts::MapError::Alias(manifest::WriteError::Changed))
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), pulled);
    assert!(r.svc.snapshot().mappings.is_empty());
    // Over the bytes shown: exactly the shown edit, comments kept, and the mapping.
    std::fs::write(&file, theirs).unwrap();
    r.svc.map_sharing(m.clone(), p.bytes.clone()).await.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), shown);
    let text = String::from_utf8(shown).unwrap();
    assert!(text.starts_with("# The team's binding: keep this comment.\n"));
    assert!(text.contains("aliases = [\"orders_p\"]"));
    assert_eq!(r.svc.snapshot().mappings.as_ref(), [m]);
}

#[test]
fn with_alias_is_none_when_the_name_is_bound_already() {
    let text = b"format = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n";
    assert_eq!(manifest::with_alias(text, "x", &name("ORDERS_P")), Ok(None));
    assert_eq!(manifest::with_alias(text, "x", &name("orders")), Ok(None));
    assert!(matches!(
        manifest::with_alias(b"<<<<", "x", &name("a")),
        Err(manifest::WriteError::Unreadable(_))
    ));
}

// ---- watch: automatic / poll (R-T9-1) -------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_poll_choice_is_saved_and_survives_a_restart() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let id = {
        let r = rig(app.path());
        r.ready().await;
        let id = r.svc.attach(lib.clone(), None, false).await.unwrap();
        r.up(id).await;
        assert_eq!(
            r.svc.snapshot().library(id).map(|l| l.watch),
            Some(WatchMode::Automatic)
        );
        assert!(!r.registry_text().contains("watch"));
        r.svc.set_watch(id, WatchMode::Poll).await.unwrap();
        wait_until("polling by choice", || {
            r.state(id) == Some(LibraryState::Polling(PollReason::UserChoice))
        })
        .await;
        assert_eq!(
            r.svc.snapshot().library(id).map(|l| l.watch),
            Some(WatchMode::Poll)
        );
        assert!(r.registry_text().contains("watch = \"poll\""));
        id
    };
    // A new service reads the choice back, and a change still reaches the polled library.
    // (The fake sources share one table keyed by root, so the hint is sent to a service whose
    // watcher was never replaced: the old watcher's drop would unregister the new one's root.)
    let r = rig(app.path());
    r.ready().await;
    wait_until("polled at the start", || {
        r.state(id) == Some(LibraryState::Polling(PollReason::UserChoice))
    })
    .await;
    r.svc.settled().await;
    write_files(&lib, &["b.sql"]);
    r.hint(&canonical(&lib), &["b.sql"]);
    wait_until("b.sql listed", || {
        r.svc
            .snapshot()
            .view(None)
            .library(id)
            .is_some_and(|l| l.file_count() == 2)
    })
    .await;
    r.svc.set_watch(id, WatchMode::Automatic).await.unwrap();
    wait_until("watched again", || {
        r.state(id) == Some(LibraryState::Watching)
    })
    .await;
    assert!(!r.registry_text().contains("watch"));
}

#[tokio::test(flavor = "multi_thread")]
async fn set_watch_on_an_unknown_library_is_refused() {
    let app = tempfile::tempdir().unwrap();
    let r = rig(app.path());
    r.ready().await;
    assert_eq!(
        r.svc.set_watch(LibraryId::new(), WatchMode::Poll).await,
        Err(RegistryError::UnknownLibrary)
    );
}

// ---- "Use this folder" (T10 m-h) --------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn use_this_folder_adopts_a_replaced_root_and_keeps_its_mappings() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let r = rig(app.path());
    r.ready().await;
    let id = r.svc.attach(lib.clone(), None, false).await.unwrap();
    r.up(id).await;
    let profile = ProfileId::new();
    let m = LocalMapping {
        profile,
        database: DatabaseName::new("orders_p"),
        library: id,
    };
    r.svc.map(m.clone(), false).await.unwrap();
    // The folder is moved away and another one takes its path (a restore from a backup): a
    // different folder, so the library is refused.
    let root = canonical(&lib);
    std::fs::rename(&lib, d.path().join("scripts-old")).unwrap();
    write_files(&lib, &["restored.sql"]);
    r.hint(&root, &[""]);
    wait_until("RootChanged", || {
        r.state(id) == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    // Asked only for a library in that state, and then it is the same library again.
    r.svc.adopt_root(id).await.unwrap();
    wait_until("served again", || {
        r.state(id) == Some(LibraryState::Watching)
    })
    .await;
    let snap = r.svc.snapshot();
    assert_eq!(snap.mappings.as_ref(), [m]);
    let rels: Vec<String> = snap
        .view(None)
        .library(id)
        .map(|l| l.entries().iter().map(|e| e.rel.as_str().to_owned()).collect())
        .unwrap_or_default();
    assert_eq!(rels, ["restored.sql"]);
    assert!(
        !snap
            .view(None)
            .library(id)
            .is_some_and(|l| l.diagnostics().iter().any(|d| matches!(d, LibraryDiag::RootRefused(_))))
    );
    // A second adopt on a served library is refused.
    assert_eq!(
        r.svc.adopt_root(id).await,
        Err(RegistryError::UnknownLibrary)
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn use_this_folder_never_adopts_a_link() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let r = rig(app.path());
    r.ready().await;
    let id = r.svc.attach(lib.clone(), None, false).await.unwrap();
    r.up(id).await;
    let root = canonical(&lib);
    let elsewhere = d.path().join("elsewhere");
    write_files(&elsewhere, &["planted.sql"]);
    std::fs::rename(&lib, d.path().join("scripts-old")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &lib).unwrap();
    r.hint(&root, &[""]);
    wait_until("RootChanged", || {
        r.state(id) == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    r.svc.adopt_root(id).await.unwrap();
    r.svc.settled().await;
    // Validated from scratch: a link root is still refused, and nothing is listed.
    wait_until("refused again", || {
        r.state(id) == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    assert!(
        r.svc
            .snapshot()
            .view(None)
            .library(id)
            .is_some_and(|l| l.file_count() == 0)
    );
}

// ---- R-T10-7 ----------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_manifest_keeps_the_last_good_binding() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    // The folder is named after another database: falling back to it would bind `billing`.
    let lib = d.path().join("billing");
    write_files(&lib, &["a.sql"]);
    std::fs::create_dir_all(lib.join(".hacivat")).unwrap();
    let file = lib.join(".hacivat/workspace.toml");
    std::fs::write(&file, "format = 1\ndatabase = \"orders\"\naliases = [\"orders_p\"]\n").unwrap();
    let r = rig(app.path());
    r.ready().await;
    let id = r.svc.attach(lib.clone(), None, false).await.unwrap();
    r.up(id).await;
    assert_eq!(r.db(id).as_deref(), Some("orders"));
    let root = canonical(&lib);
    // A half-merged file.
    std::fs::write(&file, "<<<<<<< HEAD\ndatabase = \"orders\"\n=======\n").unwrap();
    r.hint(&root, &[".hacivat/workspace.toml"]);
    wait_until("the diagnostic", || {
        r.svc.snapshot().view(None).library(id).is_some_and(|l| {
            l.diagnostics()
                .contains(&LibraryDiag::Manifest(manifest::Diag::InvalidToml))
        })
    })
    .await;
    let snap = r.svc.snapshot();
    let l = snap.library(id).unwrap();
    assert_eq!(l.db.as_str(), "orders", "the last good binding is kept");
    assert_eq!(
        l.aliases.iter().map(LogicalDb::as_str).collect::<Vec<_>>(),
        ["orders_p"]
    );
    // A good file again rebinds.
    std::fs::write(&file, "format = 1\ndatabase = \"orders2\"\n").unwrap();
    r.hint(&root, &[".hacivat/workspace.toml"]);
    wait_until("the new binding", || r.db(id).as_deref() == Some("orders2")).await;
    // Removed on purpose: the folder name binds (R-M4b-7), as before.
    std::fs::remove_file(&file).unwrap();
    r.hint(&root, &[".hacivat/workspace.toml"]);
    wait_until("the folder name", || r.db(id).as_deref() == Some("billing")).await;
}
// ## m4b-t16-attach
