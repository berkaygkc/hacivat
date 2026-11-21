//! Library-relative paths and the name rules every open re-validates (HV-SCRIPT-03 steps 4, 8).
//!
//! ## m4b-t01a-seams
//! Task 1a: [`RelPath`], whose constructor enforces the structural invariant (relative,
//! `/`-separated, no empty, `.` or `..` component, no `\`, no NUL or other C0 control), and the
//! name policy seams [`component`] and [`join`]. A `RelPath` never reaches the file system
//! except through [`join`].
//!
//! ## m4b-t04-scanner
//! Task 4 writes the policy (ADR 0009 `m4b-t04`):
//! - [`component`] refuses a name that is empty, `.`, `..`, holds `/`, `\`, NUL or any control
//!   character (C0, DEL, C1), is longer than [`MAX_NAME_BYTES`] bytes or is not UTF-8; on
//!   Windows also the characters `<>:"|?*`, a trailing dot or space, and the device names
//!   (`CON`, `PRN`, `AUX`, `NUL`, `CONIN$`, `CONOUT$`, `COM0`–`COM9`, `LPT0`–`LPT9` and the
//!   superscript `COM¹²³`/`LPT¹²³`) with any extension. [`component_on`] applies either host's
//!   rules, so every runner tests the Windows ones.
//! - [`join`] re-validates every component (and refuses the skipped ones: a leading `.`,
//!   `node_modules`), re-checks that the root is still its canonical self (and in no
//!   environment folder), walks the folders with `symlink_metadata` refusing any link or
//!   junction on the way, and confirms with `realpath` that each name it answers is the one
//!   asked for, byte for byte. A final file link answers its target only when the target is
//!   itself a listable script inside the library (alg/28 step 5). It answers a [`Joined`]:
//!   what it resolved, which the caller re-checks for scope (see [`Joined`]).
//! - ## m4b-t04-scanner fix round 1 (I2, I3, m1, m7; rulings R-T4-1, R-T4-2): links are
//!   resolved as text inside the root ([`resolve_link`]); no system call names a path outside
//!   it. [`join_entry`] refuses an entry that changed since the scan ([`PathError::Changed`]).
//! - [`display_name`] is the only display form: NFC, then every control and invisible
//!   formatting character (the chooser's set, `Default_Ignorable_Code_Point` and the
//!   interlinear annotation controls) as `<U+XXXX>` (R-M4b-23). Identity stays the OS bytes.
//!
//! `join` answers a path, so its checks are only as fresh as the call: the opens that follow
//! (Task 5) are relative to a directory handle with `O_NOFOLLOW` (HV-SCRIPT-03 step 8).

use std::ffi::{OsStr, OsString};
use std::fmt::{self, Write as _};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use icu_normalizer::ComposingNormalizerBorrowed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::envscope::root_env_folder;
use crate::registry::LibraryRoot;
use crate::scan::{self, EntryKind, Fs, LinkState, MetaKind, RealFs, Resolved, ScriptEntry};

/// A path inside a library, relative to its root, `/`-separated. Identity is the exact text
/// (the OS bytes the scan saw, NFD on macOS included); display goes through
/// [`RelPath::display_text`]. Serialized as the plain string (`workspace.json`, Task 1b);
/// deserializing applies the same structural checks as [`RelPath::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelPath(Arc<str>);

/// Why a path or a name was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// Empty, absolute, an empty, `.` or `..` component, a `\`, or a control character.
    Malformed,
    /// One component breaks the name rules.
    Name(NameProblem),
    /// A folder on the way is a symbolic link or a junction (never followed, Q15).
    LinkOnPath,
    /// A component on the way is not a folder.
    NotAFolder,
    /// The last component is neither a file nor a folder (a FIFO, a socket, a device).
    NotAFile,
    /// The path resolves outside the library, through a link that leaves it, dangles, loops
    /// or names something the scan would not list.
    OutsideRoot,
    /// The library root is gone, or no longer resolves to its canonical path.
    RootChanged,
    /// ## m4b-t04-scanner fix round 1 (m7): the root sits inside an environment folder.
    RootInEnvFolder,
    /// ## m4b-t04-scanner fix round 1 (m1): the path resolves to another spelling of a name
    /// (an 8.3 short name on Windows, a case or normalisation variant).
    Alias,
    /// ## m4b-t04-scanner fix round 1 (I2): what is on disk is not what the scan listed (a
    /// plain file became a link, a link points elsewhere, a kind changed).
    Changed,
    /// The file system refused a lookup on the way.
    Io(io::ErrorKind),
}

