//! M4b Task 9: the watcher's deterministic logic (HV-SCRIPT-08; R-M4b-14, -16; alg/28 C3–C6).
//!
//! The reconciler runs on a real temp folder with an injected clock (no OS events, no sleeps);
//! the watcher end to end runs on `testing::FakeEventSource` with condition waits (≤ 10 s).

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hacivat_scripts::fileio::{self, ReadLimits};
use hacivat_scripts::scan::{Fs, Limits, Meta, RealFs};
use hacivat_scripts::testing::{FakeEventSource, FakeHandle};
use hacivat_scripts::watch::limits::{
    budget_from, count_dirs, fs_kind_from_mountinfo, inotify_budget_at,
};
use hacivat_scripts::watch::{
    Backend, Debouncer, EchoTable, EventSource, Flushed, FsKind, HintQueue, LostReason, PollReason,
    RawHint, Reconciler, WatchBatch, WatchError, WatchEvent, WatchOpts, Watcher, choose_backend,
};
use hacivat_scripts::{DiskStamp, LibraryId, LibraryRoot, RelPath, SaveText};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------------------------
// Helpers.

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn t0() -> Instant {
    static T0: OnceLock<Instant> = OnceLock::new();
    *T0.get_or_init(Instant::now)
}

fn rel(s: &str) -> RelPath {
    RelPath::parse(s).unwrap()
}

fn raw_modify(name: &str) -> RawHint {
    RawHint::Path(Path::new("/lib").join(name))
}

fn stamp(len: u64, hash: u128) -> DiskStamp {
    DiskStamp {
        len,
        mtime: UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        hash,
    }
}

const W: Duration = Duration::from_millis(150);

/// A library in a temp folder.
struct Lib {
    dir: tempfile::TempDir,
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
        let lib = Lib { dir, root };
        for (name, text) in files {
            lib.write(name, text);
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

    fn hint(&self, name: &str) -> RawHint {
        RawHint::Path(self.path(name))
    }

    fn rec(&self) -> Reconciler {
        Reconciler::new(self.root.clone(), WatchOpts::default())
    }

    fn rec_with(&self, opts: WatchOpts) -> Reconciler {
        Reconciler::new(self.root.clone(), opts)
    }

    /// The stamp a tab reading `name` gets.
    fn read_stamp(&self, name: &str) -> DiskStamp {
        fileio::read(&self.root, &rel(name), None, ReadLimits::default())
            .unwrap()
            .stamp
    }
}

/// Pushes `hints` at `at`, then ticks the simulated clock through every due time.
fn run(r: &mut Reconciler, hints: Vec<RawHint>, at: Instant) -> Vec<WatchEvent> {
    for h in hints {
        r.push(h, at);
    }
    drain(r, at)
}

/// Ticks at `at` and then at every time the reconciler asks for, until nothing is due.
fn drain(r: &mut Reconciler, at: Instant) -> Vec<WatchEvent> {
    let mut out = r.tick(at);
    let mut now = at;
    for _ in 0..64 {
        let Some(due) = r.next_due() else {
            return out;
        };
        now = due.max(now);
        out.extend(r.tick(now));
    }
    panic!("the reconciler never settled: {out:?}");
}

/// Waits (10 s ceiling) for `done`, looking every few milliseconds.
fn poll_until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "{what}: not within 10 s");
        std::thread::sleep(ms(5));
    }
}

fn text(s: &str) -> SaveText {
    SaveText {
        text: s.into(),
        meta: Default::default(),
    }
}

// ---------------------------------------------------------------------------------------------
// Debounce, bursts and queues (R-M4b-14; HV-SCRIPT-08 steps 3, 9).

#[test]
fn burst_becomes_one_rescan() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..1_001 {
        d.push(raw_modify(&format!("f{i}.sql")), t0());
    }
    assert_eq!(d.flush(t0() + ms(151)), vec![Flushed::Rescan]);
}

#[test]
fn a_thousand_hints_are_not_a_burst_and_windows_count_apart() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..1_000 {
        d.push(raw_modify(&format!("f{i}.sql")), t0());
    }
    let out = d.flush(t0() + ms(10));
    assert!(
        matches!(&out[..], [Flushed::Paths(p)] if p.len() == 1_000),
        "{out:?}"
    );
    // 600 + 600 inside one window, flushed in between: a burst.
    let mut d = Debouncer::new(ms(150));
    for i in 0..600 {
        d.push(raw_modify(&format!("a{i}.sql")), t0());
    }
    let _ = d.flush(t0() + ms(50));
    for i in 0..600 {
        d.push(raw_modify(&format!("b{i}.sql")), t0() + ms(100));
    }
    assert_eq!(d.flush(t0() + ms(120)), vec![Flushed::Rescan]);
    // 600 now and 600 two windows later: two ordinary windows.
    let mut d = Debouncer::new(ms(150));
    for i in 0..600 {
        d.push(raw_modify(&format!("a{i}.sql")), t0());
    }
    let _ = d.flush(t0() + ms(200));
    for i in 0..600 {
        d.push(raw_modify(&format!("b{i}.sql")), t0() + ms(300));
    }
    assert!(matches!(&d.flush(t0() + ms(310))[..], [Flushed::Paths(p)] if p.len() == 600));
}

