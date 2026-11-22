// ## m4b-t10-service (fix round 1)
//! The library service's I/O, run only on its bounded workers (I6, ruling R-T10-4): root
//! validation (I3), scans and probes, settle looks, stamps of files open in a tab (I4), rename
//! pairing, and the attach and alias writes. Every function here takes what it needs by value
//! and returns a plain result; the actor applies it. Nothing here touches the actor's state, so
//! a worker stuck on a hung mount stalls only its own library.
//!
//! File system access goes through the injected [`Fs`] (listing, `lstat`, `realpath`,
//! `readlink`), `pathsafe::join_with`, `scan::scan`, `manifest` and `fileio`; script content is
//! read only by `fileio::stamp_of` (T5 mg2).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use hacivat_proto::EnvTag;

use super::{LibraryDiag, RootRefusal, UnavailableReason};
use crate::envscope;
use crate::fileio::{self, DiskStamp};
use crate::manifest::{self, ManifestV1};
use crate::names::LogicalDb;
use crate::pathsafe::{self, JoinedKind, PathError, RelPath};
use crate::registry::{LibraryId, LibraryRoot, RegisteredLibrary};
use crate::scan::{self, EntryKind, Fs, Limits, LinkState, Meta, MetaKind, ScriptEntry};
use crate::tree;

/// The file system the workers use (tests inject one that stalls or counts).
pub(crate) type SharedFs = Arc<dyn Fs + Send + Sync>;

/// The folders no library may be (the attach checks, HV-SCRIPT-01 step 2), as configured. Fix
/// round 2 (F1): they are resolved by the job that uses them, never on the actor, since a home
/// folder can itself sit on a network mount.
#[derive(Debug, Clone, Default)]
pub(super) struct Rules {
    pub home: Option<PathBuf>,
    pub app: Vec<PathBuf>,
}

/// [`Rules`] resolved by a worker.
struct Resolved {
    home: Option<PathBuf>,
    app: Vec<PathBuf>,
}

impl Rules {
    fn resolve(&self, fs: &dyn Fs) -> Resolved {
        Resolved {
            home: self.home.as_deref().map(|h| canonical_lenient(fs, h)),
            app: self.app.iter().map(|d| canonical_lenient(fs, d)).collect(),
        }
    }

    /// The attach checks that need no registry: a filesystem root, the home folder, an app
    /// folder (inside or around), an environment folder.
    fn check(&self, fs: &dyn Fs, canonical: &Path) -> Result<(), RootRefusal> {
        let r = self.resolve(fs);
        if canonical.parent().is_none() {
            return Err(RootRefusal::FilesystemRoot);
        }
        if r.home.as_deref() == Some(canonical) {
            return Err(RootRefusal::HomeFolder);
        }
        if r.app.iter().any(|d| overlaps(canonical, d)) {
            return Err(RootRefusal::AppFolder);
        }
        Ok(())
    }
}

/// The canonical form of `path`, or of its deepest existing ancestor with the rest appended.
fn canonical_lenient(fs: &dyn Fs, path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(base) = fs.canonicalize(ancestor) {
            let rest = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
            return if rest.as_os_str().is_empty() {
                base
            } else {
                base.join(rest)
            };
        }
    }
    path.to_path_buf()
}

/// What a library's root was when it was first resolved this session.
#[derive(Debug, Clone, Default)]
pub(super) struct Session {
    pub canonical: Option<PathBuf>,
    pub identity: Option<(u64, u64)>,
}

/// A root that passed validation.
#[derive(Debug, Clone)]
pub(super) struct Valid {
    pub root: LibraryRoot,
    pub identity: Option<(u64, u64)>,
}

/// Why a root is not served.
#[derive(Debug, Clone)]
pub(super) struct Refused {
    pub reason: UnavailableReason,
    pub diags: Vec<LibraryDiag>,
}

impl Refused {
    fn changed(why: RootRefusal) -> Self {
        Refused {
            reason: UnavailableReason::RootChanged,
            diags: vec![LibraryDiag::RootRefused(why)],
        }
    }

    fn io(kind: io::ErrorKind) -> Self {
        Refused {
            reason: if kind == io::ErrorKind::NotFound {
                UnavailableReason::Missing
            } else {
                UnavailableReason::Unreadable(kind)
            },
            diags: Vec::new(),
        }
    }
}

