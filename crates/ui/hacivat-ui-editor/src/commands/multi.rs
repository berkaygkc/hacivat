//! Multi-cursor commands (UI-EDIT-05 multi; M3b Task 31): add a caret above or below, select
//! the next or every occurrence, split the selection into lines, and the selection-only undo
//! and redo (Ctrl/⌘+U), over `hacivat_text::multi`. None edits the text.
//!
//! - **Add caret above/below** moves from the main caret one line at its goal column, measured
//!   on the painted layout when the frame shows the line (the vertical moves' geometry), and
//!   the new caret becomes the main one, so repeating walks on.
//! - **Next occurrence** from a caret first selects the word under it with the language
//!   service's words (a quoted identifier or a string is one word), then adds exact matches
//!   (`hacivat_text::select_next_occurrence`; whole words when the range is one).
//! - **Undo/redo selection** hand the model's selection-only commit to `apply_committed`.

use hacivat_text::units::word_at;
use hacivat_text::{
    MAX_OCCURRENCES, SelectionRange, UserEvent, add_caret_vertical_with,
    select_all_occurrences as all_occurrences, select_next_occurrence as next_occurrence,
    split_into_lines,
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

/// A caret's word, from the language service's words: the main caret becomes that selection.
fn expand_main_to_word(v: &mut EditorView, cx: &mut Cx<'_>) -> bool {
    let sel = v.model.selection().clone();
    let main = *sel.main();
    if !main.is_empty() {
        return false;
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
        return true;
    }
    let mut ranges: Vec<SelectionRange> = sel.ranges().to_vec();
    ranges[sel.main_index()] = SelectionRange::new(w.start, w.end);
    let set = hacivat_text::SelectionSet::normalized(ranges, sel.main_index());
    select_set(v, set, UserEvent::Select, cx);
    true
}

pub(super) fn select_next_occurrence(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    if expand_main_to_word(v, cx) {
        return;
    }
    if let Some(next) = next_occurrence(v.model.buffer(), v.model.selection()) {
        select_set(v, next, UserEvent::Select, cx);
    }
}

pub(super) fn select_all_occurrences(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    if v.model.selection().main().is_empty() {
        expand_main_to_word(v, cx);
    }
    let all = all_occurrences(v.model.buffer(), v.model.selection(), MAX_OCCURRENCES);
    select_set(v, all, UserEvent::Select, cx);
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