#[test]
fn a_burst_rescans_once_per_window_and_lost_is_never_swallowed() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..1_001 {
        d.push(raw_modify(&format!("f{i}.sql")), t0());
    }
    d.push(RawHint::Lost, t0());
    assert_eq!(d.flush(t0()), vec![Flushed::Lost, Flushed::Rescan]);
    // Later hints of the same window are looked at one by one, not rescanned again.
    d.push(raw_modify("late.sql"), t0() + ms(20));
    assert_eq!(
        d.flush(t0() + ms(30)),
        vec![Flushed::Paths(vec!["/lib/late.sql".into()])]
    );
}

#[test]
fn hint_channel_overflow_is_one_rescan() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let q = HintQueue::new(10_000);
    // The consumer is paused: nobody takes while 10,001 hints arrive one by one.
    for i in 0..10_001 {
        q.push(vec![lib.hint(&format!("f{i}.sql"))]);
    }
    q.push(vec![RawHint::Lost]);
    let taken = q.take(Some(Instant::now())).unwrap();
    assert!(taken.overflowed);
    assert_eq!(
        taken.hints,
        vec![RawHint::Lost],
        "an overflow keeps only `Lost`"
    );
    assert_eq!(q.queued(), 0);
    // What the watcher does with an overflow: one rescan.
    let mut r = lib.rec();
    let events = run(
        &mut r,
        vec![RawHint::Rescan(lib.root.canonical.clone())],
        t0(),
    );
    assert_eq!(events, vec![WatchEvent::Rescan]);
    // And the queue works again afterwards.
    q.push(vec![lib.hint("a.sql")]);
    let again = q.take(Some(Instant::now())).unwrap();
    assert!(!again.overflowed);
    assert_eq!(again.hints, vec![lib.hint("a.sql")]);
}

#[test]
fn a_stopped_queue_wakes_its_taker() {
    let q = Arc::new(HintQueue::new(16));
    let taker = {
        let q = q.clone();
        std::thread::spawn(move || q.take(None))
    };
    q.stop();
    assert_eq!(taker.join().unwrap(), None);
    q.push(vec![raw_modify("a.sql")]);
    assert_eq!(q.queued(), 0, "pushes after stop are dropped");
}

// ---------------------------------------------------------------------------------------------
// Echo (HV-SCRIPT-08 step 6).

#[test]
fn own_write_is_an_echo() {
    let mut e = EchoTable::default();
    e.record(rel("a.sql"), stamp(3, 7));
    assert!(e.is_echo(&rel("a.sql"), &stamp(3, 7)));
    assert!(!e.is_echo(&rel("a.sql"), &stamp(3, 8)));
    assert!(!e.is_echo(&rel("b.sql"), &stamp(3, 7)));
}

#[test]
fn echo_expires_after_5s() {
    let mut e = EchoTable::default();
    e.record_at(rel("a.sql"), stamp(3, 7), t0());
    assert!(e.is_echo_at(&rel("a.sql"), &stamp(3, 7), t0() + ms(4_999)));
    assert!(!e.is_echo_at(&rel("a.sql"), &stamp(3, 7), t0() + ms(5_000)));
    e.expire(t0() + ms(5_000));
    assert!(e.is_empty());
}

#[test]
fn two_quick_saves_both_stay_echoes() {
    let mut e = EchoTable::default();
    e.record_at(rel("a.sql"), stamp(3, 1), t0());
    e.record_at(rel("a.sql"), stamp(4, 2), t0() + ms(100));
    assert!(e.is_echo_at(&rel("a.sql"), &stamp(3, 1), t0() + ms(200)));
    assert!(e.is_echo_at(&rel("a.sql"), &stamp(4, 2), t0() + ms(200)));
}

#[test]
fn own_save_through_fileio_is_not_reported_but_a_later_edit_is() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    let echo = r.echo();
    let before = lib.read_stamp("a.sql");
    fileio::save_with_echo(
        &lib.root,
        &rel("a.sql"),
        None,
        text("SELECT 22;"),
        Some(before),
        &mut |s| echo.record(rel("a.sql"), *s),
    )
    .unwrap();
    // The save's events: the temp (a dot name) and the file.
    let events = run(
        &mut r,
        vec![
            lib.hint(".a.sql.hv-0123456789abcdef0123456789abcdef.tmp"),
            lib.hint("a.sql"),
        ],
        t0(),
    );
    assert_eq!(events, vec![]);
    lib.write("a.sql", "SELECT 333;");
    let events = run(&mut r, vec![lib.hint("a.sql")], t0() + ms(1_000));
    assert_eq!(events, vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn own_new_file_is_not_reported() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    let echo = r.echo();
    fileio::save_with_echo(
        &lib.root,
        &rel("new.sql"),
        None,
        text("SELECT 1;"),
        None,
        &mut |s| echo.record(rel("new.sql"), *s),
    )
    .unwrap();
    assert_eq!(run(&mut r, vec![lib.hint("new.sql")], t0()), vec![]);
    assert_eq!(r.known_files(), 2);
}

