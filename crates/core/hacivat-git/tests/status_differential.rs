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
        let args = vec![
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
        ];
        // The whole repository, filtered here: the CLI matches a pathspec case-sensitively,
        // which would hide a case-renamed library folder from the oracle.
        let folds = folds_case(&self.library());
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
            let Some((rel, case_differs)) = strip_lib(&path, &self.lib, folds) else {
                continue;
            };
            // R-T7-1: the index spells the library folder differently only by case.
            let badge = if case_differs {
                "Unknown(CaseMismatch)"
            } else {
                badge
            };
            // Deleted paths are absent: asked of the OS (its own case and normalisation
            // rules), not of the product's listing rule (fix round 1, I2).
            if std::fs::symlink_metadata(self.library().join(&rel)).is_err() {
                continue;
            }
            let e = out.entry(rel).or_insert_with(|| badge.to_owned());
            if rank(badge) > rank(e) {
                *e = badge.to_owned();
            }
        }
        out
    }
}

/// NFC, the one normal form both sides are compared in (macOS keeps NFD names, the index
/// holds NFC ones under `core.precomposeUnicode`).
fn nfc(s: &str) -> String {
    gix::utils::str::precompose(s.into()).into_owned()
}

/// The CLI's repository path relative to the library folder `lib`, in the CLI's spelling, and
/// whether the folder matched only by folding case. The folder is compared in NFC, and ASCII
/// case-insensitively only where the file system folds case (`folds`).
fn strip_lib(path: &str, lib: &str, folds: bool) -> Option<(String, bool)> {
    if lib.is_empty() {
        return Some((path.to_owned(), false));
    }
    let n = lib.split('/').count();
    let comps: Vec<&str> = path.split('/').collect();
    if comps.len() <= n {
        return None;
    }
    let (head, want) = (nfc(&comps[..n].join("/")), nfc(lib));
    let rest = comps[n..].join("/");
    if head == want {
        Some((rest, false))
    } else if folds && head.eq_ignore_ascii_case(&want) {
        Some((rest, true))
    } else {
        None
    }
}

/// Our answer against the CLI's, with two independent rules (fix round 1, I2):
/// - every CLI path the OS finds on disk has exactly one key of ours equal to it in NFC, with
///   the same badge; or, failing that, one equal to it ASCII case-insensitively, whose badge
///   is `Unknown(CaseMismatch)`; and we have no key beyond those;
/// - every key of ours is the disk's own spelling, component by component.
fn assert_matches_cli(label: &str, st: &GitState, repo: &Repo) {
    let got = ours(st);
    let want = repo.oracle();
    let mut matched = std::collections::BTreeSet::new();
    for (p, b) in &want {
        let exact: Vec<&String> = got.keys().filter(|k| nfc(k) == nfc(p)).collect();
        if let [k] = exact.as_slice() {
            assert_eq!(&got[*k], b, "{label}: {p}");
            matched.insert((*k).clone());
            continue;
        }
        let folded: Vec<&String> = got
            .keys()
            .filter(|k| nfc(k).eq_ignore_ascii_case(&nfc(p)))
            .collect();
        match folded.as_slice() {
            [k] => {
                assert_eq!(got[*k], "Unknown(CaseMismatch)", "{label}: {p} as {k}");
                matched.insert((*k).clone());
            }
            _ => panic!("{label}: the CLI lists {p} ({b}); ours has no key for it: {got:?}"),
        }
    }
    let extra: Vec<&String> = got.keys().filter(|k| !matched.contains(*k)).collect();
    assert!(
        extra.is_empty(),
        "{label}: keys the CLI does not list: {extra:?}"
    );
    for k in got.keys() {
        assert!(
            is_disk_spelling(&repo.library(), k),
            "{label}: {k:?} is not the disk's spelling"
        );
    }
}

/// Whether `rel` names an entry whose every component is byte-equal to a name its parent
/// directory lists.
fn is_disk_spelling(base: &Path, rel: &str) -> bool {
    let mut dir = base.to_path_buf();
    for comp in rel.split('/') {
        let listed = std::fs::read_dir(&dir).is_ok_and(|d| {
            d.filter_map(Result::ok)
                .any(|e| e.file_name().to_str() == Some(comp))
        });
        if !listed {
            return false;
        }
        dir.push(comp);
    }
    true
}

