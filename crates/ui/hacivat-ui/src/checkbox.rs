//! Checkbox (NOTES "NEW primitives"): a box, a label and an optional sub-label; Space and click
//! toggle it. Task 6 draws it; the Task 1 stub renders an empty `div`.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, SharedString, Window, div,
};

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A labelled checkbox (NOTES "NEW primitives").
#[allow(dead_code, reason = "Task 6")]
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    sub_label: Option<SharedString>,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_toggle: Option<ToggleHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, label: SharedString) -> Self {
        Checkbox {
            id: id.into(),
            label,
            checked: false,
            sub_label: None,
            disabled: false,
            focus: None,
            on_toggle: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// A second line under the label.
    pub fn sub_label(mut self, sub_label: SharedString) -> Self {
        self.sub_label = Some(sub_label);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn focus_handle(mut self, handle: FocusHandle) -> Self {
        self.focus = Some(handle);
        self
    }

    /// Called with the new state.
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl IntoElement for Checkbox {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        div().into_any_element()
    }
}
