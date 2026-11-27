//! The result tab strip and its toolbar (Task 30).
//!
//! - **Tab strip** (30 px on `head`, the shell's `TabStrip` anatomy; NOTES surface 9): Output
//!   (scroll-text glyph) first, with a 6 px `text-2` dot while it has lines the user has not
//!   seen; then `Result N` (table glyph) per result. A production result carries the 7 px red
//!   swatch; a staging one the hollow swatch and "(staging)" in `text-2`. The active tab is
//!   `raised` (its ink `text`), the others `head` (ink `text-2`).
//! - **Result toolbar** (34 px `raised`, Task 21's Overflow Rule bar): Copy, and on the right
//!   the rows and their time in mono `text-2`.
//! - **Output toolbar**: Clear, Copy, Scroll to end.
//! - **The column hint** under the toolbar when a result has more than 2,000 columns: a `raised`
//!   info row of prose (it wraps), outside the toolbar so the Overflow Rule never hides it.
//!
//! No M3b or M6 affordance (sort, find, Export, Edit) is offered here.
//!
//! ## m3b-t25-results (design/m3b-safe surface 10)
//! - **Tab titles** are data when they are a statement's comment: at most 190 px, ellipsized
//!   with the whole title in the tooltip (120 px in a short strip, NOTES ruling 30). A pinned
//!   tab adds the 12 px pin glyph (`text-2`, tooltip `hv-results-tab-pinned`); an untyped
//!   result (run as one) adds "untyped (run as one)" in `text-2` (its short form "untyped" in a
//!   short strip), the tooltip saying why. A right press on a result tab opens its menu.
//! - **The result toolbar** adds Σ Count rows after Copy (`count.rs`).
//! - **The result-set limit** (R-M3-66) is an info row under the toolbar, as the column hint.

use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::EnvTag;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::{Declare as _, Tip};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toolbar::{Toolbar, ToolbarItem};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, Bounds, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, Role, SharedString, StatefulInteractiveElement, Styled, Window,
    canvas, div, px,
};
use hacivat_ui_core::theme::Theme;

use crate::connections::capsule::env_word_id;
use crate::l10n_global::tr;
use crate::results::tabs::ActiveResult;

/// More columns than this get the hint row (HV-GRID-02).
pub const WIDE_RESULT_COLUMNS: usize = 2_000;

/// What the tab strip shows.
#[derive(Debug, Clone)]
pub struct TabsModel {
    pub active: ActiveResult,
    pub unread: bool,
    /// Each result tab's title.
    pub results: Vec<SharedString>,
    /// The environment of the console's connection.
    pub env: Option<EnvTag>,
    // ## m3b-t25-results
    /// Each result tab's M3b facts, in the order of `results` (missing: none of them).
    pub facts: Vec<TabFacts>,
    /// The strip is short: the untyped note takes its short form and titles ellipsize at
    /// [`SHORT_TITLE_PX`] (NOTES ruling 30).
    pub short: bool,
    // Fix round 1 (I1, m9)
    /// Each result tab's whole title (its accessible name; `results` holds what the label
    /// draws, the repeat `(k)` apart).
    pub names: Vec<SharedString>,
    /// The result tabs the strip draws, in order (`None`: every one); the others are counted
    /// on the "+N" button ([`more_button`]).
    pub shown: Option<Vec<usize>>,
}

// ## m3b-t25-results

/// The widest a result tab's title is drawn (data; the whole title is its tooltip).
pub const TITLE_PX: f32 = 190.0;
/// [`TITLE_PX`] in a short strip.
pub const SHORT_TITLE_PX: f32 = 120.0;

/// What a result tab shows beside its title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TabFacts {
    /// The title is data (a statement's comment), not a Fluent message.
    pub data_title: bool,
    pub pinned: bool,
    /// The result came untyped (run as one).
    pub untyped: bool,
    /// Fix round 1 (m9): `k` of `(k)`, drawn after the title so a cut title keeps it.
    pub repeat: Option<u64>,
}

