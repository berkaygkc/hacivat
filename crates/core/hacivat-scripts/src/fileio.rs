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
//!
//! - **Exact round trip.** [`read`] sniffs a BOM (UTF-8, UTF-16LE/BE), else takes strict
//!   UTF-8, else answers [`ReadError::NeedsEncoding`] (the app asks, then calls [`read_with`]),
//!   and refuses as [`ReadError::Binary`] a BOM-less file with more than 1 % NUL in its first
//!   64 KiB. Nothing is ever decoded with replacement, and line endings are never normalised
//!   (the text holds them as they are; [`FileMeta::line_ending`] is the dominant one, ties
//!   LF). [`read_with`] also re-encodes what it decoded and refuses with [`ReadError::Lossy`]
//!   when that would not give the same bytes back (Shift_JIS's duplicate NEC/IBM rows), so an
//!   unchanged save never rewrites a file. [`save`] encodes in the file's encoding and BOM and
//!   refuses a character the encoding cannot hold ([`SaveError::Unencodable`]), before it
//!   touches the disk.
//! - **Tiers** (UI-EDIT-23): [`tier_for`] by size and line count, plus Reduced for a line over
//!   64 KiB; above 20 MiB the read is a preview of the first 2.5 MiB cut after the last line
//!   break, and its stamp is still the whole file's (hashed streaming, in constant memory).
//! - **Paths** (HV-SCRIPT-03 step 8). Every call walks the [`RelPath`] from the library's
//!   canonical root one folder at a time, each opened as a handle without following a link
//!   and re-checked around every operation (the same no-`unsafe` approach as Task 3's manifest
//!   writer, ruling R-T3-1: on Linux the operations go through `/proc/self/fd/<n>/`, on macOS
//!   the opens use `O_NOFOLLOW_ANY`, on Windows the folder handles are held without
//!   `FILE_SHARE_DELETE`). A link or junction in a folder position, a folder or other non-file
//!   at the end, a component starting with `.` (never listed, C11; it keeps `.git/` and
//!   `.hacivat/` out of reach) and, on Windows, a device or stream name are refused. A link at
//!   the end (an `InsideLink`) is resolved once and followed only to a regular file inside the
//!   root with no hidden component; the save then writes the target and the link stays.
//! - **Save** (HV-SCRIPT-07 step 3): with `expect`, the file's current stamp must equal it
//!   ([`SaveError::ChangedOnDisk`] carries the one on disk; a vanished file is
//!   [`SaveError::DeletedOnDisk`]); the file must be writable as it is (a read-only file is
//!   refused, never replaced). The bytes go into `.<name>.hv-<uuid v4>.tmp` in the same folder
//!   (a dot name, so the scanner never lists it; never the process id, R-T1a-4), created
//!   without following a link, 0600 until it takes the original's permission bits and owner,
//!   written and synced; the stamp is re-checked, the echo is reported
//!   ([`save_with_echo`], step 4), and the temp is renamed over the name relative to the
//!   folder handle (n8), and the folder synced. Any failure removes the temp and leaves the
//!   original as it was. A file with more than one hard link, or one whose owner the temp
//!   cannot take, is written in place instead (write, truncate, sync), so the link and the
//!   owner survive; that write is not atomic (alg/28 §3 item 4).
//! - **Stream** ([`open_stream`]): one handle for the whole run, 64 KiB reads decoded
//!   incrementally (a character cut by a read waits for the next one), and after every read
//!   the handle's and the path's stamps are compared with the ones at open; a change answers
//!   [`ChunkError::Changed`] and nothing read after it is handed out.

use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use twox_hash::XxHash3_128;

use crate::encoding::{self, StreamDecoder};
use crate::pathsafe::RelPath;
use crate::registry::LibraryRoot;
use crate::service::{ChunkError, ScriptChunks};

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
    /// A stub that reads nothing (the service's, until Task 10).
    NotImplemented,
    /// The path failed the re-validation (HV-SCRIPT-03 step 8): a link or junction on the way,
    /// a link out of the library, a folder or other non-file, or a hidden component.
    PathRefused,
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
    Io(io::ErrorKind),
}

