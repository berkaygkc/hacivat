//! `SqlDocument` and its O(1) snapshots (Task 8). M3a rebuilds the document on every change
//! (R-M3-22: off the UI thread); incremental relex and resplit are M3b.
//!
//! A document lexes its text once (`lexed`, what highlighting reads) and runs the splitter's
//! structural pass once; the editor-mode split is built eagerly and the execution-mode split
//! the first time it is asked for, from the same pass, once per revision (every snapshot of a
//! revision shares it).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use sql_dialect::{ClientFlavor, DialectCtx};
use sql_lexer::{LexState, Lexed, TokenKind, lex};
use sql_text::edit::MisalignedEdit;
use sql_text::{TextChunks, TextEdit, TextRange, TextSize};

use crate::item::{
    AmbiguityReason, AmbiguousBoundary, ClientSegment, Confidence, CopyData, Item, Split,
    SplitMode, Stmt, StmtHead, overlaps,
};
use crate::key::StmtKey;
use crate::recover::shift_stmt;
use crate::splitter::{Analysis, SplitStart, analyze, carried_reason, lexer_for, size, split_from};

/// How far past an offset the splitter's second lexing is read to see whether the offset is
/// inside a token: longer than any opener (`/*M!`) or string prefix (`U&'`).
const RELEX_WINDOW: usize = 16;

/// Whether a selection edge inside a token of `kind` cuts it: every token but white space and
/// plain comments, whose text is trivia in every reading.
fn cuts(kind: TokenKind) -> bool {
    !matches!(
        kind,
        TokenKind::Whitespace
            | TokenKind::Newline
            | TokenKind::LineComment
            | TokenKind::BlockComment
    )
}

/// Why a cut token is uncertain: inside an executable comment or a hint the server reads
/// code; anything else is a construct cut short.
fn cut_reason(kind: TokenKind) -> AmbiguityReason {
    match kind {
        TokenKind::ExecutableComment | TokenKind::HintComment => {
            AmbiguityReason::AmbiguousCommentEnd
        }
        _ => AmbiguityReason::UnterminatedConstruct,
    }
}

fn is_comment(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::LineComment | TokenKind::BlockComment)
}

/// Makes every statement of `split` that `cut` overlaps `Heuristic`, with the boundary cut
/// to it.
fn mark_uncertain(split: &mut Split, cut: AmbiguousBoundary) {
    let mut marked = Vec::new();
    for item in &mut split.items {
        if let Item::Statement(st) = item
            && overlaps(cut.at, st.strict)
            && let Some(at) = cut.at.intersect(st.strict).filter(|r| !r.is_empty())
        {
            st.confidence = Confidence::Heuristic;
            marked.push(AmbiguousBoundary { at, ..cut });
        }
    }
    split.ambiguous.extend(marked);
    split
        .ambiguous
        .sort_by_key(|a| (a.at.start(), a.at.end(), a.reason as u8));
    split.ambiguous.dedup();
}

/// What a change did to a document's statements (editor mode), by content key: `changed`
/// keys are new, `removed` keys are gone, `moved` keys stayed but start elsewhere; `dirty` is
/// the range of the new text whose analysis may differ.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocDelta {
    pub changed: Vec<StmtKey>,
    pub moved: Vec<StmtKey>,
    pub removed: Vec<StmtKey>,
    /// The text range whose analysis changed.
    pub dirty: TextRange,
}

/// A SQL text with its lexing and its editor-mode split.
#[derive(Debug, Clone)]
pub struct SqlDocument {
    text: Arc<str>,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    lexed: Arc<Lexed>,
    analysis: Arc<Analysis>,
    editor: Arc<Split>,
    execution: Arc<OnceLock<Arc<Split>>>,
    revision: u64,
}

/// An immutable analysis of one document revision: Send + Sync, O(1) to clone. The
/// execution-mode split is computed once, from the same lexing, when first asked for.
#[derive(Debug, Clone)]
pub struct DocSnapshot {
    text: Arc<str>,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    lexed: Arc<Lexed>,
    analysis: Arc<Analysis>,
    editor: Arc<Split>,
    execution: Arc<OnceLock<Arc<Split>>>,
    revision: u64,
}

fn whole(text: &str) -> TextRange {
    TextRange::up_to(size(text.len()))
}

