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
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window,
    div, point, px,
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
    // ## m3c-int (I2)
    /// The adapter generation the question or the fetching is for: a re-run ends either.
    generation: u64,
    /// The error a submit found (past the end).
    error: Option<SharedString>,
    grid: Option<WeakEntity<GridView>>,
    adapter: Option<Weak<GridAdapter>>,
    primary: FocusHandle,
    // ## m3c-shotfix F5 (fix round 1, I1)
    /// The scroll of the lines between the title and the buttons (a short room).
    body_scroll: ScrollHandle,
    // ## m3c-shotfix F5 (fix round 2, m-i): the room the host gives the popover, as the current
    // layout measures it (`room_probe`); a change draws the popover again.
    room: Rc<std::cell::Cell<Option<f32>>>,
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
        // ## m3c-shotfix F5 (fix round 2, m-i): every opening starts at the top.
        p.body_scroll.set_offset(point(px(0.0), px(0.0)));
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
    // ## m3c-int (I3): go to row is the table's: off while the tab shows another view, also on
    // a tab switched to that already showed it (it never targets the hidden table grid).
    if !super::views::shows_table(grid, cx) {
        if p.read(cx).open || !matches!(p.read(cx).state, GotoState::Idle) {
            cx.defer(move |cx| p.update(cx, |p, cx| p.dismiss(cx)));
        }
        return;
    }
    // Only a new grid or a fetching needs anything (and nothing repaints otherwise: this runs
    // in every frame of the tile).
    let (same, fetching) = {
        let p = p.read(cx);
        (
            p.grid
                .as_ref()
                .is_some_and(|g| g.entity_id() == grid.entity_id()),
            // ## m3c-int (I2): so does a question asked about an older generation.
            matches!(p.state, GotoState::Fetching { .. })
                || (matches!(p.state, GotoState::Ask { .. })
                    && p.generation != adapter.generation()),
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
            // ## m3c-int (I2)
            if p.reran(cx) {
                return;
            }
            p.drive(cx);
        });
    });
}

/// The popover, while shown (the tile lays it over its grid).
pub(crate) fn overlay(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let p = popover(console, cx).filter(|p| p.read(cx).open)?;
    Some(p.into_any_element())
}

