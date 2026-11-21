//! HV-SCRIPT-11: the status agrees with the git CLI, path for path, on fixture repositories this
//! test builds (`alg/28` §0.4 and §0.7 U-01, gate E5), and the rest of the status contract that
//! needs a real repository: unknown overrides, the index never written, redirection, formats,
//! the timed worker.
//!
//! The git CLI runs here only to build fixtures in this test's own temp directories and to
//! compute the oracle (`NoGitSubprocess` exempts `tests/`, R-T1a-1). The product crate never
//! starts a program.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use hacivat_git::{FileStatus, GitError, GitLimits, GitState, UnknownWhy, status};

// ---------------------------------------------------------------------------------------------
// Fixture support (the git CLI on this test's own temp directories).
// ---------------------------------------------------------------------------------------------

/// Prints `HV-FACT <line>` straight to stdout: libtest captures only the `print!` family, so
/// the line reaches the CI log of a passing test too (R-W3-14).
fn fact(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(format!("HV-FACT {line}\n").as_bytes());
    let _ = out.flush();
}

/// `(major, minor)` of the git CLI on `PATH`, and its version text.
fn cli_version() -> &'static (u32, u32, String) {
    static V: OnceLock<(u32, u32, String)> = OnceLock::new();
    V.get_or_init(|| {
        let out = Command::new("git")
            .arg("--version")
            .output()
            .expect("the git CLI is needed to build fixtures");
        let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let v = text.split_whitespace().nth(2).unwrap_or("0.0").to_owned();
        let mut it = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        (it.next().unwrap_or(0), it.next().unwrap_or(0), v)
    })
}

/// Whether the CLI is new enough (git >= 2.30) for the oracle; prints the skip line otherwise.
fn cli_ok(test: &str) -> bool {
    let (maj, min, v) = cli_version();
    let ok = (*maj, *min) >= (2, 30);
    if !ok {
        fact(&format!(
            "t07 {test} skipped git_version={v} (needs >= 2.30)"
        ));
    }
    ok
}

/// The canonical path, without Windows' verbatim `\\?\` prefix (which the git CLI does not
/// take; the product canonicalizes on its own).
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

/// A fixture repository: `dir` holds the worktree, `env` the CLI's empty home and global config
/// (so the repository's own tree stays clean) and any sibling directories a fixture needs.
struct Repo {
    dir: tempfile::TempDir,
    env: tempfile::TempDir,
    /// The worktree the library and the oracle use (`dir`, or a linked worktree or clone).
    work: PathBuf,
    /// The library, relative to `work` (`""` is the worktree itself).
    lib: String,
}

