//! Task 10: the library service (HV-SCRIPT-01..04, -08 service half, -11 refresh). One actor on
//! the core runtime owns the registry, the scans, the watch sources and the status refresh, and
//! publishes immutable snapshots through `arc-swap`.
//!
//! Watch hints come from fake event sources (`testing::FakeHandle::sources`, one per library's
//! Task 9 watcher, all driven by one handle), so every test is deterministic: it writes a file, then hands the service the hint
//! an OS watcher would, and waits on a condition (never a fixed sleep; ceiling 10 s).
//!
//! The git CLI runs only in `git_*` tests, on fixture repositories they create
//! (`NoGitSubprocess` exempts `tests/`, R-T1a-1).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use hacivat_git::FileStatus;
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_scripts::ScriptChunks as _;
use hacivat_scripts::fileio::{self, ReadLimits};
use hacivat_scripts::scan::{Diag, Fs, LimitHit, Limits, Meta, RealFs};
use hacivat_scripts::service::{
    AttachError, LibraryDiag, LibraryService, MapError, ServiceOptions, Sources, WorkspaceSnapshot,
};
use hacivat_scripts::testing::FakeHandle;
use hacivat_scripts::watch::{PollEventSource, WatchEvent};
use hacivat_scripts::{
    DatabaseName, EventSource, GitSummary, LibraryEvent, LibraryId, LibraryState, LocalMapping,
    PollReason, RawHint, ReadError, RelPath, SaveError, SaveText, ScriptEntry, Subscription,
    TextEncoding, UnavailableReason, WatchError,
};
// Only the `cfg(unix)` root-refusal tests name it; a plain import is unused on Windows.
#[cfg(unix)]
use hacivat_scripts::RootRefusal;
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
    rx: &mut Subscription,
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
    /// Events as an untagged connection sees them (unscoped files only).
    rx: Subscription,
    fake: FakeHandle,
    paths: LocalPaths,
}

fn rig_with(app: &Path, o: ServiceOptions) -> Rig {
    let paths = LocalPaths::under(app);
    let fake = FakeHandle::default();
    let svc = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        fake.sources(),
        o,
    );
    let rx = svc.subscribe(None);
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

    /// The listing an untagged connection sees.
    fn rels(&self, id: LibraryId) -> Vec<String> {
        self.rels_in(id, None)
    }

    /// The listing a connection whose environment is `env` sees.
    fn rels_in(&self, id: LibraryId, env: Option<EnvTag>) -> Vec<String> {
        entries_in(&self.snap(), id, env)
            .iter()
            .map(|e| e.rel.as_str().to_owned())
            .collect()
    }

    /// Hands the service the hints an OS watcher would deliver for `files` under `root`.
    fn hint(&self, root: &Path, files: &[&str]) {
        let batch = files.iter().map(|f| RawHint::Path(root.join(f))).collect();
        assert!(self.fake.emit(root, batch), "nothing watches {root:?}");
    }
}

fn entries_in(snap: &WorkspaceSnapshot, id: LibraryId, env: Option<EnvTag>) -> Vec<ScriptEntry> {
    snap.view(env)
        .library(id)
        .map(|l| l.entries().into_iter().cloned().collect())
        .unwrap_or_default()
}

fn entry_in(snap: &WorkspaceSnapshot, id: LibraryId, r: &str) -> ScriptEntry {
    snap.view(None)
        .library(id)
        .and_then(|l| l.entry(&rel(r)).cloned())
        .unwrap()
}

fn diags_in(snap: &WorkspaceSnapshot, id: LibraryId, env: Option<EnvTag>) -> Vec<LibraryDiag> {
    snap.view(env)
        .library(id)
        .map(|l| l.diagnostics())
        .unwrap_or_default()
}

/// Attaches `dir` and waits for its first scan and its first status (both asynchronous), so a
/// test's own change is the next thing published.
async fn attach(r: &Rig, dir: &Path) -> LibraryId {
    let id = r.svc.attach(dir.into(), None, false).await.unwrap();
    wait_until("the first scan and status", || {
        let snap = r.snap();
        snap.view(None).library(id).is_some_and(|l| {
            l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
        })
    })
    .await;
    // And the confirming scan after it (the service's own background work), so a test's own
    // change is the next thing that happens.
    r.svc.settled().await;
    id
}

/// [`attach`] for a service where another library hangs: `settled()` would wait for that one,
/// so this waits for this library's own first scan and status only.
async fn attach_while_another_hangs(r: &Rig, dir: &Path) -> LibraryId {
    let id = r.svc.attach(dir.into(), None, false).await.unwrap();
    wait_until("the first scan and status", || {
        r.snap().view(None).library(id).is_some_and(|l| {
            l.git() != GitSummary::Pending && l.library().state == LibraryState::Watching
        })
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
        r.rels_in(id, Some(EnvTag::Production)).len() == 3
    })
    .await;
    assert_eq!(
        r.rels_in(id, Some(EnvTag::Production)),
        ["@production", "@production/b.sql", "a.sql"]
    );
    // An untagged connection never sees the scoped folder or file (m2).
    assert_eq!(r.rels(id), ["a.sql"]);
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
    // A tab has it open (I4: only such files are stamped and announced).
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
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
    let a = entry_in(&snap, id, "a.sql");
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
    r.svc.read(id, &rel("b.sql"), None).await.unwrap();
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
    let a = entry_in(&snap, id, "a.sql");
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
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
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
    wait_until("the watch to stop", || r.fake.watched().is_empty()).await;
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
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Unavailable(UnavailableReason::Missing))
            && r.rels(id).is_empty()
    })
    .await;
    wait_until("the watch to stop", || r.fake.watched().is_empty()).await;
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
    // Each library comes up on its own worker after the registry is read.
    wait_until("Unavailable(Missing)", || {
        r.snap().library(id).map(|l| l.state.clone())
            == Some(LibraryState::Unavailable(UnavailableReason::Missing))
    })
    .await;
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
            r.svc
                .open_stream(id, &b, env, TextEncoding::UTF_8, None)
                .await,
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
        .open_stream(id, &b, Some(EnvTag::Production), TextEncoding::UTF_8, None)
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
    assert_eq!(entries_in(&snap, id, None).len(), 2);
    let diags = diags_in(&snap, id, None);
    assert!(
        diags.contains(&LibraryDiag::ScanStopped(LimitHit::Files(2))),
        "{diags:?}"
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
    let diags = &diags_in(&snap, id, None);
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
    wait_until("Unavailable(InEnvFolder)", || {
        r.snap().library(id).map(|l| l.state.clone())
            == Some(LibraryState::Unavailable(UnavailableReason::InEnvFolder))
    })
    .await;
    let snap = r.snap();
    assert!(entries_in(&snap, id, Some(EnvTag::Staging)).is_empty());
    assert!(diags_in(&snap, id, None).contains(&LibraryDiag::RootInEnvFolder("@staging".into())));
    wait_until("the watch to stop", || r.fake.watched().is_empty()).await;
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
    let diags = &diags_in(&snap, id, Some(EnvTag::Staging));
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
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
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
    // Every source refuses: the watcher falls back to Task 9's poll source.
    let fake = FakeHandle::default();
    let svc = LibraryService::open_with(
        &paths,
        &tokio::runtime::Handle::current(),
        fake.refusing_sources(WatchError::Backend("refused".into())),
        opts(),
    );
    let mut rx = svc.subscribe(None);
    wait_until("load", || svc.snapshot().state == LibraryState::Watching).await;
    let id = svc.attach(d.path().into(), None, false).await.unwrap();
    wait_until("polling", || {
        svc.snapshot()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Polling(PollReason::BackendLost))
    })
    .await;
    svc.read(id, &rel("a.sql"), None).await.unwrap();
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
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
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