/// Fix round 1, I3: the root is checked at every open and every rescan, as at attach plus
/// more. The registered path itself must be a real folder, never a link or junction (so a
/// folder replaced by a link between sessions is refused, not adopted). It must still resolve
/// to the canonical path and folder identity (device, inode) it had when first resolved this
/// session. It must not be a filesystem root, the home folder, an app folder, or inside or
/// around another library, and must not sit in an environment folder.
pub(super) fn validate(
    fs: &dyn Fs,
    reg: &RegisteredLibrary,
    session: &Session,
    others: &[(LibraryId, PathBuf)],
    rules: &Rules,
) -> Result<Valid, Refused> {
    let meta = fs
        .symlink_metadata(&reg.path)
        .map_err(|e| Refused::io(e.kind()))?;
    match meta.kind {
        MetaKind::Dir => {}
        MetaKind::Symlink => return Err(Refused::changed(RootRefusal::Link)),
        MetaKind::File | MetaKind::Other => return Err(Refused::changed(RootRefusal::NotAFolder)),
    }
    let canonical = fs
        .canonicalize(&reg.path)
        .map_err(|e| Refused::io(e.kind()))?;
    // This session's resolution, else (fix round 2, m-a) the one `libraries.toml` recorded.
    let expected = session.canonical.as_ref().or(reg.canonical.as_ref());
    if expected.is_some_and(|c| c != &canonical) {
        return Err(Refused::changed(RootRefusal::Moved));
    }
    rules.check(fs, &canonical).map_err(Refused::changed)?;
    if let Some((other, _)) = others.iter().find(|(_, p)| overlaps(&canonical, p)) {
        return Err(Refused::changed(RootRefusal::Overlaps(*other)));
    }
    if let Some(name) = envscope::root_env_folder(&canonical) {
        return Err(Refused {
            reason: UnavailableReason::InEnvFolder,
            diags: vec![LibraryDiag::RootInEnvFolder(name)],
        });
    }
    let identity = fileio::folder_identity(&canonical);
    if session.identity.is_some() && identity != session.identity {
        return Err(Refused::changed(RootRefusal::IdentityChanged));
    }
    // Across sessions only the inode is compared: a device number can change with a reboot.
    if let (Some(inode), Some((_, now))) = (reg.inode, identity)
        && inode != now
    {
        return Err(Refused::changed(RootRefusal::IdentityChanged));
    }
    Ok(Valid {
        root: LibraryRoot {
            id: reg.id,
            path: reg.path.clone(),
            canonical,
        },
        identity,
    })
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// What a manifest read found.
#[derive(Debug, Clone)]
pub(super) struct ManifestOut {
    pub db: LogicalDb,
    pub aliases: Vec<LogicalDb>,
    pub diags: Vec<LibraryDiag>,
}

pub(super) fn read_manifest(root: &LibraryRoot) -> ManifestOut {
    let read = manifest::read(&root.canonical);
    let folder = root.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut diags: Vec<LibraryDiag> = read
        .diags
        .iter()
        .cloned()
        .map(LibraryDiag::Manifest)
        .collect();
    let db = read.logical(folder).unwrap_or_else(|| {
        diags.push(LibraryDiag::NoLogicalName);
        LogicalDb::unbindable()
    });
    ManifestOut {
        db,
        aliases: read.aliases,
        diags,
    }
}

/// What a probe found at a hinted path (through `join_with`: no link followed).
#[derive(Debug)]
pub(super) enum Probe {
    Absent,
    /// A plain regular file, with its `lstat`.
    File(Meta),
    /// A folder, a link, or something odd: rescan the library.
    Rescan,
    /// A name the scan never lists.
    Ignore,
}

pub(super) fn probe(fs: &dyn Fs, root: &LibraryRoot, rel: &RelPath) -> Probe {
    match pathsafe::join_with(fs, root, rel) {
        Ok(j) => match j.kind {
            JoinedKind::Absent => Probe::Absent,
            JoinedKind::File if j.target.is_none() => match fs.symlink_metadata(&j.path) {
                Ok(meta) if meta.kind == MetaKind::File => Probe::File(meta),
                _ => Probe::Rescan,
            },
            JoinedKind::File | JoinedKind::Folder => Probe::Rescan,
        },
        Err(PathError::Name(_)) => Probe::Ignore,
        Err(_) => Probe::Rescan,
    }
}

/// The file changes between two listings, or from patching one (folders are not announced).
#[derive(Debug, Default)]
pub(super) struct Changes {
    /// Files gone, with their length (rename pairing).
    pub removed: Vec<(RelPath, u64)>,
    pub created: Vec<RelPath>,
    pub changed: Vec<RelPath>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.created.is_empty() && self.changed.is_empty()
    }
}

