// ## m4b-t01a-seams
//! `.hacivat/workspace.toml`, untrusted input with a fixed schema (HV-SCRIPT-01 steps 3–4;
//! R-M4b-8, -21, -42).
//!
//! ## m4b-t01a-seams
//! Task 1a: the read seams as fail-closed stubs.
//!
//! ## m4b-t03-registry
//! Task 3: the reader, the typed writer and the alias edit.
//!
//! - **Reading** ([`read`], [`read_bytes`]) never panics and never binds more than the schema
//!   says: `format` (integer, 1), `database` (a [`LogicalDb`]) and `aliases` (at most
//!   [`MAX_ALIASES`] of them). A link at `.hacivat` or at the file, a file that is not regular
//!   (a FIFO is never opened for reading), one over [`MANIFEST_MAX_BYTES`] (checked before it is
//!   read, and again while reading), one that is not UTF-8 or not TOML, or a newer `format` is
//!   ignored with a [`Diag`]; the library then binds by its folder name
//!   ([`ManifestRead::logical`]). Unknown keys are ignored and named (escaped, cut, at most
//!   [`MAX_KEY_DIAGS`]); no value is ever kept in a diagnostic.
//! - **Writing** ([`write_new`]) writes exactly a [`ManifestV1`], whose type has no field but
//!   `format`, `database` and `aliases` (L14: no credential can reach a library). It never
//!   replaces a file: the text goes into a same-folder temp file (`create_new`, a random name,
//!   synced) that is published with a hard link (`linkat`, which fails when the name exists),
//!   and on a volume without hard links (exFAT, FAT, SMB) the file itself is created with
//!   `create_new`, which never replaces one either. Modes are left to the umask (R-M4b-42), so
//!   teammates can read the file; never `fsio::atomic_write`.
//! - **Links (I4, n5).** Every operation runs under a directory handle opened without
//!   following a link, and re-checks before and after that the path still names that folder
//!   (device and inode on Unix). A `.hacivat` that is a link (a Windows junction included) or
//!   not a folder is refused and nothing is created where it points. On Linux the operations
//!   go through `/proc/self/fd/<n>/`, so they are relative to the handle itself, and without
//!   procfs every write refuses (`Unsupported`, fix round 1); on macOS the
//!   opens also refuse a link in any component (`O_NOFOLLOW_ANY`); on Windows the handles are
//!   held without `FILE_SHARE_DELETE`, so neither folder can be renamed or replaced while the
//!   write runs. `std` has no `openat`, and this crate takes no `unsafe` (ADR 0009
//!   `m4b-t03`).
//! - **The alias edit** ([`add_alias`]) goes through `toml_edit`, so a teammate's comments,
//!   layout, dominant line ending and BOM stay, and replaces the file with a same-folder temp file and a rename, which never
//!   writes through a hard link; the file's mode is kept.

use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::path::Path;

use serde::Serialize;

// ## m4b-t05-fileio fix round 1 (review m12): one shared copy of T3's folder handles.
use crate::fileio::dir::{Dir, FsError, identity, is_link};

use crate::names::{LogicalDb, NameError, escape_untrusted};

/// The largest manifest that is read (R-M4b-14).
pub const MANIFEST_MAX_BYTES: usize = 64 * 1024;
/// The most aliases a manifest binds (R-M4b-21).
pub const MAX_ALIASES: usize = 32;
/// The manifest format this build reads and writes.
pub const FORMAT: u32 = 1;
/// The most unknown keys named one by one; the rest are counted
/// ([`Diag::MoreUnknownKeys`]).
pub const MAX_KEY_DIAGS: usize = 32;
/// The deepest array or inline-table nesting that is parsed. The schema needs 1 (`aliases`).
pub(crate) const MAX_NESTING: usize = 16;
/// How much of an unknown key a diagnostic keeps, in characters.
const KEY_SHOWN_CHARS: usize = 64;