#[test]
fn a_foreign_write_with_the_echos_length_and_time_is_still_reported() {
    // Same length, and the recorded time forced to match: only the hash tells them apart.
    let lib = Lib::new(&[("a.sql", "AAAA")]);
    let mut r = lib.rec();
    lib.write("a.sql", "BBBB");
    let on_disk = fs::metadata(lib.path("a.sql")).unwrap().modified().unwrap();
    r.echo().record(
        rel("a.sql"),
        DiskStamp {
            len: 4,
            mtime: on_disk,
            hash: 1,
        },
    );
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], t0()),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
}

// ---------------------------------------------------------------------------------------------
// Reconciliation of editor save patterns (alg/28 §0.5, C3, C4).

#[test]
fn atomic_rename_save_is_one_change() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write("a.sql.tmp", "SELECT 2222;");
    fs::rename(lib.path("a.sql.tmp"), lib.path("a.sql")).unwrap();
    let events = run(&mut r, vec![lib.hint("a.sql.tmp"), lib.hint("a.sql")], t0());
    assert_eq!(events, vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn jetbrains_safe_write_is_one_change() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write("a.sql___jb_tmp___", "SELECT 22;");
    fs::rename(lib.path("a.sql"), lib.path("a.sql___jb_old___")).unwrap();
    fs::rename(lib.path("a.sql___jb_tmp___"), lib.path("a.sql")).unwrap();
    fs::remove_file(lib.path("a.sql___jb_old___")).unwrap();
    let hints = ["a.sql___jb_tmp___", "a.sql___jb_old___", "a.sql"]
        .map(|n| lib.hint(n))
        .to_vec();
    assert_eq!(
        run(&mut r, hints, t0()),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
}

#[test]
fn vim_backup_rename_save_is_one_change() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    // vim `backupcopy=auto`: probe `4913`, rename the original to `a.sql~`, write the new file,
    // delete the backup.
    lib.write("4913", "");
    fs::remove_file(lib.path("4913")).unwrap();
    fs::rename(lib.path("a.sql"), lib.path("a.sql~")).unwrap();
    lib.write("a.sql", "SELECT 22;");
    fs::remove_file(lib.path("a.sql~")).unwrap();
    let hints = vec![lib.hint("4913"), lib.hint("a.sql~"), lib.hint("a.sql")];
    assert_eq!(
        run(&mut r, hints, t0()),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
}

#[test]
fn a_save_seen_mid_rename_is_not_a_removal() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    fs::rename(lib.path("a.sql"), lib.path("a.sql~")).unwrap();
    r.push(lib.hint("a.sql"), t0());
    // The first look finds no file.
    assert_eq!(r.tick(t0()), vec![]);
    lib.write("a.sql", "SELECT 22;");
    fs::remove_file(lib.path("a.sql~")).unwrap();
    assert_eq!(drain(&mut r, t0()), vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn sed_temp_is_never_listed() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write(".!123!a.sql", "SELECT 2;");
    assert_eq!(run(&mut r, vec![lib.hint(".!123!a.sql")], t0()), vec![]);
}

#[test]
fn inplace_write_reported_as_create_is_a_change() {
    // FSEvents reports an in-place write as `Create(File)`; the hint has no kind, and a known
    // file with a new stamp is a change.
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write("a.sql", "SELECT 1; SELECT 2;");
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], t0()),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
}

#[test]
fn an_unchanged_file_is_not_reported() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    assert_eq!(run(&mut r, vec![lib.hint("a.sql")], t0()), vec![]);
}

#[test]
fn half_written_file_waits_to_settle() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write("a.sql", "SEL");
    r.push(lib.hint("a.sql"), t0());
    assert_eq!(r.tick(t0()), vec![]);
    // The writer is still at it at the second look: no report, another window.
    lib.write("a.sql", "SELECT 1; SELECT");
    assert_eq!(r.tick(t0() + W), vec![]);
    assert_eq!(r.next_due(), Some(t0() + W + W));
    lib.write("a.sql", "SELECT 1; SELECT 2;");
    assert_eq!(r.tick(t0() + W + W), vec![]);
    // Stable at two looks one window apart: now it is reported.
    assert_eq!(
        r.tick(t0() + W * 3),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
    assert_eq!(r.next_due(), None);
}

#[test]
fn a_file_that_never_settles_is_reported_unsettled_after_ten_looks() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    let mut body = String::from("x");
    r.push(lib.hint("a.sql"), t0());
    let mut events = Vec::new();
    for i in 0..12u32 {
        body.push('y');
        lib.write("a.sql", &body);
        events.extend(r.tick(t0() + W * i));
    }
    assert_eq!(events, vec![WatchEvent::Unsettled(rel("a.sql"))]);
}

