// ## m4b-t13-explorer
//! The tree's file operations (R-M4b-18, Q11; NOTES M4b surface 2): New script, New folder,
//! Rename and Move to Trash, inside one library, for a connection whose environment is `env`.
//!
//! - **Inside the library only.** Every path is a [`RelPath`] whose components pass the scan's
//!   own rules (`pathsafe::component`, no skipped name), and every operation runs under a
//!   [`Dir`] chain opened from the stored canonical root without following a link
//!   (`O_NOFOLLOW` / `O_NOFOLLOW_ANY`), re-checked (device and inode) before and after.
//! - **Never replace (fix round 1, R-T13-1).** A new script is `O_CREAT | O_EXCL`; a new folder
//!   is `mkdir`. A renamed file (or file link) is hard-linked to its new name, which fails when
//!   that name exists, then unlinked; without hard links the rename is refused
//!   ([`FileOpError::NoSafeRename`]), never a replacing `rename` (Windows `rename` replaces
//!   files). A name already taken is `Exists` unless it reaches the entry itself under another
//!   spelling, proved by a known identity (Unix) or by a listing whose only case-insensitive
//!   match is the entry: that case-only change goes through a random temp name, each step after
//!   an absence check. A folder is renamed after an absence check of its new name (the residual
//!   race is in ADR 0009 `m4b-t13`).
//! - **Scope, fail closed (R-T5-6, R-M4b-6, R-T13-2).** The source must be shown under `env` and
//!   the result must be too, and a rename never changes the scope of the entry or anything
//!   inside it ([`FileOpError::ScopeChange`]). A folder is renamed or trashed only when the scan
//!   listed its whole subtree and a names-only walk of it now finds no environment folder `env`
//!   does not show ([`FileOpError::HoldsHidden`], [`FileOpError::NotFullyListed`]): that would
//!   move or delete files the user cannot see.
//! - **Move to Trash** goes through [`Trasher`]; the product's is [`OsTrash`], the `trash`
//!   crate's `TrashContext` with `DeleteMethod::NsFileManager` on macOS (the default would run
//!   `osascript` and ask Finder; T1a review m3). `trash::delete`/`delete_all` are banned in
//!   `clippy.toml`, so the context is the only route.

use std::io;
use std::path::{Path, PathBuf};

use hacivat_proto::EnvTag;

use crate::envscope::{self, admits, entry_scope, visible};
use crate::fileio::dir::{Dir, FsError, identity, is_link};
use crate::pathsafe::{self, PathError, RelPath};
use crate::registry::LibraryRoot;
use crate::scan::{self, EntryKind, ScriptEntry};
use crate::service::{FileOpError, LibraryDiag};

/// Moves a path to the operating system's trash.
pub trait Trasher: Send + Sync {
    /// Moves `path` (a file, a folder with its contents, or a link itself, never its target).
    fn trash(&self, path: &Path) -> io::Result<()>;
}

/// The operating system's trash: Finder's Trash (through `NSFileManager`), the Recycle Bin, or
/// the freedesktop trash.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsTrash;

impl Trasher for OsTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        os_trash_context()
            .delete(path)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

/// The context every trash call uses: on macOS `NSFileManager`'s `trashItemAtURL`, never the
/// Finder script (`osascript`).
fn os_trash_context() -> trash::TrashContext {
    // ## m4b-t13-explorer fix round 1 (review m1): the one blessed constructor; `clippy.toml`
    // bans `TrashContext::new`/`default` everywhere else, so the Finder default is never used.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    #[expect(
        clippy::disallowed_methods,
        reason = "the one TrashContext constructor: NsFileManager is set on macOS right below"
    )]
    let mut ctx = trash::TrashContext::new();
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx
}

/// A new name's checks: one listable component, and a script's name ends in `.sql` or `.md`.
fn check_name(rel: &RelPath, kind: EntryKind) -> Result<(), FileOpError> {
    let name = rel.name();
    pathsafe::component(name.as_ref()).map_err(path_error)?;
    if pathsafe::is_skipped(name) {
        return Err(FileOpError::BadName(pathsafe::NameProblem::Skipped));
    }
    // Every host's rules, so a library shared with Windows never gets a name it cannot open.
    pathsafe::component_on(name.as_ref(), pathsafe::Host::Windows).map_err(path_error)?;
    let script = scan::script_kind(name);
    match kind {
        EntryKind::Folder => Ok(()),
        EntryKind::Sql | EntryKind::Markdown if script.is_some() => Ok(()),
        EntryKind::Sql | EntryKind::Markdown => Err(FileOpError::NotAScript),
    }
}

