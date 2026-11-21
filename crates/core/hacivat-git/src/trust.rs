//! What Hacivat checks itself, before and after `gix` opens a repository (HV-SCRIPT-11 steps 2
//! and 3; `alg/28` C1, I1, m1, m2, n2; R-M4b-10).
//!
//! Before the open (metadata and three tiny files only, on the status worker):
//! - **Ownership** (Unix): the worktree, the `.git` entry, the git dir and common dir a gitfile
//!   leads to, and every alternate object directory must belong to the current user, else
//!   `Untrusted(ForeignOwner)`. Forced reduced trust makes `gix` open such a repository anyway,
//!   so this is Hacivat's check, as git's `safe.directory` is git's.
//! - **Layout**: a symlinked `.git` is not followed. A gitfile is followed only to a linked
//!   worktree's git dir (`<common>/worktrees/<name>`, whose `commondir` leads back to
//!   `<common>`), to a submodule's git dir under an enclosing repository's `.git/modules/`, or
//!   inside the worktree. A plain `.git` directory with a `commondir`, and alternates that
//!   name anything but an existing directory the user owns (or nest deeper than four), are
//!   refused: `Unavailable(Redirected)`. Linked worktrees and `clone --shared` repositories
//!   are read under the same isolated, reduced-trust open (n2).
//!
//! After the open: `git_dir_trust() == Reduced`, and no repository-kind config section has full
//! trust (gix can promote single sections when a `safe.directory` matches the config file),
//! else `Unavailable(TrustPromoted)`; the directories `gix` resolved are the ones checked here,
//! else `Unavailable(Redirected)`; an object format or ref storage this build cannot read is
//! `Unavailable(Unsupported)`.

// ## m4b-t07-git
use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::discover::{DotGit, Found};
use crate::{FailKind, GitError, GitState, UntrustReason};

/// The git dir and common dir the checks accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) git_dir: PathBuf,
    pub(crate) common_dir: PathBuf,
}

/// Whether the current user owns a path (`lstat`). Injected so that tests can name a foreign
/// owner without root.
pub(crate) type Owner<'a> = &'a dyn Fn(&Path) -> std::io::Result<bool>;

/// The real owner check: Unix only (R-M4b-10); elsewhere every path counts as owned.
pub(crate) fn current_user_owns(p: &Path) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        gix::sec::identity::is_path_owned_by_current_user(p)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        Ok(true)
    }
}

/// The most bytes read from a gitfile, a `commondir` file or an alternates file.
const SMALL_FILE_MAX: u64 = 64 * 1024;
/// How deep alternates may chain (an alternate's own alternates).
const ALTERNATES_DEPTH_MAX: usize = 4;

fn redirected<T>() -> Result<T, GitState> {
    Err(GitState::Unavailable(GitError::Redirected))
}

fn require_owned(p: &Path, owned: Owner<'_>) -> Result<(), GitState> {
    match owned(p) {
        Ok(true) => Ok(()),
        Ok(false) => Err(GitState::Untrusted(UntrustReason::ForeignOwner)),
        Err(_) => Err(GitState::Unavailable(GitError::Failed(
            FailKind::OwnerUnreadable,
        ))),
    }
}

/// A small regular file's text (a gitfile, `commondir`, alternates), or `None` when it is not
/// a regular file, is larger than [`SMALL_FILE_MAX`], or is not UTF-8.
fn small_text(p: &Path) -> Option<String> {
    let md = std::fs::symlink_metadata(p).ok()?;
    if !md.is_file() || md.len() > SMALL_FILE_MAX {
        return None;
    }
    let mut buf = Vec::new();
    std::fs::File::open(p)
        .ok()?
        .take(SMALL_FILE_MAX + 1)
        .read_to_end(&mut buf)
        .ok()?;
    if buf.len() as u64 > SMALL_FILE_MAX {
        return None;
    }
    String::from_utf8(buf).ok()
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn resolve(base: &Path, text: &str) -> Option<PathBuf> {
    let p = Path::new(text);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    };
    std::fs::canonicalize(p).ok().filter(|c| c.is_dir())
}