/// Why a save failed. The file on disk is unchanged in every case but an I/O error in the
/// middle of an in-place write (a hard-linked file, or one whose owner a new file cannot take).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// A stub that writes nothing (the service's, until Task 10).
    NotImplemented,
    /// The file is not what the editor last read (HV-SCRIPT-08); carries the stamp on disk.
    ChangedOnDisk(DiskStamp),
    /// The path became a link or another kind of file since it was validated.
    PathChanged,
    /// The path is not one a script may have: a hidden component (never listed, C11) or, on
    /// Windows, a device or stream name.
    PathRefused,
    /// `expect` was given, and the file is gone ("Deleted on disk — Save to recreate" saves
    /// with no `expect`).
    DeletedOnDisk,
    /// The text has a character the file's encoding cannot hold.
    Unencodable {
        first_offset: usize,
    },
    Io(io::ErrorKind),
}

impl From<&Decoded> for SaveText {
    /// The decoded text as read, to be saved the way it was read.
    fn from(d: &Decoded) -> Self {
        SaveText {
            text: d.text.clone(),
            meta: d.meta,
        }
    }
}

/// The largest file in the Full tier (UI-EDIT-23 T0).
pub const FULL_MAX_BYTES: u64 = 4 << 20;
/// The most lines in the Full tier.
pub const FULL_MAX_LINES: usize = 100_000;
/// The longest line in the Full tier, in bytes.
pub const FULL_MAX_LINE_BYTES: usize = 64 << 10;
/// The largest file read whole without being asked (UI-EDIT-23 T1).
pub const REDUCED_MAX_BYTES: u64 = 20 << 20;
/// The largest file read whole when asked ([`ReadLimits::whole`], UI-EDIT-23 "Open anyway").
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

/// Reads and decodes `rel` under `root` (UI-EDIT-02 step 1): a BOM names the encoding, else
/// strict UTF-8, else [`ReadError::NeedsEncoding`]. Above 20 MiB (64 MiB with
/// [`ReadLimits::whole`]) it reads a preview.
pub fn read(root: &LibraryRoot, rel: &RelPath, limits: ReadLimits) -> Result<Decoded, ReadError> {
    let opened = open_for_read(root, rel)?;
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
/// [`ReadError::NeedsEncoding`]. The encoding's own BOM, if present, is kept for the save.
pub fn read_with(root: &LibraryRoot, rel: &RelPath, encoding: &str) -> Result<Decoded, ReadError> {
    let enc = encoding::lookup(encoding).ok_or(ReadError::UnknownEncoding)?;
    let opened = open_for_read(root, rel)?;
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
    };
    Ok(decoded(text, meta, stamp, len, false))
}

/// Saves `text` to `rel` under `root`; with `expect`, only if the file still has that stamp.
/// Returns the stamp of what is now on disk (HV-SCRIPT-07 step 3).
pub fn save(
    root: &LibraryRoot,
    rel: &RelPath,
    text: SaveText,
    expect: Option<DiskStamp>,
) -> Result<DiskStamp, SaveError> {
    save_with_echo(root, rel, text, expect, &mut |_| {})
}

