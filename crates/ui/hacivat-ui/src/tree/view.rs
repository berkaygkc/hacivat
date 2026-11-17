//! [`TreeView`]: the view, its virtualized element, keyboard, pointer and accessibility glue.
//!
//! **Geometry** (DESIGN.md Explorer Tree): 23 px rows (`tree_row_height`), 14 px per level
//! (`tree_indent`), chevron + kind icon + label, flags after the label, mono meta
//! right-aligned. Hover fills `hover`; a selected row fills `blue-tint` with a 1 px `blue`
//! left edge and a `blue` icon. The cursor row of a keyboard-focused tree has a 1 px `blue`
//! ring. A group header is a small-caps label at `text-3`, a `seam-soft` rule and a mono count.
//! Nothing is drawn below 11 px.
//!
//! **Virtualization.** The view keeps a pixel scroll offset; each frame builds the rows that
//! intersect the viewport plus [`TREE_OVERSCAN_ROWS`] on each side, and nothing else.
//!
//! **Keyboard** (a widget never binds keys: it reads key-downs while focused, and leaves
//! every chord with ⌘, Ctrl or Alt to the host's keymap): ↑/↓ (Shift extends a multi-select),
//! → expands or moves to the first child, ← collapses or moves to the parent, Home/End,
//! PageUp/PageDown, Enter activates, Shift-F10 and the menu key open the context menu, Space
//! toggles the cursor row in a multi-select, other printable letters type ahead. Nothing fires
//! while an input is composing (IME).

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Element, ElementId,
    EventEmitter, FluentBuilder as _, FocusHandle, Focusable, GlobalElementId, InspectorElementId,
    InteractiveElement, IntoElement, KeyDownEvent, LayoutId, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Render, Role, ScrollEvent, StatefulInteractiveElement as _, Style,
    Styled, Window, accesskit, div, point, px, relative, size,
};
use hacivat_ui_core::theme::Theme;

use super::a11y::{A11Y_OVERSCAN_ROWS, ITEM_ACTIONS, ItemNode, ItemsIn, visible_items};
use super::flat::{Flat, KeySet, RowKind};
use super::typeahead::{Fold, TypeAhead, find};
use super::{ChildState, RowFlag, StatusKind, TreeEvent, TreeKey, TreeModel, TreeRowData};
use crate::capsule::env_swatch;
use crate::icon::{Icon, IconName, IconSize};
use crate::input::FocusedInput;
use crate::text::{Label, TextFn, TextRegistry, TextSource};

/// Rows built above and below the ones on screen (keyboard steps never land on an unbuilt row).
pub const TREE_OVERSCAN_ROWS: usize = 2;

/// A virtualized tree over `M` (`cx.new(|cx| TreeView::new(model, cx))`).
pub struct TreeView<M: TreeModel> {
    model: M,
    /// The model generation `flat` was built from.
    built_from: Option<u64>,
    flat: Flat,
    expanded: KeySet,
    /// Expanded nodes with `NotLoaded` children already announced (once per expansion).
    announced: KeySet,
    selection: KeySet,
    cursor: Option<TreeKey>,
    /// Where a Shift range starts.
    range_from: Option<TreeKey>,
    multi: bool,
    /// The content offset of the viewport's top edge, px.
    scroll_y: f64,
    row_h: f32,
    /// The last frame's element bounds.
    bounds: Option<Bounds<Pixels>>,
    /// The rows the last frame built.
    built: Range<usize>,
    last_build: Duration,
    focus: FocusHandle,
    a11y_label: Option<TextFn>,
    typeahead: TypeAhead,
    fold: Fold,
    a11y: A11yState,
}

#[derive(Default)]
struct A11yState {
    /// Frames that built items.
    builds: u64,
    /// The last frame's items.
    items: Rc<Vec<ItemNode>>,
    /// Tests stand in for an attached screen reader.
    forced: bool,
}

impl<M: TreeModel> EventEmitter<TreeEvent> for TreeView<M> {}

impl<M: TreeModel> Focusable for TreeView<M> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl<M: TreeModel> std::fmt::Debug for TreeView<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeView")
            .field("rows", &self.flat.len())
            .field("expanded", &self.expanded.len())
            .field("cursor", &self.cursor)
            .field("scroll_y", &self.scroll_y)
            .finish_non_exhaustive()
    }
}

