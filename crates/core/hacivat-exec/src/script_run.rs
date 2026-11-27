// ## m4b-t01a-seams
// ## m4b-t12-script-run (the seam's body; ADR 0006 `m4b-t12`, fix round 1)
//! "Run as script" (HV-SCRIPT-12, the reduced HV-EXEC-15; R-M4b-20, R-M4b-24, R-M4b-47,
//! R-T8-1, R-T8-3, R-T10-3, R-T12-2): a script too large for the editor streams from disk
//! through the `StreamSplitter`, and every unit is classified, gated and confirmed as a Run's
//! are, one at a time, on the session the tab was opened under.
//!
//! **Two passes (R-M4b-47).** Pass 1 streams the whole file with `StreamPolicy::Eager` and runs
//! nothing. The **whole file** is refused, as M3 refuses a whole script, when:
//! - an item refuses (`stream::any_refuses` on every `feed` batch and on `finish`: an uncertain
//!   boundary, a statement that is not `Exact`, `TooLong`, `NoBoundary`);
//! - an item is an unsupported client command, a `COPY … FROM STDIN`/`TO STDOUT`, or a statement
//!   with a parameter;
//! - the session's lexing modes are unknown and the file splits or prepares differently under
//!   any mode combination that changes how the dialect lexes (M3's rule, `ModesDiffer`);
//! - the gate, under the session's state at the start and the live environment tag, denies a
//!   unit (R-T12-2: read-only connections, the classifier's and the policy's denials, everything
//!   decided without the user). Units are classified in the starting context, and after a unit
//!   that may change the context in a context the classifier reads as unknown;
//! - a read fails.
//!
//! Only a clean pass 1 opens pass 2 ([`ScriptChunks::reopen_verified`]), which runs. Every
//! dialect gets both passes: `Eager` is `Exact` off MySQL/MariaDB (R-T8-4).
//!
//! **Identical chunks (R-T8-1; supersedes the T10 carry "one `feed` per `next_chunk()`").** The
//! splitter is fed one `feed` per chunk of [`Rechunk`], which cuts the text the source hands out
//! into [`SCRIPT_CHUNK_BYTES`] pieces at a character boundary: the chunks are a function of the
//! text alone, whatever reads the source made. Pass 1 records each chunk's length and SHA-256;
//! pass 2 checks every chunk against that record **before** feeding it, and checks that it ends
//! where pass 1 ended: a difference stops the run (`FileChanged`).
//!
//! **Change detection (R-T10-3).** The source owns the stamp: its `next_chunk` re-checks the
//! open file's stamp per read and answers `ChunkError::Changed` once it moved, and
//! `reopen_verified` refuses a file that is not the one pass 1 read. A change between the
//! passes refuses the file (nothing ran); a change in pass 2 stops the run before the next unit.
//!
//! **Held text (T8 m-fr3-3, R-T8-3).** Off MySQL/MariaDB the stream is `Exact`, which holds a
//! line with no boundary whole: once it holds more than [`STREAM_HELD_MAX`] (64 MiB) or more
//! than [`STREAM_HELD_ITEMS_MAX`] items the file is refused with `NoBoundary`.
//!
//! **Per unit (pass 2), as Run.** Before every unit the runner reads the session's context
//! (`run_context`, answered after the previous unit's request) and stops when the connection
//! was lost or reset since the run began (`SessionEvent::Lost`/`Reconnected`, a session that is
//! reconnecting: the new connection lost what the file set, R-M4b-27), when the session now
//! lexes otherwise than the file was split (M3's rule: both readings known and different), or,
//! on production or read-only, when the context could not be read. The unit is classified on
//! the pool in that context, gated with the connection as it is now (the live environment tag,
//! the read-only flag, the transaction status and mode), asked about through the same
//! [`ConfirmPort`] (Execute all never answers a typed name and covers nothing on production;
//! Skip all skips; "don't ask again" only where the dialog offers it; Cancel stops the run and
//! the units the user confirmed before it stay applied), and sent through [`ExecService`] as a
//! checked one-unit plan carrying the context generation it was classified under. Right before
//! the send the tag, the read-only flag and the connection are checked again. Results are not
//! kept. The service's log lines of each unit reach the console's log numbered by the unit's
//! place in the file, with one summary line for the run. Stop works before every chunk and
//! unit, during the context read, the classification and a dialog, and cancels a unit in
//! flight. Every unit stops the run at its first failure: a streamed run asks no question per
//! unit (R-M3-61's dialog needs units still waiting in one run). A transaction left open stays
//! open; the session's state carries it to the M3 transaction UI, and the summary says so.
//!
//! **History (R-M4b-24).** One summary row (origin `File`, the file's name as its text), never a
//! row per unit.
//!
//! **Memory, per shape.** An INSERT dump runs in constant memory: the splitter holds about one
//! statement (T8), plus one chunk being cut, the line-count text from the last unit's start, 40
//! bytes of pass record per 64 KiB, and the session's per-unit buffers. The shapes pass 1
//! refuses cost, before the refusal, the splitter's bound (T8: at most 64 MiB of text and
//! 65,536 items held; about 287 MB RSS on MySQL's worst shapes; off MySQL the item cap keeps a
//! line of tiny statements under about 100 MB) plus up to 64 MiB of line-count text. With the
//! modes unknown, pass 1 runs one splitter per mode combination (2 on PostgreSQL, 4 on MySQL, 8
//! on MariaDB), and the splitter's share multiplies by that many.
//!
//! [`ScriptChunks`] is the source: `hacivat_scripts::LibraryService::open_stream` returns that
//! crate's own reader, which the app adapts (neither crate depends on the other, R-M4b-28).

use std::fmt;
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use hacivat_proto::{DbmsKind, EnvTag, ProfileId};
use hacivat_results::FetchPolicy;
use hacivat_session::session::{
    ClassifierContext, ErrorPolicy, RunSummary, SessionEvent, SessionHandle, SessionSnapshot,
    SessionState, UnitId,
};
use sha2::{Digest, Sha256};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes};
use sql_split::exec_range::UnitKind;
use sql_split::stream::{
    STREAM_HELD_ITEMS_MAX, STREAM_HELD_MAX, StreamItem, StreamPolicy, StreamSplitter, any_refuses,
};
use sql_split::{AmbiguityReason, Confidence, ParamDiag, ParamSettings, SplitMode, SplitStart};
use tokio::sync::{broadcast, mpsc, watch};

use crate::classify_pool::{ClassifyPool, modes_of, run_ctx_in};
use crate::gate::{
    ConfirmAnswer, ConfirmPort, ConfirmQuestion, ConnSafety, GateDecision, GateItem, GateReason,
    gate,
};
use crate::history::{HistoryMeta, HistoryRecord, Outcome};
use crate::log::{LogArg, LogMsg, SessionLog};
use crate::params::{
    ParamValueStore, Prepared, PreparedUnit, param_diag_message_id, prepare_units,
};
use crate::service::{CatalogRun, ExecService, RunRequest, apply_answer};
use crate::suppress::SuppressSet;
use crate::units::{ExecUnit, expect_for, uncertain_message_id};
use sql_safety::Expect;

/// The size of the chunks the splitter is fed (64 KiB, cut back to a character boundary).
pub const SCRIPT_CHUNK_BYTES: usize = 64 << 10;

/// How long the run waits for the session's state at its end (a unit a cancel did not reach
/// may still hold the session).
const SETTLE_WAIT: Duration = Duration::from_secs(5);

/// How long a cancelled unit may take to end before the run gives up waiting for it.
const CANCEL_GRACE: Duration = Duration::from_secs(30);

/// A script read from disk in UTF-8 chunks, each cut at a character boundary: one pass over
/// the file. The runner reads a pass to its end, then asks for the next one.
///
/// ## m4b-t18-run-as-script (named exception): `Sync` too, so a run's future is `Send` and the
/// app can run it on the core runtime (the run holds its request across every await).
pub trait ScriptChunks: Send + Sync {
    /// The next chunk, `None` at the end. Blocking: the runner calls it on the blocking pool.
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>>;

    /// The second pass over the same file (R-M4b-47, R-T10-3), called only after this pass
    /// answered `None`. It must answer `Err(ChunkError::Changed)` unless the file is the one
    /// this pass read to its end (its opening stamp, its decoding), and the new pass must stop
    /// with `ChunkError::Changed` at the first read whose bytes differ from this pass's. A
    /// source that cannot read the file again answers an error (the run then sends nothing).
    fn reopen_verified(
        self: Box<Self>,
    ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>>;
}

/// Why a chunk could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// The file changed on disk since the run started.
    Changed,
    /// The bytes did not decode in the file's encoding.
    Decode,
    Io(io::ErrorKind),
}

/// Where in the file: the statement's number (from 1, statements only) and its first line
/// (from 1; a line ends at `\n`, `\r\n` or a lone `\r`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptPosition {
    pub unit: u64,
    pub line: u64,
}

/// Why a script run sent nothing (pass 1, or the check before it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefuseReason {
    /// An uncertain statement boundary (M3's whole split refuses it; R-M3-24).
    Uncertain(AmbiguityReason),
    /// A statement whose boundaries are not `Exact` (never without an uncertain boundary).
    NotExact,
    /// A statement longer than 64 MiB.
    TooLong,
    /// No statement boundary within 64 MiB of text, or within 65,536 items (R-T8-3).
    NoBoundary,
    /// A client command Hacivat does not run as the client would (`\c`, `\i`, `SOURCE`…).
    ClientCommand,
    /// A statement with a parameter (`None`: it needs a value; no prompt in a streamed run), or
    /// one whose parameters can never be bound.
    Params(Option<ParamDiag>),
    /// `COPY … FROM STDIN` / `TO STDOUT`: consoles do not stream data.
    CopyUnsupported,
    /// The gate denies the unit whatever the user answers (R-T12-2; its reason).
    Denied(GateReason),
    /// The file could not be read, or changed (also between the passes).
    Read(ChunkError),
    /// The session reads text in other lexing modes than the tab, or its modes are unknown and
    /// the file reads differently under some of them.
    ModesDiffer,
    /// Stop was pressed before anything ran.
    Cancelled,
    // ## m4b-t18-run-as-script (named exception; the T12 re-review's n1 and n3, R-T14-2)
    /// More than 65,536 statements on one line with no line break (the item cap, R-T8-3): its
    /// own words, never "64 MiB" (n1).
    NoBoundaryItems,
    /// The session's lexing modes are unknown and the file reads differently under some of
    /// them (n3; `ModesDiffer` is the tab's known modes against the session's).
    ModesUnknown,
    /// The app's database check before pass 2 ([`ScriptRunner::run_checked`]) refused: the
    /// session is not on the tab's database, or that could not be checked (R-M4b-27).
    DatabaseCheck,
    // ## m4b-t18-run-as-script
}

