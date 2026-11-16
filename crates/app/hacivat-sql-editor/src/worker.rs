//! The per-document worker on the GPUI background executor (R-M3-22; Task 24).
//!
//! The UI thread only puts the latest [`Job`] (the snapshot's version, an O(1) clone of its
//! rope, the console's SQL context) into a `tokio::sync::watch`. The worker loop takes the
//! newest job, brings the `SqlDocument` it kept from the previous job up to the job's text with
//! `SqlDocument::apply` (one edit: the bytes between the two texts' common prefix and suffix;
//! M3b Task 5, R-M3-65) or builds a fresh one under a new SQL context (the text is copied here,
//! off the UI thread), computes the execution split and what highlighting and the gutter need,
//! and publishes an [`Analysis`] on a second `watch`. One analysis runs at a time;
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
use sql_text::{TextEdit, TextRange, TextSize};
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

// ## m3b-t05-incremental (named lines): the worker keeps its document between jobs and brings
// it up to each job's text with `SqlDocument::apply` (R-M3-65) instead of rebuilding it; the
// analysis stays off the UI thread (R-M3-22).
/// The document the worker keeps between jobs, and the context generation it was built under.
pub(crate) struct Kept {
    doc: SqlDocument,
    generation: u64,
}

/// Analyses `job` (on the worker; never on the UI thread) from the document kept from the
/// previous job (relexed and resplit incrementally when the context is the same), and returns
/// the document to keep.
pub(crate) fn analyse_from(kept: Option<Kept>, job: &Job) -> (Analysis, Kept) {
    let doc = document(kept, job);
    let snap = doc.snapshot();
    // Computed here, once, for every reader: the preview and Run's reuse never build it on the
    // UI thread.
    let _ = snap.execution_split();
    let analysis = Analysis {
        version: job.version,
        generation: job.generation,
        regions: highlight::regions(&snap),
        exec_inner: highlight::exec_inner(&snap),
        statements: statements::spans(&snap),
        doc: snap,
    };
    (
        analysis,
        Kept {
            doc,
            generation: job.generation,
        },
    )
}

/// `job`'s document: the kept one with the change to the job's text applied, or a fresh one
/// (no document kept, another SQL context, or a change it cannot apply).
fn document(kept: Option<Kept>, job: &Job) -> SqlDocument {
    if let Some(Kept {
        mut doc,
        generation,
    }) = kept
        && generation == job.generation
        && *doc.snapshot().ctx() == job.ctx
        && doc.snapshot().flavor() == job.flavor
    {
        let applied = match change_between(doc.text(), &job.buffer) {
            None => true,
            Some(edit) => doc.apply(std::slice::from_ref(&edit)).is_ok(),
        };
        if applied {
            return doc;
        }
    }
    SqlDocument::from_chunks(&RopeChunks(job.buffer.clone()), job.ctx, job.flavor)
}

/// The one edit that turns `old` into `new`'s text: the bytes between their common prefix and
/// suffix (moved to character boundaries); `None` when they are equal. Compares the texts once,
/// chunk by chunk, without copying the rope.
pub(crate) fn change_between(old: &str, new: &TextBuffer) -> Option<TextEdit> {
    let chunks: Vec<&str> = new.chunks_in(0..new.len_bytes()).collect();
    let (ob, new_len) = (old.as_bytes(), new.len_bytes());
    let mut p = 0;
    for c in &chunks {
        let cb = c.as_bytes();
        let n = ob[p..].iter().zip(cb).take_while(|(x, y)| x == y).count();
        p += n;
        if n < cb.len() {
            break;
        }
    }
    let p = p.min(ob.len()).min(new_len);
    if p == ob.len() && p == new_len {
        return None;
    }
    let max = (ob.len() - p).min(new_len - p);
    let mut s = 0;
    for c in chunks.iter().rev() {
        let cb = c.as_bytes();
        let n = ob[..ob.len() - s]
            .iter()
            .rev()
            .zip(cb.iter().rev())
            .take_while(|(x, y)| x == y)
            .count()
            .min(max - s);
        s += n;
        if n < cb.len() || s == max {
            break;
        }
    }
    // Equal bytes before `p` and from the suffix on: a character boundary in the old text is
    // one in the new text there too.
    let mut p = p;
    while !old.is_char_boundary(p) {
        p -= 1;
    }
    while !old.is_char_boundary(ob.len() - s) {
        s -= 1;
    }
    let delete = TextRange::new(size(p), size(ob.len() - s));
    Some(TextEdit {
        delete,
        insert: new.slice(p..new_len - s).into_owned(),
    })
}

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
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
    // ## m3b-t05-incremental: the document kept between jobs (dropped after a panic: the next
    // job builds a fresh one).
    let mut kept: Option<Kept> = None;
    loop {
        let job = jobs.borrow_and_update().clone();
        if let Some(job) = job {
            let previous = kept.take();
            if let Some((analysis, doc)) = contained(&faults, || analyse_from(previous, &job)) {
                kept = Some(doc);
                count.fetch_add(1, Ordering::Relaxed);
                if out.send(Some(Arc::new(analysis))).is_err() {
                    return;
                }
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