#[test]
fn a_new_script_is_created_after_it_settles() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write("b.sql", "SELECT 2;");
    r.push(lib.hint("b.sql"), t0());
    assert_eq!(r.tick(t0()), vec![], "never reported at the first look");
    assert_eq!(drain(&mut r, t0()), vec![WatchEvent::Created(rel("b.sql"))]);
}

#[test]
fn a_removed_file_is_removed_after_it_stays_gone() {
    let lib = Lib::new(&[("a.sql", "x"), ("b.md", "y")]);
    let mut r = lib.rec();
    fs::remove_file(lib.path("b.md")).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("b.md")], t0()),
        vec![WatchEvent::Removed(rel("b.md"))]
    );
    assert_eq!(r.known_files(), 1);
}

#[test]
fn events_outside_listed_types_are_dropped_but_folders_rescan() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write("c.txt", "notes");
    assert_eq!(run(&mut r, vec![lib.hint("c.txt")], t0()), vec![]);
    // A new folder arrives as one hint on the folder: its subtree is reconciled.
    lib.write("reports/r.sql", "SELECT 3;");
    lib.write("reports/notes.txt", "n");
    assert_eq!(
        run(&mut r, vec![lib.hint("reports")], t0() + ms(1_000)),
        vec![WatchEvent::Created(rel("reports/r.sql"))]
    );
}

#[test]
fn hints_outside_the_root_and_in_skipped_folders_are_dropped() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write("node_modules/x.sql", "SELECT 1;");
    lib.write(".hidden/y.sql", "SELECT 1;");
    lib.write(".git/config", "[core]");
    let hints = vec![
        RawHint::Path("/elsewhere/a.sql".into()),
        lib.hint("node_modules/x.sql"),
        lib.hint(".hidden/y.sql"),
        lib.hint(".git/config"),
    ];
    assert_eq!(run(&mut r, hints, t0()), vec![]);
}

#[test]
fn repository_index_change_is_reported_once() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write(".git/index", "i");
    lib.write(".git/HEAD", "h");
    let hints = vec![
        lib.hint(".git/index"),
        lib.hint(".git/HEAD"),
        lib.hint(".git/objects/ab"),
    ];
    assert_eq!(
        run(&mut r, hints, t0()),
        vec![WatchEvent::RepositoryChanged]
    );
}

#[test]
fn folder_delete_hint_on_parent_rescans() {
    let lib = Lib::new(&[
        ("a.sql", "x"),
        ("reports/c.sql", "c"),
        ("reports/r.md", "r"),
    ]);
    let mut r = lib.rec();
    fs::remove_dir_all(lib.path("reports")).unwrap();
    // `rm -rf reports` reaches the window only as a metadata change on `reports`.
    assert_eq!(
        run(&mut r, vec![lib.hint("reports")], t0()),
        vec![
            WatchEvent::Removed(rel("reports/c.sql")),
            WatchEvent::Removed(rel("reports/r.md")),
        ]
    );
}

#[test]
fn folder_move_hint_rescans_both() {
    let lib = Lib::new(&[
        ("a.sql", "x"),
        ("reports/c.sql", "c"),
        ("reports/r.md", "rr"),
    ]);
    let mut r = lib.rec();
    // The tabs that had these files open told the watcher their content stamps.
    r.remember(&rel("reports/c.sql"), &lib.read_stamp("reports/c.sql"));
    r.remember(&rel("reports/r.md"), &lib.read_stamp("reports/r.md"));
    fs::rename(lib.path("reports"), lib.path("archive")).unwrap();
    // `mv reports archive` reaches the window only as `Create(Folder)` on `archive`.
    assert_eq!(
        run(&mut r, vec![lib.hint("archive")], t0()),
        vec![
            WatchEvent::Renamed {
                from: rel("reports/c.sql"),
                to: rel("archive/c.sql")
            },
            WatchEvent::Renamed {
                from: rel("reports/r.md"),
                to: rel("archive/r.md")
            },
        ]
    );
}

#[test]
fn folder_move_without_known_hashes_is_removals_and_creations() {
    let lib = Lib::new(&[("reports/c.sql", "c")]);
    let mut r = lib.rec();
    fs::rename(lib.path("reports"), lib.path("archive")).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("archive")], t0()),
        vec![
            WatchEvent::Removed(rel("reports/c.sql")),
            WatchEvent::Created(rel("archive/c.sql")),
        ]
    );
}

