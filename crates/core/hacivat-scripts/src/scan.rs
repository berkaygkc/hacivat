//! The library scan (HV-SCRIPT-03; R-M4b-12..15): `lstat` only, never a content read, never a
//! directory link followed.
//!
//! ## m4b-t01a-seams
//! Task 1a: the entry types. Task 4 writes the scan (limits, skips, names, links, dataless).
//!
//! ## m4b-t04-scanner
//! [`scan`] (ADR 0009 `m4b-t04`):
//! 1. The root must still resolve, as chosen and as stored, to its canonical path; else
//!    nothing is listed and [`ScanResult::root`] says why (`Unavailable(RootChanged)` or
//!    `Missing` for the service).
//! 2. An iterative walk (an explicit stack, never recursion) from the canonical root, through
//!    the [`Fs`] seam, which can list a folder's names, `lstat` a path and resolve a link, and
//!    has no way to read a file. Names are visited in byte order, so a stop is deterministic.
//! 3. Skipped, silently: any name starting with `.` (folders and files, alg/28 C11) and
//!    `node_modules` (ASCII case-insensitive). Refused with a diagnostic: names that fail
//!    [`pathsafe::component`](crate::pathsafe::component).
//! 4. Listed: `.sql` and `.md` files (ASCII case-insensitive extension) and the folders that
//!    hold one at any depth. FIFOs, sockets and devices are never listed (so never opened).
//! 5. Links: a link to a folder is never followed and never listed. A file link (its own name
//!    ends in `.sql`/`.md`) is resolved once: a regular script file inside the root, in no
//!    skipped folder, is `InsideLink`, with the target's metadata and the intersection of the
//!    link's and the target's environment scope (a plain `a.sql` pointing into `@staging/`
//!    shows on staging only); a loop is `Loop`; anything else (outside, dangling, a folder, a
//!    FIFO, a target under `.git/`) is `OutsideLink`, and the outside target is not even
//!    `lstat`ed. Both are listed disabled.
//! 6. Limits ([`Limits`]): 20,000 listed files, depth 32 (a listed entry has at most 32
//!    components), 200,000 visited names, 256 diagnostics (environment diagnostics first, the
//!    rest counted in [`ScanResult::suppressed`]). Hitting the first three stops the whole scan
//!    where it is and keeps what was found ([`ScanResult::stopped`]).
//! 7. Environment diagnostics come from the listed folders, one per unknown `@name` and one
//!    per folder where a chain becomes contradictory.
//!
//! The output is in tree order ([`tree::order`](crate::tree::order)).

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use hacivat_proto::EnvTag;

use crate::envscope::{self, EnvFolder, EnvScope, ScopeProblem};
use crate::pathsafe::{self, NameProblem, PathError, RelPath, display_name};
use crate::registry::LibraryRoot;
use crate::tree;

/// One listed file or folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptEntry {
    pub rel: RelPath,
    pub kind: EntryKind,
    pub scope: EnvScope,
    pub link: LinkState,
    /// A cloud placeholder: listed from metadata only, never read by a scan (R-M4b-15).
    pub dataless: bool,
    pub len: u64,
    pub mtime: SystemTime,
}

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntryKind {
    Folder,
    Sql,
    Markdown,
}

/// How an entry relates to symbolic links (Q15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkState {
    Plain,
    /// A file link that resolves inside the canonical root: listed, opens the target.
    InsideLink,
    /// A file link that resolves outside (or dangles): listed disabled, never opened.
    OutsideLink,
    /// A link loop: listed disabled.
    Loop,
}
// ## m4b-t01a-seams

// ## m4b-t04-scanner

/// What `lstat` says a path is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKind {
    File,
    Dir,
    /// A symbolic link or (Windows) a junction or other name-surrogate reparse point.
    Symlink,
    /// A FIFO, socket or device.
    Other,
}

/// The metadata a scan uses, from `lstat` (never `stat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub kind: MetaKind,
    pub len: u64,
    pub mtime: SystemTime,
    /// [`dataless::is_dataless`](crate::dataless::is_dataless) of the same metadata.
    pub dataless: bool,
}

/// What a link resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// The canonical path of the final target.
    Target(PathBuf),
    Dangling,
    Loop,
    Error(io::ErrorKind),
}

