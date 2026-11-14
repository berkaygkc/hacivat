//! The transactional model (UI-EDIT-01; Task 5).
//!
//! UI-EDIT-01's dispatch order: filters → buffer apply → selection (given or mapped) → fields
//! in order → version += 1 → observers → dirty ranges; a dispatch from an observer is queued.
//!
//! Working since Task 1b: `new`, `dispatch` (a change set whose old length is not the
//! document's is vetoed; then apply, selection, version, change log) and `snapshot`. Task 5
//! adds filters, fields, history and brackets; until then `undo`/`redo` find nothing.

use std::time::Duration;

use crate::buffer::TextBuffer;
use crate::change::ChangeDesc;
use crate::changelog::ChangeLog;
use crate::selection::SelectionSet;
use crate::transaction::{Transaction, UserEvent};

/// Why a transaction was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Veto(pub &'static str);

/// State that follows every commit (marks, tiers).
pub trait Field: 'static {
    fn on_commit(&mut self, tr: &Transaction, old: &TextBuffer, new: &TextBuffer, version: u64);
}

/// Rewrites or refuses a transaction before it applies (read-only, size guards).
pub trait TransactionFilter: 'static {
    fn filter(&self, m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto>;
}

/// An O(1) view of one version: Send + Sync, for the language service's worker.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub buffer: TextBuffer,
    pub selection: SelectionSet,
    pub version: u64,
}

/// What a dispatch committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    pub version: u64,
    pub desc: ChangeDesc,
    pub buffer_changed: bool,
}

/// The model's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelConfig {
    /// Undo groups kept (1,000).
    pub history_groups: u32,
    /// History bytes kept (64 MiB).
    pub history_bytes: u64,
    /// How long typing joins one undo group (500 ms).
    pub group_window: Duration,
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            history_groups: 1_000,
            history_bytes: 64 << 20,
            group_window: Duration::from_millis(500),
        }
    }
}

/// A field's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldId(pub u32);

/// An open history bracket (an IME composition, a command).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BracketId(pub u32);

/// The editor's document model, on the UI thread.
pub struct EditorModel {
    buffer: TextBuffer,
    selection: SelectionSet,
    version: u64,
    log: ChangeLog,
    config: ModelConfig,
    fields: Vec<Box<dyn Field>>,
    filters: Vec<Box<dyn TransactionFilter>>,
    next_bracket: u32,
}

impl std::fmt::Debug for EditorModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorModel")
            .field("buffer", &self.buffer)
            .field("selection", &self.selection)
            .field("version", &self.version)
            .field("config", &self.config)
            .field("fields", &self.fields.len())
            .field("filters", &self.filters.len())
            .finish_non_exhaustive()
    }
}

impl EditorModel {
    /// A model over `text` with a caret at the start, at version 0.
    pub fn new(text: &str, cfg: ModelConfig) -> Self {
        EditorModel {
            buffer: TextBuffer::from_str(text),
            selection: SelectionSet::cursor(0),
            version: 0,
            log: ChangeLog::default(),
            config: cfg,
            fields: Vec::new(),
            filters: Vec::new(),
            next_bracket: 0,
        }
    }

    /// Applies `tr` (the module docs).
    pub fn dispatch(&mut self, tr: Transaction) -> Result<Committed, Veto> {
        if tr.changes.len_old() != self.buffer.len_bytes() {
            return Err(Veto("the change set is for a text of another length"));
        }
        let rope = self.buffer.rope_mut();
        for (range, ins) in tr.changes.replacements_from_end() {
            let start = rope.byte_to_char(range.start);
            let end = rope.byte_to_char(range.end);
            rope.remove(start..end);
            rope.insert(start, &ins);
        }
        let desc = tr.changes.desc();
        self.selection = tr.selection.unwrap_or_else(|| self.selection.map(&desc));
        self.version += 1;
        self.log.record(self.version, desc.clone());
        Ok(Committed {
            version: self.version,
            desc,
            buffer_changed: !tr.changes.is_identity(),
        })
    }

    /// The current version, O(1).
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            buffer: self.buffer.clone(),
            selection: self.selection.clone(),
            version: self.version,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn changelog(&self) -> &ChangeLog {
        &self.log
    }

    /// The model's bounds.
    pub fn config(&self) -> &ModelConfig {
        &self.config
    }

    /// Undoes the newest group (Task 5). Task 1b: there is no history yet.
    pub fn undo(&mut self) -> Option<Committed> {
        None
    }

    /// Redoes the newest undone group (Task 5). Task 1b: there is no history yet.
    pub fn redo(&mut self) -> Option<Committed> {
        None
    }

    /// Closes the open undo group (Run, a focus change, a non-adjacent move; Task 5).
    pub fn seal_history(&mut self) {}

    /// Opens a bracket: its transactions form one undo group (Task 5).
    pub fn begin_bracket(&mut self, kind: UserEvent) -> BracketId {
        let _ = kind;
        self.next_bracket += 1;
        BracketId(self.next_bracket)
    }

    pub fn end_bracket(&mut self, b: BracketId) {
        let _ = b;
    }

    /// Adds a field (Task 5 runs fields on every commit).
    pub fn add_field(&mut self, f: Box<dyn Field>) -> FieldId {
        self.fields.push(f);
        FieldId(u32::try_from(self.fields.len() - 1).expect("fewer than 4 billion fields"))
    }

    /// Adds a filter (Task 5 runs filters before every apply).
    pub fn add_filter(&mut self, f: Box<dyn TransactionFilter>) {
        self.filters.push(f);
    }
}