#[test]
fn file_rename_pairs_by_hash_only_when_the_contents_match() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;"), ("b.sql", "SELECT 2;")]);
    let mut r = lib.rec();
    r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
    r.remember(&rel("b.sql"), &lib.read_stamp("b.sql"));
    fs::rename(lib.path("a.sql"), lib.path("c.sql")).unwrap();
    // b.sql is replaced by a file of the same length but other contents.
    fs::remove_file(lib.path("b.sql")).unwrap();
    lib.write("d.sql", "SELECT 9;");
    let hints = ["a.sql", "c.sql", "b.sql", "d.sql"]
        .map(|n| lib.hint(n))
        .to_vec();
    assert_eq!(
        run(&mut r, hints, t0()),
        vec![
            WatchEvent::Renamed {
                from: rel("a.sql"),
                to: rel("c.sql")
            },
            WatchEvent::Removed(rel("b.sql")),
            WatchEvent::Created(rel("d.sql")),
        ]
    );
}

#[test]
fn a_hint_in_another_normal_form_reaches_the_known_file() {
    // macOS hands out NFD names; an event may carry the other form.
    let nfd = "cafe\u{301}.sql";
    let nfc = "caf\u{e9}.sql";
    let lib = Lib::new(&[(nfd, "x")]);
    let mut r = lib.rec();
    lib.write(nfd, "xyz");
    assert_eq!(
        run(&mut r, vec![lib.hint(nfc)], t0()),
        vec![WatchEvent::Changed(rel(nfd))]
    );
}

#[test]
fn rescan_hint_is_one_rescan_and_relearns_the_library() {
    // FSEvents `MustScanSubDirs` and inotify `IN_Q_OVERFLOW` arrive as `RawHint::Rescan`.
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write("b.sql", "y");
    assert_eq!(
        run(
            &mut r,
            vec![
                lib.hint("a.sql"),
                RawHint::Rescan(lib.root.canonical.clone())
            ],
            t0()
        ),
        vec![WatchEvent::Rescan]
    );
    assert_eq!(
        r.known_files(),
        2,
        "b.sql was learned by the rescan, not reported"
    );
    lib.write("b.sql", "yy");
    assert_eq!(
        run(&mut r, vec![lib.hint("b.sql")], t0() + ms(1_000)),
        vec![WatchEvent::Changed(rel("b.sql"))]
    );
}

#[test]
fn a_library_at_its_file_limit_rescans_instead_of_guessing() {
    let lib = Lib::new(&[("a.sql", "x"), ("b.sql", "y")]);
    let opts = WatchOpts {
        limits: Limits {
            max_files: 2,
            ..Limits::default()
        },
        ..WatchOpts::default()
    };
    let mut r = lib.rec_with(opts);
    lib.write("c.sql", "z");
    assert_eq!(
        run(&mut r, vec![lib.hint("c.sql")], t0()),
        vec![WatchEvent::Rescan]
    );
}

#[test]
fn root_gone_is_a_rescan_never_removals() {
    let lib = Lib::new(&[("a.sql", "x"), ("b.sql", "y")]);
    let mut r = lib.rec();
    fs::remove_dir_all(&lib.root.canonical).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], t0()),
        vec![WatchEvent::Rescan]
    );
    assert_eq!(
        run(&mut r, vec![lib.hint("")], t0() + ms(1_000)),
        vec![WatchEvent::Rescan]
    );
    // The guard still owns the (now missing) folder; recreate it so its drop is a no-op.
    fs::create_dir_all(lib.dir.path()).unwrap();
}

/// A file system that records every path it is asked about.
#[derive(Default)]
struct Counting {
    paths: Mutex<Vec<PathBuf>>,
}

impl Counting {
    fn note(&self, p: &Path) {
        self.paths.lock().unwrap().push(p.to_path_buf());
    }
    fn asked_under(&self, dir: &Path) -> Vec<PathBuf> {
        let paths = self.paths.lock().unwrap();
        paths
            .iter()
            .filter(|p| p.starts_with(dir) && *p != dir)
            .cloned()
            .collect()
    }
}

impl Fs for Counting {
    fn read_dir(&self, dir: &Path, cap: usize) -> io::Result<Vec<OsString>> {
        self.note(dir);
        RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        self.note(path);
        RealFs.symlink_metadata(path)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.note(path);
        RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        self.note(link);
        RealFs.read_link(link)
    }
}

#[cfg(unix)]
#[test]
fn a_hint_on_a_directory_link_never_looks_through_it() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let outside = tempfile::tempdir().unwrap();
    for i in 0..50 {
        fs::write(outside.path().join(format!("o{i}.sql")), "SELECT 1;").unwrap();
    }
    let counting = Arc::new(Counting::default());
    let opts = WatchOpts {
        fs: counting.clone(),
        ..WatchOpts::default()
    };
    let mut r = lib.rec_with(opts);
    std::os::unix::fs::symlink(outside.path(), lib.path("outside")).unwrap();
    std::os::unix::fs::symlink("/", lib.path("root")).unwrap();
    let hints = vec![
        lib.hint("outside"),
        lib.hint("outside/o1.sql"),
        lib.hint("root"),
        lib.hint("root/etc"),
    ];
    assert_eq!(run(&mut r, hints, t0()), vec![]);
    assert_eq!(
        counting.asked_under(&lib.path("outside")),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        counting.asked_under(&lib.path("root")),
        Vec::<PathBuf>::new()
    );
}

