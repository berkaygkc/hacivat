//! Script file I/O: decode with its encoding, stamp, atomic save that keeps encoding, BOM and
//! line endings (HV-SCRIPT-07; UI-EDIT-02, R-M4b-20 tiers).
//!
//! ## m4b-t01a-seams
//! Task 1a: the types and fail-closed stubs: [`read`] answers `Err(ReadError::NotImplemented)`
//! and opens nothing; [`save`] answers `Err(SaveError::NotImplemented)` and writes nothing.
//! Task 5 writes them (fd-relative temp + rename, hard-link-safe, recovery copies).
//!
//! ## m4b-t05-fileio
//! Task 5: the reader, the saver and the "Run as script" stream (HV-SCRIPT-07, -12 step 4).
//! Fix round 1 changes are marked `(fr1 …)` with the review finding.
//!
//! - **Exact round trip.** [`read`] sniffs a BOM (UTF-8, UTF-16LE/BE), else takes strict
//!   UTF-8, else answers [`ReadError::NeedsEncoding`] (the app asks, then calls [`read_with`]),
//!   and refuses as [`ReadError::Binary`] a BOM-less file with more than 1 % NUL in its first
//!   64 KiB. Nothing is ever decoded with replacement, and line endings are never normalised
//!   (the text holds them as they are; [`FileMeta::line_ending`] is the dominant one, ties
//!   LF). [`read_with`] also re-encodes what it decoded and refuses with [`ReadError::Lossy`]
//!   when that would not give the same bytes back (Shift_JIS's duplicate NEC/IBM rows), so an
//!   unchanged save never rewrites a file; a preview in a chosen encoding gets the same check
//!   (fr1 m8). [`save`] encodes in the file's encoding and BOM and refuses a character the
//!   encoding cannot hold ([`SaveError::Unencodable`]), before it touches the disk.
//! - **Tiers** (UI-EDIT-23): [`tier_for`] by size and line count, plus Reduced for a line over
//!   64 KiB; above 20 MiB the read is a preview of the first 2.5 MiB cut after the last line
//!   break, and its stamp is still the whole file's (hashed streaming, in constant memory). A
//!   preview cannot be saved: [`Decoded::save_text`] is `None` for it (fr1 I4).
//! - **Paths** (HV-SCRIPT-03 step 8; fr1 I2, ruling R-T5-2). Every call first asks
//!   [`pathsafe::join`] (Task 4): the name rules (hidden and skipped names, Windows device
//!   names, streams and 8.3 aliases), the root check, no link on the way, and a final link
//!   resolved **as text inside the root** (`resolve_link`); this module never canonicalizes
//!   and never resolves a link itself. Every public function takes the tab's environment and
//!   re-applies the scope to what `join` resolved (`envscope::visible_at`; fr2 R-T5-6: there is
//!   no unscoped form, and `None` sees no `@`-scoped file). Then the path
//!   that `join` answered (the link's target for an `InsideLink`) is walked again from the
//!   canonical root with the shared folder handles of [`dir`] (Task 3's, ruling R-T3-1: on
//!   Linux through `/proc/self/fd/<n>/`, refusing writes without procfs; on macOS with
//!   `O_NOFOLLOW_ANY`; on Windows held without `FILE_SHARE_DELETE`), refusing anything that is
//!   a link now, so a swap after `join` is refused rather than followed.
//! - **Save** (HV-SCRIPT-07 step 3): with `expect`, the file's current stamp (hashed
//!   streaming, fr1 m5) must equal it ([`SaveError::ChangedOnDisk`] carries the one on disk; a
//!   vanished file is [`SaveError::DeletedOnDisk`]); the file must be writable as it is (a
//!   read-only file is refused, never replaced). The bytes go into `.<name>.hv-<uuid v4>.tmp`
//!   in the same folder (a dot name, so the scanner never lists it; never the process id,
//!   R-T1a-4), created without following a link, 0600 until it takes the original's
//!   permission bits and owner, written and synced; the stamp is re-checked, the echo is
//!   reported ([`save_with_echo`], step 4), and the temp is renamed over the name relative to
//!   the folder handle (n8), and the folder synced. A new file is published with a hard link,
//!   which never replaces a file that appeared meanwhile (fr1 m4). Our own temps older than a
//!   minute are swept from the folder first (fr1 m2). Any failure removes the temp and leaves
//!   the original as it was.
//! - **In place** (fr1 I1, ruling R-T5-1). A file with more than one hard link, or one whose
//!   owner the temp cannot take, is written through its own handle so the links and the owner
//!   survive. Right before, the handle is re-checked against the stamp (fr1 m3) and the
//!   original bytes are kept; on any error they are written back (with the original mtime)
//!   and verified, so the file is as it was. Only when that restore fails too is the answer
//!   [`SaveError::Damaged`], and the caller keeps the new text in a recovery copy.
//! - **Stream** ([`open_stream`]): one handle for the whole run, 64 KiB reads decoded
//!   incrementally (a character cut by a read waits for the next one), and after every read
//!   the handle's and the path's stamps are compared with the ones at open; a change answers
//!   [`ChunkError::Changed`] and nothing read after it is handed out.

// ## m4b-t05-fileio fix round 1 (review m1, m4, m12): Task 3's folder handles, shared.
pub(crate) mod dir;
// ## m4b-cifix-5-windows-identity
mod windows_id;
#[cfg(windows)]
use windows_id::windows_identity;
// ## m4b-cifix-5-windows-identity

use std::fmt;
use std::fs::{File, FileTimes, Metadata};
use std::io::{self, Read, Seek, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use hacivat_proto::EnvTag;
use twox_hash::XxHash3_128;

use crate::encoding::{self, StreamDecoder};
use crate::envscope;
use crate::pathsafe::{self, JoinedKind, PathError, RelPath};
use crate::registry::LibraryRoot;
use crate::scan::{self, EntryKind};
use crate::service::{ChunkError, ScriptChunks};
use dir::{Dir, FsError, is_link};

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
    /// ## m4b-t05-fileio fix round 2 (I-fr1-2): the text is only the start of the file (a
    /// [`SizeTier::Preview`]); set only by the preview read. A save with partial meta is refused
    /// with [`SaveError::PreviewOnly`], whatever text it carries.
    pub partial: bool,
}