/// The checks before the open. `Err` is the state to answer.
pub(crate) fn check_layout(found: &Found, owned: Owner<'_>) -> Result<Layout, GitState> {
    require_owned(&found.worktree, owned)?;
    require_owned(&found.dot_git, owned)?;
    let layout = match found.kind {
        DotGit::Symlink => return redirected(),
        DotGit::Dir => {
            let g = std::fs::canonicalize(&found.dot_git).map_err(io_failed)?;
            if exists(&g.join("commondir")) {
                return redirected();
            }
            Layout {
                git_dir: g.clone(),
                common_dir: g,
            }
        }
        DotGit::File => gitfile_layout(found, owned)?,
    };
    require_owned(&layout.git_dir, owned)?;
    require_owned(&layout.common_dir, owned)?;
    check_alternates(&layout.common_dir.join("objects"), 0, owned)?;
    Ok(layout)
}

// ## m4b-t07-git (fix round 1, I3, R-T7-2)
/// The cap on a ref-sized file: `HEAD`, a loose ref, a gitfile, `commondir`.
const REF_MAX: u64 = 4 * 1024;
/// The cap on `packed-refs`, `info/exclude`, `info/attributes`, `config`, `config.worktree`.
const TEXT_MAX: u64 = 1024 * 1024;
/// The cap on the index (20,000 entries take a few MiB; this only bounds a hostile one).
const INDEX_MAX: u64 = 256 * 1024 * 1024;
/// How deep a symbolic ref chain from `HEAD` may go.
const SYMREF_DEPTH_MAX: usize = 5;

/// `p`, when present, is a regular file (not a symlink, FIFO or device) of at most `cap`
/// bytes. Absent is fine (`Ok(false)`): gix then fails or reads nothing.
fn regular_capped(p: &Path, cap: u64) -> Result<bool, GitState> {
    let md = match std::fs::symlink_metadata(p) {
        Ok(md) => md,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(io_failed(e)),
    };
    if !md.file_type().is_file() {
        return redirected();
    }
    if md.len() > cap {
        return Err(GitState::Unavailable(GitError::Failed(FailKind::TooLarge)));
    }
    Ok(true)
}

