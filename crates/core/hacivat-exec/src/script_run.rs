// ## m4b-t01a-seams
// ## m4b-t12-script-run (the seam's body; ADR 0006 `m4b-t12`)
//! "Run as script" (HV-SCRIPT-12, the reduced HV-EXEC-15; R-M4b-20, R-M4b-24, R-M4b-47,
//! R-T8-1, R-T8-3, R-T10-3): a script too large for the editor streams from disk through the
//! `StreamSplitter`, and every unit is classified, gated and confirmed exactly as a Run's are,
//! one at a time, on the session the tab was opened under.
//!
//! **Two passes (R-M4b-47).** Pass 1 streams the whole file with `StreamPolicy::Eager` and runs
//! nothing: any item that refuses (`stream::any_refuses` on every `feed` batch and on `finish`:
//! an uncertain boundary, a statement that is not `Exact`, `TooLong`, `NoBoundary`), an
//! unsupported client command, a `COPY … FROM STDIN`/`TO STDOUT`, a statement with a parameter,
//! or a read error refuses the **whole file**, as M3's whole split refuses it. Only a clean pass
//! 1 opens pass 2 ([`ScriptChunks::reopen_verified`]), which runs. Every dialect gets both
//! passes: `Eager` is `Exact` off MySQL/MariaDB (R-T8-4), and `Exact` there would otherwise run
//! the statements before a late refusal.
//!
//! **Identical chunks (R-T8-1).** The splitter is fed one `feed` per chunk of [`Rechunk`], which
//! cuts the text the source hands out into [`SCRIPT_CHUNK_BYTES`] pieces at a character
//! boundary: the chunks are a function of the text alone, whatever reads the source made, so
//! pass 2 feeds exactly pass 1's sequence. Pass 1 records each chunk's length and SHA-256; pass
//! 2 checks every chunk against that record **before** feeding it, and checks that it ends where
//! pass 1 ended: a difference is a stop (`FileChanged`), so pass 2 never runs text pass 1 did
//! not check, even under a source that would not notice a change.
//!
//! **Change detection (R-T10-3).** The source owns the stamp: its `next_chunk` re-checks the
//! open file's stamp per read and answers `ChunkError::Changed` once it moved (the one handle a
//! run keeps, HV-SCRIPT-12 step 4), and `reopen_verified` refuses a file that is not the one
//! pass 1 read. A change between the passes refuses the file (nothing ran); a change in pass 2
//! stops the run before the next unit. The statements built from a chunk run only after that
//! chunk was read and checked.
//!
//! **Held text (T8 m-fr3-3, R-T8-3).** Off MySQL/MariaDB the stream is `Exact`, which holds a
//! line with no boundary whole: once it holds more than [`STREAM_HELD_MAX`] (64 MiB) the file is
//! refused with `NoBoundary` ("no statement boundary within 64 MiB").
//!
//! **Per unit (pass 2).** The session's context is read (`run_context`, after every earlier
//! request; its lexing modes must still be the ones the file is split in), the unit is
//! classified on the classifier pool in that context, gated with the connection as it is now
//! (`gate`, the function Run uses), asked about through the same [`ConfirmPort`] (Execute all
//! never answers a typed name, and covers nothing on production; Skip all skips; "don't ask
//! again" only where the dialog offers it), and sent through [`ExecService`] as a checked
//! one-unit plan carrying the context generation it was classified under (a confirm-in-
//! transaction question goes to the same port). Results are not kept: the ticket is dropped once
//! the unit ends, so only the log lines remain (the console's log ring is capped). Stop works
//! between units and cancels a unit in flight. Every policy stops at the first failure: a
//! streamed run asks no question per unit (R-M3-61's dialog needs units still waiting in one
//! run). A transaction left open stays open; the session's state carries it to the M3
//! transaction UI, and the summary says so.
//!
//! **History (R-M4b-24).** One summary row (origin `File`, the file's name as its text), never a
//! row per unit.
//!
//! **Memory.** Bounded by the splitter's own bound (T8: about 1 KiB held for an INSERT dump;
//! up to 64 MiB held, about 287 MB RSS, on the adversarial shapes it refuses) plus, here, one
//! chunk being cut, the text from the last unit's start to the end of what was fed (the line
//! count; at most the held text plus a chunk), 40 bytes of pass record per 64 KiB chunk, and the
//! per-unit buffers of the session.
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
use hacivat_session::session::{ErrorPolicy, SessionHandle, SessionSnapshot};
use sha2::{Digest, Sha256};
use sql_dialect::{ClientFlavor, DialectCtx, DialectId};
use sql_split::exec_range::UnitKind;
use sql_split::stream::{STREAM_HELD_MAX, StreamItem, StreamPolicy, StreamSplitter, any_refuses};
use sql_split::{AmbiguityReason, Confidence, ParamDiag, ParamSettings, SplitMode, SplitStart};
use tokio::sync::{mpsc, watch};

