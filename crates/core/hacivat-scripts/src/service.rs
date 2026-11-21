// ## m4b-t01a-seams
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
// ## m4b-t01a-seams
//!
//! ## m4b-t10-service
//! Task 10: the actor (ADR 0009 `m4b-t10`).
//!
//! - **One actor, one thread.** [`LibraryService::open`] starts one long-lived task on the core
//!   runtime's blocking pool. It alone owns the [`LibraryRegistry`] (the single writer of
//!   `libraries.toml`), every library's listing, the event source and the status refresh, and it
//!   handles one message at a time from a bounded channel: commands from the API, hint batches
//!   from the event source, and its own timers. All of its file I/O (registry, manifest, scan,
//!   `lstat`, stamps, recovery sweep) runs on that blocking thread; the repository status runs
//!   on its own blocking task, at most one per library. No lock is ever held across an `.await`.
//! - **Immutable snapshots.** After each drained burst of messages the actor publishes one new
//!   [`WorkspaceSnapshot`] (generation + 1) through `arc-swap` if anything changed, then sends
//!   the events: one [`LibraryEvent::Changed`] per library that changed, then the file events.
//!   A reader never sees a half-updated snapshot: it gets the old `Arc` or the new one. Commands
//!   reply after their snapshot is published, so a caller that awaited `attach`, `detach` or
//!   `map` reads the result at once.
//! - **Hints, not events** (alg/28 C3). The event source (Task 9's OS watcher or poller, or the
//!   test fake, through Task 1a's [`EventSource`] seam) hands over paths that may have changed.
//!   Each is mapped into the library (a path outside the canonical root, under a skipped name, or
//!   with a bad name is dropped; `.git/index`, `HEAD`, `packed-refs` and `refs/` refresh the
//!   status; `.hacivat/` re-reads the manifest) and probed through [`pathsafe::join`] (no link
//!   followed, every component re-validated). A plain script file inside a listed folder is
//!   patched in place; anything else (a folder, a link, a new folder, an error, a burst of more
//!   than 1,000 paths, a dropped batch) rescans the library with [`scan::scan`] and diffs the
//!   listing. A lost backend (`RawHint::Lost`) or a source that will not start switches the
//!   library to polling: a rescan every [`ServiceOptions::poll`].
//! - **Settle, echo, rename** (HV-SCRIPT-08 steps 5–7). A changed file is stamped only once its
//!   `(len, mtime)` is the same one settle window later (at most 10 tries), and never when it is
//!   a cloud placeholder. A stamp equal to one Hacivat's own save recorded in the last 5 s is
//!   its echo and announces nothing; otherwise [`LibraryEvent::FileChanged`] carries it. A file
//!   gone is [`LibraryEvent::FileDeleted`], unless a file of the same content hash (known from a
//!   read, a save or a stamp) appeared in the same batch: then [`LibraryEvent::FileRenamed`].
//!   Stamps go through `fileio` (the only reader of script content, T5 mg2), under an
//!   environment the entry is shown under.
//! - **Status refresh** (HV-SCRIPT-11 step 7). Debounced [`ServiceOptions::git_debounce`] after
//!   any change and after a hint under `.git/`; a trigger while a status runs queues exactly one
//!   more. A new state that equals the old one publishes nothing.
//! - **Availability** (HV-SCRIPT-01 step 5). A library whose folder is missing, unreadable, no
//!   longer its canonical path or inside an environment folder is `Unavailable`, unwatched, with
//!   no entries and no status; it is re-checked every [`ServiceOptions::recheck`] and comes back
//!   by itself (a root that resolves elsewhere now never does: fail closed).
//! - **File calls take the tab's environment** (R-T5-6): [`read`](LibraryService::read),
//!   [`save`](LibraryService::save) and [`open_stream`](LibraryService::open_stream) pass it to
//!   `fileio`, which refuses a path not shown under it (`@production/` under staging).
//! - **Launch hygiene** (T5 mf4): recovery folders of libraries that are not registered are
//!   swept ([`recovery::sweep_unregistered`]), only when the registry was read cleanly.
//! - **Detach** forgets the library and discards its recovery copies (R-T5-4); the folder is
//!   never touched.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwap;
use hacivat_git::{FileStatus, GitLimits, GitState};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_settings::paths::LocalPaths;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::envscope;
use crate::events::LibraryEvent;
use crate::fileio::{
    self, Decoded, DiskStamp, ReadError, ReadLimits, SaveError, SaveText, TextEncoding,
};
use crate::manifest::{self, ManifestV1};
use crate::names::{DatabaseName, LogicalDb, NameError};
use crate::pathsafe::{self, JoinedKind, PathError, RelPath};
use crate::recovery;
use crate::registry::{
    AttachGuard, LibraryId, LibraryRegistry, LibraryRoot, LocalMapping, RegisteredLibrary,
    RegistryDiag, RegistryError,
};
use crate::scan::{
    self, EntryKind, Fs, LimitHit, Limits, LinkState, MetaKind, RealFs, ScriptEntry,
};
use crate::tree;
use crate::watch::{EventSource, HintSink, OsEventSource, RawHint, WatchError};

/// How many events a slow receiver may lag behind before it misses some (it then re-reads the
/// snapshot).
const EVENT_CAPACITY: usize = 256;
// ## m4b-t10-service
/// The actor's inbox. A hint batch that does not fit is dropped and its library rescanned.
const INBOX_CAPACITY: usize = 1024;
/// The most messages handled before a publish.
const DRAIN_MAX: usize = 64;
/// A batch with more paths than this rescans instead (HV-SCRIPT-08 step 3).
const BURST_PATHS: usize = 1_000;
/// The most settle rounds before a changing file is stamped anyway (HV-SCRIPT-08 step 5).
const SETTLE_TRIES: u32 = 10;
/// How many deleted paths per library are remembered, so their return is announced.
const RECENT_DELETES: usize = 1_024;
// ## m4b-t10-service

/// Everything the UI reads, synchronously: all libraries, the local mappings and the status.
#[derive(Debug, Clone)]
pub struct WorkspaceSnapshot {
    pub generation: u64,
    /// The service's own state: `Scanning` until the registry and the first scans are done,
    /// then `Watching`.
    pub state: LibraryState,
    pub libraries: Arc<[Arc<LibrarySnapshot>]>,
    pub mappings: Arc<[LocalMapping]>,
    pub git: HashMap<LibraryId, GitState>,
    /// ## m4b-t10-service: what loading `libraries.toml` found (set aside, newer, unreadable,
    /// dropped rows), for Settings › Script libraries.
    pub registry: Arc<[RegistryDiag]>,
}

impl WorkspaceSnapshot {
    /// The snapshot of library `id`, if it is registered.
    pub fn library(&self, id: LibraryId) -> Option<&LibrarySnapshot> {
        self.libraries.iter().map(Arc::as_ref).find(|l| l.id == id)
    }

    // ## m4b-t10-service
    /// The badge of `rel` in library `id`, when its status is ready and the path is not clean.
    /// The status is keyed by the scan's exact `RelPath` text (R-T1a-2: the NFD spelling macOS
    /// keeps on disk, whatever the index holds).
    pub fn git_status(&self, id: LibraryId, rel: &RelPath) -> Option<FileStatus> {
        match self.git.get(&id)? {
            GitState::Ready(status) => status.by_path.get(rel.as_str()).copied(),
            _ => None,
        }
    }
    // ## m4b-t10-service
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
    /// The event source stopped, or would not start.
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
    // ## m4b-t10-service (the T4 carry: every `RootProblem` is mapped)
    /// The folder cannot be read.
    Unreadable(io::ErrorKind),
    /// The folder is (inside) an environment folder; scoping is relative to the root, so it is
    /// refused ([`LibraryDiag::RootInEnvFolder`] names the folder).
    InEnvFolder,
    // ## m4b-t10-service
}

