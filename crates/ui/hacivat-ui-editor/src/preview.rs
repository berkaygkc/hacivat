//! The Run preview band (UI-EDIT-21; Task 28): what Run would execute, drawn before Run is
//! pressed, and only when it is the truth (R-M3-8).
//!
//! - **When it is asked.** [`on_update`] restarts a 50 ms timer after every change and every
//!   selection move; when it fires, the band asks `LanguageService::exec_preview` at the
//!   editor's version (Run, `Execute1`, at the selections or else the carets, as Run asks). A
//!   fresh analysis or a new document context asks at once (unless the timer is pending).
//! - **Truth.** The answer is kept with the version and the place (`exec_at`: the carets and
//!   selections) it was asked at; a frame whose text is of another version, or whose carets or
//!   selections moved, draws no band, no gutter bar, no scrollbar marker and no status sentence
//!   until the new answer ([`plan`], review round 1). `None` from the service (no analysis of
//!   this version) draws no band. A range outside the text or off a character boundary draws no
//!   band (fail closed). A new document context drops the answer; a text change clears the
//!   chooser's override.
//! - **States** (design/m3a-query/NOTES.md surface 3, binding): `Units` and `AskPreselected`
//!   draw the `blue_tint` band (alpha × .55 dark, × .8 light) and a 1 px `blue` gutter bar;
//!   `Uncertain` the neutral `hover` fill, a dashed 1 px `text_2` bar and "boundary uncertain"
//!   at the end of the line; `NoConnection` the neutral fill, a solid `text_3` bar and "not
//!   connected"; `Nothing` no band, and the status bar says "Nothing to run here"
//!   ([`status_message`]).
//! - **Geometry.** A line the unit owns (nothing but blanks and its terminator outside it) gets
//!   the full-row band, from the gutter's edge to the right edge; a line it shares gets the exact
//!   tint over the unit's characters, without the terminator (NOTES open question 1). Several
//!   units (a selection run as separate statements) get one gutter bar each with a 2 px gap, a
//!   `panel` separator line between their bands and "1 of 3" at the end of each unit's first line.
//! - **The chooser's override** (hover in the D-011 chooser) wins over the service's answer.
//! - **Composing.** While an IME composition is open the band is frozen at its last resolved
//!   range, mapped through the edits as a container range; it asks again after the commit. Only
//!   an answer that was current when the composition began is kept (none while a query was
//!   pending), and a mapped answer is drawn only while frozen: it goes when the composition ends.

use std::ops::Range;
use std::time::Duration;

use hacivat_text::{Bias, RangeKind, Rebased, Snapshot, TextBuffer};
use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, Hsla, Pixels, SharedString, Task, Window, fill, point, px, size,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};
use smallvec::SmallVec;

use crate::element::{FrameLayout, VisibleRows};
use crate::marks::MarkStore;
use crate::service::{ExecAction, ExecAt, ExecPreview, PreviewState};
use crate::view::EditorView;

/// How long the band waits after a change or a move before it asks (UI-EDIT-21 step 1).
pub const PREVIEW_DELAY: Duration = Duration::from_millis(50);

/// The band's module-owned state (P11; `PreviewState` stays the exported verdict).
#[derive(Default)]
pub struct BandState {
    /// The last preview the service answered, at [`BandState::last_version`].
    pub last: Option<ExecPreview>,
    /// The chooser's hover override.
    pub override_ranges: Option<SmallVec<[Range<usize>; 2]>>,
    /// No update while an IME composition is open.
    pub frozen: bool,
    // ## m3a-t28-gutter
    /// The text version `last` was asked at (or, while frozen, mapped to).
    pub last_version: u64,
    // ## m3a-t28-gutter (fix round 1)
    /// Where `last` was asked (`exec_at` of the snapshot it was asked on): an answer is drawn
    /// only while the carets and selections are still there (review round 1, Critical 1).
    pub asked_at: Option<ExecAt>,
    /// `last` was mapped through a composition's edits, not resolved: drawn only while frozen
    /// and dropped when the composition ends (review round 1, Critical 2).
    pub mapped: bool,
    /// The 50 ms timer (dropping it cancels it).
    pending: Option<Task<()>>,
    /// This frame's band rectangles (built in prepaint from the frame's layout, painted by
    /// [`paint_band`]).
    rects: Vec<BandRect>,
    /// The text version `rects` were built for.
    rects_version: Option<u64>,
}

