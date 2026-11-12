//! The command palette view: a query field over a result list on a floating surface.
//!
//! The palette only presents: the app ranks the actions for each query (`rank_commands`,
//! Task 16) and hands the rows over with [`Palette::set_items`]. Up and Down move the active
//! row, Enter confirms it and Escape dismisses the palette; while the IME is composing, those
//! keys belong to the IME.

use std::ops::Range;

use hacivat_ui_core::gpui_surface::{
    App, AppContext as _, Context, Entity, EventEmitter, FluentBuilder as _, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Pixels,
    Render, Role, ScrollEvent, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};
use hacivat_ui_core::keymap::{KeymapPlatform, display_chord, parse_chord};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::input::{InputEvent, TextInput};
use crate::l10n::ui_text;
use crate::text::{Label, TextFn, TextSource};

/// One result row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    /// The action ID, returned by [`PaletteEvent::Confirmed`].
    pub id: SharedString,
    /// The localized title.
    pub title: SharedString,
    /// The bound chord, already formatted for display.
    pub keys: Option<SharedString>,
    /// Byte ranges of `title` that matched the query (underlined in blue).
    pub highlight: Vec<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEvent {
    QueryChanged(String),
    /// The action ID of the confirmed row.
    Confirmed(SharedString),
    Dismissed,
}

/// Row height (DESIGN.md: completion items are 23 px).
pub const ROW_HEIGHT_PX: f32 = 23.0;
/// Footer height (DESIGN.md: the completion popup's key-hint footer is 24 px).
pub const FOOTER_HEIGHT_PX: f32 = 24.0;
/// Rows shown at once; the window follows the active row.
pub const VISIBLE_ROWS: usize = 12;

pub struct Palette {
    input: Entity<TextInput>,
    items: Vec<PaletteItem>,
    active: usize,
    /// Index of the first visible row.
    first: usize,
    query: String,
    /// Wheel travel not yet turned into whole rows.
    scroll_rest: Pixels,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    /// The palette's focus is its query field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Palette {
    /// An empty palette; `placeholder` is the query field's placeholder.
    pub fn new(cx: &mut Context<Self>, placeholder: TextFn) -> Self {
        let input = cx.new(|cx| TextInput::new(cx, placeholder));
        let subscription = cx.subscribe(&input, |this: &mut Self, input, event, cx| {
            match event {
                InputEvent::Changed => {
                    this.query = input.read(cx).value().to_string();
                    this.set_active(0);
                    cx.emit(PaletteEvent::QueryChanged(this.query.clone()));
                }
                InputEvent::Submitted => this.confirm(this.active, cx),
                InputEvent::Cancelled => cx.emit(PaletteEvent::Dismissed),
                InputEvent::CompositionChanged(_) => {}
            }
            cx.notify();
        });
        Self {
            input,
            items: Vec::new(),
            active: 0,
            first: 0,
            query: String::new(),
            scroll_rest: px(0.0),
            _subscription: subscription,
        }
    }

    /// Replaces the rows (ranked by the app) and makes the first one active.
    pub fn set_items(&mut self, items: Vec<PaletteItem>) {
        self.items = items;
        self.first = 0;
        self.set_active(0);
    }

