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
//! Task 10 (ADR 0009 `m4b-t10`, with its fix rounds 1 and 2 and the watcher wiring).
//!
//! - **One actor, bounded workers** (I6, R-T10-4). [`LibraryService::open`] starts one actor on
//!   the core runtime's blocking pool. It alone owns the [`LibraryRegistry`](crate::registry::
//!   LibraryRegistry) (the single writer of `libraries.toml`), every library's state, the event
//!   source and the status refresh, and handles one message at a time from a bounded inbox. It
//!   does no library I/O itself (F1): validating a root, starting its watcher, scanning,
//!   probing, stamps and the attach and alias writes run as jobs on at most
//!   [`ServiceOptions::workers`] blocking workers, one job per library at a time
//!   (`service/work.rs`). A hung mount stalls its own library only: a job past
//!   [`ServiceOptions::job_deadline`] stops counting and shows it `NotResponding` (F2,
//!   R-T10-5), commands keep answering, and detach frees the worker at once.
//! - **Task 9's watcher** (R-C0-7). Each library has its own `Watcher`, started on a worker
//!   before its first scan. Its events (settled, echo-filtered, renames paired) patch the
//!   listing through probes; a batch from a dropped watcher is ignored (a generation tag). A
//!   save records its echo in that watcher's table under the path the bytes land at, and every
//!   read and save `remember`s the file's stamp in it. There is no polling or settle of the
//!   service's own: the watcher polls when it must (`Polling`), and `Unwatched` says nothing
//!   watches a library.
//! - **Immutable snapshots.** After each drained burst the actor publishes one new
//!   [`WorkspaceSnapshot`] (generation + 1) if anything changed, then sends the events. Commands
//!   reply after their snapshot is published.
//! - **Per-connection reads** (m2, I1 and R-T10-2). The snapshot's listings, status maps and
//!   diagnostics name files under `@<env>/` folders and stay crate-private; a connection reads
//!   [`WorkspaceSnapshot::view`]. Events come through a [`Subscription`] for one environment:
//!   a path a connection does not show never reaches it, and a file that moved out of its scope
//!   arrives as [`LibraryEvent::FileLeftScope`].
//! - **The root is re-validated** (I3) before every scan: a real folder (never a link), the
//!   canonical path and identity it had this session, and the attach checks again.
//! - **Watch before scan** (I5): a library's watcher starts before its first scan; events that
//!   arrive during a job wait and are probed after it.
//! - **Content is read only for open files** (I4): a changed file that no tab has open
//!   (`read` or `save` remembers it) is a listing change; its bytes are never read.
//! - **Two-pass streams** (I7, R-T10-3): [`LibraryService::open_stream`] returns a
//!   [`ScriptStream`] whose [`PassRecord`] a second pass is checked against.
//! - **File calls take the tab's environment** (R-T5-6).
//! - **Launch hygiene** (T5 mf4): recovery folders of unregistered libraries are swept, sparing
//!   any copy of a live instance (I2).

// ## m4b-t10-service (fix round 1): the actor, the workers' I/O, the view and the filtered
// events.
mod actor;
mod subscription;
mod view;
mod work;

pub use subscription::Subscription;
pub use view::{EnvView, GitSummary, LibraryView};
// ## m4b-t10-service
// ## m4b-t13-explorer (named exception): the trash seam; the file operations' I/O lives in
// `crate::fileops` (outside `service/`, whose sources name no file system call).
use crate::fileops;
#[cfg(any(test, feature = "test-support"))]
pub use crate::fileops::FolderTrash;
pub use crate::fileops::{OsTrash, Trasher};
// ## m4b-t13-explorer

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use arc_swap::ArcSwap;
use hacivat_git::{GitLimits, GitState};
use hacivat_proto::{EnvTag, ProfileId};
use hacivat_settings::paths::LocalPaths;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::events::LibraryEvent;
use crate::fileio::{
    self, Decoded, DiskStamp, PassRecord, ReadError, ReadLimits, SaveError, SaveText, ScriptStream,
    TextEncoding,
};
use crate::manifest;
use crate::names::{DatabaseName, LogicalDb, NameError};
use crate::pathsafe::RelPath;
use crate::registry::{LibraryId, LibraryRoot, LocalMapping, RegistryDiag, RegistryError};
use crate::scan::{EntryKind, LimitHit, Limits, RealFs, ScriptEntry};
use crate::watch::{EventSource, WatchError};