/// A library diagnostic. Names in it are already in their display form (escaped, NFC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryDiag {
    Manifest(manifest::Diag),
    // ## m4b-t10-service (the T4 carry)
    /// A scan diagnostic (bad names, unknown or contradictory environment folders, unreadable
    /// folders, a link hidden by its target's scope).
    Scan(scan::Diag),
    /// The scan stopped at this limit; what was found is listed (R-M4b-14).
    ScanStopped(LimitHit),
    /// This many more diagnostics were not kept.
    Suppressed(usize),
    /// The root sits in this environment folder (display form).
    RootInEnvFolder(String),
    /// Neither the manifest nor the folder name is a valid logical name: the library binds
    /// only through a local mapping.
    NoLogicalName,
    // ## m4b-t10-service
}

/// Why attaching a folder was refused; nothing was registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachError {
    // ## m4b-t10-service (Task 1a's `NotImplemented` is gone)
    /// The registry refused (read-only, already registered, nested, the home or an app folder,
    /// too many, or the folder cannot be resolved).
    Registry(RegistryError),
    /// `.hacivat/workspace.toml` could not be written (a linked `.hacivat`, say); nothing else
    /// was written into the folder.
    Manifest(manifest::WriteError),
    /// The folder is inside this environment folder (display form).
    RootInEnvFolder(String),
    /// A manifest was asked for without a database, and the folder name is not a valid name.
    NoLogicalName,
    /// The service has stopped.
    Stopped,
    // ## m4b-t10-service
}

// ## m4b-t10-service
/// Why a local mapping was refused; nothing changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    Registry(RegistryError),
    /// The alias could not be added to the library's manifest (none, linked, newer, full).
    Alias(manifest::WriteError),
    /// The database name is not a valid logical name, so it cannot be an alias.
    BadName(NameError),
    /// The library is unavailable, so its manifest cannot be written.
    Unavailable,
    /// The service has stopped.
    Stopped,
}

/// The service's timings and bounds. The defaults are the product's; tests shorten the timers.
#[derive(Debug, Clone)]
pub struct ServiceOptions {
    /// How long a changed file's `(len, mtime)` must stay put before it is stamped (the debounce
    /// window, alg/28 C6).
    pub settle: Duration,
    /// The status refresh debounce (HV-SCRIPT-11 step 7).
    pub git_debounce: Duration,
    /// The rescan interval of a polled library.
    pub poll: Duration,
    /// How often an unavailable library is checked again.
    pub recheck: Duration,
    /// How long a save's echo is remembered.
    pub echo_ttl: Duration,
    pub limits: Limits,
    pub git: GitLimits,
    /// The user's home folder, which is never a library (`AttachGuard`).
    pub home: Option<PathBuf>,
}

impl Default for ServiceOptions {
    fn default() -> Self {
        ServiceOptions {
            settle: crate::watch::DEBOUNCE,
            git_debounce: Duration::from_millis(500),
            poll: Duration::from_secs(2),
            recheck: Duration::from_secs(2),
            echo_ttl: Duration::from_secs(5),
            limits: Limits::default(),
            git: GitLimits::default(),
            home: std::env::home_dir(),
        }
    }
}
// ## m4b-t10-service

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

/// The library service. Cheap to share by reference; dropping it stops the actor.
pub struct LibraryService {
    snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    events: broadcast::Sender<LibraryEvent>,
    // ## m4b-t10-service
    rt: Handle,
    tx: mpsc::Sender<Msg>,
    closed: Arc<AtomicBool>,
    echo: Echo,
    // ## m4b-t10-service
}

impl fmt::Debug for LibraryService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryService")
            .field("generation", &self.snapshot.load().generation)
            .finish_non_exhaustive()
    }
}

impl LibraryService {
    /// Starts the service on the core runtime `rt`, over the registry under `paths`, watching
    /// libraries with the OS event source (Task 9).
    ///
    /// ## m4b-t10-service
    /// Returns at once; the registry is read and the libraries scanned on the actor
    /// (`WorkspaceSnapshot::state` is `Scanning` until then).
    pub fn open(paths: &LocalPaths, rt: &Handle) -> (Self, broadcast::Receiver<LibraryEvent>) {
        let source: Box<dyn EventSource> = match OsEventSource::new() {
            Ok(os) => Box::new(os),
            Err(_) => Box::new(NoSource),
        };
        Self::open_with(paths, rt, source, ServiceOptions::default())
    }

    // ## m4b-t10-service
    /// [`open`](Self::open) with an event source and options of the caller's (tests pass
    /// `testing::FakeEventSource` and short timers).
    pub fn open_with(
        paths: &LocalPaths,
        rt: &Handle,
        source: Box<dyn EventSource>,
        opts: ServiceOptions,
    ) -> (Self, broadcast::Receiver<LibraryEvent>) {
        let (events, rx) = broadcast::channel(EVENT_CAPACITY);
        let empty = WorkspaceSnapshot {
            generation: 0,
            state: LibraryState::Scanning,
            libraries: Arc::from([]),
            mappings: Arc::from([]),
            git: HashMap::new(),
            registry: Arc::from([]),
        };
        let snapshot = Arc::new(ArcSwap::from_pointee(empty));
        let (tx, inbox) = mpsc::channel(INBOX_CAPACITY);
        let closed = Arc::new(AtomicBool::new(false));
        let echo = Echo::default();
        let (paths, handle) = (paths.clone(), rt.clone());
        let (actor_tx, actor_closed) = (tx.clone(), closed.clone());
        let (actor_snapshot, actor_events, actor_echo) =
            (snapshot.clone(), events.clone(), echo.clone());
        rt.spawn_blocking(move || {
            // The registry is read here, on the actor's own blocking thread.
            let (registry, diags) = LibraryRegistry::load(&paths.libraries_toml());
            let actor = Actor {
                paths,
                rt: handle,
                opts,
                tx: actor_tx,
                closed: actor_closed,
                snapshot: actor_snapshot,
                events: actor_events,
                echo: actor_echo,
                source,
                registry,
                registry_diags: Arc::from([]),
                libs: Vec::new(),
                git: HashMap::new(),
                generation: 0,
                loaded: false,
                dirty: HashSet::new(),
                workspace_dirty: false,
                pending: Vec::new(),
                known: HashMap::new(),
                settle: HashMap::new(),
                deleted: HashMap::new(),
                waiters: Vec::new(),
            };
            actor.run(inbox, diags);
        });
        let service = LibraryService {
            snapshot,
            events,
            rt: rt.clone(),
            tx,
            closed,
            echo,
        };
        (service, rx)
    }
    // ## m4b-t10-service

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
    /// ## m4b-t10-service
    /// `db` is what the manifest says; without one written (`write_manifest == false`, or a
    /// manifest already there, which is read and never rewritten) the library binds what its
    /// folder says, and a database of the user's choosing is a local mapping ([`map`]). Refused
    /// before anything is written: the registry's checks (HV-SCRIPT-01 step 2) and a root inside
    /// an environment folder. A manifest that cannot be written refuses the attach.
    ///
    /// [`map`]: Self::map
    pub async fn attach(
        &self,
        path: PathBuf,
        db: Option<LogicalDb>,
        write_manifest: bool,
    ) -> Result<LibraryId, AttachError> {
        self.ask(|reply| Msg::Attach {
            path,
            db,
            write_manifest,
            reply,
        })
        .await
        .unwrap_or(Err(AttachError::Stopped))
    }

    /// Forgets library `id`; the folder is never touched.
    ///
    /// ## m4b-t10-service
    /// Stops watching it, drops its listing and status, and discards its recovery copies
    /// (R-T5-4). `Io(BrokenPipe)` when the service has stopped.
    pub async fn detach(&self, id: LibraryId) -> Result<(), RegistryError> {
        self.ask(|reply| Msg::Detach { id, reply })
            .await
            .unwrap_or(Err(STOPPED))
    }

