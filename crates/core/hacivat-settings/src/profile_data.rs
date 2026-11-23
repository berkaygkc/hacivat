// ## m4-t01c-core
//! Per-profile user data: `<config>/profiles/<uuid>.toml` ([`LocalPaths::profile_toml`]).
//!
//! This module is the **only** reader and writer of those files. The file is a sectioned
//! document: each owner keeps one top-level table of its own (`[metadata]`: M4, typed as
//! `hacivat_meta::ProfileUserData`; later milestones add theirs, such as `[masks]` or `[ai]`)
//! and never touches another's.
//!
//! - [`ProfileDataDoc::save_section`] re-reads the file under a process-wide lock, replaces
//!   only the named table (keeping its place and the comment above it), and writes the file
//!   atomically with mode `0600` (`fsio::atomic_write`; parents `0700`). Every other byte, the
//!   formatting and comments of unknown sections included, stays as it was. Two owners that
//!   loaded the file before either saved therefore never lose each other's section.
//! - A file that is not TOML is never overwritten ([`ProfileDataError::Corrupt`]): other
//!   owners' data may be in it.
//! - The profile id must be a canonical UUID (`8-4-4-4-12` ASCII hex digits), so no id can
//!   name a path outside `profiles/`.
//! - Sections are read and written through `serde`; a section of the wrong shape is an error
//!   for that section only. A section written inline (`metadata = { … }`) is replaced by a
//!   table at the end of the file.
//! - The layout of the file is kept (T1c fix round 1): a leading byte-order mark stays, and a
//!   file whose lines all end in CRLF keeps CRLF. A file that mixes CRLF and LF line endings is
//!   read but never rewritten ([`ProfileDataError::MixedLineEndings`]).
//! - The lock is per process: two app instances saving the same file at the same moment can
//!   lose one save (the last rename wins). One app instance per user is the supported setup.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml_edit::{DocumentMut, Item, Table};

use crate::fsio;
use crate::paths::LocalPaths;

/// Serialises every read-modify-write of a profile data file in this process.
static WRITER: Mutex<()> = Mutex::new(());

/// Why a profile data file could not be read or written.
#[derive(Debug)]
pub enum ProfileDataError {
    /// The profile id is not a canonical UUID.
    InvalidProfileId,
    /// The file exists but is not TOML; it is left untouched.
    Corrupt,
    /// The named section does not have the owner's shape (or is not a table).
    InvalidSection(String),
    /// The owner's value is not a TOML table.
    Encode(String),
    /// The file mixes CRLF and LF line endings; it is read, but never rewritten.
    MixedLineEndings,
    Io(io::Error),
}