impl RefuseReason {
    /// The Fluent ID that says why (`hacivat-core.ftl`, block `m4b-t12-script-run`); `$line`
    /// names the line where the refusal has one. An uncertain boundary's own reason is
    /// `units::uncertain_message_id`'s, a parameter's diagnostic `param_diag_message_id`'s, a
    /// denial's the gate's (the log writes each on its own line).
    pub fn fluent_id(&self) -> &'static str {
        match self {
            RefuseReason::Uncertain(_) | RefuseReason::NotExact => "exec-script-uncertain",
            RefuseReason::TooLong => "exec-script-too-long",
            RefuseReason::NoBoundary => "exec-script-no-boundary",
            RefuseReason::ClientCommand => "exec-script-client-command",
            RefuseReason::Params(_) => "exec-script-params",
            RefuseReason::CopyUnsupported => "exec-script-copy",
            RefuseReason::Denied(_) => "exec-script-denied-before",
            RefuseReason::Read(ChunkError::Changed) => "exec-script-file-changed-before",
            RefuseReason::Read(ChunkError::Decode) => "exec-script-decode-failed",
            RefuseReason::Read(ChunkError::Io(_)) => "exec-script-read-failed",
            RefuseReason::ModesDiffer => "exec-script-modes-differ",
            RefuseReason::Cancelled => "exec-script-cancelled-before",
            // ## m4b-t18-run-as-script (named exception)
            RefuseReason::NoBoundaryItems => "exec-script-no-boundary-items",
            RefuseReason::ModesUnknown => "exec-script-modes-unknown",
            RefuseReason::DatabaseCheck => "exec-script-database-check",
        }
    }

    /// The second line's Fluent ID and arguments, where the reason has its own words.
    fn detail(&self) -> Option<(&'static str, Vec<(&'static str, LogArg)>)> {
        match self {
            RefuseReason::Uncertain(r) => Some((uncertain_message_id(*r), Vec::new())),
            RefuseReason::Params(Some(d)) => Some((param_diag_message_id(*d), Vec::new())),
            RefuseReason::Denied(g) => Some(gate_reason_line(g)),
            _ => None,
        }
    }
}

/// A run that sent nothing: why, and where when the file says where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRefusal {
    pub reason: RefuseReason,
    pub at: Option<ScriptPosition>,
}

/// Why a script run stopped before the end (pass 2: the units before it ran).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Cancelled,
    /// The file changed on disk mid-run (a notice; carries-w1 T12).
    FileChanged,
    /// The file could not be read or decoded mid-run (`Changed` is `FileChanged`).
    Read(ChunkError),
    /// The gate denied the unit (its reason), where pass 1 could not see it.
    Denied(GateReason),
    /// The user cancelled the unit's confirmation.
    ConfirmCancelled,
    /// The session now lexes otherwise than the file was split, or its context changed while a
    /// confirmation was open.
    ContextChanged,
    /// The context of a production or read-only session could not be read.
    ContextUnconfirmed,
    /// The profile's environment tag or the session's read-only flag changed between the
    /// gate and the send.
    ConnectionChanged,
    /// The connection was lost or reset since the run began: the new one lost what the file
    /// set (R-M4b-27).
    Reconnected,
    /// The session did not run the unit (it refused it, or never started the run).
    NotSent,
    /// Pass 2 met what pass 1 did not (never on an unchanged file; fail closed).
    Split(RefuseReason),
    // ## m4b-t18-run-as-script (named exception; the T12 re-review's n5)
    /// The run lost track of the connection's events (its event stream lagged or closed): it
    /// cannot tell whether the connection was reset, so it stops (fail closed).
    LostTrack,
}

impl StopReason {
    /// The Fluent ID that says why (`hacivat-core.ftl`, block `m4b-t12-script-run`); a denial's
    /// own reason is the gate's.
    pub fn fluent_id(&self) -> &'static str {
        match self {
            StopReason::Cancelled => "exec-script-cancelled",
            StopReason::FileChanged | StopReason::Read(ChunkError::Changed) => {
                "exec-script-file-changed"
            }
            StopReason::Read(ChunkError::Decode) => "exec-script-decode-failed-mid",
            StopReason::Read(ChunkError::Io(_)) => "exec-script-read-failed-mid",
            StopReason::Denied(_) => "exec-script-denied",
            StopReason::ConfirmCancelled => "exec-script-confirm-cancelled",
            StopReason::ContextChanged => "exec-script-context-changed",
            StopReason::ContextUnconfirmed => "exec-script-context-unconfirmed",
            StopReason::ConnectionChanged => "exec-script-connection-changed",
            StopReason::Reconnected => "exec-script-reconnected",
            StopReason::NotSent => "exec-script-not-sent",
            StopReason::Split(_) => "exec-script-split-changed",
            // ## m4b-t18-run-as-script (named exception)
            StopReason::LostTrack => "exec-script-lost-track",
        }
    }
}

/// A gate reason as a log line's ID and arguments.
fn gate_reason_line(g: &GateReason) -> (&'static str, Vec<(&'static str, LogArg)>) {
    (
        g.fluent_id,
        g.args
            .iter()
            .map(|(k, v)| (*k, LogArg::Str(v.clone())))
            .collect(),
    )
}

/// A run that stopped: why, and at which unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptStop {
    pub reason: StopReason,
    pub at: Option<ScriptPosition>,
}

/// What a script run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptRunSummary {
    /// Statements in the file (pass 1's count).
    pub units_total: u64,
    /// Units the session ran.
    pub units_run: u64,
    /// Units that failed.
    pub units_failed: u64,
    /// Units the user skipped in a confirmation.
    pub units_skipped: u64,
    /// Client commands the splitter applied (`DELIMITER`) or that end a statement, not sent.
    pub client_skipped: u64,
    /// The first unit that failed.
    pub failed_at: Option<ScriptPosition>,
    /// The `GateItem::fingerprint`s the user chose "don't ask again" for (the app keeps them).
    pub suppress: Vec<u64>,
    /// The session was in a transaction when the run ended (the M3 transaction UI shows it).
    pub tx_open: bool,
    /// The last unit was cancelled and did not end within the grace: whether it ran is
    /// unknown (it is not counted).
    pub last_unit_unknown: bool,
    /// The most bytes the runner held at once (the splitters' text, the line count's text and
    /// the chunk being cut).
    pub peak_held: usize,
    // ## m4b-t18-run-as-script fix round 1 (named exception; review m4, R-T18-2)
    /// Stop reached the last unit after it was sent: it was cancelled in flight (or ended just
    /// before the cancel reached it), so the stop is "during" that statement.
    pub cancelled_in_flight: bool,
}

/// How a script run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptRunOutcome {
    /// Nothing was sent.
    Refused(ScriptRefusal),
    Finished(ScriptRunSummary),
    Stopped(ScriptStop, ScriptRunSummary),
    /// A unit failed and the run stopped there (`ScriptRunSummary::failed_at`).
    Failed(ScriptRunSummary),
}

/// Which pass a run is in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ScriptPhase {
    /// Pass 1: reading the whole file, nothing run.
    #[default]
    Checking,
    /// Pass 2: running.
    Running,
    Done,
}

/// A run's progress (the strip's figures).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptProgress {
    pub phase: ScriptPhase,
    /// Bytes of text read in this pass.
    pub bytes: u64,
    /// Statements read in this pass.
    pub units_seen: u64,
    /// Statements in the file (once pass 1 ended).
    pub units_total: Option<u64>,
    pub units_run: u64,
    pub units_failed: u64,
    /// The line of the last statement read.
    pub line: u64,
}

/// What the gate knows of the tab's connection beyond its session: the live profile's
/// environment tag (the app keeps it current: the runner reads it before every gate and again
/// before every send), the profile's setting and function allowlist, the console's "don't ask
/// again" pairs, and the connection's name for the dialogs.
#[derive(Debug, Clone)]
pub struct ScriptGate {
    pub env: watch::Receiver<Option<EnvTag>>,
    pub confirm_unsafe_where: bool,
    pub allowlist: Arc<[Arc<str>]>,
    pub suppressed: SuppressSet,
    pub connection: Arc<str>,
}

/// Where the run's one summary row goes (R-M4b-24).
#[derive(Debug, Clone)]
pub struct ScriptHistory {
    /// The history store's sink (`HistoryStore::sink`).
    pub sink: mpsc::Sender<Arc<HistoryRecord>>,
    pub meta: HistoryMeta,
    /// The file's name, as the row's text.
    pub file_name: Arc<str>,
    pub database: Option<String>,
}

/// One script run: the tab's session and console, the file, how it is split, the gate's facts,
/// where confirmations go, the console's log, Stop, the history and the catalog refresh.
/// Every unit stops the run at its first failure (there is no error policy to choose).
pub struct ScriptRunRequest {
    pub session: SessionHandle,
    pub source: Box<dyn ScriptChunks>,
    pub console: u64,
    pub profile: ProfileId,
    pub engine: DbmsKind,
    /// The tab's context: the dialect, its version and the lexing modes it reads.
    pub dialect: DialectCtx,
    /// Whether the tab read `dialect.modes` from the session (`SqlContext::Known`): then a
    /// session that now reads other modes refuses the run (M3's rule).
    pub modes_known: bool,
    pub flavor: ClientFlavor,
    pub gate: ScriptGate,
    pub confirm: Arc<dyn ConfirmPort>,
    pub log: Arc<SessionLog>,
    /// Stop: `true` stops the run between units and cancels the unit in flight.
    pub cancel: watch::Receiver<bool>,
    pub history: Option<ScriptHistory>,
    /// The profile's catalog refresh inbox (the explorer follows a script's DDL as a Run's).
    pub refresh: Option<hacivat_introspect::refresh_coord::RefreshInbox>,
}

impl fmt::Debug for ScriptRunRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptRunRequest")
            .field("console", &self.console)
            .field("engine", &self.engine)
            .field("dialect", &self.dialect.id)
            .field("modes_known", &self.modes_known)
            .field("history", &self.history.is_some())
            .finish_non_exhaustive()
    }
}

/// Runs scripts from disk unit by unit, with the app's run service and classifier pool.
#[derive(Debug, Clone)]
pub struct ScriptRunner {
    exec: Arc<ExecService>,
    classifier: Arc<ClassifyPool>,
}

impl ScriptRunner {
    pub fn new(exec: Arc<ExecService>, classifier: Arc<ClassifyPool>) -> Self {
        ScriptRunner { exec, classifier }
    }

    /// Streams `req.source` to `req.session` (the module docs), publishing `progress`.
    pub async fn run(
        &self,
        req: ScriptRunRequest,
        progress: watch::Sender<ScriptProgress>,
    ) -> ScriptRunOutcome {
        self.run_checked(req, progress, None).await
    }