/// The text of a regular file of at most `cap` bytes, read through a cap (`None`: absent).
fn capped_text(p: &Path, cap: u64) -> Result<Option<String>, GitState> {
    if !regular_capped(p, cap)? {
        return Ok(None);
    }
    let mut buf = Vec::new();
    std::fs::File::open(p)
        .and_then(|f| f.take(cap + 1).read_to_end(&mut buf))
        .map_err(io_failed)?;
    if buf.len() as u64 > cap {
        return Err(GitState::Unavailable(GitError::Failed(FailKind::TooLarge)));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Every file of the git dir and common dir that the open, the index read and the HEAD lookup
/// read is a regular file under its cap: `gix_ref`'s `ref_contents` reads with no limit and
/// follows a symlink, so a `HEAD -> /dev/zero` would let an abandoned worker allocate until the
/// app dies (review I3). `config` and `config.worktree` are [`unsupported_format`]'s.
pub(crate) fn check_repository_files(layout: &Layout) -> Result<(), GitState> {
    let (g, c) = (&layout.git_dir, &layout.common_dir);
    regular_capped(&g.join("index"), INDEX_MAX)?;
    regular_capped(&c.join("packed-refs"), TEXT_MAX)?;
    regular_capped(&c.join("info").join("exclude"), TEXT_MAX)?;
    regular_capped(&c.join("info").join("attributes"), TEXT_MAX)?;
    let Some(head) = capped_text(&g.join("HEAD"), REF_MAX)? else {
        return Ok(());
    };
    check_symref_chain(head.trim(), layout, 0)
}

/// A symbolic ref's target (`ref: refs/…`) and its chain: every directory on the way is a real
/// directory, and the ref file, when loose, a regular file under [`REF_MAX`].
fn check_symref_chain(text: &str, layout: &Layout, depth: usize) -> Result<(), GitState> {
    let Some(name) = text.strip_prefix("ref:").map(str::trim) else {
        return Ok(()); // a detached HEAD names an object id
    };
    let valid = name.starts_with("refs/")
        && !name.contains('\\')
        && name
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..");
    if !valid || depth >= SYMREF_DEPTH_MAX {
        return redirected();
    }
    let comps: Vec<&str> = name.split('/').collect();
    let mut bases = vec![layout.git_dir.clone()];
    if layout.common_dir != layout.git_dir {
        bases.push(layout.common_dir.clone());
    }
    for base in bases {
        let mut dir = base;
        let mut reached = true;
        for comp in &comps[..comps.len() - 1] {
            dir.push(comp);
            match std::fs::symlink_metadata(&dir) {
                Ok(md) if md.file_type().is_dir() => {}
                Ok(_) => return redirected(),
                Err(_) => {
                    reached = false;
                    break;
                }
            }
        }
        if !reached {
            continue;
        }
        if let Some(text) = capped_text(&dir.join(comps[comps.len() - 1]), REF_MAX)? {
            check_symref_chain(text.trim(), layout, depth + 1)?;
        }
    }
    Ok(())
}

/// The most entries the library pre-walk lists before it gives up (fail closed).
const PREWALK_ENTRIES_MAX: usize = 200_000;
/// The deepest folder the pre-walk enters.
const PREWALK_DEPTH_MAX: usize = 64;
/// The in-tree files gix reads to decide what is ignored, filtered or a submodule.
const CONTROL_FILES: [&str; 3] = [".gitignore", ".gitattributes", ".gitmodules"];

/// What the library pre-walk found that the status reports itself.
#[derive(Debug, Default)]
pub(crate) struct TreeFacts {
    /// Library-relative names (the disk's spelling) of FIFOs, sockets and devices (m5).
    pub(crate) not_files: Vec<String>,
}

/// The walk of the library's folders (names and `lstat` only) before gix walks it:
/// - a control file (`.gitignore`, `.gitattributes`, `.gitmodules`) here, in a folder between
///   the worktree and the library, or `info/exclude`/`info/attributes`, that is a cloud
///   placeholder answers `Unavailable(Dataless)`: gix would open it, and opening downloads it
///   (I4, R-T7-3). `extra` (the `testing` hook, library-relative) adds placeholders;
/// - a nested repository's `.git` (gix classifies it by reading its `HEAD` with no limit) must
///   be a real directory with a regular, capped `HEAD`, or a regular, capped gitfile whose
///   target's `HEAD` is the same; otherwise `Unavailable(Redirected)` (I3);
/// - FIFOs, sockets and devices are recorded (gix lists none of them; m5).
///
/// Symlinked folders are not entered (gix does not follow them), nor nested repositories, nor
/// `.git`. More than [`PREWALK_ENTRIES_MAX`] entries or a folder deeper than
/// [`PREWALK_DEPTH_MAX`] answers `Failed(TooLarge)`.
pub(crate) fn check_library_tree(
    worktree: &Path,
    library: &Path,
    layout: &Layout,
    extra: &std::collections::BTreeSet<String>,
    is_dataless: &dyn Fn(&std::fs::Metadata) -> bool,
) -> Result<TreeFacts, GitState> {
    let placeholder = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| is_dataless(&m));
    let dataless = || Err(GitState::Unavailable(GitError::Dataless));
    for info in ["exclude", "attributes"] {
        if placeholder(&layout.common_dir.join("info").join(info)) {
            return dataless();
        }
    }
    // The folders from the worktree down to the library's parent.
    if let Ok(below) = library.strip_prefix(worktree) {
        let mut dir = worktree.to_path_buf();
        let mut chain = vec![dir.clone()];
        for c in below.components() {
            dir.push(c);
            chain.push(dir.clone());
        }
        chain.pop();
        for d in chain {
            if CONTROL_FILES.iter().any(|f| placeholder(&d.join(f))) {
                return dataless();
            }
        }
    }
    let mut facts = TreeFacts::default();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(library.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let entries: Vec<std::fs::DirEntry> = rd.filter_map(Result::ok).collect();
        seen += entries.len();
        if seen > PREWALK_ENTRIES_MAX || depth > PREWALK_DEPTH_MAX {
            return Err(GitState::Unavailable(GitError::Failed(FailKind::TooLarge)));
        }
        let join = |name: &str| {
            if rel.is_empty() {
                name.to_owned()
            } else {
                format!("{rel}/{name}")
            }
        };
        // A nested repository: checked, not entered (gix does not enter it either).
        if dir != worktree && entries.iter().any(|e| e.file_name() == ".git") {
            check_nested(&dir.join(".git"))?;
            continue;
        }
        for e in &entries {
            let name = e.file_name();
            let Ok(ft) = e.file_type() else { continue };
            let name_str = name.to_str();
            if let Some(n) = name_str.filter(|n| CONTROL_FILES.contains(n))
                && (extra.contains(&join(n)) || placeholder(&e.path()))
            {
                return dataless();
            }
            if name == ".git" {
                continue;
            }
            match name_str {
                Some(n) if ft.is_dir() => stack.push((e.path(), join(n), depth + 1)),
                Some(n) if !ft.is_file() && !ft.is_symlink() && !ft.is_dir() => {
                    facts.not_files.push(join(n));
                }
                _ => {}
            }
        }
    }
    Ok(facts)
}

/// A nested repository's `.git` (I3): a real directory whose `HEAD` is regular and capped, or a
/// regular capped gitfile whose target's `HEAD` is.
fn check_nested(dot_git: &Path) -> Result<(), GitState> {
    let Ok(md) = std::fs::symlink_metadata(dot_git) else {
        return Ok(());
    };
    let ft = md.file_type();
    if ft.is_symlink() {
        return redirected();
    }
    if ft.is_dir() {
        regular_capped(&dot_git.join("HEAD"), REF_MAX)?;
        return Ok(());
    }
    if !ft.is_file() {
        return Ok(()); // a FIFO or socket named `.git` is no repository marker
    }
    let Some(text) = capped_text(dot_git, REF_MAX)? else {
        return Ok(());
    };
    let target = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)
        .unwrap_or("");
    let base = dot_git.parent().unwrap_or(dot_git);
    if let Some(t) = resolve(base, target) {
        regular_capped(&t.join("HEAD"), REF_MAX)?;
    }
    Ok(())
}