    /// Adds a local mapping; with `share_alias`, also appends the database as an alias to the
    /// library's manifest.
    ///
    /// ## m4b-t10-service
    /// The alias is written first (through `toml_edit`, keeping the teammate's formatting); if
    /// that fails nothing changes. The mapping goes only into `libraries.toml` (R-M4b-8).
    pub async fn map(&self, m: LocalMapping, share_alias: bool) -> Result<(), MapError> {
        self.ask(|reply| Msg::Map {
            m,
            share_alias,
            reply,
        })
        .await
        .unwrap_or(Err(MapError::Stopped))
    }

    // ## m4b-t10-service
    /// Removes the local mapping of the node `(profile, database)`; whether there was one.
    pub async fn unmap(
        &self,
        profile: ProfileId,
        database: DatabaseName,
    ) -> Result<bool, RegistryError> {
        self.ask(|reply| Msg::Unmap {
            profile,
            database,
            reply,
        })
        .await
        .unwrap_or(Err(STOPPED))
    }

    /// Drops every mapping of a deleted profile (R-M4b-2); how many.
    pub async fn drop_profile(&self, profile: ProfileId) -> Result<usize, RegistryError> {
        self.ask(|reply| Msg::DropProfile { profile, reply })
            .await
            .unwrap_or(Err(STOPPED))
    }
    // ## m4b-t10-service

    /// Reads and decodes a script for a tab under a connection whose environment is `env`.
    ///
    /// ## m4b-t05-fileio fix round 2 (named exception; I-fr1-1, ruling R-T5-6): the tab's
    /// environment is required, as in `fileio::read`, so Task 10 passes it through
    /// (`None` sees no `@`-scoped file).
    ///
    /// ## m4b-t10-service
    /// `fileio::read` on the blocking pool, with the default limits (a preview above 20 MiB).
    /// A library that is not registered or not available answers `Io(NotFound)`.
    pub async fn read(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
    ) -> Result<Decoded, ReadError> {
        let root = self
            .root_of(id)
            .ok_or(ReadError::Io(io::ErrorKind::NotFound))?;
        let path = rel.clone();
        let decoded = self
            .blocking(move || fileio::read(&root, &path, env, ReadLimits::default()))
            .await
            .unwrap_or(Err(ReadError::Io(io::ErrorKind::Interrupted)))?;
        self.tell(Msg::Known {
            id,
            rel: rel.clone(),
            hash: decoded.stamp.hash,
        })
        .await;
        Ok(decoded)
    }

    /// Saves a script (HV-SCRIPT-07) for a tab under `env`, recording the echo before the
    /// rename.
    ///
    /// ## m4b-t05-fileio fix round 2 (named exception; I-fr1-1, R-T5-6): `env` as in `read`.
    ///
    /// ## m4b-t10-service
    /// `fileio::save_with_echo` on the blocking pool; the echo is recorded before the new file
    /// replaces the old one, so the watcher's report of it announces nothing. The listing is
    /// patched before this returns.
    pub async fn save(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
        text: SaveText,
        expect: Option<DiskStamp>,
    ) -> Result<DiskStamp, SaveError> {
        let root = self
            .root_of(id)
            .ok_or(SaveError::Io(io::ErrorKind::NotFound))?;
        let (path, echo) = (rel.clone(), self.echo.clone());
        let saved = self
            .blocking(move || {
                fileio::save_with_echo(&root, &path, env, text, expect, &mut |stamp| {
                    echo.record(id, &path, *stamp);
                })
            })
            .await
            .unwrap_or(Err(SaveError::Io(io::ErrorKind::Interrupted)));
        let hash = match &saved {
            Ok(stamp) | Err(SaveError::SavedButPathChanged(stamp)) => Some(Some(stamp.hash)),
            Err(SaveError::Damaged) => Some(None),
            Err(_) => None,
        };
        if let Some(hash) = hash {
            self.tell(Msg::Touched {
                id,
                rel: rel.clone(),
                hash,
            })
            .await;
        }
        saved
    }

    /// Creates an empty script or a folder.
    ///
    /// Task 1a stub (fail closed): creates nothing. (Task 10: still a stub; the file operations
    /// are Task 13's.)
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
    /// Task 1a stub (fail closed): renames nothing. (Task 10: still a stub; Task 13's.)
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
    /// Task 1a stub (fail closed): moves nothing. (Task 10: still a stub; Task 13's.)
    pub async fn trash(&self, id: LibraryId, rel: &RelPath) -> Result<(), FileOpError> {
        let _ = (id, rel);
        Err(FileOpError::NotImplemented)
    }

    /// A chunked UTF-8 reader of a script for "Run as script" (Task 12's source), for a tab
    /// under `env`.
    ///
    /// ## m4b-t05-fileio fix round 2 (named exception; I-fr1-1, R-T5-6): `env` as in `read`.
    ///
    /// ## m4b-t10-service
    /// Async, and takes the file's `encoding` (the tab's `FileMeta::encoding`): opening the
    /// file is I/O, so it runs on the blocking pool, and `fileio::open_stream` decodes in the
    /// file's own encoding. The returned reader does blocking reads; drive it off the UI thread.
    pub async fn open_stream(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
        encoding: TextEncoding,
    ) -> Result<Box<dyn ScriptChunks>, ReadError> {
        let root = self
            .root_of(id)
            .ok_or(ReadError::Io(io::ErrorKind::NotFound))?;
        let path = rel.clone();
        self.blocking(move || fileio::open_stream(&root, &path, env, encoding))
            .await
            .unwrap_or(Err(ReadError::Io(io::ErrorKind::Interrupted)))
    }

    // ## m4b-t10-service
    /// Resolves once no changed file is waiting to settle (for tests: every `FileChanged` the
    /// hints so far will cause has been sent).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn settled(&self) {
        let _ = self.ask(|reply| Msg::Settled { reply }).await;
    }

    /// The root of an available library, from the current snapshot.
    fn root_of(&self, id: LibraryId) -> Option<LibraryRoot> {
        let snap = self.snapshot.load();
        let lib = snap.library(id)?;
        servable(&lib.state).then(|| lib.root.clone())
    }

    async fn ask<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Msg) -> Option<T> {
        let (reply, answer) = oneshot::channel();
        self.tx.send(make(reply)).await.ok()?;
        answer.await.ok()
    }

    async fn tell(&self, msg: Msg) {
        let _ = self.tx.send(msg).await;
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        self.rt.spawn_blocking(f).await.ok()
    }
    // ## m4b-t10-service
}

// ## m4b-t10-service

impl Drop for LibraryService {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.tx.try_send(Msg::Shutdown);
    }
}

/// What a registry call answers once the service has stopped.
const STOPPED: RegistryError = RegistryError::Io(io::ErrorKind::BrokenPipe);

/// Whether files of a library in `state` may be opened.
fn servable(state: &LibraryState) -> bool {
    matches!(
        state,
        LibraryState::Scanning | LibraryState::Watching | LibraryState::Polling(_)
    )
}

/// An event source for when the OS one cannot be built: nothing is watched, every library is
/// polled.
struct NoSource;

impl EventSource for NoSource {
    fn watch(&mut self, _root: &Path, _sink: HintSink) -> Result<(), WatchError> {
        Err(WatchError::Backend("no event source".into()))
    }

    fn unwatch(&mut self, _root: &Path) {}
}

/// The stamps of Hacivat's own saves, recorded before the rename (HV-SCRIPT-08 step 6). Written
/// by the save's blocking task, read by the actor; the lock is never held across an `.await`.
#[derive(Debug, Clone, Default)]
struct Echo(Arc<Mutex<EchoTable>>);

/// Each saved path's last own stamp and when it was recorded.
type EchoTable = HashMap<(LibraryId, RelPath), (DiskStamp, Instant)>;

