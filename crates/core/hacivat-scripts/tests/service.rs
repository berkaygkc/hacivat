//! Task 10: the library service (HV-SCRIPT-01..04, -08 service half, -11 refresh). One actor on
//! the core runtime owns the registry, the scans, the watch sources and the status refresh, and
//! publishes immutable snapshots through `arc-swap`.
//!
//! Watch hints come from the injected `FakeEventSource` (Task 1a's seam; the OS watcher is
//! Task 9's), so every test is deterministic: it writes a file, then hands the service the hint
//! an OS watcher would, and waits on a condition (never a fixed sleep; ceiling 10 s).
//!
//! The git CLI runs only in `git_*` tests, on fixture repositories they create
//! (`NoGitSubprocess` exempts `tests/`, R-T1a-1).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use hacivat_git::{FileStatus, GitState};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::fileio::{self, ReadLimits};
use hacivat_scripts::scan::{Diag, LimitHit, Limits};
use hacivat_scripts::service::{
    AttachError, LibraryDiag, LibraryService, MapError, ServiceOptions, WorkspaceSnapshot,
};
use hacivat_scripts::testing::{FakeEventSource, FakeHandle};
use hacivat_scripts::{
    DatabaseName, LibraryEvent, LibraryId, LibraryState, LocalMapping, PollReason, RawHint,
    ReadError, RelPath, SaveError, SaveText, TextEncoding, UnavailableReason, WatchError,
};
use hacivat_settings::LocalPaths;
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------------------------
// Support
// ---------------------------------------------------------------------------------------------

const CEILING: Duration = Duration::from_secs(10);

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn opts() -> ServiceOptions {
    ServiceOptions {
        settle: Duration::from_millis(20),
        git_debounce: Duration::from_millis(40),
        poll: Duration::from_millis(60),
        recheck: Duration::from_millis(60),
        home: None,
        ..ServiceOptions::default()
    }
}

/// The canonical form of `p` (macOS temp dirs live under `/private/var`).
fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap()
}

/// Writes `files` (relative, `/`-separated) under `dir`, each with its own name as content.
fn write_files(dir: &Path, files: &[&str]) {
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
}

/// Every file name under `dir` with its bytes, recursively, sorted.
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

/// Waits until `f` holds, polling; panics with `what` after the ceiling.
async fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + CEILING;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The next event that `pred` accepts; every event seen on the way is kept in `seen`.
async fn wait_event(
    rx: &mut broadcast::Receiver<LibraryEvent>,
    seen: &mut Vec<LibraryEvent>,
    what: &str,
    pred: impl Fn(&LibraryEvent) -> bool,
) -> LibraryEvent {
    let deadline = tokio::time::Instant::now() + CEILING;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(e)) => {
                seen.push(e.clone());
                if pred(&e) {
                    return e;
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("event channel closed waiting for {what}: {e}"),
            Err(_) => panic!("timed out waiting for {what}; seen {seen:?}"),
        }
    }
}

/// A service over app directories in `app`, fed by a fake event source.
struct Rig {
    svc: LibraryService,
    rx: broadcast::Receiver<LibraryEvent>,
    fake: FakeHandle,
    paths: LocalPaths,
}

fn rig_with(app: &Path, o: ServiceOptions) -> Rig {
    let paths = LocalPaths::under(app);
    let (source, fake) = FakeEventSource::new();
    let (svc, rx) = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        Box::new(source),
        o,
    );
    Rig {
        svc,
        rx,
        fake,
        paths,
    }
}

async fn ready(r: &Rig) {
    wait_until("the service to load", || {
        r.svc.snapshot().state == LibraryState::Watching
    })
    .await;
}

impl Rig {
    fn snap(&self) -> Arc<WorkspaceSnapshot> {
        self.svc.snapshot()
    }