const HACIVAT: &str = ".hacivat";
const MANIFEST: &str = "workspace.toml";
/// The UTF-8 byte-order mark.
const BOM: &[u8] = "\u{FEFF}".as_bytes();

/// What reading a library's manifest found.
#[derive(Debug, Clone, Default)]
pub struct ManifestRead {
    /// The `database` key, when the manifest was read and the name passed the rules.
    pub db: Option<LogicalDb>,
    /// Valid aliases, deduplicated (NFC, ASCII case-insensitive) and without the database.
    pub aliases: Vec<LogicalDb>,
    pub diags: Vec<Diag>,
}

impl ManifestRead {
    /// The logical database the library binds: `database`, else the folder name (R-M4b-7),
    /// or nothing when the folder name is not a valid name either.
    pub fn logical(&self, folder: &str) -> Option<LogicalDb> {
        self.db.clone().or_else(|| LogicalDb::parse(folder).ok())
    }
}

/// A manifest diagnostic (shown escaped; never a reason to run anything). Holds key names,
/// never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diag {
    /// `.hacivat` or `workspace.toml` is a link (or a junction); it is never followed.
    LinkIgnored,
    /// `.hacivat` is not a folder.
    NotAFolder,
    /// `workspace.toml` is not a regular file (a folder, a FIFO, a device).
    NotAFile,
    /// The folder changed while it was read (a folder replaced under the handle).
    Changed,
    Unreadable(io::ErrorKind),
    /// Over [`MANIFEST_MAX_BYTES`]; not parsed.
    TooLarge,
    NotUtf8,
    /// Arrays or inline tables nested deeper than any manifest needs; not parsed (the parser
    /// recurses, so a hostile file could exhaust a thread's stack).
    TooDeep,
    InvalidToml,
    /// No `format` key: the file is not read.
    MissingFormat,
    /// `format` is not a positive integer.
    BadFormat,
    /// Written by a newer Hacivat: bind by folder name.
    NewerFormat(i64),
    /// An unknown key, escaped and cut (its value is never kept).
    UnknownKey(String),
    /// How many more unknown keys there were.
    MoreUnknownKeys(usize),
    WrongType(Field),
    BadName(Field, NameError),
    /// More than [`MAX_ALIASES`]; the first ones are kept.
    TooManyAliases,
}

/// The manifest field a diagnostic is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Database,
    Aliases,
    /// The alias at this index of the array.
    Alias(usize),
}

/// The manifest Hacivat writes: exactly these three fields, and no other can be added without
/// a new type (L14: no profile, host, user, keyring slot or password can reach a library).
#[derive(Debug, Clone, Serialize)]
pub struct ManifestV1 {
    pub format: u32,
    pub database: LogicalDb,
    pub aliases: Vec<LogicalDb>,
}

impl ManifestV1 {
    /// A format-1 manifest.
    pub fn new(database: LogicalDb, aliases: Vec<LogicalDb>) -> Self {
        ManifestV1 {
            format: FORMAT,
            database,
            aliases,
        }
    }

    /// The exact text [`write_new`] writes (the attach dialog shows it, R-M4b-8).
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The text to write, after the writer's own checks.
    fn checked_text(&self) -> Result<String, WriteError> {
        if self.format != FORMAT {
            return Err(WriteError::Format);
        }
        if self.aliases.len() > MAX_ALIASES {
            return Err(WriteError::TooManyAliases);
        }
        let text = self.to_toml();
        if text.is_empty() {
            return Err(WriteError::Format);
        }
        Ok(text)
    }
}

/// Why a manifest write was refused. Nothing was written into the library, except that
/// `.hacivat/` may have been created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// `workspace.toml` (or a link of that name) already exists: read it instead.
    Exists,
    /// `.hacivat` is a link, a junction or not a folder.
    NotAFolder,
    /// A folder was replaced while the write ran.
    Changed,
    /// The manifest is not one this build writes (`format` ≠ 1).
    Format,
    TooManyAliases,
    /// No manifest to add an alias to (writing one needs the user to see it first, R-M4b-8).
    NoManifest,
    /// The manifest could not be read or understood, so it is not rewritten.
    Unreadable(Diag),
    Io(io::ErrorKind),
}

