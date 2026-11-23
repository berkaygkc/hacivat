//! Inline notice (NOTES "NEW primitives"): padding 8/10, radius 4, 1 px `seam-strong`, a `panel`
//! / `ground` well; lines at 12.5 px `text`, secondary lines `text-2`, parameter rows with the
//! parameter in Fragment Mono 11.5 `text` and the rest in `text-2`. The 14 px icon is `text-2`
//! (info), or a `green` circle-check when the notice reports something done. Actions are
//! existing [`Button`]s. Announced as a status.

use std::panic::Location;
use std::rc::Rc;

use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Bounds, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Role,
    SharedString, Styled, Window, canvas, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, TypeStyle};

use crate::button::Button;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yNode, lazy};
use crate::text::{RenderedText, TextRegistry, TextSource};

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

// ## m4b-t22-pre (named item, T21 r1)
/// Called at prepaint with the bounds of a notice's title lines ([`InlineNotice::on_title_bounds`]).
pub type TitleBounds = Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>;

/// A notice inside a form (NOTES "NEW primitives").
pub struct InlineNotice {
    icon: NoticeIcon,
    lines: Vec<SharedString>,
    secondary: Vec<SharedString>,
    params: Vec<(SharedString, SharedString)>,
    actions: Vec<Button>,
    trailing: Option<Button>,
    // ## m4b-t22-pre (T21 r1)
    title_id: Option<&'static str>,
    on_title: Option<TitleBounds>,
    actions_first: bool,
    // ## m4b-t22-pre fix round 1 (T22-pre review I2)
    on_actions: Option<TitleBounds>,
}

impl InlineNotice {
    pub fn new(icon: NoticeIcon) -> Self {
        InlineNotice {
            icon,
            lines: Vec::new(),
            secondary: Vec::new(),
            params: Vec::new(),
            actions: Vec::new(),
            trailing: None,
            title_id: None,
            on_title: None,
            actions_first: false,
            on_actions: None,
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

    // ## m2a-t17-dialog
    /// A button at the card's right edge, top-aligned (the mock's Undo and close buttons).
    pub fn trailing(mut self, button: Button) -> Self {
        self.trailing = Some(button);
        self
    }

    // ## m4b-t22-pre (named items, T21 r1): a notice that may sit in a scroll area too short
    // for it (a script tab's notices while a Run-as-script strip shows).
    /// Records the title lines (the [`InlineNotice::line`]s) with `#id` appended to their path,
    /// and completes their records at prepaint with what the clip they are painted in shows
    /// (`visible`, `clipped`, [`crate::text::RenderedText::folded`]), so the frame checks can
    /// require the title whole.
    pub fn title_id(mut self, id: &'static str) -> Self {
        self.title_id = Some(id);
        self
    }

    /// Calls `f` at prepaint with the bounds the title lines were laid out in.
    pub fn on_title_bounds(mut self, f: TitleBounds) -> Self {
        self.on_title = Some(f);
        self
    }

    /// Draws the action row right under the title lines, above the secondary lines and
    /// parameters: in a notice cut short, its buttons show before its explanation.
    /// ## m4b-t22-pre fix round 1 (T22-pre review m1): only the drawing moves; the explanation
    /// still comes before the buttons in the element tree (reading order) and the Tab order.
    pub fn actions_first(mut self, first: bool) -> Self {
        self.actions_first = first;
        self
    }

    // ## m4b-t22-pre fix round 1 (named item; T22-pre review I2)
    /// Calls `f` at prepaint with the bounds of the action row (no row: never called).
    pub fn on_actions_bounds(mut self, f: TitleBounds) -> Self {
        self.on_actions = Some(f);
        self
    }
}

impl IntoElement for InlineNotice {
    type Element = AnyElement;

    fn into_element(mut self) -> AnyElement {
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
            // m2a-t17-dialog: prose wraps (a long translation never clips); each line is still
            // recorded in the text registry as a Fluent message.
            let line = |text: SharedString, ink, cx: &mut App| {
                text_run(text, TextSource::Fluent, t.type_ramp.body_sm, ink, cx)
            };
            let mut body = div().flex().flex_col().flex_1().min_w_0();
            // ## m4b-t22-pre (T21 r1): the title lines in a block of their own when they are
            // checked or measured; otherwise as before.
            if self.title_id.is_some() || self.on_title.is_some() {
                body = body.child(title_block(
                    self.lines,
                    self.title_id,
                    self.on_title,
                    t.type_ramp.body_sm,
                    c.text.to_hsla(),
                    cx,
                ));
            } else {
                for text in self.lines {
                    body = body.child(line(text, c.text.to_hsla(), cx));
                }
            }
            let on_actions = self.on_actions.take();
            let actions = (!self.actions.is_empty()).then(|| {
                let row = div()
                    .flex()
                    .flex_row()
                    // ## m4b-t21-shots (named exception): a row of buttons wider than the
                    // notice wraps (the encoding notice's four "Open as …" at 960 in the
                    // pseudo-locales); a row that fits lays out as before.
                    .flex_wrap()
                    .gap(px(t.spacing.sm))
                    .mt(px(t.spacing.md))
                    .children(self.actions);
                match on_actions {
                    // ## m4b-t22-pre fix round 1: the row reports where it was laid out.
                    Some(f) => row.relative().child(
                        canvas(
                            move |bounds, window, cx| f(bounds, window, cx),
                            |_, (), _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    ),
                    None => row,
                }
            });
            // The explanation: the secondary lines, then the parameters.
            let mut explanation: Vec<AnyElement> = Vec::new();
            for text in self.secondary {
                explanation.push(line(text, c.text_2.to_hsla(), cx));
            }
            if !self.params.is_empty() {
                let mut list = div().flex().flex_col().mt(px(t.spacing.xs));
                for (key, rest) in self.params {
                    list = list.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(t.spacing.sm))
                            .child(div().flex_none().child(text_run(
                                key,
                                TextSource::Data,
                                t.type_ramp.mono_data,
                                c.text.to_hsla(),
                                cx,
                            )))
                            .child(div().flex_1().min_w_0().child(line(
                                rest,
                                c.text_2.to_hsla(),
                                cx,
                            ))),
                    );
                }
                explanation.push(list.into_any_element());
            }
            match actions {
                // ## m4b-t22-pre fix round 1 (T22-pre review m1): drawn under the title, the
                // explanation below it, in a reversed column: the explanation stays first in
                // the element tree.
                Some(row) if self.actions_first => {
                    let mut stack = div().flex().flex_col_reverse();
                    if !explanation.is_empty() {
                        stack = stack.child(
                            div()
                                .flex()
                                .flex_col()
                                .mt(px(t.spacing.sm))
                                .children(explanation),
                        );
                    }
                    body = body.child(stack.child(row));
                }
                row => {
                    body = body.children(explanation).children(row);
                }
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
                .child(body)
                .children(self.trailing.map(|b| div().flex_none().child(b)));
            A11yNode::new("inline-notice", Role::Status, name, false, card).into_any_element()
        })
        .into_any_element()
    }
}

