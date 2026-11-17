//! The value viewer (HV-GRID-12, R-M3-82; M3c Task 38): a 480 px popover with the column and
//! type, the wrapped value, a pretty toggle for JSON and XML, hex for binary, Copy raw value,
//! and "Re-run with a 64 MiB value limit" for a truncated value.
//!
//! ## m3b-t01b-seams: the seam (Task 1b). Task 38 fills it.
//!
//! ## m3c-t38 (M3c Task 38; NOTES surface 22, M3c-31, M3c-32)
//! - **One value, read whole, from the model.** The viewer opens on a **model** cell (the grid's
//!   focus is a model cell, so a sort or a filter never changes which value it reads) of the
//!   console's active result, from the snapshot the adapter holds, and closes when the result
//!   moves on (a re-run, another tab, another view). It is non-modal: anchored under its cell
//!   (above when there is no room), clamped inside the grid's area, never over the console
//!   toolbar; Escape, × or a click outside closes it and gives the focus back to the grid.
//! - **Raw bytes are copied, never the display.** The well shows the value's raw channel
//!   (`valfmt::raw_text_in`: the server's text, ungrouped numbers, bytes as hex). **Pretty**
//!   (JSON and XML) re-indents for display only: JSON keeps its key order and every token as the
//!   server wrote it (only white space outside strings changes), and a value that does not parse
//!   keeps Raw with Pretty disabled. **Copy raw value** always copies the raw text, whatever is
//!   shown: pretty-printing never changes copied bytes. A NULL or a decode error has nothing to
//!   copy.
//! - **Hex** for binary: the offset (`text-3`), 8 bytes per row in 2-byte groups (the popover is
//!   under 560 px), then the ASCII column (`text-2`).
//! - **Large values.** The well wraps the text itself (Fragment Mono has one advance) and builds
//!   only the lines it shows (`VirtualList`): a 64 MiB value costs what its visible lines cost.
//!   The lines are cut on the background executor.
//! - **Truncated** (the store cut the value at the console's value limit): "Showing the first
//!   1 MB of 5.2 MB." and **Re-run with a 64 MB value limit**, offered while the kept part is
//!   under the largest limit. It emits `ResultsEvent::RerunWithLimit`, which the run UI handles
//!   exactly like Re-run (`orchestrator::rerun`): only the text that ran, still in the editor,
//!   through Run's pipeline (parameters, classification of the bytes sent, the gate, the
//!   confirmations; production asks as Run does); only the run's fetch policy differs.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use hacivat_l10n::ArgValue;
use hacivat_ui::button::{Button, ButtonKind};
use hacivat_ui::icon::IconName;
use hacivat_ui::popover::{A11yNode, Tip};
use hacivat_ui::segmented::{SegmentContent, SegmentItem, Segmented};
use hacivat_ui::text::{Label, TextSource};
use hacivat_ui::toast::{Toast, ToastKind, ToastQueue};
use hacivat_ui_core::fonts::font_for;
use hacivat_ui_core::gpui_surface::{
    AnyElement, App, AppContext as _, ClipboardItem, Context, Entity, FocusHandle, Focusable as _,
    Global, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Task, WeakEntity, Window, div, px,
};
use hacivat_ui_core::theme::{Theme, ThemeMode, popover_surface};
use hacivat_ui_grid::{
    GridDataSource as _, GridView, ModelCell, RowRenderer, ViewRow, VirtualList,
};
use hacivat_values::valfmt::{RawText, raw_text_in};
use hacivat_values::{LenUnit, LogicalType, ValueRef};

use crate::console::ConsoleId;
use crate::exec_ui::marks::message;
use crate::results::adapter::{GridAdapter, Texts};
use crate::results::view::ResultsEvent;

/// The popover's width (NOTES surface 22).
pub const VIEWER_WIDTH_PX: f32 = 480.0;
/// The value well's tallest and shortest (it shrinks before the popover would cross the
/// console toolbar, then scrolls).
pub const WELL_MAX_PX: f32 = 240.0;
pub const WELL_MIN_PX: f32 = 120.0;
/// Hex rows below a 560 px popover (NOTES surface 22).
pub const HEX_BYTES_PER_ROW: usize = 8;
/// A line of the well (Fragment Mono 11.5/17).
const LINE_PX: f32 = 17.0;
/// The well's padding (7/10) and the popover's (16 at the sides).
const WELL_PAD_X: f32 = 10.0;
const WELL_PAD_Y: f32 = 7.0;
const POPOVER_PAD_X: f32 = 16.0;
/// The rest of the popover around the well, for placing it (header, control row, notice,
/// footer); the well takes what is left.
const CHROME_PX: f32 = 170.0;
/// The gap between the cell and the popover, and from the grid's edges.
const GAP_PX: f32 = 4.0;

/// How the viewer shows a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueForm {
    /// Wrapped text (numbers, dates, text, NULL, a decode error).
    Text,
    /// JSON: Raw / Pretty; `valid: false` keeps Raw with Pretty disabled.
    Json { valid: bool },
    /// XML: Raw / Pretty.
    Xml,
    /// Bytes as hex.
    Binary,
}

/// What the viewer shows (tests and the shots read it).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewerView {
    /// The column's name (data).
    pub column: SharedString,
    /// "jsonb · row 4".
    pub subtitle: SharedString,
    pub form: ValueForm,
    /// Pretty is pressed.
    pub pretty: bool,
    /// The well's text, as shown (pretty or raw; hex rows for bytes).
    pub body: String,
    /// "48 KB · 48,213 characters", "512 bytes".
    pub footer: SharedString,
    /// The truncation notice.
    pub truncated: Option<SharedString>,
    /// The re-run button's words when it is offered.
    pub rerun: Option<SharedString>,
    /// Copy raw value is enabled (not for NULL or a decode error).
    pub can_copy: bool,
}

