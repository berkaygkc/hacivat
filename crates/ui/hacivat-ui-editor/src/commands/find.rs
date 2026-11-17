//! Find commands (UI-EDIT-22 without replace; M3c Task 33): open the find bar
//! (`EditorEvent::FindRequested`), step to the next or previous match, close.
//!
//! ## m3b-t01b-seams: the rows are registered (`commands/mod.rs`).
//!
//! ## m3c-t33-find
//!
//! - `editor.find` asks the host for its bar, seeded with the main selection's text when it is
//!   on one line (UI-EDIT-22 step 1), else with nothing (the bar keeps its last query).
//! - `editor.find_next` / `find_prev` step while a query is set; without one they ask for the
//!   bar (unseeded), as `editor.find` would.
//! - `editor.find_close` clears the query (the host's bar closes on the `FindChanged` that
//!   follows, `FindStatus::active` false).

use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::{EditorEvent, EditorView};

type Cx<'a> = Context<'a, EditorView>;

/// The longest selection that seeds the query.
const SEED_MAX: usize = 1_000;

/// The main selection's text when it is non-empty, on one line and short.
fn seed(v: &EditorView) -> Option<String> {
    let r = v.model.selection().main().range();
    if r.is_empty() || r.len() > SEED_MAX {
        return None;
    }
    let text = v.model.buffer().slice(r).into_owned();
    (!text.contains(['\n', '\r'])).then_some(text)
}

pub(super) fn find(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    let seed = seed(v);
    cx.emit(EditorEvent::FindRequested { seed });
}

fn step(v: &mut EditorView, back: bool, cx: &mut Cx<'_>) {
    if v.find_status().active {
        v.find_step(back, cx);
    } else {
        cx.emit(EditorEvent::FindRequested { seed: None });
    }
}

pub(super) fn find_next(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    step(v, false, cx);
}

pub(super) fn find_prev(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    step(v, true, cx);
}

pub(super) fn find_close(v: &mut EditorView, _: &mut Window, cx: &mut Cx<'_>) {
    if v.find_status().active {
        v.set_find(None, cx);
    }
}