use crate::classify_pool::{ClassifyPool, modes_of, run_ctx_in};
use crate::gate::{
    ConfirmAnswer, ConfirmPort, ConfirmQuestion, ConnSafety, GateDecision, GateItem, GateReason,
    gate,
};
use crate::history::{HistoryMeta, HistoryRecord, Outcome};
use crate::log::SessionLog;
use crate::params::{ParamValueStore, Prepared, PreparedUnit, prepare_units};
use crate::service::{CatalogRun, ExecService, RunRequest, apply_answer};
use crate::suppress::SuppressSet;
use crate::units::{ExecUnit, expect_for};

/// The size of the chunks the splitter is fed (64 KiB, cut back to a character boundary).
pub const SCRIPT_CHUNK_BYTES: usize = 64 << 10;

/// A script read from disk in UTF-8 chunks, each cut at a character boundary: one pass over
/// the file. The runner reads a pass to its end, then asks for the next one.
pub trait ScriptChunks: Send {
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
/// (from 1; a line ends at `\n`).
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
    /// No statement boundary within 64 MiB of text (R-T8-3).
    NoBoundary,
    /// A client command Hacivat does not run as the client would (`\c`, `\i`, `SOURCE`…).
    ClientCommand,
    /// A statement with a parameter (`None`: it needs a value; no prompt in a streamed run), or
    /// one whose parameters can never be bound.
    Params(Option<ParamDiag>),
    /// `COPY … FROM STDIN` / `TO STDOUT`: consoles do not stream data.
    CopyUnsupported,
    /// The file could not be read, or changed (also between the passes).
    Read(ChunkError),
    /// The session reads text in other lexing modes than the file is split in.
    ModesDiffer,
    /// Stop was pressed during pass 1.
    Cancelled,
}

impl RefuseReason {
    /// The Fluent ID that says why (`hacivat-core.ftl`, block `m4b-t12-script-run`); `$line`
    /// names the line where the refusal has one. An uncertain boundary's own reason is
    /// `units::uncertain_message_id`'s, a parameter's diagnostic `param_diag_message_id`'s.
    pub fn fluent_id(&self) -> &'static str {
        match self {
            RefuseReason::Uncertain(_) | RefuseReason::NotExact => "exec-script-uncertain",
            RefuseReason::TooLong => "exec-script-too-long",
            RefuseReason::NoBoundary => "exec-script-no-boundary",
            RefuseReason::ClientCommand => "exec-script-client-command",
            RefuseReason::Params(_) => "exec-script-params",
            RefuseReason::CopyUnsupported => "exec-script-copy",
            RefuseReason::Read(e) => read_id(e, false),
            RefuseReason::ModesDiffer => "exec-script-modes-differ",
            RefuseReason::Cancelled => "exec-script-cancelled-before",
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
    /// The file could not be read or decoded mid-run.
    Read(ChunkError),
    /// The gate denied the unit (its reason).
    Denied(GateReason),
    /// The user cancelled the unit's confirmation.
    ConfirmCancelled,
    /// The session's context moved: other lexing modes than the split's, or a change while a
    /// confirmation was open.
    ContextChanged,
    /// The context of a production or read-only session could not be read.
    ContextUnconfirmed,
    /// The session did not run the unit (it refused it, or never started the run).
    NotSent,
    /// Pass 2 met what pass 1 did not (never on an unchanged file; fail closed).
    Split(RefuseReason),
}

impl StopReason {
    /// The Fluent ID that says why (`hacivat-core.ftl`, block `m4b-t12-script-run`); a denial's
    /// own reason is the gate's.
    pub fn fluent_id(&self) -> &'static str {
        match self {
            StopReason::Cancelled => "exec-script-cancelled",
            StopReason::FileChanged => "exec-script-file-changed",
            StopReason::Read(e) => read_id(e, true),
            StopReason::Denied(_) => "exec-script-denied",
            StopReason::ConfirmCancelled => "exec-script-confirm-cancelled",
            StopReason::ContextChanged => "exec-script-context-changed",
            StopReason::ContextUnconfirmed => "exec-script-context-unconfirmed",
            StopReason::NotSent => "exec-script-not-sent",
            StopReason::Split(r) => r.fluent_id(),
        }
    }
}

