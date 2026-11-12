//! Choice list (NOTES "NEW primitives"): stacked, described options with environment marks;
//! ↑/↓ move and select. Task 6 draws it; the Task 1 stub renders an empty `div`.

use hacivat_ui_core::gpui_surface::{
    Context, EventEmitter, FocusHandle, IntoElement, Render, Window, div,
};

use crate::capsule::EnvKind;
use crate::text::TextFn;

/// One choice (NOTES "NEW primitives").
#[derive(Clone)]
pub struct Choice {
    pub mark: Option<EnvKind>,
    pub label: TextFn,
    pub description: TextFn,
}

/// What a [`ChoiceList`] emits (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChoiceEvent {
    Selected(usize),
}

/// A list of described choices (NOTES "NEW primitives").
pub struct ChoiceList {
    #[allow(dead_code, reason = "Task 6")]
    choices: Vec<Choice>,
    selected: Option<usize>,
    #[allow(dead_code, reason = "Task 6")]
    focus: FocusHandle,
}

impl EventEmitter<ChoiceEvent> for ChoiceList {}

impl ChoiceList {
    pub fn new(choices: Vec<Choice>, selected: Option<usize>, cx: &mut Context<Self>) -> Self {
        ChoiceList {
            choices,
            selected,
            focus: cx.focus_handle(),
        }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
}

impl Render for ChoiceList {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
