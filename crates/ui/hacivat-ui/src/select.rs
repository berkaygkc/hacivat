//! Select (NOTES "NEW primitives"): a field-shaped button (26 px `field-height`, radius 4,
//! `seam-strong` hairline, padding 0 6 0 9, `panel`/`ground` well, the value at 13 px `text`,
//! chevrons-up-down at 12 px `text-2`) that opens the M1 [`PopupMenu`] anatomy.
//!
//! The menu is an embedded `PopupMenu` entity drawn in the select's own `deferred(anchored())`,
//! not a workspace layer (P34): it shares the 24 px rows, the 200 px minimum, the `hover` row and
//! the check column, and `deferred` paints without the content mask of a scrolling dialog body.
//! Each item's action is its index; the select listens for [`PopupMenuEvent::Chosen`].
//!
//! Keys: Space, ↓ and Enter open it; in the open menu ↑/↓ move, Enter picks and Escape closes.
//! While open the menu holds the focus and stops Escape, ↑, ↓ and Enter, so a dialog layer under
//! it stays open (the layer must handle Escape in the bubble phase or as an action, as the
//! workspace does). Closing returns the focus to the field.

use hacivat_ui_core::gpui_surface::{
    Anchor, App, Bounds, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Render,
    Role, StatefulInteractiveElement, Styled, Subscription, Window, anchored, canvas, deferred,
    div, point, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline};

use crate::icon::{Icon, IconName, IconSize};
use crate::input::{FIELD_PADDING_LEFT_PX, FIELD_PADDING_RIGHT_PX};
use crate::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use crate::popover::A11yNode;
use crate::text::{Label, TextFn, TextSource};

/// The gap between the field and its menu.
const MENU_GAP_PX: f32 = 4.0;

/// One choice of a [`Select`] (NOTES "NEW primitives").
#[derive(Clone)]
pub struct SelectOption {
    pub label: TextFn,
    /// A second, dimmer text on the option's menu row (`stored in the macOS Keychain`).
    pub hint: Option<TextFn>,
    pub disabled: bool,
}

/// What a [`Select`] emits (NOTES "NEW primitives").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectEvent {
    /// The user picked another option.
    Changed(usize),
}

/// A drop-down choice (NOTES "NEW primitives").
pub struct Select {
    options: Vec<SelectOption>,
    selected: usize,
    focus: FocusHandle,
    /// The accessible name (the form row's label); the value is the selected option.
    label: Option<TextFn>,
    menu: Option<Entity<PopupMenu>>,
    menu_subscriptions: Vec<Subscription>,
    /// The field as last painted: the menu's anchor and minimum width.
    field_bounds: Option<Bounds<Pixels>>,
    /// A press on the field dismissed the menu: the click that press finishes must not reopen
    /// it. `Dismissed` arrives when the press's effects flush, after the field's own mouse-down
    /// handler, which clears the flag first; so a press released off the field (no click) never
    /// leaves it set for the next one.
    swallow_click: bool,
}

impl EventEmitter<SelectEvent> for Select {}

impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Select {
    pub fn new(options: Vec<SelectOption>, selected: usize, cx: &mut Context<Self>) -> Self {
        Select {
            options,
            selected,
            focus: cx.focus_handle().tab_stop(true),
            label: None,
            menu: None,
            menu_subscriptions: Vec::new(),
            field_bounds: None,
            swallow_click: false,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Selects `index` without emitting.
    pub fn set_selected(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        cx.notify();
    }

    /// Replaces the options (an open menu keeps the old ones until it closes).
    pub fn set_options(&mut self, options: Vec<SelectOption>, cx: &mut Context<Self>) {
        self.options = options;
        cx.notify();
    }

    /// Names the select for assistive technology (the label of its form row).
    pub fn set_label(&mut self, label: TextFn) {
        self.label = Some(label);
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_some()
    }

    // ## m2a-t17-dialog
    /// The choices as given (their labels, hints and disabled flags).
    pub fn options(&self) -> &[SelectOption] {
        &self.options
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() || self.options.is_empty() {
            return;
        }
        let items: Vec<MenuItemSpec> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, option)| {
                let mut item = MenuItemSpec::new(option.label.clone(), i.to_string())
                    .checked(i == self.selected)
                    .disabled(option.disabled);
                if let Some(hint) = &option.hint {
                    item = item.hint(hint.clone());
                }
                item
            })
            .collect();
        let menu = PopupMenu::embedded(items, window, cx);
        let width = self.field_bounds.map(|b| b.size.width).unwrap_or_default();
        let label = self.label.clone();
        menu.update(cx, |menu, _| {
            menu.set_min_width(width);
            if let Some(label) = label {
                menu.label(label);
            }
        });
        let events = cx.subscribe_in(&menu, window, Self::on_menu_event);
        // Focus leaving the menu (Tab, a press elsewhere) closes it.
        let menu_focus = menu.focus_handle(cx);
        let blur = cx.on_focus_out(&menu_focus, window, |this, _, window, cx| {
            this.close(false, window, cx)
        });
        self.menu = Some(menu);
        self.menu_subscriptions = vec![events, blur];
        self.swallow_click = false;
        cx.notify();
    }

    /// Closes the menu; `restore` returns the focus to the field.
    fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        // Dropped once this update is done: the event in flight still reaches every subscriber
        // (the `open_dialog` pattern).
        let subscriptions = std::mem::take(&mut self.menu_subscriptions);
        cx.defer(move |_| drop((menu, subscriptions)));
        if restore {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn on_menu_event(
        &mut self,
        _: &Entity<PopupMenu>,
        event: &PopupMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PopupMenuEvent::Chosen(action) => {
                let picked = action
                    .parse::<usize>()
                    .ok()
                    .filter(|&i| i < self.options.len());
                self.close(true, window, cx);
                if let Some(index) = picked
                    && index != self.selected
                {
                    self.selected = index;
                    cx.emit(SelectEvent::Changed(index));
                }
            }
            PopupMenuEvent::Dismissed => {
                // A press on the field itself dismissed the menu (it is outside the menu); the
                // click it finishes must not open it again.
                let on_field = !window.last_input_was_keyboard()
                    && self
                        .field_bounds
                        .is_some_and(|b| b.contains(&window.mouse_position()));
                self.close(true, window, cx);
                self.swallow_click = on_field;
            }
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        if stroke.modifiers.modified() {
            return;
        }
        match (self.menu.is_some(), stroke.key.as_str()) {
            (false, "space" | "down" | "enter") => self.open(window, cx),
            (true, "escape") => self.close(true, window, cx),
            (true, "up" | "down" | "enter") => {}
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn on_field_click(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The keyboard opens the menu on key down; the key-up click is not a second toggle.
        if matches!(event, ClickEvent::Keyboard(_)) {
            return;
        }
        if std::mem::take(&mut self.swallow_click) {
            return;
        }
        if self.menu.is_some() {
            self.close(true, window, cx);
        } else {
            window.focus(&self.focus, cx);
            self.open(window, cx);
        }
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        // A field well: its own fill (NOTES cross-cutting #4, #5).
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let value = self
            .options
            .get(self.selected)
            .map(|o| (o.label)(cx))
            .unwrap_or_default();
        let name = self
            .label
            .as_ref()
            .map(|l| l(cx))
            .unwrap_or_else(|| value.clone());
        let focused = self.focus.is_focused(window) && window.last_input_was_keyboard();
        let me = cx.entity();

        let field = div()
            .id("select-field")
            .debug_selector(|| "select-field".to_string())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.swallow_click = false),
            )
            .on_click(cx.listener(Self::on_field_click))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(t.spacing.md))
            .w_full()
            .min_w_0()
            .h(px(t.spacing.field_height))
            .pl(px(FIELD_PADDING_LEFT_PX))
            .pr(px(FIELD_PADDING_RIGHT_PX))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .bg(well.to_hsla())
            .hover(|style| style.border_color(c.text_3.to_hsla()))
            .cursor_pointer()
            .child(
                div().flex_1().min_w_0().child(
                    Label::new(
                        value.clone(),
                        TextSource::Fluent,
                        t.type_ramp.body,
                        c.text.to_hsla(),
                    )
                    .truncate(true),
                ),
            )
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            );
        let field = focus_outline(field, &theme, focused);

        // The root is exactly the field's box: its bounds anchor and size the menu.
        let mut root = div()
            .relative()
            .w_full()
            .child(
                A11yNode::new("select-node", Role::ComboBox, name, false, field)
                    .value(value)
                    .expanded(self.menu.is_some()),
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        me.update(cx, |select, _| select.field_bounds = Some(bounds));
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        if let Some(menu) = &self.menu {
            let place = match self.field_bounds {
                Some(bounds) => anchored()
                    .attach(bounds, Anchor::BottomLeft)
                    .offset(point(px(0.0), px(MENU_GAP_PX))),
                None => anchored(),
            };
            root = root.child(deferred(place.child(menu.clone())).with_priority(1));
        }
        root
    }
}