/// The badges an untagged connection sees (m2: only shown entries carry one).
fn badges(snap: &WorkspaceSnapshot, id: LibraryId) -> Option<BTreeMap<String, FileStatus>> {
    let view = snap.view(None);
    let lib = view.library(id)?;
    (lib.git() == GitSummary::Ready).then(|| {
        lib.git_statuses()
            .into_iter()
            .map(|(k, v)| (k.as_str().to_owned(), v))
            .collect()
    })
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
    let scanned = entries_in(&r.snap(), id, None)
        .into_iter()
        .find(|e| e.rel.as_str() != "plain.sql")
        .map(|e| e.rel)
        .unwrap();
    std::fs::write(root.join(scanned.as_str()), "edited").unwrap();
    r.hint(&root, &[scanned.as_str()]);
    wait_until("the badge", || {
        r.snap()
            .view(None)
            .library(id)
            .and_then(|l| l.git_status(&scanned))
            == Some(FileStatus::Modified)
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

/// A **tripwire**, not an enforcement: a text scan over this task's sources (`service.rs`,
/// every file under `service/`, `index.rs`, `events.rs`) for the ways a module names the file
/// system itself. Every read, write, `lstat` and listing goes through `fileio`, `pathsafe`,
/// `scan` (the injected `Fs`), `manifest`, `registry` or `recovery`. Fix round 1 (m1) also
/// catches the renamed import the review used (`use std::{fs as disk}`), `tokio::fs`, and new
/// files under `service/`. A token rule in `check-deps` (`source_violations`) is carried to
/// Task 20.
#[test]
fn service_index_and_events_touch_no_file_themselves() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = ["service.rs", "index.rs", "events.rs"]
        .iter()
        .map(|f| src.join(f))
        .collect();
    let mut sub: Vec<PathBuf> = std::fs::read_dir(src.join("service"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    sub.sort();
    assert!(sub.len() >= 4, "{sub:?}");
    files.extend(sub);
    let braced = regex::Regex::new(r"std\s*::\s*\{[^}]*\bfs\b").unwrap();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        for banned in [
            "std::fs",
            "fs::",
            "fs as",
            "tokio::fs",
            "File::",
            "OpenOptions",
            "read_to_end",
            "read_to_string",
            "read_dir(",
            "remove_file",
            "remove_dir",
        ] {
            assert!(!text.contains(banned), "{file:?} names `{banned}`");
        }
        assert!(
            !braced.is_match(&text),
            "{file:?} imports `fs` in a brace group"
        );
    }
    // The probe the review wrote is caught.
    let probe = "use std::{fs as disk};\nfn f() { disk::read(\"x\"); }";
    assert!(probe.contains("fs as") && braced.is_match(probe));
}

// ---------------------------------------------------------------------------------------------
// Fix round 1: the reviewer's hostile tests H1-H7 (adopted) and the rulings' tests
// ---------------------------------------------------------------------------------------------

/// An event source that, like an OS watcher started a moment after the scan would, sees
/// nothing of what happened before `watch`: `watch` itself drops a file into the root first (a
/// teammate's sync landing between the scan and the watch start).
struct LateWatch {
    sinks: Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, hacivat_scripts::HintSink>>>,
}

impl hacivat_scripts::EventSource for LateWatch {
    fn watch(&mut self, root: &Path, sink: hacivat_scripts::HintSink) -> Result<(), WatchError> {
        std::fs::write(root.join("late.sql"), "SELECT 'landed in the window';").unwrap();
        self.sinks.lock().unwrap().insert(root.to_path_buf(), sink);
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        self.sinks.lock().unwrap().remove(root);
    }
}

/// H1 (I5): the watch starts before the first scan, so a file created in between is listed.
#[tokio::test(flavor = "multi_thread")]
async fn h1_file_created_between_scan_and_watch_is_listed() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let svc = LibraryService::open_with(
        &LocalPaths::under(app.path()),
        &tokio::runtime::Handle::current(),
        hacivat_scripts::Sources::Custom(Arc::new(|| {
            Ok(Box::new(LateWatch {
                sinks: Arc::default(),
            }) as Box<dyn hacivat_scripts::EventSource>)
        })),
        opts(),
    );
    wait_until("load", || svc.snapshot().state == LibraryState::Watching).await;
    let id = svc.attach(d.path().into(), None, false).await.unwrap();
    svc.settled().await;
    assert!(d.path().join("late.sql").exists());
    let listed = entries_in(&svc.snapshot(), id, None)
        .iter()
        .any(|e| e.rel.as_str() == "late.sql");
    assert!(listed, "late.sql never listed");
}

/// H2 (I2): the launch sweep spares the copies of an instance that is alive (holds its lock),
/// however old they are.
#[tokio::test(flavor = "multi_thread")]
async fn h2_sweep_spares_a_live_instances_copies() {
    use hacivat_scripts::recovery::{self, CopyMeta, Instance, RecoveryKey};
    let app = tempfile::tempdir().unwrap();
    let paths = LocalPaths::under(app.path());
    let live = Instance::start(&paths).unwrap();
    assert!(live.is_locked());
    let lib = LibraryId::new();
    let key = RecoveryKey {
        library: lib,
        rel: rel("draft.sql"),
        profile: None,
        database: None,
    };
    recovery::store(
        &paths,
        &live,
        &key,
        "-- an hour of unsaved work",
        &CopyMeta::default(),
    )
    .unwrap();
    let dir = paths.script_recovery_dir(&lib.to_string()).unwrap();
    // The live instance's dirty tab has been idle for 11 minutes.
    let old = SystemTime::now() - Duration::from_secs(11 * 60);
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let f = std::fs::File::options().write(true).open(e.path()).unwrap();
        f.set_modified(old).unwrap();
    }
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    assert!(live.is_locked());
    assert!(dir.exists(), "a live instance's copy folder was swept");
    // Once that instance is gone, a later launch sweeps the folder. Fix round 2 (F3): under
    // full parallel load a relaunch now and then still sees the dead instance's lock as held
    // or untestable and keeps the folder, which is the fail-safe answer; so the test relaunches
    // until the sweep happens, within the ceiling.
    drop(live);
    drop(r);
    let deadline = Instant::now() + CEILING;
    let mut launches = 0;
    while dir.exists() {
        assert!(
            Instant::now() < deadline,
            "a dead instance's old copies of an unregistered library stay ({launches} launches)"
        );
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        launches += 1;
    }
    // Fix round 3 (F3): so CI shows whether the relaunches creep up.
    print_fact(&format!("t10 h2 sweep_relaunches={launches}"));
}

/// H3: a full inbox drops hint batches; the overflow flag rescans, so a file whose only hint
/// was dropped is still listed.
#[tokio::test(flavor = "multi_thread")]
async fn h3_inbox_overflow_rescans() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    write_files(d.path(), &["n.sql"]);
    // 20,000 one-path batches: far more than the inbox holds; n.sql's own hint is the last.
    for i in 0..20_000 {
        r.fake
            .emit(&root, vec![RawHint::Path(root.join(format!("x{i}.sql")))]);
    }
    r.fake.emit(&root, vec![RawHint::Path(root.join("n.sql"))]);
    wait_until("n.sql listed", || r.rels(id).iter().any(|x| x == "n.sql")).await;
}

