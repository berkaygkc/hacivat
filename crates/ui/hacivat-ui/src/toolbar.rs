//! The Overflow Rule toolbar (DESIGN.md Overflow Rule; Task 21): items by priority, measured at
//! their natural width; trailing items move into a 24 px overflow menu, lowest priority first,
//! before any label truncates. 34 px on `raised` (the editor's bar passes
//! [`Toolbar::height`]), `text-2` ink.
//!
//! How it fits. The bar lays its items out itself, in `prepaint`: every item (and every
//! compact form) is laid out once at its natural width with `AnyElement::layout_as_root`, the
//! pure [`stage_fit`] picks what stays, and the shown items are laid out again in one row.
//! A slot is the same wrapper in both passes, so the measured width is the painted width.
//!
//! Collapse order (design/m3a-query NOTES, ruling 14). Candidates go lowest priority first, a
//! tie going to the later item. A candidate with a compact form takes it first (its text stays
//! as a tooltip and as the accessible name); if it can move, it then leaves the row for the
//! menu. A pinned item never collapses. The menu lists, in priority order, every item that left
//! the row and every compacted item that asked for a menu row, and its button (a 24 px ghost
//! icon, "More") exists only while the menu has a row.
//!
//! Keyboard: the button is a Tab stop; Enter, Space or a click opens the menu, arrows move,
//! Enter chooses, Escape closes and returns the focus to the button.
//!
//! ## m3a-f2-visual (named exception): the wrapping stage. One item may [`ToolbarItem::wraps`]:
//! when the row still does not fit once every other candidate collapsed, that item's sentence
//! wraps in the room the others leave and the bar grows to hold its lines (the cursor strip's
//! reason, NOTES surface 11: padding 4, line height 16). Such a bar measures its items in
//! `request_layout` too, so its height is known before it is laid out (GPUI's measured layout,
//! as its `List` does); a bar without a wrapping item is laid out exactly as before.

use std::rc::Rc;

use hacivat_ui_core::gpui_surface::{
    Anchor, AnyElement, App, AvailableSpace, Bounds, Context, Div, Element, ElementId, Entity,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement,
    LayoutId, MouseButton, ParentElement, Pixels, Role, RunCommand, SharedString,
    StatefulInteractiveElement, Style, Styled, Subscription, Window, anchored, canvas, deferred,
    div, point, px, relative, size,
};
use hacivat_ui_core::gpui_surface::{Global, Hsla, TextRun};
use hacivat_ui_core::theme::{Theme, TypeStyle};

use crate::actions::ActionRegistry;
use crate::button::{Button, ButtonKind};
use crate::icon::IconName;
use crate::l10n::ui_text;
use crate::menu::{MenuItemSpec, PopupMenu, PopupMenuEvent};
use crate::popover::{Declare, Tip, lazy};
use crate::text::TextFn;

/// Width of the overflow button.
pub const OVERFLOW_BUTTON_PX: f32 = 24.0;
/// The gap between items.
const MENU_GAP_PX: f32 = 4.0;

/// Builds one item's element; called again for every frame and for every measurement.
pub type ItemFn = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
/// What choosing an overflow row does when it has no registered command.
pub type ChooseFn = Rc<dyn Fn(&mut Window, &mut App)>;

/// One toolbar item.
#[derive(Clone)]
pub struct ToolbarItem {
    /// Higher stays longer; the lowest collapses first.
    pub priority: u8,
    pub element: ItemFn,
    /// The menu row's text, and the tooltip and accessible name of the compact form.
    pub overflow_label: TextFn,
    /// The command the menu row runs (`ActionRegistry::run`).
    pub action: Option<RunCommand>,
    /// A narrower form this item takes before it leaves the row.
    pub compact: Option<ItemFn>,
    /// The compact form also adds a menu row (the notice's "Why this console is read-only…").
    pub compact_in_menu: bool,
    /// Never leaves the row (Run/Stop, the lock, trust signals); it may still take its compact
    /// form, and only after every item that can move has moved.
    pub pinned: bool,
    /// What choosing the menu row does when `action` is none. A row with neither is shown
    /// disabled: it only keeps the collapsed text readable.
    pub on_choose: Option<ChooseFn>,
    // ## m3a-f2-visual (named exception)
    /// The last stage: when the row still does not fit once every other item collapsed, this
    /// item's sentence wraps and the bar grows (the cursor strip, NOTES surface 11).
    pub wrap: Option<Wrap>,
}

