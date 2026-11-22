//! M4b Task 9: the watcher's deterministic logic (HV-SCRIPT-08; R-M4b-14, -16; alg/28 C3–C6).
//!
//! The reconciler runs on a real temp folder with an injected clock (no OS events, no sleeps);
//! the watcher end to end runs on `testing::FakeEventSource` with condition waits (≤ 10 s).

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hacivat_scripts::fileio::{self, ReadLimits};
use hacivat_scripts::scan::{Fs, Limits, Meta, RealFs};
use hacivat_scripts::testing::{FakeEventSource, FakeHandle};
use hacivat_scripts::watch::limits::{budget_from, count_dirs, fs_kind_from_mountinfo};
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

/// Sets `path`'s modification time `secs` seconds into the past.
fn age(path: &Path, secs: u64) {
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(SystemTime::now() - Duration::from_secs(secs))
        .unwrap();
}

/// Sets `path`'s modification time.
fn set_mtime(path: &Path, t: SystemTime) {
    let f = fs::File::options().write(true).open(path).unwrap();
    f.set_modified(t).unwrap();
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
    // 600 + 600 less than a window apart, flushed in between: one burst, rescanned once it
    // has been quiet for a window (fix round 1: the rescan sees the end state).
    let mut d = Debouncer::new(ms(150));
    for i in 0..600 {
        d.push(raw_modify(&format!("a{i}.sql")), t0());
    }
    let _ = d.flush(t0() + ms(50));
    for i in 0..600 {
        d.push(raw_modify(&format!("b{i}.sql")), t0() + ms(100));
    }
    assert_eq!(d.flush(t0() + ms(120)), vec![]);
    assert_eq!(d.due(), Some(t0() + ms(250)));
    assert_eq!(d.flush(t0() + ms(250)), vec![Flushed::Rescan]);
    assert_eq!(d.due(), None);
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
fn a_burst_rescans_once_when_quiet_and_lost_is_never_swallowed() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..1_001 {
        d.push(raw_modify(&format!("f{i}.sql")), t0());
    }
    d.push(RawHint::Lost, t0());
    assert_eq!(d.flush(t0()), vec![Flushed::Lost]);
    // The burst goes on: swallowed, and the rescan waits for it to end.
    d.push(raw_modify("late.sql"), t0() + ms(20));
    assert_eq!(d.flush(t0() + ms(30)), vec![]);
    assert_eq!(d.flush(t0() + ms(170)), vec![Flushed::Rescan]);
    // A later hint is a new burst, looked at on its own.
    d.push(raw_modify("next.sql"), t0() + ms(400));
    assert_eq!(
        d.flush(t0() + ms(410)),
        vec![Flushed::Paths(vec!["/lib/next.sql".into()])]
    );
}

#[test]
fn one_file_rewritten_in_a_loop_is_never_a_burst() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..5_000u64 {
        d.push(raw_modify("log.sql"), t0() + Duration::from_micros(i * 10));
    }
    assert_eq!(
        d.flush(t0() + ms(60)),
        vec![Flushed::Paths(vec!["/lib/log.sql".into()])]
    );
}

#[test]
fn an_endless_burst_still_rescans_every_two_seconds() {
    let mut d = Debouncer::new(ms(150));
    let mut rescans = Vec::new();
    for step in 0..50u64 {
        // 1,001 new names every 100 ms, for 5 s: the burst never quiets.
        let at = t0() + ms(step * 100);
        for i in 0..1_001 {
            d.push(raw_modify(&format!("s{step}-f{i}.sql")), at);
        }
        if d.flush(at).contains(&Flushed::Rescan) {
            rescans.push(step);
        }
    }
    // Fix round 3 (I-fr2-1): a burst still going stays overflowed after a forced rescan, so
    // one rescan every 2 s, and the count restarts only once it is quiet.
    assert_eq!(rescans, vec![20, 40]);
    assert_eq!(d.flush(t0() + ms(4_900 + 150)), vec![Flushed::Rescan]);
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
    // Old enough that its stamp proves it unchanged (R-T9-3).
    age(&lib.path("a.sql"), 60);
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
    // Fix round 1 (I3): still gone is not news; one `Rescan` per change of the root's state.
    assert_eq!(run(&mut r, vec![lib.hint("")], t0() + ms(1_000)), vec![]);
    assert_eq!(r.known_files(), 2, "never a list of removals");
    // The guard still owns the (now missing) folder; recreate it so its drop is a no-op.
    fs::create_dir_all(lib.dir.path()).unwrap();
}

/// A file system that records every path it is asked about.
#[cfg(unix)]
#[derive(Default)]
struct Counting {
    paths: Mutex<Vec<PathBuf>>,
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    // Imported here: the test is `cfg(unix)`, and a file-level import is unused on Windows.
    use hacivat_scripts::watch::limits::inotify_budget_at;
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
    // The drop is bounded (I6); under load the thread may end just after it (fix round 3).
    poll_until("the thread ended and the source dropped its sink", || {
        probe.released()
    });
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
    poll_until("the thread ended", || probe.released());
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

// ---------------------------------------------------------------------------------------------
// ## m4b-t09-watcher fix round 1 (review I1-I8, m1-m10; rulings R-T9-2..R-T9-5).

// I1 / R-T9-3: racily clean files.

/// P1 (review probe): a same-length edit whose time equals the known one, with a hint, on a
/// file a tab has open (its hash is known): re-hashed, and reported.
#[test]
fn same_len_same_mtime_edit_after_hint() {
    let lib = Lib::new(&[("a.sql", "LIMIT 10")]);
    let t = fs::metadata(lib.path("a.sql")).unwrap().modified().unwrap();
    let mut r = Reconciler::new(lib.root.clone(), WatchOpts::default());
    r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
    lib.write("a.sql", "LIMIT 20");
    set_mtime(&lib.path("a.sql"), t);
    let ev = run(&mut r, vec![lib.hint("a.sql")], Instant::now());
    assert_eq!(
        ev,
        vec![WatchEvent::Changed(rel("a.sql"))],
        "P1: edit missed"
    );
}

/// P1, for a file nobody has open: its contents were never hashed, so the hint is passed on as
/// "possibly changed" (R-T9-3: the service hashes only open files).
#[test]
fn same_len_same_mtime_edit_of_a_file_nobody_has_open_is_possibly_changed() {
    let lib = Lib::new(&[("a.sql", "LIMIT 10")]);
    let t = fs::metadata(lib.path("a.sql")).unwrap().modified().unwrap();
    let mut r = lib.rec();
    lib.write("a.sql", "LIMIT 20");
    set_mtime(&lib.path("a.sql"), t);
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], Instant::now()),
        vec![WatchEvent::PossiblyChanged(rel("a.sql"))]
    );
    // Its time set back a minute: a new stamp (a change), and from then on an equal stamp
    // proves it unchanged.
    age(&lib.path("a.sql"), 60);
    let ev = run(&mut r, vec![lib.hint("a.sql")], Instant::now() + ms(1_000));
    assert_eq!(ev, vec![WatchEvent::Changed(rel("a.sql"))]);
    assert_eq!(
        run(&mut r, vec![lib.hint("a.sql")], Instant::now() + ms(2_000)),
        vec![]
    );
}