    fn rels(&self, id: LibraryId) -> Vec<String> {
        self.snap()
            .library(id)
            .map(|l| {
                l.entries
                    .iter()
                    .map(|e| e.rel.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Hands the service the hints an OS watcher would deliver for `files` under `root`.
    fn hint(&self, root: &Path, files: &[&str]) {
        let batch = files.iter().map(|f| RawHint::Path(root.join(f))).collect();
        assert!(self.fake.emit(root, batch), "nothing watches {root:?}");
    }
}

/// Attaches `dir` and waits for its first scan and its first status (both asynchronous), so a
/// test's own change is the next thing published.
async fn attach(r: &Rig, dir: &Path) -> LibraryId {
    let id = r.svc.attach(dir.into(), None, false).await.unwrap();
    wait_until("the first scan and status", || {
        let snap = r.snap();
        snap.git.contains_key(&id)
            && snap
                .library(id)
                .is_some_and(|l| l.state == LibraryState::Watching)
    })
    .await;
    id
}

fn save_text(s: &str) -> SaveText {
    SaveText {
        text: s.into(),
        meta: Default::default(),
    }
}

fn disk_stamp(dir: &Path, id: LibraryId, r: &str) -> hacivat_scripts::DiskStamp {
    let root = hacivat_scripts::LibraryRoot {
        id,
        path: dir.to_path_buf(),
        canonical: canonical(dir),
    };
    fileio::read(&root, &rel(r), None, ReadLimits::default())
        .unwrap()
        .stamp
}

fn file_changed(id: LibraryId, r: &str) -> impl Fn(&LibraryEvent) -> bool + '_ {
    move |e| matches!(e, LibraryEvent::FileChanged { id: i, rel: x, .. } if *i == id && x.as_str() == r)
}

// ---------------------------------------------------------------------------------------------
// Plan Task 10
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn attach_scans_and_publishes() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "@production/b.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = r
        .svc
        .attach(
            d.path().into(),
            Some(hacivat_scripts::testing::logical("orders")),
            true,
        )
        .await
        .unwrap();
    wait_until("three entries", || {
        r.snap().library(id).map(|l| l.entries.len()) == Some(3)
    })
    .await;
    assert_eq!(r.rels(id), ["@production", "@production/b.sql", "a.sql"]);
    let snap = r.snap();
    let lib = snap.library(id).unwrap();
    assert_eq!(lib.db.as_str(), "orders");
    assert_eq!(lib.root.canonical, canonical(d.path()));
    assert_eq!(r.fake.watched(), [canonical(d.path())]);
    // The registry holds it (the single writer, `libraries.toml`).
    let text = std::fs::read_to_string(r.paths.libraries_toml()).unwrap();
    assert!(text.contains(&id.to_string()));
}

#[tokio::test(flavor = "multi_thread")]
async fn manifest_written_only_when_asked() {
    let app = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();
    let asked = tempfile::tempdir().unwrap();
    let existing = tempfile::tempdir().unwrap();
    write_files(plain.path(), &["a.sql"]);
    write_files(asked.path(), &["a.sql"]);
    write_files(existing.path(), &["a.sql"]);
    std::fs::create_dir(existing.path().join(".hacivat")).unwrap();
    let theirs = b"# a teammate's file\nformat = 1\ndatabase = \"billing\"\n";
    std::fs::write(existing.path().join(".hacivat/workspace.toml"), theirs).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;

    let before = tree(plain.path());
    let id = r
        .svc
        .attach(
            plain.path().into(),
            Some(hacivat_scripts::testing::logical("orders")),
            false,
        )
        .await
        .unwrap();
    assert!(!plain.path().join(".hacivat").exists());
    assert_eq!(tree(plain.path()), before);
    let folder = plain
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    wait_until("the folder-name binding", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.db.as_str() == folder)
    })
    .await;

