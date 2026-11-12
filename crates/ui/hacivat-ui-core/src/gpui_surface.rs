//! Re-exports of the GPUI types the spikes use.

// `ScrollWheelEvent` is a deprecated alias of `ScrollEvent` at the pin; re-export it under the
// stable name without tripping the deprecation lint.
pub use gpui::ScrollEvent as ScrollWheelEvent;
pub use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Hsla,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, SharedString, Size, Styled,
    UTF16Selection, Window, canvas, div, px,
};
// Task 6 (spike-editor): custom element, text shaping, popover shadow and synthetic keystrokes.
#[rustfmt::skip] pub use gpui::{AnyElement, AppContext, AvailableSpace, BoxShadow, ContentMask, Element, ElementId, Font, GlobalElementId, InspectorElementId, InteractiveElement, Keystroke, LayoutId, Modifiers, ShapedLine, Style, TextAlign, TextRun, fill, point, relative, size};