fn diff(old: &[ScriptEntry], new: &[ScriptEntry]) -> Changes {
    let file = |e: &&ScriptEntry| e.kind != EntryKind::Folder;
    let before: HashMap<&RelPath, &ScriptEntry> =
        old.iter().filter(file).map(|e| (&e.rel, e)).collect();
    let after: HashMap<&RelPath, &ScriptEntry> =
        new.iter().filter(file).map(|e| (&e.rel, e)).collect();
    let mut changes = Changes::default();
    for e in old.iter().filter(file) {
        if !after.contains_key(&e.rel) {
            changes.removed.push((e.rel.clone(), e.len));
        }
    }
    for e in new.iter().filter(file) {
        match before.get(&e.rel) {
            None => changes.created.push(e.rel.clone()),
            Some(o) if o != &e => changes.changed.push(e.rel.clone()),
            Some(_) => {}
        }
    }
    changes
}

/// An environment the entry is shown under (`None` first), so `fileio`'s scope check admits the
/// service's own stamp; `None` when it is shown nowhere.
pub(super) fn admitting(entry: &ScriptEntry) -> Option<Option<EnvTag>> {
    std::iter::once(None)
        .chain(EnvTag::ALL.into_iter().map(Some))
        .find(|env| envscope::visible(entry, *env))
}

/// The stamp of a listed file through `fileio` (never a placeholder, never a file shown
/// nowhere). Counts every content read (I4's test).
pub(super) fn stamp(
    root: &LibraryRoot,
    entry: &ScriptEntry,
    stamps: &AtomicU64,
) -> Option<DiskStamp> {
    if entry.dataless {
        return None;
    }
    let env = admitting(entry)?;
    stamps.fetch_add(1, Ordering::Relaxed);
    fileio::stamp_of(root, &entry.rel, env).ok()
}

/// One refresh of a library: probes of hinted paths, or a full validated scan.
pub(super) struct RefreshJob {
    pub reg: RegisteredLibrary,
    pub session: Session,
    pub others: Vec<(LibraryId, PathBuf)>,
    pub rules: Arc<Rules>,
    /// Validate the root only (the first step of a bring-up: the watch starts before the scan).
    pub validate_only: bool,
    /// A full scan (else `probes` are patched in, rescanning if one needs it).
    pub scan: bool,
    pub probes: Vec<RelPath>,
    pub old: Arc<[ScriptEntry]>,
    pub stopped: bool,
    pub limits: Limits,
    /// Content hashes of the files open in a tab (rename pairing).
    pub remembered: HashMap<RelPath, u128>,
    pub fs: SharedFs,
    pub stamps: Arc<AtomicU64>,
}

/// What a refresh found.
pub(super) enum RefreshOut {
    Refused(Refused),
    Validated(Valid),
    /// Nothing to change.
    Unchanged,
    Listing(Box<Listing>),
}

pub(super) struct Listing {
    pub valid: Option<Valid>,
    pub entries: Vec<ScriptEntry>,
    /// Set when a full scan ran.
    pub scanned: Option<Scanned>,
    pub changes: Changes,
    /// `(from, to, hash)`: a file open in a tab that moved (alg/28 C4).
    pub renames: Vec<(RelPath, RelPath, u128)>,
}

pub(super) struct Scanned {
    pub diags: Vec<LibraryDiag>,
    pub stopped: bool,
    pub manifest: ManifestOut,
    /// The library root holds the repository's `.git` (else a monorepo or no repository: the
    /// status gets the slow backstop refresh, m7).
    pub has_git_dir: bool,
}

impl RefreshJob {
    pub fn run(self) -> RefreshOut {
        let fs = self.fs.as_ref();
        if self.validate_only {
            return match validate(fs, &self.reg, &self.session, &self.others, &self.rules) {
                Ok(v) => RefreshOut::Validated(v),
                Err(r) => RefreshOut::Refused(r),
            };
        }
        if !self.scan
            && let Some(canonical) = &self.session.canonical
        {
            let root = LibraryRoot {
                id: self.reg.id,
                path: self.reg.path.clone(),
                canonical: canonical.clone(),
            };
            match self.patch(&root) {
                Some(Some(listing)) => return RefreshOut::Listing(Box::new(listing)),
                Some(None) => return RefreshOut::Unchanged,
                None => {}
            }
        }
        self.full_scan()
    }

