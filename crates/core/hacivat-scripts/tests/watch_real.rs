//! M4b Task 9: the watcher on the real OS backend (HV-SCRIPT-08; alg/28 §0.5, U-06..U-09).
//!
//! Every wait is a condition with a 10 s ceiling, never a sleep. An absence is asserted after a
//! sentinel file written later has been reported (events settle in order). These tests run on
//! the three CI runners: FSEvents here, inotify on ubuntu (U-06), `ReadDirectoryChangesW` on
//! windows (U-07).

use std::fs;
use std::path::{Path, PathBuf};
#[cfg(any(unix, windows))] // ## m4b-t20-adversarial (named line): the junction test
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hacivat_scripts::fileio::{self, ReadLimits};
#[cfg(any(unix, windows))] // ## m4b-t20-adversarial (named line)
use hacivat_scripts::scan::{Fs, Meta, RealFs};
use hacivat_scripts::watch::{
    Backend, EventSource, OsEventSource, PollEventSource, PollReason, WatchBatch, WatchEvent,
    WatchOpts, Watcher,
};
use hacivat_scripts::{LibraryId, LibraryRoot, RelPath, SaveText};
use tokio::sync::mpsc;

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

struct Lib {
    _dir: tempfile::TempDir,
    root: LibraryRoot,
}

impl Lib {
    fn new(files: &[(&str, &str)]) -> Lib {
        let dir = tempfile::tempdir().unwrap();
        let root = LibraryRoot {
            id: LibraryId::new(),
            path: dir.path().into(),
            canonical: dir.path().canonicalize().unwrap(),
        };
        let lib = Lib { _dir: dir, root };
        // ## m4b-cifix-2-fixture-age (review I2): every fixture file is an hour old, so none
        // is racily clean (R-T9-3's 2 s window) when a watcher starts. A late FSEvents history
        // hint on a fixture file then finds an equal, old stamp and reports nothing; before,
        // a test run alone could see `PossiblyChanged` for it
        // (`folder_move_is_seen_as_renames_for_known_contents`).
        let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3_600);
        for (name, text) in files {
            lib.write(name, text);
            fs::File::options()
                .write(true)
                .open(lib.path(name))
                .unwrap()
                .set_modified(an_hour_ago)
                .unwrap();
        }
        lib
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.canonical.join(name)
    }

    fn write(&self, name: &str, text: &str) {
        let p = self.path(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, text).unwrap();
    }
}

struct Real {
    w: Watcher,
    rt: tokio::runtime::Runtime,
    rx: mpsc::Receiver<WatchBatch>,
    seen: Vec<WatchEvent>,
    /// When each event of `seen` arrived (fix round 3, m-fr2-1).
    arrived: Vec<Instant>,
    sentinels: u32,
}

impl Real {
    fn os(lib: &Lib) -> Real {
        Self::with(
            lib,
            WatchOpts::default(),
            Box::new(OsEventSource::new().unwrap()),
        )
    }

    fn with(lib: &Lib, opts: WatchOpts, source: Box<dyn hacivat_scripts::EventSource>) -> Real {
        let (tx, rx) = mpsc::channel(256);
        let w = Watcher::start(lib.root.clone(), opts, tx, source).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        Real {
            w,
            rt,
            rx,
            seen: Vec::new(),
            arrived: Vec::new(),
            sentinels: 0,
        }
    }

    /// Waits (10 s ceiling) until the events seen so far satisfy `done`.
    fn wait_for(&mut self, what: &str, done: impl Fn(&[WatchEvent]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(&self.seen) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self
                .rt
                .block_on(async { tokio::time::timeout(left, self.rx.recv()).await })
            {
                Ok(Some(batch)) => {
                    self.arrived
                        .extend(std::iter::repeat_n(Instant::now(), batch.len()));
                    self.seen.extend(batch);
                }
                Ok(None) => panic!("{what}: the watcher hung up; saw {:?}", self.seen),
                Err(_) => panic!("{what}: not within 10 s; saw {:?}", self.seen),
            }
        }
    }

