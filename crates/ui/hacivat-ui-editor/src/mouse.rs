//! Mouse selection and wheel scrolling (Task 19).
//!
//! - **Clicks** by `MouseDownEvent::click_count`: one places the caret (Shift extends the
//!   selection from its anchor), two select the word (`units::word_at` over the language
//!   service's tokens), three the line with its break. A drag extends by the unit of the click
//!   that started it, the anchor at the far edge of the first unit. Pointer hits snap to grapheme
//!   clusters (`FrameLayout::offset_for_point`), never inside a CRLF.
//! - Every selection goes through `EditorView::commit` (`UserEvent::SelectPointer`), so an open
//!   IME composition is committed first (`ImeCore::dispatch`), and the click tells the platform
//!   to drop its copy (`ime::send_owed_discard`, F4).
//! - **A drag past the text's top or bottom** scrolls by the overshoot on each move, so the
//!   selection can grow beyond the view. ## m3c-t34-editor (ED18 carry): while the pointer
//!   rests there, a timer goes on doing what a move would, once a frame
//!   ([`DRAG_AUTOSCROLL_TICK`]), until the pointer is back inside, the button is up or the
//!   editor lost the focus.
//! - **Several carets** (## m3c-t31-multi): a click with Option (macOS) or Alt adds a caret,
//!   the new main one; a drag with Option (macOS) or Ctrl+Alt selects a column, a box in
//!   display columns (`hacivat_text::column_selection`), the main range on the pointer's line.
//!   A middle click pastes the Linux primary selection at the pointer.
//! - **A right click** places the caret at the pointer (unless it falls inside the selection,
//!   which it keeps) and runs `editor.context_menu` with the pointer readable through
//!   `EditorView::context_menu_point` while it runs: Task 25 opens the menu there (at the caret
//!   when the keyboard asked).
//! - **The wheel** scrolls by pixels (or lines × the row height); Shift turns a vertical wheel
//!   sideways. The I-beam shows over the text (the cursor style on the text's hitbox).
//!
//! Keys never come here: commands arrive from the app's dispatcher and text through the input
//! handler (`ime.rs`).

use std::ops::Range;
use std::time::Duration;

use hacivat_text::units::word_at;
use hacivat_text::{
    Bias, SelectionRange, SelectionSet, Transaction, UserEvent, add_caret, column_selection,
};
use hacivat_ui_core::gpui_surface::{
    Context, CursorStyle, Entity, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, ScrollEvent, Task, Window,
};

use crate::element::FramePrepaint;
use crate::view::EditorView;

/// What a click selects, and a drag extends by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClickUnit {
    Char,
    Word,
    Line,
    // ## m3c-t31-multi: a column (box) drag.
    Column,
}

/// A drag in progress: its unit and the first unit's range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DragState {
    pub unit: ClickUnit,
    pub origin: Range<usize>,
    // ## m3c-t31-multi
    /// A column drag's anchor: (line, display column).
    pub column: Option<(u32, usize)>,
    /// The column drag reached another cell (fix round 1, I1): only then does the box
    /// replace the selection, so a press that jitters in place keeps every caret.
    pub column_started: bool,
}

// ## m3c-t34-editor
/// How often a drag resting past the text's edge scrolls on (once a frame).
pub const DRAG_AUTOSCROLL_TICK: Duration = Duration::from_millis(16);

/// The edge autoscroll of a drag: where the pointer rests, and the timer (dropping it stops it).
pub(crate) struct Autoscroll {
    at: Point<Pixels>,
    _task: Task<()>,
}

impl std::fmt::Debug for Autoscroll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Autoscroll")
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

/// The pointer is at `at` during a drag: past the edge (`over`) the timer runs (started once)
/// and follows the pointer; back inside it stops.
fn follow_edge(v: &mut EditorView, at: Point<Pixels>, over: bool, cx: &mut Context<EditorView>) {
    if !over {
        v.autoscroll = None;
        return;
    }
    if let Some(a) = v.autoscroll.as_mut() {
        a.at = at;
        return;
    }
    let task = cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(DRAG_AUTOSCROLL_TICK).await;
            let more = this
                .update(cx, |v, cx| {
                    // The drag ended (button up, blur) or the pointer came back: done.
                    let (Some(_), Some(at)) =
                        (v.drag.as_ref(), v.autoscroll.as_ref().map(|a| a.at))
                    else {
                        v.autoscroll = None;
                        return false;
                    };
                    drag(v, at, cx);
                    cx.notify();
                    v.autoscroll.is_some()
                })
                .unwrap_or(false);
            if !more {
                break;
            }
        }
    });
    v.autoscroll = Some(Autoscroll { at, _task: task });
}