impl Default for FileMeta {
    fn default() -> Self {
        FileMeta {
            encoding: TextEncoding::UTF_8,
            bom: false,
            line_ending: LineEnding::Lf,
            partial: false,
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
    /// A stub that reads nothing (the service's, until Task 10).
    NotImplemented,
    /// The path failed the re-validation (HV-SCRIPT-03 step 8, [`pathsafe::join`]): a name the
    /// scan never lists, a link or junction on the way, a link out of the library, a folder or
    /// other non-file, a root that moved.
    PathRefused,
    /// ## m4b-t05-fileio fix round 1 (I2), round 2 (R-T5-6): what the path resolves to now
    /// is not shown under this connection's environment.
    OutOfScope,
    /// No BOM, not UTF-8 and no remembered choice: the user picks one ([`read_with`]).
    NeedsEncoding {
        guess: Option<TextEncoding>,
    },
    /// More than 1 % NUL bytes in the first 64 KiB, without a UTF-16 BOM (UI-EDIT-23).
    Binary,
    /// [`read_with`] got a label that names no encoding it offers.
    UnknownEncoding,
    /// The bytes decode, but encoding the text back would not give the same bytes: a save of
    /// the unchanged text would rewrite the file.
    Lossy,
    /// The bytes do not decode in the chosen encoding.
    Undecodable,
    // ## m4b-t10-service fix round 1 (named exception; I7, R-T10-3)
    /// A second pass ([`open_stream_checked`] with a [`PassRecord`]) found the file is not the
    /// one the first pass read: refused before a byte is handed out.
    Changed,
    // ## m4b-t10-service
    Io(io::ErrorKind),
}

/// Why a save failed. The file on disk is unchanged in every case but [`SaveError::Damaged`]
/// and [`SaveError::SavedButPathChanged`] (which is a completed save).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// A stub that writes nothing (the service's, until Task 10).
    NotImplemented,
    /// The file is not what the editor last read (HV-SCRIPT-08); carries the stamp on disk.
    ChangedOnDisk(DiskStamp),
    /// The path became a link or another kind of file since it was validated, or resolves out
    /// of the library.
    PathChanged,
    /// The path is not one a script may have: a name the scan never lists (a hidden component,
    /// C11; `node_modules`), a Windows device or stream name, an 8.3 alias.
    PathRefused,
    /// ## m4b-t05-fileio fix round 1 (I2), round 2 (R-T5-6): not shown under this
    /// connection's environment.
    OutOfScope,
    /// `expect` was given, and the file is gone ("Deleted on disk — Save to recreate" saves
    /// with no `expect`).
    DeletedOnDisk,
    /// The text has a character the file's encoding cannot hold.
    Unencodable {
        first_offset: usize,
    },
    /// ## m4b-t05-fileio fix round 1 (I1, ruling R-T5-1): an in-place write failed and the
    /// original bytes could not be written back: the file holds neither version. The caller
    /// keeps the new text in a recovery copy (`recovery::CopyMeta::damaged`).
    Damaged,
    /// ## m4b-t05-fileio fix round 2 (I-fr1-2): the text's meta is `partial` (a preview holds
    /// only the start of the file); nothing is written.
    PreviewOnly,
    /// ## m4b-t05-fileio fix round 2 (mf2): **the save went through**: the new bytes are on
    /// disk with this stamp (the tab is clean at it), but the folder check after the write
    /// failed (a folder on the way was moved or replaced meanwhile). A warning, never a
    /// failure: the caller treats it as saved and tells the user the folder changed.
    SavedButPathChanged(DiskStamp),
    Io(io::ErrorKind),
}

impl Decoded {
    /// ## m4b-t05-fileio fix round 1 (I4): the text as read, to be saved the way it was read;
    /// `None` for a [`SizeTier::Preview`], which holds only the start of the file and must never
    /// be saved over it.
    pub fn save_text(&self) -> Option<SaveText> {
        (self.tier != SizeTier::Preview).then(|| SaveText {
            text: self.text.clone(),
            meta: self.meta,
        })
    }
}

/// The encoding a WHATWG label names (`"windows-1254"`, `"ISO-8859-9"`, `"Shift_JIS"`), as
/// [`read_with`] takes it; `None` for an unknown label and for `replacement`. For the app's
/// encoding chooser and the recovery sidecar.
pub fn lookup_encoding(label: &str) -> Option<TextEncoding> {
    encoding::lookup(label)
}

/// The largest file in the Full tier (UI-EDIT-23 T0).
pub const FULL_MAX_BYTES: u64 = 4 << 20;
/// The most lines in the Full tier.
pub const FULL_MAX_LINES: usize = 100_000;
/// The longest line in the Full tier, in bytes.
pub const FULL_MAX_LINE_BYTES: usize = 64 << 10;
/// The largest file read whole without being asked (UI-EDIT-23 T1).
pub const REDUCED_MAX_BYTES: u64 = 20 << 20;
/// The largest file read whole when asked ([`ReadLimits::whole`], UI-EDIT-23 "Open anyway"),
/// and the largest one written in place (its original is kept in memory, fr1 I1).
pub const WHOLE_MAX_BYTES: u64 = 64 << 20;
/// How much of a file a preview reads: 2.5 MiB.
pub const PREVIEW_BYTES: usize = 5 << 19;
/// How much of a file the binary check looks at.
pub const BINARY_PROBE_BYTES: usize = 64 << 10;
/// One read of [`open_stream`].
pub const STREAM_CHUNK_BYTES: usize = 64 << 10;
/// The longest file name the temp name may reach (`NAME_MAX` on every target).
const NAME_MAX_BYTES: usize = 255;
const TEMP_MARK: &str = ".hv-";
const TEMP_SUFFIX: &str = ".tmp";
/// The random part of a temp name: a v4 UUID in simple form.
const TEMP_RANDOM_LEN: usize = 32;
/// One read of a streaming hash.
const HASH_CHUNK_BYTES: usize = 1 << 20;

/// The tier of a file of `len` bytes and `lines` lines (UI-EDIT-23 step 1: Full up to 4 MiB
/// and 100,000 lines, Reduced up to 20 MiB, Preview above).
pub fn tier_for(len: u64, lines: usize) -> SizeTier {
    if len > REDUCED_MAX_BYTES {
        SizeTier::Preview
    } else if len > FULL_MAX_BYTES || lines > FULL_MAX_LINES {
        SizeTier::Reduced
    } else {
        SizeTier::Full
    }
}

/// Reads and decodes `rel` under `root` for a tab under a connection whose environment is
/// `env` (UI-EDIT-02 step 1): a BOM names the encoding, else strict UTF-8, else
/// [`ReadError::NeedsEncoding`]. Above 20 MiB (64 MiB with [`ReadLimits::whole`]) it reads a
/// preview, whose meta is `partial`.
///
/// ## m4b-t05-fileio fix round 2 (I-fr1-1, ruling R-T5-6)
/// `env` is required: what the path resolves to must be shown under it
/// (`envscope::visible_at`), else [`ReadError::OutOfScope`]. `None` is an untagged connection
/// or an unbound tab, which sees no `@`-scoped file (fail closed). There is no unscoped form.
pub fn read(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    limits: ReadLimits,
) -> Result<Decoded, ReadError> {
    let opened = open_for_read(root, rel, env)?;
    let cap = if limits.whole {
        WHOLE_MAX_BYTES
    } else {
        REDUCED_MAX_BYTES
    };
    if opened.meta.len() > cap {
        return read_preview(opened, None);
    }
    let len = opened.meta.len();
    let (bytes, stamp) = read_all(opened)?;
    let (text, meta) = decode_sniffed(&bytes)?;
    Ok(decoded(text, meta, stamp, len, limits.whole))
}

/// Reads `rel` in the encoding the user chose by its WHATWG label (`"windows-1254"`), after
/// [`ReadError::NeedsEncoding`], under `env` as [`read`]. The encoding's own BOM, if present, is
/// kept for the save.
pub fn read_with(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    encoding: &str,
) -> Result<Decoded, ReadError> {
    let enc = encoding::lookup(encoding).ok_or(ReadError::UnknownEncoding)?;
    let opened = open_for_read(root, rel, env)?;
    if opened.meta.len() > REDUCED_MAX_BYTES {
        return read_preview(opened, Some(enc));
    }
    let len = opened.meta.len();
    let (bytes, stamp) = read_all(opened)?;
    let own_bom = encoding::bom_of(enc);
    let bom = !own_bom.is_empty() && bytes.starts_with(own_bom);
    let body = if bom { &bytes[own_bom.len()..] } else { &bytes };
    let text = encoding::decode_exact(enc, body).ok_or(ReadError::Undecodable)?;
    match encoding::encode_exact(enc, bom, &text) {
        Ok(back) if back == bytes => {}
        _ => return Err(ReadError::Lossy),
    }
    let line_ending = line_stats(&text).ending;
    let meta = FileMeta {
        encoding: enc,
        bom,
        line_ending,
        partial: false,
    };
    Ok(decoded(text, meta, stamp, len, false))
}

/// Saves `text` to `rel` under `root` for a tab under `env` (checked as in [`read`], else
/// [`SaveError::OutOfScope`]); with `expect`, only if the file still has that stamp. Returns
/// the stamp of what is now on disk (HV-SCRIPT-07 step 3).
pub fn save(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    text: SaveText,
    expect: Option<DiskStamp>,
) -> Result<DiskStamp, SaveError> {
    save_with_echo(root, rel, env, text, expect, &mut |_| {})
}

/// [`save`], calling `echo` with the stamp of the new file before it replaces the old one
/// (HV-SCRIPT-07 step 4: the service records it in the watcher's echo table, so the save's own
/// events are dropped). An in-place write reports it right after the write.
pub fn save_with_echo(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    text: SaveText,
    expect: Option<DiskStamp>,
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, SaveError> {
    save_echoing_landing(root, rel, env, text, expect, &mut |_, stamp| echo(stamp))
}

// ## m4b-t10-service fix round 3 (m-f; a Task 5 named exception)
/// [`save_with_echo`], whose callback also gets the path the bytes land at, relative to the
/// root: for a file link, its target inside the library (the `join` this save made, not an
/// older listing's), else `rel`. The service records the echo under it, which is where the
/// watcher sees the write.
pub fn save_echoing_landing(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    text: SaveText,
    expect: Option<DiskStamp>,
    echo: &mut dyn FnMut(&RelPath, &DiskStamp),
) -> Result<DiskStamp, SaveError> {
    // fr2 I-fr1-2: a preview's meta travels with whatever text the tab builds; never saved.
    if text.meta.partial {
        return Err(SaveError::PreviewOnly);
    }
    let bytes = encoding::encode_exact(text.meta.encoding, text.meta.bom, &text.text)
        .map_err(|first_offset| SaveError::Unencodable { first_offset })?;
    drop(text);
    let (target, landing) = locate_landing(root, rel, env).map_err(Refusal::for_save)?;
    let echo = &mut |stamp: &DiskStamp| echo(&landing, stamp);
    let dir = target.dir();
    dir.sweep_stale_temps(&is_temp_name);
    match target {
        Target::Missing { dir, name } => {
            if expect.is_some() {
                return Err(SaveError::DeletedOnDisk);
            }
            publish_new(&dir, &name, &bytes, echo)
        }
        Target::Existing { dir, name, lstat } => {
            let current = stamp_streaming(&dir, &name, &lstat)?;
            if let Some(expected) = expect
                && expected != current
            {
                return Err(SaveError::ChangedOnDisk(current));
            }
            // Writable as it is: a read-only file is refused, never replaced.
            let writer = dir.open_write(&name, &lstat).map_err(fs_save_error)?;
            let original = Original {
                meta: &lstat,
                stamp: current,
            };
            if hard_links(&lstat) > 1 {
                return in_place(&dir, &name, writer, &original, &bytes, echo);
            }
            // Not held across the rename (Windows replaces only a file nobody holds).
            drop(writer);
            match replace(&dir, &name, &bytes, &original, echo) {
                Ok(stamp) => Ok(stamp),
                Err(Replace::Failed(e)) => Err(e),
                Err(Replace::OwnerNotKept) => {
                    let writer = dir.open_write(&name, &lstat).map_err(fs_save_error)?;
                    in_place(&dir, &name, writer, &original, &bytes, echo)
                }
            }
        }
    }
}

// ## m4b-t10-service (named exception): the service's stamp of a changed file.
/// The current stamp of `rel` under `env`, scoped and located exactly as [`read`] (the same
/// `join`, `visible_at` and no-follow folder handles), its bytes hashed streaming and dropped:
/// nothing is decoded or kept. The library service uses it for its echo, change and rename
/// checks (HV-SCRIPT-08 steps 5–7), so script content is read only here, in `fileio` (T5 mg2).
pub(crate) fn stamp_of(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
) -> Result<DiskStamp, ReadError> {
    match locate(root, rel, env).map_err(Refusal::for_read)? {
        Target::Missing { .. } => Err(ReadError::Io(io::ErrorKind::NotFound)),
        Target::Existing { dir, name, lstat } => {
            stamp_streaming(&dir, &name, &lstat).map_err(|e| match e {
                SaveError::Io(kind) => ReadError::Io(kind),
                _ => ReadError::PathRefused,
            })
        }
    }
}
// ## m4b-t10-service

/// A chunked UTF-8 reader of `rel` in `encoding` for "Run as script" (HV-SCRIPT-12), under
/// `env` as [`read`]: the source behind `LibraryService::open_stream` (Task 10 wires it). Its
/// BOM is dropped.
pub fn open_stream(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    encoding: TextEncoding,
) -> Result<Box<dyn ScriptChunks>, ReadError> {
    Ok(Box::new(open_stream_checked(
        root, rel, env, encoding, None,
    )?))
}

// ## m4b-t10-service fix round 1 (named exception; I7, ruling R-T10-3)
/// What one pass of a [`ScriptStream`] read: the file's identity at open (length, mtime and,
/// privately, its full fingerprint: device, inode and ctime on Unix) and a running XXH3-64 of
/// the raw bytes at the end of every 64 KiB block (one `u64` per block, so memory stays
/// bounded: 2,400 values for a 150 MB file).
///
/// **The pass-1 / pass-2 contract (Task 12, R-M4b-47).** Pass 1 streams the file and keeps
/// [`ScriptStream::record`]. Pass 2 calls [`open_stream_checked`] with that record: it is
/// refused at once ([`ReadError::Changed`]) unless the file has the fingerprint pass 1 opened,
/// and it stops with [`ChunkError::Changed`] the moment a block's running hash differs from
/// pass 1's, before that block's text is handed out (or when the file has more or fewer blocks
/// than pass 1 read). Pass 2 therefore sees exactly pass 1's bytes, chunked the same way, or
/// nothing past the first difference. A record of an incomplete pass checks only the blocks it
/// holds and then refuses to read further.
///
/// Fix round 2 (F4): the record binds the decoding too. Pass 2 is refused at open when its
/// encoding is not pass 1's, and stops at its first block when the BOM pass 1 found (and
/// dropped) is not there. The stream never decides a line ending (the text keeps the file's
/// own), so there is no line-ending choice to bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassRecord {
    pub len: u64,
    pub mtime: SystemTime,
    pub encoding: TextEncoding,
    /// Whether the first block began with the encoding's BOM (dropped from the text); `None`
    /// before the first block was read.
    pub bom: Option<bool>,
    fingerprint: Fingerprint,
    blocks: Vec<u64>,
    complete: bool,
}

impl PassRecord {
    /// How many 64 KiB blocks the pass read.
    pub fn blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the pass read to the end of the file.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    #[cfg(test)]
    fn tamper(&mut self, block: usize) {
        self.blocks[block] ^= 1;
    }
}

/// A "Run as script" reader that records what it read ([`PassRecord`]).
pub struct ScriptStream(FileStream);

impl fmt::Debug for ScriptStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptStream")
            .field("blocks", &self.0.blocks.len())
            .finish_non_exhaustive()
    }
}

