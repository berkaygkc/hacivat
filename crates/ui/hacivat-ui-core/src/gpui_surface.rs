//! Re-exports of the GPUI types the spikes use.

// `ScrollWheelEvent` is a deprecated alias of `ScrollEvent` at the pin; re-export it under the
// stable name without tripping the deprecation lint.
pub use gpui::ScrollEvent as ScrollWheelEvent;
pub use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Hsla,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, SharedString, Size, Styled,
    UTF16Selection, Window, canvas, div, px,
};
// Tasks 5, 6, 7, 8, 9 (grid, editor, IME, harman VF, probes): custom elements, text shaping,
// scroll input, popover shadow, accessibility, clipboard, synthetic keystrokes, input handler.
#[rustfmt::skip] pub use gpui::{AnyElement, AppContext, AvailableSpace, BoxShadow, ClipboardEntry, ContentMask, Div, Element, ElementId, ElementInputHandler, Font, FontFeatures, FontId, FontStyle, FontWeight, GlobalElementId, HighlightStyle, InspectorElementId, InteractiveElement, Keystroke, LayoutId, Modifiers, Role, ShapedLine, Style, StyledText, TextAlign, TextRun, UnderlineStyle, WindowHandle, accesskit, fill, hsla, point, relative, size, svg};
