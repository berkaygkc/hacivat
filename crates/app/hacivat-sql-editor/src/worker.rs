//! The per-document worker on the GPUI background executor (R-M3-22; Task 24).
//!
//! The UI thread only puts the latest [`Job`] (the snapshot's version, an O(1) clone of its
//! rope, the console's SQL context) into a `tokio::sync::watch`. The worker loop takes the
//! newest job, builds a fresh `SqlDocument` from the rope's chunks (the one copy of the text is
//! made here, off the UI thread), computes the execution split and what highlighting and the
//! gutter need, and publishes an [`Analysis`] on a second `watch`. One analysis runs at a time;
//! jobs that arrive meanwhile collapse into the newest, so a burst of keystrokes costs one
//! analysis (latest wins).

use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use hacivat_text::TextBuffer;
use hacivat_ui_editor::{ExecPreview, HighlightClass, StatementSpan};
use sql_dialect::{ClientFlavor, DialectCtx};
use sql_split::{DocSnapshot, SqlDocument};
use tokio::sync::watch;

use crate::chunks::RopeChunks;
use crate::preview::{PreviewKey, preview_of};
use crate::{highlight, statements};

/// What to analyse: one editor version under one SQL context.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    pub version: u64,
    /// The service's context generation when the job was made (bumped by every
    /// `set_sql_context`), so an analysis under an older dialect is never served as current.
    pub generation: u64,
    pub buffer: TextBuffer,
    pub ctx: DialectCtx,
    pub flavor: ClientFlavor,
}

/// One version's analysis: the document snapshot (its execution split already computed) and
/// what the highlighter and the gutter read from it.
#[derive(Debug)]
pub(crate) struct Analysis {
    pub version: u64,
    pub generation: u64,
    pub doc: DocSnapshot,
    /// Client commands and `COPY … FROM STDIN` data (and its `\.`), sorted, not overlapping:
    /// painted as commands and data whatever the whole-text lexing says inside them.
    pub regions: Vec<(Range<usize>, HighlightClass)>,
    /// The tokens inside every running executable comment, sorted, not overlapping.
    pub exec_inner: Vec<(Range<usize>, HighlightClass)>,
    /// The editor-mode statements, `Exact` only when execution would send them as they are.
    pub statements: Vec<StatementSpan>,
}

/// Analyses `job` (on the worker; never on the UI thread).
pub(crate) fn analyse(job: &Job) -> Analysis {
    let doc =
        SqlDocument::from_chunks(&RopeChunks(job.buffer.clone()), job.ctx, job.flavor).snapshot();
    // Computed here, once, for every reader: the preview and Run's reuse never build it on the
    // UI thread.
    let _ = doc.execution_split();
    Analysis {
        version: job.version,
        generation: job.generation,
        regions: highlight::regions(&doc),
        exec_inner: highlight::exec_inner(&doc),
        statements: statements::spans(&doc),
        doc,
    }
}

/// Panics the workers contained, and a test hook that makes the next worker step panic.
#[derive(Debug, Default)]
pub(crate) struct Faults {
    pub panics: AtomicUsize,
    pub inject: AtomicBool,
}

/// Runs `f`, containing a panic (review M1): the GPUI background executor catches none, and an
/// unwinding lexer or splitter bug would take the app, and the console's unsaved text, down
/// with it. `None` after a panic, which is counted (never logged with text).
fn contained<R>(faults: &Faults, f: impl FnOnce() -> R) -> Option<R> {
    let out = catch_unwind(AssertUnwindSafe(|| {
        if faults.inject.swap(false, Ordering::Relaxed) {
            panic!("injected worker panic (test hook)");
        }
        f()
    }));
    if out.is_err() {
        faults.panics.fetch_add(1, Ordering::Relaxed);
    }
    out.ok()
}

/// The worker loop: analyses the newest job whenever one arrives, until either channel closes
/// (the document was closed). `count` counts analyses (tests). A job whose analysis panics
/// publishes nothing: that version has no preview and no `doc_snapshot` (fail closed), and
/// highlighting keeps the previous analysis, rebased.
pub(crate) async fn run(
    mut jobs: watch::Receiver<Option<Job>>,
    out: watch::Sender<Option<Arc<Analysis>>>,
    count: Arc<AtomicUsize>,
    faults: Arc<Faults>,
) {
    loop {
        let job = jobs.borrow_and_update().clone();
        if let Some(job) = job
            && let Some(analysis) = contained(&faults, || analyse(&job))
        {
            count.fetch_add(1, Ordering::Relaxed);
            if out.send(Some(Arc::new(analysis))).is_err() {
                return;
            }
        }
        if jobs.changed().await.is_err() {
            return;
        }
    }
}

/// A selection preview too long to resolve on the UI thread (review I1): the question, and the
/// analysis it is asked of.
#[derive(Debug, Clone)]
pub(crate) struct PreviewJob {
    pub key: PreviewKey,
    pub doc: DocSnapshot,
}

/// The preview loop: answers the newest preview job (latest wins) until either channel closes.
/// The answer is the connected one; the service draws it neutral when offline.
pub(crate) async fn previews(
    mut jobs: watch::Receiver<Option<PreviewJob>>,
    out: watch::Sender<Option<(PreviewKey, ExecPreview)>>,
    faults: Arc<Faults>,
) {
    loop {
        let job = jobs.borrow_and_update().clone();
        if let Some(job) = job {
            let k = &job.key;
            let answer = contained(&faults, || {
                preview_of(&job.doc, &k.at, k.action, true, k.remembered)
            });
            if let Some(answer) = answer
                && out.send(Some((job.key.clone(), answer))).is_err()
            {
                return;
            }
        }
        if jobs.changed().await.is_err() {
            return;
        }
    }
}