/// What is wrong with one name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameProblem {
    Empty,
    Dot,
    DotDot,
    /// `/` or `\`.
    Separator,
    Nul,
    /// A C0, DEL or C1 control character other than NUL.
    Control,
    NotUtf8,
    /// Longer than [`MAX_NAME_BYTES`] bytes.
    TooLong,
    /// A Windows device name, with or without an extension.
    Reserved,
    /// Windows drops a trailing dot or space, so the name would open another file.
    TrailingDotOrSpace,
    /// One of `<>:"|?*` on Windows.
    WindowsCharacter,
    /// A name the scan skips (a leading `.`, `node_modules`): never opened through [`join`].
    Skipped,
}

/// Whose name rules apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Unix,
    Windows,
}

impl Host {
    /// The host this build runs on.
    pub const CURRENT: Host = if cfg!(windows) {
        Host::Windows
    } else {
        Host::Unix
    };
}

/// The longest file or folder name accepted, in bytes (R-M4b-14).
pub const MAX_NAME_BYTES: usize = 255;

impl RelPath {
    /// A library-relative path from `/`-separated text, checked structurally.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let well_formed = !text.is_empty()
            && text.split('/').all(|c| {
                !c.is_empty()
                    && c != "."
                    && c != ".."
                    && !c.contains('\\')
                    && !c.chars().any(char::is_control)
            });
        if well_formed {
            Ok(RelPath(text.into()))
        } else {
            Err(PathError::Malformed)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The components, root first.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The last component.
    pub fn name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// The path with one more component, `name`, which must pass [`component`].
    pub fn child(&self, name: &str) -> Result<RelPath, PathError> {
        component(OsStr::new(name))?;
        Ok(RelPath(format!("{}/{name}", self.0).into()))
    }

    /// The path as a person reads it ([`display_name`] over the whole text).
    pub fn display_text(&self) -> String {
        display_name(&self.0)
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for RelPath {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RelPath {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        RelPath::parse(&s).map_err(|_| serde::de::Error::custom("not a library-relative path"))
    }
}

/// Validates one file or folder name read from a library under this host's rules (the fuzz
/// target's entry, `fuzz/scripts/fuzz_targets/pathsafe.rs`). An accepted name is one
/// [`RelPath`] component.
pub fn component(name: &OsStr) -> Result<&str, PathError> {
    component_on(name, Host::CURRENT)
}

/// [`component`] under `host`'s rules.
pub fn component_on(name: &OsStr, host: Host) -> Result<&str, PathError> {
    let bad = |p| Err(PathError::Name(p));
    let Some(s) = name.to_str() else {
        return bad(NameProblem::NotUtf8);
    };
    if s.is_empty() {
        return bad(NameProblem::Empty);
    }
    if s == "." {
        return bad(NameProblem::Dot);
    }
    if s == ".." {
        return bad(NameProblem::DotDot);
    }
    if s.contains(['/', '\\']) {
        return bad(NameProblem::Separator);
    }
    if s.contains('\0') {
        return bad(NameProblem::Nul);
    }
    if s.chars().any(char::is_control) {
        return bad(NameProblem::Control);
    }
    if s.len() > MAX_NAME_BYTES {
        return bad(NameProblem::TooLong);
    }
    if host == Host::Windows {
        if s.contains(['<', '>', ':', '"', '|', '?', '*']) {
            return bad(NameProblem::WindowsCharacter);
        }
        if s.ends_with(['.', ' ']) {
            return bad(NameProblem::TrailingDotOrSpace);
        }
        if is_windows_device(s) {
            return bad(NameProblem::Reserved);
        }
    }
    Ok(s)
}

/// Whether Windows opens a device for `name`: the part before the first `.`, trailing spaces
/// dropped, is a device name (ASCII case-insensitive; the superscript digits exactly).
fn is_windows_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let fixed = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if fixed.iter().any(|d| stem.eq_ignore_ascii_case(d)) {
        return true;
    }
    let numbered = |prefix: &str| {
        stem.get(..3)
            .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
            && stem.get(3..).is_some_and(|n| {
                let mut cs = n.chars();
                matches!(
                    (cs.next(), cs.next()),
                    (Some('0'..='9' | '\u{B9}' | '\u{B2}' | '\u{B3}'), None)
                )
            })
    };
    numbered("COM") || numbered("LPT")
}

/// Whether the scan skips `name` (and [`join`] refuses it): a leading `.` (`.git`, `.hacivat`,
/// editor temps such as `sed -i`'s `.!<pid>!a.sql`, alg/28 C11) or `node_modules`, ASCII
/// case-insensitive (R-M4b-13).
pub fn is_skipped(name: &str) -> bool {
    name.starts_with('.') || name.eq_ignore_ascii_case("node_modules")
}

/// A name the scan would list or descend into: it passes [`component`] and is not
/// [skipped](is_skipped).
fn listable(name: &str) -> Result<(), PathError> {
    component(OsStr::new(name))?;
    if is_skipped(name) {
        return Err(PathError::Name(NameProblem::Skipped));
    }
    Ok(())
}

/// What [`join`] resolved (fix round 1, I2: `join` answers what it resolved, not only a path).
///
/// **Caller contract.** Before opening, the caller (the read and save of Task 5, the tab of
/// Task 14, "Go to script" of Task 15) re-applies the environment scope to what was resolved:
/// [`envscope::visible_at(rel, kind, joined.target.as_ref(), env)`](crate::envscope::visible_at).
/// A path that was a plain file when it was scanned may be a link now, and a link may point
/// elsewhere now; the scope of the file actually opened is the one that counts. With a scanned
/// entry in hand, use [`join_entry`], which also refuses such a change outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    /// What to open: the path itself, or for a file link its canonical target.
    pub path: PathBuf,
    /// For a file link, the target inside the library (it passed every scan rule).
    pub target: Option<RelPath>,
    pub kind: JoinedKind,
}

/// What [`join`] found at the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinedKind {
    /// Nothing yet: a new script or folder may be created there.
    Absent,
    File,
    Folder,
}