// ## m3a-f2-visual (named exception): the toolbar's wrapping stage.
/// How a wrapping item draws and measures its sentence.
#[derive(Clone)]
pub struct Wrap {
    /// The sentence as the item draws it in the form it wraps in (its compact form when it
    /// has one: every compact form is taken before the last stage).
    pub text: SharedString,
    /// Its style; `line_height` is the wrapped lines' pitch.
    pub style: TypeStyle,
    /// The wrapped form: the item with its sentence in a paragraph that fills its slot.
    pub element: ItemFn,
}

/// A wrapped bar's padding above and below its lines.
pub const WRAP_PAD_PX: f32 = 4.0;

impl ToolbarItem {
    pub fn new(priority: u8, element: ItemFn, overflow_label: TextFn) -> Self {
        Self {
            priority,
            element,
            overflow_label,
            action: None,
            compact: None,
            compact_in_menu: false,
            pinned: false,
            on_choose: None,
            wrap: None,
        }
    }

    /// An item that never collapses.
    pub fn pinned(element: ItemFn, overflow_label: TextFn) -> Self {
        Self {
            pinned: true,
            ..Self::new(u8::MAX, element, overflow_label)
        }
    }

    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn action(mut self, command: RunCommand) -> Self {
        self.action = Some(command);
        self
    }

    pub fn compact(mut self, element: ItemFn) -> Self {
        self.compact = Some(element);
        self
    }

    pub fn compact_in_menu(mut self, in_menu: bool) -> Self {
        self.compact_in_menu = in_menu;
        self
    }

    pub fn on_choose(mut self, on_choose: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_choose = Some(Rc::new(on_choose));
        self
    }

    // ## m3a-f2-visual (named exception)
    /// Makes this item the bar's wrapping stage (see [`Wrap`]); one item per bar.
    pub fn wraps(mut self, wrap: Wrap) -> Self {
        self.wrap = Some(wrap);
        self
    }
}

// ---- the pure fit -------------------------------------------------------------------------

/// What [`stage_fit`] needs to know about one item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitItem {
    pub width: f32,
    pub priority: u8,
    /// The compact form's width.
    pub compact: Option<f32>,
    /// May leave the row for the menu.
    pub movable: bool,
    /// Taking the compact form adds a menu row.
    pub compact_in_menu: bool,
}

impl FitItem {
    pub fn new(width: f32, priority: u8) -> Self {
        Self {
            width,
            priority,
            compact: None,
            movable: true,
            compact_in_menu: false,
        }
    }
}

/// Which items stay in the row and which move into the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fit {
    /// Indices that stay in the row, in item order (compacted ones included).
    pub shown: Vec<usize>,
    /// Indices that left the row, in item order.
    pub overflow: Vec<usize>,
}

/// [`Fit`] with the compact forms and the menu's rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub fit: Fit,
    /// Shown in their compact form.
    pub compact: Vec<usize>,
    /// The menu's rows in priority order (lowest priority first, ties by item order).
    pub menu: Vec<usize>,
}

/// The Overflow Rule over plain `(width, priority)` pairs, every item movable, no gaps.
pub fn fit(items: &[(f32, u8)], available: f32) -> Fit {
    let items: Vec<FitItem> = items.iter().map(|&(w, p)| FitItem::new(w, p)).collect();
    stage_fit(&items, available, 0.0, OVERFLOW_BUTTON_PX).fit
}

/// The Overflow Rule: collapses candidates lowest priority first until the row, with `gap`
/// between its children and the overflow `button` once the menu has a row, fits `available`.
pub fn stage_fit(items: &[FitItem], available: f32, gap: f32, button: f32) -> Staged {
    #[derive(Clone, Copy, PartialEq)]
    enum Stage {
        Full,
        Compact,
        Moved,
    }
    let mut stage = vec![Stage::Full; items.len()];
    let in_menu = |stage: &[Stage], i: usize| {
        stage[i] == Stage::Moved || (stage[i] == Stage::Compact && items[i].compact_in_menu)
    };
    let total = |stage: &[Stage]| {
        let mut sum = 0.0;
        let mut children = 0usize;
        for (i, item) in items.iter().enumerate() {
            match stage[i] {
                Stage::Full => sum += item.width,
                Stage::Compact => sum += item.compact.unwrap_or(item.width),
                Stage::Moved => continue,
            }
            children += 1;
        }
        if (0..items.len()).any(|i| in_menu(stage, i)) {
            sum += button;
            children += 1;
        }
        sum + gap * children.saturating_sub(1) as f32
    };
    let mut order: Vec<usize> = (0..items.len())
        .filter(|&i| items[i].movable || items[i].compact.is_some())
        .collect();
    order.sort_by(|&a, &b| items[a].priority.cmp(&items[b].priority).then(b.cmp(&a)));
    'collapse: for i in order {
        if total(&stage) <= available {
            break;
        }
        if items[i].compact.is_some() {
            stage[i] = Stage::Compact;
            if total(&stage) <= available {
                break 'collapse;
            }
        }
        if items[i].movable {
            stage[i] = Stage::Moved;
        }
    }
    let shown = (0..items.len())
        .filter(|&i| stage[i] != Stage::Moved)
        .collect();
    let overflow = (0..items.len())
        .filter(|&i| stage[i] == Stage::Moved)
        .collect();
    let compact = (0..items.len())
        .filter(|&i| stage[i] == Stage::Compact)
        .collect();
    let mut menu: Vec<usize> = (0..items.len()).filter(|&i| in_menu(&stage, i)).collect();
    menu.sort_by_key(|&i| (items[i].priority, i));
    Staged {
        fit: Fit { shown, overflow },
        compact,
        menu,
    }
}