impl SqlDocument {
    pub fn new(text: String, ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        Self::build(Arc::from(text), ctx, flavor, 0)
    }

    fn build(text: Arc<str>, ctx: DialectCtx, flavor: ClientFlavor, revision: u64) -> Self {
        let lexed = lex(&text, lexer_for(&ctx), ctx.modes, &LexState::Initial);
        let analysis = analyze(&text, Some(&lexed), &ctx, flavor, &SplitStart::default());
        let editor = analysis.to_split(&text, SplitMode::Editor);
        SqlDocument {
            text,
            ctx,
            flavor,
            lexed: Arc::new(lexed),
            analysis: Arc::new(analysis),
            editor: Arc::new(editor),
            execution: Arc::new(OnceLock::new()),
            revision,
        }
    }

    /// A document over chunked text (a rope's chunks).
    pub fn from_chunks(text: &dyn TextChunks, ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        Self::new(text.chunks().collect(), ctx, flavor)
    }

    /// Applies `edits` in order (each against the text the previous one left) and rebuilds
    /// the analysis (the M3a body: a full rebuild).
    ///
    /// # Errors
    /// `MisalignedEdit` when an edit does not fit the text it meets (past the end, or splitting
    /// a character). The document is then unchanged, revision included: it no longer mirrors
    /// the editor's buffer, and the caller must rebuild it from that buffer before analysing
    /// again.
    pub fn apply(&mut self, edits: &[TextEdit]) -> Result<DocDelta, MisalignedEdit> {
        let mut text = String::from(&*self.text);
        for edit in edits {
            edit.apply(&mut text)?;
        }
        let old = std::mem::replace(
            self,
            Self::build(Arc::from(text), self.ctx, self.flavor, self.revision + 1),
        );
        Ok(delta(&old, self))
    }

    /// Changes the dialect context (a `SET sql_mode`, a reconnect) and rebuilds.
    pub fn set_context(&mut self, ctx: DialectCtx, flavor: ClientFlavor) -> DocDelta {
        let old = std::mem::replace(
            self,
            Self::build(self.text.clone(), ctx, flavor, self.revision + 1),
        );
        delta(&old, self)
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lexed(&self) -> &Lexed {
        &self.lexed
    }

    /// Bumped by every change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The editor-mode items.
    pub fn statements(&self) -> &[Item] {
        &self.editor.items
    }

    /// The item whose smart range contains `at` (the first, where two touch); O(log n).
    pub fn statement_at(&self, at: TextSize) -> Option<usize> {
        statement_at(&self.editor.items, at)
    }

    /// An O(1) snapshot of this revision.
    pub fn snapshot(&self) -> DocSnapshot {
        DocSnapshot {
            text: self.text.clone(),
            ctx: self.ctx,
            flavor: self.flavor,
            lexed: self.lexed.clone(),
            analysis: self.analysis.clone(),
            editor: self.editor.clone(),
            execution: self.execution.clone(),
            revision: self.revision,
        }
    }
}

fn statement_at(items: &[Item], at: TextSize) -> Option<usize> {
    let i = items.partition_point(|it| it.smart().end() < at);
    items
        .get(i)
        .filter(|it| it.smart().contains_inclusive(at))
        .map(|_| i)
}

/// The statement keys of `split`, with where each starts.
fn keyed(split: &Split) -> Vec<(StmtKey, TextSize)> {
    split
        .statements()
        .map(|s| (s.key, s.strict.start()))
        .collect()
}

/// The difference between two revisions' editor splits.
fn delta(old: &SqlDocument, new: &SqlDocument) -> DocDelta {
    let mut pool: HashMap<StmtKey, Vec<TextSize>> = HashMap::new();
    for (k, at) in keyed(&old.editor) {
        pool.entry(k).or_default().push(at);
    }
    for v in pool.values_mut() {
        v.reverse();
    }
    let (mut changed, mut moved) = (Vec::new(), Vec::new());
    let mut dirty: Option<TextRange> = None;
    for s in new.editor.statements() {
        match pool.get_mut(&s.key).and_then(Vec::pop) {
            Some(at) if at == s.strict.start() => {}
            Some(_) => moved.push(s.key),
            None => {
                changed.push(s.key);
                dirty = Some(dirty.map_or(s.smart, |d| d.cover(s.smart)));
            }
        }
    }
    let mut removed: Vec<StmtKey> = pool
        .into_iter()
        .flat_map(|(k, left)| std::iter::repeat_n(k, left.len()))
        .collect();
    removed.sort();
    // The edited bytes themselves: between the common prefix and suffix of the two texts.
    let (a, b) = (old.text.as_bytes(), new.text.as_bytes());
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    if a != b || old.ctx != new.ctx || old.flavor != new.flavor {
        let mut edited = TextRange::new(size(prefix), size(b.len() - suffix));
        if old.ctx != new.ctx || old.flavor != new.flavor {
            edited = whole(&new.text);
        }
        dirty = Some(dirty.map_or(edited, |d| d.cover(edited)));
    }
    DocDelta {
        changed,
        moved,
        removed,
        dirty: dirty.unwrap_or_else(|| TextRange::empty(size(prefix.min(b.len())))),
    }
}

impl DocSnapshot {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn lexed(&self) -> &Lexed {
        &self.lexed
    }

