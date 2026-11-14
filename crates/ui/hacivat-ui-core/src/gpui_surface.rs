//! Re-exports of the GPUI types the rest of the workspace uses. Production code outside this
//! crate reaches GPUI only through this module (§5.1).

pub use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Hsla,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, ScrollEvent, SharedString,
    Size, Styled, UTF16Selection, Window, canvas, div, px,
};
// Tasks 5, 6, 7, 8, 9 (grid, editor, IME, harman VF, probes): custom elements, text shaping,
// scroll input, popover shadow, accessibility, clipboard, synthetic keystrokes, input handler.
#[rustfmt::skip] pub use gpui::{AnyElement, AppContext, AvailableSpace, BoxShadow, ClipboardEntry, ContentMask, Div, Element, ElementId, ElementInputHandler, Font, FontFeatures, FontId, FontStyle, FontWeight, GlobalElementId, HighlightStyle, InspectorElementId, InteractiveElement, Keystroke, LayoutId, Modifiers, Role, ShapedLine, Style, StyledText, TextAlign, TextRun, UnderlineStyle, WindowHandle, accesskit, fill, hsla, point, relative, size, svg};
// M1 shell: globals, the one generic action (F1), application menus, window chrome, events.
#[rustfmt::skip] pub use gpui::{Action, AnyView, AnyWindowHandle, Axis, ClickEvent, Decorations, EntityId, EventEmitter, FocusOutEvent, Global, KeyBinding, Menu, MenuItem, MouseButton, MouseDownEvent, OsAction, Overflow, Subscription, SystemMenuType, Task, WindowAppearance, WindowControlArea, WindowDecorations};
// Task 13: Linux client-side decoration resize edges (`Window::start_window_resize`).
pub use gpui::ResizeEdge;
// M2a Task 21: a scroll viewport's offset and size (the connection sheet's scroll thumb).
pub use gpui::ScrollHandle;
// Widgets: stateful elements (`role`, `aria_label`, `aria_selected`, `on_click`, `on_drag`,
// `tooltip` live on `StatefulInteractiveElement` at the pin), builders, pointer events.
#[rustfmt::skip] pub use gpui::{AssetSource, DragMoveEvent, MouseMoveEvent, MouseUpEvent, Stateful, StatefulInteractiveElement, WeakEntity};
// M2a (Task 1): anchored popovers and selects (`deferred(anchored())`), file pickers.
pub use crate::actions::{InertCommand, RunCommand};
// ## m3a-t01b-seams: the editor and grid elements (Tasks 19, 20, 27, 28): hitboxes and cursor
// styles, view caching, the accessibility subtree, the text input configuration, the
// clipboard's string with metadata, pointer and modifier events, quads and their edges and
// corners, scroll deltas, strikethrough runs.
#[rustfmt::skip] pub use gpui::{A11ySubtreeBuilder, Autocapitalize, BorderStyle, ClipboardString, Corners, CursorStyle, Edges, Hitbox, HitboxBehavior, ModifiersChangedEvent, MouseExitEvent, PaintQuad, ScrollDelta, StrikethroughStyle, StyleRefinement, TextInputConfiguration, TouchPhase, quad};
/// `.when(..)`/`.map(..)` builders (`gpui::prelude::FluentBuilder` at the pin).
pub use gpui::prelude::FluentBuilder;
pub use gpui::{Anchor, AnchoredPositionMode, PathPromptOptions, anchored, deferred};
/// GPUI's test contexts, for downstream tests (`test-support`).
#[cfg(feature = "test-support")]
pub use gpui::{HeadlessAppContext, TestAppContext, VisualTestContext};
/// Headless capture (R8, Task 18): `HeadlessAppContext::with_platform(
/// current_platform(true).text_system(), assets, current_headless_renderer)`.
#[cfg(feature = "test-support")]
pub use gpui::{Platform, PlatformHeadlessRenderer, PlatformTextSystem};
#[cfg(feature = "test-support")]
pub use gpui_platform::{current_headless_renderer, current_platform};

#[cfg(test)]
mod tests {
    use super::*;

    /// Widgets reach the stateful-element API (accessibility, click, tooltip) and the pointer
    /// event types through this module alone (no `gpui::` path below).
    #[test]
    fn stateful_element_api_is_reachable_through_the_surface() {
        struct Host;
        fn widget() -> Stateful<Div> {
            div()
                .id("save")
                .role(Role::Button)
                .aria_label("Save")
                .aria_selected(false)
                .on_click(|_: &ClickEvent, _: &mut Window, _: &mut App| {})
                .when(true, |el| el.cursor_pointer())
        }
        fn handlers(
            _: &MouseUpEvent,
            _: &MouseMoveEvent,
            _: &DragMoveEvent<Host>,
            _: WeakEntity<Host>,
        ) {
        }
        let _ = (widget, handlers);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn headless_capture_entry_points_are_reachable() {
        let _: fn(bool) -> std::rc::Rc<dyn Platform> = current_platform;
        let _: fn() -> anyhow::Result<Option<Box<dyn PlatformHeadlessRenderer>>> =
            current_headless_renderer;
    }

    /// Spikes name the scroll event by GPUI's current name.
    #[test]
    fn scroll_event_is_gpuis_current_type() {
        fn same(e: super::ScrollEvent) -> gpui::ScrollEvent {
            e
        }
        let _ = same;
    }
}
