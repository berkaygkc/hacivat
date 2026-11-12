//! The title-bar connection capsule (DESIGN.md, Title-Bar Capsule): a 26 px segmented capsule,
//! 4 px radius. The production block is the only `cp_fill` quad in a window, with its label
//! in `capsule_env` small caps; it arrives pre-cased from Fluent (`PRODUCTION`, `PROD`).

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, Div, ElementId, InteractiveElement, IntoElement, ParentElement,
    Pixels, Point, Role, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeTokens};

use crate::button::ClickHandler;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::lazy;
use crate::popover::{A11yDecl, Declare};
use crate::text::{Label, TextSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvKind {
    Production,
    Staging,
    Local,
    Other,
    // M2a (Task 1); their marks arrive with Task 9.
    Dev,
    Test,
}

/// The 7 px environment swatch: production a filled red square, staging a hollow square,
/// local a hollow circle. `Other` has no mark (an empty 0 px element); `Dev` and `Test` have
/// none until Task 9.
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
        EnvKind::Other | EnvKind::Dev | EnvKind::Test => div().size(px(0.0)).flex_none(),
    }
}

/// A clickable part of the capsule (M2a, NOTES surface 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapsuleSegment {
    Body,
    Retry,
    Unencrypted,
}

/// Called with the clicked segment and the bottom-left of its bounds, the anchor for
/// `open_anchored` (P4).
pub type SegmentHandler = Rc<dyn Fn(CapsuleSegment, Point<Pixels>, &mut Window, &mut App)>;

/// What the capsule shows. Every string is chrome text from Fluent except the connection's
/// name and engine, which are data. The M2a states render from Task 9 on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsuleModel {
    /// No connection yet.
    Placeholder { label: SharedString },
    /// A connect in progress.
    Connecting {
        env: EnvKind,
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        label: SharedString,
        step: SharedString,
        clock: SharedString,
    },
    Connection {
        env: EnvKind,
        /// Pre-cased by Fluent (`PRODUCTION` in English, `PROD` in Turkish).
        env_label: SharedString,
        name: SharedString,
        engine: SharedString,
        access: SharedString,
        read_only: bool,
        path: SharedString,
        direct: bool,
        latency: Option<SharedString>,
        /// The unencrypted badge's label, when the connection has no TLS.
        unencrypted: Option<SharedString>,
        /// The unsupported-version badge's label.
        unsupported: Option<SharedString>,
    },
    /// The last connect failed.
    Failed {
        label: SharedString,
        name: SharedString,
        retry: SharedString,
    },
}

pub struct Capsule {
    model: CapsuleModel,
    on_click: Option<ClickHandler>,
    #[allow(dead_code, reason = "Task 9")]
    on_segment: Option<SegmentHandler>,
}

impl Capsule {
    pub fn new(model: CapsuleModel) -> Self {
        Self {
            model,
            on_click: None,
            on_segment: None,
        }
    }

    /// Called when a segment is clicked, with its bottom-left corner (Task 9 wires it).
    pub fn on_segment(
        mut self,
        handler: impl Fn(CapsuleSegment, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_segment = Some(Rc::new(handler));
        self
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
            let label =
                |source, s: &SharedString, style, color: hacivat_ui_core::gpui_surface::Hsla| {
                    Label::new(s.clone(), source, style, color)
                };
            // Chrome text is Fluent; the connection's name and engine are user data.
            let text = |s: &SharedString, style, color| label(TextSource::Fluent, s, style, color);
            let data = |s: &SharedString, style, color| label(TextSource::Data, s, style, color);
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
            let mut description = None;
            match &self.model {
                // Task 9 draws the connecting and failed states; until then they show their
                // label like the placeholder.
                CapsuleModel::Placeholder { label }
                | CapsuleModel::Connecting { label, .. }
                | CapsuleModel::Failed { label, .. } => {
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
                    ..
                } => {
                    name_for_label = name.clone();
                    description =
                        Some(SharedString::from(format!("{env_label}, {access}, {path}")));
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
                                .rounded_tl(px(t.radii.xs))
                                .rounded_bl(px(t.radii.xs))
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
                                .child(data(name, strong, c.text.to_hsla()))
                                .child(data(engine, body, c.text_2.to_hsla())),
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
            let mut decl = A11yDecl::new(Role::Button, name_for_label);
            decl.description = description;
            let mut frame = frame.declare_full(decl).cursor_pointer();
            if let Some(handler) = self.on_click {
                frame = frame.on_click(move |event, window, cx| handler(event, window, cx));
            }
            frame.into_any_element()
        })
        .into_any_element()
    }
}