/// m10: the owners checked before the open, checked again on what gix opened.
pub(crate) fn check_owners_again(
    repo: &gix::Repository,
    worktree: &Path,
    owned: Owner<'_>,
) -> Result<(), GitState> {
    require_owned(worktree, owned)?;
    require_owned(repo.git_dir(), owned)?;
    require_owned(repo.common_dir(), owned)?;
    Ok(())
}
// ## m4b-t07-git

fn io_failed(_: std::io::Error) -> GitState {
    GitState::Unavailable(GitError::Failed(FailKind::Open))
}

/// Where a gitfile may lead (n2): a linked worktree's git dir, a submodule's git dir, or a
/// directory inside the worktree.
fn gitfile_layout(found: &Found, owned: Owner<'_>) -> Result<Layout, GitState> {
    let Some(text) = small_text(&found.dot_git) else {
        return redirected();
    };
    let Some(target) = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|t| !t.is_empty())
    else {
        return redirected();
    };
    let Some(t) = resolve(&found.worktree, target) else {
        return redirected();
    };
    require_owned(&t, owned)?;
    let commondir = t.join("commondir");
    // A linked worktree: `<common>/worktrees/<name>`, whose `commondir` leads back.
    let parent = t.parent();
    if parent
        .and_then(Path::file_name)
        .is_some_and(|n| n == "worktrees")
    {
        let common = parent.and_then(Path::parent).map(Path::to_path_buf);
        let Some(text) = small_text(&commondir) else {
            return redirected();
        };
        let back = resolve(&t, text.trim());
        return match (common, back) {
            (Some(c), Some(b)) if c == b => Ok(Layout {
                git_dir: t,
                common_dir: c,
            }),
            _ => redirected(),
        };
    }
    if exists(&commondir) {
        return redirected();
    }
    let inside_worktree = t.starts_with(&found.worktree);
    let submodule = found
        .worktree
        .ancestors()
        .skip(1)
        .any(|a| t.starts_with(a.join(".git").join("modules")));
    if inside_worktree || submodule {
        Ok(Layout {
            git_dir: t.clone(),
            common_dir: t,
        })
    } else {
        redirected()
    }
}