impl fmt::Display for ProfileDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileId => f.write_str("invalid profile id"),
            Self::Corrupt => f.write_str("profile data file is not TOML"),
            Self::InvalidSection(name) => write!(f, "invalid section [{name}]"),
            Self::Encode(e) => write!(f, "section is not a table: {e}"),
            Self::MixedLineEndings => f.write_str("profile data file mixes CRLF and LF"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProfileDataError {}

/// One profile's data file, as last read or written by this value.
#[derive(Debug, Clone)]
pub struct ProfileDataDoc {
    path: PathBuf,
    doc: DocumentMut,
}

impl ProfileDataDoc {
    /// Reads `profiles/<uuid>.toml` under `paths.config`; a missing file is an empty document.
    pub fn load(paths: &LocalPaths, uuid: &str) -> Result<Self, ProfileDataError> {
        if !is_canonical_uuid(uuid) {
            return Err(ProfileDataError::InvalidProfileId);
        }
        let path = paths.profile_toml(uuid);
        let (doc, _) = read_doc(&path)?;
        Ok(ProfileDataDoc { path, doc })
    }

    /// The section `name` as the owner's type; `None` when the file has none.
    pub fn section<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, ProfileDataError> {
        let Some(item) = self.doc.get(name) else {
            return Ok(None);
        };
        let invalid = || ProfileDataError::InvalidSection(name.to_owned());
        let table = item.clone().into_table().map_err(|_| invalid())?;
        let mut alone = DocumentMut::new();
        for (k, v) in table.iter() {
            alone.insert(k, v.clone());
        }
        toml::from_str(&alone.to_string())
            .map(Some)
            .map_err(|_| invalid())
    }

    /// Replaces the section `name` with `value` in the file on disk (see the module docs).
    pub fn save_section<T: Serialize>(
        &mut self,
        name: &str,
        value: &T,
    ) -> Result<(), ProfileDataError> {
        let text = toml::to_string(value).map_err(|e| ProfileDataError::Encode(e.to_string()))?;
        let fresh = text
            .parse::<DocumentMut>()
            .map_err(|e| ProfileDataError::Encode(e.to_string()))?;
        let mut section = fresh.as_table().clone();
        section.set_implicit(false);
        clear_positions(&mut section);

        // ## m4b-t22-pre (T21 m12): how long the save waited for the lock, and wrote (debug
        // builds only, fix round 1 m5).
        #[cfg(debug_assertions)]
        let asked = std::time::Instant::now();
        let _writer = WRITER.lock().unwrap_or_else(|e| e.into_inner());
        #[cfg(debug_assertions)]
        let locked = std::time::Instant::now();
        let (mut doc, layout) = read_doc(&self.path)?;
        if layout.mixed {
            return Err(ProfileDataError::MixedLineEndings);
        }
        if let Some(Item::Table(old)) = doc.get_mut(name) {
            section.set_position(old.position());
            *section.decor_mut() = old.decor().clone();
            *old = section;
        } else {
            // New, or written some other way (inline): after everything the file holds, one
            // blank line below it.
            doc.remove(name);
            section.set_position(Some(last_position(doc.as_table()).map_or(0, |p| p + 1)));
            if !doc.to_string().is_empty() {
                section.decor_mut().set_prefix("\n");
            }
            doc.insert(name, Item::Table(section));
        }
        let mut out = doc.to_string();
        if layout.crlf {
            out = out.replace('\n', "\r\n");
        }
        if layout.bom {
            out.insert(0, BOM);
        }
        let written = fsio::atomic_write(&self.path, out.as_bytes()).map_err(ProfileDataError::Io);
        #[cfg(debug_assertions)]
        timings::note(locked - asked, locked.elapsed());
        written?;
        self.doc = doc;
        Ok(())
    }
}

// ## m4b-t22-pre (named item, T21 m12)
/// How long saves took in this process: for the diagnostics a test prints when a save it
/// waited on timed out (the durable write syncs twice, under the process-wide lock). Nothing
/// reads these to decide anything. ## m4b-t22-pre fix round 1 (T22-pre review m5): kept in
/// debug builds only (tests); a release build records nothing and reads zeros.
pub mod timings {
    #[cfg(debug_assertions)]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(debug_assertions)]
    use std::time::Duration;

    #[cfg(debug_assertions)]
    static SAVES: AtomicU64 = AtomicU64::new(0);
    #[cfg(debug_assertions)]
    static LOCK_WAIT_MAX_US: AtomicU64 = AtomicU64::new(0);
    #[cfg(debug_assertions)]
    static LOCKED_MAX_US: AtomicU64 = AtomicU64::new(0);
    #[cfg(debug_assertions)]
    static LOCK_WAIT_LAST_US: AtomicU64 = AtomicU64::new(0);
    #[cfg(debug_assertions)]
    static LOCKED_LAST_US: AtomicU64 = AtomicU64::new(0);

    /// The saves so far: how many, and the longest and the last wait for the lock and time
    /// under it (the read, the write and its syncs), in microseconds.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct SaveTimings {
        pub saves: u64,
        pub lock_wait_max_us: u64,
        pub locked_max_us: u64,
        pub lock_wait_last_us: u64,
        pub locked_last_us: u64,
    }

    #[cfg(debug_assertions)]
    pub(super) fn note(wait: Duration, locked: Duration) {
        let (wait, locked) = (micros(wait), micros(locked));
        SAVES.fetch_add(1, Ordering::Relaxed);
        LOCK_WAIT_MAX_US.fetch_max(wait, Ordering::Relaxed);
        LOCKED_MAX_US.fetch_max(locked, Ordering::Relaxed);
        LOCK_WAIT_LAST_US.store(wait, Ordering::Relaxed);
        LOCKED_LAST_US.store(locked, Ordering::Relaxed);
    }

    #[cfg(debug_assertions)]
    fn micros(d: Duration) -> u64 {
        u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
    }

    /// The timings so far.
    #[cfg(not(debug_assertions))]
    pub fn get() -> SaveTimings {
        SaveTimings::default()
    }

    /// The timings so far.
    #[cfg(debug_assertions)]
    pub fn get() -> SaveTimings {
        SaveTimings {
            saves: SAVES.load(Ordering::Relaxed),
            lock_wait_max_us: LOCK_WAIT_MAX_US.load(Ordering::Relaxed),
            locked_max_us: LOCKED_MAX_US.load(Ordering::Relaxed),
            lock_wait_last_us: LOCK_WAIT_LAST_US.load(Ordering::Relaxed),
            locked_last_us: LOCKED_LAST_US.load(Ordering::Relaxed),
        }
    }
}