    /// Patches the hinted paths into the listing; `None` when one of them needs a rescan.
    fn patch(&self, root: &LibraryRoot) -> Option<Option<Listing>> {
        let fs = self.fs.as_ref();
        let mut probes = self.probes.clone();
        probes.sort();
        probes.dedup();
        let mut entries: Vec<ScriptEntry> = self.old.to_vec();
        let mut changes = Changes::default();
        let mut files = entries
            .iter()
            .filter(|e| e.kind != EntryKind::Folder)
            .count();
        for rel in probes {
            let known = entries.iter().position(|e| e.rel == rel);
            match probe(fs, root, &rel) {
                Probe::Ignore => {}
                Probe::Rescan => return None,
                Probe::Absent => match known {
                    Some(i) if entries[i].kind == EntryKind::Folder => return None,
                    Some(i) => {
                        let e = entries.remove(i);
                        files -= 1;
                        changes.removed.push((rel, e.len));
                    }
                    None => {}
                },
                Probe::File(meta) => {
                    let Some(kind) = scan::script_kind(rel.name()) else {
                        continue;
                    };
                    match known {
                        Some(i)
                            if entries[i].link != LinkState::Plain || entries[i].kind != kind =>
                        {
                            return None;
                        }
                        Some(i) => {
                            let e = &mut entries[i];
                            if (e.len, e.mtime, e.dataless) != (meta.len, meta.mtime, meta.dataless)
                            {
                                e.len = meta.len;
                                e.mtime = meta.mtime;
                                e.dataless = meta.dataless;
                                changes.changed.push(rel);
                            }
                        }
                        None => {
                            let parent = rel.as_str().rsplit_once('/').map(|(p, _)| p);
                            let listed = parent.is_none_or(|p| {
                                entries
                                    .iter()
                                    .any(|e| e.kind == EntryKind::Folder && e.rel.as_str() == p)
                            });
                            let depth = rel.components().count();
                            if !listed
                                || depth > self.limits.max_depth
                                || self.stopped
                                || files >= self.limits.max_files
                            {
                                return None;
                            }
                            files += 1;
                            changes.created.push(rel.clone());
                            entries.push(ScriptEntry {
                                scope: envscope::entry_scope(&rel, kind),
                                rel,
                                kind,
                                link: LinkState::Plain,
                                dataless: meta.dataless,
                                len: meta.len,
                                mtime: meta.mtime,
                                target: None,
                            });
                        }
                    }
                }
            }
        }
        if changes.is_empty() {
            return Some(None);
        }
        // A folder left with no script is not listed: let the scan decide.
        let emptied = changes.removed.iter().any(|(rel, _)| {
            rel.as_str().rsplit_once('/').is_some_and(|(parent, _)| {
                let prefix = format!("{parent}/");
                !entries
                    .iter()
                    .any(|e| e.kind != EntryKind::Folder && e.rel.as_str().starts_with(&prefix))
            })
        });
        if emptied {
            return None;
        }
        tree::sort(&mut entries);
        let renames = self.pair(root, &entries, &mut changes);
        Some(Some(Listing {
            valid: None,
            entries,
            scanned: None,
            changes,
            renames,
        }))
    }

    fn full_scan(&self) -> RefreshOut {
        let fs = self.fs.as_ref();
        let valid = match validate(fs, &self.reg, &self.session, &self.others, &self.rules) {
            Ok(v) => v,
            Err(r) => return RefreshOut::Refused(r),
        };
        let result = scan::scan(&valid.root, &self.limits, fs);
        if let Some(problem) = result.root {
            let refused = match problem {
                scan::RootProblem::Missing => Refused::io(io::ErrorKind::NotFound),
                scan::RootProblem::Changed => Refused::changed(RootRefusal::Moved),
                scan::RootProblem::Unreadable(kind) => Refused::io(kind),
                scan::RootProblem::InEnvFolder(name) => Refused {
                    reason: UnavailableReason::InEnvFolder,
                    diags: vec![LibraryDiag::RootInEnvFolder(name)],
                },
            };
            return RefreshOut::Refused(refused);
        }
        let manifest = read_manifest(&valid.root);
        let mut diags: Vec<LibraryDiag> = result.diags.into_iter().map(LibraryDiag::Scan).collect();
        diags.extend(result.stopped.map(LibraryDiag::ScanStopped));
        if result.suppressed > 0 {
            diags.push(LibraryDiag::Suppressed(result.suppressed));
        }
        let has_git_dir = fs
            .symlink_metadata(&valid.root.canonical.join(".git"))
            .is_ok();
        let mut changes = diff(&self.old, &result.entries);
        let renames = self.pair(&valid.root, &result.entries, &mut changes);
        RefreshOut::Listing(Box::new(Listing {
            valid: Some(valid),
            entries: result.entries,
            scanned: Some(Scanned {
                diags,
                stopped: result.stopped.is_some(),
                manifest,
                has_git_dir,
            }),
            changes,
            renames,
        }))
    }