/// The file-system calls a scan may make. There is no call that reads a file: the scan lists
/// names and metadata only (R-M4b-15; a library on iCloud Drive or OneDrive is not
/// downloaded). Tests inject an in-memory one.
pub trait Fs {
    /// The names in `dir` (no `.` or `..`), in any order.
    fn read_dir(&self, dir: &Path) -> io::Result<Vec<OsString>>;
    /// `lstat`: a link is described, not followed.
    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta>;
    /// `realpath`.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// `realpath` of a link, with loops and dangling links told apart.
    fn resolve(&self, link: &Path) -> Resolved;
}

/// The real file system (`std::fs`).
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl Fs for RealFs {
    fn read_dir(&self, dir: &Path) -> io::Result<Vec<OsString>> {
        std::fs::read_dir(dir)?
            .map(|e| e.map(|e| e.file_name()))
            .collect()
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<Meta> {
        let m = std::fs::symlink_metadata(path)?;
        let ft = m.file_type();
        let kind = if ft.is_symlink() {
            MetaKind::Symlink
        } else if ft.is_dir() {
            MetaKind::Dir
        } else if ft.is_file() {
            MetaKind::File
        } else {
            MetaKind::Other
        };
        Ok(Meta {
            kind,
            len: m.len(),
            mtime: m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            dataless: crate::dataless::is_dataless(&m),
        })
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn resolve(&self, link: &Path) -> Resolved {
        match std::fs::canonicalize(link) {
            Ok(target) => Resolved::Target(target),
            Err(e) if is_loop(&e) => Resolved::Loop,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Resolved::Dangling,
            Err(e) => Resolved::Error(e.kind()),
        }
    }
}

/// `ELOOP` on Unix; `ERROR_CANT_RESOLVE_FILENAME` (1921) on Windows. (`ErrorKind::
/// FilesystemLoop` is not stable.)
fn is_loop(e: &io::Error) -> bool {
    #[cfg(unix)]
    let code = libc::ELOOP;
    #[cfg(windows)]
    let code = 1921;
    #[cfg(not(any(unix, windows)))]
    let code = i32::MIN;
    e.raw_os_error() == Some(code)
}

/// A scan's bounds (R-M4b-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Listed files (folders do not count).
    pub max_files: usize,
    /// Components of a listed entry.
    pub max_depth: usize,
    /// Names looked at, listed or not (a tree of a million `.txt` files stops too).
    pub max_visited: usize,
    /// Diagnostics kept; the rest are counted.
    pub max_diags: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_files: 20_000,
            max_depth: 32,
            max_visited: 200_000,
            max_diags: 256,
        }
    }
}

/// Which limit stopped a scan (the value is the limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitHit {
    Files(usize),
    Depth(usize),
    Visited(usize),
}

/// Why nothing was listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootProblem {
    /// The folder is gone or unmounted.
    Missing,
    /// It no longer resolves to the canonical path it was attached with (it became a link,
    /// or a link on its path was repointed).
    Changed,
    Unreadable(io::ErrorKind),
}

/// A scan diagnostic. Names are in their display form ([`display_name`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diag {
    /// A name that breaks the rules, in the folder `parent` (`None`: the root); not listed.
    BadName {
        parent: Option<RelPath>,
        name: String,
        problem: NameProblem,
    },
    /// "`@prod` is not an environment tag — files under it are hidden (did you mean
    /// `@production`?)" (R-M4b-6). One per distinct name.
    UnknownEnvFolder {
        name: String,
        suggestion: Option<EnvTag>,
    },
    /// `folder` is an environment folder inside another one that allows no tag in common.
    EnvContradiction { folder: RelPath },
    /// A folder (or an entry) the file system would not show; skipped.
    Unreadable {
        folder: RelPath,
        kind: io::ErrorKind,
    },
}

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    /// Tree order.
    pub entries: Vec<ScriptEntry>,
    pub diags: Vec<Diag>,
    /// Diagnostics dropped past [`Limits::max_diags`].
    pub suppressed: usize,
    pub stopped: Option<LimitHit>,
    /// Set when the root could not be scanned at all (then `entries` is empty).
    pub root: Option<RootProblem>,
}

/// The entry kind of a file named `name`: `.sql` or `.md` (ASCII case-insensitive), with a
/// non-empty stem.
pub fn script_kind(name: &str) -> Option<EntryKind> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        None
    } else if ext.eq_ignore_ascii_case("sql") {
        Some(EntryKind::Sql)
    } else if ext.eq_ignore_ascii_case("md") {
        Some(EntryKind::Markdown)
    } else {
        None
    }
}