fn read_id(e: &ChunkError, running: bool) -> &'static str {
    match (e, running) {
        (ChunkError::Changed, true) => "exec-script-file-changed",
        (ChunkError::Changed, false) => "exec-script-file-changed-before",
        (ChunkError::Decode, _) => "exec-script-decode-failed",
        (ChunkError::Io(_), _) => "exec-script-read-failed",
    }
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
    /// The most bytes the runner held at once (the splitter's text, the line count's text and
    /// the chunk being cut).
    pub peak_held: usize,
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
/// environment tag and setting, its function allowlist, the console's "don't ask again" pairs,
/// and the connection's name for the dialogs.
#[derive(Debug, Clone)]
pub struct ScriptGate {
    pub env: Option<EnvTag>,
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
/// where confirmations go, the console's log, the error policy, Stop, the history and the
/// catalog refresh.
pub struct ScriptRunRequest {
    pub session: SessionHandle,
    pub source: Box<dyn ScriptChunks>,
    pub console: u64,
    pub profile: ProfileId,
    pub engine: DbmsKind,
    /// The context the file is split, scanned and checked in (the tab's).
    pub dialect: DialectCtx,
    pub flavor: ClientFlavor,
    pub gate: ScriptGate,
    pub confirm: Arc<dyn ConfirmPort>,
    pub log: Arc<SessionLog>,
    /// `StopOnError` by default; every policy stops at the first failure (the module docs).
    pub policy: ErrorPolicy,
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
            .field("policy", &self.policy)
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
        let mut run = Run::new(self, req, progress);
        let out = run.go().await;
        run.publish(|p| p.phase = ScriptPhase::Done);
        out
    }
}

/// [`ScriptRunner::run`] as a free function.
pub async fn run_file(
    runner: &ScriptRunner,
    req: ScriptRunRequest,
    progress: watch::Sender<ScriptProgress>,
) -> ScriptRunOutcome {
    runner.run(req, progress).await
}

// =================================================================================================
// Reading: identical chunks, the pass record, line numbers.

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
/// offset asked about to the end of what was fed, and the line at that offset.
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

    /// The line at stream offset `off` (an offset before the last one asked about answers that
    /// one's line); the text before `off` is dropped.
    fn at(&mut self, off: u64) -> u64 {
        if off > self.base {
            let n = usize::try_from(off - self.base).unwrap_or(usize::MAX);
            let end = self.pos.saturating_add(n).min(self.text.len());
            let newlines = self.text[self.pos..end].iter().filter(|&&b| b == b'\n');
            self.line += newlines.count() as u64;
            self.base += (end - self.pos) as u64;
            self.pos = end;
        }
        self.line
    }

    fn held(&self) -> usize {
        self.text.len() - self.pos
    }
}

/// One item of the stream with its line.
struct Located {
    item: StreamItem,
    line: u64,
}

/// What a step of a pass read.
enum Step {
    /// The items a chunk settled (`end`: the stream's last ones); `refuses`:
    /// `stream::any_refuses` of the batch (R-M4b-47).
    Items {
        items: Vec<Located>,
        end: bool,
        refuses: bool,
    },
    Failed(ChunkError),
    /// The splitter holds more than 64 MiB of text with no boundary (off MySQL/MariaDB).
    Held {
        line: u64,
    },
}

/// One pass over the file: the chunks, the splitter, the line count and the pass record.
struct Reader {
    chunks: Rechunk,
    split: Option<StreamSplitter>,
    lines: Lines,
    fed: u64,
    /// The held-text guard (off MySQL/MariaDB, T8 m-fr3-3).
    guard: bool,
    record: PassRecord,
    /// Pass 2: the next chunk of `record` to check.
    check: Option<usize>,
    peak: usize,
}

impl Reader {
    fn new(source: Box<dyn ScriptChunks>, dialect: DialectCtx, flavor: ClientFlavor) -> Reader {
        Reader {
            chunks: Rechunk::new(source),
            split: Some(StreamSplitter::with_policy(
                dialect,
                flavor,
                StreamPolicy::Eager,
            )),
            lines: Lines::new(),
            fed: 0,
            guard: !matches!(dialect.id, DialectId::MySql | DialectId::MariaDb),
            record: PassRecord::default(),
            check: None,
            peak: 0,
        }
    }

    /// Pass 2 over `source`, checked against this pass's record.
    fn second(
        self,
        source: Box<dyn ScriptChunks>,
        dialect: DialectCtx,
        flavor: ClientFlavor,
    ) -> Reader {
        Reader {
            record: self.record,
            check: Some(0),
            peak: self.peak,
            ..Reader::new(source, dialect, flavor)
        }
    }

