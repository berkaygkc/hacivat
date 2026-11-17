//! Find in the editor (UI-EDIT-22 without replace; M3c Task 33): the matcher seam, the find
//! state and what the view's find methods do.
//!
//! The matcher is built by the app (R-M3-10: Turkish-tolerant `[iIİı]` and accent classes over
//! the original text, in `console/find_bar.rs`); the editor only takes the compiled
//! [`FindMatcher`] and steps through its matches.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). `EditorView::{set_find, find_step,
//! select_all_matches, find_status}` delegate here in one line (P11), and the fan-out tells
//! this module of every commit ([`on_commit`]).
//!
//! ## m3c-t33-find
//!
//! - **Counting never runs on the UI thread.** [`set_find`] and every text change start a
//!   count on the background executor: the text is flattened there once per version (the
//!   matcher reads one `&str`; the `regex` crate has no rope cursor and no manifest may add
//!   one), then searched in batches of [`BATCH_BYTES`]; each batch is handed back to the view
//!   ("3 of 1,240+") until the end. A newer query or text drops the older count (a generation
//!   number; its task is dropped too). The matches are kept in order, at most [`MAX_KEPT`]
//!   (the outlines, the scrollbar ticks and the steps use them); counting goes on past that.
//! - **Steps** ([`find_step`]) select the next (previous) match after the main range, wrapping
//!   around; a main range that is itself a match steps past it. They use the kept matches once
//!   the count is complete, else ask the matcher directly over the version's text (built here
//!   on first need, then shared with the count). A step into a fold opens it (every selection
//!   commit reveals the selection, `fold_gutter::on_commit`).
//! - **Select all matches** ([`select_all_matches`]) selects at most `cap` of them as one
//!   selection change; past the cap the status bar and the live region say "Selected the first
//!   10,000 of 41,208 matches" (`hv-find-capped`).
//! - **Paint** ([`paint`]): every kept match on the painted rows but the main selection gets a
//!   1 px `text-3` outline, no fill (NOTES M3c-10); the current match is the selection.
//! - The view emits `EditorEvent::FindChanged` whenever the status may have changed (a count
//!   batch, a step, a new query), so the host's bar re-reads [`status`].

use std::ops::Range;
use std::sync::Arc;

use hacivat_text::{Committed, SelectionRange, SelectionSet, TextBuffer, UserEvent};
use hacivat_ui::l10n::UiArg;
use hacivat_ui_core::gpui_surface::{App, Context, Hsla, Task, Window, fill, hsla, px};

use crate::commands;
use crate::element::FrameLayout;
use crate::view::{EditorEvent, EditorView};

/// A compiled find query over the document's text.
// ## m3c-t33-find (named exception to Task 1b's seam): `Send + Sync`, so the count runs on the
// background executor (`set_find` takes an `Arc`).
pub trait FindMatcher: Send + Sync {
    /// The first match starting at or after byte `from`.
    fn find(&self, hay: &str, from: usize) -> Option<Range<usize>>;
    /// The last match ending at or before byte `before`.
    fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>>;
}

/// What the find panel shows ("3 of 41").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FindStatus {
    /// The current match (the main selection is a kept match), 1-based.
    pub current: Option<u32>,
    /// The match count; `None` while counting.
    pub total: Option<u32>,
    /// The count stopped at the cap.
    pub capped: bool,
    // ## m3c-t33-find
    /// The matches counted so far ("1,240+" while counting).
    pub counted: u32,
    /// A query is set (`set_find(Some(..))`).
    pub active: bool,
}

/// The most matches the state keeps (ALG UI-EDIT-22 step 11: highlighting stops at 100,000).
pub const MAX_KEPT: usize = 100_000;
/// The text one count batch searches (ALG UI-EDIT-22 step 4: chunks of 1 MB).
pub const BATCH_BYTES: usize = 1 << 20;