impl ScriptStream {
    /// The opening stamp and the running hashes so far.
    pub fn record(&self) -> PassRecord {
        PassRecord {
            len: self.0.at_open.len,
            mtime: self.0.at_open.mtime.unwrap_or(UNIX_EPOCH),
            encoding: self.0.encoding,
            bom: self.0.bom,
            fingerprint: self.0.at_open,
            blocks: self.0.blocks.clone(),
            complete: self.0.done && !self.0.failed,
        }
    }
}

impl ScriptChunks for ScriptStream {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        self.0.next_chunk()
    }
}

/// [`open_stream`] that records its pass; with `expect`, a second pass checked against the
/// first ([`PassRecord`]).
pub fn open_stream_checked(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
    encoding: TextEncoding,
    expect: Option<PassRecord>,
) -> Result<ScriptStream, ReadError> {
    let decoder = StreamDecoder::new(encoding).ok_or(ReadError::UnknownEncoding)?;
    let opened = open_for_read(root, rel, env)?;
    let at_open = Fingerprint::of(&opened.meta);
    if expect
        .as_ref()
        .is_some_and(|e| e.fingerprint != at_open || e.encoding != encoding)
    {
        return Err(ReadError::Changed);
    }
    Ok(ScriptStream(FileStream {
        at_open,
        // ## m4b-cifix-5-windows-identity
        path_id: path_identity(&opened.dir, &opened.name),
        // ## m4b-cifix-5-windows-identity
        dir: opened.dir,
        name: opened.name,
        file: opened.file,
        decoder,
        done: false,
        failed: false,
        hasher: twox_hash::XxHash3_64::new(),
        blocks: Vec::new(),
        encoding,
        bom: None,
        expect_bom: expect.as_ref().and_then(|e| e.bom),
        expect: expect.map(|e| e.blocks),
    }))
}

/// The identity a library root keeps for a session (I3): its device and inode, and on Linux
/// its birth time.
///
/// ## m4b-cifix-2: device and inode alone do not tell a folder from one made again at the same
/// path. Linux's ext4 hands a freed inode to the next `mkdir` in the same block group, so a
/// root deleted and made again often keeps its number (CI 44990057141). The birth time
/// (`statx` `btime`, read through `std`'s `Metadata::created`) is new for the new folder and
/// survives a rename, so a root that went away and came back under another name stays the
/// same folder.
///
/// ## m4b-cifix-2 fix round (review C1, R-CI2-1): Linux only, where userland cannot set
/// `btime`. On macOS, setting a folder's modification time earlier than its birth time moves
/// the birth time back (APFS, HFS+), so an `rsync -a`, `cp -Rp`, `tar -x` or `touch -t` into
/// the root would refuse an unchanged library. APFS never reuses an object id (HFS+ CNIDs only
/// grow), so `(dev, ino)` is enough there. Residual: a network or FUSE mount may report a
/// settable birth time (an NFSv4 `time_create`, an SMB creation time); such a library is
/// polled, and a restore into its root can still refuse it (ADR 0009).
///
/// ## m4b-cifix-4: the same identity now serves files too ([`FileIdentity`], a SQLite script
/// tab's database file, R-T14-1): a folder's is a [`FileIdentity`] read from a folder.
pub(crate) type FolderIdentity = FileIdentity;

// ## m4b-cifix-4-file-identity
/// The identity of a file or folder: its device and inode (Unix), and on Linux its birth time
/// (m4b-cifix-2, R-CI2-1; m4b-cifix-4 for files).
///
/// Device and inode alone do not tell a file from one made again at the same path: on ext4 a
/// file deleted and written again in the same folder usually gets the freed inode back (CI
/// 45077623376, a SQLite script tab whose database file was replaced at its path ran on the
/// new file). The birth time (`statx` `btime`) is new for the new file, userland cannot set it,
/// and a rename keeps it. On macOS the birth time moves back with a modification time set
/// earlier, and APFS never reuses an object id, so `(dev, ino)` stays the identity there.
/// Residual: a delete and a create inside one tick of Linux's coarse clock (1 to 4 ms), on an
/// inode ext4 hands back, keep the same identity; the inode generation would close it, but
/// reading it needs an `ioctl` (`unsafe`), as for folders (ADR 0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    pub dev: u64,
    pub ino: u64,
    /// The birth time on Linux; `None` elsewhere, or where the file system reports none. Both
    /// sides of a comparison are read on the same file system, so `None` matches only `None`.
    pub born: Option<SystemTime>,
}

impl FileIdentity {
    /// The identity `meta` describes (Unix); `None` off Unix, where `std` has none.
    ///
    /// ## m4b-cifix-5: a Windows `Metadata` carries no stable identity either (its volume
    /// serial and file index are behind the unstable `windows_by_handle`), so a SQLite tab's
    /// file stays path-only there (T14's residual). A folder's identity, and the read stream's
    /// path check, read the real one by path instead (`windows_id::windows_identity`).
    pub fn of(meta: &Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Some(FileIdentity {
                dev: meta.dev(),
                ino: meta.ino(),
                born: unsettable_birth(meta),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = meta;
            None
        }
    }

    /// The text the app keeps it as (`workspace.json`, a recovery copy's sidecar):
    /// `dev:ino`, then `:secs.nanos` of the birth time when it has one (`-secs.nanos` before
    /// 1970). [`FileIdentity::parse`] reads it back exactly.
    pub fn to_text(&self) -> String {
        let (dev, ino) = (self.dev, self.ino);
        match self.born {
            None => format!("{dev}:{ino}"),
            Some(t) => match t.duration_since(UNIX_EPOCH) {
                Ok(d) => format!("{dev}:{ino}:{}.{:09}", d.as_secs(), d.subsec_nanos()),
                Err(e) => {
                    let d = e.duration();
                    format!("{dev}:{ino}:-{}.{:09}", d.as_secs(), d.subsec_nanos())
                }
            },
        }
    }

    /// Reads [`FileIdentity::to_text`] back; `None` for anything else. A text without a birth
    /// time (written before m4b-cifix-4) reads as none, so on Linux it no longer matches the
    /// file it named: a Run refuses until the tab is opened again (fail closed).
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(':');
        let dev = parts.next()?.parse().ok()?;
        let ino = parts.next()?.parse().ok()?;
        let born = match parts.next() {
            None => None,
            Some(b) => {
                let (before, b) = match b.strip_prefix('-') {
                    Some(rest) => (true, rest),
                    None => (false, b),
                };
                let (secs, nanos) = b.split_once('.')?;
                if nanos.len() != 9 || !nanos.bytes().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                let d = std::time::Duration::new(secs.parse().ok()?, nanos.parse().ok()?);
                Some(if before {
                    UNIX_EPOCH.checked_sub(d)?
                } else {
                    UNIX_EPOCH.checked_add(d)?
                })
            }
        };
        if parts.next().is_some() {
            return None;
        }
        Some(FileIdentity { dev, ino, born })
    }
}
// ## m4b-cifix-4-file-identity