/// Reads `<library_dir>/.hacivat/workspace.toml`.
pub fn read(library_dir: &Path) -> ManifestRead {
    let folder = library_dir
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    let found = Dir::open_root(library_dir)
        .and_then(|root| root.subdir(HACIVAT))
        .and_then(|h| read_manifest_file(&h));
    match found {
        Ok((bytes, _)) => read_bytes(&bytes, folder),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => ManifestRead::default(),
        Err(e) => ManifestRead {
            diags: vec![read_diag(e)],
            ..ManifestRead::default()
        },
    }
}

/// Reads a manifest's bytes for a library folder named `folder` (the fuzz target's entry,
/// `fuzz/scripts/fuzz_targets/workspace_toml.rs`; [`read`] calls it after its `lstat` checks).
/// `folder` is the name the library binds when there is no `database`, so an alias equal to it
/// is not repeated.
pub fn read_bytes(bytes: &[u8], folder: &str) -> ManifestRead {
    let mut out = ManifestRead::default();
    if bytes.len() > MANIFEST_MAX_BYTES {
        out.diags.push(Diag::TooLarge);
        return out;
    }
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(bytes) else {
        out.diags.push(Diag::NotUtf8);
        return out;
    };
    if nesting_depth(text) > MAX_NESTING {
        out.diags.push(Diag::TooDeep);
        return out;
    }
    let Ok(table) = text.parse::<toml::Table>() else {
        out.diags.push(Diag::InvalidToml);
        return out;
    };
    match table.get("format") {
        None => {
            out.diags.push(Diag::MissingFormat);
            return out;
        }
        Some(toml::Value::Integer(1)) => {}
        Some(toml::Value::Integer(n)) if *n > 1 => {
            out.diags.push(Diag::NewerFormat(*n));
            return out;
        }
        Some(_) => {
            out.diags.push(Diag::BadFormat);
            return out;
        }
    }

    let unknown: Vec<&String> = table
        .keys()
        .filter(|k| !matches!(k.as_str(), "format" | "database" | "aliases"))
        .collect();
    for key in unknown.iter().take(MAX_KEY_DIAGS) {
        out.diags
            .push(Diag::UnknownKey(escape_untrusted(key, KEY_SHOWN_CHARS)));
    }
    if unknown.len() > MAX_KEY_DIAGS {
        out.diags
            .push(Diag::MoreUnknownKeys(unknown.len() - MAX_KEY_DIAGS));
    }

    match table.get("database") {
        None => {}
        Some(toml::Value::String(s)) => match LogicalDb::parse(s) {
            Ok(name) => out.db = Some(name),
            Err(e) => out.diags.push(Diag::BadName(Field::Database, e)),
        },
        Some(_) => out.diags.push(Diag::WrongType(Field::Database)),
    }

    let bound = out.logical(folder);
    match table.get("aliases") {
        None => {}
        Some(toml::Value::Array(items)) => {
            let mut too_many = false;
            for (i, item) in items.iter().enumerate() {
                let toml::Value::String(s) = item else {
                    out.diags.push(Diag::WrongType(Field::Alias(i)));
                    continue;
                };
                let alias = match LogicalDb::parse(s) {
                    Ok(alias) => alias,
                    Err(e) => {
                        out.diags.push(Diag::BadName(Field::Alias(i), e));
                        continue;
                    }
                };
                let repeated = bound.as_ref().is_some_and(|b| b.same_db(&alias))
                    || out.aliases.iter().any(|a| a.same_db(&alias));
                if repeated {
                    continue;
                }
                if out.aliases.len() == MAX_ALIASES {
                    too_many = true;
                    continue;
                }
                out.aliases.push(alias);
            }
            if too_many {
                out.diags.push(Diag::TooManyAliases);
            }
        }
        Some(_) => out.diags.push(Diag::WrongType(Field::Aliases)),
    }
    out
}