// ---- the probe ----------------------------------------------------------------------------

/// What the last frame decided, recorded only while this global is installed (tests and the
/// shots harness; the app never installs it).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolbarProbe {
    /// The natural width measured for each item (its full form), in item order.
    pub measured: Vec<f32>,
    pub staged: Option<Staged>,
    /// The width the row had.
    pub available: f32,
}

impl Global for ToolbarProbe {}

// ---- state --------------------------------------------------------------------------------

struct MenuEntry {
    label: TextFn,
    action: Option<RunCommand>,
    on_choose: Option<ChooseFn>,
}

/// The overflow button's state, kept with the toolbar across frames.
struct ToolbarState {
    focus: FocusHandle,
    menu: Option<Entity<PopupMenu>>,
    subscriptions: Vec<Subscription>,
    button_bounds: Option<Bounds<Pixels>>,
    /// The menu's rows as of the last frame.
    entries: Vec<MenuEntry>,
    swallow_click: bool,
}

impl ToolbarState {
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_some() || self.entries.is_empty() {
            return;
        }
        let items: Vec<MenuItemSpec> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                MenuItemSpec::new(e.label.clone(), i.to_string())
                    .disabled(e.action.is_none() && e.on_choose.is_none())
            })
            .collect();
        let menu = PopupMenu::embedded(items, window, cx);
        menu.update(cx, |menu, _| {
            menu.label(Rc::new(|cx| ui_text(cx, "hui-menu-more")))
        });
        let events = cx.subscribe_in(&menu, window, Self::on_menu_event);
        let menu_focus = menu.focus_handle(cx);
        let blur = cx.on_focus_out(&menu_focus, window, |this, _, window, cx| {
            this.close(false, window, cx)
        });
        self.menu = Some(menu);
        self.subscriptions = vec![events, blur];
        self.swallow_click = false;
        cx.notify();
    }

    fn close(&mut self, restore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        // Dropped once this update is done: the event in flight still reaches every subscriber.
        let subscriptions = std::mem::take(&mut self.subscriptions);
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
            PopupMenuEvent::Chosen(index) => {
                let picked = index
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| self.entries.get(i))
                    .map(|e| (e.action.clone(), e.on_choose.clone()));
                self.close(true, window, cx);
                match picked {
                    Some((Some(command), _)) => {
                        ActionRegistry::run(&command.id, window, cx);
                    }
                    Some((None, Some(run))) => run(window, cx),
                    _ => {}
                }
            }
            PopupMenuEvent::Dismissed => {
                // A press on the button itself dismissed the menu; the click it finishes must
                // not open it again.
                let on_button = !window.last_input_was_keyboard()
                    && self
                        .button_bounds
                        .is_some_and(|b| b.contains(&window.mouse_position()));
                self.close(true, window, cx);
                self.swallow_click = on_button;
            }
        }
    }
}

// ---- the toolbar --------------------------------------------------------------------------

pub struct Toolbar {
    id: ElementId,
    items: Vec<ToolbarItem>,
    trailing: Vec<ToolbarItem>,
    height: Option<f32>,
    // ## m3a-f2-visual (named exception)
    wrap_row: bool,
}