#[cfg(unix)]
#[test]
fn a_new_file_link_inside_the_library_is_created() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    std::os::unix::fs::symlink("a.sql", lib.path("alias.sql")).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("alias.sql")], t0()),
        vec![WatchEvent::Created(rel("alias.sql"))]
    );
}

// ---------------------------------------------------------------------------------------------
// Backend choice (R-M4b-14; HV-SCRIPT-08 step 1).

#[test]
fn small_inotify_budget_chooses_polling() {
    assert_eq!(
        choose_backend(5_000, Some(6_000), FsKind::Local),
        Backend::Poll(PollReason::WatchBudget)
    );
    assert_eq!(
        choose_backend(5_000, Some(10_000), FsKind::Local),
        Backend::Os
    );
    assert_eq!(
        choose_backend(5_001, Some(10_000), FsKind::Local),
        Backend::Poll(PollReason::WatchBudget)
    );
    assert_eq!(choose_backend(5_000, None, FsKind::Unknown), Backend::Os);
}

#[test]
fn network_fs_chooses_polling() {
    assert_eq!(
        choose_backend(10, None, FsKind::Network),
        Backend::Poll(PollReason::NetworkFs)
    );
    assert_eq!(
        choose_backend(10, Some(1_000_000), FsKind::Network),
        Backend::Poll(PollReason::NetworkFs)
    );
    let mountinfo = "\
22 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw
40 22 0:41 / /mnt/share rw,relatime shared:20 - cifs //nas/share rw,vers=3.1.1
41 22 0:42 / /mnt/nfs rw shared:21 - nfs4 nas:/export rw
42 40 8:2 / /mnt/share/local\\040disk rw shared:22 - ext4 /dev/sdb1 rw
43 22 0:50 / /mnt/c rw,noatime - 9p drvfs rw
44 22 0:51 / /home/u/remote rw - fuse.sshfs u@nas: rw
";
    let kind = |p: &str| fs_kind_from_mountinfo(mountinfo, Path::new(p));
    assert_eq!(kind("/home/u/lib"), FsKind::Local);
    assert_eq!(kind("/mnt/share/lib"), FsKind::Network);
    assert_eq!(kind("/mnt/nfs/lib"), FsKind::Network);
    assert_eq!(kind("/mnt/c/Users/lib"), FsKind::Network);
    assert_eq!(kind("/home/u/remote/lib"), FsKind::Network);
    // The longest mount point wins, and `\040` is a space.
    assert_eq!(kind("/mnt/share/local disk/lib"), FsKind::Local);
    // `/mnt/sharex` is not under `/mnt/share`.
    assert_eq!(kind("/mnt/sharex/lib"), FsKind::Local);
    assert_eq!(fs_kind_from_mountinfo("", Path::new("/x")), FsKind::Unknown);
}

#[test]
fn inotify_budget_is_max_user_watches_minus_the_watches_in_use() {
    assert_eq!(budget_from("8192\n", 100), Some(8_092));
    assert_eq!(budget_from("8192", 9_000), Some(0));
    assert_eq!(budget_from("lots", 0), None);
}

#[cfg(unix)]
#[test]
fn inotify_budget_reads_a_simulated_proc() {
    use std::os::unix::fs::symlink;
    let proc_root = tempfile::tempdir().unwrap();
    let p = proc_root.path();
    fs::create_dir_all(p.join("sys/fs/inotify")).unwrap();
    // A small `max_user_watches`, as on many distributions before 5.11.
    fs::write(p.join("sys/fs/inotify/max_user_watches"), "8192\n").unwrap();
    for (pid, watches) in [("101", 3usize), ("202", 2)] {
        fs::create_dir_all(p.join(pid).join("fd")).unwrap();
        fs::create_dir_all(p.join(pid).join("fdinfo")).unwrap();
        symlink("anon_inode:inotify", p.join(pid).join("fd/7")).unwrap();
        symlink("/dev/null", p.join(pid).join("fd/1")).unwrap();
        let mut info = String::from("pos:\t0\nflags:\t00\n");
        for w in 0..watches {
            info.push_str(&format!(
                "inotify wd:{w} ino:1 sdev:1 mask:fce ignored_mask:0\n"
            ));
        }
        fs::write(p.join(pid).join("fdinfo/7"), info).unwrap();
        fs::write(p.join(pid).join("fdinfo/1"), "inotify wd:99 not counted\n").unwrap();
    }
    fs::create_dir_all(p.join("self")).unwrap();
    assert_eq!(inotify_budget_at(p), Some(8_192 - 5));
    // 5,000 folders against what is left: polled.
    assert_eq!(
        choose_backend(5_000, inotify_budget_at(p), FsKind::Local),
        Backend::Poll(PollReason::WatchBudget)
    );
    assert_eq!(inotify_budget_at(&p.join("missing")), None);
}