/// The part of [`FolderIdentity`] userland cannot set: the birth time, on Linux only.
#[cfg(unix)]
fn unsettable_birth(meta: &std::fs::Metadata) -> Option<std::time::SystemTime> {
    #[cfg(target_os = "linux")]
    {
        meta.created().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = meta;
        None
    }
}

/// The [`FolderIdentity`] of a folder (Unix, and Windows since m4b-cifix-5); `None` where it
/// cannot be read or the path is not a real folder (a link or junction is never one).
pub(crate) fn folder_identity(path: &std::path::Path) -> Option<FolderIdentity> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    #[cfg(windows)]
    {
        // The id is read through a handle that follows a link: the path must still be a real
        // folder afterwards, or the id may name the link's target.
        let id = windows_identity(path)?;
        let again = std::fs::symlink_metadata(path).ok()?;
        again.is_dir().then_some(id)
    }
    #[cfg(not(windows))]
    {
        FileIdentity::of(&meta)
    }
}

// ## m4b-cifix-5-windows-identity
/// The Windows identity of `name` in `dir`, for the read stream's path check; `None` off
/// Windows, where the [`Fingerprint`] already holds the device and inode.
fn path_identity(dir: &Dir, name: &str) -> Option<FileIdentity> {
    #[cfg(windows)]
    {
        let m = dir.lstat(name).ok()?;
        if is_link(&m) {
            return None;
        }
        windows_identity(&dir.child(name))
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, name);
        None
    }
}
// ## m4b-cifix-5-windows-identity
// ## m4b-t10-service

/// The temp name a save of `name` writes first: `.<name>.hv-<uuid v4>.tmp`, with `<name>` cut
/// on a character boundary so the whole stays within 255 bytes (alg/28 m10). It starts with
/// `.`, so the scanner never lists it (C11), and never ends in `.sql` or `.md`.
pub fn temp_name_for(name: &str) -> String {
    let room = NAME_MAX_BYTES - 1 - TEMP_MARK.len() - TEMP_RANDOM_LEN - TEMP_SUFFIX.len();
    let mut cut = name.len().min(room);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        ".{}{TEMP_MARK}{}{TEMP_SUFFIX}",
        &name[..cut],
        uuid::Uuid::new_v4().simple()
    )
}

