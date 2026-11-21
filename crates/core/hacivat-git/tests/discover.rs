//! HV-SCRIPT-11 step 1: discovery walks up from the library root to the first `.git` entry and
//! stops at the user's home directory (a `$HOME/.git` dotfiles repository never claims every
//! library under home, R-M4b-10) and at a filesystem boundary (`discover::tests` covers the
//! boundary with an injected device id; a test cannot make a mount point).
//!
//! The home ceiling is read from the environment, so those cases re-run this test binary as a
//! child with `HOME` set on the child only (no test calls `set_var`). The git CLI builds the
//! fixtures (`NoGitSubprocess` exempts `tests/`).

use std::path::{Path, PathBuf};
use std::process::Command;

use hacivat_git::{FileStatus, GitLimits, GitState, status};

const CHILD_LIB: &str = "HV_T07_CHILD_LIB";
const CHILD_OUT: &str = "HV_T07_CHILD_OUT";

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).expect("canonical path")
}

fn git(cwd: &Path, args: &[&str]) {
    // A global config path that never exists: the CLI reads it as empty.
    let empty = cwd.join("hv-t07-no-global-config");
    let mut c = Command::new("git");
    c.current_dir(cwd).args(args);
    for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_CONFIG_PARAMETERS"] {
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

/// `<top>/repo` is a repository with a committed `lib/a.sql`, changed on disk; `<top>/repo/lib`
/// is the library.
fn repo_with_library(top: &Path) -> (PathBuf, PathBuf) {
    let repo = top.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    write(&repo.join("lib/a.sql"), b"select 1;\n");
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "--no-verify", "-m", "base"]);
    write(&repo.join("lib/a.sql"), b"select 'changed';\n");
    (repo.clone(), repo.join("lib"))
}

fn describe(st: &GitState) -> String {
    match st {
        GitState::Ready(s) => {
            let parts: Vec<String> = s
                .by_path
                .iter()
                .map(|(k, v)| format!("{}={v:?}", k.as_str()))
                .collect();
            format!("Ready {}", parts.join(";"))
        }
        other => format!("{other:?}"),
    }
}

fn child_run() -> bool {
    let (Some(lib), Some(out)) = (std::env::var_os(CHILD_LIB), std::env::var_os(CHILD_OUT)) else {
        return false;
    };
    let st = status(Path::new(&lib), &GitLimits::default());
    std::fs::write(out, describe(&st)).expect("write the child result");
    true
}

/// Runs the test `name` as a child with `HOME` (and `USERPROFILE`) set to `home`.
fn in_child(name: &str, lib: &Path, home: &Path, out_dir: &Path) -> String {
    let out = out_dir.join(format!("child-{name}.txt"));
    let st = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--quiet"])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env(CHILD_LIB, lib)
        .env(CHILD_OUT, &out)
        .status()
        .unwrap();
    assert!(st.success());
    std::fs::read_to_string(&out).unwrap()
}

#[test]
fn discovery_stops_at_home_and_fs_boundary() {
    if child_run() {
        return;
    }
    // The repository's worktree is the home directory itself: never claimed.
    let t = tempfile::tempdir().unwrap();
    let top = canonical(t.path());
    let (repo, lib) = repo_with_library(&top);
    let out = in_child("discovery_stops_at_home_and_fs_boundary", &lib, &repo, &top);
    assert_eq!(out, "NotARepo", "home is the worktree");
    // A repository above home: never claimed either.
    let home = repo.join("lib");
    write(&home.join("sub/s.sql"), b"select 3;\n");
    let out = in_child("discovery_stops_at_home_and_fs_boundary", &home.join("sub"), &home, &top);
    assert_eq!(out, "NotARepo", "the repository is above home");
    // Control: home elsewhere, the same library has its badges (the committed file changed,
    // and the untracked one written above).
    let elsewhere = top.join("elsewhere-home");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let out = in_child("discovery_stops_at_home_and_fs_boundary", &lib, &elsewhere, &top);
    assert_eq!(out, "Ready a.sql=Modified;sub/s.sql=New");
}

#[test]
fn discovery_finds_the_repository_above_the_library() {
    let t = tempfile::tempdir().unwrap();
    let top = canonical(t.path());
    let (_repo, lib) = repo_with_library(&top);
    write(&lib.join("deeper/b.sql"), b"select 2;\n");
    let st = status(&lib.join("deeper"), &GitLimits::default());
    let GitState::Ready(s) = &st else {
        panic!("{st:?}")
    };
    // Keys are relative to the library root, and paths outside it are dropped.
    assert_eq!(s.by_path.get("b.sql"), Some(&FileStatus::New));
    assert_eq!(s.by_path.len(), 1, "{:?}", s.by_path);
}

#[test]
fn not_a_repo_is_not_an_error() {
    let t = tempfile::tempdir().unwrap();
    write(&t.path().join("lib/a.sql"), b"select 1;\n");
    let st = status(&t.path().join("lib"), &GitLimits::default());
    assert!(matches!(st, GitState::NotARepo), "{st:?}");
}

#[test]
fn bare_repository_is_not_a_repo() {
    let t = tempfile::tempdir().unwrap();
    let bare = canonical(t.path()).join("bare.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    let lib = bare.join("lib");
    write(&lib.join("a.sql"), b"select 1;\n");
    let st = status(&lib, &GitLimits::default());
    assert!(matches!(st, GitState::NotARepo), "{st:?}");
}

#[test]
fn missing_library_is_unavailable() {
    let t = tempfile::tempdir().unwrap();
    let st = status(&t.path().join("gone"), &GitLimits::default());
    assert!(
        matches!(st, GitState::Unavailable(hacivat_git::GitError::Failed(_))),
        "{st:?}"
    );
}

/// A `.git` that is a symlink is a redirection we do not follow.
#[cfg(unix)]
#[test]
fn symlinked_dot_git_is_redirected() {
    let t = tempfile::tempdir().unwrap();
    let top = canonical(t.path());
    let (repo, _lib) = repo_with_library(&top);
    let other = top.join("other");
    std::fs::create_dir_all(&other).unwrap();
    write(&other.join("a.sql"), b"select 1;\n");
    std::os::unix::fs::symlink(repo.join(".git"), other.join(".git")).unwrap();
    let st = status(&other, &GitLimits::default());
    assert!(
        matches!(st, GitState::Unavailable(hacivat_git::GitError::Redirected)),
        "{st:?}"
    );
}