use actor::{Echoes, Msg};

/// How many events a slow receiver may lag behind before it misses some (it then re-reads the
/// snapshot).
const EVENT_CAPACITY: usize = 256;

/// Everything the UI reads, synchronously: all libraries, the local mappings and the status.
///
/// ## m4b-t10-service fix round 1 (m2)
/// The per-path data (each library's entries and diagnostics, and the status maps) is read
/// through [`view`](Self::view) for one connection's environment; it is not public here.
/// Fix round 2 (m-b): its `Debug` prints counts, never a path.
#[derive(Clone)]
pub struct WorkspaceSnapshot {
    pub generation: u64,
    /// The service's own state: `Scanning` until the registry is read and its libraries are
    /// started, then `Watching`.
    pub state: LibraryState,
    pub libraries: Arc<[Arc<LibrarySnapshot>]>,
    pub mappings: Arc<[LocalMapping]>,
    pub(crate) git: HashMap<LibraryId, GitState>,
    /// ## m4b-t10-service: what loading `libraries.toml` found (set aside, newer, unreadable,
    /// dropped rows), for Settings › Script libraries.
    pub registry: Arc<[RegistryDiag]>,
}

impl WorkspaceSnapshot {
    /// The snapshot of library `id`, if it is registered.
    pub fn library(&self, id: LibraryId) -> Option<&LibrarySnapshot> {
        self.libraries.iter().map(Arc::as_ref).find(|l| l.id == id)
    }

    // ## m4b-t10-service fix round 1
    /// A snapshot of `libraries` with no mappings and no status (tests, the index's callers).
    pub fn new(generation: u64, libraries: Vec<Arc<LibrarySnapshot>>) -> Self {
        WorkspaceSnapshot {
            generation,
            state: LibraryState::Watching,
            libraries: Arc::from(libraries),
            mappings: Arc::from([]),
            git: HashMap::new(),
            registry: Arc::from([]),
        }
    }
    // ## m4b-t10-service
}

/// One library as last scanned. Its `Debug` prints counts, never a path (fix round 2, m-b).
#[derive(Clone)]
pub struct LibrarySnapshot {
    pub generation: u64,
    pub id: LibraryId,
    pub root: LibraryRoot,
    pub db: LogicalDb,
    pub aliases: Arc<[LogicalDb]>,
    /// Sorted, folders first. ## m4b-t10-service fix round 1 (m2): read through
    /// [`LibraryView::entries`].
    pub(crate) entries: Arc<[ScriptEntry]>,
    pub state: LibraryState,
    /// ## m4b-t10-service fix round 1 (m2): read through [`LibraryView::diagnostics`].
    pub(crate) diagnostics: Arc<[LibraryDiag]>,
}

// ## m4b-t10-service fix round 2 (m-b): a log line never names a file (or a root) of any
// environment.
impl fmt::Debug for WorkspaceSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkspaceSnapshot")
            .field("generation", &self.generation)
            .field("state", &self.state)
            .field("libraries", &self.libraries)
            .field("mappings", &self.mappings.len())
            .field("repository", &self.git.len())
            .field("registry", &self.registry.len())
            .finish()
    }
}

impl fmt::Debug for LibrarySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibrarySnapshot")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .field("state", &self.state)
            .field("entries", &self.entries.len())
            .field("diagnostics", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}
// ## m4b-t10-service