/// Writes `<root>/.hacivat/workspace.toml` as `manifest`, only if it does not exist
/// (R-M4b-8): creates `.hacivat/` with default permissions when absent, refuses a `.hacivat`
/// that is a link or not a folder, and never replaces a file.
pub fn write_new(root: &Path, manifest: &ManifestV1) -> Result<(), WriteError> {
    write_new_with(root, manifest, true)
}

/// [`write_new`] on a volume whose hard links fail (exFAT, FAT, SMB answer `ENOTSUP`/`EPERM`):
/// the link attempt is answered `Unsupported`, so the real fallback runs. For tests.
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn write_new_without_hard_links(root: &Path, manifest: &ManifestV1) -> Result<(), WriteError> {
    write_new_with(root, manifest, false)
}

fn write_new_with(root: &Path, manifest: &ManifestV1, hard_links: bool) -> Result<(), WriteError> {
    let text = manifest.checked_text()?;
    let root = Dir::open_root(root).map_err(write_error)?;
    let h = hacivat_creating(&root).map_err(write_error)?;
    h.sweep_stale_temps(&is_our_temp);
    match h.lstat(MANIFEST) {
        Ok(_) => return Err(WriteError::Exists),
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(write_error(e)),
    }
    let (tmp, mut f) = create_temp(&h).map_err(write_error)?;
    let written = write_synced(&mut f, text.as_bytes());
    drop(f);
    let linked = written.map_err(FsError::Io).and_then(|()| {
        if hard_links {
            h.hard_link(&tmp, MANIFEST)
        } else {
            Err(FsError::Io(io::ErrorKind::Unsupported.into()))
        }
    });
    h.discard_temp(&tmp);
    match linked {
        Ok(()) => {}
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(WriteError::Exists);
        }
        // No hard links on this volume (or the temp write failed, and fails again here).
        Err(FsError::Io(_)) => create_in_place(&h, text.as_bytes())?,
        Err(e) => return Err(write_error(e)),
    }
    h.sync();
    Ok(())
}

/// Creates the manifest itself with `create_new` (never replacing a file) and writes it; a
/// partial file it created is removed again.
///
/// ## m4b-t03-registry fix round 1 (review m3): the partial file is unlinked only while the
/// name still is the file this call created (same device and inode), so a `.hacivat` swapped
/// for a link in that window never loses the other folder's `workspace.toml`.
fn create_in_place(h: &Dir, bytes: &[u8]) -> Result<(), WriteError> {
    let mut f = h.create_new(MANIFEST).map_err(|e| match e {
        FsError::Io(e) if e.kind() == io::ErrorKind::AlreadyExists => WriteError::Exists,
        e => write_error(e),
    })?;
    if let Err(e) = write_synced(&mut f, bytes) {
        let ours = f.metadata().ok().map(|m| identity(&m));
        drop(f);
        if let (Some(ours), Ok(now)) = (ours, h.lstat(MANIFEST))
            && !is_link(&now)
            && identity(&now) == ours
        {
            let _ = h.remove_file(MANIFEST);
        }
        return Err(WriteError::Io(e.kind()));
    }
    Ok(())
}