// ## m3c-t31-multi (fix round 1, M2)
/// The press's gesture ended (button up, a move without it, a blur): no drag, and the
/// selection history's gesture closes.
pub(crate) fn end_drag(v: &mut EditorView) -> bool {
    v.drag = None;
    // ## m3c-t34-editor: the edge autoscroll ends with it.
    v.autoscroll = None;
    v.model.end_selection_gesture();
    // ## m3c-t32-fold (named exception, fix round 1 M5): a column box reaching into a fold
    // opens it once the drag ends, not under the pointer mid-drag.
    crate::fold_gutter::after_drag(v)
}

// ## m3c-t31-multi (UI-EDIT-05 steps 7 and 9; NOTES surface 14)
/// The column-drag modifier: Option on macOS, Ctrl+Alt elsewhere.
fn column_drag(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.alt && !m.control && !m.platform
    } else {
        m.control && m.alt && !m.platform
    }
}

/// The add-a-caret modifier: Option on macOS, Alt elsewhere (a click without a drag).
fn adds_caret(m: &Modifiers) -> bool {
    m.alt && !m.control && !m.platform
}

/// The (line, display column) under `p` on the last frame: the column from the x, so a box can
/// reach past a short line's end.
fn line_and_column(v: &EditorView, p: Point<Pixels>) -> Option<(u32, usize)> {
    let layout = v.layout.as_ref()?;
    let at = layout.clamped_offset_for_point(p)?;
    let line = u32::try_from(layout.buffer.byte_to_line(at)).ok()?;
    let advance = f64::from(layout.advance.max(1.0));
    let col = ((f64::from(f32::from(p.x)) - layout.origin_x) / advance).round();
    Some((line, col.max(0.0) as usize))
}

/// Selects the box from the drag's anchor to `head`.
fn select_column(
    v: &mut EditorView,
    anchor: (u32, usize),
    head: (u32, usize),
    cx: &mut Context<EditorView>,
) {
    let set = column_selection(v.model.buffer(), anchor, head, v.config.tab_size);
    if *v.model.selection() != set {
        commit_set(v, set, cx);
    }
}

/// Makes `set` the selection through the IME core's dispatch (`SelectPointer`).
fn commit_set(v: &mut EditorView, set: SelectionSet, cx: &mut Context<EditorView>) {
    let len = v.model.buffer().len_bytes();
    let tr = Transaction {
        changes: hacivat_text::ChangeSet::identity(len),
        selection: Some(set),
        ann: v.annotations(UserEvent::SelectPointer, hacivat_text::Origin::User, cx),
    };
    v.commit(tr, cx);
}

/// The most a drag past the edge scrolls per move, in rows.
const DRAG_SCROLL_ROWS: f32 = 3.0;

/// Registers the frame's pointer listeners (paint phase) and the I-beam over the text.
pub(crate) fn listen(view: &Entity<EditorView>, frame: &FramePrepaint, window: &mut Window) {
    window.set_cursor_style(CursorStyle::IBeam, &frame.text_hitbox);
    let text_hit = frame.text_hitbox.clone();
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseDownEvent, phase, window, cx| {
        if !phase.bubble() || !text_hit.is_hovered(window) {
            return;
        }
        v.update(cx, |v, cx| down(v, ev, window, cx));
    });
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _window, cx| {
        if !phase.bubble() {
            return;
        }
        v.update(cx, |v, cx| {
            if v.drag.is_none() {
                return;
            }
            if ev.pressed_button == Some(MouseButton::Left) {
                drag(v, ev.position, cx);
            } else if end_drag(v) {
                cx.notify();
            }
        });
    });
    let v = view.clone();
    window.on_mouse_event(move |ev: &MouseUpEvent, phase, _window, cx| {
        if phase.bubble() && ev.button == MouseButton::Left {
            v.update(cx, |v, cx| {
                if end_drag(v) {
                    cx.notify();
                }
            });
        }
    });
    let v = view.clone();
    let hit = frame.hitbox.clone();
    window.on_mouse_event(move |ev: &ScrollEvent, phase, window, cx| {
        if !phase.bubble() || !hit.id.should_handle_scroll(window) {
            return;
        }
        v.update(cx, |v, cx| wheel(v, ev, cx));
    });
}