impl std::fmt::Debug for BandState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BandState")
            .field("last", &self.last)
            .field("last_version", &self.last_version)
            .field("asked_at", &self.asked_at)
            .field("mapped", &self.mapped)
            .field("override_ranges", &self.override_ranges)
            .field("frozen", &self.frozen)
            .field("pending", &self.pending.is_some())
            .finish_non_exhaustive()
    }
}

impl BandState {
    /// A preview query is waiting for its 50 ms timer.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
}

/// Why the band may need a fresh preview (the view's call points).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refresh {
    /// A committed change of the text.
    Changed,
    /// The selection or a caret moved, the text unchanged.
    SelectionMoved,
    /// The language service's analysis of a newer version landed (`LanguageService::on_update`).
    AnalysisLanded,
    /// The document context changed (connected, tier).
    ContextChanged,
}

/// How a band is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BandStyle {
    /// What Run executes (`Units`, `AskPreselected`, the chooser's hover): `blue_tint`, a `blue`
    /// gutter bar.
    Run,
    /// A boundary is uncertain: the neutral `hover` fill, a dashed `text_2` bar.
    Uncertain,
    /// No connection: the neutral `hover` fill, a solid `text_3` bar.
    Offline,
}

/// What the band shows over one frame's text: the style and the byte ranges (one per unit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandPlan {
    pub style: BandStyle,
    pub units: SmallVec<[Range<usize>; 2]>,
    /// From the chooser's hover, not the service.
    pub from_override: bool,
    // ## m3a-t33-exec-ui (named exception)
    /// Run would open the chooser on these units, offering this many candidates (0: it would
    /// not, or this is the chooser's own hover).
    pub choices: u32,
    // ## m3c-t34-editor (named exception)
    /// Run sends the selection as one unit of this many statements (`ExecPreview::as_one`).
    pub as_one: u32,
}

/// Called by the view after every commit, analysis and context change: restarts the 50 ms
/// timer (a change or a move), or asks at once (an analysis or a context change, unless the
/// timer is pending). While composing it only maps the frozen band through the edit.
///
/// A text change clears the chooser's override (its ranges are of the text it was hovered on);
/// a context change drops the answer asked under the old context (review round 1, I3 and C1).
pub fn on_update(v: &mut EditorView, why: Refresh, cx: &mut Context<EditorView>) {
    match why {
        Refresh::Changed => v.band.override_ranges = None,
        Refresh::ContextChanged => {
            v.band.last = None;
            v.band.mapped = false;
        }
        Refresh::SelectionMoved | Refresh::AnalysisLanded => {}
    }
    // A composition that ended on a path that did not say so (an unmark) ends the freeze too.
    if v.band.frozen && !v.ime.is_composing() {
        on_composition(&mut v.band, false);
    }
    if v.band.frozen || v.ime.is_composing() {
        follow_frozen(v);
        return;
    }
    match why {
        Refresh::Changed | Refresh::SelectionMoved => {
            v.band.pending = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(PREVIEW_DELAY).await;
                let _ = this.update(cx, |v, cx| {
                    v.band.pending = None;
                    ask(v);
                    cx.notify();
                });
            }));
        }
        Refresh::AnalysisLanded | Refresh::ContextChanged => {
            if v.band.pending.is_none() {
                ask(v);
                cx.notify();
            }
        }
    }
}

/// Asks the service for the preview at the editor's version.
fn ask(v: &mut EditorView) {
    let snap = v.model.snapshot();
    let at = exec_at(&snap);
    let answer = v
        .service
        .exec_preview(v.doc, snap.version, &at, ExecAction::Execute1);
    v.band.last = answer;
    v.band.last_version = snap.version;
    v.band.asked_at = Some(at);
    v.band.mapped = false;
}