/// The value the viewer reads, once per open (and per Raw / Pretty).
#[derive(Clone)]
struct Loaded {
    view: ViewerView,
    /// What Copy raw value copies.
    raw: Option<Arc<str>>,
    /// The kept part's bytes, when the store cut the value.
    kept: Option<u64>,
    /// The result tab's unit (Re-run).
    unit_ordinal: u32,
}

/// The token kinds the well colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Tok {
    Key,
    Str,
    Num,
    Lit,
    Tag,
    Attr,
    /// Hex offsets.
    Dim,
    /// Hex ASCII.
    Soft,
}

/// The well's lines: the text, each wrapped line's byte range, and the coloured runs.
#[derive(Clone, Default)]
struct WellLines {
    text: Arc<str>,
    lines: Arc<[Range<usize>]>,
    tokens: Arc<[(Range<usize>, Tok)]>,
}

impl RowRenderer for WellLines {
    fn len(&self) -> u64 {
        self.lines.len() as u64
    }

    fn row(&self, ix: u64, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let Some(range) = self.lines.get(ix as usize).cloned() else {
            return div().into_any_element();
        };
        let text = SharedString::from(self.text[range.clone()].to_string());
        // The coloured runs inside this line (tokens are sorted and do not overlap).
        let first = self.tokens.partition_point(|(r, _)| r.end <= range.start);
        let mut highlights = Vec::new();
        for (r, tok) in self.tokens[first..].iter() {
            if r.start >= range.end {
                break;
            }
            let (s, e) = (r.start.max(range.start), r.end.min(range.end));
            if s < e {
                let color = token_color(*tok, &theme);
                highlights.push((
                    s - range.start..e - range.start,
                    HighlightStyle {
                        color: Some(color),
                        ..HighlightStyle::default()
                    },
                ));
            }
        }
        let style = t.type_ramp.mono_data;
        div()
            .h(px(LINE_PX))
            .whitespace_nowrap()
            .font(font_for(&style))
            .text_size(px(style.size_px))
            .line_height(px(LINE_PX))
            .text_color(c.text.to_hsla())
            .child(StyledText::new(text).with_highlights(highlights))
            .into_any_element()
    }
}

fn token_color(tok: Tok, theme: &Theme) -> Hsla {
    let c = &theme.tokens.colors;
    match tok {
        Tok::Key | Tok::Attr => c.syn_identifier,
        Tok::Str => c.syn_string,
        Tok::Num => c.syn_number,
        Tok::Lit | Tok::Tag => c.syn_keyword,
        Tok::Dim => c.text_3,
        Tok::Soft => c.text_2,
    }
    .to_hsla()
}

/// One console's value viewer.
pub struct ValueViewer {
    console: ConsoleId,
    open: bool,
    take_focus: bool,
    cell: Option<ModelCell>,
    adapter: Option<Weak<GridAdapter>>,
    /// The adapter generation the cell was read from: a re-run closes the viewer.
    generation: u64,
    /// The grid the viewer is anchored to (the table's, or the Transposed view's) and the cell
    /// there; the focus goes back to it.
    anchor: Option<(WeakEntity<GridView>, ModelCell)>,
    /// The table grid (row numbers through its view map).
    table: Option<WeakEntity<GridView>>,
    pretty: bool,
    loaded: Option<Loaded>,
    well: Entity<VirtualList<WellLines>>,
    /// The width the well's lines were cut for (characters per line), and the form.
    cut_for: Option<(usize, bool)>,
    cutting: Option<Task<()>>,
    focus: FocusHandle,
    /// The char advance of the well's font, measured once.
    advance: Cell<Option<f32>>,
}

#[derive(Default)]
struct Viewers(HashMap<ConsoleId, Entity<ValueViewer>>);

impl Global for Viewers {}

fn viewer(console: ConsoleId, cx: &App) -> Option<Entity<ValueViewer>> {
    cx.try_global::<Viewers>()?.0.get(&console).cloned()
}

fn ensure(console: ConsoleId, cx: &mut App) -> Entity<ValueViewer> {
    if let Some(v) = viewer(console, cx) {
        return v;
    }
    let v = cx.new(|cx| ValueViewer::new(console, cx));
    cx.default_global::<Viewers>().0.insert(console, v.clone());
    v
}

/// Opens the viewer on `cell` of console `console`'s active result (the tile calls it on
/// `GridEvent::ValueRequested`), anchored to that cell in the table grid.
pub fn open(console: ConsoleId, cell: ModelCell, cx: &mut App) {
    let anchor = super::find::active_target(console, cx).map(|(g, _)| (g.downgrade(), cell));
    open_anchored(console, cell, anchor, cx);
}

/// [`open`] anchored to `anchor` (a grid and its cell that shows `cell`: the Transposed view's).
pub(crate) fn open_anchored(
    console: ConsoleId,
    cell: ModelCell,
    anchor: Option<(WeakEntity<GridView>, ModelCell)>,
    cx: &mut App,
) {
    let Some((table, adapter)) = super::find::active_target(console, cx) else {
        return;
    };
    let unit_ordinal = crate::console::Consoles::get(cx, console)
        .and_then(|s| s.results.clone())
        .and_then(|r| r.read(cx).active_tab().map(|t| t.key().unit_ordinal))
        .unwrap_or(0);
    let v = ensure(console, cx);
    v.update(cx, |v, cx| {
        v.open = true;
        v.take_focus = true;
        v.cell = Some(cell);
        v.adapter = Some(Rc::downgrade(&adapter));
        v.generation = adapter.generation();
        v.anchor = anchor;
        v.table = Some(table.downgrade());
        v.pretty = false;
        v.loaded = None;
        v.cut_for = None;
        v.load(&adapter, &table, unit_ordinal, cx);
        cx.notify();
    });
    notify_hosts(cx);
}