impl LibrarySnapshot {
    // ## m4b-t10-service fix round 1
    /// A library snapshot (tests, `binding` callers): no diagnostics, generation 1.
    pub fn new(
        root: LibraryRoot,
        db: LogicalDb,
        aliases: Vec<LogicalDb>,
        entries: Vec<ScriptEntry>,
        state: LibraryState,
    ) -> Self {
        LibrarySnapshot {
            generation: 1,
            id: root.id,
            root,
            db,
            aliases: Arc::from(aliases),
            entries: Arc::from(entries),
            state,
            diagnostics: Arc::from([]),
        }
    }
    // ## m4b-t10-service
}

/// Where a library (or the service) stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryState {
    Scanning,
    Watching,
    Polling(PollReason),
    Unavailable(UnavailableReason),
    /// ## m4b-t10-service (watcher wiring): listed, but nothing watches it (the OS backend was
    /// lost and no poll source could watch it either, Task 9's `Backend::Unwatched`, or no
    /// watcher could start). A Refresh still rescans it.
    Unwatched,
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
    // ## m4b-t10-service fix round 2 (F2, R-T10-5)
    /// The folder did not answer within `ServiceOptions::job_deadline` (a hung mount); shown so
    /// until it answers.
    NotResponding,
    // ## m4b-t10-service
}

/// A library diagnostic. Names in it are already in their display form (escaped, NFC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryDiag {
    Manifest(manifest::Diag),
    // ## m4b-t10-service (the T4 carry)
    /// A scan diagnostic (bad names, unknown or contradictory environment folders, unreadable
    /// folders, a link hidden by its target's scope).
    Scan(crate::scan::Diag),
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
    // ## m4b-t10-service fix round 1
    /// Why the root is `Unavailable(RootChanged)` (I3).
    RootRefused(RootRefusal),
    /// This many diagnostics name paths this connection does not show (a [`LibraryView`]
    /// counts them instead of naming them, m2).
    HiddenProblems(usize),
    // ## m4b-t10-service
}

// ## m4b-t10-service fix round 1 (I3)
/// Why a registered root is not served (re-checked at every open and rescan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootRefusal {
    /// The registered path is a link or junction now.
    Link,
    /// It is not a folder.
    NotAFolder,
    /// It resolves to another canonical path than when it was first resolved this session.
    Moved,
    /// It is another folder than the one first resolved this session (device, inode, and
    /// birth time where reported).
    IdentityChanged,
    FilesystemRoot,
    HomeFolder,
    /// An app folder, inside one or containing one.
    AppFolder,
    /// Inside or around that library.
    Overlaps(LibraryId),
}
// ## m4b-t10-service

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
    /// ## fix round 1 (I3): the chosen path is a link or junction; attach its target instead
    /// (a link root is refused at every launch).
    RootIsLink,
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

/// The service's timings and bounds. The defaults are the product's; tests shorten the timers
/// and may inject a file system.
#[derive(Clone)]
pub struct ServiceOptions {
    /// The watcher's debounce and settle window (alg/28 C6; Task 9's `WatchOpts::window`).
    pub settle: Duration,
    /// The status refresh debounce (HV-SCRIPT-11 step 7).
    pub git_debounce: Duration,
    /// Fix round 1 (m7): a slow status refresh of polled libraries and of libraries whose
    /// repository's `.git` is above their root (a commit changes nothing in the library).
    pub git_backstop: Duration,
    /// How often an unavailable library is checked again.
    pub recheck: Duration,
    pub limits: Limits,
    pub git: GitLimits,
    /// The user's home folder, which is never a library (`AttachGuard`).
    pub home: Option<PathBuf>,
    /// Fix round 1 (I6): how many library jobs run at once (one per library at most).
    pub workers: usize,
    /// Fix round 2 (F2, R-T10-5): a job running longer stops counting against `workers`, and
    /// its library is shown `Unavailable(NotResponding)` until it answers.
    pub job_deadline: Duration,
    /// Fix round 1 (I6): the file system the workers list and `lstat` through (tests inject one
    /// that stalls).
    pub fs: Arc<dyn crate::scan::Fs + Send + Sync>,
    // ## m4b-t13-explorer (named exception)
    /// Where Move to Trash moves a file ([`OsTrash`]; tests pass a folder of their own).
    pub trasher: Arc<dyn Trasher>,
}