/// Where Run is asked on `snap`: the main range only (## m3c-t31-multi, controller ruling 2:
/// with several carets Run follows the main one, the last added): its selection, else its
/// caret. `editor.run` (`commands/run.rs`) builds its request with this same function, so the
/// band and Run never ask about different places (review round 1, Minor 9).
pub(crate) fn exec_at(snap: &Snapshot) -> ExecAt {
    let main = snap.selection.main();
    if main.is_empty() {
        ExecAt::Carets(smallvec::smallvec![main.head])
    } else {
        let r: Range<usize> = main.range();
        ExecAt::Selections(smallvec::smallvec![r])
    }
}

/// Enters the freeze (once per composition, whichever comes first: the view's
/// `composition_changed(true)` or the first marked text). Only an answer that was true when the
/// composition began is kept: outside a freeze an answer is current exactly when no query is
/// pending (every change or move since the last answer starts the timer), and a mapped answer
/// never outlives its freeze. Otherwise there is no band while composing (review round 1,
/// Critical 2).
fn enter_freeze(s: &mut BandState) {
    if s.frozen {
        return;
    }
    s.frozen = true;
    if s.pending.take().is_some() || s.asked_at.is_none() || s.mapped {
        s.last = None;
        s.mapped = false;
    }
}

/// The frozen band follows the composition's edits as container ranges; a range the edit
/// deleted (or a change log too short to map) leaves no band.
fn follow_frozen(v: &mut EditorView) {
    enter_freeze(&mut v.band);
    let version = v.model.snapshot().version;
    if v.band.last_version == version {
        return;
    }
    let log = v.model.changelog();
    let from = v.band.last_version;
    if let Some(p) = v.band.last.as_mut() {
        let mut mapped = SmallVec::new();
        for r in &p.ranges {
            match log.rebase(from, r.clone(), RangeKind::Container) {
                Rebased::Mapped(r) => mapped.push(r),
                Rebased::Deleted | Rebased::TooOld => {
                    v.band.last = None;
                    break;
                }
            }
        }
        if let Some(p) = v.band.last.as_mut() {
            p.ranges = mapped;
            v.band.mapped = true;
        }
    }
    v.band.last_version = version;
}

/// Called by the view when an IME composition opens or closes: the band freezes while composing
/// (a pending query, and the answer it would have replaced, are dropped), and when it closes
/// a mapped band goes at once; the view's refresh after the commit asks again.
pub fn on_composition(s: &mut BandState, composing: bool) {
    if composing {
        enter_freeze(s);
        return;
    }
    s.frozen = false;
    if s.mapped {
        s.last = None;
        s.mapped = false;
    }
}

/// Sets (or, with `None`, clears) the chooser's override.
pub fn set_override(s: &mut BandState, ranges: Option<SmallVec<[Range<usize>; 2]>>) {
    s.override_ranges = ranges;
}

/// The ranges are bytes of `buf`, each on character boundaries.
fn fits(buf: &TextBuffer, units: &[Range<usize>]) -> bool {
    let len = buf.len_bytes();
    units.iter().all(|r| {
        r.start <= r.end
            && r.end <= len
            && buf.is_char_boundary(r.start)
            && buf.is_char_boundary(r.end)
    })
}

/// The service's last answer when it is about `snap`: asked at `snap`'s version and at its
/// carets and selections (`exec_at`), or, while a composition is open, mapped through its edits
/// to `snap`'s version. Everything the band draws or says goes through here.
fn answer<'a>(s: &'a BandState, snap: &Snapshot) -> Option<&'a ExecPreview> {
    let p = s.last.as_ref()?;
    if s.last_version != snap.version {
        return None;
    }
    if s.mapped {
        return s.frozen.then_some(p);
    }
    (s.asked_at.as_ref() == Some(&exec_at(snap))).then_some(p)
}

