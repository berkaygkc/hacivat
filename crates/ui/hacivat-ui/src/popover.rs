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

/// One recorded accessibility declaration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct A11yDecl {
    pub role: Option<Role>,
    pub name: SharedString,
    pub selected: Option<bool>,
    pub description: Option<SharedString>,
    pub value: Option<SharedString>,
    pub live: Option<accesskit::Live>,
    pub disabled: bool,
    pub modal: bool,
    pub active_descendant: bool,
}

impl A11yDecl {
    pub fn new(role: Role, name: SharedString) -> Self {
        Self {
            role: Some(role),
            name,
            ..Self::default()
        }
    }

    pub fn selected(mut self, selected: Option<bool>) -> Self {
        self.selected = selected;
        self
    }

    pub fn description(mut self, description: SharedString) -> Self {
        self.description = Some(description);
        self
    }

    pub fn active_descendant(mut self) -> Self {
        self.active_descendant = true;
        self
    }
}

thread_local! {
    static DECLARED: RefCell<Option<Vec<A11yDecl>>> = const { RefCell::new(None) };
}

/// Starts (or restarts) recording every declaration the widgets make on this thread. GPUI
/// builds its accessibility tree only while assistive technology is attached, and the test
/// platforms never attach one, so tests assert on the declarations instead.
pub fn a11y_record() {
    DECLARED.with(|d| *d.borrow_mut() = Some(Vec::new()));
}

/// Stops recording and returns the declarations made since [`a11y_record`].
pub fn a11y_take() -> Vec<A11yDecl> {
    DECLARED.with(|d| d.borrow_mut().take().unwrap_or_default())
}

fn a11y_note(decl: &A11yDecl) {
    DECLARED.with(|d| {
        if let Some(log) = d.borrow_mut().as_mut() {
            log.push(decl.clone());
        }
    });
}

/// `.declare(role, name, selected)`: sets the AccessKit role and accessible name (and the
/// selected state) of a stateful element, and records the declaration for tests.
pub trait Declare: StatefulInteractiveElement + Sized {
    fn declare(self, role: Role, name: SharedString, selected: Option<bool>) -> Self {
        self.declare_full(A11yDecl::new(role, name).selected(selected))
    }

    /// Like `declare`, with the description and active-descendant extras GPUI supports.
    fn declare_full(self, decl: A11yDecl) -> Self {
        a11y_note(&decl);
        let mut el = self.role(decl.role.unwrap_or(Role::GenericContainer));
        el = el.aria_label(decl.name.clone());
        if let Some(selected) = decl.selected {
            el = el.aria_selected(selected);
        }
        if let Some(description) = decl.description {
            el = el.aria_description(description);
        }
        if decl.active_descendant {
            el = el.aria_active_descendant();
        }
        el
    }
}

impl<E: StatefulInteractiveElement> Declare for E {}

/// Wraps `child` in an accessibility node. GPUI's `.role(..)` cannot mark a node modal,
/// disabled, live or valued, so the elements that need those go through this one.
pub struct A11yNode {
    id: ElementId,
    decl: A11yDecl,
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
        let mut decl = A11yDecl::new(role, label);
        decl.modal = modal;
        Self {
            id: id.into(),
            decl,
            child: child.into_any_element(),
        }
    }

    /// The node is announced when its value changes (toasts).
    pub fn live(mut self, live: accesskit::Live) -> Self {
        self.decl.live = Some(live);
        self
    }

    pub fn value(mut self, value: SharedString) -> Self {
        self.decl.value = Some(value);
        self
    }

    pub fn description(mut self, description: SharedString) -> Self {
        self.decl.description = Some(description);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.decl.disabled = disabled;
        self
    }

    pub fn selected(mut self, selected: Option<bool>) -> Self {
        self.decl.selected = selected;
        self
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
        a11y_note(&self.decl);
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
        self.decl.role
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        let d = &self.decl;
        node.set_label(d.name.to_string());
        if d.modal {
            node.set_modal();
        }
        if d.disabled {
            node.set_disabled();
        }
        if let Some(selected) = d.selected {
            node.set_selected(selected);
        }
        if let Some(description) = &d.description {
            node.set_description(description.to_string());
        }
        if let Some(value) = &d.value {
            node.set_value(value.to_string());
        }
        if let Some(live) = d.live {
            node.set_live(live);
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