    // ## m4b-t18-run-as-script (named exception; R-M4b-27, R-T14-2)
    /// [`ScriptRunner::run`] with the app's database check: after a clean pass 1 and before
    /// anything is sent, `check` asks whether the session is on the tab's database and answers
    /// the session's context generation it read (`None`: refused, nothing is sent,
    /// `RefuseReason::DatabaseCheck`). The first unit is sent only under that generation (else
    /// `StopReason::ContextChanged`, nothing sent); a loss or reset after the check stops the
    /// run as any other. Stop while the check waits refuses the run.
    pub async fn run_checked(
        &self,
        req: ScriptRunRequest,
        progress: watch::Sender<ScriptProgress>,
        check: Option<DatabaseCheck>,
    ) -> ScriptRunOutcome {
        let mut run = Run::new(self, req, progress);
        run.check = check;
        let out = run.go().await;
        run.log_outcome(&out);
        run.publish(|p| p.phase = ScriptPhase::Done);
        out
    }
}

// ## m4b-t18-run-as-script (named exception)
/// The app's check that the tab's session is on the tab's database, run between the passes
/// ([`ScriptRunner::run_checked`]): `Some(generation)` lets the run go on under that context
/// generation, `None` refuses it.
pub type DatabaseCheck = Box<dyn FnOnce() -> BoxFuture<'static, Option<u64>> + Send + Sync>;

/// Why a run must stop for its connection (`Run::connection_lost`): the session's `state`
/// now, and the events since pass 2 began (`None` before it). A lagged or closed stream is a
/// lost track (fail closed), never "reset" (the T12 re-review's n5).
fn lost_reason(
    state: &SessionState,
    events: Option<&mut broadcast::Receiver<SessionEvent>>,
) -> Option<StopReason> {
    if matches!(
        state,
        SessionState::Disconnected | SessionState::Connecting | SessionState::Broken(_)
    ) {
        return Some(StopReason::Reconnected);
    }
    let events = events?;
    loop {
        match events.try_recv() {
            Ok(SessionEvent::Lost(_) | SessionEvent::Reconnected) => {
                return Some(StopReason::Reconnected);
            }
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Empty) => return None,
            Err(
                broadcast::error::TryRecvError::Lagged(_) | broadcast::error::TryRecvError::Closed,
            ) => return Some(StopReason::LostTrack),
        }
    }
}
// ## m4b-t18-run-as-script

/// [`ScriptRunner::run`] as a free function.
pub async fn run_file(
    runner: &ScriptRunner,
    req: ScriptRunRequest,
    progress: watch::Sender<ScriptProgress>,
) -> ScriptRunOutcome {
    runner.run(req, progress).await
}

// =================================================================================================
// Reading: identical chunks, the pass record, line numbers, the mode variants.

/// The source's text cut into [`SCRIPT_CHUNK_BYTES`] pieces at a character boundary (the
/// module docs): what the splitter is fed depends on the text alone.
struct Rechunk {
    inner: Box<dyn ScriptChunks>,
    buf: String,
    pos: usize,
    ended: bool,
    failed: bool,
}

impl Rechunk {
    fn new(inner: Box<dyn ScriptChunks>) -> Rechunk {
        Rechunk {
            inner,
            buf: String::new(),
            pos: 0,
            ended: false,
            failed: false,
        }
    }

    /// Bytes buffered, not handed out yet.
    fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn next(&mut self) -> Option<Result<String, ChunkError>> {
        if self.failed {
            return None;
        }
        loop {
            let avail = self.buffered();
            if avail >= SCRIPT_CHUNK_BYTES || (self.ended && avail > 0) {
                let mut end = if avail >= SCRIPT_CHUNK_BYTES {
                    self.pos + SCRIPT_CHUNK_BYTES
                } else {
                    self.buf.len()
                };
                while !self.buf.is_char_boundary(end) {
                    end -= 1;
                }
                let out = self.buf[self.pos..end].to_string();
                self.pos = end;
                return Some(Ok(out));
            }
            if self.ended {
                return None;
            }
            match self.inner.next_chunk() {
                None => self.ended = true,
                Some(Err(e)) => {
                    self.failed = true;
                    return Some(Err(e));
                }
                Some(Ok(s)) => {
                    // Less than a chunk is left before `pos`'s text is dropped.
                    self.buf.drain(..self.pos);
                    self.pos = 0;
                    self.buf.push_str(&s);
                }
            }
        }
    }
}

/// Pass 1's record of the chunks it fed (length and SHA-256 each); pass 2 is checked against it.
#[derive(Default)]
struct PassRecord {
    chunks: Vec<(usize, [u8; 32])>,
}

fn digest(s: &str) -> (usize, [u8; 32]) {
    (s.len(), Sha256::digest(s.as_bytes()).into())
}

/// Line numbers of stream offsets that only grow (the module docs): the text from the last
/// offset asked about to the end of what was fed, and the line at that offset. A line ends at
/// `\n`, `\r\n` or a lone `\r` (a `\r` that is the last byte fed counts once the next byte is
/// known; until then the line before it is answered).
struct Lines {
    text: Vec<u8>,
    pos: usize,
    /// The stream offset of `text[pos]`.
    base: u64,
    /// The line at `base`, from 1.
    line: u64,
}

impl Lines {
    fn new() -> Lines {
        Lines {
            text: Vec::new(),
            pos: 0,
            base: 0,
            line: 1,
        }
    }

    fn push(&mut self, s: &str) {
        if self.pos > 0 && self.pos * 2 >= self.text.len() {
            self.text.drain(..self.pos);
            self.pos = 0;
        }
        self.text.extend_from_slice(s.as_bytes());
    }

    /// Whether byte `i` of `text` ends a line.
    fn ends_line(&self, i: usize) -> bool {
        match self.text[i] {
            b'\n' => true,
            b'\r' => self.text.get(i + 1).is_some_and(|&b| b != b'\n'),
            _ => false,
        }
    }

    /// The line at stream offset `off` (an offset before the last one asked about answers that
    /// one's line); the text before `off` is dropped.
    fn at(&mut self, off: u64) -> u64 {
        if off > self.base {
            let n = usize::try_from(off - self.base).unwrap_or(usize::MAX);
            let end = self.pos.saturating_add(n).min(self.text.len());
            let ends = (self.pos..end).filter(|&i| self.ends_line(i)).count();
            self.line += ends as u64;
            self.base += (end - self.pos) as u64;
            self.pos = end;
        }
        self.line
    }

    fn held(&self) -> usize {
        self.text.len() - self.pos
    }
}

/// Every lexing-mode combination that changes how `base`'s dialect splits text (M3's
/// `exec_ui::context::variants`), `base`'s own modes first.
fn mode_variants(base: &DialectCtx) -> Vec<DialectCtx> {
    let with = |modes: LexModes| DialectCtx { modes, ..*base };
    let mut out = vec![*base];
    match base.id {
        DialectId::Postgres | DialectId::PgWire(_) => {
            for scs in [true, false] {
                out.push(with(LexModes {
                    standard_conforming_strings: scs,
                    ..base.modes
                }));
            }
        }
        DialectId::MySql | DialectId::MariaDb => {
            let oracle: &[bool] = if base.id == DialectId::MariaDb {
                &[false, true]
            } else {
                &[false]
            };
            for ansi in [false, true] {
                for nbe in [false, true] {
                    for &ora in oracle {
                        out.push(with(LexModes {
                            ansi_quotes: ansi,
                            no_backslash_escapes: nbe,
                            mariadb_oracle_mode: ora,
                            ..base.modes
                        }));
                    }
                }
            }
        }
        _ => {}
    }
    let mut seen = Vec::new();
    out.retain(|d| {
        let new = !seen.contains(&d.modes);
        seen.push(d.modes);
        new
    });
    out
}

/// One item of the stream with its line and what it is for the run.
struct Located {
    item: StreamItem,
    line: u64,
    checked: Checked,
}

/// What a step of a pass read.
enum Step {
    /// The items a chunk settled (`end`: the stream's last ones); `refuses`:
    /// `stream::any_refuses` of the batch (R-M4b-47); `alike` (at the end): every mode reading
    /// gave the same items (always true when the modes are known).
    Items {
        items: Vec<Located>,
        end: bool,
        refuses: bool,
        alike: bool,
    },
    Failed(ChunkError),
    /// The splitter holds more than 64 MiB of text or 65,536 items with no boundary (off
    /// MySQL/MariaDB).
    Held {
        line: u64,
        // ## m4b-t18-run-as-script (named exception, n1): only the item cap was passed.
        items: bool,
    },
}

/// The held-text guard of the `Exact` path (T8 m-fr3-3): over either cap.
fn over_caps(split: &StreamSplitter) -> bool {
    split.held_len() > STREAM_HELD_MAX || split.held_items() > STREAM_HELD_ITEMS_MAX
}

/// Another reading of the file under other lexing modes (pass 1 with the modes unknown): its
/// splitter and the digest of everything it gave out and decided.
struct Variant {
    dialect: DialectCtx,
    split: Option<StreamSplitter>,
    digest: Sha256,
    broken: bool,
}

/// One pass over the file: the chunks, the splitter, the line count, the pass record and, in
/// pass 1 with the modes unknown, the other readings.
struct Reader {
    chunks: Rechunk,
    dialect: DialectCtx,
    flavor: ClientFlavor,
    split: Option<StreamSplitter>,
    lines: Lines,
    fed: u64,
    /// The held-text guard (off MySQL/MariaDB, T8 m-fr3-3).
    guard: bool,
    record: PassRecord,
    /// Pass 2: the next chunk of `record` to check.
    check: Option<usize>,
    /// The digest of this reading, when there are others to compare it with.
    digest: Option<Sha256>,
    variants: Vec<Variant>,
    peak: usize,
}

impl Reader {
    /// A pass over `source` in `dialect`, and in each of `others` beside it.
    fn new(
        source: Box<dyn ScriptChunks>,
        dialect: DialectCtx,
        flavor: ClientFlavor,
        others: &[DialectCtx],
    ) -> Reader {
        let splitter = |d: DialectCtx| StreamSplitter::with_policy(d, flavor, StreamPolicy::Eager);
        Reader {
            chunks: Rechunk::new(source),
            dialect,
            flavor,
            split: Some(splitter(dialect)),
            lines: Lines::new(),
            fed: 0,
            guard: !matches!(dialect.id, DialectId::MySql | DialectId::MariaDb),
            record: PassRecord::default(),
            check: None,
            digest: (!others.is_empty()).then(Sha256::new),
            variants: others
                .iter()
                .map(|&d| Variant {
                    dialect: d,
                    split: Some(splitter(d)),
                    digest: Sha256::new(),
                    broken: false,
                })
                .collect(),
            peak: 0,
        }
    }

