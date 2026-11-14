//! Grouped linear history (UI-EDIT-04), the model's first field: linear `done` and `undone`
//! stacks of groups, each group the change set that undoes (or redoes) it plus the selections
//! on either side.
//!
//! A history transaction joins the open top group only when all of these hold:
//! - nothing sealed the group: [`EditorModel::seal_history`](crate::EditorModel::seal_history)
//!   (focus change, Run, a command flagged `seals_history`), a selection commit that moves a
//!   caret away from where the group's last change ended, an undo or redo, a non-history edit;
//! - it is within the 500 ms window of the group's last change;
//! - it continues the group's region at every cursor, by family: typing appends where the
//!   group's last insertion ended; a Backspace deletes back from there, inside what the group
//!   typed (so a typo fixed while typing joins, and the group can become the identity);
//!   delete-backward and delete-forward runs continue their own run; the families never mix
//!   otherwise;
//! - the word rule: once the group ends with whitespace or `;`, only more whitespace or `;`
//!   joins it, so undo takes back one word with its trailing spaces.
//!
//! Paste, drop, compose (outside a bracket), commands, undo and redo never join, and nothing
//! joins them. While a bracket is open (an IME composition, a command), every history
//! transaction joins the bracket's group; brackets nest and the group seals when the outermost
//! one ends. A group whose change composes to the identity is dropped. At most 1,000 groups and
//! 64 MiB of edited bytes (deleted plus inserted) are kept, oldest evicted first; the newest
//! group always stays, and a group over 16 MiB evicts every older one (UI-EDIT-04 step 8). An
//! undo or redo applies its group's change exactly (the model vetoes one a filter rewrites), so
//! the group is never lost.
//!
//! A non-history transaction (`add_to_history: false`) rebases every stored group over its
//! change ([`ChangeSet::transform`]), so undo still restores the right bytes; a group it
//! overlaps, and every group older than that, is dropped instead (fail closed: undo never
//! applies a guess).
//!
//! Nothing here is O(document): joining is O(sections), an undo O(changes · log n), a rebase
//! O(groups × sections).

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use smallvec::SmallVec;

use crate::buffer::TextBuffer;
use crate::change::ChangeSet;
use crate::model::ModelConfig;
use crate::selection::SelectionSet;
use crate::transaction::{Transaction, UserEvent};

/// A group editing more bytes than this (a huge replace-all) is kept, but evicts every older
/// group (UI-EDIT-04 step 8).
pub(crate) const LARGE_GROUP_BYTES: u64 = 16 << 20;

/// One undo (or redo) step.
#[derive(Debug, Clone)]
pub(crate) struct Group {
    /// Applied to the current side's text, it makes the other side's.
    pub(crate) changes: ChangeSet,
    /// The selection applying `changes` sets.
    pub(crate) restore_sel: SelectionSet,
    /// The selection on the current side (where `changes` applies).
    current_sel: SelectionSet,
    kind: UserEvent,
    t_last: Instant,
    /// Where each cursor's last change ended, in the current text (adjacency).
    ends: SmallVec<[usize; 1]>,
    /// Where each cursor's typed run starts, in the current text (a Backspace inside it joins).
    starts: SmallVec<[usize; 1]>,
    /// The group's text ends with whitespace or `;` (the word rule).
    trailing_break: bool,
    weight: u64,
}

/// The model's history state.
#[derive(Debug)]
pub(crate) struct History {
    done: VecDeque<Group>,
    undone: Vec<Group>,
    /// The top group may take more changes.
    open: bool,
    /// The open brackets, innermost last.
    brackets: SmallVec<[(u32, UserEvent); 2]>,
    /// The top group is the open bracket's.
    bracket_group: bool,
    bytes: u64,
    max_groups: usize,
    max_bytes: u64,
    window: Duration,
}

/// Whitespace or `;`: the end of a word for the word rule.
fn is_break(c: char) -> bool {
    c.is_whitespace() || c == ';'
}

/// A change's edits: the old range, the inserted text, and where the edit starts in the new
/// text.
type Edit<'a> = (Range<usize>, &'a Arc<str>, usize);

fn edits_of(c: &ChangeSet) -> SmallVec<[Edit<'_>; 1]> {
    let mut delta: isize = 0;
    c.edits()
        .map(|(r, ins)| {
            let new_start = r.start.saturating_add_signed(delta);
            delta += ins.len() as isize - r.len() as isize;
            (r, ins, new_start)
        })
        .collect()
}

impl History {
    pub(crate) fn new(cfg: &ModelConfig) -> Self {
        History {
            done: VecDeque::new(),
            undone: Vec::new(),
            open: false,
            brackets: SmallVec::new(),
            bracket_group: false,
            bytes: 0,
            max_groups: usize::try_from(cfg.history_groups).unwrap_or(usize::MAX),
            max_bytes: cfg.history_bytes,
            window: cfg.group_window,
        }
    }

    pub(crate) fn undo_depth(&self) -> usize {
        self.done.len()
    }

    pub(crate) fn redo_depth(&self) -> usize {
        self.undone.len()
    }