/// What the band shows over `snap` (`None`: no band). The chooser's override wins; otherwise
/// the service's last answer, only when it is about `snap` ([`answer`]) and is not `Nothing`.
pub fn plan(s: &BandState, snap: &Snapshot) -> Option<BandPlan> {
    if let Some(units) = &s.override_ranges {
        let units: SmallVec<[Range<usize>; 2]> =
            units.iter().filter(|r| !r.is_empty()).cloned().collect();
        return (!units.is_empty() && fits(&snap.buffer, &units)).then_some(BandPlan {
            style: BandStyle::Run,
            units,
            from_override: true,
            choices: 0,
            as_one: 0,
        });
    }
    let p = answer(s, snap)?;
    let style = match p.state {
        PreviewState::Units | PreviewState::AskPreselected => BandStyle::Run,
        PreviewState::Uncertain => BandStyle::Uncertain,
        PreviewState::NoConnection => BandStyle::Offline,
        PreviewState::Nothing => return None,
    };
    let units: SmallVec<[Range<usize>; 2]> =
        p.ranges.iter().filter(|r| !r.is_empty()).cloned().collect();
    (!units.is_empty() && fits(&snap.buffer, &units)).then_some(BandPlan {
        style,
        units,
        from_override: false,
        choices: if p.state == PreviewState::AskPreselected {
            p.choices
        } else {
            0
        },
        // ## m3c-t34-editor (named exception)
        as_one: if p.state == PreviewState::Units {
            p.as_one
        } else {
            0
        },
    })
}

/// What the band of `v` shows at its current text (`None`: no band): [`plan`] over the view's
/// state, for the console host and the tests.
pub fn current(v: &EditorView) -> Option<BandPlan> {
    plan(&v.band, &v.model.snapshot())
}

/// The status bar's sentence for the preview at `snap` (`Nothing`: "Nothing to run here";
/// `Uncertain`: "Boundary uncertain: select the statement to run it"); `None` otherwise or
/// when the answer is not of `snap`'s version. Task 34's editor segment reads it (on
/// `EditorEvent::StatusChanged` and every repaint).
pub fn status_message(v: &EditorView, cx: &App) -> Option<SharedString> {
    let s = &v.band;
    if s.override_ranges.is_some() {
        return None;
    }
    let id = match answer(s, &v.model.snapshot())?.state {
        PreviewState::Nothing => "hv-editor-nothing-to-run",
        PreviewState::Uncertain => "hv-editor-uncertain",
        _ => return None,
    };
    Some(crate::commands::message(cx, id, &[]))
}

/// The lines (0-based, end exclusive) of the statement Run would execute at the caret, for the
/// scrollbar marker: from the first unit's first line to the last unit's last line.
pub fn current_statement(s: &BandState, snap: &Snapshot) -> Option<Range<u32>> {
    let p = plan(s, snap)?;
    let first = p.units.first()?;
    let last = p.units.last()?;
    let (a, b) = unit_lines(&snap.buffer, first.start..last.end);
    Some(a..b + 1)
}

/// The first and last line of the non-empty range `r` (a range that ends at a line's start does
/// not reach that line).
pub(crate) fn unit_lines(buf: &TextBuffer, r: Range<usize>) -> (u32, u32) {
    let line = |b: usize| u32::try_from(buf.byte_to_line(b)).unwrap_or(u32::MAX);
    let a = line(r.start);
    let b = line(r.end.saturating_sub(1).max(r.start));
    (a, b.max(a))
}

/// One gutter bar of the band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutterBar {
    /// Lines, end exclusive.
    pub lines: Range<u32>,
    pub style: BandStyle,
    /// One of several units: a 2 px gap from its neighbour.
    pub gap: bool,
    /// An executed (green) bar is already at x = 4: this one moves to x = 2.
    pub shifted: bool,
}

/// A hint at the end of a unit's first line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewHint {
    /// "2 of 3": unit `i` (1-based) of `n`.
    Unit { i: usize, n: usize },
    /// "boundary uncertain", with the ink alert glyph.
    Uncertain,
    /// "not connected", with the unplug glyph.
    Offline,
    // ## m3a-t33-exec-ui (named exception)
    /// "2 choices", with the chevrons-up-down glyph: Run would open the chooser.
    Choices { n: u32 },
    // ## m3c-t34-editor
    /// "as one · 2 statements", with the layers glyph: Run sends the selection as one unit.
    AsOne { n: u32 },
}

