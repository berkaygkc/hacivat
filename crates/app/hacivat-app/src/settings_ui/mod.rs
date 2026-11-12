//! The settings tab (Task 17): a left navigation (General, Appearance, Keyboard), a search field
//! and one row per setting. Every row is a `Segmented`; a choice applies at once (except text
//! rendering, which waits for a restart, R11) and is saved to `settings.toml` through
//! [`update_settings`](crate::shell::update_settings), the ordered background writer.

pub mod rows;
pub mod search;

use std::rc::Rc;

use hacivat_l10n::{ExactDecimal, L10n, NoArgs, WhenValue, ids};
use hacivat_settings::{Settings, TextRenderingPref};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::text::{Label, TextFn, TextSource};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, ClickEvent, Context, Div, Entity, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div, px,
};
use hacivat_ui_core::platform::TEXT_RENDERING_IS_LIVE;
use hacivat_ui_core::theme::{Theme, focus_outline};

use crate::l10n_global::{L10nGlobal, text_fn, tr};
use crate::shell::workspace::Workspace;
use crate::shell::{AppState, TabContent, TabContentRegistry};

use rows::{Choice, ChoiceLabel, REGIONAL_FORMAT, TEXT_RENDERING, current_value, is_selected};
use search::search_opts;

pub use rows::{Section, SettingRow, row_section, settings_rows};
pub use search::{SearchRow, filter_rows};

/// The tab kind the settings tab registers under.
pub const SETTINGS_KIND: &str = "settings";
/// The key context while the settings tab is active.
pub const SETTINGS_CONTEXT: &str = "Settings";

// DESIGN.md describes no settings page, so these three are this page's own sizes (no token
// carries them). Rows are form rows built on the 26 px field/segmented height and the spacing
// tokens, not the 22 px key/value rows of the connection panel and inspector.
/// The navigation column (a little narrower than the 276 px explorer).
const NAV_WIDTH_PX: f32 = 176.0;
/// The column the setting titles sit in.
const TITLE_COLUMN_PX: f32 = 200.0;
/// The widest the search field gets.
const SEARCH_WIDTH_PX: f32 = 320.0;

/// The text rendering the running process started with: a saved value that differs needs a
/// restart (the note and the button show).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartedWith(TextRenderingPref);

impl hacivat_ui_core::gpui_surface::Global for StartedWith {}

/// Registers the `"settings"` tab kind (title `hv-settings-title`, context `Settings`) and
/// records the text rendering this process started with (F3).
pub fn install(cx: &mut App) {
    let started = cx
        .try_global::<AppState>()
        .map_or_else(TextRenderingPref::default, |s| s.settings.ui.text_rendering);
    cx.set_global(StartedWith(started));
    cx.default_global::<TabContentRegistry>().register(
        SETTINGS_KIND,
        TabContent {
            title: text_fn(ids::hv::SETTINGS_TITLE),
            build: Rc::new(|window: &mut Window, cx: &mut App| -> AnyView {
                cx.new(|cx| SettingsPage::new(window, cx)).into()
            }),
            context: SETTINGS_CONTEXT,
        },
    );
}

