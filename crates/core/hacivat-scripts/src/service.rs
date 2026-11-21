//! The library service: one actor on the core runtime that owns the registry, the scans, the
//! watchers and the status refresh, and publishes immutable [`WorkspaceSnapshot`]s through
//! `arc-swap` with a generation (spec §5.4 rule 2; HV-SCRIPT-01..04, -08).
//!
//! ## m4b-t01a-seams
//! Task 1a: the signatures every later task codes against, as fail-closed stubs (F31):
//! [`LibraryService::open`] publishes an empty snapshot in
//! `LibraryState::Unavailable(NotImplemented)`, starts no task and scans nothing; every other
//! call answers `NotImplemented` and touches no file. Task 10 writes the actor.
//!
//! Two deviations from the plan's sketch (ADR 0009 `m4b-t01a`): `open` takes the core
//! runtime's `tokio::runtime::Handle` (the app passes `CoreRuntime::handle()`), because
//! `CoreRuntime` lives in `hacivat-session`, which this crate never depends on (R-M4b-28); and
//! [`ScriptChunks`] mirrors `hacivat_exec::ScriptChunks` (the app adapts one to the other, as
//! for `SpanKind`), because neither crate may depend on the other.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::ArcSwap;
use hacivat_git::GitState;
use hacivat_settings::paths::LocalPaths;
use tokio::runtime::Handle;
use tokio::sync::broadcast;

use crate::events::LibraryEvent;
use crate::fileio::{Decoded, DiskStamp, ReadError, SaveError, SaveText};
use crate::manifest;
use crate::names::LogicalDb;
use crate::pathsafe::RelPath;
use crate::registry::{LibraryId, LibraryRoot, LocalMapping, RegistryError};
use crate::scan::{EntryKind, ScriptEntry};

/// How many events a slow receiver may lag behind before it misses some (it then re-reads the
/// snapshot).
const EVENT_CAPACITY: usize = 256;

/// Everything the UI reads, synchronously: all libraries, the local mappings and the status.
#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    pub generation: u64,
    /// The service's own state (Task 1a's stub: `Unavailable(NotImplemented)`).
    pub state: LibraryState,
    pub libraries: Arc<[Arc<LibrarySnapshot>]>,
    pub mappings: Arc<[LocalMapping]>,
    pub git: HashMap<LibraryId, GitState>,
}

impl WorkspaceSnapshot {
    /// The snapshot of library `id`, if it is registered.
    pub fn library(&self, id: LibraryId) -> Option<&LibrarySnapshot> {
        self.libraries.iter().map(Arc::as_ref).find(|l| l.id == id)
    }
}

/// One library as last scanned.
#[derive(Debug, Clone)]
pub struct LibrarySnapshot {
    pub generation: u64,
    pub id: LibraryId,
    pub root: LibraryRoot,
    pub db: LogicalDb,
    pub aliases: Arc<[LogicalDb]>,
    /// Sorted, folders first.
    pub entries: Arc<[ScriptEntry]>,
    pub state: LibraryState,
    pub diagnostics: Arc<[LibraryDiag]>,
}

/// Where a library (or the service) stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryState {
    Scanning,
    Watching,
    Polling(PollReason),
    Unavailable(UnavailableReason),
}

/// Why a library is polled instead of watched (HV-SCRIPT-08 step 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollReason {
    NetworkFs,
    /// Linux: the folders would take more than half the free inotify watch budget.
    WatchBudget,
    BackendLost,
}

/// Why a library shows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// Task 1a's stub serves nothing.
    NotImplemented,
    /// The folder is gone or unmounted (it stays registered).
    Missing,
    /// The root's canonical path changed (it became a link).
    RootChanged,
}

/// A library diagnostic, shown escaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryDiag {
    Manifest(manifest::Diag),
}

/// Why attaching a folder was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachError {
    /// Task 1a's stub attaches nothing.
    NotImplemented,
}

/// Why creating, renaming or trashing a file was refused; the library is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOpError {
    /// Task 1a's stub changes nothing.
    NotImplemented,
}

/// A script read from disk in UTF-8 chunks, each cut at a character boundary (Task 12's
/// "Run as script" source). Mirrors `hacivat_exec::ScriptChunks`.
pub trait ScriptChunks: Send {
    /// The next chunk, `None` at the end.
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>>;
}

/// Why a chunk could not be read. Mirrors `hacivat_exec::ChunkError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// The file changed on disk since the run started (HV-SCRIPT-12; carries-w1 T12).
    Changed,
    /// The bytes did not decode in the file's encoding.
    Decode,
    Io(io::ErrorKind),
}

/// The library service. Cheap to share by reference; the actor is Task 10's.
pub struct LibraryService {
    snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    events: broadcast::Sender<LibraryEvent>,
    paths: LocalPaths,
    rt: Handle,
}

