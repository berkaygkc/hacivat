//! The Run preview (UI-EDIT-21; R-M3-8, R-M3-24; Task 24): what Run would send, drawn before
//! it is pressed, from the analysis at exactly the editor's version.
//!
//! **The rule.** The preview asks `exec_range::resolve` the question Run asks, in execution
//! mode, with Run's policy for the action and the console's remembered chooser kind:
//! - Run runs units → `Units` with exactly those ranges;
//! - Run opens the chooser → `AskPreselected` with the ranges of the candidate it opens on;
//! - nothing at the caret → `Nothing`, no range;
//! - Run refuses (an uncertain boundary, an unsupported client command, a script spanning
//!   one), or would hand M3a a unit it does not send (`COPY … FROM STDIN`, a verbatim
//!   selection) → `Uncertain`. Only then is editor mode asked, for the ranges to draw neutral:
//!   its units or preselected candidate, the uncertain boundaries, or the command.
//!
//! So a range is drawn as runnable only when it is the range Run sends, by construction, and
//! the common case costs one resolve. Task 13 makes editor mode answer the same wherever Run
//! runs or asks (`preview_truth.rs` checks it through this adapter).
//! - Disconnected → `NoConnection` with the same ranges (drawn neutral; Run connects first).
//!
//! **Where it runs.** A caret's preview resolves on the UI thread (no lexing: p95 0.45 ms on
//! 50,000 lines). A selection's lexes and splits the selected text, so selections longer than
//! [`SYNC_SELECTION_LIMIT`] in total are answered by the document's preview worker: the service
//! answers `None` (no band) until the worker's answer for exactly that version, context
//! generation, place, action and remembered kind lands, then serves it from a cache and
//! notifies the editor (review I1).
//!
//! What this cannot know: the PostgreSQL parser oracle (R-M3-24), which Task 23 may use to run
//! an `Ambiguous` region. The preview never calls it (a blocking parse is not UI-thread work),
//! so such a region previews `Uncertain` even when Run would resolve it.

use std::ops::Range;

use hacivat_ui_editor::{ExecAction, ExecAt, ExecPreview, PreviewState};
use smallvec::SmallVec;
use sql_split::exec_range::{
    CandidateKind, CaretOrSelection, ExecPolicy, RangeResolution, TextRange, TextSize, UnitKind,
    UnitSpan, resolve,
};
// ## m3b-t24-exec-ui (named exception)
use sql_split::exec_range::{InsidePolicy, SelectionPolicy};
use sql_split::{DocSnapshot, SplitMode};

type Ranges = SmallVec<[Range<usize>; 2]>;

/// The longest total selection the preview resolves on the UI thread (review I1). Resolving a
/// selection lexes and splits the selected text (about 20 ms per MB in release: 64 KiB took
/// 1.3 ms), so a longer one is answered by the document's worker. 32 KiB keeps the UI-thread
/// work under the 1 ms rule (§5.4), not only under the 2 ms resolve budget.
pub const SYNC_SELECTION_LIMIT: usize = 32 * 1024;

/// Whether the preview at `at` is resolved off the UI thread.
pub(crate) fn off_thread(at: &ExecAt) -> bool {
    match at {
        ExecAt::Carets(_) => false,
        ExecAt::Selections(s) => {
            s.iter()
                .map(|r| r.end.abs_diff(r.start))
                .fold(0usize, usize::saturating_add)
                > SYNC_SELECTION_LIMIT
        }
    }
}

/// What a worker-computed preview answers: the analysis (version and context generation) and
/// the question (place, action, remembered chooser kind). An answer is served only for an
/// equal key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreviewKey {
    pub version: u64,
    pub generation: u64,
    pub at: ExecAt,
    pub action: ExecAction,
    pub remembered: Option<CandidateKind>,
    // ## m3b-t24-exec-ui (named exception)
    pub policy: RunPolicy,
}

/// Run's policy for `action` (the same table as `hacivat_exec::policy_for`): Run is D-011's
/// default (ask), Run 2 the largest statement, Run 3 the whole script.
pub(crate) fn policy_for(action: ExecAction) -> ExecPolicy {
    match action {
        ExecAction::Execute1 => ExecPolicy::default(),
        ExecAction::Execute2 => ExecPolicy::largest(),
        ExecAction::Execute3 => ExecPolicy::whole_script(),
    }
}