/// The absolute path of `rel` under `root`, after re-validating every component and the
/// canonical root (HV-SCRIPT-03 step 8), through the real file system. A missing last
/// component is answered (a new script or folder); a missing folder on the way is
/// `Io(NotFound)`, never created. See [`Joined`] for the caller's scope re-check.
pub fn join(root: &LibraryRoot, rel: &RelPath) -> Result<Joined, PathError> {
    join_with(&RealFs, root, rel)
}

/// [`join`] through `fs` (tests inject a file system that refuses every path outside the
/// root).
pub fn join_with(fs: &dyn Fs, root: &LibraryRoot, rel: &RelPath) -> Result<Joined, PathError> {
    let parts: Vec<&str> = rel.components().collect();
    for part in &parts {
        listable(part)?;
    }
    check_root(fs, root)?;
    let Some((last, folders)) = parts.split_last() else {
        return Err(PathError::Malformed);
    };
    let io = |e: io::Error| PathError::Io(e.kind());
    let mut path = root.canonical.clone();
    for folder in folders {
        path.push(folder);
        let meta = fs.symlink_metadata(&path).map_err(io)?;
        match meta.kind {
            MetaKind::Symlink => return Err(PathError::LinkOnPath),
            MetaKind::Dir => {}
            MetaKind::File | MetaKind::Other => return Err(PathError::NotAFolder),
        }
    }
    // Identity (fix round 1, m1): the folders on the way are the ones asked for, byte for
    // byte, not an 8.3 short-name alias (`GIT~1` for `.git`) or a case variant.
    if !folders.is_empty() {
        let real = fs.canonicalize(&path).map_err(io)?;
        let asked = RelPath::parse(&folders.join("/")).map_err(|_| PathError::Malformed)?;
        same_name(&root.canonical, &real, &asked)?;
    }
    path.push(last);
    let meta = match fs.symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Joined {
                path,
                target: None,
                kind: JoinedKind::Absent,
            });
        }
        Err(e) => return Err(io(e)),
    };
    let kind = match meta.kind {
        MetaKind::Symlink => return link_target(fs, root, &path),
        MetaKind::File => JoinedKind::File,
        MetaKind::Dir => JoinedKind::Folder,
        MetaKind::Other => return Err(PathError::NotAFile),
    };
    let real = fs.canonicalize(&path).map_err(io)?;
    same_name(&root.canonical, &real, rel)?;
    Ok(Joined {
        path,
        target: None,
        kind,
    })
}