impl Echo {
    fn lock(&self) -> std::sync::MutexGuard<'_, EchoTable> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, id: LibraryId, rel: &RelPath, stamp: DiskStamp) {
        self.lock()
            .insert((id, rel.clone()), (stamp, Instant::now()));
    }

    /// Whether `stamp` (len, mtime and hash) is one of our own writes of the last `ttl`.
    fn is_echo(&self, id: LibraryId, rel: &RelPath, stamp: &DiskStamp, ttl: Duration) -> bool {
        let mut table = self.lock();
        table.retain(|_, (_, at)| at.elapsed() <= ttl);
        table
            .get(&(id, rel.clone()))
            .is_some_and(|(own, _)| own == stamp)
    }

    fn forget(&self, id: LibraryId) {
        self.lock().retain(|(lib, _), _| *lib != id);
    }
}

/// The actor's messages.
enum Msg {
    Attach {
        path: PathBuf,
        db: Option<LogicalDb>,
        write_manifest: bool,
        reply: oneshot::Sender<Result<LibraryId, AttachError>>,
    },
    Detach {
        id: LibraryId,
        reply: oneshot::Sender<Result<(), RegistryError>>,
    },
    Map {
        m: LocalMapping,
        share_alias: bool,
        reply: oneshot::Sender<Result<(), MapError>>,
    },
    Unmap {
        profile: ProfileId,
        database: DatabaseName,
        reply: oneshot::Sender<Result<bool, RegistryError>>,
    },
    DropProfile {
        profile: ProfileId,
        reply: oneshot::Sender<Result<usize, RegistryError>>,
    },
    /// A batch from the event source.
    Hints {
        id: LibraryId,
        batch: Vec<RawHint>,
    },
    /// A tab read the file: its content hash is known (rename pairing).
    Known {
        id: LibraryId,
        rel: RelPath,
        hash: u128,
    },
    /// A save wrote the file (`hash` when it is known).
    Touched {
        id: LibraryId,
        rel: RelPath,
        hash: Option<u128>,
    },
    Settle {
        id: LibraryId,
        rel: RelPath,
    },
    /// The poll or recheck timer of a library.
    Tick {
        id: LibraryId,
        seq: u64,
    },
    GitTick {
        id: LibraryId,
        seq: u64,
    },
    GitDone {
        id: LibraryId,
        state: GitState,
    },
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    Settled {
        reply: oneshot::Sender<()>,
    },
    Shutdown,
}

/// A changed file waiting for its `(len, mtime)` to hold still.
#[derive(Debug, Clone, Copy)]
struct Settle {
    len: u64,
    mtime: SystemTime,
    tries: u32,
}

/// The repository status runs of one library.
#[derive(Debug, Default)]
struct GitRun {
    seq: u64,
    in_flight: bool,
    again: bool,
}

/// One registered library, as the actor keeps it.
struct Lib {
    reg: RegisteredLibrary,
    /// Its canonical root, once resolved (kept for the session: a root that resolves elsewhere
    /// later is `RootChanged`, never adopted).
    root: Option<LibraryRoot>,
    state: LibraryState,
    db: LogicalDb,
    aliases: Arc<[LogicalDb]>,
    manifest_diags: Vec<LibraryDiag>,
    scan_diags: Vec<LibraryDiag>,
    entries: Vec<ScriptEntry>,
    stopped: bool,
    watched: Option<PathBuf>,
    /// Set by the source's sink when a batch did not fit the inbox.
    overflow: Arc<AtomicBool>,
    /// The poll or recheck timer chain that is live, if any.
    timer: Option<u64>,
    next_timer: u64,
    git: GitRun,
    snap: Arc<LibrarySnapshot>,
}

impl Lib {
    fn new(reg: RegisteredLibrary) -> Self {
        let root = LibraryRoot {
            id: reg.id,
            path: reg.path.clone(),
            canonical: reg.path.clone(),
        };
        let db = folder_db(&reg.path).unwrap_or_else(LogicalDb::unbindable);
        let snap = Arc::new(LibrarySnapshot {
            generation: 0,
            id: reg.id,
            root,
            db: db.clone(),
            aliases: Arc::from([]),
            entries: Arc::from([]),
            state: LibraryState::Scanning,
            diagnostics: Arc::from([]),
        });
        Lib {
            reg,
            root: None,
            state: LibraryState::Scanning,
            db,
            aliases: Arc::from([]),
            manifest_diags: Vec::new(),
            scan_diags: Vec::new(),
            entries: Vec::new(),
            stopped: false,
            watched: None,
            overflow: Arc::new(AtomicBool::new(false)),
            timer: None,
            next_timer: 0,
            git: GitRun::default(),
            snap,
        }
    }

    fn id(&self) -> LibraryId {
        self.reg.id
    }

    fn entry(&self, rel: &RelPath) -> Option<&ScriptEntry> {
        self.entries.iter().find(|e| &e.rel == rel)
    }

    fn snapshot(&self, generation: u64) -> Arc<LibrarySnapshot> {
        let root = self.root.clone().unwrap_or_else(|| LibraryRoot {
            id: self.reg.id,
            path: self.reg.path.clone(),
            canonical: self.reg.path.clone(),
        });
        let diagnostics: Vec<LibraryDiag> = self
            .manifest_diags
            .iter()
            .chain(&self.scan_diags)
            .cloned()
            .collect();
        Arc::new(LibrarySnapshot {
            generation,
            id: self.reg.id,
            root,
            db: self.db.clone(),
            aliases: self.aliases.clone(),
            entries: Arc::from(self.entries.as_slice()),
            state: self.state.clone(),
            diagnostics: Arc::from(diagnostics),
        })
    }
}

/// The logical name the folder name gives, when it is a valid one.
fn folder_db(path: &Path) -> Option<LogicalDb> {
    LogicalDb::parse(path.file_name()?.to_str()?).ok()
}

impl LogicalDb {
    /// The name of a library that binds nothing automatically: empty, which no parsed name
    /// (and so no database node) ever equals.
    fn unbindable() -> Self {
        LogicalDb(Arc::from(""))
    }
}

/// Where a hinted path lands in a library.
#[derive(Debug, PartialEq, Eq)]
enum HintPath {
    /// The root itself: rescan.
    Root,
    /// A file inside the repository's `.git` that changes the status.
    Git,
    /// `.hacivat/`: the manifest may have changed.
    Manifest,
    Rel(RelPath),
    /// Outside the root, under a skipped name, or not a name the scan could list.
    Ignore,
}

fn classify(root: &LibraryRoot, path: &Path) -> HintPath {
    let Some(rest) = path
        .strip_prefix(&root.canonical)
        .or_else(|_| path.strip_prefix(&root.path))
        .ok()
    else {
        return HintPath::Ignore;
    };
    let mut parts = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(name) => match name.to_str() {
                Some(name) => parts.push(name),
                None => return HintPath::Ignore,
            },
            _ => return HintPath::Ignore,
        }
    }
    match parts.as_slice() {
        [] => HintPath::Root,
        [".git", "index" | "HEAD" | "packed-refs", ..] | [".git", "refs", ..] => HintPath::Git,
        [".hacivat", ..] => HintPath::Manifest,
        _ if parts.iter().any(|p| pathsafe::is_skipped(p)) => HintPath::Ignore,
        _ => RelPath::parse(&parts.join("/")).map_or(HintPath::Ignore, HintPath::Rel),
    }
}

/// What is at a hinted path now, through `pathsafe::join` (no link followed).
#[derive(Debug)]
enum Probe {
    Absent,
    /// A plain regular file, with its `lstat`.
    File(scan::Meta),
    /// A folder, a link, or something odd: rescan the library.
    Rescan,
    /// A name the scan never lists.
    Ignore,
}

fn probe(root: &LibraryRoot, rel: &RelPath) -> Probe {
    match pathsafe::join(root, rel) {
        Ok(j) => match j.kind {
            JoinedKind::Absent => Probe::Absent,
            JoinedKind::File if j.target.is_none() => match RealFs.symlink_metadata(&j.path) {
                Ok(meta) if meta.kind == MetaKind::File => Probe::File(meta),
                _ => Probe::Rescan,
            },
            JoinedKind::File | JoinedKind::Folder => Probe::Rescan,
        },
        Err(PathError::Name(_)) => Probe::Ignore,
        Err(_) => Probe::Rescan,
    }
}