impl<M: TreeModel> TreeView<M> {
    pub fn new(model: M, cx: &mut Context<Self>) -> Self {
        let row_h = Theme::get(cx).tokens.spacing.tree_row_height;
        TreeView {
            model,
            built_from: None,
            flat: Flat::default(),
            expanded: KeySet::default(),
            announced: KeySet::default(),
            selection: KeySet::default(),
            cursor: None,
            range_from: None,
            multi: false,
            scroll_y: 0.0,
            row_h,
            bounds: None,
            built: 0..0,
            last_build: Duration::ZERO,
            focus: cx.focus_handle().tab_stop(true),
            a11y_label: None,
            typeahead: TypeAhead::default(),
            fold: Fold::default(),
            a11y: A11yState::default(),
        }
    }

    // ---- model ------------------------------------------------------------------------------

    pub fn model(&self) -> &M {
        &self.model
    }

    /// Changes the model and re-flattens on the next frame if its generation moved.
    pub fn update_model<R>(&mut self, f: impl FnOnce(&mut M) -> R, cx: &mut Context<Self>) -> R {
        let r = f(&mut self.model);
        self.model_changed(cx);
        r
    }

    /// Replaces the model (expansion and selection are kept by key).
    pub fn set_model(&mut self, model: M, cx: &mut Context<Self>) {
        self.model = model;
        self.built_from = None;
        cx.notify();
    }

    /// The model's answers changed (its generation moved): re-flatten before the next frame.
    pub fn model_changed(&mut self, cx: &mut Context<Self>) {
        self.sync(cx);
        cx.notify();
    }