impl Repo {
    fn init() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let env = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(env.path().join("home")).unwrap();
        std::fs::write(env.path().join("global.cfg"), "").unwrap();
        let work = canonical(dir.path());
        let r = Repo {
            dir,
            env,
            work,
            lib: String::new(),
        };
        r.git(&["init", "-q", "-b", "main"]);
        r
    }

    fn root(&self) -> PathBuf {
        canonical(self.dir.path())
    }

    fn library(&self) -> PathBuf {
        if self.lib.is_empty() {
            self.work.clone()
        } else {
            self.work.join(&self.lib)
        }
    }

    fn index_path(&self) -> PathBuf {
        self.root().join(".git").join("index")
    }

    /// The CLI with an isolated environment: no global or system config, a fixed identity.
    fn cmd_in(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut c = Command::new("git");
        c.current_dir(cwd).args(args);
        for k in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_COUNT",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
        ] {
            c.env_remove(k);
        }
        let home = self.env.path().join("home");
        c.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", self.env.path().join("xdg"))
            .env("GIT_CONFIG_GLOBAL", self.env.path().join("global.cfg"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .env("GIT_TERMINAL_PROMPT", "0");
        c
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> Vec<u8> {
        let out = self.cmd_in(cwd, args).output().expect("run the git CLI");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        self.git_in(&self.root(), args)
    }

    /// Runs the CLI where failure is the point (a conflicting merge).
    fn git_may_fail(&self, args: &[&str]) {
        let _ = self
            .cmd_in(&self.root(), args)
            .output()
            .expect("run the git CLI");
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.root().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn commit_all(&self) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--no-verify", "-m", "fixture"]);
    }

    fn status(&self) -> GitState {
        status(&self.library(), &GitLimits::default())
    }

    /// `git status --porcelain=v2 -z --untracked-files=all` in the worktree, mapped as the
    /// product maps its own answer (`alg/28` §0.4): Conflicted > New > Modified; deleted paths
    /// (absent from the disk) absent.
    fn oracle(&self) -> BTreeMap<String, String> {
        let mut args = vec![
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
        ];
        if !self.lib.is_empty() {
            args.push("--");
            args.push(&self.lib);
        }
        let raw = self.git_in(&self.work, &args);
        let mut out = BTreeMap::new();
        let mut recs = raw.split(|b| *b == 0).filter(|r| !r.is_empty());
        while let Some(rec) = recs.next() {
            let rec = String::from_utf8(rec.to_vec()).expect("UTF-8 fixture paths");
            let (badge, path) = match rec.as_bytes()[0] {
                b'1' => {
                    let f: Vec<&str> = rec.splitn(9, ' ').collect();
                    (xy_badge(f[1]), f[8].to_owned())
                }
                b'2' => {
                    let f: Vec<&str> = rec.splitn(10, ' ').collect();
                    let _orig = recs.next();
                    (xy_badge(f[1]), f[9].to_owned())
                }
                b'u' => {
                    let f: Vec<&str> = rec.splitn(11, ' ').collect();
                    (Some("Conflicted"), f[10].to_owned())
                }
                b'?' => (Some("New"), rec[2..].to_owned()),
                _ => (None, String::new()),
            };
            let Some(badge) = badge else { continue };
            let rel = if self.lib.is_empty() {
                path
            } else {
                match path.strip_prefix(&format!("{}/", self.lib)) {
                    Some(r) => r.to_owned(),
                    None => continue,
                }
            };
            let Some(disk) = on_disk(&self.library(), &rel) else {
                continue;
            };
            let e = out.entry(disk).or_insert_with(|| badge.to_owned());
            if rank(badge) > rank(e) {
                *e = badge.to_owned();
            }
        }
        out
    }
}

fn rank(b: &str) -> u8 {
    match b {
        "Conflicted" => 3,
        "New" => 2,
        _ => 1,
    }
}

/// A porcelain v2 `XY` pair as a badge: an added, renamed or copied index entry (or an
/// intent-to-add) is New, a modification or type change on either side is Modified.
fn xy_badge(xy: &str) -> Option<&'static str> {
    let (x, y) = (xy.as_bytes()[0], xy.as_bytes()[1]);
    if matches!(x, b'A' | b'R' | b'C') || y == b'A' {
        Some("New")
    } else if matches!(x, b'M' | b'T') || matches!(y, b'M' | b'T') {
        Some("Modified")
    } else {
        None
    }
}

/// `rel` (as the CLI spells it, precomposed on macOS) as the disk spells it, component by
/// component; `None` when the path is not on the disk.
fn on_disk(base: &Path, rel: &str) -> Option<String> {
    let mut dir = base.to_path_buf();
    let mut parts = Vec::new();
    for comp in rel.split('/') {
        let want = gix::utils::str::precompose(comp.into()).into_owned();
        let found = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .find(|n| n == comp || gix::utils::str::precompose(n.as_str().into()) == want)?;
        dir.push(&found);
        parts.push(found);
    }
    std::fs::symlink_metadata(&dir).ok()?;
    Some(parts.join("/"))
}

/// Our answer as `path -> badge` text, the shape the oracle has.
fn ours(st: &GitState) -> BTreeMap<String, String> {
    let GitState::Ready(s) = st else {
        panic!("expected Ready, got {st:?}");
    };
    s.by_path
        .iter()
        .map(|(k, v)| {
            let b = match v {
                FileStatus::Modified => "Modified".to_owned(),
                FileStatus::New => "New".to_owned(),
                FileStatus::Conflicted => "Conflicted".to_owned(),
                FileStatus::Unknown(w) => format!("Unknown({w:?})"),
            };
            (k.as_str().to_owned(), b)
        })
        .collect()
}