// ## m2a-t17-dialog
/// Notice text that wraps at the notice's width (prose) and is recorded in the text registry
/// when built, so every part of a notice is recorded in reading order (like `explain.rs`'s
/// paragraphs; `Label` is single-line and records at paint).
#[track_caller]
fn text_run(
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    cx: &mut App,
) -> AnyElement {
    text_run_at(text, source, style, color, None, cx).0
}

// ## m4b-t22-pre (T21 r1)
/// A notice's title lines in one block: each recorded as [`text_run`] records (with `#id`
/// appended to its path), then completed at prepaint from the clip it is painted in; `on_title`
/// gets the block's bounds.
#[track_caller]
fn title_block(
    lines: Vec<SharedString>,
    id: Option<&'static str>,
    on_title: Option<TitleBounds>,
    style: TypeStyle,
    color: Hsla,
    cx: &mut App,
) -> AnyElement {
    let mut block = div().relative().flex().flex_col().w_full();
    for text in lines {
        let (el, index) = text_run_at(text.clone(), TextSource::Fluent, style, color, id, cx);
        let el = match index {
            Some(index) => div()
                .relative()
                .w_full()
                .child(el)
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            let (visible, clipped, folded) =
                                crate::text::fit_in_clip(bounds, window, cx);
                            TextRegistry::update_at(cx, index, &text, |r| {
                                r.visible = visible;
                                r.clipped = clipped;
                                r.folded = folded;
                            });
                        },
                        |_, (), _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .into_any_element(),
            None => el,
        };
        block = block.child(el);
    }
    if let Some(f) = on_title {
        block = block.child(
            canvas(
                move |bounds, window, cx| f(bounds, window, cx),
                |_, (), _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    }
    block.into_any_element()
}

/// [`text_run`], with `#id` appended to the recorded path, and the record's index in the frame
/// when one was made with an id (the title lines complete it at prepaint).
#[track_caller]
fn text_run_at(
    text: SharedString,
    source: TextSource,
    style: TypeStyle,
    color: Hsla,
    id: Option<&'static str>,
    cx: &mut App,
) -> (AnyElement, Option<usize>) {
    let at = Location::caller();
    let make = || RenderedText {
        text: text.clone(),
        source,
        size_px: style.size_px,
        painted_px: 0.0,
        clipped: false,
        truncated: false,
        // ## m2b-t24-shots: recorded when built, before layout.
        visible: true,
        outside_card: false,
        folded: false,
        path: match id {
            Some(id) => format!(
                "{}:{}:{}#{id}",
                at.file().replace('\\', "/"),
                at.line(),
                at.column()
            ),
            None => format!(
                "{}:{}:{}",
                at.file().replace('\\', "/"),
                at.line(),
                at.column()
            ),
        }
        .into(),
    };
    let index = match id {
        Some(_) => TextRegistry::record_index(cx, make),
        None => {
            TextRegistry::record(cx, make);
            None
        }
    };
    let el = div()
        .w_full()
        .font(font_for(&style))
        .text_size(px(style.size_px))
        .line_height(px(style.line_height))
        .text_color(color)
        .child(text)
        .into_any_element();
    (el, index)
}
