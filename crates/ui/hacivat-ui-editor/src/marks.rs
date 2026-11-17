//! Gutter execution marks (UI-EDIT-19; Task 28), after `design/m3a-query/NOTES.md` surface 5
//! (binding over the plan's wording and the research's spinner and red square):
//!
//! - **Anchors.** A mark's range is a container range (`RangeKind::Container`: it grows with
//!   typing at its edges and goes only when one edit deleted all of it). Its glyph sits on the
//!   line of its header: the first token that is not a comment.
//! - **Submission.** [`add`] rebases each range from the version the units were resolved at
//!   through the `ChangeLog` (`TooOld` or `Deleted`: the mark is dropped); a range edited in
//!   between is stale from the start. A mark of the same key, or of exactly the same range, is
//!   replaced (re-execution).
//! - **Staleness.** An edit that touches a mark's range (inside it, or at its edges) marks it
//!   stale: its glyph and bar dim to `text_3`, its end-of-line status says "edited since
//!   execution", its tooltip "Edited since execution. The marks show the last run." A mark still
//!   in flight (queued, running, cancelling) keeps its live glyph until it finishes.
//! - **Tombstones.** A mark whose range one edit deleted becomes a tombstone for 64 text
//!   transactions; an edit that puts the same text back at the same place (undo) restores it,
//!   state and all.
//! - **Glyphs** (a 12 × 18 px slot at x = 8): queued, a 7 px hollow `seam_strong` square;
//!   running (after 50 ms; nothing before), the 7 px amber run marker; succeeded, the green check
//!   and a 1 px green bar at x = 4 over the range; failed, the ink alert glyph (`text`);
//!   cancelling, a 7 px hollow amber square; cancelled, the ban glyph and skipped, skip-forward,
//!   both `text_2`. **Never red.** Two marks on one row: the worse glyph and a 3 px `text_2`
//!   dot at its lower right.
//! - **End of line** (micro 11.5 `text_3`, right-aligned 14 px from the text area's right
//!   edge): the app's `eol` text (running and cancelling in amber, failed in `text` with the
//!   SQLSTATE chip, succeeded after a green check); two marks are listed divided by `|`. When
//!   the status would come within 12 px of its line's text it is hidden, except a failure's
//!   SQLSTATE chip, which moves onto a panel pad over the line's tail (the short form; the other
//!   short forms need the app's formatted durations, a Task 33 note).
//! - **Timer.** While a mark is running or cancelling, the editor repaints once a second (and
//!   once at a running mark's 50 ms), scheduled from paint: a window that is not painted (not
//!   visible) schedules nothing, and the timer stops when nothing runs ([`MarkStore::ticking`]).

use std::cell::RefCell;
use std::hash::Hasher;
use std::ops::Range;
use std::time::{Duration, Instant};

use hacivat_text::{Bias, ChangeLog, Committed, MapMode, RangeKind, Rebased, Snapshot, TextBuffer};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::l10n::UiArg;
use hacivat_ui::popover::Tip;
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, AvailableSpace, Bounds, ContentMask, Context,
    Hitbox, HitboxBehavior, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div, fill,
    point, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle, popover_surface};
use twox_hash::XxHash3_64;

use crate::element::FrameLayout;
use crate::gutter::{BAR_X, GutterCtx, Inks, NUMBER_PAD, hint_text};
use crate::preview::PreviewHint;

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
    // ## m3b-t01b-seams: a cancel escalated to Terminate/Deactivate (Task 23 sets it; Task 34
    // paints its gutter badge). Until then it reads as Cancelling everywhere.
    Escalated {
        since: Instant,
    },
}

/// How long a running mark shows nothing (no flash for a fast statement).
pub const RUNNING_GRACE: Duration = Duration::from_millis(50);
/// The live clock's period.
pub const TICK: Duration = Duration::from_secs(1);
/// Text transactions a tombstone is kept for.
pub const TOMBSTONE_TRANSACTIONS: u32 = 64;
/// The most bytes of a mark scanned for its header.
const HEADER_SCAN: usize = 64 << 10;
/// The glyph slot: x, width; the hollow and filled squares' side.
const GLYPH_X: f32 = 8.0;
const GLYPH_W: f32 = 12.0;
const SQUARE: f32 = 7.0;
/// The "more on this row" dot.
const DOT: f32 = 3.0;
/// The end of line: right inset, the gap it keeps from the text, its parts' gap, the pad's
/// left padding.
const EOL_RIGHT: f32 = NUMBER_PAD;
const EOL_CLEAR: f32 = 12.0;
const EOL_GAP: f32 = 5.0;
const PAD_LEFT: f32 = 10.0;

/// One mark.
struct Mark {
    key: MarkKey,
    range: Range<usize>,
    /// The first byte of the first token that is not a comment.
    header: usize,
    state: Option<ExecMarkState>,
    label: Option<TextFn>,
    eol: Option<TextFn>,
    // ## m3a-t33-exec-ui (named exception): the app's short form on a long line.
    short: Option<TextFn>,
    stale: bool,
}