/// Whether the file system under `dir` folds case (APFS, NTFS by default).
fn folds_case(dir: &Path) -> bool {
    let probe = dir.join("hv-t07-CASE-probe");
    std::fs::write(&probe, b"").unwrap();
    let folds = std::fs::symlink_metadata(dir.join("hv-t07-case-probe")).is_ok();
    std::fs::remove_file(&probe).unwrap();
    folds
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
// The fixtures of `alg/28` §0.4 (18) plus a linked worktree and a `clone --shared` (n2), and
// (fix round 1) an NFD-spelled library folder (I1) and case-only renames (I2).
// ---------------------------------------------------------------------------------------------

const FIXTURES: [&str; 23] = [
    "nfd-library-folder",
    "case-only-rename",
    "case-only-library-rename",
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
/// `çalışma` ("work") decomposed: `c` + U+0327, `s` + U+0327 (`ı` has no decomposition).
const NFD_CALISMA: &str = "c\u{327}al\u{131}s\u{327}ma";
/// `İşler` decomposed: `I` + U+0307, `s` + U+0327.
const NFD_ISLER: &str = "I\u{307}s\u{327}ler.sql";

fn base(r: &Repo) {
    r.write("a.sql", b"select 1;\n");
    r.write("b.sql", b"select 2;\n");
    r.write("reports/c.sql", b"select 3;\n");
    r.commit_all();
}

fn build_fixture(name: &str) -> Repo {
    let mut r = Repo::init();
    match name {
        // I1: the library folder itself is NFD on disk (macOS keeps it), the index NFC under
        // `core.precomposeUnicode`; a Turkish name, the primary user's case.
        "nfd-library-folder" => {
            r.write(&format!("{NFD_CALISMA}/a.sql"), b"select 1;\n");
            r.write(&format!("{NFD_CALISMA}/{NFD_ISLER}"), b"select 2;\n");
            r.write("outside.sql", b"select 3;\n");
            r.commit_all();
            r.write(&format!("{NFD_CALISMA}/a.sql"), b"select 11;\n");
            r.write(&format!("{NFD_CALISMA}/{NFD_ISLER}"), b"select 22;\n");
            r.write(&format!("{NFD_CALISMA}/yeni.sql"), b"select 4;\n");
            r.write("outside.sql", b"select 33;\n");
            r.lib = NFD_CALISMA.into();
        }
        // I2: `Foo.sql` committed, renamed on disk to `foo.sql` (no CLI), then changed.
        "case-only-rename" => {
            base(&r);
            r.write("Foo.sql", b"select 1;\n");
            r.commit_all();
            std::fs::rename(r.root().join("Foo.sql"), r.root().join("foo.sql")).unwrap();
            r.write("foo.sql", b"select 'changed';\n");
        }
        // I2: the library folder `scripts/` renamed on disk to `Scripts/`, a file changed.
        "case-only-library-rename" => {
            r.write("scripts/a.sql", b"select 1;\n");
            r.write("scripts/b.sql", b"select 2;\n");
            r.commit_all();
            std::fs::rename(r.root().join("scripts"), r.root().join("Scripts")).unwrap();
            r.write("Scripts/a.sql", b"select 'changed';\n");
            r.lib = "Scripts".into();
        }
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
        let st = repo.status();
        assert_eq!(index_bytes(&repo), before, "{fx}: the index changed");
        assert_matches_cli(fx, &st, &repo);
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

/// P1 (I1): the library folder is a Turkish NFD name inside the repository. Every change must
/// show, keyed by the disk's (NFD) spelling, never an empty "clean" map.
#[test]
fn nfd_named_library_folder_has_badges() {
    let repo = build_fixture("nfd-library-folder");
    let st = repo.status();
    let GitState::Ready(s) = &st else {
        panic!("expected Ready, got {st:?}");
    };
    let listed: Vec<String> = std::fs::read_dir(repo.library())
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().into_string().ok())
        .collect();
    let isler = listed
        .iter()
        .find(|n| nfc(n) == nfc(NFD_ISLER))
        .expect("listed");
    assert_eq!(
        s.by_path.get("a.sql"),
        Some(&FileStatus::Modified),
        "{:?}",
        s.by_path
    );
    assert_eq!(
        s.by_path.get(isler.as_str()),
        Some(&FileStatus::Modified),
        "{:?}",
        s.by_path
    );
    assert_eq!(
        s.by_path.get("yeni.sql"),
        Some(&FileStatus::New),
        "{:?}",
        s.by_path
    );
    assert_eq!(
        s.by_path.len(),
        3,
        "outside.sql is not in the library: {:?}",
        s.by_path
    );
}

/// P2 (I2, R-T7-1): `Foo.sql` in the index, `foo.sql` on disk, changed. On a file system that
/// folds case the CLI reports `Foo.sql` modified; ours keys the disk's `foo.sql` with
/// `Unknown(CaseMismatch)`, never drops it. The expected side is read from the CLI output and
/// `read_dir`, not from the product's mapping. Where the file system does not fold case, the
/// rename is a deletion plus a new file.
#[test]
fn case_only_rename_is_unknown_case_mismatch() {
    for fx in ["case-only-rename", "case-only-library-rename"] {
        let repo = build_fixture(fx);
        let folds = folds_case(&repo.library());
        let raw = String::from_utf8(repo.git_in(
            &repo.work,
            &["--no-optional-locks", "status", "--porcelain=v2", "-z"],
        ))
        .unwrap();
        let got = ours(&repo.status());
        let (changed, key) = if fx == "case-only-rename" {
            ("Foo.sql", "foo.sql")
        } else {
            ("scripts/a.sql", "a.sql")
        };
        let disk_name = std::fs::read_dir(repo.library())
            .unwrap()
            .filter_map(|e| e.unwrap().file_name().into_string().ok())
            .find(|n| n.eq_ignore_ascii_case(key));
        assert_eq!(
            disk_name.as_deref(),
            Some(key),
            "{fx}: the disk spells it {key}"
        );
        if folds {
            assert!(
                raw.split('\0')
                    .any(|r| r.starts_with("1 .M") && r.ends_with(changed)),
                "{fx}: the CLI reports {changed} modified: {raw:?}"
            );
            assert_eq!(
                got.get(key).map(String::as_str),
                Some("Unknown(CaseMismatch)"),
                "{fx}: {got:?}"
            );
            assert!(!got.contains_key(changed), "{fx}: {got:?}");
        } else {
            assert_eq!(
                got.get(key).map(String::as_str),
                Some("New"),
                "{fx}: {got:?}"
            );
        }
        fact(&format!("t07 {fx} folds_case={folds} ours={got:?}"));
    }
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

fn short() -> GitLimits {
    GitLimits {
        timeout: std::time::Duration::from_millis(300),
        ..GitLimits::default()
    }
}

/// A worker that blocks where `at` says until the returned sender drops (or sends).
fn stalled(lib: &Path, at: hacivat_git::testing::Stall) -> (GitState, std::sync::mpsc::Sender<()>) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    // Bounded, so a regression (the caller resolving the path itself) fails the test instead of
    // hanging it.
    let st = hacivat_git::testing::status_with_stall(lib, &short(), at, move || {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(10));
    });
    (st, tx)
}

/// A worker that never answers in time (m3): `Unavailable(Timeout)` without joining it; while it
/// is stuck, the library answers `Paused` at once (m2) and starts no second worker. Since fix
/// round 2 a FIFO `.gitignore` no longer hangs anything (F1), so the stuck worker is injected.
#[test]
fn timeout_is_unavailable_not_partial() {
    let repo = build_fixture("worktree-modified");
    let (first, release) = stalled(&repo.library(), hacivat_git::testing::Stall::AfterResolve);
    assert!(
        matches!(first, GitState::Unavailable(GitError::Timeout)),
        "{first:?}"
    );
    let second = status(&repo.library(), &short());
    assert!(
        matches!(second, GitState::Unavailable(GitError::Paused)),
        "{second:?}"
    );
    drop(release);
}

/// F2: the caller does no file-system work. While the worker's own path resolution blocks
/// (a stale network mount), the caller still answers within its budget, and a second call with
/// the same spelling answers `Paused` without resolving anything.
#[test]
fn caller_does_no_file_system_work() {
    let repo = build_fixture("worktree-modified");
    let (first, release) = stalled(&repo.library(), hacivat_git::testing::Stall::BeforeResolve);
    assert!(
        matches!(first, GitState::Unavailable(GitError::Timeout)),
        "{first:?}"
    );
    let second = status(&repo.library(), &short());
    assert!(
        matches!(second, GitState::Unavailable(GitError::Paused)),
        "{second:?}"
    );
    drop(release);
}

/// P4 (m1, F2): the one-stuck-worker cap is per canonical library. Once the stuck worker has
/// resolved its path, a call through an alias (a symlink, `.`, `..`) answers `Paused`: its own
/// worker resolves the alias, finds the canonical library taken, and stops.
#[cfg(unix)]
#[test]
fn stuck_worker_cap_is_per_canonical_library() {
    let repo = build_fixture("worktree-modified");
    let alias = canonical(repo.env.path()).join("alias");
    std::os::unix::fs::symlink(repo.root(), &alias).unwrap();
    let (first, release) = stalled(&repo.library(), hacivat_git::testing::Stall::AfterResolve);
    assert!(
        matches!(first, GitState::Unavailable(GitError::Timeout)),
        "{first:?}"
    );
    for spelling in [
        alias.clone(),
        alias.join("."),
        repo.root().join("reports/.."),
    ] {
        let st = status(&spelling, &short());
        assert!(
            matches!(st, GitState::Unavailable(GitError::Paused)),
            "{spelling:?}: {st:?}"
        );
    }
    drop(release);
}

/// P9 (F1, R-T7-2): every `.gitignore`, `.gitattributes` and `.gitmodules` gix would read, in
/// the library or a folder between the worktree and it, must be a regular file of at most
/// 1 MiB: a 300 MiB sparse one is refused before anything reads it (`Failed(TooLarge)`), a FIFO
/// is refused at once (`Unavailable(Redirected)`), never a hang.
#[test]
fn huge_or_special_control_file_is_refused() {
    let too_large = GitState::Unavailable(GitError::Failed(hacivat_git::FailKind::TooLarge));
    for (case, file, lib) in [
        ("root-gitignore", ".gitignore", ""),
        ("root-gitattributes", ".gitattributes", ""),
        ("nested-gitignore", "reports/.gitignore", ""),
        ("ancestor-gitignore", ".gitignore", "reports"),
        ("ancestor-gitmodules", ".gitmodules", "reports"),
    ] {
        let mut repo = build_fixture("clean");
        repo.lib = lib.into();
        let f = std::fs::File::create(repo.root().join(file)).unwrap();
        f.set_len(300 << 20).unwrap();
        drop(f);
        let st = repo.status();
        assert_eq!(format!("{st:?}"), format!("{too_large:?}"), "{case}");
    }
    #[cfg(unix)]
    for (case, file, lib) in [
        ("fifo-gitattributes", ".gitattributes", ""),
        ("fifo-nested-gitignore", "reports/.gitignore", ""),
        ("fifo-ancestor-gitignore", ".gitignore", "reports"),
    ] {
        let mut repo = build_fixture("clean");
        repo.lib = lib.into();
        assert!(
            Command::new("mkfifo")
                .arg(repo.root().join(file))
                .status()
                .unwrap()
                .success()
        );
        let st = repo.status();
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "{case}: {st:?}"
        );
    }
}

/// F3 (R-T7-4): the repository's own files that the open, the index read and the HEAD lookup
/// touch are never opened when they are cloud placeholders: `HEAD`, `config`, `index`,
/// `packed-refs`, the loose ref HEAD names, a pack file, a nested repository's gitfile or
/// `HEAD`. The library answers `Unavailable(Dataless)`.
#[test]
fn dataless_repository_internals_are_unavailable() {
    let repo = build_fixture("clean");
    repo.git(&["repack", "-a", "-d", "-q"]);
    let pack = std::fs::read_dir(repo.root().join(".git/objects/pack"))
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().into_string().ok())
        .find(|n| n.ends_with(".pack"))
        .expect("a pack after repack");
    let pack = format!(".git/objects/pack/{pack}");
    std::fs::write(
        repo.root().join(".git/packed-refs"),
        "# pack-refs with: peeled\n",
    )
    .unwrap();
    // A nested repository (untracked) with a gitfile, and one with a `.git` directory.
    let other = build_fixture("clean");
    std::fs::create_dir_all(repo.root().join("nested")).unwrap();
    std::fs::write(
        repo.root().join("nested/.git"),
        format!("gitdir: {}\n", other.root().join(".git").display()),
    )
    .unwrap();
    std::fs::create_dir_all(repo.root().join("inner/.git/objects")).unwrap();
    std::fs::create_dir_all(repo.root().join("inner/.git/refs")).unwrap();
    std::fs::write(
        repo.root().join("inner/.git/HEAD"),
        "ref: refs/heads/main\n",
    )
    .unwrap();
    // Control: nothing flagged, the status is there.
    assert!(matches!(repo.status(), GitState::Ready(_)), "control");
    for flagged in [
        ".git/HEAD",
        ".git/config",
        ".git/index",
        ".git/packed-refs",
        ".git/refs/heads/main",
        pack.as_str(),
        "nested/.git",
        "inner/.git/HEAD",
    ] {
        let st = hacivat_git::testing::status_treating_as_dataless(
            &repo.library(),
            &GitLimits::default(),
            &[flagged],
        );
        assert!(
            matches!(st, GitState::Unavailable(GitError::Dataless)),
            "{flagged}: {st:?}"
        );
    }
}