/// H4: generations a reader sees never go backwards, and no library snapshot is newer than
/// its workspace, while hints and commands race; the final listing equals the disk.
#[tokio::test(flavor = "multi_thread")]
async fn h4_generation_is_monotonic_under_races() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    let svc = Arc::new(r.svc);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let (svc, stop) = (svc.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut last = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let s = svc.snapshot();
                assert!(s.generation >= last, "{last} -> {}", s.generation);
                assert!(s.libraries.iter().all(|l| l.generation <= s.generation));
                last = s.generation;
            }
            last
        })
    };
    for i in 0..300 {
        let f = format!("f{}.sql", i % 7);
        if i % 2 == 0 {
            std::fs::write(d.path().join(&f), format!("{i}")).unwrap();
        } else {
            let _ = std::fs::remove_file(d.path().join(&f));
        }
        r.fake.emit(&root, vec![RawHint::Path(root.join(&f))]);
        if i % 50 == 0 {
            let _ = svc.unmap(ProfileId::new(), DatabaseName::new("x")).await;
        }
    }
    svc.settled().await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let last = reader.join().unwrap();
    print_fact(&format!("t10 h4 gen_last={last}"));
    let mut on_disk: Vec<String> = std::fs::read_dir(d.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let listing = || {
        let mut l: Vec<String> = entries_in(&svc.snapshot(), id, None)
            .iter()
            .map(|e| e.rel.as_str().to_owned())
            .collect();
        l.sort();
        l
    };
    let deadline = Instant::now() + CEILING;
    while listing() != on_disk {
        assert!(
            Instant::now() < deadline,
            "listing {:?} disk {on_disk:?}",
            listing()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// H6 (I3): a library folder replaced by a link between sessions is refused at the next
/// launch, never adopted: nothing outside the library is listed.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn h6_root_replaced_by_a_link_between_sessions_is_not_adopted() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let outside = d.path().join("outside");
    write_files(&outside, &["not_in_the_library.sql"]);
    let id = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        let id = attach(&r, &lib).await;
        assert_eq!(r.rels(id), ["a.sql"]);
        id
    };
    std::fs::remove_dir_all(&lib).unwrap();
    std::os::unix::fs::symlink(&outside, &lib).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    wait_until("RootChanged", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    assert!(r.rels(id).is_empty());
    assert!(diags_in(&r.snap(), id, None).contains(&LibraryDiag::RootRefused(RootRefusal::Link)));
    wait_until("the watch to stop", || r.fake.watched().is_empty()).await;
    // Attaching a link is refused up front too.
    assert_eq!(
        r.svc.attach(lib.clone(), None, false).await,
        Err(AttachError::RootIsLink)
    );
}

/// I3: within a session, a folder deleted and recreated at the same path (another identity)
/// is not adopted either.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_root_with_a_new_identity_is_refused() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("scripts");
    write_files(&lib, &["a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, &lib).await;
    let root = canonical(&lib);
    std::fs::remove_dir_all(&lib).unwrap();
    write_files(&lib, &["planted.sql"]);
    r.hint(&root, &[""]);
    wait_until("RootChanged", || {
        r.snap()
            .library(id)
            .is_some_and(|l| l.state == LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    assert!(r.rels(id).is_empty());
    assert!(
        diags_in(&r.snap(), id, None)
            .contains(&LibraryDiag::RootRefused(RootRefusal::IdentityChanged))
    );
}

/// H7 (I1, R-T10-2): a file a staging tab opened, moved from `@staging/` into `@production/`,
/// reaches the staging subscriber as "no longer listed under this connection", never with the
/// production path; a production subscriber hears nothing of the staging path.
#[tokio::test(flavor = "multi_thread")]
async fn h7_rename_is_never_paired_across_an_env_boundary() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@staging/x.sql", "@production/keep.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    let x = rel("@staging/x.sql");
    r.svc.read(id, &x, Some(EnvTag::Staging)).await.unwrap();
    let mut staging = r.svc.subscribe(Some(EnvTag::Staging));
    let mut production = r.svc.subscribe(Some(EnvTag::Production));
    std::fs::rename(
        d.path().join("@staging/x.sql"),
        d.path().join("@production/x.sql"),
    )
    .unwrap();
    r.hint(&root, &["@staging/x.sql", "@production/x.sql"]);
    let mut seen = Vec::new();
    let e = wait_event(&mut staging, &mut seen, "the staging removal", |e| {
        !matches!(
            e,
            LibraryEvent::Changed { .. } | LibraryEvent::GitChanged { .. }
        )
    })
    .await;
    assert_eq!(e, LibraryEvent::FileLeftScope { id, rel: x });
    r.svc.settled().await;
    while let Ok(e) = staging.try_recv() {
        seen.push(e);
    }
    let names_production = |e: &LibraryEvent| format!("{e:?}").contains("@production");
    assert!(!seen.iter().any(names_production), "{seen:?}");
    let mut heard = Vec::new();
    while let Ok(e) = production.try_recv() {
        heard.push(e);
    }
    assert!(
        !heard.iter().any(|e| format!("{e:?}").contains("@staging")),
        "{heard:?}"
    );
    // An untagged subscriber hears neither path.
    let mut untagged = Vec::new();
    while let Ok(e) = r.rx.try_recv() {
        untagged.push(e);
    }
    assert!(
        !untagged.iter().any(|e| format!("{e:?}").contains('@')),
        "{untagged:?}"
    );
}

/// I1: a rename inside one scope reaches that scope's subscriber as a rename.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_inside_a_scope_reaches_its_subscriber() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@staging/x.sql", "@staging/other.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    r.svc
        .read(id, &rel("@staging/x.sql"), Some(EnvTag::Staging))
        .await
        .unwrap();
    let mut staging = r.svc.subscribe(Some(EnvTag::Staging));
    std::fs::rename(
        d.path().join("@staging/x.sql"),
        d.path().join("@staging/y.sql"),
    )
    .unwrap();
    r.hint(&root, &["@staging/x.sql", "@staging/y.sql"]);
    let mut seen = Vec::new();
    let e = wait_event(&mut staging, &mut seen, "the rename", |e| {
        !matches!(
            e,
            LibraryEvent::Changed { .. } | LibraryEvent::GitChanged { .. }
        )
    })
    .await;
    assert_eq!(
        e,
        LibraryEvent::FileRenamed {
            id,
            from: rel("@staging/x.sql"),
            to: rel("@staging/y.sql")
        }
    );
}

/// I4: a changed file that no tab has open is a listing change only; its content is never
/// read. A file a tab opened is stamped and announced.
#[tokio::test(flavor = "multi_thread")]
async fn an_unopened_changed_file_is_never_read() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["open.sql", "closed.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    let before = r.svc.stamps_read();
    std::fs::write(d.path().join("closed.sql"), "changed by a teammate's pull").unwrap();
    r.hint(&root, &["closed.sql"]);
    wait_until("the new length", || {
        entry_in(&r.snap(), id, "closed.sql").len == "changed by a teammate's pull".len() as u64
    })
    .await;
    r.svc.settled().await;
    assert_eq!(r.svc.stamps_read(), before, "an unopened file was read");
    let mut seen = Vec::new();
    while let Ok(e) = r.rx.try_recv() {
        seen.push(e);
    }
    assert!(!seen.iter().any(file_changed(id, "closed.sql")), "{seen:?}");
    // The open one is read once it changes.
    r.svc.read(id, &rel("open.sql"), None).await.unwrap();
    std::fs::write(d.path().join("open.sql"), "edited elsewhere").unwrap();
    r.hint(&root, &["open.sql"]);
    wait_event(
        &mut r.rx,
        &mut seen,
        "open.sql changed",
        file_changed(id, "open.sql"),
    )
    .await;
    assert!(r.svc.stamps_read() > before);
}

/// A file system whose listing of one folder blocks until released (a hung mount).
struct StallFs {
    stalled: PathBuf,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

impl hacivat_scripts::scan::Fs for StallFs {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        if dir.starts_with(&self.stalled) {
            let (lock, cvar) = &*self.gate;
            let mut open = lock.lock().unwrap();
            while !*open {
                open = cvar.wait(open).unwrap();
            }
        }
        hacivat_scripts::scan::RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<hacivat_scripts::scan::Meta> {
        hacivat_scripts::scan::RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        hacivat_scripts::scan::RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> std::io::Result<PathBuf> {
        hacivat_scripts::scan::RealFs.read_link(link)
    }
}

/// I6 (R-T10-4): a library whose scan hangs stalls only itself: another library attaches and
/// scans, commands answer, and detaching the stuck one returns without waiting for its job.
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_library_stalls_only_itself() {
    let app = tempfile::tempdir().unwrap();
    let hung = tempfile::tempdir().unwrap();
    let fine = tempfile::tempdir().unwrap();
    write_files(hung.path(), &["a.sql"]);
    write_files(fine.path(), &["b.sql"]);
    let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let o = ServiceOptions {
        fs: Arc::new(StallFs {
            stalled: canonical(hung.path()),
            gate: gate.clone(),
        }),
        ..opts()
    };
    let r = rig_with(app.path(), o);
    ready(&r).await;
    let release = || {
        let (lock, cvar) = &*gate;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    };
    let stuck = r.svc.attach(hung.path().into(), None, false).await.unwrap();
    let ok = tokio::time::timeout(CEILING, async {
        let other = attach_while_another_hangs(&r, fine.path()).await;
        assert_eq!(r.rels(other), ["b.sql"]);
        let m = LocalMapping {
            profile: ProfileId::new(),
            database: DatabaseName::new("orders"),
            library: other,
        };
        r.svc.map(m, false).await.unwrap();
        r.svc.detach(stuck).await.unwrap();
        assert!(r.snap().library(stuck).is_none());
        assert_eq!(
            r.snap().library(other).map(|l| l.state.clone()),
            Some(LibraryState::Watching)
        );
    })
    .await;
    release();
    ok.expect("a command waited on the hung library");
    r.svc.settled().await;
}

/// I7 (R-T10-3): the pass-1/pass-2 contract. An identical second pass yields the same chunks;
/// a change between the passes refuses pass 2 at open; a change during pass 2 stops it.
#[tokio::test(flavor = "multi_thread")]
async fn two_passes_see_the_same_file_or_stop() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let body: String = (0..30_000).map(|i| format!("SELECT {i};\n")).collect();
    std::fs::write(d.path().join("big.sql"), &body).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let big = rel("big.sql");
    let utf8 = TextEncoding::UTF_8;
    let drain = |s: &mut hacivat_scripts::ScriptStream| {
        use hacivat_scripts::ScriptChunks as _;
        let mut out = Vec::new();
        while let Some(c) = s.next_chunk() {
            out.push(c);
        }
        out
    };
    let mut pass1 = r.svc.open_stream(id, &big, None, utf8, None).await.unwrap();
    let chunks1 = drain(&mut pass1);
    let record = pass1.record();
    assert!(record.is_complete() && record.blocks() >= 3, "{record:?}");
    assert_eq!(record.len, body.len() as u64);
    let mut pass2 = r
        .svc
        .open_stream(id, &big, None, utf8, Some(record.clone()))
        .await
        .unwrap();
    assert_eq!(drain(&mut pass2), chunks1);
    // A change during pass 2: the next chunk stops it.
    let mut pass2 = r
        .svc
        .open_stream(id, &big, None, utf8, Some(record.clone()))
        .await
        .unwrap();
    {
        use hacivat_scripts::ScriptChunks as _;
        assert!(pass2.next_chunk().unwrap().is_ok());
        std::fs::write(
            d.path().join("big.sql"),
            body.replace("SELECT 29999", "DELETE 29999"),
        )
        .unwrap();
        let rest: Vec<_> = std::iter::from_fn(|| pass2.next_chunk()).collect();
        assert_eq!(
            rest.last(),
            Some(&Err(hacivat_scripts::ChunkError::Changed))
        );
        assert!(
            rest[..rest.len() - 1].is_empty(),
            "{} chunks after the change",
            rest.len()
        );
    }
    // A change between the passes: pass 2 is refused at once.
    assert!(matches!(
        r.svc.open_stream(id, &big, None, utf8, Some(record)).await,
        Err(ReadError::Changed)
    ));
}

/// m2: a connection's view filters counts, badges and diagnostics by its environment; a
/// diagnostic naming another environment's folder is counted, not named.
#[tokio::test(flavor = "multi_thread")]
async fn a_view_never_names_what_its_connection_does_not_show() {
    let (repo, _home) = fixture_repo(&[
        "a.sql",
        "@production/p.sql",
        "@staging/s.sql",
        "@production/@staging/c.sql",
        "@prod/x.sql",
    ]);
    let app = tempfile::tempdir().unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, repo.path()).await;
    let root = canonical(repo.path());
    std::fs::write(root.join("@production/p.sql"), "edited").unwrap();
    std::fs::write(root.join("a.sql"), "edited").unwrap();
    r.hint(&root, &["@production/p.sql", "a.sql"]);
    let badge = |env: Option<EnvTag>, path: &str| {
        r.snap()
            .view(env)
            .library(id)
            .and_then(|l| l.git_status(&rel(path)))
    };
    wait_until("the badges", || {
        badge(Some(EnvTag::Production), "@production/p.sql") == Some(FileStatus::Modified)
    })
    .await;
    let snap = r.snap();
    let staging = snap.view(Some(EnvTag::Staging));
    let production = snap.view(Some(EnvTag::Production));
    let (s, p) = (
        staging.library(id).unwrap(),
        production.library(id).unwrap(),
    );
    // Counts.
    assert_eq!(s.file_count(), 2);
    assert_eq!(p.file_count(), 2);
    assert_eq!(snap.view(None).library(id).unwrap().file_count(), 1);
    // Badges.
    assert_eq!(badge(Some(EnvTag::Staging), "@production/p.sql"), None);
    let staged: Vec<String> = s
        .git_statuses()
        .into_iter()
        .map(|(k, _)| k.as_str().to_owned())
        .collect();
    assert_eq!(staged, ["a.sql"]);
    // Diagnostics: the misspelt folder is named (R-M4b-6), the contradiction only counted.
    for v in [s, p] {
        let diags = v.diagnostics();
        assert!(
            diags
                .iter()
                .any(|d| matches!(d, LibraryDiag::Scan(Diag::UnknownEnvFolder { .. }))),
            "{diags:?}"
        );
        assert!(diags.contains(&LibraryDiag::HiddenProblems(1)), "{diags:?}");
        assert!(
            !format!("{diags:?}").contains("@production/@staging"),
            "{diags:?}"
        );
    }
    // Entries: no folder or file of another environment.
    assert!(
        s.entries()
            .iter()
            .all(|e| !e.rel.as_str().starts_with("@production"))
    );
}

// ---------------------------------------------------------------------------------------------
// Fix round 2: the re-review's probes H6b, H8, H9, H11, H12 (adopted) and F1's hung mount
// ---------------------------------------------------------------------------------------------

/// A file system on which every call under one of `stalled` blocks until the gate opens (a
/// dead NFS or SMB mount: `lstat`, `realpath`, `readdir` and `readlink` all hang). Counts
/// the calls that blocked.
struct DeadMount {
    stalled: Vec<PathBuf>,
    gate: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    entered: Arc<std::sync::atomic::AtomicUsize>,
}

impl DeadMount {
    fn new(stalled: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(DeadMount {
            stalled,
            gate: Arc::default(),
            entered: Arc::default(),
        })
    }

    fn wait(&self, path: &Path) {
        if !self.stalled.iter().any(|s| path.starts_with(s)) {
            return;
        }
        let (lock, cvar) = &*self.gate;
        let mut open = lock.lock().unwrap();
        if !*open {
            self.entered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        while !*open {
            open = cvar.wait(open).unwrap();
        }
    }

    fn release(&self) {
        let (lock, cvar) = &*self.gate;
        *lock.lock().unwrap() = true;
        cvar.notify_all();
    }

    fn blocked(&self) -> usize {
        self.entered.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// The service's injected `Fs`, sharing one `DeadMount`.
struct Mounted(Arc<DeadMount>);

impl hacivat_scripts::scan::Fs for Mounted {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        self.0.wait(dir);
        hacivat_scripts::scan::RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<hacivat_scripts::scan::Meta> {
        self.0.wait(path);
        hacivat_scripts::scan::RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        self.0.wait(path);
        hacivat_scripts::scan::RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> std::io::Result<PathBuf> {
        self.0.wait(link);
        hacivat_scripts::scan::RealFs.read_link(link)
    }
}

/// Options with a short job deadline (F2), over `mount`.
fn on_mount(mount: &Arc<DeadMount>) -> ServiceOptions {
    ServiceOptions {
        fs: Arc::new(Mounted(mount.clone())),
        job_deadline: Duration::from_millis(300),
        ..opts()
    }
}

fn state_of(r: &Rig, id: LibraryId) -> Option<LibraryState> {
    r.snap().library(id).map(|l| l.state.clone())
}

/// F1: a library on a hung mount blocks nothing at launch, at attach or on a registry reload.
/// The actor itself never touches a library folder; only that library's own job waits.
#[tokio::test(flavor = "multi_thread")]
async fn a_hung_mount_never_blocks_launch_attach_or_reload() {
    let app = tempfile::tempdir().unwrap();
    let hung = tempfile::tempdir().unwrap();
    let fine = tempfile::tempdir().unwrap();
    let late = tempfile::tempdir().unwrap();
    write_files(hung.path(), &["a.sql"]);
    write_files(fine.path(), &["b.sql"]);
    write_files(late.path(), &["c.sql"]);
    let (hung_id, fine_id) = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        (attach(&r, hung.path()).await, attach(&r, fine.path()).await)
    };
    // The mount dies between sessions.
    let mount = DeadMount::new(vec![canonical(hung.path())]);
    let r = rig_with(app.path(), on_mount(&mount));
    let all = tokio::time::timeout(CEILING, async {
        ready(&r).await;
        wait_until("the healthy library", || {
            state_of(&r, fine_id) == Some(LibraryState::Watching)
        })
        .await;
        wait_until("NotResponding", || {
            state_of(&r, hung_id)
                == Some(LibraryState::Unavailable(UnavailableReason::NotResponding))
        })
        .await;
        // Attach while it hangs.
        let id = attach_while_another_hangs(&r, late.path()).await;
        assert_eq!(r.rels(id), ["c.sql"]);
        // A command that saves the registry, and one whose save fails (the registry is then
        // read back from the file, by text only).
        let m = LocalMapping {
            profile: ProfileId::new(),
            database: DatabaseName::new("orders"),
            library: fine_id,
        };
        r.svc.map(m, false).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let config = r.paths.config.clone();
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o500)).unwrap();
            // Running as root, a read-only folder is still writable: then there is no failure
            // to provoke.
            let probe = config.join(".hv-t10-probe");
            let writable = std::fs::write(&probe, b"x").is_ok();
            let _ = std::fs::remove_file(&probe);
            if !writable {
                assert!(r.svc.drop_profile(ProfileId::new()).await.is_err());
            }
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        r.svc.detach(hung_id).await.unwrap();
        assert!(r.snap().library(hung_id).is_none());
    })
    .await;
    let blocked = mount.blocked();
    mount.release();
    all.expect("the service waited on the hung mount");
    assert!(blocked >= 1, "the hung folder was never asked for");
    r.svc.settled().await;
}

/// F2: a library that stops answering is `NotResponding` and comes back once it answers.
#[tokio::test(flavor = "multi_thread")]
async fn a_library_that_stops_answering_comes_back() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["a.sql"]);
    let id = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        attach(&r, d.path()).await
    };
    let mount = DeadMount::new(vec![canonical(d.path())]);
    let r = rig_with(app.path(), on_mount(&mount));
    ready(&r).await;
    wait_until("NotResponding", || {
        state_of(&r, id) == Some(LibraryState::Unavailable(UnavailableReason::NotResponding))
    })
    .await;
    assert!(r.rels(id).is_empty());
    mount.release();
    wait_until("the library to answer", || {
        state_of(&r, id) == Some(LibraryState::Watching) && r.rels(id) == ["a.sql"]
    })
    .await;
    assert_eq!(r.fake.watched(), [canonical(d.path())]);
}

/// H8 (F2, R-T10-5): 20 libraries, 8 of them on one dead mount: the 12 healthy ones come up,
/// and the 8 are shown not responding.
#[tokio::test(flavor = "multi_thread")]
async fn h8_eight_hung_libraries_of_twenty_stall_only_themselves() {
    let app = tempfile::tempdir().unwrap();
    let base = tempfile::tempdir().unwrap();
    let mut dirs = Vec::new();
    for k in 0..20 {
        let p = base.path().join(format!("lib{k:02}"));
        write_files(&p, &["a.sql"]);
        dirs.push(canonical(&p));
    }
    // Only listings hang (the review's probe): the attach itself resolves the folder.
    let mount = DeadMount::new(dirs[..8].to_vec());
    let listings_only = ServiceOptions {
        fs: Arc::new(ListingsOnly(mount.clone())),
        job_deadline: Duration::from_millis(300),
        ..opts()
    };
    let r = rig_with(app.path(), listings_only);
    ready(&r).await;
    let mut ids = Vec::new();
    for d in &dirs {
        ids.push(r.svc.attach(d.clone(), None, false).await.unwrap());
    }
    wait_until("the 12 healthy libraries", || {
        ids[8..]
            .iter()
            .all(|id| state_of(&r, *id) == Some(LibraryState::Watching))
    })
    .await;
    wait_until("the 8 hung ones shown not responding", || {
        ids[..8].iter().all(|id| {
            state_of(&r, *id) == Some(LibraryState::Unavailable(UnavailableReason::NotResponding))
        })
    })
    .await;
    print_fact(&format!("t10 h8 blocked={} fine_up=12/12", mount.blocked()));
    mount.release();
    r.svc.settled().await;
}

/// Listings under the dead mount hang; `lstat` and `realpath` answer (the review's H8/H9).
struct ListingsOnly(Arc<DeadMount>);

impl hacivat_scripts::scan::Fs for ListingsOnly {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        self.0.wait(dir);
        hacivat_scripts::scan::RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<hacivat_scripts::scan::Meta> {
        hacivat_scripts::scan::RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        hacivat_scripts::scan::RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> std::io::Result<PathBuf> {
        hacivat_scripts::scan::RealFs.read_link(link)
    }
}

/// H9 (F2): detaching a hung library frees its worker at once, so a user who retries (detach,
/// attach again) eight times still gets a healthy library scanned.
#[tokio::test(flavor = "multi_thread")]
async fn h9_detach_of_a_hung_library_releases_its_worker() {
    let app = tempfile::tempdir().unwrap();
    let hung = tempfile::tempdir().unwrap();
    let fine = tempfile::tempdir().unwrap();
    write_files(hung.path(), &["a.sql"]);
    write_files(fine.path(), &["b.sql"]);
    let mount = DeadMount::new(vec![canonical(hung.path())]);
    let o = ServiceOptions {
        fs: Arc::new(ListingsOnly(mount.clone())),
        // A deadline far away: only the detach can free the workers here.
        job_deadline: Duration::from_secs(3600),
        ..opts()
    };
    let r = rig_with(app.path(), o);
    ready(&r).await;
    for k in 1..=8 {
        let id = r.svc.attach(hung.path().into(), None, false).await.unwrap();
        wait_until("the retry to hang", || mount.blocked() >= k).await;
        let t = Instant::now();
        r.svc.detach(id).await.unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "detach waited");
    }
    let other = attach(&r, fine.path()).await;
    assert_eq!(r.rels(other), ["b.sql"]);
    mount.release();
    r.svc.settled().await;
}

/// H6b (m-a): a folder above the root replaced by a link between sessions (its target holds a
/// folder of the same name) is refused at the next launch: the recorded canonical path differs.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn h6b_parent_replaced_by_a_link_between_sessions_is_refused() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    let lib = d.path().join("db").join("scripts");
    write_files(&lib, &["a.sql"]);
    let other = d.path().join("elsewhere");
    write_files(&other, &["scripts/not_in_the_library.sql"]);
    let id = {
        let r = rig_with(app.path(), opts());
        ready(&r).await;
        let id = attach(&r, &lib).await;
        assert_eq!(r.rels(id), ["a.sql"]);
        id
    };
    let toml = std::fs::read_to_string(LocalPaths::under(app.path()).libraries_toml()).unwrap();
    assert!(toml.contains("canonical = "), "{toml}");
    std::fs::remove_dir_all(d.path().join("db")).unwrap();
    std::os::unix::fs::symlink(&other, d.path().join("db")).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    wait_until("RootChanged", || {
        state_of(&r, id) == Some(LibraryState::Unavailable(UnavailableReason::RootChanged))
    })
    .await;
    assert!(r.rels(id).is_empty());
    assert!(diags_in(&r.snap(), id, None).contains(&LibraryDiag::RootRefused(RootRefusal::Moved)));
}

/// H11 (m-b): `Debug` of a snapshot, a view or a library (a log line) names no file of any
/// environment.
#[tokio::test(flavor = "multi_thread")]
async fn h11_debug_of_a_view_names_nothing_hidden() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["@production/secret_plan.sql", "a.sql"]);
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let snap = r.snap();
    let view = snap.view(Some(EnvTag::Staging));
    let lv = view.library(id).unwrap();
    assert_eq!(lv.file_count(), 1);
    let index = hacivat_scripts::ScriptIndex::build(&snap);
    let printed = format!("{snap:?} {view:?} {lv:?} {:?} {index:?}", lv.library());
    assert!(!printed.contains("secret_plan"), "{printed}");
    assert!(!printed.contains("a.sql"), "{printed}");
    assert!(!index.is_empty_for(Some(EnvTag::Staging)));
    assert!(!index.is_empty_for(None));
}

