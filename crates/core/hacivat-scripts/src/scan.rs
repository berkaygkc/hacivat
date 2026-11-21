//! The library scan (HV-SCRIPT-03; R-M4b-12..15): `lstat` only, never a content read, never a
//! directory link followed.
//!
//! ## m4b-t01a-seams
//! Task 1a: the entry types. Task 4 writes the scan (limits, skips, names, links, dataless).

use std::time::SystemTime;

use crate::envscope::EnvScope;
use crate::pathsafe::RelPath;

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