/// The deepest nesting of `[`/`{` tokens in TOML text, as the parser will see them.
///
/// ## m4b-t03-registry fix round 1 (review m1)
/// A token-for-token copy of the boundaries of the pinned `toml_parser` 1.1.5 lexer
/// (`src/lexer/mod.rs`, which `toml` and `toml_edit` both use), so that no string, comment or
/// atom form hides a bracket the parser then recurses on:
/// - a comment runs from `#` to the next CR or LF (not included);
/// - a one-line basic string `"…"` ends at its closing `"` or **before** an LF; a backslash
///   takes the next byte with it only when that byte is `\` or `"`, so `"\` + LF ends at the LF;
/// - a one-line literal string `'…'` ends at its closing `'` or before an LF;
/// - a multi-line basic string `"""…"""` ends after the first `"""` that no backslash takes
///   (plus up to two more `"`), a multi-line literal `'''…'''` after the first `'''` (plus up to
///   two more `'`); unclosed, either runs to the end;
/// - anything else that is not punctuation or whitespace is an atom up to the next of
///   `. = , [ ] { }`, space, tab, `#`, CR, LF: a quote inside an atom starts no string.
///
/// Table headers count too (`[[a]]` is 2). Unbalanced text is fine: the parser refuses it. A
/// bump of `toml_parser` must re-check these rules (the `deeply_nested…` test pins them).
pub(crate) fn nesting_depth(text: &str) -> usize {
    const ATOM_END: &[u8] = b".=,[]{} \t#\r\n";
    // Re-review n1: the lexer skips one leading BOM (`Lexer::new`), so this does too; a
    // second BOM is an atom for both.
    let b = text.as_bytes();
    let b = b.strip_prefix(BOM).unwrap_or(b);
    let (mut i, mut depth, mut max) = (0, 0usize, 0usize);
    while i < b.len() {
        i = match b[i] {
            b'[' | b'{' => {
                depth += 1;
                max = max.max(depth);
                i + 1
            }
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                i + 1
            }
            b'.' | b'=' | b',' | b' ' | b'\t' | b'\r' | b'\n' => i + 1,
            b'#' => find_any(b, i, b"\r\n").unwrap_or(b.len()),
            b'\'' if b[i..].starts_with(b"'''") => multi_line_end(b, i + 3, b'\''),
            b'\'' => match find_any(b, i + 1, b"'\n") {
                Some(j) if b[j] == b'\'' => j + 1,
                Some(j) => j,
                None => b.len(),
            },
            b'"' if b[i..].starts_with(b"\"\"\"") => multi_line_end(b, i + 3, b'"'),
            b'"' => basic_end(b, i + 1),
            _ => find_any(b, i, ATOM_END).unwrap_or(b.len()),
        };
    }
    max
}

/// The first index at or after `from` whose byte is in `set`.
fn find_any(b: &[u8], from: usize, set: &[u8]) -> Option<usize> {
    b[from..]
        .iter()
        .position(|c| set.contains(c))
        .map(|p| from + p)
}

/// A backslash in a basic string takes the next byte with it only when it is `\` or `"`.
fn after_escape(b: &[u8], backslash: usize) -> usize {
    match b.get(backslash + 1) {
        Some(b'\\' | b'"') => backslash + 2,
        _ => backslash + 1,
    }
}

/// The end of a one-line basic string whose body starts at `k`: after its closing quote, or at
/// (before) an LF.
fn basic_end(b: &[u8], mut k: usize) -> usize {
    while k < b.len() {
        match b[k] {
            b'"' => return k + 1,
            b'\n' => return k,
            b'\\' => k = after_escape(b, k),
            _ => k += 1,
        }
    }
    b.len()
}

/// The end of a multi-line string with quote `q` whose body starts at `k`: after the first
/// unescaped triple quote and up to two more quotes; the end of the text when unclosed.
fn multi_line_end(b: &[u8], mut k: usize, q: u8) -> usize {
    let delim = [q, q, q];
    while k < b.len() {
        if q == b'"' && b[k] == b'\\' {
            k = after_escape(b, k);
            continue;
        }
        if b[k..].starts_with(&delim) {
            let mut end = k + 3;
            for _ in 0..2 {
                if b.get(end) == Some(&q) {
                    end += 1;
                }
            }
            return end;
        }
        k += 1;
    }
    b.len()
}

// ## m4b-t05-fileio fix round 1 (review m12): the procfs probe lives with the shared `Dir`.
#[cfg(any(test, feature = "test-support"))]
pub use crate::fileio::dir::with_procfs_absent;