/// H12 (F4): pass 2 in another encoding than pass 1 is refused before it hands out any text.
#[tokio::test(flavor = "multi_thread")]
async fn h12_pass_two_in_another_encoding_is_refused() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("e.sql"), "SELECT 'é';\n".as_bytes()).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let e = rel("e.sql");
    let mut p1 = r
        .svc
        .open_stream(id, &e, None, TextEncoding::UTF_8, None)
        .await
        .unwrap();
    let mut t1 = String::new();
    while let Some(c) = p1.next_chunk() {
        t1.push_str(&c.unwrap());
    }
    let record = p1.record();
    assert_eq!(record.encoding, TextEncoding::UTF_8);
    assert_eq!(record.bom, Some(false));
    let w1254 = hacivat_scripts::fileio::lookup_encoding("windows-1254").unwrap();
    assert!(matches!(
        r.svc
            .open_stream(id, &e, None, w1254, Some(record.clone()))
            .await,
        Err(ReadError::Changed)
    ));
    // The same encoding passes.
    let mut p2 = r
        .svc
        .open_stream(id, &e, None, TextEncoding::UTF_8, Some(record))
        .await
        .unwrap();
    let mut t2 = String::new();
    while let Some(c) = p2.next_chunk() {
        t2.push_str(&c.unwrap());
    }
    assert_eq!(t2, t1);
}

