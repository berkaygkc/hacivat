//! `TreeView<M>`: the entity hosts create with `cx.new(|cx| TreeView::new(model, cx))`.
//!
//! ## m4-t01b-seams: the spike's public API with stub bodies. The stub draws the roots as a
//! flat list of 23 px rows (label, status glyph, badge, meta), never a chevron and never a
//! child, whatever the model's [`ChildState`](super::ChildState) says. A click selects, a double
//! click or Enter activates, a right click asks for the menu. Each row declares itself a
//! `TreeItem` named by its label, with the status text as its description. Virtualization,
//! expansion, scroll anchoring, the full keyboard, type-ahead and the AccessKit tree are
//! Task 8's.

use hacivat_ui_core::gpui_surface::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Render, Role, SharedString, Styled,
    Window, div, px,
};
use hacivat_ui_core::theme::Theme;

use super::{Fold, KeySet, StatusKind, TreeEvent, TreeKey, TreeModel};
use crate::input::FocusedInput;
use crate::popover::{A11yDecl, Declare as _};
use crate::text::{Label, TextFn, TextSource};

/// A tree over `M`. Stub (Task 8 fills it): a flat, non-expandable list of the roots.
pub struct TreeView<M: TreeModel> {
    model: M,
    expanded: KeySet,
    selection: KeySet,
    cursor: Option<TreeKey>,
    multi: bool,
    focus: FocusHandle,
    a11y_label: Option<TextFn>,
    fold: Fold,
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
            .field("cursor", &self.cursor)
            .field("multi", &self.multi)
            .field("fold", &self.fold)
            .finish_non_exhaustive()
    }
}

impl<M: TreeModel> TreeView<M> {
    pub fn new(model: M, cx: &mut Context<Self>) -> Self {
        TreeView {
            model,
            expanded: KeySet::default(),
            selection: KeySet::default(),
            cursor: None,
            multi: false,
            focus: cx.focus_handle().tab_stop(true),
            a11y_label: None,
            fold: Fold::default(),
        }
    }

    // ---- model ------------------------------------------------------------------------------

    pub fn model(&self) -> &M {
        &self.model
    }

    /// Changes the model and redraws.
    pub fn update_model<R>(&mut self, f: impl FnOnce(&mut M) -> R, cx: &mut Context<Self>) -> R {
        let r = f(&mut self.model);
        self.model_changed(cx);
        r
    }

    /// Replaces the model (selection is kept by key).
    pub fn set_model(&mut self, model: M, cx: &mut Context<Self>) {
        self.model = model;
        cx.notify();
    }

    /// The model's answers changed (its generation moved).
    pub fn model_changed(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    // ---- options ----------------------------------------------------------------------------

    pub fn set_multi_select(&mut self, multi: bool) {
        self.multi = multi;
    }

    /// The tree's accessible name.
    pub fn set_a11y_label(&mut self, label: TextFn) {
        self.a11y_label = Some(label);
    }

    pub fn set_typeahead_fold(&mut self, fold: Fold) {
        self.fold = fold;
    }

    // ---- expansion (stub: nothing expands) ---------------------------------------------------

    /// The expanded keys. Stub: always empty.
    pub fn expanded(&self) -> &KeySet {
        &self.expanded
    }

    pub fn is_expanded(&self, key: TreeKey) -> bool {
        self.expanded.contains(&key)
    }

    /// Replaces the whole expansion. Stub: ignored (the flat stub expands nothing).
    pub fn set_expanded_keys(
        &mut self,
        _keys: impl IntoIterator<Item = TreeKey>,
        _cx: &mut Context<Self>,
    ) {
    }

    /// Expands or collapses `key`. Stub: ignored.
    pub fn set_expanded(&mut self, _key: TreeKey, _expanded: bool, _cx: &mut Context<Self>) {}

    // ---- selection --------------------------------------------------------------------------

    pub fn selection(&self) -> &KeySet {
        &self.selection
    }

    /// The focused row.
    pub fn cursor(&self) -> Option<TreeKey> {
        self.cursor
    }

    /// Selects `key` alone and makes it the cursor, when it is a shown row.
    pub fn select(&mut self, key: TreeKey, cx: &mut Context<Self>) {
        if !self.model.roots().contains(&key) {
            return;
        }
        if self.cursor != Some(key) || self.selection.len() != 1 {
            self.selection.clear();
            self.selection.insert(key);
            self.cursor = Some(key);
            cx.emit(TreeEvent::SelectionChanged(Some(key)));
            cx.notify();
        }
    }

    /// Scrolls `key` into view. Stub: the flat list does not scroll.
    pub fn scroll_to(&mut self, _key: TreeKey, _cx: &mut Context<Self>) {}

    /// Scrolls by `dy` px. Stub: the flat list does not scroll.
    pub fn scroll_by(&mut self, _dy: f64, _cx: &mut Context<Self>) {}

    // ---- input ------------------------------------------------------------------------------

    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if cx.try_global::<FocusedInput>().is_some_and(|f| f.composing) {
            return;
        }
        let k = &event.keystroke;
        let m = k.modifiers;
        if m.platform || m.control || m.alt || m.function || m.shift {
            return;
        }
        if k.key == "enter"
            && let Some(key) = self.cursor
        {
            cx.emit(TreeEvent::Activate(key));
            cx.stop_propagation();
        }
    }

