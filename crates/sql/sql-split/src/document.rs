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
use sql_lexer::{LexState, Lexed, lex};
use sql_text::edit::MisalignedEdit;
use sql_text::{TextChunks, TextEdit, TextRange, TextSize};

use crate::item::{AmbiguousBoundary, ClientSegment, Item, Split, SplitMode};
use crate::key::StmtKey;
use crate::recover::shift_stmt;
use crate::splitter::{Analysis, SplitStart, analyze, lexer_for, size, split_from};

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
        let at = at.min(size(self.text.len()));
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
        // Lex from the last place lexing restarted (or the document's own lexing before any).
        let restart = self
            .analysis
            .restarts
            .iter()
            .rev()
            .find(|(p, _)| *p <= at)
            .cloned();
        let lp = lexer_for(&self.ctx);
        let lex = match restart {
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

    /// The selection `range` split on its own in `mode`, from the state at its start, with
    /// document offsets (a selection run as separate statements).
    pub fn split_selection(&self, range: TextRange, mode: SplitMode) -> Split {
        let range = TextRange::new(
            range.start().min(size(self.text.len())),
            range.end().min(size(self.text.len())),
        );
        let start = self.split_start_at(range.start());
        let Some(piece) = self
            .text
            .get(usize::from(range.start())..usize::from(range.end()))
        else {
            return Split::default();
        };
        let s = split_from(piece, &self.ctx, self.flavor, mode, &start);
        let by = range.start();
        Split {
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
        }
    }

    fn client_at(&self, at: TextSize) -> Option<&ClientSegment> {
        self.analysis
            .client
            .iter()
            .find(|c| c.range.start() < at && at < c.range.end())
    }

    fn in_copy_rows(&self, at: TextSize) -> bool {
        self.analysis.items.iter().any(|it| match it {
            crate::splitter::RawItem::Stmt(s) => s
                .copy_data
                .is_some_and(|d| d.rows.start() <= at && at < d.rows.end()),
            crate::splitter::RawItem::Empty(_) => false,
        })
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
