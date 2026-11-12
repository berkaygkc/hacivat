//! Popup menus. Named `PopupMenu` so it never collides with GPUI's application `Menu` (F22).
//!
//! A [`PopupMenu`] is a view the host (Task 13's `Layers`) places over the window. It takes
//! focus, navigates with arrows, Home and End, chooses with Enter or a click, dismisses with
//! Escape or a click outside, and on Windows and Linux chooses by access key. Labels and keys
//! are [`TextFn`]/[`KeyFn`] closures re-read on every render (F16).

use hacivat_ui_core::gpui_surface::Entity;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Point, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::accesskey::{AccessKey, AccessKeyAssigner, same_key};
use crate::button::key_label;
use crate::popover::Declare;
use crate::text::{KeyFn, Label, TextFn, TextSource};

/// Minimum width of the menu surface.
pub const MENU_MIN_WIDTH_PX: f32 = 200.0;
/// Height of one row.
pub const MENU_ROW_PX: f32 = 24.0;

#[derive(Clone)]
pub struct MenuItemSpec {
    pub label: TextFn,
    /// The shortcut, generated from the keymap (`TextSource::Shortcut`).
    pub keys: Option<TextFn>,
    pub accesskey: Option<KeyFn>,
    pub disabled: bool,
    /// The machine action ID reported in [`PopupMenuEvent::Chosen`].
    pub action: SharedString,
    /// A separator line instead of an item (every other field is ignored).
    pub separator: bool,
}

impl MenuItemSpec {
    pub fn new(label: TextFn, action: impl Into<SharedString>) -> Self {
        Self {
            label,
            keys: None,
            accesskey: None,
            disabled: false,
            action: action.into(),
            separator: false,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: std::rc::Rc::new(|_| SharedString::default()),
            keys: None,
            accesskey: None,
            disabled: true,
            action: SharedString::default(),
            separator: true,
        }
    }

    pub fn keys(mut self, keys: TextFn) -> Self {
        self.keys = Some(keys);
        self
    }