/// The find module's state in `EditorView` (P11).
#[derive(Default)]
pub struct FindState {
    pub(crate) matcher: Option<Arc<dyn FindMatcher>>,
    pub(crate) status: FindStatus,
    // ## m3c-t33-find
    /// Bumped by every new query and text: an older count's batches are dropped.
    generation: u64,
    /// The text version the matches are for.
    version: u64,
    /// The text of `text.0` as one string (built once per version).
    text: Option<(u64, Arc<str>)>,
    /// The matches found so far, in order, at most [`MAX_KEPT`].
    matches: Vec<Range<usize>>,
    /// The count reached the end of the text.
    done: bool,
    /// The running count (dropping it cancels it).
    job: Option<Task<()>>,
}

impl std::fmt::Debug for FindState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FindState")
            .field("active", &self.matcher.is_some())
            .field("status", &self.status)
            .field("kept", &self.matches.len())
            .field("done", &self.done)
            .finish()
    }
}

impl FindState {
    /// Every match is kept and the count is complete: the kept list answers every step.
    fn complete(&self) -> bool {
        self.done && !self.status.capped
    }
}

/// The buffer's text as one string.
fn flatten(buf: &TextBuffer) -> Arc<str> {
    let mut s = String::with_capacity(buf.len_bytes());
    for chunk in buf.chunks_in(0..buf.len_bytes()) {
        s.push_str(chunk);
    }
    Arc::from(s)
}

/// The next character boundary after `at` (`None` at the end).
fn after(hay: &str, at: usize) -> Option<usize> {
    let c = hay.get(at..)?.chars().next()?;
    Some(at + c.len_utf8())
}

/// The next non-empty match at or after `from` (an empty match is passed over).
fn next_match(m: &dyn FindMatcher, hay: &str, mut from: usize) -> Option<Range<usize>> {
    loop {
        let r = m.find(hay, from)?;
        if r.end > r.start && r.end <= hay.len() {
            return Some(r);
        }
        from = after(hay, r.start.max(from))?;
    }
}

/// One count batch: the matches from `from` on, until the batch has searched [`BATCH_BYTES`].
/// Answers them, where the next batch starts and whether the text ended.
fn scan(m: &dyn FindMatcher, hay: &str, from: usize) -> (Vec<Range<usize>>, usize, bool) {
    let limit = from.saturating_add(BATCH_BYTES);
    let mut found = Vec::new();
    let mut at = from;
    while let Some(r) = next_match(m, hay, at) {
        at = r.end;
        let past = r.start >= limit;
        found.push(r);
        if past {
            return (found, at, false);
        }
    }
    (found, hay.len(), true)
}

/// The text of the current version, built now when no count built it yet.
fn text_now(v: &mut EditorView) -> Arc<str> {
    let version = v.model.version();
    if let Some((at, text)) = &v.find.text
        && *at == version
    {
        return text.clone();
    }
    let text = flatten(v.model.buffer());
    v.find.text = Some((version, text.clone()));
    text
}

/// Starts counting the current query over the current text (dropping any older count).
fn recount(v: &mut EditorView, cx: &mut Context<EditorView>) {
    v.find.generation = v.find.generation.wrapping_add(1);
    v.find.matches.clear();
    v.find.done = false;
    v.find.version = v.model.version();
    v.find.status = FindStatus {
        active: v.find.matcher.is_some(),
        ..FindStatus::default()
    };
    v.find.job = None;
    let Some(matcher) = v.find.matcher.clone() else {
        return;
    };
    let generation = v.find.generation;
    let version = v.find.version;
    let ready = v
        .find
        .text
        .as_ref()
        .filter(|(at, _)| *at == version)
        .map(|(_, t)| t.clone());
    let buf = v.model.buffer().clone();
    v.find.job = Some(cx.spawn(async move |this, cx| {
        let text = match ready {
            Some(text) => text,
            None => {
                let text = cx
                    .background_executor()
                    .spawn(async move { flatten(&buf) })
                    .await;
                let shared = text.clone();
                let alive = this.update(cx, |v, _| {
                    if v.find.generation == generation && v.find.text.is_none() {
                        v.find.text = Some((version, shared));
                    }
                    v.find.generation == generation
                });
                if !matches!(alive, Ok(true)) {
                    return;
                }
                text
            }
        };
        let mut from = 0;
        loop {
            let (m, t) = (matcher.clone(), text.clone());
            let (found, next, done) = cx
                .background_executor()
                .spawn(async move { scan(m.as_ref(), &t, from) })
                .await;
            let go_on = this.update(cx, |v, cx| on_batch(v, generation, found, done, cx));
            if done || !matches!(go_on, Ok(true)) {
                return;
            }
            from = next;
        }
    }));
}

