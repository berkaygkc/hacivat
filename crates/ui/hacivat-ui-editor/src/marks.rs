//! Gutter execution marks (UI-EDIT-19; Task 28): container anchors rebased from the submitting
//! version, staleness after an edit, tombstones for undo, the 1 Hz timer while running, and the
//! painted glyphs (Failed is an ink glyph, Cancelling amber, never red).
//!
//! Task 1b: the store keeps the marks and their states as given (no rebasing, no staleness);
//! `paint` paints nothing.

use std::ops::Range;
use std::time::{Duration, Instant};

use hacivat_text::{ChangeLog, Committed, Snapshot};
use hacivat_ui::text::TextFn;
use hacivat_ui_core::gpui_surface::{App, SharedString, Window};

use crate::element::FrameLayout;
use crate::gutter::GutterCtx;

/// One mark's key (the app's unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MarkKey(pub u64);

/// What a succeeded unit produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RowsOutcome {
    Fetched { rows: u64, more: bool },
    Affected(u64),
}

/// A mark's state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExecMarkState {
    Queued,
    Running {
        started: Instant,
    },
    Succeeded {
        took: Duration,
        rows: Option<RowsOutcome>,
    },
    Failed {
        sqlstate: Option<SharedString>,
    },
    Cancelling {
        since: Instant,
    },
    Cancelled,
    Skipped,
}

/// One mark.
struct Mark {
    key: MarkKey,
    range: Range<usize>,
    version: u64,
    state: Option<ExecMarkState>,
    label: Option<TextFn>,
    eol: Option<TextFn>,
}

/// The editor's marks (module-owned state, P11).
#[derive(Default)]
pub struct MarkStore {
    marks: Vec<Mark>,
}

impl std::fmt::Debug for MarkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.marks.iter().map(|m| {
                (
                    m.key,
                    &m.range,
                    m.version,
                    &m.state,
                    m.label.is_some(),
                    m.eol.is_some(),
                )
            }))
            .finish()
    }
}

impl MarkStore {
    /// The number of marks.
    pub fn len(&self) -> usize {
        self.marks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// The state of the mark `key`, when it has one.
    pub fn state(&self, key: MarkKey) -> Option<&ExecMarkState> {
        self.marks
            .iter()
            .find(|m| m.key == key)
            .and_then(|m| m.state.as_ref())
    }
}

/// Adds `marks` computed at `version`; a mark of the same key is replaced (re-execution).
/// `log` rebases them to `snap.version`; `snap` places each container anchor and its header
/// (Task 28).
pub fn add(
    s: &mut MarkStore,
    version: u64,
    marks: Vec<(MarkKey, Range<usize>)>,
    log: &ChangeLog,
    snap: &Snapshot,
) {
    let _ = (log, snap);
    for (key, range) in marks {
        s.marks.retain(|m| m.key != key);
        s.marks.push(Mark {
            key,
            range,
            version,
            state: None,
            label: None,
            eol: None,
        });
    }
}

/// Sets a mark's state, its gutter label and its end-of-line status.
pub fn set_state(
    s: &mut MarkStore,
    key: MarkKey,
    state: ExecMarkState,
    label: Option<TextFn>,
    eol: Option<TextFn>,
) {
    if let Some(m) = s.marks.iter_mut().find(|m| m.key == key) {
        m.state = Some(state);
        m.label = label;
        m.eol = eol;
    }
}

/// Removes every mark.
pub fn clear(s: &mut MarkStore) {
    s.marks.clear();
}

/// Follows a committed transaction, `snap` being the version it produced (Task 28: rebase,
/// staleness, tombstones).
pub fn on_commit(s: &mut MarkStore, c: &Committed, log: &ChangeLog, snap: &Snapshot) {
    let _ = (s, c, log, snap);
}

/// Paints the marks into the gutter, and each end-of-line status where `layout` says it does
/// not collide with the text (Task 28). Task 1b: nothing.
pub fn paint(
    m: &MarkStore,
    g: &GutterCtx<'_>,
    layout: &FrameLayout,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = (m, g, layout, window, cx);
}