#[test]
fn count_dirs_counts_every_folder_and_stops_at_the_cap() {
    let lib = Lib::new(&[
        ("a/b/c.sql", "x"),
        (".git/objects/x", "y"),
        ("d/e.sql", "z"),
    ]);
    // root, a, a/b, .git, .git/objects, d
    assert_eq!(count_dirs(&lib.root.canonical, 1_000), 6);
    assert_eq!(count_dirs(&lib.root.canonical, 3), 3);
}

// ---------------------------------------------------------------------------------------------
// The watcher end to end on the fake source.

struct Events {
    rt: tokio::runtime::Runtime,
    rx: mpsc::Receiver<WatchBatch>,
    seen: Vec<WatchEvent>,
}

impl Events {
    fn new() -> (mpsc::Sender<WatchBatch>, Events) {
        let (tx, rx) = mpsc::channel(64);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        (
            tx,
            Events {
                rt,
                rx,
                seen: Vec::new(),
            },
        )
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
                Ok(Some(batch)) => self.seen.extend(batch),
                Ok(None) => panic!("{what}: the watcher hung up; saw {:?}", self.seen),
                Err(_) => panic!("{what}: not within 10 s; saw {:?}", self.seen),
            }
        }
    }
}

fn fake_watcher(lib: &Lib) -> (Watcher, FakeHandle, FakeHandle, Events) {
    let (src, handle) = FakeEventSource::new();
    let (poll, poll_handle) = FakeEventSource::new();
    let mut poll = Some(poll);
    let (tx, ev) = Events::new();
    let w = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(src),
        Box::new(move || {
            poll.take()
                .map(|p| Box::new(p) as Box<dyn EventSource>)
                .ok_or(WatchError::Backend("one fallback".into()))
        }),
    )
    .unwrap();
    (w, handle, poll_handle, ev)
}

#[test]
fn fake_source_events_flow_through_the_watcher() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (w, handle, _poll, mut ev) = fake_watcher(&lib);
    assert_eq!(w.backend(), Backend::Os);
    assert_eq!(handle.watched(), vec![lib.root.canonical.clone()]);
    lib.write("a.sql", "xy");
    assert!(handle.emit(&lib.root.canonical, vec![lib.hint("a.sql")]));
    ev.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
    assert_eq!(ev.seen, vec![WatchEvent::Changed(rel("a.sql"))]);
}

#[test]
fn burst_becomes_one_rescan_end_to_end() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (_w, handle, _poll, mut ev) = fake_watcher(&lib);
    let rescans = |e: &[WatchEvent]| e.iter().filter(|x| **x == WatchEvent::Rescan).count();
    // One batch is queued whole, so its 1,001 hints are counted in one window.
    let burst: Vec<RawHint> = (0..1_001).map(|i| lib.hint(&format!("f{i}.sql"))).collect();
    handle.emit(&lib.root.canonical, burst);
    ev.wait_for("burst", |e| rescans(e) == 1);
    // More than the queue holds is one rescan too (and the window rule would make it one).
    let overflow: Vec<RawHint> = (0..10_001)
        .map(|i| lib.hint(&format!("g{i}.sql")))
        .collect();
    handle.emit(&lib.root.canonical, overflow);
    ev.wait_for("overflow", |e| rescans(e) == 2);
    // Both are handled: a later file is reported on its own.
    lib.write("sentinel.sql", "s");
    handle.emit(&lib.root.canonical, vec![lib.hint("sentinel.sql")]);
    ev.wait_for("sentinel", |e| {
        e.contains(&WatchEvent::Created(rel("sentinel.sql")))
    });
    assert_eq!(
        ev.seen,
        vec![
            WatchEvent::Rescan,
            WatchEvent::Rescan,
            WatchEvent::Created(rel("sentinel.sql"))
        ]
    );
}

#[test]
fn backend_lost_switches_to_polling() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (w, handle, poll, mut ev) = fake_watcher(&lib);
    handle.emit(&lib.root.canonical, vec![RawHint::Lost]);
    ev.wait_for("lost", |e| {
        e.contains(&WatchEvent::BackendLost(LostReason::Failed))
    });
    assert_eq!(
        ev.seen,
        vec![
            WatchEvent::BackendLost(LostReason::Failed),
            WatchEvent::Rescan
        ]
    );
    assert_eq!(w.backend(), Backend::Poll(PollReason::BackendLost));
    assert_eq!(
        handle.watched(),
        Vec::<PathBuf>::new(),
        "the lost source was unwatched"
    );
    assert_eq!(poll.watched(), vec![lib.root.canonical.clone()]);
    // Hints now come from the poll source.
    lib.write("a.sql", "xyz");
    poll.emit(&lib.root.canonical, vec![lib.hint("a.sql")]);
    ev.wait_for("changed", |e| {
        e.contains(&WatchEvent::Changed(rel("a.sql")))
    });
    // A loss reported by the fallback itself keeps it (there is nothing further to fall back
    // to) and rescans; the backend is not lost twice.
    poll.emit(&lib.root.canonical, vec![RawHint::Lost]);
    let before = ev.seen.len();
    ev.wait_for("rescan", |e| e[before..].contains(&WatchEvent::Rescan));
    assert_eq!(ev.seen[before..].to_vec(), vec![WatchEvent::Rescan]);
    assert_eq!(poll.watched(), vec![lib.root.canonical.clone()]);
    assert_eq!(w.backend(), Backend::Poll(PollReason::BackendLost));
}