// Fix round 1 (I1): a result tab never shrinks below its chrome; tabs that do not fit leave
// the strip for the "+N" menu, so every drawn tab (and its environment swatch) is whole.

/// A tab's padding, glyph and right seam.
pub const TAB_CHROME_PX: f32 = 12.0 * 2.0 + 12.0 + 1.0;
/// The gap between a tab's parts.
pub const TAB_GAP_PX: f32 = 7.0;
/// The pin glyph, and the environment swatch.
pub const TAB_GLYPH_PX: f32 = 12.0;
pub const TAB_SWATCH_PX: f32 = 7.0;
/// The unread dot.
pub const TAB_DOT_PX: f32 = 6.0;

/// What a press on the "+N" button does, at the point under it.
pub type OnMore = Rc<dyn Fn(Point<Pixels>, &mut Window, &mut App)>;

/// What a right press on result tab `ix` does (at the pointer).
pub type OnTabMenu = Rc<dyn Fn(usize, Point<Pixels>, &mut Window, &mut App)>;

/// Told where result tab `ix` is painted (its menu opens under it from the keyboard and the shots).
pub type OnTabBounds = Rc<dyn Fn(usize, Bounds<Pixels>)>;

/// What a click on a tab does.
pub type OnActivate = Rc<dyn Fn(ActiveResult, &mut Window, &mut App)>;

/// The result tab strip.
pub fn tab_strip(
    model: &TabsModel,
    on_activate: OnActivate,
    on_menu: Option<OnTabMenu>,
    on_bounds: Option<OnTabBounds>,
    on_more: Option<OnMore>,
    cx: &App,
) -> AnyElement {
    let theme = Theme::get(cx);
    let t = theme.tokens;
    let c = t.colors;
    let mut strip = div()
        .id("results-tabs")
        .debug_selector(|| "results-tabs".to_string())
        .declare(Role::TabList, tr(cx, ids::hv::RESULTS_TABS, NoArgs), None)
        .flex()
        .flex_row()
        .items_start()
        .w_full()
        .h(px(t.spacing.panel_header_height))
        .flex_none()
        .overflow_hidden()
        .bg(c.head.to_hsla());
    let output = tr(cx, ids::hv::RESULTS_OUTPUT, NoArgs);
    strip = strip.child(tab(
        TabSpec {
            selector: "results-tab-output".into(),
            label_id: "results-tab-output".into(),
            title: output,
            icon: IconName::ScrollText,
            active: model.active == ActiveResult::Output,
            unread: model
                .unread
                .then(|| tr(cx, ids::hv::RESULTS_UNREAD, NoArgs)),
            env: None,
            env_id: None,
            target: ActiveResult::Output,
            facts: None,
            short: model.short,
            on_menu: None,
            on_bounds: None,
            name: None,
        },
        on_activate.clone(),
        cx,
    ));
    let shown: Vec<usize> = match &model.shown {
        Some(shown) => shown.clone(),
        None => (0..model.results.len()).collect(),
    };
    for ix in shown.iter().copied() {
        let Some(title) = model.results.get(ix) else {
            continue;
        };
        let env = match model.env {
            Some(EnvTag::Production) => Some((EnvTag::Production, None)),
            Some(EnvTag::Staging) => Some((
                EnvTag::Staging,
                Some(tr(
                    cx,
                    ids::hv::RESULTS_ENV_SUFFIX,
                    ids::hv::HvResultsEnvSuffix {
                        env: Text(tr(cx, env_word_id(Some(EnvTag::Staging)), NoArgs).to_string()),
                    },
                )),
            )),
            _ => None,
        };
        strip = strip.child(tab(
            TabSpec {
                selector: format!("results-tab-{ix}").into(),
                label_id: format!("results-tab-{ix}").into(),
                title: title.clone(),
                icon: IconName::Table2,
                active: model.active == ActiveResult::Tab(ix),
                unread: None,
                env,
                env_id: Some(format!("results-tab-env-{ix}").into()),
                target: ActiveResult::Tab(ix),
                // ## m3b-t25-results
                facts: Some(model.facts.get(ix).copied().unwrap_or_default()),
                short: model.short,
                on_menu: on_menu.clone(),
                on_bounds: on_bounds
                    .clone()
                    .map(|f| -> Rc<dyn Fn(Bounds<Pixels>)> { Rc::new(move |b| f(ix, b)) }),
                name: model.names.get(ix).cloned(),
            },
            on_activate.clone(),
            cx,
        ));
    }
    // Fix round 1 (I1): the tabs left out.
    let hidden = model.results.len().saturating_sub(shown.len());
    if hidden > 0 {
        strip = strip.child(more_button(hidden, on_more, cx));
    }
    strip.into_any_element()
}

