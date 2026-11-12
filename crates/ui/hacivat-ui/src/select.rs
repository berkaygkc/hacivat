//! Select (NOTES "NEW primitives"): a field that opens the M1 `PopupMenu` anatomy in its own
//! `deferred(anchored())` (P34). Task 6 draws it; the Task 1 stub renders an empty `div`.

use hacivat_ui_core::gpui_surface::{
    Context, EventEmitter, FocusHandle, IntoElement, Render, Window, div,
};

use crate::text::TextFn;

/// One choice of a [`Select`] (NOTES "NEW primitives").
#[derive(Clone)]
pub struct SelectOption {
    pub label: TextFn,
    /// A second, dimmer line (`(Keychain)`).
    pub hint: Option<TextFn>,
    pub disabled: bool,
}

/// What a [`Select`] emits (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectEvent {
    /// The user picked another option.
    Changed(usize),
}

/// A drop-down choice (NOTES "NEW primitives").
pub struct Select {
    #[allow(dead_code, reason = "Task 6")]
    options: Vec<SelectOption>,
    selected: usize,
    open: bool,
    #[allow(dead_code, reason = "Task 6")]
    focus: FocusHandle,
}

impl EventEmitter<SelectEvent> for Select {}

impl Select {
    pub fn new(options: Vec<SelectOption>, selected: usize, cx: &mut Context<Self>) -> Self {
        Select {
            options,
            selected,
            open: false,
            focus: cx.focus_handle(),
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects `index` without emitting.
    pub fn set_selected(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        cx.notify();
    }

    pub fn set_options(&mut self, options: Vec<SelectOption>, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

impl Render for Select {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