/// A count batch landed: keeps its matches (up to the cap) and tells the host. `false` when
/// the count is stale.
fn on_batch(
    v: &mut EditorView,
    generation: u64,
    found: Vec<Range<usize>>,
    done: bool,
    cx: &mut Context<EditorView>,
) -> bool {
    if v.find.generation != generation {
        return false;
    }
    let st = &mut v.find;
    st.status.counted = st
        .status
        .counted
        .saturating_add(u32::try_from(found.len()).unwrap_or(u32::MAX));
    let room = MAX_KEPT.saturating_sub(st.matches.len());
    if found.len() > room {
        st.status.capped = true;
    }
    st.matches.extend(found.into_iter().take(room));
    if done {
        st.done = true;
        st.status.total = Some(st.status.counted);
        st.job = None;
    }
    cx.emit(EditorEvent::FindChanged);
    cx.notify();
    !done
}

/// Sets (or clears, `None`) the query and starts counting it.
pub(crate) fn set_find(
    v: &mut EditorView,
    m: Option<Arc<dyn FindMatcher>>,
    cx: &mut Context<EditorView>,
) {
    v.find.matcher = m;
    recount(v, cx);
    cx.emit(EditorEvent::FindChanged);
    cx.notify();
}

/// The kept match after (`back`: before) the main range `r`, wrapping; a range that is itself
/// a match steps past it.
fn step_in(matches: &[Range<usize>], r: &Range<usize>, back: bool) -> Option<Range<usize>> {
    if matches.is_empty() {
        return None;
    }
    let at = matches.partition_point(|m| m.start < r.start);
    let on_match = matches.get(at).is_some_and(|m| m == r);
    let i = if back {
        at.checked_sub(1).unwrap_or(matches.len() - 1)
    } else if on_match {
        (at + 1) % matches.len()
    } else if at < matches.len() {
        at
    } else {
        0
    };
    matches.get(i).cloned()
}

/// [`step_in`] by asking the matcher over `hay`.
fn step_direct(
    m: &dyn FindMatcher,
    hay: &str,
    r: &Range<usize>,
    back: bool,
) -> Option<Range<usize>> {
    if back {
        let before = |at: usize| {
            let mut found = m.find_back(hay, at);
            // A match ending inside the range that starts before it is fine; one that is the
            // range itself is not.
            while let Some(f) = found.clone() {
                if f.start < r.start && f.end > f.start {
                    return Some(f);
                }
                found = f.start.checked_sub(1).and_then(|b| m.find_back(hay, b));
            }
            None
        };
        before(r.start).or_else(|| {
            let last = m.find_back(hay, hay.len())?;
            (last.end > last.start).then_some(last)
        })
    } else {
        let mut first = next_match(m, hay, r.start);
        if first.as_ref() == Some(r) {
            first = next_match(m, hay, r.end);
        }
        first.or_else(|| next_match(m, hay, 0))
    }
}

/// Selects the next (`back`: previous) match from the main range and answers it; `None` (the
/// selection unchanged) when there is none.
pub(crate) fn find_step(
    v: &mut EditorView,
    back: bool,
    cx: &mut Context<EditorView>,
) -> Option<Range<usize>> {
    let m = v.find.matcher.clone()?;
    let r = v.model.selection().main().range();
    let found = if v.find.complete() && v.find.version == v.model.version() {
        step_in(&v.find.matches, &r, back)
    } else {
        let hay = text_now(v);
        step_direct(m.as_ref(), &hay, &r, back)
    }?;
    commands::select(
        v,
        SelectionRange::new(found.start, found.end),
        UserEvent::Select,
        cx,
    );
    cx.emit(EditorEvent::FindChanged);
    Some(found)
}