/// Every alternate object directory (`objects/info/alternates`, one path per line, relative
/// to `objects`) is an existing directory the user owns; chains are followed to
/// [`ALTERNATES_DEPTH_MAX`].
fn check_alternates(objects: &Path, depth: usize, owned: Owner<'_>) -> Result<(), GitState> {
    let file = objects.join("info").join("alternates");
    if !exists(&file) {
        return Ok(());
    }
    let Some(text) = small_text(&file) else {
        return redirected();
    };
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // The C-quoted form is not followed.
        if line.starts_with('"') || depth >= ALTERNATES_DEPTH_MAX {
            return redirected();
        }
        let Some(alt) = resolve(objects, line) else {
            return redirected();
        };
        require_owned(&alt, owned)?;
        check_alternates(&alt, depth + 1, owned)?;
    }
    Ok(())
}

/// The trust invariants after the open (C1, I1).
pub(crate) fn check_trust(repo: &gix::Repository) -> Result<(), GitError> {
    if repo.git_dir_trust() != gix::sec::Trust::Reduced {
        return Err(GitError::TrustPromoted);
    }
    if promoted_sections(repo.config_snapshot().plumbing()) {
        return Err(GitError::TrustPromoted);
    }
    Ok(())
}

/// Whether any repository-kind section of `config` has full trust.
pub(crate) fn promoted_sections(config: &gix::config::File) -> bool {
    config.sections().any(|s| {
        s.meta().source.kind() == gix::config::source::Kind::Repository
            && s.meta().trust == gix::sec::Trust::Full
    })
}

/// The directories `gix` resolved are the ones [`check_layout`] accepted.
pub(crate) fn check_resolved(
    repo: &gix::Repository,
    layout: &Layout,
    worktree: &Path,
) -> Result<(), GitError> {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let same = canon(repo.git_dir()).as_deref() == Some(layout.git_dir.as_path())
        && canon(repo.common_dir()).as_deref() == Some(layout.common_dir.as_path())
        && repo.workdir().and_then(canon).as_deref() == Some(worktree);
    if same {
        Ok(())
    } else {
        Err(GitError::Redirected)
    }
}

/// An object format or ref storage this build cannot read (m2), from the repository's own
/// config file as written (read raw, no includes, whatever its trust). A config that cannot be
/// read counts as unsupported, fail closed (fix round 1, m4): the common dir's `config` missing,
/// not a regular file (a symlink, FIFO, device), over [`TEXT_MAX`], or not parseable; the same
/// for the git dir's `config.worktree` when present.
pub(crate) fn unsupported_format(git_dir: &Path, common_dir: &Path) -> bool {
    use gix::bstr::ByteSlice as _;
    let parse = |path: PathBuf, required: bool| -> Result<Option<gix::config::File>, ()> {
        match regular_capped(&path, TEXT_MAX) {
            Ok(true) => {}
            Ok(false) if !required => return Ok(None),
            _ => return Err(()),
        }
        gix::config::File::from_path_no_includes(path, gix::config::Source::Local)
            .map(Some)
            .map_err(|_| ())
    };
    let Ok(Some(file)) = parse(common_dir.join("config"), true) else {
        return true;
    };
    if parse(git_dir.join("config.worktree"), false).is_err() {
        return true;
    }
    let other = |key: &str, ok: &str| {
        file.string(key)
            .is_some_and(|v| !v.trim().eq_ignore_ascii_case(ok.as_bytes()))
    };
    other("extensions.objectFormat", "sha1") || other("extensions.refStorage", "files")
}
// ## m4b-t07-git

#[cfg(test)]
mod tests {
    use super::*;

    fn found(worktree: &Path, kind: DotGit) -> Found {
        Found {
            worktree: worktree.to_path_buf(),
            dot_git: worktree.join(".git"),
            kind,
            prefix: String::new(),
        }
    }

    fn all_owned(_: &Path) -> std::io::Result<bool> {
        Ok(true)
    }