    pub fn accesskey(mut self, key: KeyFn) -> Self {
        self.accesskey = Some(key);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn selectable(&self) -> bool {
        !self.separator && !self.disabled
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupMenuEvent {
    /// The user chose the item with this action ID.
    Chosen(SharedString),
    Dismissed,
}

pub struct PopupMenu {
    anchor: Point<Pixels>,
    items: Vec<MenuItemSpec>,
    selected: Option<usize>,
    focus: FocusHandle,
    label: Option<TextFn>,
    access_keys: bool,
}

impl EventEmitter<PopupMenuEvent> for PopupMenu {}

impl Focusable for PopupMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PopupMenu {
    /// Opens at `anchor` (the menu's top left, in window coordinates) and takes the focus.
    pub fn new(
        anchor: Point<Pixels>,
        items: Vec<MenuItemSpec>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<PopupMenu> {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            PopupMenu {
                anchor,
                selected: items.iter().position(MenuItemSpec::selectable),
                items,
                focus,
                label: None,
                access_keys: !cfg!(target_os = "macos"),
            }
        })
    }

    /// Names the menu for assistive technology.
    pub fn label(&mut self, label: TextFn) {
        self.label = Some(label);
    }

    /// Whether access keys are shown and accepted (default: Windows and Linux).
    pub fn set_access_keys(&mut self, on: bool) {
        self.access_keys = on;
    }

    /// The highlighted row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    fn step(&mut self, forward: bool) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let start = self.selected.unwrap_or(if forward { n - 1 } else { 0 });
        for k in 1..=n {
            let i = if forward {
                (start + k) % n
            } else {
                (start + n - k) % n
            };
            if self.items[i].selectable() {
                self.selected = Some(i);
                return;
            }
        }
    }

    fn edge(&mut self, last: bool) {
        let mut rows = (0..self.items.len()).filter(|&i| self.items[i].selectable());
        self.selected = if last { rows.next_back() } else { rows.next() };
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get(index).filter(|i| i.selectable()) {
            cx.emit(PopupMenuEvent::Chosen(item.action.clone()));
        }
    }

    /// The access key of each row: explicit keys from the catalog, then the first free letter.
    fn access_key_of_rows(&self, cx: &App) -> Vec<AccessKey> {
        let labels: Vec<(SharedString, Option<char>)> = self
            .items
            .iter()
            .map(|i| ((i.label)(cx), i.accesskey.as_ref().and_then(|k| k(cx))))
            .collect();
        let rows: Vec<(&str, Option<char>, Option<char>)> = labels
            .iter()
            .zip(&self.items)
            .map(|((label, key), item)| {
                if item.separator || item.disabled {
                    ("", None, None)
                } else {
                    (label.as_ref(), *key, None)
                }
            })
            .collect();
        AccessKeyAssigner::assign(&rows)
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let stroke = &event.keystroke;
        match stroke.key.as_str() {
            "down" => self.step(true),
            "up" => self.step(false),
            "home" => self.edge(false),
            "end" => self.edge(true),
            "enter" => {
                if let Some(i) = self.selected {
                    self.choose(i, cx);
                }
            }
            "escape" => cx.emit(PopupMenuEvent::Dismissed),
            _ if self.access_keys && !stroke.modifiers.control && !stroke.modifiers.platform => {
                let Some(typed) = stroke
                    .key
                    .chars()
                    .next()
                    .filter(|_| stroke.key.chars().count() == 1)
                else {
                    return;
                };
                let keys = self.access_key_of_rows(cx);
                let label_char = |i: usize, key: &AccessKey, cx: &App| match key {
                    AccessKey::InLabel(at) => (self.items[i].label)(cx)[*at..].chars().next(),
                    AccessKey::Suffix(c) => Some(*c),
                    AccessKey::None => None,
                };
                let hit = keys
                    .iter()
                    .enumerate()
                    .find(|(i, k)| label_char(*i, k, cx).is_some_and(|c| same_key(c, typed)))
                    .map(|(i, _)| i);
                if let Some(i) = hit {
                    self.choose(i, cx);
                }
            }
            _ => return,
        }
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for PopupMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let keys = if self.access_keys {
            self.access_key_of_rows(cx)
        } else {
            vec![AccessKey::None; self.items.len()]
        };

        // Keep the menu inside the window: flip to the other side of the anchor when it would
        // overflow.
        let viewport = window.viewport_size();
        let height = self.items.len() as f32 * MENU_ROW_PX + 8.0;
        let left = if self.anchor.x + px(MENU_MIN_WIDTH_PX) > viewport.width {
            (viewport.width - px(MENU_MIN_WIDTH_PX)).max(px(0.0))
        } else {
            self.anchor.x
        };
        let top = if self.anchor.y + px(height) > viewport.height {
            (self.anchor.y - px(height)).max(px(0.0))
        } else {
            self.anchor.y
        };

        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                if item.separator {
                    return div()
                        .h(px(1.0))
                        .my(px(t.spacing.xs))
                        .bg(c.seam.to_hsla())
                        .into_any_element();
                }
                let selected = self.selected == Some(i);
                let ink = if item.disabled { c.text_3 } else { c.text }.to_hsla();
                let text = (item.label)(cx);
                let mut row = div()
                    .id(("menu-item", i))
                    .debug_selector(move || format!("menu-item-{i}"))
                    .declare(Role::MenuItem, text.clone(), None)
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(t.spacing.xl))
                    .h(px(MENU_ROW_PX))
                    .px(px(t.spacing.lg))
                    .rounded(px(t.radii.xs))
                    .child(key_label(&text, Some(keys[i]), t.type_ramp.body, ink));
                if let Some(shortcut) = &item.keys {
                    row = row.child(Label::new(
                        shortcut(cx),
                        TextSource::Shortcut,
                        t.type_ramp.mono_kbd,
                        c.text_3.to_hsla(),
                    ));
                }
                if !item.disabled {
                    let action_index = i;
                    row = row
                        .cursor_pointer()
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.selected != Some(action_index) {
                                this.selected = Some(action_index);
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| this.choose(action_index, cx)));
                    if selected {
                        row = row.bg(c.hover.to_hsla());
                    }
                }
                row.into_any_element()
            })
            .collect();

        let menu_name = self.label.as_ref().map(|l| l(cx)).unwrap_or_default();
        let menu = popover_surface(&theme)
            .id("popup-menu")
            .debug_selector(|| "popup-menu".to_string())
            .track_focus(&self.focus)
            .declare(Role::Menu, menu_name.clone(), None)
            .absolute()
            .left(left)
            .top(top)
            .min_w(px(MENU_MIN_WIDTH_PX))
            .py(px(t.spacing.xs))
            .px(px(t.spacing.xs))
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PopupMenuEvent::Dismissed)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        menu.children(rows)
    }
}