/// Whether console `console`'s viewer is open.
pub fn is_open(console: ConsoleId, cx: &App) -> bool {
    viewer(console, cx).is_some_and(|v| v.read(cx).open)
}

/// What console `console`'s open viewer shows.
pub fn view(console: ConsoleId, cx: &App) -> Option<ViewerView> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    v.read(cx).loaded.as_ref().map(|l| l.view.clone())
}

/// The cell the open viewer reads.
pub fn cell(console: ConsoleId, cx: &App) -> Option<ModelCell> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    v.read(cx).cell
}

/// Presses Raw (`false`) or Pretty (`true`), as the switch does (refused for invalid JSON).
pub fn set_pretty(console: ConsoleId, pretty: bool, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.set_pretty(pretty, cx));
    }
}

/// Copy raw value: the raw text on the clipboard, whatever the well shows.
pub fn copy_raw(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.copy_raw(cx));
    }
}

/// Re-run with the largest value limit (when offered).
pub fn rerun(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx) {
        v.update(cx, |v, cx| v.rerun(cx));
    }
}

/// Closes the viewer and gives the focus back to its grid.
pub fn close(console: ConsoleId, cx: &mut App) {
    if let Some(v) = viewer(console, cx).filter(|v| v.read(cx).open) {
        v.update(cx, |v, cx| v.close(cx));
    }
}

/// Forgets console `console`'s viewer (its console closed).
pub fn console_closed(console: ConsoleId, cx: &mut App) {
    if cx.has_global::<Viewers>() {
        cx.global_mut::<Viewers>().0.remove(&console);
    }
}

/// The tile shows `adapter` for console `console`: a viewer reading another result (another
/// tab, a re-run) closes.
pub(crate) fn sync(console: ConsoleId, adapter: &Rc<GridAdapter>, cx: &mut App) {
    let Some(v) = viewer(console, cx) else {
        return;
    };
    let stale = {
        let v = v.read(cx);
        v.open
            && !v
                .adapter
                .as_ref()
                .and_then(Weak::upgrade)
                .is_some_and(|a| Rc::ptr_eq(&a, adapter) && a.generation() == v.generation)
    };
    if stale {
        cx.defer(move |cx| v.update(cx, |v, cx| v.hide(cx)));
    }
}

/// The open viewer of console `console`, placed in the grid's area (the tile's body).
pub(crate) fn overlay(console: ConsoleId, cx: &App) -> Option<AnyElement> {
    let v = viewer(console, cx).filter(|v| v.read(cx).open)?;
    let (top, bottom, left, well_h) = v.read(cx).placement(cx);
    let mut place = div().absolute().left(px(left));
    place = match (top, bottom) {
        (Some(top), _) => place.top(px(top)),
        (None, Some(bottom)) => place.bottom(px(bottom)),
        (None, None) => place.top(px(GAP_PX)),
    };
    let _ = well_h;
    Some(place.child(v).into_any_element())
}

fn notify_hosts(cx: &mut App) {
    for host in super::view::hosts(cx) {
        host.update(cx, |_, cx| cx.notify());
    }
}

impl ValueViewer {
    fn new(console: ConsoleId, cx: &mut Context<Self>) -> Self {
        ValueViewer {
            console,
            open: false,
            take_focus: false,
            cell: None,
            adapter: None,
            generation: 0,
            anchor: None,
            table: None,
            pretty: false,
            loaded: None,
            well: cx.new(|_| VirtualList::new(WellLines::default(), LINE_PX)),
            cut_for: None,
            cutting: None,
            focus: cx.focus_handle(),
            advance: Cell::new(None),
        }
    }