// Fix round 1 (I1)
/// The "+N" button after the drawn tabs: `hidden` result tabs, listed in a menu.
pub fn more_button(hidden: usize, on_more: Option<OnMore>, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let text = more_text(hidden, cx);
    let tip = tr(
        cx,
        ids::hv::RESULTS_TABS_MORE_TIP,
        ids::hv::HvResultsTabsMoreTip {
            n: Count(hidden as u64),
        },
    );
    let mut el = div()
        .id("results-tabs-more")
        .debug_selector(|| "results-tabs-more".to_string())
        .declare(Role::Button, tip.clone(), None)
        .flex()
        .flex_row()
        .items_center()
        .h(px(t.spacing.panel_header_height))
        .flex_none()
        .px(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(c.hover.to_hsla()))
        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
        .child(
            Label::new(
                text,
                TextSource::Fluent,
                t.type_ramp.body,
                c.text_2.to_hsla(),
            )
            .id("results-tabs-more"),
        );
    if let Some(on_more) = on_more {
        el = el
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |event, window, cx| {
                let at = event.position();
                let on_more = on_more.clone();
                window.defer(cx, move |window, cx| on_more(at, window, cx));
            });
    }
    el.into_any_element()
}

/// The "+N" button's text.
pub fn more_text(hidden: usize, cx: &App) -> SharedString {
    tr(
        cx,
        ids::hv::RESULTS_TABS_MORE,
        ids::hv::HvResultsTabsMore {
            n: Count(hidden as u64),
        },
    )
}

struct TabSpec {
    selector: SharedString,
    label_id: SharedString,
    title: SharedString,
    icon: IconName,
    active: bool,
    /// The unread dot's accessible name.
    unread: Option<SharedString>,
    env: Option<(EnvTag, Option<SharedString>)>,
    env_id: Option<SharedString>,
    target: ActiveResult,
    // ## m3b-t25-results
    /// A result tab's facts (`None`: the Output tab).
    facts: Option<TabFacts>,
    short: bool,
    on_menu: Option<OnTabMenu>,
    on_bounds: Option<Rc<dyn Fn(Bounds<Pixels>)>>,
    /// The accessible name when it differs from the drawn title (a repeat).
    name: Option<SharedString>,
}

