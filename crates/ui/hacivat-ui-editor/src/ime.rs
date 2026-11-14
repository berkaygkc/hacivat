//! The platform input handler (UI-EDIT-12 view side; Task 27): UTF-16 ranges through the
//! buffer, marked text through `ImeCore`, `bounds_for_range` from `last_layout()`, the blur
//! rule, and `notify_text_input_changed` only after non-IME edits (R-M3-7).
//!
//! Task 1b (working minimal): `replace_text_in_range` inserts plain text at the range (or the
//! selection) through `dispatch`; a read-only editor accepts no text; marked text is not
//! supported yet (`replace_and_mark_text_in_range` does nothing, `marked_text_range` is
//! `None`); no bounds are known.
//!
//! The view's focus, blur, commit and frame call points (fix round 1) reach `on_focus`,
//! `on_blur`, `after_commit` and `on_frame` below; Task 27 fills them (`FocusedInput`, the blur
//! rule, `notify_text_input_changed` only after non-IME edits) and keeps its state in
//! [`InputState`].

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_text::{
    Annotations, ChangeSet, Committed, Origin, SelectionSet, Transaction, UserEvent,
};
use hacivat_ui_core::gpui_surface::{
    Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window,
};

use crate::view::EditorView;

/// The input handler's module-owned state (P11; Task 27 adds its fields: a text-input
/// notification owed to the platform, the `FocusedInput` bookkeeping). Keep `Default`.
#[derive(Debug, Clone, Default)]
pub struct InputState {}

/// The editor took the focus (Task 27: write `FocusedInput { entity, composing }`).
/// Task 1b: nothing.
pub(crate) fn on_focus(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}

/// The editor lost the focus (Task 27: the blur rule; an in-window focus move commits the
/// composition, an app switch keeps it). Task 1b: nothing.
pub(crate) fn on_blur(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}

/// After every `Committed`, with the `UserEvent` that made it (Task 27: owe the platform
/// `notify_text_input_changed` unless the edit came from the IME). The view calls it from
/// `apply_committed`, which has no window; Task 27 records here and notifies in `on_frame`.
/// Task 27's own marked text commits through `ImeCore::mark`/`commit` and hands the
/// `Committed` to `EditorView::apply_committed` too. Task 1b: nothing.
pub(crate) fn after_commit(
    v: &mut EditorView,
    committed: &Committed,
    event: UserEvent,
    cx: &mut Context<EditorView>,
) {
    let _ = (v, committed, event, cx);
}

/// Once per frame, at the start of the element's prepaint (`element::prepaint_frame`; Task 27:
/// send what `after_commit` owes). Task 1b: nothing.
pub(crate) fn on_frame(v: &mut EditorView, window: &mut Window, cx: &mut Context<EditorView>) {
    let _ = (v, window, cx);
}

impl EntityInputHandler for EditorView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let snap = self.model.snapshot();
        Some(self.ime.text_for_range(&snap.buffer, range, adjusted_range))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let snap = self.model.snapshot();
        let main = snap.selection.main();
        let r = main.range();
        Some(UTF16Selection {
            range: snap.buffer.byte_to_utf16(r.start)..snap.buffer.byte_to_utf16(r.end),
            reversed: main.head < main.anchor,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime.marked_utf16(&self.model)
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.ime.unmark(&mut self.model);
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.read_only {
            return;
        }
        let snap = self.model.snapshot();
        let bytes = match range {
            Some(r16) => {
                let start = snap.buffer.utf16_to_byte(r16.start);
                start..snap.buffer.utf16_to_byte(r16.end).max(start)
            }
            None => snap.selection.main().range(),
        };
        let caret = bytes.start + text.len();
        let Ok(changes) =
            ChangeSet::from_edits(snap.buffer.len_bytes(), [(bytes, Arc::from(text))])
        else {
            return;
        };
        self.commit(
            Transaction {
                changes,
                selection: Some(SelectionSet::cursor(caret)),
                ann: Annotations {
                    user_event: UserEvent::InputType,
                    add_to_history: true,
                    time: Instant::now(),
                    origin: Origin::User,
                },
            },
            cx,
        );
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Marked text arrives with Task 27; the composition stays with the platform until it
        // commits through `replace_text_in_range`.
        let _ = (range, new_text, new_selected_range);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let _ = (range_utf16, element_bounds);
        None
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let _ = point;
        None
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        !self.config.read_only
    }
}
