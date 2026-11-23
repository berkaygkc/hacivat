// ## m4b-t10-service (fix round 1: the actor moved here, with its worker pool; wired to Task 9's
// watcher after fix round 2)
//! The service's actor: one long-lived task on the core runtime's blocking pool that owns every
//! piece of mutable state and handles one message at a time. It does no library I/O itself
//! (I6, F1; R-T10-4): each library's validation, watcher start, scans, probes and stamps run as
//! jobs on at most `ServiceOptions::workers` blocking workers, one job per library at a time,
//! and come back as messages. Work that arrives while a library's job runs waits in that
//! library's queue (coalesced: one rescan, the reported paths, the due stamps). A job of a
//! library that was detached meanwhile frees its worker at once and is ignored when it returns;
//! one that runs past `job_deadline` stops counting and shows its library not responding (F2,
//! R-T10-5).
//!
//! Each library is watched by its own Task 9 [`Watcher`], started on a worker before the
//! library's first scan (I5): it reconciles the source's hints (settle, echo, rename pairing)
//! and delivers [`WatchEvent`]s, which a forwarding task tags with the watcher's generation, so
//! a batch from a watcher that was dropped (a library detached, gone or re-attached) is
//! ignored.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwap;
use hacivat_git::GitState;
use hacivat_proto::ProfileId;
use hacivat_settings::paths::LocalPaths;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::work::{
    self, HeldRemoval, ManifestJob, ManifestOut, PairJob, RefreshJob, RefreshOut, Rules, Session,
    StampJob, StampOut,
};
use super::{
    AttachError, LibraryDiag, LibrarySnapshot, LibraryState, MapError, ServiceOptions, Sources,
    UnavailableReason, WorkspaceSnapshot, servable,
};
use crate::events::LibraryEvent;
use crate::fileio::DiskStamp;
use crate::manifest;
use crate::names::{DatabaseName, LogicalDb};
use crate::pathsafe::RelPath;
use crate::recovery;
use crate::registry::{
    LibraryId, LibraryRegistry, LibraryRoot, LocalMapping, RegisteredLibrary, RegistryDiag,
    RegistryError,
};
// ## m4b-t16-attach (named lines)
use super::{PollReason, RootRefusal};
use crate::registry::WatchMode;
// ## m4b-t16-attach
use crate::scan::ScriptEntry;
use crate::watch::{
    Backend, EchoHandle, EventSource, HintSink, WatchBatch, WatchError, WatchEvent, WatchOpts,
    Watcher,
};

/// The actor's inbox.
pub(super) const INBOX_CAPACITY: usize = 1024;
/// What a registry call answers once the service has stopped.
pub(super) const STOPPED: RegistryError = RegistryError::Io(io::ErrorKind::BrokenPipe);
/// The most messages handled before a publish.
const DRAIN_MAX: usize = 64;
/// A batch with more paths than this rescans instead (HV-SCRIPT-08 step 3).
const BURST_PATHS: usize = 1_000;
/// How many deleted paths per library are remembered, so their return is announced.
const RECENT_DELETES: usize = 1_024;
/// Batches a watcher may queue towards the actor before it waits.
const WATCH_CHANNEL: usize = 64;
/// Fix round 3 (m-g): the confirming scans of one chain at the base pace; each one past them
/// waits twice as long as the one before (up to [`MAX_CHAIN_WAIT`]).
const MAX_CONFIRMS: u8 = 3;
/// Fix round 3 (m-g): the longest wait between two confirming scans of one chain.
const MAX_CHAIN_WAIT: Duration = Duration::from_secs(30);
/// Fix round 3 (G1): the most recently appeared files kept as rename candidates.
const MAX_ARRIVED: usize = 64;
/// Fix round 3 (G1): how long an appeared file stays a rename candidate.
const ARRIVAL_SPAN: Duration = Duration::from_secs(10);

/// A source that refuses: a factory that could not build one (the watcher falls back).
struct Refusing(WatchError);

impl EventSource for Refusing {
    fn watch(&mut self, _root: &Path, _sink: HintSink) -> Result<(), WatchError> {
        Err(self.0.clone())
    }

    fn unwatch(&mut self, _root: &Path) {}
}

/// Each watched library's echo table (Task 9's `EchoHandle`): a save records the stamp of what
/// it wrote under the path the bytes land at, before the rename (HV-SCRIPT-07 step 4), so the
/// watcher announces nothing for it. Written by the save's blocking task; the lock is never
/// held across an `.await`.
#[derive(Debug, Clone, Default)]
pub(super) struct Echoes(Arc<Mutex<HashMap<LibraryId, EchoHandle>>>);

impl Echoes {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<LibraryId, EchoHandle>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn get(&self, id: LibraryId) -> Option<EchoHandle> {
        self.lock().get(&id).cloned()
    }

    fn set(&self, id: LibraryId, echo: Option<EchoHandle>) {
        let mut table = self.lock();
        match echo {
            Some(e) => table.insert(id, e),
            None => table.remove(&id),
        };
    }
}