    /// Pass 2 over `source` in this pass's dialect, checked against this pass's record.
    fn second(self, source: Box<dyn ScriptChunks>) -> Reader {
        Reader {
            record: self.record,
            check: Some(0),
            peak: self.peak,
            ..Reader::new(source, self.dialect, self.flavor, &[])
        }
    }

    /// `items` with their lines and decisions (and this reading's digest).
    fn locate(&mut self, items: Vec<StreamItem>) -> Vec<Located> {
        let (dialect, flavor) = (self.dialect, self.flavor);
        items
            .into_iter()
            .map(|item| {
                let line = self.lines.at(offset_of(&item));
                let checked = check_item(&item, &dialect, flavor);
                if let Some(d) = self.digest.as_mut() {
                    note(d, &item, &checked);
                }
                Located {
                    item,
                    line,
                    checked,
                }
            })
            .collect()
    }

    /// Feeds `chunk` to the other readings (`None`: their end).
    fn feed_variants(&mut self, chunk: Option<&str>) {
        let (guard, flavor) = (self.guard, self.flavor);
        for v in &mut self.variants {
            if v.broken {
                continue;
            }
            let items = match chunk {
                Some(c) => match v.split.as_mut() {
                    Some(s) => {
                        let items = s.feed(c);
                        if guard && over_caps(s) {
                            v.broken = true;
                        }
                        items
                    }
                    None => Vec::new(),
                },
                None => v
                    .split
                    .take()
                    .map(StreamSplitter::finish)
                    .unwrap_or_default(),
            };
            for item in &items {
                note(&mut v.digest, item, &check_item(item, &v.dialect, flavor));
            }
        }
    }

    /// Whether every other reading gave exactly this one's items and decisions.
    fn alike(&mut self) -> bool {
        let Some(mine) = self.digest.take() else {
            return true;
        };
        let mine = mine.finalize();
        self.variants
            .drain(..)
            .all(|v| !v.broken && v.digest.finalize() == mine)
    }

    fn held_now(&self) -> usize {
        let split = self.split.as_ref().map_or(0, StreamSplitter::held_len);
        let others: usize = self
            .variants
            .iter()
            .filter_map(|v| v.split.as_ref())
            .map(StreamSplitter::held_len)
            .sum();
        split + others + self.lines.held() + self.chunks.buffered()
    }

    /// Reads one chunk and feeds it (blocking: the source reads, the splitters split).
    fn step(&mut self) -> Step {
        match self.chunks.next() {
            Some(Err(e)) => Step::Failed(e),
            Some(Ok(chunk)) => {
                let d = digest(&chunk);
                match &mut self.check {
                    None => self.record.chunks.push(d),
                    Some(next) => {
                        // Never feed a chunk pass 1 did not read (R-T8-1, R-T10-3).
                        if self.record.chunks.get(*next) != Some(&d) {
                            return Step::Failed(ChunkError::Changed);
                        }
                        *next += 1;
                    }
                }
                self.lines.push(&chunk);
                self.fed += chunk.len() as u64;
                let Some(split) = self.split.as_mut() else {
                    return Step::Items {
                        items: Vec::new(),
                        end: true,
                        refuses: false,
                        alike: true,
                    };
                };
                let items = split.feed(&chunk);
                let held = split.held_len();
                let over = self.guard && over_caps(split);
                // ## m4b-t18-run-as-script (named exception, n1)
                let items_only = held <= STREAM_HELD_MAX;
                let refuses = any_refuses(&items);
                let located = self.locate(items);
                self.feed_variants(Some(&chunk));
                self.peak = self.peak.max(self.held_now() + chunk.len());
                if over {
                    let line = self.lines.at(self.fed - held as u64);
                    return Step::Held {
                        line,
                        items: items_only,
                    };
                }
                // Later items start in the held text: what lies before it is never asked for.
                self.lines.at(self.fed - held as u64);
                Step::Items {
                    items: located,
                    end: false,
                    refuses,
                    alike: true,
                }
            }
            None => {
                if let Some(next) = self.check
                    && next != self.record.chunks.len()
                {
                    return Step::Failed(ChunkError::Changed);
                }
                let items = self
                    .split
                    .take()
                    .map(StreamSplitter::finish)
                    .unwrap_or_default();
                let refuses = any_refuses(&items);
                let located = self.locate(items);
                self.feed_variants(None);
                let alike = self.alike();
                Step::Items {
                    items: located,
                    end: true,
                    refuses,
                    alike,
                }
            }
        }
    }
}

/// Adds `item` and its decision to a reading's digest.
fn note(h: &mut Sha256, item: &StreamItem, checked: &Checked) {
    let tag = |h: &mut Sha256, t: u8, off: u64| {
        h.update([t]);
        h.update(off.to_le_bytes());
    };
    match item {
        StreamItem::Statement {
            text,
            offset,
            confidence,
        } => {
            tag(h, 1, *offset);
            h.update([u8::from(*confidence == Confidence::Exact)]);
            h.update((text.len() as u64).to_le_bytes());
            h.update(text.as_bytes());
        }
        StreamItem::Client {
            text,
            offset,
            supported,
        } => {
            tag(h, 2, *offset);
            h.update([u8::from(*supported)]);
            h.update((text.len() as u64).to_le_bytes());
            h.update(text.as_bytes());
        }
        StreamItem::Ambiguous { offset, reason } => {
            tag(h, 3, *offset);
            h.update(format!("{reason:?}").as_bytes());
        }
        StreamItem::TooLong { offset } => tag(h, 4, *offset),
        StreamItem::NoBoundary { offset } => tag(h, 5, *offset),
    }
    match checked {
        Checked::Unit(p) => {
            h.update([10]);
            h.update((p.final_text.len() as u64).to_le_bytes());
            h.update(p.final_text.as_bytes());
            h.update((p.binds.len() as u64).to_le_bytes());
        }
        Checked::Client => h.update([11]),
        Checked::Refused(r) => {
            h.update([12]);
            h.update(format!("{r:?}").as_bytes());
        }
    }
}

fn offset_of(item: &StreamItem) -> u64 {
    match item {
        StreamItem::Statement { offset, .. }
        | StreamItem::Client { offset, .. }
        | StreamItem::Ambiguous { offset, .. }
        | StreamItem::TooLong { offset }
        | StreamItem::NoBoundary { offset } => *offset,
    }
}

/// Runs `step` on the blocking pool with `reader`, giving it back (a panic is a read error).
async fn blocking<T: Send + 'static>(
    reader: Reader,
    step: impl FnOnce(&mut Reader) -> T + Send + 'static,
) -> Option<(Reader, T)> {
    tokio::task::spawn_blocking(move || {
        let mut reader = reader;
        let out = step(&mut reader);
        (reader, out)
    })
    .await
    .ok()
}

// =================================================================================================
// Checking an item (both passes alike).

/// A store that remembers no parameter value: a streamed run asks for none.
struct NoValues;

impl ParamValueStore for NoValues {
    fn get(&self, _: &ProfileId, _: &str) -> Option<hacivat_values::BindValue> {
        None
    }

    fn put(&self, _: &ProfileId, _: &str, _: hacivat_values::BindValue) {}
}

/// What an item of the stream is for the run.
enum Checked {
    Unit(Box<PreparedUnit>),
    /// A supported client command: not sent.
    Client,
    Refused(RefuseReason),
}

/// The item's decision (the module docs), the same in both passes.
fn check_item(item: &StreamItem, dialect: &DialectCtx, flavor: ClientFlavor) -> Checked {
    match item {
        StreamItem::Ambiguous { reason, .. } => Checked::Refused(RefuseReason::Uncertain(*reason)),
        StreamItem::TooLong { .. } => Checked::Refused(RefuseReason::TooLong),
        StreamItem::NoBoundary { .. } => Checked::Refused(RefuseReason::NoBoundary),
        StreamItem::Client { supported, .. } => {
            if *supported {
                Checked::Client
            } else {
                Checked::Refused(RefuseReason::ClientCommand)
            }
        }
        StreamItem::Statement {
            confidence: Confidence::Heuristic,
            ..
        } => Checked::Refused(RefuseReason::NotExact),
        StreamItem::Statement {
            text,
            offset,
            confidence: Confidence::Exact,
        } => {
            if copy_stream(text, dialect, flavor) {
                return Checked::Refused(RefuseReason::CopyUnsupported);
            }
            let start = usize::try_from(*offset).unwrap_or(usize::MAX);
            let unit = ExecUnit {
                text: Arc::from(text.as_str()),
                doc_range: start..start.saturating_add(text.len()),
                kind: UnitKind::Statement,
                // A script run keeps no cursor: its results are not shown.
                suspendable: false,
                title: None,
            };
            match prepare_units(
                vec![unit],
                dialect,
                &ParamSettings::default(),
                &NoValues,
                &ProfileId::default(),
            ) {
                Prepared::Ready(mut units) if units.len() == 1 => {
                    Checked::Unit(Box::new(units.remove(0)))
                }
                Prepared::Ready(_) | Prepared::NeedsValues(_) => {
                    Checked::Refused(RefuseReason::Params(None))
                }
                Prepared::Refused { diag, .. } => {
                    Checked::Refused(RefuseReason::Params(Some(diag)))
                }
            }
        }
    }
}

/// Whether `text` (one statement) is a `COPY` that streams rows to or from the client, as the
/// splitter's own head reads it (M3's `UnitKind::CopyStream`).
fn copy_stream(text: &str, dialect: &DialectCtx, flavor: ClientFlavor) -> bool {
    let head = text
        .get(..4)
        .is_some_and(|w| w.eq_ignore_ascii_case("COPY"));
    let word_ends = text
        .as_bytes()
        .get(4)
        .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_' || *b >= 0x80));
    if !(head && word_ends) {
        return false;
    }
    let split = sql_split::split_from(
        text,
        dialect,
        flavor,
        SplitMode::Execution,
        &SplitStart::default(),
    );
    // A COPY the splitter does not see as one statement is not sent either (fail closed).
    split.statements().count() != 1
        || split
            .statements()
            .any(|s| s.head.copy_stream || s.copy_data.is_some())
}

// =================================================================================================
// The run.

/// What a unit did.
enum UnitDone {
    Ran,
    Skipped,
    Failed,
    Stop(StopReason),
}

/// The state of one run.
struct Run<'a> {
    runner: &'a ScriptRunner,
    req: ScriptRunRequest,
    progress: watch::Sender<ScriptProgress>,
    summary: ScriptRunSummary,
    suppressed: SuppressSet,
    /// The context the file is split, prepared and checked in.
    dialect: DialectCtx,
    /// The session read its modes at the start: a later reading that differs stops the run.
    modes_fixed: bool,
    /// Execute all answered (off production): later calm confirmations run.
    execute_all: bool,
    skip_all: bool,
    /// The session's events since pass 2 began (a loss or a reset stops the run).
    events: Option<broadcast::Receiver<SessionEvent>>,
    /// Pass 2 began (the end writes the run's summary line).
    running: bool,
    started: Instant,
    started_at: SystemTime,
    // ## m4b-t18-run-as-script (named exception; R-M4b-27, R-T14-2)
    /// The app's database check, run after a clean pass 1 ([`ScriptRunner::run_checked`]).
    check: Option<DatabaseCheck>,
    /// The context generation that check read: the first unit's must be it.
    checked_generation: Option<u64>,
    // ## demofix-1 (PO demo F6)
    /// The statements that ran with nothing to say, not yet written to the console's log.
    quiet: QuietLog,
}

