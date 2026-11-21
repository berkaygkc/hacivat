//! Script file I/O: decode with its encoding, stamp, atomic save that keeps encoding, BOM and
//! line endings (HV-SCRIPT-07; UI-EDIT-02, R-M4b-20 tiers).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and fail-closed stubs: [`read`] answers `Err(ReadError::NotImplemented)`
//! and opens nothing; [`save`] answers `Err(SaveError::NotImplemented)` and writes nothing.
//! Task 5 writes them (fd-relative temp + rename, hard-link-safe, recovery copies).

use std::fmt;
use std::io;
use std::time::SystemTime;

use crate::pathsafe::RelPath;
use crate::registry::LibraryRoot;

/// What a file looked like on disk when it was read or written: `(len, mtime)` plus the
/// XXH3-128 of its bytes (`twox-hash`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiskStamp {
    pub len: u64,
    pub mtime: SystemTime,
    pub hash: u128,
}

/// A text encoding by its WHATWG name (`"UTF-8"`, `"windows-1254"`, …). Task 5 maps it onto
/// `encoding_rs`, whose types stay private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextEncoding(pub(crate) &'static str);

impl TextEncoding {
    pub const UTF_8: TextEncoding = TextEncoding("UTF-8");

    pub fn name(self) -> &'static str {
        self.0
    }
}

/// The dominant line ending of a file (ties are LF).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

/// How a file's text is kept on save.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileMeta {
    pub encoding: TextEncoding,
    pub bom: bool,
    pub line_ending: LineEnding,
}

impl Default for FileMeta {
    fn default() -> Self {
        FileMeta {
            encoding: TextEncoding::UTF_8,
            bom: false,
            line_ending: LineEnding::Lf,
        }
    }
}

/// How much of a file the editor takes (R-M4b-20; UI-EDIT-23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeTier {
    Full,
    Reduced,
    /// A read-only preview of the first part, cut at a line break.
    Preview,
}

/// Bounds of one read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadLimits {
    /// Read even past the preview threshold (the user asked for the whole file).
    pub whole: bool,
}

/// A decoded file. Its `Debug` never shows the text: scripts may hold secrets.
#[derive(Clone)]
pub struct Decoded {
    pub text: String,
    pub meta: FileMeta,
    pub stamp: DiskStamp,
    pub tier: SizeTier,
}

impl fmt::Debug for Decoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Decoded")
            .field("text_len", &self.text.len())
            .field("meta", &self.meta)
            .field("stamp", &self.stamp)
            .field("tier", &self.tier)
            .finish()
    }
}

/// The text to save and how to encode it. Its `Debug` never shows the text.
#[derive(Clone)]
pub struct SaveText {
    pub text: String,
    pub meta: FileMeta,
}

impl fmt::Debug for SaveText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaveText")
            .field("text_len", &self.text.len())
            .field("meta", &self.meta)
            .finish()
    }
}

/// Why a read failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Task 1a's stub reads nothing.
    NotImplemented,
    /// The path failed the re-validation (HV-SCRIPT-03 step 8).
    PathRefused,
    /// No BOM, not UTF-8 and no remembered choice: the user picks one.
    NeedsEncoding {
        guess: Option<TextEncoding>,
    },
    Binary,
    Io(io::ErrorKind),
}

/// Why a save failed; the file on disk is unchanged in every case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// Task 1a's stub writes nothing.
    NotImplemented,
    /// The file is not what the editor last read (HV-SCRIPT-08); carries the stamp on disk.
    ChangedOnDisk(DiskStamp),
    /// The path became a link or another kind of file since it was validated.
    PathChanged,
    /// The text has a character the file's encoding cannot hold.
    Unencodable {
        first_offset: usize,
    },
    Io(io::ErrorKind),
}

/// Reads and decodes `rel` under `root`.
///
/// Task 1a stub (fail closed): opens nothing.
pub fn read(root: &LibraryRoot, rel: &RelPath, limits: ReadLimits) -> Result<Decoded, ReadError> {
    let _ = (root, rel, limits);
    Err(ReadError::NotImplemented)
}

/// Saves `text` to `rel` under `root`; with `expect`, only if the file still has that stamp.
///
/// Task 1a stub (fail closed): writes nothing.
pub fn save(
    root: &LibraryRoot,
    rel: &RelPath,
    text: SaveText,
    expect: Option<DiskStamp>,
) -> Result<DiskStamp, SaveError> {
    let _ = (root, rel, text, expect);
    Err(SaveError::NotImplemented)
}
// ## m4b-t01a-seams
