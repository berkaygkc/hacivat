//! Multi-cursor commands (UI-EDIT-05 multi; M3b Task 31): add a caret above or below, select
//! the next or every occurrence, split the selection into lines, and the selection-only undo
//! and redo (Ctrl/⌘+U), over `hacivat_text::multi`. None edits the text.
//!
//! - **Add caret above/below** moves from the main caret one line at its goal column, measured
//!   on the painted layout when the frame shows the line (the vertical moves' geometry), and
//!   the new caret becomes the main one, so repeating walks on.
//! - **Next occurrence** from a caret first selects the word under it with the language
//!   service's words (a quoted identifier or a string is one word), then adds exact matches
//!   (`hacivat_text::select_next_occurrence_with`; whole words while the search that began at
//!   a caret goes on, a substring for a range selected by hand: UI-EDIT-05 step 5).
//! - **Undo/redo selection** hand the model's selection-only commit to `apply_committed`.

use hacivat_text::units::word_at;
use hacivat_text::{
    MAX_OCCURRENCES, SelectionRange, SelectionSet, UserEvent, add_caret_vertical_with,
    select_all_occurrences_with as all_occurrences_with,
    select_next_occurrence_with as next_occurrence_with, split_into_lines,
};
use hacivat_ui_core::gpui_surface::{Context, Window};

use super::movement::Geometry;
use super::{select_set, words};
use crate::view::EditorView;

type Cx<'a> = Context<'a, EditorView>;

fn vertical(v: &mut EditorView, down: bool, cx: &mut Cx<'_>) {
    let snap = v.model.snapshot();
    let next = {
        let geo = Geometry::of(v, &snap);
        add_caret_vertical_with(
            &snap.buffer,
            &snap.selection,
            down,
            |at| geo.x_of(at),
            |line, x| geo.offset_at(line, x),
        )
    };
    select_set(v, next, UserEvent::Select, cx);
}

pub(super) fn add_caret_above(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, false, cx);
}

pub(super) fn add_caret_below(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    vertical(v, true, cx);
}

/// The selection with the main caret widened to its word (the language service's words), or
/// `None` when the main range is not a caret or the caret has no word (blank).
fn main_widened_to_word(v: &EditorView) -> Option<Option<SelectionSet>> {
    let sel = v.model.selection();
    let main = *sel.main();
    if !main.is_empty() {
        return None;
    }
    let w = word_at(v.model.buffer(), main.head, words(v).as_ref());
    let blank = v
        .model
        .buffer()
        .slice(w.clone())
        .chars()
        .next()
        .is_none_or(char::is_whitespace);
    if blank {
        return Some(None);
    }
    let mut ranges: Vec<SelectionRange> = sel.ranges().to_vec();
    ranges[sel.main_index()] = SelectionRange::new(w.start, w.end);
    Some(Some(SelectionSet::normalized(ranges, sel.main_index())))
}

// ## m3c-t31-multi (fix round 1, M10): whole words only while the search that began at a caret
// goes on: `CommandState::whole_word_at` holds the model's commit count after the command's own
// selection, so any other selection change ends it.
/// The current search began at a caret and nothing else changed the selection since.
fn searching_whole_words(v: &EditorView) -> bool {
    v.commands.whole_word_at == Some(v.model.commit_count())
}

pub(super) fn select_next_occurrence(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    match main_widened_to_word(v) {
        Some(None) => {}
        Some(Some(word)) => {
            select_set(v, word, UserEvent::Select, cx);
            v.commands.whole_word_at = Some(v.model.commit_count());
        }
        None => {
            let whole = searching_whole_words(v);
            if let Some(next) = next_occurrence_with(v.model.buffer(), v.model.selection(), whole) {
                select_set(v, next, UserEvent::Select, cx);
                if whole {
                    v.commands.whole_word_at = Some(v.model.commit_count());
                }
            } else {
                // ## m3c-t33-find (named exception, T31 ruling 7): every occurrence is selected
                // already; said politely (UI-EDIT-05 step 5).
                let text = super::message(cx, "hv-find-no-more", &[]);
                super::say(v, text, cx);
            }
        }
    }
}

/// Fix round 1 (M2, M11): one selection change (one selection-history step), from the word of
/// a caret (whole words) or the main range. Past the cap it says "Selected the first 10,000 of
/// N matches" (`hv-find-capped`; ## m3c-t33-find).
pub(super) fn select_all_occurrences(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let (from, whole) = match main_widened_to_word(v) {
        Some(None) => return,
        Some(Some(word)) => (word, true),
        None => (v.model.selection().clone(), searching_whole_words(v)),
    };
    let (all, total) = all_occurrences_with(v.model.buffer(), &from, MAX_OCCURRENCES, whole);
    // ## m3c-t33-find (named exception, T31 ruling 7): past the cap the status bar and the live
    // region say how many of how many (UI-EDIT-05 step 6, `hv-find-capped`).
    let kept = all.ranges().len();
    select_set(v, all, UserEvent::Select, cx);
    if total > kept {
        crate::find::say_capped(v, kept as u64, total as u64, cx);
    }
}

pub(super) fn split_selection_into_lines(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let split = split_into_lines(v.model.buffer(), v.model.selection());
    select_set(v, split, UserEvent::Select, cx);
}

/// Undo (`redo`: redo) the selection change. An open composition ends first (as the text's
/// undo does); the text never changes.
fn step(v: &mut EditorView, redo: bool, cx: &mut Cx<'_>) {
    super::acted(v);
    if v.ime.is_composing() {
        v.ime.unmark(&mut v.model);
        v.composition_changed(false, cx);
    }
    let done = if redo {
        v.model.redo_selection()
    } else {
        v.model.undo_selection()
    };
    if let Some(c) = done {
        v.apply_committed(c, UserEvent::Select, cx);
    }
}

pub(super) fn undo_selection(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    step(v, false, cx);
}

pub(super) fn redo_selection(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    step(v, true, cx);
}
