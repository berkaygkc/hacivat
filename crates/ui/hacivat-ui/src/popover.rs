//! Plumbing shared by the chrome widgets: [`lazy`] elements (a widget builds its element tree
//! when GPUI lays it out, so it can read the `Theme` and the UI text provider) and the
//! [`Tip`] tooltip view.
//!
//! `gpui::RenderOnce` is not part of `gpui_surface`, so widgets that are plain builders
//! (`Button::new(..)`) become elements through [`lazy`] instead of `#[derive(IntoElement)]`.

use std::cell::RefCell;
use std::panic::Location;

use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, ParentElement, Pixels, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, accesskit, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::text::{Label, TextSource};

/// An element whose tree is built from `(window, cx)` at layout time.
pub struct Lazy<F> {
    build: Option<F>,
}

/// Wraps `build` as an element. `build` runs once, during the frame's layout phase.
pub fn lazy<F>(build: F) -> Lazy<F>
where
    F: FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
{
    Lazy { build: Some(build) }
}

impl<F> Element for Lazy<F>
where
    F: FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
{
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let build = self.build.take().expect("a lazy element is laid out once");
        let mut element = build(window, cx);
        let layout = element.request_layout(window, cx);
        (layout, element)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        element: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        element: &mut AnyElement,
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

/// One recorded `(role, name, selected)`.
pub type Declaration = (Role, SharedString, Option<bool>);

thread_local! {
    static DECLARED: RefCell<Option<Vec<Declaration>>> =
        const { RefCell::new(None) };
}

/// Starts (or restarts) recording every `(role, name, selected)` the widgets declare on this
/// thread. GPUI builds its accessibility tree only while assistive technology is attached, and
/// the test platforms never attach one, so tests assert on the declarations instead.
pub fn a11y_record() {
    DECLARED.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

/// Stops recording and returns the declarations made since [`a11y_record`].
pub fn a11y_take() -> Vec<Declaration> {
    DECLARED.with(|d| d.borrow_mut().take().unwrap_or_default())
}

fn a11y_note(role: Role, name: &SharedString, selected: Option<bool>) {
    DECLARED.with(|d| {
        if let Some(log) = d.borrow_mut().as_mut() {
            log.push((role, name.clone(), selected));
        }
    });
}

/// `.declare(role, name, selected)`: sets the AccessKit role and accessible name (and the
/// selected state) of a stateful element, and records the declaration for tests.
pub trait Declare: StatefulInteractiveElement + Sized {
    fn declare(self, role: Role, name: SharedString, selected: Option<bool>) -> Self {
        a11y_note(role, &name, selected);
        let el = self.role(role).aria_label(name);
        match selected {
            Some(selected) => el.aria_selected(selected),
            None => el,
        }
    }
}

impl<E: StatefulInteractiveElement> Declare for E {}

/// Wraps `child` in an accessibility node with `role` and `label`, optionally modal. GPUI's
/// `.role(..)` has no way to mark a node modal, so a dialog goes through this element.
pub struct A11yNode {
    id: ElementId,
    role: Role,
    label: SharedString,
    modal: bool,
    child: AnyElement,
}

impl A11yNode {
    pub fn new(
        id: impl Into<ElementId>,
        role: Role,
        label: SharedString,
        modal: bool,
        child: impl IntoElement,
    ) -> Self {
        a11y_note(role, &label, None);
        Self {
            id: id.into(),
            role,
            label,
            modal,
            child: child.into_any_element(),
        }
    }
}

impl Element for A11yNode {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }

    fn a11y_role(&self) -> Option<Role> {
        Some(self.role)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_label(self.label.to_string());
        if self.modal {
            node.set_modal();
        }
    }
}

/// A one-line tooltip on a popover surface.
pub struct Tip {
    text: SharedString,
}

impl Tip {
    /// The tooltip view for `text`, in the shape `.tooltip(..)` expects.
    pub fn view(text: SharedString, cx: &mut App) -> AnyView {
        cx.new(|_| Tip { text }).into()
    }
}

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        popover_surface(&theme)
            .px(px(8.0))
            .py(px(4.0))
            .child(Label::new(
                self.text.clone(),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                t.colors.text.to_hsla(),
            ))
    }
}