/// Lists the library at `root` (see the module docs).
pub fn scan(root: &LibraryRoot, limits: &Limits, fs: &dyn Fs) -> ScanResult {
    if let Err(problem) = check_root(root, fs) {
        return ScanResult {
            root: Some(problem),
            ..ScanResult::default()
        };
    }
    let mut walk = Walk {
        root,
        limits,
        fs,
        files: Vec::new(),
        folders: Vec::new(),
        diags: Vec::new(),
        visited: 0,
        stopped: None,
        root_problem: None,
    };
    walk.run();
    if let Some(problem) = walk.root_problem {
        return ScanResult {
            root: Some(problem),
            ..ScanResult::default()
        };
    }

    // Folders that hold a listed file at any depth.
    let mut holding: HashSet<&str> = HashSet::new();
    for f in &walk.files {
        let s = f.rel.as_str();
        holding.extend(s.match_indices('/').map(|(i, _)| &s[..i]));
    }
    let mut entries: Vec<ScriptEntry> = walk
        .folders
        .iter()
        .filter(|d| holding.contains(d.rel.as_str()))
        .cloned()
        .collect();
    entries.append(&mut walk.files);
    tree::sort(&mut entries);

    let mut diags = env_diags(&entries);
    diags.append(&mut walk.diags);
    let suppressed = diags.len().saturating_sub(limits.max_diags);
    diags.truncate(limits.max_diags);
    ScanResult {
        entries,
        diags,
        suppressed,
        stopped: walk.stopped,
        root: None,
    }
}

/// The root resolves, as stored and as chosen, to its canonical path.
fn check_root(root: &LibraryRoot, fs: &dyn Fs) -> Result<(), RootProblem> {
    for p in [&root.canonical, &root.path] {
        match fs.canonicalize(p) {
            Ok(real) if real == root.canonical => {}
            Ok(_) => return Err(RootProblem::Changed),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(RootProblem::Missing),
            Err(e) => return Err(RootProblem::Unreadable(e.kind())),
        }
    }
    Ok(())
}

struct Frame {
    dir: PathBuf,
    rel: Option<RelPath>,
    depth: usize,
}

struct Walk<'a> {
    root: &'a LibraryRoot,
    limits: &'a Limits,
    fs: &'a dyn Fs,
    files: Vec<ScriptEntry>,
    folders: Vec<ScriptEntry>,
    diags: Vec<Diag>,
    visited: usize,
    stopped: Option<LimitHit>,
    root_problem: Option<RootProblem>,
}

/// What to do after one name.
enum Step {
    Go,
    Stop(LimitHit),
}

