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

use std::rc::Rc;

use hacivat_l10n::{Count, NoArgs, Text, ids};
use hacivat_proto::EnvTag;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::capsule::env_swatch;
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui::toolbar::{Toolbar, ToolbarItem};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
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
}

/// What a click on a tab does.
pub type OnActivate = Rc<dyn Fn(ActiveResult, &mut Window, &mut App)>;

/// The result tab strip.
pub fn tab_strip(model: &TabsModel, on_activate: OnActivate, cx: &App) -> AnyElement {
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
        },
        on_activate.clone(),
        cx,
    ));
    for (ix, title) in model.results.iter().enumerate() {
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
            },
            on_activate.clone(),
            cx,
        ));
    }
    strip.into_any_element()
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
        .declare(Role::Tab, spec.title.clone(), Some(spec.active))
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
        .child(Icon::new(spec.icon).size(IconSize::Chrome).color(ink))
        .child(
            Label::new(spec.title, TextSource::Fluent, t.type_ramp.body, ink)
                .id(ElementId::Name(spec.label_id)),
        );
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
    el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| on_activate(target, window, cx))
        .into_any_element()
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
                .kind(ButtonKind::Ghost)
                .icon(icon)
                .on_raised(true)
                .on_click(move |_, window, cx| action(window, cx))
                .into_any_element()
        }),
        label,
    )
    .on_choose(move |window, cx| choose(window, cx))
}

/// The toolbar of a result tab: Copy, and `rows` (the rows and their time) on the right.
pub fn result_toolbar(rows: SharedString, copy: Action, cx: &App) -> AnyElement {
    let label: TextFn = {
        let rows = rows.clone();
        Rc::new(move |_| rows.clone())
    };
    Toolbar::new("results-toolbar")
        .item(tool_button(
            "results-copy",
            IconName::Copy,
            ids::hv::RESULTS_COPY,
            2,
            copy,
            cx,
        ))
        .trailing(ToolbarItem::pinned(
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
        ))
        .into_any_element()
}

/// The Output tab's toolbar: Clear, Copy, Scroll to end.
pub fn output_toolbar(clear: Action, copy: Action, end: Action, cx: &App) -> AnyElement {
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
        .item(tool_button(
            "results-log-end",
            IconName::ArrowDownToLine,
            ids::hv::RESULTS_LOG_END,
            2,
            end,
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
