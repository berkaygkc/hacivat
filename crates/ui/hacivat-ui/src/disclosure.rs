//! Disclosure (NOTES "NEW primitives"): a titled section that folds, showing a one-line summary
//! while closed. Task 6 draws it; the Task 1 stub renders an empty `div`.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, IntoElement, SharedString, Window, div,
};

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A folding section (NOTES "NEW primitives").
#[allow(dead_code, reason = "Task 6")]
pub struct Disclosure {
    id: ElementId,
    title: SharedString,
    summary: Option<SharedString>,
    open: bool,
    on_toggle: Option<ToggleHandler>,
    content: Option<AnyElement>,
}

impl Disclosure {
    pub fn new(id: impl Into<ElementId>, title: SharedString) -> Self {
        Disclosure {
            id: id.into(),
            title,
            summary: None,
            open: false,
            on_toggle: None,
            content: None,
        }
    }

    /// The line shown while closed.
    pub fn summary(mut self, summary: SharedString) -> Self {
        self.summary = Some(summary);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Called with the new open state.
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    /// What shows while open.
    pub fn content(mut self, content: AnyElement) -> Self {
        self.content = Some(content);
        self
    }
}

impl IntoElement for Disclosure {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        div().into_any_element()
    }
}
