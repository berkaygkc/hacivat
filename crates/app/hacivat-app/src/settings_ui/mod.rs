//! The settings tab (Task 17): a left navigation (General, Appearance, Keyboard; since M3b
//! Task 42 also Editor, Execution, Results, History), a search field and one row per setting.
//! A row is a `Segmented` (the largest-value row is a number field with a unit, see
//! [`value_limit`]); a choice applies at once (except text rendering, which waits for a restart,
//! R11) and is saved to `settings.toml` through [`update_settings`](crate::shell::update_settings),
//! the ordered background writer.

pub mod rows;
pub mod search;
// ## m3b-t42-settings
pub mod value_limit;
// ## m4-t01b-seams: Settings › Metadata (M4 Task 27).
pub mod metadata;

use std::rc::Rc;

use hacivat_l10n::{ExactDecimal, L10n, NoArgs, WhenValue, ids};
use hacivat_settings::{Settings, TextRenderingPref};
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::dialog::{DialogButton, DialogEvent, DialogSize, DialogSpec};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::Declare as _;
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::select::{Select, SelectEvent, SelectOption};
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, AnyView, App, AppContext as _, ClickEvent, Context, Div, Entity, FocusHandle,
    Focusable, Global, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window,
    div, px,
};
use hacivat_ui_core::platform::TEXT_RENDERING_IS_LIVE;
use hacivat_ui_core::theme::{Theme, focus_outline};

use crate::l10n_global::{L10nGlobal, text_fn, tr};
use crate::shell::workspace::Workspace;
use crate::shell::{AppState, TabContent, TabContentRegistry, update_settings};

use rows::{
    Choice, ChoiceLabel, MAX_VALUE_BYTES, REGIONAL_FORMAT, TEXT_RENDERING, current_value,
    is_selected,
};
use search::search_opts;
use value_limit::{Amount, Clamped, Fit, SizeUnit, amount_text, display_unit};

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
// ## m3b-t42-settings: NOTES surface 25's own sizes.
/// The largest-value number field.
const VALUE_FIELD_PX: f32 = 88.0;
/// The KB / MB select beside it.
const UNIT_SELECT_PX: f32 = 76.0;

/// The text rendering the running process started with: a saved value that differs needs a
/// restart (the note and the button show).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartedWith(TextRenderingPref);

impl hacivat_ui_core::gpui_surface::Global for StartedWith {}

// ## m3b-t42-settings
/// The settings page while a settings tab shows it (a workspace has one settings tab).
struct OpenPage(WeakEntity<SettingsPage>);

impl Global for OpenPage {}

