//! The transactional model (UI-EDIT-01).
//!
//! [`EditorModel::dispatch`] runs, in this order:
//! 1. the checks: a change set for a text of another length is vetoed; edits are snapped
//!    outward to character boundaries (an insertion point inside a character moves before it);
//! 2. the filters, in the order added, each seeing the change as it will apply (each may
//!    rewrite the transaction or veto it; a rewrite is checked and snapped again). An undo or
//!    redo a filter rewrites is vetoed instead, and its group stays;
//! 3. the buffer apply;
//! 4. the selection: the given one or the old one mapped, clamped to the text, on character
//!    boundaries, and never between the CR and LF of a CRLF (a caret there moves past the LF);
//! 5. the fields in order: history first (record, rebase, or the undo/redo bookkeeping), then
//!    each added [`Field`], told the version the commit makes;
//! 6. version += 1 and a change-log entry, when the text changed (a selection-only commit keeps
//!    the version: versions count text changes, so a language-service snapshot stays current
//!    while the caret moves);
//! 7. the observers, in the order added;
//! 8. the dirty range ([`Committed::dirty`]).
//!
//! A transaction an observer pushes onto its [`ObserverQueue`] is dispatched after the current
//! one, through the same steps, never re-entrantly (an observer gets no `&mut EditorModel`); it
//! is rebased over anything committed after the version it was built at, and dropped when it
//! overlaps that. One dispatch applies at most [`MAX_QUEUED`] queued transactions. The
//! returned [`Committed`] then covers everything the call committed.
//!
//! The model is a plain mutable struct on the UI thread; fields are plain mutable structs
//! too. A [`Snapshot`] is the rope clone plus the selection and the version: O(1), Send + Sync.
//! Nothing in a dispatch is O(document).

use std::collections::VecDeque;
use std::ops::Range;
use std::time::{Duration, Instant};

use crate::buffer::{Bias, TextBuffer};
use crate::change::{ChangeDesc, ChangeSet};
use crate::changelog::ChangeLog;
use crate::history::History;
use crate::selection::{SelectionRange, SelectionSet};
use crate::transaction::{Annotations, Origin, Transaction, UserEvent};

/// Why a transaction was refused: a Fluent message id (never English).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Veto(pub &'static str);

/// State that follows every commit (marks, tiers).
pub trait Field: 'static {
    /// After `tr` changed `old` into `new` (the same text for a selection-only commit);
    /// `version` is the version the commit makes.
    fn on_commit(&mut self, tr: &Transaction, old: &TextBuffer, new: &TextBuffer, version: u64);
}

/// Rewrites or refuses a transaction before it applies (read-only, size guards).
pub trait TransactionFilter: 'static {
    fn filter(&self, m: &EditorModel, tr: Transaction) -> Result<Transaction, Veto>;
}

/// Hears of every commit, after the fields and the version (the language service, IME
/// notifications). It may queue transactions; they apply after this dispatch.
pub trait TransactionObserver: 'static {
    /// `tr` committed as `c`, making `buffer`.
    fn observe(
        &mut self,
        tr: &Transaction,
        c: &Committed,
        buffer: &TextBuffer,
        queue: &mut ObserverQueue,
    );
}

/// The transactions observers queue during a dispatch.
#[derive(Debug, Default)]
pub struct ObserverQueue {
    /// Each with the version it was built against.
    pending: VecDeque<(Transaction, u64)>,
    version: u64,
}

impl ObserverQueue {
    /// Queues `tr`, built against the text the observer was shown; it is dispatched after the
    /// current dispatch, rebased over whatever committed in between.
    pub fn dispatch(&mut self, tr: Transaction) {
        self.pending.push_back((tr, self.version));
    }
}

/// The most queued transactions one dispatch applies; the rest are dropped (a runaway
/// observer cannot hang the UI thread).
pub const MAX_QUEUED: usize = 256;

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
    /// The version after the commit (unchanged by a selection-only commit).
    pub version: u64,
    /// The change from the text before the dispatch to the text after it.
    pub desc: ChangeDesc,
    pub buffer_changed: bool,
    /// The bytes of the new text the commit touched (one range over every edit), for the view
    /// to re-lay out; `None` when the text did not change.
    pub dirty: Option<Range<usize>>,
}