/// Appends `alias` to the manifest's `aliases` through `toml_edit`, keeping comments, layout,
/// line endings and the BOM (R-M4b-8's "Also add … as an alias for everyone"). Nothing changes
/// when the library already binds that name. Refuses a missing, linked, unreadable, invalid or
/// newer manifest and a full alias list, and then writes nothing.
pub fn add_alias(root: &Path, alias: &LogicalDb) -> Result<(), WriteError> {
    add_alias_checked(root, alias, None)
}

// ## m4b-t16-attach (named exception): the mapping dialog shows exactly what is written.
/// [`add_alias`] only while the manifest's bytes are still `before` (what the mapping dialog
/// read and showed, [`ManifestPreview::bytes`]); a file changed since is refused with
/// [`WriteError::Changed`] and nothing is written.
pub fn add_alias_if_unchanged(
    root: &Path,
    alias: &LogicalDb,
    before: &[u8],
) -> Result<(), WriteError> {
    add_alias_checked(root, alias, Some(before))
}

fn add_alias_checked(
    root: &Path,
    alias: &LogicalDb,
    before: Option<&[u8]>,
) -> Result<(), WriteError> {
    // ## m4b-t16-attach
    let root_dir = Dir::open_root(root).map_err(write_error)?;
    let h = root_dir.subdir(HACIVAT).map_err(|e| match e {
        FsError::Io(e) if e.kind() == io::ErrorKind::NotFound => WriteError::NoManifest,
        e => write_error(e),
    })?;
    h.sweep_stale_temps(&is_our_temp);
    let (bytes, meta) = read_manifest_file(&h).map_err(|e| match e {
        FsError::Io(e) if e.kind() == io::ErrorKind::NotFound => WriteError::NoManifest,
        e => WriteError::Unreadable(read_diag(e)),
    })?;
    // ## m4b-t16-attach (named lines)
    if before.is_some_and(|b| b != bytes.as_slice()) {
        return Err(WriteError::Changed);
    }
    let folder = root.file_name().and_then(OsStr::to_str).unwrap_or("");
    match with_alias(&bytes, folder, alias)? {
        None => Ok(()),
        Some(out) => replace(&h, &out, &meta),
    }
}

/// The bytes [`add_alias`] writes for a manifest of `bytes` in a library folder named
/// `folder` (`None`: the library binds `alias` already, nothing is written). Pure: the mapping
/// dialog shows its result (R-M4b-8).
pub fn with_alias(
    bytes: &[u8],
    folder: &str,
    alias: &LogicalDb,
) -> Result<Option<Vec<u8>>, WriteError> {
    // ## m4b-t16-attach
    let read = read_bytes(bytes, folder);
    if let Some(d) = read.diags.iter().find(|d| refuses_edit(d)) {
        return Err(WriteError::Unreadable(d.clone()));
    }
    let bound = read.logical(folder);
    if bound.is_some_and(|b| b.same_db(alias)) || read.aliases.iter().any(|a| a.same_db(alias)) {
        return Ok(None);
    }

    let bom = bytes.starts_with(BOM);
    let text = std::str::from_utf8(bytes.strip_prefix(BOM).unwrap_or(bytes))
        .map_err(|_| WriteError::Unreadable(Diag::NotUtf8))?;
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|_| WriteError::Unreadable(Diag::InvalidToml))?;
    match doc.get_mut("aliases") {
        None => {
            let mut list = toml_edit::Array::new();
            list.push(alias.as_str());
            doc.insert("aliases", toml_edit::value(list));
        }
        Some(item) => {
            let Some(list) = item.as_array_mut() else {
                return Err(WriteError::Unreadable(Diag::WrongType(Field::Aliases)));
            };
            if list.len() >= MAX_ALIASES {
                return Err(WriteError::TooManyAliases);
            }
            list.push(alias.as_str());
        }
    }
    // ## m4b-t03-registry fix round 1 (review I1): `toml_edit` writes LF after every line, so
    // the text goes back to the file's dominant line ending (ties LF), and keeps its BOM.
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let edited = doc.to_string().replace("\r\n", "\n");
    let edited = if crlf > lf {
        edited.replace('\n', "\r\n")
    } else {
        edited
    };
    let mut out = Vec::with_capacity(edited.len() + BOM.len());
    if bom {
        out.extend_from_slice(BOM);
    }
    out.extend_from_slice(edited.as_bytes());
    if out.len() > MANIFEST_MAX_BYTES {
        return Err(WriteError::Unreadable(Diag::TooLarge));
    }
    // ## m4b-t16-attach (named line)
    Ok(Some(out))
}