/// The settings tab's content.
pub struct SettingsPage {
    search: Entity<TextInput>,
    query: String,
    section: Section,
    nav_focus: Vec<FocusHandle>,
    /// The searchable titles, rebuilt only when the localization generation changes.
    search_rows: Option<(u64, Vec<SearchRow>)>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx, text_fn(ids::hv::SETTINGS_SEARCH)));
        let subscription = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed) {
                this.query = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        Self {
            search,
            query: String::new(),
            section: Section::General,
            nav_focus: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            search_rows: None,
            _subscriptions: vec![subscription],
        }
    }

    /// The keys of the rows to show: the selected section's, or every match of the search.
    fn visible_keys(&mut self, cx: &App) -> Vec<&'static str> {
        let all = settings_rows();
        let Some(global) = cx.try_global::<L10nGlobal>() else {
            return all.iter().map(|r| r.key).collect();
        };
        if self.query.trim().is_empty() {
            return all
                .iter()
                .filter(|r| row_section(r.key) == Some(self.section))
                .map(|r| r.key)
                .collect();
        }
        let l10n: &L10n = &global.l10n;
        let generation = l10n.generation();
        if self.search_rows.as_ref().map(|(g, _)| *g) != Some(generation) {
            let rows = all.iter().map(|r| SearchRow::of(r, l10n)).collect();
            self.search_rows = Some((generation, rows));
        }
        let rows = self.search_rows.as_ref().map_or(&[][..], |(_, r)| &r[..]);
        filter_rows(&self.query, rows, &search_opts(l10n))
    }

    fn nav(&self, theme: &Theme, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let mut nav = div()
            .flex()
            .flex_col()
            .flex_none()
            .w(px(NAV_WIDTH_PX))
            .h_full()
            .pt(px(t.spacing.md))
            .bg(c.side.to_hsla())
            .border_r_1()
            .border_color(c.seam.to_hsla());
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let selected = self.section == section && self.query.trim().is_empty();
            let handle = self.nav_focus[i].clone();
            let focused = handle.is_focused(window) && window.last_input_was_keyboard();
            let name = tr(cx, section.title(), NoArgs);
            let ink = if selected { c.text } else { c.text_2 };
            let mut item = div()
                .id(("settings-nav", i))
                .debug_selector(move || format!("settings-nav-{i}"))
                .declare(Role::Button, name.clone(), Some(selected))
                .flex()
                .flex_row()
                .items_center()
                .h(px(t.spacing.tree_row_height))
                .pl(px(t.spacing.xl))
                .pr(px(t.spacing.lg))
                .cursor_pointer()
                .track_focus(&handle)
                .tab_stop(true)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.section = section;
                    this.query.clear();
                    this.search.update(cx, |input, cx| {
                        input.set_value("");
                        cx.notify();
                    });
                    cx.notify();
                }))
                .child(
                    Label::new(name, TextSource::Fluent, t.type_ramp.body, ink.to_hsla())
                        .id(("settings-nav-label", i)),
                );
            if selected {
                // The tree's selection look: blue tint with a 1 px blue left edge.
                item = item
                    .bg(c.blue_tint.to_hsla())
                    .border_l_1()
                    .border_color(c.blue.to_hsla());
            } else {
                item = item.hover(|s| s.bg(c.hover.to_hsla()));
            }
            nav = nav.child(focus_outline(item, theme, focused));
        }
        nav
    }

    /// One row: its title and the control, with the extras some rows show under the control.
    fn row(
        &self,
        row: &SettingRow,
        settings: &Settings,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = theme.tokens;
        let c = &t.colors;
        let key = row.key;
        let title = tr(cx, row.title, NoArgs);
        let current = current_value(settings, key);
        let mut group = Segmented::new(key).mono(false);
        for choice in rows::choices(cx, key) {
            let value = choice.value.clone();
            let (content, tooltip) = segment_content(cx, &choice);
            group = group.item(SegmentItem {
                content,
                pressed: is_selected(key, &choice, &current),
                tooltip,
                a11y_name: None,
                on_click: Rc::new(move |_, _, cx| {
                    rows::apply_choice(cx, key, &value);
                    cx.refresh_windows();
                }),
            });
        }
        let mut control = div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(t.spacing.sm))
            .child(group.into_any_element());
        match key {
            REGIONAL_FORMAT => {
                control = control.child(self.regional_preview(theme, cx));
            }
            TEXT_RENDERING => {
                if let Some(note) = self.restart_note(settings, theme, cx) {
                    control = control.child(note);
                }
            }
            _ => {}
        }
        div()
            .debug_selector(move || format!("settings-row-{key}"))
            .flex()
            .flex_row()
            .items_start()
            .w_full()
            .py(px(t.spacing.sm))
            .pl(px(t.spacing.xl))
            .pr(px(t.spacing.xl))
            .gap(px(t.spacing.xl))
            .border_b_1()
            .border_color(c.seam_soft.to_hsla())
            .child(
                div()
                    .flex_none()
                    .w(px(TITLE_COLUMN_PX))
                    .h(px(t.spacing.field_height))
                    .flex()
                    .items_center()
                    .child(
                        Label::new(
                            title,
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id(SharedString::from(format!("settings-row-title-{key}"))),
                    ),
            )
            .child(control)
    }

    /// The sample of the regional format in the format locale now in effect.
    fn regional_preview(&self, theme: &Theme, cx: &App) -> AnyElement {
        let t = theme.tokens;
        let text = regional_preview_text(cx);
        Label::new(
            text,
            TextSource::Fluent,
            t.type_ramp.mono_data,
            t.colors.text_2.to_hsla(),
        )
        .id("settings-regional-preview")
        .into_any_element()
    }

    /// "Applies after restart" and the restart button, once the saved text rendering differs
    /// from the one this process started with.
    fn restart_note(
        &self,
        settings: &Settings,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let started = cx.try_global::<StartedWith>().map(|s| s.0)?;
        if TEXT_RENDERING_IS_LIVE || settings.ui.text_rendering == started {
            return None;
        }
        let t = theme.tokens;
        let note = Label::new(
            tr(cx, ids::hv::SETTINGS_RESTART_NOTE, NoArgs),
            TextSource::Fluent,
            t.type_ramp.body_sm,
            t.colors.text_2.to_hsla(),
        )
        .id("settings-restart-note");
        let button = Button::new(
            "settings-restart-now",
            tr(cx, ids::hv::SETTINGS_RESTART_NOW, NoArgs),
        )
        .on_click(|_, window, cx| open_restart_dialog(window, cx));
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(t.spacing.lg))
                .child(note)
                .child(
                    div()
                        .debug_selector(|| "settings-restart-now".to_string())
                        .child(button),
                )
                .into_any_element(),
        )
    }
}