/// The actor's messages.
pub(super) enum Msg {
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
    // ## m4b-t16-attach (named exception)
    /// An attach that writes exactly this manifest (the text the dialog showed).
    AttachWriting {
        path: PathBuf,
        manifest: manifest::ManifestV1,
        reply: oneshot::Sender<Result<LibraryId, AttachError>>,
    },
    SetWatch {
        id: LibraryId,
        watch: WatchMode,
        reply: oneshot::Sender<Result<(), RegistryError>>,
    },
    AdoptRoot {
        id: LibraryId,
        // ## m4b-t20-adversarial (named line, m4)
        seen: Option<crate::fileio::FolderIdentity>,
        reply: oneshot::Sender<Result<bool, RegistryError>>,
    },
    // ## m4b-t16-attach
    Map {
        m: LocalMapping,
        share_alias: bool,
        // ## m4b-t16-attach (named line): the manifest bytes the alias edit was shown over.
        expect: Option<Vec<u8>>,
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
    /// A batch from library `id`'s watcher of generation `generation`.
    Watch {
        id: LibraryId,
        generation: u64,
        batch: WatchBatch,
    },
    /// A tab read (or reloaded) the file: it is open, with this stamp (I4; `remember`).
    Remember {
        id: LibraryId,
        rel: RelPath,
        stamp: DiskStamp,
    },
    /// A save wrote the file (`stamp` when it is known).
    Touched {
        id: LibraryId,
        rel: RelPath,
        stamp: Option<DiskStamp>,
    },
    // ## m4b-t14-tabs (named exception)
    /// The last tab of the file closed: it is no longer open.
    Forget {
        id: LibraryId,
        rel: RelPath,
    },
    /// Whether the file is remembered as open (tests).
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    IsRemembered {
        id: LibraryId,
        rel: RelPath,
        reply: oneshot::Sender<bool>,
    },
    // ## m4b-t14-tabs
    /// The recheck timer of an unavailable library.
    Tick {
        id: LibraryId,
        seq: u64,
    },
    /// The slow status backstop (m7).
    Backstop,
    GitTick {
        id: LibraryId,
        seq: u64,
    },
    GitDone {
        id: LibraryId,
        state: GitState,
    },
    /// A library job came back.
    Refreshed {
        id: LibraryId,
        job: u64,
        out: RefreshOut,
    },
    Stamped {
        id: LibraryId,
        job: u64,
        rel: RelPath,
        out: StampOut,
    },
    WatchStarted {
        id: LibraryId,
        job: u64,
        result: Result<Watcher, WatchError>,
    },
    /// Fix round 3 (G2): a manifest-only read came back (`None`: the root is not the folder
    /// this session knows).
    ManifestRead {
        id: LibraryId,
        job: u64,
        out: Option<ManifestOut>,
    },
    /// Fix round 3 (G1): a pairing of held removals came back.
    Paired {
        id: LibraryId,
        job: u64,
        out: Vec<(RelPath, Option<RelPath>)>,
    },
    /// Fix round 2 (F2, R-T10-5): a job ran past `ServiceOptions::job_deadline`.
    JobOverdue {
        job: u64,
    },
    /// A probe disagreed with the watcher a while ago: rescan now.
    Resync {
        id: LibraryId,
        generation: u64,
    },
    AttachResolved {
        token: u64,
        result: Result<work::Valid, AttachError>,
    },
    AttachWritten {
        token: u64,
        result: Result<(), manifest::WriteError>,
    },
    AliasWritten {
        token: u64,
        result: Result<(), manifest::WriteError>,
    },
    // ## m4b-t13-explorer (named exception)
    /// The tree's Refresh, or a file operation that changed the library: rescan it and refresh
    /// its repository status.
    Rescan {
        id: LibraryId,
    },
    // ## m4b-t13-explorer
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    Settled {
        reply: oneshot::Sender<()>,
    },
    /// Fix round 3 (H16, test support): a watch batch as the current watcher (or, `stale`, the
    /// previous one) would send it.
    #[cfg(any(test, feature = "test-support"))]
    InjectWatch {
        id: LibraryId,
        stale: bool,
        batch: WatchBatch,
    },
    Shutdown,
}

/// What the service hands the actor.
pub(super) struct Wiring {
    pub paths: LocalPaths,
    pub rt: Handle,
    pub opts: ServiceOptions,
    pub tx: mpsc::Sender<Msg>,
    pub inbox: mpsc::Receiver<Msg>,
    pub closed: Arc<AtomicBool>,
    pub snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    pub events: broadcast::Sender<LibraryEvent>,
    pub echoes: Echoes,
    pub stamps: Arc<AtomicU64>,
    pub sources: Sources,
}

/// Starts the actor on the runtime's blocking pool; the registry is read there.
pub(super) fn start(w: Wiring) {
    let rt = w.rt.clone();
    rt.spawn_blocking(move || {
        // F1: the rows are compared by text; each root is validated by its own job.
        let (registry, diags) = LibraryRegistry::load_text_only(&w.paths.libraries_toml());
        let mut app = vec![
            w.paths.config.clone(),
            w.paths.data.clone(),
            w.paths.cache.clone(),
        ];
        app.extend(w.paths.runtime.clone());
        // F1: as configured; a job resolves them (a home folder may be on a network mount).
        let rules = Arc::new(Rules {
            home: w.opts.home.clone(),
            app,
        });
        let actor = Actor {
            paths: w.paths,
            rt: w.rt,
            opts: w.opts,
            tx: w.tx,
            closed: w.closed,
            snapshot: w.snapshot,
            events: w.events,
            echoes: w.echoes,
            stamps: w.stamps,
            sources: w.sources,
            rules,
            registry,
            registry_diags: Arc::from([]),
            libs: Vec::new(),
            git: HashMap::new(),
            generation: 0,
            loaded: false,
            dirty: HashSet::new(),
            workspace_dirty: false,
            pending: Vec::new(),
            in_flight: 0,
            jobs: HashMap::new(),
            next_job_id: 0,
            next_lib: 0,
            next_token: 0,
            next_watch: 0,
            attaches: HashMap::new(),
            aliases: HashMap::new(),
            waiters: Vec::new(),
        };
        actor.run(w.inbox, diags);
    });
}

/// The repository status runs of one library.
#[derive(Debug, Default)]
struct GitRun {
    seq: u64,
    in_flight: bool,
    again: bool,
}

/// Work queued for a library's next job.
#[derive(Debug, Default)]
struct Want {
    validate: bool,
    watch: bool,
    scan: bool,
    probes: Vec<(RelPath, work::Expect)>,
    /// Fix round 3 (G2): re-read the manifest only.
    manifest: bool,
    /// Fix round 3 (G1): pair the held removals with the files that appeared.
    pair: bool,
}

impl Want {
    fn is_empty(&self) -> bool {
        !self.validate
            && !self.watch
            && !self.scan
            && self.probes.is_empty()
            && !self.manifest
            && !self.pair
    }
}

/// A scan's removal of a file open in a tab, held (it may have moved).
#[derive(Debug, Clone, Copy)]
struct Held {
    len: u64,
    /// The watcher reported it removed (or a probe of its report found it gone): a pairing
    /// that finds no match makes it a delete, polled or not.
    reported: bool,
    /// A pairing found no match while the library was polled: it waits for the poller's report
    /// (or the backstop).
    unpaired: bool,
}

/// One registered library, as the actor keeps it.
struct Lib {
    reg: RegisteredLibrary,
    /// Its canonical root and identity, once first resolved this session (a root that resolves
    /// elsewhere later is `RootChanged`, never adopted).
    session: Session,
    state: LibraryState,
    db: LogicalDb,
    aliases: Arc<[LogicalDb]>,
    manifest_diags: Vec<LibraryDiag>,
    // ## m4b-t16-attach (named line; R-T10-7): `db` and `aliases` came from a manifest read.
    manifest_good: bool,
    // ## m4b-t16-attach fix round 1 (I3): a confirmed "Use this folder" waits for its
    // validation, and the job that runs it.
    adopting: bool,
    adopt_job: Option<u64>,
    adopt_reply: Option<oneshot::Sender<Result<bool, RegistryError>>>,
    // ## m4b-t20-adversarial (named lines, T16 re-review m4): the folder the last refusal
    // found at the path, and the one a confirmed adoption was shown for.
    refused_seen: Option<crate::fileio::FolderIdentity>,
    adopt_seen: Option<crate::fileio::FolderIdentity>,
    scan_diags: Vec<LibraryDiag>,
    entries: Arc<[ScriptEntry]>,
    stopped: bool,
    has_git_dir: bool,
    /// Its Task 9 watcher, and that watcher's generation (a batch of an older one is ignored).
    watcher: Option<Watcher>,
    watch_gen: u64,
    /// No watcher could start (or none watches it any more): listed, `Unwatched`.
    unwatched: bool,
    /// The recheck timer chain that is live, if any.
    timer: Option<u64>,
    next_timer: u64,
    git: GitRun,
    /// A job of this library is running.
    busy: bool,
    /// Its id (F2: an answer from an abandoned job is ignored).
    job: Option<u64>,
    want: Want,
    /// A resync rescan is scheduled.
    resync_due: bool,
    /// Open files a scan found gone, waiting for the watcher's word (rename or delete) or a
    /// pairing (fix round 3, G1).
    held: HashMap<RelPath, Held>,
    /// Fix round 3 (G1): files that appeared lately, with when (rename candidates; at most
    /// `MAX_ARRIVED`, for `ARRIVAL_SPAN`).
    arrived: VecDeque<(RelPath, Instant)>,
    /// Fix round 3 (G1): renames announced by a pairing, so the watcher's own later word for
    /// the same move is not announced twice.
    renamed: HashMap<RelPath, RelPath>,
    /// The scan queued now is a resync's confirming scan.
    confirming: bool,
    /// Fix round 3 (m-g): confirming scans so far in this chain.
    confirms: u8,
    /// Fix round 3 (m-g): how long its last full scan took.
    scan_cost: Duration,
    /// When its watcher last reported anything.
    last_event: Option<Instant>,
    /// Files open in a tab whose new stamp is due (de-duplicated).
    stamp_due: VecDeque<RelPath>,
    /// Files open in a tab, with their last known stamp (I4: only these are read; a re-stamp
    /// equal to it announces nothing, so a `PossiblyChanged` that did not change never reaches
    /// the user).
    remembered: HashMap<RelPath, Option<DiskStamp>>,
    /// Renames the watcher reported (`from → to`): announced once the listing no longer has
    /// `from` (so a reader of the snapshot sees the move), and that removal is not a delete.
    moved: HashMap<RelPath, RelPath>,
    deleted: HashSet<RelPath>,
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
            // ## m4b-t16-attach (named line)
            watch: reg.watch,
            // ## m4b-t20-adversarial (named line)
            refused_identity: None,
        });
        Lib {
            reg,
            session: Session::default(),
            state: LibraryState::Scanning,
            db,
            aliases: Arc::from([]),
            manifest_diags: Vec::new(),
            // ## m4b-t16-attach (named line)
            manifest_good: false,
            adopting: false,
            adopt_job: None,
            adopt_reply: None,
            // ## m4b-t20-adversarial (named lines)
            refused_seen: None,
            adopt_seen: None,
            scan_diags: Vec::new(),
            entries: Arc::from([]),
            stopped: false,
            has_git_dir: true,
            watcher: None,
            watch_gen: 0,
            unwatched: false,
            timer: None,
            next_timer: 0,
            git: GitRun::default(),
            busy: false,
            job: None,
            want: Want::default(),
            resync_due: false,
            held: HashMap::new(),
            arrived: VecDeque::new(),
            renamed: HashMap::new(),
            confirming: false,
            confirms: 0,
            scan_cost: Duration::ZERO,
            last_event: None,
            stamp_due: VecDeque::new(),
            remembered: HashMap::new(),
            moved: HashMap::new(),
            deleted: HashSet::new(),
            snap,
        }
    }

    fn id(&self) -> LibraryId {
        self.reg.id
    }

    fn root(&self) -> Option<LibraryRoot> {
        Some(LibraryRoot {
            id: self.reg.id,
            path: self.reg.path.clone(),
            canonical: self.session.canonical.clone()?,
        })
    }

    /// Up: scanned at least once and not unavailable since.
    fn up(&self) -> bool {
        matches!(
            self.state,
            LibraryState::Watching | LibraryState::Polling(_) | LibraryState::Unwatched
        )
    }

    /// The state its watcher says (once it is up).
    fn watched_state(&self) -> LibraryState {
        match self.watcher.as_ref().map(Watcher::backend) {
            _ if self.unwatched => LibraryState::Unwatched,
            Some(Backend::Os) => LibraryState::Watching,
            Some(Backend::Poll(reason)) => LibraryState::Polling(reason),
            Some(Backend::Unwatched) | None => LibraryState::Unwatched,
        }
    }

    fn snapshot(&self, generation: u64) -> Arc<LibrarySnapshot> {
        let root = self.root().unwrap_or_else(|| LibraryRoot {
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
            entries: self.entries.clone(),
            state: self.state.clone(),
            diagnostics: Arc::from(diagnostics),
            // ## m4b-t16-attach (named line)
            watch: self.reg.watch,
            // ## m4b-t20-adversarial (named line, m4)
            refused_identity: (self.state
                == LibraryState::Unavailable(UnavailableReason::RootChanged))
            .then_some(self.refused_seen)
            .flatten(),
        })
    }
}