/// A mark one edit deleted, kept so that putting the text back (undo) restores it.
struct Tombstone {
    mark: Mark,
    /// Where the deleted text was, in the current text (mapped through later edits).
    at: usize,
    /// The mark's start from the deleted text's start (old coordinates), its length and its
    /// text's hash.
    rel: usize,
    len: usize,
    hash: u64,
    /// Text transactions since the deletion.
    age: u32,
}

/// The repaint timer of the running and cancelling marks: when it fires, and its task.
struct Ticker {
    due: Instant,
    _task: Task<()>,
}

/// The editor's marks (module-owned state, P11).
#[derive(Default)]
pub struct MarkStore {
    marks: Vec<Mark>,
    tombstones: Vec<Tombstone>,
    /// The text the last commit produced (a deletion's mark text is hashed from it).
    prev: Option<TextBuffer>,
    timer: RefCell<Option<Ticker>>,
}

impl std::fmt::Debug for MarkStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkStore")
            .field(
                "marks",
                &self
                    .marks
                    .iter()
                    .map(|m| (m.key, &m.range, &m.state, m.stale))
                    .collect::<Vec<_>>(),
            )
            .field("tombstones", &self.tombstones.len())
            .field("ticking", &self.ticking())
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
        self.mark(key).and_then(|m| m.state.as_ref())
    }

    /// The range of the mark `key` (bytes of the current text).
    pub fn range(&self, key: MarkKey) -> Option<Range<usize>> {
        self.mark(key).map(|m| m.range.clone())
    }

    /// The header offset of the mark `key` (its glyph's line).
    pub fn header(&self, key: MarkKey) -> Option<usize> {
        self.mark(key).map(|m| m.header)
    }

    /// The mark `key` was edited since it ran.
    pub fn is_stale(&self, key: MarkKey) -> bool {
        self.mark(key).is_some_and(|m| m.stale)
    }

    /// Deleted marks waiting for an undo.
    pub fn tombstones(&self) -> usize {
        self.tombstones.len()
    }

    /// The repaint timer of running and cancelling marks is scheduled.
    pub fn ticking(&self) -> bool {
        self.timer.borrow().is_some()
    }

    /// Each mark with a state: the state, its range, whether it is stale (the scrollbar's
    /// markers).
    /// Only marks with a glyph at `now`: a running mark shows nothing for its first 50 ms, here
    /// as in the gutter (review round 1, Minor 5). The `u8` is the glyph's severity (the worse
    /// one wins a shared marker).
    pub(crate) fn markers(
        &self,
        now: Instant,
    ) -> impl Iterator<Item = (&ExecMarkState, Range<usize>, bool, u8)> {
        self.marks
            .iter()
            .filter(|m| !m.range.is_empty())
            .filter_map(move |m| {
                let glyph = glyph_of(m, now)?;
                Some((
                    m.state.as_ref()?,
                    m.range.clone(),
                    m.stale,
                    glyph.severity(),
                ))
            })
    }

    fn mark(&self, key: MarkKey) -> Option<&Mark> {
        self.marks.iter().find(|m| m.key == key)
    }

    /// A succeeded mark that is not stale (a green bar at x = 4) covers one of `lines`.
    pub(crate) fn executed_bar_on(&self, buf: &TextBuffer, lines: Range<u32>) -> bool {
        self.marks.iter().any(|m| {
            has_bar(m) && !m.stale && {
                let (a, b) = crate::preview::unit_lines(buf, m.range.clone());
                a < lines.end && lines.start <= b
            }
        })
    }
}

/// A succeeded mark draws a range bar (green, or `text_3` once stale).
fn has_bar(m: &Mark) -> bool {
    matches!(m.state, Some(ExecMarkState::Succeeded { .. })) && !m.range.is_empty()
}

/// The first byte of `r` in `buf` that is neither blank nor inside a comment (`--` to the line
/// end, `/* … */` nested as PostgreSQL nests them; `/*!` and `/*+` are code: the MySQL
/// executable comment and the optimizer hint); `r.start` when there is none.
fn header_of(buf: &TextBuffer, r: &Range<usize>) -> usize {
    // At most the first 64 KB are scanned (a longer leading comment puts the glyph on the
    // range's first line).
    let end = buf
        .snap(r.end.min(r.start + HEADER_SCAN), Bias::Before)
        .max(r.start);
    let text = buf.slice(r.start..end);
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' => i += 1,
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') && !matches!(b.get(i + 2), Some(b'!' | b'+')) => {
                let mut depth = 0usize;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            _ => return r.start + i,
        }
    }
    r.start
}

fn hash_of(buf: &TextBuffer, r: Range<usize>) -> u64 {
    let mut h = XxHash3_64::default();
    for chunk in buf.chunks_in(r) {
        h.write(chunk.as_bytes());
    }
    h.finish()
}