impl Focusable for SettingsPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let settings = cx
            .try_global::<AppState>()
            .map(|s| s.settings.clone())
            .unwrap_or_default();
        let keys = self.visible_keys(cx);
        let all = settings_rows();
        let mut list = div()
            .id("settings-rows")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            // m2a-t21-shots: the frame checks treat this clip as a scroll viewport.
            .child(hacivat_ui::text::scroll_viewport());
        if self.query.trim().is_empty() {
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(t.spacing.panel_header_height))
                    .pl(px(t.spacing.xl))
                    .child(
                        Label::new(
                            tr(cx, self.section.heading(), NoArgs),
                            TextSource::Fluent,
                            t.type_ramp.panel_title,
                            c.text_2.to_hsla(),
                        )
                        .id("settings-section-title"),
                    ),
            );
        }
        for key in &keys {
            if let Some(row) = all.iter().find(|r| r.key == *key) {
                list = list.child(self.row(row, &settings, &theme, cx));
            }
        }
        if keys.is_empty() {
            list = list.child(
                div().pl(px(t.spacing.xl)).pt(px(t.spacing.lg)).child(
                    Label::new(
                        tr(cx, ids::hv::SETTINGS_NO_RESULTS, NoArgs),
                        TextSource::Fluent,
                        t.type_ramp.body,
                        c.text_3.to_hsla(),
                    )
                    .id("settings-no-results"),
                ),
            );
        }
        let search = div()
            .flex_none()
            .px(px(t.spacing.xl))
            .py(px(t.spacing.md))
            .border_b_1()
            .border_color(c.seam.to_hsla())
            .child(
                div()
                    .max_w(px(SEARCH_WIDTH_PX))
                    .debug_selector(|| "settings-search".to_string())
                    .child(self.search.clone()),
            );
        div()
            .debug_selector(|| "settings-page".to_string())
            .flex()
            .flex_row()
            .size_full()
            // Tab content is a pane: `panel` (white in light), like the editor and results; the
            // nav keeps `side`. `ground` is for the window ground and the bars (DESIGN.md).
            .bg(c.panel.to_hsla())
            .child(self.nav(&theme, window, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(search)
                    .child(list),
            )
    }
}

/// How a choice shows as a segment, and its tooltip.
fn segment_content(cx: &App, choice: &Choice) -> (SegmentContent, SharedString) {
    match &choice.label {
        ChoiceLabel::Msg(id) => {
            let text = tr(cx, *id, NoArgs);
            (SegmentContent::Label(text.clone()), text)
        }
        ChoiceLabel::Data(text) => (SegmentContent::Verbatim(text.clone()), text.clone()),
    }
}

/// The regional-format sample: `1234.5` and 2026-11-11 14:05 (a wall-clock value, so the time
/// zone never shifts it, F30), rendered by the installed localization.
pub fn regional_preview_text(cx: &App) -> SharedString {
    tr(
        cx,
        ids::hv::SETTINGS_REGIONAL_PREVIEW,
        ids::hv::HvSettingsRegionalPreview {
            number: ExactDecimal("1234.5".to_string()),
            when: WhenValue::Local {
                year: 2026,
                month: 11,
                day: 11,
                hour: 14,
                minute: 5,
                second: 0,
            },
        },
    )
}

/// Asks to restart, then restarts: the pending settings and layout writes are flushed first,
/// because `App::restart` ends the process from inside an update, where GPUI's quit handlers
/// (`on_app_quit`) cannot take the app (macOS ignores their refusal and terminates).
fn open_restart_dialog(window: &mut Window, cx: &mut App) {
    let Some(workspace) = window.root::<Workspace>().flatten() else {
        return;
    };
    let button = |label: TextFn, kind| DialogButton {
        label,
        kind,
        accesskey: None,
        english_accesskey: None,
    };
    let spec = DialogSpec {
        title: text_fn(ids::hv::SETTINGS_RESTART_TITLE),
        body: text_fn(ids::hv::SETTINGS_RESTART_BODY),
        buttons: vec![
            button(text_fn(ids::hv::SETTINGS_RESTART_NOW), ButtonKind::Primary),
            button(
                text_fn(ids::hv::SETTINGS_RESTART_LATER),
                ButtonKind::Default,
            ),
        ],
        cancel: 1,
        primary: 0,
        size: DialogSize::Standard,
    };
    let dialog = workspace.update(cx, |ws, cx| ws.open_dialog(spec, window, cx));
    // The workspace drops its handle to the dialog when it removes the layer, on the same event;
    // `App::subscribe` only calls a handler whose entity can still be upgraded, so this closure
    // holds the dialog until that event came (then the dialog and the subscription go).
    let mut keep_alive = Some(dialog.clone());
    cx.subscribe(&dialog, move |_, event: &DialogEvent, cx| {
        keep_alive.take();
        if matches!(event, DialogEvent::Pressed(0)) {
            // After this update: the platform may quit synchronously.
            cx.defer(|cx| {
                crate::shell::flush_pending_writes(cx);
                cx.restart();
            });
        }
    })
    .detach();
}