/// The range a click of `unit` at `at` selects.
fn unit_range(v: &EditorView, at: usize, unit: ClickUnit) -> Range<usize> {
    let buf = v.model.buffer();
    match unit {
        ClickUnit::Char => at..at,
        ClickUnit::Word => {
            let tok = v.service.token_oracle(v.doc);
            word_at(buf, at, tok.as_ref())
        }
        ClickUnit::Line => buf.line_range(buf.byte_to_line(at), true),
        ClickUnit::Column => at..at,
    }
}

/// Selects with `anchor` and `head` through the IME core's dispatch.
fn select(v: &mut EditorView, anchor: usize, head: usize, cx: &mut Context<EditorView>) {
    let len = v.model.buffer().len_bytes();
    let tr = Transaction {
        changes: hacivat_text::ChangeSet::identity(len),
        selection: Some(SelectionSet::single(SelectionRange {
            anchor,
            head,
            goal_x: None,
            assoc: Bias::After,
        })),
        ann: v.annotations(UserEvent::SelectPointer, hacivat_text::Origin::User, cx),
    };
    v.commit(tr, cx);
}

/// The selection a drag from `origin` to `current` makes: the anchor at the far edge of the
/// first unit.
pub(crate) fn extend(origin: &Range<usize>, current: &Range<usize>) -> (usize, usize) {
    if current.start < origin.start {
        (origin.end, current.start)
    } else {
        (origin.start, current.end.max(origin.end))
    }
}

pub(crate) fn down(
    v: &mut EditorView,
    ev: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<EditorView>,
) {
    let Some(at) = v
        .layout
        .as_ref()
        .and_then(|l| l.clamped_offset_for_point(ev.position))
    else {
        return;
    };
    window.focus(&v.focus, cx);
    cx.stop_propagation();
    match ev.button {
        MouseButton::Left => {
            // ## m3c-t31-multi (fix round 1, M2): the press and its drag are one step of the
            // selection history.
            v.model.begin_selection_gesture();
            // ## m3c-t31-multi: a modifier-click adds a caret (the new main one); a
            // modifier-drag selects a column from here.
            let column = column_drag(&ev.modifiers);
            if ev.click_count <= 1 && (column || adds_caret(&ev.modifiers)) {
                v.drag = None;
                if adds_caret(&ev.modifiers) {
                    let set = add_caret(v.model.selection(), at);
                    commit_set(v, set, cx);
                } else if let Some(anchor) = line_and_column(v, ev.position) {
                    select_column(v, anchor, anchor, cx);
                }
                if column {
                    v.drag = line_and_column(v, ev.position).map(|anchor| DragState {
                        unit: ClickUnit::Column,
                        origin: at..at,
                        column: Some(anchor),
                        column_started: false,
                    });
                }
                crate::ime::send_owed_discard(v, window);
                return;
            }
            let unit = match ev.click_count {
                0 | 1 => ClickUnit::Char,
                2 => ClickUnit::Word,
                _ => ClickUnit::Line,
            };
            if unit == ClickUnit::Char && ev.modifiers.shift {
                let anchor = v.model.selection().main().anchor;
                v.drag = Some(DragState {
                    unit,
                    origin: anchor..anchor,
                    column: None,
                    column_started: false,
                });
                select(v, anchor, at, cx);
                // ## m3a-f4-ime: a composition the click committed is dropped on the platform.
                crate::ime::send_owed_discard(v, window);
                return;
            }
            // ## m3a-t25-editor-commands (named exception, UI-EDIT-06 step 9): a double click
            // inside a string selects its content first, a second one the string with its
            // quotes (`commands::double_click`). A drag still extends by whole words.
            let origin = if unit == ClickUnit::Word {
                crate::commands::double_click(v, at)
            } else {
                unit_range(v, at, unit)
            };
            v.drag = Some(DragState {
                unit,
                origin: origin.clone(),
                column: None,
                column_started: false,
            });
            select(v, origin.start, origin.end, cx);
            // ## m3a-f4-ime: a composition the click committed is dropped on the platform at
            // once (the IME's next key must not find it), as AppKit text views do.
            crate::ime::send_owed_discard(v, window);
        }
        MouseButton::Right => {
            let main = v.model.selection().main().range();
            if !(main.start <= at && at <= main.end && !main.is_empty()) {
                select(v, at, at, cx);
            }
            crate::ime::send_owed_discard(v, window);
            v.open_context_menu_at(ev.position, window, cx);
        }
        // ## m3c-t31-multi (UI-EDIT-14 step 6): the primary selection pasted at the pointer,
        // the selections kept (Linux only).
        #[cfg(target_os = "linux")]
        MouseButton::Middle => {
            // The fork has no asynchronous primary read (fork carry): the read is synchronous,
            // the normalisation and the size check run on the background executor (fix round
            // 1, I2).
            if let Some(text) = cx.read_from_primary().and_then(|i| i.text()) {
                crate::clipboard::paste_text_at(v, at, text, cx);
            }
            crate::ime::send_owed_discard(v, window);
        }
        _ => {}
    }
}