/// Adds `marks` computed at `version`; a mark of the same key, or of exactly the same range, is
/// replaced (re-execution). `log` rebases them to `snap.version` (`TooOld` or `Deleted`: the
/// mark is dropped; a range edited in between is stale); `snap` places each header.
pub fn add(
    s: &mut MarkStore,
    version: u64,
    marks: Vec<(MarkKey, Range<usize>)>,
    log: &ChangeLog,
    snap: &Snapshot,
) {
    let buf = &snap.buffer;
    if s.prev.is_none() {
        s.prev = Some(buf.clone());
    }
    for (key, range) in marks {
        let range = if range.start <= range.end {
            range
        } else {
            range.end..range.start
        };
        let Rebased::Mapped(now) = log.rebase(version, range.clone(), RangeKind::Container) else {
            s.marks.retain(|m| m.key != key);
            continue;
        };
        if now.end > buf.len_bytes()
            || !buf.is_char_boundary(now.start)
            || !buf.is_char_boundary(now.end)
        {
            // ## m3a-t28-gutter (fix round 1, Minor 8): the re-run replaces the old outcome even
            // when its own range cannot be placed.
            s.marks.retain(|m| m.key != key);
            continue;
        }
        // The same rule as `on_commit`: an edit inside it, or text added at its edges.
        let stale = now.len() != range.len()
            || !matches!(
                log.rebase(version, range.clone(), RangeKind::Content),
                Rebased::Mapped(_)
            );
        s.marks.retain(|m| m.key != key && m.range != now);
        s.tombstones.retain(|t| t.mark.key != key);
        let mark = Mark {
            key,
            header: header_of(buf, &now),
            range: now,
            state: None,
            label: None,
            eol: None,
            short: None,
            stale,
        };
        let at = s
            .marks
            .partition_point(|m| m.range.start <= mark.range.start);
        s.marks.insert(at, mark);
    }
}

/// Sets a mark's state, its gutter label (its tooltip) and its end-of-line status.
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
    } else if let Some(t) = s.tombstones.iter_mut().find(|t| t.mark.key == key) {
        // A deleted statement's result still arrives: an undo restores it with its outcome.
        t.mark.state = Some(state);
        t.mark.label = label;
        t.mark.eol = eol;
    }
}

// ## m3a-t33-exec-ui (named exception)
/// Sets a mark's short form: what its end-of-line status shows on a long line, on the pad
/// (a success: the green check and this text; a running or cancelling statement: this text in
/// amber). Without one only a failure's SQLSTATE chip shows there. A new state keeps it until
/// the app sets another.
pub fn set_short(s: &mut MarkStore, key: MarkKey, short: Option<TextFn>) {
    if let Some(m) = s.marks.iter_mut().find(|m| m.key == key) {
        m.short = short;
    } else if let Some(t) = s.tombstones.iter_mut().find(|t| t.mark.key == key) {
        t.mark.short = short;
    }
}

/// Removes every mark (and every tombstone).
pub fn clear(s: &mut MarkStore) {
    s.marks.clear();
    s.tombstones.clear();
    s.timer.borrow_mut().take();
}

/// Follows a committed transaction, `snap` being the version it produced: restores the
/// tombstones whose text the edit put back, rebases each mark, marks the touched ones stale and
/// turns the deleted ones into tombstones.
pub fn on_commit(s: &mut MarkStore, c: &Committed, log: &ChangeLog, snap: &Snapshot) {
    let _ = log;
    let prev = s.prev.replace(snap.buffer.clone());
    if !c.buffer_changed || (s.marks.is_empty() && s.tombstones.is_empty()) {
        return;
    }
    let desc = &c.desc;
    let buf = &snap.buffer;
    // The tombstones first: the text this edit put back may be a deleted mark's (an undo).
    let mut back = Vec::new();
    for mut t in std::mem::take(&mut s.tombstones) {
        let Some(at) = desc.map_pos(t.at, Bias::Before, MapMode::Simple) else {
            continue;
        };
        t.at = at;
        let r = at + t.rel..at + t.rel + t.len;
        let touched = c
            .dirty
            .as_ref()
            .is_some_and(|d| d.start <= r.end && r.start <= d.end);
        if touched
            && r.end <= buf.len_bytes()
            && buf.is_char_boundary(r.start)
            && buf.is_char_boundary(r.end)
            && hash_of(buf, r.clone()) == t.hash
        {
            let mut m = t.mark;
            let header_rel = m.header.saturating_sub(m.range.start);
            m.header = (r.start + header_rel).min(r.end);
            m.range = r;
            back.push(m);
            continue;
        }
        t.age += 1;
        if t.age <= TOMBSTONE_TRANSACTIONS {
            s.tombstones.push(t);
        }
    }
    let mut kept = Vec::with_capacity(s.marks.len());
    for mut m in std::mem::take(&mut s.marks) {
        let old = m.range.clone();
        if let Some(new) = desc.map_range(old.clone(), RangeKind::Container) {
            let content = desc.map_range(old.clone(), RangeKind::Content);
            if content.is_none() || new.len() != old.len() {
                m.stale = true;
            }
            m.header = desc
                .map_pos(m.header, Bias::After, MapMode::Simple)
                .unwrap_or(new.start)
                .clamp(new.start, new.end);
            m.range = new;
            kept.push(m);
            continue;
        }
        // One edit deleted all of it: a tombstone, when its text is known.
        let Some(prev) = prev.as_ref().filter(|p| p.len_bytes() == desc.len_old()) else {
            continue;
        };
        let Some(at) = desc.map_pos(old.start, Bias::Before, MapMode::Simple) else {
            continue;
        };
        // ## m3a-t28-gutter (fix round 1, Minor 6): the mark's offset in the deleted text, in old
        // coordinates. Every old position from the deletion's start to the mark's start maps
        // (`Before`) to `at`, every one before it maps lower, so the deletion's start is the
        // least position that maps to `at` (a binary search, O(log n · sections)).
        let (mut lo, mut hi) = (0, old.start);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if desc.map_pos(mid, Bias::Before, MapMode::Simple) == Some(at) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        s.tombstones.push(Tombstone {
            rel: old.start - lo,
            len: old.len(),
            hash: hash_of(prev, old),
            at,
            mark: m,
            age: 0,
        });
    }
    s.marks = kept;
    for m in back {
        s.marks.retain(|k| k.key != m.key);
        let at = s.marks.partition_point(|k| k.range.start <= m.range.start);
        s.marks.insert(at, m);
    }
}

