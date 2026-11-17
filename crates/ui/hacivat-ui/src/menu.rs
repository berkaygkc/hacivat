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
    AnyElement, App, AppContext, Bounds, Context, ElementId, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, Pixels,
    Point, Render, Role, ScrollEvent, SharedString, StatefulInteractiveElement, Styled, Window,
    accesskit, anchored, div, point, px,
};
use hacivat_ui_core::theme::{Theme, popover_surface};

use crate::accesskey::{AccessKey, AccessKeyAssigner, AccessKeyLocale, typed_matches};
use crate::button::key_label_from;
use crate::icon::{Icon, IconName, IconSize};
use crate::popover::{A11yDecl, A11yNode, Declare};
use crate::text::{KeyFn, Label, TextFn, TextRegistry, TextSource};

/// Minimum width of the menu surface.
pub const MENU_MIN_WIDTH_PX: f32 = 200.0;
/// Height of one row.
pub const MENU_ROW_PX: f32 = 24.0;
// ## m3a-f2-visual (named exception)
/// The key-hint footer: 24 px, plus the 4 px it keeps from the last row.
pub const MENU_FOOTER_PX: f32 = 28.0;

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
    // ## m3b-t26-prod-unlock (named exception: the T22 review m12 carry)
    /// An informational note (the mock's `.mnote`: 12 px `text-2`), never selectable: not a
    /// disabled command, so the Raised Ink Rule's `text-2` applies, never disabled `text-3`.
    pub note: bool,
    // ## m3b-t29-shots (named exception; T29 review I1)
    /// An inert sentence that wraps inside the menu's width (the chooser's explanations,
    /// NOTES M3b ruling 19: "on its own wrapping line"): the row grows to its lines.
    pub wraps: bool,
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
            // ## m3b-t26-prod-unlock
            note: false,
            // ## m3b-t29-shots
            wraps: false,
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
            // ## m3b-t26-prod-unlock
            note: false,
            // ## m3b-t29-shots
            wraps: false,
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

    // ## m3b-t26-prod-unlock (named exception: the T22 review m12 carry)
    /// Makes the row an informational note: 12 px `text-2` (the Raised Ink Rule), never
    /// selectable, never chosen.
    // ## m3b-t29-shots (named exception; T29 review I1)
    /// A sentence that wraps inside the menu (never actionable, as a note).
    pub fn wraps(mut self, wraps: bool) -> Self {
        self.wraps = wraps;
        self.disabled |= wraps;
        self
    }

    pub fn note(mut self, note: bool) -> Self {
        self.note = note;
        // Never actionable: every path that skips a disabled row skips a note.
        self.disabled |= note;
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
        // ## m3b-t26-prod-unlock (named exception): a note is text, not a disabled command.
        if self.note {
            c.text_2
        } else if self.disabled {
            c.text_3
        } else if self.danger {
            c.red
        } else {
            c.text
        }
        .to_hsla()
    }

    fn selectable(&self) -> bool {
        // ## m3b-t26-prod-unlock (named exception): a note is never selectable.
        !self.separator && !self.disabled && !self.note
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
    // ## m3a-t35-shots (named exception): at most this wide (the chooser's 460 px, NOTES
    // surface 4); a row's data hint ellipsizes to fit.
    max_width: Option<Pixels>,
    items: Vec<MenuItemSpec>,
    selected: Option<usize>,
    focus: FocusHandle,
    label: Option<TextFn>,
    // ## m2b-t24-shots
    /// The visible title row, when set.
    title: Option<TextFn>,
    access_keys: bool,
    // ## m3a-f2-visual (named exception): the statement chooser's anatomy (NOTES surface 4).
    /// The highlighted row takes `blue-tint-strong` and its hint goes to `text`.
    active_tint: bool,
    /// A 24 px key-hint footer on `head`: `(chord, Fluent ID)` pairs.
    key_hints: Vec<(&'static str, &'static str)>,
    // ## m3a-f5-demo (follow-up; review m-2): a menu taller than the room the window gives it
    // shows a window of whole rows (separators included) above the status bar, from item
    // `first`; the keys and the wheel move it a row at a time.
    first: usize,
    /// Wheel travel not yet turned into rows.
    wheel: f32,
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
                max_width: None,
                selected,
                items,
                focus,
                label: None,
                title: None,
                access_keys: !cfg!(target_os = "macos"),
                active_tint: false,
                key_hints: Vec::new(),
                first: 0,
                wheel: 0.0,
            }
        })
    }

    /// Widens the menu to at least `width` (never under [`MENU_MIN_WIDTH_PX`]).
    pub fn set_min_width(&mut self, width: Pixels) {
        self.min_width = width.max(px(MENU_MIN_WIDTH_PX));
    }

    // ## m3a-t35-shots (named exception)
    /// Keeps the menu at most `width` wide (never under its minimum width, whichever is set
    /// last: the two are resolved when the menu draws); a row's hint ellipsizes to fit, its
    /// label never. Fix round 1 (M7).
    pub fn set_max_width(&mut self, width: Pixels) {
        self.max_width = Some(width);
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

    // ## m3a-f2-visual (named exception)
    /// The highlighted row reads as a selection, not a hover: `blue-tint-strong`, its hint in
    /// `text` (the chooser: Enter runs that row; NOTES ruling 18).
    pub fn set_active_tint(&mut self, on: bool) {
        self.active_tint = on;
    }

    // ## m3a-f2-visual (named exception)
    /// A 24 px footer on `head` listing the menu's keys: each a chord (drawn for the platform,
    /// as the palette's footer) and its Fluent ID, such as `("enter", "hui-palette-hint-run")`.
    pub fn set_key_hints(&mut self, hints: Vec<(&'static str, &'static str)>) {
        self.key_hints = hints;
    }

    /// Whether access keys are shown and accepted (default: Windows and Linux).
    pub fn set_access_keys(&mut self, on: bool) {
        self.access_keys = on;
    }

    /// The highlighted row.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    // ## m3a-t33-exec-ui (named exception): the statement chooser opens on Run's preselected
    // candidate, which is not always the checked (remembered) row.
    /// Highlights row `index` when it can be chosen (else nothing changes).
    pub fn select(&mut self, index: usize) {
        if self.items.get(index).is_some_and(MenuItemSpec::selectable) {
            self.selected = Some(index);
        }
    }

    // ## m3b-t24-exec-ui (named exception): the chooser's "Run as one" row is never
    // preselected (R-M3-60, NOTES surface 9): Enter does nothing until ↓ highlights it.
    /// Highlights no row: Enter chooses nothing until an arrow key or the pointer highlights
    /// one.
    pub fn clear_selection(&mut self) {
        self.selected = None;
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
        // (A scrolling menu brings the highlighted row into its window when it draws.)
        cx.notify();
        cx.stop_propagation();
    }

    // ## m3a-f5-demo (follow-up)
    /// The height of `item` in the rows column: a row, or a separator with its margins.
    fn entry_height(item: &MenuItemSpec, xs: f32) -> f32 {
        if item.separator {
            1.0 + 2.0 * xs
        } else if item.wraps {
            // ## m3b-t29-shots: a wrapping sentence is budgeted two lines (its height is known
            // only once laid out; check (m) sees a menu that runs past its room).
            2.0 * MENU_ROW_PX
        } else {
            MENU_ROW_PX
        }
    }

    // ## m3a-f5-demo (review m-2)
    /// The end of the window of whole entries from `first` that fits in `room`.
    fn window_end(entries: &[f32], first: usize, room: f32) -> usize {
        let mut used = 0.0;
        let mut end = first;
        while end < entries.len() && used + entries[end] <= room {
            used += entries[end];
            end += 1;
        }
        end.max((first + 1).min(entries.len()))
    }

    /// The window of a scrolling menu: from `self.first`, clamped so the last item can be the
    /// last shown and moved so the highlighted row is in it.
    fn rows_window(&mut self, entries: &[f32], room: f32) -> std::ops::Range<usize> {
        let n = entries.len();
        // The last first item whose window reaches the end.
        let mut last_first = n.saturating_sub(1);
        while last_first > 0 && Self::window_end(entries, last_first - 1, room) == n {
            last_first -= 1;
        }
        let mut first = self.first.min(last_first);
        if let Some(sel) = self.selected {
            if sel < first {
                first = sel;
            }
            while Self::window_end(entries, first, room) <= sel && first < sel {
                first += 1;
            }
        }
        self.first = first;
        first..Self::window_end(entries, first, room)
    }

    /// The wheel over a scrolling menu: a row per row's height of travel.
    fn on_wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        self.wheel += f32::from(event.delta.pixel_delta(px(MENU_ROW_PX)).y);
        let mut moved = false;
        while self.wheel <= -MENU_ROW_PX {
            self.wheel += MENU_ROW_PX;
            self.first += 1;
            moved = true;
        }
        while self.wheel >= MENU_ROW_PX {
            self.wheel -= MENU_ROW_PX;
            self.first = self.first.saturating_sub(1);
            moved = true;
        }
        if moved {
            // The wheel moves the window, not the highlight: a highlight left outside it is
            // dropped rather than pulling the window back.
            self.selected = None;
            cx.notify();
        }
        cx.stop_propagation();
    }
}

