// ## m4b-t20-adversarial
//! Task 20: more hostile repositories (plan Review Focus 4; Task 20 step 1), beside Task 7's
//! `hostile_repo.rs` and `status_differential.rs`. The rows here are the ones those do not
//! cover: a gitfile that leads outside the library and outside the home folder to a hostile
//! repository, configuration includes that live outside the repository, a 1 GiB index, and
//! indexes carrying extensions `gix` does not know.
//!
//! The git CLI builds the fixtures only, on repositories this test creates (`NoGitSubprocess`
//! exempts `tests/`). Every program a hostile config names writes its own marker; no marker
//! may exist after a status.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use hacivat_git::{GitError, GitLimits, GitState, status};

/// An `HV-FACT` line straight to stdout (libtest captures `println!`; M4 T33c).
fn fact(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("HV-FACT {line}\n").as_bytes());
    let _ = out.flush();
}

/// The canonical path, without Windows' verbatim `\\?\` prefix (the git CLI does not take it).
fn canonical(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).expect("canonical path");
    #[cfg(windows)]
    if let Some(s) = c
        .to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .filter(|s| !s.starts_with("UNC"))
    {
        return PathBuf::from(s);
    }
    c
}

fn git(cwd: &Path, args: &[&str]) {
    let empty = cwd.join("hv-t20-no-global-config");
    let mut c = Command::new("git");
    c.current_dir(cwd).args(args);
    for k in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_CONFIG_PARAMETERS",
    ] {
        c.env_remove(k);
    }
    c.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid");
    let out = c.output().expect("run the git CLI");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// A committed repository at `<top>/<name>` holding `a.sql`, changed on disk.
fn repo(top: &Path, name: &str) -> PathBuf {
    let r = top.join(name);
    std::fs::create_dir_all(&r).unwrap();
    git(&r, &["init", "-q", "-b", "main"]);
    write(&r.join("a.sql"), b"select 1;\n");
    git(&r, &["add", "-A"]);
    git(&r, &["commit", "-q", "--no-verify", "-m", "base"]);
    write(&r.join("a.sql"), b"select 'changed';\n");
    r
}

/// The shell command that leaves marker `tag` in `markers`.
fn touch(markers: &Path, tag: &str) -> String {
    let m = markers.join(tag);
    if cfg!(windows) {
        format!("cmd /C type nul > \"{}\"", m.display())
    } else {
        format!("touch '{}'", m.display())
    }
}

/// Sets `key` to `value` in the config file `file` (created if needed).
fn set(file: &Path, key: &str, value: &str) {
    let out = Command::new("git")
        .args(["config", "--file"])
        .arg(file)
        .args([key, value])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git config {key}");
}

/// Every program key a hostile config can set, each touching its own marker.
fn arm(file: &Path, markers: &Path, tag: &str) {
    for (k, v) in [
        (
            "core.fsmonitor",
            touch(markers, &format!("{tag}-fsmonitor")),
        ),
        ("filter.x.clean", touch(markers, &format!("{tag}-clean"))),
        (
            "filter.x.process",
            touch(markers, &format!("{tag}-process")),
        ),
        ("diff.external", touch(markers, &format!("{tag}-diff"))),
        ("core.sshCommand", touch(markers, &format!("{tag}-ssh"))),
        (
            "core.hooksPath",
            markers.join("hooks").display().to_string(),
        ),
    ] {
        set(file, k, &v);
    }
}

fn markers_left(markers: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(markers)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "hooks")
        .collect();
    v.sort();
    v
}

/// The plan's row: a library whose `.git` is a gitfile naming a hostile repository outside
/// the library and outside the home folder. Task 7 ruled the answer (carries-w1 T7 n2): a
/// gitfile that is neither a linked worktree nor a submodule is `Unavailable(Redirected)` (the
/// plan's earlier `Untrusted(GitdirOutside)` was not built; this asserts the ruling). Nothing
/// of the target is run, and its index is never written.
#[test]
fn a_gitfile_leading_outside_the_library_and_home_runs_nothing() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let markers = top.join("markers");
    std::fs::create_dir_all(&markers).unwrap();
    let elsewhere = repo(&top, "elsewhere");
    arm(&elsewhere.join(".git/config"), &markers, "target");
    write(&elsewhere.join(".gitattributes"), b"* filter=x diff=x\n");
    let index_before = std::fs::read(elsewhere.join(".git/index")).unwrap();
    let lib = top.join("home-is-not-here/library");
    write(&lib.join("a.sql"), b"select 1;\n");
    for gitdir in [
        elsewhere.join(".git").display().to_string(),
        // The same target, spelt relative to the library.
        "../../elsewhere/.git".to_owned(),
    ] {
        write(&lib.join(".git"), format!("gitdir: {gitdir}\n").as_bytes());
        let st = status(&lib, &GitLimits::default());
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "{gitdir}: {st:?}"
        );
    }
    assert_eq!(markers_left(&markers), Vec::<String>::new());
    assert_eq!(
        std::fs::read(elsewhere.join(".git/index")).unwrap(),
        index_before
    );
}