/// The model's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelConfig {
    /// Undo groups kept (1,000).
    pub history_groups: u32,
    /// History bytes kept (64 MiB): every byte the groups delete or insert.
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

/// What a commit does to the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Edit,
    Undo,
    Redo,
}

/// The editor's document model, on the UI thread.
pub struct EditorModel {
    buffer: TextBuffer,
    selection: SelectionSet,
    version: u64,
    log: ChangeLog,
    config: ModelConfig,
    history: History,
    fields: Vec<Box<dyn Field>>,
    filters: Vec<Box<dyn TransactionFilter>>,
    observers: Vec<Box<dyn TransactionObserver>>,
    queue: ObserverQueue,
    next_bracket: u32,
}

impl std::fmt::Debug for EditorModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorModel")
            .field("buffer", &self.buffer)
            .field("selection", &self.selection)
            .field("version", &self.version)
            .field("config", &self.config)
            .field("undo_depth", &self.history.undo_depth())
            .field("redo_depth", &self.history.redo_depth())
            .field("fields", &self.fields.len())
            .field("filters", &self.filters.len())
            .field("observers", &self.observers.len())
            .finish_non_exhaustive()
    }
}

/// `cs` with every edit on character boundaries of `buf`: a deletion widens to whole
/// characters, an insertion point inside a character moves before it. Vetoed when two edits
/// overlap once snapped.
fn snap_to_chars(buf: &TextBuffer, cs: ChangeSet) -> Result<ChangeSet, Veto> {
    if cs
        .edits()
        .all(|(r, _)| buf.is_char_boundary(r.start) && buf.is_char_boundary(r.end))
    {
        return Ok(cs);
    }
    let edits: Vec<_> = cs
        .edits()
        .map(|(r, ins)| {
            let start = buf.snap_to_char(r.start, Bias::Before);
            let end = if r.is_empty() {
                start
            } else {
                buf.snap_to_char(r.end, Bias::After)
            };
            (start..end, ins.clone())
        })
        .collect();
    ChangeSet::from_edits(cs.len_old(), edits).map_err(|_| Veto("editor-veto-overlapping-edits"))
}

/// `sel` clamped to `buf`, on its character boundaries, and out of every CRLF: an end between
/// the CR and the LF moves past the LF, so the next character typed cannot split the break.
fn fit_selection(sel: SelectionSet, buf: &TextBuffer) -> SelectionSet {
    // `snap` with `After` keeps a char boundary unless it is inside a CRLF.
    let ok = |b: usize| buf.is_char_boundary(b) && buf.snap(b, Bias::After) == b;
    if sel.ranges().iter().all(|r| ok(r.anchor) && ok(r.head)) {
        return sel;
    }
    let main = sel
        .ranges()
        .iter()
        .position(|r| std::ptr::eq(r, sel.main()))
        .unwrap_or(0);
    let fit = |b: usize| buf.snap(buf.snap_to_char(b, Bias::Before), Bias::After);
    SelectionSet::normalized(
        sel.ranges().iter().map(|r| SelectionRange {
            anchor: fit(r.anchor),
            head: fit(r.head),
            ..*r
        }),
        main,
    )
}

impl EditorModel {
    /// A model over `text` with a caret at the start, at version 0.
    pub fn new(text: &str, cfg: ModelConfig) -> Self {
        EditorModel {
            buffer: TextBuffer::from_str(text),
            selection: SelectionSet::cursor(0),
            version: 0,
            log: ChangeLog::default(),
            history: History::new(&cfg),
            config: cfg,
            fields: Vec::new(),
            filters: Vec::new(),
            observers: Vec::new(),
            queue: ObserverQueue::default(),
            next_bracket: 0,
        }
    }