/// Whether a reading problem means the file must not be rewritten.
fn refuses_edit(d: &Diag) -> bool {
    matches!(
        d,
        Diag::TooLarge
            | Diag::NotUtf8
            | Diag::TooDeep
            | Diag::InvalidToml
            | Diag::MissingFormat
            | Diag::BadFormat
            | Diag::NewerFormat(_)
            | Diag::WrongType(Field::Aliases)
    )
}

/// Replaces the manifest with `bytes` through a same-folder temp file and a rename (never
/// writing through a hard link), keeping the old file's mode.
fn replace(h: &Dir, bytes: &[u8], old: &Metadata) -> Result<(), WriteError> {
    let (tmp, mut f) = create_temp(h).map_err(write_error)?;
    let written = keep_mode(&f, old).and_then(|()| write_synced(&mut f, bytes));
    drop(f);
    let renamed = written
        .map_err(FsError::Io)
        .and_then(|()| h.rename(&tmp, MANIFEST));
    if let Err(e) = renamed {
        h.discard_temp(&tmp);
        return Err(write_error(e));
    }
    h.sync();
    Ok(())
}

#[cfg(unix)]
fn keep_mode(f: &File, old: &Metadata) -> io::Result<()> {
    f.set_permissions(old.permissions())
}

#[cfg(not(unix))]
fn keep_mode(_: &File, _: &Metadata) -> io::Result<()> {
    Ok(())
}

fn write_synced(f: &mut File, bytes: &[u8]) -> io::Result<()> {
    f.write_all(bytes)?;
    f.sync_all()
}

/// A random same-folder temp name (R-T1a-4: never the process id). Starts with `.`, so the
/// scanner never lists it.
fn temp_name() -> String {
    format!(".{MANIFEST}.hv-{}.tmp", uuid::Uuid::new_v4().simple())
}

