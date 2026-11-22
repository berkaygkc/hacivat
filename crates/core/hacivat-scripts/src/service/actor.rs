// ## m4b-t10-service (fix round 1: the actor moved here, with its worker pool)
//! The service's actor: one long-lived task on the core runtime's blocking pool that owns every
//! piece of mutable state and handles one message at a time. It does no library I/O itself
//! (I6, R-T10-4): each library's validation, scans, probes, settle looks and stamps run as jobs
//! on at most `ServiceOptions::workers` blocking workers, one job per library at a time, and
//! come back as messages. Work that arrives while a library's job runs waits in that library's
//! queue (coalesced: one rescan, the hinted paths, the due settle looks), so a watch started
//! before the first scan loses nothing (I5). A job of a library that was detached meanwhile is
//! dropped when it returns; one that never returns holds only its own worker.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use arc_swap::ArcSwap;
use hacivat_git::GitState;
use hacivat_proto::ProfileId;
use hacivat_settings::paths::LocalPaths;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::work::{self, RefreshJob, RefreshOut, Rules, Session, SettleJob, SettleOut};
use super::{
    AttachError, LibraryDiag, LibrarySnapshot, LibraryState, MapError, PollReason, ServiceOptions,
    UnavailableReason, WorkspaceSnapshot, servable,
};
use crate::events::LibraryEvent;
use crate::fileio::DiskStamp;
use crate::manifest;
use crate::names::{DatabaseName, LogicalDb};
use crate::pathsafe::{self, RelPath};
use crate::recovery;
use crate::registry::{
    LibraryId, LibraryRegistry, LibraryRoot, LocalMapping, RegisteredLibrary, RegistryDiag,
    RegistryError,
};
use crate::scan::ScriptEntry;
use crate::watch::{EventSource, HintSink, RawHint, WatchError};

/// The actor's inbox. A hint batch that does not fit is dropped and its library rescanned.
pub(super) const INBOX_CAPACITY: usize = 1024;
/// What a registry call answers once the service has stopped.
pub(super) const STOPPED: RegistryError = RegistryError::Io(io::ErrorKind::BrokenPipe);
/// The most messages handled before a publish.
const DRAIN_MAX: usize = 64;
/// A batch with more paths than this rescans instead (HV-SCRIPT-08 step 3).
const BURST_PATHS: usize = 1_000;
/// The most settle rounds before a changing file is stamped anyway (HV-SCRIPT-08 step 5).
const SETTLE_TRIES: u32 = 10;
/// How many deleted paths per library are remembered, so their return is announced.
const RECENT_DELETES: usize = 1_024;

/// An event source for when the OS one cannot be built: nothing is watched, every library is
/// polled.
pub(super) struct NoSource;

impl EventSource for NoSource {
    fn watch(&mut self, _root: &Path, _sink: HintSink) -> Result<(), WatchError> {
        Err(WatchError::Backend("no event source".into()))
    }

    fn unwatch(&mut self, _root: &Path) {}
}

/// The stamps of Hacivat's own saves, recorded before the rename (HV-SCRIPT-08 step 6). Written
/// by the save's blocking task, read by the actor; the lock is never held across an `.await`.
#[derive(Debug, Clone, Default)]
pub(super) struct Echo(Arc<Mutex<EchoTable>>);

/// Each saved path's last own stamp and when it was recorded.
type EchoTable = HashMap<(LibraryId, RelPath), (DiskStamp, Instant)>;

impl Echo {
    fn lock(&self) -> std::sync::MutexGuard<'_, EchoTable> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn record(&self, id: LibraryId, rel: &RelPath, stamp: DiskStamp) {
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
    /// A tab read the file: it is open, with this content hash (I4, rename pairing).
    Remember {
        id: LibraryId,
        rel: RelPath,
        hash: Option<u128>,
    },
    /// A save wrote the file (`hash` when it is known).
    Touched {
        id: LibraryId,
        rel: RelPath,
        hash: Option<u128>,
    },
    /// A changed file's settle window passed.
    SettleDue {
        id: LibraryId,
        rel: RelPath,
    },
    /// The poll or recheck timer of a library.
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
    SettleDone {
        id: LibraryId,
        job: u64,
        rel: RelPath,
        out: SettleOut,
    },
    /// Fix round 2 (F2, R-T10-5): a job ran past `ServiceOptions::job_deadline`.
    JobOverdue {
        job: u64,
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
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    Settled {
        reply: oneshot::Sender<()>,
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
    pub echo: Echo,
    pub stamps: Arc<AtomicU64>,
    pub source: Box<dyn EventSource>,
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
            echo: w.echo,
            stamps: w.stamps,
            source: w.source,
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
            attaches: HashMap::new(),
            aliases: HashMap::new(),
            waiters: Vec::new(),
        };
        actor.run(w.inbox, diags);
    });
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