    /// Writes a new sentinel script and waits until it is reported: everything that happened
    /// before it has been reported too. A rescan still pending when it was written may learn
    /// it without reporting it; after every such rescan the sentinel is written again, longer,
    /// so the next look reports it as changed.
    fn sentinel(&mut self, lib: &Lib) -> RelPath {
        self.sentinels += 1;
        let name = format!("zz-sentinel-{}.sql", self.sentinels);
        let r = rel(&name);
        let mut body = String::from("SELECT 0;");
        lib.write(&name, &body);
        let mut from = self.seen.len();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let tail = &self.seen[from..];
            if tail.iter().any(|e| names(e, &r)) {
                return r;
            }
            if let Some(k) = tail.iter().rposition(|e| *e == WatchEvent::Rescan) {
                from += k + 1;
                body.push_str(" --");
                lib.write(&name, &body);
                continue;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self
                .rt
                .block_on(async { tokio::time::timeout(left, self.rx.recv()).await })
            {
                Ok(Some(batch)) => {
                    self.arrived
                        .extend(std::iter::repeat_n(Instant::now(), batch.len()));
                    self.seen.extend(batch);
                }
                Ok(None) => panic!("{name}: the watcher hung up; saw {:?}", self.seen),
                Err(_) => panic!("{name}: not within 10 s; saw {:?}", self.seen),
            }
        }
    }

    /// The events seen, without the sentinels, and without a `PossiblyChanged` that repeats a
    /// file reported `Created` or `Changed` less than R-T9-3's window (plus two debounce
    /// windows of delivery) before it (fix round 3, m-fr2-1: never "any later time").
    ///
    /// Fix round 2 (root cause of a 1-in-60 failure of `external_in_place_write_is_seen`):
    /// FSEvents can deliver one write in two debounced batches (the write, then `sync_all`,
    /// more than a window apart under load). The second hint arrives after the first change
    /// settled, while the file's time is still within R-T9-3's 2 s window and no tab has it
    /// open, so the ruled answer is `PossiblyChanged`. It names a file the test already saw
    /// reported; it is not a second change.
    fn reported(&self) -> Vec<WatchEvent> {
        use hacivat_scripts::watch::{DEBOUNCE, RACY_WINDOW};
        let window = RACY_WINDOW + DEBOUNCE * 2;
        let mut out: Vec<WatchEvent> = Vec::new();
        for (i, e) in self.seen.iter().enumerate() {
            if matches!(e, WatchEvent::Created(r) | WatchEvent::Changed(r) | WatchEvent::Unsettled(r) | WatchEvent::PossiblyChanged(r) if r.as_str().starts_with("zz-sentinel-"))
            {
                continue;
            }
            if let WatchEvent::PossiblyChanged(r) = e
                && self.seen[..i]
                    .iter()
                    .zip(&self.arrived[..i])
                    .any(|(x, at)| {
                        matches!(x, WatchEvent::Created(y) | WatchEvent::Changed(y) if y == r)
                            && self.arrived[i].saturating_duration_since(*at) < window
                    })
            {
                continue;
            }
            out.push(e.clone());
        }
        out
    }
}

/// Whether `e` reports the sentinel `r` (created, or changed after a rescan learned it).
fn names(e: &WatchEvent, r: &RelPath) -> bool {
    matches!(e, WatchEvent::Created(x) | WatchEvent::Changed(x) | WatchEvent::Unsettled(x) if x == r)
}

