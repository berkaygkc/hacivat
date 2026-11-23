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
    // Fix round 2 (re-review m5-fr1): the reviewer's oracle. Read, `core.bare = true` would make
    // the status `Redirected` (the control below); `Ready` proves the include was not read.
    set(&outside, "core.bare", "true");
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
    // The control (fix round 2): the same `core.bare` in the repository's own config is read,
    // and refuses the repository, so the `Ready` above means the include was not followed.
    // (`core.autocrlf`, `core.excludesFile` and `status.showUntrackedFiles` are not honoured by
    // this pipeline even from the repository's own config, observed in fix round 1.)
    set(&cfg, "core.bare", "true");
    let st = status(&r, &GitLimits::default());
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "{st:?}"
    );
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
    // Fix round 2 (re-review I1-fr1): 500 raw entries in one file are refused before anything
    // (ours or gix's store open) resolves one: `Redirected` at once, never `Timeout`.
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "{st:?}"
    );
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
    // R-T20-2 (fix round 3): an alternate with an alternates file of its own (even empty) is
    // refused; with none, one absolute alternate reads.
    std::fs::remove_file(other.join(".git/objects/info/alternates")).unwrap();
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
    // 1,000 copies of one short line: since fix round 2 over the raw-entry cap, refused before
    // anything resolves it (`Redirected`); answered at once.
    std::os::unix::fs::symlink(deep.join("l1"), objects.join("link0")).unwrap();
    write(&alternates, "link0\n".repeat(1_000).as_bytes());
    not_stuck("repeated");
    // 1,000 distinct short lines (each its own link into the chain): refused before one is
    // resolved.
    let mut lines = String::new();
    for i in 1..1_000 {
        std::os::unix::fs::symlink(deep.join("l1"), objects.join(format!("link{i}"))).unwrap();
        lines.push_str(&format!("link{i}\n"));
    }
    write(&alternates, lines.as_bytes());
    not_stuck("distinct");
    assert!(matches!(
        status(&r, &GitLimits::default()),
        GitState::Unavailable(GitError::Redirected)
    ));
}

// ## m4b-t20-adversarial fix round 2 (re-review I1-fr1, the reviewer's shape)
/// One short line repeated to 64 KiB (with whitespace variants), each a 2-hop link chain through
/// 400-deep folders: our own check resolved it once, but gix's store open re-read the raw file
/// and resolved every entry, uninterruptibly (27–113 s measured), so the status answered
/// `Timeout`, then `Paused`, on every refresh. Now the raw entries are capped before gix sees
/// the file: `Redirected` at once, on both passes. A file within the caps (16 copies) is read
/// as before.
#[cfg(unix)]
#[test]
fn a_repeated_short_alternates_line_never_reaches_gix() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let other = repo(&top, "other");
    let mut deep = top.join("deep");
    for _ in 0..400 {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    // objects/l0 → deep/l1 → other's objects: each hop an absolute path through `deep`.
    std::os::unix::fs::symlink(other.join(".git/objects"), deep.join("l1")).unwrap();
    let objects = r.join(".git/objects");
    std::os::unix::fs::symlink(deep.join("l1"), objects.join("l0")).unwrap();
    let alternates = objects.join("info/alternates");
    let mut text = String::new();
    let variants = ["l0\n", " l0\n", "l0 \n", "\tl0\n"];
    let mut i = 0;
    while text.len() + 8 < 64 * 1024 {
        text.push_str(variants[i % variants.len()]);
        i += 1;
    }
    write(&alternates, text.as_bytes());
    for pass in ["first", "refresh"] {
        let st = status(&r, &GitLimits::default());
        fact(&format!(
            "t20 alternates_repeated pass={pass} state={}",
            format!("{st:?}").replace(' ', "")
        ));
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "{pass}: {st:?}"
        );
    }
    // Within R-T20-2: 16 copies of the absolute path of that link, read (never `Timeout` or
    // `Paused`, never `Redirected`).
    write(
        &alternates,
        format!("{}\n", objects.join("l0").display())
            .repeat(16)
            .as_bytes(),
    );
    for pass in ["first", "refresh"] {
        let st = status(&r, &GitLimits::default());
        assert!(
            !matches!(
                st,
                GitState::Unavailable(GitError::Timeout | GitError::Paused | GitError::Redirected)
            ),
            "{pass}: {st:?}"
        );
    }
}