/// The logical name the folder name gives, when it is a valid one.
fn folder_db(path: &Path) -> Option<LogicalDb> {
    LogicalDb::parse(path.file_name()?.to_str()?).ok()
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

/// An attach between its steps.
struct PendingAttach {
    path: PathBuf,
    db: Option<LogicalDb>,
    write_manifest: bool,
    // ## m4b-t16-attach (named line): write exactly this one, never over an existing file.
    exact: Option<manifest::ManifestV1>,
    root: Option<LibraryRoot>,
    identity: Option<crate::fileio::FolderIdentity>,
    reply: oneshot::Sender<Result<LibraryId, AttachError>>,
}

/// A map waiting for its alias write.
struct PendingMap {
    m: LocalMapping,
    reply: oneshot::Sender<Result<(), MapError>>,
}

/// A library job.
enum Job {
    Refresh(Box<RefreshJob>),
    Stamp(RelPath, Box<StampJob>),
    Watch(Box<WatchJob>),
    Manifest(Box<ManifestJob>),
    Pair(Box<PairJob>),
}

/// Starts a library's watcher (it watches, then scans the library once: blocking).
struct WatchJob {
    root: LibraryRoot,
    opts: WatchOpts,
    sources: Sources,
    sink: mpsc::Sender<WatchBatch>,
}

impl WatchJob {
    fn run(self) -> Result<Watcher, WatchError> {
        match self.sources {
            Sources::Os => Watcher::open(self.root, self.opts, self.sink),
            Sources::Custom(make) => {
                let source = make().unwrap_or_else(|e| Box::new(Refusing(e)));
                Watcher::start(self.root, self.opts, self.sink, source)
            }
        }
    }
}

struct Actor {
    paths: LocalPaths,
    rt: Handle,
    opts: ServiceOptions,
    tx: mpsc::Sender<Msg>,
    closed: Arc<AtomicBool>,
    snapshot: Arc<ArcSwap<WorkspaceSnapshot>>,
    events: broadcast::Sender<LibraryEvent>,
    echoes: Echoes,
    stamps: Arc<AtomicU64>,
    sources: Sources,
    rules: Arc<Rules>,
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
    /// Library jobs counted against the cap (at most `opts.workers`).
    in_flight: usize,
    /// Running jobs, and whether each still counts (F2: abandoned and overdue ones do not).
    jobs: HashMap<u64, bool>,
    next_job_id: u64,
    /// Where the next round of `dispatch` starts (fairness).
    next_lib: usize,
    next_token: u64,
    next_watch: u64,
    attaches: HashMap<u64, PendingAttach>,
    aliases: HashMap<u64, PendingMap>,
    waiters: Vec<oneshot::Sender<()>>,
}

impl Actor {
    fn run(mut self, mut inbox: mpsc::Receiver<Msg>, diags: Vec<RegistryDiag>) {
        self.load(diags);
        self.after(self.opts.git_backstop, Msg::Backstop);
        self.dispatch();
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
            self.dispatch();
            self.flush();
        }
        for i in 0..self.libs.len() {
            self.drop_watcher(i);
        }
    }

    // ---- launch ------------------------------------------------------------------------------

    /// Sweeps stale recovery folders (only over a registry read cleanly) and queues every
    /// registered library's bring-up.
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
            let mut lib = Lib::new(reg);
            lib.want.validate = true;
            self.libs.push(lib);
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
            } => self.attach(path, db, write_manifest, reply),
            Msg::AttachResolved { token, result } => self.attach_resolved(token, result),
            Msg::AttachWritten { token, result } => self.attach_written(token, result),
            // ## m4b-t16-attach (named exception)
            Msg::AttachWriting {
                path,
                manifest,
                reply,
            } => self.attach_writing(path, manifest, reply),
            Msg::SetWatch { id, watch, reply } => {
                let r = self.set_watch(id, watch);
                self.dispatch();
                let _ = reply.send(r);
            }
            Msg::AdoptRoot { id, seen, reply } => match self.adopt_root(id, seen) {
                // ## m4b-t16-attach fix round 1: answered when its validation is done.
                Ok(i) => {
                    self.libs[i].adopt_reply = Some(reply);
                    self.dispatch();
                }
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            },
            // ## m4b-t16-attach
            Msg::Detach { id, reply } => {
                let r = self.detach(id);
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Map {
                m,
                share_alias,
                expect,
                reply,
            } => self.map(m, share_alias, expect, reply),
            Msg::AliasWritten { token, result } => self.alias_written(token, result),
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
            Msg::Watch {
                id,
                generation,
                batch,
            } => self.watch_batch(id, generation, batch),
            Msg::Remember { id, rel, stamp } => {
                // m6: nothing is kept for a library that is gone.
                if let Some(i) = self.index(id) {
                    self.remember(i, rel, Some(stamp));
                }
            }
            Msg::Touched { id, rel, stamp } => {
                if let Some(i) = self.index(id) {
                    if stamp.is_some() || !self.libs[i].remembered.contains_key(&rel) {
                        self.remember(i, rel.clone(), stamp);
                    }
                    self.libs[i].want.probes.push((rel, work::Expect::Either));
                    self.git_trigger(i);
                }
            }
            // ## m4b-t14-tabs (named exception)
            Msg::Forget { id, rel } => {
                if let Some(i) = self.index(id) {
                    let lib = &mut self.libs[i];
                    lib.remembered.remove(&rel);
                    lib.stamp_due.retain(|r| *r != rel);
                }
            }
            Msg::IsRemembered { id, rel, reply } => {
                let open = self
                    .index(id)
                    .is_some_and(|i| self.libs[i].remembered.contains_key(&rel));
                let _ = reply.send(open);
            }
            // ## m4b-t14-tabs
            Msg::Tick { id, seq } => self.tick(id, seq),
            Msg::Backstop => self.backstop(),
            Msg::GitTick { id, seq } => self.git_tick(id, seq),
            Msg::GitDone { id, state } => self.git_done(id, state),
            Msg::Refreshed { id, job, out } => {
                if let Some(i) = self.job_done(id, job) {
                    // ## m4b-t16-attach fix round 1 (I3): the adoption's own answer.
                    let adoption = self.libs[i].adopt_job == Some(job);
                    if adoption {
                        let lib = &mut self.libs[i];
                        lib.adopt_job = None;
                        lib.adopting = false;
                    }
                    let reply = if adoption {
                        self.libs[i].adopt_reply.take()
                    } else {
                        None
                    };
                    let adopted = match out {
                        RefreshOut::Validated(v) if adoption => {
                            if self.commit_adoption(i, &v) {
                                self.refreshed(i, RefreshOut::Validated(v));
                                true
                            } else {
                                let diags = self.libs[i].scan_diags.clone();
                                self.make_unavailable(i, UnavailableReason::RootChanged, diags);
                                false
                            }
                        }
                        out => {
                            self.refreshed(i, out);
                            false
                        }
                    };
                    if let Some(reply) = reply {
                        let _ = reply.send(Ok(adopted));
                    }
                }
            }
            Msg::Stamped { id, job, rel, out } => {
                if let Some(i) = self.job_done(id, job) {
                    self.stamped(i, rel, out);
                }
            }
            Msg::WatchStarted { id, job, result } => match self.job_done(id, job) {
                Some(i) => self.watch_started(i, result),
                // Its library is gone: drop the watcher off this thread.
                None => {
                    if let Ok(w) = result {
                        self.rt.spawn_blocking(move || drop(w));
                    }
                }
            },
            Msg::ManifestRead { id, job, out } => {
                if let Some(i) = self.job_done(id, job) {
                    match out {
                        Some(m) => {
                            if self.apply_manifest(i, m) {
                                self.dirty.insert(id);
                            }
                        }
                        None => self.libs[i].want.scan = true,
                    }
                }
            }
            Msg::Paired { id, job, out } => {
                if let Some(i) = self.job_done(id, job) {
                    self.paired(i, out);
                }
            }
            Msg::JobOverdue { job } => self.job_overdue(job),
            Msg::Resync { id, generation } => {
                if let Some(i) = self.index(id) {
                    let quiet = self.quiet_for();
                    let lib = &mut self.libs[i];
                    lib.resync_due = false;
                    if lib.watch_gen != generation || !lib.up() {
                        return;
                    }
                    // Still changing: wait until it has been quiet for the whole span.
                    if lib.last_event.is_some_and(|at| at.elapsed() < quiet) {
                        self.resync_later(i);
                    } else {
                        lib.want.scan = true;
                        lib.confirming = true;
                    }
                }
            }
            // ## m4b-t13-explorer (named exception)
            Msg::Rescan { id } => {
                if let Some(i) = self.index(id) {
                    self.libs[i].want.scan = true;
                    self.git_trigger(i);
                }
            }
            // ## m4b-t13-explorer
            Msg::Settled { reply } => self.waiters.push(reply),
            #[cfg(any(test, feature = "test-support"))]
            Msg::InjectWatch { id, stale, batch } => {
                if let Some(i) = self.index(id) {
                    let current = self.libs[i].watch_gen;
                    let generation = if stale {
                        current.wrapping_sub(1)
                    } else {
                        current
                    };
                    self.watch_batch(id, generation, batch);
                }
            }
            Msg::Shutdown => {}
        }
    }

    fn index(&self, id: LibraryId) -> Option<usize> {
        self.libs.iter().position(|l| l.id() == id)
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// Runs `f` on the blocking pool and sends its message back (command steps; not counted
    /// against the library workers, so a stuck one holds up only its own command).
    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let _ = tx.blocking_send(f());
        });
    }

    /// Records a file as open in a tab, and tells its watcher (`remember`, R-T9-3).
    fn remember(&mut self, i: usize, rel: RelPath, stamp: Option<DiskStamp>) {
        let lib = &mut self.libs[i];
        if let (Some(w), Some(stamp)) = (&lib.watcher, stamp) {
            w.remember(rel.clone(), stamp);
        }
        lib.remembered.insert(rel, stamp);
    }

    // ---- registry ----------------------------------------------------------------------------

    fn attach(
        &mut self,
        path: PathBuf,
        db: Option<LogicalDb>,
        write_manifest: bool,
        reply: oneshot::Sender<Result<LibraryId, AttachError>>,
    ) {
        let token = self.token();
        let (fs, rules) = (self.opts.fs.clone(), self.rules.clone());
        let probe = path.clone();
        self.attaches.insert(
            token,
            PendingAttach {
                path,
                db,
                write_manifest,
                // ## m4b-t16-attach (named line)
                exact: None,
                root: None,
                identity: None,
                reply,
            },
        );
        self.spawn(move || Msg::AttachResolved {
            token,
            result: work::resolve_attach(fs.as_ref(), &probe, &rules),
        });
    }

    fn attach_resolved(&mut self, token: u64, result: Result<work::Valid, AttachError>) {
        let Some(mut pa) = self.attaches.remove(&token) else {
            return;
        };
        let resolved = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = pa.reply.send(Err(e));
                return;
            }
        };
        // F1: no file-system call here. The worker resolved the folder and made the guard's
        // checks; overlap is checked against the canonical paths this actor holds.
        let others = self.known_roots(None);
        let inode = resolved.identity.map(|i| i.ino);
        let root = match self.registry.add_resolved(
            &pa.path,
            &resolved.root.canonical,
            inode,
            &others,
            SystemTime::now(),
        ) {
            Ok(root) => root,
            Err(e) => {
                let _ = pa.reply.send(Err(AttachError::Registry(e)));
                return;
            }
        };
        pa.identity = resolved.identity;
        // ## m4b-t16-attach (named exception): the dialog's manifest, exactly.
        if let Some(exact) = pa.exact.clone() {
            let canonical = root.canonical.clone();
            pa.root = Some(root);
            self.attaches.insert(token, pa);
            self.spawn(move || Msg::AttachWritten {
                token,
                result: manifest::write_new(&canonical, &exact),
            });
            return;
        }
        // ## m4b-t16-attach
        if pa.write_manifest {
            let Some(db) = pa.db.clone().or_else(|| folder_db(&pa.path)) else {
                let _ = self.registry.remove(root.id);
                let _ = pa.reply.send(Err(AttachError::NoLogicalName));
                return;
            };
            let canonical = root.canonical.clone();
            pa.root = Some(root);
            self.attaches.insert(token, pa);
            self.spawn(move || Msg::AttachWritten {
                token,
                result: work::write_manifest(&canonical, db),
            });
            return;
        }
        self.finish_attach(pa, root);
    }

    fn attach_written(&mut self, token: u64, result: Result<(), manifest::WriteError>) {
        let Some(mut pa) = self.attaches.remove(&token) else {
            return;
        };
        let Some(root) = pa.root.take() else {
            return;
        };
        match result {
            Ok(()) => self.finish_attach(pa, root),
            Err(e) => {
                let _ = self.registry.remove(root.id);
                let _ = pa.reply.send(Err(AttachError::Manifest(e)));
            }
        }
    }

    fn finish_attach(&mut self, pa: PendingAttach, root: LibraryRoot) {
        let id = root.id;
        if let Err(e) = self.registry.save() {
            self.reload_registry();
            let _ = pa.reply.send(Err(AttachError::Registry(e)));
            return;
        }
        let Some(reg) = self.registry.library(id).cloned() else {
            let _ = pa
                .reply
                .send(Err(AttachError::Registry(RegistryError::UnknownLibrary)));
            return;
        };
        let mut lib = Lib::new(reg);
        lib.session.canonical = Some(root.canonical.clone());
        lib.session.identity = pa.identity;
        // I5: the watcher starts first (on a worker), then the scan (which re-validates the
        // root) is queued behind it.
        lib.want.watch = true;
        lib.want.scan = true;
        self.libs.push(lib);
        self.workspace_dirty = true;
        self.dirty.insert(id);
        self.dispatch();
        self.flush();
        let _ = pa.reply.send(Ok(id));
    }

    // ## m4b-t16-attach (named exception)
    fn attach_writing(
        &mut self,
        path: PathBuf,
        exact: manifest::ManifestV1,
        reply: oneshot::Sender<Result<LibraryId, AttachError>>,
    ) {
        let token = self.token();
        let (fs, rules) = (self.opts.fs.clone(), self.rules.clone());
        let probe = path.clone();
        self.attaches.insert(
            token,
            PendingAttach {
                path,
                db: Some(exact.database.clone()),
                write_manifest: true,
                exact: Some(exact),
                root: None,
                identity: None,
                reply,
            },
        );
        self.spawn(move || Msg::AttachResolved {
            token,
            result: work::resolve_attach(fs.as_ref(), &probe, &rules),
        });
    }

    /// R-T9-1: the user's watch choice, saved; a library that is up gets a new watcher of that
    /// kind (then a scan, as at the start).
    fn set_watch(&mut self, id: LibraryId, watch: WatchMode) -> Result<(), RegistryError> {
        let Some(i) = self.index(id) else {
            return Err(RegistryError::UnknownLibrary);
        };
        if self.libs[i].reg.watch == watch {
            return Ok(());
        }
        self.edit_registry(|reg| {
            if reg.set_watch(id, watch) {
                Ok(())
            } else {
                Err(RegistryError::UnknownLibrary)
            }
        })?;
        if let Some(reg) = self.registry.library(id).cloned() {
            self.libs[i].reg = reg;
        }
        if self.libs[i].up() {
            self.drop_watcher(i);
            let lib = &mut self.libs[i];
            lib.unwatched = false;
            lib.want.watch = true;
            lib.want.scan = true;
        }
        self.dirty.insert(id);
        Ok(())
    }

    /// T10 m-h: the user confirmed that the folder now at the library's path is the library.
    fn adopt_root(
        &mut self,
        id: LibraryId,
        seen: Option<crate::fileio::FolderIdentity>,
    ) -> Result<usize, RegistryError> {
        let Some(i) = self.index(id) else {
            return Err(RegistryError::UnknownLibrary);
        };
        // ## m4b-t16-attach fix round 1 (I3): only a folder refused for its identity (moved or
        // replaced) can be adopted; a link, an overlap or an app folder never can.
        let lib = &mut self.libs[i];
        if !adoptable(&lib.state, &lib.scan_diags) {
            return Err(RegistryError::UnknownLibrary);
        }
        // Nothing is recorded yet: the folder is validated with only the identity checks
        // waived (`RefreshJob::waived`), and the new identity is saved only if it passes
        // (`commit_adoption`). A refusal leaves the old record and `RootChanged`.
        lib.adopting = true;
        // ## m4b-t20-adversarial (named line, m4)
        lib.adopt_seen = seen;
        lib.want.validate = true;
        self.dirty.insert(id);
        Ok(i)
    }

    /// The adopted folder passed validation: its identity is recorded (and saved) and its
    /// binding is read from scratch (I2: nothing of the old folder's manifest is kept; only the
    /// user's local mappings, which belong to the library). `false` when the save failed.
    fn commit_adoption(&mut self, i: usize, v: &work::Valid) -> bool {
        // ## m4b-t20-adversarial (named lines, T16 re-review m4): only the folder the confirm
        // was shown for; another one swapped in meanwhile is refused, however valid.
        if let Some(seen) = self.libs[i].adopt_seen.take()
            && v.identity != Some(seen)
        {
            return false;
        }
        let id = self.libs[i].id();
        let canonical = v.root.canonical.clone();
        let inode = v.identity.map(|x| x.ino);
        let saved = self.edit_registry(|reg| {
            if reg.clear_resolved(id) && reg.set_resolved(id, &canonical, inode) {
                Ok(())
            } else {
                Err(RegistryError::UnknownLibrary)
            }
        });
        if saved.is_err() {
            return false;
        }
        if let Some(reg) = self.registry.library(id).cloned() {
            self.libs[i].reg = reg;
        }
        let lib = &mut self.libs[i];
        lib.session = Session {
            canonical: Some(canonical),
            identity: v.identity,
        };
        lib.manifest_good = false;
        lib.db = folder_db(&lib.reg.path).unwrap_or_else(LogicalDb::unbindable);
        lib.aliases = Arc::from([]);
        lib.manifest_diags.clear();
        self.dirty.insert(id);
        true
    }
    // ## m4b-t16-attach

    fn detach(&mut self, id: LibraryId) -> Result<(), RegistryError> {
        self.edit_registry(|reg| reg.remove(id))?;
        if let Some(i) = self.index(id) {
            // A job still running for it is abandoned: its slot is freed now (F2, R-T10-5)
            // and its late answer finds no library.
            if let Some(job) = self.libs[i].job {
                self.uncount(job);
            }
            self.drop_watcher(i);
            self.libs.remove(i);
        }
        self.git.remove(&id);
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

    fn map(
        &mut self,
        m: LocalMapping,
        share_alias: bool,
        // ## m4b-t16-attach (named line)
        expect: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<(), MapError>>,
    ) {
        if self.registry.library(m.library).is_none() {
            let _ = reply.send(Err(MapError::Registry(RegistryError::UnknownLibrary)));
            return;
        }
        if !share_alias {
            let r = self.set_mapping(m);
            self.flush();
            let _ = reply.send(r);
            return;
        }
        let alias = match LogicalDb::parse(m.database.as_str()) {
            Ok(alias) => alias,
            Err(e) => {
                let _ = reply.send(Err(MapError::BadName(e)));
                return;
            }
        };
        let root = self
            .index(m.library)
            .filter(|&i| self.libs[i].up())
            .and_then(|i| self.libs[i].root());
        let Some(root) = root else {
            let _ = reply.send(Err(MapError::Unavailable));
            return;
        };
        let token = self.token();
        self.aliases.insert(token, PendingMap { m, reply });
        // ## m4b-t16-attach (named lines): only over the bytes the dialog showed.
        self.spawn(move || Msg::AliasWritten {
            token,
            result: match &expect {
                Some(before) => manifest::add_alias_if_unchanged(&root.canonical, &alias, before),
                None => manifest::add_alias(&root.canonical, &alias),
            },
        });
    }

    fn alias_written(&mut self, token: u64, result: Result<(), manifest::WriteError>) {
        let Some(pm) = self.aliases.remove(&token) else {
            return;
        };
        if let Err(e) = result {
            let _ = pm.reply.send(Err(MapError::Alias(e)));
            return;
        }
        if let Some(i) = self.index(pm.m.library) {
            // The rescan reads the manifest again.
            self.libs[i].want.scan = true;
        }
        let r = self.set_mapping(pm.m);
        self.flush();
        let _ = pm.reply.send(r);
    }

    fn set_mapping(&mut self, m: LocalMapping) -> Result<(), MapError> {
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
        // F1: by text only; no library folder is touched.
        let (registry, _) = LibraryRegistry::load_text_only(&self.paths.libraries_toml());
        self.registry = registry;
    }

    /// The canonical roots this actor knows (this session's, else the recorded one, else the
    /// registered path), but `except`'s.
    fn known_roots(&self, except: Option<usize>) -> Vec<(LibraryId, PathBuf)> {
        self.libs
            .iter()
            .enumerate()
            .filter(|&(j, _)| Some(j) != except)
            .map(|(_, l)| {
                let root = l
                    .session
                    .canonical
                    .clone()
                    .or_else(|| l.reg.canonical.clone())
                    .unwrap_or_else(|| l.reg.path.clone());
                (l.id(), root)
            })
            .collect()
    }

    /// m-a: records a root's first resolution in `libraries.toml`, for the next launch.
    fn record_resolution(&mut self, i: usize) {
        let lib = &self.libs[i];
        let (Some(canonical), None) = (lib.session.canonical.clone(), &lib.reg.canonical) else {
            return;
        };
        let (id, inode) = (lib.id(), lib.session.identity.map(|i| i.ino));
        if self.registry.set_resolved(id, &canonical, inode) {
            if self.registry.save().is_err() {
                self.reload_registry();
                return;
            }
            if let Some(reg) = self.registry.library(id).cloned() {
                self.libs[i].reg = reg;
            }
        }
    }

    // ---- jobs --------------------------------------------------------------------------------

    /// Starts the next job of every idle library that has work, up to the worker cap.
    fn dispatch(&mut self) {
        let n = self.libs.len();
        if n == 0 {
            return;
        }
        let start = self.next_lib % n;
        self.next_lib = self.next_lib.wrapping_add(1);
        for k in 0..n {
            if self.in_flight >= self.opts.workers.max(1) {
                break;
            }
            let i = (start + k) % n;
            if self.libs[i].busy {
                continue;
            }
            if let Some(job) = self.next_job(i) {
                self.run_job(i, job);
            }
        }
    }

    fn next_job(&mut self, i: usize) -> Option<Job> {
        let others: Vec<(LibraryId, PathBuf)> = self
            .libs
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .filter_map(|(_, l)| Some((l.id(), l.session.canonical.clone()?)))
            .collect();
        let lib = &mut self.libs[i];
        // An unavailable library only re-validates, then watches and scans (that is what brings
        // it back); reported paths and stamps have nothing to look at.
        if matches!(lib.state, LibraryState::Unavailable(_)) {
            lib.want.probes.clear();
            lib.stamp_due.clear();
        }
        if !lib.want.validate && lib.want.watch {
            lib.want.watch = false;
            if let Some(root) = lib.root() {
                self.next_watch += 1;
                lib.watch_gen = self.next_watch;
                let (sink, mut batches) = mpsc::channel(WATCH_CHANNEL);
                let (tx, id, generation) = (self.tx.clone(), lib.id(), lib.watch_gen);
                // Forwards this watcher's batches, tagged with its generation, until it drops.
                self.rt.spawn(async move {
                    while let Some(batch) = batches.recv().await {
                        let msg = Msg::Watch {
                            id,
                            generation,
                            batch,
                        };
                        if tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                });
                // ## m4b-t16-attach (named lines; R-T9-1): the user's "poll" choice is kept.
                let backend = match lib.reg.watch {
                    WatchMode::Poll => Backend::Poll(PollReason::UserChoice),
                    WatchMode::Automatic => WatchOpts::default().backend,
                };
                let opts = WatchOpts {
                    window: self.opts.settle,
                    limits: self.opts.limits,
                    fs: self.opts.fs.clone(),
                    backend,
                    ..WatchOpts::default()
                };
                return Some(Job::Watch(Box::new(WatchJob {
                    root,
                    opts,
                    sources: self.sources.clone(),
                    sink,
                })));
            }
        }
        if lib.want.validate || lib.want.scan || !lib.want.probes.is_empty() {
            let validate_only = lib.want.validate;
            let scan = !validate_only && (lib.want.scan || lib.want.probes.len() > BURST_PATHS);
            let probes = if validate_only || scan {
                Vec::new()
            } else {
                std::mem::take(&mut lib.want.probes)
            };
            if validate_only {
                lib.want.validate = false;
            } else if scan {
                lib.want.scan = false;
                lib.want.probes.clear();
                // The scan reads the manifest too.
                lib.want.manifest = false;
            }
            // ## m4b-t16-attach fix round 1 (I3): a confirmed adoption's validation waives
            // only the identity checks.
            let waived = lib.adopting && validate_only;
            let mut reg = lib.reg.clone();
            if waived {
                reg.canonical = None;
                reg.inode = None;
            }
            let job = RefreshJob {
                reg,
                session: if waived {
                    Session::default()
                } else {
                    lib.session.clone()
                },
                waived,
                others,
                rules: self.rules.clone(),
                validate_only,
                scan,
                probes,
                old: lib.entries.clone(),
                stopped: lib.stopped,
                limits: self.opts.limits,
                fs: self.opts.fs.clone(),
            };
            return Some(Job::Refresh(Box::new(job)));
        }
        if std::mem::take(&mut lib.want.manifest)
            && let Some(root) = lib.root()
        {
            let identity = lib.session.identity;
            return Some(Job::Manifest(Box::new(ManifestJob { root, identity })));
        }
        if std::mem::take(&mut lib.want.pair)
            && let Some(job) = self.pair_job(i)
        {
            return Some(Job::Pair(Box::new(job)));
        }
        let lib = &mut self.libs[i];
        while let Some(rel) = lib.stamp_due.pop_front() {
            let (Some(root), Some(entry)) = (
                lib.root(),
                lib.entries.iter().find(|e| e.rel == rel).cloned(),
            ) else {
                continue;
            };
            let job = StampJob {
                root,
                entry,
                fs: self.opts.fs.clone(),
                stamps: self.stamps.clone(),
            };
            return Some(Job::Stamp(rel, Box::new(job)));
        }
        None
    }

    fn run_job(&mut self, i: usize, job: Job) {
        self.next_job_id += 1;
        let job_id = self.next_job_id;
        let lib = &mut self.libs[i];
        lib.busy = true;
        lib.job = Some(job_id);
        // ## m4b-t16-attach fix round 1 (I3)
        if matches!(&job, Job::Refresh(j) if j.waived) {
            lib.adopt_job = Some(job_id);
        }
        let (id, tx) = (lib.id(), self.tx.clone());
        self.in_flight += 1;
        self.jobs.insert(job_id, true);
        self.rt.spawn_blocking(move || {
            let msg = match job {
                Job::Refresh(job) => Msg::Refreshed {
                    id,
                    job: job_id,
                    out: job.run(),
                },
                Job::Stamp(rel, job) => Msg::Stamped {
                    id,
                    job: job_id,
                    rel,
                    out: job.run(),
                },
                Job::Watch(job) => Msg::WatchStarted {
                    id,
                    job: job_id,
                    result: job.run(),
                },
                Job::Manifest(job) => Msg::ManifestRead {
                    id,
                    job: job_id,
                    out: job.run(),
                },
                Job::Pair(job) => Msg::Paired {
                    id,
                    job: job_id,
                    out: job.run(),
                },
            };
            let _ = tx.blocking_send(msg);
        });
        self.after(self.opts.job_deadline, Msg::JobOverdue { job: job_id });
    }

    /// Stops counting `job` against the worker cap (it was abandoned or is overdue).
    fn uncount(&mut self, job: u64) {
        if let Some(counted) = self.jobs.get_mut(&job)
            && std::mem::take(counted)
        {
            self.in_flight = self.in_flight.saturating_sub(1);
        }
    }

    /// A job came back: frees its slot (if it still held one) and answers its library, unless
    /// the library is gone or this is not its job any more.
    fn job_done(&mut self, id: LibraryId, job: u64) -> Option<usize> {
        self.uncount(job);
        self.jobs.remove(&job);
        let i = self.index(id)?;
        let lib = &mut self.libs[i];
        if lib.job != Some(job) {
            return None;
        }
        lib.busy = false;
        lib.job = None;
        Some(i)
    }

    /// F2 (R-T10-5): a job past its deadline stops counting against the cap, so a hung mount
    /// cannot starve other libraries, and its library shows `Unavailable(NotResponding)`
    /// until the job returns (it keeps the library's one job, so no second one piles up).
    fn job_overdue(&mut self, job: u64) {
        if !self.jobs.get(&job).copied().unwrap_or(false) {
            return;
        }
        self.uncount(job);
        let Some(i) = self.libs.iter().position(|l| l.job == Some(job)) else {
            return;
        };
        self.make_unavailable(i, UnavailableReason::NotResponding, Vec::new());
    }

    fn refreshed(&mut self, i: usize, out: RefreshOut) {
        match out {
            RefreshOut::Refused(r) => {
                // ## m4b-t20-adversarial (named lines, m4): a new folder at the path is a new
                // snapshot even when the refusal reads the same.
                if self.libs[i].refused_seen != r.seen {
                    self.libs[i].refused_seen = r.seen;
                    let id = self.libs[i].id();
                    self.dirty.insert(id);
                }
                self.make_unavailable(i, r.reason, r.diags);
            }
            RefreshOut::Validated(v) => {
                let lib = &mut self.libs[i];
                lib.session.canonical = Some(v.root.canonical);
                lib.session.identity = lib.session.identity.or(v.identity);
                self.record_resolution(i);
                // I5: the watcher first, then the scan.
                let lib = &mut self.libs[i];
                lib.want.watch = lib.watcher.is_none();
                lib.want.scan = true;
            }
            RefreshOut::Unchanged { disagreed } => {
                if disagreed {
                    self.resync_later(i);
                }
            }
            RefreshOut::Listing(listing) => {
                let listing = *listing;
                // A full scan may have caught a moment the watcher never reports back from (a
                // file that came and went before it settled), and the watcher's events lag the
                // disk, so no event says so. The scan is confirmed by another one after a quiet
                // span, and again until a confirming scan finds nothing changed: the disk has
                // stopped moving. A library at rest costs one extra scan per full scan.
                //
                // Fix round 3 (m-g): the chain is paced (`chain_wait`): never sooner than four
                // times what the last full scan took (a large polled library is never rescanned
                // back to back), and past `MAX_CONFIRMS` confirming scans each wait doubles.
                let lib = &mut self.libs[i];
                let scanned = listing.scanned.is_some();
                if scanned {
                    lib.scan_cost = listing.cost;
                }
                let was_confirming = scanned && std::mem::take(&mut lib.confirming);
                if was_confirming {
                    lib.confirms = lib.confirms.saturating_add(1);
                }
                let stable = was_confirming && listing.changes.is_empty();
                if stable {
                    lib.confirms = 0;
                }
                let confirm = scanned && !stable;
                if listing.disagreed || confirm {
                    self.resync_later(i);
                }
                let id = self.libs[i].id();
                let was_up = self.libs[i].up();
                let lib = &mut self.libs[i];
                if let Some(v) = &listing.valid {
                    lib.session.canonical = Some(v.root.canonical.clone());
                    lib.session.identity = lib.session.identity.or(v.identity);
                }
                let mut changed = !listing.changes.is_empty();
                if lib.entries.as_ref() != listing.entries.as_slice() {
                    lib.entries = Arc::from(listing.entries);
                    changed = true;
                }
                let mut manifest = None;
                if let Some(s) = listing.scanned {
                    if lib.scan_diags != s.diags {
                        changed = true;
                    }
                    lib.scan_diags = s.diags;
                    lib.stopped = s.stopped;
                    lib.has_git_dir = s.has_git_dir;
                    if !was_up {
                        lib.state = lib.watched_state();
                        changed = true;
                    }
                    manifest = Some(s.manifest);
                }
                if let Some(m) = manifest {
                    changed |= self.apply_manifest(i, m);
                }
                if changed {
                    self.dirty.insert(id);
                }
                self.record_resolution(i);
                // A late answer after `NotResponding` (which dropped the watcher): watch again
                // and look once more, since the watch starts after this scan.
                let lib = &mut self.libs[i];
                if !was_up && lib.watcher.is_none() && !lib.unwatched {
                    lib.want.watch = true;
                    lib.want.scan = true;
                }
                self.announce(i, listing.changes, scanned, stable);
                if !was_up && self.libs[i].up() {
                    self.git_trigger_now(i);
                } else if changed {
                    self.git_trigger(i);
                }
            }
        }
    }

    /// Takes a manifest read (a scan's, or fix round 3's manifest-only read): the library's
    /// database name, aliases and manifest diagnostics. Whether anything changed.
    fn apply_manifest(&mut self, i: usize, m: ManifestOut) -> bool {
        let lib = &mut self.libs[i];
        // ## m4b-t16-attach (named exception; R-T10-7): a manifest that is there but cannot be
        // read now (a half-merged file, a `.hacivat` swapped for a link) keeps the binding the
        // last good read gave, with its diagnostic: it never falls back to the folder name,
        // which could bind another database.
        if m.unreadable && lib.manifest_good {
            // ## m4b-t16-attach fix round 1 (I1): and says so, with the binding it keeps.
            let mut diags = m.diags;
            diags.push(LibraryDiag::ManifestKept(lib.db.display_escaped()));
            let changed = lib.manifest_diags != diags;
            lib.manifest_diags = diags;
            return changed;
        }
        lib.manifest_good = m.good;
        // ## m4b-t16-attach
        let text = |n: &[LogicalDb]| n.iter().map(|x| x.as_str().to_owned()).collect::<Vec<_>>();
        let changed = lib.db.as_str() != m.db.as_str()
            || text(&lib.aliases) != text(&m.aliases)
            || lib.manifest_diags != m.diags;
        lib.db = m.db;
        lib.aliases = Arc::from(m.aliases);
        lib.manifest_diags = m.diags;
        changed
    }

    /// Turns listing changes into events, and queues the files open in a tab to be stamped
    /// (I4: no other file is read). A removal the watcher reported as a rename is not a delete.
    ///
    /// A *scan's* removal of a file open in a tab is held: the watcher may still report it as
    /// a rename (its events lag the disk). It is announced when the watcher says `Removed`
    /// (`FileDeleted`) or `Renamed` (`FileRenamed`), or once a stable confirming scan shows the
    /// file is gone. Fix round 3 (G1): then it is first paired with the files that appeared
    /// meanwhile, by length and content hash (a `Rescan` from the watcher, as for a `git
    /// checkout`, carries no rename): a match is `FileRenamed`, else `FileDeleted`.
    fn announce(&mut self, i: usize, changes: work::Changes, scanned: bool, stable: bool) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        let now = Instant::now();
        for rel in &changes.created {
            lib.held.remove(rel);
            lib.arrived
                .retain(|(r, at)| r != rel && now.duration_since(*at) < ARRIVAL_SPAN);
            if lib.arrived.len() >= MAX_ARRIVED {
                lib.arrived.pop_front();
            }
            lib.arrived.push_back((rel.clone(), now));
        }
        for (rel, len) in changes.removed {
            if lib.moved.contains_key(&rel) {
                continue;
            }
            if lib.remembered.contains_key(&rel) {
                // A scan's removal waits for the watcher's word or a stable scan; a patched
                // one (the watcher's report) is paired now.
                lib.held.insert(
                    rel,
                    Held {
                        len,
                        reported: !scanned,
                        unpaired: false,
                    },
                );
                if !scanned {
                    lib.want.pair = true;
                }
                continue;
            }
            Self::announce_deleted(&mut self.pending, lib, rel);
        }
        if stable && !lib.held.is_empty() {
            lib.want.pair = true;
        }
        // The renames whose old path the listing has dropped now.
        let gone: Vec<RelPath> = lib
            .moved
            .keys()
            .filter(|from| !lib.entries.iter().any(|e| &e.rel == *from))
            .cloned()
            .collect();
        for from in gone {
            if let Some(to) = lib.moved.remove(&from) {
                self.pending
                    .push(LibraryEvent::FileRenamed { id, from, to });
            }
        }
        let mut due = Vec::new();
        for rel in changes.created {
            if lib.deleted.remove(&rel) && lib.remembered.contains_key(&rel) {
                due.push(rel);
            }
        }
        due.extend(
            changes
                .changed
                .into_iter()
                .filter(|rel| lib.remembered.contains_key(rel)),
        );
        for rel in due {
            self.queue_stamp(i, rel);
        }
    }

    fn announce_deleted(pending: &mut Vec<LibraryEvent>, lib: &mut Lib, rel: RelPath) {
        if lib.deleted.len() >= RECENT_DELETES {
            lib.deleted.clear();
        }
        lib.deleted.insert(rel.clone());
        pending.push(LibraryEvent::FileDeleted { id: lib.id(), rel });
    }

    /// Fix round 3 (G1): the pairing of library `i`'s held removals, or `None` when no read is
    /// needed (each held file with a known content hash, against the listed files that
    /// appeared since and have its length; the others are answered at once).
    fn pair_job(&mut self, i: usize) -> Option<PairJob> {
        let lib = &mut self.libs[i];
        let root = lib.root()?;
        let mut held = Vec::new();
        let mut answered: Vec<(RelPath, Option<RelPath>)> = Vec::new();
        for (from, h) in &lib.held {
            match lib.remembered.get(from).copied().flatten() {
                Some(stamp) => held.push(HeldRemoval {
                    from: from.clone(),
                    len: h.len,
                    hash: stamp.hash,
                }),
                None => answered.push((from.clone(), None)),
            }
        }
        held.sort_by(|a, b| a.from.cmp(&b.from));
        let lens: HashSet<u64> = held.iter().map(|h| h.len).collect();
        let now = Instant::now();
        let candidates: Vec<ScriptEntry> = lib
            .arrived
            .iter()
            .filter(|(_, at)| now.duration_since(*at) < ARRIVAL_SPAN)
            .filter_map(|(rel, _)| lib.entries.iter().find(|e| &e.rel == rel))
            .filter(|e| lens.contains(&e.len) && !lib.remembered.contains_key(&e.rel))
            .cloned()
            .collect();
        if candidates.is_empty() {
            answered.extend(held.drain(..).map(|h| (h.from, None)));
        }
        if !answered.is_empty() {
            self.paired(i, answered);
        }
        if held.is_empty() {
            return None;
        }
        Some(PairJob {
            root,
            held,
            candidates,
            fs: self.opts.fs.clone(),
            stamps: self.stamps.clone(),
        })
    }

    /// Fix round 3 (G1): a pairing's answer. A held file that matched a file still listed moved
    /// there (`FileRenamed`); one that matched nothing is deleted, except while the library is
    /// polled: then it waits for the poller's report, a later pairing or the backstop.
    fn paired(&mut self, i: usize, out: Vec<(RelPath, Option<RelPath>)>) {
        let mut moved = Vec::new();
        let lib = &mut self.libs[i];
        let id = lib.id();
        let polled = matches!(lib.state, LibraryState::Polling(_));
        for (from, to) in out {
            if !lib.held.contains_key(&from) {
                // The watcher's word came first.
                continue;
            }
            if lib.entries.iter().any(|e| e.rel == from) {
                // It came back.
                lib.held.remove(&from);
                continue;
            }
            match to.filter(|to| lib.entries.iter().any(|e| &e.rel == to)) {
                Some(to) => {
                    lib.held.remove(&from);
                    lib.arrived.retain(|(r, _)| *r != to);
                    // The tab's stamp moves with it (the watcher hears it too, so it pairs a
                    // later move of the new name).
                    if let Some(known) = lib.remembered.remove(&from) {
                        moved.push((to.clone(), known));
                    }
                    if lib.renamed.len() >= RECENT_DELETES {
                        lib.renamed.clear();
                    }
                    lib.renamed.insert(from.clone(), to.clone());
                    self.pending
                        .push(LibraryEvent::FileRenamed { id, from, to });
                }
                None if polled && lib.held.get(&from).is_some_and(|h| !h.reported) => {
                    if let Some(h) = lib.held.get_mut(&from) {
                        h.unpaired = true;
                    }
                }
                None => {
                    lib.held.remove(&from);
                    Self::announce_deleted(&mut self.pending, lib, from);
                }
            }
        }
        for (to, known) in moved {
            self.remember(i, to, known);
        }
    }

    /// Queues a new stamp of an open file (once: repeated reports of one path collapse).
    fn queue_stamp(&mut self, i: usize, rel: RelPath) {
        let lib = &mut self.libs[i];
        if lib.remembered.contains_key(&rel) && !lib.stamp_due.contains(&rel) {
            lib.stamp_due.push_back(rel);
        }
    }

    /// An open file's new stamp: announced only when it differs from the one the tab knows (an
    /// own save, an echo, a `PossiblyChanged` that changed nothing: silent).
    fn stamped(&mut self, i: usize, rel: RelPath, out: StampOut) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        let Some(meta) = out.meta else {
            // Gone or no longer a plain file: the watcher's report patches the listing.
            return;
        };
        if let Some(pos) = lib.entries.iter().position(|e| e.rel == rel) {
            let e = &lib.entries[pos];
            if (e.len, e.mtime, e.dataless) != (meta.len, meta.mtime, meta.dataless) {
                let mut entries = lib.entries.to_vec();
                entries[pos].len = meta.len;
                entries[pos].mtime = meta.mtime;
                entries[pos].dataless = meta.dataless;
                lib.entries = Arc::from(entries);
                self.dirty.insert(id);
            }
        }
        let Some(stamp) = out.stamp else {
            return;
        };
        let known = lib.remembered.get(&rel).copied().flatten();
        if !lib.remembered.contains_key(&rel) || known == Some(stamp) {
            return;
        }
        self.remember(i, rel.clone(), Some(stamp));
        self.pending
            .push(LibraryEvent::FileChanged { id, rel, stamp });
    }

    // ---- the watcher -------------------------------------------------------------------------

    fn watch_started(&mut self, i: usize, result: Result<Watcher, WatchError>) {
        let id = self.libs[i].id();
        match result {
            Ok(w) => {
                // Its first look at the open files' stamps (T9 carry: `remember` after every
                // read; these were read before it started).
                for (rel, stamp) in &self.libs[i].remembered {
                    if let Some(stamp) = stamp {
                        w.remember(rel.clone(), *stamp);
                    }
                }
                self.echoes.set(id, Some(w.echo()));
                let lib = &mut self.libs[i];
                lib.unwatched = false;
                lib.watcher = Some(w);
            }
            // Not even a poll source would watch it: listed, nothing watches it.
            Err(_) => self.libs[i].unwatched = true,
        }
        self.update_backend(i);
    }

    /// Drops library `i`'s watcher off this thread (its thread is joined, bounded), and makes
    /// any batch it still sends stale.
    fn drop_watcher(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        self.next_watch += 1;
        lib.watch_gen = self.next_watch;
        self.echoes.set(lib.id(), None);
        if let Some(w) = lib.watcher.take() {
            self.rt.spawn_blocking(move || drop(w));
        }
    }

    /// The library's state follows its watcher's backend once it is up.
    fn update_backend(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        if lib.up() {
            let state = lib.watched_state();
            if lib.state != state {
                lib.state = state;
                self.dirty.insert(lib.id());
            }
        }
    }

    /// One batch of watch events (Task 9): reconciled already, so paths are probed to patch the
    /// listing, open files are re-stamped, and the backend's state is shown.
    fn watch_batch(&mut self, id: LibraryId, generation: u64, batch: WatchBatch) {
        let Some(i) = self.index(id) else { return };
        let lib = &mut self.libs[i];
        if lib.watch_gen != generation || matches!(lib.state, LibraryState::Unavailable(_)) {
            return;
        }
        lib.last_event = Some(Instant::now());
        let mut git = false;
        let mut stamp = Vec::new();
        for event in batch {
            match event {
                WatchEvent::Changed(rel) | WatchEvent::Unsettled(rel) => {
                    stamp.push(rel.clone());
                    lib.want.probes.push((rel, work::Expect::Present));
                }
                WatchEvent::Created(rel) => lib.want.probes.push((rel, work::Expect::Present)),
                WatchEvent::Removed(rel) => {
                    // A scan dropped it already, held for this word from the watcher. Fix
                    // round 3 (G1): after a rescan the watcher may not know where it went (it
                    // learned the new name silently), so it is paired first.
                    if let Some(h) = lib.held.get_mut(&rel) {
                        h.reported = true;
                        lib.want.pair = true;
                    }
                    lib.want.probes.push((rel, work::Expect::Absent));
                }
                // Its `(len, mtime)` did not change; only an open file needs a look, and when
                // the look finds the same stamp nobody hears of it (T9 carry).
                WatchEvent::PossiblyChanged(rel) => stamp.push(rel),
                WatchEvent::Renamed { from, to } => {
                    lib.want.probes.push((from.clone(), work::Expect::Either));
                    lib.want.probes.push((to.clone(), work::Expect::Either));
                    // Fix round 3 (G1): a pairing announced this move already.
                    if lib.renamed.get(&from) == Some(&to) {
                        lib.renamed.remove(&from);
                        continue;
                    }
                    if let Some(known) = lib.remembered.remove(&from) {
                        lib.remembered.insert(to.clone(), known);
                    }
                    let dropped = !lib.entries.iter().any(|e| e.rel == from);
                    if lib.held.remove(&from).is_some() || dropped {
                        // A scan already dropped the old path: the rename is news now.
                        self.pending
                            .push(LibraryEvent::FileRenamed { id, from, to });
                    } else {
                        lib.moved.insert(from, to);
                    }
                }
                WatchEvent::Rescan => lib.want.scan = true,
                WatchEvent::RepositoryChanged => git = true,
                // Fix round 3 (G2, R-T10-6): `.hacivat/workspace.toml` changed: a manifest-only
                // read re-binds the library (no scan).
                WatchEvent::ManifestChanged => lib.want.manifest = true,
                // The backend changed (lost, re-armed, nothing left): shown below; a re-armed
                // watch found a change nothing announced, so look again.
                WatchEvent::BackendLost(_) => {}
                WatchEvent::BackendRearmed => lib.want.scan = true,
            }
        }
        for rel in stamp {
            self.queue_stamp(i, rel);
        }
        self.update_backend(i);
        if git {
            self.git_trigger(i);
        }
    }

    /// The disk did not match the watcher's report: things are still changing. One rescan
    /// (coalesced) a few windows later sees where they ended, which the watcher may never
    /// report (a file that came back and went again before its settle).
    ///
    /// The same holds for a full scan made while the watcher was still reporting: it may have
    /// caught an intermediate state the watcher never reports back from.
    fn resync_later(&mut self, i: usize) {
        let wait = self.chain_wait(i);
        let lib = &mut self.libs[i];
        if lib.resync_due {
            return;
        }
        lib.resync_due = true;
        let (id, generation) = (lib.id(), lib.watch_gen);
        self.after(wait, Msg::Resync { id, generation });
    }

    /// Fix round 3 (m-g): the wait before library `i`'s next confirming scan: the quiet span,
    /// or four times its last full scan's cost if longer; doubled for each confirming scan of
    /// this chain past [`MAX_CONFIRMS`], up to [`MAX_CHAIN_WAIT`].
    ///
    /// The chain is not cut off after `MAX_CONFIRMS`: a hard stop left deleted files listed in
    /// 36 of 80 loaded H4 runs, since the chain is what brings the listing to what the watcher
    /// never reports (a file that came and went before it settled).
    fn chain_wait(&self, i: usize) -> Duration {
        let lib = &self.libs[i];
        let base = self.quiet_for().max(lib.scan_cost.saturating_mul(4));
        let past = u32::from(lib.confirms.saturating_sub(MAX_CONFIRMS - 1)).min(8);
        base.saturating_mul(1 << past).min(MAX_CHAIN_WAIT.max(base))
    }

    /// How long a library must have had no watch event before a resync scan is trusted.
    fn quiet_for(&self) -> Duration {
        self.opts.settle * 4
    }

    // ---- availability ------------------------------------------------------------------------

    fn make_unavailable(&mut self, i: usize, reason: UnavailableReason, diags: Vec<LibraryDiag>) {
        self.drop_watcher(i);
        let lib = &mut self.libs[i];
        // ## m4b-t16-attach fix round 1 (I2): a folder that is not this library's any more never
        // hands its "last good binding" to the folder that comes back at the path.
        if reason == UnavailableReason::RootChanged {
            lib.manifest_good = false;
        }
        let id = lib.id();
        let state = LibraryState::Unavailable(reason);
        let changed = lib.state != state || !lib.entries.is_empty() || lib.scan_diags != diags;
        lib.state = state;
        lib.entries = Arc::from([]);
        lib.scan_diags = diags;
        lib.stopped = false;
        lib.unwatched = false;
        lib.want = Want::default();
        lib.stamp_due.clear();
        lib.moved.clear();
        lib.held.clear();
        lib.arrived.clear();
        lib.renamed.clear();
        lib.confirms = 0;
        lib.confirming = false;
        if self.git.remove(&id).is_some() {
            self.workspace_dirty = true;
        }
        if changed {
            self.dirty.insert(id);
        }
        self.arm_timer(i, self.opts.recheck);
    }

    /// Starts a new recheck timer chain for library `i` (the old one, if any, is ignored).
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
        let lib = &mut self.libs[i];
        if lib.timer != Some(seq) {
            return;
        }
        lib.timer = None;
        if matches!(lib.state, LibraryState::Unavailable(_)) {
            lib.want.validate = true;
        }
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

    /// Refreshes the status of library `i` now (the first one after it comes up).
    fn git_trigger_now(&mut self, i: usize) {
        let lib = &mut self.libs[i];
        lib.git.seq += 1;
        let (id, seq) = (lib.id(), lib.git.seq);
        self.git_tick(id, seq);
    }

    /// m7: polled or unwatched libraries, and libraries whose repository's `.git` is above
    /// their root, get a status refresh every `git_backstop` (a commit there changes nothing in
    /// the library).
    ///
    /// Fix round 3: every library that is up also re-reads its manifest (G2's belt and braces),
    /// and a held removal a pairing could not match on a polled library is announced deleted
    /// (G1: its poller had a whole backstop period to report it).
    fn backstop(&mut self) {
        for i in 0..self.libs.len() {
            let lib = &mut self.libs[i];
            if lib.up() {
                lib.want.manifest = true;
            }
            let unpaired: Vec<RelPath> = lib
                .held
                .iter()
                .filter(|(_, h)| h.unpaired)
                .map(|(r, _)| r.clone())
                .collect();
            for rel in unpaired {
                lib.held.remove(&rel);
                Self::announce_deleted(&mut self.pending, lib, rel);
            }
            let lib = &self.libs[i];
            let not_watched = matches!(
                lib.state,
                LibraryState::Polling(_) | LibraryState::Unwatched
            );
            if lib.up() && (not_watched || !lib.has_git_dir) {
                self.git_trigger_now(i);
            }
        }
        self.after(self.opts.git_backstop, Msg::Backstop);
    }

    fn git_tick(&mut self, id: LibraryId, seq: u64) {
        let Some(i) = self.index(id) else { return };
        let lib = &mut self.libs[i];
        if lib.git.seq != seq || !lib.up() {
            return;
        }
        if lib.git.in_flight {
            lib.git.again = true;
            return;
        }
        let Some(root) = lib.session.canonical.clone() else {
            return;
        };
        lib.git.in_flight = true;
        lib.git.again = false;
        let limits = self.opts.git;
        self.spawn(move || Msg::GitDone {
            id,
            state: hacivat_git::status(&root, &limits),
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

    fn after(&self, delay: Duration, msg: Msg) {
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = tx.send(msg).await;
        });
    }

    /// No job running or queued, no open file waiting for its stamp, no attach under way.
    fn quiet(&self) -> bool {
        self.attaches.is_empty()
            && self.aliases.is_empty()
            && self
                .libs
                .iter()
                .all(|l| !l.busy && l.want.is_empty() && l.stamp_due.is_empty() && !l.resync_due)
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
        if self.quiet() {
            for waiter in self.waiters.drain(..) {
                let _ = waiter.send(());
            }
        }
    }
}

// ## m4b-t10-service

// ## m4b-t16-attach fix round 1 (I3)
/// Whether "Use this folder" can fix the library: it is `RootChanged` only because the folder at
/// its path is another one (moved or replaced), never a link, a non-folder, an overlap or an app
/// folder.
pub(super) fn adoptable(state: &LibraryState, diags: &[LibraryDiag]) -> bool {
    *state == LibraryState::Unavailable(UnavailableReason::RootChanged)
        && diags.iter().any(|d| {
            matches!(
                d,
                LibraryDiag::RootRefused(RootRefusal::Moved | RootRefusal::IdentityChanged)
            )
        })
        && !diags.iter().any(|d| {
            matches!(
                d,
                LibraryDiag::RootRefused(
                    RootRefusal::Link
                        | RootRefusal::NotAFolder
                        | RootRefusal::FilesystemRoot
                        | RootRefusal::HomeFolder
                        | RootRefusal::AppFolder
                        | RootRefusal::Overlaps(_)
                )
            )
        })
}
// ## m4b-t16-attach
