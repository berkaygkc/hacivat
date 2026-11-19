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
//!   for that section only.

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
    Io(io::Error),
}

impl fmt::Display for ProfileDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileId => f.write_str("invalid profile id"),
            Self::Corrupt => f.write_str("profile data file is not TOML"),
            Self::InvalidSection(name) => write!(f, "invalid section [{name}]"),
            Self::Encode(e) => write!(f, "section is not a table: {e}"),
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
        let doc = read_doc(&path)?;
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

        let _writer = WRITER.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = read_doc(&self.path)?;
        match doc.get_mut(name) {
            Some(Item::Table(old)) => {
                section.set_position(old.position());
                *section.decor_mut() = old.decor().clone();
                *old = section;
            }
            Some(other) => *other = Item::Table(section),
            None => {
                // After everything the file holds, one blank line below it.
                section.set_position(Some(last_position(doc.as_table()).map_or(0, |p| p + 1)));
                if !doc.to_string().is_empty() {
                    section.decor_mut().set_prefix("\n");
                }
                doc.insert(name, Item::Table(section));
            }
        }
        fsio::atomic_write(&self.path, doc.to_string().as_bytes()).map_err(ProfileDataError::Io)?;
        self.doc = doc;
        Ok(())
    }
}

/// The file at `path` as a document; a missing file is an empty one.
fn read_doc(path: &std::path::Path) -> Result<DocumentMut, ProfileDataError> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .map_err(|_| ProfileDataError::Corrupt),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(DocumentMut::new()),
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
