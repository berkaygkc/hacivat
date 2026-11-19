//! The pointer and assistive technology.
//!
//! - A click selects (⌘/Ctrl-click toggles and Shift-click extends in a multi-select); a double
//!   click activates; a right click selects the row (unless it is already selected) and asks
//!   for the menu at the pointer.
//! - The chevron's mouse-down expands or collapses without selecting or activating.
//! - A group header that can expand takes the cursor on a click, toggles on a left click and
//!   asks for its menu on a right click; it never becomes the selection (R-M4-49). A header
//!   that cannot expand and the loading and failed child rows ignore the pointer.
//! - Assistive-technology actions (Focus, Click, Expand, Collapse, ScrollIntoView,
//!   ShowContextMenu) act as the keyboard would.

use hacivat_ui_core::gpui_surface::{Context, MouseButton, MouseDownEvent, Window, accesskit};

use super::TreeView;
use crate::tree::{TreeEvent, TreeKey, TreeModel};

impl<M: TreeModel> TreeView<M> {
    pub(super) fn on_row_mouse_down(
        &mut self,
        ix: usize,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.typeahead.clear();
        if !self.focusable(ix) {
            return;
        }
        let key = self.flat.rows()[ix].key;
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

    /// The chevron: expands or collapses without selecting or activating.
    pub(super) fn on_chevron_down(
        &mut self,
        key: TreeKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.toggle(key, cx);
        cx.stop_propagation();
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
        let Some(ix) = self.flat.index_of(key).filter(|&ix| self.focusable(ix)) else {
            return;
        };
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
            accesskit::Action::Expand => {
                self.expand(key, true, cx);
                cx.notify();
            }
            accesskit::Action::Collapse => {
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
