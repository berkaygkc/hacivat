// ## m4b-t13-explorer
//! The tree's file operations (R-M4b-18, Q11; NOTES M4b surface 2): New script, New folder,
//! Rename and Move to Trash, inside one library, for a connection whose environment is `env`.
//!
//! - **Inside the library only.** Every path is a [`RelPath`] whose components pass the scan's
//!   own rules (`pathsafe::component`, no skipped name), and every operation runs under a
//!   [`Dir`] chain opened from the stored canonical root without following a link
//!   (`O_NOFOLLOW` / `O_NOFOLLOW_ANY`), re-checked (device and inode) before and after.
//! - **Never replace.** A new script is `O_CREAT | O_EXCL`; a new folder is `mkdir`; a renamed
//!   file is hard-linked to its new name (which fails when that name exists) and then unlinked.
//!   A folder, a file link, or a volume without hard links (FAT, exFAT, some SMB shares) is
//!   renamed after an `lstat` of the new name; the residual (a file created at that name in
//!   between is replaced) is recorded in ADR 0009 `m4b-t13`.
//! - **Scope, fail closed (R-T5-6, R-M4b-6).** The source must be shown under `env` and the
//!   result must be too: a staging connection cannot create, rename or trash into or out of an
//!   `@production` folder. A folder that holds a listed file the connection does not show is
//!   neither renamed nor trashed ([`FileOpError::HoldsHidden`]): that would move or delete a
//!   file the user cannot see.
//! - **Move to Trash** goes through [`Trasher`]; the product's is [`OsTrash`], the `trash`
//!   crate's `TrashContext` with `DeleteMethod::NsFileManager` on macOS (the default would run
//!   `osascript` and ask Finder; T1a review m3). `trash::delete`/`delete_all` are banned in
//!   `clippy.toml`, so the context is the only route.

use std::io;
use std::path::{Path, PathBuf};

use hacivat_proto::EnvTag;

use crate::envscope::{admits, entry_scope, visible};
use crate::fileio::dir::{Dir, FsError, identity, is_link};
use crate::pathsafe::{self, PathError, RelPath};
use crate::registry::LibraryRoot;
use crate::scan::{self, EntryKind, ScriptEntry};
use crate::service::FileOpError;

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
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
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

/// Renames `from` to `to` in the same folder.
pub(crate) fn rename(
    root: &LibraryRoot,
    entries: &[ScriptEntry],
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
    if entry.kind == EntryKind::Folder {
        holds_only_shown(entries, from, env)?;
    }
    if from == to {
        return Ok(());
    }
    let (dir, old) = parent_of(root, from)?;
    let new = to.name();
    let here = dir.lstat(old).map_err(fs_error)?;
    match dir.lstat(new) {
        // The same file under another spelling (a case-only rename on a case-insensitive
        // volume): renaming it onto itself replaces nothing.
        Ok(there) if identity(&there) == identity(&here) => {
            return dir.rename(old, new).map_err(fs_error);
        }
        Ok(_) => return Err(FileOpError::Exists),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(fs_error(e)),
    }
    if here.is_file() && !is_link(&here) {
        match dir.hard_link(old, new) {
            Ok(()) => return dir.remove_file(old).map_err(fs_error),
            Err(FsError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(FileOpError::Exists);
            }
            // No hard links on this volume: the checked rename below.
            Err(FsError::Io(_)) => {}
            Err(e) => return Err(fs_error(e)),
        }
    }
    dir.rename(old, new).map_err(fs_error)
}

/// Moves `rel` to the trash through `trasher`.
pub(crate) fn trash(
    root: &LibraryRoot,
    entries: &[ScriptEntry],
    rel: &RelPath,
    env: Option<EnvTag>,
    trasher: &dyn Trasher,
) -> Result<(), FileOpError> {
    let entry = listed(entries, rel, env)?;
    if entry.kind == EntryKind::Folder {
        holds_only_shown(entries, rel, env)?;
    }
    let (dir, name) = parent_of(root, rel)?;
    let target: PathBuf = dir.child(name);
    dir.lstat(name).map_err(fs_error)?;
    dir.write_checked(|| trasher.trash(&target))
        .map_err(|e| match e {
            FsError::Io(e) => FileOpError::Trash(e.kind()),
            other => fs_error(other),
        })
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