// ## m3a-f5-demo (follow-up): where the last anchored menu drew, for the frame checks.

/// What the last anchored menu drew, recorded while the [`TextRegistry`] is enabled (tests
/// and the shots): its rows' column, each drawn row's item index and bounds (a scrolling menu
/// draws only its window), the window's height and the status bar's.
#[derive(Debug, Clone, Default)]
pub struct MenuLayout {
    pub viewport: Option<Bounds<Pixels>>,
    pub rows: Vec<(usize, Bounds<Pixels>)>,
    pub window_height: f32,
    pub statusbar_height: f32,
    // ## m3b-ci (controller fix)
    /// The window's width: a row must end inside it.
    pub window_width: f32,
}

impl Global for MenuLayout {}

impl MenuLayout {
    /// What is wrong with the last anchored menu: a row shown only in part (cut at its
    /// column's edge), or one drawn over the status bar or past the window's bottom, or
    /// (## m3b-ci) past the window's left or right side.
    pub fn violations(cx: &App) -> Vec<String> {
        let Some(layout) = cx.try_global::<MenuLayout>() else {
            return Vec::new();
        };
        let slack = 0.5;
        let floor = layout.window_height - layout.statusbar_height;
        let mut out = Vec::new();
        for (i, row) in &layout.rows {
            let (top, bottom) = (f32::from(row.top()), f32::from(row.bottom()));
            if let Some(v) = layout.viewport {
                let (vt, vb) = (f32::from(v.top()), f32::from(v.bottom()));
                if top < vt - slack || bottom > vb + slack {
                    out.push(format!(
                        "menu row {i} is cut at its column ({top}..{bottom})"
                    ));
                }
            }
            if bottom > floor + slack {
                out.push(format!(
                    "menu row {i} reaches the status bar or the window's bottom \
                     ({bottom} > {floor})"
                ));
            }
            // ## m3b-ci (controller fix): nothing clips a row at the window's sides but the
            // window itself, so its hint would be cut there (CI run 44051024805, Linux).
            let (left, right) = (f32::from(row.left()), f32::from(row.right()));
            if layout.window_width > 0.0 && (left < -slack || right > layout.window_width + slack) {
                out.push(format!(
                    "menu row {i} runs past the window's side ({left}..{right} in {})",
                    layout.window_width
                ));
            }
        }
        out
    }
}

