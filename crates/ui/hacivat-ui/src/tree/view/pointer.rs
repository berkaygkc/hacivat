//! The pointer and assistive technology.
//!
//! - A click selects (⌘/Ctrl-click toggles and Shift-click extends in a multi-select); a double
//!   click activates; a right click selects the row (unless it is already selected) and asks
//!   for the menu at the pointer.
//! - The chevron's mouse-down expands or collapses without selecting or activating (a double
//!   click toggles once).
//! - A group header that can expand takes the cursor on a click, toggles on a left click and
//!   asks for its menu on a right click; it never becomes the selection (R-M4-49). A header
//!   that cannot expand and the loading and failed child rows ignore the pointer.
//! - The failed row's retry button, the level pill and the row's own button emit their events
//!   and change nothing else.
//! - Assistive-technology actions (Focus, Click, Expand, Collapse, ScrollIntoView,
//!   ShowContextMenu) act as the keyboard would.
//!
//! Every handler captures the row's key, not its index, and finds the row when the event
//! arrives (fix round 1, I1): the listeners are the last drawn frame's, and the model may have
//! changed since. A key that is no longer shown does nothing.

use hacivat_ui_core::gpui_surface::{
    Context, MouseButton, MouseDownEvent, Pixels, Point, Window, accesskit,
};

use super::TreeView;
use crate::tree::{TreeEvent, TreeKey, TreeModel};

impl<M: TreeModel> TreeView<M> {
    pub(super) fn on_row_mouse_down(
        &mut self,
        key: TreeKey,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.typeahead.clear();
        self.sync(cx);
        let Some(ix) = self.focusable_row(key) else {
            return;
        };
        if self.is_header(ix) {
            self.set_cursor(Some(ix), false, cx);
            match e.button {
                // A double click toggles once.
                MouseButton::Left if e.click_count % 2 == 1 => self.toggle(key, cx),
                MouseButton::Right => cx.emit(TreeEvent::ContextMenu(key, e.position)),
                _ => {}
            }
            cx.stop_propagation();
            return;
        }
        match e.button {
            MouseButton::Left => {
                let toggle = self.multi && e.modifiers.secondary(cx);
                if toggle {
                    if !self.selection.remove(&key) {
                        self.selection.insert(key);
                    }
                    self.range_from = Some(key);
                    self.cursor = Some(key);
                    self.cursor_row = ix;
                    self.note_selection(Some(key), cx);
                    cx.notify();
                } else {
                    self.set_cursor(Some(ix), e.modifiers.shift, cx);
                }
                if e.click_count == 2 && !toggle {
                    cx.emit(TreeEvent::Activate(key));
                }
            }
            MouseButton::Right => {
                if !self.selection.contains(&key) {
                    self.set_cursor(Some(ix), false, cx);
                }
                cx.emit(TreeEvent::ContextMenu(key, e.position));
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    /// The chevron: expands or collapses without selecting or activating; a double click
    /// toggles once.
    pub(super) fn on_chevron_down(
        &mut self,
        key: TreeKey,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        cx.stop_propagation();
        self.sync(cx);
        if e.click_count % 2 == 1 && self.flat.index_of(key).is_some() {
            self.toggle(key, cx);
        }
    }

    /// A button inside a row (retry, the pill, the row action): emits `event` when the row of
    /// `key` is still shown, and nothing else (no selection, no activation).
    pub(super) fn on_row_button(
        &mut self,
        key: TreeKey,
        event: impl FnOnce(&Self) -> Option<TreeEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.sync(cx);
        if self.flat.index_of(key).is_some()
            && let Some(event) = event(self)
        {
            cx.emit(event);
        }
    }

    /// Where the pill of node `key` opens its list: the pill's bottom-left, `x` px from the
    /// row's left edge, the pill being `h` px high and centred in the row.
    pub(super) fn pill_anchor(&self, key: TreeKey, x: f32, h: f32) -> Option<Point<Pixels>> {
        let b = self.row_bounds(key)?;
        let top = (self.row_h - h) / 2.0;
        Some(Point::new(
            b.left() + Pixels::from(x),
            b.top() + Pixels::from(top + h),
        ))
    }

    /// The pointer entered or left node `key`'s row.
    pub(super) fn on_row_hover(&mut self, key: TreeKey, hovered: bool, cx: &mut Context<Self>) {
        let was = self.hovered;
        if hovered {
            self.hovered = Some(key);
        } else if self.hovered == Some(key) {
            self.hovered = None;
        }
        if was != self.hovered {
            cx.notify();
        }
    }

    /// Runs an assistive technology's `action` on node `key`.
    pub(super) fn a11y_action(
        &mut self,
        key: TreeKey,
        action: accesskit::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync(cx);
        let Some(ix) = self.focusable_row(key) else {
            return;
        };
        let expandable = self.model.children(key).expandable();
        match action {
            accesskit::Action::Focus => {
                window.focus(&self.focus, cx);
                self.set_cursor(Some(ix), false, cx);
            }
            accesskit::Action::Click => {
                self.set_cursor(Some(ix), false, cx);
                if self.is_header(ix) {
                    self.toggle(key, cx);
                } else {
                    cx.emit(TreeEvent::Activate(key));
                }
            }
            // A leaf neither expands nor collapses (fix round 1, M2).
            accesskit::Action::Expand if expandable => {
                self.expand(key, true, cx);
                cx.notify();
            }
            accesskit::Action::Collapse if expandable => {
                self.collapse(key, true, cx);
                cx.notify();
            }
            accesskit::Action::ScrollIntoView => {
                self.reveal(ix);
                cx.notify();
            }
            accesskit::Action::ShowContextMenu => {
                self.set_cursor(Some(ix), false, cx);
                self.keyboard_menu(Some(ix), cx);
            }
            _ => {}
        }
    }
}
