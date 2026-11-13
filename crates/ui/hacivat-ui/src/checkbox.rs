//! Checkbox (NOTES "NEW primitives"): a 14 × 14 box (radius 3, 1 px `seam-strong`, `panel` /
//! `ground` well; checked: `blue` fill and border with a 12 px `on-accent` check), a 13 px `text`
//! label and an optional `text-2` sub-label. Space and a click toggle it; Enter is left to the
//! dialog. Focus draws the 1 px blue ring. The debug selector is `checkbox-<id>` (the box:
//! `checkbox-<id>-box`).

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use crate::button::{focus_visible, interaction_state};
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare, lazy};
use crate::text::{Label, TextSource};

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A labelled checkbox (NOTES "NEW primitives").
pub struct Checkbox {
    id: ElementId,
    label: SharedString,
    checked: bool,
    sub_label: Option<SharedString>,
    disabled: bool,
    focus: Option<FocusHandle>,
    on_toggle: Option<ToggleHandler>,
}

impl Checkbox {
    pub fn new(id: impl Into<ElementId>, label: SharedString) -> Self {
        Checkbox {
            id: id.into(),
            label,
            checked: false,
            sub_label: None,
            disabled: false,
            focus: None,
            on_toggle: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// A dimmer text after the label.
    pub fn sub_label(mut self, sub_label: SharedString) -> Self {
        self.sub_label = Some(sub_label);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn focus_handle(mut self, handle: FocusHandle) -> Self {
        self.focus = Some(handle);
        self
    }

    /// Called with the new state.
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    fn build(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let Checkbox {
            id,
            label,
            checked,
            sub_label,
            disabled,
            focus,
            on_toggle,
        } = self;
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };

        // A disabled box keeps its outline and well; a disabled check is drawn in text-3
        // (DESIGN.md: disabled controls are exempt from the Raised Ink Rule).
        let box_selector = format!("checkbox-{id}-box");
        let mut mark = div()
            .debug_selector(move || box_selector)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(14.0))
            .rounded(px(t.radii.xs))
            .border_1();
        mark = match (checked, disabled) {
            (true, false) => mark
                .bg(c.blue.to_hsla())
                .border_color(c.blue.to_hsla())
                .child(
                    Icon::new(IconName::Check)
                        .size(IconSize::Chrome)
                        .color(c.on_accent.to_hsla()),
                ),
            (true, true) => mark
                .bg(well.to_hsla())
                .border_color(c.seam_strong.to_hsla())
                .child(
                    Icon::new(IconName::Check)
                        .size(IconSize::Chrome)
                        .color(c.text_3.to_hsla()),
                ),
            (false, _) => mark
                .bg(well.to_hsla())
                .border_color(c.seam_strong.to_hsla()),
        };

        let (ink, sub_ink) = if disabled {
            (c.text_3, c.text_3)
        } else {
            (c.text, c.text_2)
        };
        let selector = format!("checkbox-{id}");
        let mut row = div()
            .id(id.clone())
            .debug_selector(move || selector)
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(t.spacing.md))
            // ## m2a-t21-shots: in a narrow row the sub-label ellipsizes (tooltip: all of
            // it); the box and the label keep their size.
            .flex_shrink(1.0)
            .min_w(px(0.0))
            .child(div().flex_none().child(mark))
            .child(div().flex_none().child(Label::new(
                label.clone(),
                TextSource::Fluent,
                t.type_ramp.body,
                ink.to_hsla(),
            )));
        if let Some(sub) = sub_label.clone() {
            row = row.child(
                Label::new(sub, TextSource::Fluent, t.type_ramp.body, sub_ink.to_hsla())
                    .truncate(true),
            );
        }

        // Assistive technology hears the state (also when disabled) and the sub-label as the
        // description.
        if disabled {
            let mut node = A11yNode::new((id, "node"), Role::CheckBox, label, false, row)
                .disabled(true)
                .toggled(checked);
            if let Some(sub) = sub_label {
                node = node.description(sub);
            }
            return node.into_any_element();
        }
        let mut decl = A11yDecl::new(Role::CheckBox, label).toggled(checked);
        if let Some(sub) = sub_label {
            decl = decl.description(sub);
        }

        let (_, keyed) = interaction_state(&id, window, cx);
        let handle = focus.unwrap_or(keyed).tab_stop(true);
        let focused = focus_visible(&handle, window);
        let toggle_on_key = on_toggle.clone();
        let row = focus_outline(row.track_focus(&handle), &theme, focused)
            .declare_full(decl)
            .cursor_pointer()
            // Space toggles on key down, in the capture phase, and prevents the default so GPUI
            // does not turn its key-up into a second (click) toggle.
            .capture_key_down(move |event: &KeyDownEvent, window, cx| {
                let stroke = &event.keystroke;
                if stroke.key != "space" || stroke.modifiers.modified() {
                    return;
                }
                window.prevent_default();
                cx.stop_propagation();
                if let Some(handler) = toggle_on_key.as_ref().filter(|_| !event.is_held) {
                    handler(!checked, window, cx);
                }
            })
            .on_click(move |event, window, cx| {
                // Keyboard clicks (Enter) belong to the dialog.
                if matches!(event, ClickEvent::Keyboard(_)) {
                    return;
                }
                if let Some(handler) = &on_toggle {
                    handler(!checked, window, cx);
                }
            });
        row.into_any_element()
    }
}

impl IntoElement for Checkbox {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}