    /// Reads the cell (module docs): the raw text, the form, the notice and the footer.
    fn load(
        &mut self,
        adapter: &Rc<GridAdapter>,
        table: &Entity<GridView>,
        unit_ordinal: u32,
        cx: &App,
    ) {
        let Some(cell) = self.cell else { return };
        let raw = adapter.raw_view();
        let snapshot = &raw.snapshot;
        let col = cell.col.0 as usize;
        if cell.row.0 >= snapshot.row_count() || col >= raw.facts.len() {
            self.loaded = None;
            return;
        }
        let facts = &raw.facts[col];
        let value = snapshot.cell(cell.row.0, cell.col.0);
        let (inner, cut) = match value {
            ValueRef::Truncated(t) => (t.prefix.view(), Some((t.total_len, t.unit))),
            v => (v, None),
        };
        let form = match (inner, &facts.logical) {
            (ValueRef::Bytes(_), _) => ValueForm::Binary,
            (ValueRef::Null | ValueRef::Error(_), _) => ValueForm::Text,
            (ValueRef::Json(_), _) | (ValueRef::Text(_), LogicalType::Json { .. }) => {
                ValueForm::Json { valid: false }
            }
            (ValueRef::Xml(_), _) | (ValueRef::Text(_), LogicalType::Xml) => ValueForm::Xml,
            _ => ValueForm::Text,
        };
        let texts = Texts::of(cx);
        let (raw_text, can_copy): (Option<Arc<str>>, bool) = match inner {
            ValueRef::Null => (None, false),
            ValueRef::Error(e) => (Some(Arc::from(&*e.message)), false),
            v => match raw_text_in(v, &facts.view(raw.display.engine()), raw.display.zone()) {
                RawText::Null => (None, false),
                RawText::Text(t) => (Some(Arc::from(t.as_ref())), true),
            },
        };
        let form = match form {
            ValueForm::Json { .. } => ValueForm::Json {
                // A cut JSON value never parses: it stays Raw.
                valid: cut.is_none() && raw_text.as_deref().is_some_and(is_json),
            },
            f => f,
        };
        let (body, footer, kept) = match inner {
            ValueRef::Bytes(b) => (
                hex_lines(b, HEX_BYTES_PER_ROW).join("\n"),
                message(
                    cx,
                    "hv-vv-bytes",
                    vec![("n", ArgValue::Count(b.len() as u64))],
                ),
                b.len() as u64,
            ),
            ValueRef::Null => (
                hacivat_ui_grid::NULL_MARKER.to_string(),
                SharedString::default(),
                0,
            ),
            _ => {
                let text = raw_text.as_deref().unwrap_or_default();
                let n = text.chars().count() as u64;
                (
                    text.to_string(),
                    message(
                        cx,
                        "hv-vv-chars",
                        vec![
                            ("size", ArgValue::Text(texts.size(text.len() as u64))),
                            ("n", ArgValue::Count(n)),
                        ],
                    ),
                    text.len() as u64,
                )
            }
        };
        let cap = u64::from(hacivat_results::window::MAX_CELL_CAP);
        let truncated = cut.map(|(total, unit)| {
            let shown = texts.size(kept);
            match total {
                Some(total) => {
                    let size = match unit {
                        LenUnit::Bytes => texts.size(total),
                        LenUnit::Chars => texts.tr(
                            hacivat_l10n::ids::hv::GRID_SIZE_CHARS,
                            hacivat_l10n::ids::hv::HvGridSizeChars {
                                n: hacivat_l10n::Count(total),
                            },
                        ),
                    };
                    message(
                        cx,
                        "hv-vv-truncated",
                        vec![
                            ("shown", ArgValue::Text(shown)),
                            ("size", ArgValue::Text(size)),
                        ],
                    )
                }
                None => message(
                    cx,
                    "hv-vv-truncated-unknown",
                    vec![("shown", ArgValue::Text(shown))],
                ),
            }
        });
        // Re-run helps while the kept part is under the largest limit.
        let rerun = (cut.is_some() && kept < cap).then(|| {
            message(
                cx,
                "hv-vv-rerun",
                vec![("limit", ArgValue::Text(texts.size(cap)))],
            )
        });
        let column = adapter
            .columns()
            .get(col)
            .map(|c| (c.name.clone(), c.type_label.clone()))
            .unwrap_or_default();
        let row = table
            .read(cx)
            .view_map()
            .model_to_view(cell.row)
            .map_or(cell.row.0, |v: ViewRow| v.0)
            + 1;
        let subtitle = message(
            cx,
            "hv-vv-sub",
            vec![
                ("type", ArgValue::Text(column.1.to_string())),
                ("row", ArgValue::Count(row)),
            ],
        );
        self.loaded = Some(Loaded {
            view: ViewerView {
                column: column.0,
                subtitle,
                form,
                pretty: false,
                body,
                footer,
                truncated,
                rerun,
                can_copy,
            },
            raw: raw_text.filter(|_| can_copy),
            kept: cut.map(|_| kept),
            unit_ordinal,
        });
    }

    fn set_pretty(&mut self, pretty: bool, cx: &mut Context<Self>) {
        let Some(l) = self.loaded.as_mut() else {
            return;
        };
        let can = matches!(
            l.view.form,
            ValueForm::Json { valid: true } | ValueForm::Xml
        );
        let pretty = pretty && can;
        if l.view.pretty == pretty {
            return;
        }
        let raw = l.raw.clone().unwrap_or_else(|| Arc::from(""));
        l.view.pretty = pretty;
        l.view.body = match (pretty, l.view.form) {
            (true, ValueForm::Json { .. }) => pretty_json(&raw).unwrap_or_else(|| raw.to_string()),
            (true, ValueForm::Xml) => pretty_xml(&raw),
            _ => raw.to_string(),
        };
        self.pretty = pretty;
        self.cut_for = None;
        cx.notify();
    }