#[test]
fn external_in_place_write_is_seen() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut w = Real::os(&lib);
    // The in-place write of VS Code and most editors (U-08: a real VS Code save is the gate's).
    let mut f = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(lib.path("a.sql"))
        .unwrap();
    std::io::Write::write_all(&mut f, b"SELECT 1; SELECT 2;").unwrap();
    f.sync_all().unwrap();
    drop(f);
    w.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn external_temp_then_rename_write_is_seen() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut w = Real::os(&lib);
    // Atomic save (JetBrains "safe write", Sublime, many tools): temp, then rename over.
    lib.write("a.sql.tmp", "SELECT 22;");
    fs::rename(lib.path("a.sql.tmp"), lib.path("a.sql")).unwrap();
    w.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn external_backup_rename_save_is_one_change() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut w = Real::os(&lib);
    // vim `backupcopy=auto`: probe `4913`, rename the original away, write, drop the backup.
    lib.write("4913", "");
    fs::remove_file(lib.path("4913")).unwrap();
    fs::rename(lib.path("a.sql"), lib.path("a.sql~")).unwrap();
    lib.write("a.sql", "SELECT 333;");
    fs::remove_file(lib.path("a.sql~")).unwrap();
    w.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn folder_delete_is_seen() {
    let lib = Lib::new(&[
        ("a.sql", "x"),
        ("reports/c.sql", "c"),
        ("reports/r.md", "r"),
    ]);
    let mut w = Real::os(&lib);
    fs::remove_dir_all(lib.path("reports")).unwrap();
    let want = [
        WatchEvent::Removed(rel("reports/c.sql")),
        WatchEvent::Removed(rel("reports/r.md")),
    ];
    w.wait_for("removed", |e| want.iter().all(|x| e.contains(x)));
    w.sentinel(&lib);
    assert_eq!(w.reported(), want.to_vec());
}

#[test]
fn folder_move_is_seen_as_renames_for_known_contents() {
    let lib = Lib::new(&[("reports/c.sql", "c"), ("reports/d.sql", "dd")]);
    let mut w = Real::os(&lib);
    // A tab had `c.sql` open: its stamp is known, so its move is a rename.
    let stamp = fileio::read(
        &lib.root,
        &rel("reports/c.sql"),
        None,
        ReadLimits::default(),
    )
    .unwrap()
    .stamp;
    w.w.remember(rel("reports/c.sql"), stamp);
    // Let the remembered stamp reach the watcher's thread before the move.
    w.sentinel(&lib);
    fs::rename(lib.path("reports"), lib.path("archive")).unwrap();
    let want = [
        WatchEvent::Renamed {
            from: rel("reports/c.sql"),
            to: rel("archive/c.sql"),
        },
        WatchEvent::Removed(rel("reports/d.sql")),
        WatchEvent::Created(rel("archive/d.sql")),
    ];
    w.wait_for("moved", |e| want.iter().all(|x| e.contains(x)));
    w.sentinel(&lib);
    assert_eq!(w.reported(), want.to_vec());
}

#[test]
fn own_save_is_not_reported() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut w = Real::os(&lib);
    let echo = w.w.echo();
    let read = fileio::read(&lib.root, &rel("a.sql"), None, ReadLimits::default()).unwrap();
    fileio::save_with_echo(
        &lib.root,
        &rel("a.sql"),
        None,
        SaveText {
            text: "SELECT 42;".into(),
            meta: read.meta,
        },
        Some(read.stamp),
        &mut |s| echo.record(rel("a.sql"), *s),
    )
    .unwrap();
    // A new file saved by Hacivat is its own write too.
    fileio::save_with_echo(
        &lib.root,
        &rel("new.sql"),
        None,
        SaveText {
            text: "SELECT 7;".into(),
            meta: read.meta,
        },
        None,
        &mut |s| echo.record(rel("new.sql"), *s),
    )
    .unwrap();
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![]);
    // A foreign edit afterwards is reported.
    lib.write("a.sql", "SELECT 4242;");
    w.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
}

/// Records every path the reconciler asks the file system about.
#[cfg(any(unix, windows))] // ## m4b-t20-adversarial (named line)
#[derive(Default)]
struct Counting {
    paths: Mutex<Vec<PathBuf>>,
}

#[cfg(any(unix, windows))] // ## m4b-t20-adversarial (named line)
impl Counting {
    fn asked_under(&self, dir: &Path) -> Vec<PathBuf> {
        let paths = self.paths.lock().unwrap();
        paths
            .iter()
            .filter(|p| p.starts_with(dir) && *p != dir)
            .cloned()
            .collect()
    }
    fn asked(&self, p: &Path) -> bool {
        self.paths.lock().unwrap().iter().any(|x| x == p)
    }
}