/// The gutter's hit targets and elements of one frame (the glyphs with their tooltips, the
/// end-of-line statuses and the band's hints): laid out and inserted in prepaint, painted by
/// [`paint`].
#[derive(Default)]
pub struct MarkHitboxes {
    /// Each glyph's slot, by the mark it shows (the worse one of a row).
    pub glyphs: Vec<(MarkKey, Hitbox)>,
    /// The end-of-line statuses placed this frame: their line and whether they sit on the pad.
    pub statuses: Vec<(u32, bool)>,
    elements: RefCell<Vec<(AnyElement, bool)>>,
}

impl std::fmt::Debug for MarkHitboxes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkHitboxes")
            .field(
                "glyphs",
                &self.glyphs.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            )
            .field("statuses", &self.statuses)
            .field("elements", &self.elements.borrow().len())
            .finish()
    }
}

/// What a glyph slot shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Glyph {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelling,
    Cancelled,
    Skipped,
}

impl Glyph {
    /// Which of two marks on one row shows: the worse.
    fn severity(self) -> u8 {
        match self {
            Glyph::Failed => 7,
            Glyph::Cancelling => 6,
            Glyph::Running => 5,
            Glyph::Queued => 4,
            Glyph::Cancelled => 3,
            Glyph::Skipped => 2,
            Glyph::Succeeded => 1,
        }
    }

    /// Still in flight: staleness waits until it finishes.
    fn live(self) -> bool {
        matches!(self, Glyph::Queued | Glyph::Running | Glyph::Cancelling)
    }
}

// ## m3c-t32-fold
/// Which mark a folded header's row shows (NOTES M3c-7, controller carry): the one that most
/// needs the user. The editor's states in that order: a cancel not answered (`Escalated`) >
/// failed > running > cancelling > queued > cancelled > skipped > done > stale. The app shows
/// "awaiting an answer" as a running mark and "rolled back" through its label (NOTES ruling
/// 31), so they rank as those states here.
fn fold_rank(m: &Mark, glyph: Glyph) -> u8 {
    if m.stale && !glyph.live() {
        return 0;
    }
    match m.state {
        Some(ExecMarkState::Escalated { .. }) => 9,
        Some(ExecMarkState::Failed { .. }) => 8,
        Some(ExecMarkState::Running { .. }) => 7,
        Some(ExecMarkState::Cancelling { .. }) => 6,
        Some(ExecMarkState::Queued) => 5,
        Some(ExecMarkState::Cancelled) => 4,
        Some(ExecMarkState::Skipped) => 3,
        Some(ExecMarkState::Succeeded { .. }) => 2,
        None => 1,
    }
}

/// The glyph of `m` at `now` (`None`: no state yet, or running for less than 50 ms).
fn glyph_of(m: &Mark, now: Instant) -> Option<Glyph> {
    Some(match m.state.as_ref()? {
        ExecMarkState::Queued => Glyph::Queued,
        ExecMarkState::Running { started } => {
            if now.saturating_duration_since(*started) < RUNNING_GRACE {
                return None;
            }
            Glyph::Running
        }
        ExecMarkState::Succeeded { .. } => Glyph::Succeeded,
        ExecMarkState::Failed { .. } => Glyph::Failed,
        // ## m3b-t01b-seams: Escalated paints as Cancelling until Task 34's badge.
        ExecMarkState::Cancelling { .. } | ExecMarkState::Escalated { .. } => Glyph::Cancelling,
        ExecMarkState::Cancelled => Glyph::Cancelled,
        ExecMarkState::Skipped => Glyph::Skipped,
    })
}

/// When the marks next need a repaint: a running mark at its 50 ms and then on each whole
/// second of its clock, a cancelling one on each whole second; `None` when nothing runs.
fn next_tick(m: &MarkStore, now: Instant) -> Option<Instant> {
    let next = |since: Instant, grace: bool| {
        let el = now.saturating_duration_since(since);
        if grace && el < RUNNING_GRACE {
            return since + RUNNING_GRACE;
        }
        let secs = el.as_secs() + 1;
        since + TICK * u32::try_from(secs).unwrap_or(u32::MAX)
    };
    m.marks
        .iter()
        .filter_map(|mk| match mk.state.as_ref()? {
            ExecMarkState::Running { started } => Some(next(*started, true)),
            // ## m3b-t01b-seams: Escalated ticks as Cancelling.
            ExecMarkState::Cancelling { since } | ExecMarkState::Escalated { since } => {
                Some(next(*since, false))
            }
            _ => None,
        })
        .min()
}