    fn copy_raw(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.loaded.as_ref() else {
            return;
        };
        let Some(raw) = l.raw.clone() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(raw.to_string()));
        let toast = match l.kept {
            Some(kept) => {
                let shown = Texts::of(cx).size(kept);
                let text = message(
                    cx,
                    "hv-vv-copied-cut",
                    vec![("shown", ArgValue::Text(shown))],
                );
                let mut toast = Toast::new(ToastKind::Info, Rc::new(move |_: &App| text.clone()));
                toast.icon = Some(IconName::TriangleAlert);
                toast.wide = true;
                toast
            }
            None => {
                let mut toast = Toast::new(
                    ToastKind::Info,
                    Rc::new(|cx: &App| message(cx, "hv-vv-copied", vec![])),
                );
                toast.icon = Some(IconName::CopyCheck);
                toast
            }
        };
        ToastQueue::push(cx, toast);
    }

    fn rerun(&mut self, cx: &mut Context<Self>) {
        let Some(l) = self.loaded.as_ref().filter(|l| l.view.rerun.is_some()) else {
            return;
        };
        let (console, unit_ordinal) = (self.console, l.unit_ordinal);
        self.hide(cx);
        cx.defer(move |cx| {
            let Some(results) =
                crate::console::Consoles::get(cx, console).and_then(|s| s.results.clone())
            else {
                return;
            };
            results.update(cx, |_, cx| {
                cx.emit(ResultsEvent::RerunWithLimit {
                    console,
                    unit_ordinal,
                    cell_cap: hacivat_results::window::MAX_CELL_CAP,
                })
            });
        });
    }

    /// Closes without moving the focus.
    fn hide(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.loaded = None;
        self.cutting = None;
        cx.defer(notify_hosts);
        cx.notify();
    }

    /// Closes and gives the focus back to the grid it is anchored to.
    fn close(&mut self, cx: &mut Context<Self>) {
        let grid = self.anchor.as_ref().and_then(|(g, _)| g.upgrade());
        self.hide(cx);
        if let Some(grid) = grid {
            let handle = grid.read(cx).focus_handle(cx);
            cx.defer(move |cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, cx| window.focus(&handle, cx));
                }
            });
        }
    }

    /// Where the popover goes in the grid's area: `(top, bottom, left, well height)` in the
    /// area's coordinates (module docs).
    fn placement(&self, cx: &App) -> (Option<f32>, Option<f32>, f32, f32) {
        let Some((grid, cell)) = self
            .anchor
            .as_ref()
            .and_then(|(g, c)| Some((g.upgrade()?, *c)))
        else {
            return (Some(GAP_PX), None, GAP_PX, WELL_MAX_PX);
        };
        let g = grid.read(cx);
        let Some(geom) = g.geometry() else {
            return (Some(GAP_PX), None, GAP_PX, WELL_MAX_PX);
        };
        let area_w = f32::from(geom.bounds.size.width);
        let area_h = f32::from(geom.bounds.size.height);
        let origin = geom.bounds.origin;
        let scroll = g.scroll_state();
        let layout = g.column_layout();
        let view_row = g.view_map().model_to_view(cell.row).map_or(0, |v| v.0);
        let row_top = f32::from(geom.body.top() - origin.y) + scroll.row_top(view_row) as f32;
        let row_bottom = row_top + geom.row_h;
        let x = layout.index_of(cell.col).map_or(GAP_PX, |ix| {
            let screen = if ix < geom.frozen {
                layout.x_of(ix) as f32
            } else {
                layout.screen_x(ix, scroll.left_x) as f32
            };
            f32::from(geom.body.left() - origin.x) + screen
        });
        let left = x.min(area_w - VIEWER_WIDTH_PX - GAP_PX).max(GAP_PX);
        let below = area_h - row_bottom - 2.0 * GAP_PX;
        let above = row_top - 2.0 * GAP_PX;
        let need = CHROME_PX + WELL_MAX_PX;
        if below >= need {
            (Some(row_bottom + GAP_PX), None, left, WELL_MAX_PX)
        } else if above >= need {
            (None, Some(area_h - row_top + GAP_PX), left, WELL_MAX_PX)
        } else if below >= above {
            let well = (below - CHROME_PX).clamp(WELL_MIN_PX, WELL_MAX_PX);
            (
                Some(
                    (row_bottom + GAP_PX)
                        .min(area_h - CHROME_PX - well)
                        .max(GAP_PX),
                ),
                None,
                left,
                well,
            )
        } else {
            let well = (above - CHROME_PX).clamp(WELL_MIN_PX, WELL_MAX_PX);
            (Some(GAP_PX), None, left, well)
        }
    }

    /// The well's font advance (Fragment Mono: one advance for every glyph).
    fn advance(&self, window: &Window, cx: &App) -> f32 {
        if let Some(a) = self.advance.get() {
            return a;
        }
        let style = Theme::get(cx).tokens.type_ramp.mono_data;
        let ts = window.text_system();
        let id = ts.resolve_font(&font_for(&style));
        let a = ts
            .em_advance(id, px(style.size_px))
            .map(f32::from)
            .ok()
            .filter(|a| *a > 0.0)
            .unwrap_or(style.size_px * 0.6);
        self.advance.set(Some(a));
        a
    }

    /// Cuts the well's lines for `cols` characters per line (on the background executor).
    fn cut_lines(&mut self, cols: usize, cx: &mut Context<Self>) {
        let Some(l) = self.loaded.as_ref() else {
            return;
        };
        let key = (cols, l.view.pretty);
        if self.cut_for == Some(key) {
            return;
        }
        self.cut_for = Some(key);
        let text: Arc<str> = Arc::from(l.view.body.as_str());
        let form = l.view.form;
        let job = cx.background_spawn(async move {
            let lines = wrap_lines(&text, cols);
            let tokens = match form {
                ValueForm::Json { .. } => json_tokens(&text),
                ValueForm::Xml => xml_tokens(&text),
                ValueForm::Binary => hex_tokens(&text),
                ValueForm::Text => Vec::new(),
            };
            WellLines {
                text,
                lines: lines.into(),
                tokens: tokens.into(),
            }
        });
        let well = self.well.clone();
        self.cutting = Some(cx.spawn(async move |_, cx| {
            let lines = job.await;
            cx.update(|cx| {
                well.update(cx, |w, cx| {
                    *w.rows_mut(cx) = lines;
                    w.scroll_to_top(cx);
                })
            });
        }));
    }
}