    let id = r
        .svc
        .attach(
            asked.path().into(),
            Some(hacivat_scripts::testing::logical("orders")),
            true,
        )
        .await
        .unwrap();
    let text = std::fs::read_to_string(asked.path().join(".hacivat/workspace.toml")).unwrap();
    assert_eq!(text, "format = 1\ndatabase = \"orders\"\naliases = []\n");
    wait_until("the manifest binding", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.db.as_str() == "orders")
    })
    .await;

    // An existing manifest is read, never rewritten.
    let id = r
        .svc
        .attach(
            existing.path().into(),
            Some(hacivat_scripts::testing::logical("orders")),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(existing.path().join(".hacivat/workspace.toml")).unwrap(),
        theirs
    );
    wait_until("the teammate's binding", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.db.as_str() == "billing")
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn incremental_change_bumps_generation_once() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "b.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    // The attach's own events are not this change's.
    while r.rx.try_recv().is_ok() {}
    let before = r.snap().generation;
    std::fs::write(d.path().join("a.sql"), "SELECT 2; -- longer").unwrap();
    r.hint(&root, &["a.sql"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "a.sql changed",
        file_changed(id, "a.sql"),
    )
    .await;
    let snap = r.snap();
    assert_eq!(snap.generation, before + 1, "events {seen:?}");
    let lib = snap.library(id).unwrap();
    assert_eq!(lib.generation, before + 1);
    let a = lib
        .entries
        .iter()
        .find(|e| e.rel.as_str() == "a.sql")
        .unwrap();
    assert_eq!(a.len, "SELECT 2; -- longer".len() as u64);
    // One `Changed` for that one publish.
    let changed = seen
        .iter()
        .filter(|e| matches!(e, LibraryEvent::Changed { .. }))
        .count();
    assert_eq!(changed, 1, "{seen:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn own_save_produces_no_file_changed_event() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "b.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    let opened = r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    let stamp = r
        .svc
        .save(
            id,
            &rel("a.sql"),
            None,
            save_text("SELECT 42;"),
            Some(opened.stamp),
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(d.path().join("a.sql")).unwrap(),
        b"SELECT 42;"
    );
    // The OS reports our own write (twice, as FSEvents does), then a foreign one on b.sql.
    r.hint(&root, &["a.sql"]);
    r.hint(&root, &["a.sql"]);
    std::fs::write(d.path().join("b.sql"), "foreign edit").unwrap();
    r.hint(&root, &["b.sql"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "b.sql changed",
        file_changed(id, "b.sql"),
    )
    .await;
    // Every change the hints caused has been stamped and announced by now.
    r.svc.settled().await;
    while let Ok(e) = r.rx.try_recv() {
        seen.push(e);
    }
    assert!(
        !seen.iter().any(file_changed(id, "a.sql")),
        "own save reported: {seen:?}"
    );
    let snap = r.snap();
    let a = snap
        .library(id)
        .unwrap()
        .entries
        .iter()
        .find(|e| e.rel.as_str() == "a.sql")
        .cloned()
        .unwrap();
    assert_eq!((a.len, a.mtime), (stamp.len, stamp.mtime));
}

#[tokio::test(flavor = "multi_thread")]
async fn external_change_produces_file_changed_with_new_stamp() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    // Temp then rename, the way most editors save.
    std::fs::write(d.path().join(".a.sql.swp"), "SELECT 'new';").unwrap();
    std::fs::rename(d.path().join(".a.sql.swp"), d.path().join("a.sql")).unwrap();
    r.hint(&root, &[".a.sql.swp", "a.sql"]);
    let mut seen = Vec::new();
    let e = wait_event(
        &mut r.rx,
        &mut seen,
        "a.sql changed",
        file_changed(id, "a.sql"),
    )
    .await;
    let LibraryEvent::FileChanged { stamp, .. } = e else {
        unreachable!()
    };
    assert_eq!(stamp, disk_stamp(d.path(), id, "a.sql"));
    // The temp is never listed.
    assert_eq!(r.rels(id), ["a.sql"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn detach_stops_watching_and_never_touches_the_folder() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "x/b.md", "notes.txt"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    // A recovery copy of this library, which detach discards (R-T5-4).
    let rec = r.paths.script_recovery_dir(&id.to_string()).unwrap();
    std::fs::create_dir_all(&rec).unwrap();
    std::fs::write(rec.join("copy.sql"), "unsaved").unwrap();
    let before = tree(d.path());
    r.svc.detach(id).await.unwrap();
    assert!(r.fake.watched().is_empty());
    assert!(r.snap().library(id).is_none());
    assert_eq!(tree(d.path()), before);
    assert!(!rec.exists());
    let text = std::fs::read_to_string(r.paths.libraries_toml()).unwrap();
    assert!(!text.contains(&id.to_string()));
    // Detaching again is refused, and still touches nothing.
    assert!(r.svc.detach(id).await.is_err());
    assert_eq!(tree(d.path()), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_folder_is_unavailable_and_recovers_when_it_returns() {
    let app = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let lib = parent.path().join("lib");
    write_files(&lib, &["a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, &lib).await;
    let root = canonical(&lib);
    let away = parent.path().join("away");
    std::fs::rename(&lib, &away).unwrap();
    r.hint(&root, &["a.sql"]);
    wait_until("Unavailable(Missing)", || {
        r.snap().library(id).is_some_and(|l| {
            l.state == LibraryState::Unavailable(UnavailableReason::Missing) && l.entries.is_empty()
        })
    })
    .await;
    assert!(r.fake.watched().is_empty());
    assert!(matches!(
        r.svc.read(id, &rel("a.sql"), None).await,
        Err(ReadError::Io(std::io::ErrorKind::NotFound))
    ));
    std::fs::rename(&away, &lib).unwrap();
    wait_until("the library to come back", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Watching)
    })
    .await;
    assert_eq!(r.rels(id), ["a.sql"]);
    assert_eq!(r.fake.watched(), [root]);
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_at_launch_is_unavailable_and_recovers() {
    let app = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let lib = parent.path().join("lib");
    write_files(&lib, &["a.sql"]);
    let id = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        attach(&r, &lib).await
    };
    let away = parent.path().join("away");
    std::fs::rename(&lib, &away).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert_eq!(
        r.snap().library(id).map(|l| l.state.clone()),
        Some(LibraryState::Unavailable(UnavailableReason::Missing))
    );
    std::fs::rename(&away, &lib).unwrap();
    wait_until("the library to come back", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Watching)
    })
    .await;
    assert_eq!(r.rels(id), ["a.sql"]);
}

// ---------------------------------------------------------------------------------------------
// Carries (carries-w1.md, T10 lines)
// ---------------------------------------------------------------------------------------------

/// R-T5-6: the tab's environment is required on every file call, and a `@production` path is
/// refused under staging (and under an untagged connection).
#[tokio::test(flavor = "multi_thread")]
async fn production_path_is_refused_under_staging() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@production/b.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let b = rel("@production/b.sql");
    for env in [Some(EnvTag::Staging), None] {
        assert!(matches!(
            r.svc.read(id, &b, env).await,
            Err(ReadError::OutOfScope)
        ));
        assert!(matches!(
            r.svc
                .save(id, &b, env, save_text("DROP TABLE t;"), None)
                .await,
            Err(SaveError::OutOfScope)
        ));
        assert!(matches!(
            r.svc.open_stream(id, &b, env, TextEncoding::UTF_8).await,
            Err(ReadError::OutOfScope)
        ));
    }
    assert_eq!(
        std::fs::read(d.path().join("@production/b.sql")).unwrap(),
        b"@production/b.sql"
    );
    let ok = r.svc.read(id, &b, Some(EnvTag::Production)).await.unwrap();
    assert_eq!(ok.text, "@production/b.sql");
    let mut stream = r
        .svc
        .open_stream(id, &b, Some(EnvTag::Production), TextEncoding::UTF_8)
        .await
        .unwrap();
    let mut text = String::new();
    while let Some(chunk) = stream.next_chunk() {
        text.push_str(&chunk.unwrap());
    }
    assert_eq!(text, "@production/b.sql");
    // An unknown library opens nothing.
    assert!(matches!(
        r.svc
            .read(LibraryId::new(), &b, Some(EnvTag::Production))
            .await,
        Err(ReadError::Io(std::io::ErrorKind::NotFound))
    ));
}

/// T4 carry: the scan's `stopped` and `suppressed` are visible (R-M4b-14).
#[tokio::test(flavor = "multi_thread")]
async fn scan_stopped_and_suppressed_are_shown() {
    let limited = |limits: Limits| ServiceOptions { limits, ..opts() };
    // The file limit stops the scan where it is.
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "b.sql", "c.sql"]);
    let r = rig_with(
        app.path(),
        limited(Limits {
            max_files: 2,
            ..Limits::default()
        }),
    );
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let snap = r.snap();
    let lib = snap.library(id).unwrap();
    assert_eq!(lib.entries.len(), 2);
    assert!(
        lib.diagnostics
            .contains(&LibraryDiag::ScanStopped(LimitHit::Files(2))),
        "{:?}",
        lib.diagnostics
    );
    drop(r);
    // Diagnostics past the cap are counted, not dropped silently.
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@p1/x.sql", "@p2/x.sql", "@p3/x.sql", "a.sql"]);
    let r = rig_with(
        app.path(),
        limited(Limits {
            max_diags: 1,
            ..Limits::default()
        }),
    );
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let snap = r.snap();
    let diags = &snap.library(id).unwrap().diagnostics;
    assert!(diags.contains(&LibraryDiag::Suppressed(2)), "{diags:?}");
    assert!(
        diags
            .iter()
            .any(|d| matches!(d, LibraryDiag::Scan(Diag::UnknownEnvFolder { .. }))),
        "{diags:?}"
    );
}