// ## m3c-shotfix F5 (fix round 2, m-i)
/// Measures, in the host's current layout, the room it gives console `console`'s popover (the
/// host's absolutely placed wrapper); a change draws again, so the frame shown is planned on
/// the layout it shows.
pub(crate) fn room_probe(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let room = popover(console, cx)?.read(cx).room.clone();
    Some(
        hacivat_ui_core::gpui_surface::canvas(
            move |bounds, window, _| {
                let h = f32::from(bounds.size.height);
                if room.get().is_none_or(|r| (r - h).abs() > 0.5) {
                    room.set(Some(h));
                    window.refresh();
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element(),
    )
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
            // ## inputnav-1 (named exception, F2): the well around it draws the frame and the
            // ring; the field drew a second border and ring inside it.
            input.set_frameless(true);
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
            generation: 0,
            error: None,
            grid: None,
            adapter: None,
            primary: cx.focus_handle().tab_stop(true),
            body_scroll: ScrollHandle::new(),
            room: Rc::default(),
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

    // ## m3c-shotfix F5
    /// Whether the grid's area (the popover's room, as the last frame laid it out) is too short
    /// for the title, the field, a three-line question and the buttons ([`short_room_px`]).
    fn short_room(&self, window: &Window, cx: &App) -> bool {
        let need = short_room_px(window, cx);
        // ## fix round 2 (m-i): the room the host's layout gives (its probe redraws when it
        // changes, so the frame shown decides on its own layout); the grid's last frame only
        // before the first measure.
        let room = self.room.get().map(|r| r + 2.0 * HOST_GAP_PX).or_else(|| {
            self.target()
                .and_then(|(grid, _)| grid.read(cx).geometry().map(|g| g.bounds.size.height))
                .map(f32::from)
        });
        room.is_some_and(|h| h < need)
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
                // ## m3c-t37 fix round 1 (m3): split where `parse_goto` does (a comma names
                // the column only where it groups nothing).
                let text = self.input.read(cx).value();
                let group = self
                    .target()
                    .map_or(',', |(_, a)| separators(&a.display()).1);
                let name = text
                    .char_indices()
                    .find(|(_, c)| *c == ':' || (*c == ',' && group != ','))
                    .map_or("", |(at, c)| &text[at + c.len_utf8()..])
                    .trim()
                    .to_owned();
                GotoLine::Error(message(
                    cx,
                    "hv-goto-unknown-col",
                    vec![("name", ArgValue::Text(name))],
                ))
            }
            // ## m3c-t37 fix round 1 (m3): a number too large for a row, or no number.
            Err(GotoError::OutOfRange { .. } | GotoError::NotANumber) => {
                GotoLine::Error(message(cx, "hv-goto-not-a-row", vec![]))
            }
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
        // ## m3c-t37 fix round 1 (m3): under Filter rows the rows are the filter's.
        let filtered = self
            .target()
            .is_some_and(|(g, _)| matches!(g.read(cx).view_map(), ViewMap::Filtered { .. }));
        GotoLine::Error(message(
            cx,
            if filtered {
                "hv-goto-past-end-filter"
            } else {
                "hv-goto-past-end"
            },
            vec![
                ("row", ArgValue::Count(row)),
                ("total", ArgValue::Count(last.unwrap_or(0))),
            ],
        ))
    }

    /// Enter: the grid goes to the cell; past the fetched rows it fetches (asking above a
    /// million); an error stays on the line.
    fn submit(&mut self, cx: &mut Context<Self>) {
        // ## m3c-int (I3): a hidden popover takes no Enter (it may target a grid not shown).
        if matches!(self.state, GotoState::Fetching { .. }) || !self.open {
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
                    // ## m3c-int (I2)
                    self.generation = adapter.generation();
                    if more > ASK_ABOVE {
                        self.state = GotoState::Ask {
                            row: target.row(),
                            more,
                        };
                        self.pending = Some((target, loaded));
                        self.take_focus = true;
                        // A new question starts at its first line (fix round 2, m-i).
                        self.body_scroll.set_offset(point(px(0.0), px(0.0)));
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
            // ## m3c-int (I2): the answer is for the result it was asked about.
            if self.reran(cx) {
                return;
            }
            self.start_fetch(target, loaded, cx);
        }
    }

    // ## m3c-int (I2)
    /// The result was re-run since the question or the fetching started: it ends (no row of
    /// the new result is fetched or gone to), and the line says why. Answers whether it ended.
    fn reran(&mut self, cx: &mut Context<Self>) -> bool {
        if matches!(self.state, GotoState::Idle)
            || !self
                .target()
                .is_some_and(|(_, a)| a.generation() != self.generation)
        {
            return false;
        }
        self.state = GotoState::Idle;
        self.pending = None;
        self.error = Some(message(cx, "hv-goto-fetch-rerun", vec![]));
        cx.notify();
        true
    }

    fn start_fetch(&mut self, target: GotoTarget, loaded: u64, cx: &mut Context<Self>) {
        self.state = GotoState::Fetching {
            row: target.row(),
            more: target.row().saturating_sub(loaded),
        };
        self.pending = Some((target, loaded));
        self.take_focus = true;
        self.body_scroll.set_offset(point(px(0.0), px(0.0)));
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
        // ## m3c-int (I2): never a row of a newer generation.
        if self.reran(cx) {
            return;
        }
        let snapshot = adapter.snapshot();
        let loaded = snapshot.row_count();
        if loaded >= target.row() {
            // The grid must have painted the rows first (its count is the last frame's).
            if grid.read(cx).generation() != Some(adapter.generation()) {
                return;
            }
            match grid.update(cx, |g, cx| g.go_to(target, cx)) {
                Ok(GotoPlace::Shown(_)) => self.landed(&grid, cx),
                Ok(GotoPlace::NotFetched) => {}
                // ## m3c-t37 fix round 1 (m4): the rows came but the row is not shown (Filter
                // rows went on, the column was hidden): the fetching ends and says why.
                Err(e) => {
                    self.state = GotoState::Idle;
                    self.pending = None;
                    self.error = match e {
                        GotoError::OutOfRange { last } => {
                            match self.past_end(target.row(), last, cx) {
                                GotoLine::Error(e) => Some(e),
                                _ => None,
                            }
                        }
                        GotoError::HiddenColumn(col) => Some(message(
                            cx,
                            "hv-goto-hidden-col",
                            vec![("name", ArgValue::Text(self.column_name(col).to_string()))],
                        )),
                        _ => None,
                    };
                    cx.notify();
                }
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
                self.state = GotoState::Idle;
                self.pending = None;
                // ## m3c-t37 fix round 1 (m4): say why the fetching stopped.
                let row = ArgValue::Count(loaded);
                self.error = match other {
                    Completeness::Complete => match self.past_end(target.row(), Some(loaded), cx) {
                        GotoLine::Error(e) => Some(e),
                        _ => None,
                    },
                    Completeness::Suspended(_) => {
                        Some(message(cx, "hv-goto-fetch-paused", vec![("row", row)]))
                    }
                    _ => Some(message(cx, "hv-goto-fetch-closed", vec![("row", row)])),
                };
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

    // ## m3c-int (I3)
    /// The tab shows a view other than the table: the popover hides and its question or
    /// fetching ends (no focus move: the view shown keeps it).
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.state = GotoState::Idle;
        self.pending = None;
        self.open = false;
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

// ## m3c-shotfix F5 (fix round 1, m10): the numbers the popover is laid out with; the
// short-room threshold adds them up ([`short_room_px`]), and a test pins it to the drawn popover.
/// The popover's padding and the gap between its rows.
const PAD_PX: f32 = 10.0;
const ROW_GAP_PX: f32 = 8.0;
/// The field's height.
const FIELD_PX: f32 = 26.0;
/// The popover surface's border.
const SURFACE_BORDER_PX: f32 = 1.0;
/// The host's gap above and below the popover (`view.rs`).
pub const HOST_GAP_PX: f32 = 4.0;

/// The height of the grid's area under which the asking popover gives up its title and its
/// field to the question: the padding, the title, the field, a three-line question, the
/// buttons and the gaps between them, and the host's gaps.
pub fn short_room_px(window: &Window, cx: &App) -> f32 {
    let t = Theme::get(cx).tokens;
    let line = f32::from(window.pixel_snap(px(t.type_ramp.caption.line_height)));
    2.0 * PAD_PX
        + line
        + ROW_GAP_PX
        + FIELD_PX
        + ROW_GAP_PX
        + 3.0 * line
        + ROW_GAP_PX
        + t.spacing.control_height
        + 2.0 * SURFACE_BORDER_PX
        + 2.0 * HOST_GAP_PX
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
        // ## m3c-shotfix F5 (fix round 1, I1): asking or fetching in a short room, the title
        // and then the field give way to the question (the title only names what was opened;
        // the question names the row the field holds).
        let short = asking && self.short_room(window, cx);
        let title = div()
            .debug_selector(|| "goto-title".to_string())
            .flex()
            .flex_row()
            .flex_none()
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
            .h(px(FIELD_PX))
            .flex()
            .items_center()
            // ## inputnav-1 (named exception): the mock's `.in` padding (0 6 0 9), now that the
            // field inside is frameless.
            .pl(px(9.0))
            .pr(px(6.0))
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
        // ## m3c-shotfix F5: the popover never runs past its room (under the toolbar, above the
        // strip; NOTES cross-cutting 3): its title and its keys or buttons always show, and in
        // a short room what lies between them (the field and the lines) scrolls.
        // ## inputnav-1 (named exception, F2): room for the field's focus ring, which the body's
        // clip cut at the sides.
        let mut body = hacivat_ui_core::theme::ring_room(
            div()
                .id("goto-body")
                .debug_selector(|| "goto-body".to_string())
                .flex()
                .flex_col()
                .gap(px(ROW_GAP_PX))
                .flex_shrink(1.0)
                .min_h(px(0.0))
                .overflow_y_scroll(),
        )
        .track_scroll(&self.body_scroll)
        .child(hacivat_ui::text::scroll_viewport());
        if !short {
            body = body.child(field);
        }
        let foot: AnyElement;
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
                body = body.child(row);
                foot = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(12.0))
                    .child(hint("enter", "hv-goto-key-go"))
                    .child(hint("escape", "hui-palette-hint-close"))
                    .into_any_element();
            }
            GotoState::Ask { row, more } => {
                let (e1, e2) = (entity.clone(), entity.clone());
                body = body.child(
                    div()
                        .debug_selector(|| "goto-question".to_string())
                        .flex_none()
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
                        ),
                );
                foot = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("goto-cancel", message(cx, "hv-goto-cancel", vec![]))
                            .kind(ButtonKind::Default)
                            .on_raised(true)
                            .on_click(move |_, _, cx| e1.update(cx, |p, cx| p.stop(cx))),
                    )
                    .child(
                        Button::new("goto-fetch-ok", message(cx, "hv-goto-fetch-ok", vec![]))
                            .kind(ButtonKind::Primary)
                            .on_raised(true)
                            .focus_handle(self.primary.clone())
                            .on_click(move |_, _, cx| e2.update(cx, |p, cx| p.fetch_and_go(cx))),
                    )
                    .into_any_element();
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
                            // ## m3c-shotfix F5: the line wraps in what the marker leaves it.
                            .child(
                                div().flex_1().min_w(px(0.0)).child(
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
                            ),
                    )
                    // ## m3c-shotfix F5: prose, so it wraps in the 300 px card.
                    .child(
                        paragraph(
                            message(cx, "hv-goto-fetching-note", vec![]),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )
                        .id("goto-fetching-note"),
                    );
                foot = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .justify_end()
                    .child(
                        Button::new("goto-stop", message(cx, "hv-goto-stop", vec![]))
                            .kind(ButtonKind::Default)
                            .on_raised(true)
                            .focus_handle(self.primary.clone())
                            .on_click(move |_, _, cx| e.update(cx, |p, cx| p.stop(cx))),
                    )
                    .into_any_element();
            }
        }
        let panel = popover_surface(&theme)
            .id(("goto-popover", self.console.0))
            .debug_selector(|| "goto-popover".to_string())
            .w(px(300.0))
            // ## m3c-shotfix F5: at most its room (the host gives it the grid's height).
            .max_h_full()
            .flex()
            .flex_col()
            .gap(px(ROW_GAP_PX))
            .p(px(PAD_PX))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| {
                if !matches!(p.state, GotoState::Ask { .. }) {
                    p.hide(cx);
                }
            }))
            .on_key_down(cx.listener(
                |p, ev: &hacivat_ui_core::gpui_surface::KeyDownEvent, _, cx| {
                    let k = &ev.keystroke;
                    if k.modifiers.modified() {
                        return;
                    }
                    if k.key == "escape" {
                        cx.stop_propagation();
                        p.hide(cx);
                        return;
                    }
                    // ## m3c-shotfix F5 (fix round 1, I1): while it asks or fetches, the arrows
                    // and the page keys scroll the question from its buttons, so all of it is
                    // read without a pointer (the buttons take no arrows).
                    if !matches!(p.state, GotoState::Ask { .. } | GotoState::Fetching { .. }) {
                        return;
                    }
                    let line = Theme::get(cx).tokens.type_ramp.caption.line_height;
                    let dy = match k.key.as_str() {
                        "up" => -line,
                        "down" => line,
                        "pageup" => -3.0 * line,
                        "pagedown" => 3.0 * line,
                        _ => return,
                    };
                    let o = p.body_scroll.offset();
                    let max = p.body_scroll.max_offset();
                    let y = (o.y - px(dy)).clamp(-max.y.max(px(0.0)), px(0.0));
                    p.body_scroll.set_offset(point(o.x, y));
                    cx.stop_propagation();
                    cx.notify();
                },
            ))
            .children((!short).then_some(title))
            .child(body)
            .child(foot);
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