    /// The group an undo applies.
    pub(crate) fn top_undo(&self) -> Option<&Group> {
        self.done.back()
    }

    /// The group a redo applies.
    pub(crate) fn top_redo(&self) -> Option<&Group> {
        self.undone.last()
    }

    // ## m3a-t25-editor-commands (review round 1, named exception): the Edit menu's labels.
    /// The kind of the group an undo applies.
    pub(crate) fn top_undo_kind(&self) -> Option<UserEvent> {
        self.done.back().map(|g| g.kind)
    }

    /// The kind of the group a redo applies.
    pub(crate) fn top_redo_kind(&self) -> Option<UserEvent> {
        self.undone.last().map(|g| g.kind)
    }

    /// Closes the open group (a bracket's too: its later transactions start a new one).
    pub(crate) fn seal(&mut self) {
        self.open = false;
        self.bracket_group = false;
    }

    pub(crate) fn begin_bracket(&mut self, id: u32, kind: UserEvent) {
        if self.brackets.is_empty() {
            self.seal();
        }
        self.brackets.push((id, kind));
    }

    /// Ends bracket `id` and any opened inside it; the outermost one's end seals its group. An
    /// unknown or ended bracket does nothing.
    pub(crate) fn end_bracket(&mut self, id: u32) {
        if let Some(i) = self.brackets.iter().position(|(b, _)| *b == id) {
            self.brackets.truncate(i);
            if self.brackets.is_empty() {
                self.seal();
            }
        }
    }

    /// Drops every group (the stored changes no longer fit the text).
    pub(crate) fn clear(&mut self) {
        self.done.clear();
        self.undone.clear();
        self.bytes = 0;
        self.seal();
    }

    /// A history transaction changed the text from `old` to `new`; `before` and `after` are the
    /// selections around it.
    pub(crate) fn record(
        &mut self,
        tr: &Transaction,
        old: &TextBuffer,
        new: &TextBuffer,
        before: &SelectionSet,
        after: &SelectionSet,
    ) {
        for g in self.undone.drain(..) {
            self.bytes -= g.weight;
        }
        let inv = tr.changes.invert(old);
        let edits = edits_of(&tr.changes);
        let ends: SmallVec<[usize; 1]> = edits.iter().map(|(_, ins, s)| s + ins.len()).collect();
        let in_bracket = !self.brackets.is_empty();
        let join = if in_bracket {
            self.bracket_group && !self.done.is_empty()
        } else {
            self.joinable(tr, &edits)
        };
        if join {
            let g = self.done.back_mut().expect("joining needs a top group");
            g.changes = inv.compose(&g.changes);
            for (i, start) in g.starts.iter_mut().enumerate() {
                // Earlier cursors' edits shifted this cursor's run.
                if let Some((r, _, s)) = edits.get(i) {
                    *start = start.saturating_add_signed(*s as isize - r.start as isize);
                }
            }
            if g.starts.len() != ends.len() {
                g.starts = edits.iter().map(|(_, _, s)| *s).collect();
            }
            g.ends = ends;
            g.current_sel = after.clone();
            g.t_last = tr.ann.time;
            self.bytes -= g.weight;
            g.weight = g.changes.weight();
            self.bytes += g.weight;
        } else {
            let kind = match self.brackets.first() {
                Some((_, kind)) => *kind,
                None => tr.ann.user_event,
            };
            let weight = inv.weight();
            self.done.push_back(Group {
                changes: inv,
                restore_sel: before.clone(),
                current_sel: after.clone(),
                kind,
                t_last: tr.ann.time,
                starts: edits.iter().map(|(_, _, s)| *s).collect(),
                ends,
                trailing_break: false,
                weight,
            });
            self.bytes += weight;
            self.open = !in_bracket;
            self.bracket_group = in_bracket;
        }
        let g = self.done.back_mut().expect("a group was just recorded");
        g.trailing_break = match (g.starts.first(), g.ends.first()) {
            (Some(&s), Some(&e)) if e > s => new.char_before(e).is_some_and(is_break),
            _ => false,
        };
        if g.changes.is_identity() {
            let g = self.done.pop_back().expect("the identity group is the top");
            self.bytes -= g.weight;
            self.open = false;
            self.bracket_group = false;
        }
        self.evict();
    }