/// The manifest's bytes (at most one past the cap) and its metadata, opened without following
/// a link and never blocking on a FIFO.
fn read_manifest_file(h: &Dir) -> Result<(Vec<u8>, Metadata), FsError> {
    let meta = h.lstat(MANIFEST)?;
    if is_link(&meta) {
        return Err(FsError::Link);
    }
    if !meta.is_file() {
        return Err(FsError::NotFile);
    }
    if meta.len() > MANIFEST_MAX_BYTES as u64 {
        return Err(FsError::TooLarge);
    }
    let file = h.open_read(MANIFEST, &meta)?;
    let mut bytes = Vec::new();
    file.take(MANIFEST_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(FsError::Io)?;
    if bytes.len() > MANIFEST_MAX_BYTES {
        return Err(FsError::TooLarge);
    }
    Ok((bytes, meta))
}

fn read_diag(e: FsError) -> Diag {
    match e {
        FsError::Link => Diag::LinkIgnored,
        FsError::NotDir => Diag::NotAFolder,
        FsError::NotFile => Diag::NotAFile,
        FsError::TooLarge => Diag::TooLarge,
        FsError::Changed => Diag::Changed,
        FsError::Io(e) => Diag::Unreadable(e.kind()),
    }
}

fn write_error(e: FsError) -> WriteError {
    match e {
        FsError::Link | FsError::NotDir => WriteError::NotAFolder,
        FsError::NotFile => WriteError::Unreadable(Diag::NotAFile),
        FsError::TooLarge => WriteError::Unreadable(Diag::TooLarge),
        FsError::Changed => WriteError::Changed,
        FsError::Io(e) => WriteError::Io(e.kind()),
    }
}

/// Whether `name` is exactly a temp name [`temp_name`] makes.
fn is_our_temp(name: &str) -> bool {
    name.strip_prefix(&format!(".{MANIFEST}.hv-"))
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .is_some_and(|id| {
            id.len() == 32
                && id
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
}

// ## m4b-t05-fileio fix round 1 (review m12): the `Dir` methods that only the manifest uses,
// over the shared `crate::fileio::dir::Dir`.

/// A new temp file with a random name, mode from the umask (R-M4b-42).
fn create_temp(h: &Dir) -> Result<(String, File), FsError> {
    let name = temp_name();
    let f = h.create_temp(&name, false)?;
    Ok((name, f))
}

/// `.hacivat` under `root`, created with default permissions when absent (R-M4b-42).
fn hacivat_creating(root: &Dir) -> Result<Dir, FsError> {
    match root.lstat(HACIVAT) {
        Ok(_) => {}
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
            match root.write_checked(|| fs::DirBuilder::new().create(root.child(HACIVAT))) {
                Ok(()) => {}
                Err(FsError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    }
    root.subdir(HACIVAT)
}

// ## m4b-t01a-seams

// ## m4b-t16-attach (named exception): what the attach dialog shows of a folder's manifest.

/// What the attach dialog shows of a folder's manifest (R-M4b-8, Q6): whether one is there,
/// its text when it could be read, and what Hacivat makes of it. Read exactly as [`read`]
/// reads (no link followed, never a FIFO, at most [`MANIFEST_MAX_BYTES`]).
#[derive(Debug, Clone, Default)]
pub struct ManifestPreview {
    /// Something is at `.hacivat/workspace.toml` (or `.hacivat` itself is a link or a file):
    /// the attach writes nothing and reads what is there.
    pub present: bool,
    /// The file's text (UTF-8, its BOM dropped) when it was read; untrusted, so it is shown
    /// escaped (`names::escape_untrusted`).
    pub text: Option<String>,
    /// The file's bytes as read (empty when it was not read): what
    /// [`add_alias_if_unchanged`] compares against.
    pub bytes: Vec<u8>,
    pub read: ManifestRead,
}

/// Reads `<library_dir>/.hacivat/workspace.toml` for the attach dialog.
pub fn preview(library_dir: &Path) -> ManifestPreview {
    let folder = library_dir
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    let found = Dir::open_root(library_dir)
        .and_then(|root| root.subdir(HACIVAT))
        .and_then(|h| read_manifest_file(&h));
    match found {
        Ok((bytes, _)) => {
            let text = std::str::from_utf8(bytes.strip_prefix(BOM).unwrap_or(&bytes))
                .ok()
                .map(str::to_string);
            ManifestPreview {
                present: true,
                text,
                read: read_bytes(&bytes, folder),
                bytes,
            }
        }
        Err(FsError::Io(e)) if e.kind() == io::ErrorKind::NotFound => ManifestPreview::default(),
        Err(e) => ManifestPreview {
            present: true,
            text: None,
            bytes: Vec::new(),
            read: ManifestRead {
                diags: vec![read_diag(e)],
                ..ManifestRead::default()
            },
        },
    }
}

/// Whether `d` says a manifest that is there could not be read or understood (R-T10-7: the
/// library then keeps the binding it last read, instead of falling back to its folder name).
/// A newer `format` is not one: it binds by folder name on purpose (R-M4b-21).
pub fn is_read_failure(d: &Diag) -> bool {
    matches!(
        d,
        Diag::LinkIgnored
            | Diag::NotAFolder
            | Diag::NotAFile
            | Diag::Changed
            | Diag::Unreadable(_)
            | Diag::TooLarge
            | Diag::NotUtf8
            | Diag::TooDeep
            | Diag::InvalidToml
            | Diag::MissingFormat
            | Diag::BadFormat
            | Diag::WrongType(Field::Database)
            | Diag::BadName(Field::Database, _)
    )
}
// ## m4b-t16-attach