/// One part of an end-of-line status.
enum Part {
    Text(SharedString, Hsla),
    Icon(IconName, Hsla),
    /// The SQLSTATE chip (data, mono 11, 1 px `seam_strong`, radius 3).
    Chip(SharedString),
    /// The divider between two marks' statuses.
    Divider,
}

/// The fonts and inks an end-of-line status uses.
struct EolStyle {
    inks: Inks,
    micro: TypeStyle,
    mono: TypeStyle,
    line_height: Pixels,
}

fn eol_element(parts: Vec<Part>, pad: bool, st: &EolStyle) -> AnyElement {
    let inks = st.inks;
    let mut row = div()
        .flex()
        .flex_row()
        .flex_none()
        .items_center()
        .gap(px(EOL_GAP))
        .h(st.line_height);
    if pad {
        row = row
            .bg(inks.panel)
            .pl(px(PAD_LEFT))
            // ## m3a-f2-visual (named exception): the status keeps its inset inside the pad.
            .pr(px(EOL_RIGHT))
            .border_l_1()
            .border_color(inks.seam_soft);
    }
    row.children(parts.into_iter().map(|p| {
        match p {
            Part::Text(t, color) => {
                Label::new(t, TextSource::Fluent, st.micro, color).into_any_element()
            }
            Part::Icon(name, color) => Icon::new(name)
                .size(IconSize::Chrome)
                .color(color)
                .into_any_element(),
            Part::Chip(code) => div()
                .flex_none()
                .border_1()
                .border_color(inks.seam_strong)
                .rounded(px(3.0))
                .px(px(4.0))
                .child(Label::new(code, TextSource::Data, st.mono, inks.text))
                .into_any_element(),
            Part::Divider => div()
                .flex_none()
                .w(px(1.0))
                .h(px(11.0))
                .bg(inks.text_3)
                .into_any_element(),
        }
    }))
    .into_any_element()
}

/// The status parts of `m` (`short`: the pad's short form); empty when it has none.
fn status_parts(m: &Mark, glyph: Glyph, short: bool, inks: &Inks, cx: &App) -> Vec<Part> {
    let mut parts = Vec::new();
    let terminal_stale = m.stale && !glyph.live();
    if short {
        if let Some(ExecMarkState::Failed {
            sqlstate: Some(code),
        }) = &m.state
            && !terminal_stale
        {
            parts.push(Part::Chip(code.clone()));
        }
        // ## m3a-t33-exec-ui (named exception): the app's short form (NOTES surface 5: check
        // + duration, the amber clock); queued, skipped and stale drop their words.
        if let (Some(text), false) = (m.short.as_ref(), terminal_stale) {
            match glyph {
                Glyph::Succeeded => {
                    parts.push(Part::Icon(IconName::Check, inks.green));
                    parts.push(Part::Text(text(cx), inks.text_3));
                }
                Glyph::Running | Glyph::Cancelling => {
                    parts.push(Part::Text(text(cx), inks.amber));
                }
                _ => {}
            }
        }
        return parts;
    }
    if terminal_stale {
        parts.push(Part::Text(
            crate::commands::message(cx, "hv-editor-mark-stale", &[]),
            inks.text_3,
        ));
        return parts;
    }
    let Some(eol) = m.eol.as_ref() else {
        return parts;
    };
    let text = eol(cx);
    match (&m.state, glyph) {
        (_, Glyph::Succeeded) => {
            parts.push(Part::Icon(IconName::Check, inks.green));
            parts.push(Part::Text(text, inks.text_3));
        }
        (_, Glyph::Running | Glyph::Cancelling) => parts.push(Part::Text(text, inks.amber)),
        (Some(ExecMarkState::Failed { sqlstate }), _) => {
            parts.push(Part::Text(text, inks.text));
            if let Some(code) = sqlstate {
                parts.push(Part::Chip(code.clone()));
            }
        }
        _ => parts.push(Part::Text(text, inks.text_3)),
    }
    parts
}

/// The glyph's content in its 12 × 18 slot.
fn glyph_element(glyph: Glyph, stale: bool, inks: &Inks) -> AnyElement {
    let square = || div().flex_none().size(px(SQUARE));
    let dim = |c: Hsla| {
        if stale && !glyph.live() {
            inks.text_3
        } else {
            c
        }
    };
    let icon = |name: IconName, c: Hsla| {
        Icon::new(name)
            .size(IconSize::Chrome)
            .color(dim(c))
            .into_any_element()
    };
    match glyph {
        Glyph::Queued => square()
            .border_1()
            .border_color(inks.seam_strong)
            .into_any_element(),
        Glyph::Running => square().bg(inks.amber).into_any_element(),
        Glyph::Cancelling => square()
            .border_1()
            .border_color(inks.amber)
            .into_any_element(),
        Glyph::Succeeded => icon(IconName::Check, inks.green),
        Glyph::Failed => icon(IconName::TriangleAlert, inks.text),
        Glyph::Cancelled => icon(IconName::Ban, inks.text_2),
        Glyph::Skipped => icon(IconName::SkipForward, inks.text_2),
    }
}