/// P1b (review probe): Hacivat's save is absorbed as an echo; then a foreign same-length edit
/// lands with the same time (a 1-2 s tick). The echo needs an equal content hash, so the
/// foreign edit is reported.
#[test]
fn external_edit_right_after_own_save_coarse_mtime() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = Reconciler::new(lib.root.clone(), WatchOpts::default());
    let echo = r.echo();
    let before = lib.read_stamp("a.sql");
    let mut ours = None;
    fileio::save_with_echo(
        &lib.root,
        &rel("a.sql"),
        None,
        text("SELECT 22;"),
        Some(before),
        &mut |s| {
            ours = Some(*s);
            echo.record(rel("a.sql"), *s)
        },
    )
    .unwrap();
    let t0 = Instant::now();
    assert_eq!(run(&mut r, vec![lib.hint("a.sql")], t0), vec![]);
    lib.write("a.sql", "SELECT 33;");
    set_mtime(&lib.path("a.sql"), ours.unwrap().mtime);
    let ev = run(&mut r, vec![lib.hint("a.sql")], t0 + ms(400));
    assert_eq!(
        ev,
        vec![WatchEvent::Changed(rel("a.sql"))],
        "P1b: edit hidden"
    );
}

/// Times in whole seconds, as HFS+, ext3 and many SMB servers keep them.
struct Coarse;

impl Fs for Coarse {
    fn read_dir(&self, dir: &Path, cap: usize) -> io::Result<Vec<OsString>> {
        RealFs.read_dir(dir, cap)
    }
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        let mut m = RealFs.symlink_metadata(path)?;
        let secs = m.mtime.duration_since(UNIX_EPOCH).unwrap().as_secs();
        m.mtime = UNIX_EPOCH + Duration::from_secs(secs);
        Ok(m)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        RealFs.canonicalize(path)
    }
    fn read_link(&self, link: &Path) -> io::Result<PathBuf> {
        RealFs.read_link(link)
    }
}

#[test]
fn a_same_second_edit_on_a_coarse_time_file_system_is_not_lost() {
    let lib = Lib::new(&[("a.sql", "LIMIT 10"), ("old.sql", "LIMIT 10")]);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let second = UNIX_EPOCH + Duration::from_secs(now);
    set_mtime(&lib.path("a.sql"), second + ms(100));
    // A file last written a minute ago: a coarse time that old proves it unchanged.
    set_mtime(
        &lib.path("old.sql"),
        second - Duration::from_secs(60) + ms(100),
    );
    let opts = WatchOpts {
        fs: Arc::new(Coarse),
        ..WatchOpts::default()
    };
    let mut r = lib.rec_with(opts);
    // Edited within the same second, same length: the coarse view shows the same stamp.
    lib.write("a.sql", "LIMIT 20");
    set_mtime(&lib.path("a.sql"), second + ms(700));
    assert_eq!(
        run(
            &mut r,
            vec![lib.hint("a.sql"), lib.hint("old.sql")],
            Instant::now()
        ),
        vec![WatchEvent::PossiblyChanged(rel("a.sql"))]
    );
}

// I2: rename pairing on a unique hash only.

/// P2 (review probe): two identical open files; `rm a.sql; mv b.sql c.sql` in one window.
#[test]
fn identical_files_rename_pairing() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;"), ("b.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
    r.remember(&rel("b.sql"), &lib.read_stamp("b.sql"));
    fs::remove_file(lib.path("a.sql")).unwrap();
    fs::rename(lib.path("b.sql"), lib.path("c.sql")).unwrap();
    let ev = run(
        &mut r,
        ["a.sql", "b.sql", "c.sql"].map(|n| lib.hint(n)).to_vec(),
        t0(),
    );
    assert_eq!(
        ev,
        vec![
            WatchEvent::Removed(rel("a.sql")),
            WatchEvent::Removed(rel("b.sql")),
            WatchEvent::Created(rel("c.sql")),
        ],
        "P2: an ambiguous hash must not pair"
    );
}

#[test]
fn two_new_copies_of_one_moved_file_do_not_pair_either() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
    fs::rename(lib.path("a.sql"), lib.path("b.sql")).unwrap();
    fs::copy(lib.path("b.sql"), lib.path("c.sql")).unwrap();
    let ev = run(
        &mut r,
        ["a.sql", "b.sql", "c.sql"].map(|n| lib.hint(n)).to_vec(),
        t0(),
    );
    assert_eq!(
        ev,
        vec![
            WatchEvent::Removed(rel("a.sql")),
            WatchEvent::Created(rel("b.sql")),
            WatchEvent::Created(rel("c.sql")),
        ]
    );
}

// I3: a library at its limit.

fn limited(lib: &Lib) -> WatchOpts {
    let _ = lib;
    WatchOpts {
        limits: Limits {
            max_files: 2,
            ..Limits::default()
        },
        ..WatchOpts::default()
    }
}

/// P4 (review probe), deterministic: root hints (poll passes) on a library stopped at its
/// limit are quiet while nothing changes; a change is one `Rescan`.
#[test]
fn polled_library_at_limit_rescans_once_per_change() {
    let lib = Lib::new(&[("a.sql", "x"), ("b.sql", "y"), ("c.sql", "z")]);
    let mut r = lib.rec_with(limited(&lib));
    for pass in 0..5u64 {
        let at = t0() + Duration::from_secs(2 * pass);
        assert_eq!(run(&mut r, vec![lib.hint("")], at), vec![], "pass {pass}");
    }
    // A change the stopped scan sees (it lists the first files in byte order).
    lib.write("a.sql", "xx");
    let at = t0() + Duration::from_secs(20);
    assert_eq!(
        run(&mut r, vec![lib.hint("")], at),
        vec![WatchEvent::Rescan]
    );
    let at = t0() + Duration::from_secs(22);
    assert_eq!(run(&mut r, vec![lib.hint("")], at), vec![]);
    // Back under the limit: the state changed, once more.
    for n in ["b.sql", "c.sql"] {
        fs::remove_file(lib.path(n)).unwrap();
    }
    let at = t0() + Duration::from_secs(24);
    assert_eq!(
        run(&mut r, vec![lib.hint("")], at),
        vec![WatchEvent::Rescan]
    );
    let at = t0() + Duration::from_secs(26);
    assert_eq!(run(&mut r, vec![lib.hint("")], at), vec![]);
}