// ## m3b-t24-exec-ui (named exception: CHOOSE-06 and R-M3-60)
/// What a caret inside a statement runs with Run (CHOOSE-06's "Customize…": the console's
/// choice, kept until it closes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InsideChoice {
    /// D-011: offer the candidates (one runs directly).
    #[default]
    Ask,
    Smallest,
    Largest,
    Whole,
}

/// A console's run policy (CHOOSE-06): what a caret inside a statement runs with Run, and how
/// a selection runs with any Run. The default is M3a's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RunPolicy {
    pub inside: InsideChoice,
    /// A selection runs as one unit (R-M3-60), else as separate statements.
    pub selection_as_one: bool,
}

/// The one policy table Run and its preview share: Run 2 and Run 3 keep their own carets
/// (largest, whole script); Run takes the console's inside choice; every Run takes the
/// console's selection choice. With the default `custom` this is [`policy_for`].
pub fn run_policy(action: ExecAction, custom: &RunPolicy) -> ExecPolicy {
    let mut p = policy_for(action);
    if action == ExecAction::Execute1 {
        p.inside = match custom.inside {
            InsideChoice::Ask => InsidePolicy::Ask,
            InsideChoice::Smallest => InsidePolicy::SmallestStatement,
            InsideChoice::Largest => InsidePolicy::LargestStatement,
            InsideChoice::Whole => InsidePolicy::WholeScript,
        };
    }
    if custom.selection_as_one {
        p.selection = SelectionPolicy::AsOne;
    }
    p
}