fn index_bytes(repo: &Repo) -> Vec<u8> {
    std::fs::read(repo.index_path()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// The fixtures of `alg/28` §0.4 (18) plus a linked worktree and a `clone --shared` (n2).
// ---------------------------------------------------------------------------------------------

const FIXTURES: [&str; 20] = [
    "clean",
    "worktree-modified",
    "index-modified",
    "untracked",
    "index-added",
    "deleted",
    "staged-rename",
    "conflict",
    "autocrlf-crlf",
    "library-is-subfolder",
    "nested-untracked-dir",
    "ignored",
    "mode-change",
    "added-then-modified",
    "same-size-same-second",
    "touched-unchanged",
    "unicode-names",
    "symlink-typechange",
    "linked-worktree",
    "shared-clone",
];

/// The NFD spelling of `café` and `über` (decomposed accents), as macOS keeps such names.
const NFD_CAFE: &str = "cafe\u{301}.sql";
const NFD_UBER: &str = "u\u{308}ber";

fn base(r: &Repo) {
    r.write("a.sql", b"select 1;\n");
    r.write("b.sql", b"select 2;\n");
    r.write("reports/c.sql", b"select 3;\n");
    r.commit_all();
}

fn build_fixture(name: &str) -> Repo {
    let mut r = Repo::init();
    match name {
        "clean" => base(&r),
        "worktree-modified" => {
            base(&r);
            r.write("a.sql", b"select 1, 2, 3;\n");
        }
        "index-modified" => {
            base(&r);
            r.write("a.sql", b"select 10;\n");
            r.git(&["add", "a.sql"]);
        }
        "untracked" => {
            base(&r);
            r.write("new.sql", b"select 4;\n");
            r.write("fresh/n.sql", b"select 5;\n");
        }
        "index-added" => {
            base(&r);
            r.write("new.sql", b"select 4;\n");
            r.git(&["add", "new.sql"]);
        }
        "deleted" => {
            base(&r);
            std::fs::remove_file(r.root().join("a.sql")).unwrap();
            r.git(&["rm", "-q", "--cached", "b.sql"]);
            std::fs::remove_file(r.root().join("b.sql")).unwrap();
        }
        "staged-rename" => {
            base(&r);
            r.git(&["mv", "a.sql", "renamed.sql"]);
        }
        "conflict" => {
            base(&r);
            r.git(&["checkout", "-q", "-b", "other"]);
            r.write("a.sql", b"select 'theirs';\n");
            r.commit_all();
            r.git(&["checkout", "-q", "main"]);
            r.write("a.sql", b"select 'ours';\n");
            r.commit_all();
            r.git_may_fail(&["merge", "-q", "--no-edit", "other"]);
        }
        "autocrlf-crlf" => {
            r.git(&["config", "core.autocrlf", "true"]);
            r.write("a.sql", b"select 1;\r\nselect 2;\r\n");
            r.write("b.sql", b"select 3;\r\n");
            r.commit_all();
            r.write("a.sql", b"select 1;\r\nselect 22;\r\n");
        }
        "library-is-subfolder" => {
            r.write("lib/a.sql", b"select 1;\n");
            r.write("lib/b.sql", b"select 2;\n");
            r.write("outside.sql", b"select 3;\n");
            r.commit_all();
            r.write("lib/a.sql", b"select 11;\n");
            r.write("lib/n.sql", b"select 4;\n");
            r.write("outside.sql", b"select 33;\n");
            r.write("other.sql", b"select 5;\n");
            r.lib = "lib".into();
        }
        "nested-untracked-dir" => {
            base(&r);
            r.write("deep/er/est/z.sql", b"select 6;\n");
            r.write("deep/y.sql", b"select 7;\n");
        }
        "ignored" => {
            r.write(".gitignore", b"secret.sql\nbuild/\n");
            base(&r);
            r.write("secret.sql", b"select 8;\n");
            r.write("build/out.sql", b"select 9;\n");
            r.write("visible.sql", b"select 10;\n");
        }
        "mode-change" => {
            base(&r);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let p = r.root().join("a.sql");
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        "added-then-modified" => {
            base(&r);
            r.write("new.sql", b"select 4;\n");
            r.git(&["add", "new.sql"]);
            r.write("new.sql", b"select 44;\n");
        }
        "same-size-same-second" => {
            base(&r);
            // Same length, right after the commit: the racy-timestamp case.
            r.write("a.sql", b"select 9;\n");
        }
        "touched-unchanged" => {
            base(&r);
            let f = std::fs::File::options()
                .write(true)
                .open(r.root().join("a.sql"))
                .unwrap();
            let later = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
            f.set_modified(later).unwrap();
        }
        "unicode-names" => {
            base(&r);
            r.write(NFD_CAFE, b"select 1;\n");
            r.write(&format!("{NFD_UBER}/x.sql"), b"select 2;\n");
            r.commit_all();
            r.write(NFD_CAFE, b"select 11;\n");
            r.write(&format!("{NFD_UBER}/x.sql"), b"select 22;\n");
        }
        "symlink-typechange" => {
            base(&r);
            #[cfg(unix)]
            {
                let p = r.root().join("a.sql");
                std::fs::remove_file(&p).unwrap();
                std::os::unix::fs::symlink("b.sql", &p).unwrap();
            }
        }
        "linked-worktree" => {
            base(&r);
            let wt = canonical(r.env.path()).join("wt");
            r.git(&["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()]);
            std::fs::write(wt.join("a.sql"), b"select 'in the linked worktree';\n").unwrap();
            std::fs::write(wt.join("w.sql"), b"select 12;\n").unwrap();
            r.work = wt;
        }
        "shared-clone" => {
            base(&r);
            let clone = canonical(r.env.path()).join("clone");
            r.git(&[
                "clone",
                "-q",
                "--shared",
                r.root().to_str().unwrap(),
                clone.to_str().unwrap(),
            ]);
            assert!(clone.join(".git/objects/info/alternates").is_file());
            std::fs::write(clone.join("a.sql"), b"select 'in the shared clone';\n").unwrap();
            r.work = clone;
        }
        other => panic!("unknown fixture {other}"),
    }
    r
}

#[test]
fn status_matches_git_cli() {
    if !cli_ok("status_matches_git_cli") {
        return;
    }
    let mut equal = 0;
    for fx in FIXTURES {
        let repo = build_fixture(fx);
        let before = index_bytes(&repo);
        // Ours first: the CLI may refresh the index (never with --no-optional-locks, but the
        // order also keeps the index check meaningful).
        let got = ours(&repo.status());
        assert_eq!(index_bytes(&repo), before, "{fx}: the index changed");
        let want = repo.oracle();
        assert_eq!(got, want, "{fx}");
        equal += 1;
    }
    let (_, _, v) = cli_version();
    fact(&format!(
        "t07 git_version={v} fixtures={} equal={equal}",
        FIXTURES.len()
    ));
}

/// The scan keys a file by the bytes the file system gives (NFD on macOS when the name was
/// written decomposed) while the index may hold the precomposed name: the badge's key must be
/// the scan's spelling, byte for byte (R-T1a-2 carry).
#[test]
fn nfd_named_modified_file_has_badge() {
    let repo = build_fixture("unicode-names");
    let st = repo.status();
    let GitState::Ready(s) = &st else {
        panic!("expected Ready, got {st:?}");
    };
    let listed: Vec<String> = std::fs::read_dir(repo.library())
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().into_string().ok())
        .collect();
    let cafe = listed
        .iter()
        .find(|n| n.ends_with(".sql") && n.starts_with("cafe"))
        .expect("the NFD file is listed");
    let uber = listed
        .iter()
        .find(|n| n.ends_with("ber"))
        .expect("the NFD folder is listed");
    assert_eq!(
        s.by_path.get(cafe.as_str()),
        Some(&FileStatus::Modified),
        "key {cafe:?} in {:?}",
        s.by_path
    );
    let nested = format!("{uber}/x.sql");
    assert_eq!(
        s.by_path.get(nested.as_str()),
        Some(&FileStatus::Modified),
        "key {nested:?} in {:?}",
        s.by_path
    );
}

#[test]
fn index_is_never_written() {
    for fx in [
        "worktree-modified",
        "same-size-same-second",
        "touched-unchanged",
    ] {
        let repo = build_fixture(fx);
        let before = index_bytes(&repo);
        assert!(!before.is_empty());
        let _ = repo.status();
        let _ = repo.status();
        assert_eq!(index_bytes(&repo), before, "{fx}");
        assert!(!repo.root().join(".git/index.lock").exists(), "{fx}");
    }
}

#[test]
fn filtered_path_is_unknown() {
    let repo = build_fixture("clean");
    repo.write(
        ".gitattributes",
        b"a.sql filter=x\nb.sql filter=lfs\nreports/* -filter\n",
    );
    repo.commit_all();
    repo.write("a.sql", b"select 'changed';\n");
    repo.write("n.sql", b"select 4;\n");
    let st = repo.status();
    let GitState::Ready(s) = &st else {
        panic!("expected Ready, got {st:?}");
    };
    // Changed and filtered: never Modified.
    assert_eq!(
        s.by_path.get("a.sql"),
        Some(&FileStatus::Unknown(UnknownWhy::Filtered))
    );
    // Clean but filtered (an LFS pointer): its bytes are not what the index compares.
    assert_eq!(
        s.by_path.get("b.sql"),
        Some(&FileStatus::Unknown(UnknownWhy::Filtered))
    );
    // `-filter` unsets it: clean, no entry.
    assert_eq!(s.by_path.get("reports/c.sql"), None);
    assert_eq!(s.by_path.get("n.sql"), Some(&FileStatus::New));
}

#[test]
fn submodule_root_is_unknown() {
    let sub = Repo::init();
    base(&sub);
    let repo = build_fixture("clean");
    repo.git(&[
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        sub.root().to_str().unwrap(),
        "mods/sub",
    ]);
    repo.commit_all();
    std::fs::write(repo.root().join("mods/sub/a.sql"), b"changed inside\n").unwrap();
    let st = repo.status();
    let GitState::Ready(s) = &st else {
        panic!("expected Ready, got {st:?}");
    };
    assert_eq!(
        s.by_path.get("mods/sub"),
        Some(&FileStatus::Unknown(UnknownWhy::Submodule)),
        "{:?}",
        s.by_path
    );
    // Nothing inside the submodule is listed: it is not opened.
    assert!(
        !s.by_path
            .keys()
            .any(|k| k.as_str().starts_with("mods/sub/")),
        "{:?}",
        s.by_path
    );
}

/// A tracked file that status must hash (same length, new mtime) and that cannot be read: not
/// excluded, the whole call fails (`alg/28` §0.8); excluded as dataless, status is `Ready` and
/// the file is never opened (I2, n6, n7).
#[test]
fn excluded_path_is_never_opened_by_status() {
    let repo = build_fixture("clean");
    repo.write("a.sql", b"select 7;\n");
    let p = repo.root().join("a.sql");
    let f = std::fs::File::options().write(true).open(&p).unwrap();
    f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(10))
        .unwrap();
    drop(f);
    let _guard = unreadable(&p);
    let plain = repo.status();
    assert!(
        matches!(plain, GitState::Unavailable(GitError::Failed(_))),
        "not excluded: {plain:?}"
    );
    let excluded = hacivat_git::testing::status_treating_as_dataless(
        &repo.library(),
        &GitLimits::default(),
        &["a.sql"],
    );
    let GitState::Ready(s) = &excluded else {
        panic!("excluded: expected Ready, got {excluded:?}");
    };
    assert_eq!(
        s.by_path.get("a.sql"),
        Some(&FileStatus::Unknown(UnknownWhy::Dataless))
    );
}

/// Makes `p` unreadable until the guard drops: mode 000 on Unix, an open handle that shares
/// nothing on Windows (n7: no `chmod` there).
fn unreadable(p: &Path) -> impl Drop {
    struct Guard {
        #[cfg(unix)]
        path: PathBuf,
        #[cfg(windows)]
        _held: std::fs::File,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o644));
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).unwrap();
        Guard {
            path: p.to_path_buf(),
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        let held = std::fs::File::options()
            .read(true)
            .share_mode(0)
            .open(p)
            .unwrap();
        Guard { _held: held }
    }
}

#[test]
fn unsupported_repo_format_is_unavailable() {
    if !cli_ok("unsupported_repo_format_is_unavailable") {
        return;
    }
    let mut checked = 0;
    for (flag, what) in [
        ("--object-format=sha256", "sha256"),
        ("--ref-format=reftable", "reftable"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let probe = Repo::init();
        let out = probe
            .cmd_in(dir.path(), &["init", "-q", flag, "."])
            .output()
            .unwrap();
        if !out.status.success() {
            fact(&format!(
                "t07 unsupported_format {what} skipped (the CLI cannot create it)"
            ));
            continue;
        }
        std::fs::write(dir.path().join("a.sql"), b"select 1;\n").unwrap();
        let st = status(dir.path(), &GitLimits::default());
        assert!(
            matches!(st, GitState::Unavailable(GitError::Unsupported)),
            "{what}: {st:?}"
        );
        checked += 1;
    }
    fact(&format!("t07 unsupported_format checked={checked}"));
}

#[test]
fn core_worktree_in_repo_config_is_ignored() {
    let repo = build_fixture("worktree-modified");
    let elsewhere = canonical(repo.env.path()).join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("decoy-only.sql"), b"select 0;\n").unwrap();
    repo.git(&["config", "core.worktree", elsewhere.to_str().unwrap()]);
    let st = repo.status();
    let got = ours(&st);
    assert_eq!(got.get("a.sql").map(String::as_str), Some("Modified"));
    assert!(!got.contains_key("decoy-only.sql"), "{got:?}");
}

#[test]
fn linked_worktree_and_shared_clone_are_read() {
    for fx in ["linked-worktree", "shared-clone"] {
        let repo = build_fixture(fx);
        let got = ours(&repo.status());
        assert_eq!(
            got.get("a.sql").map(String::as_str),
            Some("Modified"),
            "{fx}"
        );
    }
}

/// A gitfile pointing out of the repository's tree, a `commondir` that leaves it, and
/// alternates that name a missing path or a file: `Unavailable(Redirected)`, nothing read.
#[test]
fn gitfile_and_alternates_redirection_is_unavailable() {
    // A gitfile whose target is a repository elsewhere (not a linked worktree, not a submodule).
    let other = build_fixture("clean");
    let lib = tempfile::tempdir().unwrap();
    std::fs::write(lib.path().join("a.sql"), b"select 1;\n").unwrap();
    std::fs::write(
        lib.path().join(".git"),
        format!("gitdir: {}\n", other.root().join(".git").display()),
    )
    .unwrap();
    let st = status(lib.path(), &GitLimits::default());
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "gitfile elsewhere: {st:?}"
    );

    // A `commondir` in a plain `.git` directory.
    let repo = build_fixture("clean");
    std::fs::write(
        repo.root().join(".git/commondir"),
        other.root().join(".git").to_str().unwrap(),
    )
    .unwrap();
    let st = repo.status();
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "commondir: {st:?}"
    );

    // Alternates naming a missing path, and a file.
    for target in ["missing-objects", "a-file"] {
        let repo = build_fixture("clean");
        let t = canonical(repo.env.path()).join(target);
        if target == "a-file" {
            std::fs::write(&t, b"").unwrap();
        }
        std::fs::write(
            repo.root().join(".git/objects/info/alternates"),
            format!("{}\n", t.display()),
        )
        .unwrap();
        let st = repo.status();
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "alternates {target}: {st:?}"
        );
    }
}