// ## m4b-t20-adversarial fix round 3 (ruling R-T20-2; the re-review's H3 and H4)

/// A folder `deep` 400 levels down, and in the root objects folder a link `name` that reaches
/// `target` in `hops` hops, each an absolute path through `deep` (the reviewer's expensive
/// shape: every resolution walks hundreds of components per hop).
#[cfg(unix)]
fn expensive_link(top: &Path, objects: &Path, name: &str, target: &Path, hops: usize) {
    let mut deep = top.join(format!("deep-{name}"));
    for _ in 0..400 {
        deep.push("d");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let mut next = target.to_path_buf();
    for i in (1..hops).rev() {
        let link = deep.join(format!("h{i}"));
        std::os::unix::fs::symlink(&next, &link).unwrap();
        next = link;
    }
    std::os::unix::fs::symlink(&next, objects.join(name)).unwrap();
}

/// An object folder whose alternates file is 64 KiB of `line`.
#[cfg(unix)]
fn folder_with_64_kib_alternates(top: &Path, name: &str, line: &str) -> PathBuf {
    let r = repo(top, name);
    let objects = r.join(".git/objects");
    let mut text = String::new();
    while text.len() + line.len() + 1 < 64 * 1024 {
        text.push_str(line);
        text.push('\n');
    }
    write(&objects.join("info/alternates"), text.as_bytes());
    objects
}

/// Two passes at the default budget: `Redirected` each time, never `Timeout` or `Paused`.
fn refused_twice(r: &Path, what: &str) {
    for pass in ["first", "refresh"] {
        let st = status(r, &GitLimits::default());
        fact(&format!(
            "t20 r_t20_2 case={what} pass={pass} state={}",
            format!("{st:?}").replace(' ', "")
        ));
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "{what} {pass}: {st:?}"
        );
    }
}

/// H3: the root file holds one entry with a trailing space, `l0 `. Git (and the old walk)
/// resolve `l0`, a harmless alternate; gix resolves `l0 ` itself, a second link, to a folder
/// whose alternates file is 64 KiB of `k0` (each a 2-hop chain through 400-deep folders). Before
/// R-T20-2: `Timeout`, then `Paused`. Now an entry that is not its own trimmed form is refused.
#[cfg(unix)]
#[test]
fn h3_an_entry_with_trailing_whitespace_is_refused() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let objects = r.join(".git/objects");
    let harmless = repo(&top, "harmless").join(".git/objects");
    let expensive = folder_with_64_kib_alternates(&top, "expensive", "k0");
    // gix joins the nested `k0` to the root objects folder (H4's finding), so `k0` lives there.
    expensive_link(&top, &objects, "k0", &harmless, 2);
    std::os::unix::fs::symlink(&harmless, objects.join("l0")).unwrap();
    std::os::unix::fs::symlink(&expensive, objects.join("l0 ")).unwrap();
    write(&objects.join("info/alternates"), b"l0 \n");
    refused_twice(&r, "h3");
    // The absolute, trimmed form of the harmless alternate (and no chain) reads.
    write(
        &objects.join("info/alternates"),
        format!("{}\n", harmless.display()).as_bytes(),
    );
    assert!(matches!(
        status(&r, &GitLimits::default()),
        GitState::Ready(_)
    ));
}