/// `real` (canonical) is `asked` under `root`, byte for byte: `Alias` for another spelling
/// inside the root (an 8.3 short name, a case variant, a skipped folder reached through one),
/// `OutsideRoot` for anything else.
fn same_name(root: &Path, real: &Path, asked: &RelPath) -> Result<(), PathError> {
    match inside_rel(root, real) {
        Some(got) if &got == asked => Ok(()),
        Some(_) => Err(PathError::Alias),
        None if real.starts_with(root) && real != root => Err(PathError::Alias),
        None => Err(PathError::OutsideRoot),
    }
}

/// [`join`] for an entry the scan listed: refuses with [`PathError::Changed`] when what is on
/// disk now is not what was scanned (a plain file that became a link, a link that points
/// elsewhere, a link that became a plain file, a file that became a folder), and refuses a
/// disabled link (`OutsideLink`, `Loop`) outright (fix round 1, I2).
pub fn join_entry(root: &LibraryRoot, entry: &ScriptEntry) -> Result<Joined, PathError> {
    join_entry_with(&RealFs, root, entry)
}

/// [`join_entry`] through `fs`.
pub fn join_entry_with(
    fs: &dyn Fs,
    root: &LibraryRoot,
    entry: &ScriptEntry,
) -> Result<Joined, PathError> {
    match entry.link {
        LinkState::OutsideLink | LinkState::Loop => return Err(PathError::OutsideRoot),
        LinkState::Plain | LinkState::InsideLink => {}
    }
    let joined = join_with(fs, root, &entry.rel)?;
    let want = match entry.kind {
        EntryKind::Folder => JoinedKind::Folder,
        EntryKind::Sql | EntryKind::Markdown => JoinedKind::File,
    };
    if joined.kind == JoinedKind::Absent {
        return Err(PathError::Io(io::ErrorKind::NotFound));
    }
    if joined.kind != want || joined.target != entry.target {
        return Err(PathError::Changed);
    }
    Ok(joined)
}

/// The root still resolves, as chosen and as stored, to its canonical path, and sits in no
/// environment folder.
fn check_root(fs: &dyn Fs, root: &LibraryRoot) -> Result<(), PathError> {
    for p in [&root.canonical, &root.path] {
        match fs.canonicalize(p) {
            Ok(real) if real == root.canonical => {}
            _ => return Err(PathError::RootChanged),
        }
    }
    if root_env_folder(&root.canonical).is_some() {
        return Err(PathError::RootInEnvFolder);
    }
    Ok(())
}

/// What the file link at `link` resolves to, when it is a regular script file the scan would
/// list inside the root; `OutsideRoot` otherwise (out, dangling, a loop, a folder, a skipped
/// place such as `.git/`, another kind of file).
fn link_target(fs: &dyn Fs, root: &LibraryRoot, link: &Path) -> Result<Joined, PathError> {
    match resolve_link(fs, &root.canonical, link) {
        Resolved::Target { path, rel, meta }
            if meta.kind == MetaKind::File && scan::script_kind(rel.name()).is_some() =>
        {
            Ok(Joined {
                path,
                target: Some(rel),
                kind: JoinedKind::File,
            })
        }
        _ => Err(PathError::OutsideRoot),
    }
}

/// The most links one resolution follows (the kernel's own limit).
const MAX_HOPS: usize = 40;

/// One step left to walk in [`resolve_link`].
enum Step {
    Parent,
    Name(OsString),
}

