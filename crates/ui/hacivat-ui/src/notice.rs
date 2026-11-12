//! Inline notice (NOTES "NEW primitives"): an icon, lines of text, mono parameters and an
//! optional action, inside a form. Task 6 draws it; the Task 1 stub renders an empty `div`.

use hacivat_ui_core::gpui_surface::{AnyElement, IntoElement, SharedString, div};

use crate::button::Button;

/// The notice's icon (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeIcon {
    Info,
    Done,
}

/// A notice inside a form (NOTES "NEW primitives").
#[allow(dead_code, reason = "Task 6")]
pub struct InlineNotice {
    icon: NoticeIcon,
    lines: Vec<SharedString>,
    secondary: Vec<SharedString>,
    params: Vec<(SharedString, SharedString)>,
    action: Option<Button>,
}

impl InlineNotice {
    pub fn new(icon: NoticeIcon) -> Self {
        InlineNotice {
            icon,
            lines: Vec::new(),
            secondary: Vec::new(),
            params: Vec::new(),
            action: None,
        }
    }

    pub fn line(mut self, line: SharedString) -> Self {
        self.lines.push(line);
        self
    }

    /// A dimmer line.
    pub fn secondary(mut self, line: SharedString) -> Self {
        self.secondary.push(line);
        self
    }

    /// A parameter row: `key` in mono, then `rest`.
    pub fn param(mut self, key: SharedString, rest: SharedString) -> Self {
        self.params.push((key, rest));
        self
    }

    pub fn action(mut self, button: Button) -> Self {
        self.action = Some(button);
        self
    }
}

impl IntoElement for InlineNotice {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        div().into_any_element()
    }
}