/// Selects every match (at most `cap`, as ranges of a multi-cursor selection) and answers how
/// many. Past the cap it says how many of how many it selected. The main range is the match
/// at or after the main caret.
pub(crate) fn select_all_matches(
    v: &mut EditorView,
    cap: u32,
    cx: &mut Context<EditorView>,
) -> u32 {
    let Some(m) = v.find.matcher.clone() else {
        return 0;
    };
    let cap = (cap.max(1)) as usize;
    let (ranges, total): (Vec<Range<usize>>, u64) =
        if v.find.complete() && v.find.version == v.model.version() {
            let all = &v.find.matches;
            (all[..all.len().min(cap)].to_vec(), all.len() as u64)
        } else {
            // The count has not finished: the matches are collected here (UI-EDIT-22's rare
            // path; a select all while counting a large text).
            let hay = text_now(v);
            let mut kept = Vec::new();
            let mut total = 0u64;
            let mut at = 0;
            while let Some(r) = next_match(m.as_ref(), &hay, at) {
                at = r.end;
                total += 1;
                if kept.len() < cap {
                    kept.push(r);
                }
            }
            (kept, total)
        };
    if ranges.is_empty() {
        return 0;
    }
    let caret = v.model.selection().main().range().start;
    let main = ranges
        .partition_point(|r| r.start < caret)
        .min(ranges.len() - 1);
    let n = ranges.len();
    let set = SelectionSet::normalized(
        ranges
            .into_iter()
            .map(|r| SelectionRange::new(r.start, r.end)),
        main,
    );
    commands::select_set(v, set, UserEvent::Select, cx);
    if total > n as u64 {
        say_capped(v, n as u64, total, cx);
    }
    cx.emit(EditorEvent::FindChanged);
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Says "Selected the first `max` of `n` matches" (the status bar and the live region).
pub(crate) fn say_capped(v: &mut EditorView, max: u64, n: u64, cx: &mut Context<EditorView>) {
    let text = commands::message(
        cx,
        "hv-find-capped",
        &[("max", UiArg::Count(max)), ("n", UiArg::Count(n))],
    );
    commands::say(v, text, cx);
}

/// The panel's status: the count, and the current match when the main selection is one.
pub(crate) fn status(v: &EditorView) -> FindStatus {
    let mut s = v.find.status;
    if s.active && v.find.version == v.model.version() {
        let r = v.model.selection().main().range();
        let at = v.find.matches.partition_point(|m| m.start < r.start);
        if v.find.matches.get(at) == Some(&r) {
            s.current = u32::try_from(at + 1).ok();
        }
    }
    s
}

/// The kept matches over the bytes `range` (the scrollbar's ticks and the outlines).
pub(crate) fn matches_in(v: &EditorView, range: Range<usize>) -> &[Range<usize>] {
    if v.find.matcher.is_none() || v.find.version != v.model.version() {
        return &[];
    }
    let m = &v.find.matches;
    let from = m.partition_point(|r| r.end <= range.start);
    let to = m.partition_point(|r| r.start < range.end).max(from);
    &m[from..to]
}

/// Every commit, from the fan-out (`EditorView::apply_committed`): a text change recounts; any
/// commit may move the current match.
// ## m3c-t33-find (named exception to Task 1b's seam): takes the view's context (the recount
// spawns its task).
pub(crate) fn on_commit(
    v: &mut EditorView,
    committed: &Committed,
    text_changed: bool,
    cx: &mut Context<EditorView>,
) {
    let _ = committed;
    if v.find.matcher.is_none() {
        return;
    }
    if text_changed {
        v.find.text = None;
        recount(v, cx);
    }
    cx.emit(EditorEvent::FindChanged);
}

/// The 1 px `text-3` outline of every kept match on the painted rows, but the main selection
/// (the current match is the selection, NOTES M3c-10).
pub(crate) fn paint(v: &EditorView, layout: &FrameLayout, window: &mut Window, cx: &App) {
    if v.find.matcher.is_none() || layout.version() != v.find.version {
        return;
    }
    let rows = layout.rows();
    let buf = &layout.buffer;
    let lo = buf.line_to_byte(rows.first_line as usize);
    let next = rows.last_line as usize + 1;
    let hi = if next < buf.len_lines() {
        buf.line_to_byte(next)
    } else {
        buf.len_bytes() + 1
    };
    let main = v.model.selection().main().range();
    let ink = crate::gutter::Inks::of(cx).text_3;
    for r in matches_in(v, lo..hi) {
        if *r == main {
            continue;
        }
        outline_range(layout, r.clone(), ink, window);
    }
}

/// Outlines `r` line by line (a match may span lines).
fn outline_range(layout: &FrameLayout, r: Range<usize>, ink: Hsla, window: &mut Window) {
    let buf = &layout.buffer;
    let first = buf.byte_to_line(r.start);
    let last = buf.byte_to_line(r.end.max(r.start));
    for line in first..=last {
        let lr = buf.line_range(line, false);
        let piece = r.start.max(lr.start)..r.end.min(lr.end);
        if piece.start >= piece.end && first != last {
            continue;
        }
        let Some(b) = layout.bounds_for_range(piece) else {
            continue;
        };
        if b.size.width <= px(0.0) {
            continue;
        }
        window.paint_quad(
            fill(b, hsla(0.0, 0.0, 0.0, 0.0))
                .border_widths(px(1.0))
                .border_color(ink),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A literal matcher.
    struct Lit(&'static str);

    impl FindMatcher for Lit {
        fn find(&self, hay: &str, from: usize) -> Option<Range<usize>> {
            let i = hay.get(from..)?.find(self.0)?;
            Some(from + i..from + i + self.0.len())
        }
        fn find_back(&self, hay: &str, before: usize) -> Option<Range<usize>> {
            let i = hay.get(..before)?.rfind(self.0)?;
            Some(i..i + self.0.len())
        }
    }

    #[test]
    fn steps_wrap_and_pass_the_current_match() {
        let m = vec![0..1, 4..5, 8..9];
        assert_eq!(step_in(&m, &(0..0), false), Some(0..1));
        assert_eq!(step_in(&m, &(0..1), false), Some(4..5));
        assert_eq!(step_in(&m, &(8..9), false), Some(0..1));
        assert_eq!(step_in(&m, &(0..1), true), Some(8..9));
        assert_eq!(step_in(&m, &(6..6), true), Some(4..5));
        assert_eq!(step_in(&[], &(0..0), false), None);
    }

    #[test]
    fn direct_steps_agree_with_the_kept_list() {
        let hay = "a x a x a";
        let m = Lit("a");
        let kept = vec![0..1, 4..5, 8..9];
        for r in [0..0, 0..1, 3..3, 4..5, 8..9, 9..9, 2..6] {
            for back in [false, true] {
                assert_eq!(
                    step_direct(&m, hay, &r, back),
                    step_in(&kept, &r, back),
                    "{r:?} back={back}"
                );
            }
        }
    }

    #[test]
    fn a_batch_stops_after_its_bytes() {
        let hay = "ab ".repeat(BATCH_BYTES / 3 * 2);
        let (first, next, done) = scan(&Lit("ab"), &hay, 0);
        assert!(!done);
        assert!((BATCH_BYTES..BATCH_BYTES + 6).contains(&next), "{next}");
        let (rest, _, done) = scan(&Lit("ab"), &hay, next);
        assert!(done);
        assert_eq!(first.len() + rest.len(), BATCH_BYTES / 3 * 2);
    }

    #[test]
    fn empty_matches_are_passed_over() {
        struct Empty;
        impl FindMatcher for Empty {
            fn find(&self, hay: &str, from: usize) -> Option<Range<usize>> {
                (from <= hay.len()).then_some(from..from)
            }
            fn find_back(&self, _: &str, before: usize) -> Option<Range<usize>> {
                Some(before..before)
            }
        }
        assert_eq!(scan(&Empty, "ışık", 0), (Vec::new(), "ışık".len(), true));
    }
}