/// Work queued for a library's next job.
#[derive(Debug, Default)]
struct Want {
    validate: bool,
    scan: bool,
    probes: Vec<RelPath>,
}

impl Want {
    fn is_empty(&self) -> bool {
        !self.validate && !self.scan && self.probes.is_empty()
    }
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
    scan_diags: Vec<LibraryDiag>,
    entries: Arc<[ScriptEntry]>,
    stopped: bool,
    has_git_dir: bool,
    watched: Option<PathBuf>,
    /// The source would not watch it, or stopped: it is polled.
    polling: bool,
    /// Set by the source's sink when a batch did not fit the inbox.
    overflow: Arc<AtomicBool>,
    /// The poll or recheck timer chain that is live, if any.
    timer: Option<u64>,
    next_timer: u64,
    git: GitRun,
    /// A job of this library is running.
    busy: bool,
    /// Its id (F2: an answer from an abandoned job is ignored).
    job: Option<u64>,
    want: Want,
    settles: HashMap<RelPath, Settle>,
    settle_due: VecDeque<RelPath>,
    /// Files open in a tab, with their last known content hash (I4: only these are read).
    remembered: HashMap<RelPath, Option<u128>>,
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
        });
        Lib {
            reg,
            session: Session::default(),
            state: LibraryState::Scanning,
            db,
            aliases: Arc::from([]),
            manifest_diags: Vec::new(),
            scan_diags: Vec::new(),
            entries: Arc::from([]),
            stopped: false,
            has_git_dir: true,
            watched: None,
            polling: false,
            overflow: Arc::new(AtomicBool::new(false)),
            timer: None,
            next_timer: 0,
            git: GitRun::default(),
            busy: false,
            job: None,
            want: Want::default(),
            settles: HashMap::new(),
            settle_due: VecDeque::new(),
            remembered: HashMap::new(),
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
            LibraryState::Watching | LibraryState::Polling(_)
        )
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
        })
    }
}