    /// Whether `tr` (outside a bracket) continues the open top group.
    fn joinable(&self, tr: &Transaction, edits: &[Edit<'_>]) -> bool {
        let Some(g) = self.done.back() else {
            return false;
        };
        if !self.open || self.bracket_group || edits.len() != g.ends.len() {
            return false;
        }
        match tr.ann.time.checked_duration_since(g.t_last) {
            Some(d) if d <= self.window => {}
            _ => return false,
        }
        match (g.kind, tr.ann.user_event) {
            (UserEvent::InputType, UserEvent::InputType) => {
                continues(edits, g, |r, ins, end, _| {
                    r.is_empty()
                        && r.start == end
                        && (!g.trailing_break || ins.chars().next().is_some_and(is_break))
                })
            }
            (UserEvent::InputType, UserEvent::DeleteBackward) => {
                continues(edits, g, |r, ins, end, start| {
                    ins.is_empty() && r.end == end && r.start >= start
                })
            }
            (UserEvent::DeleteBackward, UserEvent::DeleteBackward) => {
                continues(edits, g, |r, ins, end, _| ins.is_empty() && r.end == end)
            }
            (UserEvent::DeleteForward, UserEvent::DeleteForward) => {
                continues(edits, g, |r, ins, end, _| ins.is_empty() && r.start == end)
            }
            _ => false,
        }
    }

    /// A commit that changed only the selection: a caret moved away from where the open group's
    /// last change ended seals it (outside brackets).
    pub(crate) fn selection_moved(&mut self, sel: &SelectionSet) {
        if !self.open || !self.brackets.is_empty() {
            return;
        }
        let Some(g) = self.done.back() else {
            return;
        };
        let adjacent = sel.ranges().len() == g.ends.len()
            && sel
                .ranges()
                .iter()
                .zip(&g.ends)
                .all(|(r, &end)| r.is_empty() && r.head == end);
        if !adjacent {
            self.open = false;
        }
    }

    /// The top group's change was applied (exactly) over `old`: it moves to the redo stack as
    /// the change that redoes it.
    pub(crate) fn finish_undo(&mut self, old: &TextBuffer) {
        if let Some(g) = self.done.pop_back() {
            let redo = self.flipped(g, old);
            self.undone.push(redo);
        }
        self.seal();
    }

    /// The top redo group's change was applied over `old`: it moves back to the undo stack.
    pub(crate) fn finish_redo(&mut self, old: &TextBuffer) {
        if let Some(g) = self.undone.pop() {
            let undo = self.flipped(g, old);
            self.done.push_back(undo);
        }
        self.seal();
        self.evict();
    }

    /// `g`, its change applied over `old`, as the step that takes it back.
    fn flipped(&mut self, g: Group, old: &TextBuffer) -> Group {
        let changes = g.changes.invert(old);
        let weight = changes.weight();
        self.bytes = self.bytes - g.weight + weight;
        Group {
            changes,
            restore_sel: g.current_sel,
            current_sel: g.restore_sel,
            kind: g.kind,
            t_last: g.t_last,
            ends: SmallVec::new(),
            starts: SmallVec::new(),
            trailing_break: false,
            weight,
        }
    }

    /// A non-history change `c` of the current text: every stored group is rebased over it,
    /// from the newest down; a group it overlaps goes, with everything older.
    pub(crate) fn rebase(&mut self, c: &ChangeSet) {
        self.open = false;
        let kept = rebase_stack(self.done.iter_mut().rev(), c);
        let drop = self.done.len() - kept;
        if kept == 0 {
            self.bracket_group = false;
        }
        self.done.drain(..drop);
        let kept = rebase_stack(self.undone.iter_mut().rev(), c);
        let drop = self.undone.len() - kept;
        self.undone.drain(..drop);
        self.bytes = self.done.iter().chain(&self.undone).map(|g| g.weight).sum();
    }

    /// Keeps the bounds: the group count, a newest group over [`LARGE_GROUP_BYTES`] evicting
    /// everything older, then the byte budget (never the newest group).
    fn evict(&mut self) {
        while self.done.len() > self.max_groups {
            if let Some(g) = self.done.pop_front() {
                self.bytes -= g.weight;
            }
        }
        let large = self
            .done
            .back()
            .is_some_and(|g| g.weight > LARGE_GROUP_BYTES);
        while large && self.done.len() > 1 {
            if let Some(g) = self.done.pop_front() {
                self.bytes -= g.weight;
            }
        }
        while self.bytes > self.max_bytes && self.done.len() > 1 {
            if let Some(g) = self.done.pop_front() {
                self.bytes -= g.weight;
            }
        }
        if self.done.is_empty() {
            self.open = false;
            self.bracket_group = false;
        }
    }
}

/// Every edit passes `ok(old range, inserted text, where its cursor's run in `g` ends, where it
/// starts)`.
fn continues(
    edits: &[Edit<'_>],
    g: &Group,
    ok: impl Fn(&Range<usize>, &str, usize, usize) -> bool,
) -> bool {
    edits
        .iter()
        .zip(g.ends.iter().zip(&g.starts))
        .all(|((r, ins, _), (&end, &start))| ok(r, ins, end, start))
}

/// Rebases `groups` (newest first) over `c`; the number rebased before the first one `c`
/// overlaps.
fn rebase_stack<'a>(groups: impl Iterator<Item = &'a mut Group>, c: &ChangeSet) -> usize {
    let mut c = c.clone();
    let mut kept = 0;
    for g in groups {
        let Some((mine, theirs)) = g.changes.transform(&c) else {
            break;
        };
        g.current_sel = g.current_sel.map(&c.desc());
        g.restore_sel = g.restore_sel.map(&theirs.desc());
        g.changes = mine;
        c = theirs;
        kept += 1;
    }
    kept
}