    fn locate(&mut self, items: Vec<StreamItem>) -> Vec<Located> {
        items
            .into_iter()
            .map(|item| {
                let line = self.lines.at(offset_of(&item));
                Located { item, line }
            })
            .collect()
    }

    /// Reads one chunk and feeds it (blocking: the source reads, the splitter splits).
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
                    };
                };
                let items = split.feed(&chunk);
                let held = split.held_len();
                let refuses = any_refuses(&items);
                let located = self.locate(items);
                self.peak = self
                    .peak
                    .max(held + self.lines.held() + self.chunks.buffered() + chunk.len());
                if self.guard && held > STREAM_HELD_MAX {
                    let line = self.lines.at(self.fed - held as u64);
                    return Step::Held { line };
                }
                // Later items start in the held text: what lies before it is never asked for.
                self.lines.at(self.fed - held as u64);
                Step::Items {
                    items: located,
                    end: false,
                    refuses,
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
                Step::Items {
                    items: located,
                    end: true,
                    refuses,
                }
            }
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
    /// Execute all answered (off production): later calm confirmations run.
    execute_all: bool,
    skip_all: bool,
    started: Instant,
    started_at: SystemTime,
}

impl<'a> Run<'a> {
    fn new(
        runner: &'a ScriptRunner,
        req: ScriptRunRequest,
        progress: watch::Sender<ScriptProgress>,
    ) -> Run<'a> {
        let suppressed = req.gate.suppressed.clone();
        Run {
            runner,
            req,
            progress,
            summary: ScriptRunSummary::default(),
            suppressed,
            execute_all: false,
            skip_all: false,
            started: Instant::now(),
            started_at: SystemTime::now(),
        }
    }

    fn cancelled(&self) -> bool {
        *self.req.cancel.borrow()
    }

    fn production(&self) -> bool {
        self.req.gate.env == Some(EnvTag::Production)
    }

    fn publish(&self, f: impl FnOnce(&mut ScriptProgress)) {
        self.progress.send_modify(f);
    }

    async fn go(&mut self) -> ScriptRunOutcome {
        let refuse = |reason, at| ScriptRunOutcome::Refused(ScriptRefusal { reason, at });
        // The session must read text as the file is split.
        let (sctx, _) = self.req.session.run_context().await;
        let (now, known) = modes_of(self.req.engine, &sctx);
        if known && now.modes != self.req.dialect.modes {
            return refuse(RefuseReason::ModesDiffer, None);
        }
        let placeholder: Box<dyn ScriptChunks> = Box::new(Spent);
        let source = std::mem::replace(&mut self.req.source, placeholder);
        let (dialect, flavor) = (self.req.dialect, self.req.flavor);

        // Pass 1: nothing runs.
        let mut reader = Reader::new(source, dialect, flavor);
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
            let (items, end, refuses) = match step {
                Step::Failed(e) => return refuse(RefuseReason::Read(e), None),
                Step::Held { line } => {
                    return refuse(
                        RefuseReason::NoBoundary,
                        Some(ScriptPosition {
                            unit: units + 1,
                            line,
                        }),
                    );
                }
                Step::Items {
                    items,
                    end,
                    refuses,
                } => (items, end, refuses),
            };
            // R-M4b-47: every batch is checked (`any_refuses`), not only `stopped()`; the item
            // that refuses names where.
            for l in &items {
                let unit_no = units + 1;
                match check_item(&l.item, &dialect, flavor) {
                    Checked::Unit(_) => units += 1,
                    Checked::Client => {}
                    Checked::Refused(reason) => {
                        return refuse(
                            reason,
                            Some(ScriptPosition {
                                unit: unit_no,
                                line: l.line,
                            }),
                        );
                    }
                }
            }
            if refuses {
                // `check_item` refuses every item that `refuses()`; this stays the last word.
                return refuse(RefuseReason::NotExact, None);
            }
            let line = items.last().map(|l| l.line);
            self.publish(|p| {
                p.bytes = reader.fed;
                p.units_seen = units;
                p.line = line.unwrap_or(p.line);
            });
            if end {
                break;
            }
        }
        self.summary.units_total = units;
        self.summary.peak_held = reader.peak;

        // Between the passes: the file must be the one pass 1 read.
        if self.cancelled() {
            return refuse(RefuseReason::Cancelled, None);
        }
        let inner = std::mem::replace(&mut reader.chunks.inner, Box::new(Spent));
        let second = match inner.reopen_verified().await {
            Ok(s) => s,
            Err(e) => return refuse(RefuseReason::Read(e), None),
        };
        let mut reader = reader.second(second, dialect, flavor);
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
                Step::Held { line } => {
                    let at = ScriptPosition {
                        unit: ordinal + 1,
                        line,
                    };
                    break 'run self.stop(StopReason::Split(RefuseReason::NoBoundary), Some(at));
                }
                Step::Items { items, end, .. } => (items, end),
            };
            for l in items {
                let at = ScriptPosition {
                    unit: ordinal + 1,
                    line: l.line,
                };
                let prepared = match check_item(&l.item, &dialect, flavor) {
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
        // A context read is answered after the last unit's request: the state is settled then.
        let _ = self.req.session.run_context().await;
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

    /// The connection as the gate sees it now.
    fn conn(&self) -> ConnSafety {
        let snap = self.req.session.state().borrow().clone();
        ConnSafety {
            engine: self.req.engine,
            env: self.req.gate.env,
            read_only: snap.read_only,
            tx_status: snap.tx.status,
            tx_owner: snap.tx.owner,
            tx_mode: snap.tx.mode,
            confirm_unsafe_where: self.req.gate.confirm_unsafe_where,
        }
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
        let (modes, known) = modes_of(self.req.engine, &sctx);
        if known && modes.modes != self.req.dialect.modes {
            return UnitDone::Stop(StopReason::ContextChanged);
        }
        let conn = self.conn();
        if generation.is_none() && (self.production() || conn.read_only) {
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
                match self.answer(&item, &prepared[0], at).await {
                    // Stop pressed while the dialog was open is a Stop, not the user's Cancel.
                    None if self.cancelled() => return UnitDone::Stop(StopReason::Cancelled),
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
                        || now.read_only != conn.read_only
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
        let refresh = self.req.refresh.clone().map(|inbox| CatalogRun {
            inbox,
            dialect: self.req.dialect.id,
            reports: reports.iter().map(|r| Some(r.items.clone())).collect(),
        });
        let unit = prepared[0].unit.clone();
        let req = RunRequest {
            console: self.req.console,
            engine: self.req.engine,
            units: vec![unit],
            fetch: FetchPolicy::default(),
            log: self.req.log.clone(),
            prepared: Some(prepared),
            plan: Some(plan),
            policy: ErrorPolicy::StopOnError,
            run_as_one: false,
            history: None,
            confirm: Some(self.req.confirm.clone()),
            context_generation: generation,
        };
        let mut ticket = self.runner.exec.run_with_refresh(&session, req, refresh);
        let summary = match self.or_cancel(&mut ticket.done).await {
            Some(done) => done,
            None => {
                drop(session.cancel(ticket.seq));
                let late = tokio::time::timeout(Duration::from_secs(30), &mut ticket.done).await;
                // A unit that ended before the cancel reached it ran: it is counted.
                if let Ok(Ok(s)) = late
                    && s.executed > 0
                    && s.failed == 0
                {
                    self.summary.units_run += 1;
                }
                return UnitDone::Stop(StopReason::Cancelled);
            }
        };
        // The results are not kept: the ticket goes here.
        drop(ticket);
        match summary {
            Ok(s) if s.failed > 0 => UnitDone::Failed,
            Ok(s) if s.executed > 0 => UnitDone::Ran,
            Ok(_) if skipped => UnitDone::Skipped,
            Ok(_) | Err(_) => UnitDone::Stop(StopReason::NotSent),
        }
    }

    /// Asks about `item` (Confirm or Warn) unless an earlier answer covers it; `None` when
    /// the user cancelled (or closed the dialog).
    async fn answer(
        &mut self,
        item: &GateItem,
        prepared: &PreparedUnit,
        at: ScriptPosition,
    ) -> Option<ConfirmAnswer> {
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
        if self.execute_all && !needs_name && !self.production() {
            return Some(ConfirmAnswer::Execute);
        }
        let q = question(
            item,
            prepared,
            at,
            self.summary.units_total,
            self.production(),
            self.req.gate.connection.clone(),
        )?;
        let offered = offers_suppress(&q);
        let rx = self.req.confirm.confirm(q);
        let answer = self.or_cancel(rx).await?.unwrap_or(ConfirmAnswer::Cancel);
        match answer {
            ConfirmAnswer::Cancel => None,
            ConfirmAnswer::Execute | ConfirmAnswer::Skip => Some(answer),
            ConfirmAnswer::ExecuteAll => {
                self.execute_all = !self.production();
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