impl Toolbar {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            trailing: Vec::new(),
            height: None,
            wrap_row: false,
        }
    }

    // ## m3a-f2-visual (named exception)
    /// The very last stage: when even the wrapping item cannot fit (a narrow pane), the row
    /// itself wraps onto lines and the bar grows, so no pinned item (a trust signal: the
    /// production lock, the read-only chip, the environment mark) is ever cut by its edge.
    pub fn wrap_row(mut self) -> Self {
        self.wrap_row = true;
        self
    }

    /// An item on the left, after the ones already added.
    pub fn item(mut self, item: ToolbarItem) -> Self {
        self.items.push(item);
        self
    }

    /// An item on the right, after the ones already added there.
    pub fn trailing(mut self, item: ToolbarItem) -> Self {
        self.trailing.push(item);
        self
    }

    /// The bar's height in px (the editor's bar is 32; the default is the result bar's 34).
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    // ## m3a-f2-visual (named exception)
    /// Whether the bar measures itself (a wrapping item, or a wrapping row): its height can
    /// grow. At most one item wraps (review M10).
    fn measured(&self) -> bool {
        let wrapping = self
            .items
            .iter()
            .chain(self.trailing.iter())
            .filter(|i| i.wrap.is_some())
            .count();
        debug_assert!(wrapping <= 1, "a toolbar has at most one wrapping item");
        wrapping > 0 || self.wrap_row
    }
}

impl IntoElement for Toolbar {
    type Element = AnyElement;

    fn into_element(self) -> AnyElement {
        lazy(move |window, cx| self.build(window, cx)).into_any_element()
    }
}

impl Toolbar {
    fn build(self, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let height = self.height.unwrap_or(t.spacing.result_toolbar_height);
        let id = self.id.clone();
        let selector = format!("toolbar-{id}");
        let ink: Hsla = c.text_2.to_hsla();
        // ## m3a-f2-visual (named exception): a bar with a wrapping item is at least its
        // height and grows with the wrapped lines (its row measures itself).
        let wraps = self.measured();
        // The bar draws its own chrome; the row element inside it fills it.
        let bar = div()
            .debug_selector(move || selector)
            .flex()
            .flex_row()
            .items_center()
            .w_full();
        let bar = if wraps {
            bar.min_h(px(height))
        } else {
            bar.h(px(height))
        };
        let inner = div().flex_1().min_w(px(0.0));
        let inner = if wraps { inner } else { inner.h_full() };
        bar.flex_none()
            .pl(px(t.spacing.xl))
            .pr(px(t.spacing.lg))
            .bg(c.raised.to_hsla())
            .text_color(ink)
            .child(inner.child(Row {
                toolbar: Some(self),
            }))
            .into_any_element()
    }
}

/// The row element: measures in `prepaint`, then lays out and paints what fits.
struct Row {
    toolbar: Option<Toolbar>,
}

struct RowPrepaint {
    row: AnyElement,
}

fn slot(
    id: &ElementId,
    index: usize,
    element: AnyElement,
    compact_of: Option<SharedString>,
) -> AnyElement {
    slot_sized(id, index, element, compact_of, None)
}

// ## m3a-f2-visual (named exception): `slot` at a given width (the wrapped item).
fn slot_sized(
    id: &ElementId,
    index: usize,
    element: AnyElement,
    compact_of: Option<SharedString>,
    width: Option<f32>,
) -> AnyElement {
    let base = div()
        .id(ElementId::from((id.clone(), format!("slot-{index}"))))
        .debug_selector(move || format!("toolbar-item-{index}"))
        .flex()
        .flex_row()
        .items_center()
        .flex_none();
    let base = match width {
        Some(w) => base.w(px(w)),
        None => base,
    }
    .child(element);
    match compact_of {
        // A collapsed segment keeps its text as the tooltip and as its accessible name.
        Some(text) => {
            let tip = text.clone();
            base.declare(Role::Group, text, None)
                .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                .into_any_element()
        }
        None => base.into_any_element(),
    }
}

// ## m3a-f2-visual (named exception): measuring shared by `request_layout` (a wrapping bar)
// and `prepaint`, the row's width once staged, and the wrapping bar's height.

/// The flexible space between the leading and the trailing items. Its negative right margin
/// cancels the second gap the row would put around it, so the row is exactly as wide as
/// [`staged_width`] counts (fix round 1: a tight row no longer pushes its trailing item a gap
/// past its edge); with room to spare it draws as before.
fn spacer(gap: f32) -> Div {
    div().flex_1().min_w(px(0.0)).mr(px(-gap))
}

/// Half a pixel of slack: a row that fits within it does not wrap.
const WRAP_SLACK_PX: f32 = 0.5;