impl Render for ValueViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.take_focus) {
            let handle = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&handle, cx));
        }
        let theme = *Theme::get(cx);
        let t = theme.tokens;
        let c = &t.colors;
        let (_, _, _, well_h) = self.placement(cx);
        let advance = self.advance(window, cx);
        let inner_w = VIEWER_WIDTH_PX - 2.0 * POPOVER_PAD_X - 2.0 * WELL_PAD_X - 6.0;
        let cols = ((inner_w / advance).floor() as usize).max(8);
        self.cut_lines(cols, cx);
        let Some(l) = self.loaded.as_ref() else {
            return div().into_any_element();
        };
        let entity = cx.entity();
        let view = &l.view;
        let header = div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(8.0))
            .pt(px(12.0))
            .px(px(POPOVER_PAD_X))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Label::new(
                            view.column.clone(),
                            TextSource::Catalog,
                            t.type_ramp.code,
                            c.text.to_hsla(),
                        )
                        .truncate(true)
                        .id("value-viewer-column"),
                    )
                    .child(
                        Label::new(
                            view.subtitle.clone(),
                            TextSource::Fluent,
                            t.type_ramp.body_sm,
                            c.text_2.to_hsla(),
                        )
                        .id("value-viewer-sub"),
                    ),
            )
            .child({
                let e = entity.clone();
                Button::new(
                    "value-viewer-close",
                    message(cx, "hui-dialog-close", vec![]),
                )
                .kind(ButtonKind::Icon)
                .icon(IconName::X)
                .on_raised(true)
                .on_click(move |_, _, cx| e.update(cx, |v, cx| v.close(cx)))
            });
        let mut control = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(POPOVER_PAD_X))
            .pt(px(8.0));
        let mut has_control = false;
        match view.form {
            ValueForm::Json { .. } | ValueForm::Xml => {
                has_control = true;
                let disabled = matches!(view.form, ValueForm::Json { valid: false });
                let (e1, e2) = (entity.clone(), entity.clone());
                let mut seg = Segmented::new("value-viewer-form").item(SegmentItem {
                    content: SegmentContent::Label(message(cx, "hv-vv-raw", vec![])),
                    pressed: !view.pretty,
                    tooltip: message(cx, "hv-vv-raw", vec![]),
                    a11y_name: None,
                    on_click: Rc::new(move |_, _, cx| {
                        e1.update(cx, |v, cx| v.set_pretty(false, cx))
                    }),
                });
                if !disabled {
                    seg = seg.item(SegmentItem {
                        content: SegmentContent::Label(message(cx, "hv-vv-pretty", vec![])),
                        pressed: view.pretty,
                        tooltip: message(cx, "hv-vv-pretty-tip", vec![]),
                        a11y_name: Some(message(cx, "hv-vv-pretty", vec![])),
                        on_click: Rc::new(move |_, _, cx| {
                            e2.update(cx, |v, cx| v.set_pretty(true, cx))
                        }),
                    });
                }
                control = control.child(seg);
                if disabled {
                    control =
                        control.child(div().flex_1().min_w_0().child(hacivat_ui::text::paragraph(
                            message(cx, "hv-vv-not-json", vec![]),
                            TextSource::Fluent,
                            t.type_ramp.caption,
                            c.text_2.to_hsla(),
                        )));
                }
            }
            ValueForm::Binary => {
                has_control = true;
                control = control.child(Segmented::new("value-viewer-form").item(SegmentItem {
                    content: SegmentContent::Label(message(cx, "hv-vv-hex", vec![])),
                    pressed: true,
                    tooltip: message(cx, "hv-vv-hex", vec![]),
                    a11y_name: None,
                    on_click: Rc::new(|_, _, _| {}),
                }));
            }
            ValueForm::Text => {}
        }
        let well_fill = match theme.mode {
            ThemeMode::Dark => c.panel,
            ThemeMode::Light => c.ground,
        };
        let lines = self.well.read(cx).rows().len() as f32;
        let height = (lines * LINE_PX).clamp(LINE_PX, well_h) + 2.0 * WELL_PAD_Y;
        let e_keys = entity.clone();
        let well = div()
            .id("value-viewer-well")
            .debug_selector(|| "value-viewer-well".to_string())
            .track_focus(&self.focus)
            .mx(px(POPOVER_PAD_X))
            .mt(px(8.0))
            .h(px(height))
            .px(px(WELL_PAD_X))
            .py(px(WELL_PAD_Y))
            .bg(well_fill.to_hsla())
            .border_1()
            .border_color(c.seam_strong.to_hsla())
            .rounded(px(4.0))
            .overflow_hidden()
            .on_key_down(move |ev: &KeyDownEvent, _, cx| {
                let k = &ev.keystroke;
                let dy = match k.key.as_str() {
                    "up" => -LINE_PX,
                    "down" => LINE_PX,
                    "pageup" => -WELL_MAX_PX,
                    "pagedown" => WELL_MAX_PX,
                    _ => return,
                };
                if k.modifiers.modified() {
                    return;
                }
                let well = e_keys.read(cx).well.clone();
                well.update(cx, |w, cx| w.scroll_px(f64::from(dy), cx));
                cx.stop_propagation();
            })
            .child(self.well.clone());
        let mut body = div().flex().flex_col().w(px(VIEWER_WIDTH_PX)).child(header);
        if has_control {
            body = body.child(control);
        }
        body = body.child(well);
        if let Some(notice) = view.truncated.clone() {
            let mut row = div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .px(px(POPOVER_PAD_X))
                .pt(px(8.0))
                .child(hacivat_ui::text::paragraph(
                    notice,
                    TextSource::Fluent,
                    t.type_ramp.caption,
                    c.text_2.to_hsla(),
                ));
            if let Some(words) = view.rerun.clone() {
                let e = entity.clone();
                let tip = message(cx, "hv-vv-rerun-tip", vec![]);
                row = row.child(
                    div()
                        .id("value-viewer-rerun-tip")
                        .tooltip(move |_, cx| Tip::view(tip.clone(), cx))
                        .child(
                            Button::new("value-viewer-rerun", words)
                                .kind(ButtonKind::Default)
                                .icon(IconName::RotateCw)
                                .on_raised(true)
                                .on_click(move |_, _, cx| e.update(cx, |v, cx| v.rerun(cx))),
                        ),
                );
            }
            body = body.child(row);
        }
        let e = entity.clone();
        body = body.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .pt(px(12.0))
                .pb(px(14.0))
                .px(px(POPOVER_PAD_X))
                .child(
                    Label::new(
                        view.footer.clone(),
                        TextSource::Fluent,
                        t.type_ramp.mono_data,
                        c.text_2.to_hsla(),
                    )
                    .id("value-viewer-size"),
                )
                .child(
                    Button::new("value-viewer-copy", message(cx, "hv-vv-copy", vec![]))
                        .kind(ButtonKind::Default)
                        .icon(IconName::Copy)
                        .on_raised(true)
                        .disabled(!view.can_copy)
                        .on_click(move |_, _, cx| e.update(cx, |v, cx| v.copy_raw(cx))),
                ),
        );
        let panel = popover_surface(&theme)
            .id(("value-viewer", self.console.0))
            .debug_selector(|| "value-viewer".to_string())
            .on_mouse_down_out(cx.listener(|v, _, _, cx| v.hide(cx)))
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.key == "escape" && !ev.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    v.close(cx);
                }
            }))
            .child(body);
        A11yNode::new(
            "value-viewer-node",
            Role::Dialog,
            message(cx, "hv-vv-label", vec![]),
            false,
            panel,
        )
        .into_any_element()
    }
}