/// [`save`], calling `echo` with the stamp of the new file before it replaces the old one
/// (HV-SCRIPT-07 step 4: the service records it in the watcher's echo table, so the save's own
/// events are dropped). An in-place write reports it right after the write.
pub fn save_with_echo(
    root: &LibraryRoot,
    rel: &RelPath,
    text: SaveText,
    expect: Option<DiskStamp>,
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, SaveError> {
    let bytes = encoding::encode_exact(text.meta.encoding, text.meta.bom, &text.text)
        .map_err(|first_offset| SaveError::Unencodable { first_offset })?;
    drop(text);
    match locate(root, rel).map_err(save_error)? {
        Target::Missing { dir, name } => {
            if expect.is_some() {
                return Err(SaveError::DeletedOnDisk);
            }
            match replace(&dir, &name, &bytes, None, echo) {
                Ok(stamp) => Ok(stamp),
                Err(Replace::Failed(e)) => Err(e),
                // Only an existing file has an owner to keep.
                Err(Replace::OwnerNotKept) => Err(SaveError::Io(io::ErrorKind::PermissionDenied)),
            }
        }
        Target::Existing { dir, name, lstat } => {
            let current = stamp_of_file(&dir, &name, &lstat)?;
            if let Some(expected) = expect
                && expected != current
            {
                return Err(SaveError::ChangedOnDisk(current));
            }
            // Writable as it is: a read-only file is refused, never replaced.
            let writer = dir.open_write(&name, &lstat).map_err(save_error)?;
            let original = Original {
                meta: &lstat,
                stamp: current,
            };
            if hard_links(&lstat) > 1 {
                return in_place(&dir, writer, &bytes, echo);
            }
            // Not held across the rename (Windows replaces only a file nobody holds).
            drop(writer);
            match replace(&dir, &name, &bytes, Some(&original), echo) {
                Ok(stamp) => Ok(stamp),
                Err(Replace::Failed(e)) => Err(e),
                Err(Replace::OwnerNotKept) => {
                    let writer = dir.open_write(&name, &lstat).map_err(save_error)?;
                    in_place(&dir, writer, &bytes, echo)
                }
            }
        }
    }
}

/// A chunked UTF-8 reader of `rel` in `encoding` for "Run as script" (HV-SCRIPT-12): the
/// source behind `LibraryService::open_stream` (Task 10 wires it). Its BOM is dropped.
pub fn open_stream(
    root: &LibraryRoot,
    rel: &RelPath,
    encoding: TextEncoding,
) -> Result<Box<dyn ScriptChunks>, ReadError> {
    let decoder = StreamDecoder::new(encoding).ok_or(ReadError::UnknownEncoding)?;
    let opened = open_for_read(root, rel)?;
    Ok(Box::new(FileStream {
        at_open: Fingerprint::of(&opened.meta),
        dir: opened.dir,
        name: opened.name,
        file: opened.file,
        decoder,
        done: false,
    }))
}

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

/// Whether `name` has the shape of [`temp_name_for`] (the watcher's expected transient,
/// HV-SCRIPT-07 step 4).
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with('.')
        && name
            .strip_suffix(TEMP_SUFFIX)
            .and_then(|s| s.rsplit_once(TEMP_MARK))
            .is_some_and(|(_, random)| {
                random.len() == TEMP_RANDOM_LEN && random.bytes().all(|b| b.is_ascii_hexdigit())
            })
}

/// Fault injection for the failure tests (feature `test-support`; never in a product build):
/// the next save on this thread fails at the given step.
#[cfg(any(test, feature = "test-support"))]
pub mod faults {
    use std::cell::{Cell, RefCell};
    use std::io;

    /// A step of the atomic save.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Fault {
        Write,
        Sync,
        Rename,
    }

    thread_local! {
        static NEXT: Cell<Option<Fault>> = const { Cell::new(None) };
        static LAST_TEMP: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// The next save on this thread fails at `f`, once.
    pub fn inject(f: Fault) {
        NEXT.set(Some(f));
    }

    /// The temp name the last save on this thread created.
    pub fn last_temp() -> Option<String> {
        LAST_TEMP.with_borrow(Clone::clone)
    }

    pub(super) fn hit(at: Fault) -> io::Result<()> {
        if NEXT.get() == Some(at) {
            NEXT.set(None);
            return Err(io::Error::other("injected fault"));
        }
        Ok(())
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
    }

    #[inline]
    pub(super) fn hit(at: Fault) -> io::Result<()> {
        let (Fault::Write | Fault::Sync | Fault::Rename) = at;
        Ok(())
    }

    #[inline]
    pub(super) fn record_temp(_: &str) {}
}

use faults::Fault;

// ---- reading -------------------------------------------------------------------------------

/// An open regular file under its folder handle, and its `fstat`.
struct Opened {
    dir: Dir,
    name: String,
    file: File,
    meta: Metadata,
}

