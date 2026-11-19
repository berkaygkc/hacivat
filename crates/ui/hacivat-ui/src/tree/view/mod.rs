//! [`TreeView`]: the entity hosts create with `cx.new(|cx| TreeView::new(model, cx))`.
//!
//! - this file: the view's state, the model contract (re-flatten on a generation change),
//!   expansion, the cursor and the selection, accessibility accessors, `Render`;
//! - [`anchor`]: scrolling, the viewport and scroll anchoring;
//! - [`keyboard`]: key-downs and type-ahead;
//! - [`pointer`]: mouse-downs and the assistive-technology actions;
//! - [`element`]: the virtualized element, the row anatomy and the accessibility hand-off.
//!
//! **The cursor and the selection** (R-M4-49, R-M4-50). The cursor is the keyboard's row; the
//! selection is what the host's actions act on. A group header that can expand takes the
//! cursor but never the selection, and [`TreeEvent::SelectionChanged`] never names one; a
//! header that cannot expand (an environment label) and the loading and failed child rows
//! never take the cursor. With single selection (M4's explorer), the selection is never a
//! hidden row: collapsing the subtree that holds it clears it.

mod anchor;
mod element;
mod keyboard;
mod pointer;

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use hacivat_ui_core::gpui_surface::{
    App, Bounds, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, Role, ScrollEvent, Styled, Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::a11y::ItemNode;
use super::flat::Flat;
use super::typeahead::{Fold, TypeAhead};
use super::{KeySet, TreeEvent, TreeKey, TreeModel};
use crate::popover::Declare as _;
use crate::text::TextFn;
use element::{A11yFrame, TreeElement};

/// Rows built above and below the ones on screen (keyboard steps never land on an unbuilt row).
pub const TREE_OVERSCAN_ROWS: usize = 2;

/// A virtualized tree over `M`.
pub struct TreeView<M: TreeModel> {
    model: M,
    /// The model generation `flat` was built from.
    built_from: Option<u64>,
    flat: Flat,
    expanded: KeySet,
    /// Expanded nodes with `NotLoaded` children already announced (once per expansion).
    announced: KeySet,
    selection: KeySet,
    /// The keyboard's row: a selectable row or a group header that can expand.
    cursor: Option<TreeKey>,
    /// The selected row [`TreeEvent::SelectionChanged`] last named.
    lead: Option<TreeKey>,
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
            .field("multi", &self.multi)
            .field("fold", &self.fold)
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
            lead: None,
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

    /// Changes the model and re-flattens if its generation moved.
    pub fn update_model<R>(&mut self, f: impl FnOnce(&mut M) -> R, cx: &mut Context<Self>) -> R {
        let r = f(&mut self.model);
        self.model_changed(cx);
        r
    }

    /// Replaces the model (expansion and selection are kept by key).
    pub fn set_model(&mut self, model: M, cx: &mut Context<Self>) {
        self.model = model;
        self.built_from = None;
        self.sync(cx);
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

    // ---- options ----------------------------------------------------------------------------

    /// Allows more than one selected row (⌘/Ctrl-click toggles, Shift extends, Space toggles).
    /// M4's explorer keeps single selection (R-M4-50).
    pub fn set_multi_select(&mut self, multi: bool) {
        self.multi = multi;
    }

    /// The tree's accessible name.
    pub fn set_a11y_label(&mut self, label: TextFn) {
        self.a11y_label = Some(label);
    }

    /// How type-ahead folds letters (`Fold::Locale { turkic: false }` by default; the host
    /// sets [`Fold::Tolerant`] for Turkish users, R-M4-50).
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

    /// The selected rows (never a group header).
    pub fn selection(&self) -> &KeySet {
        &self.selection
    }

    /// The keyboard's row: a selectable row or a group header (R-M4-49).
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

    /// How long the last frame took to build, lay out and prepaint its rows (and the
    /// accessibility items while they are built).
    pub fn last_build_time(&self) -> Duration {
        self.last_build
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
        self.prune_hidden_selection(cx);
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
            let cursor_inside = self.cursor_ix().is_some_and(|c| c > ix && c < end);
            // Hidden expanded nodes are asked for again when an expansion reveals them.
            for r in &self.flat.rows()[ix + 1..end] {
                self.announced.remove(&r.key);
            }
            let n = self.flat.collapse_at(ix);
            self.shift_for_remove(ix, n);
            if cursor_inside {
                let to = self.focusable_at_or_above(ix);
                self.set_cursor(to, false, cx);
            }
        }
        self.prune_hidden_selection(cx);
        if user {
            cx.emit(TreeEvent::Collapse(key));
        }
    }

    // ---- the cursor and the selection -------------------------------------------------------

    /// Whether row `ix` is a group header.
    fn is_header(&self, ix: usize) -> bool {
        self.flat
            .get(ix)
            .is_some_and(|r| r.is_node() && self.model.row(r.key).is_some_and(|d| d.group_header))
    }

    /// Whether row `ix` can be selected: a node the model shows that is not a group header.
    fn selectable(&self, ix: usize) -> bool {
        self.flat
            .get(ix)
            .is_some_and(|r| r.is_node() && self.model.row(r.key).is_some_and(|d| !d.group_header))
    }

    /// Whether row `ix` can take the cursor: a selectable row, or a group header that can
    /// expand (R-M4-49). A header without children is a label the keys pass over.
    fn focusable(&self, ix: usize) -> bool {
        let Some(r) = self.flat.get(ix).filter(|r| r.is_node()) else {
            return false;
        };
        match self.model.row(r.key) {
            Some(d) if d.group_header => self.model.children(r.key).expandable(),
            Some(_) => true,
            None => false,
        }
    }

    fn focusable_at_or_above(&self, ix: usize) -> Option<usize> {
        (0..=ix.min(self.flat.len().saturating_sub(1)))
            .rev()
            .find(|&j| self.focusable(j))
    }

    /// The next row that takes the cursor from `from` in direction `dir`, at most `steps` such
    /// rows away (stopping at the last one on the way).
    fn step(&self, from: Option<usize>, dir: isize, steps: usize) -> Option<usize> {
        let len = self.flat.len();
        if len == 0 {
            return None;
        }
        let Some(from) = from else {
            return if dir > 0 {
                (0..len).find(|&j| self.focusable(j))
            } else {
                (0..len).rev().find(|&j| self.focusable(j))
            };
        };
        let mut best = None;
        let mut taken = 0;
        let mut j = from as isize + dir;
        while j >= 0 && (j as usize) < len && taken < steps {
            if self.focusable(j as usize) {
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

    /// Makes `key` the cursor and reveals it, when it is a shown row that takes the cursor: a
    /// row is then selected alone; a group header leaves the selection as it is (R-M4-49).
    pub fn select(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        self.sync(cx);
        if let Some(ix) = self.flat.index_of(key).filter(|&ix| self.focusable(ix)) {
            self.set_cursor(Some(ix), false, cx);
        }
    }

    /// Scrolls node `key`'s row into view (when it is shown).
    pub fn scroll_to(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        self.sync(cx);
        if let Some(ix) = self.flat.index_of(key) {
            self.reveal(ix);
            cx.notify();
        }
    }

    /// Moves the cursor to row `to`. A selectable row becomes the selection (alone, or the
    /// range from the anchor when `extend`ing a multi-select); a group header takes the cursor
    /// only.
    fn set_cursor(&mut self, to: Option<usize>, extend: bool, cx: &mut Context<Self>) {
        let Some(ix) = to else {
            return;
        };
        let Some(key) = self.flat.get(ix).map(|r| r.key) else {
            return;
        };
        if self.selectable(ix) {
            if extend && self.multi {
                let from = self
                    .range_from
                    .and_then(|k| self.flat.index_of(k))
                    .unwrap_or(ix);
                self.selection.clear();
                for j in from.min(ix)..=from.max(ix) {
                    if self.selectable(j) {
                        self.selection.insert(self.flat.rows()[j].key);
                    }
                }
            } else {
                self.selection.clear();
                self.selection.insert(key);
                self.range_from = Some(key);
            }
            self.note_selection(Some(key), cx);
        }
        self.cursor = Some(key);
        self.reveal(ix);
        cx.notify();
    }

    /// Emits [`TreeEvent::SelectionChanged`] when the selected row it names changed.
    fn note_selection(&mut self, lead: Option<TreeKey>, cx: &mut Context<Self>) {
        if self.lead != lead {
            self.lead = lead;
            cx.emit(TreeEvent::SelectionChanged(lead));
        }
    }

    /// Single selection never holds a hidden row (R-M4-50 leaves hidden rows of a multi-select
    /// to the milestone that adds a bulk action): a selection a collapse hid is cleared.
    fn prune_hidden_selection(&mut self, cx: &mut Context<Self>) {
        if self.multi || self.selection.is_empty() {
            return;
        }
        let flat = &self.flat;
        self.selection.retain(|k| flat.index_of(*k).is_some());
        if self.selection.is_empty() {
            self.range_from = None;
            self.note_selection(None, cx);
        }
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
}

impl<M: TreeModel> Render for TreeView<M> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);
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
        // While no assistive technology is attached: no role, no item, no string.
        let frame = self
            .a11y_active(window)
            .then(|| Rc::new(A11yFrame::default()));
        if let Some(frame) = &frame {
            let name = self.a11y_label.as_ref().map(|l| l(cx)).unwrap_or_default();
            root = element::a11y_root(root.declare(Role::Tree, name, None), frame.clone());
        }
        root.child(TreeElement::new(cx.entity(), frame))
    }
}