    fn top() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let p = std::fs::canonicalize(t.path()).unwrap();
        (t, p)
    }

    #[test]
    fn foreign_owner_of_dot_git_or_worktree_is_untrusted() {
        let (_t, top) = top();
        std::fs::create_dir_all(top.join("wt/.git/objects")).unwrap();
        let f = found(&top.join("wt"), DotGit::Dir);
        for foreign in [top.join("wt"), top.join("wt/.git")] {
            let owner = |p: &Path| Ok(p != foreign);
            assert_eq!(
                check_layout(&f, &owner).unwrap_err().into_cmp(),
                GitState::Untrusted(UntrustReason::ForeignOwner).into_cmp(),
                "{foreign:?}"
            );
        }
        assert!(check_layout(&f, &all_owned).is_ok());
    }

    #[test]
    fn foreign_owner_of_an_alternate_is_untrusted() {
        let (_t, top) = top();
        std::fs::create_dir_all(top.join("wt/.git/objects/info")).unwrap();
        std::fs::create_dir_all(top.join("alt/objects")).unwrap();
        std::fs::write(
            top.join("wt/.git/objects/info/alternates"),
            format!("{}\n", top.join("alt/objects").display()),
        )
        .unwrap();
        let f = found(&top.join("wt"), DotGit::Dir);
        assert!(check_layout(&f, &all_owned).is_ok());
        let alt = top.join("alt/objects");
        let owner = |p: &Path| Ok(p != alt);
        assert_eq!(
            check_layout(&f, &owner).unwrap_err().into_cmp(),
            GitState::Untrusted(UntrustReason::ForeignOwner).into_cmp()
        );
    }

    #[test]
    fn an_unreadable_owner_is_unavailable() {
        let (_t, top) = top();
        std::fs::create_dir_all(top.join("wt/.git")).unwrap();
        let f = found(&top.join("wt"), DotGit::Dir);
        let owner = |_: &Path| Err(std::io::Error::other("no owner"));
        assert!(matches!(
            check_layout(&f, &owner),
            Err(GitState::Unavailable(GitError::Failed(_)))
        ));
    }

    #[test]
    fn linked_worktree_gitfile_is_followed_and_a_wrong_commondir_is_not() {
        let (_t, top) = top();
        let common = top.join("main/.git");
        let wtdir = common.join("worktrees/side");
        std::fs::create_dir_all(&wtdir).unwrap();
        std::fs::create_dir_all(common.join("objects")).unwrap();
        std::fs::write(wtdir.join("commondir"), "../..\n").unwrap();
        std::fs::create_dir_all(top.join("side")).unwrap();
        std::fs::write(
            top.join("side/.git"),
            format!("gitdir: {}\n", wtdir.display()),
        )
        .unwrap();
        let f = found(&top.join("side"), DotGit::File);
        assert_eq!(
            check_layout(&f, &all_owned).unwrap(),
            Layout {
                git_dir: wtdir.clone(),
                common_dir: common.clone()
            }
        );
        // `commondir` leading anywhere else: refused.
        std::fs::create_dir_all(top.join("other/.git")).unwrap();
        std::fs::write(
            wtdir.join("commondir"),
            top.join("other/.git").display().to_string(),
        )
        .unwrap();
        assert_eq!(
            check_layout(&f, &all_owned).unwrap_err().into_cmp(),
            GitState::Unavailable(GitError::Redirected).into_cmp()
        );
    }

    #[test]
    fn submodule_gitfile_is_followed_and_one_elsewhere_is_not() {
        let (_t, top) = top();
        let modules = top.join("super/.git/modules/sub");
        std::fs::create_dir_all(modules.join("objects")).unwrap();
        std::fs::create_dir_all(top.join("super/sub")).unwrap();
        std::fs::write(top.join("super/sub/.git"), "gitdir: ../.git/modules/sub\n").unwrap();
        let f = found(&top.join("super/sub"), DotGit::File);
        assert_eq!(check_layout(&f, &all_owned).unwrap().git_dir, modules);
        // A gitfile into an unrelated repository: refused.
        std::fs::create_dir_all(top.join("elsewhere/.git")).unwrap();
        std::fs::write(
            top.join("super/sub/.git"),
            format!("gitdir: {}\n", top.join("elsewhere/.git").display()),
        )
        .unwrap();
        assert_eq!(
            check_layout(&f, &all_owned).unwrap_err().into_cmp(),
            GitState::Unavailable(GitError::Redirected).into_cmp()
        );
        // Not a gitfile at all.
        std::fs::write(top.join("super/sub/.git"), "hello\n").unwrap();
        assert!(check_layout(&f, &all_owned).is_err());
    }

    #[test]
    fn a_symlinked_dot_git_is_refused() {
        let (_t, top) = top();
        std::fs::create_dir_all(top.join("wt")).unwrap();
        let f = found(&top.join("wt"), DotGit::Symlink);
        assert_eq!(
            check_layout(&f, &all_owned).unwrap_err().into_cmp(),
            GitState::Unavailable(GitError::Redirected).into_cmp()
        );
    }

    #[test]
    fn deep_or_quoted_alternates_are_refused() {
        let (_t, top) = top();
        // A chain of five alternates: one more than allowed.
        let mut prev = top.join("wt/.git/objects");
        std::fs::create_dir_all(prev.join("info")).unwrap();
        for i in 0..5 {
            let next = top.join(format!("alt{i}/objects"));
            std::fs::create_dir_all(next.join("info")).unwrap();
            std::fs::write(prev.join("info/alternates"), next.display().to_string()).unwrap();
            prev = next;
        }
        let f = found(&top.join("wt"), DotGit::Dir);
        assert!(check_layout(&f, &all_owned).is_err());
        std::fs::write(
            top.join("wt/.git/objects/info/alternates"),
            "\"quoted\\npath\"\n",
        )
        .unwrap();
        assert!(check_layout(&f, &all_owned).is_err());
    }

    /// Opening with `isolated()` alone takes trust from ownership: full. The check refuses it
    /// (the "forced promotion" of `trust_is_reduced_even_with_global_safe_directory_star`).
    #[test]
    fn a_fully_trusted_open_is_refused() {
        let (_t, top) = top();
        gix::init(&top).unwrap();
        let full = gix::open_opts(&top, gix::open::Options::isolated()).unwrap();
        assert_eq!(full.git_dir_trust(), gix::sec::Trust::Full);
        assert_eq!(check_trust(&full), Err(GitError::TrustPromoted));
        let reduced = gix::open_opts(
            &top,
            gix::open::Options::isolated().with(gix::sec::Trust::Reduced),
        )
        .unwrap();
        assert_eq!(check_trust(&reduced), Ok(()));
        assert!(!promoted_sections(reduced.config_snapshot().plumbing()));
    }

    /// A single repository section with full trust (what a `safe.directory` matching the config
    /// file would make) is refused even while the repository's own trust is reduced (I1).
    #[test]
    fn a_promoted_repository_section_is_refused() {
        let meta = |source, trust| gix::config::file::Metadata {
            path: None,
            source,
            level: 0,
            trust,
        };
        let mut f =
            gix::config::File::new(meta(gix::config::Source::Local, gix::sec::Trust::Reduced));
        f.new_section("core", None).unwrap();
        assert!(!promoted_sections(&f));
        f.new_section_with_meta(
            "filter",
            Some("x".into()),
            meta(gix::config::Source::User, gix::sec::Trust::Full),
        )
        .unwrap();
        assert!(
            !promoted_sections(&f),
            "a user section is not a repository section"
        );
        f.new_section_with_meta(
            "filter",
            Some("y".into()),
            meta(gix::config::Source::Local, gix::sec::Trust::Full),
        )
        .unwrap();
        assert!(promoted_sections(&f));
    }

    #[test]
    fn formats_this_build_cannot_read_are_unsupported() {
        let (_t, top) = top();
        let cfg = top.join("config");
        // m4: no config at all is unreadable, so unsupported.
        assert!(unsupported_format(&top, &top));
        std::fs::write(&cfg, "[core]\n\trepositoryformatversion = 0\n").unwrap();
        assert!(!unsupported_format(&top, &top));
        std::fs::write(&cfg, "[extensions]\n\tobjectFormat = sha1\n").unwrap();
        assert!(!unsupported_format(&top, &top));
        std::fs::write(&cfg, "[extensions]\n\tobjectFormat = sha256\n").unwrap();
        assert!(unsupported_format(&top, &top));
        std::fs::write(&cfg, "[extensions]\n\trefStorage = reftable\n").unwrap();
        assert!(unsupported_format(&top, &top));
        // m4: unparseable, too large, a broken config.worktree.
        std::fs::write(&cfg, "[core\n").unwrap();
        assert!(unsupported_format(&top, &top));
        std::fs::write(&cfg, format!("# {}\n", "x".repeat(2 << 20))).unwrap();
        assert!(unsupported_format(&top, &top));
        std::fs::write(&cfg, "[core]\n").unwrap();
        std::fs::write(top.join("config.worktree"), "[[[").unwrap();
        assert!(unsupported_format(&top, &top));
    }

    #[cfg(unix)]
    fn layout_of(g: &Path) -> Layout {
        Layout {
            git_dir: g.to_path_buf(),
            common_dir: g.to_path_buf(),
        }
    }

    /// I3: HEAD, the ref it names, `packed-refs` and the index must be regular files under
    /// their caps; a symlink is refused before anything reads it.
    #[cfg(unix)]
    #[test]
    fn repository_files_must_be_regular_and_capped() {
        let (_t, top) = top();
        let g = top.join("g");
        std::fs::create_dir_all(g.join("refs/heads")).unwrap();
        std::fs::write(g.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(g.join("refs/heads/main"), "0".repeat(40)).unwrap();
        assert!(check_repository_files(&layout_of(&g)).is_ok());
        let elsewhere = top.join("elsewhere");
        std::fs::write(&elsewhere, "x").unwrap();
        for f in ["refs/heads/main", "packed-refs", "index"] {
            let p = g.join(f);
            let _ = std::fs::remove_file(&p);
            std::os::unix::fs::symlink(&elsewhere, &p).unwrap();
            assert_eq!(
                check_repository_files(&layout_of(&g))
                    .unwrap_err()
                    .into_cmp(),
                GitState::Unavailable(GitError::Redirected).into_cmp(),
                "{f}"
            );
            std::fs::remove_file(&p).unwrap();
        }
        // A symlinked folder on the way to HEAD's ref.
        std::fs::remove_dir_all(g.join("refs/heads")).unwrap();
        std::os::unix::fs::symlink(&top, g.join("refs/heads")).unwrap();
        assert!(check_repository_files(&layout_of(&g)).is_err());
        // A ref outside `refs/`.
        std::fs::write(g.join("HEAD"), "ref: ../../etc/x\n").unwrap();
        assert!(check_repository_files(&layout_of(&g)).is_err());
        // A loose ref over its cap.
        std::fs::remove_file(g.join("refs/heads")).unwrap();
        std::fs::create_dir_all(g.join("refs/heads")).unwrap();
        std::fs::write(g.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(g.join("refs/heads/main"), "0".repeat(8192)).unwrap();
        assert_eq!(
            check_repository_files(&layout_of(&g))
                .unwrap_err()
                .into_cmp(),
            GitState::Unavailable(GitError::Failed(FailKind::TooLarge)).into_cmp()
        );
    }

    /// m10: ownership is checked again on the directories gix opened.
    #[test]
    fn ownership_is_checked_again_after_the_open() {
        let (_t, top) = top();
        gix::init(&top).unwrap();
        let repo = gix::open_opts(
            &top,
            gix::open::Options::isolated().with(gix::sec::Trust::Reduced),
        )
        .unwrap();
        assert!(check_owners_again(&repo, &top, &all_owned).is_ok());
        let git_dir = repo.git_dir().to_path_buf();
        let owner = |p: &Path| Ok(p != git_dir);
        assert_eq!(
            check_owners_again(&repo, &top, &owner)
                .unwrap_err()
                .into_cmp(),
            GitState::Untrusted(UntrustReason::ForeignOwner).into_cmp()
        );
    }

    /// `GitState` has no `PartialEq` (it holds an `Arc`); compare the shape as text.
    trait IntoCmp {
        fn into_cmp(self) -> String;
    }
    impl IntoCmp for GitState {
        fn into_cmp(self) -> String {
            format!("{self:?}")
        }
    }
}