/// The open settings page, if a settings tab was built and still lives.
pub fn open_page(cx: &App) -> Option<Entity<SettingsPage>> {
    cx.try_global::<OpenPage>()?.0.upgrade()
}

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
                let page = cx.new(|cx| SettingsPage::new(window, cx));
                cx.set_global(OpenPage(page.downgrade()));
                page.into()
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
    // ## m3b-t42-settings: the largest-value row.
    limit_input: Entity<TextInput>,
    limit_unit: Entity<Select>,
    /// The size the field shows and its unit: the saved size once the field is committed.
    limit_shown: Option<(u32, SizeUnit)>,
    /// The text the field was given for `limit_shown` (anything else was typed).
    limit_shown_text: String,
    /// The note after a typed size was set to a bound.
    limit_note: Option<Clamped>,
    // ## m4-t27-refresh-ui (named lines): the Metadata section's own controls.
    metadata: metadata::MetadataControls,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextInput::new(cx, text_fn(ids::hv::SETTINGS_SEARCH)));
        let subscription = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Changed) {
                this.query = input.read(cx).value().to_string();
                cx.notify();
            }
        });
        // ## m3b-t42-settings
        let limit_input = cx.new(|cx| {
            let mut input = TextInput::new(cx, text_fn(ids::hv::SETTINGS_MAX_VALUE));
            input.set_mono(true);
            input
        });
        let options = SizeUnit::ALL
            .iter()
            .map(|unit| SelectOption {
                label: text_fn(unit.label()),
                hint: None,
                disabled: false,
            })
            .collect();
        let limit_unit = cx.new(|cx| {
            let mut select = Select::new(options, SizeUnit::Mb.index(), cx);
            select.set_label(text_fn(ids::hv::SETTINGS_MAX_VALUE));
            select
        });
        let limit_events = cx.subscribe(&limit_input, |this, _, event: &InputEvent, cx| {
            match event {
                InputEvent::Submitted => this.commit_limit(cx),
                // Escape (F15: `Cancelled` is Escape) puts the saved size back.
                InputEvent::Cancelled => this.restore_limit(cx),
                InputEvent::Changed | InputEvent::CompositionChanged(_) => {}
            }
        });
        let unit_events = cx.subscribe(&limit_unit, |this, _, event: &SelectEvent, cx| {
            let SelectEvent::Changed(index) = *event;
            if let Some(unit) = SizeUnit::ALL.get(index) {
                this.change_unit(*unit, cx);
            }
        });
        // Leaving the row (the field, the select and its menu) keeps what was typed, as Enter
        // does; moving from the field to the select waits: choosing a unit then saves the
        // typed number in the unit it was typed in, first (fix round 1, m2).
        let field_focus = limit_input.focus_handle(cx);
        let field_blur = cx.on_focus_out(&field_focus, window, |this, _, window, cx| {
            this.settle_limit(window, cx)
        });
        let unit_focus = limit_unit.focus_handle(cx);
        let unit_blur = cx.on_focus_out(&unit_focus, window, |this, _, window, cx| {
            this.settle_limit(window, cx)
        });
        // The menu closing without a choice (a press elsewhere) leaves the row too.
        let unit_closed = cx.observe_in(&limit_unit, window, |this, _, window, cx| {
            this.settle_limit(window, cx)
        });
        // ## m4-t27-refresh-ui (named lines)
        let metadata = metadata::MetadataControls::new(window, cx);
        Self {
            search,
            query: String::new(),
            section: Section::General,
            nav_focus: Section::ALL.iter().map(|_| cx.focus_handle()).collect(),
            search_rows: None,
            limit_input,
            limit_unit,
            limit_shown: None,
            limit_shown_text: String::new(),
            limit_note: None,
            // ## m4-t27-refresh-ui (named lines)
            metadata,
            _subscriptions: vec![
                subscription,
                limit_events,
                unit_events,
                field_blur,
                unit_blur,
                unit_closed,
            ],
        }
    }

    // ## m4-t27-refresh-ui (named lines; the M3c carry m3c.md §8.1, T43b fix m5, P-M4-18)
    /// Shows `section`, as a click on its navigation item does (the search is cleared).
    pub fn show_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if section == Section::Metadata {
            metadata::section_shown(cx);
        }
        self.section = section;
        self.query.clear();
        self.search.update(cx, |input, cx| {
            input.set_value("");
            cx.notify();
        });
        cx.notify();
    }

    // ## m3b-t42-settings
    /// The largest-value field's text.
    pub fn value_limit_text(&self, cx: &App) -> String {
        self.limit_input.read(cx).value().to_string()
    }

    /// The largest-value field's unit.
    pub fn value_limit_unit(&self) -> SizeUnit {
        self.limit_shown.map_or(SizeUnit::Mb, |(_, unit)| unit)
    }

    /// The saved largest value.
    fn saved_limit(cx: &App) -> u32 {
        cx.try_global::<AppState>()
            .map(|s| s.settings.results.max_value_bytes)
            .unwrap_or_default()
            .get()
    }

    /// Saves the field's number in the field's unit, set to the nearest bound when outside
    /// 64 KB–64 MB (the row then says so). An untouched field saves nothing; what is not a size
    /// puts the saved size back.
    fn commit_limit(&mut self, cx: &mut Context<Self>) {
        let saved = Self::saved_limit(cx);
        let unit = self
            .limit_shown
            .map_or_else(|| display_unit(saved, None), |(_, unit)| unit);
        if !self.limit_typed(cx) {
            return;
        }
        let text = self.limit_input.read(cx).value().to_string();
        let (decimal, group) = value_limit::separators(cx);
        let Some(amount) = value_limit::parse_amount(&text, decimal, group) else {
            self.restore_limit(cx);
            return;
        };
        let (limit, fit) = value_limit::fit(amount.bytes(unit));
        self.limit_note = (fit != Fit::Within).then_some(Clamped {
            typed: amount,
            unit,
            fit,
        });
        if limit.get() != saved {
            update_settings(cx, |s| s.results.max_value_bytes = limit);
        }
        let limit = limit.get();
        self.show_limit(limit, display_unit(limit, Some(unit)), cx);
        cx.notify();
    }

    // ## m3c-t42 fix round 1 (review m2, controller ruling)
    /// The select chose `unit`: a typed number is saved first, in the unit it was typed in;
    /// then the saved size shows converted to `unit` (64 MB as 65,536 KB). Nothing else is
    /// written.
    fn change_unit(&mut self, unit: SizeUnit, cx: &mut Context<Self>) {
        self.commit_limit(cx);
        self.show_limit(Self::saved_limit(cx), unit, cx);
        cx.notify();
    }

    /// Shows the size the field showed last (the saved one) again.
    fn restore_limit(&mut self, cx: &mut Context<Self>) {
        let saved = Self::saved_limit(cx);
        let (bytes, unit) = self
            .limit_shown
            .unwrap_or((saved, display_unit(saved, None)));
        self.show_limit(bytes, unit, cx);
        cx.notify();
    }

    /// Puts `bytes` in the field, exactly, in `unit`.
    fn show_limit(&mut self, bytes: u32, unit: SizeUnit, cx: &mut Context<Self>) {
        let text = amount_text(cx, Amount::of(bytes, unit));
        self.limit_input.update(cx, |input, cx| {
            if input.value() != text {
                input.set_value(text.clone());
                cx.notify();
            }
        });
        self.limit_unit.update(cx, |select, cx| {
            if select.selected() != unit.index() {
                select.set_selected(unit.index(), cx);
            }
        });
        self.limit_shown = Some((bytes, unit));
        self.limit_shown_text = text;
    }

    /// Whether the field holds something other than what it was given.
    fn limit_typed(&self, cx: &App) -> bool {
        self.limit_shown.is_some() && *self.limit_input.read(cx).value() != *self.limit_shown_text
    }

    /// Whether the focus is in the row: the field, the select or the select's menu.
    fn limit_row_focused(&self, window: &Window, cx: &App) -> bool {
        let select = self.limit_unit.read(cx);
        self.limit_input.focus_handle(cx).is_focused(window)
            || select.focus_handle(cx).is_focused(window)
            || select.is_open()
    }

    /// Saves what was typed once the focus has left the row.
    fn settle_limit(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.limit_row_focused(window, cx) {
            self.commit_limit(cx);
        }
    }

    /// Before a frame: the field follows the saved size and the regional format, unless it
    /// holds what the user typed (the focus leaving the row saves that, after this frame). A
    /// size saved elsewhere shows in its own unit and drops the clamp note.
    fn follow_saved_limit(&mut self, cx: &mut Context<Self>) {
        if self.limit_typed(cx) {
            return;
        }
        let saved = Self::saved_limit(cx);
        let unit = match self.limit_shown {
            Some((bytes, unit)) if bytes == saved => unit,
            Some(_) => {
                self.limit_note = None;
                display_unit(saved, None)
            }
            None => display_unit(saved, None),
        };
        let current = self.limit_shown == Some((saved, unit))
            && amount_text(cx, Amount::of(saved, unit)) == self.limit_shown_text;
        if !current {
            self.show_limit(saved, unit, cx);
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
                // ## m4-t27-refresh-ui (named lines, P-M4-18)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.show_section(section, cx);
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
        // ## m3b-t42-settings: the largest-value row is a field and a unit; every other row a
        // Segmented. While searching, a row names its section after the control.
        let mut line = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(t.spacing.md));
        if key == MAX_VALUE_BYTES {
            line = line.children(self.value_limit_controls());
        } else if let Some(controls) = self.metadata.control(key, cx) {
            // ## m4-t27-refresh-ui (named lines): the Metadata section's selects, fields and
            // button.
            line = line.children(controls);
        } else {
            line = line.child(segmented_row(key, settings, cx));
        }
        if !self.query.trim().is_empty()
            && let Some(section) = row_section(key)
        {
            line = line.child(
                Label::new(
                    tr(cx, section.title(), NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .id(SharedString::from(format!("settings-row-section-{key}"))),
            );
        }
        let (title_id, note_id) = rows::row_ids(key).unwrap_or(("", ""));
        let mut control = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(t.spacing.xs))
            .child(line);
        match key {
            REGIONAL_FORMAT => {
                control = control.child(self.regional_preview(theme, cx));
            }
            TEXT_RENDERING => {
                if let Some(note) = self.restart_note(settings, theme, cx) {
                    control = control.child(note);
                }
            }
            MAX_VALUE_BYTES => {
                if let Some(clamped) = self.limit_note {
                    control = control.child(clamp_note(clamped, theme, cx));
                }
            }
            // ## m4-t27-refresh-ui (named lines): the active console's connection and its
            // cache size.
            rows::CLEAR_CACHE => {
                if let Some(note) = self.metadata.clear_note(cx) {
                    control = control.child(
                        paragraph(
                            note,
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .id("settings-row-note-metadata.clear_cache"),
                    );
                }
            }
            _ => {}
        }
        if let Some(note) = rows::row_note(key) {
            control = control.child(
                paragraph(
                    tr(cx, note, NoArgs),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                )
                .id(note_id),
            );
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
                // ## m3b-t42-settings: a long title wraps in its column (NOTES surface 25).
                div()
                    .flex_none()
                    .w(px(TITLE_COLUMN_PX))
                    .min_h(px(t.spacing.field_height))
                    .flex()
                    .items_center()
                    .child(
                        paragraph(
                            title,
                            TextSource::Fluent,
                            t.type_ramp.body,
                            c.text_2.to_hsla(),
                        )
                        .id(title_id),
                    ),
            )
            .child(control)
    }

    // ## m3b-t42-settings
    /// The largest-value row's field and unit select.
    fn value_limit_controls(&self) -> [AnyElement; 2] {
        [
            div()
                .debug_selector(|| "settings-max-value-field".to_string())
                .flex_none()
                .w(px(VALUE_FIELD_PX))
                .child(self.limit_input.clone())
                .into_any_element(),
            div()
                .debug_selector(|| "settings-max-value-unit".to_string())
                .flex_none()
                .w(px(UNIT_SELECT_PX))
                .child(self.limit_unit.clone())
                .into_any_element(),
        ]
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

// ## m3b-t42-settings
/// Row `key`'s choices as one Segmented.
fn segmented_row(key: &'static str, settings: &Settings, cx: &mut App) -> AnyElement {
    let current = current_value(settings, key);
    let mut group = Segmented::new(key)
        .mono(rows::is_mono(key))
        // ## m3c-t42 fix round 1 (review I1)
        .arrows_move_focus_only(rows::is_guard(key));
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
    group.into_any_element()
}

/// The clamp note under the largest-value field: the info glyph and its sentence.
fn clamp_note(clamped: Clamped, theme: &Theme, cx: &App) -> AnyElement {
    let t = theme.tokens;
    let ink = t.colors.text_2.to_hsla();
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(t.spacing.sm))
        .child(
            div()
                .flex_none()
                .pt(px(2.0))
                .child(Icon::new(IconName::Info).size(IconSize::Chrome).color(ink)),
        )
        .child(
            div().flex_1().min_w_0().child(
                paragraph(
                    value_limit::clamped_text(cx, clamped),
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    ink,
                )
                .id("settings-max-value-clamped"),
            ),
        )
        .into_any_element()
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
        // ## m3b-t42-settings
        self.follow_saved_limit(cx);
        // ## m4-t27-refresh-ui (named lines)
        self.metadata.follow(&settings, cx);
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
            // ## m3a-t33-exec-ui (named exception, T33 review C3): through the quit guard (an
            // open user transaction prompts first); the restart itself is deferred to after
            // this update and flushes the pending writes.
            cx.defer(|cx| {
                crate::commands::request_restart(cx);
            });
        }
    })
    .detach();
}