fn path_error(e: PathError) -> FileOpError {
    match e {
        PathError::Name(p) => FileOpError::BadName(p),
        PathError::Io(io::ErrorKind::NotFound) => FileOpError::NotFound,
        other => FileOpError::Path(other),
    }
}

fn fs_error(e: FsError) -> FileOpError {
    match e {
        FsError::Io(e) if e.kind() == io::ErrorKind::AlreadyExists => FileOpError::Exists,
        FsError::Io(e) if e.kind() == io::ErrorKind::NotFound => FileOpError::NotFound,
        FsError::Io(e) => FileOpError::Io(e.kind()),
        FsError::Link => FileOpError::Path(PathError::LinkOnPath),
        FsError::NotDir => FileOpError::Path(PathError::NotAFolder),
        FsError::NotFile => FileOpError::Path(PathError::NotAFile),
        FsError::Changed | FsError::TooLarge => FileOpError::Path(PathError::Changed),
    }
}

/// The folder that holds `rel`, opened from the canonical root without following a link, and
/// `rel`'s last name. `pathsafe::join` re-validates the root and every component first.
fn parent_of<'a>(root: &LibraryRoot, rel: &'a RelPath) -> Result<(Dir, &'a str), FileOpError> {
    pathsafe::join(root, rel).map_err(path_error)?;
    let parts: Vec<&str> = rel.components().collect();
    let (name, folders) = parts
        .split_last()
        .ok_or(FileOpError::Path(PathError::Malformed))?;
    let mut dir = Dir::open_canonical(&root.canonical).map_err(fs_error)?;
    for folder in folders {
        dir = dir.subdir(folder).map_err(fs_error)?;
    }
    Ok((dir, name))
}

/// Whether a new `kind` at `rel` would be shown under `env`.
fn shown_at(rel: &RelPath, kind: EntryKind, env: Option<EnvTag>) -> bool {
    admits(&entry_scope(rel, kind), env)
}

/// The listed entry at `rel` when `env` shows it.
fn listed<'a>(
    entries: &'a [ScriptEntry],
    rel: &RelPath,
    env: Option<EnvTag>,
) -> Result<&'a ScriptEntry, FileOpError> {
    let entry = entries
        .iter()
        .find(|e| &e.rel == rel)
        .ok_or(FileOpError::NotFound)?;
    if !visible(entry, env) {
        return Err(FileOpError::NotListed);
    }
    Ok(entry)
}

/// A folder must not hold a listed entry `env` does not show (it would move or delete it).
fn holds_only_shown(
    entries: &[ScriptEntry],
    folder: &RelPath,
    env: Option<EnvTag>,
) -> Result<(), FileOpError> {
    let prefix = format!("{}/", folder.as_str());
    let hidden = entries
        .iter()
        .filter(|e| e.rel.as_str().starts_with(&prefix))
        .any(|e| e.kind != EntryKind::Folder && !visible(e, env));
    if hidden {
        Err(FileOpError::HoldsHidden)
    } else {
        Ok(())
    }
}

// ## m4b-t13-explorer fix round 1 (review I1, controller ruling): a folder is moved or deleted
// only when its whole subtree was listed, and no entry in it is hidden from the connection.