    /// Applies `tr`, then whatever the observers queue (the module docs).
    pub fn dispatch(&mut self, tr: Transaction) -> Result<Committed, Veto> {
        self.run(tr, Action::Edit)
    }

    fn run(&mut self, tr: Transaction, action: Action) -> Result<Committed, Veto> {
        let (first, changes) = match self.commit(tr, action) {
            Ok(done) => done,
            Err(v) => {
                self.queue.pending.clear();
                return Err(v);
            }
        };
        if self.queue.pending.is_empty() {
            return Ok(first);
        }
        // Each committed text change with the version it made, for rebasing queued ones.
        let mut applied: Vec<(u64, ChangeSet)> = Vec::new();
        if first.buffer_changed {
            applied.push((first.version, changes));
        }
        let mut count = 0;
        while let Some((mut tr, built_at)) = self.queue.pending.pop_front() {
            count += 1;
            if count > MAX_QUEUED {
                self.queue.pending.clear();
                break;
            }
            let mut fits = true;
            for (_, cs) in applied.iter().filter(|(v, _)| *v > built_at) {
                // Built for a text of another length (a buggy observer): dropped, as the
                // length check would.
                if tr.changes.len_old() != cs.len_old() {
                    fits = false;
                    break;
                }
                let Some((mine, theirs)) = tr.changes.transform(cs) else {
                    fits = false;
                    break;
                };
                tr.changes = mine;
                tr.selection = tr.selection.map(|s| s.map(&theirs.desc()));
            }
            if !fits {
                continue;
            }
            if let Ok((c, cs)) = self.commit(tr, Action::Edit)
                && c.buffer_changed
            {
                applied.push((c.version, cs));
            }
        }
        let Some(((_, first_cs), rest)) = applied.split_first() else {
            return Ok(Committed {
                version: self.version,
                ..first
            });
        };
        let all = rest
            .iter()
            .fold(first_cs.clone(), |acc, (_, cs)| acc.compose(cs));
        let desc = all.desc();
        Ok(Committed {
            version: self.version,
            dirty: desc.changed_range(),
            desc,
            buffer_changed: true,
        })
    }

    /// One transaction through steps 1–7; the change set as applied.
    fn commit(&mut self, tr: Transaction, action: Action) -> Result<(Committed, ChangeSet), Veto> {
        // Filters judge the change as it will apply, so it is checked and snapped first.
        let mut tr = self.checked(tr)?;
        if !self.filters.is_empty() {
            for f in &self.filters {
                tr = f.filter(self, tr)?;
            }
            tr = self.checked(tr)?;
        }
        // An undo or redo applies its group's change exactly, or not at all (fail closed: a
        // rewritten one would lose the group).
        let stored = match action {
            Action::Undo => self.history.top_undo().map(|g| &g.changes),
            Action::Redo => self.history.top_redo().map(|g| &g.changes),
            Action::Edit => None,
        };
        if stored.is_some_and(|c| *c != tr.changes) {
            return Err(Veto("editor-veto-history-rewritten"));
        }
        let old = self.buffer.clone();
        tr.changes.apply(&mut self.buffer);
        let desc = tr.changes.desc();
        let changed = !tr.changes.is_identity();
        let after = fit_selection(
            match &tr.selection {
                Some(s) => s.clone(),
                None => self.selection.map(&desc),
            },
            &self.buffer,
        );
        let before = std::mem::replace(&mut self.selection, after);
        match action {
            Action::Undo => self.history.finish_undo(&old),
            Action::Redo => self.history.finish_redo(&old),
            Action::Edit if !changed => self.history.selection_moved(&self.selection),
            Action::Edit if tr.ann.add_to_history => {
                self.history
                    .record(&tr, &old, &self.buffer, &before, &self.selection);
            }
            Action::Edit => self.history.rebase(&tr.changes),
        }
        let version = if changed {
            self.version + 1
        } else {
            self.version
        };
        for f in &mut self.fields {
            f.on_commit(&tr, &old, &self.buffer, version);
        }
        if changed {
            self.version = version;
            self.log.record(version, desc.clone());
        }
        let committed = Committed {
            version,
            dirty: desc.changed_range(),
            desc,
            buffer_changed: changed,
        };
        self.queue.version = version;
        for o in &mut self.observers {
            o.observe(&tr, &committed, &self.buffer, &mut self.queue);
        }
        Ok((committed, tr.changes))
    }

