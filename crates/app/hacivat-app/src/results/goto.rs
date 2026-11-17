//! Go to row (HV-GRID-14; M3c Task 37): the go-to field over `hacivat_ui_grid::parse_goto`,
//! fetching past the loaded rows with a confirmation above a million.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). STUB: opens nothing (`GridEvent::GotoRequested` is
//! not answered yet). Task 37 fills it.
//!
//! ## m3c-t37
//! - **The popover** (NOTES surface 20, M3c-27): 300 px under the result toolbar's left edge,
//!   over the grid (never over the toolbar, the cursor strip or the status bar): the title, a
//!   26 px mono field ("row or row:column"), one line under it: the hint, the live target
//!   ("Row 1,000, column city"), or an ink error with the field border `text-2` ("There's no
//!   row 40: the result has 12 rows.", "No column is named ilce.", "Column meta is hidden." with
//!   Show it and go), then the keys ("↵ Go · esc Close").
//! - **Rows are view rows, 1-based,** in the format locale's digits and grouping (`1.000` is
//!   row 1000 in Turkish; the separators are read from how the result's display formats a
//!   number); a column by position, name, ASCII case-insensitive name, then the tolerant key
//!   (`parse_goto`). Enter goes (`GridView::go_to`: the cell takes the focus).
//! - **Past the fetched rows** of an open cursor it fetches, in steps of 100,000 rows, so it
//!   can stop between them; when the row arrives the grid goes to it. With more than 1,000,000
//!   rows to fetch it asks first, in place: Cancel (Default) and **Fetch and go** (Primary,
//!   last, focused; M3c-42). While it fetches: the static amber marker, "Fetching to row N: k of
//!   m rows" and **Stop fetching** (focused), which ends the fetching with the cursor open and
//!   what was fetched kept. Escape hides the popover; the fetching goes on (the strip shows it)
//!   and the grid still goes to the row when it arrives. A memory pause or the end of the
//!   result stops it (past the end names the last row).

use std::collections::HashMap;
use std::rc::{Rc, Weak};

use hacivat_l10n::ArgValue;
use hacivat_results::Completeness;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::{Icon, IconName, IconSize};
use hacivat_ui::input::{InputEvent, TextInput};
use hacivat_ui::popover::A11yNode;
use hacivat_ui::step_trail::run_marker;
use hacivat_ui::text::{Label, TextFn, TextSource, paragraph};
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    Styled as _, Subscription, WeakEntity, Window, div, px,
};
use hacivat_ui_core::keymap::{KeymapPlatform, display_chord, parse_chord};
use hacivat_ui_core::theme::{Theme, ThemeMode, focus_outline, popover_surface};
use hacivat_ui_grid::model::{GotoError, GotoPlace, GotoTarget, ViewMap, parse_goto};
use hacivat_ui_grid::{ColId, GridView};
use hacivat_values::ValueRef;
use hacivat_values::valfmt::{BinaryDisplay, ColumnFacts, Purpose, format_cell};

use crate::console::ConsoleId;
use crate::exec_ui::marks::message;
use crate::results::adapter::{GridAdapter, ResultDisplay};

/// Above this many rows to fetch, go to row asks first.
pub const ASK_ABOVE: u64 = 1_000_000;
/// Rows asked for per fetching step (it stops between steps).
pub const FETCH_STEP: u64 = 100_000;

/// What the popover is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GotoState {
    /// The field.
    Idle,
    /// More than [`ASK_ABOVE`] rows to fetch: Cancel or Fetch and go.
    Ask { row: u64, more: u64 },
    /// Fetching toward `row` (`more` rows were missing when it started).
    Fetching { row: u64, more: u64 },
}

/// The line under the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GotoLine {
    Hint(SharedString),
    Target(SharedString),
    Error(SharedString),
    /// A hidden column: its error and "Show it and go".
    Hidden(SharedString, ColId),
}