    fn on_row_mouse_down(
        &mut self,
        key: TreeKey,
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        self.select(key, cx);
        match e.button {
            MouseButton::Left if e.click_count == 2 => cx.emit(TreeEvent::Activate(key)),
            MouseButton::Right => cx.emit(TreeEvent::ContextMenu(key, e.position)),
            _ => {}
        }
    }
}

impl<M: TreeModel> Render for TreeView<M> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let name: SharedString = self.a11y_label.as_ref().map(|l| l(cx)).unwrap_or_default();
        let mut root = div()
            .id(("hv-tree", cx.entity_id()))
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(Self::on_key))
            .declare(Role::Tree, name, None);
        let roots = self.model.roots();
        let set_size = roots.len();
        for (ix, key) in roots.iter().copied().enumerate() {
            let Some(data) = self.model.row(key) else {
                continue;
            };
            let selected = self.selection.contains(&key);
            let ink = if data.dim { c.text_3 } else { c.text };
            let mut decl = A11yDecl::new(Role::TreeItem, data.label.clone())
                .selected((!data.group_header).then_some(selected));
            decl.position = Some((ix + 1, set_size));
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .w_full()
                .flex_none()
                .h(px(t.spacing.tree_row_height))
                .px(px(t.spacing.md))
                .gap(px(t.spacing.xs))
                .child(
                    div().min_w(px(0.0)).flex_shrink(1.0).child(
                        Label::new(
                            data.label,
                            data.label_source,
                            if data.group_header {
                                t.type_ramp.label_sc
                            } else {
                                t.type_ramp.body
                            },
                            ink.to_hsla(),
                        )
                        .truncate(true),
                    ),
                );
            if let Some(badge) = data.badge {
                row = row.child(
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
                let status_ink = match status.kind {
                    StatusKind::Modified | StatusKind::Conflicted => c.amber,
                    StatusKind::New => c.green,
                    StatusKind::ContainsChanges => c.text_2,
                    StatusKind::Unknown => c.text_3,
                };
                decl = decl.description(status.text);
                row = row.child(div().flex_none().child(Label::new(
                    status.glyph,
                    TextSource::Fluent,
                    t.type_ramp.mono_micro,
                    status_ink.to_hsla(),
                )));
            }
            row = row.child(div().flex_1());
            if let Some(meta) = data.meta {
                row = row.child(div().flex_none().child(Label::new(
                    meta,
                    TextSource::Data,
                    t.type_ramp.mono_micro,
                    c.text_2.to_hsla(),
                )));
            }
            if selected {
                row = row.bg(c.blue_tint.to_hsla());
            }
            let row = row
                .id(("hv-tree-row", ix))
                .declare_full(decl)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        this.on_row_mouse_down(key, e, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        this.on_row_mouse_down(key, e, window, cx);
                    }),
                );
            root = root.child(row);
        }
        root
    }
}