/// Whether `name` is exactly a [`temp_name_for`] name: a leading `.`, then `.hv-`, 32
/// lowercase hex digits and `.tmp` (the watcher's expected transient, HV-SCRIPT-07 step 4, and
/// the stale-temp sweep, fr1 m2).
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with('.')
        && name
            .strip_suffix(TEMP_SUFFIX)
            .and_then(|s| s.rsplit_once(TEMP_MARK))
            .is_some_and(|(_, random)| {
                random.len() == TEMP_RANDOM_LEN
                    && random
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
}

/// Fault injection for the failure tests (feature `test-support`; never in a product build):
/// the next save on this thread fails at the given step.
#[cfg(any(test, feature = "test-support"))]
pub mod faults {
    use std::cell::{Cell, RefCell};
    use std::io;

    /// A step of a save.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fault {
        /// Before any byte is written.
        Write,
        /// ## fix round 1 (I1): after this many bytes are written (an ENOSPC or EFBIG stand-in).
        WriteAfter(usize),
        Sync,
        Rename,
        /// ## fix round 1 (I1): the write-back of the original after a failed in-place write.
        Restore,
        /// ## fix round 2 (mf1): the last sync of that write-back.
        RestoreSync,
    }

    thread_local! {
        static NEXT: Cell<Option<Fault>> = const { Cell::new(None) };
        static LAST_TEMP: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// The next save on this thread fails at `f`, once.
    pub fn inject(f: Fault) {
        NEXT.set(Some(f));
    }

    /// Two faults: `first` fires at its step, then `then` at its own (a failed write whose
    /// restore fails too).
    pub fn inject_both(first: Fault, then: Fault) {
        NEXT.set(Some(first));
        AFTER.set(Some(then));
    }

    thread_local! {
        static AFTER: Cell<Option<Fault>> = const { Cell::new(None) };
    }

    thread_local! {
        static BEFORE_PUBLISH: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    /// Runs `f` once, on this thread, in the last window of a save: between the stamp check
    /// and an in-place write (fix round 1 m3), or between the re-check and the publish of a new
    /// file (m4). Another writer acts there.
    pub fn before_publish(f: impl FnOnce() + 'static) {
        BEFORE_PUBLISH.set(Some(Box::new(f)));
    }

    pub(super) fn run_before_publish() {
        if let Some(f) = BEFORE_PUBLISH.take() {
            f();
        }
    }

    thread_local! {
        static AFTER_PUBLISH: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    /// ## fix round 2 (mf2): runs `f` once, on this thread, right after a save published its
    /// bytes (rename, hard link or in-place write) and before the folder check that follows.
    pub fn after_publish(f: impl FnOnce() + 'static) {
        AFTER_PUBLISH.set(Some(Box::new(f)));
    }

    pub(super) fn run_after_publish() {
        if let Some(f) = AFTER_PUBLISH.take() {
            f();
        }
    }

    /// The temp name the last save on this thread created.
    pub fn last_temp() -> Option<String> {
        LAST_TEMP.with_borrow(Clone::clone)
    }

    fn fire() {
        NEXT.set(AFTER.take());
    }

    pub(super) fn hit(at: Fault) -> io::Result<()> {
        if NEXT.get() == Some(at) {
            fire();
            return Err(io::Error::other("injected fault"));
        }
        Ok(())
    }

    /// How many bytes of a write go through before an injected `WriteAfter` fault.
    pub(super) fn write_budget() -> Option<usize> {
        match NEXT.get() {
            Some(Fault::WriteAfter(n)) => {
                fire();
                Some(n)
            }
            _ => None,
        }
    }

    pub(super) fn record_temp(name: &str) {
        LAST_TEMP.set(Some(name.to_owned()));
    }
}

#[cfg(not(any(test, feature = "test-support")))]
mod faults {
    use std::io;

    #[derive(Clone, Copy)]
    pub(super) enum Fault {
        Write,
        Sync,
        Rename,
        Restore,
        RestoreSync,
    }

    #[inline]
    pub(super) fn hit(at: Fault) -> io::Result<()> {
        let (Fault::Write | Fault::Sync | Fault::Rename | Fault::Restore | Fault::RestoreSync) = at;
        Ok(())
    }

    #[inline]
    pub(super) fn write_budget() -> Option<usize> {
        None
    }

    #[inline]
    pub(super) fn record_temp(_: &str) {}

    #[inline]
    pub(super) fn run_before_publish() {}

    #[inline]
    pub(super) fn run_after_publish() {}
}

use faults::Fault;

/// `write_all`, unless a fault says otherwise (a `WriteAfter(n)` writes `n` bytes and fails).
fn write_faulted(mut file: &File, bytes: &[u8]) -> io::Result<()> {
    faults::hit(Fault::Write)?;
    if let Some(n) = faults::write_budget() {
        file.write_all(&bytes[..n.min(bytes.len())])?;
        return Err(io::Error::other("injected partial write"));
    }
    file.write_all(bytes)
}

// ---- paths -------------------------------------------------------------------------------------

/// Why a path was not opened.
#[derive(Debug)]
enum Refusal {
    Path(PathError),
    Fs(FsError),
    OutOfScope,
}

impl From<PathError> for Refusal {
    fn from(e: PathError) -> Self {
        Refusal::Path(e)
    }
}

impl From<FsError> for Refusal {
    fn from(e: FsError) -> Self {
        Refusal::Fs(e)
    }
}

impl Refusal {
    fn for_read(self) -> ReadError {
        match self {
            Refusal::Path(PathError::Io(k)) => ReadError::Io(k),
            Refusal::Fs(FsError::Io(e)) => ReadError::Io(e.kind()),
            Refusal::OutOfScope => ReadError::OutOfScope,
            Refusal::Path(_) | Refusal::Fs(_) => ReadError::PathRefused,
        }
    }

    fn for_save(self) -> SaveError {
        match self {
            Refusal::Path(e) => match e {
                PathError::Io(k) => SaveError::Io(k),
                PathError::Malformed
                | PathError::Name(_)
                | PathError::Alias
                | PathError::RootInEnvFolder => SaveError::PathRefused,
                PathError::LinkOnPath
                | PathError::NotAFolder
                | PathError::NotAFile
                | PathError::OutsideRoot
                | PathError::RootChanged
                | PathError::Changed => SaveError::PathChanged,
            },
            Refusal::Fs(e) => fs_save_error(e),
            Refusal::OutOfScope => SaveError::OutOfScope,
        }
    }
}

fn fs_save_error(e: FsError) -> SaveError {
    match e {
        FsError::Io(e) => SaveError::Io(e.kind()),
        FsError::Link
        | FsError::NotDir
        | FsError::NotFile
        | FsError::TooLarge
        | FsError::Changed => SaveError::PathChanged,
    }
}

fn fs_read_error(e: FsError) -> ReadError {
    Refusal::Fs(e).for_read()
}

/// Where a path ends, under its folder handle: an existing regular file, or nothing yet.
enum Target {
    Existing {
        dir: Dir,
        name: String,
        lstat: Metadata,
    },
    Missing {
        dir: Dir,
        name: String,
    },
}

impl Target {
    fn dir(&self) -> &Dir {
        match self {
            Target::Existing { dir, .. } | Target::Missing { dir, .. } => dir,
        }
    }
}

/// `pathsafe::join` decides (fr1 I2: the link, if any, resolved as text inside the root); then
/// the path it answered is walked with no-follow folder handles, and anything that is a link
/// now is refused.
fn locate(root: &LibraryRoot, rel: &RelPath, env: Option<EnvTag>) -> Result<Target, Refusal> {
    locate_landing(root, rel, env).map(|(target, _)| target)
}

/// [`locate`], with the path it opens relative to the root (a file link's target, else
/// `rel`). ## m4b-t10-service fix round 3 (m-f)
fn locate_landing(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
) -> Result<(Target, RelPath), Refusal> {
    let joined = pathsafe::join(root, rel)?;
    {
        let kind = match joined.kind {
            JoinedKind::Folder => EntryKind::Folder,
            JoinedKind::File | JoinedKind::Absent => {
                scan::script_kind(rel.name()).unwrap_or(EntryKind::Sql)
            }
        };
        if !envscope::visible_at(rel, kind, joined.target.as_ref(), env) {
            return Err(Refusal::OutOfScope);
        }
    }
    if joined.kind == JoinedKind::Folder {
        return Err(Refusal::Fs(FsError::NotFile));
    }
    let open = joined.target.as_ref().unwrap_or(rel);
    let mut dir = Dir::open_canonical(&root.canonical)?;
    let parts: Vec<&str> = open.components().collect();
    let Some((last, folders)) = parts.split_last() else {
        return Err(Refusal::Path(PathError::Malformed));
    };
    for folder in folders {
        dir = dir.subdir(folder)?;
    }
    let name = (*last).to_owned();
    let lstat = match dir.lstat(&name) {
        Ok(m) => m,
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            // A link's target that vanished is not a place to create a file.
            if joined.target.is_some() {
                return Err(Refusal::Path(PathError::Changed));
            }
            return Ok((Target::Missing { dir, name }, open.clone()));
        }
        Err(e) => return Err(e.into()),
    };
    if is_link(&lstat) {
        return Err(Refusal::Path(PathError::Changed));
    }
    if !lstat.is_file() {
        return Err(Refusal::Fs(FsError::NotFile));
    }
    Ok((Target::Existing { dir, name, lstat }, open.clone()))
}

// ---- reading -------------------------------------------------------------------------------

/// An open regular file under its folder handle, and its `fstat`.
struct Opened {
    dir: Dir,
    name: String,
    file: File,
    meta: Metadata,
}

fn open_for_read(
    root: &LibraryRoot,
    rel: &RelPath,
    env: Option<EnvTag>,
) -> Result<Opened, ReadError> {
    match locate(root, rel, env).map_err(Refusal::for_read)? {
        Target::Missing { .. } => Err(ReadError::Io(io::ErrorKind::NotFound)),
        Target::Existing { dir, name, lstat } => {
            let file = dir.open_read(&name, &lstat).map_err(fs_read_error)?;
            let meta = file.metadata().map_err(|e| ReadError::Io(e.kind()))?;
            Ok(Opened {
                dir,
                name,
                file,
                meta,
            })
        }
    }
}

/// All bytes of a file of at most [`WHOLE_MAX_BYTES`], and their stamp. A file that changes
/// while it is read answers `Io(Interrupted)` (the watcher reads it again once it settles).
fn read_all(opened: Opened) -> Result<(Vec<u8>, DiskStamp), ReadError> {
    let io_err = |e: io::Error| ReadError::Io(e.kind());
    let len = opened.meta.len();
    let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    (&opened.file)
        .take(len + 1)
        .read_to_end(&mut bytes)
        .map_err(io_err)?;
    let after = opened.file.metadata().map_err(io_err)?;
    if bytes.len() as u64 != len || Fingerprint::of(&after) != Fingerprint::of(&opened.meta) {
        return Err(ReadError::Io(io::ErrorKind::Interrupted));
    }
    let stamp = stamp_from(&after, bytes.len() as u64, XxHash3_128::oneshot(&bytes));
    Ok((bytes, stamp))
}

/// The first [`PREVIEW_BYTES`] decoded and cut after the last line break, with the whole
/// file's stamp (hashed streaming). `chosen` is the user's encoding, else the sniffed one.
fn read_preview(opened: Opened, chosen: Option<TextEncoding>) -> Result<Decoded, ReadError> {
    let io_err = |e: io::Error| ReadError::Io(e.kind());
    let mut file = &opened.file;
    let mut prefix = vec![0u8; PREVIEW_BYTES];
    let mut got = 0;
    while got < prefix.len() {
        match file.read(&mut prefix[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(e)),
        }
    }
    prefix.truncate(got);
    let mut hasher = XxHash3_128::new();
    hasher.write(&prefix);
    let mut total = prefix.len() as u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                hasher.write(&buf[..n]);
                total += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err(e)),
        }
    }
    let after = opened.file.metadata().map_err(io_err)?;
    if total != opened.meta.len() || Fingerprint::of(&after) != Fingerprint::of(&opened.meta) {
        return Err(ReadError::Io(io::ErrorKind::Interrupted));
    }
    let stamp = stamp_from(&after, total, hasher.finish_128());

    let sniffed = encoding::sniff_bom(&prefix).map(|(e, _)| e);
    let enc = match (chosen, sniffed) {
        (Some(c), _) => c,
        (None, Some(s)) => s,
        (None, None) if looks_binary(&prefix) => return Err(ReadError::Binary),
        (None, None) => TextEncoding::UTF_8,
    };
    let bom = {
        let own = encoding::bom_of(enc);
        !own.is_empty() && prefix.starts_with(own)
    };
    let mut decoder = StreamDecoder::new(enc).ok_or(ReadError::UnknownEncoding)?;
    let Some(mut text) = decoder.feed(&prefix, false) else {
        return Err(if chosen.is_some() {
            ReadError::Undecodable
        } else {
            ReadError::NeedsEncoding { guess: None }
        });
    };
    text.truncate(preview_cut(&text));
    // fr1 m8: the preview must encode back to the bytes it came from, as `read_with` checks.
    match encoding::encode_exact(enc, bom, &text) {
        Ok(back) if prefix.starts_with(&back) => {}
        _ => return Err(ReadError::Lossy),
    }
    let meta = FileMeta {
        encoding: enc,
        bom,
        line_ending: line_stats(&text).ending,
        partial: true,
    };
    Ok(Decoded {
        text,
        meta,
        stamp,
        tier: SizeTier::Preview,
    })
}

/// Where a preview ends: after the last `\n`, else after the last `\r` that is not the final
/// character, else the whole decoded prefix without a final `\r` (which may be the first half
/// of a CRLF the cut split).
fn preview_cut(text: &str) -> usize {
    if let Some(i) = text.rfind('\n') {
        return i + 1;
    }
    match text.rfind('\r') {
        Some(i) if i + 1 < text.len() => i + 1,
        _ => text.strip_suffix('\r').map_or(text.len(), str::len),
    }
}

/// UI-EDIT-02 step 1 on a whole file.
fn decode_sniffed(bytes: &[u8]) -> Result<(String, FileMeta), ReadError> {
    if let Some((enc, n)) = encoding::sniff_bom(bytes)
        && let Some(text) = encoding::decode_exact(enc, &bytes[n..])
    {
        let line_ending = line_stats(&text).ending;
        return Ok((
            text,
            FileMeta {
                encoding: enc,
                bom: true,
                line_ending,
                partial: false,
            },
        ));
    }
    if looks_binary(bytes) {
        return Err(ReadError::Binary);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let line_ending = line_stats(text).ending;
            Ok((
                text.to_owned(),
                FileMeta {
                    encoding: TextEncoding::UTF_8,
                    bom: false,
                    line_ending,
                    partial: false,
                },
            ))
        }
        Err(_) => Err(ReadError::NeedsEncoding { guess: None }),
    }
}

fn decoded(text: String, meta: FileMeta, stamp: DiskStamp, len: u64, whole: bool) -> Decoded {
    let stats = line_stats(&text);
    let tier = match tier_for(len, stats.lines) {
        SizeTier::Full if stats.longest > FULL_MAX_LINE_BYTES => SizeTier::Reduced,
        // Asked for the whole file past 20 MiB: Reduced, with a warning (the app's).
        SizeTier::Preview if whole => SizeTier::Reduced,
        t => t,
    };
    Decoded {
        text,
        meta,
        stamp,
        tier,
    }
}

/// More than 1 % NUL bytes in the first [`BINARY_PROBE_BYTES`] (UI-EDIT-23 edge case).
fn looks_binary(bytes: &[u8]) -> bool {
    let probe = &bytes[..bytes.len().min(BINARY_PROBE_BYTES)];
    let nul = probe.iter().filter(|b| **b == 0).count();
    nul * 100 > probe.len()
}

struct LineStats {
    ending: LineEnding,
    lines: usize,
    longest: usize,
}