/// Every item's form at its natural width, through the same slot as the paint pass (`height`:
/// the row's height when known); also the full widths, for the probe.
fn measure_items(
    toolbar: &Toolbar,
    height: AvailableSpace,
    window: &mut Window,
    cx: &mut App,
) -> (Vec<FitItem>, Vec<f32>) {
    let id = toolbar.id.clone();
    let space = size(AvailableSpace::MaxContent, height);
    let measure = |build: &ItemFn,
                   index: usize,
                   compact: Option<SharedString>,
                   window: &mut Window,
                   cx: &mut App| {
        let mut s = slot(&id, index, build(window, cx), compact);
        f32::from(s.layout_as_root(space, window, cx).width)
    };
    let all = toolbar.items.iter().chain(toolbar.trailing.iter());
    let mut fit_items = Vec::new();
    let mut measured = Vec::new();
    for (i, item) in all.enumerate() {
        let full = measure(&item.element, i, None, window, cx);
        let compact = match &item.compact {
            Some(f) => {
                let label = (item.overflow_label)(cx);
                Some(measure(f, i, Some(label), window, cx))
            }
            None => None,
        };
        measured.push(full);
        fit_items.push(FitItem {
            width: full,
            priority: item.priority,
            compact,
            movable: !item.pinned,
            compact_in_menu: item.compact_in_menu,
        });
    }
    (fit_items, measured)
}

/// Item `i`'s width in `staged`'s form.
fn own_width(items: &[FitItem], staged: &Staged, i: usize) -> f32 {
    if staged.compact.contains(&i) {
        items[i].compact.unwrap_or(items[i].width)
    } else {
        items[i].width
    }
}

/// The width the row takes in `staged`'s forms, with `gap` between its children and the
/// overflow `button` once the menu has a row (what [`stage_fit`] compares to the room).
pub fn staged_width(items: &[FitItem], staged: &Staged, gap: f32, button: f32) -> f32 {
    let mut sum: f32 = staged
        .fit
        .shown
        .iter()
        .map(|&i| own_width(items, staged, i))
        .sum();
    let mut children = staged.fit.shown.len();
    if !staged.menu.is_empty() {
        sum += button;
        children += 1;
    }
    sum + gap * children.saturating_sub(1) as f32
}