/// One rectangle of the band, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BandRect {
    pub bounds: Bounds<Pixels>,
    pub style: BandStyle,
    /// The exact tint on a shared line (not the full row).
    pub exact: bool,
    /// The first line of a unit after the first: a 1 px `panel` line on its top edge.
    pub separator: bool,
}

/// What the gutter and the end-of-line hints take from the band over one frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BandFrame {
    pub bars: SmallVec<[GutterBar; 2]>,
    pub hints: SmallVec<[(u32, PreviewHint); 2]>,
}

/// Whether `s` (text outside a unit on its line) holds only blanks, after an optional leading
/// terminator when `after` is set.
/// Scanned in place (no copy of a long line), stopping at the first other character.
fn blank_outside(buf: &TextBuffer, r: Range<usize>, after: bool) -> bool {
    let mut terminator = after;
    for chunk in buf.chunks_in(r) {
        for c in chunk.chars() {
            if c.is_whitespace() {
                continue;
            }
            if terminator && c == ';' {
                terminator = false;
                continue;
            }
            return false;
        }
    }
    true
}

/// The first byte of `r` that is not blank (`r.end` when all of it is).
fn first_non_blank(buf: &TextBuffer, r: Range<usize>) -> usize {
    let mut at = r.start;
    for chunk in buf.chunks_in(r.clone()) {
        for (i, c) in chunk.char_indices() {
            if !c.is_whitespace() {
                return at + i;
            }
        }
        at += chunk.len();
    }
    r.end
}

/// How far back a unit's end is trimmed of blanks and its terminator.
const TRIM_WINDOW: usize = 256;

/// `r.end` less the blanks, one `;` and the blanks before it at its end (looking at most
/// [`TRIM_WINDOW`] bytes back, so a long line is never copied whole).
fn trim_terminator(buf: &TextBuffer, r: Range<usize>) -> usize {
    let lo = buf
        .snap(r.end.saturating_sub(TRIM_WINDOW).max(r.start), Bias::Before)
        .max(r.start);
    let tail = buf.slice(lo..r.end);
    let t = tail.trim_end();
    let t = t.strip_suffix(';').unwrap_or(t).trim_end();
    lo + t.len()
}

