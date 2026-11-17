//! The language service seam (UI-EDIT-11, -21): what the editor asks of the SQL adapter, on the
//! UI thread. The service never lexes on the calling thread (R-M3-22) and never previews a
//! range execution would not run (R-M3-8).

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use hacivat_text::display::{Decoration, RangeSet};
use hacivat_text::{ChangeDesc, ChangeLog, FoldRange, Snapshot, TokenOracle};
use hacivat_ui_core::gpui_surface::App;
use smallvec::{SmallVec, smallvec};

use crate::config::{DocContext, DocId};

// ## m3c-t34-editor
/// A dialect's comment tokens (`editor.toggle_comment`): the line comment and the block
/// comment's opening and closing tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommentTokens {
    /// The line comment (`None`: the dialect has none, and lines are wrapped in the block
    /// comment instead; ## m3c-t34-editor fix round 1, M12).
    pub line: Option<&'static str>,
    pub block: (&'static str, &'static str),
    /// The line comment needs white space (or the line's end) after it to be one: MySQL's
    /// `-- ` (`--1` is an expression there; fix round 1, M9).
    pub line_needs_space: bool,
}

impl CommentTokens {
    /// Standard SQL's, which every supported dialect reads: `--` and `/* */` (MySQL needs a
    /// space after `--`, and the command always writes one).
    pub const SQL: CommentTokens = CommentTokens {
        line: Some("--"),
        block: ("/*", "*/"),
        line_needs_space: false,
    };

    /// MySQL's and MariaDB's: `-- ` with its space (`#` also comments there, but `--` is what
    /// the command writes).
    pub const MYSQL: CommentTokens = CommentTokens {
        line_needs_space: true,
        ..CommentTokens::SQL
    };
}

/// Whether a statement's boundaries are certain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Confidence {
    Exact,
    Heuristic,
}

/// A statement as the editor draws it: smart and strict ranges (bytes), the header's offset
/// (the first non-comment token, where the gutter mark sits).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatementSpan {
    pub smart: Range<usize>,
    pub strict: Range<usize>,
    pub header: usize,
    pub confidence: Confidence,
}

/// What the Run preview band shows (the verdict; the band's own state is `preview::BandState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewState {
    Units,
    AskPreselected,
    Nothing,
    /// A boundary is uncertain: Run would send nothing (R-M3-24).
    Uncertain,
    NoConnection,
}

/// The ranges Run would execute and the verdict.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecPreview {
    pub ranges: SmallVec<[Range<usize>; 2]>,
    pub state: PreviewState,
    // ## m3a-t33-exec-ui (named exception, T28 ruling 2): the "N choices" hint.
    /// With `AskPreselected`, the candidates Run's chooser offers; 0 otherwise.
    pub choices: u32,
    // ## m3c-t34-editor (named exception; the T24/T29 carry, NOTES surface 9): "as one · 2
    // statements".
    /// Run sends the selection as one unit holding this many statements; 0 otherwise.
    pub as_one: u32,
}

/// Which Run (Run, Run 2, Run 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecAction {
    Execute1,
    Execute2,
    Execute3,
}

/// Where Run was asked: carets or selections, bytes at `ExecRequest::snapshot.version`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExecAt {
    Carets(SmallVec<[usize; 1]>),
    Selections(SmallVec<[Range<usize>; 1]>),
}

/// A Run request (`EditorEvent::ExecuteRequested`): the document, the action, where, and the
/// exact snapshot it was asked on.
#[derive(Debug, Clone)]
pub struct ExecRequest {
    pub doc: DocId,
    pub action: ExecAction,
    pub at: ExecAt,
    pub snapshot: Snapshot,
}

/// Called when a fresh analysis lands, so the editor repaints.
pub type InvalidateFn = Rc<dyn Fn(&mut App)>;

/// A decoration source's ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u16);

/// Decorations for a visible range, rebased to `snap.version` through `log`.
pub trait DecorationSource: 'static {
    fn id(&self) -> SourceId;
    fn priority(&self) -> i16;
    fn decorations(
        &self,
        snap: &Snapshot,
        log: &ChangeLog,
        range: Range<usize>,
    ) -> Arc<RangeSet<Decoration>>;
}