/// One run over all of `text` in `style`, for measuring it.
fn text_run(text: &SharedString, style: &TypeStyle) -> TextRun {
    TextRun {
        len: text.len(),
        font: hacivat_ui_core::fonts::font_for(style),
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// The least room a wrapping sentence keeps before a row-wrapping bar puts its items on
/// lines instead.
const MIN_WRAP_TEXT_PX: f32 = 48.0;
/// The overflow button's index in a [`Plan::Lines`] line.
const BUTTON_SLOT: usize = usize::MAX;

/// What a measured bar does at a width (the stages after [`stage_fit`]).
#[derive(Debug, Clone, PartialEq)]
enum Plan {
    /// The row fits once staged: one line at the bar's height.
    Fits,
    /// The wrapping item's sentence wraps; its slot takes `room` px (its own width less the
    /// excess).
    Wrap { room: f32 },
    /// The row itself wraps (a bar with [`Toolbar::wrap_row`]): each line's items (or
    /// [`BUTTON_SLOT`]) with their widths. An item wider than the bar gets the bar's width;
    /// the wrapping item then wraps its sentence in it.
    Lines(Vec<Vec<(usize, f32)>>),
}

/// The measured inputs of a bar's last stages, shared by `request_layout` and `prepaint`.
#[derive(Clone)]
struct Measure {
    items: Vec<FitItem>,
    /// The wrapping item: its index, what it draws besides its sentence (a glyph, a button),
    /// and its sentence.
    wrap: Option<(usize, f32, Wrap)>,
    wrap_row: bool,
    gap: f32,
    bar: f32,
}

impl Measure {
    fn of(toolbar: &Toolbar, items: Vec<FitItem>, window: &mut Window, cx: &App) -> Measure {
        let theme = *Theme::get(cx);
        let wrap = toolbar
            .items
            .iter()
            .chain(toolbar.trailing.iter())
            .enumerate()
            .find_map(|(i, item)| item.wrap.clone().map(|w| (i, w)))
            .map(|(i, wrap)| {
                let text = f32::from(
                    window
                        .text_system()
                        .shape_line(
                            wrap.text.clone(),
                            px(wrap.style.size_px),
                            &[text_run(&wrap.text, &wrap.style)],
                            None,
                        )
                        .width,
                );
                // Against the form the item wraps in: its compact one when it has one.
                let form = items[i].compact.unwrap_or(items[i].width);
                (i, (form - text).max(0.0), wrap)
            });
        Measure {
            items,
            wrap,
            wrap_row: toolbar.wrap_row,
            gap: theme.tokens.spacing.md,
            bar: toolbar
                .height
                .unwrap_or(theme.tokens.spacing.result_toolbar_height),
        }
    }

    fn staged(&self, width: f32) -> Staged {
        stage_fit(&self.items, width, self.gap, OVERFLOW_BUTTON_PX)
    }

    /// The last stages at `width`, once `staged`: the wrapping item's sentence wraps while it
    /// keeps [`MIN_WRAP_TEXT_PX`] (at any room, in a bar that cannot wrap its row); then a
    /// row-wrapping bar puts its items on lines, so no item is cut by the bar's edge.
    fn plan(&self, staged: &Staged, width: f32) -> Plan {
        let total = staged_width(&self.items, staged, self.gap, OVERFLOW_BUTTON_PX);
        if total <= width + WRAP_SLACK_PX {
            return Plan::Fits;
        }
        if let Some((w, overhead, _)) = &self.wrap
            && staged.fit.shown.contains(w)
        {
            let room = own_width(&self.items, staged, *w) - (total - width);
            if !self.wrap_row || room - overhead >= MIN_WRAP_TEXT_PX {
                return Plan::Wrap { room };
            }
        }
        if !self.wrap_row {
            return Plan::Fits;
        }
        let mut seq: Vec<(usize, f32)> = staged
            .fit
            .shown
            .iter()
            .map(|&i| (i, own_width(&self.items, staged, i).min(width)))
            .collect();
        if !staged.menu.is_empty() {
            seq.push((BUTTON_SLOT, OVERFLOW_BUTTON_PX));
        }
        let mut lines: Vec<Vec<(usize, f32)>> = Vec::new();
        let mut used = 0.0;
        for (i, w) in seq {
            match lines.last_mut() {
                Some(line) if used + self.gap + w <= width + WRAP_SLACK_PX => {
                    line.push((i, w));
                    used += self.gap + w;
                }
                _ => {
                    lines.push(vec![(i, w)]);
                    used = w;
                }
            }
        }
        Plan::Lines(lines)
    }

    /// The height of the wrapping item's sentence in `room` px of slot, with its padding.
    fn wrapped(&self, room: f32, window: &mut Window) -> f32 {
        let Some((_, overhead, wrap)) = &self.wrap else {
            return self.bar;
        };
        // Half a pixel narrower than the paragraph gets: never fewer lines than it paints.
        let text = (room - overhead - WRAP_SLACK_PX).max(1.0);
        let pitch = px(wrap.style.line_height);
        let lines: f32 = window
            .text_system()
            .shape_text(
                wrap.text.clone(),
                px(wrap.style.size_px),
                &[text_run(&wrap.text, &wrap.style)],
                Some(px(text)),
                None,
            )
            .map(|lines| lines.iter().map(|l| f32::from(l.size(pitch).height)).sum())
            .unwrap_or(wrap.style.line_height);
        (lines + 2.0 * WRAP_PAD_PX).max(self.bar)
    }

    /// Whether `plan` squeezes the wrapping item `i` to `width` (its sentence wraps).
    fn squeezed(&self, staged: &Staged, i: usize, width: f32) -> bool {
        self.wrap.as_ref().is_some_and(|(w, ..)| *w == i)
            && width + WRAP_SLACK_PX < own_width(&self.items, staged, i)
    }

    /// Each line's height in `plan`: the bar's, or the wrapped sentence's when taller.
    fn line_heights(&self, plan: &Plan, staged: &Staged, window: &mut Window) -> Vec<f32> {
        match plan {
            Plan::Fits => vec![self.bar],
            Plan::Wrap { room } => vec![self.wrapped(*room, window)],
            Plan::Lines(lines) => lines
                .iter()
                .map(|line| {
                    line.iter().fold(self.bar, |h, &(i, w)| {
                        if self.squeezed(staged, i, w) {
                            h.max(self.wrapped(w, window))
                        } else {
                            h
                        }
                    })
                })
                .collect(),
        }
    }
}

/// The measure function of a bar with a wrapping item or a wrapping row: its natural width,
/// and its height at the width it is given. `None` for any other bar.
#[allow(clippy::type_complexity, reason = "GPUI's measure function")]
fn wrap_measure(
    toolbar: &Toolbar,
    window: &mut Window,
    cx: &mut App,
) -> Option<
    impl Fn(
        hacivat_ui_core::gpui_surface::Size<Option<Pixels>>,
        hacivat_ui_core::gpui_surface::Size<AvailableSpace>,
        &mut Window,
        &mut App,
    ) -> hacivat_ui_core::gpui_surface::Size<Pixels>
    + 'static,
> {
    if !toolbar.measured() {
        return None;
    }
    let (items, _) = measure_items(toolbar, AvailableSpace::MaxContent, window, cx);
    let m = Measure::of(toolbar, items, window, cx);
    let natural = staged_width(
        &m.items,
        &m.staged(f32::INFINITY),
        m.gap,
        OVERFLOW_BUTTON_PX,
    );
    Some(
        move |known: hacivat_ui_core::gpui_surface::Size<Option<Pixels>>,
              available: hacivat_ui_core::gpui_surface::Size<AvailableSpace>,
              window: &mut Window,
              _: &mut App| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(w) => Some(w),
                _ => None,
            });
            match width {
                Some(w) => {
                    let staged = m.staged(f32::from(w));
                    let plan = m.plan(&staged, f32::from(w));
                    let h: f32 = m.line_heights(&plan, &staged, window).iter().sum();
                    size(w, px(h))
                }
                None => size(px(natural), px(m.bar)),
            }
        },
    )
}