/// Builds the band's rectangles for this frame into `s` and returns its gutter bars and hints
/// (called in prepaint, after the frame's layout is known).
pub(crate) fn layout(
    s: &mut BandState,
    marks: &MarkStore,
    layout: &FrameLayout,
    snap: &Snapshot,
) -> BandFrame {
    s.rects.clear();
    s.rects_version = Some(snap.version);
    let mut out = BandFrame::default();
    let Some(plan) = plan(s, snap) else {
        return out;
    };
    let buf = &snap.buffer;
    let rows = layout.rows();
    let area = rows.bounds;
    let lh = rows.line_height;
    let n = plan.units.len();
    let multi = n > 1;
    for (ui, u) in plan.units.iter().enumerate() {
        let (l0, l1) = unit_lines(buf, u.clone());
        out.bars.push(GutterBar {
            lines: l0..l1 + 1,
            style: plan.style,
            gap: multi,
            shifted: marks.executed_bar_on(buf, l0..l1 + 1),
        });
        let hint = match plan.style {
            BandStyle::Uncertain => Some(PreviewHint::Uncertain),
            BandStyle::Offline => Some(PreviewHint::Offline),
            // ## m3a-t33-exec-ui (named exception): the chooser's count on the first unit.
            BandStyle::Run if ui == 0 && plan.choices > 1 => {
                Some(PreviewHint::Choices { n: plan.choices })
            }
            // ## m3c-t34-editor (named exception): "as one · 2 statements" on its first line.
            BandStyle::Run if ui == 0 && plan.as_one > 1 => {
                Some(PreviewHint::AsOne { n: plan.as_one })
            }
            BandStyle::Run if multi => Some(PreviewHint::Unit { i: ui + 1, n }),
            BandStyle::Run => None,
        };
        if let Some(h) = hint {
            out.hints.push((l0, h));
        }
        let first = l0.max(rows.first_line);
        let last = l1.min(rows.last_line);
        // ## m3c-t32-fold (named exception, fix round 1 M7): the painted rows only, never the
        // hidden lines of a folded unit one by one.
        for row in layout.rows_in(first, last) {
            let line = row.line;
            let lr = buf.line_range(line as usize, false);
            let left_out =
                line == l0 && !blank_outside(buf, lr.start..u.start.max(lr.start), false);
            let right_out =
                line == l1 && u.end < lr.end && !blank_outside(buf, u.end..lr.end, true);
            if !left_out && !right_out {
                s.rects.push(BandRect {
                    bounds: Bounds::new(point(area.left(), row.top), size(area.size.width, lh)),
                    style: plan.style,
                    exact: false,
                    separator: multi && ui > 0 && line == l0,
                });
                continue;
            }
            // The exact tint: the unit's characters on this line, blanks and the terminator
            // trimmed off its end.
            let c0 = if line == l0 {
                u.start.max(lr.start)
            } else {
                first_non_blank(buf, lr.clone())
            };
            let c1 = if line == l1 {
                trim_terminator(buf, c0..u.end.min(lr.end))
            } else {
                lr.end
            };
            if c1 <= c0 {
                continue;
            }
            let (Some(x0), Some(x1)) = (layout.x_for_offset(c0), layout.x_for_offset(c1)) else {
                continue;
            };
            if x1 > x0 {
                s.rects.push(BandRect {
                    bounds: Bounds::new(point(x0, row.top), size(x1 - x0, lh)),
                    style: plan.style,
                    exact: true,
                    separator: false,
                });
            }
        }
    }
    out
}

/// The band's inks: the `blue_tint` fill at .55 (dark) or .8 (light) of its alpha, the neutral
/// `hover` fill, the separator's `panel`, the uncertain tint's dashed `text_3` edge.
pub(crate) struct BandInks {
    pub run: Hsla,
    pub neutral: Hsla,
    pub separator: Hsla,
    pub dash: Hsla,
}

impl BandInks {
    pub fn of(cx: &App) -> Self {
        let theme = cx
            .try_global::<Theme>()
            .copied()
            .unwrap_or_else(|| Theme::new(ThemeMode::Dark));
        let c = &theme.tokens.colors;
        let mut run = c.blue_tint.to_hsla();
        run.a *= match theme.mode {
            ThemeMode::Dark => 0.55,
            ThemeMode::Light => 0.8,
        };
        BandInks {
            run,
            neutral: c.hover.to_hsla(),
            separator: c.panel.to_hsla(),
            dash: c.text_3.to_hsla(),
        }
    }
}