/// P4 end to end through the watcher: five poll passes say nothing, a pass after a change says
/// `Rescan` once. A repository hint after the passes is a marker: hints are taken in order, so
/// once it is reported every pass before it has been looked at.
#[test]
fn polled_library_at_limit_rescans_forever() {
    let lib = Lib::new(&[("a.sql", "x"), ("b.sql", "y"), ("c.sql", "z")]);
    let (src, handle) = FakeEventSource::new();
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(lib.root.clone(), limited(&lib), tx, Box::new(src)).unwrap();
    let marks = |e: &[WatchEvent]| {
        e.iter()
            .filter(|x| **x == WatchEvent::RepositoryChanged)
            .count()
    };
    for _ in 0..5 {
        handle.emit(&lib.root.canonical, vec![lib.hint("")]);
    }
    handle.emit(&lib.root.canonical, vec![lib.hint(".git/index")]);
    ev.wait_for("marker", |e| marks(e) == 1);
    assert_eq!(
        ev.seen,
        vec![WatchEvent::RepositoryChanged],
        "P4: quiet passes rescanned"
    );
    lib.write("a.sql", "xx");
    for _ in 0..5 {
        handle.emit(&lib.root.canonical, vec![lib.hint("")]);
    }
    handle.emit(&lib.root.canonical, vec![lib.hint(".git/index")]);
    ev.wait_for("marker", |e| marks(e) == 2);
    assert_eq!(
        ev.seen.iter().filter(|e| **e == WatchEvent::Rescan).count(),
        1,
        "{:?}",
        ev.seen
    );
}

#[test]
fn a_missing_root_is_one_rescan_per_change() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    let moved = lib.dir.path().with_extension("moved");
    fs::rename(lib.dir.path(), &moved).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("")], t0()),
        vec![WatchEvent::Rescan]
    );
    assert_eq!(
        run(&mut r, vec![lib.hint("")], t0() + Duration::from_secs(2)),
        vec![]
    );
    fs::rename(&moved, lib.dir.path()).unwrap();
    assert_eq!(
        run(&mut r, vec![lib.hint("")], t0() + Duration::from_secs(4)),
        vec![WatchEvent::Rescan]
    );
    assert_eq!(r.known_files(), 1);
}

// I4: the poll source and the repository.

#[test]
fn poll_pass_reports_repository_changes() {
    use hacivat_scripts::watch::PollEventSource;
    let lib = Lib::new(&[("a.sql", "x"), (".git/HEAD", "ref: refs/heads/main\n")]);
    lib.write(".git/index", "i");
    let src = PollEventSource::with_interval(ms(20)).unwrap();
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(lib.root.clone(), WatchOpts::default(), tx, Box::new(src)).unwrap();
    // Once a pass has seen the sentinel, the stamps are recorded; a commit rewrites the index.
    lib.write("sentinel.sql", "s");
    ev.wait_for("sentinel", |e| {
        e.contains(&WatchEvent::Created(rel("sentinel.sql")))
    });
    lib.write(".git/index", "index after a commit");
    ev.wait_for("repository", |e| e.contains(&WatchEvent::RepositoryChanged));
    // A checkout moves HEAD.
    let before = ev.seen.len();
    lib.write(".git/HEAD", "ref: refs/heads/topic\n");
    ev.wait_for("HEAD", |e| {
        e[before..].contains(&WatchEvent::RepositoryChanged)
    });
}

#[cfg(unix)]
#[test]
fn a_linked_repository_folder_is_never_looked_into_by_the_poll_source() {
    use hacivat_scripts::watch::PollEventSource;
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let lib = Lib::new(&[("a.sql", "x")]);
    std::os::unix::fs::symlink(outside.path(), lib.path(".git")).unwrap();
    let mut src = PollEventSource::with_interval(ms(20)).unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<RawHint>>();
    let tx = Mutex::new(tx);
    src.watch(
        &lib.root.canonical,
        Box::new(move |b| {
            let _ = tx.lock().unwrap().send(b);
        }),
    )
    .unwrap();
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    fs::write(outside.path().join("HEAD"), "ref: refs/heads/other\n").unwrap();
    for _ in 0..3 {
        let batch = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(batch, vec![RawHint::Path(lib.root.canonical.clone())]);
    }
}

// I5 / R-T9-4: per-folder watches over our own walk.

#[cfg(unix)]
#[test]
fn watch_dirs_are_real_folders_only() {
    let lib = Lib::new(&[
        ("a/b/c.sql", "x"),
        ("empty/keep.txt", "k"),
        ("node_modules/m/x.sql", "x"),
        (".hidden/y.sql", "y"),
        (".git/HEAD", "h"),
        (".git/objects/ab/cd", "o"),
    ]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), lib.path("linked")).unwrap();
    std::os::unix::fs::symlink("/", lib.path("a/root")).unwrap();
    let counting = Arc::new(Counting::default());
    let r = lib.rec_with(WatchOpts {
        fs: counting.clone(),
        ..WatchOpts::default()
    });
    let dirs = r.watch_dirs().unwrap();
    let want: BTreeSet<PathBuf> = ["", ".git", "a", "a/b", "empty"]
        .iter()
        .map(|d| lib.root.canonical.join(d))
        .map(|p| PathBuf::from(p.to_str().unwrap().trim_end_matches('/')))
        .collect();
    assert_eq!(dirs, want);
    assert_eq!(
        counting.asked_under(&lib.path("linked")),
        Vec::<PathBuf>::new()
    );
    assert_eq!(
        counting.asked_under(&lib.path("a/root")),
        Vec::<PathBuf>::new()
    );
    // Only `.git/logs` itself is looked at (fix round 2, m-fr1-1), never anything inside.
    assert_eq!(
        counting.asked_under(&lib.path(".git")),
        vec![lib.path(".git/logs")]
    );
    let small = lib.rec_with(WatchOpts {
        max_dirs: 3,
        ..WatchOpts::default()
    });
    assert_eq!(small.watch_dirs(), Err(4));
}

/// What a [`PerDir`] does with each folder a sync adds.
type OnAdd = Arc<dyn Fn(&Path) + Send + Sync>;