/// Resolves the link at `link` (inside the canonical `root`, under real folders) as **text
/// only, inside the root** (fix round 1, I3; ruling R-T4-1, which replaces "resolved with
/// `realpath`"):
/// 1. read the link text (`read_link`, an `lstat`-class call on the link itself);
/// 2. a relative text is walked from the link's folder; an absolute one must start with the
///    canonical root, component for component, or it is `Outside`, so UNC, device and `\\?\`
///    targets, another drive and any other root are refused on Windows without being touched;
/// 3. walk the steps one by one: `..` pops (above the root: `Outside`), a name is `lstat`ed,
///    always inside the root; a link met on the way is read and its text spliced in (at most
///    [`MAX_HOPS`] links, then `Loop`);
/// 4. the final path, which has no link left on it, is canonicalised (so `realpath` stays
///    inside) and must be a name the scan would list ([`inside_rel`]); that also catches an
///    8.3 alias or a case-folded spelling of a skipped folder.
///
/// No system call ever names a path outside the root.
pub fn resolve_link(fs: &dyn Fs, root: &Path, link: &Path) -> Resolved {
    let Some(parent) = link.parent() else {
        return Resolved::Outside;
    };
    let mut done = parent.to_path_buf();
    let mut todo: Vec<Step> = Vec::new(); // a stack: the next step is at the end
    let mut hops = 1;
    let mut text = match fs.read_link(link) {
        Ok(text) => text,
        Err(e) => return Resolved::Error(e.kind()),
    };
    'splice: loop {
        if !splice(root, &text, &mut done, &mut todo) {
            return Resolved::Outside;
        }
        while let Some(step) = todo.pop() {
            match step {
                Step::Parent => {
                    if done.as_path() == root || !done.pop() {
                        return Resolved::Outside;
                    }
                }
                Step::Name(name) => {
                    done.push(&name);
                    let meta = match fs.symlink_metadata(&done) {
                        Ok(meta) => meta,
                        Err(e) if e.kind() == io::ErrorKind::NotFound => {
                            return Resolved::Dangling;
                        }
                        Err(e) => return Resolved::Error(e.kind()),
                    };
                    match meta.kind {
                        MetaKind::Symlink => {
                            hops += 1;
                            if hops > MAX_HOPS {
                                return Resolved::Loop;
                            }
                            text = match fs.read_link(&done) {
                                Ok(text) => text,
                                Err(e) => return Resolved::Error(e.kind()),
                            };
                            done.pop();
                            continue 'splice;
                        }
                        MetaKind::Dir => {}
                        // A file with steps after it (ENOTDIR).
                        MetaKind::File | MetaKind::Other if !todo.is_empty() => {
                            return Resolved::Dangling;
                        }
                        MetaKind::File | MetaKind::Other => {}
                    }
                }
            }
        }
        break;
    }
    if done.as_path() == root || !done.starts_with(root) {
        return Resolved::Outside;
    }
    let meta = match fs.symlink_metadata(&done) {
        Ok(meta) => meta,
        Err(e) => return Resolved::Error(e.kind()),
    };
    let Ok(real) = fs.canonicalize(&done) else {
        return Resolved::Outside;
    };
    match inside_rel(root, &real) {
        Some(rel) => Resolved::Target {
            path: real,
            rel,
            meta,
        },
        None => Resolved::Outside,
    }
}

/// Puts the steps of the link text `text` on top of `todo`; an absolute text restarts from
/// the root. `false` when the text leaves the root (no file-system call is made).
fn splice(root: &Path, text: &Path, done: &mut PathBuf, todo: &mut Vec<Step>) -> bool {
    let absolute = text
        .components()
        .next()
        .is_some_and(|c| matches!(c, Component::Prefix(_) | Component::RootDir));
    let rest: &Path = if absolute {
        match text.strip_prefix(root) {
            Ok(rest) => {
                *done = root.to_path_buf();
                rest
            }
            Err(_) => return false,
        }
    } else {
        text
    };
    let mut steps = Vec::new();
    for c in rest.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => steps.push(Step::Parent),
            Component::Normal(name) => steps.push(Step::Name(name.to_os_string())),
            Component::Prefix(_) | Component::RootDir => return false,
        }
    }
    todo.extend(steps.into_iter().rev());
    true
}

/// `target` (canonical) as a library-relative path, when it lies strictly inside `root`
/// (canonical) and every component is one the scan would list or descend into.
pub(crate) fn inside_rel(root: &Path, target: &Path) -> Option<RelPath> {
    let rest = target.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rest.components() {
        let Component::Normal(name) = c else {
            return None;
        };
        let name = name.to_str()?;
        listable(name).ok()?;
        parts.push(name);
    }
    if parts.is_empty() {
        return None;
    }
    RelPath::parse(&parts.join("/")).ok()
}

/// `name` as a person reads it: NFC (macOS hands out NFD), then every control and invisible
/// formatting character written as `<U+XXXX>` (R-M4b-23), so a right-to-left override cannot
/// reorder a name and a zero-width character cannot make two names look the same.
pub fn display_name(name: &str) -> String {
    let nfc = ComposingNormalizerBorrowed::new_nfc().normalize(name);
    let mut out = String::with_capacity(nfc.len());
    for c in nfc.chars() {
        if c.is_control() || is_invisible_format(c) {
            let _ = write!(out, "<U+{:04X}>", u32::from(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// The characters shown as their code point: Unicode's `Default_Ignorable_Code_Point` set
/// (its reserved ranges included) and the interlinear annotation controls, the same set as
/// the statement chooser's labels (`sql-split` `is_invisible_format`), which this crate cannot
/// reach.
pub(crate) fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}
// ## m4b-t04-scanner