    pub fn ctx(&self) -> &DialectCtx {
        &self.ctx
    }

    pub fn flavor(&self) -> ClientFlavor {
        self.flavor
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The editor-mode split (heuristic items included).
    pub fn editor_split(&self) -> &Split {
        &self.editor
    }

    /// The execution-mode split, computed once from the same lexing.
    pub fn execution_split(&self) -> &Split {
        self.execution
            .get_or_init(|| Arc::new(self.analysis.to_split(&self.text, SplitMode::Execution)))
    }

    /// The editor item whose smart range contains `at`; O(log n).
    pub fn statement_at(&self, at: TextSize) -> Option<usize> {
        statement_at(&self.editor.items, at)
    }

    /// Where a split of the text from `at` starts: the lexer state at `at` (inside a string, a
    /// dollar quote, a comment or `COPY` data) and the client delimiter in force there.
    pub fn split_start_at(&self, at: TextSize) -> SplitStart {
        let at = self.floor(at);
        let delimiter = self.analysis.delimiter_at(at);
        if self.client_at(at).is_some() {
            return SplitStart {
                lex: LexState::Initial,
                delimiter,
            };
        }
        if self.in_copy_rows(at) {
            return SplitStart {
                lex: LexState::InCopyData,
                delimiter,
            };
        }
        let lp = lexer_for(&self.ctx);
        let lex = match self.lex_origin(at) {
            // Before any restart the document's own lexing is the splitter's.
            None => {
                let from = self
                    .lexed
                    .token_at(at)
                    .map_or(at, |i| self.lexed.range(i).start());
                state_after(
                    &self.text[usize::from(from)..usize::from(at)],
                    lp,
                    &self.ctx,
                )
            }
            Some((p, state)) => {
                let piece = &self.text[usize::from(p)..usize::from(at)];
                lex_state_after(piece, lp, &self.ctx, &state)
            }
        };
        SplitStart { lex, delimiter }
    }

    /// `at` inside the text, moved back to a character boundary.
    fn floor(&self, at: TextSize) -> TextSize {
        let mut i = usize::from(at).min(self.text.len());
        while !self.text.is_char_boundary(i) {
            i -= 1;
        }
        size(i)
    }

    /// Where the splitter's second lexing can be picked up at or before `at`, and in which
    /// state: the last restart, or a later checkpoint (at most `CHECKPOINT_BYTES` and one
    /// token back). `None` before the first restart.
    fn lex_origin(&self, at: TextSize) -> Option<(TextSize, LexState)> {
        let a = &self.analysis;
        let r = a.restarts.partition_point(|(p, _)| *p <= at);
        let (restart, state) = a.restarts.get(r.checked_sub(1)?)?;
        let c = a.checkpoints.partition_point(|p| *p <= at);
        match c.checked_sub(1).map(|i| a.checkpoints[i]) {
            Some(cp) if cp > *restart => Some((cp, LexState::Initial)),
            _ => Some((*restart, state.clone())),
        }
    }

    /// What the selection `range` cuts through, in text order: at most one boundary for its
    /// start and one for its end. Empty when both ends are clean.
    ///
    /// A unit is sent alone and the server reads it from a clean state, so a selection is
    /// only what it looks like when its ends fall between the document's tokens. The answer
    /// comes from the document's own tokens and `COPY` ranges, never from lexing the selected
    /// text by itself:
    /// - **start inside a token** (after its first byte): the rest of the token is not what
    ///   it reads as alone. The rest of a string, a quoted identifier, a dollar-quoted body
    ///   (a string prefix such as `E` or `U&` and the opening quote are part of the token) or
    ///   of a word, a number or an operator is `UnterminatedConstruct`, to the token's end.
    ///   Inside an executable comment or a hint (their openers `/*!`, `/*M!`, `/*+`
    ///   included) the server reads code where the document reads a comment, with no end both
    ///   agree on: `AmbiguousCommentEnd`, to the selection's end;
    /// - **start inside a `COPY … FROM STDIN` statement or its data** (after the statement's
    ///   first byte) with rows selected: the rows are data, never SQL. `UnterminatedConstruct`,
    ///   to the end of the data;
    /// - **end inside a token**: the token is cut short. Same reasons, over the selected part
    ///   of the token;
    /// - **start or end strictly inside an `AmbiguousCommentEnd` boundary** of the document
    ///   (after an executable comment's or hint's lexer end, before the end the server may
    ///   read): the server may still be inside the comment. `AmbiguousCommentEnd`.
    ///
    /// The `COPY` rule holds wherever the `COPY` statement is: other items may follow it on
    /// its line. Offsets past the text or inside a character are moved back first.
    ///
    /// White space and plain comments (`-- …`, `/* … */`) are not cut. The part of a comment
    /// at either end of the selection is trivia and is left out (the rest of a block comment
    /// the selection starts inside, the first part of a comment it ends inside), and text
    /// selected wholly inside one `--` comment is read as it is selected (a statement kept
    /// in a comment can be run). A selection that starts inside a `--` comment and runs past
    /// its end is cut at the comment's end (`UnterminatedConstruct`): read alone, a quote in
    /// the comment's text would pair with the quotes after it.
    ///
    /// After a client command, a client delimiter or `COPY` data the splitter lexes again, so
    /// its tokens can differ from [`DocSnapshot::lexed`]; there both readings are asked, and
    /// either one saying "inside" is enough.
    pub fn selection_cuts(&self, range: TextRange) -> Vec<AmbiguousBoundary> {
        let range = self.without_comment_parts(range);
        let (start, end) = self.edge_cuts(range);
        let mut out = Vec::new();
        if let Some((reason, cut, _)) = start {
            out.push(AmbiguousBoundary {
                at: TextRange::new(range.start(), cut),
                reason,
            });
        }
        if let Some(b) = end
            && !out.contains(&b)
        {
            out.push(b);
        }
        out
    }

    /// The cut at each end of `range` (already without its comment parts): for the start, why
    /// it is not a clean place to split from, where the cut-off construct ends and whether
    /// the statement goes on after it (`start_cut`); for the end, the boundary. Each end is
    /// looked at once.
    fn edge_cuts(
        &self,
        range: TextRange,
    ) -> (
        Option<(AmbiguityReason, TextSize, bool)>,
        Option<AmbiguousBoundary>,
    ) {
        if range.is_empty() {
            return (None, None);
        }
        let start = self.start_cut(range);
        let end = self
            .token_cut_by(range.end())
            .map(|(kind, token)| (cut_reason(kind), token.start()))
            .or_else(|| {
                self.ambiguous_comment_at(range.end())
                    .map(|a| (AmbiguityReason::AmbiguousCommentEnd, a.start()))
            })
            .map(|(reason, from)| AmbiguousBoundary {
                at: TextRange::new(from.max(range.start()), range.end()),
                reason,
            });
        (start, end)
    }

    /// The document's `AmbiguousCommentEnd` boundary that `at` is strictly inside: an
    /// executable comment or hint whose end the server may read later than the lexer does
    /// (`/*! , "*/ … " */`). Between the two ends the server may be reading the comment's
    /// body where the document shows code.
    fn ambiguous_comment_at(&self, at: TextSize) -> Option<TextRange> {
        [&*self.editor, self.execution_split()]
            .into_iter()
            .find_map(|split| {
                let upto = split.ambiguous.partition_point(|a| a.at.start() < at);
                split.ambiguous[..upto]
                    .iter()
                    .rev()
                    .find(|a| a.reason == AmbiguityReason::AmbiguousCommentEnd && at < a.at.end())
                    .map(|a| a.at)
            })
    }

    /// `range` without the part of a plain comment it holds at either end: the rest of a
    /// block comment it starts inside (opener included), and the first part of a comment it
    /// ends inside when that comment starts in it. Those parts are trivia in the document,
    /// and nothing of them is ever sent; read alone, a comment's first byte (`/`, `-`) would
    /// be an operator. A range wholly inside one `--` comment is kept: that is the selected
    /// text. Not where the splitter lexed again: there a comment of the document's lexing
    /// is not trusted, and the edge is a cut. Both ends are first moved back to a character
    /// boundary inside the text.
    pub fn without_comment_parts(&self, range: TextRange) -> TextRange {
        let (mut start, mut end) = (self.floor(range.start()), self.floor(range.end()));
        let inside = |at: TextSize| {
            self.lexed.token_at(at).and_then(|i| {
                let (kind, r) = (self.lexed.kind(i), self.lexed.range(i));
                (r.start() < at && at < r.end() && is_comment(kind) && !self.relexed_at(at))
                    .then_some((kind, r))
            })
        };
        if let Some((TokenKind::BlockComment, r)) = inside(start) {
            start = r.end().min(end);
        }
        if let Some((_, r)) = inside(end)
            && start <= r.start()
        {
            end = r.start();
        }
        TextRange::new(start, end.max(start))
    }

    /// Why the start of `range` is not a clean place to split from, where the cut-off
    /// construct ends (at most `range.end()`), and whether the statement goes on after it (a
    /// token's rest is part of the statement around it; `COPY` data ends its statement).
    fn start_cut(&self, range: TextRange) -> Option<(AmbiguityReason, TextSize, bool)> {
        let (at, end) = (range.start(), range.end());
        // `COPY` rows reached from inside their statement or from inside the data: any
        // `COPY` of the document whose statement starts before `at` and whose rows the range
        // overlaps, wherever its statement is (other items may follow it on its line).
        if let Some((owner, data)) = self.copy_overlapping(range)
            && owner < at
        {
            let data_end = data.end.map_or(data.rows.end(), |e| e.end());
            return Some((
                AmbiguityReason::UnterminatedConstruct,
                data_end.min(end),
                false,
            ));
        }
        // Inside a `--` comment and running past its end: read alone, the comment's text is
        // code, and a quote in it would pair with the quotes after the comment, out of step
        // with the document. (Wholly inside the comment it is just the selected text.)
        if let Some(i) = self.lexed.token_at(at)
            && self.lexed.kind(i) == TokenKind::LineComment
        {
            let r = self.lexed.range(i);
            if r.start() < at && at < r.end() && r.end() < end {
                return Some((AmbiguityReason::UnterminatedConstruct, r.end(), false));
            }
        }
        if let Some((kind, token)) = self.token_cut_by(at) {
            let reason = cut_reason(kind);
            // A comment's tail has no end both readings agree on; and a token of the
            // splitter's second lexing has no known end here.
            let whole = reason == AmbiguityReason::AmbiguousCommentEnd || self.relexed_at(at);
            let cut = if whole { end } else { token.end().min(end) };
            return Some((reason, cut, true));
        }
        // After an executable comment's or hint's lexer end but before the end the server
        // may read: the server may still be inside the comment.
        if self.ambiguous_comment_at(at).is_some() {
            return Some((AmbiguityReason::AmbiguousCommentEnd, end, false));
        }
        // The splitter's own state there (inside `COPY` data, or a construct its second
        // lexing sees).
        carried_reason(&self.split_start_at(at).lex).map(|reason| (reason, end, false))
    }

    /// The significant token that `at` is strictly inside (after its first byte, before its
    /// end), with its kind: in the document's lexing, or in the splitter's second lexing
    /// where there is one. `None` between tokens, and inside white space or a plain comment.
    fn token_cut_by(&self, at: TextSize) -> Option<(TokenKind, TextRange)> {
        // Where the splitter lexed again, a comment of the document's one lexing may be code
        // to the splitter (a quote in `COPY` data opens a "string" that swallows the opener):
        // there every token cuts but white space.
        let relexed = self.relexed_at(at);
        let doc = self.lexed.token_at(at).and_then(|i| {
            let (kind, r) = (self.lexed.kind(i), self.lexed.range(i));
            let cut = cuts(kind) || (relexed && is_comment(kind));
            (r.start() < at && at < r.end() && cut).then_some((kind, r))
        });
        if doc.is_some() {
            return doc;
        }
        // The splitter's second lexing, from its nearest restart or checkpoint to a little
        // past `at` (an opener or a string prefix is at most a few bytes).
        let (from, state) = self.lex_origin(at)?;
        if self.client_at(at).is_some() || self.in_copy_rows(at) {
            return None;
        }
        let mut upto = (usize::from(at) + RELEX_WINDOW).min(self.text.len());
        while !self.text.is_char_boundary(upto) {
            upto += 1;
        }
        let piece = &self.text[usize::from(from)..upto];
        let again = lex(piece, lexer_for(&self.ctx), self.ctx.modes, &state);
        let rel = at - from;
        let i = again.token_at(rel)?;
        let r = again.range(i);
        (r.start() < rel && rel < r.end() && cuts(again.kind(i))).then(|| (again.kind(i), r + from))
    }

    /// Whether the splitter lexed again before `at` (its tokens may differ from `lexed`).
    fn relexed_at(&self, at: TextSize) -> bool {
        self.analysis.restarts.first().is_some_and(|(p, _)| *p < at)
    }

    /// The selection `range` split on its own in `mode`, with document offsets (a selection
    /// run as separate statements).
    ///
    /// A unit is sent alone, and the server reads it from a clean state. So whatever the
    /// selection cuts through ([`DocSnapshot::selection_cuts`]) is never part of a certain
    /// statement:
    /// - when it starts inside a token, the token's rest and what follows it up to the
    ///   statement's end are one `Heuristic` statement, with a boundary over the token's rest
    ///   (`UnterminatedConstruct`; inside an executable comment or a hint
    ///   `AmbiguousCommentEnd`, and then the rest is the whole selection). Splitting goes on
    ///   after the token, from the document's own state there, so the split never runs out
    ///   of step with the document: the rest of a string is never read as code, nor code
    ///   after it as a string;
    /// - when it starts inside a `COPY` statement with rows selected, the selected part of
    ///   the statement and its data is one `Heuristic` statement, and splitting goes on after
    ///   the data;
    /// - when it ends inside a token, every statement holding part of that token is
    ///   `Heuristic`;
    /// - a selection that starts inside a client command's line yields the rest of that line
    ///   as an unsupported client segment, never as SQL;
    /// - when the selection's own split reads a client command the document does not have at
    ///   that place, or misses one it has (a selection that starts in the middle of a line
    ///   can make the line's rest look like a command line), every statement of it is
    ///   `Heuristic` (`ClientCommand`).
    ///
    /// Otherwise the selection is split from the lexer state and client delimiter at its
    /// start (inside a plain comment, the comment's rest is trivia).
    pub fn split_selection(&self, range: TextRange, mode: SplitMode) -> Split {
        match self.split_selection_or(range, mode, false) {
            Ok(split) | Err((split, _)) => split,
        }
    }

    /// [`DocSnapshot::split_selection`], or, when the selection cuts anything
    /// ([`DocSnapshot::selection_cuts`]), those cuts alone: the caller that refuses on a cut
    /// gets both answers from one look at each end.
    pub(crate) fn split_selection_checked(
        &self,
        range: TextRange,
        mode: SplitMode,
    ) -> Result<Split, Vec<AmbiguousBoundary>> {
        self.split_selection_or(range, mode, true)
            .map_err(|(_, cuts)| cuts)
    }

    fn split_selection_or(
        &self,
        range: TextRange,
        mode: SplitMode,
        refuse: bool,
    ) -> Result<Split, (Split, Vec<AmbiguousBoundary>)> {
        let range = TextRange::new(
            range.start().min(size(self.text.len())),
            range.end().min(size(self.text.len())),
        );
        if self
            .text
            .get(usize::from(range.start())..usize::from(range.end()))
            .is_none()
        {
            return Ok(Split::default());
        }
        // Inside a client command: the rest of the command is its argument.
        let command = self.client_at(range.start()).map(|c| ClientSegment {
            range: TextRange::new(range.start(), c.range.end().min(range.end())),
            kind: c.kind,
            supported: false,
        });
        let rest = self.without_comment_parts(TextRange::new(
            command.map_or(range.start(), |c| c.range.end()),
            range.end(),
        ));
        let range = TextRange::new(range.start(), rest.end());
        let (start_cut, end_cut) = self.edge_cuts(rest);
        if refuse && (start_cut.is_some() || end_cut.is_some()) {
            let mut cuts: Vec<AmbiguousBoundary> = start_cut
                .map(|(reason, cut, _)| AmbiguousBoundary {
                    at: TextRange::new(rest.start(), cut),
                    reason,
                })
                .into_iter()
                .collect();
            cuts.extend(end_cut.filter(|b| !cuts.contains(b)));
            return Err((Split::default(), cuts));
        }
        // The cut-off rest of what the selection starts inside, and where splitting goes on.
        let mut tail = start_cut.map(|(reason, cut, _)| AmbiguousBoundary {
            at: TextRange::new(rest.start(), cut),
            reason,
        });
        let joins = start_cut.is_some_and(|(_, _, joins)| joins);
        let mut by = tail.map_or(rest.start(), |t| t.at.end());
        let mut start = self.split_start_at(by);
        if tail.is_some() && by < rest.end() && carried_reason(&start.lex).is_some() {
            // No clean state after the cut either: the whole selection is the rest.
            by = rest.end();
            start = self.split_start_at(by);
        }
        if let Some(t) = &mut tail {
            t.at = TextRange::new(rest.start(), by);
        }
        let piece = &self.text[usize::from(by)..usize::from(range.end())];
        let s = split_from(piece, &self.ctx, self.flavor, mode, &start);
        let mut out = Split {
            items: s
                .items
                .into_iter()
                .map(|it| match it {
                    Item::Empty { range } => Item::Empty { range: range + by },
                    Item::Statement(st) => Item::Statement(shift_stmt(st, by)),
                })
                .collect(),
            ambiguous: s
                .ambiguous
                .into_iter()
                .map(|a| AmbiguousBoundary { at: a.at + by, ..a })
                .collect(),
            client: s
                .client
                .into_iter()
                .map(|c| ClientSegment {
                    range: c.range + by,
                    ..c
                })
                .collect(),
            diagnostics: s
                .diagnostics
                .into_iter()
                .map(|d| crate::item::Diagnostic {
                    range: d.range + by,
                    ..d
                })
                .collect(),
        };
        // The selection's own split must find the client commands the document has there,
        // and no other. A selection that starts in the middle of a line makes that line's
        // rest look like a line of its own (`x .mode csv`, `a DELIMITER //`), and a command
        // read there that the document does not have swallows text by another rule than the
        // document's: from then on the split is out of step with it. Nothing after such a
        // difference is certain.
        let doc_commands = self
            .analysis
            .client
            .iter()
            .map(|c| c.range.start())
            .filter(|s| by <= *s && *s < range.end());
        // Likewise `COPY` data: a statement cut at its start can read as a `COPY … FROM STDIN`
        // the document does not have (`TEMP «COPY t FROM STDIN;…»`), and its "data" would
        // swallow lines the document reads as SQL or comments.
        // And the other way: rows the document reads as `COPY` data must be data here too.
        // A `COPY` whose statement is not in the selection (other items follow it on its
        // line, and the selection starts after them) would have its rows read as SQL.
        let own: Vec<TextSize> = out
            .statements()
            .filter_map(|st| st.copy_data)
            .map(|d| d.rows.start())
            .collect();
        let copy_in_step = own.iter().all(|at| self.copy_rows_start_at(*at))
            && self
                .copies_overlapping(TextRange::new(by, range.end()))
                .all(|(_, d)| own.contains(&d.rows.start()));
        if !copy_in_step || !doc_commands.eq(out.client.iter().map(|c| c.range.start())) {
            mark_uncertain(
                &mut out,
                AmbiguousBoundary {
                    at: TextRange::new(by, range.end()),
                    reason: AmbiguityReason::ClientCommand,
                },
            );
        }
        if let Some(t) = tail {
            self.prepend_tail(&mut out, t, joins, start.delimiter.as_deref());
        }
        // The end cut (the start's is the tail).
        if let Some(cut) = end_cut {
            mark_uncertain(&mut out, cut);
        }
        if let Some(c) = command {
            out.client.insert(0, c);
        }
        Ok(out)
    }

    /// Puts the cut-off rest `tail` at the front of `out` as a `Heuristic` statement (its
    /// text without outer white space) with its boundary. When the statement goes on after
    /// the rest (`joins`) and `out` begins with a statement, that statement is the one: it
    /// is made to start at the rest.
    fn prepend_tail(
        &self,
        out: &mut Split,
        tail: AmbiguousBoundary,
        joins: bool,
        delimiter: Option<&str>,
    ) {
        let text = &self.text[tail.at];
        let lead = text.len() - text.trim_start().len();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let strict = TextRange::at(tail.at.start() + size(lead), size(trimmed.len()));
        out.ambiguous.insert(
            0,
            AmbiguousBoundary {
                at: strict,
                reason: tail.reason,
            },
        );
        if joins && let Some(Item::Statement(first)) = out.items.first_mut() {
            first.strict = TextRange::new(strict.start(), first.strict.end());
            first.smart = TextRange::new(strict.start(), first.smart.end().max(first.strict.end()));
            first.leading_comments = None;
            first.confidence = Confidence::Heuristic;
            first.head = StmtHead::default();
            first.key = StmtKey::of(&self.ctx, delimiter, &self.text[first.strict]);
            return;
        }
        out.items.insert(
            0,
            Item::Statement(Stmt {
                strict,
                smart: strict,
                terminator: None,
                leading_comments: None,
                confidence: Confidence::Heuristic,
                head: StmtHead::default(),
                key: StmtKey::of(&self.ctx, delimiter, trimmed),
                copy_data: None,
            }),
        );
    }

    fn client_at(&self, at: TextSize) -> Option<&ClientSegment> {
        // Segments are in text order and do not overlap.
        let client = &self.analysis.client;
        let i = client.partition_point(|c| c.range.start() < at);
        client.get(i.checked_sub(1)?).filter(|c| at < c.range.end())
    }

    /// The document's `COPY` data with rows that overlap `range`, in text order, each with
    /// where its statement starts.
    fn copies_overlapping(&self, range: TextRange) -> impl Iterator<Item = (TextSize, CopyData)> {
        let copies = &self.analysis.copies;
        let from = copies.partition_point(|(_, d)| d.rows.end() <= range.start());
        copies[from..]
            .iter()
            .take_while(move |(_, d)| d.rows.start() < range.end())
            .filter(|(_, d)| !d.rows.is_empty())
            .copied()
    }

    /// The first `COPY` data of the document whose rows overlap `range`, with where its
    /// statement starts.
    pub(crate) fn copy_overlapping(&self, range: TextRange) -> Option<(TextSize, CopyData)> {
        self.copies_overlapping(range).next()
    }

    /// Whether a `COPY … FROM STDIN` statement of the document has its data start at `at`.
    fn copy_rows_start_at(&self, at: TextSize) -> bool {
        let copies = &self.analysis.copies;
        let i = copies.partition_point(|(_, d)| d.rows.start() < at);
        copies.get(i).is_some_and(|(_, d)| d.rows.start() == at)
    }

    fn in_copy_rows(&self, at: TextSize) -> bool {
        let copies = &self.analysis.copies;
        let i = copies.partition_point(|(_, d)| d.rows.end() <= at);
        copies.get(i).is_some_and(|(_, d)| d.rows.start() <= at)
    }
}

/// The lexer state after lexing `piece` from `Initial`.
fn state_after(piece: &str, lp: &'static sql_dialect::LexDialect, ctx: &DialectCtx) -> LexState {
    lex_state_after(piece, lp, ctx, &LexState::Initial)
}

fn lex_state_after(
    piece: &str,
    lp: &'static sql_dialect::LexDialect,
    ctx: &DialectCtx,
    from: &LexState,
) -> LexState {
    lex(piece, lp, ctx.modes, from).end_state().clone()
}
