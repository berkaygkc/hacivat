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
// ## m4b-t20-adversarial fix round 1 (named line, I1): the worker's interrupt flag.
use std::sync::atomic::{AtomicBool, Ordering};

use crate::discover::{DotGit, Found};
use crate::{FailKind, GitError, GitState, TooLargeWhy, UntrustReason};

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
// ## m4b-t20-adversarial (named exception)
/// The most distinct alternate object folders one repository may name, its chain included
/// (`git clone --shared` or `--reference` make one; a few is generous).
const ALTERNATES_MAX: usize = 8;
/// ## m4b-t20-adversarial fix round 1 (review I1): the most distinct alternates lines (raw
/// text, comments and blanks left out) the whole chain may hold, counted **before** any line
/// is resolved: a line's resolution (`canonicalize`, up to `MAXSYMLINKS` hops of any length)
/// is the expensive step, so a file of many cheap lines is refused at once. Real clones have
/// one or two.
const ALTERNATE_LINES_MAX: usize = 4 * ALTERNATES_MAX;
/// ## m4b-t20-adversarial fix round 1 (review I1): the most entries an object folder's `pack`
/// folder may hold for the placeholder check (three files per pack; `git gc` keeps one pack).
/// Past it the status answers `TooLarge(LooseObjects)` ("run git gc"), which repacks them.
const PACK_ENTRIES_MAX: usize = 20_000;

/// A flag never set (the unit tests, which have no worker to interrupt).
#[cfg(test)]
static NEVER: AtomicBool = AtomicBool::new(false);

/// The answer of a worker whose caller gave up (it is discarded; the caller already answered
/// `Timeout`): stop now, never finish the work.
fn interrupted(interrupt: &AtomicBool) -> Result<(), GitState> {
    if interrupt.load(Ordering::SeqCst) {
        Err(GitState::Unavailable(GitError::Timeout))
    } else {
        Ok(())
    }
}

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

/// The checks before the open. `Err` is the state to answer. Every small file read here (a
/// gitfile, `commondir`, alternates) is checked for a cloud placeholder first (p1), and so is
/// every alternate's object folder.
#[cfg(test)]
pub(crate) fn check_layout(
    found: &Found,
    owned: Owner<'_>,
    placeholder: Placeholder<'_>,
) -> Result<Layout, GitState> {
    check_layout_bounded(found, owned, placeholder, &NEVER)
}

/// [`check_layout`], stopping as soon as `interrupt` is set (the status worker's flag, set when
/// its caller answered `Timeout`). ## m4b-t20-adversarial fix round 1 (named exception, I1).
pub(crate) fn check_layout_bounded(
    found: &Found,
    owned: Owner<'_>,
    placeholder: Placeholder<'_>,
    interrupt: &AtomicBool,
) -> Result<Layout, GitState> {
    require_owned(&found.worktree, owned)?;
    require_owned(&found.dot_git, owned)?;
    let layout = match found.kind {
        DotGit::Symlink => return redirected(),
        DotGit::Dir => {
            let g = std::fs::canonicalize(&found.dot_git).map_err(io_failed)?;
            if placeholder(&g.join("commondir")) {
                return dataless();
            }
            if exists(&g.join("commondir")) {
                return redirected();
            }
            Layout {
                git_dir: g.clone(),
                common_dir: g,
            }
        }
        DotGit::File => {
            if placeholder(&found.dot_git) {
                return dataless();
            }
            gitfile_layout(found, owned, placeholder)?
        }
    };
    require_owned(&layout.git_dir, owned)?;
    require_owned(&layout.common_dir, owned)?;
    check_alternates(
        &layout.common_dir.join("objects"),
        owned,
        placeholder,
        interrupt,
    )?;
    Ok(layout)
}

/// The most loose objects an object folder may hold for the placeholder check (G2, R-T7-6).
/// `git gc --auto` keeps them near 6,700; past this the status answers
/// `TooLarge(LooseObjects)` ("too many loose objects; run git gc").
const LOOSE_OBJECTS_MAX: usize = 20_000;

