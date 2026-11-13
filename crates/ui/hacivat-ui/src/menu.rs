//! Popup menus. Named `PopupMenu` so it never collides with GPUI's application `Menu` (F22).
//!
//! A [`PopupMenu`] is a view the host (Task 13's `Layers`) places over the window. It takes
//! focus, navigates with arrows, Home and End, chooses with Enter or a click, dismisses with
//! Escape or a click outside, and on Windows and Linux chooses by access key. Labels and keys
//! are [`TextFn`]/[`KeyFn`] closures re-read on every render (F16).
//!
//! M2a: a menu can carry a check column (a `blue` check on the [`MenuItemSpec::checked`] row)
//! and per-row `text-2` hints, and a [`Select`](crate::select::Select) embeds one with
//! [`PopupMenu::embedded`] inside its own `deferred(anchored())` instead of a workspace layer
//! (P34), so both share this row anatomy.
//!
//! M2b (Task 24, T20 review M1b): a menu can show a title row (12 px `text-2`, padding 4/10,
//! not selectable; the mock's `.menu .mh`), and an item can draw its label in Fragment Mono
//! 11.5 (`mono`: host aliases, paths).

use hacivat_ui_core::gpui_surface::Entity;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels, Point, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, accesskit, div, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::accesskey::{AccessKey, AccessKeyAssigner, AccessKeyLocale, typed_matches};
use crate::button::key_label_from;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare};
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
    /// Shows the check in the menu's check column (a [`Select`](crate::select::Select)'s current
    /// option). A menu with any checked row draws the column on every row.
    pub checked: bool,
    /// A dimmer second text after the label (`text-2`).
    pub hint: Option<TextFn>,
    // ## m2a-t21-shots
    /// A destructive item (NOTES §8: "Delete… in red"): its label is drawn in `red` while it is
    /// enabled. Off by default.
    pub danger: bool,
    // ## m2b-t24-shots
    /// The label in Fragment Mono 11.5, as data (a host alias or a path). Off by default.
    pub mono: bool,
    /// Where the hint's text comes from: Fluent (default) or data (a resolved address).
    pub hint_source: TextSource,
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
            checked: false,
            hint: None,
            danger: false,
            mono: false,
            hint_source: TextSource::Fluent,
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
            checked: false,
            hint: None,
            danger: false,
            mono: false,
            hint_source: TextSource::Fluent,
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

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn hint(mut self, hint: TextFn) -> Self {
        self.hint = Some(hint);
        self
    }

    // ## m2b-t24-shots
    /// Draws the label in Fragment Mono 11.5, as data (`TextSource::Data`).
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Marks the hint's text as data (or Fluent, the default).
    pub fn hint_source(mut self, source: TextSource) -> Self {
        self.hint_source = source;
        self
    }

    // ## m2a-t21-shots
    /// Marks a destructive item (its label is `red` while enabled).
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// The label's ink: `text-3` when disabled, `red` for an enabled destructive item, else
    /// `text`.
    pub fn label_ink(
        &self,
        c: &hacivat_ui_core::theme::Colors,
    ) -> hacivat_ui_core::gpui_surface::Hsla {
        if self.disabled {
            c.text_3
        } else if self.danger {
            c.red
        } else {
            c.text
        }
        .to_hsla()
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
    /// The top left in window coordinates; `None` for an embedded menu, which its host places
    /// (`deferred(anchored())`).
    anchor: Option<Point<Pixels>>,
    min_width: Pixels,
    items: Vec<MenuItemSpec>,
    selected: Option<usize>,
    focus: FocusHandle,
    label: Option<TextFn>,
    // ## m2b-t24-shots
    /// The visible title row, when set.
    title: Option<TextFn>,
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
        Self::build(Some(anchor), items, window, cx)
    }

    /// A menu its host positions (a `Select`'s `deferred(anchored())`, P34); takes the focus.
    pub fn embedded(
        items: Vec<MenuItemSpec>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<PopupMenu> {
        Self::build(None, items, window, cx)
    }

    fn build(
        anchor: Option<Point<Pixels>>,
        items: Vec<MenuItemSpec>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<PopupMenu> {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            // The highlight starts on the checked row (a Select's current option).
            let selected = items
                .iter()
                .position(|i| i.checked && i.selectable())
                .or_else(|| items.iter().position(MenuItemSpec::selectable));
            PopupMenu {
                anchor,
                min_width: px(MENU_MIN_WIDTH_PX),
                selected,
                items,
                focus,
                label: None,
                title: None,
                access_keys: !cfg!(target_os = "macos"),
            }
        })
    }

    /// Widens the menu to at least `width` (never under [`MENU_MIN_WIDTH_PX`]).
    pub fn set_min_width(&mut self, width: Pixels) {
        self.min_width = width.max(px(MENU_MIN_WIDTH_PX));
    }

    /// Names the menu for assistive technology.
    pub fn label(&mut self, label: TextFn) {
        self.label = Some(label);
    }

    // ## m2b-t24-shots
    /// Shows `title` as the menu's first row (12 px `text-2`, not selectable). It also names
    /// the menu for assistive technology unless [`PopupMenu::label`] does.
    pub fn title(&mut self, title: TextFn) {
        self.title = Some(title);
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
                let turkic = AccessKeyLocale::turkic(cx);
                let keys = self.access_key_of_rows(cx);
                let label_char = |i: usize, key: &AccessKey, cx: &App| match key {
                    AccessKey::InLabel(at) => (self.items[i].label)(cx)[*at..].chars().next(),
                    AccessKey::Suffix(c) => Some(*c),
                    AccessKey::None => None,
                };
                let hit = keys
                    .iter()
                    .enumerate()
                    .find(|(i, k)| {
                        label_char(*i, k, cx).is_some_and(|c| typed_matches(c, typed, turkic))
                    })
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
        // overflow. An embedded menu is placed by its host's `anchored()`.
        let position = self.anchor.map(|anchor| {
            let viewport = window.viewport_size();
            let height: f32 = self
                .items
                .iter()
                .map(|item| {
                    if item.separator {
                        1.0 + 2.0 * t.spacing.xs
                    } else {
                        MENU_ROW_PX
                    }
                })
                .sum::<f32>()
                + 2.0 * t.spacing.xs
                + 2.0
                // ## m2b-t24-shots: the title row.
                + if self.title.is_some() {
                    t.type_ramp.caption.line_height + 2.0 * t.spacing.xs
                } else {
                    0.0
                };
            let left = if anchor.x + self.min_width > viewport.width {
                (viewport.width - self.min_width).max(px(0.0))
            } else {
                anchor.x
            };
            let top = if anchor.y + px(height) > viewport.height {
                (anchor.y - px(height)).max(px(0.0))
            } else {
                anchor.y
            };
            (left, top)
        });
        let check_column = self.items.iter().any(|i| i.checked && !i.separator);

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
                let ink = item.label_ink(c);
                let text = (item.label)(cx);
                // The check column (12 px, `blue` check) sits before the label on every row of a
                // menu that has one.
                let mut lead = div().flex().flex_row().items_center().gap(px(t.spacing.xl));
                if check_column {
                    let mut column = div().flex().flex_none().w(px(IconSize::Chrome.px()));
                    if item.checked {
                        column = column.child(
                            div()
                                .debug_selector(move || format!("menu-item-check-{i}"))
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::Chrome)
                                        .color(c.blue.to_hsla()),
                                ),
                        );
                    }
                    lead = lead.child(column);
                }
                // ## m2b-t24-shots: a mono label is data (a host alias, a path).
                let (style, source) = if item.mono {
                    (t.type_ramp.mono_data, TextSource::Data)
                } else {
                    (t.type_ramp.body, TextSource::Fluent)
                };
                lead = lead.child(key_label_from(&text, source, Some(keys[i]), style, ink));
                let mut row = div()
                    .id(("menu-item", i))
                    .debug_selector(move || format!("menu-item-{i}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(t.spacing.xl))
                    .h(px(MENU_ROW_PX))
                    .px(px(t.spacing.lg))
                    .rounded(px(t.radii.xs))
                    .child(lead);
                if item.hint.is_some() || item.keys.is_some() {
                    // NOTES: the hint and the keys sit right, 16 px from the label at least.
                    let mut trail = div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(t.spacing.xl))
                        .pl(px(t.spacing.xs));
                    if let Some(hint) = &item.hint {
                        // Raised Ink Rule: the hint on the raised menu uses text-2.
                        trail = trail.child(Label::new(
                            hint(cx),
                            item.hint_source,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        ));
                    }
                    if let Some(shortcut) = &item.keys {
                        trail = trail.child(Label::new(
                            shortcut(cx),
                            TextSource::Shortcut,
                            t.type_ramp.mono_kbd,
                            // Raised Ink Rule: key hints on the raised menu use text-2.
                            c.text_2.to_hsla(),
                        ));
                    }
                    row = row.child(trail);
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
                if item.disabled {
                    return A11yNode::new(("menu-item-node", i), Role::MenuItem, text, false, row)
                        .disabled(true)
                        .into_any_element();
                }
                let role = if check_column {
                    Role::MenuItemRadio
                } else {
                    Role::MenuItem
                };
                let mut decl = A11yDecl::new(role, text);
                if selected {
                    decl = decl.active_descendant();
                }
                let row = row.declare_full(decl);
                if check_column {
                    row.aria_toggled(accesskit::Toggled::from(item.checked))
                        .into_any_element()
                } else {
                    row.into_any_element()
                }
            })
            .collect();

        let menu_name = self
            .label
            .as_ref()
            .or(self.title.as_ref())
            .map(|l| l(cx))
            .unwrap_or_default();
        let mut menu = popover_surface(&theme)
            .id("popup-menu")
            .debug_selector(|| "popup-menu".to_string())
            .track_focus(&self.focus)
            .declare(Role::Menu, menu_name.clone(), None);
        if let Some((left, top)) = position {
            menu = menu.absolute().left(left).top(top);
        }
        let menu = menu
            .min_w(self.min_width)
            .py(px(t.spacing.xs))
            .px(px(t.spacing.xs))
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PopupMenuEvent::Dismissed)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        // ## m2b-t24-shots: the title row (the mock's `.menu .mh`: padding 4/10, 12 px text-2;
        // Raised Ink Rule).
        let title = self.title.as_ref().map(|title| {
            div()
                .debug_selector(|| "popup-menu-title".to_string())
                .py(px(t.spacing.xs))
                .px(px(t.spacing.lg))
                .child(
                    Label::new(
                        title(cx),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    )
                    .id("popup-menu-title"),
                )
        });
        menu.children(title).children(rows)
    }
}

// ## m2a-t21-shots
#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use hacivat_ui_core::gpui_surface::{App, SharedString};
    use hacivat_ui_core::theme::{DARK, LIGHT};

    use super::MenuItemSpec;

    fn item() -> MenuItemSpec {
        MenuItemSpec::new(Rc::new(|_: &App| SharedString::from("Delete…")), "x.delete")
    }

    #[test]
    fn a_danger_item_is_red_until_it_is_disabled() {
        for c in [&DARK.colors, &LIGHT.colors] {
            assert!(!item().danger, "off by default");
            assert_eq!(item().label_ink(c), c.text.to_hsla());
            assert_eq!(item().danger(true).label_ink(c), c.red.to_hsla());
            assert_eq!(
                item().danger(true).disabled(true).label_ink(c),
                c.text_3.to_hsla()
            );
        }
    }
}