/// A source that watches per folder and records what it was told.
#[derive(Clone, Default)]
struct PerDir {
    synced: Arc<Mutex<Vec<BTreeSet<PathBuf>>>>,
    refuse: bool,
    /// Called for every folder a sync adds (fix round 2, N2: a file written there before its
    /// watch exists, so no event will ever name it).
    on_add: Option<OnAdd>,
    seen: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl EventSource for PerDir {
    fn watch(&mut self, _root: &Path, _sink: hacivat_scripts::HintSink) -> Result<(), WatchError> {
        Ok(())
    }
    fn unwatch(&mut self, _root: &Path) {}
    fn per_dir(&self) -> bool {
        true
    }
    fn sync_dirs(&mut self, _root: &Path, dirs: &BTreeSet<PathBuf>) -> Result<usize, WatchError> {
        let mut synced = self.synced.lock().unwrap();
        let added = match synced.last() {
            Some(before) => dirs.difference(before).count(),
            None => dirs.len(),
        };
        synced.push(dirs.clone());
        drop(synced);
        if self.refuse {
            return Err(WatchError::Backend("watch limit".into()));
        }
        if let Some(on_add) = &self.on_add {
            let before = self.seen.lock().unwrap().clone();
            for d in dirs.difference(&before) {
                on_add(d);
            }
        }
        self.seen.lock().unwrap().extend(dirs.iter().cloned());
        Ok(added)
    }
}

#[test]
fn per_folder_watches_follow_new_and_removed_folders() {
    let lib = Lib::new(&[("a/x.sql", "x")]);
    let src = PerDir::default();
    let synced = src.synced.clone();
    let (fake, handle) = FakeEventSource::new();
    let (tx, mut ev) = Events::new();
    // The fake delivers the hints; the per-folder source is the one synced.
    struct Both(PerDir, FakeEventSource);
    impl EventSource for Both {
        fn watch(
            &mut self,
            root: &Path,
            sink: hacivat_scripts::HintSink,
        ) -> Result<(), WatchError> {
            self.1.watch(root, sink)
        }
        fn unwatch(&mut self, root: &Path) {
            self.1.unwatch(root)
        }
        fn per_dir(&self) -> bool {
            true
        }
        fn sync_dirs(
            &mut self,
            root: &Path,
            dirs: &BTreeSet<PathBuf>,
        ) -> Result<usize, WatchError> {
            self.0.sync_dirs(root, dirs)
        }
    }
    let _w = Watcher::start(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(Both(src, fake)),
    )
    .unwrap();
    let dirs = |names: &[&str]| -> BTreeSet<PathBuf> {
        names.iter().map(|n| lib.root.canonical.join(n)).collect()
    };
    let root_only: BTreeSet<PathBuf> = BTreeSet::from([lib.root.canonical.clone()]);
    assert_eq!(synced.lock().unwrap()[0], &root_only | &dirs(&["a"]));
    lib.write("b/y.sql", "y");
    handle.emit(&lib.root.canonical, vec![lib.hint("b")]);
    ev.wait_for("created", |e| {
        e.contains(&WatchEvent::Created(rel("b/y.sql")))
    });
    assert_eq!(
        synced.lock().unwrap().last().unwrap(),
        &(&root_only | &dirs(&["a", "b"]))
    );
    fs::remove_dir_all(lib.path("a")).unwrap();
    handle.emit(&lib.root.canonical, vec![lib.hint("a")]);
    ev.wait_for("removed", |e| {
        e.contains(&WatchEvent::Removed(rel("a/x.sql")))
    });
    assert_eq!(
        synced.lock().unwrap().last().unwrap(),
        &(&root_only | &dirs(&["b"]))
    );
}

#[test]
fn too_many_folders_for_one_watch_each_falls_back_to_polling() {
    let lib = Lib::new(&[("a/x.sql", "x"), ("b/y.sql", "y")]);
    let (poll, poll_handle) = FakeEventSource::new();
    let mut poll = Some(poll);
    let (tx, mut ev) = Events::new();
    let w = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts {
            max_dirs: 2,
            ..WatchOpts::default()
        },
        tx,
        Box::new(PerDir::default()),
        Box::new(move || Ok(Box::new(poll.take().unwrap()) as Box<dyn EventSource>)),
    )
    .unwrap();
    assert_eq!(w.backend(), Backend::Poll(PollReason::WatchBudget));
    assert_eq!(poll_handle.watched(), vec![lib.root.canonical.clone()]);
    ev.wait_for("lost", |e| !e.is_empty());
    assert_eq!(
        ev.seen,
        vec![WatchEvent::BackendLost(LostReason::WatchBudget)]
    );
}

#[test]
fn a_refused_folder_watch_falls_back_to_polling() {
    let lib = Lib::new(&[("a/x.sql", "x")]);
    let (poll, _poll_handle) = FakeEventSource::new();
    let mut poll = Some(poll);
    let (tx, _ev) = Events::new();
    let w = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(PerDir {
            refuse: true,
            ..PerDir::default()
        }),
        Box::new(move || Ok(Box::new(poll.take().unwrap()) as Box<dyn EventSource>)),
    )
    .unwrap();
    assert_eq!(w.backend(), Backend::Poll(PollReason::WatchBudget));
}

// I6: a bounded drop.

/// A file system whose `symlink_metadata` of `a.sql` blocks while `gate` is set.
struct Gated {
    gate: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
}

impl Fs for Gated {
    fn read_dir(&self, d: &Path, cap: usize) -> io::Result<Vec<OsString>> {
        RealFs.read_dir(d, cap)
    }
    fn symlink_metadata(&self, p: &Path) -> io::Result<Meta> {
        if self.gate.load(Ordering::SeqCst) && p.ends_with("a.sql") {
            self.entered.store(true, Ordering::SeqCst);
            while self.gate.load(Ordering::SeqCst) {
                std::thread::park_timeout(ms(10));
            }
        }
        RealFs.symlink_metadata(p)
    }
    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        RealFs.canonicalize(p)
    }
    fn read_link(&self, p: &Path) -> io::Result<PathBuf> {
        RealFs.read_link(p)
    }
}

/// P5 (review probe): dropping the watcher while its thread is stuck in a file-system call (a
/// hung network share) returns within the bound, and the thread ends once the call does.
#[test]
fn drop_blocks_during_a_blocked_fs_call() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let gate = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(AtomicBool::new(false));
    let opts = WatchOpts {
        fs: Arc::new(Gated {
            gate: gate.clone(),
            entered: entered.clone(),
        }),
        ..WatchOpts::default()
    };
    let (src, handle) = FakeEventSource::new();
    let (tx, _ev) = Events::new();
    let w = Watcher::start(lib.root.clone(), opts, tx, Box::new(src)).unwrap();
    let probe = w.probe();
    gate.store(true, Ordering::SeqCst);
    lib.write("a.sql", "xy");
    handle.emit(&lib.root.canonical, vec![lib.hint("a.sql")]);
    poll_until("the thread is in the call", || {
        entered.load(Ordering::SeqCst)
    });
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let dropper = std::thread::spawn(move || {
        let started = Instant::now();
        drop(w);
        let _ = done_tx.send(started.elapsed());
    });
    // The drop returns while the call is still hung.
    let took = done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("P5: the drop blocked on the hung call");
    assert!(gate.load(Ordering::SeqCst));
    assert!(!probe.released(), "abandoned, still in the call");
    dropper.join().unwrap();
    gate.store(false, Ordering::SeqCst);
    poll_until("the abandoned thread ends", || probe.released());
    assert!(took < Duration::from_secs(5), "{took:?}");
}

// I7: short names (U-07); the Windows body is in `watch_real.rs`.