/// One console's go-to popover.
pub struct GotoPopover {
    console: ConsoleId,
    input: Entity<TextInput>,
    open: bool,
    /// The field (or the question's primary) takes the focus at the next frame.
    take_focus: bool,
    state: GotoState,
    /// The target being fetched toward, and the rows loaded when the fetching started.
    pending: Option<(GotoTarget, u64)>,
    /// The error a submit found (past the end).
    error: Option<SharedString>,
    grid: Option<WeakEntity<GridView>>,
    adapter: Option<Weak<GridAdapter>>,
    primary: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

#[derive(Default)]
struct Popovers(HashMap<ConsoleId, Entity<GotoPopover>>);

impl Global for Popovers {}

fn popover(console: ConsoleId, cx: &App) -> Option<Entity<GotoPopover>> {
    cx.try_global::<Popovers>()?.0.get(&console).cloned()
}

/// Opens the go-to field over console `console`'s active result (the tile calls it on
/// `GridEvent::GotoRequested`).
pub fn open(console: ConsoleId, cx: &mut App) {
    let p = match popover(console, cx) {
        Some(p) => p,
        None => {
            let p = cx.new(|cx| GotoPopover::new(console, cx));
            cx.default_global::<Popovers>().0.insert(console, p.clone());
            p
        }
    };
    let target = super::find::active_target(console, cx);
    p.update(cx, |p, cx| {
        if let Some((grid, adapter)) = target {
            let same = p
                .grid
                .as_ref()
                .is_some_and(|g| g.entity_id() == grid.entity_id());
            if !same {
                p.stop(cx);
            }
            p.grid = Some(grid.downgrade());
            p.adapter = Some(Rc::downgrade(&adapter));
        }
        p.open = true;
        p.take_focus = true;
        if !matches!(p.state, GotoState::Fetching { .. }) {
            p.state = GotoState::Idle;
            p.error = None;
        }
        p.input.update(cx, |i, cx| {
            i.select_all();
            cx.notify();
        });
        cx.notify();
    });
    notify_hosts(cx);
}

/// Forgets console `console`'s popover (its console closed).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if cx.has_global::<Popovers>() {
        cx.global_mut::<Popovers>().0.remove(&console);
    }
}

/// Whether console `console`'s popover is shown.
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    popover(console, cx).is_some_and(|p| p.read(cx).open)
}

/// The popover's state.
pub fn state(console: ConsoleId, cx: &App) -> Option<GotoState> {
    Some(popover(console, cx)?.read(cx).state)
}

/// The line under the field.
pub fn line(console: ConsoleId, cx: &App) -> Option<GotoLine> {
    Some(popover(console, cx)?.read(cx).line(cx))
}

/// Types `text` into the field.
pub fn set_text(console: ConsoleId, text: &str, cx: &mut App) {
    if let Some(p) = popover(console, cx) {
        let text = text.to_owned();
        p.update(cx, |p, cx| {
            p.error = None;
            p.input.update(cx, |i, cx| {
                i.set_value(text);
                cx.notify();
            });
            cx.notify();
        });
    }
}

/// Enter: goes, asks, or starts fetching.
pub fn submit(console: ConsoleId, cx: &mut App) {
    if let Some(p) = popover(console, cx) {
        p.update(cx, |p, cx| p.submit(cx));
    }
}

/// Fetch and go (the question's primary).
pub fn fetch_and_go(console: ConsoleId, cx: &mut App) {
    if let Some(p) = popover(console, cx) {
        p.update(cx, |p, cx| p.fetch_and_go(cx));
    }
}

/// Stop fetching: the cursor stays open with what was fetched.
pub fn stop(console: ConsoleId, cx: &mut App) {
    if let Some(p) = popover(console, cx) {
        p.update(cx, |p, cx| p.stop(cx));
    }
}

/// Closes (hides) the popover; a fetching goes on.
pub fn close(console: ConsoleId, cx: &mut App) {
    if let Some(p) = popover(console, cx) {
        p.update(cx, |p, cx| p.hide(cx));
    }
}