pub(crate) fn drag(v: &mut EditorView, position: Point<Pixels>, cx: &mut Context<EditorView>) {
    let Some(state) = v.drag.clone() else {
        return;
    };
    let Some(layout) = v.layout.as_ref() else {
        return;
    };
    // Past the top or bottom: scroll by the overshoot (bounded per move).
    let area = layout.rows().bounds;
    let lh = f32::from(layout.rows().line_height);
    let over = if position.y < area.top() {
        f32::from(position.y - area.top())
    } else if position.y > area.bottom() {
        f32::from(position.y - area.bottom())
    } else {
        0.0
    };
    let Some(at) = layout.clamped_offset_for_point(position) else {
        return;
    };
    if over != 0.0 {
        let dy = over.clamp(-DRAG_SCROLL_ROWS * lh, DRAG_SCROLL_ROWS * lh);
        v.scroll_by(f64::from(dy));
    }
    // ## m3c-t34-editor: past the edge, it goes on while the pointer rests.
    follow_edge(v, position, over != 0.0, cx);
    // ## m3c-t31-multi: a column drag selects the box to the pointer.
    if let Some(anchor) = state.column {
        if let Some(head) = line_and_column(v, position)
            && (state.column_started || head != anchor)
        {
            if let Some(d) = v.drag.as_mut() {
                d.column_started = true;
            }
            select_column(v, anchor, head, cx);
        }
        if over != 0.0 {
            cx.notify();
        }
        return;
    }
    let current = unit_range(v, at, state.unit);
    let (anchor, head) = if state.unit == ClickUnit::Char {
        (state.origin.start, at)
    } else {
        extend(&state.origin, &current)
    };
    let sel = v.model.selection().main();
    if sel.anchor != anchor || sel.head != head {
        select(v, anchor, head, cx);
    } else if over != 0.0 {
        cx.notify();
    }
}

pub(crate) fn wheel(v: &mut EditorView, ev: &ScrollEvent, cx: &mut Context<EditorView>) {
    let Some(layout) = v.layout.as_ref() else {
        return;
    };
    let d = ev.delta.pixel_delta(layout.rows().line_height);
    let (mut dx, mut dy) = (-f32::from(d.x), -f32::from(d.y));
    if ev.modifiers.shift && dx == 0.0 {
        (dx, dy) = (dy, 0.0);
    }
    v.mark_input(cx);
    if dy != 0.0 {
        v.scroll_by(f64::from(dy));
    }
    if dx != 0.0 {
        v.scroll.scroll_x = (v.scroll.scroll_x + f64::from(dx)).max(0.0);
    }
    if dx != 0.0 || dy != 0.0 {
        cx.stop_propagation();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_keeps_the_first_unit_and_grows_either_way() {
        // Double-click on 10..15, drag right onto 20..24: 10..24.
        assert_eq!(extend(&(10..15), &(20..24)), (10, 24));
        // Drag left onto 2..6: anchored at the first word's end.
        assert_eq!(extend(&(10..15), &(2..6)), (15, 2));
        // Back inside the first unit: the unit itself.
        assert_eq!(extend(&(10..15), &(10..15)), (10, 15));
    }
}