/// The logical name the folder name gives, when it is a valid one.
fn folder_db(path: &Path) -> Option<LogicalDb> {
    LogicalDb::parse(path.file_name()?.to_str()?).ok()
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
    root: Option<LibraryRoot>,
    identity: Option<(u64, u64)>,
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
    Settle(RelPath, Box<SettleJob>),
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
    stamps: Arc<AtomicU64>,
    source: Box<dyn EventSource>,
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
            self.check_overflow();
            self.dispatch();
            self.flush();
        }
        for lib in &mut self.libs {
            if let Some(root) = lib.watched.take() {
                self.source.unwatch(&root);
            }
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
            Msg::Detach { id, reply } => {
                let r = self.detach(id);
                self.flush();
                let _ = reply.send(r);
            }
            Msg::Map {
                m,
                share_alias,
                reply,
            } => self.map(m, share_alias, reply),
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
            Msg::Hints { id, batch } => self.hints(id, batch),
            Msg::Remember { id, rel, hash } => {
                // m6: nothing is kept for a library that is gone.
                if let Some(i) = self.index(id) {
                    self.libs[i].remembered.insert(rel, hash);
                }
            }
            Msg::Touched { id, rel, hash } => {
                if let Some(i) = self.index(id) {
                    let lib = &mut self.libs[i];
                    let known = lib.remembered.entry(rel.clone()).or_default();
                    if hash.is_some() {
                        *known = hash;
                    }
                    lib.want.probes.push(rel);
                    self.git_trigger(i);
                }
            }
            Msg::SettleDue { id, rel } => {
                if let Some(i) = self.index(id) {
                    let lib = &mut self.libs[i];
                    if lib.settles.contains_key(&rel) && !lib.settle_due.contains(&rel) {
                        lib.settle_due.push_back(rel);
                    }
                }
            }
            Msg::Tick { id, seq } => self.tick(id, seq),
            Msg::Backstop => self.backstop(),
            Msg::GitTick { id, seq } => self.git_tick(id, seq),
            Msg::GitDone { id, state } => self.git_done(id, state),
            Msg::Refreshed { id, job, out } => {
                if let Some(i) = self.job_done(id, job) {
                    self.refreshed(i, out);
                }
            }
            Msg::SettleDone { id, job, rel, out } => {
                if let Some(i) = self.job_done(id, job) {
                    self.settle_done(i, rel, out);
                }
            }
            Msg::JobOverdue { job } => self.job_overdue(job),
            Msg::Settled { reply } => self.waiters.push(reply),
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
        let inode = resolved.identity.map(|(_, ino)| ino);
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
        self.libs.push(lib);
        let i = self.libs.len() - 1;
        // I5: the watch starts before the first scan; the scan (which re-validates the root)
        // is queued behind it.
        self.ensure_watch(i);
        self.libs[i].want.scan = true;
        self.workspace_dirty = true;
        self.dirty.insert(id);
        self.dispatch();
        self.flush();
        let _ = pa.reply.send(Ok(id));
    }

    fn detach(&mut self, id: LibraryId) -> Result<(), RegistryError> {
        self.edit_registry(|reg| reg.remove(id))?;
        if let Some(i) = self.index(id) {
            // A job still running for it is abandoned: its slot is freed now (F2, R-T10-5)
            // and its late answer finds no library.
            if let Some(job) = self.libs[i].job {
                self.uncount(job);
            }
            let mut lib = self.libs.remove(i);
            if let Some(root) = lib.watched.take() {
                self.source.unwatch(&root);
            }
        }
        self.git.remove(&id);
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

    fn map(
        &mut self,
        m: LocalMapping,
        share_alias: bool,
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
        self.spawn(move || Msg::AliasWritten {
            token,
            result: manifest::add_alias(&root.canonical, &alias),
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
        let (id, inode) = (lib.id(), lib.session.identity.map(|(_, ino)| ino));
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
        // An unavailable library only re-validates and then scans (its scan is what brings it
        // back); hinted paths and settle looks have nothing to look at.
        if matches!(lib.state, LibraryState::Unavailable(_)) {
            lib.want.probes.clear();
            lib.settle_due.clear();
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
            }
            let job = RefreshJob {
                reg: lib.reg.clone(),
                session: lib.session.clone(),
                others,
                rules: self.rules.clone(),
                validate_only,
                scan,
                probes,
                old: lib.entries.clone(),
                stopped: lib.stopped,
                limits: self.opts.limits,
                remembered: lib
                    .remembered
                    .iter()
                    .filter_map(|(rel, hash)| Some((rel.clone(), (*hash)?)))
                    .collect(),
                fs: self.opts.fs.clone(),
                stamps: self.stamps.clone(),
            };
            return Some(Job::Refresh(Box::new(job)));
        }
        while let Some(rel) = lib.settle_due.pop_front() {
            let (Some(s), Some(root)) = (lib.settles.get(&rel).copied(), lib.root()) else {
                lib.settles.remove(&rel);
                continue;
            };
            let Some(entry) = lib.entries.iter().find(|e| e.rel == rel).cloned() else {
                lib.settles.remove(&rel);
                continue;
            };
            let job = SettleJob {
                root,
                entry,
                len: s.len,
                mtime: s.mtime,
                last_try: s.tries + 1 >= SETTLE_TRIES,
                stamp: lib.remembered.contains_key(&rel),
                fs: self.opts.fs.clone(),
                stamps: self.stamps.clone(),
            };
            return Some(Job::Settle(rel, Box::new(job)));
        }
        None
    }

    fn run_job(&mut self, i: usize, job: Job) {
        self.next_job_id += 1;
        let job_id = self.next_job_id;
        let lib = &mut self.libs[i];
        lib.busy = true;
        lib.job = Some(job_id);
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
                Job::Settle(rel, job) => Msg::SettleDone {
                    id,
                    job: job_id,
                    rel,
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
            RefreshOut::Refused(r) => self.make_unavailable(i, r.reason, r.diags),
            RefreshOut::Validated(v) => {
                let lib = &mut self.libs[i];
                lib.session.canonical = Some(v.root.canonical);
                lib.session.identity = lib.session.identity.or(v.identity);
                self.record_resolution(i);
                // I5: the watch first, then the scan.
                self.ensure_watch(i);
                self.libs[i].want.scan = true;
            }
            RefreshOut::Unchanged => {}
            RefreshOut::Listing(listing) => {
                let listing = *listing;
                let id = self.libs[i].id();
                let was_up = self.libs[i].up();
                let lib = &mut self.libs[i];
                if let Some(v) = &listing.valid {
                    lib.session.canonical = Some(v.root.canonical.clone());
                    lib.session.identity = lib.session.identity.or(v.identity);
                }
                let mut changed = !listing.changes.is_empty() || !listing.renames.is_empty();
                if lib.entries.as_ref() != listing.entries.as_slice() {
                    lib.entries = Arc::from(listing.entries);
                    changed = true;
                }
                if let Some(s) = listing.scanned {
                    let text = |n: &[LogicalDb]| {
                        n.iter().map(|x| x.as_str().to_owned()).collect::<Vec<_>>()
                    };
                    if lib.db.as_str() != s.manifest.db.as_str()
                        || text(&lib.aliases) != text(&s.manifest.aliases)
                        || lib.manifest_diags != s.manifest.diags
                        || lib.scan_diags != s.diags
                    {
                        changed = true;
                    }
                    lib.db = s.manifest.db;
                    lib.aliases = Arc::from(s.manifest.aliases);
                    lib.manifest_diags = s.manifest.diags;
                    lib.scan_diags = s.diags;
                    lib.stopped = s.stopped;
                    lib.has_git_dir = s.has_git_dir;
                    if !was_up {
                        lib.state = if lib.polling {
                            LibraryState::Polling(PollReason::BackendLost)
                        } else {
                            LibraryState::Watching
                        };
                        changed = true;
                    }
                }
                if changed {
                    self.dirty.insert(id);
                }
                self.record_resolution(i);
                // A late answer after `NotResponding` (which unwatched it): watch again and
                // look once more, since the watch starts after this scan.
                if !was_up && self.libs[i].watched.is_none() && !self.libs[i].polling {
                    self.ensure_watch(i);
                    self.libs[i].want.scan = true;
                }
                self.announce(i, listing.changes, listing.renames);
                if !was_up && self.libs[i].up() {
                    self.git_trigger_now(i);
                } else if changed {
                    self.git_trigger(i);
                }
            }
        }
    }

    /// Turns listing changes into events (renames were paired by the job), and queues the
    /// files open in a tab to settle and be stamped (I4: no other file is read).
    fn announce(
        &mut self,
        i: usize,
        changes: work::Changes,
        renames: Vec<(RelPath, RelPath, u128)>,
    ) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        for (from, to, hash) in renames {
            lib.remembered.remove(&from);
            lib.remembered.insert(to.clone(), Some(hash));
            lib.settles.remove(&from);
            self.pending
                .push(LibraryEvent::FileRenamed { id, from, to });
        }
        for (rel, _) in changes.removed {
            lib.settles.remove(&rel);
            if lib.deleted.len() >= RECENT_DELETES {
                lib.deleted.clear();
            }
            lib.deleted.insert(rel.clone());
            self.pending.push(LibraryEvent::FileDeleted { id, rel });
        }
        let mut settle = Vec::new();
        for rel in changes.created {
            if lib.deleted.remove(&rel) && lib.remembered.contains_key(&rel) {
                settle.push(rel);
            }
        }
        settle.extend(
            changes
                .changed
                .into_iter()
                .filter(|rel| lib.remembered.contains_key(rel)),
        );
        for rel in settle {
            self.queue_settle(i, rel);
        }
    }

    // ---- settle ------------------------------------------------------------------------------

    fn queue_settle(&mut self, i: usize, rel: RelPath) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        let Some((len, mtime)) = lib
            .entries
            .iter()
            .find(|e| e.rel == rel)
            .map(|e| (e.len, e.mtime))
        else {
            return;
        };
        match lib.settles.get_mut(&rel) {
            // Already waiting: its look compares against the newest metadata.
            Some(s) => {
                s.len = len;
                s.mtime = mtime;
            }
            None => {
                lib.settles.insert(
                    rel.clone(),
                    Settle {
                        len,
                        mtime,
                        tries: 0,
                    },
                );
                self.after(self.opts.settle, Msg::SettleDue { id, rel });
            }
        }
    }

    fn settle_done(&mut self, i: usize, rel: RelPath, out: SettleOut) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        let Some(mut s) = lib.settles.remove(&rel) else {
            return;
        };
        let Some(meta) = out.meta else {
            // Gone or no longer a plain file: the hint that says so patches the listing.
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
        if !out.still && s.tries + 1 < SETTLE_TRIES {
            s.len = meta.len;
            s.mtime = meta.mtime;
            s.tries += 1;
            lib.settles.insert(rel.clone(), s);
            self.after(self.opts.settle, Msg::SettleDue { id, rel });
            return;
        }
        let Some(stamp) = out.stamp else {
            return;
        };
        lib.remembered.insert(rel.clone(), Some(stamp.hash));
        if self.echo.is_echo(id, &rel, &stamp, self.opts.echo_ttl) {
            return;
        }
        self.pending
            .push(LibraryEvent::FileChanged { id, rel, stamp });
    }

    // ---- availability ------------------------------------------------------------------------

    fn make_unavailable(&mut self, i: usize, reason: UnavailableReason, diags: Vec<LibraryDiag>) {
        let lib = &mut self.libs[i];
        let id = lib.id();
        if let Some(root) = lib.watched.take() {
            self.source.unwatch(&root);
        }
        let state = LibraryState::Unavailable(reason);
        let changed = lib.state != state || !lib.entries.is_empty() || lib.scan_diags != diags;
        lib.state = state;
        lib.entries = Arc::from([]);
        lib.scan_diags = diags;
        lib.stopped = false;
        lib.polling = false;
        lib.want = Want::default();
        lib.settles.clear();
        lib.settle_due.clear();
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
        if lib.watched.is_some() || lib.polling {
            return;
        }
        let Some(root) = lib.session.canonical.clone() else {
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
                lib.timer = None;
            }
            Err(_) => {
                lib.polling = true;
                if lib.up() {
                    lib.state = LibraryState::Polling(PollReason::BackendLost);
                }
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
        lib.polling = true;
        if lib.up() {
            lib.state = LibraryState::Polling(PollReason::BackendLost);
        }
        lib.want.scan = true;
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
        let lib = &mut self.libs[i];
        if lib.timer != Some(seq) {
            return;
        }
        lib.timer = None;
        match lib.state {
            LibraryState::Unavailable(_) => lib.want.validate = true,
            _ if lib.polling => {
                lib.want.scan = true;
                let poll = self.opts.poll;
                self.arm_timer(i, poll);
            }
            _ => {}
        }
    }

    // ---- hints -------------------------------------------------------------------------------

    /// Handles one batch of hints for library `id`: queued for its next job (I5: hints that
    /// arrive while a scan runs are probed after it).
    fn hints(&mut self, id: LibraryId, batch: Vec<RawHint>) {
        let Some(i) = self.index(id) else { return };
        if matches!(self.libs[i].state, LibraryState::Unavailable(_)) {
            return;
        }
        let Some(root) = self.libs[i].root() else {
            return;
        };
        let mut git = false;
        for hint in batch {
            match hint {
                RawHint::Lost => self.lose_backend(i),
                RawHint::Rescan(_) => self.libs[i].want.scan = true,
                RawHint::Path(p) => match classify(&root, &p) {
                    HintPath::Root | HintPath::Manifest => self.libs[i].want.scan = true,
                    HintPath::Git => git = true,
                    HintPath::Rel(rel) => self.libs[i].want.probes.push(rel),
                    HintPath::Ignore => {}
                },
            }
        }
        if git {
            self.git_trigger(i);
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

    /// m7: polled libraries and libraries whose repository's `.git` is above their root get a
    /// status refresh every `git_backstop` (a commit there changes nothing in the library).
    fn backstop(&mut self) {
        for i in 0..self.libs.len() {
            let lib = &self.libs[i];
            if lib.up() && (lib.polling || !lib.has_git_dir) {
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

    /// Rescans every library whose source dropped a batch (the inbox was full).
    fn check_overflow(&mut self) {
        for lib in &mut self.libs {
            if lib.overflow.swap(false, Ordering::AcqRel) && lib.up() {
                lib.want.scan = true;
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

    /// No job running or queued, no file waiting to settle, no attach under way.
    fn quiet(&self) -> bool {
        self.attaches.is_empty()
            && self.aliases.is_empty()
            && self.libs.iter().all(|l| {
                !l.busy && l.want.is_empty() && l.settles.is_empty() && l.settle_due.is_empty()
            })
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