/// The tile shows `grid` over `adapter` for console `console`: the popover targets them, and a
/// fetching toward a row goes on (or lands).
pub(crate) fn sync(
    console: ConsoleId,
    grid: &Entity<GridView>,
    adapter: &Rc<GridAdapter>,
    cx: &mut App,
) {
    let Some(p) = popover(console, cx) else {
        return;
    };
    // Only a new grid or a fetching needs anything (and nothing repaints otherwise: this runs
    // in every frame of the tile).
    let (same, fetching) = {
        let p = p.read(cx);
        (
            p.grid
                .as_ref()
                .is_some_and(|g| g.entity_id() == grid.entity_id()),
            matches!(p.state, GotoState::Fetching { .. }),
        )
    };
    if same && !fetching {
        return;
    }
    let (grid, adapter) = (grid.clone(), adapter.clone());
    cx.defer(move |cx| {
        p.update(cx, |p, cx| {
            let same = p
                .grid
                .as_ref()
                .is_some_and(|g| g.entity_id() == grid.entity_id());
            if !same {
                p.stop(cx);
            }
            p.grid = Some(grid.downgrade());
            p.adapter = Some(Rc::downgrade(&adapter));
            p.drive(cx);
        });
    });
}

/// The popover, while shown (the tile lays it over its grid).
pub(crate) fn overlay(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let p = popover(console, cx).filter(|p| p.read(cx).open)?;
    Some(p.into_any_element())
}

fn notify_hosts(cx: &mut App) {
    for host in super::view::hosts(cx) {
        host.update(cx, |_, cx| cx.notify());
    }
}

/// The keymap platform of this build (the key hints' glyphs).
fn platform() -> KeymapPlatform {
    if cfg!(target_os = "macos") {
        KeymapPlatform::MacOs
    } else if cfg!(windows) {
        KeymapPlatform::Windows
    } else {
        KeymapPlatform::Linux
    }
}

/// The format locale's decimal and group separators, as `display` writes 1,234,567.5.
fn separators(display: &ResultDisplay) -> (char, char) {
    let logical = hacivat_values::LogicalType::Float { bits: 64 };
    let facts = ColumnFacts {
        logical: &logical,
        native_name: "",
        id_like: false,
        binary: BinaryDisplay::Detect,
        engine: display.engine(),
    };
    let shown = format_cell(
        ValueRef::F64(1_234_567.5),
        &facts,
        display.settings(),
        Purpose::Cell,
    );
    let marks: Vec<char> = shown
        .text
        .chars()
        .filter(|c| !c.is_ascii_digit() && *c != '-')
        .collect();
    match (marks.first(), marks.last()) {
        (Some(g), Some(d)) if marks.len() >= 2 => (*d, *g),
        (Some(d), _) => (*d, if *d == ',' { '.' } else { ',' }),
        _ => ('.', ','),
    }
}