/// An environment the entry is shown under (`None` first: an unscoped file), so `fileio`'s
/// scope check admits the service's own stamp; `None` when it is shown nowhere.
fn admitting(entry: &ScriptEntry) -> Option<Option<EnvTag>> {
    std::iter::once(None)
        .chain(EnvTag::ALL.into_iter().map(Some))
        .find(|env| envscope::visible(entry, *env))
}

/// Whether two status answers are the same (nothing to publish).
fn same_git(a: &GitState, b: &GitState) -> bool {
    match (a, b) {
        (GitState::NotARepo, GitState::NotARepo) => true,
        (GitState::Untrusted(x), GitState::Untrusted(y)) => x == y,
        (GitState::Unavailable(x), GitState::Unavailable(y)) => x == y,
        (GitState::Ready(x), GitState::Ready(y)) => x == y,
        _ => false,
    }
}

/// What a listing change was, for the events.
#[derive(Debug, Default)]
struct Changes {
    /// Files gone, with their length (rename pairing).
    removed: Vec<(RelPath, u64)>,
    created: Vec<RelPath>,
    changed: Vec<RelPath>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.created.is_empty() && self.changed.is_empty()
    }
}

/// The file changes between two listings (folders are not announced).
fn diff(old: &[ScriptEntry], new: &[ScriptEntry]) -> Changes {
    let file = |e: &&ScriptEntry| e.kind != EntryKind::Folder;
    let before: HashMap<&RelPath, &ScriptEntry> =
        old.iter().filter(file).map(|e| (&e.rel, e)).collect();
    let after: HashMap<&RelPath, &ScriptEntry> =
        new.iter().filter(file).map(|e| (&e.rel, e)).collect();
    let mut changes = Changes::default();
    for e in old.iter().filter(file) {
        if !after.contains_key(&e.rel) {
            changes.removed.push((e.rel.clone(), e.len));
        }
    }
    for e in new.iter().filter(file) {
        match before.get(&e.rel) {
            None => changes.created.push(e.rel.clone()),
            Some(o) if o != &e => changes.changed.push(e.rel.clone()),
            Some(_) => {}
        }
    }
    changes
}

struct Actor {
    paths: LocalPaths,
    rt: Handle,
    opts: ServiceOptions,
    tx: mpsc::Sender<Msg>,
    closed: Arc<AtomicBool>,
    snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    events: broadcast::Sender<LibraryEvent>,
    echo: Echo,
    source: Box<dyn EventSource>,
    registry: LibraryRegistry,
    registry_diags: Arc<[RegistryDiag]>,
    libs: Vec<Lib>,
    git: HashMap<LibraryId, GitState>,
    generation: u64,
    loaded: bool,
    /// Libraries whose snapshot must be rebuilt at the next publish.
    dirty: HashSet<LibraryId>,
    /// Something outside the libraries changed (mappings, status, the service state).
    workspace_dirty: bool,
    /// Events to send after the next publish.
    pending: Vec<LibraryEvent>,
    /// Content hashes known from reads, saves and stamps (rename pairing).
    known: HashMap<(LibraryId, RelPath), u128>,
    settle: HashMap<(LibraryId, RelPath), Settle>,
    /// Paths announced deleted, whose return is announced as a change.
    deleted: HashMap<LibraryId, HashSet<RelPath>>,
    waiters: Vec<oneshot::Sender<()>>,
}