/// T4 carry: `RootProblem::InEnvFolder` is mapped, and attach refuses such a root up front.
#[tokio::test(flavor = "multi_thread")]
async fn root_in_an_env_folder_is_refused_and_mapped() {
    let app = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let lib = parent.path().join("@staging").join("lib");
    write_files(&lib, &["a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert!(matches!(
        r.svc.attach(lib.clone(), None, false).await,
        Err(AttachError::RootInEnvFolder(name)) if name == "@staging"
    ));
    assert!(r.snap().libraries.is_empty());
    drop(r);
    // A registry row written by hand (or by an older build) is shown unavailable, with why.
    let id = LibraryId::new();
    let text = format!(
        "version = 1\n\n[[library]]\nid = \"{id}\"\npath = {:?}\nadded_at = \"\"\n",
        lib.to_str().unwrap()
    );
    let paths = LocalPaths::under(app.path());
    std::fs::create_dir_all(&paths.config).unwrap();
    std::fs::write(paths.libraries_toml(), text).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let snap = r.snap();
    let l = snap.library(id).unwrap();
    assert_eq!(
        l.state,
        LibraryState::Unavailable(UnavailableReason::InEnvFolder)
    );
    assert!(l.entries.is_empty());
    assert!(
        l.diagnostics
            .contains(&LibraryDiag::RootInEnvFolder("@staging".into()))
    );
    assert!(r.fake.watched().is_empty());
}