/// `include.path` and every `includeIf` form, naming a config **outside** the repository that
/// sets `core.fsmonitor` (and the other program keys): the status still runs nothing.
#[test]
fn includes_outside_the_repository_run_nothing() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let markers = top.join("markers");
    std::fs::create_dir_all(&markers).unwrap();
    let r = repo(&top, "repo");
    write(&r.join("eol.sql"), b"select 1;\nselect 2;\n");
    git(&r, &["add", "eol.sql"]);
    git(&r, &["commit", "-q", "--no-verify", "-m", "eol"]);
    write(&r.join("eol.sql"), b"select 1;\r\nselect 2;\r\n");
    let outside = top.join("outside.cfg");
    arm(&outside, &markers, "include");
    let cfg = r.join(".git/config");
    let inc = outside.display().to_string();
    for key in [
        "include.path",
        "includeIf.gitdir:/.path",
        "includeIf.gitdir/i:**/.path",
        "includeIf.onbranch:main.path",
        "includeIf.onbranch:**.path",
        "includeIf.hasconfig:remote.*.url:**.path",
    ] {
        set(&cfg, key, &inc);
    }
    write(&r.join(".gitattributes"), b"a.sql filter=x diff=x\n");
    // ## m4b-t20-adversarial fix round 1 (review m5): the outside config also sets a key the
    // status honours, with an effect it shows: `core.autocrlf = true` makes a CRLF copy of an
    // LF-committed file clean. Read, `eol.sql` would lose its badge. (`eol.sql` is committed
    // before the includes are armed: the git CLI itself follows them and runs fsmonitor.)
    set(&outside, "core.autocrlf", "true");
    let st = status(&r, &GitLimits::default());
    let GitState::Ready(s) = &st else {
        panic!("the status reads the repository: {st:?}");
    };
    let badge = |p: &str| s.by_path.get(&hacivat_git::GitPath::new(p)).copied();
    // A path whose attributes name a filter is never guessed.
    assert_eq!(
        badge("a.sql"),
        Some(hacivat_git::FileStatus::Unknown(
            hacivat_git::UnknownWhy::Filtered
        )),
        "{s:?}"
    );
    // Nothing of the outside config was read: the CRLF copy is still modified.
    assert_eq!(
        badge("eol.sql"),
        Some(hacivat_git::FileStatus::Modified),
        "{s:?}"
    );
    assert_eq!(markers_left(&markers), Vec::<String>::new());
    // The whole map is pinned: nothing an included file could set changed a badge.
    assert_eq!(s.by_path.len(), 3, "{s:?}");
    // Recorded (fix round 1): no control can show an include being read through a badge.
    // This pipeline's status honours none of `core.autocrlf`, `core.excludesFile` or
    // `status.showUntrackedFiles` even from the repository's own config (observed), so the
    // keys an include could bring that matter are the program keys, which the markers cover.
}

/// A 1 GiB index: never decoded (`TooLarge(Index)`, refused by its size before gix reads it),
/// never a partial map, never a panic, never rewritten. The file is sparse on APFS and ext4
/// (`set_len`); NTFS allocates the whole length, so the row is skipped on Windows (review m5).
#[test]
#[cfg_attr(
    windows,
    ignore = "NTFS set_len allocates the full 1 GiB; no sparse file without fsutil"
)]
fn a_one_gib_index_is_unavailable() {
    let top = tempfile::tempdir().unwrap();
    let r = repo(&canonical(top.path()), "repo");
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(r.join(".git/index"))
        .unwrap();
    f.set_len(1 << 30).unwrap();
    drop(f);
    let st = status(&r, &GitLimits::default());
    fact(&format!(
        "t20 one_gib_index state={}",
        format!("{st:?}").replace(' ', "")
    ));
    assert!(
        matches!(
            st,
            GitState::Unavailable(GitError::TooLarge(hacivat_git::TooLargeWhy::Index))
        ),
        "{st:?}"
    );
    // The index was never rewritten (still the sparse 1 GiB file).
    assert_eq!(
        std::fs::metadata(r.join(".git/index")).unwrap().len(),
        1 << 30
    );
}