#[test]
fn start_failure_falls_back_to_polling() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (src, handle) = FakeEventSource::refusing(WatchError::Backend("no watch".into()));
    let (poll, poll_handle) = FakeEventSource::new();
    let mut poll = Some(poll);
    let (tx, mut ev) = Events::new();
    let w = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(src),
        Box::new(move || Ok(Box::new(poll.take().unwrap()) as Box<dyn EventSource>)),
    )
    .unwrap();
    assert_eq!(w.backend(), Backend::Poll(PollReason::BackendLost));
    assert_eq!(handle.watched(), Vec::<PathBuf>::new());
    assert_eq!(poll_handle.watched(), vec![lib.root.canonical.clone()]);
    ev.wait_for("lost", |e| !e.is_empty());
    assert_eq!(
        ev.seen,
        vec![WatchEvent::BackendLost(LostReason::StartFailed)]
    );
}

#[test]
fn start_fails_when_the_fallback_fails_too() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (src, _handle) = FakeEventSource::refusing(WatchError::Backend("no watch".into()));
    let (tx, _ev) = Events::new();
    let r = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(src),
        Box::new(|| Err(WatchError::Backend("no poll".into()))),
    );
    assert_eq!(r.unwrap_err(), WatchError::Backend("no watch".into()));
}

#[test]
fn dropping_the_watcher_releases_everything() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (w, handle, _poll, _ev) = fake_watcher(&lib);
    let probe = w.probe();
    assert!(!probe.released());
    drop(w);
    assert!(
        probe.released(),
        "the thread ended and the source dropped its sink"
    );
    assert_eq!(handle.watched(), Vec::<PathBuf>::new());
}

#[test]
fn a_closed_receiver_ends_the_watcher() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (w, handle, _poll, ev) = fake_watcher(&lib);
    let probe = w.probe();
    drop(ev);
    lib.write("b.sql", "y");
    handle.emit(&lib.root.canonical, vec![lib.hint("b.sql")]);
    // The thread ends at its first delivery after the hang-up (condition wait, 10 s).
    poll_until("the watcher unwatches", || handle.watched().is_empty());
    assert_eq!(handle.watched(), Vec::<PathBuf>::new());
    drop(w);
    assert!(probe.released());
}

#[test]
fn a_full_receiver_never_blocks_the_watcher_and_loses_nothing() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (src, handle) = FakeEventSource::new();
    let (tx, rx) = mpsc::channel(1);
    let w = Watcher::start(lib.root.clone(), WatchOpts::default(), tx, Box::new(src)).unwrap();
    let mut names = HashSet::new();
    for i in 0..5 {
        let n = format!("n{i}.sql");
        lib.write(&n, "y");
        handle.emit(&lib.root.canonical, vec![lib.hint(&n)]);
        names.insert(rel(&n));
    }
    let mut ev = Events {
        rt: tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap(),
        rx,
        seen: Vec::new(),
    };
    ev.wait_for("all five", |e| {
        let got: HashSet<RelPath> = e
            .iter()
            .filter_map(|x| match x {
                WatchEvent::Created(r) => Some(r.clone()),
                _ => None,
            })
            .collect();
        got == names
    });
    drop(w);
}

#[test]
fn a_file_dated_near_the_epoch_reconciles() {
    // A file whose time is before the epoch (FAT, a restored archive) still reconciles.
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    let f = fs::File::options()
        .write(true)
        .open(lib.path("a.sql"))
        .unwrap();
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
        .unwrap();
    drop(f);
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], t0()),
        vec![WatchEvent::Changed(rel("a.sql"))]
    );
}

#[test]
fn poll_source_hints_each_root_every_interval_until_unwatched() {
    use hacivat_scripts::watch::PollEventSource;
    let (tx, rx) = std::sync::mpsc::channel::<Vec<RawHint>>();
    let tx = Mutex::new(tx);
    let mut src = PollEventSource::with_interval(ms(20)).unwrap();
    src.watch(
        Path::new("/lib"),
        Box::new(move |b| {
            let _ = tx.lock().unwrap().send(b);
        }),
    )
    .unwrap();
    for _ in 0..3 {
        let batch = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(batch, vec![RawHint::Path("/lib".into())]);
    }
    src.unwatch(Path::new("/lib"));
    // The sink went with the root: the channel closes once the queued batches are read.
    while rx.recv_timeout(Duration::from_secs(10)).is_ok() {}
    assert!(matches!(
        rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
    drop(src);
}