fn open_for_read(root: &LibraryRoot, rel: &RelPath) -> Result<Opened, ReadError> {
    match locate(root, rel).map_err(read_error)? {
        Target::Missing { .. } => Err(ReadError::Io(io::ErrorKind::NotFound)),
        Target::Existing { dir, name, lstat } => {
            let file = dir.open_read(&name, &lstat).map_err(read_error)?;
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
    let meta = FileMeta {
        encoding: enc,
        bom,
        line_ending: line_stats(&text).ending,
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

/// The current stamp of the regular file `name`, read through a no-follow handle.
fn stamp_of_file(dir: &Dir, name: &str, lstat: &Metadata) -> Result<DiskStamp, SaveError> {
    let file = dir.open_read(name, lstat).map_err(save_error)?;
    let mut bytes = Vec::new();
    (&file)
        .read_to_end(&mut bytes)
        .map_err(|e| SaveError::Io(e.kind()))?;
    let meta = file.metadata().map_err(|e| SaveError::Io(e.kind()))?;
    Ok(stamp_from(
        &meta,
        bytes.len() as u64,
        XxHash3_128::oneshot(&bytes),
    ))
}

/// The atomic save: a same-folder temp, synced, renamed over `name`.
fn replace(
    dir: &Dir,
    name: &str,
    bytes: &[u8],
    original: Option<&Original<'_>>,
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, Replace> {
    let tmp = temp_name_for(name);
    faults::record_temp(&tmp);
    let file = dir
        .create_new(&tmp, original.is_some())
        .map_err(save_error)?;
    let written = write_temp(&file, bytes, original);
    drop(file);
    let published = written.and_then(|stamp| {
        recheck(dir, name, original)?;
        echo(&stamp);
        faults::hit(Fault::Rename)?;
        dir.rename(&tmp, name).map_err(save_error)?;
        dir.sync();
        Ok(stamp)
    });
    if published.is_err() {
        let _ = dir.remove_file(&tmp);
    }
    published
}

fn write_temp(
    mut file: &File,
    bytes: &[u8],
    original: Option<&Original<'_>>,
) -> Result<DiskStamp, Replace> {
    if let Some(o) = original {
        keep_mode(file, o.meta)?;
        if keep_owner(file, o.meta).is_err() {
            return Err(Replace::OwnerNotKept);
        }
    }
    faults::hit(Fault::Write)?;
    file.write_all(bytes)?;
    faults::hit(Fault::Sync)?;
    file.sync_all()?;
    let meta = file.metadata()?;
    Ok(stamp_from(
        &meta,
        bytes.len() as u64,
        XxHash3_128::oneshot(bytes),
    ))
}

/// Right before the rename: the name still holds the file that was checked (or still nothing).
fn recheck(dir: &Dir, name: &str, original: Option<&Original<'_>>) -> Result<(), SaveError> {
    let now = match dir.lstat(name) {
        Ok(m) => Some(m),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(save_error(e)),
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
            Err(SaveError::ChangedOnDisk(stamp_of_file(dir, name, &m)?))
        }
        (Some(_), None) => Err(SaveError::DeletedOnDisk),
        (_, Some(_)) => Err(SaveError::PathChanged),
    }
}

/// Writes over the file through its own handle: the inode, its other names and its owner stay.
fn in_place(
    dir: &Dir,
    mut file: File,
    bytes: &[u8],
    echo: &mut dyn FnMut(&DiskStamp),
) -> Result<DiskStamp, SaveError> {
    let io_err = |e: io::Error| SaveError::Io(e.kind());
    faults::hit(Fault::Write).map_err(io_err)?;
    file.seek(io::SeekFrom::Start(0)).map_err(io_err)?;
    file.write_all(bytes).map_err(io_err)?;
    file.set_len(bytes.len() as u64).map_err(io_err)?;
    faults::hit(Fault::Sync).map_err(io_err)?;
    file.sync_all().map_err(io_err)?;
    let meta = file.metadata().map_err(io_err)?;
    let stamp = stamp_from(&meta, bytes.len() as u64, XxHash3_128::oneshot(bytes));
    echo(&stamp);
    dir.verify().map_err(save_error)?;
    Ok(stamp)
}

#[cfg(unix)]
fn keep_mode(file: &File, original: &Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(
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
    // `std` exposes no link count on Windows (`number_of_links` is unstable); a rename there
    // replaces the name, as Notepad's save does.
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
    decoder: StreamDecoder,
    done: bool,
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
                    return Some(Err(ChunkError::Io(e.kind())));
                }
            };
            // Checked after the read: bytes read under a changed stamp are never handed out.
            if !self.unchanged() {
                self.done = true;
                return Some(Err(ChunkError::Changed));
            }
            let last = n < buf.len();
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

// ---- paths and folder handles ----------------------------------------------------------------

/// Where a path ends: an existing regular file, or nothing yet.
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

/// Walks `rel` from the canonical root; a link at the end is followed once, only to a regular
/// file inside the root.
fn locate(root: &LibraryRoot, rel: &RelPath) -> Result<Target, FsError> {
    check_rel(rel)?;
    let (dir, name, lstat) = walk(root, rel)?;
    match lstat {
        None => Ok(Target::Missing { dir, name }),
        Some(l) if is_link(&l) => {
            let target = resolve_inside(root, &dir, &name)?;
            if check_rel(&target).is_err() {
                return Err(FsError::Outside);
            }
            match walk(root, &target)? {
                (dir, name, Some(l)) if l.is_file() && !is_link(&l) => Ok(Target::Existing {
                    dir,
                    name,
                    lstat: l,
                }),
                _ => Err(FsError::Outside),
            }
        }
        Some(l) if l.is_file() => Ok(Target::Existing {
            dir,
            name,
            lstat: l,
        }),
        Some(_) => Err(FsError::NotFile),
    }
}

fn walk(root: &LibraryRoot, rel: &RelPath) -> Result<(Dir, String, Option<Metadata>), FsError> {
    let mut dir = Dir::open_root(&root.canonical)?;
    let parts: Vec<&str> = rel.components().collect();
    let Some((last, folders)) = parts.split_last() else {
        return Err(FsError::Refused);
    };
    for folder in folders {
        dir = dir.subdir(folder)?;
    }
    let lstat = match dir.lstat(last) {
        Ok(m) => Some(m),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    Ok((dir, (*last).to_owned(), lstat))
}

/// The library-relative path a link at `name` resolves to, if it stays inside the root.
fn resolve_inside(root: &LibraryRoot, dir: &Dir, name: &str) -> Result<RelPath, FsError> {
    let real = fs::canonicalize(dir.child(name)).map_err(|_| FsError::Outside)?;
    let inside = real
        .strip_prefix(&root.canonical)
        .map_err(|_| FsError::Outside)?;
    let mut parts = Vec::new();
    for c in inside.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str().ok_or(FsError::Outside)?),
            _ => return Err(FsError::Outside),
        }
    }
    RelPath::parse(&parts.join("/")).map_err(|_| FsError::Outside)
}

/// The names a script path may hold here: never a hidden component (C11), and on Windows no
/// device name, stream, reserved character or trailing dot or space.
fn check_rel(rel: &RelPath) -> Result<(), FsError> {
    for c in rel.components() {
        if c.starts_with('.') || !portable(c) {
            return Err(FsError::Refused);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn portable(name: &str) -> bool {
    const DEVICES: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    !name.ends_with(['.', ' '])
        && !name.contains([':', '<', '>', '"', '|', '?', '*'])
        && !DEVICES.iter().any(|d| d.eq_ignore_ascii_case(stem))
}

#[cfg(not(windows))]
fn portable(_: &str) -> bool {
    true
}

/// What went wrong under a [`Dir`].
#[derive(Debug)]
enum FsError {
    /// A name a script path may not hold ([`check_rel`]).
    Refused,
    /// A link (or junction) where a folder or file was expected.
    Link,
    NotDir,
    NotFile,
    /// A link that leaves the root, dangles, loops, or ends on something other than a script.
    Outside,
    /// The path no longer names the folder that was opened.
    Changed,
    Io(io::Error),
}

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> Self {
        #[cfg(unix)]
        {
            match e.raw_os_error() {
                Some(libc::ELOOP) => return FsError::Link,
                Some(libc::ENOTDIR) => return FsError::NotDir,
                _ => {}
            }
        }
        FsError::Io(e)
    }
}

fn read_error(e: FsError) -> ReadError {
    match e {
        FsError::Io(e) => ReadError::Io(e.kind()),
        FsError::Refused
        | FsError::Link
        | FsError::NotDir
        | FsError::NotFile
        | FsError::Outside
        | FsError::Changed => ReadError::PathRefused,
    }
}

fn save_error(e: FsError) -> SaveError {
    match e {
        FsError::Io(e) => SaveError::Io(e.kind()),
        FsError::Refused => SaveError::PathRefused,
        FsError::Link
        | FsError::NotDir
        | FsError::NotFile
        | FsError::Outside
        | FsError::Changed => SaveError::PathChanged,
    }
}

fn is_link(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// A file's identity: device and inode on Unix. Windows has no stable equivalent in `std`; the
/// folder handles there are held without `FILE_SHARE_DELETE`, so a folder cannot change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Identity(u64, u64);

#[cfg(unix)]
fn identity(meta: &Metadata) -> Identity {
    use std::os::unix::fs::MetadataExt;
    Identity(meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity(_: &Metadata) -> Identity {
    Identity(0, 0)
}

/// A folder opened without following a link, with the folders from the root down to it and
/// their identities, re-checked around every operation (Task 3's approach, R-T3-1; kept local
/// until the controller reconciles the two copies).
#[derive(Debug)]
struct Dir {
    path: PathBuf,
    /// Held for the whole operation (on Windows without `FILE_SHARE_DELETE`).
    handle: File,
    chain: Vec<(PathBuf, Identity)>,
    /// Linux: `/proc/self/fd/<n>`, the handle as a path prefix.
    #[cfg(target_os = "linux")]
    proc: Option<PathBuf>,
}

impl Dir {
    /// The library root by its canonical path, never canonicalized again here: a root that
    /// became a link is refused, not followed.
    fn open_root(canonical: &Path) -> Result<Dir, FsError> {
        if !canonical.is_absolute() {
            return Err(FsError::Changed);
        }
        let lstat = fs::symlink_metadata(canonical)?;
        Dir::open(canonical.to_path_buf(), canonical, &lstat, Vec::new())
    }

    fn open(
        path: PathBuf,
        via: &Path,
        lstat: &Metadata,
        mut chain: Vec<(PathBuf, Identity)>,
    ) -> Result<Dir, FsError> {
        if is_link(lstat) {
            return Err(FsError::Link);
        }
        if !lstat.is_dir() {
            return Err(FsError::NotDir);
        }
        let handle = open_dir_handle(via)?;
        let meta = handle.metadata()?;
        if is_link(&meta) {
            return Err(FsError::Link);
        }
        if !meta.is_dir() {
            return Err(FsError::NotDir);
        }
        if identity(&meta) != identity(lstat) {
            return Err(FsError::Changed);
        }
        chain.push((path.clone(), identity(&meta)));
        #[cfg(target_os = "linux")]
        let proc = {
            use std::os::fd::AsRawFd;
            let p = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
            fs::metadata(&p)
                .ok()
                .filter(|m| identity(m) == identity(&meta))
                .map(|_| p)
        };
        Ok(Dir {
            path,
            handle,
            chain,
            #[cfg(target_os = "linux")]
            proc,
        })
    }

    /// The path an operation on `name` uses.
    fn child(&self, name: &str) -> PathBuf {
        #[cfg(target_os = "linux")]
        if let Some(p) = &self.proc {
            return p.join(name);
        }
        self.path.join(name)
    }

    /// Every folder from the root down still is the one that was opened.
    fn verify(&self) -> Result<(), FsError> {
        for (path, id) in &self.chain {
            let m = fs::symlink_metadata(path).map_err(|_| FsError::Changed)?;
            if is_link(&m) || !m.is_dir() || identity(&m) != *id {
                return Err(FsError::Changed);
            }
        }
        Ok(())
    }

    /// Runs `op` between two [`Dir::verify`]s.
    fn checked<T>(&self, op: impl FnOnce() -> io::Result<T>) -> Result<T, FsError> {
        self.verify()?;
        let out = op()?;
        self.verify()?;
        Ok(out)
    }

    fn lstat(&self, name: &str) -> Result<Metadata, FsError> {
        self.checked(|| fs::symlink_metadata(self.child(name)))
    }

    /// The folder `name` under this one; a link or a non-folder is refused.
    fn subdir(&self, name: &str) -> Result<Dir, FsError> {
        let lstat = self.lstat(name)?;
        let dir = Dir::open(
            self.path.join(name),
            &self.child(name),
            &lstat,
            self.chain.clone(),
        )?;
        self.verify()?;
        Ok(dir)
    }

    /// `name` opened with `opts` plus the no-follow flags (and, on Unix, `O_NONBLOCK`, so a
    /// FIFO swapped in never blocks), checked to be the regular file `lstat` saw.
    fn open_file(
        &self,
        name: &str,
        opts: &mut OpenOptions,
        lstat: &Metadata,
    ) -> Result<File, FsError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(no_follow_flags() | libc::O_NONBLOCK);
        }
        #[cfg(not(unix))]
        no_follow(opts);
        let f = self.checked(|| opts.open(self.child(name)))?;
        let meta = f.metadata()?;
        if is_link(&meta) {
            return Err(FsError::Link);
        }
        if !meta.is_file() {
            return Err(FsError::NotFile);
        }
        if identity(&meta) != identity(lstat) {
            return Err(FsError::Changed);
        }
        Ok(f)
    }

    fn open_read(&self, name: &str, lstat: &Metadata) -> Result<File, FsError> {
        self.open_file(name, OpenOptions::new().read(true), lstat)
    }

    /// For writing, without truncating or creating: the writable check and the in-place write.
    fn open_write(&self, name: &str, lstat: &Metadata) -> Result<File, FsError> {
        self.open_file(name, OpenOptions::new().write(true), lstat)
    }

    /// A new file `name` (never an existing one, never through a link): 0600 when `private`,
    /// else the umask's mode.
    fn create_new(&self, name: &str, private: bool) -> Result<File, FsError> {
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        no_follow(&mut opts);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        self.checked(|| opts.open(self.child(name)))
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        self.checked(|| rename_retrying(&self.child(from), &self.child(to)))
    }

    fn remove_file(&self, name: &str) -> Result<(), FsError> {
        self.checked(|| fs::remove_file(self.child(name)))
    }

    /// Flushes the folder's entries (Unix; best effort: the file is already in place).
    fn sync(&self) {
        #[cfg(unix)]
        let _ = self.handle.sync_all();
        #[cfg(not(unix))]
        let _ = &self.handle;
    }
}

/// Windows: a rename over a file another program holds open can fail with a sharing violation
/// for a moment (an indexer, a virus scanner); three retries with backoff (alg/28 edge case).
#[cfg(windows)]
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    let mut wait = std::time::Duration::from_millis(50);
    for _ in 0..3 {
        match fs::rename(from, to) {
            // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32 | 33)) => {
                std::thread::sleep(wait);
                wait *= 2;
            }
            other => return other,
        }
    }
    fs::rename(from, to)
}

#[cfg(not(windows))]
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(unix)]
fn no_follow_flags() -> i32 {
    // No link in any component, the last one included (macOS 11+); it implies `O_NOFOLLOW`.
    #[cfg(target_vendor = "apple")]
    {
        libc::O_NOFOLLOW_ANY
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        libc::O_NOFOLLOW
    }
}

#[cfg(unix)]
fn no_follow(opts: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    opts.custom_flags(no_follow_flags());
}

#[cfg(windows)]
fn no_follow(opts: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    opts.custom_flags(win::FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn no_follow(_: &mut OpenOptions) {}

#[cfg(unix)]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | no_follow_flags())
        .open(path)
}

#[cfg(windows)]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(win::FILE_SHARE_READ | win::FILE_SHARE_WRITE)
        .custom_flags(win::FILE_FLAG_BACKUP_SEMANTICS | win::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_dir_handle(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Win32 constants (`winnt.h`, `winbase.h`), written as shifts.
#[cfg(windows)]
mod win {
    pub const FILE_SHARE_READ: u32 = 1;
    pub const FILE_SHARE_WRITE: u32 = 1 << 1;
    pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 1 << 21;
    pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 1 << 25;
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
}
// ## m4b-t01a-seams