impl std::fmt::Debug for ValueViewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValueViewer")
            .field("console", &self.console)
            .field("open", &self.open)
            .field("cell", &self.cell)
            .finish_non_exhaustive()
    }
}

// ---- pure text helpers (any thread) -------------------------------------------------------------

/// Whether `text` is one JSON value.
pub fn is_json(text: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok()
}

/// `text` re-indented by two spaces per level, keeping every token (and the key order) as
/// written: only white space outside strings changes. `None` when `text` is not JSON.
pub fn pretty_json(text: &str) -> Option<String> {
    if !is_json(text) {
        return None;
    }
    let mut out = String::with_capacity(text.len() + text.len() / 4);
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let mut it = text.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        for _ in 0..depth {
            out.push_str("  ");
        }
    };
    while let Some(ch) = it.next() {
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                out.push(ch);
            }
            '{' | '[' => {
                let close = if ch == '{' { '}' } else { ']' };
                let mut look = it.clone();
                while look.peek().is_some_and(|c| c.is_whitespace()) {
                    look.next();
                }
                if look.peek() == Some(&close) {
                    // An empty container stays on its line.
                    out.push(ch);
                    out.push(close);
                    it = look;
                    it.next();
                    continue;
                }
                out.push(ch);
                depth += 1;
                newline(&mut out, depth);
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(ch);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

/// One piece of an XML text.
enum XmlPiece<'a> {
    Open(&'a str),
    Close(&'a str),
    /// A self-closing tag, a declaration, a comment, CDATA.
    Single(&'a str),
    Text(&'a str),
}

fn xml_pieces(text: &str) -> Vec<XmlPiece<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // The tag ends at the first `>` outside quotes (comments and CDATA at their end).
            let rest = &text[i..];
            let end = if rest.starts_with("<!--") {
                rest.find("-->").map(|e| e + 3)
            } else if rest.starts_with("<![CDATA[") {
                rest.find("]]>").map(|e| e + 3)
            } else {
                let mut quote = None;
                rest.char_indices().find_map(|(j, c)| match (quote, c) {
                    (None, '"' | '\'') => {
                        quote = Some(c);
                        None
                    }
                    (Some(q), c) if c == q => {
                        quote = None;
                        None
                    }
                    (None, '>') => Some(j + 1),
                    _ => None,
                })
            }
            .unwrap_or(rest.len());
            let tag = &rest[..end];
            out.push(if tag.starts_with("</") {
                XmlPiece::Close(tag)
            } else if tag.ends_with("/>") || tag.starts_with("<?") || tag.starts_with("<!") {
                XmlPiece::Single(tag)
            } else {
                XmlPiece::Open(tag)
            });
            i += end;
        } else {
            let end = text[i..].find('<').map_or(text.len(), |e| i + e);
            out.push(XmlPiece::Text(&text[i..end]));
            i = end;
        }
    }
    out
}

/// `text` (XML) with one element per line, indented by two spaces per level; an element that
/// holds only text stays on one line. Display only.
pub fn pretty_xml(text: &str) -> String {
    if !text.contains('<') {
        return text.to_string();
    }
    let pieces = xml_pieces(text);
    let mut lines: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let indent = |depth: usize| "  ".repeat(depth);
    let mut i = 0;
    while i < pieces.len() {
        match &pieces[i] {
            XmlPiece::Open(tag) => {
                // `<a>text</a>` stays on one line.
                if let (Some(XmlPiece::Text(t)), Some(XmlPiece::Close(close))) =
                    (pieces.get(i + 1), pieces.get(i + 2))
                    && !t.contains('\n')
                {
                    lines.push(format!("{}{tag}{}{close}", indent(depth), t.trim()));
                    i += 3;
                    continue;
                }
                if let Some(XmlPiece::Close(close)) = pieces.get(i + 1) {
                    lines.push(format!("{}{tag}{close}", indent(depth)));
                    i += 2;
                    continue;
                }
                lines.push(format!("{}{tag}", indent(depth)));
                depth += 1;
            }
            XmlPiece::Close(tag) => {
                depth = depth.saturating_sub(1);
                lines.push(format!("{}{tag}", indent(depth)));
            }
            XmlPiece::Single(tag) => lines.push(format!("{}{tag}", indent(depth))),
            XmlPiece::Text(t) => {
                let t = t.trim();
                if !t.is_empty() {
                    lines.push(format!("{}{t}", indent(depth)));
                }
            }
        }
        i += 1;
    }
    lines.join("\n")
}

/// `bytes` as hex rows: the offset, `per_row` bytes in 2-byte groups, the ASCII column (a
/// printable ASCII byte as itself, any other as `.`).
pub fn hex_lines(bytes: &[u8], per_row: usize) -> Vec<String> {
    let per_row = per_row.max(2);
    // Two hex digits per byte, a space between groups of two bytes.
    let hex_w = per_row * 2 + per_row.div_ceil(2) - 1;
    bytes
        .chunks(per_row)
        .enumerate()
        .map(|(i, chunk)| {
            let mut hex = String::with_capacity(hex_w);
            for (j, b) in chunk.iter().enumerate() {
                if j > 0 && j % 2 == 0 {
                    hex.push(' ');
                }
                hex.push_str(&format!("{b:02x}"));
            }
            let ascii: String = chunk
                .iter()
                .map(|b| {
                    if (b' '..=b'~').contains(b) {
                        char::from(*b)
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:08x}  {hex:<hex_w$}  {ascii}", i * per_row)
        })
        .collect()
}

/// The byte ranges of `text`'s lines wrapped at `cols` characters (a line break always ends a
/// line; a long line is cut at `cols`).
fn wrap_lines(text: &str, cols: usize) -> Vec<Range<usize>> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        let mut from = start;
        let mut n = 0;
        for (i, _) in line.char_indices() {
            if n == cols {
                out.push(from..start + i);
                from = start + i;
                n = 0;
            }
            n += 1;
        }
        out.push(from..start + line.len());
        start += line.len() + 1;
    }
    out
}

/// JSON's coloured tokens: keys, strings, numbers, literals.
fn json_tokens(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(b.len());
                // A string followed by `:` is a key.
                let mut j = i;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                let tok = if j < b.len() && b[j] == b':' {
                    Tok::Key
                } else {
                    Tok::Str
                };
                out.push((start..i, tok));
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                while i < b.len() && matches!(b[i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    i += 1;
                }
                out.push((start..i, Tok::Num));
            }
            b't' | b'f' | b'n' => {
                let start = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                out.push((start..i, Tok::Lit));
            }
            _ => i += 1,
        }
    }
    out
}