/// Whether `rel` is `folder` or lies under it.
fn at_or_under(rel: Option<&RelPath>, folder: &RelPath) -> bool {
    match rel {
        Some(r) => {
            r == folder
                || r.as_str()
                    .strip_prefix(folder.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        }
        // The root holds every folder.
        None => true,
    }
}

/// Whether the scan listed the whole subtree of `folder`: no limit stopped the scan (it does
/// not say where), no diagnostics were dropped, nothing at or under it was unreadable or
/// refused by name, and no dataless folder at or under it was left unentered.
fn fully_listed(entries: &[ScriptEntry], diags: &[LibraryDiag], folder: &RelPath) -> bool {
    use crate::scan::Diag;
    let partial_diag = diags.iter().any(|d| match d {
        LibraryDiag::ScanStopped(_) | LibraryDiag::Suppressed(_) => true,
        LibraryDiag::Scan(Diag::Unreadable { folder: f, .. }) => at_or_under(Some(f), folder),
        LibraryDiag::Scan(Diag::BadName { parent, .. }) => at_or_under(parent.as_ref(), folder),
        LibraryDiag::Scan(Diag::LinkScope { rel }) => at_or_under(Some(rel), folder),
        _ => false,
    });
    let dataless_folder = entries
        .iter()
        .any(|e| e.kind == EntryKind::Folder && e.dataless && at_or_under(Some(&e.rel), folder));
    !partial_diag && !dataless_folder
}

/// How many names a folder check reads before it gives up (the scan's file limit).
const WALK_MAX_NAMES: usize = 20_000;
/// How deep it goes (the scan's depth limit).
const WALK_MAX_DEPTH: usize = 32;

/// Reads the names under folder `rel` (opened as `dir`) on disk now, without following a link
/// or entering a dataless folder, and refuses when one names an environment folder `env` does
/// not show (`HoldsHidden`: it would move or delete files under it unseen), or when the walk
/// cannot see everything (`NotFullyListed`). Names and metadata only: no file is read.
fn check_subtree(dir: Dir, rel: &RelPath, env: Option<EnvTag>) -> Result<(), FileOpError> {
    let mut todo: Vec<(Dir, RelPath, usize)> = vec![(dir, rel.clone(), 0)];
    let mut seen = 0usize;
    while let Some((dir, at, depth)) = todo.pop() {
        if depth > WALK_MAX_DEPTH {
            return Err(FileOpError::NotFullyListed);
        }
        let names = dir
            .checked(|| list_names(&dir.child("")))
            .map_err(|_| FileOpError::NotFullyListed)?;
        for name in names {
            seen += 1;
            if seen > WALK_MAX_NAMES {
                return Err(FileOpError::NotFullyListed);
            }
            if pathsafe::is_skipped(&name) {
                continue;
            }
            let Ok(meta) = dir.lstat(&name) else {
                return Err(FileOpError::NotFullyListed);
            };
            if is_link(&meta) || !meta.is_dir() {
                continue;
            }
            let Ok(child) = at.child(&name) else {
                // A name the scan refuses: its content was never listed.
                return Err(FileOpError::NotFullyListed);
            };
            if !admits(&envscope::folder_scope(&child), env) {
                return Err(FileOpError::HoldsHidden);
            }
            if crate::dataless::is_dataless(&meta) {
                return Err(FileOpError::NotFullyListed);
            }
            let sub = dir.subdir(&name).map_err(|_| FileOpError::NotFullyListed)?;
            todo.push((sub, child, depth + 1));
        }
    }
    Ok(())
}

/// The names in the folder at `path` (never `.` or `..`).
fn list_names(path: &Path) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(path)? {
        // A name that is not UTF-8 is never listed by the scan either: the walk is partial.
        let name = e?
            .file_name()
            .into_string()
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        out.push(name);
    }
    Ok(out)
}

/// The checks a folder passes before it is renamed or trashed: listed in full by the scan,
/// no listed entry hidden from `env`, and no environment folder `env` does not show on disk.
fn folder_may_move(
    root: &LibraryRoot,
    entries: &[ScriptEntry],
    diags: &[LibraryDiag],
    folder: &RelPath,
    env: Option<EnvTag>,
) -> Result<(), FileOpError> {
    holds_only_shown(entries, folder, env)?;
    if !fully_listed(entries, diags, folder) {
        return Err(FileOpError::NotFullyListed);
    }
    let (parent, name) = parent_of(root, folder)?;
    let dir = parent.subdir(name).map_err(fs_error)?;
    check_subtree(dir, folder, env)
}
// ## m4b-t13-explorer

/// Creates an empty script or a folder at `rel`.
pub(crate) fn create(
    root: &LibraryRoot,
    rel: &RelPath,
    kind: EntryKind,
    env: Option<EnvTag>,
) -> Result<(), FileOpError> {
    check_name(rel, kind)?;
    if !shown_at(rel, kind, env) {
        return Err(FileOpError::NotListed);
    }
    // ## m4b-t13-explorer fix round 1 (review m2, R-T13-1): a name taken in any ASCII case is
    // `Exists` on every system.
    {
        let (dir, name) = chain_to_parent(root, rel)?;
        collides(&dir, name, None)?;
    }
    let (dir, name) = parent_of(root, rel)?;
    match kind {
        EntryKind::Folder => dir.create_dir(name).map_err(fs_error),
        EntryKind::Sql | EntryKind::Markdown => {
            let file = dir.create_new(name).map_err(fs_error)?;
            drop(file);
            dir.sync();
            Ok(())
        }
    }
}