/// The cifix-2 re-review's N3: a library inside a bigger repository gets the outer git dir's
/// `index.lock` path with its status (the watcher `lstat`s it to tell a checkout's burst); a
/// library that is the worktree, with its own `.git` folder, gets none (its watcher sees the
/// lock there). The path is the validated git dir's, never one the index or worktree names.
#[test]
fn a_library_inside_a_bigger_repository_gets_its_index_lock_path() {
    let top = tempfile::tempdir().unwrap();
    let r = repo(&canonical(top.path()), "repo");
    write(&r.join("lib/b.sql"), b"select 2;\n");
    let lock_of = |st: &GitState| match st {
        GitState::Ready(s) => s.index_lock.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(lock_of(&status(&r, &GitLimits::default())), None);
    let inner = lock_of(&status(&r.join("lib"), &GitLimits::default()));
    assert_eq!(
        inner.map(|p| canonical(p.parent().unwrap()).join(p.file_name().unwrap())),
        Some(canonical(&r.join(".git")).join("index.lock"))
    );
}

/// Task 20's own probe (from the T7 review's q2, the per-alternate cost): the alternates
/// chain was bounded in depth only. An alternates file may list many lines, and an alternate
/// may list itself, so a hostile `.git/objects/info/alternates` fans out to `lines^depth`
/// folder checks of up to 20,000 `lstat`s each: the worker outlives the caller's 5 s budget
/// and the library answers `Paused` until the app restarts. Now every alternate folder is
/// checked once (the fan-out is answered at once), and more than a few distinct ones refuse
/// the repository (`Redirected`).
#[test]
fn a_fan_out_of_alternates_is_refused_at_once() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let other = repo(&top, "other");
    let (b, c) = (repo(&top, "b"), repo(&top, "c"));
    // `repo` lists `other` 500 times, `other` lists `b` 500 times, `b` lists `c` 500 times:
    // within the depth bound, 500³ folder checks. (An alternate listing itself already
    // reaches the depth bound at once.)
    let line = format!("{}\n", other.join(".git/objects").display());
    let to = |x: &Path| format!("{}\n", x.join(".git/objects").display()).repeat(500);
    write(
        &r.join(".git/objects/info/alternates"),
        to(&other).as_bytes(),
    );
    write(
        &other.join(".git/objects/info/alternates"),
        to(&b).as_bytes(),
    );
    write(&b.join(".git/objects/info/alternates"), to(&c).as_bytes());
    let limits = GitLimits {
        timeout: std::time::Duration::from_secs(2),
        ..GitLimits::default()
    };
    let st = status(&r, &limits);
    fact(&format!(
        "t20 alternates_fan_out state={}",
        format!("{st:?}").replace(' ', "")
    ));
    // Each of the three folders is checked once: answered within the budget, never `Timeout`
    // (and so never a stuck worker that leaves the library `Paused`).
    assert!(matches!(st, GitState::Ready(_)), "{st:?}");
    // Many distinct alternates (each a real, owned object folder) are refused.
    let mut lines = String::new();
    for i in 0..20 {
        let o = repo(&top, &format!("alt{i}"));
        lines.push_str(&format!("{}\n", o.join(".git/objects").display()));
    }
    write(&r.join(".git/objects/info/alternates"), lines.as_bytes());
    let st = status(&r, &limits);
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "{st:?}"
    );
    // One alternate, as `git clone --shared` makes, is still read.
    write(&r.join(".git/objects/info/alternates"), line.as_bytes());
    write(&other.join(".git/objects/info/alternates"), b"");
    let st = status(&r, &GitLimits::default());
    assert!(matches!(st, GitState::Ready(_)), "{st:?}");
}

