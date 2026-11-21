// ## m4b-t01a-seams
//! "Run as script" (HV-SCRIPT-12, R-M4b-20, -46): a script too large for the editor streams
//! from disk through the `StreamSplitter`, and every unit is classified and gated exactly as a
//! Run's are, on the session the tab was opened under.
//!
//! Task 1a: the seams as fail-closed stubs (F31): [`ScriptRunner::run`] and [`run_file`]
//! answer [`ScriptRunOutcome::Refused`] with [`RefuseReason::NotImplemented`], pull no chunk
//! and send nothing to the session. Task 12 writes the body (one open handle; per chunk: read,
//! re-check the stamp, then release the statements built from it; stop on a change).
//!
//! [`ScriptChunks`] is the source: `hacivat_scripts::LibraryService::open_stream` returns that
//! crate's own mirror of it, which the app adapts (neither crate depends on the other,
//! R-M4b-28).

use std::fmt;
use std::io;
use std::sync::Arc;

use hacivat_proto::DbmsKind;
use hacivat_session::session::{ErrorPolicy, SessionHandle};

use crate::log::SessionLog;

/// A script read from disk in UTF-8 chunks, each cut at a character boundary.
pub trait ScriptChunks: Send {
    /// The next chunk, `None` at the end.
    fn next_chunk(&mut self) -> Option<Result<String, ChunkError>>;
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

/// Why a script run sent nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefuseReason {
    /// Task 1a's stub runs nothing.
    NotImplemented,
}

/// Why a script run stopped before the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    Cancelled,
    /// The file changed on disk mid-run (a notice; carries-w1 T12).
    FileChanged,
    /// The gate or the user refused a unit.
    Refused,
}

/// What a script run did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptRunSummary {
    /// Units the session ran.
    pub units_run: u64,
    /// Units that failed.
    pub units_failed: u64,
}

/// How a script run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptRunOutcome {
    /// Nothing was sent.
    Refused(RefuseReason),
    Finished(ScriptRunSummary),
    Stopped(StopReason, ScriptRunSummary),
    Failed(ScriptRunSummary),
}

/// One script run: the tab's console, its engine and log, the error policy.
pub struct ScriptRunRequest {
    pub console: u64,
    pub engine: DbmsKind,
    pub log: Arc<SessionLog>,
    pub policy: ErrorPolicy,
}

impl fmt::Debug for ScriptRunRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptRunRequest")
            .field("console", &self.console)
            .field("engine", &self.engine)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// Runs scripts from disk unit by unit. Task 12 gives it the gate, the classifier pool and the
/// confirmation port it needs.
#[derive(Debug, Default)]
pub struct ScriptRunner {
    _private: (),
}

impl ScriptRunner {
    pub fn new() -> Self {
        ScriptRunner::default()
    }

    /// Streams `source` to `console` per `req`.
    ///
    /// Task 1a stub (fail closed): pulls no chunk, sends nothing.
    pub async fn run(
        &self,
        console: &SessionHandle,
        source: Box<dyn ScriptChunks>,
        req: ScriptRunRequest,
    ) -> ScriptRunOutcome {
        let _ = (console, source, req);
        ScriptRunOutcome::Refused(RefuseReason::NotImplemented)
    }
}

/// [`ScriptRunner::run`] as a free function.
///
/// Task 1a stub (fail closed): pulls no chunk, sends nothing.
pub async fn run_file(
    runner: &ScriptRunner,
    console: &SessionHandle,
    source: Box<dyn ScriptChunks>,
    req: ScriptRunRequest,
) -> ScriptRunOutcome {
    runner.run(console, source, req).await
}
// ## m4b-t01a-seams