/// F4: pass 1 dropped a BOM; a pass 2 whose file has none any more stops at its first block.
#[tokio::test(flavor = "multi_thread")]
async fn pass_two_without_pass_ones_bom_stops() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("b.sql"), "\u{feff}SELECT 1;\n".as_bytes()).unwrap();
    let r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let b = rel("b.sql");
    let mut p1 = r
        .svc
        .open_stream(id, &b, None, TextEncoding::UTF_8, None)
        .await
        .unwrap();
    while p1.next_chunk().is_some() {}
    let record = p1.record();
    assert_eq!(record.bom, Some(true));
    let mut forged = record;
    forged.bom = Some(false);
    let mut p2 = r
        .svc
        .open_stream(id, &b, None, TextEncoding::UTF_8, Some(forged))
        .await
        .unwrap();
    assert_eq!(
        p2.next_chunk(),
        Some(Err(hacivat_scripts::ChunkError::Changed))
    );
}

/// T9 carries: a hint for a file that did not change (a `PossiblyChanged` for an unopened
/// file, a re-hash that matches for an open one) reaches no subscriber, and only the open file
/// is ever read.
#[tokio::test(flavor = "multi_thread")]
async fn a_hint_that_changed_nothing_reaches_nobody() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    write_files(d.path(), &["open.sql", "closed.sql"]);
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    r.svc.read(id, &rel("open.sql"), None).await.unwrap();
    r.svc.settled().await;
    while r.rx.try_recv().is_ok() {}
    let before = r.svc.stamps_read();
    for _ in 0..3 {
        r.hint(&root, &["open.sql", "closed.sql"]);
    }
    // A real change afterwards proves the hints above were all handled.
    std::fs::write(d.path().join("open.sql"), "edited").unwrap();
    r.hint(&root, &["open.sql"]);
    let mut seen = Vec::new();
    wait_event(
        &mut r.rx,
        &mut seen,
        "open.sql changed",
        file_changed(id, "open.sql"),
    )
    .await;
    r.svc.settled().await;
    while let Ok(e) = r.rx.try_recv() {
        seen.push(e);
    }
    let changed = seen
        .iter()
        .filter(|e| matches!(e, LibraryEvent::FileChanged { .. }))
        .count();
    assert_eq!(changed, 1, "{seen:?}");
    let read = r.svc.stamps_read() - before;
    assert!(read <= 2, "{read} content reads");
}