/// The message `id` with a unit's position (`i` of `n`) when given.
fn hint_message(cx: &App, hint: PreviewHint) -> SharedString {
    let (id, args) = hint_text(hint);
    // ## m3a-t33-exec-ui (named exception)
    if let PreviewHint::Choices { n } = hint {
        return crate::commands::message(cx, id, &[("n", UiArg::Count(u64::from(n)))]);
    }
    match args {
        Some((i, n)) => crate::commands::message(
            cx,
            id,
            &[("i", UiArg::Count(i as u64)), ("n", UiArg::Count(n as u64))],
        ),
        None => crate::commands::message(cx, id, &[]),
    }
}

/// The tooltip of a row with several marks: "Two statements on this line", then each mark's
/// own tooltip, one per line, on the popover surface (as `Tip`).
struct MarksTip {
    lines: Vec<SharedString>,
}

impl MarksTip {
    fn view(lines: Vec<SharedString>, cx: &mut App) -> AnyView {
        cx.new(|_| MarksTip { lines }).into()
    }
}

impl Render for MarksTip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        popover_surface(&theme)
            .px(px(8.0))
            .py(px(4.0))
            .flex()
            .flex_col()
            .children(self.lines.iter().map(|line| {
                Label::new(
                    line.clone(),
                    TextSource::Fluent,
                    t.type_ramp.body_sm,
                    t.colors.text.to_hsla(),
                )
            }))
    }
}