impl Actor {
    fn run(mut self, mut inbox: mpsc::Receiver<Msg>, diags: Vec<RegistryDiag>) {
        self.load(diags);
        self.flush();
        'run: while let Some(first) = inbox.blocking_recv() {
            let mut next = Some(first);
            let mut handled = 0;
            while let Some(msg) = next.take() {
                if matches!(msg, Msg::Shutdown) || self.closed.load(Ordering::Acquire) {
                    break 'run;
                }
                self.handle(msg);
                handled += 1;
                if handled < DRAIN_MAX {
                    next = inbox.try_recv().ok();
                }
            }
            self.check_overflow();
            self.flush();
        }
        for lib in &mut self.libs {
            if let Some(root) = lib.watched.take() {
                self.source.unwatch(&root);
            }
        }
    }

    // ---- launch ------------------------------------------------------------------------------

    /// The launch: sweeps stale recovery folders (only over a registry read cleanly) and brings
    /// every registered library up.
    fn load(&mut self, diags: Vec<RegistryDiag>) {
        let registry = &self.registry;
        let trustworthy = !registry.is_read_only()
            && !diags.iter().any(|d| {
                matches!(
                    d,
                    RegistryDiag::SetAside(_)
                        | RegistryDiag::QuarantineFailed(_)
                        | RegistryDiag::NewerVersion(_)
                        | RegistryDiag::Unreadable(_)
                        | RegistryDiag::NotAFile
                )
            });
        if trustworthy {
            recovery::sweep_unregistered(&self.paths, &|id| registry.library(id).is_some());
        }
        let regs = registry.libraries().to_vec();
        self.registry_diags = Arc::from(diags);
        for reg in regs {
            self.libs.push(Lib::new(reg));
            self.bring_up(self.libs.len() - 1);
        }
        self.loaded = true;
        self.workspace_dirty = true;
    }

    // ---- messages ----------------------------------------------------------------------------

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Attach {
                path,
                db,
                write_manifest,
                reply,
            } => {
                let r = self.attach(&path, db, write_manifest);
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Detach { id, reply } => {
                let r = self.detach(id);
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Map {
                m,
                share_alias,
                reply,
            } => {
                let r = self.map(m, share_alias);
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Unmap {
                profile,
                database,
                reply,
            } => {
                let r = self.edit_registry(|reg| Ok(reg.remove_mapping(profile, &database)));
                self.flush();
                let _ = reply.send(r);
            }
            Msg::DropProfile { profile, reply } => {
                let r = self.edit_registry(|reg| Ok(reg.drop_profile(profile)));
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Hints { id, batch } => self.hints(id, batch),
            Msg::Known { id, rel, hash } => {
                self.known.insert((id, rel), hash);
            }
            Msg::Touched { id, rel, hash } => {
                if let Some(hash) = hash {
                    self.known.insert((id, rel.clone()), hash);
                }
                if let Some(i) = self.index(id) {
                    self.probe_paths(i, vec![rel]);
                    self.git_trigger(i);
                }
            }
            Msg::Settle { id, rel } => self.settle_check(id, rel),
            Msg::Tick { id, seq } => self.tick(id, seq),
            Msg::GitTick { id, seq } => self.git_tick(id, seq),
            Msg::GitDone { id, state } => self.git_done(id, state),
            Msg::Settled { reply } => self.waiters.push(reply),
            Msg::Shutdown => {}
        }
    }

    fn index(&self, id: LibraryId) -> Option<usize> {
        self.libs.iter().position(|l| l.id() == id)
    }

    // ---- registry ----------------------------------------------------------------------------

    fn attach(
        &mut self,
        path: &Path,
        db: Option<LogicalDb>,
        write_manifest: bool,
    ) -> Result<LibraryId, AttachError> {
        let guard = AttachGuard::new(&self.paths, self.opts.home.as_deref());
        let root = self
            .registry
            .add(path, &guard, SystemTime::now())
            .map_err(AttachError::Registry)?;
        let id = root.id;
        let undo = |reg: &mut LibraryRegistry| {
            let _ = reg.remove(id);
        };
        if let Some(name) = envscope::root_env_folder(&root.canonical) {
            undo(&mut self.registry);
            return Err(AttachError::RootInEnvFolder(name));
        }
        if write_manifest {
            let Some(db) = db.or_else(|| folder_db(path)) else {
                undo(&mut self.registry);
                return Err(AttachError::NoLogicalName);
            };
            match manifest::write_new(&root.canonical, &ManifestV1::new(db, Vec::new())) {
                Ok(()) | Err(manifest::WriteError::Exists) => {}
                Err(e) => {
                    undo(&mut self.registry);
                    return Err(AttachError::Manifest(e));
                }
            }
        }
        if let Err(e) = self.registry.save() {
            self.reload_registry();
            return Err(AttachError::Registry(e));
        }
        let Some(reg) = self.registry.library(id).cloned() else {
            return Err(AttachError::Registry(RegistryError::UnknownLibrary));
        };
        let mut lib = Lib::new(reg);
        lib.root = Some(root);
        self.libs.push(lib);
        self.bring_up(self.libs.len() - 1);
        Ok(id)
    }

    fn detach(&mut self, id: LibraryId) -> Result<(), RegistryError> {
        self.edit_registry(|reg| reg.remove(id))?;
        if let Some(i) = self.index(id) {
            let mut lib = self.libs.remove(i);
            if let Some(root) = lib.watched.take() {
                self.source.unwatch(&root);
            }
        }
        self.git.remove(&id);
        self.settle.retain(|(lib, _), _| *lib != id);
        self.known.retain(|(lib, _), _| *lib != id);
        self.deleted.remove(&id);
        self.echo.forget(id);
        // R-T5-4: the library's unsaved copies go with it (none is not an error).
        let _ = recovery::discard_library(&self.paths, id);
        self.dirty.remove(&id);
        self.workspace_dirty = true;
        self.pending.push(LibraryEvent::Changed {
            id,
            generation: self.generation + 1,
        });
        Ok(())
    }

    fn map(&mut self, m: LocalMapping, share_alias: bool) -> Result<(), MapError> {
        if self.registry.library(m.library).is_none() {
            return Err(MapError::Registry(RegistryError::UnknownLibrary));
        }
        if share_alias {
            let alias = LogicalDb::parse(m.database.as_str()).map_err(MapError::BadName)?;
            let i = self
                .index(m.library)
                .ok_or(MapError::Registry(RegistryError::UnknownLibrary))?;
            let root = match (&self.libs[i].root, servable(&self.libs[i].state)) {
                (Some(root), true) => root.canonical.clone(),
                _ => return Err(MapError::Unavailable),
            };
            manifest::add_alias(&root, &alias).map_err(MapError::Alias)?;
            self.read_manifest(i);
        }
        let library = m.library;
        self.edit_registry(|reg| reg.set_mapping(m))
            .map_err(MapError::Registry)?;
        self.dirty.insert(library);
        Ok(())
    }

    /// Applies `edit` to the registry and saves it; on a failed save the registry is read back
    /// from the file, which the failed atomic write left as it was.
    fn edit_registry<T>(
        &mut self,
        edit: impl FnOnce(&mut LibraryRegistry) -> Result<T, RegistryError>,
    ) -> Result<T, RegistryError> {
        let out = edit(&mut self.registry)?;
        if let Err(e) = self.registry.save() {
            self.reload_registry();
            return Err(e);
        }
        self.workspace_dirty = true;
        Ok(out)
    }

    fn reload_registry(&mut self) {
        let (registry, _) = LibraryRegistry::load(&self.paths.libraries_toml());
        self.registry = registry;
    }

    // ---- availability ------------------------------------------------------------------------

    /// Resolves, reads and scans library `i`, and starts watching it.
    fn bring_up(&mut self, i: usize) {
        if self.libs[i].root.is_none() {
            match self.libs[i].reg.root() {
                Ok(root) => self.libs[i].root = Some(root),
                Err(e) => {
                    let reason = if e.kind() == io::ErrorKind::NotFound {
                        UnavailableReason::Missing
                    } else {
                        UnavailableReason::Unreadable(e.kind())
                    };
                    self.make_unavailable(i, reason, Vec::new());
                    return;
                }
            }
        }
        self.rescan(i);
    }

    fn make_unavailable(&mut self, i: usize, reason: UnavailableReason, diags: Vec<LibraryDiag>) {
        let id = self.libs[i].id();
        let lib = &mut self.libs[i];
        if let Some(root) = lib.watched.take() {
            self.source.unwatch(&root);
        }
        let state = LibraryState::Unavailable(reason);
        let changed = lib.state != state || !lib.entries.is_empty() || lib.scan_diags != diags;
        lib.state = state;
        lib.entries.clear();
        lib.scan_diags = diags;
        lib.stopped = false;
        self.settle.retain(|(l, _), _| *l != id);
        if self.git.remove(&id).is_some() {
            self.workspace_dirty = true;
        }
        if changed {
            self.dirty.insert(id);
        }
        self.arm_timer(i, self.opts.recheck);
    }

    /// Starts watching library `i` (else polls it).
    fn ensure_watch(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        if lib.watched.is_some() || matches!(lib.state, LibraryState::Polling(_)) {
            return;
        }
        let Some(root) = lib.root.as_ref().map(|r| r.canonical.clone()) else {
            return;
        };
        let (tx, overflow, id) = (self.tx.clone(), lib.overflow.clone(), lib.id());
        let sink: HintSink = Box::new(move |batch| {
            if tx.try_send(Msg::Hints { id, batch }).is_err() {
                overflow.store(true, Ordering::Release);
            }
        });
        match self.source.watch(&root, sink) {
            Ok(()) => {
                lib.watched = Some(root);
                lib.state = LibraryState::Watching;
                lib.timer = None;
            }
            Err(_) => {
                lib.state = LibraryState::Polling(PollReason::BackendLost);
                let poll = self.opts.poll;
                self.arm_timer(i, poll);
            }
        }
        self.dirty.insert(self.libs[i].id());
    }

    /// The event source of library `i` stopped: poll it from now on.
    fn lose_backend(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        if let Some(root) = lib.watched.take() {
            self.source.unwatch(&root);
        }
        lib.state = LibraryState::Polling(PollReason::BackendLost);
        self.dirty.insert(lib.id());
        let poll = self.opts.poll;
        self.arm_timer(i, poll);
    }

    /// Starts a new timer chain for library `i` (the old one, if any, is ignored from now on).
    fn arm_timer(&mut self, i: usize, after: Duration) {
        let lib = &mut self.libs[i];
        lib.next_timer += 1;
        let seq = lib.next_timer;
        lib.timer = Some(seq);
        let id = lib.id();
        self.after(after, Msg::Tick { id, seq });
    }

    fn tick(&mut self, id: LibraryId, seq: u64) {
        let Some(i) = self.index(id) else { return };
        if self.libs[i].timer != Some(seq) {
            return;
        }
        self.libs[i].timer = None;
        match self.libs[i].state {
            LibraryState::Polling(_) => {
                self.rescan(i);
                if matches!(self.libs[i].state, LibraryState::Polling(_)) {
                    let poll = self.opts.poll;
                    self.arm_timer(i, poll);
                }
                self.git_trigger_if_changed(i);
            }
            LibraryState::Unavailable(_) => self.bring_up(i),
            LibraryState::Scanning | LibraryState::Watching => {}
        }
    }

    // ---- listing -----------------------------------------------------------------------------

    fn read_manifest(&mut self, i: usize) {
        let Some(root) = self.libs[i].root.clone() else {
            return;
        };
        let read = manifest::read(&root.canonical);
        let folder = root.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let mut diags: Vec<LibraryDiag> = read
            .diags
            .iter()
            .cloned()
            .map(LibraryDiag::Manifest)
            .collect();
        let db = read.logical(folder).unwrap_or_else(|| {
            diags.push(LibraryDiag::NoLogicalName);
            LogicalDb::unbindable()
        });
        let lib = &mut self.libs[i];
        let aliases: Arc<[LogicalDb]> = Arc::from(read.aliases);
        // Exact text, not `same_db`: a change of spelling is a change of what is shown.
        let text = |names: &[LogicalDb]| {
            names
                .iter()
                .map(|n| n.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        if lib.db.as_str() != db.as_str()
            || text(&lib.aliases) != text(&aliases)
            || lib.manifest_diags != diags
        {
            lib.db = db;
            lib.aliases = aliases;
            lib.manifest_diags = diags;
            self.dirty.insert(lib.id());
        }
    }

    /// Rescans library `i` and announces what changed.
    fn rescan(&mut self, i: usize) {
        let Some(root) = self.libs[i].root.clone() else {
            return;
        };
        let result = scan::scan(&root, &self.opts.limits, &RealFs);
        if let Some(problem) = result.root {
            let (reason, diags) = match problem {
                scan::RootProblem::Missing => (UnavailableReason::Missing, Vec::new()),
                scan::RootProblem::Changed => (UnavailableReason::RootChanged, Vec::new()),
                scan::RootProblem::Unreadable(kind) => {
                    (UnavailableReason::Unreadable(kind), Vec::new())
                }
                scan::RootProblem::InEnvFolder(name) => (
                    UnavailableReason::InEnvFolder,
                    vec![LibraryDiag::RootInEnvFolder(name)],
                ),
            };
            self.make_unavailable(i, reason, diags);
            return;
        }
        self.read_manifest(i);
        let mut diags: Vec<LibraryDiag> = result.diags.into_iter().map(LibraryDiag::Scan).collect();
        diags.extend(result.stopped.map(LibraryDiag::ScanStopped));
        if result.suppressed > 0 {
            diags.push(LibraryDiag::Suppressed(result.suppressed));
        }
        let lib = &mut self.libs[i];
        let changes = diff(&lib.entries, &result.entries);
        let was_up = matches!(lib.state, LibraryState::Watching | LibraryState::Polling(_));
        if !changes.is_empty() || lib.entries != result.entries || lib.scan_diags != diags {
            self.dirty.insert(lib.id());
        }
        lib.entries = result.entries;
        lib.scan_diags = diags;
        lib.stopped = result.stopped.is_some();
        if !was_up {
            self.ensure_watch(i);
            self.git_trigger_now(i);
        }
        self.announce(i, changes);
    }

    /// Handles one batch of hints for library `id`.
    fn hints(&mut self, id: LibraryId, batch: Vec<RawHint>) {
        let Some(i) = self.index(id) else { return };
        if !matches!(
            self.libs[i].state,
            LibraryState::Watching | LibraryState::Polling(_)
        ) {
            return;
        }
        let Some(root) = self.libs[i].root.clone() else {
            return;
        };
        let (mut rescan, mut git, mut manifest) = (false, false, false);
        let mut paths = Vec::new();
        for hint in batch {
            match hint {
                RawHint::Lost => {
                    self.lose_backend(i);
                    rescan = true;
                }
                RawHint::Rescan(_) => rescan = true,
                RawHint::Path(p) => match classify(&root, &p) {
                    HintPath::Root => rescan = true,
                    HintPath::Git => git = true,
                    HintPath::Manifest => manifest = true,
                    HintPath::Rel(rel) => paths.push(rel),
                    HintPath::Ignore => {}
                },
            }
        }
        if manifest {
            self.read_manifest(i);
        }
        let before = self.dirty.contains(&id);
        if rescan || paths.len() > BURST_PATHS {
            self.rescan(i);
        } else if !paths.is_empty() {
            self.probe_paths(i, paths);
        }
        let changed =
            (!before && self.dirty.contains(&id)) || self.settle.keys().any(|k| k.0 == id);
        if git || changed {
            self.git_trigger(i);
        }
    }

    /// Patches the listing of library `i` for the hinted `paths`, or rescans it when one of
    /// them is not a plain script file inside a listed folder.
    fn probe_paths(&mut self, i: usize, mut paths: Vec<RelPath>) {
        let Some(root) = self.libs[i].root.clone() else {
            return;
        };
        paths.sort();
        paths.dedup();
        enum Step {
            Remove(RelPath),
            Update(RelPath, scan::Meta),
            Insert(ScriptEntry),
        }
        let max_depth = self.opts.limits.max_depth;
        let max_files = self.opts.limits.max_files;
        let lib = &self.libs[i];
        let mut steps = Vec::new();
        let mut rescan = false;
        let mut files = lib
            .entries
            .iter()
            .filter(|e| e.kind != EntryKind::Folder)
            .count();
        for rel in paths {
            let known = lib.entry(&rel);
            match probe(&root, &rel) {
                Probe::Ignore => {}
                Probe::Rescan => rescan = true,
                Probe::Absent => match known {
                    Some(e) if e.kind == EntryKind::Folder => rescan = true,
                    Some(_) => steps.push(Step::Remove(rel)),
                    None => {}
                },
                Probe::File(meta) => {
                    let Some(kind) = scan::script_kind(rel.name()) else {
                        continue;
                    };
                    match known {
                        Some(e) if e.link != LinkState::Plain || e.kind != kind => rescan = true,
                        Some(e) => {
                            if (e.len, e.mtime, e.dataless) != (meta.len, meta.mtime, meta.dataless)
                            {
                                steps.push(Step::Update(rel, meta));
                            }
                        }
                        None => {
                            let parent = rel.as_str().rsplit_once('/').map(|(p, _)| p);
                            let parent_listed = parent.is_none_or(|p| {
                                lib.entries
                                    .iter()
                                    .any(|e| e.kind == EntryKind::Folder && e.rel.as_str() == p)
                            });
                            let depth = rel.components().count();
                            if !parent_listed
                                || depth > max_depth
                                || lib.stopped
                                || files >= max_files
                            {
                                rescan = true;
                            } else {
                                files += 1;
                                steps.push(Step::Insert(ScriptEntry {
                                    scope: envscope::entry_scope(&rel, kind),
                                    rel,
                                    kind,
                                    link: LinkState::Plain,
                                    dataless: meta.dataless,
                                    len: meta.len,
                                    mtime: meta.mtime,
                                    target: None,
                                }));
                            }
                        }
                    }
                }
            }
            if rescan {
                break;
            }
        }
        if rescan {
            self.rescan(i);
            return;
        }
        if steps.is_empty() {
            return;
        }
        let lib = &mut self.libs[i];
        let mut changes = Changes::default();
        let mut sort = false;
        for step in steps {
            match step {
                Step::Remove(rel) => {
                    if let Some(pos) = lib.entries.iter().position(|e| e.rel == rel) {
                        let e = lib.entries.remove(pos);
                        changes.removed.push((rel, e.len));
                    }
                }
                Step::Update(rel, meta) => {
                    if let Some(e) = lib.entries.iter_mut().find(|e| e.rel == rel) {
                        e.len = meta.len;
                        e.mtime = meta.mtime;
                        e.dataless = meta.dataless;
                        changes.changed.push(rel);
                    }
                }
                Step::Insert(entry) => {
                    changes.created.push(entry.rel.clone());
                    lib.entries.push(entry);
                    sort = true;
                }
            }
        }
        if sort {
            tree::sort(&mut lib.entries);
        }
        // A folder left with no script is not listed: let the scan decide.
        let emptied = changes.removed.iter().any(|(rel, _)| {
            rel.as_str().rsplit_once('/').is_some_and(|(parent, _)| {
                !lib.entries.iter().any(|e| {
                    e.kind != EntryKind::Folder && e.rel.as_str().starts_with(&format!("{parent}/"))
                })
            })
        });
        self.dirty.insert(lib.id());
        self.announce(i, changes);
        if emptied {
            self.rescan(i);
        }
    }

    /// Turns listing changes into events: renames paired by content hash, deletes, and the
    /// changed (or returned) files queued to settle.
    fn announce(&mut self, i: usize, changes: Changes) {
        let id = self.libs[i].id();
        let Changes {
            removed,
            mut created,
            changed,
        } = changes;
        for (from, len) in removed {
            let pair = self
                .known
                .get(&(id, from.clone()))
                .copied()
                .and_then(|hash| {
                    let pos = created.iter().position(|to| {
                        self.libs[i]
                            .entry(to)
                            .is_some_and(|e| e.len == len && !e.dataless)
                            && self.stamp(i, to).is_some_and(|s| s.hash == hash)
                    })?;
                    Some(created.remove(pos))
                });
            self.known.remove(&(id, from.clone()));
            self.settle.remove(&(id, from.clone()));
            match pair {
                Some(to) => {
                    if let Some(hash) = self.libs[i]
                        .entry(&to)
                        .and_then(|_| self.stamp(i, &to))
                        .map(|s| s.hash)
                    {
                        self.known.insert((id, to.clone()), hash);
                    }
                    self.pending
                        .push(LibraryEvent::FileRenamed { id, from, to });
                }
                None => {
                    let recent = self.deleted.entry(id).or_default();
                    if recent.len() >= RECENT_DELETES {
                        recent.clear();
                    }
                    recent.insert(from.clone());
                    self.pending
                        .push(LibraryEvent::FileDeleted { id, rel: from });
                }
            }
        }
        for rel in created {
            let returned = self
                .deleted
                .get_mut(&id)
                .is_some_and(|recent| recent.remove(&rel));
            if returned {
                self.queue_settle(i, rel);
            }
        }
        for rel in changed {
            self.queue_settle(i, rel);
        }
    }

    /// The current stamp of a listed file, through `fileio` (never a placeholder, never a file
    /// shown nowhere).
    fn stamp(&self, i: usize, rel: &RelPath) -> Option<DiskStamp> {
        let lib = &self.libs[i];
        let entry = lib.entry(rel)?;
        if entry.dataless {
            return None;
        }
        let env = admitting(entry)?;
        fileio::stamp_of(lib.root.as_ref()?, rel, env).ok()
    }

    // ---- settle ------------------------------------------------------------------------------

    fn queue_settle(&mut self, i: usize, rel: RelPath) {
        let id = self.libs[i].id();
        let Some(e) = self.libs[i].entry(&rel) else {
            return;
        };
        let (len, mtime) = (e.len, e.mtime);
        let key = (id, rel.clone());
        match self.settle.get_mut(&key) {
            // Already waiting: its timer will compare against the newest metadata.
            Some(s) => {
                s.len = len;
                s.mtime = mtime;
            }
            None => {
                self.settle.insert(
                    key,
                    Settle {
                        len,
                        mtime,
                        tries: 0,
                    },
                );
                self.after(self.opts.settle, Msg::Settle { id, rel });
            }
        }
    }

    fn settle_check(&mut self, id: LibraryId, rel: RelPath) {
        let Some(mut s) = self.settle.remove(&(id, rel.clone())) else {
            return;
        };
        let Some(i) = self.index(id) else { return };
        let Some(root) = self.libs[i].root.clone() else {
            return;
        };
        if !servable(&self.libs[i].state) {
            return;
        }
        let Probe::File(meta) = probe(&root, &rel) else {
            // Gone or no longer a plain file: the hint that says so patches the listing.
            return;
        };
        if let Some(e) = self.libs[i].entries.iter_mut().find(|e| e.rel == rel)
            && (e.len, e.mtime, e.dataless) != (meta.len, meta.mtime, meta.dataless)
        {
            e.len = meta.len;
            e.mtime = meta.mtime;
            e.dataless = meta.dataless;
            self.dirty.insert(id);
        }
        let still = (s.len, s.mtime) == (meta.len, meta.mtime);
        if !still && s.tries + 1 < SETTLE_TRIES {
            s.len = meta.len;
            s.mtime = meta.mtime;
            s.tries += 1;
            self.settle.insert((id, rel.clone()), s);
            self.after(self.opts.settle, Msg::Settle { id, rel });
            return;
        }
        let Some(stamp) = self.stamp(i, &rel) else {
            return;
        };
        self.known.insert((id, rel.clone()), stamp.hash);
        if self.echo.is_echo(id, &rel, &stamp, self.opts.echo_ttl) {
            return;
        }
        self.pending
            .push(LibraryEvent::FileChanged { id, rel, stamp });
    }

    // ---- repository status -------------------------------------------------------------------

    /// Refreshes the status of library `i` after the debounce.
    fn git_trigger(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        lib.git.seq += 1;
        let msg = Msg::GitTick {
            id: lib.id(),
            seq: lib.git.seq,
        };
        self.after(self.opts.git_debounce, msg);
    }

    fn git_trigger_if_changed(&mut self, i: usize) {
        let id = self.libs[i].id();
        if self.dirty.contains(&id) {
            self.git_trigger(i);
        }
    }

    /// Refreshes the status of library `i` now (the first one after it comes up).
    fn git_trigger_now(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        lib.git.seq += 1;
        let (id, seq) = (lib.id(), lib.git.seq);
        self.git_tick(id, seq);
    }

    fn git_tick(&mut self, id: LibraryId, seq: u64) {
        let Some(i) = self.index(id) else { return };
        let lib = &mut self.libs[i];
        if lib.git.seq != seq || !servable(&lib.state) {
            return;
        }
        if lib.git.in_flight {
            lib.git.again = true;
            return;
        }
        let Some(root) = lib.root.as_ref().map(|r| r.canonical.clone()) else {
            return;
        };
        lib.git.in_flight = true;
        lib.git.again = false;
        let (tx, limits) = (self.tx.clone(), self.opts.git);
        self.rt.spawn_blocking(move || {
            let state = hacivat_git::status(&root, &limits);
            let _ = tx.blocking_send(Msg::GitDone { id, state });
        });
    }

    fn git_done(&mut self, id: LibraryId, state: GitState) {
        let Some(i) = self.index(id) else { return };
        let lib = &mut self.libs[i];
        lib.git.in_flight = false;
        let again = std::mem::take(&mut lib.git.again);
        if servable(&lib.state) {
            let same = self.git.get(&id).is_some_and(|old| same_git(old, &state));
            if !same {
                self.git.insert(id, state);
                self.workspace_dirty = true;
                self.pending.push(LibraryEvent::GitChanged { id });
            }
        }
        if again {
            self.git_trigger(i);
        }
    }

    // ---- plumbing ----------------------------------------------------------------------------

    /// Rescans every library whose source dropped a batch (the inbox was full).
    fn check_overflow(&mut self) {
        for i in 0..self.libs.len() {
            if self.libs[i].overflow.swap(false, Ordering::AcqRel)
                && matches!(
                    self.libs[i].state,
                    LibraryState::Watching | LibraryState::Polling(_)
                )
            {
                self.rescan(i);
                self.git_trigger_if_changed(i);
            }
        }
    }

    fn after(&self, delay: Duration, msg: Msg) {
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(msg).await;
        });
    }

    /// Publishes one new snapshot if anything changed, then sends the events.
    fn flush(&mut self) {
        if !self.dirty.is_empty() || self.workspace_dirty {
            self.generation += 1;
            let generation = self.generation;
            let mut changed = Vec::new();
            for lib in &mut self.libs {
                if self.dirty.contains(&lib.id()) {
                    lib.snap = lib.snapshot(generation);
                    changed.push(lib.id());
                }
            }
            let snap = WorkspaceSnapshot {
                generation,
                state: if self.loaded {
                    LibraryState::Watching
                } else {
                    LibraryState::Scanning
                },
                libraries: self.libs.iter().map(|l| l.snap.clone()).collect(),
                mappings: Arc::from(self.registry.mappings()),
                git: self.git.clone(),
                registry: self.registry_diags.clone(),
            };
            self.snapshot.store(Arc::new(snap));
            self.dirty.clear();
            self.workspace_dirty = false;
            for id in changed {
                let _ = self.events.send(LibraryEvent::Changed { id, generation });
            }
        }
        for event in self.pending.drain(..) {
            let _ = self.events.send(event);
        }
        if self.settle.is_empty() {
            for waiter in self.waiters.drain(..) {
                let _ = waiter.send(());
            }
        }
    }
}
// ## m4b-t10-service