    /// Pairs a removed file that a tab has open with a created file of the same length and
    /// content hash (alg/28 C4). Only the candidates of such a file are read.
    fn pair(
        &self,
        root: &LibraryRoot,
        entries: &[ScriptEntry],
        changes: &mut Changes,
    ) -> Vec<(RelPath, RelPath, u128)> {
        let mut renames = Vec::new();
        let mut removed = Vec::new();
        for (from, len) in std::mem::take(&mut changes.removed) {
            let Some(&hash) = self.remembered.get(&from) else {
                removed.push((from, len));
                continue;
            };
            let found = changes.created.iter().position(|to| {
                entries
                    .iter()
                    .find(|e| &e.rel == to)
                    .filter(|e| e.len == len && !e.dataless)
                    .and_then(|e| stamp(root, e, &self.stamps))
                    .is_some_and(|s| s.hash == hash)
            });
            match found {
                Some(i) => renames.push((from, changes.created.remove(i), hash)),
                None => removed.push((from, len)),
            }
        }
        changes.removed = removed;
        renames
    }
}

/// One settle look at a changed file (HV-SCRIPT-08 step 5), and its stamp when it held still
/// and a tab has it open (I4).
pub(super) struct SettleJob {
    pub root: LibraryRoot,
    pub entry: ScriptEntry,
    pub len: u64,
    pub mtime: SystemTime,
    pub last_try: bool,
    pub stamp: bool,
    pub fs: SharedFs,
    pub stamps: Arc<AtomicU64>,
}

pub(super) struct SettleOut {
    pub meta: Option<Meta>,
    pub still: bool,
    pub stamp: Option<DiskStamp>,
}

impl SettleJob {
    pub fn run(self) -> SettleOut {
        let Probe::File(meta) = probe(self.fs.as_ref(), &self.root, &self.entry.rel) else {
            return SettleOut {
                meta: None,
                still: false,
                stamp: None,
            };
        };
        let still = (meta.len, meta.mtime) == (self.len, self.mtime);
        let mut entry = self.entry.clone();
        entry.dataless = meta.dataless;
        let stamp = (self.stamp && (still || self.last_try))
            .then(|| stamp(&self.root, &entry, &self.stamps))
            .flatten();
        SettleOut {
            meta: Some(meta),
            still,
            stamp,
        }
    }
}

/// The attach's first step, off the actor (I6): the chosen path must be a real folder, not a
/// link (I3: a link root would be refused at the next launch).
///
/// Fix round 2 (F1): it also answers the folder's canonical path and identity, and makes the
/// guard's checks, so the actor registers it with no file-system call
/// (`LibraryRegistry::add_resolved`).
pub(super) fn resolve_attach(
    fs: &dyn Fs,
    path: &Path,
    rules: &Rules,
) -> Result<Valid, super::AttachError> {
    use super::AttachError;
    use crate::registry::RegistryError;
    let io = |e: io::Error| AttachError::Registry(RegistryError::Io(e.kind()));
    if !path.is_absolute() {
        return Err(AttachError::Registry(RegistryError::NotAbsolute));
    }
    let meta = fs.symlink_metadata(path).map_err(io)?;
    match meta.kind {
        MetaKind::Dir => {}
        MetaKind::Symlink => return Err(AttachError::RootIsLink),
        MetaKind::File | MetaKind::Other => {
            return Err(AttachError::Registry(RegistryError::NotAFolder));
        }
    }
    let canonical = fs.canonicalize(path).map_err(io)?;
    rules.check(fs, &canonical).map_err(|why| {
        AttachError::Registry(match why {
            RootRefusal::FilesystemRoot => RegistryError::FilesystemRoot,
            RootRefusal::HomeFolder => RegistryError::HomeFolder,
            _ => RegistryError::AppFolder,
        })
    })?;
    if let Some(name) = envscope::root_env_folder(&canonical) {
        return Err(AttachError::RootInEnvFolder(name));
    }
    Ok(Valid {
        identity: fileio::folder_identity(&canonical),
        root: LibraryRoot {
            id: LibraryId(uuid::Uuid::nil()),
            path: path.to_path_buf(),
            canonical,
        },
    })
}

/// Writes a new manifest (only when absent, R-M4b-8).
pub(super) fn write_manifest(canonical: &Path, db: LogicalDb) -> Result<(), manifest::WriteError> {
    match manifest::write_new(canonical, &ManifestV1::new(db, Vec::new())) {
        Ok(()) | Err(manifest::WriteError::Exists) => Ok(()),
        Err(e) => Err(e),
    }
}
// ## m4b-t10-service