// ---------------------------------------------------------------------------------------------
// ## m4b-t10-service fix round 3: the re-review's H14, H14b, H15 and H16, adopted
// ---------------------------------------------------------------------------------------------

/// Every event in the subscription's queue but the snapshot and status notices.
fn file_events(rx: &mut Subscription) -> Vec<LibraryEvent> {
    let mut seen = Vec::new();
    while let Ok(e) = rx.try_recv() {
        if !matches!(
            e,
            LibraryEvent::Changed { .. } | LibraryEvent::GitChanged { .. }
        ) {
            seen.push(e);
        }
    }
    seen
}

fn is_deleted(e: &LibraryEvent, r: &RelPath) -> bool {
    matches!(e, LibraryEvent::FileDeleted { rel, .. } if rel == r)
}

fn is_renamed(e: &LibraryEvent, f: &RelPath, t: &RelPath) -> bool {
    matches!(e, LibraryEvent::FileRenamed { from, to, .. } if from == f && to == t)
}

/// H14 (G1): an open file renamed while a rescan's confirming chain runs, with the watcher's
/// word for it arriving only after the chain settled (a polled or busy backend). The tab
/// never hears "deleted"; it hears the rename. 20 rounds.
#[tokio::test(flavor = "multi_thread")]
async fn h14_rename_racing_a_confirming_scan_is_never_a_delete() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.sql"), "SELECT 'mine';").unwrap();
    std::fs::write(d.path().join("other.sql"), "x").unwrap();
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    // Open too: its edits are the marker that a round's late hints were handled.
    r.svc.read(id, &rel("other.sql"), None).await.unwrap();
    r.svc.settled().await;
    file_events(&mut r.rx);
    for round in 0..20 {
        let (from, to) = if round % 2 == 0 {
            ("a.sql", "b.sql")
        } else {
            ("b.sql", "a.sql")
        };
        // The rename lands, and a rescan's chain sees it before the watcher says anything
        // about it. (Renamed first: a test thread descheduled between a `Rescan` and the
        // rename could let the whole chain end before the rename, with no word ever after.)
        std::fs::rename(d.path().join(from), d.path().join(to)).unwrap();
        assert!(r.fake.emit(&root, vec![RawHint::Rescan(root.clone())]));
        // The chain runs to its end (`settled()` alone does not wait for the watcher's own
        // settle, m-i).
        wait_until("the listing to show the new name", || {
            r.rels(id) == [to, "other.sql"]
        })
        .await;
        r.svc.settled().await;
        // Only then the watcher's word for it, and a marker change after it.
        r.hint(&root, &[from, to]);
        std::fs::write(d.path().join("other.sql"), format!("marker {round}")).unwrap();
        r.hint(&root, &["other.sql"]);
        let mut seen = Vec::new();
        wait_event(
            &mut r.rx,
            &mut seen,
            "the marker",
            file_changed(id, "other.sql"),
        )
        .await;
        r.svc.settled().await;
        seen.extend(file_events(&mut r.rx));
        let (f, t) = (rel(from), rel(to));
        assert!(
            !seen.iter().any(|e| is_deleted(e, &f)),
            "round {round}: the tab heard deleted: {seen:?}"
        );
        assert!(
            seen.iter().any(|e| is_renamed(e, &f, &t)),
            "round {round}: no rename heard: {seen:?}"
        );
        assert_eq!(r.rels(id), [to, "other.sql"]);
    }
}