// ## demofix-1 (PO demo F6): the Output of a streamed run is bounded.
/// The statements a streamed run keeps one by one before it writes them: the last ones before
/// the end, a failure or a server notice. Earlier statements that ran with nothing to say (a
/// row count, "Completed") are counted in one `exec-script-quiet` line instead of a line each.
pub const SCRIPT_TAIL_STATEMENTS: usize = 50;
/// A counted line is written at least every this many statements, so a long clean run still
/// shows its progress in the log (the strip shows every statement).
pub const SCRIPT_QUIET_EVERY: u64 = 10_000;

/// A line of a unit's own log, as the console's log takes it.
type UnitLine = (
    LogMsg,
    Option<Arc<str>>,
    Option<std::ops::Range<usize>>,
    Option<UnitId>,
);

/// The quiet statements of a run: the counted ones, then the last ones kept one by one. Its
/// memory is bounded by [`SCRIPT_TAIL_STATEMENTS`] units' lines, whatever the file's length.
#[derive(Debug, Default)]
struct QuietLog {
    /// The counted statements: the first and last numbers, how many, and the rows they changed.
    first: u64,
    last: u64,
    n: u64,
    rows: u64,
    /// The last quiet statements' own lines, oldest first.
    tail: std::collections::VecDeque<(u64, Vec<UnitLine>)>,
}

/// ## demofix-1 fix round 1 (m4): a statement that took this long keeps its own line (with its
/// time) even in a quiet run.
pub const SCRIPT_SLOW_MS: u64 = 1_000;

/// The rows a line reports when it is one a quiet statement writes (`None`: it says more).
/// Quiet: a row count or "Completed" under [`SCRIPT_SLOW_MS`], and (fix round 1, m3) a
/// transaction-control line (`BEGIN`, `SET` and `SAVEPOINT` write "Completed"; `COMMIT` and
/// `ROLLBACK` write their own), so a per-row `BEGIN; INSERT; COMMIT;` script stays bounded.
fn quiet_rows(msg: &LogMsg) -> Option<u64> {
    match msg {
        LogMsg::RowsAffected { rows, ms } if *ms < SCRIPT_SLOW_MS => Some(*rows),
        LogMsg::Completed { ms } if *ms < SCRIPT_SLOW_MS => Some(0),
        LogMsg::TxCommitted | LogMsg::TxRolledBack => Some(0),
        _ => None,
    }
}

// ## demofix-1 fix round 1 (m3, m4)
#[cfg(test)]
mod quiet_tests {
    use super::{LogMsg, SCRIPT_SLOW_MS, quiet_rows};

    #[test]
    fn slow_statements_and_failures_keep_their_own_line() {
        assert_eq!(
            quiet_rows(&LogMsg::RowsAffected { rows: 3, ms: 12 }),
            Some(3)
        );
        assert_eq!(quiet_rows(&LogMsg::Completed { ms: 0 }), Some(0));
        assert_eq!(quiet_rows(&LogMsg::TxCommitted), Some(0));
        assert_eq!(quiet_rows(&LogMsg::TxRolledBack), Some(0));
        assert_eq!(
            quiet_rows(&LogMsg::RowsAffected {
                rows: 1,
                ms: SCRIPT_SLOW_MS
            }),
            None,
            "a statement of a second or more keeps its line with its time"
        );
        assert_eq!(quiet_rows(&LogMsg::Completed { ms: 40_000 }), None);
        assert_eq!(quiet_rows(&LogMsg::ImplicitCommit), None);
        assert_eq!(quiet_rows(&LogMsg::TxRolledBackByServer), None);
    }
}