impl fmt::Debug for ServiceOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceOptions")
            .field("settle", &self.settle)
            .field("git_debounce", &self.git_debounce)
            .field("git_backstop", &self.git_backstop)
            .field("recheck", &self.recheck)
            .field("workers", &self.workers)
            .field("job_deadline", &self.job_deadline)
            .finish_non_exhaustive()
    }
}

impl Default for ServiceOptions {
    fn default() -> Self {
        ServiceOptions {
            settle: crate::watch::DEBOUNCE,
            git_debounce: Duration::from_millis(500),
            git_backstop: Duration::from_secs(60),
            recheck: Duration::from_secs(2),
            limits: Limits::default(),
            git: GitLimits::default(),
            home: std::env::home_dir(),
            workers: 8,
            job_deadline: Duration::from_secs(10),
            fs: Arc::new(RealFs),
            // ## m4b-t13-explorer (named exception)
            trasher: Arc::new(OsTrash),
        }
    }
}
// ## m4b-t10-service

// ## m4b-t10-service (watcher wiring)
/// A builder of one library's event source.
pub type SourceFactory = Arc<dyn Fn() -> Result<Box<dyn EventSource>, WatchError> + Send + Sync>;

/// Where each library's watcher gets its event source.
#[derive(Clone)]
pub enum Sources {
    /// Task 9's choice per library (`Watcher::open`): the OS watcher, or polling for a network
    /// file system or past the Linux watch budget.
    Os,
    /// The caller's (tests: `testing::FakeHandle::sources`).
    Custom(SourceFactory),
}
// ## m4b-t10-service

// ## m4b-t13-explorer fix round 1
/// What the file operations check against: the root, the listing and its diagnostics.
type Listing = (LibraryRoot, Arc<[ScriptEntry]>, Arc<[LibraryDiag]>);