/// H4: the root lists alternate `a` (absolute). `a`'s own file says `k0`, which git joins to
/// `a`'s objects folder (an empty one) but gix joins to the **root** objects folder, where
/// `k0` links to a folder whose alternates file is 64 KiB of `k1` (`k1` a 2-hop chain through
/// 400-deep folders). Before R-T20-2: `Timeout`, then `Paused` for over a minute. Now an
/// alternate with alternates of its own is refused, its file unread.
#[cfg(unix)]
#[test]
fn h4_a_nested_alternates_file_is_refused() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let objects = r.join(".git/objects");
    let a = repo(&top, "a").join(".git/objects");
    std::fs::create_dir_all(a.join("k0")).unwrap();
    write(&a.join("info/alternates"), b"k0\n");
    let harmless = repo(&top, "harmless").join(".git/objects");
    let expensive = folder_with_64_kib_alternates(&top, "expensive", "k1");
    std::os::unix::fs::symlink(&expensive, objects.join("k0")).unwrap();
    expensive_link(&top, &objects, "k1", &harmless, 2);
    write(
        &objects.join("info/alternates"),
        format!("{}\n", a.display()).as_bytes(),
    );
    refused_twice(&r, "h4");
}

/// R-T20-2: a nested alternates file of any size is refused (an empty one, one absolute line,
/// 64 KiB); a relative entry in the root file is refused; one absolute alternate with no chain,
/// as `git clone --shared` makes, reads (`Ready`).
#[test]
fn only_single_level_absolute_alternates_are_read() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    let objects = r.join(".git/objects");
    let a = repo(&top, "a").join(".git/objects");
    let b = repo(&top, "b").join(".git/objects");
    write(
        &objects.join("info/alternates"),
        format!("{}\n", a.display()).as_bytes(),
    );
    assert!(
        matches!(status(&r, &GitLimits::default()), GitState::Ready(_)),
        "a single absolute alternate reads"
    );
    for (what, nested) in [
        ("empty", String::new()),
        ("one", format!("{}\n", b.display())),
        ("large", format!("{}\n", b.display()).repeat(1_000)),
    ] {
        write(&a.join("info/alternates"), nested.as_bytes());
        refused_twice(&r, &format!("nested-{what}"));
    }
    std::fs::remove_file(a.join("info/alternates")).unwrap();
    // A relative entry (git and gix may join it to different folders) is refused.
    write(&objects.join("info/alternates"), b"../../a/.git/objects\n");
    refused_twice(&r, "relative");
    // CR line ends (an entry that is not its own trimmed form).
    write(
        &objects.join("info/alternates"),
        format!("{}\r\n", a.display()).as_bytes(),
    );
    refused_twice(&r, "crlf");
}

// ## m4b-t20-adversarial fix round 4 (re-review I-fr3-1; Review Focus 3: badges never touch
// the network)

/// The network and device starts, each naming a host that does not exist (`.invalid` never
/// resolves): a status that touched one would wait on name resolution and SMB.
const REMOTE_STARTS: [&str; 6] = [
    r"\\hv-t20-nohost.invalid\share\objects",
    "//hv-t20-nohost.invalid/share/objects",
    r"\\?\UNC\hv-t20-nohost.invalid\share\objects",
    r"\\.\UNC\hv-t20-nohost.invalid\share\objects",
    r"\??\UNC\hv-t20-nohost.invalid\share\objects",
    r"\\?\GLOBALROOT\Device\Mup\hv-t20-nohost.invalid\share\objects",
];

/// One status of `r`: `Redirected`, and (on Windows, where the start means a host) at once.
fn refused_locally(r: &Path, what: &str) {
    let started = std::time::Instant::now();
    let st = status(r, &GitLimits::default());
    let took = started.elapsed();
    fact(&format!(
        "t20 remote_path case={what} ms={} state={}",
        took.as_millis(),
        format!("{st:?}").replace(' ', "")
    ));
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "{what}: {st:?}"
    );
    #[cfg(windows)]
    assert!(took < std::time::Duration::from_secs(2), "{what}: {took:?}");
}