impl<'a> Run<'a> {
    fn new(
        runner: &'a ScriptRunner,
        req: ScriptRunRequest,
        progress: watch::Sender<ScriptProgress>,
    ) -> Run<'a> {
        let suppressed = req.gate.suppressed.clone();
        let dialect = req.dialect;
        Run {
            runner,
            req,
            progress,
            summary: ScriptRunSummary::default(),
            suppressed,
            dialect,
            modes_fixed: false,
            execute_all: false,
            skip_all: false,
            events: None,
            running: false,
            started: Instant::now(),
            started_at: SystemTime::now(),
            // ## m4b-t18-run-as-script (named exception)
            check: None,
            checked_generation: None,
            quiet: QuietLog::default(),
        }
    }

    fn cancelled(&self) -> bool {
        *self.req.cancel.borrow()
    }

    /// The live environment tag.
    fn env(&self) -> Option<EnvTag> {
        *self.req.gate.env.borrow()
    }

    fn publish(&self, f: impl FnOnce(&mut ScriptProgress)) {
        self.progress.send_modify(f);
    }

    /// The outcome's lines in the console's log: why nothing ran, why the run stopped, and the
    /// run's summary.
    fn log_outcome(&mut self, out: &ScriptRunOutcome) {
        // ## demofix-1 (PO demo F6): the statements still waiting, before why the run ended.
        self.write_quiet();
        let log = &self.req.log;
        let at_args = |at: Option<ScriptPosition>| -> Vec<(&'static str, LogArg)> {
            at.map(|p| {
                vec![
                    ("line", LogArg::U64(p.line)),
                    ("n", LogArg::U64(p.unit.saturating_sub(1))),
                ]
            })
            .unwrap_or_default()
        };
        let line = |id: &'static str, args: Vec<(&'static str, LogArg)>| {
            log.append(
                LogMsg::RunUi {
                    id,
                    args,
                    warn: true,
                },
                None,
                None,
            );
        };
        match out {
            ScriptRunOutcome::Refused(r) => {
                line(r.reason.fluent_id(), at_args(r.at));
                if let Some((id, args)) = r.reason.detail() {
                    line(id, args);
                }
            }
            ScriptRunOutcome::Stopped(s, _) => {
                line(s.reason.fluent_id(), at_args(s.at));
                match &s.reason {
                    StopReason::Denied(g) => {
                        let (id, args) = gate_reason_line(g);
                        line(id, args);
                    }
                    StopReason::Split(r) => {
                        if let Some((id, args)) = r.detail() {
                            line(id, args);
                        }
                    }
                    _ => {}
                }
            }
            ScriptRunOutcome::Finished(_) | ScriptRunOutcome::Failed(_) => {}
        }
        if !self.running {
            return;
        }
        let s = match out {
            ScriptRunOutcome::Finished(s)
            | ScriptRunOutcome::Failed(s)
            | ScriptRunOutcome::Stopped(_, s) => s,
            ScriptRunOutcome::Refused(_) => return,
        };
        let n = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
        log.append(
            LogMsg::Summary(RunSummary {
                executed: n(s.units_run),
                total: n(s.units_total),
                failed: n(s.units_failed),
                skipped: n(s.units_total.saturating_sub(s.units_run + s.units_failed)),
                aborted: !matches!(out, ScriptRunOutcome::Finished(_)),
                duration_ms: u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            }),
            None,
            None,
        );
    }

    async fn go(&mut self) -> ScriptRunOutcome {
        let refuse = |reason, at| ScriptRunOutcome::Refused(ScriptRefusal { reason, at });
        // The session's modes: known, the file is split in them (and a tab that read others
        // refuses, M3's rule); unknown, it must read alike under every combination.
        let (sctx, _) = self.req.session.run_context().await;
        let (now, known) = modes_of(self.req.engine, &sctx);
        let others = if known {
            if self.req.modes_known && now.modes != self.req.dialect.modes {
                return refuse(RefuseReason::ModesDiffer, None);
            }
            self.dialect = DialectCtx {
                modes: now.modes,
                ..self.req.dialect
            };
            self.modes_fixed = true;
            Vec::new()
        } else {
            mode_variants(&self.dialect).split_off(1)
        };
        let placeholder: Box<dyn ScriptChunks> = Box::new(Spent);
        let source = std::mem::replace(&mut self.req.source, placeholder);

        // Pass 1: nothing runs.
        let mut check = PassGate::new(&sctx);
        let mut reader = Reader::new(source, self.dialect, self.req.flavor, &others);
        let mut units = 0u64;
        loop {
            if self.cancelled() {
                return refuse(RefuseReason::Cancelled, None);
            }
            let Some((r, step)) = blocking(reader, Reader::step).await else {
                return refuse(
                    RefuseReason::Read(ChunkError::Io(io::ErrorKind::Other)),
                    None,
                );
            };
            reader = r;
            let (items, end, refuses, alike) = match step {
                Step::Failed(e) => return refuse(RefuseReason::Read(e), None),
                Step::Held { line, items } => {
                    let at = ScriptPosition {
                        unit: units + 1,
                        line,
                    };
                    // ## m4b-t18-run-as-script (named exception, n1)
                    let reason = if items {
                        RefuseReason::NoBoundaryItems
                    } else {
                        RefuseReason::NoBoundary
                    };
                    return refuse(reason, Some(at));
                }
                Step::Items {
                    items,
                    end,
                    refuses,
                    alike,
                } => (items, end, refuses, alike),
            };
            // R-M4b-47: every batch is checked (`any_refuses`), not only `stopped()`; the item
            // that refuses names where.
            let mut batch: Vec<(PreparedUnit, ScriptPosition)> = Vec::new();
            let mut first_refusing = None;
            for l in items {
                let at = ScriptPosition {
                    unit: units + 1,
                    line: l.line,
                };
                if first_refusing.is_none() && l.item.refuses() {
                    first_refusing = Some(at);
                }
                match l.checked {
                    Checked::Unit(p) => {
                        units += 1;
                        batch.push((*p, at));
                    }
                    Checked::Client => {}
                    Checked::Refused(reason) => return refuse(reason, Some(at)),
                }
            }
            if refuses {
                // `check_item` refuses every item that `refuses()`; this stays the last word.
                return refuse(RefuseReason::NotExact, first_refusing);
            }
            let line = batch.last().map(|(_, at)| at.line);
            // R-T12-2: what the gate denies whatever the user answers refuses the file.
            match self.or_cancel(check.check(self, batch)).await {
                None => return refuse(RefuseReason::Cancelled, None),
                Some(Some((reason, at))) => return refuse(RefuseReason::Denied(reason), Some(at)),
                Some(None) => {}
            }
            self.publish(|p| {
                p.bytes = reader.fed;
                p.units_seen = units;
                p.line = line.unwrap_or(p.line);
            });
            if end {
                if !alike {
                    // ## m4b-t18-run-as-script (named exception, n3): the modes are unknown.
                    return refuse(RefuseReason::ModesUnknown, None);
                }
                break;
            }
        }
        self.summary.units_total = units;
        self.summary.peak_held = reader.peak;

        // Between the passes: the file must be the one pass 1 read.
        if self.cancelled() {
            return refuse(RefuseReason::Cancelled, None);
        }
        // A loss or a reset from here on stops the run.
        self.events = Some(self.req.session.events());
        // ## m4b-t18-run-as-script (named exception; R-M4b-27, R-T14-2): the app's check that
        // the session is on the tab's database, after the events are followed (a reset after
        // the check stops the run) and before the file is opened again; it answers the context
        // generation it read, and the first unit is sent only under it.
        if let Some(check) = self.check.take() {
            match self.or_cancel(check()).await {
                None => return refuse(RefuseReason::Cancelled, None),
                Some(None) => return refuse(RefuseReason::DatabaseCheck, None),
                Some(Some(generation)) => self.checked_generation = Some(generation),
            }
        }
        let inner = std::mem::replace(&mut reader.chunks.inner, Box::new(Spent));
        let second = match inner.reopen_verified().await {
            Ok(s) => s,
            Err(e) => return refuse(RefuseReason::Read(e), None),
        };
        let mut reader = reader.second(second);
        self.running = true;
        self.publish(|p| {
            *p = ScriptProgress {
                phase: ScriptPhase::Running,
                units_total: Some(units),
                ..ScriptProgress::default()
            };
        });

        // Pass 2: each unit runs once the chunk it came from was read and checked.
        let mut ordinal = 0u64;
        let outcome = 'run: loop {
            if self.cancelled() {
                break 'run self.stop(StopReason::Cancelled, None);
            }
            let Some((r, step)) = blocking(reader, Reader::step).await else {
                break 'run self.stop(StopReason::Read(ChunkError::Io(io::ErrorKind::Other)), None);
            };
            reader = r;
            self.summary.peak_held = self.summary.peak_held.max(reader.peak);
            let (items, end) = match step {
                Step::Failed(ChunkError::Changed) => {
                    break 'run self.stop(StopReason::FileChanged, None);
                }
                Step::Failed(e) => break 'run self.stop(StopReason::Read(e), None),
                Step::Held { line, items } => {
                    let at = ScriptPosition {
                        unit: ordinal + 1,
                        line,
                    };
                    // ## m4b-t18-run-as-script (named exception, n1)
                    let reason = if items {
                        RefuseReason::NoBoundaryItems
                    } else {
                        RefuseReason::NoBoundary
                    };
                    break 'run self.stop(StopReason::Split(reason), Some(at));
                }
                Step::Items { items, end, .. } => (items, end),
            };
            for l in items {
                let at = ScriptPosition {
                    unit: ordinal + 1,
                    line: l.line,
                };
                let prepared = match l.checked {
                    Checked::Client => {
                        self.summary.client_skipped += 1;
                        continue;
                    }
                    Checked::Refused(r) => break 'run self.stop(StopReason::Split(r), Some(at)),
                    Checked::Unit(p) => p,
                };
                ordinal += 1;
                self.publish(|p| {
                    p.units_seen = ordinal;
                    p.line = at.line;
                    p.bytes = reader.fed;
                });
                match self.unit(*prepared, at).await {
                    UnitDone::Ran => self.summary.units_run += 1,
                    UnitDone::Skipped => self.summary.units_skipped += 1,
                    UnitDone::Failed => {
                        self.summary.units_failed += 1;
                        self.summary.failed_at = Some(at);
                        break 'run None;
                    }
                    UnitDone::Stop(reason) => break 'run self.stop(reason, Some(at)),
                }
                let (run, failed) = (self.summary.units_run, self.summary.units_failed);
                self.publish(|p| {
                    p.units_run = run;
                    p.units_failed = failed;
                });
            }
            if end {
                break 'run None;
            }
        };
        self.finish(outcome).await
    }

    fn stop(&self, reason: StopReason, at: Option<ScriptPosition>) -> Option<ScriptStop> {
        Some(ScriptStop { reason, at })
    }

    /// The run's end: the transaction state, the history row, the outcome.
    async fn finish(&mut self, stop: Option<ScriptStop>) -> ScriptRunOutcome {
        // A context read is answered after the last unit's request, so the state has settled;
        // a unit a cancel did not reach may hold the session, so the wait is bounded.
        let _ = tokio::time::timeout(SETTLE_WAIT, self.req.session.run_context()).await;
        let snap = self.req.session.state().borrow().clone();
        self.summary.tx_open = snap.tx.status != hacivat_driver::TxStatus::Idle;
        let outcome = if let Some(stop) = &stop {
            match stop.reason {
                StopReason::Cancelled => Outcome::Cancelled,
                StopReason::Denied(_) | StopReason::ConfirmCancelled => Outcome::Denied,
                _ => Outcome::Failed,
            }
        } else if self.summary.units_failed > 0 {
            Outcome::Failed
        } else {
            Outcome::Ok
        };
        self.record(outcome, &snap).await;
        let (run, failed) = (self.summary.units_run, self.summary.units_failed);
        self.publish(|p| {
            p.units_run = run;
            p.units_failed = failed;
        });
        let summary = std::mem::take(&mut self.summary);
        match stop {
            Some(stop) => ScriptRunOutcome::Stopped(stop, summary),
            None if summary.units_failed > 0 => ScriptRunOutcome::Failed(summary),
            None => ScriptRunOutcome::Finished(summary),
        }
    }

    /// The one summary history row (R-M4b-24).
    async fn record(&self, outcome: Outcome, snap: &SessionSnapshot) {
        let Some(h) = &self.req.history else {
            return;
        };
        if h.meta.private {
            return;
        }
        let ms = |t: SystemTime| {
            t.duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
                .unwrap_or(0)
        };
        let row = HistoryRecord {
            ts_start_ms: ms(self.started_at),
            duration_ms: u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX),
            profile: h.meta.profile,
            database: h.database.clone(),
            schema: None,
            console: h.meta.console,
            origin: h.meta.origin,
            statement_masked: h.file_name.clone(),
            text_hash: [0; 32],
            fingerprint: 0,
            params_masked: None,
            outcome,
            rows_affected: None,
            rows_fetched: None,
            error_code: None,
            error_message: None,
            tx_mode: snap.tx.mode,
            run: None,
            redacted: false,
        };
        let _ = h.sink.send(Arc::new(row)).await;
    }

    /// The connection as the gate sees it now, under the live tag.
    fn conn(&self) -> ConnSafety {
        let snap = self.req.session.state().borrow().clone();
        ConnSafety {
            engine: self.req.engine,
            env: self.env(),
            read_only: snap.read_only,
            tx_status: snap.tx.status,
            tx_owner: snap.tx.owner,
            tx_mode: snap.tx.mode,
            confirm_unsafe_where: self.req.gate.confirm_unsafe_where,
        }
    }

    /// Why the run must stop for its connection: it was lost or reset since pass 2 began, or
    /// is not connected now (`Reconnected`), or a missed event hides whether it was
    /// (`LostTrack`: fail closed). `None`: it is the connection the run began on.
    ///
    /// ## m4b-t18-run-as-script (named exception, the T12 re-review's n5): the reason, not a
    /// flag, so a lagged stream says it lost track instead of "the connection was reset".
    fn connection_lost(&mut self) -> Option<StopReason> {
        let state = self.req.session.state().borrow().state.clone();
        lost_reason(&state, self.events.as_mut())
    }

    /// Waits for `fut`, or for Stop (`None`).
    async fn or_cancel<T>(&self, fut: impl Future<Output = T>) -> Option<T> {
        let mut cancel = self.req.cancel.clone();
        let stopped = async move {
            if cancel.wait_for(|c| *c).await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            () = stopped => None,
            v = fut => Some(v),
        }
    }

    /// One unit (pass 2; the module docs).
    async fn unit(&mut self, prepared: PreparedUnit, at: ScriptPosition) -> UnitDone {
        if self.cancelled() {
            return UnitDone::Stop(StopReason::Cancelled);
        }
        let session = self.req.session.clone();
        // The context, read after every earlier request on the session.
        let Some((sctx, generation)) = self.or_cancel(session.run_context()).await else {
            return UnitDone::Stop(StopReason::Cancelled);
        };
        if let Some(lost) = self.connection_lost() {
            return UnitDone::Stop(lost);
        }
        // ## m4b-t18-run-as-script (named exception; R-T14-2): the first unit is sent only
        // under the context generation the app's database check read; anything that moved the
        // context in between (a reconnect, a replay, a queued USE) stops the run, nothing sent.
        if let Some(checked) = self.checked_generation.take()
            && generation != Some(checked)
        {
            return UnitDone::Stop(StopReason::ContextChanged);
        }
        let (modes, known) = modes_of(self.req.engine, &sctx);
        if self.modes_fixed && known && modes.modes != self.dialect.modes {
            return UnitDone::Stop(StopReason::ContextChanged);
        }
        let conn = self.conn();
        if generation.is_none() && (conn.env == Some(EnvTag::Production) || conn.read_only) {
            return UnitDone::Stop(StopReason::ContextUnconfirmed);
        }
        // Classified in that context, exactly the bytes sent.
        let ctx = run_ctx_in(
            self.req.engine,
            self.req.gate.allowlist.clone(),
            &sctx,
            expect_for(&prepared.unit),
        );
        let texts = vec![prepared.final_text.clone()];
        let reports = match self
            .or_cancel(self.runner.classifier.classify_units(texts, ctx))
            .await
        {
            Some(Ok(r)) => r,
            Some(Err(never)) => match never {},
            None => return UnitDone::Stop(StopReason::Cancelled),
        };
        let prepared = vec![prepared];
        let mut plan = gate(
            &prepared,
            &reports,
            &conn,
            &self.suppressed,
            self.req.console,
        );
        let Some(item) = plan.items.first().cloned() else {
            return UnitDone::Stop(StopReason::NotSent);
        };
        let mut generation = generation;
        let mut skipped = false;
        match &item.decision {
            GateDecision::Deny(reason) => {
                return UnitDone::Stop(StopReason::Denied(reason.clone()));
            }
            GateDecision::Confirm { .. } | GateDecision::Warn(_) => {
                match self.answer(&item, &prepared[0], at, conn.env).await {
                    // Stop pressed while the dialog was open is a Stop, not the user's Cancel.
                    None if self.cancelled() => return UnitDone::Stop(StopReason::Cancelled),
                    // The user's No: the units they confirmed before it stay applied.
                    None => return UnitDone::Stop(StopReason::ConfirmCancelled),
                    Some(answer) => {
                        skipped = matches!(answer, ConfirmAnswer::Skip | ConfirmAnswer::SkipAll);
                        apply_answer(&mut plan, 0, answer);
                    }
                }
                if !skipped {
                    // The connection again after the dialog: a unit is sent only in the
                    // context and state it was classified and gated in.
                    let Some((again, g)) = self.or_cancel(session.run_context()).await else {
                        return UnitDone::Stop(StopReason::Cancelled);
                    };
                    let now = self.conn();
                    if again != sctx
                        || now.tx_status != conn.tx_status
                        || now.tx_mode != conn.tx_mode
                    {
                        return UnitDone::Stop(StopReason::ContextChanged);
                    }
                    generation = g;
                }
            }
            GateDecision::Run
            | GateDecision::RunInTxThenConfirm { .. }
            | GateDecision::ExplainAnalyzeRollback => {}
        }
        if self.cancelled() {
            return UnitDone::Stop(StopReason::Cancelled);
        }
        // Right before the send, as M3's Run: the connection it was gated on, still.
        if let Some(lost) = self.connection_lost() {
            return UnitDone::Stop(lost);
        }
        let now = self.conn();
        if now.env != conn.env || now.read_only != conn.read_only {
            return UnitDone::Stop(StopReason::ConnectionChanged);
        }
        let refresh = self.req.refresh.clone().map(|inbox| CatalogRun {
            inbox,
            dialect: self.dialect.id,
            reports: reports.iter().map(|r| Some(r.items.clone())).collect(),
        });
        let unit = prepared[0].unit.clone();
        // ## m4b-t18-run-as-script fix round 1 (named exception; review I2)
        let masked: Arc<[Arc<str>]> = Arc::from(vec![prepared[0].masked.clone()]);
        // The service's lines go to the unit's own log, then to the console's numbered by the
        // unit's place in the file.
        let unit_log = SessionLog::new();
        let req = RunRequest {
            console: self.req.console,
            engine: self.req.engine,
            units: vec![unit],
            fetch: FetchPolicy::default(),
            log: unit_log.clone(),
            prepared: Some(prepared),
            plan: Some(plan),
            policy: ErrorPolicy::StopOnError,
            run_as_one: false,
            history: None,
            confirm: Some(self.req.confirm.clone()),
            context_generation: generation,
        };
        let mut ticket = self.runner.exec.run_with_refresh(&session, req, refresh);
        // ## m4b-t18-run-as-script fix round 1 (named exception; review I2): the port follows
        // this unit's run (its confirm-in-transaction prompt shows the statement and its loss).
        self.req.confirm.follow_run(
            ticket.run,
            ticket.choices.clone(),
            ticket.status.clone(),
            masked,
            session.clone(),
        );
        let summary = match self.or_cancel(&mut ticket.done).await {
            Some(done) => done,
            None => {
                // ## m4b-t18-run-as-script fix round 1 (named exception; review m4)
                self.summary.cancelled_in_flight = true;
                drop(session.cancel(ticket.seq));
                match tokio::time::timeout(CANCEL_GRACE, &mut ticket.done).await {
                    // A unit that ended before the cancel reached it ran: it is counted.
                    Ok(Ok(s)) if s.executed > 0 && s.failed == 0 => self.summary.units_run += 1,
                    Ok(_) => {}
                    Err(_) => self.summary.last_unit_unknown = true,
                }
                self.copy_log(&unit_log, at.unit);
                return UnitDone::Stop(StopReason::Cancelled);
            }
        };
        // The results are not kept: the ticket goes here.
        drop(ticket);
        self.copy_log(&unit_log, at.unit);
        match summary {
            Ok(s) if s.failed > 0 => UnitDone::Failed,
            Ok(s) if s.executed > 0 => UnitDone::Ran,
            Ok(_) if skipped => UnitDone::Skipped,
            Ok(_) | Err(_) => UnitDone::Stop(StopReason::NotSent),
        }
    }

    /// The unit's log lines in the console's log, statement numbers being the unit's place in
    /// the file (`unit`); its per-run summary line is left out (the run writes one).
    ///
    /// ## demofix-1 (PO demo F6): a unit whose lines only say it ran (a row count, "Completed")
    /// waits in the run's [`QuietLog`]: the last [`SCRIPT_TAIL_STATEMENTS`] are written one by
    /// one before the next line that says more (a failure, a notice, a stop) and at the end,
    /// and earlier ones are counted in one `exec-script-quiet` line. The log keeps its order.
    fn copy_log(&mut self, unit_log: &SessionLog, unit: u64) {
        let n = u32::try_from(unit).unwrap_or(u32::MAX);
        let mut lines: Vec<UnitLine> = Vec::new();
        for line in unit_log.lines().iter() {
            let msg = match &line.msg {
                LogMsg::Summary(_) => continue,
                LogMsg::Error { view, .. } => LogMsg::Error {
                    n,
                    view: view.clone(),
                },
                LogMsg::SkippedByUser { .. } => LogMsg::SkippedByUser { n },
                LogMsg::Retried { .. } => LogMsg::Retried { n },
                LogMsg::RunStopped { .. } => LogMsg::RunStopped { n },
                LogMsg::RolledBackContinue { .. } => LogMsg::RolledBackContinue { n },
                other => other.clone(),
            };
            lines.push((msg, line.tag.clone(), line.source.clone(), line.unit));
        }
        if lines.is_empty() {
            return;
        }
        if lines.iter().all(|l| quiet_rows(&l.0).is_some()) {
            self.quiet.tail.push_back((unit, lines));
            if self.quiet.tail.len() > SCRIPT_TAIL_STATEMENTS
                && let Some((at, old)) = self.quiet.tail.pop_front()
            {
                let q = &mut self.quiet;
                if q.n == 0 {
                    q.first = at;
                }
                q.last = at;
                q.n += 1;
                q.rows = old
                    .iter()
                    .filter_map(|l| quiet_rows(&l.0))
                    .fold(q.rows, u64::saturating_add);
                if q.n >= SCRIPT_QUIET_EVERY {
                    self.write_counted();
                }
            }
            return;
        }
        self.write_quiet();
        for (msg, tag, source, unit) in lines {
            self.req.log.append_tagged(msg, tag, source, unit);
        }
    }

    // ## demofix-1 (PO demo F6)
    /// Writes the counted quiet statements as one line, if there are any.
    fn write_counted(&mut self) {
        let q = std::mem::take(&mut self.quiet.n);
        if q == 0 {
            return;
        }
        let (first, last, rows) = (self.quiet.first, self.quiet.last, self.quiet.rows);
        self.quiet.rows = 0;
        self.req.log.append(
            LogMsg::RunUi {
                id: "exec-script-quiet",
                args: vec![
                    ("first", LogArg::U64(first)),
                    ("last", LogArg::U64(last)),
                    ("n", LogArg::U64(q)),
                    ("rows", LogArg::U64(rows)),
                ],
                warn: false,
            },
            None,
            None,
        );
    }

    /// Writes the waiting quiet statements: the counted line, then the kept ones one by one.
    fn write_quiet(&mut self) {
        self.write_counted();
        for (_, lines) in std::mem::take(&mut self.quiet.tail) {
            for (msg, tag, source, unit) in lines {
                self.req.log.append_tagged(msg, tag, source, unit);
            }
        }
    }

    /// Asks about `item` (Confirm or Warn) unless an earlier answer covers it; `None` when
    /// the user cancelled (or closed the dialog). `env` is the tag it was gated under.
    async fn answer(
        &mut self,
        item: &GateItem,
        prepared: &PreparedUnit,
        at: ScriptPosition,
        env: Option<EnvTag>,
    ) -> Option<ConfirmAnswer> {
        let production = env == Some(EnvTag::Production);
        let needs_name = matches!(
            &item.decision,
            GateDecision::Confirm {
                type_to_confirm: Some(_),
                ..
            }
        );
        if self.skip_all {
            return Some(ConfirmAnswer::Skip);
        }
        if self.execute_all && !needs_name && !production {
            return Some(ConfirmAnswer::Execute);
        }
        let q = question(
            item,
            prepared,
            at,
            self.summary.units_total,
            production,
            self.req.gate.connection.clone(),
        )?;
        let offered = offers_suppress(&q);
        let rx = self.req.confirm.confirm(q);
        let answer = self.or_cancel(rx).await?.unwrap_or(ConfirmAnswer::Cancel);
        match answer {
            ConfirmAnswer::Cancel => None,
            ConfirmAnswer::Execute | ConfirmAnswer::Skip => Some(answer),
            ConfirmAnswer::ExecuteAll => {
                self.execute_all = !production;
                Some(ConfirmAnswer::Execute)
            }
            ConfirmAnswer::SkipAll => {
                self.skip_all = true;
                Some(ConfirmAnswer::Skip)
            }
            ConfirmAnswer::ExecuteAndSuppress => {
                if offered {
                    self.suppressed.insert(self.req.console, item.fingerprint);
                    self.summary.suppress.push(item.fingerprint);
                }
                Some(ConfirmAnswer::Execute)
            }
        }
    }
}