/// Why creating, renaming or trashing a file was refused; the library is unchanged.
///
/// ## m4b-t13-explorer (named exception): the Task 1a stub's `NotImplemented` is gone; these are
/// the refusals of the filled operations (`crate::fileops`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOpError {
    /// The library is not registered, or not available (missing, not answering).
    Unavailable,
    /// The name breaks the scan's rules on this host or on Windows (a separator, `.`/`..`, a
    /// reserved device name, a trailing dot or space, a skipped name such as `.git`).
    BadName(crate::pathsafe::NameProblem),
    /// A script's name ends in `.sql` or `.md`.
    NotAScript,
    /// Something already has that name.
    Exists,
    /// Nothing is listed (or on disk) at the path.
    NotFound,
    /// The path, or what it would become, is not shown under the caller's connection
    /// (R-T5-6, R-M4b-6).
    NotListed,
    /// The folder holds a listed file the caller's connection does not show.
    HoldsHidden,
    /// A rename moves only the name, never to another folder.
    OtherFolder,
    /// The path was refused on the way (a link, outside the root, a changed folder).
    Path(crate::pathsafe::PathError),
    /// The operating system's trash refused it.
    Trash(io::ErrorKind),
    Io(io::ErrorKind),
    // ## m4b-t13-explorer fix round 1
    /// The rename would change which connections see the entry or anything inside it
    /// (R-T13-2: renaming `@staging` to `common`, say).
    ScopeChange,
    /// The folder's subtree was not listed in full (a scan limit, an unreadable or dataless
    /// folder, a refused name), so it cannot be checked for files the connection does not show
    /// (review I1): "too large to check".
    NotFullyListed,
    /// The file system cannot rename without risking a replace (no hard links): refused
    /// (R-T13-1).
    NoSafeRename,
    /// The trash took it, but a folder on the way changed meanwhile (review m13).
    TrashedButChanged,
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
    /// Each watched library's echo table (Task 9's `EchoHandle`), which a save records into.
    echoes: Echoes,
    /// How many times a script's content was read for a stamp (I4's test).
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    stamps: Arc<AtomicU64>,
    // ## m4b-t10-service
    // ## m4b-t13-explorer (named exception)
    trasher: Arc<dyn Trasher>,
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
    /// Returns at once; the registry is read and the libraries started on the actor
    /// (`WorkspaceSnapshot::state` is `Scanning` until then). Fix round 1 (R-T10-2): events are
    /// read through [`subscribe`](Self::subscribe) for one environment, so `open` returns no
    /// receiver.
    pub fn open(paths: &LocalPaths, rt: &Handle) -> Self {
        Self::open_with(paths, rt, Sources::Os, ServiceOptions::default())
    }

    // ## m4b-t10-service
    /// [`open`](Self::open) with event sources and options of the caller's (tests pass
    /// `testing::FakeHandle::sources` and short timers).
    pub fn open_with(
        paths: &LocalPaths,
        rt: &Handle,
        sources: Sources,
        opts: ServiceOptions,
    ) -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        // ## m4b-t13-explorer (named exception)
        let trasher = opts.trasher.clone();
        let empty = WorkspaceSnapshot {
            generation: 0,
            state: LibraryState::Scanning,
            libraries: Arc::from([]),
            mappings: Arc::from([]),
            git: HashMap::new(),
            registry: Arc::from([]),
        };
        let snapshot = Arc::new(ArcSwap::from_pointee(empty));
        let (tx, inbox) = mpsc::channel(actor::INBOX_CAPACITY);
        let closed = Arc::new(AtomicBool::new(false));
        let echoes = Echoes::default();
        let stamps = Arc::new(AtomicU64::new(0));
        actor::start(actor::Wiring {
            paths: paths.clone(),
            rt: rt.clone(),
            opts,
            tx: tx.clone(),
            inbox,
            closed: closed.clone(),
            snapshot: snapshot.clone(),
            events: events.clone(),
            echoes: echoes.clone(),
            stamps: stamps.clone(),
            sources,
        });
        LibraryService {
            snapshot,
            events,
            trasher,
            rt: rt.clone(),
            tx,
            closed,
            echoes,
            stamps,
        }
    }
    // ## m4b-t10-service

    /// The current snapshot; never blocks.
    pub fn snapshot(&self) -> Arc<WorkspaceSnapshot> {
        self.snapshot.load_full()
    }

    /// A receiver of the service's events for a connection whose environment is `env`.
    ///
    /// ## m4b-t10-service fix round 1 (I1, R-T10-2)
    /// Filtered: a path the connection does not show never reaches it ([`Subscription`]).
    pub fn subscribe(&self, env: Option<EnvTag>) -> Subscription {
        Subscription::new(self.events.subscribe(), env, self.snapshot.clone())
    }

    /// Registers `path` as a library bound to `db` (the folder name when `None`), writing
    /// `.hacivat/workspace.toml` only when `write_manifest` and the file is absent.
    ///
    /// ## m4b-t10-service
    /// `db` is what a manifest written now says. Without one written (`write_manifest ==
    /// false`, or a manifest already there, which is read and never rewritten) the library
    /// binds what its folder says, so the effective name is the snapshot's `db` once the
    /// library has been scanned (m5); a database of the user's choosing is a local mapping
    /// ([`map`](Self::map)). Refused before anything is written: a link (fix round 1, I3), the
    /// registry's checks (HV-SCRIPT-01 step 2) and a root inside an environment folder. A
    /// manifest that cannot be written refuses the attach. The folder's I/O runs on a worker,
    /// so a hung folder holds up this call only.
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
    /// (R-T5-4). A job of the library that is still running is abandoned, never waited for.
    /// `Io(BrokenPipe)` when the service has stopped.
    pub async fn detach(&self, id: LibraryId) -> Result<(), RegistryError> {
        self.ask(|reply| Msg::Detach { id, reply })
            .await
            .unwrap_or(Err(actor::STOPPED))
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
        .unwrap_or(Err(actor::STOPPED))
    }

    /// Drops every mapping of a deleted profile (R-M4b-2); how many.
    pub async fn drop_profile(&self, profile: ProfileId) -> Result<usize, RegistryError> {
        self.ask(|reply| Msg::DropProfile { profile, reply })
            .await
            .unwrap_or(Err(actor::STOPPED))
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
    /// The file is then remembered as open in a tab (its changes are stamped and announced,
    /// I4). A library that is not registered or not available answers `Io(NotFound)`.
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
        // The watcher learns the open file's stamp (rename pairing, and an edit that keeps
        // its `(len, mtime)`, R-T9-3); a later reload goes through here too (T9 carry).
        self.tell(Msg::Remember {
            id,
            rel: rel.clone(),
            stamp: decoded.stamp,
        })
        .await;
        Ok(decoded)
    }

    // ## m4b-t14-tabs (named exception: the script tabs' two service calls)
    /// [`read`](Self::read) in the encoding the user chose (`label`, a WHATWG name such as
    /// `"windows-1254"`; NOTES surface 10), after `read` answered `NeedsEncoding`. The file is
    /// remembered as open, as by `read`.
    pub async fn read_with(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
        label: &'static str,
    ) -> Result<Decoded, ReadError> {
        let root = self
            .root_of(id)
            .ok_or(ReadError::Io(io::ErrorKind::NotFound))?;
        let path = rel.clone();
        let decoded = self
            .blocking(move || fileio::read_with(&root, &path, env, label))
            .await
            .unwrap_or(Err(ReadError::Io(io::ErrorKind::Interrupted)))?;
        self.tell(Msg::Remember {
            id,
            rel: rel.clone(),
            stamp: decoded.stamp,
        })
        .await;
        Ok(decoded)
    }

    /// The last tab of `rel` closed (carries-w1 T14, T10 re-review): the file is no longer open,
    /// so its stamp is forgotten and its later changes are listing changes again (I4: only open
    /// files are read).
    pub async fn forget(&self, id: LibraryId, rel: &RelPath) {
        self.tell(Msg::Forget {
            id,
            rel: rel.clone(),
        })
        .await;
    }

    /// [`forget`](Self::forget) queued now, from any thread (the UI's tab close): it is in the
    /// inbox before any later read can remember the file again (T14 review minor 13). A full
    /// inbox hands it to the core runtime instead.
    pub fn forget_now(&self, id: LibraryId, rel: &RelPath) {
        let msg = Msg::Forget {
            id,
            rel: rel.clone(),
        };
        if let Err(mpsc::error::TrySendError::Full(msg)) = self.tx.try_send(msg) {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let _ = tx.send(msg).await;
            });
        }
    }

    /// Stops the service now, as dropping it does (the app at quit: the core runtime's shutdown
    /// then never waits for the actor). Every later call answers as a stopped service does.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self.tx.try_send(Msg::Shutdown);
    }

    /// Whether `rel` is remembered as open in a tab (tests of the last-tab rule).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn is_remembered(&self, id: LibraryId, rel: &RelPath) -> bool {
        self.ask(|reply| Msg::IsRemembered {
            id,
            rel: rel.clone(),
            reply,
        })
        .await
        .unwrap_or(false)
    }
    // ## m4b-t14-tabs

    /// Saves a script (HV-SCRIPT-07) for a tab under `env`, recording the echo before the
    /// rename.
    ///
    /// ## m4b-t05-fileio fix round 2 (named exception; I-fr1-1, R-T5-6): `env` as in `read`.
    ///
    /// ## m4b-t10-service
    /// `fileio::save_echoing_landing` on the blocking pool; the echo is recorded before the
    /// new file replaces the old one, so the watcher's report of it announces nothing. The
    /// file is remembered as open, and its listing entry refreshed.
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
        // The echo goes under the path the bytes land at: a file link's target (R-C0-7). Fix
        // round 3 (m-f): the one this save's own `join` resolved, never an older listing's.
        let (path, echo) = (rel.clone(), self.echoes.get(id));
        let saved = self
            .blocking(move || {
                fileio::save_echoing_landing(
                    &root,
                    &path,
                    env,
                    text,
                    expect,
                    &mut |landing, stamp| {
                        if let Some(echo) = &echo {
                            echo.record(landing.clone(), *stamp);
                        }
                    },
                )
            })
            .await
            .unwrap_or(Err(SaveError::Io(io::ErrorKind::Interrupted)));
        let stamp = match &saved {
            Ok(stamp) | Err(SaveError::SavedButPathChanged(stamp)) => Some(Some(*stamp)),
            Err(SaveError::Damaged) => Some(None),
            Err(_) => None,
        };
        if let Some(stamp) = stamp {
            self.tell(Msg::Touched {
                id,
                rel: rel.clone(),
                stamp,
            })
            .await;
        }
        saved
    }

    // ## m4b-t13-explorer (named exception): Task 1a's stubs, filled. Each runs on the
    // blocking pool, for a tree row under a connection whose environment is `env` (R-T5-6),
    // and asks for a rescan of the library afterwards, so the listing shows the change even
    // before the watcher reports it.
    /// Creates an empty script or a folder at `rel` (never over an existing name).
    pub async fn create(
        &self,
        id: LibraryId,
        rel: &RelPath,
        kind: EntryKind,
        env: Option<EnvTag>,
    ) -> Result<(), FileOpError> {
        let root = self.root_of(id).ok_or(FileOpError::Unavailable)?;
        let rel2 = rel.clone();
        let out = self
            .blocking(move || fileops::create(&root, &rel2, kind, env))
            .await
            .unwrap_or(Err(FileOpError::Io(io::ErrorKind::Interrupted)));
        self.after_op(id, &out).await;
        out
    }

    /// Renames a script or folder to `to`, in the same folder (never over an existing name).
    pub async fn rename(
        &self,
        id: LibraryId,
        from: &RelPath,
        to: &RelPath,
        env: Option<EnvTag>,
    ) -> Result<(), FileOpError> {
        let (root, entries, diags) = self.listing_of(id).ok_or(FileOpError::Unavailable)?;
        let (from2, to2) = (from.clone(), to.clone());
        let out = self
            .blocking(move || fileops::rename(&root, &entries, &diags, &from2, &to2, env))
            .await
            .unwrap_or(Err(FileOpError::Io(io::ErrorKind::Interrupted)));
        self.after_op(id, &out).await;
        out
    }

    /// Moves a script or folder to the OS Trash (Q11), through [`ServiceOptions::trasher`].
    pub async fn trash(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
    ) -> Result<(), FileOpError> {
        let (root, entries, diags) = self.listing_of(id).ok_or(FileOpError::Unavailable)?;
        let (rel2, trasher) = (rel.clone(), self.trasher.clone());
        let out = self
            .blocking(move || fileops::trash(&root, &entries, &diags, &rel2, env, trasher.as_ref()))
            .await
            .unwrap_or(Err(FileOpError::Io(io::ErrorKind::Interrupted)));
        self.after_op(id, &out).await;
        out
    }

    /// Lists library `id` again and refreshes its repository status (the tree's Refresh).
    pub async fn refresh(&self, id: LibraryId) {
        self.tell(Msg::Rescan { id }).await;
    }

    /// After a file operation that changed something: a rescan.
    async fn after_op(&self, id: LibraryId, out: &Result<(), FileOpError>) {
        if matches!(out, Ok(()) | Err(FileOpError::TrashedButChanged)) {
            self.refresh(id).await;
        }
    }

    /// The root, every listed entry and the diagnostics of an available library.
    fn listing_of(&self, id: LibraryId) -> Option<Listing> {
        let snap = self.snapshot.load();
        let lib = snap.library(id)?;
        servable(&lib.state).then(|| {
            (
                lib.root.clone(),
                lib.entries.clone(),
                lib.diagnostics.clone(),
            )
        })
    }
    // ## m4b-t13-explorer

    /// A chunked UTF-8 reader of a script for "Run as script" (Task 12's source), for a tab
    /// under `env`.
    ///
    /// ## m4b-t05-fileio fix round 2 (named exception; I-fr1-1, R-T5-6): `env` as in `read`.
    ///
    /// ## m4b-t10-service
    /// Async, and takes the file's `encoding` (the tab's `FileMeta::encoding`): opening is I/O,
    /// so it runs on the blocking pool, and the stream decodes in the file's own encoding. The
    /// reader does blocking reads; drive it off the UI thread.
    ///
    /// Fix round 1 (I7, R-T10-3), the pass-1/pass-2 contract: pass 1 opens with `expect: None`
    /// and keeps [`ScriptStream::record`]; pass 2 opens with `Some(record)` and is refused with
    /// [`ReadError::Changed`] unless the file is the one pass 1 opened, and stops with
    /// [`ChunkError::Changed`] at the first block whose running hash differs
    /// ([`PassRecord`]).
    pub async fn open_stream(
        &self,
        id: LibraryId,
        rel: &RelPath,
        env: Option<EnvTag>,
        encoding: TextEncoding,
        expect: Option<PassRecord>,
    ) -> Result<ScriptStream, ReadError> {
        let root = self
            .root_of(id)
            .ok_or(ReadError::Io(io::ErrorKind::NotFound))?;
        let path = rel.clone();
        self.blocking(move || fileio::open_stream_checked(&root, &path, env, encoding, expect))
            .await
            .unwrap_or(Err(ReadError::Io(io::ErrorKind::Interrupted)))
    }

    // ## m4b-t10-service
    /// Resolves once the service is quiet: no job running or queued, no open file waiting for
    /// its stamp, no confirming scan due, no attach under way.
    ///
    /// Fix round 3 (m-i): it does **not** wait for the Task 9 watcher's own pending settle. A
    /// hint handed to a watcher is reported only after the watcher's window (`settle`), so
    /// `emit(...); settled().await` can resolve before that report arrives and see a listing
    /// that has not caught up yet. Tests wait on a condition (the listing, an event) first,
    /// and call `settled()` after it for the service's own follow-up work.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn settled(&self) {
        let _ = self.ask(|reply| Msg::Settled { reply }).await;
    }

    /// Fix round 3 (H16): hands library `id` a watch batch as its watcher would, tagged with
    /// the current watcher's generation, or with `stale`, the previous one (a dropped watcher's
    /// late batch).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub async fn inject_watch_batch(
        &self,
        id: LibraryId,
        stale: bool,
        batch: crate::watch::WatchBatch,
    ) {
        self.tell(Msg::InjectWatch { id, stale, batch }).await;
    }

    /// How many times a script's content was read for a stamp (fix round 1, I4: only files
    /// open in a tab are ever read).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn stamps_read(&self) -> u64 {
        self.stamps.load(Ordering::Relaxed)
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

/// Whether files of a library in `state` may be opened.
fn servable(state: &LibraryState) -> bool {
    matches!(
        state,
        LibraryState::Scanning
            | LibraryState::Watching
            | LibraryState::Polling(_)
            | LibraryState::Unwatched
    )
}

impl LogicalDb {
    /// The name of a library that binds nothing automatically: empty, which no parsed name
    /// (and so no database node) ever equals.
    pub(crate) fn unbindable() -> Self {
        LogicalDb(Arc::from(""))
    }
}
// ## m4b-t10-service