/// n1: the index ceiling is gix's own reduced-trust allocation limit (16 MiB of decoded
/// entries), measured here with a real index at the boundary: the largest entry count is
/// `Ready`, one more is `Unavailable(TooLarge)`, never a silent `Failed`.
#[test]
fn index_entry_ceiling_is_measured_at_the_boundary() {
    if !cli_ok("index_entry_ceiling_is_measured_at_the_boundary") {
        return;
    }
    let ceiling = (16 << 20) / std::mem::size_of::<gix::index::Entry>();
    // ## m4b-cifix-1: the test measures a size ceiling, not the 5 s call budget. Decoding and
    // walking ~200,000 entries in a debug build takes longer than 5 s on a slow CI runner
    // (CI 44981552034, macOS: `Unavailable(Timeout)` at the ceiling), so the call gets a budget
    // no runner reaches and a `Timeout` can never stand in for either answer. The product's
    // budget stays `GitLimits::default().timeout` (5 s, asserted in the crate's unit tests).
    let limits = GitLimits {
        max_entries: usize::MAX,
        timeout: std::time::Duration::from_secs(600),
        // ## m4b-t20-adversarial fix round 2 (named line): the new field, its default.
        ..GitLimits::default()
    };
    let mut results = Vec::new();
    for n in [ceiling, ceiling + 1] {
        let repo = Repo::init();
        let blob =
            String::from_utf8(repo.git(&["hash-object", "-w", "--stdin"])).unwrap_or_default();
        let blob = if blob.trim().is_empty() {
            // `hash-object --stdin` with no input hashes the empty blob.
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_owned()
        } else {
            blob.trim().to_owned()
        };
        let mut info = String::with_capacity(n * 40);
        for i in 0..n {
            info.push_str(&format!("100644 {blob}\td{}/f{i}\n", i % 512));
        }
        let mut child = repo
            .cmd_in(&repo.root(), &["update-index", "--index-info"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(info.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        let st = status(&repo.library(), &limits);
        results.push(format!(
            "{n}:{}",
            match &st {
                GitState::Ready(_) => "Ready".to_owned(),
                other => format!("{other:?}"),
            }
        ));
        if n == ceiling {
            assert!(matches!(st, GitState::Ready(_)), "{n} entries: {st:?}");
        } else {
            assert!(
                matches!(
                    st,
                    GitState::Unavailable(GitError::TooLarge(hacivat_git::TooLargeWhy::Index))
                ),
                "{n} entries: {st:?}"
            );
        }
    }
    fact(&format!(
        "t07 index_ceiling entry_size={} max_entries={ceiling} results={}",
        std::mem::size_of::<gix::index::Entry>(),
        results.join(",")
    ));
}

/// n2: a non-ASCII case-only rename of the library folder (`Çal/` in the index, `çal/` on
/// disk) is `Unknown(CaseMismatch)`, never `New`, where the file system folds case.
#[test]
fn non_ascii_case_only_library_rename_is_case_mismatch() {
    let mut repo = Repo::init();
    repo.write("\u{c7}al/a.sql", b"select 1;\n");
    repo.commit_all();
    std::fs::rename(repo.root().join("\u{c7}al"), repo.root().join("\u{e7}al")).unwrap();
    repo.write("\u{e7}al/a.sql", b"select 'changed';\n");
    repo.lib = "\u{e7}al".into();
    if !folds_case(&repo.library()) {
        fact("t07 non_ascii_case_only_library_rename skipped (the file system does not fold case)");
        return;
    }
    let got = ours(&repo.status());
    assert_eq!(
        got.get("a.sql").map(String::as_str),
        Some("Unknown(CaseMismatch)"),
        "{got:?}"
    );
}

/// n5: a dataless folder is not listed, by the pre-walk or by gix (T4 does not read dataless
/// folders either): nothing under it is reported, nor refused. Its oversize `.gitignore` would be
/// `Failed(TooLarge)` and its FIFO `Unknown(NotAFile)` if anything listed it.
#[test]
fn dataless_folder_is_not_listed() {
    let repo = build_fixture("clean");
    let f = std::fs::File::create(repo.root().join("reports/.gitignore")).unwrap();
    f.set_len(2 << 20).unwrap();
    drop(f);
    repo.write("reports/new.sql", b"select 4;\n");
    #[cfg(unix)]
    assert!(
        Command::new("mkfifo")
            .arg(repo.root().join("reports/pipe"))
            .status()
            .unwrap()
            .success()
    );
    repo.write("a.sql", b"select 'changed';\n");
    let st = hacivat_git::testing::status_treating_as_dataless(
        &repo.library(),
        &GitLimits::default(),
        &["reports"],
    );
    let got = ours(&st);
    assert_eq!(
        got.get("a.sql").map(String::as_str),
        Some("Modified"),
        "{got:?}"
    );
    assert!(!got.keys().any(|k| k.starts_with("reports/")), "{got:?}");
}

/// R-T7-3 (I4): a `.gitignore` or `.gitattributes` the status would read is a cloud
/// placeholder: it is not opened (that downloads it), and the library answers
/// `Unavailable(Dataless)`. Tracked and untracked, at the library root and deeper.
#[test]
fn dataless_control_file_is_unavailable() {
    for (case, file, tracked) in [
        ("tracked-gitignore", ".gitignore", true),
        ("untracked-gitignore", "reports/.gitignore", false),
        ("tracked-gitattributes", ".gitattributes", true),
        ("untracked-gitattributes", "reports/.gitattributes", false),
    ] {
        let repo = build_fixture("clean");
        repo.write(file, b"*.tmp\n");
        if tracked {
            repo.commit_all();
        }
        repo.write("a.sql", b"select 'changed';\n");
        let st = hacivat_git::testing::status_treating_as_dataless(
            &repo.library(),
            &GitLimits::default(),
            &[file],
        );
        assert!(
            matches!(st, GitState::Unavailable(GitError::Dataless)),
            "{case}: {st:?}"
        );
        // Control: not a placeholder, the status is there.
        assert_eq!(
            ours(&repo.status()).get("a.sql").map(String::as_str),
            Some("Modified"),
            "{case}"
        );
    }
}

/// m5: an untracked FIFO is not a file: `Unknown(NotAFile)`, never New, and never opened.
#[cfg(unix)]
#[test]
fn untracked_fifo_is_unknown() {
    let repo = build_fixture("clean");
    let fifo = repo.root().join("pipe.sql");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let got = ours(&repo.status());
    assert_eq!(
        got.get("pipe.sql").map(String::as_str),
        Some("Unknown(NotAFile)"),
        "{got:?}"
    );
}

/// m4: a repository config that cannot be read (garbage, a symlink, too large) is
/// `Unsupported`, not opened anyway.
#[test]
fn unreadable_config_is_unsupported() {
    let garbage = build_fixture("clean");
    std::fs::write(
        garbage.root().join(".git/config"),
        b"[core\n\tbare = = =\n[[[",
    )
    .unwrap();
    let st = garbage.status();
    assert!(
        matches!(st, GitState::Unavailable(GitError::Unsupported)),
        "garbage: {st:?}"
    );
    let big = build_fixture("clean");
    let cfg = big.root().join(".git/config");
    let mut text = std::fs::read_to_string(&cfg).unwrap();
    text.push_str(&format!("# {}\n", "x".repeat(2 * 1024 * 1024)));
    std::fs::write(&cfg, text).unwrap();
    let st = big.status();
    assert!(
        matches!(st, GitState::Unavailable(GitError::Unsupported)),
        "too large: {st:?}"
    );
    #[cfg(unix)]
    {
        let linked = build_fixture("clean");
        let cfg = linked.root().join(".git/config");
        let real = canonical(linked.env.path()).join("real.cfg");
        std::fs::rename(&cfg, &real).unwrap();
        std::os::unix::fs::symlink(&real, &cfg).unwrap();
        let st = linked.status();
        assert!(
            matches!(st, GitState::Unavailable(GitError::Unsupported)),
            "symlink: {st:?}"
        );
    }
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

// ## m4b-t07-git (fix round 3)

fn dataless_state(repo: &Repo, flagged: &str) -> GitState {
    hacivat_git::testing::status_treating_as_dataless(
        &repo.library(),
        &GitLimits::default(),
        &[flagged],
    )
}

// ## m4b-cifix-2-hook-spelling
/// An absolute path outside the library as the status itself spells it, for the dataless hook,
/// which matches paths exactly: `std::fs::canonicalize`, unstripped. On Windows that is the
/// verbatim `\\?\C:\…` form with the disk's own case, which is how the status resolves a
/// gitfile's target and the alternates (`trust::resolve`). The test's [`canonical`] strips the
/// verbatim prefix for the CLI, so a hook entry built from it named no path the status checks
/// (CI 44990057141, Windows: the `commondir` case answered `Ready`). The product's own
/// placeholder test reads the file's attributes and never compares spellings.
fn as_the_status_spells_it(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).expect("the flagged file exists")
}
// ## m4b-cifix-2-hook-spelling

fn is_dataless(st: &GitState) -> bool {
    matches!(st, GitState::Unavailable(GitError::Dataless))
}

/// P11 (G1, R-T7-5): gix's open scans `refs/replace/**` with an uncapped read and follows a
/// symlinked `refs/replace` out of the repository. The status opens with
/// `core.useReplaceRefs=false`, so the folder is never read:
/// - a functional proof: `git replace HEAD <side>` would make a status that honours replace
///   refs see the side commit's tree (`a.sql` staged as modified); ours stays clean, as the CLI
///   does under `GIT_NO_REPLACE_OBJECTS=1`. A 200 MiB file sits next to the replace ref;
/// - a symlinked `refs/replace` is refused before the open.
#[cfg(unix)]
#[test]
fn replace_refs_are_never_scanned() {
    let repo = build_fixture("clean");
    repo.git(&["checkout", "-q", "-b", "side"]);
    repo.write("a.sql", b"select 'replaced';\n");
    repo.commit_all();
    repo.git(&["checkout", "-q", "main"]);
    let main_head = String::from_utf8(repo.git(&["rev-parse", "main"])).unwrap();
    repo.git(&["replace", main_head.trim(), "side"]);
    let replace = repo.root().join(".git/refs/replace");
    assert!(replace.join(main_head.trim()).is_file());
    let big = std::fs::File::create(replace.join("1".repeat(40))).unwrap();
    big.set_len(200 << 20).unwrap();
    drop(big);
    let got = ours(&repo.status());
    assert!(got.is_empty(), "replace refs were honoured: {got:?}");
    let cli = repo
        .cmd_in(
            &repo.root(),
            &["--no-optional-locks", "status", "--porcelain=v2"],
        )
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()
        .unwrap();
    assert!(cli.stdout.is_empty(), "the CLI without replace refs agrees");
    // `refs/replace` a symlink to a folder outside the repository: refused.
    let repo = build_fixture("worktree-modified");
    let outside = canonical(repo.env.path()).join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let big = std::fs::File::create(outside.join("4".repeat(40))).unwrap();
    big.set_len(200 << 20).unwrap();
    drop(big);
    std::os::unix::fs::symlink(&outside, repo.root().join(".git/refs/replace")).unwrap();
    let st = repo.status();
    assert!(
        matches!(st, GitState::Unavailable(GitError::Redirected)),
        "symlinked refs/replace: {st:?}"
    );
}

/// P10 (G2, R-T7-6): loose objects are read on every status (the HEAD commit and every tree
/// reachable from it), so a placeholder among them is never opened: `Unavailable(Dataless)`.
/// The fan-out folder itself counts too.
#[test]
fn dataless_loose_object_is_unavailable() {
    let repo = build_fixture("clean");
    let head = String::from_utf8(repo.git(&["rev-parse", "HEAD"])).unwrap();
    let head = head.trim();
    let tree = String::from_utf8(repo.git(&["rev-parse", "HEAD^{tree}"])).unwrap();
    let tree = tree.trim();
    assert!(matches!(repo.status(), GitState::Ready(_)), "control");
    for (case, flagged) in [
        (
            "commit",
            format!(".git/objects/{}/{}", &head[..2], &head[2..]),
        ),
        (
            "tree",
            format!(".git/objects/{}/{}", &tree[..2], &tree[2..]),
        ),
        ("fan-out folder", format!(".git/objects/{}", &tree[..2])),
    ] {
        let st = dataless_state(&repo, &flagged);
        assert!(is_dataless(&st), "{case}: {st:?}");
    }
}

/// G2: more loose objects than can be checked before a status answers the diagnostic "too
/// many loose objects; run git gc" (`TooLarge(LooseObjects)`), never a status that might
/// open a placeholder.
#[test]
fn too_many_loose_objects_is_unavailable() {
    let repo = build_fixture("clean");
    let fan = repo.root().join(".git/objects/00");
    std::fs::create_dir_all(&fan).unwrap();
    for i in 0..20_001 {
        std::fs::write(fan.join(format!("{i:038}")), b"").unwrap();
    }
    let st = repo.status();
    assert!(
        matches!(
            st,
            GitState::Unavailable(GitError::TooLarge(hacivat_git::TooLargeWhy::LooseObjects))
        ),
        "{st:?}"
    );
}

/// p1: the small files our own checks read before the open, and the split index, are never
/// opened as placeholders: a linked worktree's `commondir` and `gitdir`, the alternates file
/// and an alternate's loose object, `sharedindex.*`. A FIFO `sharedindex.*` never hangs.
#[test]
fn dataless_layout_files_and_split_index_are_unavailable() {
    // Linked worktree: its git dir's `commondir` and `gitdir`.
    let main = build_fixture("linked-worktree");
    let wt_git = main.root().join(".git").join("worktrees").join("wt");
    for f in ["commondir", "gitdir"] {
        let flagged = as_the_status_spells_it(&wt_git.join(f));
        let st = dataless_state(&main, flagged.to_str().unwrap());
        assert!(is_dataless(&st), "linked worktree {f}: {st:?}");
    }
    // A shared clone: its alternates file, and a loose object in the alternate.
    let clone = build_fixture("shared-clone");
    let st = dataless_state(&clone, ".git/objects/info/alternates");
    assert!(is_dataless(&st), "alternates: {st:?}");
    let head = String::from_utf8(clone.git(&["rev-parse", "HEAD"])).unwrap();
    let head = head.trim().to_owned();
    let in_alternate = clone
        .root()
        .join(".git/objects")
        .join(&head[..2])
        .join(&head[2..]);
    assert!(
        in_alternate.is_file(),
        "the source repository holds HEAD loose"
    );
    let flagged = as_the_status_spells_it(&in_alternate);
    let st = dataless_state(&clone, flagged.to_str().unwrap());
    assert!(is_dataless(&st), "alternate's loose object: {st:?}");
    // A split index: `sharedindex.<sha>`.
    let repo = build_fixture("worktree-modified");
    repo.git(&["update-index", "--split-index"]);
    let shared = std::fs::read_dir(repo.root().join(".git"))
        .unwrap()
        .filter_map(|e| e.unwrap().file_name().into_string().ok())
        .find(|n| n.starts_with("sharedindex."))
        .expect("a shared index after --split-index");
    let st = dataless_state(&repo, &format!(".git/{shared}"));
    assert!(is_dataless(&st), "sharedindex: {st:?}");
    #[cfg(unix)]
    {
        let p = repo.root().join(".git").join(&shared);
        std::fs::remove_file(&p).unwrap();
        assert!(Command::new("mkfifo").arg(&p).status().unwrap().success());
        let st = repo.status();
        assert!(
            matches!(st, GitState::Unavailable(GitError::Redirected)),
            "FIFO sharedindex: {st:?}"
        );
    }
}
// ## m4b-t07-git