/// G2 (R-T7-6), p1: an object folder's packs and loose objects, by `lstat`: the `pack` folder
/// and its entries, every two-hex-digit fan-out folder and its entries. gix reads loose
/// objects on every status (the HEAD commit, every tree reachable from it), so a placeholder
/// among them answers `Unavailable(Dataless)`. More than [`LOOSE_OBJECTS_MAX`] loose entries
/// answers `Unavailable(TooLarge(LooseObjects))`.
///
/// ## m4b-t20-adversarial fix round 1 (named exception, review I1): `pack` entries are counted
/// too (more than [`PACK_ENTRIES_MAX`] answers the same `TooLarge(LooseObjects)`), and the
/// interrupt flag is read on every entry.
fn check_objects_dataless(
    objects: &Path,
    placeholder: Placeholder<'_>,
    interrupt: &AtomicBool,
) -> Result<(), GitState> {
    if placeholder(objects) {
        return dataless();
    }
    let Ok(rd) = std::fs::read_dir(objects) else {
        return Ok(());
    };
    let mut loose = 0usize;
    let mut packed = 0usize;
    for fan in rd.filter_map(Result::ok) {
        interrupted(interrupt)?;
        let name = fan.file_name();
        let Some(n) = name.to_str() else { continue };
        let is_fan = n.len() == 2 && n.bytes().all(|b| b.is_ascii_hexdigit());
        if !is_fan && n != "pack" {
            continue;
        }
        if placeholder(&fan.path()) {
            return dataless();
        }
        let Ok(inner) = std::fs::read_dir(fan.path()) else {
            continue;
        };
        for e in inner.filter_map(Result::ok) {
            interrupted(interrupt)?;
            let (count, max) = if is_fan {
                (&mut loose, LOOSE_OBJECTS_MAX)
            } else {
                (&mut packed, PACK_ENTRIES_MAX)
            };
            *count += 1;
            if *count > max {
                return Err(GitState::Unavailable(GitError::TooLarge(
                    TooLargeWhy::LooseObjects,
                )));
            }
            if placeholder(&e.path()) {
                return dataless();
            }
        }
    }
    Ok(())
}

// ## m4b-t07-git (fix round 1, I3, R-T7-2)
/// The cap on a ref-sized file: `HEAD`, a loose ref, a gitfile, `commondir`.
const REF_MAX: u64 = 4 * 1024;
/// The cap on `packed-refs`, `info/exclude`, `info/attributes`, `config`, `config.worktree`.
const TEXT_MAX: u64 = 1024 * 1024;
/// gix's allocation limit under reduced trust (`gitoxide.objects.allocLimitIfReducedTrust`,
/// default 16 MiB; the repository cannot raise it: the key is read from trusted sections only
/// and `GIT_ALLOC_LIMIT` is denied). gix refuses an index whose decoded entries exceed it
/// (`entries × size_of::<Entry>()`), so this is the status's real ceiling (fix round 2, n1).
const GIX_ALLOC_LIMIT: u64 = 16 * 1024 * 1024;
/// The cap on the index file: the same limit (its entries and paths both fit under it when gix
/// can decode it).
const INDEX_MAX: u64 = GIX_ALLOC_LIMIT;

/// Whether a path is a cloud placeholder (the real flag, or the `testing` hook's set).
pub(crate) type Placeholder<'a> = &'a dyn Fn(&Path) -> bool;

fn dataless<T>() -> Result<T, GitState> {
    Err(GitState::Unavailable(GitError::Dataless))
}
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
pub(crate) fn check_repository_files(
    layout: &Layout,
    placeholder: Placeholder<'_>,
) -> Result<(), GitState> {
    let (g, c) = (&layout.git_dir, &layout.common_dir);
    check_index_size(&g.join("index"))?;
    // p1: a split index's shared part is read like the index: a FIFO never hangs.
    for shared in shared_indexes(g) {
        regular_capped(&shared, INDEX_MAX)?;
    }
    // G1 (R-T7-5): `refs/replace` is never scanned (`core.useReplaceRefs=false`), and a
    // symlinked one is refused, should anything walk it.
    if std::fs::symlink_metadata(c.join("refs").join("replace"))
        .is_ok_and(|m| !m.file_type().is_dir())
    {
        return redirected();
    }
    regular_capped(&c.join("packed-refs"), TEXT_MAX)?;
    regular_capped(&c.join("info").join("exclude"), TEXT_MAX)?;
    regular_capped(&c.join("info").join("attributes"), TEXT_MAX)?;
    let Some(head) = capped_text(&g.join("HEAD"), REF_MAX)? else {
        return Ok(());
    };
    check_symref_chain(head.trim(), layout, 0, placeholder)
}