    /// Re-flattens when the model's generation moved since the last build, keeping the first
    /// visible row where it is on screen.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let generation = self.model.generation();
        if self.built_from == Some(generation) {
            return;
        }
        let anchor = self.anchor();
        let mut not_loaded = Vec::new();
        self.flat = Flat::build(&self.model, &self.expanded, &mut not_loaded);
        self.built_from = Some(generation);
        // Announce each node that became expanded-and-not-loaded; forget the others, so a
        // node whose children are dropped later is announced again.
        let mut still = KeySet::default();
        for key in not_loaded {
            if !self.announced.contains(&key) {
                cx.emit(TreeEvent::Expand(key));
            }
            still.insert(key);
        }
        self.announced = still;
        if let Some((key, offset)) = anchor
            && let Some(ix) = self.flat.index_of(key)
        {
            self.scroll_y = ix as f64 * f64::from(self.row_h) + offset;
        }
        self.clamp_scroll();
    }

    /// The first visible node row and how far the viewport's top is into it.
    fn anchor(&self) -> Option<(TreeKey, f64)> {
        let h = f64::from(self.row_h);
        let top = (self.scroll_y / h).floor().max(0.0) as usize;
        let (ix, row) = self
            .flat
            .rows()
            .iter()
            .enumerate()
            .skip(top)
            .find(|(_, r)| r.is_node())?;
        Some((row.key, self.scroll_y - ix as f64 * h))
    }

    // ---- options ----------------------------------------------------------------------------

    /// Allows more than one selected row (⌘/Ctrl-click toggles, Shift extends).
    pub fn set_multi_select(&mut self, multi: bool) {
        self.multi = multi;
    }

    /// The tree's accessible name.
    pub fn set_a11y_label(&mut self, label: TextFn) {
        self.a11y_label = Some(label);
    }

    /// How type-ahead folds letters ([`Fold::Tolerant`] by default).
    pub fn set_typeahead_fold(&mut self, fold: Fold) {
        self.fold = fold;
    }

    // ---- state ------------------------------------------------------------------------------

    pub fn expanded(&self) -> &KeySet {
        &self.expanded
    }

    pub fn is_expanded(&self, key: TreeKey) -> bool {
        self.expanded.contains(&key)
    }

    pub fn selection(&self) -> &KeySet {
        &self.selection
    }

    /// The focused row.
    pub fn cursor(&self) -> Option<TreeKey> {
        self.cursor
    }

    /// The flattened rows (as of the last sync).
    pub fn flat(&self) -> &Flat {
        &self.flat
    }

    /// The rows the last frame built.
    pub fn built_rows(&self) -> Range<usize> {
        self.built.clone()
    }

    /// How long the last frame took to build, lay out and prepaint its rows.
    pub fn last_build_time(&self) -> Duration {
        self.last_build
    }

    pub fn scroll_offset(&self) -> f64 {
        self.scroll_y
    }

    /// Where node `key`'s row is in window coordinates at the current scroll (it may lie
    /// outside the viewport); `None` before the first frame or when the row is not shown.
    pub fn row_bounds(&self, key: TreeKey) -> Option<Bounds<Pixels>> {
        let ix = self.flat.index_of(key)?;
        self.bounds_of(ix)
    }

    fn bounds_of(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let b = self.bounds?;
        let top = ix as f64 * f64::from(self.row_h) - self.scroll_y;
        Some(Bounds::new(
            point(b.left(), b.top() + px(top as f32)),
            size(b.size.width, px(self.row_h)),
        ))
    }

    // ---- expansion --------------------------------------------------------------------------

    /// Replaces the whole expansion (a restored session, "expand all"): one re-flatten, and a
    /// load request for each expanded node whose children are `NotLoaded`.
    pub fn set_expanded_keys(
        &mut self,
        keys: impl IntoIterator<Item = TreeKey>,
        cx: &mut Context<Self>,
    ) {
        self.expanded = keys.into_iter().collect();
        self.built_from = None;
        self.sync(cx);
        cx.notify();
    }

    /// Expands or collapses `key` (by the host: no `Expand`/`Collapse` event, except the
    /// load request for children that are `NotLoaded`).
    pub fn set_expanded(&mut self, key: TreeKey, expanded: bool, cx: &mut Context<Self>) {
        self.sync(cx);
        if expanded {
            self.expand(key, false, cx);
        } else {
            self.collapse(key, false, cx);
        }
        cx.notify();
    }

    fn toggle(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        if self.expanded.contains(&key) {
            self.collapse(key, true, cx);
        } else {
            self.expand(key, true, cx);
        }
        cx.notify();
    }

    fn expand(&mut self, key: TreeKey, user: bool, cx: &mut Context<Self>) {
        if !self.expanded.insert(key) {
            return;
        }
        let mut not_loaded = Vec::new();
        if let Some(ix) = self.flat.index_of(key) {
            let n = self
                .flat
                .expand_at(ix, &self.model, &self.expanded, &mut not_loaded);
            self.shift_for_insert(ix + 1, n);
        }
        for k in not_loaded {
            if self.announced.insert(k) && !(user && k == key) {
                cx.emit(TreeEvent::Expand(k));
            }
        }
        if user {
            // The user's expansion is announced whatever its children's state.
            cx.emit(TreeEvent::Expand(key));
        }
    }

    fn collapse(&mut self, key: TreeKey, user: bool, cx: &mut Context<Self>) {
        if !self.expanded.remove(&key) {
            return;
        }
        self.announced.remove(&key);
        if let Some(ix) = self.flat.index_of(key) {
            let end = self.flat.subtree_end(ix);
            let cursor_inside = self
                .cursor
                .and_then(|c| self.flat.index_of(c))
                .is_some_and(|c| c > ix && c < end);
            let n = self.flat.collapse_at(ix);
            self.shift_for_remove(ix, n);
            if cursor_inside {
                let to = self.selectable_at_or_above(ix);
                self.set_cursor(to, false, cx);
            }
        }
        if user {
            cx.emit(TreeEvent::Collapse(key));
        }
    }

    /// Scroll anchoring for `n` rows inserted at `at`.
    fn shift_for_insert(&mut self, at: usize, n: usize) {
        let h = f64::from(self.row_h);
        if n > 0 && (at as f64) * h <= self.scroll_y {
            self.scroll_y += n as f64 * h;
        }
    }

    /// Scroll anchoring for the `n` rows after row `ix` removed.
    fn shift_for_remove(&mut self, ix: usize, n: usize) {
        let h = f64::from(self.row_h);
        let (start, end) = ((ix + 1) as f64 * h, (ix + 1 + n) as f64 * h);
        if n == 0 || self.scroll_y < start {
            return;
        }
        if self.scroll_y >= end {
            self.scroll_y -= n as f64 * h;
        } else {
            // The top row was removed: the collapsed node becomes the top row.
            self.scroll_y = ix as f64 * h;
        }
        self.clamp_scroll();
    }

    // ---- selection --------------------------------------------------------------------------

    /// Whether row `ix` can take the cursor: a node that is not a group header.
    fn selectable(&self, ix: usize) -> bool {
        self.flat
            .get(ix)
            .is_some_and(|r| r.is_node() && !self.model.row(r.key).is_some_and(|d| d.group_header))
    }

    fn selectable_at_or_above(&self, ix: usize) -> Option<usize> {
        (0..=ix.min(self.flat.len().saturating_sub(1)))
            .rev()
            .find(|&j| self.selectable(j))
    }

    /// The next selectable row from `from` in direction `dir`, at most `steps` rows away
    /// (stopping at the last selectable one on the way).
    fn step(&self, from: Option<usize>, dir: isize, steps: usize) -> Option<usize> {
        let len = self.flat.len();
        if len == 0 {
            return None;
        }
        let Some(from) = from else {
            return if dir > 0 {
                (0..len).find(|&j| self.selectable(j))
            } else {
                (0..len).rev().find(|&j| self.selectable(j))
            };
        };
        let mut best = None;
        let mut taken = 0;
        let mut j = from as isize + dir;
        while j >= 0 && (j as usize) < len && taken < steps {
            if self.selectable(j as usize) {
                best = Some(j as usize);
                taken += 1;
            }
            j += dir;
        }
        best
    }

    fn cursor_ix(&self) -> Option<usize> {
        self.cursor.and_then(|c| self.flat.index_of(c))
    }

    /// Selects `key` alone, makes it the cursor and reveals it.
    pub fn select(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        self.sync(cx);
        let ix = self.flat.index_of(key);
        self.selection.clear();
        self.selection.insert(key);
        self.range_from = Some(key);
        let changed = self.cursor != Some(key);
        self.cursor = Some(key);
        if let Some(ix) = ix {
            self.reveal(ix);
        }
        if changed {
            cx.emit(TreeEvent::SelectionChanged(Some(key)));
        }
        cx.notify();
    }

    /// Scrolls node `key`'s row into view (when it is shown).
    pub fn scroll_to(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        self.sync(cx);
        if let Some(ix) = self.flat.index_of(key) {
            self.reveal(ix);
            cx.notify();
        }
    }

    /// Moves the cursor to row `to` (alone selected, or extending the range).
    fn set_cursor(&mut self, to: Option<usize>, extend: bool, cx: &mut Context<Self>) {
        let Some(ix) = to else {
            return;
        };
        let key = self.flat.rows()[ix].key;
        if extend && self.multi {
            let from = self
                .range_from
                .and_then(|k| self.flat.index_of(k))
                .unwrap_or(ix);
            self.selection.clear();
            let (a, b) = (from.min(ix), from.max(ix));
            for j in a..=b {
                if self.selectable(j) {
                    self.selection.insert(self.flat.rows()[j].key);
                }
            }
        } else {
            self.selection.clear();
            self.selection.insert(key);
            self.range_from = Some(key);
        }
        let changed = self.cursor != Some(key);
        self.cursor = Some(key);
        self.reveal(ix);
        if changed {
            cx.emit(TreeEvent::SelectionChanged(Some(key)));
        }
        cx.notify();
    }

    // ---- scrolling --------------------------------------------------------------------------

    fn viewport_h(&self) -> f64 {
        self.bounds
            .map_or(0.0, |b| f64::from(f32::from(b.size.height)))
    }

    fn content_h(&self) -> f64 {
        self.flat.len() as f64 * f64::from(self.row_h)
    }

    fn clamp_scroll(&mut self) {
        let max = (self.content_h() - self.viewport_h()).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max);
    }

    /// Scrolls by `dy` px (positive towards later rows).
    pub fn scroll_by(&mut self, dy: f64, cx: &mut Context<Self>) {
        self.scroll_y += dy;
        self.clamp_scroll();
        cx.notify();
    }

    fn reveal(&mut self, ix: usize) {
        let h = f64::from(self.row_h);
        let (top, bottom) = (ix as f64 * h, (ix + 1) as f64 * h);
        let view = self.viewport_h();
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + view && view > 0.0 {
            self.scroll_y = bottom - view;
        }
        self.clamp_scroll();
    }

    /// Rows that fit the viewport.
    fn page_rows(&self) -> usize {
        ((self.viewport_h() / f64::from(self.row_h)).floor() as usize).max(2) - 1
    }

    /// The rows intersecting the viewport.
    fn visible(&self) -> Range<usize> {
        let h = f64::from(self.row_h);
        let first = (self.scroll_y / h).floor().max(0.0) as usize;
        let last = ((self.scroll_y + self.viewport_h()) / h).ceil() as usize;
        first.min(self.flat.len())..last.min(self.flat.len())
    }

    // ---- input ------------------------------------------------------------------------------

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
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
            ("enter", false) => {
                if let Some(key) = self.cursor.filter(|_| cur.is_some()) {
                    cx.emit(TreeEvent::Activate(key));
                }
            }
            ("f10", true) | ("menu", _) => self.keyboard_menu(cur, cx),
            ("space", false) if !self.typeahead.is_active(now) => {
                if self.multi
                    && let Some(key) = self.cursor
                    && !self.selection.remove(&key)
                {
                    self.selection.insert(key);
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
        // Expanded: the first selectable row of its subtree.
        let end = self.flat.subtree_end(ix);
        if let Some(first) = (ix + 1..end).find(|&j| self.selectable(j)) {
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
        // The nearest selectable ancestor (group headers are passed over).
        let mut p = self.flat.parent_of(ix);
        while let Some(j) = p {
            if self.selectable(j) {
                self.set_cursor(Some(j), false, cx);
                return;
            }
            p = self.flat.parent_of(j);
        }
    }

    fn keyboard_menu(&mut self, cur: Option<usize>, cx: &mut Context<Self>) {
        let (Some(ix), Some(key)) = (cur, self.cursor) else {
            return;
        };
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
            let r = self.flat.get(ix)?;
            if !r.is_node() {
                return None;
            }
            let data = self.model.row(r.key)?;
            if data.group_header {
                return None;
            }
            self.model.search_text(r.key)
        });
        if found.is_some() {
            self.set_cursor(found, false, cx);
        }
    }

    fn on_row_mouse_down(
        &mut self,
        ix: usize,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.typeahead.clear();
        let Some(row) = self.flat.get(ix).copied() else {
            return;
        };
        let header = self.model.row(row.key).is_some_and(|d| d.group_header);
        if header {
            if e.button == MouseButton::Left && self.model.children(row.key).expandable() {
                self.toggle(row.key, cx);
            }
            return;
        }
        if !row.is_node() {
            return;
        }
        match e.button {
            MouseButton::Left => {
                let toggle = self.multi && e.modifiers.secondary(cx);
                if toggle {
                    if !self.selection.remove(&row.key) {
                        self.selection.insert(row.key);
                    }
                    self.range_from = Some(row.key);
                    let changed = self.cursor != Some(row.key);
                    self.cursor = Some(row.key);
                    if changed {
                        cx.emit(TreeEvent::SelectionChanged(Some(row.key)));
                    }
                    cx.notify();
                } else {
                    self.set_cursor(Some(ix), e.modifiers.shift, cx);
                }
                if e.click_count == 2 && !toggle {
                    cx.emit(TreeEvent::Activate(row.key));
                }
            }
            MouseButton::Right => {
                if !self.selection.contains(&row.key) {
                    self.set_cursor(Some(ix), false, cx);
                }
                cx.emit(TreeEvent::ContextMenu(row.key, e.position));
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    // ---- accessibility ----------------------------------------------------------------------

    /// Frames that built the accessibility items (only while assistive technology is active).
    pub fn a11y_builds(&self) -> u64 {
        self.a11y.builds
    }

    /// The items the last active frame built (empty without assistive technology).
    pub fn a11y_items(&self) -> Rc<Vec<ItemNode>> {
        self.a11y.items.clone()
    }

    /// Builds the items as if a screen reader were attached (test windows never attach one).
    #[doc(hidden)]
    pub fn assume_a11y_active(&mut self, on: bool, cx: &mut Context<Self>) {
        self.a11y.forced = on;
        if !on {
            self.a11y.items = Rc::default();
        }
        cx.notify();
    }

    fn a11y_active(&self, window: &Window) -> bool {
        self.a11y.forced || window.is_a11y_active()
    }

    /// Runs an assistive technology's `action` on node `key`.
    fn a11y_action(
        &mut self,
        key: TreeKey,
        action: accesskit::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync(cx);
        let Some(ix) = self.flat.index_of(key) else {
            return;
        };
        match action {
            accesskit::Action::Focus => {
                window.focus(&self.focus, cx);
                self.set_cursor(Some(ix), false, cx);
            }
            accesskit::Action::Click => {
                self.set_cursor(Some(ix), false, cx);
                cx.emit(TreeEvent::Activate(key));
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

    // ---- rows -------------------------------------------------------------------------------

    fn build_row(&mut self, ix: usize, ring: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let row = self.flat.rows()[ix];
        let indent = t.spacing.md + f32::from(row.depth) * t.spacing.tree_indent;
        let chevron_w = IconSize::Chrome.px();
        let base = div()
            .id(row_id(row.key, row.kind))
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .h(px(self.row_h))
            .pl(px(indent))
            .pr(px(t.spacing.md))
            .gap(px(t.spacing.xs));
        if row.kind != RowKind::Node {
            let label = match self.model.children(row.key) {
                ChildState::Loading { label } | ChildState::Failed { label } => label,
                _ => Default::default(),
            };
            let failed = row.kind == RowKind::Failed;
            return base
                .child(div().w(px(chevron_w)).flex_none())
                .when(failed, |el| {
                    el.child(
                        Icon::new(IconName::CircleAlert)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    )
                })
                .child(
                    div().min_w(px(0.0)).flex_shrink(1.0).child(
                        Label::new(
                            label,
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .truncate(true),
                    ),
                )
                .into_any_element();
        }
        let data = self
            .model
            .row(row.key)
            .unwrap_or_else(|| TreeRowData::new("", TextSource::Data));
        let expandable = self.model.children(row.key).expandable();
        let open = self.expanded.contains(&row.key);
        let key = row.key;
        let chevron = if expandable {
            div()
                .id(("hv-tree-chevron", ix))
                .flex_none()
                .w(px(chevron_w))
                .cursor_pointer()
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(IconSize::Chrome)
                    .color(c.text_3.to_hsla()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        window.focus(&this.focus, cx);
                        this.toggle(key, cx);
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        } else {
            div().flex_none().w(px(chevron_w)).into_any_element()
        };
        let mouse = || {
            cx.listener(move |this: &mut Self, e: &MouseDownEvent, window, cx| {
                this.on_row_mouse_down(ix, e, window, cx);
            })
        };
        if data.group_header {
            let mut el = base
                .child(chevron)
                .child(div().flex_none().child(Label::new(
                    data.label,
                    data.label_source,
                    t.type_ramp.label_sc,
                    c.text_3.to_hsla(),
                )))
                .child(div().flex_1().h(px(1.0)).bg(c.seam_soft.to_hsla()));
            if let Some(meta) = data.meta {
                el = el.child(div().flex_none().child(Label::new(
                    meta,
                    TextSource::Fluent,
                    t.type_ramp.mono_micro,
                    c.text_3.to_hsla(),
                )));
            }
            return el
                .on_mouse_down(MouseButton::Left, mouse())
                .into_any_element();
        }
        let selected = self.selection.contains(&row.key);
        let is_cursor = self.cursor == Some(row.key);
        let ink = if data.dim { c.text_3 } else { c.text };
        let icon_ink = if selected {
            c.blue
        } else if data.dim {
            c.text_3
        } else {
            c.text_2
        };
        let mut el = base.child(chevron);
        if let Some(icon) = data.icon {
            el = el.child(
                div().flex_none().pr(px(t.spacing.s_2xs)).child(
                    Icon::new(icon)
                        .size(IconSize::Chrome)
                        .color(icon_ink.to_hsla()),
                ),
            );
        }
        el = el.child(
            div().min_w(px(0.0)).flex_shrink(1.0).child(
                Label::new(
                    data.label,
                    data.label_source,
                    t.type_ramp.body,
                    ink.to_hsla(),
                )
                .truncate(true),
            ),
        );
        for flag in data.flags {
            el = el.child(match flag {
                RowFlag::Env(kind) => env_swatch(kind, &theme).into_any_element(),
                RowFlag::Lock { production } => Icon::new(IconName::Lock)
                    .size(IconSize::Chrome)
                    .color(if production { c.red } else { c.text_2 }.to_hsla())
                    .into_any_element(),
                RowFlag::Icon { icon, .. } => Icon::new(icon)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla())
                    .into_any_element(),
            });
        }
        if let Some(badge) = data.badge {
            el = el.child(
                div()
                    .flex_none()
                    .px(px(t.spacing.xs))
                    .border_1()
                    .border_color(c.seam_strong.to_hsla())
                    .rounded(px(t.radii.xs))
                    .child(Label::new(
                        badge,
                        TextSource::Fluent,
                        t.type_ramp.micro,
                        c.text_2.to_hsla(),
                    )),
            );
        }
        if let Some(status) = data.status {
            let ink = match status.kind {
                StatusKind::Modified | StatusKind::Conflicted => c.amber,
                StatusKind::New => c.green,
                StatusKind::ContainsChanges => c.text_2,
                StatusKind::Unknown => c.text_3,
            };
            el = el.child(div().flex_none().child(Label::new(
                status.glyph,
                TextSource::Fluent,
                t.type_ramp.mono_micro,
                ink.to_hsla(),
            )));
        }
        el = el.child(div().flex_1());
        if let Some(meta) = data.meta {
            el = el.child(div().flex_none().child(Label::new(
                meta,
                TextSource::Data,
                t.type_ramp.mono_micro,
                c.text_2.to_hsla(),
            )));
        }
        el = if selected {
            // The 1 px blue left edge: a child quad, so no row carries a border.
            el.relative().bg(c.blue_tint.to_hsla()).child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(1.0))
                    .h_full()
                    .bg(c.blue.to_hsla()),
            )
        } else {
            el.hover(|s| s.bg(c.hover.to_hsla()))
        };
        if ring && is_cursor {
            // The focus ring: an overlay, so the row's content never shifts by the border.
            el = el.relative().child(
                div()
                    .absolute()
                    .inset_0()
                    .border_1()
                    .border_color(c.blue.to_hsla()),
            );
        }
        el.on_mouse_down(MouseButton::Left, mouse())
            .on_mouse_down(MouseButton::Right, mouse())
            .into_any_element()
    }
}

/// A row's element id: its key and kind (stable across scrolls and generations).
fn row_id(key: TreeKey, kind: RowKind) -> ElementId {
    let mut bytes = [0u8; 20];
    bytes[..16].copy_from_slice(&key.0.to_le_bytes());
    bytes[16] = kind as u8 + 1;
    ElementId::from(bytes)
}

/// Whether a key's text is a character to type ahead with (not a control character).
fn printable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| !c.is_control())
}

impl<M: TreeModel> Render for TreeView<M> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);
        let a11y = self.a11y_active(window).then(|| {
            let name = self.a11y_label.as_ref().map(|l| l(cx)).unwrap_or_default();
            (name, Rc::new(A11yFrame::default()))
        });
        let mut root = div()
            .id(("hv-tree", cx.entity_id()))
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                // GPUI's deltas are positive when the content should move down.
                let d = e.delta.pixel_delta(px(this.row_h));
                this.scroll_by(-f64::from(f32::from(d.y)), cx);
            }));
        let frame = a11y.as_ref().map(|(_, f)| f.clone());
        if let Some((name, frame)) = a11y {
            root = root.role(Role::Tree).aria_label(name);
            root = root.a11y_synthetic_children(move |b| {
                let items = frame.items.borrow();
                let scale = frame.scale.get();
                let mut ids = frame.ids.borrow_mut();
                let mut focus = None;
                for (item, rect) in items.iter() {
                    let mut node = item.node();
                    let (x0, y0, x1, y1) = *rect;
                    node.set_bounds(accesskit::Rect {
                        x0: f64::from(x0 * scale),
                        y0: f64::from(y0 * scale),
                        x1: f64::from(x1 * scale),
                        y1: f64::from(y1 * scale),
                    });
                    let id = b.synthetic_node_id(item.id_key());
                    if b.push_child(id, node) {
                        if item.focused {
                            focus = Some(id);
                        }
                        if item.selected.is_some() {
                            ids.push((id, item.key));
                        }
                    }
                }
                if let Some(id) = focus {
                    b.parent_node().set_active_descendant(id);
                }
            });
        }
        root.child(TreeElement {
            view: cx.entity(),
            a11y: frame,
        })
    }
}