/// Lays out the marks of the painted rows (prepaint): each row's glyph (with its tooltip and
/// hit target) and end-of-line status, and the band's hints on rows without a status.
pub fn prepaint(
    m: &MarkStore,
    g: &GutterCtx<'_>,
    window: &mut Window,
    cx: &mut App,
) -> MarkHitboxes {
    let mut out = MarkHitboxes::default();
    let ov = g.overlay;
    // ## m3c-t32-fold (named exception): the painted rows and the lines their folds hide.
    let Some((first, last)) = ov.covered() else {
        return out;
    };
    let buf = &g.snapshot.buffer;
    let now = cx.background_executor().now();
    let theme = cx
        .try_global::<Theme>()
        .copied()
        .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
    let st = EolStyle {
        inks: Inks::of(cx),
        micro: theme.tokens.type_ramp.micro,
        mono: theme.tokens.type_ramp.mono_micro,
        line_height: g.line_height,
    };
    let inks = st.inks;
    let vis = buf.line_to_byte(first as usize)..buf.line_range(last as usize, true).end;
    // The marks by their header's row (## m3c-t32-fold: a mark on a hidden line is on its
    // fold's header row, `true` for "inside").
    let mut rows: Vec<(u32, Vec<_>)> = Vec::new();
    for mk in &m.marks {
        if mk.header < vis.start || mk.header > vis.end {
            continue;
        }
        let Some(glyph) = glyph_of(mk, now) else {
            continue;
        };
        let own = u32::try_from(buf.byte_to_line(mk.header)).unwrap_or(u32::MAX);
        if own < first || own > last {
            continue;
        }
        // ## m3c-t32-fold (named exception)
        let Some(line) = ov.row_showing(own).map(|r| r.line) else {
            continue;
        };
        let inside = line != own;
        match rows.iter_mut().find(|(l, _)| *l == line) {
            Some((_, v)) => v.push((mk, glyph, inside)),
            None => rows.push((line, vec![(mk, glyph, inside)])),
        }
    }
    let mut eols: Vec<(u32, Vec<Part>, Vec<Part>)> = Vec::new();
    for (line, all) in &rows {
        // ## m3c-t32-fold (named exception): a folded header's row shows the one mark that
        // most needs the user (NOTES M3c-7), saying where it is; other rows list theirs.
        let folded = all.iter().any(|m| m.2);
        let winner = if folded {
            all.iter().max_by_key(|m| fold_rank(m.0, m.1))
        } else {
            all.iter().max_by_key(|m| m.1.severity())
        };
        let Some(&(worst, glyph, worst_inside)) = winner else {
            continue;
        };
        let shown: Vec<(&Mark, Glyph)> = if folded {
            vec![(worst, glyph)]
        } else {
            all.iter().map(|&(m, g, _)| (m, g)).collect()
        };
        let marks = &shown;
        let slot = Bounds::new(
            point(g.bounds.left() + px(GLYPH_X), g.top_of(*line)),
            size(px(GLYPH_W), g.line_height),
        );
        // ## m3a-t28-gutter (fix round 1, Minor 7): two marks on a row list both statements
        // under "Two statements on this line" (NOTES surface 5).
        let tip_of = |mk: &Mark, gl: Glyph| -> Option<SharedString> {
            if mk.stale && !gl.live() {
                Some(crate::commands::message(cx, "hv-editor-tip-stale", &[]))
            } else {
                mk.label.as_ref().map(|f| f(cx))
            }
        };
        let mut tip_lines: Vec<SharedString> = marks
            .iter()
            .filter_map(|&(mk, gl)| tip_of(mk, gl))
            .collect();
        if marks.len() > 1 {
            tip_lines.insert(0, crate::commands::message(cx, "hv-editor-tip-two", &[]));
        }
        let mut el = div()
            .id(("hv-exec-mark", worst.key.0))
            .relative()
            .w(px(GLYPH_W))
            .h(g.line_height)
            .flex()
            .items_center()
            .justify_center()
            .child(glyph_element(glyph, worst.stale, &inks));
        if all.len() > 1 {
            el = el.child(
                div()
                    .absolute()
                    .right(px(-DOT))
                    .bottom(px(2.0))
                    .size(px(DOT))
                    .bg(inks.text_2),
            );
        }
        match tip_lines.len() {
            0 => {}
            1 => {
                let tip = tip_lines.remove(0);
                el = el.tooltip(move |_, cx| Tip::view(tip.clone(), cx));
            }
            _ => el = el.tooltip(move |_, cx| MarksTip::view(tip_lines.clone(), cx)),
        }
        let mut el = el.into_any_element();
        el.prepaint_as_root(
            slot.origin,
            size(
                AvailableSpace::Definite(px(GLYPH_W)),
                AvailableSpace::Definite(g.line_height),
            ),
            window,
            cx,
        );
        out.elements.borrow_mut().push((el, false));
        out.glyphs.push((
            worst.key,
            window.insert_hitbox(slot, HitboxBehavior::Normal),
        ));
        // The row's status: each mark's, divided.
        let (mut full, mut short) = (Vec::new(), Vec::new());
        // ## m3c-t32-fold (named exception): a failure inside a fold names its line.
        if worst_inside
            && let Some(ExecMarkState::Failed { sqlstate }) = &worst.state
            && !worst.stale
        {
            let at = buf.byte_to_line(worst.header) as u64 + 1;
            full.push(Part::Text(
                crate::commands::message(
                    cx,
                    "hv-editor-fold-failed",
                    &[("line", UiArg::Count(at))],
                ),
                inks.text,
            ));
            if let Some(code) = sqlstate {
                full.push(Part::Chip(code.clone()));
                short.push(Part::Chip(code.clone()));
            }
        }
        let named = !full.is_empty();
        for &(mk, gl) in marks.iter().filter(|_| !named) {
            for (list, is_short) in [(&mut full, false), (&mut short, true)] {
                let parts = status_parts(mk, gl, is_short, &inks, cx);
                if !parts.is_empty() {
                    if !list.is_empty() {
                        list.push(Part::Divider);
                    }
                    list.extend(parts);
                }
            }
        }
        if !full.is_empty() {
            eols.push((*line, full, short));
        }
    }
    // The band's hints, on rows no status took.
    for &(line, hint) in &ov.band.hints {
        if line < first || line > last || eols.iter().any(|(l, ..)| *l == line) {
            continue;
        }
        let mut parts = Vec::new();
        match hint {
            PreviewHint::Uncertain => parts.push(Part::Icon(IconName::CircleAlert, inks.text)),
            PreviewHint::Offline => parts.push(Part::Icon(IconName::Unplug, inks.text_3)),
            PreviewHint::Unit { .. } => {}
            // ## m3a-t33-exec-ui (named exception)
            PreviewHint::Choices { .. } => {
                parts.push(Part::Icon(IconName::ChevronsUpDown, inks.text_3));
            }
        }
        parts.push(Part::Text(hint_message(cx, hint), inks.text_3));
        eols.push((line, parts, Vec::new()));
    }
    // Each status right-aligned; on collision with its line's text, the short form on the pad,
    // or nothing.
    let right = ov.text_area.right() - px(EOL_RIGHT);
    let lh = g.line_height;
    let fit = |el: &mut AnyElement, window: &mut Window, cx: &mut App| {
        el.layout_as_root(
            size(AvailableSpace::MinContent, AvailableSpace::Definite(lh)),
            window,
            cx,
        )
        .width
    };
    for (line, full, short) in eols {
        let Some(row) = ov.row(line) else {
            continue;
        };
        let mut el = eol_element(full, false, &st);
        let mut x = right - fit(&mut el, window, cx);
        let mut pad = false;
        if row.text_end + px(EOL_CLEAR) > x {
            if short.is_empty() {
                continue;
            }
            el = eol_element(short, true, &st);
            // ## m3a-f2-visual (named exception in Task 28's file): the pad reaches the code
            // area's right edge (its right padding is the status's inset), so no code shows
            // past it.
            x = ov.text_area.right() - fit(&mut el, window, cx);
            pad = true;
        }
        if x < ov.text_area.left() {
            continue;
        }
        el.prepaint_at(point(x, row.top), window, cx);
        out.elements.borrow_mut().push((el, true));
        out.statuses.push((line, pad));
    }
    out
}

