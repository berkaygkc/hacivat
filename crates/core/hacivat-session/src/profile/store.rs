//! The `connections.toml` store: load with diagnostics and quarantine, save atomically, never
//! a secret (HV-SEC-05, P29). Bodies in Task 12.

use std::path::{Path, PathBuf};

use hacivat_proto::ProfileId;

use super::Profile;

/// A problem found while loading `connections.toml`; shown, never dropped (P29).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProfileDiag {
    InvalidEntry {
        index: usize,
        reason: String,
    },
    DuplicateId {
        id: ProfileId,
    },
    UnknownEngine {
        index: usize,
        engine: String,
    },
    /// An entry carried a password key; it was ignored (never echoed).
    SecretKeyIgnored {
        index: usize,
    },
    NewerVersion {
        found: u32,
    },
}

/// What [`ProfileStore::open`] loaded (P29).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoadedProfiles {
    pub profiles: Vec<Profile>,
    pub diags: Vec<ProfileDiag>,
    /// Where an unreadable file was set aside.
    pub quarantined: Option<PathBuf>,
}

/// Why saving failed (HV-SEC-05).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StoreError {
    /// The file is from a newer version, or not writable: nothing is written.
    ReadOnly,
    Io(String),
}

/// `connections.toml` (HV-SEC-05).
#[derive(Debug)]
pub struct ProfileStore {
    path: PathBuf,
    read_only: bool,
}

impl ProfileStore {
    /// Opens the store at `path` and loads its profiles (Task 12). The stub loads nothing.
    pub fn open(path: PathBuf) -> (ProfileStore, LoadedProfiles) {
        (
            ProfileStore {
                path,
                read_only: false,
            },
            LoadedProfiles::default(),
        )
    }

    /// Writes `profiles`, keeping unknown keys (Task 12).
    pub fn save(&self, _profiles: &[Profile]) -> Result<(), StoreError> {
        Err(StoreError::Io(String::new()))
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
