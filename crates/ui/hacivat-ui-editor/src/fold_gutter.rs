//! Folding in the editor (UI-EDIT-10; M3b Task 32): the folded ranges of the document, the
//! gutter's fold toggles, and what `editor.fold`/`unfold`/`fold_all`/`unfold_all` do.
//!
//! The foldable ranges come from the language service
//! (`LanguageService::folding_ranges`); the folded set is `hacivat_text::FoldSet`, mapped
//! through every commit; its hidden lines feed the chunked height map
//! (`HeightMap::chunked`).
//!
//! ## m3b-t01b-seams: the seam (Task 1b). `EditorView::{folds, toggle_fold_at}` delegate here
//! in one line (P11), and the fan-out maps the set on every commit ([`on_commit`]), so Task 32
//! never edits `view.rs`. The stubs fold nothing, so the display stays M3a's.

use hacivat_text::{Committed, FoldSet};
use hacivat_ui_core::gpui_surface::{Context, Window};

use crate::view::EditorView;

/// The folding module's state in `EditorView` (P11).
#[derive(Debug, Default)]
pub struct FoldState {
    pub(crate) folds: FoldSet,
}

/// The folded ranges.
pub(crate) fn folds(v: &EditorView) -> &FoldSet {
    &v.fold.folds
}

/// Folds the foldable range that starts on `line` (0-based), or unfolds the fold there;
/// `true` when anything changed. STUB: `false`.
pub(crate) fn toggle_fold_at(v: &mut EditorView, line: u32, cx: &mut Context<EditorView>) -> bool {
    let _ = (v, line, cx);
    false
}

/// Every commit, from the fan-out (`EditorView::apply_committed`): the folds follow the text.
pub(crate) fn on_commit(v: &mut EditorView, committed: &Committed) {
    if committed.buffer_changed {
        v.fold.folds.map(&committed.desc);
    }
}

// ---- the folding commands (`editor.fold`, `unfold`, `fold_all`, `unfold_all`) ---------------
// ## m3b-t01b-seams: registered in `commands/mod.rs`; no-ops until Task 32 fills them (they live
// here so Task 32 never edits `commands/`).

pub(crate) fn fold(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, cx);
}

pub(crate) fn unfold(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, cx);
}

pub(crate) fn fold_all(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, cx);
}

pub(crate) fn unfold_all(v: &mut EditorView, _: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, cx);
}