/// An untracked FIFO named `.gitignore` blocks gix's open forever (`alg/28` §0.8, m3): the call
/// answers `Unavailable(Timeout)` without joining the worker, and a second call while that
/// worker is stuck answers at once and starts no second one.
#[cfg(unix)]
#[test]
fn timeout_is_unavailable_not_partial() {
    let repo = build_fixture("worktree-modified");
    let fifo = repo.root().join("reports/.gitignore");
    let out = Command::new("mkfifo").arg(&fifo).output().unwrap();
    assert!(out.status.success());
    let limits = GitLimits {
        timeout: std::time::Duration::from_millis(300),
        ..GitLimits::default()
    };
    let first = status(&repo.library(), &limits);
    assert!(
        matches!(first, GitState::Unavailable(GitError::Timeout)),
        "{first:?}"
    );
    // The first worker is still blocked in the FIFO's open: no second worker, no partial answer.
    let second = status(&repo.library(), &limits);
    assert!(
        matches!(second, GitState::Unavailable(GitError::Timeout)),
        "{second:?}"
    );
    // Release the stuck worker (a writer completes its open; closing gives it end of file).
    drop(std::fs::File::options().write(true).open(&fifo).unwrap());
}

/// More changed paths than `max_entries`: no partial map (badges missing would read as clean).
#[test]
fn too_many_entries_is_unavailable_not_partial() {
    let repo = build_fixture("clean");
    for i in 0..5 {
        repo.write(&format!("n{i}.sql"), b"select 1;\n");
    }
    let limits = GitLimits {
        max_entries: 3,
        ..GitLimits::default()
    };
    let st = status(&repo.library(), &limits);
    assert!(
        matches!(st, GitState::Unavailable(GitError::Failed(_))),
        "{st:?}"
    );
    let limits = GitLimits {
        max_entries: 5,
        ..GitLimits::default()
    };
    assert_eq!(ours(&status(&repo.library(), &limits)).len(), 5);
}