#[cfg(any(unix, windows))] // ## m4b-t20-adversarial (named line)
impl Fs for Counting {
    fn read_dir(&self, dir: &Path, cap: usize) -> std::io::Result<Vec<std::ffi::OsString>> {
        self.paths.lock().unwrap().push(dir.into());
        RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> std::io::Result<Meta> {
        self.paths.lock().unwrap().push(path.into());
        RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        self.paths.lock().unwrap().push(path.into());
        RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> std::io::Result<PathBuf> {
        self.paths.lock().unwrap().push(link.into());
        RealFs.read_link(link)
    }
}

#[cfg(unix)]
#[test]
fn dir_symlink_created_after_watch_is_not_walked() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let big = tempfile::tempdir().unwrap();
    for i in 0..10_000 {
        fs::write(big.path().join(format!("f{i:05}.sql")), "SELECT 1;").unwrap();
    }
    let counting = Arc::new(Counting::default());
    let opts = WatchOpts {
        fs: counting.clone(),
        ..WatchOpts::default()
    };
    let mut w = Real::with(&lib, opts, Box::new(OsEventSource::new().unwrap()));
    std::os::unix::fs::symlink(big.path(), lib.path("outside")).unwrap();
    std::os::unix::fs::symlink("/", lib.path("root")).unwrap();
    // Writes inside the link targets, after the links exist.
    fs::write(big.path().join("f00001.sql"), "SELECT 2;").unwrap();
    fs::write(big.path().join("new.sql"), "SELECT 3;").unwrap();
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![]);
    // Fix round 1 (m7): on Linux, the backend's own watches are looked at too.
    //
    // ## m4b-cifix-3-link-watch (CI 45068616161): by what they watch, not by how many there
    // are. The process's inotify watches include those of the tests running beside this one,
    // so a count moved with them (CI saw 5 before, 4 after: another test's watcher ended).
    // A walk through either link would watch the link's target folder (`big`, or `/`), so no
    // watch of this process may be on one of them; and this watcher's own root is watched
    // (the control that the watches are read at all).
    #[cfg(target_os = "linux")]
    {
        let watched = own_inotify_watched();
        let own = inode_of(&lib.root.canonical, false).unwrap();
        // The links themselves, their targets, and the folders one step below `/` (a walk
        // into `root/usr` resolves the link and watches `/usr`).
        let mut through: Vec<(PathBuf, (u64, u64))> = Vec::new();
        for (p, follow) in [
            (lib.path("outside"), false),
            (lib.path("root"), false),
            (big.path().to_path_buf(), true),
            (PathBuf::from("/"), true),
        ] {
            through.extend(inode_of(&p, follow).map(|i| (p, i)));
        }
        for e in fs::read_dir("/").unwrap().flatten() {
            if fs::metadata(e.path()).is_ok_and(|m| m.is_dir()) {
                through.extend(inode_of(&e.path(), true).map(|i| (e.path(), i)));
            }
        }
        // ## m4b-t20-adversarial (named lines, cifix-3 review m1): matched by inode alone,
        // like the control, so a wrong device encoding (or btrfs's per-subvolume `st_dev`)
        // can never make these checks pass vacuously; an inode shared across devices can only
        // fail the test, the safe direction.
        let hits: Vec<&PathBuf> = through
            .iter()
            .filter(|(_, i)| watched.iter().any(|w| w.1 == i.1))
            .map(|(p, _)| p)
            .collect();
        // The control matches the inode alone (btrfs reports a subvolume's own `st_dev`).
        let own_seen = watched.iter().any(|w| w.1 == own.1);
        println!(
            "HV-FACT t09 link_watches total={} through_link={} own_root={own_seen}",
            watched.len(),
            hits.len(),
        );
        assert!(own_seen, "the root's own watch is read");
        assert_eq!(
            hits,
            Vec::<&PathBuf>::new(),
            "a watch was added through a link"
        );
    }
    // The scan that settled the sentinel listed the root, so it met both links, and went
    // through neither.
    assert!(counting.asked(&lib.path("outside")));
    assert_eq!(
        counting.asked_under(&lib.path("outside")),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        counting.asked_under(&lib.path("root")),
        Vec::<PathBuf>::new()
    );
}