    pub fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    /// The current query (the field's value, including text being composed).
    pub fn query(&self) -> String {
        self.query.clone()
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    fn set_active(&mut self, index: usize) {
        self.active = index.min(self.items.len().saturating_sub(1));
        if self.active < self.first {
            self.first = self.active;
        } else if self.active >= self.first + VISIBLE_ROWS {
            self.first = self.active + 1 - VISIBLE_ROWS;
        }
    }

    fn confirm(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get(index) {
            cx.emit(PaletteEvent::Confirmed(item.id.clone()));
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.input.read(cx).is_composing() {
            return;
        }
        let m = &event.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.function {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.set_active(self.active.saturating_sub(1)),
            "down" => self.set_active(self.active + 1),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let row = px(ROW_HEIGHT_PX);
        let travel = self.scroll_rest - event.delta.pixel_delta(row).y;
        let rows = (travel / row).trunc();
        self.scroll_rest = travel - row * rows;
        let max_first = self.items.len().saturating_sub(VISIBLE_ROWS) as f32;
        self.first = (self.first as f32 + rows).clamp(0.0, max_first) as usize;
        cx.notify();
    }
}

fn current_platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(windows) {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

/// A generated shortcut label for a chord such as `"up down"`.
fn key_label(chord: &str) -> SharedString {
    let presses = parse_chord(chord).unwrap_or_default();
    display_chord(&presses, current_platform()).into()
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = t.colors;
        let ty = t.type_ramp;

        let rows = self
            .items
            .iter()
            .enumerate()
            .skip(self.first)
            .take(VISIBLE_ROWS)
            .map(|(ix, item)| {
                let active = ix == self.active;
                div()
                    .id(("palette-row", ix))
                    .role(Role::ListBoxOption)
                    .aria_label(item.title.clone())
                    .aria_selected(active)
                    .flex()
                    .items_center()
                    .gap(px(t.spacing.md))
                    .h(px(ROW_HEIGHT_PX))
                    .px(px(t.spacing.lg))
                    .when(active, |el| el.bg(c.blue_tint_strong.to_hsla()))
                    .when(!active, |el| el.hover(|s| s.bg(c.hover.to_hsla())))
                    .on_click(cx.listener(move |this, _, _, cx| this.confirm(ix, cx)))
                    .child(
                        div().flex_1().min_w(px(0.0)).flex().child(
                            Label::new(
                                item.title.clone(),
                                TextSource::Fluent,
                                ty.body,
                                c.text.to_hsla(),
                            )
                            .truncate(true)
                            .highlight(item.highlight.clone(), c.blue.to_hsla()),
                        ),
                    )
                    .when_some(item.keys.clone(), |el, keys| {
                        el.child(Label::new(
                            keys,
                            TextSource::Shortcut,
                            ty.mono_kbd,
                            c.text_3.to_hsla(),
                        ))
                    })
            })
            .collect::<Vec<_>>();

        let list = div()
            .id("palette-list")
            .role(Role::ListBox)
            .flex()
            .flex_col()
            .py(px(t.spacing.xs))
            .on_scroll(cx.listener(Self::on_scroll))
            .children(rows)
            .when(self.items.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(ROW_HEIGHT_PX))
                        .px(px(t.spacing.lg))
                        .child(Label::new(
                            ui_text(cx, "hui-palette-no-results"),
                            TextSource::Fluent,
                            ty.body,
                            c.text_3.to_hsla(),
                        )),
                )
            });

        let hint = |keys: &str, id: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(t.spacing.xs))
                .child(Label::new(
                    key_label(keys),
                    TextSource::Shortcut,
                    ty.mono_kbd,
                    c.text_3.to_hsla(),
                ))
                .child(Label::new(
                    ui_text(cx, id),
                    TextSource::Fluent,
                    ty.caption,
                    c.text_3.to_hsla(),
                ))
        };
        let footer = div()
            .flex()
            .items_center()
            .gap(px(t.spacing.xl))
            .h(px(FOOTER_HEIGHT_PX))
            .px(px(t.spacing.lg))
            .border_t_1()
            .border_color(c.seam.to_hsla())
            .child(hint("up down", "hui-palette-hint-navigate"))
            .child(hint("enter", "hui-palette-hint-run"))
            .child(hint("escape", "hui-palette-hint-close"));

        popover_surface(&theme)
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .on_key_down(cx.listener(Self::on_key_down))
            .child(div().p(px(t.spacing.sm)).child(self.input.clone()))
            .child(
                div()
                    .border_t_1()
                    .border_color(c.seam.to_hsla())
                    .child(list),
            )
            .child(footer)
    }
}
