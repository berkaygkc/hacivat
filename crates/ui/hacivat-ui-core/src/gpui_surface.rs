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
pub use crate::actions::RunCommand;
/// GPUI's test contexts, for downstream tests (`test-support`).
#[cfg(feature = "test-support")]
pub use gpui::{HeadlessAppContext, TestAppContext, VisualTestContext};

#[cfg(test)]
mod tests {
    /// Spikes name the scroll event by GPUI's current name.
    #[test]
    fn scroll_event_is_gpuis_current_type() {
        fn same(e: super::ScrollEvent) -> gpui::ScrollEvent {
            e
        }
        let _ = same;
    }
}
