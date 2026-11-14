//! `SqlDocument` and its O(1) snapshots (Task 8). M3a rebuilds the document on every change
//! (R-M3-22: off the UI thread); incremental relex and resplit are M3b.

use std::sync::{Arc, OnceLock};

use sql_dialect::{ClientFlavor, DialectCtx, DialectId, LexModes, lex_profile};
use sql_lexer::{LexState, Lexed, lex};
use sql_text::{TextChunks, TextEdit, TextRange, TextSize};

use crate::item::{Item, Split, SplitMode};
use crate::key::StmtKey;
use crate::splitter::split;

/// What a change did to a document's statements.
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
    text: String,
    ctx: DialectCtx,
    flavor: ClientFlavor,
    lexed: Arc<Lexed>,
    editor: Arc<Split>,
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
    editor: Arc<Split>,
    execution: Arc<OnceLock<Arc<Split>>>,
    revision: u64,
}

/// `text` lexed for `ctx`; a dialect without a lexer profile lexes with the generic one.
fn lex_for(text: &str, ctx: &DialectCtx) -> Lexed {
    let profile = lex_profile(ctx).or_else(|_| {
        lex_profile(&DialectCtx {
            id: DialectId::Generic,
            version: None,
            modes: LexModes::default(),
        })
    });
    match profile {
        Ok(profile) => lex(text, profile, ctx.modes, &LexState::Initial),
        Err(_) => unreachable!("the generic dialect always has a lexer profile"),
    }
}

fn whole(text: &str) -> TextRange {
    TextRange::up_to(TextSize::from(
        u32::try_from(text.len()).expect("a SQL text is under 4 GiB"),
    ))
}

impl SqlDocument {
    pub fn new(text: String, ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        let lexed = lex_for(&text, &ctx);
        let editor = split(&text, &lexed, &ctx, flavor, SplitMode::Editor);
        SqlDocument {
            text,
            ctx,
            flavor,
            lexed: Arc::new(lexed),
            editor: Arc::new(editor),
            revision: 0,
        }
    }

    /// A document over chunked text (a rope's chunks).
    pub fn from_chunks(text: &dyn TextChunks, ctx: DialectCtx, flavor: ClientFlavor) -> Self {
        Self::new(text.chunks().collect(), ctx, flavor)
    }

    /// Applies `edits` in order (each against the text the previous one left) and rebuilds
    /// the analysis (the M3a body: a full rebuild).
    pub fn apply(&mut self, edits: &[TextEdit]) -> DocDelta {
        for edit in edits {
            let range = usize::from(edit.delete.start())..usize::from(edit.delete.end());
            self.text.replace_range(range, &edit.insert);
        }
        self.rebuild()
    }

    /// Changes the dialect context (a `SET sql_mode`, a reconnect) and rebuilds.
    pub fn set_context(&mut self, ctx: DialectCtx, flavor: ClientFlavor) -> DocDelta {
        self.ctx = ctx;
        self.flavor = flavor;
        self.rebuild()
    }

    fn rebuild(&mut self) -> DocDelta {
        let rebuilt = SqlDocument::new(std::mem::take(&mut self.text), self.ctx, self.flavor);
        let revision = self.revision + 1;
        *self = SqlDocument {
            revision,
            ..rebuilt
        };
        DocDelta {
            changed: Vec::new(),
            moved: Vec::new(),
            removed: Vec::new(),
            dirty: whole(&self.text),
        }
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

    /// The item whose smart range contains `at`.
    pub fn statement_at(&self, at: TextSize) -> Option<usize> {
        self.editor.items.iter().position(|item| match item {
            Item::Statement(s) => s.smart.contains_inclusive(at),
            Item::Empty { range } => range.contains_inclusive(at),
        })
    }

    /// An O(1) snapshot of this revision.
    pub fn snapshot(&self) -> DocSnapshot {
        DocSnapshot {
            text: Arc::from(self.text.as_str()),
            ctx: self.ctx,
            flavor: self.flavor,
            lexed: self.lexed.clone(),
            editor: self.editor.clone(),
            execution: Arc::new(OnceLock::new()),
            revision: self.revision,
        }
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
        self.execution.get_or_init(|| {
            Arc::new(split(
                &self.text,
                &self.lexed,
                &self.ctx,
                self.flavor,
                SplitMode::Execution,
            ))
        })
    }
}