/// n1: an index larger than gix decodes under reduced trust answers `Unavailable(TooLarge)`
/// (a diagnostic), never a silent `Failed`: the file over [`INDEX_MAX`], or its header's entry
/// count times `size_of::<Entry>()` over [`GIX_ALLOC_LIMIT`]. Reads the 12-byte header only.
fn check_index_size(index: &Path) -> Result<(), GitState> {
    let too_large = || {
        Err(GitState::Unavailable(GitError::TooLarge(
            TooLargeWhy::Index,
        )))
    };
    match regular_capped(index, INDEX_MAX) {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(GitState::Unavailable(GitError::Failed(FailKind::TooLarge))) => return too_large(),
        Err(e) => return Err(e),
    }
    let mut header = [0u8; 12];
    std::fs::File::open(index)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut header))
        .map_err(io_failed)?;
    let entries = u64::from(u32::from_be_bytes([
        header[8], header[9], header[10], header[11],
    ]));
    let entry_size = std::mem::size_of::<gix::index::Entry>() as u64;
    if &header[..4] == b"DIRC" && entries.saturating_mul(entry_size) > GIX_ALLOC_LIMIT {
        return too_large();
    }
    Ok(())
}

/// F3 (R-T7-4, "never download"): the repository's own files that the open, the index read and
/// the HEAD lookup open are checked with `lstat` first; a cloud placeholder among them answers
/// `Unavailable(Dataless)`: `HEAD`, `config`, `config.worktree`, the index, `packed-refs`,
/// `info/exclude`, `info/attributes`, and every entry of `objects/pack` (the folder itself
/// too). Loose objects are not checked (thousands of `lstat`s; a recorded residual). The refs
/// on HEAD's chain are checked by [`check_symref_chain`].
pub(crate) fn check_internals_dataless(
    layout: &Layout,
    placeholder: Placeholder<'_>,
    // ## m4b-t20-adversarial fix round 1 (named line, I1)
    interrupt: &AtomicBool,
) -> Result<(), GitState> {
    let (g, c) = (&layout.git_dir, &layout.common_dir);
    let files = [
        g.join("HEAD"),
        c.join("config"),
        g.join("config.worktree"),
        g.join("index"),
        c.join("packed-refs"),
        c.join("info").join("exclude"),
        c.join("info").join("attributes"),
    ];
    if files.iter().any(|f| placeholder(f)) {
        return dataless();
    }
    // p1: a split index's `sharedindex.<sha>`.
    for shared in shared_indexes(g) {
        if placeholder(&shared) {
            return dataless();
        }
    }
    // G2: packs and loose objects (fix round 3; the fr2 residual was wrong: gix reads loose
    // objects on every status).
    check_objects_dataless(&c.join("objects"), placeholder, interrupt)
}

/// The git dir's `sharedindex.*` files (a split index).
fn shared_indexes(git_dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(git_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("sharedindex."))
                })
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// A symbolic ref's target (`ref: refs/…`) and its chain: every directory on the way is a real
/// directory, and the ref file, when loose, a regular file under [`REF_MAX`].
/// A ref that is a cloud placeholder answers `Unavailable(Dataless)` before it is read (F3).
fn check_symref_chain(
    text: &str,
    layout: &Layout,
    depth: usize,
    placeholder: Placeholder<'_>,
) -> Result<(), GitState> {
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
        let file = dir.join(comps[comps.len() - 1]);
        if placeholder(&file) {
            return dataless();
        }
        if let Some(text) = capped_text(&file, REF_MAX)? {
            check_symref_chain(text.trim(), layout, depth + 1, placeholder)?;
        }
    }
    Ok(())
}