fn size(n: usize) -> TextSize {
    TextSize::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The editor's carets or selections as `exec_range`'s points (Run converts with this too).
pub fn points(at: &ExecAt) -> Vec<CaretOrSelection> {
    match at {
        ExecAt::Carets(c) => c
            .iter()
            .map(|&p| CaretOrSelection::Caret(size(p)))
            .collect(),
        ExecAt::Selections(s) => s
            .iter()
            .map(|r| {
                let (a, b) = (r.start.min(r.end), r.start.max(r.end));
                CaretOrSelection::Selection(TextRange::new(size(a), size(b)))
            })
            .collect(),
    }
}

fn to_range(r: TextRange) -> Range<usize> {
    usize::from(r.start())..usize::from(r.end())
}

fn ranges(units: &[UnitSpan]) -> Ranges {
    units.iter().map(|u| to_range(u.range)).collect()
}

/// Units M3a sends as they are: not `COPY … FROM STDIN` (its data is not sent; Task 23 logs
/// `exec-copy-unsupported`), not a verbatim selection (M3b).
// ## m3a-f1-followups (named exception, follow-up F1-a; it replaces T33's text-based
// `copy_stream`): nor any other `COPY` that streams (`COPY … TO STDOUT`, a selection of
// `COPY t FROM STDIN;` without its rows). Which unit is such a `COPY` is the splitter's
// answer (`UnitKind::CopyStream`), the one Run refuses by (`hacivat_exec::units`), so the
// preview and Run read one analysis and never disagree on a `COPY`.
// ## m3b-t24-exec-ui (named exception: R-M3-60): `sent(units)` became `sent_as(units, as_one)`.
/// Under a policy whose selection runs as one, a verbatim selection (the one unit
/// `SelectionPolicy::AsOne` resolves to) is then sent as one unit, exactly as Run sends it
/// (`hacivat_exec::units`: a `Verbatim` unit runs as one). `COPY` streams never are.
fn sent_as(units: &[UnitSpan], as_one: bool) -> bool {
    units.iter().all(|u| match u.kind {
        UnitKind::CopyIn { .. } | UnitKind::CopyStream => false,
        UnitKind::Verbatim => as_one,
        UnitKind::Statement | UnitKind::Subquery | UnitKind::Batch => true,
    })
}

/// What to draw for a resolution Run refuses.
fn drawn(r: &RangeResolution) -> Ranges {
    match r {
        RangeResolution::Units(u) => ranges(u),
        RangeResolution::Ask {
            candidates,
            preselect,
        } => candidates
            .get(*preselect)
            .map(|c| ranges(&c.units))
            .unwrap_or_default(),
        RangeResolution::Ambiguous(b) => {
            let mut out: Ranges = b.iter().map(|a| to_range(a.at)).collect();
            out.sort_by_key(|r| (r.start, r.end));
            out.dedup();
            out
        }
        RangeResolution::UnsupportedClientCommand { at } => [to_range(*at)].into_iter().collect(),
        RangeResolution::NothingToRun => Ranges::new(),
    }
}

/// Every point is a non-empty selection (an empty one resolves as a caret).
fn only_selections(points: &[CaretOrSelection]) -> bool {
    points
        .iter()
        .all(|p| matches!(p, CaretOrSelection::Selection(r) if !r.is_empty()))
}

/// The preview of Run `action` at `at` over `doc` (module docs). Pure; the caller passes an
/// analysis at exactly the editor's version.
pub fn preview_of(
    doc: &DocSnapshot,
    at: &ExecAt,
    action: ExecAction,
    connected: bool,
    remembered: Option<CandidateKind>,
) -> ExecPreview {
    preview_with(
        doc,
        at,
        action,
        connected,
        remembered,
        &RunPolicy::default(),
    )
}

// ## m3b-t24-exec-ui (named exception: CHOOSE-06, R-M3-60)
/// [`preview_of`] under the console's run policy `custom` ([`run_policy`], the table Run uses):
/// with a selection run as one, the verbatim selection previews as one runnable unit.
pub fn preview_with(
    doc: &DocSnapshot,
    at: &ExecAt,
    action: ExecAction,
    connected: bool,
    remembered: Option<CandidateKind>,
    custom: &RunPolicy,
) -> ExecPreview {
    let points = points(at);
    let policy = run_policy(action, custom);
    let as_one = policy.selection == SelectionPolicy::AsOne;
    let sent = |units: &[UnitSpan]| sent_as(units, as_one);
    let run = resolve(
        doc,
        &points,
        &policy,
        SplitMode::Execution,
        None,
        remembered,
    );
    let (state, ranges) = match &run {
        RangeResolution::Units(u) if sent(u) => (PreviewState::Units, ranges(u)),
        RangeResolution::Ask {
            candidates,
            preselect,
        } if candidates.get(*preselect).is_some_and(|c| sent(&c.units)) => (
            PreviewState::AskPreselected,
            ranges(&candidates[*preselect].units),
        ),
        RangeResolution::NothingToRun => (PreviewState::Nothing, Ranges::new()),
        // Run would hand over a unit M3a does not send: drawn as the units, neutral.
        RangeResolution::Units(_) | RangeResolution::Ask { .. } => {
            (PreviewState::Uncertain, drawn(&run))
        }
        // Non-empty selections resolve the same in both modes (`exec_range`: "a selection has
        // one answer for both modes"), so they are not lexed a second time.
        RangeResolution::Ambiguous(_) | RangeResolution::UnsupportedClientCommand { .. }
            if only_selections(&points) =>
        {
            (PreviewState::Uncertain, drawn(&run))
        }
        RangeResolution::Ambiguous(_) | RangeResolution::UnsupportedClientCommand { .. } => {
            let editor = resolve(doc, &points, &policy, SplitMode::Editor, None, remembered);
            (PreviewState::Uncertain, drawn(&editor))
        }
    };
    // ## m3a-t33-exec-ui (named exception): how many candidates Run's chooser offers, the
    // ones it would not leave out (the same `sent` rule as `hacivat_exec`'s chooser).
    let choices = match (&run, state) {
        (RangeResolution::Ask { candidates, .. }, PreviewState::AskPreselected) if connected => {
            let n = candidates.iter().filter(|c| sent(&c.units)).count();
            u32::try_from(n).unwrap_or(u32::MAX)
        }
        _ => 0,
    };
    ExecPreview {
        ranges,
        state: if connected {
            state
        } else {
            PreviewState::NoConnection
        },
        choices,
    }
}