/// A hint by another spelling of a known file's name (an 8.3 short name on Windows) is not a
/// known path; it is reconciled by a scan, which reports the file under its listed name.
#[cfg(not(windows))]
#[test]
fn short_name_event_is_reconciled() {
    let lib = Lib::new(&[("longname_script.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    lib.write("longname_script.sql", "SELECT 1; SELECT 2;");
    assert_eq!(
        run(&mut r, vec![lib.hint("LONGNA~1.SQL")], t0()),
        vec![WatchEvent::Changed(rel("longname_script.sql"))]
    );
}

// I8 / R-T9-2: the backstop.

#[test]
fn backstop_root_hint_finds_a_change_no_event_reported() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec_with(WatchOpts {
        backstop: Some(Duration::from_secs(30)),
        ..WatchOpts::default()
    });
    r.start_backstop(t0());
    assert_eq!(r.next_due(), Some(t0() + Duration::from_secs(30)));
    // An edit the backend never reported.
    lib.write("a.sql", "xyz");
    assert_eq!(r.tick(t0() + Duration::from_secs(29)), vec![]);
    let at = t0() + Duration::from_secs(30);
    // The backstop's look, then the settle look one window later.
    assert_eq!(r.tick(at), vec![]);
    assert_eq!(r.next_due(), Some(at + W));
    assert_eq!(r.tick(at + W), vec![WatchEvent::Changed(rel("a.sql"))]);
    assert_eq!(r.next_due(), Some(at + Duration::from_secs(30)));
}

#[test]
fn the_backstop_is_on_by_default_on_windows_only() {
    assert_eq!(
        WatchOpts::default().backstop,
        cfg!(windows).then_some(Duration::from_secs(30))
    );
}

// m4: no fallback.

#[test]
fn a_lost_backend_without_a_fallback_is_unwatched() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let (src, handle) = FakeEventSource::new();
    let (tx, mut ev) = Events::new();
    let w = Watcher::start_with_fallback(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(src),
        Box::new(|| Err(WatchError::Backend("no poll".into()))),
    )
    .unwrap();
    handle.emit(&lib.root.canonical, vec![RawHint::Lost]);
    ev.wait_for("lost", |e| !e.is_empty());
    assert_eq!(ev.seen[0], WatchEvent::BackendLost(LostReason::NoFallback));
    assert_eq!(w.backend(), Backend::Unwatched);
}

// m5: poll pace.

#[test]
fn poll_pace_backs_off_while_idle_and_resets_on_change() {
    use hacivat_scripts::watch::poll::next_interval;
    let (base, max) = (Duration::from_secs(2), Duration::from_secs(30));
    let s = Duration::from_secs;
    assert_eq!(next_interval(s(2), false, ms(50), base, max), s(4));
    assert_eq!(next_interval(s(16), false, ms(50), base, max), s(30));
    assert_eq!(next_interval(s(30), false, ms(50), base, max), s(30));
    assert_eq!(next_interval(s(30), true, ms(50), base, max), s(2));
    // Never closer than five times what a pass cost (a slow share).
    assert_eq!(next_interval(s(2), true, ms(1_200), base, max), s(6));
    assert_eq!(next_interval(s(30), false, s(10), base, max), s(50));
}

#[test]
fn the_poll_source_paces_each_root() {
    use hacivat_scripts::watch::PollEventSource;
    let mut src =
        PollEventSource::with_pace(Duration::from_secs(2), Duration::from_secs(30)).unwrap();
    src.watch(Path::new("/lib"), Box::new(|_| {})).unwrap();
    assert_eq!(
        src.interval(Path::new("/lib")),
        Some(Duration::from_secs(2))
    );
    for _ in 0..5 {
        src.pace(Path::new("/lib"), false, ms(10));
    }
    assert_eq!(
        src.interval(Path::new("/lib")),
        Some(Duration::from_secs(30))
    );
    src.pace(Path::new("/lib"), true, ms(10));
    assert_eq!(
        src.interval(Path::new("/lib")),
        Some(Duration::from_secs(2))
    );
}

// m3: VM shared folders.

#[test]
fn vm_shared_folders_are_polled() {
    let mountinfo = "\
22 1 8:1 / / rw - ext4 /dev/sda1 rw
50 22 0:60 / /media/sf_lib rw - vboxsf lib rw
51 22 0:61 / /mnt/host rw - virtiofs host rw
52 22 0:62 / /mnt/hgfs rw - fuse.vmhgfs-fuse vmhgfs-fuse rw
53 22 0:63 / /media/psf rw - prl_fs none rw
54 22 0:64 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
";
    for p in [
        "/media/sf_lib/x",
        "/mnt/host/x",
        "/mnt/hgfs/x",
        "/media/psf/x",
        "/run/user/1000/gvfs/smb/x",
    ] {
        assert_eq!(
            fs_kind_from_mountinfo(mountinfo, Path::new(p)),
            FsKind::Network,
            "{p}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// ## m4b-t09-watcher fix round 2 (re-review N1, N2, m-fr1-1..6).

/// Ticks a reconciler every 100 ms for `steps` steps, pushing `hints(k)` before each tick, and
/// collects `(ms, event)`.
fn every_100ms(
    r: &mut Reconciler,
    start: Instant,
    steps: u64,
    mut hints: impl FnMut(u64) -> Vec<RawHint>,
) -> Vec<(u64, WatchEvent)> {
    let mut out = Vec::new();
    for k in 0..steps {
        let at = start + ms(100 * k);
        for h in hints(k) {
            r.push(h, at);
        }
        out.extend(r.tick(at).into_iter().map(|e| (100 * k, e)));
    }
    out
}

/// P8 (re-review probe): 10 ignored paths (`node_modules`) every 100 ms for 30 s, and a real
/// edit of `a.sql` at 12 s. Ignored names never count toward a burst, so the edit is reported
/// on its own, within 2 s plus the debounce, and nothing is rescanned.
#[test]
fn noise_churn_starves_a_real_edit() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
    let mut r = lib.rec();
    let start = Instant::now();
    let out = every_100ms(&mut r, start, 300, |k| {
        let mut hints: Vec<RawHint> = (0..10)
            .map(|j| lib.hint(&format!("node_modules/p{}.js", k * 10 + j)))
            .collect();
        if k == 120 {
            lib.write("a.sql", "SELECT 1; SELECT 2;");
            hints.push(lib.hint("a.sql"));
        }
        hints
    });
    let changed: Vec<u64> = out
        .iter()
        .filter(|(_, e)| *e == WatchEvent::Changed(rel("a.sql")))
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(changed.len(), 1, "{out:?}");
    assert!(changed[0] <= 12_000 + 2_000 + 150, "{changed:?}");
    assert!(
        !out.iter().any(|(_, e)| *e == WatchEvent::Rescan),
        "{out:?}"
    );
}

/// The same with churn that does count (names that may be folders: build output without an
/// extension), past the burst limit: rescans happen, and an open file's edit is still
/// settled and reported on its own, never learned silently by a rescan.
#[test]
fn counted_churn_past_the_limit_still_reports_an_open_files_edit() {
    let lib = Lib::new(&[("a.sql", "SELECT 1;"), ("b.sql", "SELECT 2;")]);
    let mut r = lib.rec();
    r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
    let start = Instant::now();
    let out = every_100ms(&mut r, start, 200, |k| {
        let mut hints: Vec<RawHint> = (0..20)
            .map(|j| lib.hint(&format!("gen/f{}", k * 20 + j)))
            .collect();
        // Inside the second overflow (it begins near 12.1 s): the hints are swallowed.
        if k == 130 {
            lib.write("a.sql", "SELECT 1; SELECT 11;");
            lib.write("b.sql", "SELECT 2; SELECT 22;");
            hints.push(lib.hint("a.sql"));
            hints.push(lib.hint("b.sql"));
        }
        hints
    });
    let mut out = out;
    out.extend(
        drain(&mut r, start + ms(20_000))
            .into_iter()
            .map(|e| (20_000, e)),
    );
    let rescans = out.iter().filter(|(_, e)| *e == WatchEvent::Rescan).count();
    assert!(rescans >= 2, "{out:?}");
    let changed: Vec<u64> = out
        .iter()
        .filter(|(_, e)| *e == WatchEvent::Changed(rel("a.sql")))
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(changed.len(), 1, "{out:?}");
    assert!(changed[0] <= 13_000 + 2_000 + 2 * 150 + 100, "{changed:?}");
    // b.sql nobody has open: covered by the rescans (the service rescans the tree).
    assert!(
        !out.iter()
            .any(|(_, e)| *e == WatchEvent::Changed(rel("b.sql")))
    );
}

#[test]
fn ignored_names_never_count_toward_a_burst() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    let mut hints: Vec<RawHint> = Vec::new();
    for i in 0..1_500 {
        hints.push(lib.hint(&format!("node_modules/m/{i}.js")));
        hints.push(lib.hint(&format!(".git/objects/{i:02x}/obj")));
        hints.push(lib.hint(&format!(".next/cache/{i}.json")));
        hints.push(lib.hint(&format!("target/debug/{i}.o")));
    }
    assert_eq!(run(&mut r, hints, t0()), vec![]);
    // Names that may be folders or scripts still count.
    let hints: Vec<RawHint> = (0..1_001).map(|i| lib.hint(&format!("f{i}.sql"))).collect();
    assert_eq!(
        run(&mut r, hints, t0() + ms(1_000)),
        vec![WatchEvent::Rescan]
    );
}

#[test]
fn uncounted_hints_do_not_extend_or_start_a_burst() {
    let mut d = Debouncer::new(ms(150));
    for i in 0..5_000 {
        d.push_path(format!("/lib/b/{i}.o").into(), t0(), false);
    }
    assert_eq!(d.hold_until(), None);
    let out = d.flush(t0() + ms(1));
    assert!(
        matches!(&out[..], [Flushed::Paths(p)] if p.len() == 5_000),
        "{out:?}"
    );
}

/// N2: a folder added by a sync may already hold files written before its watch existed; no
/// event will name them, so the watcher looks at the root once more.
#[test]
fn files_made_before_a_new_folders_watch_are_found() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let root = lib.root.canonical.clone();
    let src = PerDir {
        on_add: Some(Arc::new(move |d: &Path| {
            if d.ends_with("n") {
                // Written between the walk that listed `n` and its watch: never an event.
                fs::write(d.join("late.sql"), "SELECT 9;").unwrap();
            }
        })),
        ..PerDir::default()
    };
    let (fake, handle) = FakeEventSource::new();
    struct Both(PerDir, FakeEventSource);
    impl EventSource for Both {
        fn watch(
            &mut self,
            root: &Path,
            sink: hacivat_scripts::HintSink,
        ) -> Result<(), WatchError> {
            self.1.watch(root, sink)
        }
        fn unwatch(&mut self, root: &Path) {
            self.1.unwatch(root)
        }
        fn per_dir(&self) -> bool {
            true
        }
        fn sync_dirs(
            &mut self,
            root: &Path,
            dirs: &BTreeSet<PathBuf>,
        ) -> Result<usize, WatchError> {
            self.0.sync_dirs(root, dirs)
        }
    }
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(
        lib.root.clone(),
        WatchOpts::default(),
        tx,
        Box::new(Both(src, fake)),
    )
    .unwrap();
    // An empty folder: its scan finds nothing, so no look is pending when its watch is added.
    fs::create_dir(lib.path("n")).unwrap();
    handle.emit(&root, vec![lib.hint("n")]);
    ev.wait_for("late file", |e| {
        e.contains(&WatchEvent::Created(rel("n/late.sql")))
    });
}

/// A root hint the watcher pushes itself (after the first sync, after a sync that added
/// folders) wakes it; it never waits for an unrelated event.
#[test]
fn a_hint_pushed_by_the_watcher_itself_is_due_at_once() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    assert_eq!(r.next_due(), None);
    r.push(lib.hint(""), t0());
    assert_eq!(r.next_due(), Some(t0()));
    let _ = r.tick(t0());
    assert_eq!(r.next_due(), None);
}