/// Records the rows of the anchored menu just prepainted (`viewport`: its rows' column).
fn note_menu_rows(
    cx: &mut App,
    window: &Window,
    statusbar: f32,
    viewport: Option<Bounds<Pixels>>,
    rows: Vec<(usize, Bounds<Pixels>)>,
) {
    if !cx.try_global::<TextRegistry>().is_some_and(|r| r.enabled) {
        return;
    }
    cx.set_global(MenuLayout {
        viewport,
        rows,
        window_height: f32::from(window.viewport_size().height),
        statusbar_height: statusbar,
        window_width: f32::from(window.viewport_size().width),
    });
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
        // ## m3a-f5-demo (follow-up): the room ends above the status bar. A menu that fits on
        // neither side of its anchor but fits the room moves up until it does (review m-3); one
        // taller than the room stays under its anchor and shows a window of whole rows.
        let entries: Vec<f32> = self
            .items
            .iter()
            .map(|item| Self::entry_height(item, t.spacing.xs))
            .collect();
        let rows_height: f32 = entries.iter().sum();
        let mut room: Option<f32> = None;
        let position = self.anchor.map(|anchor| {
            let viewport = window.viewport_size();
            let chrome = 2.0 * t.spacing.xs
                + 2.0
                // ## m2b-t24-shots: the title row.
                + if self.title.is_some() {
                    t.type_ramp.caption.line_height + 2.0 * t.spacing.xs
                } else {
                    0.0
                }
                // ## m3a-f2-visual: the key-hint footer.
                + if self.key_hints.is_empty() {
                    0.0
                } else {
                    MENU_FOOTER_PX
                };
            let height = chrome + rows_height;
            let floor = f32::from(viewport.height) - t.spacing.statusbar_height - t.spacing.xs;
            let left = if anchor.x + self.min_width > viewport.width {
                (viewport.width - self.min_width).max(px(0.0))
            } else {
                anchor.x
            };
            let below = f32::from(anchor.y) + height <= floor;
            let above = f32::from(anchor.y) - height >= 0.0;
            let top = if below {
                anchor.y
            } else if above {
                anchor.y - px(height)
            } else if height <= floor {
                px((floor - height).max(0.0))
            } else {
                room = Some((floor - f32::from(anchor.y) - chrome).max(MENU_ROW_PX));
                anchor.y
            };
            (left, top)
        });
        let shown = match room {
            Some(room) => self.rows_window(&entries, room),
            None => 0..self.items.len(),
        };
        let check_column = self.items.iter().any(|i| i.checked && !i.separator);

        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| shown.contains(i))
            .map(|(i, item)| {
                if item.separator {
                    return div()
                        .h(px(1.0))
                        .flex_none()
                        .my(px(t.spacing.xs))
                        .bg(c.seam.to_hsla())
                        .into_any_element();
                }
                let selected = self.selected == Some(i);
                let ink = item.label_ink(c);
                let text = (item.label)(cx);
                // ## m3b-t29-shots (named exception; T29 review I1): a wrapping sentence takes the
                // menu's width and as many lines as it needs, inside the card.
                if item.wraps {
                    let row = div()
                        .id(("menu-item", i))
                        .debug_selector(move || format!("menu-item-{i}"))
                        .w_full()
                        .min_h(px(MENU_ROW_PX))
                        .py(px(t.spacing.xs))
                        .px(px(t.spacing.lg))
                        .child(
                            crate::text::paragraph(
                                text.clone(),
                                TextSource::Fluent,
                                t.type_ramp.body,
                                ink,
                            )
                            .id("menu-item-text"),
                        );
                    return A11yNode::new(("menu-item-node", i), Role::MenuItem, text, false, row)
                        .disabled(true)
                        .into_any_element();
                }
                // The check column (12 px, `blue` check) sits before the label on every row of a
                // menu that has one.
                // ## m3a-t35-shots (named exception): the label never gives way to the hint.
                let mut lead = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(t.spacing.xl));
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
                } else if item.note {
                    // ## m3b-t26-prod-unlock (named exception): the note's 12 px caption.
                    (t.type_ramp.caption, TextSource::Fluent)
                } else {
                    (t.type_ramp.body, TextSource::Fluent)
                };
                // ## m3b-t29-shots (named exception; T29 review I4): the row's label says it is a
                // menu row (`#menu-item-label`), which no button outside a menu draws.
                lead = lead.child(key_label_from(
                    &text,
                    source,
                    Some(keys[i]),
                    style,
                    ink,
                    Some(&ElementId::from("menu-item-label")),
                ));
                let mut row = div()
                    .id(("menu-item", i))
                    .debug_selector(move || format!("menu-item-{i}"))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(t.spacing.xl))
                    .h(px(MENU_ROW_PX))
                    .flex_none()
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
                        .pl(px(t.spacing.xs))
                        // ## m3a-t35-shots (named exception): the hint gives way to the label.
                        .min_w(px(0.0));
                    if let Some(hint) = &item.hint {
                        // ## m3a-f2-visual: the active row's hint goes to `text` (chooser).
                        let hint_ink = if self.active_tint && selected && !item.disabled {
                            c.text.to_hsla()
                        } else {
                            c.text_2.to_hsla()
                        };
                        // Raised Ink Rule: the hint on the raised menu uses text-2.
                        // ## m3a-t35-shots (named exception): a hint wider than the menu leaves
                        // it ellipsizes (the chooser's SQL label, data, NOTES surface 4), the
                        // whole text in its tooltip; one that fits draws as before.
                        trail = trail.child(
                            Label::new(hint(cx), item.hint_source, t.type_ramp.caption, hint_ink)
                                .truncate(true)
                                // ## m3b-t29-shots (named exception)
                                .id("menu-item-hint"),
                        );
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
                        // ## m3a-f2-visual: `blue-tint-strong` where the row reads as a
                        // selection (the chooser), else the hover fill.
                        row = row.bg(if self.active_tint {
                            c.blue_tint_strong.to_hsla()
                        } else {
                            c.hover.to_hsla()
                        });
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
        // ## m3a-f5-demo (follow-up): the rows' column records where each row drew; an anchored
        // menu then records it with the column's bounds ([`MenuLayout`]).
        let drawn_rows: std::rc::Rc<std::cell::RefCell<Vec<Bounds<Pixels>>>> = Default::default();
        let has_title = self.title.is_some();
        let statusbar = t.spacing.statusbar_height;
        // ## m3b-t29-shots (named exception; T29 review I1): the menu is a card: text that runs
        // past its sides is recorded so (`RenderedText::outside_card`), as a dialog's is. Its
        // scope's canvas is the surface's first child, so the rows' column comes one later.
        let surface = popover_surface(&theme);
        let surface = if self.anchor.is_some() {
            let drawn_rows = drawn_rows.clone();
            let first = shown.start;
            crate::text::card_scope_then(surface, move |bounds, window, cx| {
                let column = bounds.get(1 + usize::from(has_title)).copied();
                let rows = drawn_rows
                    .borrow()
                    .iter()
                    .enumerate()
                    .map(|(k, b)| (first + k, *b))
                    .collect();
                note_menu_rows(cx, window, statusbar, column, rows);
            })
        } else {
            crate::text::card_scope(surface)
        };
        let menu = surface
            .id("popup-menu")
            .debug_selector(|| "popup-menu".to_string())
            .track_focus(&self.focus)
            .declare(Role::Menu, menu_name.clone(), None);
        let mut menu = menu.min_w(self.min_width);
        // ## m3a-t35-shots (named exception)
        // ## m3b-ci (controller fix): an anchored menu is also never wider than the window
        // less its margins; past that its hints ellipsize (the label never gives way).
        let window_room = (position.is_some())
            .then(|| (window.viewport_size().width - px(2.0 * t.spacing.xs)).max(px(0.0)));
        let max = match (self.max_width, window_room) {
            (Some(max), Some(room)) => Some(max.min(room)),
            (max, room) => max.or(room),
        };
        if let Some(max) = max {
            menu = menu.max_w(max.max(self.min_width));
        }
        let menu = menu
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
        // ## m3a-f2-visual: the key-hint footer (the mock's `.menu .mfoot`: 24 px on `head`, a
        // `seam` top edge, edge to edge under the rows; chords in mono 11, words 11.5, both
        // `text-2` on the raised menu).
        let footer = (!self.key_hints.is_empty()).then(|| {
            let hint = |(chord, id): &(&'static str, &'static str)| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(5.0))
                    .child(Label::new(
                        crate::palette::key_label(chord),
                        TextSource::Shortcut,
                        t.type_ramp.mono_kbd,
                        c.text_2.to_hsla(),
                    ))
                    .child(Label::new(
                        crate::l10n::ui_text(cx, id),
                        TextSource::Fluent,
                        t.type_ramp.micro,
                        c.text_2.to_hsla(),
                    ))
            };
            div()
                .debug_selector(|| "popup-menu-footer".to_string())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.xl))
                .h(px(MENU_FOOTER_PX - t.spacing.xs))
                .mt(px(t.spacing.xs))
                .mx(px(-t.spacing.xs))
                .mb(px(-t.spacing.xs))
                .px(px(t.spacing.lg))
                .border_t_1()
                .border_color(c.seam.to_hsla())
                .bg(c.head.to_hsla())
                .rounded_bl(px(t.radii.md))
                .rounded_br(px(t.radii.md))
                .children(self.key_hints.iter().map(hint))
        });
        // ## m3a-f5-demo (follow-up): the rows in their own column, which scrolls when the menu
        // has less room than its rows (`rows_window`, whole rows); an anchored menu records
        // where its rows drew for the frame checks ([`MenuLayout`]).
        let column = div()
            .on_children_prepainted(move |bounds, _, _| {
                *drawn_rows.borrow_mut() = bounds;
            })
            .id("popup-menu-rows")
            .flex()
            .flex_col()
            .flex_none();
        // A window of whole rows: the wheel moves it a row at a time (review m-2).
        let column = if room.is_some() {
            column.on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| this.on_wheel(e, cx)))
        } else {
            column
        };
        let column = column.children(rows);
        let menu = menu.children(title).child(column).children(footer);
        // ## m3b-ci (controller fix): an anchored menu wider than the room right of its anchor
        // moves left until it ends inside the window (CI run 44051024805: with Linux's wider
        // fonts the transaction menu ran past the right edge at 960x600 and its hints were cut
        // there). Placing from `min_width` alone cannot see the menu's real width; `anchored`
        // shifts it once laid out. A menu that fits stays where it was placed.
        match position {
            Some((left, top)) => anchored()
                .position(point(left, top))
                .snap_to_window_with_margin(px(t.spacing.xs))
                .child(menu)
                .into_any_element(),
            None => menu.into_any_element(),
        }
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

    // ## m3b-t26-prod-unlock (named exception: the T22 review m12 carry)
    /// A note is `text-2` on the raised menu (the Raised Ink Rule), never disabled `text-3`,
    /// and never selectable or chosen.
    #[test]
    fn a_note_is_text_2_and_never_selectable() {
        for c in [&DARK.colors, &LIGHT.colors] {
            let note = item().note(true);
            assert_eq!(note.label_ink(c), c.text_2.to_hsla());
            assert!(!note.selectable());
            assert!(
                note.disabled,
                "every path that skips a disabled row skips it"
            );
            assert!(item().selectable() && !item().note);
        }
    }
}