/// Paints the band built for this frame (clipped to the text area by the caller). Nothing when
/// the rectangles are of another version than the frame's text.
pub fn paint_band(
    p: &BandState,
    rows: &VisibleRows,
    snap: &Snapshot,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = rows;
    if p.rects.is_empty() || p.rects_version != Some(snap.version) {
        return;
    }
    let inks = BandInks::of(cx);
    for r in &p.rects {
        let color = match r.style {
            BandStyle::Run => inks.run,
            BandStyle::Uncertain | BandStyle::Offline => inks.neutral,
        };
        window.paint_quad(fill(r.bounds, color));
        if r.separator {
            window.paint_quad(fill(
                Bounds::new(r.bounds.origin, size(r.bounds.size.width, px(1.0))),
                inks.separator,
            ));
        }
        if r.exact && r.style == BandStyle::Uncertain {
            crate::gutter::dashed_hline(
                window,
                point(r.bounds.left(), r.bounds.bottom() - px(1.0)),
                r.bounds.size.width,
                inks.dash,
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init, reason = "byte ranges as data")]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, div, px};
    use hacivat_text::{ChangeDesc, TokenOracle};
    use hacivat_ui_core::gpui_surface::{IntoElement, ParentElement as _, Render, Styled as _};
    use smallvec::smallvec;

    use super::*;
    use crate::config::{DocContext, DocId, EditorConfig};
    use crate::service::{
        DecorationSource, InvalidateFn, LanguageService, NoTokens, StatementSpan,
    };

    /// A service whose preview is the statement around each caret (split on `;`), at every
    /// version it is asked: what a fresh analysis answers.
    struct Statements;

    impl LanguageService for Statements {
        fn open(&self, _: DocId, _: &Snapshot, _: DocContext, _: &mut App) {}
        fn set_context(&self, _: DocId, _: DocContext, _: &mut App) {}
        fn change(&self, _: DocId, _: &Snapshot, _: &ChangeDesc, _: &mut App) {}
        fn close(&self, _: DocId) {}
        fn on_update(&self, _: DocId, _: InvalidateFn) {}
        fn statements(&self, _: DocId, _: u64, _: Range<usize>) -> Arc<[StatementSpan]> {
            Arc::from(Vec::new())
        }
        fn exec_preview(&self, _: DocId, _: u64, _: &ExecAt, _: ExecAction) -> Option<ExecPreview> {
            Some(ExecPreview {
                ranges: smallvec![0..8],
                state: PreviewState::Units,
                choices: 0,
                as_one: 0,
            })
        }
        fn token_oracle(&self, _: DocId) -> Rc<dyn TokenOracle> {
            Rc::new(NoTokens)
        }
        fn decoration_sources(&self, _: DocId) -> SmallVec<[Rc<dyn DecorationSource>; 2]> {
            SmallVec::new()
        }
    }

    struct Host {
        editor: Entity<EditorView>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(800.0)).h(px(300.0)).child(self.editor.clone())
        }
    }

    fn open<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (Entity<EditorView>, &'a mut VisualTestContext) {
        let slot: Rc<RefCell<Option<Entity<EditorView>>>> = Rc::default();
        let out = slot.clone();
        let text = text.to_string();
        let (_, vcx) = cx.add_window_view(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorView::new(
                    DocId(28),
                    &text,
                    EditorConfig::default(),
                    Rc::new(Statements),
                    Rc::new(|_: &App| "Console 1".into()),
                    window,
                    cx,
                )
            });
            *out.borrow_mut() = Some(editor.clone());
            Host { editor }
        });
        let editor = slot.borrow().clone().unwrap();
        (editor, vcx)
    }

    fn settle(vcx: &mut VisualTestContext) {
        vcx.executor().advance_clock(PREVIEW_DELAY);
        vcx.run_until_parked();
    }

    fn band(e: &Entity<EditorView>, vcx: &VisualTestContext) -> Option<BandPlan> {
        vcx.read(|cx| {
            let v = e.read(cx);
            plan(&v.band, &v.model.snapshot())
        })
    }

    /// The chooser's hover wins over the service; an open composition freezes the band at its
    /// last resolved range (mapped through the composition's edits, never asked again), and the
    /// band asks again after the commit.
    #[gpui::test]
    fn chooser_override_wins_and_composition_freezes(cx: &mut TestAppContext) {
        let (e, vcx) = open(cx, "SELECT 1; SELECT 2;");
        e.update(vcx, |v, cx| v.select_range(3..3, cx));
        settle(vcx);
        assert_eq!(band(&e, vcx).map(|p| p.units.to_vec()), Some(vec![0..8]));
        // The chooser hovers the second statement: its range wins.
        e.update(vcx, |v, cx| {
            v.set_preview_override(Some(smallvec![10..18]), cx)
        });
        let p = band(&e, vcx).unwrap();
        assert_eq!((p.units.to_vec(), p.from_override), (vec![10..18], true));
        e.update(vcx, |v, cx| v.set_preview_override(None, cx));
        assert_eq!(band(&e, vcx).map(|p| p.units.to_vec()), Some(vec![0..8]));

        // A composition opens at offset 3 and marks "xyz" there.
        e.update(vcx, |v, cx| {
            v.composition_changed(true, cx);
            let now = cx.background_executor().now();
            let c = v
                .ime
                .mark(&mut v.model, None, "xyz", None, now)
                .expect("the mark applies");
            v.apply_committed(c, hacivat_text::UserEvent::InputTypeCompose, cx);
        });
        settle(vcx);
        let frozen = vcx.read(|cx| e.read(cx).band.is_pending());
        assert!(!frozen, "no query while composing");
        // The band grew with the marked text (a container range): 0..11, never re-asked.
        assert_eq!(band(&e, vcx).map(|p| p.units.to_vec()), Some(vec![0..11]));
        // The composition ends: the mapped band goes at once (nobody resolved it), and the band
        // asks again (the service answers 0..8 here). Review round 1, Critical 2.
        e.update(vcx, |v, cx| {
            v.ime.unmark(&mut v.model);
            v.composition_changed(false, cx);
        });
        assert_eq!(band(&e, vcx), None, "the mapped band after the composition");
        assert!(vcx.read(|cx| e.read(cx).band.is_pending()));
        settle(vcx);
        assert_eq!(band(&e, vcx).map(|p| p.units.to_vec()), Some(vec![0..8]));
    }

    /// Each of the two guards on a mapped answer holds on its own: `plan` never draws one outside
    /// a freeze, and the end of a composition drops it. Review round 1, Critical 2.
    #[test]
    fn a_mapped_answer_is_drawn_only_while_frozen() {
        let snap = Snapshot {
            buffer: TextBuffer::from_str("SELECT 1; SELECT 2;"),
            selection: hacivat_text::SelectionSet::single(hacivat_text::SelectionRange {
                anchor: 3,
                head: 3,
                goal_x: None,
                assoc: Bias::After,
            }),
            version: 4,
        };
        let mut s = BandState {
            last: Some(ExecPreview {
                ranges: smallvec![0..11],
                state: PreviewState::Units,
                choices: 0,
                as_one: 0,
            }),
            last_version: 4,
            asked_at: Some(ExecAt::Carets(smallvec![1])),
            mapped: true,
            frozen: true,
            ..BandState::default()
        };
        assert!(plan(&s, &snap).is_some(), "frozen: the mapped band");
        s.frozen = false;
        assert_eq!(plan(&s, &snap), None, "a mapped answer outside a freeze");
        s.frozen = true;
        on_composition(&mut s, false);
        assert!(s.last.is_none() && !s.mapped && !s.frozen);
    }

    /// Typing, then composing within 50 ms: the answer was not of the text the composition began
    /// on, so it is never mapped into a band, whichever comes first, the freeze or the first
    /// marked text. Review round 1, Critical 2.
    #[gpui::test]
    fn a_composition_begun_while_a_query_is_pending_shows_no_band(cx: &mut TestAppContext) {
        use gpui::EntityInputHandler as _;
        for freeze_first in [true, false] {
            let (e, vcx) = open(cx, "SELECT 1; SELECT 2;");
            e.update(vcx, |v, cx| v.select_range(3..3, cx));
            settle(vcx);
            assert!(band(&e, vcx).is_some());
            e.update_in(vcx, |v, window, cx| {
                v.replace_text_in_range(None, "q", window, cx);
            });
            assert!(vcx.read(|cx| e.read(cx).band.is_pending()));
            e.update(vcx, |v, cx| {
                if freeze_first {
                    v.composition_changed(true, cx);
                }
                let now = cx.background_executor().now();
                let c = v
                    .ime
                    .mark(&mut v.model, None, "xyz", None, now)
                    .expect("the mark applies");
                v.apply_committed(c, hacivat_text::UserEvent::InputTypeCompose, cx);
                if !freeze_first {
                    v.composition_changed(true, cx);
                }
            });
            assert_eq!(band(&e, vcx), None, "freeze first: {freeze_first}");
            settle(vcx);
            assert_eq!(band(&e, vcx), None, "freeze first: {freeze_first}, settled");
        }
    }
}