// ## m4b-t13-explorer fix round 1 (review C1, controller ruling R-T13-1): no rename ever
// replaces an entry, on any system.

/// What `new` is in `dir`, next to `old` (which `here` describes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Nothing has that name.
    Absent,
    /// The name reaches `old` itself under another spelling (a case-only change on a
    /// case-insensitive volume).
    SameEntry,
}

/// Classifies `new` in `dir`: absent, or `old` itself (proved by a known identity, or by a
/// listing whose only case-insensitive match of `new` is `old`); anything else is `Exists`.
fn target_of(
    dir: &Dir,
    old: &str,
    new: &str,
    here: &std::fs::Metadata,
) -> Result<Target, FileOpError> {
    // Any other entry with that name in any ASCII case is a collision on every system: a
    // library is shared with case-insensitive volumes (macOS, Windows).
    collides(dir, new, Some(old))?;
    let there = match dir.lstat(new) {
        Ok(there) => there,
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => return Ok(Target::Absent),
        Err(e) => return Err(fs_error(e)),
    };
    // `new` reaches an entry under another spelling: `old` itself only when a known identity
    // says so, or when the only case-insensitive match is `old` (unknown identities off Unix).
    let same_identity = identity(&there).known() && identity(&there) == identity(here);
    if same_identity || old.eq_ignore_ascii_case(new) {
        Ok(Target::SameEntry)
    } else {
        Err(FileOpError::Exists)
    }
}

/// `Exists` when `dir` lists `name` exactly, or another entry (not `except`) equal to it in
/// ASCII case.
fn collides(dir: &Dir, name: &str, except: Option<&str>) -> Result<(), FileOpError> {
    let names = dir
        .checked(|| list_names(&dir.child("")))
        .map_err(fs_error)?;
    let taken = names
        .iter()
        .any(|n| n == name || (Some(n.as_str()) != except && n.eq_ignore_ascii_case(name)));
    if taken {
        Err(FileOpError::Exists)
    } else {
        Ok(())
    }
}

/// The folder chain to `rel`'s parent, opened without following a link (no `join`: used to
/// look for a name collision before `join` refuses another spelling as an alias).
fn chain_to_parent<'a>(
    root: &LibraryRoot,
    rel: &'a RelPath,
) -> Result<(Dir, &'a str), FileOpError> {
    let parts: Vec<&str> = rel.components().collect();
    let (name, folders) = parts
        .split_last()
        .ok_or(FileOpError::Path(PathError::Malformed))?;
    let mut dir = Dir::open_canonical(&root.canonical).map_err(fs_error)?;
    for folder in folders {
        dir = dir.subdir(folder).map_err(fs_error)?;
    }
    Ok((dir, name))
}

/// `name` is absent from `dir` right now.
fn absent(dir: &Dir, name: &str) -> Result<(), FileOpError> {
    match dir.lstat(name) {
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(FileOpError::Exists),
        Err(e) => Err(fs_error(e)),
    }
}

/// A case-only rename, in two steps through a random temp name (a leading dot: the scan skips
/// it), each step after a check that its target is absent. A failed second step moves the
/// entry back.
fn rename_case_only(dir: &Dir, old: &str, new: &str) -> Result<(), FileOpError> {
    let tmp = format!(".hv-rename-{}", uuid::Uuid::new_v4().simple());
    absent(dir, &tmp)?;
    dir.rename(old, &tmp).map_err(fs_error)?;
    let second = absent(dir, new).and_then(|()| dir.rename(&tmp, new).map_err(fs_error));
    if let Err(e) = second {
        if absent(dir, old).is_ok() {
            let _ = dir.rename(&tmp, old);
        }
        return Err(e);
    }
    Ok(())
}