impl GotoPopover {
    fn new(console: ConsoleId, cx: &mut Context<Self>) -> Self {
        let placeholder: TextFn = Rc::new(|cx: &App| message(cx, "hv-goto-placeholder", vec![]));
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx, placeholder);
            input.set_mono(true);
            input
        });
        let subscriptions = vec![cx.subscribe(&input, Self::on_input_event)];
        GotoPopover {
            console,
            input,
            open: false,
            take_focus: false,
            state: GotoState::Idle,
            pending: None,
            error: None,
            grid: None,
            adapter: None,
            primary: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    fn on_input_event(&mut self, _: Entity<TextInput>, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Changed => {
                self.error = None;
                if matches!(self.state, GotoState::Ask { .. }) {
                    self.state = GotoState::Idle;
                }
                cx.notify();
            }
            InputEvent::Submitted => self.submit(cx),
            InputEvent::Cancelled => self.hide(cx),
            InputEvent::CompositionChanged(_) => {}
        }
    }

    fn target(&self) -> Option<(Entity<GridView>, Rc<GridAdapter>)> {
        Some((
            self.grid.as_ref()?.upgrade()?,
            self.adapter.as_ref()?.upgrade()?,
        ))
    }

    /// The field's text, parsed against the grid's columns in the format locale.
    fn parsed(&self, cx: &App) -> Option<Result<GotoTarget, GotoError>> {
        let (_, adapter) = self.target()?;
        let (decimal, group) = separators(&adapter.display());
        let columns = hacivat_ui_grid::GridDataSource::columns(&*adapter);
        Some(parse_goto(
            self.input.read(cx).value(),
            &columns,
            decimal,
            group,
        ))
    }

    /// The rows the grid shows and the known total (a filter's rows are all known).
    fn rows(&self, cx: &App) -> Option<(u64, Option<u64>)> {
        let (grid, adapter) = self.target()?;
        let snapshot = adapter.snapshot();
        let loaded = snapshot.row_count();
        Some(match grid.read(cx).view_map() {
            map @ ViewMap::Filtered { .. } => (map.len(), Some(map.len())),
            _ => (
                loaded,
                matches!(snapshot.completeness(), Completeness::Complete).then_some(loaded),
            ),
        })
    }

    fn column_name(&self, col: ColId) -> SharedString {
        self.target()
            .and_then(|(_, a)| {
                hacivat_ui_grid::GridDataSource::columns(&*a)
                    .iter()
                    .find(|c| c.id == col)
                    .map(|c| c.name.clone())
            })
            .unwrap_or_default()
    }

    /// The line under the field: the hint, the live target, or an error.
    fn line(&self, cx: &App) -> GotoLine {
        if let Some(e) = &self.error {
            return GotoLine::Error(e.clone());
        }
        let hint = || GotoLine::Hint(message(cx, "hv-goto-hint", vec![]));
        let Some(parsed) = self.parsed(cx) else {
            return hint();
        };
        match parsed {
            Err(GotoError::Empty) => hint(),
            Err(GotoError::UnknownColumn) => {
                let text = self.input.read(cx).value();
                let name = text
                    .split_once([':', ','])
                    .map_or("", |(_, c)| c)
                    .trim()
                    .to_owned();
                GotoLine::Error(message(
                    cx,
                    "hv-goto-unknown-col",
                    vec![("name", ArgValue::Text(name))],
                ))
            }
            Err(GotoError::OutOfRange { last }) => self.past_end(u64::MAX, last, cx),
            Err(_) => GotoLine::Error(message(cx, "hv-goto-hint", vec![])),
            Ok(t) => {
                if let Some(col) = t.col()
                    && let Some((grid, _)) = self.target()
                    && grid.read(cx).column_layout().index_of(col).is_none()
                {
                    let name = self.column_name(col);
                    return GotoLine::Hidden(
                        message(
                            cx,
                            "hv-goto-hidden-col",
                            vec![("name", ArgValue::Text(name.to_string()))],
                        ),
                        col,
                    );
                }
                if let Some((shown, total)) = self.rows(cx)
                    && let Err(GotoError::OutOfRange { last }) =
                        hacivat_ui_grid::model::place_row(t.row(), shown, total)
                {
                    return self.past_end(t.row(), last, cx);
                }
                let row = ArgValue::Count(t.row());
                match t.col() {
                    Some(col) => GotoLine::Target(message(
                        cx,
                        "hv-goto-target",
                        vec![
                            ("row", row),
                            ("col", ArgValue::Text(self.column_name(col).to_string())),
                        ],
                    )),
                    None => GotoLine::Target(message(cx, "hv-goto-target-row", vec![("row", row)])),
                }
            }
        }
    }

    fn past_end(&self, row: u64, last: Option<u64>, cx: &App) -> GotoLine {
        GotoLine::Error(message(
            cx,
            "hv-goto-past-end",
            vec![
                ("row", ArgValue::Count(row)),
                ("total", ArgValue::Count(last.unwrap_or(0))),
            ],
        ))
    }

    /// Enter: the grid goes to the cell; past the fetched rows it fetches (asking above a
    /// million); an error stays on the line.
    fn submit(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, GotoState::Fetching { .. }) {
            return;
        }
        let Some((grid, adapter)) = self.target() else {
            return;
        };
        let Some(Ok(target)) = self.parsed(cx) else {
            cx.notify();
            return;
        };
        match grid.update(cx, |g, cx| g.go_to(target, cx)) {
            Ok(GotoPlace::Shown(_)) => self.landed(&grid, cx),
            Ok(GotoPlace::NotFetched) => {
                let snapshot = adapter.snapshot();
                let loaded = snapshot.row_count();
                let open = matches!(
                    snapshot.completeness(),
                    Completeness::Suspended(_) | Completeness::Streaming
                ) && snapshot.paused().is_none();
                if !open {
                    let line = self.past_end(target.row(), Some(loaded), cx);
                    if let GotoLine::Error(e) = line {
                        self.error = Some(e);
                    }
                } else {
                    let more = target.row().saturating_sub(loaded);
                    if more > ASK_ABOVE {
                        self.state = GotoState::Ask {
                            row: target.row(),
                            more,
                        };
                        self.pending = Some((target, loaded));
                        self.take_focus = true;
                    } else {
                        self.start_fetch(target, loaded, cx);
                    }
                }
            }
            Err(GotoError::OutOfRange { last }) => {
                if let GotoLine::Error(e) = self.past_end(target.row(), last, cx) {
                    self.error = Some(e);
                }
            }
            Err(_) => {}
        }
        cx.notify();
    }

    /// The grid went to the row: the popover closes, the grid takes the focus.
    fn landed(&mut self, grid: &Entity<GridView>, cx: &mut Context<Self>) {
        self.state = GotoState::Idle;
        self.pending = None;
        self.open = false;
        let handle = grid.read(cx).focus_handle(cx);
        cx.defer(move |cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
            }
            notify_hosts(cx);
        });
        cx.notify();
    }

    fn fetch_and_go(&mut self, cx: &mut Context<Self>) {
        if let (GotoState::Ask { .. }, Some((target, loaded))) = (self.state, self.pending) {
            self.start_fetch(target, loaded, cx);
        }
    }

    fn start_fetch(&mut self, target: GotoTarget, loaded: u64, cx: &mut Context<Self>) {
        self.state = GotoState::Fetching {
            row: target.row(),
            more: target.row().saturating_sub(loaded),
        };
        self.pending = Some((target, loaded));
        self.take_focus = true;
        self.drive(cx);
        notify_hosts(cx);
    }

    /// While fetching: the row arrived (go), the result ended or paused (stop), else ask for
    /// the next step (the source asks nothing twice).
    fn drive(&mut self, cx: &mut Context<Self>) {
        let (GotoState::Fetching { .. }, Some((target, _))) = (self.state, self.pending) else {
            return;
        };
        let Some((grid, adapter)) = self.target() else {
            return;
        };
        let snapshot = adapter.snapshot();
        let loaded = snapshot.row_count();
        if loaded >= target.row() {
            // The grid must have painted the rows first (its count is the last frame's).
            if grid.read(cx).generation() == Some(adapter.generation())
                && let Ok(GotoPlace::Shown(_)) = grid.update(cx, |g, cx| g.go_to(target, cx))
            {
                self.landed(&grid, cx);
            }
            return;
        }
        match snapshot.completeness() {
            // The source asks nothing twice for one need; no repaint here (the tile repaints
            // when rows arrive).
            Completeness::Suspended(_) if snapshot.paused().is_none() => {
                let upto = target.row().min(loaded.saturating_add(FETCH_STEP));
                adapter.source().ensure_rows(upto);
            }
            Completeness::Streaming => {}
            other => {
                let complete = matches!(other, Completeness::Complete);
                self.state = GotoState::Idle;
                self.pending = None;
                if complete
                    && let GotoLine::Error(e) = self.past_end(target.row(), Some(loaded), cx)
                {
                    self.error = Some(e);
                }
                cx.notify();
            }
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.state,
            GotoState::Fetching { .. } | GotoState::Ask { .. }
        ) {
            self.state = GotoState::Idle;
            self.pending = None;
            cx.notify();
        }
    }

    /// Escape: hides the popover; the question is dropped, a fetching goes on.
    fn hide(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, GotoState::Ask { .. }) {
            self.state = GotoState::Idle;
            self.pending = None;
        }
        self.open = false;
        if let Some((grid, _)) = self.target() {
            let handle = grid.read(cx).focus_handle(cx);
            cx.defer(move |cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
                }
            });
        }
        cx.defer(notify_hosts);
        cx.notify();
    }

    /// "Show it and go": the hidden column shows, and the grid goes.
    fn show_and_go(&mut self, col: ColId, cx: &mut Context<Self>) {
        if let Some((grid, _)) = self.target() {
            grid.update(cx, |g, cx| g.set_column_hidden(col, false, cx));
            self.submit(cx);
        }
    }
}