/// The most entries the library pre-walk lists before it gives up (fail closed).
const PREWALK_ENTRIES_MAX: usize = 200_000;
/// The deepest folder the pre-walk enters.
const PREWALK_DEPTH_MAX: usize = 64;
/// The most dataless folders a library may hold (each becomes one pathspec exclusion; few keep
/// the pathspec match linear, n5).
const DATALESS_DIRS_MAX: usize = 256;
/// The in-tree files gix reads to decide what is ignored, filtered or a submodule.
const CONTROL_FILES: [&str; 3] = [".gitignore", ".gitattributes", ".gitmodules"];

/// What the library pre-walk found that the status reports or excludes itself.
#[derive(Debug, Default)]
pub(crate) struct TreeFacts {
    /// Library-relative names (the disk's spelling) of FIFOs, sockets and devices (m5).
    pub(crate) not_files: Vec<String>,
    /// Library-relative dataless folders, listed by nobody (n5).
    pub(crate) dataless_dirs: Vec<String>,
}

/// A control file gix would read (F1 completes R-T7-2, I4 / R-T7-3): absent, or a regular file
/// of at most [`TEXT_MAX`] that is not a cloud placeholder. A FIFO, socket, device or symlink
/// answers `Unavailable(Redirected)` at once (no hang), an oversize file `Failed(TooLarge)`
/// (gix-glob reads it whole with no cap), a placeholder `Unavailable(Dataless)`.
fn check_control_file(p: &Path, placeholder: Placeholder<'_>) -> Result<(), GitState> {
    if placeholder(p) {
        return dataless();
    }
    regular_capped(p, TEXT_MAX).map(|_| ())
}

/// The walk of the library's folders (names and `lstat` only) before gix walks it:
/// - every control file (`.gitignore`, `.gitattributes`, `.gitmodules`) here and in the folders
///   between the worktree and the library goes through [`check_control_file`];
/// - a nested repository's `.git` (gix classifies it by reading its `HEAD` with no limit) must
///   be a real directory with a regular, capped `HEAD`, or a regular, capped gitfile whose
///   target's `HEAD` is the same; otherwise `Unavailable(Redirected)` (I3); a placeholder among
///   them is `Unavailable(Dataless)` (F3);
/// - FIFOs, sockets and devices are recorded (gix lists none of them; m5);
/// - a dataless folder is recorded and not listed (n5, as T4's scanner): the status excludes it
///   from gix's walk too. More than [`DATALESS_DIRS_MAX`] of them answers
///   `Unavailable(Dataless)`.
///
/// Symlinked folders are not entered (gix does not follow them), nor nested repositories, nor
/// `.git`. More than [`PREWALK_ENTRIES_MAX`] entries (counted while listing, so one huge folder
/// is never buffered whole, n4) or a folder deeper than [`PREWALK_DEPTH_MAX`] answers
/// `Failed(TooLarge)`.
pub(crate) fn check_library_tree(
    worktree: &Path,
    library: &Path,
    placeholder: Placeholder<'_>,
) -> Result<TreeFacts, GitState> {
    let too_large = || Err(GitState::Unavailable(GitError::Failed(FailKind::TooLarge)));
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
            for f in CONTROL_FILES {
                check_control_file(&d.join(f), placeholder)?;
            }
        }
    }
    let mut facts = TreeFacts::default();
    let mut seen = 0usize;
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(library.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel, depth)) = stack.pop() {
        if depth > PREWALK_DEPTH_MAX {
            return too_large();
        }
        let join = |name: &str| {
            if rel.is_empty() {
                name.to_owned()
            } else {
                format!("{rel}/{name}")
            }
        };
        // A nested repository: checked, not entered (gix does not enter it either).
        let dot_git = dir.join(".git");
        if dir != worktree && std::fs::symlink_metadata(&dot_git).is_ok() {
            check_nested(&dot_git, placeholder)?;
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            seen += 1;
            if seen > PREWALK_ENTRIES_MAX {
                return too_large();
            }
            let name = e.file_name();
            let Ok(ft) = e.file_type() else { continue };
            let Some(n) = name.to_str() else { continue };
            if CONTROL_FILES.contains(&n) {
                check_control_file(&e.path(), placeholder)?;
            }
            if n == ".git" {
                continue;
            }
            if ft.is_dir() {
                if placeholder(&e.path()) {
                    facts.dataless_dirs.push(join(n));
                    if facts.dataless_dirs.len() > DATALESS_DIRS_MAX {
                        return dataless();
                    }
                } else {
                    stack.push((e.path(), join(n), depth + 1));
                }
            } else if !ft.is_file() && !ft.is_symlink() {
                facts.not_files.push(join(n));
            }
        }
    }
    Ok(facts)
}

