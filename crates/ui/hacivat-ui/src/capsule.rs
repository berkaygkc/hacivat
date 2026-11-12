//! The title-bar connection capsule (DESIGN.md, Title-Bar Capsule): a 26 px segmented capsule,
//! 4 px radius. The production block is the only `cp_fill` quad in a window, with its label
//! in `capsule_env` small caps; it arrives pre-cased from Fluent (`PRODUCTION`, `PROD`).

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Role, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeTokens};

use crate::button::ClickHandler;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::Declare;
use crate::popover::lazy;
use crate::text::{Label, TextSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvKind {
    Production,
    Staging,
    Local,
    Other,
}

/// The 7 px environment swatch: production a filled red square, staging a hollow square,
/// local a hollow circle. `Other` has no mark (an empty 0 px element).
pub fn env_swatch(kind: EnvKind, theme: &Theme) -> Div {
    let c = &theme.tokens.colors;
    let base = div().size(px(7.0)).flex_none();
    match kind {
        EnvKind::Production => base.bg(c.red.to_hsla()),
        EnvKind::Staging => base.border_1().border_color(c.text_2.to_hsla()),
        EnvKind::Local => base
            .border_1()
            .border_color(c.text_3.to_hsla())
            .rounded_full(),
        EnvKind::Other => div().size(px(0.0)).flex_none(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsuleModel {
    /// No connection yet.
    Placeholder { label: SharedString },
    Connection {
        env: EnvKind,
        /// Pre-cased by Fluent (`PRODUCTION` in English, `PROD` in Turkish).
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        access: SharedString,
        path: SharedString,
    },
}

pub struct Capsule {
    model: CapsuleModel,
    on_click: Option<ClickHandler>,
}

impl Capsule {
    pub fn new(model: CapsuleModel) -> Self {
        Self {
            model,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// One cell after the environment block: 9 px padding, a `seam_strong` divider on its left.
fn cell(t: &ThemeTokens) -> Div {
    let c = &t.colors;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(t.spacing.sm))
        .px(px(9.0))
        .h_full()
        .flex_none()
        .border_l_1()
        .border_color(c.seam_strong.to_hsla())
        .group_hover("capsule", |s| s.bg(c.hover.to_hsla()))
}

impl IntoElement for Capsule {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let body = t.type_ramp.body;
            let strong = hacivat_ui_core::theme::TypeStyle {
                weight: 600,
                ..body
            };
            let text = |s: &SharedString, style, color: hacivat_ui_core::gpui_surface::Hsla| {
                Label::new(s.clone(), TextSource::Fluent, style, color)
            };
            let mut frame = div()
                .id(ElementId::from("capsule"))
                .group("capsule")
                .debug_selector(|| "capsule".to_string())
                .flex()
                .flex_row()
                .items_center()
                .flex_none()
                .h(px(26.0))
                .border_1()
                .border_color(c.seam_strong.to_hsla())
                .rounded(px(t.radii.sm))
                .overflow_hidden()
                .bg(c.panel.to_hsla());
            let name_for_label;
            match &self.model {
                CapsuleModel::Placeholder { label } => {
                    name_for_label = label.clone();
                    frame = frame.child(div().flex().items_center().px(px(9.0)).child(text(
                        label,
                        body,
                        c.text_3.to_hsla(),
                    )));
                }
                CapsuleModel::Connection {
                    env,
                    env_label,
                    name,
                    engine,
                    access,
                    path,
                } => {
                    name_for_label = name.clone();
                    let production = *env == EnvKind::Production;
                    if production {
                        // The one cp_fill quad in the window (named rule).
                        frame = frame.child(
                            div()
                                .debug_selector(|| "cp-fill".to_string())
                                .flex()
                                .flex_row()
                                .items_center()
                                .h_full()
                                .flex_none()
                                .pl(px(9.0))
                                .pr(px(10.0))
                                .bg(c.cp_fill.to_hsla())
                                .child(text(
                                    env_label,
                                    t.type_ramp.capsule_env,
                                    c.on_cp_fill.to_hsla(),
                                )),
                        );
                    } else {
                        frame = frame.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(t.spacing.sm))
                                .h_full()
                                .flex_none()
                                .pl(px(9.0))
                                .pr(px(10.0))
                                .child(env_swatch(*env, &theme))
                                .child(text(env_label, strong, c.text.to_hsla())),
                        );
                    }
                    let lock = if production { c.red } else { c.text_2 };
                    frame = frame
                        .child(
                            cell(t)
                                .child(text(name, strong, c.text.to_hsla()))
                                .child(text(engine, body, c.text_2.to_hsla())),
                        )
                        .child(
                            cell(t)
                                .child(
                                    Icon::new(IconName::Lock)
                                        .size(IconSize::Chrome)
                                        .color(lock.to_hsla()),
                                )
                                .child(text(access, body, c.text.to_hsla())),
                        )
                        .child(
                            cell(t)
                                .child(
                                    Icon::new(IconName::ArrowLeftRight)
                                        .size(IconSize::Chrome)
                                        .color(c.green.to_hsla()),
                                )
                                .child(text(path, body, c.text.to_hsla())),
                        );
                }
            }
            frame = frame.child(
                cell(t).px(px(6.0)).child(
                    Icon::new(IconName::ChevronsUpDown)
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                ),
            );
            let mut frame = frame
                .declare(Role::Button, name_for_label, None)
                .cursor_pointer();
            if let Some(handler) = self.on_click {
                frame = frame.on_click(move |event, window, cx| handler(event, window, cx));
            }
            frame.into_any_element()
        })
        .into_any_element()
    }
}