// m-fr1-1: ref-only repository moves.

#[test]
fn a_ref_only_move_is_a_repository_change() {
    let lib = Lib::new(&[("a.sql", "x"), (".git/logs/HEAD", "0 1 t\n")]);
    let mut r = lib.rec();
    assert_eq!(
        run(&mut r, vec![lib.hint(".git/logs/HEAD")], t0()),
        vec![WatchEvent::RepositoryChanged]
    );
    let dirs = r.watch_dirs().unwrap();
    assert!(dirs.contains(&lib.path(".git/logs")), "{dirs:?}");
    assert!(!dirs.contains(&lib.path(".git/logs/refs")));
}

#[test]
fn the_poll_source_sees_a_ref_only_move() {
    use hacivat_scripts::watch::PollEventSource;
    let lib = Lib::new(&[("a.sql", "x"), (".git/HEAD", "ref: refs/heads/main\n")]);
    lib.write(".git/logs/HEAD", "0 1 t\n");
    let src = PollEventSource::with_interval(ms(20)).unwrap();
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(lib.root.clone(), WatchOpts::default(), tx, Box::new(src)).unwrap();
    lib.write("sentinel.sql", "s");
    ev.wait_for("sentinel", |e| {
        e.contains(&WatchEvent::Created(rel("sentinel.sql")))
    });
    // `reset --soft` moves the branch: only `logs/HEAD` (and the ref) change.
    lib.write(".git/logs/HEAD", "0 1 t\n1 2 t\n");
    ev.wait_for("logs", |e| e.contains(&WatchEvent::RepositoryChanged));
}

// m-fr1-2: a future date is racy only near the look.

#[test]
fn a_file_dated_far_ahead_is_not_racy_forever() {
    let lib = Lib::new(&[("far.sql", "x"), ("near.sql", "y")]);
    set_mtime(
        &lib.path("far.sql"),
        SystemTime::now() + Duration::from_secs(3_600),
    );
    set_mtime(&lib.path("near.sql"), SystemTime::now() + ms(900));
    let mut r = lib.rec();
    assert_eq!(
        run(
            &mut r,
            vec![lib.hint("far.sql"), lib.hint("near.sql")],
            t0()
        ),
        vec![WatchEvent::PossiblyChanged(rel("near.sql"))]
    );
}

// m-fr1-4: a hint through a folder swapped for a link.

#[cfg(unix)]
#[test]
fn a_hint_through_a_linked_folder_triggers_a_scan_and_so_a_sync() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), lib.path("swapped")).unwrap();
    let mut r = lib.rec();
    r.push(lib.hint("swapped/notes.txt"), t0());
    assert_eq!(r.tick(t0()), vec![]);
    assert!(
        r.last_tick().scanned,
        "the folder watches are re-synced after a scan"
    );
}

// m-fr1-5: the backstop re-arms a silently stopped source.