/// Pass 1's gate check (R-T12-2): every unit classified and gated under the session's state at
/// the start and the live tag; a unit the gate denies refuses the file.
struct PassGate {
    ctx: ClassifierContext,
    /// A unit that may change the context was met: later ones are classified in a context the
    /// classifier reads as unknown (fail closed), as M3's `classify_units` reads the units after
    /// such a unit.
    changed: bool,
}

impl PassGate {
    fn new(start: &ClassifierContext) -> PassGate {
        PassGate {
            ctx: start.clone(),
            changed: false,
        }
    }

    /// The first unit of `batch` the gate denies, with its reason.
    async fn check(
        &mut self,
        run: &Run<'_>,
        batch: Vec<(PreparedUnit, ScriptPosition)>,
    ) -> Option<(GateReason, ScriptPosition)> {
        if batch.is_empty() {
            return None;
        }
        let ctx = if self.changed {
            ClassifierContext {
                server_version: self.ctx.server_version.clone(),
                ..ClassifierContext::default()
            }
        } else {
            self.ctx.clone()
        };
        let classify = run_ctx_in(
            run.req.engine,
            run.req.gate.allowlist.clone(),
            &ctx,
            Expect::SingleStatement,
        );
        let (units, at): (Vec<PreparedUnit>, Vec<ScriptPosition>) = batch.into_iter().unzip();
        let texts: Vec<Arc<str>> = units.iter().map(|p| p.final_text.clone()).collect();
        let reports = match run.runner.classifier.classify_units(texts, classify).await {
            Ok(r) => r,
            Err(never) => match never {},
        };
        let plan = gate(
            &units,
            &reports,
            &run.conn(),
            &run.suppressed,
            run.req.console,
        );
        for (item, pos) in plan.items.iter().zip(&at) {
            if let GateDecision::Deny(reason) = &item.decision {
                return Some((reason.clone(), *pos));
            }
        }
        if units
            .iter()
            .zip(&reports)
            .any(|(p, r)| crate::changes_context(&p.final_text, r))
        {
            self.changed = true;
        }
        None
    }
}

