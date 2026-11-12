//! Inline notice (NOTES "NEW primitives"): padding 8/10, radius 4, 1 px `seam-strong`, a `panel`
//! / `ground` well; lines at 12.5 px `text`, secondary lines `text-2`, parameter rows with the
//! parameter in Fragment Mono 11.5 `text` and the rest in `text-2`. The 14 px icon is `text-2`
//! (info), or a `green` circle-check when the notice reports something done. Actions are
//! existing [`Button`]s. Announced as a status.

use hacivat_ui_core::gpui_surface::{
    AnyElement, InteractiveElement, IntoElement, ParentElement, Role, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode};

use crate::button::Button;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yNode, lazy};
use crate::text::{Label, TextSource};

/// The gap between the icon and the text column: 9 px, between the `md` (8) and `lg` (10)
/// tokens (NOTES "NEW primitives": inline notice; mock `.notice { gap: 9px }`).
const ICON_GAP_PX: f32 = 9.0;
/// Drops the 14 px icon onto the first line's cap height (mock `.notice > svg { margin-top: 1px }`).
const ICON_NUDGE_PX: f32 = 1.0;

/// The notice's icon (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeIcon {
    Info,
    Done,
}

/// A notice inside a form (NOTES "NEW primitives").
pub struct InlineNotice {
    icon: NoticeIcon,
    lines: Vec<SharedString>,
    secondary: Vec<SharedString>,
    params: Vec<(SharedString, SharedString)>,
    actions: Vec<Button>,
}

impl InlineNotice {
    pub fn new(icon: NoticeIcon) -> Self {
        InlineNotice {
            icon,
            lines: Vec::new(),
            secondary: Vec::new(),
            params: Vec::new(),
            actions: Vec::new(),
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

    /// A parameter row: `key` (machine text, `sslmode=verify-full`) in mono, then `rest`.
    pub fn param(mut self, key: SharedString, rest: SharedString) -> Self {
        self.params.push((key, rest));
        self
    }

    /// Adds a button to the action row (call once per button).
    pub fn action(mut self, button: Button) -> Self {
        self.actions.push(button);
        self
    }
}

impl IntoElement for InlineNotice {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let well = match theme.mode {
                ThemeMode::Dark => c.panel,
                ThemeMode::Light => c.ground,
            };
            let icon = match self.icon {
                NoticeIcon::Info => Icon::new(IconName::Info).color(c.text_2.to_hsla()),
                NoticeIcon::Done => Icon::new(IconName::CircleCheck).color(c.green.to_hsla()),
            }
            .size(IconSize::Default);
            let name = self.lines.first().cloned().unwrap_or_default();
            let line = |text: SharedString, ink| {
                Label::new(text, TextSource::Fluent, t.type_ramp.body_sm, ink)
            };
            let mut body = div().flex().flex_col().flex_1().min_w_0();
            for text in self.lines {
                body = body.child(line(text, c.text.to_hsla()));
            }
            for text in self.secondary {
                body = body.child(line(text, c.text_2.to_hsla()));
            }
            if !self.params.is_empty() {
                let mut list = div().flex().flex_col().mt(px(t.spacing.xs));
                for (key, rest) in self.params {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(t.spacing.sm))
                            .child(Label::new(
                                key,
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                            ))
                            .child(line(rest, c.text_2.to_hsla())),
                    );
                }
                body = body.child(list);
            }
            if !self.actions.is_empty() {
                body = body.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(t.spacing.sm))
                        .mt(px(t.spacing.md))
                        .children(self.actions),
                );
            }
            let card = div()
                .debug_selector(|| "inline-notice".to_string())
                .flex()
                .flex_row()
                .items_start()
                .gap(px(ICON_GAP_PX))
                .px(px(t.spacing.lg))
                .py(px(t.spacing.md))
                .rounded(px(t.radii.sm))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .bg(well.to_hsla())
                .child(div().flex_none().mt(px(ICON_NUDGE_PX)).child(icon))
                .child(body);
            A11yNode::new("inline-notice", Role::Status, name, false, card).into_any_element()
        })
        .into_any_element()
    }
}
