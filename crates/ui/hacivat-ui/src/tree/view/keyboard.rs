//! Keys (a widget never binds keys: it reads key-downs while focused, and leaves every chord
//! with ⌘, Ctrl, Alt or fn to the host's keymap). Nothing fires while an input is composing
//! (IME).
//!
//! ↑/↓ (Shift extends a multi-select), → expands or moves to the first child, ← collapses or
//! moves to the parent, Home/End, PageUp/PageDown (the viewport less one row), Enter activates
//! (a group header toggles), Shift-F10 and the menu key open the context menu at the row's
//! bottom-left, Space toggles the cursor row in a multi-select, other printable characters
//! type ahead. Group headers that can expand are cursor stops (R-M4-49); type-ahead skips them.

use std::time::Instant;

use hacivat_ui_core::gpui_surface::{Context, KeyDownEvent, Window};

use super::TreeView;
use crate::input::FocusedInput;
use crate::tree::typeahead::find;
use crate::tree::{TreeEvent, TreeModel};

impl<M: TreeModel> TreeView<M> {
    pub(super) fn on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.try_global::<FocusedInput>().is_some_and(|f| f.composing) {
            return;
        }
        self.sync(cx);
        let k = &event.keystroke;
        let m = k.modifiers;
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        let cur = self.cursor_ix();
        let now = cx.background_executor().now();
        match (k.key.as_str(), m.shift) {
            ("up", shift) => {
                let to = self.step(cur, -1, 1);
                self.set_cursor(to, shift, cx);
            }
            ("down", shift) => {
                let to = self.step(cur, 1, 1);
                self.set_cursor(to, shift, cx);
            }
            ("pageup", shift) => {
                let to = self.step(cur, -1, self.page_rows());
                self.set_cursor(to, shift, cx);
            }
            ("pagedown", shift) => {
                let to = self.step(cur, 1, self.page_rows());
                self.set_cursor(to, shift, cx);
            }
            ("home", shift) => {
                let to = self.step(None, 1, 1);
                self.set_cursor(to, shift, cx);
            }
            ("end", shift) => {
                let to = self.step(None, -1, 1);
                self.set_cursor(to, shift, cx);
            }
            ("right", false) => self.right(cur, cx),
            ("left", false) => self.left(cur, cx),
            ("enter", false) => self.enter(cur, cx),
            ("f10", true) | ("menu", _) => self.keyboard_menu(cur, cx),
            ("space", false) if !self.typeahead.is_active(now) => {
                if self.multi
                    && let Some(ix) = cur.filter(|&ix| self.selectable(ix))
                {
                    let key = self.flat.rows()[ix].key;
                    if !self.selection.remove(&key) {
                        self.selection.insert(key);
                    }
                }
                cx.notify();
            }
            _ => {
                let Some(typed) = k.key_char.as_deref().filter(|s| printable(s)) else {
                    return;
                };
                self.type_ahead(typed, now, cx);
            }
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn right(&mut self, cur: Option<usize>, cx: &mut Context<Self>) {
        let Some(ix) = cur else {
            let to = self.step(None, 1, 1);
            self.set_cursor(to, false, cx);
            return;
        };
        let key = self.flat.rows()[ix].key;
        if !self.model.children(key).expandable() {
            return;
        }
        if !self.expanded.contains(&key) {
            self.expand(key, true, cx);
            cx.notify();
            return;
        }
        // Expanded: the first row of its subtree that takes the cursor.
        let end = self.flat.subtree_end(ix);
        if let Some(first) = (ix + 1..end).find(|&j| self.focusable(j)) {
            self.set_cursor(Some(first), false, cx);
        }
    }

    fn left(&mut self, cur: Option<usize>, cx: &mut Context<Self>) {
        let Some(ix) = cur else {
            return;
        };
        let key = self.flat.rows()[ix].key;
        if self.expanded.contains(&key) && self.model.children(key).expandable() {
            self.collapse(key, true, cx);
            cx.notify();
            return;
        }
        // The nearest ancestor that takes the cursor (a kind-group header does).
        let mut p = self.flat.parent_of(ix);
        while let Some(j) = p {
            if self.focusable(j) {
                self.set_cursor(Some(j), false, cx);
                return;
            }
            p = self.flat.parent_of(j);
        }
    }

    /// Enter: a row activates; a group header expands or collapses (R-M4-49).
    fn enter(&mut self, cur: Option<usize>, cx: &mut Context<Self>) {
        let Some(ix) = cur else {
            return;
        };
        let key = self.flat.rows()[ix].key;
        if self.is_header(ix) {
            self.toggle(key, cx);
        } else {
            cx.emit(TreeEvent::Activate(key));
        }
    }

    /// Shift-F10 or the menu key: the cursor row's menu, at its bottom-left.
    pub(super) fn keyboard_menu(&mut self, cur: Option<usize>, cx: &mut Context<Self>) {
        let Some(ix) = cur else {
            return;
        };
        let key = self.flat.rows()[ix].key;
        self.reveal(ix);
        if let Some(b) = self.bounds_of(ix) {
            cx.emit(TreeEvent::ContextMenu(key, b.bottom_left()));
        }
        cx.notify();
    }

    fn type_ahead(&mut self, typed: &str, now: Instant, cx: &mut Context<Self>) {
        let query = self.typeahead.push(typed, now).to_string();
        let from = self.cursor_ix();
        let found = find(self.flat.len(), from, &query, self.fold, |ix| {
            // Group headers, loading and failed rows are skipped.
            if !self.selectable(ix) {
                return None;
            }
            self.model.search_text(self.flat.rows()[ix].key)
        });
        if found.is_some() {
            self.set_cursor(found, false, cx);
        }
    }
}

/// Whether a key's text is a character to type ahead with (not a control character).
fn printable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| !c.is_control())
}