/// A rect in logical window px: left, top, right, bottom.
type Rect = (f32, f32, f32, f32);

/// What the element hands the root's accessibility callback within one frame.
#[derive(Default)]
struct A11yFrame {
    /// Each item and its window rect (logical px: left, top, right, bottom).
    items: RefCell<Vec<(ItemNode, Rect)>>,
    scale: std::cell::Cell<f32>,
    /// The pushed items' node ids, for their actions.
    ids: RefCell<Vec<(accesskit::NodeId, TreeKey)>>,
}

/// The tree's element: lays out and paints the visible rows only.
struct TreeElement<M: TreeModel> {
    view: hacivat_ui_core::gpui_surface::Entity<TreeView<M>>,
    a11y: Option<Rc<A11yFrame>>,
}

impl<M: TreeModel> Element for TreeElement<M> {
    type RequestLayoutState = ();
    type PrepaintState = Vec<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let started = Instant::now();
        let (rows, row_h, ring) = self.view.update(cx, |tree, _| {
            tree.bounds = Some(bounds);
            tree.clamp_scroll();
            let visible = tree.visible();
            let rows = visible.start.saturating_sub(TREE_OVERSCAN_ROWS)
                ..(visible.end + TREE_OVERSCAN_ROWS).min(tree.flat.len());
            tree.built = rows.clone();
            let ring = tree.focus.is_focused(window) && window.last_input_was_keyboard();
            (rows, tree.row_h, ring)
        });
        let space = size(
            AvailableSpace::Definite(bounds.size.width),
            AvailableSpace::Definite(px(row_h)),
        );
        let mut items = Vec::with_capacity(rows.len());
        for ix in rows {
            let (mut el, top) = self.view.update(cx, |tree, cx| {
                let top = ix as f64 * f64::from(tree.row_h) - tree.scroll_y;
                (tree.build_row(ix, ring, cx), top as f32)
            });
            el.prepaint_as_root(
                point(bounds.left(), bounds.top() + px(top)),
                space,
                window,
                cx,
            );
            items.push(el);
        }
        if let Some(frame) = &self.a11y {
            let scale = window.scale_factor();
            self.view.update(cx, |tree, _| {
                tree.a11y.builds += 1;
                let items = visible_items(
                    &tree.model,
                    ItemsIn {
                        flat: &tree.flat,
                        visible: tree.visible(),
                        overscan: A11Y_OVERSCAN_ROWS,
                        expanded: &tree.expanded,
                        selection: &tree.selection,
                        cursor: tree.cursor,
                    },
                );
                let (left, right) = (f32::from(bounds.left()), f32::from(bounds.right()));
                let rects = items
                    .iter()
                    .map(|item| {
                        let top = f32::from(bounds.top())
                            + (item.row as f64 * f64::from(tree.row_h) - tree.scroll_y) as f32;
                        (item.clone(), (left, top, right, top + tree.row_h))
                    })
                    .collect();
                *frame.items.borrow_mut() = rects;
                frame.scale.set(scale);
                tree.a11y.items = Rc::new(items);
            });
        }
        let took = started.elapsed();
        self.view.update(cx, |tree, _| tree.last_build = took);
        items
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        items: &mut Vec<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) {
        TextRegistry::note_scroll_viewport(cx, bounds);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for el in items.iter_mut() {
                el.paint(window, cx);
            }
        });
        if let Some(frame) = &self.a11y {
            let view = self.view.downgrade();
            for (id, key) in frame.ids.borrow().iter().copied() {
                for action in ITEM_ACTIONS {
                    let view = view.clone();
                    window.on_a11y_action(id, action, move |_, window, cx| {
                        let _ = view.update(cx, |v, cx| v.a11y_action(key, action, window, cx));
                    });
                }
            }
        }
    }
}