impl Walk<'_> {
    fn run(&mut self) {
        let mut stack = vec![Frame {
            dir: self.root.canonical.clone(),
            rel: None,
            depth: 0,
        }];
        while let Some(frame) = stack.pop() {
            let mut names = match self.fs.read_dir(&frame.dir) {
                Ok(names) => names,
                Err(e) => {
                    match &frame.rel {
                        None => self.root_problem = Some(RootProblem::Unreadable(e.kind())),
                        Some(rel) => self.diags.push(Diag::Unreadable {
                            folder: rel.clone(),
                            kind: e.kind(),
                        }),
                    }
                    continue;
                }
            };
            names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
            let mut subfolders = Vec::new();
            for name in &names {
                if let Step::Stop(hit) = self.visit(&frame, name, &mut subfolders) {
                    self.stopped = Some(hit);
                    return;
                }
            }
            stack.extend(subfolders.into_iter().rev());
        }
    }

    fn visit(&mut self, frame: &Frame, name: &OsStr, subfolders: &mut Vec<Frame>) -> Step {
        self.visited += 1;
        if self.visited > self.limits.max_visited {
            return Step::Stop(LimitHit::Visited(self.limits.max_visited));
        }
        if name.as_encoded_bytes().first() == Some(&b'.') {
            return Step::Go;
        }
        let text = match pathsafe::component(name) {
            Ok(text) => text,
            Err(PathError::Name(problem)) => {
                self.diags.push(Diag::BadName {
                    parent: frame.rel.clone(),
                    name: display_name(&name.to_string_lossy()),
                    problem,
                });
                return Step::Go;
            }
            Err(_) => return Step::Go,
        };
        if pathsafe::is_skipped(text) {
            return Step::Go;
        }
        let rel = match &frame.rel {
            None => RelPath::parse(text),
            Some(parent) => parent.child(text),
        };
        let Ok(rel) = rel else {
            return Step::Go;
        };
        let depth = frame.depth + 1;
        let path = frame.dir.join(name);
        let meta = match self.fs.symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => {
                self.diags.push(Diag::Unreadable {
                    folder: rel,
                    kind: e.kind(),
                });
                return Step::Go;
            }
        };
        let kind = match meta.kind {
            MetaKind::Dir => None,
            MetaKind::File | MetaKind::Symlink => match script_kind(text) {
                Some(kind) => Some(kind),
                None => return Step::Go,
            },
            MetaKind::Other => return Step::Go,
        };
        if depth > self.limits.max_depth {
            return Step::Stop(LimitHit::Depth(self.limits.max_depth));
        }
        let Some(kind) = kind else {
            self.folders.push(ScriptEntry {
                scope: envscope::folder_scope(&rel),
                rel: rel.clone(),
                kind: EntryKind::Folder,
                link: LinkState::Plain,
                dataless: false,
                len: meta.len,
                mtime: meta.mtime,
            });
            subfolders.push(Frame {
                dir: path,
                rel: Some(rel),
                depth,
            });
            return Step::Go;
        };
        if self.files.len() >= self.limits.max_files {
            return Step::Stop(LimitHit::Files(self.limits.max_files));
        }
        let entry = if meta.kind == MetaKind::Symlink {
            self.link_entry(&path, rel, kind, meta)
        } else {
            ScriptEntry {
                scope: envscope::scope_of(&rel),
                rel,
                kind,
                link: LinkState::Plain,
                dataless: meta.dataless,
                len: meta.len,
                mtime: meta.mtime,
            }
        };
        self.files.push(entry);
        Step::Go
    }

    /// The entry for the file link at `path` (resolved once; see the module docs, step 5).
    fn link_entry(&self, path: &Path, rel: RelPath, kind: EntryKind, own: Meta) -> ScriptEntry {
        let here = envscope::scope_of(&rel);
        let disabled = |link| ScriptEntry {
            rel: rel.clone(),
            kind,
            scope: here.clone(),
            link,
            dataless: false,
            len: own.len,
            mtime: own.mtime,
        };
        let target = match self.fs.resolve(path) {
            Resolved::Target(target) => target,
            Resolved::Loop => return disabled(LinkState::Loop),
            Resolved::Dangling | Resolved::Error(_) => return disabled(LinkState::OutsideLink),
        };
        let Some(target_rel) = pathsafe::inside_rel(&self.root.canonical, &target) else {
            return disabled(LinkState::OutsideLink);
        };
        if script_kind(target_rel.name()).is_none() {
            return disabled(LinkState::OutsideLink);
        }
        match self.fs.symlink_metadata(&target) {
            Ok(meta) if meta.kind == MetaKind::File => ScriptEntry {
                scope: envscope::combine(&here, &envscope::scope_of(&target_rel)),
                rel,
                kind,
                link: LinkState::InsideLink,
                dataless: meta.dataless,
                len: meta.len,
                mtime: meta.mtime,
            },
            _ => disabled(LinkState::OutsideLink),
        }
    }
}

/// One diagnostic per distinct unknown environment folder name, and one per listed folder
/// where the chain of environment folders first becomes contradictory (tree order).
fn env_diags(entries: &[ScriptEntry]) -> Vec<Diag> {
    let mut out = Vec::new();
    let mut named: HashSet<String> = HashSet::new();
    for e in entries.iter().filter(|e| e.kind == EntryKind::Folder) {
        match envscope::classify(e.rel.name()) {
            EnvFolder::Unknown { suggestion } => {
                let name = display_name(e.rel.name());
                if named.insert(name.clone()) {
                    out.push(Diag::UnknownEnvFolder { name, suggestion });
                }
            }
            EnvFolder::Tag(_) => {
                let parent_ok = e
                    .rel
                    .as_str()
                    .rsplit_once('/')
                    .and_then(|(p, _)| RelPath::parse(p).ok())
                    .is_none_or(|p| !matches!(envscope::folder_scope(&p), EnvScope::Hidden(_)));
                if parent_ok && e.scope == EnvScope::Hidden(ScopeProblem::Contradiction) {
                    out.push(Diag::EnvContradiction {
                        folder: e.rel.clone(),
                    });
                }
            }
            EnvFolder::NotEnv => {}
        }
    }
    out
}
// ## m4b-t04-scanner