/// The SQL language service, as an `Rc<dyn LanguageService>` on the UI thread.
pub trait LanguageService: 'static {
    fn open(&self, doc: DocId, snap: &Snapshot, ctx: DocContext, cx: &mut App);
    fn set_context(&self, doc: DocId, ctx: DocContext, cx: &mut App);
    /// A committed change; never lexes on this thread (R-M3-22).
    fn change(&self, doc: DocId, snap: &Snapshot, changes: &ChangeDesc, cx: &mut App);
    fn close(&self, doc: DocId);
    /// Registers the repaint callback for a fresh analysis of `doc`.
    ///
    /// **Reentrancy (review round 2):** never call `notify` synchronously from `open`,
    /// `set_context` or `change`, nor from inside any other call of this trait. The editor calls
    /// them while it is itself being updated, and its `notify` updates the editor again
    /// (`preview::on_update`), which GPUI refuses. Call it from a later foreground task (the
    /// worker's result landing on the UI thread), as Task 24's design does.
    fn on_update(&self, doc: DocId, notify: InvalidateFn);
    fn statements(&self, doc: DocId, version: u64, range: Range<usize>) -> Arc<[StatementSpan]>;
    /// The preview only (never what runs); `None` when there is no analysis at `version`.
    fn exec_preview(
        &self,
        doc: DocId,
        version: u64,
        at: &ExecAt,
        action: ExecAction,
    ) -> Option<ExecPreview>;
    fn token_oracle(&self, doc: DocId) -> Rc<dyn TokenOracle>;
    fn decoration_sources(&self, doc: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]>;
    // ## m3b-t01b-seams
    /// The ranges of `doc` that can fold at `version` (Task 32; the app's adapter fills it).
    /// Default: none, so nothing folds.
    fn folding_ranges(&self, _doc: DocId, _version: u64) -> Arc<[FoldRange]> {
        Arc::from([])
    }
    // ## m3c-t34-editor
    /// The dialect's comment tokens (Task 34's toggle comment). Default: [`CommentTokens::SQL`].
    fn comment_tokens(&self, _doc: DocId) -> CommentTokens {
        CommentTokens::SQL
    }
}

/// A token oracle that knows no words (the class fallback applies).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTokens;

impl TokenOracle for NoTokens {
    fn words_on_line(&self, line: Range<usize>) -> Option<SmallVec<[Range<usize>; 16]>> {
        let _ = line;
        None
    }
}

/// The service that knows nothing (fail closed, F31): no statements, no decorations, no
/// words, and a preview that is always `Uncertain` with no range, so nothing reads as runnable.
#[derive(Debug, Clone, Copy, Default)]
pub struct FailClosedService;

impl LanguageService for FailClosedService {
    fn open(&self, doc: DocId, snap: &Snapshot, ctx: DocContext, cx: &mut App) {
        let _ = (doc, snap, ctx, cx);
    }

    fn set_context(&self, doc: DocId, ctx: DocContext, cx: &mut App) {
        let _ = (doc, ctx, cx);
    }

    fn change(&self, doc: DocId, snap: &Snapshot, changes: &ChangeDesc, cx: &mut App) {
        let _ = (doc, snap, changes, cx);
    }

    fn close(&self, doc: DocId) {
        let _ = doc;
    }

    fn on_update(&self, doc: DocId, notify: InvalidateFn) {
        let _ = (doc, notify);
    }

    fn statements(&self, doc: DocId, version: u64, range: Range<usize>) -> Arc<[StatementSpan]> {
        let _ = (doc, version, range);
        Arc::from(Vec::new())
    }

    fn exec_preview(
        &self,
        doc: DocId,
        version: u64,
        at: &ExecAt,
        action: ExecAction,
    ) -> Option<ExecPreview> {
        let _ = (doc, version, at, action);
        Some(ExecPreview {
            ranges: smallvec![],
            state: PreviewState::Uncertain,
            choices: 0,
            as_one: 0,
        })
    }

    fn token_oracle(&self, doc: DocId) -> Rc<dyn TokenOracle> {
        let _ = doc;
        Rc::new(NoTokens)
    }

    fn decoration_sources(&self, doc: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
        let _ = doc;
        SmallVec::new()
    }
}