/// A source with nothing left (what a run holds once it handed its source on).
struct Spent;

impl ScriptChunks for Spent {
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
        None
    }

    fn reopen_verified(
        self: Box<Self>,
    ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
        Box::pin(async { Err(ChunkError::Changed) })
    }
}

/// The Fluent IDs of the gate's reasons that are never "don't ask again" (the app's dialog
/// rule, `exec_ui::confirm::offers_suppress`).
const NOT_ANALYSED: &str = "exec-gate-not-analysed";
const IMPLICIT_COMMIT: &str = "exec-gate-implicit-commit";

/// Whether the dialog for `q` offers "Execute and don't ask again" (the app's rule: never with
/// a typed name, never for an unanalysed statement or an implicit commit, never on production,
/// and only when the gate marked it suppressible).
fn offers_suppress(q: &ConfirmQuestion) -> bool {
    let unanalysed = q
        .reasons
        .iter()
        .any(|r| r.fluent_id == NOT_ANALYSED || r.fluent_id == IMPLICIT_COMMIT);
    q.suppressible && q.type_to_confirm.is_none() && !unanalysed && !q.production
}

/// The question for `item` ("statement `at.unit` of `total`"), its decision's facts unchanged.
fn question(
    item: &GateItem,
    prepared: &PreparedUnit,
    at: ScriptPosition,
    total: u64,
    production: bool,
    connection: Arc<str>,
) -> Option<ConfirmQuestion> {
    let (reasons, type_to_confirm, irreversible, suppressible, warn_only) = match &item.decision {
        GateDecision::Confirm {
            reasons,
            type_to_confirm,
            irreversible,
            suppressible,
        } => (
            reasons.clone(),
            type_to_confirm.clone(),
            *irreversible,
            *suppressible,
            false,
        ),
        GateDecision::Warn(reasons) => (reasons.clone(), None, false, true, true),
        _ => return None,
    };
    let index = u32::try_from(at.unit).unwrap_or(u32::MAX);
    Some(ConfirmQuestion {
        run: hacivat_session::session::RunId(0),
        unit: index.saturating_sub(1),
        index,
        total: u32::try_from(total).unwrap_or(u32::MAX),
        sql_masked: prepared.masked.clone(),
        reasons,
        type_to_confirm,
        irreversible,
        suppressible,
        warn_only,
        production,
        connection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source handing out `text` in reads of `sizes` (cycled, widened to a character
    /// boundary).
    struct Reads {
        text: String,
        sizes: Vec<usize>,
        at: usize,
        k: usize,
    }

    impl ScriptChunks for Reads {
        fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
            if self.at >= self.text.len() {
                return None;
            }
            let mut end = (self.at + self.sizes[self.k % self.sizes.len()]).min(self.text.len());
            while !self.text.is_char_boundary(end) {
                end += 1;
            }
            let s = self.text[self.at..end].to_string();
            self.at = end;
            self.k += 1;
            Some(Ok(s))
        }

        fn reopen_verified(
            self: Box<Self>,
        ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
            Box::pin(async { Err(ChunkError::Changed) })
        }
    }

    fn rechunked(text: &str, sizes: &[usize]) -> Vec<String> {
        let mut r = Rechunk::new(Box::new(Reads {
            text: text.to_string(),
            sizes: sizes.to_vec(),
            at: 0,
            k: 0,
        }));
        let mut out = Vec::new();
        while let Some(c) = r.next() {
            out.push(c.expect("no read fails"));
        }
        out
    }

    #[test]
    fn rechunk_depends_on_the_text_alone() {
        // R-T8-1: whatever reads the source makes, the splitter is fed the same chunks.
        let text: String = (0..40_000).map(|i| format!("çğ{i}ı;\n")).collect();
        let want = rechunked(&text, &[SCRIPT_CHUNK_BYTES]);
        assert!(want.len() > 3);
        for sizes in [
            vec![1],
            vec![3, 1, 17],
            vec![70_000],
            vec![1 << 20],
            vec![65_535, 2, 65_537],
            vec![text.len()],
        ] {
            assert_eq!(rechunked(&text, &sizes), want, "reads of {sizes:?}");
        }
        assert_eq!(want.concat(), text);
        for c in &want[..want.len() - 1] {
            assert!(c.len() <= SCRIPT_CHUNK_BYTES && c.len() + 4 > SCRIPT_CHUNK_BYTES);
        }
    }

    #[test]
    fn rechunk_stops_at_a_read_error() {
        struct Fails(u8);
        impl ScriptChunks for Fails {
            fn next_chunk(&mut self) -> Option<Result<String, ChunkError>> {
                self.0 += 1;
                match self.0 {
                    1 => Some(Ok("SELECT 1;\n".into())),
                    2 => Some(Err(ChunkError::Changed)),
                    _ => Some(Ok("DELETE FROM t;\n".into())),
                }
            }
            fn reopen_verified(
                self: Box<Self>,
            ) -> BoxFuture<'static, Result<Box<dyn ScriptChunks>, ChunkError>> {
                Box::pin(async { Err(ChunkError::Changed) })
            }
        }
        let mut r = Rechunk::new(Box::new(Fails(0)));
        // The text before the error is never handed out with text after it.
        assert_eq!(r.next(), Some(Err(ChunkError::Changed)));
        assert_eq!(r.next(), None);
    }

    #[test]
    fn lines_count_newlines_up_to_each_offset() {
        let mut l = Lines::new();
        l.push("a\nb\n");
        l.push("c\r\nd");
        assert_eq!(l.at(0), 1);
        assert_eq!(l.at(2), 2);
        assert_eq!(l.at(4), 3);
        assert_eq!(l.at(7), 4);
        assert_eq!(l.at(1), 4, "never backwards");
        assert_eq!(l.held(), 1);
    }

    #[test]
    fn lines_count_lone_carriage_returns() {
        // Review m8: a CR-only file numbers its lines too; `\r\n` split across two pushes
        // still counts once.
        let mut l = Lines::new();
        l.push("a\rb\r");
        l.push("\nc\rd");
        assert_eq!(l.at(2), 2);
        assert_eq!(l.at(5), 3);
        assert_eq!(l.at(8), 4);
    }

    #[test]
    fn mode_variants_cover_every_lexing_combination() {
        let pg = DialectCtx::new(DialectId::Postgres, None);
        assert_eq!(mode_variants(&pg).len(), 2);
        assert_eq!(mode_variants(&pg)[0], pg);
        let my = DialectCtx::new(DialectId::MySql, None);
        assert_eq!(mode_variants(&my).len(), 4);
        let maria = DialectCtx::new(DialectId::MariaDb, None);
        assert_eq!(mode_variants(&maria).len(), 8);
        let lite = DialectCtx::new(DialectId::Sqlite, None);
        assert_eq!(mode_variants(&lite), [lite]);
    }

    #[test]
    fn copy_stream_reads_the_splitters_head() {
        let pg = DialectCtx::new(DialectId::Postgres, None);
        assert!(copy_stream("COPY t FROM STDIN", &pg, ClientFlavor::Psql));
        assert!(copy_stream("copy t to stdout", &pg, ClientFlavor::Psql));
        assert!(!copy_stream(
            "COPY t FROM '/tmp/x.csv'",
            &pg,
            ClientFlavor::Psql
        ));
        assert!(!copy_stream("COPYRIGHT", &pg, ClientFlavor::Psql));
        assert!(!copy_stream(
            "SELECT 'COPY t FROM STDIN'",
            &pg,
            ClientFlavor::Psql
        ));
    }

    #[test]
    fn suppress_is_never_offered_on_production_or_with_a_typed_name() {
        let item = GateItem {
            unit: 0,
            decision: GateDecision::Confirm {
                reasons: Vec::new(),
                type_to_confirm: None,
                irreversible: false,
                suppressible: true,
            },
            verdict: hacivat_session::session::UnitVerdict::Write,
            effects: hacivat_session::session::SessionEffects::empty(),
            class: crate::GateClass::Dml,
            fingerprint: 9,
        };
        let unit = ExecUnit {
            text: Arc::from("DELETE FROM t"),
            doc_range: 0..13,
            kind: UnitKind::Statement,
            suspendable: false,
            title: None,
        };
        let prepared = PreparedUnit {
            final_text: unit.text.clone(),
            masked: unit.text.clone(),
            binds: Arc::from(Vec::new()),
            offsets: crate::OffsetMap::identity(0),
            unit,
        };
        let at = ScriptPosition { unit: 3, line: 7 };
        let q = question(&item, &prepared, at, 9, false, Arc::from("c")).expect("asks");
        assert_eq!((q.index, q.total, q.unit), (3, 9, 2));
        assert!(offers_suppress(&q));
        let q = question(&item, &prepared, at, 9, true, Arc::from("c")).expect("asks");
        assert!(!offers_suppress(&q));
    }
}

// ## m4b-t18-run-as-script (named exception; the T12 re-review's n5)
#[cfg(test)]
mod t18_tests {
    use super::*;

    #[test]
    fn a_lagged_or_closed_event_stream_is_a_lost_track_never_a_reset() {
        let (tx, mut rx) = broadcast::channel(2);
        for req in 0..3 {
            let _ = tx.send(SessionEvent::SlowCancel { req });
        }
        assert_eq!(
            lost_reason(&SessionState::Idle, Some(&mut rx)),
            Some(StopReason::LostTrack)
        );
        // After the lag the stream reads again: ordinary events are no loss.
        assert_eq!(lost_reason(&SessionState::Idle, Some(&mut rx)), None);
        let _ = tx.send(SessionEvent::Reconnected);
        assert_eq!(
            lost_reason(&SessionState::Idle, Some(&mut rx)),
            Some(StopReason::Reconnected)
        );
        drop(tx);
        assert_eq!(
            lost_reason(&SessionState::Idle, Some(&mut rx)),
            Some(StopReason::LostTrack)
        );
    }

    #[test]
    fn a_session_that_is_not_connected_was_reset() {
        for state in [SessionState::Disconnected, SessionState::Connecting] {
            assert_eq!(lost_reason(&state, None), Some(StopReason::Reconnected));
        }
        assert_eq!(lost_reason(&SessionState::Idle, None), None);
    }
}

// ## m4b-t18-run-as-script (named exception): the app runs a script on the core runtime.
#[cfg(test)]
mod t18_send {
    use super::*;

    fn is_send<T: Send>(_: &T) {}

    #[allow(dead_code)]
    fn a_run_is_send(runner: &ScriptRunner, req: ScriptRunRequest) {
        let (tx, _) = watch::channel(ScriptProgress::default());
        is_send(&runner.run_checked(req, tx, None));
    }
}