/// XML's coloured tokens: tag names, attribute names, attribute values.
fn xml_tokens(text: &str) -> Vec<(Range<usize>, Tok)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        // The tag's name.
        let start = i;
        i += 1;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            if b[i] == b'/' && i > start + 1 {
                break;
            }
            i += 1;
        }
        out.push((start..i, Tok::Tag));
        // Attributes until `>`.
        while i < b.len() && b[i] != b'>' {
            if b[i] == b'"' || b[i] == b'\'' {
                let q = b[i];
                let s = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                i = (i + 1).min(b.len());
                out.push((s..i, Tok::Str));
            } else if b[i].is_ascii_alphabetic() {
                let s = i;
                while i < b.len()
                    && !matches!(b[i], b'=' | b'>' | b'/')
                    && !b[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                out.push((s..i, Tok::Attr));
            } else {
                i += 1;
            }
        }
        if i < b.len() {
            out.push((i..i + 1, Tok::Tag));
            i += 1;
        }
    }
    out
}

/// Hex rows' coloured parts: the offset dim, the ASCII column soft.
fn hex_tokens(text: &str) -> Vec<(Range<usize>, Tok)> {
    let mut out = Vec::new();
    let mut start = 0;
    for line in text.split('\n') {
        out.push((start..start + line.len().min(8), Tok::Dim));
        if let Some(at) = line.rfind("  ") {
            out.push((start + at + 2..start + line.len(), Tok::Soft));
        }
        start += line.len() + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_json_keeps_tokens_and_key_order() {
        let raw = r#"{"b":1,"a":[1.50,true,null,"x, y"],"e":{},"f":[]}"#;
        let pretty = pretty_json(raw).unwrap();
        assert_eq!(
            pretty,
            "{\n  \"b\": 1,\n  \"a\": [\n    1.50,\n    true,\n    null,\n    \"x, y\"\n  ],\n  \"e\": {},\n  \"f\": []\n}"
        );
        // Only white space outside strings changed.
        let squeeze = |s: &str| {
            let mut out = String::new();
            let mut in_str = false;
            let mut esc = false;
            for c in s.chars() {
                if in_str {
                    out.push(c);
                    if esc {
                        esc = false;
                    } else if c == '\\' {
                        esc = true;
                    } else if c == '"' {
                        in_str = false;
                    }
                } else if c == '"' {
                    in_str = true;
                    out.push(c);
                } else if !c.is_whitespace() {
                    out.push(c);
                }
            }
            out
        };
        assert_eq!(squeeze(&pretty), squeeze(raw));
        assert_eq!(pretty_json("{\"a\": 1"), None);
        assert_eq!(pretty_json("not json"), None);
    }

    #[test]
    fn pretty_xml_indents_elements() {
        let raw = r#"<?xml version="1.0"?><a x="1"><b>text</b><c/><d><e>1</e></d></a>"#;
        assert_eq!(
            pretty_xml(raw),
            "<?xml version=\"1.0\"?>\n<a x=\"1\">\n  <b>text</b>\n  <c/>\n  <d>\n    <e>1</e>\n  </d>\n</a>"
        );
        assert_eq!(pretty_xml("plain"), "plain");
    }

    #[test]
    fn hex_rows_have_offsets_groups_and_ascii() {
        let bytes: Vec<u8> = (0u8..10).chain(*b"Az").collect();
        let lines = hex_lines(&bytes, 8);
        assert_eq!(
            lines,
            [
                "00000000  0001 0203 0405 0607  ........",
                "00000008  0809 417a            ..Az",
            ]
        );
    }

    #[test]
    fn lines_wrap_at_the_width_and_at_breaks() {
        let text = "abcdef\nxy\n";
        let lines: Vec<&str> = wrap_lines(text, 4).into_iter().map(|r| &text[r]).collect();
        assert_eq!(lines, ["abcd", "ef", "xy", ""]);
        let text = "ğğğğğ";
        let lines: Vec<&str> = wrap_lines(text, 2).into_iter().map(|r| &text[r]).collect();
        assert_eq!(lines, ["ğğ", "ğğ", "ğ"]);
    }

    #[test]
    fn json_tokens_tell_keys_from_strings() {
        let text = r#"{"k": "v", "n": -1.5e3, "t": true}"#;
        let toks: Vec<(&str, Tok)> = json_tokens(text)
            .into_iter()
            .map(|(r, t)| (&text[r], t))
            .collect();
        assert_eq!(
            toks,
            [
                ("\"k\"", Tok::Key),
                ("\"v\"", Tok::Str),
                ("\"n\"", Tok::Key),
                ("-1.5e3", Tok::Num),
                ("\"t\"", Tok::Key),
                ("true", Tok::Lit),
            ]
        );
    }
}