// ## m3c-t43b (named exception: Task 43's shot seams)
/// Shot fixtures for the settings rows (`design/m3b-safe` surface 25): what a click on a
/// navigation item, typing in the search field and typing in the largest-value field do,
/// without the pointer (the shots harness has no element bounds to aim at).
#[cfg(feature = "test-support")]
pub mod fixtures {
    use super::*;

    /// Shows `section`, as a click on its navigation item does (the search is cleared).
    pub fn show_section(section: Section, cx: &mut App) {
        // ## m4-t27-refresh-ui (named lines, P-M4-18): the navigation's own handler.
        if let Some(page) = open_page(cx) {
            page.update(cx, |p, cx| p.show_section(section, cx));
        }
    }

    /// Puts `query` in the search field, as typing it does.
    pub fn search(query: &str, cx: &mut App) {
        if let Some(page) = open_page(cx) {
            let query = query.to_string();
            page.update(cx, |p, cx| {
                p.search.update(cx, |input, cx| {
                    input.set_value(query.clone());
                    cx.notify();
                });
                p.query = query;
                cx.notify();
            });
        }
    }

    /// Replaces the largest-value field's text with `text` and presses Enter.
    pub fn enter_value_limit(text: &str, cx: &mut App) {
        if let Some(page) = open_page(cx) {
            let text = text.to_string();
            page.update(cx, |p, cx| {
                p.limit_input.update(cx, |input, cx| {
                    input.set_value(text);
                    cx.notify();
                });
                p.commit_limit(cx);
            });
        }
    }

    /// The section shown and the search query.
    pub fn shown(cx: &App) -> Option<(Section, String)> {
        let page = open_page(cx)?;
        let p = page.read(cx);
        Some((p.section, p.query.clone()))
    }

    /// Whether the largest-value row shows its clamp note.
    pub fn clamped(cx: &App) -> bool {
        open_page(cx).is_some_and(|p| p.read(cx).limit_note.is_some())
    }
}