fn tab(spec: TabSpec, on_activate: OnActivate, cx: &App) -> AnyElement {
    let theme = *Theme::get(cx);
    let t = theme.tokens;
    let c = t.colors;
    let ink = if spec.active { c.text } else { c.text_2 }.to_hsla();
    let selector = spec.selector.to_string();
    let target = spec.target;
    let mut el = div()
        .id(ElementId::Name(spec.selector.clone()))
        .debug_selector(move || selector)
        .declare(
            Role::Tab,
            spec.name.clone().unwrap_or_else(|| spec.title.clone()),
            Some(spec.active),
        )
        .flex()
        .flex_row()
        .items_center()
        .h(px(t.spacing.panel_header_height))
        .flex_none()
        .px(px(12.0))
        .gap(px(7.0))
        .border_r_1()
        .border_color(c.seam.to_hsla())
        .cursor_pointer()
        .child(Icon::new(spec.icon).size(IconSize::Chrome).color(ink));
    // ## m3b-t25-results: a result tab's title is capped and ellipsized (the whole in its
    // tooltip); its pin glyph and untyped note follow it.
    el = match spec.facts {
        None => el.child(
            Label::new(spec.title, TextSource::Fluent, t.type_ramp.body, ink)
                .id(ElementId::Name(spec.label_id)),
        ),
        Some(facts) => {
            let source = if facts.data_title {
                TextSource::Data
            } else {
                TextSource::Fluent
            };
            let cap = if spec.short { SHORT_TITLE_PX } else { TITLE_PX };
            let el = el.child(
                div().min_w(px(0.0)).max_w(px(cap)).child(
                    Label::new(spec.title, source, t.type_ramp.body, ink)
                        .truncate(true)
                        .id(ElementId::Name(spec.label_id.clone())),
                ),
            );
            // Fix round 1 (m9): the repeat after the cut title, whole.
            match facts.repeat {
                Some(n) => el.child(
                    Label::new(
                        repeat_text(n, cx),
                        TextSource::Fluent,
                        t.type_ramp.body,
                        ink,
                    )
                    .id(ElementId::Name(format!("{}-repeat", spec.label_id).into())),
                ),
                None => el,
            }
        }
    };
    if let Some(facts) = spec.facts {
        if facts.pinned {
            let tip = tr(cx, ids::hv::RESULTS_TAB_PINNED, NoArgs);
            let pin = format!("{}-pin", spec.selector);
            el = el.child(
                div()
                    .id(ElementId::Name(pin.clone().into()))
                    .debug_selector(move || pin)
                    .flex_none()
                    .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                    .child(
                        Icon::new(IconName::Pin)
                            .size(IconSize::Chrome)
                            .color(c.text_2.to_hsla()),
                    ),
            );
        }
        if facts.untyped {
            let text = if spec.short {
                tr(cx, ids::hv::RESULTS_UNTYPED_SHORT, NoArgs)
            } else {
                tr(cx, ids::hv::RESULTS_UNTYPED, NoArgs)
            };
            let id = format!("{}-untyped", spec.selector);
            let tip = tr(cx, ids::hv::RESULTS_UNTYPED_TIP, NoArgs);
            el = el.child(
                div()
                    .id(ElementId::Name(format!("{id}-tip").into()))
                    .flex_none()
                    .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                    .child(
                        Label::new(
                            text,
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id(ElementId::Name(id.into())),
                    ),
            );
        }
    }
    if spec.active {
        el = el.bg(c.raised.to_hsla());
    } else {
        el = el.hover(|s| s.bg(c.hover.to_hsla()));
    }
    if let Some((env, suffix)) = spec.env {
        el = el.child(env_swatch(
            crate::connections::capsule::env_kind(Some(env)),
            &theme,
        ));
        if let (Some(suffix), Some(id)) = (suffix, spec.env_id) {
            el = el.child(
                Label::new(
                    suffix,
                    TextSource::Fluent,
                    t.type_ramp.body,
                    c.text_2.to_hsla(),
                )
                .id(ElementId::Name(id)),
            );
        }
    }
    if let Some(name) = spec.unread {
        el = el.child(
            div()
                .id("results-unread")
                .debug_selector(|| "results-unread".to_string())
                .declare(Role::Status, name, None)
                .size(px(6.0))
                .flex_none()
                .rounded_full()
                .bg(c.text_2.to_hsla()),
        );
    }
    // ## m3b-t25-results: where the tab is painted.
    if let Some(record) = spec.on_bounds {
        el = el.relative().child(
            canvas(move |b, _, _| record(b), |_, _, _, _| {})
                .absolute()
                .size_full(),
        );
    }
    // ## m3b-t25-results: a right press opens a result tab's menu.
    if let (Some(on_menu), ActiveResult::Tab(ix)) = (spec.on_menu, target) {
        el = el.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            cx.stop_propagation();
            let at = event.position;
            let on_menu = on_menu.clone();
            window.defer(cx, move |window, cx| on_menu(ix, at, window, cx));
        });
    }
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_activate(target, window, cx))
        .into_any_element()
}