// ## m4b-t20-adversarial (named exception, T9 fr1 carry: the Windows junction variant)
/// A junction made inside the library after the watch started (`mklink /J`, which needs no
/// privilege, unlike a symbolic link) is never walked: the reconciler's scan meets it and goes
/// through it nowhere, and nothing written behind it is reported.
#[cfg(windows)]
#[test]
fn junction_created_after_watch_is_not_walked() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let big = tempfile::tempdir().unwrap();
    for i in 0..1_000 {
        fs::write(big.path().join(format!("f{i:04}.sql")), "SELECT 1;").unwrap();
    }
    let counting = Arc::new(Counting::default());
    let opts = WatchOpts {
        fs: counting.clone(),
        ..WatchOpts::default()
    };
    let mut w = Real::with(&lib, opts, Box::new(OsEventSource::new().unwrap()));
    let made = std::process::Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(lib.path("outside"))
        .arg(big.path())
        .output()
        .unwrap();
    assert!(made.status.success(), "mklink /J: {made:?}");
    fs::write(big.path().join("f0001.sql"), "SELECT 2;").unwrap();
    fs::write(big.path().join("new.sql"), "SELECT 3;").unwrap();
    w.sentinel(&lib);
    assert_eq!(w.reported(), vec![]);
    assert!(counting.asked(&lib.path("outside")));
    assert_eq!(
        counting.asked_under(&lib.path("outside")),
        Vec::<PathBuf>::new()
    );
    let _ = std::io::Write::write_all(
        &mut std::io::stdout().lock(),
        b"HV-FACT t20 junction_walked=false\n",
    );
}
// ## m4b-t20-adversarial

#[test]
fn polling_fallback_sees_a_change() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;"), ("sub/b.md", "notes")]);
    let opts = WatchOpts {
        backend: Backend::Poll(PollReason::NetworkFs),
        ..WatchOpts::default()
    };
    let mut w = Real::with(&lib, opts, Box::new(PollEventSource::new().unwrap()));
    assert_eq!(w.w.backend(), Backend::Poll(PollReason::NetworkFs));
    lib.write("a.sql", "SELECT 1; SELECT 2;");
    fs::remove_file(lib.path("sub/b.md")).unwrap();
    let want = [
        WatchEvent::Changed(rel("a.sql")),
        WatchEvent::Removed(rel("sub/b.md")),
    ];
    w.wait_for("polled", |e| want.iter().all(|x| e.contains(x)));
}

