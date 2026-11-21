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
//!   `node_modules`), re-checks that the root is still its canonical self, walks the folders with
//!   `symlink_metadata` refusing any link or junction on the way, and confirms with `realpath`
//!   that what it answers is inside the root. A final file link answers its canonical target
//!   only when the target is itself a listable script inside the library (alg/28 step 5).
//!   Containment is a component-wise byte comparison of canonical paths, never a case fold.
//! - [`display_name`] is the only display form: NFC, then every control and invisible
//!   formatting character (the chooser's set, `Default_Ignorable_Code_Point` and the
//!   interlinear annotation controls) as `<U+XXXX>` (R-M4b-23). Identity stays the OS bytes.
//!
//! `join` answers a path, so its checks are only as fresh as the call: the opens that follow
//! (Task 5) are relative to a directory handle with `O_NOFOLLOW` (HV-SCRIPT-03 step 8).

use std::ffi::OsStr;
use std::fmt::{self, Write as _};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use icu_normalizer::ComposingNormalizerBorrowed;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::registry::LibraryRoot;
use crate::scan;

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

/// The absolute path of `rel` under `root`, after re-validating every component and the
/// canonical root (HV-SCRIPT-03 step 8). A missing last component is answered (a new script
/// or folder); a missing folder on the way is `Io(NotFound)`, never created.
pub fn join(root: &LibraryRoot, rel: &RelPath) -> Result<PathBuf, PathError> {
    let parts: Vec<&str> = rel.components().collect();
    for part in &parts {
        listable(part)?;
    }
    check_root(root)?;
    let Some((last, folders)) = parts.split_last() else {
        return Err(PathError::Malformed);
    };
    let mut path = root.canonical.clone();
    for folder in folders {
        path.push(folder);
        let meta = fs::symlink_metadata(&path).map_err(|e| PathError::Io(e.kind()))?;
        if meta.file_type().is_symlink() {
            return Err(PathError::LinkOnPath);
        }
        if !meta.is_dir() {
            return Err(PathError::NotAFolder);
        }
    }
    let parent = fs::canonicalize(&path).map_err(|e| PathError::Io(e.kind()))?;
    if !parent.starts_with(&root.canonical) {
        return Err(PathError::OutsideRoot);
    }
    path.push(last);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(path),
        Err(e) => return Err(PathError::Io(e.kind())),
    };
    if meta.file_type().is_symlink() {
        return link_target(root, &path);
    }
    if !meta.is_file() && !meta.is_dir() {
        return Err(PathError::NotAFile);
    }
    let real = fs::canonicalize(&path).map_err(|e| PathError::Io(e.kind()))?;
    if real.starts_with(&root.canonical) && real != root.canonical {
        Ok(path)
    } else {
        Err(PathError::OutsideRoot)
    }
}

/// The root still resolves, as chosen and as stored, to its canonical path.
fn check_root(root: &LibraryRoot) -> Result<(), PathError> {
    for p in [&root.canonical, &root.path] {
        match fs::canonicalize(p) {
            Ok(real) if real == root.canonical => {}
            _ => return Err(PathError::RootChanged),
        }
    }
    Ok(())
}

/// The canonical target of the file link at `link`, when it is a regular script file the scan
/// would list inside the root; `OutsideRoot` otherwise (out, dangling, a loop, a folder, a
/// skipped place such as `.git/`, another kind of file).
fn link_target(root: &LibraryRoot, link: &Path) -> Result<PathBuf, PathError> {
    let target = fs::canonicalize(link).map_err(|_| PathError::OutsideRoot)?;
    let rel = inside_rel(&root.canonical, &target).ok_or(PathError::OutsideRoot)?;
    let meta = fs::symlink_metadata(&target).map_err(|_| PathError::OutsideRoot)?;
    if meta.is_file() && scan::script_kind(rel.name()).is_some() {
        Ok(target)
    } else {
        Err(PathError::OutsideRoot)
    }
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