/// Line breaks are exactly LF, CRLF and a lone CR (UI-EDIT-02 step 2); the dominant one is the
/// most frequent, LF on a tie or when there is none (step 3).
fn line_stats(text: &str) -> LineStats {
    let b = text.as_bytes();
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let (mut longest, mut start, mut i) = (0usize, 0usize, 0usize);
    while i < b.len() {
        let width = match b[i] {
            b'\n' => {
                lf += 1;
                1
            }
            b'\r' if b.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                2
            }
            b'\r' => {
                cr += 1;
                1
            }
            _ => {
                i += 1;
                continue;
            }
        };
        longest = longest.max(i - start);
        i += width;
        start = i;
    }
    longest = longest.max(b.len() - start);
    let top = lf.max(crlf).max(cr);
    let at_top = [lf, crlf, cr].iter().filter(|n| **n == top).count();
    let ending = if top == 0 || at_top > 1 || lf == top {
        LineEnding::Lf
    } else if crlf == top {
        LineEnding::CrLf
    } else {
        LineEnding::Cr
    };
    LineStats {
        ending,
        lines: lf + crlf + cr + 1,
        longest,
    }
}

// ---- saving --------------------------------------------------------------------------------

/// The file a save replaces, as it was checked.
struct Original<'a> {
    meta: &'a Metadata,
    stamp: DiskStamp,
}

enum Replace {
    Failed(SaveError),
    /// The temp file cannot take the original's owner: write in place instead.
    OwnerNotKept,
}

impl From<SaveError> for Replace {
    fn from(e: SaveError) -> Self {
        Replace::Failed(e)
    }
}

impl From<io::Error> for Replace {
    fn from(e: io::Error) -> Self {
        Replace::Failed(SaveError::Io(e.kind()))
    }
}

impl From<FsError> for Replace {
    fn from(e: FsError) -> Self {
        Replace::Failed(fs_save_error(e))
    }
}