/// Paints the marks: the succeeded marks' range bars (1 px at x = 4: `green`, `text_3` once
/// stale), the glyphs and the end-of-line statuses laid out in [`prepaint`] (the statuses
/// clipped to the text area), then schedules the next repaint of a running mark's clock.
pub fn paint(
    m: &MarkStore,
    g: &GutterCtx<'_>,
    layout: &FrameLayout,
    hits: &MarkHitboxes,
    window: &mut Window,
    cx: &mut App,
) {
    let inks = Inks::of(cx);
    let buf = &g.snapshot.buffer;
    let area = layout.rows().bounds;
    // ## m3c-t32-fold (named exception): the lines the painted rows' folds hide included.
    if let Some((first, last)) = g.overlay.covered() {
        let vis = buf.line_to_byte(first as usize)..buf.line_range(last as usize, true).end;
        for mk in m.marks.iter().filter(|mk| has_bar(mk)) {
            if mk.range.end < vis.start || mk.range.start > vis.end {
                continue;
            }
            let (a, b) = crate::preview::unit_lines(buf, mk.range.clone());
            // ## m3c-t32-fold (named exception): a bar over folded lines collapses onto the
            // header's row.
            let y0 = (g.top_of(a) + px(1.0)).max(g.bounds.top());
            let y1 = (g.bottom_of(b) - px(1.0)).min(g.bounds.bottom());
            if y1 > y0 {
                window.paint_quad(fill(
                    Bounds::new(
                        point(g.bounds.left() + px(BAR_X), y0),
                        size(px(1.0), y1 - y0),
                    ),
                    if mk.stale { inks.text_3 } else { inks.green },
                ));
            }
        }
    }
    for (el, in_text) in hits.elements.borrow_mut().iter_mut() {
        if *in_text {
            window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                el.paint(window, cx);
            });
        } else {
            el.paint(window, cx);
        }
    }
    schedule(m, window, cx);
}

/// Schedules (or stops) the repaint of running and cancelling marks.
fn schedule(m: &MarkStore, window: &mut Window, cx: &mut App) {
    let now = cx.background_executor().now();
    let mut timer = m.timer.borrow_mut();
    let Some(due) = next_tick(m, now) else {
        timer.take();
        return;
    };
    if timer.as_ref().is_some_and(|t| t.due == due) {
        return;
    }
    let view = window.current_view();
    let wait = due.saturating_duration_since(now);
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        cx.update(|cx| cx.notify(view));
    });
    *timer = Some(Ticker { due, _task: task });
}

// ## m3a-t28-gutter (fix round 1): the tombstone's offset in the deleted text.
#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hacivat_text::{ChangeSet, SelectionRange, SelectionSet};

    use super::*;

    fn snap(buffer: &TextBuffer, version: u64) -> Snapshot {
        Snapshot {
            buffer: buffer.clone(),
            selection: SelectionSet::single(SelectionRange {
                anchor: 0,
                head: 0,
                goal_x: None,
                assoc: Bias::After,
            }),
            version,
        }
    }

    fn commit(s: &mut MarkStore, cs: &ChangeSet, after: &TextBuffer, version: u64) {
        let desc = cs.desc();
        let c = Committed {
            version,
            dirty: desc.changed_range(),
            desc,
            buffer_changed: true,
        };
        on_commit(s, &c, &ChangeLog::default(), &snap(after, version));
    }

    /// One transaction inserts a comment above and deletes the mark with the line break before
    /// it (a multi-range edit: the deletion starts one byte before the mark, and its new start is
    /// shifted by the insertion). Undo puts both back: the mark is restored at its old place.
    /// Review round 1, Minor 6.
    #[test]
    fn a_multi_range_edit_then_undo_restores_the_mark() {
        let b0 = TextBuffer::from_str("SELECT 1;\nSELECT 2;\n");
        let mut s = MarkStore::default();
        add(
            &mut s,
            0,
            vec![(MarkKey(1), 10..19)],
            &ChangeLog::default(),
            &snap(&b0, 0),
        );
        set_state(&mut s, MarkKey(1), ExecMarkState::Cancelled, None, None);
        let edit = ChangeSet::from_edits(
            b0.len_bytes(),
            [(0..0, Arc::from("-- a\n")), (9..19, Arc::from(""))],
        )
        .unwrap();
        let mut b1 = b0.clone();
        edit.apply(&mut b1);
        assert_eq!(b1.slice(0..b1.len_bytes()), "-- a\nSELECT 1;\n");
        commit(&mut s, &edit, &b1, 1);
        assert_eq!((s.len(), s.tombstones()), (0, 1));
        let undo = edit.invert(&b0);
        let mut b2 = b1.clone();
        undo.apply(&mut b2);
        commit(&mut s, &undo, &b2, 2);
        assert_eq!(s.range(MarkKey(1)), Some(10..19), "{s:?}");
        assert_eq!(s.header(MarkKey(1)), Some(10));
        assert_eq!(s.state(MarkKey(1)), Some(&ExecMarkState::Cancelled));
        assert_eq!(s.tombstones(), 0);
    }
}