impl Element for Row {
    type RequestLayoutState = Entity<ToolbarState>;
    type PrepaintState = Option<RowPrepaint>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Entity<ToolbarState>) {
        let key = self
            .toolbar
            .as_ref()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| ElementId::from("toolbar"));
        let state =
            window.use_keyed_state(ElementId::from((key, "state")), cx, |_, cx| ToolbarState {
                focus: cx.focus_handle().tab_stop(true),
                menu: None,
                subscriptions: Vec::new(),
                button_bounds: None,
                entries: Vec::new(),
                swallow_click: false,
            });
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        // ## m3a-f2-visual (named exception): a bar with a wrapping item measures its height
        // here (how many lines its sentence wraps to at the width it gets).
        if let Some(measure) = self
            .toolbar
            .as_ref()
            .and_then(|t| wrap_measure(t, window, cx))
        {
            return (window.request_measured_layout(style, measure), state);
        }
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Entity<ToolbarState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<RowPrepaint> {
        let toolbar = self.toolbar.take()?;
        let theme = *Theme::get(cx);
        let gap = theme.tokens.spacing.md;
        let id = toolbar.id.clone();
        let lead = toolbar.items.len();
        let all: Vec<&ToolbarItem> = toolbar
            .items
            .iter()
            .chain(toolbar.trailing.iter())
            .collect();

        // Pass one: every form at its natural width, through the same slot as the paint pass.
        // ## m3a-f2-visual (named exception): the measuring moved into `measure_items`, which
        // a wrapping bar's `request_layout` shares.
        let (fit_items, measured) = measure_items(
            &toolbar,
            AvailableSpace::Definite(bounds.size.height),
            window,
            cx,
        );
        let staged = stage_fit(
            &fit_items,
            f32::from(bounds.size.width),
            gap,
            OVERFLOW_BUTTON_PX,
        );
        if cx.has_global::<ToolbarProbe>() {
            let probe = cx.global_mut::<ToolbarProbe>();
            probe.measured = measured;
            probe.staged = Some(staged.clone());
            probe.available = f32::from(bounds.size.width);
        }

        // The menu's rows for the overflow button.
        let entries: Vec<MenuEntry> = staged
            .menu
            .iter()
            .map(|&i| MenuEntry {
                label: all[i].overflow_label.clone(),
                action: all[i].action.clone(),
                on_choose: all[i].on_choose.clone(),
            })
            .collect();
        // ## m3a-f2-visual (named exception, review M7): no ⋯ menu without a row that can be
        // chosen (a menu of disabled rows would take the focus with nowhere to go).
        let has_menu = entries
            .iter()
            .any(|e| e.action.is_some() || e.on_choose.is_some());
        let entries = if has_menu { entries } else { Vec::new() };
        state.update(cx, |s, _| s.entries = entries);
        // ## m3a-f2-visual (named exception): the last stages: the wrapping item's sentence
        // wraps (its slot takes the room the others leave), then a row-wrapping bar puts its
        // items on lines.
        let width = f32::from(bounds.size.width);
        let m = Measure::of(&toolbar, fit_items, window, cx);
        let plan = if m.wrap.is_some() || m.wrap_row {
            m.plan(&staged, width)
        } else {
            Plan::Fits
        };
        if let Plan::Lines(lines) = &plan {
            let heights = m.line_heights(&plan, &staged, window);
            let mut column = div().flex().flex_col().w(bounds.size.width);
            for (line, h) in lines.iter().zip(heights) {
                let mut row = div().flex().flex_row().items_center().gap(px(gap)).h(px(h));
                for &(i, w) in line {
                    if i == BUTTON_SLOT {
                        if has_menu {
                            row = row.child(overflow_button(&id, state, window, cx));
                        }
                        continue;
                    }
                    let item = all[i];
                    let compact = staged.compact.contains(&i);
                    let tip = compact.then(|| (item.overflow_label)(cx));
                    let element = match (&item.wrap, m.squeezed(&staged, i, w)) {
                        (Some(wrap), true) => (wrap.element)(window, cx),
                        _ if compact => {
                            let form = item.compact.as_ref().expect("a compact item has its form");
                            form(window, cx)
                        }
                        _ => (item.element)(window, cx),
                    };
                    row = row.child(slot_sized(&id, i, element, tip, Some(w)));
                }
                column = column.child(row);
            }
            let mut column = column.into_any_element();
            column.prepaint_as_root(
                bounds.origin,
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::Definite(bounds.size.height),
                ),
                window,
                cx,
            );
            return Some(RowPrepaint { row: column });
        }
        let wrapped: Option<(usize, f32)> = match plan {
            Plan::Wrap { room } => m.wrap.as_ref().map(|(w, ..)| (*w, room)),
            _ => None,
        };

        // Pass two: the row.
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(gap))
            .w(bounds.size.width)
            .h(bounds.size.height);
        let mut spacer_done = false;
        for &i in &staged.fit.shown {
            if i >= lead && !spacer_done {
                row = row.child(spacer(gap));
                spacer_done = true;
            }
            let item = all[i];
            // ## m3a-f2-visual (named exception): the wrapped item takes the room the others
            // leave (its own width less the excess); a compact one keeps its whole text as the
            // tooltip.
            if let Some((wi, room)) = wrapped
                && wi == i
                && let Some(wrap) = &item.wrap
            {
                let tip = staged
                    .compact
                    .contains(&i)
                    .then(|| (item.overflow_label)(cx));
                let element = (wrap.element)(window, cx);
                row = row.child(slot_sized(&id, i, element, tip, Some(room)));
                continue;
            }
            let element = if staged.compact.contains(&i) {
                let compact = item.compact.as_ref().expect("a compact item has its form");
                slot(&id, i, compact(window, cx), Some((item.overflow_label)(cx)))
            } else {
                slot(&id, i, (item.element)(window, cx), None)
            };
            row = row.child(element);
        }
        if !spacer_done {
            row = row.child(spacer(gap));
        }
        if has_menu {
            row = row.child(overflow_button(&id, state, window, cx));
        }
        let mut row = row.into_any_element();
        row.prepaint_as_root(
            bounds.origin,
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(bounds.size.height),
            ),
            window,
            cx,
        );
        Some(RowPrepaint { row })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Entity<ToolbarState>,
        prepaint: &mut Option<RowPrepaint>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(p) = prepaint {
            p.row.paint(window, cx);
        }
    }
}