impl Render for GotoPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let asking = matches!(
            self.state,
            GotoState::Ask { .. } | GotoState::Fetching { .. }
        );
        if std::mem::take(&mut self.take_focus) {
            let handle = if asking {
                self.primary.clone()
            } else {
                self.input.focus_handle(cx)
            };
            // After this frame: the field's focus listeners are installed by then.
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let entity = cx.entity();
        let title = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .child(
                Icon::new(IconName::ArrowRight)
                    .size(IconSize::Chrome)
                    .color(c.text_2.to_hsla()),
            )
            .child(Label::new(
                message(cx, "hv-goto-title", vec![]),
                TextSource::Fluent,
                t.type_ramp.caption,
                c.text.to_hsla(),
            ));
        let line = self.line(cx);
        let invalid = matches!(line, GotoLine::Error(_) | GotoLine::Hidden(..));
        let well = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let field_focused = self.input.focus_handle(cx).is_focused(window);
        let field = div()
            .id("goto-field")
            .debug_selector(|| "goto-field".to_string())
            .h(px(26.0))
            .flex()
            .items_center()
            .px(px(6.0))
            .rounded(px(t.radii.sm))
            .border_1()
            .border_color(if invalid {
                c.text_2.to_hsla()
            } else {
                c.seam_strong.to_hsla()
            })
            .bg(well.to_hsla())
            .child(div().flex_1().child(self.input.clone()));
        let field = focus_outline(field, &theme, field_focused);
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(title)
            .child(field);
        match self.state {
            GotoState::Idle => {
                let (glyph, ink, text) = match &line {
                    GotoLine::Hint(t) => (IconName::Info, c.text_2, t.clone()),
                    GotoLine::Target(t) => (IconName::ArrowRight, c.text_2, t.clone()),
                    GotoLine::Error(t) | GotoLine::Hidden(t, _) => {
                        (IconName::CircleAlert, c.text, t.clone())
                    }
                };
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(12.0))
                            .pt(px(1.0))
                            .flex_none()
                            .child(Icon::new(glyph).size(IconSize::Chrome).color(ink.to_hsla())),
                    )
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            paragraph(text, TextSource::Fluent, t.type_ramp.caption, ink.to_hsla())
                                .id("goto-line"),
                        ),
                    );
                if let GotoLine::Hidden(_, col) = line {
                    let e = entity.clone();
                    row = row.child(
                        Button::new("goto-show-col", message(cx, "hv-goto-show-col", vec![]))
                            .kind(ButtonKind::Default)
                            .compact(true)
                            .on_raised(true)
                            .on_click(move |_, _, cx| e.update(cx, |p, cx| p.show_and_go(col, cx))),
                    );
                }
                // The keys, as the palette's footer draws them ("↵ Go · esc Close").
                let hint = |chord: &str, id: &str| {
                    let presses = parse_chord(chord).unwrap_or_default();
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.0))
                        .child(Label::new(
                            SharedString::from(display_chord(&presses, platform())),
                            TextSource::Shortcut,
                            t.type_ramp.mono_kbd,
                            c.text_2.to_hsla(),
                        ))
                        .child(Label::new(
                            message(cx, id, vec![]),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        ))
                };
                body = body.child(row).child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(hint("enter", "hv-goto-key-go"))
                        .child(hint("escape", "hui-palette-hint-close")),
                );
            }
            GotoState::Ask { row, more } => {
                let (e1, e2) = (entity.clone(), entity.clone());
                body = body
                    .child(
                        paragraph(
                            message(
                                cx,
                                "hv-goto-fetch",
                                vec![
                                    ("row", ArgValue::Count(row)),
                                    ("more", ArgValue::Count(more)),
                                ],
                            ),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text.to_hsla(),
                        )
                        .id("goto-line"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.0))
                            .child(
                                Button::new("goto-cancel", message(cx, "hv-goto-cancel", vec![]))
                                    .kind(ButtonKind::Default)
                                    .on_raised(true)
                                    .on_click(move |_, _, cx| e1.update(cx, |p, cx| p.stop(cx))),
                            )
                            .child(
                                Button::new(
                                    "goto-fetch-ok",
                                    message(cx, "hv-goto-fetch-ok", vec![]),
                                )
                                .kind(ButtonKind::Primary)
                                .on_raised(true)
                                .focus_handle(self.primary.clone())
                                .on_click(move |_, _, cx| {
                                    e2.update(cx, |p, cx| p.fetch_and_go(cx))
                                }),
                            ),
                    );
            }
            GotoState::Fetching { row, more } => {
                let done = self
                    .target()
                    .zip(self.pending)
                    .map_or(0, |((_, a), (_, from))| {
                        a.snapshot().row_count().saturating_sub(from)
                    });
                let e = entity.clone();
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_start()
                            .gap(px(6.0))
                            .child(div().pt(px(4.0)).child(run_marker(&theme)))
                            .child(
                                paragraph(
                                    message(
                                        cx,
                                        "hv-goto-fetching",
                                        vec![
                                            ("row", ArgValue::Count(row)),
                                            ("done", ArgValue::Count(done.min(more))),
                                            ("more", ArgValue::Count(more)),
                                        ],
                                    ),
                                    TextSource::Fluent,
                                    t.type_ramp.caption,
                                    c.text.to_hsla(),
                                )
                                .id("goto-line"),
                            ),
                    )
                    .child(Label::new(
                        message(cx, "hv-goto-fetching-note", vec![]),
                        TextSource::Fluent,
                        t.type_ramp.caption,
                        c.text_2.to_hsla(),
                    ))
                    .child(
                        div().flex().flex_row().justify_end().child(
                            Button::new("goto-stop", message(cx, "hv-goto-stop", vec![]))
                                .kind(ButtonKind::Default)
                                .on_raised(true)
                                .focus_handle(self.primary.clone())
                                .on_click(move |_, _, cx| e.update(cx, |p, cx| p.stop(cx))),
                        ),
                    );
            }
        }
        let panel = popover_surface(&theme)
            .id(("goto-popover", self.console.0))
            .debug_selector(|| "goto-popover".to_string())
            .w(px(300.0))
            .p(px(10.0))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                if !matches!(p.state, GotoState::Ask { .. }) {
                    p.hide(cx);
                }
            }))
            .on_key_down(cx.listener(
                |p, ev: &hacivat_ui_core::gpui_surface::KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" && !ev.keystroke.modifiers.modified() {
                        cx.stop_propagation();
                        p.hide(cx);
                    }
                },
            ))
            .child(body);
        A11yNode::new(
            "goto-popover-node",
            Role::Dialog,
            message(cx, "hv-goto-title", vec![]),
            false,
            panel,
        )
    }
}

impl std::fmt::Debug for GotoPopover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GotoPopover")
            .field("console", &self.console)
            .field("open", &self.open)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