#[test]
fn a_backstop_look_finding_an_unannounced_change_asks_for_a_rearm() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec_with(WatchOpts {
        backstop: Some(Duration::from_secs(30)),
        ..WatchOpts::default()
    });
    r.start_backstop(t0());
    lib.write("a.sql", "xyz");
    let at = t0() + Duration::from_secs(30);
    let _ = r.tick(at);
    assert!(r.last_tick().rearm);
    // A change that a hint announced is not a reason to re-arm.
    lib.write("a.sql", "xyzw");
    r.push(lib.hint("a.sql"), at + ms(500));
    let _ = r.tick(at + ms(500));
    let at = at + Duration::from_secs(30);
    let _ = r.tick(at);
    assert!(!r.last_tick().rearm);
}

#[test]
fn the_watcher_rearms_a_source_that_stopped_silently() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let watches = Arc::new(Mutex::new(0usize));
    struct Counted(FakeEventSource, Arc<Mutex<usize>>);
    impl EventSource for Counted {
        fn watch(
            &mut self,
            root: &Path,
            sink: hacivat_scripts::HintSink,
        ) -> Result<(), WatchError> {
            *self.1.lock().unwrap() += 1;
            self.0.watch(root, sink)
        }
        fn unwatch(&mut self, root: &Path) {
            self.0.unwatch(root)
        }
    }
    let (fake, _handle) = FakeEventSource::new();
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(
        lib.root.clone(),
        WatchOpts {
            backstop: Some(ms(50)),
            ..WatchOpts::default()
        },
        tx,
        Box::new(Counted(fake, watches.clone())),
    )
    .unwrap();
    // The source never reports this write.
    lib.write("b.sql", "y");
    ev.wait_for("rearmed", |e| e.contains(&WatchEvent::BackendRearmed));
    ev.wait_for("created", |e| {
        e.contains(&WatchEvent::Created(rel("b.sql")))
    });
    assert!(*watches.lock().unwrap() >= 2);
}

// ---------------------------------------------------------------------------------------------
// ## m4b-t09-watcher fix round 3 (re-review I-fr2-1, m-fr2-1).

/// P14 (re-review probe), on a fake clock: a checkout that takes 4 s (one changed file per
/// millisecond, 4,000 files, never a quiet window) is only rescans, never per-file events,
/// except for the one file a tab has open (R-T9-6).
#[test]
fn slow_checkout_per_file_events() {
    let names: Vec<String> = (0..4_000).map(|i| format!("many/f{i:04}.sql")).collect();
    let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "SELECT 1;")).collect();
    let lib = Lib::new(&files);
    let mut r = lib.rec();
    r.remember(&rel("many/f2500.sql"), &lib.read_stamp("many/f2500.sql"));
    let start = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    for (i, n) in names.iter().enumerate() {
        lib.write(n, "SELECT 2; -- topic");
        let at = start + ms(i as u64);
        r.push(lib.hint(n), at);
        if i % 10 == 0 {
            out.extend(r.tick(at));
        }
    }
    out.extend(drain(&mut r, start + Duration::from_secs(10)));
    let rescans = out.iter().filter(|e| **e == WatchEvent::Rescan).count();
    let per_file: Vec<&WatchEvent> = out.iter().filter(|e| **e != WatchEvent::Rescan).collect();
    assert!(rescans >= 1, "{out:?}");
    assert_eq!(
        per_file,
        vec![&WatchEvent::Changed(rel("many/f2500.sql"))],
        "rescans={rescans}"
    );
}

/// m-fr2-1 (P12's shape): FSEvents can deliver one write in two batches. The second hint,
/// for an unchanged stamp still within R-T9-3's window, is `PossiblyChanged` when no tab has
/// the file open; once the consumer remembers what it re-read, it is nothing.
#[test]
fn a_second_hint_for_the_same_write_is_possibly_changed_unless_remembered() {
    for remembered in [false, true] {
        let lib = Lib::new(&[("a.sql", "SELECT 1;")]);
        let mut r = lib.rec();
        let start = Instant::now() + Duration::from_secs(5);
        lib.write("a.sql", "SELECT 1; SELECT 2;");
        let first = run(&mut r, vec![lib.hint("a.sql")], start);
        assert_eq!(first, vec![WatchEvent::Changed(rel("a.sql"))]);
        if remembered {
            // The service reloads the tab and remembers what it read.
            r.remember(&rel("a.sql"), &lib.read_stamp("a.sql"));
        }
        let second = run(&mut r, vec![lib.hint("a.sql")], start + ms(800));
        let want = if remembered {
            vec![]
        } else {
            vec![WatchEvent::PossiblyChanged(rel("a.sql"))]
        };
        assert_eq!(second, want, "remembered={remembered}");
    }
}

// ## m4b-t10-service fix round 3 (G2, R-T10-6; a Task 9 named exception): the library's
// manifest, `.hacivat/workspace.toml`, is reported on every backend.

#[test]
fn a_manifest_change_is_reported_and_nothing_else_under_the_folder() {
    let lib = Lib::new(&[("a.sql", "x"), (".hacivat/workspace.toml", "format = 1\n")]);
    let mut r = lib.rec();
    lib.write(
        ".hacivat/workspace.toml",
        "format = 1\ndatabase = \"billing\"\n",
    );
    lib.write(".hacivat/notes.sql", "n");
    let hints = vec![
        lib.hint(".hacivat/workspace.toml"),
        lib.hint(".hacivat/notes.sql"),
        lib.hint(".hidden/workspace.toml"),
    ];
    assert_eq!(run(&mut r, hints, t0()), vec![WatchEvent::ManifestChanged]);
}

#[test]
fn a_new_or_removed_manifest_folder_is_a_manifest_change() {
    let lib = Lib::new(&[("a.sql", "x")]);
    let mut r = lib.rec();
    lib.write(".hacivat/workspace.toml", "format = 1\n");
    let out = run(&mut r, vec![lib.hint(".hacivat")], t0());
    assert!(out.contains(&WatchEvent::ManifestChanged), "{out:?}");
}

#[cfg(unix)]
#[test]
fn the_manifest_folder_is_watched_folder_by_folder() {
    let lib = Lib::new(&[
        ("a.sql", "x"),
        (".hacivat/workspace.toml", "format = 1\n"),
        (".other/y.sql", "y"),
    ]);
    let dirs = lib.rec().watch_dirs().unwrap();
    assert!(dirs.contains(&lib.path(".hacivat")), "{dirs:?}");
    assert!(!dirs.contains(&lib.path(".other")), "{dirs:?}");
    // A linked `.hacivat` is not a folder of the library.
    let linked = Lib::new(&[("a.sql", "x")]);
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), linked.path(".hacivat")).unwrap();
    let dirs = linked.rec().watch_dirs().unwrap();
    assert!(!dirs.contains(&linked.path(".hacivat")), "{dirs:?}");
}