/// Every repository-supplied path text × every network or device start: alternates entries,
/// a gitfile's `gitdir:`, a linked worktree's `commondir` and `gitdir` back-pointer, and config
/// values gix may open as paths (`core.worktree`, `core.excludesFile`). Each answers
/// `Redirected` before anything opens it; with each restored the repository reads again. On
/// Windows each start is an absolute path, so only the guard stands between it and SMB; the
/// same rule runs (and is tested) on every OS.
#[test]
fn no_repository_path_leaves_the_local_drives() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    git(&r, &["worktree", "add", "-q", "../wt"]);
    let wt = top.join("wt");
    let private = r.join(".git/worktrees/wt");
    let gf = top.join("gf");
    std::fs::create_dir_all(&gf).unwrap();
    let alternates = r.join(".git/objects/info/alternates");
    let config = r.join(".git/config");
    for start in REMOTE_STARTS {
        // Rust reads `\??\` as rooted, not prefixed; Win32 passes it through as an NT path.
        #[cfg(windows)]
        assert!(
            Path::new(start).is_absolute() || start.starts_with(r"\??\"),
            "{start}"
        );
        write(&alternates, format!("{start}\n").as_bytes());
        refused_locally(&r, &format!("alternates {start}"));
        std::fs::remove_file(&alternates).unwrap();

        write(&gf.join(".git"), format!("gitdir: {start}\n").as_bytes());
        refused_locally(&gf, &format!("gitfile {start}"));

        for file in ["commondir", "gitdir"] {
            let p = private.join(file);
            let kept = std::fs::read(&p).unwrap();
            write(&p, format!("{start}\n").as_bytes());
            refused_locally(&wt, &format!("{file} {start}"));
            write(&p, &kept);
        }

        for key in ["core.worktree", "core.excludesFile"] {
            let kept = std::fs::read(&config).unwrap();
            set(&config, key, start);
            refused_locally(&r, &format!("{key} {start}"));
            write(&config, &kept);
        }
    }
    // Restored, both read; a remote's URL on a share is an ordinary clone source and reads too.
    set(&config, "remote.origin.url", REMOTE_STARTS[0]);
    for lib in [&r, &wt] {
        let st = status(lib, &GitLimits::default());
        assert!(
            matches!(st, GitState::Ready(_)),
            "{}: {st:?}",
            lib.display()
        );
    }
}

/// The guard itself bites on Unix too, where the starts above would fail later anyway (no such
/// host, or not absolute): `//<a real local folder>` is a working POSIX spelling of that folder,
/// and the guard reads it as UNC. Each source, spelled `//…`, answers `Redirected`; spelled
/// `/…`, the same target reads (`Ready`). RED with `local_spelling` answering `true`.
#[cfg(unix)]
#[test]
fn the_guard_alone_refuses_a_working_double_slash_spelling() {
    let top = tempfile::tempdir().unwrap();
    let top = canonical(top.path());
    let r = repo(&top, "repo");
    git(&r, &["worktree", "add", "-q", "../wt"]);
    let wt = top.join("wt");
    let private = r.join(".git/worktrees/wt");
    let shared = repo(&top, "shared").join(".git/objects");
    // A gitfile leading to a git dir inside its own worktree (followed, n2).
    let gf = repo(&top, "gf");
    std::fs::rename(gf.join(".git"), gf.join("inner.git")).unwrap();
    let config = r.join(".git/config");
    let check = |lib: &Path, file: &Path, text: String, what: &str| {
        let kept = std::fs::read(file).ok();
        write(file, format!("/{text}\n").as_bytes());
        refused_locally(lib, &format!("twin {what}"));
        write(file, format!("{text}\n").as_bytes());
        let st = status(lib, &GitLimits::default());
        assert!(matches!(st, GitState::Ready(_)), "control {what}: {st:?}");
        match kept {
            Some(k) => write(file, &k),
            None => std::fs::remove_file(file).unwrap(),
        }
    };
    let alternates = r.join(".git/objects/info/alternates");
    check(&r, &alternates, shared.display().to_string(), "alternates");
    let gitdir = format!("gitdir: {}", gf.join("inner.git").display());
    let kept = gitdir.clone();
    // The gitfile's text starts with `gitdir: `, so the slash goes after it.
    write(
        &gf.join(".git"),
        format!("gitdir: /{}\n", gf.join("inner.git").display()).as_bytes(),
    );
    refused_locally(&gf, "twin gitfile");
    write(&gf.join(".git"), format!("{kept}\n").as_bytes());
    let st = status(&gf, &GitLimits::default());
    assert!(matches!(st, GitState::Ready(_)), "control gitfile: {st:?}");
    check(
        &wt,
        &private.join("commondir"),
        r.join(".git").display().to_string(),
        "commondir",
    );
    check(
        &wt,
        &private.join("gitdir"),
        wt.join(".git").display().to_string(),
        "gitdir",
    );
    let kept = std::fs::read(&config).unwrap();
    set(
        &config,
        "core.excludesFile",
        &format!("/{}", top.join("ignore").display()),
    );
    refused_locally(&r, "twin core.excludesFile");
    write(&config, &kept);
    set(
        &config,
        "core.excludesFile",
        &top.join("ignore").display().to_string(),
    );
    assert!(matches!(
        status(&r, &GitLimits::default()),
        GitState::Ready(_)
    ));
}

