//! Re-exports of the GPUI types the spikes use.

// `ScrollWheelEvent` is a deprecated alias of `ScrollEvent` at the pin; re-export it under the
// stable name without tripping the deprecation lint.
pub use gpui::ScrollEvent as ScrollWheelEvent;
pub use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle, Focusable, Hsla,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, SharedString, Size, Styled,
    UTF16Selection, Window, canvas, div, px,
};
// Task 9 (probes): text shaping, accessibility and clipboard types.
pub use gpui::{
    AppContext, ClipboardEntry, Element, ElementId, Font, FontFeatures, FontWeight,
    GlobalElementId, InspectorElementId, LayoutId, Role, Style, TextRun, WindowHandle, accesskit,
    hsla,
};