fn perf_enabled() -> bool {
    std::env::var_os("HACIVAT_PERF").is_some_and(|v| v == "1")
}

#[test]
#[ignore = "perf"]
fn status_of_10k_files_under_1s() {
    if !perf_enabled() {
        return;
    }
    let repo = Repo::init();
    for i in 0..10_000 {
        repo.write(&format!("d{}/f{i}.sql", i % 100), b"select 1;\n");
    }
    repo.commit_all();
    for i in (0..10_000).step_by(100) {
        repo.write(&format!("d{}/f{i}.sql", i % 100), b"select 22;\n");
    }
    let t = std::time::Instant::now();
    let got = ours(&repo.status());
    let ms = t.elapsed().as_millis();
    fact(&format!(
        "t07 perf status_10k_ms={ms} changed={}",
        got.len()
    ));
    assert_eq!(got.len(), 100);
    assert!(ms < 1000, "{ms} ms");
}

#[test]
#[ignore = "perf"]
fn status_with_20k_excludes_under_1s() {
    if !perf_enabled() {
        return;
    }
    let repo = Repo::init();
    for i in 0..20_000 {
        repo.write(&format!("d{}/f{i}.sql", i % 100), b"select 1;\n");
    }
    repo.commit_all();
    let names: Vec<String> = (0..20_000)
        .map(|i| format!("d{}/f{i}.sql", i % 100))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let t = std::time::Instant::now();
    let st = hacivat_git::testing::status_treating_as_dataless(
        &repo.library(),
        &GitLimits::default(),
        &refs,
    );
    let ms = t.elapsed().as_millis();
    let n = match &st {
        GitState::Ready(s) => s.by_path.len(),
        other => panic!("{other:?}"),
    };
    fact(&format!("t07 perf status_20k_excludes_ms={ms} unknown={n}"));
    assert_eq!(n, 20_000);
    assert!(ms < 1000, "{ms} ms");
}