#[test]
fn dropping_the_watcher_stops_its_threads() {
    for source in [
        Box::new(OsEventSource::new().unwrap()) as Box<dyn hacivat_scripts::EventSource>,
        Box::new(PollEventSource::new().unwrap()),
    ] {
        let lib = Lib::new(&[("a.sql", "x")]);
        let mut w = Real::with(&lib, WatchOpts::default(), source);
        w.sentinel(&lib);
        let probe = w.w.probe();
        let started = Instant::now();
        drop(w);
        // The drop is bounded (fix round 1, I6: at most `DROP_JOIN_BOUND`, then the thread is
        // abandoned, never waited for).
        assert!(started.elapsed() < Duration::from_secs(5));
        // Fix round 3 (root cause of 11 of 40 failures under parallel load): under load the
        // thread's teardown (unwatch, the source's drop joining notify's debouncer thread) can
        // outlast that bound, so the thread ends shortly after the drop returns. Condition
        // wait, 10 s ceiling: its own thread ended, and the source's debouncer thread (which
        // holds the hint queue through the sink) was joined in the source's drop.
        let deadline = Instant::now() + Duration::from_secs(10);
        while !probe.released() {
            assert!(
                Instant::now() < deadline,
                "the watcher's threads never ended"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn burst_of_creates_loses_nothing() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut w = Real::os(&lib);
    let n = 3_000u32;
    for i in 0..n {
        lib.write(&format!("many/f{i:04}.sql"), "SELECT 1;");
    }
    // Either one rescan (the burst rule) or every file one by one; never a lost file. Which
    // one depends on how the OS spread the events over windows.
    let rescans = |e: &[WatchEvent]| e.contains(&WatchEvent::Rescan);
    let created = |e: &[WatchEvent]| {
        e.iter()
            .filter(|x| matches!(x, WatchEvent::Created(r) if r.as_str().starts_with("many/")))
            .count()
    };
    w.wait_for("burst", |e| rescans(e) || created(e) == n as usize);
    w.sentinel(&lib);
    println!(
        "HV-FACT t09 burst_creates={n} rescans={} per_file_events={}",
        w.seen.iter().filter(|e| **e == WatchEvent::Rescan).count(),
        created(&w.seen)
    );
}

#[test]
fn a_hint_storm_from_one_file_stays_one_change() {
    // Many writes of one file inside a window: one change, never one per write.
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut w = Real::os(&lib);
    let mut body = String::from("x");
    for _ in 0..50 {
        body.push('y');
        lib.write("a.sql", &body);
    }
    w.wait_for("changed", |e| {
        e.iter()
            .any(|x| matches!(x, WatchEvent::Changed(_) | WatchEvent::Unsettled(_)))
    });
    w.sentinel(&lib);
    let changes = w.reported();
    assert!(changes.len() <= 3, "{changes:?}");
}

/// U-06: how many inotify watches a 5,000-folder library takes (CI ubuntu prints the fact).
#[cfg(target_os = "linux")]
#[test]
fn inotify_watches_for_5000_folders_are_counted() {
    let lib = Lib::new(&[]);
    for i in 0..5_000 {
        fs::create_dir_all(lib.path(&format!("d{:02}/f{i:04}", i % 50))).unwrap();
    }
    let before = own_inotify_watches();
    let w = Real::os(&lib);
    let after = own_inotify_watches();
    let budget = hacivat_scripts::watch::inotify_budget();
    println!(
        "HV-FACT t09 inotify_dirs=5051 watches_added={} budget_free={budget:?} backend={:?}",
        after.saturating_sub(before),
        hacivat_scripts::watch::backend_for(&lib.root.canonical)
    );
    assert!(budget.is_some());
    assert!(after > before);
    drop(w);
}

#[cfg(target_os = "linux")]
fn own_inotify_watches() -> usize {
    let mut n = 0;
    for fd in fs::read_dir("/proc/self/fdinfo").unwrap().flatten() {
        if let Ok(text) = fs::read_to_string(fd.path()) {
            n += text
                .lines()
                .filter(|l| l.starts_with("inotify wd:"))
                .count();
        }
    }
    n
}

// ## m4b-cifix-3-link-watch
/// What every inotify watch of this process is on, as `(device, inode)` in the kernel's
/// encoding (`/proc/self/fdinfo`: `inotify wd:… ino:<hex> sdev:<hex> …`).
#[cfg(target_os = "linux")]
fn own_inotify_watched() -> std::collections::HashSet<(u64, u64)> {
    let mut out = std::collections::HashSet::new();
    for fd in fs::read_dir("/proc/self/fdinfo").unwrap().flatten() {
        let Ok(text) = fs::read_to_string(fd.path()) else {
            continue;
        };
        for line in text.lines().filter(|l| l.starts_with("inotify wd:")) {
            let field = |key: &str| {
                line.split_whitespace()
                    .find_map(|t| t.strip_prefix(key))
                    .and_then(|v| u64::from_str_radix(v, 16).ok())
            };
            if let (Some(ino), Some(dev)) = (field("ino:"), field("sdev:")) {
                out.insert((dev, ino));
            }
        }
    }
    out
}

/// `path`'s `(device, inode)` in the kernel's encoding (`MKDEV`: major << 20 | minor), as
/// fdinfo prints them; `follow` resolves a link first.
#[cfg(target_os = "linux")]
fn inode_of(path: &Path, follow: bool) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = if follow {
        fs::metadata(path)
    } else {
        fs::symlink_metadata(path)
    }
    .ok()?;
    // glibc's `gnu_dev_major`/`gnu_dev_minor` of the user-space `st_dev`.
    let dev = m.dev();
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & 0xffff_f000);
    let minor = (dev & 0xff) | ((dev >> 12) & 0xffff_ff00);
    Some(((major << 20) | minor, m.ino()))
}
// ## m4b-cifix-3-link-watch

/// Idle cost (review probe): CPU time of this process over 5 s with ten idle watchers.
#[cfg(unix)]
#[test]
#[ignore = "perf"]
fn idle_cpu_is_recorded() {
    if std::env::var("HACIVAT_PERF").as_deref() != Ok("1") {
        return;
    }
    let libs: Vec<Lib> = (0..10).map(|_| Lib::new(&[("a.sql", "x")])).collect();
    let watchers: Vec<Real> = libs.iter().map(Real::os).collect();
    let before = cpu_time();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(5) {
        std::thread::park_timeout(Duration::from_millis(500));
    }
    let after = cpu_time();
    println!(
        "HV-FACT t09 idle_watchers=10 wall_ms={} cpu_ms={}",
        started.elapsed().as_millis(),
        after.saturating_sub(before).as_millis()
    );
    drop(watchers);
}

#[cfg(unix)]
/// This process's user + system CPU time, from `ps` (tests may run tools; product code may
/// not).
fn cpu_time() -> Duration {
    let out = std::process::Command::new("ps")
        .args(["-o", "time=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    // `[[dd-]hh:]mm:ss.cc`
    let t = text.trim();
    let mut secs = 0f64;
    for part in t.split(':') {
        secs = secs * 60.0 + part.trim().parse::<f64>().unwrap_or(0.0);
    }
    Duration::from_secs_f64(secs)
}

// ---------------------------------------------------------------------------------------------
// ## m4b-t09-watcher fix round 1 (review I5, I7, m6, m7).

/// I5 / R-T9-4: a per-folder source (Linux's default; on macOS FSEvents with non-recursive
/// watches) sees a folder made after the start, files in it, and a folder removed.
#[test]
fn per_directory_source_sees_new_and_removed_folders() {
    let lib = Lib::new(&[("a/x.sql", "x")]);
    let mut w = Real::with(
        &lib,
        WatchOpts::default(),
        Box::new(OsEventSource::per_directory().unwrap()),
    );
    lib.write("b/c/y.sql", "y");
    w.wait_for("created", |e| {
        e.contains(&WatchEvent::Created(rel("b/c/y.sql")))
    });
    // `b/c` is watched now: an in-place edit there is seen without any folder event.
    lib.write("b/c/y.sql", "yyy");
    w.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("b/c/y.sql")))
    });
    fs::remove_dir_all(lib.path("a")).unwrap();
    w.wait_for("removed", |e| {
        e.contains(&WatchEvent::Removed(rel("a/x.sql")))
    });
}