/// Renames an open file back and forth 20 times, each with the watcher's hints in time; with
/// `rescan`, a `Rescan` comes in the same moment (what Task 9 sends for a burst, an overflow or
/// a limit: a `git checkout` or `pull`), so the watcher itself never pairs it.
async fn rename_rounds(rescan: bool) -> (usize, usize, Vec<LibraryEvent>) {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.sql"), "SELECT 'mine';").unwrap();
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    r.svc.read(id, &rel("a.sql"), None).await.unwrap();
    r.svc.settled().await;
    file_events(&mut r.rx);
    let mut seen = Vec::new();
    for round in 0..20 {
        let (from, to) = if round % 2 == 0 {
            ("a.sql", "b.sql")
        } else {
            ("b.sql", "a.sql")
        };
        if rescan {
            assert!(r.fake.emit(&root, vec![RawHint::Rescan(root.clone())]));
        }
        std::fs::rename(d.path().join(from), d.path().join(to)).unwrap();
        r.hint(&root, &[from, to]);
        wait_until("the listing to show the new name", || r.rels(id) == [to]).await;
        r.svc.settled().await;
        let round_events = file_events(&mut r.rx);
        let (f, t) = (rel(from), rel(to));
        let deleted_at = round_events.iter().position(|e| is_deleted(e, &f));
        let renamed_at = round_events.iter().position(|e| is_renamed(e, &f, &t));
        assert!(
            deleted_at.is_none_or(|d| renamed_at.is_some_and(|x| x < d)),
            "round {round}: deleted before renamed: {round_events:?}"
        );
        seen.extend(round_events);
    }
    let deletes = seen
        .iter()
        .filter(|e| matches!(e, LibraryEvent::FileDeleted { .. }))
        .count();
    let renames = seen
        .iter()
        .filter(|e| matches!(e, LibraryEvent::FileRenamed { .. }))
        .count();
    (renames, deletes, seen)
}