// ## m4b-t20-adversarial fix round 1 (review I1, the reviewer's probe H2)
/// Short alternates lines that are expensive to resolve: each one goes through a 30-link chain
/// whose every hop is an absolute path through a 150-deep folder chain. Before fix round 1
/// each line was resolved (`canonicalize`) before it was counted, so 1,000 of them took about
/// 8 s and the status answered `Timeout` (its worker went on, the library `Paused`). Now the
/// lines are counted and de-duplicated by their text before any is resolved: 1,000 copies of
/// one line resolve once, 1,000 distinct lines are refused at once. Both answer far
/// inside the default 5 s budget (never `Timeout`), and a second status (a refresh) does the
/// same bounded work again (never `Paused`).
#[cfg(unix)]
#[test]
fn many_expensive_alternates_lines_are_refused_before_any_is_resolved() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let other = repo(&top, "other");
    let mut deep = top.join("deep");
    for _ in 0..150 {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    // l0 → deep/l1 → … → deep/l29 → other's objects, each hop an absolute path.
    let hops = 30;
    for i in (1..hops).rev() {
        let target = if i == hops - 1 {
            other.join(".git/objects")
        } else {
            deep.join(format!("l{}", i + 1))
        };
        std::os::unix::fs::symlink(target, deep.join(format!("l{i}"))).unwrap();
    }
    let objects = r.join(".git/objects");
    let alternates = objects.join("info/alternates");
    let not_stuck = |what: &str| {
        for pass in ["first", "refresh"] {
            let st = status(&r, &GitLimits::default());
            fact(&format!(
                "t20 alternates_lines case={what} pass={pass} state={}",
                match &st {
                    GitState::Ready(_) => "ready".to_owned(),
                    other => format!("{other:?}").replace(' ', ""),
                }
            ));
            assert!(
                !matches!(
                    st,
                    GitState::Unavailable(GitError::Timeout | GitError::Paused)
                ),
                "{what} {pass}: {st:?}"
            );
        }
    };
    // 1,000 copies of one short line: our check resolves it once (gix's own open then refuses
    // a 30-link alternate: `Failed(Open)`, observed); answered at once either way.
    std::os::unix::fs::symlink(deep.join("l1"), objects.join("a0")).unwrap();
    write(&alternates, "a0\n".repeat(1_000).as_bytes());
    not_stuck("repeated");
    // 1,000 distinct short lines (each its own link into the chain): refused before one is
    // resolved.
    let mut lines = String::new();
    for i in 1..1_000 {
        std::os::unix::fs::symlink(deep.join("l1"), objects.join(format!("a{i}"))).unwrap();
        lines.push_str(&format!("a{i}\n"));
    }
    write(&alternates, lines.as_bytes());
    not_stuck("distinct");
    assert!(matches!(
        status(&r, &GitLimits::default()),
        GitState::Unavailable(GitError::Redirected)
    ));
}

/// The index with one more extension before its trailing SHA-1, the checksum recomputed.
fn with_extension(index: &[u8], signature: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let body = &index[..index.len() - 20];
    let mut out = body.to_vec();
    out.extend_from_slice(signature);
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    out.extend_from_slice(data);
    let mut h = gix::hash::hasher(gix::hash::Kind::Sha1);
    h.update(&out);
    let id = h.try_finalize().unwrap();
    out.extend_from_slice(id.as_bytes());
    out
}

/// Extensions `gix` does not know: a mandatory one (lowercase first letter: a reader that
/// does not know it must refuse the index) makes the status unavailable; an optional one
/// (uppercase) may be skipped, and then the badges equal those of the plain index. An
/// extension whose length runs past the end is refused. Never a panic, never the index
/// rewritten.
#[test]
fn an_index_with_an_unknown_extension_never_panics() {
    let top = tempfile::tempdir().unwrap();
    let r = repo(&canonical(top.path()), "repo");
    let plain = std::fs::read(r.join(".git/index")).unwrap();
    let baseline = format!("{:?}", status(&r, &GitLimits::default()));
    assert!(baseline.starts_with("Ready"), "{baseline}");
    let cases: [(&str, Vec<u8>); 4] = [
        ("mandatory", with_extension(&plain, b"hvzz", b"payload")),
        ("optional", with_extension(&plain, b"HVZZ", b"payload")),
        ("empty-mandatory", with_extension(&plain, b"zzzz", b"")),
        ("overlong", {
            // A size that runs past the end of the file, the checksum still right.
            let mut v = with_extension(&plain, b"HVZZ", b"x");
            let at = v.len() - 20 - 1 - 4;
            v[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            let body_len = v.len() - 20;
            let mut h = gix::hash::hasher(gix::hash::Kind::Sha1);
            h.update(&v[..body_len]);
            let id = h.try_finalize().unwrap();
            v[body_len..].copy_from_slice(id.as_bytes());
            v
        }),
    ];
    for (case, bytes) in cases {
        std::fs::write(r.join(".git/index"), &bytes).unwrap();
        let st = status(&r, &GitLimits::default());
        let shown = match &st {
            GitState::Ready(_) => "ready".to_owned(),
            other => format!("{other:?}").replace(' ', ""),
        };
        fact(&format!("t20 index_extension case={case} state={shown}"));
        match case {
            "optional" => assert!(
                matches!(st, GitState::Unavailable(_)) || format!("{st:?}") == baseline,
                "{case}: {st:?}"
            ),
            _ => assert!(matches!(st, GitState::Unavailable(_)), "{case}: {st:?}"),
        }
        assert_eq!(
            std::fs::read(r.join(".git/index")).unwrap(),
            bytes,
            "{case}: the index was rewritten"
        );
    }
}
// ## m4b-t20-adversarial