const BOM: char = '\u{feff}';

/// How a file was laid out on disk, so a save writes it back the same way.
#[derive(Debug, Clone, Copy, Default)]
struct Layout {
    /// It began with a byte-order mark.
    bom: bool,
    /// Every line ended in CRLF.
    crlf: bool,
    /// Some lines ended in CRLF and some in LF.
    mixed: bool,
}

/// The file at `path` as a document (parsed without its byte-order mark and with LF line
/// endings) and its layout; a missing file is an empty one.
fn read_doc(path: &std::path::Path) -> Result<(DocumentMut, Layout), ProfileDataError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (bom, text) = match text.strip_prefix(BOM) {
                Some(rest) => (true, rest),
                None => (false, text.as_str()),
            };
            let crlf_count = text.matches("\r\n").count();
            let lf_count = text.matches('\n').count();
            let layout = Layout {
                bom,
                crlf: crlf_count > 0 && crlf_count == lf_count,
                mixed: crlf_count > 0 && crlf_count != lf_count,
            };
            let doc = text
                .replace("\r\n", "\n")
                .parse::<DocumentMut>()
                .map_err(|_| ProfileDataError::Corrupt)?;
            Ok((doc, layout))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Ok((DocumentMut::new(), Layout::default()))
        }
        Err(e) if e.kind() == io::ErrorKind::InvalidData => Err(ProfileDataError::Corrupt),
        Err(e) => Err(ProfileDataError::Io(e)),
    }
}

/// Drops every nested table's document position, so the section's subtables follow its header
/// wherever the section sits in the file.
fn clear_positions(table: &mut Table) {
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(t) => {
                t.set_position(None);
                clear_positions(t);
            }
            Item::ArrayOfTables(a) => {
                for t in a.iter_mut() {
                    t.set_position(None);
                    clear_positions(t);
                }
            }
            _ => {}
        }
    }
}

/// The highest document position of `table` and every table under it.
fn last_position(table: &Table) -> Option<isize> {
    let mut last = table.position();
    for (_, item) in table.iter() {
        let nested: Vec<&Table> = match item {
            Item::Table(t) => vec![t],
            Item::ArrayOfTables(a) => a.iter().collect(),
            _ => Vec::new(),
        };
        for t in nested {
            last = last.max(last_position(t));
        }
    }
    last
}

/// `8-4-4-4-12` ASCII hex digits, lower or upper case (what `ProfileId` writes is lower case).
fn is_canonical_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_shape() {
        assert!(is_canonical_uuid("6f9619ff-8b86-d011-b42d-00c04fc964ff"));
        assert!(!is_canonical_uuid("6f9619ff-8b86-d011-b42d-00c04fc964f"));
        assert!(!is_canonical_uuid("6f9619ff-8b86-d011-b42d-00c04fc964fg"));
        assert!(!is_canonical_uuid("6f9619ff/8b86-d011-b42d-00c04fc964ff"));
    }
}