/// The XXH3-128 of everything `file` holds from its start, read in 1 MiB pieces (fr1 m5: a
/// file swapped for a huge one is never read into memory), and the byte count.
fn hash_streaming(mut file: &File) -> io::Result<(u128, u64)> {
    file.seek(io::SeekFrom::Start(0))?;
    let mut hasher = XxHash3_128::new();
    let mut buf = vec![0u8; HASH_CHUNK_BYTES];
    let mut total = 0u64;
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok((hasher.finish_128(), total)),
            Ok(n) => {
                hasher.write(&buf[..n]);
                total += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// The current stamp of the regular file `name`, read through a no-follow handle.
fn stamp_streaming(dir: &Dir, name: &str, lstat: &Metadata) -> Result<DiskStamp, SaveError> {
    let file = dir.open_read(name, lstat).map_err(fs_save_error)?;
    let (hash, len) = hash_streaming(&file).map_err(|e| SaveError::Io(e.kind()))?;
    let meta = file.metadata().map_err(|e| SaveError::Io(e.kind()))?;
    Ok(stamp_from(&meta, len, hash))
}

/// The atomic save over an existing file: a same-folder temp that takes the original's mode
/// and owner, synced, renamed over `name`.
fn replace(
    dir: &Dir,
    name: &str,
    bytes: &[u8],
    original: &Original<'_>,
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, Replace> {
    let tmp = temp_name_for(name);
    faults::record_temp(&tmp);
    let file = dir.create_temp(&tmp, true)?;
    let written = write_temp(&file, bytes, Some(original));
    drop(file);
    let published = written.and_then(|stamp| {
        recheck(dir, name, Some(original))?;
        echo(&stamp);
        faults::hit(Fault::Rename)?;
        let still = dir.rename_noting_after(&tmp, name, faults::run_after_publish)?;
        dir.sync();
        Ok((stamp, still))
    });
    match published {
        Ok((stamp, true)) => Ok(stamp),
        // fr2 mf2: renamed, so saved; only the folder check after it failed.
        Ok((stamp, false)) => Err(Replace::Failed(SaveError::SavedButPathChanged(stamp))),
        Err(e) => {
            dir.discard_temp(&tmp);
            Err(e)
        }
    }
}

/// A new file: the temp, synced, is published with a hard link, which fails when the name
/// exists, so a file that appeared meanwhile is never replaced (fr1 m4, T3's publish). On a
/// volume without hard links (exFAT, FAT, SMB) the name itself is created with `create_new`,
/// which never replaces one either.
fn publish_new(
    dir: &Dir,
    name: &str,
    bytes: &[u8],
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, SaveError> {
    let tmp = temp_name_for(name);
    faults::record_temp(&tmp);
    let file = dir.create_temp(&tmp, false).map_err(fs_save_error)?;
    let written = write_temp(&file, bytes, None);
    drop(file);
    let linked = match written {
        Ok(stamp) => recheck(dir, name, None)
            .and_then(|()| {
                echo(&stamp);
                faults::run_before_publish();
                faults::hit(Fault::Rename).map_err(|e| SaveError::Io(e.kind()))
            })
            .map(|()| {
                let linked = dir.publish_link_noting_after(&tmp, name, faults::run_after_publish);
                (stamp, linked)
            }),
        Err(Replace::Failed(e)) => Err(e),
        Err(Replace::OwnerNotKept) => Err(SaveError::Io(io::ErrorKind::PermissionDenied)),
    };
    dir.discard_temp(&tmp);
    let (stamp, linked) = linked?;
    match linked {
        Ok(true) => {
            dir.sync();
            Ok(stamp)
        }
        // fr2 mf2: published, so saved; only the folder check after it failed.
        Ok(false) => Err(SaveError::SavedButPathChanged(stamp)),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => Err(appeared(dir, name)),
        // No hard links on this volume (`EPERM`, `ENOTSUP`): the no-replace fallback.
        Err(FsError::Io(_)) => create_in_place(dir, name, bytes),
        Err(e) => Err(fs_save_error(e)),
    }
}

/// What a name that appeared during a new-file save holds now.
fn appeared(dir: &Dir, name: &str) -> SaveError {
    match dir.lstat(name) {
        Ok(m) if m.is_file() && !is_link(&m) => match stamp_streaming(dir, name, &m) {
            Ok(stamp) => SaveError::ChangedOnDisk(stamp),
            Err(e) => e,
        },
        Ok(_) => SaveError::PathChanged,
        Err(e) => fs_save_error(e),
    }
}

/// The no-hard-link fallback: the name created with `create_new` and written; a partial file
/// it created is removed again while the name still is that file (T3's `create_in_place`).
fn create_in_place(dir: &Dir, name: &str, bytes: &[u8]) -> Result<DiskStamp, SaveError> {
    let file = dir.create_new(name).map_err(|e| match e {
        FsError::Io(e) if e.kind() == io::ErrorKind::AlreadyExists => appeared(dir, name),
        e => fs_save_error(e),
    })?;
    let written = (|| -> io::Result<DiskStamp> {
        write_faulted(&file, bytes)?;
        file.sync_all()?;
        Ok(stamp_from(
            &file.metadata()?,
            bytes.len() as u64,
            XxHash3_128::oneshot(bytes),
        ))
    })();
    match written {
        Ok(stamp) => {
            dir.sync();
            Ok(stamp)
        }
        Err(e) => {
            let ours = file.metadata().ok().map(|m| dir::identity(&m));
            drop(file);
            if let (Some(ours), Ok(now)) = (ours, dir.lstat(name))
                && !is_link(&now)
                && dir::identity(&now) == ours
            {
                let _ = dir.remove_file(name);
            }
            Err(SaveError::Io(e.kind()))
        }
    }
}

fn write_temp(
    file: &File,
    bytes: &[u8],
    original: Option<&Original<'_>>,
) -> Result<DiskStamp, Replace> {
    if let Some(o) = original {
        keep_mode(file, o.meta)?;
        if keep_owner(file, o.meta).is_err() {
            return Err(Replace::OwnerNotKept);
        }
    }
    write_faulted(file, bytes)?;
    faults::hit(Fault::Sync)?;
    file.sync_all()?;
    let meta = file.metadata()?;
    Ok(stamp_from(
        &meta,
        bytes.len() as u64,
        XxHash3_128::oneshot(bytes),
    ))
}

/// Right before the publish: the name still holds the file that was checked (or still nothing).
fn recheck(dir: &Dir, name: &str, original: Option<&Original<'_>>) -> Result<(), SaveError> {
    let now = match dir.lstat(name) {
        Ok(m) => Some(m),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(fs_save_error(e)),
    };
    match (original, now) {
        (None, None) => Ok(()),
        (Some(o), Some(m))
            if m.is_file()
                && Fingerprint::of(&m) == Fingerprint::of(o.meta)
                && o.stamp.len == m.len() =>
        {
            Ok(())
        }
        (_, Some(m)) if m.is_file() && !is_link(&m) => {
            Err(SaveError::ChangedOnDisk(stamp_streaming(dir, name, &m)?))
        }
        (Some(_), None) => Err(SaveError::DeletedOnDisk),
        (_, Some(_)) => Err(SaveError::PathChanged),
    }
}

/// Writes over the file through its own handle, so the inode, its other names and its owner
/// stay (fr1 I1, ruling R-T5-1): the handle is re-checked against the stamp right before (m3),
/// the original bytes are kept, and on any error they are written back with the original
/// mtime and verified. [`SaveError::Damaged`] only when that restore fails too.
fn in_place(
    dir: &Dir,
    name: &str,
    file: File,
    original: &Original<'_>,
    bytes: &[u8],
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, SaveError> {
    let io_err = |e: io::Error| SaveError::Io(e.kind());
    if original.stamp.len > WHOLE_MAX_BYTES {
        return Err(SaveError::Io(io::ErrorKind::FileTooLarge));
    }
    faults::run_before_publish();
    // m3: still the file the stamp was taken of, unchanged since.
    let now = file.metadata().map_err(io_err)?;
    let mut kept = Vec::with_capacity(usize::try_from(original.stamp.len).unwrap_or(0));
    (&file).seek(io::SeekFrom::Start(0)).map_err(io_err)?;
    (&file).read_to_end(&mut kept).map_err(io_err)?;
    let unchanged = Fingerprint::of(&now) == Fingerprint::of(original.meta)
        && kept.len() as u64 == original.stamp.len
        && XxHash3_128::oneshot(&kept) == original.stamp.hash;
    if !unchanged {
        return Err(match dir.lstat(name) {
            Ok(m) if m.is_file() && !is_link(&m) => match stamp_streaming(dir, name, &m) {
                Ok(stamp) => SaveError::ChangedOnDisk(stamp),
                Err(e) => e,
            },
            _ => SaveError::PathChanged,
        });
    }
    match write_over(&file, bytes, false) {
        Ok(()) => {
            let meta = file.metadata().map_err(io_err)?;
            let stamp = stamp_from(&meta, bytes.len() as u64, XxHash3_128::oneshot(bytes));
            echo(&stamp);
            // fr2 mf2: written through the file's own handle, so saved; a folder moved since is
            // a warning, never a plain failure.
            faults::run_after_publish();
            if dir.verify().is_err() {
                return Err(SaveError::SavedButPathChanged(stamp));
            }
            Ok(stamp)
        }
        Err(e) => {
            // The content decides; the mtime is put back too where the file system can hold
            // it, so the editor's stamp still matches.
            let restored = write_over(&file, &kept, true).and_then(|()| {
                let _ = file.set_times(FileTimes::new().set_modified(original.stamp.mtime));
                // fr2 mf1: a write-back that may not be durable is not a restore.
                faults::hit(Fault::RestoreSync)?;
                file.sync_all()?;
                let (hash, len) = hash_streaming(&file)?;
                Ok(len == original.stamp.len && hash == original.stamp.hash)
            });
            match restored {
                Ok(true) => Err(SaveError::Io(e.kind())),
                Ok(false) | Err(_) => Err(SaveError::Damaged),
            }
        }
    }
}

/// `bytes` over the whole file through `file`: from the start, then the length cut, then synced.
/// `restoring` marks the write-back of the original (a fault point of its own).
fn write_over(mut file: &File, bytes: &[u8], restoring: bool) -> io::Result<()> {
    file.seek(io::SeekFrom::Start(0))?;
    if restoring {
        faults::hit(Fault::Restore)?;
        file.write_all(bytes)?;
    } else {
        write_faulted(file, bytes)?;
    }
    file.set_len(bytes.len() as u64)?;
    if !restoring {
        faults::hit(Fault::Sync)?;
    }
    file.sync_all()
}

#[cfg(unix)]
fn keep_mode(file: &File, original: &Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(
        original.permissions().mode() & 0o777,
    ))
}

#[cfg(not(unix))]
fn keep_mode(_: &File, _: &Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn keep_owner(file: &File, original: &Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    std::os::unix::fs::fchown(file, Some(original.uid()), Some(original.gid()))
}

#[cfg(not(unix))]
fn keep_owner(_: &File, _: &Metadata) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn hard_links(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.nlink()
}

#[cfg(not(unix))]
fn hard_links(_: &Metadata) -> u64 {
    // fr1 m11: `std` exposes no link count on Windows (`MetadataExt::number_of_links` is
    // unstable, `windows_by_handle`), and this crate has no `windows-sys`; a rename there
    // replaces the name, as Notepad's save does (ADR 0009, a recorded residual).
    1
}

fn stamp_from(meta: &Metadata, len: u64, hash: u128) -> DiskStamp {
    DiskStamp {
        len,
        mtime: meta.modified().unwrap_or(UNIX_EPOCH),
        hash,
    }
}

// ---- the stream ----------------------------------------------------------------------------

/// What tells two versions of a file apart without reading it: size, times and identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    mtime: Option<SystemTime>,
    #[cfg(unix)]
    unix: (u64, u64, i64, i64),
    #[cfg(not(unix))]
    created: Option<SystemTime>,
}

impl Fingerprint {
    fn of(meta: &Metadata) -> Self {
        Fingerprint {
            len: meta.len(),
            mtime: meta.modified().ok(),
            #[cfg(unix)]
            unix: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec())
            },
            #[cfg(not(unix))]
            created: meta.created().ok(),
        }
    }
}

struct FileStream {
    dir: Dir,
    name: String,
    file: File,
    at_open: Fingerprint,
    // ## m4b-cifix-5-windows-identity
    /// The file's Windows identity at its path when opened (`None` off Windows): another file
    /// renamed over the path is caught by it even when its size and times match.
    path_id: Option<FileIdentity>,
    // ## m4b-cifix-5-windows-identity
    decoder: StreamDecoder,
    done: bool,
    // ## m4b-t10-service fix round 1 (named exception; I7): the pass record.
    /// The stream ended on an error (its record is incomplete).
    failed: bool,
    hasher: twox_hash::XxHash3_64,
    blocks: Vec<u64>,
    /// Pass 1's running hashes, when this is pass 2.
    expect: Option<Vec<u64>>,
    /// Fix round 2 (F4): the decoding bound by the record.
    encoding: TextEncoding,
    bom: Option<bool>,
    expect_bom: Option<bool>,
    // ## m4b-t10-service
}

impl FileStream {
    /// Fills `buf` from the handle; fewer bytes than its length means the end.
    fn fill(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut got = 0;
        while got < buf.len() {
            match self.file.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(got)
    }

    /// The handle and the path both still show the file as it was at open.
    fn unchanged(&self) -> bool {
        let handle = self.file.metadata().map(|m| Fingerprint::of(&m));
        let path = self.dir.lstat(&self.name).map(|m| Fingerprint::of(&m));
        matches!((handle, path), (Ok(h), Ok(p)) if h == self.at_open && p == self.at_open)
            // ## m4b-cifix-5-windows-identity
            && path_identity(&self.dir, &self.name) == self.path_id
        // ## m4b-cifix-5-windows-identity
    }
}

impl ScriptChunks for FileStream {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
        while !self.done {
            let n = match self.fill(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    self.done = true;
                    self.failed = true;
                    return Some(Err(ChunkError::Io(e.kind())));
                }
            };
            // Checked after the read: bytes read under a changed stamp are never handed out.
            if !self.unchanged() {
                self.done = true;
                self.failed = true;
                return Some(Err(ChunkError::Changed));
            }
            let last = n < buf.len();
            // ## m4b-t10-service fix round 1 (I7): the running hash of this block, checked
            // against pass 1's before the block's text is handed out.
            {
                use std::hash::Hasher as _;
                self.hasher.write(&buf[..n]);
                let running = self.hasher.finish();
                let at = self.blocks.len();
                self.blocks.push(running);
                if at == 0 {
                    let bom = encoding::bom_of(self.encoding);
                    self.bom = Some(!bom.is_empty() && buf[..n].starts_with(bom));
                }
                let bom_differs = at == 0 && self.expect_bom.is_some_and(|b| Some(b) != self.bom);
                let differs = bom_differs
                    || self.expect.as_ref().is_some_and(|pass1| {
                        pass1.get(at) != Some(&running) || (last && pass1.len() != at + 1)
                    });
                if differs {
                    self.done = true;
                    self.failed = true;
                    return Some(Err(ChunkError::Changed));
                }
            }
            // ## m4b-t10-service
            let Some(text) = self.decoder.feed(&buf[..n], last) else {
                self.done = true;
                return Some(Err(ChunkError::Decode));
            };
            self.done = last;
            if !text.is_empty() {
                return Some(Ok(text));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_stats_counts_each_break_once() {
        let s = line_stats("SELECT 1;\r\nSELECT 2;\n");
        assert_eq!((s.lines, s.ending), (3, LineEnding::Lf));
        assert_eq!(line_stats("a\r\nb\r\n").ending, LineEnding::CrLf);
        assert_eq!(line_stats("a\rb\r").ending, LineEnding::Cr);
        assert_eq!(line_stats("x\u{2028}y").lines, 1);
        assert_eq!(line_stats("").lines, 1);
        assert_eq!(line_stats("ab\ncdef\r\ng").longest, 4);
        // A CRLF tie with a CR is a tie too: LF.
        assert_eq!(line_stats("a\r\nb\rc").ending, LineEnding::Lf);
    }

    #[test]
    fn preview_never_ends_on_a_lone_cr_at_the_cut() {
        assert_eq!(preview_cut("a\nb\r"), 2);
        assert_eq!(preview_cut("a\rb"), 2);
        assert_eq!(preview_cut("abc\r"), 3);
        assert_eq!(preview_cut("abc"), 3);
    }

    // ## m4b-t10-service fix round 1 (named exception; I7)
    /// Pass 2 stops at the first block whose running hash is not pass 1's, before its text is
    /// handed out, even when the file's fingerprint is the same (a tampered record stands for
    /// bytes that differ without a visible metadata change).
    #[test]
    fn second_pass_stops_at_the_first_block_that_differs() {
        let dir = tempfile::tempdir().unwrap();
        let text: String = (0..3 * STREAM_CHUNK_BYTES / 10)
            .map(|i| format!("{:09}\n", i % 1_000_000_000))
            .collect();
        std::fs::write(dir.path().join("big.sql"), &text).unwrap();
        let id = crate::registry::LibraryId::new();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let root = LibraryRoot {
            id,
            path: canonical.clone(),
            canonical,
        };
        let rel = RelPath::parse("big.sql").unwrap();
        let utf8 = TextEncoding::UTF_8;
        let drain = |s: &mut ScriptStream| {
            let mut out = Vec::new();
            while let Some(c) = s.next_chunk() {
                out.push(c);
            }
            out
        };
        let mut pass1 = open_stream_checked(&root, &rel, None, utf8, None).unwrap();
        let chunks1 = drain(&mut pass1);
        let record = pass1.record();
        assert!(record.is_complete());
        assert_eq!(record.blocks(), 3);
        // An identical second pass yields the same chunks.
        let mut pass2 = open_stream_checked(&root, &rel, None, utf8, Some(record.clone())).unwrap();
        assert_eq!(drain(&mut pass2), chunks1);
        // A record that differs at block 1: block 0 is handed out, then `Changed`.
        let mut forged = record;
        forged.tamper(1);
        let mut pass2 = open_stream_checked(&root, &rel, None, utf8, Some(forged)).unwrap();
        let got = drain(&mut pass2);
        assert_eq!(got.len(), 2, "{} chunks", got.len());
        assert!(got[0].is_ok());
        assert_eq!(got[1], Err(ChunkError::Changed));
    }
    // ## m4b-t10-service

    // ## m4b-cifix-2-root-birth
    /// Device and inode alone are not a folder's identity: ext4 gives a freed inode to the
    /// next folder made in the same place (CI 44990057141). The birth time tells them apart,
    /// and a rename keeps it.
    #[test]
    fn folder_identity_tells_a_folder_made_again_from_a_renamed_one() {
        use std::time::{Duration, SystemTime};
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let reused = |born| FolderIdentity {
            dev: 1,
            ino: 7,
            born,
        };
        assert_eq!(reused(Some(t)), reused(Some(t)));
        // m4b-cifix-5: a step every `SystemTime` holds (Windows counts in 100 ns, and
        // `t + 1 ns` is `t` there, CI 45091594060).
        let step = Duration::from_micros(1);
        assert_ne!(t + step, t);
        assert_ne!(reused(Some(t)), reused(Some(t + step)));
        assert_ne!(reused(Some(t)), reused(None));
        // m4b-cifix-5: Windows has a real folder identity too (volume serial and file id).
        #[cfg(any(unix, windows))]
        {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().join("scripts");
            std::fs::create_dir(&root).unwrap();
            let first = folder_identity(&root).unwrap();
            // Fix round (C1): the birth time is part of the identity on Linux only.
            assert_eq!(first.born.is_some(), cfg!(target_os = "linux"));
            // Fix round (C1): a restore that sets the folder's time back (`rsync -a`,
            // `touch -t`) keeps the identity; on macOS it moves the birth time back as well.
            // (A folder opens as a `File` on Unix only.)
            #[cfg(unix)]
            {
                let restored = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
                std::fs::File::open(&root)
                    .unwrap()
                    .set_modified(restored)
                    .unwrap();
            }
            assert_eq!(folder_identity(&root), Some(first));
            let away = d.path().join("away");
            std::fs::rename(&root, &away).unwrap();
            assert_eq!(folder_identity(&away), Some(first));
            std::fs::rename(&away, &root).unwrap();
            assert_eq!(folder_identity(&root), Some(first));
            std::fs::remove_dir(&root).unwrap();
            // Linux stamps a new inode from the coarse clock: wait (on a condition) until a
            // new file is born after the old folder, so the next one is too. (m4b-cifix-5:
            // only where the birth time is part of the identity; on Windows a probe made again
            // within 15 s keeps its first creation time, NTFS tunnelling, so it never passes.)
            if let Some(born) = first.born {
                let probe = d.path().join("probe");
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                loop {
                    std::fs::write(&probe, b"").unwrap();
                    let later = std::fs::metadata(&probe).unwrap().created().unwrap() > born;
                    std::fs::remove_file(&probe).unwrap();
                    if later {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the birth clock stood still"
                    );
                    std::thread::yield_now();
                }
            }
            std::fs::create_dir(&root).unwrap();
            assert_ne!(folder_identity(&root), Some(first));
        }
    }
    // ## m4b-cifix-2-root-birth

    // ## m4b-cifix-4-file-identity
    /// A file's identity round-trips through its text exactly, and a malformed text reads as
    /// none.
    #[test]
    fn file_identity_text_round_trips() {
        use std::time::Duration;
        let at = |secs, nanos| FileIdentity {
            dev: 16_777_232,
            ino: 4_242,
            born: Some(UNIX_EPOCH + Duration::new(secs, nanos)),
        };
        let none = FileIdentity {
            born: None,
            ..at(0, 0)
        };
        let before = FileIdentity {
            born: UNIX_EPOCH.checked_sub(Duration::new(5, 7)),
            ..none
        };
        for id in [
            at(1_800_000_000, 123_456_789),
            at(0, 0),
            at(1, 1),
            none,
            before,
        ] {
            assert_eq!(FileIdentity::parse(&id.to_text()), Some(id), "{id:?}");
        }
        assert_eq!(none.to_text(), "16777232:4242");
        // m4b-cifix-5: Windows holds a `SystemTime` in 100 ns steps, so `UNIX_EPOCH + 5 ns` is
        // the epoch itself there (CI 45091594060). The text is exact for every time the
        // platform holds; the nanosecond digit is asserted where the platform holds it.
        assert_eq!(at(1, 500).to_text(), "16777232:4242:1.000000500");
        if at(1, 5) != at(1, 0) {
            assert_eq!(at(1, 5).to_text(), "16777232:4242:1.000000005");
        }
        assert_eq!(cfg!(windows), at(1, 5) == at(1, 0));
        for bad in [
            "",
            "1",
            "1:",
            "x:2",
            "1:2:",
            "1:2:3",
            "1:2:3.4",
            "1:2:3.0000000001",
            "1:2:3.00000000x",
            "1:2:3.000000000:4",
        ] {
            assert_eq!(FileIdentity::parse(bad), None, "{bad:?}");
        }
    }

    // ## m4b-cifix-5-windows-identity
    /// A 128-bit Windows file id becomes `ino` exactly when it fits (NTFS), and by a hash when
    /// it uses the high half (ReFS), so two such ids that share a half stay apart.
    #[test]
    fn a_file_id_folds_exactly_when_it_fits() {
        use super::windows_id::fold_file_id;
        assert_eq!(fold_file_id(0x0001_0000_0000_002a), 0x0001_0000_0000_002a);
        assert_eq!(fold_file_id(u128::from(u64::MAX)), u64::MAX);
        let high = |h: u64, l: u64| fold_file_id((u128::from(h) << 64) | u128::from(l));
        assert_ne!(high(1, 7), 7);
        assert_ne!(high(1, 7), high(2, 7));
        assert_ne!(high(1, 7), high(1, 8));
        assert_eq!(high(1, 7), high(1, 7));
    }

    /// Review m1: a file id of 0 (WebDAV, some NAS and FUSE shares) is no identity, never a
    /// match; any other id is one, from the low-resolution index or the 128-bit id alike.
    #[test]
    fn a_zero_file_id_is_no_identity() {
        use super::windows_id::identity_from;
        assert_eq!(identity_from(0x1234_abcd, 0), None);
        assert_eq!(identity_from(0, 0), None);
        let id = identity_from(0x1234_abcd, 42).unwrap();
        assert_eq!((id.dev, id.ino, id.born), (0x1234_abcd, 42, None));
        let wide = identity_from(7, (1u128 << 64) | 42).unwrap();
        assert_ne!(wide.ino, 0);
        assert_ne!(wide.ino, 42);
    }
    // ## m4b-cifix-5-windows-identity

    /// A file deleted and written again at its path is another file, even where ext4 hands it
    /// the freed inode (CI 45077623376); a rename keeps the identity.
    #[test]
    fn file_identity_tells_a_file_written_again_from_a_renamed_one() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("orders.db");
        std::fs::write(&file, b"").unwrap();
        let id = |p: &std::path::Path| FileIdentity::of(&std::fs::metadata(p).unwrap());
        let first = id(&file);
        assert_eq!(first.is_some(), cfg!(unix));
        let Some(first) = first else {
            return; // No identity off Unix: the path alone is compared.
        };
        assert_eq!(first.born.is_some(), cfg!(target_os = "linux"));
        let away = d.path().join("away.db");
        std::fs::rename(&file, &away).unwrap();
        assert_eq!(id(&away), Some(first));
        std::fs::rename(&away, &file).unwrap();
        std::fs::remove_file(&file).unwrap();
        // Linux stamps a new inode from the coarse clock: wait (on a condition) until a new
        // file is born after the old one, so the next one is too.
        if let Some(born) = first.born {
            let probe = d.path().join("probe");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                std::fs::write(&probe, b"").unwrap();
                let later = std::fs::metadata(&probe).unwrap().created().unwrap() > born;
                std::fs::remove_file(&probe).unwrap();
                if later {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the birth clock stood still"
                );
                std::thread::yield_now();
            }
        }
        std::fs::write(&file, b"replaced").unwrap();
        assert_ne!(id(&file), Some(first));
    }
    // ## m4b-cifix-4-file-identity
}
// ## m4b-t01a-seams
