//! Choice list, radio rows (NOTES "NEW primitives"): a group with a 1 px `seam-strong` border,
//! radius 4, rows divided by `seam`. Each row is at least 34 px, padding 7/10: a 14 px radio
//! (1 px `seam-strong` on the well; selected: `blue` border and a 6 px `blue` dot), the 7 px
//! environment mark, the label at 13 px weight 600 and the description at 12.5 px `text-2`. The
//! selected row is `blue-tint`, a hovered row `hover`. One tab stop; ↑/↓ move and select (they
//! wrap), Space selects the first row when none is. Rows are `choice-<index>` for tests.

use hacivat_ui_core::gpui_surface::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, Role, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use crate::capsule::{EnvKind, env_swatch};
use crate::popover::{A11yDecl, Declare};
use crate::text::{Label, TextFn, TextSource};

/// The minimum row height.
pub const CHOICE_ROW_MIN_PX: f32 = 34.0;
const RADIO_PX: f32 = 14.0;
const DOT_PX: f32 = 6.0;
const MARK_COLUMN_PX: f32 = 7.0;
const ROW_PAD_Y_PX: f32 = 7.0;

/// One choice (NOTES "NEW primitives").
#[derive(Clone)]
pub struct Choice {
    pub mark: Option<EnvKind>,
    pub label: TextFn,
    pub description: TextFn,
}

/// What a [`ChoiceList`] emits (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChoiceEvent {
    Selected(usize),
}

/// A list of described choices (NOTES "NEW primitives").
pub struct ChoiceList {
    choices: Vec<Choice>,
    selected: Option<usize>,
    focus: FocusHandle,
    /// The radio group's accessible name.
    label: Option<TextFn>,
}

impl EventEmitter<ChoiceEvent> for ChoiceList {}

impl Focusable for ChoiceList {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ChoiceList {
    pub fn new(choices: Vec<Choice>, selected: Option<usize>, cx: &mut Context<Self>) -> Self {
        ChoiceList {
            choices,
            selected,
            focus: cx.focus_handle().tab_stop(true),
            label: None,
        }
    }

    /// Names the radio group for assistive technology (the question it answers).
    pub fn set_label(&mut self, label: TextFn) {
        self.label = Some(label);
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.choices.len() && self.selected != Some(index) {
            self.selected = Some(index);
            cx.emit(ChoiceEvent::Selected(index));
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        let n = self.choices.len();
        if n == 0 || stroke.modifiers.modified() {
            return;
        }
        let next = match (stroke.key.as_str(), self.selected) {
            ("down", None) | ("space", None) => 0,
            ("up", None) => n - 1,
            ("down", Some(i)) => (i + 1) % n,
            ("up", Some(i)) => (i + n - 1) % n,
            _ => return,
        };
        window.prevent_default();
        cx.stop_propagation();
        self.select(next, cx);
    }
}

impl Render for ChoiceList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let focused = self.focus.is_focused(window) && window.last_input_was_keyboard();

        let mut group = div()
            .id("choice-list")
            .debug_selector(|| "choice-list".to_string())
            .track_focus(&self.focus)
            .declare(
                Role::RadioGroup,
                self.label.as_ref().map(|l| l(cx)).unwrap_or_default(),
                None,
            )
            .on_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .w_full()
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(t.radii.sm))
            .overflow_hidden();
        for (i, choice) in self.choices.iter().enumerate() {
            let selected = self.selected == Some(i);
            let label = (choice.label)(cx);
            let mut radio = div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(RADIO_PX))
                .rounded_full()
                .border_1()
                .bg(well.to_hsla())
                .border_color(c.seam_strong.to_hsla());
            if selected {
                radio = radio
                    .border_color(c.blue.to_hsla())
                    .child(div().size(px(DOT_PX)).rounded_full().bg(c.blue.to_hsla()));
            }
            let mark = div()
                .flex()
                .flex_none()
                .w(px(MARK_COLUMN_PX))
                .children(choice.mark.map(|m| env_swatch(m, &theme)));
            let mut row = div()
                .id(("choice", i))
                .debug_selector(move || format!("choice-{i}"))
                .declare_full(
                    A11yDecl::new(Role::RadioButton, label.clone())
                        .selected(Some(selected))
                        .toggled(selected),
                )
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.lg))
                .min_h(px(CHOICE_ROW_MIN_PX))
                .px(px(t.spacing.lg))
                .py(px(ROW_PAD_Y_PX))
                .cursor_pointer()
                .child(radio)
                .child(mark)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(Label::new(
                            label,
                            TextSource::Fluent,
                            t.type_ramp.body_strong,
                            c.text.to_hsla(),
                        ))
                        .child(Label::new(
                            (choice.description)(cx),
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_2.to_hsla(),
                        )),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select(i, cx);
                }));
            if i > 0 {
                row = row.border_t_1().border_color(c.seam.to_hsla());
            }
            row = if selected {
                row.bg(c.blue_tint.to_hsla())
            } else {
                row.hover(|style| style.bg(c.hover.to_hsla()))
            };
            group = group.child(row);
        }
        // The rows clip to the rounded group, so the ring goes around the group.
        focus_outline(group, &theme, focused)
    }
}