    /// `tr` with a change set for the current text, its edits on character boundaries.
    fn checked(&self, mut tr: Transaction) -> Result<Transaction, Veto> {
        if tr.changes.len_old() != self.buffer.len_bytes() {
            return Err(Veto("editor-veto-stale-change"));
        }
        tr.changes = snap_to_chars(&self.buffer, tr.changes)?;
        Ok(tr)
    }

    /// The current version, O(1).
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            buffer: self.buffer.clone(),
            selection: self.selection.clone(),
            version: self.version,
        }
    }

    /// The text (for filters and commands; a reader on another thread takes a snapshot).
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    pub fn selection(&self) -> &SelectionSet {
        &self.selection
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

    /// Undoes the newest group, restoring its selection; `None` when there is none or a filter
    /// refuses or rewrites it (the group stays). Never joins what comes next.
    pub fn undo(&mut self) -> Option<Committed> {
        let g = self.history.top_undo()?;
        self.apply_history(g.changes.clone(), g.restore_sel.clone(), Action::Undo)
    }

    /// Redoes the newest undone group; `None` when there is none or a filter refuses or
    /// rewrites it (the group stays).
    pub fn redo(&mut self) -> Option<Committed> {
        let g = self.history.top_redo()?;
        self.apply_history(g.changes.clone(), g.restore_sel.clone(), Action::Redo)
    }

    fn apply_history(
        &mut self,
        changes: ChangeSet,
        selection: SelectionSet,
        action: Action,
    ) -> Option<Committed> {
        if changes.len_old() != self.buffer.len_bytes() {
            // The stored change no longer fits the text: never apply a guess.
            self.history.clear();
            return None;
        }
        let user_event = if action == Action::Undo {
            UserEvent::Undo
        } else {
            UserEvent::Redo
        };
        let tr = Transaction {
            changes,
            selection: Some(selection),
            ann: Annotations {
                user_event,
                add_to_history: false,
                time: Instant::now(),
                origin: Origin::User,
            },
        };
        self.run(tr, action).ok()
    }

    /// Undo groups held.
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }

    /// Undone groups that can be redone.
    pub fn redo_depth(&self) -> usize {
        self.history.redo_depth()
    }

    /// Closes the open undo group: a focus change, Run, a command flagged `seals_history`.
    pub fn seal_history(&mut self) {
        self.history.seal();
    }

    /// Opens a bracket: every history transaction until the outermost bracket ends forms one
    /// undo group of `kind` (brackets nest).
    pub fn begin_bracket(&mut self, kind: UserEvent) -> BracketId {
        self.next_bracket += 1;
        self.history.begin_bracket(self.next_bracket, kind);
        BracketId(self.next_bracket)
    }

    /// Ends bracket `b` (and any opened inside it); an unknown or ended bracket does nothing.
    pub fn end_bracket(&mut self, b: BracketId) {
        self.history.end_bracket(b.0);
    }

    /// Adds a field; fields run in the order added, after history.
    pub fn add_field(&mut self, f: Box<dyn Field>) -> FieldId {
        self.fields.push(f);
        FieldId(u32::try_from(self.fields.len() - 1).expect("fewer than 4 billion fields"))
    }

    /// Adds a filter; filters run in the order added, before every apply (undo and redo too).
    pub fn add_filter(&mut self, f: Box<dyn TransactionFilter>) {
        self.filters.push(f);
    }

    /// Adds an observer; observers run in the order added, after every commit.
    pub fn add_observer(&mut self, o: Box<dyn TransactionObserver>) {
        self.observers.push(o);
    }
}