/// m6: a caller's poll choice (R-T9-1's per-library "poll") is kept by `open`.
#[test]
fn open_keeps_a_chosen_poll_backend() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (tx, _rx) = mpsc::channel(8);
    let opts = WatchOpts {
        backend: Backend::Poll(PollReason::NetworkFs),
        ..WatchOpts::default()
    };
    let w = Watcher::open(lib.root.clone(), opts, tx).unwrap();
    assert_eq!(w.backend(), Backend::Poll(PollReason::NetworkFs));
}

/// Runs the git command line on a fixture repository this test made (R-T1a-1: tests only),
/// with no system or global configuration and no hooks.
fn git_cli(dir: &Path, args: &[&str]) {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "core.autocrlf=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// alg/28 HV-SCRIPT-08 named test: a branch switch that rewrites 2,000 scripts is one rescan,
/// never 2,000 events (the burst rule over the whole checkout).
#[test]
fn git_checkout_2000_is_one_rescan() {
    let lib = Lib::new(&[]);
    for i in 0..2_000 {
        lib.write(&format!("many/f{i:04}.sql"), "SELECT 1;");
    }
    git_cli(&lib.root.canonical, &["init", "-q"]);
    git_cli(&lib.root.canonical, &["add", "-A"]);
    git_cli(&lib.root.canonical, &["commit", "-q", "-m", "one"]);
    git_cli(&lib.root.canonical, &["checkout", "-q", "-b", "topic"]);
    for i in 0..2_000 {
        lib.write(&format!("many/f{i:04}.sql"), "SELECT 2; -- topic");
    }
    git_cli(&lib.root.canonical, &["commit", "-q", "-a", "-m", "two"]);
    git_cli(&lib.root.canonical, &["checkout", "-q", "main"]);
    let mut w = Real::os(&lib);
    git_cli(&lib.root.canonical, &["checkout", "-q", "topic"]);
    w.wait_for("rescan", |e| e.contains(&WatchEvent::Rescan));
    w.sentinel(&lib);
    let per_file = w
        .reported()
        .iter()
        .filter(|e| !matches!(e, WatchEvent::Rescan | WatchEvent::RepositoryChanged))
        .count();
    let rescans = w.seen.iter().filter(|e| **e == WatchEvent::Rescan).count();
    println!("HV-FACT t09 git_checkout_files=2000 rescans={rescans} per_file_events={per_file}");
    assert_eq!(per_file, 0, "{:?}", &w.reported()[..per_file.min(10)]);
}

/// U-07: a hint by a file's 8.3 short name is reconciled to the listed long name.
#[cfg(windows)]
#[test]
fn short_name_event_is_reconciled() {
    use hacivat_scripts::watch::{RawHint, Reconciler};
    use std::os::windows::process::CommandExt;
    let lib = Lib::new(&[("longname_script.sql", "SELECT 1;")]);
    let long = lib._dir.path().join("longname_script.sql");
    // The short name through cmd's `%~s` modifier (this crate makes no Win32 call).
    let out = std::process::Command::new("cmd")
        .raw_arg(format!(
            "/C for %I in (\"{}\") do @echo %~sI",
            long.display()
        ))
        .output()
        .unwrap();
    let short = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let Some(short_name) = Path::new(&short).file_name().map(|n| n.to_owned()) else {
        panic!("no short name from cmd: {short:?}");
    };
    if short_name == "longname_script.sql" {
        println!("HV-FACT t09 short_names=off");
        return;
    }
    println!("HV-FACT t09 short_names=on short={short_name:?}");
    let mut r = Reconciler::new(lib.root.clone(), WatchOpts::default());
    lib.write("longname_script.sql", "SELECT 1; SELECT 2;");
    let at = Instant::now();
    r.push(RawHint::Path(lib.root.canonical.join(&short_name)), at);
    let mut out = r.tick(at);
    let mut now = at;
    while let Some(due) = r.next_due() {
        now = due.max(now);
        out.extend(r.tick(now));
        if out.len() > 4 {
            break;
        }
    }
    assert_eq!(out, vec![WatchEvent::Changed(rel("longname_script.sql"))]);
}

// ## m4b-gatefix-1-idle (E12 watcher idle CPU)
// T9 fr2's `debouncer_root_bookkeeping_for_5000_folders_is_recorded` measured
// `notify-debouncer-full`'s own root list (m-fr1-3); the source no longer uses that debouncer,
// so the residual and its record are gone.

/// The real OS source wakes nothing while nothing happens: over a quiet window (one in which
/// the backend sent nothing) its debounce thread did not wake once. The old
/// `notify-debouncer-full` loop woke every 37.5 ms (16 times per window). A window in which the
/// OS reported something (Spotlight, a late initial event) proves nothing and is retried, up to
/// 10 windows; then the test fails.
#[test]
fn an_idle_os_source_wakes_nothing() {
    let lib = Lib::new(&[("a.sql", "SELECT 1"), ("d/b.sql", "SELECT 2")]);
    let mut src = OsEventSource::new().unwrap();
    src.watch(&lib.root.canonical, Box::new(|_| {})).unwrap();
    let window = Duration::from_millis(600);
    for _ in 0..10 {
        let before = src.debounce_counts().unwrap();
        // An observation of absence, not a wait for a condition: load only delays wakes.
        std::thread::sleep(window);
        let after = src.debounce_counts().unwrap();
        if after.events != before.events {
            continue;
        }
        assert_eq!(after.wakes, before.wakes, "woke while idle: {before:?} -> {after:?}");
        println!("HV-FACT gatefix1 idle_os_source window_ms=600 wakes=0");
        return;
    }
    panic!("no quiet window in 10: the OS kept reporting events");
}
// ## m4b-gatefix-1-idle
