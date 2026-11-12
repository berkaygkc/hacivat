//! Step trail (NOTES "NEW primitives"): the connect steps with their state and durations, in the
//! test strip and the error dialog. An inline list of 20 px items with a 14 px column gap that
//! wraps without separators:
//! - done: a 12 px `green` check, the label in `text` and the duration in mono 11 px `text-2`;
//! - running: the 7 px square `amber` run marker and the label in `text`;
//! - failed: a 12 px ✕ in `text` (no new hue for errors) and the label at weight 600;
//! - pending: a 7 px hollow square in `seam-strong` and the label in `text-2`.
//!
//! The labels and durations come from the caller (Fluent); the trail owns no text. Debug
//! selectors: `step-trail`, `trail-<i>`, `trail-run-<i>` and `trail-pending-<i>`.

use hacivat_ui_core::gpui_surface::{
    AnyElement, Div, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::icon::{Icon, IconName, IconSize};
use crate::popover::lazy;
use crate::text::{Label, TextSource};

/// The height of one trail item (NOTES "Step trail": 20 px).
const ITEM_HEIGHT_PX: f32 = 20.0;
/// The gap between an item's glyph, label and duration (mock `.trail .st { gap: 5px }`, between
/// the `xs` (4) and `sm` (6) tokens).
const ITEM_GAP_PX: f32 = 5.0;
/// The size of the run marker and the pending square (DESIGN.md: state marks are 7 px).
const MARK_PX: f32 = 7.0;

/// A step's state; a finished step may show its duration (formatted by Fluent).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TrailState {
    Pending,
    Running,
    Done { duration: Option<SharedString> },
    Failed,
}

/// One step of a trail.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrailItem {
    pub label: SharedString,
    pub state: TrailState,
}

/// The 7 px square `amber` run marker: "in progress" (DESIGN.md splash; NOTES cross-cutting #3,
/// the only place amber appears in M2a). Shared by the trail, the capsule and the status bar.
pub fn run_marker(theme: &Theme) -> Div {
    div()
        .size(px(MARK_PX))
        .flex_none()
        .bg(theme.tokens.colors.amber.to_hsla())
}

/// A row of connect steps (NOTES surface 1).
pub struct StepTrail {
    items: Vec<TrailItem>,
}

impl StepTrail {
    pub fn new(items: Vec<TrailItem>) -> Self {
        StepTrail { items }
    }
}

impl IntoElement for StepTrail {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |_, cx| {
            let theme = *Theme::get(cx);
            let t = theme.tokens;
            let c = &t.colors;
            let caption = t.type_ramp.caption;
            let label =
                |s: &SharedString, style: TypeStyle, color: hacivat_ui_core::gpui_surface::Hsla| {
                    Label::new(s.clone(), TextSource::Fluent, style, color)
                };
            let items = self.items.into_iter().enumerate().map(|(i, item)| {
                let row = div()
                    .debug_selector(move || format!("trail-{i}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .flex_none()
                    .gap(px(ITEM_GAP_PX))
                    .h(px(ITEM_HEIGHT_PX));
                match item.state {
                    TrailState::Done { duration } => row
                        .child(
                            Icon::new(IconName::Check)
                                .size(IconSize::Chrome)
                                .color(c.green.to_hsla()),
                        )
                        .child(label(&item.label, caption, c.text.to_hsla()))
                        .children(
                            duration.map(|d| label(&d, t.type_ramp.mono_micro, c.text_2.to_hsla())),
                        ),
                    TrailState::Running => row
                        .child(run_marker(&theme).debug_selector(move || format!("trail-run-{i}")))
                        .child(label(&item.label, caption, c.text.to_hsla())),
                    TrailState::Failed => row
                        .child(
                            Icon::new(IconName::X)
                                .size(IconSize::Chrome)
                                .color(c.text.to_hsla()),
                        )
                        .child(label(
                            &item.label,
                            TypeStyle {
                                weight: 600,
                                ..caption
                            },
                            c.text.to_hsla(),
                        )),
                    TrailState::Pending => row
                        .child(
                            div()
                                .debug_selector(move || format!("trail-pending-{i}"))
                                .size(px(MARK_PX))
                                .flex_none()
                                .border_1()
                                .border_color(c.seam_strong.to_hsla()),
                        )
                        .child(label(&item.label, caption, c.text_2.to_hsla())),
                }
            });
            div()
                .debug_selector(|| "step-trail".to_string())
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_x(px(t.spacing.s_2xl))
                .gap_y(px(t.spacing.s_2xs))
                .children(items)
                .into_any_element()
        })
        .into_any_element()
    }
}