#[test]
fn poll_pass_reports_manifest_changes() {
    use hacivat_scripts::watch::PollEventSource;
    let lib = Lib::new(&[("a.sql", "x"), (".hacivat/workspace.toml", "format = 1\n")]);
    let src = PollEventSource::with_interval(ms(20)).unwrap();
    let (tx, mut ev) = Events::new();
    let _w = Watcher::start(lib.root.clone(), WatchOpts::default(), tx, Box::new(src)).unwrap();
    // Once a pass has seen the sentinel, the stamps are recorded; a pull rewrites the manifest.
    lib.write("sentinel.sql", "s");
    ev.wait_for("sentinel", |e| {
        e.contains(&WatchEvent::Created(rel("sentinel.sql")))
    });
    lib.write(
        ".hacivat/workspace.toml",
        "format = 1\ndatabase = \"billing\"\n",
    );
    ev.wait_for("manifest", |e| e.contains(&WatchEvent::ManifestChanged));
}

// ---------------------------------------------------------------------------------------------
// ## m4b-cifix-2-repo-op (review I1, R-CI2-3): a repository operation is one burst.

/// A library of `n` known scripts (`many/f0000.sql` …) and `open.sql`, all an hour old (none
/// racy), with a real `.git` folder.
fn repo_lib(n: usize) -> Lib {
    let lib = Lib::new(&[]);
    for i in 0..n {
        let name = format!("many/f{i:04}.sql");
        lib.write(&name, "SELECT 1;");
        age(&lib.path(&name), 3_600);
    }
    lib.write("open.sql", "SELECT 1;");
    age(&lib.path("open.sql"), 3_600);
    fs::create_dir_all(lib.path(".git")).unwrap();
    lib
}

/// Rewrites the files of `range` and hands their hints in at `at`.
fn rewrite(r: &mut Reconciler, lib: &Lib, range: std::ops::Range<usize>, at: Instant) {
    for i in range {
        let name = format!("many/f{i:04}.sql");
        lib.write(&name, "SELECT 2; -- topic");
        r.push(lib.hint(&name), at);
    }
}

fn per_file(events: &[WatchEvent]) -> usize {
    events
        .iter()
        .filter(|e| !matches!(e, WatchEvent::Rescan | WatchEvent::RepositoryChanged))
        .count()
}

/// The CI flake's shape, deterministic: a checkout of 1,200 files whose events pause for more
/// than a window after the first 600 (FSEvents under load). While `.git/index.lock` exists
/// the burst counts across the pause and nothing unopened settles; when the lock goes, the
/// whole operation is one `Rescan`. Without the lock (the control) the pause splits it into
/// two bursts under the limit, and every file is reported one by one.
#[test]
fn a_checkout_that_pauses_mid_way_is_one_rescan_while_the_index_is_locked() {
    // The control: no lock.
    let lib = repo_lib(1_200);
    let mut r = lib.rec();
    let t = t0();
    rewrite(&mut r, &lib, 0..600, t);
    let mut out = r.tick(t + ms(10));
    out.extend(r.tick(t + ms(200)));
    rewrite(&mut r, &lib, 600..1_200, t + ms(350));
    out.extend(drain(&mut r, t + ms(360)));
    assert_eq!(per_file(&out), 1_200, "the control reports every file");
    assert!(!out.contains(&WatchEvent::Rescan));

    // The same checkout under `index.lock`, with an open file edited mid-way.
    let lib = repo_lib(1_200);
    let mut r = lib.rec();
    r.remember(&rel("open.sql"), &lib.read_stamp("open.sql"));
    let t = t0() + Duration::from_secs(100);
    lib.write(".git/index.lock", "");
    r.push(lib.hint(".git/index.lock"), t);
    rewrite(&mut r, &lib, 0..600, t + ms(5));
    lib.write("open.sql", "SELECT 'edited while git ran';");
    r.push(lib.hint("open.sql"), t + ms(5));
    let mut out = r.tick(t + ms(10));
    // The pause: more than a window with nothing new. Only the open file settles (R-T9-6).
    out.extend(r.tick(t + ms(200)));
    out.extend(r.tick(t + ms(340)));
    assert_eq!(out, vec![WatchEvent::Changed(rel("open.sql"))]);
    rewrite(&mut r, &lib, 600..1_200, t + ms(350));
    out.extend(r.tick(t + ms(360)));
    out.extend(r.tick(t + ms(600)));
    assert_eq!(
        per_file(&out),
        1,
        "nothing unopened settles under the lock: {out:?}"
    );
    // git drops the lock: one rescan for the whole operation.
    fs::remove_file(lib.path(".git/index.lock")).unwrap();
    r.push(lib.hint(".git/index.lock"), t + ms(700));
    assert_eq!(drain(&mut r, t + ms(700)), vec![WatchEvent::Rescan]);
}

/// An operation under the limit: when the lock goes, the files it held settle and are
/// reported; a lock gone with no hint saying so is seen by the re-look a window later.
#[test]
fn a_small_operation_releases_its_files_when_the_lock_goes_unheard() {
    let lib = repo_lib(10);
    let mut r = lib.rec();
    let t = t0() + Duration::from_secs(200);
    lib.write(".git/index.lock", "");
    r.push(lib.hint(".git/index.lock"), t);
    rewrite(&mut r, &lib, 0..10, t + ms(5));
    let mut out = r.tick(t + ms(10));
    out.extend(r.tick(t + ms(400)));
    assert_eq!(per_file(&out), 0, "{out:?}");
    fs::remove_file(lib.path(".git/index.lock")).unwrap();
    out.extend(drain(&mut r, t + ms(500)));
    let changed = out
        .iter()
        .filter(|e| matches!(e, WatchEvent::Changed(_)))
        .count();
    assert_eq!(changed, 10, "{out:?}");
    assert!(!out.contains(&WatchEvent::Rescan));
}

/// A stale lock (a crashed git) holds one operation for at most `REPO_OP_MAX`, then never
/// again until it has been seen gone, and is not looked at every window: later edits settle
/// as usual.
#[test]
fn a_stale_index_lock_holds_at_most_once_for_the_bound() {
    use hacivat_scripts::watch::REPO_OP_MAX;
    let lib = repo_lib(1_200);
    let mut r = lib.rec();
    let t = t0() + Duration::from_secs(300);
    lib.write(".git/index.lock", "");
    r.push(lib.hint(".git/index.lock"), t);
    rewrite(&mut r, &lib, 0..1_200, t + ms(5));
    assert_eq!(per_file(&r.tick(t + ms(10))), 0);
    assert_eq!(r.tick(t + ms(5_000)), vec![]);
    // At the bound the overflowed operation is one rescan.
    assert_eq!(r.tick(t + REPO_OP_MAX), vec![WatchEvent::Rescan]);
    assert_eq!(drain(&mut r, t + REPO_OP_MAX + ms(1)), vec![]);
    assert_eq!(
        r.next_due(),
        None,
        "a stale lock is not looked at every window"
    );
    // The lock is still there; another edit is a burst of one and settles as usual.
    let later = t + REPO_OP_MAX + Duration::from_secs(5);
    lib.write("open.sql", "SELECT 3;");
    r.push(lib.hint(".git/index.lock"), later);
    r.push(lib.hint("open.sql"), later);
    assert_eq!(
        drain(&mut r, later),
        vec![WatchEvent::Changed(rel("open.sql"))]
    );
}
// ## m4b-cifix-2-repo-op