// Fix round 1 (m9)
/// A repeated title's `(k)`.
pub fn repeat_text(n: u64, cx: &App) -> SharedString {
    tr(
        cx,
        ids::hv::RESULTS_TAB_REPEAT,
        ids::hv::HvResultsTabRepeat { n: Count(n) },
    )
}

/// What a toolbar button does.
pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// A ghost toolbar button `button-<id>` with `icon`; it moves into the overflow menu last.
fn tool_button(
    id: &'static str,
    icon: IconName,
    msg: hacivat_l10n::MsgRef<NoArgs>,
    priority: u8,
    action: Action,
    cx: &App,
) -> ToolbarItem {
    tool_button_kind(id, icon, msg, priority, action, ButtonKind::Ghost, cx)
}

// ## demofix-1 (PO demo F8)
/// [`tool_button`] drawn as `kind`.
fn tool_button_kind(
    id: &'static str,
    icon: IconName,
    msg: hacivat_l10n::MsgRef<NoArgs>,
    priority: u8,
    action: Action,
    kind: ButtonKind,
    cx: &App,
) -> ToolbarItem {
    let text: SharedString = tr(cx, msg, NoArgs);
    let label: TextFn = {
        let text = text.clone();
        Rc::new(move |_| text.clone())
    };
    let choose = action.clone();
    ToolbarItem::new(
        priority,
        Rc::new(move |_, _| {
            let action = action.clone();
            Button::new(id, text.clone())
                .kind(kind)
                .icon(icon)
                .on_raised(true)
                .on_click(move |_, window, cx| action(window, cx))
                .into_any_element()
        }),
        label,
    )
    .on_choose(move |window, cx| choose(window, cx))
}

// ## m3c-t41-chrome (named exception; PO M3c-18): the narrow result toolbar's give-way order.
// The view switch and the record toggle (Task 38's right-group items) take their compact
// forms first, at [`VIEW_ITEMS_PRIORITY`]; Count rows (priority 1) moves into ⋯ before Copy;
// Copy gives way last.
/// Copy's priority: the last of the result toolbar's items to give way.
pub const COPY_PRIORITY: u8 = 200;
/// The priority the view switch and the record toggle carry, so they take their compact forms
/// before Copy moves (lower than [`COPY_PRIORITY`]).
pub const VIEW_ITEMS_PRIORITY: u8 = 100;

/// The toolbar of a result tab: Copy, and `rows` (the rows and their time) on the right.
/// ## m3b-t25-results: `extra` (Count rows) follows Copy.
pub fn result_toolbar(
    rows: SharedString,
    copy: Action,
    extra: Vec<ToolbarItem>,
    cx: &App,
) -> AnyElement {
    result_toolbar_with(rows, copy, extra, Vec::new(), cx)
}

// ## m3c-t38 (named exception): the right group's items after rows · timing (the view switch
// and the record toggle).
/// [`result_toolbar`] with `trailing` after rows · timing.
pub fn result_toolbar_with(
    rows: SharedString,
    copy: Action,
    extra: Vec<ToolbarItem>,
    trailing: Vec<ToolbarItem>,
    cx: &App,
) -> AnyElement {
    let label: TextFn = {
        let rows = rows.clone();
        Rc::new(move |_| rows.clone())
    };
    let mut bar = Toolbar::new("results-toolbar").item(tool_button(
        "results-copy",
        IconName::Copy,
        ids::hv::RESULTS_COPY,
        // ## m3c-t41-chrome (PO M3c-18)
        COPY_PRIORITY,
        copy,
        cx,
    ));
    for item in extra {
        bar = bar.item(item);
    }
    let bar = bar.trailing(ToolbarItem::pinned(
        Rc::new(move |_, cx| {
            let t = Theme::get(cx).tokens;
            Label::new(
                rows.clone(),
                TextSource::Fluent,
                t.type_ramp.mono_kbd,
                t.colors.text_2.to_hsla(),
            )
            .id("results-rows-timing")
            .into_any_element()
        }),
        label,
    ));
    // ## m3c-t38
    trailing
        .into_iter()
        .fold(bar, |bar, item| bar.trailing(item))
        .into_any_element()
}