/// H14b (G1): a rename with a `Rescan` in the same moment is one rename, 20 rounds of 20.
#[tokio::test(flavor = "multi_thread")]
async fn h14b_rename_during_a_rescan_is_one_rename() {
    let (renames, deletes, seen) = rename_rounds(true).await;
    print_fact(&format!(
        "t10-fr3 h14b rescan=true renames={renames} deletes={deletes}"
    ));
    assert_eq!((renames, deletes), (20, 0), "{seen:?}");
}

/// H14b's control: without the rescan, the watcher's own pairing gives 20 renames.
#[tokio::test(flavor = "multi_thread")]
async fn h14b_rename_reported_in_time_is_one_rename() {
    let (renames, deletes, seen) = rename_rounds(false).await;
    print_fact(&format!(
        "t10-fr3 h14b rescan=false renames={renames} deletes={deletes}"
    ));
    assert_eq!((renames, deletes), (20, 0), "{seen:?}");
}

/// A library whose `.hacivat/workspace.toml` says `database = "orders"`.
fn bound_library() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join(".hacivat")).unwrap();
    std::fs::write(
        d.path().join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"orders\"\n",
    )
    .unwrap();
    std::fs::write(d.path().join("a.sql"), "x").unwrap();
    d
}

fn rebind_to_billing(d: &Path) {
    std::fs::write(
        d.join(".hacivat/workspace.toml"),
        "format = 1\ndatabase = \"billing\"\n",
    )
    .unwrap();
}

fn db_of(r: &Rig, id: LibraryId) -> Option<String> {
    r.snap().library(id).map(|l| l.db.as_str().to_owned())
}

/// Counts root listings (a full scan lists the root once).
struct CountingFs {
    root: PathBuf,
    lists: Arc<AtomicUsize>,
}

impl Fs for CountingFs {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        if dir == self.root {
            self.lists.fetch_add(1, Ordering::SeqCst);
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

/// H15 (G2, R-T10-6): a teammate's pull rewrites `.hacivat/workspace.toml` (`orders` →
/// `billing`); the watcher's word on that path re-binds the library after the debounce, with
/// a manifest-only read (no full scan).
#[tokio::test(flavor = "multi_thread")]
async fn h15_manifest_change_rebinds_without_a_rescan() {
    let app = tempfile::tempdir().unwrap();
    let d = bound_library();
    let root = canonical(d.path());
    let lists = Arc::new(AtomicUsize::new(0));
    let o = ServiceOptions {
        fs: Arc::new(CountingFs {
            root: root.clone(),
            lists: lists.clone(),
        }),
        ..opts()
    };
    let r = rig_with(app.path(), o);
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    assert_eq!(db_of(&r, id).as_deref(), Some("orders"));
    let scans = lists.load(Ordering::SeqCst);
    rebind_to_billing(d.path());
    r.hint(&root, &[".hacivat/workspace.toml"]);
    wait_until("the new binding", || {
        db_of(&r, id).as_deref() == Some("billing")
    })
    .await;
    r.svc.settled().await;
    let more = lists.load(Ordering::SeqCst) - scans;
    print_fact(&format!(
        "t10-fr3 h15 source=fake db=billing full_scans={more}"
    ));
    assert_eq!(more, 0, "the manifest change caused a full scan");
}

/// H15 through the poll source: the manifest is in its stat set.
#[tokio::test(flavor = "multi_thread")]
async fn h15_manifest_change_rebinds_on_a_polled_library() {
    let app = tempfile::tempdir().unwrap();
    let d = bound_library();
    let paths = LocalPaths::under(app.path());
    let sources = Sources::Custom(Arc::new(|| {
        PollEventSource::with_interval(Duration::from_millis(30))
            .map(|s| Box::new(s) as Box<dyn EventSource>)
    }));
    let svc =
        LibraryService::open_with(&paths, &tokio::runtime::Handle::current(), sources, opts());
    wait_until("the service to load", || {
        svc.snapshot().state == LibraryState::Watching
    })
    .await;
    let id = svc.attach(d.path().into(), None, false).await.unwrap();
    let db = |svc: &LibraryService| svc.snapshot().library(id).map(|l| l.db.as_str().to_owned());
    wait_until("the first binding", || {
        db(&svc).as_deref() == Some("orders")
    })
    .await;
    // Let a pass or two record the manifest's stamp first.
    tokio::time::sleep(Duration::from_millis(150)).await;
    rebind_to_billing(d.path());
    wait_until("the new binding", || db(&svc).as_deref() == Some("billing")).await;
    print_fact("t10-fr3 h15 source=poll db=billing");
}

/// G2's backstop: with no word from the watcher at all, the status refresh's backstop (60 s,
/// here short) re-reads the manifest.
#[tokio::test(flavor = "multi_thread")]
async fn a_manifest_change_nobody_reported_is_seen_at_the_backstop() {
    let app = tempfile::tempdir().unwrap();
    let d = bound_library();
    let r = rig_with(
        app.path(),
        ServiceOptions {
            git_backstop: Duration::from_millis(200),
            ..opts()
        },
    );
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    assert_eq!(db_of(&r, id).as_deref(), Some("orders"));
    rebind_to_billing(d.path());
    wait_until("the new binding", || {
        db_of(&r, id).as_deref() == Some("billing")
    })
    .await;
}

/// H16: a batch tagged with an older watcher's generation is ignored; the same batch with the
/// current generation is acted on (positive control).
#[tokio::test(flavor = "multi_thread")]
async fn h16_a_stale_watchers_batch_is_ignored() {
    let app = tempfile::tempdir().unwrap();
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.sql"), "SELECT 1;").unwrap();
    std::fs::write(d.path().join("b.sql"), "SELECT 2;").unwrap();
    let mut r = rig_with(app.path(), opts());
    ready(&r).await;
    let id = attach(&r, d.path()).await;
    let root = canonical(d.path());
    let (a, b) = (rel("a.sql"), rel("b.sql"));
    r.svc.read(id, &a, None).await.unwrap();
    // a.sql is really deleted, and announced.
    std::fs::remove_file(d.path().join("a.sql")).unwrap();
    r.hint(&root, &["a.sql"]);
    r.svc.settled().await;
    file_events(&mut r.rx);
    let renamed = vec![WatchEvent::Renamed {
        from: a.clone(),
        to: b.clone(),
    }];
    // A stale watcher's late word: "a.sql was renamed to b.sql".
    r.svc.inject_watch_batch(id, true, renamed.clone()).await;
    r.svc.settled().await;
    let stale = file_events(&mut r.rx);
    r.svc.inject_watch_batch(id, false, renamed).await;
    r.svc.settled().await;
    let current = file_events(&mut r.rx);
    assert!(
        !stale.iter().any(|e| is_renamed(e, &a, &b)),
        "a stale batch was acted on: {stale:?}"
    );
    assert!(
        current.iter().any(|e| is_renamed(e, &a, &b)),
        "positive control: {current:?}"
    );
}