/// T4 carry: `Diag::LinkScope` reaches the library's diagnostics.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn link_scope_diagnostic_is_mapped() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@production/t.sql", "@staging/x.sql"]);
    std::os::unix::fs::symlink("../@production/t.sql", d.path().join("@staging/l.sql")).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let snap = r.snap();
    let diags = &snap.library(id).unwrap().diagnostics;
    assert!(
        diags.iter().any(|d| matches!(
            d,
            LibraryDiag::Scan(Diag::LinkScope { rel }) if rel.as_str() == "@staging/l.sql"
        )),
        "{diags:?}"
    );
}

/// T5 mf4: at launch, recovery folders of libraries that are not registered are swept; a
/// registered library's copies, fresh folders and the instance locks stay.
#[tokio::test(flavor = "multi_thread")]
async fn recovery_folders_of_unregistered_libraries_are_swept_at_launch() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let paths = LocalPaths::under(app.path());
    let kept = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        attach(&r, d.path()).await
    };
    let old = SystemTime::now() - Duration::from_secs(3600);
    let folder = |id: &str, age: SystemTime| {
        let dir = paths.script_recovery_dir(id).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let f = std::fs::File::create(dir.join("x.sql")).unwrap();
        f.set_modified(age).unwrap();
        dir
    };
    let orphan = folder(&LibraryId::new().to_string(), old);
    let fresh = folder(&LibraryId::new().to_string(), SystemTime::now());
    let mine = folder(&kept.to_string(), old);
    let instances = folder("instances", old);
    let not_ours = folder("not-a-uuid", old);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert!(!orphan.exists());
    assert!(
        fresh.exists(),
        "a folder written in the last minutes may be another instance's"
    );
    assert!(mine.exists());
    assert!(instances.exists());
    assert!(not_ours.exists());
}

/// A registry that could not be read is not a list to sweep against.
#[tokio::test(flavor = "multi_thread")]
async fn no_sweep_when_the_registry_is_not_trustworthy() {
    let app = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(app.path());
    std::fs::create_dir_all(&paths.config).unwrap();
    std::fs::write(paths.libraries_toml(), "version = 99\n").unwrap();
    let dir = paths
        .script_recovery_dir(&LibraryId::new().to_string())
        .unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let f = std::fs::File::create(dir.join("x.sql")).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert!(dir.exists());
    assert!(!r.snap().registry.is_empty());
}