/// A nested repository's `.git` (I3, F3): a real directory whose `HEAD` is regular, capped and
/// no placeholder, or a regular capped gitfile (no placeholder) whose target's `HEAD` is.
fn check_nested(dot_git: &Path, placeholder: Placeholder<'_>) -> Result<(), GitState> {
    let Ok(md) = std::fs::symlink_metadata(dot_git) else {
        return Ok(());
    };
    let ft = md.file_type();
    if ft.is_symlink() {
        return redirected();
    }
    if placeholder(dot_git) {
        return dataless();
    }
    let head_ok = |head: &Path| {
        if placeholder(head) {
            return dataless();
        }
        regular_capped(head, REF_MAX).map(|_| ())
    };
    if ft.is_dir() {
        return head_ok(&dot_git.join("HEAD"));
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
        head_ok(&t.join("HEAD"))?;
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
fn gitfile_layout(
    found: &Found,
    owned: Owner<'_>,
    placeholder: Placeholder<'_>,
) -> Result<Layout, GitState> {
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
    // p1: the linked worktree's `commondir` (read just below) and `gitdir` back-pointer.
    if placeholder(&commondir) || placeholder(&t.join("gitdir")) {
        return dataless();
    }
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
///
/// ## m4b-t20-adversarial (named exception, Task 20's probe from the T7 review's q2): each
/// alternate folder is checked once (`seen`, by canonical path), and more than
/// [`ALTERNATES_MAX`] distinct ones refuse the repository. The depth bound alone let a file of
/// many lines fan out to `lines^depth` folder checks, a worker that outlived every budget.
///
/// ## m4b-t20-adversarial fix round 1 (review I1): the work is bounded before any line is
/// resolved. Each file's lines are first de-duplicated by their raw text, and the distinct
/// lines of the whole chain are counted: more than [`ALTERNATE_LINES_MAX`] refuse the
/// repository at once, before a single `canonicalize`. The worker's interrupt flag is read
/// before each resolution and inside every folder walk, so a worker whose caller answered
/// `Timeout` stops instead of finishing.
fn check_alternates(
    objects: &Path,
    owned: Owner<'_>,
    placeholder: Placeholder<'_>,
    interrupt: &AtomicBool,
) -> Result<(), GitState> {
    let mut walk = AltWalk {
        owned,
        placeholder,
        interrupt,
        seen: Vec::new(),
        lines: 0,
    };
    walk.visit(objects, 0)
}

/// One repository's alternates chain, with what it has spent so far.
struct AltWalk<'a> {
    owned: Owner<'a>,
    placeholder: Placeholder<'a>,
    interrupt: &'a AtomicBool,
    /// The alternate folders checked (canonical): each once.
    seen: Vec<PathBuf>,
    /// The distinct lines of the chain met so far.
    lines: usize,
}

// (A named lifetime: `check-deps`' `BackendInPublicApi` reads `'_` as a `_` import.)
impl<'a> AltWalk<'a> {
    fn visit(&mut self, objects: &Path, depth: usize) -> Result<(), GitState> {
        let file = objects.join("info").join("alternates");
        if (self.placeholder)(&file) {
            return dataless();
        }
        if !exists(&file) {
            return Ok(());
        }
        let Some(text) = small_text(&file) else {
            return redirected();
        };
        // Bounded text work first: no line is resolved before every line was counted.
        let mut raw: Vec<&str> = Vec::new();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // The C-quoted form is not followed.
            if line.starts_with('"') || depth >= ALTERNATES_DEPTH_MAX {
                return redirected();
            }
            if raw.contains(&line) {
                continue;
            }
            self.lines += 1;
            if self.lines > ALTERNATE_LINES_MAX {
                return redirected();
            }
            raw.push(line);
        }
        for line in raw {
            interrupted(self.interrupt)?;
            let Some(alt) = resolve(objects, line) else {
                return redirected();
            };
            // ## m4b-t20-adversarial: once each, and a bounded number of them.
            if self.seen.contains(&alt) {
                continue;
            }
            if self.seen.len() >= ALTERNATES_MAX {
                return redirected();
            }
            self.seen.push(alt.clone());
            require_owned(&alt, self.owned)?;
            // p1: an alternate's objects are read like ours (same caps, per folder).
            check_objects_dataless(&alt, self.placeholder, self.interrupt)?;
            self.visit(&alt, depth + 1)?;
        }
        Ok(())
    }
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
                check_layout(&f, &owner, &|_| false).unwrap_err().into_cmp(),
                GitState::Untrusted(UntrustReason::ForeignOwner).into_cmp(),
                "{foreign:?}"
            );
        }
        assert!(check_layout(&f, &all_owned, &|_| false).is_ok());
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
        assert!(check_layout(&f, &all_owned, &|_| false).is_ok());
        let alt = top.join("alt/objects");
        let owner = |p: &Path| Ok(p != alt);
        assert_eq!(
            check_layout(&f, &owner, &|_| false).unwrap_err().into_cmp(),
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
            check_layout(&f, &owner, &|_| false),
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
            check_layout(&f, &all_owned, &|_| false).unwrap(),
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
            check_layout(&f, &all_owned, &|_| false)
                .unwrap_err()
                .into_cmp(),
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
        assert_eq!(
            check_layout(&f, &all_owned, &|_| false).unwrap().git_dir,
            modules
        );
        // A gitfile into an unrelated repository: refused.
        std::fs::create_dir_all(top.join("elsewhere/.git")).unwrap();
        std::fs::write(
            top.join("super/sub/.git"),
            format!("gitdir: {}\n", top.join("elsewhere/.git").display()),
        )
        .unwrap();
        assert_eq!(
            check_layout(&f, &all_owned, &|_| false)
                .unwrap_err()
                .into_cmp(),
            GitState::Unavailable(GitError::Redirected).into_cmp()
        );
        // Not a gitfile at all.
        std::fs::write(top.join("super/sub/.git"), "hello\n").unwrap();
        assert!(check_layout(&f, &all_owned, &|_| false).is_err());
    }

    #[test]
    fn a_symlinked_dot_git_is_refused() {
        let (_t, top) = top();
        std::fs::create_dir_all(top.join("wt")).unwrap();
        let f = found(&top.join("wt"), DotGit::Symlink);
        assert_eq!(
            check_layout(&f, &all_owned, &|_| false)
                .unwrap_err()
                .into_cmp(),
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
        assert!(check_layout(&f, &all_owned, &|_| false).is_err());
        std::fs::write(
            top.join("wt/.git/objects/info/alternates"),
            "\"quoted\\npath\"\n",
        )
        .unwrap();
        assert!(check_layout(&f, &all_owned, &|_| false).is_err());
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
    fn no_placeholder(_: &Path) -> bool {
        false
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
        assert!(check_repository_files(&layout_of(&g), &no_placeholder).is_ok());
        let elsewhere = top.join("elsewhere");
        std::fs::write(&elsewhere, "x").unwrap();
        for f in ["refs/heads/main", "packed-refs", "index"] {
            let p = g.join(f);
            let _ = std::fs::remove_file(&p);
            std::os::unix::fs::symlink(&elsewhere, &p).unwrap();
            assert_eq!(
                check_repository_files(&layout_of(&g), &no_placeholder)
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
        assert!(check_repository_files(&layout_of(&g), &no_placeholder).is_err());
        // A ref outside `refs/`.
        std::fs::write(g.join("HEAD"), "ref: ../../etc/x\n").unwrap();
        assert!(check_repository_files(&layout_of(&g), &no_placeholder).is_err());
        // A loose ref over its cap.
        std::fs::remove_file(g.join("refs/heads")).unwrap();
        std::fs::create_dir_all(g.join("refs/heads")).unwrap();
        std::fs::write(g.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(g.join("refs/heads/main"), "0".repeat(8192)).unwrap();
        assert_eq!(
            check_repository_files(&layout_of(&g), &no_placeholder)
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