/// The Output tab's toolbar: Clear, Copy, Scroll to end.
// ## demofix-1 (PO demo F8): `paused` (the user scrolled the log away from its newest line
// while lines arrive) draws "Scroll to end" as a Default button, outlined and in ink, so the
// way back to following is the toolbar's most visible item; following, it is Ghost.
/// The kind of the Output toolbar's "Scroll to end" while the log is `paused` or following.
pub fn log_end_kind(paused: bool) -> ButtonKind {
    if paused {
        ButtonKind::Default
    } else {
        ButtonKind::Ghost
    }
}

pub fn output_toolbar(
    clear: Action,
    copy: Action,
    end: Action,
    paused: bool,
    cx: &App,
) -> AnyElement {
    Toolbar::new("results-output-toolbar")
        .item(tool_button(
            "results-log-clear",
            IconName::Eraser,
            ids::hv::RESULTS_LOG_CLEAR,
            1,
            clear,
            cx,
        ))
        .item(tool_button(
            "results-log-copy",
            IconName::Copy,
            ids::hv::RESULTS_COPY,
            3,
            copy,
            cx,
        ))
        .item(tool_button_kind(
            "results-log-end",
            IconName::ArrowDownToLine,
            ids::hv::RESULTS_LOG_END,
            2,
            end,
            log_end_kind(paused),
            cx,
        ))
        .into_any_element()
}

/// The info row of a result with more than [`WIDE_RESULT_COLUMNS`] columns.
pub fn columns_hint(columns: usize, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let text = tr(
        cx,
        ids::hv::GRID_TOO_MANY_COLUMNS,
        ids::hv::HvGridTooManyColumns {
            n: Count(columns as u64),
        },
    );
    div()
        .debug_selector(|| "results-columns-hint".to_string())
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .bg(c.raised.to_hsla())
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .child(
            div().pt(px(2.0)).child(
                Icon::new(IconName::Info)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            ),
        )
        .child(
            div().flex_1().min_w(px(0.0)).child(
                paragraph(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .id("results-columns-hint"),
            ),
        )
        .into_any_element()
}

// ## m3b-t25-results
/// The info row of a statement that returned more result sets than a run keeps (R-M3-66):
/// `kept` shown, `skipped` read and not kept.
pub fn set_limit_row(kept: u32, skipped: u32, cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    let c = t.colors;
    let text = tr(
        cx,
        ids::hv::RESULTS_SET_LIMIT,
        ids::hv::HvResultsSetLimit {
            kept: Count(u64::from(kept)),
            skipped: Count(u64::from(skipped)),
        },
    );
    div()
        .debug_selector(|| "results-set-limit".to_string())
        .flex()
        .flex_row()
        .items_start()
        .gap(px(8.0))
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .bg(c.raised.to_hsla())
        .border_t_1()
        .border_color(c.seam.to_hsla())
        .child(
            div().pt(px(2.0)).child(
                Icon::new(IconName::Layers)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            ),
        )
        .child(
            div().flex_1().min_w(px(0.0)).child(
                paragraph(
                    text,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .id("results-set-limit"),
            ),
        )
        .into_any_element()
}

/// The Output tab's empty state, centred in `text-3` on the panel.
pub fn log_empty(cx: &App) -> AnyElement {
    let t = Theme::get(cx).tokens;
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .px(px(24.0))
        .child(
            paragraph(
                tr(cx, ids::hv::RESULTS_LOG_EMPTY, NoArgs),
                TextSource::Fluent,
                t.type_ramp.body_sm,
                t.colors.text_3.to_hsla(),
            )
            .id("results-log-empty")
            .centered(),
        )
        .into_any_element()
}