impl fmt::Debug for LibraryService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryService")
            .field("generation", &self.snapshot.load().generation)
            .finish_non_exhaustive()
    }
}

impl LibraryService {
    /// Starts the service on the core runtime `rt`, over the registry under `paths`.
    ///
    /// Task 1a stub (fail closed): reads no registry, starts no task, publishes one empty
    /// snapshot in `LibraryState::Unavailable(NotImplemented)`.
    pub fn open(paths: &LocalPaths, rt: &Handle) -> (Self, broadcast::Receiver<LibraryEvent>) {
        let (events, rx) = broadcast::channel(EVENT_CAPACITY);
        let empty = WorkspaceSnapshot {
            generation: 0,
            state: LibraryState::Unavailable(UnavailableReason::NotImplemented),
            libraries: Arc::from([]),
            mappings: Arc::from([]),
            git: HashMap::new(),
        };
        let service = LibraryService {
            snapshot: Arc::new(ArcSwap::from_pointee(empty)),
            events,
            paths: paths.clone(),
            rt: rt.clone(),
        };
        (service, rx)
    }

    /// The current snapshot; never blocks.
    pub fn snapshot(&self) -> Arc<WorkspaceSnapshot> {
        self.snapshot.load_full()
    }

    /// Another receiver of the service's events.
    pub fn subscribe(&self) -> broadcast::Receiver<LibraryEvent> {
        self.events.subscribe()
    }

    /// Registers `path` as a library bound to `db` (the folder name when `None`), writing
    /// `.hacivat/workspace.toml` only when `write_manifest` and the file is absent.
    ///
    /// Task 1a stub (fail closed): attaches nothing and touches no file.
    pub async fn attach(
        &self,
        path: PathBuf,
        db: Option<LogicalDb>,
        write_manifest: bool,
    ) -> Result<LibraryId, AttachError> {
        let _ = (&self.paths, &self.rt, path, db, write_manifest);
        Err(AttachError::NotImplemented)
    }

    /// Forgets library `id`; the folder is never touched.
    ///
    /// Task 1a stub (fail closed): changes nothing.
    pub async fn detach(&self, id: LibraryId) -> Result<(), RegistryError> {
        let _ = id;
        Err(RegistryError::NotImplemented)
    }

    /// Adds a local mapping; with `share_alias`, also appends the database as an alias to the
    /// library's manifest.
    ///
    /// Task 1a stub (fail closed): changes nothing.
    pub async fn map(&self, m: LocalMapping, share_alias: bool) -> Result<(), RegistryError> {
        let _ = (m, share_alias);
        Err(RegistryError::NotImplemented)
    }

    /// Reads and decodes a script.
    ///
    /// Task 1a stub (fail closed): opens nothing.
    pub async fn read(&self, id: LibraryId, rel: &RelPath) -> Result<Decoded, ReadError> {
        let _ = (id, rel);
        Err(ReadError::NotImplemented)
    }

    /// Saves a script (HV-SCRIPT-07), recording the echo before the rename.
    ///
    /// Task 1a stub (fail closed): writes nothing.
    pub async fn save(
        &self,
        id: LibraryId,
        rel: &RelPath,
        text: SaveText,
        expect: Option<DiskStamp>,
    ) -> Result<DiskStamp, SaveError> {
        let _ = (id, rel, text, expect);
        Err(SaveError::NotImplemented)
    }

    /// Creates an empty script or a folder.
    ///
    /// Task 1a stub (fail closed): creates nothing.
    pub async fn create(
        &self,
        id: LibraryId,
        rel: &RelPath,
        kind: EntryKind,
    ) -> Result<(), FileOpError> {
        let _ = (id, rel, kind);
        Err(FileOpError::NotImplemented)
    }

    /// Renames a script or folder inside its library.
    ///
    /// Task 1a stub (fail closed): renames nothing.
    pub async fn rename(
        &self,
        id: LibraryId,
        from: &RelPath,
        to: &RelPath,
    ) -> Result<(), FileOpError> {
        let _ = (id, from, to);
        Err(FileOpError::NotImplemented)
    }

    /// Moves a script or folder to the OS Trash (Q11).
    ///
    /// Task 1a stub (fail closed): moves nothing.
    pub async fn trash(&self, id: LibraryId, rel: &RelPath) -> Result<(), FileOpError> {
        let _ = (id, rel);
        Err(FileOpError::NotImplemented)
    }

    /// A chunked UTF-8 reader of a script for "Run as script" (Task 12's source).
    ///
    /// Task 1a stub (fail closed): opens nothing.
    pub fn open_stream(
        &self,
        id: LibraryId,
        rel: &RelPath,
    ) -> Result<Box<dyn ScriptChunks>, ReadError> {
        let _ = (id, rel);
        Err(ReadError::NotImplemented)
    }
}
// ## m4b-t01a-seams