// ---------------------------------------------------------------------------------------------
// Watch events (HV-SCRIPT-08 service half)
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn created_and_deleted_files_patch_the_listing() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "x/b.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    write_files(d.path(), &["x/c.sql", "new/deep/d.md"]);
    std::fs::remove_file(d.path().join("a.sql")).unwrap();
    r.hint(&root, &["x/c.sql", "new", "a.sql", "notes.txt"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "a.sql deleted",
        |e| matches!(e, LibraryEvent::FileDeleted { rel, .. } if rel.as_str() == "a.sql"),
    )
    .await;
    wait_until("the new listing", || {
        r.rels(id)
            == [
                "new",
                "new/deep",
                "new/deep/d.md",
                "x",
                "x/b.sql",
                "x/c.sql",
            ]
    })
    .await;
    // A folder deleted as a whole surfaces as one hint on the folder.
    std::fs::remove_dir_all(d.path().join("x")).unwrap();
    r.hint(&root, &["x"]);
    wait_until("x gone", || {
        r.rels(id) == ["new", "new/deep", "new/deep/d.md"]
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rename_on_disk_is_paired_by_content() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "other.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    // The tab read it, so its content hash is known.
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    std::fs::rename(d.path().join("a.sql"), d.path().join("moved.sql")).unwrap();
    r.hint(&root, &["a.sql", "moved.sql"]);
    let mut seen = Vec::new();
    let e = wait_event(&mut r.rx, &mut seen, "the rename", |e| {
        matches!(
            e,
            LibraryEvent::FileRenamed { .. } | LibraryEvent::FileDeleted { .. }
        )
    })
    .await;
    assert_eq!(
        e,
        LibraryEvent::FileRenamed {
            id,
            from: rel("a.sql"),
            to: rel("moved.sql")
        }
    );
    assert_eq!(r.rels(id), ["moved.sql", "other.sql"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn hints_outside_the_root_or_under_dot_folders_are_ignored() {
    let app = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let lib = parent.path().join("lib");
    write_files(&lib, &["a.sql"]);
    write_files(parent.path(), &["outside.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, &lib).await;
    let root = canonical(&lib);
    write_files(&lib, &[".hidden/x.sql", "node_modules/y.sql"]);
    let before = r.snap().generation;
    let batch = vec![
        RawHint::Path(canonical(parent.path()).join("outside.sql")),
        RawHint::Path(root.join("..").join("outside.sql")),
        RawHint::Path(root.join(".hidden/x.sql")),
        RawHint::Path(root.join("node_modules/y.sql")),
    ];
    assert!(r.fake.emit(&root, batch));
    // A marker change processed after the batch proves the batch was handled.
    std::fs::write(lib.join("a.sql"), "marker").unwrap();
    r.hint(&root, &["a.sql"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "the marker",
        file_changed(id, "a.sql"),
    )
    .await;
    assert_eq!(r.rels(id), ["a.sql"]);
    assert_eq!(r.snap().generation, before + 1, "{seen:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_failure_falls_back_to_polling() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let paths = LocalPaths::under(app.path());
    let (source, _fake) = FakeEventSource::refusing(WatchError::Backend("refused".into()));
    let (svc, mut rx) = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        Box::new(source),
        opts(),
    );
    wait_until("load", || svc.snapshot().state == LibraryState::Watching).await;
    let id = svc.attach(d.path().into(), None, false).await.unwrap();
    wait_until("polling", || {
        svc.snapshot()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Polling(PollReason::BackendLost))
    })
    .await;
    // No hint at all: the poll finds the change.
    std::fs::write(d.path().join("a.sql"), "changed while polled").unwrap();
    let mut seen = Vec::new();
    wait_event(
        &mut rx,
        &mut seen,
        "a.sql changed",
        file_changed(id, "a.sql"),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_backend_switches_to_polling() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    assert!(r.fake.emit(&root, vec![RawHint::Lost]));
    wait_until("polling", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Polling(PollReason::BackendLost))
    })
    .await;
    std::fs::write(d.path().join("b.sql"), "new").unwrap();
    std::fs::write(d.path().join("a.sql"), "edited").unwrap();
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "a.sql changed",
        file_changed(id, "a.sql"),
    )
    .await;
    wait_until("b.sql listed", || r.rels(id) == ["a.sql", "b.sql"]).await;
}

// ---------------------------------------------------------------------------------------------
// Registry operations
// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn map_adds_a_local_mapping_and_shares_an_alias_only_when_asked() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    std::fs::create_dir(d.path().join(".hacivat")).unwrap();
    let theirs = "# keep me\nformat = 1\ndatabase = \"orders\"\n";
    std::fs::write(d.path().join(".hacivat/workspace.toml"), theirs).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let profile = ProfileId::new();
    let m = LocalMapping {
        profile,
        database: DatabaseName::new("orders_p"),
        library: id,
    };
    r.svc.map(m.clone(), false).await.unwrap();
    assert_eq!(&*r.snap().mappings, std::slice::from_ref(&m));
    assert_eq!(
        std::fs::read_to_string(d.path().join(".hacivat/workspace.toml")).unwrap(),
        theirs
    );
    let m2 = LocalMapping {
        database: DatabaseName::new("orders_q"),
        ..m.clone()
    };
    r.svc.map(m2.clone(), true).await.unwrap();
    let text = std::fs::read_to_string(d.path().join(".hacivat/workspace.toml")).unwrap();
    assert!(text.starts_with("# keep me\n"), "{text}");
    assert!(text.contains("orders_q"), "{text}");
    wait_until("the shared alias", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.aliases.iter().any(|a| a.as_str() == "orders_q"))
    })
    .await;
    // A mapping to a library that is not registered is refused.
    let bad = LocalMapping {
        library: LibraryId::new(),
        ..m.clone()
    };
    assert!(matches!(
        r.svc.map(bad, false).await,
        Err(MapError::Registry(_))
    ));
    // Unmap and dropping a deleted profile's rows (R-M4b-2).
    assert!(
        r.svc
            .unmap(profile, DatabaseName::new("orders_p"))
            .await
            .unwrap()
    );
    assert_eq!(&*r.snap().mappings, [m2]);
    assert_eq!(r.svc.drop_profile(profile).await.unwrap(), 1);
    assert!(r.snap().mappings.is_empty());
    let text = std::fs::read_to_string(r.paths.libraries_toml()).unwrap();
    assert!(!text.contains("[[mapping]]"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn libraries_and_mappings_survive_a_restart() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "x/b.sql"]);
    let profile = ProfileId::new();
    let id = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        let id = attach(&r, d.path()).await;
        let m = LocalMapping {
            profile,
            database: DatabaseName::new("orders"),
            library: id,
        };
        r.svc.map(m, false).await.unwrap();
        id
    };
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    wait_until("the rescan", || r.rels(id) == ["x", "x/b.sql", "a.sql"]).await;
    assert_eq!(r.snap().mappings.len(), 1);
    assert_eq!(r.fake.watched(), [canonical(d.path())]);
}

#[tokio::test(flavor = "multi_thread")]
async fn attach_refusals_change_nothing() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql", "inner/b.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    assert!(matches!(
        r.svc.attach(d.path().into(), None, false).await,
        Err(AttachError::Registry(hacivat_scripts::RegistryError::AlreadyRegistered(x))) if x == id
    ));
    assert!(matches!(
        r.svc.attach(d.path().join("inner"), None, true).await,
        Err(AttachError::Registry(hacivat_scripts::RegistryError::Nested(x))) if x == id
    ));
    assert!(!d.path().join("inner/.hacivat").exists());
    assert_eq!(r.snap().libraries.len(), 1);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_linked_hacivat_folder_refuses_the_attach_and_writes_nothing() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    std::os::unix::fs::symlink(outside.path(), d.path().join(".hacivat")).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert!(matches!(
        r.svc.attach(d.path().into(), None, true).await,
        Err(AttachError::Manifest(_))
    ));
    assert!(tree(outside.path()).is_empty());
    assert!(r.snap().libraries.is_empty());
    assert!(!r.paths.libraries_toml().exists());
}