/// The 24 px "More" button, with its menu (an embedded `PopupMenu` in a `deferred(anchored())`)
/// while open.
fn overflow_button(
    id: &ElementId,
    state: &Entity<ToolbarState>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (open, focus, bounds, menu) = {
        let s = state.read(cx);
        (
            s.menu.is_some(),
            s.focus.clone(),
            s.button_bounds,
            s.menu.clone(),
        )
    };
    let _ = window;
    let click_state = state.clone();
    let down_state = state.clone();
    let bounds_state = state.clone();
    let button = Button::new(
        ElementId::from((id.clone(), "overflow")),
        ui_text(cx, "hui-menu-more"),
    )
    .kind(ButtonKind::Icon)
    .fixed_width(OVERFLOW_BUTTON_PX)
    .icon(IconName::Ellipsis)
    .expanded(open)
    .focus_handle(focus)
    .on_click(move |event, window, cx| {
        // The keyboard opens the menu on the click it makes; a second toggle must not follow.
        let keyboard = matches!(
            event,
            hacivat_ui_core::gpui_surface::ClickEvent::Keyboard(_)
        );
        click_state.update(cx, |s, cx| {
            if std::mem::take(&mut s.swallow_click) && !keyboard {
                return;
            }
            if s.menu.is_some() {
                s.close(true, window, cx);
            } else {
                s.open(window, cx);
            }
        });
    });
    let mut root = div()
        .relative()
        .flex_none()
        .debug_selector(|| "toolbar-overflow".to_string())
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            down_state.update(cx, |s, _| s.swallow_click = false)
        })
        .child(button)
        .child(
            canvas(
                move |bounds, _, cx| {
                    bounds_state.update(cx, |s, _| s.button_bounds = Some(bounds));
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        );
    if let Some(menu) = menu {
        let place = match bounds {
            Some(bounds) => anchored()
                .anchor(Anchor::TopRight)
                .attach(bounds, Anchor::BottomRight)
                .offset(point(px(0.0), px(MENU_GAP_PX))),
            None => anchored(),
        };
        root = root.child(deferred(place.child(menu)).with_priority(1));
    }
    root.into_any_element()
}