/// Re-review I1-fr1 (b), defence in depth: a worker abandoned at its deadline (a stall here,
/// as an uninterruptible read would) leaves the library `Paused` only while it runs. Once it
/// ends, the library answers `Timeout` at once for its backoff, starting no worker, and then
/// is retried and answers normally: never `Paused` for good, never a worker back to back.
#[test]
fn an_overrun_backs_off_then_retries() {
    let top = tempfile::tempdir().unwrap();
    let r = repo(&canonical(top.path()), "repo");
    let limits = GitLimits {
        timeout: std::time::Duration::from_millis(300),
        // Fix round 3 (m-fr2-1): seconds, so a loaded runner still lands inside it.
        retry_after: std::time::Duration::from_secs(3),
        ..GitLimits::default()
    };
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (ended_tx, ended) = std::sync::mpsc::channel::<()>();
    let first = hacivat_git::testing::status_with_stall(
        &r,
        &limits,
        hacivat_git::testing::Stall::AfterResolve,
        move || {
            let _ = released.recv_timeout(std::time::Duration::from_secs(10));
            let _ = ended_tx.send(());
        },
    );
    assert!(
        matches!(first, GitState::Unavailable(GitError::Timeout)),
        "{first:?}"
    );
    assert!(matches!(
        status(&r, &limits),
        GitState::Unavailable(GitError::Paused)
    ));
    release.send(()).unwrap();
    ended
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    // Later calls get a roomy budget, so a slow runner never overruns again.
    let roomy = GitLimits {
        timeout: std::time::Duration::from_secs(5),
        ..limits
    };
    // The stuck worker ended (its overrun recorded on the way out): condition wait for the
    // first answer that is not `Paused`; it is the backoff's `Timeout`, given at once.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let next = loop {
        let st = status(&r, &roomy);
        if !matches!(st, GitState::Unavailable(GitError::Paused)) {
            break st;
        }
        assert!(std::time::Instant::now() < deadline, "still Paused");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(
        matches!(next, GitState::Unavailable(GitError::Timeout)),
        "{next:?}"
    );
    // After the backoff the library is retried, and answers.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let st = status(&r, &roomy);
        if matches!(st, GitState::Ready(_)) {
            break;
        }
        assert!(
            matches!(st, GitState::Unavailable(GitError::Timeout)),
            "only the backoff's answer before the retry: {st:?}"
        );
        assert!(std::time::Instant::now() < deadline, "never retried");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
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