// ---------------------------------------------------------------------------------------------
// Repository status refresh (HV-SCRIPT-11 step 7)
// ---------------------------------------------------------------------------------------------

/// Runs the git CLI in `dir` with an empty home and no global or system config.
fn git(dir: &Path, home: &Path, args: &[&str]) {
    let mut c = Command::new("git");
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_PARAMETERS",
    ] {
        c.env_remove(var);
    }
    let out = c
        .args(args)
        .current_dir(dir)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("GIT_CONFIG_GLOBAL", home.join("global.cfg"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .expect("the git CLI builds this test's fixture");
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn fixture_repo(files: &[&str]) -> (tempfile::TempDir, tempfile::TempDir) {
    let repo = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    write_files(repo.path(), files);
    git(repo.path(), home.path(), &["init", "-q"]);
    git(repo.path(), home.path(), &["add", "-A"]);
    git(repo.path(), home.path(), &["commit", "-q", "-m", "fixture"]);
    (repo, home)
}

fn badges(snap: &WorkspaceSnapshot, id: LibraryId) -> Option<BTreeMap<String, FileStatus>> {
    match snap.git.get(&id)? {
        GitState::Ready(s) => Some(
            s.by_path
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), *v))
                .collect(),
        ),
        _ => None,
    }
}

fn print_fact(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("HV-FACT {line}\n").as_bytes());
}

