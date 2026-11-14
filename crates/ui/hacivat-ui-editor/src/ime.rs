//! The platform input handler (UI-EDIT-12 view side; Task 27): UTF-16 ranges through the
//! buffer, marked text through `ImeCore`, `bounds_for_range` from `last_layout()`, the blur
//! rule, and `notify_text_input_changed` only after non-IME edits (R-M3-7).
//!
//! Task 1b (working minimal): `replace_text_in_range` inserts plain text at the range (or the
//! selection) through `dispatch`; a read-only editor accepts no text; marked text is not
//! supported yet (`replace_and_mark_text_in_range` does nothing, `marked_text_range` is
//! `None`); no bounds are known.

use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use hacivat_text::{Annotations, ChangeSet, Origin, SelectionSet, Transaction, UserEvent};
use hacivat_ui_core::gpui_surface::{
    Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window,
};

use crate::view::EditorView;

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
        self.ime.marked_utf16(&self.model.snapshot().buffer)
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