/// Renames `from` to `to` in the same folder.
pub(crate) fn rename(
    root: &LibraryRoot,
    entries: &[ScriptEntry],
    diags: &[LibraryDiag],
    from: &RelPath,
    to: &RelPath,
    env: Option<EnvTag>,
) -> Result<(), FileOpError> {
    let entry = listed(entries, from, env)?;
    let parent = |r: &RelPath| {
        let s = r.as_str();
        s.rfind('/').map(|i| s[..i].to_owned())
    };
    if parent(from) != parent(to) {
        return Err(FileOpError::OtherFolder);
    }
    check_name(to, entry.kind)?;
    if !shown_at(to, entry.kind, env) {
        return Err(FileOpError::NotListed);
    }
    // R-T13-2: a rename never changes which connections see the entry or anything inside it.
    if entry_scope(from, entry.kind) != entry_scope(to, entry.kind) {
        return Err(FileOpError::ScopeChange);
    }
    if from == to {
        return Ok(());
    }
    let folder = entry.kind == EntryKind::Folder;
    if folder {
        folder_may_move(root, entries, diags, from, env)?;
    }
    let (dir, old) = parent_of(root, from)?;
    let new = to.name();
    let here = dir.lstat(old).map_err(fs_error)?;
    match target_of(&dir, old, new, &here)? {
        Target::SameEntry => rename_case_only(&dir, old, new),
        // A folder: its target is checked absent right before the rename (the race is in
        // ADR 0009 `m4b-t13`).
        Target::Absent if folder || here.is_dir() => {
            absent(&dir, new)?;
            dir.rename(old, new).map_err(fs_error)
        }
        // A file (or a file link): `link` fails when the name exists, so nothing is replaced.
        Target::Absent => match dir.hard_link(old, new) {
            Ok(()) => dir.remove_file(old).map_err(fs_error),
            Err(FsError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
                Err(FileOpError::Exists)
            }
            // No hard links on this volume (FAT, exFAT, some shares): refused, never a
            // replacing rename (R-T13-1).
            Err(FsError::Io(_)) => Err(FileOpError::NoSafeRename),
            Err(e) => Err(fs_error(e)),
        },
    }
}
// ## m4b-t13-explorer

/// Moves `rel` to the trash through `trasher`.
pub(crate) fn trash(
    root: &LibraryRoot,
    entries: &[ScriptEntry],
    diags: &[LibraryDiag],
    rel: &RelPath,
    env: Option<EnvTag>,
    trasher: &dyn Trasher,
) -> Result<(), FileOpError> {
    let entry = listed(entries, rel, env)?;
    if entry.kind == EntryKind::Folder {
        folder_may_move(root, entries, diags, rel, env)?;
    }
    let (dir, name) = parent_of(root, rel)?;
    let target: PathBuf = dir.child(name);
    dir.lstat(name).map_err(fs_error)?;
    // ## m4b-t13-explorer fix round 1 (review m13): checked before; once the trash has taken
    // it, a failed after-check is "moved, but the folder changed meanwhile", never "nothing
    // changed".
    dir.writable().map_err(fs_error)?;
    dir.verify().map_err(fs_error)?;
    trasher
        .trash(&target)
        .map_err(|e| FileOpError::Trash(e.kind()))?;
    dir.verify().map_err(|_| FileOpError::TrashedButChanged)
}

/// A trash that moves into a folder of the test's (no file reaches the user's trash).
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct FolderTrash(pub PathBuf);

#[cfg(any(test, feature = "test-support"))]
impl Trasher for FolderTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        std::fs::rename(path, self.0.join(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn the_os_trash_uses_ns_file_manager_never_the_finder_script() {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        assert!(matches!(
            os_trash_context().delete_method(),
            DeleteMethod::NsFileManager
        ));
    }

    #[test]
    fn new_names_follow_the_scan_and_every_hosts_rules() {
        let r = |s: &str| RelPath::parse(s).unwrap();
        assert_eq!(check_name(&r("a.sql"), EntryKind::Sql), Ok(()));
        assert_eq!(check_name(&r("notes.MD"), EntryKind::Markdown), Ok(()));
        assert_eq!(check_name(&r("x/rapor"), EntryKind::Folder), Ok(()));
        assert_eq!(
            check_name(&r("a.txt"), EntryKind::Sql),
            Err(FileOpError::NotAScript)
        );
        assert_eq!(
            check_name(&r(".hidden.sql"), EntryKind::Sql),
            Err(FileOpError::BadName(pathsafe::NameProblem::Skipped))
        );
        assert_eq!(
            check_name(&r("CON.sql"), EntryKind::Sql),
            Err(FileOpError::BadName(pathsafe::NameProblem::Reserved))
        );
        assert_eq!(
            check_name(&r("a.sql."), EntryKind::Sql),
            Err(FileOpError::BadName(
                pathsafe::NameProblem::TrailingDotOrSpace
            ))
        );
        assert_eq!(
            check_name(&r("a:b.sql"), EntryKind::Sql),
            Err(FileOpError::BadName(
                pathsafe::NameProblem::WindowsCharacter
            ))
        );
    }
}
// ## m4b-t13-explorer