#[tokio::test(flavor = "multi_thread")]
async fn git_refresh_after_index_change() {
    let (repo, home) = fixture_repo(&["a.sql", "b.sql"]);
    let app = tempfile::tempdir().unwrap();
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, repo.path()).await;
    let root = canonical(repo.path());
    wait_until("a clean status", || {
        badges(&r.snap(), id) == Some(BTreeMap::new())
    })
    .await;
    // A change in the library refreshes the status (debounced).
    std::fs::write(repo.path().join("a.sql"), "edited").unwrap();
    r.hint(&root, &["a.sql"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "GitChanged",
        |e| matches!(e, LibraryEvent::GitChanged { id: i } if *i == id),
    )
    .await;
    wait_until("a.sql modified", || {
        badges(&r.snap(), id) == Some(BTreeMap::from([("a.sql".into(), FileStatus::Modified)]))
    })
    .await;
    // A commit touches only `.git/index` and `HEAD`: their hints refresh it too.
    git(repo.path(), home.path(), &["commit", "-q", "-am", "edit"]);
    r.hint(&root, &[".git/index", ".git/HEAD"]);
    wait_until("clean after the commit", || {
        badges(&r.snap(), id) == Some(BTreeMap::new())
    })
    .await;
    print_fact("t10 git_refresh=ok");
}

/// R-T1a-2 carry: the status key equals the scan's `RelPath` byte for byte. macOS keeps the
/// decomposed (NFD) spelling of a name on disk while the index may hold the precomposed one;
/// the badge of an NFD-named modified file must still be found by the entry's own path.
#[tokio::test(flavor = "multi_thread")]
async fn git_badge_of_an_nfd_named_file_is_found_by_its_scan_path() {
    let nfd = "mu\u{308}s\u{327}teri.sql";
    let (repo, _home) = fixture_repo(&[nfd, "plain.sql"]);
    let app = tempfile::tempdir().unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, repo.path()).await;
    let root = canonical(repo.path());
    wait_until("a clean status", || {
        badges(&r.snap(), id) == Some(BTreeMap::new())
    })
    .await;
    let scanned = r
        .snap()
        .library(id)
        .unwrap()
        .entries
        .iter()
        .find(|e| e.rel.as_str() != "plain.sql")
        .map(|e| e.rel.clone())
        .unwrap();
    std::fs::write(root.join(scanned.as_str()), "edited").unwrap();
    r.hint(&root, &[scanned.as_str()]);
    wait_until("the badge", || {
        r.snap().git_status(id, &scanned) == Some(FileStatus::Modified)
    })
    .await;
    let snap = r.snap();
    let keys: Vec<String> = badges(&snap, id).unwrap().into_keys().collect();
    assert_eq!(keys, [scanned.as_str().to_owned()]);
    print_fact(&format!(
        "t10 nfd_badge scan_key_nfd={}",
        scanned.as_str() == nfd
    ));
}

// ---------------------------------------------------------------------------------------------
// Script content goes only through `fileio` (T5 mg2)
// ---------------------------------------------------------------------------------------------

/// A tripwire over this task's sources: the service, the index and the events never touch the
/// file system themselves; every read, write, `lstat` and listing goes through `fileio`,
/// `pathsafe`, `scan`, `manifest`, `registry` or `recovery`. (A check-deps rule would belong in
/// Task 1a's `xtask/src/check_deps.rs`.)
#[test]
fn service_index_and_events_touch_no_file_themselves() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in ["service.rs", "index.rs", "events.rs"] {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        for banned in [
            "std::fs",
            "fs::",
            "File::",
            "OpenOptions",
            "read_to_end",
            "read_to_string",
            "read_dir",
            "canonicalize(",
        ] {
            assert!(!text.contains(banned), "{file} names `{banned}`");
        }
    }
}
